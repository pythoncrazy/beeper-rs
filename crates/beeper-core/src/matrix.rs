use crate::config::{now_rfc3339_millis, now_unix_millis, rfc3339_from_millis};
use crate::import::DesktopCrypto;
use crate::models::{
    Attachment, AttachmentSize, BridgeAccountInfo, Chat, ChatParticipants, ChatReminder, EditHistoryEntry, Label,
    Message, MessageLink, PollData, PollOption, PollRules, Reaction, User, VerificationSas, VerificationSasEmoji,
    VerificationState, ALL_FILES, ALL_FORMATS, BEEPER_POLLS, DELETE_FOR_EVERYONE, DELETE_FOR_ME, EDITS, FILE_GIF,
    FILE_VOICE, FORMATS,
};
use crate::prefs::Ad;
use crate::store::{sort_key, BeeperStore, NetworkSpace, SharedStore, NOTE_CHAT_ID};
use futures_util::StreamExt;
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::deserialized_responses::{RawAnySyncOrStrippedState, TimelineEvent};
use matrix_sdk::encryption::backups::BackupState;
use matrix_sdk::encryption::verification::{SasState, Verification, VerificationRequestState};
use matrix_sdk::encryption::{BackupDownloadStrategy, EncryptionSettings};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk::notification_settings::RoomNotificationMode;
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::api::client::media::get_content_thumbnail::v3::Method;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType as SendReceiptType;
use matrix_sdk::ruma::events::reaction::ReactionEventContent;
use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
use matrix_sdk::ruma::events::relation::{Annotation, Reply};
use matrix_sdk::ruma::events::room::message::{Relation, ReplacementMetadata, RoomMessageEventContent};
use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::tag::{TagInfo, TagName};
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, RoomAccountDataEventType, StateEventType};
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::OwnedEventId;
use matrix_sdk::{Client, LoopCtrl, Room, RoomMemberships, SessionMeta, SessionTokens};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::sync::oneshot;

pub const UTD_TEXT: &str = "Waiting for message";
const HISTORY: u32 = 30;

enum Op {
    Upsert(Box<Message>),
    React(String, Reaction, Box<Message>),
    Edit {
        target: String,
        text: Option<String>,
        html: Option<String>,
        links: Vec<MessageLink>,
        attachments: Vec<Attachment>,
        message_type: &'static str,
        unresolved_media: Option<String>,
        timestamp: Option<String>,
    },
    Redact(String),
    Remove(String),
    /// A poll response: the poll, the voter and the answers.
    Vote(String, String, Vec<String>),
    EndPoll(String),
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn body(c: &Value) -> Option<String> {
    let (b, p) = (s(&c["body"])?, &c["com.beeper.per_message_profile"]);
    match (s(&p["displayname"]), p["has_fallback"] == true) {
        (Some(n), true) => Some(b.strip_prefix(&format!("{n}: ")).expect("profile fallback prefix").into()),
        _ => Some(b),
    }
}

fn caption(c: &Value) -> Option<String> {
    (unresolved_media(c).is_some() || s(&c["filename"]).is_some_and(|f| c["body"] != f.as_str()))
        .then(|| body(c))
        .flatten()
}

fn html(c: &Value) -> Option<String> {
    (c["format"] == "org.matrix.custom.html").then(|| s(&c["formatted_body"])).flatten()
}

fn links(c: &Value) -> Vec<MessageLink> {
    c["com.beeper.linkpreviews"].as_array().into_iter().flatten().filter_map(link).collect()
}

async fn states(room: &Room, kind: StateEventType) -> Vec<Value> {
    room.get_state_events(kind).await.expect("read room state").into_iter().map(state_json).collect()
}

fn state_json(r: RawAnySyncOrStrippedState) -> Value {
    match r {
        RawAnySyncOrStrippedState::Sync(r) => r.deserialize_as::<Value>(),
        RawAnySyncOrStrippedState::Stripped(r) => r.deserialize_as::<Value>(),
    }
    .expect("state event JSON")
}

async fn state(room: &Room, kind: &str) -> Option<Value> {
    Some(states(room, kind.into()).await.into_iter().next()?["content"].take())
}

const REMINDER: &str = "com.beeper.chats.reminder";

async fn account_data(room: &Room, kind: &str) -> Option<Value> {
    let raw = room.account_data(RoomAccountDataEventType::from(kind)).await.expect("read room account data")?;
    Some(serde_json::from_str::<Value>(raw.json().get()).expect("account data JSON")["content"].take())
}

async fn tag(room: &Room, name: TagName, on: bool) -> matrix_sdk::Result<()> {
    match on {
        true => room.set_tag(name, TagInfo::new()).await.map(drop),
        false => room.remove_tag(name).await.map(drop),
    }
}

/// Adds a pinned room to the end of the `com.beeper.favourites_order` inbox list, or removes it.
async fn edit_pins(client: &Client, id: &str, add: bool) -> matrix_sdk::Result<()> {
    let (account, kind) = (client.account(), GlobalAccountDataEventType::from("com.beeper.favourites_order"));
    let raw = account.account_data_raw(kind.clone()).await?;
    let mut v = raw.map_or(Value::Null, |r| r.deserialize_as::<Value>().expect("account data JSON"));
    let mut ids = v["Inbox"].as_array().cloned().unwrap_or_default();
    ids.retain(|x| x != id);
    if add {
        ids.push(id.into());
    }
    v["Inbox"] = ids.into();
    let raw = Raw::from_json(serde_json::value::to_raw_value(&v).expect("account data JSON"));
    account.set_account_data_raw(kind, raw).await.map(drop)
}

fn account_id(bridge: &Value, accounts: &[BridgeAccountInfo]) -> String {
    let Some(name) = s(&bridge["com.beeper.bridge_name"]) else { return "hungryserv".into() };
    let mut matches = accounts.iter().filter(|a| a.bridge_id == name);
    let network = s(&bridge["network"]["id"]);
    let found = match network {
        Some(n) if name == "slackgo" => matches.find(|a| a.login_id.starts_with(&n)),
        _ => matches.next(),
    };
    found.map_or(name, |a| a.account_id.clone())
}

pub const RULES: &str = "com.beeper.chat_list_rules";

/// Desktop's `Xl`: the `chat_list_rules` visibility key of an account.
fn visibility_key(account_id: &str) -> &str {
    match account_id.starts_with("slackgo.") {
        true => account_id.split('-').next().expect("split yields a part"),
        false => account_id,
    }
}

/// Desktop's `xce`: the `chat_list_rules` key and the "Show in Inbox" option of an account, or of one of its network
/// spaces, which has only "all" and "none".
pub fn inbox_rule<'a>(rules: &'a Value, account_id: &str, bridge: &str, network: Option<&str>) -> (String, &'a str) {
    let vis = |key: &str| rules["visibility"][key].as_str();
    match network.filter(|_| !matches!(bridge, "slackgo" | "whatsapp" | "telegram")) {
        Some(n) => {
            let key = format!("{account_id}.{n}");
            let option = vis(&key).filter(|&o| o != "dmOnly").unwrap_or("none");
            (key, option)
        }
        None => {
            let default = if matches!(bridge, "slackgo" | "discordgo") { "dmOnly" } else { "all" };
            let key = visibility_key(account_id);
            (key.into(), vis(key).unwrap_or(default))
        }
    }
}

/// Desktop's `setInboxVisibility`.
pub fn with_inbox_visibility(mut rules: Value, key: &str, option: &str) -> Value {
    rules["visibility"][key] = option.into();
    rules
}

pub fn hidden_from_inbox(rules: &Value, account_id: &str, bridge: &Value, group: bool) -> bool {
    let name = bridge["com.beeper.bridge_name"].as_str().unwrap_or_default();
    let scoped = match s(&bridge["network"]["id"]) {
        Some(n) if !matches!(name, "slackgo" | "whatsapp" | "telegram") => {
            rules["visibility"][format!("{account_id}.{n}")].as_str()
        }
        _ => None,
    };
    match scoped.unwrap_or(inbox_rule(rules, account_id, name, None).1) {
        "all" => false,
        "none" => true,
        "dmOnly" => group && bridge["com.beeper.room_type.v2"] != "group_dm",
        other => panic!("unknown inbox visibility {other}"),
    }
}

/// Desktop's poll limits from `com.beeper.room_features`, or `None` when the room has no polls.
fn poll_rules(f: &Value) -> Option<PollRules> {
    let level = |k: &str| f[k].as_i64().is_some_and(|n| n > 0);
    let limit = |k: &str| f[k].as_u64().filter(|&n| n > 0).map(|n| u16::try_from(n).unwrap_or(u16::MAX));
    level("poll").then(|| PollRules {
        hidden_votes: level("poll_hidden_votes"),
        end: level("poll_end"),
        duplicates: level("poll_duplicate_options"),
        max_options: limit("poll_max_options").unwrap_or(0),
        max_length: limit("poll_option_max_length").unwrap_or(200),
    })
}

/// Desktop's `canCurrentUserEdit…` as `EDITS` bits: the room features support the state event, and the power level
/// of `me` reaches the level of the event.
fn edit_bits(features: Option<&Value>, levels: &Value, me: &str) -> u8 {
    let mine = levels["users"][me].as_i64().or(levels["users_default"].as_i64()).unwrap_or(0);
    EDITS.iter().enumerate().fold(0, |m, (i, k)| {
        let supported = features.is_none_or(|f| f["state"][k]["level"].as_i64().is_some_and(|l| l > 0));
        let needed = levels["events"][k].as_i64().or(levels["state_default"].as_i64()).unwrap_or(50);
        m | (u8::from(supported && mine >= needed) << i)
    })
}

async fn chat(room: &Room, accounts: &[BridgeAccountInfo], rules: &Value) -> Chat {
    let bridge = state(room, "m.bridge").await.unwrap_or(Value::Null);
    let service = room.service_members().unwrap_or_default();
    let bot = s(&bridge["bridgebot"]);
    let me = room.own_user_id();
    let members = room.members_no_sync(RoomMemberships::JOIN).await.expect("read room members");
    let levels = state(room, "m.room.power_levels").await.unwrap_or(Value::Null);
    let users_default = levels["users_default"].as_i64().unwrap_or(0).try_into().expect("power level");
    let contents: HashMap<String, Value> = states(room, StateEventType::RoomMember)
        .await
        .into_iter()
        .filter_map(|mut v| Some((s(&v["state_key"])?, v["content"].take())))
        .collect();
    let ident = |id: &str, scheme: Option<&str>| {
        let ids = contents.get(id)?["com.beeper.bridge.identifiers"].as_array()?;
        ids.iter()
            .filter_map(|i| i.as_str()?.split_once(':'))
            .find(|(k, _)| scheme.map_or(!matches!(*k, "tel" | "mailto"), |want| *k == want))
            .map(|(_, v)| v.to_string())
    };
    let items: Vec<User> = members
        .iter()
        .filter(|m| !service.contains(m.user_id()) && bot.as_deref() != Some(m.user_id().as_str()))
        .map(|m| {
            let id = m.user_id().as_str();
            User {
                id: id.to_string(),
                full_name: Some(m.name().to_string()),
                username: ident(id, None),
                email: ident(id, Some("mailto")),
                phone_number: ident(id, Some("tel")),
                img_url: m.avatar_url().filter(|u| u.is_valid()).map(|u| u.to_string()),
                is_self: Some(m.user_id() == me),
                cannot_message: Some(false),
                custom_nickname: None,
                is_network_bot: Some(contents.get(id).is_some_and(|c| c["com.beeper.bridge.is_network_bot"] == true)),
                is_admin: Some(m.power_level() > UserPowerLevel::Int(users_default)),
            }
        })
        .collect();
    let tags = room.tags().await.expect("read room tags").unwrap_or_default();
    let dm = if bridge.is_null() { items.len() <= 2 } else { bridge["com.beeper.room_type.v2"] == "dm" };
    let names = |own: bool| items.iter().filter(move |u| u.is_self == Some(own)).filter_map(|u| u.full_name.clone());
    let others: Vec<String> = names(false).collect();
    let title = room.name().filter(|n| !n.is_empty()).unwrap_or_else(|| {
        if others.is_empty() {
            names(true).next().expect("own membership")
        } else {
            others.join(", ")
        }
    });
    let account_id = account_id(&bridge, accounts);
    let features = state(room, "com.beeper.room_features").await;
    Chat {
        id: room.room_id().to_string(),
        is_hidden_from_inbox: Some(hidden_from_inbox(rules, &account_id, &bridge, !dm)),
        inbox_group: !dm && bridge["com.beeper.room_type.v2"] != "group_dm",
        is_request: Some(bridge["channel"]["com.beeper.message_request"] == true),
        network_name: s(&bridge["network"]["displayname"]),
        network_id: s(&bridge["network"]["id"]),
        img_url: room.avatar_url().filter(|u| u.is_valid()).map(|u| u.to_string()),
        account_id,
        participants: ChatParticipants { has_more: false, total: items.len(), items },
        title,
        chat_type: if dm { "single" } else { "group" }.into(),
        unread_count: u32::try_from(room.unread_notification_counts().notification_count).expect("unread count"),
        unread_mentions: u32::try_from(room.unread_notification_counts().highlight_count).expect("mention count"),
        description: room.topic(),
        draft: None,
        is_archived: Some(false),
        is_marked_unread: Some(room.is_marked_unread()),
        is_muted: Some(
            room.user_defined_notification_mode().await == Some(RoomNotificationMode::MentionsAndKeywordsOnly),
        ),
        is_pinned: Some(tags.contains_key(&TagName::Favorite)),
        is_low_priority: Some(tags.contains_key(&TagName::LowPriority)),
        label_ids: Vec::new(),
        last_activity: None,
        last_read_message_sort_key: None,
        local_chat_id: None,
        merge: None,
        merged_into_chat_id: None,
        preview: None,
        reminder: account_data(room, REMINDER).await.and_then(|r| {
            Some(ChatReminder {
                remind_at_ms: r["remind_at_ms"].as_i64().filter(|&ms| ms > 0)?,
                dismiss_on_incoming_message: r["dismiss_on_incoming_message"] == true,
            })
        }),
        chat_theme: None,
        note: None,
        notifiable: None,
        delete_modes: features.as_ref().map_or(0, |f| {
            (u8::from(f["delete_chat"] == true) * DELETE_FOR_ME)
                | (u8::from(f["delete_chat_for_everyone"] == true) * DELETE_FOR_EVERYONE)
        }),
        edits: edit_bits(features.as_ref(), &levels, me.as_str()),
        formatting: features.as_ref().map_or(ALL_FORMATS, |f| {
            FORMATS.iter().enumerate().filter(|(_, k)| f["formatting"][**k] == 2).fold(0, |m, (i, _)| m | 1 << i)
        }),
        files: features.as_ref().map_or(ALL_FILES, |f| {
            let has = |kind: &str| f["file"].get(kind).is_some_and(|v| !v.is_null());
            (u8::from(has("fi.mau.gif")) * FILE_GIF) | (u8::from(has("org.matrix.msc3245.voice")) * FILE_VOICE)
        }),
        polls: features.as_ref().map_or(Some(BEEPER_POLLS), poll_rules),
        flags: 0,
    }
}

fn url_field(url: &Value, file: &Value) -> Option<String> {
    match file.as_object() {
        Some(f) => Some(format!(
            "{}?{ENCRYPTED_FILE}={}",
            s(&file["url"]).expect("encrypted files have a URL"),
            crate::codec::url_encode(&Value::Object(f.clone()).to_string())
        )),
        None => s(url),
    }
}

fn attachment(c: &Value, kind: &str) -> Attachment {
    let info = &c["info"];
    let dim = |k: &str| info[k].as_u64().map(|n| u32::try_from(n).expect("media dimension"));
    let voice = c.get("org.matrix.msc3245.voice").is_some();
    Attachment {
        attachment_type: kind.into(),
        duration: info["duration"].as_f64().map(|ms| ms / 1000.0),
        file_name: s(&c["filename"]).or(s(&c["body"])),
        file_size: info["size"].as_u64(),
        is_gif: Some(
            info["fi.mau.gif"] == true
                || info["org.matrix.msc4230.is_animated"] == true
                || info["mimetype"] == "image/gif",
        ),
        is_sticker: Some(false),
        is_voice_note: Some(voice),
        mime_type: s(&info["mimetype"]),
        poster_img: url_field(&info["thumbnail_url"], &info["thumbnail_file"]),
        size: dim("w").map(|w| AttachmentSize { width: Some(w), height: dim("h") }),
        src_url: url_field(&c["url"], &c["file"]),
        ..Default::default()
    }
}

fn content(kind: &str, c: &Value) -> Option<(Option<String>, Vec<Attachment>, &'static str)> {
    match (kind, c["msgtype"].as_str()) {
        ("m.sticker", _) => {
            Some((None, vec![Attachment { is_sticker: Some(true), ..attachment(c, "img") }], "STICKER"))
        }
        ("m.room.message", Some("m.image")) => Some((caption(c), vec![attachment(c, "img")], "IMAGE")),
        ("m.room.message", Some("m.video")) => Some((caption(c), vec![attachment(c, "video")], "VIDEO")),
        ("m.room.message", Some("m.audio")) if c.get("org.matrix.msc3245.voice").is_some() => {
            Some((caption(c), vec![attachment(c, "audio")], "VOICE"))
        }
        ("m.room.message", Some("m.audio")) => Some((caption(c), vec![attachment(c, "audio")], "AUDIO")),
        ("m.room.message", Some("m.file")) => Some((caption(c), vec![attachment(c, "unknown")], "FILE")),
        ("m.room.message", Some("m.location")) => Some((body(c), Vec::new(), "LOCATION")),
        ("m.room.message", Some("m.notice")) => Some((body(c), Vec::new(), "NOTICE")),
        ("m.room.message", _) => Some((body(c), Vec::new(), "TEXT")),
        _ => None,
    }
}

/// Desktop's `Ia`: the text of an MSC1767 text block.
fn poll_text(v: &Value) -> String {
    let m = &v["m.text"];
    let plain = || m.as_array()?.iter().find(|t| t["mimetype"].is_null() || t["mimetype"] == "text/plain");
    s(&v["org.matrix.msc1767.text"])
        .or_else(|| s(m))
        .or_else(|| s(&plain()?["body"]))
        .or_else(|| s(&v["text"]))
        .or_else(|| s(&v["body"]))
        .unwrap_or_default()
}

/// Desktop's `handlePollStart`.
fn poll(c: &Value) -> Option<PollData> {
    let p =
        [&c["org.matrix.msc3381.poll.start"], &c["m.poll.start"], &c["m.poll"]].into_iter().find(|p| p.is_object())?;
    let answers = p["answers"].as_array().map(Vec::as_slice).unwrap_or_default();
    let options = answers.iter().map(|a| PollOption {
        id: s(&a["m.id"]).or(s(&a["id"])).unwrap_or_default(),
        label: poll_text(a),
        voter_ids: Vec::new(),
    });
    Some(PollData {
        question: poll_text(&p["question"]),
        options: options.collect(),
        max_selections: p["max_selections"].as_u64().filter(|&n| n > 0).map_or(1, |n| n.try_into().unwrap_or(u32::MAX)),
        undisclosed: p["kind"].as_str().is_some_and(|k| k.contains("undisclosed")),
        ended: false,
        fallback: poll_text(c),
    })
}

/// Desktop's poll response answers.
fn poll_answers(c: &Value) -> Vec<String> {
    let answers =
        [&c["org.matrix.msc3381.poll.response"]["answers"], &c["m.poll.response"]["answers"], &c["m.selections"]];
    let list = answers.into_iter().find_map(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter().filter_map(|a| a.as_str().map(str::to_string)).collect()
}

fn unresolved_media(c: &Value) -> Option<String> {
    c.get("com.beeper.unresolved_media").filter(|v| !v.is_null()).map(Value::to_string)
}

/// The store operation for a timeline event. With `hidden`, the events that Desktop hides become `HIDDEN` rows.
async fn op(room: &Room, chat_id: &str, account_id: &str, e: &Value, hidden: bool) -> Option<Op> {
    let deleted = e["unsigned"].get("redacted_because").is_some();
    let (c, rel) = (&e["content"], &e["content"]["m.relates_to"]);
    let sender = s(&e["sender"]).expect("event sender");
    let id = s(&e["event_id"]).expect("event ID");
    let kind = s(&e["type"]).expect("event type");
    let hide = || hidden.then(|| (Some(kind.clone()), Vec::new(), "HIDDEN"));
    let poll = matches!(kind.as_str(), "org.matrix.msc3381.poll.start" | "m.poll.start").then(|| poll(c)).flatten();
    let (text, attachments, message_type) = match kind.as_str() {
        "m.room.message" | "m.sticker" | "m.room.encrypted" if deleted => (None, Vec::new(), "TEXT"),
        _ if deleted => return None,
        "m.reaction" => (None, Vec::new(), "REACTION"),
        "m.room.redaction" => return Some(Op::Redact(s(&e["redacts"]).or(s(&c["redacts"]))?)),
        "m.room.message" if rel["rel_type"] == "m.replace" => {
            let (new, ts) = (&c["m.new_content"], e["origin_server_ts"].as_i64().expect("event timestamp"));
            let (text, attachments, message_type) = content("m.room.message", new)?;
            if text.is_none() && attachments.is_empty() {
                return None;
            }
            let timestamp = (c["com.beeper.dont_render_edited"] != true).then(|| rfc3339_from_millis(ts));
            return Some(Op::Edit {
                target: s(&rel["event_id"])?,
                text,
                html: html(new),
                links: links(new),
                attachments,
                message_type,
                unresolved_media: unresolved_media(new),
                timestamp,
            });
        }
        "m.room.encrypted" => (Some(UTD_TEXT.into()), Vec::new(), "TEXT"),
        "org.matrix.msc3381.poll.response" | "m.poll.response" => {
            return Some(Op::Vote(s(&rel["event_id"])?, sender, poll_answers(c)));
        }
        "org.matrix.msc3381.poll.end" | "m.poll.end" => return Some(Op::EndPoll(s(&rel["event_id"])?)),
        _ if poll.is_some() => (poll.as_ref().map(|p| p.question.clone()), Vec::new(), "POLL"),
        "m.room.member" => membership(room, e).await.map(|t| (Some(t), Vec::new(), "MEMBERSHIP")).or_else(hide)?,
        k => content(k, c).or_else(hide)?,
    };
    let profile_name = s(&c["com.beeper.per_message_profile"]["displayname"]);
    let user = matrix_sdk::ruma::UserId::parse(&sender).expect("sender is a user ID");
    let member = room.get_member_no_sync(&user).await.expect("read room member");
    let order = e["unsigned"]["com.beeper.hs.order"].as_i64().expect("Beeper events carry com.beeper.hs.order");
    let reaction = (message_type == "REACTION").then(|| Reaction {
        id: id.clone(),
        participant_id: sender.clone(),
        reaction_key: s(&rel["key"]).expect("reaction key"),
        animated: None,
        img_url: None,
        sort_key: sort_key(order),
    });
    let msg = Box::new(Message {
        id,
        account_id: account_id.into(),
        chat_id: chat_id.into(),
        sender_name: Some(profile_name.unwrap_or_else(|| member.map_or_else(|| sender.clone(), |m| m.name().into()))),
        is_sender: Some(user == room.own_user_id()),
        sender_id: sender,
        sort_key: sort_key(order),
        timestamp: rfc3339_from_millis(e["origin_server_ts"].as_i64().expect("event timestamp")),
        attachments,
        is_unread: Some(false),
        is_bookmarked: Some(false),
        is_deleted: deleted.then_some(true),
        linked_message_id: s(&rel["m.in_reply_to"]["event_id"]).or(reaction.as_ref().and(s(&rel["event_id"]))),
        reactions: Vec::new(),
        text,
        html: html(c),
        message_type: Some(message_type.into()),
        poll,
        unresolved_media: unresolved_media(c).filter(|_| !deleted),
        edit_history: Vec::new(),
        is_edited: None,
        links: links(c),
        seen: Default::default(),
        flags: 0,
    });
    match reaction {
        Some(r) => Some(Op::React(msg.linked_message_id.clone()?, r, msg)),
        None => Some(Op::Upsert(msg)),
    }
}

/// The timeline text of an `m.room.member` event, or `None` when Desktop hides it.
async fn membership(room: &Room, e: &Value) -> Option<String> {
    let (c, prev) = (&e["content"], &e["unsigned"]["prev_content"]);
    let (now, before) = (c["membership"].as_str()?, prev["membership"].as_str().unwrap_or_default());
    let bot = |c: &Value| c["com.beeper.bridge.is_network_bot"] == true;
    if bot(c) || c["com.beeper.exclude_from_timeline"] == true || now == before {
        return None;
    }
    let (target, sender) = (s(&e["state_key"]).expect("member state key"), s(&e["sender"]).expect("event sender"));
    let me = room.own_user_id().as_str();
    if !matches!(now, "join" | "leave" | "ban" | "invite") || target != me && room.joined_members_count() >= 100 {
        return None;
    }
    let name = |id: &str| {
        let user = matrix_sdk::ruma::UserId::parse(id).expect("member is a user ID");
        async move { room.get_member_no_sync(&user).await.expect("read room member").map(|m| m.name().to_string()) }
    };
    let who = match s(&c["displayname"]).or(s(&prev["displayname"])) {
        Some(n) => n,
        None => name(&target).await.unwrap_or_else(|| target.clone()),
    };
    let by = match sender != target && matches!(now, "invite" | "leave" | "ban") {
        true => Some(match state_member(room, &sender).await.is_some_and(|c| bot(&c)) {
            true => None,
            false if sender == me => Some("You".to_string()),
            false => Some(name(&sender).await.unwrap_or(sender)),
        }),
        false => None,
    };
    Some(membership_text(now, before, (&who, target == me), by.as_ref().map(Option::as_deref)))
}

async fn state_member(room: &Room, user: &str) -> Option<Value> {
    let raw = room.get_state_event(StateEventType::RoomMember, user).await.expect("read room member state")?;
    Some(state_json(raw)["content"].take())
}

/// Desktop's text for `who` (name, is me) going from `before` to `now`, changed by `by`: `Some(None)` is a network bot.
fn membership_text(now: &str, before: &str, (name, me): (&str, bool), by: Option<Option<&str>>) -> String {
    let n = if me { "You" } else { name };
    let (a, was, is) = if me { ("you", "were", "are") } else { (n, "was", "is") };
    let r = by.flatten();
    let removed =
        r.map_or_else(|| format!("{n} {was} removed from the chat"), |r| format!("{r} removed {a} from the chat"));
    match (now, before) {
        ("join", _) => format!("{n} joined the chat"),
        ("leave", "ban") => r.map_or_else(|| format!("{n} {was} unbanned"), |r| format!("{r} unbanned {a}")),
        ("leave", "invite") if by.is_none() => format!("{n} declined the invitation"),
        ("leave", "invite") => {
            let whose = if me { "your invitation".to_string() } else { format!("{n}'s invitation") };
            match r {
                Some(r) => format!("{r} withdrew {whose}"),
                None if me => "Your invitation was withdrawn".into(),
                None => format!("{whose} was withdrawn"),
            }
        }
        ("leave", "join") if by.is_none() => format!("{n} left the chat"),
        ("leave" | "ban", "join") => removed,
        ("leave", _) => format!("{n} {is} no longer in the chat"),
        ("ban", _) => r.map_or_else(|| format!("{n} {was} banned"), |r| format!("{r} banned {a}")),
        ("invite", _) => {
            r.map_or_else(|| format!("{n} {was} invited to the chat"), |r| format!("{r} invited {a} to the chat"))
        }
        _ => unreachable!("membership {now}"),
    }
}

pub fn link(p: &Value) -> Option<MessageLink> {
    Some(MessageLink {
        url: s(&p["matched_url"]).or(s(&p["og:url"]))?,
        title: s(&p["og:title"]).unwrap_or_default(),
        summary: s(&p["og:description"]),
    })
}

pub fn reaction_text(key: &str, target: Option<&Message>, who: Option<&str>) -> String {
    let whose = if target.is_some_and(|t| t.is_sender == Some(true)) { "your" } else { "a" };
    let plain = |t: &Message| {
        t.html.as_deref().map(crate::codec::html_text).or_else(|| t.text.as_deref().map(crate::codec::markdown_links))
    };
    let object = match target.map(|t| (plain(t).filter(|x| !x.is_empty()), t.attachments.first())) {
        Some((Some(text), _)) => format!("\"{text}\""),
        Some((None, Some(a))) => match a.attachment_type.as_str() {
            "img" => format!("{whose} photo"),
            "audio" => format!("{whose} voice note"),
            "unknown" => format!("{whose} file"),
            other => format!("{whose} {other}"),
        },
        _ => "a message".into(),
    };
    let verb = reaction_verb(key);
    who.map_or_else(|| format!("{verb} {object}"), |w| format!("{w} {verb} {object}"))
}

fn reaction_verb(key: &str) -> String {
    match key.trim().replace('\u{fe0f}', "").as_str() {
        "\u{2764}" | "\u{2665}" => "loved".into(),
        "👍" => "liked".into(),
        "👎" => "disliked".into(),
        "😂" | "😹" => "laughed at".into(),
        "❓" | "🤔" => "questioned".into(),
        "❗" | "\u{203c}" => "emphasized".into(),
        k if ["haha", "laugh"].contains(&k.to_lowercase().as_str()) => "laughed at".into(),
        k if k.starts_with("mxc://") || k.starts_with("localmxc://") => "reacted to".into(),
        _ => format!("reacted {} to", key.trim()),
    }
}

/// JavaScript's `Number.MIN_SAFE_INTEGER`, the order Beeper gives a chat with no events.
const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

async fn history(room: &Room) -> Vec<TimelineEvent> {
    let mut opts = MessagesOptions::backward();
    opts.limit = HISTORY.into();
    let mut chunk = room.messages(opts).await.expect("load room history").chunk;
    chunk.reverse();
    chunk
}

async fn set_typing(store: &SharedStore, room: &Room, ids: &Value) {
    let me = room.own_user_id().as_str();
    let ids: Vec<String> = ids.as_array().expect("typing user IDs").iter().map(|i| s(i).expect("a user ID")).collect();
    let ids: Vec<String> = ids.into_iter().filter(|i| i != me).collect();
    let chat = chat_id(room).await;
    let mut st = store.write();
    let old = if ids.is_empty() { st.typing.remove(&chat) } else { st.typing.insert(chat.clone(), ids.clone()) };
    if old.unwrap_or_default() != ids {
        st.emit_ws("chat.typing", &chat, None, None, None);
    }
}

async fn sync_receipts(store: &SharedStore, room: &Room) {
    let chat = chat_id(room).await;
    let peers: Vec<String> = {
        let st = store.read();
        let Some(&idx) = st.chat_idx_by_id.get(&chat) else { return };
        st.chats[idx].participants.items.iter().filter(|u| u.is_self != Some(true)).map(|u| u.id.clone()).collect()
    };
    let mut updates = Vec::new();
    for uid in peers {
        let Ok(parsed) = matrix_sdk::ruma::UserId::parse(&uid) else { continue };
        let receipt = room.load_user_receipt(ReceiptType::Read, &ReceiptThread::Unthreaded, &parsed).await;
        if let Some((ev, r)) = receipt.expect("read receipt") {
            let ts = r.ts.map_or_else(now_rfc3339_millis, |t| rfc3339_from_millis(i64::from(t.0)));
            updates.push((uid, ev.to_string(), ts));
        }
    }
    if updates.is_empty() {
        return;
    }
    let mut st = store.write();
    let indices = st.chat_msg_indices.get(&chat).cloned().unwrap_or_default();
    for (uid, ev, ts) in updates {
        let target = indices.iter().copied().find(|&i| {
            let m = &st.messages[i];
            m.id == ev || m.reactions.iter().any(|r| r.id == ev)
        });
        let Some(target) = target else { continue };
        for &i in &indices {
            if i != target && st.messages[i].seen.contains_key(&uid) {
                st.update_message(i, |m| {
                    m.seen.remove(&uid);
                });
            }
        }
        if st.messages[target].seen.get(&uid) != Some(&ts) {
            st.update_message(target, |m| {
                m.seen.insert(uid, ts);
            });
        }
    }
}

/// Desktop's label rooms: the name, the `com.beeper.label.*` state and the `m.space.child` chats.
async fn sync_label(store: &SharedStore, room: &Room) {
    let get = async |kind: &str| state(room, kind).await.unwrap_or(Value::Null);
    let children = states(room, StateEventType::SpaceChild).await.into_iter();
    let joined = children.filter(|v| v["content"]["via"].as_array().is_some_and(|via| !via.is_empty()));
    let label = Label {
        id: room.room_id().to_string(),
        name: room.name().unwrap_or_default(),
        color: get("com.beeper.label.color").await["color_index"].as_u64().and_then(|n| u8::try_from(n).ok()),
        icon: s(&get("com.beeper.label.icon").await["icon"]),
        muted: get("com.beeper.muted").await["muted"] == true,
        inbox: match get("com.beeper.label.inbox_visibility").await["visibility"].as_str() {
            Some("dmOnly") => Some("dmOnly"),
            Some("none") => Some("none"),
            _ => None,
        },
        rooms: joined.filter_map(|v| s(&v["state_key"])).collect(),
    };
    store.write().set_label(label);
}

/// Desktop's `createLabel` and `addRoomToLabel`.
async fn create_label(store: &SharedStore, client: &Client, label: Label) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::room::create_room::v3::{Request, RoomPreset};
    fn raw<T>(v: Value) -> Raw<T> {
        Raw::from_json(serde_json::value::to_raw_value(&v).expect("label JSON"))
    }
    let event =
        |kind: &str, content: Value| raw(serde_json::json!({ "type": kind, "state_key": "", "content": content }));
    let mut request = Request::new();
    request.creation_content = Some(raw(serde_json::json!({ "type": "m.space", "com.beeper.label": true })));
    request.initial_state = std::iter::once(event("m.room.name", serde_json::json!({ "name": label.name })))
        .chain(label.color.map(|c| event("com.beeper.label.color", serde_json::json!({ "color_index": c }))))
        .chain(label.icon.as_ref().map(|i| event("com.beeper.label.icon", serde_json::json!({ "icon": i }))))
        .collect();
    request.preset = Some(RoomPreset::PrivateChat);
    let room = client.create_room(request).await.map_err(|e| e.to_string())?;
    for chat in &label.rooms {
        let via = serde_json::json!({ "via": ["beeper.com"] });
        room.send_state_event_raw("m.space.child", chat, via).await.map_err(|e| e.to_string())?;
    }
    store.write().created_label(&label.id, room.room_id().as_str());
    Ok(())
}

async fn sync_space(store: &SharedStore, room: &Room) {
    let bridge = state(room, "m.bridge").await.unwrap_or(Value::Null);
    // A Slack login is one workspace, so the workspace space is the account itself.
    if bridge["com.beeper.bridge_name"] == "slackgo" {
        return;
    }
    let (Some(id), Some(name)) = (s(&bridge["network"]["id"]), s(&bridge["network"]["displayname"])) else { return };
    let mut st = store.write();
    let space = NetworkSpace { id, account_id: account_id(&bridge, &st.bridge_accounts), name };
    match st.network_spaces.iter().position(|n| n.id == space.id) {
        Some(i) => st.network_spaces[i] = space,
        None => st.network_spaces.push(space),
    }
}

async fn sync_room(store: &SharedStore, room: &Room, mut events: Vec<TimelineEvent>) {
    let create = state(room, "m.room.create").await.expect("rooms have a create event");
    if create["com.beeper.label"] == true {
        return sync_label(store, room).await;
    }
    if create["type"].is_string() {
        return sync_space(store, room).await;
    }
    if state(room, "com.beeper.managed").await.is_some() {
        return;
    }
    let chat_id = chat_id(room).await;
    let note = chat_id == NOTE_CHAT_ID;
    let accounts = store.read().bridge_accounts.clone();
    let rules = store.read().inbox_rules.clone();
    let pins = global(&room.client(), "com.beeper.favourites_order").await;
    let rank = pins["Inbox"].as_array().and_then(|ids| ids.iter().position(|id| id == room.room_id().as_str()));
    let chat = chat(room, &accounts, &rules).await;
    let seen = {
        let st = store.read();
        st.chat_idx_by_id.get(&chat_id).is_some_and(|&i| st.chats[i].notifiable.is_some())
    };
    if !seen {
        let known: HashSet<String> = events.iter().filter_map(|e| Some(e.event_id()?.to_string())).collect();
        let older =
            history(room).await.into_iter().filter(|e| e.event_id().is_some_and(|id| !known.contains(id.as_str())));
        events = older.chain(events).collect();
    }
    let (mut ops, mut non_messages) = (Vec::new(), Vec::new());
    let hidden = store.read().settings.on("hiddenEvents");
    for e in &events {
        let v: Value = serde_json::from_str(e.raw().json().get()).expect("timeline event JSON");
        ops.extend(s(&v["unsigned"]["transaction_id"]).map(Op::Remove));
        match op(room, &chat_id, &chat.account_id, &v, hidden).await {
            Some(Op::Upsert(m)) => ops.push(Op::Upsert(m)),
            o => {
                non_messages.extend(s(&v["event_id"]));
                ops.extend(o);
            }
        }
    }
    let done = account_data(room, "com.beeper.inbox.done").await.and_then(|d| d["at_order"].as_i64());
    let reacted =
        ops.iter().filter_map(|o| if let Op::React(_, _, m) = o { Some(m.sort_key.clone()) } else { None }).max();
    let redacted = ops.iter().any(|o| matches!(o, Op::Redact(_)));
    let (members, group) = (chat.participants.total, chat.chat_type == "group");
    {
        let mut st = store.write();
        let old = match rank {
            Some(r) => st.pin_rank.insert(chat_id.clone(), r),
            None => st.pin_rank.remove(&chat_id),
        };
        if old != rank {
            st.persist_state_to_sqlite();
        }
        if !note {
            st.upsert_chat(chat);
        } else {
            let i = st.chat_idx_by_id[NOTE_CHAT_ID];
            let room = |c: &Chat| {
                (
                    c.img_url.clone(),
                    c.is_muted,
                    c.is_pinned,
                    c.is_low_priority,
                    (c.unread_count, c.unread_mentions),
                    c.is_marked_unread,
                    c.reminder.clone(),
                )
            };
            if room(&st.chats[i]) != room(&chat) {
                st.update_chat(i, |c| {
                    (
                        c.img_url,
                        c.is_muted,
                        c.is_pinned,
                        c.is_low_priority,
                        (c.unread_count, c.unread_mentions),
                        c.is_marked_unread,
                        c.reminder,
                    ) = room(&chat);
                });
            }
        }
        for id in non_messages {
            if st.msg_idx_by_id.contains_key(&id) {
                st.remove_message(&chat_id, &id).expect("remove a decrypted placeholder");
            }
        }
        for op in ops {
            match op {
                Op::Upsert(m) => st.upsert_message(*m),
                Op::Vote(target, voter, answers) => st.update_poll(&target, |p| p.respond(&voter, &answers)),
                Op::EndPoll(target) => st.update_poll(&target, |p| p.ended = true),
                Op::React(target, r, mut preview) => {
                    let target_msg = st.msg_idx_by_id.get(&target).map(|&i| &st.messages[i]);
                    let target_mine = target_msg.is_some_and(|m| m.is_sender == Some(true));
                    let who = match preview.is_sender == Some(true) {
                        true => Some("You"),
                        false => group.then(|| preview.sender_name.as_deref().expect("sender name")),
                    };
                    preview.text = Some(reaction_text(&r.reaction_key, target_msg, who));
                    if let Some(&i) = st.msg_idx_by_id.get(&target) {
                        st.update_message(i, |m| {
                            m.reactions.retain(|x| x.id != r.id);
                            m.reactions.push(r);
                        });
                    }
                    let idx = st.chat_idx_by_id[&chat_id];
                    let shown = preview.is_sender == Some(true) || (members <= 20 && (!group || target_mine));
                    let newer = st.chats[idx]
                        .preview
                        .as_ref()
                        .is_none_or(|p| p.sort_key < preview.sort_key || p.id == preview.id);
                    if shown && newer {
                        st.update_chat(idx, |c| {
                            c.last_activity = Some(preview.timestamp.clone());
                            c.preview = Some(*preview);
                        });
                    }
                }
                Op::Edit { target, text, html, links, attachments, message_type, unresolved_media, timestamp } => {
                    if let Some(&i) = st.msg_idx_by_id.get(&target) {
                        st.update_message(i, |m| {
                            let prev = match text {
                                Some(t) => m.text.replace(t).unwrap_or_default(),
                                None => m.text.take().unwrap_or_default(),
                            };
                            if let Some(timestamp) =
                                timestamp.filter(|t| m.edit_history.last().is_none_or(|e| e.timestamp != *t))
                            {
                                m.edit_history.push(EditHistoryEntry { text: prev, timestamp });
                            }
                            if !attachments.is_empty() {
                                m.attachments = attachments;
                                m.message_type = Some(message_type.into());
                            }
                            m.unresolved_media = unresolved_media;
                            m.links = links;
                            m.html = html;
                        });
                    }
                }
                Op::Remove(id) => {
                    if st.msg_idx_by_id.get(&id).is_some_and(|&i| st.messages[i].chat_id == chat_id) {
                        st.remove_message(&chat_id, &id).expect("remove a local echo");
                    } else if let Some(i) = st
                        .chat_msg_indices
                        .get(&chat_id)
                        .and_then(|v| v.iter().copied().find(|&i| st.messages[i].reactions.iter().any(|r| r.id == id)))
                    {
                        st.update_message(i, |m| m.reactions.retain(|r| r.id != id));
                    }
                }
                Op::Redact(target) => {
                    if st.msg_idx_by_id.contains_key(&target) {
                        st.delete_message(&chat_id, &target).expect("redact a known message");
                    } else {
                        let i = st.messages.iter().position(|m| m.reactions.iter().any(|r| r.id == target));
                        if let Some(i) = i {
                            st.update_message(i, |m| m.reactions.retain(|r| r.id != target));
                        }
                    }
                }
            }
        }
        let idx = st.chat_idx_by_id[&chat_id];
        let msgs = st.chat_msg_indices.get(&chat_id).map_or(&[][..], |v| &v[..]).iter().map(|&i| &st.messages[i]);
        let last = msgs.clone().rev().find(|m| m.message_type.as_deref() != Some("MEMBERSHIP")).map(|m| &m.sort_key);
        let stored = msgs.flat_map(|m| &m.reactions).map(|r| &r.sort_key).max();
        let old = st.chats[idx].notifiable.as_ref().filter(|_| !redacted);
        let notifiable = [old, last, reacted.as_ref(), stored].into_iter().flatten().max().cloned();
        let notifiable = Some(notifiable.unwrap_or_else(|| sort_key(MIN_SAFE_INTEGER)));
        let archived = st.chats[idx].is_pinned != Some(true) && done.is_some_and(|d| Some(sort_key(d)) >= notifiable);
        if (st.chats[idx].is_archived, &st.chats[idx].notifiable) != (Some(archived), &notifiable) {
            st.update_chat(idx, |c| (c.is_archived, c.notifiable) = (Some(archived), notifiable));
        }
    }
    sync_receipts(store, room).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFlag {
    Archived,
    Pinned,
    LowPriority,
    Muted,
    MarkedUnread,
}

pub enum Outgoing {
    Send {
        txn: String,
        text: String,
        reply_to: Option<String>,
        /// A link preview with the `og:image:type` of its image.
        preview: Option<crate::cloud::PagePreview>,
    },
    Edit {
        target: String,
        text: String,
    },
    Redact {
        target: String,
    },
    React {
        txn: String,
        target: String,
        key: String,
    },
    /// An MSC3381 poll start, like Desktop's `sendPoll`.
    Poll {
        txn: String,
        poll: PollData,
    },
    /// An MSC3381 poll end, like Desktop's `sendPollEnd`.
    EndPoll {
        target: String,
    },
    /// An MSC3381 poll response with the answer IDs.
    Vote {
        target: String,
        answers: Vec<String>,
    },
    /// A label space with its chats as children, like Desktop's `createLabel`. Its chat is the label ID here.
    CreateLabel(Label),
    /// The name, color and icon of a label, with the chats to add (true) or remove (false), like Desktop's `updateLabel`.
    EditLabel(Label, Vec<(String, bool)>),
    /// Leaves and forgets the room, like Desktop's `deleteLabel`.
    Leave,
    /// A state event with an empty state key.
    State(&'static str, Value),
    /// Uploads an image with its MIME type and makes it the room avatar, or removes the avatar.
    Avatar(Option<(Vec<u8>, String)>),
    /// Adds (true) or removes a chat from a label.
    LabelChild(String, bool),
    Read(String),
    Flag(ChatFlag, bool),
    Reminder(Option<ChatReminder>),
    DeleteChat {
        for_everyone: bool,
    },
    /// Send the content of `event` from the `from` chat again, with the current `text` and `html`.
    Forward {
        txn: String,
        from: String,
        event: String,
        text: Option<String>,
        html: Option<String>,
    },
    Media {
        txn: String,
        bytes: Vec<u8>,
        name: String,
        mime: String,
        info: MediaInfo,
        caption: Option<String>,
        reply_to: Option<String>,
    },
}

pub enum MediaInfo {
    /// The width and height of an image.
    Image(u32, u32),
    /// A recorded voice note: its length and its waveform, with values from 0 to 1.
    Voice { ms: u64, wave: Vec<f32> },
    /// An other file, which Matrix shows by its MIME type.
    File,
}

pub async fn flush(store: SharedStore) -> Result<(), String> {
    loop {
        let next = {
            let mut st = store.write();
            if st.matrix.is_none() || st.outbox.is_empty() {
                st.pushing = false;
                return Ok(());
            }
            st.outbox.remove(0)
        };
        if let Err(e) = push(&store, next.0, next.1).await {
            store.write().pushing = false;
            return Err(e);
        }
    }
}

async fn joined_room(client: &matrix_sdk::Client, chat: &str) -> Result<Room, String> {
    for room in client.joined_rooms() {
        if room.room_id() == chat || chat == NOTE_CHAT_ID && chat_id(&room).await == chat {
            return Ok(room);
        }
    }
    Err(format!("{chat} is not a joined Matrix room"))
}

pub async fn join(client: Option<matrix_sdk::Client>, room: &'static str) -> Result<(), String> {
    let room = matrix_sdk::ruma::RoomOrAliasId::parse(room).expect("a valid room ID");
    let client = client.ok_or("Not connected to Matrix")?;
    client.join_room_by_id_or_alias(&room, &[]).await.map(drop).map_err(|e| e.to_string())
}

pub async fn typing(client: matrix_sdk::Client, chat: String, on: bool) -> Result<(), String> {
    joined_room(&client, &chat).await?.typing_notice(on).await.map_err(|e| e.to_string())
}

fn text_content(text: String) -> RoomMessageEventContent {
    match crate::codec::markdown_html(&text) {
        Some(html) => RoomMessageEventContent::text_html(text, html),
        None => RoomMessageEventContent::text_plain(text),
    }
}

async fn push(store: &SharedStore, chat: String, out: Outgoing) -> Result<(), String> {
    let client = store.read().matrix.clone().expect("flush checks the Matrix client");
    if let Outgoing::CreateLabel(label) = out {
        return create_label(store, &client, label).await;
    }
    let chat = store.read().sent.get(&chat).cloned().unwrap_or(chat);
    let room = joined_room(&client, &chat).await?;
    let event = |id: String| {
        let id = store.read().sent.get(&id).cloned().unwrap_or(id);
        OwnedEventId::try_from(id).map_err(|e| e.to_string())
    };
    let e = |x: matrix_sdk::Error| x.to_string();
    let sent = |txn: String, id: OwnedEventId| store.write().sent.insert(txn, id.to_string());
    match out {
        Outgoing::Send { txn, text, reply_to, preview } => {
            let mut content = text_content(text);
            content.relates_to =
                reply_to.map(|r| event(r).map(|id| Relation::Reply(Reply::with_event_id(id)))).transpose()?;
            let mut content = serde_json::to_value(content).expect("message JSON");
            if let Some((mut p, image)) = preview {
                if let Some(bytes) = image {
                    if room.latest_encryption_state().await.map_err(e)?.is_encrypted() {
                        let file = client.upload_encrypted_file(&mut bytes.as_slice()).await.map_err(e)?;
                        p["beeper:image:encryption"] = serde_json::to_value(file).expect("encrypted file JSON");
                    } else {
                        let mime: mime::Mime =
                            p["og:image:type"].as_str().expect("an image type").parse().expect("a MIME type");
                        let uri = client.media().upload(&mime, bytes, None).await.map_err(e)?.content_uri;
                        p["og:image"] = uri.to_string().into();
                    }
                }
                content["com.beeper.linkpreviews"] = Value::Array(vec![p]);
            }
            let id = room.send_raw("m.room.message", content).with_transaction_id(txn.as_str().into());
            sent(txn, id.await.map_err(e)?.response.event_id);
        }
        Outgoing::Edit { target, text } => {
            let content = text_content(text).make_replacement(ReplacementMetadata::new(event(target)?, None));
            room.send(content).await.map_err(e)?;
        }
        Outgoing::Redact { target } => {
            room.redact(&event(target)?, None, None).await.map_err(|x| x.to_string())?;
        }
        Outgoing::React { txn, target, key } => {
            let content = ReactionEventContent::new(Annotation::new(event(target)?, key));
            sent(txn.clone(), room.send(content).with_transaction_id(txn.into()).await.map_err(e)?.response.event_id);
        }
        Outgoing::Poll { txn, poll } => {
            let text = "org.matrix.msc1767.text";
            let answers: Vec<Value> =
                poll.options.iter().map(|o| serde_json::json!({ "id": o.id, text: o.label })).collect();
            let kind = if poll.undisclosed { "undisclosed" } else { "disclosed" };
            let content = serde_json::json!({
                text: poll.question,
                "org.matrix.msc3381.poll.start": {
                    "answers": answers,
                    "kind": format!("org.matrix.msc3381.poll.{kind}"),
                    "max_selections": poll.max_selections,
                    "question": { text: poll.question },
                },
            });
            let id = room.send_raw("org.matrix.msc3381.poll.start", content).with_transaction_id(txn.as_str().into());
            sent(txn, id.await.map_err(e)?.response.event_id);
        }
        Outgoing::EndPoll { target } => {
            let content = serde_json::json!({
                "m.relates_to": { "rel_type": "m.reference", "event_id": event(target)?.to_string() },
                "org.matrix.msc3381.poll.end": {},
                "org.matrix.msc1767.text": "Poll ended",
            });
            room.send_raw("org.matrix.msc3381.poll.end", content).await.map_err(e)?;
        }
        Outgoing::Vote { target, answers } => {
            let content = serde_json::json!({
                "m.relates_to": { "rel_type": "m.reference", "event_id": event(target)?.to_string() },
                "org.matrix.msc3381.poll.response": { "answers": answers },
            });
            room.send_raw("org.matrix.msc3381.poll.response", content).await.map_err(e)?;
        }
        Outgoing::Read(target) => {
            room.send_single_receipt(SendReceiptType::Read, ReceiptThread::Unthreaded, event(target)?)
                .await
                .map_err(e)?;
        }
        Outgoing::Flag(flag, on) => match flag {
            ChatFlag::Archived => {
                let now = now_unix_millis();
                let mut done = serde_json::json!({ "at_ts": now, "updated_ts": now });
                if on {
                    let st = store.read();
                    let last = st.chat_msg_indices.get(&chat).and_then(|v| v.last()).map(|&i| &st.messages[i].sort_key);
                    let last = last.max(st.chats[st.chat_idx_by_id[&chat]].notifiable.as_ref());
                    let order = last.map_or(MIN_SAFE_INTEGER, |k| k.parse::<i64>().expect("numeric sort key"));
                    done["at_order"] = order.into();
                }
                let raw = Raw::from_json(serde_json::value::to_raw_value(&done).expect("inbox done JSON"));
                room.set_account_data_raw("com.beeper.inbox.done".into(), raw).await.map_err(e)?;
            }
            ChatFlag::Pinned => {
                tag(&room, TagName::Favorite, on).await.map_err(e)?;
                edit_pins(&client, room.room_id().as_str(), on).await.map_err(e)?;
            }
            ChatFlag::LowPriority => tag(&room, TagName::LowPriority, on).await.map_err(e)?,
            ChatFlag::Muted => {
                let settings = client.notification_settings().await;
                match on {
                    true => {
                        let mode = RoomNotificationMode::MentionsAndKeywordsOnly;
                        settings.set_room_notification_mode(room.room_id(), mode).await
                    }
                    false => settings.delete_user_defined_room_rules(room.room_id()).await,
                }
                .map_err(|x| x.to_string())?;
            }
            ChatFlag::MarkedUnread => room.set_unread_flag(on).await.map_err(e)?,
        },
        Outgoing::Reminder(at) => {
            let v = at.map_or(serde_json::json!({}), |r| {
                serde_json::json!({ "remind_at_ms": r.remind_at_ms, "dismiss_on_incoming_message": r.dismiss_on_incoming_message })
            });
            let raw = Raw::from_json(serde_json::value::to_raw_value(&v).expect("reminder JSON"));
            room.set_account_data_raw(REMINDER.into(), raw).await.map_err(e)?;
        }
        Outgoing::DeleteChat { for_everyone } => {
            let content = serde_json::json!({ "delete_for_everyone": for_everyone });
            room.send_raw("com.beeper.delete_chat", content).await.map_err(e)?;
        }
        Outgoing::Forward { txn, from, event: id, text, html } => {
            let source = joined_room(&client, &from).await?.event(&event(id)?, None).await.map_err(e)?;
            let mut ev: Value = source.raw().deserialize_as().map_err(|x| x.to_string())?;
            let kind = ev["type"].as_str().expect("an event type").to_string();
            assert!(matches!(kind.as_str(), "m.room.message" | "m.sticker"), "cannot forward a {kind} event");
            let c = ev["content"].as_object_mut().expect("event content");
            c.remove("m.relates_to");
            c.remove("m.mentions");
            if matches!(c["msgtype"].as_str(), Some("m.text" | "m.notice" | "m.emote")) {
                c.insert("body".into(), text.expect("a text message has text").into());
                c.remove("format");
                c.remove("formatted_body");
                if let Some(h) = html {
                    c.insert("format".into(), "org.matrix.custom.html".into());
                    c.insert("formatted_body".into(), h.into());
                }
            }
            let sent_id = room.send_raw(&kind, ev["content"].take()).with_transaction_id(txn.as_str().into());
            sent(txn, sent_id.await.map_err(e)?.response.event_id);
        }
        Outgoing::Media { txn, bytes: data, name, mime, info, caption, reply_to } => {
            use matrix_sdk::attachment::{
                AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo,
            };
            use matrix_sdk::room::reply::{EnforceThread, Reply};
            let reply = reply_to
                .map(|r| {
                    event(r).map(|event_id| Reply {
                        event_id,
                        enforce_thread: EnforceThread::MaybeThreaded,
                        add_mentions: matrix_sdk::ruma::events::room::message::AddMentions::Yes,
                    })
                })
                .transpose()?;
            let size = Some(matrix_sdk::ruma::UInt::try_from(data.len()).expect("the media size fits"));
            let info = match info {
                MediaInfo::Image(w, h) => AttachmentInfo::Image(BaseImageInfo {
                    width: Some(w.into()),
                    height: Some(h.into()),
                    size,
                    is_animated: Some(mime == "image/gif"),
                    ..Default::default()
                }),
                MediaInfo::Voice { ms, wave } => AttachmentInfo::Voice(BaseAudioInfo {
                    duration: Some(std::time::Duration::from_millis(ms)),
                    size,
                    waveform: Some(wave),
                }),
                MediaInfo::File => AttachmentInfo::File(BaseFileInfo { size }),
            };
            let caption = caption.map(matrix_sdk::ruma::events::room::message::TextMessageEventContent::plain);
            let config = AttachmentConfig::new().txn_id(txn.clone().into()).info(info).caption(caption).reply(reply);
            let parsed_mime = mime.parse().expect("valid media mime");
            sent(txn, room.send_attachment(&name, &parsed_mime, data, config).await.map_err(e)?.event_id);
        }
        Outgoing::CreateLabel(_) => unreachable!("a label has no room before it is created"),
        Outgoing::EditLabel(label, changes) => {
            let state = |kind: &'static str, content: Value| room.send_state_event_raw(kind, "", content);
            state("m.room.name", serde_json::json!({ "name": label.name })).await.map_err(e)?;
            state("com.beeper.label.color", serde_json::json!({ "color_index": label.color })).await.map_err(e)?;
            state("com.beeper.label.icon", serde_json::json!({ "icon": label.icon })).await.map_err(e)?;
            for (chat, add) in changes {
                let content = if add { serde_json::json!({ "via": ["beeper.com"] }) } else { serde_json::json!({}) };
                room.send_state_event_raw("m.space.child", &chat, content).await.map_err(e)?;
            }
        }
        Outgoing::State(kind, content) => drop(room.send_state_event_raw(kind, "", content).await.map_err(e)?),
        Outgoing::Avatar(image) => {
            let content = match image {
                Some((bytes, mime)) => {
                    let mime: mime::Mime = mime.parse().expect("an image MIME type");
                    serde_json::json!({ "url": client.media().upload(&mime, bytes, None).await.map_err(e)?.content_uri })
                }
                None => serde_json::json!({}),
            };
            room.send_state_event_raw("m.room.avatar", "", content).await.map_err(e)?;
        }
        Outgoing::LabelChild(chat, add) => {
            let content = if add { serde_json::json!({ "via": ["beeper.com"] }) } else { serde_json::json!({}) };
            room.send_state_event_raw("m.space.child", &chat, content).await.map_err(e)?;
        }
        Outgoing::Leave => {
            room.leave().await.map_err(e)?;
            if let Some(left) = client.get_room(room.room_id()) {
                let _ = left.forget().await;
            }
        }
    }
    Ok(())
}

const ENCRYPTED_FILE: &str = "encryptedFileInfoJSON";

/// The bytes of a Matrix (`mxc://`) or web (`https://`) media URL.
pub async fn media(client: Option<Client>, src: String) -> Result<Vec<u8>, String> {
    if src.starts_with("https://") {
        return crate::cloud::http_bytes(&src).await;
    }
    let client = client.ok_or("Not connected to Matrix")?;
    let source = match src.split_once(&format!("?{ENCRYPTED_FILE}=")) {
        Some((_, json)) => {
            let json = crate::codec::url_decode(json).ok_or("bad encrypted file info")?;
            let json = match json.starts_with('{') {
                true => json.into_bytes(),
                false => vodozemac::base64_decode(json.trim_end_matches('=').replace('-', "+").replace('_', "/"))
                    .map_err(|e| e.to_string())?,
            };
            MediaSource::Encrypted(Box::new(serde_json::from_slice(&json).map_err(|e| e.to_string())?))
        }
        None => MediaSource::Plain(src.into()),
    };
    let request = MediaRequestParameters { source, format: MediaFormat::File };
    client.media().get_media_content(&request, true).await.map_err(|e| e.to_string())
}

/// The session and bridge login that a bridge provisioning request acts as.
pub struct Provision {
    pub homeserver: String,
    pub access_token: String,
    pub user_id: String,
    pub bridge_id: String,
    pub login_id: String,
}

impl Provision {
    fn url(&self, path: &str) -> String {
        let enc = crate::codec::url_encode;
        format!(
            "{}/_matrix/client/unstable/com.beeper.bridge/{}/_matrix/provision/v3/{path}?user_id={}&login_id={}",
            self.homeserver.trim_end_matches('/'),
            enc(&self.bridge_id),
            enc(&self.user_id),
            enc(&self.login_id),
        )
    }

    async fn post(self, path: &str, body: String) -> Result<String, String> {
        let resp = reqwest::Client::new()
            .post(self.url(path))
            .bearer_auth(self.access_token)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())
    }
}

impl Provision {
    async fn get(&self, path: &str) -> Result<Option<Value>, String> {
        let resp = reqwest::Client::new().get(self.url(path)).bearer_auth(&self.access_token).send().await;
        let resp = resp.map_err(|e| e.to_string())?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = resp.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&body).map(Some).map_err(|e| e.to_string())
    }
}

/// The bridge's provisioning capabilities: `resolve_identifier` and `group_creation`.
pub async fn capabilities(p: Provision) -> Result<Value, String> {
    p.get("capabilities").await?.ok_or_else(|| "the bridge has no provisioning capabilities".into())
}

/// The bridge's `search_users`: the network users that match a name.
pub async fn search_users(p: Provision, query: String) -> Result<Vec<Value>, String> {
    let body = p.post("search_users", serde_json::json!({ "query": query }).to_string()).await?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(v["results"].as_array().cloned().unwrap_or_default())
}

/// The bridge's `resolve_identifier`: the network user of a phone number, email or username, if there is one.
pub async fn resolve_identifier(p: Provision, identifier: String) -> Result<Option<Value>, String> {
    p.get(&format!("resolve_identifier/{}", crate::codec::url_encode(&identifier))).await
}

pub async fn resolve_media(p: Provision, event_id: String, raw: String) -> Result<(), String> {
    p.post(&format!("resolve_media/{}", crate::codec::url_encode(&event_id)), raw).await.map(drop)
}

/// The bridge's `create_dm`: the room of the direct chat with `identifier`, which the bridge makes when it is missing.
pub async fn create_dm(p: Provision, identifier: String) -> Result<String, String> {
    let body = p.post(&format!("create_dm/{}", crate::codec::url_encode(&identifier)), "{}".into()).await?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v["dm_room_mxid"].as_str().map(str::to_string).ok_or_else(|| "the bridge sent no dm_room_mxid".into())
}

pub async fn thumbnail(client: Client, url: String) -> Result<Vec<u8>, String> {
    let size = matrix_sdk::ruma::uint!(96);
    let format = MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(Method::Crop, size, size));
    let request = MediaRequestParameters { source: MediaSource::Plain(url.into()), format };
    client.media().get_media_content(&request, true).await.map_err(|e| e.to_string())
}

async fn connect(store: &SharedStore) -> Result<Client, String> {
    let (s, dir) = {
        let st = store.read();
        (st.beeper_state.clone(), st.matrix_dir())
    };
    let encryption = EncryptionSettings {
        auto_enable_cross_signing: false,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
        auto_enable_backups: true,
    };
    let client = Client::builder()
        .homeserver_url(&s.homeserver)
        .sqlite_store(dir, None)
        .with_encryption_settings(encryption)
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let meta = SessionMeta {
        user_id: s.user_id.parse().map_err(|e| format!("bad Matrix user ID {}: {e}", s.user_id))?,
        device_id: s.device_id.into(),
    };
    let tokens = SessionTokens { access_token: s.access_token, refresh_token: None };
    client.restore_session(MatrixSession { meta, tokens }).await.map_err(|e| e.to_string())?;
    Ok(client)
}

async fn chat_id(room: &Room) -> String {
    let note = state(room, "com.beeper.rooms.note_to_self").await.is_some();
    if note {
        NOTE_CHAT_ID.into()
    } else {
        room.room_id().to_string()
    }
}

async fn has_utd(store: &SharedStore, room: &Room) -> bool {
    let chat = chat_id(room).await;
    let st = store.read();
    st.chat_msg_indices.get(&chat).into_iter().flatten().any(|&i| st.messages[i].text.as_deref() == Some(UTD_TEXT))
}

async fn retry_undecrypted(client: Client, store: SharedStore) {
    let mut keys = client.encryption().room_keys_received_stream().await.expect("room key stream");
    while let Some(first) = keys.next().await {
        let (mut rooms, mut lagged) = (HashSet::new(), false);
        let window = std::time::Duration::from_millis(300);
        let mut batch = Some(first);
        while let Some(b) = batch {
            match b {
                Ok(infos) => rooms.extend(infos.into_iter().map(|k| k.room_id)),
                Err(_) => lagged = true,
            }
            batch = tokio::time::timeout(window, keys.next()).await.ok().flatten();
        }
        let rooms =
            if lagged { client.joined_rooms() } else { rooms.iter().filter_map(|id| client.get_room(id)).collect() };
        refetch_undecrypted(&store, rooms).await;
    }
}

async fn refetch_undecrypted(store: &SharedStore, rooms: Vec<Room>) {
    futures_util::stream::iter(rooms)
        .for_each_concurrent(8, |room| async move {
            if has_utd(store, &room).await {
                sync_room(store, &room, history(&room).await).await;
            }
        })
        .await;
}

async fn global(client: &Client, kind: &str) -> Value {
    let raw =
        client.account().account_data_raw(GlobalAccountDataEventType::from(kind)).await.expect("read account data");
    raw.map_or(Value::Null, |r| r.deserialize_as().expect("account data JSON"))
}

/// Applies a change of the inbox rules to every room.
async fn sync_rules(client: &Client, store: &SharedStore) {
    let rules = global(client, RULES).await;
    if std::mem::replace(&mut store.write().inbox_rules, rules.clone()) != rules {
        let rooms = futures_util::stream::iter(client.joined_rooms());
        rooms.for_each_concurrent(8, |room| async move { sync_room(store, &room, Vec::new()).await }).await;
    }
}

/// The recent emoji, the skin tone and the feature flags of the global account data.
async fn sync_account_data(client: &Client, store: &SharedStore) {
    let recent = global(client, "io.element.recent_emoji").await;
    let recent = recent["recent_emoji"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| (e[0].as_str().expect("recent emoji key").into(), e[1].as_u64().expect("recent emoji count")));
    let recent = recent.collect();
    let tone = global(client, "com.beeper.emoji_skin_tone").await["tone"].as_u64().map_or(0, |t| t as u8);
    let (flags, usage) = (global(client, crate::features::FLAGS).await, global(client, crate::features::USAGE).await);
    let mut st = store.write();
    st.recent_emoji = recent;
    st.skin_tone = tone;
    (st.features.flags, st.features.usage) = (flags, usage);
}

/// Desktop's `userProfileStore`: the own name and avatar come from the Matrix profile.
async fn sync_profile(client: &Client, store: &SharedStore) {
    let profile = match client.account().fetch_user_profile().await {
        Ok(r) => r.data,
        Err(e) => return store.write().matrix_error = Some(e.to_string()),
    };
    let field = |k: &str| profile.get(k).and_then(Value::as_str).map(str::to_string);
    let mut st = store.write();
    let i = st.chat_idx_by_id[NOTE_CHAT_ID];
    st.update_chat(i, |c| {
        let me = &mut c.participants.items[0];
        me.full_name = field("displayname").or(me.full_name.take());
        me.img_url = field("avatar_url");
    });
}

/// Applies Desktop's synced preferences to the settings.
async fn sync_prefs(client: &Client, store: &SharedStore) {
    let mut events = std::collections::BTreeMap::<&'static str, Value>::new();
    for (_, ad) in crate::prefs::SYNCED {
        let kind = match *ad {
            Ad::Desktop(..) => crate::prefs::DESKTOP_PREFS,
            Ad::Event(kind, _) => kind,
        };
        if !events.contains_key(kind) {
            events.insert(kind, global(client, kind).await);
        }
    }
    let mut st = store.write();
    let mut settings = st.settings.clone();
    for (key, ad) in crate::prefs::SYNCED {
        let value = match ad {
            Ad::Desktop(k, values) => values.decode(&events[crate::prefs::DESKTOP_PREFS][k]),
            Ad::Event(kind, field) => events[kind][field].as_bool().map(u8::from),
        };
        if let Some(x) = value {
            settings.set_pref(key, x);
        }
    }
    st.synced_prefs = events;
    if settings.prefs != st.settings.prefs {
        st.settings = settings;
        st.persist_state_to_sqlite();
        st.emit_ws("settings.updated", "", None, None, None);
    }
}

/// The account data write that syncs a changed preference to Desktop, if Desktop syncs it and the first sync is done.
pub fn pref_write(st: &mut BeeperStore, key: &str) -> Option<(&'static str, Value)> {
    let x = st.settings.pref(key);
    let (kind, field, value) = match crate::prefs::synced(key)? {
        Ad::Desktop(k, values) => (crate::prefs::DESKTOP_PREFS, k, values.encode(x)),
        Ad::Event(kind, field) => (kind, field, (x == 1).into()),
    };
    let content = st.synced_prefs.get_mut(kind)?;
    if content.is_null() {
        *content = serde_json::json!({});
    }
    content[field] = value;
    Some((kind, content.clone()))
}

pub async fn set_global(client: Client, kind: &'static str, content: Value) -> Result<(), String> {
    let raw = Raw::from_json(serde_json::value::to_raw_value(&content).expect("account data JSON"));
    let kind = GlobalAccountDataEventType::from(kind);
    client.account().set_account_data_raw(kind, raw).await.map(drop).map_err(|e| e.to_string())
}

async fn retry_after_backup(client: Client, store: SharedStore) {
    let mut states = client.encryption().backups().state_stream();
    while let Some(state) = states.next().await {
        if matches!(state, Ok(BackupState::Enabled)) {
            refresh_e2ee(&client, &store).await;
            refetch_undecrypted(&store, client.joined_rooms()).await;
        }
    }
}

async fn refresh_e2ee(client: &Client, store: &SharedStore) {
    let device = client.encryption().get_own_device().await.expect("read the own device");
    let device = device.expect("signed-in client has its own device");
    let can_sign = client.encryption().cross_signing_status().await.is_some_and(|s| s.has_self_signing);
    if can_sign && !device.is_cross_signed_by_owner() {
        if let Err(e) = device.verify().await {
            store.write().matrix_error = Some(format!("Could not sign this device: {e}"));
        }
    }
    let verified = device.is_cross_signed_by_owner();
    let state = (verified, client.encryption().backups().are_enabled().await);
    let mut st = store.write();
    if (st.beeper_state.verified, st.beeper_state.key_backup) != state {
        (st.beeper_state.verified, st.beeper_state.key_backup) = state;
        st.persist_state_to_sqlite();
        st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
        st.emit_ws("setup.updated", "", None, None, None);
    }
}

const DESKTOP_PICKLE_KEY: &[u8] = b"pickle";

fn unpickle_secret(pickle: &str) -> Result<String, String> {
    let raw = vodozemac::base64_decode(pickle).map_err(|e| e.to_string())?;
    let plain =
        vodozemac::hazmat::Cipher::new_pickle(DESKTOP_PICKLE_KEY).decrypt_pickle(&raw).map_err(|e| e.to_string())?;
    String::from_utf8(plain).map_err(|e| e.to_string())
}

pub async fn import_crypto(dir: &std::path::Path, c: &DesktopCrypto) -> Result<(), String> {
    use matrix_sdk::ruma::{DeviceKeyAlgorithm, OwnedRoomId, OwnedUserId};
    use matrix_sdk_crypto::olm::SenderData;
    use matrix_sdk_crypto::olm::{InboundGroupSession, PickledInboundGroupSession, PrivateCrossSigningIdentity};
    use matrix_sdk_crypto::store::types::{BackupDecryptionKey, Changes};
    use matrix_sdk_crypto::store::CryptoStore;
    use matrix_sdk_crypto::types::{EventEncryptionAlgorithm, SigningKey, SigningKeys};
    use vodozemac::{Curve25519PublicKey, Ed25519PublicKey};
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let mut groups = Vec::new();
    for g in &c.group_sessions {
        let pickle = vodozemac::megolm::InboundGroupSession::from_libolm_pickle(&g.pickle, DESKTOP_PICKLE_KEY);
        let signing = Ed25519PublicKey::from_base64(&g.signing_key).map_err(|x| e(&x))?;
        let mut signing_key = SigningKeys::new();
        signing_key.insert(DeviceKeyAlgorithm::Ed25519, SigningKey::Ed25519(signing));
        groups.push(
            InboundGroupSession::from_pickle(PickledInboundGroupSession {
                pickle: pickle.map_err(|x| e(&x))?.pickle(),
                sender_key: Curve25519PublicKey::from_base64(&g.sender_key).map_err(|x| e(&x))?,
                signing_key,
                sender_data: SenderData::default(),
                forwarder_data: None,
                room_id: g.room_id.parse::<OwnedRoomId>().map_err(|x| e(&x))?,
                imported: true,
                backed_up: true,
                history_visibility: None,
                algorithm: EventEncryptionAlgorithm::MegolmV1AesSha2,
                shared_history: g.shared_history,
            })
            .map_err(|x| e(&x))?,
        );
    }
    let secret = |name: &str| {
        let (_, v) = c.secrets.iter().find(|(n, _)| n == name).ok_or(format!("Desktop has no {name} secret"))?;
        unpickle_secret(v)
    };
    let user: OwnedUserId = c.user_id.parse().map_err(|x| e(&x))?;
    let identity = PrivateCrossSigningIdentity::empty(&user);
    let (master, self_signing, user_signing) = (
        secret("m.cross_signing.master")?,
        secret("m.cross_signing.self_signing")?,
        secret("m.cross_signing.user_signing")?,
    );
    identity
        .import_secrets_unchecked(Some(&master), Some(&self_signing), Some(&user_signing))
        .await
        .map_err(|x| e(&x))?;
    let backup = BackupDecryptionKey::from_base64(&secret("m.megolm_backup.v1")?).map_err(|x| e(&x))?;
    let store = matrix_sdk::SqliteCryptoStore::open(dir, None).await.map_err(|x| e(&x))?;
    let changes = Changes {
        private_identity: Some(identity),
        backup_version: Some(c.backup_version.clone()),
        backup_decryption_key: Some(backup),
        inbound_group_sessions: groups,
        ..Default::default()
    };
    store.save_changes(changes).await.map_err(|x| e(&x))
}

pub async fn recover(store: SharedStore, key: String) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    client.encryption().recovery().recover(key.trim()).await.map_err(|e| e.to_string())?;
    set_verification(&store, "verified", None);
    Ok(())
}

fn set_verification(store: &SharedStore, state: &str, sas: Option<VerificationSas>) {
    let mut st = store.write();
    if state == "verified" {
        st.beeper_state.verified = true;
        st.beeper_state.cross_signing = true;
        st.persist_state_to_sqlite();
        st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
    }
    st.setup_status.session.verification = Some(VerificationState {
        verification_id: "matrix".into(),
        direction: "outgoing".into(),
        methods: vec!["sas".into()],
        state: state.into(),
        updated_at: now_rfc3339_millis(),
        sas,
        qr: None,
        other_device: None,
    });
    st.emit_ws("setup.updated", "", None, None, None);
}

#[derive(Clone)]
pub struct DeviceInfo {
    pub name: String,
    /// `device_model` and `device_type` of Beeper's `com.beeper.device_metadata.<device ID>` account data.
    pub model: Option<String>,
    pub kind: Option<String>,
    pub last_seen_ms: Option<u64>,
    pub verified: bool,
    pub current: bool,
}

pub async fn devices(client: Client) -> Result<Vec<DeviceInfo>, String> {
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let current = client.device_id().expect("signed-in client has a device ID").to_owned();
    let mut out = Vec::new();
    for d in client.devices().await.map_err(|e| e.to_string())?.devices {
        let device = client.encryption().get_device(&me, &d.device_id).await.map_err(|e| e.to_string())?;
        let meta = global(&client, &format!("com.beeper.device_metadata.{}", d.device_id)).await;
        let field = |k: &str| meta[k].as_str().filter(|v| !v.is_empty()).map(str::to_string);
        out.push(DeviceInfo {
            name: d.display_name.unwrap_or_else(|| "Unknown".into()),
            model: field("device_model"),
            kind: field("device_type"),
            last_seen_ms: d.last_seen_ts.map(|t| t.0.into()),
            verified: device.is_some_and(|d| d.is_verified()),
            current: d.device_id == current,
        });
    }
    Ok(out)
}

pub async fn logout(store: SharedStore) -> Result<(), String> {
    let (client, demo, dir) = {
        let st = store.read();
        (st.matrix.clone(), st.config.demo, st.data_dir.clone())
    };
    if !demo {
        client.ok_or("Matrix is not connected yet")?.logout().await.map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join(crate::store::LOGGED_OUT), "").map_err(|e| format!("Could not mark the logout: {e}"))
}

pub async fn verify(store: SharedStore, confirm: oneshot::Receiver<bool>) -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let identity = client.encryption().get_user_identity(&me).await.map_err(|x| e(&x))?;
    let request = identity.ok_or("this account has no cross-signing identity")?.request_verification().await;
    let request = request.map_err(|x| e(&x))?;
    set_verification(&store, "requested", None);
    let mut requests = request.changes();
    let mut sas = None;
    let mut sas_changes = futures_util::stream::pending().boxed();
    let mut confirm = Some(confirm);
    loop {
        tokio::select! {
            state = requests.next() => match state.ok_or("the verification request ended")? {
                VerificationRequestState::Ready { .. } if request.is_ready() => {
                    request.start_sas().await.map_err(|x| e(&x))?;
                }
                VerificationRequestState::Transitioned { verification: Verification::SasV1(s), .. } => {
                    if !s.we_started() {
                        s.accept().await.map_err(|x| e(&x))?;
                    }
                    sas_changes = s.changes().boxed();
                    sas = Some(s);
                }
                VerificationRequestState::Cancelled(c) => {
                    set_verification(&store, "cancelled", None);
                    return Err(c.reason().to_string());
                }
                _ => {}
            },
            Some(state) = sas_changes.next() => match state {
                SasState::KeysExchanged { emojis: Some(emojis), decimals: (a, b, c) } => {
                    let emojis = emojis.emojis.iter();
                    let emojis =
                        emojis.map(|x| VerificationSasEmoji { emoji: x.symbol.into(), description: x.description.into() });
                    let sas_info = VerificationSas { decimals: Some(format!("{a} {b} {c}")), emojis: emojis.collect() };
                    set_verification(&store, "sas_started", Some(sas_info));
                    let sas = sas.as_ref().expect("SAS changes come from a SAS");
                    match confirm.take().expect("emojis are shown once").await {
                        Ok(true) => sas.confirm().await.map_err(|x| e(&x))?,
                        _ => {
                            sas.cancel().await.map_err(|x| e(&x))?;
                            set_verification(&store, "cancelled", None);
                            return Err("the emojis did not match".into());
                        }
                    }
                }
                SasState::Done { .. } => {
                    set_verification(&store, "verified", None);
                    return Ok(());
                }
                SasState::Cancelled(c) => {
                    set_verification(&store, "cancelled", None);
                    return Err(c.reason().to_string());
                }
                _ => {}
            },
        }
    }
}

pub async fn run(store: SharedStore) {
    crate::cloud::install_tls();
    let mut rx = store.read().ws_tx.subscribe();
    while !store.read().signed_in() {
        rx.recv().await.ok();
    }
    let client = match connect(&store).await {
        Ok(c) => c,
        Err(e) => return store.write().matrix_error = Some(e),
    };
    store.write().matrix = Some(client.clone());
    tokio::spawn(retry_undecrypted(client.clone(), store.clone()));
    tokio::spawn(retry_after_backup(client.clone(), store.clone()));
    sync_account_data(&client, &store).await;
    sync_prefs(&client, &store).await;
    sync_profile(&client, &store).await;
    store.write().inbox_rules = global(&client, RULES).await;
    let first = !store.read().beeper_state.first_sync_done;
    futures_util::stream::iter(client.joined_rooms())
        .for_each_concurrent(8, |room| {
            let store = store.clone();
            async move {
                let events = if first || has_utd(&store, &room).await { history(&room).await } else { Vec::new() };
                sync_room(&store, &room, events).await
            }
        })
        .await;
    if first {
        let mut st = store.write();
        st.beeper_state.first_sync_done = true;
        st.persist_state_to_sqlite();
    }
    let settings = SyncSettings::default().timeout(std::time::Duration::from_secs(30));
    let client2 = client.clone();
    client
        .sync_with_result_callback(settings, |r| {
            let (client, store) = (client2.clone(), store.clone());
            async move {
                match r {
                    Err(e) => store.write().matrix_error = Some(e.to_string()),
                    Ok(resp) => {
                        store.write().matrix_error = None;
                        refresh_e2ee(&client, &store).await;
                        sync_account_data(&client, &store).await;
                        sync_prefs(&client, &store).await;
                        sync_rules(&client, &store).await;
                        for id in resp.rooms.left.keys() {
                            // A forgotten room, like a deleted label, is gone from the client.
                            let chat = match client.get_room(id) {
                                Some(room) => chat_id(&room).await,
                                None => id.to_string(),
                            };
                            store.write().remove_chat(&chat);
                        }
                        for (id, update) in resp.rooms.joined {
                            let room = client.get_room(&id).expect("synced room is known");
                            sync_room(&store, &room, update.timeline.events).await;
                            for e in &update.ephemeral {
                                let v: Value = e.deserialize_as().expect("ephemeral event JSON");
                                if v["type"] == "m.typing" {
                                    set_typing(&store, &room, &v["content"]["user_ids"]).await;
                                } else if v["type"] == "m.receipt" {
                                    sync_receipts(&store, &room).await;
                                }
                            }
                        }
                    }
                }
                Ok(LoopCtrl::Continue)
            }
        })
        .await
        .expect("Matrix sync loop");
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn demo_logout_deletes_local_data() {
        let dir = std::env::temp_dir().join(format!("beeper-rs-logout-{}", std::process::id()));
        let mut cfg = crate::config::CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23391, None);
        cfg.demo = true;
        let store = crate::store::StoreLock::shared(crate::store::BeeperStore::from_config(cfg.clone()));
        let cached = dir.join("media").join("cached");
        std::fs::write(&cached, "x").expect("write a cached file");
        super::logout(store).await.expect("demo logout");
        crate::store::BeeperStore::from_config(cfg);
        assert!(!cached.exists() && !dir.join(crate::store::LOGGED_OUT).exists(), "the next start deletes the data");
        std::fs::remove_dir_all(dir).expect("remove the test data dir");
    }

    #[test]
    fn group_edits_follow_room_features_and_power_levels_like_desktop() {
        use super::edit_bits;
        use serde_json::json;
        let levels = json!({ "users": { "@me:x": 50 }, "events": { "m.room.avatar": 100 }, "state_default": 50 });
        assert_eq!(edit_bits(None, &levels, "@me:x"), 0b101, "the avatar needs power 100");
        assert_eq!(edit_bits(None, &levels, "@other:x"), 0, "users_default is 0");
        let features = json!({ "state": { "m.room.name": { "level": 2 }, "m.room.topic": { "level": 0 } } });
        assert_eq!(edit_bits(Some(&features), &levels, "@me:x"), 0b001, "features without a level reject the event");
        assert_eq!(edit_bits(None, &json!(null), "@me:x"), 0, "without power levels the events need 50");
        assert_eq!(edit_bits(None, &json!({ "users_default": 50 }), "@me:x"), 0b111);
    }

    #[test]
    fn show_in_inbox_sets_the_rule_of_the_account() {
        use super::{hidden_from_inbox, inbox_rule, with_inbox_visibility};
        use serde_json::json;
        let (tg, slack) = (json!({"com.beeper.bridge_name": "telegram"}), json!({"com.beeper.bridge_name": "slackgo"}));
        let (team, rules) = ("slackgo.T1-U1", json!({"order": ["x"]}));
        assert_eq!(inbox_rule(&rules, "telegram", "telegram", None), ("telegram".into(), "all"));
        assert_eq!(inbox_rule(&rules, team, "slackgo", None), ("slackgo.T1".into(), "dmOnly"));
        assert_eq!(inbox_rule(&rules, "telegram", "telegram", Some("topic")).0, "telegram");
        for (option, dm, group) in [("all", false, false), ("dmOnly", false, true), ("none", true, true)] {
            let r = with_inbox_visibility(rules.clone(), "telegram", option);
            assert_eq!((r["order"].clone(), inbox_rule(&r, "telegram", "telegram", None).1), (json!(["x"]), option));
            assert_eq!([false, true].map(|g| hidden_from_inbox(&r, "telegram", &tg, g)), [dm, group]);
        }
        let r = with_inbox_visibility(rules.clone(), "slackgo.T1", "none");
        assert_eq!(r["visibility"], json!({"slackgo.T1": "none"}));
        assert!(hidden_from_inbox(&r, "slackgo.T1-U2", &slack, false));
        let discord = json!({"com.beeper.bridge_name": "discordgo", "network": {"id": "G1"}});
        assert_eq!(inbox_rule(&rules, "discordgo", "discordgo", Some("G1")), ("discordgo.G1".into(), "none"));
        let r = with_inbox_visibility(rules, "discordgo.G1", "all");
        assert_eq!(inbox_rule(&r, "discordgo", "discordgo", Some("G1")).1, "all");
        assert!(!hidden_from_inbox(&r, "discordgo", &discord, true));
    }

    #[test]
    fn polls_follow_desktop() {
        use super::{poll_rules, PollRules};
        assert_eq!(poll_rules(&json!({"poll": 0, "poll_max_options": 5})), None);
        let rules = PollRules { hidden_votes: false, end: false, duplicates: true, max_options: 5, max_length: 200 };
        assert_eq!(poll_rules(&json!({"poll": 2, "poll_duplicate_options": 1, "poll_max_options": 5})), Some(rules));
        use serde_json::json;
        let start = json!({
            "org.matrix.msc1767.text": "Lunch?\n1. Pizza\n2. Soup",
            "org.matrix.msc3381.poll.start": {
                "question": { "org.matrix.msc1767.text": "Lunch?" },
                "kind": "org.matrix.msc3381.poll.undisclosed",
                "max_selections": 2,
                "answers": [
                    { "id": "1", "org.matrix.msc1767.text": "Pizza" },
                    { "m.id": "2", "m.text": [{ "mimetype": "text/html", "body": "<b>Soup</b>" }, { "body": "Soup" }] },
                ],
            },
        });
        let mut p = super::poll(&start).expect("a poll start");
        assert_eq!((p.question.as_str(), p.max_selections, p.undisclosed), ("Lunch?", 2, true));
        assert_eq!(
            p.options.iter().map(|o| (o.id.as_str(), o.label.as_str())).collect::<Vec<_>>(),
            [("1", "Pizza"), ("2", "Soup")]
        );
        assert_eq!(p.fallback, "Lunch?\n1. Pizza\n2. Soup");
        let single =
            super::poll(&json!({ "m.poll": { "question": { "m.text": "Q" }, "answers": [] } })).expect("a poll");
        assert_eq!((single.max_selections, single.undisclosed), (1, false));
        assert!(super::poll(&json!({ "body": "no poll" })).is_none());
        let answers = |c| super::poll_answers(&c);
        assert_eq!(answers(json!({ "org.matrix.msc3381.poll.response": { "answers": ["1", 2, "2"] } })), ["1", "2"]);
        assert_eq!(answers(json!({ "m.selections": ["2"] })), ["2"]);
        p.respond("@a:x", &["1".into(), "2".into()]);
        p.respond("@b:x", &["1".into()]);
        p.respond("@a:x", &["2".into(), "9".into()]);
        assert_eq!((p.voters(), p.picks("@a:x"), p.picks("@b:x")), (2, vec!["2".to_string()], vec!["1".to_string()]));
    }

    #[test]
    fn media_body_is_a_caption_only_when_it_differs_from_the_file_name() {
        let c = |v: serde_json::Value| super::caption(&v);
        assert_eq!(c(serde_json::json!({"body": "a.jpg", "filename": "a.jpg"})), None);
        assert_eq!(c(serde_json::json!({"body": "a.jpg"})), None);
        assert_eq!(c(serde_json::json!({"body": "look", "filename": "a.jpg"})).as_deref(), Some("look"));
        assert_eq!(
            c(serde_json::json!({"body": "https://www.instagram.com/reel/x/", "com.beeper.unresolved_media": {}}))
                .as_deref(),
            Some("https://www.instagram.com/reel/x/")
        );
    }

    #[test]
    fn membership_text_matches_desktop() {
        use super::membership_text as t;
        assert_eq!(t("join", "invite", ("Ann", false), None), "Ann joined the chat");
        assert_eq!(t("leave", "join", ("Ann", false), None), "Ann left the chat");
        assert_eq!(t("leave", "join", ("Ann", false), Some(Some("Bob"))), "Bob removed Ann from the chat");
        assert_eq!(t("leave", "join", ("Ann", true), Some(None)), "You were removed from the chat");
        assert_eq!(t("leave", "invite", ("Ann", false), None), "Ann declined the invitation");
        assert_eq!(t("leave", "invite", ("Ann", false), Some(Some("You"))), "You withdrew Ann's invitation");
        assert_eq!(t("leave", "invite", ("Ann", true), Some(None)), "Your invitation was withdrawn");
        assert_eq!(t("leave", "ban", ("Ann", false), Some(Some("Bob"))), "Bob unbanned Ann");
        assert_eq!(t("ban", "leave", ("Ann", false), None), "Ann was banned");
        assert_eq!(t("invite", "", ("Ann", false), Some(Some("Bob"))), "Bob invited Ann to the chat");
        assert_eq!(t("invite", "", ("Ann", true), Some(None)), "You were invited to the chat");
    }

    #[test]
    fn reaction_verbs_ignore_variation_selectors() {
        for (key, verb) in
            [("👍\u{fe0f}", "liked"), ("\u{2764}", "loved"), ("\u{2764}\u{fe0f}", "loved"), ("🎉", "reacted 🎉 to")]
        {
            assert_eq!(super::reaction_verb(key), verb, "{key:?}");
        }
    }

    #[test]
    fn reaction_text_quotes_the_plain_text() {
        let m: crate::models::Message = serde_json::from_value(serde_json::json!({
            "id": "1", "chatID": "!r", "accountID": "a", "senderID": "@u", "timestamp": "2026-01-01T00:00:00Z",
            "sortKey": "1", "type": "TEXT", "text": "[https://x.io](https://x.io)",
            "html": "<a href=\"https://x.io\">https://x.io</a>", "attachments": [],
        }))
        .unwrap();
        assert_eq!(super::reaction_text("\u{2764}", Some(&m), None), "loved \"https://x.io\"");
        let m = crate::models::Message { html: None, ..m };
        assert_eq!(
            super::reaction_text("\u{2764}", Some(&m), None),
            "loved \"https://x.io\"",
            "markdown links lose the URL"
        );
    }
}

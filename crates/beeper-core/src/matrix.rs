use crate::config::{now_rfc3339_millis, now_unix_millis, rfc3339_from_millis};
use crate::import::DesktopCrypto;
use crate::models::{
    timer_distance, AppVerification, Attachment, AttachmentSize, Bookmark, BridgeAccountInfo, Chat, ChatFlag,
    ChatParticipants, ChatReminder, EditHistoryEntry, FileRules, Label, Message, MessageLink, MessageReminder,
    MessageRules, MessageTimer, Older, PollData, PollOption, PollRules, Reaction, SendError, Sticker, StickerPack,
    StickerSource, ThreadFollows, Timers, Transcription, User, VerificationDevice, VerificationError, VerificationQr,
    VerificationSas, VerificationSasEmoji, ALL_FORMATS, ALL_MEMBER_ACTIONS, BEEPER_MESSAGES, BEEPER_POLLS, BLOCK_USER,
    BOOKMARK, BOOKMARKS_ROOM, CHAT_SOUNDS, DEFAULT_TIMERS, DELETE_FOR_EVERYONE, DELETE_FOR_ME, DISAPPEARING_TIMER,
    EDITS, FORMATS, MANAGED_ROOMS, MANAGE_KICK, MANAGE_ROLE, MEMBER_ACTIONS, MEMBER_KICK, MESSAGE_REMINDER,
    MSG_MENTION, NOTIFICATION_SOUND, NO_UNBLOCK, PERSONAL_AVATAR, PERSONAL_NAME, REMINDERS_ROOM, REMOVED_MESSAGE,
    REMOVED_MESSAGES_ROOM, REPORT_SPAM, SNOOZED, TRANSCRIPTION, TRANSCRIPTIONS_ROOM, USER_BLOCKED,
};
use crate::prefs::Ad;
use crate::store::{order_of, sort_key, BeeperStore, NetworkSpace, SharedStore, NOTE_CHAT_ID};
use futures_util::StreamExt;
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::deserialized_responses::{RawAnySyncOrStrippedState, TimelineEvent};
use matrix_sdk::encryption::backups::BackupState;
use matrix_sdk::encryption::recovery::{RecoveryError, RecoveryState};
use matrix_sdk::encryption::secret_storage::SecretStorageError;
use matrix_sdk::encryption::verification::{
    CancelInfo, QrVerificationData, QrVerificationState, SasState, Verification, VerificationRequest,
    VerificationRequestState,
};
use matrix_sdk::encryption::{BackupDownloadStrategy, CrossSigningResetAuthType, EncryptionSettings};
use matrix_sdk::event_handler::Ctx;
use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk::notification_settings::RoomNotificationMode;
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::api::client::media::get_content_thumbnail::v3::Method;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType as SendReceiptType;
use matrix_sdk::ruma::api::client::uiaa::AuthData;
use matrix_sdk::ruma::api::error::ErrorKind;
use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent;
use matrix_sdk::ruma::events::key::verification::VerificationMethod;
use matrix_sdk::ruma::events::reaction::ReactionEventContent;
use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
use matrix_sdk::ruma::events::relation::{Annotation, Reply};
use matrix_sdk::ruma::events::room::message::{Relation, RoomMessageEventContent};
use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::tag::{TagInfo, TagName};
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, Mentions, RoomAccountDataEventType, StateEventType};
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{OwnedDeviceId, OwnedEventId, OwnedUserId};
use matrix_sdk::{Client, LoopCtrl, Room, RoomMemberships, RoomState, SessionMeta, SessionTokens};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tokio::sync::oneshot;

pub const UTD_TEXT: &str = "Waiting for message";
const HISTORY: u32 = 30;
/// Desktop's `THREAD_FOLLOWS_EVENT_TYPE`.
const THREAD_FOLLOWS: &str = "com.beeper.thread_follows";

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
        timer: Option<MessageTimer>,
    },
    /// The `com.beeper.disappearing_timer` of an edit without content.
    Timer(String, MessageTimer),
    /// A redaction, and whether the bridge redacted view-once media after its last view.
    Redact(String, bool),
    Remove(String),
    /// Desktop's `com.beeper.view_limited` update: the views left of view-once media.
    ViewsLeft(String, u64),
    /// A poll response: the poll, the voter and the answers.
    Vote(String, String, Vec<String>),
    EndPoll(String),
    /// A `com.beeper.message_send_status`: the message, whether it reached a user, the error, the status, and the
    /// disappearing timer of the bridge.
    SendStatus(String, bool, Option<SendError>, Box<crate::models::SendStatus>, Option<MessageTimer>),
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn body(c: &Value) -> Option<String> {
    let (b, p) = (s(&c["body"])?, &c["com.beeper.per_message_profile"]);
    // Other clients can set `has_fallback` without the fallback in the body. Desktop shows the body then.
    let fallback = s(&p["displayname"]).filter(|_| p["has_fallback"] == true).map(|n| format!("{n}: "));
    Some(fallback.and_then(|f| b.strip_prefix(&f).map(str::to_string)).unwrap_or(b))
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

/// Desktop's `findMatchingBridgeAccount`: the login of the room's receiver, the one Slack login of the room's
/// workspace, or the one login of a room without a receiver.
fn account_id(bridge: &Value, accounts: &[BridgeAccountInfo]) -> String {
    let Some(name) = s(&bridge["com.beeper.bridge_name"]) else { return "hungryserv".into() };
    let logins: Vec<&BridgeAccountInfo> = accounts.iter().filter(|a| a.bridge_id == name).collect();
    let receiver = s(&bridge["channel"]["fi.mau.receiver"]).filter(|r| *r != name && r != "unknown");
    let one = |v: Vec<&&BridgeAccountInfo>| (v.len() == 1).then(|| v[0].account_id.clone());
    let workspace = s(&bridge["network"]["id"]).filter(|_| name == "slackgo").and_then(|n| {
        one(logins.iter().filter(|a| a.login_id == n || a.login_id.starts_with(&format!("{n}-"))).collect())
    });
    let by_receiver = receiver.as_ref().and_then(|r| logins.iter().find(|a| a.login_id == *r));
    let only = receiver.is_none().then(|| one(logins.iter().collect())).flatten();
    by_receiver.map(|a| a.account_id.clone()).or(workspace).or(only).unwrap_or(name)
}

pub const RULES: &str = "com.beeper.chat_list_rules";
/// Desktop's `LAST_VIEWED_REQUESTS_AT`: when the Requests view last showed each account's requests.
pub const REQUESTS_VIEWED: &str = "com.beeper.message_requests.last_viewed_at";
/// The bridge states that the homeserver keeps, in the shape of `whoami`'s `user.bridges`.
const BRIDGE_STATE: &str = "com.beeper.bridge_state";

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
    let vis = |key: &str| visibility(rules, key);
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

/// The `chat_list_rules` option of `key`. Desktop's `YH` skips the values that it does not know, so a newer client
/// cannot break the inbox.
fn visibility<'a>(rules: &'a Value, key: &str) -> Option<&'a str> {
    rules["visibility"][key].as_str().filter(|v| matches!(*v, "all" | "dmOnly" | "none"))
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
            visibility(rules, &format!("{account_id}.{n}"))
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

/// Desktop's `canStartEditMessage`, `deletionMode`, `canReact` and `supportsReply` levels from
/// `com.beeper.room_features`.
fn message_rules(f: &Value) -> MessageRules {
    let level = |k: &str| f[k].as_i64().unwrap_or(0);
    MessageRules {
        edit: level("edit") > 0,
        edit_max_age: f["edit_max_age"].as_u64().filter(|&s| s > 0),
        delete: level("delete") == 2,
        delete_max_age: f["delete_max_age"].as_u64().filter(|&s| s > 0),
        delete_for_me: f["delete_for_me"] == true && level("delete") < 1,
        react: level("reaction") > 0,
        reply: level("reply") > 0,
    }
}

/// The non-null `file` kinds of `com.beeper.room_features`. The bridge writes the state, so a kind that does not parse
/// is left out with a log line.
pub(crate) fn file_rules(f: &Value) -> FileRules {
    let kinds = f["file"].as_object().into_iter().flatten().filter(|(_, v)| !v.is_null());
    kinds
        .filter_map(|(k, v)| {
            let rule = serde_json::from_value(v.clone()).map_err(|e| eprintln!("room features: file kind {k}: {e}"));
            Some((k.clone(), rule.ok()?))
        })
        .collect()
}

/// The file features of a room, and whether the room has its own room features. Desktop takes the defaults of the
/// bridge for a room without room features or with empty ones.
pub(crate) fn file_features(features: Option<&Value>, bridge: &Value) -> (crate::models::Files, bool) {
    match features.filter(|f| f.as_object().is_some_and(|f| !f.is_empty())) {
        Some(f) => (crate::models::Files::new(file_rules(f)), true),
        None => (crate::models::default_files(bridge), false),
    }
}

/// The `Chat::block` bits from the room features and the `m.bridge` channel.
fn block_bits(features: Option<&Value>, bridge: &Value) -> u8 {
    let on = |f: Option<&Value>, k: &str, bit: u8| u8::from(f.is_some_and(|f| f[k] == true)) * bit;
    on(features, "block_user", BLOCK_USER)
        | on(features, "no_unblock", NO_UNBLOCK)
        | on(features, "report_spam", REPORT_SPAM)
        | on(Some(&bridge["channel"]), "com.beeper.user_blocked", USER_BLOCKED)
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
    EDITS.iter().enumerate().fold(0, |m, (i, k)| m | (u8::from(can_send(features, levels, me, k)) << i))
}

fn can_send(features: Option<&Value>, levels: &Value, me: &str, k: &str) -> bool {
    let mine = levels["users"][me].as_i64().or(levels["users_default"].as_i64()).unwrap_or(0);
    let supported = features.is_none_or(|f| f["state"][k]["level"].as_i64().is_some_and(|l| l > 0));
    supported && mine >= levels["events"][k].as_i64().or(levels["state_default"].as_i64()).unwrap_or(50)
}

/// The `MANAGE_` bits of `id` for `me`: Desktop kicks members below its own power level when the room supports
/// kicks, and changes the roles of others when it can send power levels, in Matrix rooms only of those below it.
fn manage(levels: &Value, actions: u8, me: &str, id: &str, bridged: bool) -> u8 {
    let power = |id: &str| levels["users"][id].as_i64().or(levels["users_default"].as_i64()).unwrap_or(0);
    let (mine, below) = (power(me), power(id) < power(me));
    let kick = actions & MEMBER_KICK != 0 && mine >= levels["kick"].as_i64().unwrap_or(50) && below;
    let roles =
        mine >= levels["events"]["m.room.power_levels"].as_i64().or(levels["state_default"].as_i64()).unwrap_or(50);
    let role = roles && id != me && (bridged || below);
    (u8::from(kick) * MANAGE_KICK) | (u8::from(role) * MANAGE_ROLE)
}

/// Desktop's `roomFeatures.disappearing_timer`; a room without features has the Beeper defaults, which have it.
fn timers(features: Option<&Value>, levels: &Value, me: &str) -> Option<Timers> {
    let t = features.map_or(Some(&Value::Null), |f| f.get("disappearing_timer").filter(|t| t.is_object()))?;
    Some(Timers {
        kind: t["types"][0].as_str().unwrap_or("after_send").into(),
        timers: t["timers"]
            .as_array()
            .map_or(DEFAULT_TIMERS.to_vec(), |a| a.iter().filter_map(Value::as_u64).collect()),
        can_set: can_send(features, levels, me, DISAPPEARING_TIMER),
    })
}

async fn chat(room: &Room, accounts: &[BridgeAccountInfo], rules: &Value) -> Chat {
    let bridge = state(room, "m.bridge").await.unwrap_or(Value::Null);
    let service = room.service_members().unwrap_or_default();
    let bot = s(&bridge["bridgebot"]);
    let me = room.own_user_id();
    // Desktop lists the invited members too, as pending (`isPending`).
    let members =
        room.members_no_sync(RoomMemberships::JOIN | RoomMemberships::INVITE).await.expect("read room members");
    let levels = state(room, "m.room.power_levels").await.unwrap_or(Value::Null);
    let users_default = levels["users_default"].as_i64().unwrap_or(0).try_into().expect("power level");
    let features = state(room, "com.beeper.room_features").await;
    let (file, room_features) = file_features(features.as_ref(), &bridge);
    let tombstone = state(room, "m.room.tombstone").await;
    let member_actions = features.as_ref().map_or(ALL_MEMBER_ACTIONS, |f| {
        let on = |k: &str| f["member_actions"][k].as_i64().is_some_and(|l| l > 0);
        MEMBER_ACTIONS.iter().enumerate().filter(|(_, k)| on(k)).fold(0, |m, (i, _)| m | 1 << i)
    });
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
    // Desktop names a member without a display name by its user ID.
    let unnamed: HashSet<String> =
        members.iter().filter(|m| m.display_name().is_none()).map(|m| m.user_id().to_string()).collect();
    let mut items: Vec<User> = members
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
                is_pending: (*m.membership() == matrix_sdk::ruma::events::room::member::MembershipState::Invite)
                    .then_some(true),
                manage: manage(&levels, member_actions, me.as_str(), id, !bridge.is_null()),
                ..Default::default()
            }
        })
        .collect();
    items.sort_by(crate::models::member_order);
    let tags = room.tags().await.expect("read room tags").unwrap_or_default();
    // Desktop's thread type (`rT`): a note to self is single, a declared DM is single up to two people, and a room
    // with a name or a declared group DM is a group.
    let note = state(room, "com.beeper.rooms.note_to_self").await.is_some();
    let named = room.name().is_some_and(|n| !n.is_empty());
    let dm = note
        || match bridge["com.beeper.room_type.v2"].as_str() {
            Some("dm") => items.len() <= 2,
            Some("group_dm") => false,
            _ => !named && items.len() <= 2,
        };
    let names = |own: bool| items.iter().filter(move |u| u.is_self == Some(own)).filter_map(|u| u.full_name.clone());
    let others: Vec<String> = names(false).collect();
    let title = room.name().filter(|n| !n.is_empty()).unwrap_or_else(|| {
        if others.is_empty() {
            // The own member of an invite has not joined, so an invite may have no joined member to name it by.
            names(true).next().unwrap_or_else(|| room.room_id().to_string())
        } else {
            // Desktop's `tIe`: a group is named by its members, a direct chat by the name of the other member.
            let anonymous = |u: &User| User { full_name: None, ..u.clone() };
            let titled: Vec<User> =
                items.iter().map(|u| if unnamed.contains(&u.id) { anonymous(u) } else { u.clone() }).collect();
            match dm {
                true => titled.iter().find(|u| u.is_self != Some(true)).map(|u| u.sort_name().to_string()),
                false => Some(crate::models::group_title(&titled)),
            }
            .expect("a chat with other members")
        }
    });
    let account_id = account_id(&bridge, accounts);
    // Desktop's `hasExited`: in a room that the user left, the own member stays, exited, and does not count above.
    if room.state() == RoomState::Left {
        items.push(User { id: me.to_string(), is_self: Some(true), has_exited: Some(true), ..Default::default() });
    }
    let mut chat = Chat {
        id: room.room_id().to_string(),
        is_hidden_from_inbox: Some(hidden_from_inbox(rules, &account_id, &bridge, !dm)),
        inbox_group: !dm && bridge["com.beeper.room_type.v2"] != "group_dm",
        is_request: Some(bridge["channel"]["com.beeper.message_request"] == true),
        created_at: states(room, "m.room.create".into()).await.first().and_then(|e| e["origin_server_ts"].as_i64()),
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
            // Desktop's `mutedRoomIDs` of `com.beeper.muted`, or the room's push rule. The SDK's
            // `Room::user_defined_notification_mode` is empty for a room the user left, but its rule still mutes it.
            muted_rooms(&room.client()).await.iter().any(|r| r == room.room_id().as_str())
                || room
                    .client()
                    .notification_settings()
                    .await
                    .get_user_defined_room_notification_mode(room.room_id())
                    .await
                    == Some(RoomNotificationMode::MentionsAndKeywordsOnly),
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
        // Desktop's `com.beeper.chats.snoozed` handler keeps a snooze that ends in the future.
        snoozed_until_ms: account_data(room, SNOOZED)
            .await
            .and_then(|s| s["snoozed_until_ms"].as_i64())
            .filter(|&ms| ms > now_unix_millis()),
        chat_theme: None,
        note: None,
        notifiable: None,
        delete_modes: features.as_ref().map_or(0, |f| {
            (u8::from(f["delete_chat"] == true) * DELETE_FOR_ME)
                | (u8::from(f["delete_chat_for_everyone"] == true) * DELETE_FOR_EVERYONE)
        }),
        can_accept_request: features.as_ref().is_some_and(|f| f["message_request"]["accept_with_button"] == 2),
        block: block_bits(features.as_ref(), &bridge),
        edits: edit_bits(features.as_ref(), &levels, me.as_str()),
        member_actions,
        formatting: features.as_ref().map_or(ALL_FORMATS, |f| {
            FORMATS.iter().enumerate().filter(|(_, k)| f["formatting"][**k] == 2).fold(0, |m, (i, _)| m | 1 << i)
        }),
        file,
        room_features,
        polls: features.as_ref().map_or(Some(BEEPER_POLLS), poll_rules),
        messages: features.as_ref().map_or(BEEPER_MESSAGES, message_rules),
        allowed_reactions: (features.as_ref().map(|f| f["allowed_reactions"].as_array().into_iter().flatten()))
            .into_iter()
            .flatten()
            .filter_map(|r| r.as_str().map(str::to_string))
            .collect(),
        disappearing: state(room, DISAPPEARING_TIMER).await.and_then(|t| t["timer"].as_u64()).filter(|&t| t > 0),
        timers: timers(features.as_ref(), &levels, me.as_str()),
        older: Older::Unknown,
        is_read_only: tombstone.is_some() || read_only(&levels, me.as_str()),
        replacement_room: tombstone.and_then(|t| s(&t["replacement_room"])),
        can_mention_room: features.as_ref().is_none_or(|f| f["formatting"]["at_room_mention"].as_i64() >= Some(1))
            && power(&levels, me.as_str()) >= levels["notifications"]["room"].as_i64().unwrap_or(50),
        threads: features.as_ref().is_some_and(|f| f["thread"].as_i64() > Some(0)),
        follows: account_data(room, THREAD_FOLLOWS).await.and_then(|f| ThreadFollows::parse(&f)).unwrap_or_default(),
        thread_reads: BTreeMap::new(),
        personal: None,
        // Desktop's `Up`: a sound it knows, else the Default sound.
        notification_sound: account_data(room, NOTIFICATION_SOUND)
            .await
            .and_then(|d| s(&d["sound"]))
            .filter(|n| CHAT_SOUNDS.contains(&n.as_str())),
        flags: 0,
    };
    let name = account_data(room, PERSONAL_NAME).await.and_then(|n| s(&n["name"])).filter(|n| !n.is_empty());
    let photo = account_data(room, PERSONAL_AVATAR).await.and_then(|a| url_field(&a["url"], &a["file"]));
    chat.set_personal(name, photo);
    chat
}

/// Desktop's `isReadOnly` from `m.room.power_levels`: your level is below the level of `m.room.message`.
fn read_only(levels: &Value, me: &str) -> bool {
    power(levels, me) < levels["events"]["m.room.message"].as_i64().or(levels["events_default"].as_i64()).unwrap_or(0)
}

/// The power level of `user` in `m.room.power_levels`.
fn power(levels: &Value, user: &str) -> i64 {
    levels["users"][user].as_i64().or(levels["users_default"].as_i64()).unwrap_or(0)
}

fn url_field(url: &Value, file: &Value) -> Option<String> {
    match file.as_object() {
        Some(f) => Some(format!(
            "{}?{ENCRYPTED_FILE}={}",
            s(&file["url"])?,
            crate::codec::url_encode(&Value::Object(f.clone()).to_string())
        )),
        None => s(url),
    }
}

fn attachment(c: &Value, kind: &str) -> Attachment {
    let info = &c["info"];
    let dim = |k: &str| info[k].as_u64().and_then(|n| u32::try_from(n).ok());
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
        // Desktop's `wi`: an emote is plain text, and another msgtype (a `com.beeper.gallery`, a `com.beeper.command`,
        // an `m.key.verification.request`) is a `HIDDEN` row.
        ("m.room.message", Some("m.text" | "m.emote")) => Some((body(c), Vec::new(), "TEXT")),
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

/// Desktop's `isBridgeBot` (`Cp`): the bot of a bridge on the Beeper homeserver.
fn bridge_bot(user: &str) -> bool {
    let local = user.ends_with(":beeper.local") || user.ends_with(".localhost");
    let bot = user.strip_prefix('@').and_then(|u| u.strip_suffix("bot:beeper.local"));
    bot.is_some_and(|b| {
        (1..=32).contains(&b.len()) && b.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    }) || (user.starts_with("@_bridgebot:") && local)
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
    // Desktop's `hs.order ?? 0`: other clients and servers can leave it out.
    let order = e["unsigned"]["com.beeper.hs.order"].as_i64().unwrap_or(0);
    let poll = matches!(kind.as_str(), "org.matrix.msc3381.poll.start" | "m.poll.start").then(|| poll(c)).flatten();
    let (text, attachments, message_type) = match kind.as_str() {
        "m.room.message" | "m.sticker" | "m.room.encrypted" if deleted => (None, Vec::new(), "TEXT"),
        _ if deleted => return None,
        "m.reaction" => (None, Vec::new(), "REACTION"),
        "m.room.redaction" => {
            let target = s(&e["redacts"]).or(s(&c["redacts"]))?;
            return Some(Op::Redact(target, c["com.beeper.view_limited"] == true));
        }
        "m.room.message" if rel["rel_type"] == "m.replace" => {
            let (new, ts) = (&c["m.new_content"], e["origin_server_ts"].as_i64().expect("event timestamp"));
            let (text, attachments, message_type) = content("m.room.message", new)?;
            // Desktop's `saveMessageTimer` of an edit.
            let (target, timer) = (s(&rel["event_id"])?, MessageTimer::of(&new[DISAPPEARING_TIMER], order));
            if text.is_none() && attachments.is_empty() {
                return Some(Op::Timer(target, timer?));
            }
            let timestamp = (c["com.beeper.dont_render_edited"] != true).then(|| rfc3339_from_millis(ts));
            return Some(Op::Edit {
                target,
                text,
                html: html(new),
                links: links(new),
                attachments,
                message_type,
                unresolved_media: unresolved_media(new),
                timestamp,
                timer,
            });
        }
        "m.room.encrypted" => (Some(UTD_TEXT.into()), Vec::new(), "TEXT"),
        // Desktop's `handleMessageSendStatus`: a bridge took the message, or could not send it.
        "com.beeper.message_send_status" => {
            let ok = matches!(c["status"].as_str(), Some("SUCCESS" | "PENDING"));
            let error = (!ok).then(|| SendError { message: s(&c["message"]), retrying: false });
            let delivered = c["delivered_to_users"].as_array().is_some_and(|u| !u.is_empty());
            // Desktop's `handleMessageSendStatus` keeps the status for `sendStatus`.
            let status = crate::models::SendStatus {
                status: s(&c["status"])?,
                reason: s(&c["reason"]),
                message: s(&c["message"]),
                ts: c["ts"].as_i64(),
                delivered_to_users: (c["delivered_to_users"].as_array().into_iter().flatten())
                    .filter_map(|u| u.as_str().map(String::from))
                    .collect(),
                internal_error: s(&c["internal_error"]),
            };
            // Desktop's `applyDisappearingTimer` of the timer that a local bridge set on the message that it took.
            let local = sender.ends_with(":beeper.local") || sender.ends_with(".localhost");
            let timer = MessageTimer::of(&c["disappearing_timer"], order)
                .filter(|_| c["status"] == "SUCCESS" && rel["rel_type"] == "m.reference" && local);
            return Some(Op::SendStatus(s(&rel["event_id"])?, delivered, error, Box::new(status), timer));
        }
        "org.matrix.msc3381.poll.response" | "m.poll.response" => {
            return Some(Op::Vote(s(&rel["event_id"])?, sender, poll_answers(c)));
        }
        "org.matrix.msc3381.poll.end" | "m.poll.end" => return Some(Op::EndPoll(s(&rel["event_id"])?)),
        _ if poll.is_some() => (poll.as_ref().map(|p| p.question.clone()), Vec::new(), "POLL"),
        "m.room.member" => membership(room, e).await.map(|t| (Some(t), Vec::new(), "MEMBERSHIP")).or_else(hide)?,
        DISAPPEARING_TIMER if c["com.beeper.exclude_from_timeline"] == true => hide()?,
        DISAPPEARING_TIMER => {
            let text = match c["timer"].as_u64().unwrap_or(0) {
                0 => "Disappearing messages turned off".into(),
                t => format!("New messages set to disappear after {}", timer_distance(t)),
            };
            (Some(text), Vec::new(), "DISAPPEARING_TIMER")
        }
        // Desktop's `handleViewLimitedUpdateEvent` takes the views left only from a bridge bot.
        "com.beeper.view_limited" => match (s(&rel["event_id"]), c["remaining_count"].as_u64().filter(|&n| n > 0)) {
            (Some(target), Some(left)) if bridge_bot(&sender) => return Some(Op::ViewsLeft(target, left)),
            _ => hide()?,
        },
        k => content(k, c).or_else(hide)?,
    };
    let profile_name = s(&c["com.beeper.per_message_profile"]["displayname"]);
    let user = matrix_sdk::ruma::UserId::parse(&sender).expect("sender is a user ID");
    let member = room.get_member_no_sync(&user).await.expect("read room member");
    // Desktop's `key || ""`, and its reaction of a custom emoji: its shortcode as the key and its image.
    let key = rel["key"].as_str().unwrap_or_default();
    let img_url = key.starts_with("mxc://").then(|| key.to_string());
    let shortcode = img_url.as_ref().and_then(|_| s(&c["com.beeper.reaction.shortcode"])).map(|s| s.replace(':', ""));
    let reaction = (message_type == "REACTION").then(|| Reaction {
        id: id.clone(),
        participant_id: sender.clone(),
        reaction_key: shortcode.unwrap_or_else(|| key.into()),
        animated: None,
        img_url,
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
        removed: false,
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
        timer: (message_type == "DISAPPEARING_TIMER").then(|| c["timer"].as_u64().unwrap_or(0)),
        send_error: None,
        is_delivered: false,
        // Desktop's `threadFields`.
        thread_root: s(&rel["event_id"]).filter(|_| rel["rel_type"] == "m.thread"),
        broadcast: rel["rel_type"] == "m.thread" && c["com.beeper.broadcast_outside_thread"] == true,
        fallback: rel["rel_type"] == "m.thread" && rel["is_falling_back"] != false,
        // Desktop's `mentions`: the user IDs of `m.mentions`, and none without it.
        mentions: c["m.mentions"].is_object().then(|| {
            let ids = c["m.mentions"]["user_ids"].as_array().into_iter().flatten();
            ids.filter_map(|u| u.as_str().map(String::from)).collect()
        }),
        send_status: None,
        call: crate::models::CallAction::of(c),
        disappearing: MessageTimer::of(&c[DISAPPEARING_TIMER], order),
        view_limit: c.get("com.beeper.view_limited").filter(|l| l.is_object() && !deleted).cloned(),
        views_left: None,
        view_expired: deleted && e["unsigned"]["redacted_because"]["content"]["com.beeper.view_limited"] == true,
        view_kind: None,
        flags: match c["m.mentions"]["user_ids"].as_array() {
            Some(ids) if ids.iter().any(|u| u == room.own_user_id().as_str()) => MSG_MENTION,
            _ => 0,
        },
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

/// A link of `com.beeper.linkpreviews`, with its `og:image`, which can be encrypted.
pub fn link(p: &Value) -> Option<MessageLink> {
    let dim = |k: &str| p[k].as_u64().map(|n| u32::try_from(n).expect("image dimension"));
    let (width, height) = (dim("og:image:width"), dim("og:image:height"));
    Some(MessageLink {
        url: s(&p["matched_url"]).or(s(&p["og:url"]))?,
        title: s(&p["og:title"]).unwrap_or_default(),
        summary: s(&p["og:description"]),
        img: url_field(&p["og:image"], &p["beeper:image:encryption"]),
        img_size: width.zip(height).map(|(w, h)| AttachmentSize { width: Some(w), height: Some(h) }),
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

/// One page of the room timeline back from `from`, oldest first, and the token of the page before it, or `None`
/// at the start of the room.
async fn page(
    room: &Room,
    from: Option<String>,
    limit: u32,
) -> matrix_sdk::Result<(Vec<TimelineEvent>, Option<String>)> {
    let mut opts = MessagesOptions::backward();
    (opts.from, opts.limit) = (from, limit.into());
    let page = room.messages(opts).await?;
    let end = page.end.filter(|_| !page.chunk.is_empty());
    Ok((page.chunk.into_iter().rev().collect(), end))
}

fn is_forbidden(e: &matrix_sdk::Error) -> bool {
    matches!(e.client_api_error_kind(), Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden))
}

/// The newest page of a room, or nothing when the homeserver does not give it.
async fn history(room: &Room) -> Vec<TimelineEvent> {
    page(room, None, HISTORY).await.map_or_else(|e| no_history(room, &e), |p| p.0)
}

fn no_history(room: &Room, e: &matrix_sdk::Error) -> Vec<TimelineEvent> {
    eprintln!("Could not load the history of {}: {e}", room.room_id());
    Vec::new()
}

/// Desktop's `loadOlderMessages`: loads pages back from the `older` token of the chat until a page adds a message
/// or the room has no older events.
pub async fn load_older(store: SharedStore, chat: String) -> Result<(), String> {
    let client = store.read().matrix.clone().expect("only a Matrix chat loads older messages");
    // The note chat shows before the homeserver has a note room, and until then it has no history.
    if chat == NOTE_CHAT_ID && joined_room(&client, &chat).await.is_err() {
        let mut st = store.write();
        let idx = st.chat_idx(&chat);
        st.update_chat(idx, |c| c.older = Older::Start);
        return Ok(());
    }
    let room = chat_room(&client, &chat).await?;
    let count = || {
        let mut st = store.write();
        st.load_chat(&chat);
        st.timeline_indices(&chat).len()
    };
    let start = count();
    while count() == start {
        let from = {
            let st = store.read();
            match &st.chat(&chat).older {
                Older::Start => return Ok(()),
                Older::Unknown => None,
                Older::From(token) => Some(token.clone()),
            }
        };
        let (events, end) = match page(&room, from, HISTORY).await {
            Ok(p) => p,
            // A room that the user left shows its history only as far as the homeserver lets them read it.
            Err(e) if room.state() == RoomState::Left && is_forbidden(&e) => (Vec::new(), None),
            Err(e) => return Err(e.to_string()),
        };
        sync_room(&store, &room, events).await;
        let mut st = store.write();
        let idx = st.chat_idx(&chat);
        st.update_chat(idx, |c| c.older = end.map_or(Older::Start, Older::From));
    }
    Ok(())
}

/// The most pages back that fill a gap of a limited sync.
const GAP_PAGES: usize = 10;

/// Whether the room has synced into the chat list before, so that it has events from an earlier sync.
async fn synced_before(store: &SharedStore, room: &Room) -> bool {
    let chat = chat_id(room).await;
    let st = store.read();
    st.chat_idx_by_id.get(&chat).is_some_and(|&i| st.chats[i].notifiable.is_some())
}

/// A limited sync timeline leaves a gap between the last sync and `from`: loads pages back from it until a page
/// reaches a message that the chat has, so that the timeline has no hole.
async fn fill_gap(store: SharedStore, room: Room, from: String) {
    let chat = chat_id(&room).await;
    let mut from = Some(from);
    for _ in 0..GAP_PAGES {
        let Some(token) = from.take() else { return };
        let (events, end) = match page(&room, Some(token), HISTORY).await {
            Ok(p) => p,
            Err(e) => return eprintln!("Could not fill a gap in {}: {e}", room.room_id()),
        };
        let reached = {
            let mut st = store.write();
            st.load_chat(&chat);
            events.iter().filter_map(TimelineEvent::event_id).any(|id| st.find_message(id.as_str()).is_some())
        };
        sync_room(&store, &room, events).await;
        if reached {
            return;
        }
        from = end;
    }
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
    let me = room.own_user_id().to_string();
    for uid in peers.into_iter().chain([me.clone()]) {
        let Ok(parsed) = matrix_sdk::ruma::UserId::parse(&uid) else { continue };
        let receipt = room.load_user_receipt(ReceiptType::Read, &ReceiptThread::Unthreaded, &parsed).await;
        if let Some((ev, r)) = receipt.expect("read receipt") {
            let ts = r.ts.map_or_else(now_unix_millis, |t| i64::from(t.0));
            updates.push((uid, ev.to_string(), ts));
        }
    }
    if updates.is_empty() {
        return;
    }
    let mut st = store.write();
    st.load_chat(&chat);
    let indices = st.timeline_indices(&chat).to_vec();
    for (uid, ev, at) in updates {
        let target = indices.iter().copied().find(|&i| {
            let m = &st.messages[i];
            m.id == ev || m.reactions.iter().any(|r| r.id == ev)
        });
        let Some(target) = target else { continue };
        // Desktop's `seenByMeAt` and `recipientReadTimerStartedAt`: the read starts the timers of the messages
        // that it reads, your own ones for a read of the recipient.
        let read = st.messages[target].sort_key.clone();
        for &i in &indices {
            let m = &st.messages[i];
            let sender = m.is_sender == Some(true);
            let waits = m.disappearing.as_ref().is_some_and(|t| t.waits_for_read(sender));
            if waits && sender != (uid == me) && m.sort_key <= read {
                st.update_message(i, |m| m.disappearing.as_mut().expect("a timer").read_at = Some(at));
            }
        }
        // Desktop's `lastReadMessageSortKey`: the order of the event of your own receipt.
        if uid == me {
            let idx = st.chat_idx(&chat);
            if st.chats[idx].last_read_message_sort_key.as_ref().is_none_or(|k| *k < read) {
                st.update_chat(idx, |c| c.last_read_message_sort_key = Some(read));
            }
            continue;
        }
        let ts = rfc3339_from_millis(at);
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

/// Desktop's bookmark restoration: the whole bookmarks room once, then its new events.
async fn sync_bookmarks(store: &SharedStore, room: &Room, events: Vec<TimelineEvent>) {
    let events = if store.read().bookmarks_room.is_none() { all_history(room).await } else { events };
    let me = room.own_user_id().as_str();
    let changes = events
        .iter()
        .filter_map(|e| bookmark_change(&serde_json::from_str(e.raw().json().get()).expect("timeline event JSON"), me))
        .collect();
    store.write().apply_bookmarks(room.room_id().as_str(), changes);
}

/// Desktop's `applyBookmarkEvent` and `applyBookmarkRedaction`: the message and its new bookmark, if any.
fn bookmark_change(e: &Value, me: &str) -> Option<(String, Option<Bookmark>)> {
    let rel = &e["content"]["com.beeper.relates_to"];
    if rel["rel_type"] != BOOKMARK {
        return None;
    }
    let (target, room_id) = (s(&rel["event_id"])?, s(&rel["room_id"])?);
    match e["type"].as_str()? {
        BOOKMARK if e["sender"] == me => {
            let order = e["unsigned"]["com.beeper.hs.order"].as_i64().unwrap_or(0);
            Some((target, Some(Bookmark { room_id, event_id: s(&e["event_id"])?, order })))
        }
        "m.room.redaction" => Some((target, None)),
        _ => None,
    }
}

/// Desktop's message reminder restoration (`restoreMessageReminders`): the whole reminders room once, then its new
/// events (`handleMessageReminderRoomEvents`).
async fn sync_message_reminders(store: &SharedStore, room: &Room, events: Vec<TimelineEvent>) {
    let events = if store.read().reminders_room.is_none() { all_history(room).await } else { events };
    let me = room.own_user_id().as_str();
    let changes = events
        .iter()
        .filter_map(|e| {
            message_reminder_change(&serde_json::from_str(e.raw().json().get()).expect("timeline event JSON"), me)
        })
        .collect();
    store.write().apply_message_reminders(room.room_id().as_str(), changes);
}

/// Desktop's `applyMessageReminderEvent` and `applyMessageReminderRedaction`: the message and its new reminder with
/// the reminder event, if any. A redacted reminder event changes nothing, and so do the malformed events and the
/// reminders of another sender.
fn message_reminder_change(e: &Value, me: &str) -> Option<(String, Option<(String, MessageReminder)>)> {
    let rel = &e["content"]["com.beeper.relates_to"];
    if rel["rel_type"] != MESSAGE_REMINDER {
        return None;
    }
    let (target, room_id) = (s(&rel["event_id"])?, s(&rel["room_id"])?);
    match e["type"].as_str()? {
        MESSAGE_REMINDER if e["sender"] == me && e["unsigned"]["redacted_because"].is_null() => {
            let remind_at_ms = e["content"]["remind_at_ms"].as_i64().filter(|&at| at > 0)?;
            Some((target, Some((s(&e["event_id"])?, MessageReminder { room_id, remind_at_ms }))))
        }
        "m.room.redaction" => Some((target, None)),
        _ => None,
    }
}

/// Desktop's restoration of the removed messages: the whole removed-messages room once, then its new events.
async fn sync_removals(store: &SharedStore, room: &Room, events: Vec<TimelineEvent>) {
    let events = if store.read().removals.is_none() { all_history(room).await } else { events };
    let me = room.own_user_id().as_str();
    let removals = events
        .iter()
        .filter_map(|e| removal(&serde_json::from_str(e.raw().json().get()).expect("timeline event JSON"), me))
        .collect();
    store.write().apply_removals(removals);
}

/// Desktop's `parseRemovalEvent`: the removed message and the removal event. Desktop does not undo a removal when the
/// removal is redacted, so a redaction is not a change.
fn removal(e: &Value, me: &str) -> Option<(String, String)> {
    let rel = &e["content"]["com.beeper.relates_to"];
    let valid = e["type"] == REMOVED_MESSAGE && e["sender"] == me && rel["rel_type"] == REMOVED_MESSAGE;
    let redacted = !e["unsigned"]["redacted_because"].is_null();
    s(&rel["room_id"]).filter(|_| valid && !redacted)?;
    Some((s(&rel["event_id"])?, s(&e["event_id"])?))
}

/// Desktop's `handleTranscriptionEvent` for the new events of the transcriptions room. Like Desktop, it does not
/// read the older events of the room.
fn sync_transcriptions(store: &SharedStore, events: Vec<TimelineEvent>) {
    let mut st = store.write();
    for e in events {
        if let Some((target, t)) = transcription(&serde_json::from_str(e.raw().json().get()).expect("event JSON")) {
            st.set_transcription(&target, t);
        }
    }
}

/// Desktop's `parseTranscription` (`p`) of a `com.beeper.transcription` event, and the message that it relates to.
fn transcription(e: &Value) -> Option<(String, Transcription)> {
    if e["type"] != TRANSCRIPTION {
        return None;
    }
    let c = &e["content"];
    let rel = &c["com.beeper.relates_to"];
    let plain = |t: &&Value| t["mimetype"].is_null() || t["mimetype"] == "text/plain";
    let text = c["m.text"].as_array()?.iter().rev().filter(plain).find_map(|t| s(&t["body"]))?;
    let target = s(&rel["event_id"]).filter(|_| s(&rel["room_id"]).is_some())?;
    let engine = s(&c["com.beeper.transcription.model"]).unwrap_or_else(|| "unknown".into());
    Some((target, Transcription { transcription: text, engine, language: s(&c["com.beeper.transcription.language"]) }))
}

/// Desktop's `hydrateMissingBookmarks` for one message that is not loaded: the event as a message, or None when
/// the server does not give it.
pub async fn bookmarked_message(client: Client, chat: String, target: String, account_id: String) -> Option<Message> {
    let room = joined_room(&client, &chat).await.ok()?;
    let event = room.event(&OwnedEventId::try_from(target).ok()?, None).await.ok()?;
    let v: Value = serde_json::from_str(event.raw().json().get()).expect("event JSON");
    match op(&room, &chat, &account_id, &v, false).await {
        Some(Op::Upsert(m)) => Some(*m),
        _ => None,
    }
}

async fn all_history(room: &Room) -> Vec<TimelineEvent> {
    let (mut pages, mut from) = (Vec::new(), None);
    loop {
        let (events, end) = match page(room, from, 100).await {
            Ok(p) => p,
            Err(e) => return pages.into_iter().rev().flatten().chain(no_history(room, &e)).collect(),
        };
        pages.push(events);
        match end {
            Some(token) => from = Some(token),
            None => return pages.into_iter().rev().flatten().collect(),
        }
    }
}

/// A redaction with extra content, which `Room::redact` does not send.
async fn redact_with(client: &Client, room: &Room, event: &str, content: Value) -> Result<(), String> {
    let mut url = client.homeserver();
    let txn = matrix_sdk::ruma::TransactionId::new();
    url.path_segments_mut().expect("a base URL").pop_if_empty().extend([
        "_matrix",
        "client",
        "v3",
        "rooms",
        room.room_id().as_str(),
        "redact",
        event,
        txn.as_str(),
    ]);
    let token = client.access_token().ok_or("Not signed in to Matrix")?;
    let res = crate::cloud::http_client()
        .put(url.as_str())
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(content.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    res.error_for_status().map(drop).map_err(|e| e.to_string())
}

async fn sync_space(store: &SharedStore, room: &Room) {
    let bridge = state(room, "m.bridge").await.unwrap_or(Value::Null);
    // A Slack login is one workspace, so the workspace space is the account itself.
    if bridge["com.beeper.bridge_name"] == "slackgo" {
        return;
    }
    let (Some(id), Some(name)) = (s(&bridge["network"]["id"]), s(&bridge["network"]["displayname"])) else { return };
    let mut st = store.write();
    let avatar = s(&bridge["network"]["avatar_url"]).filter(|u| u.starts_with("mxc://"));
    let space = NetworkSpace { id, account_id: account_id(&bridge, &st.bridge_accounts), name, avatar };
    match st.network_spaces.iter().position(|n| n.id == space.id) {
        Some(i) => st.network_spaces[i] = space,
        None => st.network_spaces.push(space),
    }
}

/// Desktop's `createThreadFromInvite`: an invite to a room is a read-only message request, from the stripped state
/// of the room, at the time it came.
async fn sync_invite(store: &SharedStore, room: &Room) {
    let (accounts, rules) = {
        let st = store.read();
        (st.bridge_accounts.clone(), st.inbox_rules.clone())
    };
    let mut chat = chat(room, &accounts, &rules).await;
    // Desktop lists the own invite as a pending member that the inviter added.
    if let Ok(invite) = room.invite_details().await {
        let me = invite.invitee;
        let items = &mut chat.participants.items;
        if !items.iter().any(|u| u.id == me.user_id().as_str()) {
            items.push(User {
                id: me.user_id().to_string(),
                full_name: Some(me.name().to_string()),
                img_url: me.avatar_url().filter(|u| u.is_valid()).map(|u| u.to_string()),
                is_self: Some(true),
                ..Default::default()
            });
        }
        let own = items.iter_mut().find(|u| u.id == me.user_id().as_str()).expect("the own member");
        (own.is_pending, own.added_by) = (Some(true), Some(invite.inviter_id.to_string()));
        chat.participants.total = chat.participants.items.len();
    }
    (chat.is_request, chat.can_accept_request, chat.is_read_only) = (Some(true), true, true);
    // Desktop's `canDeclineRequest` of a Matrix invite is a leave, and an invite has no history to load yet.
    (chat.delete_modes, chat.older) = (chat.delete_modes | DELETE_FOR_ME, Older::Start);
    // Desktop makes an invite a group when it has a name, a photo, or more than two people.
    let group = room.name().is_some() || room.avatar_url().is_some() || chat.participants.items.len() > 2;
    chat.chat_type = if group { "group" } else { "single" }.into();
    let mut st = store.write();
    chat.last_activity = st.find_chat(&chat.id).and_then(|c| c.last_activity.clone()).or(Some(now_rfc3339_millis()));
    st.upsert_chat(chat);
}

/// Desktop keeps a chat that the user left or was removed from (its own member `hasExited`, see `chat`), and the
/// message of that leave previews it (`shouldPreview`). A chat that is gone here, as after Delete Chat, stays gone.
async fn sync_left(store: &SharedStore, room: &Room, events: Vec<TimelineEvent>) {
    let chat = chat_id(room).await;
    if store.read().resolve_chat_idx(&chat).is_none() {
        return;
    }
    let me = room.own_user_id().to_string();
    let exit = events.iter().rev().find_map(|e| {
        let v: Value = serde_json::from_str(e.raw().json().get()).expect("timeline event JSON");
        (v["type"] == "m.room.member" && v["state_key"] == me.as_str()).then(|| s(&v["event_id"])).flatten()
    });
    sync_room(store, room, events).await;
    let mut st = store.write();
    let idx = st.chat_idx(&chat);
    if let Some(m) = exit.and_then(|id| st.loaded(&id).cloned()) {
        st.update_chat(idx, |c| (c.last_activity, c.preview) = (Some(m.timestamp.clone()), Some(m)));
    }
}

pub(crate) async fn sync_room(store: &SharedStore, room: &Room, mut events: Vec<TimelineEvent>) {
    match room.room_id().as_str() {
        BOOKMARKS_ROOM => return sync_bookmarks(store, room, events).await,
        TRANSCRIPTIONS_ROOM => return sync_transcriptions(store, events),
        REMOVED_MESSAGES_ROOM => return sync_removals(store, room, events).await,
        REMINDERS_ROOM => return sync_message_reminders(store, room, events).await,
        id if MANAGED_ROOMS.contains(&id) => return,
        _ => {}
    }
    let create = state(room, "m.room.create").await.expect("rooms have a create event");
    if create["com.beeper.label"] == true {
        return sync_label(store, room).await;
    }
    // Desktop's `computeAccountID`: a space is "$space" and a room of another type is "$other", out of every chat list,
    // but a `com.beeper.local` room is a chat like an untyped one.
    match create["type"].as_str() {
        Some("m.space") => return sync_space(store, room).await,
        Some(t) if t != "com.beeper.local" => return,
        _ => {}
    }
    let chat_id = chat_id(room).await;
    let note = chat_id == NOTE_CHAT_ID;
    let accounts = store.read().bridge_accounts.clone();
    let rules = store.read().inbox_rules.clone();
    let pins = global(&room.client(), "com.beeper.favourites_order").await;
    let rank = pins["Inbox"].as_array().and_then(|ids| ids.iter().position(|id| id == room.room_id().as_str()));
    let mut chat = chat(room, &accounts, &rules).await;
    let seen = {
        let st = store.read();
        st.chat_idx_by_id.get(&chat_id).is_some_and(|&i| st.chats[i].notifiable.is_some())
    };
    let mut older = None;
    if !seen {
        let known: HashSet<String> = events.iter().filter_map(|e| Some(e.event_id()?.to_string())).collect();
        // Without the page, `older` stays unknown and the next load of older messages tries again.
        if let Ok((page, end)) = page(room, None, HISTORY).await.inspect_err(|e| drop(no_history(room, e))) {
            older = Some(end.map_or(Older::Start, Older::From));
            let page = page.into_iter().filter(|e| e.event_id().is_some_and(|id| !known.contains(id.as_str())));
            events = page.chain(events).collect();
        }
    }
    let (mut ops, mut non_messages, mut roots) = (Vec::new(), Vec::new(), BTreeSet::new());
    let hidden = store.read().settings.on("hiddenEvents");
    for e in &events {
        let v: Value = serde_json::from_str(e.raw().json().get()).expect("timeline event JSON");
        ops.extend(s(&v["unsigned"]["transaction_id"]).map(Op::Remove));
        match op(room, &chat_id, &chat.account_id, &v, hidden).await {
            Some(Op::Upsert(m)) => {
                roots.extend(m.thread_root.clone());
                ops.push(Op::Upsert(m));
            }
            o => {
                non_messages.extend(s(&v["event_id"]));
                ops.extend(o);
            }
        }
    }
    let done = account_data(room, "com.beeper.inbox.done").await;
    let done = done.map(|d| (d["at_order"].as_i64(), d["updated_ts"].as_i64()));
    let reacted =
        ops.iter().filter_map(|o| if let Op::React(_, _, m) = o { Some(m.sort_key.clone()) } else { None }).max();
    let redacted = ops.iter().any(|o| matches!(o, Op::Redact(..)));
    let (members, group) = (chat.participants.total, chat.chat_type == "group");
    {
        let mut st = store.write();
        // A pin changed here keeps its local place until the homeserver's `favourites_order` shows it.
        let rank = match st.local_flag(&chat_id, ChatFlag::Pinned) {
            Some(true) => rank.or(st.pin_rank.get(&chat_id).copied()),
            Some(false) => None,
            None => rank,
        };
        let old = match rank {
            Some(r) => st.pin_rank.insert(chat_id.clone(), r),
            None => st.pin_rank.remove(&chat_id),
        };
        if old != rank {
            st.persist_state_to_sqlite();
        }
        st.keep_local_flags(&mut chat);
        if !note {
            st.upsert_chat(chat);
        } else {
            let i = st.chat_idx(NOTE_CHAT_ID);
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
        st.load_chat(&chat_id);
        // Desktop finds the target of an event only in the room of the event.
        let here = |st: &BeeperStore, id: &str| st.loaded_idx(id).filter(|&i| st.messages[i].chat_id == chat_id);
        // A placeholder that decrypted to an edit, a reaction or a hidden event was no message, so its order does not
        // count, like a redacted one: Desktop's `countsAsUnread` row of an edit is the edited message.
        let mut stale = redacted;
        for id in non_messages {
            if here(&st, &id).is_some() {
                st.remove_message(&chat_id, &id).expect("remove a decrypted placeholder");
                stale = true;
            }
        }
        for op in ops {
            match op {
                Op::Upsert(m) => st.upsert_message(*m),
                Op::Vote(target, voter, answers) if here(&st, &target).is_some() => {
                    st.update_poll(&target, |p| p.respond(&voter, &answers));
                }
                Op::EndPoll(target) if here(&st, &target).is_some() => st.update_poll(&target, |p| p.ended = true),
                Op::Vote(..) | Op::EndPoll(_) => {}
                Op::SendStatus(target, delivered, error, status, timer) => {
                    let i = here(&st, &target);
                    if let Some((i, timer)) = i.zip(timer) {
                        apply_timer(&mut st, i, timer);
                    }
                    let m = |i: usize| &st.messages[i];
                    let changed = |i| {
                        (m(i).is_delivered, &m(i).send_error, m(i).send_status.as_ref())
                            != (delivered, &error, Some(&*status))
                    };
                    if let Some(i) = i.filter(|&i| changed(i)) {
                        st.update_message(i, |m| {
                            (m.is_delivered, m.send_error, m.send_status) = (delivered, error, Some(*status))
                        });
                    }
                }
                Op::React(target, r, mut preview) => {
                    let target_idx = here(&st, &target);
                    let target_msg = target_idx.map(|i| &st.messages[i]);
                    let target_mine = target_msg.is_some_and(|m| m.is_sender == Some(true));
                    let who = match preview.is_sender == Some(true) {
                        true => Some("You"),
                        false => group.then(|| preview.sender_name.as_deref().expect("sender name")),
                    };
                    preview.text = Some(reaction_text(&r.reaction_key, target_msg, who));
                    if let Some(i) = target_idx {
                        st.update_message(i, |m| {
                            m.reactions.retain(|x| x.id != r.id);
                            m.reactions.push(r);
                        });
                    }
                    let idx = st.chat_idx(&chat_id);
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
                Op::Timer(target, timer) => {
                    if let Some(i) = here(&st, &target) {
                        apply_timer(&mut st, i, timer);
                    }
                }
                Op::Edit {
                    target,
                    text,
                    html,
                    links,
                    attachments,
                    message_type,
                    unresolved_media,
                    timestamp,
                    timer,
                } => {
                    if let Some(i) = here(&st, &target) {
                        if let Some(timer) = timer {
                            apply_timer(&mut st, i, timer);
                        }
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
                Op::Remove(id) if here(&st, &id).is_some() => {
                    st.remove_message(&chat_id, &id).expect("remove a local echo");
                }
                Op::Redact(target, expired) if here(&st, &target).is_some() => {
                    match expired {
                        true => st.expire_message(&chat_id, &target),
                        false => st.delete_message(&chat_id, &target),
                    }
                    .expect("redact a known message");
                }
                // Desktop's `MIN(remainingCount)` of the updates of a message.
                Op::ViewsLeft(target, left) => {
                    let i = here(&st, &target).filter(|&i| st.messages[i].view_limit.is_some());
                    if let Some(i) = i.filter(|&i| st.messages[i].views_left.is_none_or(|n| left < n)) {
                        st.update_message(i, |m| m.views_left = Some(left));
                    }
                }
                Op::Remove(id) | Op::Redact(id, _) => {
                    let reacted = st
                        .timeline_indices(&chat_id)
                        .iter()
                        .copied()
                        .find(|&i| st.messages[i].reactions.iter().any(|r| r.id == id));
                    if let Some(i) = reacted {
                        st.update_message(i, |m| m.reactions.retain(|r| r.id != id));
                    }
                }
            }
        }
        let idx = st.chat_idx(&chat_id);
        if let Some(older) = older {
            st.update_chat(idx, |c| c.older = older);
        }
        let msgs = st.timeline(&chat_id);
        let last = msgs.clone().rev().find(|m| m.message_type.as_deref() != Some("MEMBERSHIP")).map(|m| &m.sort_key);
        let stored = msgs.flat_map(|m| &m.reactions).map(|r| &r.sort_key).max();
        let old = st.chats[idx].notifiable.as_ref().filter(|_| !stale);
        let notifiable = [old, last, reacted.as_ref(), stored].into_iter().flatten().max().cloned();
        let notifiable = Some(notifiable.unwrap_or_else(|| sort_key(MIN_SAFE_INTEGER)));
        let archived = st
            .local_flag(&chat_id, ChatFlag::Archived)
            .unwrap_or_else(|| archived_upto(&st.chats[idx], done, notifiable.as_deref().expect("a notifiable order")));
        if (st.chats[idx].is_archived, &st.chats[idx].notifiable) != (Some(archived), &notifiable) {
            st.update_chat(idx, |c| (c.is_archived, c.notifiable) = (Some(archived), notifiable));
        }
    }
    sync_thread_reads(store, room, roots).await;
    sync_receipts(store, room).await;
}

/// Desktop's `applyDisappearingTimer`: the timer of a later event replaces the one of the message.
fn apply_timer(st: &mut BeeperStore, i: usize, timer: MessageTimer) {
    let new = MessageTimer::merge(st.messages[i].disappearing.clone(), Some(timer));
    if st.messages[i].disappearing != new {
        st.update_message(i, |m| m.disappearing = new);
    }
}

/// Desktop's `archiveThread` content of `com.beeper.inbox.done`: archived up to `order` at `now`, or an empty object to
/// unarchive.
fn inbox_done(order: Option<i64>, now: i64) -> Value {
    match order {
        Some(order) => serde_json::json!({ "at_ts": now, "updated_ts": now, "at_order": order }),
        None => serde_json::json!({}),
    }
}

/// Desktop's `isArchived` of a thread from its `com.beeper.inbox.done` (`at_order`, `updated_ts`): with an order, the
/// chat is archived up to that homeserver order; with only a time, up to that time when nothing is unread.
fn archived_upto(c: &Chat, done: Option<(Option<i64>, Option<i64>)>, notifiable: &str) -> bool {
    match done {
        _ if c.flag(ChatFlag::Pinned) => false,
        Some((Some(order), _)) => sort_key(order).as_str() >= notifiable,
        Some((None, Some(at))) => {
            let unread = c.is_unread();
            at >= crate::store::activity_key(c).0 && at < now_unix_millis() && !unread
        }
        _ => false,
    }
}

/// Desktop's `lastReadHsOrder` of the threads of `roots`: the own thread receipt, and for a new thread the own
/// receipt of the room, like Desktop's `loadThread`.
async fn sync_thread_reads(store: &SharedStore, room: &Room, roots: BTreeSet<String>) {
    if roots.is_empty() {
        return;
    }
    let me = room.own_user_id();
    let receipt = async |thread: ReceiptThread| {
        let r = room.load_user_receipt(ReceiptType::Read, &thread, me).await.expect("read receipt");
        r.map(|(event, _)| event.to_string())
    };
    let main = [receipt(ReceiptThread::Unthreaded).await, receipt(ReceiptThread::Main).await];
    let mut reads = Vec::new();
    for root in roots {
        let thread = OwnedEventId::try_from(root.as_str()).ok().map(ReceiptThread::Thread);
        reads.push((
            root,
            match thread {
                Some(t) => receipt(t).await,
                None => None,
            },
        ));
    }
    let chat = chat_id(room).await;
    let mut st = store.write();
    let key = |st: &BeeperStore, e: &Option<String>| e.as_ref().and_then(|e| Some(st.stored_message(e)?.sort_key));
    let main = main.iter().filter_map(|e| key(&st, e)).max().unwrap_or_else(|| sort_key(MIN_SAFE_INTEGER));
    let idx = st.chat_idx(&chat);
    let old = &st.chats[idx].thread_reads;
    let new: Vec<(String, String)> = reads
        .into_iter()
        .map(|(root, e)| {
            let read = old.get(&root).cloned().unwrap_or_else(|| main.clone()).max(key(&st, &e).unwrap_or_default());
            (root, read)
        })
        .filter(|(root, read)| old.get(root) != Some(read))
        .collect();
    if !new.is_empty() {
        st.update_chat(idx, |c| c.thread_reads.extend(new));
    }
}

/// A mention pill of the composer: the label after "@" and the user ID, or [`ROOM_MENTION`] for "@room".
pub type Mention = (String, String);

/// Desktop's ID of the "@room" mention.
pub const ROOM_MENTION: &str = "@room";

/// Desktop's `threadSendOptions`: a reply in the thread of `root` to `quoted`, which the user `chosen` or which is the
/// newest message of the thread, and whether Desktop's "Also send to chat" is on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadSend {
    pub root: String,
    pub quoted: String,
    pub chosen: bool,
    pub broadcast: bool,
}

/// A change for the homeserver. The outbox table of the store keeps it until the homeserver takes it.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Outgoing {
    Send {
        txn: String,
        text: String,
        reply_to: Option<String>,
        /// A link preview with the `og:image:type` of its image.
        preview: Option<crate::cloud::PagePreview>,
        mentions: Vec<Mention>,
        /// The thread of a thread reply, whose `quoted` is `reply_to`.
        #[serde(default)]
        thread: Option<ThreadSend>,
        /// The text has links that go without a preview: Desktop then sends an empty list, so the bridge makes none.
        #[serde(default)]
        links: bool,
    },
    Edit {
        target: String,
        text: String,
        mentions: Vec<Mention>,
        /// Whether the message is an attachment, whose caption the text is.
        #[serde(default)]
        caption: bool,
    },
    Redact {
        target: String,
    },
    React {
        txn: String,
        target: String,
        key: String,
        /// The image of a custom emoji, whose shortcode `key` is.
        #[serde(default)]
        img_url: Option<String>,
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
    /// Leaves the room, and forgets it for Desktop's `deleteLabel`.
    Leave(bool),
    /// A state event with an empty state key.
    State(String, Value),
    /// Desktop's `removeParticipant`: a leave membership for the user.
    Kick(String),
    /// Desktop's `addParticipant`: an invite for the user.
    Invite(String),
    /// Desktop's `changeParticipantRole`: the admin power level for the user (true), or the default one.
    Role(String, bool),
    /// Adds (true) or removes a Desktop bookmark of a message in a room. Its chat is the bookmarks room.
    Bookmark {
        room: String,
        target: String,
        on: bool,
    },
    /// Desktop's `sendMessageReminder` (`at`) or `redactMessageReminder` (None) of a message in a room. Its chat is
    /// the reminders room.
    MessageReminder {
        room: String,
        target: String,
        at: Option<i64>,
    },
    /// Desktop's `removeMessageFromBeeper`: a removal of a message in a room. Its chat is the removed-messages room.
    RemoveFromBeeper {
        txn: String,
        room: String,
        target: String,
    },
    /// Desktop's `setTranscription`: the transcription of a message in a room. Its chat is the transcriptions room.
    Transcription {
        room: String,
        target: String,
        transcription: Transcription,
    },
    /// Uploads an image with its MIME type and makes it the room avatar, or removes the avatar.
    Avatar(Option<(Vec<u8>, String)>),
    /// Sets room account data of this type.
    RoomData(String, Value),
    /// Uploads an image with its MIME type and makes it the personal photo of the room, or removes that photo.
    PersonalAvatar(Option<(Vec<u8>, String)>),
    /// Adds (true) or removes a chat from a label.
    LabelChild(String, bool),
    Read(String),
    /// Desktop's `sendThreadReadReceipt`: a read receipt for `event` in the thread of `root`.
    ThreadRead {
        root: String,
        event: String,
    },
    Flag(ChatFlag, bool),
    Reminder(Option<ChatReminder>),
    /// Desktop's `com.beeper.thread_follows` room account data with a new `com.beeper.revision_id`.
    ThreadFollows(ThreadFollows),
    DeleteChat {
        for_everyone: bool,
    },
    AcceptRequest,
    /// Desktop's `blockUser`: the bridge blocks (true) or unblocks the other person of a DM, and can report spam.
    BlockUser {
        block: bool,
        report_spam: bool,
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
        /// The `bytes` column of the outbox row keeps the file.
        #[serde(skip)]
        bytes: Vec<u8>,
        name: String,
        mime: String,
        info: MediaInfo,
        caption: Option<String>,
        reply_to: Option<String>,
        /// The thread of a thread reply, whose `quoted` is `reply_to`.
        #[serde(default)]
        thread: Option<ThreadSend>,
        /// Desktop's `isGif`: a GIF, or the MP4 of a GIF.
        #[serde(default)]
        gif: bool,
        /// Desktop's `stickerPackImage`: the pack image of a sticker from the sticker picker.
        #[serde(default)]
        sticker: Option<Box<Sticker>>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum MediaInfo {
    /// The width and height of an image.
    Image(u32, u32),
    /// A recorded voice note: its length and its waveform, with values from 0 to 1.
    Voice { ms: u64, wave: Vec<f32> },
    /// The width, height and length in ms of a video.
    Video(u32, u32, u64),
    /// The length in ms of an audio file.
    Audio(u64),
    /// An other file, which Matrix shows by its MIME type.
    File,
}

impl MediaInfo {
    /// The info of a file that is not a voice note, by its MIME type, width, height and length in seconds.
    pub fn of(mime: &str, dims: Option<(u32, u32)>, duration: Option<f64>) -> MediaInfo {
        let ms = duration.map(|s| (s * 1000.0).round() as u64);
        match (mime.split('/').next(), dims, ms) {
            (Some("image"), Some((w, h)), _) => MediaInfo::Image(w, h),
            (Some("video"), dims, Some(ms)) => {
                let (w, h) = dims.unwrap_or_default();
                MediaInfo::Video(w, h, ms)
            }
            (Some("audio"), _, Some(ms)) => MediaInfo::Audio(ms),
            _ => MediaInfo::File,
        }
    }

    /// The width and height, and the length in seconds.
    fn facts(&self) -> (Option<(u32, u32)>, Option<f64>) {
        let s = |ms: &u64| Some(*ms as f64 / 1000.0);
        match self {
            MediaInfo::Image(w, h) => (Some((*w, *h)), None),
            MediaInfo::Voice { ms, .. } | MediaInfo::Audio(ms) => (None, s(ms)),
            MediaInfo::Video(w, h, ms) => (Some((*w, *h)), s(ms)),
            MediaInfo::File => (None, None),
        }
    }
}

impl Outgoing {
    /// The local echo that the change sends, or the message that it edits.
    pub fn message(&self) -> Option<&str> {
        match self {
            Outgoing::Send { txn, .. }
            | Outgoing::Poll { txn, .. }
            | Outgoing::Forward { txn, .. }
            | Outgoing::Media { txn, .. } => Some(txn),
            Outgoing::Edit { target, .. } => Some(target),
            _ => None,
        }
    }
}

/// matrix-sdk's `make_media_type` without a thumbnail: the message of an uploaded file, with the caption as the body.
fn media_type(
    mime: &mime::Mime,
    name: String,
    source: MediaSource,
    caption: Option<String>,
    info: matrix_sdk::attachment::AttachmentInfo,
) -> matrix_sdk::ruma::events::room::message::MessageType {
    use matrix_sdk::attachment::AttachmentInfo;
    use matrix_sdk::ruma::events::room::message::*;
    use matrix_sdk::ruma::events::room::ImageInfo;
    let (body, filename) = match caption {
        Some(caption) => (caption, Some(name)),
        None => (name, None),
    };
    let mimetype = Some(mime.as_ref().to_owned());
    match mime.type_() {
        mime::IMAGE => {
            let mut info = ImageInfo::from(info);
            info.mimetype = mimetype;
            let mut c = ImageMessageEventContent::new(body, source).info(Some(Box::new(info)));
            c.filename = filename;
            MessageType::Image(c)
        }
        mime::AUDIO => {
            let mut c = AudioMessageEventContent::new(body, source);
            if let AttachmentInfo::Audio(a) | AttachmentInfo::Voice(a) = &info {
                if let (Some(duration), Some(wave)) = (a.duration, &a.waveform) {
                    let max = f32::from(UnstableAmplitude::MAX);
                    let wave = wave.iter().map(|v| ((v.clamp(0.0, 1.0) * max) as u16).into()).collect();
                    c.audio = Some(UnstableAudioDetailsContentBlock::new(duration, wave));
                }
            }
            if matches!(info, AttachmentInfo::Voice(_)) {
                c.voice = Some(UnstableVoiceContentBlock::new());
            }
            let mut info = AudioInfo::from(info);
            info.mimetype = mimetype;
            let mut c = c.info(Box::new(info));
            c.filename = filename;
            MessageType::Audio(c)
        }
        mime::VIDEO => {
            let mut info = VideoInfo::from(info);
            info.mimetype = mimetype;
            let mut c = VideoMessageEventContent::new(body, source).info(Some(Box::new(info)));
            c.filename = filename;
            MessageType::Video(c)
        }
        _ => {
            let mut info = FileInfo::from(info);
            info.mimetype = mimetype;
            let mut c = FileMessageEventContent::new(body, source).info(Some(Box::new(info)));
            c.filename = filename;
            MessageType::File(c)
        }
    }
}

/// Desktop's `prepareOutgoingMedia` for a room with room features, on a worker thread: the file to upload and its info.
async fn prepare_media(
    store: &SharedStore,
    chat: &str,
    txn: &str,
    data: Vec<u8>,
    (name, mime): (String, String),
    info: MediaInfo,
    sticker: bool,
) -> Result<(Vec<u8>, String, String, MediaInfo), String> {
    use crate::media::Sent;
    let (rules, dir) = {
        let st = store.read();
        let c = &st.chats[st.resolve_chat_idx(chat).expect("the chat of a send exists")];
        (c.room_features.then(|| c.file.clone()), st.config.data_dir.join("media").join("outgoing-uploads").join(txn))
    };
    let Some(rules) = rules else { return Ok((data, name, mime, info)) };
    let (dims, duration) = info.facts();
    let voice = matches!(info, MediaInfo::Voice { .. });
    let sent = match (voice, sticker) {
        (true, _) => Sent::Voice,
        (_, true) => Sent::Sticker,
        _ => Sent::File,
    };
    let f = crate::media::Upload { name, path: Default::default(), mime, dims, duration, failed: false };
    let job = move || crate::media::prepare_upload(&dir, &data, f, &rules, sent);
    let (data, f) = tokio::task::spawn_blocking(job).await.expect("the media thread")?;
    let info = if voice { info } else { MediaInfo::of(&f.mime, f.dims, f.duration) };
    Ok((data, f.name, f.mime, info))
}

/// The local echo of the file of `txn`, when there is one, takes the name and the type of the converted file.
fn converted(store: &SharedStore, txn: &str, name: &str, mime: &str) {
    let mut st = store.write();
    let Some(i) = st.find_message(txn) else { return };
    let a = &st.messages[i].attachments[0];
    if (a.file_name.as_deref(), a.mime_type.as_deref()) != (Some(name), Some(mime)) {
        st.update_message(i, |m| {
            (m.attachments[0].file_name, m.attachments[0].mime_type) = (Some(name.into()), Some(mime.into()));
        });
    }
}

/// Desktop's `sendMessageInternal` of a sticker: the content of the image without `msgtype` and `filename`, with the
/// body of the pack image, Desktop's info, the thumbnail of the pack info when it tells of the same file (`FI`), and
/// the pack of the sticker (`com.beeper.msc4459.image_source_packs`).
fn sticker_content(content: &mut Value, s: Box<Sticker>, caption: bool, dims: Option<(u32, u32)>) {
    let c = content.as_object_mut().expect("message JSON");
    c.remove("msgtype");
    c.remove("filename");
    if !caption {
        let body = s.body.or(s.source.as_ref().map(|x| x.shortcode.clone())).unwrap_or_else(|| "Sticker".into());
        c.insert("body".into(), body.into());
    }
    let mut info = serde_json::Map::new();
    info.insert("mimetype".into(), c["info"]["mimetype"].clone());
    if let Some((w, h)) = dims {
        info.insert("w".into(), w.into());
        info.insert("h".into(), h.into());
    }
    info.insert("size".into(), c["info"]["size"].clone());
    let same = ["mimetype", "w", "h"].iter().all(|k| s.info[k] == info[*k])
        && (s.info["size"].is_null() || s.info["size"] == info["size"]);
    for k in ["thumbnail_url", "thumbnail_info", "thumbnail_file"].into_iter().filter(|k| same && !s.info[k].is_null())
    {
        info.insert(k.into(), s.info[k].clone());
    }
    c.insert("info".into(), info.into());
    let url = c
        .get("url")
        .or(c.get("file").map(|f| &f["url"]))
        .and_then(Value::as_str)
        .expect("an uploaded file")
        .to_string();
    if let Some(x) = s.source {
        let source =
            serde_json::json!({ url: { "room_id": x.room_id, "state_key": x.state_key, "shortcode": x.shortcode } });
        c.insert("com.beeper.msc4459.image_source_packs".into(), source);
    }
}

/// Pushes the outbox to the homeserver. A message that fails keeps an error, like Desktop's failed echoes. A removal
/// that fails stays pending for the next launch, like Desktop's `resendPendingRemovals`. Other changes that fail are
/// dropped, and the error comes back.
async fn flush(store: SharedStore) -> Result<(), PushError> {
    loop {
        let Some((row, chat, out)) = store.write().next_outgoing() else { return Ok(()) };
        // A failed change waits for Retry only with a local echo that shows the error.
        let message = out.message().filter(|id| store.read().stored_message(id).is_some()).map(str::to_string);
        let retry = !matches!(out, Outgoing::Edit { .. });
        let pending = matches!(out, Outgoing::RemoveFromBeeper { .. });
        let flag = match out {
            Outgoing::Flag(f, on) => Some((chat.clone(), f, on)),
            _ => None,
        };
        let r = push(&store, chat, out).await;
        let mut st = store.write();
        match (r, message) {
            (Ok(()), message) => {
                if let Some((chat, f, on)) = flag {
                    st.flag_sent(chat, f, on);
                }
                st.pushed(row, message.as_deref(), None);
            }
            // Like the send queue of Desktop's SDK, a change that the network or the homeserver could not take now
            // stays pending, and the push tries it again later.
            (Err(e), _) if e.transient => {
                st.pushed(row, None, Some((e.message.clone(), true)));
                return Err(e);
            }
            (Err(e), Some(message)) => st.pushed(row, Some(&message), Some((e.message, retry))),
            (Err(e), None) => {
                if !pending {
                    st.pushed(row, None, None);
                }
                return Err(e);
            }
        }
    }
}

/// The push task of the sync loop: it pushes the outbox at the start and after each change that the store queues.
/// After a transient error it tries again after 1 s, then twice as long each time up to a minute.
async fn push_loop(store: SharedStore) {
    let ready = store.read().outbox_ready.clone();
    let mut wait = None;
    loop {
        match flush(store.clone()).await {
            Ok(()) => wait = None,
            Err(e) if e.transient => wait = Some(wait.map_or(1, |w: u64| (w * 2).min(60))),
            Err(e) => {
                store.write().matrix_error = Some(format!("Could not send to Matrix: {}", e.message));
                wait = None;
            }
        }
        match wait {
            Some(s) => drop(tokio::time::timeout(std::time::Duration::from_secs(s), ready.notified()).await),
            None => ready.notified().await,
        }
    }
}

/// An error of a push: its message, and whether the network or the homeserver could take the change later.
#[derive(Debug)]
struct PushError {
    message: String,
    transient: bool,
}

impl From<String> for PushError {
    fn from(message: String) -> Self {
        Self { message, transient: false }
    }
}

impl From<matrix_sdk::Error> for PushError {
    /// Like matrix-sdk's `RetryKind`: a network failure, a rate limit, or a server error can pass later.
    fn from(x: matrix_sdk::Error) -> Self {
        let transient = match &x {
            matrix_sdk::Error::Http(h) => match h.as_client_api_error() {
                Some(api) => api.status_code.as_u16() == 429 || api.status_code.is_server_error(),
                None => matches!(**h, matrix_sdk::HttpError::Reqwest(_)),
            },
            _ => false,
        };
        Self { message: x.to_string(), transient }
    }
}

/// The room of a chat: a joined room, or an invite or a room the user left, which keep their chats.
async fn chat_room(client: &matrix_sdk::Client, chat: &str) -> Result<Room, String> {
    let known = matrix_sdk::ruma::RoomId::parse(chat).ok().and_then(|id| client.get_room(&id));
    match known {
        Some(room) if matches!(room.state(), RoomState::Invited | RoomState::Left) => Ok(room),
        _ => joined_room(client, chat).await,
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

/// Desktop's `loadMessage`: one message of a chat by its event, outside the timeline, as a quote of a reply shows it.
pub async fn fetch_message(store: SharedStore, chat: String, id: String) -> Result<Message, String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected")?;
    let room = chat_room(&client, &chat).await?;
    let event =
        room.event(&OwnedEventId::try_from(id).map_err(|e| e.to_string())?, None).await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(event.raw().json().get()).map_err(|e| e.to_string())?;
    let (account, hidden) = {
        let st = store.read();
        (st.find_chat(&chat).map(|c| c.account_id.clone()).unwrap_or_default(), st.settings.on("hiddenEvents"))
    };
    match op(&room, &chat, &account, &v, hidden).await {
        Some(Op::Upsert(m)) => Ok(*m),
        _ => Err("The event is not a message".into()),
    }
}

/// Joins a room by its ID or alias, and returns the room ID.
pub async fn join(store: SharedStore, room: String) -> Result<String, String> {
    let (room, vias) = room_address(&room).map_err(|e| format!("{e:?}"))?;
    join_via(store, room, vias).await
}

/// Joins a room by its ID or alias through the `vias` servers, and returns the room ID. Like Desktop's `Uh`, it waits
/// up to 10 seconds for the sync to bring the room into the store, so that the caller can open it.
pub async fn join_via(store: SharedStore, room: String, vias: Vec<String>) -> Result<String, String> {
    let room = matrix_sdk::ruma::RoomOrAliasId::parse(room).map_err(|e| e.to_string())?;
    let vias: Vec<matrix_sdk::ruma::OwnedServerName> =
        vias.into_iter().map(|v| v.try_into().expect("room_address checks the servers")).collect();
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    let mut changes = store.read().ws_tx.subscribe();
    let id = client.join_room_by_id_or_alias(&room, &vias).await.map_err(|e| e.to_string())?.room_id().to_string();
    let synced = async {
        while store.read().resolve_chat_idx(&id).is_none() {
            if let Err(tokio::sync::broadcast::error::RecvError::Closed) = changes.recv().await {
                return;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), synced).await;
    Ok(id)
}

/// Why `room_address` refuses an input, like Desktop's reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadAddress {
    Empty,
    MixedSigil,
    Invalid,
}

/// Desktop's `parseMatrixRoomInput` (`Lh`): a room alias (`#`), a room ID (`!`), a `https://matrix.to/#/` link, or a
/// `matrix:` URI, with the `via` servers of a link. Gives the room and the servers.
pub fn room_address(input: &str) -> Result<(String, Vec<String>), BadAddress> {
    // Desktop's `xr`: `^(?:[a-zA-Z0-9.-]+|\[[0-9a-fA-F:.]+\])(?::([0-9]{1,5}))?$` with a port up to 65535.
    fn server(s: &str) -> bool {
        let (host_ok, port) = match s.strip_prefix('[') {
            Some(rest) => match rest.split_once(']') {
                Some((ip, after)) => {
                    (!ip.is_empty() && ip.chars().all(|c| c.is_ascii_hexdigit() || ":.".contains(c)), after)
                }
                None => return false,
            },
            None => {
                let (host, port) = s.find(':').map_or((s, ""), |i| s.split_at(i));
                (!host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)), port)
            }
        };
        let port_ok = match port.strip_prefix(':') {
            Some(p) => {
                (1..=5).contains(&p.len())
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && p.parse::<u32>().is_ok_and(|n| n <= 65535)
            }
            None => port.is_empty(),
        };
        host_ok && port_ok
    }
    // Desktop's `Yo`: the sigil, at most 255 bytes, and a local part and a valid server after the first ':'.
    fn valid(id: &str, sigil: char) -> bool {
        if !id.starts_with(sigil) || id.len() > 255 {
            return false;
        }
        match id[1..].find(':').map(|i| i + 1) {
            None => sigil == '!' && id.len() > 1 && !id.contains('\0'),
            Some(1) => false,
            Some(i) => !id[1..i].contains('\0') && server(&id[i + 1..]),
        }
    }
    let target = |id: String, vias: Vec<String>| {
        let sigil = if id.starts_with('#') { '#' } else { '!' };
        (valid(&id, sigil)).then_some((id, vias)).ok_or(BadAddress::Invalid)
    };
    let decode = |s: &str| crate::codec::url_decode(s);
    let vias = |query: &str| -> Option<Vec<String>> {
        let vias: Vec<String> = query
            .split('&')
            .filter_map(|kv| kv.strip_prefix("via="))
            .map(|v| decode(v).unwrap_or_default())
            .filter(|v| !v.is_empty())
            .collect();
        vias.iter().all(|v| server(v)).then_some(vias)
    };
    let t = input.trim();
    if t.is_empty() {
        return Err(BadAddress::Empty);
    }
    if t.starts_with("#!") {
        return Err(BadAddress::MixedSigil);
    }
    if t.starts_with('#') || t.starts_with('!') {
        return target(t.into(), Vec::new());
    }
    if let Some(rest) = t.strip_prefix("https://matrix.to/#/") {
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut parts = path.split('/');
        let id = parts.next().and_then(decode).ok_or(BadAddress::Invalid)?;
        let event = parts.next().map(|e| decode(e).is_some_and(|e| e.starts_with('$')));
        if event == Some(false) || parts.next().is_some() {
            return Err(BadAddress::Invalid);
        }
        return target(id, vias(query).ok_or(BadAddress::Invalid)?);
    }
    if let Some(rest) = t.strip_prefix("matrix:") {
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path.split('/').collect();
        let id = parts.get(1).and_then(|p| decode(p)).ok_or(BadAddress::Invalid)?;
        let event_ok = parts.len() == 2 || (parts.len() == 4 && parts[2] == "e" && decode(parts[3]).is_some());
        let sigil = match parts[0] {
            "r" | "room" => '#',
            "roomid" => '!',
            _ => return Err(BadAddress::Invalid),
        };
        if path.starts_with('/') || !event_ok {
            return Err(BadAddress::Invalid);
        }
        return target(format!("{sigil}{id}"), vias(query).ok_or(BadAddress::Invalid)?);
    }
    Err(BadAddress::Invalid)
}

pub async fn typing(client: matrix_sdk::Client, chat: String, on: bool) -> Result<(), String> {
    joined_room(&client, &chat).await?.typing_notice(on).await.map_err(|e| e.to_string())
}

/// Desktop's `formatMessage`: the text, its HTML with a `matrix.to` link for each user mention, and `m.mentions` with
/// the mentioned users, the sender of `reply_to`, and "@room".
/// Uploads the image of a link preview: the key of the preview that names it, and its value.
async fn preview_image(
    client: &Client,
    room: &Room,
    p: &Value,
    bytes: Vec<u8>,
) -> Result<(&'static str, Value), String> {
    let e = |x: &dyn std::fmt::Display| x.to_string();
    if room.latest_encryption_state().await.map_err(|x| e(&x))?.is_encrypted() {
        let file = client.upload_encrypted_file(&mut bytes.as_slice()).await.map_err(|x| e(&x))?;
        return Ok(("beeper:image:encryption", serde_json::to_value(file).expect("encrypted file JSON")));
    }
    let mime: mime::Mime = p["og:image:type"].as_str().ok_or("no image type")?.parse().map_err(|x| e(&x))?;
    let uri = client.media().upload(&mime, bytes, None).await.map_err(|x| e(&x))?.content_uri;
    Ok(("og:image", uri.to_string().into()))
}

/// Desktop's `getCaptionEditableAttachmentContent`: the media fields of the attachment message `target` (as last
/// edited), which an edit of its caption keeps.
async fn caption_attachment(
    room: &Room,
    target: &matrix_sdk::ruma::EventId,
) -> Result<serde_json::Map<String, Value>, String> {
    let unavailable = || "Attachment event content is unavailable".to_string();
    let event = room.event(target, None).await.map_err(|_| unavailable())?;
    let event: Value = serde_json::from_str(event.raw().json().get()).map_err(|e| e.to_string())?;
    let content = event["content"].get("m.new_content").unwrap_or(&event["content"]);
    let media = ["m.image", "m.video", "m.audio", "m.file"].contains(&content["msgtype"].as_str().unwrap_or_default());
    if event["type"] != "m.room.message" || !media || (content.get("url").is_none() && content.get("file").is_none()) {
        return Err(unavailable());
    }
    let mut fields: serde_json::Map<String, Value> =
        ["url", "file", "info", "org.matrix.msc3245.voice", "org.matrix.msc1767.audio", "msgtype"]
            .into_iter()
            .filter_map(|k| Some((k.to_string(), content.get(k)?.clone())))
            .collect();
    let filename = content.get("filename").or_else(|| content.get("body")).cloned().unwrap_or_default();
    fields.insert("filename".into(), filename);
    Ok(fields)
}

/// Desktop's `editMessage` content: the new text in `m.new_content`, with the media of an attachment whose caption
/// it is (an empty caption leaves the file name), and the plain text as the fallback, without a `* ` prefix.
fn edit_content(
    target: &matrix_sdk::ruma::EventId,
    text: String,
    mentions: &[Mention],
    attachment: Option<serde_json::Map<String, Value>>,
) -> Result<Value, String> {
    let caption = text.trim().is_empty().then(|| attachment.as_ref().map(|a| a["filename"].clone())).flatten();
    let content = text_content(text, mentions, None)?;
    let mut new = serde_json::to_value(&content).expect("message content JSON");
    let mentions = new.as_object_mut().and_then(|m| m.remove("m.mentions"));
    let fields = attachment.unwrap_or_default();
    for (k, v) in &fields {
        new[k] = v.clone();
    }
    if let Some(name) = caption {
        new["body"] = name;
        let new = new.as_object_mut().expect("an object");
        new.remove("format");
        new.remove("formatted_body");
    }
    let mut outer = serde_json::json!({ "msgtype": new["msgtype"], "body": new["body"] });
    for (k, v) in fields {
        outer[k] = v;
    }
    outer["m.relates_to"] = serde_json::json!({ "event_id": target, "rel_type": "m.replace" });
    outer["m.new_content"] = new;
    if let Some(mentions) = mentions {
        outer["m.mentions"] = mentions;
    }
    Ok(outer)
}

fn text_content(text: String, mentions: &[Mention], reply_to: Option<&str>) -> Result<RoomMessageEventContent, String> {
    let users: Vec<&Mention> = mentions.iter().filter(|m| m.1 != ROOM_MENTION).collect();
    let ids: Result<BTreeSet<OwnedUserId>, _> =
        users.iter().map(|m| m.1.as_str()).chain(reply_to).map(OwnedUserId::try_from).collect();
    let mut m = Mentions::with_user_ids(ids.map_err(|e| e.to_string())?);
    m.room = mentions.iter().any(|m| m.1 == ROOM_MENTION);
    let mut content = match crate::codec::markdown_html(&mention_links(&text, &users)) {
        Some(html) => RoomMessageEventContent::text_html(text, html),
        None => RoomMessageEventContent::text_plain(text),
    };
    content.mentions = Some(m);
    Ok(content)
}

/// The Markdown of Desktop's mention pills: each "@label" of `users` becomes `[@label](https://matrix.to/#/id)`, the
/// longest label first.
fn mention_links(text: &str, users: &[&Mention]) -> String {
    let (mut md, mut rest) = (String::with_capacity(text.len()), text);
    while let Some(i) = rest.find('@') {
        md.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        match users.iter().filter(|m| rest.starts_with(m.0.as_str())).max_by_key(|m| m.0.len()) {
            Some((label, id)) => {
                let escaped: String =
                    label.chars().flat_map(|c| "\\[]*_`~".contains(c).then_some('\\').into_iter().chain([c])).collect();
                md.push_str(&format!("[@{escaped}](https://matrix.to/#/{id})"));
                rest = &rest[label.len()..];
            }
            None => md.push('@'),
        }
    }
    md + rest
}

/// Desktop's `disappearingTimerForMessages`: the timer state of the room, which new messages carry, but not an empty
/// one when the network omits it (`omit_empty_timer`, which the Beeper defaults of a room without features have).
async fn timer_for_messages(room: &Room) -> Option<Value> {
    let timer = state(room, DISAPPEARING_TIMER).await?;
    let features = state(room, "com.beeper.room_features").await;
    let omit = features.is_none_or(|f| f["disappearing_timer"]["omit_empty_timer"] == true);
    (!(timer["timer"].is_null() && omit)).then_some(timer)
}

async fn push(store: &SharedStore, chat: String, out: Outgoing) -> Result<(), PushError> {
    let client = store.read().matrix.clone().expect("flush checks the Matrix client");
    if let Outgoing::CreateLabel(label) = out {
        return Ok(create_label(store, &client, label).await?);
    }
    let chat = store.read().sent.get(&chat).cloned().unwrap_or(chat);
    let room = chat_room(&client, &chat).await?;
    let event = |id: String| {
        let id = store.read().sent.get(&id).cloned().unwrap_or(id);
        OwnedEventId::try_from(id).map_err(|e| e.to_string())
    };
    let e = PushError::from;
    let sent = |txn: String, id: OwnedEventId| store.write().sent.insert(txn, id.to_string());
    match out {
        Outgoing::Send { txn, text, reply_to, preview, mentions, thread, links } => {
            // Desktop mentions the sender of the quoted message, but not of the fallback reply of a thread reply.
            let sender = reply_to.as_ref().filter(|_| thread.as_ref().is_none_or(|t| t.chosen)).and_then(|r| {
                let st = store.read();
                let m = st.stored_message(r)?;
                (!st.is_me(&m.sender_id)).then_some(m.sender_id)
            });
            let mut content = text_content(text, &mentions, sender.as_deref())?;
            if thread.is_none() {
                content.relates_to =
                    reply_to.map(|r| event(r).map(|id| Relation::Reply(Reply::with_event_id(id)))).transpose()?;
            }
            let mut content = serde_json::to_value(content).expect("message JSON");
            if let Some(t) = thread {
                // Desktop's `buildThreadRelation`.
                content["m.relates_to"] = serde_json::json!({
                    "rel_type": "m.thread",
                    "event_id": event(t.root)?.to_string(),
                    "is_falling_back": !t.chosen,
                    "m.in_reply_to": { "event_id": event(t.quoted)?.to_string() },
                });
                if t.broadcast {
                    content["com.beeper.broadcast_outside_thread"] = true.into();
                }
            }
            if let Some((mut p, image)) = preview {
                // Desktop's `buildLinkPreview` sends the preview without its image when the upload fails.
                if let Some(bytes) = image {
                    match preview_image(&client, &room, &p, bytes).await {
                        Ok((key, value)) => p[key] = value,
                        Err(err) => eprintln!("link preview: dropping its image: {err}"),
                    }
                }
                content["com.beeper.linkpreviews"] = Value::Array(vec![p]);
            } else if links {
                content["com.beeper.linkpreviews"] = Value::Array(Vec::new());
            }
            if let Some(timer) = timer_for_messages(&room).await {
                content[DISAPPEARING_TIMER] = timer;
            }
            let id = room.send_raw("m.room.message", content).with_transaction_id(txn.as_str().into());
            sent(txn, id.await.map_err(e)?.response.event_id);
        }
        Outgoing::Edit { target, text, mentions, caption } => {
            let target = event(target)?;
            let attachment = match caption {
                true => Some(caption_attachment(&room, &target).await?),
                false => None,
            };
            room.send_raw("m.room.message", edit_content(&target, text, &mentions, attachment)?).await.map_err(e)?;
        }
        Outgoing::Redact { target } => {
            room.redact(&event(target)?, None, None).await.map_err(|x| x.to_string())?;
        }
        // Desktop's `sendReactionEvent`: a custom emoji reacts with its image and names its shortcode.
        Outgoing::React { txn, target, key, img_url: Some(img) } => {
            let content = serde_json::json!({
                "m.relates_to": { "rel_type": "m.annotation", "event_id": event(target)?, "key": img },
                "com.beeper.reaction.shortcode": format!(":{key}:"),
            });
            let id = room.send_raw("m.reaction", content).with_transaction_id(txn.as_str().into());
            sent(txn, id.await.map_err(e)?.response.event_id);
        }
        Outgoing::React { txn, target, key, img_url: None } => {
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
        Outgoing::ThreadRead { root, event: target } => {
            let thread = ReceiptThread::Thread(event(root)?);
            room.send_single_receipt(SendReceiptType::Read, thread, event(target)?).await.map_err(e)?;
        }
        Outgoing::Flag(flag, on) => match flag {
            ChatFlag::Archived => {
                let order = on.then(|| {
                    let st = store.read();
                    let last = st.stored("chat_id = ?1 ORDER BY sort_key DESC LIMIT 1", [&chat]).pop();
                    let last = last.map(|m| m.sort_key).max(st.chat(&chat).notifiable.clone());
                    last.map_or(MIN_SAFE_INTEGER, |k| order_of(&k))
                });
                let done = inbox_done(order, now_unix_millis());
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
                // Desktop reads the mute from `com.beeper.muted`, so the room goes in or out of its list too.
                let mut list = global(&client, MUTED).await;
                let id = room.room_id().as_str();
                let mut ids: Vec<Value> = list["room_ids"].as_array().cloned().unwrap_or_default();
                ids.retain(|r| r != id);
                if on {
                    ids.push(id.into());
                }
                if !list.is_object() {
                    list = serde_json::json!({});
                }
                list["room_ids"] = ids.into();
                set_global(client.clone(), MUTED, list).await?;
            }
            ChatFlag::MarkedUnread => room.set_unread_flag(on).await.map_err(e)?,
        },
        Outgoing::ThreadFollows(follows) => {
            let mut v = serde_json::to_value(follows).expect("thread follows JSON");
            v["com.beeper.revision_id"] = matrix_sdk::ruma::TransactionId::new().to_string().into();
            let raw = Raw::from_json(serde_json::value::to_raw_value(&v).expect("thread follows JSON"));
            room.set_account_data_raw(THREAD_FOLLOWS.into(), raw).await.map_err(e)?;
        }
        Outgoing::Reminder(at) => {
            let v = at.map_or(serde_json::json!({}), |r| {
                serde_json::json!({ "remind_at_ms": r.remind_at_ms, "dismiss_on_incoming_message": r.dismiss_on_incoming_message })
            });
            let raw = Raw::from_json(serde_json::value::to_raw_value(&v).expect("reminder JSON"));
            room.set_account_data_raw(REMINDER.into(), raw).await.map_err(e)?;
        }
        // Desktop accepts an invite of a Matrix chat by a join, and other requests with the bridge's event.
        Outgoing::AcceptRequest if room.state() == RoomState::Invited => room.join().await.map_err(e)?,
        Outgoing::AcceptRequest => {
            room.send_raw("com.beeper.accept_message_request", serde_json::json!({})).await.map_err(e)?;
        }
        Outgoing::BlockUser { block, report_spam } => {
            let content = serde_json::json!({ "block": block, "report_spam": report_spam });
            room.send_raw("com.beeper.block_user", content).await.map_err(e)?;
        }
        // Desktop's `declineMessageRequest` of a Matrix chat leaves the room.
        Outgoing::DeleteChat { .. } if room.state() == RoomState::Invited => room.leave().await.map_err(e)?,
        // Desktop's `forgetRoom`: a chat that the user left or was removed from goes with its room.
        Outgoing::DeleteChat { .. } if room.state() == RoomState::Left => room.forget().await.map_err(e)?,
        Outgoing::DeleteChat { for_everyone } => {
            let content = serde_json::json!({ "delete_for_everyone": for_everyone });
            room.send_raw("com.beeper.delete_chat", content).await.map_err(e)?;
        }
        Outgoing::Forward { txn, from, event: id, text, html } => {
            let source = joined_room(&client, &from).await?.event(&event(id)?, None).await.map_err(e)?;
            let mut ev: Value = source.raw().deserialize_as().map_err(|x| x.to_string())?;
            let kind = ev["type"].as_str().expect("an event type").to_string();
            // Desktop forwards any message, but a poll or a message that is not decrypted has no content to send again.
            if !matches!(kind.as_str(), "m.room.message" | "m.sticker") {
                return Err(format!("Cannot forward a {kind} event").into());
            }
            let c = ev["content"].as_object_mut().expect("event content");
            c.remove("m.relates_to");
            c.remove("m.mentions");
            let typed = matches!(c.get("msgtype").and_then(Value::as_str), Some("m.text" | "m.notice" | "m.emote"));
            if let Some(text) = text.filter(|_| typed) {
                c.insert("body".into(), text.into());
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
        Outgoing::Media { txn, bytes: data, name, mime, info, caption, reply_to, thread, gif, sticker } => {
            // Desktop's `Kf`: a sticker goes as `m.sticker` when the room features take its type, else as an image.
            let sticker = sticker.filter(|_| {
                let st = store.read();
                let c = &st.chats[st.resolve_chat_idx(&chat).expect("the chat of a send exists")];
                c.room_features && crate::media::takes_sticker(&c.file, &mime)
            });
            let (data, name, mime, info) =
                prepare_media(store, &chat, &txn, data, (name, mime), info, sticker.is_some()).await?;
            converted(store, &txn, &name, &mime);
            let dims = match info {
                MediaInfo::Image(w, h) | MediaInfo::Video(w, h, _) => Some((w, h)),
                _ => None,
            };
            use matrix_sdk::attachment::{AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo};
            use matrix_sdk::room::reply::{EnforceThread, Reply};
            use matrix_sdk::ruma::events::room::message::{AddMentions, ReplyWithinThread, RoomMessageEventContent};
            // The thread relation of the SDK is Desktop's `buildThreadRelation`, and "Also send to chat" adds its flag.
            let broadcast = thread.as_ref().is_some_and(|t| t.broadcast);
            let (enforce_thread, add_mentions) = match thread.map(|t| t.chosen) {
                Some(true) => (EnforceThread::Threaded(ReplyWithinThread::Yes), AddMentions::Yes),
                Some(false) => (EnforceThread::Threaded(ReplyWithinThread::No), AddMentions::No),
                None => (EnforceThread::MaybeThreaded, AddMentions::Yes),
            };
            let reply = reply_to
                .map(|r| event(r).map(|event_id| Reply { event_id, enforce_thread, add_mentions }))
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
                MediaInfo::Video(w, h, ms) => AttachmentInfo::Video(BaseVideoInfo {
                    duration: Some(std::time::Duration::from_millis(ms)),
                    width: Some(w.into()),
                    height: Some(h.into()),
                    size,
                    blurhash: None,
                }),
                MediaInfo::Audio(ms) => AttachmentInfo::Audio(BaseAudioInfo {
                    duration: Some(std::time::Duration::from_millis(ms)),
                    size,
                    waveform: None,
                }),
                MediaInfo::File => AttachmentInfo::File(BaseFileInfo { size }),
            };
            let mime: mime::Mime = mime.parse().expect("valid media mime");
            let source = match room.latest_encryption_state().await.map_err(e)?.is_encrypted() {
                true => MediaSource::Encrypted(Box::new(
                    client.upload_encrypted_file(&mut data.as_slice()).await.map_err(e)?,
                )),
                false => MediaSource::Plain(client.media().upload(&mime, data, None).await.map_err(e)?.content_uri),
            };
            let caption_given = caption.is_some();
            let content = RoomMessageEventContent::new(media_type(&mime, name, source, caption, info));
            let content = match reply {
                Some(r) => room.make_reply_event(content.into(), r).await.map_err(|x| x.to_string())?,
                None => content,
            };
            // The SDK sends this content in `send_attachment`, but its info cannot carry Desktop's GIF flag.
            let mut content = serde_json::to_value(content).expect("message JSON");
            if gif {
                content["info"]["fi.mau.gif"] = true.into();
            }
            if broadcast {
                content["com.beeper.broadcast_outside_thread"] = true.into();
            }
            if let Some(timer) = timer_for_messages(&room).await {
                content[DISAPPEARING_TIMER] = timer;
            }
            let kind = match sticker {
                Some(s) => {
                    sticker_content(&mut content, s, caption_given, dims);
                    "m.sticker"
                }
                None => "m.room.message",
            };
            let id = room.send_raw(kind, content).with_transaction_id(txn.as_str().into());
            sent(txn, id.await.map_err(e)?.response.event_id);
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
        Outgoing::State(kind, content) => drop(room.send_state_event_raw(&kind, "", content).await.map_err(e)?),
        Outgoing::Invite(user) => {
            let user = matrix_sdk::ruma::UserId::parse(&user).expect("a Matrix user ID");
            room.invite_user_by_id(&user).await.map_err(e)?;
        }
        Outgoing::Kick(user) => {
            let content = serde_json::json!({ "membership": "leave" });
            drop(room.send_state_event_raw("m.room.member", &user, content).await.map_err(e)?);
        }
        Outgoing::Role(user, admin) => {
            let mut levels = state(&room, "m.room.power_levels").await.unwrap_or_else(|| serde_json::json!({}));
            let native = state(&room, "m.bridge").await.is_none();
            match admin {
                true => levels["users"][&user] = if native { 100 } else { 50 }.into(),
                false => drop(levels["users"].as_object_mut().and_then(|u| u.remove(&user))),
            }
            drop(room.send_state_event_raw("m.room.power_levels", "", levels).await.map_err(e)?);
        }
        Outgoing::Bookmark { room: chat, target, on } => {
            let target = event(target)?.to_string();
            let known = store.read().bookmarks.get(&target).map(|b| b.event_id.clone());
            let rel = serde_json::json!({ "com.beeper.relates_to": { "event_id": target, "room_id": chat, "rel_type": BOOKMARK } });
            match (on, known) {
                (true, None) => {
                    let id = room.send_raw(BOOKMARK, rel).await.map_err(e)?.response.event_id.to_string();
                    store.write().bookmarks.insert(target, Bookmark { room_id: chat, event_id: id, order: 0 });
                }
                (false, Some(id)) => {
                    redact_with(&client, &room, &id, rel).await?;
                    store.write().bookmarks.remove(&target);
                }
                _ => {}
            }
        }
        // Desktop sends a new reminder event for each time, and its dismissal redacts the latest one.
        Outgoing::MessageReminder { room: chat, target, at } => {
            let target = event(target)?.to_string();
            let rel = serde_json::json!({ "event_id": target, "room_id": chat, "rel_type": MESSAGE_REMINDER });
            match at {
                Some(at) => {
                    let content = serde_json::json!({ "com.beeper.relates_to": rel, "remind_at_ms": at });
                    let id = room.send_raw(MESSAGE_REMINDER, content).await.map_err(e)?.response.event_id;
                    store.write().reminder_events.insert(target, id.to_string());
                }
                None => {
                    let known = store.read().reminder_events.get(&target).cloned();
                    if let Some(id) = known {
                        redact_with(&client, &room, &id, serde_json::json!({ "com.beeper.relates_to": rel })).await?;
                        store.write().reminder_events.remove(&target);
                    }
                }
            }
        }
        // Desktop sends a pending removal again after a restart, but not when the room already has a removal.
        Outgoing::RemoveFromBeeper { txn, room: chat, target } => {
            let target = event(target)?.to_string();
            if store.read().removals.as_ref().is_none_or(|r| !r.contains_key(&target)) {
                let rel = serde_json::json!({ "event_id": target, "room_id": chat, "rel_type": REMOVED_MESSAGE });
                let content = serde_json::json!({ "com.beeper.relates_to": rel });
                room.send_raw(REMOVED_MESSAGE, content).with_transaction_id(txn.as_str().into()).await.map_err(e)?;
            }
        }
        Outgoing::Transcription { room: chat, target, transcription: t } => {
            let content = serde_json::json!({
                "msgtype": TRANSCRIPTION,
                "com.beeper.relates_to": { "room_id": chat, "event_id": event(target)?, "rel_type": TRANSCRIPTION },
                "com.beeper.transcription.model": t.engine,
                "com.beeper.transcription.language": t.language,
                "m.text": [{ "body": t.transcription }],
            });
            room.send_raw(TRANSCRIPTION, content).await.map_err(e)?;
        }
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
        Outgoing::RoomData(kind, content) => {
            let raw = Raw::from_json(serde_json::value::to_raw_value(&content).expect("room account data JSON"));
            room.set_account_data_raw(kind.into(), raw).await.map_err(e)?;
        }
        Outgoing::PersonalAvatar(image) => {
            let content = match image {
                Some((bytes, mime)) => {
                    let info = serde_json::json!({ "mimetype": mime, "size": bytes.len() });
                    if room.latest_encryption_state().await.map_err(e)?.is_encrypted() {
                        let file = client.upload_encrypted_file(&mut bytes.as_slice()).await.map_err(e)?;
                        serde_json::json!({ "file": file, "info": info })
                    } else {
                        let mime: mime::Mime = mime.parse().expect("an image MIME type");
                        let url = client.media().upload(&mime, bytes, None).await.map_err(e)?.content_uri;
                        serde_json::json!({ "url": url, "info": info })
                    }
                }
                None => serde_json::json!({}),
            };
            let raw = Raw::from_json(serde_json::value::to_raw_value(&content).expect("personal avatar JSON"));
            room.set_account_data_raw(PERSONAL_AVATAR.into(), raw).await.map_err(e)?;
        }
        Outgoing::LabelChild(chat, add) => {
            let content = if add { serde_json::json!({ "via": ["beeper.com"] }) } else { serde_json::json!({}) };
            room.send_state_event_raw("m.space.child", &chat, content).await.map_err(e)?;
        }
        Outgoing::Leave(forget) => {
            room.leave().await.map_err(e)?;
            if let Some(left) = client.get_room(room.room_id()).filter(|_| forget) {
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
    // The demo seed has its photos inline.
    if let Some((_, b64)) = src.strip_prefix("data:").and_then(|s| s.split_once(";base64,")) {
        return vodozemac::base64_decode(b64).map_err(|e| e.to_string());
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

/// The session and bridge login that a bridge provisioning request acts as. The homeserver passes the request to
/// the bridge, like the SDK's `CallCloudBridgeAPI`. An empty `login_id` acts as the default login, or a new one.
#[derive(Clone)]
pub struct Provision {
    pub homeserver: String,
    pub access_token: String,
    pub user_id: String,
    pub bridge_id: String,
    pub login_id: String,
}

/// The body of a bridge reply, or the Matrix `error` text of a failed one.
async fn bridge_reply(resp: Result<reqwest::Response, reqwest::Error>) -> Result<String, String> {
    let resp = resp.map_err(|e| e.without_url().to_string())?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.without_url().to_string())?;
    match status.is_success() {
        true => Ok(body),
        false => Err(serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(format!("HTTP {status}"))),
    }
}

impl Provision {
    fn url(&self, path: &str) -> String {
        let enc = crate::codec::url_encode;
        let login = match self.login_id.is_empty() {
            true => String::new(),
            false => format!("&login_id={}", enc(&self.login_id)),
        };
        format!(
            "{}/_matrix/client/unstable/com.beeper.bridge/{}/_matrix/provision/v3/{path}?user_id={}{login}",
            self.homeserver.trim_end_matches('/'),
            enc(&self.bridge_id),
            enc(&self.user_id),
        )
    }

    /// A POST without a timeout, because `display_and_wait` steps answer only when the user acts.
    pub(crate) async fn post(&self, path: &str, body: String) -> Result<String, String> {
        let req = crate::cloud::wait_client().post(self.url(path)).bearer_auth(&self.access_token);
        bridge_reply(req.header("content-type", "application/json").body(body).send().await).await
    }

    /// The JSON reply, or `None` when the homeserver or the bridge does not know the path (`M_NOT_FOUND`).
    pub(crate) async fn get(&self, path: &str) -> Result<Option<Value>, String> {
        let resp = crate::cloud::http_client().get(self.url(path)).bearer_auth(&self.access_token).send().await;
        if resp.as_ref().is_ok_and(|r| r.status() == reqwest::StatusCode::NOT_FOUND) {
            return Ok(None);
        }
        serde_json::from_str(&bridge_reply(resp).await?).map(Some).map_err(|e| e.to_string())
    }
}

/// Desktop's `logoutCloudBridgeAccount`: delete the bridge when the login is its only one, or else log the login
/// out of the bridge.
pub async fn remove_account(p: Provision, api_url: String, sole: bool) -> Result<(), String> {
    if !sole {
        let path = format!("logout/{}", crate::codec::url_encode(&p.login_id));
        return p.post(&path, "{}".into()).await.map(drop);
    }
    let url = format!("{}/bridge/{}", api_url.trim_end_matches('/'), crate::codec::url_encode(&p.bridge_id));
    let resp = crate::cloud::http_client().delete(url).bearer_auth(&p.access_token).send().await;
    resp.map_err(|e| e.to_string())?.error_for_status().map(drop).map_err(|e| e.to_string())
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

/// The SDK's `ViewLimitedMedia`: tells the bridge that the view-once media of `event_id` was viewed once under
/// `limit`, so the bridge counts the view and deletes the media after its last view.
pub async fn view_limited_media(p: Provision, event_id: String, limit: Value) -> Result<(), String> {
    let body = serde_json::json!({ "com.beeper.view_limited": limit }).to_string();
    p.post(&format!("view_limited_media/{}", crate::codec::url_encode(&event_id)), body).await.map(drop)
}

/// The bridge's `create_dm`: the room of the direct chat with `identifier`, which the bridge makes when it is missing.
pub async fn create_dm(p: Provision, identifier: String) -> Result<String, String> {
    let body = p.post(&format!("create_dm/{}", crate::codec::url_encode(&identifier)), "{}".into()).await?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v["dm_room_mxid"].as_str().map(str::to_string).ok_or_else(|| "the bridge sent no dm_room_mxid".into())
}

/// The bridge's `create_group`: a new group of the given type with the users, and its name when the bridge allows
/// one. It returns the room ID.
pub async fn create_group(p: Provision, kind: String, name: String, users: Vec<String>) -> Result<String, String> {
    let mut body = serde_json::json!({ "type": kind, "participants": users });
    if !name.is_empty() {
        body["name"] = serde_json::json!({ "name": name });
    }
    let body = p.post(&format!("create_group/{}", crate::codec::url_encode(&kind)), body.to_string()).await?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v["mxid"].as_str().map(str::to_string).ok_or_else(|| "the bridge sent no mxid".into())
}

/// Desktop SDK's `GetProvisioningCapabilities` of its `matrix` pseudo-bridge, which Desktop's
/// `loadProvisioningCapabilities` gives the Matrix account: direct chats with Matrix IDs, the user directory, and
/// private and public rooms of up to 100 people.
pub fn matrix_capabilities() -> Value {
    let field = |allowed: bool| serde_json::json!({ "allowed": allowed });
    let room = |description: &str, username: bool, disappear: bool| {
        serde_json::json!({
            "type_description": description,
            "name": field(true),
            "username": field(username),
            "avatar": field(true),
            "topic": field(true),
            "disappear": field(disappear),
            "participants": { "allowed": true, "max_length": 100 },
            "parent": field(false),
        })
    };
    serde_json::json!({
        "resolve_identifier": {
            "create_dm": true,
            "lookup_phone": false,
            "lookup_email": false,
            "lookup_username": true,
            "any_phone": false,
            "contact_list": false,
            "search": true,
        },
        "group_creation": {
            "group": room("a private room", false, true),
            "public_room": room("a public room", true, false),
        },
    })
}

/// Desktop's `searchMatrixUsers`: up to 100 users of the homeserver's user directory (the SDK's
/// `searchUserDirectory`), with their Matrix IDs as usernames, then a typed Matrix ID that the directory missed.
pub async fn search_matrix_users(store: SharedStore, query: String) -> Result<Vec<User>, String> {
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    let found = client.search_users(&query, 100).await.map_err(|e| e.to_string())?;
    let user = |id: String, name: Option<String>, avatar: Option<String>| User {
        username: Some(id.clone()),
        id,
        full_name: name.filter(|n| !n.is_empty()),
        img_url: avatar,
        ..Default::default()
    };
    let mut users: Vec<User> = found
        .results
        .into_iter()
        .map(|u| user(u.user_id.to_string(), u.display_name, u.avatar_url.map(|a| a.to_string())))
        .collect();
    if let Some(id) = typed_matrix_id(&query).filter(|id| users.iter().all(|u| &u.id != id)) {
        users.push(user(id.clone(), Some(id), None));
    }
    Ok(users)
}

/// The Matrix ID of a query that starts with `@`, for Desktop's `searchMatrixUsers`: `@x` is `@x:beeper.com`, and
/// the server name must look like a domain (Desktop's `kI`, `^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$`).
fn typed_matrix_id(query: &str) -> Option<String> {
    if !query.starts_with('@') || query.len() < 2 {
        return None;
    }
    let id = if query.contains(':') { query.to_string() } else { format!("{query}:beeper.com") };
    let (host, tld) = id.split(':').nth(1)?.rsplit_once('.')?;
    let host_ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    (host_ok && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())).then_some(id)
}

/// Desktop SDK's `matrixResolveIdentifier`, the `resolve_identifier` of its `matrix` pseudo-bridge: the profile of a
/// Matrix ID as a bridge reply, or `None` when the homeserver has no such user.
pub async fn resolve_matrix_user(store: SharedStore, identifier: String) -> Result<Option<Value>, String> {
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    let id = matrix_sdk::ruma::UserId::parse(&identifier)
        .map_err(|e| format!("user ID {identifier:?} is not valid: {e}"))?;
    let profile = match client.account().fetch_user_profile_of(&id).await {
        Ok(p) => p,
        Err(e) if e.client_api_error_kind() == Some(&ErrorKind::NotFound) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let field = |key: &str| profile.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(Some(serde_json::json!({
        "id": id,
        "name": field("displayname"),
        "avatar_url": field("avatar_url"),
        "identifiers": [id.matrix_uri(false).to_string()],
        "mxid": id,
    })))
}

/// The `m.room.encryption` initial state of the rooms that Desktop's SDK makes.
fn megolm_state() -> Value {
    serde_json::json!({ "type": "m.room.encryption", "state_key": "", "content": { "algorithm": "m.megolm.v1.aes-sha2" } })
}

/// Desktop SDK's `createMatrixDM`, the `create_dm` of its `matrix` pseudo-bridge: a new direct room with a Matrix
/// user, encrypted when the user has devices. Like the SDK, a user that the homeserver does not know still gets the
/// room, it makes a new room each time, and it leaves `m.direct` alone. It returns the room ID.
pub async fn create_matrix_dm(store: SharedStore, identifier: String) -> Result<String, String> {
    resolve_matrix_user(store.clone(), identifier.clone())
        .await
        .map_err(|e| format!("failed to resolve identifier: {e}"))?;
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    let user = matrix_sdk::ruma::UserId::parse(&identifier).map_err(|e| e.to_string())?;
    let cached = client.encryption().get_user_devices(&user).await.map_err(|e| e.to_string())?;
    // The SDK's `FetchKeys` asks the homeserver about a user whose devices it does not track.
    let encrypted = match cached.devices().next() {
        Some(_) => true,
        None => {
            let query = serde_json::json!({ "device_keys": { user.as_str(): [] }, "timeout": 10000 });
            let reply = client_api(&client, reqwest::Method::POST, &["keys", "query"], &[], Some(query.to_string()))
                .await
                .map_err(|e| format!("failed to query devices for {identifier:?}: {e}"))?;
            let v: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
            v["device_keys"][user.as_str()].as_object().is_some_and(|d| !d.is_empty())
        }
    };
    let mut body = serde_json::json!({ "invite": [user], "preset": "trusted_private_chat", "is_direct": true });
    if encrypted {
        body["initial_state"] = serde_json::json!([megolm_state()]);
    }
    create_room(client, body.to_string()).await
}

/// Desktop SDK's `convertGroupCreateToMatrix` for a `group` or an `unencrypted_group` of its `matrix` pseudo-bridge:
/// a new private room with the users, encrypted unless it is an `unencrypted_group`, and named when `name` is not
/// empty. It returns the room ID.
pub async fn create_matrix_group(
    store: SharedStore,
    kind: String,
    name: String,
    users: Vec<String>,
) -> Result<String, String> {
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    for (i, u) in users.iter().enumerate() {
        matrix_sdk::ruma::UserId::parse(u).map_err(|e| format!("user ID {} ({u:?}) is not valid: {e}", i + 1))?;
    }
    let mut body = serde_json::json!({ "invite": users, "preset": "private_chat" });
    if kind != "unencrypted_group" {
        body["initial_state"] = serde_json::json!([megolm_state()]);
    }
    if !name.is_empty() {
        body["name"] = name.into();
    }
    create_room(client, body.to_string()).await
}

/// Desktop's `setRoomAvatar`: uploads the image without encryption and sends it as the `m.room.avatar` of the room,
/// which does not have to be synced yet.
pub async fn set_room_avatar(store: SharedStore, room: String, bytes: Vec<u8>, mime: String) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
    let mime: mime::Mime = mime.parse().map_err(|e| format!("bad image type {mime}: {e}"))?;
    let url = client.media().upload(&mime, bytes, None).await.map_err(|e| e.to_string())?.content_uri;
    let content = serde_json::json!({ "url": url }).to_string();
    let path = ["rooms", &room, "state", "m.room.avatar", ""];
    client_api(&client, reqwest::Method::PUT, &path, &[], Some(content)).await.map(drop)
}

pub async fn thumbnail(client: Client, url: String) -> Result<Vec<u8>, String> {
    let size = matrix_sdk::ruma::uint!(96);
    let format = MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(Method::Crop, size, size));
    let request = MediaRequestParameters { source: MediaSource::Plain(url.into()), format };
    client.media().get_media_content(&request, true).await.map_err(|e| e.to_string())
}

pub(crate) async fn connect(store: &SharedStore) -> Result<Client, String> {
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
    !store.read().stored("chat_id = ?1 AND json ->> '$.text' = ?2 LIMIT 1", [&chat, UTD_TEXT]).is_empty()
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

/// Fetches again the messages of the rooms that wait for their keys, by event, so that a message on an older page
/// decrypts too: the newest 200 of each room.
async fn refetch_undecrypted(store: &SharedStore, rooms: Vec<Room>) {
    futures_util::stream::iter(rooms)
        .for_each_concurrent(8, |room| async move {
            let chat = chat_id(&room).await;
            let waiting = "chat_id = ?1 AND json ->> '$.text' = ?2 ORDER BY sort_key DESC LIMIT 200";
            let ids: Vec<String> = store.read().stored(waiting, [&chat, UTD_TEXT]).into_iter().map(|m| m.id).collect();
            let mut events = Vec::new();
            for id in ids.iter().rev().filter_map(|id| OwnedEventId::try_from(id.as_str()).ok()) {
                // An event that the homeserver does not give stays waiting.
                if let Ok(e) = room.event(&id, None).await {
                    events.push(e);
                }
            }
            if !events.is_empty() {
                sync_room(store, &room, events).await;
            }
        })
        .await;
}

/// Desktop's global account data of the muted rooms.
const MUTED: &str = "com.beeper.muted";

/// The `room_ids` of `com.beeper.muted`.
async fn muted_rooms(client: &Client) -> Vec<String> {
    let list = global(client, MUTED).await;
    list["room_ids"].as_array().into_iter().flatten().filter_map(|r| r.as_str().map(String::from)).collect()
}

async fn global(client: &Client, kind: &str) -> Value {
    let raw =
        client.account().account_data_raw(GlobalAccountDataEventType::from(kind)).await.expect("read account data");
    raw.map_or(Value::Null, |r| r.deserialize_as().expect("account data JSON"))
}

/// Desktop SDK's `AccountDataBridgeAccountOverrides`: the account nicknames, encrypted with MSC4483.
pub const ACCOUNT_OVERRIDES: &str = "com.beeper.bridge_account_overrides";
/// Desktop SDK's tries of a write of the overrides while other devices change them.
const OVERRIDE_TRIES: usize = 5;
/// Desktop SDK's `ErrRevisionIDMismatch`: the account data has another revision than the write expects.
const REVISION_MISMATCH: &str = "COM.BEEPER.REVISION_ID_MISMATCH";

/// Desktop SDK's `bridgeAccountOverrides`: the nickname of each login of each bridge, and the revision.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct AccountOverrides {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    accounts: BTreeMap<String, BTreeMap<String, AccountOverride>>,
    #[serde(rename = "com.beeper.revision_id", default, skip_serializing_if = "String::is_empty")]
    revision_id: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AccountOverride {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
}

/// Desktop SDK's `setName` on the overrides `content`: the nickname of the login `account` of `bridge`, or its removal
/// for an empty `name`, with a new revision.
pub fn with_account_name(content: &Value, bridge: &str, account: &str, name: &str) -> Result<Value, String> {
    let mut o: AccountOverrides = match content {
        Value::Null => AccountOverrides::default(),
        c => serde_json::from_value(c.clone()).map_err(|e| format!("failed to parse bridge account overrides: {e}"))?,
    };
    match name {
        "" => {
            if let Some(logins) = o.accounts.get_mut(bridge) {
                logins.remove(account);
                if logins.is_empty() {
                    o.accounts.remove(bridge);
                }
            }
        }
        _ => {
            _ = o
                .accounts
                .entry(bridge.into())
                .or_default()
                .insert(account.into(), AccountOverride { name: name.into() })
        }
    }
    o.revision_id = matrix_sdk::ruma::TransactionId::new().to_string();
    Ok(serde_json::to_value(o).expect("overrides JSON"))
}

/// Desktop SDK's `getEncryptedAccountDataKey`: the MSC4483 key from secret storage, which the stored recovery key
/// opens. With `create`, a new key goes to secret storage when it has none (`generateAndStoreEncryptedAccountDataKey`).
async fn account_data_key(client: &Client, store: &SharedStore, create: bool) -> Result<[u8; 32], String> {
    let (key, code) = {
        let st = store.read();
        (st.account_data_key, st.beeper_state.secrets.recovery_code.clone())
    };
    if let Some(key) = key {
        return Ok(key);
    }
    if code.is_empty() {
        return Err("encrypted account data key unavailable: no stored recovery code to access SSSS".into());
    }
    let storage = client.encryption().secret_storage();
    let secrets =
        storage.open_secret_store(&code).await.map_err(|e| format!("failed to load default SSSS key: {e}"))?;
    let found = secrets.get_secret(crate::msc4483::KEY_SECRET).await;
    let key = match found.map_err(|e| format!("cannot look up encrypted account data key in SSSS: {e}"))? {
        Some(secret) => crate::msc4483::key_from_secret(&secret)?,
        None if create => {
            let key = crate::msc4483::new_key();
            let put = secrets.put_secret(crate::msc4483::KEY_SECRET, &vodozemac::base64_encode(key)).await;
            put.map_err(|e| format!("failed to upload encrypted account data key to SSSS: {e}"))?;
            key
        }
        None => return Err("encrypted account data key unavailable".into()),
    };
    store.write().account_data_key = Some(key);
    Ok(key)
}

/// Desktop SDK's `MaybeDecryptAccountData` of the synced overrides. Content that cannot be decrypted yet stays as it
/// is, and the next recovery key tries again.
async fn sync_account_overrides(client: &Client, store: &SharedStore) {
    let synced = global(client, ACCOUNT_OVERRIDES).await;
    if store.read().account_overrides_synced == synced {
        return;
    }
    let decrypted = match crate::msc4483::is_encrypted(&synced) {
        true => account_data_key(client, store, false)
            .await
            .and_then(|key| crate::msc4483::decrypt(&key, ACCOUNT_OVERRIDES, &synced)),
        false => Ok(synced.clone()),
    };
    let content = decrypted.unwrap_or_else(|e| {
        eprintln!("Could not decrypt {ACCOUNT_OVERRIDES}: {e}");
        synced.clone()
    });
    let mut st = store.write();
    (st.account_overrides_synced, st.account_overrides) = (synced, content);
    st.emit_ws("setup.updated", "", None, None, None);
}

/// Desktop SDK's `setBridgeAccountName`: the nickname of the login `account` of `bridge`, or none for an empty `name`,
/// in the encrypted overrides. The write expects the revision that it read. When another device wrote first, the
/// homeserver answers with the current content, and the name goes on that, up to 5 tries.
pub async fn set_bridge_account_name(
    store: SharedStore,
    bridge: String,
    account: String,
    name: String,
) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let mut current = store.read().account_overrides.clone();
    if crate::msc4483::is_encrypted(&current) {
        return Err("bridge account overrides are not decrypted yet".into());
    }
    let path = ["user", &own_id(&client), "account_data", ACCOUNT_OVERRIDES];
    for _ in 0..OVERRIDE_TRIES {
        let next = with_account_name(&current, &bridge, &account, name.trim())?;
        let key = account_data_key(&client, &store, true).await?;
        let body = crate::msc4483::encrypt(&key, ACCOUNT_OVERRIDES, &next).to_string();
        let expected = current["com.beeper.revision_id"].as_str().unwrap_or_default().to_string();
        let query = [("com.beeper.expect_revision_id", expected.as_str())];
        let sent = client_request(&client, reqwest::Method::PUT, &path, &query, Some(body)).await;
        let (status, text) = sent.map_err(|e| format!("failed to set bridge account overrides: {e}"))?;
        if status.is_success() {
            store.write().account_overrides = next;
            return Ok(());
        }
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        current = match (v["errcode"].as_str(), v.get("com.beeper.current_content")) {
            (Some(REVISION_MISMATCH), Some(c)) if crate::msc4483::is_encrypted(c) => {
                crate::msc4483::decrypt(&key, ACCOUNT_OVERRIDES, c)?
            }
            (Some(REVISION_MISMATCH), Some(c)) => c.clone(),
            _ => return Err(format!("failed to set bridge account overrides: {}", matrix_error(status, &v))),
        };
    }
    Err(format!("bridge account overrides kept changing after {OVERRIDE_TRIES} attempts"))
}

/// Desktop's room image pack event types (`_u`): the stable type, and the older type that it replaces (`MR`).
const PACK_TYPES: [&str; 2] = ["m.room.image_pack", "im.ponies.room_emotes"];
/// Desktop's `space_children` depth limit (`RT`) of the packs of the spaces of a room.
const PACK_DEPTH: usize = 5;

/// Desktop's `imagePackStore.fetchStickerPacks` with `getImagePacksForRoom`: the packs with stickers of `room`, of
/// its spaces up to 5 levels up, of the rooms that `m.image_pack.rooms` lists, and the own `m.image_pack`.
pub async fn sticker_packs(store: SharedStore, room: String) -> Result<Vec<StickerPack>, String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    // Desktop's `space_children`: the `m.space.child` state of the joined spaces.
    let mut parents: HashMap<String, Vec<String>> = HashMap::new();
    for space in client.joined_rooms().into_iter().filter(|r| r.is_space()) {
        for child in states(&space, "m.space.child".into()).await {
            if child["content"].as_object().is_some_and(|c| !c.is_empty()) {
                let child = child["state_key"].as_str().expect("a state event has a state key").to_string();
                parents.entry(child).or_default().push(space.room_id().to_string());
            }
        }
    }
    let mut rooms = vec![room];
    let mut level = rooms.clone();
    for _ in 0..PACK_DEPTH {
        let up: Vec<String> = level.iter().flat_map(|r| parents.get(r).into_iter().flatten()).cloned().collect();
        level.clear();
        for r in up {
            if !rooms.contains(&r) {
                rooms.push(r.clone());
                level.push(r);
            }
        }
    }
    let mut packs = Vec::new();
    for r in &rooms {
        packs.extend(room_packs(&client, r).await);
    }
    // Desktop's `withGlobalPacks`: the packs of other rooms that you added to all rooms.
    let global_rooms = global(&client, "m.image_pack.rooms").await;
    for (r, keys) in global_rooms["rooms"].as_object().into_iter().flatten() {
        let keys = keys.as_object();
        for p in room_packs(&client, r).await {
            if keys.is_some_and(|k| k.contains_key(&p.2))
                && !packs.iter().any(|q| (&q.0, &q.1, &q.2) == (&p.0, &p.1, &p.2))
            {
                packs.push(p);
            }
        }
    }
    let stable = |p: &(String, &str, String, Value)| p.1 == PACK_TYPES[0];
    let replaced: HashSet<(String, String)> =
        packs.iter().filter(|p| stable(p)).map(|p| (p.0.clone(), p.2.clone())).collect();
    packs.retain(|p| stable(p) || !replaced.contains(&(p.0.clone(), p.2.clone())));
    let mut packs: Vec<StickerPack> = packs
        .into_iter()
        .map(|(room, kind, key, content)| {
            let source = Some((room.as_str(), key.as_str()));
            sticker_pack(&content, format!("{kind}|{room}|{key}"), &key, source)
        })
        .collect();
    let own = global(&client, "m.image_pack").await;
    if !own.is_null() {
        packs.push(sticker_pack(&own, "account".into(), "", None));
    }
    packs.retain(|p| !p.stickers.is_empty());
    Ok(packs)
}

/// The image packs of a known room: the room, the event type, the state key and the content.
async fn room_packs(client: &Client, room: &str) -> Vec<(String, &'static str, String, Value)> {
    let Some(r) = matrix_sdk::ruma::RoomId::parse(room).ok().and_then(|id| client.get_room(&id)) else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for kind in PACK_TYPES {
        for mut e in states(&r, kind.into()).await {
            let key = e["state_key"].as_str().expect("a state event has a state key").to_string();
            // Desktop's `applyPackEvent` drops a pack with empty content.
            if e["content"].as_object().is_some_and(|c| !c.is_empty()) {
                packs.push((room.to_string(), kind, key, e["content"].take()));
            }
        }
    }
    packs
}

/// Desktop's `U0e` and `z0e`: the pack of image pack `content` and its images for stickers, one for each URL with all
/// its shortcodes. `source` is the room and the state key of a pack in a room.
fn sticker_pack(content: &Value, id: String, key: &str, source: Option<(&str, &str)>) -> StickerPack {
    // Desktop's `Ej`: an image without a usage is an emoji and a sticker.
    let sticker = |usage: &Value| usage.as_array().is_none_or(|u| u.is_empty() || u.iter().any(|x| x == "sticker"));
    let mut stickers: Vec<Sticker> = Vec::new();
    let images = content["images"].as_object().into_iter().flatten();
    let images =
        images.filter(|(_, i)| sticker(if i["usage"].is_null() { &content["pack"]["usage"] } else { &i["usage"] }));
    for (code, (url, i)) in images.filter_map(|(c, i)| Some((c, (s(&i["url"]).filter(|u| !u.is_empty())?, i)))) {
        match stickers.iter_mut().find(|x| x.url == url) {
            Some(x) => x.shortcodes.push(code.clone()),
            None => {
                let (body, info) = (s(&i["body"]), i["info"].clone());
                stickers.push(Sticker { url, shortcodes: vec![code.clone()], body, info, source: None });
            }
        }
    }
    for x in &mut stickers {
        x.source = source.map(|(room_id, state_key)| StickerSource {
            room_id: room_id.into(),
            state_key: state_key.into(),
            shortcode: x.shortcodes[0].clone(),
        });
    }
    let title = s(&content["pack"]["display_name"]).unwrap_or_else(|| key.into());
    StickerPack { id, title, avatar: s(&content["pack"]["avatar_url"]), stickers }
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
    // Other clients write this list too, so an entry that is not an emoji and a count is skipped.
    let recent = recent["recent_emoji"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e[0].as_str()?.into(), e[1].as_u64()?)));
    let recent = recent.collect();
    let tone = global(client, "com.beeper.emoji_skin_tone").await["tone"].as_u64().map_or(0, |t| t as u8);
    let (flags, usage) = (global(client, crate::features::FLAGS).await, global(client, crate::features::USAGE).await);
    let bridges = global(client, BRIDGE_STATE).await;
    sync_account_overrides(client, store).await;
    let viewed = global(client, REQUESTS_VIEWED).await;
    let muted = muted_rooms(client).await;
    let changed: Vec<String> = {
        let st = store.read();
        let old = &st.muted_rooms;
        muted.iter().filter(|r| !old.contains(r)).chain(old.iter().filter(|r| !muted.contains(r))).cloned().collect()
    };
    // A room that another device muted or unmuted through `com.beeper.muted`, unless a change of it is on its way.
    // An unmute there also drops the room's push rule, so that the mute goes everywhere.
    let mut flips = Vec::new();
    for id in &changed {
        let on = muted.contains(id);
        let pending = store.read().local_flag(id, ChatFlag::Muted).is_some();
        if let (false, false, Ok(room)) = (on, pending, matrix_sdk::ruma::RoomId::parse(id.as_str())) {
            if let Err(e) = client.notification_settings().await.delete_user_defined_room_rules(&room).await {
                eprintln!("Could not unmute {id}: {e}");
            }
        }
        flips.push((id.clone(), on));
    }
    let mut st = store.write();
    st.merge_requests_viewed(&viewed);
    st.muted_rooms = muted;
    for (id, on) in flips {
        if st.local_flag(&id, ChatFlag::Muted).is_none() {
            if let Some(i) = st.resolve_chat_idx(&id) {
                st.update_chat(i, |c| c.is_muted = Some(on));
            }
        }
    }
    st.recent_emoji = recent;
    st.skin_tone = tone;
    // The flags decide what the windows show (polls, Send Later, ...), so a change draws them again.
    if (&st.features.flags, &st.features.usage) != (&flags, &usage) {
        (st.features.flags, st.features.usage) = (flags, usage);
        st.emit_ws("setup.updated", "", None, None, None);
    }
    // Desktop's `BRIDGE_STATES_UPDATED`: the homeserver updates the bridge states while the app runs.
    if bridges.is_null() {
        return;
    }
    match crate::cloud::cloud_accounts(&bridges["bridges"]) {
        Ok(accounts) if accounts != st.bridge_accounts => {
            st.set_cloud_accounts(accounts);
            st.emit_ws("setup.updated", "", None, None, None);
        }
        Ok(_) => {}
        Err(e) => st.matrix_error = Some(e.message),
    }
}

/// Desktop's `userProfileStore`: the own name and avatar come from the Matrix profile.
async fn sync_profile(client: &Client, store: &SharedStore) {
    let profile = match client.account().fetch_user_profile().await {
        Ok(r) => r.data,
        Err(e) => return store.write().matrix_error = Some(e.to_string()),
    };
    let field = |k: &str| profile.get(k).and_then(Value::as_str).map(str::to_string);
    let mut st = store.write();
    let i = st.chat_idx(NOTE_CHAT_ID);
    st.update_chat(i, |c| {
        let me = &mut c.participants.items[0];
        me.full_name = field("displayname").or(me.full_name.take());
        me.img_url = field("avatar_url");
    });
}

/// Desktop's `userProfileStore.setAvatar` and `setFullName`: uploads the avatar image with its MIME type, sets the
/// name, and reads the profile again.
pub async fn set_profile(
    store: SharedStore,
    name: Option<String>,
    avatar: Option<(Vec<u8>, String)>,
) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let e = |e: &dyn std::fmt::Display| e.to_string();
    if let Some((bytes, mime)) = avatar {
        let mime: mime::Mime = mime.parse().expect("an image MIME type");
        let url = client.media().upload(&mime, bytes, None).await.map_err(|x| e(&x))?.content_uri;
        client.account().set_avatar_url(Some(&url)).await.map_err(|x| e(&x))?;
    }
    if let Some(name) = name {
        client.account().set_display_name(Some(&name)).await.map_err(|x| e(&x))?;
    }
    sync_profile(&client, &store).await;
    Ok(())
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
    // Desktop's `setFromAccountData`: without `enable_emoji_autocomplete`, the legacy `disable_emoji_autocomplete`
    // applies.
    let desktop = events.get_mut(crate::prefs::DESKTOP_PREFS).expect("Desktop prefs are synced");
    if desktop["enable_emoji_autocomplete"].is_null() {
        if let Some(off) = desktop["disable_emoji_autocomplete"].as_bool() {
            desktop["enable_emoji_autocomplete"] = (!off).into();
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
    if let Some(e) = events[crate::prefs::DESKTOP_PREFS][crate::prefs::QUICK_REACTION].as_str() {
        settings.quick_reaction = e.to_string();
    }
    if let Some(v) = events[crate::prefs::DESKTOP_PREFS].get(crate::prefs::TIME_SUGGESTION_OPTIONS) {
        settings.time_suggestions = crate::prefs::time_suggestion_options(v);
    }
    st.synced_prefs = events;
    let s = &st.settings;
    if (&settings.prefs, &settings.quick_reaction, &settings.time_suggestions)
        != (&s.prefs, &s.quick_reaction, &s.time_suggestions)
    {
        st.update_settings(|s| *s = settings);
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
    synced_write(st, kind, field, value)
}

/// The account data write that syncs the Quick React emoji to Desktop.
pub fn quick_reaction_write(st: &mut BeeperStore) -> Option<(&'static str, Value)> {
    let emoji = st.settings.quick_reaction.clone().into();
    synced_write(st, crate::prefs::DESKTOP_PREFS, crate::prefs::QUICK_REACTION, emoji)
}

/// The account data write of Desktop's `setTimeSuggestionOptions`, which syncs the time suggestions.
pub fn time_suggestions_write(st: &mut BeeperStore) -> Option<(&'static str, Value)> {
    let options = st.settings.time_suggestions.clone().into();
    synced_write(st, crate::prefs::DESKTOP_PREFS, crate::prefs::TIME_SUGGESTION_OPTIONS, options)
}

fn synced_write(st: &mut BeeperStore, kind: &'static str, field: &str, value: Value) -> Option<(&'static str, Value)> {
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

/// A request of the client-server API at `/_matrix/client/v3/{path}` with the session of the client, for the Matrix
/// proxy of the Desktop API, which passes bodies and replies through as they are. Gives the body of the reply, or the
/// Matrix error of the homeserver as `M_CODE (HTTP 404): error`.
async fn client_api(
    client: &Client,
    method: reqwest::Method,
    path: &[&str],
    query: &[(&str, &str)],
    body: Option<String>,
) -> Result<String, String> {
    let (status, text) = client_request(client, method, path, query, body).await?;
    match status.is_success() {
        true => Ok(text),
        false => Err(matrix_error(status, &serde_json::from_str(&text).unwrap_or_default())),
    }
}

/// The Matrix error `v` of a reply as `M_CODE (HTTP 404): error`.
fn matrix_error(status: reqwest::StatusCode, v: &Value) -> String {
    match v["errcode"].as_str() {
        Some(code) => format!("{code} (HTTP {}): {}", status.as_u16(), v["error"].as_str().unwrap_or_default()),
        None => format!("HTTP {status}"),
    }
}

/// The status and the body of a request of the client-server API at `/_matrix/client/v3/{path}`.
async fn client_request(
    client: &Client,
    method: reqwest::Method,
    path: &[&str],
    query: &[(&str, &str)],
    body: Option<String>,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut url = client.homeserver();
    url.path_segments_mut().expect("a base URL").pop_if_empty().extend(["_matrix", "client", "v3"]).extend(path);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    let token = client.access_token().ok_or("Not signed in to Matrix")?;
    let mut req = crate::cloud::http_client().request(method, url.as_str()).bearer_auth(token);
    if let Some(body) = body {
        req = req.header("content-type", "application/json").body(body);
    }
    let res = req.send().await.map_err(|e| e.without_url().to_string())?;
    let status = res.status();
    Ok((status, res.text().await.map_err(|e| e.without_url().to_string())?))
}

fn own_id(client: &Client) -> String {
    client.user_id().expect("signed-in client has a user ID").to_string()
}

/// Desktop SDK's `getAccountData`: the content of a global account data event of the user.
pub async fn get_account_data(client: Client, kind: String) -> Result<String, String> {
    client_api(&client, reqwest::Method::GET, &["user", &own_id(&client), "account_data", &kind], &[], None).await
}

/// Desktop SDK's `setAccountData`: sets a global account data event of the user to the JSON text `content`.
pub async fn set_account_data(client: Client, kind: String, content: String) -> Result<(), String> {
    let path = ["user", &own_id(&client), "account_data", &kind];
    client_api(&client, reqwest::Method::PUT, &path, &[], Some(content)).await.map(drop)
}

/// Desktop SDK's `getRoomAccountData`: the content of an account data event of the user in a room.
pub async fn get_room_account_data(client: Client, room: String, kind: String) -> Result<String, String> {
    let path = ["user", &own_id(&client), "rooms", &room, "account_data", &kind];
    client_api(&client, reqwest::Method::GET, &path, &[], None).await
}

/// Desktop SDK's `setRoomAccountData`: sets an account data event of the user in a room to the JSON text `content`.
pub async fn set_room_account_data(client: Client, room: String, kind: String, content: String) -> Result<(), String> {
    let path = ["user", &own_id(&client), "rooms", &room, "account_data", &kind];
    client_api(&client, reqwest::Method::PUT, &path, &[], Some(content)).await.map(drop)
}

/// Desktop SDK's `fetchRoomState`: the state events of a room, from the homeserver.
pub async fn fetch_room_state(client: Client, room: String) -> Result<String, String> {
    client_api(&client, reqwest::Method::GET, &["rooms", &room, "state"], &[], None).await
}

/// Desktop SDK's `fetchRoomStateEvent`: the whole state event of a type and state key, from the homeserver.
pub async fn fetch_room_state_event(client: Client, room: String, kind: String, key: String) -> Result<String, String> {
    let path = ["rooms", &room, "state", &kind, &key];
    client_api(&client, reqwest::Method::GET, &path, &[("format", "event")], None).await
}

/// Desktop SDK's `fetchEvent`: an event of a known room, decrypted when the client has its key.
pub async fn fetch_event(client: Client, room: String, event: String) -> Result<String, String> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room).map_err(|e| e.to_string())?;
    let room = client.get_room(&room_id).ok_or_else(|| format!("Room {room_id} not found"))?;
    let event = OwnedEventId::try_from(event).map_err(|e| e.to_string())?;
    Ok(room.event(&event, None).await.map_err(|e| e.to_string())?.raw().json().get().into())
}

/// Desktop SDK's `getProfile`: the profile of a user, from the homeserver.
pub async fn get_profile(client: Client, user: String) -> Result<String, String> {
    client_api(&client, reqwest::Method::GET, &["profile", &user], &[], None).await
}

/// Desktop SDK's `joinRoomByID` for a room ID, and `joinRoom` through the server `via`, when given, for an alias.
/// Gives the ID of the joined room.
pub async fn join_room(client: Client, room: String, via: String) -> Result<String, String> {
    use matrix_sdk::ruma::{OwnedServerName, RoomId, RoomOrAliasId};
    let room = RoomOrAliasId::parse(room).map_err(|e| e.to_string())?;
    let joined = match <&RoomId>::try_from(&*room) {
        Ok(id) => client.join_room_by_id(id).await,
        Err(_) => {
            let vias = match via.is_empty() {
                true => Vec::new(),
                false => vec![OwnedServerName::try_from(via).map_err(|e| e.to_string())?],
            };
            client.join_room_by_id_or_alias(&room, &vias).await
        }
    };
    Ok(joined.map_err(|e| e.to_string())?.room_id().to_string())
}

/// Desktop SDK's `leaveRoom`.
pub async fn leave_room(client: Client, room: String) -> Result<(), String> {
    client_api(&client, reqwest::Method::POST, &["rooms", &room, "leave"], &[], Some("{}".into())).await.map(drop)
}

/// Desktop SDK's `createRoom` with the JSON text of the request, which keeps keys that the SDK's request type drops.
/// Gives the ID of the new room.
pub async fn create_room(client: Client, request: String) -> Result<String, String> {
    let reply = client_api(&client, reqwest::Method::POST, &["createRoom"], &[], Some(request)).await?;
    let v: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
    v["room_id"].as_str().map(String::from).ok_or_else(|| "createRoom reply has no room_id".into())
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

/// Desktop SDK's client state: reads the encrypted messaging state of this device into the Beeper state, and
/// marks it `initialized`. A device that can cross-sign signs itself.
async fn refresh_e2ee(client: &Client, store: &SharedStore) {
    let encryption = client.encryption();
    let device = encryption.get_own_device().await.expect("read the own device");
    let device = device.expect("signed-in client has its own device");
    let keys = encryption.cross_signing_status().await.expect("signed-in client has an olm machine");
    // The new signature shows in the device keys only after the next key query.
    let mut verified = device.is_cross_signed_by_owner();
    if keys.has_self_signing && !verified {
        match device.verify().await {
            Ok(()) => verified = true,
            Err(e) => store.write().matrix_error = Some(format!("Could not sign this device: {e}")),
        }
    }
    let me = client.user_id().expect("signed-in client has a user ID");
    let identity = encryption.get_user_identity(me).await.expect("read the own identity").is_some();
    let backup = encryption.backups().are_enabled().await;
    let storage = matches!(encryption.recovery().state(), RecoveryState::Enabled | RecoveryState::Incomplete);
    let mut st = store.write();
    let old = st.beeper_state.clone();
    let s = &mut st.beeper_state;
    (s.initialized, s.cross_signing, s.verified, s.secret_storage, s.key_backup) =
        (true, identity, verified, storage, backup);
    let secrets = &mut s.secrets;
    (secrets.master_key, secrets.self_signing_key, secrets.user_signing_key, secrets.megolm_backup_key) =
        (keys.has_master, keys.has_self_signing, keys.has_user_signing, backup);
    if st.beeper_state != old {
        st.persist_state_to_sqlite();
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
    use matrix_sdk::ruma::{DeviceKeyAlgorithm, OwnedRoomId};
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
                backed_up: c.backup_version.is_some(),
                history_visibility: None,
                algorithm: EventEncryptionAlgorithm::MegolmV1AesSha2,
                shared_history: g.shared_history,
            })
            .map_err(|x| e(&x))?,
        );
    }
    // Desktop keeps only the secrets it has: a user without a recovery key has none, and verifies this device later.
    let secret = |name: &str| c.secrets.iter().find(|(n, _)| n == name).map(|(_, v)| unpickle_secret(v)).transpose();
    let user: OwnedUserId = c.user_id.parse().map_err(|x| e(&x))?;
    let identity = PrivateCrossSigningIdentity::empty(&user);
    let (master, self_signing, user_signing) = (
        secret("m.cross_signing.master")?,
        secret("m.cross_signing.self_signing")?,
        secret("m.cross_signing.user_signing")?,
    );
    identity
        .import_secrets_unchecked(master.as_deref(), self_signing.as_deref(), user_signing.as_deref())
        .await
        .map_err(|x| e(&x))?;
    let backup = secret("m.megolm_backup.v1")?.map(|k| BackupDecryptionKey::from_base64(&k)).transpose();
    let store = matrix_sdk::SqliteCryptoStore::open(dir, None).await.map_err(|x| e(&x))?;
    let changes = Changes {
        private_identity: Some(identity),
        backup_version: c.backup_version.clone(),
        backup_decryption_key: backup.map_err(|x| e(&x))?,
        inbound_group_sessions: groups,
        ..Default::default()
    };
    store.save_changes(changes).await.map_err(|x| e(&x))
}

/// The text of Desktop's error when the homeserver asks to sign in again before a reset.
pub const REAUTH: &str = "Re-authentication is required to continue";

/// Desktop SDK's user-interactive auth with the token of `updateUIAuthToken`: the Beeper JWT, for the auth `session`.
fn jwt_auth(store: &SharedStore, session: Option<String>) -> Result<AuthData, String> {
    let st = store.read();
    let token = st.ui_auth_token.clone().ok_or(REAUTH)?;
    let Value::Object(data) = serde_json::json!({ "token": token, "user": st.beeper_state.user_id }) else {
        unreachable!("the auth data is a JSON object")
    };
    Ok(AuthData::new("org.matrix.login.jwt", session, data).expect("a custom auth type"))
}

/// Desktop's check of a refused auth: the homeserver asks for auth again, or refuses the Beeper JWT.
fn auth_error(e: matrix_sdk::Error) -> String {
    match e.as_uiaa_response().is_some() || e.client_api_error_kind() == Some(&ErrorKind::Forbidden) {
        true => REAUTH.into(),
        false => e.to_string(),
    }
}

/// Desktop's `beginResetCrossSigning`: makes a new cross-signing identity, secret storage and key backup, and returns
/// their new recovery key. An account that has an identity loses it, like Desktop's reset. The recovery key
/// waits for [`confirm_recovery_key`]. The homeserver can ask for auth, which the Beeper JWT gives.
pub async fn new_recovery_key(store: &SharedStore) -> Result<String, String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let encryption = client.encryption();
    match encryption.request_user_identity(&me).await.map_err(|x| e(&x))? {
        // The cross-signing reset comes first: when the auth fails, the backup and secret storage stay.
        Some(_) => {
            if let Some(handle) = encryption.reset_cross_signing().await.map_err(|x| e(&x))? {
                let done = match handle.auth_type() {
                    CrossSigningResetAuthType::Uiaa(uiaa) => match jwt_auth(store, uiaa.session.clone()) {
                        Ok(auth) => handle.auth(Some(auth)).await.map_err(auth_error),
                        Err(x) => Err(x),
                    },
                    CrossSigningResetAuthType::OAuth(_) => Err("The homeserver asks for OAuth".into()),
                };
                if let Err(x) = done {
                    handle.cancel().await;
                    return Err(x);
                }
            }
            encryption.backups().disable_and_delete().await.map_err(|x| e(&x))?;
        }
        None => {
            if let Err(x) = encryption.bootstrap_cross_signing(None).await {
                let uiaa = x.as_uiaa_response().ok_or_else(|| e(&x))?;
                let auth = jwt_auth(store, uiaa.session.clone())?;
                encryption.bootstrap_cross_signing(Some(auth)).await.map_err(auth_error)?;
            }
        }
    }
    let key = encryption.recovery().enable().await.map_err(|x| e(&x))?;
    {
        let mut st = store.write();
        st.pending_recovery_key = Some(key.clone());
        let s = &mut st.beeper_state;
        (s.has_backed_up_code, s.secrets.recovery_code) = (false, key.clone());
        s.recovery_code_generated_at = crate::config::now_unix_millis();
        st.persist_state_to_sqlite();
        st.emit_ws("setup.updated", "", None, None, None);
    }
    refresh_e2ee(&client, store).await;
    Ok(key)
}

/// Desktop's `confirmResetCrossSigning`: the user typed the new recovery key again, so it is saved.
pub fn confirm_recovery_key(store: &SharedStore, key: &str) -> Result<(), String> {
    let mut st = store.write();
    let pending = st.pending_recovery_key.as_deref().ok_or("No new recovery key waits for a confirmation")?;
    if recovery_key_chars(pending) != recovery_key_chars(key) {
        return Err("The Recovery Key you entered doesn't match your Recovery Key.".into());
    }
    st.pending_recovery_key = None;
    st.beeper_state.has_backed_up_code = true;
    st.persist_state_to_sqlite();
    st.emit_ws("setup.updated", "", None, None, None);
    Ok(())
}

/// The letters and digits of a recovery key, which Desktop shows in groups with dashes.
pub fn recovery_key_chars(key: &str) -> String {
    key.chars().filter(char::is_ascii_alphanumeric).collect()
}

pub async fn recover(store: SharedStore, key: String) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let key = recovery_key_chars(&key);
    client.encryption().recovery().recover(&key).await.map_err(|e| match e {
        RecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(e)) => format!("Invalid recovery key: {e}"),
        e => e.to_string(),
    })?;
    // Desktop's SDK keeps the recovery key that verified this device, and decrypts the account data again with it.
    {
        let mut st = store.write();
        st.beeper_state.secrets.recovery_code = key;
        st.account_overrides_synced = Value::Null;
        st.persist_state_to_sqlite();
        st.emit_ws("setup.updated", "", None, None, None);
    }
    refresh_e2ee(&client, &store).await;
    Ok(())
}

/// Desktop's `AppStateTracker.handleAppEvent`: applies a change of the Matrix verification `id` to the tracked one.
/// A newer verification replaces the tracked one, like in Desktop, and the changes of the older one stop.
fn track(store: &SharedStore, id: &str, change: impl FnOnce(&mut AppVerification)) {
    let mut st = store.write();
    let Some(v) = st.verification.as_mut().filter(|v| v.id == id) else { return };
    change(v);
    v.available_actions = v.actions();
    // The secrets and the signature of the other device come later, and the next sync reads them.
    if v.state == "done" {
        (st.beeper_state.verified, st.beeper_state.cross_signing) = (true, true);
        st.persist_state_to_sqlite();
    }
    st.emit_ws("setup.updated", "", None, None, None);
}

/// Desktop's check of encrypted messaging after the sign-in (`pre-verification`).
#[derive(Clone)]
pub struct E2eeSetup {
    /// The account has a cross-signing identity.
    pub cross_signing: bool,
    pub verified: bool,
    /// The other devices that the account signed, which can verify this device.
    pub devices: Vec<DeviceInfo>,
}

/// Waits for the Matrix client of a new sign-in, and reads what its encrypted messaging needs.
pub async fn e2ee_setup(store: SharedStore) -> Result<E2eeSetup, String> {
    let client = loop {
        if let Some(c) = store.read().matrix.clone() {
            break c;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let encryption = client.encryption();
    let identity = encryption.request_user_identity(&me).await.map_err(|e| e.to_string())?;
    // The state of the app follows the check, so that the onboarding sees the same `verified`.
    refresh_e2ee(&client, &store).await;
    let verified = store.read().beeper_state.verified;
    let mut devices = Vec::new();
    for d in self::devices(client.clone()).await? {
        let id = d.id.as_str().into();
        let device = encryption.get_device(&me, id).await.map_err(|e| e.to_string())?;
        if !d.current && device.is_some_and(|d| d.is_cross_signed_by_owner()) {
            devices.push(d);
        }
    }
    Ok(E2eeSetup { cross_signing: identity.is_some(), verified, devices })
}

#[derive(Clone)]
pub struct DeviceInfo {
    pub id: String,
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
            id: d.device_id.to_string(),
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

/// Desktop SDK's `DeviceMetadata` of this device. The SDK sends Go's `runtime.GOOS` and `GOARCH` as `os` and
/// `architecture`. It leaves out `os_version` on a computer, and beeper-rs has no SDK build for the
/// `build_git_commit`, `build_git_branch` and `build_go_version` that the SDK adds.
fn device_metadata(device_id: &str) -> Value {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        arch => arch,
    };
    serde_json::json!({
        "device_id": device_id,
        "app_type": "desktop",
        "app_name": crate::config::DEVICE_NAME,
        "app_version": crate::config::APP_VERSION,
        "device_model": crate::config::DEVICE_MODEL,
        "device_type": "computer",
        "os": os,
        "architecture": architecture,
    })
}

/// Desktop SDK's `updateDeviceMetadata`, which runs at each start: it sets the `com.beeper.device_metadata.<device
/// ID>` account data of this device when the stored one differs.
async fn update_device_metadata(client: Client) {
    let id = client.device_id().expect("signed-in client has a device ID").to_string();
    let kind = format!("com.beeper.device_metadata.{id}");
    let meta = device_metadata(&id);
    if global(&client, &kind).await != meta {
        let raw = Raw::from_json(serde_json::value::to_raw_value(&meta).expect("account data JSON"));
        if let Err(e) = client.account().set_account_data_raw(kind.into(), raw).await {
            eprintln!("Could not set the device metadata: {e}");
        }
    }
}

/// Desktop's `deleteDevicesByID` for another device of the account. The homeserver asks for auth, which the Beeper
/// JWT gives.
pub async fn delete_device(store: SharedStore, id: String) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let ids = [OwnedDeviceId::from(id)];
    if let Err(x) = client.delete_devices(&ids, None).await {
        let uiaa = x.as_uiaa_response().ok_or_else(|| x.to_string())?;
        let auth = jwt_auth(&store, uiaa.session.clone())?;
        client.delete_devices(&ids, Some(auth)).await.map_err(|x| auth_error(x.into()))?;
    }
    Ok(())
}

pub async fn logout(store: SharedStore) -> Result<(), String> {
    let (client, demo, dir) = {
        let st = store.read();
        (st.matrix.clone(), st.config.demo, st.config.data_dir.clone())
    };
    if !demo {
        client.ok_or("Matrix is not connected yet")?.logout().await.map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join(crate::store::LOGGED_OUT), "").map_err(|e| format!("Could not mark the logout: {e}"))
}

/// The verification methods of this device: emojis, and QR codes that it shows, or that an API client scans for it.
const METHODS: [VerificationMethod; 4] = [
    VerificationMethod::SasV1,
    VerificationMethod::QrCodeShowV1,
    VerificationMethod::QrCodeScanV1,
    VerificationMethod::ReciprocateV1,
];

/// Desktop's `startVerification`: asks the other devices of the account to verify this device.
pub async fn start_verification(store: &SharedStore, purpose: &'static str) -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let identity = client.encryption().get_user_identity(&me).await.map_err(|x| e(&x))?;
    let identity = identity.ok_or("this account has no cross-signing identity")?;
    let request = identity.request_verification_with_methods(METHODS.to_vec()).await;
    begin(store, request.map_err(|x| e(&x))?, purpose, None);
    Ok(())
}

/// A verification request from another device of the account: Desktop's `VERIFICATION_REQUESTED`.
async fn verification_requested(ev: ToDeviceKeyVerificationRequestEvent, client: Client, store: Ctx<SharedStore>) {
    if client.user_id() != Some(&*ev.sender) {
        return;
    }
    let encryption = client.encryption();
    // The SDK drops the requests that are too old to answer, like the ones of a long time offline.
    let Some(request) = encryption.get_verification_request(&ev.sender, &ev.content.transaction_id).await else {
        return;
    };
    match encryption.get_device(&ev.sender, &ev.content.from_device).await {
        Ok(device) => {
            let name = device.and_then(|d| d.display_name().map(str::to_string));
            begin(&store, request, "device", Some(VerificationDevice { id: ev.content.from_device.to_string(), name }));
        }
        Err(e) => store.write().matrix_error = Some(e.to_string()),
    }
}

/// Tracks a new verification like Desktop's `AppStateTracker`, and runs it until it ends. A verification with
/// `from` a device is incoming.
fn begin(store: &SharedStore, request: VerificationRequest, purpose: &'static str, from: Option<VerificationDevice>) {
    let (tx, rx) = oneshot::channel();
    let id = request.flow_id().to_string();
    {
        let mut st = store.write();
        st.verification = Some(AppVerification {
            id: id.clone(),
            purpose,
            direction: if from.is_some() { "incoming" } else { "outgoing" },
            state: "requested",
            methods: Vec::new(),
            available_actions: Vec::new(),
            other_user_id: from.is_some().then(|| request.other_user_id().to_string()),
            other_device: from,
            qr: None,
            sas: None,
            error: None,
        });
        st.sas_confirm = Some(tx);
    }
    track(store, &id, |_| {});
    let s = store.clone();
    tokio::spawn(async move {
        if let Err(e) = run_verification(&s, request, rx).await {
            s.write().matrix_error = Some(e);
        }
    });
}

/// Runs a verification to its end. `confirm` carries the answer of the user to the emojis.
async fn run_verification(
    store: &SharedStore,
    request: VerificationRequest,
    confirm: oneshot::Receiver<bool>,
) -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let id = request.flow_id().to_string();
    let cancelled = |info: CancelInfo| {
        let error = VerificationError { code: info.cancel_code().to_string(), reason: info.reason().into() };
        track(store, &id, |v| (v.state, v.error) = ("cancelled", Some(error)));
        Ok(())
    };
    let mut requests = request.changes();
    let mut sas = None;
    let mut sas_changes = futures_util::stream::pending().boxed();
    let mut qr_changes = futures_util::stream::pending().boxed();
    let mut confirm = Some(confirm);
    loop {
        tokio::select! {
            state = requests.next() => match state.ok_or("the verification request ended")? {
                VerificationRequestState::Ready { their_methods, other_device_data, .. } if request.is_ready() => {
                    let emojis = their_methods.contains(&VerificationMethod::SasV1);
                    let scans = their_methods.contains(&VerificationMethod::QrCodeScanV1);
                    // Desktop's `qr_data`: the QR code for the other device to scan, when it can scan one.
                    let qr = match scans {
                        true => request.generate_qr_code().await.map_err(|x| e(&x))?,
                        false => None,
                    };
                    let qr = qr.map(|q| q.to_bytes()).transpose().map_err(|x| e(&x))?;
                    track(store, &id, |v| {
                        v.state = "ready";
                        v.methods = [(scans, "qr"), (emojis, "sas")].into_iter().filter(|m| m.0).map(|m| m.1).collect();
                        let name = other_device_data.display_name().map(str::to_string);
                        v.other_device = Some(VerificationDevice { id: other_device_data.device_id().to_string(), name });
                        v.qr = qr.map(|data| VerificationQr { data });
                    });
                    // Desktop starts the emojis when it accepted the request, and waits for a choice when it sent it.
                    if !request.we_started() && emojis {
                        request.start_sas().await.map_err(|x| e(&x))?;
                    }
                }
                VerificationRequestState::Transitioned { verification: Verification::QrV1(qr), .. } => {
                    qr_changes = qr.changes().boxed();
                }
                VerificationRequestState::Transitioned { verification: Verification::SasV1(s), .. } => {
                    // The device that did not send the start accepts it. `we_started` tells who sent the request.
                    if matches!(s.state(), SasState::Started { .. }) {
                        s.accept().await.map_err(|x| e(&x))?;
                    }
                    sas_changes = s.changes().boxed();
                    sas = Some(s);
                }
                VerificationRequestState::Cancelled(c) => return cancelled(c),
                _ => {}
            },
            Some(state) = sas_changes.next() => match state {
                SasState::KeysExchanged { emojis: Some(emojis), decimals: (a, b, c) } => {
                    let emojis = emojis.emojis.iter();
                    let emojis =
                        emojis.map(|x| VerificationSasEmoji { emoji: x.symbol.into(), description: x.description.into() });
                    let shown = VerificationSas { emojis: emojis.collect(), decimals: format!("{a} {b} {c}") };
                    track(store, &id, |v| {
                        v.state = "sas_ready";
                        if !v.methods.contains(&"sas") {
                            v.methods.push("sas");
                        }
                        v.sas = Some(shown);
                    });
                    let sas = sas.as_ref().expect("SAS changes come from a SAS");
                    match confirm.take().expect("emojis are shown once").await {
                        Ok(true) => sas.confirm().await.map_err(|x| e(&x))?,
                        _ => sas.cancel().await.map_err(|x| e(&x))?,
                    }
                }
                SasState::Done { .. } => {
                    track(store, &id, |v| v.state = "done");
                    return Ok(());
                }
                SasState::Cancelled(c) => return cancelled(c),
                _ => {}
            },
            Some(state) = qr_changes.next() => match state {
                QrVerificationState::Scanned => track(store, &id, |v| v.state = "qr_scanned"),
                QrVerificationState::Done { .. } => {
                    track(store, &id, |v| v.state = "done");
                    return Ok(());
                }
                QrVerificationState::Cancelled(c) => return cancelled(c),
                _ => {}
            },
        }
    }
}

/// The Matrix request of the verification with the flow ID `id`, if the SDK still has it.
async fn verification_request(store: &SharedStore, id: &str) -> Result<Option<VerificationRequest>, String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID");
    Ok(client.encryption().get_verification_request(me, id).await)
}

/// Desktop's `acceptVerification`: answers a request from another device.
pub async fn accept_verification(store: &SharedStore, id: &str) -> Result<(), String> {
    let request = verification_request(store, id).await?.ok_or("The verification request is gone")?;
    request.accept_with_methods(METHODS.to_vec()).await.map_err(|e| e.to_string())
}

/// Desktop's `startSAS`: compares emojis instead of the QR code.
pub async fn start_sas(store: &SharedStore, id: &str) -> Result<(), String> {
    let request = verification_request(store, id).await?.ok_or("The verification request is gone")?;
    request.start_sas().await.map(drop).map_err(|e| e.to_string())
}

/// Desktop's `confirmQRCodeScanned`: the other device scanned the QR code of this device.
pub async fn confirm_qr(store: &SharedStore, id: &str) -> Result<(), String> {
    let client = store.read().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID");
    match client.encryption().get_verification(me, id).await {
        Some(Verification::QrV1(qr)) => qr.confirm().await.map_err(|e| e.to_string()),
        _ => Err("The QR code verification is gone".into()),
    }
}

/// Desktop's `handleScannedQRData`: this device scanned the QR code of the other device, as base64.
pub async fn scan_qr(store: &SharedStore, data: &str) -> Result<(), String> {
    let bytes = vodozemac::base64_decode(data.trim_end_matches('=')).map_err(|e| e.to_string())?;
    let data = QrVerificationData::from_bytes(bytes).map_err(|e| e.to_string())?;
    let request = verification_request(store, data.flow_id()).await?.ok_or("The verification request is gone")?;
    match request.scan_qr_code(data).await.map_err(|e| e.to_string())? {
        Some(_) => Ok(()),
        None => Err("The verification request is not ready for a QR code".into()),
    }
}

/// Desktop's `cancelVerification`: cancels the request, or the emojis when they are on the screen.
pub async fn cancel_verification(store: &SharedStore, id: &str) -> Result<(), String> {
    if let Some(tx) = store.write().sas_confirm.take() {
        tx.send(false).ok();
    }
    match verification_request(store, id).await? {
        Some(request) => request.cancel().await.map_err(|e| e.to_string()),
        None => Ok(()),
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
    {
        let mut st = store.write();
        (st.matrix, st.syncing) = (Some(client.clone()), true);
    }
    tokio::spawn(push_loop(store.clone()));
    tokio::spawn(update_device_metadata(client.clone()));
    tokio::spawn(retry_undecrypted(client.clone(), store.clone()));
    tokio::spawn(retry_after_backup(client.clone(), store.clone()));
    // Like Desktop's SDK, the state is `initialized` when the device knows the identity of the account. When the
    // query fails, the first sync reads the state.
    client.encryption().wait_for_e2ee_initialization_tasks().await;
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    match client.encryption().request_user_identity(&me).await {
        Ok(_) => refresh_e2ee(&client, &store).await,
        Err(e) => store.write().matrix_error = Some(e.to_string()),
    }
    client.add_event_handler_context(store.clone());
    client.add_event_handler(verification_requested);
    sync_account_data(&client, &store).await;
    sync_prefs(&client, &store).await;
    sync_profile(&client, &store).await;
    store.write().inbox_rules = global(&client, RULES).await;
    for room in client.invited_rooms() {
        sync_invite(&store, &room).await;
    }
    // The chats that stay after a leave or a removal, without history to load again.
    for room in client.left_rooms() {
        let chat = chat_id(&room).await;
        if store.read().resolve_chat_idx(&chat).is_some() {
            sync_room(&store, &room, Vec::new()).await;
        }
    }
    let first = !store.read().beeper_state.first_sync_done;
    let rooms = client.joined_rooms();
    store.write().initial_sync = (0, rooms.len());
    futures_util::stream::iter(rooms)
        .for_each_concurrent(8, |room| {
            let store = store.clone();
            async move {
                let events = if first || has_utd(&store, &room).await { history(&room).await } else { Vec::new() };
                sync_room(&store, &room, events).await;
                store.write().initial_sync.0 += 1;
            }
        })
        .await;
    if first {
        let mut st = store.write();
        st.beeper_state.first_sync_done = true;
        st.persist_state_to_sqlite();
    }
    // Desktop's SDK catches up at once at the start, so "Syncing..." ends without waiting for a long poll.
    let now = SyncSettings::default().timeout(std::time::Duration::ZERO);
    if !on_sync(&client, &store, client.sync_once(now).await).await {
        return;
    }
    let settings = SyncSettings::default().timeout(std::time::Duration::from_secs(30));
    let client2 = client.clone();
    client
        .sync_with_result_callback(settings, |r| {
            let (client, store) = (client2.clone(), store.clone());
            async move {
                Ok(match on_sync(&client, &store, r).await {
                    true => LoopCtrl::Continue,
                    false => LoopCtrl::Break,
                })
            }
        })
        .await
        .expect("Matrix sync loop");
}

/// Applies a response of the Matrix sync loop: the account data, the rooms, the invites, and the left rooms. False
/// when the session has ended, which stops the loop.
async fn on_sync(client: &Client, store: &SharedStore, r: matrix_sdk::Result<matrix_sdk::sync::SyncResponse>) -> bool {
    match r {
        // The homeserver no longer knows this device's token: it was signed out from another device or the session
        // was revoked. The next start signs in again on clean data, as after Log Out.
        Err(e) if matches!(e.client_api_error_kind(), Some(ErrorKind::UnknownToken(_))) => {
            let mut st = store.write();
            if let Err(e) = std::fs::write(st.config.data_dir.join(crate::store::LOGGED_OUT), "") {
                eprintln!("Could not mark the ended session: {e}");
            }
            st.matrix_error = Some(e.to_string());
            st.session_ended = true;
            st.emit_ws("setup.updated", "", None, None, None);
            return false;
        }
        Err(e) => {
            let mut st = store.write();
            if st.matrix_error.replace(e.to_string()).is_none() {
                // The "Offline" pill and the API's app state follow the error.
                st.emit_ws("setup.updated", "", None, None, None);
            }
        }
        Ok(resp) => {
            {
                let mut st = store.write();
                // The homeserver answers again, so the changes that waited for it go now.
                let back = st.matrix_error.take().is_some();
                if back {
                    st.outbox_ready.notify_one();
                }
                // "Syncing..." and "Offline" end with this sync, so the windows draw again.
                if back || std::mem::replace(&mut st.syncing, false) {
                    st.emit_ws("setup.updated", "", None, None, None);
                }
                st.syncs += 1;
            }
            refresh_e2ee(client, store).await;
            sync_account_data(client, store).await;
            sync_prefs(client, store).await;
            sync_rules(client, store).await;
            for id in resp.rooms.invited.keys() {
                sync_invite(store, &client.get_room(id).expect("an invited room is known")).await;
            }
            for (id, update) in resp.rooms.left {
                match client.get_room(&id) {
                    Some(room) if room.state() == RoomState::Left => {
                        sync_left(store, &room, update.timeline.events).await
                    }
                    // A forgotten room, like a deleted label, is gone from the client.
                    _ => store.write().remove_chat(id.as_str()),
                }
            }
            for (id, update) in resp.rooms.joined {
                let room = client.get_room(&id).expect("synced room is known");
                // A limited timeline of a room that synced before leaves out the events since the last sync.
                let mut gap = update.timeline.prev_batch.clone().filter(|_| update.timeline.limited);
                if gap.is_some() && !synced_before(store, &room).await {
                    gap = None;
                }
                sync_room(store, &room, update.timeline.events).await;
                if let Some(from) = gap {
                    tokio::spawn(fill_gap(store.clone(), room.clone(), from));
                }
                for e in &update.ephemeral {
                    let v: Value = e.deserialize_as().expect("ephemeral event JSON");
                    if v["type"] == "m.typing" {
                        set_typing(store, &room, &v["content"]["user_ids"]).await;
                    } else if v["type"] == "m.receipt" {
                        sync_receipts(store, &room).await;
                        let me = room.own_user_id().as_str();
                        let receipts = v["content"].as_object().into_iter().flatten();
                        if receipts.into_iter().any(|(_, r)| r["m.read"].get(me).is_some()) {
                            let chat = chat_id(&room).await;
                            let roots = {
                                let st = store.read();
                                let reads = st.chat_idx_by_id.get(&chat).map(|&i| &st.chats[i].thread_reads);
                                reads.into_iter().flat_map(|r| r.keys().cloned()).collect()
                            };
                            sync_thread_reads(store, &room, roots).await;
                        }
                    }
                }
            }
            store.write().synced();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::testing::{stub_message, stub_store, stub_sync};
    use serde_json::{json, Value};

    /// Desktop keeps a chat that the user was removed from: its own member has exited, the removal previews it, and
    /// it stays muted. A chat that was deleted here stays deleted when the leave of its room comes.
    #[tokio::test]
    async fn a_removal_keeps_the_chat_like_desktop() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let syncs = AtomicUsize::new(0);
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/sync" if syncs.fetch_add(1, Relaxed) == 0 => {
                // The room is muted: a room rule without actions.
                let mute = json!({ "rule_id": "!room:x", "default": false, "enabled": true, "actions": [] });
                let rules = json!({ "type": "m.push_rules", "content": { "global": { "room": [mute] } } });
                let mut sync = stub_sync();
                sync["account_data"] = json!({ "events": [rules] });
                (200, sync)
            }
            "/_matrix/client/v3/sync" => {
                let kick = json!({ "type": "m.room.member", "state_key": "@me:x", "sender": "@vic:x",
                    "event_id": "$kick", "origin_server_ts": 1_700_000_000_000_i64, "content": { "membership": "leave" } });
                let left = json!({ "state": { "events": [] }, "timeline": { "events": [kick] } });
                (200, json!({ "next_batch": "s2", "rooms": { "leave": { "!room:x": left } } }))
            }
            p if p.ends_with("/messages") => (200, json!({ "start": "t0", "chunk": [] })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("removal"), &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        let room = client.get_room(matrix_sdk::ruma::room_id!("!room:x")).expect("the joined room");
        super::sync_room(&store, &room, Vec::new()).await;
        super::on_sync(&client, &store, client.sync_once(super::SyncSettings::default()).await).await;
        {
            let st = store.read();
            let c = st.chat("!room:x");
            assert!(c.exited(), "the user's member has exited");
            assert_eq!(c.is_muted, Some(true), "the room rule still mutes it");
            assert_eq!(c.preview.as_ref().map(|m| m.id.as_str()), Some("$kick"), "the removal previews the chat");
        }
        super::load_older(store.clone(), "!room:x".into()).await.expect("its history still loads");
        store.write().remove_chat("!room:x");
        super::sync_left(&store, &room, Vec::new()).await;
        assert!(store.read().resolve_chat_idx("!room:x").is_none(), "a deleted chat stays deleted");
    }

    /// Like Desktop's SDK on Linux, a start sets the device metadata when the stored one is missing or differs, and
    /// sends nothing when it matches.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[tokio::test]
    async fn device_metadata_like_desktop() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        use std::sync::{Arc, Mutex};
        let meta = json!({ "device_id": "DEV", "app_type": "desktop", "app_name": "Beeper Desktop v4 (Linux)",
            "app_version": "4.3.308", "device_model": "Linux PC", "device_type": "computer", "os": "linux",
            "architecture": "amd64" });
        // The account data of the second and the third sync: the metadata that the first start set, then the
        // metadata of Desktop's SDK, with its build.
        let stored = Arc::new(Mutex::new(Value::Null));
        let (sent, next) = (Arc::new(Mutex::new(Vec::new())), stored.clone());
        let (log, syncs) = (sent.clone(), AtomicUsize::new(0));
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                "/_matrix/client/v3/sync" => {
                    let content = next.lock().unwrap().clone();
                    let mut sync = stub_sync();
                    // A new batch each time, since the client skips a batch that it has.
                    sync["next_batch"] = json!(format!("s{}", syncs.fetch_add(1, Relaxed)));
                    if !content.is_null() {
                        let event = json!({ "type": "com.beeper.device_metadata.DEV", "content": content });
                        sync["account_data"] = json!({ "events": [event] });
                    }
                    (200, sync)
                }
                p if p.contains("device_metadata") => {
                    log.lock().unwrap().push((method.to_string(), p.to_string(), body));
                    (200, json!({}))
                }
                _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("device-metadata"), &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        super::update_device_metadata(client.clone()).await;
        let put = ("PUT".to_string(), "/_matrix/client/v3/user/@me:x/account_data/com.beeper.device_metadata.DEV");
        assert_eq!(*sent.lock().unwrap(), [(put.0.clone(), put.1.to_string(), meta.clone())], "a new device sets it");
        *stored.lock().unwrap() = meta.clone();
        client.sync_once(super::SyncSettings::default()).await.expect("sync the metadata");
        super::update_device_metadata(client.clone()).await;
        assert_eq!(sent.lock().unwrap().len(), 1, "the same metadata sends nothing");
        let mut desktop = meta.clone();
        desktop["build_go_version"] = json!("go1.25.1");
        *stored.lock().unwrap() = desktop;
        client.sync_once(super::SyncSettings::default()).await.expect("sync Desktop's metadata");
        super::update_device_metadata(client).await;
        assert_eq!(sent.lock().unwrap()[1..], [(put.0, put.1.to_string(), meta)], "other metadata is replaced");
    }

    /// A device signed out from elsewhere gets M_UNKNOWN_TOKEN from the sync: the loop stops, and the next start signs
    /// in again on clean data instead of staying offline.
    #[tokio::test]
    async fn an_ended_session_starts_over() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let syncs = AtomicUsize::new(0);
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/sync" if syncs.fetch_add(1, Relaxed) == 0 => (200, stub_sync()),
            "/_matrix/client/v3/sync" => (
                401,
                json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Unrecognised access token", "soft_logout": false }),
            ),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let dir = crate::testing::temp_dir("ended-session");
        let store = stub_store(&dir, &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        let more = super::on_sync(&client, &store, client.sync_once(super::SyncSettings::default()).await).await;
        assert!(!more, "the sync loop stops");
        assert!(store.read().session_ended, "the app starts over");
        assert!(dir.join(crate::store::LOGGED_OUT).exists(), "the next start deletes the data");
    }

    /// A homeserver that refuses the history of a room neither stops the sync of the room, which loads it again
    /// later, nor shows an error for a room the user was banned from, whose history simply ends there.
    #[tokio::test]
    async fn refused_history_like_desktop() {
        use crate::models::Older;
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let syncs = AtomicUsize::new(0);
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/sync" if syncs.fetch_add(1, Relaxed) == 0 => (200, stub_sync()),
            "/_matrix/client/v3/sync" => {
                let ban = json!({ "type": "m.room.member", "state_key": "@me:x", "sender": "@vic:x",
                    "event_id": "$ban", "origin_server_ts": 1_700_000_000_000_i64, "content": { "membership": "ban" } });
                let left = json!({ "state": { "events": [] }, "timeline": { "events": [ban] } });
                (200, json!({ "next_batch": "s2", "rooms": { "leave": { "!room:x": left } } }))
            }
            p if p.ends_with("/messages") => (403, json!({ "errcode": "M_FORBIDDEN", "error": "not in room" })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("refused-history"), &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        let room = client.get_room(matrix_sdk::ruma::room_id!("!room:x")).expect("the joined room");
        super::sync_room(&store, &room, Vec::new()).await;
        assert_eq!(store.read().chat("!room:x").older, Older::Unknown, "the next load tries again");
        super::on_sync(&client, &store, client.sync_once(super::SyncSettings::default()).await).await;
        super::load_older(store.clone(), "!room:x".into()).await.expect("no error for a banned room");
        let st = store.read();
        assert_eq!(st.chat("!room:x").older, Older::Start, "its history ends there");
        assert!(st.chat("!room:x").exited(), "a load of the room keeps the user's member exited");
    }

    #[tokio::test]
    async fn older_messages_load_back_to_the_start_of_the_room_like_desktop() {
        use crate::models::Older;
        let dir = crate::testing::temp_dir("older");
        let store = crate::testing::stub_room_store(&dir, &crate::testing::stub_homeserver().await).await;
        let state = || {
            let st = store.read();
            let texts = st.timeline_indices("!room:x").iter().map(|&i| st.messages[i].text.clone().unwrap());
            (texts.collect::<Vec<_>>(), st.chat("!room:x").older.clone())
        };
        let texts = |a: i64| (a..=70).map(|i| format!("m{i}")).collect::<Vec<_>>();
        assert_eq!(state(), (texts(41), Older::From("t40".into())), "the first sync loads the newest 30 events");
        super::load_older(store.clone(), "!room:x".into()).await.expect("load older messages");
        assert_eq!(state(), (texts(11), Older::From("t10".into())), "older messages go before the loaded ones");
        super::load_older(store.clone(), "!room:x".into()).await.expect("load older messages");
        assert_eq!(state(), (texts(1), Older::Start), "the last page has no token");
        super::load_older(store.clone(), "!room:x".into()).await.expect("load at the start of the room");
        assert_eq!(state().0.len(), 70);
        drop(store);
        let cfg = crate::config::CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23392, None);
        let st = crate::store::BeeperStore::from_config(cfg);
        assert_eq!(st.chat("!room:x").older, Older::Start, "the pagination state persists");
    }

    /// A store with the stub rooms `!room:x`, whose history is `events`, and `other.0`, whose history is `other.1`.
    /// The stub also gives each event of `events` and sends any event.
    async fn stub_history(
        name: &str,
        events: Vec<Value>,
        other: (&'static str, Vec<Value>),
    ) -> crate::store::SharedStore {
        let mut sync = stub_sync();
        sync["rooms"]["join"][other.0] = sync["rooms"]["join"]["!room:x"].clone();
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, sync.clone()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => {
                let room = crate::codec::url_decode(p).expect("an encoded room ID");
                let page = if room.contains(other.0) { &other.1 } else { &events };
                (200, json!({ "start": "t", "chunk": page.iter().rev().collect::<Vec<_>>() }))
            }
            p if p.contains("/event/") => {
                let id = crate::codec::url_decode(p.rsplit('/').next().expect("an event ID")).expect("an encoded ID");
                (200, events.iter().find(|e| e["event_id"] == id.as_str()).expect("a stub event").clone())
            }
            p if p.contains("/send/") => (200, json!({ "event_id": "$sent" })),
            p if p.contains("/account_data/") || p.contains("/tags/") => (200, json!({})),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        stub_store(&crate::testing::temp_dir(name), &url).await
    }

    /// Desktop's `lastReadMessageSortKey` of a room: the order of the event of your own read receipt, which a read
    /// here goes ahead of until the receipt catches up.
    #[tokio::test]
    async fn own_receipts_set_the_read_position_like_desktop() {
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/sync" => {
                let mut sync = stub_sync();
                let receipt =
                    json!({ "type": "m.receipt", "content": { "$e2": { "m.read": { "@me:x": { "ts": 1 } } } } });
                sync["rooms"]["join"]["!room:x"]["ephemeral"] = json!({ "events": [receipt] });
                (200, sync)
            }
            p if p.ends_with("/messages") => {
                (200, json!({ "start": "t3", "chunk": [stub_message(3), stub_message(2), stub_message(1)] }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = crate::testing::stub_room_store(&crate::testing::temp_dir("own-receipt"), &url).await;
        let read = |store: &crate::store::SharedStore| store.read().chat("!room:x").last_read_message_sort_key.clone();
        assert_eq!(read(&store), Some(super::sort_key(2)), "the receipt of m2");
        let idx = store.read().chat_idx("!room:x");
        store.write().mark_read(idx, false);
        assert_eq!(read(&store), Some(super::sort_key(3)), "a read here");
        let room = store.read().matrix.clone().unwrap().get_room(matrix_sdk::ruma::room_id!("!room:x")).unwrap();
        super::sync_room(&store, &room, Vec::new()).await;
        assert_eq!(read(&store), Some(super::sort_key(3)), "the older receipt does not move it back");
    }

    /// A limited sync of a room that synced before leaves out the events since the last sync, and pages back from its
    /// `prev_batch` fill the gap up to the messages the chat has, so the timeline has no hole.
    #[tokio::test]
    async fn limited_syncs_fill_the_gap() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        use std::sync::Arc;
        let (syncs, pages) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (syncs2, pages2) = (syncs.clone(), pages.clone());
        let url = crate::bridge_stub::serve(move |_, path, _| {
            let (path, query) = path.split_once('?').unwrap_or((path, ""));
            match path {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                "/_matrix/client/v3/sync" if syncs2.fetch_add(1, Relaxed) == 0 => (200, stub_sync()),
                "/_matrix/client/v3/sync" => {
                    let events: Vec<Value> = (131..=140).map(stub_message).collect();
                    let timeline = json!({ "events": events, "limited": true, "prev_batch": "t130" });
                    let room = json!({ "state": { "events": [] }, "timeline": timeline });
                    (200, json!({ "next_batch": "s2", "rooms": { "join": { "!room:x": room } } }))
                }
                // 30 messages back from the token `tN` of message N, or the newest 30 of the first sync.
                p if p.ends_with("/messages") => {
                    pages2.fetch_add(1, Relaxed);
                    let from = query.split('&').find_map(|kv| kv.strip_prefix("from=t"));
                    let to: i64 = from.map_or(70, |t| t.parse().expect("a stub token"));
                    let chunk: Vec<Value> = (to - 29..=to).rev().map(stub_message).collect();
                    (200, json!({ "start": format!("t{to}"), "end": format!("t{}", to - 30), "chunk": chunk }))
                }
                _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
            }
        })
        .await;
        let store = crate::testing::stub_room_store(&crate::testing::temp_dir("sync-gap"), &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        let texts = |store: &crate::store::SharedStore| {
            let mut st = store.write();
            st.load_chat("!room:x");
            let texts = st.timeline_indices("!room:x").iter().map(|&i| st.messages[i].text.clone().unwrap());
            texts.collect::<Vec<_>>()
        };
        let range = |a: i64, b: i64| (a..=b).map(|i| format!("m{i}")).collect::<Vec<_>>();
        assert_eq!(texts(&store), range(41, 70), "the first sync loads the newest page");
        assert_eq!(pages.load(Relaxed), 1);
        super::on_sync(&client, &store, client.sync_once(super::SyncSettings::default()).await).await;
        for _ in 0..200 {
            if texts(&store).len() == 100 && pages.load(Relaxed) == 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(texts(&store), range(41, 140), "the gap from m71 to m130 is filled");
        assert_eq!(pages.load(Relaxed), 4, "the fill stops at the page that reaches a loaded message");
    }

    /// A message that waits for its key is fetched again by event when keys come, so one on an older page decrypts.
    #[tokio::test]
    async fn waiting_messages_decrypt_by_event() {
        let at = 1_700_000_000_000_i64;
        let megolm =
            json!({ "algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "x", "sender_key": "k", "session_id": "s" });
        let enc = json!({ "type": "m.room.encrypted", "event_id": "$utd", "sender": "@vic:x", "origin_server_ts": at,
            "content": megolm });
        let plain = json!({ "type": "m.room.message", "event_id": "$utd", "sender": "@vic:x", "origin_server_ts": at,
            "content": { "msgtype": "m.text", "body": "decrypted at last" } });
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, json!({ "start": "t", "chunk": [enc.clone()] })),
            p if p.contains("/event/") => (200, plain.clone()),
            p if p.contains("/account_data/") || p.contains("/tags/") => (200, json!({})),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("redecrypt"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        let text = |store: &crate::store::SharedStore| store.read().stored_message("$utd").and_then(|m| m.text);
        assert_eq!(text(&store).as_deref(), Some(super::UTD_TEXT), "the message waits for its key");
        let room = store.read().matrix.clone().unwrap().get_room(matrix_sdk::ruma::room_id!("!room:x")).unwrap();
        super::refetch_undecrypted(&store, vec![room]).await;
        assert_eq!(text(&store).as_deref(), Some("decrypted at last"));
    }

    /// An edit that waits for its key is no newer message once it decrypts, so a chat archived up to the edited
    /// message stays archived, like Desktop, whose edit only changes the row of the edited message.
    #[tokio::test]
    async fn a_decrypted_edit_keeps_the_archive_like_desktop() {
        let msg = stub_message(5);
        let megolm =
            json!({ "algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "x", "sender_key": "k", "session_id": "s" });
        let enc = json!({ "type": "m.room.encrypted", "event_id": "$edit", "sender": "@me:x", "origin_server_ts": 2,
            "content": megolm, "unsigned": { "com.beeper.hs.order": 9 } });
        let edit = json!({ "type": "m.room.message", "event_id": "$edit", "sender": "@me:x", "origin_server_ts": 2,
            "unsigned": { "com.beeper.hs.order": 9 },
            "content": { "msgtype": "m.text", "body": "* m5!", "m.new_content": { "msgtype": "m.text", "body": "m5!" },
                "m.relates_to": { "rel_type": "m.replace", "event_id": "$e5" } } });
        let mut sync = stub_sync();
        let done = json!({ "type": "com.beeper.inbox.done", "content": { "at_order": 5, "at_ts": 1 } });
        sync["rooms"]["join"]["!room:x"]["account_data"] = json!({ "events": [done] });
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, sync.clone()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, json!({ "start": "t", "chunk": [enc.clone(), msg.clone()] })),
            p if p.contains("/event/") => (200, edit.clone()),
            p if p.contains("/account_data/") || p.contains("/tags/") => (200, json!({})),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("decrypted-edit"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        assert_eq!(store.read().chat("!room:x").is_archived, Some(false), "an undecrypted event may be a message");
        let room = store.read().matrix.clone().unwrap().get_room(matrix_sdk::ruma::room_id!("!room:x")).unwrap();
        super::refetch_undecrypted(&store, vec![room]).await;
        let st = store.read();
        assert_eq!(st.stored_message("$e5").and_then(|m| m.text).as_deref(), Some("m5!"), "the edit applies");
        assert!(st.stored_message("$edit").is_none(), "the placeholder goes");
        assert_eq!(st.chat("!room:x").is_archived, Some(true), "archived up to the edited message again");
    }

    /// Desktop's `loadMessage`: one message by its event, without adding it to the timeline.
    #[tokio::test]
    async fn a_quoted_message_loads_by_event() {
        let store = stub_history("fetch-message", vec![stub_message(7)], ("!other:x", Vec::new())).await;
        let m = super::fetch_message(store.clone(), "!room:x".into(), "$e7".into()).await.expect("the message");
        assert_eq!((m.id.as_str(), m.text.as_deref()), ("$e7", Some("m7")));
        assert!(store.read().loaded("$e7").is_none(), "the timeline does not get it");
    }

    /// Desktop's `mutedRoomIDs`: a room in `com.beeper.muted` is muted, and a mute here goes in or out of that list.
    #[tokio::test]
    async fn mutes_go_through_beeper_muted_like_desktop() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(None));
        let w = written.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/v3/versions" | "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                "/_matrix/client/v3/sync" => {
                    let muted =
                        json!({ "type": "com.beeper.muted", "content": { "room_ids": ["!room:x"], "other": 1 } });
                    let mut sync = stub_sync();
                    sync["account_data"] = json!({ "events": [muted] });
                    (200, sync)
                }
                p if p.ends_with("/messages") => (200, json!({ "start": "t0", "chunk": [] })),
                p if method == "PUT" && p.ends_with("/account_data/com.beeper.muted") => {
                    *w.lock().unwrap() = Some(body);
                    (200, json!({}))
                }
                p if p.contains("/pushrules/") || p.contains("/account_data/") => (200, json!({})),
                _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("beeper-muted"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        assert_eq!(store.read().chat("!room:x").is_muted, Some(true), "the list mutes it");
        {
            let mut st = store.write();
            let idx = st.chat_idx("!room:x");
            st.set_chat_flag(idx, crate::models::ChatFlag::Muted, Some(false));
        }
        super::flush(store.clone()).await.expect("the stub takes the unmute");
        let body = written.lock().unwrap().clone().expect("the list was written");
        assert_eq!(body, json!({ "room_ids": [], "other": 1 }), "the room left the list, the rest stayed");
    }

    async fn sync_stub_room(store: &crate::store::SharedStore, id: &str) {
        let client = store.read().matrix.clone().expect("a Matrix client");
        let room = client.get_room(&matrix_sdk::ruma::OwnedRoomId::try_from(id).expect("a room ID")).expect("the room");
        super::sync_room(store, &room, Vec::new()).await;
    }

    /// Waits until the push task of the sync loop makes `done` true.
    async fn pushed_until(store: &crate::store::SharedStore, done: impl Fn(&crate::store::BeeperStore) -> bool) {
        let wait = async {
            while !done(&store.read()) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), wait).await.expect("the push task ends in time");
    }

    fn stub_event(id: &str, kind: &str, content: Value) -> Value {
        json!({ "type": kind, "sender": "@me:x", "event_id": id, "origin_server_ts": 1_700_000_000_000i64,
            "content": content, "unsigned": { "com.beeper.hs.order": 1 } })
    }

    /// Desktop's optimistic thread update: a flag changed here keeps its value until a sync shows the change, also
    /// when a sync of the room (here the stub, which never has the change) comes first.
    #[tokio::test]
    async fn changed_flags_stay_until_a_sync_shows_them_like_desktop() {
        use super::archived_upto;
        let text = stub_event("$a", "m.room.message", json!({ "msgtype": "m.text", "body": "hi" }));
        let store = stub_history("local-flags", vec![text], ("!other:x", Vec::new())).await;
        sync_stub_room(&store, "!room:x").await;
        let flags = |store: &crate::store::SharedStore| {
            let st = store.read();
            let c = st.chat("!room:x");
            (c.is_archived, c.is_low_priority)
        };
        assert_eq!(flags(&store), (Some(false), Some(false)));
        {
            let mut st = store.write();
            let i = st.chat_idx("!room:x");
            assert!(st.set_chat_flag(i, super::ChatFlag::Archived, Some(true)));
            assert!(st.set_chat_flag(i, super::ChatFlag::LowPriority, Some(true)));
        }
        sync_stub_room(&store, "!other:x").await;
        let pin = |store: &crate::store::SharedStore| store.read().pin_rank.get("!other:x").copied();
        let i = store.read().chat_idx("!other:x");
        assert!(store.write().set_chat_flag(i, super::ChatFlag::Pinned, Some(true)));
        assert_eq!(pin(&store), Some(0), "a new pin goes last in the pin order");
        sync_stub_room(&store, "!other:x").await;
        assert_eq!(pin(&store), Some(0), "the pin keeps its place until favourites_order shows it");
        sync_stub_room(&store, "!room:x").await;
        assert_eq!(flags(&store), (Some(true), Some(true)), "the queued changes win over the room state");
        super::flush(store.clone()).await.expect("the stub takes the account data and the tag");
        assert!(store.read().outbox.is_empty());
        sync_stub_room(&store, "!room:x").await;
        assert_eq!(flags(&store), (Some(true), Some(true)), "the sync in flight may not show the sent changes");
        for _ in 0..2 {
            let mut st = store.write();
            st.syncs += 1;
            st.synced();
        }
        sync_stub_room(&store, "!room:x").await;
        assert_eq!(flags(&store), (Some(false), Some(false)), "later syncs show the homeserver state");
        sync_stub_room(&store, "!other:x").await;
        assert_eq!(pin(&store), None, "the stub's empty favourites_order wins after the sent change");
        let mut c = {
            let st = store.read();
            st.chat("!room:x").clone()
        };
        (c.unread_count, c.is_marked_unread, c.is_pinned, c.last_activity) = (0, None, None, None);
        let (old, new) = (crate::store::sort_key(3), crate::store::sort_key(9));
        assert!(archived_upto(&c, Some((Some(9), None)), &old) && !archived_upto(&c, Some((Some(3), None)), &new));
        assert!(!archived_upto(&c, Some((None, None)), &old) && !archived_upto(&c, None, &old));
        assert!(archived_upto(&c, Some((None, Some(10))), &new), "a time without an order, after the last activity");
        c.unread_count = 1;
        assert!(!archived_upto(&c, Some((None, Some(10))), &new), "an unread chat is not archived up to a time");
        c.is_pinned = Some(true);
        assert!(!archived_upto(&c, Some((Some(9), None)), &old), "a pinned chat is never archived");
    }

    /// Desktop's `editMessage`: a caption edit keeps the attachment, an empty caption leaves its file name, and the
    /// fallback has no `* ` prefix.
    #[test]
    fn edits_keep_the_attachment_like_desktop() {
        let target = matrix_sdk::ruma::EventId::parse("$photo:beeper.com").unwrap();
        let text = super::edit_content(&target, "fixed".into(), &[], None).unwrap();
        assert_eq!(text["msgtype"], "m.text");
        assert_eq!(text["body"], "fixed");
        assert_eq!(text["m.new_content"], json!({ "msgtype": "m.text", "body": "fixed" }));
        assert_eq!(text["m.relates_to"], json!({ "event_id": "$photo:beeper.com", "rel_type": "m.replace" }));
        assert_eq!(text["m.mentions"], json!({}));

        let photo: serde_json::Map<String, Value> = serde_json::from_value(json!({
            "msgtype": "m.image", "url": "mxc://beeper.com/abc", "info": { "w": 4, "h": 3 }, "filename": "cat.jpg",
        }))
        .unwrap();
        let caption = super::edit_content(&target, "a **cat**".into(), &[], Some(photo.clone())).unwrap();
        let new = &caption["m.new_content"];
        assert_eq!(
            (&new["msgtype"], &new["url"], &new["filename"]),
            (&json!("m.image"), &json!("mxc://beeper.com/abc"), &json!("cat.jpg"))
        );
        assert_eq!((&new["body"], &new["formatted_body"]), (&json!("a **cat**"), &json!("a <strong>cat</strong>")));
        assert_eq!(
            (&caption["msgtype"], &caption["url"], &caption["body"]),
            (&json!("m.image"), &json!("mxc://beeper.com/abc"), &json!("a **cat**"))
        );
        assert!(caption.get("formatted_body").is_none(), "Desktop keeps the HTML in the new content");

        let removed = super::edit_content(&target, String::new(), &[], Some(photo)).unwrap();
        assert_eq!(removed["m.new_content"]["body"], "cat.jpg");
        assert_eq!(removed["body"], "cat.jpg");
    }

    #[test]
    fn archive_state_like_desktop() {
        use super::inbox_done;
        use crate::store::{order_of, sort_key};
        assert_eq!(inbox_done(None, 5), json!({}), "Desktop unarchives with an empty object");
        assert_eq!(inbox_done(Some(-3), 5), json!({ "at_ts": 5, "updated_ts": 5, "at_order": -3 }));
        assert!([-5, 0, 7, -9_007_199_254_740_991].iter().all(|&o| order_of(&sort_key(o)) == o));
    }

    /// Desktop's `wi`: an emote is text, and a gallery, a command or a verification request is not shown as text.
    #[tokio::test]
    async fn msgtypes_like_desktop() {
        let event = |id: &str, kind: &str| stub_event(id, "m.room.message", json!({ "msgtype": kind, "body": id }));
        let kinds = [("$t", "m.text"), ("$e", "m.emote"), ("$g", "com.beeper.gallery"), ("$c", "com.beeper.command")];
        let mut events: Vec<Value> = kinds.iter().map(|(id, kind)| event(id, kind)).collect();
        events.push(event("$v", "m.key.verification.request"));
        let store = stub_history("msgtypes", events, ("!other:x", Vec::new())).await;
        sync_stub_room(&store, "!room:x").await;
        let st = store.read();
        let kind = |id: &str| {
            st.messages.iter().find(|m| m.id == id).and_then(|m| m.message_type.clone()).filter(|t| t != "HIDDEN")
        };
        assert_eq!((kind("$t").as_deref(), kind("$e").as_deref()), (Some("TEXT"), Some("TEXT")));
        assert!(["$g", "$c", "$v"].iter().all(|id| kind(id).is_none()), "no text rows for the other msgtypes");
    }

    /// Other clients send events without the fields that Desktop expects, and Desktop shows them as well as it can.
    #[tokio::test]
    async fn bad_events_from_other_clients_like_desktop() {
        let profile = json!({ "displayname": "Ann", "has_fallback": true });
        let text = json!({ "msgtype": "m.text", "body": "Bob: hi", "com.beeper.per_message_profile": profile });
        let mut text = stub_event("$a", "m.room.message", text);
        text["unsigned"] = json!({});
        let image = json!({ "msgtype": "m.image", "body": "a.png", "info": { "w": 1u64 << 40, "h": 10 }, "file": {} });
        let reaction = json!({ "m.relates_to": { "rel_type": "m.annotation", "event_id": "$a" } });
        let events = vec![text, stub_event("$b", "m.room.message", image), stub_event("$c", "m.reaction", reaction)];
        let store = stub_history("bad-events", events, ("!other:x", Vec::new())).await;
        sync_stub_room(&store, "!room:x").await;
        {
            let st = store.read();
            let m = |id: &str| st.message(id);
            assert_eq!(m("$a").text.as_deref(), Some("Bob: hi"), "a body without the fallback stays");
            assert_eq!(m("$a").sort_key, crate::store::sort_key(0), "Desktop's hs.order ?? 0");
            let a = &m("$b").attachments[0];
            assert!(a.src_url.is_none() && a.size.is_none(), "a file without a URL, and a width that does not fit");
            assert_eq!(m("$a").reactions.iter().map(|r| r.reaction_key.as_str()).collect::<Vec<_>>(), [""]);
        }
        let poll = json!({ "org.matrix.msc3381.poll.start": { "question": { "body": "Q" }, "answers": [] } });
        let sticker = json!({ "body": "s", "url": "mxc://x/s", "info": {} });
        let events =
            vec![stub_event("$p", "org.matrix.msc3381.poll.start", poll), stub_event("$s", "m.sticker", sticker)];
        let store = stub_history("forward", events, ("!other:x", Vec::new())).await;
        let forward = |event: &str| super::Outgoing::Forward {
            txn: format!("txn{event}"),
            from: "!room:x".into(),
            event: event.into(),
            text: None,
            html: None,
        };
        store.write().queue("!room:x", forward("$p"));
        let err = super::flush(store.clone()).await.expect_err("a poll has no content to forward");
        assert_eq!(
            (err.message.as_str(), err.transient),
            ("Cannot forward a org.matrix.msc3381.poll.start event", false)
        );
        store.write().queue("!room:x", forward("$s"));
        super::flush(store.clone()).await.expect("a sticker has no msgtype and forwards");
        assert_eq!(store.read().sent.get("txn$s").map(String::as_str), Some("$sent"));
    }

    /// Desktop finds the target of an event only in the room of the event, so a redaction, an edit, a reaction or a
    /// vote in one room does not change a message of an other room.
    #[tokio::test]
    async fn events_change_only_their_own_room_like_desktop() {
        let rel = |kind: &str| json!({ "rel_type": kind, "event_id": "$note" });
        let mut redaction = stub_event("$r", "m.room.redaction", json!({}));
        redaction["redacts"] = json!("$note");
        let new = json!({ "msgtype": "m.text", "body": "spoofed" });
        let edit =
            json!({ "msgtype": "m.text", "body": "* spoofed", "m.new_content": new, "m.relates_to": rel("m.replace") });
        let mut reaction = rel("m.annotation");
        reaction["key"] = json!("👍");
        let vote =
            json!({ "m.relates_to": rel("m.reference"), "org.matrix.msc3381.poll.response": { "answers": ["1"] } });
        let events = vec![
            redaction,
            stub_event("$e", "m.room.message", edit),
            stub_event("$k", "m.reaction", json!({ "m.relates_to": reaction })),
            stub_event("$v", "org.matrix.msc3381.poll.response", vote),
        ];
        let answer = json!({ "id": "1", "org.matrix.msc1767.text": "A" });
        let poll = json!({ "org.matrix.msc3381.poll.start": { "question": { "body": "Q" }, "answers": [answer] } });
        let other = vec![stub_event("$note", "org.matrix.msc3381.poll.start", poll)];
        let store = stub_history("own-room", events, ("!other:x", other)).await;
        sync_stub_room(&store, "!other:x").await;
        sync_stub_room(&store, "!room:x").await;
        let st = store.read();
        let m = st.message("$note");
        assert!(m.is_deleted.is_none() && m.text.as_deref() == Some("Q") && m.reactions.is_empty());
        assert_eq!(m.poll.as_ref().map(|p| p.voters()), Some(0));
    }

    /// Desktop's view-limited media: the limit of `com.beeper.view_limited`, the views left that only a bridge bot
    /// reports, and the bridge's redaction after the last view, which keeps the message as expired media of its kind.
    #[tokio::test]
    async fn view_once_media_like_desktop() {
        let image = |id: &str, limit: Value| {
            let content = json!({ "msgtype": "m.image", "body": "a.jpg", "url": "mxc://x/a",
                "info": { "mimetype": "image/jpeg" }, "com.beeper.view_limited": limit });
            let mut e = stub_event(id, "m.room.message", content);
            e["sender"] = json!("@vic:x");
            e
        };
        let left = |id: &str, sender: &str, target: &str, n: u64| {
            let rel = json!({ "rel_type": "m.reference", "event_id": target });
            let mut e = stub_event(id, "com.beeper.view_limited", json!({ "m.relates_to": rel, "remaining_count": n }));
            e["sender"] = json!(sender);
            e
        };
        let mut redaction = stub_event("$r", "m.room.redaction", json!({ "com.beeper.view_limited": true }));
        redaction["redacts"] = json!("$gone");
        let events = vec![
            image("$twice", json!({ "type": "count", "count": 2 })),
            left("$u1", "@whatsappbot:beeper.local", "$twice", 1),
            left("$u2", "@vic:x", "$twice", 0),
            image("$timed", json!({ "type": "count", "count": 1, "time": 5000 })),
            left("$u3", "@vic:x", "$timed", 1),
            image("$gone", json!({ "type": "count", "count": 1 })),
            redaction,
        ];
        let store = stub_history("view-once", events, ("!other:x", Vec::new())).await;
        sync_stub_room(&store, "!room:x").await;
        let st = store.read();
        let twice = st.message("$twice");
        assert!(twice.view_once() && twice.view_once_replay() && twice.view_once_time().is_none());
        assert_eq!(twice.views_left, Some(1), "the bridge bot's count, not the user's");
        assert_eq!(twice.view_once_limit(), Some(json!({ "type": "count", "count": 1 })), "Desktop's nZt");
        let timed = st.message("$timed");
        assert_eq!((timed.view_once_time(), timed.views_left, timed.view_once_replay()), (Some(5000), None, false));
        let gone = st.message("$gone");
        assert!(gone.view_once_expired() && !gone.view_once() && gone.attachments.is_empty());
        assert_eq!(gone.view_once_kind(), Some("img"), "the expired message keeps the kind of its media");
        let preview = st.chat("!room:x").preview.clone().expect("a preview");
        assert_eq!(preview.id, "$gone", "Desktop's `Pct`: expired media still previews its chat");
    }

    /// The SDK's `ViewLimitedMedia`: a provisioning request through the homeserver to the bridge, with the limit of the
    /// view.
    #[tokio::test]
    async fn a_view_goes_to_the_bridge_like_desktop() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let url = crate::bridge_stub::serve(move |method, path, body| {
            s.lock().unwrap().push((method.to_string(), path.to_string(), body));
            (200, json!({}))
        })
        .await;
        let p = super::Provision {
            homeserver: url,
            access_token: "token".into(),
            user_id: "@me:beeper.com".into(),
            bridge_id: "whatsapp".into(),
            login_id: "15550100".into(),
        };
        let limit = json!({ "type": "count", "count": 1, "time": 5000 });
        super::view_limited_media(p, "$a:b".into(), limit.clone()).await.expect("the bridge takes the view");
        let path = "/_matrix/client/unstable/com.beeper.bridge/whatsapp/_matrix/provision/v3/view_limited_media/%24a%3Ab?user_id=%40me%3Abeeper.com&login_id=15550100";
        assert_eq!(*seen.lock().unwrap(), [("POST".into(), path.into(), json!({ "com.beeper.view_limited": limit }))]);
    }

    /// Desktop's threaded replies: the `m.thread` relations of a room with threads, the thread follows, the own
    /// thread receipt, and the thread replies and receipts that Desktop sends.
    #[tokio::test]
    async fn threads_like_desktop() {
        let rel = |falling_back: bool, to: &str| json!({ "rel_type": "m.thread", "event_id": "$e1", "is_falling_back": falling_back, "m.in_reply_to": { "event_id": to } });
        let reply = |i: i64, rel: Value, broadcast: bool| {
            let mut e = stub_message(i);
            (e["sender"], e["content"]["m.relates_to"]) = (json!("@ann:x"), rel);
            if broadcast {
                e["content"]["com.beeper.broadcast_outside_thread"] = json!(true);
            }
            e
        };
        let events = [
            stub_message(1),
            reply(2, rel(true, "$e1"), false),
            reply(3, rel(false, "$e2"), true),
            stub_message(4),
            reply(5, rel(true, "$e3"), false),
        ];
        let mut sync = stub_sync();
        let room = &mut sync["rooms"]["join"]["!room:x"];
        let features = json!({ "type": "com.beeper.room_features", "state_key": "", "sender": "@me:x",
            "event_id": "$features", "origin_server_ts": 1, "content": { "thread": 1 } });
        room["state"]["events"].as_array_mut().unwrap().push(features);
        let follows =
            json!({ "follow_all": false, "followed": [], "unfollowed": ["$e1"], "com.beeper.revision_id": "r" });
        room["account_data"] = json!({ "events": [{ "type": "com.beeper.thread_follows", "content": follows }] });
        let receipt = json!({ "$e2": { "m.read": { "@me:x": { "ts": 1, "thread_id": "$e1" } } } });
        room["ephemeral"] = json!({ "events": [{ "type": "m.receipt", "content": receipt }] });
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                p if p.ends_with("/messages") => {
                    (200, json!({ "start": "t", "chunk": events.iter().rev().collect::<Vec<_>>() }))
                }
                p if p.contains("/event/") => {
                    (200, events.iter().find(|e| p.ends_with(e["event_id"].as_str().unwrap())).unwrap().clone())
                }
                "/_matrix/client/v1/media/config" => (200, json!({ "m.upload.size": 1_000_000 })),
                "/_matrix/media/v3/upload" => (200, json!({ "content_uri": "mxc://x/f" })),
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p => {
                    log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("threads"), &url).await;
        store.write().features = crate::features::Flags::free_plan();
        sync_stub_room(&store, "!room:x").await;
        let chat = || {
            let st = store.read();
            st.chat("!room:x").clone()
        };
        {
            let c = chat();
            let st = store.read();
            assert!(c.threads && st.threaded(&c), "the thread room feature and the threads flag");
            let m = |id: &str| st.message(id).clone();
            assert_eq!(
                (m("$e2").thread_root.as_deref(), m("$e2").fallback, m("$e2").broadcast),
                (Some("$e1"), true, false)
            );
            assert_eq!(
                (m("$e3").fallback, m("$e3").broadcast, m("$e3").linked_message_id),
                (false, true, Some("$e2".into()))
            );
            assert_eq!(m("$e4").thread_root, None);
            let replies: Vec<&str> = st.thread_replies("$e1").map(|m| m.id.as_str()).collect();
            assert_eq!(replies, ["$e2", "$e3", "$e5"]);
            assert_eq!(
                c.preview.as_ref().map(|p| p.id.as_str()),
                Some("$e4"),
                "a reply only in the thread does not preview"
            );
            assert_eq!(c.follows.unfollowed, ["$e1"]);
            assert!(!st.follows_thread(&c, "$e1"), "an explicit unfollow beats the own root");
            let joined = crate::models::Chat { follows: Default::default(), ..c.clone() };
            assert!(st.follows_thread(&joined, "$e1"), "the user follows a thread with the own root");
            assert_eq!(c.thread_reads["$e1"], crate::store::sort_key(2), "the own thread receipt");
            assert_eq!(st.thread_unread(&c, "$e1"), (2, 0));
        }
        store.write().mark_thread_read("!room:x", "$e1", None);
        let c = chat();
        assert_eq!(store.read().thread_unread(&c, "$e1"), (0, 0));
        let t = store.read().thread_send("$e1", None, true);
        assert_eq!(t, super::ThreadSend { root: "$e1".into(), quoted: "$e5".into(), chosen: false, broadcast: true });
        let out = super::Outgoing::Send {
            txn: "txn".into(),
            text: "hi".into(),
            reply_to: Some(t.quoted.clone()),
            preview: None,
            mentions: Vec::new(),
            thread: Some(t.clone()),
            links: true,
        };
        store.write().queue("!room:x", out);
        let (bytes, name, mime) = (b"%PDF".to_vec(), "a.pdf".to_string(), "application/pdf".to_string());
        let caption = None;
        let file = super::Outgoing::Media {
            txn: "file".into(),
            bytes,
            name,
            mime,
            info: super::MediaInfo::File,
            caption,
            reply_to: Some(t.quoted.clone()),
            thread: Some(t),
            gif: false,
            sticker: None,
        };
        store.write().queue("!room:x", file);
        let gif = super::Outgoing::Media {
            txn: "gif".into(),
            bytes: vec![0; 9],
            name: "g.mp4".into(),
            mime: "video/mp4".into(),
            info: super::MediaInfo::Video(3, 2, 1500),
            caption: Some("hi".into()),
            reply_to: None,
            thread: None,
            gif: true,
            sticker: None,
        };
        store.write().queue("!room:x", gif);
        store.write().follow_thread("!room:x", "$e1", true);
        super::flush(store.clone()).await.expect("send the thread receipt, the reply and the follow");
        let sent = sent.lock().unwrap();
        let (receipt, body) = sent.iter().find(|(p, _)| p.contains("/receipt/")).expect("a thread receipt");
        assert!(receipt.ends_with("/receipt/m.read/$e5"), "{receipt}");
        assert_eq!(body["thread_id"], "$e1");
        let (_, body) = sent.iter().find(|(p, _)| p.contains("/send/m.room.message/")).expect("a thread reply");
        let rel = json!({ "rel_type": "m.thread", "event_id": "$e1", "is_falling_back": true, "m.in_reply_to": { "event_id": "$e5" } });
        assert_eq!((&body["m.relates_to"], &body["com.beeper.broadcast_outside_thread"]), (&rel, &json!(true)));
        assert_eq!(body["m.mentions"], json!({}), "Desktop does not mention the sender of the fallback reply");
        assert_eq!(body["com.beeper.linkpreviews"], json!([]), "links without a preview get none from the bridge");
        let (_, body) =
            sent.iter().find(|(p, b)| p.contains("/send/m.room.message/") && b["msgtype"] == "m.file").expect("a file");
        assert_eq!(body["com.beeper.broadcast_outside_thread"], json!(true), "a file has \"Also send to chat\" too");
        let (_, body) =
            sent.iter().find(|(p, b)| p.contains("/send/m.room.message/") && b["msgtype"] == "m.video").unwrap();
        let info = json!({ "mimetype": "video/mp4", "size": 9, "w": 3, "h": 2, "duration": 1500, "fi.mau.gif": true });
        assert_eq!((&body["info"], &body["body"], &body["filename"]), (&info, &json!("hi"), &json!("g.mp4")));
        let (_, body) =
            sent.iter().find(|(p, _)| p.ends_with("/account_data/com.beeper.thread_follows")).expect("a follow");
        assert_eq!(
            (&body["followed"], &body["unfollowed"], &body["follow_all"]),
            (&json!(["$e1"]), &json!([]), &json!(false))
        );
        assert!(body["com.beeper.revision_id"].is_string(), "Desktop writes a new revision");
    }

    /// Desktop's sticker packs: the packs of the room, its space, the rooms of `m.image_pack.rooms` and the account,
    /// the sticker images of each pack, and the `m.sticker` event of a sticker.
    #[tokio::test]
    async fn sticker_packs_like_desktop() {
        let state = |kind: &str, key: &str, content: Value| {
            json!({ "type": kind, "state_key": key, "sender": "@me:x", "event_id": format!("${kind}{key}"),
                "origin_server_ts": 1, "content": content })
        };
        let img = |url: &str, usage: Value| {
            let info = json!({ "mimetype": "image/png", "w": 2, "h": 2, "thumbnail_url": "mxc://x/t" });
            json!({ "url": url, "usage": usage, "body": "b", "info": info })
        };
        let pack = |code: &str, url: &str| json!({ "images": { code: img(url, Value::Null) } });
        let joined = |events: Vec<Value>| json!({ "state": { "events": events }, "timeline": { "events": [] } });
        let member = state("m.room.member", "@me:x", json!({ "membership": "join" }));
        let mut sync = stub_sync();
        let png_only = json!({ "mime_types": { "image/png": 2 } });
        let features = json!({ "file": { "m.sticker": png_only, "m.image": png_only } });
        let cats = json!({ "pack": { "display_name": "Cats" }, "images": { "cat": img("mxc://x/cat", Value::Null),
            "kitty": img("mxc://x/cat", Value::Null), "smile": img("mxc://x/e", json!(["emoticon"])) } });
        let dogs = json!({ "pack": { "usage": ["sticker"] }, "images": { "dog": img("mxc://x/dog", Value::Null) } });
        let room = sync["rooms"]["join"]["!room:x"]["state"]["events"].as_array_mut().unwrap();
        room.extend([
            state("com.beeper.room_features", "", features),
            state("m.room.image_pack", "a", cats),
            state("im.ponies.room_emotes", "a", pack("old", "mxc://x/old")),
            state("im.ponies.room_emotes", "b", dogs),
            state("m.room.image_pack", "gone", json!({})),
        ]);
        let space = json!({ "creator": "@me:x", "room_version": "10", "type": "m.space" });
        sync["rooms"]["join"]["!space:x"] = joined(vec![
            state("m.room.create", "", space),
            member.clone(),
            state("m.space.child", "!room:x", json!({ "via": ["x"] })),
            state("m.room.image_pack", "", pack("fox", "mxc://x/fox")),
        ]);
        let create = state("m.room.create", "", json!({ "creator": "@me:x", "room_version": "10" }));
        sync["rooms"]["join"]["!other:x"] = joined(vec![
            create,
            member,
            state("m.room.image_pack", "g", pack("owl", "mxc://x/owl")),
            state("m.room.image_pack", "h", pack("bat", "mxc://x/bat")),
        ]);
        sync["account_data"] = json!({ "events": [
            { "type": "m.image_pack", "content": pack("own", "mxc://x/own") },
            { "type": "m.image_pack.rooms", "content": { "rooms": { "!other:x": { "g": {} } } } },
        ] });
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                "/_matrix/client/v1/media/config" => (200, json!({ "m.upload.size": 1_000_000 })),
                "/_matrix/media/v3/upload" => (200, json!({ "content_uri": "mxc://x/f" })),
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                p if p.ends_with("/messages") => (200, json!({ "start": "t", "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p => {
                    log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("stickers"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        let packs = super::sticker_packs(store.clone(), "!room:x".into()).await.expect("the packs");
        let ids: Vec<(&str, &str)> = packs.iter().map(|p| (p.id.as_str(), p.title.as_str())).collect();
        let want = [
            ("m.room.image_pack|!room:x|a", "Cats"),
            ("im.ponies.room_emotes|!room:x|b", "b"),
            ("m.room.image_pack|!space:x|", ""),
            ("m.room.image_pack|!other:x|g", "g"),
            ("account", ""),
        ];
        assert_eq!(ids, want, "the room, its space, the global rooms and the account, and the stable type wins");
        let cat = packs[0].stickers.clone();
        assert_eq!(cat.len(), 1, "one sticker for each URL, and no emoticons");
        assert_eq!(
            (cat[0].shortcodes.clone(), packs[4].stickers[0].source.clone()),
            (vec!["cat".into(), "kitty".into()], None)
        );
        let mut png = Vec::new();
        ::image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).unwrap();
        let sticker = super::Outgoing::Media {
            txn: "sticker".into(),
            bytes: png.clone(),
            name: "cat.png".into(),
            mime: "image/png".into(),
            info: super::MediaInfo::Image(2, 2),
            caption: None,
            reply_to: None,
            thread: None,
            gif: false,
            sticker: Some(Box::new(cat[0].clone())),
        };
        store.write().queue("!room:x", sticker);
        super::flush(store.clone()).await.expect("send the sticker");
        let sticker = sent.lock().unwrap().iter().find(|(p, _)| p.contains("/send/m.sticker/")).cloned();
        let (_, body) = sticker.expect("an m.sticker event");
        let info = json!({ "mimetype": "image/png", "w": 2, "h": 2, "size": png.len(), "thumbnail_url": "mxc://x/t" });
        let source = json!({ "mxc://x/f": { "room_id": "!room:x", "state_key": "a", "shortcode": "cat" } });
        assert_eq!(
            (&body["body"], &body["info"], &body["url"], &body["com.beeper.msc4459.image_source_packs"]),
            (&json!("b"), &info, &json!("mxc://x/f"), &source)
        );
        assert!(body.get("msgtype").is_none() && body.get("filename").is_none());
        // The local echo of a converted file takes the name and the type of the conversion.
        let mut jpg = Vec::new();
        ::image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut jpg), ::image::ImageFormat::Jpeg).unwrap();
        let att = crate::models::Attachment {
            file_name: Some("a.jpg".into()),
            mime_type: Some("image/jpeg".into()),
            ..Default::default()
        };
        let (_, txn, _) = store.write().send_message("!room:x", None, None, Some(att)).expect("a local echo");
        let file = super::Outgoing::Media {
            txn: txn.clone(),
            bytes: jpg,
            name: "a.jpg".into(),
            mime: "image/jpeg".into(),
            info: super::MediaInfo::Image(2, 2),
            caption: None,
            reply_to: None,
            thread: None,
            gif: false,
            sticker: None,
        };
        store.write().queue("!room:x", file);
        super::flush(store.clone()).await.expect("send the image");
        let echo = store.read().stored_message(&txn).expect("the echo");
        let a = &echo.attachments[0];
        assert_eq!((a.file_name.as_deref(), a.mime_type.as_deref()), (Some("a.png"), Some("image/png")));
    }

    /// Desktop's blocking: the `block_user`, `no_unblock` and `report_spam` room features, the
    /// `com.beeper.user_blocked` of the bridge channel, and the `com.beeper.block_user` event that the SDK sends.
    #[tokio::test]
    async fn blocking_follows_the_bridge_like_desktop() {
        use crate::models::{BLOCK_USER, NO_UNBLOCK, REPORT_SPAM, USER_BLOCKED};
        let mut sync = stub_sync();
        let state = |kind: &str, content: Value| {
            json!({ "type": kind, "state_key": "", "sender": "@me:x", "event_id": format!("${kind}"),
                "origin_server_ts": 1, "content": content })
        };
        let features = state("com.beeper.room_features", json!({ "block_user": true, "report_spam": true }));
        let bridge = json!({ "bridgebot": "@bot:x", "channel": { "id": "c", "com.beeper.user_blocked": true },
            "protocol": { "id": "telegram" }, "com.beeper.room_type.v2": "dm" });
        let events = sync["rooms"]["join"]["!room:x"]["state"]["events"].as_array_mut().unwrap();
        events.extend([features, state("m.bridge", bridge)]);
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                p if p.ends_with("/messages") => (200, json!({ "start": "t", "chunk": [] })),
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p => {
                    log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("block"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        let chat = || {
            let st = store.read();
            st.chat("!room:x").clone()
        };
        let c = chat();
        assert_eq!(c.block, BLOCK_USER | REPORT_SPAM | USER_BLOCKED);
        assert!(!store.read().can_block(&c), "the block-users flag is off");
        store.write().features.flags = json!({ "features": { "*": { "block-users": true } } });
        assert!(store.read().user_blocked(&c));
        store.write().set_user_blocked("!room:x", false, false);
        let c = chat();
        assert!(!store.read().user_blocked(&c) && c.block & NO_UNBLOCK == 0);
        store.write().set_user_blocked("!room:x", true, true);
        super::flush(store.clone()).await.expect("send the block events");
        let sent = sent.lock().unwrap();
        let bodies: Vec<&Value> =
            sent.iter().filter(|(p, _)| p.contains("/send/com.beeper.block_user/")).map(|(_, b)| b).collect();
        let want = [json!({ "block": false, "report_spam": false }), json!({ "block": true, "report_spam": true })];
        assert_eq!(bodies, [&want[0], &want[1]]);
    }

    /// Desktop keeps each cloud login of a bridge as its own account, and puts a room on the login of its receiver.
    #[test]
    fn several_logins_of_a_bridge_like_desktop() {
        let on = json!({ "state_event": "CONNECTED" });
        let bridges = json!({
            "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "a": on, "b": on } },
            "telegram": { "bridgeState": { "bridgeType": "telegram" }, "remoteState": { "t": on } },
        });
        let accounts = crate::cloud::cloud_accounts(&bridges).expect("two logins of one bridge are valid");
        let mut ids: Vec<&str> = accounts.iter().map(|a| a.account_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["telegram", "whatsapp.a", "whatsapp.b"]);
        let room = |bridge: &str, receiver: Option<&str>| {
            let mut v = json!({ "com.beeper.bridge_name": bridge });
            if let Some(r) = receiver {
                v["channel"] = json!({ "fi.mau.receiver": r });
            }
            super::account_id(&v, &accounts)
        };
        assert_eq!(room("whatsapp", Some("b")), "whatsapp.b");
        assert_eq!(room("whatsapp", None), "whatsapp", "Desktop does not guess between two logins");
        assert_eq!(room("telegram", None), "telegram");
        assert_eq!(room("telegram", Some("t")), "telegram");
    }

    /// Desktop's `BRIDGE_STATES_UPDATED`: a `com.beeper.bridge_state` change in a sync replaces the cloud accounts.
    /// New feature flags in a sync draw the windows again too.
    #[tokio::test]
    async fn synced_bridge_states_and_flags_update_the_windows() {
        let mut sync = stub_sync();
        let bad = json!({ "state_event": "BAD_CREDENTIALS", "has_error": true, "message": "Logged out" });
        let bridges = json!({ "signal": { "bridgeState": { "bridgeType": "signal" }, "remoteState": { "s": bad } } });
        let event = json!({ "type": "com.beeper.bridge_state", "content": { "bridges": bridges } });
        let flags = json!({ "features": { "*": { "enable-polls": true } } });
        let flags = json!({ "type": crate::features::FLAGS, "content": flags });
        sync["account_data"] = json!({ "events": [event, flags] });
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, sync.clone()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            _ => (404, json!({ "errcode": "M_UNRECOGNIZED", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("bridge-state"), &url).await;
        let mut events = store.read().ws_tx.subscribe();
        let client = store.read().matrix.clone().expect("a client");
        super::sync_account_data(&client, &store).await;
        let accounts = store.read().bridge_accounts.clone();
        let accounts: Vec<_> = accounts.iter().map(|a| (a.account_id.as_str(), a.remote_error.as_deref())).collect();
        assert_eq!(accounts, [("signal", Some("Logged out"))]);
        assert!(store.read().feature("enable-polls").on, "the synced flags");
        for change in ["flags", "bridge states"] {
            assert_eq!(events.try_recv().expect(change).event_type, "setup.updated", "{change}");
        }
        super::sync_account_data(&client, &store).await;
        assert!(events.try_recv().is_err(), "the same states change nothing");
    }

    /// Desktop SDK's account nicknames: `com.beeper.bridge_account_overrides` is encrypted with the MSC4483 key of
    /// secret storage, which the stored recovery key opens. The synced nickname is the account's `customName`, and
    /// `setBridgeAccountName` writes a new revision that expects the read one. When another device wrote first, the
    /// homeserver answers with its content, and the name goes on that.
    #[tokio::test]
    async fn account_nicknames_are_encrypted_account_data_like_desktop() {
        use matrix_sdk::ruma::events::secret::request::SecretName;
        let ssss = matrix_sdk_crypto::secret_storage::SecretStorageKey::new();
        let key = [7; 32];
        let name = SecretName::from(crate::msc4483::KEY_SECRET);
        let secret = ssss.encrypt(vodozemac::base64_encode(key).into_bytes(), &name);
        let secret = json!({ "encrypted": { ssss.key_id(): secret } });
        let default_key = json!({ "key": ssss.key_id() });
        let key_info = serde_json::to_value(ssss.event_content()).expect("the key info JSON");
        let key_path = format!("/_matrix/client/v3/user/@me:x/account_data/m.secret_storage.key.{}", ssss.key_id());
        let kind = super::ACCOUNT_OVERRIDES;
        let synced = json!({ "accounts": { "whatsapp": { "a": { "name": "Work" } } }, "com.beeper.revision_id": "r1" });
        let theirs =
            json!({ "accounts": { "telegram": { "t": { "name": "Phone" } } }, "com.beeper.revision_id": "r2" });
        let mut sync = stub_sync();
        let event = json!({ "type": kind, "content": crate::msc4483::encrypt(&key, kind, &synced) });
        sync["account_data"] = json!({ "events": [event] });
        let conflict = json!({ "errcode": "COM.BEEPER.REVISION_ID_MISMATCH", "error": "revision mismatch",
            "com.beeper.current_content": crate::msc4483::encrypt(&key, kind, &theirs) });
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |method, path, body| {
            match (method, path.split_once('?').map_or(path, |p| p.0)) {
                (_, "/_matrix/client/versions") => (200, json!({ "versions": ["v1.11"] })),
                (_, "/_matrix/client/v3/sync") => (200, sync.clone()),
                (_, "/_matrix/client/v3/keys/upload") => (200, json!({ "one_time_key_counts": {} })),
                ("GET", "/_matrix/client/v3/user/@me:x/account_data/m.secret_storage.default_key") => {
                    (200, default_key.clone())
                }
                ("GET", p) if p == key_path => (200, key_info.clone()),
                ("GET", "/_matrix/client/v3/user/@me:x/account_data/dev.zirco.msc4483.account_data.key") => {
                    (200, secret.clone())
                }
                ("PUT", "/_matrix/client/v3/user/@me:x/account_data/com.beeper.bridge_account_overrides") => {
                    let mut log = log.lock().unwrap();
                    log.push((path.to_string(), body));
                    match log.len() {
                        1 => (409, conflict.clone()),
                        _ => (200, json!({})),
                    }
                }
                _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
            }
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("account-nicknames"), &url).await;
        let on = json!({ "state_event": "CONNECTED" });
        let bridges =
            json!({ "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "a": on } } });
        let accounts = crate::cloud::cloud_accounts(&bridges).expect("valid bridge states");
        store.write().set_cloud_accounts(accounts);
        store.write().beeper_state.secrets.recovery_code = ssss.to_base58();
        let client = store.read().matrix.clone().expect("a client");
        super::sync_account_data(&client, &store).await;
        assert_eq!(store.read().custom_name("whatsapp"), Some("Work"), "the synced nickname is decrypted");
        let set =
            |name: &str| super::set_bridge_account_name(store.clone(), "whatsapp".into(), "a".into(), name.into());
        set("  Personal ").await.expect("the second try writes the name");
        let puts = sent.lock().unwrap().clone();
        let expected: Vec<&str> = puts.iter().map(|(p, _)| p.split_once('?').expect("a query").1).collect();
        assert_eq!(expected, ["com.beeper.expect_revision_id=r1", "com.beeper.expect_revision_id=r2"]);
        let body = &puts[1].1;
        let fields: Vec<&String> = body.as_object().expect("an object").keys().collect();
        assert_eq!(fields, ["com.beeper.revision_id", "encrypted"], "only the revision stays in plaintext");
        let revision = body["com.beeper.revision_id"].as_str().expect("a new revision");
        assert!(!["r1", "r2"].contains(&revision));
        let names = json!({ "telegram": { "t": { "name": "Phone" } }, "whatsapp": { "a": { "name": "Personal" } } });
        let plain = crate::msc4483::decrypt(&key, kind, body).expect("the body decrypts with the key");
        assert_eq!(plain, json!({ "accounts": names, "com.beeper.revision_id": revision }));
        assert_eq!(store.read().custom_name("whatsapp"), Some("Personal"));
        set("").await.expect("the removal writes");
        let body = sent.lock().unwrap()[2].1.clone();
        let plain = crate::msc4483::decrypt(&key, kind, &body).expect("the body decrypts with the key");
        assert_eq!(plain["accounts"], json!({ "telegram": { "t": { "name": "Phone" } } }), "an empty name removes it");
        assert_eq!(store.read().custom_name("whatsapp"), None);
    }

    /// Desktop's SDK reports `initialized` after it reads the encryption state, and an account without cross-signing
    /// then needs its setup.
    #[tokio::test]
    async fn the_read_e2ee_state_is_initialized_like_desktop() {
        let url = crate::bridge_stub::serve(|_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/keys/query" => (200, json!({ "device_keys": { "@me:x": {} } })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("e2ee-state"), &url).await;
        assert_eq!(store.read().app_state().state, "initializing");
        let client = store.read().matrix.clone().expect("a client");
        super::refresh_e2ee(&client, &store).await;
        let state = store.read().app_state();
        assert_eq!(
            (state.state, state.e2ee.initialized, state.e2ee.cross_signing),
            ("needs-cross-signing-setup", true, false)
        );
        let e2ee =
            |store: &crate::store::SharedStore| serde_json::to_value(store.read().app_state()).unwrap()["e2ee"].clone();
        assert!(e2ee(&store).get("recoveryKeyGeneratedAt").is_none(), "Desktop leaves out a time of 0");
        store.write().beeper_state.recovery_code_generated_at = 1_763_200_000_000;
        assert_eq!(e2ee(&store)["recoveryKeyGeneratedAt"], 1_763_200_000_000_i64);
    }

    /// Like Desktop's SDK, the user-interactive auth of the cross-signing keys sends the Beeper JWT, and without a
    /// JWT the user must sign in again.
    #[tokio::test]
    async fn cross_signing_auth_sends_the_beeper_jwt_like_desktop() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = sent.clone();
        let url = crate::bridge_stub::serve(move |_, path, body| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/keys/query" => (200, json!({ "device_keys": { "@me:x": {} } })),
            "/_matrix/client/v3/keys/device_signing/upload" if body["auth"].is_null() => {
                (401, json!({ "session": "s1", "flows": [{ "stages": ["org.matrix.login.jwt"] }], "params": {} }))
            }
            "/_matrix/client/v3/keys/device_signing/upload" => {
                seen.lock().unwrap().push(body["auth"].clone());
                (403, json!({ "errcode": "M_FORBIDDEN", "error": "JWT validation failed" }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("jwt-auth"), &url).await;
        assert_eq!(super::new_recovery_key(&store).await, Err(super::REAUTH.into()), "no JWT");
        assert!(sent.lock().unwrap().is_empty());
        store.write().ui_auth_token = Some("old.jwt".into());
        assert_eq!(super::new_recovery_key(&store).await, Err(super::REAUTH.into()), "a refused JWT");
        let auth = json!({ "type": "org.matrix.login.jwt", "session": "s1", "token": "old.jwt", "user": "@me:x" });
        assert_eq!(*sent.lock().unwrap(), [auth]);
    }

    /// Desktop's `fetchContacts` and `ihe`: every account gives its contact list, a bridge without a list gives none,
    /// and a contact needs a phone number or a Matrix user.
    #[tokio::test(flavor = "multi_thread")]
    async fn contacts_like_desktop() {
        let url = crate::bridge_stub::serve(|_, path, _| match path {
            p if p.contains("/whatsapp/") => (200, json!({ "contacts": null })),
            p if p.contains("/discordgo/") => (
                200,
                json!({ "contacts": [
                    { "id": "2", "name": "Ann", "mxid": "@discord_2:beeper.local" },
                    { "id": "3", "name": "Nobody", "identifiers": [] },
                ] }),
            ),
            _ => (200, json!({ "contacts": [{ "id": "1", "name": "Sam", "identifiers": ["tel:15550100"] }] })),
        })
        .await;
        let on = json!({ "state_event": "CONNECTED" });
        let bridges = json!({
            "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "a": on } },
            "telegram": { "bridgeState": { "bridgeType": "telegram" }, "remoteState": { "t": on } },
            "discordgo": { "bridgeState": { "bridgeType": "discordgo" }, "remoteState": { "d": on } },
        });
        let accounts = crate::cloud::cloud_accounts(&bridges).expect("valid logins");
        let cloud = crate::cloud::Cloud::new(&url, &url);
        let users = tokio::task::spawn_blocking(move || cloud.contacts("token", "@me:x", &accounts)).await.unwrap();
        let users: Vec<_> =
            users.iter().map(|u| (u.full_name.as_deref(), u.phone_number.as_deref(), u.mxid.as_deref())).collect();
        assert_eq!(
            users,
            [(Some("Ann"), None, Some("@discord_2:beeper.local")), (Some("Sam"), Some("+15550100"), None)]
        );
        assert_eq!(crate::cloud::contact_account("contact:discordgo_d:2"), Some("discordgo_d"));
    }

    #[tokio::test]
    async fn failed_sends_stay_in_the_outbox_for_retry_like_desktop() {
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        let fail = std::sync::Arc::new(AtomicBool::new(true));
        let failing = fail.clone();
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split('?').next().expect("a path") {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, json!({ "start": "t0", "chunk": [] })),
            p if p.contains("/send/m.room.message/") && failing.load(Relaxed) => {
                (403, json!({ "errcode": "M_FORBIDDEN", "error": "stub refusal" }))
            }
            p if p.contains("/send/m.room.message/") => (200, json!({ "event_id": "$sent" })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let dir = crate::testing::temp_dir("outbox");
        let store = stub_store(&dir, &url).await;
        sync_stub_room(&store, "!room:x").await;
        let pushes = tokio::spawn(super::push_loop(store.clone()));
        let draft = crate::store::Draft { text: Some("hi".into()), ..Default::default() };
        let txn = store.write().send("!room:x", draft).expect("a local echo").1.remove(0);
        let error = |st: &crate::store::BeeperStore| {
            (st.stored_message(&txn).expect("the local echo").send_error, st.outbox.len())
        };
        pushed_until(&store, |st| error(st).0.is_some()).await;
        super::flush(store.clone()).await.expect("a failed message is not a flush error");
        let (e, queued) = error(&store.read());
        let e = e.expect("the local echo keeps the error");
        assert!(e.message.as_deref().is_some_and(|m| m.contains("stub refusal")) && !e.retrying, "{e:?}");
        assert_eq!(queued, 1, "the send waits in the outbox for Retry");
        pushes.abort();
        assert!(pushes.await.is_err_and(|e| e.is_cancelled()));
        drop(store);
        let store = stub_store(&dir, &url).await;
        tokio::spawn(super::push_loop(store.clone()));
        assert_eq!(error(&store.read()).0.map(|e| e.retrying), Some(false), "the error and the outbox persist");
        fail.store(false, Relaxed);
        store.write().retry_send(&txn);
        pushed_until(&store, |st| error(st) == (None, 0)).await;
        let cfg = crate::config::CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23392, None);
        assert!(crate::store::BeeperStore::from_config(cfg).outbox.is_empty(), "the outbox row is gone");
    }

    /// The first sync of the loop does not wait for a long poll, and its end tells the windows that "Syncing..." is
    /// over.
    #[tokio::test]
    async fn the_first_sync_ends_syncing_and_tells_the_windows() {
        let url = crate::bridge_stub::serve(|_, path, _| match path.split('?').next().expect("a path") {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("first-sync"), &url).await;
        let mut events = store.read().ws_tx.subscribe();
        store.write().syncing = true;
        let client = store.read().matrix.clone().expect("a Matrix client");
        let now = super::SyncSettings::default().timeout(std::time::Duration::ZERO);
        super::on_sync(&client, &store, client.sync_once(now).await).await;
        assert!(!store.read().syncing);
        let kinds: Vec<String> = std::iter::from_fn(|| events.try_recv().ok()).map(|e| e.event_type).collect();
        assert!(kinds.iter().any(|k| k == "setup.updated"), "{kinds:?}");
    }

    /// Desktop's `createThreadFromInvite`: an invite is a read-only group request with Accept (a join) and Decline.
    #[tokio::test]
    async fn invites_are_requests_like_desktop() {
        let joins = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = joins.clone();
        let url = crate::bridge_stub::serve(move |method, path, _| match path.split('?').next().expect("a path") {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => {
                let ev = |kind: &str, key: &str, sender: &str, content: Value| {
                    json!({ "type": kind, "state_key": key, "sender": sender, "content": content })
                };
                let state = [
                    ev("m.room.name", "", "@vic:x", json!({ "name": "Team room" })),
                    ev("m.room.member", "@vic:x", "@vic:x", json!({ "membership": "join", "displayname": "Vic" })),
                    ev("m.room.member", "@me:x", "@vic:x", json!({ "membership": "invite" })),
                ];
                let mut sync = stub_sync();
                sync["rooms"]["invite"] = json!({ "!team:x": { "invite_state": { "events": state } } });
                (200, sync)
            }
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if method == "POST" && p.ends_with("/join") => {
                seen.lock().unwrap().push(p.to_string());
                (200, json!({ "room_id": "!team:x" }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("invites"), &url).await;
        let room = {
            let client = store.read().matrix.clone().expect("a Matrix client");
            client.get_room(matrix_sdk::ruma::room_id!("!team:x")).expect("the invited room")
        };
        super::sync_invite(&store, &room).await;
        {
            let st = store.read();
            let c = st.chat("!team:x");
            assert_eq!((c.is_request, c.chat_type.as_str(), c.title.as_str()), (Some(true), "group", "Team room"));
            assert!(c.can_accept_request && c.is_read_only && c.delete_modes & crate::models::DELETE_FOR_ME != 0);
            assert_eq!(c.older, crate::models::Older::Start, "an invite has no history to load");
            let me = c.participants.items.iter().find(|u| u.is_self == Some(true)).expect("the own pending member");
            assert_eq!((me.is_pending, me.added_by.as_deref()), (Some(true), Some("@vic:x")), "vic invited me");
        }
        store.write().accept_request("!team:x");
        super::flush(store.clone()).await.expect("the stub takes the join");
        assert_eq!(joins.lock().unwrap().len(), 1, "Accept joins the room");
    }

    /// Like the send queue of Desktop's SDK, a send that the homeserver cannot take now (here two 503 answers) stays
    /// pending without an error, and the push sends it again by itself.
    #[tokio::test]
    async fn transient_send_errors_retry_by_themselves() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let tries = std::sync::Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split('?').next().expect("a path") {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, json!({ "start": "t0", "chunk": [] })),
            p if p.contains("/send/m.room.message/") && counted.fetch_add(1, Relaxed) < 2 => {
                (503, json!({ "errcode": "M_UNKNOWN", "error": "stub overload" }))
            }
            p if p.contains("/send/m.room.message/") => (200, json!({ "event_id": "$sent" })),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let dir = crate::testing::temp_dir("outbox-transient");
        let store = stub_store(&dir, &url).await;
        sync_stub_room(&store, "!room:x").await;
        tokio::spawn(super::push_loop(store.clone()));
        let draft = crate::store::Draft { text: Some("hi".into()), ..Default::default() };
        let txn = store.write().send("!room:x", draft).expect("a local echo").1.remove(0);
        pushed_until(&store, |_| tries.load(Relaxed) >= 1).await;
        assert!(store.read().stored_message(&txn).expect("the echo").send_error.is_none(), "no Not sent");
        pushed_until(&store, |st| st.sent.contains_key(&txn)).await;
        assert!(store.read().stored_message(&txn).expect("the echo").send_error.is_none());
        assert!(tries.load(Relaxed) >= 3 && store.read().outbox.is_empty(), "the third try went through");
    }

    #[tokio::test]
    async fn message_send_statuses_mark_delivered_and_failed_messages_like_desktop() {
        let status = |i: i64, target: i64, content: Value| {
            let mut content = content;
            content["m.relates_to"] = json!({ "rel_type": "m.reference", "event_id": format!("$e{target}") });
            json!({ "type": "com.beeper.message_send_status", "sender": "@bot:x", "event_id": format!("$s{i}"),
                "origin_server_ts": 1_700_000_100_000i64, "content": content, "unsigned": { "com.beeper.hs.order": 10 + i } })
        };
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split('?').next().expect("a path") {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => {
                let delivered = status(1, 1, json!({ "status": "SUCCESS", "delivered_to_users": ["@alice:x"] }));
                let failed = status(2, 2, json!({ "status": "FAIL_PERMANENT", "message": "Network down" }));
                (200, json!({ "start": "t2", "chunk": [failed, delivered, stub_message(2), stub_message(1)] }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("mss"), &url).await;
        let room = store.read().matrix.clone().unwrap().get_room(matrix_sdk::ruma::room_id!("!room:x")).unwrap();
        super::sync_room(&store, &room, Vec::new()).await;
        let st = store.read();
        let m = |id: &str| st.message(id);
        assert!(m("$e1").is_delivered && m("$e1").send_error.is_none(), "a delivered message");
        let error = crate::models::SendError { message: Some("Network down".into()), retrying: false };
        assert_eq!((m("$e2").is_delivered, m("$e2").send_error.as_ref()), (false, Some(&error)), "a failed message");
        assert_eq!(st.timeline_indices("!room:x").len(), 2, "send statuses are no timeline rows");
    }

    #[tokio::test]
    async fn demo_logout_deletes_local_data() {
        let cfg = crate::testing::demo_config("logout");
        let dir = cfg.data_dir.clone();
        let store = crate::store::StoreLock::shared(crate::store::BeeperStore::from_config(cfg.clone()));
        let cached = dir.join("media").join("cached");
        std::fs::write(&cached, "x").expect("write a cached file");
        super::logout(store).await.expect("demo logout");
        crate::store::BeeperStore::from_config(cfg);
        assert!(!cached.exists() && !dir.join(crate::store::LOGGED_OUT).exists(), "the next start deletes the data");
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
    fn mentions_link_like_desktop() {
        let (ann, anna) = (("Ann".to_string(), "@a:x".to_string()), ("Anna B".to_string(), "@b:x".to_string()));
        let links = super::mention_links("hi @Anna B and @Ann, mail a@b", &[&ann, &anna]);
        assert_eq!(links, "hi [@Anna B](https://matrix.to/#/@b:x) and [@Ann](https://matrix.to/#/@a:x), mail a@b");
        let odd = ("[x]".to_string(), "@c:x".to_string());
        assert_eq!(super::mention_links("@[x]", &[&odd]), "[@\\[x\\]](https://matrix.to/#/@c:x)");
        let room = ("room".to_string(), super::ROOM_MENTION.to_string());
        let c = super::text_content("@Ann @room".into(), &[ann, room], Some("@r:x")).unwrap();
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["m.mentions"], serde_json::json!({ "user_ids": ["@a:x", "@r:x"], "room": true }));
        assert_eq!(v["formatted_body"], "<a href=\"https://matrix.to/#/@a:x\">@Ann</a> @room");
        assert_eq!(v["body"], "@Ann @room");
        let plain = serde_json::to_value(super::text_content("hi".into(), &[], None).unwrap()).unwrap();
        assert_eq!(plain["m.mentions"], serde_json::json!({}), "Desktop always sends m.mentions");
    }

    #[test]
    fn read_only_follows_power_levels_like_desktop() {
        use serde_json::json;
        let levels = json!({ "users": { "@admin:x": 100 }, "events": { "m.room.message": 50 } });
        assert!(super::read_only(&levels, "@me:x") && !super::read_only(&levels, "@admin:x"));
        assert!(!super::read_only(&json!(null), "@me:x"), "a room without power levels takes messages");
        assert!(super::read_only(&json!({ "events_default": 10 }), "@me:x"), "the default level of events");
    }

    #[test]
    fn member_actions_follow_power_levels_like_desktop() {
        use super::manage;
        use crate::models::{ALL_MEMBER_ACTIONS, MANAGE_KICK, MANAGE_ROLE};
        use serde_json::json;
        let levels = json!({ "users": { "@me:x": 50, "@admin:x": 100 }, "events": { "m.room.power_levels": 100 } });
        let both = MANAGE_KICK | MANAGE_ROLE;
        assert_eq!(manage(&levels, ALL_MEMBER_ACTIONS, "@me:x", "@other:x", false), MANAGE_KICK);
        assert_eq!(manage(&levels, ALL_MEMBER_ACTIONS, "@me:x", "@admin:x", false), 0, "a higher member stays");
        assert_eq!(manage(&levels, 0, "@me:x", "@other:x", false), 0, "the room does not support kicks");
        assert_eq!(manage(&levels, ALL_MEMBER_ACTIONS, "@admin:x", "@me:x", false), both);
        assert_eq!(manage(&levels, ALL_MEMBER_ACTIONS, "@admin:x", "@admin:x", false), 0, "not the own user");
        let peers = json!({ "users_default": 100, "kick": 50, "state_default": 50 });
        assert_eq!(manage(&peers, ALL_MEMBER_ACTIONS, "@me:x", "@other:x", false), 0, "equal power in Matrix rooms");
        assert_eq!(manage(&peers, ALL_MEMBER_ACTIONS, "@me:x", "@other:x", true), MANAGE_ROLE, "any power in bridges");
    }

    /// Desktop finds the bookmarks room by its room ID, reads all of it, and does not show it as a chat.
    #[tokio::test]
    async fn bookmarks_come_from_the_managed_room_like_desktop() {
        use crate::models::BOOKMARKS_ROOM;
        let rel = json!({ "event_id": "$m", "room_id": "!room:x", "rel_type": "com.beeper.bookmark" });
        let bookmark = stub_event("$b", "com.beeper.bookmark", json!({ "com.beeper.relates_to": rel }));
        let text = stub_event("$m", "m.room.message", json!({ "msgtype": "m.text", "body": "hi" }));
        let store = stub_history("bookmarks-room", vec![text], (BOOKMARKS_ROOM, vec![bookmark])).await;
        sync_stub_room(&store, "!room:x").await;
        sync_stub_room(&store, BOOKMARKS_ROOM).await;
        let st = store.read();
        assert_eq!(st.bookmarks_room.as_deref(), Some(BOOKMARKS_ROOM));
        assert_eq!(st.message("$m").is_bookmarked, Some(true));
        assert!(!st.chat_idx_by_id.contains_key(BOOKMARKS_ROOM), "the bookmarks room is not a chat");
    }

    /// Desktop's message reminders: the own reminders of the reminders room, the latest event of a message winning
    /// and a redaction ending it; a new reminder is a new `com.beeper.message_reminder` event there, and Mark as
    /// Done redacts it with the relation in the redaction.
    #[tokio::test]
    async fn message_reminders_follow_the_reminders_room_like_desktop() {
        use crate::models::REMINDERS_ROOM;
        let rel = |id: &str| json!({ "event_id": id, "room_id": "!room:x", "rel_type": "com.beeper.message_reminder" });
        let remind = |id: &str, target: &str, at: i64| {
            stub_event(
                id,
                "com.beeper.message_reminder",
                json!({ "com.beeper.relates_to": rel(target), "remind_at_ms": at }),
            )
        };
        let mut foreign = remind("$r3", "$c", 9);
        foreign["sender"] = "@other:x".into();
        let redaction =
            stub_event("$r5", "m.room.redaction", json!({ "redacts": "$r4", "com.beeper.relates_to": rel("$b") }));
        let reminders =
            vec![remind("$r1", "$a", 5), remind("$r2", "$a", 7), foreign, remind("$r4", "$b", 8), redaction];
        let mut sync = stub_sync();
        sync["rooms"]["join"][REMINDERS_ROOM] = sync["rooms"]["join"]["!room:x"].clone();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |method, path, body| {
            let path = crate::codec::url_decode(path.split('?').next().expect("a path")).expect("an encoded path");
            match path.as_str() {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                p if p.ends_with("/messages") => {
                    let page = if p.contains(REMINDERS_ROOM) { reminders.clone() } else { Vec::new() };
                    (200, json!({ "start": "t", "chunk": page.into_iter().rev().collect::<Vec<_>>() }))
                }
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p => {
                    log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            }
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("message-reminders"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        sync_stub_room(&store, REMINDERS_ROOM).await;
        {
            let st = store.read();
            assert_eq!(st.reminders_room.as_deref(), Some(REMINDERS_ROOM));
            let active: Vec<(&str, i64)> =
                st.message_reminders_of("!room:x").iter().map(|(id, r)| (*id, r.remind_at_ms)).collect();
            assert_eq!(
                active,
                [("$a", 7)],
                "the latest event wins, others' reminders are skipped, a redaction ends one"
            );
            assert!(!st.chat_idx_by_id.contains_key(REMINDERS_ROOM), "the reminders room is not a chat");
        }
        let at = super::now_unix_millis() + 60_000;
        store.write().set_message_reminder("!room:x", "$m", Some(at));
        super::flush(store.clone()).await.expect("the reminder is sent");
        store.write().set_message_reminder("!room:x", "$m", None);
        super::flush(store.clone()).await.expect("the reminder is redacted");
        let room = format!("/_matrix/client/v3/rooms/{REMINDERS_ROOM}/");
        let sent: Vec<(String, Value)> = sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(p, b)| Some((p.strip_prefix(&room)?.to_string(), b.clone())))
            .collect();
        assert!(sent[0].0.starts_with("send/com.beeper.message_reminder/"), "{}", sent[0].0);
        assert_eq!(sent[0].1, json!({ "com.beeper.relates_to": rel("$m"), "remind_at_ms": at }));
        assert!(sent[1].0.starts_with("redact/$sent/"), "{}", sent[1].0);
        assert_eq!(sent[1].1, json!({ "com.beeper.relates_to": rel("$m") }));
        assert!(store.read().message_reminders_of("!room:x").iter().all(|(id, _)| *id != "$m"));
    }

    /// Desktop's "Remove from Beeper": the own removals of the removed-messages room delete their messages, a new
    /// removal goes to that room with one transaction ID and stays pending after a failure until the next launch, and
    /// a redaction of a removed message redacts its removal.
    #[tokio::test]
    async fn removals_follow_the_removed_messages_room_like_desktop() {
        use crate::models::REMOVED_MESSAGES_ROOM;
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        let rel = |id: &str| json!({ "com.beeper.relates_to": { "event_id": id, "room_id": "!room:x", "rel_type": "com.beeper.removed_message" } });
        let text = |id: &str| stub_event(id, "m.room.message", json!({ "msgtype": "m.text", "body": id }));
        let mut redaction = stub_event("$x", "m.room.redaction", json!({ "redacts": "$c" }));
        redaction["redacts"] = "$c".into();
        let events = vec![text("$a"), text("$b"), text("$c"), redaction];
        let mut foreign = stub_event("$ro", "com.beeper.removed_message", rel("$b"));
        foreign["sender"] = "@other:x".into();
        let removals = vec![
            stub_event("$ra", "com.beeper.removed_message", rel("$a")),
            foreign,
            stub_event("$rc", "com.beeper.removed_message", rel("$c")),
        ];
        let mut sync = stub_sync();
        sync["rooms"]["join"][REMOVED_MESSAGES_ROOM] = sync["rooms"]["join"]["!room:x"].clone();
        let (failing, sent) =
            (std::sync::Arc::new(AtomicBool::new(true)), std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        let (fail_now, sent_log) = (failing.clone(), sent.clone());
        let url = crate::bridge_stub::serve(move |method, path, body| {
            let path = crate::codec::url_decode(path.split('?').next().expect("a path")).expect("an encoded path");
            match path.as_str() {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                p if p.ends_with("/messages") => {
                    let page = if p.contains(REMOVED_MESSAGES_ROOM) { &removals } else { &events };
                    (200, json!({ "start": "t", "chunk": page.iter().rev().collect::<Vec<_>>() }))
                }
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p if p.contains("/send/com.beeper.removed_message/") && fail_now.load(Relaxed) => {
                    (403, json!({ "errcode": "M_FORBIDDEN", "error": "stub refusal" }))
                }
                p => {
                    sent_log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            }
        })
        .await;
        let dir = crate::testing::temp_dir("removals");
        let store = stub_store(&dir, &url).await;
        sync_stub_room(&store, "!room:x").await;
        sync_stub_room(&store, REMOVED_MESSAGES_ROOM).await;
        let state = |store: &crate::store::SharedStore, id: &str| {
            let mut st = store.write();
            let i = st.find_message(id).expect("a stored message");
            (st.messages[i].is_deleted == Some(true), st.messages[i].removed, st.messages[i].text.is_some())
        };
        assert_eq!(state(&store, "$a"), (true, true, false), "an own removal deletes the message");
        assert_eq!(state(&store, "$b"), (false, false, true), "Desktop skips the removals of other users");
        assert_eq!(state(&store, "$c"), (true, false, false), "a redacted message is not removed");
        {
            let mut st = store.write();
            st.delete_message("!room:x", "$a").expect("a redaction of a removed message");
            st.remove_from_beeper("$b");
        }
        assert_eq!(state(&store, "$a"), (true, false, false), "a redaction ends the removal");
        assert_eq!(state(&store, "$b"), (true, true, false), "a removal deletes the message at once");
        assert!(super::flush(store.clone()).await.is_err(), "the removal fails");
        // The changes for the removed-messages room, with the path after the room.
        let room = format!("/_matrix/client/v3/rooms/{REMOVED_MESSAGES_ROOM}/");
        let changes = || {
            let sent = sent.lock().unwrap();
            sent.iter().filter_map(|(p, b)| Some((p.strip_prefix(&room)?.to_string(), b.clone()))).collect::<Vec<_>>()
        };
        let redacted: Vec<String> = changes().into_iter().map(|c| c.0[..10].to_string()).collect();
        assert_eq!(redacted, ["redact/$rc", "redact/$ra"], "Desktop's cleanup redacts the removals");
        drop(store);
        failing.store(false, Relaxed);
        let store = stub_store(&dir, &url).await;
        assert_eq!(state(&store, "$b"), (true, true, false), "the removal persists");
        super::flush(store.clone()).await.expect("the pending removal goes again after a restart");
        let (path, body) = changes().pop().expect("the removal");
        assert!(path.starts_with("send/com.beeper.removed_message/~beeper-"), "{path}");
        assert_eq!(body, rel("$b"));
    }

    #[test]
    fn removal_events_like_desktop() {
        let rel = json!({ "event_id": "$m", "room_id": "!r", "rel_type": "com.beeper.removed_message" });
        let ev = json!({ "type": "com.beeper.removed_message", "sender": "@me:x", "event_id": "$rm",
            "content": { "com.beeper.relates_to": rel } });
        assert_eq!(super::removal(&ev, "@me:x"), Some(("$m".into(), "$rm".into())));
        assert_eq!(super::removal(&ev, "@other:x"), None, "Desktop takes only own removals");
        let mut redacted = ev.clone();
        redacted["unsigned"] = json!({ "redacted_because": {} });
        assert_eq!(super::removal(&redacted, "@me:x"), None);
        for key in ["room_id", "event_id"] {
            let mut bad = ev.clone();
            bad["content"]["com.beeper.relates_to"][key].take();
            assert_eq!(super::removal(&bad, "@me:x"), None, "a removal without {key}");
        }
        let mut bookmark = ev.clone();
        bookmark["content"]["com.beeper.relates_to"]["rel_type"] = "com.beeper.bookmark".into();
        assert_eq!(super::removal(&bookmark, "@me:x"), None);
    }

    /// Desktop's `managedRoomEventHandlers` take the events of the managed rooms, and the transcriptions room is not
    /// in the chat list, so these rooms are never chats. Desktop shows `!tips:beeper.local` as a chat.
    #[tokio::test]
    async fn managed_rooms_are_not_chats_like_desktop() {
        use crate::models::MANAGED_ROOMS;
        for room in MANAGED_ROOMS.into_iter().chain(["!tips:beeper.local"]) {
            let text = stub_event("$t", "m.room.message", json!({ "msgtype": "m.text", "body": "hi" }));
            let name = format!("managed-{}", &room[1..room.find(':').expect("a room ID")]);
            let store = stub_history(&name, Vec::new(), (room, vec![text])).await;
            sync_stub_room(&store, room).await;
            let st = store.read();
            let chat = !MANAGED_ROOMS.contains(&room);
            assert_eq!(st.chat_idx_by_id.contains_key(room), chat, "{room} is a chat: {chat}");
            assert_eq!(st.messages.iter().any(|m| m.chat_id == room), chat, "{room} has messages: {chat}");
        }
    }

    /// Desktop's `computeAccountID` and `getRooms`: a `com.beeper.local` room and a room with `com.beeper.managed`
    /// state are chats, and a room of another type is in no chat list.
    #[tokio::test]
    async fn typed_rooms_like_desktop() {
        let mut sync = stub_sync();
        let base = sync["rooms"]["join"]["!room:x"].clone();
        let typed = |kind: Option<&str>, extra: Option<Value>| {
            let mut room = base.clone();
            let events = room["state"]["events"].as_array_mut().expect("state events");
            if let Some(kind) = kind {
                events[0]["content"]["type"] = kind.into();
            }
            events.extend(extra);
            room
        };
        let managed = json!({ "type": "com.beeper.managed", "state_key": "", "sender": "@me:x",
            "event_id": "$managed", "origin_server_ts": 1, "content": {} });
        sync["rooms"]["join"]["!local:x"] = typed(Some("com.beeper.local"), None);
        sync["rooms"]["join"]["!other:x"] = typed(Some("com.example.other"), None);
        sync["rooms"]["join"]["!managed:x"] = typed(None, Some(managed));
        let url = crate::bridge_stub::serve(move |_, path, _| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, sync.clone()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.ends_with("/messages") => (200, json!({ "start": "t", "chunk": [] })),
            p if p.contains("/account_data/") || p.contains("/tags/") => (200, json!({})),
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("typed-rooms"), &url).await;
        for (room, chat) in [("!local:x", true), ("!managed:x", true), ("!other:x", false)] {
            sync_stub_room(&store, room).await;
            assert_eq!(store.read().chat_idx_by_id.contains_key(room), chat, "{room} is a chat: {chat}");
        }
    }

    /// Desktop's `parseMatrixRoomInput`: aliases, IDs, matrix.to links and matrix: URIs with their `via` servers.
    #[test]
    fn matrix_room_addresses_like_desktop() {
        use super::{room_address, BadAddress::*};
        let ok = |s: &str, id: &str, vias: &[&str]| {
            assert_eq!(room_address(s), Ok((id.into(), vias.iter().map(|v| v.to_string()).collect())), "{s}");
        };
        ok(" #room:example.org ", "#room:example.org", &[]);
        ok("!abc:example.org:8448", "!abc:example.org:8448", &[]);
        ok("!opaque", "!opaque", &[]);
        ok("https://matrix.to/#/%23room%3Aexample.org", "#room:example.org", &[]);
        ok("https://matrix.to/#/!abc:x.org/$ev?via=a.org&via=[::1]:80", "!abc:x.org", &["a.org", "[::1]:80"]);
        ok("matrix:r/room:example.org", "#room:example.org", &[]);
        ok("matrix:roomid/abc:x.org/e/ev?via=b.org", "!abc:x.org", &["b.org"]);
        assert_eq!(room_address("  "), Err(Empty));
        assert_eq!(room_address("#!x:y"), Err(MixedSigil));
        for bad in [
            "room",
            "#:x.org",
            "#room:bad_host",
            "#r:x.org:70000",
            "https://matrix.to/#/!a:x.org/notevent",
            "https://matrix.to/#/!a:x.org?via=bad_host",
            "matrix:u/user:x.org",
            "matrix:/r/x",
        ] {
            assert_eq!(room_address(bad), Err(Invalid), "{bad}");
        }
    }

    #[test]
    fn bookmark_events_like_desktop() {
        use super::bookmark_change;
        use serde_json::json;
        let rel = json!({ "com.beeper.relates_to": { "event_id": "$m", "room_id": "!r", "rel_type": "com.beeper.bookmark" } });
        let ev = |kind: &str, sender: &str| json!({ "type": kind, "sender": sender, "event_id": "$b", "content": rel, "unsigned": { "com.beeper.hs.order": 7 } });
        let (target, b) = bookmark_change(&ev("com.beeper.bookmark", "@me:x"), "@me:x").expect("a bookmark");
        let b = b.expect("an added bookmark");
        assert!(target == "$m" && b.room_id == "!r" && b.event_id == "$b" && b.order == 7);
        assert!(bookmark_change(&ev("com.beeper.bookmark", "@other:x"), "@me:x").is_none(), "Desktop skips others");
        assert_eq!(bookmark_change(&ev("m.room.redaction", "@me:x"), "@me:x"), Some(("$m".into(), None)));
        let bare = json!({ "type": "m.room.redaction", "sender": "@me:x", "redacts": "$b", "content": {} });
        assert!(bookmark_change(&bare, "@me:x").is_none(), "a redaction without a relation is unresolved");
    }

    /// Desktop's `applyMessageReminderEvent`: a redacted event and one without a positive `remind_at_ms` change
    /// nothing.
    #[test]
    fn message_reminder_events_like_desktop() {
        use super::message_reminder_change as change;
        use crate::models::MessageReminder;
        use serde_json::json;
        let rel = json!({ "event_id": "$m", "room_id": "!r", "rel_type": "com.beeper.message_reminder" });
        let ev = json!({ "type": "com.beeper.message_reminder", "sender": "@me:x", "event_id": "$e",
            "content": { "com.beeper.relates_to": rel, "remind_at_ms": 9 } });
        let r = MessageReminder { room_id: "!r".into(), remind_at_ms: 9 };
        assert_eq!(change(&ev, "@me:x"), Some(("$m".into(), Some(("$e".into(), r)))));
        let mut redacted = ev.clone();
        redacted["unsigned"] = json!({ "redacted_because": {} });
        assert_eq!(change(&redacted, "@me:x"), None);
        for at in [json!(0), json!("9"), Value::Null] {
            let mut bad = ev.clone();
            bad["content"]["remind_at_ms"] = at;
            assert_eq!(change(&bad, "@me:x"), None, "a malformed time");
        }
    }

    /// Desktop's `parseTranscription`: the last plain text, and the engine "unknown" when the event has none.
    #[test]
    fn transcription_events_like_desktop() {
        use super::transcription;
        use crate::models::Transcription;
        use serde_json::json;
        let rel = json!({ "room_id": "!r", "event_id": "$m", "rel_type": "com.beeper.transcription" });
        let texts = json!([{ "body": "one" }, { "body": "<b>two</b>", "mimetype": "text/html" }, { "body": "three", "mimetype": "text/plain" }]);
        let ev = |content: serde_json::Value| json!({ "type": "com.beeper.transcription", "content": content });
        let full = ev(json!({ "com.beeper.relates_to": rel, "m.text": texts,
            "com.beeper.transcription.model": "openai-whisper", "com.beeper.transcription.language": "fr" }));
        let want = Transcription {
            transcription: "three".into(),
            engine: "openai-whisper".into(),
            language: Some("fr".into()),
        };
        assert_eq!(transcription(&full), Some(("$m".into(), want)));
        let bare = transcription(&ev(json!({ "com.beeper.relates_to": rel, "m.text": [{ "body": "hi" }] })));
        assert_eq!(
            bare,
            Some(("$m".into(), Transcription { transcription: "hi".into(), engine: "unknown".into(), language: None }))
        );
        assert!(transcription(&ev(json!({ "com.beeper.relates_to": rel, "m.text": [{ "body": "" }] }))).is_none());
        assert!(transcription(&ev(json!({ "com.beeper.relates_to": { "event_id": "$m" }, "m.text": texts }))).is_none());
        let other = json!({ "type": "m.room.message", "content": { "com.beeper.relates_to": rel, "m.text": texts } });
        assert!(transcription(&other).is_none());
    }

    #[test]
    fn disappearing_timers_follow_room_features_like_desktop() {
        use super::timers;
        use crate::models::{DEFAULT_TIMERS, DISAPPEARING_TIMER};
        use serde_json::json;
        let levels = json!({ "users": { "@me:x": 50 }, "events": { DISAPPEARING_TIMER: 100 } });
        let beeper = timers(None, &levels, "@me:x").expect("the Beeper defaults have timers");
        assert!(
            beeper.kind == "after_send" && beeper.timers == DEFAULT_TIMERS && !beeper.can_set,
            "the event needs 100"
        );
        assert!(timers(Some(&json!({})), &levels, "@me:x").is_none(), "features without disappearing_timer");
        let f = json!({
            "disappearing_timer": { "types": ["after_read"], "timers": [3_600_000, 86_400_000] },
            "state": { DISAPPEARING_TIMER: { "level": 2 } },
        });
        let t = timers(Some(&f), &json!({ "users_default": 50 }), "@me:x").expect("timers");
        assert!(t.kind == "after_read" && t.timers == [3_600_000, 86_400_000] && t.can_set);
    }

    /// Desktop's `disappearingTimer` of messages: their event, a later edit or a local bridge's send status sets it,
    /// your read starts an `after_read` timer and the recipient's one of your `after_read_by_recipient` message, and
    /// new messages carry the timer of the room.
    #[tokio::test]
    async fn messages_disappear_like_desktop() {
        use crate::models::{Countdown, DISAPPEARING_TIMER};
        let (sent_at, my_read, vic_read) = (1_700_000_000_000_i64, 1_700_000_100_000_i64, 1_700_000_200_000_i64);
        let event = |i: i64, sender: &str, kind: &str, content: Value| {
            json!({ "type": kind, "sender": sender, "event_id": format!("$e{i}"), "origin_server_ts": sent_at + i,
                "content": content, "unsigned": { "com.beeper.hs.order": i } })
        };
        let text = |i: i64, sender: &str, timer: Value| {
            event(i, sender, "m.room.message", json!({ "msgtype": "m.text", "body": "hi", DISAPPEARING_TIMER: timer }))
        };
        let status = |i: i64, sender: &str, target: &str, timer: u64| {
            let content = json!({ "status": "SUCCESS", "disappearing_timer": { "type": "after_send", "timer": timer },
                "m.relates_to": { "rel_type": "m.reference", "event_id": target } });
            event(i, sender, "com.beeper.message_send_status", content)
        };
        let new =
            json!({ "msgtype": "m.text", "body": "", DISAPPEARING_TIMER: { "type": "after_read", "timer": 120_000 } });
        let edit = json!({ "msgtype": "m.text", "body": "* ", "m.new_content": new,
            "m.relates_to": { "rel_type": "m.replace", "event_id": "$e4" } });
        let events = [
            text(1, "@vic:x", json!({ "type": "after_send", "timer": 3_600_000 })),
            text(2, "@vic:x", json!({ "type": "after_read", "timer": 60_000 })),
            text(3, "@me:x", json!({ "type": "after_read_by_recipient", "timer": 60_000 })),
            text(4, "@vic:x", json!({ "type": "after_read", "timer": 60_000 })),
            text(5, "@me:x", json!(null)),
            event(6, "@vic:x", "m.room.message", edit),
            status(7, "@whatsappbot:beeper.local", "$e5", 5_000),
            status(8, "@vic:x", "$e5", 1_000),
        ];
        let mut sync = stub_sync();
        let room = &mut sync["rooms"]["join"]["!room:x"];
        let state = |kind: &str, key: &str, content: Value| {
            json!({ "type": kind, "state_key": key, "sender": "@me:x", "event_id": format!("${kind}{key}"),
                "origin_server_ts": 1, "content": content })
        };
        let timer = json!({ "type": "after_read", "timer": 86_400_000 });
        let states = room["state"]["events"].as_array_mut().unwrap();
        states.push(state("m.room.member", "@vic:x", json!({ "membership": "join", "displayname": "Vic" })));
        states.push(state(DISAPPEARING_TIMER, "", timer.clone()));
        let receipts = json!({ "$e2": { "m.read": { "@me:x": { "ts": my_read } } },
            "$e3": { "m.read": { "@vic:x": { "ts": vic_read } } } });
        room["ephemeral"] = json!({ "events": [{ "type": "m.receipt", "content": receipts }] });
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/sync" => (200, sync.clone()),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                p if p.ends_with("/messages") => {
                    (200, json!({ "start": "t", "chunk": events.iter().rev().collect::<Vec<_>>() }))
                }
                p if p.ends_with("/members") => (200, json!({ "chunk": [] })),
                _ if method == "GET" => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
                p => {
                    log.lock().unwrap().push((p.to_string(), body));
                    (200, json!({ "event_id": "$sent" }))
                }
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("disappearing"), &url).await;
        sync_stub_room(&store, "!room:x").await;
        {
            let st = store.read();
            let timer = |id: &str| st.message(id).disappearing.clone().expect("a timer");
            let at = |id: &str| (timer(id).timer, timer(id).read_at);
            assert_eq!(st.message("$e1").expiry(sent_at + 1 + 3_000_000), Some(Countdown::Left(600_000)));
            assert!(st.message("$e1").disappeared(sent_at + 1 + 3_600_000), "an after_send timer starts at the send");
            assert_eq!(at("$e2"), (60_000, Some(my_read)), "your read starts the timer of a received message");
            assert_eq!(at("$e3"), (60_000, Some(vic_read)), "the recipient's read starts the timer of yours");
            assert_eq!(at("$e4"), (120_000, None), "the edit set the timer, which waits for your read");
            assert_eq!(st.message("$e4").expiry(i64::MAX), Some(Countdown::Unread));
            assert_eq!(at("$e5"), (5_000, None), "only a local bridge sets the timer of a send status");
        }
        let out = super::Outgoing::Send {
            txn: "txn".into(),
            text: "hi".into(),
            reply_to: None,
            preview: None,
            mentions: Vec::new(),
            thread: None,
            links: false,
        };
        store.write().queue("!room:x", out);
        super::flush(store.clone()).await.expect("send the message");
        let sent = sent.lock().unwrap();
        let (_, body) = sent.iter().find(|(p, _)| p.contains("/send/m.room.message/")).expect("a message");
        assert_eq!(body[DISAPPEARING_TIMER], timer, "Desktop's disappearingTimerForMessages");
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
        let newer = with_inbox_visibility(rules.clone(), "telegram", "mentionsOnly");
        assert!(!hidden_from_inbox(&newer, "telegram", &tg, true), "Desktop skips an option that it does not know");
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
    fn message_rules_follow_desktop() {
        use super::{message_rules, MessageRules};
        use serde_json::json;
        let line = json!({"edit": 0, "delete": 1, "reaction": 2, "reply": 2, "edit_max_age": 86400});
        let rules = MessageRules {
            edit: false,
            edit_max_age: Some(86400),
            delete: false,
            delete_max_age: None,
            delete_for_me: false,
            react: true,
            reply: true,
        };
        assert_eq!(message_rules(&line), rules, "partial delete is not Delete for Everyone");
        let limited = message_rules(&json!({"edit": 1, "delete": 2, "delete_max_age": 60, "reaction": -2}));
        assert!(limited.edit && limited.delete && !limited.react && !limited.reply);
        assert!(MessageRules::young(Some(60), &crate::config::now_rfc3339_millis()));
        assert!(!MessageRules::young(Some(60), "2020-01-01T00:00:00.000Z"));
        assert!(MessageRules::young(None, "2020-01-01T00:00:00.000Z"));
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

    /// Like the `matrix` pseudo-bridge of Desktop's SDK, New Chat on the Matrix account reads the profile of a Matrix
    /// ID, asks for the devices of a user it does not track, and creates trusted private direct rooms, encrypted only
    /// for a user with devices, and private groups, encrypted unless New Chat turned it off. Desktop's
    /// `setRoomAvatar` then uploads the avatar of the group.
    #[tokio::test]
    async fn matrix_new_chat_requests_like_desktop_sdk() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            p if p.starts_with("/_matrix/client/v3/profile/") && p.contains("vic") => {
                (200, json!({ "displayname": "Vic", "avatar_url": "mxc://x/vic" }))
            }
            p if p.starts_with("/_matrix/client/v3/profile/") => {
                (404, json!({ "errcode": "M_NOT_FOUND", "error": "Profile was not found" }))
            }
            "/_matrix/client/v3/keys/query" if body["device_keys"]["@vic:x"].is_array() => {
                log.lock().unwrap().push((method.to_string(), "keys/query".to_string(), body));
                (200, json!({ "device_keys": { "@vic:x": { "VICDEV": { "user_id": "@vic:x", "device_id": "VICDEV" } } } }))
            }
            "/_matrix/client/v3/keys/query" if body["device_keys"]["@ghost:x"].is_array() => {
                log.lock().unwrap().push((method.to_string(), "keys/query".to_string(), body));
                (200, json!({ "device_keys": { "@ghost:x": {} } }))
            }
            "/_matrix/client/v3/keys/query" => (200, json!({ "device_keys": {} })),
            "/_matrix/client/v3/createRoom" => {
                log.lock().unwrap().push((method.to_string(), "createRoom".to_string(), body));
                (200, json!({ "room_id": "!new:x" }))
            }
            "/_matrix/client/v1/media/config" => (200, json!({ "m.upload.size": 1_000_000 })),
            "/_matrix/media/v3/upload" => {
                log.lock().unwrap().push((method.to_string(), "upload".to_string(), body));
                (200, json!({ "content_uri": "mxc://x/f" }))
            }
            p if p.starts_with("/_matrix/client/v3/rooms/") => {
                let p = p.strip_prefix("/_matrix/client/v3/").expect("a room request");
                log.lock().unwrap().push((method.to_string(), p.to_string(), body));
                (200, json!({ "event_id": "$avatar" }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("matrix-new-chat"), &url).await;
        let vic = super::resolve_matrix_user(store.clone(), "@vic:x".into()).await.expect("Vic's profile");
        let expected = json!({ "id": "@vic:x", "name": "Vic", "avatar_url": "mxc://x/vic",
            "identifiers": ["matrix:u/vic:x"], "mxid": "@vic:x" });
        assert_eq!(vic, Some(expected));
        assert_eq!(super::resolve_matrix_user(store.clone(), "@ghost:x".into()).await, Ok(None), "an unknown user");
        let bad = super::resolve_matrix_user(store.clone(), "vic".into()).await.expect_err("not a Matrix ID");
        assert!(bad.starts_with("user ID \"vic\" is not valid"), "{bad}");
        let take = || std::mem::take(&mut *sent.lock().unwrap());
        let encryption = json!({ "type": "m.room.encryption", "state_key": "",
            "content": { "algorithm": "m.megolm.v1.aes-sha2" } });
        assert_eq!(super::create_matrix_dm(store.clone(), "@vic:x".into()).await, Ok("!new:x".into()));
        let room = json!({ "invite": ["@vic:x"], "preset": "trusted_private_chat", "is_direct": true,
            "initial_state": [encryption] });
        let keys = json!({ "device_keys": { "@vic:x": [] }, "timeout": 10000 });
        let request = |path: &str, body: Value| ("POST".to_string(), path.to_string(), body);
        assert_eq!(take(), [request("keys/query", keys), request("createRoom", room)], "a user with devices");
        assert_eq!(super::create_matrix_dm(store.clone(), "@ghost:x".into()).await, Ok("!new:x".into()));
        let room = json!({ "invite": ["@ghost:x"], "preset": "trusted_private_chat", "is_direct": true });
        let keys = json!({ "device_keys": { "@ghost:x": [] }, "timeout": 10000 });
        assert_eq!(take(), [request("keys/query", keys), request("createRoom", room)], "an unknown user, unencrypted");
        let bad = super::create_matrix_dm(store.clone(), "ghost".into()).await.expect_err("not a Matrix ID");
        assert!(bad.starts_with("failed to resolve identifier: user ID"), "{bad}");
        let users = vec!["@vic:x".to_string(), "@ghost:x".to_string()];
        let group =
            |kind: &str, name: &str| super::create_matrix_group(store.clone(), kind.into(), name.into(), users.clone());
        assert_eq!(group("group", "Team").await, Ok("!new:x".into()));
        let room = json!({ "name": "Team", "invite": users, "preset": "private_chat", "initial_state": [encryption] });
        assert_eq!(take(), [request("createRoom", room)]);
        assert_eq!(group("group", "").await, Ok("!new:x".into()));
        assert!(take()[0].2.get("name").is_none(), "a group without a name sends none");
        assert_eq!(group("unencrypted_group", "").await, Ok("!new:x".into()));
        let room = json!({ "invite": users, "preset": "private_chat" });
        assert_eq!(take(), [request("createRoom", room)], "an unencrypted group has no encryption state");
        let bad = vec!["@vic:x".into(), "ghost".into()];
        let bad = super::create_matrix_group(store.clone(), "group".into(), String::new(), bad).await;
        assert!(bad.expect_err("a bad member").starts_with("user ID 2 (\"ghost\") is not valid"));
        assert!(take().is_empty(), "a bad member creates nothing");
        let avatar = super::set_room_avatar(store, "!new:x".into(), b"png".to_vec(), "image/png".into()).await;
        assert_eq!(avatar, Ok(()));
        let upload = ("POST".to_string(), "upload".to_string(), json!("png"));
        let state = ("PUT".to_string(), "rooms/!new:x/state/m.room.avatar/".to_string(), json!({ "url": "mxc://x/f" }));
        assert_eq!(take(), [upload, state], "Desktop's `setRoomAvatar` uploads the image, then sends the state");
    }

    /// Desktop's `searchMatrixUsers`: the user directory with a limit of 100, Matrix IDs as usernames, and a typed
    /// Matrix ID that the directory missed, on `beeper.com` without a server name, when its server is a domain.
    #[tokio::test]
    async fn matrix_user_search_like_desktop() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |_, path, body| match path.split_once('?').map_or(path, |p| p.0) {
            "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
            "/_matrix/client/v3/sync" => (200, stub_sync()),
            "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
            "/_matrix/client/v3/user_directory/search" => {
                log.lock().unwrap().push(body);
                let vic = json!({ "user_id": "@vic:example.com", "display_name": "Vic", "avatar_url": "mxc://x/vic" });
                (200, json!({ "limited": false, "results": [vic, { "user_id": "@una:example.com" }] }))
            }
            _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
        })
        .await;
        let store = stub_store(&crate::testing::temp_dir("matrix-user-search"), &url).await;
        let search = |q: &str| {
            let found = super::search_matrix_users(store.clone(), q.into());
            async { serde_json::to_value(found.await.expect("the directory")).expect("users as JSON") }
        };
        let found = search("vi").await;
        assert_eq!(*sent.lock().unwrap(), [json!({ "search_term": "vi", "limit": 100 })]);
        let vic = json!({ "id": "@vic:example.com", "username": "@vic:example.com", "fullName": "Vic",
            "imgURL": "mxc://x/vic" });
        let una = json!({ "id": "@una:example.com", "username": "@una:example.com" });
        assert_eq!(found, json!([vic, una]));
        let sam = json!({ "id": "@sam:beeper.com", "username": "@sam:beeper.com", "fullName": "@sam:beeper.com" });
        assert_eq!(search("@sam").await, json!([vic, una, sam]), "@x is on beeper.com");
        assert_eq!(search("@vic:example.com").await, json!([vic, una]), "a listed ID comes once");
        for q in ["@", "@sam:localhost", "@sam:x.y1", "sam:beeper.com"] {
            assert_eq!(search(q).await, json!([vic, una]), "{q} adds nobody");
        }
    }

    /// Desktop's `time_suggestion_options` comes in normalized from `com.beeper.desktop.prefs`, and
    /// `setTimeSuggestionOptions` writes the list back with the other Desktop prefs, which the next sync reads again.
    #[tokio::test]
    async fn time_suggestions_sync_like_desktop() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        use std::sync::{Arc, Mutex};
        let options = json!([" Tomorrow ", "", 7, "Tomorrow", "In 3w", "a", "b", "c", "d", "e", "f", "g"]);
        let stored = Arc::new(Mutex::new(json!({ "time_suggestion_options": options, "sticker_packs": true })));
        let (sent, next, syncs) = (Arc::new(Mutex::new(Vec::new())), stored.clone(), AtomicUsize::new(0));
        let log = sent.clone();
        let url =
            crate::bridge_stub::serve(move |method, path, body| match path.split_once('?').map_or(path, |p| p.0) {
                "/_matrix/client/versions" => (200, json!({ "versions": ["v1.11"] })),
                "/_matrix/client/v3/keys/upload" => (200, json!({ "one_time_key_counts": {} })),
                "/_matrix/client/v3/sync" => {
                    let mut sync = stub_sync();
                    sync["next_batch"] = json!(format!("s{}", syncs.fetch_add(1, Relaxed)));
                    let event = json!({ "type": "com.beeper.desktop.prefs", "content": next.lock().unwrap().clone() });
                    sync["account_data"] = json!({ "events": [event] });
                    (200, sync)
                }
                p if p.ends_with("/account_data/com.beeper.desktop.prefs") => {
                    log.lock().unwrap().push((method.to_string(), body));
                    (200, json!({}))
                }
                _ => (404, json!({ "errcode": "M_NOT_FOUND", "error": "not in the stub" })),
            })
            .await;
        let store = stub_store(&crate::testing::temp_dir("time-suggestions"), &url).await;
        let client = store.read().matrix.clone().expect("a Matrix client");
        super::sync_prefs(&client, &store).await;
        let read = store.read().settings.time_suggestions.clone();
        assert_eq!(read, ["Tomorrow", "In 3w", "a", "b", "c", "d", "e", "f"], "Desktop's `xpe`");
        let (kind, content) = {
            let mut st = store.write();
            st.update_settings(|s| s.time_suggestions = vec!["In 3w".into(), "Tomorrow".into()]);
            super::time_suggestions_write(&mut st).expect("the prefs are synced")
        };
        assert_eq!(content, json!({ "time_suggestion_options": ["In 3w", "Tomorrow"], "sticker_packs": true }));
        super::set_global(client.clone(), kind, content.clone()).await.expect("the write");
        assert_eq!(*sent.lock().unwrap(), [("PUT".to_string(), content.clone())]);
        store.write().update_settings(|s| s.time_suggestions.clear());
        *stored.lock().unwrap() = content;
        client.sync_once(super::SyncSettings::default()).await.expect("sync the written prefs");
        super::sync_prefs(&client, &store).await;
        assert_eq!(store.read().settings.time_suggestions, ["In 3w", "Tomorrow"], "the write comes back");
        *stored.lock().unwrap() = json!({ "time_suggestion_options": [] });
        client.sync_once(super::SyncSettings::default()).await.expect("sync a reset");
        super::sync_prefs(&client, &store).await;
        let st = store.read();
        assert_eq!(st.settings.time_suggestions(), crate::prefs::TIME_SUGGESTIONS, "a reset is Desktop's defaults");
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

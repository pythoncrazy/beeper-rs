use crate::config::{now_rfc3339_millis, rfc3339_from_millis};
use crate::import::DesktopCrypto;
use crate::models::{
    Attachment, AttachmentSize, BridgeAccountInfo, Chat, ChatParticipants, EditHistoryEntry, Message, MessageLink,
    Reaction, User, VerificationSas, VerificationSasEmoji, VerificationState,
};
use crate::store::{sort_key, BeeperStore, SharedStore, NOTE_CHAT_ID};
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
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::tag::TagName;
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, RoomAccountDataEventType, StateEventType};
use matrix_sdk::{Client, LoopCtrl, Room, RoomMemberships, SessionMeta, SessionTokens};
use matrix_sdk_crypto::vodozemac;
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::oneshot;

pub const UTD_TEXT: &str = "Waiting for message";
const HISTORY: u32 = 30;

enum Op {
    Upsert(Box<Message>),
    React(String, Reaction, Box<Message>),
    Edit(String, String, Vec<MessageLink>, String),
    Redact(String),
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

fn links(c: &Value) -> Vec<MessageLink> {
    c["com.beeper.linkpreviews"].as_array().into_iter().flatten().filter_map(link).collect()
}

async fn state(room: &Room, kind: &str) -> Option<Value> {
    let events = room.get_state_events(StateEventType::from(kind)).await.expect("read room state");
    let raw = match events.into_iter().next()? {
        RawAnySyncOrStrippedState::Sync(r) => r.json().get().to_string(),
        RawAnySyncOrStrippedState::Stripped(r) => r.json().get().to_string(),
    };
    Some(serde_json::from_str::<Value>(&raw).expect("state event JSON")["content"].take())
}

async fn account_data(room: &Room, kind: &str) -> Option<Value> {
    let raw = room.account_data(RoomAccountDataEventType::from(kind)).await.expect("read room account data")?;
    Some(serde_json::from_str::<Value>(raw.json().get()).expect("account data JSON")["content"].take())
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

fn hidden_from_inbox(rules: &Value, account_id: &str, bridge: &Value, group: bool) -> bool {
    let name = bridge["com.beeper.bridge_name"].as_str().unwrap_or_default();
    let vis = |key: &str| rules["visibility"][key].as_str().map(str::to_string);
    let default = if matches!(name, "slackgo" | "discordgo") { "dmOnly" } else { "all" };
    let base = vis(account_id).unwrap_or(default.into());
    let scoped = match s(&bridge["network"]["id"]) {
        _ if name == "slackgo" => vis(account_id.split('-').next().expect("split yields a part")),
        Some(n) if !matches!(name, "whatsapp" | "telegram") => vis(&format!("{account_id}.{n}")),
        _ => None,
    };
    match scoped.unwrap_or(base).as_str() {
        "all" => false,
        "none" => true,
        "dmOnly" => group && bridge["com.beeper.room_type.v2"] != "group_dm",
        other => panic!("unknown inbox visibility {other}"),
    }
}

async fn chat(room: &Room, accounts: &[BridgeAccountInfo], rules: &Value) -> Chat {
    let bridge = state(room, "m.bridge").await.unwrap_or(Value::Null);
    let service = room.service_members().unwrap_or_default();
    let bot = s(&bridge["bridgebot"]);
    let me = room.own_user_id();
    let members = room.members_no_sync(RoomMemberships::JOIN).await.expect("read room members");
    let items: Vec<User> = members
        .iter()
        .filter(|m| !service.contains(m.user_id()) && bot.as_deref() != Some(m.user_id().as_str()))
        .map(|m| User {
            id: m.user_id().to_string(),
            full_name: Some(m.name().to_string()),
            username: None,
            email: None,
            phone_number: None,
            img_url: m.avatar_url().filter(|u| u.is_valid()).map(|u| u.to_string()),
            is_self: Some(m.user_id() == me),
            cannot_message: Some(false),
            custom_nickname: None,
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
    Chat {
        id: room.room_id().to_string(),
        is_hidden_from_inbox: Some(hidden_from_inbox(rules, &account_id, &bridge, !dm)),
        is_request: Some(bridge["channel"]["com.beeper.message_request"] == true),
        network_name: s(&bridge["network"]["displayname"]),
        img_url: room.avatar_url().filter(|u| u.is_valid()).map(|u| u.to_string()),
        account_id,
        participants: ChatParticipants { has_more: false, total: items.len(), items },
        title,
        chat_type: if dm { "single" } else { "group" }.into(),
        unread_count: u32::try_from(room.unread_notification_counts().notification_count).expect("unread count"),
        description: room.topic(),
        draft: None,
        is_archived: Some(false),
        is_marked_unread: Some(room.is_marked_unread()),
        is_muted: Some(matches!(
            room.user_defined_notification_mode().await,
            Some(RoomNotificationMode::Mute | RoomNotificationMode::MentionsAndKeywordsOnly)
        )),
        is_pinned: Some(tags.contains_key(&TagName::Favorite)),
        is_low_priority: Some(tags.contains_key(&TagName::LowPriority)),
        label_ids: Vec::new(),
        last_activity: None,
        last_read_message_sort_key: None,
        local_chat_id: None,
        merge: None,
        merged_into_chat_id: None,
        preview: None,
        reminder: None,
        chat_theme: None,
        flags: 0,
    }
}

fn attachment(c: &Value, kind: &str) -> Attachment {
    let info = &c["info"];
    let dim = |k: &str| info[k].as_u64().map(|n| u32::try_from(n).expect("media dimension"));
    let voice = c.get("org.matrix.msc3245.voice").is_some();
    Attachment {
        attachment_type: kind.into(),
        id: None,
        duration: info["duration"].as_f64().map(|ms| ms / 1000.0),
        file_name: s(&c["filename"]).or(s(&c["body"])),
        file_size: info["size"].as_u64(),
        is_gif: Some(info["fi.mau.gif"] == true),
        is_sticker: Some(false),
        is_voice_note: Some(voice),
        mime_type: s(&info["mimetype"]),
        poster_img: None,
        size: dim("w").map(|w| AttachmentSize { width: Some(w), height: dim("h") }),
        src_url: s(&c["url"]),
        transcript: None,
    }
}

async fn op(room: &Room, chat_id: &str, account_id: &str, e: &Value) -> Option<Op> {
    if e["unsigned"].get("redacted_because").is_some() {
        return None;
    }
    let (c, rel) = (&e["content"], &e["content"]["m.relates_to"]);
    let sender = s(&e["sender"]).expect("event sender");
    let id = s(&e["event_id"]).expect("event ID");
    let kind = s(&e["type"]).expect("event type");
    let (text, attachments, message_type) = match (kind.as_str(), c["msgtype"].as_str()) {
        ("m.reaction", _) => (None, Vec::new(), "REACTION"),
        ("m.room.redaction", _) => return Some(Op::Redact(s(&e["redacts"]).or(s(&c["redacts"]))?)),
        ("m.room.message", _) if rel["rel_type"] == "m.replace" => {
            let (new, ts) = (&c["m.new_content"], e["origin_server_ts"].as_i64().expect("event timestamp"));
            return Some(Op::Edit(s(&rel["event_id"])?, body(new)?, links(new), rfc3339_from_millis(ts)));
        }
        ("m.room.encrypted", _) => (Some(UTD_TEXT.into()), Vec::new(), "TEXT"),
        ("m.sticker", _) => (None, vec![Attachment { is_sticker: Some(true), ..attachment(c, "img") }], "STICKER"),
        ("m.room.message", Some("m.image")) => (None, vec![attachment(c, "img")], "IMAGE"),
        ("m.room.message", Some("m.video")) => (None, vec![attachment(c, "video")], "VIDEO"),
        ("m.room.message", Some("m.audio")) if c.get("org.matrix.msc3245.voice").is_some() => {
            (None, vec![attachment(c, "audio")], "VOICE")
        }
        ("m.room.message", Some("m.audio")) => (None, vec![attachment(c, "audio")], "AUDIO"),
        ("m.room.message", Some("m.file")) => (None, vec![attachment(c, "unknown")], "FILE"),
        ("m.room.message", Some("m.location")) => (body(c), Vec::new(), "LOCATION"),
        ("m.room.message", Some("m.notice")) => (body(c), Vec::new(), "NOTICE"),
        ("m.room.message", _) => (body(c), Vec::new(), "TEXT"),
        _ => return None,
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
        linked_message_id: s(&rel["m.in_reply_to"]["event_id"]).or(reaction.as_ref().and(s(&rel["event_id"]))),
        reactions: Vec::new(),
        text,
        message_type: Some(message_type.into()),
        poll: None,
        edit_history: Vec::new(),
        links: links(c),
        flags: 0,
    });
    match reaction {
        Some(r) => Some(Op::React(msg.linked_message_id.clone()?, r, msg)),
        None => Some(Op::Upsert(msg)),
    }
}

fn link(p: &Value) -> Option<MessageLink> {
    Some(MessageLink {
        url: s(&p["matched_url"]).or(s(&p["og:url"]))?,
        title: s(&p["og:title"])?,
        summary: s(&p["og:description"]),
    })
}

fn reaction_text(key: &str, target: Option<&Message>, who: Option<&str>) -> String {
    let whose = if target.is_some_and(|t| t.is_sender == Some(true)) { "your" } else { "a" };
    let object = match target.map(|t| (t.text.as_deref().filter(|x| !x.is_empty()), t.attachments.first())) {
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
    match key.trim() {
        "❤️" | "♥️" => "loved".into(),
        "👍" => "liked".into(),
        "👎" => "disliked".into(),
        "😂" | "😹" => "laughed at".into(),
        "❓" | "🤔" => "questioned".into(),
        "❗" | "‼️" => "emphasized".into(),
        k if ["haha", "laugh"].contains(&k.to_lowercase().as_str()) => "laughed at".into(),
        k if k.starts_with("mxc://") || k.starts_with("localmxc://") => "reacted to".into(),
        k => format!("reacted {k} to"),
    }
}

async fn history(room: &Room) -> Vec<TimelineEvent> {
    let mut opts = MessagesOptions::backward();
    opts.limit = HISTORY.into();
    let mut chunk = room.messages(opts).await.expect("load room history").chunk;
    chunk.reverse();
    chunk
}

async fn sync_room(store: &SharedStore, room: &Room, mut events: Vec<TimelineEvent>) {
    if state(room, "m.room.create").await.expect("rooms have a create event")["type"].is_string() {
        return;
    }
    let chat_id = chat_id(room).await;
    let note = chat_id == NOTE_CHAT_ID;
    let accounts = store.read().unwrap().bridge_accounts.clone();
    let account = room.client().account();
    let rules = account
        .account_data_raw(GlobalAccountDataEventType::from("com.beeper.chat_list_rules"))
        .await
        .expect("read chat list rules")
        .map_or(Value::Null, |r| r.deserialize_as().expect("chat list rules JSON"));
    let chat = chat(room, &accounts, &rules).await;
    if !store.read().unwrap().chat_msg_indices.contains_key(&chat_id) {
        let known: HashSet<String> = events.iter().filter_map(|e| Some(e.event_id()?.to_string())).collect();
        let older =
            history(room).await.into_iter().filter(|e| e.event_id().is_some_and(|id| !known.contains(id.as_str())));
        events = older.chain(events).collect();
    }
    let (mut ops, mut non_messages) = (Vec::new(), Vec::new());
    for e in &events {
        let v: Value = serde_json::from_str(e.raw().json().get()).expect("timeline event JSON");
        match op(room, &chat_id, &chat.account_id, &v).await {
            Some(Op::Upsert(m)) => ops.push(Op::Upsert(m)),
            o => {
                non_messages.extend(s(&v["event_id"]));
                ops.extend(o);
            }
        }
    }
    let done = account_data(room, "com.beeper.inbox.done").await.and_then(|d| d["at_order"].as_i64());
    let (members, group) = (chat.participants.total, chat.chat_type == "group");
    let mut st = store.write().unwrap();
    if !note {
        st.upsert_chat(chat);
    } else {
        let i = st.chat_idx_by_id[NOTE_CHAT_ID];
        let room = |c: &Chat| {
            (c.img_url.clone(), c.is_muted, c.is_pinned, c.is_low_priority, c.unread_count, c.is_marked_unread)
        };
        if room(&st.chats[i]) != room(&chat) {
            st.update_chat(i, |c| {
                (c.img_url, c.is_muted, c.is_pinned, c.is_low_priority, c.unread_count, c.is_marked_unread) =
                    room(&chat);
            });
        }
    }
    for id in non_messages {
        if st.msg_idx_by_id.contains_key(&id) {
            st.delete_message(&chat_id, &id).expect("remove a decrypted placeholder");
        }
    }
    for op in ops {
        match op {
            Op::Upsert(m) => st.upsert_message(*m),
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
                let newer = st.chats[idx].preview.as_ref().is_none_or(|p| p.sort_key < preview.sort_key);
                if shown && newer {
                    st.update_chat(idx, |c| {
                        c.last_activity = Some(preview.timestamp.clone());
                        c.preview = Some(*preview);
                    });
                }
            }
            Op::Edit(target, text, links, timestamp) => {
                if let Some(&i) = st.msg_idx_by_id.get(&target) {
                    st.update_message(i, |m| {
                        let prev = m.text.replace(text).unwrap_or_default();
                        if m.edit_history.last().is_none_or(|e| e.timestamp != timestamp) {
                            m.edit_history.push(EditHistoryEntry { text: prev, timestamp });
                        }
                        m.links = links;
                    });
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
    let last = st.chat_msg_indices.get(&chat_id).and_then(|v| v.last()).map(|&i| st.messages[i].sort_key.clone());
    let archived = !note && done.is_some_and(|d| last.is_none_or(|l| sort_key(d) >= l));
    if st.chats[idx].is_archived != Some(archived) {
        st.update_chat(idx, |c| c.is_archived = Some(archived));
    }
}

pub async fn thumbnail(client: Client, url: String) -> Result<Vec<u8>, String> {
    let size = matrix_sdk::ruma::uint!(96);
    let format = MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(Method::Crop, size, size));
    let request = MediaRequestParameters { source: MediaSource::Plain(url.into()), format };
    client.media().get_media_content(&request, true).await.map_err(|e| e.to_string())
}

async fn connect(store: &SharedStore) -> Result<Client, String> {
    let (s, dir) = {
        let st = store.read().unwrap();
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
    let st = store.read().unwrap();
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
            store.write().unwrap().matrix_error = Some(format!("Could not sign this device: {e}"));
        }
    }
    let verified = device.is_cross_signed_by_owner();
    let state = (verified, client.encryption().backups().are_enabled().await);
    let mut st = store.write().unwrap();
    if (st.beeper_state.verified, st.beeper_state.key_backup) != state {
        (st.beeper_state.verified, st.beeper_state.key_backup) = state;
        st.persist_state_to_sqlite();
        st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
        st.emit_ws("setup.updated", "", None, None, None);
    }
}

const DESKTOP_PICKLE_KEY: &[u8] = b"pickle";

fn unpickle_secret(pickle: &str) -> Result<String, String> {
    use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
    use hmac::{KeyInit, Mac};
    let raw = vodozemac::base64_decode(pickle).map_err(|e| e.to_string())?;
    let (ciphertext, tag) = raw.split_at(raw.len().checked_sub(8).ok_or("the secret is too short")?);
    let mut keys = [0u8; 80];
    hkdf::Hkdf::<sha2::Sha256>::new(Some(&[0]), DESKTOP_PICKLE_KEY).expand(b"Pickle", &mut keys).expect("80 bytes");
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&keys[32..64]).expect("any HMAC key length");
    mac.update(ciphertext);
    mac.verify_truncated_left(tag).map_err(|_| "the secret has a bad MAC")?;
    let aes = cbc::Decryptor::<aes::Aes256>::new_from_slices(&keys[..32], &keys[64..]).expect("AES-256 key and IV");
    let plain = aes.decrypt_padded_vec::<Pkcs7>(ciphertext).map_err(|e| e.to_string())?;
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
    let client = store.read().unwrap().matrix.clone().ok_or("Matrix is not connected yet")?;
    client.encryption().recovery().recover(key.trim()).await.map_err(|e| e.to_string())?;
    set_verification(&store, "verified", None);
    Ok(())
}

fn set_verification(store: &SharedStore, state: &str, sas: Option<VerificationSas>) {
    let mut st = store.write().unwrap();
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
        out.push(DeviceInfo {
            name: d.display_name.unwrap_or_else(|| "Unknown".into()),
            last_seen_ms: d.last_seen_ts.map(|t| t.0.into()),
            verified: device.is_some_and(|d| d.is_verified()),
            current: d.device_id == current,
        });
    }
    Ok(out)
}

pub async fn logout(store: SharedStore) -> Result<(), String> {
    let (client, demo, dir) = {
        let st = store.read().unwrap();
        (st.matrix.clone(), st.config.demo, st.data_dir.clone())
    };
    if !demo {
        client.ok_or("Matrix is not connected yet")?.logout().await.map_err(|e| e.to_string())?;
    }
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Could not delete the local data: {e}")),
        _ => Ok(()),
    };
    for f in ["beeper-rs.db", "beeper-rs.db-wal", "beeper-rs.db-shm"] {
        gone(std::fs::remove_file(dir.join(f)))?;
    }
    for d in ["matrix", "media"] {
        gone(std::fs::remove_dir_all(dir.join(d)))?;
    }
    Ok(())
}

pub async fn verify(store: SharedStore, confirm: oneshot::Receiver<bool>) -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let client = store.read().unwrap().matrix.clone().ok_or("Matrix is not connected yet")?;
    let me = client.user_id().expect("signed-in client has a user ID").to_owned();
    let identity = client.encryption().get_user_identity(&me).await.map_err(|x| e(&x))?;
    let request = identity.ok_or("this account has no cross-signing identity")?.request_verification().await;
    let request = request.map_err(|x| e(&x))?;
    set_verification(&store, "requested", None);
    let mut changes = request.changes();
    let sas = loop {
        match changes.next().await.ok_or("the verification request ended")? {
            VerificationRequestState::Ready { .. } => {
                if let Some(sas) = request.start_sas().await.map_err(|x| e(&x))? {
                    break sas;
                }
            }
            VerificationRequestState::Transitioned { verification: Verification::SasV1(sas), .. } => {
                sas.accept().await.map_err(|x| e(&x))?;
                break sas;
            }
            VerificationRequestState::Cancelled(c) => return Err(c.reason().to_string()),
            _ => {}
        }
    };
    let mut confirm = Some(confirm);
    let mut changes = sas.changes();
    while let Some(state) = changes.next().await {
        match state {
            SasState::KeysExchanged { emojis: Some(emojis), decimals: (a, b, c) } => {
                let emojis = emojis.emojis.iter();
                let emojis =
                    emojis.map(|x| VerificationSasEmoji { emoji: x.symbol.into(), description: x.description.into() });
                let sas_info = VerificationSas { decimals: Some(format!("{a} {b} {c}")), emojis: emojis.collect() };
                set_verification(&store, "sas_started", Some(sas_info));
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
        }
    }
    Err("the verification ended".into())
}

pub async fn run(store: SharedStore) {
    crate::cloud::install_tls();
    let mut rx = store.read().unwrap().ws_tx.subscribe();
    while !store.read().unwrap().signed_in() {
        rx.recv().await.ok();
    }
    let client = match connect(&store).await {
        Ok(c) => c,
        Err(e) => return store.write().unwrap().matrix_error = Some(e),
    };
    store.write().unwrap().matrix = Some(client.clone());
    tokio::spawn(retry_undecrypted(client.clone(), store.clone()));
    tokio::spawn(retry_after_backup(client.clone(), store.clone()));
    let first = !store.read().unwrap().beeper_state.first_sync_done;
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
        let mut st = store.write().unwrap();
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
                    Err(e) => store.write().unwrap().matrix_error = Some(e.to_string()),
                    Ok(resp) => {
                        store.write().unwrap().matrix_error = None;
                        refresh_e2ee(&client, &store).await;
                        for (id, update) in resp.rooms.joined {
                            let room = client.get_room(&id).expect("synced room is known");
                            sync_room(&store, &room, update.timeline.events).await;
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
        let store = std::sync::Arc::new(std::sync::RwLock::new(crate::store::BeeperStore::from_config(cfg)));
        assert!(dir.join("beeper-rs.db").exists());
        super::logout(store).await.expect("demo logout");
        assert!(!dir.join("beeper-rs.db").exists() && !dir.join("media").exists());
        std::fs::remove_dir_all(dir).expect("remove the test data dir");
    }
}

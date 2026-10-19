use crate::config::rfc3339_from_millis;
use crate::models::{
    Attachment, AttachmentSize, Chat, ChatParticipants, Message, MessageLink, Reaction, ThemeChoice, User, ALL_FORMATS,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

const SNAPSHOT_FILES: [&str; 4] = ["index.db", "index.db-wal", "account.db", "account.db-wal"];

#[derive(Debug, Clone)]
pub struct DesktopCrypto {
    pub user_id: String,
    pub backup_version: String,
    pub group_sessions: Vec<GroupSessionRow>,
    pub secrets: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct GroupSessionRow {
    pub room_id: String,
    pub sender_key: String,
    pub signing_key: String,
    pub pickle: String,
    pub shared_history: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BeeperData {
    pub crypto: Option<DesktopCrypto>,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
    pub pin_rank: HashMap<String, usize>,
    pub recent_chat_ids: Vec<String>,
    pub theme: Option<ThemeChoice>,
}

pub fn has_beeper_data(dir: &Path) -> bool {
    dir.join("index.db").is_file()
}

pub fn read_beeper_data(dir: &Path) -> Result<BeeperData, String> {
    if !has_beeper_data(dir) {
        return Err(format!("{} has no Beeper Desktop data", dir.display()));
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_nanos();
    let tmp = std::env::temp_dir().join(format!("beeper-rs-import-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
    let out = SNAPSHOT_FILES
        .iter()
        .map(|f| (dir.join(f), tmp.join(f)))
        .filter(|(src, _)| src.is_file())
        .try_for_each(|(src, dst)| {
            std::fs::copy(&src, dst).map(drop).map_err(|e| format!("copy {}: {e}", src.display()))
        })
        .and_then(|()| {
            read_snapshot(&tmp, dir).map_err(|e| format!("read Beeper Desktop data in {}: {e}", dir.display()))
        });
    std::fs::remove_dir_all(&tmp).unwrap_or_else(|e| panic!("remove import snapshot {}: {e}", tmp.display()));
    out
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

fn read_kv_json(db: &Connection, key: &str) -> Result<Value, String> {
    db.query_row("SELECT value FROM key_values WHERE key = ?1", [key], |r| r.get::<_, String>(0))
        .optional()
        .map_err(|e| e.to_string())?
        .map_or(Ok(Value::Null), |s| serde_json::from_str(&s).map_err(|e| format!("{key} JSON: {e}")))
}

fn read_recent_chats(dir: &Path, remap: &dyn Fn(&str) -> String) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("Local Storage/leveldb")) else { return Vec::new() };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    paths.sort();
    let mut out = Vec::new();
    let needle = b"thread-history";
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        for i in bytes.windows(needle.len()).enumerate().filter_map(|(i, w)| (w == needle).then_some(i)) {
            let rest = &bytes[i + needle.len()..bytes.len().min(i + 8192)];
            let Some(s) = rest.windows(2).position(|w| w == b"[[") else { continue };
            let Some(e) = rest[s..].windows(2).position(|w| w == b"]]") else { continue };
            if let Ok(pairs) = serde_json::from_slice::<Vec<(String, String)>>(&rest[s..s + e + 2]) {
                if !pairs.is_empty() {
                    out = pairs.into_iter().map(|(_, id)| remap(&id)).collect();
                }
            }
        }
    }
    out
}

fn read_snapshot(dir: &Path, src_dir: &Path) -> Result<BeeperData, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let crypto = match dir.join("account.db").is_file() {
        true => read_crypto(&open(&dir.join("account.db")).map_err(e)?).map_err(|e| format!("read the keys: {e}"))?,
        false => None,
    };
    let db = open(&dir.join("index.db")).map_err(e)?;
    let note_id: Option<String> = db
        .query_row(
            "SELECT threadID FROM threads WHERE json_extract(CAST(thread AS TEXT), '$.extra.isNoteToSelf') = 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(e)?;
    let remap = |s: &str| match note_id.as_deref() == Some(s) {
        true => crate::store::NOTE_CHAT_ID.to_string(),
        false => s.to_string(),
    };
    let rules = read_kv_json(&db, "ad:com.beeper.chat_list_rules")?;
    let pins = read_kv_json(&db, "ad:com.beeper.favourites_order")?;
    let pin_rank = pins["Inbox"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, v)| Some((remap(v.as_str()?), i)))
        .collect();
    let recent_chat_ids = read_recent_chats(src_dir, &remap);
    let mut people = read_participants(&db).map_err(e)?;
    let chats = read_chats(&db, &mut people, &rules)?;
    let messages = read_messages(&db, &chats)?;
    Ok(BeeperData { crypto, chats, messages, pin_rank, recent_chat_ids, theme: read_theme(&db)? })
}

fn read_theme(db: &Connection) -> Result<Option<ThemeChoice>, String> {
    let prefs: Option<String> = db
        .query_row("SELECT value FROM key_values WHERE key = 'localPrefs'", [], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(prefs) = prefs else { return Ok(None) };
    let prefs: Value = serde_json::from_str(&prefs).map_err(|e| format!("localPrefs JSON: {e}"))?;
    match prefs["native_theme_source"].as_str() {
        Some("light") => Ok(Some(ThemeChoice::Light)),
        Some("dark") => Ok(Some(ThemeChoice::Dark)),
        Some("system") => Ok(Some(ThemeChoice::System)),
        other => Err(format!("unknown Beeper Desktop theme {other:?}")),
    }
}

fn read_crypto(db: &Connection) -> rusqlite::Result<Option<DesktopCrypto>> {
    let Some((user_id, device_id)) = db
        .query_row("SELECT user_id, device_id FROM account", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?
    else {
        return Ok(None);
    };
    let account = format!("{user_id}/{device_id}");
    let text = |b: Vec<u8>| String::from_utf8(b).expect("libolm pickles are base64 text");
    let backup_version =
        db.query_row("SELECT key_backup_version FROM crypto_account WHERE account_id = ?1", [&account], |r| r.get(0))?;
    let group_sessions = db
        .prepare(
            "SELECT room_id, sender_key, signing_key, session, shared_history IS 1
             FROM crypto_megolm_inbound_session WHERE account_id = ?1 AND session IS NOT NULL",
        )?
        .query_map([&account], |r| {
            Ok(GroupSessionRow {
                room_id: r.get(0)?,
                sender_key: r.get(1)?,
                signing_key: r.get(2)?,
                pickle: text(r.get(3)?),
                shared_history: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let secrets = db
        .prepare("SELECT name, secret FROM crypto_secrets WHERE account_id = ?1")?
        .query_map([&account], |r| Ok((r.get(0)?, text(r.get(1)?))))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(DesktopCrypto { user_id, backup_version, group_sessions, secrets }))
}

fn read_participants(db: &Connection) -> rusqlite::Result<HashMap<String, Vec<User>>> {
    let mut ids: HashMap<String, [Option<String>; 3]> = HashMap::new();
    let mut id_stmt = db.prepare("SELECT participant_id, identifier, identifier_type FROM participant_identifiers")?;
    let id_rows =
        id_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    for row in id_rows {
        let (pid, val, kind) = row?;
        let slot = match kind.as_str() {
            "username" => 0,
            "email" => 1,
            "phone" => 2,
            _ => continue,
        };
        ids.entry(pid).or_default()[slot].get_or_insert(val);
    }
    let mut stmt = db.prepare(
        "SELECT room_id, id, full_name, nickname, img_url, is_self, cannot_message, is_network_bot, is_admin FROM participants
         WHERE COALESCE(has_exited, 0) = 0 ORDER BY rowid",
    )?;
    let mut people: HashMap<String, Vec<User>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        let id: String = r.get(1)?;
        let [username, email, phone_number] = ids.get(&id).cloned().unwrap_or_default();
        let name: Option<String> = r.get::<_, Option<String>>(2)?.filter(|n| !n.is_empty());
        let nickname: Option<String> = r.get::<_, Option<String>>(3)?.filter(|n| !n.is_empty());
        let user = User {
            full_name: Some(name.or_else(|| nickname.clone()).unwrap_or_else(|| id.clone())),
            id,
            username,
            email,
            phone_number,
            img_url: r.get(4)?,
            is_self: Some(r.get::<_, Option<i64>>(5)? == Some(1)),
            cannot_message: Some(r.get::<_, Option<i64>>(6)? == Some(1)),
            custom_nickname: nickname,
            is_network_bot: Some(r.get::<_, Option<i64>>(7)? == Some(1)),
            is_admin: Some(r.get::<_, Option<i64>>(8)? == Some(1)),
        };
        Ok((r.get::<_, String>(0)?, user))
    })?;
    for row in rows {
        let (room, user) = row?;
        people.entry(room).or_default().push(user);
    }
    Ok(people)
}

fn display_name(u: &User) -> &str {
    u.custom_nickname.as_deref().or(u.full_name.as_deref()).expect("imported participants have a name")
}

fn str_field(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).is_some_and(|x| !matches!(x, Value::Null | Value::Bool(false)) && x.as_i64() != Some(0))
}

fn read_reaction_preview(t: &Value, id: &str, account_id: &str, group: bool, items: &[User]) -> Option<Message> {
    let plm = t.get("partialLastMessage")?;
    let key = str_field(plm.get("action")?, "reactionKey")?;
    let sender_id = str_field(plm, "senderID")?;
    let is_sender = flag(plm, "isSender");
    let sender_name = items.iter().find(|u| u.id == sender_id).map(|u| display_name(u).to_string());
    let who = match is_sender {
        true => Some("You"),
        false => group.then(|| sender_name.as_deref().unwrap_or(sender_id.as_str())),
    };
    let target = plm.get("linkedMessage").map(|l| Message {
        id: String::new(),
        account_id: String::new(),
        chat_id: String::new(),
        sender_id: String::new(),
        sender_name: None,
        sort_key: String::new(),
        timestamp: String::new(),
        attachments: l
            .get("attachments")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |a| a.iter().map(read_attachment).collect()),
        is_sender: Some(flag(l, "isSender")),
        is_unread: None,
        is_bookmarked: None,
        is_deleted: None,
        linked_message_id: None,
        reactions: Vec::new(),
        text: str_field(l, "text"),
        html: None,
        message_type: None,
        poll: None,
        unresolved_media: None,
        edit_history: Vec::new(),
        links: Vec::new(),
        seen: Default::default(),
        flags: 0,
    });
    let text = Some(crate::matrix::reaction_text(&key, target.as_ref(), who));
    Some(Message {
        id: str_field(plm, "id")?,
        account_id: account_id.into(),
        chat_id: id.into(),
        sender_id,
        sender_name,
        sort_key: crate::store::sort_key(plm.get("sortKey")?.as_i64()?),
        timestamp: str_field(plm, "timestamp")?,
        attachments: Vec::new(),
        is_sender: Some(is_sender),
        is_unread: Some(false),
        is_bookmarked: Some(false),
        is_deleted: None,
        linked_message_id: str_field(plm, "linkedMessageID").or_else(|| str_field(plm.get("linkedMessage")?, "id")),
        reactions: Vec::new(),
        text,
        html: None,
        message_type: Some("REACTION".into()),
        poll: None,
        unresolved_media: None,
        edit_history: Vec::new(),
        links: Vec::new(),
        seen: Default::default(),
        flags: 0,
    })
}

fn read_chats(db: &Connection, people: &mut HashMap<String, Vec<User>>, rules: &Value) -> Result<Vec<Chat>, String> {
    let mut stmt = db
        .prepare(
            "SELECT threadID, accountID, CAST(thread AS TEXT), timestamp FROM threads
             WHERE COALESCE(is_label, 0) = 0 AND json_extract(CAST(thread AS TEXT), '$.extra.roomType') IS NULL
             AND json_extract(CAST(thread AS TEXT), '$.extra.isNoteToSelf') IS NOT 1
             ORDER BY rowid",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|(id, account, json, ts)| {
            let t: Value = serde_json::from_str(&json).map_err(|e| format!("thread {id}: {e}"))?;
            let items = people.remove(&id).unwrap_or_default();
            let group = t.get("type").and_then(Value::as_str) == Some("group");
            let others = items.iter().filter(|u| u.is_self != Some(true)).map(display_name).collect::<Vec<_>>();
            let title = str_field(&t, "title").unwrap_or_else(|| match others.as_slice() {
                [] => "Unnamed chat".into(),
                [one] => one.to_string(),
                many if group => many.iter().take(3).copied().collect::<Vec<_>>().join(", "),
                [first, ..] => first.to_string(),
            });
            let account_id = account.or_else(|| str_field(&t, "accountID")).unwrap_or_else(|| "hungryserv".into());
            let bridge = t.pointer("/extra/bridge").unwrap_or(&Value::Null);
            let preview = read_reaction_preview(&t, &id, &account_id, group, &items);
            let ms = t.get("timestamp").and_then(Value::as_i64).or(ts);
            let last_activity = preview.as_ref().map(|p| p.timestamp.clone()).or_else(|| ms.map(rfc3339_from_millis));
            let count = |k: &str| t.get(k).and_then(Value::as_u64).map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
            Ok(Chat {
                is_hidden_from_inbox: Some(crate::matrix::hidden_from_inbox(rules, &account_id, bridge, group)),
                account_id,
                participants: ChatParticipants { has_more: false, total: items.len(), items },
                title,
                chat_type: if group { "group" } else { "single" }.into(),
                unread_count: count("unreadCount"),
                unread_mentions: count("unreadMentionsCount"),
                description: str_field(&t, "description"),
                draft: None,
                is_archived: Some(flag(&t, "isArchived")),
                is_marked_unread: Some(flag(&t, "isMarkedUnread")),
                is_muted: Some(flag(&t, "mutedUntil")),
                is_pinned: Some(flag(&t, "isPinned")),
                is_low_priority: Some(flag(&t, "isLowPriority")),
                label_ids: Vec::new(),
                last_activity,
                last_read_message_sort_key: t
                    .get("lastReadMessageSortKey")
                    .and_then(Value::as_i64)
                    .map(crate::store::sort_key),
                local_chat_id: None,
                merge: None,
                merged_into_chat_id: None,
                preview,
                reminder: None,
                chat_theme: None,
                notifiable: None,
                is_request: Some(flag(&t, "isMessageRequest")),
                network_name: str_field(&t, "networkName")
                    .or_else(|| bridge.get("network").and_then(|n| str_field(n, "displayname"))),
                network_id: bridge.get("network").and_then(|n| str_field(n, "id")),
                img_url: str_field(&t, "imgURL"),
                delete_modes: 0,
                formatting: ALL_FORMATS,
                flags: 0,
                id,
            })
        })
        .collect()
}

fn read_attachment(a: &Value) -> Attachment {
    let kind = match a.get("type").and_then(Value::as_str) {
        Some("img") => "img",
        Some("video") => "video",
        Some("audio") => "audio",
        _ => "unknown",
    };
    let voice = flag(a, "isVoiceNote");
    let default_name = match (voice, kind) {
        (true, _) => "Voice message",
        (_, "img") => "Image",
        (_, "video") => "Video",
        (_, "audio") => "Audio",
        _ => "File",
    };
    let size = a.get("size").map(|s| AttachmentSize {
        width: s.get("width").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        height: s.get("height").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
    });
    Attachment {
        attachment_type: kind.into(),
        id: str_field(a, "id"),
        duration: a
            .get("duration")
            .and_then(Value::as_f64)
            .or_else(|| a.pointer("/extra/duration").and_then(Value::as_f64).map(|ms| ms / 1000.0)),
        file_name: Some(str_field(a, "fileName").unwrap_or_else(|| default_name.into())),
        file_size: a.get("fileSize").and_then(Value::as_u64),
        is_gif: Some(flag(a, "isGif")),
        is_sticker: Some(flag(a, "isSticker")),
        is_voice_note: Some(voice),
        mime_type: str_field(a, "mimeType"),
        poster_img: str_field(a, "posterImg"),
        size,
        src_url: str_field(a, "srcURL"),
        transcript: None,
    }
}

fn read_messages(db: &Connection, chats: &[Chat]) -> Result<Vec<Message>, String> {
    let names: HashMap<(&str, &str), &str> = chats
        .iter()
        .flat_map(|c| c.participants.items.iter().map(move |u| ((c.id.as_str(), u.id.as_str()), display_name(u))))
        .collect();
    let accounts: HashMap<&str, &str> = chats.iter().map(|c| (c.id.as_str(), c.account_id.as_str())).collect();
    let unread_chats: HashMap<&str, Option<&str>> = chats
        .iter()
        .filter(|c| c.unread_count > 0 || c.is_marked_unread == Some(true))
        .map(|c| (c.id.as_str(), c.last_read_message_sort_key.as_deref()))
        .collect();
    let mut receipts: HashMap<(String, String), BTreeMap<String, String>> = HashMap::new();
    db.prepare(
        "SELECT roomID, COALESCE(NULLIF(lastVisibleEventID, ''), relatesToID), userID, timestamp FROM mx_read_receipts",
    )
    .and_then(|mut q| {
        q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?
        .try_for_each(|x| {
            let (room, ev, user, ts) = x?;
            if let Some(ev) = ev.filter(|e| !e.is_empty()) {
                receipts.entry((room, ev)).or_default().insert(user, rfc3339_from_millis(ts.unwrap_or(0)));
            }
            Ok(())
        })
    })
    .map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT messageID, threadID, timestamp, CAST(message AS TEXT), NULL, 0 FROM messages
             UNION ALL
             SELECT eventID, roomID, timestamp, CAST(message AS TEXT), hsOrder, isDeleted FROM mx_room_messages
             WHERE type NOT IN ('REACTION', 'MEMBERSHIP', 'HIDDEN')
             ORDER BY 3, 5",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| e.to_string())?;
    let mut reactions: HashMap<String, Vec<Reaction>> = HashMap::new();
    db.prepare(
        "SELECT eventID, reactionID, senderID, description, \"order\" FROM mx_reactions WHERE isDeleted = 0 ORDER BY \"order\"",
    )
    .and_then(|mut q| {
        q.query_map([], |r| {
            let reaction = Reaction {
                id: r.get(1)?,
                participant_id: r.get(2)?,
                reaction_key: r.get(3)?,
                animated: None,
                img_url: None,
                sort_key: crate::store::sort_key(r.get(4)?),
            };
            Ok((r.get::<_, String>(0)?, reaction))
        })?
        .try_for_each(|x| x.map(|(target, r)| reactions.entry(target).or_default().push(r)))
    })
    .map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (id, chat_id, ts, json, order, deleted) in rows {
        let Some(&account_id) = accounts.get(chat_id.as_str()) else { continue };
        let m: Value = serde_json::from_str(&json).map_err(|e| format!("message {id}: {e}"))?;
        if flag(&m, "isHidden") || !seen.insert(id.clone()) {
            continue;
        }
        let deleted = deleted || flag(&m, "isDeleted");
        let sender_id =
            m.get("senderID").map_or_else(String::new, |s| s.as_str().map_or_else(|| s.to_string(), str::to_string));
        let sender_name = m
            .pointer("/extra/impersonator/fullName")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| names.get(&(chat_id.as_str(), sender_id.as_str())).map(|n| n.to_string()))
            .unwrap_or_else(|| sender_id.clone());
        let attachments: Vec<Attachment> = match deleted {
            true => Vec::new(),
            false => m
                .get("attachments")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |a| a.iter().map(read_attachment).collect()),
        };
        let mut listed = m.get("reactions").and_then(Value::as_array).map_or_else(Vec::new, |rs| {
            rs.iter()
                .filter_map(|r| {
                    Some(Reaction {
                        id: str_field(r, "id")?,
                        participant_id: str_field(r, "participantID")?,
                        reaction_key: str_field(r, "reactionKey")?,
                        animated: None,
                        img_url: None,
                        sort_key: String::new(),
                    })
                })
                .collect()
        });
        listed.extend(reactions.remove(&id).into_iter().flatten());
        if deleted {
            listed.clear();
        }
        let message_type = match attachments.first() {
            Some(a) if a.is_voice_note == Some(true) => "VOICE",
            Some(a) if a.attachment_type == "img" => "IMAGE",
            Some(a) if a.attachment_type == "video" => "VIDEO",
            Some(a) if a.attachment_type == "audio" => "AUDIO",
            Some(_) => "FILE",
            None => "TEXT",
        };
        let sort_key = order.map_or_else(String::new, crate::store::sort_key);
        let is_sender = flag(&m, "isSender");
        let is_unread = !is_sender
            && !deleted
            && unread_chats
                .get(chat_id.as_str())
                .is_some_and(|last| last.is_some_and(|k| !sort_key.is_empty() && sort_key.as_str() > k));
        let seen_by = receipts.remove(&(chat_id.clone(), id.clone())).unwrap_or_default();
        out.push(Message {
            account_id: account_id.into(),
            chat_id,
            sender_name: Some(sender_name),
            sender_id,
            sort_key,
            timestamp: rfc3339_from_millis(ts),
            attachments,
            is_sender: Some(is_sender),
            is_unread: Some(is_unread),
            is_bookmarked: Some(false),
            is_deleted: deleted.then_some(true),
            linked_message_id: str_field(&m, "linkedMessageID"),
            reactions: listed,
            text: str_field(&m, "text").filter(|_| !deleted),
            html: m.get("extra").and_then(|e| str_field(e, "sanitizedFormattedBody")).filter(|_| !deleted),
            message_type: Some(message_type.into()),
            poll: None,
            unresolved_media: m
                .pointer("/extra/lastEditionID")
                .and_then(Value::as_str)
                .is_none()
                .then(|| m.pointer("/extra/unresolvedMedia").and_then(Value::as_str).map(str::to_string))
                .flatten()
                .filter(|_| !deleted),
            edit_history: Vec::new(),
            links: m
                .get("links")
                .and_then(Value::as_array)
                .filter(|_| !deleted)
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    Some(MessageLink {
                        url: str_field(l, "url")?,
                        title: str_field(l, "title").unwrap_or_default(),
                        summary: str_field(l, "description"),
                    })
                })
                .collect(),
            seen: seen_by,
            flags: 0,
            id,
        });
    }
    for m in &mut out {
        m.linked_message_id = m.linked_message_id.take().filter(|l| seen.contains(l));
    }
    Ok(out)
}

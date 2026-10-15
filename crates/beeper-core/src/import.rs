use crate::config::rfc3339_from_millis;
use crate::models::{
    Attachment, AttachmentSize, Chat, ChatParticipants, MatrixSession, Message, Profile, Reaction, User,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const SNAPSHOT_FILES: [&str; 4] = ["index.db", "index.db-wal", "account.db", "account.db-wal"];

#[derive(Debug, Clone, Default)]
pub struct BeeperData {
    pub session: Option<(MatrixSession, String)>,
    pub profile: Profile,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
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
        .and_then(|()| read_snapshot(&tmp).map_err(|e| format!("read Beeper Desktop data in {}: {e}", dir.display())));
    std::fs::remove_dir_all(&tmp).unwrap_or_else(|e| panic!("remove import snapshot {}: {e}", tmp.display()));
    out
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

fn read_snapshot(dir: &Path) -> Result<BeeperData, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let session = match dir.join("account.db").is_file() {
        true => read_session(&open(&dir.join("account.db")).map_err(e)?)?,
        false => None,
    };
    let db = open(&dir.join("index.db")).map_err(e)?;
    let profile = read_profile(&db).map_err(e)?;
    let mut people = read_participants(&db).map_err(e)?;
    let chats = read_chats(&db, &mut people)?;
    let messages = read_messages(&db, &chats)?;
    Ok(BeeperData { session, profile, chats, messages })
}

fn read_session(db: &Connection) -> Result<Option<(MatrixSession, String)>, String> {
    let mut stmt =
        db.prepare("SELECT user_id, device_id, access_token, homeserver FROM account").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?, r.get(3)?)))
        .and_then(Iterator::collect::<rusqlite::Result<Vec<(String, String, _, String)>>>)
        .map_err(|e| e.to_string())?;
    match rows.as_slice() {
        [] => Ok(None),
        [(user_id, device_id, Some(token), homeserver)] if !token.is_empty() => Ok(Some((
            MatrixSession { user_id: user_id.clone(), access_token: token.clone(), device_id: device_id.clone() },
            homeserver.clone(),
        ))),
        [_] => Ok(None),
        more => Err(format!("account.db has {} accounts", more.len())),
    }
}

fn read_profile(db: &Connection) -> rusqlite::Result<Profile> {
    let raw: Option<String> = db
        .query_row("SELECT CAST(value AS TEXT) FROM key_values WHERE key = 'userProfile'", [], |r| r.get(0))
        .optional()?;
    let v: Value = raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let field = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(Profile { full_name: field("fullName"), username: field("username"), email: field("email") })
}

fn read_participants(db: &Connection) -> rusqlite::Result<HashMap<String, Vec<User>>> {
    let mut stmt = db.prepare(
        "SELECT room_id, id, full_name, nickname, img_url, is_self, cannot_message FROM participants
         WHERE COALESCE(has_exited, 0) = 0 ORDER BY rowid",
    )?;
    let mut people: HashMap<String, Vec<User>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        let id: String = r.get(1)?;
        let name: Option<String> = r.get::<_, Option<String>>(2)?.filter(|n| !n.is_empty());
        let nickname: Option<String> = r.get::<_, Option<String>>(3)?.filter(|n| !n.is_empty());
        let user = User {
            full_name: Some(name.or_else(|| nickname.clone()).unwrap_or_else(|| id.clone())),
            id,
            username: None,
            email: None,
            phone_number: None,
            img_url: r.get(4)?,
            is_self: Some(r.get::<_, Option<i64>>(5)? == Some(1)),
            cannot_message: Some(r.get::<_, Option<i64>>(6)? == Some(1)),
            custom_nickname: nickname,
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

fn read_chats(db: &Connection, people: &mut HashMap<String, Vec<User>>) -> Result<Vec<Chat>, String> {
    let mut stmt = db
        .prepare(
            "SELECT threadID, accountID, CAST(thread AS TEXT), timestamp FROM threads
             WHERE COALESCE(is_label, 0) = 0 ORDER BY timestamp",
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
            let ms = t.get("timestamp").and_then(Value::as_i64).or(ts);
            let count = |k: &str| t.get(k).and_then(Value::as_u64).map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
            Ok(Chat {
                account_id: account.or_else(|| str_field(&t, "accountID")).unwrap_or_else(|| "hungryserv".into()),
                participants: ChatParticipants { has_more: false, total: items.len(), items },
                title,
                chat_type: if group { "group" } else { "single" }.into(),
                unread_count: count("unreadCount"),
                description: str_field(&t, "description"),
                draft: None,
                is_archived: Some(flag(&t, "isArchived")),
                is_marked_unread: Some(flag(&t, "isMarkedUnread")),
                is_muted: Some(flag(&t, "mutedUntil")),
                is_pinned: Some(flag(&t, "isPinned")),
                is_low_priority: Some(flag(&t, "isLowPriority")),
                label_ids: Vec::new(),
                last_activity: ms.map(rfc3339_from_millis),
                last_read_message_sort_key: None,
                local_chat_id: None,
                merge: None,
                merged_into_chat_id: None,
                preview: None,
                reminder: None,
                chat_theme: None,
                is_hidden_from_inbox: None,
                network_name: None,
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
        duration: a.get("duration").and_then(Value::as_f64),
        file_name: Some(str_field(a, "fileName").unwrap_or_else(|| default_name.into())),
        file_size: a.get("fileSize").and_then(Value::as_u64),
        is_gif: Some(flag(a, "isGif")),
        is_sticker: Some(flag(a, "isSticker")),
        is_voice_note: Some(voice),
        mime_type: str_field(a, "mimeType"),
        poster_img: None,
        size,
        src_url: None,
        transcript: None,
    }
}

fn read_messages(db: &Connection, chats: &[Chat]) -> Result<Vec<Message>, String> {
    let names: HashMap<(&str, &str), &str> = chats
        .iter()
        .flat_map(|c| c.participants.items.iter().map(move |u| ((c.id.as_str(), u.id.as_str()), display_name(u))))
        .collect();
    let accounts: HashMap<&str, &str> = chats.iter().map(|c| (c.id.as_str(), c.account_id.as_str())).collect();
    let mut stmt = db
        .prepare("SELECT messageID, threadID, timestamp, CAST(message AS TEXT) FROM messages ORDER BY timestamp, rowid")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (id, chat_id, ts, json) in rows {
        let Some(&account_id) = accounts.get(chat_id.as_str()) else { continue };
        let m: Value = serde_json::from_str(&json).map_err(|e| format!("message {id}: {e}"))?;
        if flag(&m, "isHidden") || flag(&m, "isDeleted") || !seen.insert(id.clone()) {
            continue;
        }
        let sender_id =
            m.get("senderID").map_or_else(String::new, |s| s.as_str().map_or_else(|| s.to_string(), str::to_string));
        let sender_name = m
            .pointer("/extra/impersonator/fullName")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| names.get(&(chat_id.as_str(), sender_id.as_str())).map(|n| n.to_string()))
            .unwrap_or_else(|| sender_id.clone());
        let attachments: Vec<Attachment> = m
            .get("attachments")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |a| a.iter().map(read_attachment).collect());
        let reactions = m.get("reactions").and_then(Value::as_array).map_or_else(Vec::new, |rs| {
            rs.iter()
                .filter_map(|r| {
                    Some(Reaction {
                        id: str_field(r, "id")?,
                        participant_id: str_field(r, "participantID")?,
                        reaction_key: str_field(r, "reactionKey")?,
                        animated: None,
                        img_url: None,
                    })
                })
                .collect()
        });
        let message_type = match attachments.first() {
            Some(a) if a.is_voice_note == Some(true) => "VOICE",
            Some(a) if a.attachment_type == "img" => "IMAGE",
            Some(a) if a.attachment_type == "video" => "VIDEO",
            Some(a) if a.attachment_type == "audio" => "AUDIO",
            Some(_) => "FILE",
            None => "TEXT",
        };
        out.push(Message {
            account_id: account_id.into(),
            chat_id,
            sender_name: Some(sender_name),
            sender_id,
            sort_key: String::new(),
            timestamp: rfc3339_from_millis(ts),
            attachments,
            is_sender: Some(flag(&m, "isSender")),
            is_unread: Some(false),
            is_bookmarked: Some(false),
            linked_message_id: str_field(&m, "linkedMessageID"),
            reactions,
            text: str_field(&m, "text"),
            message_type: Some(message_type.into()),
            poll: None,
            edit_history: Vec::new(),
            links: Vec::new(),
            flags: 0,
            id,
        });
    }
    for m in &mut out {
        m.linked_message_id = m.linked_message_id.take().filter(|l| seen.contains(l));
    }
    Ok(out)
}

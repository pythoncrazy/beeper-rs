//! Desktop's Storage settings: the local messages and the cached media, by type, chat and age.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OpenFlags};

use crate::models::Attachment;
use crate::store::SharedStore;

/// Desktop's `IMAGE`, `VIDEO`, `FILE`, `VOICE` and `MESSAGE` storage types, then its temporary cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Image,
    Video,
    File,
    Voice,
    Message,
    Cache,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Image, Kind::Video, Kind::File, Kind::Voice, Kind::Message, Kind::Cache];

    fn of(a: &Attachment) -> Kind {
        match a.attachment_type.as_str() {
            "img" => Kind::Image,
            "video" => Kind::Video,
            "audio" => Kind::Voice,
            _ => Kind::File,
        }
    }
}

/// One stored message, cached media file or uploaded file.
#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub chat: Option<String>,
    /// Unix seconds when a message was sent, or when a file was last used.
    pub secs: i64,
    pub bytes: u64,
    /// The message id, the `mxc://` URI of cached media, or the upload id.
    key: String,
}

/// Everything that is stored on this device.
pub fn scan(store: &SharedStore) -> Vec<Entry> {
    let st = store.read();
    let mut owner = HashMap::new();
    for &i in st.msg_idx_by_id.values() {
        let m = &st.messages[i];
        for a in &m.attachments {
            if let Some(uri) =
                a.src_url.as_deref().and_then(|s| s.split('?').next()).filter(|s| s.starts_with("mxc://"))
            {
                owner.insert(uri.to_string(), (Kind::of(a), m.chat_id.clone()));
            }
        }
    }
    let message = |(key, chat, secs, bytes)| Entry { kind: Kind::Message, chat: Some(chat), secs, bytes, key };
    let mut all: Vec<Entry> = st.message_sizes().into_iter().map(message).collect();
    all.extend(st.assets.values().map(|a| Entry {
        kind: Kind::Cache,
        chat: None,
        secs: a.created_epoch_secs,
        bytes: a.file_size,
        key: a.upload_id.clone(),
    }));
    let path = st.matrix_dir().join("matrix-sdk-media.sqlite3");
    drop(st);
    if !path.exists() {
        return all;
    }
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open the media store");
    let mut q = db.prepare("SELECT uri, length(data), last_access FROM media").expect("prepare the media query");
    let rows = q.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?, r.get(2)?))).expect("media");
    for row in rows {
        let (uri, bytes, secs) = row.expect("read a media row");
        let key = String::from_utf8(uri).expect("the media store has plain keys");
        assert!(key.starts_with("mxc://"), "media key {key} is not an mxc URI");
        let (kind, chat) = owner.get(&key).cloned().map_or((Kind::Cache, None), |(k, c)| (k, Some(c)));
        all.push(Entry { kind, chat, secs, bytes: bytes as u64, key });
    }
    all
}

/// The entries of the `kinds` in `chat` (or in all chats) that are older than `days`, or of any age for 0.
pub fn select<'a>(all: &'a [Entry], kinds: &[Kind], chat: Option<&str>, days: i64, now_secs: i64) -> Vec<&'a Entry> {
    let cutoff = now_secs - days * 86_400;
    all.iter()
        .filter(|e| kinds.contains(&e.kind) && chat.is_none_or(|c| e.chat.as_deref() == Some(c)))
        .filter(|e| days == 0 || e.secs < cutoff)
        .collect()
}

/// Deletes the entries from this device. Messages come back when the account is synced again.
pub async fn clear(store: SharedStore, entries: Vec<Entry>) -> Result<(), String> {
    let (mut messages, mut uploads, mut media) = (HashSet::new(), Vec::new(), Vec::new());
    for e in entries {
        match e.kind {
            Kind::Message => drop(messages.insert(e.key)),
            _ if e.key.starts_with("mxc://") => media.push(e.key),
            _ => uploads.push(e.key),
        }
    }
    if !media.is_empty() {
        let client = store.read().matrix.clone().ok_or("Not connected to Matrix")?;
        for uri in media {
            client.media().remove_media_content_for_uri(uri.as_str().into()).await.map_err(|e| e.to_string())?;
        }
    }
    store.write().forget(&messages, &uploads);
    Ok(())
}

use crate::config::{
    assert_not_original_beeper_dir, millis_from_rfc3339, now_rfc3339_millis, now_unix_secs, rfc3339_from_millis,
    CentralConfig, DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE, DEFAULT_TOKEN, DEFAULT_USER_ID,
    SEED_JSON, WS_CHANNEL_CAPACITY,
};
use crate::matrix::{MediaInfo, Mention, Outgoing, ThreadSend};
use crate::models::*;
use crate::Error;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::broadcast;

pub type SharedStore = std::sync::Arc<StoreLock>;

thread_local!(static LOCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

/// A std `RwLock` deadlocks when a thread that holds the store locks it again while a writer waits,
/// so a nested lock on one thread panics. The generation counts the starts and ends of write locks, so that a
/// reader can keep values that it computed from the store until the next write.
pub struct StoreLock(std::sync::RwLock<BeeperStore>, AtomicU64);

/// A held lock, with the generation that it moves when it is a write lock.
pub struct Locked<'a, G>(G, Option<&'a AtomicU64>);

impl StoreLock {
    pub fn shared(store: BeeperStore) -> SharedStore {
        std::sync::Arc::new(Self(std::sync::RwLock::new(store), AtomicU64::new(0)))
    }

    fn hold<'a, G>(lock: impl FnOnce() -> G, writes: Option<&'a AtomicU64>) -> Locked<'a, G> {
        assert!(!LOCKED.replace(true), "the store is already locked on this thread");
        let held = Locked(lock(), writes);
        writes.map(|w| w.fetch_add(1, Ordering::Release));
        held
    }

    pub fn read(&self) -> Locked<'_, std::sync::RwLockReadGuard<'_, BeeperStore>> {
        Self::hold(|| self.0.read().expect("store lock"), None)
    }

    pub fn write(&self) -> Locked<'_, std::sync::RwLockWriteGuard<'_, BeeperStore>> {
        Self::hold(|| self.0.write().expect("store lock"), Some(&self.1))
    }

    /// The generation of the store, or `None` while a write lock can change it.
    pub fn generation(&self) -> Option<u64> {
        Some(self.1.load(Ordering::Acquire)).filter(|g| g % 2 == 0)
    }
}

impl<G> Drop for Locked<'_, G> {
    fn drop(&mut self) {
        self.1.map(|w| w.fetch_add(1, Ordering::Release));
        LOCKED.set(false);
    }
}

impl<G: std::ops::Deref> std::ops::Deref for Locked<'_, G> {
    type Target = G::Target;
    fn deref(&self) -> &G::Target {
        &self.0
    }
}

impl<G: std::ops::DerefMut> std::ops::DerefMut for Locked<'_, G> {
    fn deref_mut(&mut self) -> &mut G::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAsset {
    pub upload_id: String,
    pub mxc_url: String,
    pub disk_name: String,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub created_epoch_secs: i64,
}

pub const NOTE_CHAT_ID: &str = "!note_self:beeper.com";
/// Logout writes this file and the next start deletes the local data, because
/// Windows can't delete the databases while this process has them open.
pub const LOGGED_OUT: &str = "logged-out";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    bridges: Vec<BridgeAccountInfo>,
    chats: Vec<Chat>,
    messages: Vec<Message>,
    labels: Vec<SeedLabel>,
}

/// A demo label of the seed. The chats of the label have its ID in their `labelIDs`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedLabel {
    id: String,
    name: String,
    color: Option<u8>,
    icon: Option<String>,
}

const SCHEMA_VERSION: i64 = 10;
const SCHEMA: &str = "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
     CREATE TABLE chats (id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
     CREATE TABLE assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;";

/// The messages table with its chat index and Desktop's full-text search table `mx_room_messages_fts`. The search
/// text is the message text, file names and voice note transcripts. `n` is the search row: a plain rowid can change
/// on `VACUUM`.
fn messages_schema() -> String {
    format!(
        "CREATE TABLE messages (n INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, chat_id TEXT NOT NULL, \
         sort_key TEXT NOT NULL, json TEXT NOT NULL) STRICT;
         CREATE INDEX messages_by_chat ON messages (chat_id, sort_key);
         CREATE VIRTUAL TABLE messages_fts USING fts5(text, content='', contentless_delete=1);
         CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
             DELETE FROM messages_fts WHERE rowid = old.n;
         END; {}",
        search_triggers()
    )
}
/// The triggers that index the search text of a new or changed message.
fn search_triggers() -> String {
    let new = "concat_ws(' ', new.json ->> '$.text', (SELECT group_concat(concat_ws(' ', a.value ->> '$.fileName', \
               a.value ->> '$.transcription.transcription'), ' ') FROM json_each(new.json, '$.attachments') a))";
    format!(
        "CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
             INSERT INTO messages_fts (rowid, text) VALUES (new.n, {new});
         END;
         CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
             DELETE FROM messages_fts WHERE rowid = old.n;
             INSERT INTO messages_fts (rowid, text) VALUES (new.n, {new});
         END;"
    )
}
/// Schema 9 keeps a transcript as Desktop's `transcription` object, with the engine `unknown` of Desktop's `p`.
fn transcriptions() -> String {
    let old = "'$.attachments[0].transcript'";
    format!(
        "DROP TRIGGER messages_ai; DROP TRIGGER messages_au; {}
         UPDATE messages SET json = json_set(json_remove(json, {old}), '$.attachments[0].transcription',
             json_object('transcription', json ->> {old}, 'engine', 'unknown')) WHERE json ->> {old} IS NOT NULL;",
        search_triggers()
    )
}
/// Schema 6 adds the outbox: the JSON of each `Outgoing`, and the file of a media message.
const OUTBOX: &str =
    "CREATE TABLE outbox (id INTEGER PRIMARY KEY, chat_id TEXT NOT NULL, json TEXT NOT NULL, bytes BLOB) STRICT;";
/// Schema 7 changes the cross-signing and backup secrets of the state from texts to flags, like Desktop's.
fn secret_flags() -> String {
    let flag = |k: &str| format!("'$.secrets.{k}', json(iif(value ->> '$.secrets.{k}' IN ('', 0), 'false', 'true'))");
    let keys = ["master_key", "self_signing_key", "user_signing_key", "megolm_backup_key"].map(flag).join(", ");
    format!("UPDATE kv SET value = json_set(value, {keys}) WHERE key = '{KEY_STATE}';")
}
/// The messages with attachments, links or URLs in the text.
const MEDIA: &str = "(json ->> '$.attachments[0]' IS NOT NULL OR json ->> '$.links[0]' IS NOT NULL OR json ->> '$.text' LIKE '%http%://%')";
/// Schema 8 orders all messages, the messages with media and the thread replies, which the store reads from SQLite.
fn order_schema() -> String {
    format!(
        "CREATE INDEX messages_by_key ON messages (sort_key);
         CREATE INDEX messages_with_media ON messages (sort_key) WHERE {MEDIA};
         CREATE INDEX messages_by_thread ON messages ({THREAD}, sort_key) WHERE {THREAD} IS NOT NULL;"
    )
}
/// The time of a message: an RFC 3339 time with milliseconds in UTC, which sorts like the time.
const TIME: &str = "(json ->> '$.timestamp')";
/// Schema 10 orders the messages by time and row, like Desktop's `searchRoomMessages`.
fn time_schema() -> String {
    format!("CREATE INDEX messages_by_time ON messages ({TIME}, n);")
}
/// The root of a thread reply.
const THREAD: &str = "(json ->> '$.threadRootEventID')";
/// The most messages that stay loaded when a chat loads.
const LOADED_MESSAGES: usize = 5_000;
const KEY_STATE: &str = "state";
const KEY_BRIDGES: &str = "bridges";
const KEY_SETTINGS: &str = "settings";
const KEY_PIN_RANK: &str = "pinRank";
const KEY_RECENT_CHATS: &str = "recentChats";
const KEY_SCHEDULED: &str = "scheduledMessages";
const KEY_CONTACTS: &str = "contacts";
const KEY_SEQ: &str = "nextSeq";
const KEY_SETUP: &str = "setup";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    Fresh,
    Imported,
}

impl Setup {
    fn key(self) -> &'static str {
        match self {
            Setup::Fresh => "fresh",
            Setup::Imported => "imported",
        }
    }
}

/// A text key that sorts like `seq`. Beeper puts events before the history at `Number.MIN_SAFE_INTEGER`.
pub fn sort_key(seq: i64) -> String {
    const SPAN: i64 = 10_000_000_000_000_000;
    assert!(seq.abs() < SPAN, "sort key {seq} does not fit in 16 digits");
    if seq < 0 {
        format!("-{:016}", SPAN + seq)
    } else {
        format!("{seq:016}")
    }
}

/// The order of a [`sort_key`].
pub fn order_of(key: &str) -> i64 {
    const SPAN: i64 = 10_000_000_000_000_000;
    match key.strip_prefix('-') {
        Some(rest) => rest.parse::<i64>().expect("numeric sort key") - SPAN,
        None => key.parse().expect("numeric sort key"),
    }
}

/// SQL that changes the 12-digit sort keys of schema 2 to the 16-digit keys of schema 3. It runs again for
/// schema 4 because the demo seed kept 12-digit keys until then.
fn widen_sort_keys() -> String {
    let widen = |x: &str| {
        format!(
            "CASE WHEN length(ltrim({x}, '-')) = 16 THEN {x} WHEN substr({x}, 1, 1) = '-' THEN '-9999' || substr({x}, 2) \
             ELSE '0000' || {x} END"
        )
    };
    let chat = |path: &str| {
        let x = format!("json_extract(json, '{path}')");
        format!("UPDATE chats SET json = json_set(json, '{path}', {}) WHERE {x} IS NOT NULL;", widen(&x))
    };
    format!(
        "UPDATE messages SET sort_key = {}; UPDATE messages SET json = json_set(json, '$.sortKey', sort_key); {} {}",
        widen("sort_key"),
        chat("$.preview.sortKey"),
        chat("$.lastReadMessageSortKey")
    )
}

fn demo_state() -> BeeperStateRecord {
    BeeperStateRecord {
        access_token: DEFAULT_TOKEN.to_string(),
        device_id: DEFAULT_DEVICE_ID.to_string(),
        user_id: DEFAULT_USER_ID.to_string(),
        homeserver: DEFAULT_HOMESERVER.to_string(),
        first_sync_done: true,
        initialized: true,
        secret_storage: true,
        cross_signing: true,
        verified: true,
        key_backup: true,
        has_backed_up_code: true,
        secrets: BeeperStateSecrets {
            master_key: true,
            self_signing_key: true,
            user_signing_key: true,
            megolm_backup_key: true,
            recovery_code: DEFAULT_RECOVERY_CODE.to_string(),
        },
        recovery_code_generated_at: 0,
    }
}

/// The key of a chat in `chat_order`: the last activity in Unix milliseconds (0 without activity) and the chat ID.
/// The order of a chat in the list: Desktop's `activityTimestamp`, the newer of its activity and its draft.
pub fn activity_key(c: &Chat) -> (i64, String) {
    let ms = c.last_activity.as_deref().map(|t| millis_from_rfc3339(t).expect("RFC 3339 activity"));
    let draft = c.draft.as_ref().and_then(|d| d.ts);
    (ms.unwrap_or(0).max(draft.unwrap_or(0)), c.id.clone())
}

/// Desktop's `markRemovedFromBeeper` and `deleteRemovedMessageData`: a deleted message without its content.
fn removed_from_beeper(m: &mut Message) {
    (m.is_deleted, m.removed) = (Some(true), true);
    (m.text, m.html, m.poll) = (None, None, None);
    m.edit_history.clear();
    m.attachments.clear();
    m.reactions.clear();
    m.links.clear();
}

/// The SQL conditions of a read of stored messages, with their arguments.
#[derive(Debug, Default)]
pub struct Query(Vec<String>, Vec<String>);

impl Query {
    /// Adds a condition with one `?` for `arg`.
    pub fn and(mut self, cond: &str, arg: impl Into<String>) -> Self {
        self.1.push(arg.into());
        self.0.push(cond.replace('?', &format!("?{}", self.1.len())));
        self
    }

    /// The messages of the chats `ids`.
    pub fn chats(self, ids: &[&str]) -> Self {
        self.and("chat_id IN (SELECT value FROM json_each(?))", serde_json::to_string(ids).expect("chat ID JSON"))
    }

    /// Desktop's message search with partial matches (`searchRoomMessages` with `useFts`): the messages that contain
    /// `text` as a phrase, where the last word can be the start of a word.
    pub fn text(self, text: &str) -> Self {
        assert!(!text.trim().is_empty(), "a message search has words");
        let phrase = format!("\"{}\"*", text.trim().replace('"', "\"\""));
        self.and("n IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)", phrase)
    }

    /// The messages after the sort key `cursor`, or before it.
    pub fn past(self, cursor: Option<&str>, after: bool) -> Self {
        match cursor {
            Some(c) => self.and(if after { "sort_key > ?" } else { "sort_key < ?" }, c),
            None => self,
        }
    }

    /// The messages before the time and the row `n` of a cursor of Desktop's `searchRoomMessages`, or after them.
    pub fn past_row(mut self, (ms, n): (i64, i64), after: bool) -> Self {
        // The times of the years 0 to 9999 have four-digit years, which sort like the times.
        let at = rfc3339_from_millis(ms.clamp(-62_167_219_200_000, 253_402_300_799_999));
        self.1.extend([at, n.to_string()]);
        let op = if after { ">" } else { "<" };
        self.0.push(format!("({TIME}, n) {op} (?{}, CAST(?{} AS INTEGER))", self.1.len() - 1, self.1.len()));
        self
    }

    /// The messages with attachments, links or URLs in the text.
    pub fn media(mut self) -> Self {
        self.0.push(MEDIA.into());
        self
    }
}

/// A row of a thread that counts as unread: its sort key, its event, whether you sent it, and whether it mentions you.
struct ThreadRow {
    key: String,
    id: String,
    mine: bool,
    mention: bool,
}

/// A network inside a bridged account, like a Discord server. Its chats have the same `network_id`.
#[derive(Debug, Clone, Default)]
pub struct NetworkSpace {
    pub id: String,
    pub account_id: String,
    pub name: String,
    /// The network's own picture (`m.bridge` `network.avatar_url`), like a Discord server's icon.
    pub avatar: Option<String>,
}

/// A message to send, as Desktop's composer sends it: the text is one message and each file is one message, except
/// one file with text, which takes the text as its caption. The first message takes the reply.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub text: Option<String>,
    pub reply_to: Option<String>,
    /// The mention pills of the text.
    pub mentions: Vec<Mention>,
    /// The link preview of the text, with its image.
    pub preview: Option<crate::cloud::PagePreview>,
    /// The root of the thread of a thread reply, and Desktop's "Also send to chat".
    pub thread: Option<(String, bool)>,
    pub files: Vec<OutFile>,
    /// Desktop's `links` of the composer: the text has links, whose previews the user may have left out.
    pub links: bool,
}

/// A file to send, with what the sender knows of it. The store makes the attachment of the local echo and the media
/// info for the homeserver from it.
#[derive(Debug, Clone, Default)]
pub struct OutFile {
    pub bytes: Vec<u8>,
    pub name: String,
    pub mime: String,
    /// The width and height of an image or a video.
    pub size: Option<(u32, u32)>,
    /// The length in seconds of a video, an audio file or a voice note.
    pub duration: Option<f64>,
    /// Desktop's `isGif`: a GIF, or the MP4 of a GIF.
    pub gif: bool,
    /// Desktop's `isSticker`.
    pub sticker: bool,
    /// Desktop's `stickerPackImage`: the pack image of a sticker from the sticker picker.
    pub pack: Option<Box<Sticker>>,
    /// The waveform of a recorded voice note, with values from 0 to 1.
    pub voice: Option<Vec<f32>>,
    /// The URL that the local echo shows until the homeserver has the file.
    pub echo_url: Option<String>,
}

impl OutFile {
    /// The media info for the homeserver.
    fn info(&self) -> MediaInfo {
        assert!(self.sticker || self.pack.is_none(), "a sticker pack image is for a sticker");
        match &self.voice {
            Some(wave) => MediaInfo::Voice { ms: (self.duration.unwrap_or(0.0) * 1000.0) as u64, wave: wave.clone() },
            None => MediaInfo::of(&self.mime, self.size, self.duration),
        }
    }

    /// The attachment of the local echo, with the kind that a sync of the sent file shows.
    fn attachment(&self, info: &MediaInfo) -> Attachment {
        let kind = match info {
            MediaInfo::Image(..) => "img",
            MediaInfo::Video(..) => "video",
            MediaInfo::Audio(_) | MediaInfo::Voice { .. } => "audio",
            MediaInfo::File => "unknown",
        };
        Attachment {
            attachment_type: kind.into(),
            duration: self.duration,
            file_name: Some(self.name.clone()),
            file_size: Some(self.bytes.len() as u64),
            is_gif: Some(self.gif),
            is_sticker: Some(self.sticker),
            is_voice_note: Some(self.voice.is_some()),
            mime_type: Some(self.mime.clone()),
            size: self.size.map(|(w, h)| AttachmentSize { width: Some(w), height: Some(h) }),
            src_url: self.echo_url.clone(),
            ..Attachment::default()
        }
    }
}

pub struct BeeperStore {
    pub config: CentralConfig,
    pub setup: Option<Setup>,
    db: Mutex<Connection>,
    pub beeper_state: BeeperStateRecord,
    /// Desktop's `AppStateTracker`: the last trusted device verification.
    pub verification: Option<AppVerification>,
    pub bridge_accounts: Vec<BridgeAccountInfo>,
    /// Position of each pinned chat in the `com.beeper.favourites_order` account data.
    pub pin_rank: HashMap<String, usize>,
    pub recent_chat_ids: Vec<String>,
    pub network_spaces: Vec<NetworkSpace>,
    pub contacts: Vec<User>,
    pub bridge_sessions: HashMap<String, BridgeLoginSession>,
    pub accounts: Vec<Account>,
    pub labels: Vec<Label>,
    pub chats: Vec<Chat>,
    pub(crate) chat_idx_by_id: HashMap<String, usize>,
    /// The `activity_key` of each chat, for Desktop's chat order `ORDER BY timestamp DESC`.
    chat_order: BTreeSet<(i64, String)>,
    local_chat_idx_by_id: HashMap<String, usize>,
    /// The messages of the loaded chats. SQLite keeps all messages, and `load_chat` loads the messages of a chat.
    pub messages: Vec<Message>,
    pub(crate) msg_idx_by_id: HashMap<String, usize>,
    /// The messages of each loaded chat in timeline order. A chat that is not loaded has no entry.
    pub(crate) chat_msg_indices: HashMap<String, Vec<usize>>,
    /// The loaded chats, the chat that was used last at the end.
    loaded: Vec<String>,
    /// The chats that the windows show, which stay loaded.
    pub shown: Vec<String>,
    /// The replies of each thread in timeline order, by the ID of the thread root.
    thread_msg_indices: HashMap<String, Vec<usize>>,
    pub assets: HashMap<String, StoredAsset>,
    assets_by_mxc: HashMap<String, String>,
    pub connect: crate::connect::Connect,
    /// A new token request (no outcome), or the end of one.
    pub connect_tx: broadcast::Sender<(String, Option<crate::connect::Outcome>)>,
    pub settings: BeeperSettings,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub focus_tx: broadcast::Sender<Option<String>>,
    /// The arguments of each later launch, which hands them to this instance.
    pub launch_tx: broadcast::Sender<Vec<String>>,
    /// Whether the Desktop API server should listen.
    pub api_on: tokio::sync::watch::Sender<bool>,
    pub matrix: Option<matrix_sdk::Client>,
    pub matrix_error: Option<String>,
    /// The homeserver ended this device's session (signed out elsewhere); the app starts over at sign-in.
    pub session_ended: bool,
    /// Desktop's `isSyncing`: the app is connected to Matrix and waits for its first sync.
    pub syncing: bool,
    pub sas_confirm: Option<tokio::sync::oneshot::Sender<bool>>,
    /// The new recovery key of a reset, until the user types it again.
    pub pending_recovery_key: Option<String>,
    /// Desktop's `INITIAL_SYNC_PROGRESS`: the rooms that the first sync read, and all its rooms.
    pub initial_sync: (usize, usize),
    /// Desktop SDK's UI auth token: the Beeper JWT that answers the user-interactive auth of the homeserver.
    pub ui_auth_token: Option<String>,
    /// The changes for the homeserver, with their outbox row and chat, in the order of the rows.
    pub outbox: Vec<(i64, String, Outgoing)>,
    /// Desktop's bookmarks room, after its history is read.
    pub bookmarks_room: Option<String>,
    /// The bookmarks of the bookmarks room by the event ID of the message.
    pub bookmarks: HashMap<String, Bookmark>,
    /// The bookmarked messages that are not loaded, as Chat Info fetched them, or None when they are unavailable.
    pub bookmarked: HashMap<String, Option<Message>>,
    /// Desktop's `mx_removed_messages` after the history of the removed-messages room is read: the removal events by
    /// the event ID of the removed message.
    pub removals: Option<HashMap<String, Vec<String>>>,
    /// Wakes the push task of the Matrix sync loop when the outbox has a new change.
    pub(crate) outbox_ready: std::sync::Arc<tokio::sync::Notify>,
    /// Flags whose change the homeserver has, with the value and the sync after which a sync shows the change.
    pub sent_flags: HashMap<(String, ChatFlag), (bool, u64)>,
    /// The number of syncs of the Matrix sync loop.
    pub syncs: u64,
    /// The Matrix IDs of sent events and created labels by their local IDs.
    pub sent: HashMap<String, String>,
    pub typing: HashMap<String, Vec<String>>,
    /// `io.element.recent_emoji` keys (emoji or emoji-mart ids) with their use counts.
    pub recent_emoji: Vec<(String, u64)>,
    pub scheduled: Vec<Scheduled>,
    /// The last synced global account data of each event in [`crate::prefs::SYNCED`].
    pub synced_prefs: BTreeMap<&'static str, serde_json::Value>,
    /// `com.beeper.emoji_skin_tone`, 0 is the default yellow.
    pub skin_tone: u8,
    /// The last synced [`crate::matrix::RULES`].
    pub inbox_rules: serde_json::Value,
    /// Desktop's `mutedRoomIDs`: the `room_ids` of the global `com.beeper.muted` account data.
    pub muted_rooms: Vec<String>,
    /// Desktop's `lastViewedRequestsAtByAccount`: when the Requests view last showed each account's requests, in
    /// milliseconds, by [`BeeperStore::request_key`].
    pub requests_viewed: HashMap<String, i64>,
    pub features: crate::features::Flags,
    pub whoami: Profile,
    next_seq: i64,
}

impl BeeperStore {
    pub fn from_config(config: CentralConfig) -> Self {
        let mut store = Self::open(config);
        store.load_chats();
        store
    }

    /// Opens the store without its chats, which `load_chats` adds. Before that, only the config and the stored
    /// settings are complete.
    pub fn open(mut config: CentralConfig) -> Self {
        assert_not_original_beeper_dir(&config.data_dir);
        let logged_out = config.data_dir.join(LOGGED_OUT);
        if logged_out.exists() {
            let gone = |r: std::io::Result<()>| {
                if let Err(e) = r {
                    assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "delete the logged-out data: {e}");
                }
            };
            for f in ["beeper-rs.db", "beeper-rs.db-wal", "beeper-rs.db-shm"] {
                gone(std::fs::remove_file(config.data_dir.join(f)));
            }
            for d in ["matrix", "media"] {
                gone(std::fs::remove_dir_all(config.data_dir.join(d)));
            }
            std::fs::remove_file(logged_out).expect("remove the logout marker");
        }
        std::fs::create_dir_all(&config.data_dir).expect("create the beeper-rs data dir");
        std::fs::create_dir_all(&config.assets_dir).expect("create isolated media dir");
        let conn = Connection::open(&config.db_path).expect("open beeper-rs.db");
        let sqlite = rusqlite::version_number();
        assert!(
            sqlite >= 3_038_000,
            "beeper-rs needs SQLite 3.38 or later for STRICT tables and JSON; this system has {}",
            rusqlite::version()
        );
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             PRAGMA mmap_size=268435456;
             PRAGMA cache_size=-16000;",
        )
        .expect("configure SQLite");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).expect("read schema version");
        match version {
            0 => conn
                .execute_batch(&format!(
                    "BEGIN; {SCHEMA} {} {OUTBOX} {} {} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;",
                    messages_schema(),
                    order_schema(),
                    time_schema()
                ))
                .expect("create beeper-rs schema"),
            1..SCHEMA_VERSION => {
                let steps = [
                    "UPDATE assets SET json = json_set(json_remove(json, '$.local_file_path'), '$.disk_name',
                         substr(json_extract(json, '$.local_file_path'), instr(json_extract(json, '$.local_file_path'), upload_id || '_')));"
                        .to_string(),
                    widen_sort_keys(),
                    widen_sort_keys(),
                    format!(
                        "ALTER TABLE messages RENAME TO messages_v4; {}
                         INSERT INTO messages (id, chat_id, sort_key, json)
                             SELECT id, chat_id, sort_key, json FROM messages_v4 ORDER BY rowid;
                         DROP TABLE messages_v4;",
                        messages_schema()
                    ),
                    OUTBOX.to_string(),
                    secret_flags(),
                    order_schema(),
                    transcriptions(),
                    time_schema(),
                ];
                for (from, sql) in (version..).zip(&steps[version as usize - 1..]) {
                    conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", from + 1))
                        .unwrap_or_else(|e| panic!("migrate beeper-rs schema {from} to {}: {e}", from + 1));
                }
            }
            SCHEMA_VERSION => {}
            other => panic!(
                "{} has schema version {other}, and this beeper-rs build reads version {SCHEMA_VERSION}",
                config.db_path.display()
            ),
        }
        let stored_token = "SELECT value ->> '$.access_token' FROM kv WHERE key = ?1";
        let token: Option<String> =
            conn.query_row(stored_token, [KEY_STATE], |r| r.get(0)).optional().expect("read the stored token");
        // A data folder from `--demo` holds the demo account, which must never reach the network.
        config.demo |= token.as_deref() == Some(DEFAULT_TOKEN);
        let beeper_state = if config.demo { demo_state() } else { BeeperStateRecord::default() };
        let features =
            if config.demo { crate::features::Flags::free_plan() } else { crate::features::Flags::default() };
        let mut store = Self {
            setup: None,
            settings: BeeperSettings::default(),
            config,
            db: Mutex::new(conn),
            verification: None,
            beeper_state,
            bridge_accounts: Vec::new(),
            pin_rank: HashMap::new(),
            recent_chat_ids: Vec::new(),
            network_spaces: Vec::new(),
            contacts: Vec::new(),
            bridge_sessions: HashMap::new(),
            accounts: Vec::new(),
            labels: Vec::new(),
            chats: Vec::new(),
            chat_idx_by_id: HashMap::new(),
            chat_order: BTreeSet::new(),
            local_chat_idx_by_id: HashMap::new(),
            messages: Vec::new(),
            msg_idx_by_id: HashMap::new(),
            chat_msg_indices: HashMap::new(),
            loaded: Vec::new(),
            shown: Vec::new(),
            thread_msg_indices: HashMap::new(),
            assets: HashMap::new(),
            assets_by_mxc: HashMap::new(),
            connect: crate::connect::Connect::default(),
            connect_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            ws_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            focus_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            launch_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            api_on: tokio::sync::watch::channel(false).0,
            matrix: None,
            matrix_error: None,
            session_ended: false,
            syncing: false,
            sas_confirm: None,
            pending_recovery_key: None,
            initial_sync: (0, 0),
            ui_auth_token: None,
            outbox: Vec::new(),
            bookmarks_room: None,
            bookmarks: HashMap::new(),
            bookmarked: HashMap::new(),
            removals: None,
            outbox_ready: Default::default(),
            sent_flags: HashMap::new(),
            syncs: 0,
            sent: HashMap::new(),
            typing: HashMap::new(),
            recent_emoji: Vec::new(),
            scheduled: Vec::new(),
            synced_prefs: BTreeMap::new(),
            skin_tone: 0,
            inbox_rules: serde_json::Value::Null,
            muted_rooms: Vec::new(),
            requests_viewed: HashMap::new(),
            features,
            whoami: Profile::default(),
            next_seq: 1000,
        };
        store.load_from_sqlite();
        store
    }

    /// Desktop's `buildSnapshot`: the sign-in and encrypted messaging setup state.
    pub fn app_state(&self) -> AppState {
        let r = &self.beeper_state;
        let state = r.app_state();
        AppState {
            state,
            matrix: (state != "needs-login").then(|| AppMatrix {
                user_id: r.user_id.clone(),
                device_id: r.device_id.clone(),
                homeserver: r.homeserver.clone(),
            }),
            e2ee: AppE2ee {
                initialized: r.initialized,
                secret_storage: r.secret_storage,
                cross_signing: r.cross_signing,
                verified: r.verified,
                secrets: AppSecrets {
                    master_key: r.secrets.master_key,
                    self_signing_key: r.secrets.self_signing_key,
                    user_signing_key: r.secrets.user_signing_key,
                    megolm_backup_key: r.secrets.megolm_backup_key,
                    recovery_key: !r.secrets.recovery_code.is_empty(),
                },
                key_backup: r.key_backup,
                first_sync_done: r.first_sync_done,
                has_backed_up_recovery_key: r.has_backed_up_code,
                recovery_key_generated_at: (r.recovery_code_generated_at > 0).then_some(r.recovery_code_generated_at),
            },
            verification: self.app_verification().cloned(),
        }
    }

    /// Desktop's `buildVerificationSnapshot`: a signed-out app has no verification.
    pub fn app_verification(&self) -> Option<&AppVerification> {
        self.verification.as_ref().filter(|_| self.signed_in())
    }

    fn exec(&self, sql: &str, args: &[&dyn ToSql]) {
        self.db.lock().unwrap().prepare_cached(sql).expect("prepare SQL").execute(args).expect("execute SQL");
    }

    /// Changes the settings and saves them, and turns the Desktop API on or off when "Enable Desktop API" changed.
    pub fn update_settings(&mut self, f: impl FnOnce(&mut BeeperSettings)) {
        f(&mut self.settings);
        self.persist_state_to_sqlite();
        let on = self.settings.enable_pas_api;
        self.api_on.send_if_modified(|v| std::mem::replace(v, on) != on);
    }

    pub fn persist_state_to_sqlite(&self) {
        for (k, v) in [
            (KEY_STATE, serde_json::to_string(&self.beeper_state)),
            (KEY_BRIDGES, serde_json::to_string(&self.bridge_accounts)),
            (KEY_SETTINGS, serde_json::to_string(&self.settings)),
            (KEY_PIN_RANK, serde_json::to_string(&self.pin_rank)),
            (KEY_RECENT_CHATS, serde_json::to_string(&self.recent_chat_ids)),
            (KEY_CONTACTS, serde_json::to_string(&self.contacts)),
        ] {
            self.exec("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", params![k, v.unwrap()]);
        }
    }

    fn persist_seq(&self) {
        self.put_kv(KEY_SEQ, &self.next_seq.to_string());
    }

    fn persist_chat(&self, idx: usize) {
        let c = &self.chats[idx];
        self.exec(
            "INSERT INTO chats (id, json) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            params![c.id, serde_json::to_string(c).unwrap()],
        );
    }

    fn persist_message(&self, idx: usize) {
        let m = &self.messages[idx];
        self.exec(
            "INSERT INTO messages (id, chat_id, sort_key, json) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (id)
             DO UPDATE SET chat_id = excluded.chat_id, sort_key = excluded.sort_key, json = excluded.json",
            params![m.id, m.chat_id, m.sort_key, serde_json::to_string(m).unwrap()],
        );
    }

    /// Reads the stored messages that `filter`, the SQL after `WHERE`, selects in its order, until `f` returns false.
    pub fn scan_stored(&self, filter: &str, args: impl rusqlite::Params, mut f: impl FnMut(Message) -> bool) {
        self.scan_rows(filter, args, |_, m| f(m));
    }

    /// [`Self::scan_stored`] with the row `n` of each message.
    fn scan_rows(&self, filter: &str, args: impl rusqlite::Params, mut f: impl FnMut(i64, Message) -> bool) {
        let db = self.db.lock().unwrap();
        let mut q =
            db.prepare_cached(&format!("SELECT n, json FROM messages WHERE {filter}")).expect("prepare a message read");
        let mut rows = q.query(args).expect("read messages");
        while let Some(row) = rows.next().expect("read a message row") {
            let mut m: Message =
                serde_json::from_str(row.get_ref(1).unwrap().as_str().unwrap()).expect("parse a stored message");
            m.sync_flags();
            if !f(row.get(0).expect("a message row"), m) {
                break;
            }
        }
    }

    /// The stored messages that `filter` selects, like `scan_stored`.
    pub fn stored(&self, filter: &str, args: impl rusqlite::Params) -> Vec<Message> {
        let mut all = Vec::new();
        self.scan_stored(filter, args, |m| {
            all.push(m);
            true
        });
        all
    }

    /// The stored message `id`.
    pub fn stored_message(&self, id: &str) -> Option<Message> {
        self.stored("id = ?1", [id]).pop()
    }

    /// Loads the 100 chats with the newest activity, up to half of `LOADED_MESSAGES` messages, so that they open at
    /// once.
    pub fn load_recent(&mut self) {
        let recent: Vec<String> = self.chat_order.iter().rev().take(100).map(|k| k.1.clone()).collect();
        for id in recent {
            if self.msg_idx_by_id.len() >= LOADED_MESSAGES / 2 {
                break;
            }
            self.load_chat(&id);
        }
    }

    /// Loads the messages of the chat `id`, unless they are loaded, and unloads old chats. It can move the indices of
    /// `messages`, so call it before taking them.
    pub fn load_chat(&mut self, id: &str) {
        self.loaded.retain(|c| c != id);
        self.loaded.push(id.into());
        if !self.chat_msg_indices.contains_key(id) {
            assert!(self.chat_idx_by_id.contains_key(id), "load the messages of the unknown chat {id}");
            self.chat_msg_indices.insert(id.into(), Vec::new());
            for m in self.stored("chat_id = ?1 ORDER BY sort_key", [id]) {
                self.insert_message(m);
            }
        }
        self.unload_old();
    }

    /// With more than `LOADED_MESSAGES` loaded messages, unloads the chats that were used least recently down to half
    /// of them, except the chat that was used last and the shown chats. So one unload frees the room for many loads.
    fn unload_old(&mut self) {
        let mut live = self.msg_idx_by_id.len();
        if live <= LOADED_MESSAGES {
            return;
        }
        let old = self.loaded[..self.loaded.len() - 1].iter().filter(|c| {
            let drop = live > LOADED_MESSAGES / 2 && !self.shown.contains(c);
            live -= if drop { self.chat_msg_indices[*c].len() } else { 0 };
            drop
        });
        let old: HashSet<String> = old.cloned().collect();
        self.loaded.retain(|c| !old.contains(c));
        self.chat_msg_indices.retain(|c, _| !old.contains(c));
        self.compact();
    }

    /// The index in `messages` of the message with the ID, or `None` when it is not in memory. [`Self::find_message`]
    /// also loads it.
    pub fn loaded_idx(&self, id: &str) -> Option<usize> {
        self.msg_idx_by_id.get(id).copied()
    }

    /// The index in `messages` of the message with the ID. It panics when the message is not in memory.
    pub fn message_idx(&self, id: &str) -> usize {
        self.loaded_idx(id).unwrap_or_else(|| panic!("{id} is not a loaded message"))
    }

    /// The message with the ID, or `None` when it is not in memory.
    pub fn loaded(&self, id: &str) -> Option<&Message> {
        self.loaded_idx(id).map(|i| &self.messages[i])
    }

    /// The message with the ID. It panics when the message is not in memory.
    pub fn message(&self, id: &str) -> &Message {
        &self.messages[self.message_idx(id)]
    }

    /// The indices in `messages` of the loaded messages of the chat, in timeline order. A chat without loaded
    /// messages has none; [`Self::load_chat`] loads them.
    pub fn timeline_indices(&self, chat: &str) -> &[usize] {
        self.chat_msg_indices.get(chat).map_or(&[], Vec::as_slice)
    }

    /// The loaded messages of the chat, in timeline order.
    pub fn timeline(&self, chat: &str) -> impl DoubleEndedIterator<Item = &Message> + ExactSizeIterator + Clone {
        self.timeline_indices(chat).iter().map(|&i| &self.messages[i])
    }

    /// Loads the chat of the stored message `id`, and returns the index of the message.
    pub fn find_message(&mut self, id: &str) -> Option<usize> {
        if !self.msg_idx_by_id.contains_key(id) {
            let db = self.db.lock().unwrap();
            let chat: Option<String> = db
                .prepare_cached("SELECT chat_id FROM messages WHERE id = ?1")
                .unwrap()
                .query_row([id], |r| r.get(0))
                .optional()
                .expect("find a message");
            drop(db);
            self.load_chat(&chat?);
        }
        Some(self.msg_idx_by_id[id])
    }

    /// Drops the messages of the chats that are not loaded, and the removed messages, from `messages`.
    fn compact(&mut self) {
        let messages = std::mem::take(&mut self.messages);
        let mut moved = vec![usize::MAX; messages.len()];
        for (i, m) in messages.into_iter().enumerate() {
            if self.chat_msg_indices.contains_key(&m.chat_id) && self.msg_idx_by_id.get(&m.id) == Some(&i) {
                moved[i] = self.messages.len();
                self.messages.push(m);
            }
        }
        self.msg_idx_by_id.retain(|_, i| {
            *i = moved[*i];
            *i != usize::MAX
        });
        for i in self.chat_msg_indices.values_mut().flatten() {
            *i = moved[*i];
            assert_ne!(*i, usize::MAX, "a listed message stays");
        }
        self.thread_msg_indices.retain(|_, list| {
            list.iter_mut().for_each(|i| *i = moved[*i]);
            !list.contains(&usize::MAX)
        });
    }

    fn load_from_sqlite(&mut self) {
        let (setup, state, bridges, settings, pin_rank, recents, contacts, assets) = {
            let conn = self.db.lock().unwrap();
            let kv = |k: &str| {
                conn.query_row("SELECT value FROM kv WHERE key = ?1", [k], |r| r.get::<_, String>(0))
                    .optional()
                    .expect("read kv")
            };
            let rows = |sql: &str| -> Vec<String> {
                let mut stmt = conn.prepare(sql).expect("prepare row query");
                stmt.query_map([], |r| r.get(0)).expect("query rows").map(|r| r.expect("read row")).collect()
            };
            (
                kv(KEY_SETUP),
                kv(KEY_STATE),
                kv(KEY_BRIDGES),
                kv(KEY_SETTINGS),
                kv(KEY_PIN_RANK),
                kv(KEY_RECENT_CHATS),
                kv(KEY_CONTACTS),
                rows("SELECT json FROM assets"),
            )
        };
        self.setup = setup.map(|s| match s.as_str() {
            "fresh" => Setup::Fresh,
            "imported" => Setup::Imported,
            other => panic!("unknown setup value {other} in {}", self.config.db_path.display()),
        });
        if let Some(s) = state {
            self.beeper_state = serde_json::from_str(&s).expect("parse stored state");
        }
        let kv = |k| {
            self.db
                .lock()
                .unwrap()
                .query_row("SELECT value FROM kv WHERE key = ?1", [k], |r| r.get(0))
                .optional()
                .expect("read kv")
        };
        self.connect = crate::connect::Connect::load(kv(crate::connect::KEY_TOKENS), kv(crate::connect::KEY_CLIENTS));
        if let Some(s) = kv(KEY_SCHEDULED) {
            self.scheduled = serde_json::from_str(&s).expect("parse stored scheduled messages");
        }
        if let Some(b) = bridges {
            self.bridge_accounts = serde_json::from_str(&b).expect("parse stored bridge accounts");
        }
        if let Some(p) = pin_rank {
            self.pin_rank = serde_json::from_str(&p).expect("parse stored pin rank");
        }
        if let Some(r) = recents {
            self.recent_chat_ids = serde_json::from_str(&r).expect("parse stored recent chats");
        }
        if let Some(c) = contacts {
            self.contacts = serde_json::from_str(&c).expect("parse stored contacts");
        }
        if let Some(s) = settings {
            self.settings = serde_json::from_str(&s).expect("parse stored settings");
            self.settings.prefs.retain(|k, _| !crate::prefs::RETIRED.contains(&k.as_str()));
            self.settings.assert_valid();
        }
        self.pin_app_locale();
        for a in assets {
            let asset: StoredAsset = serde_json::from_str(&a).expect("parse stored asset");
            self.assets_by_mxc.insert(asset.mxc_url.clone(), asset.upload_id.clone());
            self.assets.insert(asset.upload_id.clone(), asset);
        }
    }

    /// Loads the stored chats, or seeds the chats of a new demo store.
    pub fn load_chats(&mut self) {
        let (seq, chats, messages) = {
            let conn = self.db.lock().unwrap();
            let mut chats = conn.prepare("SELECT json FROM chats ORDER BY rowid").expect("prepare the chat query");
            let seq = "SELECT value FROM kv WHERE key = ?1";
            (
                conn.query_row(seq, [KEY_SEQ], |r| r.get::<_, String>(0)).optional().expect("read kv"),
                chats.query_map([], |r| r.get(0)).expect("query chats").collect::<Result<Vec<String>, _>>().unwrap(),
                conn.query_row("SELECT max(sort_key) FROM messages", [], |r| r.get::<_, Option<String>>(0))
                    .expect("read the last sort key"),
            )
        };
        let fresh = chats.is_empty();
        assert!(!fresh || messages.is_none(), "stored messages without chats");
        if fresh && self.config.demo {
            self.seed_default_workspace();
            self.finish_setup(Setup::Fresh);
        }
        for c in chats {
            self.push_chat(serde_json::from_str(&c).expect("parse stored chat"), false);
        }
        if let Some(key) = messages {
            self.next_seq = self.next_seq.max(key.parse().expect("sort keys are numbers"));
        }
        self.outbox = self.outbox_rows(None);
        match seq {
            Some(s) => self.next_seq = self.next_seq.max(s.parse().expect("parse stored nextSeq")),
            None => assert!(fresh, "stored chats without a stored nextSeq"),
        }
        self.persist_seq();
        if self.signed_in() {
            self.ensure_note_chat(&Profile::default());
        }
        self.rebuild_accounts_from_bridges();
        if self.config.demo {
            let seed: Seed = serde_json::from_str(SEED_JSON).expect("parse bundled seed.json");
            let label = |l: SeedLabel| {
                let rooms = self.chats.iter().filter(|c| c.label_ids.contains(&l.id)).map(|c| c.id.clone()).collect();
                Label { id: l.id, name: l.name, color: l.color, icon: l.icon, rooms, ..Label::default() }
            };
            self.labels = seed.labels.into_iter().map(label).collect();
        }
    }

    fn put_kv(&self, key: &str, value: &str) {
        self.exec("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", params![key, value]);
    }

    pub fn persist_connect(&mut self) {
        let tokens = self.connect.tokens_json();
        self.put_kv(crate::connect::KEY_TOKENS, &tokens);
        self.put_kv(crate::connect::KEY_CLIENTS, &self.connect.clients_json());
    }

    /// Desktop's `createToken` of Settings > Integrations: a token without a request.
    pub fn create_token(
        &mut self,
        scopes: Vec<String>,
        expires_secs: i64,
        client: Option<crate::connect::ClientInfo>,
    ) -> String {
        let token = self.connect.create(scopes, expires_secs, client);
        self.persist_connect();
        token
    }

    pub fn revoke_token(&mut self, id: &str) {
        assert!(self.connect.tokens.remove(id).is_some(), "revoke the unknown token {id}");
        self.persist_connect();
    }

    fn persist_scheduled(&self) {
        self.put_kv(KEY_SCHEDULED, &serde_json::to_string(&self.scheduled).expect("serialize scheduled messages"));
    }

    /// Desktop's `scheduleMessages` for one content part: a text, or a stored upload with `text` as its caption.
    pub fn schedule(
        &mut self,
        chat_id: &str,
        text: String,
        reply_to: Option<String>,
        at: (i64, bool),
        upload: Option<String>,
    ) {
        let (send_on, no_reply) = at;
        let chat_id = self.resolve_canonical_chat_id(chat_id).expect("schedule in a known chat");
        let id = self.next_id("pending");
        let added_on = crate::config::now_unix_millis();
        let s = Scheduled {
            id,
            chat_id,
            text,
            reply_to,
            added_on,
            send_on,
            send_only_on_no_response: no_reply,
            canceled: false,
            upload,
        };
        self.scheduled.push(s);
        self.persist_scheduled();
    }

    /// Desktop's `updateScheduledMessage`: `f` changes the entry `id`, and removes it when it returns false.
    pub fn update_scheduled(&mut self, id: &str, f: impl FnOnce(&mut Scheduled) -> bool) {
        let i = self.scheduled.iter().position(|s| s.id == id).expect("a known scheduled message");
        if !f(&mut self.scheduled[i]) {
            self.scheduled.remove(i);
        }
        self.persist_scheduled();
    }

    /// Desktop's `cancelSendIfReplied` for each entry, then the entries due at `now`, which leave the store.
    pub fn take_due_scheduled(&mut self, now: i64) -> Vec<Scheduled> {
        // The messages after the entry come last in the timeline.
        let replied = |st: &Self, s: &Scheduled| {
            let mut found = false;
            st.scan_stored("chat_id = ?1 ORDER BY sort_key DESC", [&s.chat_id], |m| {
                let later = crate::config::millis_from_rfc3339(&m.timestamp).is_some_and(|t| t >= s.added_on);
                found = later && m.is_sender != Some(true) && !m.silent();
                later && !found
            });
            found
        };
        let cancel: Vec<usize> = (0..self.scheduled.len())
            .filter(|&i| {
                let s = &self.scheduled[i];
                s.send_only_on_no_response && !s.canceled && replied(self, s)
            })
            .collect();
        cancel.iter().for_each(|&i| self.scheduled[i].canceled = true);
        let (due, rest) =
            std::mem::take(&mut self.scheduled).into_iter().partition(|s| !s.canceled && s.send_on <= now);
        self.scheduled = rest;
        let due: Vec<Scheduled> = due;
        if !cancel.is_empty() || !due.is_empty() {
            self.persist_scheduled();
        }
        due
    }

    /// Answers a token request from the "Approve connection" dialog.
    pub fn answer_token_request(&mut self, id: &str, grant: Option<(Vec<String>, i64)>) {
        let outcome =
            if grant.is_some() { crate::connect::Outcome::Approved } else { crate::connect::Outcome::Rejected };
        if self.connect.answer(id, grant) {
            self.persist_connect();
            self.connect_tx.send((id.to_string(), Some(outcome))).ok();
        }
    }

    pub fn finish_setup(&mut self, how: Setup) {
        assert!(self.setup.is_none(), "setup already finished as {:?}", self.setup);
        self.setup = Some(how);
        self.put_kv(KEY_SETUP, how.key());
    }

    pub fn import_beeper(&mut self, data: crate::import::BeeperData) {
        assert!(self.setup.is_none() && !self.signed_in(), "import into a store that is already set up");
        assert!(self.chats.is_empty() && self.messages.is_empty(), "import into a store that has chats");
        self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin import transaction");
        self.finish_setup(Setup::Imported);
        let (chats, messages) = (self.chats.len(), self.messages.len());
        for c in data.chats {
            self.push_chat(c, true);
        }
        for mut m in data.messages {
            if m.sort_key.is_empty() {
                self.next_seq += 1;
                m.sort_key = sort_key(self.next_seq);
            }
            self.push_message(m, false);
        }
        (chats..self.chats.len()).for_each(|i| self.persist_chat(i));
        (messages..self.messages.len()).for_each(|i| self.persist_message(i));
        self.persist_seq();
        self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit import transaction");
        self.unload_old();
        self.pin_rank = data.pin_rank;
        self.recent_chat_ids = data.recent_chat_ids;
        if let Some(theme) = data.theme {
            self.settings.theme = theme;
        }
        self.settings.spaces_order = data.spaces_order;
        self.settings.ever_labeled = data.ever_labeled;
        self.persist_state_to_sqlite();
    }

    pub fn matrix_dir(&self) -> PathBuf {
        self.config.data_dir.join("matrix")
    }

    pub fn signed_in(&self) -> bool {
        !self.beeper_state.access_token.is_empty()
    }

    /// Desktop's `hasConnectedNetwork`: a chat account other than Beeper and AI.
    pub fn has_connected_network(&self) -> bool {
        self.bridge_accounts.iter().any(|a| !matches!(a.bridge_id.as_str(), "hungryserv" | "ai"))
    }

    /// Desktop's `applyAppLocale`: without an app language, a signed-in app takes English, and a signed-out app the
    /// system language, which it keeps after the sign-in.
    fn pin_app_locale(&mut self) {
        if !self.settings.prefs.contains_key("appLanguage") {
            self.settings.set_pref("appLanguage", u8::from(self.signed_in()));
        }
    }

    pub fn set_cloud_accounts(&mut self, accounts: Vec<BridgeAccountInfo>) {
        assert!(self.signed_in(), "cloud accounts need a signed-in session");
        self.bridge_accounts = accounts;
        self.rebuild_accounts_from_bridges();
        self.persist_state_to_sqlite();
    }

    pub fn set_contacts(&mut self, contacts: Vec<User>) {
        self.contacts = contacts;
        self.persist_state_to_sqlite();
    }

    pub fn sign_in(
        &mut self,
        session: MatrixSession,
        homeserver: String,
        profile: &Profile,
        accounts: Vec<BridgeAccountInfo>,
    ) {
        assert!(!self.signed_in(), "sign in while already signed in as {}", self.beeper_state.user_id);
        if self.setup.is_none() {
            self.finish_setup(Setup::Fresh);
        }
        self.beeper_state = BeeperStateRecord {
            user_id: session.user_id,
            access_token: session.access_token,
            device_id: session.device_id,
            homeserver,
            ..BeeperStateRecord::default()
        };
        self.ui_auth_token = session.jwt;
        self.pin_app_locale();
        self.persist_state_to_sqlite();
        self.whoami = profile.clone();
        self.ensure_note_chat(profile);
        self.set_cloud_accounts(accounts);
        self.emit_ws(
            "chat.upserted",
            NOTE_CHAT_ID,
            None,
            Some(self.chats[self.chat_idx_by_id[NOTE_CHAT_ID]].clone()),
            None,
        );
    }

    fn ensure_note_chat(&mut self, profile: &Profile) {
        if self.chat_idx_by_id.contains_key(NOTE_CHAT_ID) {
            return;
        }
        let user_id = self.beeper_state.user_id.clone();
        let local = user_id.trim_start_matches('@').split(':').next().expect("Matrix user ID").to_string();
        let me = User {
            id: user_id,
            full_name: Some(profile.full_name.clone().unwrap_or_else(|| local.clone())),
            username: Some(profile.username.clone().unwrap_or(local)),
            email: profile.email.clone(),
            phone_number: None,
            img_url: None,
            is_self: Some(true),
            cannot_message: Some(false),
            custom_nickname: None,
            is_network_bot: None,
            ..Default::default()
        };
        let chat = Chat {
            id: NOTE_CHAT_ID.into(),
            inbox_group: false,
            account_id: "hungryserv".into(),
            participants: ChatParticipants { has_more: false, items: vec![me], total: 1 },
            title: "Note to self".into(),
            chat_type: "single".into(),
            unread_count: 0,
            unread_mentions: 0,
            description: None,
            draft: None,
            is_archived: Some(false),
            is_marked_unread: Some(false),
            is_muted: Some(false),
            is_pinned: Some(true),
            is_low_priority: Some(false),
            label_ids: Vec::new(),
            last_activity: Some(now_rfc3339_millis()),
            last_read_message_sort_key: None,
            local_chat_id: Some("local_note_self".into()),
            merge: None,
            merged_into_chat_id: None,
            preview: None,
            reminder: None,
            chat_theme: None,
            note: None,
            notifiable: None,
            is_hidden_from_inbox: None,
            is_request: None,
            created_at: None,
            network_name: None,
            network_id: None,
            img_url: None,
            delete_modes: 0,
            can_accept_request: false,
            block: 0,
            edits: 0,
            member_actions: 0,
            formatting: ALL_FORMATS,
            file: beeper_files(),
            room_features: false,
            polls: Some(BEEPER_POLLS),
            messages: BEEPER_MESSAGES,
            disappearing: None,
            timers: None,
            older: Older::Unknown,
            is_read_only: false,
            replacement_room: None,
            can_mention_room: false,
            threads: false,
            follows: ThreadFollows::default(),
            thread_reads: BTreeMap::new(),
            personal: None,
            notification_sound: None,
            flags: 0,
        };
        self.push_chat(chat, true);
    }

    fn seed_default_workspace(&mut self) {
        let seed: Seed = serde_json::from_str(SEED_JSON).expect("parse bundled seed.json");
        for b in seed.bridges {
            if !self.bridge_accounts.iter().any(|x| x.bridge_id == b.bridge_id) {
                self.bridge_accounts.push(b);
            }
        }
        self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin seed transaction");
        for c in seed.chats {
            self.push_chat(c, true);
        }
        for m in seed.messages {
            self.push_message(m, true);
        }
        for c in &self.chats {
            for u in &c.participants.items {
                if u.is_self != Some(true)
                    && u.full_name.as_deref().is_some_and(|n| {
                        !n.is_empty() && !self.contacts.iter().any(|x| x.full_name.as_deref() == Some(n))
                    })
                {
                    self.contacts.push(User { id: format!("contact:{}:{}", c.account_id, u.id), ..u.clone() });
                }
            }
        }
        self.contacts.sort_by_key(|u| {
            let ch = u.full_name.as_deref().and_then(|s| s.chars().next()).unwrap_or(' ').to_ascii_uppercase();
            (!ch.is_ascii_alphabetic(), ch)
        });
        self.persist_state_to_sqlite();
        self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit seed transaction");
    }

    pub fn me(&self) -> &User {
        &self.chats[self.chat_idx_by_id[NOTE_CHAT_ID]].participants.items[0]
    }

    /// Adds a change to the outbox, which the push task of the Matrix sync loop sends.
    pub fn queue(&mut self, chat_id: &str, out: Outgoing) {
        let bytes = match &out {
            Outgoing::Media { bytes, .. } => Some(bytes.as_slice()),
            _ => None,
        };
        let json = serde_json::to_string(&out).expect("outgoing JSON");
        self.exec("INSERT INTO outbox (chat_id, json, bytes) VALUES (?1, ?2, ?3)", params![chat_id, json, bytes]);
        let row = self.db.lock().unwrap().last_insert_rowid();
        self.outbox.push((row, chat_id.into(), out));
        self.outbox_ready.notify_one();
    }

    /// The outbox row `id`, or all rows in order.
    fn outbox_rows(&self, id: Option<i64>) -> Vec<(i64, String, Outgoing)> {
        let db = self.db.lock().unwrap();
        let mut q =
            db.prepare_cached("SELECT id, chat_id, json, bytes FROM outbox WHERE ?1 IN (id, 0) ORDER BY id").unwrap();
        let rows = q.query_map([id.unwrap_or(0)], |r| {
            let mut out: Outgoing = serde_json::from_str(&r.get::<_, String>(2)?).expect("parse a stored change");
            if let Outgoing::Media { bytes, .. } = &mut out {
                *bytes = r.get(3)?;
            }
            Ok((r.get(0)?, r.get(1)?, out))
        });
        rows.expect("read the outbox").map(|r| r.expect("read an outbox row")).collect()
    }

    /// Desktop's `hasErroredMessages`: a send of the chat failed and waits for Retry.
    pub fn has_errored(&self, chat: &str) -> bool {
        self.outbox.iter().any(|o| o.1 == chat && self.waits_for_retry(&o.2))
    }

    /// A message of the change failed and waits for Retry.
    fn waits_for_retry(&self, out: &Outgoing) -> bool {
        let m = out.message().and_then(|id| self.stored_message(id));
        m.and_then(|m| m.send_error).is_some_and(|e| !e.retrying)
    }

    /// The next change for the homeserver. At the end of the outbox the push stops.
    pub(crate) fn next_outgoing(&mut self) -> Option<(i64, String, Outgoing)> {
        let next = self.outbox.iter().position(|o| !self.waits_for_retry(&o.2)).filter(|_| self.matrix.is_some());
        next.map(|i| self.outbox.remove(i))
    }

    /// Ends the push of an outbox row, a change for `message`. A failed message keeps the error, and a failed send
    /// goes back into the outbox for Retry, like Desktop's failed echoes.
    pub fn pushed(&mut self, row: i64, message: Option<&str>, error: Option<(String, bool)>) {
        match error.as_ref().is_some_and(|e| e.1) {
            true => {
                let o = self.outbox_rows(Some(row)).pop().expect("the outbox row of the push");
                let at = self.outbox.partition_point(|x| x.0 < row);
                self.outbox.insert(at, o);
            }
            false => self.exec("DELETE FROM outbox WHERE id = ?1", params![row]),
        }
        let error = error.map(|(message, _)| SendError { message: Some(message), retrying: false });
        if let Some(i) = message.and_then(|id| self.find_message(id)) {
            if self.messages[i].send_error != error {
                self.update_message(i, |m| m.send_error = error);
            }
        }
    }

    /// Desktop's `retryErroredMessage`.
    pub fn retry_send(&mut self, id: &str) {
        let i = self.find_message(id).expect("a failed message");
        self.update_message(i, |m| m.send_error.as_mut().expect("a failed message").retrying = true);
        self.outbox_ready.notify_one();
    }

    /// Desktop's `removeErroredMessage` for a send that the homeserver did not take: the local echo and its change.
    pub fn remove_unsent(&mut self, id: &str) {
        let i = self.outbox.iter().position(|o| o.2.message() == Some(id)).expect("a failed send in the outbox");
        let (row, chat, _) = self.outbox.remove(i);
        self.exec("DELETE FROM outbox WHERE id = ?1", params![row]);
        self.remove_message(&chat, id).expect("remove the local echo");
    }

    /// Desktop's optimistic thread update: the value of a flag that changed here and that a sync may not show yet, from
    /// the outbox or from a change that the homeserver has.
    pub fn local_flag(&self, chat: &str, flag: ChatFlag) -> Option<bool> {
        let queued = self.outbox.iter().rev().find_map(|(_, c, o)| match o {
            Outgoing::Flag(f, on) if c == chat && *f == flag => Some(*on),
            _ => None,
        });
        queued.or_else(|| self.sent_flags.get(&(chat.to_string(), flag)).map(|v| v.0))
    }

    /// Keeps the flags of `c` that changed here and that the homeserver state of `c` may not show yet.
    /// The archive flag comes from `com.beeper.inbox.done` later, in `sync_room`.
    pub fn keep_local_flags(&self, c: &mut Chat) {
        for flag in ChatFlag::ALL.into_iter().filter(|&f| f != ChatFlag::Archived) {
            if let Some(on) = self.local_flag(&c.id, flag) {
                *c.flag_mut(flag) = Some(on);
            }
        }
    }

    /// A flag change reached the homeserver. The sync in flight may not show it, the sync after it does.
    pub fn flag_sent(&mut self, chat: String, flag: ChatFlag, on: bool) {
        self.sent_flags.insert((chat, flag), (on, self.syncs + 2));
    }

    /// A sync of the Matrix sync loop is done: the changes that it shows need no local value any more.
    pub fn synced(&mut self) {
        let n = self.syncs;
        self.sent_flags.retain(|_, v| v.1 > n);
    }

    /// Sets a chat flag here and on the homeserver. `None` toggles the flag.
    /// Returns false, like Desktop, when asked to archive a pinned chat.
    pub fn set_chat_flag(&mut self, idx: usize, flag: ChatFlag, on: Option<bool>) -> bool {
        let c = &self.chats[idx];
        let value = on.unwrap_or(!c.flag(flag));
        if flag == ChatFlag::Archived && value && c.flag(ChatFlag::Pinned) {
            return false;
        }
        self.update_chat(idx, |c| *c.flag_mut(flag) = Some(value));
        let id = self.chats[idx].id.clone();
        if flag == ChatFlag::Pinned {
            // Desktop's `favourites_order` edit: a new pin goes last, an unpin leaves the list.
            let last = self.pin_rank.values().max().map_or(0, |r| r + 1);
            match value {
                true => self.pin_rank.insert(id.clone(), last),
                false => self.pin_rank.remove(&id),
            };
            self.persist_state_to_sqlite();
        }
        self.queue(&id, Outgoing::Flag(flag, value));
        true
    }

    /// Desktop's `setFavouritesOrder` after a drag in the pinned grid: the pins of `shown` in that order, then the other
    /// pins in their order. Returns the `com.beeper.favourites_order` inbox list to write.
    pub fn set_pin_order(&mut self, shown: &[String]) -> Vec<String> {
        let mut rest: Vec<(usize, String)> =
            self.pin_rank.iter().filter(|(id, _)| !shown.contains(id)).map(|(id, &r)| (r, id.clone())).collect();
        rest.sort();
        let order: Vec<String> = shown.iter().cloned().chain(rest.into_iter().map(|(_, id)| id)).collect();
        self.pin_rank = order.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        self.persist_state_to_sqlite();
        order
    }

    /// Accepts a message request here at once and asks the bridge to accept it, like Desktop.
    pub fn accept_request(&mut self, id: &str) {
        let idx = self.chat_idx_by_id[id];
        let c = &self.chats[idx];
        assert!(c.is_request == Some(true) && c.can_accept_request, "{id} is not a request that can be accepted");
        self.update_chat(idx, |c| c.is_request = Some(false));
        self.queue(id, Outgoing::AcceptRequest);
    }

    /// Desktop's `canBlockUser`: the account has blocking, and the bridge blocks the other person of the DM.
    pub fn can_block(&self, c: &Chat) -> bool {
        self.feature("block-users").on && c.block & BLOCK_USER != 0
    }

    /// Desktop's `isUserBlocked`.
    pub fn user_blocked(&self, c: &Chat) -> bool {
        self.can_block(c) && c.block & USER_BLOCKED != 0
    }

    /// Desktop's `setUserBlocked`: the bridge blocks (true) or unblocks the other person of a DM, and reports spam
    /// when `report_spam`.
    pub fn set_user_blocked(&mut self, id: &str, block: bool, report_spam: bool) {
        let idx = self.chat_idx_by_id[id];
        let bits = self.chats[idx].block;
        assert!(self.can_block(&self.chats[idx]), "{id} does not support blocking");
        assert!(block || bits & NO_UNBLOCK == 0, "{id} does not support unblocking");
        assert!(!report_spam || block && bits & REPORT_SPAM != 0, "{id} reports spam only while blocking");
        self.update_chat(idx, |c| c.block = c.block & !USER_BLOCKED | (u8::from(block) * USER_BLOCKED));
        self.queue(id, Outgoing::BlockUser { block, report_spam });
    }

    /// Sets or dismisses a chat reminder here and on the homeserver.
    pub fn set_reminder(&mut self, idx: usize, reminder: Option<ChatReminder>) {
        self.update_chat(idx, |c| c.reminder = reminder.clone());
        let id = self.chats[idx].id.clone();
        self.queue(&id, Outgoing::Reminder(reminder));
    }

    /// The user ID of the user on an account: the user of the account, or the Beeper user for the Beeper chats.
    pub fn self_id(&self, account_id: &str) -> String {
        let account = self.accounts.iter().find(|a| a.account_id == account_id);
        account.map_or(&self.me().id, |a| &a.user.id).clone()
    }

    pub fn is_me(&self, user_id: &str) -> bool {
        user_id == self.me().id || user_id.starts_with("@self_")
    }

    pub fn rebuild_accounts_from_bridges(&mut self) {
        self.accounts = self
            .bridge_accounts
            .iter()
            .map(|b| Account {
                account_id: b.account_id.clone(),
                login_id: b.login_id.clone(),
                bridge: AccountBridge {
                    id: b.bridge_id.clone(),
                    bridge_type: b.bridge_type.clone(),
                    provider: match b.mode {
                        BridgeMode::Cloud => "cloud",
                        BridgeMode::OnDevice => "local",
                    }
                    .into(),
                },
                network: b.network_name.clone(),
                user: User { id: format!("@self_{}:beeper.com", b.bridge_id), ..self.me().clone() },
                status: account_status(&b.state).into(),
            })
            .collect();
    }

    /// Adds a chat. A new chat has no stored messages, so it is loaded.
    fn push_chat(&mut self, mut chat: Chat, new: bool) -> usize {
        chat.sync_flags();
        chat.participants.items.shrink_to_fit();
        chat.flags = self.label_flags(&chat);
        let idx = self.chats.len();
        assert!(self.chat_idx_by_id.insert(chat.id.clone(), idx).is_none(), "duplicate chat {}", chat.id);
        self.chat_order.insert(activity_key(&chat));
        if let Some(loc) = &chat.local_chat_id {
            self.local_chat_idx_by_id.insert(loc.clone(), idx);
        }
        if new {
            self.chat_msg_indices.insert(chat.id.clone(), Vec::new());
            self.loaded.push(chat.id.clone());
        }
        self.chats.push(chat);
        if new {
            self.persist_chat(idx);
        }
        idx
    }

    /// Puts a message of a loaded chat into the message lists.
    fn insert_message(&mut self, mut msg: Message) -> usize {
        msg.sync_flags();
        let digits = msg.sort_key.strip_prefix('-').unwrap_or(&msg.sort_key).len();
        assert_eq!(digits, 16, "sort key {} of {} is not from sort_key", msg.sort_key, msg.id);
        self.next_seq = self.next_seq.max(msg.sort_key.parse().expect("sort keys are numbers"));
        let idx = self.messages.len();
        assert!(self.msg_idx_by_id.insert(msg.id.clone(), idx).is_none(), "duplicate message {}", msg.id);
        let chat = self.chat_msg_indices.get_mut(&msg.chat_id).expect("the chat of the message is loaded");
        let thread = msg.thread_root.as_ref().map(|r| self.thread_msg_indices.entry(r.clone()).or_default());
        for list in std::iter::once(chat).chain(thread) {
            list.insert(list.partition_point(|&i| self.messages[i].sort_key <= msg.sort_key), idx);
        }
        self.messages.push(msg);
        idx
    }

    /// Adds a message of a loaded chat, which can preview the chat.
    fn push_message(&mut self, msg: Message, persist: bool) -> usize {
        let idx = self.insert_message(msg);
        let msg = &self.messages[idx];
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        // Like the timeline, a thread reply previews a chat that does not show threads.
        let shown =
            !msg.silent() && !msg.thread_only(self.threaded(&self.chats[c_idx])) && msg.flags & MSG_DELETED == 0;
        let chat = &mut self.chats[c_idx];
        if shown && chat.preview.as_ref().is_none_or(|p| p.sort_key <= msg.sort_key) {
            self.chat_order.remove(&activity_key(chat));
            chat.last_activity = Some(msg.timestamp.clone());
            chat.preview = Some(msg.clone());
            self.chat_order.insert(activity_key(chat));
        }
        if persist {
            self.persist_chat(c_idx);
            self.persist_message(idx);
        }
        idx
    }

    /// Upserts a label and the `labelIDs` of the chats that it adds or drops.
    pub fn set_label(&mut self, label: Label) {
        if !self.settings.ever_labeled {
            self.settings.ever_labeled = true;
            self.persist_state_to_sqlite();
        }
        let old = self.labels.iter().position(|l| l.id == label.id);
        let before = old.map_or_else(Vec::new, |i| self.labels[i].rooms.clone());
        let id = label.id.clone();
        let rules = old.is_some_and(|i| self.labels[i].inbox != label.inbox);
        let gone = before.iter().filter(|r| !label.rooms.contains(r));
        let kept = label.rooms.iter().filter(|r| rules || !before.contains(r));
        let changed: Vec<String> = gone.chain(kept).cloned().collect();
        let rooms = label.rooms.clone();
        match old {
            Some(i) => self.labels[i] = label,
            None => self.labels.push(label),
        }
        for chat in changed {
            if let Some(&i) = self.chat_idx_by_id.get(&chat) {
                let on = rooms.contains(&chat);
                self.update_chat(i, |c| {
                    c.label_ids.retain(|l| *l != id);
                    if on {
                        c.label_ids.push(id.clone());
                    }
                });
            }
        }
    }

    /// Desktop's `createLabel`: adds the label here at once and creates its label space on the homeserver.
    pub fn create_label(&mut self, name: &str, color: Option<u8>, icon: Option<String>, rooms: Vec<String>) -> String {
        assert!(!name.trim().is_empty(), "a label has a name");
        let label =
            Label { id: self.next_id("label"), name: name.trim().into(), color, icon, rooms, ..Label::default() };
        let id = label.id.clone();
        self.set_label(label.clone());
        self.queue(&id, Outgoing::CreateLabel(label));
        id
    }

    /// Gives the label `draft` the ID of its new label space.
    pub fn created_label(&mut self, draft: &str, id: &str) {
        self.sent.insert(draft.into(), id.into());
        let Some(label) = self.labels.iter().find(|l| l.id == draft).cloned() else { return };
        self.remove_label(draft);
        self.set_label(Label { id: id.into(), ..label });
    }

    /// Desktop's `updateLabel` from the label window: the name, color, icon and chats of the label `id`.
    pub fn update_label(&mut self, id: &str, name: &str, color: Option<u8>, icon: Option<String>, rooms: Vec<String>) {
        assert!(!name.trim().is_empty(), "a label has a name");
        let old = self.labels.iter().find(|l| l.id == id).expect("an edited label").clone();
        let removed = old.rooms.iter().filter(|r| !rooms.contains(r)).map(|r| (r.clone(), false));
        let changes =
            rooms.iter().filter(|r| !old.rooms.contains(r)).map(|r| (r.clone(), true)).chain(removed).collect();
        let label = Label { name: name.trim().into(), color, icon, rooms, ..old };
        self.set_label(label.clone());
        self.queue(id, Outgoing::EditLabel(label, changes));
    }

    /// Desktop's `updateThread("chatNotificationSound")`: a sound of [`CHAT_SOUNDS`], or `None` for the Default sound.
    pub fn set_notification_sound(&mut self, id: &str, sound: Option<String>) {
        assert!(sound.as_deref().is_none_or(|s| CHAT_SOUNDS.contains(&s)), "an unknown chat sound {sound:?}");
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| c.notification_sound = sound.clone());
        let content = sound.map_or(serde_json::json!({}), |sound| serde_json::json!({ "sound": sound }));
        self.queue(id, Outgoing::RoomData(NOTIFICATION_SOUND.into(), content));
    }

    /// Desktop's `updateThread("disappearingTimer")`: a timer in ms, or 0 for Off.
    pub fn set_disappearing(&mut self, id: &str, timer: u64) {
        let idx = self.chat_idx_by_id[id];
        let kind = self.chats[idx].timers.as_ref().expect("the room has disappearing timers").kind.clone();
        self.update_chat(idx, |c| c.disappearing = Some(timer).filter(|&t| t > 0));
        let content =
            if timer == 0 { serde_json::json!({}) } else { serde_json::json!({ "type": kind, "timer": timer }) };
        self.queue(id, Outgoing::State(DISAPPEARING_TIMER.into(), content));
    }

    /// Desktop's `removeParticipant`; the member leaves the list before the sync confirms it.
    pub fn remove_member(&mut self, id: &str, user: &str) {
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| {
            c.participants.items.retain(|u| u.id != user);
            c.participants.total = c.participants.items.len();
        });
        self.queue(id, Outgoing::Kick(user.into()));
    }

    /// Desktop's `addParticipant`, which lists the member before the sync confirms the invite.
    pub fn add_member(&mut self, id: &str, user: User) {
        let idx = self.chat_idx_by_id[id];
        let uid = user.id.clone();
        self.update_chat(idx, |c| {
            assert!(c.participants.items.iter().all(|u| u.id != user.id), "not a member yet");
            c.participants.items.push(user);
            c.participants.total = c.participants.items.len();
        });
        self.queue(id, Outgoing::Invite(uid));
    }

    /// Desktop's `changeParticipantRole`, which marks the member before the sync confirms it.
    pub fn set_admin(&mut self, id: &str, user: &str, admin: bool) {
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| {
            let u = c.participants.items.iter_mut().find(|u| u.id == user).expect("a member of the chat");
            u.is_admin = Some(admin);
        });
        self.queue(id, Outgoing::Role(user.into(), admin));
    }

    /// Desktop's `updateGroup` from the Edit Group window: the changed title and description of a group, and its new
    /// avatar (a local URL of the image, the image and its MIME type) or `Some(None)` to remove the avatar.
    pub fn update_group(
        &mut self,
        id: &str,
        title: Option<String>,
        topic: Option<String>,
        avatar: Option<Option<(String, Vec<u8>, String)>>,
    ) {
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| {
            if let Some(t) = &topic {
                c.description = Some(t.clone()).filter(|t| !t.is_empty());
            }
            c.edit_room(|c| {
                if let Some(t) = &title {
                    c.title = t.clone();
                }
                if let Some(a) = &avatar {
                    c.img_url = a.as_ref().map(|a| a.0.clone());
                }
            });
        });
        if let Some(name) = title {
            self.queue(id, Outgoing::State("m.room.name".into(), serde_json::json!({ "name": name })));
        }
        if let Some(topic) = topic {
            self.queue(id, Outgoing::State("m.room.topic".into(), serde_json::json!({ "topic": topic })));
        }
        if let Some(avatar) = avatar {
            self.queue(id, Outgoing::Avatar(avatar.map(|(_, bytes, mime)| (bytes, mime))));
        }
    }

    /// Desktop's `setPersonalRoomName` and `setPersonalRoomAvatar`: `Some(None)` removes the name or the photo.
    pub fn set_personal(
        &mut self,
        id: &str,
        title: Option<Option<String>>,
        avatar: Option<Option<(String, Vec<u8>, String)>>,
    ) {
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| {
            let (old_title, old_img) = c.personal_parts();
            let img = avatar.as_ref().map_or(old_img, |a| a.as_ref().map(|a| a.0.clone()));
            c.set_personal(title.clone().unwrap_or(old_title), img);
        });
        if let Some(name) = title {
            let content = name.map_or(serde_json::json!({}), |name| serde_json::json!({ "name": name }));
            self.queue(id, Outgoing::RoomData(PERSONAL_NAME.into(), content));
        }
        if let Some(avatar) = avatar {
            self.queue(id, Outgoing::PersonalAvatar(avatar.map(|(_, bytes, mime)| (bytes, mime))));
        }
    }

    /// Desktop's `setLabelInboxVisibility`: `None` shows all chats of the label in the inbox.
    pub fn set_label_inbox(&mut self, id: &str, inbox: Option<&'static str>) {
        let label = self.labels.iter().find(|l| l.id == id).expect("a label").clone();
        self.set_label(Label { inbox, ..label });
        let content = serde_json::json!({ "visibility": inbox.unwrap_or("all") });
        self.queue(id, Outgoing::State("com.beeper.label.inbox_visibility".into(), content));
    }

    /// Desktop's `addRoomToLabel` and `removeRoomFromLabel`.
    pub fn set_label_room(&mut self, id: &str, chat: &str, add: bool) {
        let mut label = self.labels.iter().find(|l| l.id == id).expect("a label").clone();
        label.rooms.retain(|r| r != chat);
        if add {
            label.rooms.push(chat.into());
        }
        self.set_label(label);
        self.queue(id, Outgoing::LabelChild(chat.into(), add));
    }

    /// Desktop's `toggleMuteLabel`.
    pub fn set_label_muted(&mut self, id: &str, muted: bool) {
        let label = self.labels.iter().find(|l| l.id == id).expect("a label").clone();
        self.set_label(Label { muted, ..label });
        self.queue(id, Outgoing::State("com.beeper.muted".into(), serde_json::json!({ "muted": muted })));
    }

    /// Desktop's `areNotificationsMuted`: the chat is muted, all labels of the chat are muted, or for a chat
    /// without labels, "Enable notifications" of its account is off.
    pub fn notifications_muted(&self, c: &Chat) -> bool {
        if c.flag(ChatFlag::Muted) {
            return true;
        }
        if self.feature("labels-v2").on && !c.label_ids.is_empty() {
            return c.label_ids.iter().all(|id| self.labels.iter().any(|l| &l.id == id && l.muted));
        }
        !self.settings.account(&c.account_id).notifications
    }

    /// Desktop's `deleteLabel`: leaves the label space.
    pub fn delete_label(&mut self, id: &str) {
        self.remove_label(id);
        self.queue(id, Outgoing::Leave(true));
    }

    /// Desktop's `leaveChat`: the user leaves the room, and the chat stays with the user's member exited, without a
    /// composer, until the user deletes it.
    pub fn leave_room(&mut self, id: &str) {
        let idx = self.chat_idx(id);
        self.update_chat(idx, |c| {
            for u in c.participants.items.iter_mut().filter(|u| u.is_self == Some(true)) {
                u.has_exited = Some(true);
            }
        });
        self.queue(id, Outgoing::Leave(false));
    }

    pub fn remove_label(&mut self, id: &str) {
        let Some(label) = self.labels.iter().find(|l| l.id == id).cloned() else { return };
        self.set_label(Label { rooms: Vec::new(), ..label });
        self.labels.retain(|l| l.id != id);
    }

    pub fn upsert_chat(&mut self, mut chat: Chat) {
        let Some(&idx) = self.chat_idx_by_id.get(&chat.id) else {
            chat.label_ids = self.labels.iter().filter(|l| l.rooms.contains(&chat.id)).map(|l| l.id.clone()).collect();
            let idx = self.push_chat(chat, true);
            let c = self.chats[idx].clone();
            return self.emit_ws("chat.upserted", &c.id, None, Some(c.clone()), None);
        };
        self.update_chat(idx, |c| {
            *c = Chat {
                draft: c.draft.take(),
                label_ids: std::mem::take(&mut c.label_ids),
                last_activity: c.last_activity.take().or(chat.last_activity.clone()),
                local_chat_id: c.local_chat_id.take(),
                merge: c.merge.take(),
                merged_into_chat_id: c.merged_into_chat_id.take(),
                preview: c.preview.take(),
                chat_theme: c.chat_theme.take(),
                note: c.note.take(),
                notifiable: c.notifiable.take(),
                older: std::mem::take(&mut c.older),
                thread_reads: std::mem::take(&mut c.thread_reads),
                ..chat
            }
        });
    }

    /// Drops a chat that the user left, with its messages.
    pub fn remove_chat(&mut self, id: &str) {
        self.remove_label(id);
        let Some(idx) = self.chat_idx_by_id.remove(id) else { return };
        let chat = self.chats.remove(idx);
        assert!(self.chat_order.remove(&activity_key(&chat)), "chat {id} is in the chat order");
        if let Some(local) = chat.local_chat_id {
            self.local_chat_idx_by_id.remove(&local);
        }
        for i in self.chat_idx_by_id.values_mut().chain(self.local_chat_idx_by_id.values_mut()).filter(|i| **i > idx) {
            *i -= 1;
        }
        self.loaded.retain(|c| c != id);
        if self.chat_msg_indices.remove(id).is_some() {
            self.compact();
        }
        self.exec("DELETE FROM messages WHERE chat_id = ?1", params![id]);
        self.exec("DELETE FROM chats WHERE id = ?1", params![id]);
        self.recent_chat_ids.retain(|c| c != id);
        self.typing.remove(id);
        self.outbox.retain(|(_, c, _)| c != id);
        self.exec("DELETE FROM outbox WHERE chat_id = ?1", params![id]);
        self.emit_ws("chat.deleted", id, None, None, None);
    }

    /// Asks the bridge to delete the chat, and removes it here at once like Desktop.
    pub fn delete_chat(&mut self, id: &str, for_everyone: bool) {
        let modes = self.chats[self.chat_idx_by_id[id]].delete_modes;
        let mode = if for_everyone { DELETE_FOR_EVERYONE } else { DELETE_FOR_ME };
        assert!(modes & mode != 0, "{id} does not support delete mode {mode}");
        self.remove_chat(id);
        self.queue(id, Outgoing::DeleteChat { for_everyone });
    }

    pub fn upsert_message(&mut self, mut msg: Message) {
        if self.bookmarks_room.is_some() {
            msg.is_bookmarked = Some(self.bookmarks.contains_key(&msg.id));
        }
        self.load_chat(&msg.chat_id);
        // Desktop's upsert keeps `isDeleted` when the message has a removal.
        let stored = self.msg_idx_by_id.get(&msg.id).is_some_and(|&i| self.messages[i].removed);
        if msg.is_deleted != Some(true) && (stored || self.removals.as_ref().is_some_and(|r| r.contains_key(&msg.id))) {
            removed_from_beeper(&mut msg);
        }
        let Some(&idx) = self.msg_idx_by_id.get(&msg.id) else {
            let idx = self.push_message(msg, true);
            let m = self.messages[idx].clone();
            return self.emit_ws("message.upserted", &m.chat_id, Some(&m.id), None, Some(m.clone()));
        };
        if msg.text.as_deref() == Some(crate::matrix::UTD_TEXT)
            && self.messages[idx].text.as_deref() != Some(crate::matrix::UTD_TEXT)
        {
            return;
        }
        self.update_message(idx, |m| {
            let mut seen = std::mem::take(&mut m.seen);
            seen.extend(msg.seen);
            *m = Message {
                reactions: std::mem::take(&mut m.reactions),
                edit_history: std::mem::take(&mut m.edit_history),
                is_bookmarked: m.is_bookmarked,
                is_unread: m.is_unread,
                seen,
                ..msg
            }
        });
    }

    pub fn update_chat(&mut self, idx: usize, f: impl FnOnce(&mut Chat)) -> Chat {
        let old_key = activity_key(&self.chats[idx]);
        f(&mut self.chats[idx]);
        assert!(self.chat_order.remove(&old_key), "chat {} is in the chat order", old_key.1);
        self.chat_order.insert(activity_key(&self.chats[idx]));
        self.chats[idx].sync_flags();
        self.chats[idx].participants.items.shrink_to_fit();
        self.chats[idx].flags = self.label_flags(&self.chats[idx]);
        self.persist_chat(idx);
        let chat = self.chats[idx].clone();
        self.emit_ws("chat.upserted", &chat.id, None, Some(chat.clone()), None);
        chat
    }

    pub fn update_message(&mut self, idx: usize, f: impl FnOnce(&mut Message)) -> Message {
        f(&mut self.messages[idx]);
        self.messages[idx].sync_flags();
        self.persist_message(idx);
        let msg = self.messages[idx].clone();
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        if self.chats[c_idx].preview.as_ref().is_some_and(|p| p.id == msg.id) {
            match msg.flags & MSG_DELETED {
                0 => self.chats[c_idx].preview = Some(msg.clone()),
                _ => self.chats[c_idx].preview = self.last_shown(&msg.chat_id),
            }
            self.persist_chat(c_idx);
        }
        self.emit_ws("message.upserted", &msg.chat_id, Some(&msg.id), None, Some(msg.clone()));
        msg
    }

    /// Desktop's chat preview after its message is deleted: the newest shown message that is left.
    fn last_shown(&self, chat: &str) -> Option<Message> {
        let threads = self.chat_idx_by_id.get(chat).is_some_and(|&i| self.threaded(&self.chats[i]));
        let mut last = None;
        self.scan_stored("chat_id = ?1 ORDER BY sort_key DESC", [chat], |m| {
            let shown = !m.silent() && !m.thread_only(threads) && m.flags & MSG_DELETED == 0;
            last = shown.then_some(m);
            !shown
        });
        last
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        self.next_seq += 1;
        self.persist_seq();
        format!("{prefix}_{}_{}", crate::config::now_unix_millis(), self.next_seq)
    }

    /// The index in `chats` of the chat with the ID. It panics for an unknown chat.
    pub fn chat_idx(&self, id: &str) -> usize {
        *self.chat_idx_by_id.get(id).unwrap_or_else(|| panic!("{id} is not a known chat"))
    }

    /// The chat with the ID. It panics for an unknown chat.
    pub fn chat(&self, id: &str) -> &Chat {
        &self.chats[self.chat_idx(id)]
    }

    /// The chat with the ID or the local chat ID, or `None` for an unknown chat.
    pub fn find_chat(&self, chat_or_local_id: &str) -> Option<&Chat> {
        self.resolve_chat_idx(chat_or_local_id).map(|i| &self.chats[i])
    }

    /// The index in `chats` of the chat with the ID or the local chat ID, or `None` for an unknown chat.
    pub fn resolve_chat_idx(&self, chat_or_local_id: &str) -> Option<usize> {
        self.chat_idx_by_id.get(chat_or_local_id).or_else(|| self.local_chat_idx_by_id.get(chat_or_local_id)).copied()
    }

    pub fn resolve_canonical_chat_id(&self, chat_or_local_id: &str) -> Option<String> {
        self.resolve_chat_idx(chat_or_local_id).map(|idx| self.chats[idx].id.clone())
    }

    fn send_target(&self, idx: usize) -> usize {
        let Some(merge) = &self.chats[idx].merge else { return idx };
        let member = |id: &str| self.resolve_chat_idx(id).expect("merged member chat exists");
        merge.default_chat_id.as_deref().map(member).unwrap_or_else(|| {
            merge
                .chat_ids
                .iter()
                .map(|id| member(id))
                .max_by(|&a, &b| self.chats[a].last_activity.cmp(&self.chats[b].last_activity))
                .expect("merged chat has member chatIDs")
        })
    }

    pub fn emit_ws(
        &self,
        event_type: &str,
        chat_id: &str,
        message_id: Option<&str>,
        chat: Option<Chat>,
        message: Option<Message>,
    ) {
        self.ws_tx
            .send(WsEvent {
                event_type: event_type.to_string(),
                chat_id: Some(chat_id.to_string()),
                message_id: message_id.map(str::to_string),
                timestamp: now_rfc3339_millis(),
                chat,
                message,
            })
            .ok();
    }

    fn set_merged_into(&mut self, ids: &[String], parent: Option<&str>) {
        for id in ids {
            let idx = self.resolve_chat_idx(id).expect("merged member chat exists");
            self.update_chat(idx, |c| c.merged_into_chat_id = parent.map(str::to_string));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_or_start_chat(
        &mut self,
        account_id: &str,
        chat_type: &str,
        title: Option<String>,
        participant_ids: Vec<String>,
        message_text: Option<String>,
        merge_chat_ids: Option<Vec<String>>,
        default_chat_id: Option<String>,
    ) -> (Chat, String) {
        let id = self.next_id("!chat");
        let items: Vec<User> = participant_ids
            .iter()
            .map(|pid| User {
                id: pid.clone(),
                full_name: Some(pid.trim_start_matches('@').split(':').next().unwrap().to_string()),
                username: Some(pid.clone()),
                email: None,
                phone_number: None,
                img_url: None,
                is_self: Some(false),
                cannot_message: Some(false),
                custom_nickname: None,
                is_network_bot: None,
                ..Default::default()
            })
            .collect();
        let merge = merge_chat_ids.map(|chat_ids| {
            assert!(!chat_ids.is_empty(), "merge chatIDs cannot be empty");
            ChatMergeInfo { chat_ids, default_chat_id }
        });
        let title = title.unwrap_or_else(|| {
            assert!(!items.is_empty(), "a chat without a title needs participants");
            items.iter().filter_map(|u| u.full_name.as_deref()).collect::<Vec<_>>().join(", ")
        });
        let chat = Chat {
            id: id.clone(),
            local_chat_id: Some(format!("local_{}", id.trim_start_matches('!'))),
            inbox_group: chat_type == "group",
            account_id: account_id.to_string(),
            title,
            chat_type: chat_type.to_string(),
            unread_count: 0,
            unread_mentions: 0,
            description: None,
            draft: None,
            is_archived: Some(false),
            is_marked_unread: Some(false),
            is_muted: Some(false),
            is_pinned: Some(false),
            is_low_priority: Some(false),
            label_ids: Vec::new(),
            last_activity: Some(now_rfc3339_millis()),
            last_read_message_sort_key: None,
            merge: merge.clone(),
            merged_into_chat_id: None,
            preview: None,
            reminder: None,
            chat_theme: None,
            note: None,
            notifiable: None,
            is_hidden_from_inbox: None,
            is_request: None,
            created_at: None,
            network_name: None,
            network_id: None,
            img_url: None,
            participants: ChatParticipants { has_more: false, total: items.len(), items },
            delete_modes: 0,
            can_accept_request: false,
            block: 0,
            edits: 0,
            member_actions: 0,
            formatting: ALL_FORMATS,
            file: beeper_files(),
            room_features: false,
            polls: Some(BEEPER_POLLS),
            messages: BEEPER_MESSAGES,
            disappearing: None,
            timers: None,
            older: Older::Unknown,
            is_read_only: false,
            replacement_room: None,
            can_mention_room: false,
            threads: false,
            follows: ThreadFollows::default(),
            thread_reads: BTreeMap::new(),
            personal: None,
            notification_sound: None,
            flags: 0,
        };
        let idx = self.push_chat(chat, true);
        if let Some(m) = &merge {
            self.set_merged_into(&m.chat_ids, Some(&id));
        }
        if let Some(txt) = message_text.filter(|s| !s.trim().is_empty()) {
            self.send_message(&id, Some(txt), None, None).expect("send first message to new chat");
        }
        let chat = self.chats[idx].clone();
        self.emit_ws("chat.upserted", &chat.id, None, Some(chat.clone()), None);
        (chat, "created".to_string())
    }

    /// Desktop's chat update: the draft and the merge of a chat. Returns the changed chat.
    pub fn patch_chat(
        &mut self,
        chat_or_local_id: &str,
        clear_draft: bool,
        new_draft: Option<ChatDraft>,
        new_merge: Option<Option<ChatMergeInfo>>,
    ) -> Result<Chat, Error> {
        let idx = self.resolve_chat_idx(chat_or_local_id).ok_or(Error::ChatNotFound)?;
        let has_content = |d: &ChatDraft| d.text.as_deref().is_some_and(|s| !s.is_empty()) || !d.attachments.is_empty();
        let draft = match new_draft {
            _ if clear_draft => None,
            Some(d) if has_content(&d) && self.chats[idx].draft.as_ref().is_some_and(has_content) => {
                return Err(Error::DraftNotEmpty)
            }
            Some(d) => Some(d).filter(has_content),
            None => self.chats[idx].draft.clone(),
        };
        if let Some(merge) = new_merge {
            let parent = self.chats[idx].id.clone();
            if let Some(old) = self.chats[idx].merge.take() {
                self.set_merged_into(&old.chat_ids, None);
            }
            if let Some(m) = &merge {
                self.set_merged_into(&m.chat_ids, Some(&parent));
            }
            self.chats[idx].merge = merge;
        }
        Ok(self.update_chat(idx, |c| c.draft = draft))
    }

    /// Sends a draft: writes its local echoes and queues them for the homeserver. Returns the chat ID of the send and
    /// the transaction IDs of the echoes, in order.
    pub fn send(&mut self, chat_or_local_id: &str, draft: Draft) -> Result<(String, Vec<String>), Error> {
        let (chat_id, sends) = self.echo_draft(chat_or_local_id, draft)?;
        let txns = sends.iter().map(|out| out.message().expect("a send has an echo").to_string()).collect();
        sends.into_iter().for_each(|out| self.queue(&chat_id, out));
        Ok((chat_id, txns))
    }

    /// Writes the local echoes of a draft, and returns the chat ID of the send and the changes that send the echoes,
    /// in order. [`BeeperStore::send`] queues them at once.
    pub fn echo_draft(&mut self, chat_or_local_id: &str, draft: Draft) -> Result<(String, Vec<Outgoing>), Error> {
        let Draft { text, mut reply_to, mentions, preview, thread, files, links } = draft;
        assert!(text.is_some() || !files.is_empty(), "a draft has text or files");
        let rules = self.resolve_chat_idx(chat_or_local_id).map(|i| self.chats[i].file.clone());
        let captioned = |f: &OutFile| {
            rules.as_ref().is_some_and(|r| crate::media::takes_caption(r, &f.mime, f.gif, f.voice.is_some()))
        };
        let caption = text.clone().filter(|_| matches!(&files[..], [f] if captioned(f)));
        let (mut chat_id, mut sends) = (String::new(), Vec::new());
        if let Some(text) = text.filter(|_| caption.is_none()) {
            let send = |txn, reply_to, thread| Outgoing::Send {
                txn,
                text: text.clone(),
                reply_to,
                preview,
                mentions,
                thread,
                links,
            };
            let (cid, out) =
                self.echo_part(chat_or_local_id, Some(text.clone()), reply_to.take(), None, &thread, send)?;
            if let Outgoing::Send { txn, preview: Some((v, _)), .. } = &out {
                if let Some(link) = crate::matrix::link(v) {
                    let i = self.msg_idx_by_id[txn];
                    self.update_message(i, |m| m.links = vec![link]);
                }
            }
            sends.push(out);
            chat_id = cid;
        }
        for f in files {
            let info = f.info();
            let att = Some(f.attachment(&info));
            let OutFile { bytes, name, mime, gif, pack, .. } = f;
            let (text, caption) = (caption.clone(), caption.clone());
            let media = |txn, reply_to, thread| {
                let sticker = pack;
                Outgoing::Media { txn, bytes, name, mime, info, caption, reply_to, thread, gif, sticker }
            };
            let (cid, out) = self.echo_part(chat_or_local_id, text, reply_to.take(), att, &thread, media)?;
            sends.push(out);
            chat_id = cid;
        }
        Ok((chat_id, sends))
    }

    /// The local echo of one message of a draft, and its change from `send`, which takes the transaction ID, the
    /// quoted message and the thread. In a thread the message quotes the chosen reply, else the newest reply.
    fn echo_part(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        reply_to: Option<String>,
        att: Option<Attachment>,
        thread: &Option<(String, bool)>,
        send: impl FnOnce(String, Option<String>, Option<ThreadSend>) -> Outgoing,
    ) -> Result<(String, Outgoing), Error> {
        let Some((root, broadcast)) = thread else {
            let (chat_id, txn, _) = self.send_message(chat_or_local_id, text, reply_to.clone(), att)?;
            return Ok((chat_id, send(txn, reply_to, None)));
        };
        let chosen = reply_to.filter(|r| r == root || self.thread_replies(root).any(|m| m.id == *r));
        let t = self.thread_send(root, chosen, *broadcast);
        let (chat_id, txn, _) = self.send_in_thread(chat_or_local_id, text, att, &t)?;
        Ok((chat_id, send(txn, Some(t.quoted.clone()), Some(t))))
    }

    /// Adds the local echo of a sent message. Returns the chat ID, the message ID (also the transaction ID) and the echo.
    pub fn send_message(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        reply_to_message_id: Option<String>,
        attachment: Option<Attachment>,
    ) -> Result<(String, String, Message), Error> {
        self.compose(chat_or_local_id, text, reply_to_message_id, attachment, None)
    }

    /// Desktop's `sendThreadedReply`: a reply in the thread `t`, which reads the thread through it.
    pub fn send_in_thread(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        attachment: Option<Attachment>,
        t: &ThreadSend,
    ) -> Result<(String, String, Message), Error> {
        let sent = self.compose(chat_or_local_id, text, Some(t.quoted.clone()), attachment, Some(t))?;
        let idx = self.chat_idx_by_id[&sent.0];
        self.update_chat(idx, |c| drop(c.thread_reads.insert(t.root.clone(), sent.2.sort_key.clone())));
        // Desktop's `refollowThread`: a reply ends an unfollow.
        if self.chats[idx].follows.unfollowed.contains(&t.root) {
            let follows = self.update_chat(idx, |c| c.follows.unfollowed.retain(|r| *r != t.root)).follows;
            self.queue(&sent.0, Outgoing::ThreadFollows(follows));
        }
        Ok(sent)
    }

    fn compose(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        reply_to_message_id: Option<String>,
        attachment: Option<Attachment>,
        thread: Option<&ThreadSend>,
    ) -> Result<(String, String, Message), Error> {
        let c_idx = self.send_target(self.resolve_chat_idx(chat_or_local_id).ok_or(Error::ChatNotFound)?);
        let (chat_id, account_id) = (self.chats[c_idx].id.clone(), self.chats[c_idx].account_id.clone());
        self.load_chat(&chat_id);
        let msg_id = self.next_id("msg");
        let mut attachment = attachment;
        let msg_type = match attachment.as_mut() {
            None => "TEXT",
            Some(att) if att.is_voice_note == Some(true) => "VOICE",
            Some(att) if att.is_sticker == Some(true) => "STICKER",
            Some(att) if att.attachment_type == "img" => "IMAGE",
            Some(att) if att.attachment_type == "video" => "VIDEO",
            Some(att) if att.attachment_type == "audio" => "AUDIO",
            Some(_) => "FILE",
        };
        let msg = Message {
            id: msg_id.clone(),
            account_id: account_id.clone(),
            chat_id: chat_id.clone(),
            sender_id: self.self_id(&account_id),
            sender_name: self.me().full_name.clone(),
            sort_key: sort_key(self.next_seq),
            timestamp: now_rfc3339_millis(),
            html: text.as_deref().and_then(crate::codec::markdown_html),
            text,
            message_type: Some(msg_type.to_string()),
            is_sender: Some(true),
            is_unread: Some(false),
            is_bookmarked: Some(false),
            is_deleted: None,
            removed: false,
            linked_message_id: reply_to_message_id,
            attachments: attachment.into_iter().collect(),
            reactions: Vec::new(),
            poll: None,
            unresolved_media: None,
            edit_history: Vec::new(),
            is_edited: None,
            links: Vec::new(),
            seen: BTreeMap::new(),
            timer: None,
            send_error: None,
            is_delivered: false,
            thread_root: thread.map(|t| t.root.clone()),
            broadcast: thread.is_some_and(|t| t.broadcast),
            fallback: thread.is_some_and(|t| !t.chosen),
            mentions: None,
            send_status: None,
            call: None,
            flags: 0,
        };
        let m_idx = self.push_message(msg, true);
        let msg = self.messages[m_idx].clone();
        self.emit_ws("message.upserted", &chat_id, Some(&msg_id), None, Some(msg.clone()));
        self.emit_ws("chat.upserted", &chat_id, None, Some(self.chats[c_idx].clone()), None);
        Ok((chat_id, msg_id, msg))
    }

    /// The chat ID and index of a message in the chat, or in a member chat of a merged chat. It loads the chat of the
    /// message.
    pub fn message_in_chat(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<(String, usize), Error> {
        let idx = self.resolve_chat_idx(chat_or_local_id).ok_or(Error::ChatNotFound)?;
        let m_idx = self.find_message(message_id).ok_or(Error::MessageNotFound)?;
        let target = self.messages[m_idx].chat_id.clone();
        self.in_chat(idx, &target).then_some((target, m_idx)).ok_or(Error::MessageNotFound)
    }

    /// Whether the chat `idx` shows the messages of the chat `target`: it is the chat, or the merged chat of it.
    pub fn in_chat(&self, idx: usize, target: &str) -> bool {
        let chat = &self.chats[idx];
        let mut members = chat.merge.iter().flat_map(|m| &m.chat_ids);
        chat.id == target || members.any(|id| self.resolve_canonical_chat_id(id).as_deref() == Some(target))
    }

    /// Edits the text of a message here, and queues the edit with its mention pills for the homeserver. Returns the
    /// chat ID of the message and the edited message.
    pub fn edit_message(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        text: String,
        mentions: Vec<Mention>,
    ) -> Result<(String, Message), Error> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let msg = self.update_message(m_idx, |m| {
            m.html = crate::codec::markdown_html(&text);
            if let Some(prev) = m.text.replace(text.clone()) {
                m.edit_history.push(EditHistoryEntry { text: prev, timestamp: now_rfc3339_millis() });
            }
        });
        let caption = !msg.attachments.is_empty();
        self.queue(&chat_id, Outgoing::Edit { target: msg.id.clone(), text, mentions, caption });
        Ok((chat_id, msg))
    }

    /// Deletes a message for everyone: here, and with a redaction that the outbox sends. Returns the chat ID of the
    /// message.
    pub fn redact_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, Error> {
        let chat_id = self.delete_message(chat_or_local_id, message_id)?;
        self.queue(&chat_id, Outgoing::Redact { target: message_id.into() });
        Ok(chat_id)
    }

    /// Deletes a message here only, as for a redaction that a sync shows. Returns the chat ID of the message.
    pub fn delete_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, Error> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let removed = self.messages[m_idx].removed;
        self.update_message(m_idx, |m| {
            removed_from_beeper(m);
            (m.removed, m.is_edited) = (false, Some(true));
        });
        if removed {
            self.clean_up_removals(message_id);
        }
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
    }

    /// Desktop's `removeMessage`: "Remove from Beeper" deletes the message here at once, and the outbox sends the
    /// removal to the removed-messages room with one transaction ID, also after a restart.
    pub fn remove_from_beeper(&mut self, message_id: &str) {
        let m_idx = self.find_message(message_id).expect("a stored message");
        let m = self.update_message(m_idx, removed_from_beeper);
        let txn = format!("~beeper-{}", matrix_sdk::ruma::TransactionId::new());
        self.queue(REMOVED_MESSAGES_ROOM, Outgoing::RemoveFromBeeper { txn, room: m.chat_id, target: m.id });
    }

    /// Desktop's `handleRemovedMessageRoomEvents`: the removal events of the removed-messages room by the event ID of
    /// the removed message, in timeline order.
    pub fn apply_removals(&mut self, removals: Vec<(String, String)>) {
        self.removals.get_or_insert_default();
        for (target, removal) in removals {
            self.removals.as_mut().expect("read above").entry(target.clone()).or_default().push(removal);
            match self.find_message(&target) {
                Some(i) if self.messages[i].is_deleted != Some(true) => {
                    drop(self.update_message(i, removed_from_beeper))
                }
                Some(i) if !self.messages[i].removed => self.clean_up_removals(&target),
                _ => {}
            }
        }
    }

    /// Desktop's `cleanup` of a removed message that a redaction deleted: it redacts the removals of the message.
    fn clean_up_removals(&mut self, target: &str) {
        for removal in self.removals.as_mut().and_then(|r| r.remove(target)).into_iter().flatten() {
            self.queue(REMOVED_MESSAGES_ROOM, Outgoing::Redact { target: removal });
        }
    }

    /// Removes a message from its chat and from the stored messages. Returns the chat ID of the message.
    pub fn remove_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, Error> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        self.messages[m_idx].is_deleted = Some(true);
        self.messages[m_idx].flags |= MSG_DELETED;
        (self.messages[m_idx].text, self.messages[m_idx].html) = (None, None);
        self.msg_idx_by_id.remove(message_id);
        self.chat_msg_indices.get_mut(&chat_id).expect("chat has messages").retain(|&i| i != m_idx);
        if let Some(root) = &self.messages[m_idx].thread_root {
            self.thread_msg_indices.get_mut(root).expect("a thread has replies").retain(|&i| i != m_idx);
        }
        self.exec("DELETE FROM messages WHERE id = ?1", params![message_id]);
        let c_idx = self.chat_idx_by_id[&chat_id];
        if self.chats[c_idx].preview.as_ref().is_some_and(|p| p.id == message_id) {
            self.chats[c_idx].preview = self.last_shown(&chat_id);
            self.persist_chat(c_idx);
        }
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
    }

    /// Desktop's `useThreadedReplies`: the threads feature is on and the room supports threads.
    pub fn threaded(&self, chat: &Chat) -> bool {
        chat.threads && self.feature("enable-threads").on
    }

    /// The `activity_key` of each chat, oldest first: Desktop's chat order `ORDER BY timestamp DESC` read backwards.
    pub fn chat_order(&self) -> &BTreeSet<(i64, String)> {
        &self.chat_order
    }

    /// The loaded replies of the thread of `root` in timeline order.
    pub fn thread_replies(&self, root: &str) -> impl DoubleEndedIterator<Item = &Message> {
        self.thread_msg_indices.get(root).into_iter().flatten().map(|&i| &self.messages[i])
    }

    /// Desktop's `threadSendOptions` for a reply in the thread of `root` to `chosen`, else to its newest message.
    pub fn thread_send(&self, root: &str, chosen: Option<String>, broadcast: bool) -> ThreadSend {
        let newest = || self.thread_replies(root).next_back().map_or_else(|| root.to_string(), |m| m.id.clone());
        ThreadSend { root: root.into(), chosen: chosen.is_some(), quoted: chosen.unwrap_or_else(newest), broadcast }
    }

    /// Desktop's `setThreadFollowing`: follows or unfollows the thread of `root` explicitly.
    pub fn follow_thread(&mut self, chat_id: &str, root: &str, on: bool) {
        let idx = self.chat_idx_by_id[chat_id];
        let follows = self.update_chat(idx, |c| c.follows.set(root, on)).follows;
        self.queue(chat_id, Outgoing::ThreadFollows(follows));
    }

    /// Desktop's `setFollowAllThreads`.
    pub fn follow_all_threads(&mut self, chat_id: &str, on: bool) {
        let idx = self.chat_idx_by_id[chat_id];
        let follows = self.update_chat(idx, |c| c.follows.follow_all = Some(on)).follows;
        self.queue(chat_id, Outgoing::ThreadFollows(follows));
    }

    /// Desktop's `followsThread`. The user takes part in a thread with its root or a reply. It reads SQLite, so that
    /// the chat need not be loaded.
    pub fn follows_thread(&self, chat: &Chat, root: &str) -> bool {
        let mine = format!(
            "chat_id = ?1 AND (id = ?2 OR {THREAD} = ?2) AND json ->> '$.isSender' AND json ->> '$.isDeleted' IS NOT 1 \
             LIMIT 1"
        );
        chat.follows.follows(root, chat.participants.total, !self.stored(&mine, [&chat.id, root]).is_empty())
    }

    /// The stored replies of the thread of `root` in the chat `chat`, oldest first, for a chat that need not be loaded.
    pub fn stored_replies(&self, chat: &str, root: &str) -> Vec<Message> {
        self.stored(&format!("chat_id = ?1 AND {THREAD} = ?2 ORDER BY sort_key"), [chat, root])
    }

    /// Desktop's `unreadCount` and `unreadMentionsCount` of a thread: the counted rows of others after its read mark.
    pub fn thread_unread(&self, chat: &Chat, root: &str) -> (usize, usize) {
        let Some(read) = chat.thread_reads.get(root) else { return (0, 0) };
        let unread = self.thread_rows(chat, root).into_iter().filter(|r| r.key > *read && !r.mine);
        unread.fold((0, 0), |(n, at), r| (n + 1, at + usize::from(r.mention)))
    }

    /// Desktop's rows with `countsAsUnreadInThread`: the live stored replies of the thread, and the reactions to its
    /// replies that Desktop keeps as hidden rows, in a room of 20 members or fewer, of a direct chat or to your reply.
    fn thread_rows(&self, chat: &Chat, root: &str) -> Vec<ThreadRow> {
        let small = chat.participants.total <= 20;
        let replies = self.stored_replies(&chat.id, root).into_iter().filter(|m| m.is_deleted != Some(true));
        replies
            .flat_map(|m| {
                let mine = m.is_sender == Some(true);
                let kept = small && (chat.chat_type != "group" || mine);
                let reactions: Vec<ThreadRow> = (m.reactions.iter().filter(|_| kept))
                    .map(|r| ThreadRow {
                        key: r.sort_key.clone(),
                        id: r.id.clone(),
                        mine: self.is_me(&r.participant_id),
                        mention: false,
                    })
                    .collect();
                let mention = m.flags & MSG_MENTION != 0;
                std::iter::once(ThreadRow { key: m.sort_key, id: m.id, mine, mention }).chain(reactions)
            })
            .collect()
    }

    /// Desktop's `unreadThreadEntries`: the followed threads with unread replies of the chats of `chats`, as the chat
    /// index and the root, the newest reply first.
    pub fn unread_threads(&self, chats: impl IntoIterator<Item = usize>) -> Vec<(usize, String)> {
        let mut all: Vec<(String, usize, String)> = chats
            .into_iter()
            .filter(|&i| self.threaded(&self.chats[i]))
            .flat_map(|i| self.chats[i].thread_reads.keys().map(move |r| (i, r)))
            .filter_map(|(i, r)| {
                let followed = self.thread_unread(&self.chats[i], r).0 > 0 && self.follows_thread(&self.chats[i], r);
                let newest = || self.stored_replies(&self.chats[i].id, r).pop().expect("an unread thread has replies");
                followed.then(|| (newest().timestamp, i, r.clone()))
            })
            .collect();
        all.sort_by(|a, b| b.0.cmp(&a.0));
        all.into_iter().map(|(_, i, r)| (i, r)).collect()
    }

    /// Desktop's `markReplyThreadRead`: an unread thread is read through `through`, else through its newest counted
    /// row, and its thread receipt goes to that event.
    pub fn mark_thread_read(&mut self, chat_id: &str, root: &str, through: Option<&str>) {
        self.load_chat(chat_id);
        let idx = self.chat_idx_by_id[chat_id];
        if self.thread_unread(&self.chats[idx], root).0 == 0 {
            return;
        }
        let (key, event) = match through {
            Some(id) => (self.messages[self.msg_idx_by_id[id]].sort_key.clone(), id.to_string()),
            None => {
                let rows = self.thread_rows(&self.chats[idx], root);
                let last = rows.into_iter().max_by(|a, b| a.key.cmp(&b.key)).expect("an unread thread has rows");
                (last.key, last.id)
            }
        };
        if self.chats[idx].thread_reads[root] >= key {
            return;
        }
        self.update_chat(idx, |c| drop(c.thread_reads.insert(root.into(), key)));
        if event.starts_with('$') {
            self.queue(chat_id, Outgoing::ThreadRead { root: root.into(), event });
        }
    }

    /// Desktop's `getMessageRequestLastViewedAccountID`: the `<bridge ID>.<login ID>` of the bridge login of a chat,
    /// or `hungryserv.<user ID>` for a Beeper chat.
    pub fn request_key(&self, c: &Chat) -> Option<String> {
        if let Some(a) = self.bridge_accounts.iter().find(|a| a.account_id == c.account_id) {
            return Some(format!("{}.{}", a.bridge_id, a.login_id));
        }
        let me = &self.beeper_state.user_id;
        (c.account_id == "hungryserv" && !me.is_empty()).then(|| format!("hungryserv.{me}"))
    }

    /// Desktop's `requestCountsTowardsUnreadBadge`: a shown request that came after the Requests view last showed the
    /// requests of its account, by its `createdAt ?? timestamp`.
    pub fn new_request(&self, c: &Chat) -> bool {
        if c.flags & CHAT_REQUEST == 0 || c.flags & CHAT_HIDDEN != 0 {
            return false;
        }
        let Some(seen) = self.request_key(c).and_then(|k| self.requests_viewed.get(&k).copied()) else { return true };
        let last = || c.last_activity.as_deref().and_then(crate::config::millis_from_rfc3339);
        c.created_at.or_else(last).unwrap_or(0) > seen
    }

    /// Desktop's `updateLastViewedRequestsAt`: the accounts of the requests were seen `now`. Gives the new
    /// `com.beeper.message_requests.last_viewed_at`, or `None` when there is no request.
    pub fn view_requests(&mut self, now: i64) -> Option<serde_json::Value> {
        let requests = self.chats.iter().filter(|c| c.flags & CHAT_REQUEST != 0);
        let keys: HashSet<String> = requests.filter_map(|c| self.request_key(c)).collect();
        if keys.is_empty() {
            return None;
        }
        self.requests_viewed.extend(keys.into_iter().map(|k| (k, now)));
        Some(serde_json::json!(self.requests_viewed))
    }

    /// Desktop's `LAST_VIEWED_REQUESTS_AT` effect: another device's times, each kept when it is newer.
    pub fn merge_requests_viewed(&mut self, content: &serde_json::Value) {
        for (k, at) in content.as_object().into_iter().flatten() {
            if let Some(at) = at.as_i64() {
                let seen = self.requests_viewed.entry(k.clone()).or_default();
                *seen = (*seen).max(at);
            }
        }
    }

    /// Desktop's `labelInboxVisibility`: the labels of a chat override the inbox rule of its account.
    fn label_flags(&self, c: &Chat) -> u16 {
        if !self.feature("labels-v2").on || c.label_ids.is_empty() {
            return c.flags;
        }
        let labels = self.labels.iter().filter(|l| c.label_ids.contains(&l.id));
        let rules: Vec<&str> = labels.filter_map(|l| l.inbox).collect();
        let hidden = rules.contains(&"none") || (rules.contains(&"dmOnly") && c.inbox_group);
        c.flags & !CHAT_HIDDEN | if hidden { CHAT_HIDDEN } else { 0 }
    }

    pub fn feature(&self, key: &str) -> crate::features::Feature {
        self.features.get(key, self.scheduled.len())
    }

    /// Desktop's `markReadOnResponse`: incognito mode is on, and the account can use it.
    pub fn incognito(&self) -> bool {
        self.settings.on("incognito") && self.feature("stealth-mode").on
    }

    /// Desktop's `showIncognitoModeIndicator`.
    pub fn incognito_indicator(&self) -> bool {
        self.incognito() && self.settings.on("incognitoIndicator")
    }

    /// Desktop's `showTypingIndicator`: typing notices go out, unless incognito mode turned them off.
    pub fn typing_notices(&self) -> bool {
        !self.feature("stealth-mode").on || self.settings.on("typingIndicators")
    }

    /// Desktop's `addRecentEmoji`: move `key` to the front with one more use and keep 100 entries.
    pub fn add_recent_emoji(&mut self, key: &str) {
        let n = self.recent_emoji.iter().position(|(k, _)| k == key).map_or(0, |i| self.recent_emoji.remove(i).1);
        self.recent_emoji.insert(0, (key.into(), n + 1));
        self.recent_emoji.truncate(100);
    }

    /// Adds the reaction of the user here, and queues it for the homeserver. Returns the chat ID of the message and the
    /// transaction ID of the reaction.
    pub fn add_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
        img_url: Option<String>,
        transaction_id: Option<String>,
    ) -> Result<(String, String), Error> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let tx_id = transaction_id.unwrap_or_else(|| self.next_id("tx_rx"));
        let rx_id = tx_id.clone();
        let pid = self.self_id(&self.messages[m_idx].account_id);
        self.update_message(m_idx, |m| {
            m.reactions.retain(|r| !(r.participant_id == pid && r.reaction_key == reaction_key));
            m.reactions.push(Reaction {
                id: rx_id,
                participant_id: pid.clone(),
                reaction_key: reaction_key.to_string(),
                animated: Some(false),
                img_url: img_url.clone(),
                sort_key: String::new(),
            });
        });
        let out = Outgoing::React { txn: tx_id.clone(), target: message_id.into(), key: reaction_key.into(), img_url };
        self.queue(&chat_id, out);
        Ok((chat_id, tx_id))
    }

    /// Removes the reactions of the user with the key here, and queues the redaction of the first one for the
    /// homeserver. Returns the chat ID of the message.
    pub fn remove_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
    ) -> Result<String, Error> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let mine = |r: &Reaction| r.reaction_key == reaction_key && self.is_me(&r.participant_id);
        let ids: Vec<String> =
            self.messages[m_idx].reactions.iter().filter(|r| mine(r)).map(|r| r.id.clone()).collect();
        let first = ids.first().ok_or(Error::ReactionNotFound)?.clone();
        self.update_message(m_idx, |m| m.reactions.retain(|r| !ids.contains(&r.id)));
        self.queue(&chat_id, Outgoing::Redact { target: first });
        Ok(chat_id)
    }

    pub fn toggle_bookmark(&mut self, message_id: &str) -> bool {
        let m_idx = self.find_message(message_id).expect("a stored message");
        let m = self.update_message(m_idx, |m| m.is_bookmarked = Some(m.is_bookmarked != Some(true)));
        let on = m.is_bookmarked == Some(true);
        if let Some(room) = self.bookmarks_room.clone() {
            self.queue(&room, Outgoing::Bookmark { room: m.chat_id, target: m.id, on });
        }
        on
    }

    /// Desktop's "Remove Bookmark" in Chat Info, also for a message that is not loaded.
    pub fn remove_bookmark(&mut self, target: &str) {
        if self.find_message(target).is_some() {
            assert!(!self.toggle_bookmark(target), "{target} was bookmarked");
            return;
        }
        let room = self.bookmarks_room.clone().expect("bookmarks of messages that are not loaded come from the room");
        let chat = self.bookmarks[target].room_id.clone();
        self.queue(&room, Outgoing::Bookmark { room: chat, target: target.into(), on: false });
    }

    /// Desktop's `applyBookmark` for the bookmarks room events, in timeline order.
    pub fn apply_bookmarks(&mut self, room: &str, changes: Vec<(String, Option<Bookmark>)>) {
        self.bookmarks_room = Some(room.into());
        for (target, b) in changes {
            let on = b.is_some();
            match b {
                Some(b) => self.bookmarks.insert(target.clone(), b),
                None => self.bookmarks.remove(&target),
            };
            if let Some(i) = self.find_message(&target).filter(|&i| self.messages[i].is_bookmarked != Some(on)) {
                self.update_message(i, |m| m.is_bookmarked = Some(on));
            }
        }
    }

    /// Desktop's `setTranscriptionByMessageEventID`: the transcription of the first attachment of the message `target`.
    /// Like Desktop, it skips a message that is not stored or has no attachment.
    pub fn set_transcription(&mut self, target: &str, t: Transcription) {
        if let Some(i) = self.find_message(target).filter(|&i| !self.messages[i].attachments.is_empty()) {
            self.update_message(i, |m| m.attachments[0].transcription = Some(t));
        }
    }

    /// Desktop's `transcribeAttachment` after the transcription: the transcription of the message `target`, here and
    /// in the transcriptions room.
    pub fn transcribed(&mut self, target: &str, t: Transcription) {
        let i = self.find_message(target).expect("a transcribed message is stored");
        let room = self.messages[i].chat_id.clone();
        self.set_transcription(target, t.clone());
        self.queue(TRANSCRIPTIONS_ROOM, Outgoing::Transcription { room, target: target.into(), transcription: t });
    }

    /// Desktop's `sendPoll`: a pending poll message with the options `1`, `2`, ..., and its poll start event.
    pub fn send_poll(&mut self, chat: &str, question: String, answers: Vec<String>, multiple: bool, undisclosed: bool) {
        assert!(answers.len() >= 2, "a poll has two options");
        let options = answers.into_iter().enumerate();
        let options: Vec<PollOption> =
            options.map(|(i, label)| PollOption { id: (i + 1).to_string(), label, voter_ids: Vec::new() }).collect();
        let max_selections = if multiple { options.len().max(2) as u32 } else { 1 };
        let poll =
            PollData { fallback: question.clone(), question, options, max_selections, undisclosed, ended: false };
        let (chat_id, txn, _) = self.send_message(chat, Some(poll.question.clone()), None, None).expect("send a poll");
        self.update_message(self.msg_idx_by_id[&txn], |m| {
            m.html = None;
            m.message_type = Some("POLL".into());
            m.poll = Some(poll.clone());
        });
        self.queue(&chat_id, Outgoing::Poll { txn, poll });
    }

    /// Desktop's `endPoll`: ends the own poll `message_id`, here and on the homeserver.
    pub fn end_poll(&mut self, chat_id: &str, message_id: &str) {
        self.update_poll(message_id, |p| p.ended = true);
        self.queue(chat_id, Outgoing::EndPoll { target: message_id.into() });
    }

    /// Desktop's `votePoll`: replaces the own answers to the poll `message_id`, here and on the homeserver.
    pub fn vote_poll(&mut self, chat_id: &str, message_id: &str, answers: Vec<String>) {
        let me = self.me().id.clone();
        self.update_poll(message_id, |p| p.respond(&me, &answers));
        self.queue(chat_id, Outgoing::Vote { target: message_id.into(), answers });
    }

    /// Changes the poll `message_id` when it is loaded. Desktop ignores responses to other messages.
    pub fn update_poll(&mut self, message_id: &str, f: impl FnOnce(&mut PollData)) {
        if let Some(i) = self.find_message(message_id).filter(|&i| self.messages[i].poll.is_some()) {
            self.update_message(i, |m| f(m.poll.as_mut().expect("the filter keeps polls")));
        }
    }

    pub fn mark_unread_from_message(&mut self, chat_or_local_id: &str, message_id: &str) {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id).expect("chat message exists");
        let from = self.messages[m_idx].sort_key.clone();
        let later: Vec<usize> =
            self.chat_msg_indices[&chat_id].iter().copied().filter(|&i| self.messages[i].sort_key >= from).collect();
        for &i in &later {
            self.update_message(i, |m| m.is_unread = Some(true));
        }
        self.update_chat(self.chat_idx_by_id[&chat_id], |c| {
            c.unread_count = later.len() as u32;
            c.is_marked_unread = Some(true);
        });
    }

    /// Desktop's `markAsRead` with a message: everything up to it is read, and the receipt goes to it.
    pub fn mark_read_through(&mut self, chat_or_local_id: &str, message_id: &str) {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id).expect("chat message exists");
        let key = self.messages[m_idx].sort_key.clone();
        let indices = self.chat_msg_indices[&chat_id].clone();
        for &i in &indices {
            if self.messages[i].sort_key <= key && self.messages[i].is_unread == Some(true) {
                self.update_message(i, |m| m.is_unread = Some(false));
            }
        }
        let left = indices.iter().filter(|&&i| self.messages[i].is_unread == Some(true)).count() as u32;
        let idx = self.chat_idx_by_id[&chat_id];
        if self.chats[idx].flag(ChatFlag::MarkedUnread) {
            self.queue(&chat_id, Outgoing::Flag(ChatFlag::MarkedUnread, false));
        }
        self.update_chat(idx, |c| {
            c.unread_count = left;
            c.unread_mentions = if left == 0 { 0 } else { c.unread_mentions };
            c.is_marked_unread = Some(false);
            c.last_read_message_sort_key = c.last_read_message_sort_key.clone().max(Some(key));
        });
        if message_id.starts_with('$') {
            self.queue(&chat_id, Outgoing::Read(message_id.into()));
        }
    }

    /// Desktop's `markAsRead`: reads the chat here, and queues the read receipt of its newest event when it had unread
    /// messages. `clear_messages` also takes the unread mark off each message.
    pub fn mark_read(&mut self, idx: usize, clear_messages: bool) {
        let id = self.chats[idx].id.clone();
        if let Some(event) = self.mark_chat_read(&id, clear_messages) {
            self.queue(&id, Outgoing::Read(event));
        }
    }

    /// Reads the chat here only, as the homeserver does for a message that the user sends. Returns the newest event
    /// of the chat when the chat had unread messages.
    pub fn mark_chat_read(&mut self, chat_or_local_id: &str, clear_messages: bool) -> Option<String> {
        let idx = self.resolve_chat_idx(chat_or_local_id)?;
        let chat_id = self.chats[idx].id.clone();
        self.load_chat(&chat_id);
        let indices = self.chat_msg_indices[&chat_id].clone();
        let last_key = indices.last().map(|&i| self.messages[i].sort_key.clone());
        let last_event = indices.iter().rev().map(|&i| &self.messages[i].id).find(|id| id.starts_with('$')).cloned();
        let had_unread = self.chats[idx].flags & CHAT_UNREAD != 0;
        if self.chats[idx].flag(ChatFlag::MarkedUnread) {
            self.queue(&chat_id, Outgoing::Flag(ChatFlag::MarkedUnread, false));
        }
        if had_unread || self.chats[idx].last_read_message_sort_key < last_key {
            self.update_chat(idx, |c| {
                c.unread_count = 0;
                c.unread_mentions = 0;
                c.is_marked_unread = Some(false);
                if last_key.is_some() {
                    c.last_read_message_sort_key = last_key;
                }
            });
        }
        if clear_messages {
            for i in indices {
                if self.messages[i].is_unread == Some(true) {
                    self.update_message(i, |m| m.is_unread = Some(false));
                }
            }
        }
        had_unread.then_some(last_event).flatten()
    }

    pub fn stale_inbox_chats(&self) -> Vec<usize> {
        let only_read = self.settings.pref("archiveScope") == 1;
        let days = crate::prefs::ARCHIVE_AGE_DAYS[self.settings.pref("archiveAge") as usize];
        self.inbox_chats_before(only_read, (now_unix_secs() - days * 86_400) * 1000)
    }

    /// Desktop's archive-all state: the unarchived and unpinned chats, or only the read ones, with no activity since
    /// `cutoff` in Unix milliseconds.
    pub fn inbox_chats_before(&self, only_read: bool, cutoff: i64) -> Vec<usize> {
        let ms = |c: &Chat| c.last_activity.as_deref().map(|t| millis_from_rfc3339(t).expect("RFC 3339 activity"));
        (0..self.chats.len())
            .filter(|&i| {
                let c = &self.chats[i];
                c.merged_into_chat_id.is_none()
                    && c.flags & (CHAT_ARCHIVED | CHAT_PINNED) == 0
                    && !(only_read && c.flags & CHAT_UNREAD != 0)
                    && ms(c).is_some_and(|t| t < cutoff)
            })
            .collect()
    }

    pub fn store_uploaded_asset(&mut self, file_name: String, mime_type: String, bytes: &[u8]) -> StoredAsset {
        assert!(!bytes.is_empty(), "uploaded asset bytes cannot be empty");
        let upload_id = self.next_id("upl");
        let safe_name: String =
            file_name.chars().map(|c| if c.is_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
        let disk_name = format!("{upload_id}_{safe_name}");
        std::fs::write(self.config.assets_dir.join(&disk_name), bytes).expect("write uploaded asset to disk");
        // Desktop's `extractMediaDimensions` reads the size of an image.
        let size = imagesize::blob_size(bytes).ok().filter(|_| mime_type.starts_with("image/"));
        let dim = |n: usize| u32::try_from(n).ok();
        let asset = StoredAsset {
            mxc_url: format!("mxc://beeper.com/{upload_id}"),
            upload_id: upload_id.clone(),
            disk_name,
            file_name,
            mime_type,
            file_size: bytes.len() as u64,
            width: size.and_then(|s| dim(s.width)),
            height: size.and_then(|s| dim(s.height)),
            duration: None,
            created_epoch_secs: now_unix_secs(),
        };
        self.exec(
            "INSERT OR REPLACE INTO assets (upload_id, json) VALUES (?1, ?2)",
            params![asset.upload_id, serde_json::to_string(&asset).unwrap()],
        );
        self.assets_by_mxc.insert(asset.mxc_url.clone(), upload_id.clone());
        self.assets.insert(upload_id, asset.clone());
        asset
    }

    pub fn resolve_asset_path(&self, url: &str) -> Option<(PathBuf, String)> {
        let id = self.assets_by_mxc.get(url).map_or(url, String::as_str);
        match self.assets.get(id) {
            Some(a) => Some((self.config.assets_dir.join(&a.disk_name), a.mime_type.clone())),
            None => crate::platform::file_url_to_path(url)
                .filter(|p| p.exists())
                .map(|p| (p, "application/octet-stream".to_string())),
        }
    }

    /// Reads the stored messages that `q` selects, newest first, until `f` returns false.
    pub fn scan(&self, q: &Query, f: impl FnMut(Message) -> bool) {
        let all = if q.0.is_empty() { "1".to_string() } else { q.0.join(" AND ") };
        self.scan_stored(&format!("{all} ORDER BY sort_key DESC"), rusqlite::params_from_iter(&q.1), f);
    }

    /// Reads the stored messages that `q` selects, newest first by time and then by row like Desktop's
    /// `searchRoomMessages`, with their rows, until `f` returns false.
    pub fn scan_by_time(&self, q: &Query, f: impl FnMut(i64, Message) -> bool) {
        let all = if q.0.is_empty() { "1".to_string() } else { q.0.join(" AND ") };
        self.scan_rows(&format!("{all} ORDER BY {TIME} DESC, n DESC"), rusqlite::params_from_iter(&q.1), f);
    }

    /// The id, chat, Unix seconds and stored bytes of each message.
    pub fn message_sizes(&self) -> Vec<(String, String, i64, u64)> {
        let db = self.db.lock().unwrap();
        let mut q = db
            .prepare("SELECT id, chat_id, json_extract(json, '$.timestamp'), length(json) FROM messages")
            .expect("prepare message sizes");
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get(3)?))).expect("sizes");
        rows.map(|r| {
            let (id, chat, ts, bytes) = r.expect("read a message size");
            (id, chat, millis_from_rfc3339(&ts).expect("message timestamps are RFC 3339") / 1000, bytes)
        })
        .collect()
    }

    /// Desktop's `deletePendingMessage`: removes the uploads `ids` that no draft and no Send Later entry keep.
    pub fn drop_unused_uploads(&mut self, ids: &[String]) {
        let drafts = self.chats.iter().flat_map(|c| c.draft.iter().flat_map(|d| &d.attachments));
        let kept: HashSet<&str> = drafts
            .map(|a| a.upload_id.as_str())
            .chain(self.scheduled.iter().filter_map(|s| s.upload.as_deref()))
            .collect();
        let unused: HashSet<&String> =
            ids.iter().filter(|id| !kept.contains(id.as_str()) && self.assets.contains_key(*id)).collect();
        let unused: Vec<String> = unused.into_iter().cloned().collect();
        self.forget(&HashSet::new(), &unused);
    }

    /// Drops local messages and uploaded files to free storage, like Desktop's `clearStorage`.
    pub fn forget(&mut self, messages: &HashSet<String>, uploads: &[String]) {
        for id in uploads {
            let a = self.assets.remove(id).expect("a stored upload");
            self.assets_by_mxc.remove(&a.mxc_url);
            std::fs::remove_file(self.config.assets_dir.join(&a.disk_name)).expect("remove an uploaded file");
            self.exec("DELETE FROM assets WHERE upload_id = ?1", params![id]);
        }
        if messages.is_empty() {
            return;
        }
        let msgs = &self.messages;
        for list in self.chat_msg_indices.values_mut().chain(self.thread_msg_indices.values_mut()) {
            list.retain(|&i| !messages.contains(&msgs[i].id));
        }
        self.msg_idx_by_id.retain(|id, _| !messages.contains(id));
        self.compact();
        self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin forget transaction");
        for id in messages {
            self.exec("DELETE FROM messages WHERE id = ?1", params![id]);
        }
        self.db.lock().unwrap().execute_batch("COMMIT; VACUUM;").expect("commit forget transaction");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    /// Desktop's `applyAppLocale` at launch: a signed-out app keeps the system language after the sign-in, and a
    /// signed-in app without a language takes English.
    #[test]
    fn app_language_like_desktop() {
        let dir = crate::testing::temp_dir("app-language");
        let config = crate::config::CentralConfig::new(dir, "127.0.0.1".into(), 23392, None);
        let session = || crate::models::MatrixSession {
            user_id: "@me:beeper.com".into(),
            access_token: "t".into(),
            device_id: "D".into(),
            jwt: None,
        };
        let mut st = super::BeeperStore::from_config(config.clone());
        assert_eq!(st.settings.pref("appLanguage"), 0, "a signed-out app takes the system language");
        st.sign_in(session(), "http://127.0.0.1:1".into(), &Default::default(), Vec::new());
        assert_eq!(st.settings.pref("appLanguage"), 0, "and keeps it after the sign-in");
        st.settings.prefs.remove("appLanguage");
        st.persist_state_to_sqlite();
        drop(st);
        let st = super::BeeperStore::from_config(config);
        assert_eq!(st.settings.pref("appLanguage"), 1, "a signed-in app without a language takes English");
    }

    /// The IDs of the stored messages that a message search finds.
    fn found(st: &super::BeeperStore, q: &str) -> HashSet<String> {
        let mut ids = HashSet::new();
        st.scan(&super::Query::default().text(q), |m| ids.insert(m.id) || true);
        ids
    }

    #[test]
    fn widened_sort_keys_match_sort_key() {
        let old = |seq: i64| if seq < 0 { format!("-{:012}", 1_000_000_000_000 + seq) } else { format!("{seq:012}") };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(&format!("{} {}", super::SCHEMA, super::messages_schema())).unwrap();
        let seqs = [-5, -1, 0, 7, 123_456_789];
        for seq in seqs {
            let (key, json) = (old(seq), serde_json::json!({ "sortKey": old(seq) }).to_string());
            db.execute("INSERT INTO messages (id, chat_id, sort_key, json) VALUES (?1, 'c', ?1, ?2)", [&key, &json])
                .unwrap();
            let chat = serde_json::json!({ "preview": { "sortKey": key }, "lastReadMessageSortKey": key });
            db.execute("INSERT INTO chats VALUES (?1, ?2)", [&key, &chat.to_string()]).unwrap();
        }
        db.execute_batch(&super::widen_sort_keys().repeat(2)).unwrap();
        let rows = |sql: &str| -> Vec<String> {
            db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        let want: Vec<String> = seqs.iter().map(|&s| super::sort_key(s)).collect();
        for sql in [
            "SELECT sort_key FROM messages ORDER BY sort_key",
            "SELECT json_extract(json, '$.sortKey') FROM messages ORDER BY sort_key",
            "SELECT json_extract(json, '$.preview.sortKey') FROM chats ORDER BY 1",
            "SELECT json_extract(json, '$.lastReadMessageSortKey') FROM chats ORDER BY 1",
        ] {
            assert_eq!(rows(sql), want, "{sql}");
        }
    }

    #[test]
    fn schema_4_migrates_to_message_search() {
        let config = crate::testing::demo_config("schema-4");
        let fresh = super::BeeperStore::from_config(config.clone());
        let ids = |st: &super::BeeperStore, q| -> HashSet<String> { found(st, q) };
        let want = ids(&fresh, "borr");
        assert_eq!(want.len(), 2, "the demo has a message with \"borrow\" and one with \"borrowed\"");
        drop(fresh);
        let dir = &config.data_dir;
        let v4 = dir.join("v4.db");
        {
            let db = rusqlite::Connection::open(&config.db_path).unwrap();
            db.execute_batch(&format!(
                "ATTACH '{}' AS v4;
                 CREATE TABLE v4.kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
                 CREATE TABLE v4.chats (id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
                 CREATE TABLE v4.messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL) STRICT;
                 CREATE TABLE v4.assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
                 INSERT INTO v4.kv SELECT * FROM kv;
                 INSERT INTO v4.chats SELECT * FROM chats ORDER BY rowid;
                 INSERT INTO v4.messages SELECT id, chat_id, sort_key, json FROM messages ORDER BY n;
                 PRAGMA v4.user_version = 4;",
                v4.display()
            ))
            .unwrap();
        }
        for f in ["beeper-rs.db-wal", "beeper-rs.db-shm"] {
            let _ = std::fs::remove_file(dir.join(f));
        }
        std::fs::rename(v4, &config.db_path).unwrap();
        let st = super::BeeperStore::from_config(config);
        assert_eq!(ids(&st, "borr"), want, "search finds the same messages after the migration");
        assert!(found(&st, "orrow").is_empty(), "only the last word is a prefix");
    }

    #[test]
    fn schema_5_migrates_to_the_outbox() {
        let config = crate::testing::demo_config("schema-5");
        let ids = |st: &super::BeeperStore| -> HashSet<String> { found(st, "borr") };
        let want = ids(&super::BeeperStore::from_config(config.clone()));
        assert_eq!(want.len(), 2);
        // Schema 5 is schema 8 without the outbox table, the secret flags and the message order indices.
        rusqlite::Connection::open(&config.db_path)
            .unwrap()
            .execute_batch("DROP TABLE outbox; DROP INDEX messages_by_key; DROP INDEX messages_with_media; DROP INDEX messages_by_thread; DROP INDEX messages_by_time; PRAGMA user_version = 5;")
            .unwrap();
        let mut st = super::BeeperStore::from_config(config.clone());
        assert_eq!(ids(&st), want, "the message search stays");
        st.queue(super::NOTE_CHAT_ID, super::Outgoing::Flag(super::ChatFlag::Muted, true));
        drop(st);
        let st = super::BeeperStore::from_config(config);
        let version: i64 = st.db.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, super::SCHEMA_VERSION);
        assert!(
            matches!(&st.outbox[..], [(_, c, super::Outgoing::Flag(super::ChatFlag::Muted, true))] if c == super::NOTE_CHAT_ID),
            "the new outbox keeps a change"
        );
    }

    /// Schema 9 changes the `transcript` text of schema 8 to Desktop's `transcription`, and search finds it.
    #[test]
    fn schema_8_migrates_transcripts() {
        let config = crate::testing::demo_config("schema-8");
        let old = "'$.attachments[0].transcript', json ->> '$.attachments[0].transcription.transcription'";
        drop(super::BeeperStore::from_config(config.clone()));
        rusqlite::Connection::open(&config.db_path)
            .unwrap()
            .execute_batch(&format!(
                "UPDATE messages SET json = json_remove(json_set(json, {old}), '$.attachments[0].transcription')
                     WHERE id = 'msg_1000c'; DROP INDEX messages_by_time; PRAGMA user_version = 8;"
            ))
            .unwrap();
        let mut st = super::BeeperStore::from_config(config);
        let i = st.find_message("msg_1000c").unwrap();
        let t = st.messages[i].attachments[0].transcription.clone().expect("a transcription");
        assert_eq!((t.engine.as_str(), t.language), ("unknown", None));
        assert!(t.transcription.starts_with("Native Rust"));
        assert_eq!(found(&st, "instantaneous"), HashSet::from(["msg_1000c".to_string()]));
    }

    /// A start loads no messages. A chat loads its stored messages on first use, and the chats that were used least
    /// recently unload, except the shown chats.
    #[test]
    fn chats_load_on_demand_and_old_chats_unload() {
        let config = crate::testing::demo_config("load");
        drop(super::BeeperStore::from_config(config.clone()));
        let mut st = super::BeeperStore::from_config(config);
        assert!(st.chat_msg_indices.is_empty() && st.messages.is_empty(), "a start loads no messages");
        let fit = st.chats.iter().all(|c| c.participants.items.capacity() == c.participants.items.len());
        assert!(fit, "the participant lists have no spare room");
        assert_eq!(found(&st, "borr").len(), 2, "the search reads the messages that are not loaded");
        let stored = |st: &super::BeeperStore, c: &str| -> Vec<String> {
            st.stored("chat_id = ?1 ORDER BY sort_key", [c]).into_iter().map(|m| m.id).collect()
        };
        let loaded = |st: &super::BeeperStore, c: &str| -> Vec<String> {
            st.chat_msg_indices[c].iter().map(|&i| st.messages[i].id.clone()).collect()
        };
        let chat = st.chats.iter().map(|c| c.id.clone()).find(|c| stored(&st, c).len() > 2).expect("a long chat");
        let preview = |st: &super::BeeperStore| st.chats[st.chat_idx_by_id[&chat]].preview.clone().unwrap().id;
        let last = preview(&st);
        st.delete_message(&chat, &last).unwrap();
        assert_eq!(loaded(&st, &chat), stored(&st, &chat), "a change loads the chat");
        assert!(preview(&st) != last, "the preview of a deleted message moves to an older message");
        let others: Vec<String> = st.chats.iter().map(|c| c.id.clone()).filter(|c| *c != chat).take(3).collect();
        let template = super::Message { thread_root: None, ..st.messages[st.chat_msg_indices[&chat][0]].clone() };
        {
            let db = st.db.lock().unwrap();
            db.execute_batch("BEGIN").unwrap();
            for (k, c) in others.iter().enumerate() {
                for j in 0..2000 {
                    let (id, key) = (format!("bulk_{k}_{j}"), super::sort_key(1_000_000 + (k * 2000 + j) as i64));
                    let m = super::Message { id, chat_id: c.clone(), sort_key: key, ..template.clone() };
                    let json = serde_json::to_string(&m).unwrap();
                    let sql = "INSERT INTO messages (id, chat_id, sort_key, json) VALUES (?1, ?2, ?3, ?4)";
                    db.execute(sql, [&m.id, c, &m.sort_key, &json]).unwrap();
                }
            }
            db.execute_batch("COMMIT").unwrap();
        }
        st.shown = vec![others[0].clone()];
        others.iter().for_each(|c| st.load_chat(c));
        let mut open: Vec<&String> = st.chat_msg_indices.keys().collect();
        open.sort();
        let mut want = vec![&others[0], &others[2]];
        want.sort();
        assert_eq!(open, want, "the shown chat and the last chat stay");
        assert_eq!(st.messages.len(), st.msg_idx_by_id.len());
        assert!(st.msg_idx_by_id.iter().all(|(id, &i)| st.messages[i].id == *id));
        assert!(want.iter().all(|c| loaded(&st, c) == stored(&st, c)));
        assert!(st.find_message(&last).is_some() && st.chat_msg_indices.contains_key(&chat), "a lookup loads");
    }

    #[test]
    fn schema_6_migrates_to_secret_flags() {
        let config = crate::testing::demo_config("schema-6");
        drop(super::BeeperStore::from_config(config.clone()));
        // Schema 6 keeps the secrets as texts, and an empty text is a secret that this device does not have.
        rusqlite::Connection::open(&config.db_path)
            .unwrap()
            .execute_batch(
                "UPDATE kv SET value = json_set(value, '$.secrets.master_key', 'mk', '$.secrets.self_signing_key', '',
                     '$.secrets.user_signing_key', 'usk', '$.secrets.megolm_backup_key', '') WHERE key = 'state';
                 DROP INDEX messages_by_key; DROP INDEX messages_with_media; DROP INDEX messages_by_thread;
                 DROP INDEX messages_by_time; PRAGMA user_version = 6;",
            )
            .unwrap();
        let st = super::BeeperStore::from_config(config);
        let s = &st.beeper_state.secrets;
        assert_eq!(
            [s.master_key, s.self_signing_key, s.user_signing_key, s.megolm_backup_key],
            [true, false, true, false]
        );
    }

    #[test]
    fn a_removed_chat_leaves_the_other_chats_intact() {
        let config = crate::testing::demo_config("remove-chat");
        let mut st = super::BeeperStore::from_config(config);
        let with_messages = |st: &super::BeeperStore| {
            st.chats.iter().map(|c| c.id.clone()).filter(|c| !st.chat_msg_indices[c].is_empty()).collect::<Vec<_>>()
        };
        let (gone, kept) = (with_messages(&st)[0].clone(), with_messages(&st)[1].clone());
        let tombstone = st.messages[st.chat_msg_indices[&kept][0]].id.clone();
        st.remove_message(&kept, &tombstone).unwrap();
        let before: Vec<(String, Vec<String>)> = st
            .chats
            .iter()
            .filter(|c| c.id != gone)
            .map(|c| (c.id.clone(), st.chat_msg_indices[&c.id].iter().map(|&i| st.messages[i].id.clone()).collect()))
            .collect();
        st.remove_chat(&gone);
        let after: Vec<(String, Vec<String>)> = st
            .chats
            .iter()
            .map(|c| (c.id.clone(), st.chat_msg_indices[&c.id].iter().map(|&i| st.messages[i].id.clone()).collect()))
            .collect();
        assert_eq!(after, before);
        assert_eq!(st.messages.len(), st.msg_idx_by_id.len(), "the tombstones and the messages of the chat are gone");
        assert!(st.msg_idx_by_id.iter().all(|(id, &i)| st.messages[i].id == *id));
        assert!(st.chats.iter().enumerate().all(|(i, c)| st.chat_idx_by_id[&c.id] == i));
    }

    #[test]
    fn a_demo_data_folder_stays_offline_without_the_demo_flag() {
        let config = crate::config::CentralConfig { demo: false, ..crate::testing::demo_config("demo-folder") };
        assert!(!super::BeeperStore::from_config(config.clone()).config.demo, "a signed-out folder is not a demo");
        // A new demo config deletes the folder, and the demo store writes its data.
        drop(super::BeeperStore::from_config(crate::testing::demo_config("demo-folder")));
        let st = super::BeeperStore::from_config(config);
        assert!(st.config.demo && st.signed_in(), "the demo account does not start the Matrix sync");
    }

    #[test]
    fn sent_messages_come_last() {
        let config = crate::testing::demo_config("send-order");
        let mut st = super::BeeperStore::from_config(config);
        let chat = st.messages.iter().max_by_key(|m| m.sort_key.clone()).unwrap().chat_id.clone();
        let (_, id, _) = st.send_message(&chat, Some("hi".into()), None, None).unwrap();
        let last = *st.chat_msg_indices[&chat].last().unwrap();
        assert_eq!(st.messages[last].id, id);
        assert_eq!(st.chats[st.chat_idx_by_id[&chat]].preview.as_ref().unwrap().id, id);
    }

    /// Desktop's `hasErroredMessages`: a chat whose send failed shows it until Retry starts.
    #[test]
    fn a_failed_send_marks_its_chat_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("errored"));
        let chat = st.chats.iter().find(|c| c.id != super::NOTE_CHAT_ID).unwrap().id.clone();
        let draft = super::Draft { text: Some("hi".into()), ..Default::default() };
        let (_, txns) = st.send(&chat, draft).unwrap();
        let id = txns[0].clone();
        assert!(!st.has_errored(&chat), "a send on its way");
        let row = st.outbox.iter().rev().find(|o| o.1 == chat).expect("the queued send").0;
        st.outbox.retain(|o| o.0 != row);
        st.pushed(row, Some(&id), Some(("refused".into(), true)));
        assert!(st.has_errored(&chat), "the failed send waits for Retry");
        st.retry_send(&id);
        assert!(!st.has_errored(&chat), "Retry clears it");
    }

    /// Desktop's threaded replies in the demo: a reply only in the thread neither previews the chat nor counts
    /// for an other thread, and an own reply reads the thread.
    #[test]
    fn thread_replies_like_desktop() {
        let config = crate::testing::demo_config("thread-replies");
        let mut st = super::BeeperStore::from_config(config);
        let (chat, root) = ("!sl_infra:beeper.com", "msg_1007");
        let c = st.chats[st.chat_idx_by_id[chat]].clone();
        assert!(st.threaded(&c) && st.follows_thread(&c, root), "a room of 3 members follows all threads");
        assert_eq!(st.thread_replies(root).count(), 4);
        assert_eq!(st.thread_unread(&c, root), (2, 0));
        assert_eq!(c.preview.unwrap().id, "msg_1010", "the reply that is also sent to the chat previews it");
        let t = st.thread_send(root, None, false);
        assert_eq!((t.quoted.as_str(), t.chosen), ("msg_1011", false), "the reply goes to the newest reply");
        let (_, id, msg) = st.send_in_thread(chat, Some("ok".into()), None, &t).unwrap();
        assert_eq!(
            (msg.thread_root.as_deref(), msg.fallback, msg.linked_message_id.as_deref()),
            (Some(root), true, Some("msg_1011"))
        );
        let c = &st.chats[st.chat_idx_by_id[chat]];
        assert_eq!((st.thread_unread(c, root), c.preview.as_ref().unwrap().id.as_str()), ((0, 0), "msg_1010"));
        // Desktop's hidden reaction rows: a reaction to your reply counts as unread in the thread.
        let i = st.msg_idx_by_id[&id];
        let key = format!("{}~", st.messages[i].sort_key);
        let r = super::Reaction {
            id: "$r".into(),
            participant_id: "@dev:x".into(),
            reaction_key: "👍".into(),
            animated: None,
            img_url: None,
            sort_key: key.clone(),
        };
        st.update_message(i, |m| m.reactions.push(r));
        let c = st.chats[st.chat_idx_by_id[chat]].clone();
        assert_eq!(st.thread_unread(&c, root), (1, 0));
        st.mark_thread_read(chat, root, None);
        let c = &st.chats[st.chat_idx_by_id[chat]];
        assert_eq!((st.thread_unread(c, root), &c.thread_reads[root]), ((0, 0), &key), "the reaction is read too");
        st.remove_message(chat, &id).unwrap();
        assert_eq!(st.thread_replies(root).count(), 4, "a removed reply leaves the thread");
        // A room without threads shows a thread reply in the chat, so the reply previews it.
        let idx = st.chat_idx_by_id[chat];
        st.update_chat(idx, |c| c.threads = false);
        let (_, id, _) = st.send_in_thread(chat, Some("in the chat".into()), None, &t).unwrap();
        assert_eq!(st.chats[idx].preview.as_ref().unwrap().id, id, "the reply previews the chat");
    }

    #[test]
    fn labels_set_the_label_ids_of_their_chats() {
        let config = crate::testing::demo_config("labels");
        let mut st = super::BeeperStore::from_config(config);
        let mut plain = st.chats.iter().filter(|c| c.label_ids.is_empty()).map(|c| c.id.clone());
        let (a, b) = (plain.next().unwrap(), plain.next().unwrap());
        let ids = |st: &super::BeeperStore, c: &str| st.chats[st.chat_idx_by_id[c]].label_ids.clone();
        let label =
            super::Label { id: "!work".into(), name: "Work".into(), rooms: vec![a.clone()], ..Default::default() };
        st.set_label(label.clone());
        assert_eq!((ids(&st, &a), ids(&st, &b)), (vec!["!work".to_string()], vec![]));
        st.set_label(super::Label { rooms: vec![b.clone()], ..label });
        assert_eq!((ids(&st, &a), ids(&st, &b)), (vec![], vec!["!work".to_string()]));
        st.remove_chat("!work");
        assert!(ids(&st, &b).is_empty() && st.labels.iter().all(|l| l.id != "!work"), "a left label room drops");
    }
    /// A send writes the echoes and queues their changes in one step. One file takes the text as its caption, a video
    /// keeps its size and length for the homeserver, and the echo of an other file has the kind that a sync shows.
    #[test]
    fn sends_build_the_echo_and_the_change_of_each_file() {
        use super::{Draft, MediaInfo, OutFile, Outgoing};
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("send-files"));
        let chat = "!tg_rust:beeper.com";
        let file = |name: &str, mime: &str| OutFile {
            bytes: vec![1, 2],
            name: name.into(),
            mime: mime.into(),
            ..OutFile::default()
        };
        let video = OutFile { size: Some((3, 2)), duration: Some(1.5), ..file("v.mp4", "video/mp4") };
        let draft = Draft { text: Some("look".into()), files: vec![video], ..Draft::default() };
        let (cid, txns) = st.send(chat, draft).unwrap();
        let i = st.find_message(&txns[0]).unwrap();
        let (m, a) = (&st.messages[i], &st.messages[i].attachments[0]);
        assert_eq!(
            (cid.as_str(), txns.len(), m.text.as_deref(), a.attachment_type.as_str()),
            (chat, 1, Some("look"), "video")
        );
        assert!(
            matches!(&st.outbox[..], [(_, c, Outgoing::Media { info: MediaInfo::Video(3, 2, 1500), caption: Some(t), .. })]
            if c == chat && t == "look")
        );
        let draft =
            Draft { text: Some("two".into()), files: vec![file("a.zip", "application/zip"); 2], ..Draft::default() };
        let (_, txns) = st.send(chat, draft).unwrap();
        let kinds: Vec<_> = txns
            .iter()
            .map(|t| st.messages[st.msg_idx_by_id[t]].attachments.first().map(|a| a.attachment_type.clone()))
            .collect();
        assert_eq!(
            kinds,
            [None, Some("unknown".into()), Some("unknown".into())],
            "the text and each file go as one message each"
        );
        assert_eq!(st.outbox.len(), 4);
    }
    #[test]
    fn bookmarks_follow_the_bookmarks_room() {
        let config = crate::testing::demo_config("bookmarks");
        let mut st = super::BeeperStore::from_config(config);
        let m = st.messages[0].clone();
        assert!(st.toggle_bookmark(&m.id) && st.outbox.is_empty(), "without the bookmarks room a bookmark is local");
        let b = super::Bookmark { room_id: m.chat_id.clone(), event_id: "$b".into(), order: 1 };
        st.apply_bookmarks("!bookmarks", vec![(m.id.clone(), None), ("$other".into(), Some(b.clone()))]);
        assert_eq!(st.messages[0].is_bookmarked, Some(false), "the room removes the local bookmark");
        st.apply_bookmarks("!bookmarks", vec![(m.id.clone(), Some(b))]);
        assert_eq!(st.messages[0].is_bookmarked, Some(true));
        assert!(!st.toggle_bookmark(&m.id));
        assert!(matches!(&st.outbox[..], [(_, r, super::Outgoing::Bookmark { room, target, on: false })]
            if r == "!bookmarks" && *room == m.chat_id && *target == m.id));
        st.upsert_message(super::Message { is_bookmarked: None, ..m.clone() });
        assert_eq!(st.messages[0].is_bookmarked, Some(false), "a synced message keeps its bookmark state");
        st.remove_bookmark("$other");
        assert!(
            matches!(&st.outbox[1], (_, r, super::Outgoing::Bookmark { room, target, on: false })
            if r == "!bookmarks" && *room == m.chat_id && target == "$other"),
            "a message that is not loaded"
        );
    }
    /// Desktop's `setTranscription`: the message shows the transcription, and the transcriptions room gets it.
    #[test]
    fn transcriptions_go_to_the_transcriptions_room() {
        let config = crate::testing::demo_config("transcribed");
        let mut st = super::BeeperStore::from_config(config);
        let t = super::Transcription { transcription: "Hi".into(), engine: "openai-whisper".into(), language: None };
        st.transcribed("msg_1003", t.clone());
        let i = st.find_message("msg_1003").unwrap();
        assert_eq!(st.messages[i].attachments[0].transcription.as_ref(), Some(&t));
        assert!(matches!(&st.outbox[..], [(_, r, super::Outgoing::Transcription { room, target, transcription })]
            if r == super::TRANSCRIPTIONS_ROOM && room == "!tg_rust:beeper.com" && target == "msg_1003" && *transcription == t));
    }
    /// Desktop's `updateThread("chatNotificationSound")`: the `com.beeper.notification_sound` room account data, empty
    /// for the Default sound.
    #[test]
    fn chat_notification_sound_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("chat-sound"));
        let id = "!tg_rust:beeper.com";
        st.set_notification_sound(id, Some("silent".into()));
        assert_eq!(st.chat(id).notification_sound.as_deref(), Some("silent"));
        st.set_notification_sound(id, None);
        assert_eq!(st.chat(id).notification_sound, None);
        let sent: Vec<_> = st.outbox.iter().map(|(_, r, o)| (r.as_str(), o)).collect();
        assert!(matches!(&sent[..], [(r, super::Outgoing::RoomData(k, on)), (_, super::Outgoing::RoomData(_, off))]
            if *r == id && k == super::NOTIFICATION_SOUND && *on == serde_json::json!({ "sound": "silent" }) && *off == serde_json::json!({})));
    }

    #[test]
    fn personal_name_and_photo_like_desktop() {
        let config = crate::testing::demo_config("personal");
        let mut st = super::BeeperStore::from_config(config);
        let id = "!tg_rust:beeper.com";
        let chat = |st: &super::BeeperStore| st.chats[st.chat_idx_by_id[id]].clone();
        let (room, img) = (chat(&st).title, chat(&st).img_url);
        let photo = ("file:///me.png".to_string(), vec![1, 2], "image/png".to_string());
        st.set_personal(id, Some(Some("Mine".into())), Some(Some(photo)));
        let c = chat(&st);
        assert_eq!((c.title.as_str(), c.img_url.as_deref()), ("Mine", Some("file:///me.png")));
        assert_eq!((c.room_title(), c.room_img_url()), (room.as_str(), img.as_deref()));
        assert!(
            matches!(&st.outbox[..], [(_, r, super::Outgoing::RoomData(k, v)), (_, _, super::Outgoing::PersonalAvatar(Some((b, m))))]
            if r == id && k == super::PERSONAL_NAME && *v == serde_json::json!({ "name": "Mine" }) && *b == [1, 2] && m == "image/png")
        );
        st.update_group(id, Some("Team".into()), None, None);
        let c = chat(&st);
        assert_eq!(
            (c.title.as_str(), c.room_title()),
            ("Mine", "Team"),
            "the personal name stays over a new room name"
        );
        st.set_personal(id, Some(None), Some(None));
        let c = chat(&st);
        assert_eq!((c.title.as_str(), c.img_url.as_deref(), c.personal.is_none()), ("Team", img.as_deref(), true));
        assert!(
            matches!(&st.outbox[3..], [(_, _, super::Outgoing::RoomData(k, v)), (_, _, super::Outgoing::PersonalAvatar(None))]
            if k == super::PERSONAL_NAME && *v == serde_json::json!({}))
        );
    }
}

use crate::config::{
    assert_not_original_beeper_dir, millis_from_rfc3339, now_rfc3339_millis, now_unix_millis, now_unix_secs,
    rfc3339_from_millis, CentralConfig, DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE, DEFAULT_TOKEN,
    DEFAULT_USER_ID, SEED_JSON, WS_CHANNEL_CAPACITY,
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

    /// Whether a thread panicked while it held the write lock, so that the store cannot be locked again.
    pub fn poisoned(&self) -> bool {
        self.0.is_poisoned()
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

/// How long a typing update shows its users at most: Desktop's `durationMs: 6e4` of a `m.typing`, whatever the
/// timeout that the typing users sent.
pub const TYPING_DURATION: std::time::Duration = std::time::Duration::from_secs(60);
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

const SCHEMA_VERSION: i64 = 11;
const SCHEMA: &str = "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
     CREATE TABLE chats (id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
     CREATE TABLE assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;";

/// Schema 11 keeps, for the start pass of the next run, the JSON of the reads and of the members of each joined room
/// as its last read gave them (`matrix::start_pass`), and indexes the messages that wait for their keys, which the
/// start pass looks for in each chat.
fn start_schema() -> String {
    format!(
        "CREATE TABLE room_reads (room_id TEXT PRIMARY KEY, reads TEXT, members TEXT) STRICT;
         CREATE INDEX messages_waiting ON messages (chat_id) WHERE {};",
        waiting()
    )
}
/// The messages that wait for their keys ([`crate::matrix::UTD_TEXT`]).
fn waiting() -> String {
    format!("json ->> '$.text' = '{}'", crate::matrix::UTD_TEXT)
}
/// The `kv` key of the mark of the last sync that the chats show (`matrix::sync_mark`).
const KEY_SYNCED: &str = "synced";
/// The JSON of the reads and of the members that SQLite keeps of each room, see [`BeeperStore::kept_rooms`].
pub(crate) type KeptRooms = HashMap<String, (Option<String>, Option<String>)>;

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
    (activity_ms(c), c.id.clone())
}

/// The time of [`activity_key`].
pub fn activity_ms(c: &Chat) -> i64 {
    let ms = c.last_activity.as_deref().map(|t| millis_from_rfc3339(t).expect("RFC 3339 activity"));
    let draft = c.draft.as_ref().and_then(|d| d.ts);
    ms.unwrap_or(0).max(draft.unwrap_or(0))
}

/// What a merged chat shows of its member chats.
struct UnionRow {
    preview: Option<Message>,
    activity: Option<String>,
    unread: u32,
    mentions: u32,
    archived: bool,
}

/// Desktop's `G2`, the order of the merged messages of a merged chat: by time, then chat, then sort key.
fn merged_order(m: &Message) -> (i64, &str, &str) {
    (millis_from_rfc3339(&m.timestamp).expect("message timestamp"), &m.chat_id, &m.sort_key)
}

/// Desktop's `markRemovedFromBeeper` and `deleteRemovedMessageData`: a deleted message without its content.
fn removed_from_beeper(m: &mut Message) {
    (m.is_deleted, m.removed) = (Some(true), true);
    (m.text, m.html, m.poll) = (None, None, None);
    m.edit_history.clear();
    m.attachments.clear();
    m.reactions.clear();
    m.links.clear();
    (m.view_limit, m.views_left) = (None, None);
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
        self.phrase(text, true)
    }

    /// Desktop's `pT`: the messages that contain `text` as a phrase, where with `partial` the last word can be the
    /// start of a word.
    pub fn phrase(self, text: &str, partial: bool) -> Self {
        assert!(!text.trim().is_empty(), "a message search has words");
        let phrase = format!("\"{}\"{}", text.trim().replace('"', "\"\""), if partial { "*" } else { "" });
        self.and("n IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)", phrase)
    }

    /// Desktop's `bookmarkedOnly`: the bookmarked messages.
    pub fn bookmarked(mut self) -> Self {
        self.0.push("IFNULL(json ->> '$.isBookmarked', 0) = 1".into());
        self
    }

    /// Desktop's `isSentByMe` condition of `from:me` (`me`) and `from:others`.
    pub fn sent(mut self, me: bool) -> Self {
        self.0.push(format!("IFNULL(json ->> '$.isSender', 0) = {}", u8::from(me)));
        self
    }

    /// Desktop's `has:` condition: an attachment for `None`, else an attachment of one of the `types`, where the
    /// type `link` takes the messages with link previews too.
    pub fn has(mut self, types: Option<&[&str]>) -> Self {
        let Some(types) = types else {
            self.0.push("json_array_length(messages.json, '$.attachments') > 0".into());
            return self;
        };
        let typed = "EXISTS (SELECT 1 FROM json_each(messages.json, '$.attachments') a \
                     WHERE a.value ->> '$.type' IN (SELECT value FROM json_each(?)))";
        let cond = match types.contains(&"link") {
            true => format!("({typed} OR json_array_length(messages.json, '$.links') > 0)"),
            false => typed.into(),
        };
        self.and(&cond, serde_json::to_string(types).expect("attachment type JSON"))
    }

    /// Desktop's `after:` (`after`) or `before:` condition: the messages strictly after or before the Unix
    /// milliseconds `ms`.
    pub fn time(self, ms: i64, after: bool) -> Self {
        let op = if after { '>' } else { '<' };
        self.and(&format!("unixepoch(json ->> '$.timestamp', 'subsec') * 1000 {op} CAST(? AS INTEGER)"), ms.to_string())
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
    /// The URL of the first frame that the local echo of a video shows until the homeserver has the file.
    pub echo_poster: Option<String>,
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
            poster_img: self.echo_poster.clone(),
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
    /// Counts the chats added and removed and the changes of `update_chat`, but not the previews of new messages, so
    /// that a view can keep what it finds from the people and kinds of the chats over the writes of a sync.
    pub chat_changes: u64,
    /// This store among the stores of the process, so that a cache of values found from its chats keys them by
    /// `(instance, chat_changes)`.
    pub instance: u64,
    pub(crate) chat_idx_by_id: HashMap<String, usize>,
    /// The `activity_key` of each chat, for Desktop's chat order `ORDER BY timestamp DESC`.
    chat_order: BTreeSet<(i64, String)>,
    local_chat_idx_by_id: HashMap<String, usize>,
    /// The messages of the loaded chats. SQLite keeps all messages, and `load_chat` loads the messages of a chat.
    /// `update_message` changes one, which moves the timeline revision of its chat.
    pub messages: Vec<Message>,
    pub(crate) msg_idx_by_id: HashMap<String, usize>,
    /// The messages of each loaded chat in timeline order. A chat that is not loaded has no entry.
    pub(crate) chat_msg_indices: HashMap<String, Vec<usize>>,
    /// The loaded chats, the chat that was used last at the end.
    loaded: Vec<String>,
    /// The chat that [`Self::with_chat`] writes into without loading it, the length of `messages` and the count of
    /// `moves` when it started. The messages of the chat that it reads or adds come after that length: they are in
    /// `msg_idx_by_id`, but in no message list.
    unloaded: Option<(String, usize, u64)>,
    /// The chats that the windows show, which stay loaded.
    pub shown: Vec<String>,
    /// The replies of each thread in timeline order, by the ID of the thread root.
    thread_msg_indices: HashMap<String, Vec<usize>>,
    /// The timeline revision of each chat whose messages changed: the count of `timeline_writes` at the last message
    /// that was added to the chat, changed or removed, so that a view can keep what it finds from the messages of a
    /// chat until they change.
    timeline_revs: HashMap<String, u64>,
    timeline_writes: u64,
    /// Counts the compactions of `messages` that moved the indices of messages.
    moves: u64,
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
    /// An error for the toast of the main window.
    pub matrix_error: Option<String>,
    /// The last sync of the Matrix sync loop failed, so the homeserver may not be reachable. Like Desktop's SDK, the
    /// loop tries again quietly, and the outbox goes again when a sync works.
    pub sync_failing: bool,
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
    /// Desktop's `bookmarkStore.status` "failed": the history of the bookmarks room did not load.
    pub bookmarks_failed: bool,
    /// The bookmarks of the bookmarks room by the event ID of the message.
    pub bookmarks: HashMap<String, Bookmark>,
    /// The bookmarked messages that are not loaded, as Chat Info fetched them, or None when they are unavailable.
    pub bookmarked: HashMap<String, Option<Message>>,
    /// Desktop's reminders room, after its history is read: its `messageRemindersStatus` is "restored".
    pub reminders_room: Option<String>,
    /// The active message reminders of the reminders room by the event ID of the message.
    pub message_reminders: HashMap<String, MessageReminder>,
    /// The latest reminder event of each message with a reminder, which a dismissal redacts.
    pub reminder_events: HashMap<String, String>,
    /// Desktop's `mx_removed_messages` after the history of the removed-messages room is read: the removal events by
    /// the event ID of the removed message.
    pub removals: Option<HashMap<String, Vec<String>>>,
    /// Wakes the push task of the Matrix sync loop when the outbox has a new change.
    pub(crate) outbox_ready: std::sync::Arc<tokio::sync::Notify>,
    /// A sync response holds the SQLite transaction of [`Self::begin_sync`].
    sync_tx: bool,
    /// Flags whose change the homeserver has, with the value and the sync after which a sync shows the change.
    pub sent_flags: HashMap<(String, ChatFlag), (bool, u64)>,
    /// The number of syncs of the Matrix sync loop.
    pub syncs: u64,
    /// The Matrix IDs of sent events and created labels by their local IDs.
    pub sent: HashMap<String, String>,
    /// Who types in each chat, by its Matrix `m.typing`.
    pub typing: HashMap<String, Vec<String>>,
    /// The number of `m.typing` updates of each chat, which a typing timeout compares with its own.
    pub typing_updates: HashMap<String, u64>,
    /// `io.element.recent_emoji` keys (emoji or emoji-mart ids) with their use counts.
    pub recent_emoji: Vec<(String, u64)>,
    /// Desktop's `accountDataStore.imagePack`: the own `m.image_pack`, whose images are the custom emoji, or null.
    pub image_pack: serde_json::Value,
    pub scheduled: Vec<Scheduled>,
    /// The last synced global account data of each event in [`crate::prefs::SYNCED`].
    pub synced_prefs: BTreeMap<&'static str, serde_json::Value>,
    /// `com.beeper.emoji_skin_tone`, 0 is the default yellow.
    pub skin_tone: u8,
    /// The last synced [`crate::matrix::RULES`].
    pub inbox_rules: serde_json::Value,
    /// Desktop's `mutedRoomIDs`: the `room_ids` of the global `com.beeper.muted` account data.
    pub muted_rooms: Vec<String>,
    /// The state events and the room account data that the chats of the rooms read from the SDK, between syncs.
    pub(crate) room_reads: crate::matrix::RoomReadsCache,
    /// The last synced [`crate::matrix::ACCOUNT_OVERRIDES`], encrypted with MSC4483.
    pub account_overrides_synced: serde_json::Value,
    /// Desktop SDK's decrypted `com.beeper.bridge_account_overrides`: the account nicknames by bridge and login, or
    /// the synced content while it cannot be decrypted.
    pub account_overrides: serde_json::Value,
    /// Desktop SDK's MSC4483 key of encrypted account data, once read from secret storage.
    pub account_data_key: Option<[u8; 32]>,
    /// Desktop's `lastViewedRequestsAtByAccount`: when the Requests view last showed each account's requests, in
    /// milliseconds, by [`BeeperStore::request_key`].
    pub requests_viewed: HashMap<String, i64>,
    pub features: crate::features::Flags,
    pub whoami: Profile,
    /// The state of the drafts sync of `com.beeper.draft` (see [`crate::drafts`]).
    pub drafts: crate::drafts::DraftSync,
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
                    "BEGIN; {SCHEMA} {} {OUTBOX} {} {} {} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;",
                    messages_schema(),
                    order_schema(),
                    time_schema(),
                    start_schema()
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
                    start_schema(),
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
        let mut store = crate::with_defaults!(Self {
            config,
            db: Mutex::new(conn),
            beeper_state,
            instance: {
                static STORES: AtomicU64 = AtomicU64::new(0);
                STORES.fetch_add(1, Ordering::Relaxed)
            },
            connect_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            ws_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            focus_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            launch_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            api_on: tokio::sync::watch::channel(false).0,
            initial_sync: (0, 0),
            image_pack: serde_json::Value::Null,
            inbox_rules: serde_json::Value::Null,
            account_overrides_synced: serde_json::Value::Null,
            account_overrides: serde_json::Value::Null,
            features,
            next_seq: 1000,
        }, default: setup, settings, verification, bridge_accounts, pin_rank, recent_chat_ids, network_spaces,
            contacts, bridge_sessions, accounts, labels, chats, chat_changes, chat_idx_by_id, chat_order,
            local_chat_idx_by_id, messages, msg_idx_by_id, chat_msg_indices, loaded, unloaded, shown,
            thread_msg_indices, timeline_revs, timeline_writes, moves, assets, assets_by_mxc, connect, matrix,
            matrix_error, sync_failing, session_ended, syncing, sas_confirm, pending_recovery_key, ui_auth_token,
            outbox, bookmarks_room, bookmarks_failed, bookmarks, bookmarked, reminders_room, message_reminders,
            reminder_events, removals, outbox_ready, sync_tx, sent_flags, syncs, sent, typing, typing_updates, recent_emoji, scheduled,
            synced_prefs, skin_tone, muted_rooms, room_reads, account_data_key, requests_viewed, whoami, drafts);
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
        self.put_json(KEY_STATE, &self.beeper_state);
        self.put_json(KEY_BRIDGES, &self.bridge_accounts);
        self.put_json(KEY_SETTINGS, &self.settings);
        self.put_json(KEY_PIN_RANK, &self.pin_rank);
        self.put_json(KEY_RECENT_CHATS, &self.recent_chat_ids);
        self.put_json(KEY_CONTACTS, &self.contacts);
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
        // Desktop's `loadUnloadedMembers`: a merged chat shows the messages of its member chats.
        let members = self.chat_idx_by_id.get(id).map(|&i| self.members(i)).unwrap_or_default();
        let members: Vec<String> = members.into_iter().map(|i| self.chats[i].id.clone()).collect();
        for c in members.iter().map(String::as_str).chain([id]) {
            if self.is_unloaded(c) {
                self.detach();
            }
            self.loaded.retain(|l| l != c);
            self.loaded.push(c.into());
            if !self.chat_msg_indices.contains_key(c) {
                assert!(self.chat_idx_by_id.contains_key(c), "load the messages of the unknown chat {c}");
                self.chat_msg_indices.insert(c.into(), Vec::new());
                for m in self.stored("chat_id = ?1 ORDER BY sort_key", [c]) {
                    self.insert_message(m);
                }
            }
        }
        self.unload_old();
    }

    /// Runs `f`, which writes into the chat `chat`, with the stored messages `ids` of the chat in `messages`, so that
    /// `loaded_idx` finds them like those of a loaded chat. A loaded chat stays loaded and counts as used, like after
    /// [`Self::load_chat`]. A chat that is not loaded stays unloaded, so that a sync does not read all its messages
    /// for a new one: `f` finds the messages `ids` of it, the ones that `upsert_message` changes and the ones that it
    /// adds, but no message list has them, and they leave `messages` when `f` ends. Each write of a message goes to
    /// SQLite, so the stored messages of the chat stay all of its messages.
    pub fn with_chat<R>(&mut self, chat: &str, ids: &[&str], f: impl FnOnce(&mut Self) -> R) -> R {
        if self.chat_msg_indices.contains_key(chat) || self.unloaded.is_some() {
            self.load_chat(chat);
            return f(self);
        }
        assert!(self.chat_idx_by_id.contains_key(chat), "write into the unknown chat {chat}");
        self.unloaded = Some((chat.into(), self.messages.len(), self.moves));
        if !ids.is_empty() {
            let filter = format!("chat_id = ?1 AND id IN ({})", vec!["?"; ids.len()].join(", "));
            self.attach_stored(&filter, rusqlite::params_from_iter(std::iter::once(chat).chain(ids.iter().copied())));
        }
        let r = f(self);
        self.detach();
        r
    }

    /// Whether `chat` is the chat that [`Self::with_chat`] writes into without loading it.
    fn is_unloaded(&self, chat: &str) -> bool {
        self.unloaded.as_ref().is_some_and(|u| u.0 == chat)
    }

    /// Puts the stored messages of the unloaded chat that `filter` selects into `messages`, unless they are there,
    /// and returns their indices in the order of `filter`.
    fn attach_stored(&mut self, filter: &str, args: impl rusqlite::Params) -> Vec<usize> {
        let found = self.stored(filter, args);
        let attach = |st: &mut Self, m: Message| {
            assert!(st.is_unloaded(&m.chat_id), "{} is no message of the unloaded chat", m.id);
            let idx = st.messages.len();
            st.msg_idx_by_id.insert(m.id.clone(), idx);
            st.messages.push(m);
            idx
        };
        found.into_iter().map(|m| self.msg_idx_by_id.get(&m.id).copied().unwrap_or_else(|| attach(self, m))).collect()
    }

    /// Ends [`Self::with_chat`] in a chat that is not loaded: its messages leave `messages`.
    fn detach(&mut self) {
        let Some((chat, base, moves)) = self.unloaded.take() else { return };
        if moves == self.moves {
            for (i, m) in self.messages.iter().enumerate().skip(base) {
                if m.chat_id == chat && self.msg_idx_by_id.get(&m.id) == Some(&i) {
                    self.msg_idx_by_id.remove(&m.id);
                }
            }
            if self.messages[base..].iter().all(|m| m.chat_id == chat) {
                return self.messages.truncate(base);
            }
        } else {
            self.msg_idx_by_id.retain(|_, i| self.messages[*i].chat_id != chat);
        }
        self.compact();
    }

    /// The index of the message `id` of the chat `chat`: a loaded message of it, or in [`Self::with_chat`] a stored
    /// message of the unloaded chat, which goes into `messages`.
    fn chat_message(&mut self, chat: &str, id: &str) -> Option<usize> {
        match self.loaded_idx(id) {
            Some(i) => Some(i).filter(|&i| self.messages[i].chat_id == chat),
            None if self.is_unloaded(chat) => self.attach_stored("chat_id = ?1 AND id = ?2", [chat, id]).pop(),
            None => None,
        }
    }

    /// The first message of the chat in timeline order with the reaction `reaction`, in [`Self::with_chat`] also of
    /// a chat that is not loaded.
    pub fn reacted_message(&mut self, chat: &str, reaction: &str) -> Option<usize> {
        if self.is_unloaded(chat) {
            let filter = "chat_id = ?1 AND instr(json, '\"reactions\"') > 0 AND EXISTS (SELECT 1 FROM \
                          json_each(messages.json, '$.reactions') r WHERE r.value ->> '$.id' = ?2) \
                          ORDER BY sort_key, n LIMIT 1";
            return self.attach_stored(filter, [chat, reaction]).pop();
        }
        let has = |i: &usize| self.messages[*i].reactions.iter().any(|r| r.id == reaction);
        self.timeline_indices(chat).iter().copied().find(has)
    }

    /// The order of the newest message of the chat that is no membership change, and the newest order of the
    /// reactions to its messages, in [`Self::with_chat`] also of a chat that is not loaded.
    pub fn newest_orders(&self, chat: &str) -> (Option<String>, Option<String>) {
        if !self.is_unloaded(chat) {
            let msgs = self.timeline(chat);
            let last = msgs.clone().rev().find(|m| m.message_type.as_deref() != Some("MEMBERSHIP"));
            let reacted = msgs.flat_map(|m| &m.reactions).map(|r| &r.sort_key).max();
            return (last.map(|m| m.sort_key.clone()), reacted.cloned());
        }
        let db = self.db.lock().unwrap();
        let one = |sql: &str| -> Option<String> {
            let mut q = db.prepare_cached(sql).expect("prepare an order read");
            q.query_row([chat], |r| r.get(0)).optional().expect("read an order").flatten()
        };
        (
            one("SELECT sort_key FROM messages WHERE chat_id = ?1 AND (json ->> '$.type') IS NOT 'MEMBERSHIP' \
                 ORDER BY sort_key DESC LIMIT 1"),
            // A reaction sent here has no order yet: the empty text, which sorts first.
            one("SELECT max(coalesce(r.value ->> '$.sortKey', '')) FROM messages m, json_each(m.json, '$.reactions') r \
                 WHERE m.chat_id = ?1 AND instr(m.json, '\"reactions\"') > 0"),
        )
    }

    /// The indices of the messages of the chat in timeline order that a read receipt of one of the `events` can
    /// change: all loaded ones, or in [`Self::with_chat`] in a chat that is not loaded, the stored ones that someone
    /// has seen, that have a disappearing timer, or that have one of the events as their ID or a reaction.
    pub fn receipt_messages(&mut self, chat: &str, events: &[&str]) -> Vec<usize> {
        if !self.is_unloaded(chat) {
            return self.timeline_indices(chat).to_vec();
        }
        let event: String = (2..events.len() + 2).map(|n| format!(" OR id = ?{n} OR instr(json, ?{n}) > 0")).collect();
        let filter = format!(
            "chat_id = ?1 AND (instr(json, '\"seen\"') > 0 OR instr(json, '\"disappearingTimer\"') > 0{event}) \
             ORDER BY sort_key, n"
        );
        self.attach_stored(&filter, rusqlite::params_from_iter(std::iter::once(chat).chain(events.iter().copied())))
    }

    /// Whether the message `id` is stored, loaded or not.
    pub fn has_message(&self, id: &str) -> bool {
        self.msg_idx_by_id.contains_key(id) || {
            let db = self.db.lock().unwrap();
            let mut q = db.prepare_cached("SELECT 1 FROM messages WHERE id = ?1").expect("prepare a message lookup");
            q.exists([id]).expect("look up a message")
        }
    }

    /// With more than `LOADED_MESSAGES` loaded messages, unloads the chats that were used least recently down to half
    /// of them, except the chat that was used last and the shown chats with their member chats. So one unload frees
    /// the room for many loads.
    fn unload_old(&mut self) {
        let mut live = self.msg_idx_by_id.len();
        if live <= LOADED_MESSAGES {
            return;
        }
        let shown = self.shown.iter().filter_map(|c| self.chat_idx_by_id.get(c));
        let shown: HashSet<&str> =
            shown.flat_map(|&i| self.members(i).into_iter().chain([i])).map(|i| self.chats[i].id.as_str()).collect();
        let old = self.loaded[..self.loaded.len() - 1].iter().filter(|c| {
            let drop = live > LOADED_MESSAGES / 2 && !shown.contains(c.as_str());
            live -= if drop { self.chat_msg_indices[*c].len() } else { 0 };
            drop
        });
        let old: HashSet<String> = old.cloned().collect();
        self.loaded.retain(|c| !old.contains(c));
        self.chat_msg_indices.retain(|c, _| !old.contains(c));
        old.iter().for_each(|c| self.touch_timeline(c));
        self.compact();
    }

    /// Unloads the chat, like `unload_old` when it was used least recently.
    #[cfg(test)]
    pub(crate) fn unload_chat(&mut self, id: &str) {
        self.loaded.retain(|c| c != id);
        assert!(self.chat_msg_indices.remove(id).is_some(), "{id} is loaded");
        self.touch_timeline(id);
        self.compact();
    }

    /// Moves the timeline revision of the chat after a change of its messages.
    fn touch_timeline(&mut self, chat: &str) {
        self.timeline_writes += 1;
        match self.timeline_revs.get_mut(chat) {
            Some(rev) => *rev = self.timeline_writes,
            None => drop(self.timeline_revs.insert(chat.into(), self.timeline_writes)),
        }
    }

    /// The timeline revision of the chat, which moves whenever a message of the chat is added, changed or removed,
    /// or the chat unloads.
    pub fn timeline_rev(&self, chat: &str) -> u64 {
        self.timeline_revs.get(chat).copied().unwrap_or(0)
    }

    /// Counts the changes that moved the indices of `messages`.
    pub fn message_moves(&self) -> u64 {
        self.moves
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

    /// The loaded messages that the chat shows, in timeline order: its own, or Desktop's `syncMergedMessages` of a
    /// merged chat, the messages of its member chats in `G2` order from the newest of the oldest loaded times of the
    /// members with older history (`clampVisibleMessages`), so that no member leaves a hole.
    pub fn chat_timeline(&self, chat: &str) -> Vec<&Message> {
        let members = self.chat_idx_by_id.get(chat).map(|&i| self.members(i)).unwrap_or_default();
        if members.is_empty() {
            return self.timeline(chat).collect();
        }
        let ids = members.iter().map(|&i| self.chats[i].id.as_str());
        let mut all: Vec<&Message> = ids.flat_map(|c| self.timeline(c)).collect();
        all.sort_by_cached_key(|m| merged_order(m));
        let older = members.iter().filter(|&&i| self.matrix.is_some() && self.chats[i].older != Older::Start);
        let oldest = |i: &usize| self.timeline(&self.chats[*i].id).next().map(|m| merged_order(m).0);
        match older.filter_map(oldest).max() {
            Some(boundary) => all.into_iter().filter(|m| merged_order(m).0 >= boundary).collect(),
            None => all,
        }
    }

    /// Desktop's `boundaryHolders` of a merged chat: the member chats with older history whose oldest loaded message
    /// is the newest, which load older messages first. A chat that is not merged holds its own boundary.
    pub fn boundary_holders(&self, chat: &str) -> Vec<String> {
        let idx = self.chat_idx(chat);
        let members = self.members(idx);
        if members.is_empty() {
            return vec![chat.into()];
        }
        let older = members.into_iter().filter(|&i| self.chats[i].older != Older::Start);
        let oldest = |i: usize| self.timeline(&self.chats[i].id).next().map_or(i64::MAX, |m| merged_order(m).0);
        let timed: Vec<(i64, usize)> = older.map(|i| (oldest(i), i)).collect();
        let boundary = timed.iter().map(|t| t.0).max();
        timed.into_iter().filter(|t| Some(t.0) == boundary).map(|(_, i)| self.chats[i].id.clone()).collect()
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
        // The messages that stay are the ones that the timelines of the loaded chats list, which `msg_idx_by_id`
        // finds too, so the messages move up in place without a lookup of each. In [`Self::with_chat`] the messages
        // of the unloaded chat that `msg_idx_by_id` finds stay too, though no timeline lists them.
        let before = self.messages.len();
        let mut keep = vec![false; before];
        self.chat_msg_indices.values().flatten().for_each(|&i| keep[i] = true);
        if let Some((chat, ..)) = &self.unloaded {
            let unloaded = self.msg_idx_by_id.values().filter(|&&i| self.messages[i].chat_id == *chat);
            unloaded.for_each(|&i| keep[i] = true);
        }
        let (mut moved, mut kept) = (vec![usize::MAX; keep.len()], 0);
        for (i, _) in keep.iter().enumerate().filter(|k| *k.1) {
            (moved[i], kept) = (kept, kept + 1);
        }
        let mut keep = keep.into_iter();
        self.messages.retain(|_| keep.next().expect("a flag for each message"));
        self.msg_idx_by_id.retain(|_, i| {
            *i = moved[*i];
            *i != usize::MAX
        });
        assert_eq!(self.msg_idx_by_id.len(), kept, "each loaded message is in the timeline of its chat");
        for i in self.chat_msg_indices.values_mut().flatten() {
            *i = moved[*i];
        }
        self.thread_msg_indices.retain(|_, list| {
            list.iter_mut().for_each(|i| *i = moved[*i]);
            !list.contains(&usize::MAX)
        });
        if self.messages.len() != before {
            self.moves += 1;
        }
    }

    fn load_from_sqlite(&mut self) {
        self.setup = self.kv(KEY_SETUP).map(|s| match s.as_str() {
            "fresh" => Setup::Fresh,
            "imported" => Setup::Imported,
            other => panic!("unknown setup value {other} in {}", self.config.db_path.display()),
        });
        if let Some(state) = self.kv_json(KEY_STATE) {
            self.beeper_state = state;
        }
        self.connect =
            crate::connect::Connect::load(self.kv(crate::connect::KEY_TOKENS), self.kv(crate::connect::KEY_CLIENTS));
        if let Some(scheduled) = self.kv_json(KEY_SCHEDULED) {
            self.scheduled = scheduled;
        }
        if let Some(bridges) = self.kv_json(KEY_BRIDGES) {
            self.bridge_accounts = bridges;
        }
        if let Some(pin_rank) = self.kv_json(KEY_PIN_RANK) {
            self.pin_rank = pin_rank;
        }
        if let Some(recents) = self.kv_json(KEY_RECENT_CHATS) {
            self.recent_chat_ids = recents;
        }
        if let Some(contacts) = self.kv_json(KEY_CONTACTS) {
            self.contacts = contacts;
        }
        if let Some(settings) = self.kv_json::<BeeperSettings>(KEY_SETTINGS) {
            self.settings = settings;
            self.settings.prefs.retain(|k, _| !crate::prefs::RETIRED.contains(&k.as_str()));
            self.settings.assert_valid();
        }
        self.pin_app_locale();
        let assets: Vec<String> = {
            let conn = self.db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT json FROM assets").expect("prepare row query");
            stmt.query_map([], |r| r.get(0)).expect("query rows").map(|r| r.expect("read row")).collect()
        };
        for a in assets {
            let asset: StoredAsset = serde_json::from_str(&a).expect("parse stored asset");
            self.assets_by_mxc.insert(asset.mxc_url.clone(), asset.upload_id.clone());
            self.assets.insert(asset.upload_id.clone(), asset);
        }
    }

    /// The stored value of `key` in the `kv` table.
    fn kv(&self, key: &str) -> Option<String> {
        let db = self.db.lock().unwrap();
        db.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional().expect("read kv")
    }

    /// The stored JSON value of `key` in the `kv` table.
    fn kv_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let v = self.kv(key)?;
        Some(serde_json::from_str(&v).unwrap_or_else(|e| panic!("parse the stored {key}: {e}")))
    }

    /// Loads the stored chats, or seeds the chats of a new demo store.
    pub fn load_chats(&mut self) {
        let seq = self.kv(KEY_SEQ);
        let (chats, messages) = {
            let conn = self.db.lock().unwrap();
            let mut chats = conn.prepare("SELECT json FROM chats ORDER BY rowid").expect("prepare the chat query");
            (
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
        self.apply_union_limit();
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

    /// Stores `value` as the JSON of `key` in the `kv` table.
    fn put_json(&self, key: &str, value: &impl Serialize) {
        self.put_kv(key, &serde_json::to_string(value).expect("a stored value as JSON"));
    }

    /// Desktop's `pushThreadHistory`: chat `id` goes first in the saved history of the opened chats, which keeps 50.
    pub fn push_recent_chat(&mut self, id: &str) {
        if self.recent_chat_ids.first().is_some_and(|c| c == id) {
            return;
        }
        self.recent_chat_ids.retain(|c| c != id);
        self.recent_chat_ids.insert(0, id.to_string());
        self.recent_chat_ids.truncate(50);
        self.put_json(KEY_RECENT_CHATS, &self.recent_chat_ids);
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
        self.put_json(KEY_SCHEDULED, &self.scheduled);
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

    /// What the last run kept of each room for the start pass (`matrix::start_pass`): the JSON of the reads and of
    /// the members of the room, when `mark` is the mark of the last sync that the chats show. Else none of it holds,
    /// and it is dropped.
    pub(crate) fn kept_rooms(&mut self, mark: Option<&str>) -> Option<KeptRooms> {
        if mark.is_none() || self.kv(KEY_SYNCED).as_deref() != mark {
            self.exec("DELETE FROM room_reads", &[]);
            return None;
        }
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT room_id, reads, members FROM room_reads").expect("prepare the kept rooms");
        let rows = q.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?)))).expect("read the kept rooms");
        Some(rows.collect::<Result<_, _>>().expect("read a kept room"))
    }

    /// Whether a stored message of `chat` waits for its keys.
    pub(crate) fn has_waiting(&self, chat: &str) -> bool {
        let db = self.db.lock().unwrap();
        let mut q = db
            .prepare_cached(&format!("SELECT 1 FROM messages WHERE chat_id = ?1 AND {} LIMIT 1", waiting()))
            .expect("prepare the waiting messages");
        q.exists([chat]).expect("read the waiting messages")
    }

    /// Keeps the fresh `reads` or `members` of `room` for the next start, see [`Self::kept_rooms`].
    pub(crate) fn keep_room(&self, room: &str, reads: Option<&str>, members: Option<&str>) {
        self.exec(
            "INSERT INTO room_reads (room_id, reads, members) VALUES (?1, ?2, ?3) ON CONFLICT (room_id) DO UPDATE
             SET reads = coalesce(excluded.reads, reads), members = coalesce(excluded.members, members)",
            params![room, reads, members],
        );
    }

    /// A sync changes the account data of the rooms `reads` and the state of the rooms `members`, so what was kept of
    /// their reads, and also of their members, no longer holds.
    pub(crate) fn drop_kept(&mut self, reads: &[String], members: &[String]) {
        if reads.is_empty() && members.is_empty() {
            return;
        }
        self.batch(|st| {
            for r in reads {
                st.exec("UPDATE room_reads SET reads = NULL WHERE room_id = ?1", params![r]);
            }
            for r in members {
                st.exec("DELETE FROM room_reads WHERE room_id = ?1", params![r]);
            }
        });
    }

    /// The chats show the sync of `mark`, see [`Self::kept_rooms`].
    pub(crate) fn mark_synced(&self, mark: &str) {
        self.put_kv(KEY_SYNCED, mark);
    }

    /// Runs `f` with its writes to SQLite in one transaction, so that they make one commit, or in the transaction
    /// that is open already.
    pub fn batch<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let outer = self.db.lock().unwrap().is_autocommit();
        if outer {
            self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin a batch");
        }
        let r = f(self);
        if outer {
            self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit a batch");
        }
        r
    }

    /// Opens one SQLite transaction for the writes of the rooms of a sync response, which their `batch` joins, so that
    /// they make one commit and not one each. The sync does not hold the store lock while it reads the Matrix state of
    /// the next room, and the writes of the windows meanwhile go into the transaction too: they share the connection,
    /// so they read what they wrote, and they commit with the sync. So the sync commits before it waits for the
    /// homeserver, see [`Self::commit_sync`].
    pub fn begin_sync(&mut self) {
        let db = self.db.lock().unwrap();
        if !self.sync_tx && db.is_autocommit() {
            db.execute_batch("BEGIN IMMEDIATE;").expect("begin the writes of a sync");
            self.sync_tx = true;
        }
    }

    /// Commits the transaction of [`Self::begin_sync`], and returns whether it was open.
    pub fn commit_sync(&mut self) -> bool {
        let open = std::mem::take(&mut self.sync_tx);
        if open {
            self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit the writes of a sync");
        }
        open
    }

    /// Adds new chats and their messages in one SQLite transaction. A message without a sort key takes the next one.
    pub fn add_all(&mut self, chats: Vec<Chat>, messages: Vec<Message>) {
        self.batch(|st| {
            let (first_chat, first_message) = (st.chats.len(), st.messages.len());
            for c in chats {
                st.push_chat(c, true);
            }
            for mut m in messages {
                if m.sort_key.is_empty() {
                    st.next_seq += 1;
                    m.sort_key = sort_key(st.next_seq);
                }
                st.push_message(m, false);
            }
            (first_chat..st.chats.len()).for_each(|i| st.persist_chat(i));
            (first_message..st.messages.len()).for_each(|i| st.persist_message(i));
            st.persist_seq();
        });
        self.unload_old();
    }

    pub fn import_beeper(&mut self, data: crate::import::BeeperData) {
        assert!(self.setup.is_none() && !self.signed_in(), "import into a store that is already set up");
        assert!(self.chats.is_empty() && self.messages.is_empty(), "import into a store that has chats");
        self.finish_setup(Setup::Imported);
        self.add_all(data.chats, data.messages);
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

    /// Desktop's `customName` of an account: the nickname of its bridge login in [`Self::account_overrides`].
    pub fn custom_name(&self, account_id: &str) -> Option<&str> {
        let b = self.bridge_accounts.iter().find(|b| b.account_id == account_id)?;
        let name = self.account_overrides["accounts"][&b.bridge_id][&b.login_id]["name"].as_str();
        name.filter(|n| !n.is_empty())
    }

    /// The bridge login of a chat account, for the bridge provisioning API. An account without a bridge login is
    /// `<bridge>.<login>`.
    pub fn provision(&self, account_id: &str) -> crate::matrix::Provision {
        let found = self.bridge_accounts.iter().find(|a| a.account_id == account_id);
        let (bridge_id, login_id) = found.map_or_else(
            || {
                let (b, l) = account_id.split_once('.').unwrap_or((account_id, ""));
                (b.to_string(), l.to_string())
            },
            |a| (a.bridge_id.clone(), a.login_id.clone()),
        );
        let s = &self.beeper_state;
        crate::matrix::Provision {
            homeserver: s.homeserver.clone(),
            access_token: s.access_token.clone(),
            user_id: s.user_id.clone(),
            bridge_id,
            login_id,
        }
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
            is_self: Some(true),
            cannot_message: Some(false),
            ..Default::default()
        };
        let chat = Chat {
            id: NOTE_CHAT_ID.into(),
            account_id: "hungryserv".into(),
            participants: ChatParticipants { has_more: false, items: vec![me], total: 1 },
            title: "Note to self".into(),
            chat_type: "single".into(),
            is_archived: Some(false),
            is_marked_unread: Some(false),
            is_muted: Some(false),
            is_pinned: Some(true),
            is_low_priority: Some(false),
            last_activity: Some(now_rfc3339_millis()),
            local_chat_id: Some("local_note_self".into()),
            edits: 0,
            member_actions: 0,
            ..Default::default()
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
        // Desktop's `archiveRooms` of a merged chat archives its member chats, and it is archived when they all are.
        let members = self.members(idx);
        if flag == ChatFlag::Archived && !members.is_empty() {
            for m in members {
                self.set_chat_flag(m, flag, Some(value));
            }
            return true;
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
        // The bit first: the feature flag is a slower read.
        c.block & USER_BLOCKED != 0 && self.can_block(c)
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
        self.chat_changes += 1;
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

    /// Puts a message of a loaded chat into the message lists. A message of the chat that `with_chat` writes into
    /// without loading it goes only into `messages`.
    fn insert_message(&mut self, mut msg: Message) -> usize {
        msg.sync_flags();
        let digits = msg.sort_key.strip_prefix('-').unwrap_or(&msg.sort_key).len();
        assert_eq!(digits, 16, "sort key {} of {} is not from sort_key", msg.sort_key, msg.id);
        self.next_seq = self.next_seq.max(msg.sort_key.parse().expect("sort keys are numbers"));
        let idx = self.messages.len();
        assert!(self.msg_idx_by_id.insert(msg.id.clone(), idx).is_none(), "duplicate message {}", msg.id);
        if let Some(chat) = self.chat_msg_indices.get_mut(&msg.chat_id) {
            let thread = msg.thread_root.as_ref().map(|r| self.thread_msg_indices.entry(r.clone()).or_default());
            for list in std::iter::once(chat).chain(thread) {
                list.insert(list.partition_point(|&i| self.messages[i].sort_key <= msg.sort_key), idx);
            }
        } else {
            assert!(self.is_unloaded(&msg.chat_id), "the chat of the message {} is loaded", msg.id);
        }
        self.touch_timeline(&msg.chat_id);
        self.messages.push(msg);
        idx
    }

    /// Adds a message of a loaded chat, which can preview the chat.
    fn push_message(&mut self, msg: Message, persist: bool) -> usize {
        let idx = self.insert_message(msg);
        let msg = &self.messages[idx];
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        // Like the timeline, a thread reply previews a chat that does not show threads.
        let shown = !msg.silent() && !msg.thread_only(self.threaded(&self.chats[c_idx])) && msg.live_or_expired();
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
            self.sync_parent(c_idx);
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
        self.apply_group(
            id,
            title.as_deref(),
            topic.as_deref(),
            avatar.as_ref().map(|a| a.as_ref().map(|a| a.0.as_str())),
        );
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
        if !c.label_ids.is_empty() && self.feature("labels-v2").on {
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
            let in_union = |c: &&Chat| {
                c.flags & CHAT_INACTIVE_UNION == 0 && c.merge.as_ref().is_some_and(|m| m.chat_ids.contains(&chat.id))
            };
            chat.merged_into_chat_id = self.chats.iter().find(in_union).map(|c| c.id.clone());
            let idx = self.push_chat(chat, true);
            let c = self.chats[idx].clone();
            self.emit_ws("chat.upserted", &c.id, None, Some(c.clone()), None);
            return self.sync_parent(idx);
        };
        let mut new = self.chats[idx].clone();
        let merge = |c: &mut Chat| {
            *c = Chat {
                draft: c.draft.take(),
                synced_draft: c.synced_draft.take(),
                label_ids: std::mem::take(&mut c.label_ids),
                last_activity: c.last_activity.take().or(chat.last_activity.clone()),
                local_chat_id: c.local_chat_id.take(),
                merge: c.merge.take(),
                merged_into_chat_id: c.merged_into_chat_id.take(),
                preview: c.preview.take(),
                reminder: ChatReminder::synced(chat.reminder.clone(), c.reminder.as_ref()),
                note: c.note.take(),
                notifiable: c.notifiable.take(),
                older: std::mem::take(&mut c.older),
                thread_reads: std::mem::take(&mut c.thread_reads),
                // A read here goes ahead of the receipt that the homeserver shows later.
                last_read_message_sort_key: c
                    .last_read_message_sort_key
                    .take()
                    .max(chat.last_read_message_sort_key.clone()),
                ..chat
            }
        };
        merge(&mut new);
        // Each sync upserts its rooms, mostly unchanged: those write nothing, and the views keep what they found. The
        // `manage` bits of the members are not in the JSON, and a chat loaded from SQLite has none until a sync sets
        // them.
        let json = |c: &Chat| serde_json::to_string(c).expect("chat JSON");
        let manage = |c: &Chat| c.participants.items.iter().map(|u| u.manage).collect::<Vec<_>>();
        if json(&new) != json(&self.chats[idx]) || manage(&new) != manage(&self.chats[idx]) {
            self.update_chat(idx, |c| *c = new);
        }
    }

    /// The new title, description and avatar URL of a group here, which the homeserver took.
    pub fn apply_group(&mut self, id: &str, title: Option<&str>, topic: Option<&str>, avatar: Option<Option<&str>>) {
        let idx = self.chat_idx_by_id[id];
        self.update_chat(idx, |c| {
            if let Some(t) = topic {
                c.description = Some(t.to_string()).filter(|t| !t.is_empty());
            }
            c.edit_room(|c| {
                if let Some(t) = title {
                    c.title = t.to_string();
                }
                if let Some(a) = avatar {
                    c.img_url = a.map(str::to_string);
                }
            });
        });
    }

    /// Drops a chat that the user left, with its messages.
    pub fn remove_chat(&mut self, id: &str) {
        self.remove_label(id);
        let Some(&idx) = self.chat_idx_by_id.get(id) else { return };
        let union = self.chats[idx].merge.take();
        if let Some(m) = &union {
            self.set_merged_into(&m.chat_ids, id, false);
        }
        let idx = self.chat_idx_by_id.remove(id).expect("the chat is here");
        let chat = self.chats.remove(idx);
        self.chat_changes += 1;
        assert!(self.chat_order.remove(&activity_key(&chat)), "chat {id} is in the chat order");
        if let Some(local) = chat.local_chat_id {
            self.local_chat_idx_by_id.remove(&local);
        }
        for i in self.chat_idx_by_id.values_mut().chain(self.local_chat_idx_by_id.values_mut()).filter(|i| **i > idx) {
            *i -= 1;
        }
        self.loaded.retain(|c| c != id);
        if self.chat_msg_indices.remove(id).is_some() {
            self.touch_timeline(id);
            self.compact();
        }
        self.exec("DELETE FROM messages WHERE chat_id = ?1", params![id]);
        self.exec("DELETE FROM chats WHERE id = ?1", params![id]);
        self.recent_chat_ids.retain(|c| c != id);
        self.typing.remove(id);
        self.outbox.retain(|(_, c, _)| c != id);
        self.exec("DELETE FROM outbox WHERE chat_id = ?1", params![id]);
        self.emit_ws("chat.deleted", id, None, None, None);
        if let Some(p) = chat.merged_into_chat_id.and_then(|p| self.resolve_chat_idx(&p)) {
            self.update_chat(p, |_| {});
        }
        // A merged chat past the limit can take the place of this one.
        if union.is_some() {
            self.apply_union_limit();
        }
    }

    /// Asks the bridge to delete the chat, and removes it here at once like Desktop.
    pub fn delete_chat(&mut self, id: &str, for_everyone: bool) {
        let modes = self.chats[self.chat_idx_by_id[id]].delete_modes;
        let mode = if for_everyone { DELETE_FOR_EVERYONE } else { DELETE_FOR_ME };
        assert!(modes & mode != 0, "{id} does not support delete mode {mode}");
        let parents = self.union_parents(id);
        self.remove_chat(id);
        self.queue(id, Outgoing::DeleteChat { for_everyone });
        parents.iter().for_each(|p| self.drop_from_union(p, id));
    }

    /// Desktop's `room delete` of a room that went elsewhere: the chat goes, and so does its place in the merged chats
    /// that list it.
    pub fn remove_deleted_chat(&mut self, id: &str) {
        let parents = self.union_parents(id);
        self.remove_chat(id);
        parents.iter().for_each(|p| self.drop_from_union(p, id));
    }

    /// Desktop's `getUnionParentsForChild`: the merged chats that list the room, past the limit too.
    fn union_parents(&self, room: &str) -> Vec<String> {
        let lists = |c: &&Chat| c.merge.as_ref().is_some_and(|m| m.chat_ids.iter().any(|r| r == room));
        self.chats.iter().filter(lists).map(|c| c.id.clone()).collect()
    }

    /// Desktop's clean-up of the unions of a deleted room: an `m.space.child` without `via` takes it out of the merged
    /// chat `parent`, and a merged chat that has no other member room is left (`leaveUnionSpace`).
    fn drop_from_union(&mut self, parent: &str, room: &str) {
        let Some(idx) = self.resolve_chat_idx(parent) else { return };
        let Some(mut merge) = self.chats[idx].merge.clone() else { return };
        merge.chat_ids.retain(|r| r != room);
        match merge.chat_ids.is_empty() {
            true => self.unmerge(parent),
            false => {
                self.set_merge(idx, Some(merge));
                self.queue(parent, Outgoing::LabelChild(room.into(), false));
            }
        }
    }

    pub fn upsert_message(&mut self, mut msg: Message) {
        if self.bookmarks_room.is_some() {
            msg.is_bookmarked = Some(self.bookmarks.contains_key(&msg.id));
        }
        // A chat that `with_chat` writes into without loading it reads only the stored message.
        let found = match self.is_unloaded(&msg.chat_id) {
            true => self.loaded_idx(&msg.id).or_else(|| self.chat_message(&msg.chat_id, &msg.id)),
            false => {
                self.load_chat(&msg.chat_id);
                self.loaded_idx(&msg.id)
            }
        };
        // Desktop's upsert keeps `isDeleted` when the message has a removal.
        let stored = found.is_some_and(|i| self.messages[i].removed);
        if msg.is_deleted != Some(true) && (stored || self.removals.as_ref().is_some_and(|r| r.contains_key(&msg.id))) {
            removed_from_beeper(&mut msg);
        }
        let Some(idx) = found else {
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
            let disappearing = crate::models::MessageTimer::merge(m.disappearing.take(), msg.disappearing.take());
            // Desktop keeps the views left apart from the message, and an expired message keeps the kind of its media.
            let views_left = msg.view_limit.as_ref().and(m.views_left);
            let view_kind = msg.view_kind.clone().or_else(|| m.view_once_kind().map(str::to_string));
            *m = Message {
                reactions: std::mem::take(&mut m.reactions),
                edit_history: std::mem::take(&mut m.edit_history),
                is_bookmarked: m.is_bookmarked,
                is_unread: m.is_unread,
                seen,
                disappearing,
                views_left,
                view_kind: view_kind.filter(|_| msg.view_expired),
                ..msg
            }
        });
    }

    pub fn update_chat(&mut self, idx: usize, f: impl FnOnce(&mut Chat)) -> Chat {
        let old_key = activity_key(&self.chats[idx]);
        self.chat_changes += 1;
        f(&mut self.chats[idx]);
        if let Some(row) = self.union_row(idx) {
            let c = &mut self.chats[idx];
            (c.preview, c.unread_count, c.unread_mentions) = (row.preview, row.unread, row.mentions);
            c.last_activity = row.activity.or(c.last_activity.take());
            c.is_archived = Some(row.archived);
        }
        assert!(self.chat_order.remove(&old_key), "chat {} is in the chat order", old_key.1);
        self.chat_order.insert(activity_key(&self.chats[idx]));
        self.chats[idx].sync_flags();
        self.chats[idx].participants.items.shrink_to_fit();
        self.chats[idx].flags = self.label_flags(&self.chats[idx]);
        self.persist_chat(idx);
        let chat = self.chats[idx].clone();
        self.emit_ws("chat.upserted", &chat.id, None, Some(chat.clone()), None);
        self.sync_parent(idx);
        chat
    }

    /// Desktop's `UnionThreadStore` row of the merged chat `idx`: the newest preview and activity of its member chats
    /// (`lastNonSilentMessage`, `sortTimestamp`), the sums of their unread counts, and archived when all of them are
    /// (`isRoomDone`). `None` for another chat, or a merged chat without member chats here.
    fn union_row(&self, idx: usize) -> Option<UnionRow> {
        let members = self.members(idx);
        let ms = |m: &Message| millis_from_rfc3339(&m.timestamp);
        let newest = members.iter().filter_map(|&i| self.chats[i].preview.as_ref()).max_by_key(|m| ms(m));
        let activity = members
            .iter()
            .filter_map(|&i| self.chats[i].last_activity.as_ref())
            .max_by_key(|t| millis_from_rfc3339(t).expect("RFC 3339 activity"));
        (!members.is_empty()).then(|| UnionRow {
            preview: newest.cloned(),
            activity: activity.cloned(),
            unread: members.iter().map(|&i| self.chats[i].unread_count).sum(),
            mentions: members.iter().map(|&i| self.chats[i].unread_mentions).sum(),
            archived: members.iter().all(|&i| self.chats[i].flag(ChatFlag::Archived)),
        })
    }

    /// Updates the merged chat of the chat `idx` after a change of the chat.
    fn sync_parent(&mut self, idx: usize) {
        let parent = self.chats[idx].merged_into_chat_id.as_deref().and_then(|p| self.resolve_chat_idx(p));
        if let Some(p) = parent {
            self.update_chat(p, |_| {});
        }
    }

    pub fn update_message(&mut self, idx: usize, f: impl FnOnce(&mut Message)) -> Message {
        let chat = self.messages[idx].chat_id.clone();
        f(&mut self.messages[idx]);
        self.messages[idx].sync_flags();
        self.touch_timeline(&chat);
        if self.messages[idx].chat_id != chat {
            let now = self.messages[idx].chat_id.clone();
            self.touch_timeline(&now);
        }
        self.persist_message(idx);
        let msg = self.messages[idx].clone();
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        if self.chats[c_idx].preview.as_ref().is_some_and(|p| p.id == msg.id) {
            match msg.live_or_expired() {
                true => self.chats[c_idx].preview = Some(msg.clone()),
                false => self.chats[c_idx].preview = self.last_shown(&msg.chat_id),
            }
            self.persist_chat(c_idx);
            self.sync_parent(c_idx);
        }
        self.emit_ws("message.upserted", &msg.chat_id, Some(&msg.id), None, Some(msg.clone()));
        msg
    }

    /// Desktop's chat preview after its message is deleted: the newest shown message that is left.
    fn last_shown(&self, chat: &str) -> Option<Message> {
        let threads = self.chat_idx_by_id.get(chat).is_some_and(|&i| self.threaded(&self.chats[i]));
        let mut last = None;
        self.scan_stored("chat_id = ?1 ORDER BY sort_key DESC", [chat], |m| {
            let shown = !m.silent() && !m.thread_only(threads) && m.live_or_expired();
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

    /// The indices of the member chats of the merged chat `idx` that are here, in its order (Desktop's
    /// `memberThreads`); none for another chat.
    pub fn members(&self, idx: usize) -> Vec<usize> {
        let ids = self.chats[idx].merge.iter().flat_map(|m| &m.chat_ids);
        ids.filter_map(|id| self.resolve_chat_idx(id)).collect()
    }

    /// Desktop's `defaultMemberChat`: the default member chat of a merged chat, else its first one.
    pub fn default_member(&self, idx: usize) -> Option<usize> {
        let members = self.members(idx);
        let default = self.chats[idx].merge.as_ref().and_then(|m| m.default_chat_id.as_deref());
        default.and_then(|d| members.iter().copied().find(|&i| self.chats[i].id == d)).or(members.first().copied())
    }

    /// The chat that a send to the chat `idx` through the Desktop API goes to: the default member chat of a merged
    /// chat, else its member chat with the newest activity, or the chat itself.
    fn send_target(&self, idx: usize) -> usize {
        let members = self.members(idx);
        let default = self.chats[idx].merge.as_ref().and_then(|m| m.default_chat_id.as_deref());
        let default = default.and_then(|d| members.iter().copied().find(|&i| self.chats[i].id == d));
        let newest =
            || members.iter().copied().max_by(|&a, &b| self.chats[a].last_activity.cmp(&self.chats[b].last_activity));
        default.or_else(newest).unwrap_or(idx)
    }

    /// Desktop's `seedComposeTarget` of a merged chat, where its composer sends: the member chat with the newest
    /// message, unless it is your own and older than 30 minutes, which goes to the default member chat. Another chat
    /// sends to itself.
    pub fn compose_target(&self, idx: usize) -> usize {
        let members = self.members(idx);
        let ms = |i: &usize| self.chats[*i].preview.as_ref().and_then(|m| millis_from_rfc3339(&m.timestamp));
        let latest = members.iter().filter(|i| ms(i).is_some()).max_by_key(|i| ms(i));
        let stale = |&i: &usize| {
            let m = self.chats[i].preview.as_ref().expect("a member with a message");
            m.is_sender == Some(true) && ms(&i).expect("a message time") + 30 * 60_000 <= now_unix_millis()
        };
        match latest {
            Some(&i) if !stale(&i) => i,
            _ => self.default_member(idx).unwrap_or(idx),
        }
    }

    /// Desktop's `m.typing` of `chat`: `ids` type, each of them for [`TYPING_DURATION`] at most unless another update
    /// comes (`USER_ACTIVITY` with `durationMs: 6e4`). Returns the number of the update, which
    /// [`Self::typing_timeout`] takes.
    pub fn typing_update(&mut self, chat: &str, ids: Vec<String>) -> u64 {
        let n = self.typing_updates.entry(chat.to_string()).or_default();
        *n += 1;
        let n = *n;
        self.set_typing(chat, ids);
        n
    }

    /// Desktop's `deleteUserActivity` when the activity of the typing update `n` of `chat` times out: whoever is left
    /// of its users stops, unless a later update came.
    pub fn typing_timeout(&mut self, chat: &str, n: u64) {
        if self.typing_updates.get(chat) == Some(&n) {
            self.set_typing(chat, Vec::new());
        }
    }

    /// The users who type in `chat`, other than you, from its `m.typing`.
    pub fn set_typing(&mut self, chat: &str, ids: Vec<String>) {
        let old = match ids.is_empty() {
            true => self.typing.remove(chat),
            false => self.typing.insert(chat.to_string(), ids.clone()),
        };
        if old.unwrap_or_default() != ids {
            self.emit_ws("chat.typing", chat, None, None, None);
        }
    }

    /// Desktop's `onUserActivityStopped` of `onNewMessages`: a new message of a user who types ends their typing, also
    /// when their client did not end the notice.
    pub fn stop_typing(&mut self, chat: &str, senders: &[String]) {
        let ids = self.typing.get(chat).into_iter().flatten().filter(|u| !senders.contains(u)).cloned().collect();
        self.set_typing(chat, ids);
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

    /// Merges the member chats here into `parent` (`on`), or takes those of `parent` out of it. A member room that is
    /// not here joins when it comes (`upsert_chat`).
    fn set_merged_into(&mut self, ids: &[String], parent: &str, on: bool) {
        let here: Vec<usize> = ids.iter().filter_map(|id| self.resolve_chat_idx(id)).collect();
        for idx in here {
            let now = self.chats[idx].merged_into_chat_id.as_deref();
            if (now == Some(parent)) != on {
                self.update_chat(idx, |c| c.merged_into_chat_id = on.then(|| parent.to_string()));
            }
        }
    }

    /// Sets the member chats of the chat `idx`, or makes it no merged chat with `None`.
    fn set_merge(&mut self, idx: usize, merge: Option<ChatMergeInfo>) {
        let id = self.chats[idx].id.clone();
        let new = merge.as_ref().map_or(&[][..], |m| &m.chat_ids[..]);
        let old = self.chats[idx].merge.iter().flat_map(|m| &m.chat_ids);
        let gone: Vec<String> = old.filter(|c| !new.contains(c)).cloned().collect();
        let new = new.to_vec();
        self.chats[idx].merge = merge;
        self.set_merged_into(&gone, &id, false);
        self.set_merged_into(&new, &id, true);
        self.update_chat(idx, |_| {});
        self.apply_union_limit();
    }

    /// Desktop's `activeUnionIds`: without a `max-merge-chats` limit, or within it, every merged chat is active, else
    /// the oldest ones by creation. A merged chat past it is hidden, and its member chats show on their own.
    pub fn apply_union_limit(&mut self) {
        // Until the feature flags come, no merged chat hides.
        let max = if self.features.flags.is_null() { -1 } else { self.features.max_merge_chats() };
        let mut unions: Vec<usize> = (0..self.chats.len()).filter(|&i| self.chats[i].merge.is_some()).collect();
        unions.sort_by_key(|&i| self.chats[i].created_at.unwrap_or(i64::MAX));
        let active = if max < 0 { unions.len() } else { usize::try_from(max).expect("a limit from 0") };
        for (n, i) in unions.into_iter().enumerate() {
            let on = n < active;
            let id = self.chats[i].id.clone();
            let rooms = self.chats[i].merge.as_ref().map(|m| m.chat_ids.clone()).unwrap_or_default();
            if (self.chats[i].flags & CHAT_INACTIVE_UNION == 0) != on {
                self.update_chat(i, |c| c.flags ^= CHAT_INACTIVE_UNION);
            }
            self.set_merged_into(&rooms, &id, on);
        }
    }

    /// Desktop's `handleUnionSync`: the member rooms of the merged chat `id`, from the `m.space.child` state of its
    /// `com.beeper.union` room.
    pub fn set_union(&mut self, id: &str, rooms: Vec<String>) {
        let idx = self.chat_idx(id);
        let default_chat_id = self.chats[idx].merge.as_ref().and_then(|m| m.default_chat_id.clone());
        if self.chats[idx].merge.as_ref().is_none_or(|m| m.chat_ids != rooms) {
            self.set_merge(idx, Some(ChatMergeInfo { chat_ids: rooms, default_chat_id }));
        }
    }

    /// Desktop's `canCreateUnion`: the plan has no `max-merge-chats` limit, or fewer merged chats than it.
    pub fn can_create_union(&self) -> bool {
        let max = self.features.max_merge_chats();
        max < 0 || (self.chats.iter().filter(|c| c.merge.is_some()).count() as i64) < max
    }

    /// Desktop's `createUnion`: a merged chat of `rooms` here at once, and a `com.beeper.union` room with their
    /// `m.space.child` state, the title and the photo `avatar` (an `mxc://` URL) on the homeserver. Returns the ID of
    /// the merged chat, which stays its local ID once the room exists ([`Self::created_union`]).
    pub fn create_union(
        &mut self,
        title: &str,
        rooms: Vec<String>,
        default: Option<String>,
        avatar: Option<String>,
    ) -> String {
        let title = Some(title.to_string()).filter(|t| !t.is_empty());
        let (chat, _) = self.create_or_start_chat(
            "hungryserv",
            "single",
            title.clone(),
            Vec::new(),
            None,
            Some(rooms.clone()),
            default,
        );
        if avatar.is_some() {
            let idx = self.chat_idx(&chat.id);
            self.update_chat(idx, |c| c.img_url.clone_from(&avatar));
        }
        self.queue(&chat.id, Outgoing::CreateUnion { title, rooms, avatar });
        chat.id
    }

    /// The homeserver made the room `room` of the merged chat `draft`: the chat takes the room ID, and keeps `draft`
    /// as its local ID.
    pub fn created_union(&mut self, draft: &str, room: &str) {
        self.sent.insert(draft.into(), room.into());
        let Some(&i) = self.chat_idx_by_id.get(draft) else { return };
        let mut chat = self.chats[i].clone();
        self.remove_chat(draft);
        if self.chat_idx_by_id.contains_key(room) {
            return;
        }
        let merge = chat.merge.take();
        (chat.id, chat.local_chat_id) = (room.into(), Some(draft.into()));
        self.upsert_chat(chat);
        let idx = self.chat_idx(room);
        self.set_merge(idx, merge);
    }

    /// Desktop's `updateUnion` from the merged chat editor: the title and the member rooms of the merged chat `id`,
    /// with an `m.space.child` change for each room that joins or leaves it, and its default member chat.
    pub fn update_union(&mut self, id: &str, title: &str, rooms: Vec<String>, default: Option<String>) {
        let idx = self.resolve_chat_idx(id).expect("an edited merged chat");
        let id = self.chats[idx].id.clone();
        let old = self.chats[idx].merge.as_ref().map(|m| m.chat_ids.clone()).unwrap_or_default();
        let added = rooms.iter().filter(|r| !old.contains(r)).map(|r| (r.clone(), true));
        let changes: Vec<(String, bool)> =
            added.chain(old.iter().filter(|r| !rooms.contains(r)).map(|r| (r.clone(), false))).collect();
        self.set_merge(idx, Some(ChatMergeInfo { chat_ids: rooms, default_chat_id: default }));
        for (room, add) in changes {
            self.queue(&id, Outgoing::LabelChild(room, add));
        }
        if self.chats[idx].title != title {
            self.update_chat(idx, |c| c.title = title.to_string());
            self.queue(&id, Outgoing::State("m.room.name".into(), serde_json::json!({ "name": title })));
        }
    }

    /// Desktop's `removeUnion`: the merged chat goes here at once, and the user leaves and forgets its room.
    pub fn unmerge(&mut self, id: &str) {
        self.remove_chat(id);
        self.queue(id, Outgoing::Leave(true));
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
                is_self: Some(false),
                cannot_message: Some(false),
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
            is_archived: Some(false),
            is_marked_unread: Some(false),
            is_muted: Some(false),
            is_pinned: Some(false),
            is_low_priority: Some(false),
            last_activity: Some(now_rfc3339_millis()),
            merge: merge.clone(),
            participants: ChatParticipants { has_more: false, total: items.len(), items },
            edits: 0,
            member_actions: 0,
            ..Default::default()
        };
        let idx = self.push_chat(chat, true);
        if let Some(m) = &merge {
            self.set_merged_into(&m.chat_ids, &id, true);
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
            self.set_merge(idx, merge);
        }
        self.put_draft(idx, draft);
        Ok(self.chats[idx].clone())
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
        let rules = self.resolve_chat_idx(chat_or_local_id).map(|i| self.chats[self.send_target(i)].file.clone());
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
        let idx = self.resolve_chat_idx(chat_or_local_id).ok_or(Error::ChatNotFound)?;
        // Desktop's `targetComposerAtMessage`: a reply in a merged chat goes to the chat of the message.
        let replied = reply_to_message_id.as_deref().and_then(|r| self.loaded(r)).map(|m| m.chat_id.as_str());
        let c_idx = match replied.filter(|&c| c != self.chats[idx].id && self.in_chat(idx, c)) {
            Some(c) => self.chat_idx(c),
            None => self.send_target(idx),
        };
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
            linked_message_id: reply_to_message_id,
            attachments: attachment.into_iter().collect(),
            thread_root: thread.map(|t| t.root.clone()),
            broadcast: thread.is_some_and(|t| t.broadcast),
            fallback: thread.is_some_and(|t| !t.chosen),
            ..Default::default()
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
        // Desktop's `handleRedaction` stores a new message without the reply and the mentions, as a redacted event
        // from a sync has none.
        self.update_message(m_idx, |m| {
            removed_from_beeper(m);
            (m.removed, m.is_edited) = (false, Some(true));
            (m.linked_message_id, m.mentions) = (None, None);
            m.flags &= !MSG_MENTION;
        });
        if removed {
            self.clean_up_removals(message_id);
        }
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
    }

    /// Desktop's redaction with `com.beeper.view_limited`: the bridge deleted view-once media after its last view, and
    /// the message stays as an expired pill that keeps the kind of the media.
    pub fn expire_message(&mut self, chat_id: &str, message_id: &str) -> Result<String, Error> {
        let (_, m_idx) = self.message_in_chat(chat_id, message_id)?;
        self.update_message(m_idx, |m| {
            m.view_kind = m.view_once_kind().map(str::to_string);
            m.view_expired = true;
        });
        self.delete_message(chat_id, message_id)
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
        match self.chat_msg_indices.get_mut(&chat_id) {
            Some(list) => {
                list.retain(|&i| i != m_idx);
                if let Some(root) = &self.messages[m_idx].thread_root {
                    self.thread_msg_indices.get_mut(root).expect("a thread has replies").retain(|&i| i != m_idx);
                }
            }
            None => assert!(self.is_unloaded(&chat_id), "the chat of {message_id} is loaded"),
        }
        self.touch_timeline(&chat_id);
        self.exec("DELETE FROM messages WHERE id = ?1", params![message_id]);
        let c_idx = self.chat_idx_by_id[&chat_id];
        if self.chats[c_idx].preview.as_ref().is_some_and(|p| p.id == message_id) {
            self.chats[c_idx].preview = self.last_shown(&chat_id);
            self.persist_chat(c_idx);
            self.sync_parent(c_idx);
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
        if c.label_ids.is_empty() || !self.feature("labels-v2").on {
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

    /// Desktop's `bookmarkStore.isRestored`: the bookmarks room was read, or the demo keeps its bookmarks locally.
    pub fn bookmarks_restored(&self) -> bool {
        self.bookmarks_room.is_some() || self.config.demo
    }

    /// Desktop's `applyBookmark` for the bookmarks room events, in timeline order.
    pub fn apply_bookmarks(&mut self, room: &str, changes: Vec<(String, Option<Bookmark>)>) {
        (self.bookmarks_room, self.bookmarks_failed) = (Some(room.into()), false);
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

    /// Desktop's `applyMessageReminderEvent` and `applyMessageReminderRedaction` for the reminders room events, in
    /// timeline order: the message and its new reminder with the reminder event, if any. Each chat that changes is
    /// upserted.
    pub fn apply_message_reminders(&mut self, room: &str, changes: Vec<(String, Option<(String, MessageReminder)>)>) {
        self.reminders_room = Some(room.into());
        let mut touched = BTreeSet::new();
        for (target, r) in changes {
            let old = match r {
                Some((event, r)) => {
                    self.reminder_events.insert(target.clone(), event);
                    touched.insert(r.room_id.clone());
                    self.message_reminders.insert(target, r)
                }
                None => {
                    self.reminder_events.remove(&target);
                    self.message_reminders.remove(&target)
                }
            };
            touched.extend(old.map(|r| r.room_id));
        }
        touched.iter().for_each(|chat| self.touch_chat(chat));
    }

    /// Desktop's `setMessageReminder` (`at`) and `clearMessageReminder` (None) of the message `target` in `chat`.
    pub fn set_message_reminder(&mut self, chat: &str, target: &str, at: Option<i64>) {
        let room = self.reminders_room.clone().expect("message reminders wait for the reminders room");
        match at {
            Some(remind_at_ms) => {
                self.message_reminders.insert(target.into(), MessageReminder { room_id: chat.into(), remind_at_ms });
            }
            None => drop(self.message_reminders.remove(target)),
        }
        self.queue(&room, Outgoing::MessageReminder { room: chat.into(), target: target.into(), at });
        self.touch_chat(chat);
    }

    /// Desktop's `messageRemindersForChat`: the message reminders of the chat and of the chats merged into it, the
    /// soonest first, with the event ID of their message.
    pub fn message_reminders_of(&self, chat: &str) -> Vec<(&str, &MessageReminder)> {
        let of = |room: &str| {
            room == chat
                || self
                    .resolve_chat_idx(room)
                    .is_some_and(|i| self.chats[i].merged_into_chat_id.as_deref() == Some(chat))
        };
        let mut v: Vec<_> =
            self.message_reminders.iter().filter(|(_, r)| of(&r.room_id)).map(|(id, r)| (id.as_str(), r)).collect();
        v.sort_by_key(|(id, r)| (r.remind_at_ms, *id));
        v
    }

    /// Upserts the chat `id`, or its parent chat, after a change that is not in the chat itself.
    fn touch_chat(&self, id: &str) {
        let Some(i) = self.resolve_chat_idx(id) else { return };
        let c = &self.chats[i];
        let c = c.merged_into_chat_id.as_deref().and_then(|p| self.resolve_chat_idx(p)).map_or(c, |p| &self.chats[p]);
        self.emit_ws("chat.upserted", &c.id, None, Some(c.clone()), None);
    }

    /// Desktop's `setSnooze` and `clearSnooze` of one chat: `com.beeper.chats.snoozed` room account data with
    /// `snoozed_until_ms`, or empty.
    pub fn set_snooze(&mut self, idx: usize, until: Option<i64>) {
        self.update_chat(idx, |c| c.snoozed_until_ms = until);
        let id = self.chats[idx].id.clone();
        let content = until.map_or(serde_json::json!({}), |ms| serde_json::json!({ "snoozed_until_ms": ms }));
        self.queue(&id, Outgoing::RoomData(SNOOZED.into(), content));
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

    /// Desktop's `sendNudge`: "{name} sent you a nudge" as an `m.notice` with `com.beeper.nudge`, named like
    /// `getLocalSenderDisplayName`.
    pub fn send_nudge(&mut self, chat_id: &str) {
        let me = self.me();
        let full = me.full_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let user = me.username.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let local = || {
            let id = me.id.strip_prefix('@')?;
            Some(id.split_once(':').filter(|(l, _)| !l.is_empty()).map_or(id, |(l, _)| l))
        };
        let name = full.or(user).or_else(local).unwrap_or("Someone").to_string();
        self.queue(chat_id, Outgoing::Nudge(format!("{name} sent you a nudge")));
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
        let idx = self.chat_idx_by_id[&chat_id];
        // A Matrix room keeps no unread mark on its messages, only a count: the messages of others after this one are
        // left of it, until the homeserver counts again from the receipt.
        let marked = indices.iter().filter(|&&i| self.messages[i].is_unread == Some(true)).count();
        let after =
            indices.iter().map(|&i| &self.messages[i]).filter(|m| m.sort_key > key && m.is_sender != Some(true));
        let left = u32::try_from(marked)
            .expect("unread count")
            .max(u32::try_from(after.count()).expect("unread count").min(self.chats[idx].unread_count));
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

    /// Desktop's `markAsRead` through a message of the merged chat `idx`: each unread member chat is read through its
    /// newest message from the time of that message or before (`oIe`).
    pub fn mark_union_read_through(&mut self, idx: usize, message_id: &str) {
        let at = merged_order(self.message(message_id)).0;
        for i in self.members(idx) {
            let c = &self.chats[i];
            let read = c.last_read_message_sort_key.as_ref();
            let target = self.timeline(&c.id).rfind(|m| merged_order(m).0 <= at);
            let target = target.filter(|m| c.is_unread() && read.is_none_or(|r| m.sort_key > *r));
            if let Some((chat, id)) = target.map(|m| (m.chat_id.clone(), m.id.clone())) {
                self.mark_read_through(&chat, &id);
            }
        }
    }

    /// Desktop's `markAsRead`: reads the chat here, and queues the read receipt of its newest event when it had unread
    /// messages. `clear_messages` also takes the unread mark off each message.
    pub fn mark_read(&mut self, idx: usize, clear_messages: bool) {
        // Desktop's `markAsRead` of a merged chat reads each member chat, and its own unread mark.
        for m in self.members(idx) {
            self.mark_read(m, clear_messages);
        }
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
                c.listed()
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
    /// `searchRoomMessages`, or oldest first like its `sortOrder: "asc"`, with their rows, until `f` returns false.
    pub fn scan_by_time(&self, q: &Query, oldest: bool, f: impl FnMut(i64, Message) -> bool) {
        let all = if q.0.is_empty() { "1".to_string() } else { q.0.join(" AND ") };
        let dir = if oldest { "ASC" } else { "DESC" };
        self.scan_rows(&format!("{all} ORDER BY {TIME} {dir}, n {dir}"), rusqlite::params_from_iter(&q.1), f);
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

    /// Desktop's `handleRedaction`: a deleted reply leaves its reply line and its mentions, as a redacted event has
    /// neither.
    #[test]
    fn a_deleted_reply_drops_its_reply_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("deleted-reply"));
        let chat = st.chats.iter().map(|c| c.id.clone()).find(|c| st.timeline_indices(c).len() > 2).expect("a chat");
        st.load_chat(&chat);
        let ids: Vec<String> = st.chat_msg_indices[&chat].iter().map(|&i| st.messages[i].id.clone()).collect();
        let (root, reply) = (ids[0].clone(), ids[1].clone());
        let i = st.find_message(&reply).expect("the reply");
        st.update_message(i, |m| {
            (m.linked_message_id, m.mentions) = (Some(root), Some(vec!["@me:beeper.com".into()]));
            m.flags |= super::MSG_MENTION;
        });
        st.delete_message(&chat, &reply).unwrap();
        let stored = st.stored("id = ?1", [&reply]).pop().expect("the stored reply");
        for m in [st.message(&reply), &stored] {
            assert_eq!((&m.linked_message_id, &m.mentions), (&None, &None), "a deleted message replies to nothing");
            assert_eq!(m.flags & super::MSG_MENTION, 0, "and mentions no one");
        }
    }

    /// The IDs of the stored messages that a message search finds.
    fn found(st: &super::BeeperStore, q: &str) -> HashSet<String> {
        let mut ids = HashSet::new();
        st.scan(&super::Query::default().text(q), |m| ids.insert(m.id) || true);
        ids
    }

    /// Desktop's `searchRoomMessages` with `sortOrder: "asc"` reads oldest first by time and row, and its `after`
    /// cursor takes the messages past a row.
    #[test]
    fn message_search_reads_oldest_first() {
        let st = super::BeeperStore::from_config(crate::testing::demo_config("oldest-first"));
        let rows = |q: super::Query, oldest| {
            let mut rows = Vec::new();
            st.scan_by_time(&q, oldest, |n, m| {
                rows.push((crate::config::millis_from_rfc3339(&m.timestamp).expect("a message time"), n));
                true
            });
            rows
        };
        let oldest = rows(super::Query::default().has(None), true);
        assert!(oldest.len() > 2, "the demo has attachments");
        assert!(oldest.is_sorted(), "oldest first");
        let mut newest = rows(super::Query::default().has(None), false);
        newest.reverse();
        assert_eq!(newest, oldest);
        assert_eq!(rows(super::Query::default().has(None).past_row(oldest[1], true), true), oldest[2..]);
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

    /// The start pass looks for the messages that wait for their keys in each chat through their index, not by
    /// reading the JSON of every stored message.
    #[test]
    fn waiting_messages_are_found_by_their_index() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("waiting-index"));
        let sql =
            format!("EXPLAIN QUERY PLAN SELECT 1 FROM messages WHERE chat_id = ?1 AND {} LIMIT 1", super::waiting());
        let plan: String = st.db.lock().unwrap().query_row(&sql, ["!a:x"], |r| r.get(3)).unwrap();
        assert!(plan.contains("messages_waiting"), "the plan {plan}");
        let m = st.stored("1 LIMIT 1", []).remove(0);
        let chat = m.chat_id.clone();
        assert!(!st.has_waiting(&chat));
        st.load_chat(&chat);
        let i = st.find_message(&m.id).expect("the loaded message");
        st.update_message(i, |m| m.text = Some(crate::matrix::UTD_TEXT.into()));
        assert!(st.has_waiting(&chat), "a message that waits for its keys");
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
            .execute_batch("DROP TABLE outbox; DROP INDEX messages_by_key; DROP INDEX messages_with_media; DROP INDEX messages_by_thread; DROP INDEX messages_by_time; DROP TABLE room_reads; DROP INDEX messages_waiting; PRAGMA user_version = 5;")
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
                     WHERE id = 'msg_1000c'; DROP INDEX messages_by_time; DROP TABLE room_reads;
                     DROP INDEX messages_waiting; PRAGMA user_version = 8;"
            ))
            .unwrap();
        let mut st = super::BeeperStore::from_config(config);
        let i = st.find_message("msg_1000c").unwrap();
        let t = st.messages[i].attachments[0].transcription.clone().expect("a transcription");
        assert_eq!((t.engine.as_str(), t.language), ("unknown", None));
        assert!(t.transcription.starts_with("Native Rust"));
        assert_eq!(found(&st, "instantaneous"), HashSet::from(["msg_1000c".to_string()]));
    }

    /// A sync response keeps the writes of its rooms in one SQLite transaction, which their batches join, and commits
    /// them at once.
    #[test]
    fn a_sync_commits_its_rooms_at_once() {
        let config = crate::testing::demo_config("sync-tx");
        let mut st = super::BeeperStore::from_config(config.clone());
        let other = rusqlite::Connection::open(&config.db_path).expect("a second connection");
        let committed = |id: &str| {
            let q = "SELECT count(*) FROM messages WHERE id = ?1";
            other.query_row(q, [id], |r| r.get::<_, i64>(0)).expect("count a message") == 1
        };
        let chats: Vec<String> = st.chats.iter().take(2).map(|c| c.id.clone()).collect();
        let template = st.stored("1 LIMIT 1", []).pop().expect("a stored message");
        let upsert = |st: &mut super::BeeperStore, n: usize| {
            let (id, chat_id) = (format!("$sync{n}"), chats[n % 2].clone());
            let m = super::Message { id, chat_id, sort_key: super::sort_key(9_000_000 + n as i64), ..template.clone() };
            st.batch(|st| st.upsert_message(m));
        };
        st.begin_sync();
        upsert(&mut st, 0);
        upsert(&mut st, 1);
        assert!(!committed("$sync0") && !committed("$sync1"), "the rooms of the sync wait for its commit");
        assert!(st.stored_message("$sync1").is_some(), "the store reads its own writes");
        assert!(st.commit_sync() && committed("$sync0") && committed("$sync1"), "the sync commits them once");
        assert!(!st.commit_sync(), "the transaction is closed");
        upsert(&mut st, 2);
        assert!(committed("$sync2"), "a batch without a sync commits on its own");
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
                 DROP INDEX messages_by_time; DROP TABLE room_reads; DROP INDEX messages_waiting; PRAGMA user_version = 6;",
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
        // The demo store writes its data into the emptied folder.
        std::fs::remove_dir_all(&config.data_dir).expect("empty the folder");
        drop(super::BeeperStore::from_config(crate::config::CentralConfig { demo: true, ..config.clone() }));
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
    /// Desktop's `setSnooze` and `clearSnooze`: `com.beeper.chats.snoozed` room account data with the end of the
    /// snooze, or empty.
    #[test]
    fn snoozes_are_room_account_data_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("snooze"));
        let id = st.chats[0].id.clone();
        st.set_snooze(0, Some(1_900_000_000_000));
        assert_eq!(st.chats[0].snoozed_until_ms, Some(1_900_000_000_000));
        st.set_snooze(0, None);
        assert_eq!(st.chats[0].snoozed_until_ms, None);
        let data: Vec<_> = st.outbox.iter().map(|(_, r, o)| (r.clone(), o)).collect();
        assert!(matches!(&data[..], [(a, super::Outgoing::RoomData(k, on)), (b, super::Outgoing::RoomData(_, off))]
            if *a == id && *b == id && k == "com.beeper.chats.snoozed"
                && *on == serde_json::json!({ "snoozed_until_ms": 1_900_000_000_000_i64 })
                && *off == serde_json::json!({})));
    }

    /// Desktop's `UnionThreadStore`: a merged chat reads and archives its member chats, is archived when they all are,
    /// answers a reply in the chat of the message, and a member room that comes later joins it.
    #[test]
    fn merged_chats_act_on_their_member_chats_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("merged-members"));
        let (kai, lena) = ("!ig_kai:beeper.com", "!sg_lena:beeper.com");
        let members = Some(vec![kai.to_string(), lena.to_string(), "!later:x".to_string()]);
        let (chat, _) =
            st.create_or_start_chat("hungryserv", "single", Some("Friends".into()), vec![], None, members, None);
        let idx = st.chat_idx(&chat.id);
        assert_eq!(st.members(idx).len(), 2, "a member room that is not here yet is left out");
        for id in [kai, lena] {
            let i = st.chat_idx(id);
            st.update_chat(i, |c| c.unread_count = 1);
        }
        assert_eq!(st.chats[idx].unread_count, 2);
        st.mark_read(idx, false);
        assert_eq!((st.chat(kai).unread_count, st.chat(lena).unread_count, st.chats[idx].unread_count), (0, 0, 0));
        assert!(st.set_chat_flag(idx, super::ChatFlag::Archived, Some(true)));
        assert!(st.chat(kai).flag(super::ChatFlag::Archived) && st.chats[idx].flag(super::ChatFlag::Archived));
        let kai_idx = st.chat_idx(kai);
        st.set_chat_flag(kai_idx, super::ChatFlag::Archived, Some(false));
        assert!(!st.chats[idx].flag(super::ChatFlag::Archived), "archived only while all member chats are");
        st.load_chat(&chat.id);
        let (to, _, _) = st.send_message(&chat.id, Some("hi".into()), Some("msg_x4".into()), None).unwrap();
        assert_eq!(to, kai, "a reply goes to the chat of its message");
        let later = super::Chat {
            id: "!later:x".into(),
            local_chat_id: None,
            merged_into_chat_id: None,
            ..st.chat(lena).clone()
        };
        st.upsert_chat(later);
        assert_eq!(st.chat("!later:x").merged_into_chat_id.as_deref(), Some(chat.id.as_str()));
        assert_eq!(st.members(idx).len(), 3);
    }

    /// The `manage` bits of the members are not stored, so a chat loaded from SQLite has none: an upsert that brings
    /// them changes the chat, though its JSON is the same.
    #[test]
    fn an_upsert_brings_the_manage_bits_of_a_loaded_chat() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("upsert-manage"));
        let chat = st.chats[0].clone();
        assert!(chat.participants.items.iter().all(|u| u.manage == 0), "a loaded chat has no manage bits");
        let mut managed = chat.clone();
        managed.participants.items.iter_mut().for_each(|u| u.manage = 3);
        st.upsert_chat(managed);
        assert!(st.chat(&chat.id).participants.items.iter().all(|u| u.manage == 3));
    }

    /// Desktop's typing users: each `m.typing` shows its users for 60 s at most, which a later update starts again.
    /// A new message of one of them stops them alone, and leaves the others to the timeout of the update.
    #[test]
    fn typing_times_out_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("typing-timeout"));
        let chat = st.chats[0].id.clone();
        let mut rx = st.ws_tx.subscribe();
        let typing = |st: &super::BeeperStore| st.typing.get(&chat).cloned().unwrap_or_default();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let first = st.typing_update(&chat, ids(&["@a:x", "@b:x"]));
        assert_eq!(rx.try_recv().expect("an event").event_type, "chat.typing");
        let again = st.typing_update(&chat, ids(&["@a:x", "@b:x"]));
        assert!(rx.try_recv().is_err(), "the same users send no event");
        st.typing_timeout(&chat, first);
        assert_eq!(typing(&st), ids(&["@a:x", "@b:x"]), "the timeout of an older update keeps them");
        st.typing_timeout(&chat, again);
        assert!(typing(&st).is_empty(), "that of the last update stops them");
        assert_eq!(rx.try_recv().expect("an event").event_type, "chat.typing");
        let last = st.typing_update(&chat, ids(&["@a:x", "@b:x"]));
        st.stop_typing(&chat, &ids(&["@a:x"]));
        assert_eq!(typing(&st), ids(&["@b:x"]), "a message stops its sender");
        st.typing_timeout(&chat, last);
        assert!(typing(&st).is_empty(), "and the timeout of the update the others");
    }

    /// A sync upserts each of its rooms: an unchanged room writes nothing, so it sends no event and keeps what the
    /// views found from the chats, and a changed one is written.
    #[test]
    fn an_unchanged_chat_upsert_writes_nothing() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("unchanged-upsert"));
        let mut rx = st.ws_tx.subscribe();
        let chat = st.chats[0].clone();
        let changes = st.chat_changes;
        st.upsert_chat(chat.clone());
        assert_eq!(st.chat_changes, changes, "the same chat changes nothing");
        assert!(rx.try_recv().is_err(), "and sends no event");
        st.upsert_chat(super::Chat { title: "Renamed".into(), ..chat.clone() });
        assert_eq!((st.chat_changes, st.chat(&chat.id).title.as_str()), (changes + 1, "Renamed"));
        assert_eq!(rx.try_recv().expect("an event").event_type, "chat.upserted");
    }

    /// Desktop's `activeUnionIds`: past `max-merge-chats` (1 by default), the newer merged chats hide and their member
    /// chats show on their own, until the limit grows or an older merged chat goes.
    #[test]
    fn merged_chats_past_the_limit_hide_like_desktop() {
        let mut st = super::BeeperStore::from_config(crate::testing::demo_config("merged-limit"));
        let pair = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        let old = st.create_union("Old", pair("!ig_kai:beeper.com", "!sg_lena:beeper.com"), None, None);
        let i = st.chat_idx(&old);
        st.update_chat(i, |c| c.created_at = Some(1));
        let new = st.create_union("New", pair("!wa_sam:beeper.com", "!im_jordan:beeper.com"), None, None);
        let i = st.chat_idx(&new);
        st.update_chat(i, |c| c.created_at = Some(2));
        st.apply_union_limit();
        let shown = |st: &super::BeeperStore, id: &str| st.chat(id).listed();
        assert!(shown(&st, &old) && !shown(&st, &new), "the oldest merged chat is the one that the plan allows");
        assert!(shown(&st, "!wa_sam:beeper.com") && !shown(&st, "!ig_kai:beeper.com"));
        st.features.flags = serde_json::json!({ "features": { "*": { "max-merge-chats": "-1" } } });
        st.apply_union_limit();
        assert!(shown(&st, &new) && !shown(&st, "!wa_sam:beeper.com"), "no limit shows every merged chat");
        st.features.flags = serde_json::json!({ "features": { "*": { "max-merge-chats": 1 } } });
        st.apply_union_limit();
        st.unmerge(&old);
        assert!(shown(&st, &new) && !shown(&st, "!wa_sam:beeper.com"), "the next merged chat takes the free place");
        // A member room that went elsewhere leaves its merged chat.
        st.remove_deleted_chat("!wa_sam:beeper.com");
        let rooms = st.chat(&new).merge.as_ref().map(|m| m.chat_ids.clone());
        assert_eq!(rooms, Some(vec!["!im_jordan:beeper.com".to_string()]));
        let out = st.outbox.last().map(|(_, c, o)| {
            (c.clone(), matches!(o, super::Outgoing::LabelChild(r, false) if r == "!wa_sam:beeper.com"))
        });
        assert_eq!(out, Some((new.clone(), true)));
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

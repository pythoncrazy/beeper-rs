use crate::config::{
    assert_not_original_beeper_dir, millis_from_rfc3339, now_rfc3339_millis, now_unix_secs, CentralConfig,
    DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE, DEFAULT_TOKEN, DEFAULT_USER_ID, SEED_JSON,
    WS_CHANNEL_CAPACITY,
};
use crate::matrix::{ChatFlag, Outgoing};
use crate::models::*;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::broadcast;

pub type SharedStore = std::sync::Arc<StoreLock>;

thread_local!(static LOCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

/// A std `RwLock` deadlocks when a thread that holds the store locks it again while a writer waits,
/// so a nested lock on one thread panics.
pub struct StoreLock(std::sync::RwLock<BeeperStore>);

pub struct Locked<G>(G);

impl StoreLock {
    pub fn shared(store: BeeperStore) -> SharedStore {
        std::sync::Arc::new(Self(std::sync::RwLock::new(store)))
    }

    fn hold<G>(lock: impl FnOnce() -> G) -> Locked<G> {
        assert!(!LOCKED.replace(true), "the store is already locked on this thread");
        Locked(lock())
    }

    pub fn read(&self) -> Locked<std::sync::RwLockReadGuard<'_, BeeperStore>> {
        Self::hold(|| self.0.read().expect("store lock"))
    }

    pub fn write(&self) -> Locked<std::sync::RwLockWriteGuard<'_, BeeperStore>> {
        Self::hold(|| self.0.write().expect("store lock"))
    }
}

impl<G> Drop for Locked<G> {
    fn drop(&mut self) {
        LOCKED.set(false);
    }
}

impl<G: std::ops::Deref> std::ops::Deref for Locked<G> {
    type Target = G::Target;
    fn deref(&self) -> &G::Target {
        &self.0
    }
}

impl<G: std::ops::DerefMut> std::ops::DerefMut for Locked<G> {
    fn deref_mut(&mut self) -> &mut G::Target {
        &mut self.0
    }
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> =
        text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[derive(Debug, Default, Clone)]
pub struct InvertedIndex {
    postings: BTreeMap<String, Vec<u32>>,
}

impl InvertedIndex {
    pub fn insert_doc(&mut self, doc: u32, text: &str) {
        for tok in tokenize(text) {
            let list = self.postings.entry(tok).or_default();
            let pos = list.binary_search(&doc).expect_err("document indexed twice");
            list.insert(pos, doc);
        }
    }

    pub fn remove_doc(&mut self, doc: u32, text: &str) {
        for tok in tokenize(text) {
            let list = self.postings.get_mut(&tok).expect("removed token is indexed");
            list.remove(list.binary_search(&doc).expect("removed document is indexed"));
        }
    }

    fn prefix_docs(&self, prefix: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .postings
            .range::<str, _>((std::ops::Bound::Included(prefix), std::ops::Bound::Unbounded))
            .take_while(|(t, _)| t.starts_with(prefix))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn query_all_tokens(&self, query: &str) -> Vec<u32> {
        let mut sets = tokenize(query).into_iter().map(|q| self.prefix_docs(&q));
        let Some(first) = sets.next() else { return Vec::new() };
        sets.fold(first, |acc, ids| acc.into_iter().filter(|id| ids.binary_search(id).is_ok()).collect())
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

#[rustfmt::skip]
const LINK_PREVIEWS: [(&str, &str, &str); 3] = [
    ("https://arxiv.org/abs/2001.08361", "Scaling Laws for Neural Language Models", "We study empirical scaling laws for language model performance on the cross-entropy loss. The loss scales as a power-law with model size, dataset size, and the amount of compute used for training."),
    ("https://github.com/pythoncrazy/navify", "pythoncrazy/navify", "A fast, native Navidrome client written in Rust with iced. Low memory use, instant startup, and a polished desktop interface."),
    ("https://github.com/pythoncrazy/beeper-rs", "pythoncrazy/beeper-rs", "Ultra-fast, cross-platform, Rust-native drop-in replacement for Beeper Desktop with a compatible Desktop API server."),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    bridges: Vec<BridgeAccountInfo>,
    chats: Vec<Chat>,
    messages: Vec<Message>,
}

const SCHEMA_VERSION: i64 = 4;
const SCHEMA: &str = "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
     CREATE TABLE chats (id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
     CREATE TABLE messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL) STRICT;
     CREATE TABLE assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;";
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
            master_key: "mk_ed25519_beeper_rs_master".to_string(),
            self_signing_key: "ssk_ed25519_beeper_rs_self".to_string(),
            user_signing_key: "usk_ed25519_beeper_rs_user".to_string(),
            megolm_backup_key: "mbk_curve25519_beeper_rs_backup".to_string(),
            recovery_code: DEFAULT_RECOVERY_CODE.to_string(),
        },
    }
}

fn chat_index_text(c: &Chat) -> String {
    let people = c
        .participants
        .items
        .iter()
        .flat_map(|p| [&p.full_name, &p.custom_nickname, &p.username, &p.phone_number, &p.email]);
    [&Some(c.title.clone()), &c.description]
        .into_iter()
        .chain(people)
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

fn message_index_text(m: &Message) -> String {
    let files = m.attachments.iter().flat_map(|a| [&a.file_name, &a.transcript]);
    [&m.text, &m.sender_name].into_iter().chain(files).flatten().map(String::as_str).collect::<Vec<_>>().join(" ")
}

/// A network inside a bridged account, like a Discord server. Its chats have the same `network_id`.
#[derive(Debug, Clone)]
pub struct NetworkSpace {
    pub id: String,
    pub account_id: String,
    pub name: String,
}

pub struct BeeperStore {
    pub config: CentralConfig,
    pub data_dir: PathBuf,
    pub setup: Option<Setup>,
    db: Mutex<Connection>,
    pub beeper_state: BeeperStateRecord,
    pub setup_status: SetupStatusResponse,
    pub verifications: HashMap<String, VerificationState>,
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
    pub chat_idx_by_id: HashMap<String, usize>,
    local_chat_idx_by_id: HashMap<String, usize>,
    pub messages: Vec<Message>,
    pub msg_idx_by_id: HashMap<String, usize>,
    pub chat_msg_indices: HashMap<String, Vec<usize>>,
    pub msg_search_index: InvertedIndex,
    pub chat_search_index: InvertedIndex,
    pub assets: HashMap<String, StoredAsset>,
    assets_by_mxc: HashMap<String, String>,
    pub connect: crate::connect::Connect,
    /// A new token request (no outcome), or the end of one.
    pub connect_tx: broadcast::Sender<(String, Option<crate::connect::Outcome>)>,
    pub settings: BeeperSettings,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub focus_tx: broadcast::Sender<Option<String>>,
    /// Whether the Desktop API server should listen.
    pub api_on: tokio::sync::watch::Sender<bool>,
    pub link_previews: HashMap<String, LinkPreview>,
    pub matrix: Option<matrix_sdk::Client>,
    pub matrix_error: Option<String>,
    pub sas_confirm: Option<tokio::sync::oneshot::Sender<bool>>,
    pub outbox: Vec<(String, Outgoing)>,
    pub pushing: bool,
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
    pub features: crate::features::Flags,
    next_seq: i64,
}

impl BeeperStore {
    pub fn from_config(config: CentralConfig) -> Self {
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
                .execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"))
                .expect("create beeper-rs schema"),
            1..SCHEMA_VERSION => {
                let steps = [
                    "UPDATE assets SET json = json_set(json_remove(json, '$.local_file_path'), '$.disk_name',
                         substr(json_extract(json, '$.local_file_path'), instr(json_extract(json, '$.local_file_path'), upload_id || '_')));"
                        .to_string(),
                    widen_sort_keys(),
                    widen_sort_keys(),
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
        let beeper_state = if config.demo { demo_state() } else { BeeperStateRecord::default() };
        let features =
            if config.demo { crate::features::Flags::free_plan() } else { crate::features::Flags::default() };
        let mut store = Self {
            data_dir: config.data_dir.clone(),
            setup: None,
            settings: config.settings.clone(),
            config,
            db: Mutex::new(conn),
            setup_status: Self::build_setup_from_state(&beeper_state),
            beeper_state,
            verifications: HashMap::new(),
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
            local_chat_idx_by_id: HashMap::new(),
            messages: Vec::new(),
            msg_idx_by_id: HashMap::new(),
            chat_msg_indices: HashMap::new(),
            msg_search_index: InvertedIndex::default(),
            chat_search_index: InvertedIndex::default(),
            assets: HashMap::new(),
            assets_by_mxc: HashMap::new(),
            link_previews: LINK_PREVIEWS
                .iter()
                .map(|&(url, title, summary)| {
                    (url.to_string(), LinkPreview { title: title.into(), summary: summary.into() })
                })
                .collect(),
            connect: crate::connect::Connect::default(),
            connect_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            ws_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            focus_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            api_on: tokio::sync::watch::channel(false).0,
            matrix: None,
            matrix_error: None,
            sas_confirm: None,
            outbox: Vec::new(),
            pushing: false,
            sent: HashMap::new(),
            typing: HashMap::new(),
            recent_emoji: Vec::new(),
            scheduled: Vec::new(),
            synced_prefs: BTreeMap::new(),
            skin_tone: 0,
            inbox_rules: serde_json::Value::Null,
            features,
            next_seq: 1000,
        };
        store.load_from_sqlite_or_seed();
        if store.config.demo {
            let label = |(id, name, color, icon): (&str, &str, Option<u8>, Option<&str>)| {
                let rooms = store.chats.iter().filter(|c| c.label_ids.iter().any(|l| l == id));
                Label {
                    id: id.into(),
                    name: name.into(),
                    color,
                    icon: icon.map(Into::into),
                    rooms: rooms.map(|c| c.id.clone()).collect(),
                    ..Label::default()
                }
            };
            let labels = [
                ("lbl_favorite", "Favorites", Some(6), Some("3257-5950")),
                ("lbl_work", "Work", Some(11), None),
                ("lbl_vip", "VIP", None, None),
            ];
            store.labels = labels.map(label).into();
        }
        store
    }

    pub fn build_setup_from_state(state: &BeeperStateRecord) -> SetupStatusResponse {
        let s = &state.secrets;
        let secrets = [
            ("masterKey", &s.master_key),
            ("selfSigningKey", &s.self_signing_key),
            ("userSigningKey", &s.user_signing_key),
            ("megolmBackupKey", &s.megolm_backup_key),
            ("recoveryCode", &s.recovery_code),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), !v.is_empty()))
        .collect();
        let signed_in = !state.access_token.is_empty();
        let phase = |active: bool, inactive: &str| match (active, state.verified) {
            (false, _) => inactive.to_string(),
            (true, false) => "needs_verification".to_string(),
            (true, true) => "ready".to_string(),
        };
        SetupStatusResponse {
            session: SetupSession {
                state: phase(signed_in, "signed_out"),
                e2ee: SetupE2ee {
                    state: phase(state.initialized, "idle"),
                    cross_signing: state.cross_signing,
                    first_sync_done: state.first_sync_done,
                    has_backed_up_code: state.has_backed_up_code,
                    initialized: state.initialized,
                    key_backup: state.key_backup,
                    secret_storage: state.secret_storage,
                    verified: state.verified,
                    verification_methods: ["recovery_key", "other_device", "qr_code", "sas"].map(String::from).to_vec(),
                    secrets,
                },
                matrix: signed_in.then(|| SetupMatrix {
                    homeserver: state.homeserver.clone(),
                    user_id: state.user_id.clone(),
                    device_id: Some(state.device_id.clone()),
                }),
                verification: None,
            },
        }
    }

    fn exec(&self, sql: &str, args: &[&dyn ToSql]) {
        self.db.lock().unwrap().prepare_cached(sql).expect("prepare SQL").execute(args).expect("execute SQL");
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
            "INSERT OR REPLACE INTO messages (id, chat_id, sort_key, json) VALUES (?1, ?2, ?3, ?4)",
            params![m.id, m.chat_id, m.sort_key, serde_json::to_string(m).unwrap()],
        );
    }

    fn load_from_sqlite_or_seed(&mut self) {
        let (setup, state, bridges, settings, pin_rank, recents, contacts, seq, chats, messages, assets) = {
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
                kv(KEY_SEQ),
                rows("SELECT json FROM chats ORDER BY rowid"),
                rows("SELECT json FROM messages ORDER BY sort_key ASC"),
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
        self.setup_status = Self::build_setup_from_state(&self.beeper_state);
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
            self.settings = BeeperSettings {
                pas_port: self.settings.pas_port,
                ..serde_json::from_str(&s).expect("parse stored settings")
            };
            self.settings.assert_valid();
        }
        for a in assets {
            let asset: StoredAsset = serde_json::from_str(&a).expect("parse stored asset");
            self.assets_by_mxc.insert(asset.mxc_url.clone(), asset.upload_id.clone());
            self.assets.insert(asset.upload_id.clone(), asset);
        }
        let fresh = chats.is_empty();
        assert!(!fresh || messages.is_empty(), "stored messages without chats");
        if fresh && self.config.demo {
            self.seed_default_workspace();
            self.finish_setup(Setup::Fresh);
        }
        for c in chats {
            self.push_chat(serde_json::from_str(&c).expect("parse stored chat"), false);
        }
        for m in messages {
            self.push_message(serde_json::from_str(&m).expect("parse stored message"), false);
        }
        match seq {
            Some(s) => self.next_seq = self.next_seq.max(s.parse().expect("parse stored nextSeq")),
            None => assert!(fresh, "stored chats without a stored nextSeq"),
        }
        self.persist_seq();
        if self.signed_in() {
            self.ensure_note_chat(&Profile::default());
        }
        self.rebuild_accounts_from_bridges();
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

    /// Desktop's `scheduleCurrentDraft`.
    pub fn schedule(&mut self, chat_id: &str, text: String, reply_to: Option<String>, send_on: i64, no_reply: bool) {
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
        let replied = |st: &Self, s: &Scheduled| {
            let msgs = st.chat_msg_indices.get(&s.chat_id).into_iter().flatten().map(|&i| &st.messages[i]);
            msgs.filter(|m| crate::config::millis_from_rfc3339(&m.timestamp).is_some_and(|t| t >= s.added_on))
                .any(|m| m.is_sender != Some(true) && !m.silent())
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
            self.push_chat(c, false);
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
        self.data_dir.join("matrix")
    }

    pub fn signed_in(&self) -> bool {
        !self.beeper_state.access_token.is_empty()
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
        self.setup_status = Self::build_setup_from_state(&self.beeper_state);
        self.persist_state_to_sqlite();
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
            is_admin: None,
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
            notifiable: None,
            is_hidden_from_inbox: None,
            is_request: None,
            network_name: None,
            network_id: None,
            img_url: None,
            delete_modes: 0,
            formatting: ALL_FORMATS,
            files: ALL_FILES,
            polls: Some(BEEPER_POLLS),
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
                    self.contacts.push(u.clone());
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

    pub fn queue(&mut self, chat_id: &str, out: Outgoing) {
        if !self.config.demo {
            self.outbox.push((chat_id.into(), out));
        }
    }

    /// Sets a chat flag here and on the homeserver. `None` toggles the flag.
    /// Returns false, like Desktop, when asked to archive a pinned chat.
    pub fn set_chat_flag(&mut self, idx: usize, flag: ChatFlag, on: Option<bool>) -> bool {
        let c = &self.chats[idx];
        if flag == ChatFlag::Archived && c.is_pinned == Some(true) && on.unwrap_or(c.is_archived != Some(true)) {
            return false;
        }
        let mut value = false;
        self.update_chat(idx, |c| {
            let slot = match flag {
                ChatFlag::Archived => &mut c.is_archived,
                ChatFlag::Pinned => &mut c.is_pinned,
                ChatFlag::LowPriority => &mut c.is_low_priority,
                ChatFlag::Muted => &mut c.is_muted,
                ChatFlag::MarkedUnread => &mut c.is_marked_unread,
            };
            value = on.unwrap_or(*slot != Some(true));
            *slot = Some(value);
        });
        let id = self.chats[idx].id.clone();
        self.queue(&id, Outgoing::Flag(flag, value));
        true
    }

    /// Sets or dismisses a chat reminder here and on the homeserver.
    pub fn set_reminder(&mut self, idx: usize, reminder: Option<ChatReminder>) {
        self.update_chat(idx, |c| c.reminder = reminder.clone());
        let id = self.chats[idx].id.clone();
        self.queue(&id, Outgoing::Reminder(reminder));
    }

    pub fn start_flush(&mut self) -> bool {
        let start = !self.pushing && self.matrix.is_some() && !self.outbox.is_empty();
        self.pushing |= start;
        start
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
                user: User { id: format!("@self_{}:beeper.com", b.bridge_id), ..self.me().clone() },
            })
            .collect();
    }

    fn push_chat(&mut self, mut chat: Chat, persist: bool) -> usize {
        chat.sync_flags();
        chat.flags = self.label_flags(&chat);
        let idx = self.chats.len();
        assert!(self.chat_idx_by_id.insert(chat.id.clone(), idx).is_none(), "duplicate chat {}", chat.id);
        self.chat_search_index.insert_doc(idx as u32, &chat_index_text(&chat));
        self.chat_msg_indices.entry(chat.id.clone()).or_default();
        if let Some(loc) = &chat.local_chat_id {
            self.local_chat_idx_by_id.insert(loc.clone(), idx);
        }
        self.chats.push(chat);
        if persist {
            self.persist_chat(idx);
        }
        idx
    }

    fn push_message(&mut self, mut msg: Message, persist: bool) -> usize {
        msg.sync_flags();
        let digits = msg.sort_key.strip_prefix('-').unwrap_or(&msg.sort_key).len();
        assert_eq!(digits, 16, "sort key {} of {} is not from sort_key", msg.sort_key, msg.id);
        self.next_seq = self.next_seq.max(msg.sort_key.parse().expect("sort keys are numbers"));
        let idx = self.messages.len();
        assert!(self.msg_idx_by_id.insert(msg.id.clone(), idx).is_none(), "duplicate message {}", msg.id);
        self.msg_search_index.insert_doc(idx as u32, &message_index_text(&msg));
        let list = self.chat_msg_indices.entry(msg.chat_id.clone()).or_default();
        let pos = list.partition_point(|&i| self.messages[i].sort_key <= msg.sort_key);
        list.insert(pos, idx);
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        let chat = &mut self.chats[c_idx];
        if !msg.silent() && chat.preview.as_ref().is_none_or(|p| p.sort_key <= msg.sort_key) {
            chat.last_activity = Some(msg.timestamp.clone());
            chat.preview = Some(msg.clone());
        }
        self.messages.push(msg);
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

    /// Desktop's `setLabelInboxVisibility`: `None` shows all chats of the label in the inbox.
    pub fn set_label_inbox(&mut self, id: &str, inbox: Option<&'static str>) {
        let label = self.labels.iter().find(|l| l.id == id).expect("a label").clone();
        self.set_label(Label { inbox, ..label });
        let content = serde_json::json!({ "visibility": inbox.unwrap_or("all") });
        self.queue(id, Outgoing::LabelState("com.beeper.label.inbox_visibility", content));
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
        self.queue(id, Outgoing::LabelState("com.beeper.muted", serde_json::json!({ "muted": muted })));
    }

    /// Desktop's `areNotificationsMuted`: the chat is muted, all labels of the chat are muted, or for a chat
    /// without labels, "Enable notifications" of its account is off.
    pub fn notifications_muted(&self, c: &Chat) -> bool {
        if c.is_muted == Some(true) {
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
        self.queue(id, Outgoing::Leave);
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
                notifiable: c.notifiable.take(),
                ..chat
            }
        });
    }

    /// Drops a chat that the user left, with its messages, and rebuilds the indices.
    pub fn remove_chat(&mut self, id: &str) {
        self.remove_label(id);
        if !self.chat_idx_by_id.contains_key(id) {
            return;
        }
        let (chats, messages) = (std::mem::take(&mut self.chats), std::mem::take(&mut self.messages));
        let live = std::mem::take(&mut self.msg_idx_by_id);
        (self.chat_idx_by_id, self.local_chat_idx_by_id, self.chat_msg_indices) = Default::default();
        (self.chat_search_index, self.msg_search_index) = Default::default();
        for c in chats.into_iter().filter(|c| c.id != id) {
            self.push_chat(c, false);
        }
        for (i, m) in messages.into_iter().enumerate() {
            if m.chat_id != id && live.get(&m.id) == Some(&i) {
                self.push_message(m, false);
            }
        }
        self.exec("DELETE FROM messages WHERE chat_id = ?1", params![id]);
        self.exec("DELETE FROM chats WHERE id = ?1", params![id]);
        self.recent_chat_ids.retain(|c| c != id);
        self.typing.remove(id);
        self.outbox.retain(|(c, _)| c != id);
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

    pub fn upsert_message(&mut self, msg: Message) {
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
        let old = chat_index_text(&self.chats[idx]);
        f(&mut self.chats[idx]);
        self.chats[idx].sync_flags();
        self.chats[idx].flags = self.label_flags(&self.chats[idx]);
        self.chat_search_index.remove_doc(idx as u32, &old);
        self.chat_search_index.insert_doc(idx as u32, &chat_index_text(&self.chats[idx]));
        self.persist_chat(idx);
        let chat = self.chats[idx].clone();
        self.emit_ws("chat.upserted", &chat.id, None, Some(chat.clone()), None);
        chat
    }

    pub fn update_message(&mut self, idx: usize, f: impl FnOnce(&mut Message)) -> Message {
        let old = message_index_text(&self.messages[idx]);
        f(&mut self.messages[idx]);
        self.messages[idx].sync_flags();
        self.msg_search_index.remove_doc(idx as u32, &old);
        self.msg_search_index.insert_doc(idx as u32, &message_index_text(&self.messages[idx]));
        self.persist_message(idx);
        let msg = self.messages[idx].clone();
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        if self.chats[c_idx].preview.as_ref().is_some_and(|p| p.id == msg.id) {
            self.chats[c_idx].preview = Some(msg.clone());
            self.persist_chat(c_idx);
        }
        self.emit_ws("message.upserted", &msg.chat_id, Some(&msg.id), None, Some(msg.clone()));
        msg
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        self.next_seq += 1;
        self.persist_seq();
        format!("{prefix}_{}_{}", crate::config::now_unix_millis(), self.next_seq)
    }

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
                is_admin: None,
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
            notifiable: None,
            is_hidden_from_inbox: None,
            is_request: None,
            network_name: None,
            network_id: None,
            img_url: None,
            participants: ChatParticipants { has_more: false, total: items.len(), items },
            delete_modes: 0,
            formatting: ALL_FORMATS,
            files: ALL_FILES,
            polls: Some(BEEPER_POLLS),
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

    pub fn patch_chat(
        &mut self,
        chat_or_local_id: &str,
        clear_draft: bool,
        new_draft: Option<ChatDraft>,
        new_merge: Option<Option<ChatMergeInfo>>,
    ) -> Result<Chat, &'static str> {
        let idx = self.resolve_chat_idx(chat_or_local_id).ok_or("Chat not found")?;
        let has_content = |d: &ChatDraft| d.text.as_deref().is_some_and(|s| !s.is_empty()) || !d.attachments.is_empty();
        let draft = match new_draft {
            _ if clear_draft => None,
            Some(d) if has_content(&d) && self.chats[idx].draft.as_ref().is_some_and(has_content) => {
                return Err("Cannot overwrite a non-empty draft. Clear the draft first by setting draft to null or draft.text to an empty string.")
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

    pub fn send_message(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        reply_to_message_id: Option<String>,
        attachment: Option<Attachment>,
    ) -> Result<(String, String, Message), &'static str> {
        let c_idx = self.send_target(self.resolve_chat_idx(chat_or_local_id).ok_or("Chat not found")?);
        let (chat_id, account_id) = (self.chats[c_idx].id.clone(), self.chats[c_idx].account_id.clone());
        let msg_id = self.next_id("msg");
        let mut attachment = attachment;
        let msg_type = match attachment.as_mut() {
            None => "TEXT",
            Some(att) if att.is_voice_note == Some(true) => "VOICE",
            Some(att) if att.attachment_type == "img" => "IMAGE",
            Some(att) if att.attachment_type == "video" => "VIDEO",
            Some(_) => "FILE",
        };
        let msg = Message {
            id: msg_id.clone(),
            account_id: account_id.clone(),
            chat_id: chat_id.clone(),
            sender_id: format!("@self_{account_id}:beeper.com"),
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
            linked_message_id: reply_to_message_id,
            attachments: attachment.into_iter().collect(),
            reactions: Vec::new(),
            poll: None,
            unresolved_media: None,
            edit_history: Vec::new(),
            is_edited: None,
            links: Vec::new(),
            seen: BTreeMap::new(),
            flags: 0,
        };
        let m_idx = self.push_message(msg, true);
        let msg = self.messages[m_idx].clone();
        self.emit_ws("message.upserted", &chat_id, Some(&msg_id), None, Some(msg.clone()));
        self.emit_ws("chat.upserted", &chat_id, None, Some(self.chats[c_idx].clone()), None);
        Ok((chat_id, msg_id, msg))
    }

    fn message_in_chat(&self, chat_or_local_id: &str, message_id: &str) -> Result<(String, usize), &'static str> {
        let chat_id = self.resolve_canonical_chat_id(chat_or_local_id).ok_or("Chat not found")?;
        let &m_idx = self.msg_idx_by_id.get(message_id).ok_or("Message not found")?;
        assert_eq!(self.messages[m_idx].chat_id, chat_id, "message belongs to another chat");
        Ok((chat_id, m_idx))
    }

    pub fn edit_message(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        new_text: String,
    ) -> Result<(String, Message), &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let msg = self.update_message(m_idx, |m| {
            m.html = crate::codec::markdown_html(&new_text);
            let prev = m.text.replace(new_text).expect("edited message has text");
            m.edit_history.push(EditHistoryEntry { text: prev, timestamp: now_rfc3339_millis() });
        });
        Ok((chat_id, msg))
    }

    pub fn delete_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        self.update_message(m_idx, |m| {
            (m.is_deleted, m.is_edited) = (Some(true), Some(true));
            (m.text, m.html, m.poll) = (None, None, None);
            m.edit_history.clear();
            m.attachments.clear();
            m.reactions.clear();
            m.links.clear();
        });
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
    }

    pub fn remove_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        self.msg_search_index.remove_doc(m_idx as u32, &message_index_text(&self.messages[m_idx]));
        self.messages[m_idx].is_deleted = Some(true);
        self.messages[m_idx].flags |= MSG_DELETED;
        (self.messages[m_idx].text, self.messages[m_idx].html) = (None, None);
        self.msg_idx_by_id.remove(message_id);
        self.chat_msg_indices.get_mut(&chat_id).expect("chat has messages").retain(|&i| i != m_idx);
        self.exec("DELETE FROM messages WHERE id = ?1", params![message_id]);
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
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

    pub fn add_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
        transaction_id: Option<String>,
    ) -> Result<(String, String), &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let tx_id = transaction_id.unwrap_or_else(|| self.next_id("tx_rx"));
        let rx_id = tx_id.clone();
        let pid = format!("@self_{}:beeper.com", self.messages[m_idx].account_id);
        self.update_message(m_idx, |m| {
            m.reactions.retain(|r| !(r.participant_id == pid && r.reaction_key == reaction_key));
            m.reactions.push(Reaction {
                id: rx_id,
                participant_id: pid.clone(),
                reaction_key: reaction_key.to_string(),
                animated: Some(false),
                img_url: None,
                sort_key: String::new(),
            });
        });
        Ok((chat_id, tx_id))
    }

    pub fn remove_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
    ) -> Result<String, &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        let mine = |r: &Reaction| r.reaction_key == reaction_key && self.is_me(&r.participant_id);
        let ids: Vec<String> =
            self.messages[m_idx].reactions.iter().filter(|r| mine(r)).map(|r| r.id.clone()).collect();
        self.update_message(m_idx, |m| m.reactions.retain(|r| !ids.contains(&r.id)));
        Ok(chat_id)
    }

    pub fn toggle_bookmark(&mut self, message_id: &str) -> bool {
        let m_idx = self.msg_idx_by_id[message_id];
        self.update_message(m_idx, |m| m.is_bookmarked = Some(m.is_bookmarked != Some(true))).is_bookmarked
            == Some(true)
    }

    pub fn retranscribe_voice_note(&mut self, message_id: &str) -> String {
        let m_idx = self.msg_idx_by_id[message_id];
        let label = crate::prefs::LANGUAGES[self.settings.pref("appLanguage") as usize];
        let msg = self.update_message(m_idx, |m| {
            let att =
                m.attachments.iter_mut().find(|a| a.is_voice_note == Some(true)).expect("message has a voice note");
            let old = att.transcript.as_deref().expect("voice note has a transcript");
            att.transcript = Some(format!("[Re-transcribed ({label})]: {old}"));
        });
        msg.attachments.into_iter().find_map(|a| a.transcript).unwrap()
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
        if let Some(&i) = self.msg_idx_by_id.get(message_id).filter(|&&i| self.messages[i].poll.is_some()) {
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

    pub fn mark_chat_read(&mut self, chat_or_local_id: &str, clear_messages: bool) -> Option<String> {
        let idx = self.resolve_chat_idx(chat_or_local_id)?;
        let chat_id = self.chats[idx].id.clone();
        let indices = self.chat_msg_indices.get(&chat_id).cloned().unwrap_or_default();
        let last_key = indices.last().map(|&i| self.messages[i].sort_key.clone());
        let last_event = indices.iter().rev().map(|&i| &self.messages[i].id).find(|id| id.starts_with('$')).cloned();
        let had_unread = self.chats[idx].flags & CHAT_UNREAD != 0;
        if self.chats[idx].is_marked_unread == Some(true) {
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

    pub fn set_user_nickname(&mut self, chat_or_local_id: &str, user_id: &str, nickname: Option<String>) {
        let idx = self.resolve_chat_idx(chat_or_local_id).expect("chat exists");
        self.update_chat(idx, |c| {
            c.participants
                .items
                .iter_mut()
                .filter(|p| p.id == user_id)
                .for_each(|p| p.custom_nickname = nickname.clone());
            if let (Some(nick), "single") = (&nickname, c.chat_type.as_str()) {
                c.title = nick.clone();
            }
        });
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
        let asset = StoredAsset {
            mxc_url: format!("mxc://beeper.com/{upload_id}"),
            upload_id: upload_id.clone(),
            disk_name,
            file_name,
            mime_type,
            file_size: bytes.len() as u64,
            width: None,
            height: None,
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
        for id in messages {
            let i = self.msg_idx_by_id.remove(id).expect("a stored message");
            self.msg_search_index.remove_doc(i as u32, &message_index_text(&self.messages[i]));
            let m = &mut self.messages[i];
            (m.is_deleted, m.text, m.html) = (Some(true), None, None);
            m.flags |= MSG_DELETED;
        }
        let msgs = &self.messages;
        for list in self.chat_msg_indices.values_mut() {
            list.retain(|&i| !messages.contains(&msgs[i].id));
        }
        self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin forget transaction");
        for id in messages {
            self.exec("DELETE FROM messages WHERE id = ?1", params![id]);
        }
        self.db.lock().unwrap().execute_batch("COMMIT; VACUUM;").expect("commit forget transaction");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn widened_sort_keys_match_sort_key() {
        let old = |seq: i64| if seq < 0 { format!("-{:012}", 1_000_000_000_000 + seq) } else { format!("{seq:012}") };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(super::SCHEMA).unwrap();
        let seqs = [-5, -1, 0, 7, 123_456_789];
        for seq in seqs {
            let (key, json) = (old(seq), serde_json::json!({ "sortKey": old(seq) }).to_string());
            db.execute("INSERT INTO messages VALUES (?1, 'c', ?1, ?2)", [&key, &json]).unwrap();
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
    fn sent_messages_come_last() {
        let dir = std::env::temp_dir().join(format!("beeper-rs-send-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = crate::config::CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23392, None);
        config.demo = true;
        let mut st = super::BeeperStore::from_config(config);
        let chat = st.messages.iter().max_by_key(|m| m.sort_key.clone()).unwrap().chat_id.clone();
        let (_, id, _) = st.send_message(&chat, Some("hi".into()), None, None).unwrap();
        let last = *st.chat_msg_indices[&chat].last().unwrap();
        assert_eq!(st.messages[last].id, id);
        assert_eq!(st.chats[st.chat_idx_by_id[&chat]].preview.as_ref().unwrap().id, id);
        drop(st);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn labels_set_the_label_ids_of_their_chats() {
        let dir = std::env::temp_dir().join(format!("beeper-rs-labels-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = crate::config::CentralConfig::new(dir.clone(), "127.0.0.1".into(), 23393, None);
        config.demo = true;
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
        drop(st);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

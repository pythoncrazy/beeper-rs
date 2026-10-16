use crate::config::{
    assert_not_original_beeper_dir, millis_from_rfc3339, now_rfc3339_millis, now_unix_secs, CentralConfig,
    DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE, DEFAULT_TOKEN, DEFAULT_USER_ID, SEED_JSON,
    WS_CHANNEL_CAPACITY,
};
use crate::models::*;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::broadcast;

pub type SharedStore = std::sync::Arc<std::sync::RwLock<BeeperStore>>;

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

#[derive(Debug, Clone)]
pub struct OAuthClientEntry {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthCodeEntry {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

pub const NOTE_CHAT_ID: &str = "!note_self:beeper.com";

#[rustfmt::skip]
const LINK_PREVIEWS: [(&str, &str, &str, &str); 3] = [
    ("https://arxiv.org/abs/2001.08361", "arXiv.org", "Scaling Laws for Neural Language Models", "We study empirical scaling laws for language model performance on the cross-entropy loss. The loss scales as a power-law with model size, dataset size, and the amount of compute used for training."),
    ("https://github.com/pythoncrazy/navify", "GitHub", "pythoncrazy/navify", "A fast, native Navidrome client written in Rust with iced. Low memory use, instant startup, and a polished desktop interface."),
    ("https://github.com/pythoncrazy/beeper-rs", "GitHub", "pythoncrazy/beeper-rs", "Ultra-fast, cross-platform, Rust-native drop-in replacement for Beeper Desktop with a compatible Desktop API server."),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    bridges: Vec<BridgeAccountInfo>,
    chats: Vec<Chat>,
    messages: Vec<Message>,
}

const SCHEMA_VERSION: i64 = 2;
const SCHEMA: &str = "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
     CREATE TABLE chats (id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;
     CREATE TABLE messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL) STRICT;
     CREATE TABLE assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL) STRICT;";
const KEY_STATE: &str = "state";
const KEY_BRIDGES: &str = "bridges";
const KEY_SETTINGS: &str = "settings";
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

pub fn sort_key(seq: i64) -> String {
    const SPAN: i64 = 1_000_000_000_000;
    assert!(seq.abs() < SPAN, "sort key {seq} does not fit in 12 digits");
    if seq < 0 {
        format!("-{:012}", SPAN + seq)
    } else {
        format!("{seq:012}")
    }
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

pub struct BeeperStore {
    pub config: CentralConfig,
    pub data_dir: PathBuf,
    pub setup: Option<Setup>,
    db: Mutex<Connection>,
    pub beeper_state: BeeperStateRecord,
    pub setup_status: SetupStatusResponse,
    pub verifications: HashMap<String, VerificationState>,
    pub bridge_accounts: Vec<BridgeAccountInfo>,
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
    pub oauth_clients: HashMap<String, OAuthClientEntry>,
    pub oauth_codes: HashMap<String, OAuthCodeEntry>,
    pub valid_tokens: BTreeSet<String>,
    pub settings: BeeperSettings,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub focus_tx: broadcast::Sender<Option<String>>,
    pub link_previews: HashMap<String, LinkPreview>,
    pub matrix: Option<matrix_sdk::Client>,
    pub matrix_error: Option<String>,
    pub sas_confirm: Option<tokio::sync::oneshot::Sender<bool>>,
    next_seq: i64,
}

impl BeeperStore {
    pub fn from_config(config: CentralConfig) -> Self {
        assert_not_original_beeper_dir(&config.data_dir);
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
            1 => conn
                .execute_batch(
                    "BEGIN;
                     UPDATE assets SET json = json_set(json_remove(json, '$.local_file_path'), '$.disk_name',
                         substr(json_extract(json, '$.local_file_path'), instr(json_extract(json, '$.local_file_path'), upload_id || '_')));
                     PRAGMA user_version = 2;
                     COMMIT;",
                )
                .expect("migrate beeper-rs schema 1 to 2"),
            SCHEMA_VERSION => {}
            other => panic!(
                "{} has schema version {other}, and this beeper-rs build reads version {SCHEMA_VERSION}",
                config.db_path.display()
            ),
        }
        let beeper_state = if config.demo { demo_state() } else { BeeperStateRecord::default() };
        let label = |id: &str, name: &str| Label { id: id.to_string(), name: name.to_string() };
        let mut store = Self {
            data_dir: config.data_dir.clone(),
            setup: None,
            settings: config.settings.clone(),
            config,
            db: Mutex::new(conn),
            setup_status: Self::build_setup_from_state(&beeper_state),
            valid_tokens: BTreeSet::new(),
            beeper_state,
            verifications: HashMap::new(),
            bridge_accounts: Vec::new(),
            bridge_sessions: HashMap::new(),
            accounts: Vec::new(),
            labels: vec![label("lbl_favorite", "Favorites"), label("lbl_work", "Work"), label("lbl_vip", "VIP")],
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
                .map(|&(url, site, title, summary)| {
                    (url.to_string(), LinkPreview { site: site.into(), title: title.into(), summary: summary.into() })
                })
                .collect(),
            oauth_clients: HashMap::new(),
            oauth_codes: HashMap::new(),
            ws_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            focus_tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
            matrix: None,
            matrix_error: None,
            sas_confirm: None,
            next_seq: 1000,
        };
        store.load_from_sqlite_or_seed();
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
            "INSERT OR REPLACE INTO chats (id, json) VALUES (?1, ?2)",
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
        let (setup, state, bridges, settings, seq, chats, messages, assets) = {
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
                kv(KEY_SEQ),
                rows("SELECT json FROM chats"),
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
        if self.signed_in() {
            self.valid_tokens.insert(self.beeper_state.access_token.clone());
        }
        self.setup_status = Self::build_setup_from_state(&self.beeper_state);
        if let Some(b) = bridges {
            self.bridge_accounts = serde_json::from_str(&b).expect("parse stored bridge accounts");
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
        self.next_seq = match seq {
            Some(s) => s.parse().expect("parse stored nextSeq"),
            None => {
                assert!(fresh, "stored chats without a stored nextSeq");
                1000 + self.messages.len() as i64 + self.chats.len() as i64
            }
        };
        self.persist_seq();
        if self.signed_in() {
            self.ensure_note_chat(&Profile::default());
        }
        self.rebuild_accounts_from_bridges();
    }

    fn put_kv(&self, key: &str, value: &str) {
        self.exec("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", params![key, value]);
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
            self.next_seq = self.next_seq.max(m.sort_key.parse().expect("sort keys are numbers"));
            self.push_message(m, false);
        }
        (chats..self.chats.len()).for_each(|i| self.persist_chat(i));
        (messages..self.messages.len()).for_each(|i| self.persist_message(i));
        self.persist_seq();
        self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit import transaction");
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
        self.valid_tokens.insert(self.beeper_state.access_token.clone());
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
        };
        let chat = Chat {
            id: NOTE_CHAT_ID.into(),
            account_id: "hungryserv".into(),
            participants: ChatParticipants { has_more: false, items: vec![me], total: 1 },
            title: "Note to self".into(),
            chat_type: "single".into(),
            unread_count: 0,
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
            is_hidden_from_inbox: None,
            is_request: None,
            network_name: None,
            img_url: None,
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
        self.persist_state_to_sqlite();
        self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit seed transaction");
    }

    pub fn me(&self) -> &User {
        &self.chats[self.chat_idx_by_id[NOTE_CHAT_ID]].participants.items[0]
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
        let idx = self.chats.len();
        assert!(self.chat_idx_by_id.insert(chat.id.clone(), idx).is_none(), "duplicate chat {}", chat.id);
        self.chat_search_index.insert_doc(idx as u32, &chat_index_text(&chat));
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
        let idx = self.messages.len();
        assert!(self.msg_idx_by_id.insert(msg.id.clone(), idx).is_none(), "duplicate message {}", msg.id);
        self.msg_search_index.insert_doc(idx as u32, &message_index_text(&msg));
        let list = self.chat_msg_indices.entry(msg.chat_id.clone()).or_default();
        let pos = list.partition_point(|&i| self.messages[i].sort_key <= msg.sort_key);
        list.insert(pos, idx);
        let c_idx = self.chat_idx_by_id[&msg.chat_id];
        let chat = &mut self.chats[c_idx];
        if chat.preview.as_ref().is_none_or(|p| p.sort_key <= msg.sort_key) {
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

    pub fn upsert_chat(&mut self, chat: Chat) {
        let Some(&idx) = self.chat_idx_by_id.get(&chat.id) else {
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
                reminder: c.reminder.take(),
                chat_theme: c.chat_theme.take(),
                ..chat
            }
        });
    }

    pub fn upsert_message(&mut self, msg: Message) {
        let Some(&idx) = self.msg_idx_by_id.get(&msg.id) else {
            let idx = self.push_message(msg, true);
            let m = self.messages[idx].clone();
            return self.emit_ws("message.upserted", &m.chat_id, Some(&m.id), None, Some(m.clone()));
        };
        self.update_message(idx, |m| {
            *m = Message {
                reactions: std::mem::take(&mut m.reactions),
                edit_history: std::mem::take(&mut m.edit_history),
                is_bookmarked: m.is_bookmarked,
                ..msg
            }
        });
    }

    pub fn update_chat(&mut self, idx: usize, f: impl FnOnce(&mut Chat)) -> Chat {
        let old = chat_index_text(&self.chats[idx]);
        f(&mut self.chats[idx]);
        self.chats[idx].sync_flags();
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
        format!("{prefix}_{}", self.next_seq)
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
            account_id: account_id.to_string(),
            title,
            chat_type: chat_type.to_string(),
            unread_count: 0,
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
            is_hidden_from_inbox: None,
            is_request: None,
            network_name: None,
            img_url: None,
            participants: ChatParticipants { has_more: false, total: items.len(), items },
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
        let idx = self.resolve_chat_idx(chat_or_local_id).ok_or("chat_not_found")?;
        let has_content = |d: &ChatDraft| d.text.as_deref().is_some_and(|s| !s.is_empty()) || !d.attachments.is_empty();
        let draft = match new_draft {
            _ if clear_draft => None,
            Some(d) if has_content(&d) && self.chats[idx].draft.as_ref().is_some_and(has_content) => {
                return Err("existing_draft_not_empty")
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
        let c_idx = self.send_target(self.resolve_chat_idx(chat_or_local_id).ok_or("chat_not_found")?);
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
            text,
            message_type: Some(msg_type.to_string()),
            is_sender: Some(true),
            is_unread: Some(false),
            is_bookmarked: Some(false),
            linked_message_id: reply_to_message_id,
            attachments: attachment.into_iter().collect(),
            reactions: Vec::new(),
            poll: None,
            edit_history: Vec::new(),
            links: Vec::new(),
            flags: 0,
        };
        let m_idx = self.push_message(msg, true);
        let msg = self.messages[m_idx].clone();
        self.emit_ws("message.upserted", &chat_id, Some(&msg_id), None, Some(msg.clone()));
        self.emit_ws("chat.upserted", &chat_id, None, Some(self.chats[c_idx].clone()), None);
        Ok((chat_id, msg_id, msg))
    }

    fn message_in_chat(&self, chat_or_local_id: &str, message_id: &str) -> Result<(String, usize), &'static str> {
        let chat_id = self.resolve_canonical_chat_id(chat_or_local_id).ok_or("chat_not_found")?;
        let &m_idx = self.msg_idx_by_id.get(message_id).ok_or("message_not_found")?;
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
            let prev = m.text.replace(new_text).expect("edited message has text");
            m.edit_history.push(EditHistoryEntry { text: prev, timestamp: now_rfc3339_millis() });
        });
        Ok((chat_id, msg))
    }

    pub fn delete_message(&mut self, chat_or_local_id: &str, message_id: &str) -> Result<String, &'static str> {
        let (chat_id, m_idx) = self.message_in_chat(chat_or_local_id, message_id)?;
        self.msg_search_index.remove_doc(m_idx as u32, &message_index_text(&self.messages[m_idx]));
        self.messages[m_idx].flags |= MSG_DELETED;
        self.messages[m_idx].text = None;
        self.msg_idx_by_id.remove(message_id);
        self.chat_msg_indices.get_mut(&chat_id).expect("chat has messages").retain(|&i| i != m_idx);
        self.exec("DELETE FROM messages WHERE id = ?1", params![message_id]);
        self.emit_ws("message.deleted", &chat_id, Some(message_id), None, None);
        Ok(chat_id)
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
        let rx_id = self.next_id("rx");
        let pid = format!("@self_{}:beeper.com", self.messages[m_idx].account_id);
        self.update_message(m_idx, |m| {
            m.reactions.retain(|r| !(r.participant_id == pid && r.reaction_key == reaction_key));
            m.reactions.push(Reaction {
                id: rx_id,
                participant_id: pid.clone(),
                reaction_key: reaction_key.to_string(),
                animated: Some(false),
                img_url: None,
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
        self.update_message(m_idx, |m| m.reactions.retain(|r| r.reaction_key != reaction_key));
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

    pub fn vote_poll(&mut self, message_id: &str, option_id: &str, voter_id: &str) {
        let m_idx = self.msg_idx_by_id[message_id];
        self.update_message(m_idx, |m| {
            let poll = m.poll.as_mut().expect("message has poll");
            assert!(poll.options.iter().any(|o| o.id == option_id), "poll option {option_id} exists");
            for opt in &mut poll.options {
                let voted = opt.voter_ids.iter().any(|v| v == voter_id);
                if opt.id == option_id && !voted {
                    opt.voter_ids.push(voter_id.to_string());
                } else if opt.id == option_id || !poll.allow_multiple {
                    opt.voter_ids.retain(|v| v != voter_id);
                }
            }
        });
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
        let cutoff = (now_unix_secs() - days * 86_400) * 1000;
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

    pub fn total_unread_chats(&self) -> u32 {
        self.chats.iter().filter(|c| c.flags & CHAT_ARCHIVED == 0 && c.flags & CHAT_UNREAD != 0).count() as u32
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

    pub fn clear_cached_media_older_than_days(&mut self, days: i64) -> usize {
        let cutoff = now_unix_secs() - days * 86_400;
        let old: Vec<String> =
            self.assets.values().filter(|a| a.created_epoch_secs <= cutoff).map(|a| a.upload_id.clone()).collect();
        for id in &old {
            let a = self.assets.remove(id).unwrap();
            self.assets_by_mxc.remove(&a.mxc_url);
            std::fs::remove_file(self.config.assets_dir.join(&a.disk_name)).expect("remove cached media file");
            self.exec("DELETE FROM assets WHERE upload_id = ?1", params![id]);
        }
        old.len()
    }
}

use crate::config::{
    assert_not_original_beeper_dir, copy_beeper_data_to_isolated_dir, now_rfc3339_millis,
    now_unix_secs, CentralConfig, DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE,
    DEFAULT_TOKEN, DEFAULT_USER_ID, SEEDED_BRIDGES, WS_CHANNEL_CAPACITY,
};
use crate::models::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::broadcast;

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.sort();
    out.dedup();
    out
}

#[inline]
pub fn intersect_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut res = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            res.push(a[i]);
            i += 1;
            j += 1;
        } else if a[i] < b[j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    res
}

#[inline]
pub fn union_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut res = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            res.push(a[i]);
            i += 1;
            j += 1;
        } else if a[i] < b[j] {
            res.push(a[i]);
            i += 1;
        } else {
            res.push(b[j]);
            j += 1;
        }
    }
    res.extend_from_slice(&a[i..]);
    res.extend_from_slice(&b[j..]);
    res
}

#[derive(Debug, Default, Clone)]
pub struct InvertedIndex {
    postings: HashMap<String, Vec<u32>>,
}

impl InvertedIndex {
    pub fn insert_doc(&mut self, doc_id: u32, text: &str) {
        for tok in tokenize(text) {
            let list = self.postings.entry(tok).or_default();
            if let Err(pos) = list.binary_search(&doc_id) {
                list.insert(pos, doc_id);
            }
        }
    }

    pub fn remove_doc(&mut self, doc_id: u32, text: &str) {
        for tok in tokenize(text) {
            if let Some(list) = self.postings.get_mut(&tok) {
                if let Ok(pos) = list.binary_search(&doc_id) {
                    list.remove(pos);
                }
            }
        }
    }

    pub fn query_all_tokens(&self, query: &str) -> Vec<u32> {
        let q_tokens = tokenize(query);
        if q_tokens.is_empty() {
            return Vec::new();
        }
        let mut current: Option<Vec<u32>> = None;
        for q in &q_tokens {
            let mut term_vec = Vec::new();
            for (term, ids) in &self.postings {
                if term.starts_with(q) {
                    term_vec = if term_vec.is_empty() {
                        ids.clone()
                    } else {
                        union_sorted(&term_vec, ids)
                    };
                }
            }
            if term_vec.is_empty() {
                return Vec::new();
            }
            current = Some(match current {
                None => term_vec,
                Some(prev) => intersect_sorted(&prev, &term_vec),
            });
            if current.as_ref().unwrap().is_empty() {
                return Vec::new();
            }
        }
        current.unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAsset {
    pub upload_id: String,
    pub mxc_url: String,
    pub local_file_path: String,
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

const NOTE_MESSAGES: [(&str, &str); 6] = [
    ("2026-10-06T19:02:00.000Z", "https://arxiv.org/abs/2001.08361"),
    ("2026-10-06T19:04:00.000Z", "Read this before the Thursday sync"),
    ("2026-10-11T10:41:00.000Z", "https://github.com/pythoncrazy/navify"),
    ("2026-10-12T21:15:00.000Z", "Pick up the Pebble Time 2 dev kit from the office"),
    ("2026-10-14T03:30:00.000Z", "https://github.com/pythoncrazy/beeper-rs"),
    ("2026-10-14T03:34:00.000Z", "Compare the inbox density with the compact chat list"),
];

type SeedChat = (&'static str, &'static str, &'static str, &'static str, u32, &'static str, &'static str, &'static str);

const EXTRA_CHATS: [SeedChat; 8] = [
    ("!wa_family:beeper.com", "acc_whatsapp", "Family", "group", 2, "2026-10-14T03:10:00.000Z", "Mom", "Dinner at 7 tonight? Bring the charger you borrowed"),
    ("!tg_weekly:beeper.com", "acc_telegram", "This Week in Rust", "group", 0, "2026-10-13T21:00:00.000Z", "Nell", "Issue 620 is out: iced 0.14, wgpu 27, and a new borrow checker"),
    ("!im_jordan:beeper.com", "acc_imessage", "Jordan Park", "single", 0, "2026-10-13T18:22:00.000Z", "Jordan", ""),
    ("!sg_lena:beeper.com", "acc_signal", "Lena Fischer", "single", 1, "2026-10-12T16:40:00.000Z", "Lena", "See you at the meetup! I will save you a seat"),
    ("!ig_kai:beeper.com", "acc_instagram", "Kai Nakamura", "single", 0, "2026-10-11T12:05:00.000Z", "Kai", "haha that reel is exactly us at the hackathon"),
    ("!sl_design:beeper.com", "acc_slack", "#design-review", "group", 0, "2026-10-10T09:30:00.000Z", "Priya", "New icons are in Figma, please review before Friday"),
    ("!wa_sam:beeper.com", "acc_whatsapp", "Sam Rivera", "single", 0, "2026-10-08T22:14:00.000Z", "Sam", "ok sounds good 👍"),
    ("!tg_pebble:beeper.com", "acc_telegram", "Pebble Devs", "group", 0, "2026-10-05T15:00:00.000Z", "Katharine", "Firmware 4.4 beta is live for all testers"),
];

const LINK_PREVIEWS: [(&str, &str, &str, &str); 3] = [
    ("https://arxiv.org/abs/2001.08361", "arXiv.org", "Scaling Laws for Neural Language Models", "We study empirical scaling laws for language model performance on the cross-entropy loss. The loss scales as a power-law with model size, dataset size, and the amount of compute used for training."),
    ("https://github.com/pythoncrazy/navify", "GitHub", "pythoncrazy/navify", "A fast, native Navidrome client written in Rust with iced. Low memory use, instant startup, and a polished desktop interface."),
    ("https://github.com/pythoncrazy/beeper-rs", "GitHub", "pythoncrazy/beeper-rs", "Ultra-fast, cross-platform, Rust-native drop-in replacement for Beeper Desktop with a compatible Desktop API server."),
];

#[derive(Debug, Clone)]
pub struct SettingCatalogEntry {
    pub id: &'static str,
    pub section: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

const fn sc(id: &'static str, section: &'static str, title: &'static str, description: &'static str) -> SettingCatalogEntry {
    SettingCatalogEntry { id, section, title, description }
}

pub const SETTINGS_CATALOG: [SettingCatalogEntry; 24] = [
    sc("show_archived_count", "Chat List", "Show archived count", "Display total archived chats badge in the sidebar header"),
    sc("show_starred_pin_view", "Chat List", "Show starred pin view", "Pin tab for starred and favorite conversations"),
    sc("show_all_chats_pin_view", "Chat List", "Show all chats pin view", "Pin tab showing all unified inbox conversations"),
    sc("show_unread_pin_view", "Chat List", "Show unread pin view", "Quick filter pin tab for unread conversations"),
    sc("show_drafts_pin_view", "Chat List", "Show drafts pin view", "Quick filter pin tab for conversations with unsent drafts"),
    sc("filter_pins_by_network", "Chat List", "Filter pins by network", "Scope pinned avatars to the currently selected chat network"),
    sc("show_favorite_chats_section", "Chat List", "Show favorite chats section", "Display a dedicated Favorites group above the main chat list"),
    sc("keep_favorites_in_inbox", "Chat List", "Keep favorites in inbox", "Keep favorited conversations visible inside the main inbox list"),
    sc("show_message_previews", "Chat List", "Show message previews", "Show latest message snippet below each chat title in the sidebar"),
    sc("unread_chats_always_on_top", "Chat List", "Unread chats always on top", "Sort unread threads above read threads in the chat list"),
    sc("show_full_names_in_chats", "Conversation View", "Show full names in chats", "Display full sender names instead of first names in group chats"),
    sc("auto_play_videos", "Conversation View", "Auto-play videos", "Automatically play inline video attachments and GIFs"),
    sc("show_chat_header", "Conversation View", "Show chat header", "Show the top header bar with participant details in conversation view"),
    sc("show_message_input_buttons", "Conversation View", "Show message input buttons", "Show attachment, emoji, voice note, poll, and schedule buttons in composer"),
    sc("link_previews", "Conversation View", "Link previews", "Generate rich previews for URLs shared in messages"),
    sc("show_right_panel_by_default", "Conversation View", "Show right panel by default", "Keep the Chat Info, Shared Media, and Bookmarks inspector open"),
    sc("load_remote_content_automatically", "Conversation View", "Load remote content automatically", "Fetch external images and link metadata automatically"),
    sc("use_twenty_four_hour_time", "Conversation View", "Use 24-hour time", "Format message timestamps using 24-hour military clock"),
    sc("theme", "Appearance", "CSS Theme", "Switch between Light, Dark, Dracula, Nocturne, Dark Purple, and Custom CSS"),
    sc("no_transparency", "Appearance", "No transparency", "Disable window vibrancy and translucent backgrounds (.no-transparency)"),
    sc("reduce_transparency", "Appearance", "Reduce transparency", "Use solid header and sidebar surfaces (.reduce-transparency)"),
    sc("fix_lockdown_emojis", "Appearance", "Fix Lockdown Mode Emojis", "Apply fix-lockdown-mode-emojis.css bitmap font override"),
    sc("auto_transcribe_voice_notes", "Voice & Media", "Transcribe voice notes", "Automatically transcribe incoming and outgoing voice notes on-device"),
    sc("repeat_message_alerts", "Notifications", "Repeat message alerts", "Repeat unread notification alerts up to 3 times"),
];

pub struct BeeperStore {
    pub config: CentralConfig,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub assets_dir: PathBuf,
    pub copied_files_from_source: usize,
    pub db: Mutex<Connection>,
    pub beeper_state: BeeperStateRecord,
    pub setup_status: SetupStatusResponse,
    pub verifications: HashMap<String, VerificationState>,
    pub bridge_accounts: Vec<BridgeAccountInfo>,
    pub bridge_sessions: HashMap<String, BridgeLoginSession>,
    pub accounts: Vec<Account>,
    pub labels: Vec<Label>,
    pub chats: Vec<Chat>,
    pub chat_idx_by_id: HashMap<String, usize>,
    pub local_chat_idx_by_id: HashMap<String, usize>,
    pub messages: Vec<Message>,
    pub msg_idx_by_id: HashMap<String, usize>,
    pub chat_msg_indices: HashMap<String, Vec<usize>>,
    pub msg_search_index: InvertedIndex,
    pub chat_search_index: InvertedIndex,
    pub settings_search_index: InvertedIndex,
    pub assets: HashMap<String, StoredAsset>,
    pub assets_by_mxc: HashMap<String, String>,
    pub oauth_clients: HashMap<String, OAuthClientEntry>,
    pub oauth_codes: HashMap<String, OAuthCodeEntry>,
    pub valid_tokens: BTreeSet<String>,
    pub settings: BeeperSettings,
    pub focused_chat_id: Option<String>,
    pub focused_message_id: Option<String>,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub link_previews: HashMap<String, LinkPreview>,
    next_seq: u64,
}

impl BeeperStore {
    pub fn open_or_create(data_dir: PathBuf, pas_port: u16) -> Self {
        let config = CentralConfig::new(data_dir, "127.0.0.1".to_string(), pas_port, None);
        Self::from_config(config)
    }

    pub fn from_config(config: CentralConfig) -> Self {
        assert_not_original_beeper_dir(&config.data_dir);
        std::fs::create_dir_all(&config.data_dir).expect("create isolated BeeperRs data_dir");
        let mut copied_files_from_source = 0;
        if !config.db_path.exists() {
            if let Some(ref src_dir) = config.source_beeper_dir {
                copied_files_from_source =
                    copy_beeper_data_to_isolated_dir(src_dir, &config.data_dir);
            }
        }
        std::fs::create_dir_all(&config.assets_dir).expect("create isolated media dir");
        let (ws_tx, _) = broadcast::channel(WS_CHANNEL_CAPACITY);

        let conn = Connection::open(&config.db_path).expect("open isolated index.db");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             PRAGMA mmap_size=268435456;
             PRAGMA cache_size=-16000;
             CREATE TABLE IF NOT EXISTS key_values (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS beeper_rs_chats (id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS beeper_rs_messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS beeper_rs_assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL);",
        )
        .expect("init SQLite WAL and schema");

        let mut settings_search_index = InvertedIndex::default();
        for (i, entry) in SETTINGS_CATALOG.iter().enumerate() {
            settings_search_index.insert_doc(
                i as u32,
                &format!("{} {} {}", entry.section, entry.title, entry.description),
            );
        }

        let default_state = BeeperStateRecord {
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
        };

        let setup_status = Self::build_setup_from_state(&default_state, None);
        let mut valid_tokens = BTreeSet::new();
        valid_tokens.insert(default_state.access_token.clone());

        let mut store = Self {
            data_dir: config.data_dir.clone(),
            db_path: config.db_path.clone(),
            assets_dir: config.assets_dir.clone(),
            copied_files_from_source,
            settings: config.settings.clone(),
            config,
            db: Mutex::new(conn),
            beeper_state: default_state,
            setup_status,
            verifications: HashMap::new(),
            bridge_accounts: Vec::new(),
            bridge_sessions: HashMap::new(),
            accounts: Vec::new(),
            labels: vec![
                Label {
                    id: "lbl_favorite".to_string(),
                    name: "Favorites".to_string(),
                },
                Label {
                    id: "lbl_work".to_string(),
                    name: "Work".to_string(),
                },
                Label {
                    id: "lbl_vip".to_string(),
                    name: "VIP".to_string(),
                },
            ],
            chats: Vec::new(),
            chat_idx_by_id: HashMap::new(),
            local_chat_idx_by_id: HashMap::new(),
            messages: Vec::new(),
            msg_idx_by_id: HashMap::new(),
            chat_msg_indices: HashMap::new(),
            msg_search_index: InvertedIndex::default(),
            chat_search_index: InvertedIndex::default(),
            settings_search_index,
            assets: HashMap::new(),
            assets_by_mxc: HashMap::new(),
            link_previews: HashMap::new(),
            oauth_clients: HashMap::new(),
            oauth_codes: HashMap::new(),
            valid_tokens,
            focused_chat_id: None,
            focused_message_id: None,
            ws_tx,
            next_seq: 1000,
        };

        store.load_from_sqlite_or_seed();
        store
    }

    pub fn build_setup_from_state(
        state: &BeeperStateRecord,
        flow: Option<SetupFlowStep>,
    ) -> SetupStatusResponse {
        let mut secrets = HashMap::new();
        secrets.insert(
            "masterKey".to_string(),
            !state.secrets.master_key.is_empty(),
        );
        secrets.insert(
            "selfSigningKey".to_string(),
            !state.secrets.self_signing_key.is_empty(),
        );
        secrets.insert(
            "userSigningKey".to_string(),
            !state.secrets.user_signing_key.is_empty(),
        );
        secrets.insert(
            "megolmBackupKey".to_string(),
            !state.secrets.megolm_backup_key.is_empty(),
        );
        secrets.insert(
            "recoveryCode".to_string(),
            !state.secrets.recovery_code.is_empty(),
        );

        let session_state = if state.access_token.is_empty() {
            "signed_out"
        } else if !state.verified {
            "needs_verification"
        } else {
            "ready"
        };
        let e2ee_state = if !state.initialized {
            "idle"
        } else if !state.verified {
            "needs_verification"
        } else {
            "ready"
        };

        SetupStatusResponse {
            session: SetupSession {
                state: session_state.to_string(),
                e2ee: SetupE2ee {
                    state: e2ee_state.to_string(),
                    cross_signing: state.cross_signing,
                    first_sync_done: state.first_sync_done,
                    has_backed_up_code: state.has_backed_up_code,
                    initialized: state.initialized,
                    key_backup: state.key_backup,
                    secret_storage: state.secret_storage,
                    verified: state.verified,
                    verification_methods: vec![
                        "recovery_key".to_string(),
                        "other_device".to_string(),
                        "qr_code".to_string(),
                        "sas".to_string(),
                    ],
                    secrets,
                },
                matrix: if state.access_token.is_empty() {
                    None
                } else {
                    Some(SetupMatrix {
                        homeserver: state.homeserver.clone(),
                        user_id: state.user_id.clone(),
                        device_id: Some(state.device_id.clone()),
                    })
                },
                verification: None,
            },
            setup_flow: flow,
        }
    }

    pub fn persist_state_to_sqlite(&self) {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare_cached("INSERT OR REPLACE INTO key_values (key, value) VALUES (?1, ?2)")
            .expect("prepare key_values statement");
        for (k, v) in [
            (
                "beeperState",
                serde_json::to_string(&self.beeper_state).unwrap(),
            ),
            (
                "bridgeAccounts",
                serde_json::to_string(&self.bridge_accounts).unwrap(),
            ),
            (
                "beeperSettings",
                serde_json::to_string(&self.settings).unwrap(),
            ),
        ] {
            stmt.execute(params![k, v]).expect("write key_values");
        }
    }

    fn persist_chat_row(&self, chat: &Chat) {
        let conn = self.db.lock().unwrap();
        conn.prepare_cached("INSERT OR REPLACE INTO beeper_rs_chats (id, json) VALUES (?1, ?2)")
            .unwrap()
            .execute(params![chat.id, serde_json::to_string(chat).unwrap()])
            .expect("persist chat row");
    }

    fn persist_message_row(&self, msg: &Message) {
        let conn = self.db.lock().unwrap();
        conn.prepare_cached(
            "INSERT OR REPLACE INTO beeper_rs_messages (id, chat_id, sort_key, json) VALUES (?1, ?2, ?3, ?4)",
        )
        .unwrap()
        .execute(params![
            msg.id,
            msg.chat_id,
            msg.sort_key,
            serde_json::to_string(msg).unwrap()
        ])
        .expect("persist message row");
    }

    fn delete_message_row(&self, msg_id: &str) {
        let conn = self.db.lock().unwrap();
        conn.prepare_cached("DELETE FROM beeper_rs_messages WHERE id = ?1")
            .unwrap()
            .execute(params![msg_id])
            .expect("delete message row");
    }

    fn load_from_sqlite_or_seed(&mut self) {
        let (raw_state, raw_bridges, raw_settings, loaded_chats, loaded_msgs, loaded_assets) = {
            let conn = self.db.lock().unwrap();
            let raw_state = conn
                .query_row::<String, _, _>(
                    "SELECT value FROM key_values WHERE key = 'beeperState'",
                    [],
                    |r| r.get(0),
                )
                .ok();
            let raw_bridges = conn
                .query_row::<String, _, _>(
                    "SELECT value FROM key_values WHERE key = 'bridgeAccounts'",
                    [],
                    |r| r.get(0),
                )
                .ok();
            let raw_settings = conn
                .query_row::<String, _, _>(
                    "SELECT value FROM key_values WHERE key = 'beeperSettings'",
                    [],
                    |r| r.get(0),
                )
                .ok();
            let loaded_chats: Vec<Chat> = conn
                .prepare("SELECT json FROM beeper_rs_chats")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| serde_json::from_str(&r.ok()?).ok())
                .collect();
            let loaded_msgs: Vec<Message> = if loaded_chats.is_empty() {
                Vec::new()
            } else {
                conn.prepare("SELECT json FROM beeper_rs_messages ORDER BY sort_key ASC")
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .filter_map(|r| serde_json::from_str(&r.ok()?).ok())
                    .collect()
            };
            let loaded_assets: Vec<StoredAsset> = conn
                .prepare("SELECT json FROM beeper_rs_assets")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| serde_json::from_str(&r.ok()?).ok())
                .collect();
            (
                raw_state,
                raw_bridges,
                raw_settings,
                loaded_chats,
                loaded_msgs,
                loaded_assets,
            )
        };

        if let Some(s) = raw_state {
            self.beeper_state = BeeperStateRecord::from_json_str(&s);
            self.valid_tokens
                .insert(self.beeper_state.access_token.clone());
            self.setup_status = Self::build_setup_from_state(&self.beeper_state, None);
        }
        if let Some(b) = raw_bridges {
            self.bridge_accounts = BridgeAccountInfo::vec_from_json_str(&b);
        }
        if let Some(st) = raw_settings {
            if let Ok(parsed) = serde_json::from_str::<BeeperSettings>(&st) {
                let pas_port = self.settings.pas_port;
                self.settings = parsed;
                self.settings.pas_port = pas_port;
            }
        }
        for mut asset in loaded_assets {
            if let Some(fname) = std::path::Path::new(&asset.local_file_path).file_name() {
                let isolated_media_path = self.assets_dir.join(fname);
                if isolated_media_path.exists() {
                    asset.local_file_path = isolated_media_path.to_string_lossy().to_string();
                }
            }
            self.assets_by_mxc
                .insert(asset.mxc_url.clone(), asset.upload_id.clone());
            self.assets.insert(asset.upload_id.clone(), asset);
        }

        for c in loaded_chats {
            self.index_and_push_chat(c, false);
        }
        for m in loaded_msgs {
            self.index_and_push_message(m, false);
        }
        if !self.chat_idx_by_id.contains_key("!ig_design:beeper.com")
            || !self.msg_idx_by_id.contains_key("msg_1000b")
            || !self.chat_idx_by_id.contains_key(NOTE_CHAT_ID)
        {
            self.seed_default_workspace();
        } else {
            self.rebuild_accounts_from_bridges();
        }
        self.link_previews = LINK_PREVIEWS
            .iter()
            .map(|(url, site, title, summary)| (url.to_string(), LinkPreview { site: site.to_string(), title: title.to_string(), summary: summary.to_string() }))
            .collect();
        self.next_seq = 1000 + self.messages.len() as u64 + self.chats.len() as u64;
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
                bridge_id: b.bridge_id.clone(),
                mode: Some(if b.mode.as_deref() == Some("on_device") {
                    BridgeMode::OnDevice
                } else {
                    BridgeMode::Cloud
                }),
                user: User {
                    id: format!("@self_{}:beeper.com", b.bridge_id),
                    full_name: Some(format!("Alex ({})", b.network_name)),
                    username: Some(format!("alex_{}", b.bridge_id)),
                    email: Some("alex@beeper.com".to_string()),
                    phone_number: Some("+14155550199".to_string()),
                    img_url: None,
                    is_self: Some(true),
                    cannot_message: Some(false),
                    custom_nickname: None,
                },
            })
            .collect();
    }

    fn seed_default_workspace(&mut self) {
        for (bid, btype, name, mode) in SEEDED_BRIDGES {
            if !self.bridge_accounts.iter().any(|b| b.bridge_id == bid) {
                self.bridge_accounts.push(BridgeAccountInfo {
                    account_id: format!("acc_{bid}"),
                    bridge_id: bid.to_string(),
                    bridge_type: btype.to_string(),
                    is_running: true,
                    login_id: Some(format!("login_{bid}_01")),
                    mode: Some(
                        if mode == BridgeMode::OnDevice {
                            "on_device"
                        } else {
                            "cloud"
                        }
                        .to_string(),
                    ),
                    network_name: name.to_string(),
                    state: "CONNECTED".to_string(),
                });
            }
        }
        self.rebuild_accounts_from_bridges();
        let self_user = |accounts: &[Account], idx: usize| -> User {
            accounts
                .get(idx)
                .or_else(|| accounts.first())
                .map(|a| a.user.clone())
                .unwrap_or_else(|| User {
                    id: "@self:beeper.com".to_string(),
                    full_name: Some("Alex".to_string()),
                    username: Some("alex".to_string()),
                    email: Some("alex@beeper.com".to_string()),
                    phone_number: None,
                    img_url: None,
                    is_self: Some(true),
                    cannot_message: Some(false),
                    custom_nickname: None,
                })
        };

        let make_user = |id: &str, name: &str, uname: &str, phone: Option<&str>| User {
            id: id.to_string(),
            full_name: Some(name.to_string()),
            username: Some(uname.to_string()),
            email: Some(format!("{uname}@example.com")),
            phone_number: phone.map(str::to_string),
            img_url: None,
            is_self: Some(false),
            cannot_message: Some(false),
            custom_nickname: None,
        };

        let mk_chat = |id: &str, loc: &str, acc: &str, title: &str, ctype: &str, unread: u32, desc: &str, muted: bool, pinned: bool, labels: &[&str], ts: &str, sk: &str, users: Vec<User>| Chat {
            id: id.to_string(),
            local_chat_id: Some(loc.to_string()),
            account_id: acc.to_string(),
            title: title.to_string(),
            chat_type: ctype.to_string(),
            unread_count: unread,
            description: Some(desc.to_string()),
            draft: None,
            is_archived: Some(false),
            is_marked_unread: Some(false),
            is_muted: Some(muted),
            is_pinned: Some(pinned),
            is_low_priority: Some(false),
            label_ids: labels.iter().map(|s| s.to_string()).collect(),
            last_activity: Some(ts.to_string()),
            last_read_message_sort_key: Some(sk.to_string()),
            merge: None,
            merged_into_chat_id: None,
            preview: None,
            reminder: None,
            chat_theme: None,
            participants: ChatParticipants { has_more: false, total: users.len(), items: users },
            flags: 0,
        };

        let me = User { id: DEFAULT_USER_ID.to_string(), is_self: Some(true), ..self_user(&[], 0) };
        let mut chats_seed = vec![
            mk_chat(NOTE_CHAT_ID, "local_note_self", "hungryserv", "Note to self", "single", 0, "Notes, links, and reminders", false, true, &[], "2026-10-14T03:34:00.000Z", "000000", vec![me.clone()]),
            mk_chat("!wa_eric:beeper.com", "local_wa_eric", "acc_whatsapp", "Eric Migicovsky", "single", 1, "Beeper & Pebble founder", false, true, &["lbl_favorite", "lbl_vip"], "2026-10-14T08:15:00.000Z", "000001", vec![make_user("@eric:beeper.com", "Eric Migicovsky", "eric", Some("+14155550101")), self_user(&self.accounts, 0)]),
            mk_chat("!im_sarah:beeper.com", "local_im_sarah", "acc_imessage", "Sarah Chen", "single", 0, "iMessage on-device", true, true, &["lbl_favorite"], "2026-10-14T08:10:00.000Z", "000002", vec![make_user("@sarah:beeper.com", "Sarah Chen", "sarahc", Some("+14155550102")), self_user(&self.accounts, 1)]),
            mk_chat("!tg_rust:beeper.com", "local_tg_rust", "acc_telegram", "Beeper Rust Core Team", "group", 2, "Architecture, E2EE & native desktop performance", false, true, &["lbl_work"], "2026-10-14T08:20:00.000Z", "000003", vec![make_user("@tulir:beeper.com", "Tulir Asokan", "tulir", None), make_user("@batuhan:beeper.com", "Batuhan", "batuhan", None), self_user(&self.accounts, 2)]),
            mk_chat("!sg_security:beeper.com", "local_sg_security", "acc_signal", "Signal Security Ops", "single", 0, "Zero-knowledge E2EE verification channel", false, false, &[], "2026-10-14T07:45:00.000Z", "000004", vec![make_user("@secops:beeper.com", "SecOps Lead", "secops", Some("+14155550190")), self_user(&self.accounts, 3)]),
            mk_chat("!ig_design:beeper.com", "local_ig_design", "acc_instagram", "Maya Lin", "single", 0, "Design Systems & Desktop Typography", false, true, &["lbl_favorite"], "2026-10-14T07:30:00.000Z", "000005", vec![make_user("@maya:beeper.com", "Maya Lin", "mayalin", None), self_user(&self.accounts, 4)]),
            mk_chat("!sl_infra:beeper.com", "local_sl_infra", "acc_slack", "#desktop-release", "group", 1, "Beeper v4.3 Desktop release coordination", false, false, &["lbl_work"], "2026-10-14T07:15:00.000Z", "000006", vec![make_user("@devon:beeper.com", "Devon Brooks", "devon", None), make_user("@eric:beeper.com", "Eric Migicovsky", "eric", None), self_user(&self.accounts, 5)]),
        ];
        for (i, (cid, acc, title, ctype, unread, ts, sender, _)) in EXTRA_CHATS.iter().enumerate() {
            let peer = make_user(&format!("@{}:beeper.com", sender.to_lowercase()), sender, &sender.to_lowercase(), None);
            chats_seed.push(mk_chat(cid, &format!("local_x{i}"), acc, title, ctype, *unread, title, false, false, &[], ts, &format!("00010{i}"), vec![peer, self_user(&self.accounts, 0)]));
        }

        self.db.lock().unwrap().execute_batch("BEGIN IMMEDIATE;").expect("begin seed transaction");
        for c in chats_seed {
            self.index_and_push_chat(c, true);
        }

        let mk_msg = |id: &str, acc: &str, cid: &str, sid: &str, sname: &str, sk: &str, ts: &str, text: &str, mtype: &str, is_self: bool, unread: bool, bmark: bool, reply: Option<&str>, atts: Vec<Attachment>, rxs: Vec<Reaction>, poll: Option<PollData>, edits: Vec<EditHistoryEntry>| Message {
            id: id.to_string(),
            account_id: acc.to_string(),
            chat_id: cid.to_string(),
            sender_id: sid.to_string(),
            sender_name: Some(sname.to_string()),
            sort_key: sk.to_string(),
            timestamp: ts.to_string(),
            text: Some(text.to_string()),
            message_type: Some(mtype.to_string()),
            is_sender: Some(is_self),
            is_unread: Some(unread),
            is_bookmarked: Some(bmark),
            linked_message_id: reply.map(str::to_string),
            attachments: atts,
            reactions: rxs,
            poll,
            edit_history: edits,
            flags: 0,
        };

        let rx = |id: &str, pid: &str, key: &str| Reaction {
            id: id.to_string(),
            participant_id: pid.to_string(),
            reaction_key: key.to_string(),
            animated: Some(false),
            img_url: None,
        };

        let voice_att = |id: &str, fname: &str, dur: f64, tr: &str| Attachment {
            attachment_type: "audio".to_string(),
            id: Some(id.to_string()),
            duration: Some(dur),
            file_name: Some(fname.to_string()),
            file_size: Some(48_200),
            is_gif: Some(false),
            is_sticker: Some(false),
            is_voice_note: Some(true),
            mime_type: Some("audio/ogg".to_string()),
            poster_img: None,
            size: None,
            src_url: Some(format!("mxc://beeper.com/{id}")),
            transcript: Some(tr.to_string()),
        };

        let pdf_att = |id: &str, fname: &str, sz: u64| Attachment {
            attachment_type: "unknown".to_string(),
            id: Some(id.to_string()),
            duration: None,
            file_name: Some(fname.to_string()),
            file_size: Some(sz),
            is_gif: Some(false),
            is_sticker: Some(false),
            is_voice_note: Some(false),
            mime_type: Some("application/pdf".to_string()),
            poster_img: None,
            size: None,
            src_url: Some(format!("mxc://beeper.com/{id}")),
            transcript: None,
        };

        let mut messages_seed: Vec<Message> = NOTE_MESSAGES
            .iter()
            .enumerate()
            .map(|(i, (ts, text))| mk_msg(&format!("msg_note_{i}"), "hungryserv", NOTE_CHAT_ID, DEFAULT_USER_ID, "Alex", &format!("00009{i:02}"), ts, text, "TEXT", true, false, false, None, Vec::new(), Vec::new(), None, Vec::new()))
            .collect();
        for (i, (cid, acc, _, _, unread, ts, sender, text)) in EXTRA_CHATS.iter().enumerate() {
            let img = text.is_empty().then(|| Attachment { attachment_type: "img".to_string(), id: Some(format!("att_x{i}")), duration: None, file_name: Some("IMG_2041.jpg".to_string()), file_size: Some(812_004), is_gif: Some(false), is_sticker: Some(false), is_voice_note: Some(false), mime_type: Some("image/jpeg".to_string()), poster_img: None, size: None, src_url: Some(format!("mxc://beeper.com/x{i}")), transcript: None });
            let mtype = if img.is_some() { "IMAGE" } else { "TEXT" };
            messages_seed.push(mk_msg(&format!("msg_x{i}"), acc, cid, &format!("@{}:beeper.com", sender.to_lowercase()), sender, &format!("00020{i:02}"), ts, text, mtype, false, *unread > 0, false, None, img.into_iter().collect(), Vec::new(), None, Vec::new()));
        }
        messages_seed.extend([
            mk_msg("msg_1000a", "acc_whatsapp", "!wa_eric:beeper.com", "@eric:beeper.com", "Eric Migicovsky", "0000997", "2026-10-14T08:11:00.000Z", "Hey Alex! How is the native Rust Beeper Desktop client coming along?", "TEXT", false, false, false, None, Vec::new(), Vec::new(), None, Vec::new()),
            mk_msg("msg_1000b", "acc_whatsapp", "!wa_eric:beeper.com", "@self_whatsapp:beeper.com", "Alex", "0000998", "2026-10-14T08:12:00.000Z", "Just migrated the UI to Iced with Inter + Lucide vector rendering and full SQLite WAL persistence. Memory usage is under 28 MB!", "TEXT", true, false, false, Some("msg_1000a"), vec![pdf_att("att_wa_spec", "beeper-rs-v4-architecture.pdf", 428_032)], vec![rx("rx_0", "@eric:beeper.com", "❤️"), rx("rx_0b", "@self_whatsapp:beeper.com", "🔥")], None, Vec::new()),
            mk_msg("msg_1000c", "acc_whatsapp", "!wa_eric:beeper.com", "@eric:beeper.com", "Eric Migicovsky", "0000999", "2026-10-14T08:14:00.000Z", "Voice note from hardware lab:", "VOICE", false, false, false, None, vec![voice_att("att_wa_vn", "eric-voice-note.ogg", 14.2, "Native Rust + Iced GPU rendering makes Beeper feel instantaneous across Linux, macOS, and Windows.")], Vec::new(), None, Vec::new()),
            mk_msg("msg_1001", "acc_whatsapp", "!wa_eric:beeper.com", "@eric:beeper.com", "Eric Migicovsky", "0001001", "2026-10-14T08:15:00.000Z", "Welcome to beeper-rs! Check out the local Desktop API on port 23373 and the zero-bloat Rust UI.", "TEXT", false, true, true, Some("msg_1000b"), Vec::new(), vec![rx("rx_1", "@self_whatsapp:beeper.com", "⚡")], None, Vec::new()),
            mk_msg("msg_1002", "acc_imessage", "!im_sarah:beeper.com", "@sarah:beeper.com", "Sarah Chen", "0001002", "2026-10-14T08:10:00.000Z", "Let's ship the release at 14:00 UTC. Check https://beeper.com and ping @eric:beeper.com", "TEXT", false, false, false, None, Vec::new(), Vec::new(), None, vec![EditHistoryEntry { text: "Let's ship the release at 13:00 UTC.".to_string(), timestamp: "2026-10-14T08:09:00.000Z".to_string() }]),
            mk_msg("msg_1003", "acc_telegram", "!tg_rust:beeper.com", "@tulir:beeper.com", "Tulir Asokan", "0001003", "2026-10-14T08:19:00.000Z", "Voice note from standup + architecture PDF attached.", "VOICE", false, true, false, None, vec![voice_att("att_voice_1", "standup-note.ogg", 14.5, "All 16 Mautrix bridges and the SQLite WAL index are running at sub-millisecond latency."), pdf_att("att_pdf_1", "beeper-v4-spec.pdf", 128_500)], Vec::new(), None, Vec::new()),
            mk_msg("msg_1004", "acc_telegram", "!tg_rust:beeper.com", "@batuhan:beeper.com", "Batuhan", "0001004", "2026-10-14T08:20:00.000Z", "Poll: Which default dark theme do you prefer?", "POLL", false, true, false, None, Vec::new(), Vec::new(), Some(PollData {
                question: "Which default dark theme do you prefer?".to_string(),
                allow_multiple: false,
                options: vec![
                    PollOption { id: "opt_1".to_string(), label: "Beeper Dark".to_string(), voter_ids: vec!["@tulir:beeper.com".to_string()] },
                    PollOption { id: "opt_2".to_string(), label: "Dracula".to_string(), voter_ids: vec!["@batuhan:beeper.com".to_string()] },
                    PollOption { id: "opt_3".to_string(), label: "Nocturne".to_string(), voter_ids: Vec::new() },
                    PollOption { id: "opt_4".to_string(), label: "Dark Purple".to_string(), voter_ids: Vec::new() },
                ],
            }), Vec::new()),
            mk_msg("msg_1005", "acc_signal", "!sg_security:beeper.com", "@secops:beeper.com", "SecOps Lead", "0001005", "2026-10-14T07:45:00.000Z", "Cross-signing master key and Megolm backup verified across all connected devices.", "TEXT", false, false, false, None, Vec::new(), Vec::new(), None, Vec::new()),
            mk_msg("msg_1006", "acc_instagram", "!ig_design:beeper.com", "@maya:beeper.com", "Maya Lin", "0001006", "2026-10-14T07:30:00.000Z", "The 17px bubble radius and #2561fb accent match the Beeper design spec.", "TEXT", false, false, false, None, Vec::new(), Vec::new(), None, Vec::new()),
            mk_msg("msg_1007", "acc_slack", "!sl_infra:beeper.com", "@devon:beeper.com", "Devon Brooks", "0001007", "2026-10-14T07:15:00.000Z", "Linux, macOS, and Windows builds passed CI with zero warnings.", "TEXT", false, true, false, None, Vec::new(), Vec::new(), None, Vec::new()),
        ]);

        for m in messages_seed {
            self.index_and_push_message(m, true);
        }
        self.persist_state_to_sqlite();
        self.db.lock().unwrap().execute_batch("COMMIT;").expect("commit seed transaction");
    }

    fn chat_index_text(c: &Chat) -> String {
        let mut s = format!("{} {}", c.title, c.description.as_deref().unwrap_or(""));
        for p in &c.participants.items {
            for field in [
                &p.full_name,
                &p.custom_nickname,
                &p.username,
                &p.phone_number,
                &p.email,
            ]
            .into_iter()
            .flatten()
            {
                s.push(' ');
                s.push_str(field);
            }
        }
        s
    }

    fn message_index_text(m: &Message) -> String {
        let mut s = format!(
            "{} {}",
            m.text.as_deref().unwrap_or(""),
            m.sender_name.as_deref().unwrap_or("")
        );
        for a in &m.attachments {
            for field in [&a.file_name, &a.transcript].into_iter().flatten() {
                s.push(' ');
                s.push_str(field);
            }
        }
        s
    }

    pub fn index_and_push_chat(&mut self, mut chat: Chat, persist: bool) -> usize {
        chat.sync_flags();
        if let Some(&existing_idx) = self.chat_idx_by_id.get(&chat.id) {
            let old_text = Self::chat_index_text(&self.chats[existing_idx]);
            self.chat_search_index
                .remove_doc(existing_idx as u32, &old_text);
            self.chat_search_index
                .insert_doc(existing_idx as u32, &Self::chat_index_text(&chat));
            if let Some(ref loc) = chat.local_chat_id {
                self.local_chat_idx_by_id.insert(loc.clone(), existing_idx);
            }
            if persist {
                self.persist_chat_row(&chat);
            }
            self.chats[existing_idx] = chat;
            return existing_idx;
        }
        let idx = self.chats.len();
        self.chat_search_index
            .insert_doc(idx as u32, &Self::chat_index_text(&chat));
        self.chat_idx_by_id.insert(chat.id.clone(), idx);
        if let Some(ref loc) = chat.local_chat_id {
            self.local_chat_idx_by_id.insert(loc.clone(), idx);
        }
        if persist {
            self.persist_chat_row(&chat);
        }
        self.chats.push(chat);
        idx
    }

    pub fn index_and_push_message(&mut self, mut msg: Message, persist: bool) -> usize {
        msg.sync_flags();
        if let Some(&existing_idx) = self.msg_idx_by_id.get(&msg.id) {
            return existing_idx;
        }
        let idx = self.messages.len();
        self.msg_search_index
            .insert_doc(idx as u32, &Self::message_index_text(&msg));
        self.msg_idx_by_id.insert(msg.id.clone(), idx);
        self.chat_msg_indices
            .entry(msg.chat_id.clone())
            .or_default()
            .push(idx);

        if let Some(&c_idx) = self.chat_idx_by_id.get(&msg.chat_id) {
            self.chats[c_idx].last_activity = Some(msg.timestamp.clone());
            self.chats[c_idx].preview = Some(msg.clone());
            self.chats[c_idx].sync_flags();
            if persist {
                let c_clone = self.chats[c_idx].clone();
                self.persist_chat_row(&c_clone);
            }
        }
        if persist {
            self.persist_message_row(&msg);
        }
        self.messages.push(msg);
        idx
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        self.next_seq += 1;
        format!("{prefix}_{}", self.next_seq)
    }

    pub fn resolve_chat_idx(&self, chat_or_local_id: &str) -> Option<usize> {
        self.chat_idx_by_id
            .get(chat_or_local_id)
            .copied()
            .or_else(|| self.local_chat_idx_by_id.get(chat_or_local_id).copied())
    }

    pub fn resolve_canonical_chat_id(&self, chat_or_local_id: &str) -> Option<String> {
        self.resolve_chat_idx(chat_or_local_id)
            .map(|idx| self.chats[idx].id.clone())
    }

    pub fn resolve_send_target_chat_id(&self, chat_or_local_id: &str) -> Option<String> {
        let idx = self.resolve_chat_idx(chat_or_local_id)?;
        let chat = &self.chats[idx];
        if let Some(ref merge) = chat.merge {
            assert!(
                !merge.chat_ids.is_empty(),
                "merged chat must have member chatIDs"
            );
            if let Some(ref def_id) = merge.default_chat_id {
                return self.resolve_canonical_chat_id(def_id);
            }
            let mut best_id = merge.chat_ids[0].clone();
            let mut best_ts = String::new();
            for member_id in &merge.chat_ids {
                if let Some(m_idx) = self.resolve_chat_idx(member_id) {
                    let ts = self.chats[m_idx].last_activity.clone().unwrap_or_default();
                    if ts >= best_ts {
                        best_ts = ts;
                        best_id = self.chats[m_idx].id.clone();
                    }
                }
            }
            return Some(best_id);
        }
        Some(chat.id.clone())
    }

    pub fn emit_ws(&self, event: WsEvent) {
        let _ = self.ws_tx.send(event);
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
        let now = now_rfc3339_millis();
        let items: Vec<User> = participant_ids
            .iter()
            .map(|pid| User {
                id: pid.clone(),
                full_name: Some(
                    pid.trim_start_matches('@')
                        .split(':')
                        .next()
                        .unwrap_or(pid)
                        .to_string(),
                ),
                username: Some(pid.clone()),
                email: None,
                phone_number: None,
                img_url: None,
                is_self: Some(false),
                cannot_message: Some(false),
                custom_nickname: None,
            })
            .collect();
        let merge_info = merge_chat_ids.map(|ids| {
            assert!(!ids.is_empty(), "merge chatIDs cannot be empty");
            ChatMergeInfo {
                chat_ids: ids,
                default_chat_id,
            }
        });

        let computed_title = title.unwrap_or_else(|| {
            if items.is_empty() {
                "Merged Chat".to_string()
            } else {
                items
                    .iter()
                    .filter_map(|u| u.full_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        });

        let mut chat = Chat {
            id: id.clone(),
            local_chat_id: Some(format!("local_{}", id.trim_start_matches('!'))),
            account_id: account_id.to_string(),
            title: computed_title,
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
            last_activity: Some(now),
            last_read_message_sort_key: None,
            merge: merge_info.clone(),
            merged_into_chat_id: None,
            preview: None,
            reminder: None,
            chat_theme: None,
            participants: ChatParticipants {
                has_more: false,
                total: items.len(),
                items,
            },
            flags: 0,
        };
        chat.sync_flags();
        let idx = self.index_and_push_chat(chat, true);

        if let Some(ref merge) = merge_info {
            for member_id in &merge.chat_ids {
                if let Some(m_idx) = self.resolve_chat_idx(member_id) {
                    self.chats[m_idx].merged_into_chat_id = Some(id.clone());
                    let member_clone = self.chats[m_idx].clone();
                    self.persist_chat_row(&member_clone);
                }
            }
        }

        if let Some(txt) = message_text.filter(|s| !s.trim().is_empty()) {
            let _ = self.send_message(&id, Some(txt), None, None);
        }

        let updated_chat = self.chats[idx].clone();
        self.emit_ws(WsEvent {
            event_type: "chat.upserted".to_string(),
            chat_id: Some(updated_chat.id.clone()),
            message_id: None,
            timestamp: now_rfc3339_millis(),
            chat: Some(updated_chat.clone()),
            message: None,
        });
        (updated_chat, "created".to_string())
    }

    pub fn patch_chat(
        &mut self,
        chat_or_local_id: &str,
        clear_draft: bool,
        new_draft: Option<ChatDraft>,
        new_merge: Option<Option<ChatMergeInfo>>,
    ) -> Result<Chat, &'static str> {
        let idx = self
            .resolve_chat_idx(chat_or_local_id)
            .ok_or("chat_not_found")?;
        if clear_draft {
            self.chats[idx].draft = None;
        } else if let Some(draft) = new_draft {
            let draft_has_content = draft
                .text
                .as_deref()
                .map(|s| !s.is_empty())
                .unwrap_or(false)
                || !draft.attachments.is_empty();
            if draft_has_content {
                let existing_non_empty = self.chats[idx]
                    .draft
                    .as_ref()
                    .map(|d| {
                        d.text.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                            || !d.attachments.is_empty()
                    })
                    .unwrap_or(false);
                if existing_non_empty {
                    return Err("existing_draft_not_empty");
                }
                self.chats[idx].draft = Some(draft);
            } else {
                self.chats[idx].draft = None;
            }
        }

        if let Some(merge_opt) = new_merge {
            let parent_id = self.chats[idx].id.clone();
            if let Some(old_m) = self.chats[idx].merge.take() {
                for mid in old_m.chat_ids {
                    if let Some(m_idx) = self.resolve_chat_idx(&mid) {
                        self.chats[m_idx].merged_into_chat_id = None;
                        let c = self.chats[m_idx].clone();
                        self.persist_chat_row(&c);
                    }
                }
            }
            if let Some(ref m) = merge_opt {
                for mid in &m.chat_ids {
                    if let Some(m_idx) = self.resolve_chat_idx(mid) {
                        self.chats[m_idx].merged_into_chat_id = Some(parent_id.clone());
                        let c = self.chats[m_idx].clone();
                        self.persist_chat_row(&c);
                    }
                }
            }
            self.chats[idx].merge = merge_opt;
        }

        self.chats[idx].sync_flags();
        let chat = self.chats[idx].clone();
        self.persist_chat_row(&chat);
        self.emit_ws(WsEvent {
            event_type: "chat.upserted".to_string(),
            chat_id: Some(chat.id.clone()),
            message_id: None,
            timestamp: now_rfc3339_millis(),
            chat: Some(chat.clone()),
            message: None,
        });
        Ok(chat)
    }

    pub fn send_message(
        &mut self,
        chat_or_local_id: &str,
        text: Option<String>,
        reply_to_message_id: Option<String>,
        attachment: Option<Attachment>,
    ) -> Result<(String, String, Message), &'static str> {
        let target_chat_id = self
            .resolve_send_target_chat_id(chat_or_local_id)
            .ok_or("chat_not_found")?;
        let c_idx = self.resolve_chat_idx(&target_chat_id).unwrap();
        let account_id = self.chats[c_idx].account_id.clone();
        let msg_id = self.next_id("msg");
        let sort_key = format!("{:07}", self.next_seq);
        let now = now_rfc3339_millis();

        let mut attachments = Vec::new();
        let mut msg_type = "TEXT".to_string();
        if let Some(mut att) = attachment {
            if att.is_voice_note == Some(true) {
                msg_type = "VOICE".to_string();
                if self.settings.auto_transcribe_voice_notes && att.transcript.is_none() {
                    att.transcript = Some(format!(
                        "[On-device transcript for {}]",
                        att.file_name.as_deref().unwrap_or("voice-note.ogg")
                    ));
                }
            } else if att.attachment_type == "img" {
                msg_type = "IMAGE".to_string();
            } else if att.attachment_type == "video" {
                msg_type = "VIDEO".to_string();
            } else {
                msg_type = "FILE".to_string();
            }
            attachments.push(att);
        }

        let mut msg = Message {
            id: msg_id.clone(),
            account_id: account_id.clone(),
            chat_id: target_chat_id.clone(),
            sender_id: format!("@self_{account_id}:beeper.com"),
            sender_name: Some("Alex".to_string()),
            sort_key,
            timestamp: now.clone(),
            text,
            message_type: Some(msg_type),
            is_sender: Some(true),
            is_unread: Some(false),
            is_bookmarked: Some(false),
            linked_message_id: reply_to_message_id,
            attachments,
            reactions: Vec::new(),
            poll: None,
            edit_history: Vec::new(),
            flags: 0,
        };
        msg.sync_flags();
        self.index_and_push_message(msg.clone(), true);

        self.emit_ws(WsEvent {
            event_type: "message.upserted".to_string(),
            chat_id: Some(target_chat_id.clone()),
            message_id: Some(msg_id.clone()),
            timestamp: now.clone(),
            chat: None,
            message: Some(msg.clone()),
        });
        let updated_chat = self.chats[c_idx].clone();
        self.emit_ws(WsEvent {
            event_type: "chat.upserted".to_string(),
            chat_id: Some(target_chat_id.clone()),
            message_id: None,
            timestamp: now,
            chat: Some(updated_chat),
            message: None,
        });
        Ok((target_chat_id, msg_id, msg))
    }

    pub fn edit_message(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        new_text: String,
    ) -> Result<(String, Message), &'static str> {
        let canonical_chat_id = self
            .resolve_canonical_chat_id(chat_or_local_id)
            .ok_or("chat_not_found")?;
        let &m_idx = self
            .msg_idx_by_id
            .get(message_id)
            .ok_or("message_not_found")?;
        assert_eq!(self.messages[m_idx].chat_id, canonical_chat_id);

        self.msg_search_index.remove_doc(
            m_idx as u32,
            &Self::message_index_text(&self.messages[m_idx]),
        );

        let now = now_rfc3339_millis();
        if let Some(prev_text) = self.messages[m_idx].text.take() {
            self.messages[m_idx].edit_history.push(EditHistoryEntry {
                text: prev_text,
                timestamp: now.clone(),
            });
        }
        self.messages[m_idx].text = Some(new_text);
        self.messages[m_idx].sync_flags();
        self.msg_search_index.insert_doc(
            m_idx as u32,
            &Self::message_index_text(&self.messages[m_idx]),
        );

        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);

        if let Some(&c_idx) = self.chat_idx_by_id.get(&canonical_chat_id) {
            if self.chats[c_idx].preview.as_ref().map(|p| p.id.as_str()) == Some(message_id) {
                self.chats[c_idx].preview = Some(updated.clone());
                let c_clone = self.chats[c_idx].clone();
                self.persist_chat_row(&c_clone);
            }
        }

        self.emit_ws(WsEvent {
            event_type: "message.upserted".to_string(),
            chat_id: Some(canonical_chat_id.clone()),
            message_id: Some(updated.id.clone()),
            timestamp: now,
            chat: None,
            message: Some(updated.clone()),
        });
        Ok((canonical_chat_id, updated))
    }

    pub fn delete_message(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
    ) -> Result<String, &'static str> {
        let canonical_chat_id = self
            .resolve_canonical_chat_id(chat_or_local_id)
            .ok_or("chat_not_found")?;
        let &m_idx = self
            .msg_idx_by_id
            .get(message_id)
            .ok_or("message_not_found")?;
        assert_eq!(self.messages[m_idx].chat_id, canonical_chat_id);

        self.msg_search_index.remove_doc(
            m_idx as u32,
            &Self::message_index_text(&self.messages[m_idx]),
        );
        self.messages[m_idx].flags |= MSG_DELETED;
        self.messages[m_idx].text = None;
        self.msg_idx_by_id.remove(message_id);
        if let Some(list) = self.chat_msg_indices.get_mut(&canonical_chat_id) {
            list.retain(|&i| i != m_idx);
        }
        self.delete_message_row(message_id);

        self.emit_ws(WsEvent {
            event_type: "message.deleted".to_string(),
            chat_id: Some(canonical_chat_id.clone()),
            message_id: Some(message_id.to_string()),
            timestamp: now_rfc3339_millis(),
            chat: None,
            message: None,
        });
        Ok(canonical_chat_id)
    }

    pub fn add_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
        transaction_id: Option<String>,
    ) -> Result<(String, String), &'static str> {
        let canonical_chat_id = self
            .resolve_canonical_chat_id(chat_or_local_id)
            .ok_or("chat_not_found")?;
        let &m_idx = self
            .msg_idx_by_id
            .get(message_id)
            .ok_or("message_not_found")?;
        let tx_id = transaction_id.unwrap_or_else(|| self.next_id("tx_rx"));
        let rx_id = self.next_id("rx");
        let self_pid = format!("@self_{}:beeper.com", self.messages[m_idx].account_id);
        self.messages[m_idx]
            .reactions
            .retain(|r| !(r.participant_id == self_pid && r.reaction_key == reaction_key));
        self.messages[m_idx].reactions.push(Reaction {
            id: rx_id,
            participant_id: self_pid,
            reaction_key: reaction_key.to_string(),
            animated: Some(false),
            img_url: None,
        });
        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);
        self.emit_ws(WsEvent {
            event_type: "message.upserted".to_string(),
            chat_id: Some(canonical_chat_id.clone()),
            message_id: Some(message_id.to_string()),
            timestamp: now_rfc3339_millis(),
            chat: None,
            message: Some(updated),
        });
        Ok((canonical_chat_id, tx_id))
    }

    pub fn remove_reaction(
        &mut self,
        chat_or_local_id: &str,
        message_id: &str,
        reaction_key: &str,
    ) -> Result<String, &'static str> {
        let canonical_chat_id = self
            .resolve_canonical_chat_id(chat_or_local_id)
            .ok_or("chat_not_found")?;
        let &m_idx = self
            .msg_idx_by_id
            .get(message_id)
            .ok_or("message_not_found")?;
        self.messages[m_idx]
            .reactions
            .retain(|r| r.reaction_key != reaction_key);
        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);
        self.emit_ws(WsEvent {
            event_type: "message.upserted".to_string(),
            chat_id: Some(canonical_chat_id.clone()),
            message_id: Some(message_id.to_string()),
            timestamp: now_rfc3339_millis(),
            chat: None,
            message: Some(updated),
        });
        Ok(canonical_chat_id)
    }

    pub fn toggle_bookmark(&mut self, message_id: &str) -> bool {
        let &m_idx = self.msg_idx_by_id.get(message_id).expect("message exists");
        let next = !(self.messages[m_idx].is_bookmarked == Some(true));
        self.messages[m_idx].is_bookmarked = Some(next);
        self.messages[m_idx].sync_flags();
        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);
        next
    }

    pub fn retranscribe_voice_note(&mut self, message_id: &str) -> String {
        let &m_idx = self.msg_idx_by_id.get(message_id).expect("message exists");
        self.msg_search_index.remove_doc(
            m_idx as u32,
            &Self::message_index_text(&self.messages[m_idx]),
        );

        let mut new_transcript = String::new();
        for att in &mut self.messages[m_idx].attachments {
            if att.is_voice_note == Some(true) {
                let updated_tr = format!(
                    "[Re-transcribed ({})]: {}",
                    self.settings.language.label(),
                    att.transcript
                        .as_deref()
                        .unwrap_or("Voice note audio clear.")
                );
                att.transcript = Some(updated_tr.clone());
                new_transcript = updated_tr;
            }
        }
        assert!(
            !new_transcript.is_empty(),
            "message had no voice note attachment"
        );
        self.msg_search_index.insert_doc(
            m_idx as u32,
            &Self::message_index_text(&self.messages[m_idx]),
        );
        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);
        new_transcript
    }

    pub fn vote_poll(&mut self, message_id: &str, option_id: &str, voter_id: &str) {
        let &m_idx = self.msg_idx_by_id.get(message_id).expect("message exists");
        let poll = self.messages[m_idx]
            .poll
            .as_mut()
            .expect("message has poll");
        if !poll.allow_multiple {
            for opt in &mut poll.options {
                if opt.id != option_id {
                    opt.voter_ids.retain(|v| v != voter_id);
                }
            }
        }
        let target = poll
            .options
            .iter_mut()
            .find(|o| o.id == option_id)
            .expect("poll option exists");
        if target.voter_ids.contains(&voter_id.to_string()) {
            target.voter_ids.retain(|v| v != voter_id);
        } else {
            target.voter_ids.push(voter_id.to_string());
        }
        let updated = self.messages[m_idx].clone();
        self.persist_message_row(&updated);
    }

    pub fn mark_unread_from_message(&mut self, chat_or_local_id: &str, message_id: &str) {
        let c_idx = self
            .resolve_chat_idx(chat_or_local_id)
            .expect("chat exists");
        let chat_id = self.chats[c_idx].id.clone();
        let &target_m_idx = self.msg_idx_by_id.get(message_id).expect("message exists");
        let target_sort_key = self.messages[target_m_idx].sort_key.clone();

        let mut unread_count = 0u32;
        if let Some(indices) = self.chat_msg_indices.get(&chat_id).cloned() {
            for i in indices {
                if self.messages[i].sort_key >= target_sort_key {
                    self.messages[i].is_unread = Some(true);
                    self.messages[i].sync_flags();
                    unread_count += 1;
                }
            }
        }
        self.chats[c_idx].unread_count = unread_count.max(1);
        self.chats[c_idx].is_marked_unread = Some(true);
        self.chats[c_idx].sync_flags();
        let c_clone = self.chats[c_idx].clone();
        self.persist_chat_row(&c_clone);
    }

    pub fn set_user_nickname(
        &mut self,
        chat_or_local_id: &str,
        user_id: &str,
        nickname: Option<String>,
    ) {
        let c_idx = self
            .resolve_chat_idx(chat_or_local_id)
            .expect("chat exists");
        self.chat_search_index
            .remove_doc(c_idx as u32, &Self::chat_index_text(&self.chats[c_idx]));

        for p in &mut self.chats[c_idx].participants.items {
            if p.id == user_id {
                p.custom_nickname = nickname.clone();
            }
        }
        if self.chats[c_idx].chat_type == "single" {
            if let Some(ref nick) = nickname {
                self.chats[c_idx].title = nick.clone();
            }
        }
        self.chat_search_index
            .insert_doc(c_idx as u32, &Self::chat_index_text(&self.chats[c_idx]));
        let c_clone = self.chats[c_idx].clone();
        self.persist_chat_row(&c_clone);
    }

    pub fn total_unread_chats(&self) -> u32 {
        self.chats
            .iter()
            .filter(|c| (c.flags & CHAT_ARCHIVED) == 0 && (c.flags & CHAT_UNREAD) != 0)
            .count() as u32
    }

    pub fn store_uploaded_asset(
        &mut self,
        file_name: String,
        mime_type: String,
        bytes: &[u8],
    ) -> StoredAsset {
        assert!(!bytes.is_empty(), "uploaded asset bytes cannot be empty");
        let upload_id = self.next_id("upl");
        let safe_name: String = file_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file_path = self.assets_dir.join(format!("{upload_id}_{safe_name}"));
        std::fs::write(&file_path, bytes).expect("write uploaded asset to disk");

        let mxc_url = format!("mxc://beeper.com/{upload_id}");
        let asset = StoredAsset {
            upload_id: upload_id.clone(),
            mxc_url: mxc_url.clone(),
            local_file_path: file_path.to_string_lossy().to_string(),
            file_name,
            mime_type,
            file_size: bytes.len() as u64,
            width: Some(1280),
            height: Some(720),
            duration: None,
            created_epoch_secs: now_unix_secs(),
        };
        self.db
            .lock()
            .unwrap()
            .prepare_cached(
                "INSERT OR REPLACE INTO beeper_rs_assets (upload_id, json) VALUES (?1, ?2)",
            )
            .unwrap()
            .execute(params![
                asset.upload_id,
                serde_json::to_string(&asset).unwrap()
            ])
            .expect("persist asset");
        self.assets_by_mxc.insert(mxc_url, upload_id.clone());
        self.assets.insert(upload_id, asset.clone());
        asset
    }

    pub fn resolve_asset_path(&self, url: &str) -> Option<(PathBuf, String)> {
        if let Some(upl_id) = self.assets_by_mxc.get(url) {
            let a = self.assets.get(upl_id)?;
            return Some((PathBuf::from(&a.local_file_path), a.mime_type.clone()));
        }
        if let Some(a) = self.assets.get(url) {
            return Some((PathBuf::from(&a.local_file_path), a.mime_type.clone()));
        }
        if let Some(p) = crate::platform::file_url_to_path(url) {
            if p.exists() {
                return Some((p, "application/octet-stream".to_string()));
            }
        }
        None
    }

    pub fn calculate_storage_bytes(&self) -> (u64, u64) {
        let db_bytes = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let mut media_bytes = 0u64;
        if let Ok(entries) = std::fs::read_dir(&self.assets_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    media_bytes += meta.len();
                }
            }
        }
        (db_bytes, media_bytes)
    }

    pub fn clear_cached_media_older_than_days(&mut self, days: i64) -> usize {
        let cutoff = now_unix_secs() - (days * 86_400);
        let to_remove: Vec<String> = self
            .assets
            .iter()
            .filter(|(_, a)| a.created_epoch_secs <= cutoff)
            .map(|(k, _)| k.clone())
            .collect();
        let conn = self.db.lock().unwrap();
        for id in &to_remove {
            if let Some(a) = self.assets.remove(id) {
                self.assets_by_mxc.remove(&a.mxc_url);
                let _ = std::fs::remove_file(&a.local_file_path);
                let _ = conn.execute(
                    "DELETE FROM beeper_rs_assets WHERE upload_id = ?1",
                    params![id],
                );
            }
        }
        to_remove.len()
    }
}

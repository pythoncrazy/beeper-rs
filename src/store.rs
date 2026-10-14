use crate::config::{
    CentralConfig, DEFAULT_DEVICE_ID, DEFAULT_HOMESERVER, DEFAULT_RECOVERY_CODE, DEFAULT_TOKEN,
    DEFAULT_USER_ID, SEEDED_BRIDGES, WS_CHANNEL_CAPACITY,
};
use crate::models::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
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
            let mut matched_docs = BTreeSet::new();
            for (term, ids) in &self.postings {
                if term.starts_with(q) {
                    matched_docs.extend(ids.iter().copied());
                }
            }
            let term_vec: Vec<u32> = matched_docs.into_iter().collect();
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

#[derive(Debug, Clone)]
pub struct SettingCatalogEntry {
    pub id: &'static str,
    pub section: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

pub const SETTINGS_CATALOG: [SettingCatalogEntry; 24] = [
    SettingCatalogEntry {
        id: "show_archived_count",
        section: "Chat List",
        title: "Show archived count",
        description: "Display total archived chats badge in the sidebar header",
    },
    SettingCatalogEntry {
        id: "show_starred_pin_view",
        section: "Chat List",
        title: "Show starred pin view",
        description: "Pin tab for starred and favorite conversations",
    },
    SettingCatalogEntry {
        id: "show_all_chats_pin_view",
        section: "Chat List",
        title: "Show all chats pin view",
        description: "Pin tab showing all unified inbox conversations",
    },
    SettingCatalogEntry {
        id: "show_unread_pin_view",
        section: "Chat List",
        title: "Show unread pin view",
        description: "Quick filter pin tab for unread conversations",
    },
    SettingCatalogEntry {
        id: "show_drafts_pin_view",
        section: "Chat List",
        title: "Show drafts pin view",
        description: "Quick filter pin tab for conversations with unsent drafts",
    },
    SettingCatalogEntry {
        id: "filter_pins_by_network",
        section: "Chat List",
        title: "Filter pins by network",
        description: "Scope pinned avatars to the currently selected chat network",
    },
    SettingCatalogEntry {
        id: "show_favorite_chats_section",
        section: "Chat List",
        title: "Show favorite chats section",
        description: "Display a dedicated Favorites group above the main chat list",
    },
    SettingCatalogEntry {
        id: "keep_favorites_in_inbox",
        section: "Chat List",
        title: "Keep favorites in inbox",
        description: "Keep favorited conversations visible inside the main inbox list",
    },
    SettingCatalogEntry {
        id: "show_message_previews",
        section: "Chat List",
        title: "Show message previews",
        description: "Show latest message snippet below each chat title in the sidebar",
    },
    SettingCatalogEntry {
        id: "unread_chats_always_on_top",
        section: "Chat List",
        title: "Unread chats always on top",
        description: "Sort unread threads above read threads in the chat list",
    },
    SettingCatalogEntry {
        id: "show_full_names_in_chats",
        section: "Conversation View",
        title: "Show full names in chats",
        description: "Display full sender names instead of first names in group chats",
    },
    SettingCatalogEntry {
        id: "auto_play_videos",
        section: "Conversation View",
        title: "Auto-play videos",
        description: "Automatically play inline video attachments and GIFs",
    },
    SettingCatalogEntry {
        id: "show_chat_header",
        section: "Conversation View",
        title: "Show chat header",
        description: "Show the top header bar with participant details in conversation view",
    },
    SettingCatalogEntry {
        id: "show_message_input_buttons",
        section: "Conversation View",
        title: "Show message input buttons",
        description: "Show attachment, emoji, voice note, poll, and schedule buttons in composer",
    },
    SettingCatalogEntry {
        id: "link_previews",
        section: "Conversation View",
        title: "Link previews",
        description: "Generate rich previews for URLs shared in messages",
    },
    SettingCatalogEntry {
        id: "show_right_panel_by_default",
        section: "Conversation View",
        title: "Show right panel by default",
        description: "Keep the Chat Info, Shared Media, and Bookmarks inspector open",
    },
    SettingCatalogEntry {
        id: "load_remote_content_automatically",
        section: "Conversation View",
        title: "Load remote content automatically",
        description: "Fetch external images and link metadata automatically",
    },
    SettingCatalogEntry {
        id: "use_twenty_four_hour_time",
        section: "Conversation View",
        title: "Use 24-hour time",
        description: "Format message timestamps using 24-hour military clock",
    },
    SettingCatalogEntry {
        id: "theme",
        section: "Appearance",
        title: "CSS Theme",
        description:
            "Switch between Texts Light, Texts Dark, Dracula, Nocturne, Dark Purple, and Custom CSS",
    },
    SettingCatalogEntry {
        id: "no_transparency",
        section: "Appearance",
        title: "No transparency",
        description: "Disable window vibrancy and translucent backgrounds (.no-transparency)",
    },
    SettingCatalogEntry {
        id: "reduce_transparency",
        section: "Appearance",
        title: "Reduce transparency",
        description: "Use solid header and sidebar surfaces (.reduce-transparency)",
    },
    SettingCatalogEntry {
        id: "fix_lockdown_emojis",
        section: "Appearance",
        title: "Fix Lockdown Mode Emojis",
        description: "Apply fix-lockdown-mode-emojis.css bitmap font override",
    },
    SettingCatalogEntry {
        id: "auto_transcribe_voice_notes",
        section: "Voice & Media",
        title: "Transcribe voice notes",
        description: "Automatically transcribe incoming and outgoing voice notes on-device",
    },
    SettingCatalogEntry {
        id: "repeat_message_alerts",
        section: "Notifications",
        title: "Repeat message alerts",
        description: "Repeat unread notification alerts up to 3 times",
    },
];

pub struct BeeperStore {
    pub config: CentralConfig,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub assets_dir: PathBuf,
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
    next_seq: u64,
}

impl BeeperStore {
    pub fn open_or_create(data_dir: PathBuf, pas_port: u16) -> Self {
        let config = CentralConfig::new(data_dir.clone(), "127.0.0.1".to_string(), pas_port, None);
        Self::from_config(config)
    }

    pub fn from_config(config: CentralConfig) -> Self {
        std::fs::create_dir_all(&config.data_dir).expect("create BeeperTexts data_dir");
        std::fs::create_dir_all(&config.assets_dir).expect("create BeeperTexts/media");
        let (ws_tx, _) = broadcast::channel(WS_CHANNEL_CAPACITY);

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
            settings: config.settings.clone(),
            config,
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
            oauth_clients: HashMap::new(),
            oauth_codes: HashMap::new(),
            valid_tokens,
            focused_chat_id: None,
            focused_message_id: None,
            ws_tx,
            next_seq: 1000,
        };

        store.init_sqlite();
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

    fn open_conn(&self) -> Connection {
        let conn = Connection::open(&self.db_path).expect("open BeeperTexts/index.db");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .expect("configure WAL pragma");
        conn
    }

    fn init_sqlite(&self) {
        self.open_conn()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS key_values (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, json TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sort_key TEXT NOT NULL, json TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS assets (upload_id TEXT PRIMARY KEY, json TEXT NOT NULL);",
            )
            .expect("init SQLite schema");
    }

    pub fn persist_state_to_sqlite(&self) {
        let conn = self.open_conn();
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
            conn.execute(
                "INSERT OR REPLACE INTO key_values (key, value) VALUES (?1, ?2)",
                params![k, v],
            )
            .expect("write key_values");
        }
    }

    fn persist_chat_row(&self, chat: &Chat) {
        self.open_conn()
            .execute(
                "INSERT OR REPLACE INTO chats (id, json) VALUES (?1, ?2)",
                params![chat.id, serde_json::to_string(chat).unwrap()],
            )
            .expect("persist chat row");
    }

    fn persist_message_row(&self, msg: &Message) {
        self.open_conn()
            .execute(
                "INSERT OR REPLACE INTO messages (id, chat_id, sort_key, json) VALUES (?1, ?2, ?3, ?4)",
                params![msg.id, msg.chat_id, msg.sort_key, serde_json::to_string(msg).unwrap()],
            )
            .expect("persist message row");
    }

    fn delete_message_row(&self, msg_id: &str) {
        self.open_conn()
            .execute("DELETE FROM messages WHERE id = ?1", params![msg_id])
            .expect("delete message row");
    }

    fn load_from_sqlite_or_seed(&mut self) {
        let conn = self.open_conn();
        if let Ok(raw_state) = conn.query_row::<String, _, _>(
            "SELECT value FROM key_values WHERE key = 'beeperState'",
            [],
            |r| r.get(0),
        ) {
            self.beeper_state = serde_json::from_str(&raw_state).expect("parse stored beeperState");
            self.valid_tokens
                .insert(self.beeper_state.access_token.clone());
            self.setup_status = Self::build_setup_from_state(&self.beeper_state, None);
        }
        if let Ok(raw_bridges) = conn.query_row::<String, _, _>(
            "SELECT value FROM key_values WHERE key = 'bridgeAccounts'",
            [],
            |r| r.get(0),
        ) {
            self.bridge_accounts =
                serde_json::from_str(&raw_bridges).expect("parse stored bridgeAccounts");
        }
        if let Ok(raw_settings) = conn.query_row::<String, _, _>(
            "SELECT value FROM key_values WHERE key = 'beeperSettings'",
            [],
            |r| r.get(0),
        ) {
            let pas_port = self.settings.pas_port;
            self.settings =
                serde_json::from_str(&raw_settings).expect("parse stored beeperSettings");
            self.settings.pas_port = pas_port;
        }

        let loaded_chats: Vec<Chat> = conn
            .prepare("SELECT json FROM chats")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
            .collect();

        if loaded_chats.is_empty() {
            drop(conn);
            self.seed_default_workspace();
        } else {
            let loaded_msgs: Vec<Message> = conn
                .prepare("SELECT json FROM messages ORDER BY sort_key ASC")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
                .collect();
            drop(conn);
            self.rebuild_accounts_from_bridges();
            for c in loaded_chats {
                self.index_and_push_chat(c, false);
            }
            for m in loaded_msgs {
                self.index_and_push_message(m, false);
            }
            self.next_seq = 1000 + self.messages.len() as u64 + self.chats.len() as u64;
        }
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
        self.bridge_accounts = SEEDED_BRIDGES
            .iter()
            .map(|(bid, btype, name, mode)| BridgeAccountInfo {
                account_id: format!("acc_{bid}"),
                bridge_id: (*bid).to_string(),
                bridge_type: (*btype).to_string(),
                is_running: true,
                login_id: Some(format!("login_{bid}_01")),
                mode: Some(
                    if *mode == BridgeMode::OnDevice {
                        "on_device"
                    } else {
                        "cloud"
                    }
                    .to_string(),
                ),
                network_name: (*name).to_string(),
                state: "CONNECTED".to_string(),
            })
            .collect();
        self.rebuild_accounts_from_bridges();

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

        let chats_seed = vec![
            Chat {
                id: "!wa_eric:beeper.com".to_string(),
                local_chat_id: Some("local_wa_eric".to_string()),
                account_id: "acc_whatsapp".to_string(),
                title: "Eric Migicovsky".to_string(),
                chat_type: "single".to_string(),
                unread_count: 1,
                description: Some("Beeper & Pebble founder".to_string()),
                draft: None,
                is_archived: Some(false),
                is_marked_unread: Some(false),
                is_muted: Some(false),
                is_pinned: Some(true),
                is_low_priority: Some(false),
                label_ids: vec!["lbl_favorite".to_string(), "lbl_vip".to_string()],
                last_activity: Some("2026-10-14T08:15:00.000Z".to_string()),
                last_read_message_sort_key: Some("000001".to_string()),
                merge: None,
                merged_into_chat_id: None,
                preview: None,
                reminder: None,
                chat_theme: None,
                participants: ChatParticipants {
                    has_more: false,
                    total: 2,
                    items: vec![
                        make_user(
                            "@eric:beeper.com",
                            "Eric Migicovsky",
                            "eric",
                            Some("+14155550101"),
                        ),
                        self.accounts[0].user.clone(),
                    ],
                },
                flags: 0,
            },
            Chat {
                id: "!im_sarah:beeper.com".to_string(),
                local_chat_id: Some("local_im_sarah".to_string()),
                account_id: "acc_imessage".to_string(),
                title: "Sarah Chen".to_string(),
                chat_type: "single".to_string(),
                unread_count: 0,
                description: Some("iMessage on-device".to_string()),
                draft: None,
                is_archived: Some(false),
                is_marked_unread: Some(false),
                is_muted: Some(true),
                is_pinned: Some(true),
                is_low_priority: Some(false),
                label_ids: vec!["lbl_favorite".to_string()],
                last_activity: Some("2026-10-14T08:10:00.000Z".to_string()),
                last_read_message_sort_key: Some("000002".to_string()),
                merge: None,
                merged_into_chat_id: None,
                preview: None,
                reminder: None,
                chat_theme: None,
                participants: ChatParticipants {
                    has_more: false,
                    total: 2,
                    items: vec![
                        make_user(
                            "@sarah:beeper.com",
                            "Sarah Chen",
                            "sarahc",
                            Some("+14155550102"),
                        ),
                        self.accounts[1].user.clone(),
                    ],
                },
                flags: 0,
            },
            Chat {
                id: "!tg_rust:beeper.com".to_string(),
                local_chat_id: Some("local_tg_rust".to_string()),
                account_id: "acc_telegram".to_string(),
                title: "Beeper Rust Core Team".to_string(),
                chat_type: "group".to_string(),
                unread_count: 2,
                description: Some("Architecture, E2EE & native desktop performance".to_string()),
                draft: None,
                is_archived: Some(false),
                is_marked_unread: Some(false),
                is_muted: Some(false),
                is_pinned: Some(true),
                is_low_priority: Some(false),
                label_ids: vec!["lbl_work".to_string()],
                last_activity: Some("2026-10-14T08:20:00.000Z".to_string()),
                last_read_message_sort_key: Some("000003".to_string()),
                merge: None,
                merged_into_chat_id: None,
                preview: None,
                reminder: None,
                chat_theme: None,
                participants: ChatParticipants {
                    has_more: false,
                    total: 3,
                    items: vec![
                        make_user("@tulir:beeper.com", "Tulir Asokan", "tulir", None),
                        make_user("@batuhan:beeper.com", "Batuhan", "batuhan", None),
                        self.accounts[2].user.clone(),
                    ],
                },
                flags: 0,
            },
            Chat {
                id: "!sg_security:beeper.com".to_string(),
                local_chat_id: Some("local_sg_security".to_string()),
                account_id: "acc_signal".to_string(),
                title: "Signal Security Ops".to_string(),
                chat_type: "single".to_string(),
                unread_count: 0,
                description: Some("Zero-knowledge E2EE verification channel".to_string()),
                draft: None,
                is_archived: Some(false),
                is_marked_unread: Some(false),
                is_muted: Some(false),
                is_pinned: Some(false),
                is_low_priority: Some(false),
                label_ids: Vec::new(),
                last_activity: Some("2026-10-14T07:45:00.000Z".to_string()),
                last_read_message_sort_key: Some("000004".to_string()),
                merge: None,
                merged_into_chat_id: None,
                preview: None,
                reminder: None,
                chat_theme: None,
                participants: ChatParticipants {
                    has_more: false,
                    total: 2,
                    items: vec![
                        make_user(
                            "@secops:beeper.com",
                            "SecOps Lead",
                            "secops",
                            Some("+14155550190"),
                        ),
                        self.accounts[3].user.clone(),
                    ],
                },
                flags: 0,
            },
        ];

        for c in chats_seed {
            self.index_and_push_chat(c, true);
        }

        let messages_seed = vec![
            Message {
                id: "msg_1001".to_string(),
                account_id: "acc_whatsapp".to_string(),
                chat_id: "!wa_eric:beeper.com".to_string(),
                sender_id: "@eric:beeper.com".to_string(),
                sender_name: Some("Eric Migicovsky".to_string()),
                sort_key: "0001001".to_string(),
                timestamp: "2026-10-14T08:15:00.000Z".to_string(),
                text: Some("Welcome to beeper-rs! Check out the local Desktop API on port 23373 and the zero-bloat Rust UI.".to_string()),
                message_type: Some("TEXT".to_string()),
                is_sender: Some(false),
                is_unread: Some(true),
                is_bookmarked: Some(true),
                linked_message_id: None,
                attachments: Vec::new(),
                reactions: vec![Reaction {
                    id: "rx_1".to_string(),
                    participant_id: "@self_whatsapp:beeper.com".to_string(),
                    reaction_key: "⚡".to_string(),
                    animated: Some(false),
                    img_url: None,
                }],
                poll: None,
                edit_history: Vec::new(),
                flags: 0,
            },
            Message {
                id: "msg_1002".to_string(),
                account_id: "acc_imessage".to_string(),
                chat_id: "!im_sarah:beeper.com".to_string(),
                sender_id: "@sarah:beeper.com".to_string(),
                sender_name: Some("Sarah Chen".to_string()),
                sort_key: "0001002".to_string(),
                timestamp: "2026-10-14T08:10:00.000Z".to_string(),
                text: Some("Let's ship the release at 14:00 UTC. Check https://beeper.com and ping @eric:beeper.com".to_string()),
                message_type: Some("TEXT".to_string()),
                is_sender: Some(false),
                is_unread: Some(false),
                is_bookmarked: Some(false),
                linked_message_id: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                poll: None,
                edit_history: vec![EditHistoryEntry {
                    text: "Let's ship the release at 13:00 UTC.".to_string(),
                    timestamp: "2026-10-14T08:09:00.000Z".to_string(),
                }],
                flags: 0,
            },
            Message {
                id: "msg_1003".to_string(),
                account_id: "acc_telegram".to_string(),
                chat_id: "!tg_rust:beeper.com".to_string(),
                sender_id: "@tulir:beeper.com".to_string(),
                sender_name: Some("Tulir Asokan".to_string()),
                sort_key: "0001003".to_string(),
                timestamp: "2026-10-14T08:19:00.000Z".to_string(),
                text: Some("Voice note from standup + architecture PDF attached.".to_string()),
                message_type: Some("VOICE".to_string()),
                is_sender: Some(false),
                is_unread: Some(true),
                is_bookmarked: Some(false),
                linked_message_id: None,
                attachments: vec![
                    Attachment {
                        attachment_type: "audio".to_string(),
                        id: Some("att_voice_1".to_string()),
                        duration: Some(14.5),
                        file_name: Some("standup-note.ogg".to_string()),
                        file_size: Some(48_200),
                        is_gif: Some(false),
                        is_sticker: Some(false),
                        is_voice_note: Some(true),
                        mime_type: Some("audio/ogg".to_string()),
                        poster_img: None,
                        size: None,
                        src_url: Some("mxc://beeper.com/standup_voice_01".to_string()),
                        transcript: Some("All 16 Mautrix bridges and the SQLite WAL index are running at sub-millisecond latency.".to_string()),
                    },
                    Attachment {
                        attachment_type: "unknown".to_string(),
                        id: Some("att_pdf_1".to_string()),
                        duration: None,
                        file_name: Some("beeper-v4-spec.pdf".to_string()),
                        file_size: Some(128_500),
                        is_gif: Some(false),
                        is_sticker: Some(false),
                        is_voice_note: Some(false),
                        mime_type: Some("application/pdf".to_string()),
                        poster_img: None,
                        size: None,
                        src_url: Some("mxc://beeper.com/beeper_v4_spec_pdf".to_string()),
                        transcript: None,
                    },
                ],
                reactions: Vec::new(),
                poll: None,
                edit_history: Vec::new(),
                flags: 0,
            },
            Message {
                id: "msg_1004".to_string(),
                account_id: "acc_telegram".to_string(),
                chat_id: "!tg_rust:beeper.com".to_string(),
                sender_id: "@batuhan:beeper.com".to_string(),
                sender_name: Some("Batuhan".to_string()),
                sort_key: "0001004".to_string(),
                timestamp: "2026-10-14T08:20:00.000Z".to_string(),
                text: Some("Poll: Which default dark theme do you prefer?".to_string()),
                message_type: Some("POLL".to_string()),
                is_sender: Some(false),
                is_unread: Some(true),
                is_bookmarked: Some(false),
                linked_message_id: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                poll: Some(PollData {
                    question: "Which default dark theme do you prefer?".to_string(),
                    allow_multiple: false,
                    options: vec![
                        PollOption { id: "opt_1".to_string(), label: "Texts Dark".to_string(), voter_ids: vec!["@tulir:beeper.com".to_string()] },
                        PollOption { id: "opt_2".to_string(), label: "Dracula".to_string(), voter_ids: vec!["@batuhan:beeper.com".to_string()] },
                        PollOption { id: "opt_3".to_string(), label: "Nocturne".to_string(), voter_ids: Vec::new() },
                        PollOption { id: "opt_4".to_string(), label: "Dark Purple".to_string(), voter_ids: Vec::new() },
                    ],
                }),
                edit_history: Vec::new(),
                flags: 0,
            },
        ];

        for m in messages_seed {
            self.index_and_push_message(m, true);
        }
        self.persist_state_to_sqlite();
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
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
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
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

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

        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
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
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
            created_epoch_secs: chrono::Utc::now().timestamp(),
        };
        self.open_conn()
            .execute(
                "INSERT OR REPLACE INTO assets (upload_id, json) VALUES (?1, ?2)",
                params![asset.upload_id, serde_json::to_string(&asset).unwrap()],
            )
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
        if let Some(stripped) = url.strip_prefix("file://") {
            let p = PathBuf::from(stripped);
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
        let cutoff = chrono::Utc::now().timestamp() - (days * 86_400);
        let to_remove: Vec<String> = self
            .assets
            .iter()
            .filter(|(_, a)| a.created_epoch_secs <= cutoff)
            .map(|(k, _)| k.clone())
            .collect();
        let conn = self.open_conn();
        for id in &to_remove {
            if let Some(a) = self.assets.remove(id) {
                self.assets_by_mxc.remove(&a.mxc_url);
                let _ = std::fs::remove_file(&a.local_file_path);
                let _ = conn.execute("DELETE FROM assets WHERE upload_id = ?1", params![id]);
            }
        }
        to_remove.len()
    }
}

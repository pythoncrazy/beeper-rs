use crate::models::{
    BeeperSettings, BridgeMode, LanguageChoice, NetworkKind, NetworkSpec, ThemeChoice,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const APP_BUNDLE_ID: &str = "com.automattic.beeper.desktop";
pub const APP_NAME: &str = "Beeper";
pub const APP_VERSION: &str = "4.3.176";
pub const DEFAULT_PAS_HOST: &str = "127.0.0.1";
pub const DEFAULT_PAS_PORT: u16 = 23373;
pub const DEFAULT_HOMESERVER: &str = "https://matrix.beeper.com";
pub const DEFAULT_USER_ID: &str = "@alex:beeper.com";
pub const DEFAULT_DEVICE_ID: &str = "BEEPER_RS_DEV01";
pub const DEFAULT_TOKEN: &str = "bp_tok_beeper_rs_default_secret";
pub const DEFAULT_RECOVERY_CODE: &str = "EsTc 9K4p Lm2Q Xv8N Beeper RsKey 2026";
pub const TRAY_ICON_SIZE: u32 = 32;
pub const WS_CHANNEL_CAPACITY: usize = 512;
pub const OPENAPI_SPEC_JSON: &str = include_str!("../assets/openapi.json");
pub const CSS_VARIABLES: &str = include_str!("../assets/themes/variables.css");
pub const CSS_TEXTS: &str = include_str!("../assets/themes/texts.css");
pub const CSS_DRACULA: &str = include_str!("../assets/themes/dracula.css");
pub const CSS_NOCTURNE: &str = include_str!("../assets/themes/nocturne.css");
pub const CSS_DARK_PURPLE: &str = include_str!("../assets/themes/dark-purple.css");
pub const CSS_FIX_LOCKDOWN_EMOJIS: &str =
    include_str!("../assets/themes/fix-lockdown-mode-emojis.css");

#[derive(Debug, Clone)]
pub struct CentralConfig {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub assets_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub profile: Option<String>,
    pub settings: BeeperSettings,
}

impl CentralConfig {
    pub fn new(data_dir: PathBuf, host: String, port: u16, profile: Option<String>) -> Self {
        assert!(!host.trim().is_empty(), "host must not be empty");
        assert!(port > 0, "port must be positive");
        let db_path = data_dir.join("index.db");
        let assets_dir = data_dir.join("media");
        let mut settings = Self::default_settings();
        settings.pas_port = port;
        Self {
            data_dir,
            db_path,
            assets_dir,
            host,
            port,
            profile,
            settings,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    pub fn default_settings() -> BeeperSettings {
        BeeperSettings {
            show_archived_count: true,
            show_starred_pin_view: true,
            show_all_chats_pin_view: true,
            show_unread_pin_view: true,
            show_drafts_pin_view: true,
            filter_pins_by_network: true,
            show_favorite_chats_section: false,
            keep_favorites_in_inbox: false,
            show_message_previews: true,
            unread_chats_always_on_top: false,
            show_full_names_in_chats: true,
            auto_play_videos: true,
            show_chat_header: true,
            show_message_input_buttons: true,
            link_previews: true,
            show_right_panel_by_default: false,
            load_remote_content_automatically: true,
            use_twenty_four_hour_time: true,
            theme: ThemeChoice::TextsDark,
            no_transparency: false,
            reduce_transparency: false,
            fix_lockdown_emojis: false,
            zoom_percent: 100,
            custom_css_path: None,
            custom_css_source: String::new(),
            language: LanguageChoice::English,
            repeat_message_alerts: 0,
            auto_transcribe_voice_notes: true,
            enable_pas_api: true,
            pas_port: DEFAULT_PAS_PORT,
            launch_at_startup: false,
            minimize_to_tray: true,
        }
    }
}

pub const NETWORK_SPECS: [NetworkSpec; 16] = [
    NetworkSpec {
        kind: NetworkKind::WhatsApp,
        bridge_id: "whatsapp",
        bridge_type: "whatsapp",
        display_name: "WhatsApp",
        badge_code: "WA",
        brand_rgb: [37, 211, 102],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Messenger,
        bridge_id: "facebook",
        bridge_type: "meta",
        display_name: "FB Messenger",
        badge_code: "FB",
        brand_rgb: [0, 132, 255],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Instagram,
        bridge_id: "instagram",
        bridge_type: "meta",
        display_name: "Instagram",
        badge_code: "IG",
        brand_rgb: [225, 48, 108],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Telegram,
        bridge_id: "telegram",
        bridge_type: "telegram",
        display_name: "Telegram",
        badge_code: "TG",
        brand_rgb: [38, 165, 228],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Signal,
        bridge_id: "signal",
        bridge_type: "signal",
        display_name: "Signal",
        badge_code: "SG",
        brand_rgb: [58, 118, 240],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Viber,
        bridge_id: "viber",
        bridge_type: "viber",
        display_name: "Viber",
        badge_code: "VB",
        brand_rgb: [115, 96, 242],
        supports_on_device: true,
        supports_cloud: false,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::IMessage,
        bridge_id: "imessage",
        bridge_type: "imessage",
        display_name: "iMessage",
        badge_code: "iM",
        brand_rgb: [52, 199, 89],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: true,
        supports_e2ee: true,
        supports_polls: false,
        supports_edit_history: true,
    },
    NetworkSpec {
        kind: NetworkKind::GoogleMessages,
        bridge_id: "gmessages",
        bridge_type: "gmessages",
        display_name: "Google Messages",
        badge_code: "GM",
        brand_rgb: [26, 115, 232],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: true,
        supports_e2ee: true,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::GoogleVoice,
        bridge_id: "gvoice",
        bridge_type: "gvoice",
        display_name: "Google Voice",
        badge_code: "GV",
        brand_rgb: [15, 157, 88],
        supports_on_device: false,
        supports_cloud: true,
        supports_sms_mms: true,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::GoogleChat,
        bridge_id: "googlechat",
        bridge_type: "googlechat",
        display_name: "Google Chat",
        badge_code: "GC",
        brand_rgb: [0, 172, 71],
        supports_on_device: false,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::TwitterX,
        bridge_id: "twitter",
        bridge_type: "twitter",
        display_name: "Twitter / X",
        badge_code: "X",
        brand_rgb: [29, 155, 240],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Slack,
        bridge_id: "slack",
        bridge_type: "slack",
        display_name: "Slack",
        badge_code: "SL",
        brand_rgb: [224, 30, 90],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Discord,
        bridge_id: "discord",
        bridge_type: "discord",
        display_name: "Discord",
        badge_code: "DC",
        brand_rgb: [88, 101, 242],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: true,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::LinkedIn,
        bridge_id: "linkedin",
        bridge_type: "linkedin",
        display_name: "LinkedIn",
        badge_code: "IN",
        brand_rgb: [10, 102, 194],
        supports_on_device: true,
        supports_cloud: true,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Line,
        bridge_id: "line",
        bridge_type: "line",
        display_name: "LINE",
        badge_code: "LN",
        brand_rgb: [6, 199, 85],
        supports_on_device: true,
        supports_cloud: false,
        supports_sms_mms: false,
        supports_e2ee: true,
        supports_polls: false,
        supports_edit_history: false,
    },
    NetworkSpec {
        kind: NetworkKind::Tumblr,
        bridge_id: "tumblr",
        bridge_type: "tumblr",
        display_name: "Tumblr",
        badge_code: "TB",
        brand_rgb: [0, 25, 53],
        supports_on_device: true,
        supports_cloud: false,
        supports_sms_mms: false,
        supports_e2ee: false,
        supports_polls: false,
        supports_edit_history: false,
    },
];

pub fn network_spec_by_bridge(bridge_id: &str) -> &'static NetworkSpec {
    let found = NETWORK_SPECS.iter().find(|n| n.bridge_id == bridge_id);
    assert!(found.is_some(), "unknown bridge_id: {bridge_id}");
    found.unwrap()
}

pub fn resolve_data_dir_for_os(
    os_name: &str,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    if let Some(override_dir) = env_vars.get("BEEPER_USER_DATA_DIR") {
        let trimmed = override_dir.trim();
        assert!(
            !trimmed.is_empty(),
            "BEEPER_USER_DATA_DIR must not be empty"
        );
        return PathBuf::from(trimmed);
    }
    let folder = match profile.map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => format!("BeeperTexts-{p}"),
        None => "BeeperTexts".to_string(),
    };
    match os_name {
        "macos" | "darwin" => home_dir
            .join("Library")
            .join("Application Support")
            .join(folder),
        "windows" | "win32" => env_vars
            .get("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Roaming"))
            .join(folder),
        "linux" => env_vars
            .get("XDG_CONFIG_HOME")
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join(".config"))
            .join(folder),
        other => panic!("unsupported os_name: {other}"),
    }
}

pub const SEEDED_BRIDGES: [(&str, &str, &str, BridgeMode); 6] = [
    ("whatsapp", "whatsapp", "WhatsApp", BridgeMode::OnDevice),
    ("imessage", "imessage", "iMessage", BridgeMode::OnDevice),
    ("telegram", "telegram", "Telegram", BridgeMode::Cloud),
    ("signal", "signal", "Signal", BridgeMode::OnDevice),
    ("slack", "slack", "Slack", BridgeMode::Cloud),
    ("discord", "discord", "Discord", BridgeMode::Cloud),
];

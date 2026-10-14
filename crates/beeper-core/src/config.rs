use crate::models::{BeeperSettings, LanguageChoice, NetworkKind, NetworkSpec, ThemeChoice};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const APP_BUNDLE_ID: &str = "com.automattic.beeper.desktop";
pub const APP_NAME: &str = "Beeper";
pub const APP_VERSION: &str = "4.3.176";
pub const DEFAULT_PAS_HOST: &str = "127.0.0.1";
pub const DEFAULT_PAS_PORT: u16 = 23374;
pub const DEFAULT_HOMESERVER: &str = "https://matrix.beeper.com";
pub const DEFAULT_API_URL: &str = "https://api.beeper.com";
pub const DEFAULT_USER_ID: &str = "@alex:beeper.com";
pub const DEFAULT_DEVICE_ID: &str = "BEEPER_RS_DEV01";
pub const DEFAULT_TOKEN: &str = "bp_tok_beeper_rs_default_secret";
pub const DEFAULT_RECOVERY_CODE: &str = "EsTc 9K4p Lm2Q Xv8N Beeper RsKey 2026";
pub const TRAY_ICON_SIZE: u32 = 32;
pub const WS_CHANNEL_CAPACITY: usize = 512;
pub const ISOLATED_DATA_FOLDER_PREFIX: &str = "BeeperRs";
pub const ORIGINAL_BEEPER_FOLDER_PREFIX: &str = "BeeperTexts";
pub const OPENAPI_SPEC_JSON: &str = include_str!("../assets/openapi.json");
pub const SEED_JSON: &str = include_str!("../assets/seed.json");
pub const CSS_VARIABLES: &str = include_str!("../assets/themes/variables.css");
pub const CSS_TEXTS: &str = include_str!("../assets/themes/texts.css");
pub const CSS_DRACULA: &str = include_str!("../assets/themes/dracula.css");
pub const CSS_NOCTURNE: &str = include_str!("../assets/themes/nocturne.css");
pub const CSS_DARK_PURPLE: &str = include_str!("../assets/themes/dark-purple.css");
pub const CSS_FIX_LOCKDOWN_EMOJIS: &str = include_str!("../assets/themes/fix-lockdown-mode-emojis.css");

#[derive(Debug, Clone)]
pub struct CentralConfig {
    pub data_dir: PathBuf,
    pub source_beeper_dir: Option<PathBuf>,
    pub db_path: PathBuf,
    pub assets_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub profile: Option<String>,
    pub settings: BeeperSettings,
    pub demo: bool,
    pub api_url: String,
    pub homeserver: String,
}

impl CentralConfig {
    pub fn new(data_dir: PathBuf, host: String, port: u16, profile: Option<String>) -> Self {
        assert!(!host.trim().is_empty(), "host must not be empty");
        assert!(port > 0, "port must be positive");
        assert_not_original_beeper_dir(&data_dir);
        let settings = BeeperSettings { pas_port: port, ..BeeperSettings::default() };
        Self {
            db_path: data_dir.join("index.db"),
            assets_dir: data_dir.join("media"),
            data_dir,
            source_beeper_dir: None,
            host,
            port,
            profile,
            settings,
            demo: false,
            api_url: DEFAULT_API_URL.into(),
            homeserver: DEFAULT_HOMESERVER.into(),
        }
    }

    pub fn with_source_beeper_dir(mut self, source_dir: Option<PathBuf>) -> Self {
        assert!(source_dir.as_ref() != Some(&self.data_dir), "source_beeper_dir must differ from data_dir");
        self.source_beeper_dir = source_dir;
        self
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

impl Default for BeeperSettings {
    fn default() -> Self {
        Self {
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
            enable_pas_api: false,
            pas_port: DEFAULT_PAS_PORT,
            launch_at_startup: false,
            minimize_to_tray: true,
            incognito_mode: false,
            archive_after_reminder: true,
            mark_read_on_archive: true,
            after_archive: 0,
            remind_later_default: 0,
            compact_chat_list: true,
        }
    }
}

const fn net(
    kind: NetworkKind,
    bridge_id: &'static str,
    bridge_type: &'static str,
    display_name: &'static str,
    brand_rgb: [u8; 3],
    [supports_on_device, supports_cloud, supports_sms_mms, supports_e2ee, supports_polls]: [bool; 5],
) -> NetworkSpec {
    NetworkSpec {
        kind,
        bridge_id,
        bridge_type,
        display_name,
        brand_rgb,
        supports_on_device,
        supports_cloud,
        supports_sms_mms,
        supports_e2ee,
        supports_polls,
        supports_edit_history: matches!(kind, NetworkKind::IMessage),
    }
}

use NetworkKind::*;
const T: bool = true;
const F: bool = false;

#[rustfmt::skip]
pub const NETWORK_SPECS: [NetworkSpec; 17] = [
    net(Beeper, "hungryserv", "hungryserv", "Beeper", [37, 97, 251], [F, T, F, T, T]),
    net(WhatsApp, "whatsapp", "whatsapp", "WhatsApp", [37, 211, 102], [T, T, F, T, T]),
    net(Messenger, "facebook", "meta", "FB Messenger", [0, 132, 255], [T, T, F, T, T]),
    net(Instagram, "instagram", "meta", "Instagram", [225, 48, 108], [T, T, F, T, T]),
    net(Telegram, "telegram", "telegram", "Telegram", [38, 165, 228], [T, T, F, F, T]),
    net(Signal, "signal", "signal", "Signal", [58, 118, 240], [T, T, F, T, F]),
    net(Viber, "viber", "viber", "Viber", [115, 96, 242], [T, F, F, T, T]),
    net(IMessage, "imessage", "imessage", "iMessage", [52, 199, 89], [T, T, T, T, F]),
    net(GoogleMessages, "gmessages", "gmessages", "Google Messages", [26, 115, 232], [T, T, T, T, F]),
    net(GoogleVoice, "gvoice", "gvoice", "Google Voice", [15, 157, 88], [F, T, T, F, F]),
    net(GoogleChat, "googlechat", "googlechat", "Google Chat", [0, 172, 71], [F, T, F, F, F]),
    net(TwitterX, "twitter", "twitter", "Twitter / X", [29, 155, 240], [T, T, F, F, F]),
    net(Slack, "slack", "slack", "Slack", [224, 30, 90], [T, T, F, F, F]),
    net(Discord, "discord", "discord", "Discord", [88, 101, 242], [T, T, F, F, T]),
    net(LinkedIn, "linkedin", "linkedin", "LinkedIn", [10, 102, 194], [T, T, F, F, F]),
    net(Line, "line", "line", "LINE", [6, 199, 85], [T, F, F, T, F]),
    net(Tumblr, "tumblr", "tumblr", "Tumblr", [0, 25, 53], [T, F, F, F, F]),
];

pub fn network_spec(bridge_id: &str) -> Option<&'static NetworkSpec> {
    NETWORK_SPECS.iter().find(|n| n.bridge_id == bridge_id)
}

pub fn account_network(account_id: &str) -> &'static NetworkSpec {
    network_spec(account_id.strip_prefix("acc_").unwrap_or(account_id))
        .unwrap_or_else(|| panic!("account {account_id} has no known network"))
}

pub fn is_original_beeper_dir(path: &Path) -> bool {
    path.components().any(|comp| {
        let lower = comp.as_os_str().to_string_lossy().to_ascii_lowercase();
        lower == "beepertexts" || lower.starts_with("beepertexts-")
    })
}

pub fn assert_not_original_beeper_dir(path: &Path) {
    assert!(!is_original_beeper_dir(path), "refusing to write to original BeeperTexts directory: {path:?}");
}

fn os_base_config_dir(os_name: &str, env_vars: &HashMap<String, String>, home_dir: &Path) -> PathBuf {
    let env_or = |key: &str, rel: &[&str]| {
        env_vars.get(key).map_or_else(|| rel.iter().fold(home_dir.to_path_buf(), |p, s| p.join(s)), PathBuf::from)
    };
    match os_name {
        "darwin" => home_dir.join("Library").join("Application Support"),
        "win32" => env_or("APPDATA", &["AppData", "Roaming"]),
        "linux" => env_or("XDG_CONFIG_HOME", &[".config"]),
        other => panic!("unsupported os_name: {other}"),
    }
}

fn profile_folder(prefix: &str, profile: Option<&str>) -> String {
    profile.map_or_else(|| prefix.to_string(), |p| format!("{prefix}-{p}"))
}

pub fn resolve_original_data_dir_for_os(
    os_name: &str,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    env_vars.get("BEEPER_ORIGINAL_DATA_DIR").map_or_else(
        || os_base_config_dir(os_name, env_vars, home_dir).join(profile_folder(ORIGINAL_BEEPER_FOLDER_PREFIX, profile)),
        PathBuf::from,
    )
}

pub fn resolve_data_dir_for_os(
    os_name: &str,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    let dir = env_vars.get("BEEPER_RS_DATA_DIR").map_or_else(
        || os_base_config_dir(os_name, env_vars, home_dir).join(profile_folder(ISOLATED_DATA_FOLDER_PREFIX, profile)),
        PathBuf::from,
    );
    assert_not_original_beeper_dir(&dir);
    dir
}

const SKIPPED_BEEPER_ENTRIES: [&str; 13] = [
    "Cache",
    "Code Cache",
    "GPUCache",
    "DawnCache",
    "Crashpad",
    "IndexedDB",
    "Local Storage",
    "Session Storage",
    "Service Worker",
    "blob_storage",
    "Shared Dictionary",
    "Network",
    "Dictionaries",
];

pub fn copy_beeper_data_to_isolated_dir(source_dir: &Path, target_dir: &Path) -> usize {
    assert_not_original_beeper_dir(target_dir);
    assert_ne!(source_dir, target_dir, "source_dir and target_dir must be distinct");
    assert!(!target_dir.starts_with(source_dir), "target_dir must not be inside source_dir");
    std::fs::create_dir_all(target_dir).expect("create isolated target_dir");
    let mut copied = 0;
    for entry in std::fs::read_dir(source_dir).expect("read source_dir") {
        let entry = entry.expect("read source_dir entry");
        let name = entry.file_name().into_string().expect("UTF-8 file name in Beeper data dir");
        if name.ends_with("-shm")
            || name.ends_with(".lock")
            || name.starts_with("Singleton")
            || SKIPPED_BEEPER_ENTRIES.contains(&name.as_str())
        {
            continue;
        }
        let (src, dst) = (entry.path(), target_dir.join(&name));
        if entry.file_type().expect("file type").is_dir() {
            copied += copy_beeper_data_to_isolated_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy file to isolated dir");
            copied += 1;
        }
    }
    copied
}

fn unix_now() -> std::time::Duration {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("system clock before UNIX_EPOCH")
}

pub fn now_unix_secs() -> i64 {
    unix_now().as_secs() as i64
}

pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    (yoe as i64 + era * 400 + i64::from(m <= 2), m, d)
}

pub fn now_rfc3339_millis() -> String {
    let now = unix_now();
    let secs = now.as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", sod / 3600, sod % 3600 / 60, sod % 60, now.subsec_millis())
}

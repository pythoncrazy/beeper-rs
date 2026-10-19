use crate::models::{BeeperSettings, NetworkKind, NetworkSpec, ThemeChoice};
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
pub const WS_CHANNEL_CAPACITY: usize = 512;
pub const DATA_FOLDER_PREFIX: &str = "beeper-rs";
const LEGACY_DATA_FOLDER_PREFIX: &str = "BeeperRs";
const LEGACY_DATA_FILES: [&str; 4] = ["beeper-rs.db", "beeper-rs.db-wal", "beeper-rs.db-shm", "media"];
pub const ORIGINAL_BEEPER_FOLDER_PREFIX: &str = "BeeperTexts";
pub const OPENAPI_SPEC_JSON: &str = include_str!("../assets/openapi.json");
pub const SEED_JSON: &str = include_str!("../assets/seed.json");

#[derive(Debug, Clone)]
pub struct CentralConfig {
    pub data_dir: PathBuf,
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
            db_path: data_dir.join("beeper-rs.db"),
            assets_dir: data_dir.join("media"),
            data_dir,
            host,
            port,
            profile,
            settings,
            demo: false,
            api_url: DEFAULT_API_URL.into(),
            homeserver: DEFAULT_HOMESERVER.into(),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

impl Default for BeeperSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            zoom_percent: 100,
            enable_pas_api: false,
            pas_port: DEFAULT_PAS_PORT,
            prefs: Default::default(),
            hotkeys_off: Default::default(),
            reminders_order: 0,
            microphone: String::new(),
            accounts: Default::default(),
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
pub const NETWORK_SPECS: [NetworkSpec; 20] = [
    net(Beeper, "hungryserv", "hungryserv", "Beeper", [37, 97, 251], [F, T, F, T, T]),
    net(WhatsApp, "whatsapp", "whatsapp", "WhatsApp", [37, 211, 102], [T, T, F, T, T]),
    net(TwitterX, "twitter", "twitter", "X", [29, 155, 240], [T, T, F, F, F]),
    net(Telegram, "telegram", "telegram", "Telegram", [38, 165, 228], [T, T, F, F, T]),
    net(Slack, "slack", "slack", "Slack", [224, 30, 90], [T, T, F, F, F]),
    net(Discord, "discord", "discord", "Discord", [88, 101, 242], [T, T, F, F, T]),
    net(Messenger, "facebook", "meta", "Facebook", [0, 132, 255], [T, T, F, T, T]),
    net(Instagram, "instagram", "meta", "Instagram", [225, 48, 108], [T, T, F, T, T]),
    net(LinkedIn, "linkedin", "linkedin", "LinkedIn", [10, 102, 194], [T, T, F, F, F]),
    net(GoogleMessages, "gmessages", "gmessages", "Google Messages", [26, 115, 232], [T, T, T, T, F]),
    net(Signal, "signal", "signal", "Signal", [58, 118, 240], [T, T, F, T, F]),
    net(GoogleChat, "googlechat", "googlechat", "Google Chat", [0, 172, 71], [F, T, F, F, F]),
    net(Line, "line", "line", "LINE", [6, 199, 85], [T, F, F, T, F]),
    net(GoogleVoice, "gvoice", "gvoice", "Google Voice", [15, 157, 88], [F, T, T, F, F]),
    net(Tumblr, "tumblr", "tumblr", "Tumblr", [0, 25, 53], [T, F, F, F, F]),
    net(Viber, "viber", "viber", "Viber", [115, 96, 242], [T, F, F, T, T]),
    net(Snapchat, "snapchat", "snapchat", "Snapchat", [255, 252, 0], [F, T, F, F, F]),
    net(WeChat, "wechat", "wechat", "WeChat", [7, 193, 96], [T, T, F, F, F]),
    net(IMessage, "imessage", "imessage", "iMessage", [52, 199, 89], [T, T, T, T, F]),
    net(Bluesky, "bluesky", "bluesky", "Bluesky", [0, 133, 255], [T, F, F, F, F]),
];

pub fn network_spec(bridge_id: &str) -> Option<&'static NetworkSpec> {
    NETWORK_SPECS.iter().find(|n| n.bridge_id == bridge_id)
}

const NETWORK_ALIASES: [(&str, &str); 4] =
    [("messenger", "facebook"), ("imsg", "imessage"), ("matrix", "hungryserv"), ("beeper", "hungryserv")];

pub fn account_network(account_id: &str) -> &'static NetworkSpec {
    let id = account_id.to_ascii_lowercase();
    let id = ["acc_", "local-", "bridge-"].iter().fold(id.as_str(), |s, p| s.strip_prefix(p).unwrap_or(s));
    let id = NETWORK_ALIASES.iter().find(|(alias, _)| id.starts_with(alias)).map_or(id, |(_, bridge)| bridge);
    NETWORK_SPECS.iter().find(|n| id.starts_with(n.bridge_id)).unwrap_or(&NETWORK_SPECS[0])
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
        || os_base_config_dir(os_name, env_vars, home_dir).join(profile_folder(DATA_FOLDER_PREFIX, profile)),
        PathBuf::from,
    );
    assert_not_original_beeper_dir(&dir);
    dir
}

pub fn migrate_legacy_data_dir(data_dir: &Path, profile: Option<&str>) {
    let legacy = data_dir.with_file_name(profile_folder(LEGACY_DATA_FOLDER_PREFIX, profile));
    if !legacy.is_dir() || legacy.file_name() == data_dir.file_name() {
        return;
    }
    assert!(
        !data_dir.exists(),
        "{} and {} both exist. Remove the one you do not use.",
        legacy.display(),
        data_dir.display()
    );
    std::fs::create_dir_all(data_dir).unwrap_or_else(|e| panic!("create {}: {e}", data_dir.display()));
    let keep = legacy.join("beeper-rs.db").is_file();
    for name in LEGACY_DATA_FILES.iter().filter(|_| keep).map(|n| legacy.join(n)).filter(|p| p.exists()) {
        let to = data_dir.join(name.file_name().expect("file name"));
        std::fs::rename(&name, &to).unwrap_or_else(|e| panic!("move {} to {}: {e}", name.display(), to.display()));
    }
    std::fs::remove_dir_all(&legacy).unwrap_or_else(|e| panic!("remove {}: {e}", legacy.display()));
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

pub fn rfc3339_from_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", sod / 3600, sod % 3600 / 60, sod % 60, ms.rem_euclid(1000))
}

pub fn millis_from_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let sep = |i: usize, c: u8| (b.get(i) == Some(&c)).then_some(());
    sep(4, b'-')?;
    sep(7, b'-')?;
    sep(13, b':')?;
    sep(16, b':')?;
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (yy, mm) = (y - i64::from(m <= 2), (m + 9) % 12);
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mm + 2) / 5 + d - 1 - 719_468;
    let frac = s.get(19..).filter(|f| f.starts_with('.')).map_or(Some(0), |f| {
        let digits: String = f[1..].chars().take_while(char::is_ascii_digit).chain("000".chars()).take(3).collect();
        digits.parse().ok()
    })?;
    Some(((days * 24 + num(11..13)?) * 60 + num(14..16)?) * 60_000 + num(17..19)? * 1000 + frac)
}

/// Shifts Unix milliseconds by the local UTC offset at that instant, so day and clock math shows local time.
pub fn local_millis(utc_ms: i64) -> i64 {
    use chrono::{Offset, TimeZone};
    let at = chrono::DateTime::from_timestamp_millis(utc_ms).expect("timestamp in range").naive_utc();
    utc_ms + i64::from(chrono::Local.offset_from_utc_datetime(&at).fix().local_minus_utc()) * 1000
}

pub fn now_unix_millis() -> i64 {
    unix_now().as_millis() as i64
}

pub fn now_rfc3339_millis() -> String {
    rfc3339_from_millis(now_unix_millis())
}

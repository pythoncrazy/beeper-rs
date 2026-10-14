use crate::models::{BeeperSettings, LanguageChoice, NetworkKind, NetworkSpec, ThemeChoice};
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
            enable_pas_api: true,
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

pub fn url_encode(input: &str) -> String {
    input
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("percent escape is UTF-8");
                out.push(u8::from_str_radix(hex, 16).expect("valid percent-encoded byte"));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).expect("valid utf8 in url_decode")
}

fn md_pad(data: &[u8]) -> Vec<u8> {
    let mut msg = data.to_vec();
    msg.push(0x80);
    msg.resize(msg.len() + (64 + 56 - msg.len() % 64) % 64, 0);
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    msg
}

fn be_words<const N: usize>(chunk: &[u8; 64]) -> [u32; N] {
    let mut w = [0u32; N];
    for (i, c) in chunk.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*c);
    }
    w
}

fn be_bytes<const W: usize, const B: usize>(state: [u32; W]) -> [u8; B] {
    let mut out = [0u8; B];
    for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *o = s.to_be_bytes();
    }
    out
}

#[rustfmt::skip]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = [0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for chunk in md_pad(data).as_chunks::<64>().0 {
        let mut w = be_words::<64>(chunk);
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let [a, b, c, d, e, f, g, h] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t1 = h.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (s, x) in state.iter_mut().zip(v) {
            *s = s.wrapping_add(x);
        }
    }
    be_bytes(state)
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    for chunk in md_pad(data).as_chunks::<64>().0 {
        let mut w = be_words::<80>(chunk);
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut v = state;
        for (i, wi) in w.into_iter().enumerate() {
            let [a, b, c, d, e] = v;
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5a827999),
                1 => (b ^ c ^ d, 0x6ed9eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            v = [
                a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi),
                a,
                b.rotate_left(30),
                c,
                d,
            ];
        }
        for (s, x) in state.iter_mut().zip(v) {
            *s = s.wrapping_add(x);
        }
    }
    be_bytes(state)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode_with(data: &[u8], url: bool) -> String {
    let sym = |v: u32| match (B64[v as usize & 63], url) {
        (b'+', true) => '-',
        (b'/', true) => '_',
        (c, _) => c as char,
    };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(sym(n >> (18 - 6 * i)));
        }
        if !url {
            out.push_str(&"=="[..3 - chunk.len()]);
        }
    }
    out
}

pub fn base64_encode_standard(data: &[u8]) -> String {
    base64_encode_with(data, false)
}

pub fn base64_encode_url_nopad(data: &[u8]) -> String {
    base64_encode_with(data, true)
}

pub fn base64_decode_standard(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for b in input.bytes().filter(|&b| b != b'=') {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            other => panic!("invalid base64 byte: {other}"),
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

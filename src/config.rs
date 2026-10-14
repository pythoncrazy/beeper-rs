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
pub const ISOLATED_DATA_FOLDER_PREFIX: &str = "BeeperRs";
pub const ORIGINAL_BEEPER_FOLDER_PREFIX: &str = "BeeperTexts";
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
        let db_path = data_dir.join("index.db");
        let assets_dir = data_dir.join("media");
        let mut settings = Self::default_settings();
        settings.pas_port = port;
        Self {
            data_dir,
            source_beeper_dir: None,
            db_path,
            assets_dir,
            host,
            port,
            profile,
            settings,
        }
    }

    pub fn with_source_beeper_dir(mut self, source_dir: Option<PathBuf>) -> Self {
        if let Some(ref src) = source_dir {
            assert_ne!(
                src, &self.data_dir,
                "source_beeper_dir must differ from isolated data_dir"
            );
        }
        self.source_beeper_dir = source_dir;
        self
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
    badge_code: &'static str,
    brand_rgb: [u8; 3],
    flags: [bool; 5],
) -> NetworkSpec {
    NetworkSpec {
        kind,
        bridge_id,
        bridge_type,
        display_name,
        badge_code,
        brand_rgb,
        supports_on_device: flags[0],
        supports_cloud: flags[1],
        supports_sms_mms: flags[2],
        supports_e2ee: flags[3],
        supports_polls: flags[4],
        supports_edit_history: matches!(kind, NetworkKind::IMessage),
    }
}

pub const NETWORK_SPECS: [NetworkSpec; 16] = [
    net(NetworkKind::WhatsApp, "whatsapp", "whatsapp", "WhatsApp", "WA", [37, 211, 102], [true, true, false, true, true]),
    net(NetworkKind::Messenger, "facebook", "meta", "FB Messenger", "FB", [0, 132, 255], [true, true, false, true, true]),
    net(NetworkKind::Instagram, "instagram", "meta", "Instagram", "IG", [225, 48, 108], [true, true, false, true, true]),
    net(NetworkKind::Telegram, "telegram", "telegram", "Telegram", "TG", [38, 165, 228], [true, true, false, false, true]),
    net(NetworkKind::Signal, "signal", "signal", "Signal", "SG", [58, 118, 240], [true, true, false, true, false]),
    net(NetworkKind::Viber, "viber", "viber", "Viber", "VB", [115, 96, 242], [true, false, false, true, true]),
    net(NetworkKind::IMessage, "imessage", "imessage", "iMessage", "iM", [52, 199, 89], [true, true, true, true, false]),
    net(NetworkKind::GoogleMessages, "gmessages", "gmessages", "Google Messages", "GM", [26, 115, 232], [true, true, true, true, false]),
    net(NetworkKind::GoogleVoice, "gvoice", "gvoice", "Google Voice", "GV", [15, 157, 88], [false, true, true, false, false]),
    net(NetworkKind::GoogleChat, "googlechat", "googlechat", "Google Chat", "GC", [0, 172, 71], [false, true, false, false, false]),
    net(NetworkKind::TwitterX, "twitter", "twitter", "Twitter / X", "X", [29, 155, 240], [true, true, false, false, false]),
    net(NetworkKind::Slack, "slack", "slack", "Slack", "SL", [224, 30, 90], [true, true, false, false, false]),
    net(NetworkKind::Discord, "discord", "discord", "Discord", "DC", [88, 101, 242], [true, true, false, false, true]),
    net(NetworkKind::LinkedIn, "linkedin", "linkedin", "LinkedIn", "IN", [10, 102, 194], [true, true, false, false, false]),
    net(NetworkKind::Line, "line", "line", "LINE", "LN", [6, 199, 85], [true, false, false, true, false]),
    net(NetworkKind::Tumblr, "tumblr", "tumblr", "Tumblr", "TB", [0, 25, 53], [true, false, false, false, false]),
];

pub fn normalize_bridge_id(raw: &str) -> &'static str {
    let lower = raw.trim().to_ascii_lowercase();
    let stripped = lower
        .strip_prefix("sh-")
        .or_else(|| lower.strip_prefix("mautrix-"))
        .unwrap_or(&lower);
    let clean = stripped
        .strip_suffix("-go")
        .or_else(|| stripped.strip_suffix("go"))
        .unwrap_or(stripped);
    match clean {
        "whatsapp" | "wa" => "whatsapp",
        "facebook" | "messenger" | "fb" | "meta" => "facebook",
        "instagram" | "ig" => "instagram",
        "telegram" | "tg" => "telegram",
        "signal" | "sig" => "signal",
        "viber" => "viber",
        "imessage" | "imessagecloud" | "imessage-cloud" | "apple" => "imessage",
        "gmessages" | "googlemessages" | "androidsms" | "sms" | "rcs" => "gmessages",
        "gvoice" | "googlevoice" => "gvoice",
        "googlechat" | "gchat" => "googlechat",
        "twitter" | "x" | "bluesky" | "bsky" => "twitter",
        "slack" => "slack",
        "discord" => "discord",
        "linkedin" => "linkedin",
        "line" => "line",
        "tumblr" | "hungryserv" | "matrix" | "beeper" => "tumblr",
        _ => {
            if let Some(spec) = NETWORK_SPECS.iter().find(|n| lower.contains(n.bridge_id)) {
                spec.bridge_id
            } else {
                "whatsapp"
            }
        }
    }
}

pub fn network_spec_by_bridge(bridge_id: &str) -> &'static NetworkSpec {
    if let Some(found) = NETWORK_SPECS.iter().find(|n| n.bridge_id == bridge_id) {
        return found;
    }
    let norm = normalize_bridge_id(bridge_id);
    NETWORK_SPECS
        .iter()
        .find(|n| n.bridge_id == norm)
        .expect("normalized bridge_id in NETWORK_SPECS")
}

pub fn is_original_beeper_dir(path: &Path) -> bool {
    path.components().any(|comp| {
        if let Some(s) = comp.as_os_str().to_str() {
            let lower = s.to_ascii_lowercase();
            lower == "beepertexts" || lower.starts_with("beepertexts-")
        } else {
            false
        }
    })
}

pub fn assert_not_original_beeper_dir(path: &Path) {
    assert!(
        !is_original_beeper_dir(path),
        "refusing to write to original BeeperTexts directory: {:?}",
        path
    );
}

fn os_base_config_dir(
    os_name: &str,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    match os_name {
        "macos" | "darwin" => home_dir.join("Library").join("Application Support"),
        "windows" | "win32" => env_vars
            .get("APPDATA")
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Roaming")),
        "linux" => env_vars
            .get("XDG_CONFIG_HOME")
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join(".config")),
        other => panic!("unsupported os_name: {other}"),
    }
}

pub fn resolve_original_data_dir_for_os(
    os_name: &str,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    if let Some(override_dir) = env_vars.get("BEEPER_ORIGINAL_DATA_DIR") {
        let trimmed = override_dir.trim();
        assert!(
            !trimmed.is_empty(),
            "BEEPER_ORIGINAL_DATA_DIR must not be empty"
        );
        return PathBuf::from(trimmed);
    }
    let folder = match profile.map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => format!("{ORIGINAL_BEEPER_FOLDER_PREFIX}-{p}"),
        None => ORIGINAL_BEEPER_FOLDER_PREFIX.to_string(),
    };
    os_base_config_dir(os_name, env_vars, home_dir).join(folder)
}

pub fn resolve_data_dir_for_os(
    os_name: &str,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    if let Some(override_dir) = env_vars
        .get("BEEPER_RS_DATA_DIR")
        .or_else(|| env_vars.get("BEEPER_USER_DATA_DIR"))
    {
        let trimmed = override_dir.trim();
        assert!(
            !trimmed.is_empty(),
            "BEEPER_USER_DATA_DIR must not be empty"
        );
        let candidate = PathBuf::from(trimmed);
        assert_not_original_beeper_dir(&candidate);
        return candidate;
    }
    let folder = match profile.map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => format!("{ISOLATED_DATA_FOLDER_PREFIX}-{p}"),
        None => ISOLATED_DATA_FOLDER_PREFIX.to_string(),
    };
    let resolved = os_base_config_dir(os_name, env_vars, home_dir).join(folder);
    assert_not_original_beeper_dir(&resolved);
    resolved
}

pub fn copy_beeper_data_to_isolated_dir(source_dir: &Path, target_dir: &Path) -> usize {
    assert_not_original_beeper_dir(target_dir);
    assert_ne!(
        source_dir, target_dir,
        "source_dir and target_dir must be distinct"
    );
    assert!(
        !target_dir.starts_with(source_dir),
        "target_dir must not be inside source_dir"
    );
    if !source_dir.exists() {
        return 0;
    }
    std::fs::create_dir_all(target_dir).expect("create isolated target_dir");
    let mut copied = 0;
    let entries = std::fs::read_dir(source_dir).expect("read source_dir");
    for entry in entries.flatten() {
        let src_path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.ends_with("-shm")
            || name_str.ends_with(".lock")
            || name_str.starts_with("Singleton")
            || matches!(
                name_str.as_ref(),
                "Cache"
                    | "Code Cache"
                    | "GPUCache"
                    | "DawnCache"
                    | "Crashpad"
                    | "IndexedDB"
                    | "Local Storage"
                    | "Session Storage"
                    | "Service Worker"
                    | "blob_storage"
                    | "Shared Dictionary"
                    | "Network"
                    | "Dictionaries"
            )
        {
            continue;
        }
        let dst_path = target_dir.join(&file_name);
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                copied += copy_beeper_data_to_isolated_dir(&src_path, &dst_path);
            } else if ft.is_file() {
                std::fs::copy(&src_path, &dst_path).expect("copy file to isolated dir");
                copied += 1;
            }
        }
    }
    copied
}

pub const SEEDED_BRIDGES: [(&str, &str, &str, BridgeMode); 6] = [
    ("whatsapp", "whatsapp", "WhatsApp", BridgeMode::OnDevice),
    ("imessage", "imessage", "iMessage", BridgeMode::OnDevice),
    ("telegram", "telegram", "Telegram", BridgeMode::Cloud),
    ("signal", "signal", "Signal", BridgeMode::OnDevice),
    ("instagram", "meta", "Instagram", BridgeMode::Cloud),
    ("slack", "slack", "Slack", BridgeMode::Cloud),
];

pub fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH")
        .as_secs() as i64
}

pub fn now_rfc3339_millis() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH");
    let total_secs = dur.as_secs() as i64;
    let millis = dur.subsec_millis();
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("valid hex utf8");
            let val = u8::from_str_radix(hex, 16).expect("valid percent-encoded byte");
            out.push(val);
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("valid utf8 in url_decode")
}

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
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999u32),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdcu32),
                _ => (b ^ c ^ d, 0xca62c1d6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64_encode_with(data: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 6) & 0x3F) as usize] as char);
        out.push(alphabet[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        if pad {
            out.push_str("==");
        }
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 6) & 0x3F) as usize] as char);
        if pad {
            out.push('=');
        }
    }
    out
}

pub fn base64_encode_standard(data: &[u8]) -> String {
    base64_encode_with(data, B64_STD, true)
}

pub fn base64_encode_url_nopad(data: &[u8]) -> String {
    base64_encode_with(data, B64_URL, false)
}

pub fn base64_decode_standard(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            other => panic!("invalid base64 byte: {other}"),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    out
}

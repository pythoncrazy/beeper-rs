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
            keybindings: Default::default(),
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

/// Desktop's `K0` for its time suggestions and typed times: chrono-node with forward dates, which tries the text and
/// then "in " and the text, and compromise for "this weekend", which starts on Saturday at midnight. Like chrono-node,
/// a weekday falls at noon, and a relative day keeps the time of `now`.
pub fn parse_when<Tz: chrono::TimeZone>(text: &str, now: &chrono::DateTime<Tz>) -> Option<chrono::DateTime<Tz>> {
    use chrono::{Datelike, Days, Months, NaiveDateTime, NaiveTime, TimeDelta};
    const WEEKDAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
    const NUMBERS: [&str; 13] =
        ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
    let text = text.trim().to_lowercase();
    let mut w: Vec<String> = text.split_whitespace().map(String::from).collect();
    if w.len() > 1 && matches!(w.last().map(String::as_str), Some("am" | "pm")) {
        let half = w.pop()?;
        w.last_mut()?.push_str(&half);
    }
    let clock = |s: &str, at: bool| -> Option<(u32, u32)> {
        let (s, pm) = match (s.strip_suffix("am"), s.strip_suffix("pm")) {
            (Some(s), _) => (s, Some(false)),
            (_, Some(s)) => (s, Some(true)),
            _ => (s, None),
        };
        let (h, m) = match s.split_once(':') {
            Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
            Some(_) => return None,
            None => (s.parse::<u32>().ok()?, 0),
        };
        let h = match pm {
            Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
            Some(_) => return None,
            None if at || s.contains(':') => h,
            None => return None,
        };
        (h < 24 && m < 60).then_some((h, m))
    };
    let at_word = |w: &[String]| w.len() > 1 && w[w.len() - 2] == "at";
    let mut hm = w.last().and_then(|s| match s.as_str() {
        "noon" => Some((12, 0)),
        "midnight" => Some((0, 0)),
        s => clock(s, at_word(&w)),
    });
    if hm.is_some() {
        if at_word(&w) {
            w.pop();
        }
        w.pop();
    }
    let part = match w.last().map(String::as_str) {
        Some("morning") => Some(6),
        Some("afternoon") => Some(15),
        Some("evening") => Some(20),
        Some("night" | "tonight") => Some(22),
        _ => None,
    };
    if let Some(h) = part {
        if w.pop().as_deref() == Some("tonight") || w == ["this"] {
            w.clear();
        }
        hm = Some(match hm {
            Some((h0, m)) if h >= 12 && h0 < 12 => (h0 + 12, m),
            Some(t) => t,
            None => (h, 0),
        });
    }
    let base = now.naive_local();
    let today = base.date();
    let wd = today.weekday().num_days_from_sunday();
    let at_time =
        |d: chrono::NaiveDate, (h, m): (u32, u32)| d.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("a clock"));
    let add = |n: u32, unit: &str| -> Option<NaiveDateTime> {
        match unit.strip_suffix('s').unwrap_or(unit) {
            "second" => base.checked_add_signed(TimeDelta::seconds(n.into())),
            "minute" => base.checked_add_signed(TimeDelta::minutes(n.into())),
            "hour" => base.checked_add_signed(TimeDelta::hours(n.into())),
            "day" => base.checked_add_days(Days::new(n.into())),
            "week" => base.checked_add_days(Days::new(7 * u64::from(n))),
            "month" => base.checked_add_months(Months::new(n)),
            "year" => base.checked_add_months(Months::new(n.checked_mul(12)?)),
            _ => None,
        }
    };
    let weekday = |s: &str| WEEKDAYS.iter().position(|d| *d == s).map(|t| t as u32);
    let words: Vec<&str> = w.iter().map(String::as_str).collect();
    let at = match words[..] {
        [] => {
            let at = at_time(today, hm?);
            if at > base {
                at
            } else {
                at.checked_add_days(Days::new(1))?
            }
        }
        ["today"] => hm.map_or(base, |t| at_time(today, t)),
        ["tomorrow" | "tmr"] => {
            let day = base.checked_add_days(Days::new(1))?;
            hm.map_or(day, |t| at_time(day.date(), t))
        }
        ["this", "weekend"] | ["weekend"] => {
            let sat = today.checked_add_signed(TimeDelta::days(if wd == 0 { -1 } else { 6 - i64::from(wd) }))?;
            at_time(sat, hm.unwrap_or((0, 0)))
        }
        ["next", unit @ ("week" | "month" | "year")] => {
            let day = add(1, unit)?;
            hm.map_or(day, |t| at_time(day.date(), t))
        }
        [d] | ["this", d] if weekday(d).is_some() => {
            let t = weekday(d)?;
            let at = at_time(today + Days::new(u64::from((t + 7 - wd) % 7)), hm.unwrap_or((12, 0)));
            if at > base {
                at
            } else {
                at.checked_add_days(Days::new(7))?
            }
        }
        ["next", d] if weekday(d).is_some() => {
            let (t, fwd) = (weekday(d)?, (weekday(d)? + 7 - wd) % 7);
            let days = match (wd, t) {
                (0, 0) | (6, 6) => 7,
                (0, t) => t,
                (6, 0) => 8,
                (6, t) => t + 1,
                (_, t) if t < wd && t != 0 => fwd,
                _ => fwd + 7,
            };
            at_time(today + Days::new(days.into()), hm.unwrap_or((12, 0)))
        }
        ["in", n, unit] | [n, unit] if hm.is_none() => {
            let n = match n {
                "a" | "an" => 1,
                n => NUMBERS.iter().position(|x| *x == n).map(|n| n as u32).or_else(|| n.parse().ok())?,
            };
            add(n, unit)?
        }
        _ => return None,
    };
    now.timezone().from_local_datetime(&at).earliest()
}

/// [`parse_when`] in local time, in Unix milliseconds.
pub fn parse_when_millis(text: &str, now_ms: i64) -> Option<i64> {
    let now =
        chrono::DateTime::from_timestamp_millis(now_ms).expect("timestamp in range").with_timezone(&chrono::Local);
    parse_when(text, &now).map(|at| at.timestamp_millis())
}

pub fn now_unix_millis() -> i64 {
    unix_now().as_millis() as i64
}

pub fn now_rfc3339_millis() -> String {
    rfc3339_from_millis(now_unix_millis())
}

#[cfg(test)]
mod tests {
    use super::parse_when;
    use chrono::{TimeZone, Utc};

    #[test]
    fn parse_when_reads_desktops_suggestions() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 15, 30, 0).unwrap();
        let when = |s: &str| parse_when(s, &now).map(|t| t.format("%a %F %R").to_string());
        let cases = [
            ("In an Hour", "Tue 2026-10-20 16:30"),
            ("In Two Hours", "Tue 2026-10-20 17:30"),
            ("This Evening", "Tue 2026-10-20 20:00"),
            ("Tomorrow", "Wed 2026-10-21 15:30"),
            ("This Weekend", "Sat 2026-10-24 00:00"),
            ("Next Week", "Tue 2026-10-27 15:30"),
            ("next month", "Fri 2026-11-20 15:30"),
            ("next year", "Wed 2027-10-20 15:30"),
            ("in a minute", "Tue 2026-10-20 15:31"),
            ("45 minutes", "Tue 2026-10-20 16:15"),
            ("3 weeks", "Tue 2026-11-10 15:30"),
            ("monday", "Mon 2026-10-26 12:00"),
            ("tuesday", "Tue 2026-10-27 12:00"),
            ("wednesday", "Wed 2026-10-21 12:00"),
            ("next monday", "Mon 2026-10-26 12:00"),
            ("next wednesday", "Wed 2026-10-28 12:00"),
            ("next sunday", "Sun 2026-11-01 12:00"),
            ("Monday morning at 9", "Mon 2026-10-26 09:00"),
            ("friday at 5pm", "Fri 2026-10-23 17:00"),
            ("tomorrow at 9:15 am", "Wed 2026-10-21 09:15"),
            ("tonight", "Tue 2026-10-20 22:00"),
            ("at 9", "Wed 2026-10-21 09:00"),
            ("noon", "Wed 2026-10-21 12:00"),
        ];
        for (text, at) in cases {
            assert_eq!(when(text).as_deref(), Some(at), "{text}");
        }
        for text in ["", "5", "soon", "at 25", "13pm", "in 2 hours at 9"] {
            assert_eq!(when(text), None, "{text}");
        }
        let sunday = Utc.with_ymd_and_hms(2026, 10, 25, 10, 0, 0).unwrap();
        let weekend = parse_when("This Weekend", &sunday).unwrap();
        assert!(weekend < sunday, "on Sunday this weekend has started, so Desktop drops the suggestion");
    }
}

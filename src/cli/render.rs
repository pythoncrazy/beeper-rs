//! The text output of the upstream CLI: its ink components (`lib/ink/components.js`), laid out like yoga lays out ink
//! boxes, and wrapped like `wrap-ansi`. Colors show only on a terminal, like chalk.

use super::{js_string, truthy, BIN};
use chrono::{DateTime, Datelike, Local, TimeZone};
use serde_json::{Map, Value};
use unicode_width::UnicodeWidthChar;

// The theme (`lib/ink/theme.js`), from the dark palette of Desktop.
const PRIMARY: &str = "#2561fb";
const PRIMARY_GLOW: &str = "#5a86ff";
const LINK: &str = "#5cadff";
const TEXT: &str = "#ededed";
const MUTED: &str = "#adadad";
const SUBTLE: &str = "#7e7e7e";
const MINE: &str = "#4cc38a";
const WARN: &str = "#f6ce46";
const WARN_ALT: &str = "#f1a10d";
const DANGER: &str = "#ff6369";
const MAGENTA: &str = "#912ce1";
const CYAN: &str = "#00c2d7";
const MENTION: &str = "#5a86ff";
const DRAFT: &str = "#f6ce46";

/// The tint of each network, from the `iconBackground` of the Desktop bridges.
const BRIDGE_TINTS: [(&str, &str); 23] = [
    ("imessage", "#19BA3B"),
    ("imessagecloud", "#19BA3B"),
    ("imessagego", "#19BA3B"),
    ("androidsms", "#19BA3B"),
    ("whatsapp", "#48C95F"),
    ("telegram", "#2EA4DB"),
    ("signal", "#3542FF"),
    ("discord", "#5865F2"),
    ("linkedin", "#086CE1"),
    ("twitter", "#202124"),
    ("x", "#202124"),
    ("bluesky", "#549B57"),
    ("beeper", "#0D4FFB"),
    ("beeperai", "#0D4FFB"),
    ("ai", "#0D4FFB"),
    ("instagram", "#d833ca"),
    ("facebook", "#0d4ffb"),
    ("messenger", "#0d4ffb"),
    ("googlechat", "#1ab5a2"),
    ("googlevoice", "#0eb3ef"),
    ("googlemessages", "#19ba3b"),
    ("slack", "#9745ea"),
    ("matrix", "#0eb3ef"),
];

const SENDER_COLORS: [&str; 8] =
    ["#63c174", "#f1a10d", "#f76190", "#bf7af0", "#00c2d7", "#f65cb6", "#849dff", "#0ac5b3"];

const RAIL: &str = "▎";

fn bridge_color(network: Option<&str>) -> Option<&'static str> {
    let key: String = network?.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect();
    BRIDGE_TINTS.iter().find(|t| t.0 == key).map(|t| t.1)
}

/// The group sender color: the Java string hash of the UTF-16 units of the ID.
fn sender_color(id: Option<&str>) -> &'static str {
    let Some(id) = id else { return TEXT };
    let hash = id.encode_utf16().fold(0i32, |h, c| h.wrapping_shl(5).wrapping_sub(h).wrapping_add(i32::from(c)));
    SENDER_COLORS[((hash as i64).abs() % 8) as usize]
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct St {
    color: Option<&'static str>,
    bold: bool,
    italic: bool,
}

/// An ink node: a text with its colored parts, a `flexGrow` spacer, or a box with its direction and margins.
#[derive(Clone)]
pub enum El {
    Text(Vec<(String, St)>),
    Grow,
    Empty,
    Box { col: bool, ml: usize, mt: usize, mb: usize, kids: Vec<El> },
}

impl El {
    fn ml(self, n: usize) -> Self {
        match self {
            El::Box { col, mt, mb, kids, .. } => El::Box { col, ml: n, mt, mb, kids },
            _ => unreachable!("only a box has margins"),
        }
    }
    fn mt(self, n: usize) -> Self {
        match self {
            El::Box { col, ml, mb, kids, .. } => El::Box { col, ml, mt: n, mb, kids },
            _ => unreachable!("only a box has margins"),
        }
    }
    fn mb(self, n: usize) -> Self {
        match self {
            El::Box { col, ml, mt, kids, .. } => El::Box { col, ml, mt, mb: n, kids },
            _ => unreachable!("only a box has margins"),
        }
    }
}

fn t(s: impl Into<String>, color: &'static str) -> El {
    El::Text(vec![(s.into(), St { color: Some(color), ..St::default() })])
}

fn tb(s: impl Into<String>, color: &'static str) -> El {
    El::Text(vec![(s.into(), St { color: Some(color), bold: true, ..St::default() })])
}

fn plain(s: &str) -> El {
    El::Text(vec![(s.into(), St::default())])
}

fn row(kids: Vec<El>) -> El {
    El::Box { col: false, ml: 0, mt: 0, mb: 0, kids }
}

fn col(kids: Vec<El>) -> El {
    El::Box { col: true, ml: 0, mt: 0, mb: 0, kids }
}

fn when(cond: bool, el: impl FnOnce() -> El) -> El {
    if cond {
        el()
    } else {
        El::Empty
    }
}

type Cell = (char, St);

fn width(cells: &[Cell]) -> usize {
    cells.iter().map(|c| c.0.width().unwrap_or(0)).sum()
}

/// `wrap-ansi` with `{trim: false, hard: true}`, the wrap of ink, for one line.
fn wrap_line(line: &[Cell], cols: f32) -> Vec<Vec<Cell>> {
    let mut rows: Vec<Vec<Cell>> = vec![Vec::new()];
    for (i, word) in line.split(|c| c.0 == ' ').enumerate() {
        let mut row_len = width(rows.last().expect("a row")) as f32;
        if i != 0 {
            if row_len >= cols {
                rows.push(Vec::new());
                row_len = 0.0;
            }
            rows.last_mut().expect("a row").push((' ', St::default()));
            row_len += 1.0;
        }
        let len = width(word) as f32;
        if len > cols {
            let this_line = 1.0 + ((len - (cols - row_len) - 1.0) / cols).floor();
            if ((len - 1.0) / cols).floor() < this_line {
                rows.push(Vec::new());
            }
            let mut visible = width(rows.last().expect("a row")) as f32;
            for (j, &c) in word.iter().enumerate() {
                let w = c.0.width().unwrap_or(0) as f32;
                match visible + w <= cols {
                    true => rows.last_mut().expect("a row").push(c),
                    false => (rows.push(vec![c]), visible = 0.0).1,
                }
                visible += w;
                if visible == cols && j < word.len() - 1 {
                    rows.push(Vec::new());
                    visible = 0.0;
                }
            }
            continue;
        }
        if row_len + len > cols && row_len > 0.0 && len > 0.0 {
            rows.push(Vec::new());
        }
        rows.last_mut().expect("a row").extend_from_slice(word);
    }
    rows
}

fn cells(spans: &[(String, St)]) -> Vec<Cell> {
    spans.iter().flat_map(|(s, st)| s.chars().map(move |c| (c, *st))).collect()
}

fn lines(spans: &[(String, St)]) -> Vec<Vec<Cell>> {
    cells(spans).split(|c| c.0 == '\n').map(<[Cell]>::to_vec).collect()
}

fn natural(spans: &[(String, St)]) -> f32 {
    lines(spans).iter().map(|l| width(l)).max().unwrap_or(0) as f32
}

fn wrapped(spans: &[(String, St)], cols: f32) -> Vec<Vec<Cell>> {
    lines(spans).iter().flat_map(|l| wrap_line(l, cols)).collect()
}

/// The width that yoga gives a node as a flex basis in a row of `avail` columns (ink's `measureTextNode`).
fn measure(el: &El, avail: f32) -> f32 {
    match el {
        El::Text(spans) => {
            let w = natural(spans);
            match w <= avail || (w >= 1.0 && avail > 0.0 && avail < 1.0) {
                true => w,
                false => wrapped(spans, avail).iter().map(|l| width(l)).max().unwrap_or(0) as f32,
            }
        }
        El::Grow | El::Empty => 0.0,
        El::Box { col, ml, kids, .. } => {
            let inner = avail - *ml as f32;
            let sizes = kids.iter().map(|k| measure(k, inner));
            *ml as f32 + if *col { sizes.fold(0.0, f32::max) } else { sizes.sum::<f32>().min(inner) }
        }
    }
}

/// Yoga's `roundValueToPixelGrid`.
fn snap(v: f32, ceil: bool, floor: bool) -> f32 {
    let f = v.rem_euclid(1.0);
    let near = |a: f32, b: f32| (a - b).abs() < 0.0001;
    let up = match () {
        _ if near(f, 0.0) => 0.0,
        _ if near(f, 1.0) => 1.0,
        _ if ceil => 1.0,
        _ if floor => 0.0,
        _ => f32::from(u8::from(f > 0.5 || near(f, 0.5))),
    };
    v - f + up
}

struct Grid(Vec<Vec<(String, St)>>);

impl Grid {
    fn write(&mut self, x: usize, y: usize, line: &[Cell]) {
        if self.0.len() <= y {
            self.0.resize_with(y + 1, Vec::new);
        }
        let row = &mut self.0[y];
        let mut x = x;
        for &(c, st) in line {
            let w = c.width().unwrap_or(0);
            if w == 0 {
                if let Some(prev) = x.checked_sub(1).and_then(|p| row.get_mut(p)) {
                    prev.0.push(c);
                }
                continue;
            }
            if row.len() < x + w {
                row.resize(x + w, (" ".into(), St::default()));
            }
            row[x] = (c.to_string(), st);
            (1..w).for_each(|i| row[x + i] = (String::new(), st));
            x += w;
        }
    }
}

/// Lays out `el` at `x` with `w` columns like yoga, writes it into `g`, and returns its height with its margins.
fn draw(el: &El, x: f32, y: usize, w: f32, g: &mut Grid) -> usize {
    match el {
        El::Grow | El::Empty => 0,
        El::Text(spans) => {
            let text = if natural(spans) > w { wrapped(spans, w) } else { lines(spans) };
            text.iter().enumerate().for_each(|(i, l)| g.write(x as usize, y + i, l));
            text.len()
        }
        El::Box { col: true, ml, mt, mb, kids } => {
            let (x, w) = (x + *ml as f32, w - *ml as f32);
            let h = kids.iter().fold(0, |h, k| h + draw(k, x, y + mt + h, w, g));
            mt + h + mb
        }
        El::Box { col: false, ml, mt, mb, kids } => {
            let (x, w) = (x + *ml as f32, w - *ml as f32);
            let bases: Vec<f32> = kids.iter().map(|k| measure(k, w)).collect();
            let sum: f32 = bases.iter().sum();
            let grows = kids.iter().filter(|k| matches!(k, El::Grow)).count() as f32;
            let sizes: Vec<f32> = kids
                .iter()
                .zip(&bases)
                .map(|(k, b)| match (sum > w, k) {
                    (true, _) => b * w / sum,
                    (false, El::Grow) => (w - sum) / grows,
                    (false, _) => *b,
                })
                .collect();
            let mut pos = x;
            let mut h = 0;
            for (k, size) in kids.iter().zip(sizes) {
                let (left, right) = match k {
                    El::Text(_) => {
                        let frac = size.rem_euclid(1.0);
                        let fractional = frac > 0.0001 && frac < 0.9999;
                        (snap(pos, false, true), snap(pos + size, fractional, !fractional))
                    }
                    _ => (snap(pos, false, false), snap(pos + size, false, false)),
                };
                h = h.max(draw(k, left, y + mt, right - left, g));
                pos += size;
            }
            mt + h + mb
        }
    }
}

/// The columns of the terminal on `fd` (1 for stdout, 2 for stderr), or 80 like ink when it is not a terminal.
pub fn columns(fd: i32) -> usize {
    #[cfg(target_os = "linux")]
    {
        #[repr(C)]
        struct WinSize([u16; 4]);
        unsafe extern "C" {
            fn ioctl(fd: i32, request: u64, ...) -> i32;
        }
        let mut size = WinSize([0; 4]);
        // SAFETY: TIOCGWINSZ writes one `winsize` into `size`, and fails on a file that is not a terminal.
        if unsafe { ioctl(fd, 0x5413, &mut size) } == 0 && size.0[1] > 0 {
            return usize::from(size.0[1]);
        }
    }
    80
}

/// `wrap-ansi` with `{trim: false, hard: true}` for plain text.
pub fn wrap_plain(text: &str, cols: usize) -> Vec<String> {
    wrapped(&[(text.into(), St::default())], cols as f32).iter().map(|l| l.iter().map(|c| c.0).collect()).collect()
}

/// Whether chalk colors stdout, and whether with 24-bit colors.
fn color_mode() -> Option<bool> {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let term = std::env::var("TERM").unwrap_or_default();
    let on = tty && term != "dumb" && std::env::var_os("NO_COLOR").is_none();
    on.then(|| matches!(std::env::var("COLORTERM").as_deref(), Ok("truecolor" | "24bit")))
}

fn ansi(st: St, truecolor: bool) -> (String, String) {
    let mut open = String::new();
    let mut close = String::new();
    if let Some(hex) = st.color {
        let n = u32::from_str_radix(&hex[1..], 16).expect("a theme color is hex");
        let [r, g, b] = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        match truecolor {
            true => open.push_str(&format!("\x1b[38;2;{r};{g};{b}m")),
            false => {
                // chalk's `rgbToAnsi256`.
                let c = |v: u8| (f32::from(v) / 255.0 * 5.0).round() as u16;
                let code = match r == g && g == b {
                    true if r < 8 => 16,
                    true if r > 248 => 231,
                    true => ((f32::from(r) - 8.0) / 247.0 * 24.0).round() as u16 + 232,
                    false => 16 + 36 * c(r) + 6 * c(g) + c(b),
                };
                open.push_str(&format!("\x1b[38;5;{code}m"));
            }
        }
        close += "\x1b[39m";
    }
    if st.bold {
        open = format!("\x1b[1m{open}");
        close += "\x1b[22m";
    }
    if st.italic {
        open = format!("\x1b[3m{open}");
        close += "\x1b[23m";
    }
    (open, close)
}

/// The text of `el` like the last frame of ink: each line without trailing spaces.
pub fn layout(el: &El, cols: usize, color: Option<bool>) -> String {
    let mut g = Grid(Vec::new());
    let h = draw(el, 0.0, 0, cols as f32, &mut g);
    g.0.resize_with(h.max(g.0.len()), Vec::new);
    let rows = g.0.iter().map(|row| {
        let end = row.iter().rposition(|c| c.0 != " " && !c.0.is_empty()).map_or(0, |i| i + 1);
        let end = end + row[end..].iter().take_while(|c| c.0.is_empty()).count();
        let mut out = String::new();
        let mut cur = St::default();
        for (s, st) in &row[..end] {
            if let (Some(tc), true) = (color, *st != cur) {
                out.push_str(&ansi(cur, tc).1);
                out.push_str(&ansi(*st, tc).0);
                cur = *st;
            }
            out.push_str(s);
        }
        if let Some(tc) = color {
            out.push_str(&ansi(cur, tc).1);
        }
        out
    });
    rows.collect::<Vec<_>>().join("\n")
}

pub fn print(el: &El) {
    println!("{}", layout(el, columns(1), color_mode()));
}

// ─── formatting (`lib/ink/format.js`) ──────────────────────────────────────────

/// `stringValue`: a string that is not blank.
pub fn sv(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.trim().is_empty())
}

fn compact(values: &[Option<String>]) -> Vec<String> {
    values.iter().flatten().filter(|s| !s.is_empty()).cloned().collect()
}

fn parse_time(v: &Value) -> Option<DateTime<Local>> {
    match v {
        Value::Number(n) => Local.timestamp_millis_opt(n.as_f64()? as i64).single(),
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Local)).or_else(|| {
            let day = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
            Some(chrono::Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0)?).with_timezone(&Local))
        }),
        _ => None,
    }
}

/// `formatTime`: "just now", "5m ago", "3d from now", else the date like `toLocaleDateString` in en-US.
pub fn format_time(v: &Value) -> Option<String> {
    if !v.is_string() && !v.is_number() {
        return None;
    }
    let Some(date) = parse_time(v) else { return Some(js_string(v)) };
    let now = Local::now();
    let diff = (now - date).num_milliseconds() as f64;
    let (abs, suffix) = (diff.abs(), if diff >= 0.0 { "ago" } else { "from now" });
    let round = |unit: f64| (abs / unit).round();
    Some(match abs {
        a if a < 60_000.0 => "just now".into(),
        a if a < 3_600_000.0 => format!("{}m {suffix}", round(60_000.0)),
        a if a < 86_400_000.0 => format!("{}h {suffix}", round(3_600_000.0)),
        a if a < 604_800_000.0 => format!("{}d {suffix}", round(86_400_000.0)),
        _ if date.year() == now.year() => date.format("%b %-d").to_string(),
        _ => date.format("%b %-d, %Y").to_string(),
    })
}

fn format_bytes(bytes: f64) -> String {
    match bytes {
        b if b < 1024.0 => format!("{b} B"),
        b if b < 1024.0 * 1024.0 => format!("{:.1} KB", b / 1024.0),
        b => format!("{:.1} MB", b / 1024.0 / 1024.0),
    }
}

fn format_duration(ms: f64) -> String {
    let seconds = (ms / 1000.0).round() as i64;
    match seconds < 60 {
        true => format!("{seconds}s"),
        false => format!("{}m {}s", seconds / 60, seconds % 60),
    }
}

fn short_id(id: &str) -> String {
    let local = id.split(':').next().expect("split gives one item");
    let local = local.strip_prefix('@').unwrap_or(local);
    if local.is_empty() {
        id.into()
    } else {
        local.into()
    }
}

/// `cleanText`: the text without HTML tags, Markdown links and marks, and runs of spaces, cut at 240 characters.
fn clean_text(v: &Value) -> Option<String> {
    let raw = sv(v)?;
    let mut text = String::new();
    let mut tag = false;
    for c in raw.chars() {
        match (tag, c) {
            (false, '<') if raw.contains('>') => (tag = true, text.push(' ')).0,
            (true, '>') => tag = false,
            (true, _) => {}
            (false, c) => text.push(c),
        }
    }
    let text = unlink(&text);
    let text: String = text.chars().filter(|c| !"*_`~>#".contains(*c)).collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let units: Vec<u16> = text.encode_utf16().collect();
    match units.len() {
        0 => None,
        n if n <= 240 => Some(text),
        _ => Some(format!("{}…", String::from_utf16_lossy(&units[..239]))),
    }
}

/// `[label](url)` becomes `label`.
fn unlink(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").filter(|&close| close > 0 && !after[..close].contains(']')).and_then(|close| {
            let url = &after[close + 2..];
            url.find(')').filter(|&end| end > 0).map(|end| (close, close + 2 + end + 1))
        });
        match link {
            Some((close, end)) => {
                out += &rest[..open];
                out += &after[..close];
                rest = &after[end..];
            }
            None => {
                out += &rest[..=open];
                rest = after;
            }
        }
    }
    out + rest
}

fn attachment_label(attachments: &[Value]) -> String {
    let label = |a: &Value| match a.is_object() {
        true => sv(&a["fileName"]).or(sv(&a["type"])).unwrap_or("attachment").to_string(),
        false => "attachment".into(),
    };
    match attachments {
        [one] => label(one),
        all => format!("{} attachments", all.len()),
    }
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn message_text(m: &Value) -> Option<String> {
    if truthy(&m["isDeleted"]) {
        return Some("deleted message".into());
    }
    if let Some(text) = clean_text(&m["text"]) {
        return Some(text);
    }
    match (arr(&m["attachments"]), arr(&m["links"]).len()) {
        (a, _) if !a.is_empty() => Some(attachment_label(a)),
        (_, 0) => None,
        (_, n) => Some(format!("{n} link{}", if n == 1 { "" } else { "s" })),
    }
}

fn participants_summary(p: &Value) -> Option<String> {
    let s = |n: u64| format!("{n} participant{}", if n == 1 { "" } else { "s" });
    match (p["total"].as_f64(), arr(&p["items"]).len()) {
        (Some(total), _) => Some(match total.fract() == 0.0 {
            true => s(total as u64),
            false => format!("{total} participants"),
        }),
        (None, 0) => None,
        (None, n) => Some(s(n as u64)),
    }
}

fn flag(v: &Value, key: &str, old: &str) -> bool {
    truthy(if v[key].is_null() { &v[old] } else { &v[key] })
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().expect("a JSON number is a float"),
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn js_num(n: f64) -> String {
    js_string(&serde_json::json!(n)).trim_end_matches(".0").to_string()
}

// ─── components (`lib/ink/components.js`) ──────────────────────────────────────

fn rail(color: &'static str) -> El {
    t(RAIL, color)
}

/// A card or row head: the rail, a space, and a bold title.
fn head(color: &'static str, title: &str, rest: Vec<El>) -> El {
    row([vec![rail(color), plain(" "), tb(title, TEXT)], rest].concat())
}

fn kv(label: &str, value: &str, color: &'static str) -> El {
    row(vec![t(format!("{label:<12}"), SUBTLE), t(value, color)]).ml(2)
}

pub struct Empty {
    pub title: String,
    pub subtitle: String,
    pub suggestions: Vec<(String, &'static str)>,
}

impl Empty {
    pub fn new(title: &str, subtitle: impl Into<String>) -> Self {
        Self { title: title.into(), subtitle: subtitle.into(), suggestions: Vec::new() }
    }
    /// A suggestion. `command` is without the program name.
    pub fn try_(mut self, command: impl AsRef<str>, hint: &'static str) -> Self {
        self.suggestions.push((format!("{BIN} {}", command.as_ref()), hint));
        self
    }
}

fn suggestions(items: &[(String, &'static str)]) -> El {
    when(!items.is_empty(), || {
        let rows = items.iter().map(|(command, hint)| {
            let hint = when(!hint.is_empty(), || t(format!("  — {hint}"), MUTED));
            row(vec![t("  › ", PRIMARY), t(command, PRIMARY_GLOW), hint])
        });
        col([vec![t("Try", SUBTLE)], rows.collect()].concat()).mt(1)
    })
}

pub fn empty_state(e: &Empty) -> El {
    let subtitle = when(!e.subtitle.is_empty(), || row(vec![t(&e.subtitle, MUTED)]).ml(2));
    col(vec![row(vec![rail(SUBTLE), plain(" "), tb(&e.title, TEXT)]), subtitle, suggestions(&e.suggestions)])
}

pub fn success(message: &str, detail: Option<&str>) -> El {
    let detail = detail.map_or(El::Empty, |d| t(format!("  {d}"), MUTED));
    row(vec![t("✓", MINE), plain(" "), tb(message, TEXT), detail])
}

fn chat_rail_color(chat: &Value, unread: f64, mentions: f64) -> &'static str {
    match () {
        _ if mentions > 0.0 => MENTION,
        _ if unread > 0.0 => PRIMARY,
        _ if flag(chat, "isPinned", "pinned") => WARN,
        _ => bridge_color(sv(&chat["network"])).unwrap_or(SUBTLE),
    }
}

fn chat_preview(chat: &Value) -> El {
    if chat["draft"].is_object() {
        let text = clean_text(&chat["draft"]["text"]).map_or(String::new(), |t| format!("  {t}"));
        let st = |color, italic| St { color: Some(color), italic, bold: false };
        return row(vec![El::Text(vec![("draft".into(), st(DRAFT, true)), (text, st(MUTED, false))])]).ml(2);
    }
    let last = if chat["latestMessage"].is_null() { &chat["lastMessage"] } else { &chat["latestMessage"] };
    if !last.is_object() {
        return El::Empty;
    }
    let sender = sv(&last["senderName"]).or(truthy(&last["isSender"]).then_some("you"));
    let text = message_text(last).unwrap_or_default();
    if text.is_empty() && sender.is_none() {
        return El::Empty;
    }
    let st = |color| St { color: Some(color), ..St::default() };
    let sender = sender.map(|s| (format!("{s}  "), st(SUBTLE)));
    row(vec![El::Text(sender.into_iter().chain([(text, st(MUTED))]).collect())]).ml(2)
}

fn local_id(chat: &Value) -> Option<String> {
    let id = if chat["localChatID"].is_null() { &chat["rowID"] } else { &chat["localChatID"] };
    (truthy(&chat["localChatID"]) || truthy(&chat["rowID"])).then(|| js_string(id))
}

pub fn chat_row(chat: &Value) -> El {
    let (unread, mentions) = (num(&chat["unreadCount"]), num(&chat["unreadMentionsCount"]));
    let muted = flag(chat, "isMuted", "muted");
    let network = sv(&chat["network"]);
    let title = El::Text(vec![(
        js_string(&chat["title"]),
        St { color: Some(if muted { MUTED } else { TEXT }), bold: unread > 0.0, italic: false },
    )]);
    let mark = |on: bool, s: &str, color| when(on, || t(format!("  {s}"), color));
    let unread_text = format!(" {}{}", js_num(unread), if mentions > 0.0 { "" } else { " unread" });
    let unread_st = St { color: Some(if muted { SUBTLE } else { PRIMARY }), bold: !muted, italic: false };
    let last = format_time(&chat["lastActivity"]);
    let head = row(vec![
        rail(chat_rail_color(chat, unread, mentions)),
        plain(" "),
        title,
        network.map_or(El::Empty, |n| t(format!("  {}", n.to_lowercase()), bridge_color(Some(n)).unwrap_or(SUBTLE))),
        mark(flag(chat, "isPinned", "pinned"), "★", WARN),
        mark(muted, "◐", MAGENTA),
        mark(flag(chat, "isArchived", "archived"), "◇", SUBTLE),
        mark(flag(chat, "isLowPriority", "lowPriority"), "◌", SUBTLE),
        El::Grow,
        when(mentions > 0.0, || tb(format!("  @{}", js_num(mentions)), MENTION)),
        when(unread > 0.0, || El::Text(vec![(unread_text, unread_st)])),
        last.map_or(El::Empty, |l| t(format!("  {l}"), SUBTLE)),
    ]);
    let ids = match local_id(chat) {
        Some(local) => vec![t("local ", SUBTLE), t(local, TEXT), t("  id ", SUBTLE), t(js_string(&chat["id"]), SUBTLE)],
        None => vec![t(js_string(&chat["id"]), SUBTLE)],
    };
    col(vec![head, chat_preview(chat), row(ids).ml(2)]).mb(1)
}

pub fn chat_detail(chat: &Value) -> El {
    let (unread, mentions) = (num(&chat["unreadCount"]), num(&chat["unreadMentionsCount"]));
    let participants = &chat["participants"];
    let items = arr(&participants["items"]);
    let network = sv(&chat["network"]);
    let mark = |on: bool, s: &str, color| when(on, || t(format!("  {s}"), color));
    let head = head(
        chat_rail_color(chat, unread, mentions),
        &js_string(&chat["title"]),
        vec![
            network
                .map_or(El::Empty, |n| t(format!("  {}", n.to_lowercase()), bridge_color(Some(n)).unwrap_or(SUBTLE))),
            mark(flag(chat, "isPinned", "pinned"), "★ pinned", WARN),
            mark(flag(chat, "isMuted", "muted"), "◐ muted", MAGENTA),
            mark(flag(chat, "isArchived", "archived"), "◇ archived", SUBTLE),
            mark(flag(chat, "isLowPriority", "lowPriority"), "◌ low-priority", SUBTLE),
        ],
    );
    let unread_text = format!(
        "{}{}",
        js_num(unread),
        if mentions > 0.0 { format!(" ({} @)", js_num(mentions)) } else { String::new() }
    );
    let people = participants_summary(participants).unwrap_or_else(|| items.len().to_string());
    let list = when(!items.is_empty(), || {
        let rows = items.iter().take(20).map(|p| {
            let id = sv(&p["id"]);
            let name = sv(&p["fullName"])
                .or(sv(&p["username"]))
                .map_or_else(|| short_id(&js_string(&p["id"])), str::to_string);
            let you = when(truthy(&p["isSelf"]), || t("  you", MINE));
            row(vec![t("·", sender_color(id)), plain("  "), t(name, TEXT), you]).ml(2)
        });
        let more = when(items.len() > 20, || row(vec![t(format!("… {} more", items.len() - 20), SUBTLE)]).ml(2));
        col([vec![row(vec![t("PARTICIPANTS", SUBTLE)]).ml(2)], rows.collect(), vec![more]].concat()).mt(1)
    });
    col(vec![
        head,
        when(truthy(&chat["type"]), || kv("type", &js_string(&chat["type"]), MUTED)),
        when(truthy(&chat["lastActivity"]), || {
            let last = format_time(&chat["lastActivity"]).unwrap_or_else(|| js_string(&chat["lastActivity"]));
            kv("last", &last, MUTED)
        }),
        when(unread > 0.0, || kv("unread", &unread_text, TEXT)),
        when(truthy(participants), || kv("people", &people, MUTED)),
        list,
        row(vec![t("id    ", SUBTLE), t(js_string(&chat["id"]), TEXT)]).ml(2).mt(1),
        local_id(chat).map_or(El::Empty, |l| row(vec![t("local ", SUBTLE), t(l, TEXT)]).ml(2)),
        when(truthy(&chat["accountID"]), || {
            row(vec![t("acct  ", SUBTLE), t(js_string(&chat["accountID"]), MUTED)]).ml(2)
        }),
    ])
}

pub fn message_row(m: &Value) -> El {
    let mine = truthy(&m["isSender"]);
    let sender_id = sv(&m["senderID"]);
    let sender =
        sv(&m["senderName"]).map(str::to_string).or(sender_id.map(short_id)).unwrap_or_else(|| "unknown".into());
    let status = if m["sendStatus"].is_object() { &m["sendStatus"] } else { &Value::Null };
    let failure = truthy(&status["status"]) && status["status"] != "SUCCESS";
    let (attachments, reactions) = (arr(&m["attachments"]), arr(&m["reactions"]));
    let rail_color = if mine { MINE } else { sender_color(sender_id) };
    let kind = when(!m["type"].is_null() && m["type"] != "TEXT", || {
        t(format!("  {}", js_string(&m["type"]).to_lowercase()), MAGENTA)
    });
    let head = row(vec![
        rail(rail_color),
        plain(" "),
        tb(if mine { "you".into() } else { sender }, rail_color),
        kind,
        when(truthy(&m["isUnread"]), || t("  ● unread", PRIMARY)),
        when(truthy(&m["isDeleted"]), || t("  deleted", DANGER)),
        when(truthy(&m["editedTimestamp"]), || t("  ✎ edited", SUBTLE)),
        when(!attachments.is_empty(), || t(format!("  📎 {}", attachment_label(attachments)), WARN)),
        when(!reactions.is_empty(), || t(format!("  ♥{}", reactions.len()), MAGENTA)),
        El::Grow,
        format_time(&m["timestamp"]).map_or(El::Empty, |ts| t(ts, SUBTLE)),
    ]);
    let failure = when(failure, || {
        let reason = when(truthy(&status["message"]), || t(format!("  {}", js_string(&status["message"])), MUTED));
        row(vec![t(format!("✗ {}", js_string(&status["status"])), DANGER), reason]).ml(2)
    });
    let chat = when(truthy(&m["chatID"]), || t(format!("  in {}", js_string(&m["chatID"])), SUBTLE));
    col(vec![
        head,
        message_text(m).map_or(El::Empty, |text| row(vec![t(text, TEXT)]).ml(2)),
        failure,
        row(vec![t(js_string(&m["id"]), SUBTLE), chat]).ml(2),
    ])
    .mb(1)
}

fn user_row(u: &Value) -> El {
    let title = ["fullName", "username", "email", "phoneNumber"].iter().find_map(|k| sv(&u[*k]));
    let title = title.map_or_else(|| js_string(&u["id"]), str::to_string);
    let handles = compact(&[
        truthy(&u["username"]).then(|| format!("@{}", js_string(&u["username"]))),
        truthy(&u["email"]).then(|| js_string(&u["email"])),
        truthy(&u["phoneNumber"]).then(|| js_string(&u["phoneNumber"])),
    ]);
    let you = truthy(&u["isSelf"]);
    col(vec![
        head(
            if you { MINE } else { sender_color(sv(&u["id"])) },
            &title,
            vec![when(you, || t("  you", MINE)), when(truthy(&u["cannotMessage"]), || t("  cannot message", DANGER))],
        ),
        when(!handles.is_empty(), || row(vec![t(handles.join("  "), MUTED)]).ml(2)),
        row(vec![
            t(js_string(&u["id"]), SUBTLE),
            when(truthy(&u["accountID"]), || t(format!("  on {}", js_string(&u["accountID"])), SUBTLE)),
        ])
        .ml(2),
    ])
    .mb(1)
}

fn account_row(a: &Value) -> El {
    let id = if a["accountID"].is_null() { &a["id"] } else { &a["accountID"] };
    let bridge = a["bridge"].is_object().then_some(&a["bridge"]);
    let bridge_label = match bridge {
        Some(b) => Some(compact(&["id", "provider", "type"].map(|k| sv(&b[k]).map(str::to_string))).join(" ")),
        None => sv(&a["bridge"]).map(str::to_string),
    };
    let bridge_id = bridge.and_then(|b| sv(&b["id"]));
    let title = sv(&a["displayName"]).or(sv(&a["name"])).or(sv(&a["network"])).or(bridge_id);
    let title = title.map_or_else(|| js_string(id), str::to_string);
    let network = sv(&a["network"]).or(bridge.and_then(|b| sv(&b["type"])));
    let tint = bridge_color(network);
    let state = sv(&a["state"]).map(str::to_lowercase);
    let low = state.clone().unwrap_or_default();
    let connected = low.contains("online") || low.contains("connect");
    let errored = low.contains("error") || low.contains("fail");
    let tone = if connected {
        MINE
    } else if errored {
        DANGER
    } else {
        WARN_ALT
    };
    let glyph = if connected {
        "●"
    } else if errored {
        "✗"
    } else {
        "○"
    };
    let handles = compact(&[
        truthy(&a["username"]).then(|| format!("@{}", js_string(&a["username"]))),
        sv(&a["userID"]).map(str::to_string),
    ]);
    col(vec![
        head(
            tint.unwrap_or(PRIMARY),
            &title,
            vec![
                network.map_or(El::Empty, |n| t(format!("  {}", n.to_lowercase()), tint.unwrap_or(MUTED))),
                state.map_or(El::Empty, |s| t(format!("  {glyph} {s}"), tone)),
            ],
        ),
        when(!handles.is_empty(), || row(vec![t(handles.join("  "), MUTED)]).ml(2)),
        row(vec![
            t(js_string(id), SUBTLE),
            bridge_label.filter(|l| !l.is_empty()).map_or(El::Empty, |l| t(format!("  bridge {l}"), SUBTLE)),
        ])
        .ml(2),
    ])
    .mb(1)
}

fn bridge_row(b: &Value) -> El {
    let id = js_string(&b["id"]);
    let network = sv(&b["network"]).or(sv(&b["type"]));
    let status = sv(&b["status"]);
    let accounts = arr(&b["accounts"]);
    let available = status == Some("available");
    let connected = status == Some("connected") || !accounts.is_empty();
    let tone = if available {
        MINE
    } else if connected {
        PRIMARY
    } else {
        WARN
    };
    let flows = arr(&b["loginFlows"]);
    let flows_list = when(!flows.is_empty(), || {
        let lines = flows.iter().take(5).map(|f| {
            let flow_id = if f["id"].is_null() { &f["type"] } else { &f["id"] };
            let flow_id = if flow_id.is_null() { "flow".into() } else { js_string(flow_id) };
            let name = sv(&f["name"]).map_or(String::new(), |n| format!("  {n}"));
            let desc = sv(&f["description"]).map_or(String::new(), |d| format!(" - {d}"));
            t(format!("· {flow_id}{name}{desc}"), MUTED)
        });
        let more = when(flows.len() > 5, || t(format!("  {} more flows", flows.len() - 5), SUBTLE));
        col([lines.collect(), vec![more]].concat()).ml(2)
    });
    let accounts_list = when(!accounts.is_empty(), || {
        let lines = accounts.iter().take(3).map(|a| {
            let id = if a["accountID"].is_null() { &a["id"] } else { &a["accountID"] };
            let status = match truthy(&a["status"]) {
                true => format!("  {}", js_string(&a["status"]).replace('_', " ")),
                false => String::new(),
            };
            t(format!("· {}{status}", js_string(id)), MUTED)
        });
        let more = when(accounts.len() > 3, || t(format!("  {} more", accounts.len() - 3), SUBTLE));
        col([lines.collect(), vec![more]].concat()).ml(2)
    });
    let count = b["activeAccountCount"].as_f64().map_or(El::Empty, |n| t(format!("  accounts {}", js_num(n)), SUBTLE));
    let multiple = b["supportsMultipleAccounts"]
        .as_bool()
        .map_or(El::Empty, |m| t(format!("  {}", if m { "multiple allowed" } else { "single account" }), SUBTLE));
    col(vec![
        head(
            tone,
            sv(&b["displayName"]).unwrap_or(&id),
            vec![
                network.map_or(El::Empty, |n| {
                    t(format!("  {}", n.to_lowercase()), bridge_color(Some(n)).unwrap_or(MUTED))
                }),
                sv(&b["provider"]).map_or(El::Empty, |p| t(format!("  {p}"), SUBTLE)),
                status.map_or(El::Empty, |s| t(format!("  {}", s.replace('_', " ")), tone)),
            ],
        ),
        sv(&b["statusText"]).map_or(El::Empty, |s| row(vec![t(s, MUTED)]).ml(2)),
        row(vec![
            t("id ", SUBTLE),
            t(&id, TEXT),
            count,
            multiple,
            when(!flows.is_empty(), || t(format!("  flows {}", flows.len()), SUBTLE)),
        ])
        .ml(2),
        flows_list,
        accounts_list,
        when(available, || row(vec![t(format!("{BIN} accounts add {id}"), PRIMARY_GLOW)]).ml(2)),
    ])
    .mb(1)
}

fn send_result(r: &Value) -> El {
    let message = if r["message"].is_object() { &r["message"] } else { &Value::Null };
    let send_status = if message["sendStatus"].is_object() { &message["sendStatus"] } else { &Value::Null };
    let state = sv(&r["state"]);
    let status = sv(&send_status["status"]);
    let failed = status.is_some_and(|s| s.starts_with("FAIL"));
    let rail_color = if failed {
        DANGER
    } else if state == Some("resolved") {
        MINE
    } else {
        PRIMARY
    };
    let badge = match () {
        _ if failed => t("✗ failed", DANGER),
        _ if status == Some("SUCCESS") => t("✓ sent", MINE),
        _ if state == Some("resolved") => t("✓ resolved", MINE),
        _ => t("○ accepted by Desktop", PRIMARY),
    };
    let tone = if failed { TEXT } else { MUTED };
    col(vec![
        head(rail_color, "Message send", vec![plain("  "), badge]),
        kv("chat", &js_string(&r["chatID"]), MUTED),
        when(truthy(&r["pendingMessageID"]), || kv("pending", &js_string(&r["pendingMessageID"]), MUTED)),
        sv(&message["id"]).map_or(El::Empty, |id| kv("message", id, TEXT)),
        status.map_or(El::Empty, |s| kv("status", s, tone)),
        when(truthy(&send_status["message"]), || kv("reason", &js_string(&send_status["message"]), tone)),
        when(truthy(&r["hint"]), || row(vec![t(js_string(&r["hint"]), SUBTLE)]).ml(2)),
    ])
}

fn asset_row(a: &Value) -> El {
    let title = ["fileName", "uploadID", "srcURL"].iter().find_map(|k| sv(&a[*k])).unwrap_or("asset");
    let meta = compact(&[
        a["fileSize"].as_f64().map(format_bytes),
        (truthy(&a["width"]) && truthy(&a["height"]))
            .then(|| format!("{}×{}", js_string(&a["width"]), js_string(&a["height"]))),
        a["duration"].as_f64().map(format_duration),
    ]);
    let error = truthy(&a["error"]);
    col(vec![
        head(
            if error { DANGER } else { CYAN },
            title,
            vec![
                sv(&a["mimeType"]).map_or(El::Empty, |m| t(format!("  {m}"), SUBTLE)),
                when(!meta.is_empty(), || t(format!("  {}", meta.join("  ")), MUTED)),
            ],
        ),
        when(truthy(&a["srcURL"]), || row(vec![t(js_string(&a["srcURL"]), SUBTLE)]).ml(2)),
        when(truthy(&a["uploadID"]), || row(vec![t(format!("upload {}", js_string(&a["uploadID"])), SUBTLE)]).ml(2)),
        when(error, || row(vec![t(format!("✗ {}", js_string(&a["error"])), DANGER)]).ml(2)),
    ])
    .mb(1)
}

fn info_card(info: &Value) -> El {
    let user = if info["user"].is_object() { &info["user"] } else { &Value::Null };
    let name = ["fullName", "username", "id"].iter().find_map(|k| sv(&user[*k]));
    let endpoints = info["endpoints"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_str().map(|url| row(vec![t(format!("{k:<12}"), SUBTLE), t(url, LINK)]).ml(2)));
    let mut kids = vec![
        head(
            PRIMARY,
            "Beeper Desktop",
            vec![
                sv(&info["version"]).map_or(El::Empty, |v| t(format!("  v{v}"), MUTED)),
                sv(&info["platform"]).map_or(El::Empty, |p| t(format!("  {p}"), SUBTLE)),
            ],
        ),
        name.map_or(El::Empty, |n| kv("user", n, TEXT)),
    ];
    kids.extend(endpoints);
    col(kids)
}

fn doctor_card(record: &Map<String, Value>) -> El {
    let checks: Vec<(bool, String, String)> = match &record["checks"] {
        Value::Array(list) => list
            .iter()
            .map(|c| (truthy(&c["ok"]), js_string(&c["name"]), sv(&c["detail"]).unwrap_or("").into()))
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(name, v)| {
                let detail = if v.is_object() || v.is_array() { v.to_string() } else { js_string(v) };
                let ok = match (name.as_str(), v["reachable"].as_bool()) {
                    ("readiness", _) => v["state"] == "ready",
                    (_, Some(r)) => r,
                    _ => truthy(v),
                };
                (ok, name.clone(), detail)
            })
            .collect(),
        _ => Vec::new(),
    };
    let ok = truthy(&record["ok"]);
    let longest = checks.iter().map(|c| c.1.chars().count()).max().unwrap_or(0);
    let rows = checks.iter().map(|(ok, name, detail)| {
        let (glyph, color) = if *ok { ("✓", MINE) } else { ("✗", DANGER) };
        let detail = when(!detail.is_empty(), || t(detail, MUTED));
        row(vec![t(glyph, color), plain("  "), t(format!("{name:<w$}", w = longest + 2), TEXT), detail]).ml(2)
    });
    let badge = if ok { t("✓ healthy", MINE) } else { t("✗ attention needed", DANGER) };
    let tips = [("setup", "continue target setup"), ("status", "verify the API endpoint")]
        .map(|(c, h)| (format!("{BIN} {c}"), h));
    col(vec![
        head(if ok { MINE } else { DANGER }, "Doctor", vec![plain("  "), badge]),
        col(rows.collect()).mt(1),
        when(!ok, || suggestions(&tips)),
    ])
}

fn auth_card(auth: &Value) -> El {
    let ok = truthy(&auth["authenticated"]);
    let expires = when(truthy(&auth["expiresAt"]), || {
        kv("expires", &format_time(&auth["expiresAt"]).unwrap_or_else(|| js_string(&auth["expiresAt"])), MUTED)
    });
    let source = if auth["source"].is_null() { "none".into() } else { js_string(&auth["source"]) };
    let badge = if ok { t("✓ signed in", MINE) } else { t("○ signed out", WARN) };
    col(vec![
        head(if ok { MINE } else { WARN }, "Authentication", vec![plain("  "), badge]),
        kv("endpoint", &js_string(&auth["baseURL"]), LINK),
        kv("source", &source, MUTED),
        when(truthy(&auth["clientID"]), || kv("client", &js_string(&auth["clientID"]), SUBTLE)),
        when(truthy(&auth["scope"]), || kv("scope", &js_string(&auth["scope"]), MUTED)),
        expires,
        when(!ok, || suggestions(&[(format!("{BIN} setup"), "continue target setup")])),
    ])
}

fn readiness_card(data: &Value) -> El {
    let target = data["target"].is_object().then_some(&data["target"]);
    let readiness = if data["readiness"].is_object() { &data["readiness"] } else { data };
    let state = sv(&readiness["state"]).unwrap_or("unknown");
    let ready = state == "ready";
    let actions: Vec<String> = arr(&readiness["actions"]).iter().map(js_string).collect();
    let target_rows = target.map_or(vec![], |tg| {
        let name = ["id", "name"].iter().map(|k| &tg[*k]).find(|v| !v.is_null()).map_or("selected".into(), js_string);
        vec![
            kv("target", &name, TEXT),
            when(truthy(&tg["baseURL"]), || kv("endpoint", &js_string(&tg["baseURL"]), LINK)),
        ]
    });
    let tips: Vec<(String, &str)> = actions.iter().take(4).map(|c| (format!("{BIN} {c}"), "")).collect();
    let state_color = if ready { MINE } else { WARN };
    col([
        vec![head(
            state_color,
            if ready { "Ready" } else { "Not ready" },
            vec![t(format!("  {}", state.replace('-', " ")), state_color)],
        )],
        target_rows,
        vec![
            sv(&readiness["message"]).map_or(El::Empty, |m| kv("next", m, if ready { MUTED } else { TEXT })),
            when(!actions.is_empty() && !ready, || suggestions(&tips)),
        ],
    ]
    .concat())
}

fn mask_token(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    match chars.len() <= 12 {
        true => "••••".into(),
        false => {
            format!("{}…{}", chars[..6].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
        }
    }
}

fn config_value(key: &str, value: &Value) -> El {
    let secret = |k: &str| ["token", "secret", "key"].iter().any(|s| k.to_lowercase().contains(s));
    match value {
        Value::Null => t("—", SUBTLE),
        Value::String(s) if secret(key) => t(mask_token(s), SUBTLE),
        Value::Object(map) => {
            let inner: Vec<(&String, &Value)> = map.iter().filter(|(_, v)| !v.is_null()).collect();
            if inner.is_empty() {
                return t("{}", SUBTLE);
            }
            let w = inner.iter().map(|(k, _)| k.len()).max().expect("a config entry") + 2;
            let exact = |k: &str| ["accesstoken", "token", "secret", "key"].contains(&k.to_lowercase().as_str());
            col(inner
                .iter()
                .map(|(k, v)| {
                    let shown = match v {
                        Value::String(s) if exact(k) => t(mask_token(s), SUBTLE),
                        v if v.is_object() || v.is_array() => t(v.to_string(), TEXT),
                        v => t(js_string(v), TEXT),
                    };
                    row(vec![t(format!("{k:<w$}"), SUBTLE), shown])
                })
                .collect())
        }
        v => t(js_string(v), TEXT),
    }
}

pub fn config_view(data: &Map<String, Value>) -> El {
    let entries: Vec<(&String, &Value)> = data.iter().filter(|(_, v)| !v.is_null()).collect();
    if entries.is_empty() {
        let e = Empty::new("No config set", "The CLI is using defaults.")
            .try_("config set baseURL <url>", "override the API endpoint");
        return empty_state(&e);
    }
    let w = entries.iter().map(|(k, _)| k.len()).max().expect("a config entry") + 2;
    let rows = entries.iter().map(|(k, v)| match v.is_object() {
        true => col(vec![row(vec![t(format!("{k:<w$}"), SUBTLE)]), row(vec![config_value(k, v)]).ml(w + 2)]).ml(2),
        false => row(vec![row(vec![t(format!("{k:<w$}"), SUBTLE), config_value(k, v)])]).ml(2),
    });
    col([vec![head(PRIMARY, "Config", vec![])], rows.collect()].concat())
}

fn generic_row(item: &Map<String, Value>) -> El {
    let title =
        ["title", "displayName", "name", "id", "messageID"].iter().find_map(|k| item.get(*k).filter(|v| !v.is_null()));
    let scalars: Vec<(&String, &Value)> = item
        .iter()
        .filter(|(k, v)| {
            !v.is_null() && !["title", "displayName", "name"].contains(&k.as_str()) && !v.is_object() && !v.is_array()
        })
        .collect();
    let w = scalars.iter().map(|(k, _)| k.chars().count()).max().map_or(0, |n| n + 2);
    let rows = scalars.iter().map(|(k, v)| row(vec![t(format!("{k:<w$}"), SUBTLE), t(js_string(v), TEXT)]).ml(2));
    col([vec![title.map_or(El::Empty, |v| head(PRIMARY, &js_string(v), vec![]))], rows.collect()].concat()).mb(1)
}

fn search_view(r: &Value) -> El {
    let (chats, messages) = (arr(&r["chats"]), arr(&r["messages"]));
    if chats.is_empty() && messages.is_empty() {
        let e = Empty::new("Nothing matched your search", "Try a different query, account, or check spelling.")
            .try_("chats", "browse everything")
            .try_("chats search \"<term>\"", "narrow with filters");
        return empty_state(&e);
    }
    let section = |label: &str, n: usize| row(vec![t(label, SUBTLE), t(format!("  {n}"), SUBTLE)]).mt(1);
    col([
        vec![when(!chats.is_empty(), || section("CHATS", chats.len()))],
        chats.iter().map(chat_row).collect(),
        vec![when(!messages.is_empty(), || section("MESSAGES", messages.len()))],
        messages.iter().map(message_row).collect(),
    ]
    .concat())
}

/// `detectKind` of `lib/ink/render.js`: the kind of a record from its shape.
fn kind(r: &Map<String, Value>) -> &'static str {
    let is = |k: &str| r.get(k).is_some_and(Value::is_string);
    let n = |k: &str| r.get(k).is_some_and(Value::is_number);
    let obj = |k: &str| r.get(k).is_some_and(|v| v.is_object() || v.is_array());
    let has = |k: &str| r.get(k).is_some_and(truthy);
    match () {
        _ if is("id") && is("accountID") && is("title") && n("unreadCount") => {
            match has("participants") && obj("participants") {
                true => "chatDetail",
                false => "chat",
            }
        }
        _ if is("id") && is("chatID") && is("senderID") && is("timestamp") => "message",
        _ if is("chatID")
            && is("pendingMessageID")
            && (r.get("accepted") == Some(&Value::Bool(true)) || is("state")) =>
        {
            "sendResult"
        }
        _ if is("id") && ["fullName", "username", "email", "phoneNumber"].iter().any(|k| is(k)) => "user",
        _ if is("id")
            && is("provider")
            && is("status")
            && (is("displayName") || r.get("accounts").is_some_and(Value::is_array)) =>
        {
            "bridge"
        }
        _ if r.get("accountID").filter(|v| !v.is_null()).or(r.get("id")).is_some_and(Value::is_string)
            && (is("network") || is("bridge") || is("displayName")) =>
        {
            "account"
        }
        _ if is("uploadID") || is("srcURL") => "asset",
        _ if is("version") && obj("endpoints") => "info",
        _ if r.get("ok").is_some_and(Value::is_boolean) && obj("checks") => "doctor",
        _ if has("readiness") && obj("readiness") => "readiness",
        _ if is("state") && r.get("actions").is_some_and(Value::is_array) => "readiness",
        _ if r.get("authenticated").is_some_and(Value::is_boolean) && is("baseURL") => "auth",
        _ if is("sub") && (is("email") || is("name") || is("preferred_username")) => "oauth",
        _ if r.get("chats").is_some_and(Value::is_array) && r.get("messages").is_some_and(Value::is_array) => "search",
        _ => "generic",
    }
}

fn row_for(kind: &str, item: &Value) -> El {
    let empty = Map::new();
    let map = item.as_object().unwrap_or(&empty);
    match kind {
        "chat" | "chatDetail" => chat_row(item),
        "message" => message_row(item),
        "user" => user_row(item),
        "account" => account_row(item),
        "bridge" => bridge_row(item),
        "asset" => asset_row(item),
        _ => generic_row(map),
    }
}

/// `renderList`: the rows of `items` in the kind of the first item, or the empty state.
pub fn list(items: &[Value], empty: Option<&Empty>) -> El {
    let Some(first) = items.first() else { return empty.map_or(El::Empty, empty_state) };
    let empty_map = Map::new();
    let kind = kind(first.as_object().unwrap_or(&empty_map));
    col(items.iter().map(|i| row_for(kind, i)).collect())
}

fn oauth_card(user: &Value) -> El {
    let name = ["name", "preferred_username", "email"].iter().find_map(|k| sv(&user[*k]));
    col(vec![
        head(PRIMARY, &name.map_or_else(|| js_string(&user["sub"]), str::to_string), vec![t("  you", SUBTLE)]),
        when(truthy(&user["email"]), || kv("email", &js_string(&user["email"]), TEXT)),
        when(truthy(&user["preferred_username"]), || {
            kv("username", &format!("@{}", js_string(&user["preferred_username"])), MUTED)
        }),
        when(truthy(&user["sub"]), || kv("sub", &js_string(&user["sub"]), SUBTLE)),
    ])
}

/// `renderValue`: the card of one value.
pub fn value(v: &Value) -> El {
    match v {
        Value::Array(items) => list(items, None),
        Value::Null => El::Empty,
        Value::Object(r) => match kind(r) {
            "info" => info_card(v),
            "doctor" => doctor_card(r),
            "auth" => auth_card(v),
            "oauth" => oauth_card(v),
            "search" => search_view(v),
            "readiness" => readiness_card(v),
            "sendResult" => send_result(v),
            "chat" | "chatDetail" => chat_detail(v),
            k => row_for(k, v),
        },
        v => plain(&js_string(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(el: &El) -> String {
        layout(el, 80, None)
    }

    #[test]
    fn wrap_follows_wrap_ansi() {
        let wrap = |s: &str, cols: f32| {
            wrap_line(&cells(&[(s.into(), St::default())]), cols)
                .iter()
                .map(|l| l.iter().map(|c| c.0).collect::<String>())
                .collect::<Vec<_>>()
        };
        assert_eq!(wrap("aaa bbb ccc", 7.0), ["aaa bbb", " ccc"]);
        assert_eq!(wrap("aaa bbb ccc", 8.0), ["aaa bbb ", "ccc"]);
        assert_eq!(wrap("abcdefghij", 4.0), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("ab cdefghij", 4.0), ["ab ", "cdef", "ghij"]);
        assert_eq!(wrap("abcd efg", 4.0), ["abcd", " efg"]);
    }

    #[test]
    fn rows_shrink_and_round_like_yoga() {
        // The doctor row of upstream: the long detail shrinks the glyph to nothing and cuts the name.
        let detail = "{\"reachable\":true,\"version\":\"4.3.176\",\"bundleID\":\"com.automattic.beeper.desktop\",\"actualType\":\"desktop\"}".to_string();
        let el = row(vec![t("✓", MINE), plain("  "), t("target     ", TEXT), t(detail, MUTED)]).ml(2);
        let out = text(&el);
        assert_eq!(
            out.lines().next(),
            Some("    target   {\"reachable\":true,\"version\":\"4.3.176\",\"bundleID\":\"com.automattic.be")
        );
        assert_eq!(out.lines().nth(1), Some("             eper.desktop\",\"actualType\":\"desktop\"}"));
    }

    #[test]
    fn chat_rows_match_upstream() {
        let chat = json!({
            "id": "!tg_rust:beeper.com", "localChatID": "local_tg_rust", "accountID": "acc_telegram",
            "title": "Beeper Rust Core Team", "unreadCount": 2, "isPinned": true, "lastActivity": "2020-10-14T12:00:00Z"
        });
        let out = text(&list(&[chat], None));
        assert_eq!(
            out,
            "▎ Beeper Rust Core Team  ★                                2 unread  Oct 14, 2020\n  local local_tg_rust  id !tg_rust:beeper.com\n"
        );
    }

    #[test]
    fn generic_rows_and_empty_states_match_upstream() {
        let version = json!({ "name": "@beeper/cli", "version": "0.6.2" });
        assert_eq!(text(&value(&version)), "▎ @beeper/cli\n  version  0.6.2\n");
        let e = Empty::new("No config set", "The CLI is using defaults.")
            .try_("config set baseURL <url>", "override the API endpoint");
        assert_eq!(
            text(&empty_state(&e)),
            format!("▎ No config set\n  The CLI is using defaults.\n\nTry\n  › {BIN} config set baseURL <url>  — override the API endpoint")
        );
    }
}

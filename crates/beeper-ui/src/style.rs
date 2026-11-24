use crate::prelude::*;
use iced::advanced::graphics::text::cosmic_text::{self, fontdb::Source, Fallback as _};
use iced::advanced::graphics::text::{font_system, to_attributes};
use iced::border;
use iced::font::{Family, Font};
use iced::overlay::menu;
use iced::widget::text;
use std::sync::Once;

pub mod icon {
    pub const INBOX: char = '\u{f000}';
    pub const LOW_PRIORITY_SPACE: char = '\u{f001}';
    pub const ARCHIVE: char = '\u{f002}';
    pub const SCHEDULE: char = '\u{f003}';
    pub const SCHEDULE_SEND: char = '\u{f004}';
    pub const EMAIL: char = '\u{f005}';
    pub const MAPS_UGC: char = '\u{f006}';
    pub const PHOTO_LIBRARY: char = '\u{f007}';
    pub const LIBRARY_ADD: char = '\u{f008}';
    pub const MORE_HORIZ: char = '\u{f009}';
    pub const UNFOLD_MORE: char = '\u{f00a}';
    pub const FILTER_LIST: char = '\u{f00b}';
    pub const SEARCH: char = '\u{f00c}';
    pub const EDIT_SQUARE: char = '\u{f00d}';
    pub const NOTIFICATIONS_OFF: char = '\u{f00e}';
    pub const IMAGE: char = '\u{f00f}';
    pub const CHEVRON_RIGHT: char = '\u{f010}';
    pub const EXPAND_MORE: char = '\u{f011}';
    pub const CLOSE: char = '\u{f012}';
    pub const KEEP: char = '\u{f013}';
    pub const DOWNLOAD: char = '\u{f014}';
    pub const MARK_EMAIL_UNREAD: char = '\u{f015}';
    pub const SMS: char = '\u{f016}';
    pub const MODE_COMMENT: char = '\u{f017}';
    pub const PERSON: char = '\u{f018}';
    pub const GROUP: char = '\u{f019}';
    pub const CHECK: char = '\u{f01a}';
    pub const REPLY: char = '\u{f01b}';
    pub const MOOD: char = '\u{f01c}';
    pub const VIEW_SIDEBAR: char = '\u{f01d}';
    /// Lucide's `chevron-right`, which is the Feather `ChevronRight` of Desktop's submenu rows.
    pub const CHEVRON: char = '\u{e06f}';
    pub const CLOUD: char = '\u{e088}';
    pub const CIRCLE_PLUS: char = '\u{e081}';
    pub const LINK: char = '\u{e102}';
    pub const ARROW_UP_RIGHT: char = '\u{e04d}';
    pub const PLUS: char = '\u{e13d}';
    pub const AUDIO_LINES: char = '\u{e55a}';
    pub const AT_SIGN: char = '\u{e04e}';
    pub const PAINTBRUSH: char = '\u{e2e7}';
    pub const PUZZLE: char = '\u{e29c}';
    pub const DATABASE: char = '\u{e0ad}';
    pub const INFO: char = '\u{e0f9}';
    pub const PLAY: char = '\u{e13c}';
    pub const HISTORY: char = '\u{e1f5}';
}

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!("../assets/fonts/", $file)).as_slice()
    };
}

pub const FONT_EMOJI: &[u8] = font!("Twemoji.Mozilla.ttf");
/// The Inter 3.18 WOFF files of Desktop. WOFF keeps the binary 1.3 MB smaller.
const INTER: [&[u8]; 8] = [
    font!("Inter-Regular.woff"),
    font!("Inter-Italic.woff"),
    font!("Inter-Medium.woff"),
    font!("Inter-MediumItalic.woff"),
    font!("Inter-SemiBold.woff"),
    font!("Inter-SemiBoldItalic.woff"),
    font!("Inter-Bold.woff"),
    font!("Inter-BoldItalic.woff"),
];
/// Desktop's `redacted_regular` font of "Redact names and pictures", whose letters are bars.
const REDACTED_WOFF: &[u8] = font!("Redacted-Regular.woff");
pub const CLEAR: Color = Color::TRANSPARENT;

thread_local! {
    /// While a view draws: Desktop's `redactedMode`, and whether the widgets that it makes are in a
    /// `.redacted-region`.
    static REDACT: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
}

/// Starts the view of a window with Desktop's `redactedMode` on or off, outside any region.
pub fn redacted_mode(on: bool) {
    REDACT.set((on, false));
}

/// Makes `f`'s widgets in a `.redacted-region` (`on`) or in a `.negate-redacted-region`.
pub fn region<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let (mode, was) = REDACT.get();
    REDACT.set((mode, on));
    let r = f();
    REDACT.set((mode, was));
    r
}

/// Whether the widgets made now are redacted: `.redacted .redacted-region`.
pub fn redacting() -> bool {
    REDACT.get() == (true, true)
}

/// `font`, or the redacted font while [`redacting`].
pub fn redact(font: Font) -> Font {
    match redacting() {
        true => Font { family: Family::Name("Redacted"), weight: Weight::Normal, ..font },
        false => font,
    }
}

static LOAD_FONTS: Once = Once::new();
static PLATFORM: cosmic_text::PlatformFallback = cosmic_text::PlatformFallback;
static COMMON_FALLBACK: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| std::iter::once(crate::emoji::EMOJI_FAMILY).chain(PLATFORM.common_fallback().iter().copied()).collect());

/// Desktop's font stack reaches Twemoji before the generic sans-serif fonts.
struct TwemojiFirst;

impl cosmic_text::Fallback for TwemojiFirst {
    fn common_fallback(&self) -> &[&'static str] {
        &COMMON_FALLBACK
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        PLATFORM.forbidden_fallback()
    }
    /// Without a CJK locale, fontconfig gives Han characters the Japanese font, so kanji and kana match.
    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        let cjk = ["zh", "ja", "ko"].iter().any(|l| locale.starts_with(l));
        match script {
            unicode_script::Script::Han if !cjk => PLATFORM.script_fallback(unicode_script::Script::Hiragana, locale),
            _ => PLATFORM.script_fallback(script, locale),
        }
    }
}

pub fn ensure_fonts_loaded() {
    LOAD_FONTS.call_once(|| {
        let mut fs = font_system().write().expect("font system");
        let empty = cosmic_text::FontSystem::new_with_locale_and_db(String::new(), Default::default());
        let (locale, db) = std::mem::replace(fs.raw(), empty).into_locale_and_db();
        *fs.raw() = cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(locale, db, TwemojiFirst);
        // The font database reads the TrueType files in the binary, and each WOFF file unpacked once.
        let db = fs.raw().db_mut();
        for ttf in [font!("icons.ttf"), font!("Sniglet-ExtraBold.ttf"), FONT_EMOJI] {
            db.load_font_source(Source::Binary(Arc::new(ttf)));
        }
        let knicknack = desktop_asars().find_map(|p| knicknack_woff(&p));
        KNICKNACK.store(knicknack.is_some(), Ordering::Relaxed);
        for woff in INTER.into_iter().chain([REDACTED_WOFF]).chain(knicknack.as_deref()) {
            db.load_font_source(Source::Binary(Arc::new(woff_to_sfnt(woff))));
        }
    });
}

/// Desktop draws avatar initials in Knicknack Bold. Its license does not let us ship it, so it loads from an
/// installed Beeper Desktop, and the initials use Sniglet ExtraBold, a free font of the same round shapes, when there
/// is none.
static KNICKNACK: AtomicBool = AtomicBool::new(false);

pub fn initials_font() -> Font {
    ensure_fonts_loaded();
    match KNICKNACK.load(Ordering::Relaxed) {
        true => Font { family: Family::Name("Knicknack"), weight: Weight::Medium, ..Font::DEFAULT },
        false => Font { family: Family::Name("Sniglet"), weight: Weight::ExtraBold, ..Font::DEFAULT },
    }
}

/// The `app.asar` archive of an installed Beeper Desktop: a Linux package, a running AppImage, Windows, or macOS.
fn desktop_asars() -> impl Iterator<Item = PathBuf> {
    let env = |k| std::env::var_os(k).map(PathBuf::from);
    let mounts = std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten().map(|e| e.path());
    let mounts = mounts.filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".mount_")));
    let opt = ["/opt/beeper", "/opt/Beeper", "/opt/BeeperTexts"].map(PathBuf::from);
    let windows = env("LOCALAPPDATA").into_iter().flat_map(|l| ["BeeperTexts", "beeper"].map(|n| l.join("Programs").join(n)));
    let mac = [Some(PathBuf::from("/")), env("HOME")].into_iter().flatten();
    let mac = mac.flat_map(|r| ["Beeper Desktop.app", "Beeper.app"].map(|a| r.join("Applications").join(a).join("Contents")));
    let resources = opt.into_iter().chain(mounts).chain(windows).map(|d| d.join("resources"));
    resources.chain(mac.map(|d| d.join("Resources"))).map(|d| d.join("app.asar"))
}

/// The Knicknack WOFF file in the `app.asar` archive, which is a JSON file index followed by the file contents.
fn knicknack_woff(asar: &Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek};
    let mut f = std::fs::File::open(asar).ok()?;
    let mut head = [0u8; 16];
    f.read_exact(&mut head).ok()?;
    let le = |i: usize| u64::from(u32::from_le_bytes(head[i..i + 4].try_into().expect("4 bytes")));
    let mut index = vec![0u8; usize::try_from(le(12)).ok()?];
    f.read_exact(&mut index).ok()?;
    let index: serde_json::Value = serde_json::from_slice(&index).ok()?;
    let files = index["files"]["build"]["files"]["renderer"]["files"].as_object()?;
    let (_, e) = files.iter().find(|(k, _)| k.starts_with("knicknack-bold-") && k.ends_with(".woff"))?;
    f.seek(std::io::SeekFrom::Start(8 + le(4) + e["offset"].as_str()?.parse::<u64>().ok()?)).ok()?;
    let mut woff = vec![0u8; usize::try_from(e["size"].as_u64()?).ok()?];
    f.read_exact(&mut woff).ok()?;
    Some(woff)
}

/// Unpacks a WOFF 1 font into the TrueType file that it wraps.
fn woff_to_sfnt(woff: &[u8]) -> Vec<u8> {
    assert_eq!(&woff[..4], b"wOFF", "a WOFF 1 font");
    let be32 = |i: usize| u32::from_be_bytes(woff[i..i + 4].try_into().expect("4 bytes"));
    let n = usize::from(u16::from_be_bytes([woff[12], woff[13]]));
    let tables: Vec<(&[u8], u32, Vec<u8>)> = (0..n)
        .map(|i| {
            let e = 44 + 20 * i;
            let (at, packed, full) = (be32(e + 4) as usize, be32(e + 8) as usize, be32(e + 12) as usize);
            let raw = &woff[at..at + packed];
            let mut data = Vec::with_capacity(full);
            match packed < full {
                true => std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(raw), &mut data)
                    .map(drop)
                    .expect("a WOFF table inflates"),
                false => data.extend_from_slice(raw),
            }
            assert_eq!(data.len(), full, "the WOFF table has its stated size");
            (&woff[e..e + 4], be32(e + 16), data)
        })
        .collect();
    let shift = n.ilog2();
    let range = 16u16 << shift;
    let mut out = woff[4..8].to_vec();
    for v in [n as u16, range, shift as u16, n as u16 * 16 - range] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut at = 12 + 16 * n;
    for (tag, sum, data) in &tables {
        out.extend_from_slice(tag);
        for v in [*sum, at as u32, data.len() as u32] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        at += data.len().next_multiple_of(4);
    }
    for (_, _, data) in tables {
        out.extend_from_slice(&data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

thread_local!(static REM: Cell<f32> = const { Cell::new(1.0) });

/// Sets Desktop's root font from the Font size: `--custom-font-size` is the Font size plus 3 px, and the root font of
/// 16 px is the default.
pub fn set_font_size(font_size: f32) {
    REM.set((font_size + 3.0) / 16.0);
}

/// A size that Desktop gives in `rem`, from its px at the default root font of 16 px: the text sizes and line heights
/// (`--semantic-typo-size-*`, `--semantic-typo-line-height-*`), which follow the Font size. The sizes in px, most
/// spaces, icons and avatars, do not, and the zoom scales both.
pub fn rem(px: f32) -> f32 {
    px * REM.get()
}

/// A CSS `line-height` of `px` at the root font size, as [`rem`] scales it.
pub fn lh(px: f32) -> iced::Pixels {
    iced::Pixels(rem(px))
}

pub const fn fw(weight: Weight) -> Font {
    Font { family: Family::Name("Inter"), weight, ..Font::DEFAULT }
}

fn shape<T>(s: &str, size: f32, w: Weight, width: Option<f32>, f: impl FnOnce(&cosmic_text::Buffer) -> T) -> T {
    ensure_fonts_loaded();
    let mut fs = font_system().write().expect("font system");
    let mut buf = cosmic_text::Buffer::new(fs.raw(), cosmic_text::Metrics::new(size, size * 1.2));
    buf.set_size(fs.raw(), width, None);
    buf.set_text(fs.raw(), s, &to_attributes(fw(w)), cosmic_text::Shaping::Advanced, None);
    f(&buf)
}

/// The most text measurements that each memo keeps: when its new half holds `MEMO`, its old half goes, so the
/// measurements of the last views stay.
const MEMO: usize = 4096;

pub(crate) type Memo<K, V> = RefCell<[HashMap<K, V>; 2]>;

/// The value of `key` in `memo`, made by `make` once while the key stays in use.
pub(crate) fn memo<K: std::hash::Hash + Eq, V: Clone>(
    memo: &'static std::thread::LocalKey<Memo<K, V>>, key: K, make: impl FnOnce() -> V,
) -> V {
    let (new, old) = memo.with(|m| {
        let [new, old] = &*m.borrow();
        (new.get(&key).cloned(), old.get(&key).cloned())
    });
    if let Some(v) = new {
        return v;
    }
    let v = old.unwrap_or_else(make);
    memo.with(|m| {
        let [new, old] = &mut *m.borrow_mut();
        if new.len() >= MEMO {
            *old = std::mem::take(new);
        }
        new.insert(key, v.clone());
    });
    v
}

/// The width of a text, and the end and the x position after each glyph.
type Glyphs = Rc<(f32, Vec<(usize, f32)>)>;

/// The glyphs of `s` at `size` px, already in rem.
fn glyphs(s: &str, size: f32, w: Weight) -> Glyphs {
    thread_local!(static MEMO: Memo<(String, u32, Weight), Glyphs> = RefCell::default());
    memo(&MEMO, (s.to_owned(), size.to_bits(), w), || {
        shape(s, size, w, None, |buf| {
            let mut out = Vec::new();
            let mut total = 0.0;
            for g in buf.layout_runs().flat_map(|r| r.glyphs.iter()) {
                total += g.w;
                out.push((g.end, total));
            }
            Rc::new((total, out))
        })
    })
}

/// The x position and wrapped row of each (line, byte) caret in `s` wrapped at `width`.
pub fn carets(s: &str, size: f32, width: f32, at: impl IntoIterator<Item = (usize, usize)>) -> Vec<(f32, usize)> {
    shape(s, rem(size), Weight::Normal, Some(width), |buf| {
        let at = at.into_iter().map(|(line, byte)| {
            let mut end = (0.0, 0);
            for (row, r) in buf.layout_runs().enumerate().filter(|(_, r)| r.line_i == line) {
                if let Some(g) = r.glyphs.iter().find(|g| g.end > byte) {
                    return (if g.start >= byte { g.x } else { g.x + g.w }, row);
                }
                end = (r.line_w, row);
            }
            end
        });
        at.collect()
    })
}

/// CSS `text-wrap: balance`: the narrowest width up to `max` that wraps `s` into as many lines as `max` does.
pub fn balanced_width(s: &str, max: f32, size: f32) -> f32 {
    let size = rem(size);
    thread_local!(static MEMO: Memo<(String, u32, u32), f32> = RefCell::default());
    memo(&MEMO, (s.to_owned(), max.to_bits(), size.to_bits()), || {
        let lines = |w: f32| shape(s, size, Weight::Normal, Some(w), |b| b.layout_runs().count());
        let n = lines(max);
        let (mut lo, mut hi) = (0.0, max);
        while hi - lo > 0.25 {
            let mid = (lo + hi) / 2.0;
            if lines(mid) > n {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        // A pixel of room, so that iced breaks the lines where the measure did.
        (hi + 1.0).min(max)
    })
}

/// Whether `s` is one line that is surely narrower than `max` without shaping it: printable ASCII, whose glyphs in
/// Inter are narrower than 1.2 em, and word joiners, which have no width.
fn surely_fits(s: &str, max: f32, size: f32) -> bool {
    let chars = s.chars().filter(|&c| c != '\u{2060}');
    let mut n = 0.0;
    for c in chars {
        if !c.is_ascii_graphic() && c != ' ' {
            return false;
        }
        n += 1.2 * size;
    }
    n <= max
}

/// Whether `s` at `size` takes more than one line at `max` width: a line of the text makes more than one layout run.
pub fn wraps(s: &str, max: f32, size: f32) -> bool {
    let size = rem(size);
    thread_local!(static MEMO: Memo<(String, u32, u32), bool> = RefCell::default());
    if surely_fits(s, max, size) {
        return false;
    }
    memo(&MEMO, (s.to_owned(), max.to_bits(), size.to_bits()), || {
        shape(s, size, Weight::Normal, Some(max), |b| {
            let mut last = None;
            b.layout_runs().any(|r| last.replace(r.line_i) == Some(r.line_i))
        })
    })
}

/// The lines of `s` wrapped at `max`, as the text widgets lay it out.
pub fn line_count(s: &str, max: f32, size: f32) -> usize {
    let size = rem(size);
    thread_local!(static MEMO: Memo<(String, u32, u32), usize> = RefCell::default());
    if surely_fits(s, max, size) {
        return 1;
    }
    memo(&MEMO, (s.to_owned(), max.to_bits(), size.to_bits()), || shape(s, size, Weight::Normal, Some(max), |b| b.layout_runs().count()))
}

pub fn text_width(s: &str, size: f32, w: Weight) -> f32 {
    glyphs(s, rem(size), w).0
}

fn ell_line(s: &str, max: f32, size: f32, w: Weight) -> String {
    let gs = glyphs(s, size, w);
    let (total, gs) = (gs.0, &gs.1);
    if total <= max {
        return s.into();
    }
    let cut = max - glyphs("…", size, w).0;
    let end = gs.get(gs.partition_point(|g| g.1 <= cut).wrapping_sub(1)).map_or(0, |g| g.0);
    format!("{}…", &s[..end])
}

pub fn ell<'a>(s: &str, max: f32, size: f32, w: Weight) -> text::Text<'a> {
    clamp(s, max, 1, size, w)
}

/// Collapses whitespace like CSS and keeps at most `lines` lines of `max` width, with "…" at the end of the last line.
pub fn clamp<'a>(s: &str, max: f32, lines: usize, size: f32, w: Weight) -> text::Text<'a> {
    t(clamped(s, max, lines, size, w).to_string(), size, w).wrapping(if lines == 1 {
        text::Wrapping::None
    } else {
        text::Wrapping::WordOrGlyph
    })
}

pub fn clamped(s: &str, max: f32, lines: usize, size: f32, w: Weight) -> Rc<str> {
    thread_local!(static MEMO: Memo<(String, u32, u32, Weight, usize), Rc<str>> = RefCell::default());
    assert!(lines > 0, "clamp to zero lines");
    let size = rem(size);
    memo(&MEMO, (s.to_owned(), max.max(0.0).to_bits(), size.to_bits(), w, lines), || {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let starts: Vec<usize> =
            shape(&s, size, w, Some(max.max(0.0)), |buf| buf.layout_runs().map(|r| r.glyphs.first().map_or(0, |g| g.start)).collect());
        match starts.get(lines - 1) {
            Some(&k) if starts.len() > lines => format!("{}{}", &s[..k], ell_line(&s[k..], max, size, w)).into(),
            _ => s.into(),
        }
    })
}

/// CSS inline flow of `s` after `indent` px of inline boxes in a `max` wide box of `lines` lines: the words that
/// fit on the first line, and the rest clamped to the other lines.
pub fn flow(s: &str, indent: f32, max: f32, lines: usize, size: f32, w: Weight) -> (String, Rc<str>) {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let gs = glyphs(&s, rem(size), w);
    let gs = &gs.1;
    let width = |i: usize| gs.get(gs.partition_point(|g| g.0 <= i).wrapping_sub(1)).map_or(0.0, |g| g.1);
    let breaks = s.match_indices(' ').map(|(i, _)| i).chain([s.len()]);
    let fit = breaks.take_while(|&i| width(i) <= max - indent).last().unwrap_or(0);
    (s[..fit].to_string(), clamped(s[fit..].trim_start(), max, lines - 1, size, w))
}

/// Text in the app font at `size` px in rem. Its line height of `round(size * 1.24)` is Desktop's
/// `--semantic-typo-line-height-*` for the 10, 11, 12, 13 and 16 px sizes, so only other pairs (14/18, 15/20, 17/24,
/// ...) set their own.
pub fn t<'a>(s: impl text::IntoFragment<'a>, size: f32, w: Weight) -> text::Text<'a> {
    text(s).size(rem(size)).font(redact(fw(w))).line_height(text::LineHeight::Absolute(rem((size * 1.24).round()).into()))
}

/// Text at `size` px that Desktop gives in px, like `.message .timestamp`, which the Font size does not change.
pub fn fixed<'a>(s: impl text::IntoFragment<'a>, size: f32, w: Weight) -> text::Text<'a> {
    text(s).size(size).font(redact(fw(w))).line_height(text::LineHeight::Absolute((size * 1.24).round().into()))
}

/// A [`fixed`] text on one line of at most `max` width, with "…" at its end when it is wider, like [`ell`].
pub fn fixed_ell<'a>(s: &str, max: f32, size: f32, w: Weight) -> text::Text<'a> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    fixed(ell_line(&s, max, size, w), size, w).wrapping(text::Wrapping::None)
}

/// The width of a [`fixed`] text.
pub fn fixed_width(s: &str, size: f32, w: Weight) -> f32 {
    glyphs(s, size, w).0
}

pub fn txt<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    t(s, size, Weight::Normal)
}

pub fn med<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    t(s, size, Weight::Medium)
}

pub fn semi<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    t(s, size, Weight::Semibold)
}

pub fn bold<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    t(s, size, Weight::Bold)
}

pub fn ic(c: char, size: f32) -> text::Text<'static> {
    text(c).font(Font::with_name("icons")).size(size).line_height(1.0)
}

pub const AVATAR_GRADIENTS: [(u32, u32); 20] = [
    (0x5634de, 0x7e61f2),
    (0x7d67d5, 0xa791ff),
    (0xd833ca, 0xf86fdb),
    (0xdf6f7c, 0xef939e),
    (0xe8126b, 0xff256e),
    (0xcd1557, 0xff657b),
    (0xee1f34, 0xff4658),
    (0xde4b19, 0xfa6533),
    (0xc25e3c, 0xfc9775),
    (0xbb7c55, 0xdba685),
    (0x8d6954, 0x9f8778),
    (0xc2891f, 0xffb938),
    (0xd99c00, 0xffe500),
    (0x6bad16, 0xb4fb44),
    (0x0f9240, 0x25ca63),
    (0x45a654, 0x6fe582),
    (0x07a06d, 0x06cf9c),
    (0x239084, 0x42c6b8),
    (0x0a81ac, 0x29b0e1),
    (0x1e54e1, 0x5583fc),
];

pub fn name_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

pub fn pick<'a, T>(s: &str, list: &'a [T]) -> &'a T {
    &list[name_hash(s).rem_euclid(i32::try_from(list.len()).expect("short list")) as usize]
}

pub fn initials(name: &str) -> Option<String> {
    let letters: String = name.chars().filter(|c| c.is_alphabetic() || c.is_whitespace()).collect();
    let firsts = letters.split(' ').filter(|w| !w.is_empty()).take(2).filter_map(|w| w.chars().next());
    Some(firsts.flat_map(char::to_uppercase).collect::<String>()).filter(|s| !s.is_empty())
}

pub fn mix(a: Color, b: Color, k: f32) -> Color {
    Color::from_rgba(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k, a.a + (b.a - a.a) * k)
}

/// The opaque color of `c` drawn over `bg`.
pub fn flat(bg: Color, c: Color) -> Color {
    mix(bg, Color { a: 1.0, ..c }, c.a)
}

macro_rules! brands {
    ($($id:literal),*) => {
        [$(($id, include_str!(concat!("../assets/brands/", $id, ".svg")), include_str!(concat!("../assets/brands/badge/", $id, ".svg")))),*]
    };
}

const BRANDS: [(&str, &str, &str); 26] = brands!(
    "whatsapp",
    "twitter",
    "telegram",
    "slack",
    "discord",
    "facebook",
    "instagram",
    "linkedin",
    "gmessages",
    "signal",
    "googlechat",
    "line",
    "gvoice",
    "tumblr",
    "viber",
    "snapchat",
    "groupme",
    "wechat",
    "reddit",
    "chatgpt-dots",
    "imessage",
    "grokbot",
    "muse",
    "bluesky",
    "hungryserv",
    // Desktop's `fP`, the brand of a merged chat.
    "merged-chat"
);

static BRAND_SVGS: LazyLock<HashMap<&'static str, [svg::Handle; 4]>> = LazyLock::new(|| {
    assert!(NETWORK_SPECS.iter().all(|n| BRANDS.iter().any(|b| b.0 == n.bridge_id)), "every network has a logo");
    BRANDS
        .iter()
        .map(|(id, tile, badge)| {
            assert!(tile.contains(r#"rx="9.8""#), "{id} has the tile clip");
            let round = tile.replacen(r#"rx="9.8""#, r#"rx="14""#, 1);
            let svgs = [tile.to_string(), badge.to_string(), round, grayscale(tile)];
            (*id, svgs.map(|s| svg::Handle::from_memory(s.into_bytes())))
        })
        .collect()
});

/// An SVG with its `#rgb` and `#rrggbb` colors made gray like CSS `filter: grayscale(100%)`, whose luminance takes
/// 0.2126 of red, 0.7152 of green and 0.0722 of blue.
fn grayscale(svg: &str) -> String {
    static HEX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"#([0-9a-fA-F]{6}|[0-9a-fA-F]{3})\b").expect("the color pattern compiles"));
    HEX.replace_all(svg, |c: &regex::Captures| {
        let h = &c[1];
        let channel = |i: usize| match h.len() {
            3 => u8::from_str_radix(&h[i..=i], 16).expect("a hex digit") * 17,
            _ => u8::from_str_radix(&h[2 * i..2 * i + 2], 16).expect("hex digits"),
        };
        let y = 0.2126 * f32::from(channel(0)) + 0.7152 * f32::from(channel(1)) + 0.0722 * f32::from(channel(2));
        format!("#{0:02x}{0:02x}{0:02x}", y.round() as u8)
    })
    .into_owned()
}

/// A QR code as an SVG: black modules on white, with a white `margin` of modules around them.
pub fn qr_svg(code: &qrcodegen::QrCode, margin: i32) -> svg::Handle {
    let (n, side) = (code.size(), code.size() + 2 * margin);
    let mut path = String::new();
    for y in 0..n {
        for x in (0..n).filter(|&x| code.get_module(x, y)) {
            path.push_str(&format!("M{},{}h1v1h-1z", x + margin, y + margin));
        }
    }
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" shape-rendering="crispEdges"><path fill="#fff" d="M0,0h{side}v{side}H0z"/><path fill="#000" d="{path}"/></svg>"##
    );
    svg::Handle::from_memory(svg.into_bytes())
}

/// Desktop's `BrandHeading` with `gradient`: one line of Inter Bold at `size` px with the `-0.75` px letter spacing
/// of `--semantic-typo-letter-spacing-display-md`, filled with `linear-gradient(180deg, #9e7ef8, #6a4be5)` over a
/// line box of `line` px. iced text has no gradient and no letter spacing, so the glyph outlines go into an SVG,
/// made once for each text. Returns the SVG and its width.
pub fn gradient_text(s: &str, size: f32, line: f32) -> (svg::Handle, f32) {
    use cosmic_text::Command::{Close, CurveTo, LineTo, MoveTo, QuadTo};
    const SPACING: f32 = -0.75;
    thread_local!(static MEMO: Memo<(String, u32, u32), (svg::Handle, f32)> = RefCell::default());
    memo(&MEMO, (s.to_owned(), size.to_bits(), line.to_bits()), || {
        ensure_fonts_loaded();
        let mut fs = font_system().write().expect("font system");
        let mut buf = cosmic_text::Buffer::new(fs.raw(), cosmic_text::Metrics::new(size, line));
        buf.set_text(fs.raw(), s, &to_attributes(fw(Weight::Bold)), cosmic_text::Shaping::Advanced, None);
        let mut swash = cosmic_text::SwashCache::new();
        let (mut d, mut width) = (String::new(), 0.0);
        let runs: Vec<_> = buf.layout_runs().map(|r| (r.line_y, r.line_w, r.glyphs.to_vec())).collect();
        assert_eq!(runs.len(), 1, "a heading is one line: {s}");
        for (baseline, w, glyphs) in runs {
            width = w + SPACING * glyphs.len() as f32;
            for (i, g) in glyphs.iter().enumerate() {
                let x = g.x + g.font_size * g.x_offset + SPACING * i as f32;
                let y = baseline - g.font_size * g.y_offset;
                let key = g.physical((0.0, 0.0), 1.0).cache_key;
                let p = |vx: f32, vy: f32| format!("{:.2} {:.2}", x + vx, y - vy);
                for c in swash.get_outline_commands(fs.raw(), key).expect("Inter has outlines") {
                    d.push_str(&match *c {
                        MoveTo(a) => format!("M{}", p(a.x, a.y)),
                        LineTo(a) => format!("L{}", p(a.x, a.y)),
                        QuadTo(a, b) => format!("Q{} {}", p(a.x, a.y), p(b.x, b.y)),
                        CurveTo(a, b, e) => format!("C{} {} {}", p(a.x, a.y), p(b.x, b.y), p(e.x, e.y)),
                        Close => "Z".into(),
                    });
                }
            }
        }
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {line}"><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="{line}"><stop stop-color="#9e7ef8"/><stop offset="1" stop-color="#6a4be5"/></linearGradient><path fill="url(#g)" d="{d}"/></svg>"##
        );
        (svg::Handle::from_memory(svg.into_bytes()), width)
    })
}

/// The scale of a dimmed emoji image, so that it stays sharp on a 2x screen.
const DIM_SCALE: f32 = 2.0;

/// Emoji text `s` at `size` px in a `line` px line box, with CSS `filter: brightness(k)`, as an image and the size
/// of its text box. iced draws color glyphs without a tint, so the glyphs go into an image, made once for each text.
pub fn dimmed_emoji(s: &str, size: f32, line: f32, k: f32) -> (iced::widget::image::Handle, iced::Size) {
    use iced::widget::image::Handle;
    thread_local!(static MEMO: Memo<(String, u32, u32, u32), (Handle, iced::Size)> = RefCell::default());
    memo(&MEMO, (s.to_owned(), size.to_bits(), line.to_bits(), k.to_bits()), || {
        ensure_fonts_loaded();
        let mut fs = font_system().write().expect("font system");
        let metrics = cosmic_text::Metrics::new(size * DIM_SCALE, line * DIM_SCALE);
        let mut buf = cosmic_text::Buffer::new(fs.raw(), metrics);
        let attrs = to_attributes(crate::emoji::EMOJI_FONT);
        buf.set_text(fs.raw(), s, &attrs, cosmic_text::Shaping::Advanced, None);
        let runs: Vec<_> = buf.layout_runs().map(|r| (r.line_y, r.line_w, r.glyphs.to_vec())).collect();
        let width = runs.iter().map(|r| r.1).fold(0.0, f32::max).ceil().max(1.0);
        let height = (runs.len().max(1) as f32 * line * DIM_SCALE).ceil();
        let mut out = ::image::RgbaImage::new(width as u32, height as u32);
        let mut swash = cosmic_text::SwashCache::new();
        for (baseline, _, glyphs) in runs {
            for g in glyphs {
                let glyph = g.physical((0.0, baseline), 1.0);
                let Some(img) = swash.get_image_uncached(fs.raw(), glyph.cache_key) else { continue };
                let (w, h) = (img.placement.width as i32, img.placement.height as i32);
                let (x0, y0) = (glyph.x + img.placement.left, glyph.y - img.placement.top);
                for (i, (y, x)) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).enumerate() {
                    let (px, py) = (x0 + x, y0 + y);
                    if px < 0 || py < 0 || px >= out.width() as i32 || py >= out.height() as i32 {
                        continue;
                    }
                    // A glyph without colors is black, as the jumbo emoji are only emoji.
                    let src = match img.content {
                        cosmic_text::SwashContent::Color => [0, 1, 2, 3].map(|c| img.data[i * 4 + c]),
                        _ => [0, 0, 0, img.data[i]],
                    };
                    let dst = out.get_pixel_mut(px as u32, py as u32);
                    let (sa, da) = (f32::from(src[3]) / 255.0, f32::from(dst[3]) / 255.0);
                    let a = sa + da * (1.0 - sa);
                    for c in 0..3 {
                        let v = (f32::from(src[c]) * sa + f32::from(dst[c]) * da * (1.0 - sa)) / a.max(f32::EPSILON);
                        dst[c] = v.round() as u8;
                    }
                    dst[3] = (a * 255.0).round() as u8;
                }
            }
        }
        for px in out.pixels_mut() {
            for c in 0..3 {
                px[c] = (f32::from(px[c]) * k).round() as u8;
            }
        }
        let handle = Handle::from_rgba(out.width(), out.height(), out.into_raw());
        (handle, iced::Size::new(width / DIM_SCALE, height / DIM_SCALE))
    })
}

/// A decoded image with CSS `filter: brightness(k)`, made once for each image.
pub fn dimmed_image(h: &iced::widget::image::Handle, k: f32) -> iced::widget::image::Handle {
    use iced::advanced::image::{Handle, Id};
    thread_local!(static MEMO: Memo<(Id, u32), Handle> = RefCell::default());
    let Handle::Rgba { width, height, pixels, .. } = h else { return h.clone() };
    memo(&MEMO, (h.id(), k.to_bits()), || {
        let dim = |c: u8| (f32::from(c) * k).round() as u8;
        let pixels: Vec<u8> = pixels.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [dim(r), dim(g), dim(b), a]).collect();
        Handle::from_rgba(*width, *height, pixels)
    })
}

pub fn brand_svg(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][0].clone()
}

/// The logo of a network that cannot take an account, like Desktop's `networkTileBlocked` (`grayscale(100%)`).
pub fn brand_gray(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][3].clone()
}

pub fn brand_badge(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][1].clone()
}

/// The logo clipped to a circle, like Desktop's `.ThreadListItem-module__platformIcon .is-badge`.
pub fn brand_round(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][2].clone()
}

/// CSS `padding: top right bottom left`.
pub const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> iced::Padding {
    iced::Padding { top, right, bottom, left }
}

pub fn fill_box(bg: Color, r: impl Into<border::Radius>) -> impl Fn(&Theme) -> container::Style + Copy {
    let r = r.into();
    move |_| container::Style { background: Some(bg.into()), border: border::rounded(r), ..Default::default() }
}

pub fn bordered_box(bg: Color, border_col: Color, width: f32, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width, radius: r.into() },
        ..Default::default()
    }
}

/// A box with a CSS `box-shadow: 0 <y> <blur> <glow>`.
pub fn glow_box(bg: Color, glow: Color, r: f32, y: f32, blur: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { radius: r.into(), ..Border::default() },
        shadow: Shadow { color: glow, offset: Vector::new(0.0, y), blur_radius: blur },
        text_color: None,
        snap: false,
    }
}

/// Desktop's `.right` card of the open chat: rounded 10px, with a soft shadow all around.
pub fn pane_card(bg: Color) -> impl Fn(&Theme) -> container::Style + Copy {
    move |t| container::Style {
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.18), offset: Vector::ZERO, blur_radius: 20.0 },
        ..fill_box(bg, 10.0)(t)
    }
}

pub fn shadow_box(bg: Color, border_col: Color, r: f32, blur: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width: 1.0, radius: r.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: Vector::new(0.0, 4.0), blur_radius: blur },
        text_color: None,
        snap: false,
    }
}

/// A button without padding or background, its content in `fg` also when hovered or pressed: a text link or a bare
/// icon.
pub trait Bare {
    fn bare(self, fg: Color) -> Self;
}

impl<M> Bare for button::Button<'_, M> {
    fn bare(self, fg: Color) -> Self {
        self.padding(0).style(btn_style([CLEAR; 3], [fg; 3], 0.0))
    }
}

pub fn btn_style(bgs: [Color; 3], fgs: [Color; 3], r: f32) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    use button::Status::*;
    move |_, s| {
        let k = usize::from(matches!(s, Hovered)) + 2 * usize::from(matches!(s, Pressed));
        let a = if matches!(s, Disabled) { 0.45 } else { 1.0 };
        button::Style {
            background: Some(bgs[k].scale_alpha(a).into()),
            text_color: fgs[k].scale_alpha(a),
            border: border::rounded(r),
            ..Default::default()
        }
    }
}

pub fn bordered_btn(
    bgs: [Color; 3], fgs: [Color; 3], border_col: Color, width: f32, r: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    move |t, s| button::Style { border: Border { color: border_col, width, radius: r.into() }, ..btn_style(bgs, fgs, r)(t, s) }
}

/// A text area on the chip color with corners of `r`, like Desktop's description and notes fields.
pub fn editor_style(p: &Palette, r: f32) -> impl Fn(&Theme, text_editor::Status) -> text_editor::Style + Copy {
    let (bg, placeholder, value, selection) = (p.chip, p.sub, p.strong, p.accent.scale_alpha(0.3));
    move |_, _| text_editor::Style { background: bg.into(), border: border::rounded(r), placeholder, value, selection }
}

/// Desktop's Done button of a sheet: a white label in an accent pill.
pub fn done_btn<'a, M: Clone + 'a>(p: &Palette) -> button::Button<'a, M> {
    button(med(crate::i18n::tr("Done"), 13.0).color(Color::WHITE)).padding([6, 14]).style(btn_style(
        [p.accent; 3],
        [Color::WHITE; 3],
        999.0,
    ))
}

pub fn input_style(
    bg: Color, text_col: Color, placeholder_col: Color, accent_col: Color, r: f32,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + Copy {
    move |_, _| text_input::Style {
        background: bg.into(),
        border: Border { color: CLEAR, width: 0.0, radius: r.into() },
        icon: placeholder_col,
        placeholder: placeholder_col,
        value: text_col,
        selection: accent_col.scale_alpha(0.45),
    }
}

pub fn pick_style(
    bg: Color, hover_bg: Color, text_col: Color, sub_col: Color,
) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style + Copy {
    move |_, status| pick_list::Style {
        text_color: text_col,
        placeholder_color: sub_col,
        handle_color: sub_col,
        background: (if matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) { hover_bg } else { bg }).into(),
        border: Border { color: CLEAR, width: 0.0, radius: 12.0.into() },
    }
}

pub fn pick_menu_style(bg: Color, hover_bg: Color, text_col: Color, border_col: Color) -> impl Fn(&Theme) -> menu::Style + Copy {
    move |_| menu::Style {
        background: bg.into(),
        border: Border { color: border_col, width: 1.0, radius: 8.0.into() },
        text_color: text_col,
        selected_text_color: text_col,
        selected_background: hover_bg.into(),
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 4.0), blur_radius: 14.0 },
    }
}

/// Desktop's `custom-scrollbars`: solid scrollbars, except where macOS uses overlay scrollbars.
pub const SOLID_BARS: bool = !cfg!(target_os = "macos");

/// The width of Desktop's solid scrollbar gutter.
pub const GUTTER: f32 = if SOLID_BARS { 12.0 } else { 0.0 };

/// Desktop's vertical scrollbar: a 12 px gutter with a 6 px thumb when the content overflows.
pub fn bar() -> scrollable::Direction {
    let solid = scrollable::Scrollbar::new().width(GUTTER).scroller_width(6).spacing(0);
    scrollable::Direction::Vertical(if SOLID_BARS { solid } else { scrollable::Scrollbar::default() })
}

/// `scrollbar-gutter: stable`: the thumb floats over a right padding of [`GUTTER`] that the caller adds.
pub fn stable_bar() -> scrollable::Direction {
    let solid = scrollable::Scrollbar::new().width(GUTTER).scroller_width(6);
    scrollable::Direction::Vertical(if SOLID_BARS { solid } else { scrollable::Scrollbar::default() })
}

/// Desktop's thumb: `base` at 30%, and at 50% when the pointer is on it.
pub fn scroll_style(base: Color) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + Copy {
    move |_, status| {
        let alpha = match status {
            scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. } | scrollable::Status::Dragged { .. } => 0.5,
            scrollable::Status::Active { .. } if !SOLID_BARS => 0.0,
            _ => 0.3,
        };
        rail_style(CLEAR, base.scale_alpha(alpha))
    }
}

pub fn gutter_style([track, thumb]: [Color; 2]) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + Copy {
    move |_, _| rail_style(track, thumb)
}

pub(crate) fn rail_style(track: Color, thumb: Color) -> scrollable::Style {
    let rail = |bg: Color, c: Color| scrollable::Rail {
        background: Some(bg.into()),
        border: Border::default(),
        scroller: scrollable::Scroller { background: c.into(), border: border::rounded(4) },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail(track, thumb),
        horizontal_rail: rail(CLEAR, CLEAR),
        gap: None,
        auto_scroll: scrollable::AutoScroll { background: CLEAR.into(), border: Border::default(), shadow: Shadow::default(), icon: CLEAR },
    }
}

pub const fn hex(v: u32) -> Color {
    Color::from_rgb(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0)
}

const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

/// One row for each color: the Desktop 4.3.308 CSS token, its light value and its dark value. Linux always has
/// `body.no-transparency`, so the tokens are the opaque ones.
macro_rules! palette {
    ($($(#[$doc:meta])* $field:ident: $ty:ty = $light:expr, $dark:expr;)*) => {
        #[derive(Debug, Clone, Copy)]
        pub struct Palette { $($(#[$doc])* pub $field: $ty,)* }
        pub const BEEPER_LIGHT: Palette = Palette { $($field: $light,)* };
        pub const BEEPER_DARK: Palette = Palette { $($field: $dark,)* };
    };
}

palette! {
    /// `--left-pane-bg`: the window behind the chat list and the chat, and `--transparency-main-window-bg` of the
    /// modal windows. A theme color makes `--left-pane-bg` transparent over its [`Self::backdrop`].
    window: Color = hex(0xeaebf0), hex(0x1c1d1e);
    /// The gradient of the theme color (`ui_theme`) behind the transparent `--left-pane-bg`.
    backdrop: Option<crate::ui_theme::Backdrop> = None, None;
    /// `--appearance-color-bg-primary`.
    card: Color = hex(0xffffff), hex(0x000000);
    /// `--color-background-selected`.
    selected: Color = hexa(0x000000, 0.1), hexa(0xffffff, 0.1);
    /// `--color-backing-shade`: hover fills, the account rail and the selected settings tab.
    hover: Color = hexa(0x202124, 0.1), hexa(0xf6f6f6, 0.1);
    /// `--color-text-neutrals`.
    text: Color = hex(0x171717), hex(0xededed);
    /// `--color-text-primary`.
    fg: Color = hex(0x000000), hex(0xffffff);
    /// `--color-on-surface`.
    strong: Color = hex(0x202124), hex(0xf6f6f6);
    /// `--color-text-secondary`.
    sub: Color = hexa(0x3c3c44, 0.69), hexa(0xebebf5, 0.63);
    /// `--color-text-neutrals-weak`.
    weak: Color = hex(0x595959), hex(0xadadad);
    /// `--color-on-surface-variant`.
    muted: Color = hex(0x808080), hex(0x808080);
    /// `--color-text-tertiary`.
    tertiary: Color = hexa(0x3c3c44, 0.38), hexa(0xebebf5, 0.27);
    /// `--color-on-surface` at `--divider-opacity`.
    divider: Color = hexa(0x202124, 0.2), hexa(0xf6f6f6, 0.2);
    /// `--color-background-elevated`.
    elevated: Color = hex(0xffffff), hex(0x232323);
    /// `--color-background-elevated-hover`.
    elevated_hover: Color = hex(0xededed), hex(0x2e2e2e);
    /// `--color-border-neutrals`.
    border_neutrals: Color = hex(0xe2e2e2), hex(0x343434);
    /// `--color-primary-container`.
    primary_container: Color = hex(0xffffff), hex(0x1c1c1d);
    /// `--color-tertiary-container`: the composer.
    input: Color = hex(0xf4f4f5), hex(0x1c1c1d);
    /// `--color-bg-fill-primary`: the fields of the bridge login form.
    fill_primary: Color = hexa(0x000000, 0.06), hexa(0xffffff, 0.1);
    /// `--color-border-input-active`: the border of a focused field.
    input_active: Color = hex(0xc7c7c7), hex(0x505050);
    /// `--bubble-out-bg`.
    sent: Color = hex(0x0d4ffb), hex(0x2561fb);
    /// `--color-text-on-accent-weak`: the metadata of sent bubbles.
    sent_meta: Color = hexa(0xffffff, 0.7), hexa(0xffffff, 0.7);
    /// `--bubble-out-fg`.
    sent_fg: Color = hex(0xffffff), hex(0xffffff);
    /// `--color-secondary-container`: received bubbles.
    received: Color = hex(0xefeff0), hex(0x232326);
    /// `--color-background-message-bubble-received`: link previews.
    link_card: Color = hex(0xededed), hex(0x282828);
    /// `--color-outline`.
    rule: Color = hex(0xb3b3b3), hex(0x4c4c4c);
    /// `--color-primary`.
    accent: Color = hex(0x0d4ffb), hex(0x2561fb);
    /// `--toggle-style-knob-on`.
    toggle_on: Color = hex(0x0d4ffb), hex(0x0d4ffb);
    /// The fill of a checked `Checkbox`, `--checkbox-style-bg-selected`.
    checked: Color = hex(0x0e4df2), hex(0x0e4df2);
    /// `--color-highlight-reaction`: your own reactions, which a monochrome theme fills with its backdrop.
    highlight: Color = hex(0xcbd9ff), hex(0x1b43aa);
    /// The color of the quote of your own message (`LinkedMessage` `isSender`) under a rich theme, else `None`.
    quote: Option<Color> = None, None;
    /// The `color` of `.message-download-btn` and `.Transcribe-module__button`.
    action: Color = hex(0x0d4ffb), hex(0x2561fb);
    /// `--color-brand`: the text and border of Desktop's secondary `BrandButton`.
    brand: Color = hex(0x0d4ffb), hex(0x0d4ffb);
    /// The colors of a monochrome theme's `--gradient-brand` at -7.24% and 109.66% of its 190 degree line, or `None`
    /// for Desktop's own.
    brand_stops: Option<[Color; 2]> = None, None;
    /// The text of the white unread counts, `AccountBadge` dots and pinned bubbles of a dark rich theme.
    badge: Option<Color> = None, None;
    /// `--toggle-style-border` and `--toggle-style-knob-off`.
    toggle_off: Color = hexa(0x000000, 0.302), hexa(0xffffff, 0.302);
    /// `--color-background-grouped-weak`.
    prefs_pane: Color = hex(0xf3f3f3), hex(0x161616);
    /// Chromium's own scrollbar track and thumb, which the Settings pane keeps.
    gutter: [Color; 2] = [hex(0xf1f1f1), hex(0xc1c1c1)], [hex(0x2c2c2c), hex(0x9f9f9f)];
    /// `--color-background-scrollbar` without its alpha.
    scroll: Color = hex(0x000000), hex(0xffffff);
    /// `--color-background-grouped`.
    prefs_group: Color = hex(0xffffff), hex(0x1c1c1c);
    /// `--color-border-neutrals-weak`.
    prefs_border: Color = hex(0xededed), hex(0x282828);
    /// `--color-border-neutrals`.
    neutral_border: Color = hex(0xe2e2e2), hex(0x343434);
    /// `--color-on-surface` at 0.05: the Settings search field.
    search: Color = hexa(0x202124, 0.05), hexa(0xf6f6f6, 0.05);
    /// `--color-secondary-container`.
    chip: Color = hex(0xefeff0), hex(0x232326);
    /// `--color-text-info`.
    link: Color = hex(0x0069db), hex(0x5cadff);
    /// `--color-background-button-secondary`.
    button: Color = hex(0xffffff), hex(0x505050);
    /// `--color-border-button-secondary`.
    button_border: Color = hex(0xe2e2e2), hex(0x5e5e5e);
    /// `--button-primary-main-bg`: the fill of Desktop's primary `Button`.
    main: Color = hex(0x0d4ffb), hex(0x0d4ffb);
    /// `--button-secondary-main-bg`: the fill of Desktop's secondary `Button`.
    main_muted: Color = hexa(0x0d4ffb, 0.14), hexa(0x0d4ffb, 0.26);
    /// `--button-secondary-main-text`.
    main_text: Color = hex(0x0d4ffb), hex(0x638efc);
    /// `--app-focus-ring`, which is `--semantic-colors-bg-fill-main`: the `:focus-visible` outline of Desktop's
    /// `Button`, `Toggle` and `Checkbox`.
    focus_ring: Color = hex(0x0d4ffb), hex(0x0d4ffb);
    /// `--appearance-color-bg-secondary`: the fill of Desktop's action buttons.
    bg_secondary: Color = hex(0xeaebf0), hex(0x1c1d1e);
    /// `--appearance-color-border-secondary`.
    border_secondary: Color = hex(0xdedfe3), hexa(0xffffff, 0.1);
    /// Chromium's own `input[type=range]` fill, track and track edge.
    range: Color = hex(0x0075ff), hex(0x99c8ff);
    range_track: Color = hex(0xefefef), hex(0x3b3b3b);
    range_edge: Color = hex(0x767676), hex(0x858585);
    /// `--color-background-menu-opaque`.
    menu: Color = hex(0xededed), hex(0x232323);
    /// `--color-border-translucent-weak`.
    border_weak: Color = hexa(0x000000, 0.05), hexa(0xffffff, 0.05);
    /// `--color-border-translucent-strong`.
    menu_border: Color = hexa(0x000000, 0.15), hexa(0xffffff, 0.15);
    /// The fill of `--skeleton-bg`, the box of a media that loads.
    skeleton: Color = hexa(0x000000, 0.1), hexa(0xffffff, 0.15);
    /// `--color-background-menu-option-hover`.
    menu_hover: Color = hex(0x007aff), hex(0x005bb8);
    /// `--color-on-secondary-container`.
    on_secondary: Color = hex(0x828286), hex(0x9b9ba2);
    /// `--color-text-neutrals-subtle`.
    subtle: Color = hex(0x858585), hex(0x7e7e7e);
    /// `--color-surface-dim`.
    surface_dim: Color = hex(0xebebeb), hex(0x000000);
    /// The color of `BrandReplyLineSegment`.
    reply_line: Color = hex(0xe1e1e1), hex(0x353535);
    /// `--color-text-danger`: the errors of the sign-in window.
    text_danger: Color = hex(0xcd2b31), hex(0xff6369);
    /// `--appearance-color-text-danger`.
    danger: Color = hex(0xff383c), hex(0xff383c);
    /// The color of `--shadow-medium-15` and `--shadow-large-15`: toasts and tooltips.
    shadow: Color = hexa(0x000000, 0.15), hexa(0xffffff, 0.2);
    /// `--audio-bar-border`, `--audio-bar-bg`, `--compose-message-accent` and `--color-on-secondary-container`:
    /// `AudioBar`'s ring while recording, its ring while paused, its text, and its live waveform.
    audio_bar: [Color; 4] = [hex(0xe3e9f2), hex(0xf1f4f8), hex(0x7f7f7f), hex(0x828286)],
        [hex(0x606366), hex(0x303236), hex(0xd8d8d8), hex(0x9b9ba2)];
    /// The avatar name colors.
    names: [u32; 20] = [
        0x6a4be5, 0x9745ea, 0xd833ca, 0xdf6f7c, 0xe8126b, 0xcd1557, 0xee1f34, 0xde4b19, 0xc25e3c, 0xbb7c55, 0x8d6954,
        0xc2891f, 0xd99c00, 0x6bad16, 0x0f9240, 0x45a654, 0x07a06d, 0x239084, 0x0a81ac, 0x1e54e1,
    ], [
        0x9e7ef8, 0xce7fff, 0xf86fdb, 0xef939e, 0xff256e, 0xff657b, 0xff4658, 0xfa6533, 0xfc9775, 0xdba685, 0x9f8778,
        0xffb938, 0xffe500, 0xb4fb44, 0x25ca63, 0x6fe582, 0x06cf9c, 0x42c6b8, 0x29b0e1, 0x5583fc,
    ];
}

/// A color that depends on the theme.
pub type Token = fn(&Palette) -> Color;

/// The palette of the theme, with the theme color `ui` at the time `now_ms` (which moves a rich gradient).
pub fn palette(dark: bool, ui: Option<beeper_core::models::UiTheme>, now_ms: i64) -> Palette {
    let mut p = if dark { BEEPER_DARK } else { BEEPER_LIGHT };
    if let Some(t) = ui {
        crate::ui_theme::apply(&mut p, t, dark, now_ms);
    }
    p
}

/// A slider with no visible rail or handle, over a custom track.
pub fn hidden_slider(_: &Theme, _: slider::Status) -> slider::Style {
    slider::Style {
        rail: slider::Rail { backgrounds: (CLEAR.into(), CLEAR.into()), width: 0.0, border: Default::default() },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 0.0 },
            background: CLEAR.into(),
            border_width: 0.0,
            border_color: CLEAR,
        },
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at `x` in 0..1: the curve's `y` where its `x` is `x`.
pub fn cubic_bezier(x: f32, [x1, y1, x2, y2]: [f32; 4]) -> f32 {
    let bezier = |s: f32, a: f32, b: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines that the widgets lay out, which [`surely_fits`] may skip only when they are one.
    fn shaped_lines(s: &str, max: f32, size: f32) -> usize {
        shape(s, size, Weight::Normal, Some(max), |b| b.layout_runs().count())
    }

    /// A text that `surely_fits` lets skip the shaping is one line at that width, for each ASCII glyph, even the
    /// widest, repeated up to the bound, and the shaped measurements agree on it.
    #[test]
    fn a_text_that_surely_fits_takes_one_line() {
        for size in [11.0, 13.0, 16.0, 24.0] {
            for c in (b' '..=b'~').map(char::from) {
                for max in [40.0, 200.0, 480.0] {
                    let s: String = std::iter::repeat_n(c, (max / (1.2 * size)) as usize).collect();
                    assert!(surely_fits(&s, max, size), "{s:?} at {max}");
                    assert_eq!(shaped_lines(&s, max, size).max(1), 1, "{s:?} at {max} and {size} px");
                    let w = format!("{s}\u{2060}");
                    assert!(!wraps(&w, max, size) && line_count(&w, max, size) == 1);
                }
            }
        }
        assert!(!surely_fits("caf\u{e9}", 400.0, 13.0), "only ASCII");
        assert!(!surely_fits("a\nb", 400.0, 13.0), "one line");
        let long = "word ".repeat(40);
        assert!(!surely_fits(&long, 400.0, 13.0));
        assert_eq!(wraps(&long, 400.0, 13.0), shaped_lines(&long, 400.0, 13.0) > 1);
        assert_eq!(line_count(&long, 400.0, 13.0), shaped_lines(&long, 400.0, 13.0));
        assert!(!wraps("two\nlines", 400.0, 13.0), "a line break is no wrap");
    }

    /// CSS `grayscale(100%)` of a logo: each color becomes its luminance, and the tile keeps its shapes.
    #[test]
    fn blocked_logos_are_gray_like_desktop() {
        let svg = r##"<rect fill="#48C95F"/><path fill="#fff" stroke="#000"/>"##;
        assert_eq!(grayscale(svg), r##"<rect fill="#a6a6a6"/><path fill="#ffffff" stroke="#000000"/>"##);
    }

    /// A balanced bubble keeps the room of its time after the last line: measured with digits joined to the text, as
    /// trailing spaces hang past the width.
    #[test]
    fn balanced_bubbles_keep_the_room_of_the_time() {
        let text = "Nothing bad to say about them, they put money in at our requested valuation";
        let room = text_width("12:30 AM", 11.0, Weight::Normal) + 4.0;
        let zero = text_width("0", 13.0, Weight::Normal);
        let probe = format!("{text}\u{2060}{}", "0".repeat((room / zero).ceil() as usize));
        let width = balanced_width(&probe, 400.0, 13.0);
        let last = shape(text, 13.0, Weight::Normal, Some(width), |b| b.layout_runs().last().map(|r| r.line_w));
        assert!(last.expect("a line") + room <= width + 0.5, "the time fits after the last line");
        let lines = shape(text, 13.0, Weight::Normal, Some(width), |b| b.layout_runs().count());
        assert_eq!(lines, 2, "and the lines stay balanced");
    }

    #[test]
    fn text_memos_keep_the_measurements_in_use_and_stay_bounded() {
        thread_local!(static M: Memo<usize, usize> = RefCell::default());
        for i in 1..3 * MEMO {
            assert_eq!(memo(&M, i, || i), i);
            assert_eq!(memo(&M, 0, || i), 1, "a measurement in use stays");
        }
        M.with(|m| assert!(m.borrow().iter().map(HashMap::len).sum::<usize>() <= 2 * MEMO, "the memo is bounded"));
    }

    /// A 200x200 pixmap of a card with the background `bg` and a shadow, drawn in turn with each damage rectangle.
    fn shadowed_card(bg: Color, damages: &[iced::Rectangle]) -> tiny_skia::Pixmap {
        use iced::{Point, Rectangle, Size};
        let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
        let (style, card) = (shadow_box(bg, CLEAR, 12.0, 24.0)(&Theme::Light), Size::new(120.0, 120.0));
        container::draw_background(&mut renderer, &style, Rectangle::new(Point::new(40.0, 40.0), card));
        let mut pixmap = tiny_skia::Pixmap::new(200, 200).unwrap();
        let mut mask = tiny_skia::Mask::new(200, 200).unwrap();
        let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(200, 200), 1.0);
        for &damage in damages {
            renderer.draw(&mut pixmap.as_mut(), &mut mask, &viewport, &[damage], Color::WHITE);
        }
        pixmap
    }

    /// A shadowed card that shrinks, like the mention list when the typed name leaves one person, damages its old
    /// shadow too. The software renderer left that shadow, and the old card under it, as a white card above the list.
    #[test]
    fn a_shrinking_card_damages_its_old_shadow() {
        use iced::{Point, Rectangle, Size};
        let layer = |height: f32| {
            let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
            let style = shadow_box(Color::WHITE, CLEAR, 6.0, 16.0)(&Theme::Light);
            let card = Rectangle::new(Point::new(40.0, 100.0 - height), Size::new(120.0, height));
            container::draw_background(&mut renderer, &style, card);
            renderer.layers()[0].clone()
        };
        let damage = iced_tiny_skia::Layer::damage(&layer(100.0), &layer(30.0));
        // Left of the old card, in its shadow only.
        let shadow = Point::new(30.0, 30.0);
        assert!(damage.iter().any(|d| d.contains(shadow)), "{damage:?}");
    }

    #[test]
    fn caret_redraws_keep_the_shadowed_card_outside_the_damage() {
        use iced::{Point, Rectangle, Size};
        let full = Rectangle::with_size(Size::new(200.0, 200.0));
        let caret = Rectangle::new(Point::new(60.0, 60.0), Size::new(2.0, 16.0));
        let redrawn = shadowed_card(Color::WHITE, &[[full].as_slice(), &[caret; 20]].concat());
        assert!(redrawn == shadowed_card(Color::WHITE, &[full]), "a redraw of the caret changes no pixel of the card");
    }

    #[test]
    fn shadows_compute_only_the_visible_pixels_in_the_damage() {
        use iced::{Point, Rectangle, Size};
        let screen = Rectangle::with_size(Size::new(200.0, 200.0));
        let full = shadowed_card(Color::WHITE, &[screen]);
        // The anti-aliased corners of the card can be 1 level away, with a translucent fill or a clipped fill.
        let near = |p: tiny_skia::Pixmap| full.data().iter().zip(p.data()).all(|(a, b)| a.abs_diff(*b) <= 1);
        // A card that is not opaque gets the shadow under its fill too.
        let translucent = shadowed_card(Color { a: 0.9999, ..Color::WHITE }, &[screen]);
        assert!(near(translucent), "the fill of an opaque card covers the shadow that is not computed");
        let tiles = [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)]
            .map(|(x, y)| Rectangle::new(Point::new(x, y), Size::new(100.0, 100.0)));
        assert!(near(shadowed_card(Color::WHITE, &tiles)), "the shadow in tiles is the shadow of one full draw");
    }

    #[test]
    fn a_card_in_tiles_fills_the_same_pixels_as_one_full_draw() {
        use iced::{Point, Rectangle, Size};
        let blue = Color::from_rgb(0.2, 0.4, 0.8);
        let full = shadowed_card(blue, &[Rectangle::with_size(Size::new(200.0, 200.0))]);
        // Tiles with fractional edges. The middle tile misses the rounded corners, so it fills only a rectangle.
        let side = 200.0 / 3.0;
        let tiles: Vec<Rectangle> =
            (0..9).map(|i| Rectangle::new(Point::new((i % 3) as f32 * side, (i / 3) as f32 * side), Size::new(side, side))).collect();
        let tiled = shadowed_card(blue, &tiles);
        // The masked corners of the card can be 1 level away, like in the shadow test.
        assert!(full.data().iter().zip(tiled.data()).all(|(a, b)| a.abs_diff(*b) <= 1), "the tiles draw the card");
        let middle = |p: &tiny_skia::Pixmap| -> Vec<u8> {
            (67..133).flat_map(|y| p.data()[(y * 200 + 67) * 4..(y * 200 + 133) * 4].to_vec()).collect()
        };
        assert_eq!(middle(&tiled), middle(&full), "the middle tile fills the same pixels");
    }

    /// The software renderer puts a scaled up image at its bounds, like a zoomed image of the media viewer: it moved
    /// the image up to a scale to the left and up, past its damage, which left lines when the zoom went out.
    #[test]
    fn scaled_up_images_draw_at_their_bounds_in_the_software_renderer() {
        use iced::advanced::image::{Handle, Image, Renderer as _};
        use iced::{Point, Rectangle, Size};
        let red = Handle::from_rgba(2, 2, [255, 0, 0, 255].repeat(4));
        let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
        let bounds = Rectangle::new(Point::new(10.9, 10.9), Size::new(60.0, 60.0));
        renderer.draw_image(Image::new(red), bounds, bounds);
        let (mut pixmap, mut mask) = (tiny_skia::Pixmap::new(80, 80).unwrap(), tiny_skia::Mask::new(80, 80).unwrap());
        let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(80, 80), 1.0);
        renderer.draw(&mut pixmap.as_mut(), &mut mask, &viewport, &[Rectangle::with_size(Size::new(80.0, 80.0))], Color::WHITE);
        let red = |x, y| pixmap.pixel(x, y).expect("a pixel in the pixmap").green() < 128;
        assert!(!red(9, 40) && !red(40, 9), "nothing left of or above the image");
        assert!(red(12, 40) && red(40, 12) && red(69, 69), "the image fills its bounds");
        assert!(!red(72, 40), "nothing right of the image");
    }

    /// The software renderer rounds the corners of an image with a border radius, like the image shader of wgpu: the
    /// inline images and videos of the timeline have the 17 px corners of a bubble.
    #[test]
    fn rounded_images_draw_round_in_the_software_renderer() {
        use iced::advanced::image::{Handle, Image, Renderer as _};
        use iced::{Point, Rectangle, Size};
        let red = Handle::from_rgba(2, 2, [255, 0, 0, 255].repeat(4));
        let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
        let bounds = Rectangle::new(Point::new(10.0, 10.0), Size::new(60.0, 40.0));
        renderer.draw_image(Image { border_radius: 17.0.into(), ..Image::new(red) }, bounds, bounds);
        let (mut pixmap, mut mask) = (tiny_skia::Pixmap::new(80, 80).unwrap(), tiny_skia::Mask::new(80, 80).unwrap());
        let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(80, 80), 1.0);
        let screen = [Rectangle::with_size(Size::new(80.0, 80.0))];
        renderer.draw(&mut pixmap.as_mut(), &mut mask, &viewport, &screen, Color::WHITE);
        let red = |x, y| pixmap.pixel(x, y).expect("a pixel in the pixmap").green() < 128;
        assert!(!red(11, 11) && !red(68, 11) && !red(11, 48) && !red(68, 48), "the corners are round");
        assert!(red(40, 11) && red(11, 30) && red(40, 30) && red(68, 30), "the image fills its sides");
        assert!(!red(72, 30) && !red(40, 52), "nothing past the image");
    }

    /// The software renderer draws a turned icon, like the Next Attachment arrow of the carousel and the open
    /// chevrons, which it left out.
    #[test]
    fn turned_icons_draw_in_the_software_renderer() {
        use iced::advanced::svg::{Renderer as _, Svg};
        use iced::{Point, Rectangle, Size};
        let left = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect width="10" height="20"/></svg>"#;
        let left = svg::Handle::from_memory(left.as_slice());
        let dark = |turn: f32| {
            let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
            let bounds = Rectangle::new(Point::new(10.0, 10.0), Size::new(20.0, 20.0));
            renderer.draw_svg(Svg::new(left.clone()).color(Color::BLACK).rotation(turn), bounds, bounds);
            let (mut pixmap, mut mask) = (tiny_skia::Pixmap::new(40, 40).unwrap(), tiny_skia::Mask::new(40, 40).unwrap());
            let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(40, 40), 1.0);
            let screen = Rectangle::with_size(Size::new(40.0, 40.0));
            renderer.draw(&mut pixmap.as_mut(), &mut mask, &viewport, &[screen], Color::WHITE);
            move |x, y| pixmap.pixel(x, y).expect("a pixel in the pixmap").red() < 128
        };
        let still = dark(0.0);
        assert!(still(14, 20) && !still(26, 20), "the icon fills its left half");
        let half = dark(std::f32::consts::PI);
        assert!(!half(14, 20) && half(26, 20), "a half turn fills the right half");
        let quarter = dark(std::f32::consts::FRAC_PI_2);
        assert!(quarter(20, 14) && !quarter(20, 26), "a quarter turn clockwise fills the top half");
    }

    /// A vector drawn in a translucent color keeps the color's alpha, like the faint Beeper logo at the start of a
    /// chat, which the renderer drew opaque.
    #[test]
    fn translucent_icons_stay_translucent_in_the_software_renderer() {
        use iced::advanced::svg::{Renderer as _, Svg};
        use iced::{Point, Rectangle, Size};
        let square = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect width="20" height="20"/></svg>"#;
        let square = svg::Handle::from_memory(square.as_slice());
        let red_at = |alpha: f32| {
            let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
            let bounds = Rectangle::new(Point::new(10.0, 10.0), Size::new(20.0, 20.0));
            renderer.draw_svg(Svg::new(square.clone()).color(Color::BLACK.scale_alpha(alpha)), bounds, bounds);
            let (mut pixmap, mut mask) = (tiny_skia::Pixmap::new(40, 40).unwrap(), tiny_skia::Mask::new(40, 40).unwrap());
            let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(40, 40), 1.0);
            let screen = Rectangle::with_size(Size::new(40.0, 40.0));
            renderer.draw(&mut pixmap.as_mut(), &mut mask, &viewport, &[screen], Color::WHITE);
            pixmap.pixel(20, 20).expect("a pixel in the pixmap").red()
        };
        assert!(red_at(1.0) < 5, "an opaque icon is black");
        assert!((220..=235).contains(&red_at(0.1)), "10% black over white is light grey, not {}", red_at(0.1));
    }

    #[test]
    fn palette_matches_desktop_tokens() {
        let rows: [(&str, Token, Color, Color); 10] = [
            ("--app-focus-ring", |p| p.focus_ring, hex(0x0d4ffb), hex(0x0d4ffb)),
            ("--left-pane-bg", |p| p.window, hex(0xeaebf0), hex(0x1c1d1e)),
            ("--color-primary", |p| p.accent, hex(0x0d4ffb), hex(0x2561fb)),
            ("--color-secondary-container", |p| p.received, hex(0xefeff0), hex(0x232326)),
            ("--color-text-neutrals", |p| p.text, hex(0x171717), hex(0xededed)),
            ("--color-on-surface", |p| p.strong, hex(0x202124), hex(0xf6f6f6)),
            ("--color-text-secondary", |p| p.sub, hexa(0x3c3c44, 0.69), hexa(0xebebf5, 0.63)),
            ("--color-on-secondary-container", |p| p.on_secondary, hex(0x828286), hex(0x9b9ba2)),
            ("--color-background-menu-opaque", |p| p.menu, hex(0xededed), hex(0x232323)),
            ("--color-background-menu-option-hover", |p| p.menu_hover, hex(0x007aff), hex(0x005bb8)),
        ];
        for (token, field, light, dark) in rows {
            assert_eq!((field(&BEEPER_LIGHT), field(&BEEPER_DARK)), (light, dark), "{token}");
        }
    }

    #[test]
    fn a_removed_row_damages_only_the_bounds_of_the_row() {
        use iced::advanced::renderer::{Quad, Renderer as _};
        use iced::{Point, Rectangle, Size};
        let row = |i: usize| Rectangle::new(Point::new(0.0, i as f32 * 20.0), Size::new(100.0, 20.0));
        let rows = |from: usize| {
            let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, 16.into());
            (from..10).for_each(|i| renderer.fill_quad(Quad { bounds: row(i), ..Quad::default() }, Color::BLACK));
            renderer
        };
        let (mut all, mut rest) = (rows(0), rows(1));
        let damage = iced_tiny_skia::Layer::damage(&all.layers()[0], &rest.layers()[0]);
        assert_eq!(damage, [row(0).expand(1.0)], "the rows after the removed row keep their pixels");
    }

    #[test]
    fn inter_draws_every_desktop_face_and_script() {
        ensure_fonts_loaded();
        let mut fs = font_system().write().unwrap();
        for weight in [Weight::Normal, Weight::Medium, Weight::Semibold, Weight::Bold] {
            for style in [iced::font::Style::Normal, iced::font::Style::Italic] {
                let mut buf = cosmic_text::Buffer::new(fs.raw(), cosmic_text::Metrics::new(13.0, 16.0));
                let font = Font { style, ..fw(weight) };
                let text = "Aa Жж Ωω Ếế ₂ → ≠";
                buf.set_text(fs.raw(), text, &to_attributes(font), cosmic_text::Shaping::Advanced, None);
                let runs: Vec<_> = buf.layout_runs().flat_map(|r| r.glyphs.iter().map(|g| g.font_id)).collect();
                assert_eq!(runs.len(), text.chars().count());
                for id in runs {
                    let face = fs.raw().db().face(id).unwrap();
                    assert_eq!(face.families[0].0, "Inter");
                    assert_eq!((face.weight.0, face.style), (to_attributes(font).weight.0, to_attributes(font).style));
                }
            }
        }
    }
}

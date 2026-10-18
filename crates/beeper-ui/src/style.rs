use beeper_core::config::NETWORK_SPECS;
use iced::advanced::graphics::text::cosmic_text::{self, Fallback as _};
use iced::advanced::graphics::text::{font_system, to_attributes};
use iced::font::{Family, Font, Weight};
use iced::overlay::menu;
use iced::widget::{button, container, pick_list, scrollable, slider, svg, text, text_input};
use iced::{border, Border, Color, Shadow, Theme, Vector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{LazyLock, Once};

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
    pub const BELL: char = '\u{e059}';
    pub const CLOUD: char = '\u{e088}';
    pub const CIRCLE_PLUS: char = '\u{e081}';
    pub const CIRCLE_CHECK: char = '\u{e226}';
    pub const LINK: char = '\u{e102}';
    pub const ARROW_UP_RIGHT: char = '\u{e04d}';
    pub const PLUS: char = '\u{e13d}';
    pub const SMILE: char = '\u{e164}';
    pub const AUDIO_LINES: char = '\u{e55a}';
    pub const ARROW_UP: char = '\u{e04a}';
    pub const AT_SIGN: char = '\u{e04e}';
    pub const SETTINGS: char = '\u{e154}';
    pub const PAINTBRUSH: char = '\u{e2e7}';
    pub const KEYBOARD: char = '\u{e284}';
    pub const SMARTPHONE: char = '\u{e163}';
    pub const SPARKLES: char = '\u{e412}';
    pub const PUZZLE: char = '\u{e29c}';
    pub const DATABASE: char = '\u{e0ad}';
    pub const CIRCLE_HELP: char = '\u{e082}';
    pub const INFO: char = '\u{e0f9}';
    pub const PENCIL: char = '\u{e1f9}';
    pub const TRASH_2: char = '\u{e18e}';
    pub const REPLY: char = '\u{e22a}';
    pub const PLAY: char = '\u{e13c}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const BOOKMARK: char = '\u{e060}';
    pub const MESSAGE_SQUARE: char = '\u{e117}';
    pub const TABLET: char = '\u{e17e}';
    pub const MONITOR: char = '\u{e11d}';
}

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
pub const FONT_ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic.ttf");
pub const FONT_ICONS: &[u8] = include_bytes!("../assets/fonts/icons.ttf");
pub const FONT_EMOJI: &[u8] = include_bytes!("../assets/fonts/Twemoji.Mozilla.ttf");
pub const ALL_FONTS: [&[u8]; 7] =
    [FONT_REGULAR, FONT_MEDIUM, FONT_SEMIBOLD, FONT_BOLD, FONT_ITALIC, FONT_ICONS, FONT_EMOJI];
pub const CLEAR: Color = Color::TRANSPARENT;

static LOAD_FONTS: Once = Once::new();
static PLATFORM: cosmic_text::PlatformFallback = cosmic_text::PlatformFallback;
static COMMON_FALLBACK: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    std::iter::once(crate::emoji::EMOJI_FAMILY).chain(PLATFORM.common_fallback().iter().copied()).collect()
});

/// Desktop's font stack reaches Twemoji before the generic sans-serif fonts.
struct TwemojiFirst;

impl cosmic_text::Fallback for TwemojiFirst {
    fn common_fallback(&self) -> &[&'static str] {
        &COMMON_FALLBACK
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        PLATFORM.forbidden_fallback()
    }
    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        PLATFORM.script_fallback(script, locale)
    }
}

pub fn ensure_fonts_loaded() {
    LOAD_FONTS.call_once(|| {
        let mut fs = font_system().write().expect("font system");
        let empty = cosmic_text::FontSystem::new_with_locale_and_db(String::new(), Default::default());
        let (locale, db) = std::mem::replace(fs.raw(), empty).into_locale_and_db();
        *fs.raw() = cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(locale, db, TwemojiFirst);
        for bytes in ALL_FONTS {
            fs.load_font(bytes.into());
        }
    });
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

fn glyphs(s: &str, size: f32, w: Weight) -> (f32, Vec<(usize, f32)>) {
    shape(s, size, w, None, |buf| {
        let mut out = Vec::new();
        let mut total = 0.0;
        for g in buf.layout_runs().flat_map(|r| r.glyphs.iter()) {
            total += g.w;
            out.push((g.end, total));
        }
        (total, out)
    })
}

/// The x position and wrapped row of each (line, byte) caret in `s` wrapped at `width`.
pub fn carets(s: &str, size: f32, width: f32, at: impl IntoIterator<Item = (usize, usize)>) -> Vec<(f32, usize)> {
    shape(s, size, Weight::Normal, Some(width), |buf| {
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

/// CSS `text-wrap: balance`: the narrowest width that wraps `s` into as many lines as `max` does, if that is more than one.
pub fn balanced_width(s: &str, max: f32, size: f32) -> Option<f32> {
    thread_local!(static CACHE: RefCell<HashMap<(String, u32), Option<f32>>> = RefCell::default());
    let key = (s.to_owned(), max.to_bits());
    if let Some(w) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return w;
    }
    let lines = |w: f32| shape(s, size, Weight::Normal, Some(w), |b| b.layout_runs().count());
    let n = lines(max);
    let (mut lo, mut hi) = (max / n as f32, max);
    while hi - lo > 0.5 {
        let mid = (lo + hi) / 2.0;
        if lines(mid) > n {
            lo = mid
        } else {
            hi = mid
        }
    }
    let w = (n > 1).then_some(hi);
    CACHE.with(|c| c.borrow_mut().insert(key, w));
    w
}

pub fn text_width(s: &str, size: f32, w: Weight) -> f32 {
    glyphs(s, size, w).0
}

fn ell_line(s: &str, max: f32, size: f32, w: Weight) -> String {
    let (total, gs) = glyphs(s, size, w);
    if total <= max {
        return s.into();
    }
    let cut = max - text_width("…", size, w);
    let end = gs.get(gs.partition_point(|g| g.1 <= cut).wrapping_sub(1)).map_or(0, |g| g.0);
    format!("{}…", s[..end].trim_end())
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
    type Key = (String, u32, u32, Weight, usize);
    thread_local!(static CACHE: RefCell<HashMap<Key, Rc<str>>> = RefCell::default());
    assert!(lines > 0, "clamp to zero lines");
    let key = (s.to_owned(), max.max(0.0).to_bits(), size.to_bits(), w, lines);
    CACHE.with(|c| c.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let starts: Vec<usize> = shape(&s, size, w, Some(max.max(0.0)), |buf| {
            buf.layout_runs().map(|r| r.glyphs.first().map_or(0, |g| g.start)).collect()
        });
        let res: Rc<str> = match starts.get(lines - 1) {
            Some(&k) if starts.len() > lines => format!("{}{}", &s[..k], ell_line(&s[k..], max, size, w)).into(),
            _ => s.into(),
        };
        CACHE.with(|c| {
            let mut m = c.borrow_mut();
            if m.len() > 4096 {
                m.clear();
            }
            m.insert(key, res.clone());
        });
        res
    })
}

pub fn t<'a>(s: impl text::IntoFragment<'a>, size: f32, w: Weight) -> text::Text<'a> {
    text(s).size(size).font(fw(w)).line_height(text::LineHeight::Absolute((size * 1.24).round().into()))
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

macro_rules! brands {
    ($($id:literal),*) => {
        [$(($id, include_str!(concat!("../assets/brands/", $id, ".svg")), include_str!(concat!("../assets/brands/badge/", $id, ".svg")))),*]
    };
}

const BRANDS: [(&str, &str, &str); 20] = brands!(
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
    "wechat",
    "imessage",
    "bluesky",
    "hungryserv"
);

static BRAND_SVGS: LazyLock<HashMap<&'static str, [svg::Handle; 2]>> = LazyLock::new(|| {
    assert!(NETWORK_SPECS.iter().all(|n| BRANDS.iter().any(|b| b.0 == n.bridge_id)), "every network has a logo");
    BRANDS
        .iter()
        .map(|(id, tile, badge)| (*id, [tile, badge].map(|d| svg::Handle::from_memory(d.as_bytes()))))
        .collect()
});

pub fn brand_svg(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][0].clone()
}

pub fn brand_badge(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id][1].clone()
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

pub fn glow_box(bg: Color, glow: Color, r: f32, blur: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { radius: r.into(), ..Border::default() },
        shadow: Shadow { color: glow, offset: Vector::new(0.0, 4.0), blur_radius: blur },
        text_color: None,
        snap: false,
    }
}

pub fn shadow_box(bg: Color, border_col: Color, r: f32, blur: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width: 1.0, radius: r.into() },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
            offset: Vector::new(0.0, 4.0),
            blur_radius: blur,
        },
        text_color: None,
        snap: false,
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
    bgs: [Color; 3],
    fgs: [Color; 3],
    border_col: Color,
    width: f32,
    r: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    move |t, s| button::Style {
        border: Border { color: border_col, width, radius: r.into() },
        ..btn_style(bgs, fgs, r)(t, s)
    }
}

pub fn input_style(
    bg: Color,
    text_col: Color,
    placeholder_col: Color,
    accent_col: Color,
    r: f32,
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
    bg: Color,
    hover_bg: Color,
    text_col: Color,
    sub_col: Color,
) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style + Copy {
    move |_, status| pick_list::Style {
        text_color: text_col,
        placeholder_color: sub_col,
        handle_color: sub_col,
        background: (if matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) {
            hover_bg
        } else {
            bg
        })
        .into(),
        border: Border { color: CLEAR, width: 0.0, radius: 12.0.into() },
    }
}

pub fn pick_menu_style(
    bg: Color,
    hover_bg: Color,
    text_col: Color,
    border_col: Color,
) -> impl Fn(&Theme) -> menu::Style + Copy {
    move |_| menu::Style {
        background: bg.into(),
        border: Border { color: border_col, width: 1.0, radius: 8.0.into() },
        text_color: text_col,
        selected_text_color: text_col,
        selected_background: hover_bg.into(),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
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
            scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. }
            | scrollable::Status::Dragged { .. } => 0.5,
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
        auto_scroll: scrollable::AutoScroll {
            background: CLEAR.into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: CLEAR,
        },
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub frame: Color,
    pub window: Color,
    pub rail: Color,
    pub rail_sel: Color,
    pub card: Color,
    pub selected: Color,
    pub hover: Color,
    pub text: Color,
    pub fg: Color,
    pub strong: Color,
    pub sub: Color,
    pub weak: Color,
    pub muted: Color,
    pub tertiary: Color,
    pub divider: Color,
    pub elevated: Color,
    pub input: Color,
    pub send_btn: Color,
    pub sent: Color,
    pub sent_meta: Color,
    pub received: Color,
    pub link_card: Color,
    pub outline: Color,
    pub date_text: Color,
    pub accent: Color,
    pub toggle_on: Color,
    pub toggle_off: Color,
    pub prefs_pane: Color,
    pub gutter: [Color; 2],
    pub scroll: Color,
    pub prefs_group: Color,
    pub prefs_border: Color,
    pub neutral_border: Color,
    pub nav_sel: Color,
    pub search: Color,
    pub chip: Color,
    pub link: Color,
    pub button: Color,
    pub button_border: Color,
    pub range: Color,
    pub range_track: Color,
    pub range_edge: Color,
    pub menu: Color,
    pub menu_border: Color,
    pub on_secondary: Color,
    pub surface_dim: Color,
    pub reply_line: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    /// `AudioBar`'s ring while recording, its ring while paused, its text, and its live waveform.
    pub audio_bar: [Color; 4],
    pub names: [u32; 20],
}

pub const fn hex(v: u32) -> Color {
    Color::from_rgb(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0)
}

const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

pub const BEEPER_DARK: Palette = Palette {
    frame: hex(0x212121),
    window: hex(0x232323),
    rail: hex(0x383838),
    rail_sel: hex(0x505050),
    card: hex(0x000000),
    selected: hex(0x393939),
    hover: hexa(0xf6f6f6, 0.05),
    text: hex(0xededed),
    fg: hex(0xffffff),
    strong: hex(0xf6f6f6),
    sub: hexa(0xebebf5, 0.62),
    weak: hex(0xadadad),
    muted: hex(0x808080),
    tertiary: hexa(0xebebf5, 0.27),
    divider: hexa(0xf6f6f6, 0.2),
    elevated: hex(0x232323),
    input: hex(0x1c1c1d),
    send_btn: hex(0x4d4d4d),
    sent: hex(0x2561fb),
    sent_meta: hexa(0xffffff, 0.6),
    received: hex(0x232326),
    link_card: hex(0x282828),
    outline: hex(0x28282a),
    date_text: hex(0x9a9aa1),
    accent: hex(0x2561fb),
    toggle_on: hex(0x0d4ffb),
    toggle_off: hex(0x606060),
    prefs_pane: hex(0x161616),
    gutter: [hex(0x2c2c2c), hex(0x9f9f9f)],
    scroll: Color::WHITE,
    prefs_group: hex(0x1c1c1c),
    prefs_border: hex(0x282828),
    neutral_border: hex(0x343434),
    nav_sel: hex(0x3c3c3c),
    search: hex(0x323232),
    chip: hex(0x232326),
    link: hex(0x5cadff),
    button: hex(0x505050),
    button_border: hex(0x5e5e5e),
    range: hex(0x99c8ff),
    range_track: hex(0x3b3b3b),
    range_edge: hex(0x858585),
    menu: hex(0x232323),
    menu_border: hex(0x434343),
    on_secondary: hex(0x9b9ba2),
    surface_dim: hex(0x000000),
    reply_line: hex(0x353535),
    danger: hex(0xff383c),
    success: hex(0x34c759),
    warning: hex(0xf54c03),
    audio_bar: [hex(0x606366), hex(0x303236), hex(0xd8d8d8), hex(0x9b9ba2)],
    names: [
        0x9e7ef8, 0xce7fff, 0xf86fdb, 0xef939e, 0xff256e, 0xff657b, 0xff4658, 0xfa6533, 0xfc9775, 0xdba685, 0x9f8778,
        0xffb938, 0xffe500, 0xb4fb44, 0x25ca63, 0x6fe582, 0x06cf9c, 0x42c6b8, 0x29b0e1, 0x5583fc,
    ],
};

pub const BEEPER_LIGHT: Palette = Palette {
    frame: hex(0xf0f0f0),
    window: hex(0xf0f0f0),
    rail: hex(0xe1e1e1),
    rail_sel: hex(0xcfcfcf),
    card: hex(0xffffff),
    selected: hex(0xe0e0e0),
    hover: hexa(0x000000, 0.04),
    text: hex(0x1c1c1e),
    fg: hex(0x000000),
    strong: hex(0x000000),
    sub: hexa(0x3c3c43, 0.62),
    weak: hex(0x5e5e63),
    muted: hex(0x8e8e93),
    tertiary: hexa(0x3c3c43, 0.3),
    divider: hexa(0x000000, 0.12),
    elevated: hex(0xffffff),
    input: hex(0xf4f4f5),
    send_btn: hex(0xd5d5d8),
    sent: hex(0x0d4ffb),
    sent_meta: hexa(0xffffff, 0.7),
    received: hex(0xe9e9eb),
    link_card: hex(0xededed),
    outline: hex(0xe2e2e4),
    date_text: hex(0x6c6c72),
    accent: hex(0x0d4ffb),
    toggle_on: hex(0x0d4ffb),
    toggle_off: hexa(0x000000, 0.25),
    prefs_pane: hex(0xf3f3f3),
    gutter: [hex(0xf1f1f1), hex(0xc1c1c1)],
    scroll: Color::BLACK,
    prefs_group: hex(0xffffff),
    prefs_border: hex(0xededed),
    neutral_border: hex(0xe2e2e2),
    nav_sel: hex(0xdedede),
    search: hex(0xe6e6e6),
    chip: hex(0xefeff0),
    link: hex(0x0069db),
    button: hex(0xffffff),
    button_border: hex(0xe2e2e2),
    range: hex(0x0075ff),
    range_track: hex(0xefefef),
    range_edge: hex(0x767676),
    menu: hex(0xf3f3f3),
    menu_border: hex(0xe2e2e2),
    on_secondary: hex(0x6c6c72),
    surface_dim: hex(0xebebeb),
    reply_line: hex(0xe1e1e1),
    danger: hex(0xff383c),
    success: hex(0x34c759),
    warning: hex(0xf54c03),
    audio_bar: [hex(0xe3e9f2), hex(0xf1f4f8), hex(0x7f7f7f), hex(0x828286)],
    names: [
        0x6a4be5, 0x9745ea, 0xd833ca, 0xdf6f7c, 0xe8126b, 0xcd1557, 0xee1f34, 0xde4b19, 0xc25e3c, 0xbb7c55, 0x8d6954,
        0xc2891f, 0xd99c00, 0x6bad16, 0x0f9240, 0x45a654, 0x07a06d, 0x239084, 0x0a81ac, 0x1e54e1,
    ],
};

pub fn palette(dark: bool) -> Palette {
    if dark {
        BEEPER_DARK
    } else {
        BEEPER_LIGHT
    }
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

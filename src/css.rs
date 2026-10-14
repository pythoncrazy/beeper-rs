use crate::config::{
    CSS_DARK_PURPLE, CSS_DRACULA, CSS_FIX_LOCKDOWN_EMOJIS, CSS_NOCTURNE, CSS_TEXTS, CSS_VARIABLES,
};
use crate::models::{BeeperSettings, ThemeChoice};
use iced::advanced::graphics::text::{cosmic_text, font_system, to_attributes};
use iced::font::{Family, Font, Weight};
use iced::overlay::menu;
use iced::widget::{button, container, pick_list, scrollable, svg, text, text_input};
use iced::{border, Border, Color, Shadow, Theme, Vector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{LazyLock, Once};

pub mod icon {
    pub const INBOX: char = '\u{e0f7}';
    pub const LIST_RESTART: char = '\u{e452}';
    pub const ARCHIVE: char = '\u{e041}';
    pub const CLOCK: char = '\u{e087}';
    pub const SEND_HORIZONTAL: char = '\u{e4f2}';
    pub const MAIL: char = '\u{e10f}';
    pub const MESSAGE_CIRCLE_PLUS: char = '\u{e567}';
    pub const IMAGES: char = '\u{e5c4}';
    pub const COPY_PLUS: char = '\u{e3fd}';
    pub const SQUARE_PLUS: char = '\u{e173}';
    pub const ELLIPSIS: char = '\u{e0b6}';
    pub const CHEVRONS_UP_DOWN: char = '\u{e211}';
    pub const LIST_FILTER: char = '\u{e460}';
    pub const SEARCH: char = '\u{e151}';
    pub const SQUARE_PEN: char = '\u{e172}';
    pub const BELL_OFF: char = '\u{e05a}';
    pub const BELL: char = '\u{e059}';
    pub const CIRCLE_CHECK: char = '\u{e226}';
    pub const CIRCLE_DASHED: char = '\u{e4b0}';
    pub const CHECK_CHECK: char = '\u{e38e}';
    pub const CHECK: char = '\u{e06c}';
    pub const IMAGE: char = '\u{e0f6}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const CHEVRON_LEFT: char = '\u{e06e}';
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
    pub const X: char = '\u{e1b2}';
    pub const MINUS: char = '\u{e11c}';
    pub const PIN: char = '\u{e259}';
    pub const STAR: char = '\u{e176}';
    pub const PENCIL: char = '\u{e1f9}';
    pub const TRASH_2: char = '\u{e18e}';
    pub const REPLY: char = '\u{e22a}';
    pub const FORWARD: char = '\u{e229}';
    pub const FILE_TEXT: char = '\u{e0cc}';
    pub const FOLDER: char = '\u{e0d7}';
    pub const PLAY: char = '\u{e13c}';
    pub const PAUSE: char = '\u{e12e}';
    pub const MIC: char = '\u{e118}';
    pub const CHART_BAR_BIG: char = '\u{e4a7}';
    pub const CHART_NO_AXES_COLUMN: char = '\u{e068}';
    pub const REFRESH_CW: char = '\u{e145}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const USER: char = '\u{e19f}';
    pub const USERS: char = '\u{e1a4}';
    pub const LOG_OUT: char = '\u{e10e}';
    pub const SHIELD: char = '\u{e158}';
    pub const SUN: char = '\u{e178}';
    pub const MOON: char = '\u{e11e}';
    pub const PALETTE: char = '\u{e1dd}';
    pub const VOLUME_2: char = '\u{e1ab}';
    pub const VOLUME_X: char = '\u{e1ac}';
    pub const EYE: char = '\u{e0ba}';
    pub const EYE_OFF: char = '\u{e0bb}';
    pub const BOOKMARK: char = '\u{e060}';
    pub const LOCK: char = '\u{e10b}';
    pub const KEY_ROUND: char = '\u{e4a3}';
    pub const COPY: char = '\u{e09e}';
    pub const DOWNLOAD: char = '\u{e0b2}';
    pub const UPLOAD: char = '\u{e19e}';
    pub const PAPERCLIP: char = '\u{e12d}';
    pub const STICKER: char = '\u{e302}';
    pub const MESSAGE_CIRCLE: char = '\u{e116}';
    pub const MESSAGE_SQUARE: char = '\u{e117}';
    pub const PHONE: char = '\u{e133}';
    pub const VIDEO: char = '\u{e1a5}';
    pub const SLIDERS_HORIZONTAL: char = '\u{e29a}';
    pub const CIRCLE_ALERT: char = '\u{e077}';
    pub const LOADER_CIRCLE: char = '\u{e10a}';
    pub const WIFI_OFF: char = '\u{e1af}';
    pub const SERVER: char = '\u{e153}';
    pub const GLOBE: char = '\u{e0e8}';
    pub const LANGUAGES: char = '\u{e0fe}';
    pub const ZAP: char = '\u{e1b4}';
    pub const MOON_STAR: char = '\u{e410}';
    pub const BELL_RING: char = '\u{e224}';
    pub const LAYOUT_GRID: char = '\u{e0ff}';
    pub const PANEL_RIGHT: char = '\u{e431}';
    pub const HASH: char = '\u{e0ef}';
    pub const TAG: char = '\u{e17f}';
    pub const BAN: char = '\u{e051}';
    pub const SQUARE_CHECK: char = '\u{e559}';
    pub const SQUARE: char = '\u{e167}';
    pub const CIRCLE: char = '\u{e076}';
    pub const MAIL_OPEN: char = '\u{e363}';
    pub const CLOCK_ALERT: char = '\u{e62a}';
    pub const ALARM_CLOCK: char = '\u{e03a}';
    pub const CALENDAR_CLOCK: char = '\u{e304}';
    pub const SEND: char = '\u{e152}';
}

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
pub const FONT_LUCIDE: &[u8] = include_bytes!("../assets/fonts/lucide.ttf");
pub const ALL_FONTS: [&[u8]; 5] = [FONT_REGULAR, FONT_MEDIUM, FONT_SEMIBOLD, FONT_BOLD, FONT_LUCIDE];
pub const CLEAR: Color = Color::TRANSPARENT;

static LOAD_FONTS: Once = Once::new();

pub fn ensure_fonts_loaded() {
    LOAD_FONTS.call_once(|| {
        let mut fs = font_system().write().expect("font system");
        for bytes in ALL_FONTS {
            fs.load_font(bytes.into());
        }
    });
}

pub const fn fw(weight: Weight) -> Font {
    Font { family: Family::Name("Inter"), weight, ..Font::DEFAULT }
}

fn glyphs(s: &str, size: f32, w: Weight) -> (f32, Vec<(usize, f32)>) {
    ensure_fonts_loaded();
    let mut fs = font_system().write().expect("font system");
    let mut buf = cosmic_text::Buffer::new(fs.raw(), cosmic_text::Metrics::new(size, size * 1.2));
    buf.set_text(fs.raw(), s, &to_attributes(fw(w)), cosmic_text::Shaping::Advanced, None);
    let mut out = Vec::new();
    let mut total = 0.0;
    for g in buf.layout_runs().flat_map(|r| r.glyphs.iter()) {
        total += g.w;
        out.push((g.end, total));
    }
    (total, out)
}

pub fn text_width(s: &str, size: f32, w: Weight) -> f32 {
    glyphs(s, size, w).0
}

pub fn ell<'a>(s: &str, max: f32, size: f32, w: Weight) -> text::Text<'a> {
    type Key = (String, u32, u32, Weight);
    thread_local!(static CACHE: RefCell<HashMap<Key, Rc<str>>> = RefCell::default());
    let s = s.lines().next().unwrap_or("");
    let key = (s.to_owned(), max.max(0.0).to_bits(), size.to_bits(), w);
    let out = CACHE.with(|c| c.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let (total, gs) = glyphs(s, size, w);
        let res: Rc<str> = if total <= max {
            s.into()
        } else {
            let cut = max - text_width("…", size, w);
            let idx = gs.partition_point(|g| g.1 <= cut);
            let end = gs.get(idx.wrapping_sub(1)).map_or(0, |g| g.0);
            format!("{}…", s[..end].trim_end()).into()
        };
        CACHE.with(|c| {
            let mut m = c.borrow_mut();
            if m.len() > 4096 {
                m.clear();
            }
            m.insert(key, res.clone());
        });
        res
    });
    t(out.to_string(), size, w).wrapping(text::Wrapping::None)
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
    text(c).font(Font::with_name("lucide")).size(size).line_height(1.0)
}

pub fn hue(n: usize) -> Color {
    const HUES: [[u8; 3]; 10] = [
        [217, 119, 87],
        [88, 101, 242],
        [214, 54, 201],
        [31, 185, 61],
        [234, 169, 0],
        [22, 178, 238],
        [245, 77, 11],
        [116, 89, 255],
        [32, 180, 161],
        [37, 97, 251],
    ];
    let [r, g, b] = HUES[n % HUES.len()];
    Color::from_rgb8(r, g, b)
}

pub fn hash_str(s: &str) -> usize {
    s.bytes().fold(5381usize, |acc, b| acc.wrapping_mul(33).wrapping_add(b as usize))
}

pub fn initials(title: &str) -> String {
    let mut chars = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).filter_map(|w| w.chars().next());
    let first = chars.next().unwrap_or('B').to_ascii_uppercase();
    chars.next().map_or_else(|| first.to_string(), |s| format!("{}{}", first, s.to_ascii_uppercase()))
}

pub fn mix(a: Color, b: Color, k: f32) -> Color {
    Color::from_rgba(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k, a.a + (b.a - a.a) * k)
}

const BRANDS: [(&str, &str, &str, &str); 17] = [
    ("whatsapp", "#25d366", "#fff", include_str!("../assets/brands/whatsapp.svg")),
    ("facebook", "url(#g)", "#fff", include_str!("../assets/brands/messenger.svg")),
    ("instagram", "url(#g)", "#fff", include_str!("../assets/brands/instagram.svg")),
    ("telegram", "#2aabee", "#fff", include_str!("../assets/brands/telegram.svg")),
    ("signal", "#3b45fd", "#fff", include_str!("../assets/brands/signal.svg")),
    ("viber", "#7360f2", "#fff", include_str!("../assets/brands/viber.svg")),
    ("imessage", "#34c759", "#fff", include_str!("../assets/brands/imessage.svg")),
    ("gmessages", "#1a73e8", "#fff", include_str!("../assets/brands/googlemessages.svg")),
    ("googlechat", "#00ac47", "#fff", include_str!("../assets/brands/googlechat.svg")),
    ("twitter", "#000", "#fff", include_str!("../assets/brands/x.svg")),
    ("slack", "#fff", "#4a154b", include_str!("../assets/brands/slack.svg")),
    ("discord", "#5865f2", "#fff", include_str!("../assets/brands/discord.svg")),
    ("linkedin", "#0a66c2", "#fff", include_str!("../assets/brands/linkedin.svg")),
    ("line", "#06c755", "#fff", include_str!("../assets/brands/line.svg")),
    ("tumblr", "#36465d", "#fff", include_str!("../assets/brands/tumblr.svg")),
    ("bluesky", "#1185fe", "#fff", include_str!("../assets/brands/bluesky.svg")),
    ("beeper", "#3844ff", "#fff", include_str!("../assets/brands/matrix.svg")),
];

static BRAND_SVGS: LazyLock<HashMap<&'static str, svg::Handle>> = LazyLock::new(|| {
    BRANDS
        .iter()
        .map(|(id, bg, fg, src)| {
            let d = src.split(" d=\"").nth(1).and_then(|s| s.split('"').next()).expect("simple-icons path");
            let grad = if *id == "instagram" {
                "<linearGradient id='g' x1='0' y1='1' x2='1' y2='0'><stop offset='0' stop-color='#fdd060'/><stop offset='.45' stop-color='#f0285a'/><stop offset='1' stop-color='#8a3ad9'/></linearGradient>"
            } else {
                "<linearGradient id='g' x1='0' y1='1' x2='1' y2='0'><stop offset='0' stop-color='#0a7cff'/><stop offset='.6' stop-color='#a033ff'/><stop offset='1' stop-color='#ff5280'/></linearGradient>"
            };
            let doc = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 28 28'><defs>{grad}</defs><rect width='28' height='28' rx='8' fill='{bg}'/><g transform='translate(6.5 6.5) scale(.625)'><path fill='{fg}' d='{d}'/></g></svg>");
            (*id, svg::Handle::from_memory(doc.into_bytes()))
        })
        .collect()
});

pub fn brand_svg(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS.get(bridge_id).unwrap_or(&BRAND_SVGS["beeper"]).clone()
}

pub fn fill_box(bg: Color, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style { background: Some(bg.into()), border: border::rounded(r), ..Default::default() }
}

pub fn bordered_box(bg: Color, border_col: Color, width: f32, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style { background: Some(bg.into()), border: Border { color: border_col, width, radius: r.into() }, ..Default::default() }
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

pub fn btn_style(bgs: [Color; 3], fgs: [Color; 3], r: f32) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    use button::Status::*;
    move |_, s| {
        let k = usize::from(matches!(s, Hovered)) + 2 * usize::from(matches!(s, Pressed));
        let a = if matches!(s, Disabled) { 0.45 } else { 1.0 };
        button::Style { background: Some(bgs[k].scale_alpha(a).into()), text_color: fgs[k].scale_alpha(a), border: border::rounded(r), ..Default::default() }
    }
}

pub fn input_style(bg: Color, text_col: Color, placeholder_col: Color, accent_col: Color, r: f32) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + Copy {
    move |_, _| text_input::Style {
        background: bg.into(),
        border: Border { color: CLEAR, width: 0.0, radius: r.into() },
        icon: placeholder_col,
        placeholder: placeholder_col,
        value: text_col,
        selection: accent_col.scale_alpha(0.45),
    }
}

pub fn pick_style(bg: Color, hover_bg: Color, text_col: Color, sub_col: Color) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style + Copy {
    move |_, status| pick_list::Style {
        text_color: text_col,
        placeholder_color: sub_col,
        handle_color: sub_col,
        background: (if matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) { hover_bg } else { bg }).into(),
        border: Border { color: CLEAR, width: 0.0, radius: 6.0.into() },
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

pub fn scroll_style(thumb: Color, thumb_hover: Color) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + Copy {
    let rail = |c: Color| scrollable::Rail { background: None, border: Border::default(), scroller: scrollable::Scroller { background: c.into(), border: border::rounded(4) } };
    move |_, status| {
        let col = match status {
            scrollable::Status::Active { .. } => CLEAR,
            scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. } | scrollable::Status::Dragged { .. } => thumb_hover,
            scrollable::Status::Hovered { .. } => thumb,
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail(col),
            horizontal_rail: rail(CLEAR),
            gap: None,
            auto_scroll: scrollable::AutoScroll { background: CLEAR.into(), border: Border::default(), shadow: Shadow::default(), icon: CLEAR },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub frame: Color,
    pub rail: Color,
    pub rail_sel: Color,
    pub card: Color,
    pub selected: Color,
    pub hover: Color,
    pub text: Color,
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
    pub prefs_group: Color,
    pub prefs_border: Color,
    pub nav_sel: Color,
    pub search: Color,
    pub on_secondary: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
}

const fn hex(v: u32) -> Color {
    Color::from_rgb(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0)
}

const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

pub const BEEPER_DARK: Palette = Palette {
    frame: hex(0x282828),
    rail: hex(0x3d3d3d),
    rail_sel: hex(0x505050),
    card: hex(0x000000),
    selected: hex(0x3e3e3e),
    hover: hexa(0xf6f6f6, 0.05),
    text: hex(0xededed),
    strong: hex(0xf6f6f6),
    sub: hexa(0xebebf5, 0.62),
    weak: hex(0xadadad),
    muted: hex(0x808080),
    tertiary: hexa(0xebebf5, 0.27),
    divider: hexa(0xf6f6f6, 0.2),
    elevated: hex(0x282828),
    input: hex(0x222223),
    send_btn: hex(0x4d4d4d),
    sent: hex(0x2a61fb),
    sent_meta: hexa(0xffffff, 0.6),
    received: hex(0x232326),
    link_card: hex(0x2c2c2c),
    outline: hex(0x28282a),
    date_text: hex(0x9a9aa1),
    accent: hex(0x2561fb),
    toggle_on: hex(0x0d4ffb),
    toggle_off: hexa(0xffffff, 0.3),
    prefs_pane: hex(0x1c1c1c),
    prefs_group: hex(0x222222),
    prefs_border: hex(0x2a2a2c),
    nav_sel: hex(0x3c3c3c),
    search: hex(0x323232),
    on_secondary: hex(0x9b9ba2),
    danger: hex(0xff383c),
    success: hex(0x34c759),
    warning: hex(0xf54c03),
};

pub const BEEPER_LIGHT: Palette = Palette {
    frame: hex(0xf0f0f0),
    rail: hex(0xe1e1e1),
    rail_sel: hex(0xcfcfcf),
    card: hex(0xffffff),
    selected: hex(0xe0e0e0),
    hover: hexa(0x000000, 0.04),
    text: hex(0x1c1c1e),
    strong: hex(0x000000),
    sub: hexa(0x3c3c43, 0.62),
    weak: hex(0x5e5e63),
    muted: hex(0x8e8e93),
    tertiary: hexa(0x3c3c43, 0.3),
    divider: hexa(0x000000, 0.12),
    elevated: hex(0xffffff),
    input: hex(0xf0f0f2),
    send_btn: hex(0xd5d5d8),
    sent: hex(0x0d4ffb),
    sent_meta: hexa(0xffffff, 0.7),
    received: hex(0xe9e9eb),
    link_card: hex(0xefeff1),
    outline: hex(0xe2e2e4),
    date_text: hex(0x6c6c72),
    accent: hex(0x0d4ffb),
    toggle_on: hex(0x0d4ffb),
    toggle_off: hexa(0x000000, 0.25),
    prefs_pane: hex(0xf7f7f7),
    prefs_group: hex(0xffffff),
    prefs_border: hex(0xe5e5ea),
    nav_sel: hex(0xdedede),
    search: hex(0xe6e6e6),
    on_secondary: hex(0x6c6c72),
    danger: hex(0xff383c),
    success: hex(0x34c759),
    warning: hex(0xf54c03),
};

#[derive(Debug, Clone)]
pub struct ResolvedTheme {
    pub choice: ThemeChoice,
    pub is_dark: bool,
    pub vars: HashMap<String, String>,
    pub selector_rules: HashMap<String, HashMap<String, String>>,
    pub bg_window: [u8; 4],
    pub bg_sidebar: [u8; 4],
    pub bg_right_sidebar: [u8; 4],
    pub bg_header: [u8; 4],
    pub bg_popover: [u8; 4],
    pub bg_tooltip: [u8; 4],
    pub text_primary: [u8; 4],
    pub text_secondary: [u8; 4],
    pub text_muted: [u8; 4],
    pub brand_primary: [u8; 4],
    pub brand_blue: [u8; 4],
    pub brand_green: [u8; 4],
    pub brand_red: [u8; 4],
    pub brand_yellow: [u8; 4],
    pub bubble_self: [u8; 4],
    pub bubble_other: [u8; 4],
    pub bubble_self_text: [u8; 4],
    pub bubble_other_text: [u8; 4],
    pub divider: [u8; 4],
    pub active_thread_bg: [u8; 4],
    pub header_height: f32,
    pub account_switcher_width: f32,
    pub min_sidebar_width: f32,
    pub max_sidebar_width: f32,
    pub threads_list_item_height: f32,
    pub pinned_thread_base_size: f32,
    pub cv_avatar_size: f32,
    pub bubble_radius: f32,
}

pub fn strip_css_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let b = input.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == 47 && b[i + 1] == 42 {
            i += 2;
            while i + 1 < b.len() && !(b[i] == 42 && b[i + 1] == 47) {
                i += 1;
            }
            i += 2;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn find_matching_brace(s: &str, open_idx: usize) -> usize {
    let bytes = s.as_bytes();
    assert_eq!(bytes[open_idx], b'{');
    let mut depth = 0usize;
    for (idx, &b) in bytes[open_idx..].iter().enumerate() {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return open_idx + idx;
            }
        }
    }
    panic!("unclosed brace in CSS at {open_idx}");
}

pub fn parse_css_sheet(css: &str, prefer_dark: bool, vars: &mut HashMap<String, String>, selector_rules: &mut HashMap<String, HashMap<String, String>>) {
    let clean = strip_css_comments(css);
    let mut pos = 0usize;
    while let Some(rel_open) = clean[pos..].find('{') {
        let open_idx = pos + rel_open;
        let close_idx = find_matching_brace(&clean, open_idx);
        let prelude = clean[pos..open_idx].trim();
        let inner = &clean[open_idx + 1..close_idx];
        if prelude.starts_with("@media") {
            let dark_m = prelude.contains("prefers-color-scheme") && prelude.contains("dark");
            let light_m = prelude.contains("prefers-color-scheme") && prelude.contains("light");
            if (dark_m && prefer_dark) || (light_m && !prefer_dark) || (!dark_m && !light_m) {
                parse_css_sheet(inner, prefer_dark, vars, selector_rules);
            }
        } else if !prelude.starts_with("@keyframes") && !prelude.starts_with("@font-face") {
            for decl in inner.split(';') {
                if let Some((k, v)) = decl.trim().split_once(':') {
                    let prop = k.trim();
                    let val = v.trim().trim_end_matches("!important").trim().to_string();
                    if prop.starts_with("--") {
                        vars.insert(prop.to_string(), val.clone());
                    }
                    for sel in prelude.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        selector_rules.entry(sel.to_string()).or_default().insert(prop.to_string(), val.clone());
                    }
                }
            }
        }
        pos = close_idx + 1;
    }
}

pub fn resolve_var_value(raw: &str, vars: &HashMap<String, String>, depth: u8) -> String {
    assert!(depth <= 12, "CSS var recursion limit exceeded for {raw}");
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("var(") {
        let after_var = &trimmed[start + 4..];
        let mut p_depth = 1usize;
        let mut end_idx = 0usize;
        for (i, ch) in after_var.char_indices() {
            if ch == '(' {
                p_depth += 1;
            } else if ch == ')' {
                p_depth -= 1;
                if p_depth == 0 {
                    end_idx = i;
                    break;
                }
            }
        }
        let inside = &after_var[..end_idx];
        let (var_name, default_val) = inside.split_once(',').map_or((inside.trim(), None), |(n, d)| (n.trim(), Some(d.trim())));
        let rep = vars.get(var_name).map(String::as_str).or(default_val).expect("unresolved CSS var");
        return resolve_var_value(&format!("{}{}{}", &trimmed[..start], rep, &after_var[end_idx + 1..]), vars, depth + 1);
    }
    trimmed.to_string()
}

pub fn parse_css_color(raw: &str, vars: &HashMap<String, String>) -> [u8; 4] {
    let v = resolve_var_value(raw, vars, 0).trim().to_ascii_lowercase();
    match v.as_str() {
        "transparent" | "none" => return [0, 0, 0, 0],
        "white" => return [255, 255, 255, 255],
        "black" => return [0, 0, 0, 255],
        _ => {}
    }
    if let Some(hex) = v.strip_prefix('#') {
        let h = hex.split_whitespace().next().unwrap();
        let u = |r: std::ops::Range<usize>, m: u8| u8::from_str_radix(&h[r], 16).unwrap() * m;
        return match h.len() {
            3 => [u(0..1, 17), u(1..2, 17), u(2..3, 17), 255],
            4 => [u(0..1, 17), u(1..2, 17), u(2..3, 17), u(3..4, 17)],
            6 => [u(0..2, 1), u(2..4, 1), u(4..6, 1), 255],
            8 => [u(0..2, 1), u(2..4, 1), u(4..6, 1), u(6..8, 1)],
            _ => panic!("invalid CSS hex #{h}"),
        };
    }
    let inner = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")).expect("css color").trim_end_matches(')').replace('/', ",");
    let p: Vec<&str> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let ch = |s: &str| s.strip_suffix('%').map_or_else(|| s.parse::<f32>().unwrap(), |pct| pct.parse::<f32>().unwrap() * 2.55).clamp(0.0, 255.0) as u8;
    let a = if p.len() >= 4 {
        p[3].strip_suffix('%').map_or_else(
            || {
                let af = p[3].parse::<f32>().unwrap();
                (if af <= 1.0 { af * 255.0 } else { af }).clamp(0.0, 255.0) as u8
            },
            |pct| (pct.parse::<f32>().unwrap() * 2.55).clamp(0.0, 255.0) as u8,
        )
    } else {
        255
    };
    [ch(p[0]), ch(p[1]), ch(p[2]), a]
}

pub fn parse_css_px(raw: &str, vars: &HashMap<String, String>) -> f32 {
    let v = resolve_var_value(raw, vars, 0).trim().to_ascii_lowercase();
    if let Some(px) = v.strip_suffix("px") {
        px.trim().parse::<f32>().unwrap()
    } else if let Some(rem) = v.strip_suffix("rem") {
        rem.trim().parse::<f32>().unwrap() * 16.0
    } else {
        v.parse::<f32>().unwrap()
    }
}

pub fn to_iced_color_opaque(rgba: [u8; 4]) -> Color {
    Color::from_rgb8(rgba[0], rgba[1], rgba[2])
}

impl ResolvedTheme {
    pub fn compile(settings: &BeeperSettings) -> Self {
        let is_dark = !matches!(settings.theme, ThemeChoice::TextsLight);
        let mut vars = HashMap::new();
        let mut selector_rules = HashMap::new();
        parse_css_sheet(CSS_VARIABLES, is_dark, &mut vars, &mut selector_rules);
        parse_css_sheet(CSS_TEXTS, is_dark, &mut vars, &mut selector_rules);
        match settings.theme {
            ThemeChoice::TextsLight | ThemeChoice::TextsDark => {}
            ThemeChoice::Dracula => parse_css_sheet(CSS_DRACULA, true, &mut vars, &mut selector_rules),
            ThemeChoice::Nocturne => parse_css_sheet(CSS_NOCTURNE, true, &mut vars, &mut selector_rules),
            ThemeChoice::DarkPurple => parse_css_sheet(CSS_DARK_PURPLE, true, &mut vars, &mut selector_rules),
            ThemeChoice::Custom => {}
        }
        if !settings.custom_css_source.trim().is_empty() {
            parse_css_sheet(&settings.custom_css_source, is_dark, &mut vars, &mut selector_rules);
        }
        if settings.fix_lockdown_emojis {
            parse_css_sheet(CSS_FIX_LOCKDOWN_EMOJIS, is_dark, &mut vars, &mut selector_rules);
        }
        for (flag, cls) in [(settings.no_transparency, ".no-transparency"), (settings.reduce_transparency, ".reduce-transparency")] {
            if flag {
                if let Some(rules) = selector_rules.get(cls).cloned() {
                    for (k, v) in rules.into_iter().filter(|(k, _)| k.starts_with("--")) {
                        vars.insert(k, v);
                    }
                }
            }
        }
        let get_c = |name: &str| parse_css_color(vars.get(name).unwrap(), &vars);
        let get_px = |name: &str| parse_css_px(vars.get(name).unwrap(), &vars);
        let mut bg_window = get_c("--color-background-app");
        let mut bg_sidebar = get_c("--color-background-sidebar-opaque");
        if matches!(settings.theme, ThemeChoice::Nocturne) {
            if let Some(base) = vars.get("--glass-bg-base") {
                bg_window = parse_css_color(base, &vars);
            }
            if let Some(deep) = vars.get("--glass-bg-deep") {
                bg_sidebar = parse_css_color(deep, &vars);
            }
        }
        Self {
            choice: settings.theme,
            is_dark,
            bg_window,
            bg_sidebar,
            bg_right_sidebar: get_c("--right-pane-bg"),
            bg_header: get_c("--color-background-header-right-opaque"),
            bg_popover: get_c("--color-background-menu-opaque"),
            bg_tooltip: get_c("--color-background-elevated"),
            text_primary: get_c("--color-text-neutrals"),
            text_secondary: get_c("--color-text-neutrals-weak"),
            text_muted: get_c("--color-text-neutrals-subtle"),
            brand_primary: get_c("--color-primary"),
            brand_blue: get_c("--color-base-system-blue-90"),
            brand_green: get_c("--sms-sent-bg"),
            brand_red: get_c("--error-color"),
            brand_yellow: get_c("--warning-color"),
            bubble_self: get_c("--color-background-message-bubble-sent"),
            bubble_other: get_c("--color-background-message-bubble-received"),
            bubble_self_text: get_c("--color-text-on-accent"),
            bubble_other_text: get_c("--color-text-neutrals"),
            divider: get_c("--color-border-neutrals"),
            active_thread_bg: get_c("--color-background-sidebar-thread-selected"),
            header_height: get_px("--header-height"),
            account_switcher_width: get_px("--account-switcher-width"),
            min_sidebar_width: get_px("--min-sidebar-width"),
            max_sidebar_width: get_px("--max-sidebar-width"),
            threads_list_item_height: get_px("--threads-list-item-height"),
            pinned_thread_base_size: get_px("--pinned-thread-base-size"),
            cv_avatar_size: get_px("--cv-avatar-size"),
            bubble_radius: get_px("--border-radius-conversation-bubble"),
            vars,
            selector_rules,
        }
    }

    pub fn pal(&self) -> Palette {
        let c = |v: [u8; 4]| Color::from_rgba8(v[0], v[1], v[2], v[3] as f32 / 255.0);
        match self.choice {
            ThemeChoice::TextsDark => BEEPER_DARK,
            ThemeChoice::TextsLight => BEEPER_LIGHT,
            _ => {
                let frame = c(self.bg_sidebar);
                let card = c(self.bg_window);
                let text = c(self.text_primary);
                Palette {
                    frame,
                    rail: mix(frame, text, 0.1),
                    rail_sel: mix(frame, text, 0.2),
                    card,
                    selected: mix(frame, text, 0.1),
                    elevated: mix(card, text, 0.12),
                    input: mix(card, text, 0.1),
                    link_card: mix(card, text, 0.14),
                    received: c(self.bubble_other),
                    sent: c(self.bubble_self),
                    accent: c(self.brand_primary),
                    toggle_on: c(self.brand_primary),
                    text,
                    strong: text,
                    sub: c(self.text_secondary),
                    weak: c(self.text_secondary),
                    muted: c(self.text_muted),
                    prefs_pane: mix(frame, card, 0.5),
                    prefs_group: mix(card, text, 0.08),
                    nav_sel: mix(frame, text, 0.12),
                    search: mix(frame, text, 0.06),
                    ..BEEPER_DARK
                }
            }
        }
    }
}

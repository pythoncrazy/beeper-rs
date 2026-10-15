use beeper_core::config::NETWORK_SPECS;
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
    pub const BELL: char = '\u{e059}';
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
    pub const FILE_TEXT: char = '\u{e0cc}';
    pub const PLAY: char = '\u{e13c}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const BOOKMARK: char = '\u{e060}';
    pub const MESSAGE_SQUARE: char = '\u{e117}';
}

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
pub const FONT_ICONS: &[u8] = include_bytes!("../assets/fonts/icons.ttf");
pub const ALL_FONTS: [&[u8]; 5] = [FONT_REGULAR, FONT_MEDIUM, FONT_SEMIBOLD, FONT_BOLD, FONT_ICONS];
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
    text(c).font(Font::with_name("icons")).size(size).line_height(1.0)
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
    let mut chars =
        title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).filter_map(|w| w.chars().next());
    let first = chars.next().unwrap_or('B').to_ascii_uppercase();
    chars.next().map_or_else(|| first.to_string(), |s| format!("{}{}", first, s.to_ascii_uppercase()))
}

pub fn mix(a: Color, b: Color, k: f32) -> Color {
    Color::from_rgba(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k, a.a + (b.a - a.a) * k)
}

const BRANDS: [(&str, &str, &str, &str); 20] = [
    ("whatsapp", "#48c95f", "#fff", include_str!("../assets/brands/whatsapp.svg")),
    ("twitter", "#202124", "#fff", include_str!("../assets/brands/x.svg")),
    ("telegram", "#2ea4db", "#fff", include_str!("../assets/brands/telegram.svg")),
    ("slack", "#f6f6f6", "#4a154b", include_str!("../assets/brands/slack.svg")),
    ("discord", "#5865f2", "#fff", include_str!("../assets/brands/discord.svg")),
    ("facebook", "#f6f6f6", "#0866ff", include_str!("../assets/brands/messenger.svg")),
    ("instagram", "url(#g)", "#fff", include_str!("../assets/brands/instagram.svg")),
    ("linkedin", "#086ce1", "#fff", include_str!("../assets/brands/linkedin.svg")),
    ("gmessages", "#f6f6f6", "#086ce1", include_str!("../assets/brands/googlemessages.svg")),
    ("signal", "#3542ff", "#fff", include_str!("../assets/brands/signal.svg")),
    ("googlechat", "#f6f6f6", "#00ac47", include_str!("../assets/brands/googlechat.svg")),
    ("line", "#06c755", "#fff", include_str!("../assets/brands/line.svg")),
    ("gvoice", "#f6f6f6", "#02c553", include_str!("../assets/brands/googlemessages.svg")),
    ("tumblr", "#100f0d", "#fff", include_str!("../assets/brands/tumblr.svg")),
    ("viber", "#7360f2", "#fff", include_str!("../assets/brands/viber.svg")),
    ("snapchat", "#fffc00", "#000", include_str!("../assets/brands/snapchat.svg")),
    ("wechat", "#07c160", "#fff", include_str!("../assets/brands/wechat.svg")),
    ("imessage", "#34c759", "#fff", include_str!("../assets/brands/imessage.svg")),
    ("bluesky", "#0085ff", "#fff", include_str!("../assets/brands/bluesky.svg")),
    ("hungryserv", "#3844ff", "#fff", include_str!("../assets/brands/matrix.svg")),
];

static BRAND_SVGS: LazyLock<HashMap<&'static str, svg::Handle>> = LazyLock::new(|| {
    assert!(NETWORK_SPECS.iter().all(|n| BRANDS.iter().any(|b| b.0 == n.bridge_id)), "every network has a logo");
    BRANDS
        .iter()
        .map(|(id, bg, fg, src)| {
            let d = src.split(" d=\"").nth(1).and_then(|s| s.split('"').next()).expect("simple-icons path");
            let grad = if *id == "instagram" {
                "<linearGradient id='g' x1='0' y1='1' x2='1' y2='0'><stop offset='0' stop-color='#fdd060'/><stop offset='.45' stop-color='#f0285a'/><stop offset='1' stop-color='#8a3ad9'/></linearGradient>"
            } else {
                "<linearGradient id='g' x1='0' y1='1' x2='1' y2='0'><stop offset='0' stop-color='#0a7cff'/><stop offset='.6' stop-color='#a033ff'/><stop offset='1' stop-color='#ff5280'/></linearGradient>"
            };
            let doc = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 28 28'><defs>{grad}</defs><rect width='28' height='28' rx='10' fill='{bg}'/><g transform='translate(4.5 4.5) scale(.7917)'><path fill='{fg}' d='{d}'/></g></svg>");
            (*id, svg::Handle::from_memory(doc.into_bytes()))
        })
        .collect()
});

pub fn brand_svg(bridge_id: &str) -> svg::Handle {
    BRAND_SVGS[bridge_id].clone()
}

pub fn fill_box(bg: Color, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style { background: Some(bg.into()), border: border::rounded(r), ..Default::default() }
}

pub fn bordered_box(bg: Color, border_col: Color, width: f32, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width, radius: r.into() },
        ..Default::default()
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
        border: Border { color: CLEAR, width: 0.0, radius: 6.0.into() },
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

pub fn scroll_style(
    thumb: Color,
    thumb_hover: Color,
) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + Copy {
    let rail = |c: Color| scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { background: c.into(), border: border::rounded(4) },
    };
    move |_, status| {
        let col = match status {
            scrollable::Status::Active { .. } => CLEAR,
            scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. }
            | scrollable::Status::Dragged { .. } => thumb_hover,
            scrollable::Status::Hovered { .. } => thumb,
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail(col),
            horizontal_rail: rail(CLEAR),
            gap: None,
            auto_scroll: scrollable::AutoScroll {
                background: CLEAR.into(),
                border: Border::default(),
                shadow: Shadow::default(),
                icon: CLEAR,
            },
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
    toggle_off: hex(0x444444),
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

pub fn palette(dark: bool) -> Palette {
    if dark {
        BEEPER_DARK
    } else {
        BEEPER_LIGHT
    }
}

use crate::config::{
    CSS_DARK_PURPLE, CSS_DRACULA, CSS_FIX_LOCKDOWN_EMOJIS, CSS_NOCTURNE, CSS_TEXTS, CSS_VARIABLES,
};
use crate::models::{BeeperSettings, ThemeChoice};
use iced::advanced::graphics::text::{cosmic_text, font_system, to_attributes};
use iced::font::{Family, Font, Weight};
use iced::overlay::menu;
use iced::widget::{button, container, pick_list, scrollable, text, text_input, toggler};
use iced::{border, Border, Color, Shadow, Theme, Vector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Once;

pub mod icon {
    pub const PLAY: char = '\u{e13c}';
    pub const PAUSE: char = '\u{e12e}';
    pub const NEXT: char = '\u{e160}';
    pub const PREV: char = '\u{e15f}';
    pub const SHUFFLE: char = '\u{e15e}';
    pub const REPEAT: char = '\u{e146}';
    pub const REPEAT1: char = '\u{e1fd}';
    pub const HEART: char = '\u{e0f2}';
    pub const HOME: char = '\u{e0f5}';
    pub const SEARCH: char = '\u{e151}';
    pub const LIBRARY: char = '\u{e100}';
    pub const LIST_MUSIC: char = '\u{e2e0}';
    pub const LINES: char = '\u{e55a}';
    pub const LYRICS: char = '\u{e349}';
    pub const VOL1: char = '\u{e1aa}';
    pub const VOL2: char = '\u{e1ab}';
    pub const MUTE: char = '\u{e1ac}';
    pub const PLUS: char = '\u{e13d}';
    pub const DOWN: char = '\u{e0b2}';
    pub const SETTINGS: char = '\u{e154}';
    pub const MUSIC: char = '\u{e122}';
    pub const DISC: char = '\u{e494}';
    pub const ALBUM: char = '\u{e03b}';
    pub const RADIO: char = '\u{e142}';
    pub const CHEV_DOWN: char = '\u{e06d}';
    pub const CHEV_UP: char = '\u{e070}';
    pub const LEFT: char = '\u{e06e}';
    pub const RIGHT: char = '\u{e06f}';
    pub const X: char = '\u{e1b2}';
    pub const CHECK: char = '\u{e06c}';
    pub const MORE: char = '\u{e0b6}';
    pub const BACK: char = '\u{e06e}';
    pub const FWD: char = '\u{e06f}';
    pub const CLOCK: char = '\u{e087}';
    pub const LISTPLUS: char = '\u{e23f}';
    pub const SERVER: char = '\u{e153}';
    pub const USER: char = '\u{e19f}';
    pub const USERS: char = '\u{e1a4}';
    pub const LOGOUT: char = '\u{e10e}';
    pub const STAR: char = '\u{e176}';
    pub const FOLDER: char = '\u{e0d7}';
    pub const CPLAY: char = '\u{e080}';
    pub const CPAUSE: char = '\u{e07f}';
    pub const REFRESH: char = '\u{e145}';
    pub const PENCIL: char = '\u{e1f9}';
    pub const TRASH: char = '\u{e18e}';
    pub const INFO: char = '\u{e0f9}';
    pub const SPIN: char = '\u{e10a}';
    pub const OFFLINE: char = '\u{e1af}';
    pub const ALERT: char = '\u{e077}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const LIB_OPEN: char = '\u{e54e}';
    pub const PL: char = '\u{e12a}';
    pub const PR: char = '\u{e431}';
    pub const SLIDERS: char = '\u{e55a}';
    pub const QUEUE: char = '\u{e2e0}';
    pub const IMAGE: char = '\u{e0ff}';
    pub const LIST: char = '\u{e106}';
    pub const GRID: char = '\u{e0ff}';
    pub const ADD: char = '\u{e081}';
    pub const FULL: char = '\u{e112}';
    pub const RESTORE: char = '\u{e11a}';
    pub const DEVICE: char = '\u{e210}';
}

pub mod fill {
    pub const PLAY: char = '\u{e3d0}';
    pub const PAUSE: char = '\u{e39e}';
    pub const HEART: char = '\u{e2a8}';
    pub const HOME: char = '\u{e2c2}';
    pub const LIBRARY: char = '\u{e0ce}';
    pub const STAR: char = '\u{e46a}';
    pub const SAVED: char = '\u{e184}';
}

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Figtree-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Figtree-Medium.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Figtree-Bold.ttf");
pub const FONT_BLACK: &[u8] = include_bytes!("../assets/fonts/Figtree-Black.ttf");
pub const FONT_LUCIDE: &[u8] = include_bytes!("../assets/fonts/lucide.ttf");
pub const FONT_PHOSPHOR: &[u8] = include_bytes!("../assets/fonts/phosphor-fill.ttf");
pub const CLEAR: Color = Color::TRANSPARENT;

static LOAD_FONTS: Once = Once::new();

pub fn ensure_fonts_loaded() {
    LOAD_FONTS.call_once(|| {
        let mut fs = font_system().write().expect("font system");
        for bytes in [
            FONT_REGULAR,
            FONT_MEDIUM,
            FONT_BOLD,
            FONT_BLACK,
            FONT_LUCIDE,
            FONT_PHOSPHOR,
        ] {
            fs.load_font(bytes.into());
        }
    });
}

pub const fn fw(weight: Weight) -> Font {
    Font {
        family: Family::Name("Figtree"),
        weight,
        ..Font::DEFAULT
    }
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
            let cut = max - text_width("...", size, w);
            let idx = gs.partition_point(|g| g.1 <= cut);
            let end = gs.get(idx.wrapping_sub(1)).map_or(0, |g| g.0);
            format!("{}...", s[..end].trim_end()).into()
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
    text(out.to_string()).size(size).font(fw(w)).wrapping(text::Wrapping::None)
}

pub fn txt<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    text(s).size(size).font(fw(Weight::Normal))
}

pub fn med<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    text(s).size(size).font(fw(Weight::Medium))
}

pub fn bold<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    text(s).size(size).font(fw(Weight::Bold))
}

pub fn black<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    text(s).size(size).font(fw(Weight::Black))
}

pub fn ic(c: char, size: f32) -> text::Text<'static> {
    text(c).font(Font::with_name("lucide")).size(size).line_height(1.0)
}

pub fn filled_ic(c: char, size: f32) -> text::Text<'static> {
    text(c).font(Font::with_name("Phosphor-Fill")).size(size).line_height(1.0)
}

pub fn hue(n: usize) -> Color {
    const HUES: [[u8; 3]; 10] = [
        [29, 185, 84],
        [45, 70, 185],
        [140, 25, 50],
        [232, 17, 91],
        [20, 138, 8],
        [186, 93, 7],
        [80, 155, 245],
        [175, 40, 150],
        [71, 125, 149],
        [225, 51, 0],
    ];
    let [r, g, b] = HUES[n % HUES.len()];
    Color::from_rgb8(r, g, b)
}

pub fn hash_str(s: &str) -> usize {
    s.bytes().fold(5381usize, |acc, b| acc.wrapping_mul(33).wrapping_add(b as usize))
}

pub fn initials(title: &str) -> String {
    let mut chars = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter_map(|w| w.chars().next());
    let first = chars.next().unwrap_or('B').to_ascii_uppercase();
    chars.next().map_or_else(|| first.to_string(), |s| format!("{}{}", first, s.to_ascii_uppercase()))
}

pub fn fill_box(bg: Color, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: border::rounded(r),
        ..Default::default()
    }
}

pub fn bubble_box(bg: Color, is_sender: bool) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| {
        let radius = if is_sender {
            border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 6.0, bottom_left: 16.0 }
        } else {
            border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 16.0, bottom_left: 6.0 }
        };
        container::Style {
            background: Some(bg.into()),
            border: Border { color: CLEAR, width: 0.0, radius },
            ..Default::default()
        }
    }
}

pub fn composer_box(bg: Color, border_col: Color) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border {
            color: border_col,
            width: 1.0,
            radius: border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 8.0, bottom_left: 16.0 },
        },
        ..Default::default()
    }
}

pub fn bordered_box(bg: Color, border_col: Color, width: f32, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width, radius: r.into() },
        ..Default::default()
    }
}

pub fn elevated_card(bg: Color, border_col: Color, r: f32) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border { color: border_col, width: 1.0, radius: r.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
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

pub fn btn_bordered_style(bgs: [Color; 3], fgs: [Color; 3], border_col: Color, r: f32) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    use button::Status::*;
    move |_, s| {
        let k = usize::from(matches!(s, Hovered)) + 2 * usize::from(matches!(s, Pressed));
        button::Style {
            background: Some(bgs[k].into()),
            text_color: fgs[k],
            border: Border { color: border_col, width: 1.0, radius: r.into() },
            ..Default::default()
        }
    }
}

pub fn input_style(bgs: [Color; 2], borders: [Color; 2], text_col: Color, placeholder_col: Color, accent_col: Color, widths: [f32; 2], r: f32) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + Copy {
    move |_, s| {
        let k = usize::from(matches!(s, text_input::Status::Focused { .. }));
        text_input::Style {
            background: bgs[k].into(),
            border: Border { color: borders[k], width: widths[k], radius: r.into() },
            icon: placeholder_col,
            placeholder: placeholder_col,
            value: text_col,
            selection: accent_col,
        }
    }
}

pub fn tog_style(active_bg: Color, inactive_bg: Color, fg: Color) -> impl Fn(&Theme, toggler::Status) -> toggler::Style + Copy {
    move |t, status| {
        let is_on = matches!(status, toggler::Status::Active { is_toggled: true } | toggler::Status::Hovered { is_toggled: true } | toggler::Status::Disabled { is_toggled: true });
        let mut st = toggler::default(t, status);
        st.background = (if is_on { active_bg } else { inactive_bg }).into();
        st.foreground = fg.into();
        st.border_radius = Some(10.0.into());
        st.padding_ratio = 0.12;
        st
    }
}

pub fn pick_style(bg: Color, hover_bg: Color, text_col: Color, sub_col: Color, border_col: Color) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style + Copy {
    move |_, status| {
        let cur_bg = if matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) { hover_bg } else { bg };
        pick_list::Style {
            text_color: text_col,
            placeholder_color: sub_col,
            handle_color: sub_col,
            background: cur_bg.into(),
            border: Border { color: border_col, width: 1.0, radius: 6.0.into() },
        }
    }
}

pub fn pick_menu_style(bg: Color, hover_bg: Color, text_col: Color, border_col: Color) -> impl Fn(&Theme) -> menu::Style + Copy {
    move |_| menu::Style {
        background: bg.into(),
        border: Border { color: border_col, width: 1.0, radius: 8.0.into() },
        text_color: text_col,
        selected_text_color: Color::WHITE,
        selected_background: hover_bg.into(),
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 4.0), blur_radius: 14.0 },
    }
}

pub fn scroll_style(thumb: Color, thumb_hover: Color) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + Copy {
    let rail = |c: Color| scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { background: c.into(), border: border::rounded(3) },
    };
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

    pub fn app_bg(&self) -> Color { to_iced_color_opaque(self.bg_window) }
    pub fn rail_bg(&self) -> Color {
        if self.is_dark {
            let [r, g, b, _] = self.bg_window;
            Color::from_rgb8(r.saturating_sub(4), g.saturating_sub(4), b.saturating_sub(4))
        } else {
            Color::from_rgb8(236, 236, 239)
        }
    }
    pub fn sidebar_bg(&self) -> Color { to_iced_color_opaque(self.bg_sidebar) }
    pub fn elevated_bg(&self) -> Color { to_iced_color_opaque(self.bg_tooltip) }
    pub fn elevated_hover_bg(&self) -> Color {
        let [r, g, b, _] = self.bg_tooltip;
        if self.is_dark {
            Color::from_rgb8(r.saturating_add(14), g.saturating_add(14), b.saturating_add(14))
        } else {
            Color::from_rgb8(r.saturating_sub(12), g.saturating_sub(12), b.saturating_sub(12))
        }
    }
    pub fn header_bg(&self) -> Color { to_iced_color_opaque(self.bg_header) }
    pub fn menu_bg(&self) -> Color { to_iced_color_opaque(self.bg_popover) }
    pub fn input_bg(&self) -> Color {
        self.vars.get("--color-background-input").map_or_else(|| self.elevated_bg(), |r| to_iced_color_opaque(parse_css_color(r, &self.vars)))
    }
    pub fn text_color(&self) -> Color { to_iced_color_opaque(self.text_primary) }
    pub fn sub_color(&self) -> Color { to_iced_color_opaque(self.text_secondary) }
    pub fn muted_color(&self) -> Color { to_iced_color_opaque(self.text_muted) }
    pub fn accent_color(&self) -> Color { to_iced_color_opaque(self.brand_primary) }
    pub fn on_accent_color(&self) -> Color { to_iced_color_opaque(self.bubble_self_text) }
    pub fn bubble_sent_color(&self) -> Color { to_iced_color_opaque(self.bubble_self) }
    pub fn bubble_recv_color(&self) -> Color { to_iced_color_opaque(self.bubble_other) }
    pub fn border_color(&self) -> Color { to_iced_color_opaque(self.divider) }
    pub fn border_weak_color(&self) -> Color { to_iced_color_opaque(self.divider).scale_alpha(0.55) }
    pub fn hover_overlay(&self) -> Color {
        if self.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.07) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.06) }
    }
    pub fn active_overlay(&self) -> Color {
        if self.is_dark { Color::from_rgba(1.0, 1.0, 1.0, 0.12) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.10) }
    }
    pub fn danger_color(&self) -> Color { to_iced_color_opaque(self.brand_red) }
    pub fn success_color(&self) -> Color { to_iced_color_opaque(self.brand_green) }
    pub fn warning_color(&self) -> Color { to_iced_color_opaque(self.brand_yellow) }
    pub fn audio_bar_bg(&self) -> Color {
        self.vars.get("--audio-bar-bg").map_or_else(|| self.elevated_bg(), |r| to_iced_color_opaque(parse_css_color(r, &self.vars)))
    }
}

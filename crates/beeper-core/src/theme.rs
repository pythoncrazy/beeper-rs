use crate::config::{CSS_DARK_PURPLE, CSS_DRACULA, CSS_FIX_LOCKDOWN_EMOJIS, CSS_NOCTURNE, CSS_TEXTS, CSS_VARIABLES};
use crate::models::{BeeperSettings, ThemeChoice};
use std::collections::HashMap;

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

pub fn parse_css_sheet(
    css: &str,
    prefer_dark: bool,
    vars: &mut HashMap<String, String>,
    selector_rules: &mut HashMap<String, HashMap<String, String>>,
) {
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
        let (var_name, default_val) =
            inside.split_once(',').map_or((inside.trim(), None), |(n, d)| (n.trim(), Some(d.trim())));
        let rep = vars.get(var_name).map(String::as_str).or(default_val).expect("unresolved CSS var");
        return resolve_var_value(
            &format!("{}{}{}", &trimmed[..start], rep, &after_var[end_idx + 1..]),
            vars,
            depth + 1,
        );
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
    let inner = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))
        .expect("css color")
        .trim_end_matches(')')
        .replace('/', ",");
    let p: Vec<&str> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let ch = |s: &str| {
        s.strip_suffix('%')
            .map_or_else(|| s.parse::<f32>().unwrap(), |pct| pct.parse::<f32>().unwrap() * 2.55)
            .clamp(0.0, 255.0) as u8
    };
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
        for (flag, cls) in
            [(settings.no_transparency, ".no-transparency"), (settings.reduce_transparency, ".reduce-transparency")]
        {
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
}

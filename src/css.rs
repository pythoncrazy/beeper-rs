use crate::config::{
    CSS_DARK_PURPLE, CSS_DRACULA, CSS_FIX_LOCKDOWN_EMOJIS, CSS_NOCTURNE, CSS_TEXTS, CSS_VARIABLES,
};
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
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn find_matching_brace(s: &str, open_idx: usize) -> usize {
    let bytes = s.as_bytes();
    assert_eq!(
        bytes[open_idx], b'{',
        "expected opening brace at {open_idx}"
    );
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
    panic!("unclosed brace in CSS starting at byte {open_idx}");
}

fn parse_declarations_into(
    block: &str,
    selector: &str,
    vars: &mut HashMap<String, String>,
    selector_rules: &mut HashMap<String, HashMap<String, String>>,
) {
    for decl in block.split(';') {
        let decl = decl.trim();
        if let Some((k, v)) = decl.split_once(':') {
            let prop = k.trim();
            let val = v.trim().trim_end_matches("!important").trim().to_string();
            if prop.starts_with("--") {
                vars.insert(prop.to_string(), val.clone());
            }
            for sel in selector.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                selector_rules
                    .entry(sel.to_string())
                    .or_default()
                    .insert(prop.to_string(), val.clone());
            }
        }
    }
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
            let is_dark_media =
                prelude.contains("prefers-color-scheme") && prelude.contains("dark");
            let is_light_media =
                prelude.contains("prefers-color-scheme") && prelude.contains("light");
            if (is_dark_media && prefer_dark)
                || (is_light_media && !prefer_dark)
                || (!is_dark_media && !is_light_media)
            {
                parse_css_sheet(inner, prefer_dark, vars, selector_rules);
            }
        } else if !prelude.starts_with("@keyframes") && !prelude.starts_with("@font-face") {
            parse_declarations_into(inner, prelude, vars, selector_rules);
        }
        pos = close_idx + 1;
    }
}

pub fn resolve_var_value(raw: &str, vars: &HashMap<String, String>, depth: u8) -> String {
    assert!(
        depth <= 12,
        "CSS var(--...) recursion limit exceeded for {raw}"
    );
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("var(") {
        let after_var = &trimmed[start + 4..];
        let mut paren_depth = 1usize;
        let mut end_offset = None;
        for (i, ch) in after_var.char_indices() {
            if ch == '(' {
                paren_depth += 1;
            } else if ch == ')' {
                paren_depth -= 1;
                if paren_depth == 0 {
                    end_offset = Some(i);
                    break;
                }
            }
        }
        let end_idx = end_offset.expect("unclosed var(...) in CSS");
        let inside = &after_var[..end_idx];
        let (var_name, default_val) = match inside.split_once(',') {
            Some((n, d)) => (n.trim(), Some(d.trim())),
            None => (inside.trim(), None),
        };
        let replacement = vars
            .get(var_name)
            .map(String::as_str)
            .or(default_val)
            .expect("unresolved CSS variable with no default");
        let combined = format!(
            "{}{}{}",
            &trimmed[..start],
            replacement,
            &after_var[end_idx + 1..]
        );
        return resolve_var_value(&combined, vars, depth + 1);
    }
    trimmed.to_string()
}

pub fn parse_css_color(raw: &str, vars: &HashMap<String, String>) -> [u8; 4] {
    let val = resolve_var_value(raw, vars, 0);
    let v = val.trim().to_ascii_lowercase();
    match v.as_str() {
        "transparent" | "none" => return [0, 0, 0, 0],
        "white" => return [255, 255, 255, 255],
        "black" => return [0, 0, 0, 255],
        _ => {}
    }
    if let Some(hex) = v.strip_prefix('#') {
        let hex = hex.split_whitespace().next().unwrap();
        return match hex.len() {
            3 => [
                u8::from_str_radix(&hex[0..1], 16).unwrap() * 17,
                u8::from_str_radix(&hex[1..2], 16).unwrap() * 17,
                u8::from_str_radix(&hex[2..3], 16).unwrap() * 17,
                255,
            ],
            4 => [
                u8::from_str_radix(&hex[0..1], 16).unwrap() * 17,
                u8::from_str_radix(&hex[1..2], 16).unwrap() * 17,
                u8::from_str_radix(&hex[2..3], 16).unwrap() * 17,
                u8::from_str_radix(&hex[3..4], 16).unwrap() * 17,
            ],
            6 => [
                u8::from_str_radix(&hex[0..2], 16).unwrap(),
                u8::from_str_radix(&hex[2..4], 16).unwrap(),
                u8::from_str_radix(&hex[4..6], 16).unwrap(),
                255,
            ],
            8 => [
                u8::from_str_radix(&hex[0..2], 16).unwrap(),
                u8::from_str_radix(&hex[2..4], 16).unwrap(),
                u8::from_str_radix(&hex[4..6], 16).unwrap(),
                u8::from_str_radix(&hex[6..8], 16).unwrap(),
            ],
            _ => panic!("invalid CSS hex color: #{hex}"),
        };
    }
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let inner = inner.trim_end_matches(')').replace('/', ",");
        let parts: Vec<&str> = inner
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        assert!(
            parts.len() >= 3,
            "expected at least 3 channels in rgb/rgba: {v}"
        );
        let parse_chan = |s: &str| -> u8 {
            if let Some(pct) = s.strip_suffix('%') {
                ((pct.parse::<f32>().unwrap() / 100.0) * 255.0).clamp(0.0, 255.0) as u8
            } else {
                s.parse::<f32>().unwrap().clamp(0.0, 255.0) as u8
            }
        };
        let a = if parts.len() >= 4 {
            if let Some(pct) = parts[3].strip_suffix('%') {
                ((pct.parse::<f32>().unwrap() / 100.0) * 255.0).clamp(0.0, 255.0) as u8
            } else {
                let af = parts[3].parse::<f32>().unwrap();
                if af <= 1.0 {
                    (af * 255.0).clamp(0.0, 255.0) as u8
                } else {
                    af.clamp(0.0, 255.0) as u8
                }
            }
        } else {
            255
        };
        return [
            parse_chan(parts[0]),
            parse_chan(parts[1]),
            parse_chan(parts[2]),
            a,
        ];
    }
    panic!("unsupported CSS color value: {v} (from {raw})");
}

pub fn parse_css_px(raw: &str, vars: &HashMap<String, String>) -> f32 {
    let val = resolve_var_value(raw, vars, 0);
    let v = val.trim().to_ascii_lowercase();
    if let Some(px) = v.strip_suffix("px") {
        return px.trim().parse::<f32>().unwrap();
    }
    if let Some(rem) = v.strip_suffix("rem") {
        return rem.trim().parse::<f32>().unwrap() * 16.0;
    }
    v.parse::<f32>().unwrap()
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
            ThemeChoice::Dracula => {
                parse_css_sheet(CSS_DRACULA, true, &mut vars, &mut selector_rules)
            }
            ThemeChoice::Nocturne => {
                parse_css_sheet(CSS_NOCTURNE, true, &mut vars, &mut selector_rules)
            }
            ThemeChoice::DarkPurple => {
                parse_css_sheet(CSS_DARK_PURPLE, true, &mut vars, &mut selector_rules)
            }
            ThemeChoice::Custom => {
                if !settings.custom_css_source.trim().is_empty() {
                    parse_css_sheet(
                        &settings.custom_css_source,
                        is_dark,
                        &mut vars,
                        &mut selector_rules,
                    );
                }
            }
        }
        if settings.theme != ThemeChoice::Custom && !settings.custom_css_source.trim().is_empty() {
            parse_css_sheet(
                &settings.custom_css_source,
                is_dark,
                &mut vars,
                &mut selector_rules,
            );
        }
        if settings.fix_lockdown_emojis {
            parse_css_sheet(
                CSS_FIX_LOCKDOWN_EMOJIS,
                is_dark,
                &mut vars,
                &mut selector_rules,
            );
        }
        for (flag, cls) in [
            (settings.no_transparency, ".no-transparency"),
            (settings.reduce_transparency, ".reduce-transparency"),
        ] {
            if flag {
                if let Some(rules) = selector_rules.get(cls).cloned() {
                    for (k, v) in rules {
                        if k.starts_with("--") {
                            vars.insert(k, v);
                        }
                    }
                }
            }
        }
        assert!(!vars.is_empty(), "CSS variables map must not be empty");

        let get_color = |name: &str| parse_css_color(vars.get(name).unwrap(), &vars);
        let get_px = |name: &str| parse_css_px(vars.get(name).unwrap(), &vars);

        Self {
            choice: settings.theme,
            is_dark,
            bg_window: get_color("--color-background-app"),
            bg_sidebar: get_color("--color-background-sidebar"),
            bg_right_sidebar: get_color("--right-pane-bg"),
            bg_header: get_color("--color-background-header-right"),
            bg_popover: get_color("--color-background-menu"),
            bg_tooltip: get_color("--color-background-elevated"),
            text_primary: get_color("--color-text-neutrals"),
            text_secondary: get_color("--color-text-neutrals-weak"),
            text_muted: get_color("--color-text-neutrals-subtle"),
            brand_primary: get_color("--color-primary"),
            brand_blue: get_color("--color-base-system-blue-90"),
            brand_green: get_color("--sms-sent-bg"),
            brand_red: get_color("--error-color"),
            brand_yellow: get_color("--warning-color"),
            bubble_self: get_color("--color-background-message-bubble-sent"),
            bubble_other: get_color("--color-background-message-bubble-received"),
            bubble_self_text: get_color("--color-text-on-accent"),
            bubble_other_text: get_color("--color-text-neutrals"),
            divider: get_color("--color-border-neutrals"),
            active_thread_bg: get_color("--color-background-sidebar-thread-selected"),
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

    pub fn to_color32(rgba: [u8; 4]) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
    }

    pub fn apply_to_egui(&self, ctx: &egui::Context, zoom_percent: u16) {
        let mut visuals = if self.is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = Self::to_color32(self.bg_window);
        visuals.window_fill = Self::to_color32(self.bg_popover);
        visuals.override_text_color = Some(Self::to_color32(self.text_primary));
        visuals.selection.bg_fill = Self::to_color32(self.brand_primary);
        visuals.widgets.noninteractive.bg_fill = Self::to_color32(self.bg_sidebar);
        visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0_f32, Self::to_color32(self.divider));
        ctx.set_visuals(visuals);
        ctx.set_zoom_factor((zoom_percent as f32 / 100.0).clamp(0.5, 2.5));
    }
}

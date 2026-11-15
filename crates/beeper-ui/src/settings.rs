//! The Settings window: its sections, the settings search, the hotkey recorder, and the Storage, Integrations,
//! Devices and Accounts pages.

use crate::i18n::{tr, trf, trn, trnx, trs, trx};
use crate::style::{
    bar, bold, bordered_box, bordered_btn, brand_svg, btn_style, ell, fill_box, fw, glow_box, gutter_style, hex, ic,
    icon, input_style, med, scroll_style, semi, text_width, txt, Palette, Token, CLEAR,
};
use crate::{
    ago, avatar, badge, blocked_chats, brand_key, checkbox, compact, distance, divider, downloads_dir, duration,
    ease_out, feedback_btn, first_groups, fmt_phone, group, grouped, human_size, icon_btn, inbox_label, login,
    outline_input, pill_btn, pill_style, plan_pill, pref_row, reconnect, record, select, slider_style, status_text,
    style, svg_size, theme_cards, tinted, toggle, toggle_faded, BeeperGuiApp, El, SpaceMenu, Toast, UiMessage,
    ACCOUNT_SUB_W, ADD_ICON, APP_QR, ASK_CLEAR_CACHE, ASK_CLEAR_CHAT, ASK_CLEAR_STORAGE, ASK_LOGOUT, ASK_LOGOUT_DEVICE,
    ASK_REMOTE_ACCESS, ASK_REMOVE_ACCOUNT, ASK_REVOKE_CONNECTION, CHANGELOG_URL, CHEVRON_LEFT, CLAUDE_LOGO,
    COPIED_ICON, COPY_ICON, CURSOR_LOGO, DEVICE_ICONS, ERROR_CIRCLE, INBOX_OPTIONS, ISSUES_URL, LOGO, MENU_ITEM,
    ORANGE, QR_CODE, REVERT, SELECT_ARROW, SETTINGS_SIZE, TAKE_BEEPER, VSCODE_LOGO, ZOOM_LEVEL,
};
use beeper_core::config::{now_unix_millis, now_unix_secs, APP_VERSION};
use beeper_core::matrix::DeviceInfo;
use beeper_core::models::*;
use beeper_core::prefs;
use beeper_core::storage;
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::keyboard::{self, key::Named, Key};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, mouse_area, operation, rich_text, row, scrollable, slider, span, stack, svg, text_input,
    tooltip, Column, Row, Space,
};
use iced::{window, Center, Color, Fill, Length, Padding, Pixels, Task, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Profile,
    Accounts,
    General,
    Appearance,
    Notifications,
    Hotkeys,
    Devices,
    Ai,
    BeeperPlus,
    Integrations,
    Storage,
    Help,
    About,
}

#[derive(Clone, Copy)]
pub enum TabIcon {
    /// The avatar of the Beeper account.
    Avatar,
    Glyph(char),
    Svg(&'static [u8]),
}

impl SettingsTab {
    pub const ALL: [Self; 13] = [
        Self::Profile,
        Self::Accounts,
        Self::General,
        Self::Appearance,
        Self::Notifications,
        Self::Hotkeys,
        Self::Devices,
        Self::Ai,
        Self::BeeperPlus,
        Self::Integrations,
        Self::Storage,
        Self::Help,
        Self::About,
    ];

    /// Desktop's nav items: Beeper Plus shows only when the account can subscribe.
    pub fn listed(st: &BeeperStore) -> impl Iterator<Item = Self> {
        let plus = st.whoami.can_subscribe;
        Self::ALL.into_iter().filter(move |t| *t != Self::BeeperPlus || plus)
    }

    /// The translated name of the section.
    pub fn name(self) -> &'static str {
        tr(self.meta().0)
    }

    /// Desktop's `searchKeywords` of the section.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Profile => &["recovery code", "show recovery key", "backup key", "encryption key"],
            Self::Integrations => &[
                "beeper desktop api",
                "mcp",
                "openapi",
                "access token",
                "approved connections",
                "remote access",
                "claude",
                "raycast",
                "cursor",
                "codex",
            ],
            _ => &[],
        }
    }

    /// Desktop's `.account-pref-pane` gives the Accounts, Devices and Storage panes `--appearance-color-bg-primary`.
    pub fn pane(self, p: &Palette) -> Color {
        match self {
            Self::Accounts | Self::Devices | Self::Storage => p.card,
            _ => p.prefs_pane,
        }
    }

    /// The name, the icon, and the token of the icon tile color.
    pub fn meta(self) -> (&'static str, TabIcon, Token) {
        use TabIcon::{Glyph, Svg};
        let tabs: [(_, _, Token); 13] = [
            ("Profile", TabIcon::Avatar, |_| CLEAR),
            ("Accounts", Glyph(icon::AT_SIGN), |_| hex(0x9745ea)),
            ("General", Svg(include_bytes!("../assets/settings/general.svg")), |_| hex(0x0eb3ef)),
            ("Appearance", Svg(include_bytes!("../assets/settings/appearance.svg")), |_| hex(0xd833ca)),
            ("Notifications", Svg(include_bytes!("../assets/settings/notifications.svg")), |_| hex(0x19ba3b)),
            ("Hotkeys", Svg(include_bytes!("../assets/settings/hotkeys.svg")), |_| hex(0xebaa00)),
            ("Devices", Svg(include_bytes!("../assets/settings/devices.svg")), |_| hex(0xf54c03)),
            ("AI", Svg(include_bytes!("../assets/settings/ai.svg")), |_| hex(0x19c9d4)),
            ("Beeper Plus", Svg(include_bytes!("../assets/settings/beeper_plus.svg")), |p| p.accent),
            ("Integrations", Glyph(icon::PUZZLE), |_| hex(0x7559ff)),
            ("Storage", Glyph(icon::DATABASE), |_| hex(0x32546a)),
            ("Help", Svg(include_bytes!("../assets/settings/help.svg")), |p| p.on_secondary),
            ("About", Svg(include_bytes!("../assets/settings/about.svg")), |_| hex(0x1ab5a2)),
        ];
        tabs[self as usize]
    }
}

/// A row of the settings search results.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsHit {
    /// A section, and whether its name matches.
    Section(SettingsTab, bool),
    Pref(SettingsTab, &'static prefs::Pref),
    /// A custom item by its name, like the theme cards.
    Custom(SettingsTab, &'static str),
    /// A network to add an account on.
    Network(&'static str),
}

impl SettingsHit {
    pub fn label(self) -> String {
        match self {
            Self::Section(tab, _) => tab.name().into(),
            Self::Pref(_, r) => r.name.map_or_else(|| trs(r.label, r.split_args().0), |n| tr(n).into()),
            Self::Custom(_, name) => tr(name).into(),
            Self::Network(name) => name.into(),
        }
    }
}

/// The name of a key in the hotkey combinations.
pub(crate) fn hotkey_key(key: &Key) -> Option<String> {
    Some(match key.as_ref() {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(Named::ArrowUp) => "up".into(),
        Key::Named(Named::ArrowDown) => "down".into(),
        Key::Named(Named::ArrowLeft) => "left".into(),
        Key::Named(Named::ArrowRight) => "right".into(),
        Key::Named(Named::Enter) => "enter".into(),
        Key::Named(Named::Tab) => "tab".into(),
        Key::Named(
            n @ (Named::Backspace
            | Named::Delete
            | Named::Home
            | Named::End
            | Named::PageUp
            | Named::PageDown
            | Named::F1
            | Named::F2
            | Named::F3
            | Named::F4
            | Named::F5
            | Named::F6
            | Named::F7
            | Named::F8
            | Named::F9
            | Named::F10
            | Named::F11
            | Named::F12
            | Named::F13
            | Named::F14
            | Named::F15
            | Named::F16
            | Named::F17
            | Named::F18
            | Named::F19
            | Named::F20
            | Named::F21
            | Named::F22
            | Named::F23
            | Named::F24),
        ) => format!("{n:?}").to_lowercase(),
        _ => return None,
    })
}

/// The text of Desktop's relaunch toast after a hotkey change.
pub(crate) const HOTKEYS_RELAUNCH: &str = "%s has to be relaunched for all hotkey changes to take effect";

/// Desktop's `ShortcutRecorder` while it records: the hotkey, the modifiers (Ctrl, Shift, Alt) and the key pressed
/// since the recording started, and whether Escape moved the focus to the Save and Cancel buttons.
#[derive(Debug, Clone)]
pub struct HotkeyRec {
    pub name: &'static str,
    pub(crate) mods: [bool; 3],
    pub(crate) key: Option<String>,
    pub(crate) buttons: bool,
}

impl HotkeyRec {
    /// The combination in Desktop's format, like "ctrl+shift+k".
    fn combo(&self) -> String {
        let mods = ["ctrl", "shift", "alt"].into_iter().zip(self.mods).filter(|m| m.1).map(|m| m.0);
        mods.chain(self.key.as_deref()).collect::<Vec<_>>().join("+")
    }

    /// The hotkey that already uses the combination.
    fn conflict(&self, s: &BeeperSettings) -> Option<&'static str> {
        let combo = self.combo();
        (!combo.is_empty()).then(|| prefs::hotkey_conflict(self.name, &combo, |n| s.hotkey_keys(n)))?
    }
}

/// Desktop's hotkey list hides the hotkeys of the features that the account does not show.
fn hotkey_shown(st: &BeeperStore, name: &str) -> bool {
    prefs::hotkey_feature(name).is_none_or(|f| st.feature(f).show)
}

/// Desktop's `Rx`: a settings row of a feature shows when the account shows the feature.
pub(crate) fn shown(st: &BeeperStore, r: &prefs::Pref) -> bool {
    let control = match r.control {
        prefs::Control::ReactionEmoji { general: true } => st.settings.choice("messageDoubleClick") == "React with...",
        prefs::Control::QuickReactions(_) => st.settings.on("quickReactions"),
        _ if matches!(r.key, "autoTranscribe" | "transcriptionLanguage") => st.settings.on("transcribeWithOpenai"),
        _ => true,
    };
    r.feature.is_none_or(|f| st.feature(f).show) && control
}

/// Desktop's `EmojiPickerButton` of a quick reaction in Settings, which opens the emoji picker for `slot`.
fn emoji_button<'a>(p: &Palette, emoji: &str, slot: u8) -> El<'a> {
    button(txt(emoji.to_string(), 16.0))
        .padding([4, 5])
        .style(bordered_btn([CLEAR; 3], [p.text; 3], p.chip, 1.0, 100.0))
        .on_press(UiMessage::QuickEmoji(Some(slot)))
        .into()
}

/// Desktop's settings search: each matching section, then its matching rows, then the other sections when a row
/// matches.
pub fn search_settings(st: &BeeperStore, query: &str) -> Vec<SettingsHit> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    let mut out = Vec::new();
    for tab in SettingsTab::listed(st) {
        let rows: Vec<_> = match tab {
            SettingsTab::Accounts => {
                login::addable().filter(|n| has(n.display_name)).map(|n| SettingsHit::Network(n.display_name)).collect()
            }
            _ => prefs::rows()
                .filter(|(page, r)| *page == tab.meta().0 && !r.label.is_empty() && shown(st, r))
                .filter(|(_, r)| {
                    let (label, desc) = r.split_args();
                    let name = r.name.map_or_else(|| trs(r.label, label), |n| tr(n).into());
                    has(&r.desc.map_or_else(|| name.clone(), |d| format!("{name} {}", trs(d, desc))))
                })
                .map(|(_, r)| SettingsHit::Pref(tab, r))
                .chain(
                    prefs::customs()
                        .filter(|&(page, name, keys)| {
                            page == tab.meta().0 && (has(tr(name)) || keys.iter().any(|k| has(k)))
                        })
                        .map(|(_, name, _)| SettingsHit::Custom(tab, name)),
                )
                .collect(),
        };
        if has(tab.name()) || tab.keywords().iter().any(|k| has(k)) || !rows.is_empty() {
            out.push(SettingsHit::Section(tab, true));
            out.extend(rows);
        }
    }
    if out.iter().any(|h| !matches!(h, SettingsHit::Section(..))) {
        let shown: Vec<_> =
            out.iter().filter_map(|h| if let SettingsHit::Section(t, _) = h { Some(*t) } else { None }).collect();
        out.extend(SettingsTab::listed(st).filter(|t| !shown.contains(t)).map(|t| SettingsHit::Section(t, false)));
    }
    out
}

/// Desktop's Storage page: the scan, its chart, and the filters of the clear for all chats and for the open chat.
#[derive(Default)]
pub(crate) struct StoragePage {
    pub(crate) all: Option<Vec<storage::Entry>>,
    pub(crate) chart: Option<svg::Handle>,
    /// Indices into [`STORAGE_DAYS`].
    pub(crate) ages: [usize; 2],
    /// Bits of [`storage::Kind::ALL`].
    pub(crate) kinds: [u8; 2],
    pub(crate) chat: Option<String>,
    /// The [`UiMessage::ClearStorage`] that runs.
    pub(crate) clearing: Option<u8>,
}

/// Settings > Integrations: the "Install to..." choice, the copied field, and whether the community join runs.
#[derive(Default)]
pub(crate) struct IntegrationsPage {
    pub(crate) target: usize,
    pub(crate) copied: Option<u8>,
    pub(crate) joining: bool,
    /// The token of the open revoke confirmation.
    pub(crate) revoke: Option<String>,
    pub(crate) add: Option<AddToken>,
}

/// Desktop's "Create a new token for Beeper Desktop API" dialog, with the token after Create.
pub(crate) struct AddToken {
    pub(crate) name: String,
    pub(crate) expiry: Expiry,
    pub(crate) write: bool,
    pub(crate) token: Option<String>,
    pub(crate) copied: bool,
}

/// An "Expires in" choice of a token, in seconds; 0 never expires.
#[derive(Clone, Copy, PartialEq)]
pub struct Expiry(pub(crate) i64);

impl std::fmt::Display for Expiry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            3600 => tr("1 hour"),
            86400 => tr("1 day"),
            604_800 => tr("1 week"),
            2_592_000 => tr("30 days"),
            _ => trx("Never", "access token expiry option"),
        })
    }
}

pub(crate) const EXPIRIES: [Expiry; 5] = [Expiry(3600), Expiry(86400), Expiry(604_800), Expiry(2_592_000), Expiry(0)];

/// The Beeper Developer Community room of Desktop.
pub(crate) const COMMUNITY_ROOM: &str = "!VRvJRVNZDbRuKAsKvK:beeper.com";

/// Desktop's MCP clients with their own row, and the button label of each.
const MCP_CLIENTS: [(&str, &str); 3] =
    [("Claude Desktop", "Download Extension"), ("Cursor", "Open in Cursor"), ("VS Code", "Open in Visual Studio Code")];

/// Desktop's other MCP clients, in the "Install to..." select.
const MCP_TARGETS: [&str; 4] = ["Claude Code", "Codex", "Streamable HTTP", "stdio"];

/// Desktop's `buildCopyValue` of an MCP client for the Desktop API at `base`.
pub(crate) fn mcp_value(client: &str, base: &str) -> String {
    let url = format!("{base}/v0/mcp");
    match client {
        "Claude Desktop" => "https://misc-assets.beeper.com/connect-files/BeeperDesktop.dxt".into(),
        "Cursor" => {
            let config = beeper_core::codec::base64_encode_standard(serde_json::json!({ "url": url }).to_string().as_bytes());
            format!("cursor://anysphere.cursor-deeplink/mcp/install?name=beeper&config={}", beeper_core::codec::url_encode(&config))
        }
        "VS Code" => {
            let config = serde_json::json!({ "name": "beeper", "type": "http", "url": url });
            format!("vscode:mcp/install?{}", beeper_core::codec::url_encode(&config.to_string()))
        }
        "Claude Code" => format!("claude mcp add beeper {url} -t http -s user"),
        "Codex" => format!("codex mcp add beeper --url {url}"),
        "Streamable HTTP" => {
            serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": { "beeper": { "url": url } } })).unwrap()
        }
        "stdio" => "{\n  \"mcpServers\": {\n    \"beeper\": {\n      \"command\": \"npx\",\n      \"args\": [\n        \"-y\",\n        \"@beeper/mcp-remote\"\n      ]\n    }\n  }\n}".into(),
        _ => unreachable!("{client} is not an MCP client"),
    }
}

/// Desktop's label and `--color-solids-*` color of each storage type.
fn kind_meta(k: storage::Kind) -> (&'static str, Color) {
    match k {
        storage::Kind::Image => ("Images", Color::from_rgb8(14, 179, 239)),
        storage::Kind::Video => ("Videos", Color::from_rgb8(235, 170, 0)),
        storage::Kind::File => ("Other Files", ORANGE),
        storage::Kind::Voice => ("Audio", Color::from_rgb8(25, 186, 59)),
        storage::Kind::Message => ("Messages", Color::from_rgb8(216, 51, 202)),
        storage::Kind::Cache => ("Cache", Color::from_rgb8(106, 75, 229)),
    }
}

/// The stored bytes of each type that has any.
pub(crate) fn storage_totals(all: &[storage::Entry]) -> Vec<(storage::Kind, u64)> {
    let sum = |k| all.iter().filter(|e| e.kind == k).map(|e| e.bytes).sum::<u64>();
    storage::Kind::ALL.into_iter().map(|k| (k, sum(k))).filter(|t| t.1 > 0).collect()
}

/// Desktop's ECharts storage donut, 200 px with rings from 50 to 70 px, clockwise from the top.
pub(crate) fn donut(parts: &[(storage::Kind, u64)]) -> svg::Handle {
    let total = parts.iter().map(|p| p.1).sum::<u64>() as f32;
    let round = 2.0 * std::f32::consts::PI * 60.0;
    let mut at = 0.0;
    let arcs: String = parts
        .iter()
        .map(|&(k, bytes)| {
            let [r, g, b, _] = kind_meta(k).1.into_rgba8();
            let len = round * bytes as f32 / total;
            let arc = format!(
                r##"<circle cx="100" cy="100" r="60" fill="none" stroke="#{r:02x}{g:02x}{b:02x}" stroke-width="20" stroke-dasharray="{len} {round}" stroke-dashoffset="{}" transform="rotate(-90 100 100)"/>"##,
                -at
            );
            at += len;
            arc
        })
        .collect();
    svg::Handle::from_memory(
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">{arcs}</svg>"#).into_bytes(),
    )
}

/// The dialogs over the Settings window.
pub(crate) const SETTINGS_DIALOGS: [u8; 8] = [
    ASK_LOGOUT,
    ASK_LOGOUT_DEVICE,
    ASK_CLEAR_STORAGE,
    ASK_CLEAR_CACHE,
    ASK_CLEAR_CHAT,
    ASK_REVOKE_CONNECTION,
    ASK_REMOTE_ACCESS,
    ASK_REMOVE_ACCOUNT,
];

/// Desktop's "Clear storage older than" choices, where 0 is any age.
pub(crate) const STORAGE_DAYS: [i64; 4] = [0, 30, 90, 120];

impl BeeperGuiApp {
    pub(crate) fn open_settings(&mut self, tab: SettingsTab) -> Task<UiMessage> {
        let load = self.update(UiMessage::SelectSettingsTab(tab));
        if let Some(id) = self.settings_window {
            return Task::batch([load, window::gain_focus(id)]);
        }
        let (id, open) = window::open(Self::window_settings(SETTINGS_SIZE));
        self.settings_window = Some(id);
        self.save_subwindows();
        Task::batch([load, open.map(UiMessage::WindowOpened)])
    }

    /// Desktop's keys of the settings window: Ctrl F focuses the search, ↑ and ↓ move through the results, and Enter is
    /// the submit of the input. Escape in an input clears the search, and Escape elsewhere closes the window.
    pub(crate) fn settings_key(&mut self, key: Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
        if self.hotkey_rec.is_some() {
            self.record_key(&key, mods);
            return Task::none();
        }
        if mods.command() && key == Key::Character("f".into()) {
            return operation::focus("settings-search");
        }
        if mods.command() || mods.control() || mods.alt() {
            return self.update(UiMessage::KeyPressed(key, mods));
        }
        let n = search_settings(&self.store.read(), &self.settings_query).len();
        match key {
            Key::Named(Named::Escape) if self.confirm.is_some() => self.confirm = None,
            Key::Named(Named::Escape) if self.integrations.add.is_some() => self.integrations.add = None,
            Key::Named(Named::Escape) => {
                return window::close(self.settings_window.expect("the settings window is open"))
            }
            Key::Named(k @ (Named::ArrowUp | Named::ArrowDown)) if n > 0 => {
                let step = if k == Named::ArrowDown { 1 } else { n - 1 };
                return operation::is_focused("settings-search").then(move |on| {
                    if on {
                        Task::done(UiMessage::SettingsStep(step))
                    } else {
                        Task::none()
                    }
                });
            }
            _ => {}
        }
        Task::none()
    }

    /// Desktop's `ShortcutRecorder` keys: Escape moves the focus to the Save and Cancel buttons, where Enter and Escape
    /// press them. Other keys add to the recorded combination, which has at most one key that is not a modifier.
    fn record_key(&mut self, key: &Key, mods: keyboard::Modifiers) {
        let conflict = self.hotkey_rec.as_ref().and_then(|r| r.conflict(&self.store.read().settings));
        let rec = self.hotkey_rec.as_mut().expect("the recorder records");
        match key {
            Key::Named(Named::Escape | Named::Enter) if rec.buttons => {
                if *key == Key::Named(Named::Enter) && conflict.is_none() {
                    self.save_hotkey();
                } else {
                    self.hotkey_rec = None;
                }
                return;
            }
            Key::Named(Named::Escape) => return rec.buttons = true,
            _ if rec.buttons => return,
            _ => {}
        }
        if conflict.is_some() {
            (rec.mods, rec.key) = ([false; 3], None);
        }
        let held = [mods.control() || mods.command(), mods.shift(), mods.alt()];
        let pressed = match key {
            Key::Named(Named::Control) => [true, false, false],
            Key::Named(Named::Shift) => [false, true, false],
            Key::Named(Named::Alt) => [false, false, true],
            _ => {
                let Some(k) = hotkey_key(key) else { return };
                if prefs::MODIFIER_ONLY.contains(&rec.name) || rec.key.is_some() {
                    return;
                }
                rec.key = Some(k);
                [false; 3]
            }
        };
        rec.mods = std::array::from_fn(|i| rec.mods[i] || held[i] || pressed[i]);
    }

    /// Desktop's Save of the `ShortcutRecorder`: it checks the modifiers and stores the combination, unless another
    /// hotkey uses it.
    pub(crate) fn save_hotkey(&mut self) {
        let rec = self.hotkey_rec.clone().expect("the recorder records");
        const NAVIGATION: [&str; 11] =
            ["up", "down", "left", "right", "tab", "backspace", "delete", "home", "end", "pageup", "pagedown"];
        let combo = rec.combo();
        let has_mod = rec.mods.contains(&true);
        let function =
            |k: &str| k.strip_prefix('f').and_then(|n| n.parse().ok()).is_some_and(|n: u8| (1..=24).contains(&n));
        let single = prefs::SINGLE_KEY.contains(&rec.name)
            && !has_mod
            && rec.key.as_deref().is_some_and(|k| NAVIGATION.contains(&k) || function(k));
        let error = match (combo == "enter" || single, prefs::MODIFIER_ONLY.contains(&rec.name)) {
            (false, true) if !has_mod => Some("You must use a modifier key like Ctrl, Shift or Option/Alt"),
            (false, false) if !has_mod => Some("You must also use a modifier key like Ctrl, Shift or Option/Alt"),
            (false, false) if rec.key.is_none() => Some("You must also use a non-modifier key"),
            _ => None,
        };
        if let Some(e) = error {
            return self.show(Toast::new(tr(e).into()));
        }
        if rec.conflict(&self.store.read().settings).is_none() {
            self.patch_settings(|s| drop(s.keybindings.insert(rec.name.into(), combo)));
            (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
        }
    }

    pub(crate) fn scan_storage(&self) -> Task<UiMessage> {
        let store = self.store.clone();
        let scan =
            async { tokio::task::spawn_blocking(move || storage::scan(&store)).await.expect("storage scan thread") };
        Task::perform(scan, UiMessage::StorageScanned)
    }

    /// The entries that [`UiMessage::ClearStorage`] deletes, without the cache of the clear for all chats.
    pub(crate) fn storage_pick(&self, which: u8) -> Vec<&storage::Entry> {
        let s = &self.storage;
        let w = usize::from(which == 2);
        let kinds: Vec<storage::Kind> = match which {
            1 => vec![storage::Kind::Cache],
            _ => storage::Kind::ALL
                .into_iter()
                .enumerate()
                .filter(|(i, _)| s.kinds[w] & 1 << i != 0)
                .map(|k| k.1)
                .collect(),
        };
        let days = if which == 1 { 0 } else { STORAGE_DAYS[s.ages[w]] };
        let chat = s.chat.as_deref().filter(|_| which == 2);
        storage::select(s.all.as_deref().expect("a storage scan"), &kinds, chat, days, now_unix_secs())
    }

    pub(crate) fn load_devices(&mut self) -> Task<UiMessage> {
        if self.active_settings_tab == SettingsTab::General {
            let list = async { tokio::task::spawn_blocking(record::inputs).await.expect("input list worker thread") };
            return Task::perform(list, UiMessage::MicsLoaded);
        }
        if self.active_settings_tab == SettingsTab::Storage {
            return self.scan_storage();
        }
        if self.active_settings_tab != SettingsTab::Devices {
            return Task::none();
        }
        let st = self.store.read();
        match (st.matrix.clone(), st.config.demo) {
            (Some(client), _) => Task::perform(beeper_core::matrix::devices(client), UiMessage::DevicesLoaded),
            (None, true) => {
                let name = format!("beeper-rs {:?}", self.os);
                let verified = st.beeper_state.verified;
                let id = st.beeper_state.device_id.clone();
                let me = DeviceInfo { id, name, model: None, kind: None, last_seen_ms: None, verified, current: true };
                drop(st);
                self.devices = Some(Ok(vec![me]));
                Task::none()
            }
            (None, false) => {
                drop(st);
                self.devices = Some(Err("Matrix is not connected yet".into()));
                Task::none()
            }
        }
    }

    /// Desktop's `deleteDevices`: logs out another device, then loads the list again. A refused auth opens
    /// `ReauthRequiredModal`, which logs out the device again after the sign-in code.
    pub(crate) fn logout_device(&mut self, id: String) -> Task<UiMessage> {
        (self.confirm, self.devices_busy) = (None, true);
        let logout = beeper_core::matrix::delete_device(self.store.clone(), id.clone());
        Task::perform(logout, move |r| UiMessage::DeviceLoggedOut(id.clone(), r))
    }

    pub(crate) fn device_logged_out(&mut self, id: String, r: Result<(), String>) -> Task<UiMessage> {
        match r {
            Ok(()) => return self.load_devices(),
            Err(e) if e == beeper_core::matrix::REAUTH => return self.open_reauth(UiMessage::LogoutDevice(id)),
            Err(_) => {
                self.devices_busy = false;
                let text = "Couldn't delete device. Please report a problem if this keeps happening.";
                self.fail(Toast::new(text.into()));
            }
        }
        Task::none()
    }

    pub fn view_settings(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let me = st.me();
        let name = me.full_name.clone().expect("self name");
        let hits = search_settings(&st, &self.settings_query);
        let search_input = text_input(tr("Search Settings"), &self.settings_query)
            .id("settings-search")
            .on_input(UiMessage::SettingsQueryChanged)
            .on_submit_maybe(hits.get(self.settings_focus).map(|&h| UiMessage::PickSetting(h)))
            .size(13)
            .line_height(Pixels(16.0))
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.text, p.accent, 0.0));
        let clear = (!self.settings_query.is_empty()).then(|| {
            button(ic(icon::CLOSE, 12.0).color(p.strong))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                .on_press(UiMessage::SettingsQueryChanged(String::new()))
        });
        // Desktop's 16 px Material icon is scaled by 1.2 without a change to its 16 px box.
        let search = container(
            row![ic(icon::SEARCH, 16.0 * 1.2).color(p.strong), search_input]
                .push(clear)
                .spacing(8.0 - 1.6)
                .align_y(Center)
                .padding(Padding::new(6.0 - 1.6).left(8.0 - 1.6).right(8.0)),
        )
        .style(fill_box(p.search, 38.0));
        // The search results draw the icons at Desktop's 11 px instead of 13 px.
        let tile = |tab: SettingsTab, small: bool, alpha: f32| {
            let (_, glyph, tint) = tab.meta();
            let em = if small { 11.0 } else { 13.0 };
            let glyph: El = match glyph {
                TabIcon::Avatar => self.face(me.img_url.as_deref(), &name, if small { 16.0 } else { 20.0 }, 0.0),
                TabIcon::Glyph(g) => ic(g, em).color(Color::WHITE.scale_alpha(alpha)).into(),
                TabIcon::Svg(doc) => {
                    let (w, h) = svg_size(doc);
                    svg(svg::Handle::from_memory(doc)).width(w * em / 13.0).height(h * em / 13.0).opacity(alpha).into()
                }
            };
            let fill = tint(p).scale_alpha(alpha);
            let (size, radius) = if small { (16, 4.0) } else { (20, 6.0) };
            container(glyph).center(size).style(fill_box(fill, radius))
        };
        let pill_w = plan_pill(p, st.whoami.plan).1;
        // Desktop's 34 px avatar shrinks to 28.6 px beside the full width details.
        let av = 28.6;
        let detail_w = 231.0 - av - 16.0 - pill_w;
        let profile = || {
            row![
                container(self.face(me.img_url.as_deref(), &name, av, 0.0)).height(34),
                column![ell(&name, detail_w, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(p.strong)]
                    .push(
                        me.email
                            .as_deref()
                            .map(|e| ell(e, detail_w, 10.0, Weight::Normal).line_height(Pixels(12.0)).color(p.strong))
                    )
                    .width(Fill),
                plan_pill(p, st.whoami.plan).0
            ]
            .spacing(8)
            .align_y(Center)
            .padding([6, 8])
        };
        let tabs: Vec<_> = SettingsTab::listed(&st).collect();
        let nav = tabs.iter().enumerate().fold(Column::new(), |c, (i, &tab)| {
            let on = tab == self.active_settings_tab;
            let style = btn_style([if on { p.hover } else { CLEAR }, p.hover, p.hover], [p.text; 3], 8.0);
            if tab == SettingsTab::Profile {
                return c.push(
                    button(profile()).padding(0).width(Fill).style(style).on_press(UiMessage::SelectSettingsTab(tab)),
                );
            }
            // Desktop's `allProperAccounts`: the Beeper account and the chat accounts.
            let count = (tab == SettingsTab::Accounts).then(|| {
                container(txt((st.accounts.len() + 1).to_string(), 11.0).color(p.strong))
                    .center(18)
                    .style(fill_box(p.chip, 9.0))
            });
            let line = row![tile(tab, false, 1.0), txt(tab.name(), 14.0).color(p.strong), Space::new().width(Fill)]
                .push(count)
                .spacing(8)
                .align_y(Center)
                .height(if on { 44 } else { 43 })
                .padding([0, 8]);
            // Desktop's `.pref-option:has(+ .selected)` and `:last-child` have no divider.
            let next_on = tabs.get(i + 1).is_none_or(|n| *n == self.active_settings_tab);
            c.push(button(line).padding(0).width(Fill).style(style).on_press(UiMessage::SelectSettingsTab(tab))).push(
                (!on).then(|| {
                    let rule = if next_on { CLEAR } else { p.divider };
                    container(container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0)))
                        .padding(Padding::ZERO.left(36.0))
                }),
            )
        });
        // Desktop's search results: the query in each label is at full color, and the rest at half.
        let q = self.settings_query.to_lowercase();
        let marked = |label: String, max: f32, base: Color, hi: Color| {
            let lower = label.to_lowercase();
            assert_eq!(lower.len(), label.len(), "{label} keeps its length in lower case");
            let s = style::clamped(&label, max, 1, 13.0, Weight::Normal);
            let cut = s.strip_suffix('…').unwrap_or(&s).len();
            assert!(label.starts_with(&s[..cut]), "{label} has single spaces");
            let (a, b) = lower.find(&q).map_or((0, 0), |i| (i.min(cut), (i + q.len()).min(cut)));
            rich_text![
                span::<(), _>(s[..a].to_string()).color(base),
                span(s[a..b].to_string()).color(hi),
                span(s[b..].to_string()).color(base)
            ]
            .size(13)
            .line_height(Pixels(16.0))
            .wrapping(iced::widget::text::Wrapping::None)
        };
        let results = hits.iter().enumerate().fold(Column::new(), |c, (i, &hit)| {
            let picked = self.settings_pick == Some(hit);
            let (line, alpha): (El, f32) = match hit {
                SettingsHit::Section(tab, true) => {
                    let base = if picked { Color::WHITE } else { p.strong.scale_alpha(0.5) };
                    let title = marked(hit.label(), 207.0, base, p.strong);
                    (row![tile(tab, true, 1.0), title].spacing(8).align_y(Center).padding([4, 8]).into(), 1.0)
                }
                SettingsHit::Section(tab, false) => {
                    let base = if picked { Color::WHITE } else { p.strong }.scale_alpha(0.25);
                    let title = marked(hit.label(), 207.0, base, base);
                    (row![tile(tab, true, 0.5), title].spacing(8).align_y(Center).padding([4, 8]).into(), 0.5)
                }
                _ => {
                    let ink = if picked { Color::WHITE } else { p.strong };
                    let label = marked(hit.label(), 203.0, ink.scale_alpha(0.5), ink);
                    (container(label).padding(Padding::new(6.0).left(36.0).right(8.0)).into(), 1.0)
                }
            };
            let bg = match (picked, i == self.settings_focus) {
                (true, _) => p.accent,
                (false, true) => p.strong.scale_alpha(0.1),
                (false, false) => CLEAR,
            };
            let line = container(line).width(Fill).id(format!("hit-{i}")).style(fill_box(bg.scale_alpha(alpha), 6.0));
            c.push(mouse_area(line).on_enter(UiMessage::SettingsFocus(i)).on_release(UiMessage::PickSetting(hit)))
        });
        let list: El = match self.settings_query.trim().is_empty() {
            true => nav.into(),
            false => container(results).padding(Padding::ZERO.top(3)).into(),
        };
        let left = column![
            container(semi(tr("Settings"), 14.0).color(p.strong)).center_y(35).padding([0, 8]),
            column![
                search,
                scrollable(list).id("settings-nav").direction(bar()).height(Fill).style(scroll_style(p.scroll))
            ]
            .spacing(5)
        ]
        .spacing(8)
        .width(270)
        .padding(Padding::new(8.0).right(15.0));
        let right = container(
            scrollable(container(self.settings_content(&st)).max_width(600).padding(13).center_x(Fill))
                .id("settings-pane")
                .direction(Direction::Vertical(Scrollbar::new().width(15).scroller_width(9).spacing(0)))
                .height(Fill)
                .style(gutter_style(p.gutter)),
        )
        .width(Fill)
        .height(Fill)
        .style(glow_box(self.active_settings_tab.pane(p), Color::BLACK.scale_alpha(0.12), 10.0, 0.0, 10.0));
        // Desktop's `.prefs-right-pane` fills the window width, with a `drop-shadow(0 0 10px #0000001f)`.
        let base =
            container(row![left, container(right).padding(Padding::new(8.0).right(0.0))]).width(Fill).height(Fill);
        drop(st);
        let confirm = self.confirm.filter(|d| SETTINGS_DIALOGS.contains(d)).map(|d| self.view_confirm(d));
        let add = self.integrations.add.as_ref().map(|a| self.view_add_token(a));
        let menu =
            self.chat_menu.as_ref().and_then(|(cid, at)| Some((cid.strip_prefix("prefs:")?, at))).map(|(id, at)| {
                let (menu, sub) = self.account_menu(id, SpaceMenu::Prefs);
                let size = self.settings_size;
                let (w, h) = (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + 4.0 * MENU_ITEM + 11.0);
                let (x, y) = (at.x.min(size.width - w - 8.0), at.y.min(size.height - h - 8.0));
                let menu = container(mouse_area(row![menu].push(sub)).on_press(UiMessage::Noop))
                    .padding(Padding::ZERO.left(x).top(y))
                    .width(Fill)
                    .height(Fill);
                mouse_area(menu).on_press(UiMessage::CloseChatMenu).on_right_press(UiMessage::CloseChatMenu)
            });
        let profile = self.profile_overlay();
        let picker = self.pane.emoji.as_ref().filter(|pk| pk.pref.is_some()).map(|_| {
            let picker = self.view_emoji_picker(&self.store.read()).expect("the picker is open");
            let picker = container(mouse_area(picker).on_press(UiMessage::Noop)).center(Fill);
            mouse_area(picker).on_press(UiMessage::QuickEmoji(None))
        });
        stack![base.style(fill_box(p.window, 0.0))].push(menu).push(add).push(confirm).push(profile).push(picker).into()
    }

    /// Desktop's `BeeperConnectAddTokenModal`, over the Settings window.
    fn view_add_token<'a>(&'a self, a: &'a AddToken) -> El<'a> {
        let p = &self.pal;
        let label = |t: &'static str| txt(tr(t), 12.0).line_height(Pixels(15.0)).color(p.text);
        let content: El<'a> = match &a.token {
            None => {
                let name = text_input("Cursor", &a.name)
                    .id("token-name")
                    .on_input(UiMessage::AddTokenName)
                    .on_submit(UiMessage::CreateToken)
                    .size(12)
                    .width(200)
                    .padding([4, 6])
                    .style(outline_input(p));
                let line = |t, control: El<'a>| -> El<'a> {
                    row![label(t).width(Fill), control].align_y(Center).height(38).padding([0, 8]).into()
                };
                let expiry = select(
                    p,
                    [EXPIRIES[4], EXPIRIES[0], EXPIRIES[1], EXPIRIES[2], EXPIRIES[3]],
                    a.expiry,
                    UiMessage::AddTokenExpiry,
                );
                let sensitive = column![
                    line("Allow sensitive actions", toggle(p, a.write, Some(UiMessage::AddTokenWrite))),
                    container(
                        txt(tr("Grant permission to send messages and modify your account"), 11.0)
                            .line_height(Pixels(14.0))
                            .color(p.sub)
                    )
                    .padding(Padding::new(8.0).top(0.0))
                ];
                let create = button(semi(tr("Create Access Token"), 12.0).line_height(Pixels(15.0)).color(p.accent))
                    .padding([4, 8])
                    .style(btn_style([p.accent.scale_alpha(0.1); 3], [p.accent; 3], 20.0))
                    .on_press(UiMessage::CreateToken);
                let rows = vec![
                    line("Name (optional)", name.into()),
                    divider(p.prefs_border, 0.0),
                    line("Expires In", expiry),
                    divider(p.prefs_border, 0.0),
                    sensitive.into(),
                    divider(p.prefs_border, 0.0),
                    row![Space::new().width(Fill), create].padding(13).into(),
                ];
                group(p, Some(tr("Generate an access token to connect any client")), rows)
            }
            Some(token) => {
                let note =
                    tr("<strong>Important:</strong> This token will only be shown once, please copy and save it.");
                let (strong, rest) = note.split_once("</strong>").expect("the note has a bold part");
                let note =
                    rich_text![span::<(), _>(strong.trim_start_matches("<strong>")).font(fw(Weight::Bold)), span(rest)]
                        .size(13.0)
                        .line_height(Pixels(16.0))
                        .color(p.text);
                let shown = container(semi(token.as_str(), 15.0).line_height(Pixels(20.0)).color(p.strong))
                    .padding([8, 16])
                    .style(fill_box(p.chip, 8.0));
                let icon = if a.copied { &COPIED_ICON } else { &COPY_ICON };
                let copy = row![
                    tinted(icon, 20.0, p.accent),
                    txt(tr(if a.copied { "Copied!" } else { "Copy" }), 13.0).color(p.accent)
                ]
                .spacing(4)
                .align_y(Center)
                .width(72);
                let shown = mouse_area(row![shown, copy].spacing(8).align_y(Center)).on_press(UiMessage::CopyToken);
                column![note, shown].spacing(8).align_x(Center).width(Fill).into()
            }
        };
        let title = container(
            semi(tr("Create a new token for Beeper Desktop API"), 13.0).line_height(Pixels(16.0)).color(p.strong),
        )
        .center_x(Fill)
        .padding([8, 0]);
        let pane = column![title, container(content).padding(Padding::new(16.0).top(16.0))];
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.prefs_pane, 10.0));
        mouse_area(container(pane).padding(8)).on_press(UiMessage::Noop).into()
    }

    /// Desktop's `TakeBeeperWithYou` card.
    fn take_beeper(&self) -> El<'_> {
        let p = &self.pal;
        let action: El<'_> = match self.app_qr {
            true => container(svg(APP_QR.clone()).width(108).height(108))
                .padding(4)
                .style(fill_box(Color::WHITE, 8.0))
                .into(),
            false => button(
                row![tinted(&QR_CODE, 14.0, Color::WHITE), txt(tr("Get the app"), 11.0).color(Color::WHITE)]
                    .spacing(2)
                    .align_y(Center),
            )
            .padding([4, 8])
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 20.0))
            .on_press(UiMessage::ShowAppQr)
            .into(),
        };
        let heading = semi(tr("Take Beeper with you"), 16.0).line_height(Pixels(20.0)).color(p.strong);
        let card = column![svg(TAKE_BEEPER.clone()).width(58).height(48), heading, action].spacing(12).align_x(Center);
        container(card)
            .center_x(Fill)
            .padding(Padding::new(8.0).top(20.0).bottom(12.0))
            .style(fill_box(p.prefs_group, 12.0))
            .into()
    }

    fn device_list<'a>(&self, title: &'a str, list: &[&DeviceInfo]) -> El<'a> {
        let p = &self.pal;
        let rows = list.iter().enumerate().flat_map(|(i, d)| {
            let (glyph, w, h) = match d.kind.as_deref() {
                Some("tablet") => (&DEVICE_ICONS[0], 11, 14),
                Some("computer") => (&DEVICE_ICONS[1], 18, 11),
                _ => (&DEVICE_ICONS[2], 9, 14),
            };
            let glyph = svg(glyph.clone()).width(w).height(h).style(|_, _| svg::Style { color: Some(Color::WHITE) });
            let tile = container(glyph).center(28).style(fill_box(p.accent, 4.0));
            let text = column![semi(d.name.clone(), 12.0).color(p.strong)]
                .push(d.model.clone().map(|m| txt(m, 11.0).color(p.sub)))
                .push(d.last_seen_ms.map(|t| txt(trf("Last seen: %s", &[&ago(t)]), 11.0).color(p.sub)))
                .width(Fill);
            // Desktop's `DeviceRow`: each device has "Logout", which is disabled while the list loads.
            let ask = match d.current {
                true => UiMessage::Ask(Some(ASK_LOGOUT)),
                false => UiMessage::AskLogoutDevice(d.id.clone()),
            };
            let logout = button(semi(tr("Logout"), 12.0).color(p.strong))
                .padding([4, 8])
                .style(btn_style([p.hover, p.selected, p.selected], [p.strong; 3], 20.0))
                .on_press_maybe((!self.devices_busy).then_some(ask));
            let line: El<'a> = row![tile, text, logout].spacing(12).align_y(Center).padding([8, 12]).into();
            [(i > 0).then(|| divider(p.prefs_border, 0.0)), Some(line)].into_iter().flatten()
        });
        column![
            container(txt(title, 12.0).color(p.sub)).padding([8, 0]),
            container(Column::with_children(rows)).style(bordered_box(CLEAR, p.prefs_border, 1.0, 10.0))
        ]
        .into()
    }

    fn settings_content(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let groups: Vec<El<'_>> = match self.active_settings_tab {
            SettingsTab::Accounts
                if self
                    .account_page
                    .as_deref()
                    .is_some_and(|id| id == "hungryserv" || st.bridge_accounts.iter().any(|b| b.account_id == id)) =>
            {
                self.account_page(st, self.account_page.as_deref().expect("an account page"))
            }
            SettingsTab::Accounts => {
                let account = |logo: &str, owner: &str, name: String, cloud: bool, sub: Option<String>| -> El<'_> {
                    let title = row![avatar(owner, 14.0, 0.0, p), semi(name, 14.0).color(p.strong)]
                        .push(cloud.then(|| ic(icon::CLOUD, 13.0).color(p.sub)))
                        .spacing(4)
                        .align_y(Center);
                    let error = status_text(st, logo).map(|s| {
                        let circle = svg(ERROR_CIRCLE.clone()).width(14).height(14).into();
                        self.tip(circle, s, Vec::new(), tooltip::Position::Top)
                    });
                    let line = row![
                        container(svg(brand_svg(brand_key(logo))).width(28).height(28)).center(46),
                        column![title].push(sub.map(|s| txt(s, 13.0).color(p.sub))).width(Fill),
                    ]
                    .push(error)
                    .push(ic(icon::CHEVRON_RIGHT, 16.0).color(p.strong))
                    .spacing(8)
                    .align_y(Center)
                    .padding(Padding::new(4.0).left(8.0).right(16.0));
                    let row = button(line)
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                        .on_press(UiMessage::OpenAccount(Some(logo.into())));
                    mouse_area(row).on_right_press(UiMessage::OpenChatMenu(format!("prefs:{logo}"))).into()
                };
                let me = st.me();
                let rows = std::iter::once(account(
                    "hungryserv",
                    me.full_name.as_deref().expect("self name"),
                    "Beeper".into(),
                    false,
                    me.username.clone(),
                ))
                .chain(st.bridge_accounts.iter().map(|b| {
                    let owner = b.remote_profile.as_ref().and_then(|r| r.name.as_deref()).unwrap_or("");
                    account(&b.account_id, owner, b.network_name.clone(), b.mode == BridgeMode::Cloud, b.display_text())
                }));
                let rows = rows
                    .enumerate()
                    .flat_map(|(i, r)| {
                        [
                            (i > 0).then(|| {
                                container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0)).into()
                            }),
                            Some(r),
                        ]
                    })
                    .flatten();
                let head = row![
                    med(tr("Accounts"), 12.0).color(p.weak),
                    Space::new().width(Fill),
                    icon_btn(p, icon::CIRCLE_PLUS, 16.0, 22.0, p.accent, UiMessage::ToggleAddAccount)
                ]
                .height(26)
                .align_y(Center)
                .padding([0, 4]);
                let remaining = login::FREE_ACCOUNTS.saturating_sub(st.bridge_accounts.len());
                let foot = column![
                    button(med(tr("Add an Account"), 13.0).color(p.accent))
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                        .on_press(UiMessage::ToggleAddAccount),
                    txt(trn("%d Free Account Remaining", "%d Free Accounts Remaining", remaining, &[&remaining]), 12.0)
                        .color(p.sub)
                ]
                .spacing(7)
                .align_x(Center)
                .width(Fill);
                vec![{
                    let card = container(Column::with_children(rows)).width(Fill).padding(1).style(bordered_box(
                        p.prefs_group,
                        p.chip,
                        1.0,
                        8.0,
                    ));
                    column![head, column![card, foot].spacing(11)].spacing(8).into()
                }]
                .into_iter()
                .chain(self.adding_account.then(|| group(p, None, vec![login::network_grid(st, "", p, p.strong)])))
                .collect()
            }
            SettingsTab::General => self.pref_items(prefs::GENERAL, st),
            SettingsTab::Appearance => self.pref_items(prefs::APPEARANCE, st),
            SettingsTab::Notifications => self.pref_items(prefs::NOTIFICATIONS, st),
            SettingsTab::Hotkeys => vec![self.hotkeys_page(st)],
            // Desktop's `DevicesPane`: the app card, the lists of devices, and the error.
            SettingsTab::Devices => {
                let lists = match &self.devices {
                    None => Vec::new(),
                    Some(Err(_)) => {
                        let error = tr("Failed to fetch devices. Please report a problem if this keeps happening.");
                        vec![txt(error, 13.0).color(p.danger).into()]
                    }
                    Some(Ok(all)) => [
                        (tr("Current Device"), all.iter().filter(|d| d.current).collect::<Vec<_>>()),
                        (tr("Verified Devices"), all.iter().filter(|d| !d.current && d.verified).collect()),
                        (tr("Unverified Devices"), all.iter().filter(|d| !d.current && !d.verified).collect()),
                    ]
                    .into_iter()
                    .filter(|(_, list)| !list.is_empty())
                    .map(|(title, list)| self.device_list(title, &list))
                    .collect(),
                };
                let page = column![self.take_beeper(), Space::new().height(8)].extend(lists);
                vec![page.into()]
            }
            SettingsTab::Ai => self.pref_items(prefs::AI, st),
            SettingsTab::BeeperPlus => vec![group(
                p,
                None,
                vec![pref_row(
                    p,
                    "Beeper Plus",
                    Some("Incognito mode, custom themes, and more."),
                    Some(badge(p, prefs::Badge::Plus)),
                    txt("Free plan", 12.0).color(p.sub).into(),
                )],
            )],
            SettingsTab::Integrations => self.integrations_page(st),
            SettingsTab::Storage => vec![self.storage_page(st)],
            SettingsTab::Help => unreachable!("the Help tab opens the help center"),
            SettingsTab::Profile => self.profile_page(st),
            SettingsTab::About => {
                let title =
                    column![bold("Beeper", 19.5).color(p.text), txt(format!("v{APP_VERSION}"), 13.0).color(p.weak)]
                        .spacing(4);
                let head = row![svg(LOGO.clone()).width(68).height(68), title].spacing(6).align_y(Center);
                let links = [
                    (tr("View changelog"), UiMessage::OpenUrl(CHANGELOG_URL.into())),
                    (tr("Report a problem"), UiMessage::OpenUrl(ISSUES_URL.into())),
                    (tr("Privacy"), UiMessage::OpenUrl("https://www.beeper.com/privacy".into())),
                    (
                        tr("Legal Notices"),
                        UiMessage::OpenQuickLook(
                            tr("Legal Notices").into(),
                            "beeper-rs is MIT licensed. It is not affiliated with Beeper Inc. or Automattic Inc."
                                .into(),
                        ),
                    ),
                ];
                let rows = links.into_iter().enumerate().flat_map(|(i, (label, msg))| {
                    let last = i == 3;
                    let link = button(container(txt(label, 13.0).color(p.link)).center_y(if last { 40 } else { 39 }))
                        .width(Fill)
                        .padding([0, 13])
                        .style(btn_style([CLEAR; 3], [p.link; 3], 0.0))
                        .on_press(msg);
                    let line = container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0));
                    [Some(link.into()), (!last).then(|| line.into())].into_iter().flatten()
                });
                vec![column![head, group(p, None, rows.collect())].spacing(13).into()]
            }
        };
        let banner = (!self.status_banner.is_empty()).then(|| txt(self.status_banner.clone(), 12.0).color(p.sub));
        column![].push(banner).extend(groups).spacing(28).into()
    }

    /// Desktop's `StoragePrefsSubView`.
    /// Desktop's `BeeperConnectPrefsSubView`.
    fn integrations_page<'a>(&'a self, st: &BeeperStore) -> Vec<El<'a>> {
        let p = &self.pal;
        let s = &self.integrations;
        let base = st.config.base_url();
        let body = |t: &str, color| txt(t.to_string(), 13.0).line_height(Pixels(16.0)).color(color);
        let actions =
            |buttons: Vec<El<'a>>| -> El<'a> { Row::with_children(buttons).spacing(4).padding([14, 13]).into() };
        let copy = |i: u8, value: String| {
            let icon = if s.copied == Some(i) { &COPIED_ICON } else { &COPY_ICON };
            button(tinted(icon, 16.0, p.text))
                .padding(4)
                .style(btn_style([CLEAR; 3], [p.text; 3], 4.0))
                .on_press(UiMessage::CopyField(i, value))
        };
        let community = match (st.resolve_chat_idx(COMMUNITY_ROOM).is_some(), s.joining) {
            (true, _) => "Open Developer Community chat",
            (false, true) => "Joining...",
            (false, false) => "Join Developer Community chat",
        };
        let api = vec![
            pref_row(
                p,
                tr("Allow connections"),
                Some(tr("Allow third-party apps you approve like Claude Desktop or Raycast to integrate with Beeper Desktop.")),
                None,
                toggle(p, st.settings.enable_pas_api, Some(UiMessage::ToggleSetting(|x| x.enable_pas_api ^= true))),
            ),
            divider(p.prefs_border, 0.0),
            actions(vec![
                pill_btn(p, tr("Learn more"), UiMessage::OpenUrl("https://developers.beeper.com/desktop-api/".into())),
                pill_btn(p, tr(community), UiMessage::JoinCommunity),
            ]),
        ];
        let mut mcp: Vec<El<'a>> = Vec::new();
        for ((name, label), logo) in MCP_CLIENTS.into_iter().zip([&*CLAUDE_LOGO, &*CURSOR_LOGO, &*VSCODE_LOGO]) {
            let connect =
                button(txt(tr(label), 13.0).font(fw(Weight::Medium)).line_height(Pixels(16.0)).color(p.accent))
                    .padding([4, 8])
                    .style(btn_style([CLEAR; 3], [p.accent; 3], 20.0))
                    .on_press(UiMessage::McpInstall(name));
            let line = row![svg(logo.clone()).width(18).height(18), semi(name, 14.0).color(p.text)].spacing(8);
            let line = row![line.width(Fill).align_y(Center), connect]
                .align_y(Center)
                .padding(Padding::new(13.0).bottom(12.0));
            mcp.extend([line.into(), divider(p.prefs_border, 0.0)]);
        }
        let target = MCP_TARGETS[s.target];
        let value = mcp_value(target, &base);
        let pick = select(p, MCP_TARGETS, target, |t| {
            UiMessage::McpTarget(MCP_TARGETS.iter().position(|x| *x == t).expect("a listed client"))
        });
        mcp.push(
            row![container(body(tr("Install to..."), p.text)).width(Fill), pick].align_y(Center).padding(13).into(),
        );
        let blue = if self.dark { Color::from_rgb8(10, 132, 255) } else { Color::from_rgb8(0, 122, 255) };
        let install = matches!(target, "Claude Code" | "Codex").then(|| {
            let label = semi(tr("Copy Command"), 11.0).line_height(Pixels(12.0)).color(Color::WHITE);
            container(
                button(label)
                    .padding([4, 8])
                    .style(btn_style([blue; 3], [Color::WHITE; 3], 4.0))
                    .on_press(UiMessage::McpInstall(target)),
            )
            .padding(Padding::ZERO.bottom(8.0))
        });
        let hint = match target {
            "stdio" => Some("Prefer Streamable HTTP when possible."),
            "Claude Code" | "Codex" => Some("Run this command in your terminal:"),
            _ => None,
        };
        let code = txt(value.clone(), 12.0)
            .font(iced::Font::MONOSPACE)
            .line_height(Pixels(16.0))
            .color(p.text)
            .wrapping(iced::widget::text::Wrapping::Glyph);
        let code = stack![
            container(code).width(Fill).padding(Padding::new(12.0).right(40.0)).style(fill_box(p.chip, 6.0)),
            container(container(copy(0, value)).padding(8).style(fill_box(p.chip.scale_alpha(0.8), 6.0)))
                .align_right(Fill)
        ];
        let hint = hint.map(|h| {
            container(txt(tr(h), 11.0).line_height(Pixels(14.0)).color(p.weak)).padding(Padding::ZERO.bottom(4.0))
        });
        let details = column![].push(install).push(hint).push(container(code).padding([8, 0]));
        let details = details.padding(Padding::new(13.0).top(0.0).bottom(14.0));
        mcp.push(details.into());
        mcp.push(actions(vec![pill_btn(
            p,
            tr("See docs for MCP"),
            UiMessage::OpenUrl("https://developers.beeper.com/desktop-api/mcp".into()),
        )]));
        let endpoint = |i: u8, label: &'static str, url: String| -> El<'a> {
            let field = container(ell(&url, 200.0, 13.0, Weight::Normal).color(p.text))
                .width(212)
                .padding([4, 6])
                .style(bordered_box(p.input, p.prefs_border, 1.0, 4.0));
            let field = row![field, copy(i, url)].spacing(4).align_y(Center);
            row![container(body(tr(label), p.text)).width(Fill), field].spacing(12).align_y(Center).padding(13).into()
        };
        let mut tokens: Vec<_> = st.connect.tokens.values().collect();
        tokens.sort_by_key(|t| t.created_at);
        let now = now_unix_millis();
        let connections: Vec<El<'a>> = match tokens.is_empty() {
            true => {
                vec![container(body(tr("Add a connection to get started"), p.sub)).center_x(Fill).padding(12).into()]
            }
            false => tokens
                .into_iter()
                .flat_map(|t| {
                    let c = t.client_info.clone().unwrap_or_default();
                    let name = c.name.filter(|n| !n.is_empty()).or(c.client_id);
                    let mut name = name.unwrap_or_else(|| tr("Unnamed Client").into());
                    name.extend(c.version.map(|v| format!(" v{v}")));
                    let tag = t.scopes.iter().any(|s| s == beeper_core::connect::WRITE).then(|| {
                        container(txt(tr("Sensitive actions"), 12.0).line_height(Pixels(14.0)).color(p.text))
                            .padding([1, 4])
                            .style(fill_box(p.chip, 4.0))
                    });
                    let created = distance(now - t.created_at);
                    let when = match t.expires_at {
                        Some(e) => trf("Created %1$s ago · Expires in %2$s", &[&created, &distance(e - now)]),
                        None => trf("Created %s ago · Never expires", &[&created]),
                    };
                    let text = column![
                        row![body(&name, p.text)].push(tag).spacing(6).align_y(Center),
                        body(&when, p.text.scale_alpha(0.7))
                    ];
                    let revoke = container(pill_btn(p, tr("Revoke"), UiMessage::RevokeConnection(t.id.clone())))
                        .padding(Padding::ZERO.right(8.0));
                    let line =
                        row![text.width(Fill), revoke].align_y(Center).padding(Padding::new(8.0).left(13.0).right(4.0));
                    [divider(p.prefs_border, 0.0), line.into()]
                })
                .skip(1)
                .collect(),
        };
        let plus = button(tinted(&ADD_ICON, 20.0, p.accent))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.accent; 3], 10.0))
            .on_press(UiMessage::AddToken(true));
        let approved = column![
            row![container(body(tr("Approved connections"), p.muted)).width(Fill), plus]
                .align_y(Center)
                .padding([0, 13]),
            group(p, None, connections)
        ]
        .spacing(6);
        let on = st.settings.remote_access;
        let fixed = st.config.launch_host && !on;
        let remote = pref_row(
            p,
            tr("Remote Access"),
            Some(tr("When enabled, the Desktop API & MCP will be available on the public internet. Changes take effect after relaunch.")),
            None,
            toggle(p, on, (!fixed).then_some(UiMessage::RemoteAccess(!on))),
        );
        let remote: El<'a> = match fixed {
            true => {
                let why = trf("Hostname (%s) is set by the --pas-hostname launch option", &[&st.config.host]);
                self.tip(remote, why, vec![], tooltip::Position::Top).into()
            }
            false => remote,
        };
        let advanced = vec![
            endpoint(1, "OpenAPI spec endpoint", format!("{base}/v1/spec")),
            divider(p.prefs_border, 0.0),
            endpoint(2, "MCP endpoint", format!("{base}/v0/mcp")),
            divider(p.prefs_border, 0.0),
            remote,
        ];
        let head = row![container(body(tr("Beeper Desktop API"), p.muted)).width(Fill), feedback_btn(p)];
        let api = column![head.align_y(Center).padding([0, 13]), group(p, None, api)].spacing(6);
        vec![api.into(), group(p, Some("MCP"), mcp), approved.into(), group(p, Some(tr("Advanced")), advanced)]
    }

    fn storage_page<'a>(&'a self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let s = &self.storage;
        let body = |t: String, color| txt(t, 13.0).line_height(Pixels(16.0)).color(color);
        let h4 = |t| container(body(tr(t).into(), p.muted)).padding([5, 0]);
        let Some(all) = s.all.as_deref() else {
            let loading = container(txt(tr("Loading..."), 14.0).color(p.muted)).center_x(Fill).padding(40);
            return column![h4("Storage"), loading].into();
        };
        let totals = storage_totals(all);
        let legend = totals.iter().map(|&(k, bytes)| {
            let (label, color) = kind_meta(k);
            let swatch = container(Space::new()).width(10).height(10).style(fill_box(color, 2.0));
            let size = semi(human_size(bytes), 13.0).line_height(Pixels(16.0)).color(p.text);
            row![swatch, column![body(tr(label).into(), p.text), Space::new().width(80)], size]
                .spacing(8)
                .align_y(Center)
                .into()
        });
        let total = human_size(totals.iter().map(|t| t.1).sum());
        let fg = if self.dark { Color::WHITE } else { Color::BLACK };
        let ring = stack![
            svg(s.chart.clone().expect("a chart for the scan")).width(200).height(200),
            container(txt(total, 18.0).color(fg)).center(200)
        ];
        let legend = (!totals.is_empty()).then(|| Column::with_children(legend).spacing(8));
        let chart = container(row![ring].push(legend).spacing(24).align_y(Center)).center_x(Fill).padding(16);
        let card =
            |inner: El<'a>| container(inner).width(Fill).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
        let link = |label: &'static str, color: Color, size: f32, on: bool, msg| {
            let color = if on { color } else { Color { a: 0.4, ..color } };
            button(txt(tr(label), size).line_height(1.4).color(color))
                .padding(0)
                .style(btn_style([CLEAR; 3], [color; 3], 0.0))
                .on_press_maybe(on.then_some(msg))
        };
        let warning = || {
            let warn =
                "⚠️ This will delete local data. You can recover content by logging out and back into your accounts.";
            container(txt(tr(warn), 12.0).line_height(1.4).color(p.muted).width(Fill).align_x(Center)).padding([0, 16])
        };
        let busy = matches!(s.clearing, Some(0 | 1));
        let label = |which| if s.clearing == Some(which) { "Clearing..." } else { "Clear Storage..." };
        let clear = column![
            body(tr("Clear storage older than").into(), p.muted),
            self.storage_ages(0),
            self.storage_kinds(0, busy),
            warning(),
            link(label(0), p.danger, 13.0, !busy && s.kinds[0] != 0, UiMessage::Ask(Some(ASK_CLEAR_STORAGE)))
        ]
        .spacing(12)
        .align_x(Center)
        .padding(16);
        let cached = totals.iter().any(|t| t.0 == storage::Kind::Cache);
        let cache = row![
            column![
                body(tr("Clear All Cache").into(), p.text),
                txt(tr("Clears unused previews, old drafts, and temporary data to free up space. Your messages are safe."), 13.0)
                    .line_height(1.4)
                    .color(p.muted)
            ]
            .spacing(2)
            .width(Fill),
            link("Clear", p.accent, 14.0, !busy && cached, UiMessage::Ask(Some(ASK_CLEAR_CACHE)))
        ]
        .align_y(Center)
        .padding([12, 16]);
        let mut rooms: Vec<(&str, [u64; 3])> = Vec::new();
        for e in all.iter().filter(|e| e.kind != storage::Kind::Cache) {
            let chat = e.chat.as_deref().expect("chat storage has a chat");
            let i = rooms.iter().position(|r| r.0 == chat).unwrap_or_else(|| {
                rooms.push((chat, [0; 3]));
                rooms.len() - 1
            });
            let r = &mut rooms[i].1;
            r[usize::from(e.kind == storage::Kind::Message)] += 1;
            r[2] += e.bytes;
        }
        rooms.sort_by_key(|r| std::cmp::Reverse(r.1[2]));
        let rooms = rooms.into_iter().filter(|r| r.1[2] > 0).take(10).enumerate().flat_map(
            |(i, (id, [files, msgs, bytes]))| {
                let chat = st.resolve_chat_idx(id).map(|i| &st.chats[i]);
                let title = chat.map_or(id, |c| c.title.as_str());
                let counts = [
                    (files > 0).then(|| trn("%s File", "%s Files", files as usize, &[&grouped(files)])),
                    (msgs > 0).then(|| trn("%s Message", "%s Messages", msgs as usize, &[&compact(msgs)])),
                ];
                let counts: Vec<String> = counts.into_iter().flatten().collect();
                let open = s.chat.as_deref() == Some(id);
                let chevron = svg(SELECT_ARROW.clone())
                    .width(12)
                    .height(12)
                    .rotation(iced::Radians(if open { std::f32::consts::PI } else { 0.0 }))
                    .style(move |_, _| svg::Style { color: Some(p.muted) });
                let stats = row![
                    txt(counts.join(" | "), 13.0).color(p.muted),
                    semi(human_size(bytes), 13.0)
                        .line_height(Pixels(16.0))
                        .color(p.text)
                        .width(80)
                        .align_x(iced::alignment::Horizontal::Right)
                ]
                .spacing(16)
                .align_y(Center);
                let line = row![]
                    .push(chat.map(|c| self.chat_avatar(c, 32.0, 0.0)))
                    .push(container(ell(title, 200.0, 13.0, Weight::Normal).color(p.text)).width(Fill))
                    .push(stats)
                    .push(chevron)
                    .spacing(12)
                    .align_y(Center)
                    .padding([12, 16]);
                let line = button(line)
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                    .on_press(UiMessage::StorageChat(id.into()));
                let panel = open.then(|| {
                    let busy = s.clearing == Some(2);
                    let panel = column![
                        body(tr("Clear storage older than").into(), p.muted),
                        self.storage_ages(1),
                        self.storage_kinds(1, busy)
                    ]
                    .push((s.kinds[1] & 1 << 4 != 0).then(warning))
                    .push(link(
                        label(2),
                        p.danger,
                        13.0,
                        !busy && s.kinds[1] != 0,
                        UiMessage::Ask(Some(ASK_CLEAR_CHAT)),
                    ))
                    .spacing(12)
                    .align_x(Center)
                    .padding(Padding::new(16.0).top(12.0));
                    column![divider(p.prefs_border, 0.0), panel].into()
                });
                [(i > 0).then(|| divider(p.prefs_border, 0.0)), Some(line.into()), panel].into_iter().flatten()
            },
        );
        let rooms: Vec<El<'a>> = rooms.collect();
        let by_chat = (!rooms.is_empty()).then(|| {
            column![h4("Storage by Chat"), card(Column::with_children(rooms).into())]
                .spacing(6)
                .padding(Padding::ZERO.top(32.0))
        });
        column![h4("Storage"), chart, card(clear.into()), Space::new().height(16), card(cache.into())]
            .push(by_chat)
            .into()
    }

    /// Desktop's `BrandSegmentControl` of the ages, for all chats (0) or for the open chat (1).
    fn storage_ages<'a>(&self, which: usize) -> El<'a> {
        let days = STORAGE_DAYS.iter().map(|&d| trn("%d day", "%d days", d as usize, &[&d]));
        let control = self
            .segments(days, self.storage.ages[which], |i| UiMessage::StorageAge(which, i))
            .padding(if which == 0 { 8 } else { 4 });
        if which == 0 {
            control.into()
        } else {
            control.max_width(320).into()
        }
    }

    /// Desktop's type checkboxes, the media types for all chats (0) and also Messages for the open chat (1).
    fn storage_kinds<'a>(&self, which: usize, busy: bool) -> El<'a> {
        let p = &self.pal;
        let kinds = storage::Kind::ALL[..4 + which].iter().enumerate().map(|(i, &k)| {
            let on = self.storage.kinds[which] & 1 << i != 0;
            let label = txt(tr(kind_meta(k).0), 13.0).line_height(Pixels(16.0)).color(p.text);
            button(row![checkbox(p, on), label].spacing(6).align_y(Center))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                .on_press_maybe((!busy).then_some(UiMessage::StorageKind(which, i)))
                .into()
        });
        Row::with_children(kinds).spacing(16).wrap().vertical_spacing(16).align_x(Center).into()
    }

    fn hotkeys_page(&self, st: &BeeperStore) -> El<'_> {
        let s = &st.settings;
        let p = &self.pal;
        let edge = p.neutral_border;
        let search = text_input(tr("Search hotkeys..."), &self.hotkey_query)
            .on_input(UiMessage::HotkeyQuery)
            .size(13)
            .line_height(Pixels(15.0))
            .padding([4, 7])
            .width(219)
            .style(outline_input(p));
        let revert = button(txt(tr("Revert all"), 13.0).line_height(Pixels(15.0)).width(Fill).align_x(Center))
            .width(100)
            .padding([4, 6])
            .style(pill_style(p))
            .on_press(UiMessage::RevertHotkeys);
        let icon_btn = |glyph: El<'static>, msg| {
            button(glyph).padding(0).style(btn_style([CLEAR; 3], [p.muted; 3], 0.0)).on_press(msg)
        };
        let hotkey = |name: &'static str| -> El<'_> {
            let keys = s.hotkey_keys(name);
            let rec = self.hotkey_rec.as_ref().filter(|r| r.name == name);
            let conflict = rec.and_then(|r| r.conflict(s));
            let shown = rec.map_or_else(|| keys.to_string(), HotkeyRec::combo);
            let recorder = (keys != "disabled").then(|| {
                let ink = match (conflict, rec) {
                    (Some(_), _) => p.danger,
                    (None, Some(_)) => p.muted,
                    (None, None) => p.text,
                };
                let label: El<'_> = match prefs::hotkey_label(&shown) {
                    l if l.is_empty() => txt(tr("Record Shortcut"), 11.0).color(p.muted).into(),
                    l => txt(l, 13.0).line_height(Pixels(18.0)).color(ink).into(),
                };
                let buttons = match rec {
                    Some(_) => row![]
                        .push(conflict.is_none().then(|| {
                            icon_btn(ic(icon::CHECK, 12.0).color(p.muted).into(), UiMessage::HotkeyRecorder(true))
                        }))
                        .push(icon_btn(ic(icon::CLOSE, 12.0).color(p.muted).into(), UiMessage::HotkeyRecorder(false))),
                    None => row![].push(
                        (keys != prefs::hotkey_default(name))
                            .then(|| icon_btn(tinted(&REVERT, 12.0, p.muted), UiMessage::RevertHotkey(name))),
                    ),
                };
                let border = match (conflict, rec) {
                    (Some(_), _) => p.danger,
                    (None, Some(_)) => p.accent,
                    (None, None) => edge,
                };
                let label =
                    mouse_area(container(label).center_x(160).center_y(20)).on_press(UiMessage::RecordHotkey(name));
                let buttons =
                    container(buttons.spacing(6).align_y(Center)).align_right(160).center_y(20).padding([0, 2]);
                container(stack![label, buttons]).style(bordered_box(CLEAR, border, 1.0, 4.0))
            });
            let line = row![ell(tr(name), 226.0, 13.0, Weight::Normal).color(p.text).width(Fill)]
                .push(recorder)
                .push(toggle(p, keys != "disabled", Some(UiMessage::ToggleHotkey(name))))
                .spacing(6)
                .align_y(Center);
            let warning = conflict.map(|other| {
                let text = trf("%1$s is already used by %2$s", &[&prefs::hotkey_label(&shown), &tr(other)]);
                container(txt(text, 11.7).color(p.text.scale_alpha(0.8))).align_right(Fill).padding([4, 0])
            });
            column![line].push(warning).padding([8, 0]).into()
        };
        let listed = prefs::HOTKEYS.iter().filter(|h| hotkey_shown(st, h.0));
        let shown = prefs::match_sort(listed, &self.hotkey_query, |h| tr(h.0).into());
        let groups = first_groups(&shown, |h| h.1).into_iter().map(|group| {
            let rows = shown.iter().filter(|h| h.1 == group).map(|h| hotkey(h.0));
            let head = txt(tr(group), 13.0).line_height(Pixels(16.0)).color(p.muted);
            column![container(head).padding(Padding::ZERO.top(24.0).bottom(12.0))].extend(rows).into()
        });
        let head = row![search, Space::new().width(Fill), revert].align_y(Center).padding(Padding::ZERO.bottom(10.0));
        let line = container(Space::new()).width(Fill).height(1).style(fill_box(edge, 0.0));
        let card = column![head, line, Space::new().height(6)].extend(groups);
        container(card).width(Fill).padding(14).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0)).into()
    }

    /// Desktop's Keyboard Shortcuts list: the first keys of each hotkey, at the right of the window.
    pub(crate) fn view_hotkeys_modal(&self, query: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let modifier_only = |k: &str| k.rsplit('+').next().is_some_and(|m| ["ctrl", "shift", "alt"].contains(&m));
        let all = prefs::HOTKEYS
            .iter()
            .filter(|h| !modifier_only(h.2) && hotkey_shown(&st, h.0))
            .map(|h| (h.0, h.1, st.settings.hotkey_keys(h.0)));
        let all = all.chain(prefs::FIXED_HOTKEYS.iter().copied()).map(|(n, g, k)| (n, g, k.to_string()));
        let shown = prefs::match_sort(all, query, |h| tr(h.0).into());
        drop(st);
        let row = |(name, _, keys): &(&'static str, &str, String)| -> El<'_> {
            let label = prefs::hotkey_label(keys.split(", ").next().expect("a combination"));
            let caps: Vec<String> = match label.is_empty() {
                true => vec![tr("Unset").into()],
                false => label.split(' ').map(String::from).collect(),
            };
            let width = |k: &str| (text_width(k, 13.0, Weight::Normal) + 8.0).max(22.0);
            let caps_w: f32 = caps.iter().map(|k| width(k) + 4.0).sum();
            let caps = caps.into_iter().map(|k| {
                let w = width(&k);
                container(txt(k, 13.0).color(p.text))
                    .center_x(w)
                    .center_y(22)
                    .style(fill_box(p.fg.scale_alpha(0.15), 4.0))
            });
            let caps = Row::with_children(caps.map(El::from)).spacing(4).padding(Padding::ZERO.right(4.0));
            let name = ell(tr(name), 325.0 - 22.4 - 12.0 - caps_w, 13.0, Weight::Normal).color(p.text);
            let line = row![name, Space::new().width(Fill), caps].spacing(12).align_y(Center);
            let hover = p.fg.scale_alpha(0.05);
            button(line)
                .width(325)
                .padding([4.8, 11.2])
                .style(btn_style([CLEAR, hover, hover], [p.text; 3], 8.0))
                .on_press(UiMessage::Noop)
                .into()
        };
        let groups = first_groups(&shown, |h| h.1).into_iter().map(|group| {
            let head = container(semi(tr(group), 13.0).color(p.text)).padding([4.8, 11.2]).width(325);
            let rows = shown.iter().filter(|h| h.1 == group).map(row);
            column![head].extend(rows).padding(Padding::ZERO.top(12.8)).into()
        });
        let empty = shown.is_empty().then(|| {
            container(txt(tr("No results"), 16.0).color(p.fg.scale_alpha(0.35)))
                .center_x(Fill)
                .height(Fill)
                .center_y(Fill)
        });
        let list = Column::with_children(groups).push(empty).width(Fill).align_x(Center);
        let height = 0.8 * self.pane().size.height / self.zoom;
        let list = scrollable(list)
            .direction(bar())
            .style(scroll_style(p.scroll))
            .height(height)
            .on_scroll(|v| UiMessage::HotkeysScrolled(v.absolute_offset().y > 20.0));
        let search = text_input(tr("Search..."), query)
            .id("hotkeys-search")
            .on_input(|q| UiMessage::HotkeysModal(Some(q)))
            .size(13)
            .line_height(Pixels(18.2))
            .padding([3, 6])
            .style(outline_input(p));
        let rule = p.fg.scale_alpha(if self.hotkeys_scrolled { 0.1 } else { 0.0 });
        let card = column![
            container(bold(tr("Keyboard Shortcuts"), 24.0).color(p.text)).padding([8.0, 24.0]),
            container(search).padding(Padding { top: 4.0, right: 22.4, bottom: 10.0, left: 22.4 }),
            container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0)),
            list,
        ]
        .width(350)
        .padding([13, 0]);
        let card = mouse_area(container(card).style(fill_box(p.elevated, 0.0))).on_press(UiMessage::Noop);
        let shade = container(card).align_right(Fill).center_y(Fill).padding(Padding::ZERO.right(13.0));
        mouse_area(shade.style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.4), 0.0)))
            .on_press(UiMessage::HotkeysModal(None))
            .into()
    }

    /// Desktop's `AccountPrefsSubView`: the account, and its notification settings.
    fn account_page(&self, st: &BeeperStore, id: &str) -> Vec<El<'_>> {
        let p = &self.pal;
        let bridge = (id != "hungryserv")
            .then(|| st.bridge_accounts.iter().find(|b| b.account_id == id).expect("the account of the page"));
        let (network, [name, username, phone, email]) = match bridge {
            None => {
                let me = st.me();
                ("Beeper".into(), [&me.full_name, &me.username, &me.phone_number, &me.email].map(Clone::clone))
            }
            Some(b) => {
                let r = b.remote_profile.clone().unwrap_or_default();
                (b.network_name.clone(), [r.name, r.username, r.phone, r.email])
            }
        };
        let back = button(tinted(&CHEVRON_LEFT, 16.0, p.strong))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
            .on_press(UiMessage::OpenAccount(None));
        let title = row![svg(brand_svg(brand_key(id))).width(28).height(28), semi(network, 14.0).color(p.strong)]
            .spacing(4)
            .align_y(Center);
        let head = row![back, container(title).center_x(Fill), Space::new().width(16)].align_y(Center);
        let lines = [username.map(|u| format!("@{u}")), phone.map(|n| fmt_phone(&n)), email].into_iter().flatten();
        let user = column![]
            .push(name.as_deref().map(|n| container(avatar(n, 28.0, 0.0, p)).padding(Padding::ZERO.bottom(8.0))))
            .push(name.map(|n| semi(n, 14.0).color(p.strong)))
            .extend(lines.map(|l| txt(l, 13.0).color(p.weak).into()))
            .align_x(Center);
        let card =
            container(user).center_x(Fill).padding(8).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
        let props = st.settings.account(id);
        let set = |f: fn(&mut beeper_core::models::AccountProps)| {
            let mut q = props;
            f(&mut q);
            UiMessage::SetAccountProps(id.into(), q)
        };
        let toggle = |label, on, msg| pref_row(p, tr(label), None, None, toggle_faded(p, on, Some(msg), 1.0));
        let sounds = [tr("Default").to_string(), tr("None").to_string()];
        let (account, picked) = (id.to_string(), sounds[usize::from(props.silent)].clone());
        let sound = select(p, sounds.clone(), picked, move |o| {
            UiMessage::SetAccountProps(
                account.clone(),
                beeper_core::models::AccountProps { silent: o == sounds[1], ..props },
            )
        });
        let inbox = st.matrix.as_ref().and(bridge).map(|b| {
            let labels: Vec<String> = INBOX_OPTIONS.iter().map(|&(_, l)| inbox_label(l).to_string()).collect();
            let (key, now) = beeper_core::matrix::inbox_rule(&st.inbox_rules, id, &b.bridge_id, None);
            let at = INBOX_OPTIONS.iter().position(|(o, _)| *o == now).expect("a known inbox visibility");
            let picked = labels[at].clone();
            let pick = select(p, labels.clone(), picked, move |l| {
                let i = labels.iter().position(|x| *x == l).expect("a listed option");
                UiMessage::SetInboxVisibility(key.clone(), INBOX_OPTIONS[i].0)
            });
            pref_row(p, tr("Show in Inbox"), None, None, pick)
        });
        let rows = inbox.into_iter().chain([
            toggle("Enable notifications", props.notifications, set(|q| q.notifications ^= true)),
            toggle("Show message previews", props.previews, set(|q| q.previews ^= true)),
            pref_row(p, tr("Sound"), None, None, sound),
        ]);
        // Desktop's `AccountDisconnected` banner, with Reconnect when the bridge asks for a new login.
        let again = reconnect(st, id);
        let disconnected = bridge.and_then(|b| b.remote_error.clone()).map(|message| {
            let again = again.map(|m| {
                button(med(tr("Reconnect"), 11.0))
                    .padding([4, 8])
                    .style(btn_style([Color::WHITE; 3], [p.accent; 3], 20.0))
                    .on_press(m)
            });
            let banner = column![
                svg(ERROR_CIRCLE.clone()).width(20).height(20),
                semi(tr("Account Disconnected"), 13.0).color(p.strong),
                txt(message, 11.0).color(p.strong).center(),
            ]
            .push(again)
            .spacing(8)
            .align_x(Center);
            let fill = if self.dark { p.prefs_group } else { p.chip };
            container(banner).width(Fill).padding([12, 8]).style(fill_box(fill, 8.0)).into()
        });
        // Desktop's `AccountDeleteButton`.
        let remove = bridge.map(|_| {
            let label = semi(tr("Remove Account"), 13.0).color(p.danger).width(Fill).center();
            button(label)
                .padding([5, 0])
                .width(Fill)
                .style(btn_style([CLEAR; 3], [p.danger; 3], 0.0))
                .on_press(UiMessage::AskRemoveAccount(id.into()))
                .into()
        });
        let settings = group(p, Some(tr("Settings")), rows.collect());
        [Some(head.into()), disconnected, Some(card.into()), Some(settings), remove].into_iter().flatten().collect()
    }

    /// A settings item with the id that search results scroll to, and the flash of a picked result.
    fn flashing<'a>(&self, key: &'static str, el: El<'a>) -> El<'a> {
        let flash = self
            .pref_flash
            .filter(|(k, _)| *k == key)
            .map_or(0.0, |(_, at)| 1.0 - ease_out((at.elapsed().as_secs_f32() / 2.0).min(1.0)));
        container(el).id(key).style(fill_box(self.pal.accent.scale_alpha(flash), 8.0)).into()
    }

    fn pref_items(&self, items: &'static [prefs::Item], st: &BeeperStore) -> Vec<El<'_>> {
        let s = &st.settings;
        let p = &self.pal;
        let mut heading = None;
        let mut lead = None;
        let mut out = Vec::new();
        for item in items {
            let custom = item.custom().map(|(name, _)| name);
            match *item {
                prefs::Item::Heading(h) => heading = Some(tr(h)),
                prefs::Item::Subtitle(t, args) => match (heading.take(), trs(t, args)) {
                    (Some(h), t) => {
                        out.push(group(p, Some(h), vec![container(txt(t, 13.0).color(p.sub)).padding([13, 14]).into()]))
                    }
                    (None, t) => lead = Some(t),
                },
                prefs::Item::Group(rows) => {
                    let row = |r: &'static prefs::Pref| self.flashing(r.key, self.pref_control(r, st));
                    let rows: Vec<El> = rows.iter().filter(|r| shown(st, r)).map(row).collect();
                    if rows.is_empty() {
                        heading = None;
                        continue;
                    }
                    let g = group(p, heading.take(), rows);
                    out.push(match lead.take() {
                        Some(t) => {
                            let t = txt(t, 13.0).line_height(Pixels(18.0)).color(p.sub);
                            column![container(t).padding(Padding::new(13.0).top(7.0).bottom(0.0)), g].spacing(19).into()
                        }
                        None => g,
                    })
                }
                prefs::Item::Theme => out.push(group(p, heading.take(), vec![theme_cards(p, s.theme)])),
                prefs::Item::ChatTheme => out.push(group(p, heading.take(), vec![self.chat_theme_picker(st, None)])),
                prefs::Item::Zoom(t, args) => {
                    out.push(group(p, heading.take(), vec![self.zoom_slider(s.zoom_percent, trs(t, args))]))
                }
            }
            if let Some(name) = custom {
                let el = out.pop().expect("a custom item is a group");
                out.push(self.flashing(name, el));
            }
        }
        assert!(lead.is_none(), "a subtitle without a heading comes before a group");
        out
    }

    fn zoom_slider(&self, percent: u16, desc: String) -> El<'_> {
        let p = &self.pal;
        let level = self.zoom_drag.unwrap_or(((f32::from(percent) - 100.0) / 10.0).clamp(-4.0, 4.0));
        let (fill, edge) = (p.accent, p.prefs_group);
        let dot = || {
            container(Space::new()).width(8).height(8).style(move |_: &Theme| container::Style {
                background: Some(fill.into()),
                border: iced::Border { color: edge, width: 2.0, radius: 12.0.into() },
                ..Default::default()
            })
        };
        let ticks =
            (0..9).fold(Row::new(), |r, i| if i == 0 { r } else { r.push(Space::new().width(Fill)) }.push(dot()));
        let rail = container(Space::new()).width(Fill).height(4).style(fill_box(fill, 8.0));
        let knob = container(Space::new()).width(20).height(20).style(|_: &Theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: iced::Border { radius: 20.0.into(), ..Default::default() },
            shadow: iced::Shadow {
                color: Color::BLACK.scale_alpha(0.2),
                offset: iced::Vector::new(1.0, 1.0),
                blur_radius: 2.0,
            },
            ..Default::default()
        });
        let at = (level + 4.0) as u16;
        let knob =
            row![Space::new().width(Length::FillPortion(at)), knob, Space::new().width(Length::FillPortion(8 - at))];
        let input = slider(-4.0..=4.0, level, UiMessage::ZoomDrag).on_release(UiMessage::ZoomRelease).height(20).style(
            |_, _| slider::Style {
                rail: slider::Rail {
                    backgrounds: (CLEAR.into(), CLEAR.into()),
                    width: 0.0,
                    border: iced::Border::default(),
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle { radius: 10.0 },
                    background: CLEAR.into(),
                    border_width: 0.0,
                    border_color: CLEAR,
                },
            },
        );
        let track = stack![container(rail).center_y(20), container(ticks).center_y(20), knob, input]
            .width(Length::FillPortion(34));
        let icon = |size, pad: Padding| {
            container(tinted(&ZOOM_LEVEL, size, p.weak)).padding(pad).width(Length::FillPortion(3))
        };
        let (first, last) = (
            icon(14.0, Padding::ZERO.left(8.0)),
            icon(20.0, Padding::ZERO.right(4.0)).align_x(iced::alignment::Horizontal::Right),
        );
        column![
            row![first, track, last].align_y(Center).padding(16),
            container(txt(desc, 13.0).color(p.sub)).padding(Padding::new(14.0).top(0.0).bottom(13.0))
        ]
        .into()
    }

    fn pref_select(&self, r: &'static prefs::Pref, s: &BeeperSettings) -> El<'_> {
        let prefs::Control::Select(options, _) = r.control else { panic!("{} is not a select", r.key) };
        let key = r.key;
        let shown: Vec<String> = options
            .iter()
            .map(|o| match (key, o.split_once(' ')) {
                ("appLanguage", _) => o.to_string(),
                ("chatActionButton", _) => trx(o, "primary thread action option").into(),
                ("chatDoubleClick", _) => trx(o, "thread double-click action option").into(),
                (_, Some((n, unit))) if n.parse::<usize>().is_ok() => duration(n.parse().unwrap(), unit),
                _ => tr(o).into(),
            })
            .collect();
        let picked = shown[s.pref(key) as usize].clone();
        select(&self.pal, shown.clone(), picked, move |o| {
            UiMessage::SetPref(key, shown.iter().position(|x| *x == o).expect("listed option") as u8)
        })
    }

    fn pref_control(&self, r: &'static prefs::Pref, st: &BeeperStore) -> El<'_> {
        let s = &st.settings;
        let p = &self.pal;
        let locked = r.feature.is_some_and(|f| st.feature(f).locked());
        // Desktop's `onFeatureInteraction` of a locked row opens the Beeper Plus subscription prompt.
        let upsell = UiMessage::SelectSettingsTab(SettingsTab::BeeperPlus);
        let mut press = None;
        let control = match r.control {
            prefs::Control::Toggle(_) => {
                let v = s.pref(r.key);
                let off = r.requires.is_some_and(|k| !s.on(k)) || locked;
                press = match locked {
                    true => Some(upsell.clone()),
                    false => (!off).then_some(UiMessage::SetPref(r.key, 1 - v)),
                };
                toggle_faded(p, v == 1 && !locked, press.clone(), if off { 0.5 } else { 1.0 })
            }
            prefs::Control::ReactionEmoji { .. } => {
                press = Some(UiMessage::QuickEmoji(Some(0)));
                emoji_button(p, &s.quick_reaction, 0)
            }
            prefs::Control::QuickReactions(customized) => {
                let custom = s.on(customized.key);
                let choice = |label: &'static str, on: bool, value: u8| {
                    let dot = container(
                        container(Space::new()).center(6).style(fill_box(if on { Color::WHITE } else { CLEAR }, 3.0)),
                    )
                    .center(14)
                    .style(bordered_box(
                        if on { p.accent } else { CLEAR },
                        if on { p.accent } else { p.sub },
                        1.0,
                        7.0,
                    ));
                    let label = txt(label, 13.0).color(p.text);
                    button(row![dot, label].spacing(8).align_y(Center))
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                        .on_press(UiMessage::SetPref(customized.key, value))
                };
                let emojis = s.quick_reactions.iter().zip(1..).map(|(e, slot)| emoji_button(p, e, slot));
                let own = row![choice(trx("Custom", "quick reactions option"), custom, 1)].extend(emojis);
                return column![choice(tr("Use my most frequent emoji"), !custom, 0), own.spacing(8).align_y(Center)]
                    .spacing(8)
                    .padding(Padding::new(13.0).top(0.0))
                    .into();
            }
            prefs::Control::Downloads => {
                let change = button(med(tr("Change"), 12.0).color(p.text))
                    .padding([4, 10])
                    .style(pill_style(p))
                    .on_press(UiMessage::PickDownloads);
                let dir = downloads_dir(s).display().to_string();
                return pref_row(p, tr(r.label), Some(&dir), None, change.into());
            }
            prefs::Control::BlockedUsers => {
                press = Some(UiMessage::OpenBlockedUsers);
                let n = txt(blocked_chats(st).len().to_string(), 13.0).color(p.muted);
                row![n, ic(icon::CHEVRON_RIGHT, 16.0).color(p.muted)].spacing(2).align_y(Center).into()
            }
            prefs::Control::Analytics => {
                press = Some(UiMessage::ShareAnalytics);
                toggle_faded(p, !st.whoami.analytics_disabled, press.clone(), 1.0)
            }
            prefs::Control::Select(..) if locked => {
                let cover = mouse_area(Space::new().width(Fill).height(Fill)).on_press(upsell);
                stack![self.pref_select(r, s), iced::widget::opaque(cover)].into()
            }
            prefs::Control::Select(..) => self.pref_select(r, s),
            prefs::Control::Multi(labels) => {
                let shown = labels.join(", ");
                let label = ell(&shown, 92.0, 11.0, Weight::Semibold).color(p.strong);
                let arrow = tinted(&SELECT_ARROW, 10.0, p.strong);
                let pill = row![container(label).width(Fill), arrow].align_y(Center).padding([0, 8]);
                container(pill).width(140).center_y(24).style(fill_box(p.chip, 12.0)).into()
            }
            prefs::Control::Microphone => {
                let (system, names) = match &self.mics {
                    Some(Ok((system, names))) => (system.as_str(), names.as_slice()),
                    Some(Err(_)) | None => ("", &[][..]),
                };
                let same = match system {
                    "" => tr("Same as system").to_string(),
                    x => trf("Same as system (%s)", &[&x]),
                };
                let chosen = &s.microphone;
                let shown = match chosen.is_empty() {
                    true => same.clone(),
                    false if self.mics.is_some() && !names.contains(chosen) => {
                        tr("Selected microphone unavailable").into()
                    }
                    false => chosen.clone(),
                };
                let options: Vec<String> = std::iter::once(same.clone()).chain(names.iter().cloned()).collect();
                let pick = select(p, options, shown, move |o| {
                    UiMessage::SetMicrophone(if o == same { String::new() } else { o })
                });
                let desc = matches!(self.mics, Some(Err(_))).then_some(tr("Unable to load microphones."));
                return pref_row(p, tr(r.label), desc, None, pick);
            }
            prefs::Control::Slider(_) => {
                assert!(r.desc.is_none() && r.badge.is_none(), "{} is a plain slider", r.key);
                let key = r.key;
                // Desktop's range input from 0 to 1 in steps of 0.1.
                let bar = slider(0..=100u8, s.pref(key), move |x| UiMessage::SetPref(key, x))
                    .step(10u8)
                    .height(16)
                    .style(slider_style(p));
                let body = column![
                    txt(tr(r.label), 13.0).line_height(Pixels(18.0)).color(p.text),
                    container(bar).padding([0, 2])
                ]
                .spacing(13)
                .padding(Padding::new(13.0).bottom(14.0));
                return column![
                    body,
                    container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0))
                ]
                .into();
            }
            prefs::Control::Archive(scope, age) => {
                let n = st.stale_inbox_chats().len();
                let (mut left, mut rest) = (Row::new(), tr("Archive __TYPE__ chats older than __DURATION__"));
                while let Some(at) = rest.find("__") {
                    let (words, tail) = rest.split_at(at);
                    let (pick, tail) = match tail.strip_prefix("__TYPE__") {
                        Some(tail) => (scope, tail),
                        None => (age, tail.strip_prefix("__DURATION__").expect("a known placeholder")),
                    };
                    let words = (!words.trim().is_empty()).then(|| txt(words.trim().to_string(), 13.0).color(p.text));
                    (left, rest) = (left.push(words).push(self.pref_select(pick, s)), tail);
                }
                let left = left
                    .push((!rest.trim().is_empty()).then(|| txt(rest.trim().to_string(), 13.0).color(p.text)))
                    .spacing(8)
                    .align_y(Center)
                    .wrap()
                    .vertical_spacing(8);
                return row![
                    container(left).width(Fill),
                    pill_btn(
                        p,
                        match n {
                            0 => tr("Archive chats").into(),
                            n => trnx("Archive %d chat", "Archive %d chats", n, "auto-archive catch-up button", &[&n]),
                        },
                        UiMessage::ArchiveStale
                    )
                ]
                .spacing(8)
                .align_y(Center)
                .padding(13)
                .into();
            }
        };
        let paid = locked.then(|| tr("Requires a paid subscription."));
        let (label, args) = r.split_args();
        let desc = r.desc.map(|d| [trs(d, args).as_str()].into_iter().chain(paid).collect::<Vec<_>>().join("\n\n"));
        let row = pref_row(p, trs(r.label, label), desc.as_deref(), r.badge.map(|b| badge(p, b)), control);
        // Desktop's `label-wrapper` toggles the checkbox on a click anywhere in the row.
        match press {
            Some(m) => mouse_area(row).on_press(m).into(),
            None => row,
        }
    }
}

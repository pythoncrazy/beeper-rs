//! The Settings window: its sections, the settings search, the hotkey recorder, and the Storage, Integrations,
//! Devices and Accounts pages.

use crate::chat_list;
use crate::hotkeys;
use crate::i18n::{tr, trf, trn, trnx, trs, trx};
use crate::prelude::*;
use crate::rail::SpaceMenu;
use crate::sortable;
use crate::style::{gutter_style, Token};
use crate::units::{compact, grouped};
use crate::windows;
use crate::{
    ago, divider, duration, first_groups, group, portal, pref_row, select, slider_style, status_text, style, svg_size, theme_cards,
    toggle_faded, ADD_ICON, APP_QR, CHANGELOG_URL, CHEVRON_LEFT, CLAUDE_LOGO, COPIED_ICON, COPY_ICON, CURSOR_LOGO, DEVICE_ICONS, LOGO,
    QR_CODE, REVERT, SELECT_ARROW, SETTINGS_SIZE, TAKE_BEEPER, VSCODE_LOGO, ZOOM_LEVEL,
};
use beeper_core::config::APP_VERSION;
use beeper_core::models::*;
use beeper_core::prefs;
use beeper_core::storage;
use iced::keyboard;
use iced::widget::{column, operation};
use iced::window;

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

    /// Desktop's `.account-pref-pane` gives the Profile, Accounts, Devices and Storage panes
    /// `--appearance-color-bg-primary`.
    pub fn pane(self, p: &Palette) -> Color {
        match self {
            Self::Profile | Self::Accounts | Self::Devices | Self::Storage => p.card,
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
    // Desktop's `disabled` Toggle Incognito Mode: hidden unless the account can use incognito.
    let usable = name != "Toggle Incognito Mode" || st.feature("stealth-mode").on;
    usable && prefs::hotkey_feature(name).is_none_or(|f| st.feature(f).show)
}

/// The preferences of a pane of preferences.
fn pref_page(tab: SettingsTab) -> &'static [prefs::Item] {
    match tab {
        SettingsTab::General => prefs::GENERAL,
        SettingsTab::Appearance => prefs::APPEARANCE,
        SettingsTab::Notifications => prefs::NOTIFICATIONS,
        SettingsTab::Ai => prefs::AI,
        _ => &[],
    }
}

/// Desktop's `Rx`: a settings row of a feature shows when the account shows the feature.
pub(crate) fn shown(st: &BeeperStore, r: &prefs::Pref) -> bool {
    let control = match r.control {
        prefs::Control::ReactionEmoji { general: true } => st.settings.choice("messageDoubleClick") == "React with...",
        prefs::Control::QuickReactions(_) => st.settings.on("quickReactions"),
        // Desktop's `canShow: () => stickerPacksEnabled`.
        prefs::Control::StickerPacks => st.settings.on("stickerPacks"),
        _ if matches!(r.key, "autoTranscribe" | "transcriptionLanguage") => st.settings.on("transcribeWithOpenai"),
        _ => true,
    };
    r.feature.is_none_or(|f| st.feature(f).show) && control
}

/// The values that the select of `key` offers, in Desktop's order. A composer button (Desktop's `Aee`) leaves out the
/// pick of the other button unless it is None, and offers the Sticker Picker only with sticker packs on (`sFt`).
pub(crate) fn select_values(key: &str, s: &BeeperSettings) -> Vec<u8> {
    let other = match key {
        "primaryComposerButton" => "secondaryComposerButton",
        "secondaryComposerButton" => "primaryComposerButton",
        _ => return (0..=prefs::find(key).max_value()).collect(),
    };
    let taken = s.pref(other);
    let shown = |v: &u8| *v == prefs::COMPOSER_NONE || *v != taken;
    let stickers = |v: &u8| *v != prefs::COMPOSER_STICKERS || s.on("stickerPacks");
    prefs::COMPOSER_ORDER.into_iter().filter(stickers).filter(shown).collect()
}

/// A text of the settings with the values of its `%s` placeholders, where the name of a hotkey stands for its keys,
/// like Desktop's `humanized` of a reassigned hotkey.
fn pref_text(s: &BeeperSettings, text: &'static str, args: &[&str]) -> String {
    let value = |a: &&str| match prefs::HOTKEYS.iter().any(|h| h.0 == *a) {
        true => crate::hotkey_text(s.hotkey_keys(a)),
        false => a.to_string(),
    };
    let args: Vec<String> = args.iter().map(value).collect();
    trs(text, &args.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Desktop's `EmojiPickerButton` of a quick reaction in Settings, which opens the emoji picker for `slot`.
fn emoji_button<'a>(p: &Palette, glyph: El<'a>, slot: u8) -> El<'a> {
    button(glyph)
        .padding([4, 5])
        .style(bordered_btn([CLEAR; 3], [p.text; 3], p.chip, 1.0, 100.0))
        .on_press(UiMessage::QuickEmoji(Some(slot)))
        .into()
}

/// Desktop's suggestions of the empty settings search (`v` of `PrefsLeftPane`): the label, the section, and the
/// key of the row that opens highlighted, for the sections that show, then Beeper Plus for a free account that can
/// subscribe.
pub(crate) fn suggestions(st: &BeeperStore) -> Vec<(&'static str, SettingsTab, Option<&'static str>)> {
    let rows = [
        ("Add an Account", SettingsTab::Accounts, None),
        ("Incognito Mode", SettingsTab::General, Some("incognito")),
        ("Show Recipients I'm Typing", SettingsTab::General, Some("typingIndicators")),
        ("Group Message Notifications", SettingsTab::Notifications, Some("groupNotifications")),
        ("Subscribe to Beeper Plus", SettingsTab::BeeperPlus, None),
    ];
    let listed: Vec<SettingsTab> = SettingsTab::listed(st).collect();
    rows.into_iter().filter(|r| listed.contains(&r.1)).collect()
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
            SettingsTab::Accounts => crate::networks::addable(&st.features)
                .filter(|n| has(n.display_name))
                .map(|n| SettingsHit::Network(n.display_name))
                .collect(),
            _ => prefs::rows()
                .filter(|(page, r)| *page == tab.meta().0 && !r.label.is_empty() && shown(st, r))
                .filter(|(_, r)| {
                    let (label, desc) = r.split_args();
                    let name = r.name.map_or_else(|| trs(r.label, label), |n| tr(n).into());
                    // Desktop's `searchText`: the name, the description, and the keywords.
                    let desc = r.desc.map(|d| pref_text(&st.settings, d, desc));
                    let text = [Some(name), desc, r.about.map(|a| tr(a).to_string())].into_iter().flatten();
                    has(&text.chain(r.keywords.iter().map(|k| k.to_string())).collect::<Vec<_>>().join(" "))
                })
                .map(|(_, r)| SettingsHit::Pref(tab, r))
                .chain(
                    prefs::customs()
                        .filter(|&(page, name, keys)| page == tab.meta().0 && (has(tr(name)) || keys.iter().any(|k| has(k))))
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
        let shown: Vec<_> = out.iter().filter_map(|h| if let SettingsHit::Section(t, _) = h { Some(*t) } else { None }).collect();
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
    /// The [`|a0| UiMessage::Settings(Msg::ClearStorage(a0))`] that runs.
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
    svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">{arcs}</svg>"#).into_bytes())
}

/// The dialogs over the Settings window.
pub(crate) const SETTINGS_DIALOGS: [u8; 10] = [
    ASK_LOGOUT,
    ASK_LOGOUT_DEVICE,
    ASK_CLEAR_STORAGE,
    ASK_CLEAR_CACHE,
    ASK_CLEAR_CHAT,
    ASK_REVOKE_CONNECTION,
    ASK_REMOTE_ACCESS,
    ASK_REMOVE_ACCOUNT,
    ASK_DISCONNECT_SERVER,
    ASK_REMOVE_PACK,
];

/// Desktop's "Clear storage older than" choices, where 0 is any age.
pub(crate) const STORAGE_DAYS: [i64; 4] = [0, 30, 90, 120];

/// The prefixes of the `chat_menu` of an account in the Settings window: of its row in the Accounts settings, and of
/// the actions of its page.
const ROW_MENU: &str = "prefs:";
const PAGE_MENU: &str = "account-actions:";

impl BeeperGuiApp {
    pub(crate) fn open_settings(&mut self, tab: SettingsTab) -> Task<UiMessage> {
        let load = self.update(UiMessage::Settings(Msg::SelectTab(tab)));
        if let Some(id) = self.settings_window {
            return Task::batch([load, self.raise(id)]);
        }
        // Desktop's `openInNewWindow` of "prefs": in the middle of the screen.
        let centered = window::Settings { position: window::Position::Centered, ..Self::window_settings(SETTINGS_SIZE) };
        let (id, open) = self.open_window(centered);
        self.settings_window = Some(id);
        self.save_subwindows();
        Task::batch([load, open.map(|a0| UiMessage::Window(windows::Msg::Opened(a0)))])
    }

    /// Desktop's keys of the settings window: the "Search: Room" hotkey (Ctrl F unless rebound) focuses the search, ↑
    /// and ↓ move through the results, and Enter is the submit of the input. Escape in an input clears the search, and
    /// Escape elsewhere closes the window.
    pub(crate) fn settings_key(&mut self, key: Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
        if self.hotkey_rec.is_some() {
            self.record_key(&key, mods);
            return Task::none();
        }
        if self.find_hotkey(&key, mods).is_some_and(|(h, _)| h == "Search: Room") {
            return operation::focus("settings-search");
        }
        if mods.command() || mods.control() || mods.alt() {
            return self.update(UiMessage::KeyPressed(key, mods));
        }
        let window = self.settings_window.expect("the settings window is open");
        if let Some(task) = self.focus_key(window, &key, mods, self.asking(window)) {
            return task;
        }
        let n = self.search_rows();
        match key {
            Key::Named(Named::Escape) if self.confirm.is_some() => self.confirm = None,
            // Desktop's add input of the time suggestions cancels on Escape and keeps the window open.
            Key::Named(Named::Escape) if self.time_edit.input.is_some() => {
                let settings = self.settings_window.expect("the settings window is open");
                return operation::is_focused(crate::time_suggestions::INPUT).then(move |on| match on {
                    true => Task::done(UiMessage::TimeSuggestions(crate::time_suggestions::Msg::Cancel)),
                    false => window::close(settings),
                });
            }
            Key::Named(Named::Escape) => return self.close_window(self.settings_window.expect("the settings window is open")),
            Key::Named(k @ (Named::ArrowUp | Named::ArrowDown)) if n > 0 => {
                let step = if k == Named::ArrowDown { 1 } else { n - 1 };
                return operation::is_focused("settings-search").then(move |on| {
                    if on {
                        Task::done(UiMessage::Settings(Msg::Step(step)))
                    } else {
                        Task::none()
                    }
                });
            }
            _ => {}
        }
        Task::none()
    }

    /// The rows that ↑ and ↓ go through in the search: the results, or the suggestions of the focused empty search.
    fn search_rows(&self) -> usize {
        let st = self.store.read();
        match self.settings_query.trim().is_empty() {
            true if self.settings_search_on => suggestions(&st).len(),
            _ => search_settings(&st, &self.settings_query).len(),
        }
    }

    /// Desktop's `label-wrapper` row, which Tab reaches: `.label-wrapper:focus-visible` takes
    /// `rgba(var(--color-fg-rgb) / 0.1)`. Desktop's account rows are `dnd-kit` sortables, whose keys pick the row up
    /// to reorder the accounts, which beeper-rs does not do.
    fn focus_row<'a>(&self, row: El<'a>, key: String, keys: Keys) -> El<'a> {
        let look = Look::Fill(self.pal.fg.scale_alpha(0.1), 0.0);
        self.target(self.settings_window, key, row, keys, look)
    }

    /// Whether the control `key` of `window` has the keyboard focus.
    pub(crate) fn key_held(&self, window: Option<window::Id>, key: &str) -> bool {
        window.and_then(|w| self.key_targets.get(&w)) == Some(&iced::widget::Id::from(key.to_string()))
    }

    /// Whether the Accounts pane shows the page of an account rather than the list.
    fn account_page_shown(&self, st: &BeeperStore) -> bool {
        let known = |id: &str| id == "hungryserv" || st.bridge_accounts.iter().any(|b| b.account_id == id);
        self.account_page.as_deref().is_some_and(known)
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
        const NAVIGATION: [&str; 11] = ["up", "down", "left", "right", "tab", "backspace", "delete", "home", "end", "pageup", "pagedown"];
        let combo = rec.combo();
        let has_mod = rec.mods.contains(&true);
        let function = |k: &str| k.strip_prefix('f').and_then(|n| n.parse().ok()).is_some_and(|n: u8| (1..=24).contains(&n));
        let single =
            prefs::SINGLE_KEY.contains(&rec.name) && !has_mod && rec.key.as_deref().is_some_and(|k| NAVIGATION.contains(&k) || function(k));
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
        let scan = async { tokio::task::spawn_blocking(move || storage::scan(&store)).await.expect("storage scan thread") };
        Task::perform(scan, |a0| UiMessage::Settings(Msg::StorageScanned(a0)))
    }

    /// The entries that [`|a0| UiMessage::Settings(Msg::ClearStorage(a0))`] deletes, without the cache of the clear for all chats.
    pub(crate) fn storage_pick(&self, which: u8) -> Vec<&storage::Entry> {
        let s = &self.storage;
        let w = usize::from(which == 2);
        let kinds: Vec<storage::Kind> = match which {
            1 => vec![storage::Kind::Cache],
            _ => storage::Kind::ALL.into_iter().enumerate().filter(|(i, _)| s.kinds[w] & 1 << i != 0).map(|k| k.1).collect(),
        };
        let days = if which == 1 { 0 } else { STORAGE_DAYS[s.ages[w]] };
        let chat = s.chat.as_deref().filter(|_| which == 2);
        storage::select(s.all.as_deref().expect("a storage scan"), &kinds, chat, days, now_unix_secs())
    }

    pub(crate) fn load_devices(&mut self) -> Task<UiMessage> {
        if self.active_settings_tab == SettingsTab::General {
            let list = async { tokio::task::spawn_blocking(record::inputs).await.expect("input list worker thread") };
            return Task::perform(list, |a0| UiMessage::Settings(Msg::MicsLoaded(a0)));
        }
        if self.active_settings_tab == SettingsTab::Storage {
            return self.scan_storage();
        }
        if self.active_settings_tab != SettingsTab::Devices {
            return Task::none();
        }
        let st = self.store.read();
        match (st.matrix.clone(), st.config.demo) {
            (Some(client), _) => Task::perform(beeper_core::matrix::devices(client), |a0| UiMessage::Settings(Msg::DevicesLoaded(a0))),
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
        Task::perform(logout, move |r| UiMessage::Settings(Msg::DeviceLoggedOut(id.clone(), r)))
    }

    pub(crate) fn device_logged_out(&mut self, id: String, r: Result<(), String>) -> Task<UiMessage> {
        match r {
            Ok(()) => return self.load_devices(),
            Err(e) if e == beeper_core::matrix::REAUTH => return self.open_reauth(UiMessage::Settings(Msg::LogoutDevice(id))),
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
            .on_input(|a0| UiMessage::Settings(Msg::Query(a0)))
            .on_submit_maybe(match self.settings_query.trim().is_empty() {
                true => self.settings_search_on.then_some(UiMessage::Settings(Msg::Suggest(self.settings_focus))),
                false => hits.get(self.settings_focus).map(|&h| UiMessage::Settings(Msg::Pick(h))),
            })
            .size(rem(13.0))
            .line_height(lh(16.0))
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.text, p.accent, 0.0));
        let clear = (!self.settings_query.is_empty())
            .then(|| button(ic(icon::CLOSE, 12.0).color(p.strong)).bare(p.strong).on_press(UiMessage::Settings(Msg::Query(String::new()))));
        // Desktop's 16 px Material icon is scaled by 1.2 without a change to its 16 px box.
        let search = container(
            row![
                ic(icon::SEARCH, 16.0 * 1.2).color(p.strong),
                crate::focus::watch(search_input, |on| UiMessage::Settings(Msg::SearchFocus(on)))
            ]
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
                column![ell(&name, detail_w, 13.0, Weight::Semibold).line_height(lh(16.0)).color(p.strong)]
                    .push(me.email.as_deref().map(|e| ell(e, detail_w, 10.0, Weight::Normal).line_height(lh(12.0)).color(p.strong)))
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
                return c.push(button(profile()).padding(0).width(Fill).style(style).on_press(UiMessage::Settings(Msg::SelectTab(tab))));
            }
            // Desktop's `allProperAccounts`: the Beeper account and the chat accounts.
            let count = (tab == SettingsTab::Accounts)
                .then(|| container(txt((st.accounts.len() + 1).to_string(), 11.0).color(p.strong)).center(18).style(fill_box(p.chip, 9.0)));
            let line = row![tile(tab, false, 1.0), txt(tab.name(), 14.0).color(p.strong), Space::new().width(Fill)]
                .push(count)
                .spacing(8)
                .align_y(Center)
                .height(if on { 44 } else { 43 })
                .padding([0, 8]);
            // Desktop's `.pref-option:has(+ .selected)` and `:last-child` have no divider.
            let next_on = tabs.get(i + 1).is_none_or(|n| *n == self.active_settings_tab);
            c.push(button(line).padding(0).width(Fill).style(style).on_press(UiMessage::Settings(Msg::SelectTab(tab)))).push((!on).then(
                || {
                    let rule = if next_on { CLEAR } else { p.divider };
                    container(container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0))).padding(Padding::ZERO.left(36.0))
                },
            ))
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
            .size(rem(13.0))
            .line_height(lh(16.0))
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
            c.push(mouse_area(line).on_enter(UiMessage::Settings(Msg::Focus(i))).on_release(UiMessage::Settings(Msg::Pick(hit))))
        });
        let list: El = match self.settings_query.trim().is_empty() {
            true => nav.into(),
            false => container(results).padding(Padding::ZERO.top(3)).into(),
        };
        // Desktop's `SearchSuggestions` of the focused empty search: a card 4 px under the field, as wide as it.
        let suggested = (self.settings_search_on && self.settings_query.trim().is_empty()).then(|| {
            let gray = Color::from_rgb8(0x80, 0x80, 0x80);
            let head = row![Space::new().width(20), semi(tr("Suggestions"), 11.0).color(gray).line_height(lh(14.0))]
                .spacing(8)
                .padding(Padding::ZERO.bottom(4.0));
            let rows = suggestions(&st).into_iter().enumerate().map(|(i, (label, tab, _))| -> El<'_> {
                let face: El<'_> = match tab {
                    SettingsTab::BeeperPlus => Space::new().width(20).into(),
                    _ => tile(tab, false, 1.0).into(),
                };
                let line =
                    row![face, txt(tr(label), 13.0).color(p.strong).line_height(lh(16.0))].spacing(8).align_y(Center).padding([6, 4]);
                let bg = if i == self.settings_focus { p.strong.scale_alpha(0.1) } else { CLEAR };
                mouse_area(container(line).width(Fill).style(fill_box(bg, 6.0)))
                    .on_enter(UiMessage::Settings(Msg::Focus(i)))
                    .on_release(UiMessage::Settings(Msg::Suggest(i)))
                    .into()
            });
            let card = Column::with_children(std::iter::once(head.into()).chain(rows)).padding([12, 4]);
            container(card).width(Fill).style(glow_box(p.card, Color::BLACK.scale_alpha(0.12), 10.0, 0.0, 10.0))
        });
        let field =
            column![search, scrollable(list).id("settings-nav").direction(bar()).height(Fill).style(scroll_style(p.scroll))].spacing(5);
        let field = stack![field].push(suggested.map(|s| container(s).padding(Padding::ZERO.top(28.0 + 4.0))));
        let left = column![container(semi(tr("Settings"), 14.0).color(p.strong)).center_y(35).padding([0, 8]), field]
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
        let base = container(row![left, container(right).padding(Padding::new(8.0).right(0.0))]).width(Fill).height(Fill);
        drop(st);
        let confirm = self.confirm.filter(|d| SETTINGS_DIALOGS.contains(d)).map(|d| self.view_confirm(d));
        // The menu of an account row of the Accounts settings, or of the actions of an account page.
        let menu = self.list().chat_menu.as_ref().and_then(|(cid, at)| {
            let (id, kind) = match cid.strip_prefix(ROW_MENU) {
                Some(id) => (id, SpaceMenu::Prefs),
                None => (cid.strip_prefix(PAGE_MENU)?, SpaceMenu::Page),
            };
            Some((id, kind, at))
        });
        let menu = menu.map(|(id, kind, at)| {
            let (menu, sub) = self.account_menu(id, kind);
            let size = self.settings_size;
            let items = if crate::nickname::can_nickname(&self.store.read(), id) { 5.0 } else { 4.0 };
            let (w, h) = (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + items * menu_pitch() + MENU_RULE);
            let (x, y) = (at.x.min(size.width - w - 8.0), at.y.min(size.height - h - 8.0));
            let menu = container(mouse_area(row![menu].push(sub)).on_press(UiMessage::Noop))
                .padding(Padding::ZERO.left(x).top(y))
                .width(Fill)
                .height(Fill);
            crate::backdrop(menu)
                .on_press(UiMessage::ChatList(chat_list::Msg::CloseMenu))
                .on_right_press(UiMessage::ChatList(chat_list::Msg::CloseMenu))
        });
        let profile = self.profile_overlay();
        let picker = self.pane.emoji.as_ref().filter(|pk| pk.pref.is_some()).map(|_| {
            let picker = self.view_emoji_picker(&self.store.read()).expect("the picker is open");
            let picker = container(mouse_area(picker).on_press(UiMessage::Noop)).center(Fill);
            crate::backdrop(picker).on_press(UiMessage::QuickEmoji(None))
        });
        stack![base.style(fill_box(p.window, 0.0))].push(menu).push(confirm).push(profile).push(picker).into()
    }

    /// Desktop's `BeeperConnectAddTokenModal` in its window.
    pub(crate) fn view_add_token<'a>(&'a self, a: &'a AddToken) -> El<'a> {
        let p = &self.pal;
        let label = |t: &'static str| txt(tr(t), 12.0).color(p.text);
        let content: El<'a> = match &a.token {
            None => {
                let name = text_input("Cursor", &a.name)
                    .id("token-name")
                    .on_input(|a0| UiMessage::Integrations(integrations::Msg::AddName(a0)))
                    .on_submit(UiMessage::Integrations(integrations::Msg::Create))
                    .size(rem(12.0))
                    .width(200)
                    .padding([4, 6])
                    .style(outline_input(p));
                let line = |t, control: El<'a>| -> El<'a> {
                    row![label(t).width(Fill), control].align_y(Center).height(38).padding([0, 8]).into()
                };
                let expiry = select(p, [EXPIRIES[4], EXPIRIES[0], EXPIRIES[1], EXPIRIES[2], EXPIRIES[3]], a.expiry, |a0| {
                    UiMessage::Integrations(integrations::Msg::AddExpiry(a0))
                });
                let sensitive = column![
                    line("Allow sensitive actions", toggle(p, a.write, Some(UiMessage::Integrations(integrations::Msg::AddWrite)))),
                    container(txt(tr("Grant permission to send messages and modify your account"), 11.0).color(p.sub))
                        .padding(Padding::new(8.0).top(0.0))
                ];
                let create = button(semi(tr("Create Access Token"), 12.0).color(p.accent))
                    .padding([4, 8])
                    .style(btn_style([p.accent.scale_alpha(0.1); 3], [p.accent; 3], 20.0))
                    .on_press(UiMessage::Integrations(integrations::Msg::Create));
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
                let note = tr("<strong>Important:</strong> This token will only be shown once, please copy and save it.");
                let (strong, rest) = note.split_once("</strong>").expect("the note has a bold part");
                let note = rich_text![span::<(), _>(strong.trim_start_matches("<strong>")).font(fw(Weight::Bold)), span(rest)]
                    .size(rem(13.0))
                    .line_height(lh(16.0))
                    .color(p.text);
                let shown = container(semi(token.as_str(), 15.0).line_height(lh(20.0)).color(p.strong))
                    .padding([8, 16])
                    .style(fill_box(p.chip, 8.0));
                let icon = if a.copied { &COPIED_ICON } else { &COPY_ICON };
                let copy = row![tinted(icon, 20.0, p.accent), txt(tr(if a.copied { "Copied!" } else { "Copy" }), 13.0).color(p.accent)]
                    .spacing(4)
                    .align_y(Center)
                    .width(72);
                let shown = mouse_area(row![shown, copy].spacing(8).align_y(Center))
                    .on_press(UiMessage::Integrations(integrations::Msg::CopyToken));
                column![note, shown].spacing(8).align_x(Center).width(Fill).into()
            }
        };
        let title = container(semi(tr("Create a new token for Beeper Desktop API"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let pane = column![title, container(content).padding(Padding::new(16.0).top(16.0))];
        crate::integrations::modal_frame(p, pane.into())
    }

    /// Desktop's `TakeBeeperWithYou` card.
    fn take_beeper(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let action: El<'_> = match self.app_qr {
            true => container(svg(APP_QR.clone()).width(108).height(108)).padding(4).style(fill_box(Color::WHITE, 8.0)).into(),
            false => button(
                row![tinted(&QR_CODE, 14.0, Color::WHITE), txt(tr("Get the app"), 11.0).color(Color::WHITE)].spacing(2).align_y(Center),
            )
            .padding([4, 8])
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 20.0))
            .on_press(UiMessage::Accounts(accounts::Msg::ShowAppQr))
            .into(),
        };
        let heading = semi(tr("Take Beeper with you"), 16.0).color(p.strong);
        let card = column![svg(TAKE_BEEPER.clone()).width(58).height(48), heading, action].spacing(12).align_x(Center);
        let card = card.push(self.send_link_to_phone(st));
        container(card).center_x(Fill).padding(Padding::new(8.0).top(20.0).bottom(12.0)).style(fill_box(p.prefs_group, 12.0)).into()
    }

    /// Desktop's `SendLinkToPhone`: "Send a link to %s" of the first number, or of the picked one, which with several
    /// numbers opens a list of them and their networks, and "Link sent to %s" for 3 s after a send.
    fn send_link_to_phone(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        let phones = crate::link_phones(st);
        let picked = self.link_pick.0.as_ref().and_then(|a| phones.iter().find(|(id, _)| id == a));
        let (account, phone) = picked.or(phones.first())?.clone();
        let sent = self.link_sent.as_ref().filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(3));
        let label = match sent {
            Some((to, _)) => trf("Link sent to %s", &[to]),
            None => trf("Send a link to %s", &[&phone]),
        };
        let many = phones.len() > 1;
        let busy = self.link_to.is_some();
        // `.sendLinkTrigger`: 12 px semibold in the primary color, at 0.6 while it sends.
        let fg = p.accent.scale_alpha(if busy { 0.6 } else { 1.0 });
        let chevron = many.then(|| tinted(&CHEVRON_DOWN, 16.0, p.strong));
        let trigger = button(row![semi(label, 12.0).color(fg)].push(chevron).spacing(2).align_y(Center))
            .padding([4, 8])
            .style(|_, _| button::Style::default())
            .on_press_maybe((!busy).then(|| match many {
                true => UiMessage::Accounts(accounts::Msg::ToggleLinkMenu),
                false => UiMessage::Accounts(accounts::Msg::SendLink(account.clone())),
            }));
        // `.sendLinkDropdown`: each number with its network, on the background with a border and 10 px corners.
        let menu = (many && self.link_pick.1).then(|| {
            let items = phones.iter().map(|(id, phone)| {
                let item = row![
                    semi(phone.clone(), 13.0).color(p.strong),
                    Space::new().width(Fill),
                    txt(crate::network_name(id), 12.0).color(p.sub)
                ]
                .spacing(12)
                .align_y(Center);
                button(item)
                    .padding([8, 12])
                    .width(Fill)
                    .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 6.0))
                    .on_press(UiMessage::Accounts(accounts::Msg::SendLink(id.clone())))
                    .into()
            });
            container(Column::with_children(items)).padding(4).width(240).style(bordered_box(p.card, p.prefs_border, 1.0, 10.0))
        });
        Some(column![trigger].push(menu).spacing(4).align_x(Center).into())
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
                false => UiMessage::Settings(Msg::AskLogoutDevice(d.id.clone())),
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

    /// Desktop's `LLt` under the accounts: "Add an Account" and the accounts that remain, or with none left below
    /// Beeper Plus Plus, its `AddMoreAccountsBanner` (`RLt`) with the upgrade of `moreAccountsBannerText`.
    fn accounts_foot(&self, st: &BeeperStore, add: UiMessage) -> El<'_> {
        static COLLAGE: std::sync::LazyLock<iced::widget::image::Handle> = std::sync::LazyLock::new(|| {
            iced::widget::image::Handle::from_bytes(include_bytes!("../assets/settings/platform_collage.png").as_slice())
        });
        let p = &self.pal;
        let (f, plan) = (&st.features, st.whoami.plan);
        let n = f.accounts_remaining();
        let remaining = |n: usize, free: bool| match free {
            true => trn("%d Free Account Remaining", "%d Free Accounts Remaining", n, &[&n]),
            false => trn("%d Account Remaining", "%d Accounts Remaining", n, &[&n]),
        };
        if plan != Plan::PlusPlus && n == 0 {
            let up = crate::networks::upgrade_plan(plan).expect("a plan below Beeper Plus Plus");
            // Desktop's free plan raises the limit to the `max-accounts` of Beeper Plus, which it falls back to 10 for.
            let description = match plan {
                Plan::Free => tr("You've reached the account limit. To add another account, either disable one of your current accounts, or upgrade to %1$s to raise your account limit to %2$s.")
                    .replace("%1$s", up)
                    .replace("%2$s", "10"),
                _ => trf("You've reached the account limit. To add another account, either disable one of your current accounts, or upgrade to %s to raise your account limit.", &[&up]),
            };
            let upgrade = button(med(trf("Upgrade to %s", &[&up]), 13.0).color(p.strong))
                .bare(p.strong)
                .padding([8, 10])
                .on_press(UiMessage::Settings(Msg::Open(SettingsTab::BeeperPlus)));
            let banner = column![
                semi(tr("Add More Accounts"), 13.0).color(p.strong),
                txt(remaining(0, plan == Plan::Free), 12.0).color(p.muted),
                container(iced::widget::image(COLLAGE.clone()).width(289).height(24)).padding(Padding::ZERO.top(10)),
                container(txt(description, 11.0).color(p.strong).center()).max_width(300).padding(Padding::ZERO.top(8)),
                container(upgrade).padding(Padding::ZERO.top(8)),
            ]
            .align_x(Center);
            return container(banner).width(Fill).padding([12, 8]).style(bordered_box(CLEAR, p.chip, 1.0, 8.0)).into();
        }
        // Desktop's `accountsRemainingText`.
        let left = match f.unlimited_accounts() {
            true => tr("Infinite Accounts Remaining").to_string(),
            false => remaining(n, plan == Plan::Free),
        };
        column![button(txt(tr("Add an Account"), 13.0).color(p.accent)).bare(p.accent).on_press(add), txt(left, 12.0).color(p.muted)]
            .spacing(8)
            .align_x(Center)
            .width(Fill)
            .into()
    }

    fn settings_content(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let groups: Vec<El<'_>> = match self.active_settings_tab {
            SettingsTab::Accounts if self.account_page_shown(st) => {
                self.account_page(st, self.account_page.as_deref().expect("an account page"))
            }
            SettingsTab::Accounts => {
                // Desktop's account row: the account user's photo (`userImage`), the network, and the user.
                let account = |logo: &str, owner: &str, photo: Option<&str>, name: String, cloud: bool, sub: Option<String>| -> El<'_> {
                    let title = row![self.face(photo, owner, 14.0, 0.0), semi(name, 14.0).color(p.strong)]
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
                    let row = button(line).bare(p.strong).on_press(UiMessage::Accounts(accounts::Msg::Open(Some(logo.into()))));
                    mouse_area(row).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("{ROW_MENU}{logo}")))).into()
                };
                let me = st.me();
                // Desktop's `defaultUserDisplayText` of the Beeper account is the profile's full name.
                let beeper = account(
                    "hungryserv",
                    me.full_name.as_deref().expect("self name"),
                    me.img_url.as_deref(),
                    "Beeper".into(),
                    false,
                    me.full_name.clone().or_else(|| me.username.clone()),
                );
                let rows = std::iter::once(("hungryserv", beeper)).chain(st.bridge_accounts.iter().map(|b| {
                    let r = b.remote_profile.as_ref();
                    let owner = r.and_then(|r| r.name.as_deref()).unwrap_or("");
                    let photo = r.and_then(|r| r.avatar.as_deref());
                    let row = account(
                        &b.account_id,
                        owner,
                        photo,
                        b.network_name.clone(),
                        b.mode == BridgeMode::Cloud,
                        // Desktop's `userDisplayText`: the nickname, or the user.
                        st.custom_name(&b.account_id).map(str::to_string).or_else(|| b.display_text_with(crate::fmt_phone)),
                    );
                    (b.account_id.as_str(), row)
                }));
                // Desktop's `ZLt` sorts the accounts by `accounts_order`, and is a `Drop` that the keyboard reorders
                // (see `sortable`): the moved account shows in the place it drops at, at 40 % opacity.
                let mut rows: Vec<(&str, El<'_>)> = rows.collect();
                rows.sort_by_key(|(id, _)| st.settings.accounts_order.get(*id).copied().unwrap_or(u32::MAX));
                let items: std::sync::Arc<[String]> = rows.iter().map(|(id, _)| id.to_string()).collect();
                let window = self.settings_window;
                let sort = self.key_sort_of(window, sortable::List::Accounts);
                if let Some(order) = sort.and_then(|s| s.moved(&items)) {
                    rows.sort_by_key(|(id, _)| order.iter().position(|o| o == id));
                }
                let rows = rows
                    .into_iter()
                    .enumerate()
                    .flat_map(|(i, (id, r))| {
                        let pick = Self::pick(window, sortable::List::Accounts, id, &items, 1);
                        let row = self.focus_row(r, format!("account-{id}"), Keys::press(pick));
                        // `opacity: .4` over the card, as a veil of its color.
                        let veil = container(Space::new().width(Fill).height(Fill)).style(fill_box(p.prefs_group.scale_alpha(0.6), 0.0));
                        let lifted = sort.is_some_and(|s| s.id == id);
                        [
                            (i > 0).then(|| container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0)).into()),
                            Some(stack![row].push(lifted.then_some(veil)).into()),
                        ]
                    })
                    .flatten();
                let add = UiMessage::Networks(crate::networks::Msg::Open);
                let remaining = st.features.accounts_remaining();
                // Desktop's `QLt`: the add button shows while the plan allows another account.
                let head = row![med(tr("Accounts"), 12.0).color(p.weak), Space::new().width(Fill)]
                    .push((remaining > 0).then(|| icon_btn(p, icon::CIRCLE_PLUS, 16.0, 22.0, p.accent, add.clone())))
                    .height(26)
                    .align_y(Center)
                    .padding([0, 4]);
                let card =
                    container(Column::with_children(rows)).width(Fill).padding(1).style(bordered_box(p.prefs_group, p.chip, 1.0, 8.0));
                vec![column![head, column![card, self.accounts_foot(st, add)].spacing(11)].spacing(8).into()]
            }
            tab @ (SettingsTab::General | SettingsTab::Appearance | SettingsTab::Notifications | SettingsTab::Ai) => {
                self.pref_items(pref_page(tab), st)
            }
            SettingsTab::Hotkeys => vec![self.hotkeys_page(st)],
            // Desktop's `DevicesPane`: the app card, the lists of devices, and the error.
            SettingsTab::Devices => {
                let lists = match &self.devices {
                    None => Vec::new(),
                    Some(Err(_)) => {
                        let error = tr("Failed to fetch devices. Please report a problem if this keeps happening.");
                        vec![txt(error, 13.0).color(p.text_danger).into()]
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
                let page = column![self.take_beeper(st), Space::new().height(8)].extend(lists);
                vec![page.into()]
            }
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
                let title = column![bold("Beeper", 19.5).color(p.text), txt(format!("v{APP_VERSION}"), 13.0).color(p.weak)].spacing(4);
                let head = row![svg(LOGO.clone()).width(68).height(68), title].spacing(6).align_y(Center);
                let links = [
                    (tr("View changelog"), UiMessage::OpenUrl(CHANGELOG_URL.into())),
                    (tr("Report a problem"), UiMessage::OpenUrl(ISSUES_URL.into())),
                    (tr("Privacy"), UiMessage::OpenUrl("https://www.beeper.com/privacy".into())),
                    (tr("Legal Notices"), UiMessage::OpenModal(crate::modals::Modal::Legal, self.settings_window)),
                ];
                let rows = links.into_iter().enumerate().flat_map(|(i, (label, msg))| {
                    let last = i == 3;
                    let link = button(container(txt(label, 13.0).color(p.link)).center_y(if last { 40 } else { 39 }))
                        .width(Fill)
                        .bare(p.link)
                        .padding([0, 13])
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
        let body = |t: &str, color| txt(t.to_string(), 13.0).color(color);
        let actions = |buttons: Vec<El<'a>>| -> El<'a> { Row::with_children(buttons).spacing(4).padding([14, 13]).into() };
        let copy = |i: u8, value: String| {
            let icon = if s.copied == Some(i) { &COPIED_ICON } else { &COPY_ICON };
            button(tinted(icon, 16.0, p.text))
                .bare(p.text)
                .padding(4)
                .on_press(UiMessage::Integrations(integrations::Msg::CopyField(i, value)))
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
                toggle(p, st.settings.enable_pas_api, Some(UiMessage::Settings(Msg::ToggleSetting(|x| x.enable_pas_api ^= true)))),
            ),
            divider(p.prefs_border, 0.0),
            actions(vec![
                pill_btn(p, tr("Learn more"), UiMessage::OpenUrl("https://developers.beeper.com/desktop-api/".into())),
                pill_btn(p, tr(community), UiMessage::Settings(Msg::JoinCommunity)),
            ]),
        ];
        let mut mcp: Vec<El<'a>> = Vec::new();
        for ((name, label), logo) in MCP_CLIENTS.into_iter().zip([&*CLAUDE_LOGO, &*CURSOR_LOGO, &*VSCODE_LOGO]) {
            let connect = button(txt(tr(label), 13.0).font(fw(Weight::Medium)).line_height(lh(16.0)).color(p.accent))
                .bare(p.accent)
                .padding([4, 8])
                .on_press(UiMessage::Integrations(integrations::Msg::McpInstall(name)));
            let line = row![svg(logo.clone()).width(18).height(18), semi(name, 14.0).color(p.text)].spacing(8);
            let line = row![line.width(Fill).align_y(Center), connect].align_y(Center).padding(Padding::new(13.0).bottom(12.0));
            mcp.extend([line.into(), divider(p.prefs_border, 0.0)]);
        }
        let target = MCP_TARGETS[s.target];
        let value = mcp_value(target, &base);
        let pick = select(p, MCP_TARGETS, target, |t| {
            UiMessage::Integrations(integrations::Msg::McpTarget(MCP_TARGETS.iter().position(|x| *x == t).expect("a listed client")))
        });
        mcp.push(row![container(body(tr("Install to..."), p.text)).width(Fill), pick].align_y(Center).padding(13).into());
        let blue = if self.dark { Color::from_rgb8(10, 132, 255) } else { Color::from_rgb8(0, 122, 255) };
        let install = matches!(target, "Claude Code" | "Codex").then(|| {
            let label = semi(tr("Copy Command"), 11.0).line_height(lh(12.0)).color(Color::WHITE);
            container(
                button(label)
                    .padding([4, 8])
                    .style(btn_style([blue; 3], [Color::WHITE; 3], 4.0))
                    .on_press(UiMessage::Integrations(integrations::Msg::McpInstall(target))),
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
            .line_height(lh(16.0))
            .color(p.text)
            .wrapping(iced::widget::text::Wrapping::Glyph);
        let code = stack![
            container(code).width(Fill).padding(Padding::new(12.0).right(40.0)).style(fill_box(p.chip, 6.0)),
            container(container(copy(0, value)).padding(8).style(fill_box(p.chip.scale_alpha(0.8), 6.0))).align_right(Fill)
        ];
        let hint = hint.map(|h| container(txt(tr(h), 11.0).color(p.weak)).padding(Padding::ZERO.bottom(4.0)));
        let details = column![].push(install).push(hint).push(container(code).padding([8, 0]));
        let details = details.padding(Padding::new(13.0).top(0.0).bottom(14.0));
        mcp.push(details.into());
        mcp.push(actions(vec![pill_btn(
            p,
            tr("See docs for MCP"),
            UiMessage::OpenUrl("https://developers.beeper.com/desktop-api/mcp".into()),
        )]));
        let endpoint = |i: u8, label: &'static str, url: String| -> El<'a> {
            let field = container(ell(&url, 200.0, 13.0, Weight::Normal).color(p.text)).width(212).padding([4, 6]).style(bordered_box(
                p.input,
                p.prefs_border,
                1.0,
                4.0,
            ));
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
                        container(crate::style::fixed(tr("Sensitive actions"), 12.0, Weight::Normal).line_height(lh(14.0)).color(p.text))
                            .padding([1, 4])
                            .style(fill_box(p.chip, 4.0))
                    });
                    let created = distance(now - t.created_at);
                    let when = match t.expires_at {
                        Some(e) => trf("Created %1$s ago · Expires in %2$s", &[&created, &distance(e - now)]),
                        None => trf("Created %s ago · Never expires", &[&created]),
                    };
                    let text =
                        column![row![body(&name, p.text)].push(tag).spacing(6).align_y(Center), body(&when, p.text.scale_alpha(0.7))];
                    let revoke = container(pill_btn(p, tr("Revoke"), UiMessage::Integrations(integrations::Msg::Revoke(t.id.clone()))))
                        .padding(Padding::ZERO.right(8.0));
                    let line = row![text.width(Fill), revoke].align_y(Center).padding(Padding::new(8.0).left(13.0).right(4.0));
                    [divider(p.prefs_border, 0.0), line.into()]
                })
                .skip(1)
                .collect(),
        };
        let plus = button(tinted(&ADD_ICON, 20.0, p.accent)).bare(p.accent).on_press(UiMessage::Integrations(integrations::Msg::Add(true)));
        let approved = column![
            row![container(body(tr("Approved connections"), p.muted)).width(Fill), plus].align_y(Center).padding([0, 13]),
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
            toggle(p, on, (!fixed).then_some(UiMessage::Integrations(integrations::Msg::RemoteAccess(!on)))),
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
        let body = |t: String, color| txt(t, 13.0).color(color);
        let h4 = |t| container(body(tr(t).into(), p.muted)).padding([5, 0]);
        let Some(all) = s.all.as_deref() else {
            // `StoragePrefsSubView`'s loading text, warning, buttons, cache subtitle and room counts are in px.
            let loading = container(crate::style::fixed(tr("Loading..."), 14.0, Weight::Normal).color(p.muted)).center_x(Fill).padding(40);
            return column![h4("Storage"), loading].into();
        };
        let totals = storage_totals(all);
        let legend = totals.iter().map(|&(k, bytes)| {
            let (label, color) = kind_meta(k);
            let swatch = container(Space::new()).width(10).height(10).style(fill_box(color, 2.0));
            let size = semi(human_size(bytes), 13.0).color(p.text);
            row![swatch, column![body(tr(label).into(), p.text), Space::new().width(80)], size].spacing(8).align_y(Center).into()
        });
        let total = human_size(totals.iter().map(|t| t.1).sum());
        let fg = p.fg;
        let ring = stack![
            svg(s.chart.clone().expect("a chart for the scan")).width(200).height(200),
            container(txt(total, 18.0).color(fg)).center(200)
        ];
        let legend = (!totals.is_empty()).then(|| Column::with_children(legend).spacing(8));
        let chart = container(row![ring].push(legend).spacing(24).align_y(Center)).center_x(Fill).padding(16);
        let card = |inner: El<'a>| container(inner).width(Fill).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
        let link = |label: &'static str, color: Color, size: f32, on: bool, msg| {
            let color = if on { color } else { Color { a: 0.4, ..color } };
            let label = crate::style::fixed(tr(label), size, Weight::Normal).line_height(1.4).color(color);
            button(label).bare(color).on_press_maybe(on.then_some(msg))
        };
        let warning = || {
            let warn = "⚠️ This will delete local data. You can recover content by logging out and back into your accounts.";
            let warn = crate::style::fixed(tr(warn), 12.0, Weight::Normal).line_height(1.4).color(p.muted);
            container(warn.width(Fill).align_x(Center)).padding([0, 16])
        };
        let busy = matches!(s.clearing, Some(0 | 1));
        let label = |which| if s.clearing == Some(which) { "Clearing..." } else { "Clear Storage..." };
        let clear = column![
            body(tr("Clear storage older than").into(), p.muted),
            self.storage_ages(0),
            self.storage_kinds(0, busy),
            warning(),
            link(label(0), p.text_danger, 13.0, !busy && s.kinds[0] != 0, UiMessage::Ask(Some(ASK_CLEAR_STORAGE)))
        ]
        .spacing(12)
        .align_x(Center)
        .padding(16);
        let cached = totals.iter().any(|t| t.0 == storage::Kind::Cache);
        let cache = row![
            column![
                body(tr("Clear All Cache").into(), p.text),
                crate::style::fixed(
                    tr("Clears unused previews, old drafts, and temporary data to free up space. Your messages are safe."),
                    13.0,
                    Weight::Normal
                )
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
        let rooms = rooms.into_iter().filter(|r| r.1[2] > 0).take(10).enumerate().flat_map(|(i, (id, [files, msgs, bytes]))| {
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
                crate::style::fixed(counts.join(" | "), 13.0, Weight::Normal).color(p.muted),
                semi(human_size(bytes), 13.0).color(p.text).width(80).align_x(iced::alignment::Horizontal::Right)
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
            let line = button(line).bare(p.text).on_press(UiMessage::Settings(Msg::StorageChat(id.into())));
            let panel = open.then(|| {
                let busy = s.clearing == Some(2);
                let panel =
                    column![body(tr("Clear storage older than").into(), p.muted), self.storage_ages(1), self.storage_kinds(1, busy)]
                        .push((s.kinds[1] & 1 << 4 != 0).then(warning))
                        .push(link(label(2), p.text_danger, 13.0, !busy && s.kinds[1] != 0, UiMessage::Ask(Some(ASK_CLEAR_CHAT))))
                        .spacing(12)
                        .align_x(Center)
                        .padding(Padding::new(16.0).top(12.0));
                column![divider(p.prefs_border, 0.0), panel].into()
            });
            [(i > 0).then(|| divider(p.prefs_border, 0.0)), Some(line.into()), panel].into_iter().flatten()
        });
        let rooms: Vec<El<'a>> = rooms.collect();
        let by_chat = (!rooms.is_empty())
            .then(|| column![h4("Storage by Chat"), card(Column::with_children(rooms).into())].spacing(6).padding(Padding::ZERO.top(32.0)));
        column![h4("Storage"), chart, card(clear.into()), Space::new().height(16), card(cache.into())].push(by_chat).into()
    }

    /// Desktop's `BrandSegmentControl` of the ages, for all chats (0) or for the open chat (1).
    fn storage_ages<'a>(&self, which: usize) -> El<'a> {
        let days = STORAGE_DAYS.iter().map(|&d| trn("%d day", "%d days", d as usize, &[&d]));
        let control = self
            .segments(days, self.storage.ages[which], |i| UiMessage::Settings(Msg::StorageAge(which, i)))
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
            let label = txt(tr(kind_meta(k).0), 13.0).color(p.text);
            button(row![checkbox(p, on), label].spacing(6).align_y(Center))
                .bare(p.text)
                .on_press_maybe((!busy).then_some(UiMessage::Settings(Msg::StorageKind(which, i))))
                .into()
        });
        Row::with_children(kinds).spacing(16).wrap().vertical_spacing(16).align_x(Center).into()
    }

    fn hotkeys_page(&self, st: &BeeperStore) -> El<'_> {
        let s = &st.settings;
        let p = &self.pal;
        let edge = p.border_neutrals;
        let search = text_input(tr("Search hotkeys..."), &self.hotkey_query)
            .on_input(|a0| UiMessage::Hotkeys(hotkeys::Msg::Query(a0)))
            .size(rem(13.0))
            .line_height(lh(15.0))
            .padding([4, 7])
            .width(219)
            .style(outline_input(p));
        let revert = button(txt(tr("Revert all"), 13.0).line_height(lh(15.0)).width(Fill).align_x(Center))
            .width(100)
            .padding([4, 6])
            .style(pill_style(p))
            .on_press(UiMessage::Hotkeys(hotkeys::Msg::RevertAll));
        let icon_btn = |glyph: El<'static>, msg| button(glyph).bare(p.muted).on_press(msg);
        let hotkey = |name: &'static str| -> El<'_> {
            let keys = s.hotkey_keys(name);
            let rec = self.hotkey_rec.as_ref().filter(|r| r.name == name);
            let conflict = rec.and_then(|r| r.conflict(s));
            let shown = rec.map_or_else(|| keys.to_string(), HotkeyRec::combo);
            let recorder = (keys != "disabled").then(|| {
                let ink = match (conflict, rec) {
                    (Some(_), _) => ERROR,
                    (None, Some(_)) => p.weak,
                    (None, None) => p.text,
                };
                let label: El<'_> = match prefs::hotkey_label(&shown) {
                    l if l.is_empty() => txt(tr("Record Shortcut"), 11.0).color(p.muted).into(),
                    l => txt(l, 13.0).line_height(lh(18.0)).color(ink).into(),
                };
                let buttons = match rec {
                    Some(_) => row![]
                        .push(conflict.is_none().then(|| {
                            icon_btn(ic(icon::CHECK, 12.0).color(p.muted).into(), UiMessage::Hotkeys(hotkeys::Msg::Recorder(true)))
                        }))
                        .push(icon_btn(ic(icon::CLOSE, 12.0).color(p.muted).into(), UiMessage::Hotkeys(hotkeys::Msg::Recorder(false)))),
                    None => row![].push(
                        (keys != prefs::hotkey_default(name))
                            .then(|| icon_btn(tinted(&REVERT, 12.0, p.muted), UiMessage::Hotkeys(hotkeys::Msg::Revert(name)))),
                    ),
                };
                let border = match (conflict, rec) {
                    (Some(_), _) => ERROR,
                    (None, Some(_)) => p.accent,
                    (None, None) => edge,
                };
                let label =
                    mouse_area(container(label).center_x(160).center_y(20)).on_press(UiMessage::Hotkeys(hotkeys::Msg::Record(name)));
                let buttons = container(buttons.spacing(6).align_y(Center)).align_right(160).center_y(20).padding([0, 2]);
                container(stack![label, buttons]).style(bordered_box(CLEAR, border, 1.0, 4.0))
            });
            // Desktop's `Global` tag (`.tag`) of a global hotkey, with its tooltip.
            let global = ["Focus App", "Focus App and Command Bar"].contains(&name).then(|| {
                let tag = container(txt(trx("Global", "hotkey scope tag"), 12.0).color(p.text))
                    .padding([1, 4])
                    .style(fill_box(p.strong.scale_alpha(0.1), 4.0));
                let tip = trf("Global hotkeys will work when any app is active, as long as %s is running", &[&"Beeper"]);
                container(self.tip(tag.into(), tip, vec![], tooltip::Position::Top)).padding(Padding::ZERO.left(6.0))
            });
            let title = row![ell(tr(name), 226.0, 13.0, Weight::Normal).color(p.text)].push(global).align_y(Center);
            let line = row![container(title).width(Fill)]
                .push(recorder)
                .push(toggle(p, keys != "disabled", Some(UiMessage::Hotkeys(hotkeys::Msg::Toggle(name)))))
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
            let head = txt(tr(group), 13.0).color(p.muted);
            column![container(head).padding(Padding::ZERO.top(24.0).bottom(12.0))].extend(rows).into()
        });
        let head = row![search, Space::new().width(Fill), revert].align_y(Center).padding(Padding::ZERO.bottom(10.0));
        let line = container(Space::new()).width(Fill).height(1).style(fill_box(edge, 0.0));
        // Desktop's `.footer-row`: a rule 1em below the list, then the button 10 px under it at the left.
        let texts = button(txt(tr("Switch to Texts.app keybindings"), 13.0).line_height(lh(15.0)))
            .padding([4, 6])
            .style(pill_style(p))
            .on_press(UiMessage::Hotkeys(hotkeys::Msg::TextsKeys));
        let rule = container(Space::new()).width(Fill).height(1).style(fill_box(edge, 0.0));
        let foot = column![rule, texts].spacing(10).padding(Padding::ZERO.top(13.0).bottom(13.0));
        let card = column![head, line, Space::new().height(6)].extend(groups).push(foot);
        container(card).width(Fill).padding(14).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0)).into()
    }

    /// Desktop's Keyboard Shortcuts list: the first keys of each hotkey, at the right of a `window`. In a window too
    /// narrow for it, like the sign-in window, it narrows, where Desktop's goes past the left edge of the window.
    pub(crate) fn view_hotkeys_modal(&self, query: &str, window: Size) -> El<'_> {
        let p = &self.pal;
        let card_w = (window.width - 13.0).min(350.0);
        let row_w = card_w - 25.0;
        let st = self.store.read();
        let modifier_only = |k: &str| k.rsplit('+').next().is_some_and(|m| ["ctrl", "shift", "alt"].contains(&m));
        let all = prefs::HOTKEYS
            .iter()
            .filter(|h| !modifier_only(h.2) && hotkey_shown(&st, h.0))
            .map(|h| (h.0, h.1, st.settings.hotkey_keys(h.0)));
        let fixed = prefs::FIXED_HOTKEYS.iter().map(|h| (h.0, h.1, st.settings.hotkey_keys(h.0)));
        let all = all.chain(fixed).map(|(n, g, k)| (n, g, k.to_string()));
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
                container(txt(k, 13.0).color(p.text)).center_x(w).center_y(22).style(fill_box(p.fg.scale_alpha(0.15), 4.0))
            });
            let caps = Row::with_children(caps.map(El::from)).spacing(4).padding(Padding::ZERO.right(4.0));
            let name = ell(tr(name), row_w - 22.4 - 12.0 - caps_w, 13.0, Weight::Normal).color(p.text);
            let line = row![name, Space::new().width(Fill), caps].spacing(12).align_y(Center);
            let hover = p.fg.scale_alpha(0.05);
            button(line)
                .width(row_w)
                .padding([4.8, 11.2])
                .style(btn_style([CLEAR, hover, hover], [p.text; 3], 8.0))
                .on_press(UiMessage::Noop)
                .into()
        };
        let groups = first_groups(&shown, |h| h.1).into_iter().map(|group| {
            let head = container(semi(tr(group), 13.0).line_height(lh(18.2)).color(p.text)).padding([4.8, 11.2]).width(row_w);
            let rows = shown.iter().filter(|h| h.1 == group).map(row);
            column![head].extend(rows).padding(Padding::ZERO.top(12.8)).into()
        });
        let empty = shown.is_empty().then(|| {
            // `.hotkeys-modal .hotkey-search-empty` is 16 px.
            container(crate::style::fixed(tr("No results"), 16.0, Weight::Normal).color(p.fg.scale_alpha(0.35)))
                .center_x(Fill)
                .height(Fill)
                .center_y(Fill)
        });
        let list = Column::with_children(groups).push(empty).width(Fill).align_x(Center);
        let height = 0.8 * window.height;
        let list = scrollable(list)
            .direction(bar())
            .style(scroll_style(p.scroll))
            .height(height)
            .on_scroll(|v| UiMessage::Hotkeys(hotkeys::Msg::Scrolled(v.absolute_offset().y > 20.0)));
        let search = text_input(tr("Search..."), query)
            .id("hotkeys-search")
            .on_input(|q| UiMessage::Hotkeys(hotkeys::Msg::Modal(Some(q))))
            .size(rem(13.0))
            // Chromium's `normal` line height of Inter, inside a 1 px border and 3 px by 6 px of padding.
            .line_height(lh(14.95))
            .padding([4, 7])
            .style(outline_input(p));
        let rule = p.fg.scale_alpha(if self.hotkeys_scrolled { 0.1 } else { 0.0 });
        let card = column![
            // `.modal h3` is 24 px.
            container(crate::style::fixed(tr("Keyboard Shortcuts"), 24.0, Weight::Bold).line_height(Pixels(33.6)).color(p.text))
                .padding([8.0, 24.0]),
            container(search).padding(pad(4.0, 22.4, 10.0, 22.4)),
            container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0)),
            list,
        ]
        .width(card_w)
        .padding([13, 0]);
        let card = mouse_area(container(card).style(fill_box(p.elevated, 8.0))).on_press(UiMessage::Noop);
        let shade = container(card).align_right(Fill).center_y(Fill).padding(Padding::ZERO.right(13.0));
        crate::backdrop(shade.style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.4), 0.0)))
            .on_press(UiMessage::Hotkeys(hotkeys::Msg::Modal(None)))
            .into()
    }

    /// Desktop's `AccountPrefsSubView`: the account, and its notification settings.
    fn account_page(&self, st: &BeeperStore, id: &str) -> Vec<El<'_>> {
        let p = &self.pal;
        let bridge = (id != "hungryserv").then(|| st.bridge_accounts.iter().find(|b| b.account_id == id).expect("the account of the page"));
        let (network, photo, [name, username, phone, email]) = match bridge {
            None => {
                let me = st.me();
                let user = [&me.full_name, &me.username, &me.phone_number, &me.email].map(Clone::clone);
                ("Beeper".into(), me.img_url.clone(), user)
            }
            Some(b) => {
                let r = b.remote_profile.clone().unwrap_or_default();
                (b.network_name.clone(), r.avatar, [r.name, r.username, r.phone, r.email])
            }
        };
        let back = button(tinted(&CHEVRON_LEFT, 16.0, p.strong)).bare(p.strong).on_press(UiMessage::Accounts(accounts::Msg::Open(None)));
        let title =
            row![svg(brand_svg(brand_key(id))).width(28).height(28), semi(network, 14.0).color(p.strong)].spacing(4).align_y(Center);
        let head = row![back, container(title).center_x(Fill), Space::new().width(16)].align_y(Center);
        let lines = [username.map(|u| format!("@{u}")), phone.map(|n| fmt_phone(&n)), email].into_iter().flatten();
        let user = column![]
            .push(name.as_deref().map(|n| container(self.face(photo.as_deref(), n, 28.0, 0.0)).padding(Padding::ZERO.bottom(8.0))))
            .push(name.map(|n| semi(n, 14.0).color(p.strong)))
            .extend(lines.map(|l| txt(l, 13.0).color(p.weak).into()))
            .align_x(Center);
        // Desktop's `AccountActions`: a 20 px pill with a 16 px chevron, 8 px into the top right corner of the card,
        // that opens the account menu.
        let actions = button(container(tinted(&CHEVRON_DOWN, 16.0, p.strong)).center_x(32).center_y(20))
            .padding(0)
            .style(btn_style([p.chip; 3], [p.strong; 3], 10.0))
            .on_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("{PAGE_MENU}{id}"))));
        let actions = container(actions).id("account-actions");
        let card = container(user).center_x(Fill).padding(8);
        let card = stack![card, container(actions).align_right(Fill).padding(8)];
        let card = container(card).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
        let props = st.settings.account(id);
        let set = |f: fn(&mut beeper_core::models::AccountProps)| {
            let mut q = props;
            f(&mut q);
            UiMessage::Accounts(accounts::Msg::SetProps(id.into(), q))
        };
        let toggle = |label, on, msg| pref_row(p, tr(label), None, None, toggle_faded(p, on, Some(msg), 1.0));
        let sounds = [tr("Default").to_string(), tr("None").to_string()];
        let (account, picked) = (id.to_string(), sounds[usize::from(props.silent)].clone());
        let sound = select(p, sounds.clone(), picked, move |o| {
            UiMessage::Accounts(accounts::Msg::SetProps(
                account.clone(),
                beeper_core::models::AccountProps { silent: o == sounds[1], ..props },
            ))
        });
        let inbox = st.matrix.as_ref().and(bridge).map(|b| {
            let labels: Vec<String> = INBOX_OPTIONS.iter().map(|&(_, l)| inbox_label(l).to_string()).collect();
            let (key, now) = beeper_core::matrix::inbox_rule(&st.inbox_rules, id, &b.bridge_id, None);
            let at = INBOX_OPTIONS.iter().position(|(o, _)| *o == now).expect("a known inbox visibility");
            let picked = labels[at].clone();
            let pick = select(p, labels.clone(), picked, move |l| {
                let i = labels.iter().position(|x| *x == l).expect("a listed option");
                UiMessage::Accounts(accounts::Msg::SetInboxVisibility(key.clone(), INBOX_OPTIONS[i].0))
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
        // Desktop's `AccountDisconnected` describes an error without a message as a problem, with its state and code.
        let disconnected = bridge.and_then(|b| Some((b, b.remote_error.as_deref()?))).map(|(b, message)| {
            let line = |s: String| txt(s, 11.0).color(p.strong).center().into();
            let message: Vec<El<'_>> = match message {
                "" => vec![
                    line(tr("This account is having a problem, please contact support.").into()),
                    line(format!("{}: {}", b.state, b.remote_error_code.as_deref().unwrap_or_default())),
                ],
                m => vec![line(m.to_string())],
            };
            let again = again.map(|m| {
                button(med(tr("Reconnect"), 11.0)).padding([4, 8]).style(btn_style([Color::WHITE; 3], [p.accent; 3], 20.0)).on_press(m)
            });
            let banner = column![
                svg(ERROR_CIRCLE.clone()).width(20).height(20),
                semi(tr("Account Disconnected"), 13.0).color(p.strong),
                Column::with_children(message).spacing(11).align_x(Center),
            ]
            .push(again)
            .spacing(8)
            .align_x(Center);
            let fill = if self.dark { p.prefs_group } else { p.chip };
            container(banner).width(Fill).padding([12, 8]).style(fill_box(fill, 8.0)).into()
        });
        // Desktop's `AccountDeleteButton`.
        let remove = bridge.map(|_| {
            let label = semi(tr("Remove Account"), 13.0).color(ON_ERROR).width(Fill).center();
            button(label)
                .bare(ON_ERROR)
                .padding([5, 0])
                .width(Fill)
                .on_press(UiMessage::Accounts(accounts::Msg::AskRemove(id.into())))
                .into()
        });
        let settings = group(p, Some(tr("Settings")), rows.collect());
        let servers = bridge.and_then(|b| self.servers_view(st, b));
        [Some(head.into()), disconnected, Some(card.into()), Some(settings), servers, remove].into_iter().flatten().collect()
    }

    /// Desktop's `RemoteSubView` of an account with servers: the "Servers" heading with Refresh List, and each server
    /// with its Show in Inbox and its connect switch. It shows once the bridge has sent servers.
    fn servers_view(&self, st: &BeeperStore, b: &BridgeAccountInfo) -> Option<El<'_>> {
        let servers = st.sub_accounts.get(&b.account_id).filter(|l| !l.is_empty() && accounts::has_servers(st, &b.account_id))?;
        let p = &self.pal;
        let id = b.account_id.clone();
        // Desktop's `remoteHasError` locks the switches.
        let broken = b.remote_error.is_some();
        // `RemoteSubView-module__spin`: a turn back every 1.2 s with `cubic-bezier(0.5, 0, 0.5, 1)`.
        let turn = self.servers_loading.get(&id).map_or(0.0, |at| {
            let t = at.elapsed().as_secs_f32() / 1.2 % 1.0;
            -crate::cubic_bezier(t, [0.5, 0.0, 0.5, 1.0]) * std::f32::consts::TAU
        });
        let color = p.text;
        let icon = svg(crate::MAT_SYNC.clone())
            .width(20)
            .height(20)
            .rotation(iced::Rotation::Solid(iced::Radians(turn)))
            .style(move |_, _| svg::Style { color: Some(color) });
        let refresh = button(icon).bare(p.text).padding(0).on_press(UiMessage::Accounts(accounts::Msg::FetchServers(id.clone())));
        // `.subview-prefs h4` beside the refresh button, in `RemoteSubView-module__header`.
        let head = row![container(txt(tr("Servers"), 13.0).color(p.muted)).padding([0, 13]).width(Fill), refresh].align_y(Center);
        let options = [("all", "All Messages"), ("none", "None")];
        let labels: Vec<String> = options.iter().map(|&(_, l)| inbox_label(l).to_string()).collect();
        let widest = labels.iter().map(|l| text_width(l, 11.0, Weight::Semibold)).fold(0.0, f32::max);
        let rows = servers.iter().map(|s| {
            let (key, now) = beeper_core::matrix::inbox_rule(&st.inbox_rules, &id, &b.bridge_id, Some(&s.id));
            let at = options.iter().position(|(o, _)| *o == now).expect("all or none");
            let pick = match broken || !s.connected {
                // Chromium's `select:disabled` is 70 % opaque.
                true => {
                    let fade = |c: Color| c.scale_alpha(0.7);
                    let label = semi(labels[at].clone(), 11.0).color(fade(p.strong));
                    let chevron = tinted(&SELECT_ARROW, 10.0, fade(p.strong));
                    let body = row![container(label).width(Fill), chevron].align_y(Center);
                    container(body).width(widest + 34.0).padding([4, 8]).style(fill_box(fade(p.chip), 12.0)).into()
                }
                false => {
                    let labels = labels.clone();
                    select(p, labels.clone(), labels[at].clone(), move |l| {
                        let i = labels.iter().position(|x| *x == l).expect("a listed option");
                        UiMessage::Accounts(accounts::Msg::SetInboxVisibility(key.clone(), options[i].0))
                    })
                }
            };
            let press = (!broken).then(|| UiMessage::Accounts(accounts::Msg::ToggleServer(id.clone(), s.id.clone())));
            let switch = toggle_faded(p, s.connected, press, if broken { 0.5 } else { 1.0 });
            let switch: El<'_> = match broken {
                true => {
                    let why = trf("You can't change %s while the connection is broken", &[&"server"]);
                    self.tip(switch, why, Vec::new(), tooltip::Position::Left).into()
                }
                false => switch,
            };
            // `RemoteSubView-module__toggle`: 10 px from the right edge of the row.
            row![container(pref_row(p, s.name.clone(), None, None, pick)).width(Fill), switch]
                .padding(Padding::ZERO.right(10.0))
                .align_y(Center)
                .into()
        });
        // `RemoteSubView-module__container` is 4 px further from its heading than a `prefs-group`.
        let card = container(group(p, None, rows.collect())).padding(Padding::ZERO.top(4.0));
        Some(column![head, card].spacing(6).into())
    }

    /// A settings item with the id that search results scroll to, and the flash of a picked result.
    fn flashing<'a>(&self, key: &'static str, el: El<'a>) -> El<'a> {
        let flash =
            self.pref_flash.filter(|(k, _)| *k == key).map_or(0.0, |(_, at)| 1.0 - ease_out((at.elapsed().as_secs_f32() / 2.0).min(1.0)));
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
                    (Some(h), t) => out.push(group(p, Some(h), vec![container(txt(t, 13.0).color(p.sub)).padding([13, 14]).into()])),
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
                            let t = txt(t, 13.0).line_height(lh(18.0)).color(p.sub);
                            column![container(t).padding(Padding::new(13.0).top(7.0).bottom(0.0)), g].spacing(19).into()
                        }
                        None => g,
                    })
                }
                // Desktop's Theme group has the Theme Selector and the Theme Color Selector, which flash on their own.
                prefs::Item::Theme => {
                    let cards = self.flashing("Theme Selector", theme_cards(self, s.theme));
                    let colors = self.flashing("Theme Color Selector", self.theme_colors(s.ui_theme));
                    out.push(group(p, heading.take(), vec![cards, colors]))
                }
                prefs::Item::ThemeColor => continue,
                prefs::Item::ChatTheme => out.push(group(p, heading.take(), vec![self.chat_theme_picker(st, None)])),
                prefs::Item::TimeSuggestions => out.push(group(p, heading.take(), vec![self.view_time_suggestions(st)])),
                prefs::Item::Zoom(t, args) => {
                    out.push(group(p, heading.take(), vec![self.zoom_slider(s.zoom_percent, pref_text(s, t, args))]))
                }
            }
            if let Some(name) = custom.filter(|_| !matches!(item, prefs::Item::Theme)) {
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
        let ticks = (0..9).fold(Row::new(), |r, i| if i == 0 { r } else { r.push(Space::new().width(Fill)) }.push(dot()));
        let rail = container(Space::new()).width(Fill).height(4).style(fill_box(fill, 8.0));
        let knob = container(Space::new()).width(20).height(20).style(|_: &Theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: iced::Border { radius: 20.0.into(), ..Default::default() },
            shadow: iced::Shadow { color: Color::BLACK.scale_alpha(0.2), offset: iced::Vector::new(1.0, 1.0), blur_radius: 2.0 },
            ..Default::default()
        });
        let at = (level + 4.0) as u16;
        let knob = row![Space::new().width(Length::FillPortion(at)), knob, Space::new().width(Length::FillPortion(8 - at))];
        let input = slider(-4.0..=4.0, level, |a0| UiMessage::Zoom(zoom::Msg::Drag(a0)))
            .on_release(UiMessage::Zoom(zoom::Msg::Release))
            .height(20)
            .style(|_, _| slider::Style {
                rail: slider::Rail { backgrounds: (CLEAR.into(), CLEAR.into()), width: 0.0, border: iced::Border::default() },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle { radius: 10.0 },
                    background: CLEAR.into(),
                    border_width: 0.0,
                    border_color: CLEAR,
                },
            });
        // Desktop's range input is transparent (`.zoom-slider input { opacity: 0 }`), so its focus does not show;
        // its arrows step the zoom.
        let step = |by: i8| UiMessage::Zoom(zoom::Msg::Step(by));
        let keys = Keys::step((level > -4.0).then(|| step(-1)), (level < 4.0).then(|| step(1)));
        let input = self.target(self.settings_window, "zoom", input, keys, Look::Hidden);
        let track = stack![container(rail).center_y(20), container(ticks).center_y(20), knob, input].width(Length::FillPortion(34));
        let icon = |size, pad: Padding| container(tinted(&ZOOM_LEVEL, size, p.weak)).padding(pad).width(Length::FillPortion(3));
        let (first, last) =
            (icon(14.0, Padding::ZERO.left(8.0)), icon(20.0, Padding::ZERO.right(4.0)).align_x(iced::alignment::Horizontal::Right));
        column![
            row![first, track, last].align_y(Center).padding(16),
            container(txt(desc, 13.0).color(p.sub)).padding(Padding::new(14.0).top(0.0).bottom(13.0))
        ]
        .into()
    }

    /// Desktop's `<select>` of a preference, which takes the focus while it `works`.
    fn pref_select(&self, r: &'static prefs::Pref, s: &BeeperSettings, works: bool) -> El<'_> {
        let prefs::Control::Select(options, _) = r.control else { panic!("{} is not a select", r.key) };
        let key = r.key;
        let values = select_values(key, s);
        let shown: Vec<String> = values
            .iter()
            .map(|&v| options[usize::from(v)])
            .map(|o| match (key, o.split_once(' ')) {
                ("appLanguage", _) => o.to_string(),
                ("chatActionButton", _) => trx(o, "primary thread action option").into(),
                ("chatDoubleClick", _) => trx(o, "thread double-click action option").into(),
                ("primaryComposerButton" | "secondaryComposerButton", _) => trx(o, "composer action option").into(),
                (_, Some((n, unit))) if n.parse::<usize>().is_ok() => duration(n.parse().unwrap(), unit),
                _ => tr(o).into(),
            })
            .collect();
        // A value that is not an option shows the first option, as a `<select>` does.
        let at = values.iter().position(|&v| v == s.pref(key)).unwrap_or(0);
        let picked = shown[at].clone();
        let pick_values = values.clone();
        let set = move |i: usize| UiMessage::Settings(Msg::SetPref(key, pick_values[i]));
        let window = self.settings_window;
        let id = format!("select-{key}");
        // Desktop's `.subview-prefs select:focus-visible` takes `--select-background-focused`: the fill mixed with 10 %
        // black, or white in the dark.
        let mut p = self.pal;
        if self.key_held(window, &id) {
            p.chip = style::mix(p.chip, p.fg, 0.1);
        }
        let pick = {
            let (shown, set) = (shown.clone(), set.clone());
            select(&p, shown.clone(), picked, move |o| set(shown.iter().position(|x| *x == o).expect("listed option")))
        };
        // Chromium's ← ↑ and → ↓ on a closed select pick the option before or after, without wrapping around.
        let keys = Keys::step(at.checked_sub(1).map(&set), (at + 1 < shown.len()).then(|| set(at + 1)));
        // A locked select of a Plus feature is `disabled`, and takes no focus.
        match works {
            true => self.target(window, id, pick, keys, Look::Hidden),
            false => pick,
        }
    }

    fn pref_control(&self, r: &'static prefs::Pref, st: &BeeperStore) -> El<'_> {
        let s = &st.settings;
        let p = &self.pal;
        let locked = r.feature.is_some_and(|f| st.feature(f).locked());
        // Desktop's `onFeatureInteraction` of a locked row opens the Beeper Plus subscription prompt.
        let upsell = UiMessage::Settings(Msg::SelectTab(SettingsTab::BeeperPlus));
        let mut press = None;
        let control = match r.control {
            prefs::Control::Toggle(_) => {
                let off = r.requires.is_some_and(|k| !s.on(k)) || locked;
                press = self.toggle_press(r, st);
                toggle_faded(p, s.pref(r.key) == 1 && !locked, press.clone(), if off { 0.5 } else { 1.0 })
            }
            prefs::Control::ReactionEmoji { .. } => {
                press = Some(UiMessage::QuickEmoji(Some(0)));
                let keys = Keys::press(press.clone());
                self.target(
                    self.settings_window,
                    "reaction-emoji",
                    emoji_button(p, self.quick_glyph(&s.quick_reaction), 0),
                    keys,
                    Look::Hidden,
                )
            }
            prefs::Control::QuickReactions(customized) => {
                let custom = s.on(customized.key);
                let choice = |label: &'static str, on: bool, value: u8| {
                    let dot = container(container(Space::new()).center(6).style(fill_box(if on { Color::WHITE } else { CLEAR }, 3.0)))
                        .center(14)
                        .style(bordered_box(if on { p.accent } else { CLEAR }, if on { p.accent } else { p.sub }, 1.0, 7.0));
                    let label = txt(label, 13.0).color(p.text);
                    button(row![dot, label].spacing(8).align_y(Center))
                        .bare(p.text)
                        .on_press(UiMessage::Settings(Msg::SetPref(customized.key, value)))
                };
                let emojis = s.quick_reactions.iter().zip(1..).map(|(e, slot)| emoji_button(p, self.quick_glyph(e), slot));
                let own = row![choice(trx("Custom", "quick reactions option"), custom, 1)].extend(emojis);
                return column![choice(tr("Use my most frequent emoji"), !custom, 0), own.spacing(8).align_y(Center)]
                    .spacing(8)
                    .padding(Padding::new(13.0).top(0.0))
                    .into();
            }
            prefs::Control::StickerPacks => return self.view_installed_packs(st),
            prefs::Control::Downloads => {
                let pick = UiMessage::Settings(Msg::PickDownloads);
                let change = button(med(tr("Change"), 12.0).color(p.text)).padding([4, 10]).style(pill_style(p)).on_press(pick.clone());
                let change = self.target(self.settings_window, "downloads", change, Keys::press(pick), Look::Hidden);
                let dir = downloads_dir(s).display().to_string();
                return pref_row(p, tr(r.label), Some(&dir), None, change);
            }
            prefs::Control::BlockedUsers => {
                press = Some(UiMessage::Blocking(blocking::Msg::OpenList));
                let n = txt(blocked_chats(st).len().to_string(), 13.0).color(p.muted);
                row![n, ic(icon::CHEVRON_RIGHT, 16.0).color(p.muted)].spacing(2).align_y(Center).into()
            }
            prefs::Control::Analytics => {
                press = Some(UiMessage::Settings(Msg::ShareAnalytics));
                toggle_faded(p, !st.whoami.analytics_disabled, press.clone(), 1.0)
            }
            prefs::Control::Select(..) if locked => {
                let cover = mouse_area(Space::new().width(Fill).height(Fill)).on_press(upsell);
                stack![self.pref_select(r, s, false), iced::widget::opaque(cover)].into()
            }
            prefs::Control::Select(..) => self.pref_select(r, s, true),
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
                    false if self.mics.is_some() && !names.contains(chosen) => tr("Selected microphone unavailable").into(),
                    false => chosen.clone(),
                };
                let options: Vec<String> = std::iter::once(same.clone()).chain(names.iter().cloned()).collect();
                let pick =
                    select(p, options, shown, move |o| UiMessage::Settings(Msg::SetMicrophone(if o == same { String::new() } else { o })));
                let desc = matches!(self.mics, Some(Err(_))).then_some(tr("Unable to load microphones."));
                return pref_row(p, tr(r.label), desc, None, pick);
            }
            prefs::Control::Slider(_) => {
                assert!(r.desc.is_none() && r.badge.is_none(), "{} is a plain slider", r.key);
                let key = r.key;
                // Desktop's range input from 0 to 1 in steps of 0.1.
                let value = self.settings_slider.filter(|(k, _)| *k == key).map_or(s.pref(key), |(_, v)| v);
                let bar = slider(0..=100u8, value, move |x| UiMessage::Settings(Msg::SlideTo(key, x)))
                    .on_release(UiMessage::Settings(Msg::SlideEnd))
                    .step(10u8)
                    .height(16)
                    .style(slider_style(p));
                // Chromium's range input: its arrows step it, and it shows the `outline: auto` ring.
                let set = |v: u8| UiMessage::Settings(Msg::SetPref(key, v));
                let keys = Keys::step(value.checked_sub(10).map(set), (value < 100).then(|| set(value + 10)));
                let bar = self.target(self.settings_window, format!("slider-{key}"), bar, keys, Look::Auto);
                let body = column![txt(tr(r.label), 13.0).line_height(lh(18.0)).color(p.text), container(bar).padding([0, 2])]
                    .spacing(13)
                    .padding(Padding::new(13.0).bottom(14.0));
                return column![body, container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0))].into();
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
                    (left, rest) = (left.push(words).push(self.pref_select(pick, s, true)), tail);
                }
                let left = left
                    .push((!rest.trim().is_empty()).then(|| txt(rest.trim().to_string(), 13.0).color(p.text)))
                    .spacing(8)
                    .align_y(Center)
                    .wrap()
                    .vertical_spacing(8);
                // Desktop's `ArchiveAllPreference` counts the chats, none too.
                let label = trnx("Archive %d chat", "Archive %d chats", n, "archive all button", &[&n]);
                let archive = UiMessage::ChatList(chat_list::Msg::ArchiveStale);
                let archive = self.target(
                    self.settings_window,
                    "archive-stale",
                    pill_btn(p, label, archive.clone()),
                    Keys::press(archive),
                    Look::Hidden,
                );
                return row![container(left).width(Fill), archive].spacing(8).align_y(Center).padding(13).into();
            }
        };
        let paid = locked.then(|| tr("Requires a paid subscription."));
        let (label, args) = r.split_args();
        let desc = r.desc.map(|d| [pref_text(&st.settings, d, args).as_str()].into_iter().chain(paid).collect::<Vec<_>>().join("\n\n"));
        let row = pref_row(p, trs(r.label, label), desc.as_deref(), r.badge.map(|b| badge(p, b)), control);
        // Desktop's `label-wrapper` toggles the checkbox on a click anywhere in the row.
        let row = match press.clone() {
            Some(m) => mouse_area(row).on_press(m).into(),
            None => row,
        };
        // Desktop's switch rows, and the button of Blocked users (`.BlockedUsersPref-module__button:focus-visible`
        // takes the same fill).
        match r.control {
            prefs::Control::Toggle(_) | prefs::Control::BlockedUsers => self.focus_row(row, format!("pref-{}", r.key), Keys::press(press)),
            _ => row,
        }
    }

    /// The press of a switch row (Desktop's `WrappedSwitch`): a locked feature opens the Beeper Plus prompt
    /// (`onFeatureInteraction`), and a row whose `requires` is off does nothing.
    fn toggle_press(&self, r: &'static prefs::Pref, st: &BeeperStore) -> Option<UiMessage> {
        if r.feature.is_some_and(|f| st.feature(f).locked()) {
            return Some(UiMessage::Settings(Msg::SelectTab(SettingsTab::BeeperPlus)));
        }
        let off = r.requires.is_some_and(|k| !st.settings.on(k));
        (!off).then_some(UiMessage::Settings(Msg::SetPref(r.key, 1 - st.settings.pref(r.key))))
    }
}

#[derive(Clone)]
pub enum Msg {
    Open(SettingsTab),
    SelectTab(SettingsTab),
    Query(String),
    Focus(usize),
    /// Moves the focused search result down by the given count, wrapping around.
    Step(usize),
    Pick(SettingsHit),
    /// The search field takes (`true`) or loses the keyboard focus.
    SearchFocus(bool),
    /// Desktop's suggestion of the empty search at this index.
    Suggest(usize),
    SelectTheme(ThemeChoice),
    /// Desktop's `localPrefsStore.set(UI_THEME, ...)`: a theme color, or none for Desktop's own colors.
    SelectUiTheme(Option<beeper_core::models::UiTheme>),
    /// Desktop's 36 s `setTimeout`, which moves the rich gradient.
    ThemeTick,
    SystemTheme(iced::theme::Mode),
    SetPref(&'static str, u8),
    /// A slider moves; Desktop saves its value on release (`SlideTo` then `SetPref`).
    SlideTo(&'static str, u8),
    SlideEnd,
    ToggleSetting(fn(&mut BeeperSettings)),
    /// Desktop's "Change" of "Downloads location", and the picked folder.
    PickDownloads,
    DownloadsPicked(Result<Vec<std::path::PathBuf>, String>),
    SetQuickEmoji(u8, String),
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    MicsLoaded(Result<(String, Vec<String>), String>),
    SetMicrophone(String),
    /// Desktop's "Share Analytics" toggle.
    ShareAnalytics,
    /// The server saved `analyticsDisabled`, or failed and the toggle goes back to the first value.
    AnalyticsSaved(bool, Result<(), String>),
    /// "Logout" of another device in Settings > Devices, its confirmation, and the end of the logout.
    AskLogoutDevice(String),
    LogoutDevice(String),
    DeviceLoggedOut(String, Result<(), String>),
    StorageScanned(Vec<storage::Entry>),
    /// The age choice of the clear for all chats (0) or for the open chat (1).
    StorageAge(usize, usize),
    /// Toggles a type, as its index in [`storage::Kind::ALL`], for all chats (0) or for the open chat (1).
    StorageKind(usize, usize),
    StorageChat(String),
    /// Clears the chosen storage of all chats (0), the cache (1) or the open chat (2).
    ClearStorage(u8),
    StorageCleared(Result<(), String>),
    JoinCommunity,
    CommunityJoined(Result<(), String>),
    /// Desktop's `getInstalledImagePacks` of "Installed sticker packs", and its reply.
    LoadPacks,
    Packs(Result<Vec<InstalledPack>, String>),
    /// "Use in all chats" of a pack, by its ID.
    PackInAllChats(String, bool),
    /// The Remove of a bridged pack, its confirmation, and the end of the removal.
    AskRemovePack(String),
    RemovePack(String),
    PackRemoved(String, Result<(), String>),
}

impl BeeperGuiApp {
    pub(crate) fn settings(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::SelectTab(SettingsTab::Help) | Msg::Open(SettingsTab::Help) => {
                return Task::done(UiMessage::OpenUrl("https://help.beeper.com/?ref=desktop-app".into()));
            }
            Msg::SelectTab(t)
                if self.active_settings_tab == SettingsTab::Profile
                    && t != SettingsTab::Profile
                    && self.profile.dirty(self.store.read().me()) =>
            {
                self.profile.leaving = Some(t);
            }
            Msg::SelectTab(t) => {
                let top = operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                self.active_settings_tab = t;
                (self.account_page, self.app_qr) = (None, false);
                if let Some(w) = self.settings_window {
                    self.key_targets.remove(&w);
                }
                // Desktop's `StickerPacksPreference` loads the packs when the General settings show it.
                let packs = match t == SettingsTab::General && self.store.read().settings.on("stickerPacks") {
                    true => self.update(UiMessage::Settings(Msg::LoadPacks)),
                    false => Task::none(),
                };
                return Task::batch([top, self.load_devices(), packs]);
            }
            msg @ (Msg::LoadPacks
            | Msg::Packs(..)
            | Msg::PackInAllChats(..)
            | Msg::AskRemovePack(..)
            | Msg::RemovePack(..)
            | Msg::PackRemoved(..)
            | Msg::DevicesLoaded(..)
            | Msg::AskLogoutDevice(..)
            | Msg::LogoutDevice(..)
            | Msg::DeviceLoggedOut(..)
            | Msg::MicsLoaded(..)
            | Msg::SetMicrophone(..)) => return self.settings_packs(msg),
            Msg::ShareAnalytics => {
                let mut st = self.store.write();
                let was = st.whoami.analytics_disabled;
                st.whoami.analytics_disabled = !was;
                let (cloud, token) = (self.cloud.clone(), st.beeper_state.access_token.clone());
                if !st.config.demo {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || cloud.set_analytics_disabled(&token, !was))
                                .await
                                .expect("analytics worker thread")
                        },
                        move |r| UiMessage::Settings(Msg::AnalyticsSaved(was, r.map_err(|e| e.message))),
                    );
                }
            }
            Msg::AnalyticsSaved(_, Ok(())) => {}
            Msg::AnalyticsSaved(was, Err(e)) => {
                self.store.write().whoami.analytics_disabled = was;
                self.fail(Toast::new(trf("Something went wrong: %s", &[&e])));
            }
            Msg::SystemTheme(mode) => {
                self.system_dark = mode == iced::theme::Mode::Dark;
                self.refresh_theme_and_tray();
            }
            Msg::SelectTheme(choice) => self.patch_settings(|s| s.theme = choice),
            Msg::SelectUiTheme(theme) => self.patch_settings(|s| s.ui_theme = theme),
            Msg::ThemeTick => {
                if let Some(b) = self.pal.backdrop.as_mut().filter(|b| b.rich) {
                    b.offset = crate::ui_theme::rich_offset(beeper_core::config::now_unix_millis());
                }
            }
            Msg::ToggleSetting(flip) => self.patch_settings(flip),
            Msg::SetQuickEmoji(0, e) => {
                self.patch_settings(|s| s.quick_reaction = e);
                let mut st = self.store.write();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::quick_reaction_write(&mut st).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed);
                }
            }
            Msg::SetQuickEmoji(slot, e) => {
                self.patch_settings(|s| s.quick_reactions[usize::from(slot) - 1] = e);
            }
            Msg::PickDownloads => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Folder);
                return Task::perform(pick, |a0| UiMessage::Settings(Msg::DownloadsPicked(a0)));
            }
            Msg::DownloadsPicked(Ok(dirs)) => {
                if let Some(dir) = dirs.into_iter().next() {
                    self.patch_settings(|s| s.downloads_dir = dir.display().to_string());
                }
            }
            Msg::DownloadsPicked(Err(e)) => self.fail(Toast::new(e)),
            msg @ (Msg::StorageScanned(..)
            | Msg::StorageAge(..)
            | Msg::StorageKind(..)
            | Msg::StorageChat(..)
            | Msg::ClearStorage(..)
            | Msg::StorageCleared(..)) => return self.settings_storage(msg),
            Msg::JoinCommunity => {
                if self.store.read().resolve_chat_idx(COMMUNITY_ROOM).is_some() {
                    let close = self.close_window(self.settings_window.expect("the settings window is open"));
                    return Task::batch([self.update(UiMessage::SelectChat(COMMUNITY_ROOM.into())), close]);
                }
                if !self.integrations.joining {
                    self.integrations.joining = true;
                    return Task::perform(beeper_core::matrix::join(self.store.clone(), COMMUNITY_ROOM.into()), |r| {
                        UiMessage::Settings(Msg::CommunityJoined(r.map(drop)))
                    });
                }
            }
            Msg::CommunityJoined(r) => {
                self.integrations.joining = false;
                match r {
                    Ok(()) => {
                        let text = tr("Successfully joined the Beeper community room!").into();
                        self.show(Toast { buttons: vec![(tr("Open"), UiMessage::Settings(Msg::JoinCommunity))], ..Toast::new(text) });
                    }
                    Err(e) => {
                        eprintln!("Failed to join community room: {e}");
                        self.fail(Toast::new(
                            tr("Something went wrong while joining the Beeper Developer Community chat. Please try again.").into(),
                        ));
                    }
                }
            }
            Msg::Open(tab) => {
                self.list.chat_menu = None;
                return self.open_settings(tab);
            }
            Msg::SlideTo(key, v) => self.settings_slider = Some((key, v)),
            Msg::SlideEnd => {
                if let Some((key, v)) = self.settings_slider.take() {
                    return self.update(UiMessage::Settings(Msg::SetPref(key, v)));
                }
            }
            msg @ (Msg::Query(..) | Msg::Focus(..) | Msg::Step(..) | Msg::SearchFocus(..) | Msg::Suggest(..) | Msg::Pick(..)) => {
                return self.settings_search(msg)
            }
            Msg::SetPref(key, v) => {
                if key == "incognito" {
                    let was = u8::from(self.store.read().settings.on(key));
                    let what = if v != 0 { tr("Enable Incognito Mode") } else { tr("Disable Incognito Mode") };
                    self.act(what.into(), UiMessage::Settings(Msg::SetPref(key, was)), None);
                }
                self.patch_settings(|s| s.set_pref(key, v));
                // Desktop shows `StickerPacksPreference` once the sticker packs are on, which loads the packs.
                let packs = match (key, v) {
                    ("stickerPacks", 1) => self.update(UiMessage::Settings(Msg::LoadPacks)),
                    _ => Task::none(),
                };
                self.relaunch_toast = match key {
                    "spellcheck" | "quitOnClose" | "hwAcceleration" | "hiddenEvents" => Some("Preference changed"),
                    "appLanguage" => Some("App language will change after relaunch"),
                    _ => self.relaunch_toast,
                };
                let mut st = self.store.write();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::pref_write(&mut st, key).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed).chain(packs);
                }
                return packs;
            }
        }
        self.settle()
    }

    /// The settings search: its query, keys, suggestions and picks.
    fn settings_search(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Query(q) => {
                let st = self.store.read();
                let n = search_settings(&st, &q).len();
                if n != search_settings(&st, &self.settings_query).len() {
                    self.settings_focus = 0;
                    self.settings_pick = self.settings_pick.filter(|_| n > 0);
                }
                self.settings_query = q;
            }
            Msg::Focus(i) => self.settings_focus = i,
            Msg::Step(step) => {
                let n = self.search_rows();
                self.settings_focus = (self.settings_focus + step) % n.max(1);
                if self.settings_query.trim().is_empty() {
                    return Task::none();
                }
                return locate(format!("hit-{}", self.settings_focus), "settings-nav", crate::Align::Nearest);
            }
            // Desktop's `onFocus` of the search starts at the first row.
            Msg::SearchFocus(on) => {
                if on && !self.settings_search_on {
                    self.settings_focus = 0;
                }
                self.settings_search_on = on;
            }
            // Desktop's `M` of a suggestion: its section with its row highlighted, or Beeper Plus, and the search
            // gives the focus back.
            Msg::Suggest(i) => {
                let Some(&(_, tab, key)) = suggestions(&self.store.read()).get(i) else { return Task::none() };
                if tab == SettingsTab::BeeperPlus {
                    self.settings_query.clear();
                }
                self.settings_search_on = false;
                let open = Task::batch([self.update(UiMessage::Settings(Msg::SelectTab(tab))), crate::focus::blur()]);
                return match key {
                    Some(key) => {
                        self.pref_flash = Some((key, std::time::Instant::now()));
                        Task::batch([open, locate(key, "settings-pane", crate::Align::Center)])
                    }
                    None => open,
                };
            }
            Msg::Pick(hit) => {
                self.settings_pick = Some(hit);
                let tab = match hit {
                    SettingsHit::Section(t, _) | SettingsHit::Pref(t, _) | SettingsHit::Custom(t, _) => t,
                    SettingsHit::Network(_) => SettingsTab::Accounts,
                };
                let load = Task::batch([self.update(UiMessage::Settings(Msg::SelectTab(tab))), operation::focus("settings-search")]);
                return match hit {
                    SettingsHit::Pref(_, &prefs::Pref { key, .. }) | SettingsHit::Custom(_, key) => {
                        self.pref_flash = Some((key, std::time::Instant::now()));
                        Task::batch([load, locate(key, "settings-pane", crate::Align::Center)])
                    }
                    // Desktop's network hit: the account on that network, or the Add Account window for it.
                    SettingsHit::Network(name) => {
                        let st = self.store.read();
                        let net = crate::networks::addable(&st.features).find(|n| n.display_name == name).expect("an addable network");
                        let account = st
                            .bridge_accounts
                            .iter()
                            .find(|a| beeper_core::config::account_network(&a.bridge_id).bridge_id == net.bridge_id);
                        let account = account.map(|a| a.account_id.clone());
                        drop(st);
                        let next = match account {
                            Some(id) => self.update(UiMessage::Accounts(crate::accounts::Msg::Open(Some(id)))),
                            None => self.update(UiMessage::OpenAddAccount(net.bridge_id.into(), None)),
                        };
                        Task::batch([load, next])
                    }
                    SettingsHit::Section(..) => load,
                };
            }
            _ => unreachable!("not a settings_search message"),
        }
        self.settle()
    }

    /// Settings > Storage: the scan, its filters and clearing.
    fn settings_storage(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::StorageScanned(all) => {
                self.storage.chart = Some(donut(&storage_totals(&all)));
                self.storage.all = Some(all);
            }
            Msg::StorageAge(which, i) => self.storage.ages[which] = i,
            Msg::StorageKind(which, i) => self.storage.kinds[which] ^= 1 << i,
            Msg::StorageChat(id) => {
                let s = &mut self.storage;
                (s.chat, s.ages[1], s.kinds[1]) = ((s.chat.as_ref() != Some(&id)).then_some(id), 1, 0b11111);
            }
            Msg::ClearStorage(which) => {
                let mut picked: Vec<storage::Entry> = self.storage_pick(which).into_iter().cloned().collect();
                if which == 0 {
                    picked.extend(self.storage_pick(1).into_iter().cloned());
                }
                (self.confirm, self.storage.clearing) = (None, Some(which));
                return Task::perform(storage::clear(self.store.clone(), picked), |a0| UiMessage::Settings(Msg::StorageCleared(a0)));
            }
            Msg::StorageCleared(r) => {
                self.storage.clearing = None;
                if let Err(e) = r {
                    eprintln!("Unable to clear storage: {e}");
                    self.fail(Toast::new(tr("Failed to clear storage").into()));
                }
                return self.scan_storage();
            }
            _ => unreachable!("not a settings_storage message"),
        }
        self.settle()
    }

    /// Settings > Sticker packs, Devices and the microphone.
    fn settings_packs(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::LoadPacks => {
                return Task::perform(beeper_core::matrix::installed_packs(self.store.clone()), |r| UiMessage::Settings(Msg::Packs(r)));
            }
            Msg::Packs(Err(e)) => eprintln!("StickerPacksPreference: loading packs failed: {e}"),
            Msg::Packs(Ok(packs)) => {
                // Desktop keeps the packs that "Use in all chats" turned off, which leave the installed packs of a
                // personal space, until the settings show again.
                let kept = self.installed_packs.take().into_iter().flatten();
                let kept: Vec<InstalledPack> =
                    kept.filter(|k| k.parts.iter().any(|p| self.packs_off.contains(p)) && !packs.iter().any(|n| n.id == k.id)).collect();
                self.installed_packs = Some(packs.into_iter().chain(kept).collect());
            }
            Msg::PackInAllChats(id, on) => {
                let Some(pack) = self.installed_packs.iter().flatten().find(|p| p.id == id).cloned() else {
                    return self.settle();
                };
                for part in &pack.parts {
                    match on {
                        true => _ = self.packs_off.remove(part),
                        false => _ = self.packs_off.insert(part.clone()),
                    }
                }
                let mut st = self.store.write();
                let rooms = beeper_core::matrix::with_pack_rooms(st.image_pack_rooms.clone(), &pack.parts, on);
                st.image_pack_rooms = rooms.clone();
                let client = st.matrix.clone();
                drop(st);
                // Desktop's `imagePackStore.revision` loads the packs again after the account data changes.
                let reload = self.update(UiMessage::Settings(Msg::LoadPacks));
                let Some(c) = client else { return reload };
                let write = Task::perform(beeper_core::matrix::set_global(c, beeper_core::matrix::IMAGE_PACK_ROOMS, rooms), |r| {
                    if let Err(e) = &r {
                        eprintln!("StickerPacksPreference: saving failed: {e}");
                    }
                    UiMessage::Pushed(r)
                });
                return write.chain(reload);
            }
            Msg::AskRemovePack(id) => (self.confirm, self.delete_target) = (Some(ASK_REMOVE_PACK), Some(id)),
            Msg::RemovePack(id) => {
                self.confirm = None;
                let Some(pack) = self.installed_packs.iter().flatten().find(|p| p.id == id).cloned() else {
                    return self.settle();
                };
                self.pack_removing = Some(id.clone());
                return Task::perform(beeper_core::matrix::remove_pack(self.store.clone(), pack), move |r| {
                    UiMessage::Settings(Msg::PackRemoved(id.clone(), r))
                });
            }
            Msg::PackRemoved(id, r) => {
                self.pack_removing = None;
                match r {
                    Ok(()) => {
                        let parts = self.installed_packs.iter().flatten().find(|p| p.id == id).map(|p| p.parts.clone());
                        let parts = parts.unwrap_or_default();
                        if let Some(packs) = &mut self.installed_packs {
                            packs.retain(|p| !p.parts.iter().any(|x| parts.contains(x)));
                        }
                    }
                    Err(e) => {
                        eprintln!("StickerPacksPreference: removing pack failed: {e}");
                        self.show(Toast::new(tr("Couldn't remove this sticker pack. Try again.").into()));
                    }
                }
            }
            Msg::DevicesLoaded(r) => (self.devices, self.devices_busy) = (Some(r), false),
            Msg::AskLogoutDevice(id) => (self.confirm, self.delete_target) = (Some(ASK_LOGOUT_DEVICE), Some(id)),
            Msg::LogoutDevice(id) => return self.logout_device(id),
            Msg::DeviceLoggedOut(id, r) => return self.device_logged_out(id, r),
            Msg::MicsLoaded(r) => {
                if let Err(e) = &r {
                    eprintln!("Unable to load audio input devices: {e}");
                }
                self.mics = Some(r);
            }
            Msg::SetMicrophone(name) => self.patch_settings(|s| s.microphone = name),
            _ => unreachable!("not a settings_packs message"),
        }
        self.settle()
    }

    /// Desktop's `StickerPacksPreference`: a row for each installed pack with its icon, its name and attribution,
    /// "Use in all chats", and the Remove of a pack that a bridge imported. Nothing shows until the packs load.
    fn view_installed_packs(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let Some(packs) = &self.installed_packs else { return column![].into() };
        if packs.is_empty() {
            let empty = txt(tr("Sticker packs you add from a chat show up here."), 13.0).color(p.sub);
            return container(empty).padding(13).width(Fill).into();
        }
        let rows = packs.iter().map(|pack| {
            let name = match pack.name.as_str() {
                "" => tr("Sticker pack").to_string(),
                n => n.to_string(),
            };
            // `StickerPacksPreference-module__icon`: Desktop's `StickerImage` in 32 px.
            let frames = pack.icon.as_ref().and_then(|(url, _)| self.media_at(url));
            let icon: El<'_> = match frames {
                Some(Ok(frames)) => crate::anim::gif(frames, self.animate.get(), self.started, |f| {
                    iced::widget::image(f).width(32).height(32).content_fit(iced::ContentFit::Contain)
                }),
                _ => Space::new().width(32).height(32).into(),
            };
            let text = column![txt(name.clone(), 13.0).color(p.text).wrapping(iced::widget::text::Wrapping::None)]
                .push(pack.attribution.as_ref().map(|a| txt(a.clone(), 11.0).color(p.weak)))
                .width(Fill)
                .clip(true);
            let lead = row![container(icon).width(32).height(32), text].spacing(12).align_y(Center).width(Fill);
            let rooms = &st.image_pack_rooms;
            let on = pack.parts.iter().all(|(r, k)| beeper_core::matrix::pack_in_all_chats(rooms, r, k));
            let flip = UiMessage::Settings(Msg::PackInAllChats(pack.id.clone(), !on));
            let toggle = row![txt(tr("Use in all chats"), 13.0).color(p.weak), toggle(p, on, Some(flip))].spacing(8).align_y(Center);
            let remove = pack.bridged.then(|| {
                let busy = self.pack_removing.as_ref() == Some(&pack.id);
                let press = (!busy).then(|| UiMessage::Settings(Msg::AskRemovePack(pack.id.clone())));
                button(med(tr("Remove"), 12.0).color(if busy { p.sub } else { p.text }))
                    .padding([4, 10])
                    .style(pill_style(p))
                    .on_press_maybe(press)
            });
            let actions = row![toggle].push(remove).spacing(16).align_y(Center);
            row![lead, actions].spacing(6).align_y(Center).padding(13).into()
        });
        Column::with_children(rows).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator::simulator;
    use serde_json::json;

    fn shows(app: &BeeperGuiApp, text: &str) -> bool {
        simulator(app.view_settings()).find(text).is_ok()
    }

    /// The places of Desktop's `hotkeys-modal` in a 1200 x 800 main window, as Desktop's CSS lays it out: the 24 px
    /// title on its 33.6 px line, the search field, the 18.2 px group heading and the 31.6 px rows.
    #[test]
    fn keyboard_shortcuts_like_desktop() {
        let app = crate::fuzz::demo_app("hotkeys-modal-geometry");
        let size = Size::new(1200.0, 800.0);
        let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.view_hotkeys_modal("", size));
        let mut top = |text: &str| sim.find(text).expect(text).bounds().y;
        let (title, group) = (top("Keyboard Shortcuts"), top("Composer"));
        let (first, second) = (top("Bold Text Selection"), top("Format Text Selection as Inline Code"));
        // Desktop's card is 753.55 px high in the middle of the window, with 13 px of padding.
        let card = (800.0 - 753.55) / 2.0 + 13.0;
        assert!((title - (card + 8.0)).abs() < 0.5, "the title line: {title}");
        assert!((group - (card + 49.6 + 37.95 + 12.8 + 4.8)).abs() < 0.5, "the Composer heading: {group}");
        assert!((first - (group + 18.2 + 4.8 + 4.8 + 3.0)).abs() < 0.5, "the first row: {first}");
        assert!((second - first - 31.6).abs() < 0.5, "a row is 31.6 px: {}", second - first);
    }

    /// Desktop's "Installed sticker packs" under "Use sticker packs": nothing until the packs load, the empty text,
    /// each pack with "Use in all chats" and the Remove of a bridged pack, which asks first. A pack that "Use in all
    /// chats" turned off stays until the settings show again.
    #[tokio::test(flavor = "multi_thread")]
    async fn installed_sticker_packs_like_desktop() {
        use beeper_core::models::InstalledPack;
        let mut app = crate::fuzz::demo_app("settings-sticker-packs");
        let _ = app.update(UiMessage::Settings(Msg::Open(SettingsTab::General)));
        assert!(app.installed_packs.is_none(), "the packs load only with sticker packs on");
        assert!(!shows(&app, "Sticker packs you add from a chat show up here."));
        let load = app.update(UiMessage::Settings(Msg::SetPref("stickerPacks", 1)));
        for m in crate::fuzz::task_messages(load).await {
            let _ = app.update(m);
        }
        assert_eq!(app.installed_packs, Some(Vec::new()), "the demo has no packs");
        assert!(shows(&app, "Sticker packs you add from a chat show up here."));
        let pack = |id: &str, name: &str, bridged: bool| InstalledPack {
            id: format!("m.room.image_pack|!r:x|{id}"),
            name: name.into(),
            bridged,
            parts: vec![("!r:x".into(), id.into())],
            events: vec![("!r:x".into(), "m.room.image_pack".into(), id.into())],
            attribution: bridged.then(|| "Telegram sticker pack".into()),
            ..Default::default()
        };
        let packs = vec![pack("cats", "Cats", true), pack("owls", "", false)];
        let _ = app.update(UiMessage::Settings(Msg::Packs(Ok(packs.clone()))));
        for text in ["Cats", "Telegram sticker pack", "Sticker pack", "Use in all chats", "Remove"] {
            assert!(shows(&app, text), "{text}");
        }
        let _ = app.update(UiMessage::Settings(Msg::PackInAllChats(packs[1].id.clone(), true)));
        let rooms = app.store.read().image_pack_rooms.clone();
        assert_eq!(rooms, json!({ "rooms": { "!r:x": { "owls": {} } } }));
        let _ = app.update(UiMessage::Settings(Msg::PackInAllChats(packs[1].id.clone(), false)));
        assert_eq!(app.store.read().image_pack_rooms, json!({ "rooms": {} }));
        let _ = app.update(UiMessage::Settings(Msg::Packs(Ok(vec![packs[0].clone()]))));
        assert_eq!(app.installed_packs.as_ref().map(Vec::len), Some(2), "the pack turned off stays");
        let _ = app.update(UiMessage::Settings(Msg::AskRemovePack(packs[0].id.clone())));
        assert!(shows(&app, "Remove Cats?") && shows(&app, "This removes the pack from your account on every device."));
        let _ = app.update(UiMessage::Settings(Msg::PackRemoved(packs[0].id.clone(), Err("offline".into()))));
        assert_eq!(app.status_banner, "Couldn't remove this sticker pack. Try again.");
        let _ = app.update(UiMessage::Settings(Msg::PackRemoved(packs[0].id.clone(), Ok(()))));
        assert!(!shows(&app, "Cats"), "the removed pack leaves the list");
        let st = app.store.read();
        let mut list = simulator(app.view_installed_packs(&st));
        assert!(list.find("Sticker pack").is_ok() && list.find("Remove").is_err(), "no Remove without a bridge");
    }

    /// A demo account adds no account: a network that it has no account on, picked in the search of the settings or
    /// in the command palette, opens the Accounts tab and no Add Account window, whose login would ask the Beeper
    /// servers.
    #[test]
    fn a_demo_network_hit_opens_no_add_account_window() {
        let mut app = crate::fuzz::demo_app("settings-demo-network");
        let st = app.store.read();
        let has = |n: &beeper_core::models::NetworkSpec| {
            let mut ids = st.bridge_accounts.iter().map(|a| beeper_core::config::account_network(&a.bridge_id));
            ids.any(|a| a.bridge_id == n.bridge_id)
        };
        let names: Vec<_> = crate::networks::addable(&st.features).filter(|n| !has(n)).map(|n| n.display_name).collect();
        drop(st);
        assert!(names.contains(&"Google Messages"), "the demo has no Google Messages account: {names:?}");
        for name in names {
            let _ = app.update(UiMessage::Settings(Msg::Pick(SettingsHit::Network(name))));
            assert!(app.add_accounts.is_empty(), "no Add Account window for {name}");
            assert_eq!(app.active_settings_tab, SettingsTab::Accounts);
        }
    }

    /// Desktop's `QLt` and `LLt`: the add button and "Add an Account" open the Add an Account window, the accounts
    /// that remain follow the summary, and with none left the `AddMoreAccountsBanner` offers the next plan.
    #[test]
    fn the_accounts_foot_follows_the_plan_like_desktop() {
        let mut app = crate::fuzz::demo_app("settings-accounts-foot");
        let _ = app.update(UiMessage::Settings(Msg::Open(SettingsTab::Accounts)));
        app.store.write().features.summary = json!({
            "current": { "num_accounts_against_limit": 3 },
            "max": { "num_accounts_against_limit": 5 },
        });
        assert!(shows(&app, "2 Free Accounts Remaining"));
        let mut sim = simulator(app.view_settings());
        let _ = sim.click("Add an Account").expect("Add an Account");
        let open = sim.into_messages().any(|m| matches!(m, UiMessage::Networks(crate::networks::Msg::Open)));
        assert!(open, "Add an Account opens its window");
        app.store.write().whoami.plan = Plan::Plus;
        assert!(shows(&app, "2 Accounts Remaining"));
        app.store.write().features.summary["current"]["num_accounts_against_limit"] = json!(5);
        for text in ["Add More Accounts", "0 Accounts Remaining", "Upgrade to Beeper Plus Plus"] {
            assert!(shows(&app, text), "the banner shows {text}");
        }
        assert!(!shows(&app, "Add an Account"), "the banner takes the place of the link");
        app.store.write().whoami.plan = Plan::Free;
        assert!(shows(&app, "0 Free Accounts Remaining") && shows(&app, "Upgrade to Beeper Plus"));
        app.store.write().whoami.plan = Plan::PlusPlus;
        assert!(shows(&app, "Add an Account") && shows(&app, "0 Accounts Remaining"), "Plus Plus has no banner");
        app.store.write().features.summary["max"]["num_accounts_against_limit"] = json!(-1);
        assert!(shows(&app, "Infinite Accounts Remaining"));
    }

    /// Desktop's `AccountDisconnected`: an error without a message is a problem to report, with its state and code.
    #[test]
    fn an_error_without_a_message_asks_for_support_like_desktop() {
        let mut app = crate::fuzz::demo_app("settings-account-problem");
        let _ = app.update(UiMessage::Settings(Msg::Open(SettingsTab::Accounts)));
        let id = {
            let mut st = app.store.write();
            let a = &mut st.bridge_accounts[0];
            (a.state, a.remote_error, a.remote_error_code) = ("UNKNOWN_ERROR".into(), Some(String::new()), Some("panic".into()));
            a.account_id.clone()
        };
        app.account_page = Some(id);
        assert!(shows(&app, "Account Disconnected"));
        assert!(shows(&app, "This account is having a problem, please contact support."));
        assert!(shows(&app, "UNKNOWN_ERROR: panic"));
    }
}

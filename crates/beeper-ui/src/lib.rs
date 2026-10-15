pub mod login;
pub mod style;

use crate::login::{Login, LoginMsg};

use crate::style::{
    bold, bordered_box, brand_svg, btn_style, ell, ensure_fonts_loaded, fill_box, fw, hash_str, hue, ic, icon,
    initials, input_style, med, pick_menu_style, pick_style, scroll_style, semi, shadow_box, txt, Palette, ALL_FONTS,
    CLEAR,
};
use beeper_core::cloud::Cloud;
use beeper_core::config::{account_network, civil_from_days, millis_from_rfc3339, now_unix_secs, APP_VERSION};
use beeper_core::import::has_beeper_data;
use beeper_core::models::*;
use beeper_core::platform::{
    build_tray_icon_rgba, current_original_beeper_data_dir, open_url_or_path_command, DisplayServerKind, OsKind,
    SystemTrayState,
};
use beeper_core::prefs;
use beeper_core::store::{BeeperStore, SharedStore, NOTE_CHAT_ID};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, mouse_area, operation, pick_list, rich_text, row, scrollable, slider, span, stack, svg,
    text_input, Column, Row, Space,
};
use iced::{
    border, mouse, window, Background, Bottom, Center, Color, Element, Event, Fill, Gradient, Padding, Point, Radians,
    Right, Size, Subscription, Task, Theme,
};
use std::sync::Arc;
use tokio::sync::broadcast;

pub type El<'a> = Element<'a, UiMessage>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxTab {
    AllChats,
    LowPriority,
    Archived,
    Reminders,
    Drafts,
    Unread,
    Pinned,
}

impl InboxTab {
    pub const fn label(self) -> &'static str {
        ["Inbox", "Low Priority", "Archive", "Reminders", "Drafts", "Unread", "Pinned"][self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
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

impl SettingsTab {
    pub const ALL: [Self; 12] = [
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

    pub const fn meta(self) -> (&'static str, char, u32) {
        [
            ("Accounts", icon::AT_SIGN, 0x9647e9),
            ("General", icon::SETTINGS, 0x16b2ee),
            ("Appearance", icon::PAINTBRUSH, 0xd736c9),
            ("Notifications", icon::BELL, 0x1fb93d),
            ("Hotkeys", icon::KEYBOARD, 0xeaa900),
            ("Devices", icon::SMARTPHONE, 0xf54d0b),
            ("AI", icon::SPARKLES, 0x1fc8d3),
            ("Beeper Plus", icon::PLUS, 0x2a61fb),
            ("Integrations", icon::PUZZLE, 0x7459ff),
            ("Storage", icon::DATABASE, 0x35556a),
            ("Help", icon::CIRCLE_HELP, 0x9a9aa1),
            ("About", icon::INFO, 0x20b4a1),
        ][self as usize]
    }

    fn for_section(section: &str) -> Self {
        match section {
            "General" => Self::General,
            "Appearance" => Self::Appearance,
            "Notifications" => Self::Notifications,
            other => panic!("settings section {other} has no tab"),
        }
    }
}

#[derive(Clone)]
pub enum UiMessage {
    Noop,
    SelectTab(InboxTab),
    SelectAccount(Option<String>),
    SelectChat(String),
    ComposerChanged(String),
    SubmitComposer,
    CancelComposeMode,
    StartReply(String),
    StartEdit(String, String),
    DeleteMessage(String, String),
    ToggleReaction(String, String, String),
    ToggleBookmark(String),
    MarkUnreadFromHere(String, String),
    RetranscribeVoice(String),
    VotePoll(String, String),
    SendQuickVoiceNote,
    SendQuickPoll,
    ToggleAttachMenu,
    SetReminder(String),
    TogglePinChat(String),
    ToggleMuteChat(String),
    ToggleArchiveChat(String),
    ArchiveStale,
    NotifyAnyway(String),
    NicknameChanged(String),
    SaveNickname,
    ToggleSearch,
    ToggleFilterMenu,
    SearchChanged(String),
    ClearSearch,
    ToggleCommandPalette,
    CommandQueryChanged(String),
    ExecuteCommandChat(String),
    OpenSettings(SettingsTab),
    SelectSettingsTab(SettingsTab),
    SettingsQueryChanged(String),
    ToggleRightPanel,
    ToggleDnd,
    CycleTheme,
    SelectTheme(ThemeChoice),
    SystemTheme(iced::theme::Mode),
    SetPref(&'static str, u8),
    Zoom(i8),
    ToggleSetting(fn(&mut BeeperSettings)),
    CloudAccounts(Result<Vec<BridgeAccountInfo>, String>),
    RecoveryInputChanged(String),
    VerifyRecoveryCode,
    VerifySasNow,
    ClearOldMedia,
    ExportChatJson(String),
    OpenQuickLook(String, String),
    OpenUrl(String),
    CloseModals,
    ToggleEditHistory(String),
    HoverEnter(String),
    HoverExit(String),
    DragStart,
    DragMoved(Point),
    DragEnd,
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    ScreenshotCaptured(window::Id, window::Screenshot),
    KeyPressed(Key, Modifiers),
    StoreChanged,
    FocusRequested(Option<String>),
    Login(LoginMsg),
}

struct StoreFeed(SharedStore, u8);

impl std::hash::Hash for StoreFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.1.hash(state);
    }
}

fn feed<T: Clone + Send + 'static>(
    rx: broadcast::Receiver<T>,
    f: fn(T) -> UiMessage,
) -> impl iced::futures::Stream<Item = UiMessage> {
    iced::futures::stream::unfold(rx, move |mut rx| async move {
        match rx.recv().await {
            Ok(v) => Some((f(v), rx)),
            Err(broadcast::error::RecvError::Lagged(_)) => Some((UiMessage::StoreChanged, rx)),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    })
}

impl std::fmt::Debug for UiMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UiMessage")
    }
}

pub fn search_settings_catalog(query: &str) -> Vec<(&'static str, &'static prefs::Pref)> {
    let q = query.trim().to_lowercase();
    prefs::all()
        .filter(|(page, r)| [r.label, r.desc.unwrap_or(""), page].iter().any(|s| s.to_lowercase().contains(&q)))
        .collect()
}

const ZOOM_STEPS: [u16; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const HOTKEYS: [(&str, &str); 7] = [
    ("Command bar", "Ctrl K"),
    ("Open settings", "Ctrl ,"),
    ("Search chats", "Ctrl F"),
    ("Toggle chat info", "Ctrl ."),
    ("Next chat", "Alt ↓"),
    ("Previous chat", "Alt ↑"),
    ("Close popups", "Esc"),
];

fn network_name(account_id: &str) -> &'static str {
    account_network(account_id).display_name
}

fn brand_key(account_id: &str) -> &'static str {
    account_network(account_id).bridge_id
}

pub fn format_short_ts(iso: &str, twenty_four: bool) -> String {
    let ms = millis_from_rfc3339(iso).unwrap_or_else(|| panic!("timestamp {iso} is not RFC 3339"));
    let mins = ms.div_euclid(60_000).rem_euclid(1440);
    let (h, m) = (mins / 60, format!("{:02}", mins % 60));
    if twenty_four {
        return format!("{h:02}:{m}");
    }
    format!("{}:{m} {}", (h + 11) % 12 + 1, ["AM", "PM"][usize::from(h >= 12)])
}

fn day_num(iso: &str) -> i64 {
    millis_from_rfc3339(iso).unwrap_or_else(|| panic!("timestamp {iso} is not RFC 3339")).div_euclid(86_400_000)
}

fn day_label(iso: &str) -> String {
    let (d, today) = (day_num(iso), now_unix_secs().div_euclid(86_400));
    let wd = WEEKDAYS[(d + 4).rem_euclid(7) as usize];
    match today - d {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => wd.into(),
        _ => {
            let ((y, m, dd), ty) = (civil_from_days(d), civil_from_days(today).0);
            let year = if y == ty { String::new() } else { format!(", {y}") };
            format!("{}, {} {dd}{year}", &wd[..3], MONTHS[m as usize - 1])
        }
    }
}

fn list_ts(iso: &str, h24: bool) -> String {
    let d = day_num(iso);
    match now_unix_secs().div_euclid(86_400) - d {
        0 => format_short_ts(iso, h24),
        1 => "Yesterday".into(),
        2..=6 => WEEKDAYS[(d + 4).rem_euclid(7) as usize][..3].into(),
        _ => {
            let (y, m, dd) = civil_from_days(d);
            format!("{m}/{dd}/{:02}", y % 100)
        }
    }
}

fn human_size(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.0} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

fn ghost(p: &Palette, r: f32) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    btn_style([CLEAR, p.hover, p.selected], [p.text; 3], r)
}

fn icon_btn<'a>(p: &Palette, c: char, size: f32, frame: f32, color: Color, msg: UiMessage) -> El<'a> {
    button(container(ic(c, size).color(color)).center(frame)).padding(0).style(ghost(p, 8.0)).on_press(msg).into()
}

fn count_badge<'a>(n: u32, bg: Color) -> El<'a> {
    container(semi(n.to_string(), 10.0).color(Color::WHITE)).padding([1, 5]).style(fill_box(bg, 9.0)).into()
}

fn closable_head<'a>(p: &Palette, title: &str, size: f32, msg: UiMessage) -> El<'a> {
    row![
        semi(title.to_string(), size).color(p.strong),
        Space::new().width(Fill),
        icon_btn(p, icon::CLOSE, 14.0, 26.0, p.sub, msg)
    ]
    .align_y(Center)
    .into()
}

fn divider<'a>(p: &Palette, left: f32) -> El<'a> {
    container(container(Space::new()).width(Fill).height(1).style(fill_box(p.divider.scale_alpha(0.4), 0.0)))
        .padding(Padding::ZERO.left(left))
        .into()
}

fn self_avatar<'a>(name: &str, size: f32) -> El<'a> {
    let g = Linear::new(Radians(std::f32::consts::PI))
        .add_stop(0.0, Color::from_rgb8(224, 170, 140))
        .add_stop(1.0, Color::from_rgb8(186, 116, 84));
    container(semi(initials(name).chars().take(1).collect::<String>(), size * 0.42).color(Color::WHITE))
        .center(size)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(g))),
            border: iced::border::rounded(size / 2.0),
            ..Default::default()
        })
        .into()
}

fn avatar<'a>(title: &str, size: f32) -> El<'a> {
    let label = initials(title).chars().take(1).collect::<String>();
    container(semi(label, size * 0.42).color(Color::WHITE))
        .center(size)
        .style(fill_box(hue(hash_str(title)), size / 2.0))
        .into()
}

fn chat_avatar<'a>(chat: &Chat, me: &str, size: f32) -> El<'a> {
    if chat.id == NOTE_CHAT_ID {
        self_avatar(me, size)
    } else {
        avatar(&chat.title, size)
    }
}

fn badged<'a>(p: &Palette, inner: El<'a>, account_id: &str, size: f32, badge: f32) -> El<'a> {
    let ring = container(svg(brand_svg(brand_key(account_id))).width(badge).height(badge))
        .padding(1.5)
        .style(fill_box(p.frame, badge));
    let box_ = size + badge / 3.0;
    stack![inner, container(ring).width(box_).height(box_).align_right(box_).align_bottom(box_)]
        .width(box_)
        .height(box_)
        .into()
}

fn slider_style(p: &Palette) -> impl Fn(&Theme, slider::Status) -> slider::Style + use<> {
    let (on, off) = (p.accent, p.toggle_off);
    move |_, _| slider::Style {
        rail: slider::Rail { backgrounds: (on.into(), off.into()), width: 4.0, border: border::rounded(2) },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 8.0 },
            background: Color::WHITE.into(),
            border_width: 1.0,
            border_color: off,
        },
    }
}

fn toggle<'a>(p: &Palette, on: bool, msg: Option<UiMessage>) -> El<'a> {
    let knob =
        container(Space::new()).width(14).height(14).style(fill_box(if on { p.toggle_on } else { p.toggle_off }, 7.0));
    let track =
        container(knob).width(36).height(24).padding(5).align_y(Center).align_x(if on { Right } else { iced::Left });
    button(track.style(bordered_box(CLEAR, p.toggle_off, 2.0, 12.0)))
        .padding(0)
        .style(btn_style([CLEAR; 3], [p.text; 3], 12.0))
        .on_press_maybe(msg)
        .into()
}

fn select<'a, T: ToString + PartialEq + Clone + 'a>(
    p: &Palette,
    options: impl std::borrow::Borrow<[T]> + 'a,
    selected: T,
    f: impl Fn(T) -> UiMessage + 'a,
) -> El<'a> {
    let chevron = pick_list::Icon {
        font: iced::Font::with_name("icons"),
        code_point: icon::EXPAND_MORE,
        size: Some(14.0.into()),
        line_height: iced::widget::text::LineHeight::Relative(1.0),
        shaping: iced::widget::text::Shaping::Basic,
    };
    pick_list(options, Some(selected), f)
        .text_size(12)
        .font(fw(Weight::Semibold))
        .handle(pick_list::Handle::Static(chevron))
        .padding([5, 10])
        .style(pick_style(p.hover, p.selected, p.text, p.sub))
        .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border))
        .into()
}

fn pill_btn<'a>(p: &Palette, label: impl ToString, msg: UiMessage) -> El<'a> {
    button(med(label.to_string(), 13.0).color(p.text).wrapping(iced::widget::text::Wrapping::None))
        .padding([5, 12])
        .style(btn_style([p.hover, p.selected, p.selected], [p.text; 3], 6.0))
        .on_press(msg)
        .into()
}

fn badge<'a>(p: &Palette, b: prefs::Badge) -> El<'a> {
    let label = match b {
        prefs::Badge::Plus => "+ Plus",
        prefs::Badge::Labs => "🧪 Labs",
    };
    container(semi(label, 11.0).color(p.strong)).padding([2, 7]).style(fill_box(p.outline, 9.0)).into()
}

const THEME_WINDOW: &str = r##"<rect x="10" y="7" width="76" height="47" rx="4" fill="WIN"/><circle cx="15" cy="12" r="1.4" fill="#ff5f57"/><circle cx="19.5" cy="12" r="1.4" fill="#febc2e"/><circle cx="24" cy="12" r="1.4" fill="#28c840"/><rect x="36" y="16" width="24" height="3" rx="1.5" fill="LINE"/><rect x="14" y="23" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="28" width="68" height="11" rx="2.5" fill="#0d4ffb"/><circle cx="19.5" cy="33.5" r="3" fill="#fff"/><rect x="25" y="31" width="28" height="1.8" rx=".9" fill="#fff"/><rect x="25" y="34.6" width="48" height="1.5" rx=".75" fill="#bcd2ff"/><rect x="14" y="43" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="48" width="48" height="2" rx="1" fill="LINE"/>"##;

static THEME_ART: std::sync::LazyLock<[svg::Handle; 3]> = std::sync::LazyLock::new(|| {
    let light = THEME_WINDOW.replace("WIN", "#f5f5f7").replace("LINE", "#d9d9de");
    let dark = THEME_WINDOW.replace("WIN", "#1f1f23").replace("LINE", "#3a3a41");
    let svg_of = |body: String| {
        let doc = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 61"><defs><linearGradient id="l" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#36c5f4"/><stop offset="1" stop-color="#6b4ff0"/></linearGradient><linearGradient id="d" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7a2fe0"/><stop offset="1" stop-color="#160c2e"/></linearGradient><clipPath id="r"><path d="M54 0h42v61H38z"/></clipPath></defs>{body}</svg>"##
        );
        svg::Handle::from_memory(doc.into_bytes())
    };
    [
        svg_of(format!(r##"<rect width="96" height="61" rx="6" fill="url(#l)"/>{light}"##)),
        svg_of(format!(r##"<rect width="96" height="61" rx="6" fill="url(#d)"/>{dark}"##)),
        svg_of(format!(
            r##"<rect width="96" height="61" rx="6" fill="url(#l)"/>{light}<g clip-path="url(#r)"><rect width="96" height="61" rx="6" fill="url(#d)"/>{dark}</g>"##
        )),
    ]
});

fn theme_cards<'a>(p: &Palette, current: ThemeChoice) -> El<'a> {
    let cards = ThemeChoice::ALL.iter().zip(THEME_ART.iter()).map(|(&t, art)| {
        let on = t == current;
        let edge = if on { p.accent } else { CLEAR };
        let img =
            container(svg(art.clone()).width(96).height(61)).padding(2).style(bordered_box(CLEAR, edge, 2.0, 8.0));
        let fg = if on { Color::WHITE } else { p.strong };
        let label = container(txt(t.label(), 13.0).color(fg)).padding([3, 10]).style(fill_box(edge, 12.0));
        button(column![img, label].spacing(6).align_x(Center))
            .padding(Padding { top: 7.0, right: 13.0, bottom: 9.0, left: 13.0 })
            .style(btn_style([CLEAR, p.hover, CLEAR], [p.strong; 3], 8.0))
            .on_press_maybe((!on).then_some(UiMessage::SelectTheme(t)))
            .into()
    });
    container(Row::with_children(cards).spacing(4)).center_x(Fill).padding([14, 0]).into()
}

fn pref_row<'a>(
    p: &Palette,
    title: impl ToString,
    desc: Option<&str>,
    extra: Option<El<'a>>,
    control: El<'a>,
) -> El<'a> {
    let head =
        row![txt(title.to_string(), 13.0).color(p.text)].push(extra).push(Space::new().width(Fill)).push(control);
    column![head.spacing(6).align_y(Center)]
        .push(desc.map(|d| txt(d.to_string(), 13.0).color(p.sub)))
        .spacing(8)
        .padding([13, 14])
        .into()
}

fn group<'a>(p: &Palette, title: Option<&str>, rows: Vec<El<'a>>) -> El<'a> {
    let card = container(Column::with_children(rows)).width(Fill).style(bordered_box(
        p.prefs_group,
        p.prefs_border,
        1.0,
        12.0,
    ));
    column![]
        .push(title.map(|t| container(med(t.to_string(), 12.0).color(p.weak)).padding([0, 4])))
        .push(card)
        .spacing(8)
        .into()
}

pub struct BeeperGuiApp {
    pub store: SharedStore,
    pub os: OsKind,
    pub display_server: DisplayServerKind,
    pub tray: SystemTrayState,
    pub system_dark: bool,
    pub dark: bool,
    pub zoom: f32,
    pub pal: Palette,
    pub active_tab: InboxTab,
    pub selected_account_filter: Option<String>,
    pub selected_chat_id: Option<String>,
    pub search_query: String,
    pub show_search: bool,
    pub show_filter_menu: bool,
    pub show_attach_menu: bool,
    pub command_palette_query: String,
    pub composer_text: String,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub nickname_edit_buffer: String,
    pub recovery_code_input: String,
    pub show_command_palette: bool,
    pub active_settings_tab: SettingsTab,
    pub settings_query: String,
    pub show_right_panel: bool,
    pub quick_look_target: Option<(String, String)>,
    pub expanded_history_msg_id: Option<String>,
    pub hovered_msg: Option<String>,
    pub status_banner: String,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    pub dragging: bool,
    pub main_window: Option<window::Id>,
    pub settings_window: Option<window::Id>,
    pub login_window: Option<window::Id>,
    pub login: Login,
    pub cloud: Arc<Cloud>,
}

impl BeeperGuiApp {
    pub fn new(store: SharedStore, try_native_tray: bool) -> Self {
        ensure_fonts_loaded();
        let os = OsKind::current();
        let display_server = os.detect_display_server(&std::env::vars().collect());
        let (theme, unread, panel, first_chat, cloud, source) = {
            let st = store.read().unwrap();
            let first = st.signed_in().then(|| NOTE_CHAT_ID.to_string());
            let s = &st.settings;
            let cloud = Arc::new(Cloud::new(&st.config.api_url, &st.config.homeserver));
            let source = st
                .setup
                .is_none()
                .then(|| current_original_beeper_data_dir(st.config.profile.as_deref()))
                .filter(|d| has_beeper_data(d));
            (s.theme.is_dark(false), st.total_unread_chats(), false, first, cloud, source)
        };
        let mut app = Self {
            store,
            os,
            display_server,
            tray: SystemTrayState::new(unread, false, try_native_tray),
            pal: crate::style::palette(theme),
            system_dark: false,
            dark: theme,
            zoom: 1.0,
            active_tab: InboxTab::AllChats,
            selected_account_filter: None,
            selected_chat_id: None,
            search_query: String::new(),
            show_search: false,
            show_filter_menu: false,
            show_attach_menu: false,
            command_palette_query: String::new(),
            composer_text: String::new(),
            reply_to_message_id: None,
            editing_message_id: None,
            nickname_edit_buffer: String::new(),
            recovery_code_input: String::new(),
            show_command_palette: false,
            active_settings_tab: SettingsTab::General,
            settings_query: String::new(),
            show_right_panel: panel,
            quick_look_target: None,
            expanded_history_msg_id: None,
            hovered_msg: None,
            status_banner: String::new(),
            dnd_muted: false,
            sidebar_width: 190.0,
            dragging: false,
            main_window: None,
            settings_window: None,
            login_window: None,
            login: Login::new(source),
            cloud,
        };
        if let Some(id) = first_chat {
            app.select_chat(&id);
        }
        app
    }

    pub fn boot(store: SharedStore) -> (Self, Task<UiMessage>) {
        let mut app = Self::new(store, true);
        let signed_in = app.selected_chat_id.is_some();
        let settings =
            if signed_in { Self::window_settings(Size::new(1264.0, 893.0)) } else { Self::login_window_settings() };
        let (id, open) = window::open(settings);
        *(if signed_in { &mut app.main_window } else { &mut app.login_window }) = Some(id);
        let cloud = (signed_in && !app.store.read().unwrap().config.demo).then(|| app.refresh_cloud_accounts());
        let tasks = [open.map(UiMessage::WindowOpened), iced::system::theme().map(UiMessage::SystemTheme)];
        (app, Task::batch(tasks.into_iter().chain(cloud)))
    }

    fn window_settings(size: Size) -> window::Settings {
        window::Settings {
            size,
            min_size: Some(Size::new(640.0, 480.0)),
            icon: Some({
                let (rgba, w, h) = build_tray_icon_rgba(0, false);
                window::icon::from_rgba(rgba, w, h).expect("the app icon is valid RGBA")
            }),
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: beeper_core::config::APP_BUNDLE_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn open_settings(&mut self, tab: SettingsTab) -> Task<UiMessage> {
        self.active_settings_tab = tab;
        if let Some(id) = self.settings_window {
            return window::gain_focus(id);
        }
        let (id, open) = window::open(Self::window_settings(Size::new(793.0, 705.0)));
        self.settings_window = Some(id);
        open.map(UiMessage::WindowOpened)
    }

    pub fn title(&self, id: window::Id) -> String {
        if Some(id) == self.settings_window { "Settings" } else { "Beeper" }.into()
    }

    pub fn iced_theme(&self) -> Theme {
        if self.dark {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    pub fn refresh_theme_and_tray(&mut self) {
        let st = self.store.read().unwrap();
        self.dark = st.settings.theme.is_dark(self.system_dark);
        self.pal = crate::style::palette(self.dark);
        self.zoom = f32::from(st.settings.zoom_percent) / 100.0;
        self.tray.update(st.total_unread_chats(), self.dnd_muted);
    }

    pub fn visible_chats<'a>(&self, st: &'a BeeperStore) -> Vec<&'a Chat> {
        let q_ids =
            (!self.search_query.trim().is_empty()).then(|| st.chat_search_index.query_all_tokens(&self.search_query));
        let mut list: Vec<&Chat> = st
            .chats
            .iter()
            .enumerate()
            .filter(|(idx, c)| {
                c.merged_into_chat_id.is_none()
                    && self.selected_account_filter.as_ref().is_none_or(|a| &c.account_id == a)
                    && q_ids.as_ref().is_none_or(|ids| ids.binary_search(&(*idx as u32)).is_ok())
                    && match self.active_tab {
                        InboxTab::AllChats => c.flags & (CHAT_ARCHIVED | CHAT_LOW_PRIORITY) == 0,
                        InboxTab::Unread => c.flags & CHAT_UNREAD != 0,
                        InboxTab::Pinned => c.flags & CHAT_PINNED != 0,
                        InboxTab::Drafts => c.draft.is_some(),
                        InboxTab::Archived => c.flags & CHAT_ARCHIVED != 0,
                        InboxTab::LowPriority => c.flags & CHAT_LOW_PRIORITY != 0,
                        InboxTab::Reminders => c.reminder.is_some(),
                    }
            })
            .map(|(_, c)| c)
            .collect();
        list.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        list
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        if let Some(prev_id) =
            self.selected_chat_id.clone().filter(|p| p != chat_id && self.editing_message_id.is_none())
        {
            let draft = self.composer_text.trim().to_string();
            let draft = (!draft.is_empty()).then(|| ChatDraft {
                text: Some(draft),
                reply_to_message_id: self.reply_to_message_id.clone(),
                attachments: Vec::new(),
            });
            let mut st = self.store.write().unwrap();
            st.patch_chat(&prev_id, true, None, None).expect("mark previous chat read");
            if draft.is_some() {
                st.patch_chat(&prev_id, false, draft, None).expect("save draft");
            }
        }
        let mut st = self.store.write().unwrap();
        let idx = st.resolve_chat_idx(chat_id).expect("selected chat exists");
        self.selected_chat_id = Some(chat_id.to_string());
        let chat = st.update_chat(idx, |c| {
            c.unread_count = 0;
            c.is_marked_unread = Some(false);
        });
        self.composer_text = chat.draft.as_ref().and_then(|d| d.text.clone()).unwrap_or_default();
        self.nickname_edit_buffer = chat
            .participants
            .items
            .iter()
            .find(|u| u.is_self != Some(true))
            .and_then(|u| u.custom_nickname.clone().or_else(|| u.full_name.clone()))
            .unwrap_or_default();
        self.reply_to_message_id = None;
        self.editing_message_id = None;
        drop(st);
        self.refresh_theme_and_tray();
    }

    pub fn submit_composer(&mut self) {
        let chat_id = self.selected_chat_id.clone().expect("a chat is selected");
        let text = self.composer_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut st = self.store.write().unwrap();
        if let Some(edit_id) = self.editing_message_id.take() {
            st.edit_message(&chat_id, &edit_id, text).expect("edit message");
        } else {
            st.send_message(&chat_id, Some(text), self.reply_to_message_id.take(), None).expect("send message");
            st.patch_chat(&chat_id, true, None, None).expect("mark chat read");
        }
        self.composer_text.clear();
        drop(st);
        self.refresh_theme_and_tray();
    }

    pub fn step_chat(&mut self, delta: isize) {
        let next = {
            let st = self.store.read().unwrap();
            let chats = self.visible_chats(&st);
            if chats.is_empty() {
                return;
            }
            let cur = self.selected_chat_id.as_ref().and_then(|id| chats.iter().position(|c| &c.id == id));
            chats[(cur.unwrap_or(0) as isize + delta).rem_euclid(chats.len() as isize) as usize].id.clone()
        };
        self.select_chat(&next);
    }

    pub fn update(&mut self, msg: UiMessage) -> Task<UiMessage> {
        match msg {
            UiMessage::SelectTab(tab) => self.active_tab = tab,
            UiMessage::SelectAccount(acc) => self.selected_account_filter = acc,
            UiMessage::SelectChat(id) => self.select_chat(&id),
            UiMessage::ComposerChanged(v) => self.composer_text = v,
            UiMessage::SubmitComposer => self.submit_composer(),
            UiMessage::CancelComposeMode => {
                self.reply_to_message_id = None;
                self.editing_message_id = None;
                self.composer_text.clear();
            }
            UiMessage::StartReply(mid) => {
                self.editing_message_id = None;
                self.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            UiMessage::StartEdit(mid, old_txt) => {
                self.reply_to_message_id = None;
                self.editing_message_id = Some(mid);
                self.composer_text = old_txt;
                return operation::focus("composer-input");
            }
            UiMessage::DeleteMessage(cid, mid) => {
                self.store.write().unwrap().delete_message(&cid, &mid).expect("delete message");
            }
            UiMessage::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write().unwrap();
                let mine = |r: &Reaction| r.reaction_key == key && r.participant_id.starts_with("@self_");
                if st.messages[st.msg_idx_by_id[&mid]].reactions.iter().any(mine) {
                    st.remove_reaction(&cid, &mid, &key).expect("remove reaction");
                } else {
                    st.add_reaction(&cid, &mid, &key, None).expect("add reaction");
                }
            }
            UiMessage::ToggleBookmark(mid) => {
                self.store.write().unwrap().toggle_bookmark(&mid);
            }
            UiMessage::MarkUnreadFromHere(cid, mid) => {
                self.store.write().unwrap().mark_unread_from_message(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            UiMessage::RetranscribeVoice(mid) => {
                self.status_banner = self.store.write().unwrap().retranscribe_voice_note(&mid);
            }
            UiMessage::VotePoll(mid, opt_id) => {
                self.store.write().unwrap().vote_poll(&mid, &opt_id, "@self:beeper.com");
            }
            UiMessage::SendQuickVoiceNote => {
                let cid = self.selected_chat_id.clone().expect("a chat is selected");
                {
                    let att = Attachment {
                        attachment_type: "audio".to_string(),
                        id: Some("att_vn_quick".to_string()),
                        duration: Some(8.4),
                        file_name: Some("voice-note.ogg".to_string()),
                        file_size: Some(34_816),
                        is_gif: Some(false),
                        is_sticker: Some(false),
                        is_voice_note: Some(true),
                        mime_type: Some("audio/ogg".to_string()),
                        poster_img: None,
                        size: None,
                        src_url: Some("mxc://beeper.com/quick_vn".to_string()),
                        transcript: None,
                    };
                    let text = Some("Voice note".to_string());
                    self.store.write().unwrap().send_message(&cid, text, None, Some(att)).expect("send voice note");
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SendQuickPoll => {
                let cid = self.selected_chat_id.clone().expect("a chat is selected");
                {
                    let mut st = self.store.write().unwrap();
                    let question = "Ship Beeper v4.3 today?".to_string();
                    let (_, mid, _) =
                        st.send_message(&cid, Some(format!("Poll: {question}")), None, None).expect("send poll");
                    let option = |id: &str, label: &str, voter_ids: Vec<String>| PollOption {
                        id: id.into(),
                        label: label.into(),
                        voter_ids,
                    };
                    let options = vec![
                        option("p_yes", "Yes, ship it", vec!["@self:beeper.com".into()]),
                        option("p_no", "Wait for QA", Vec::new()),
                    ];
                    let idx = st.msg_idx_by_id[&mid];
                    st.update_message(idx, |m| m.poll = Some(PollData { question, allow_multiple: false, options }));
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SetReminder(cid) => {
                let reminder =
                    ChatReminder { remind_at: "2026-10-15T09:00:00.000Z".into(), dismiss_on_incoming_message: true };
                self.update_chat(&cid, |c| c.reminder = Some(reminder));
                self.status_banner = "Reminder scheduled for tomorrow 09:00".to_string();
            }
            UiMessage::TogglePinChat(cid) => self.update_chat(&cid, |c| c.is_pinned = Some(c.is_pinned != Some(true))),
            UiMessage::ToggleMuteChat(cid) => self.update_chat(&cid, |c| c.is_muted = Some(c.is_muted != Some(true))),
            UiMessage::ArchiveStale => {
                {
                    let mut st = self.store.write().unwrap();
                    for idx in st.stale_inbox_chats() {
                        st.update_chat(idx, |c| c.is_archived = Some(true));
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::ToggleArchiveChat(cid) => {
                self.update_chat(&cid, |c| c.is_archived = Some(c.is_archived != Some(true)));
                self.refresh_theme_and_tray();
            }
            UiMessage::NotifyAnyway(cid) => {
                self.update_chat(&cid, |c| c.is_muted = Some(false));
                self.status_banner = "Notifications unmuted for conversation".to_string();
            }
            UiMessage::NicknameChanged(v) => self.nickname_edit_buffer = v,
            UiMessage::SaveNickname => {
                let cid = self.selected_chat_id.clone().expect("a chat is selected");
                let mut st = self.store.write().unwrap();
                let idx = st.resolve_chat_idx(&cid).expect("selected chat exists");
                let other = st.chats[idx].participants.items.iter().find(|u| u.is_self != Some(true));
                if let Some(uid) = other.map(|u| u.id.clone()) {
                    let nick = Some(self.nickname_edit_buffer.trim().to_string()).filter(|n| !n.is_empty());
                    st.set_user_nickname(&cid, &uid, nick);
                }
            }
            UiMessage::SearchChanged(q) => self.search_query = q,
            UiMessage::ClearSearch => self.search_query.clear(),
            UiMessage::ToggleCommandPalette => {
                self.show_command_palette = !self.show_command_palette;
                if self.show_command_palette {
                    return operation::focus("cmd-palette-input");
                }
            }
            UiMessage::CommandQueryChanged(q) => self.command_palette_query = q,
            UiMessage::ExecuteCommandChat(cid) => {
                self.show_command_palette = false;
                self.command_palette_query.clear();
                self.select_chat(&cid);
            }
            UiMessage::SelectSettingsTab(t) => self.active_settings_tab = t,
            UiMessage::ToggleRightPanel => self.show_right_panel = !self.show_right_panel,
            UiMessage::ToggleDnd => {
                self.dnd_muted = !self.dnd_muted;
                self.refresh_theme_and_tray();
            }
            UiMessage::CycleTheme => {
                let cur = self.store.read().unwrap().settings.theme;
                let next = ThemeChoice::ALL[(ThemeChoice::ALL.iter().position(|t| *t == cur).expect("listed") + 1) % 3];
                return self.update(UiMessage::SelectTheme(next));
            }
            UiMessage::SystemTheme(mode) => {
                self.system_dark = mode == iced::theme::Mode::Dark;
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectTheme(choice) => self.patch_settings(|s| s.theme = choice),
            UiMessage::ToggleSetting(flip) => self.patch_settings(flip),
            UiMessage::CloudAccounts(Ok(accounts)) => self.store.write().unwrap().set_cloud_accounts(accounts),
            UiMessage::CloudAccounts(Err(e)) => self.status_banner = format!("Could not load your chat accounts: {e}"),
            UiMessage::RecoveryInputChanged(v) => self.recovery_code_input = v,
            UiMessage::VerifyRecoveryCode => {
                let mut st = self.store.write().unwrap();
                st.beeper_state.verified = true;
                st.beeper_state.cross_signing = true;
                st.beeper_state.key_backup = true;
                st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
                st.persist_state_to_sqlite();
                self.status_banner = "E2EE Recovery Key verified".to_string();
            }
            UiMessage::VerifySasNow => {
                let mut st = self.store.write().unwrap();
                st.beeper_state.verified = true;
                st.setup_status = BeeperStore::build_setup_from_state(&st.beeper_state);
                st.persist_state_to_sqlite();
                self.status_banner = "Device SAS E2EE verification complete".to_string();
            }
            UiMessage::ClearOldMedia => {
                let n = self.store.write().unwrap().clear_cached_media_older_than_days(30);
                self.status_banner = format!("Cleared {n} cached media files");
            }
            UiMessage::ExportChatJson(cid) => {
                let st = self.store.read().unwrap();
                let count = st.chat_msg_indices.get(&cid).map_or(0, Vec::len);
                let out_path = st.data_dir.join("chat-export.json");
                let payload = serde_json::json!({ "chatID": cid, "messageCount": count });
                std::fs::write(&out_path, serde_json::to_vec_pretty(&payload).unwrap()).expect("write export json");
                self.status_banner = format!("Exported {count} messages to {}", out_path.display());
            }
            UiMessage::OpenQuickLook(title, detail) => {
                self.quick_look_target = Some((title, detail));
            }
            UiMessage::CloseModals => {
                self.show_command_palette = false;
                self.show_filter_menu = false;
                self.show_attach_menu = false;
                self.quick_look_target = None;
            }
            UiMessage::ToggleEditHistory(mid) => {
                self.expanded_history_msg_id = (self.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            UiMessage::Noop => {}
            UiMessage::OpenUrl(url) => {
                let (cmd, args) = open_url_or_path_command(self.os, &url);
                std::thread::spawn(move || {
                    assert!(std::process::Command::new(cmd).args(args).status().expect("run URL opener").success())
                });
            }
            UiMessage::ToggleAttachMenu => self.show_attach_menu = !self.show_attach_menu,
            UiMessage::ToggleSearch => {
                self.show_search = !self.show_search;
                self.search_query.clear();
                if self.show_search {
                    return operation::focus("sidebar-search-input");
                }
            }
            UiMessage::ToggleFilterMenu => self.show_filter_menu = !self.show_filter_menu,
            UiMessage::OpenSettings(tab) => return self.open_settings(tab),
            UiMessage::SettingsQueryChanged(q) => self.settings_query = q,
            UiMessage::SetPref(key, v) => self.patch_settings(|s| s.set_pref(key, v)),
            UiMessage::Zoom(step) => self.patch_settings(|s| {
                let at = ZOOM_STEPS.iter().position(|z| *z >= s.zoom_percent).expect("zoom is in the steps");
                s.zoom_percent = match step {
                    0 => 100,
                    _ => ZOOM_STEPS[at.saturating_add_signed(step as isize).min(ZOOM_STEPS.len() - 1)],
                };
            }),
            UiMessage::HoverEnter(id) => self.hovered_msg = Some(id),
            UiMessage::HoverExit(id) => {
                if self.hovered_msg.as_ref() == Some(&id) {
                    self.hovered_msg = None;
                }
            }
            UiMessage::DragStart => self.dragging = true,
            UiMessage::DragMoved(pt) => self.sidebar_width = (pt.x - 72.0).clamp(190.0, 420.0),
            UiMessage::DragEnd => self.dragging = false,
            UiMessage::WindowOpened(id) => {
                let key = if Some(id) == self.settings_window {
                    "BEEPER_SCREENSHOT_SETTINGS"
                } else {
                    "BEEPER_SCREENSHOT_EXIT"
                };
                if std::env::var(key).is_ok() {
                    return window::screenshot(id).map(move |s| UiMessage::ScreenshotCaptured(id, s));
                }
            }
            UiMessage::WindowClosed(id) => {
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                } else if [self.main_window, self.login_window].contains(&Some(id)) {
                    return iced::exit();
                }
            }
            UiMessage::ScreenshotCaptured(id, shot) => {
                let is_settings = Some(id) == self.settings_window;
                let path =
                    std::env::var(if is_settings { "BEEPER_SCREENSHOT_SETTINGS" } else { "BEEPER_SCREENSHOT_EXIT" })
                        .expect("screenshot path");
                write_rgba_png(std::path::Path::new(&path), shot.size.width, shot.size.height, &shot.rgba);
                if !is_settings && std::env::var("BEEPER_SCREENSHOT_SETTINGS").is_ok() {
                    return self.open_settings(SettingsTab::General);
                }
                return iced::exit();
            }
            UiMessage::StoreChanged
                if self.login_window.is_some()
                    && self.login.screen != login::Screen::Accounts
                    && self.store.read().unwrap().signed_in() =>
            {
                self.login.screen = login::Screen::Accounts;
            }
            UiMessage::StoreChanged => self.refresh_theme_and_tray(),
            UiMessage::FocusRequested(chat) => {
                if let Some(id) = chat {
                    self.select_chat(&id);
                }
                return window::gain_focus(self.main_window.or(self.login_window).expect("a window is open"));
            }
            UiMessage::Login(m) => return self.update_login(m),
            UiMessage::KeyPressed(key, mods) => {
                let cmd = mods.command() || mods.control();
                match key.as_ref() {
                    Key::Character("k") if cmd => return self.update(UiMessage::ToggleCommandPalette),
                    Key::Character(",") if cmd => return self.open_settings(self.active_settings_tab),
                    Key::Character("f") if cmd => return self.update(UiMessage::ToggleSearch),
                    Key::Character(".") if cmd => self.show_right_panel = !self.show_right_panel,
                    Key::Character("=" | "+") if cmd => return self.update(UiMessage::Zoom(1)),
                    Key::Character("-") if cmd => return self.update(UiMessage::Zoom(-1)),
                    Key::Character("0") if cmd => return self.update(UiMessage::Zoom(0)),
                    Key::Named(Named::Escape) => return self.update(UiMessage::CloseModals),
                    Key::Named(Named::ArrowDown) if mods.alt() => self.step_chat(1),
                    Key::Named(Named::ArrowUp) if mods.alt() => self.step_chat(-1),
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn update_chat(&self, chat_id: &str, f: impl FnOnce(&mut Chat)) {
        let mut st = self.store.write().unwrap();
        let idx = st.resolve_chat_idx(chat_id).expect("chat exists");
        st.update_chat(idx, f);
    }

    fn patch_settings(&mut self, f: impl FnOnce(&mut BeeperSettings)) {
        {
            let mut st = self.store.write().unwrap();
            f(&mut st.settings);
            st.persist_state_to_sqlite();
        }
        self.refresh_theme_and_tray();
    }

    pub fn subscription(&self) -> Subscription<UiMessage> {
        let changes = Subscription::run_with(StoreFeed(self.store.clone(), 0), |f| {
            feed(f.0.read().unwrap().ws_tx.subscribe(), |_| UiMessage::StoreChanged)
        });
        let focus = Subscription::run_with(StoreFeed(self.store.clone(), 1), |f| {
            feed(f.0.read().unwrap().focus_tx.subscribe(), UiMessage::FocusRequested)
        });
        let keys = iced::event::listen_with(|e, _, id| match e {
            Event::Window(window::Event::Closed) => Some(UiMessage::WindowClosed(id)),
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if modifiers.command()
                    || modifiers.control()
                    || modifiers.alt()
                    || key == Key::Named(Named::Escape) =>
            {
                Some(UiMessage::KeyPressed(key, modifiers))
            }
            _ => None,
        });
        let system = iced::system::theme_changes().map(UiMessage::SystemTheme);
        let drag = self.dragging.then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(UiMessage::DragMoved(position)),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::DragEnd),
                _ => None,
            })
        });
        Subscription::batch([changes, focus, keys, system].into_iter().chain(drag))
    }

    pub fn view(&self, id: window::Id) -> El<'_> {
        if Some(id) == self.settings_window {
            self.view_settings()
        } else if Some(id) == self.login_window {
            self.view_login()
        } else {
            self.view_main()
        }
    }

    pub fn view_main(&self) -> El<'_> {
        let p = &self.pal;
        let grip = mouse_area(container(Space::new()).width(8).height(Fill))
            .on_press(UiMessage::DragStart)
            .interaction(mouse::Interaction::ResizingHorizontally);
        let card = container(self.view_conversation()).padding(Padding::new(8.0).left(0.0)).width(Fill);
        let body = row![self.view_rail(), self.view_sidebar(), grip, card]
            .push(self.show_right_panel.then(|| self.view_info()));
        let base: El<'_> = container(body).width(Fill).height(Fill).style(fill_box(p.frame, 0.0)).into();
        let modal = match (&self.quick_look_target, self.show_command_palette) {
            (Some((title, detail)), _) => Some(self.view_quick_look(title, detail)),
            (None, true) => Some(self.view_command_palette()),
            _ => None,
        };
        let scrim = modal.map(|m| {
            mouse_area(
                container(mouse_area(m).on_press(UiMessage::Noop))
                    .padding(Padding::ZERO.top(90.0))
                    .center_x(Fill)
                    .height(Fill)
                    .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.45), 0.0)),
            )
            .on_press(UiMessage::CloseModals)
        });
        stack![base].push(scrim).into()
    }

    fn view_rail(&self) -> El<'_> {
        let p = &self.pal;
        let rail_btn = |glyph: El<'static>, on: bool, msg: UiMessage| {
            button(container(glyph).center(36))
                .padding(0)
                .style(btn_style([if on { p.rail_sel } else { CLEAR }, p.hover, p.rail_sel], [p.strong; 3], 14.0))
                .on_press(msg)
        };
        let tab = |id: &'static str, glyph: El<'static>, t: InboxTab| -> El<'static> {
            container(rail_btn(
                glyph,
                self.active_tab == t && self.selected_account_filter.is_none(),
                UiMessage::SelectTab(t),
            ))
            .id(id)
            .into()
        };
        let g = |c: char| -> El<'static> { ic(c, 18.0).color(p.strong).into() };
        let add_space = container(g(icon::MAPS_UGC)).center(36).style(bordered_box(CLEAR, p.tertiary, 1.0, 12.0));
        let st = self.store.read().unwrap();
        let accounts =
            std::iter::once("hungryserv".to_string()).chain(st.accounts.iter().map(|a| a.account_id.clone()));
        let accounts = accounts.fold(Column::new().spacing(8).align_x(Center), |c, acc| {
            let on = self.selected_account_filter.as_ref() == Some(&acc);
            let logo: El<'static> = svg(brand_svg(brand_key(&acc))).width(28).height(28).into();
            c.push(rail_btn(logo, on, UiMessage::SelectAccount((!on).then_some(acc))))
        });
        let top = column![
            tab("rail-inbox", g(icon::INBOX), InboxTab::AllChats),
            tab("rail-low-priority", g(icon::LOW_PRIORITY_SPACE), InboxTab::LowPriority),
            tab("rail-archive", g(icon::ARCHIVE), InboxTab::Archived),
            tab("rail-reminders", g(icon::SCHEDULE), InboxTab::Reminders),
            tab("rail-drafts", g(icon::SCHEDULE_SEND), InboxTab::Drafts),
            tab("rail-unread", g(icon::EMAIL), InboxTab::Unread),
            button(add_space).padding(0).style(ghost(p, 12.0)).on_press(UiMessage::ToggleCommandPalette),
            accounts,
            rail_btn(g(icon::MORE_HORIZ), false, UiMessage::OpenSettings(SettingsTab::Accounts)),
        ]
        .spacing(8)
        .align_x(Center)
        .width(68)
        .padding(Padding::ZERO.top(14.0));
        let me = st.me().full_name.as_deref().expect("self name");
        let footer = column![
            rail_btn(
                g(icon::PHOTO_LIBRARY),
                false,
                UiMessage::OpenQuickLook("Media".into(), "Photos, videos, and files from all chats".into())
            ),
            rail_btn(g(icon::LIBRARY_ADD), false, UiMessage::ToggleCommandPalette),
            button(self_avatar(me, 36.0))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text; 3], 18.0))
                .on_press(UiMessage::OpenSettings(SettingsTab::General)),
        ]
        .spacing(8)
        .align_x(Center)
        .padding(Padding::ZERO.bottom(16.0));
        let scroll = scrollable(top).direction(Direction::Vertical(Scrollbar::hidden())).height(Fill);
        container(column![scroll, footer].align_x(Center)).width(68).height(Fill).style(fill_box(p.rail, 0.0)).into()
    }

    fn view_sidebar(&self) -> El<'_> {
        let p = &self.pal;
        let w = self.sidebar_width;
        let st = self.store.read().unwrap();
        let chats = self.visible_chats(&st);
        let me = st.me().full_name.as_deref().expect("self name");
        let h24 = st.settings.pref("timeFormat") == 1;
        let previews = st.settings.on("showMessagePreviews");
        let header_icon = p.text.scale_alpha(0.8);
        let title = button(
            row![semi(self.active_tab.label(), 13.0).color(p.strong), ic(icon::UNFOLD_MORE, 13.0).color(p.weak)]
                .spacing(4)
                .align_y(Center),
        )
        .padding([4, 8])
        .style(ghost(p, 6.0))
        .on_press(UiMessage::ToggleFilterMenu);
        let header = row![
            title,
            Space::new().width(Fill),
            icon_btn(p, icon::FILTER_LIST, 16.0, 26.0, header_icon, UiMessage::ToggleFilterMenu),
            icon_btn(p, icon::SEARCH, 16.0, 26.0, header_icon, UiMessage::ToggleSearch),
            icon_btn(p, icon::EDIT_SQUARE, 16.0, 26.0, header_icon, UiMessage::ToggleCommandPalette)
        ]
        .align_y(Center)
        .height(38)
        .padding(Padding::ZERO.right(4.0));
        let filters = self.show_filter_menu.then(|| {
            let tabs = [
                InboxTab::AllChats,
                InboxTab::Unread,
                InboxTab::Pinned,
                InboxTab::Drafts,
                InboxTab::Reminders,
                InboxTab::LowPriority,
                InboxTab::Archived,
            ];
            let chips = tabs.into_iter().fold(Row::new().spacing(4), |r, t| {
                let on = self.active_tab == t;
                r.push(
                    button(med(t.label(), 12.0).color(if on { Color::WHITE } else { p.text }))
                        .padding([3, 8])
                        .style(btn_style(
                            [if on { p.accent } else { p.hover }, p.selected, p.selected],
                            [p.text; 3],
                            12.0,
                        ))
                        .on_press(UiMessage::SelectTab(t)),
                )
            });
            container(chips.wrap().vertical_spacing(4)).padding([4, 8])
        });
        let search = self.show_search.then(|| {
            let input = text_input("Search", &self.search_query)
                .id("sidebar-search-input")
                .on_input(UiMessage::SearchChanged)
                .size(13)
                .padding([6, 4])
                .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
            container(
                container(
                    row![
                        ic(icon::SEARCH, 14.0).color(p.sub),
                        input,
                        icon_btn(p, icon::CLOSE, 12.0, 20.0, p.sub, UiMessage::ToggleSearch)
                    ]
                    .spacing(4)
                    .align_y(Center)
                    .padding([0, 8]),
                )
                .style(fill_box(p.search, 8.0)),
            )
            .padding([4, 8])
        });
        let selected = self.selected_chat_id.as_deref();
        let pin_section = self.active_tab == InboxTab::AllChats && self.search_query.is_empty();
        let (pinned, rest): (Vec<&Chat>, Vec<&Chat>) =
            chats.iter().copied().partition(|c| pin_section && c.flags & CHAT_PINNED != 0);
        let cols = (((w - 16.0) / 80.0).floor() as usize).max(1);
        let cell = (w - 16.0) / cols as f32;
        let pins = pinned.chunks(cols).fold(Column::new(), |col, chunk| {
            col.push(chunk.iter().fold(Row::new(), |r, c| {
                let on = selected == Some(c.id.as_str());
                let label = row![ell(
                    &c.title,
                    cell - if c.is_muted == Some(true) { 34.0 } else { 20.0 },
                    11.0,
                    Weight::Medium
                )
                .color(p.text)]
                .push((c.is_muted == Some(true)).then(|| ic(icon::NOTIFICATIONS_OFF, 10.0).color(p.sub)))
                .spacing(3)
                .align_y(Center);
                let unread = (c.unread_count > 0).then(|| count_badge(c.unread_count, p.accent));
                let face =
                    stack![chat_avatar(c, me, 40.0), container(unread).width(44).align_right(44)].width(44).height(40);
                let item = column![face, label]
                    .spacing(4)
                    .align_x(Center)
                    .width(cell)
                    .padding(Padding::new(8.0).top(8.0).bottom(6.0).left(4.0).right(4.0));
                r.push(
                    button(item)
                        .padding(0)
                        .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.text; 3], 10.0))
                        .on_press(UiMessage::SelectChat(c.id.clone())),
                )
            }))
        });
        let text_w = w - 8.0 - 16.0 - 34.0 - 4.0;
        let rows = rest.iter().fold(Column::new(), |col, c| {
            let on = selected == Some(c.id.as_str());
            let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
            let m = st.chat_msg_indices.get(&c.id).and_then(|v| v.last()).map(|&i| &st.messages[i]);
            let ts = c.last_activity.as_deref().map(|t| list_ts(t, h24)).unwrap_or_default();
            let ts_w = crate::style::text_width(&ts, 10.0, Weight::Normal) + 8.0;
            let top = row![
                ell(&c.title, text_w - ts_w, 13.0, Weight::Semibold).color(p.strong),
                Space::new().width(Fill),
                txt(ts, 10.0).color(p.sub)
            ]
            .align_y(Center);
            let mine = m.is_some_and(|m| m.is_sender == Some(true));
            let image = m.is_some_and(|m| m.attachments.iter().any(|a| a.attachment_type == "img"));
            let body = m.and_then(|m| m.text.clone()).filter(|t| !t.is_empty() && previews).map(|t| {
                match (c.chat_type == "group", m.and_then(|m| m.sender_name.clone())) {
                    (true, Some(s)) if !mine => {
                        format!("{}: {t}", s.split(' ').next().expect("sender name"))
                    }
                    _ => t,
                }
            });
            let count = (c.unread_count > 0)
                .then(|| count_badge(c.unread_count, if c.is_muted == Some(true) { p.muted } else { p.accent }));
            let reserve = if count.is_some() { 26.0 } else { 0.0 } + if mine { 16.0 } else { 0.0 };
            let preview = row![]
                .push(mine.then(|| ic(icon::CIRCLE_CHECK, 12.0).color(p.sub)))
                .push(image.then(|| {
                    container(
                        row![ic(icon::IMAGE, 11.0).color(p.text), txt("Image", 11.0).color(p.text)]
                            .spacing(3)
                            .align_y(Center),
                    )
                    .padding([1, 5])
                    .style(fill_box(p.link_card, 4.0))
                }))
                .push(body.map(|b| {
                    ell(&b, text_w - reserve, 13.0, Weight::Normal).color(if unread { p.text } else { p.sub })
                }))
                .push(Space::new().width(Fill))
                .push(count)
                .spacing(4)
                .align_y(Center);
            let item = row![
                badged(p, chat_avatar(c, me, 28.0), &c.account_id, 28.0, 12.0),
                column![top, preview].spacing(1).width(Fill)
            ]
            .spacing(4)
            .align_y(Center)
            .padding([10, 8]);
            col.push(
                button(item)
                    .padding(0)
                    .width(Fill)
                    .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.text; 3], 8.0))
                    .on_press(UiMessage::SelectChat(c.id.clone())),
            )
            .push(divider(p, 52.0))
        });
        let empty = chats.is_empty().then(|| {
            container(txt(format!("No chats in {}", self.active_tab.label()), 13.0).color(p.sub))
                .center_x(Fill)
                .padding(24)
        });
        let list = scrollable(column![pins, rows].push(empty).padding(Padding::ZERO.right(4.0)))
            .height(Fill)
            .style(scroll_style(p.divider, p.weak));
        container(column![header].push(filters).push(search).push(list).padding(Padding::ZERO.left(8.0).top(8.0)))
            .width(w)
            .height(Fill)
            .into()
    }

    fn view_conversation(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let cid = self.selected_chat_id.as_deref().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
        let me = st.me().full_name.as_deref().expect("self name");
        let msgs: Vec<&Message> =
            st.chat_msg_indices.get(cid).map(|v| v.iter().map(|&i| &st.messages[i]).collect()).unwrap_or_default();
        let mut col = Column::new().spacing(2).padding(Padding::new(16.0).top(96.0).bottom(8.0));
        let mut prev: Option<&Message> = None;
        for (i, m) in msgs.iter().enumerate() {
            let day_changed = prev.is_none_or(|q| q.timestamp[..10] != m.timestamp[..10]);
            if day_changed {
                let pill = container(semi(day_label(&m.timestamp), 11.0).color(p.date_text))
                    .padding([5, 10])
                    .style(bordered_box(p.card, p.outline, 1.0, 12.0));
                col = col.push(container(pill).center_x(Fill).padding([12, 0]));
            }
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id);
            let last =
                msgs.get(i + 1).is_none_or(|n| n.sender_id != m.sender_id || n.timestamp[..10] != m.timestamp[..10]);
            if first && !day_changed {
                col = col.push(Space::new().height(6));
            }
            col = col.push(self.view_message(&st, chat, m, first, last));
            prev = Some(m);
        }
        if msgs.last().is_some_and(|m| m.is_sender == Some(true)) {
            col = col.push(
                container(txt("Sent", 11.0).color(p.muted))
                    .align_right(Fill)
                    .padding(Padding::ZERO.right(4.0).top(2.0)),
            );
        }
        let list = scrollable(col).anchor_bottom().height(Fill).style(scroll_style(p.divider, p.weak));
        let status = (chat.id != NOTE_CHAT_ID).then(|| txt(network_name(&chat.account_id), 10.0).color(p.sub));
        let name_row =
            row![semi(chat.title.clone(), 12.0).color(p.strong), ic(icon::CHEVRON_RIGHT, 12.0).color(p.tertiary)]
                .spacing(2)
                .align_y(Center);
        let pill = button(column![name_row].push(status).align_x(Center).spacing(1))
            .padding(Padding::new(6.0).left(10.0).top(if chat.id == NOTE_CHAT_ID { 6.0 } else { 8.0 }))
            .style(btn_style([p.elevated, p.selected, p.selected], [p.text; 3], 14.0))
            .on_press(UiMessage::ToggleRightPanel);
        let ring = container(chat_avatar(chat, me, 40.0)).padding(1).style(fill_box(p.card, 21.0));
        let header = stack![
            container(pill).center_x(Fill).padding(Padding::ZERO.top(44.0)),
            container(ring).center_x(Fill).padding(Padding::ZERO.top(8.0))
        ];
        let feedback = button(
            row![ic(icon::MESSAGE_SQUARE, 13.0).color(p.text), med("Feedback?", 12.0).color(p.text)]
                .spacing(5)
                .align_y(Center),
        )
        .padding([5, 10])
        .style(btn_style([p.hover, p.selected, p.selected], [p.text; 3], 14.0))
        .on_press(UiMessage::OpenQuickLook("Feedback".into(), "Tell us what to improve in beeper-rs.".into()));
        let fade = Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, p.card)
            .add_stop(0.75, p.card.scale_alpha(0.85))
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let fade = container(Space::new()).width(Fill).height(84).style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fade))),
            ..Default::default()
        });
        let overlay = stack![fade, header, container(feedback).align_right(Fill).padding([12, 12])];
        let pane = column![stack![list, overlay].height(Fill), self.view_composer(&st, chat)];
        container(pane).width(Fill).height(Fill).clip(true).style(fill_box(p.card, 10.0)).into()
    }

    fn view_message<'a>(&self, st: &BeeperStore, chat: &Chat, m: &Message, first: bool, last: bool) -> El<'a> {
        let p = &self.pal;
        let mine = m.is_sender == Some(true);
        let group = chat.chat_type == "group";
        let (bg, fg) = if mine { (p.sent, Color::WHITE) } else { (p.received, p.strong) };
        let text = m.text.clone().unwrap_or_default();
        let preview = st
            .settings
            .on("loadEmbeds")
            .then(|| text.split_whitespace().find_map(|w| st.link_previews.get(w).map(|lp| (w, lp))))
            .flatten();
        let mut parts = Column::new().spacing(2).align_x(if mine { Right } else { iced::Left });
        if group && !mine && first {
            parts = parts.push(
                container(semi(m.sender_name.clone().expect("sender name"), 11.0).color(hue(hash_str(&m.sender_id))))
                    .padding(Padding::ZERO.left(11.0)),
            );
        }
        if let Some(q) = m.linked_message_id.as_ref().and_then(|r| st.msg_idx_by_id.get(r)).map(|&i| &st.messages[i]) {
            let quote = column![
                semi(q.sender_name.clone().expect("sender name"), 11.0).color(p.text),
                ell(q.text.as_deref().unwrap_or(""), 260.0, 12.0, Weight::Normal).color(p.sub)
            ]
            .spacing(1);
            parts = parts.push(container(quote).padding([6, 11]).style(bordered_box(p.card, p.outline, 1.0, 14.0)));
        }
        if let Some((url, lp)) = preview {
            let host = url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(url).to_string();
            let body = column![
                txt(lp.site.clone(), 11.0).color(p.sub),
                semi(lp.title.clone(), 13.0).color(p.strong),
                txt(lp.summary.clone(), 13.0).color(p.sub),
                row![ic(icon::LINK, 11.0).color(p.weak), txt(host, 11.0).color(p.weak)].spacing(4).align_y(Center),
            ]
            .spacing(6)
            .padding([10, 11]);
            parts = parts.push(container(body).max_width(360).style(fill_box(p.link_card, 17.0)));
        }
        for a in &m.attachments {
            let name = a.file_name.clone().expect("attachment file name");
            let content: El<'a> = match (a.is_voice_note == Some(true), a.attachment_type.as_str()) {
                (true, _) => {
                    let bars = (0..28).fold(Row::new().spacing(2).align_y(Center), |r, k| {
                        r.push(
                            container(Space::new())
                                .width(2)
                                .height(4.0 + (hash_str(&format!("{name}{k}")) % 14) as f32)
                                .style(fill_box(fg.scale_alpha(0.7), 1.0)),
                        )
                    });
                    let play = container(ic(icon::PLAY, 14.0).color(bg)).center(30).style(fill_box(fg, 15.0));
                    let dur = a.duration.map(|d| {
                        let s = d.round() as u32;
                        txt(format!("{}:{:02}", s / 60, s % 60), 11.0).color(fg.scale_alpha(0.7))
                    });
                    let line = row![play, bars].push(dur).spacing(8).align_y(Center);
                    let transcript = a.transcript.clone().map(|t| {
                        button(txt(t, 12.0).color(fg.scale_alpha(0.75)))
                            .padding(0)
                            .style(btn_style([CLEAR; 3], [fg; 3], 0.0))
                            .on_press(UiMessage::RetranscribeVoice(m.id.clone()))
                    });
                    container(column![line].push(transcript).spacing(6).max_width(300))
                        .padding([8, 11])
                        .style(fill_box(bg, 17.0))
                        .into()
                }
                (false, "img") => button(
                    container(ic(icon::IMAGE, 28.0).color(p.sub))
                        .center_x(240)
                        .center_y(160)
                        .style(fill_box(p.link_card, 17.0)),
                )
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text; 3], 17.0))
                .on_press(UiMessage::OpenQuickLook(name, "Image attachment".into()))
                .into(),
                _ => {
                    let size = a.file_size.map(human_size).unwrap_or_default();
                    let tile = container(ic(icon::FILE_TEXT, 18.0).color(fg))
                        .center(36)
                        .style(fill_box(fg.scale_alpha(0.15), 8.0));
                    let info =
                        column![semi(name.clone(), 13.0).color(fg), txt(size.clone(), 11.0).color(fg.scale_alpha(0.7))]
                            .spacing(2);
                    button(row![tile, info].spacing(10).align_y(Center))
                        .padding([8, 11])
                        .style(btn_style([bg, bg, bg], [fg; 3], 17.0))
                        .on_press(UiMessage::OpenQuickLook(name, size))
                        .into()
                }
            };
            parts = parts.push(content);
        }
        if let Some(poll) = &m.poll {
            let total = poll.options.iter().map(|o| o.voter_ids.len()).sum::<usize>().max(1);
            let opts = poll.options.iter().fold(Column::new().spacing(4), |c, o| {
                let voted = o.voter_ids.iter().any(|v| v == "@self:beeper.com");
                let label = row![
                    txt(o.label.clone(), 13.0).color(fg),
                    Space::new().width(Fill),
                    txt(format!("{}%", o.voter_ids.len() * 100 / total), 11.0).color(fg.scale_alpha(0.7))
                ];
                c.push(
                    button(label)
                        .padding([6, 10])
                        .width(Fill)
                        .style(btn_style(
                            [
                                if voted { fg.scale_alpha(0.22) } else { fg.scale_alpha(0.08) },
                                fg.scale_alpha(0.15),
                                fg.scale_alpha(0.22),
                            ],
                            [fg; 3],
                            8.0,
                        ))
                        .on_press(UiMessage::VotePoll(m.id.clone(), o.id.clone())),
                )
            });
            parts = parts.push(
                container(column![semi(poll.question.clone(), 13.0).color(fg), opts].spacing(8).width(300))
                    .padding([10, 11])
                    .style(fill_box(bg, 17.0)),
            );
        }
        if !text.is_empty() && m.poll.is_none() && m.message_type.as_deref() != Some("VOICE") {
            let edited = if m.edit_history.is_empty() { "" } else { "Edited · " };
            let time =
                txt(format!("{edited}{}", format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1)), 11.0)
                    .color(if mine { p.sent_meta } else { p.sub });
            let spans: Vec<_> = text
                .split_inclusive(' ')
                .flat_map(|w| {
                    let url = w.trim_end().starts_with("http").then(|| w.trim_end().to_string());
                    let arrow = url.as_ref().map(|_| {
                        span(icon::ARROW_UP_RIGHT.to_string()).font(iced::Font::with_name("icons")).size(11.0).color(fg)
                    });
                    [
                        Some(
                            span(w.trim_end().to_string())
                                .size(13.0)
                                .color(fg)
                                .underline(url.is_some())
                                .link_maybe(url),
                        ),
                        arrow,
                        w.ends_with(' ').then(|| span(" ").size(13.0)),
                    ]
                    .into_iter()
                    .flatten()
                })
                .collect();
            let body = rich_text(spans)
                .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                .on_link_click(UiMessage::OpenUrl);
            parts = parts.push(
                container(row![body, time].spacing(8).align_y(Bottom).wrap())
                    .padding([8, 11])
                    .max_width(480)
                    .style(fill_box(bg, 17.0)),
            );
        }
        if self.expanded_history_msg_id.as_ref() == Some(&m.id) {
            parts = parts.push(m.edit_history.iter().fold(Column::new().spacing(2), |c, e| {
                c.push(txt(format!("{} · {}", e.text, format_short_ts(&e.timestamp, false)), 11.0).color(p.muted))
            }));
        }
        if !m.reactions.is_empty() {
            let mut keys: Vec<&str> = m.reactions.iter().map(|r| r.reaction_key.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            let pills = keys.into_iter().fold(Row::new().spacing(4), |r, k| {
                let n = m.reactions.iter().filter(|x| x.reaction_key == k).count();
                let own = m.reactions.iter().any(|x| x.reaction_key == k && x.participant_id.starts_with("@self_"));
                let pill =
                    row![txt(k.to_string(), 12.0), txt(n.to_string(), 11.0).color(p.text)].spacing(3).align_y(Center);
                r.push(
                    button(pill)
                        .padding([2, 7])
                        .style(btn_style(
                            [if own { p.accent.scale_alpha(0.3) } else { p.received }, p.selected, p.selected],
                            [p.text; 3],
                            11.0,
                        ))
                        .on_press(UiMessage::ToggleReaction(chat.id.clone(), m.id.clone(), k.to_string())),
                )
            });
            parts = parts.push(pills);
        }
        let tools = (self.hovered_msg.as_ref() == Some(&m.id)).then(|| {
            let b = |c: char, msg: UiMessage| icon_btn(p, c, 14.0, 26.0, p.weak, msg);
            let (cid, mid) = (chat.id.clone(), m.id.clone());
            let bar = row![
                b(icon::SMILE, UiMessage::ToggleReaction(cid.clone(), mid.clone(), "👍".into())),
                b(icon::REPLY, UiMessage::StartReply(mid.clone())),
                b(icon::BOOKMARK, UiMessage::ToggleBookmark(mid.clone())),
                b(icon::EMAIL, UiMessage::MarkUnreadFromHere(cid.clone(), mid.clone()))
            ]
            .push((!m.edit_history.is_empty()).then(|| b(icon::HISTORY, UiMessage::ToggleEditHistory(mid.clone()))))
            .push(mine.then(|| b(icon::PENCIL, UiMessage::StartEdit(mid.clone(), text.clone()))))
            .push(mine.then(|| b(icon::TRASH_2, UiMessage::DeleteMessage(cid.clone(), mid.clone()))));
            container(bar).padding(2).style(shadow_box(p.elevated, p.outline, 10.0, 8.0))
        });
        let side = (group && !mine).then(|| -> El<'a> {
            if last {
                avatar(m.sender_name.as_deref().expect("sender name"), 28.0)
            } else {
                Space::new().width(28).into()
            }
        });
        let line: El<'a> = if mine {
            row![Space::new().width(Fill)].push(tools).push(parts).spacing(6).align_y(Center).into()
        } else {
            row![].push(side).push(parts).push(tools).push(Space::new().width(Fill)).spacing(6).align_y(Bottom).into()
        };
        mouse_area(line)
            .on_enter(UiMessage::HoverEnter(m.id.clone()))
            .on_exit(UiMessage::HoverExit(m.id.clone()))
            .into()
    }

    fn view_composer<'a>(&self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        let banner = match (&self.editing_message_id, &self.reply_to_message_id) {
            (Some(_), _) => Some((icon::PENCIL, "Editing message".to_string())),
            (None, Some(r)) => Some((
                icon::REPLY,
                format!("Replying to {}", st.messages[st.msg_idx_by_id[r]].sender_name.clone().expect("sender name")),
            )),
            _ => None,
        }
        .map(|(c, label)| {
            row![
                ic(c, 12.0).color(p.accent),
                txt(label, 12.0).color(p.sub),
                Space::new().width(Fill),
                icon_btn(p, icon::CLOSE, 12.0, 22.0, p.sub, UiMessage::CancelComposeMode)
            ]
            .spacing(6)
            .align_y(Center)
            .padding([4, 16])
        });
        let attach = self.show_attach_menu.then(|| {
            row![
                pill_btn(p, "Poll", UiMessage::SendQuickPoll),
                pill_btn(p, "Voice note", UiMessage::SendQuickVoiceNote)
            ]
            .spacing(6)
            .padding([4, 16])
        });
        let placeholder = format!("Message {} on {}", chat.title, network_name(&chat.account_id));
        let input = text_input(&placeholder, &self.composer_text)
            .id("composer-input")
            .on_input(UiMessage::ComposerChanged)
            .on_submit(UiMessage::SubmitComposer)
            .size(13)
            .padding([9, 2])
            .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
        let net = svg(brand_svg(brand_key(&chat.account_id))).width(20).height(20);
        let smile = icon_btn(
            p,
            icon::SMILE,
            17.0,
            26.0,
            p.on_secondary,
            UiMessage::ComposerChanged(format!("{}🙂", self.composer_text)),
        );
        let voice = icon_btn(p, icon::AUDIO_LINES, 17.0, 26.0, p.on_secondary, UiMessage::SendQuickVoiceNote);
        let pill = container(
            row![net, input, smile, voice].spacing(6).align_y(Center).padding(Padding::new(0.0).left(8.0).right(6.0)),
        )
        .height(36)
        .width(Fill)
        .style(fill_box(p.input, 18.0));
        let plus = button(container(ic(icon::PLUS, 18.0).color(p.on_secondary)).center(36))
            .padding(0)
            .style(btn_style([p.input, p.selected, p.selected], [p.text; 3], 18.0))
            .on_press(UiMessage::ToggleAttachMenu);
        let ready = !self.composer_text.trim().is_empty();
        let send = button(container(ic(icon::ARROW_UP, 18.0).color(Color::WHITE)).center(34))
            .padding(0)
            .style(btn_style([if ready { p.accent } else { p.send_btn }, p.accent, p.accent], [p.text; 3], 17.0))
            .on_press(UiMessage::SubmitComposer);
        column![].push(banner).push(attach).push(row![plus, pill, send].spacing(12).align_y(Center).padding(8)).into()
    }

    fn view_info(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let cid = self.selected_chat_id.clone().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(&cid).expect("selected chat exists")];
        let me = st.me().full_name.as_deref().expect("self name");
        let action = |c: char, label: &'static str, msg: UiMessage| {
            button(
                column![ic(c, 16.0).color(p.text), txt(label, 11.0).color(p.sub)].spacing(4).align_x(Center).width(52),
            )
            .padding([8, 0])
            .style(btn_style([p.hover, p.selected, p.selected], [p.text; 3], 10.0))
            .on_press(msg)
        };
        let actions = row![
            action(
                icon::KEEP,
                if chat.is_pinned == Some(true) { "Unpin" } else { "Pin" },
                UiMessage::TogglePinChat(cid.clone())
            ),
            action(
                icon::NOTIFICATIONS_OFF,
                if chat.is_muted == Some(true) { "Unmute" } else { "Mute" },
                UiMessage::ToggleMuteChat(cid.clone())
            ),
            action(icon::ARCHIVE, "Archive", UiMessage::ToggleArchiveChat(cid.clone())),
            action(icon::SCHEDULE, "Remind", UiMessage::SetReminder(cid.clone())),
            action(icon::DOWNLOAD, "Export", UiMessage::ExportChatJson(cid.clone())),
        ]
        .spacing(4);
        let nickname = (chat.chat_type == "single" && chat.id != NOTE_CHAT_ID).then(|| {
            let input = text_input("Nickname", &self.nickname_edit_buffer)
                .on_input(UiMessage::NicknameChanged)
                .on_submit(UiMessage::SaveNickname)
                .size(13)
                .padding([7, 10])
                .style(input_style(p.input, p.strong, p.sub, p.accent, 8.0));
            column![med("Nickname", 12.0).color(p.weak), input].spacing(6)
        });
        let people = chat.participants.items.iter().fold(Column::new().spacing(8), |c, u| {
            let name = u.custom_nickname.clone().or_else(|| u.full_name.clone()).expect("participant name");
            c.push(
                row![
                    avatar(&name, 28.0),
                    column![txt(name, 13.0).color(p.text)]
                        .push(u.phone_number.clone().map(|ph| txt(ph, 11.0).color(p.sub)))
                ]
                .spacing(10)
                .align_y(Center),
            )
        });
        let body = column![
            closable_head(p, "Chat Info", 13.0, UiMessage::ToggleRightPanel),
            column![
                chat_avatar(chat, me, 64.0),
                semi(chat.title.clone(), 15.0).color(p.strong),
                txt(network_name(&chat.account_id), 12.0).color(p.sub)
            ]
            .spacing(6)
            .align_x(Center)
            .width(Fill),
            actions,
        ]
        .push(nickname)
        .push(column![med(format!("{} participants", chat.participants.total), 12.0).color(p.weak), people].spacing(8))
        .spacing(16)
        .padding(16);
        container(
            container(scrollable(body).style(scroll_style(p.divider, p.weak)))
                .width(300)
                .height(Fill)
                .style(fill_box(p.card, 10.0)),
        )
        .padding(Padding::new(8.0).left(0.0))
        .into()
    }

    fn view_command_palette(&self) -> El<'_> {
        let p = &self.pal;
        let input = text_input("Search chats and commands", &self.command_palette_query)
            .id("cmd-palette-input")
            .on_input(UiMessage::CommandQueryChanged)
            .size(14)
            .padding([10, 4])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let q = self.command_palette_query.to_lowercase();
        let st = self.store.read().unwrap();
        let hits = st.chats.iter().filter(|c| c.title.to_lowercase().contains(&q)).take(8).fold(
            Column::new().spacing(2),
            |col, c| {
                let line = row![
                    avatar(&c.title, 24.0),
                    txt(c.title.clone(), 13.0).color(p.text),
                    Space::new().width(Fill),
                    txt(network_name(&c.account_id), 11.0).color(p.sub)
                ]
                .spacing(10)
                .align_y(Center);
                col.push(
                    button(line)
                        .padding([6, 10])
                        .width(Fill)
                        .style(ghost(p, 8.0))
                        .on_press(UiMessage::ExecuteCommandChat(c.id.clone())),
                )
            },
        );
        let body = column![
            row![ic(icon::SEARCH, 16.0).color(p.sub), input].spacing(8).align_y(Center).padding([0, 12]),
            divider(p, 0.0),
            container(hits).padding(6)
        ];
        container(body).width(560).style(shadow_box(p.elevated, p.outline, 12.0, 30.0)).into()
    }

    fn view_quick_look<'a>(&self, title: &str, detail: &str) -> El<'a> {
        let p = &self.pal;
        let head = closable_head(p, title, 14.0, UiMessage::CloseModals);
        container(column![head, txt(detail.to_string(), 13.0).color(p.sub)].spacing(10).padding(16))
            .width(420)
            .style(shadow_box(p.elevated, p.outline, 12.0, 30.0))
            .into()
    }

    pub fn view_settings(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let me = st.me();
        let name = me.full_name.clone().expect("self name");
        let search_input = text_input("Search Settings", &self.settings_query)
            .on_input(UiMessage::SettingsQueryChanged)
            .size(13)
            .padding([5, 2])
            .style(input_style(CLEAR, p.strong, p.text, p.accent, 0.0));
        let search = container(
            row![ic(icon::SEARCH, 15.0).color(p.text), search_input].spacing(8).align_y(Center).padding([2, 10]),
        )
        .style(fill_box(p.search, 38.0));
        let free = semi("Free", 11.0).color(p.text);
        let profile = row![
            self_avatar(&name, 28.0),
            column![bold(name.clone(), 14.0).color(p.strong),]
                .push(me.email.clone().map(|e| txt(e, 11.0).color(p.text)))
                .width(Fill),
            free
        ]
        .spacing(10)
        .align_y(Center)
        .padding([2, 8]);
        let nav = SettingsTab::ALL.iter().fold(Column::new(), |c, &tab| {
            let (label, glyph, rgb) = tab.meta();
            let on = tab == self.active_settings_tab && self.settings_query.is_empty();
            let tile = container(ic(glyph, 13.0).color(Color::WHITE))
                .center(20)
                .style(fill_box(Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8), 6.0));
            let count = (tab == SettingsTab::Accounts).then(|| txt(st.accounts.len().to_string(), 13.0).color(p.sub));
            let line = row![tile, txt(label, 14.0).color(p.strong), Space::new().width(Fill)]
                .push(count)
                .spacing(8)
                .align_y(Center)
                .padding([12, 8]);
            let next_on = SettingsTab::ALL.get(tab as usize + 1).is_some_and(|n| *n == self.active_settings_tab);
            c.push(
                button(line)
                    .padding(0)
                    .width(Fill)
                    .style(btn_style([if on { p.nav_sel } else { CLEAR }, p.hover, p.nav_sel], [p.text; 3], 8.0))
                    .on_press(UiMessage::SelectSettingsTab(tab)),
            )
            .push((!on && !next_on).then(|| divider(p, 36.0)))
        });
        let left = column![
            container(semi("Settings", 14.0).color(p.strong)).padding([4, 8]),
            search,
            profile,
            scrollable(nav).height(Fill).style(scroll_style(p.divider, p.weak))
        ]
        .spacing(8)
        .width(270)
        .padding(Padding::new(8.0).right(0.0));
        let content = if self.settings_query.trim().is_empty() {
            self.settings_content(&st)
        } else {
            self.settings_search_results()
        };
        let right = container(
            scrollable(container(content).max_width(600).padding(16).center_x(Fill))
                .height(Fill)
                .style(scroll_style(p.divider, p.weak)),
        )
        .width(Fill)
        .height(Fill)
        .style(fill_box(p.prefs_pane, 10.0));
        container(row![left, container(right).padding(8)]).width(Fill).height(Fill).style(fill_box(p.frame, 0.0)).into()
    }

    fn settings_search_results(&self) -> El<'_> {
        let p = &self.pal;
        let hits = search_settings_catalog(&self.settings_query).into_iter().map(|(page, r)| {
            let tab = SettingsTab::for_section(page);
            button(column![txt(r.label, 13.0).color(p.text), txt(page, 12.0).color(p.sub)].spacing(2))
                .padding([10, 12])
                .width(Fill)
                .style(ghost(p, 6.0))
                .on_press(UiMessage::SelectSettingsTab(tab))
                .into()
        });
        group(p, Some("Results"), hits.collect())
    }

    fn settings_content(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let s = &st.settings;
        let groups: Vec<El<'_>> = match self.active_settings_tab {
            SettingsTab::Accounts => {
                let connected = st
                    .bridge_accounts
                    .iter()
                    .map(|b| {
                        pref_row(
                            p,
                            b.network_name.clone(),
                            Some(&format!(
                                "{} · {}",
                                b.state.to_lowercase(),
                                match b.mode {
                                    BridgeMode::OnDevice => "on device",
                                    BridgeMode::Cloud => "cloud",
                                }
                            )),
                            Some(svg(brand_svg(brand_key(&b.account_id))).width(20).height(20).into()),
                            txt("Connected", 12.0).color(p.success).into(),
                        )
                    })
                    .collect();
                let more = vec![login::network_grid(&st.bridge_accounts, "", p.strong)];
                vec![group(p, Some("Connected accounts"), connected), group(p, Some("Add an Account"), more)]
            }
            SettingsTab::General => self.pref_items(prefs::GENERAL, s),
            SettingsTab::Appearance => self.pref_items(prefs::APPEARANCE, s),
            SettingsTab::Notifications => self.pref_items(prefs::NOTIFICATIONS, s),
            SettingsTab::Hotkeys => vec![group(
                p,
                None,
                HOTKEYS
                    .iter()
                    .map(|(a, k)| {
                        pref_row(
                            p,
                            *a,
                            None,
                            None,
                            container(med(*k, 12.0).color(p.text))
                                .padding([3, 8])
                                .style(bordered_box(p.hover, p.divider, 1.0, 6.0))
                                .into(),
                        )
                    })
                    .collect(),
            )],
            SettingsTab::Devices => {
                let code = text_input("Recovery code", &self.recovery_code_input)
                    .on_input(UiMessage::RecoveryInputChanged)
                    .on_submit(UiMessage::VerifyRecoveryCode)
                    .size(13)
                    .padding([6, 10])
                    .width(200)
                    .style(input_style(p.input, p.strong, p.sub, p.accent, 6.0));
                vec![
                    group(
                        p,
                        Some("This device"),
                        vec![pref_row(
                            p,
                            format!("{:?} · {:?}", self.os, self.display_server),
                            Some(APP_VERSION),
                            None,
                            txt(if st.beeper_state.verified { "Verified" } else { "Not verified" }, 12.0)
                                .color(if st.beeper_state.verified { p.success } else { p.warning })
                                .into(),
                        )],
                    ),
                    group(
                        p,
                        Some("Encryption"),
                        vec![
                            pref_row(
                                p,
                                "Recovery code",
                                Some("Enter your recovery code to verify this device."),
                                None,
                                code.into(),
                            ),
                            pref_row(
                                p,
                                "Verify with another device",
                                None,
                                None,
                                pill_btn(p, "Verify", UiMessage::VerifySasNow),
                            ),
                        ],
                    ),
                ]
            }
            SettingsTab::Ai => Vec::new(),
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
            SettingsTab::Integrations => vec![group(
                p,
                Some("Desktop API"),
                vec![
                    pref_row(
                        p,
                        "Enable Desktop API",
                        Some("Local REST and WebSocket server for scripts and assistants. Restart Beeper to apply."),
                        None,
                        toggle(p, s.enable_pas_api, Some(UiMessage::ToggleSetting(|x| x.enable_pas_api ^= true))),
                    ),
                    pref_row(
                        p,
                        "Address",
                        None,
                        None,
                        txt(format!("http://localhost:{}", s.pas_port), 12.0).color(p.sub).into(),
                    ),
                ],
            )],
            SettingsTab::Storage => vec![group(
                p,
                None,
                vec![
                    pref_row(
                        p,
                        "Media cache",
                        Some("Remove cached media older than 30 days."),
                        None,
                        pill_btn(p, "Clear", UiMessage::ClearOldMedia),
                    ),
                    pref_row(
                        p,
                        "Export current chat",
                        None,
                        None,
                        pill_btn(
                            p,
                            "Export",
                            UiMessage::ExportChatJson(self.selected_chat_id.clone().expect("a chat is selected")),
                        ),
                    ),
                    pref_row(p, "Data folder", Some(&st.data_dir.display().to_string()), None, Space::new().into()),
                ],
            )],
            SettingsTab::Help => vec![group(
                p,
                None,
                ["Help center", "Report a problem", "Join the community"]
                    .into_iter()
                    .map(|h| pref_row(p, h, None, None, ic(icon::ARROW_UP_RIGHT, 14.0).color(p.sub).into()))
                    .collect(),
            )],
            SettingsTab::About => vec![group(
                p,
                None,
                vec![pref_row(
                    p,
                    "beeper-rs",
                    Some("Not affiliated with Beeper Inc. or Automattic Inc. MIT license."),
                    None,
                    txt(APP_VERSION, 12.0).color(p.sub).into(),
                )],
            )],
        };
        let banner = (!self.status_banner.is_empty()).then(|| txt(self.status_banner.clone(), 12.0).color(p.sub));
        column![].push(banner).extend(groups).spacing(28).into()
    }

    fn pref_items(&self, items: &'static [prefs::Item], s: &BeeperSettings) -> Vec<El<'_>> {
        let p = &self.pal;
        let mut heading = None;
        let mut out = Vec::new();
        for item in items {
            match *item {
                prefs::Item::Heading(h) => heading = Some(h),
                prefs::Item::Subtitle(t) => out.push(match heading.take() {
                    Some(h) => group(p, Some(h), vec![container(txt(t, 13.0).color(p.sub)).padding([13, 14]).into()]),
                    None => container(txt(t, 13.0).color(p.sub)).padding([0, 14]).into(),
                }),
                prefs::Item::Group(rows) => {
                    out.push(group(p, heading.take(), rows.iter().map(|r| self.pref_control(r, s)).collect()))
                }
                prefs::Item::Theme => out.push(group(p, heading.take(), vec![theme_cards(p, s.theme)])),
            }
        }
        out
    }

    fn pref_select(&self, r: &'static prefs::Pref, s: &BeeperSettings) -> El<'_> {
        let prefs::Control::Select(options, _) = r.control else { panic!("{} is not a select", r.key) };
        let key = r.key;
        select(&self.pal, options, options[s.pref(key) as usize], move |o| {
            UiMessage::SetPref(key, options.iter().position(|x| *x == o).expect("listed option") as u8)
        })
    }

    fn pref_control(&self, r: &'static prefs::Pref, s: &BeeperSettings) -> El<'_> {
        let p = &self.pal;
        let locked = r.badge == Some(prefs::Badge::Plus);
        let control = match r.control {
            prefs::Control::Toggle(_) => {
                let v = s.pref(r.key);
                toggle(p, v == 1, (!locked).then_some(UiMessage::SetPref(r.key, 1 - v)))
            }
            prefs::Control::Select(..) => self.pref_select(r, s),
            prefs::Control::Slider(_) => {
                let key = r.key;
                slider(0..=100u8, s.pref(key), move |x| UiMessage::SetPref(key, x))
                    .width(200)
                    .style(slider_style(p))
                    .into()
            }
            prefs::Control::Archive(scope, age) => {
                let n = self.store.read().unwrap().stale_inbox_chats().len();
                let left = row![
                    txt(scope.label, 13.0).color(p.text),
                    self.pref_select(scope, s),
                    txt(age.label, 13.0).color(p.text),
                    self.pref_select(age, s),
                ]
                .spacing(8)
                .align_y(Center)
                .wrap()
                .vertical_spacing(8);
                return row![
                    container(left).width(Fill),
                    pill_btn(p, format!("Archive {n} chats"), UiMessage::ArchiveStale)
                ]
                .spacing(8)
                .align_y(Center)
                .padding([13, 14])
                .into();
            }
        };
        pref_row(p, r.label, r.desc, r.badge.map(|b| badge(p, b)), control)
    }
}

pub fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("png header").write_image_data(rgba).expect("png data");
}

pub fn run_native_gui(store: SharedStore) -> iced::Result {
    let app = iced::daemon(move || BeeperGuiApp::boot(store.clone()), BeeperGuiApp::update, BeeperGuiApp::view)
        .title(BeeperGuiApp::title)
        .subscription(BeeperGuiApp::subscription)
        .theme(|a: &BeeperGuiApp, _| a.iced_theme())
        .scale_factor(|a: &BeeperGuiApp, _| a.zoom)
        .style(|a: &BeeperGuiApp, _| iced::theme::Style { background_color: a.pal.frame, text_color: a.pal.text })
        .default_font(fw(Weight::Normal));
    ALL_FONTS.into_iter().fold(app, |app, f| app.font(f)).run()
}

pub mod login;
pub mod style;

use crate::login::{Login, LoginMsg};

use crate::style::{
    bold, bordered_box, brand_svg, btn_style, clamp, ell, ensure_fonts_loaded, fill_box, fw, hex, ic, icon, initials,
    input_style, med, mix, name_hash, pick, pick_menu_style, pick_style, scroll_style, semi, shadow_box, txt, Palette,
    ALL_FONTS, AVATAR_GRADIENTS, CLEAR,
};
use beeper_core::cloud::Cloud;
use beeper_core::config::{account_network, civil_from_days, millis_from_rfc3339, now_unix_secs, APP_VERSION};
use beeper_core::import::has_beeper_data;
use beeper_core::matrix::DeviceInfo;
use beeper_core::matrix::Outgoing;
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
    button, column, container, image, mouse_area, operation, pick_list, pin, rich_text, row, scrollable, slider, span,
    stack, svg, text_input, Column, Row, Space, Stack,
};
use iced::{
    border, mouse, window, Background, Bottom, Center, Color, Element, Event, Fill, Gradient, Padding, Pixels, Point,
    Radians, Right, Shrink, Size, Subscription, Task, Theme,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};

static AVATAR_SLOTS: Semaphore = Semaphore::const_new(8);

pub type El<'a> = Element<'a, UiMessage>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxTab {
    AllChats,
    LowPriority,
    Archived,
    Reminders,
    SendLater,
    Requests,
}

impl InboxTab {
    pub const fn label(self) -> &'static str {
        ["Inbox", "Low Priority", "Archive", "Reminders", "Send Later", "Requests"][self as usize]
    }

    const fn fallback(self) -> Option<[Self; 2]> {
        match self {
            Self::AllChats => Some([Self::Archived, Self::LowPriority]),
            Self::LowPriority => Some([Self::AllChats, Self::Archived]),
            Self::Archived => Some([Self::AllChats, Self::LowPriority]),
            _ => None,
        }
    }

    const fn filterable(self) -> bool {
        !matches!(self, Self::Reminders | Self::SendLater)
    }

    const fn empty(self) -> (&'static str, &'static str) {
        [
            ("You've hit inbox zero!", ""),
            ("No messages in Low Priority", ""),
            ("Your Archive is empty", ""),
            (
                "You don't have any Reminders set",
                "Set one up by right-clicking a chat in the inbox and selecting Remind Me",
            ),
            (
                "You don't have any Send Later messages",
                "Send a message later by clicking the clock icon in the composer",
            ),
            ("You have no messages", ""),
        ][self as usize]
    }
}

const FILTERS: [(&str, char); 8] = [
    ("Unread", icon::MARK_EMAIL_UNREAD),
    ("Drafts", icon::SMS),
    ("Unanswered", icon::MODE_COMMENT),
    ("DMs", icon::PERSON),
    ("Groups", icon::GROUP),
    ("Archived", icon::ARCHIVE),
    ("Requests", icon::EMAIL),
    ("Low Priority", icon::LOW_PRIORITY_SPACE),
];
const F_ARCHIVED: usize = 5;
const F_REQUESTS: usize = 6;
const F_LOW_PRIORITY: usize = 7;

fn filter_hit(i: usize, c: &Chat, st: &BeeperStore) -> bool {
    match i {
        0 => c.flags & CHAT_UNREAD != 0,
        1 => c.draft.is_some(),
        2 => {
            st.chat_msg_indices.get(&c.id).and_then(|v| v.last()).map(|&i| &st.messages[i]).is_none_or(|m| {
                m.is_sender != Some(true) && !m.reactions.iter().any(|r| r.participant_id == st.me().id)
            })
        }
        3 => c.chat_type != "group",
        4 => c.chat_type == "group",
        F_ARCHIVED => c.flags & CHAT_ARCHIVED != 0,
        F_REQUESTS => c.flags & CHAT_REQUEST != 0,
        F_LOW_PRIORITY => c.flags & CHAT_LOW_PRIORITY != 0,
        _ => unreachable!("filter index {i}"),
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

#[derive(Clone, Copy)]
pub enum TabIcon {
    Glyph(char),
    Svg(&'static [u8]),
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

    pub const fn meta(self) -> (&'static str, TabIcon, u32) {
        use TabIcon::{Glyph, Svg};
        [
            ("Accounts", Glyph(icon::AT_SIGN), 0x9647e9),
            ("General", Svg(include_bytes!("../assets/settings/general.svg")), 0x16b2ee),
            ("Appearance", Svg(include_bytes!("../assets/settings/appearance.svg")), 0xd736c9),
            ("Notifications", Svg(include_bytes!("../assets/settings/notifications.svg")), 0x1fb93d),
            ("Hotkeys", Svg(include_bytes!("../assets/settings/hotkeys.svg")), 0xeaa900),
            ("Devices", Svg(include_bytes!("../assets/settings/devices.svg")), 0xf54d0b),
            ("AI", Svg(include_bytes!("../assets/settings/ai.svg")), 0x1fc8d3),
            ("Beeper Plus", Svg(include_bytes!("../assets/settings/beeper_plus.svg")), 0x2a61fb),
            ("Integrations", Glyph(icon::PUZZLE), 0x7459ff),
            ("Storage", Glyph(icon::DATABASE), 0x35556a),
            ("Help", Svg(include_bytes!("../assets/settings/help.svg")), 0x9a9aa1),
            ("About", Svg(include_bytes!("../assets/settings/about.svg")), 0x20b4a1),
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
    DismissBanner,
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
    FilterButton,
    PickFilter(u8),
    SearchChanged(String),
    ClearSearch,
    ToggleCommandPalette,
    CommandQueryChanged(String),
    ExecuteCommandChat(String),
    OpenSettings(SettingsTab),
    ToggleAddAccount,
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
    AvatarLoaded(String, Result<image::Handle, String>),
    MediaLoaded(String, Result<Frames, String>),
    AnimationTick,
    RecoveryInputChanged(String),
    VerifyRecoveryCode,
    Verified(Result<(), String>),
    Pushed(Result<(), String>),
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    AskLogout(bool),
    Logout,
    LoggedOut(Result<(), String>),
    VerifySasNow,
    SasMatch(bool),
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
    WindowResized(window::Id, Size),
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
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const HOTKEYS: [(&str, &str); 8] = [
    ("Command bar", "Ctrl K"),
    ("Open settings", "Ctrl ,"),
    ("Search chats", "Ctrl F"),
    ("Filter: Unread", "Ctrl Shift Y"),
    ("Toggle chat info", "Ctrl ."),
    ("Next chat", "Alt ↓"),
    ("Previous chat", "Alt ↑"),
    ("Close popups", "Esc"),
];

fn network_name(account_id: &str) -> &'static str {
    account_network(account_id).display_name
}

const RAIL_RANK: [&str; 14] = [
    "hungryserv",
    "whatsapp",
    "instagram",
    "telegram",
    "gmessages",
    "googlechat",
    "facebook",
    "signal",
    "linkedin",
    "twitter",
    "discord",
    "slack",
    "gvoice",
    "imessage",
];

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
            format!("{}, {} {dd}{year}", &wd[..3], &MONTHS[m as usize - 1][..3])
        }
    }
}

fn list_ts(iso: &str, h24: bool) -> String {
    const DAY: i64 = 86_400_000;
    let (d, now) = (day_num(iso), now_unix_secs() * 1000);
    let age = now - millis_from_rfc3339(iso).expect("RFC 3339 timestamp");
    let (y, m, dd) = civil_from_days(d);
    let month = MONTHS[m as usize - 1];
    match now.div_euclid(DAY) - d {
        0 => format_short_ts(iso, h24),
        1 => "Yesterday".into(),
        _ if age < 7 * DAY => WEEKDAYS[(d + 4).rem_euclid(7) as usize].into(),
        _ if age < 365 * DAY => format!("{month} {dd}"),
        _ => format!("{} {dd}, {y}", &month[..3]),
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

fn count_badge<'a>(n: u32, bg: Color, h: f32) -> El<'a> {
    let n = if n > 0 { n.to_string() } else { String::new() };
    container(semi(n, 10.0).color(Color::WHITE)).padding([0, 4]).center_y(h).style(fill_box(bg, h / 2.0)).into()
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

fn ago(ms: u64) -> String {
    let secs = (now_unix_secs() - (ms / 1000) as i64).max(0) as f64;
    let units = [(60.0, 60.0, "m"), (3600.0, 24.0, "h"), (86400.0, 30.0, "d"), (2_592_000.0, 12.0, "mo")];
    if secs < 60.0 {
        return "now".into();
    }
    let (size, unit) =
        units.iter().find(|(size, max, _)| secs / size < *max).map_or((31_536_000.0, "y"), |u| (u.0, u.2));
    format!("{}{unit} ago", (secs / size).round())
}

fn divider<'a>(p: &Palette, left: f32) -> El<'a> {
    container(container(Space::new()).width(Fill).height(1).style(fill_box(p.divider.scale_alpha(0.4), 0.0)))
        .padding(Padding::ZERO.left(left))
        .into()
}

fn avatar<'a>(name: &str, size: f32, p: &Palette) -> El<'a> {
    let Some(label) = initials(name) else {
        let shade = fill_box(Color { a: 0.1, ..p.strong }, size / 2.0);
        return container(svg(EYES.clone()).width(size).height(size)).style(shade).into();
    };
    let &(bottom, top) = pick(name, &AVATAR_GRADIENTS);
    let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, hex(top)).add_stop(1.0, hex(bottom));
    container(med(label, size * 0.5).color(Color::WHITE))
        .center(size)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(g))),
            border: border::rounded(size / 2.0),
            ..Default::default()
        })
        .into()
}

fn round_avatar(bytes: &[u8]) -> Result<image::Handle, String> {
    const S: u32 = 96;
    let img = ::image::load_from_memory(bytes).map_err(|e| format!("Could not decode the image: {e}"))?;
    let mut px = img.resize_to_fill(S, S, ::image::imageops::FilterType::Triangle).into_rgba8();
    let r = S as f32 / 2.0;
    for (x, y, p) in px.enumerate_pixels_mut() {
        let d = (x as f32 + 0.5 - r).hypot(y as f32 + 0.5 - r);
        p[3] = (f32::from(p[3]) * (r - d + 0.5).clamp(0.0, 1.0)) as u8;
    }
    Ok(image::Handle::from_rgba(S, S, px.into_raw()))
}

/// Image frames with their delays in milliseconds; a still image has one frame.
type Frames = Vec<(image::Handle, u128)>;

fn decode_media(bytes: Vec<u8>) -> Result<Frames, String> {
    use ::image::AnimationDecoder;
    if !bytes.starts_with(b"GIF8") {
        return Ok(vec![(image::Handle::from_bytes(bytes), 0)]);
    }
    let gif = ::image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let frames = gif.into_frames().collect_frames().map_err(|e| format!("Could not decode the GIF: {e}"))?;
    assert!(!frames.is_empty(), "a decoded GIF has frames");
    let frame = |f: ::image::Frame| {
        let (n, d) = f.delay().numer_denom_ms();
        let px = f.into_buffer();
        // Browsers play delays of 10 ms or less at 100 ms.
        let ms = match n / d {
            0..=10 => 100,
            ms => ms,
        };
        (image::Handle::from_rgba(px.width(), px.height(), px.into_raw()), u128::from(ms))
    };
    Ok(frames.into_iter().map(frame).collect())
}

fn current_frame(frames: &Frames, elapsed_ms: u128) -> image::Handle {
    let total: u128 = frames.iter().map(|f| f.1).sum();
    if total == 0 {
        return frames[0].0.clone();
    }
    let mut t = elapsed_ms % total;
    let (handle, _) = frames
        .iter()
        .find(|(_, ms)| t.checked_sub(*ms).map(|rest| t = rest).is_none())
        .expect("the time is below the total delay");
    handle.clone()
}

fn faces(chat: &Chat) -> (Vec<&User>, usize) {
    let others: Vec<&User> = chat.participants.items.iter().filter(|u| u.is_self != Some(true)).collect();
    let group = chat.chat_type == "group";
    let more = if group && others.len() > 5 { others.len() - 4 } else { 0 };
    let shown = if group { others.len() - more } else { others.len().min(1) };
    (others.into_iter().take(shown).collect(), more)
}

fn chat_avatar_urls(chat: &Chat) -> Vec<&str> {
    match chat.img_url.as_deref() {
        Some(u) => vec![u],
        None => faces(chat).0.into_iter().filter_map(|u| u.img_url.as_deref()).collect(),
    }
}

fn network_bot(chat: &Chat, user_id: &str) -> bool {
    chat.participants.items.iter().any(|u| u.id == user_id && u.is_network_bot == Some(true))
}

fn preview_sender<'a>(chat: &'a Chat, m: &Message) -> Option<&'a User> {
    let shown = (chat.chat_type == "group" || network_bot(chat, &m.sender_id))
        && m.is_sender != Some(true)
        && m.message_type.as_deref() != Some("REACTION");
    chat.participants.items.iter().find(|u| shown && u.id == m.sender_id)
}

fn badged<'a>(inner: El<'a>, account_id: &str, size: f32, badge: f32) -> El<'a> {
    let icon = svg(brand_svg(brand_key(account_id))).width(badge).height(badge);
    let w = size + badge / 8.0;
    stack![inner, container(icon).width(w).height(size).align_right(w).align_bottom(size)].width(w).height(size).into()
}

fn slider_style(p: &Palette) -> impl Fn(&Theme, slider::Status) -> slider::Style + use<> {
    let (fill, track, edge) = (p.range, p.range_track, p.range_edge);
    move |_, _| slider::Style {
        rail: slider::Rail {
            backgrounds: (fill.into(), track.into()),
            width: 8.0,
            border: iced::Border { color: edge, width: 1.0, radius: 4.0.into() },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 8.0 },
            background: fill.into(),
            border_width: 0.0,
            border_color: CLEAR,
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
        size: Some(16.0.into()),
        line_height: iced::widget::text::LineHeight::Relative(1.0),
        shaping: iced::widget::text::Shaping::Basic,
    };
    pick_list(options, Some(selected), f)
        .text_size(11)
        .text_line_height(Pixels(16.0))
        .font(fw(Weight::Semibold))
        .handle(pick_list::Handle::Static(chevron))
        .padding([4, 8])
        .style(pick_style(p.chip, p.chip, p.strong, p.strong))
        .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border))
        .into()
}

fn pill_btn<'a>(p: &Palette, label: impl ToString, msg: UiMessage) -> El<'a> {
    let label = txt(label.to_string(), 13.0).line_height(1.0).wrapping(iced::widget::text::Wrapping::None);
    let (bg, fg, edge) = (p.button, p.strong, p.button_border);
    button(label)
        .padding([4, 6])
        .style(move |_, _| button::Style {
            background: Some(bg.into()),
            text_color: fg,
            border: iced::Border { color: edge, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        })
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

static DASHED_RING: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    ["#000", "#fff"].map(|c| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="1" y="1" width="30" height="30" rx="7" fill="none" stroke="{c}" stroke-opacity=".15" stroke-width="2" stroke-dasharray="4 3"/></svg>"#).into_bytes())
    })
});

static SENT: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2" stroke-dasharray="2.4 1.84" transform="rotate(-79 6 6)"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..])
});

static EYES: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 46" fill="#fff"><path d="M22.1197 25.5092C23.0085 30.1093 20.937 33.1 16.2324 34.009C11.5277 34.918 8.49126 32.9143 7.60244 28.3142L6.68822 23.5827C5.7994 18.9826 7.87085 15.9919 12.5755 15.0829C17.2802 14.1738 20.3166 16.1776 21.2054 20.7777L22.1197 25.5092ZM14.8746 30.5902C15.1743 32.1411 16.6446 33.1113 18.2307 32.8049C19.8168 32.4984 20.8199 31.0503 20.5202 29.4994C20.2205 27.9485 18.7503 26.9783 17.1641 27.2847C15.578 27.5912 14.575 29.0393 14.8746 30.5902Z"/><path d="M39.1455 22.1235C40.0344 26.7236 37.9629 29.7142 33.2583 30.6233C28.5536 31.5323 25.5171 29.5286 24.6283 24.9284L23.7141 20.1969C22.8253 15.5968 24.8967 12.6061 29.6014 11.6971C34.306 10.7881 37.3425 12.7918 38.2313 17.3919L39.1455 22.1235ZM31.9005 27.2045C32.2002 28.7554 33.6704 29.7256 35.2566 29.4191C36.8427 29.1126 37.8457 27.6645 37.5461 26.1136C37.2464 24.5627 35.7761 23.5925 34.19 23.899C32.6039 24.2055 31.6008 25.6536 31.9005 27.2045Z"/></svg>"##[..])
});

static REPLY_CURVE: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    ["", r#" transform="matrix(-1 0 0 1 26 0)""#].map(|flip| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0.5 0.5 25 19"><path{flip} fill="currentColor" fill-rule="evenodd" d="M17 15.5C9.544 15.5 3.5 9.456 3.5 2V.5h-3V2c0 9.113 7.387 16.5 16.5 16.5h6a1.5 1.5 0 0 0 0-3z"/></svg>"#).into_bytes())
    })
});

static BELL_OFF: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742"/><path d="m2 2 20 20"/><path d="M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"/></svg>"##[..])
});

static TRASH: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><path fill="currentColor" d="M432 32H312l-9.4-18.7A24 24 0 0 0 281.1 0H166.8a23.72 23.72 0 0 0-21.4 13.3L136 32H16A16 16 0 0 0 0 48v32a16 16 0 0 0 16 16h416a16 16 0 0 0 16-16V48a16 16 0 0 0-16-16zM53.2 467a48 48 0 0 0 47.9 45h245.8a48 48 0 0 0 47.9-45L416 128H32z"/></svg>"#[..])
});

fn gap_below(m: &Message, next: &Message) -> f32 {
    let ms = |m: &Message| millis_from_rfc3339(&m.timestamp).expect("message timestamp");
    match (ms(next) - ms(m)).abs() / 60_000 {
        _ if m.timestamp[..10] != next.timestamp[..10] => 0.0,
        241.. => 26.0,
        6.. => 13.0,
        2.. => 4.0,
        _ if m.sender_id == next.sender_id => 2.0,
        _ => 4.0,
    }
}

static LOGO: std::sync::LazyLock<svg::Handle> =
    std::sync::LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/beeper.svg")));

fn tinted<'a>(h: &svg::Handle, size: f32, color: Color) -> El<'a> {
    svg(h.clone()).width(size).height(size).style(move |_, _| svg::Style { color: Some(color) }).into()
}

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
    column![head.spacing(6).align_y(Center).padding(13)]
        .push(desc.map(|d| {
            let paras = d.split("\n\n").map(|t| txt(t.to_string(), 13.0).line_height(Pixels(18.0)).color(p.sub).into());
            Column::with_children(paras).spacing(13).padding(Padding::new(13.0).top(0.0))
        }))
        .into()
}

fn group<'a>(p: &Palette, title: Option<&str>, rows: Vec<El<'a>>) -> El<'a> {
    let card = container(Column::with_children(rows)).width(Fill).padding(1).style(bordered_box(
        p.prefs_group,
        p.prefs_border,
        1.0,
        8.0,
    ));
    column![]
        .push(
            title
                .map(|t| container(txt(t.to_string(), 13.0).line_height(Pixels(16.0)).color(p.muted)).padding([0, 13])),
        )
        .push(card)
        .spacing(6)
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
    pub filters: u8,
    pub show_attach_menu: bool,
    pub command_palette_query: String,
    pub composer_text: String,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub nickname_edit_buffer: String,
    pub recovery_code_input: String,
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
    pub confirm_logout: bool,
    pub show_command_palette: bool,
    pub active_settings_tab: SettingsTab,
    pub settings_query: String,
    pub adding_account: bool,
    pub show_right_panel: bool,
    pub quick_look_target: Option<(String, String)>,
    pub expanded_history_msg_id: Option<String>,
    pub hovered_msg: Option<String>,
    pub status_banner: String,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    pub dragging: bool,
    pub main_window: Option<window::Id>,
    main_width: f32,
    pub settings_window: Option<window::Id>,
    pub login_window: Option<window::Id>,
    pub login: Login,
    pub cloud: Arc<Cloud>,
    pub avatars: HashMap<String, Option<image::Handle>>,
    pub media: HashMap<String, Option<Frames>>,
    started: std::time::Instant,
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
            filters: 0,
            show_attach_menu: false,
            command_palette_query: String::new(),
            composer_text: String::new(),
            reply_to_message_id: None,
            editing_message_id: None,
            nickname_edit_buffer: String::new(),
            recovery_code_input: String::new(),
            devices: None,
            confirm_logout: false,
            show_command_palette: false,
            active_settings_tab: SettingsTab::General,
            settings_query: String::new(),
            adding_account: false,
            show_right_panel: panel,
            quick_look_target: None,
            expanded_history_msg_id: None,
            hovered_msg: None,
            status_banner: String::new(),
            dnd_muted: false,
            sidebar_width: 244.0,
            dragging: false,
            main_window: None,
            main_width: 1264.0,
            settings_window: None,
            login_window: None,
            login: Login::new(source),
            cloud,
            avatars: HashMap::new(),
            media: HashMap::new(),
            started: std::time::Instant::now(),
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
        let tasks =
            [open.map(UiMessage::WindowOpened), iced::system::theme().map(UiMessage::SystemTheme), app.load_avatars()];
        (app, Task::batch(tasks.into_iter().chain(cloud)))
    }

    fn load_avatars(&mut self) -> Task<UiMessage> {
        let st = self.store.read().unwrap();
        let Some(client) = st.matrix.clone() else { return Task::none() };
        let selected = self.selected_chat_id.as_ref().and_then(|id| st.chat_idx_by_id.get(id)).map(|&i| &st.chats[i]);
        let members = selected.into_iter().flat_map(|c| &c.participants.items).filter_map(|u| u.img_url.as_deref());
        let senders = st.chats.iter().filter_map(|c| {
            let &i = st.chat_msg_indices.get(&c.id)?.last()?;
            preview_sender(c, &st.messages[i])?.img_url.as_deref()
        });
        let urls = st.chats.iter().flat_map(chat_avatar_urls).chain(members).chain(senders);
        let urls: HashSet<String> = urls.filter(|u| !self.avatars.contains_key(*u)).map(str::to_string).collect();
        drop(st);
        self.avatars.extend(urls.iter().map(|u| (u.clone(), None)));
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (client.clone(), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the avatar semaphore stays open");
                    let bytes = beeper_core::matrix::thumbnail(client, u).await?;
                    tokio::task::spawn_blocking(move || round_avatar(&bytes)).await.expect("avatar worker thread")
                },
                move |r| UiMessage::AvatarLoaded(url, r),
            )
        }))
    }

    fn face<'a>(&self, url: Option<&str>, name: &str, size: f32) -> El<'a> {
        match url.and_then(|u| self.avatars.get(u)?.clone()) {
            Some(h) => image(h).width(size).height(size).into(),
            None => avatar(name, size, &self.pal),
        }
    }

    fn chat_avatar<'a>(&self, chat: &Chat, size: f32) -> El<'a> {
        fn user(u: &User) -> (Option<&str>, &str) {
            (u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default())
        }
        if chat.img_url.is_some() {
            let shade = fill_box(Color { a: 0.1, ..self.pal.strong }, size / 2.0);
            return container(self.face(chat.img_url.as_deref(), &chat.title, size)).style(shade).into();
        }
        let (shown, more) = faces(chat);
        if chat.chat_type != "group" || shown.is_empty() {
            let (url, name) = shown.first().map_or((None, ""), |u| user(u));
            return self.face(url, name, size);
        }
        const SLOTS: [(f32, f32, f32); 5] =
            [(-0.14, -0.135, 0.48), (0.26, 0.08, 0.35), (-0.06, 0.28, 0.27), (0.25, -0.24, 0.215), (-0.33, 0.2, 0.2)];
        let layers = shown.into_iter().zip(SLOTS).map(|(u, (x, y, k))| {
            let (url, name) = user(u);
            let d = size * k;
            pin(self.face(url, name, d)).x(size * (0.5 + x) - d / 2.0).y(size * (0.5 + y) - d / 2.0).into()
        });
        let label = if more > 98 { "99+".to_string() } else { format!("+{more}") };
        let font = size * 0.15 * if more > 9 { 0.85 } else { 1.0 };
        let count = (more > 0).then(|| {
            let t = container(med(label, font).color(self.pal.sub)).center_x(size * 0.3);
            pin(t).x(size * 0.027).y(size * 0.484)
        });
        Stack::with_children(layers).push(count).width(size).height(size).into()
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
        let load = self.load_devices();
        if let Some(id) = self.settings_window {
            return Task::batch([load, window::gain_focus(id)]);
        }
        let (id, open) = window::open(Self::window_settings(Size::new(793.0, 705.0)));
        self.settings_window = Some(id);
        Task::batch([load, open.map(UiMessage::WindowOpened)])
    }

    fn load_devices(&mut self) -> Task<UiMessage> {
        if self.active_settings_tab != SettingsTab::Devices {
            return Task::none();
        }
        let st = self.store.read().unwrap();
        match (st.matrix.clone(), st.config.demo) {
            (Some(client), _) => Task::perform(beeper_core::matrix::devices(client), UiMessage::DevicesLoaded),
            (None, true) => {
                let name = format!("beeper-rs {:?}", self.os);
                let me = DeviceInfo { name, last_seen_ms: None, verified: st.beeper_state.verified, current: true };
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
        self.chats_with(st, self.filters)
    }

    fn chats_with<'a>(&self, st: &'a BeeperStore, filters: u8) -> Vec<&'a Chat> {
        let q_ids =
            (!self.search_query.trim().is_empty()).then(|| st.chat_search_index.query_all_tokens(&self.search_query));
        let mut list: Vec<&Chat> = st
            .chats
            .iter()
            .enumerate()
            .filter(|(idx, c)| {
                let on = |i: usize| filters & 1 << i != 0;
                let shown = |i: usize| on(i) || !filter_hit(i, c, st);
                c.merged_into_chat_id.is_none()
                    && self.selected_account_filter.as_ref().is_none_or(|a| &c.account_id == a)
                    && q_ids.as_ref().is_none_or(|ids| ids.binary_search(&(*idx as u32)).is_ok())
                    && match self.active_tab {
                        InboxTab::AllChats => {
                            let hidden = if self.selected_account_filter.is_some() { 0 } else { CHAT_HIDDEN };
                            c.flags & hidden == 0 && shown(F_ARCHIVED) && shown(F_LOW_PRIORITY) && shown(F_REQUESTS)
                        }
                        InboxTab::Requests => c.flags & CHAT_REQUEST != 0,
                        InboxTab::Archived => c.flags & CHAT_ARCHIVED != 0 && shown(F_REQUESTS),
                        InboxTab::LowPriority => {
                            c.flags & CHAT_LOW_PRIORITY != 0 && shown(F_ARCHIVED) && shown(F_REQUESTS)
                        }
                        InboxTab::Reminders => c.reminder.is_some(),
                        InboxTab::SendLater => false,
                    }
                    && (!self.active_tab.filterable()
                        || filters == 0
                        || (0..FILTERS.len()).any(|i| on(i) && filter_hit(i, c, st)))
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
        let out = if let Some(target) = self.editing_message_id.take() {
            st.edit_message(&chat_id, &target, text.clone()).expect("edit message");
            Outgoing::Edit { target, text }
        } else {
            let reply_to = self.reply_to_message_id.take();
            let (_, txn, _) =
                st.send_message(&chat_id, Some(text.clone()), reply_to.clone(), None).expect("send message");
            st.patch_chat(&chat_id, true, None, None).expect("mark chat read");
            Outgoing::Send { txn, text, reply_to }
        };
        st.queue(&chat_id, out);
        self.composer_text.clear();
        drop(st);
        self.refresh_theme_and_tray();
    }

    fn push_outbox(&mut self) -> Task<UiMessage> {
        let outbox = std::mem::take(&mut self.store.write().unwrap().outbox);
        Task::batch(outbox.into_iter().map(|(chat, out)| {
            Task::perform(beeper_core::matrix::push(self.store.clone(), chat, out), UiMessage::Pushed)
        }))
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
            UiMessage::SelectTab(tab) => {
                self.active_tab = tab;
                self.show_filter_menu = false;
            }
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
                let mut st = self.store.write().unwrap();
                st.delete_message(&cid, &mid).expect("delete message");
                st.queue(&cid, Outgoing::Redact { target: mid });
            }
            UiMessage::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write().unwrap();
                let mine = st.messages[st.msg_idx_by_id[&mid]]
                    .reactions
                    .iter()
                    .find(|r| r.reaction_key == key && st.is_me(&r.participant_id))
                    .map(|r| r.id.clone());
                let out = match mine {
                    Some(target) => {
                        st.remove_reaction(&cid, &mid, &key).expect("remove reaction");
                        Outgoing::Redact { target }
                    }
                    None => {
                        let (_, txn) = st.add_reaction(&cid, &mid, &key, None).expect("add reaction");
                        Outgoing::React { txn, target: mid, key }
                    }
                };
                st.queue(&cid, out);
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
            UiMessage::SelectSettingsTab(t) => {
                self.active_settings_tab = t;
                return self.load_devices();
            }
            UiMessage::DevicesLoaded(r) => self.devices = Some(r),
            UiMessage::AskLogout(on) => self.confirm_logout = on,
            UiMessage::Logout => {
                self.confirm_logout = false;
                return Task::perform(beeper_core::matrix::logout(self.store.clone()), UiMessage::LoggedOut);
            }
            UiMessage::LoggedOut(Ok(())) => {
                let exe = std::env::current_exe().expect("path of the running beeper-rs");
                std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn().expect("restart beeper-rs");
                std::process::exit(0);
            }
            UiMessage::LoggedOut(Err(e)) => self.status_banner = format!("Could not log out: {e}"),
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
            UiMessage::AvatarLoaded(url, Ok(h)) => {
                self.avatars.insert(url, Some(h));
            }
            UiMessage::AvatarLoaded(url, Err(e)) => {
                self.status_banner = format!("Could not load the avatar {url}: {e}")
            }
            UiMessage::MediaLoaded(url, Ok(frames)) => {
                self.media.insert(url, Some(frames));
            }
            UiMessage::MediaLoaded(_, Err(e)) => self.status_banner = format!("Could not load an image: {e}"),
            UiMessage::AnimationTick => {}
            UiMessage::RecoveryInputChanged(v) => self.recovery_code_input = v,
            UiMessage::VerifyRecoveryCode => {
                let key = std::mem::take(&mut self.recovery_code_input);
                return Task::perform(beeper_core::matrix::recover(self.store.clone(), key), UiMessage::Verified);
            }
            UiMessage::Verified(Ok(())) => self.status_banner = "This device is verified.".into(),
            UiMessage::Verified(Err(e)) => self.status_banner = format!("Could not verify this device: {e}"),
            UiMessage::Pushed(Ok(())) => {}
            UiMessage::DismissBanner => self.status_banner.clear(),
            UiMessage::Pushed(Err(e)) => self.status_banner = format!("Could not send to Matrix: {e}"),
            UiMessage::VerifySasNow => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.store.write().unwrap().sas_confirm = Some(tx);
                return Task::perform(beeper_core::matrix::verify(self.store.clone(), rx), UiMessage::Verified);
            }
            UiMessage::SasMatch(same) => {
                let tx = self.store.write().unwrap().sas_confirm.take().expect("emojis are shown once");
                tx.send(same).ok();
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
            UiMessage::FilterButton => self.filters = u8::from(self.filters == 0),
            UiMessage::PickFilter(bit) => {
                self.filters = if self.filters == bit { 0 } else { bit };
                self.show_filter_menu = false;
            }
            UiMessage::OpenSettings(tab) => return self.open_settings(tab),
            UiMessage::ToggleAddAccount => self.adding_account ^= true,
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
            UiMessage::DragMoved(pt) => self.sidebar_width = (pt.x - 72.0).clamp(164.0, 724.0),
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
            UiMessage::WindowResized(id, size) => {
                if self.main_window == Some(id) {
                    self.main_width = size.width;
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
                let focus = window::gain_focus(self.main_window.or(self.login_window).expect("a window is open"));
                return Task::batch([focus, self.load_avatars(), self.load_media()]);
            }
            UiMessage::Login(m) => return self.update_login(m),
            UiMessage::KeyPressed(key, mods) => {
                let cmd = mods.command() || mods.control();
                match key.as_ref() {
                    Key::Character("k") if cmd => return self.update(UiMessage::ToggleCommandPalette),
                    Key::Character(",") if cmd => return self.open_settings(self.active_settings_tab),
                    Key::Character("f") if cmd => return self.update(UiMessage::ToggleSearch),
                    Key::Character("y" | "Y") if cmd && mods.shift() => self.filters ^= 1,
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
        if let Some(e) = self.store.write().unwrap().matrix_error.take() {
            self.status_banner = e;
        }
        Task::batch([self.load_avatars(), self.load_media(), self.push_outbox()])
    }

    fn load_media(&mut self) -> Task<UiMessage> {
        let st = self.store.read().unwrap();
        let (Some(client), Some(chat)) = (st.matrix.clone(), self.selected_chat_id.as_ref()) else {
            return Task::none();
        };
        let images = st.chat_msg_indices.get(chat).into_iter().flatten().flat_map(|&i| &st.messages[i].attachments);
        let urls: HashSet<String> = images
            .filter(|a| a.attachment_type == "img")
            .filter_map(|a| a.src_url.clone())
            .filter(|u| !self.media.contains_key(u))
            .collect();
        drop(st);
        self.media.extend(urls.iter().map(|u| (u.clone(), None)));
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (client.clone(), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the media semaphore stays open");
                    beeper_core::matrix::media(client, u).await.and_then(decode_media)
                },
                move |r| UiMessage::MediaLoaded(url, r),
            )
        }))
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
            Event::Window(window::Event::Resized(size)) => Some(UiMessage::WindowResized(id, size)),
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
        let animated = self.selected_chat_id.as_ref().is_some_and(|chat| {
            let st = self.store.read().unwrap();
            let mut urls =
                st.chat_msg_indices.get(chat).into_iter().flatten().flat_map(|&i| &st.messages[i].attachments);
            urls.any(|a| a.src_url.as_ref().and_then(|u| self.media.get(u)?.as_ref()).is_some_and(|f| f.len() > 1))
        });
        let tick =
            animated.then(|| iced::time::every(std::time::Duration::from_millis(20)).map(|_| UiMessage::AnimationTick));
        Subscription::batch([changes, focus, keys, system].into_iter().chain(drag).chain(tick))
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
        let menu = self.show_filter_menu.then(|| {
            mouse_area(
                container(mouse_area(self.view_filter_menu()).on_press(UiMessage::Noop))
                    .padding(Padding::ZERO.left(self.sidebar_width - 14.5).top(26.0))
                    .width(Fill)
                    .height(Fill),
            )
            .on_press(UiMessage::CloseModals)
            .on_right_press(UiMessage::CloseModals)
        });
        let toast = (!self.status_banner.is_empty()).then(|| {
            let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, p.sub, UiMessage::DismissBanner);
            let text = txt(self.status_banner.clone(), 13.0).color(p.strong);
            let card = row![text.wrapping(iced::widget::text::Wrapping::WordOrGlyph), close].spacing(8).align_y(Center);
            let card = container(card).padding(8).style(shadow_box(p.card, p.outline, 10.0, 15.0));
            container(card).width(Fill).align_bottom(Fill).padding(Padding::new(8.0).left(76.0))
        });
        stack![base].push(toast).push(menu).push(scrim).into()
    }

    fn view_filter_menu(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let item = |checked: bool, glyph: char, label: String, hotkey: &'static str, bit: u8| -> El<'_> {
            let line = row![
                container(checked.then(|| ic(icon::CHECK, 14.0))).center_x(14),
                container(ic(glyph, 14.0)).padding(Padding::ZERO.left(9.0).right(4.0)),
                txt(label, 13.0),
                Space::new().width(Fill),
                txt(hotkey, 13.0).color(Color { a: 0.5, ..p.strong })
            ]
            .align_y(Center);
            container(
                button(line)
                    .padding([3.125, 8.0])
                    .width(Fill)
                    .style(btn_style([CLEAR, p.accent, p.accent], [p.text, Color::WHITE, Color::WHITE], 4.0))
                    .on_press(UiMessage::PickFilter(bit)),
            )
            .padding([2, 5])
            .into()
        };
        let counted = (0..F_ARCHIVED).map(|i| {
            let n = self.chats_with(&st, 1 << i).len();
            let hotkey = if i == 0 { "Ctrl Shift Y" } else { "" };
            item(self.filters & 1 << i != 0, FILTERS[i].1, format!("{} ({n})", FILTERS[i].0), hotkey, 1 << i)
        });
        let views = (F_ARCHIVED..FILTERS.len())
            .map(|i| item(self.filters & 1 << i != 0, FILTERS[i].1, FILTERS[i].0.into(), "", 1 << i));
        let line = container(Space::new()).width(Fill).height(1).style(fill_box(Color { a: 0.1, ..p.strong }, 0.0));
        let rule = container(line).padding([5, 8]);
        let col = Column::with_children(
            std::iter::once(item(self.filters == 0, icon::INBOX, "All".into(), "", 0))
                .chain(counted)
                .chain(std::iter::once(rule.into()))
                .chain(views),
        );
        container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
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
        let g = |c: char| -> El<'static> { ic(c, 24.0).color(p.strong).into() };
        let ring = svg(DASHED_RING[usize::from(self.dark)].clone()).width(32).height(32);
        let add_space =
            container(stack![ring, container(ic(icon::MAPS_UGC, 24.0).color(p.strong)).center(32)]).center(36);
        let st = self.store.read().unwrap();
        let mut accounts: Vec<String> =
            std::iter::once("hungryserv".to_string()).chain(st.accounts.iter().map(|a| a.account_id.clone())).collect();
        accounts.sort_by_cached_key(|a| {
            let n = account_network(a);
            (
                RAIL_RANK.iter().position(|r| *r == n.bridge_id).unwrap_or(usize::MAX),
                n.display_name.to_lowercase(),
                a.clone(),
            )
        });
        let accounts = accounts.into_iter().fold(Column::new().spacing(8).align_x(Center), |c, acc| {
            let on = self.selected_account_filter.as_ref() == Some(&acc);
            let logo: El<'static> = svg(brand_svg(brand_key(&acc))).width(28).height(28).into();
            c.push(rail_btn(logo, on, UiMessage::SelectAccount((!on).then_some(acc))))
        });
        let top = column![
            tab("rail-inbox", g(icon::INBOX), InboxTab::AllChats),
            tab("rail-archive", g(icon::ARCHIVE), InboxTab::Archived),
            button(add_space).padding(0).style(ghost(p, 12.0)).on_press(UiMessage::ToggleCommandPalette),
            tab("rail-reminders", g(icon::SCHEDULE), InboxTab::Reminders),
            tab("rail-send-later", g(icon::SCHEDULE_SEND), InboxTab::SendLater),
            tab("rail-requests", g(icon::EMAIL), InboxTab::Requests),
            tab("rail-low-priority", g(icon::LOW_PRIORITY_SPACE), InboxTab::LowPriority),
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
            button(avatar(me, 36.0, p))
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
        let h24 = st.settings.pref("timeFormat") == 1;
        let previews = st.settings.on("showMessagePreviews");
        let names: Vec<&str> =
            (0..FILTERS.len()).filter(|i| self.filters & 1 << i != 0).map(|i| FILTERS[i].0).collect();
        let title = column![row![
            semi(self.active_tab.label(), 13.0).color(p.strong),
            ic(icon::UNFOLD_MORE, 13.0).color(p.weak)
        ]
        .spacing(4)
        .align_y(Center)]
        .push((!names.is_empty()).then(|| txt(names.join(", "), 10.0).color(p.sub)))
        .padding([0, 8]);
        let head_btn = |c: char, size: f32, w: f32, msg: UiMessage| {
            button(container(ic(c, size).color(p.strong)).center_x(w).center_y(26))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 8.0))
                .on_press(msg)
        };
        let filter = self.active_tab.filterable().then(|| {
            let active = self.filters != 0;
            let fg = if active { p.accent } else { p.strong };
            let trigger: El<'_> = match self.filters.count_ones() {
                0 => Space::new().into(),
                1 => ic(icon::EXPAND_MORE, 10.0).color(fg).into(),
                n => txt(n.to_string(), 11.0).color(fg).into(),
            };
            let trigger = button(container(trigger).padding(Padding::ZERO.left(6.0)).center_y(20))
                .padding(0)
                .width(if active { 16 } else { 0 })
                .style(btn_style([CLEAR; 3], [fg; 3], 0.0))
                .on_press(UiMessage::ToggleFilterMenu);
            let pill = row![trigger, container(ic(icon::FILTER_LIST, 19.0).color(fg)).center_x(27).center_y(20)];
            let (ring, hover) = (if active { p.accent } else { CLEAR }, p.hover);
            let face = move |_: &Theme, s: button::Status| button::Style {
                background: matches!(s, button::Status::Hovered | button::Status::Pressed).then(|| hover.into()),
                border: iced::Border { color: ring, width: 1.0, radius: 12.0.into() },
                ..Default::default()
            };
            mouse_area(button(pill).padding(0).style(face).on_press(UiMessage::FilterButton))
                .on_right_press(UiMessage::ToggleFilterMenu)
        });
        let header = row![title, Space::new().width(Fill)]
            .push(filter)
            .push(head_btn(icon::SEARCH, 23.0, 34.0, UiMessage::ToggleSearch))
            .push(head_btn(icon::EDIT_SQUARE, 20.0, 34.0, UiMessage::ToggleCommandPalette))
            .align_y(Center)
            .height(52)
            .padding(Padding::ZERO.right(2.0));
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
        let (mut pinned, rest): (Vec<&Chat>, Vec<&Chat>) =
            chats.iter().copied().partition(|c| pin_section && c.flags & CHAT_PINNED != 0);
        pinned.sort_by_key(|c| (st.pin_rank.get(&c.id).copied().unwrap_or(usize::MAX), c.last_activity.clone()));
        let cols = (((w - 16.0) / 80.0).floor() as usize).max(1);
        let cell = (w - 16.0) / cols as f32;
        let pins = pinned.chunks(cols).fold(Column::new(), |col, chunk| {
            col.push(chunk.iter().fold(Row::new(), |r, c| {
                let on = selected == Some(c.id.as_str());
                let muted = c.is_muted == Some(true);
                let first_word = c.chat_type != "group" && c.title.chars().any(char::is_alphabetic);
                let name = if first_word { c.title.split_whitespace().next().expect("a letter") } else { &c.title };
                let title = ell(name, cell - if muted { 35.2 } else { 20.0 }, 12.0, Weight::Semibold);
                let label =
                    row![title.color(p.strong).line_height(iced::widget::text::LineHeight::Absolute(15.0.into()))]
                        .push(muted.then(|| tinted(&BELL_OFF, 12.0, p.strong)))
                        .spacing(3.2)
                        .align_y(Center);
                let unread = (c.unread_count > 0).then(|| count_badge(c.unread_count, p.accent, 14.0));
                let face =
                    stack![self.chat_avatar(c, 40.0), container(unread).width(44).align_right(44)].width(44).height(40);
                let item = column![face, label]
                    .spacing(4)
                    .align_x(Center)
                    .width(cell)
                    .padding(Padding::new(10.0).top(8.0).bottom(6.0));
                r.push(
                    button(item)
                        .padding(0)
                        .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.text; 3], 8.0))
                        .on_press(UiMessage::SelectChat(c.id.clone())),
                )
            }))
        });
        let counts = st.settings.on("showUnreadCount");
        let text_w = w - 8.0 - 16.0 - 36.0 - 4.0 - if counts { 0.0 } else { 12.0 };
        let rows = rest.iter().fold(Column::new(), |col, c| {
            let on = selected == Some(c.id.as_str());
            let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
            let muted = c.is_muted == Some(true);
            let m = st.chat_msg_indices.get(&c.id).and_then(|v| v.last()).map(|&i| &st.messages[i]);
            let reaction = c
                .preview
                .as_ref()
                .filter(|p| p.message_type.as_deref() == Some("REACTION") && m.is_none_or(|m| m.sort_key < p.sort_key));
            let m = reaction.or(m);
            let ts = c.last_activity.as_deref().map(|t| list_ts(t, h24)).unwrap_or_default();
            let ts_w = crate::style::text_width(&ts, 10.0, Weight::Normal) + 8.0;
            let title = c.network_name.as_ref().map_or_else(|| c.title.clone(), |n| format!("{} • {n}", c.title));
            let weight = if counts && !unread { Weight::Medium } else { Weight::Semibold };
            let bell = if muted { 16.0 } else { 0.0 };
            let top = row![ell(&title, text_w - ts_w - bell, 13.0, weight).color(p.strong)]
                .push(muted.then(|| container(tinted(&BELL_OFF, 12.0, p.strong)).padding([0, 2])))
                .push(Space::new().width(Fill))
                .push(txt(ts, 10.0).color(p.sub))
                .height(16)
                .align_y(Center);
            let mine = m.is_some_and(|m| m.is_sender == Some(true));
            let image = m.and_then(|m| m.attachments.iter().find(|a| a.attachment_type == "img")).map(|a| {
                if a.is_gif == Some(true) {
                    "GIF"
                } else {
                    "Image"
                }
            });
            let body = m.and_then(|m| match m.links.len() {
                _ if m.html.is_some() => m.html.as_deref().map(beeper_core::codec::html_text),
                _ if m.text.as_ref().is_some_and(|t| !t.is_empty()) => m.text.clone(),
                _ if !m.attachments.is_empty() => None,
                0 => None,
                1 => Some("🔗 [Link]".to_string()),
                n => Some(format!("🔗 [{n} links]")),
            });
            let body = body.filter(|_| previews).map(|t| {
                let first = m
                    .and_then(|m| m.sender_name.clone())
                    .map(|s| s.split(' ').next().expect("sender name").to_string());
                match (c.chat_type == "group" || m.is_some_and(|m| network_bot(c, &m.sender_id)), first) {
                    (true, Some(s)) if !mine && reaction.is_none() => format!("{s}: {t}"),
                    _ => t,
                }
            });
            let dot_bg = if muted { Color { a: 0.3, ..p.strong } } else { p.accent };
            let count = (counts && unread).then(|| count_badge(c.unread_count, dot_bg, 12.0));
            let dot = (!counts).then(|| {
                let dot = container(Space::new()).width(6).height(6).style(fill_box(dot_bg, 3.0));
                container(unread.then_some(dot)).width(12)
            });
            let sender = m.filter(|_| body.is_some()).and_then(|m| preview_sender(c, m));
            let reserve = if count.is_some() { 26.0 } else { 0.0 }
                + if mine { 16.0 } else { 0.0 }
                + if sender.is_some() { 14.0 } else { 0.0 };
            let preview = row![]
                .push(mine.then(|| tinted(&SENT, 12.0, p.sub)))
                .push(image.map(|label| {
                    container(
                        row![ic(icon::IMAGE, 11.0).color(p.text), txt(label, 11.0).color(p.text)]
                            .spacing(3)
                            .align_y(Center),
                    )
                    .padding([1, 5])
                    .style(fill_box(p.link_card, 4.0))
                }))
                .push(body.map(|b| {
                    let face = sender
                        .map(|u| self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 12.0));
                    row![]
                        .push(face)
                        .push(ell(&b, text_w - reserve, 13.0, Weight::Normal).color(if unread {
                            p.text
                        } else {
                            p.sub
                        }))
                        .spacing(2)
                        .align_y(Center)
                }))
                .push(Space::new().width(Fill))
                .push(count)
                .spacing(4)
                .align_y(Center);
            let item = row![]
                .push(dot)
                .push(
                    container(badged(self.chat_avatar(c, 28.0), &c.account_id, 28.0, 12.0))
                        .padding(Padding::ZERO.right(8.5)),
                )
                .push(column![top, preview.height(16)].width(Fill))
                .align_y(Center)
                .padding(Padding::new(8.0).top(10.0).bottom(9.0));
            col.push(
                button(item)
                    .padding(0)
                    .width(Fill)
                    .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.text; 3], 8.0))
                    .on_press(UiMessage::SelectChat(c.id.clone())),
            )
            .push(divider(p, 52.0))
        });
        let list: El<'_> = if chats.is_empty() {
            let (head, hint) = self.active_tab.empty();
            let header = column![semi(head, 16.0).color(p.strong).align_x(Center).line_height(1.25)]
                .push((!hint.is_empty()).then(|| txt(hint, 13.0).color(p.strong).align_x(Center)))
                .align_x(Center);
            let fallback = self.active_tab.fallback().map(|views| {
                let words = "If you're looking for something, check the".split(' ').map(|w| txt(w, 13.0).into());
                let pill = |t: InboxTab| -> El<'_> {
                    button(semi(t.label(), 12.0))
                        .padding([6, 8])
                        .style(btn_style([p.selected; 3], [p.strong; 3], 999.0))
                        .on_press(UiMessage::SelectTab(t))
                        .into()
                };
                let line = Row::with_children(words.chain([pill(views[0]), txt("or", 13.0).into(), pill(views[1])]))
                    .spacing(3.5)
                    .align_y(Center)
                    .wrap()
                    .vertical_spacing(5.5)
                    .align_x(Center);
                container(line).center_x(Fill).padding([24, 0])
            });
            column![Space::new().height(Fill), header, Space::new().height(Fill)]
                .push(fallback)
                .padding([0, 22])
                .width(Fill)
                .into()
        } else {
            scrollable(column![pins, rows].padding(Padding::ZERO.right(4.0)))
                .height(Fill)
                .style(scroll_style(p.divider, p.weak))
                .into()
        };
        container(column![header].push(search).push(list).padding(Padding::ZERO.left(8.0))).width(w).height(Fill).into()
    }

    fn view_conversation(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let cid = self.selected_chat_id.as_deref().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
        let hide = st.settings.on("hideDeleted");
        let msgs: Vec<&Message> = st.chat_msg_indices.get(cid).map_or_else(Vec::new, |v| {
            v.iter().map(|&i| &st.messages[i]).filter(|m| !(hide && m.is_deleted == Some(true))).collect()
        });
        let mut col = Column::new().padding(Padding::new(16.0).top(96.0).bottom(8.0));
        let mut prev: Option<&Message> = None;
        let last_live = msgs.iter().rposition(|m| m.is_deleted != Some(true));
        for (i, m) in msgs.iter().enumerate() {
            let day_changed = prev.is_none_or(|q| q.timestamp[..10] != m.timestamp[..10]);
            if day_changed {
                let label = semi(day_label(&m.timestamp), 11.0).line_height(Pixels(14.0)).color(p.date_text);
                let pill = container(label).padding([5, 9]).style(bordered_box(p.card, p.outline, 1.0, 12.0));
                col = col.push(container(pill).center_x(Fill).padding([4, 0]));
            }
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id);
            let last =
                msgs.get(i + 1).is_none_or(|n| n.sender_id != m.sender_id || n.timestamp[..10] != m.timestamp[..10]);
            let sent = Some(i) == last_live && m.is_sender == Some(true);
            col = col.push(self.view_message(&st, chat, m, first, last, sent));
            col = col.push(msgs.get(i + 1).map(|n| Space::new().height(gap_below(m, n))));
            prev = Some(m);
        }
        let list = scrollable(col).anchor_bottom().height(Fill).style(scroll_style(p.divider, p.weak));
        let name_row =
            row![semi(chat.title.clone(), 12.0).color(p.strong), ic(icon::CHEVRON_RIGHT, 14.0).color(p.tertiary)]
                .spacing(4)
                .align_y(Center);
        let pill = button(name_row)
            .padding(Padding::new(6.0).left(10.0))
            .style(btn_style([p.elevated, p.selected, p.selected], [p.text; 3], 14.0))
            .on_press(UiMessage::ToggleRightPanel);
        let ring = container(self.chat_avatar(chat, 40.0)).padding(1).style(fill_box(p.card, 21.0));
        let header = stack![
            container(pill).center_x(Fill).padding(Padding::ZERO.top(44.0)),
            container(ring).center_x(Fill).padding(Padding::ZERO.top(8.0))
        ];
        let (rim, hover, card) = (Color { a: 0.1, ..p.strong }, p.hover, p.card);
        let feedback = button(container(txt("Feedback?", 11.0)).center(Fill))
            .width(74)
            .height(24)
            .padding(0)
            .style(move |_, s| button::Style {
                background: Some(if matches!(s, button::Status::Hovered) { hover } else { card }.into()),
                text_color: Color { a: 0.9, ..rim },
                border: iced::Border { color: rim, width: 1.0, radius: 4.0.into() },
                ..Default::default()
            })
            .on_press(UiMessage::OpenQuickLook("Feedback".into(), "Tell us what to improve in beeper-rs.".into()));
        let fade = Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, p.card)
            .add_stop(0.75, p.card.scale_alpha(0.85))
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let fade = container(Space::new()).width(Fill).height(84).style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fade))),
            ..Default::default()
        });
        let overlay =
            stack![fade, header, container(feedback).align_right(Fill).padding(Padding::ZERO.top(6.0).right(5.0))];
        let verify = (st.matrix.is_some() && !st.beeper_state.verified).then(|| {
            let note = txt("Verify this device to read your encrypted messages.", 12.0).color(p.text);
            let go = pill_btn(p, "Verify", UiMessage::OpenSettings(SettingsTab::Devices));
            let bar = row![note, Space::new().width(Fill), go].align_y(Center);
            container(bar).padding([8, 14]).style(bordered_box(p.elevated, p.outline, 1.0, 10.0))
        });
        let verify = verify.map(|v| container(v).padding([0, 12]));
        let pane =
            column![stack![list, overlay].width(Fill).height(Fill)].push(verify).push(self.view_composer(&st, chat));
        container(pane).width(Fill).height(Fill).clip(true).style(fill_box(p.card, 10.0)).into()
    }

    fn view_message<'a>(
        &self,
        st: &BeeperStore,
        chat: &Chat,
        m: &Message,
        first: bool,
        last: bool,
        sent: bool,
    ) -> El<'a> {
        let p = &self.pal;
        let mine = m.is_sender == Some(true);
        if m.is_deleted == Some(true) {
            let (fg, italic) =
                (mix(p.card, p.text, 0.8), iced::Font { style: iced::font::Style::Italic, ..fw(Weight::Normal) });
            let icon = svg(TRASH.clone())
                .width(13.0 * 448.0 / 512.0)
                .height(13)
                .style(move |_, _| svg::Style { color: Some(fg) });
            let body =
                row![icon, txt("This message has been deleted", 13.0).line_height(Pixels(16.0)).font(italic).color(fg)];
            let pill = container(body.spacing(5).align_y(iced::Bottom))
                .padding(Padding::new(13.0).top(9.0).bottom(7.0))
                .style(bordered_box(CLEAR, mix(p.card, p.received, 0.8), 1.0, 17.0));
            return container(pill).align_x(if mine { Right } else { iced::Left }).width(Fill).into();
        }
        let group = chat.chat_type == "group";
        let (bg, fg) = if mine { (p.sent, Color::WHITE) } else { (p.received, p.strong) };
        let text = m.text.clone().unwrap_or_default();
        let preview = st
            .settings
            .on("loadEmbeds")
            .then(|| {
                let link = m.links.first().map(|l| (l.url.as_str(), None, l.title.as_str(), l.summary.as_deref()));
                link.or_else(|| {
                    text.split_whitespace().find_map(|w| {
                        let lp = st.link_previews.get(w)?;
                        Some((w, Some(lp.site.as_str()), lp.title.as_str(), Some(lp.summary.as_str())))
                    })
                })
            })
            .flatten();
        let mut parts = Column::new().spacing(2).align_x(if mine { Right } else { iced::Left });
        if (group || network_bot(chat, &m.sender_id)) && !mine && first {
            parts = parts.push(
                container(
                    semi(m.sender_name.clone().expect("sender name"), 11.0)
                        .color(hex(*pick(m.sender_name.as_deref().expect("sender name"), &p.names))),
                )
                .padding(Padding::ZERO.left(11.0)),
            );
        }
        let quote = m.linked_message_id.as_ref().and_then(|r| st.msg_idx_by_id.get(r)).map(|&i| {
            let q = &st.messages[i];
            let q_mine = q.is_sender == Some(true);
            let (edge, fill) = if q_mine { (p.sent, mix(p.card, p.sent, 0.3)) } else { (p.received, p.card) };
            let body = ell(q.text.as_deref().unwrap_or(""), 300.0, 12.0, Weight::Medium).color(p.text);
            let pill = container(body).padding(8).style(bordered_box(fill, edge, 2.0, 17.0));
            let name = (group && !q_mine).then(|| {
                container(semi(q.sender_name.clone().expect("sender name"), 12.0).color(p.strong)).padding([0, 11])
            });
            let col = column![].push(name).push(pill).spacing(4);
            (col.align_x(if q_mine { Right } else { iced::Left }), q_mine)
        });
        if let Some((url, site, title, summary)) = preview {
            let host = url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(url).to_string();
            let body = column![]
                .push(site.map(|s| txt(s.to_string(), 11.0).color(p.sub)))
                .push(clamp(title, 338.0, 3, 13.0, Weight::Semibold).color(p.strong))
                .push(summary.map(|s| clamp(s, 338.0, 3, 13.0, Weight::Normal).color(p.sub)))
                .push(
                    row![ic(icon::LINK, 11.0).color(p.weak), txt(host, 11.0).color(p.weak)].spacing(4).align_y(Center),
                )
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
                                .height(4.0 + name_hash(&format!("{name}{k}")).rem_euclid(14) as f32)
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
                (false, "img") => button({
                    let (w, h) = a.size.as_ref().map_or((240.0, 160.0), |s| {
                        let (w, h) = (s.width.unwrap_or(240) as f32, s.height.unwrap_or(160) as f32);
                        let k = (360.0 / w).min(400.0 / h).min(1.0);
                        (w * k, h * k)
                    });
                    match a.src_url.as_ref().and_then(|u| self.media.get(u)?.as_ref()) {
                        Some(frames) => El::from(
                            image(current_frame(frames, self.started.elapsed().as_millis()))
                                .width(w)
                                .height(h)
                                .border_radius(17.0),
                        ),
                        None => container(ic(icon::IMAGE, 28.0).color(p.sub))
                            .center_x(w)
                            .center_y(h)
                            .style(fill_box(p.link_card, 17.0))
                            .into(),
                    }
                })
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
        if jumbo(&text) {
            parts = parts.push(container(txt(text.clone(), 48.0)).padding([0, 12]));
        } else if !text.is_empty() && m.poll.is_none() && m.message_type.as_deref() != Some("VOICE") {
            let ts = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
            let room = crate::style::text_width(&ts, 11.0, Weight::Normal) + 8.0;
            let time = txt(ts, 11.0).color(if mine { p.sent_meta } else { p.sub });
            let me = |id: &str| id == st.beeper_state.user_id;
            let arrow =
                || span(icon::ARROW_UP_RIGHT.to_string()).font(iced::Font::with_name("icons")).size(11.0).color(fg);
            let rich = |r: beeper_core::codec::Run| {
                let font = match r.code {
                    true => iced::Font::MONOSPACE,
                    false => iced::Font {
                        style: if r.italic { iced::font::Style::Italic } else { iced::font::Style::Normal },
                        ..crate::style::fw(if r.bold { Weight::Bold } else { Weight::Normal })
                    },
                };
                let mention = r.link.as_deref().and_then(|l| l.strip_prefix("https://matrix.to/#/"));
                let mention = mention.and_then(beeper_core::codec::url_decode).filter(|id| id.starts_with('@'));
                let s = span(r.text).size(13.0).color(fg).font(font).strikethrough(r.strike);
                let bold = crate::style::fw(Weight::Bold);
                let s = match mention {
                    Some(id) if me(&id) => {
                        let pill = if mine { Color { a: 0.3, ..fg } } else { p.accent };
                        s.font(bold).color(Color::WHITE).background(pill).border(border::rounded(4)).padding(2)
                    }
                    Some(_) => s.font(bold).color(if mine { fg } else { p.accent }),
                    None => s.underline(r.link.is_some()).link_maybe(r.link.clone()),
                };
                let arrow = s.link.is_some().then(arrow);
                [Some(s), arrow].into_iter().flatten()
            };
            let plain = |w: &str| {
                let url = w.trim_end().starts_with("http").then(|| w.trim_end().to_string());
                let word = span(w.trim_end().to_string()).size(13.0).color(fg).underline(url.is_some());
                let arrow = url.is_some().then(arrow);
                [Some(word.link_maybe(url)), arrow, w.ends_with(' ').then(|| span(" ").size(13.0))]
                    .into_iter()
                    .flatten()
            };
            let blocks: Vec<(Option<String>, Vec<_>)> = match m.html.as_deref() {
                Some(html) => beeper_core::codec::html_blocks(html)
                    .into_iter()
                    .map(|b| (b.marker, b.runs.into_iter().flat_map(rich).collect()))
                    .collect(),
                None => vec![(None, text.split_inclusive(' ').flat_map(plain).collect())],
            };
            let (mut prev_item, last) = (None, blocks.len() - 1);
            let body = Column::with_children(blocks.into_iter().enumerate().map(|(i, (marker, mut spans))| {
                // iced does not draw an empty last line.
                if spans.last().is_some_and(|s| s.text.ends_with('\n')) {
                    spans.push(span("\u{200b}").size(13.0));
                }
                if i == last {
                    let nbsp = crate::style::text_width("\u{a0}", 13.0, Weight::Normal);
                    spans.push(span("\u{a0}".repeat((room / nbsp).ceil() as usize)).size(13.0));
                }
                let line = rich_text(spans)
                    .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .on_link_click(UiMessage::OpenUrl);
                let gap = match (prev_item.replace(marker.is_some()), marker.is_some()) {
                    (None, _) => 0.0,
                    (Some(true), true) => 6.0,
                    _ => 9.0,
                };
                let indent = |m: &String| if m == "•" { 15.6 } else { 23.4 };
                let marker = marker.map(|m| {
                    let w = indent(&m);
                    container(txt(m, 13.0).color(fg)).width(w).align_right(w).padding(Padding::ZERO.right(5.0))
                });
                container(row![].push(marker).push(line)).padding(Padding::ZERO.top(gap)).into()
            }));
            parts = parts.push(
                container(stack![body, container(time).align_right(Fill).align_bottom(Fill)])
                    .padding([8, 11])
                    .max_width(0.45 * self.main_width)
                    .style(fill_box(bg, 17.0)),
            );
        }
        let replies = st.chat_msg_indices[&chat.id]
            .iter()
            .filter(|&&i| st.messages[i].linked_message_id.as_ref() == Some(&m.id))
            .count();
        let replies = match replies {
            0 => None,
            1 => Some("1 Reply".to_string()),
            n => Some(format!("{n} Replies")),
        };
        let footer: Vec<String> = [sent.then_some("Sent"), (!m.edit_history.is_empty()).then_some("Edited")]
            .into_iter()
            .flatten()
            .map(str::to_string)
            .chain(replies)
            .collect();
        let footer = (!footer.is_empty()).then(|| {
            let line = footer.into_iter().enumerate().fold(Row::new(), |r, (i, t)| {
                r.push((i > 0).then(|| container(txt("·", 12.0).color(p.muted.scale_alpha(0.8))).padding([0, 4])))
                    .push(txt(t, 12.0).line_height(Pixels(15.0)).color(p.muted))
            });
            container(line).padding([0, 4])
        });
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
                let own = m.reactions.iter().any(|x| x.reaction_key == k && st.is_me(&x.participant_id));
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
        let parts = column![parts].push(footer).align_x(if mine { Right } else { iced::Left });
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
                let url =
                    chat.participants.items.iter().find(|u| u.id == m.sender_id).and_then(|u| u.img_url.as_deref());
                self.face(url, m.sender_name.as_deref().expect("sender name"), 28.0)
            } else {
                Space::new().width(28).into()
            }
        });
        let indent = (group && !mine).then(|| Space::new().width(34));
        let line: El<'a> = if mine {
            row![Space::new().width(Fill)].push(tools).push(parts).spacing(6).align_y(Center).into()
        } else {
            row![].push(side).push(parts).push(tools).push(Space::new().width(Fill)).spacing(6).align_y(Bottom).into()
        };
        let line: El<'a> = match quote {
            None => line,
            Some((q, q_mine)) => {
                let curve = svg(REPLY_CURVE[usize::from(q_mine)].clone()).width(25).height(19);
                let bar = container(Space::new()).width(3).height(Fill).style(fill_box(p.reply_line, 1.5));
                let color = p.reply_line;
                let link = column![bar, curve.style(move |_, _| svg::Style { color: Some(color) })]
                    .push(Space::new().height(Fill))
                    .align_x(if q_mine { Right } else { iced::Left })
                    .padding(Padding::ZERO.left(if q_mine { 0.0 } else { 14.0 }).right(if q_mine {
                        14.0
                    } else {
                        0.0
                    }));
                let line = match (q_mine == mine, q_mine) {
                    (true, _) => line,
                    (false, false) => stack![line, row![].push(indent).push(link)].into(),
                    (false, true) => stack![line, row![Space::new().width(Fill), link]].into(),
                };
                let head: El<'a> = if q_mine {
                    row![Space::new().width(Fill), q].into()
                } else {
                    row![].push((group && !q_mine).then(|| Space::new().width(34))).push(q).into()
                };
                column![head, line].into()
            }
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
                    self.face(u.img_url.as_deref(), &name, 28.0),
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
                self.chat_avatar(chat, 64.0),
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
                    self.chat_avatar(c, 24.0),
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
            .line_height(Pixels(16.0))
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.text, p.accent, 0.0));
        let search = container(
            row![ic(icon::SEARCH, 16.0).color(p.text), search_input].spacing(8).align_y(Center).padding([6, 8]),
        )
        .style(fill_box(p.search, 38.0));
        let free = container(semi("Free", 10.0).color(p.strong)).padding([4, 6]).style(fill_box(p.chip, 25.0));
        let profile = row![
            avatar(&name, 32.0, p),
            column![bold(name.clone(), 14.0).color(p.strong),]
                .push(me.email.clone().map(|e| txt(e, 11.0).color(p.text)))
                .width(Fill),
            free
        ]
        .spacing(8)
        .align_y(Center)
        .height(44)
        .padding([0, 8]);
        let nav = SettingsTab::ALL.iter().fold(Column::new(), |c, &tab| {
            let (label, glyph, rgb) = tab.meta();
            let on = tab == self.active_settings_tab && self.settings_query.is_empty();
            let glyph: El = match glyph {
                TabIcon::Glyph(g) => ic(g, 13.0).color(Color::WHITE).into(),
                TabIcon::Svg(doc) => svg(svg::Handle::from_memory(doc)).width(Shrink).height(Shrink).into(),
            };
            let tile = container(glyph)
                .center(20)
                .style(fill_box(Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8), 6.0));
            let count = (tab == SettingsTab::Accounts).then(|| {
                container(txt(st.accounts.len().to_string(), 11.0).color(p.strong))
                    .center(18)
                    .style(fill_box(p.chip, 9.0))
            });
            let line = row![tile, txt(label, 14.0).color(p.strong), Space::new().width(Fill)]
                .push(count)
                .spacing(8)
                .align_y(Center)
                .height(if on { 44 } else { 43 })
                .padding([0, 8]);
            let next_on = SettingsTab::ALL.get(tab as usize + 1).is_some_and(|n| *n == self.active_settings_tab);
            c.push(
                button(line)
                    .padding(0)
                    .width(Fill)
                    .style(btn_style([if on { p.nav_sel } else { CLEAR }, p.hover, p.nav_sel], [p.text; 3], 8.0))
                    .on_press(UiMessage::SelectSettingsTab(tab)),
            )
            .push((!on).then(|| {
                let rule = if next_on { CLEAR } else { p.divider };
                container(container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0)))
                    .padding(Padding::ZERO.left(36.0))
            }))
        });
        let left = column![
            container(semi("Settings", 14.0).color(p.strong)).center_y(35).padding([0, 8]),
            search,
            scrollable(column![profile, nav]).height(Fill).style(scroll_style(p.divider, p.weak))
        ]
        .spacing(8)
        .width(270)
        .padding(Padding::new(8.0).right(15.0));
        let content = if self.settings_query.trim().is_empty() {
            self.settings_content(&st)
        } else {
            self.settings_search_results()
        };
        let right = container(
            scrollable(container(content).max_width(600).padding(13).center_x(Fill))
                .height(Fill)
                .style(scroll_style(p.divider, p.weak)),
        )
        .width(Fill)
        .height(Fill)
        .style(fill_box(p.prefs_pane, 10.0));
        let base = container(row![left, container(right).padding(8)]).width(Fill).height(Fill);
        stack![base.style(fill_box(p.frame, 0.0))].push(self.confirm_logout.then(|| self.view_logout_confirm())).into()
    }

    fn sas_row<'a>(&self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let v = st.setup_status.session.verification.as_ref();
        let (desc, control): (Option<&str>, El<'a>) =
            match (v.map(|v| v.state.as_str()), v.and_then(|v| v.sas.as_ref())) {
                (Some("requested"), _) => (Some("Accept the request on your other device."), Space::new().into()),
                (Some("sas_started"), Some(sas)) => {
                    let emojis = sas.emojis.iter().map(|e| {
                        column![txt(e.emoji.clone(), 24.0), txt(e.description.clone(), 11.0).color(p.sub)]
                            .spacing(4)
                            .align_x(Center)
                            .width(Fill)
                            .into()
                    });
                    let buttons = row![
                        pill_btn(p, "They don't match", UiMessage::SasMatch(false)),
                        pill_btn(p, "They match", UiMessage::SasMatch(true))
                    ]
                    .spacing(8);
                    let title = txt("Compare the emojis on both devices", 13.0).color(p.text);
                    let buttons = row![Space::new().width(Fill), buttons];
                    return column![title, Row::with_children(emojis), buttons].spacing(12).padding([13, 14]).into();
                }
                _ => (None, pill_btn(p, "Verify", UiMessage::VerifySasNow)),
            };
        pref_row(p, "Verify with another device", desc, None, control)
    }

    fn device_list<'a>(&self, title: &'a str, list: &[&DeviceInfo]) -> El<'a> {
        let p = &self.pal;
        let rows = list.iter().enumerate().flat_map(|(i, d)| {
            let n = d.name.to_lowercase();
            let glyph = if n.contains("ipad") {
                icon::TABLET
            } else if ["ios", "android", "iphone"].iter().any(|k| n.contains(k)) {
                icon::SMARTPHONE
            } else {
                icon::MONITOR
            };
            let tile = container(ic(glyph, 14.0).color(Color::WHITE)).center(28).style(fill_box(p.accent, 4.0));
            let text = column![semi(d.name.clone(), 12.0).color(p.strong)]
                .push(d.last_seen_ms.map(|t| txt(format!("Last seen: {}", ago(t)), 11.0).color(p.sub)))
                .width(Fill);
            let logout = button(semi("Logout", 12.0).color(p.strong))
                .padding([4, 8])
                .style(btn_style([p.hover, p.selected, p.selected], [p.strong; 3], 20.0))
                .on_press(UiMessage::AskLogout(true));
            let line: El<'a> =
                row![tile, text].push(d.current.then_some(logout)).spacing(12).align_y(Center).padding([8, 12]).into();
            [(i > 0).then(|| divider(p, 0.0)), Some(line)].into_iter().flatten()
        });
        column![
            container(txt(title, 12.0).color(p.sub)).padding([8, 0]),
            container(Column::with_children(rows)).style(bordered_box(CLEAR, p.prefs_border, 1.0, 10.0))
        ]
        .into()
    }

    fn view_logout_confirm(&self) -> El<'_> {
        let p = &self.pal;
        let name = self.devices.iter().flatten().flatten().find(|d| d.current).map(|d| d.name.clone());
        let card = column![
            bold(format!("Logout of {}?", name.as_deref().unwrap_or("this device")), 14.0).color(p.strong),
            txt("This will disconnect all chat accounts on this device.", 13.0).color(p.text),
            row![
                Space::new().width(Fill),
                pill_btn(p, "Cancel", UiMessage::AskLogout(false)),
                button(med("Logout", 13.0).color(Color::WHITE))
                    .padding([5, 12])
                    .style(btn_style([p.accent; 3], [Color::WHITE; 3], 6.0))
                    .on_press(UiMessage::Logout)
            ]
            .spacing(8)
        ]
        .spacing(10)
        .padding(16)
        .width(340);
        mouse_area(
            container(
                mouse_area(container(card).style(shadow_box(p.elevated, p.menu_border, 12.0, 24.0)))
                    .on_press(UiMessage::Noop),
            )
            .center(Fill)
            .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.45), 0.0)),
        )
        .on_press(UiMessage::AskLogout(false))
        .into()
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
                let account = |logo: &str, owner: &str, name: String, cloud: bool, sub: Option<String>| -> El<'_> {
                    let title = row![avatar(owner, 14.0, p), semi(name, 14.0).color(p.strong)]
                        .push(cloud.then(|| ic(icon::CLOUD, 13.0).color(p.sub)))
                        .spacing(4)
                        .align_y(Center);
                    row![
                        container(svg(brand_svg(brand_key(logo))).width(28).height(28)).center(46),
                        column![title].push(sub.map(|s| txt(s, 13.0).color(p.sub))).width(Fill),
                        ic(icon::CHEVRON_RIGHT, 16.0).color(p.strong)
                    ]
                    .spacing(8)
                    .align_y(Center)
                    .padding(Padding::new(4.0).left(8.0).right(16.0))
                    .into()
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
                    med("Accounts", 12.0).color(p.weak),
                    Space::new().width(Fill),
                    icon_btn(p, icon::CIRCLE_PLUS, 16.0, 22.0, p.accent, UiMessage::ToggleAddAccount)
                ]
                .height(26)
                .align_y(Center)
                .padding([0, 4]);
                let remaining = login::FREE_ACCOUNTS.saturating_sub(st.bridge_accounts.len());
                let foot = column![
                    button(med("Add an Account", 13.0).color(p.accent))
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                        .on_press(UiMessage::ToggleAddAccount),
                    txt(format!("{remaining} Free Accounts Remaining"), 12.0).color(p.sub)
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
                .chain(
                    self.adding_account
                        .then(|| group(p, None, vec![login::network_grid(&st.bridge_accounts, "", p.strong)])),
                )
                .collect()
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
                let encryption = group(
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
                        self.sas_row(st),
                    ],
                );
                let mut out = match &self.devices {
                    None => Vec::new(),
                    Some(Err(e)) => vec![txt(e.clone(), 13.0).color(p.danger).into()],
                    Some(Ok(all)) => [
                        ("Current Device", all.iter().filter(|d| d.current).collect::<Vec<_>>()),
                        ("Verified Devices", all.iter().filter(|d| !d.current && d.verified).collect()),
                        ("Unverified Devices", all.iter().filter(|d| !d.current && !d.verified).collect()),
                    ]
                    .into_iter()
                    .filter(|(_, list)| !list.is_empty())
                    .map(|(title, list)| self.device_list(title, &list))
                    .collect(),
                };
                out.push(encryption);
                out
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
            SettingsTab::About => {
                let title =
                    column![bold("Beeper", 19.5).color(p.text), txt(format!("v{APP_VERSION}"), 13.0).color(p.weak)]
                        .spacing(4);
                let head = row![svg(LOGO.clone()).width(68).height(68), title].spacing(6).align_y(Center);
                let links = [
                    ("View changelog", UiMessage::OpenUrl("https://www.beeper.com/changelog/desktop".into())),
                    ("Report a problem", UiMessage::OpenUrl("https://github.com/pythoncrazy/beeper-rs/issues".into())),
                    ("Privacy", UiMessage::OpenUrl("https://www.beeper.com/privacy".into())),
                    (
                        "Legal Notices",
                        UiMessage::OpenQuickLook(
                            "Legal Notices".into(),
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

    fn pref_items(&self, items: &'static [prefs::Item], s: &BeeperSettings) -> Vec<El<'_>> {
        let p = &self.pal;
        let mut heading = None;
        let mut lead = None;
        let mut out = Vec::new();
        for item in items {
            match *item {
                prefs::Item::Heading(h) => heading = Some(h),
                prefs::Item::Subtitle(t) => match heading.take() {
                    Some(h) => {
                        out.push(group(p, Some(h), vec![container(txt(t, 13.0).color(p.sub)).padding([13, 14]).into()]))
                    }
                    None => lead = Some(t),
                },
                prefs::Item::Group(rows) => {
                    let g = group(p, heading.take(), rows.iter().map(|r| self.pref_control(r, s)).collect());
                    out.push(match lead.take() {
                        Some(t) => {
                            let t = txt(t, 13.0).line_height(Pixels(18.0)).color(p.sub);
                            column![container(t).padding(Padding::new(13.0).top(7.0).bottom(0.0)), g].spacing(19).into()
                        }
                        None => g,
                    })
                }
                prefs::Item::Theme => out.push(group(p, heading.take(), vec![theme_cards(p, s.theme)])),
            }
        }
        assert!(lead.is_none(), "a subtitle without a heading comes before a group");
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
                assert!(r.desc.is_none() && r.badge.is_none(), "{} is a plain slider", r.key);
                let key = r.key;
                let bar = slider(0..=100u8, s.pref(key), move |x| UiMessage::SetPref(key, x))
                    .height(16)
                    .style(slider_style(p));
                let body =
                    column![txt(r.label, 13.0).line_height(Pixels(18.0)).color(p.text), container(bar).padding([0, 2])]
                        .spacing(13)
                        .padding(Padding::new(13.0).bottom(14.0));
                return column![
                    body,
                    container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0))
                ]
                .into();
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
                .padding(13)
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

fn jumbo(text: &str) -> bool {
    let modifier = |c: char| matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{20e3}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}');
    let emoji = |c: char| {
        matches!(c, '\u{1f000}'..='\u{1faff}' | '\u{2600}'..='\u{27bf}' | '\u{2300}'..='\u{23ff}' | '\u{2b00}'..='\u{2bff}'
            | '\u{2190}'..='\u{21ff}' | '\u{3030}' | '\u{303d}' | '\u{3297}' | '\u{3299}' | '\u{a9}' | '\u{ae}' | '\u{2122}')
    };
    let mut glyphs = 0;
    let mut joined = false;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        match (modifier(c), emoji(c)) {
            (true, _) => joined = c == '\u{200d}',
            (false, true) => {
                glyphs += usize::from(!joined);
                joined = false;
            }
            (false, false) => return false,
        }
    }
    (1..10).contains(&glyphs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_frames_play_with_browser_delays() {
        let mut bytes = Vec::new();
        let frames = [50, 0].map(|ms| {
            let px = ::image::RgbaImage::new(2, 2);
            ::image::Frame::from_parts(px, 0, 0, ::image::Delay::from_numer_denom_ms(ms, 1))
        });
        ::image::codecs::gif::GifEncoder::new(&mut bytes).encode_frames(frames).unwrap();
        let frames = decode_media(bytes).unwrap();
        assert_eq!(frames.iter().map(|f| f.1).collect::<Vec<_>>(), [50, 100]);
        assert_eq!(current_frame(&frames, 60), frames[1].0);
        assert_eq!(current_frame(&frames, 160), frames[0].0);
        assert_eq!(decode_media(b"\x89PNG".to_vec()).unwrap().len(), 1);
    }
}

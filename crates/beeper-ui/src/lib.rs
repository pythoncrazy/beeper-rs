mod cursor;
pub mod emoji;
mod format;
mod lift;
pub mod login;
mod notify;
mod popover;
mod sound;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
mod tray {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Event {
        Up,
        Show,
        Hide,
        Quit,
    }
    pub fn events() -> iced::futures::stream::Empty<Result<Event, String>> {
        unreachable!("only the Linux build shows a tray icon")
    }
    pub async fn update(_unread: u32, _visible: bool) -> Result<(), String> {
        Ok(())
    }
}
pub mod style;
mod voice;

use crate::login::{Login, LoginMsg};

use crate::style::{
    bar, bold, bordered_box, bordered_btn, brand_svg, btn_style, clamp, clamped, ell, ensure_fonts_loaded, fill_box,
    fw, glow_box, gutter_style, hex, ic, icon, initials, input_style, med, mix, pick, pick_menu_style, pick_style,
    scroll_style, semi, shadow_box, stable_bar, text_width, txt, Palette, ALL_FONTS, AVATAR_GRADIENTS, CLEAR, GUTTER,
    SOLID_BARS,
};
use beeper_core::cloud::{Cloud, KlipyGif};
use beeper_core::config::{
    account_network, civil_from_days, local_millis, millis_from_rfc3339, now_unix_millis, now_unix_secs,
    rfc3339_from_millis, APP_VERSION,
};
use beeper_core::import::has_beeper_data;
use beeper_core::matrix::{ChatFlag, DeviceInfo, Outgoing};
use beeper_core::models::*;
use beeper_core::platform::{current_original_beeper_data_dir, open_url_or_path_command, DisplayServerKind, OsKind};
use beeper_core::prefs;
use beeper_core::store::{BeeperStore, NetworkSpace, SharedStore, NOTE_CHAT_ID};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, image, mouse_area, operation, pick_list, pin, rich_text, row, scrollable, slider, span,
    stack, svg, text_editor, text_input, tooltip, Column, Row, Space, Stack,
};
use iced::{
    border, mouse, window, Background, Bottom, Center, Color, Element, Event, Fill, Gradient, Length, Padding, Pixels,
    Point, Radians, Right, Shadow, Shrink, Size, Subscription, Task, Theme, Vector,
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

/// Desktop's silent membership events, which the chat list skips.
fn silent(m: &Message) -> bool {
    m.message_type.as_deref() == Some("MEMBERSHIP")
}

/// Composer content with `text` and the cursor at the end.
fn composer_with(text: &str) -> text_editor::Content {
    format::content(text, text.len()..text.len())
}

fn filter_hit(i: usize, c: &Chat, st: &BeeperStore) -> bool {
    match i {
        0 => c.flags & CHAT_UNREAD != 0,
        1 => c.draft.is_some(),
        2 => {
            let v = st.chat_msg_indices.get(&c.id);
            v.and_then(|v| v.iter().rev().map(|&i| &st.messages[i]).find(|m| !silent(m))).is_none_or(|m| {
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
    ClickChat(String),
    ComposerChanged(String),
    ComposerEdit(text_editor::Action),
    /// Toggles a `format::LABELS` button on the composer selection.
    Format(usize),
    LinkDialog(Option<(String, String)>),
    SaveLink,
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
    ToggleAttachMenu,
    ToggleGifPicker,
    Emoji(emoji::EmojiMsg),
    Upserted(Box<Message>),
    ChatUpserted(Box<Chat>),
    Typing(String),
    NotifyTimer(String, u64),
    Notified(Result<(u32, String), String>),
    NotificationEvent(u32, bool),
    WindowFocused(bool),
    GifQueryChanged(String),
    GifsLoaded(Result<Vec<KlipyGif>, String>),
    PickGif(KlipyGif),
    GifDownloaded(KlipyGif, Result<(Vec<u8>, Frames), String>),
    PasteClipboard,
    SetReminder(String, Option<i64>),
    ChatSub(Option<u8>),
    CheckReminders,
    ToggleFlag(String, ChatFlag),
    ToggleRead(String),
    CopyChatLink(String),
    OpenChatMenu(String),
    Cursor(Point),
    OpenMessageMenu(String),
    /// Select the message, or unselect it when it is selected.
    ClickMessage(String),
    /// Do the "On double-clicking a message" action.
    DoubleClickMessage(String),
    /// Select the message above (`false`) or below (`true`) the selected one, as with the arrow keys.
    StepMessage(bool),
    /// Open the emoji picker at the cursor to react to a message.
    React(String),
    /// An action picked in the message menu, which closes it.
    MessageMenu(Box<UiMessage>),
    CopyText(String),
    /// Copy a link and say so, like Desktop's Media window.
    CopyLink(String),
    CopyImage(String, usize),
    ImageFetched(Result<Vec<u8>, String>),
    /// Open the forward dialog for a message.
    Forward(String),
    ForwardQuery(String),
    /// Add the first chat that matches the query.
    ForwardSubmit,
    ForwardPick(String),
    ForwardSend,
    /// Show an attachment of a message full screen, in the Media window with `true`.
    OpenCarousel(String, usize, bool),
    /// Show the previous (`false`) or next (`true`) image of the carousel.
    CarouselStep(bool),
    RotateCarousel,
    CloseCarousel,
    /// Open Desktop's Media window, or focus it.
    OpenMedia,
    /// Show the Media, Links or Docs tab of the Media window.
    MediaTab(usize),
    /// Load more items in the Media window.
    MediaMore,
    /// Show the Media window items that all, you or others sent.
    MediaSentBy(u8),
    /// Show the oldest Media window items first.
    MediaOldest(bool),
    /// Search the Media window by caption, or close the search with `None`.
    MediaSearch(Option<String>),
    MediaMenu(bool),
    MediaCursor(Point),
    /// Open the context menu of a Media window item, or close it with `None`.
    MediaItemMenu(Option<(String, usize)>),
    /// Close the Media window item menu and run its action.
    MediaItemAct(Box<UiMessage>),
    MediaSelect(bool),
    /// Select or unselect a Media window item, as (message, attachment or link index).
    MediaPick(String, usize),
    MediaPickAll,
    /// Download the selected Media window attachments.
    MediaSave,
    /// Ask to delete Media window items.
    MediaAskDelete(Vec<(String, usize)>),
    /// Delete the items that the Media window dialog asks about, or cancel with `false`.
    MediaDelete(bool),
    Modifiers(keyboard::Modifiers),
    /// Open a message in the main window.
    GoToMessage(String, String),
    WindowKey(window::Id, Key, keyboard::Modifiers),
    /// Save the attachments of a message, or only one of them, to the downloads folder.
    Download(String, Option<usize>),
    Downloaded(Result<String, String>),
    AskDeleteMessage(String),
    ArchiveStale,
    NicknameChanged(String),
    SaveNickname,
    ToggleFilterMenu,
    FilterButton,
    PickFilter(u8),
    ToggleCommandPalette,
    SearchRoom(String),
    ToggleCommandMode,
    ToggleSearchExpanded,
    CommandQueryChanged(String),
    ExecuteCommandChat(String),
    JumpTo(String, String),
    OpenSearchSelect(usize),
    SearchSelect(Option<(usize, iced::Rectangle)>),
    PickSearchFilter(SearchPick),
    NewChat,
    SelectNewChat(u32),
    DeleteNewChat(u32),
    AskDeleteChat(String),
    BeginSelecting(String),
    ToggleSelected(String),
    SelectionSet(ChatFlag, bool),
    SelectionRemind(Option<i64>),
    SelectionMenu(u8, f32),
    DeleteChat(String, bool),
    NewChatQueryChanged(String),
    ToggleNewChatExpanded,
    ToggleJoinMatrix,
    ContactsLoaded(Vec<User>),
    OpenSettings(SettingsTab),
    ToggleAddAccount,
    SelectSettingsTab(SettingsTab),
    SettingsQueryChanged(String),
    ToggleRightPanel,
    ToggleMembers(String),
    ToggleDnd,
    CycleTheme,
    SelectTheme(ThemeChoice),
    SystemTheme(iced::theme::Mode),
    SetPref(&'static str, u8),
    ToggleHotkey(&'static str),
    RevertHotkeys,
    HotkeyQuery(String),
    Zoom(i8),
    ToggleSetting(fn(&mut BeeperSettings)),
    CloudAccounts(Result<Vec<BridgeAccountInfo>, String>),
    AvatarLoaded(String, Result<image::Handle, String>),
    MediaLoaded(String, Result<Frames, String>),
    Voice(voice::Msg),
    RetryMedia(String),
    ResolveMedia(String, String, String),
    MediaResolved(String, Result<(), String>),
    PlayVideo(String, u32, u32),
    VideoDecoded(String, Result<Frames, String>),
    Pasted(Result<Option<Png>, String>),
    DiscardPaste,
    LinkPreviewLoaded(String, Result<Option<beeper_core::cloud::PagePreview>, String>),
    RemoveLinkPreview(String),
    PickEmoji(usize),
    AnimationTick,
    RecoveryInputChanged(String),
    VerifyRecoveryCode,
    Verified(Result<(), String>),
    Pushed(Result<(), String>),
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    Ask(Option<u8>),
    ReminderMenu,
    OrderReminders(u8),
    DismissAllReminders,
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
    ShowQuickReactions,
    HoverChat(String, bool),
    DeleteDraft(String),
    HoverExit(String),
    DragStart,
    DragMoved(Point),
    DragEnd,
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    CloseRequested(window::Id),
    Tray(Result<tray::Event, String>),
    TrayFailed(String),
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
fn network_name(account_id: &str) -> &'static str {
    account_network(account_id).display_name
}

fn brand_key(account_id: &str) -> &'static str {
    account_network(account_id).bridge_id
}

fn iso_millis(iso: &str) -> i64 {
    millis_from_rfc3339(iso).unwrap_or_else(|| panic!("timestamp {iso} is not RFC 3339"))
}

pub fn format_short_ts(iso: &str, twenty_four: bool) -> String {
    clock(iso_millis(iso), twenty_four)
}

fn clock(ms: i64, twenty_four: bool) -> String {
    let mins = local_millis(ms).div_euclid(60_000).rem_euclid(1440);
    let (h, m) = (mins / 60, format!("{:02}", mins % 60));
    if twenty_four {
        return format!("{h:02}:{m}");
    }
    format!("{}:{m} {}", (h + 11) % 12 + 1, ["AM", "PM"][usize::from(h >= 12)])
}

/// The en-US `Date.toLocaleString()` of a timestamp, like "10/16/2026, 10:40:56 PM".
fn locale_string(iso: &str) -> String {
    let ms = local_millis(iso_millis(iso));
    let (y, mo, d) = civil_from_days(ms.div_euclid(86_400_000));
    let s = ms.div_euclid(1000).rem_euclid(86_400);
    let (h, ampm) = (s / 3600, ["AM", "PM"][usize::from(s >= 43_200)]);
    format!("{mo}/{d}/{y}, {}:{:02}:{:02} {ampm}", (h + 11) % 12 + 1, s / 60 % 60, s % 60)
}

/// Desktop's Copy text of a message: its attachment label, its text and the links the text lacks.
/// With `stamp`, the time and the sender come first.
fn copy_text(m: &Message, stamp: bool) -> String {
    let text = m.text.as_deref().unwrap_or_default();
    let links = m.links.iter().map(|l| l.url.as_str()).filter(|u| !text.contains(u));
    let body = std::iter::once(text).chain(links).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    let head = stamp
        .then(|| format!("[{}] {}:", locale_string(&m.timestamp), m.sender_name.as_deref().unwrap_or("(Unknown)")));
    head.into_iter()
        .chain(notify::attachment_text(&m.attachments, false))
        .chain((!body.is_empty()).then_some(body))
        .collect::<Vec<_>>()
        .join(" ")
}

fn day_num(iso: &str) -> i64 {
    day_of(iso_millis(iso))
}

fn day_of(ms: i64) -> i64 {
    local_millis(ms).div_euclid(86_400_000)
}

fn today() -> i64 {
    day_of(now_unix_millis())
}

fn day_label(iso: &str) -> String {
    let (d, today) = (day_num(iso), today());
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
    let d = day_num(iso);
    let age = now_unix_millis() - millis_from_rfc3339(iso).expect("RFC 3339 timestamp");
    let (y, m, dd) = civil_from_days(d);
    let month = MONTHS[m as usize - 1];
    match today() - d {
        0 => format_short_ts(iso, h24),
        1 => "Yesterday".into(),
        _ if age < 7 * DAY => WEEKDAYS[(d + 4).rem_euclid(7) as usize].into(),
        _ if age < 365 * DAY => format!("{month} {dd}"),
        _ => format!("{} {dd}, {y}", &month[..3]),
    }
}

fn human_size(n: u64) -> String {
    let i = (n.max(1) as f64).log(1024.0).floor() as i32;
    let v = format!("{:.1}", n as f64 / 1024f64.powi(i));
    format!("{} {}", v.trim_end_matches(".0"), ["B", "KB", "MB", "GB", "TB"][i as usize])
}

/// `dir/name`, or `dir/stem (n).ext` with the first free `n`.
fn unused_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let names = std::iter::once(name.to_string()).chain((1..).map(|i| format!("{stem} ({i}){ext}")));
    names.map(|n| dir.join(n)).find(|p| !p.exists()).expect("a free file name")
}

fn ghost(p: &Palette, r: f32) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    btn_style([CLEAR, p.hover, p.selected], [p.text; 3], r)
}

/// A row of a dropdown or context menu. Without a message it is greyed out.
fn menu_item<'a>(
    p: &Palette,
    lead: Option<El<'a>>,
    label: String,
    hotkey: impl iced::widget::text::IntoFragment<'a>,
    msg: Option<UiMessage>,
) -> El<'a> {
    let line = row![]
        .push(lead)
        .push(txt(label, 13.0))
        .push(Space::new().width(Fill))
        .push(txt(hotkey, 13.0).color(Color { a: 0.5, ..p.strong }))
        .align_y(Center);
    let style = btn_style([CLEAR, p.accent, p.accent], [p.text, Color::WHITE, Color::WHITE], 4.0);
    container(button(line).padding([3.125, 8.0]).width(Fill).style(style).on_press_maybe(msg)).padding([2, 5]).into()
}

pub(crate) const ISSUES_URL: &str = "https://github.com/pythoncrazy/beeper-rs/issues";
const ORANGE: Color = Color::from_rgb8(245, 76, 3);
const CHECKED: Color = Color::from_rgb8(14, 77, 242);
const ASK_LOGOUT: u8 = 0;
const ASK_DISMISS_REMINDERS: u8 = 1;
const ASK_DELETE_CHAT: u8 = 2;
const ASK_DELETE_MESSAGE: u8 = 3;
/// Desktop's default "Quick reaction emoji".
/// Wide enough for "Copy Entire Message with Timestamp" on one line.
const MESSAGE_MENU_W: f32 = 270.0;
const REMINDER_ORDERS: [&str; 4] = [
    "Order by Least Time Remaining",
    "Order by Most Time Remaining",
    "Order by Oldest Message",
    "Order by Newest Message",
];
const MENU_ITEM: f32 = 26.0;
/// Desktop's `--chat-view-bubble-padding-x`, which also insets the message footer.
const BUBBLE_PAD_X: f32 = 11.0;
const SUB_MENU_W: [f32; 2] = [124.0, 257.0];
const REMIND_PRESETS: [&str; 5] = ["In an Hour", "In Two Hours", "This Evening", "Tomorrow", "Next Week"];

/// The UTC time of a Remind Me preset, read like Desktop's chrono-node parse with forward dates.
fn remind_preset(k: usize, now: i64) -> i64 {
    const HOUR: i64 = 3_600_000;
    let evening = now - local_millis(now).rem_euclid(24 * HOUR) + 20 * HOUR;
    match k {
        0 => now + HOUR,
        1 => now + 2 * HOUR,
        2 if evening > now => evening,
        2 => evening + 24 * HOUR,
        3 => now + 24 * HOUR,
        4 => now + 7 * 24 * HOUR,
        _ => unreachable!("preset {k}"),
    }
}

fn remind_label(at: i64, h24: bool) -> String {
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{}, {:0>8}", &wd[..3], clock(at, h24))
}

/// The day of a reminder time, like Desktop's `Today` or `Tomorrow`.
fn reminder_day(at: i64) -> String {
    let (d, t) = (day_of(at), today());
    match d - t {
        -1 => "Yesterday".into(),
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ if (d + 3).div_euclid(7) == (t + 3).div_euclid(7) => WEEKDAYS[(d + 4).rem_euclid(7) as usize].into(),
        _ => {
            let (y, m, dd) = civil_from_days(d);
            format!("{m:02}/{dd:02}/{y}")
        }
    }
}

fn has_flag(c: &Chat, flag: ChatFlag) -> bool {
    let on = match flag {
        ChatFlag::Archived => c.is_archived,
        ChatFlag::Pinned => c.is_pinned,
        ChatFlag::LowPriority => c.is_low_priority,
        ChatFlag::Muted => c.is_muted,
        ChatFlag::MarkedUnread => return c.unread_count > 0 || c.is_marked_unread == Some(true),
    };
    on == Some(true)
}

fn checkbox<'a>(p: &Palette, on: bool) -> El<'a> {
    let (bg, ring) = if on { (CHECKED, CLEAR) } else { (CLEAR, Color { a: 0.3, ..p.strong }) };
    let check = on.then(|| ic(icon::CHECK, 12.0).color(Color::WHITE));
    container(check).center(16).style(bordered_box(bg, ring, 1.5, 8.0)).into()
}

fn menu_rule<'a>(p: &Palette) -> El<'a> {
    let line = container(Space::new()).width(Fill).height(1).style(fill_box(Color { a: 0.1, ..p.strong }, 0.0));
    container(line).padding([5, 8]).into()
}

fn icon_btn<'a>(p: &Palette, c: char, size: f32, frame: f32, color: Color, msg: UiMessage) -> El<'a> {
    button(container(ic(c, size).color(color)).center(frame)).padding(0).style(ghost(p, 8.0)).on_press(msg).into()
}

/// Desktop's `remove-btn`: a 20px round button over the top right corner of a composer attachment.
fn removable<'a>(p: &Palette, item: impl Into<El<'a>>, msg: UiMessage) -> El<'a> {
    let close = container(ic(icon::CLOSE, 12.0).color(p.strong)).center(20);
    let close = button(close).padding(0).style(btn_style([p.elevated, p.hover, p.selected], [p.strong; 3], 10.0));
    let item = container(item).padding(Padding::ZERO.top(5.0).right(5.0));
    stack![item, row![Space::new().width(Fill), close.on_press(msg)]].width(Shrink).into()
}

fn count_badge<'a>(c: &Chat, bg: Color, fg: Color, h: f32) -> El<'a> {
    let n = if c.unread_count > 0 { c.unread_count.to_string() } else { String::new() };
    let empty = n.is_empty() && c.unread_mentions == 0;
    let at = (c.unread_mentions > 0).then(|| ic(icon::AT_SIGN, 10.0).color(fg));
    let body = row![].push(at).push(semi(n, 10.0).color(fg)).spacing(2).align_y(Center);
    let pill = container(body).padding([0, 4]).center_y(h).style(fill_box(bg, h / 2.0));
    if empty { pill.width(h) } else { pill }.into()
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

fn divider<'a>(color: Color, left: f32) -> El<'a> {
    container(container(Space::new()).width(Fill).height(1).style(fill_box(color, 0.0)))
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
pub type Frames = Vec<(image::Handle, u128)>;
type Png = (Vec<u8>, (u32, u32));
type PlayingVideo = (String, Option<Result<(Frames, std::time::Instant), String>>);

#[derive(Clone)]
pub struct PendingMedia {
    pub bytes: Vec<u8>,
    pub name: String,
    pub mime: String,
    pub is_gif: bool,
    pub size: (u32, u32),
    pub frames: Frames,
}

/// Desktop's composer `linkPreviews` entry: a `com.beeper.linkpreviews` entry and its image when loaded.
pub enum LinkState {
    Loading,
    Loaded(serde_json::Value, Option<(Vec<u8>, image::Handle)>),
    Discarded,
}

/// The links that Desktop previews in the composer.
fn text_urls(text: &str) -> Vec<&str> {
    let link = |w: &&str| (w.starts_with("https://") || w.starts_with("http://")) && !w.contains('@');
    text.split_whitespace().filter(link).collect()
}

/// JavaScript's `encodeURIComponent`, which Desktop uses for chat links.
fn encode_uri_component(s: &str) -> String {
    s.bytes().fold(String::new(), |mut out, b| {
        match b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            true => out.push(b as char),
            false => out.push_str(&format!("%{b:02X}")),
        }
        out
    })
}

async fn clipboard_image() -> Result<Option<Png>, String> {
    let read = || {
        let mut c = arboard::Clipboard::new().map_err(|e| format!("Could not read the clipboard: {e}"))?;
        let img = match c.get_image() {
            Err(arboard::Error::ContentNotAvailable) => return Ok(None),
            // iced's X11 clipboard answers every target, image/png too, with its text.
            Err(arboard::Error::Unknown { .. }) if c.get_text().is_ok() => return Ok(None),
            r => r.map_err(|e| format!("Could not read the clipboard: {e}"))?,
        };
        let size = (u32::try_from(img.width).unwrap(), u32::try_from(img.height).unwrap());
        let px = ::image::RgbaImage::from_raw(size.0, size.1, img.bytes.into_owned()).expect("arboard gives RGBA");
        let mut png = Vec::new();
        px.write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).expect("encode PNG");
        Ok(Some((png, size)))
    };
    tokio::task::spawn_blocking(read).await.expect("clipboard task")
}

fn run_ffmpeg(bytes: &[u8], args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("ffmpeg")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg("pipe:0")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    std::thread::scope(|s| {
        s.spawn(move || {
            let _ = stdin.write_all(bytes);
        });
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        match out.status.success() && !out.stdout.is_empty() {
            true => Ok(out.stdout),
            false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        }
    })
}

fn decode_video(bytes: Vec<u8>, w: u32, h: u32) -> Result<Frames, String> {
    let vf = format!("fps=15,scale={w}:{h}:flags=fast_bilinear");
    let raw = run_ffmpeg(&bytes, &["-t", "30", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])?;
    let step = (w * h * 4) as usize;
    Ok(raw.chunks_exact(step).map(|px| (image::Handle::from_rgba(w, h, px.to_vec()), 67)).collect())
}

fn decode_media(bytes: Vec<u8>) -> Result<Frames, String> {
    use ::image::AnimationDecoder;
    if bytes.get(4..8) == Some(b"ftyp") {
        let png = run_ffmpeg(&bytes, &["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])?;
        return Ok(vec![(image::Handle::from_bytes(png), 0)]);
    }
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

fn badged<'a>(inner: El<'a>, account_id: &str, size: f32, badge: f32, mono: Option<&Palette>) -> El<'a> {
    let icon: El<'a> = match mono {
        None => svg(brand_svg(brand_key(account_id))).width(badge).height(badge).into(),
        Some(p) => {
            let fg = p.strong;
            let glyph = svg(crate::style::brand_badge(brand_key(account_id)))
                .style(move |_, _| svg::Style { color: Some(fg) })
                .width(badge)
                .height(badge);
            container(glyph).style(fill_box(p.chip, badge / 2.0)).into()
        }
    };
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
    toggle_faded(p, on, msg, 1.0)
}

fn toggle_faded<'a>(p: &Palette, on: bool, msg: Option<UiMessage>, alpha: f32) -> El<'a> {
    let fill = if on { p.toggle_on } else { p.toggle_off }.scale_alpha(alpha);
    let knob = container(Space::new()).width(16).height(16).style(fill_box(fill, 8.0));
    let track =
        container(knob).width(36).height(24).padding(4).align_y(Center).align_x(if on { Right } else { iced::Left });
    button(track.style(bordered_box(CLEAR, p.toggle_off.scale_alpha(alpha), 2.0, 12.0)))
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
    let list = pick_list(options, Some(selected), f)
        .text_size(11)
        .text_line_height(Pixels(16.0))
        .font(fw(Weight::Semibold))
        .handle(pick_list::Handle::None)
        .padding([4, 8])
        .style(pick_style(p.chip, p.chip, p.strong, p.strong))
        .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border));
    let chevron = container(tinted(&SELECT_ARROW, 10.0, p.strong)).align_right(Fill).center_y(Fill).padding([0, 8]);
    stack![list, chevron].into()
}

/// The bold arrow of Chromium's native select, which Desktop's Settings use.
static SELECT_ARROW: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M1 3.25 5 7.25 9 3.25" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##[..])
});

fn pill_btn<'a>(p: &Palette, label: impl ToString, msg: UiMessage) -> El<'a> {
    let label = txt(label.to_string(), 13.0).line_height(1.0).wrapping(iced::widget::text::Wrapping::None);
    button(label).padding([4, 6]).style(pill_style(p)).on_press(msg).into()
}

fn pill_style(p: &Palette) -> impl Fn(&Theme, button::Status) -> button::Style + use<> {
    bordered_btn([p.button; 3], [p.strong; 3], p.button_border, 1.0, 6.0)
}

fn badge<'a>(p: &Palette, b: prefs::Badge) -> El<'a> {
    match b {
        prefs::Badge::Plus => {
            let label = row![semi("+", 12.0).line_height(1.0), semi("Plus", 10.0).line_height(Pixels(12.0))];
            container(label.spacing(2).align_y(Center).padding([4, 6])).style(fill_box(p.chip, 25.0)).into()
        }
        prefs::Badge::Labs => {
            let tag = txt("🧪 Labs", 12.0).color(p.strong);
            container(tag).padding([1, 4]).style(fill_box(p.strong.scale_alpha(0.1), 4.0)).into()
        }
    }
}

const THEME_WINDOW: &str = r##"<rect x="10" y="7" width="76" height="47" rx="4" fill="WIN"/><circle cx="15" cy="12" r="1.4" fill="#ff5f57"/><circle cx="19.5" cy="12" r="1.4" fill="#febc2e"/><circle cx="24" cy="12" r="1.4" fill="#28c840"/><rect x="36" y="16" width="24" height="3" rx="1.5" fill="LINE"/><rect x="14" y="23" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="28" width="68" height="11" rx="2.5" fill="#0d4ffb"/><circle cx="19.5" cy="33.5" r="3" fill="#fff"/><rect x="25" y="31" width="28" height="1.8" rx=".9" fill="#fff"/><rect x="25" y="34.6" width="48" height="1.5" rx=".75" fill="#bcd2ff"/><rect x="14" y="43" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="48" width="48" height="2" rx="1" fill="LINE"/>"##;

static DASHED_RING: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="1" y="1" width="30" height="30" rx="7" fill="none" stroke="#000" stroke-width="2" stroke-dasharray="6 3" stroke-dashoffset="8.5"/></svg>"##[..])
});

static SENT: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    [
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2" stroke-dasharray="2.4 1.84" transform="rotate(-79 6 6)"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
    ]
    .map(svg::Handle::from_memory)
});

static VIDEO_TAG: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h480q33 0 56.5 23.5T720-720v180l160-160v440L720-420v180q0 33-23.5 56.5T640-160H160Zm0-80h480v-480H160v480Zm0 0v-480 480Z"/></svg>"##[..])
});

static PLAY_FILL: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000"><path d="M5.798 20.552C4.465 21.337 3 20.435 3 18.888V5.112c0-1.547 1.465-2.45 2.798-1.664l13.214 7.78c1.317.776 1.317 2.552 0 3.328L5.798 20.552Z"/></svg>"##[..])
});

static EYES: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 46" fill="#fff"><path d="M22.1197 25.5092C23.0085 30.1093 20.937 33.1 16.2324 34.009C11.5277 34.918 8.49126 32.9143 7.60244 28.3142L6.68822 23.5827C5.7994 18.9826 7.87085 15.9919 12.5755 15.0829C17.2802 14.1738 20.3166 16.1776 21.2054 20.7777L22.1197 25.5092ZM14.8746 30.5902C15.1743 32.1411 16.6446 33.1113 18.2307 32.8049C19.8168 32.4984 20.8199 31.0503 20.5202 29.4994C20.2205 27.9485 18.7503 26.9783 17.1641 27.2847C15.578 27.5912 14.575 29.0393 14.8746 30.5902Z"/><path d="M39.1455 22.1235C40.0344 26.7236 37.9629 29.7142 33.2583 30.6233C28.5536 31.5323 25.5171 29.5286 24.6283 24.9284L23.7141 20.1969C22.8253 15.5968 24.8967 12.6061 29.6014 11.6971C34.306 10.7881 37.3425 12.7918 38.2313 17.3919L39.1455 22.1235ZM31.9005 27.2045C32.2002 28.7554 33.6704 29.7256 35.2566 29.4191C36.8427 29.1126 37.8457 27.6645 37.5461 26.1136C37.2464 24.5627 35.7761 23.5925 34.19 23.899C32.6039 24.2055 31.6008 25.6536 31.9005 27.2045Z"/></svg>"##[..])
});

static REPLY_CURVE: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    ["", r#" transform="matrix(-1 0 0 1 26 0)""#].map(|flip| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0.5 0.5 25 19"><path{flip} fill="currentColor" fill-rule="evenodd" d="M17 15.5C9.544 15.5 3.5 9.456 3.5 2V.5h-3V2c0 9.113 7.387 16.5 16.5 16.5h6a1.5 1.5 0 0 0 0-3z"/></svg>"#).into_bytes())
    })
});

static TUNE: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M9.167 17.5v-5h1.666v1.667H17.5v1.666h-6.667V17.5zM2.5 15.833v-1.666h5v1.666zM5.833 12.5v-1.667H2.5V9.167h3.333V7.5H7.5v5zm3.334-1.667V9.167H17.5v1.666zM12.5 7.5v-5h1.667v1.667H17.5v1.666h-3.333V7.5zm-10-1.667V4.167h8.333v1.666z"/></svg>"##[..])
});

static PIN: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21.436 7.586l-3.998-4.02c-.752-.756-2.063-.764-2.83-.006-.196.196-.35.436-.418.629-.653 1.362-1.354 2.215-2.254 2.727l-.217.105c-.968.485-2.285.979-4.719.979-.266 0-.521.052-.766.152-.484.202-.879.595-1.082 1.084-.199.484-.199 1.041 0 1.525.104.249.25.471.435.651l3.235 3.235-3.822 5.353 5.352-3.822 3.227 3.227c.186.189.406.339.656.441.247.103.503.154.766.154s.519-.052.765-.154c.498-.205.883-.592 1.08-1.078.103-.242.155-.507.155-.768 0-2.436.494-3.752.978-4.721.496-.992 1.369-1.748 2.754-2.414.271-.104.51-.256.711-.457.772-.782.768-2.051-.008-2.822zm-5.248 4.801c-.819 1.643-1.188 3.37-1.195 5.604l-7.993-7.991c2.139 0 3.814-.335 5.396-1.084l.235-.105c1.399-.699 2.468-1.893 3.388-3.834l3.924 4.051c-1.863.893-3.056 1.96-3.755 3.359z"/></svg>"##[..])
});

static BELL_OFF: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742"/><path d="m2 2 20 20"/><path d="M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"/></svg>"##[..])
});

static COMPOSER_ICONS: std::sync::LazyLock<[svg::Handle; 9]> = std::sync::LazyLock::new(|| {
    [
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M9.167 10.833h-5V9.166h5v-5h1.666v5h5v1.667h-5v5H9.167z"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path transform="rotate(45 10 10)" fill="currentColor" d="M9.167 10.833h-5V9.166h5v-5h1.666v5h5v1.667h-5v5H9.167z"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M12.917 9.167q.52 0 .885-.365t.365-.885-.365-.885a1.2 1.2 0 0 0-.885-.365q-.522 0-.886.365a1.2 1.2 0 0 0-.364.885q0 .52.364.885t.886.365m-5.834 0q.522 0 .886-.365t.364-.885q0-.52-.364-.885a1.2 1.2 0 0 0-.886-.365q-.52 0-.885.365a1.2 1.2 0 0 0-.365.885q0 .52.365.885t.885.365M10 14.584q1.416 0 2.573-.802a4.57 4.57 0 0 0 1.677-2.115h-8.5a4.57 4.57 0 0 0 1.677 2.115q1.156.802 2.573.802m0 3.75a8.1 8.1 0 0 1-3.25-.657 8.4 8.4 0 0 1-2.646-1.78 8.4 8.4 0 0 1-1.781-2.647A8.1 8.1 0 0 1 1.667 10q0-1.729.656-3.25a8.4 8.4 0 0 1 1.781-2.646 8.4 8.4 0 0 1 2.646-1.78A8.1 8.1 0 0 1 10 1.666q1.73 0 3.25.656a8.4 8.4 0 0 1 2.646 1.781 8.4 8.4 0 0 1 1.781 2.646 8.1 8.1 0 0 1 .656 3.25 8.1 8.1 0 0 1-.656 3.25 8.4 8.4 0 0 1-1.781 2.646 8.4 8.4 0 0 1-2.646 1.781 8.1 8.1 0 0 1-3.25.657m0-1.667q2.792 0 4.73-1.938 1.936-1.937 1.937-4.729 0-2.79-1.938-4.729Q12.792 3.334 10 3.334T5.27 5.27t-1.937 4.73q0 2.79 1.938 4.729Q7.209 16.667 10 16.667"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M10 11.667a2.4 2.4 0 0 1-1.77-.73 2.4 2.4 0 0 1-.73-1.77v-5q0-1.042.73-1.77a2.4 2.4 0 0 1 1.77-.73 2.4 2.4 0 0 1 1.77.73q.73.728.73 1.77v5a2.4 2.4 0 0 1-.73 1.77 2.4 2.4 0 0 1-1.77.73M9.167 17.5v-2.562Q6.999 14.646 5.583 13T4.167 9.167h1.666q0 1.729 1.22 2.948Q8.27 13.333 10 13.334q1.729 0 2.948-1.22 1.218-1.218 1.219-2.947h1.666q0 2.187-1.416 3.833-1.418 1.646-3.584 1.938V17.5zM10 10q.354 0 .594-.24.24-.239.24-.593v-5a.8.8 0 0 0-.24-.594.8.8 0 0 0-.594-.24.8.8 0 0 0-.594.24.8.8 0 0 0-.24.594v5q0 .354.24.594T10 10"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M4.167 17.5q-.688 0-1.177-.49a1.6 1.6 0 0 1-.49-1.177V4.167q0-.688.49-1.177.489-.49 1.177-.49h11.666q.688 0 1.177.49.49.489.49 1.177v11.666q0 .688-.49 1.177-.489.49-1.177.49zm0-1.667h11.666V4.167H4.167zm5-3.333h1.25v-5h-1.25zm-3.334 0H7.5q.354 0 .594-.24t.24-.593V10h-1.25v1.25H6.25v-2.5h2.083v-.417a.8.8 0 0 0-.24-.593.8.8 0 0 0-.593-.24H5.833a.8.8 0 0 0-.593.24.8.8 0 0 0-.24.593v3.334q0 .354.24.593.24.24.593.24m5.834 0h1.25v-1.667h1.666v-1.25h-1.666V8.75h2.5V7.5h-3.75z"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M9.583 18.333q-1.915 0-3.25-1.333Q5 15.666 5 13.75V5q0-1.376.98-2.354a3.2 3.2 0 0 1 2.353-.98 3.2 3.2 0 0 1 2.355.98A3.2 3.2 0 0 1 11.667 5v7.916q0 .876-.604 1.48a2.01 2.01 0 0 1-1.48.604 2.01 2.01 0 0 1-1.479-.604 2.01 2.01 0 0 1-.604-1.48V5h1.25v7.916q0 .355.24.594.24.24.593.24.354 0 .594-.24t.24-.594V5q0-.875-.604-1.48a2.01 2.01 0 0 0-1.48-.603q-.875-.001-1.479.604A2.01 2.01 0 0 0 6.25 5v8.75q0 1.374.98 2.354a3.2 3.2 0 0 0 2.353.98 3.2 3.2 0 0 0 2.355-.98 3.2 3.2 0 0 0 .979-2.354V5h1.25v8.75q0 1.916-1.334 3.25-1.333 1.333-3.25 1.333"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M4.697 15.303A7.5 7.5 0 0 0 17.5 9.993l-1.667.066a5.833 5.833 0 1 1-1.708-4.184h-1.768v1.768h4.714V2.929h-1.768v1.768A7.5 7.5 0 0 0 4.697 15.303"/><path fill="currentColor" d="M10.884 5.833v5L7.55 12.917l-.834-1.25L9.217 10V5.833z"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M14.167 14.5v-9H12.5v9zm-3.334 3.833V1.667H9.167v16.666zm6.667-6.666V8.333h-1.667v3.333zM7.5 13.5v-7H5.833v7zm-3.333-1.834V8.333H2.5v3.333z"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="currentColor" d="M4.214 8.874q.65 0 1.182.322.536.323.859.86t.322 1.187q0 .645-.322 1.182a2.5 2.5 0 0 1-.86.86 2.24 2.24 0 0 1-1.181.321q-.645 0-1.182-.322a2.5 2.5 0 0 1-.86-.86 2.26 2.26 0 0 1-.321-1.18q0-.646.316-1.182a2.5 2.5 0 0 1 .86-.86 2.24 2.24 0 0 1 1.187-.328m0 .902q-.398 0-.73.204-.334.199-.533.533a1.4 1.4 0 0 0-.198.73q0 .398.198.73.2.334.532.532.333.199.73.2.4-.002.731-.2.333-.2.532-.531.198-.333.199-.73 0-.4-.2-.731-.198-.333-.53-.533a1.37 1.37 0 0 0-.731-.204m9.432.962q.208 0 .354.15.15.146.15.355 0 .205-.15.355a.48.48 0 0 1-.354.144H8.3a.495.495 0 0 1-.499-.499q0-.21.144-.354.145-.15.355-.15zM4.214 2.66q.65 0 1.182.322.536.317.859.854t.322 1.187q0 .645-.322 1.182a2.5 2.5 0 0 1-.86.864 2.24 2.24 0 0 1-1.181.323q-.645 0-1.182-.323a2.5 2.5 0 0 1-.86-.864 2.26 2.26 0 0 1-.321-1.182q0-.65.316-1.187.322-.537.86-.854a2.27 2.27 0 0 1 1.187-.322m.924 1.197q-.156 0-.258.162l-.956 1.536-.414-.52a.4.4 0 0 0-.118-.103.27.27 0 0 0-.14-.033.27.27 0 0 0-.198.086.3.3 0 0 0-.086.215q0 .124.08.215l.601.726a.6.6 0 0 0 .124.112q.07.043.162.043a.33.33 0 0 0 .29-.161l1.143-1.778a.43.43 0 0 0 .07-.225.25.25 0 0 0-.091-.194.3.3 0 0 0-.21-.08m8.508.666q.209.001.354.146.15.146.15.354 0 .21-.15.36a.48.48 0 0 1-.354.15H8.3a.47.47 0 0 1-.355-.15.5.5 0 0 1-.144-.36q0-.21.144-.354a.48.48 0 0 1 .355-.146z"/></svg>"##[..],
    ]
    .map(svg::Handle::from_memory)
});

static TRASH: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><path fill="currentColor" d="M432 32H312l-9.4-18.7A24 24 0 0 0 281.1 0H166.8a23.72 23.72 0 0 0-21.4 13.3L136 32H16A16 16 0 0 0 0 48v32a16 16 0 0 0 16 16h416a16 16 0 0 0 16-16V48a16 16 0 0 0-16-16zM53.2 467a48 48 0 0 0 47.9 45h245.8a48 48 0 0 0 47.9-45L416 128H32z"/></svg>"#[..])
});

/// Chat row hover actions: archive, unarchive, mark read, mark unread, context menu and delete draft.
static ROW_ACTIONS: std::sync::LazyLock<[svg::Handle; 7]> = std::sync::LazyLock::new(|| {
    [
        ("0 0 14 14", "M1.55556 14C1.12778 14 0.761444 13.8478 0.456555 13.5434C0.152185 13.2386 0 12.8722 0 12.4444V2.72222C0 2.52778 0.0324074 2.35278 0.0972222 2.19722C0.162037 2.04167 0.246296 1.89259 0.35 1.75L1.43889 0.427778C1.54259 0.285185 1.67222 0.178111 1.82778 0.106556C1.98333 0.0355186 2.15185 0 2.33333 0H11.6667C11.8481 0 12.0167 0.0355186 12.1722 0.106556C12.3278 0.178111 12.4574 0.285185 12.5611 0.427778L13.65 1.75C13.7537 1.89259 13.838 2.04167 13.9028 2.19722C13.9676 2.35278 14 2.52778 14 2.72222V12.4444C14 12.8722 13.8478 13.2386 13.5434 13.5434C13.2386 13.8478 12.8722 14 12.4444 14H1.55556ZM1.86667 2.33333H12.1333L11.4722 1.55556H2.52778L1.86667 2.33333ZM1.55556 3.88889V12.4444H12.4444V3.88889H1.55556ZM7 11.6667L10.1111 8.55556L9.02222 7.46667L7.77778 8.71111V5.44444H6.22222V8.71111L4.97778 7.46667L3.88889 8.55556L7 11.6667Z"),
        ("0 0 14 14", "M1.55556 14C1.12778 14 0.761444 13.8478 0.456555 13.5434C0.152185 13.2386 0 12.8722 0 12.4444V2.72222C0 2.52778 0.0324074 2.35278 0.0972222 2.19722C0.162037 2.04167 0.246296 1.89259 0.35 1.75L1.43889 0.427778C1.54259 0.285185 1.67222 0.178111 1.82778 0.106556C1.98333 0.0355186 2.15185 0 2.33333 0H11.6667C11.8481 0 12.0167 0.0355186 12.1722 0.106556C12.3278 0.178111 12.4574 0.285185 12.5611 0.427778L13.65 1.75C13.7537 1.89259 13.838 2.04167 13.9028 2.19722C13.9676 2.35278 14 2.52778 14 2.72222V12.4444C14 12.8722 13.8478 13.2386 13.5434 13.5434C13.2386 13.8478 12.8722 14 12.4444 14H1.55556ZM1.86667 2.33333H12.1333L11.4722 1.55556H2.52778L1.86667 2.33333ZM1.55556 3.88889V12.4444H12.4444V3.88889H1.55556ZM7 5L3.88889 8.11111L4.97778 9.2L6.22222 7.95556V11.2222H7.77778V7.95556L9.02222 9.2L10.1111 8.11111L7 5Z"),
        ("0 0 20 20", "M14.48 16.792 12 14.333l1.063-1.062 1.416 1.416 3.167-3.187 1.062 1.063zM2 18V3.5q0-.604.448-1.052A1.44 1.44 0 0 1 3.5 2h13q.604 0 1.052.448Q18 2.897 18 3.5v6h-1.5v-6h-13v10.875l.875-.875H10.5V15H5z"),
        ("0 0 20 20", "M2.667 18.137V4.934q0-.605.43-1.036.431-.43 1.037-.431h7.408a3.7 3.7 0 0 0 0 1.467H4.134v9.627l.843-.825H15.87V7.795q.421-.092.788-.248t.679-.412v6.601q0 .605-.431 1.036-.432.43-1.036.431H5.6zm12.47-11.736q-.918 0-1.56-.642a2.12 2.12 0 0 1-.641-1.559q0-.915.642-1.558A2.12 2.12 0 0 1 15.136 2q.917 0 1.559.642t.642 1.559q0 .915-.642 1.558a2.12 2.12 0 0 1-1.559.642"),
        ("0 0 181 38", "M19 38c10.493 0 19-8.507 19-19S29.493 0 19 0 0 8.507 0 19s8.507 19 19 19M90.25 38c10.493 0 19-8.507 19-19s-8.507-19-19-19-19 8.507-19 19 8.507 19 19 19M161.5 38c10.493 0 19-8.507 19-19s-8.507-19-19-19-19 8.507-19 19 8.507 19 19 19"),
        ("0 0 20 20", "M7.042 13.999 6 12.957l2.938-2.958L6 7.06 7.042 6.02 10 8.957l2.938-2.937 1.041 1.041L11.042 10l2.937 2.958-1.041 1.042L10 11.06z"),
        ("0 0 20 20", "m5.333 15.834-1.166-1.167L8.833 10 4.167 5.334l1.166-1.167L10 8.834l4.667-4.667 1.166 1.167L11.167 10l4.666 4.667-1.166 1.167L10 11.167z"),
    ]
    .map(|(vb, d)| {
        svg::Handle::from_memory(
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}"><path fill="currentColor" d="{d}"/></svg>"#)
                .into_bytes(),
        )
    })
});

/// Selection bar icons: archive, unarchive, mute, unmute, low priority, inbox, remind me and more.
static SELECTION_ICONS: std::sync::LazyLock<[svg::Handle; 8]> = std::sync::LazyLock::new(|| {
    [
        ("0 0 20 20", "M4.556 17a1.5 1.5 0 0 1-1.1-.457A1.5 1.5 0 0 1 3 15.444V5.722q0-.292.097-.525.098-.232.253-.447l1.089-1.322a1 1 0 0 1 .389-.321Q5.06 3 5.333 3h9.334q.272 0 .505.107a1 1 0 0 1 .39.32L16.65 4.75q.156.214.253.447t.097.525v9.722a1.5 1.5 0 0 1-.457 1.1 1.5 1.5 0 0 1-1.099.456zm.31-11.667h10.267l-.66-.777H5.527zm-.31 1.556v8.555h10.888V6.89zM10 14.667l3.111-3.111-1.089-1.09-1.244 1.245V8.444H9.222v3.267l-1.244-1.244-1.09 1.089z"),
        ("0 0 21 20", "m10.5 8.25-3 3 1.063 1.063 1.187-1.188v3.125h1.5v-3.125l1.188 1.188L13.5 11.25zM5 7v8.5h11V7zm0 10q-.618 0-1.06-.44a1.45 1.45 0 0 1-.44-1.06V5.625q0-.292.11-.562.108-.272.328-.5l1.124-1.125A1.44 1.44 0 0 1 6.126 3h8.75q.294 0 .564.104t.498.333l1.126 1.126q.218.228.328.5.11.27.109.562V15.5q0 .619-.44 1.06-.44.44-1.06.44zm.125-11.5h10.75l-1-1h-8.75z"),
        ("0 0 20 20", "M3.332 15.834v-1.667h1.667V8.334q0-.688.177-1.355t.531-1.27l1.25 1.25q-.146.333-.219.677t-.073.698v5.833h5.167L1.165 3.5l1.167-1.166 15.333 15.333-1.166 1.167-3.042-3zm11.667-3.209-1.667-1.666V8.334a3.2 3.2 0 0 0-.98-2.355A3.2 3.2 0 0 0 10 5q-.542 0-1.042.167a3 3 0 0 0-.917.5L6.832 4.459a5.5 5.5 0 0 1 .896-.584q.48-.249 1.02-.375v-.583q0-.52.365-.885.364-.365.886-.365.52 0 .885.365t.365.885V3.5a4.8 4.8 0 0 1 2.708 1.76 4.88 4.88 0 0 1 1.042 3.074zm-5 5.709q-.688 0-1.177-.49a1.6 1.6 0 0 1-.49-1.177h3.333q0 .687-.49 1.177-.489.49-1.176.49"),
        ("0 0 20 20", "M3.333 15.834v-1.667H5V8.334A4.88 4.88 0 0 1 6.042 5.26 4.8 4.8 0 0 1 8.75 3.5v-.584q0-.52.365-.885.364-.365.885-.365.52 0 .885.365.366.365.365.885V3.5a4.8 4.8 0 0 1 2.708 1.76A4.88 4.88 0 0 1 15 8.335v5.833h1.667v1.667zm6.667 2.5q-.687 0-1.177-.49a1.6 1.6 0 0 1-.49-1.177h3.334q0 .687-.49 1.177t-1.177.49m-3.333-4.167h6.666V8.334a3.2 3.2 0 0 0-.979-2.355A3.2 3.2 0 0 0 10 5a3.2 3.2 0 0 0-2.354.98 3.2 3.2 0 0 0-.98 2.354z"),
        ("0 0 20 20", "M11.667 4.167h6.666v1.556h-6.666zm0 4.278h6.666V10h-6.666zm0 4.278h6.666v1.555h-6.666zm-10-3.5c0 2.784 2.433 5.055 5.416 5.055H7.5v1.556L10 13.5l-2.5-2.333v1.556h-.417c-2.066 0-3.75-1.572-3.75-3.5 0-1.93 1.684-3.5 3.75-3.5H10V4.167H7.083c-2.983 0-5.416 2.271-5.416 5.056"),
        ("0 0 20 20", "M15.833 2.5H4.158c-.925 0-1.65.742-1.65 1.667L2.5 15.833A1.66 1.66 0 0 0 4.158 17.5h11.675c.917 0 1.667-.75 1.667-1.667V4.167c0-.925-.75-1.667-1.667-1.667m0 10H12.5c0 1.383-1.125 2.5-2.5 2.5a2.5 2.5 0 0 1-2.5-2.5H4.158V4.167h11.675z"),
        ("0 0 20 20", "m12.75 13.917 1.167-1.167-3.084-3.083V5.834H9.167v4.5zM10 18.334a8.1 8.1 0 0 1-3.25-.657 8.4 8.4 0 0 1-2.646-1.78 8.4 8.4 0 0 1-1.781-2.647A8.1 8.1 0 0 1 1.667 10q0-1.729.656-3.25a8.4 8.4 0 0 1 1.781-2.646 8.4 8.4 0 0 1 2.646-1.78A8.1 8.1 0 0 1 10 1.666q1.73 0 3.25.656a8.4 8.4 0 0 1 2.646 1.781 8.4 8.4 0 0 1 1.781 2.646 8.1 8.1 0 0 1 .656 3.25 8.1 8.1 0 0 1-.656 3.25 8.4 8.4 0 0 1-1.781 2.646 8.4 8.4 0 0 1-2.646 1.781 8.1 8.1 0 0 1-3.25.657m0-1.667q2.77 0 4.719-1.948 1.947-1.947 1.948-4.719 0-2.77-1.948-4.718T10 3.334 5.281 5.282Q3.334 7.229 3.333 10t1.948 4.72T10 16.666"),
        ("0 0 20 20", "m10 10.5 3.833-3.833L15 7.834l-5 5-5-5 1.167-1.167z"),
    ]
    .map(|(vb, d)| {
        svg::Handle::from_memory(
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}"><path fill="currentColor" d="{d}"/></svg>"#)
                .into_bytes(),
        )
    })
});

static DROP_ARROW: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M12.625 14.375 17 10l-4.375-4.375-1.062 1.063 2.562 2.562H4.5V7H3v3.75h11.125l-2.562 2.563z"/></svg>"##[..])
});

static SYS_ARROW_BACK: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="m6.52 10.833 4.668 4.667L10 16.666 3.333 10 10 3.333 11.188 4.5 6.52 9.166h10.146v1.667z"/></svg>"##[..])
});

static SYS_ROTATE: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M10 18.25q-1.083 0-2.052-.302a6.9 6.9 0 0 1-1.802-.865L7.229 16a5.5 5.5 0 0 0 2.771.75q2.271 0 3.885-1.615Q15.5 13.521 15.5 11.25t-1.615-3.906-4.01-1.594l1.188 1.188L10 8 7 5l3-3 1.063 1.063L9.875 4.25q3-.063 5.063 2.01T17 11.25q0 1.458-.552 2.73a7.1 7.1 0 0 1-1.5 2.218 7.1 7.1 0 0 1-2.219 1.5A6.8 6.8 0 0 1 10 18.25m-5-2-5-5 5-5 5 5zm0-2.125 2.875-2.875L5 8.375 2.125 11.25z"/></svg>"##[..])
});

static SYS_DOWNLOAD: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M10 13.333 5.833 9.166 7 7.958l2.167 2.167V3.333h1.666v6.792L13 7.958l1.167 1.208zm-5 3.333q-.687 0-1.177-.49A1.6 1.6 0 0 1 3.333 15v-2.5H5V15h10v-2.5h1.667V15q0 .687-.49 1.177t-1.177.49z"/></svg>"##[..])
});

static SYS_CLOSE: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="m5.333 15.834-1.166-1.167L8.833 10 4.167 5.334l1.166-1.167L10 8.834l4.667-4.667 1.166 1.167L11.167 10l4.666 4.667-1.166 1.167L10 11.167z"/></svg>"##[..])
});

/// Desktop's system icons file-default, videocam, link, sms, tune, search and done.
static MEDIA_ICONS: std::sync::LazyLock<[svg::Handle; 7]> = std::sync::LazyLock::new(|| {
    [
        "M5 18.334q-.687 0-1.177-.49a1.6 1.6 0 0 1-.49-1.177V3.334q0-.688.49-1.177T5 1.667h6.667l5 5v10q0 .687-.49 1.177t-1.177.49zM10.833 7.5V3.334H5v13.333h10V7.5z",
        "M3.333 16.666q-.687 0-1.177-.49A1.6 1.6 0 0 1 1.666 15V5q0-.688.49-1.177t1.177-.49h10q.688 0 1.177.49Q15 4.312 15 5v3.75l3.333-3.334v9.167L15 11.25V15q0 .687-.49 1.177-.489.49-1.177.49zm0-1.666h10V5h-10z",
        "M12.357 13.536 10 15.893q-1.223 1.222-2.946 1.222t-2.947-1.222q-1.222-1.224-1.222-2.947T4.107 10l2.357-2.357 1.179 1.179-2.357 2.357a2.4 2.4 0 0 0-.737 1.767q0 1.032.737 1.768.736.737 1.768.737 1.03 0 1.767-.737l2.358-2.357zm-4.125-.59-1.178-1.178 4.714-4.714 1.178 1.178zm5.304-.589-1.179-1.178 2.357-2.357a2.4 2.4 0 0 0 .737-1.768 2.4 2.4 0 0 0-.737-1.768 2.4 2.4 0 0 0-1.768-.737q-1.03 0-1.767.737L8.82 7.643 7.643 6.464 10 4.107q1.223-1.222 2.946-1.222t2.947 1.222q1.222 1.224 1.222 2.947T15.893 10z",
        "M6.667 9.167q.354 0 .593-.24.24-.24.24-.593a.8.8 0 0 0-.24-.594.8.8 0 0 0-.593-.24.8.8 0 0 0-.594.24.8.8 0 0 0-.24.594q0 .354.24.593.24.24.594.24m3.333 0q.354 0 .594-.24t.24-.593a.8.8 0 0 0-.24-.594A.8.8 0 0 0 10 7.5a.8.8 0 0 0-.594.24.8.8 0 0 0-.24.594q0 .354.24.593.24.24.594.24m3.333 0q.354 0 .594-.24t.24-.593a.8.8 0 0 0-.24-.594.8.8 0 0 0-.594-.24.8.8 0 0 0-.593.24.8.8 0 0 0-.24.594q0 .354.24.593.24.24.593.24M1.667 18.334v-15q0-.688.49-1.177.489-.49 1.176-.49h13.334q.687 0 1.177.49.49.489.49 1.177v10q0 .687-.49 1.177t-1.177.49H5zm2.625-5h12.375v-10H3.333V14.27z",
        "M9.167 17.5v-5h1.666v1.667H17.5v1.666h-6.667V17.5zM2.5 15.833v-1.666h5v1.666zM5.833 12.5v-1.667H2.5V9.167h3.333V7.5H7.5v5zm3.334-1.667V9.167H17.5v1.666zM12.5 7.5v-5h1.667v1.667H17.5v1.666h-3.333V7.5zm-10-1.667V4.167h8.333v1.666z",
        "m16.333 17.5-5.25-5.25a5.08 5.08 0 0 1-3.167 1.083q-2.27 0-3.843-1.573T2.5 7.917t1.573-3.844T7.917 2.5t3.843 1.573 1.573 3.844a5.08 5.08 0 0 1-1.083 3.166l5.25 5.25zm-8.416-5.833q1.562 0 2.656-1.094t1.094-2.656-1.094-2.657Q9.478 4.167 7.917 4.167T5.26 5.26Q4.167 6.355 4.167 7.917t1.093 2.656 2.657 1.094",
        "m7.958 15-4.75-4.75 1.188-1.187 3.562 3.562 7.646-7.646 1.188 1.188z",
    ]
    .map(|d| {
        svg::Handle::from_memory(
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="{d}"/></svg>"#)
                .into_bytes(),
        )
    })
});

/// Desktop's file row icons: Material draft, csv, folder_zip and audio_file.
static FILE_ICONS: std::sync::LazyLock<[svg::Handle; 4]> = std::sync::LazyLock::new(|| {
    [
        "M240-80q-33 0-56.5-23.5T160-160v-640q0-33 23.5-56.5T240-880h320l240 240v480q0 33-23.5 56.5T720-80H240Zm280-520v-200H240v640h480v-440H520ZM240-800v200-200 640-640Z",
        "M230-360h120v-60H250v-120h100v-60H230q-17 0-28.5 11.5T190-560v160q0 17 11.5 28.5T230-360Zm156 0h120q17 0 28.5-11.5T546-400v-60q0-17-11.5-31.5T506-506h-60v-34h100v-60H426q-17 0-28.5 11.5T386-560v60q0 17 11.5 30.5T426-456h60v36H386v60Zm264 0h60l70-240h-60l-40 138-40-138h-60l70 240ZM160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h640q33 0 56.5 23.5T880-720v480q0 33-23.5 56.5T800-160H160Zm0-80h640v-480H160v480Zm0 0v-480 480Z",
        "M640-480v-80h80v80h-80Zm0 80h-80v-80h80v80Zm0 80v-80h80v80h-80ZM447-640l-80-80H160v480h400v-80h80v80h160v-400H640v80h-80v-80H447ZM160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h240l80 80h320q33 0 56.5 23.5T880-640v400q0 33-23.5 56.5T800-160H160Zm0-80v-480 480Z",
        "M430-200q38 0 64-26t26-64v-150h120v-80H480v155q-11-8-23.5-11.5T430-380q-38 0-64 26t-26 64q0 38 26 64t64 26ZM240-80q-33 0-56.5-23.5T160-160v-640q0-33 23.5-56.5T240-880h320l240 240v480q0 33-23.5 56.5T720-80H240Zm280-520v-200H240v640h480v-440H520ZM240-800v200-200 640-640Z",
    ]
    .map(|d| {
        svg::Handle::from_memory(
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path fill="currentColor" d="{d}"/></svg>"#)
                .into_bytes(),
        )
    })
});

static MAT_FORWARD: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="m640-280-57-56 184-184-184-184 57-56 240 240-240 240ZM80-200v-160q0-83 58.5-141.5T280-560h247L383-704l57-56 240 240-240 240-57-56 144-144H280q-50 0-85 35t-35 85v160H80Z"/></svg>"##[..])
});

static MAT_LINK: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M440-280H280q-83 0-141.5-58.5T80-480q0-83 58.5-141.5T280-680h160v80H280q-50 0-85 35t-35 85q0 50 35 85t85 35h160v80ZM320-440v-80h320v80H320Zm200 160v-80h160q50 0 85-35t35-85q0-50-35-85t-85-35H520v-80h160q83 0 141.5 58.5T880-480q0 83-58.5 141.5T680-280H520Z"/></svg>"##[..])
});

/// Desktop's system icon arrow-back.
static ARROW_BACK: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="m6.52 10.833 4.668 4.667L10 16.666 3.333 10 10 3.333 11.188 4.5 6.52 9.166h10.146v1.667z"/></svg>"##[..])
});

fn fmt_phone(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() == 12 && b.starts_with(b"+1") && b[2..].iter().all(u8::is_ascii_digit) {
        format!("+1 {}-{}-{}", &s[2..5], &s[5..8], &s[8..12])
    } else {
        s.to_string()
    }
}

const RAIL_VIEWS: [Option<InboxTab>; 7] = [
    Some(InboxTab::AllChats),
    Some(InboxTab::Archived),
    None,
    Some(InboxTab::Reminders),
    Some(InboxTab::SendLater),
    Some(InboxTab::Requests),
    Some(InboxTab::LowPriority),
];

fn rail_accounts(st: &BeeperStore) -> Vec<String> {
    let mut accounts: Vec<String> =
        std::iter::once("hungryserv".to_string()).chain(st.accounts.iter().map(|a| a.account_id.clone())).collect();
    accounts.sort_by_cached_key(|a| {
        (a != "hungryserv", st.chats.iter().position(|c| &c.account_id == a).unwrap_or(usize::MAX))
    });
    let spaces = st.network_spaces.iter().filter(|n| accounts.contains(&n.account_id) && space_shown(st, n));
    accounts.extend(spaces.map(|n| n.id.clone()).collect::<Vec<_>>());
    accounts
}

/// Desktop's `allSpaces`: WhatsApp Communities and Telegram Topics are spaces only when their setting is on.
fn space_shown(st: &BeeperStore, n: &NetworkSpace) -> bool {
    match account_network(&n.account_id).kind {
        NetworkKind::WhatsApp => st.settings.on("whatsappCommunities"),
        NetworkKind::Telegram => st.settings.on("telegramTopics"),
        _ => true,
    }
}

/// Finds the network space of a rail entry. Other rail entries are accounts.
fn network_space<'a>(st: &'a BeeperStore, id: &str) -> Option<&'a NetworkSpace> {
    st.network_spaces.iter().find(|n| n.id == id && space_shown(st, n))
}

/// Desktop's space filter: an account shows the chats that are not in one of its shown network spaces.
fn in_rail_entry(st: &BeeperStore, id: &str, c: &Chat) -> bool {
    match network_space(st, id) {
        Some(n) => c.account_id == n.account_id && c.network_id.as_ref() == Some(&n.id),
        None => c.account_id == id && c.network_id.as_ref().is_none_or(|n| network_space(st, n).is_none()),
    }
}

/// Desktop's `rootMessageIDs`: the first loaded message of the reply chain of `m`.
fn reply_root<'s>(st: &'s BeeperStore, mut m: &'s Message) -> &'s str {
    while let Some(&i) = m.linked_message_id.as_ref().and_then(|r| st.msg_idx_by_id.get(r)) {
        m = &st.messages[i];
    }
    &m.id
}

fn network_label(account_id: &str) -> &'static str {
    match brand_key(account_id) {
        "beeper" => "Beeper",
        _ => account_network(account_id).display_name,
    }
}

fn chat_network(c: &Chat) -> String {
    let plat = network_label(&c.account_id);
    c.network_name.as_deref().map_or_else(|| plat.to_string(), |n| format!("{n} • {plat}"))
}

fn chat_search_subtitle(c: &Chat) -> String {
    let net = chat_network(c);
    if let Some(u) =
        (c.chat_type == "single").then(|| c.participants.items.iter().find(|u| u.is_self != Some(true))).flatten()
    {
        let sub = u
            .username
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!("@{}", s.trim_start_matches('@')))
            .or_else(|| u.phone_number.as_deref().filter(|s| !s.is_empty()).map(fmt_phone))
            .or_else(|| u.email.clone().filter(|s| !s.is_empty()));
        if let Some(sub) = sub.filter(|s| s != &c.title) {
            return sub;
        }
    }
    net
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchFilters {
    pub accounts: Vec<String>,
    pub sent: Option<bool>,
    pub group: Option<bool>,
    pub period: Option<usize>,
    pub media: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum SearchPick {
    Account(String),
    Sent(bool),
    Group(bool),
    Period(usize),
    Media(usize),
}

const SEARCH_CHIPS: [&str; 4] = ["Accounts", "Message type", "Time period", "Media"];
const MEDIA_KINDS: [&str; 5] = ["Images", "Videos", "Audio", "Links", "Files"];
const PERIODS: [&str; 6] = ["Last hour", "Today", "Yesterday", "This week", "Last 30 days", "The last year"];

impl SearchFilters {
    fn label(&self, chip: usize) -> Option<String> {
        match chip {
            0 => match self.accounts.as_slice() {
                [] => None,
                [a] => Some(network_label(a).to_string()),
                many => Some(format!("{} Accounts", many.len())),
            },
            1 => {
                let dir = self.sent.map(|s| if s { "Sent" } else { "Received" });
                let kind = self.group.map(|g| if g { "group messages" } else { "direct messages" });
                match (dir, kind) {
                    (None, None) => None,
                    (Some(d), None) => Some(format!("{d} messages")),
                    (None, Some(k)) => Some(format!("{}{}", k[..1].to_uppercase(), &k[1..])),
                    (Some(d), Some(k)) => Some(format!("{d} {k}")),
                }
            }
            2 => self.period.map(|i| PERIODS[i].to_string()),
            _ => (!self.media.is_empty())
                .then(|| self.media.iter().map(|&k| MEDIA_KINDS[k]).collect::<Vec<_>>().join(", ")),
        }
    }

    fn keeps(&self, st: &BeeperStore, room: Option<&str>, m: &Message) -> bool {
        let (after, before) = self.window();
        let group = st.chats[st.chat_idx_by_id[&m.chat_id]].chat_type == "group";
        let at = millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp");
        m.is_deleted != Some(true)
            && room.is_none_or(|r| m.chat_id == r)
            && (self.accounts.is_empty() || self.accounts.contains(&m.account_id))
            && self.sent.is_none_or(|s| (m.is_sender == Some(true)) == s)
            && self.group.is_none_or(|g| group == g)
            && (after..before).contains(&at)
            && (self.media.is_empty() || self.media.iter().any(|&k| has_media(m, k)))
    }

    /// The `[after, before)` window of message times in Unix milliseconds.
    fn window(&self) -> (i64, i64) {
        const DAY: i64 = 86_400_000;
        let now = now_unix_millis();
        let today = now - local_millis(now).rem_euclid(DAY);
        match self.period {
            None => (i64::MIN, i64::MAX),
            Some(0) => (now - 3_600_000, i64::MAX),
            Some(1) => (today, i64::MAX),
            Some(2) => (today - DAY, today),
            Some(3) => (now - 7 * DAY, i64::MAX),
            Some(4) => (now - 30 * DAY, i64::MAX),
            Some(5) => (now - 365 * DAY, i64::MAX),
            Some(i) => unreachable!("time period {i}"),
        }
    }
}

fn has_media(m: &Message, kind: usize) -> bool {
    let typed = |t: &str| m.attachments.iter().any(|a| a.attachment_type == t);
    match kind {
        0 => typed("img"),
        1 => typed("video"),
        2 => typed("audio"),
        3 => !m.links.is_empty() || m.text.as_deref().is_some_and(|t| t.contains("http://") || t.contains("https://")),
        4 => typed("unknown"),
        _ => unreachable!("media kind {kind}"),
    }
}

/// Finds the layout bounds of the container with the given id.
struct Find(iced::widget::Id, Option<iced::Rectangle>);

impl iced::advanced::widget::Operation<iced::Rectangle> for Find {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<iced::Rectangle>)) {
        operate(self)
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: iced::Rectangle) {
        if id == Some(&self.0) {
            self.1 = Some(bounds);
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<iced::Rectangle> {
        iced::advanced::widget::operation::Outcome::Some(self.1.expect("the container is shown"))
    }
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x) * (1.0 - x)
}

/// CSS `cubic-bezier(.22, 1, .36, 1)`.
fn ease_out_quint(x: f32) -> f32 {
    let bezier =
        |t: f32, p1: f32, p2: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * p1 + 3.0 * (1.0 - t) * t * t * p2 + t.powi(3);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, 0.22, 0.36) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, 1.0, 1.0)
}

/// Desktop's `animate-on-insert` for your pending message, with "Enable message animations" on: the eased
/// progress of its 260 ms `message-send-insert` animation.
fn send_rise(st: &BeeperStore, m: &Message) -> Option<f32> {
    let age = now_unix_millis() - millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp");
    let pending = m.is_sender == Some(true) && m.id.starts_with("msg_");
    (pending && age < 260 && st.settings.on("messageAnimations")).then(|| ease_out_quint(age.max(0) as f32 / 260.0))
}

/// Finds how far from the bottom the timeline must scroll to center a message.
/// Finds the timeline offset that shows a message: in the middle, or with the least scroll when `nearest`.
struct Locate(iced::widget::Id, Option<(iced::Rectangle, iced::Rectangle, f32)>, Option<iced::Rectangle>, bool);

impl iced::advanced::widget::Operation<f32> for Locate {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<f32>)) {
        operate(self)
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: iced::Rectangle) {
        if id == Some(&self.0) {
            self.2 = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&iced::widget::Id>,
        bounds: iced::Rectangle,
        content: iced::Rectangle,
        at: iced::Vector,
        _: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&iced::widget::Id::new("timeline")) {
            self.1 = Some((bounds, content, at.y));
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<f32> {
        let (view, content, at) = self.1.expect("the timeline is shown");
        let msg = self.2.expect("the message is in the timeline");
        let max = (content.height - view.height).max(0.0);
        let y = msg.y - content.y;
        let top = match self.3 {
            true => at.max(y + msg.height - view.height).min(y),
            false => y - (view.height - msg.height) / 2.0,
        };
        iced::advanced::widget::operation::Outcome::Some(max - top.clamp(0.0, max))
    }
}

/// The forward dialog: the message, the picked chats, and the search text.
struct Forward {
    mid: String,
    picked: Vec<String>,
    query: String,
}

/// The attachment that a carousel shows, its quarter turns, and if it shows the Media window items.
#[derive(Clone)]
struct Carousel {
    mid: String,
    ai: usize,
    turns: u8,
    in_media: bool,
}

/// Desktop's Media window: its tab (Media, Links or Docs), its size, how many items it shows,
/// its filters (sent by all, you or others; oldest first; the caption search), and if its filter menu is open.
struct MediaWindow {
    id: window::Id,
    tab: usize,
    size: Size,
    shown: usize,
    sent_by: u8,
    oldest: bool,
    search: Option<String>,
    menu: bool,
    cursor: Point,
    /// The item whose context menu is open, as (message, attachment or link index), and where.
    item_menu: Option<(String, usize, Point)>,
    select: bool,
    picked: Vec<(String, usize)>,
    /// The (chat, message) pairs that the delete dialog asks about, and how many items it skips.
    deleting: Option<(Vec<(String, String)>, usize)>,
}

/// If you can delete a message for everyone, like Desktop's Media window asks.
fn deletable(m: &Message) -> bool {
    m.is_sender == Some(true) && m.is_deleted != Some(true)
}

/// Desktop's Media window page size.
const MEDIA_PAGE: usize = 50;
const MEDIA_TABS: [&str; 3] = ["Media", "Links", "Docs"];

/// The items of the Media window tab, as (message index, attachment or link index), in its order.
/// Like Desktop, it skips low priority chats and message requests.
fn media_items(st: &BeeperStore, w: &MediaWindow) -> Vec<(usize, usize)> {
    let (tab, needle) = (w.tab, w.search.as_deref().unwrap_or("").trim().to_lowercase());
    let shown = |m: &&Message| {
        m.is_deleted != Some(true)
            && st.chats[st.chat_idx_by_id[&m.chat_id]].flags & (CHAT_LOW_PRIORITY | CHAT_REQUEST) == 0
            && match w.sent_by {
                0 => true,
                1 => m.is_sender == Some(true),
                2 => m.is_sender != Some(true),
                n => unreachable!("sent by {n}"),
            }
            && m.text.as_deref().unwrap_or("").to_lowercase().contains(&needle)
    };
    let mut items: Vec<_> = st
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| shown(m))
        .flat_map(|(i, m)| {
            let parts: Vec<usize> = match tab {
                1 => (0..m.links.len()).collect(),
                _ => {
                    let kinds: &[&str] = if tab == 0 { &["img", "video"] } else { &["unknown"] };
                    let atts = m.attachments.iter().enumerate();
                    atts.filter(|(_, a)| kinds.contains(&a.attachment_type.as_str())).map(|(k, _)| k).collect()
                }
            };
            let t = iso_millis(&m.timestamp);
            parts.into_iter().map(move |k| (t, i, k))
        })
        .collect();
    items.sort_by_key(|&(t, ..)| if w.oldest { t } else { -t });
    items.into_iter().map(|(_, i, k)| (i, k)).collect()
}

/// The items that the Media window filmstrip shows around the current one, as a range.
fn filmstrip(size: &Size, at: usize, len: usize) -> (usize, usize) {
    let n = ((size.width - 20.0) / 52.0) as usize;
    let start = at.saturating_sub(n / 2).min(len.saturating_sub(n));
    (start, (start + n).min(len))
}

/// Desktop's Media window date group of a day, like "Today", "Last week" or "March 2025".
fn media_group(d: i64) -> String {
    let t = today();
    let week = t - (t + 3).rem_euclid(7);
    let (y, m, _) = civil_from_days(d);
    match t - d {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ if d >= week => "This week".into(),
        _ if d >= week - 7 => "Last week".into(),
        _ if y == civil_from_days(t).0 => MONTHS[m as usize - 1].into(),
        _ => format!("{} {y}", MONTHS[m as usize - 1]),
    }
}

/// The en-US medium date of a day, like "Oct 17, 2026".
fn medium_date(d: i64) -> String {
    let (y, m, dd) = civil_from_days(d);
    format!("{} {dd}, {y}", &MONTHS[m as usize - 1][..3])
}

/// Desktop's `formatRelativeDate`: "Today", a weekday, "Tue, Oct 7", or "Oct 7, 2025" in other years.
fn relative_date(iso: &str) -> String {
    let d = day_num(iso);
    match civil_from_days(d).0 == civil_from_days(today()).0 {
        true => day_label(iso),
        false => medium_date(d),
    }
}

/// Desktop's attachment kind label of a MIME type.
fn file_kind(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        m if m.starts_with("audio/") => "Audio",
        "application/pdf" => "PDF",
        m if m.starts_with("text/") => "Text",
        m if m.starts_with("application/") => "Document",
        _ => "File",
    }
}

/// Desktop's file type label: the extension in capitals, or a name for some MIME types.
fn file_label(a: &Attachment) -> Option<String> {
    let ext = a.file_name.as_deref().and_then(|n| n.rsplit_once('.')).filter(|(stem, e)| {
        !stem.is_empty() && (1..=10).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    let named = || match a.mime_type.as_deref()? {
        "application/pdf" => Some("PDF"),
        "application/zip" | "application/x-zip-compressed" | "multipart/x-zip" => Some("ZIP"),
        "text/csv" => Some("CSV"),
        "text/plain" => Some("TXT"),
        "video/quicktime" => Some("MOV"),
        "video/x-msvideo" => Some("AVI"),
        _ => None,
    };
    ext.map(|(_, e)| e.to_ascii_uppercase()).or_else(|| named().map(String::from))
}

/// Desktop's file row icon: audio, CSV, zip, or document.
fn file_icon(a: &Attachment) -> &'static svg::Handle {
    let (mime, name) = (a.mime_type.as_deref().unwrap_or(""), a.file_name.as_deref().unwrap_or("").to_lowercase());
    let k = match () {
        _ if a.attachment_type == "audio" => 3,
        _ if mime == "text/csv" || name.ends_with(".csv") => 1,
        _ if matches!(mime, "application/zip" | "application/x-zip-compressed" | "multipart/x-zip")
            || name.ends_with(".zip") =>
        {
            2
        }
        _ => 0,
    };
    &FILE_ICONS[k]
}

/// Desktop's media duration, like "00:05" or "01:02:03".
fn clip_time(secs: f64) -> String {
    let s = secs as u64;
    let hours = (s >= 3600).then(|| format!("{:02}:", s / 3600));
    format!("{}{:02}:{:02}", hours.unwrap_or_default(), s % 3600 / 60, s % 60)
}

/// The chats the forward dialog offers: recent chats, or all chats that match the query.
fn forward_chats<'s>(st: &'s BeeperStore, f: &Forward) -> Vec<&'s Chat> {
    let q = f.query.trim().to_lowercase();
    let mut v: Vec<&Chat> = match q.is_empty() && !st.recent_chat_ids.is_empty() {
        true => st.recent_chat_ids.iter().filter_map(|id| st.chat_idx_by_id.get(id).map(|&i| &st.chats[i])).collect(),
        false => {
            let hit =
                |c: &&Chat| c.title.to_lowercase().contains(&q) || chat_search_subtitle(c).to_lowercase().contains(&q);
            let mut v: Vec<&Chat> = st.chats.iter().filter(hit).collect();
            v.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
            v
        }
    };
    v.retain(|c| c.flags & CHAT_HIDDEN == 0 && !f.picked.contains(&c.id));
    v
}

/// CSS `filter: brightness(k)`.
fn brightness(c: Color, k: f32) -> Color {
    Color { r: c.r * k, g: c.g * k, b: c.b * k, ..c }
}

fn gap_below(m: &Message, next: &Message) -> f32 {
    let ms = |m: &Message| millis_from_rfc3339(&m.timestamp).expect("message timestamp");
    match (ms(next) - ms(m)).abs() / 60_000 {
        _ if day_num(&m.timestamp) != day_num(&next.timestamp) => 0.0,
        241.. => 26.0,
        6.. => 13.0,
        2.. => 4.0,
        _ if m.sender_id == next.sender_id => 2.0,
        _ => 4.0,
    }
}

static LOGO: std::sync::LazyLock<svg::Handle> =
    std::sync::LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/beeper.svg")));

pub fn short_title(c: &Chat) -> &str {
    let first_word = c.chat_type != "group" && c.title.chars().any(char::is_alphabetic);
    if first_word {
        c.title.split_whitespace().next().expect("a letter")
    } else {
        &c.title
    }
}

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
    /// The tray icon is registered with the tray host.
    pub tray_up: bool,
    /// The main window is hidden in the tray.
    pub hidden: bool,
    /// The unread count and window state last sent to the tray and the app badge, and whether they can be stale.
    tray_sent: (Option<(u32, bool)>, bool),
    pub system_dark: bool,
    pub dark: bool,
    pub zoom: f32,
    pub pal: Palette,
    pub active_tab: InboxTab,
    pub selected_account_filter: Option<String>,
    pub selected_chat_id: Option<String>,
    pub hotkey_query: String,
    pub pasted: Option<PendingMedia>,
    pub composer_links: Vec<(String, LinkState)>,
    pub show_filter_menu: bool,
    /// The chat whose context menu is open, and where it opens in the window.
    pub chat_menu: Option<(String, Point)>,
    /// Chats picked by Begin Selecting; selection mode is on while this is not empty.
    pub selection: Vec<String>,
    /// The open selection bar menu (Remind Me or More) and the x of its button.
    selection_menu: Option<(u8, f32)>,
    chat_sub: Option<u8>,
    /// The reminder time each chat was last reminded of.
    reminded: HashMap<String, i64>,
    pub search_room: Option<String>,
    pub search_filters: SearchFilters,
    pub search_select: Option<(usize, iced::Rectangle)>,
    cursor: Point,
    message_menu: Option<(String, Point)>,
    selected_msg: Option<String>,
    forward: Option<Forward>,
    carousel: Option<Carousel>,
    media_window: Option<MediaWindow>,
    /// On X11 the copied image stays available only while its clipboard lives.
    image_clipboard: Option<arboard::Clipboard>,
    pub filters: u8,
    pub show_attach_menu: bool,
    pub show_gif_picker: bool,
    /// The URL and text of the formatting menu's link dialog.
    link_dialog: Option<(String, String)>,
    emoji: Option<emoji::Picker>,
    emoji_gen: u32,
    emoji_hovered: [bool; 2],
    booted: String,
    focused: bool,
    notified: HashSet<String>,
    notifications: HashMap<u32, String>,
    pending: notify::Pending,
    pub gif_query: String,
    pub gifs: Vec<KlipyGif>,
    pub gif_loading: bool,
    pub gif_error: Option<String>,
    pub command_palette_query: String,
    pub cmd_bar_commands_mode: bool,
    pub search_expanded: bool,
    pub creating_chat: Option<u32>,
    pub new_chats: Vec<u32>,
    pub new_chat_query: String,
    pub new_chat_expanded: bool,
    pub join_matrix_mode: bool,
    pub composer: text_editor::Content,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub nickname_edit_buffer: String,
    pub recovery_code_input: String,
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
    pub confirm: Option<u8>,
    delete_target: Option<String>,
    mods: keyboard::Modifiers,
    reminder_menu: Option<Point>,
    audio: voice::Player,
    pub show_command_palette: bool,
    pub active_settings_tab: SettingsTab,
    pub settings_query: String,
    pub adding_account: bool,
    pub show_right_panel: bool,
    pub quick_look_target: Option<(String, String)>,
    pub expanded_history_msg_id: Option<String>,
    pub hovered_msg: Option<String>,
    quick_open: bool,
    /// The selected emoji autocomplete row.
    suggest: usize,
    /// The composer text where Escape closed the emoji autocomplete.
    suggest_hidden: Option<String>,
    pub hovered_chat: Option<String>,
    pub flash: Option<(String, std::time::Instant)>,
    /// The first message IDs of the expanded membership event groups.
    open_members: HashSet<String>,
    last_click: Option<(String, std::time::Instant)>,
    pub status_banner: String,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    pub dragging: bool,
    pub main_window: Option<window::Id>,
    main_size: Size,
    pub settings_window: Option<window::Id>,
    pub login_window: Option<window::Id>,
    pub login: Login,
    pub cloud: Arc<Cloud>,
    pub avatars: HashMap<String, Option<image::Handle>>,
    pub media: HashMap<String, Option<Result<Frames, String>>>,
    pub resolving_media: HashMap<String, bool>,
    pub playing_video: Option<PlayingVideo>,
    started: std::time::Instant,
}

impl BeeperGuiApp {
    pub fn new(store: SharedStore) -> Self {
        ensure_fonts_loaded();
        let os = OsKind::current();
        let display_server = os.detect_display_server(&std::env::vars().collect());
        let (theme, panel, first_chat, cloud, source) = {
            let st = store.read().unwrap();
            let first = st.signed_in().then(|| NOTE_CHAT_ID.to_string());
            let s = &st.settings;
            let cloud = Arc::new(Cloud::new(&st.config.api_url, &st.config.homeserver));
            let source = st
                .setup
                .is_none()
                .then(|| current_original_beeper_data_dir(st.config.profile.as_deref()))
                .filter(|d| has_beeper_data(d));
            (s.theme.is_dark(false), false, first, cloud, source)
        };
        let pending = notify::Pending::new(&store.read().unwrap().chats);
        let mut app = Self {
            store,
            os,
            display_server,
            tray_up: false,
            hidden: false,
            tray_sent: (None, true),
            pal: crate::style::palette(theme),
            system_dark: false,
            dark: theme,
            zoom: 1.0,
            active_tab: InboxTab::AllChats,
            selected_account_filter: None,
            selected_chat_id: None,
            hotkey_query: String::new(),
            pasted: None,
            composer_links: Vec::new(),
            show_filter_menu: false,
            chat_menu: None,
            chat_sub: None,
            reminded: HashMap::new(),
            search_room: None,
            search_filters: SearchFilters::default(),
            search_select: None,
            cursor: Point::ORIGIN,
            message_menu: None,
            selected_msg: None,
            image_clipboard: None,
            forward: None,
            carousel: None,
            media_window: None,
            mods: keyboard::Modifiers::empty(),
            filters: 0,
            show_attach_menu: false,
            show_gif_picker: false,
            link_dialog: None,
            emoji: None,
            emoji_gen: 0,
            emoji_hovered: [false; 2],
            booted: beeper_core::config::now_rfc3339_millis(),
            focused: false,
            notified: HashSet::new(),
            notifications: HashMap::new(),
            pending,
            gif_query: String::new(),
            gifs: Vec::new(),
            gif_loading: false,
            gif_error: None,
            command_palette_query: String::new(),
            cmd_bar_commands_mode: false,
            search_expanded: false,
            creating_chat: None,
            new_chats: Vec::new(),
            new_chat_query: String::new(),
            new_chat_expanded: false,
            join_matrix_mode: false,
            composer: text_editor::Content::new(),
            reply_to_message_id: None,
            editing_message_id: None,
            nickname_edit_buffer: String::new(),
            recovery_code_input: String::new(),
            devices: None,
            confirm: None,
            delete_target: None,
            reminder_menu: None,
            audio: Default::default(),
            selection: Vec::new(),
            selection_menu: None,
            show_command_palette: false,
            active_settings_tab: SettingsTab::General,
            settings_query: String::new(),
            adding_account: false,
            show_right_panel: panel,
            quick_look_target: None,
            expanded_history_msg_id: None,
            hovered_msg: None,
            quick_open: false,
            suggest: 0,
            suggest_hidden: None,
            hovered_chat: None,
            flash: None,
            open_members: HashSet::new(),
            status_banner: String::new(),
            dnd_muted: false,
            sidebar_width: 244.0,
            dragging: false,
            main_window: None,
            main_size: Size::new(1264.0, 893.0),
            settings_window: None,
            login_window: None,
            login: Login::new(source),
            cloud,
            avatars: HashMap::new(),
            media: HashMap::new(),
            resolving_media: HashMap::new(),
            playing_video: None,
            started: std::time::Instant::now(),
            last_click: None,
        };
        if let Some(id) = first_chat {
            app.select_chat(&id);
        }
        app
    }

    pub fn boot(store: SharedStore) -> (Self, Task<UiMessage>) {
        let mut app = Self::new(store);
        let signed_in = app.selected_chat_id.is_some();
        let mut settings =
            if signed_in { Self::window_settings(Size::new(1264.0, 893.0)) } else { Self::login_window_settings() };
        let st = app.store.read().unwrap();
        app.hidden = signed_in && st.settings.on("startMinimized") && st.settings.on("showTrayIcon");
        drop(st);
        settings.visible = !app.hidden;
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
        let visible = self.visible_chats(&st);
        let senders = visible.iter().copied().chain(st.chats.iter()).filter_map(|c| {
            let &i = st.chat_msg_indices.get(&c.id)?.last()?;
            preview_sender(c, &st.messages[i])?.img_url.as_deref()
        });
        let recents = st.recent_chat_ids.iter().filter_map(|id| st.chat_idx_by_id.get(id).map(|&i| &st.chats[i]));
        let raw = visible
            .iter()
            .copied()
            .chain(recents)
            .chain(&st.chats)
            .flat_map(chat_avatar_urls)
            .chain(members)
            .chain(senders);
        let mut urls = Vec::new();
        for u in raw {
            if !self.avatars.contains_key(u) {
                self.avatars.insert(u.to_string(), None);
                urls.push(u.to_string());
            }
        }
        drop(st);
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

    fn pulse(&self, delay: u128, f: impl Fn(f32) -> f32) -> f32 {
        f(((self.started.elapsed().as_millis() + 1000 - delay) % 1000) as f32 / 1000.0)
    }

    fn typing_bubble<'a>(&self, chat: &Chat, ids: &[String]) -> El<'a> {
        let p = &self.pal;
        let dot = |delay| {
            let a = self.pulse(delay, |t| 0.2 + 0.2 * (1.0 - (2.0 * t - 1.0).abs()));
            container(Space::new()).width(6).height(6).style(fill_box(p.text.scale_alpha(a), 3.0))
        };
        let bubble =
            container(row![dot(0), dot(340), dot(670)].spacing(4)).padding([12, 11]).style(fill_box(p.received, 16.0));
        let faces = (chat.chat_type == "group").then(|| {
            let users = ids.iter().filter_map(|id| chat.participants.items.iter().find(|u| u.id == *id));
            let layers = users.enumerate().map(|(i, u)| {
                pin(self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 20.0))
                    .x(i as f32 * 10.0)
                    .into()
            });
            container(Stack::with_children(layers)).center_x(34)
        });
        container(row![].push(faces).push(bubble).spacing(8).align_y(Center))
            .padding(Padding::ZERO.left(8.0).top(8.0))
            .into()
    }

    fn typing_badge<'a>(&self) -> El<'a> {
        let p = &self.pal;
        let dot = |delay| {
            let a = self.pulse(delay, |t| match t {
                ..0.15 => 0.2 + 0.2 * t / 0.15,
                ..0.3 => 0.4 - 0.2 * (t - 0.15) / 0.15,
                _ => 0.2,
            });
            container(Space::new()).width(2).height(2).style(fill_box(p.text.scale_alpha(a), 1.0))
        };
        container(row![dot(0), dot(100), dot(200)].spacing(2))
            .center_y(12)
            .padding([0, 3])
            .style(fill_box(p.card, 6.0))
            .into()
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
            exit_on_close_request: false,
            icon: Some({
                let img =
                    ::image::load_from_memory(include_bytes!("../assets/icon.png")).expect("the app icon is a PNG");
                let img = img.into_rgba8();
                let (w, h) = img.dimensions();
                window::icon::from_rgba(img.into_raw(), w, h).expect("the app icon is valid RGBA")
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
        match () {
            _ if Some(id) == self.settings_window => "Settings",
            _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => "Media",
            _ => "Beeper",
        }
        .into()
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
        self.tray_sent.1 = true;
    }

    /// Splits the visible chats into the pinned tiles and the list rows, in display order.
    fn pins_and_rows<'a>(&self, st: &'a BeeperStore) -> (Vec<&'a Chat>, Vec<&'a Chat>) {
        let (mut pinned, rest): (Vec<&Chat>, Vec<&Chat>) = self
            .visible_chats(st)
            .into_iter()
            .partition(|c| self.active_tab == InboxTab::AllChats && c.flags & CHAT_PINNED != 0);
        pinned.sort_by_key(|c| (st.pin_rank.get(&c.id).copied().unwrap_or(usize::MAX), c.last_activity.clone()));
        match st.settings.on("regularPinned") {
            true => (vec![], pinned.into_iter().chain(rest).collect()),
            false => (pinned, rest),
        }
    }

    pub fn visible_chats<'a>(&self, st: &'a BeeperStore) -> Vec<&'a Chat> {
        self.chats_with(st, self.filters)
    }

    fn chats_with<'a>(&self, st: &'a BeeperStore, filters: u8) -> Vec<&'a Chat> {
        let mut list: Vec<&Chat> = st
            .chats
            .iter()
            .filter(|c| {
                let on = |i: usize| filters & 1 << i != 0;
                let shown = |i: usize| on(i) || !filter_hit(i, c, st);
                c.merged_into_chat_id.is_none()
                    && self.selected_account_filter.as_ref().is_none_or(|a| in_rail_entry(st, a, c))
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
            .collect();
        let remind_at = |c: &Chat| c.reminder.as_ref().map_or(0, |r| r.remind_at_ms);
        match (self.active_tab, st.settings.reminders_order) {
            (InboxTab::Reminders, 0) => list.sort_by_key(|c| remind_at(c)),
            (InboxTab::Reminders, 1) => list.sort_by_key(|c| std::cmp::Reverse(remind_at(c))),
            (InboxTab::Reminders, 2) => list.sort_by(|a, b| a.last_activity.cmp(&b.last_activity)),
            _ => list.sort_by(|a, b| b.last_activity.cmp(&a.last_activity)),
        }
        list
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        self.creating_chat = None;
        self.join_matrix_mode = false;
        let mut st = self.store.write().unwrap();
        self.leave_chat(&mut st, Some(chat_id));
        let incognito = st.settings.on("incognito");
        let idx = st.resolve_chat_idx(chat_id).expect("selected chat exists");
        self.selected_chat_id = Some(chat_id.to_string());
        if let Some(ev) = (!incognito).then(|| st.mark_chat_read(chat_id, false)).flatten() {
            let cid = st.chats[idx].id.clone();
            st.queue(&cid, Outgoing::Read(ev));
        }
        let chat = st.chats[idx].clone();
        self.composer = composer_with(chat.draft.as_ref().and_then(|d| d.text.as_deref()).unwrap_or_default());
        self.nickname_edit_buffer = chat
            .participants
            .items
            .iter()
            .find(|u| u.is_self != Some(true))
            .and_then(|u| u.custom_nickname.clone().or_else(|| u.full_name.clone()))
            .unwrap_or_default();
        self.reply_to_message_id = None;
        self.editing_message_id = None;
        self.selected_msg = None;
        self.playing_video = None;
        drop(st);
        self.refresh_theme_and_tray();
    }

    fn selected_message(&self) -> Option<Message> {
        let st = self.store.read().unwrap();
        self.selected_msg.as_ref().map(|id| st.messages[st.msg_idx_by_id[id]].clone())
    }

    /// The arrow keys select messages when no panel is open and the composer is empty.
    fn quick_react(&self, mid: String) -> UiMessage {
        let chat = self.selected_chat_id.clone().expect("an open chat");
        let st = self.store.read().unwrap();
        UiMessage::ToggleReaction(chat, mid, st.settings.choice("quickReactionEmoji").into())
    }

    /// Desktop's emoji autocomplete for the ":" word at the end of the composer.
    fn emoji_suggestions(&self, st: &BeeperStore) -> Vec<(String, &'static str)> {
        let text = &self.composer.text();
        let on = st.settings.on("emojiAutocomplete") && self.suggest_hidden.as_ref() != Some(text);
        let query = text.rfind(':').filter(|_| on).map(|at| &text[at + 1..]);
        match query.filter(|q| !q.contains(char::is_whitespace)) {
            Some(q) => emoji::suggestions(':', q, st.skin_tone),
            None => Vec::new(),
        }
    }

    /// The emoji autocomplete list, with its left edge at the ":" in the composer and its bottom at the text top.
    fn view_suggestions(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        let picks = self.emoji_suggestions(st);
        let at = self.suggest.min(picks.len().checked_sub(1)?);
        let rows: Vec<String> = picks.iter().map(|(c, name)| format!("{c} {name}")).collect();
        let width = rows.iter().map(|r| text_width(r, 13.0, Weight::Normal)).fold(0.0, f32::max) + 12.0;
        let items = rows.into_iter().enumerate().map(|(i, r)| {
            let (bg, fg) = if i == at { (p.accent, Color::WHITE) } else { (CLEAR, p.text) };
            let b = button(txt(r, 13.0).wrapping(iced::widget::text::Wrapping::None)).padding([4, 6]).width(width);
            b.style(btn_style([bg; 3], [fg; 3], 4.0)).on_press(UiMessage::PickEmoji(i)).into()
        });
        let list =
            container(Column::with_children(items).padding([4, 4])).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let (pri, sec) = composer_buttons(st);
        let unveiled = ATTACH_BUTTONS.iter().filter(|&&x| Some(x) != pri && Some(x) != sec).count() as f32;
        let plus = if self.show_attach_menu { 36.0 + 6.0 + 2.0 + unveiled * 30.0 - 2.0 } else { 36.0 };
        let text = self.composer.text();
        let line = &text[text.rfind('\n').map_or(0, |i| i + 1)..text.rfind(':').expect("an emoji query")];
        let x = 8.0 + plus + 8.0 + 14.0 + 2.0 + text_width(line, 13.0, Weight::Normal);
        let rail = if st.settings.on("showSidebar") { 72.0 } else { 0.0 };
        let x = x.min(self.main_size.width / self.zoom - rail - self.sidebar_width - width - 40.0);
        Some(container(list).align_bottom(Fill).padding(Padding::ZERO.left(x).bottom(34.0)).into())
    }

    /// The newest message you can edit in the open chat.
    fn last_own_message(&self) -> Option<Message> {
        let st = self.store.read().unwrap();
        let all = &st.chat_msg_indices[self.selected_chat_id.as_deref().expect("an open chat")];
        let own = all
            .iter()
            .map(|&i| &st.messages[i])
            .rfind(|m| m.is_sender == Some(true) && m.is_deleted != Some(true) && m.attachments.is_empty());
        own.cloned()
    }

    fn can_select_messages(&self) -> bool {
        self.selected_chat_id.is_some()
            && self.creating_chat.is_none()
            && !self.show_command_palette
            && self.forward.is_none()
            && self.emoji.is_none()
            && (self.selected_msg.is_some() || self.composer.is_empty())
    }

    /// Leave the selected chat for `next`: save its draft, mark it read, and drop its elapsed reminder.
    fn leave_chat(&self, st: &mut BeeperStore, next: Option<&str>) {
        let Some(prev) = self.selected_chat_id.as_deref().filter(|&p| Some(p) != next) else { return };
        if self.editing_message_id.is_none() {
            let draft = self.composer.text().trim().to_string();
            let draft = (!draft.is_empty()).then(|| ChatDraft {
                text: Some(draft),
                reply_to_message_id: self.reply_to_message_id.clone(),
                attachments: Vec::new(),
            });
            st.patch_chat(prev, true, None, None).expect("clear draft");
            st.patch_chat(prev, false, draft, None).expect("save draft");
        }
        let idx = st.resolve_chat_idx(prev).expect("selected chat exists");
        if !st.settings.on("incognito") && st.chats[idx].is_marked_unread != Some(true) {
            st.mark_chat_read(prev, true);
        }
        if self.expired(&st.chats[idx]) {
            st.set_reminder(idx, None);
        }
    }

    /// Desktop's reaction to new composer text: emoticons, the typing notice, and link previews.
    fn composer_changed(&mut self, old: String) -> Task<UiMessage> {
        self.selected_msg = None;
        self.link_dialog = None;
        self.suggest = 0;
        let st = self.store.read().unwrap();
        let new = self.composer.text();
        let notice = st.matrix.clone().filter(|_| st.settings.on("typingIndicators"));
        let task = notice.zip(self.selected_chat_id.clone()).map(|(client, chat)| {
            Task::perform(beeper_core::matrix::typing(client, chat, !new.is_empty()), UiMessage::Pushed)
        });
        let replaced = (!st.settings.on("keepEmoticons")).then(|| emoji::replace_emoticon(&old, new.clone()));
        drop(st);
        if let Some(r) = replaced.filter(|r| *r != new) {
            self.composer = composer_with(&r);
        }
        Task::batch([task.unwrap_or_else(Task::none), self.fetch_link_previews()])
    }

    pub fn submit_composer(&mut self) {
        let chat_id = self.selected_chat_id.clone().expect("a chat is selected");
        let text = self.composer.text().trim().to_string();
        if text.is_empty() && self.pasted.is_none() {
            return;
        }
        let mut st = self.store.write().unwrap();
        let out = if let Some(target) = self.editing_message_id.take() {
            st.edit_message(&chat_id, &target, text.clone()).expect("edit message");
            Outgoing::Edit { target, text }
        } else if let Some(PendingMedia { bytes, name, mime, is_gif, size, frames }) = self.pasted.take() {
            let key = format!("pasted:{}", st.next_id("img"));
            self.media.insert(key.clone(), Some(Ok(frames)));
            let att = Attachment {
                attachment_type: "img".into(),
                id: None,
                duration: None,
                file_name: Some(name.clone()),
                file_size: Some(bytes.len() as u64),
                is_gif: Some(is_gif),
                is_sticker: None,
                is_voice_note: None,
                mime_type: Some(mime.clone()),
                poster_img: None,
                size: Some(AttachmentSize { width: Some(size.0), height: Some(size.1) }),
                src_url: Some(key),
                transcript: None,
            };
            let reply_to = self.reply_to_message_id.take();
            let caption = (!text.is_empty()).then_some(text);
            let (_, txn, _) =
                st.send_message(&chat_id, caption.clone(), reply_to.clone(), Some(att)).expect("send image");
            st.mark_chat_read(&chat_id, true);
            Outgoing::Image { txn, bytes, name, mime, size, caption, reply_to }
        } else {
            let reply_to = self.reply_to_message_id.take();
            let urls = text_urls(&text);
            let preview = self.composer_links.iter().find_map(|(u, s)| match s {
                LinkState::Loaded(v, img) if urls.contains(&u.as_str()) => {
                    Some((v.clone(), img.as_ref().map(|(b, _)| b.clone())))
                }
                _ => None,
            });
            let (id, txn, _) =
                st.send_message(&chat_id, Some(text.clone()), reply_to.clone(), None).expect("send message");
            if let Some(link) = preview.as_ref().and_then(|(v, _)| beeper_core::matrix::link(v)) {
                let i = st.msg_idx_by_id[&id];
                st.update_message(i, |m| m.links = vec![link]);
            }
            st.mark_chat_read(&chat_id, true);
            Outgoing::Send { txn, text, reply_to, preview }
        };
        st.queue(&chat_id, out);
        self.composer = composer_with("");
        self.composer_links.clear();
        drop(st);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `fetchLinkPreviews`: forget the links that left the composer, and load the new ones unless "Disable
    /// link previews" is on or the network cannot send previews.
    fn fetch_link_previews(&mut self) -> Task<UiMessage> {
        use beeper_core::models::NetworkKind::*;
        let urls: Vec<String> = text_urls(&self.composer.text()).into_iter().map(str::to_string).collect();
        self.composer_links.retain(|(u, _)| urls.contains(u));
        let st = self.store.read().unwrap();
        let chat = self.selected_chat_id.as_ref().and_then(|c| st.resolve_chat_idx(c)).map(|i| &st.chats[i]);
        let kind = chat.map(|c| account_network(&c.account_id).kind);
        let can =
            matches!(kind, Some(Beeper | WhatsApp | TwitterX | Telegram | Slack | GoogleMessages | Signal | Line));
        if !can || st.settings.on("noOutgoingLinkPreviews") {
            return Task::none();
        }
        drop(st);
        let mut tasks = Vec::new();
        for url in urls {
            if self.composer_links.iter().all(|(u, _)| *u != url) {
                self.composer_links.push((url.clone(), LinkState::Loading));
                let load = beeper_core::cloud::link_preview(url.clone());
                tasks.push(Task::perform(load, move |r| UiMessage::LinkPreviewLoaded(url, r)));
            }
        }
        Task::batch(tasks)
    }

    fn push_outbox(&mut self) -> Task<UiMessage> {
        match self.store.write().unwrap().start_flush() {
            true => Task::perform(beeper_core::matrix::flush(self.store.clone()), UiMessage::Pushed),
            false => Task::none(),
        }
    }

    fn run_hotkey(&mut self, name: &str) -> Task<UiMessage> {
        let chat = |f: ChatFlag| self.selected_chat_id.clone().map(|c| UiMessage::ToggleFlag(c, f));
        let msg = match name {
            "Search" => Some(UiMessage::ToggleCommandPalette),
            "Search: Room" => self.selected_chat_id.clone().map(UiMessage::SearchRoom),
            "Settings" => Some(UiMessage::OpenSettings(self.active_settings_tab)),
            "Toggle Chat Info" => Some(UiMessage::ToggleRightPanel),
            "Chat: Toggle Mute" => chat(ChatFlag::Muted),
            "Chat: Toggle Pin" => chat(ChatFlag::Pinned),
            "Chat: Toggle Archive" => chat(ChatFlag::Archived),
            "Chat: Toggle Mark As Read" => self.selected_chat_id.clone().map(UiMessage::ToggleRead),
            "Zoom In" => Some(UiMessage::Zoom(1)),
            "Zoom Out" => Some(UiMessage::Zoom(-1)),
            "Reset Zoom" => Some(UiMessage::Zoom(0)),
            "Quit App" => return iced::exit(),
            "Report a Problem" => Some(UiMessage::OpenUrl(ISSUES_URL.into())),
            "Filter: Unread" => {
                self.filters ^= 1;
                None
            }
            "Select Next Chat" => {
                self.step_chat(1);
                None
            }
            "Select Previous Chat" => {
                self.step_chat(-1);
                None
            }
            _ => unreachable!("{name} has no action"),
        };
        msg.map_or_else(Task::none, |m| self.update(m))
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
        let task = self.handle(msg);
        if !std::mem::take(&mut self.tray_sent.1) || !cfg!(target_os = "linux") {
            return task;
        }
        let now = (self.badge_count(), !self.hidden);
        if self.tray_sent.0.replace(now) == Some(now) {
            return task;
        }
        let sync = Task::future(tray::update(now.0, now.1))
            .map(Result::err)
            .and_then(|e| Task::done(UiMessage::TrayFailed(e)));
        Task::batch([task, sync])
    }

    /// Desktop's `updateAppBadgeCount`: the unread chats in all chats or in the chat list, by "Badge count".
    fn badge_count(&self) -> u32 {
        let st = self.store.read().unwrap();
        let counts = |c: &Chat| {
            c.flags & (CHAT_HIDDEN | CHAT_ARCHIVED | CHAT_REQUEST) == 0
                && c.flags & CHAT_UNREAD != 0
                && (c.unread_mentions > 0 || c.flags & CHAT_MUTED == 0)
        };
        let n = match st.settings.pref("badgeCount") {
            0 => 0,
            1 => st.chats.iter().filter(|c| counts(c)).count(),
            _ => self.visible_chats(&st).into_iter().filter(|c| counts(c)).count(),
        };
        n as u32
    }

    /// Desktop's `hideToTray` and `showWindow`.
    fn set_hidden(&mut self, hidden: bool) -> Task<UiMessage> {
        let Some(id) = self.main_window else { return Task::none() };
        self.hidden = hidden;
        self.tray_sent.1 = true;
        match hidden {
            true => window::set_mode(id, window::Mode::Hidden),
            false => window::set_mode(id, window::Mode::Windowed).chain(window::gain_focus(id)),
        }
    }

    fn handle(&mut self, msg: UiMessage) -> Task<UiMessage> {
        match msg {
            UiMessage::SelectTab(tab) => {
                self.active_tab = tab;
                self.selected_account_filter = None;
                self.show_filter_menu = false;
                self.creating_chat = None;
            }
            UiMessage::SelectAccount(acc) => {
                self.active_tab = InboxTab::AllChats;
                self.selected_account_filter = acc;
                self.creating_chat = None;
            }
            UiMessage::SelectChat(id) => {
                self.select_chat(&id);
                return self.close_notifications();
            }
            UiMessage::ClickChat(id) => {
                let last = self.last_click.take();
                if last.is_none_or(|(c, at)| c != id || at.elapsed() > std::time::Duration::from_millis(500)) {
                    self.last_click = Some((id.clone(), std::time::Instant::now()));
                    return self.update(UiMessage::SelectChat(id));
                }
                let msg = match self.store.read().unwrap().settings.pref("chatDoubleClick") {
                    1 => UiMessage::ToggleFlag(id, ChatFlag::Archived),
                    2 => UiMessage::ToggleRead(id),
                    3 => UiMessage::ToggleFlag(id, ChatFlag::Muted),
                    4 => UiMessage::ToggleFlag(id, ChatFlag::LowPriority),
                    _ => return Task::none(),
                };
                return self.update(msg);
            }
            UiMessage::ComposerEdit(action) if !action.is_edit() => self.composer.perform(action),
            UiMessage::ComposerEdit(action) => {
                let old = self.composer.text();
                self.composer.perform(action);
                return self.composer_changed(old);
            }
            UiMessage::ComposerChanged(v) => {
                let old = self.composer.text();
                self.composer = composer_with(&v);
                return self.composer_changed(old);
            }
            UiMessage::Format(i) => {
                let old = self.composer.text();
                let caret = format::offset(&old, self.composer.cursor().position);
                let r = format::selection(&self.composer).unwrap_or(caret..caret);
                if i == format::LINK && !format::active(i, &old, &r) {
                    self.link_dialog = Some((String::new(), old[r].to_owned()));
                    return operation::focus("link-url");
                }
                let (text, r) = format::toggle(i, &old, r);
                self.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            UiMessage::LinkDialog(d) => {
                let closed = d.is_none();
                self.link_dialog = d;
                if closed {
                    return operation::focus("composer-input");
                }
            }
            UiMessage::SaveLink => {
                let (url, label) = self.link_dialog.take().expect("the link dialog is open");
                let old = self.composer.text();
                let caret = format::offset(&old, self.composer.cursor().position);
                let r = format::selection(&self.composer).unwrap_or(caret..caret);
                let (text, r) = format::link(&old, r, url.trim(), label.trim());
                self.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            UiMessage::SubmitComposer if !self.emoji_suggestions(&self.store.read().unwrap()).is_empty() => {
                return self.update(UiMessage::PickEmoji(self.suggest));
            }
            UiMessage::SubmitComposer => self.submit_composer(),
            UiMessage::PickEmoji(i) => {
                let picks = self.emoji_suggestions(&self.store.read().unwrap());
                let mut text = self.composer.text();
                let at = text.rfind(':').expect("an emoji query");
                text.replace_range(at.., &format!("{} ", picks[i.min(picks.len() - 1)].0));
                self.composer = composer_with(&text);
                return operation::focus("composer-input");
            }
            UiMessage::CancelComposeMode => {
                self.reply_to_message_id = None;
                self.editing_message_id = None;
                self.composer = composer_with("");
            }
            UiMessage::StartReply(mid) => {
                self.editing_message_id = None;
                self.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            UiMessage::StartEdit(mid, old_txt) => {
                self.reply_to_message_id = None;
                self.editing_message_id = Some(mid);
                self.composer = composer_with(&old_txt);
                return operation::focus("composer-input");
            }
            UiMessage::DeleteMessage(cid, mid) => {
                self.confirm = None;
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
                let mut recent = None;
                let out = match mine {
                    Some(target) => {
                        st.remove_reaction(&cid, &mid, &key).expect("remove reaction");
                        Outgoing::Redact { target }
                    }
                    None => {
                        let (_, txn) = st.add_reaction(&cid, &mid, &key, None).expect("add reaction");
                        st.add_recent_emoji(&key);
                        recent = st.matrix.clone().map(|c| (c, serde_json::json!({ "recent_emoji": st.recent_emoji })));
                        Outgoing::React { txn, target: mid, key }
                    }
                };
                st.queue(&cid, out);
                if let Some((c, content)) = recent {
                    let write = beeper_core::matrix::set_global(c, "io.element.recent_emoji", content);
                    return Task::perform(write, UiMessage::Pushed);
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
            UiMessage::SetReminder(cid, at) => {
                {
                    let mut st = self.store.write().unwrap();
                    let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                    self.remind(&mut st, idx, at);
                }
                self.chat_menu = None;
                self.refresh_theme_and_tray();
            }
            UiMessage::ChatSub(sub) => self.chat_sub = sub,
            UiMessage::CheckReminders => {
                let now = now_unix_millis();
                let mut st = self.store.write().unwrap();
                for i in 0..st.chats.len() {
                    let c = &st.chats[i];
                    let Some(at) = c.reminder.as_ref().map(|r| r.remind_at_ms).filter(|&at| at <= now) else {
                        continue;
                    };
                    if self.reminded.insert(c.id.clone(), at) == Some(at) {
                        continue;
                    }
                    let flags = [(ChatFlag::LowPriority, c.is_low_priority), (ChatFlag::Archived, c.is_archived)];
                    let ts = rfc3339_from_millis(at);
                    st.update_chat(i, |c| c.last_activity = c.last_activity.take().max(Some(ts)));
                    for (flag, _) in flags.into_iter().filter(|(_, on)| *on == Some(true)) {
                        st.set_chat_flag(i, flag, Some(false));
                    }
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::ArchiveStale => {
                {
                    let mut st = self.store.write().unwrap();
                    for idx in st.stale_inbox_chats() {
                        assert!(st.set_chat_flag(idx, ChatFlag::Archived, Some(true)));
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::ToggleFlag(cid, flag) => {
                let mut st = self.store.write().unwrap();
                let at = self.list_ids(&st).iter().position(|c| *c == cid);
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                let moved = match flag {
                    ChatFlag::Archived => {
                        let on = !has_flag(&st.chats[idx], flag);
                        self.set_archived(&mut st, idx, on)
                    }
                    _ => st.set_chat_flag(idx, flag, None),
                };
                let list = self.list_ids(&st);
                drop(st);
                if !moved {
                    self.status_banner = "Pinned chats can't be archived".into();
                }
                self.chat_menu = None;
                self.refresh_theme_and_tray();
                let selected = self.selected_chat_id.as_deref() == Some(cid.as_str());
                if let Some(at) = at.filter(|_| flag == ChatFlag::Archived && moved && selected) {
                    return self.after_action("afterArchive", &list, at, true);
                }
            }
            UiMessage::ToggleRead(cid) => {
                let mut st = self.store.write().unwrap();
                let list = self.list_ids(&st);
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                let c = &st.chats[idx];
                if c.unread_count > 0 || c.is_marked_unread == Some(true) {
                    self.mark_read(&mut st, idx);
                } else {
                    st.set_chat_flag(idx, ChatFlag::MarkedUnread, Some(true));
                }
                drop(st);
                self.chat_menu = None;
                self.refresh_theme_and_tray();
                let at = list.iter().position(|c| *c == cid);
                if let Some(at) = at.filter(|_| self.selected_chat_id.as_deref() == Some(cid.as_str())) {
                    return self.after_action("afterMarkRead", &list, at, false);
                }
            }
            UiMessage::CopyChatLink(cid) => {
                self.chat_menu = None;
                let st = self.store.read().unwrap();
                let c = &st.chats[st.resolve_chat_idx(&cid).expect("chat exists")];
                return iced::clipboard::write(format!(
                    "beeper://select-thread/{}/{}",
                    c.account_id,
                    encode_uri_component(&c.id)
                ));
            }
            UiMessage::OpenChatMenu(cid) => {
                self.chat_menu = Some((cid, self.cursor));
                self.chat_sub = None;
            }
            UiMessage::Cursor(at) => self.cursor = at,
            UiMessage::OpenMessageMenu(mid) => self.message_menu = Some((mid, self.cursor)),
            UiMessage::ClickMessage(mid) => {
                self.selected_msg = (self.selected_msg.as_ref() != Some(&mid)).then_some(mid)
            }
            UiMessage::DoubleClickMessage(mid) => {
                let action = self.store.read().unwrap().settings.pref("messageDoubleClick");
                return match action {
                    0 => Task::none(),
                    1 => self.update(UiMessage::React(mid)),
                    3 => self.update(UiMessage::Forward(mid)),
                    2 => {
                        self.selected_msg = Some(mid.clone());
                        self.update(UiMessage::StartReply(mid))
                    }
                    4 => self.update(UiMessage::OpenMessageMenu(mid)),
                    5 => self.update(self.quick_react(mid)),
                    n => unreachable!("messageDoubleClick option {n}"),
                };
            }
            UiMessage::StepMessage(down) => {
                let st = self.store.read().unwrap();
                let all = &st.chat_msg_indices[self.selected_chat_id.as_deref().expect("an open chat")];
                let ids: Vec<&str> = all
                    .iter()
                    .map(|&i| &st.messages[i])
                    .filter(|m| m.is_deleted != Some(true))
                    .map(|m| m.id.as_str())
                    .collect();
                let at = self.selected_msg.as_deref().map(|s| ids.iter().position(|&i| i == s).expect("selected"));
                let next = match (at, down) {
                    (None, false) => ids.len().checked_sub(1),
                    (None, true) => None,
                    (Some(i), false) => Some(i.saturating_sub(1)),
                    (Some(i), true) => (i + 1 < ids.len()).then_some(i + 1),
                };
                self.selected_msg = next.map(|i| ids[i].to_string());
                drop(st);
                return match &self.selected_msg {
                    Some(mid) => iced::advanced::widget::operate(Locate(format!("msg-{mid}").into(), None, None, true))
                        .then(|y| operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y })),
                    None => operation::focus("composer-input"),
                };
            }
            UiMessage::React(mid) => return self.open_emoji(Some((mid, self.cursor))),
            UiMessage::MessageMenu(msg) => {
                self.message_menu = None;
                return self.update(*msg);
            }
            UiMessage::CopyText(s) => return iced::clipboard::write(s),
            UiMessage::CopyImage(mid, ai) => {
                let st = self.store.read().unwrap();
                let a = &st.messages[st.msg_idx_by_id[&mid]].attachments[ai];
                let url = a.src_url.clone().expect("an image has a source");
                return Task::perform(beeper_core::matrix::media(st.matrix.clone(), url), UiMessage::ImageFetched);
            }
            UiMessage::ImageFetched(r) => {
                let copy = |bytes: Vec<u8>, c: &mut Option<arboard::Clipboard>| -> Result<(), String> {
                    let px = ::image::load_from_memory(&bytes).map_err(|e| e.to_string())?.into_rgba8();
                    let (width, height) = (px.width() as usize, px.height() as usize);
                    let img = arboard::ImageData { width, height, bytes: px.into_raw().into() };
                    let c = match c {
                        Some(c) => c,
                        None => c.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                    };
                    c.set_image(img).map_err(|e| e.to_string())
                };
                if let Err(e) = r.and_then(|b| copy(b, &mut self.image_clipboard)) {
                    eprintln!("copy image failed: {e}");
                    self.status_banner = "Couldn't copy the image".into();
                }
            }
            UiMessage::OpenCarousel(mid, ai, in_media) => {
                self.carousel = Some(Carousel { mid, ai, turns: 0, in_media });
                return self.load_media();
            }
            UiMessage::CarouselStep(next) => {
                let (items, at) = self.carousel_items(&self.store.read().unwrap());
                let to = if next { at + 1 } else { at.checked_sub(1).expect("a previous image") };
                let c = self.carousel.as_mut().expect("the carousel is open");
                (c.mid, c.ai) = items.into_iter().nth(to).expect("a next image");
                return self.load_media();
            }
            UiMessage::RotateCarousel => {
                let c = self.carousel.as_mut().expect("the carousel is open");
                c.turns = (c.turns + 1) % 4;
            }
            UiMessage::CloseCarousel => self.carousel = None,
            UiMessage::OpenMedia => {
                if let Some(w) = &self.media_window {
                    return window::gain_focus(w.id);
                }
                let size = Size::new(1000.0, 720.0);
                let (id, open) = window::open(Self::window_settings(size));
                self.media_window = Some(MediaWindow {
                    id,
                    tab: 0,
                    size,
                    shown: MEDIA_PAGE,
                    sent_by: 0,
                    oldest: false,
                    search: None,
                    menu: false,
                    cursor: Point::ORIGIN,
                    item_menu: None,
                    select: false,
                    picked: Vec::new(),
                    deleting: None,
                });
                return Task::batch([open.map(UiMessage::WindowOpened), self.load_media()]);
            }
            UiMessage::MediaTab(tab) => {
                self.media_mut().tab = tab;
                return self.media_reload();
            }
            UiMessage::MediaSentBy(n) => {
                let w = self.media_mut();
                (w.sent_by, w.menu) = (n, false);
                return self.media_reload();
            }
            UiMessage::MediaOldest(on) => {
                let w = self.media_mut();
                (w.oldest, w.menu) = (on, false);
                return self.media_reload();
            }
            UiMessage::MediaSearch(q) => {
                let w = self.media_mut();
                let open = q.is_some() && w.search.is_none();
                w.search = q;
                let reload = self.media_reload();
                return if open { Task::batch([reload, operation::focus("media-search")]) } else { reload };
            }
            UiMessage::MediaMenu(open) => self.media_mut().menu = open,
            UiMessage::MediaCursor(at) => self.media_mut().cursor = at,
            UiMessage::Modifiers(mods) => self.mods = mods,
            UiMessage::MediaSelect(on) => {
                let w = self.media_mut();
                w.select = on;
                w.picked.clear();
            }
            UiMessage::MediaPick(mid, k) => {
                let w = self.media_mut();
                w.select = true;
                match w.picked.iter().position(|(m, i)| *m == mid && *i == k) {
                    Some(at) => _ = w.picked.remove(at),
                    None => w.picked.push((mid, k)),
                }
            }
            UiMessage::MediaPickAll => {
                let st = self.store.read().unwrap();
                let w = self.media_window.as_ref().expect("the Media window is open");
                let all = media_items(&st, w).into_iter().take(w.shown).map(|(i, k)| (st.messages[i].id.clone(), k));
                let all = all.collect();
                drop(st);
                self.media_mut().picked = all;
            }
            UiMessage::MediaSave => {
                let w = self.media_window.as_ref().expect("the Media window is open");
                let files = if w.tab == 1 { Vec::new() } else { w.picked.clone() };
                return Task::batch(files.into_iter().map(|(mid, k)| self.update(UiMessage::Download(mid, Some(k)))));
            }
            UiMessage::MediaAskDelete(items) => {
                let st = self.store.read().unwrap();
                let mut pairs: Vec<(String, String)> = Vec::new();
                for (mid, _) in &items {
                    let m = &st.messages[st.msg_idx_by_id[mid]];
                    if deletable(m) && !pairs.iter().any(|(_, x)| x == mid) {
                        pairs.push((m.chat_id.clone(), mid.clone()));
                    }
                }
                let skipped = items.iter().filter(|(mid, _)| !deletable(&st.messages[st.msg_idx_by_id[mid]])).count();
                drop(st);
                match pairs.is_empty() {
                    true => self.status_banner = "These messages can\u{2019}t be deleted".into(),
                    false => self.media_mut().deleting = Some((pairs, skipped)),
                }
            }
            UiMessage::MediaDelete(go) => {
                let (pairs, skipped) = self.media_mut().deleting.take().expect("the delete dialog is open");
                if go {
                    let mut st = self.store.write().unwrap();
                    for (cid, mid) in &pairs {
                        st.delete_message(cid, mid).expect("delete message");
                        st.queue(cid, Outgoing::Redact { target: mid.clone() });
                    }
                    drop(st);
                    self.media_mut().picked.retain(|(m, _)| pairs.iter().all(|(_, x)| x != m));
                    if skipped > 0 {
                        let s = if skipped == 1 { "" } else { "s" };
                        self.status_banner = format!("Skipped {skipped} item{s} that can\u{2019}t be deleted");
                    }
                }
            }
            UiMessage::MediaItemMenu(item) => {
                let w = self.media_mut();
                w.item_menu = item.map(|(mid, k)| (mid, k, w.cursor));
            }
            UiMessage::MediaItemAct(msg) => {
                self.media_mut().item_menu = None;
                return self.update(*msg);
            }
            UiMessage::CopyLink(url) => {
                self.status_banner = "Copied to clipboard!".into();
                return iced::clipboard::write(url);
            }
            UiMessage::MediaMore => {
                self.media_mut().shown += MEDIA_PAGE;
                return self.load_media();
            }
            UiMessage::GoToMessage(chat, mid) => {
                let main = window::gain_focus(self.main_window.expect("the main window is open"));
                return Task::batch([self.update(UiMessage::JumpTo(chat, mid)), main]);
            }
            UiMessage::WindowKey(id, key, mods) => {
                if self.media_window.as_ref().is_none_or(|w| w.id != id) {
                    return self.update(UiMessage::KeyPressed(key, mods));
                }
                if self.carousel.as_ref().is_some_and(|c| c.in_media) {
                    return self.carousel_key(&key, mods);
                }
                if key == Key::Named(Named::Escape) {
                    let w = self.media_mut();
                    if w.item_menu.is_some() {
                        w.item_menu = None;
                    } else if w.menu {
                        w.menu = false;
                    } else if w.deleting.is_some() {
                        w.deleting = None;
                    } else if let Some(q) = w.search.as_ref().map(|q| (!q.is_empty()).then(String::new)) {
                        return self.update(UiMessage::MediaSearch(q));
                    } else if w.select {
                        (w.select, w.picked) = (false, Vec::new());
                    } else {
                        return window::close(id);
                    }
                }
            }
            UiMessage::Forward(mid) => {
                self.forward = Some(Forward { mid, picked: Vec::new(), query: String::new() });
                return operation::focus("forward-input");
            }
            UiMessage::ForwardQuery(q) => self.forward.as_mut().expect("the forward dialog is open").query = q,
            UiMessage::ForwardSubmit => {
                let st = self.store.read().unwrap();
                let first = forward_chats(&st, self.forward.as_ref().expect("the forward dialog is open"));
                let first = first.first().map(|c| c.id.clone());
                drop(st);
                return first.map_or_else(Task::none, |c| self.update(UiMessage::ForwardPick(c)));
            }
            UiMessage::ForwardPick(cid) => {
                let f = self.forward.as_mut().expect("the forward dialog is open");
                f.picked.push(cid);
                f.query.clear();
                return operation::focus("forward-input");
            }
            UiMessage::ForwardSend => {
                let Forward { mid, picked, .. } = self.forward.take().expect("the forward dialog is open");
                assert!(!picked.is_empty(), "forward needs a chat");
                let mut st = self.store.write().unwrap();
                let m = st.messages[st.msg_idx_by_id[&mid]].clone();
                for cid in &picked {
                    let (_, txn, _) = st
                        .send_message(cid, m.text.clone(), None, m.attachments.first().cloned())
                        .expect("forward message");
                    let (from, event, text, html) = (m.chat_id.clone(), m.id.clone(), m.text.clone(), m.html.clone());
                    st.queue(cid, Outgoing::Forward { txn, from, event, text, html });
                }
                drop(st);
                if let [one] = picked.as_slice() {
                    self.select_chat(one);
                }
            }
            UiMessage::Download(mid, only) => {
                let st = self.store.read().unwrap();
                let m = &st.messages[st.msg_idx_by_id[&mid]];
                let files: Vec<(String, String)> = m
                    .attachments
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| only.is_none_or(|o| o == *i))
                    .map(|(_, a)| {
                        let name = a.file_name.clone().expect("an attachment has a file name");
                        (a.src_url.clone().expect("an attachment has a source"), name)
                    })
                    .collect();
                let client = st.matrix.clone();
                self.status_banner = format!("Downloading {}", files[0].1);
                let dir = beeper_core::platform::resolve_home_dir(&std::env::vars().collect()).join("Downloads");
                return Task::perform(
                    async move {
                        let mut last = String::new();
                        for (url, name) in files {
                            let name: String =
                                name.chars().filter(|c| !r#"/\?<>:*|""#.contains(*c) && !c.is_control()).collect();
                            if name.trim_matches('.').is_empty() {
                                return Err(format!("bad file name {name:?}"));
                            }
                            let bytes = beeper_core::matrix::media(client.clone(), url).await?;
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            let path = unused_path(&dir, &name);
                            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                            last = path.file_name().expect("a file path").to_string_lossy().into_owned();
                        }
                        Ok(last)
                    },
                    UiMessage::Downloaded,
                );
            }
            UiMessage::Downloaded(r) => {
                self.status_banner = match r {
                    Ok(name) => format!("Downloaded {name}"),
                    Err(e) => format!("Download failed: {e}"),
                }
            }
            UiMessage::AskDeleteMessage(mid) => {
                (self.confirm, self.delete_target) = (Some(ASK_DELETE_MESSAGE), Some(mid))
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
            UiMessage::ToggleCommandPalette => {
                if !self.show_command_palette {
                    self.search_filters = SearchFilters::default();
                }
                self.show_command_palette ^= self.search_room.take().is_none() || !self.show_command_palette;
                self.cmd_bar_commands_mode = false;
                self.search_expanded = false;
                if self.show_command_palette {
                    return operation::focus("cmd-palette-input");
                }
            }
            UiMessage::SearchRoom(cid) => {
                (self.show_command_palette, self.cmd_bar_commands_mode) = (true, false);
                (self.search_room, self.chat_menu) = (Some(cid), None);
                self.search_filters.accounts.clear();
                return operation::focus("cmd-palette-input");
            }
            UiMessage::ToggleCommandMode => {
                self.cmd_bar_commands_mode = !self.cmd_bar_commands_mode;
                return operation::focus("cmd-palette-input");
            }
            UiMessage::ToggleSearchExpanded => self.search_expanded = !self.search_expanded,
            UiMessage::CommandQueryChanged(q) => self.command_palette_query = q,
            UiMessage::JumpTo(cid, mid) => {
                (self.show_command_palette, self.command_palette_query) = (false, String::new());
                self.select_chat(&cid);
                let msg = format!("msg-{mid}");
                self.flash = Some((mid, std::time::Instant::now()));
                return iced::advanced::widget::operate(Locate(msg.into(), None, None, false))
                    .then(|y| operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y }));
            }
            UiMessage::ExecuteCommandChat(cid) => {
                self.show_command_palette = false;
                self.command_palette_query.clear();
                self.select_chat(&cid);
            }
            UiMessage::NewChat => {
                let id = self.new_chats.iter().max().map_or(0, |n| n + 1);
                self.new_chats.insert(0, id);
                return self.update(UiMessage::SelectNewChat(id));
            }
            UiMessage::SelectNewChat(id) => {
                self.creating_chat = Some(id);
                self.show_command_palette = false;
                self.new_chat_query.clear();
                self.new_chat_expanded = false;
                self.join_matrix_mode = false;
                return operation::focus("new-chat-input");
            }
            UiMessage::DeleteNewChat(id) => {
                let i = self.new_chats.iter().position(|&n| n == id).expect("new chat exists");
                self.new_chats.remove(i);
                self.chat_menu = None;
                if self.creating_chat != Some(id) {
                    return Task::none();
                }
                self.creating_chat = None;
                let st = self.store.read().unwrap();
                let rest = self.pins_and_rows(&st).1.into_iter().map(|c| UiMessage::SelectChat(c.id.clone()));
                let items: Vec<_> = self.new_chats.iter().map(|&n| UiMessage::SelectNewChat(n)).chain(rest).collect();
                drop(st);
                if let Some(next) = items.get(i).or(i.checked_sub(1).and_then(|j| items.get(j))) {
                    return self.update(next.clone());
                }
            }
            UiMessage::NewChatQueryChanged(q) => self.new_chat_query = q,
            UiMessage::ToggleNewChatExpanded => self.new_chat_expanded = !self.new_chat_expanded,
            UiMessage::ToggleJoinMatrix => {
                self.join_matrix_mode = !self.join_matrix_mode;
                self.new_chat_query.clear();
                return operation::focus("new-chat-input");
            }
            UiMessage::SelectSettingsTab(t) => {
                self.active_settings_tab = t;
                return self.load_devices();
            }
            UiMessage::DevicesLoaded(r) => self.devices = Some(r),
            UiMessage::Ask(dialog) => (self.confirm, self.reminder_menu) = (dialog, None),
            UiMessage::AskDeleteChat(id) => {
                (self.confirm, self.chat_menu, self.delete_target) = (Some(ASK_DELETE_CHAT), None, Some(id));
            }
            UiMessage::BeginSelecting(id) => {
                (self.selection, self.chat_menu) = (vec![id.clone()], None);
                self.select_chat(&id);
            }
            UiMessage::ToggleSelected(id) => {
                match self.selection.iter().position(|c| *c == id) {
                    Some(i) => drop(self.selection.remove(i)),
                    None => self.selection.push(id.clone()),
                }
                self.select_chat(&id);
            }
            UiMessage::SelectionSet(flag, on) => {
                self.selection_menu = None;
                {
                    let mut st = self.store.write().unwrap();
                    for cid in std::mem::take(&mut self.selection) {
                        let idx = st.resolve_chat_idx(&cid).expect("selected chat exists");
                        if has_flag(&st.chats[idx], flag) == on {
                            continue;
                        }
                        let moved = match flag {
                            ChatFlag::MarkedUnread if !on => {
                                self.mark_read(&mut st, idx);
                                true
                            }
                            ChatFlag::Archived => self.set_archived(&mut st, idx, on),
                            _ => st.set_chat_flag(idx, flag, Some(on)),
                        };
                        if !moved {
                            self.status_banner = "Pinned chats can't be archived".into();
                        }
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectionRemind(at) => {
                self.selection_menu = None;
                {
                    let mut st = self.store.write().unwrap();
                    for cid in std::mem::take(&mut self.selection) {
                        let idx = st.resolve_chat_idx(&cid).expect("selected chat exists");
                        if at.is_some() || st.chats[idx].reminder.is_some() {
                            self.remind(&mut st, idx, at);
                        }
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectionMenu(menu, x) => self.selection_menu = Some((menu, x)),
            UiMessage::DeleteChat(id, everyone) => {
                self.confirm = None;
                self.store.write().unwrap().delete_chat(&id, everyone);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::Voice(msg) => return self.voice(msg),
            UiMessage::ReminderMenu => {
                self.reminder_menu = Some(self.cursor);
            }
            UiMessage::OrderReminders(i) => {
                self.reminder_menu = None;
                self.patch_settings(|s| s.reminders_order = i);
            }
            UiMessage::DismissAllReminders => {
                self.confirm = None;
                let mut st = self.store.write().unwrap();
                for i in 0..st.chats.len() {
                    if st.chats[i].reminder.is_some() {
                        st.set_reminder(i, None);
                    }
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::Logout => {
                self.confirm = None;
                return Task::perform(beeper_core::matrix::logout(self.store.clone()), UiMessage::LoggedOut);
            }
            UiMessage::LoggedOut(Ok(())) => {
                let exe = std::env::current_exe().expect("path of the running beeper-rs");
                std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn().expect("restart beeper-rs");
                std::process::exit(0);
            }
            UiMessage::LoggedOut(Err(e)) => self.status_banner = format!("Could not log out: {e}"),
            UiMessage::ToggleRightPanel => self.show_right_panel = !self.show_right_panel,
            UiMessage::ToggleMembers(id) => {
                if !self.open_members.remove(&id) {
                    self.open_members.insert(id);
                }
            }
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
            UiMessage::CloudAccounts(Ok(accounts)) => {
                let (token, uid, demo) = {
                    let mut st = self.store.write().unwrap();
                    let info = (st.beeper_state.access_token.clone(), st.beeper_state.user_id.clone(), st.config.demo);
                    st.set_cloud_accounts(accounts.clone());
                    info
                };
                if !demo {
                    let cloud = self.cloud.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || cloud.contacts(&token, &uid, &accounts))
                                .await
                                .expect("contacts worker thread")
                        },
                        UiMessage::ContactsLoaded,
                    );
                }
            }
            UiMessage::ContactsLoaded(contacts) => {
                if !contacts.is_empty() {
                    self.store.write().unwrap().set_contacts(contacts);
                }
            }
            UiMessage::CloudAccounts(Err(e)) => self.status_banner = format!("Could not load your chat accounts: {e}"),
            UiMessage::AvatarLoaded(url, Ok(h)) => {
                self.avatars.insert(url, Some(h));
            }
            UiMessage::AvatarLoaded(_, Err(_)) => {}
            UiMessage::MediaLoaded(url, r) => {
                self.media.insert(url, Some(r));
            }
            UiMessage::RetryMedia(url) => {
                self.media.remove(&url);
                return self.load_media();
            }
            UiMessage::ResolveMedia(account_id, event_id, raw) => {
                if self.resolving_media.get(&event_id) == Some(&true) {
                    return Task::none();
                }
                self.resolving_media.insert(event_id.clone(), true);
                let (homeserver, access_token, user_id, bridge_id, login_id) = {
                    let st = self.store.read().unwrap();
                    let found = st.bridge_accounts.iter().find(|a| a.account_id == account_id);
                    let (bridge_id, login_id) = found.map_or_else(
                        || {
                            let (b, l) = account_id.split_once('.').unwrap_or((&account_id, ""));
                            (b.to_string(), l.to_string())
                        },
                        |a| (a.bridge_id.clone(), a.login_id.clone()),
                    );
                    (
                        st.beeper_state.homeserver.clone(),
                        st.beeper_state.access_token.clone(),
                        st.beeper_state.user_id.clone(),
                        bridge_id,
                        login_id,
                    )
                };
                let eid = event_id.clone();
                return Task::perform(
                    beeper_core::matrix::resolve_media(
                        homeserver,
                        access_token,
                        user_id,
                        bridge_id,
                        login_id,
                        eid,
                        raw,
                    ),
                    move |r| UiMessage::MediaResolved(event_id, r),
                );
            }
            UiMessage::MediaResolved(event_id, r) => {
                if let Err(e) = r {
                    self.resolving_media.insert(event_id, false);
                    self.status_banner = format!("Could not load media: {e}");
                }
            }
            UiMessage::PlayVideo(url, w, h) => {
                if self.playing_video.as_ref().is_some_and(|(u, _)| u == &url) {
                    self.playing_video = None;
                    return Task::none();
                }
                self.playing_video = Some((url.clone(), None));
                let client = self.store.read().unwrap().matrix.clone();
                let u = url.clone();
                return Task::perform(
                    async move {
                        let bytes = beeper_core::matrix::media(client, u).await?;
                        tokio::task::spawn_blocking(move || decode_video(bytes, w, h)).await.expect("video decode task")
                    },
                    move |r| UiMessage::VideoDecoded(url, r),
                );
            }
            UiMessage::VideoDecoded(url, r) => {
                if self.playing_video.as_ref().is_some_and(|(u, _)| u == &url) {
                    self.playing_video = Some((url, Some(r.map(|f| (f, std::time::Instant::now())))));
                }
            }
            UiMessage::Pasted(r) => match r {
                Ok(img) => {
                    self.pasted = img.map(|(png, size)| PendingMedia {
                        name: "image.png".into(),
                        mime: "image/png".into(),
                        is_gif: false,
                        size,
                        frames: vec![(image::Handle::from_bytes(png.clone()), 0)],
                        bytes: png,
                    });
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::DiscardPaste => self.pasted = None,
            UiMessage::LinkPreviewLoaded(url, r) => {
                let entry = self.composer_links.iter_mut().find(|(u, s)| *u == url && matches!(s, LinkState::Loading));
                if let Some((_, s)) = entry {
                    *s = match r {
                        Ok(Some((mut v, bytes))) => {
                            let image = bytes.and_then(|b| {
                                let format = ::image::guess_format(&b).ok()?;
                                let reader = ::image::ImageReader::with_format(std::io::Cursor::new(&b), format);
                                let (w, h) = reader.into_dimensions().ok()?;
                                v["og:image:width"] = w.into();
                                v["og:image:height"] = h.into();
                                v["og:image:type"] = format.to_mime_type().into();
                                Some((b.clone(), image::Handle::from_bytes(b)))
                            });
                            LinkState::Loaded(v, image)
                        }
                        Ok(None) => LinkState::Discarded,
                        Err(e) => {
                            eprintln!("link preview failed: {e}");
                            LinkState::Discarded
                        }
                    };
                }
            }
            UiMessage::RemoveLinkPreview(url) => {
                self.composer_links.iter_mut().filter(|(u, _)| *u == url).for_each(|(_, s)| *s = LinkState::Discarded);
            }
            UiMessage::PasteClipboard => return Task::perform(clipboard_image(), UiMessage::Pasted),
            UiMessage::Emoji(m) => return self.update_emoji(m),
            UiMessage::Upserted(m) => {
                self.message_sound(&m);
                let note = self.notify_message(&m);
                return Task::batch([self.update(UiMessage::StoreChanged), note]);
            }
            UiMessage::ChatUpserted(c) => {
                let note = self.notify_chat(&c);
                return Task::batch([self.update(UiMessage::StoreChanged), note]);
            }
            UiMessage::Typing(chat) => {
                let note = self.notify_typing(chat);
                return Task::batch([self.update(UiMessage::StoreChanged), note]);
            }
            UiMessage::NotifyTimer(chat, timer) => return self.notify_timer(chat, timer),
            UiMessage::Notified(Ok((id, chat))) => _ = self.notifications.insert(id, chat),
            UiMessage::Notified(Err(e)) => self.status_banner = format!("Notifications failed: {e}"),
            UiMessage::NotificationEvent(id, clicked) => {
                let chat = self.notifications.remove(&id).filter(|_| clicked);
                return chat.map_or_else(Task::none, |c| self.update(UiMessage::FocusRequested(Some(c))));
            }
            UiMessage::WindowFocused(focused) => {
                self.focused = focused;
                return self.close_notifications();
            }
            UiMessage::ToggleGifPicker => {
                self.show_gif_picker = !self.show_gif_picker;
                if self.show_gif_picker {
                    self.show_attach_menu = true;
                    let focus = operation::focus("gif-search-input");
                    if self.gifs.is_empty() && !self.gif_loading {
                        self.gif_loading = true;
                        self.gif_error = None;
                        let q = self.gif_query.clone();
                        return Task::batch([
                            focus,
                            Task::perform(beeper_core::cloud::klipy_gifs(q), UiMessage::GifsLoaded),
                        ]);
                    }
                    return focus;
                }
            }
            UiMessage::GifQueryChanged(q) => {
                self.gif_query = q.clone();
                self.gif_loading = true;
                self.gif_error = None;
                return Task::perform(beeper_core::cloud::klipy_gifs(q), UiMessage::GifsLoaded);
            }
            UiMessage::GifsLoaded(r) => {
                self.gif_loading = false;
                match r {
                    Ok(gifs) => {
                        self.gifs = gifs;
                        self.gif_error = None;
                        return self.load_media();
                    }
                    Err(e) => {
                        self.gifs.clear();
                        self.gif_error = Some(e);
                    }
                }
            }
            UiMessage::PickGif(g) => {
                self.show_gif_picker = false;
                let url = g.gif_url.clone();
                return Task::perform(
                    async move {
                        let bytes = beeper_core::cloud::http_bytes(&url).await?;
                        let frames = decode_media(bytes.clone())?;
                        Ok((bytes, frames))
                    },
                    move |r| UiMessage::GifDownloaded(g, r),
                );
            }
            UiMessage::GifDownloaded(g, r) => match r {
                Ok((bytes, frames)) => {
                    self.pasted = Some(PendingMedia {
                        bytes,
                        name: format!("{}.gif", g.id),
                        mime: "image/gif".into(),
                        is_gif: true,
                        size: g.gif_size,
                        frames,
                    });
                    if self.store.read().unwrap().settings.on("autoSendGifs") {
                        self.submit_composer();
                    }
                }
                Err(e) => self.status_banner = format!("Could not download GIF: {e}"),
            },
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
            UiMessage::OpenSearchSelect(chip) => {
                let find = Find(format!("search-chip-{chip}").into(), None);
                return iced::advanced::widget::operate(find).map(move |r| UiMessage::SearchSelect(Some((chip, r))));
            }
            UiMessage::SearchSelect(open) => self.search_select = open,
            UiMessage::PickSearchFilter(pick) => {
                let f = &mut self.search_filters;
                match pick {
                    SearchPick::Account(a) => match f.accounts.iter().position(|x| *x == a) {
                        Some(i) => drop(f.accounts.remove(i)),
                        None => f.accounts.push(a),
                    },
                    SearchPick::Sent(v) => f.sent = (f.sent != Some(v)).then_some(v),
                    SearchPick::Group(v) => f.group = (f.group != Some(v)).then_some(v),
                    SearchPick::Media(v) => match f.media.iter().position(|&x| x == v) {
                        Some(i) => drop(f.media.remove(i)),
                        None => f.media.push(v),
                    },
                    SearchPick::Period(v) => {
                        f.period = (f.period != Some(v)).then_some(v);
                        self.search_select = None;
                    }
                }
                if matches!(self.search_select, Some((1, _))) {
                    self.search_select = None;
                }
            }
            UiMessage::CloseModals if self.search_select.is_some() => self.search_select = None,
            UiMessage::CloseModals => {
                if self.show_command_palette
                    || self.forward.is_some()
                    || self.show_filter_menu
                    || self.chat_menu.is_some()
                    || self.message_menu.is_some()
                    || self.reminder_menu.is_some()
                    || self.audio.menu.is_some()
                    || self.selection_menu.is_some()
                    || self.show_attach_menu
                    || self.show_gif_picker
                    || self.emoji.is_some()
                    || self.quick_look_target.is_some()
                {
                    self.show_command_palette = false;
                    self.forward = None;
                    self.show_filter_menu = false;
                    self.chat_menu = None;
                    self.message_menu = None;
                    self.reminder_menu = None;
                    self.audio.menu = None;
                    self.selection_menu = None;
                    self.show_attach_menu = false;
                    self.show_gif_picker = false;
                    self.emoji = None;
                    self.quick_look_target = None;
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                } else if self.join_matrix_mode {
                    self.join_matrix_mode = false;
                } else if self.creating_chat.is_some() {
                    (self.creating_chat, self.selected_chat_id) = (None, None);
                } else if self.reply_to_message_id.is_some() {
                    self.reply_to_message_id = None;
                } else if self.editing_message_id.is_some() {
                    return self.update(UiMessage::CancelComposeMode);
                } else if self.selected_msg.is_some() {
                    self.selected_msg = None;
                } else if self.selected_chat_id.is_some() {
                    self.leave_chat(&mut self.store.write().unwrap(), None);
                    self.selected_chat_id = None;
                    self.composer = composer_with("");
                    self.refresh_theme_and_tray();
                } else if self.active_tab != InboxTab::AllChats {
                    self.active_tab = InboxTab::AllChats;
                } else {
                    self.status_banner.clear();
                }
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
            UiMessage::ToggleAttachMenu => {
                self.show_attach_menu = !self.show_attach_menu;
                if !self.show_attach_menu {
                    self.show_gif_picker = false;
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
            UiMessage::SetPref(key, v) => {
                self.patch_settings(|s| s.set_pref(key, v));
                let mut st = self.store.write().unwrap();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::pref_write(&mut st, key).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed);
                }
            }
            UiMessage::ToggleHotkey(name) => self.patch_settings(|s| {
                if s.hotkey_on(name) {
                    s.hotkeys_off.insert(name.to_string());
                } else {
                    s.hotkeys_off.remove(name);
                }
            }),
            UiMessage::RevertHotkeys => self.patch_settings(|s| s.hotkeys_off.clear()),
            UiMessage::HotkeyQuery(q) => self.hotkey_query = q,
            UiMessage::Zoom(step) => self.patch_settings(|s| {
                let at = ZOOM_STEPS.iter().position(|z| *z >= s.zoom_percent).expect("zoom is in the steps");
                s.zoom_percent = match step {
                    0 => 100,
                    _ => ZOOM_STEPS[at.saturating_add_signed(step as isize).min(ZOOM_STEPS.len() - 1)],
                };
            }),
            UiMessage::HoverEnter(id) => {
                self.quick_open &= self.hovered_msg.as_ref() == Some(&id);
                self.hovered_msg = Some(id);
            }
            UiMessage::ShowQuickReactions => self.quick_open = true,
            UiMessage::HoverChat(id, true) => self.hovered_chat = Some(id),
            UiMessage::HoverChat(id, false) => {
                if self.hovered_chat.as_ref() == Some(&id) {
                    self.hovered_chat = None;
                }
            }
            UiMessage::DeleteDraft(cid) => {
                self.store.write().unwrap().patch_chat(&cid, true, None, None).expect("chat exists");
                if self.selected_chat_id.as_ref() == Some(&cid) {
                    self.composer = composer_with("");
                    self.reply_to_message_id = None;
                }
            }
            UiMessage::HoverExit(id) => {
                if self.hovered_msg.as_ref() == Some(&id) {
                    self.hovered_msg = None;
                }
            }
            UiMessage::DragStart => self.dragging = true,
            UiMessage::DragMoved(pt) => {
                let rail = if self.store.read().unwrap().settings.on("showSidebar") { 72.0 } else { 0.0 };
                self.sidebar_width = (pt.x - rail).clamp(164.0, 724.0);
            }
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
                    self.main_size = size;
                }
                if let Some(w) = self.media_window.as_mut().filter(|w| w.id == id) {
                    w.size = size;
                }
            }
            UiMessage::Tray(Ok(tray::Event::Up)) => {
                self.tray_up = true;
                self.tray_sent = (None, true);
            }
            UiMessage::Tray(Ok(tray::Event::Show)) => return self.set_hidden(false),
            UiMessage::Tray(Ok(tray::Event::Hide)) => return self.set_hidden(true),
            UiMessage::Tray(Ok(tray::Event::Quit)) => return iced::exit(),
            UiMessage::Tray(Err(e)) | UiMessage::TrayFailed(e) => {
                eprintln!("tray icon failed: {e}");
                self.tray_up = false;
                return self.set_hidden(false);
            }
            UiMessage::CloseRequested(id) => {
                let st = self.store.read().unwrap();
                let quit = st.settings.on("quitOnClose") || !(self.tray_up && st.settings.on("showTrayIcon"));
                drop(st);
                return match Some(id) == self.main_window && !quit {
                    true => self.set_hidden(true),
                    false => window::close(id),
                };
            }
            UiMessage::WindowClosed(id) => {
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                } else if self.media_window.as_ref().is_some_and(|w| w.id == id) {
                    self.media_window = None;
                    self.carousel = self.carousel.take().filter(|c| !c.in_media);
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
            UiMessage::StoreChanged => {
                let st = self.store.read().unwrap();
                let left = self.selected_chat_id.as_deref().is_some_and(|c| st.resolve_chat_idx(c).is_none());
                self.selection.retain(|c| st.resolve_chat_idx(c).is_some());
                drop(st);
                if left {
                    (self.selected_chat_id, self.chat_menu) = (None, None);
                    self.select_chat(NOTE_CHAT_ID);
                }
                self.refresh_theme_and_tray();
                return self.close_notifications();
            }
            UiMessage::FocusRequested(chat) => {
                if let Some(id) = chat {
                    self.select_chat(&id);
                }
                let focus = window::gain_focus(self.main_window.or(self.login_window).expect("a window is open"));
                return Task::batch([focus, self.load_avatars(), self.load_media(), self.close_notifications()]);
            }
            UiMessage::Login(m) => return self.update_login(m),
            UiMessage::KeyPressed(key, mods) if self.carousel.as_ref().is_some_and(|c| !c.in_media) => {
                return self.carousel_key(&key, mods);
            }
            UiMessage::KeyPressed(key, mods) => {
                let n = self.emoji_suggestions(&self.store.read().unwrap()).len();
                if n > 0 && mods.is_empty() {
                    let at = self.suggest.min(n - 1);
                    match key.as_ref() {
                        Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                            let down = key == Key::Named(Named::ArrowDown);
                            self.suggest = if down { (at + 1) % n } else { (at + n - 1) % n };
                            return Task::none();
                        }
                        Key::Named(Named::Tab) => return self.update(UiMessage::PickEmoji(at)),
                        Key::Named(Named::Escape) => {
                            self.suggest_hidden = Some(self.composer.text());
                            return Task::none();
                        }
                        _ => {}
                    }
                }
                let (shift, alt) = (mods.shift(), mods.alt());
                let hotkey = match key.as_ref() {
                    Key::Named(Named::Escape) => return self.update(UiMessage::CloseModals),
                    Key::Named(Named::ArrowUp | Named::ArrowDown) if mods.is_empty() && self.can_select_messages() => {
                        let down = key == Key::Named(Named::ArrowDown);
                        let edit = !down
                            && self.selected_msg.is_none()
                            && self.store.read().unwrap().settings.pref("upArrow") == 1;
                        let step = self.update(UiMessage::StepMessage(down));
                        let Some(m) = edit.then(|| self.last_own_message()).flatten() else { return step };
                        self.selected_msg = Some(m.id.clone());
                        return step.chain(self.update(UiMessage::StartEdit(m.id, m.text.unwrap_or_default())));
                    }
                    Key::Named(Named::Enter) if self.forward.is_some() => {
                        let picked = !self.forward.as_ref().expect("forward").picked.is_empty();
                        return match mods.control() {
                            false => self.update(UiMessage::ForwardSubmit),
                            true if picked => self.update(UiMessage::ForwardSend),
                            true => Task::none(),
                        };
                    }
                    Key::Named(Named::Backspace) if self.forward.as_ref().is_some_and(|f| f.query.is_empty()) => {
                        self.forward.as_mut().expect("forward").picked.pop();
                        return Task::none();
                    }
                    Key::Named(Named::Enter) if mods.is_empty() => {
                        let Some(m) = self.selected_message() else { return Task::none() };
                        return self.update(match m.is_sender == Some(true) {
                            true => UiMessage::StartEdit(m.id.clone(), m.text.clone().unwrap_or_default()),
                            false => UiMessage::StartReply(m.id.clone()),
                        });
                    }
                    Key::Named(Named::ArrowRight)
                        if mods.is_empty() && self.selected_msg.is_some() && self.can_select_messages() =>
                    {
                        return self.update(UiMessage::React(self.selected_msg.clone().expect("a selected message")));
                    }
                    _ if !(mods.command() || mods.control()) => None,
                    Key::Character("F" | "f")
                        if shift && self.selected_message().is_some_and(|m| m.is_deleted != Some(true)) =>
                    {
                        return self.update(UiMessage::Forward(self.selected_msg.clone().expect("a selected message")));
                    }
                    Key::Character("R" | "r") if shift && self.selected_msg.is_some() => {
                        return self.update(self.quick_react(self.selected_msg.clone().expect("a selected message")));
                    }
                    Key::Character("H" | "h") if shift && self.selected_msg.is_some() => {
                        let m = self.selected_message().expect("a selected message");
                        let Some(url) =
                            m.text.as_deref().and_then(|t| t.split_whitespace().find(|w| w.starts_with("http")))
                        else {
                            return Task::none();
                        };
                        return self.update(UiMessage::OpenUrl(url.into()));
                    }
                    Key::Character("r") if self.selected_msg.is_some() => {
                        return self
                            .update(UiMessage::StartReply(self.selected_msg.clone().expect("a selected message")));
                    }
                    Key::Character("c") if self.selected_msg.is_some() => {
                        let stamp = self.store.read().unwrap().settings.on("copyTimestamps");
                        let text = copy_text(&self.selected_message().expect("a selected message"), stamp);
                        return self.update(UiMessage::CopyText(text));
                    }
                    Key::Character("d") if self.selected_message().is_some_and(|m| !m.attachments.is_empty()) => {
                        let mid = self.selected_msg.clone().expect("a selected message");
                        return self.update(UiMessage::Download(mid, None));
                    }
                    Key::Character("t") if self.selected_chat_id.is_some() => {
                        let edit = self.selected_message().filter(|m| m.is_sender == Some(true));
                        let Some(m) = edit.or_else(|| self.last_own_message()) else { return Task::none() };
                        self.selected_msg = Some(m.id.clone());
                        return self.update(UiMessage::StartEdit(m.id, m.text.unwrap_or_default()));
                    }
                    Key::Character("v") if self.selected_chat_id.is_some() => {
                        return Task::perform(clipboard_image(), UiMessage::Pasted);
                    }
                    Key::Character("G" | "g") if shift && self.selected_chat_id.is_some() => {
                        return self.update(UiMessage::ToggleGifPicker);
                    }
                    Key::Character("j") if self.show_command_palette => {
                        return self.update(UiMessage::ToggleCommandMode);
                    }
                    Key::Character(d @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")) if alt => {
                        let i = d.parse::<usize>().expect("a digit") - 1;
                        let msg = match RAIL_VIEWS.get(i) {
                            Some(view) => view.map(UiMessage::SelectTab),
                            None => rail_accounts(&self.store.read().unwrap())
                                .get(i - RAIL_VIEWS.len())
                                .map(|a| UiMessage::SelectAccount(Some(a.clone()))),
                        };
                        return msg.map_or_else(Task::none, |m| self.update(m));
                    }
                    Key::Character("n") => return self.update(UiMessage::NewChat),
                    Key::Character("k") => Some("Search"),
                    Key::Character("f") => Some("Search: Room"),
                    Key::Character(",") => Some("Settings"),
                    Key::Character("Y" | "y") if shift => Some("Filter: Unread"),
                    Key::Character("I" | "i") if shift => Some("Toggle Chat Info"),
                    Key::Character("M" | "m") if shift => Some("Chat: Toggle Mute"),
                    Key::Character("U" | "u") if shift => Some("Chat: Toggle Mark As Read"),
                    Key::Character("P" | "p") if shift => Some("Report a Problem"),
                    Key::Character("p") => Some("Chat: Toggle Pin"),
                    Key::Character("e") => Some("Chat: Toggle Archive"),
                    Key::Character("q") => Some("Quit App"),
                    Key::Character("=" | "+") => Some("Zoom In"),
                    Key::Character("-") => Some("Zoom Out"),
                    Key::Character("0") => Some("Reset Zoom"),
                    Key::Character("]") => Some("Select Next Chat"),
                    Key::Named(Named::ArrowDown) if alt => Some("Select Next Chat"),
                    Key::Character("[") => Some("Select Previous Chat"),
                    Key::Named(Named::ArrowUp) if alt => Some("Select Previous Chat"),
                    _ => None,
                };
                if let Some(h) = hotkey.filter(|h| self.store.read().unwrap().settings.hotkey_on(h)) {
                    return self.run_hotkey(h);
                }
            }
        }
        if let Some(e) = self.store.write().unwrap().matrix_error.take() {
            self.status_banner = e;
        }
        Task::batch([self.load_avatars(), self.load_media(), self.push_outbox()])
    }

    fn media_mut(&mut self) -> &mut MediaWindow {
        self.media_window.as_mut().expect("the Media window is open")
    }

    /// Show the first page of the Media window items after a filter changes.
    fn media_reload(&mut self) -> Task<UiMessage> {
        self.media_mut().shown = MEDIA_PAGE;
        self.load_media()
    }

    fn load_media(&mut self) -> Task<UiMessage> {
        let st = self.store.read().unwrap();
        let client = st.matrix.clone();
        let images = self
            .selected_chat_id
            .as_ref()
            .filter(|_| client.is_some())
            .and_then(|chat| st.chat_msg_indices.get(chat))
            .into_iter()
            .flatten()
            .flat_map(|&i| &st.messages[i].attachments)
            .filter(|a| matches!(a.attachment_type.as_str(), "img" | "video"))
            .filter_map(|a| a.poster_img.clone().or_else(|| a.src_url.clone()));
        let gif_urls =
            self.show_gif_picker.then(|| self.gifs.iter().map(|g| g.preview_url.clone())).into_iter().flatten();
        let hits = self.media_hits(&st);
        let found =
            hits.iter().flat_map(|m| &m.attachments).filter(|a| matches!(a.attachment_type.as_str(), "img" | "video"));
        let found = found.filter_map(|a| a.poster_img.clone().or_else(|| a.src_url.clone()));
        let tiles = self.media_window.iter().filter(|w| w.tab == 0 && client.is_some());
        let tiles = tiles.flat_map(|w| media_items(&st, w).into_iter().take(w.shown));
        let shown = match &self.carousel {
            Some(c) if c.in_media => {
                let (items, at) = self.carousel_items(&st);
                let size = self.media_window.as_ref().expect("the Media window is open").size;
                let (start, end) = filmstrip(&size, at, items.len());
                items[start..end].to_vec()
            }
            c => c.iter().map(|c| (c.mid.clone(), c.ai)).collect(),
        };
        let shown = shown.into_iter().map(|(m, k)| (st.msg_idx_by_id[&m], k));
        let viewer = tiles.chain(shown).filter_map(|(i, k)| {
            let a = &st.messages[i].attachments[k];
            a.poster_img.clone().or_else(|| a.src_url.clone())
        });
        let urls: HashSet<String> =
            images.chain(gif_urls).chain(found).chain(viewer).filter(|u| !self.media.contains_key(u)).collect();
        drop(st);
        self.media.extend(urls.iter().map(|u| (u.clone(), None)));
        let audio = self.load_audio();
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
        .chain(audio)
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
            feed(f.0.read().unwrap().ws_tx.subscribe(), |e| match (e.event_type.as_str(), e.message, e.chat) {
                ("message.upserted", Some(m), _) => UiMessage::Upserted(Box::new(m)),
                ("chat.upserted", _, Some(c)) => UiMessage::ChatUpserted(Box::new(c)),
                ("chat.typing", ..) => UiMessage::Typing(e.chat_id.expect("typing events name the chat")),
                _ => UiMessage::StoreChanged,
            })
        });
        let focus = Subscription::run_with(StoreFeed(self.store.clone(), 1), |f| {
            feed(f.0.read().unwrap().focus_tx.subscribe(), UiMessage::FocusRequested)
        });
        let keys = iced::event::listen_with(|e, _, id| match e {
            Event::Window(window::Event::Closed) => Some(UiMessage::WindowClosed(id)),
            Event::Window(window::Event::CloseRequested) => Some(UiMessage::CloseRequested(id)),
            Event::Window(window::Event::Resized(size)) => Some(UiMessage::WindowResized(id, size)),
            Event::Window(window::Event::Focused) => Some(UiMessage::WindowFocused(true)),
            Event::Window(window::Event::Unfocused) => Some(UiMessage::WindowFocused(false)),
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(UiMessage::Modifiers(m)),
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if modifiers.command()
                    || modifiers.control()
                    || modifiers.alt()
                    || matches!(
                        key,
                        Key::Named(
                            Named::Escape
                                | Named::ArrowUp
                                | Named::ArrowDown
                                | Named::ArrowLeft
                                | Named::ArrowRight
                                | Named::Enter
                                | Named::Tab
                                | Named::Backspace
                        )
                    ) =>
            {
                Some(UiMessage::WindowKey(id, key, modifiers))
            }
            _ => None,
        });
        let system = iced::system::theme_changes().map(UiMessage::SystemTheme);
        let carousel = self.carousel.is_some().then(|| {
            iced::event::listen_with(|e, _, id| match e {
                Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Character(c), modifiers, .. })
                    if modifiers.is_empty() && matches!(c.as_str(), "d" | "r") =>
                {
                    Some(UiMessage::WindowKey(id, Key::Character(c), modifiers))
                }
                _ => None,
            })
        });
        let drag = self.dragging.then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(UiMessage::DragMoved(position)),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::DragEnd),
                _ => None,
            })
        });
        let (typing, due) = {
            let st = self.store.read().unwrap();
            let last = self.selected_chat_id.as_ref().and_then(|c| st.chat_msg_indices.get(c)?.last());
            let rising = last.is_some_and(|&i| send_rise(&st, &st.messages[i]).is_some());
            (!st.typing.is_empty() || rising, st.chats.iter().any(|c| c.reminder.is_some() && !self.expired(c)))
        };
        let remind =
            due.then(|| iced::time::every(std::time::Duration::from_secs(1)).map(|_| UiMessage::CheckReminders));
        let animated = typing
            || self.flash.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 1.5)
            || self.playing_video.as_ref().is_some_and(|(_, s)| matches!(s, Some(Ok(_))))
            || self.pasted.as_ref().is_some_and(|p| p.frames.len() > 1)
            || self.carousel.as_ref().is_some_and(|c| {
                let st = self.store.read().unwrap();
                let a = &st.messages[st.msg_idx_by_id[&c.mid]].attachments[c.ai];
                let url = a.poster_img.as_ref().or(a.src_url.as_ref());
                url.and_then(|u| self.media.get(u)?.as_ref()?.as_ref().ok()).is_some_and(|f| f.len() > 1)
            })
            || (self.show_gif_picker
                && self.gifs.iter().any(|g| {
                    self.media.get(&g.preview_url).and_then(|v| v.as_ref()?.as_ref().ok()).is_some_and(|f| f.len() > 1)
                }))
            || self.selected_chat_id.as_ref().is_some_and(|chat| {
                let st = self.store.read().unwrap();
                let mut urls =
                    st.chat_msg_indices.get(chat).into_iter().flatten().flat_map(|&i| &st.messages[i].attachments);
                urls.any(|a| {
                    a.src_url
                        .as_ref()
                        .and_then(|u| self.media.get(u)?.as_ref()?.as_ref().ok())
                        .is_some_and(|f| f.len() > 1)
                })
            });
        let tray = (cfg!(target_os = "linux") && self.store.read().unwrap().settings.on("showTrayIcon"))
            .then(|| Subscription::run(tray::events).map(UiMessage::Tray));
        let notes = (!self.notifications.is_empty())
            .then(|| Subscription::run(notify::events).map(|(id, clicked)| UiMessage::NotificationEvent(id, clicked)));
        let tick =
            animated.then(|| iced::time::every(std::time::Duration::from_millis(20)).map(|_| UiMessage::AnimationTick));
        Subscription::batch(
            [changes, focus, keys, system]
                .into_iter()
                .chain(drag)
                .chain(carousel)
                .chain(tick)
                .chain(remind)
                .chain(notes)
                .chain(tray)
                .chain(self.audio_ticks()),
        )
    }

    pub fn view(&self, id: window::Id) -> El<'_> {
        let view = self.view_window(id);
        match self.store.read().unwrap().settings.on("pointerCursor") {
            true => view,
            false => cursor::arrow(view).into(),
        }
    }

    fn view_window(&self, id: window::Id) -> El<'_> {
        if Some(id) == self.settings_window {
            self.view_settings()
        } else if let Some(w) = self.media_window.as_ref().filter(|w| w.id == id) {
            self.view_media(w)
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
        let st = self.store.read().unwrap();
        // The sync task can drop a chat before StoreChanged reaches update.
        let known = |c: &str| c.starts_with("draft:") || st.resolve_chat_idx(c).is_some();
        let open = self.creating_chat.is_none() && self.selected_chat_id.as_deref().is_some_and(known);
        let menu_open = self.chat_menu.as_ref().is_some_and(|(c, _)| known(c));
        drop(st);
        let pane = match self.creating_chat {
            Some(_) => self.view_new_chat(),
            None if open => self.view_conversation(),
            None => container(Space::new()).width(Fill).height(Fill).style(fill_box(p.card, 10.0)).into(),
        };
        let info = (self.show_right_panel && open).then(|| self.view_info());
        let right: El<'_> = if self.selection.is_empty() {
            row![container(pane).padding(Padding::new(8.0).left(0.0)).width(Fill)].push(info).into()
        } else {
            let content = container(row![pane].push(info)).padding(8).style(fill_box(p.selected, 18.0));
            column![self.view_selection_head(), content, self.view_selection_bar()]
                .spacing(8)
                .padding(Padding::new(8.0).left(0.0))
                .into()
        };
        let rail = self.store.read().unwrap().settings.on("showSidebar");
        let rail = rail.then(|| self.view_rail());
        let body = mouse_area(Row::new().push(rail).push(self.view_sidebar()).push(grip).push(right))
            .on_move(UiMessage::Cursor);
        let base: El<'_> = container(body).width(Fill).height(Fill).style(fill_box(p.window, 0.0)).into();
        let modal = match (&self.quick_look_target, self.show_command_palette) {
            (Some((title, detail)), _) => Some(self.view_quick_look(title, detail)),
            (None, true) => Some(self.view_command_palette()),
            (None, false) => self.forward.as_ref().map(|f| self.view_forward(f)),
        };
        let scrim = modal.map(|m| {
            mouse_area(
                container(mouse_area(m).on_press(UiMessage::Noop))
                    .center(Fill)
                    .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.2), 0.0)),
            )
            .on_press(UiMessage::CloseModals)
        });
        let select = self.search_select.filter(|_| self.show_command_palette).map(|(chip, at)| {
            mouse_area(
                container(mouse_area(self.view_search_select(chip, at)).on_press(UiMessage::Noop))
                    .padding(Padding::ZERO.left(at.x).top(at.y))
                    .width(Fill)
                    .height(Fill),
            )
            .on_press(UiMessage::SearchSelect(None))
        });
        let filter_menu = self.show_filter_menu.then(|| (self.view_filter_menu(), self.sidebar_width - 14.5, 26.0));
        let reminder_menu = self.reminder_menu.map(|at| (self.view_reminder_menu(), at.x, at.y));
        let rate_menu = self.audio.menu.map(|at| (self.view_rate_menu(), at.x, at.y));
        let chat_menu = self.chat_menu.as_ref().filter(|_| menu_open).map(|(cid, at)| {
            let (w, h) = if cid.starts_with("draft:") {
                (124.0, 31.0)
            } else {
                (233.0, 8.0 + 10.0 * MENU_ITEM + 44.0 + if self.selection.is_empty() { MENU_ITEM + 11.0 } else { 0.0 })
            };
            let x = if at.x + w > self.main_size.width { at.x - w } else { at.x };
            let sub_w = self.chat_sub.map_or(0.0, |i| SUB_MENU_W[usize::from(i)]);
            let left = x + w + sub_w > self.main_size.width;
            (
                self.view_chat_menu(cid, left),
                if left { x - sub_w } else { x },
                at.y.min(self.main_size.height - h - 8.0),
            )
        });
        let message_menu = self.message_menu.as_ref().map(|(mid, at)| {
            let (menu, h) = self.view_message_menu(mid);
            let x = if at.x + MESSAGE_MENU_W > self.main_size.width { at.x - MESSAGE_MENU_W } else { at.x };
            (menu, x, at.y.min(self.main_size.height - h - 8.0))
        });
        let react = self.emoji.as_ref().and_then(|pk| pk.react.as_ref()).map(|(_, at)| {
            let x = if at.x + 352.0 > self.main_size.width { at.x - 352.0 } else { at.x };
            (self.view_emoji_picker().expect("the picker is open"), x, at.y.min(self.main_size.height - 435.0 - 8.0))
        });
        let selection_menu = self.selection_menu.map(|(i, x)| {
            let (menu, h) = self.view_selection_menu(i);
            let w = if i == 0 { SUB_MENU_W[1] } else { 233.0 };
            (menu, x.min(self.main_size.width - w - 8.0), self.main_size.height - 60.0 - h)
        });
        let menu = filter_menu
            .or(reminder_menu)
            .or(rate_menu)
            .or(chat_menu)
            .or(message_menu)
            .or(react)
            .or(selection_menu)
            .map(|(m, x, y)| {
                mouse_area(
                    container(mouse_area(m).on_press(UiMessage::Noop))
                        .padding(Padding::ZERO.left(x).top(y))
                        .width(Fill)
                        .height(Fill),
                )
                .on_press(UiMessage::CloseModals)
                .on_right_press(UiMessage::CloseModals)
            });
        let toast = self.view_toast(76.0);
        let confirm = self.confirm.filter(|&d| d != ASK_LOGOUT).map(|d| self.view_confirm(d));
        let carousel = self.carousel.as_ref().filter(|c| !c.in_media).map(|c| self.view_carousel(c));
        stack![base].push(toast).push(menu).push(scrim).push(select).push(confirm).push(carousel).into()
    }

    /// The selected chats that still exist: the sync task can drop one before StoreChanged reaches update.
    fn selected_chats<'s>(&self, st: &'s BeeperStore) -> Vec<&'s Chat> {
        self.selection.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect()
    }

    fn view_selection_head(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let chats = self.selected_chats(&st);
        let n = chats.len();
        let step = 28.0 / 3.0;
        let avatars = chats.iter().enumerate().fold(Stack::new(), |s, (i, c)| {
            s.push(pin(container(self.chat_avatar(c, 28.0)).style(fill_box(p.card, 14.0))).x(i as f32 * step))
        });
        let avatars = container(avatars.width((n + 2) as f32 * step).height(28)).width(Fill).clip(true);
        let title = semi(format!("{n} chat{} selected", if n == 1 { "" } else { "s" }), 15.0).color(p.strong);
        let close = button(tinted(&ROW_ACTIONS[6], 20.0, p.strong))
            .padding(6)
            .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 16.0))
            .on_press(UiMessage::CloseModals);
        let row = row![avatars, container(title).width(140), container(close).width(Fill).align_right(Fill)];
        container(row.align_y(Center).padding([0, 4])).height(36).width(Fill).style(fill_box(p.selected, 18.0)).into()
    }

    fn view_selection_bar(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let chats = self.selected_chats(&st);
        let all = |f: ChatFlag| chats.iter().all(|c| has_flag(c, f));
        let x0 = 68.0 + self.sidebar_width + 16.0;
        let cell = (self.main_size.width - x0 - 16.0) / 6.0;
        let toggles = [
            (
                ChatFlag::Archived,
                ["Archive", "Unarchive"],
                ["Archive", "Unarchive"],
                [&SELECTION_ICONS[0], &SELECTION_ICONS[1]],
                vec!["Ctrl", "E"],
            ),
            (
                ChatFlag::MarkedUnread,
                ["Mark Unread", "Mark Read"],
                ["Mark as Unread", "Mark as Read"],
                [&ROW_ACTIONS[3], &ROW_ACTIONS[2]],
                vec!["Ctrl", "Shift", "U"],
            ),
            (
                ChatFlag::Muted,
                ["Mute", "Unmute"],
                ["Mute", "Unmute"],
                [&SELECTION_ICONS[2], &SELECTION_ICONS[3]],
                vec!["Ctrl", "Shift", "M"],
            ),
            (
                ChatFlag::LowPriority,
                ["Low Priority", "Move to Inbox"],
                ["Move to Low Priority", "Move to Inbox"],
                [&SELECTION_ICONS[4], &SELECTION_ICONS[5]],
                vec![],
            ),
        ]
        .map(|(f, labels, tips, glyphs, keys)| {
            let on = usize::from(all(f));
            (labels[on], tips[on], glyphs[on], UiMessage::SelectionSet(f, on == 0), keys)
        });
        let menus = [("Remind Me", &SELECTION_ICONS[6]), ("More", &SELECTION_ICONS[7])].into_iter().zip(0u8..).map(
            |((label, glyph), i)| {
                let x = x0 + cell * (4.5 + f32::from(i));
                (label, label, glyph, UiMessage::SelectionMenu(i, x), vec![])
            },
        );
        let wide = cell * 6.0 >= 800.0;
        let buttons = toggles.into_iter().chain(menus).map(|(label, tip, glyph, msg, keys)| {
            let face = row![]
                .push(wide.then(|| semi(label, 14.0).color(p.strong)))
                .push(tinted(glyph, 24.0, p.strong))
                .spacing(10)
                .align_y(Center);
            let btn = button(container(face).center_x(Fill)).padding([6, 0]).width(Fill).style(btn_style(
                [CLEAR; 3],
                [p.strong; 3],
                0.0,
            ));
            let btn = btn.on_press(msg).into();
            match self.selection_menu {
                Some(_) => btn,
                None => self.tip(btn, tip, keys, tooltip::Position::Top).into(),
            }
        });
        let bar = container(Row::with_children(buttons).align_y(Center))
            .center_y(36)
            .width(Fill)
            .style(fill_box(p.card, 18.0));
        container(bar).padding(8).style(fill_box(p.selected, 26.0)).into()
    }

    /// The Remind Me or More menu of the selection bar, and its height.
    fn view_selection_menu(&self, which: u8) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let chats = self.selected_chats(&st);
        let item = |label: String, at: String, msg| menu_item(p, None, label, at, msg);
        let items: Vec<El> = if which == 0 {
            let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
            let presets = REMIND_PRESETS.iter().enumerate().map(|(k, label)| {
                let at = remind_preset(k, now);
                item((*label).into(), remind_label(at, h24), Some(UiMessage::SelectionRemind(Some(at))))
            });
            let set = chats.iter().filter(|c| c.reminder.is_some()).count();
            let dismiss = match set {
                0 => vec![],
                1 => vec![
                    menu_rule(p),
                    item("Dismiss Reminder".into(), String::new(), Some(UiMessage::SelectionRemind(None))),
                ],
                n => vec![
                    menu_rule(p),
                    item(format!("Dismiss {n} Reminders"), String::new(), Some(UiMessage::SelectionRemind(None))),
                ],
            };
            presets.chain(dismiss).collect()
        } else {
            let n = chats.len();
            let count = |f: ChatFlag| chats.iter().filter(|c| has_flag(c, f)).count();
            let some = |f: ChatFlag| (1..n).contains(&count(f));
            let entry = |label: &str, keys: &str, f: ChatFlag, on: bool| {
                item(label.into(), keys.into(), Some(UiMessage::SelectionSet(f, on)))
            };
            let pinned = count(ChatFlag::Pinned);
            [Some(item(format!("{n} chat{} selected", if n == 1 { "" } else { "s" }), String::new(), None))]
                .into_iter()
                .chain([
                    some(ChatFlag::Archived).then(|| entry("Unarchive", "Ctrl E", ChatFlag::Archived, false)),
                    some(ChatFlag::MarkedUnread)
                        .then(|| entry("Mark as Read", "Ctrl Shift U", ChatFlag::MarkedUnread, false)),
                    some(ChatFlag::Muted).then(|| entry("Unmute", "Ctrl Shift M", ChatFlag::Muted, false)),
                    some(ChatFlag::LowPriority).then(|| entry("Move to Inbox", "", ChatFlag::LowPriority, false)),
                    (pinned < n).then(|| entry("Pin", "Ctrl P", ChatFlag::Pinned, true)),
                    (pinned > 0).then(|| entry("Unpin", "Ctrl P", ChatFlag::Pinned, false)),
                ])
                .flatten()
                .collect()
        };
        let rules = if which == 0 && items.len() > REMIND_PRESETS.len() { 11.0 - MENU_ITEM } else { 0.0 };
        let h = 8.0 + items.len() as f32 * MENU_ITEM + rules;
        let width = if which == 0 { SUB_MENU_W[1] } else { 233.0 };
        let menu = container(Column::with_children(items).padding([4, 0])).width(width).style(shadow_box(
            p.menu,
            p.menu_border,
            6.0,
            16.0,
        ));
        (menu.into(), h)
    }

    fn view_filter_menu(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let item = |checked: bool, glyph: char, label: String, hotkey: &'static str, bit: u8| -> El<'_> {
            let lead = row![
                container(checked.then(|| ic(icon::CHECK, 14.0))).center_x(14),
                container(ic(glyph, 14.0)).padding(Padding::ZERO.left(9.0).right(4.0)),
            ];
            menu_item(p, Some(lead.into()), label, hotkey, Some(UiMessage::PickFilter(bit)))
        };
        let counted = (0..F_ARCHIVED).map(|i| {
            let n = self.chats_with(&st, 1 << i).len();
            let hotkey = if i == 0 { "Ctrl Shift Y" } else { "" };
            item(self.filters & 1 << i != 0, FILTERS[i].1, format!("{} ({n})", FILTERS[i].0), hotkey, 1 << i)
        });
        let views = (F_ARCHIVED..FILTERS.len())
            .map(|i| item(self.filters & 1 << i != 0, FILTERS[i].1, FILTERS[i].0.into(), "", 1 << i));
        let col = Column::with_children(
            std::iter::once(item(self.filters == 0, icon::INBOX, "All".into(), "", 0))
                .chain(counted)
                .chain(std::iter::once(menu_rule(p)))
                .chain(views),
        );
        container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    fn view_reminder_menu(&self) -> El<'_> {
        let p = &self.pal;
        let order = self.store.read().unwrap().settings.reminders_order;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let orders = REMINDER_ORDERS
            .iter()
            .zip(0..)
            .map(|(label, i)| menu_item(p, lead(order == i), (*label).into(), "", Some(UiMessage::OrderReminders(i))));
        let ask = Some(UiMessage::Ask(Some(ASK_DISMISS_REMINDERS)));
        let col = Column::with_children(
            orders.chain([menu_rule(p), menu_item(p, lead(false), "Dismiss All Reminders".into(), "", ask)]),
        );
        container(col.padding([4, 0])).width(291).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    /// Desktop's message context menu, with its height.
    fn view_message_menu(&self, mid: &str) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let m = &st.messages[st.msg_idx_by_id[mid]];
        let (cid, mid, mine) = (m.chat_id.clone(), m.id.clone(), m.is_sender == Some(true));
        let text = m.text.clone().unwrap_or_default();
        let link = m.links.first().map(|l| l.url.clone());
        let entire = if link.is_some() { " Entire Message" } else { "" };
        let rule = || (menu_rule(p), 11.0);
        let item = |label: String, hotkey, msg: Option<UiMessage>| {
            (menu_item(p, None, label, hotkey, msg.map(|m| UiMessage::MessageMenu(Box::new(m)))), MENU_ITEM)
        };
        let bookmark = if m.is_bookmarked == Some(true) { "Remove Bookmark" } else { "Bookmark" };
        let at = format!("{} at {}", if mine { "Sent" } else { "Received" }, locale_string(&m.timestamp));
        let stamp = st.settings.on("copyTimestamps");
        let files = m.attachments.len();
        let image = m.attachments.first().is_some_and(|a| a.attachment_type == "img");
        let only_image = image && m.text.is_none();
        let noun = if image { "Image" } else { "Attachment" };
        let download = match m.attachments.first().and_then(|a| a.file_size).filter(|_| files == 1) {
            Some(n) => format!("Download {noun} ({})", human_size(n)),
            None => format!("Download {noun}{}", if files == 1 { "" } else { "s" }),
        };
        let rows: Vec<(El, f32)> = [
            link.map(|u| vec![item("Copy Link".into(), "", Some(UiMessage::CopyText(u))), rule()]),
            (mine && !text.is_empty())
                .then(|| vec![item("Edit…".into(), "Ctrl T", Some(UiMessage::StartEdit(mid.clone(), text)))]),
            Some(vec![
                item("Reply…".into(), "Ctrl R", Some(UiMessage::StartReply(mid.clone()))),
                item("React…".into(), "", Some(UiMessage::React(mid.clone()))),
                item(bookmark.into(), "", Some(UiMessage::ToggleBookmark(mid.clone()))),
            ]),
            (m.is_unread != Some(true)).then(|| {
                vec![item("Mark as Unread".into(), "", Some(UiMessage::MarkUnreadFromHere(cid, mid.clone())))]
            }),
            mine.then(|| vec![item("Delete for Everyone…".into(), "", Some(UiMessage::AskDeleteMessage(mid.clone())))]),
            Some(vec![
                rule(),
                item(
                    format!("Copy{entire}"),
                    if stamp { "" } else { "Ctrl C" },
                    Some(match only_image {
                        true => UiMessage::CopyImage(mid.clone(), 0),
                        false => UiMessage::CopyText(copy_text(m, false)),
                    }),
                ),
            ]),
            (!image).then(|| {
                let copy = UiMessage::CopyText(copy_text(m, true));
                vec![item(format!("Copy{entire} with Timestamp"), if stamp { "Ctrl C" } else { "" }, Some(copy))]
            }),
            (image && !only_image)
                .then(|| vec![item("Copy Image".into(), "", Some(UiMessage::CopyImage(mid.clone(), 0)))]),
            (m.is_deleted != Some(true))
                .then(|| vec![item("Forward…".into(), "Ctrl Shift F", Some(UiMessage::Forward(mid.clone())))]),
            (files > 0).then(|| vec![item(download, "Ctrl D", Some(UiMessage::Download(mid.clone(), None)))]),
            Some(vec![
                item(at, "", None),
                rule(),
                item("Report a Problem".into(), "Ctrl Shift P", Some(UiMessage::OpenUrl(ISSUES_URL.into()))),
            ]),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let h = 8.0 + rows.iter().map(|r| r.1).sum::<f32>();
        let col = Column::with_children(rows.into_iter().map(|r| r.0));
        (
            container(col.padding([4, 0]))
                .width(MESSAGE_MENU_W)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into(),
            h,
        )
    }

    /// Messages for the search media grid, newest first.
    fn media_hits<'s>(&self, st: &'s BeeperStore) -> Vec<&'s Message> {
        let f = &self.search_filters;
        if !self.show_command_palette || self.cmd_bar_commands_mode || f.media.is_empty() {
            return Vec::new();
        }
        let q = self.command_palette_query.trim().to_lowercase();
        let text: Option<HashSet<u32>> =
            (q.chars().count() > 1).then(|| st.msg_search_index.query_all_tokens(&q).into_iter().collect());
        let mut hits: Vec<&Message> = st
            .messages
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                text.as_ref().is_none_or(|t| {
                    t.contains(&(*i as u32))
                        || st.chats[st.chat_idx_by_id[&m.chat_id]].title.to_lowercase().contains(&q)
                }) && f.keeps(st, self.search_room.as_deref(), m)
            })
            .map(|(_, m)| m)
            .collect();
        hits.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
        hits.truncate(50);
        hits
    }

    fn view_search_select(&self, chip: usize, at: iced::Rectangle) -> El<'_> {
        let p = &self.pal;
        let f = &self.search_filters;
        let pick = |m: SearchPick| Some(UiMessage::PickSearchFilter(m));
        let radio = |on: bool, label: &str, m: SearchPick| {
            let lead = container(on.then(|| ic(icon::CHECK, 14.0))).center_x(14).padding(Padding::ZERO.right(6.0));
            menu_item(p, Some(lead.into()), label.into(), "", pick(m))
        };
        let menu = |items: Vec<El<'static>>| -> El<'static> {
            container(Column::with_children(items).padding([4, 0]))
                .width(233)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into()
        };
        let brand = |options: Vec<(String, bool, SearchPick)>| -> El<'static> {
            let any = options.iter().any(|o| o.1);
            let widest = options.iter().map(|o| crate::style::text_width(&o.0, 11.0, Weight::Semibold));
            let width = widest.fold(at.width, |w, t| w.max(t + 48.0));
            let hover = Color { a: 0.1, ..p.text };
            let options = options.into_iter().map(|(label, on, m)| {
                let fg = if on || !any { p.text } else { p.sub };
                let line = row![
                    semi(label, 11.0).line_height(Pixels(14.0)).color(fg),
                    Space::new().width(Fill),
                    ic(icon::CHECK, 16.0).color(if on { fg } else { CLEAR })
                ]
                .spacing(8)
                .align_y(Center);
                let style = btn_style([CLEAR, hover, hover], [fg; 3], 4.0);
                button(line).padding([6, 4]).width(Fill).style(style).on_press(UiMessage::PickSearchFilter(m)).into()
            });
            let head = row![
                semi(f.label(chip).unwrap_or_else(|| SEARCH_CHIPS[chip].into()), 13.0).color(p.text),
                Space::new().width(Fill),
                ic(icon::EXPAND_MORE, 14.0).color(p.text)
            ]
            .spacing(4)
            .padding([0, 12])
            .align_y(Center);
            let body = column![head, Column::with_children(options).spacing(4).padding([0, 8])].spacing(8);
            container(body).width(width).padding([4, 0]).style(shadow_box(p.chip, CLEAR, 8.0, 4.0)).into()
        };
        match chip {
            0 => {
                let st = self.store.read().unwrap();
                let mut accounts: Vec<String> = std::iter::once("hungryserv".into())
                    .chain(st.accounts.iter().map(|a| a.account_id.clone()))
                    .collect();
                accounts.sort_by_key(|a| network_label(a));
                brand(
                    accounts
                        .into_iter()
                        .map(|a| (network_label(&a).to_string(), f.accounts.contains(&a), SearchPick::Account(a)))
                        .collect(),
                )
            }
            1 => menu(vec![
                radio(f.sent == Some(true), "Sent", SearchPick::Sent(true)),
                radio(f.sent == Some(false), "Received", SearchPick::Sent(false)),
                menu_rule(p),
                radio(f.group == Some(true), "Group", SearchPick::Group(true)),
                radio(f.group == Some(false), "Direct message", SearchPick::Group(false)),
            ]),
            2 => menu(
                PERIODS.iter().enumerate().map(|(i, l)| radio(f.period == Some(i), l, SearchPick::Period(i))).collect(),
            ),
            3 => brand(
                MEDIA_KINDS
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (l.to_string(), f.media.contains(&i), SearchPick::Media(i)))
                    .collect(),
            ),
            _ => unreachable!("chip {chip} has no menu"),
        }
    }

    fn view_chat_menu(&self, cid: &str, sub_left: bool) -> El<'_> {
        let p = &self.pal;
        if let Some(n) = cid.strip_prefix("draft:") {
            let delete =
                menu_item(p, None, "Delete Chat".into(), "", Some(UiMessage::DeleteNewChat(n.parse().unwrap())));
            return container(column![delete].padding([4, 0]))
                .width(124)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into();
        }
        let st = self.store.read().unwrap();
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let on = |v: Option<bool>| v == Some(true);
        let flag = |f: ChatFlag| Some(UiMessage::ToggleFlag(c.id.clone(), f));
        let archived = on(c.is_archived);
        let unread = c.unread_count > 0 || on(c.is_marked_unread);
        let close_sub = |items: Column<'static, UiMessage>| mouse_area(items).on_enter(UiMessage::ChatSub(None));
        let open_sub = |i: u8, label: &str| {
            mouse_area(menu_item(p, None, label.into(), "›", Some(UiMessage::ChatSub(Some(i)))))
                .on_enter(UiMessage::ChatSub(Some(i)))
        };
        let col = column![
            close_sub(column![
                menu_item(
                    p,
                    None,
                    if archived { "Unarchive" } else { "Archive" }.into(),
                    "Ctrl E",
                    flag(ChatFlag::Archived).filter(|_| archived || !on(c.is_pinned)),
                ),
                menu_item(
                    p,
                    None,
                    if on(c.is_muted) { "Unmute" } else { "Mute" }.into(),
                    "Ctrl Shift M",
                    flag(ChatFlag::Muted)
                ),
                menu_item(
                    p,
                    None,
                    if unread { "Mark as Read" } else { "Mark as Unread" }.into(),
                    "Ctrl Shift U",
                    Some(UiMessage::ToggleRead(c.id.clone())),
                ),
                menu_item(
                    p,
                    None,
                    if on(c.is_pinned) { "Unpin" } else { "Pin" }.into(),
                    "Ctrl P",
                    flag(ChatFlag::Pinned)
                ),
            ]),
            open_sub(0, "Copy"),
            close_sub(column![
                menu_rule(p),
                menu_item(p, None, "Search…".into(), "Ctrl F", Some(UiMessage::SearchRoom(c.id.clone()))),
                menu_rule(p),
                menu_item(
                    p,
                    None,
                    if on(c.is_low_priority) { "Move to Inbox" } else { "Move to Low Priority" }.into(),
                    "",
                    flag(ChatFlag::LowPriority),
                ),
            ]),
            open_sub(1, "Remind Me"),
            close_sub(
                column![
                    menu_rule(p),
                    menu_item(
                        p,
                        None,
                        "Delete Chat".into(),
                        "",
                        (c.delete_modes != 0).then(|| UiMessage::AskDeleteChat(c.id.clone())),
                    ),
                ]
                .push(self.selection.is_empty().then(|| menu_rule(p)))
                .push(self.selection.is_empty().then(|| {
                    menu_item(p, None, "Begin Selecting".into(), "", Some(UiMessage::BeginSelecting(c.id.clone())))
                }))
                .push(menu_rule(p))
                .push(menu_item(
                    p,
                    None,
                    "Report a Problem".into(),
                    "Ctrl Shift P",
                    Some(UiMessage::OpenUrl(ISSUES_URL.into()))
                ))
            ),
        ];
        let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let sub = self.chat_sub.map(|i| {
            let item = |label: &str, at: String, msg| menu_item(p, None, label.into(), at, msg);
            let items: Vec<El> = match i {
                0 => vec![item("Chat Link", String::new(), Some(UiMessage::CopyChatLink(c.id.clone())))],
                _ => {
                    let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
                    let presets = REMIND_PRESETS.iter().enumerate().map(|(k, label)| {
                        let at = remind_preset(k, now);
                        item(label, remind_label(at, h24), Some(UiMessage::SetReminder(c.id.clone(), Some(at))))
                    });
                    let dismiss = c.reminder.is_some().then(|| UiMessage::SetReminder(c.id.clone(), None));
                    presets.chain([menu_rule(p), item("Dismiss Reminder", String::new(), dismiss)]).collect()
                }
            };
            let top = if i == 0 { 4.0 * MENU_ITEM } else { 7.0 * MENU_ITEM + 22.0 };
            let body = container(Column::with_children(items).padding([4, 0])).width(SUB_MENU_W[usize::from(i)]);
            column![Space::new().height(top), body.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))]
        });
        let (l, r) = if sub_left { (sub, None) } else { (None, sub) };
        row![].push(l).push(menu).push(r).into()
    }

    fn mark_read(&self, st: &mut BeeperStore, idx: usize) {
        if self.expired(&st.chats[idx]) {
            st.set_reminder(idx, None);
        }
        let cid = st.chats[idx].id.clone();
        if let Some(ev) = st.mark_chat_read(&cid, true) {
            st.queue(&cid, Outgoing::Read(ev));
        }
    }

    /// Desktop's `setIsArchived`: archiving marks the chat read when "Mark unread chats as read on archive" is on.
    /// Returns false for a pinned chat, which stays in the inbox.
    fn set_archived(&self, st: &mut BeeperStore, idx: usize, on: bool) -> bool {
        if on && st.settings.on("markReadOnArchive") {
            self.mark_read(st, idx);
        }
        st.set_chat_flag(idx, ChatFlag::Archived, Some(on))
    }

    /// Desktop's `setThreadReminder`: the reminder ends on a reply unless "Default for remind later" is "Always",
    /// and the chat is archived when "Archive after setting reminder on chat" is on.
    fn remind(&self, st: &mut BeeperStore, idx: usize, at: Option<i64>) {
        let dismiss_on_incoming_message = st.settings.pref("remindLaterDefault") == 0;
        st.set_reminder(idx, at.map(|remind_at_ms| ChatReminder { remind_at_ms, dismiss_on_incoming_message }));
        let c = &st.chats[idx];
        if at.is_some() && st.settings.on("archiveOnReminder") && c.flags & (CHAT_PINNED | CHAT_REQUEST) == 0 {
            self.set_archived(st, idx, true);
        }
    }

    /// The chat IDs in list order: the pinned tiles, then the rows.
    fn list_ids(&self, st: &BeeperStore) -> Vec<String> {
        let (pins, rows) = self.pins_and_rows(st);
        pins.into_iter().chain(rows).map(|c| c.id.clone()).collect()
    }

    /// Desktop's `afterToggleArchive` and `afterToggleRead`: select a chat next to list position `at` by the `key`
    /// setting. An archived chat left the list, so the chat at `at` is the next one.
    fn after_action(&mut self, key: &str, list: &[String], at: usize, archived: bool) -> Task<UiMessage> {
        let get = |i: Option<usize>| i.and_then(|i| list.get(i));
        let (next, prev) = match archived {
            true => (get(Some(at)).or(get(at.checked_sub(1))), get(at.checked_sub(1)).or(get(Some(at)))),
            false => (get(Some(at + 1)), get(at.checked_sub(1))),
        };
        let pick = match self.store.read().unwrap().settings.pref(key) {
            0 => next,
            1 => prev,
            2 => None,
            _ => return Task::none(),
        };
        match pick {
            Some(id) => self.update(UiMessage::SelectChat(id.clone())),
            None => {
                self.leave_chat(&mut self.store.write().unwrap(), None);
                self.selected_chat_id = None;
                Task::none()
            }
        }
    }

    /// Desktop's `hasExpiredReminder`: the reminder time came while the reminder was set.
    fn expired(&self, c: &Chat) -> bool {
        c.reminder.as_ref().is_some_and(|r| self.reminded.get(&c.id) == Some(&r.remind_at_ms))
    }

    fn row_action<'a>(&self, c: &Chat, pref: u8) -> Option<El<'a>> {
        let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
        let archived = c.flags & CHAT_ARCHIVED != 0;
        let (glyph, size, label, keys, msg) = match pref {
            _ if self.active_tab == InboxTab::Reminders => {
                (6, 20.0, "Dismiss Reminder", vec![], UiMessage::SetReminder(c.id.clone(), None))
            }
            _ if c.draft.is_some() => (5, 20.0, "Delete draft", vec![], UiMessage::DeleteDraft(c.id.clone())),
            0 if archived || c.flags & CHAT_PINNED == 0 => {
                let msg = UiMessage::ToggleFlag(c.id.clone(), ChatFlag::Archived);
                (usize::from(archived), 14.0, if archived { "Unarchive" } else { "Archive" }, vec!["Ctrl", "E"], msg)
            }
            1 => {
                let label = if unread { "Mark Read" } else { "Mark Unread" };
                (2 + usize::from(!unread), 20.0, label, vec!["Ctrl", "Shift", "U"], UiMessage::ToggleRead(c.id.clone()))
            }
            2 => (4, 20.0, "Open Context Menu", vec![], UiMessage::OpenChatMenu(c.id.clone())),
            _ => return None,
        };
        Some(self.hover_btn(glyph, size, label, keys, msg))
    }

    fn hover_btn<'a>(
        &self,
        glyph: usize,
        size: f32,
        label: &'a str,
        keys: Vec<&'static str>,
        msg: UiMessage,
    ) -> El<'a> {
        let p = &self.pal;
        let btn = button(tinted(&ROW_ACTIONS[glyph], size, p.strong)).padding(6).style(btn_style(
            [CLEAR; 3],
            [p.strong; 3],
            0.0,
        ));
        let tip = self.tip(btn.on_press(msg).into(), label, keys, tooltip::Position::Right);
        let top = 22f32.min(51.0 - 12.0 - size);
        container(tip.delay(std::time::Duration::from_secs(1)))
            .align_right(Fill)
            .padding(Padding::ZERO.top(top).right(4.0))
            .into()
    }

    fn tip<'a>(
        &self,
        el: El<'a>,
        label: impl iced::widget::text::IntoFragment<'a>,
        keys: Vec<&'static str>,
        at: tooltip::Position,
    ) -> iced::widget::Tooltip<'a, UiMessage> {
        let p = &self.pal;
        let kbd = |k: &'static str| -> El<'a> {
            container(med(k, 13.0).color(p.strong))
                .center_x(if k.len() == 1 { Length::Fixed(20.0) } else { Shrink })
                .center_y(20)
                .padding([0, 2])
                .style(fill_box(p.chip, 4.0))
                .into()
        };
        let keys = (!keys.is_empty()).then(|| Row::with_children(keys.into_iter().map(kbd)).spacing(4));
        let body = row![med(label, 13.0).color(p.strong)].push(keys).spacing(8).align_y(Center);
        tooltip(el, container(body).padding([8, 12]).style(fill_box(p.card, 18.0)), at)
    }

    fn view_rail(&self) -> El<'_> {
        let p = &self.pal;
        let rail_btn = |glyph: El<'static>, on: bool, msg: UiMessage| {
            button(container(glyph).center(36))
                .padding(0)
                .style(btn_style([if on { p.rail_sel } else { CLEAR }, p.hover, p.rail_sel], [p.strong; 3], 14.0))
                .on_press(msg)
        };
        let tip = |btn, label, keys| self.tip(btn, label, keys, tooltip::Position::Right);
        let hotkey = |n: usize| -> Vec<&'static str> {
            ["1", "2", "3", "4", "5", "6", "7", "8", "9"].get(n).map_or(vec![], |d| vec!["Ctrl", "Alt", d])
        };
        let st = self.store.read().unwrap();
        let numbers = st.settings.on("showUnreadCount");
        let badge = |btn: El<'static>, n: usize, number: bool| -> El<'static> {
            let dot = match number && numbers {
                true => container(semi(if n > 999 { "999+".into() } else { n.to_string() }, 10.0).color(Color::WHITE))
                    .padding([0, 4])
                    .center_y(14)
                    .width(Shrink)
                    .style(fill_box(p.accent, 8.0)),
                false => container(Space::new()).width(8).height(8).style(fill_box(p.accent, 4.0)),
            };
            let corner = (n > 0).then(|| container(dot).align_right(36).padding(Padding::ZERO.top(2.0).right(2.0)));
            stack![btn].push(corner).into()
        };
        let countable = |c: &Chat| {
            c.flags & CHAT_UNREAD != 0
                && c.flags & (CHAT_ARCHIVED | CHAT_REQUEST) == 0
                && c.merged_into_chat_id.is_none()
                && (c.unread_mentions > 0 || c.flags & CHAT_MUTED == 0)
        };
        let tally = |f: &dyn Fn(&Chat) -> bool| st.chats.iter().filter(|c| countable(c) && f(c)).count();
        let tab =
            |id: &'static str, key: usize, glyph: El<'static>, t: InboxTab, n: usize, number: bool| -> El<'static> {
                let on = self.active_tab == t && self.selected_account_filter.is_none();
                let btn = badge(rail_btn(glyph, on, UiMessage::SelectTab(t)).into(), n, number);
                container(tip(btn, t.label(), hotkey(key))).id(id).into()
            };
        let requests = st.chats.iter().filter(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0).count();
        let g = |c: char| -> El<'static> { ic(c, 24.0).color(p.strong).into() };
        let ring_color = if self.dark { Color::WHITE } else { Color::BLACK };
        let ring = svg(DASHED_RING.clone())
            .width(32)
            .height(32)
            .opacity(0.15_f32)
            .style(move |_, _| svg::Style { color: Some(ring_color) });
        let add_space =
            container(stack![ring, container(ic(icon::MAPS_UGC, 24.0).color(p.strong)).center(32)]).center(36);
        let accounts =
            rail_accounts(&st).into_iter().enumerate().fold(Column::new().spacing(8).align_x(Center), |c, (i, acc)| {
                let on = self.selected_account_filter.as_ref() == Some(&acc);
                let space = network_space(&st, &acc);
                let owner = space.map_or(acc.as_str(), |n| &n.account_id);
                let logo: El<'static> = svg(brand_svg(brand_key(owner))).width(28).height(28).into();
                let n = tally(&|c| {
                    in_rail_entry(&st, &acc, c) && c.flags & CHAT_HIDDEN != 0 && c.flags & CHAT_LOW_PRIORITY == 0
                });
                let label = space.map_or_else(
                    || network_label(owner).to_string(),
                    |n| format!("{} ({})", n.name, network_label(owner)),
                );
                let btn =
                    badge(rail_btn(logo, on, UiMessage::SelectAccount((!on).then_some(acc.clone()))).into(), n, true);
                c.push(self.tip(btn, label, hotkey(7 + i), tooltip::Position::Right))
            });
        let add_label = button(add_space).padding(0).style(ghost(p, 12.0)).on_press(UiMessage::ToggleCommandPalette);
        let top = column![
            tab(
                "rail-inbox",
                0,
                g(icon::INBOX),
                InboxTab::AllChats,
                tally(&|c| c.flags & (CHAT_HIDDEN | CHAT_LOW_PRIORITY) == 0),
                true
            ),
            tab("rail-archive", 1, g(icon::ARCHIVE), InboxTab::Archived, 0, false),
            tip(add_label.into(), "Create Label", vec![]),
            tab("rail-reminders", 3, g(icon::SCHEDULE), InboxTab::Reminders, 0, false),
            tab("rail-send-later", 4, g(icon::SCHEDULE_SEND), InboxTab::SendLater, 0, false),
            tab("rail-requests", 5, g(icon::EMAIL), InboxTab::Requests, requests, false),
            tab(
                "rail-low-priority",
                6,
                g(icon::LOW_PRIORITY_SPACE),
                InboxTab::LowPriority,
                tally(&|c| c.flags & CHAT_LOW_PRIORITY != 0),
                true
            ),
            accounts,
            rail_btn(g(icon::MORE_HORIZ), false, UiMessage::OpenSettings(SettingsTab::Accounts)),
        ]
        .spacing(8)
        .align_x(Center)
        .width(68)
        .padding(Padding::ZERO.top(14.0));
        let me = st.me().full_name.as_deref().expect("self name");
        let media = UiMessage::OpenMedia;
        let me_btn = button(avatar(me, 36.0, p))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.text; 3], 18.0))
            .on_press(UiMessage::OpenSettings(SettingsTab::General));
        let footer = column![
            tip(rail_btn(g(icon::PHOTO_LIBRARY), false, media).into(), "Media", vec![]),
            tip(
                rail_btn(g(icon::LIBRARY_ADD), false, UiMessage::OpenSettings(SettingsTab::Accounts)).into(),
                "Add an account",
                vec!["Ctrl", ","]
            ),
            tip(me_btn.into(), "Settings", vec!["Ctrl", ","]),
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
        let compact = st.settings.on("compactChatList");
        let (av, lines, title_size, ts_size, pin_av) =
            if compact { (28.0, 1, 13.0, 10.0, 40.0) } else { (46.0, 2, 14.0, 11.0, 52.0) };
        let names: Vec<&str> =
            (0..FILTERS.len()).filter(|i| self.filters & 1 << i != 0).map(|i| FILTERS[i].0).collect();
        let heading = self.selected_account_filter.as_deref().map_or(self.active_tab.label().to_string(), |id| {
            network_space(&st, id).map_or_else(|| network_label(id).to_string(), |n| n.name.clone())
        });
        let heading = semi(heading, 13.0).color(p.strong).wrapping(iced::widget::text::Wrapping::None);
        let title = column![row![heading, ic(icon::UNFOLD_MORE, 12.0).color(p.weak)].spacing(1.4).align_y(Center)]
            .push((!names.is_empty()).then(|| txt(names.join(", "), 10.0).color(p.sub)))
            .padding([0, 8]);
        let title = container(title).width(Fill).clip(true);
        let head_btn = |c: char, size: f32, [l, r]: [f32; 2], msg: UiMessage| {
            button(container(ic(c, size).color(p.strong)).padding(Padding::ZERO.left(l).right(r).top(2.0)).center_y(26))
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
            let pill = row![trigger, container(ic(icon::FILTER_LIST, 16.8).color(fg)).center_x(27).center_y(20)];
            let ring = if active { p.accent } else { CLEAR };
            let face = bordered_btn([CLEAR, p.hover, p.hover], [fg; 3], ring, 1.0, 12.0);
            let el = mouse_area(button(pill).padding(0).style(face).on_press(UiMessage::FilterButton))
                .on_right_press(UiMessage::ToggleFilterMenu);
            self.tip(el.into(), "Filter", vec![], tooltip::Position::Bottom)
        });
        let below = |el: El<'static>, label, keys| self.tip(el, label, keys, tooltip::Position::Bottom);
        let options = (self.active_tab == InboxTab::Reminders).then(|| {
            let btn = button(container(tinted(&TUNE, 18.0, p.strong)).center_x(30).center_y(26))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 8.0))
                .on_press(UiMessage::ReminderMenu);
            below(btn.into(), "Display Options", vec![])
        });
        let header = row![title]
            .push(filter)
            .push(options)
            .push(below(
                head_btn(icon::SEARCH, 23.0, [7.5, 10.0], UiMessage::ToggleCommandPalette).into(),
                "Search Chats",
                vec!["Ctrl", "K"],
            ))
            .push(below(
                head_btn(icon::EDIT_SQUARE, 20.0, [0.0, 3.0], UiMessage::NewChat).into(),
                "New Chat",
                vec!["Ctrl", "N"],
            ))
            .align_y(Center)
            .height(52);
        let selected = self.creating_chat.is_none().then_some(self.selected_chat_id.as_deref()).flatten();
        let (pinned, rest) = self.pins_and_rows(&st);
        let counts = st.settings.on("showUnreadCount");
        let network_icons = st.settings.on("showNetworkIcons") && self.selected_account_filter.is_none();
        let colorize = st.settings.on("colorizeNetworkIcons");
        let hover_action = st.settings.pref("chatActionButton");
        let selecting = !self.selection.is_empty();
        let picked = |c: &Chat| self.selection.contains(&c.id);
        let pick = |c: &Chat| match selecting {
            true => UiMessage::ToggleSelected(c.id.clone()),
            false => UiMessage::ClickChat(c.id.clone()),
        };
        let cols = (((w - 16.0) / 80.0).floor() as usize).max(1);
        let cell = (w - 16.0) / cols as f32;
        let pins = pinned.chunks(cols).fold(Column::new(), |col, chunk| {
            col.push(chunk.iter().fold(Row::new(), |r, c| {
                let on = selected == Some(c.id.as_str());
                let muted = c.is_muted == Some(true);
                let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
                let quiet = muted && c.unread_mentions == 0;
                let (bg, fg) = if quiet { (p.frame, p.text) } else { (p.accent, Color::WHITE) };
                let dot =
                    (!counts && unread).then(|| container(Space::new()).width(6).height(6).style(fill_box(bg, 3.0)));
                let title_w = cell - if muted { 35.2 } else { 20.0 } - if dot.is_some() { 9.2 } else { 0.0 };
                let title = ell(short_title(c), title_w, 12.0, Weight::Semibold);
                let label = row![]
                    .push(dot)
                    .push(title.color(p.strong).line_height(iced::widget::text::LineHeight::Absolute(15.0.into())))
                    .push(muted.then(|| tinted(&BELL_OFF, 12.0, p.strong)))
                    .spacing(3.2)
                    .align_y(Center);
                let unread = match (st.typing.contains_key(&c.id), counts) {
                    (true, _) => Some(self.typing_badge()),
                    (false, true) => unread.then(|| count_badge(c, bg, fg, 14.0)),
                    (false, false) => (c.unread_mentions > 0).then(|| {
                        let at = ic(icon::AT_SIGN, 10.0).color(fg);
                        container(at).padding([0, 4]).center_y(14).style(fill_box(bg, 7.0)).into()
                    }),
                };
                let face = stack![self.chat_avatar(c, pin_av)]
                    .push((!selecting).then(|| container(unread).width(pin_av + 4.0).align_right(pin_av + 4.0)))
                    .push(selecting.then(|| pin(checkbox(p, picked(c))).x(pin_av - 11.0).y(-5.0)))
                    .width(pin_av + 4.0)
                    .height(pin_av);
                let item =
                    column![face, label].spacing(if compact { 4 } else { 8 }).align_x(Center).width(cell).padding(
                        match compact {
                            true => Padding::new(10.0).top(8.0).bottom(6.0),
                            false => Padding::new(12.0).bottom(10.0),
                        },
                    );
                r.push(
                    mouse_area(
                        button(item)
                            .padding(0)
                            .style(btn_style(
                                [if on { p.selected } else { CLEAR }, p.hover, p.selected],
                                [p.text; 3],
                                8.0,
                            ))
                            .on_press(pick(c)),
                    )
                    .on_right_press(UiMessage::OpenChatMenu(c.id.clone())),
                )
            }))
        });
        let lead = match (selecting, counts) {
            (true, _) => 16.0,
            (false, true) => 0.0,
            (false, false) => 12.0,
        };
        let text_w = w - 8.0 - 16.0 - av - 8.0 - 4.0 - lead;
        let frame = |key: String, dot: Option<El<'static>>, face: El<'static>, lines: [El<'static>; 2], on, select| {
            let [top, preview] = lines;
            let item = row![]
                .push(dot)
                .push(container(face).padding(Padding::ZERO.right(8.5)))
                .push(column![top, preview].width(Fill))
                .align_y(Center)
                .padding(Padding::new(8.0).top(10.0).bottom(9.0).left(if selecting { 0.0 } else { 8.0 }));
            let action =
                (!selecting && self.hovered_chat.as_ref() == Some(&key)).then(|| match key.strip_prefix("draft:") {
                    Some(n) => Some(self.hover_btn(
                        5,
                        20.0,
                        "Delete Chat",
                        vec![],
                        UiMessage::DeleteNewChat(n.parse().unwrap()),
                    )),
                    None => self.row_action(&st.chats[st.resolve_chat_idx(&key).expect("listed chat")], hover_action),
                });
            let row = mouse_area(
                button(stack![item].push(action.flatten()))
                    .padding(0)
                    .width(Fill)
                    .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.text; 3], 8.0))
                    .on_press(select),
            )
            .on_enter(UiMessage::HoverChat(key.clone(), true))
            .on_exit(UiMessage::HoverChat(key.clone(), false));
            let inbox = st.resolve_chat_idx(&key).is_some_and(|i| st.chats[i].flags & CHAT_ARCHIVED == 0);
            let row = match inbox {
                true => row.on_middle_press(UiMessage::ToggleFlag(key.clone(), ChatFlag::Archived)),
                false => row,
            }
            .on_right_press(UiMessage::OpenChatMenu(key));
            column![row, divider(p.divider, av + 18.0)]
        };
        let drafts = self.new_chats.iter().map(|&n| {
            let dot = (!counts).then(|| container(Space::new()).width(12).into());
            let title = ell("New Chat", text_w, title_size, if counts { Weight::Medium } else { Weight::Semibold });
            let rows =
                [container(title.color(p.strong)).height(16).into(), Space::new().height(16 * lines as u32).into()];
            let face = self.face(None, "", av);
            frame(format!("draft:{n}"), dot, face, rows, self.creating_chat == Some(n), UiMessage::SelectNewChat(n))
        });
        let rows = rest.iter().fold(Column::with_children(drafts.map(El::from)), |col, c| {
            let on = selected == Some(c.id.as_str());
            let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
            let muted = c.is_muted == Some(true);
            let m = st.chat_msg_indices.get(&c.id).and_then(|v| {
                v.iter().rev().map(|&i| &st.messages[i]).find(|m| m.is_deleted != Some(true) && !silent(m))
            });
            let reaction = c
                .preview
                .as_ref()
                .filter(|p| p.message_type.as_deref() == Some("REACTION") && m.is_none_or(|m| m.sort_key < p.sort_key));
            let m = reaction.or(m);
            let ts = match (self.active_tab, &c.reminder) {
                (InboxTab::Reminders, Some(r)) => {
                    format!("{} {}", reminder_day(r.remind_at_ms), clock(r.remind_at_ms, h24))
                }
                _ => c.last_activity.as_deref().map(|t| list_ts(t, h24)).unwrap_or_default(),
            };
            let ts_w = crate::style::text_width(&ts, ts_size, Weight::Normal) + 8.0;
            let expired = self.expired(c);
            let reminded_last = c
                .reminder
                .as_ref()
                .zip(c.last_activity.as_deref())
                .is_some_and(|(r, t)| millis_from_rfc3339(t).expect("RFC 3339 timestamp") <= r.remind_at_ms);
            let style = match expired && reminded_last {
                true => iced::font::Style::Italic,
                false => iced::font::Style::Normal,
            };
            let title = c.network_name.as_ref().map_or_else(|| c.title.clone(), |n| format!("{} • {n}", c.title));
            let weight = if counts && !unread { Weight::Medium } else { Weight::Semibold };
            let bell = if muted { 16.0 } else { 0.0 };
            let pinned = c.is_pinned == Some(true);
            let pin_icon = pinned.then(|| container(tinted(&PIN, 13.0, p.strong)).padding(Padding::ZERO.right(2.0)));
            let pin_w = if pinned { 15.0 } else { 0.0 };
            let top = row![]
                .push(pin_icon)
                .push(ell(&title, text_w - ts_w - bell - pin_w, title_size, weight).color(p.strong))
                .push(muted.then(|| container(tinted(&BELL_OFF, 12.0, p.strong)).padding([0, 2])))
                .push(Space::new().width(Fill))
                .push(txt(ts, ts_size).color(p.sub).font(iced::Font { style, ..fw(Weight::Normal) }))
                .height(16)
                .align_y(Center);
            let mine = m.is_some_and(|m| m.is_sender == Some(true));
            let media_tag = m.and_then(|m| {
                m.attachments.iter().find_map(|a| match a.attachment_type.as_str() {
                    "img" if a.is_gif == Some(true) => Some((false, "GIF")),
                    "img" => Some((false, "Image")),
                    "video" if a.is_gif == Some(true) => Some((false, "GIF")),
                    "video" => Some((true, "Video")),
                    _ => None,
                })
            });
            let body = m.and_then(|m| match m.links.len() {
                _ if m.html.is_some() => m.html.as_deref().map(beeper_core::codec::html_text),
                _ if m.text.as_ref().is_some_and(|t| !t.is_empty()) => m.text.clone(),
                _ if !m.attachments.is_empty() => None,
                0 => None,
                1 => Some("🔗 [Link]".to_string()),
                n => Some(format!("🔗 [{n} links]")),
            });
            let body = body.filter(|_| previews);
            let show_sender = previews
                && !mine
                && reaction.is_none()
                && (media_tag.is_some() || body.is_some())
                && (c.chat_type == "group" || m.is_some_and(|m| network_bot(c, &m.sender_id)));
            let first = show_sender.then(|| m?.sender_name.as_deref()?.split(' ').next()).flatten();
            let body = body.map(|t| match (media_tag.is_none(), first) {
                (true, Some(s)) => format!("{s}: {t}"),
                _ => t,
            });
            let dot_bg = if muted { Color { a: 0.3, ..p.strong } } else { p.accent };
            let count = match counts {
                true => unread.then(|| count_badge(c, dot_bg, Color::WHITE, 12.0)),
                false => (c.unread_mentions > 0).then(|| ic(icon::AT_SIGN, 12.0).color(p.accent).into()),
            };
            let dot = (!counts).then(|| {
                let dot = container(Space::new()).width(6).height(6).style(fill_box(dot_bg, 3.0));
                container(unread.then_some(dot)).width(12)
            });
            let count = count
                .filter(|_| self.hovered_chat.as_ref() != Some(&c.id) || self.row_action(c, hover_action).is_none());
            let sender = show_sender.then(|| preview_sender(c, m?)).flatten();
            let digits = if counts && c.unread_count > 0 { c.unread_count.to_string().len() as f32 } else { 0.0 };
            let at = if c.unread_mentions > 0 { 12.0 } else { 0.0 };
            let prefix = first.filter(|_| media_tag.is_some()).map(|s| format!("{s}:"));
            let prefix_w = prefix.as_deref().map_or(0.0, |s| crate::style::text_width(s, 13.0, Weight::Normal) + 4.0);
            let reserve = 16.0
                + if counts { 8.0 + 6.0 * digits + at } else { at }.max(18.0)
                + if mine { 16.0 } else { 0.0 }
                + if sender.is_some() { 16.0 } else { 0.0 }
                + match media_tag {
                    Some((_, label)) if expired => crate::style::text_width(label, 13.0, Weight::Normal) + 4.0,
                    Some(_) => 52.0,
                    None => 0.0,
                }
                + if expired { crate::style::text_width("Reminder", 11.0, Weight::Semibold) + 10.0 } else { 0.0 }
                + prefix_w;
            let read = m.is_some_and(|m| m.seen.keys().any(|u| !st.is_me(u) && u != &m.sender_id));
            let fg = if unread { p.text } else { p.sub };
            let face =
                sender.map(|u| self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 12.0));
            let badge = expired.then(|| {
                let label = semi("Reminder", 11.0).line_height(Pixels(14.0)).color(Color::WHITE.scale_alpha(0.7));
                container(label).padding([0, 3]).style(fill_box(ORANGE.scale_alpha(0.7), 4.0))
            });
            let preview = row![]
                .push(badge)
                .push(mine.then(|| tinted(&SENT[usize::from(read)], 12.0, p.sub)))
                .push(face)
                .push(prefix.map(|s| txt(s, 13.0).color(fg)))
                .push(media_tag.map(|(is_video, label)| -> El<'_> {
                    if expired {
                        return txt(label, 13.0).color(fg).into();
                    }
                    let glyph: El<'_> = match is_video {
                        true => tinted(&VIDEO_TAG, 12.0, fg),
                        false => ic(icon::IMAGE, 12.0).color(fg).into(),
                    };
                    container(row![glyph, med(label, 11.0).color(fg)].spacing(2).align_y(Center))
                        .padding([1, 4])
                        .style(fill_box(p.strong.scale_alpha(0.12), 4.0))
                        .into()
                }))
                .push(body.map(|b| {
                    clamp(&b, text_w - reserve, lines, 13.0, Weight::Normal)
                        .width(text_w - reserve)
                        .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                        .color(fg)
                }))
                .push(Space::new().width(Fill))
                .push(count)
                .spacing(4)
                .align_y(if compact { Center } else { iced::Alignment::Start });
            let avatar = self.chat_avatar(c, av);
            let avatar = match network_icons {
                true => badged(avatar, &c.account_id, av, if compact { 12.0 } else { 16.0 }, (!colorize).then_some(p)),
                false => avatar,
            };
            let face = stack![avatar]
                .push(st.typing.contains_key(&c.id).then(|| pin(self.typing_badge()).x(av - 14.0).y(-2.0)));
            let dot = match selecting {
                true => Some(container(checkbox(p, picked(c))).padding(Padding::ZERO.right(8.0)).into()),
                false => dot.map(El::from),
            };
            col.push(frame(
                c.id.clone(),
                dot,
                face.into(),
                [top.into(), preview.height(16 * lines as u32).into()],
                on,
                pick(c),
            ))
        });
        let list: El<'_> = if chats.is_empty() && self.new_chats.is_empty() {
            let tab = if self.selected_account_filter.is_some() { InboxTab::Requests } else { self.active_tab };
            let (head, hint) = tab.empty();
            let header = column![semi(head, 16.0).color(p.strong).align_x(Center).line_height(1.25)]
                .push((!hint.is_empty()).then(|| txt(hint, 13.0).color(p.strong).align_x(Center)))
                .spacing(12)
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
            scrollable(column![pins, rows].padding(Padding::ZERO.right(if SOLID_BARS { 0.0 } else { 4.0 })))
                .direction(bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .into()
        };
        container(column![header, list].padding(Padding::ZERO.left(8.0))).width(w).height(Fill).into()
    }

    fn view_conversation(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let cid = self.selected_chat_id.as_deref().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
        let (hide, members) = (st.settings.on("hideDeleted"), st.settings.on("membershipEvents"));
        let msgs: Vec<&Message> = st.chat_msg_indices.get(cid).map_or_else(Vec::new, |v| {
            v.iter()
                .map(|&i| &st.messages[i])
                .filter(|m| !(hide && m.is_deleted == Some(true)) && (members || !silent(m)))
                .collect()
        });
        let mut run_start: Vec<usize> = (0..msgs.len()).collect();
        for i in 1..msgs.len() {
            let (a, b) = (msgs[i - 1], msgs[i]);
            if silent(a) && silent(b) && day_num(&a.timestamp) == day_num(&b.timestamp) {
                run_start[i] = run_start[i - 1];
            }
        }
        let mut col = Column::new().padding(Padding::new(8.0).right(8.0 + GUTTER).top(96.0).bottom(8.0));
        let mut prev: Option<&Message> = None;
        let last_live = msgs.iter().rposition(|m| m.is_deleted != Some(true));
        let mut last_seen: HashMap<&str, (&str, &str)> = HashMap::new();
        let mut replied: HashSet<&str> = HashSet::new();
        for m in msgs.iter().rev() {
            for (uid, ts) in &m.seen {
                if !st.is_me(uid) && uid != &m.sender_id && !replied.contains(uid.as_str()) {
                    last_seen.entry(uid.as_str()).or_insert((m.id.as_str(), ts.as_str()));
                }
            }
            if m.is_deleted != Some(true) && m.is_sender != Some(true) {
                replied.insert(m.sender_id.as_str());
            }
        }
        let first_unread = msgs.iter().position(|m| m.is_deleted != Some(true) && m.is_unread == Some(true));
        let (h24, group) = (st.settings.pref("timeFormat") == 1, chat.chat_type == "group");
        for (i, m) in msgs.iter().enumerate() {
            let day_changed = prev.is_none_or(|q| day_num(&q.timestamp) != day_num(&m.timestamp));
            let mut head = Column::new().push(prev.map(|q| Space::new().height(gap_below(q, m))));
            if day_changed {
                let label = semi(day_label(&m.timestamp), 11.0).line_height(Pixels(14.0)).color(p.date_text);
                let pill = container(label).padding([5, 9]).style(bordered_box(p.card, p.outline, 1.0, 12.0));
                head = head.push(container(pill).center_x(Fill).padding([4, 0]));
            }
            if Some(i) == first_unread {
                let line =
                    || container(Space::new()).width(Fill).height(1).style(fill_box(p.accent.scale_alpha(0.35), 0.0));
                let div =
                    row![line(), semi("New", 11.0).color(p.accent), line()].spacing(8).align_y(Center).padding([6, 0]);
                head = head.push(div);
            }
            if silent(m) {
                let s = run_start[i];
                let size = run_start[s..].iter().take_while(|&&r| r == s).count();
                let open = self.open_members.contains(&msgs[s].id);
                let dim = p.muted.scale_alpha(0.75);
                if size >= 3 && i == s {
                    let label = row![
                        txt(format!("{size} Membership Events"), 11.0).line_height(Pixels(14.0)).color(dim),
                        txt(if open { "Collapse" } else { "Expand" }, 11.0).color(p.accent.scale_alpha(0.75))
                    ];
                    let toggle = button(label.spacing(4))
                        .padding(0)
                        .style(btn_style([Color::TRANSPARENT; 3], [dim; 3], 0.0))
                        .on_press(UiMessage::ToggleMembers(msgs[s].id.clone()));
                    col = col.push(head.push(container(toggle).center_x(Fill)));
                    head = Column::new();
                }
                if size < 3 || open {
                    let gap = if size >= 3 { Space::new().height(6) } else { Space::new() };
                    let line = txt(m.text.clone().expect("membership text"), 11.0).line_height(Pixels(14.0));
                    let line = container(line.color(dim).align_x(Center)).center_x(Fill).padding([2, 0]);
                    col = col.push(head.push(gap).push(line));
                }
                prev = Some(m);
                continue;
            }
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id || silent(q));
            let last = msgs.get(i + 1).is_none_or(|n| {
                n.sender_id != m.sender_id || silent(n) || day_num(&n.timestamp) != day_num(&m.timestamp)
            });
            let mut viewers: Vec<(&str, &str)> =
                last_seen.iter().filter(|(_, &(mid, _))| mid == m.id).map(|(&u, &(_, ts))| (u, ts)).collect();
            viewers.sort_by_key(|&(_, ts)| ts);
            let status = if let Some(&(_, ts)) = viewers.last() {
                if group {
                    let names: Vec<&str> = viewers
                        .iter()
                        .map(|&(uid, _)| {
                            chat.participants
                                .items
                                .iter()
                                .find(|u| u.id == uid)
                                .and_then(|u| u.custom_nickname.as_deref().or(u.full_name.as_deref()))
                                .unwrap_or(uid)
                        })
                        .collect();
                    Some(if names.len() <= 4 {
                        format!("Seen by {}", names.join(", "))
                    } else {
                        format!("Seen by {} & {} others", names[..3].join(", "), names.len() - 3)
                    })
                } else {
                    let verb =
                        if matches!(m.message_type.as_deref(), Some("VOICE" | "AUDIO")) { "Played" } else { "Seen" };
                    let when = match day_label(ts).as_str() {
                        "Today" => format!("at {}", format_short_ts(ts, h24)),
                        other => other.into(),
                    };
                    Some(format!("{verb} {when}"))
                }
            } else {
                (Some(i) == last_live && m.is_sender == Some(true)).then(|| "Sent".into())
            };
            let chained = prev.is_some_and(|q| reply_root(&st, q) == reply_root(&st, m));
            let view = self.view_message(&st, chat, m, (first, last, chained), status, head);
            let view = match send_rise(&st, m) {
                Some(t) => lift::lift(view, 14.0 * (1.0 - t), 0.97 + 0.03 * t).into(),
                None => view,
            };
            let flash = self.flash.as_ref().filter(|(id, _)| *id == m.id).map(|(_, at)| {
                let t = (at.elapsed().as_secs_f32() / 1.5).min(1.0);
                let a = if t < 0.2 { ease_out(t / 0.2) } else { 1.0 - ease_out((t - 0.2) / 0.8) };
                container(Space::new()).width(Fill).height(Fill).style(fill_box(p.accent.scale_alpha(0.4 * a), 0.0))
            });
            col = col.push(container(stack![view].push(flash)).id(format!("msg-{}", m.id)));
            prev = Some(m);
        }
        col = col.push(st.typing.get(cid).map(|ids| self.typing_bubble(chat, ids)));
        let list = scrollable(col)
            .id("timeline")
            .direction(stable_bar())
            .anchor_bottom()
            .height(Fill)
            .style(scroll_style(p.scroll));
        let name_row = row![
            semi(short_title(chat).to_string(), 13.0).line_height(Pixels(16.0)).color(p.fg),
            ic(icon::CHEVRON_RIGHT, 16.0).color(p.tertiary)
        ]
        .spacing(2)
        .align_y(Center);
        let pill = button(name_row)
            .padding(Padding::new(6.0).left(10.5).right(2.5))
            .style(btn_style([p.elevated, p.selected, p.selected], [p.text; 3], 14.0))
            .on_press(UiMessage::ToggleRightPanel);
        let ring = container(self.chat_avatar(chat, 40.0)).padding(1).style(fill_box(p.card, 21.0));
        let header = stack![
            container(pill).center_x(Fill).padding(Padding::ZERO.top(44.0)),
            container(ring).center_x(Fill).padding(Padding::ZERO.top(8.0))
        ];
        let rim = Color { a: 0.1, ..p.strong };
        let feedback = button(container(txt("Feedback?", 11.0)).center(Fill))
            .width(74)
            .height(24)
            .padding(0)
            .style(bordered_btn([p.card, p.hover, p.card], [Color { a: 0.9, ..rim }; 3], rim, 1.0, 4.0))
            .on_press(UiMessage::OpenQuickLook("Feedback".into(), "Tell us what to improve in beeper-rs.".into()));
        let fade = Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, p.card)
            .add_stop(0.75, p.card.scale_alpha(0.85))
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let fade = container(Space::new()).width(Fill).height(84).style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fade))),
            ..Default::default()
        });
        let reminder = chat.reminder.as_ref().filter(|_| !self.expired(chat)).map(|r| {
            let (day, time) =
                (reminder_day(r.remind_at_ms), clock(r.remind_at_ms, st.settings.pref("timeFormat") == 1));
            let at = if day == "Today" { time.clone() } else { format!("{time} {day}") };
            let tip =
                format!("Set to remind at {at}{}", if r.dismiss_on_incoming_message { " if no reply" } else { "" });
            let label = row![ic(icon::SCHEDULE, 16.0).color(ORANGE), semi(format!("{day} {time}"), 11.0).color(ORANGE)];
            let pill = container(label.spacing(4).align_y(Center))
                .padding(Padding { top: 2.0, right: 8.0, bottom: 2.0, left: 4.0 })
                .style(fill_box(mix(p.card, ORANGE, 0.1), 30.0));
            container(self.tip(pill.into(), tip, vec![], tooltip::Position::Bottom))
                .padding(Padding::ZERO.top(9.0).left(8.0))
        });
        let feedback = st
            .settings
            .on("feedbackWidget")
            .then(|| container(feedback).align_right(Fill).padding(Padding::ZERO.top(6.0).right(5.0)));
        let overlay = stack![fade, header].push(feedback).push(reminder);
        let verify = (st.matrix.is_some() && !st.beeper_state.verified).then(|| {
            let note = txt("Verify this device to read your encrypted messages.", 12.0).color(p.text);
            let go = pill_btn(p, "Verify", UiMessage::OpenSettings(SettingsTab::Devices));
            let bar = row![note, Space::new().width(Fill), go].align_y(Center);
            container(bar).padding([8, 14]).style(bordered_box(p.elevated, p.outline, 1.0, 10.0))
        });
        let verify = verify.map(|v| container(v).padding([0, 12]));
        let pane =
            column![stack![list, overlay].width(Fill).height(Fill)].push(verify).push(self.view_composer(&st, chat));
        let secondary = st.settings.pref("secondaryComposerButton") != 6;
        let picker =
            self.view_emoji_picker().filter(|_| self.emoji.as_ref().is_some_and(|pk| pk.react.is_none())).map(|e| {
                let right = if secondary { 112.0 } else { 82.0 };
                container(e).align_right(Fill).align_bottom(Fill).padding(Padding::ZERO.right(right).bottom(36.0))
            });
        let pane = stack![pane].push(picker).push(self.view_suggestions(&st));
        let shadow = Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.18), offset: Vector::ZERO, blur_radius: 20.0 };
        let card = fill_box(p.card, 10.0);
        container(pane)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(move |t| container::Style { shadow, ..card(t) })
            .into()
    }

    fn view_message<'a>(
        &self,
        st: &BeeperStore,
        chat: &Chat,
        m: &Message,
        (first, last, chained): (bool, bool, bool),
        status: Option<String>,
        head: Column<'a, UiMessage>,
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
        let selected = self.selected_msg.as_ref() == Some(&m.id);
        let rise = send_rise(st, m).unwrap_or(1.0);
        let dim = move |c: Color| (if selected { brightness(c, 0.7) } else { c }).scale_alpha(rise);
        let (bg, fg) = if mine { (dim(p.sent), dim(Color::WHITE)) } else { (dim(p.received), dim(p.strong)) };
        let text = m.text.clone().unwrap_or_default();
        let preview = st
            .settings
            .on("loadEmbeds")
            .then(|| {
                let link = m.links.first().map(|l| (l.url.as_str(), l.title.as_str(), l.summary.as_deref()));
                link.or_else(|| {
                    text.split_whitespace().find_map(|w| {
                        let lp = st.link_previews.get(w)?;
                        Some((w, lp.title.as_str(), Some(lp.summary.as_str())))
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
            ((!chained).then(|| col.align_x(if q_mine { Right } else { iced::Left })), q_mine)
        });
        if let Some((url, title, summary)) = preview {
            let host = url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(url).to_string();
            let body =
                column![]
                    .push(clamp(title, 338.0, 3, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(dim(p.strong)))
                    .push(summary.map(|s| {
                        clamp(s, 338.0, 3, 13.0, Weight::Normal).line_height(Pixels(16.0)).color(dim(p.strong))
                    }))
                    .push(
                        row![
                            ic(icon::LINK, 12.0).color(dim(p.weak)),
                            txt(host, 11.0).line_height(Pixels(14.0)).color(dim(p.weak))
                        ]
                        .spacing(4)
                        .align_y(Center),
                    )
                    .spacing(8)
                    .padding([8, 11]);
            let wraps = text_width(title, 13.0, Weight::Semibold) > 338.0
                || summary.is_some_and(|s| text_width(s, 13.0, Weight::Normal) > 338.0);
            let card = container(body).width(if wraps { Length::Fixed(360.0) } else { Shrink });
            parts = parts.push(card.max_width(360).style(fill_box(dim(p.link_card), 17.0)));
        }
        for (ai, a) in m.attachments.iter().enumerate() {
            let name = a.file_name.clone().expect("attachment file name");
            let content: El<'a> = match (a.is_voice_note == Some(true), a.attachment_type.as_str()) {
                (_, "audio") if self.playable(a) => self.view_audio(m, a, mine, fg, bg),
                (false, "img" | "video") => {
                    let is_video = a.attachment_type == "video";
                    let (w, h) = a.size.as_ref().map_or((240.0, 160.0), |s| {
                        let (w, h) = (s.width.unwrap_or(240) as f32, s.height.unwrap_or(160) as f32);
                        let k = (360.0 / w).min(400.0 / h).min(1.0);
                        (w * k, h * k)
                    });
                    let thumb_url = a.poster_img.clone().or_else(|| a.src_url.clone());
                    let pill = |label: &'a str, alpha: f32| -> El<'a> {
                        container(
                            container(txt(label, 14.0).line_height(1.0).color(Color::WHITE))
                                .center_y(36)
                                .padding([0, 17])
                                .style(fill_box(p.toggle_on, 18.0)),
                        )
                        .center_x(w)
                        .center_y(h)
                        .style(fill_box(Color::BLACK.scale_alpha(alpha), 17.0))
                        .into()
                    };
                    let playing = a.src_url.as_ref().and_then(|u| {
                        let (pu, state) = self.playing_video.as_ref()?;
                        (pu == u).then_some(state)
                    });
                    let (slot, press): (El<'_>, _) = if let Some(Some(Ok((frames, started)))) = playing {
                        (
                            image(current_frame(frames, started.elapsed().as_millis()))
                                .width(w)
                                .height(h)
                                .border_radius(17.0)
                                .into(),
                            UiMessage::PlayVideo(
                                a.src_url.clone().unwrap_or_default(),
                                w.round() as u32,
                                h.round() as u32,
                            ),
                        )
                    } else {
                        let base: El<'_> = match thumb_url.as_ref().and_then(|u| self.media.get(u)?.as_ref()) {
                            Some(Ok(frames)) => image(current_frame(frames, self.started.elapsed().as_millis()))
                                .width(w)
                                .height(h)
                                .border_radius(17.0)
                                .into(),
                            Some(Err(_)) if m.unresolved_media.is_none() => {
                                pill("Could not load. Click to try again", 0.45)
                            }
                            _ => container(ic(icon::IMAGE, 28.0).color(p.sub))
                                .center_x(w)
                                .center_y(h)
                                .style(fill_box(p.link_card, 17.0))
                                .into(),
                        };
                        if let Some(raw) = m.unresolved_media.as_ref().filter(|_| !is_video) {
                            let failed = self.resolving_media.get(&m.id) == Some(&false);
                            let (label, alpha) = match failed {
                                true => ("Could not load. Click to try again", 0.45),
                                false => ("Click to load", 0.25),
                            };
                            (
                                stack![base, pill(label, alpha)].into(),
                                UiMessage::ResolveMedia(m.account_id.clone(), m.id.clone(), raw.clone()),
                            )
                        } else if thumb_url
                            .as_ref()
                            .and_then(|u| self.media.get(u)?.as_ref())
                            .is_some_and(Result::is_err)
                        {
                            (base, UiMessage::RetryMedia(thumb_url.expect("a failed media has a URL")))
                        } else if is_video && a.is_gif != Some(true) {
                            let play = container(
                                container(tinted(&PLAY_FILL, 20.0, Color::from_rgb8(0x20, 0x21, 0x24)))
                                    .center(36)
                                    .style(fill_box(Color::from_rgb8(0xef, 0xef, 0xf0), 18.0)),
                            )
                            .center_x(w)
                            .center_y(h);
                            (
                                stack![base, play].into(),
                                a.src_url.clone().map_or_else(
                                    || UiMessage::OpenQuickLook(name, "Video attachment".into()),
                                    |u| UiMessage::PlayVideo(u, w.round() as u32, h.round() as u32),
                                ),
                            )
                        } else {
                            (base, UiMessage::OpenCarousel(m.id.clone(), ai, false))
                        }
                    };
                    button(slot).padding(0).style(btn_style([CLEAR; 3], [p.text; 3], 17.0)).on_press(press).into()
                }
                _ => {
                    let meta = file_label(a).into_iter().chain(a.file_size.map(human_size)).collect::<Vec<_>>();
                    let info = column![clamp(&name, 206.0, 2, 14.0, Weight::Semibold).line_height(Pixels(17.0))]
                        .push(
                            (!meta.is_empty())
                                .then(|| txt(meta.join(" · "), 12.0).line_height(Pixels(15.0)).color(p.sub)),
                        )
                        .spacing(4);
                    let line = row![tinted(file_icon(a), 24.0, p.strong), info].spacing(8).align_y(Center);
                    button(container(line).max_width(238))
                        .padding([8, 11])
                        .style(btn_style([p.received; 3], [p.strong; 3], 17.0))
                        .on_press(UiMessage::Download(m.id.clone(), Some(ai)))
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
            let ts = st
                .settings
                .on("showTimestamps")
                .then(|| format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1));
            let room = ts.as_deref().map_or(0.0, |t| crate::style::text_width(t, 11.0, Weight::Normal) + 8.0);
            let time = ts.map(|t| txt(t, 11.0).color(if mine { p.sent_meta } else { p.sub }));
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
            let nbsp = crate::style::text_width("\u{a0}", 13.0, Weight::Normal);
            let pad = "\u{a0}".repeat((room / nbsp).ceil() as usize);
            let balanced = (st.settings.on("balanceText") && last == 0 && blocks[0].0.is_none())
                .then(|| {
                    crate::style::balanced_width(&format!("{text}{pad}"), 0.45 * self.main_size.width - 22.0, 13.0)
                })
                .flatten()
                .map(|w| w.ceil() + 1.0);
            let body = Column::with_children(blocks.into_iter().enumerate().map(|(i, (marker, mut spans))| {
                // iced does not draw an empty last line.
                if spans.last().is_some_and(|s| s.text.ends_with('\n')) {
                    spans.push(span("\u{200b}").size(13.0));
                }
                if i == last && room > 0.0 {
                    spans.push(span(pad.clone()).size(13.0));
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
            }))
            .width(balanced.map_or(Length::Shrink, Length::Fixed));
            parts = parts.push(
                container(stack![body].push(time.map(|t| container(t).align_right(Fill).align_bottom(Fill))))
                    .padding([8, 11])
                    .max_width(0.45 * self.main_size.width)
                    .style(fill_box(bg, 17.0)),
            );
        }
        let open = self.expanded_history_msg_id.as_ref() == Some(&m.id);
        let label = |t: String| txt(t, 12.0).line_height(Pixels(15.0)).color(p.muted);
        let edited = (!m.edit_history.is_empty()).then(|| {
            let toggle = button(label(if open { "Hide Edits" } else { "Edited" }.into())).padding(0);
            toggle.style(|_, _| button::Style::default()).on_press(UiMessage::ToggleEditHistory(m.id.clone())).into()
        });
        let footer: Vec<El<'a>> = status.map(|t| label(t).into()).into_iter().chain(edited).collect();
        let footer = (!footer.is_empty()).then(|| {
            let line = footer.into_iter().enumerate().fold(Row::new(), |r, (i, t)| {
                r.push((i > 0).then(|| container(txt("·", 12.0).color(p.muted.scale_alpha(0.8))).padding([0, 4])))
                    .push(t)
            });
            container(line).padding([0.0, BUBBLE_PAD_X])
        });
        let tail = if footer.is_some() { 15.0 } else { 0.0 } + if m.reactions.is_empty() { 0.0 } else { 21.0 };
        let edits = open.then(|| {
            m.edit_history.iter().fold(Column::new().spacing(2), |c, e| {
                c.push(txt(format!("{} · {}", e.text, format_short_ts(&e.timestamp, false)), 11.0).color(p.muted))
            })
        });
        let parts: El<'a> = if m.reactions.is_empty() {
            parts.into()
        } else {
            let mut keys: Vec<&str> = m.reactions.iter().map(|r| r.reaction_key.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            let min_w = keys.len() as f32 * 36.0 + 2.0 * BUBBLE_PAD_X;
            let order = keys.iter().flat_map(|&k| m.reactions.iter().filter(move |x| x.reaction_key == k));
            let all: Vec<_> = order.collect();
            let (shown, more) = if all.len() <= 20 { (&all[..], 0) } else { (&all[..19], all.len() - 19) };
            let list = shown.iter().fold(Column::new(), |col, x| {
                let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
                let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
                let line = row![
                    container(txt(x.reaction_key.clone(), 17.0)).padding(Padding::ZERO.right(10.0)),
                    container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0))
                        .padding(Padding::ZERO.right(8.0)),
                    med(name.to_string(), 13.0).color(p.text),
                ]
                .align_y(Center);
                col.push(container(line).padding([4, 8]))
            });
            let list = list.push((more > 0).then(|| container(txt(format!("and {more} more"), 13.0)).padding([4, 8])));
            let list = container(list).padding([2, 4]).style(bordered_box(p.menu, p.menu_border, 1.0, 6.0));
            let pills = keys.into_iter().fold(Row::new().spacing(2), |r, k| {
                let reactors: Vec<_> = m.reactions.iter().filter(|x| x.reaction_key == k).collect();
                let n = reactors.len();
                let own = reactors.iter().any(|x| st.is_me(&x.participant_id));
                let mut pill = row![txt(k.to_string(), 16.0).line_height(Pixels(15.0))].spacing(4).align_y(Center);
                if group || n > 1 {
                    for x in reactors.iter().take(3) {
                        let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
                        let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
                        pill = pill.push(self.face(u.and_then(|u| u.img_url.as_deref()), name, 16.0));
                    }
                    if n > 3 {
                        pill = pill.push(semi(format!("+{}", n - 3), 11.0).color(p.on_secondary));
                    }
                }
                let fill = if own {
                    if p.card.r < 0.5 {
                        hex(0x1b43aa)
                    } else {
                        hex(0xcbd9ff)
                    }
                } else {
                    p.received
                };
                r.push(
                    button(pill)
                        .padding([6, 7])
                        .style(bordered_btn([fill, p.selected, p.selected], [p.text; 3], p.card, 2.0, 14.0))
                        .on_press(UiMessage::ToggleReaction(chat.id.clone(), m.id.clone(), k.to_string())),
                )
            });
            stack![
                container(column![parts, Space::new().width(min_w)].align_x(if mine { Right } else { iced::Left }))
                    .padding(Padding::ZERO.bottom(21.0)),
                container(crate::popover::popover(
                    pills,
                    list,
                    self.selected_msg.as_ref() == Some(&m.id) && st.settings.on("reactionsOnSelect"),
                ))
                .align_bottom(Fill)
                .width(Fill)
                .align_x(if mine { Right } else { iced::Left })
                .padding([0.0, BUBBLE_PAD_X]),
            ]
            .into()
        };
        let parts = column![parts].push(edits).push(footer).align_x(if mine { Right } else { iced::Left });
        let tools = (self.hovered_msg.as_ref() == Some(&m.id)).then(|| {
            let muted = p.muted;
            let b = |c: char, msg: UiMessage| -> El<'a> {
                let style = move |_: &Theme, s| button::Style {
                    text_color: match s {
                        button::Status::Hovered => brightness(muted, 0.8),
                        _ => muted,
                    },
                    ..Default::default()
                };
                button(container(ic(c, 20.0)).center(32)).padding(0).style(style).on_press(msg).into()
            };
            let reply = b(icon::REPLY, UiMessage::StartReply(m.id.clone()));
            let react = mouse_area(b(icon::SMILE, UiMessage::React(m.id.clone())))
                .on_enter(UiMessage::ShowQuickReactions)
                .into();
            let quick =
                (self.quick_open && st.settings.on("quickReactions")).then(|| crate::emoji::quick_reactions(st));
            let quick = quick.into_iter().flatten().map(|e| -> El<'a> {
                let msg = UiMessage::ToggleReaction(chat.id.clone(), m.id.clone(), e.clone());
                let glyph = txt(e, 18.0);
                button(container(glyph).center(32))
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
                    .on_press(msg)
                    .into()
            });
            let all: Vec<El<'a>> = [reply, react].into_iter().chain(quick).collect();
            match mine {
                true => Row::with_children(all.into_iter().rev()),
                false => Row::with_children(all),
            }
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
        let line: El<'a> = if mine {
            row![Space::new().width(Fill)].push(tools).push(parts).spacing(6).align_y(Center).into()
        } else {
            let body = row![parts].push(tools).spacing(6).align_y(Center);
            row![].push(side).push(body).push(Space::new().width(Fill)).spacing(6).align_y(Bottom).into()
        };
        let (line, head): (El<'a>, El<'a>) = match quote {
            None => (line, head.into()),
            Some((q, q_mine)) => {
                let color = p.reply_line;
                let bar = || container(Space::new()).width(3).height(Fill).style(fill_box(color, 1.5));
                let curve = svg(REPLY_CURVE[usize::from(q_mine)].clone())
                    .width(25)
                    .height(19)
                    .style(move |_, _| svg::Style { color: Some(color) });
                let side = |c: Column<'a, UiMessage>| -> El<'a> {
                    let c = c.align_x(if q_mine { Right } else { iced::Left });
                    match q_mine {
                        true => row![Space::new().width(Fill), c.padding(Padding::ZERO.right(14.0))].into(),
                        false => row![]
                            .push(group.then(|| Space::new().width(34)))
                            .push(c.padding(Padding::ZERO.left(14.0)))
                            .into(),
                    }
                };
                let cross = q_mine != mine;
                // The curve ends level with the middle of the bubble, above the footer and the reactions.
                let link = column![bar(), curve, Space::new().height(Fill), Space::new().height(tail + 19.0)];
                let line = if cross { stack![line, side(link)].into() } else { line };
                let head = if cross && chained { stack![head, side(column![bar()])].into() } else { head.into() };
                let quote = q.map(|q| -> El<'a> {
                    if q_mine {
                        row![Space::new().width(Fill), q].into()
                    } else {
                        row![].push(group.then(|| Space::new().width(34))).push(q).into()
                    }
                });
                (column![].push(quote).push(line).into(), head)
            }
        };
        let line = mouse_area(line)
            .on_enter(UiMessage::HoverEnter(m.id.clone()))
            .on_exit(UiMessage::HoverExit(m.id.clone()))
            .on_right_press(UiMessage::OpenMessageMenu(m.id.clone()))
            .on_press(UiMessage::ClickMessage(m.id.clone()))
            .on_double_click(UiMessage::DoubleClickMessage(m.id.clone()));
        column![head, line].into()
    }

    fn view_composer<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
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
        let gif_picker = self.show_gif_picker.then(|| {
            let search = text_input("Search KLIPY...", &self.gif_query)
                .id("gif-search-input")
                .on_input(UiMessage::GifQueryChanged)
                .size(13)
                .padding([6, 12])
                .style(input_style(p.input, p.strong, p.sub, p.accent, 8.0));
            let elapsed = self.started.elapsed().as_millis();
            let carousel: El<'a> = if let Some(err) = &self.gif_error {
                container(txt(err.clone(), 12.0).color(p.danger)).center_y(215).padding([0, 14]).into()
            } else if self.gif_loading && self.gifs.is_empty() {
                container(txt("Loading GIFs...", 12.0).color(p.sub)).center_y(215).padding([0, 14]).into()
            } else {
                let items = self.gifs.iter().map(|g| {
                    let (pw, ph) = g.preview_size;
                    let w = (215.0 * pw as f32 / ph.max(1) as f32).round().max(48.0);
                    let cell: El<'a> = match self.media.get(&g.preview_url).and_then(|v| v.as_ref()) {
                        Some(Ok(frames)) => image(current_frame(frames, elapsed))
                            .width(w)
                            .height(215)
                            .content_fit(iced::ContentFit::Cover)
                            .into(),
                        _ => container(Space::new()).width(w).height(215).style(fill_box(p.hover, 4.0)).into(),
                    };
                    button(container(cell).clip(true).style(fill_box(CLEAR, 4.0)))
                        .padding(0)
                        .style(btn_style([CLEAR, p.hover, p.selected], [p.text; 3], 4.0))
                        .on_press(UiMessage::PickGif(g.clone()))
                        .into()
                });
                let dir = Direction::Horizontal(Scrollbar::new().width(4).scroller_width(4));
                scrollable(container(Row::with_children(items).spacing(4)).padding(Padding::ZERO.left(14.0)))
                    .direction(dir)
                    .height(215)
                    .width(Fill)
                    .style(scroll_style(p.scroll))
                    .into()
            };
            column![container(search).padding([0, 14]), carousel].spacing(8).padding(Padding::ZERO.top(4.0))
        });
        let action_btn = |id: u8| -> El<'a> {
            let msg = match id {
                0 => Some(UiMessage::Emoji(emoji::EmojiMsg::Toggle)),
                2 => Some(UiMessage::ToggleGifPicker),
                3 => Some(UiMessage::PasteClipboard),
                _ => None,
            };
            let active = (id == 2 && self.show_gif_picker)
                || (id == 0 && self.emoji.as_ref().is_some_and(|pk| pk.react.is_none()));
            let color = if active { p.strong } else { p.on_secondary };
            let bg = if active { p.hover } else { CLEAR };
            let glyph = svg(COMPOSER_ICONS[usize::from(id) + 2].clone())
                .width(20)
                .height(20)
                .opacity(if msg.is_some() { 1.0_f32 } else { 0.3 })
                .style(move |_, _| svg::Style { color: Some(color) });
            let b = button(container(glyph).center(28))
                .padding(0)
                .style(btn_style([bg, p.hover, p.selected], [p.text; 3], 14.0))
                .on_press_maybe(msg);
            match id {
                0 => mouse_area(b)
                    .on_enter(UiMessage::Emoji(emoji::EmojiMsg::Pointer(0, true)))
                    .on_exit(UiMessage::Emoji(emoji::EmojiMsg::Pointer(0, false)))
                    .into(),
                _ => b.into(),
            }
        };
        let (pri, sec) = composer_buttons(st);
        let placeholder = format!("Message {} on {}", chat.title, network_name(&chat.account_id));
        let suggesting = !self.emoji_suggestions(st).is_empty();
        let formats = chat.formatting;
        let input = text_editor(&self.composer)
            .id("composer-input")
            .placeholder(placeholder)
            .on_action(UiMessage::ComposerEdit)
            .key_binding(move |k| match k.key.as_ref() {
                Key::Named(Named::Enter) if !k.modifiers.shift() => {
                    Some(text_editor::Binding::Custom(UiMessage::SubmitComposer))
                }
                Key::Named(Named::ArrowUp | Named::ArrowDown) if suggesting => None,
                Key::Named(Named::Escape) => None,
                Key::Character(c) if k.modifiers.command() => {
                    let i = match (c.to_ascii_lowercase().as_str(), k.modifiers.shift()) {
                        ("b", false) => 0,
                        ("i", false) => 1,
                        ("x", true) => 2,
                        _ => return text_editor::Binding::from_key_press(k),
                    };
                    (formats & 1 << i != 0).then_some(text_editor::Binding::Custom(UiMessage::Format(i)))
                }
                _ => text_editor::Binding::from_key_press(k),
            })
            .size(13)
            .padding([9, 2])
            .height(Shrink)
            .max_height(self.main_size.height / 2.0)
            .style(move |_, _| text_editor::Style {
                background: CLEAR.into(),
                border: iced::Border::default(),
                placeholder: p.on_secondary,
                value: p.strong,
                selection: p.accent.scale_alpha(0.45),
            });
        let empty = self.composer.is_empty();
        let net = (st.settings.on("showNetworkIcons") && empty)
            .then(|| svg(brand_svg(brand_key(&chat.account_id))).width(14).height(14));
        let inline = Row::with_children([pri, sec].into_iter().flatten().map(&action_btn)).spacing(2).align_y(Center);
        let net = Row::new().push(net.map(|n| container(n).padding(Padding::ZERO.right(2.0)).center_y(36)));
        let inline = container(inline).padding(Padding::ZERO.left(2.0)).center_y(36);
        let input = self.with_formatting(input.into(), if st.settings.on("formattingMenu") { formats } else { 0 });
        let pill = container(row![net, input, inline].align_y(Bottom).padding(Padding::new(0.0).left(14.0).right(6.5)))
            .width(Fill)
            .style(fill_box(p.input, 18.0));
        let plus_icon = tinted(&COMPOSER_ICONS[usize::from(self.show_attach_menu)], 20.0, p.on_secondary);
        let toggle = button(container(plus_icon).center(36))
            .padding(0)
            .style(btn_style([p.input, p.hover, p.selected], [p.text; 3], 18.0))
            .on_press(UiMessage::ToggleAttachMenu);
        let unveiled = self.show_attach_menu.then(|| {
            let ids = ATTACH_BUTTONS.into_iter().filter(|&x| Some(x) != pri && Some(x) != sec);
            Row::with_children(ids.map(&action_btn)).spacing(2).padding([0, 1]).align_y(Center)
        });
        let plus = row![toggle].push(unveiled).spacing(6).align_y(Center);
        let ready = !self.composer.text().trim().is_empty() || self.pasted.is_some();
        let pasted = self.pasted.as_ref().map(|pm| {
            let fg = Color { a: 0.1, ..p.text };
            let h = current_frame(&pm.frames, self.started.elapsed().as_millis());
            let preview = container(image(h).height(66).content_fit(iced::ContentFit::Contain))
                .clip(true)
                .style(bordered_box(CLEAR, fg, 1.0, 8.0));
            row![removable(p, preview, UiMessage::DiscardPaste)].padding([6, 54])
        });
        let send = button(container(ic(icon::ARROW_UP, 18.0).color(Color::WHITE)).center(36))
            .padding(0)
            .style(btn_style([if ready { p.accent } else { p.send_btn }, p.accent, p.accent], [p.text; 3], 18.0))
            .on_press(UiMessage::SubmitComposer);
        let text = self.composer.text();
        let urls = text_urls(&text);
        let links = self.composer_links.iter().filter(|(u, _)| urls.contains(&u.as_str())).filter_map(|(u, s)| {
            let card: El<'a> = match s {
                LinkState::Discarded => return None,
                LinkState::Loading => {
                    let bar = |w: f32| container(Space::new()).width(w).height(12).style(fill_box(p.hover, 12.0));
                    let bars = column![bar(111.0), bar(47.0)].spacing(8);
                    container(bars).width(180).padding([8, 11]).style(fill_box(p.input, 17.0)).into()
                }
                LinkState::Loaded(v, img) => {
                    let title = v["og:title"].as_str().expect("previews have a title");
                    let host = u.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(u).to_string();
                    let img = img
                        .as_ref()
                        .map(|(_, h)| image(h.clone()).width(180).height(101).content_fit(iced::ContentFit::Cover));
                    let wide = if img.is_some() { 158.0 } else { 338.0 };
                    let link = row![
                        ic(icon::LINK, 12.0).color(p.weak),
                        txt(host, 11.0).line_height(Pixels(14.0)).color(p.weak)
                    ];
                    let text = column![
                        clamp(title, wide, 3, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(p.strong),
                        link.spacing(4).align_y(Center)
                    ];
                    let card = container(column![].push(img).push(text.spacing(8).padding([12, 11])));
                    card.width(if wide < 300.0 { Length::Fixed(180.0) } else { Shrink })
                        .clip(true)
                        .style(fill_box(p.link_card, 17.0))
                        .into()
                }
            };
            Some(removable(p, card, UiMessage::RemoveLinkPreview(u.clone())))
        });
        let links = Column::with_children(links).spacing(4).padding([2, 54]);
        let extras = Column::new().push(banner).push(gif_picker).push(pasted).push(links);
        column![extras, row![plus, pill, send].spacing(8).align_y(Center).padding(8)].into()
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
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Pinned)
            ),
            action(
                icon::NOTIFICATIONS_OFF,
                if chat.is_muted == Some(true) { "Unmute" } else { "Mute" },
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Muted)
            ),
            action(
                icon::ARCHIVE,
                if chat.is_archived == Some(true) { "Unarchive" } else { "Archive" },
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Archived)
            ),
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
            container(scrollable(body).direction(bar()).style(scroll_style(p.scroll)))
                .width(300)
                .height(Fill)
                .style(fill_box(p.card, 10.0)),
        )
        .padding(Padding::new(8.0).left(0.0))
        .into()
    }

    fn view_command_palette<'a>(&'a self) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let room = self.search_room.as_deref();
        let placeholder = match (self.cmd_bar_commands_mode, room) {
            (true, _) => "Search commands",
            (false, Some(_)) => "Search this chat",
            (false, None) => "Search chats",
        };
        let input = text_input(placeholder, &self.command_palette_query)
            .id("cmd-palette-input")
            .on_input(UiMessage::CommandQueryChanged)
            .size(13)
            .padding([6, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let kbd = |label: &'static str| {
            container(semi(label, 13.0).color(p.sub))
                .center_x(Shrink)
                .center_y(20)
                .padding([0, 2])
                .style(fill_box(Color { a: 0.1, ..p.strong }, 4.0))
        };
        let frame = |body: Column<'a, UiMessage>| -> El<'a> {
            container(body)
                .width(602)
                .height(360)
                .style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 84.0))
                .into()
        };
        let mode_btn = button(
            row![
                txt(if self.cmd_bar_commands_mode { "Chat" } else { "Commands" }, 13.0).color(p.sub),
                kbd("Ctrl"),
                kbd(if self.cmd_bar_commands_mode { "K" } else { "J" })
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([2, 6])
        .style(ghost(p, 8.0))
        .on_press(UiMessage::ToggleCommandMode);
        let inputs = row![ic(icon::SEARCH, 14.0).color(p.sub), input, mode_btn]
            .spacing(8)
            .align_y(Center)
            .padding(Padding::new(6.0).left(13.0).right(8.0));
        let q = self.command_palette_query.trim().to_lowercase();
        if self.cmd_bar_commands_mode {
            let cmds: [(&str, UiMessage); 4] = [
                ("Create new chat", UiMessage::NewChat),
                ("Search messages in all chats", UiMessage::ToggleCommandMode),
                ("Open settings", UiMessage::OpenSettings(self.active_settings_tab)),
                ("Toggle chat info", UiMessage::ToggleRightPanel),
            ];
            let list = cmds.into_iter().filter(|(t, _)| t.to_lowercase().contains(&q)).enumerate().fold(
                Column::new().padding([4, 6]),
                |col, (i, (title, msg))| {
                    let bg = if i == 0 { p.chip } else { CLEAR };
                    col.push(
                        button(container(semi(title, 13.0).color(p.strong)).center_y(36))
                            .padding([0, 12])
                            .width(Fill)
                            .style(btn_style([bg, p.chip, p.selected], [p.strong; 3], 8.0))
                            .on_press(msg),
                    )
                },
            );
            return frame(column![
                inputs,
                scrollable(list).direction(bar()).height(Fill).style(scroll_style(p.scroll))
            ]);
        }
        let hint =
            self.selected_chat_id.as_deref().and_then(|cid| st.resolve_chat_idx(cid).map(|i| &st.chats[i])).map(|c| {
                let prep = if c.chat_type == "group" { "in" } else { "with" };
                let label = match room {
                    Some(_) => "Press Ctrl K to search all of your conversations".into(),
                    None => format!("Press Ctrl F to search your conversation {prep} {}", c.title),
                };
                row![tinted(&DROP_ARROW, 20.0, p.sub), txt(label, 13.0).color(p.text)]
                    .spacing(16)
                    .align_y(Center)
                    .padding([8, 24])
            });
        let f = &self.search_filters;
        let chips = Row::with_children(SEARCH_CHIPS.iter().enumerate().map(|(i, &placeholder)| {
            let disabled = room.is_some() && i == 0;
            let fg = if disabled { p.text.scale_alpha(0.3) } else { p.text };
            let label = f.label(i);
            let style = match label {
                Some(_) => bordered_box(CLEAR, p.text, 1.0, 20.0),
                None => bordered_box(p.chip, CLEAR, 1.0, 20.0),
            };
            let body = row![
                semi(label.unwrap_or_else(|| placeholder.into()), 13.0).color(fg),
                ic(icon::EXPAND_MORE, 14.0).color(fg)
            ]
            .spacing(4)
            .align_y(Center);
            let chip = button(body).padding([4, 13]).style(btn_style([CLEAR; 3], [fg; 3], 20.0));
            container(chip.on_press_maybe((!disabled).then_some(UiMessage::OpenSearchSelect(i))))
                .id(format!("search-chip-{i}"))
                .style(style)
                .into()
        }))
        .spacing(12)
        .padding(Padding::new(6.0).left(13.0).right(13.0));
        let filtered = *f != SearchFilters::default();
        let mut chats: Vec<&Chat> = if room.is_some() || !f.media.is_empty() || (filtered && q.is_empty()) {
            Vec::new()
        } else if q.is_empty() && !st.recent_chat_ids.is_empty() {
            st.recent_chat_ids
                .iter()
                .filter_map(|id| st.chat_idx_by_id.get(id).map(|&i| &st.chats[i]))
                .filter(|c| c.flags & CHAT_HIDDEN == 0)
                .collect()
        } else {
            let mut v: Vec<&Chat> = st
                .chats
                .iter()
                .filter(|c| {
                    c.flags & CHAT_HIDDEN == 0
                        && (f.accounts.is_empty() || f.accounts.contains(&c.account_id))
                        && (q.is_empty()
                            || c.title.to_lowercase().contains(&q)
                            || chat_search_subtitle(c).to_lowercase().contains(&q))
                })
                .collect();
            v.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
            v
        };
        if q.is_empty() && chats.len() > 1 {
            chats.retain(|c| self.selected_chat_id.as_deref() != Some(c.id.as_str()));
        }
        let has_more = chats.len() > 5;
        let limit = if self.search_expanded { 12 } else { 5 };
        let cat =
            row![txt(if q.is_empty() { "Recent chats" } else { "Chats" }, 12.0).color(p.sub), Space::new().width(Fill)]
                .push(has_more.then(|| {
                    button(semi(if self.search_expanded { "Show less" } else { "Show more" }, 11.0).color(p.accent))
                        .padding(0)
                        .style(ghost(p, 4.0))
                        .on_press(UiMessage::ToggleSearchExpanded)
                }))
                .align_y(Center)
                .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0));
        let mut found: Vec<&Message> = match q.chars().count() > 1 && f.media.is_empty() {
            true => st
                .msg_search_index
                .query_all_tokens(&q)
                .into_iter()
                .map(|i| &st.messages[i as usize])
                .filter(|m| m.text.as_deref().is_some_and(|t| !t.is_empty()) && f.keeps(&st, room, m))
                .collect(),
            false => Vec::new(),
        };
        found.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
        found.truncate(20);
        let media = self.media_hits(&st);
        let none =
            found.is_empty() && chats.is_empty() && media.is_empty() && (q.chars().count() > 1 || !f.media.is_empty());
        let heading = |label: &'static str| {
            container(txt(label, 12.0).color(p.sub)).padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        };
        let h24 = st.settings.pref("timeFormat") == 1;
        let needle = q.to_ascii_lowercase();
        let messages =
            found.iter().fold(Column::new().push((!found.is_empty()).then(|| heading("Messages"))), |col, m| {
                let c = &st.chats[st.chat_idx_by_id[&m.chat_id]];
                let mine = m.is_sender == Some(true);
                let label = match (c.chat_type == "group", mine) {
                    (false, _) => c.title.clone(),
                    (true, true) => format!("{} • {}", c.title, chat_network(c)),
                    (true, false) => {
                        format!(
                            "{} in {} • {}",
                            m.sender_name.as_deref().expect("sender name"),
                            c.title,
                            chat_network(c)
                        )
                    }
                };
                let head = row![
                    txt(label, 11.0).line_height(Pixels(7.0)).color(p.text),
                    Space::new().width(Fill),
                    txt(list_ts(&m.timestamp, h24), 11.0).line_height(Pixels(7.0)).color(p.text)
                ];
                let (bg, fg) = if mine { (p.sent, Color::WHITE) } else { (p.received, p.strong) };
                let body = clamped(m.text.as_deref().expect("text message"), 554.0, 3, 13.0, Weight::Normal);
                let lower = body.to_ascii_lowercase();
                let (mut spans, mut at): (Vec<iced::widget::text::Span<()>>, _) = (Vec::new(), 0);
                for (k, hit) in lower.match_indices(&needle) {
                    spans.push(span(body[at..k].to_string()).size(13.0).color(fg));
                    let mark = span(body[k..k + hit.len()].to_string()).size(13.0);
                    spans.push(mark.color(hex(0x202124)).background(hex(0xf6ce46)));
                    at = k + hit.len();
                }
                spans.push(span(body[at..].to_string()).size(13.0).color(fg));
                let bubble = rich_text(spans)
                    .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
                let bubble = container(container(bubble).padding([8, 11]).style(fill_box(bg, 17.0)))
                    .width(Fill)
                    .align_x(if mine { Right } else { iced::Left });
                col.push(
                    button(column![head, bubble].spacing(6))
                        .padding([4, 13])
                        .width(Fill)
                        .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                        .on_press(UiMessage::JumpTo(c.id.clone(), m.id.clone())),
                )
            });
        let tiles = media.iter().map(|m| -> El<'_> {
            let c = &st.chats[st.chat_idx_by_id[&m.chat_id]];
            let card = |title: String, sub: El<'static>| -> El<'static> {
                let body =
                    column![clamp(&title, 94.0, 3, 10.0, Weight::Normal).line_height(Pixels(12.0)).color(p.text), sub];
                container(body).align_bottom(Fill).width(Fill).padding(8).style(fill_box(p.frame, 8.0)).into()
            };
            let thumb = m.attachments.iter().find(|a| matches!(a.attachment_type.as_str(), "img" | "video"));
            let body: El<'_> = match (thumb, m.links.first(), m.attachments.first()) {
                (Some(a), _, _) => {
                    match a.poster_img.as_ref().or(a.src_url.as_ref()).and_then(|u| self.media.get(u)?.as_ref()) {
                        Some(Ok(frames)) => image(frames[0].0.clone())
                            .width(Fill)
                            .height(Fill)
                            .content_fit(iced::ContentFit::Cover)
                            .border_radius(8.0)
                            .into(),
                        _ => Space::new().into(),
                    }
                }
                (None, Some(l), _) => {
                    let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
                    let title = if l.title.is_empty() { host.to_string() } else { l.title.clone() };
                    let sub = row![
                        ic(icon::LINK, 12.0).color(p.text),
                        ell(&l.url, 77.0, 10.0, Weight::Semibold).color(p.text)
                    ];
                    card(title, sub.spacing(4).align_y(Center).into())
                }
                (None, None, Some(a)) => {
                    let kind = file_kind(a.mime_type.as_deref());
                    let name = a.file_name.clone().unwrap_or_else(|| "Attachment".into());
                    card(name, semi(kind, 10.0).line_height(Pixels(12.0)).color(p.text).into())
                }
                (None, None, None) => {
                    let text = m.text.clone().expect("a link message has text");
                    card(text, Space::new().into())
                }
            };
            let face =
                container(self.chat_avatar(c, 24.0)).padding(2).style(bordered_box(CLEAR, Color::WHITE, 2.0, 14.0));
            let tile = stack![container(body).width(110).height(110).style(fill_box(p.outline, 8.0))]
                .push(container(face).padding(6));
            button(tile)
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
                .on_press(UiMessage::JumpTo(c.id.clone(), m.id.clone()))
                .into()
        });
        let mut grid = Column::new().spacing(6.5).padding(Padding::new(0.0).left(13.0).right(13.0).bottom(4.0));
        let mut tiles = tiles.peekable();
        while tiles.peek().is_some() {
            grid = grid.push(Row::with_children(tiles.by_ref().take(5)).spacing(6.5));
        }
        let media_grid = (!media.is_empty()).then(|| column![heading("Media"), grid]);
        let empty = none.then(|| {
            let desc = "Only messages loaded in Beeper can be searched.\nFor more results, scroll through older conversations.";
            column![
                container(semi("No messages matching those terms", 16.0).color(p.strong)).center(Fill),
                txt(desc, 13.0).color(p.strong).align_x(Center).width(Fill),
            ]
            .padding(Padding::new(0.0).left(44.0).right(44.0).bottom(64.0))
        });
        let cat = (!chats.is_empty()).then_some(cat);
        let hits = chats.into_iter().take(limit).enumerate().fold(Column::new().push(cat), |col, (i, c)| {
            let sub = chat_search_subtitle(c);
            let title = row![
                ell(&c.title, 260.0, 13.0, Weight::Semibold).color(p.strong),
                ell(&sub, 220.0, 13.0, Weight::Normal).color(p.sub),
            ]
            .spacing(4)
            .align_y(Center);
            let line = row![
                self.chat_avatar(c, 28.0),
                title,
                Space::new().width(Fill),
                svg(brand_svg(brand_key(&c.account_id))).width(16).height(16)
            ]
            .spacing(8)
            .align_y(Center);
            let bg = if i == 0 { p.chip } else { CLEAR };
            col.push(
                container(
                    button(line)
                        .height(36)
                        .padding([0, 6])
                        .width(Fill)
                        .style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0))
                        .on_press(UiMessage::ExecuteCommandChat(c.id.clone())),
                )
                .padding([0, 6]),
            )
        });
        frame(column![inputs].push(hint).push(chips).push(empty).push((!none).then(|| {
            scrollable(hits.push(media_grid).push(messages)).direction(bar()).height(Fill).style(scroll_style(p.scroll))
        })))
    }

    /// The (message, attachment index) items of the carousel, and the position of the shown one:
    /// the images and GIFs of the open chat, oldest first, or the Media window items.
    fn carousel_items(&self, st: &BeeperStore) -> (Vec<(String, usize)>, usize) {
        let c = self.carousel.as_ref().expect("the carousel is open");
        let items: Vec<_> = match c.in_media {
            true => media_items(st, self.media_window.as_ref().expect("the Media window is open"))
                .into_iter()
                .map(|(i, k)| (st.messages[i].id.clone(), k))
                .collect(),
            false => {
                let chat = self.selected_chat_id.as_deref().expect("an open chat");
                let messages = st.chat_msg_indices[chat].iter().map(|&i| &st.messages[i]);
                messages
                    .flat_map(|m| {
                        let shown =
                            |(_, a): &(usize, &Attachment)| a.attachment_type == "img" || a.is_gif == Some(true);
                        m.attachments.iter().enumerate().filter(shown).map(|(ai, _)| (m.id.clone(), ai))
                    })
                    .collect()
            }
        };
        let at = items.iter().position(|(m, k)| *m == c.mid && *k == c.ai).expect("the carousel shows its item");
        (items, at)
    }

    fn carousel_key(&mut self, key: &Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
        let (items, at) = self.carousel_items(&self.store.read().unwrap());
        let c = self.carousel.clone().expect("the carousel is open");
        match key.as_ref() {
            Key::Named(Named::Escape) => self.update(UiMessage::CloseCarousel),
            Key::Named(Named::ArrowLeft) if mods.is_empty() && at > 0 => self.update(UiMessage::CarouselStep(false)),
            Key::Named(Named::ArrowRight) if mods.is_empty() && at + 1 < items.len() => {
                self.update(UiMessage::CarouselStep(true))
            }
            Key::Character("d") if mods.is_empty() => self.update(UiMessage::Download(c.mid, Some(c.ai))),
            Key::Character("r") if mods.is_empty() => self.update(UiMessage::RotateCarousel),
            _ => Task::none(),
        }
    }

    /// The square cover crop of an image or video poster, or an empty tile while it loads.
    fn thumb<'a>(&self, a: &Attachment, size: f32, r: f32) -> El<'a> {
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        match url.and_then(|u| self.media.get(u)?.as_ref()) {
            Some(Ok(frames)) => image(frames[0].0.clone())
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(r)
                .into(),
            _ => container(Space::new()).width(size).height(size).style(fill_box(self.pal.chip, r)).into(),
        }
    }

    /// The name Desktop shows for the sender of a message.
    fn sender_name<'s>(st: &'s BeeperStore, m: &'s Message) -> &'s str {
        match m.is_sender == Some(true) {
            true => st.me().full_name.as_deref().expect("self name"),
            false => m.sender_name.as_deref().expect("sender name"),
        }
    }

    fn view_carousel<'a>(&'a self, car: &Carousel) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let (mid, ai) = (car.mid.as_str(), car.ai);
        let m = &st.messages[st.msg_idx_by_id[mid]];
        let c = &st.chats[st.chat_idx_by_id[&m.chat_id]];
        let a = &m.attachments[ai];
        let (items, at) = self.carousel_items(&st);
        let h24 = st.settings.pref("timeFormat") == 1;
        let white = Color::WHITE;
        let control = |h: &svg::Handle, msg| {
            button(tinted(h, 20.0, white)).padding(0).style(btn_style([CLEAR; 3], [white; 3], 0.0)).on_press(msg)
        };
        let (face, name) = match car.in_media {
            true => {
                let u = c.participants.items.iter().find(|u| u.id == m.sender_id);
                let name = Self::sender_name(&st, m);
                (u.map(|u| self.face(u.img_url.as_deref(), name, 28.0)), name)
            }
            false => (Some(self.chat_avatar(c, 28.0)), c.title.as_str()),
        };
        let title = row![]
            .push(face)
            .push(ell(name, 600.0, 14.0, Weight::Medium).color(white))
            .push(txt(format_short_ts(&m.timestamp, h24), 12.0).color(white.scale_alpha(0.7)))
            .spacing(8)
            .align_y(Center);
        let go = car.in_media.then(|| control(&MEDIA_ICONS[3], UiMessage::GoToMessage(c.id.clone(), mid.into())));
        let controls = row![]
            .push(go)
            .push(control(&SYS_ROTATE, UiMessage::RotateCarousel))
            .push(control(&SYS_DOWNLOAD, UiMessage::Download(mid.to_string(), Some(ai))))
            .push(control(&SYS_CLOSE, UiMessage::CloseCarousel))
            .spacing(8);
        let head =
            stack![container(title).center_x(Fill).center_y(34), container(controls).align_right(Fill).center_y(34)];
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        let shown: El<'a> = match url.and_then(|u| self.media.get(u)?.as_ref()) {
            Some(Ok(frames)) => image(current_frame(frames, self.started.elapsed().as_millis()))
                .content_fit(iced::ContentFit::ScaleDown)
                .rotation(iced::Rotation::Solid(iced::Radians(-std::f32::consts::FRAC_PI_2 * f32::from(car.turns))))
                .into(),
            _ => Space::new().into(),
        };
        let main = container(mouse_area(shown).on_press(UiMessage::Noop)).center(Fill).padding(8);
        let step = |next: bool| -> El<'a> {
            let arrow = svg(SYS_ARROW_BACK.clone())
                .width(20)
                .height(20)
                .rotation(if next { std::f32::consts::PI } else { 0.0 })
                .style(move |_, _| svg::Style { color: Some(white) });
            let b = button(container(arrow).center(32)).padding(0).style(btn_style(
                [Color { a: 0.1, ..p.strong }; 3],
                [white; 3],
                32.0,
            ));
            let b = container(b.on_press(UiMessage::CarouselStep(next))).center_y(Fill).padding([0, 24]);
            if next {
                b.align_right(Fill).into()
            } else {
                b.into()
            }
        };
        let strip = car.in_media.then(|| {
            let (start, end) =
                filmstrip(&self.media_window.as_ref().expect("the Media window is open").size, at, items.len());
            let thumbs = items[start..end].iter().enumerate().map(|(k, (mid, ai))| -> El<'a> {
                let a = &st.messages[st.msg_idx_by_id[mid]].attachments[*ai];
                let edge = if start + k == at { p.accent } else { CLEAR };
                let face = container(self.thumb(a, 44.0, 6.0)).padding(2).style(bordered_box(CLEAR, edge, 2.0, 8.0));
                let open = UiMessage::OpenCarousel(mid.clone(), *ai, true);
                button(face).padding(0).style(btn_style([CLEAR; 3], [white; 3], 8.0)).on_press(open).into()
            });
            let row = container(Row::with_children(thumbs).spacing(4)).padding([6, 10]).width(Fill);
            container(mouse_area(row.style(fill_box(Color::BLACK.scale_alpha(0.45), 0.0))).on_press(UiMessage::Noop))
                .align_bottom(Fill)
        });
        let body = column![container(head).padding(8).height(50), main];
        let layer = stack![body]
            .push((at > 0).then(|| step(false)))
            .push((at + 1 < items.len()).then(|| step(true)))
            .push(strip);
        mouse_area(container(layer).width(Fill).height(Fill).style(fill_box(Color::BLACK.scale_alpha(0.85), 0.0)))
            .on_press(UiMessage::CloseCarousel)
            .into()
    }

    /// The status toast at the bottom left of a window, `left` from its edge.
    fn view_toast(&self, left: f32) -> Option<El<'_>> {
        let p = &self.pal;
        (!self.status_banner.is_empty()).then(|| {
            let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, p.sub, UiMessage::DismissBanner);
            let text = txt(self.status_banner.clone(), 13.0).color(p.strong);
            let card = row![text.wrapping(iced::widget::text::Wrapping::WordOrGlyph), close].spacing(8).align_y(Center);
            let card = container(card).padding(8).style(shadow_box(p.card, p.outline, 10.0, 15.0));
            container(card).width(Fill).align_bottom(Fill).padding(Padding::new(8.0).left(left)).into()
        })
    }

    fn view_media<'a>(&'a self, w: &MediaWindow) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let [file_icon, video_icon, link_icon, _, tune_icon, search_icon, done_icon] = &*MEDIA_ICONS;
        let picking = w.select || self.mods.control();
        let picked = |mid: &str, k: usize| w.picked.iter().any(|(m, i)| m == mid && *i == k);
        let check = |on: bool| -> El<'a> {
            let (bg, edge) = if on { (p.accent, p.accent) } else { (Color::BLACK.scale_alpha(0.25), Color::WHITE) };
            let mark = on.then(|| tinted(done_icon, 14.0, Color::WHITE));
            container(mark).center(20).style(bordered_box(bg, edge, 1.5, 10.0)).into()
        };
        let items = media_items(&st, w);
        let sub = |s: String| txt(s, 12.0).line_height(Pixels(15.0)).color(p.on_secondary);
        let tabs = MEDIA_TABS.iter().enumerate().map(|(i, label)| -> El<'a> {
            let bg = if i == w.tab { p.chip } else { CLEAR };
            let label = txt(*label, 12.0).line_height(Pixels(15.0)).color(p.strong);
            button(label)
                .padding([2, 8])
                .style(btn_style([bg; 3], [p.strong; 3], 20.0))
                .on_press(UiMessage::MediaTab(i))
                .into()
        });
        let tabs = container(Row::with_children(tabs).spacing(4)).padding(4).style(fill_box(p.card, 20.0));
        let control = |glyph: &svg::Handle, size: f32, on: bool, msg| -> El<'a> {
            let (bg, fg) = if on { (p.chip, p.strong) } else { (CLEAR, p.on_secondary) };
            let b = button(container(tinted(glyph, size, fg)).center(28)).padding(0);
            b.style(btn_style([bg, p.chip, p.chip], [fg; 3], 12.0)).on_press(msg).into()
        };
        let search = UiMessage::MediaSearch(w.search.is_none().then(String::new));
        let controls = row![
            self.tip(
                control(done_icon, 18.0, w.select, UiMessage::MediaSelect(!w.select)),
                "Select",
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(tune_icon, 18.0, false, UiMessage::MediaMenu(true)),
                "Filter",
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(search_icon, 18.0, w.search.is_some(), search),
                "Search",
                vec![],
                tooltip::Position::Bottom
            ),
        ]
        .spacing(4);
        let header = container(row![tabs, Space::new().width(Fill), controls].spacing(8).align_y(Center))
            .padding([8, 10])
            .center_y(52);
        let rule = || container(Space::new()).width(Fill).height(0.5).style(fill_box(p.outline, 0.0));
        let search_row = w.search.as_deref().map(|q| {
            let input = text_input("Search by caption", q)
                .id("media-search")
                .on_input(|q| UiMessage::MediaSearch(Some(q)))
                .size(14)
                .padding(0)
                .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
            let close = self.tip(
                control(&SYS_CLOSE, 16.0, false, UiMessage::MediaSearch(None)),
                "Close",
                vec![],
                tooltip::Position::Bottom,
            );
            let line = row![tinted(search_icon, 16.0, p.on_secondary), input, close].spacing(6).align_y(Center);
            column![container(line).padding([6, 10]), rule()]
        });
        let inner = w.size.width - 16.0;
        let cols = ((inner / 160.0) as usize).max(3);
        let tile = (inner - 20.0 - 4.0 * (cols - 1) as f32) / cols as f32;
        let text_w = inner - 20.0 - 48.0 - 8.0 - 150.0;
        let mut groups: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
        for &(i, k) in items.iter().take(w.shown) {
            let label = media_group(day_num(&st.messages[i].timestamp));
            match groups.last_mut() {
                Some((l, g)) if *l == label => g.push((i, k)),
                _ => groups.push((label, vec![(i, k)])),
            }
        }
        let body = groups.iter().fold(Column::new(), |col, (label, group)| {
            let days = group.iter().map(|&(i, _)| day_num(&st.messages[i].timestamp));
            let (lo, hi) = (days.clone().min().expect("a group has items"), days.max().expect("a group has items"));
            let range = if lo == hi { medium_date(lo) } else { format!("{} - {}", medium_date(lo), medium_date(hi)) };
            let head = column![bold(label.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong), sub(range)];
            let col = col.push(container(head).padding(Padding::new(6.0).top(12.0).left(10.0).right(10.0)));
            match w.tab {
                0 => group.chunks(cols).fold(col, |col, chunk| {
                    let tiles = chunk.iter().map(|&(i, k)| -> El<'a> {
                        let m = &st.messages[i];
                        let a = &m.attachments[k];
                        let badge = match (a.is_gif == Some(true), a.attachment_type == "video") {
                            (true, _) => Some(row![semi("GIF", 12.0)]),
                            (false, true) => {
                                let time = a.duration.map(|d| semi(clip_time(d), 12.0));
                                Some(row![tinted(video_icon, 12.0, Color::WHITE)].push(time).spacing(2).align_y(Center))
                            }
                            _ => None,
                        };
                        let badge = badge.map(|b| {
                            let b = container(b).padding([1, 4]).style(move |_: &Theme| container::Style {
                                text_color: Some(Color::WHITE),
                                ..fill_box(Color::BLACK.scale_alpha(0.55), 8.0)(&Theme::Dark)
                            });
                            container(b).padding(4)
                        });
                        let fade = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                            .add_stop(0.0, CLEAR)
                            .add_stop(1.0, Color::BLACK.scale_alpha(0.55));
                        let name =
                            ell(Self::sender_name(&st, m), tile - 12.0, 12.0, Weight::Semibold).color(Color::WHITE);
                        let name =
                            container(name).width(Fill).padding(Padding::new(4.0).top(10.0).left(6.0).right(6.0));
                        let name = container(name.style(move |_| container::Style {
                            background: Some(fade.into()),
                            border: border::rounded(border::Radius::default().bottom(8)),
                            ..Default::default()
                        }))
                        .align_bottom(Fill);
                        let on = picked(&m.id, k);
                        let mark = w.select.then(|| container(check(on)).align_right(Fill).padding(4));
                        let ring = on.then(|| {
                            container(Space::new())
                                .width(Fill)
                                .height(Fill)
                                .style(bordered_box(CLEAR, p.accent, 2.0, 8.0))
                        });
                        let face = stack![self.thumb(a, tile, 8.0)].push(badge).push(name).push(mark).push(ring);
                        let face = face.width(tile).height(tile);
                        let open = match picking {
                            true => UiMessage::MediaPick(m.id.clone(), k),
                            false => UiMessage::OpenCarousel(m.id.clone(), k, true),
                        };
                        let b = button(face).padding(0).style(btn_style([CLEAR; 3], [p.strong; 3], 8.0));
                        mouse_area(b.on_press(open))
                            .on_right_press(UiMessage::MediaItemMenu(Some((m.id.clone(), k))))
                            .into()
                    });
                    col.push(
                        container(Row::with_children(tiles).spacing(4))
                            .padding(Padding::new(0.0).bottom(4.0).left(10.0).right(10.0)),
                    )
                }),
                _ => group.iter().fold(col, |col, &(i, k)| {
                    let m = &st.messages[i];
                    let (glyph, size, title, line, press) = match w.tab {
                        1 => {
                            let l = &m.links[k];
                            let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
                            let title = if l.title.is_empty() { host.to_string() } else { l.title.clone() };
                            (link_icon, 16.0, title, l.url.clone(), UiMessage::OpenUrl(l.url.clone()))
                        }
                        _ => {
                            let a = &m.attachments[k];
                            let kind = file_kind(a.mime_type.as_deref());
                            let line = std::iter::once(kind.to_string()).chain(a.file_size.map(human_size));
                            let name = a.file_name.clone().unwrap_or_else(|| "Attachment".into());
                            (
                                file_icon,
                                18.0,
                                name,
                                line.collect::<Vec<_>>().join(" · "),
                                UiMessage::Download(m.id.clone(), Some(k)),
                            )
                        }
                    };
                    let thumb = container(tinted(glyph, size, p.on_secondary)).center(40).style(fill_box(p.chip, 8.0));
                    let text = column![
                        ell(&title, text_w, 14.0, Weight::Semibold).line_height(Pixels(17.0)).color(p.strong),
                        ell(&line, text_w, 12.0, Weight::Normal).line_height(Pixels(15.0)).color(p.on_secondary),
                    ]
                    .width(Fill);
                    let meta = column![sub(Self::sender_name(&st, m).to_string()), sub(relative_date(&m.timestamp))]
                        .align_x(Right);
                    let on = picked(&m.id, k);
                    let line = row![thumb, text, meta].push(w.select.then(|| check(on))).spacing(8).align_y(Center);
                    let press = if picking { UiMessage::MediaPick(m.id.clone(), k) } else { press };
                    let bg = if on { p.chip } else { CLEAR };
                    let b = button(line).padding([6, 10]).width(Fill);
                    let b = b.style(btn_style([bg, p.chip, p.chip], [p.strong; 3], 0.0)).on_press(press);
                    col.push(mouse_area(b).on_right_press(UiMessage::MediaItemMenu(Some((m.id.clone(), k)))))
                }),
            }
        });
        let more = w.shown < items.len();
        let list: El<'a> = match items.is_empty() {
            true => container(txt("Nothing here", 14.0).color(p.on_secondary)).center_x(Fill).padding(16).into(),
            false => scrollable(body.push(Space::new().height(8)))
                .direction(bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .on_scroll(
                    move |v| if more && v.relative_offset().y > 0.8 { UiMessage::MediaMore } else { UiMessage::Noop },
                )
                .into(),
        };
        let title = container(semi("Media", 14.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let footer = w.select.then(|| {
            let bytes: u64 = match w.tab {
                1 => 0,
                _ => w
                    .picked
                    .iter()
                    .filter_map(|(mid, k)| st.messages[st.msg_idx_by_id[mid]].attachments[*k].file_size)
                    .sum(),
            };
            let n = w.picked.len();
            let count = match bytes {
                0 => format!("{n} selected"),
                _ => format!("{n} selected ({})", human_size(bytes)),
            };
            let act = |label, msg: Option<UiMessage>| -> El<'a> {
                let a = if msg.is_some() { 1.0 } else { 0.5 };
                let b =
                    button(semi(label, 12.0).line_height(Pixels(15.0)).color(p.strong.scale_alpha(a))).padding([4, 8]);
                b.style(btn_style([p.chip.scale_alpha(a); 3], [p.strong; 3], 12.0)).on_press_maybe(msg).into()
            };
            let picked = w.picked.clone();
            let bar = row![
                semi(count, 14.0).line_height(Pixels(17.0)).color(p.strong),
                Space::new().width(Fill),
                act("Delete", (n > 0).then(|| UiMessage::MediaAskDelete(picked))),
                act("Save", (n > 0 && w.tab != 1).then_some(UiMessage::MediaSave)),
                act("Select all", Some(UiMessage::MediaPickAll)),
                act("Cancel", Some(UiMessage::MediaSelect(false))),
            ]
            .spacing(4)
            .align_y(Center);
            column![rule(), container(bar).padding([8, 10]).center_y(44)]
        });
        let pane = column![header, rule()].push(search_row).push(list).push(footer);
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.card, 12.0));
        let base = container(column![title, pane]).padding(8).width(Fill).height(Fill).style(fill_box(p.window, 0.0));
        let base = mouse_area(base).on_move(UiMessage::MediaCursor);
        let item_menu = w.item_menu.as_ref().map(|(mid, k, at)| {
            let m = &st.messages[st.msg_idx_by_id[mid]];
            let act =
                |label: &str, msg| menu_item(p, None, label.into(), "", Some(UiMessage::MediaItemAct(Box::new(msg))));
            let go = act("Go to Message", UiMessage::GoToMessage(m.chat_id.clone(), mid.clone()));
            let mut actions = match w.tab {
                1 => vec![act("Copy Link", UiMessage::CopyLink(m.links[*k].url.clone()))],
                _ => std::iter::once(act("Download", UiMessage::Download(mid.clone(), Some(*k))))
                    .chain(
                        (m.attachments[*k].attachment_type == "img")
                            .then(|| act("Copy Image", UiMessage::CopyImage(mid.clone(), *k))),
                    )
                    .collect(),
            };
            actions.extend(deletable(m).then(|| act("Delete…", UiMessage::MediaAskDelete(vec![(mid.clone(), *k)]))));
            let h = 8.0 + MENU_ITEM * (1 + actions.len()) as f32 + 11.0;
            let col = Column::with_children([go, menu_rule(p)].into_iter().chain(actions));
            let menu = container(col.padding([4, 0])).width(180).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let at = Padding::ZERO.left(at.x.min(w.size.width - 180.0 - 8.0)).top(at.y.min(w.size.height - h - 8.0));
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MediaItemMenu(None))
                .on_right_press(UiMessage::MediaItemMenu(None))
        });
        let menu = w.menu.then(|| {
            let heading = |label| container(med(label, 11.0).color(p.sub)).padding([6, 13]);
            let radio = |label: &str, on: bool, msg| {
                let lead = container(on.then(|| ic(icon::CHECK, 14.0))).width(22);
                menu_item(p, Some(lead.into()), label.into(), "", Some(msg))
            };
            let col = column![
                heading("Sent By"),
                radio("All", w.sent_by == 0, UiMessage::MediaSentBy(0)),
                radio("You", w.sent_by == 1, UiMessage::MediaSentBy(1)),
                radio("Others", w.sent_by == 2, UiMessage::MediaSentBy(2)),
                menu_rule(p),
                heading("Sort by"),
                radio("Newest", !w.oldest, UiMessage::MediaOldest(false)),
                radio("Oldest", w.oldest, UiMessage::MediaOldest(true)),
            ];
            let menu = container(col.padding([4, 0])).width(180).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let x = (w.size.width - 8.0 - 10.0 - 60.0).min(w.size.width - 8.0 - 180.0);
            let at = Padding::ZERO.top(8.0 + 33.0 + 44.0).left(x);
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MediaMenu(false))
        });
        let carousel = self.carousel.as_ref().filter(|c| c.in_media).map(|c| self.view_carousel(c));
        let ask = w.deleting.as_ref().map(|(pairs, _)| {
            let n = pairs.len();
            let (title, body) = match n {
                1 => ("Delete message".to_string(), "Are you sure you want to delete this message?".to_string()),
                _ => ("Delete messages".to_string(), format!("Are you sure you want to delete {n} messages?")),
            };
            self.dialog(title, body, vec![("Delete", UiMessage::MediaDelete(true))], UiMessage::MediaDelete(false))
        });
        stack![base].push(self.view_toast(8.0)).push(menu).push(item_menu).push(ask).push(carousel).into()
    }

    fn view_forward<'a>(&'a self, f: &'a Forward) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let shade = Color { a: 0.1, ..p.strong };
        let strip = |content: El<'a>, r| container(content).width(Fill).padding([8, 13]).style(fill_box(p.chip, r));
        let head = row![tinted(&MAT_FORWARD, 16.0, p.strong), txt("Forward message", 14.0).color(p.strong)];
        let head = strip(head.spacing(12).align_y(Center).into(), border::Radius::default().top(18)).center_y(44);
        let pills = f.picked.iter().map(|cid| -> El<'a> {
            let c = &st.chats[st.chat_idx_by_id[cid]];
            let label = semi(c.title.clone(), 14.0).line_height(Pixels(16.0)).color(p.strong);
            let pill = row![self.chat_avatar(c, 16.0), label].spacing(4).align_y(Center);
            container(pill).padding(Padding::new(2.0).right(8.0)).style(fill_box(shade, 20.0)).into()
        });
        let input = text_input("Search name, @, or number", &f.query)
            .id("forward-input")
            .on_input(UiMessage::ForwardQuery)
            .size(14)
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let inputs = Row::with_children(pills).push(input).spacing(4).align_y(Center).height(24);
        let inputs = container(inputs).padding(Padding::new(18.0).left(13.0).right(20.0));
        let chats = forward_chats(&st, f);
        let cat = (f.query.trim().is_empty() && !chats.is_empty()).then(|| {
            container(txt("Recent chats", 12.0).color(p.sub))
                .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        });
        let rows = chats.iter().enumerate().fold(Column::new().push(cat), |col, (i, c)| {
            let sub = chat_search_subtitle(c);
            let title = row![
                ell(&c.title, 260.0, 13.0, Weight::Semibold).color(p.strong),
                ell(&sub, 220.0, 13.0, Weight::Normal).color(p.sub),
            ]
            .spacing(4)
            .align_y(Center);
            let line = row![self.chat_avatar(c, 32.0), title, Space::new().width(Fill)]
                .push(svg(brand_svg(brand_key(&c.account_id))).width(16).height(16))
                .spacing(8)
                .align_y(Center);
            let bg = if i == 0 { p.chip } else { CLEAR };
            let pick = button(line).height(36).padding([0, 6]).width(Fill);
            let pick = pick.style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0));
            col.push(container(pick.on_press(UiMessage::ForwardPick(c.id.clone()))).padding([0, 6]))
        });
        let ready = !f.picked.is_empty();
        let label = row![semi("Forward", 12.0), txt("CTRL ENTER", 11.0).color(Color { a: 0.8, ..Color::WHITE })];
        let send = button(label.spacing(4).align_y(Center))
            .padding([6, 8])
            .style(btn_style([if ready { p.accent } else { shade }; 3], [Color::WHITE; 3], 100.0))
            .on_press_maybe(ready.then_some(UiMessage::ForwardSend));
        let note = "Forwarding messages through Beeper will not include any attribution on the native apps.";
        let foot = row![txt(note, 14.0).color(p.sub).width(Fill), send].spacing(12).align_y(Center);
        let body = column![
            head,
            inputs,
            scrollable(rows).direction(bar()).height(300).style(scroll_style(p.scroll)),
            container(Space::new()).width(Fill).height(2).style(fill_box(shade, 0.0)),
            strip(foot.into(), border::Radius::default().bottom(18)),
        ];
        container(body).width(602).style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 84.0)).into()
    }

    fn view_new_chat(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read().unwrap();
        let fade = Color { a: 0.4, ..Color::WHITE };
        let footer = |label: &'static str, key: &'static str| {
            let pill = container(
                row![med(label, 13.0).color(fade), txt(key, 10.0).color(Color { a: 0.32, ..Color::WHITE })]
                    .spacing(4)
                    .align_y(Center),
            )
            .padding([4, 8])
            .style(fill_box(Color { a: 0.4, ..p.accent }, 20.0));
            row![Space::new().width(Fill), pill].padding(Padding::new(8.0).left(13.0)).align_y(Center)
        };
        let field = |placeholder: &'static str| {
            text_input(placeholder, &self.new_chat_query)
                .id("new-chat-input")
                .on_input(UiMessage::NewChatQueryChanged)
                .size(13)
                .padding([8, 0])
                .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0))
        };
        if self.join_matrix_mode {
            let back = button(
                row![tinted(&ARROW_BACK, 18.0, p.strong), txt("Back", 12.0).line_height(Pixels(15.0)).color(p.strong)]
                    .spacing(4)
                    .align_y(Center),
            )
            .padding([4, 6])
            .style(ghost(p, 6.0))
            .on_press(UiMessage::ToggleJoinMatrix);
            let head =
                container(row![back, txt("Join a Matrix room", 13.0).color(p.strong)].spacing(12).align_y(Center))
                    .width(Fill)
                    .padding([8, 13])
                    .style(fill_box(p.chip, 0.0));
            let inputs = row![tinted(&MAT_LINK, 18.0, p.sub), field("Room address or Matrix link")]
                .spacing(8)
                .align_y(Center)
                .padding(Padding::new(6.0).left(13.0).right(8.0));
            let body = column![head, inputs, Space::new().height(Fill), footer("Join", "ENTER")];
            return container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0)).into();
        }
        let all_acc = container(
            row![med("All accounts", 12.0).color(p.strong), ic(icon::EXPAND_MORE, 14.0).color(p.strong)]
                .spacing(4)
                .align_y(Center),
        )
        .center_y(24)
        .padding(Padding::new(0.0).left(8.0).right(6.0))
        .style(fill_box(p.chip, 12.0));
        let inputs = row![ic(icon::SEARCH, 16.0).color(p.sub), field("Search contacts"), all_acc]
            .spacing(8)
            .align_y(Center)
            .padding(Padding::new(6.0).top(10.0).left(13.0).right(8.0));
        let q = self.new_chat_query.trim().to_lowercase();
        let mut col = Column::new().padding(Padding::ZERO.top(3.0));
        if q.is_empty() {
            let group_btn = button(
                row![ic(icon::GROUP, 20.0).color(p.accent), semi("New Group", 13.0).color(p.strong)]
                    .spacing(7)
                    .align_y(Center),
            )
            .width(Fill)
            .height(36)
            .padding([0, 11])
            .style(btn_style([p.chip, p.chip, p.selected], [p.strong; 3], 6.0))
            .on_press(UiMessage::Noop);
            let join_btn = button(
                row![tinted(&MAT_LINK, 20.0, p.accent), semi("Join a Matrix room", 13.0).color(p.strong)]
                    .spacing(7)
                    .align_y(Center),
            )
            .width(Fill)
            .height(36)
            .padding([0, 11])
            .style(btn_style([CLEAR, p.chip, p.selected], [p.strong; 3], 6.0))
            .on_press(UiMessage::ToggleJoinMatrix);
            col = col.push(container(group_btn).padding([0, 8])).push(container(join_btn).padding([0, 8]));
        }
        let contacts: Vec<&User> = st
            .contacts
            .iter()
            .filter(|u| u.full_name.as_deref().unwrap_or_default().to_lowercase().contains(&q))
            .collect();
        if !contacts.is_empty() {
            col = col.push(
                container(txt("In your contacts", 12.0).color(p.sub))
                    .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0)),
            );
            let limit = if self.new_chat_expanded || !q.is_empty() { contacts.len() } else { 10.min(contacts.len()) };
            let mut last_cat = None;
            for u in contacts.iter().take(limit) {
                let name = u.full_name.as_deref().unwrap_or(&u.id);
                let ch = name.chars().next().unwrap_or(' ').to_ascii_uppercase();
                let cat = if ch.is_ascii_alphabetic() { ch.to_string() } else { "Others".to_string() };
                if last_cat.as_deref() != Some(cat.as_str()) {
                    col = col.push(
                        container(txt(cat.clone(), 12.0).color(p.sub))
                            .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0)),
                    );
                    last_cat = Some(cat);
                }
                let target = st
                    .chats
                    .iter()
                    .find(|c| c.chat_type == "single" && c.title.eq_ignore_ascii_case(name))
                    .map(|c| c.id.clone());
                let line = row![
                    self.face(u.img_url.as_deref(), name, 28.0),
                    semi(name.to_string(), 13.0).color(p.strong),
                    Space::new().width(Fill),
                    ic(icon::EXPAND_MORE, 20.0).color(p.strong)
                ]
                .spacing(8)
                .align_y(Center);
                col = col.push(
                    container(
                        button(line)
                            .height(36)
                            .padding([0, 6])
                            .width(Fill)
                            .style(ghost(p, 8.0))
                            .on_press_maybe(target.map(UiMessage::ExecuteCommandChat)),
                    )
                    .padding([0, 6]),
                );
            }
            if q.is_empty() && contacts.len() > 10 {
                let label = if self.new_chat_expanded { "Show less contacts" } else { "Show more contacts" };
                col = col.push(
                    container(
                        button(txt(label, 13.0).color(p.text))
                            .height(36)
                            .padding([0, 6])
                            .width(Fill)
                            .style(ghost(p, 8.0))
                            .on_press(UiMessage::ToggleNewChatExpanded),
                    )
                    .padding([0, 6]),
                );
            }
        }
        let body = column![
            inputs,
            scrollable(col).direction(bar()).height(Fill).style(scroll_style(p.scroll)),
            footer("Start chat", "CTRL ENTER")
        ];
        container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0)).into()
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
            scrollable(column![profile, nav]).direction(bar()).height(Fill).style(scroll_style(p.scroll))
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
                .direction(Direction::Vertical(Scrollbar::new().width(15).scroller_width(9).spacing(0)))
                .height(Fill)
                .style(gutter_style(p.gutter)),
        )
        .width(Fill)
        .height(Fill)
        .style(fill_box(p.prefs_pane, 10.0));
        let base = container(row![left, container(right).padding(8)]).width(Fill).height(Fill);
        let confirm = self.confirm.filter(|&d| d == ASK_LOGOUT).map(|d| self.view_confirm(d));
        stack![base.style(fill_box(p.frame, 0.0))].push(confirm).into()
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
                .on_press(UiMessage::Ask(Some(ASK_LOGOUT)));
            let line: El<'a> =
                row![tile, text].push(d.current.then_some(logout)).spacing(12).align_y(Center).padding([8, 12]).into();
            [(i > 0).then(|| divider(p.prefs_border, 0.0)), Some(line)].into_iter().flatten()
        });
        column![
            container(txt(title, 12.0).color(p.sub)).padding([8, 0]),
            container(Column::with_children(rows)).style(bordered_box(CLEAR, p.prefs_border, 1.0, 10.0))
        ]
        .into()
    }

    fn view_confirm(&self, dialog: u8) -> El<'_> {
        let (title, body, actions) = match dialog {
            ASK_LOGOUT => {
                let name = self.devices.iter().flatten().flatten().find(|d| d.current).map(|d| d.name.clone());
                let title = format!("Logout of {}?", name.as_deref().unwrap_or("this device"));
                (title, "This will disconnect all chat accounts on this device.", vec![("Logout", UiMessage::Logout)])
            }
            ASK_DISMISS_REMINDERS => (
                "Dismiss All Reminders?".into(),
                "This will remove Reminders from all of your chats.",
                vec![("Yes", UiMessage::DismissAllReminders)],
            ),
            ASK_DELETE_CHAT => {
                let st = self.store.read().unwrap();
                let id = self.delete_target.clone().expect("a chat to delete");
                let c = &st.chats[st.resolve_chat_idx(&id).expect("the chat to delete exists")];
                let delete = |label, everyone| (label, UiMessage::DeleteChat(id.clone(), everyone));
                let (body, actions) = match c.delete_modes {
                    DELETE_FOR_ME => {
                        ("This action will permanently delete this chat.", vec![delete("Delete Chat", false)])
                    }
                    DELETE_FOR_EVERYONE => (
                        "This action will permanently delete this chat for all participants.",
                        vec![delete("Delete for Everyone", true)],
                    ),
                    _ => (
                        "Choose to delete permanently for yourself only, or for all participants.",
                        vec![delete("Delete for Me", false), delete("Delete for Everyone", true)],
                    ),
                };
                (format!("Delete {}", c.title), body, actions)
            }
            ASK_DELETE_MESSAGE => {
                let mid = self.delete_target.clone().expect("a message to delete");
                let cid = self.selected_chat_id.clone().expect("an open chat");
                let delete = vec![("Delete", UiMessage::DeleteMessage(cid, mid))];
                ("Delete message".into(), "Are you sure you want to delete this message?", delete)
            }
            _ => unreachable!("dialog {dialog}"),
        };
        self.dialog(title, body.into(), actions, UiMessage::Ask(None))
    }

    /// A confirmation dialog over a window, with its actions after Cancel.
    fn dialog<'a>(
        &'a self,
        title: String,
        body: String,
        actions: Vec<(&'static str, UiMessage)>,
        cancel: UiMessage,
    ) -> El<'a> {
        let p = &self.pal;
        let action = |(label, msg): (&'static str, UiMessage)| {
            button(med(label, 13.0).color(Color::WHITE))
                .padding([5, 12])
                .style(btn_style([p.accent; 3], [Color::WHITE; 3], 6.0))
                .on_press(msg)
                .into()
        };
        let card = column![
            bold(title, 14.0).color(p.strong),
            txt(body, 13.0).color(p.text),
            row![Space::new().width(Fill), pill_btn(p, "Cancel", cancel.clone())]
                .extend(actions.into_iter().map(action))
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
        .on_press(cancel)
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
            SettingsTab::Hotkeys => vec![self.hotkeys_page(s)],
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
                    ("Report a problem", UiMessage::OpenUrl(ISSUES_URL.into())),
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

    fn hotkeys_page(&self, s: &BeeperSettings) -> El<'_> {
        let p = &self.pal;
        let (edge, text, muted, accent) = (p.neutral_border, p.text, p.muted, p.accent);
        let search = text_input("Search hotkeys...", &self.hotkey_query)
            .on_input(UiMessage::HotkeyQuery)
            .size(13)
            .line_height(Pixels(15.0))
            .padding([4, 7])
            .width(219)
            .style(move |_, _| text_input::Style {
                background: CLEAR.into(),
                border: iced::Border { color: edge, width: 1.0, radius: 6.0.into() },
                icon: muted,
                placeholder: muted,
                value: text,
                selection: accent.scale_alpha(0.45),
            });
        let revert = button(txt("Revert all", 13.0).line_height(Pixels(15.0)).width(Fill).align_x(Center))
            .width(100)
            .padding([4, 6])
            .style(pill_style(p))
            .on_press(UiMessage::RevertHotkeys);
        let q = self.hotkey_query.to_lowercase();
        let hotkey = |&(name, keys): &(&'static str, &'static str)| -> El<'_> {
            let on = s.hotkey_on(name);
            let recorder = on.then(|| {
                container(txt(keys, 13.0).line_height(Pixels(18.0)).color(p.text))
                    .center_x(160)
                    .center_y(20)
                    .style(bordered_box(CLEAR, edge, 1.0, 4.0))
            });
            row![txt(name, 13.0).color(p.text).width(Fill)]
                .push(recorder)
                .push(toggle(p, on, Some(UiMessage::ToggleHotkey(name))))
                .spacing(6)
                .align_y(Center)
                .padding([8, 0])
                .into()
        };
        let groups = prefs::HOTKEYS.iter().filter_map(|(group, keys)| {
            let rows: Vec<El<'_>> = keys.iter().filter(|(n, _)| n.to_lowercase().contains(&q)).map(hotkey).collect();
            let head = txt(*group, 13.0).line_height(Pixels(16.0)).color(p.muted);
            (!rows.is_empty())
                .then(|| column![container(head).padding(Padding::ZERO.top(24.0).bottom(12.0))].extend(rows).into())
        });
        let head = row![search, Space::new().width(Fill), revert].align_y(Center).padding(Padding::ZERO.bottom(10.0));
        let line = container(Space::new()).width(Fill).height(1).style(fill_box(edge, 0.0));
        let card = column![head, line, Space::new().height(6)].extend(groups);
        container(card).width(Fill).padding(14).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0)).into()
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
                let off = r.requires.is_some_and(|k| !s.on(k));
                let msg = (!locked && !off).then_some(UiMessage::SetPref(r.key, 1 - v));
                toggle_faded(p, v == 1, msg, if off { 0.5 } else { 1.0 })
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

const ATTACH_BUTTONS: [u8; 7] = [0, 3, 5, 2, 1, 4, 6];

/// The composer buttons in the pill by "Primary composer action" and "Secondary composer action".
fn composer_buttons(st: &BeeperStore) -> (Option<u8>, Option<u8>) {
    let action = |key| [Some(0), Some(1), Some(5), Some(2), Some(3), Some(4), None][usize::from(st.settings.pref(key))];
    let pri = action("primaryComposerButton");
    (pri, action("secondaryComposerButton").filter(|&x| Some(x) != pri))
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
    fn ease_out_quint_matches_css() {
        assert!(ease_out_quint(0.0).abs() < 1e-3 && (ease_out_quint(1.0) - 1.0).abs() < 1e-3);
        assert!((ease_out_quint(0.5) - 0.963).abs() < 5e-3);
    }

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

    #[test]
    fn copy_text_matches_desktop() {
        let mut m: Message = serde_json::from_value(serde_json::json!({
            "id": "m", "accountID": "a", "chatID": "c", "senderID": "s", "sortKey": "1",
            "timestamp": "2026-01-02T03:04:05Z", "text": "see https://a.b", "attachments": [{ "type": "img" }],
            "links": [{ "url": "https://a.b", "title": "" }, { "url": "https://c.d", "title": "" }],
        }))
        .expect("message JSON");
        assert_eq!(copy_text(&m, false), "🖼 [Picture] see https://a.b\nhttps://c.d");
        m.sender_name = Some("Jo".into());
        let stamped = copy_text(&m, true);
        assert!(stamped.starts_with('[') && stamped.ends_with("] Jo: 🖼 [Picture] see https://a.b\nhttps://c.d"));
    }

    #[test]
    fn downloads_name_like_desktop() {
        assert_eq!([0, 1000, 1536, 1 << 20, 5_000_000].map(human_size), ["0 B", "1000 B", "1.5 KB", "1 MB", "4.8 MB"]);
        let dir = std::env::temp_dir().join(format!("beeper-rs-unused-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unused_path(&dir, "a.png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), b"").unwrap();
        assert_eq!(unused_path(&dir, "a.png"), dir.join("a (1).png"));
        assert_eq!(unused_path(&dir, ".env"), dir.join(".env"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn media_window_formats_like_desktop() {
        assert_eq!([5.0, 65.4, 3723.0].map(clip_time), ["00:05", "01:05", "01:02:03"]);
        assert_eq!(
            [file_kind(Some("application/pdf")), file_kind(Some("audio/ogg")), file_kind(None)],
            ["PDF", "Audio", "File"]
        );
        let size = Size::new(540.0, 720.0);
        assert_eq!([0, 9, 29].map(|at| filmstrip(&size, at, 30)), [(0, 10), (4, 14), (20, 30)]);
        assert_eq!(filmstrip(&size, 1, 3), (0, 3));
        assert_eq!(medium_date(20_743), "Oct 17, 2026");
    }
}

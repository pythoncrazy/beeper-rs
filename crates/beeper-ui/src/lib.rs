mod anim;
mod chat_theme;
mod commands;
mod cursor;
mod dashed;
pub mod emoji;
mod format;
mod i18n;
mod lift;
pub mod login;
mod notify;
mod popover;
mod portal;
mod record;
mod sound;
mod spell;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
mod tray {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Event {
        Up,
        Activate,
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
mod video;
mod voice;

use crate::i18n::{tr, trf, trn, trnx, trs, trx};
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
    parse_when_millis, rfc3339_from_millis, APP_VERSION,
};
use beeper_core::import::has_beeper_data;
use beeper_core::matrix::{ChatFlag, DeviceInfo, MediaInfo, Outgoing};
use beeper_core::models::*;
use beeper_core::platform::{current_original_beeper_data_dir, open_url_or_path_command, DisplayServerKind, OsKind};
use beeper_core::prefs;
use beeper_core::storage;
use beeper_core::store::{BeeperStore, NetworkSpace, SharedStore, NOTE_CHAT_ID};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, grid, image, mouse_area, operation, pick_list, pin, rich_text, row, scrollable, slider,
    span, stack, svg, text_editor, text_input, tooltip, Column, Grid, Row, Space, Stack,
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
    pub fn label(self) -> &'static str {
        match self {
            Self::Archived => trx("Archive", "archive space name"),
            _ => tr(["Inbox", "Low Priority", "", "Reminders", "Send Later", "Requests"][self as usize]),
        }
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

    fn empty(self) -> (&'static str, &'static str) {
        let (head, hint) = [
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
        ][self as usize];
        (tr(head), tr(hint))
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
/// The hotkeys of the filters before Archived.
const CYCLE: &str = "Cycle Chats (hold shift to reverse)";
const FILTER_HOTKEYS: [&str; 5] =
    ["Filter: Unread", "Filter: Drafts", "Filter: Unanswered", "Filter: DMs", "Filter: Groups"];
const F_ARCHIVED: usize = 5;
const F_REQUESTS: usize = 6;
const F_LOW_PRIORITY: usize = 7;

/// Desktop's membership events, which the timeline groups.
fn membership(m: &Message) -> bool {
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
            v.and_then(|v| v.iter().rev().map(|&i| &st.messages[i]).find(|m| !m.silent())).is_none_or(|m| {
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

    /// The translated name of the section.
    pub fn name(self) -> &'static str {
        tr(self.meta().0)
    }

    /// Desktop's `searchKeywords` of the section.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
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
}

/// A misspelled word of the composer and its suggestions.
type Typo = (std::ops::Range<usize>, Vec<String>);

/// An item of the composer context menu.
#[derive(Debug, Clone)]
pub enum MenuEdit {
    Replace(String),
    Learn,
    Cut,
    Copy,
    Paste,
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
    /// Ctrl+Backspace and Ctrl+Delete: delete the selection, or the text from the caret to the motion.
    DeleteTo(text_editor::Motion, bool),
    /// Mod-z undoes the last composer edit group; Shift-Mod-z and Mod-y redo it.
    Undo(bool),
    /// Toggles a `format::LABELS` button on the composer selection.
    Format(usize),
    LinkDialog(Option<(String, String)>),
    SaveLink,
    SubmitComposer,
    SendAndArchive,
    CancelComposeMode,
    StartReply(String),
    StartEdit(String, String),
    DeleteMessage(String, String),
    ToggleReaction(String, String, String),
    ToggleBookmark(String),
    MarkUnreadFromHere(String, String),
    RetranscribeVoice(String),
    /// The poll message and all the own answers.
    VotePoll(String, Vec<String>),
    ToggleAttachMenu,
    TogglePicker(Picker),
    /// Desktop's `undoSend` in the open chat.
    UndoSend,
    /// Desktop's `sendQueuedMessages`: the sends of a chat, or the sends that are due.
    SendQueued(Option<String>),
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
    SetReminder(String, Option<i64>),
    OpenSubMenu(commands::SubMenu),
    CmdNoReply(bool),
    /// Keeps the time phrase for Desktop's "Last Used".
    LastTime(String),
    RemindIf(String, i64, bool),
    ChatSub(Option<u8>),
    CheckReminders,
    /// Desktop's `showWithSubMenu(SCHEDULE_MESSAGE)` for the composer.
    OpenSendLater,
    /// The composer's `sendLaterConfig`: the time and "If no reply".
    SendLater(Option<(i64, bool)>),
    /// The menu of scheduled messages at the cursor; true adds the message items.
    ScheduledMenu(Vec<String>, bool),
    SendNow(Vec<String>),
    Reschedule(Vec<String>),
    RescheduleTo(Vec<String>, i64, bool),
    Unschedule(Vec<String>),
    EditScheduled(String),
    FoldScheduled(bool),
    CheckScheduled,
    /// Desktop's `showReplyThread`: shows the reply chain of the message over the timeline.
    ShowReplies(String),
    HideReplies,
    ToggleFlag(String, ChatFlag),
    ToggleRead(String),
    MarkRead(String),
    /// Sets the wallpaper of the chat theme of a chat, or of the default theme.
    ChatWallpaper(Option<String>, Wallpaper),
    /// Sets the bubble color of the chat theme of a chat, or of the default theme; `None` clears it.
    ChatBubble(Option<String>, Option<Bubble>),
    /// Desktop's `Choose photo`: picks a photo wallpaper for a chat, or for the default theme.
    PickWallpaper(Option<String>),
    WallpaperPicked(Option<String>, Result<Vec<std::path::PathBuf>, String>),
    /// Desktop's `clearChatTheme`: the chat uses the default chat theme again.
    ClearChatTheme(String),
    /// Desktop's `editChatTheme`: opens the chat theme window of a chat.
    OpenChatTheme(String),
    /// Desktop's `PollCreatorBrowserWindow` for a chat.
    OpenPollCreator(String),
    Poll(window::Id, PollEdit),
    /// Desktop's `PollVotesBrowserWindow` for a chat's poll message.
    OpenPollVotes(String, String),
    EndPoll(String, String),
    CopyChatLink(String),
    OpenChatMenu(String),
    OpenMessageMenu(String),
    /// Chromium's composer context menu, on a misspelled word or not.
    EditMenu(Option<std::ops::Range<usize>>),
    MenuEdit(MenuEdit),
    /// Select the message, or unselect it when it is selected.
    ClickMessage(String),
    SelectMessages(Vec<String>),
    /// Copy the text of the selected messages, with their timestamps or not.
    CopySelected(bool),
    /// Do the "On double-clicking a message" action.
    DoubleClickMessage(String),
    /// Select the message above (`false`) or below (`true`) the selected one, as with the arrow keys.
    /// Select the next or the previous message, or with `extend`, add it to the selection.
    StepMessage(bool, bool),
    /// Open the emoji picker at the cursor to react to a message.
    React(String),
    /// An action picked in the message menu, which closes it.
    MessageMenu(Box<UiMessage>),
    CopyText(String),
    /// Copy a link and say so, like Desktop's Media window.
    CopyLink(String),
    CopyImage(String, usize),
    ImageFetched(Result<Vec<u8>, String>),
    /// Open the forward dialog for messages, in timeline order.
    Forward(Vec<String>),
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
    /// Escape in a text input, which unfocuses it.
    InputEscape(window::Id),
    /// Save the attachments of a message, or only one of them, to the downloads folder.
    Download(String, Option<usize>),
    Downloaded(Result<String, String>),
    AskDeleteMessage(String),
    ArchiveStale,
    /// Archives the inbox chats, or only the read ones, with no activity in the days, after Desktop's confirmation.
    ArchiveAll(bool, i64),
    MarkAllRead,
    ToggleMaximize,
    RunHotkey(&'static str),
    /// Runs the messages in order.
    Commands(Vec<UiMessage>),
    CommandHighlight(usize),
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
    ShowAppQr,
    OpenAccount(Option<String>),
    SetAccountProps(String, beeper_core::models::AccountProps),
    /// "Show in Inbox": the `chat_list_rules` key and option.
    SetInboxVisibility(String, &'static str),
    SelectSettingsTab(SettingsTab),
    SettingsQueryChanged(String),
    SettingsFocus(usize),
    /// Moves the focused search result down by the given count, wrapping around.
    SettingsStep(usize),
    PickSetting(SettingsHit),
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
    /// Opens Desktop's Keyboard Shortcuts list with the search text, or closes it.
    HotkeysModal(Option<String>),
    HotkeysScrolled(bool),
    Zoom(i8),
    /// The zoom slider moved to a level from -4 to 4, in steps of 10%.
    ZoomDrag(f32),
    ZoomRelease,
    ToggleSetting(fn(&mut BeeperSettings)),
    /// The Beeper account email and the chat accounts from `whoami`.
    CloudAccounts(Result<(Option<String>, bool, Vec<BridgeAccountInfo>), String>),
    AvatarLoaded(String, Result<image::Handle, String>),
    MediaLoaded(String, Result<Frames, String>),
    Voice(voice::Msg),
    Record(record::Msg),
    RetryMedia(String),
    ResolveMedia(String, String, String),
    MediaResolved(String, Result<(), String>),
    Video(video::Msg),
    Pasted(Result<Option<Png>, String>),
    DiscardAttachment(usize),
    /// Desktop's `openAttachmentPicker`.
    PickFiles,
    FilesPicked(Result<Vec<std::path::PathBuf>, String>),
    FilesLoaded(Result<Vec<PendingMedia>, String>),
    /// A file drag enters (true) or leaves a window.
    FileHover(window::Id, bool),
    FileDropped(window::Id, std::path::PathBuf),
    LinkPreviewLoaded(String, Result<Option<beeper_core::cloud::PagePreview>, String>),
    RemoveLinkPreview(String),
    PickEmoji(usize),
    AnimationTick,
    RecoveryInputChanged(String),
    VerifyRecoveryCode,
    Verified(Result<(), String>),
    Pushed(Result<(), String>),
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    MicsLoaded(Result<(String, Vec<String>), String>),
    SetMicrophone(String),
    Ask(Option<u8>),
    ReminderMenu,
    OrderReminders(u8),
    DismissAllReminders,
    Logout,
    LoggedOut(Result<(), String>),
    VerifySasNow,
    SasMatch(bool),
    StorageScanned(Vec<storage::Entry>),
    /// The age choice of the clear for all chats (0) or for the open chat (1).
    StorageAge(usize, usize),
    /// Toggles a type, as its index in [`storage::Kind::ALL`], for all chats (0) or for the open chat (1).
    StorageKind(usize, usize),
    StorageChat(String),
    /// Clears the chosen storage of all chats (0), the cache (1) or the open chat (2).
    ClearStorage(u8),
    StorageCleared(Result<(), String>),
    /// The "Install to..." choice, as an index into [`MCP_TARGETS`].
    McpTarget(usize),
    /// Installs the Beeper MCP server in a client of [`MCP_CLIENTS`] or [`MCP_TARGETS`].
    McpInstall(&'static str),
    /// Copies a value of Settings > Integrations; its copy button shows a check for 2 s.
    CopyField(u8, String),
    CopyDone(u8),
    JoinCommunity,
    CommunityJoined(Result<(), String>),
    /// A new token request of the Desktop API (no outcome), or the end of one.
    TokenRequest(String, Option<beeper_core::connect::Outcome>),
    TokenWrite,
    TokenExpiry(Expiry),
    /// Approve or Decline in the "Approve connection" dialog.
    TokenAnswer(bool),
    TokenClose,
    /// Revoke in Approved connections, which asks first.
    RevokeConnection(String),
    /// The Remote Access toggle, which asks before it turns on.
    RemoteAccess(bool),
    RevokeConfirmed,
    /// The "+" of Approved connections opens the token dialog, and Escape closes it.
    AddToken(bool),
    AddTokenName(String),
    AddTokenExpiry(Expiry),
    AddTokenWrite,
    CreateToken,
    CopyToken,
    OpenQuickLook(String, String),
    OpenUrl(String),
    /// The result of opening a URL in the browser.
    Opened(Result<(), String>),
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
    /// A window drew a frame, after which its screenshot has content.
    Drawn(window::Id),
    /// A message from a chat window, handled with its pane.
    InWindow(window::Id, Box<UiMessage>),
    Relaunch,
    OpenChatWindow(String),
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
fn hotkey_key(key: &Key) -> Option<String> {
    Some(match key.as_ref() {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(Named::ArrowUp) => "up".into(),
        Key::Named(Named::ArrowDown) => "down".into(),
        Key::Named(Named::ArrowLeft) => "left".into(),
        Key::Named(Named::ArrowRight) => "right".into(),
        Key::Named(Named::Enter) => "enter".into(),
        Key::Named(Named::Tab) => "tab".into(),
        _ => return None,
    })
}

/// Desktop's hotkey list hides the hotkeys of the features that the account does not show.
fn hotkey_shown(st: &BeeperStore, name: &str) -> bool {
    prefs::hotkey_feature(name).is_none_or(|f| st.feature(f).show)
}

/// Desktop's `Rx`: a settings row of a feature shows when the account shows the feature.
fn shown(st: &BeeperStore, r: &prefs::Pref) -> bool {
    r.feature.is_none_or(|f| st.feature(f).show)
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
    for tab in SettingsTab::ALL {
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
        out.extend(SettingsTab::ALL.into_iter().filter(|t| !shown.contains(t)).map(|t| SettingsHit::Section(t, false)));
    }
    out
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
fn network_name(account_id: &str) -> &'static str {
    account_network(account_id).display_name
}

/// Desktop's `userProfileLink`: the web profile of the other person of a chat, on the networks that have one.
fn profile_link(c: &Chat) -> Option<String> {
    use beeper_core::models::NetworkKind::*;
    let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| c.chat_type == "single")?;
    let name = u.username.as_deref().unwrap_or_default();
    Some(match account_network(&c.account_id).kind {
        TwitterX => format!("https://x.com/{name}"),
        Telegram => format!("https://t.me/{name}"),
        Instagram => format!("https://www.instagram.com/{name}/"),
        LinkedIn => format!("https://www.linkedin.com/in/{name}/"),
        Tumblr => format!("https://www.tumblr.com/{name}"),
        IMessage => format!("imessage://{}", u.email.as_deref().or(u.phone_number.as_deref()).unwrap_or_default()),
        _ => return None,
    })
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

/// date-fns `h:mm a` or `HH:mm`, which Desktop formats in English.
fn clock(ms: i64, twenty_four: bool) -> String {
    let (f, hm) = clock_format(ms, twenty_four);
    i18n::en_date(f, (0, 1, 1), hm)
}

/// Desktop's `Cp`: the time in the app language.
fn local_clock(ms: i64, twenty_four: bool) -> String {
    let (f, hm) = clock_format(ms, twenty_four);
    i18n::date(f, (0, 1, 1), hm)
}

fn clock_format(ms: i64, twenty_four: bool) -> (i18n::Date, (i64, i64)) {
    let mins = local_millis(ms).div_euclid(60_000).rem_euclid(1440);
    let f = match (twenty_four, mins < 720) {
        (true, _) => i18n::Date::Clock24,
        (false, true) => i18n::Date::Am,
        (false, false) => i18n::Date::Pm,
    };
    (f, (mins / 60, mins % 60))
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
/// Desktop's `Wh`: with `stamp`, the time and the sender come first. With `many` messages, each one has the sender
/// and the reactions.
fn copy_text(m: &Message, stamp: bool, many: bool) -> String {
    let text = m.text.as_deref().unwrap_or_default();
    let links = m.links.iter().map(|l| l.url.as_str()).filter(|u| !text.contains(u));
    let mut counts: Vec<(&str, usize)> = vec![];
    for r in m.reactions.iter().filter(|_| many) {
        match counts.iter_mut().find(|c| c.0 == r.reaction_key) {
            Some(c) => c.1 += 1,
            None => counts.push((&r.reaction_key, 1)),
        }
    }
    let reactions = (!counts.is_empty())
        .then(|| format!("({})", counts.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(" · ")));
    let body = std::iter::once(text).chain(links).map(str::to_string).chain(reactions);
    let body = body.filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    let name = format!("{}:", m.sender_name.as_deref().unwrap_or("(Unknown)"));
    let head = stamp.then(|| format!("[{}]", locale_string(&m.timestamp))).into_iter();
    head.chain((stamp || many).then_some(name))
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
        0 => tr("Today").into(),
        1 => tr("Yesterday").into(),
        2..=6 => tr(wd).into(),
        _ => {
            let ((y, m, dd), ty) = (civil_from_days(d), civil_from_days(today).0);
            let year = if y == ty { String::new() } else { format!(", {y}") };
            format!("{}, {} {dd}{year}", &wd[..3], i18n::en_short_month(m))
        }
    }
}

fn list_ts(iso: &str, h24: bool) -> String {
    const DAY: i64 = 86_400_000;
    let d = day_num(iso);
    let age = now_unix_millis() - millis_from_rfc3339(iso).expect("RFC 3339 timestamp");
    match today() - d {
        0 => local_clock(iso_millis(iso), h24),
        1 => tr("Yesterday").into(),
        _ if age < 7 * DAY => tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]).into(),
        _ if age < 365 * DAY => i18n::date(i18n::Date::MonthDay, civil_from_days(d), (0, 0)),
        _ => i18n::date(i18n::Date::Medium, civil_from_days(d), (0, 0)),
    }
}

fn human_size(n: u64) -> String {
    let i = (n.max(1) as f64).log(1024.0).floor() as i32;
    let v = format!("{:.1}", n as f64 / 1024f64.powi(i));
    format!("{} {}", v.trim_end_matches(".0"), ["B", "KB", "MB", "GB", "TB"][i as usize])
}

/// `Intl.NumberFormat` with `notation: "compact"`, like Desktop's message counts.
fn compact(n: u64) -> String {
    let Some((d, unit)) = [(1e9, "B"), (1e6, "M"), (1e3, "K")].into_iter().find(|&(d, _)| n as f64 >= d) else {
        return n.to_string();
    };
    let v = n as f64 / d;
    let v = if v < 10.0 { format!("{v:.1}").trim_end_matches(".0").to_string() } else { format!("{v:.0}") };
    format!("{v}{unit}")
}

/// `toLocaleString` of a count, with thousands separators.
fn grouped(n: u64) -> String {
    let s = n.to_string();
    let digits: Vec<String> = s.as_bytes().rchunks(3).rev().map(|c| String::from_utf8_lossy(c).into()).collect();
    digits.join(",")
}

/// Desktop's Storage page: the scan, its chart, and the filters of the clear for all chats and for the open chat.
#[derive(Default)]
struct StoragePage {
    all: Option<Vec<storage::Entry>>,
    chart: Option<svg::Handle>,
    /// Indices into [`STORAGE_DAYS`].
    ages: [usize; 2],
    /// Bits of [`storage::Kind::ALL`].
    kinds: [u8; 2],
    chat: Option<String>,
    /// The [`UiMessage::ClearStorage`] that runs.
    clearing: Option<u8>,
}

/// Settings > Integrations: the "Install to..." choice, the copied field, and whether the community join runs.
#[derive(Default)]
struct IntegrationsPage {
    target: usize,
    copied: Option<u8>,
    joining: bool,
    /// The token of the open revoke confirmation.
    revoke: Option<String>,
    add: Option<AddToken>,
}

/// Desktop's "Create a new token for Beeper Desktop API" dialog, with the token after Create.
struct AddToken {
    name: String,
    expiry: Expiry,
    write: bool,
    token: Option<String>,
    copied: bool,
}

/// Desktop's "Approve connection" dialog of a token request from the Desktop API.
struct TokenAsk {
    id: String,
    /// The client name and version.
    app: String,
    write: bool,
    expiry: Expiry,
    timed_out: bool,
}

/// An "Expires in" choice of a token, in seconds; 0 never expires.
#[derive(Clone, Copy, PartialEq)]
pub struct Expiry(i64);

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

const EXPIRIES: [Expiry; 5] = [Expiry(3600), Expiry(86400), Expiry(604_800), Expiry(2_592_000), Expiry(0)];

/// The Beeper Developer Community room of Desktop.
const COMMUNITY_ROOM: &str = "!VRvJRVNZDbRuKAsKvK:beeper.com";
/// Desktop's MCP clients with their own row, and the button label of each.
const MCP_CLIENTS: [(&str, &str); 3] =
    [("Claude Desktop", "Download Extension"), ("Cursor", "Open in Cursor"), ("VS Code", "Open in Visual Studio Code")];
/// Desktop's other MCP clients, in the "Install to..." select.
const MCP_TARGETS: [&str; 4] = ["Claude Code", "Codex", "Streamable HTTP", "stdio"];

/// Desktop's `buildCopyValue` of an MCP client for the Desktop API at `base`.
fn mcp_value(client: &str, base: &str) -> String {
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
fn storage_totals(all: &[storage::Entry]) -> Vec<(storage::Kind, u64)> {
    let sum = |k| all.iter().filter(|e| e.kind == k).map(|e| e.bytes).sum::<u64>();
    storage::Kind::ALL.into_iter().map(|k| (k, sum(k))).filter(|t| t.1 > 0).collect()
}

/// Desktop's ECharts storage donut, 200 px with rings from 50 to 70 px, clockwise from the top.
fn donut(parts: &[(storage::Kind, u64)]) -> svg::Handle {
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
pub(crate) const CHANGELOG_URL: &str = "https://www.beeper.com/changelog/desktop";
const FEEDBACK: &str = "Tell us what to improve in beeper-rs.";
const ORANGE: Color = Color::from_rgb8(245, 76, 3);
const CHECKED: Color = Color::from_rgb8(14, 77, 242);
const ASK_LOGOUT: u8 = 0;
const ASK_DISMISS_REMINDERS: u8 = 1;
const ASK_DELETE_CHAT: u8 = 2;
const ASK_DELETE_MESSAGE: u8 = 3;
const ASK_ARCHIVE_READ: u8 = 4;
const ASK_ARCHIVE_ALL: u8 = 5;
const ASK_ARCHIVE_WEEK: u8 = 6;
const ASK_MARK_ALL_READ: u8 = 7;
const ASK_CLEAR_STORAGE: u8 = 8;
const ASK_CLEAR_CACHE: u8 = 9;
const ASK_CLEAR_CHAT: u8 = 10;
const ASK_REVOKE_CONNECTION: u8 = 11;
const ASK_REMOTE_ACCESS: u8 = 12;
/// The dialogs over the Settings window.
const SETTINGS_DIALOGS: [u8; 6] =
    [ASK_LOGOUT, ASK_CLEAR_STORAGE, ASK_CLEAR_CACHE, ASK_CLEAR_CHAT, ASK_REVOKE_CONNECTION, ASK_REMOTE_ACCESS];
/// Desktop's "Clear storage older than" choices, where 0 is any age.
const STORAGE_DAYS: [i64; 4] = [0, 30, 90, 120];
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
/// Desktop's `qL` inbox visibility options of an account, with their labels.
const INBOX_OPTIONS: [(&str, &str); 3] = [("all", "All Messages"), ("dmOnly", "Only DMs"), ("none", "None")];
const ACCOUNT_SUB_W: f32 = 160.0;

fn inbox_label(label: &'static str) -> &'static str {
    match label {
        "All Messages" => tr(label),
        _ => trx(label, "inbox visibility option"),
    }
}
/// Desktop's `--chat-view-bubble-padding-x`, which also insets the message footer.
const BUBBLE_PAD_X: f32 = 11.0;
const SUB_MENU_W: [f32; 2] = [124.0, 257.0];
/// Desktop's default `time_suggestion_options`.
const TIME_SUGGESTIONS: [&str; 6] =
    ["In an Hour", "In Two Hours", "This Evening", "Tomorrow", "This Weekend", "Next Week"];

/// Desktop's `wJe`: the time suggestions that fall in the future, with their times.
fn remind_presets(now: i64) -> impl Iterator<Item = (&'static str, i64)> {
    TIME_SUGGESTIONS.into_iter().filter_map(move |s| parse_when_millis(s, now).filter(|at| *at > now).map(|at| (s, at)))
}

/// Desktop's `dD`: the last time phrase that set a reminder, read again now, when it falls in the future.
fn last_used(st: &BeeperStore, now: i64) -> Option<i64> {
    Some(&st.settings.last_time)
        .filter(|t| !t.is_empty())
        .and_then(|t| parse_when_millis(t, now))
        .filter(|at| *at > now)
}

/// Desktop's "Last Used" time: `EEE dd MMM`, with the year when it is not this year, and the time.
fn last_used_label(at: i64, h24: bool) -> String {
    let (y, m, d) = civil_from_days(day_of(at));
    let month = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(' ').nth(m as usize - 1).expect("a month");
    let year = if y == civil_from_days(today()).0 { String::new() } else { format!(" {y}") };
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{} {d:02} {month}{year}, {:0>8}", &wd[..3], clock(at, h24))
}

fn remind_label(at: i64, h24: bool) -> String {
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{}, {:0>8}", &wd[..3], clock(at, h24))
}

/// The day of a reminder time, like Desktop's `Today` or `Tomorrow`.
fn reminder_day(at: i64) -> String {
    let (d, t) = (day_of(at), today());
    match d - t {
        -1 => tr("Yesterday").into(),
        0 => tr("Today").into(),
        1 => tr("Tomorrow").into(),
        _ if (d + 3).div_euclid(7) == (t + 3).div_euclid(7) => tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]).into(),
        _ => i18n::date(i18n::Date::Numeric, civil_from_days(d), (0, 0)),
    }
}

/// Desktop's `nextScheduledMessageTime`.
fn next_send(st: &BeeperStore, chat_id: &str) -> Option<i64> {
    st.scheduled.iter().filter(|s| s.chat_id == chat_id).map(|s| s.send_on).min()
}

/// Desktop's `qN`: when a Send Later message sends.
fn send_day(at: i64, h24: bool) -> String {
    let (d, t, time) = (day_of(at), today(), clock(at, h24));
    match d - t {
        0 => trf("Today at %s", &[&time]),
        1 => trf("Tomorrow at %s", &[&time]),
        _ if (d + 3).div_euclid(7) == (t + 3).div_euclid(7) => {
            trf("%1$s at %2$s", &[&tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]), &time])
        }
        _ => {
            let (y, m, day) = civil_from_days(d);
            trf("%1$s at %2$s", &[&format!("{m}/{day}/{y}"), &time])
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

/// Desktop's `Dh`: the label colors, top to bottom.
const LABEL_COLORS: [(u32, u32); 12] = [
    (0x7E61F2, 0x5634DE),
    (0xA791FF, 0x7D67D5),
    (0xF86FDB, 0xD833CA),
    (0xEF939E, 0xDF6F7C),
    (0xFF4658, 0xEE1F34),
    (0xFA6533, 0xDE4B19),
    (0xFFE500, 0xD99C00),
    (0xB4FB44, 0x6BAD16),
    (0x6FE582, 0x45A654),
    (0x42C6B8, 0x239084),
    (0x29B0E1, 0x0A81AC),
    (0x5583FC, 0x1E54E1),
];

/// Desktop's label icons by id, with the Material Symbols name and path of each.
static LABEL_ICONS: std::sync::LazyLock<HashMap<&str, svg::Handle>> = std::sync::LazyLock::new(|| {
    let icon = |line: &'static str| {
        let [id, _, d] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("a label icon line") };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path fill="currentColor" d="{d}"/></svg>"#
        );
        (id, svg::Handle::from_memory(svg.into_bytes()))
    };
    include_str!("../assets/label_icons.tsv").lines().map(icon).collect()
});

/// Desktop's `LabelSpaceIcon`: the label color with its icon, or the icon alone.
fn label_icon<'a>(l: &Label, size: f32, fg: Color) -> El<'a> {
    let colors = l.color.and_then(|c| LABEL_COLORS.get(usize::from(c)).copied());
    let glyph = (l.icon.is_some() || colors.is_none())
        .then(|| LABEL_ICONS.get(l.icon.as_deref().unwrap_or("3257-5964")))
        .flatten()
        .map(|h| tinted(h, (size * 20.0 / 28.0).round(), if colors.is_some() { Color::WHITE } else { fg }));
    container(glyph)
        .center(size)
        .style(move |_: &Theme| container::Style {
            background: colors.map(|(top, bottom)| {
                let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, hex(top)).add_stop(1.0, hex(bottom));
                Background::Gradient(Gradient::Linear(g))
            }),
            border: border::rounded(8.0),
            ..Default::default()
        })
        .into()
}

/// `ls` is Desktop's `--letter-spacing`; its initials also have `margin-right: -1px`.
fn avatar<'a>(name: &str, size: f32, ls: f32, p: &Palette) -> El<'a> {
    let Some(label) = initials(name) else {
        let shade = fill_box(Color { a: 0.1, ..p.strong }, size / 2.0);
        return container(svg(EYES.clone()).width(size).height(size)).style(shade).into();
    };
    let &(bottom, top) = pick(name, &AVATAR_GRADIENTS);
    let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, hex(top)).add_stop(1.0, hex(bottom));
    let font = crate::style::initials_font();
    let letter = |c: char| txt(c.to_string(), size * 0.5).line_height(1.0).font(font).color(Color::WHITE).into();
    let trail = Padding::ZERO.right((ls - 1.0).max(0.0)).left((1.0 - ls).max(0.0));
    container(container(Row::with_children(label.chars().map(letter)).spacing(ls)).padding(trail))
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

/// Desktop's `queuedPendingMessages`: the sends of a chat that wait for "Allow undo send", and the composer to
/// restore on undo.
struct Queued {
    chat: String,
    due: i64,
    sends: Vec<(String, Outgoing)>,
    text: String,
    reply_to: Option<String>,
    files: Vec<PendingMedia>,
}

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

fn faces(chat: &Chat) -> (Vec<&User>, usize) {
    let mut others: Vec<&User> = chat.participants.items.iter().filter(|u| u.is_self != Some(true)).collect();
    let name = |u: &User| {
        [&u.custom_nickname, &u.full_name, &u.username, &u.email, &u.phone_number]
            .into_iter()
            .find_map(|n| n.as_deref().filter(|n| !n.is_empty()))
            .unwrap_or(&u.id)
            .to_lowercase()
    };
    others.sort_by_cached_key(|u| (u.is_network_bot == Some(true), u.is_admin != Some(true), name(u)));
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

/// The message that the chat list previews: the newest shown message, or a newer reaction.
fn preview_message<'s>(st: &'s BeeperStore, c: &'s Chat) -> Option<&'s Message> {
    let m = st
        .chat_msg_indices
        .get(&c.id)
        .and_then(|v| v.iter().rev().map(|&i| &st.messages[i]).find(|m| m.is_deleted != Some(true) && !m.silent()));
    let reaction = c
        .preview
        .as_ref()
        .filter(|p| p.message_type.as_deref() == Some("REACTION") && m.is_none_or(|m| m.sort_key < p.sort_key));
    reaction.or(m)
}

/// Desktop's `canStartEditMessage` for text: an own message that is not a poll.
fn editable(m: &Message) -> bool {
    m.is_sender == Some(true) && m.poll.is_none()
}

/// The text of a chat list preview, or a label for its links.
fn preview_body(m: &Message) -> Option<String> {
    match m.links.len() {
        _ if m.poll.is_some() => None,
        _ if m.html.is_some() => m.html.as_deref().map(beeper_core::codec::html_text),
        _ if m.text.as_ref().is_some_and(|t| !t.is_empty()) => m.text.clone(),
        _ if !m.attachments.is_empty() => None,
        0 => None,
        1 => Some("🔗 [Link]".to_string()),
        n => Some(format!("🔗 [{n} links]")),
    }
}

/// The text of Desktop's bubble on an unread pinned chat. A new message shows it, and after a minute it hides
/// when the chat has more unread messages than Desktop's limit for its size.
fn bubble_text(st: &BeeperStore, c: &Chat, launched: i64, now: i64) -> Option<String> {
    let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
    let at = c.last_activity.as_deref().map_or(0, |t| millis_from_rfc3339(t).expect("RFC 3339 timestamp"));
    let limit = match c.participants.total {
        0..=2 => 3,
        3..=6 => 5,
        _ => 10,
    };
    if !unread || c.unread_count == 0 || (now - at.max(launched) >= 60_000 && c.unread_count > limit) {
        return None;
    }
    let m = preview_message(st, c)?;
    let tag = || message_tag(m).map(|(_, l, d)| d.map_or_else(|| l.to_string(), |(d, _)| format!("{l} {d}")));
    let text = preview_body(m).or_else(tag)?;
    let first = preview_sender(c, m).and(m.sender_name.as_deref()).and_then(|n| n.split(' ').next());
    Some(first.map_or_else(|| text.clone(), |f| format!("{f}: {text}")))
}

/// Desktop's `PinnedThreadListItem` bubble: at most 8em wide, from 3px right of the avatar's top right corner,
/// centered on the avatar when wider than it, with a half disc below.
fn pin_bubble<'a>(text: &str, mention: bool, (bg, fg): (Color, Color), cell: f32, av: f32, compact: bool) -> El<'a> {
    let max = 80f32.min(cell - 4.0);
    let at_w = if mention { 14.0 } else { 0.0 };
    let w = (crate::style::text_width(text, 10.0, Weight::Normal) + at_w + 8.0).clamp(22.0, max);
    let right = (cell + av - 4.0) / 2.0;
    let x = right + 3.0 + ((w - av - 6.0) / 2.0).max(0.0) - w;
    let (y, h) = if compact { (10.0, 18.56) } else { (17.0, 20.0) };
    let line = row![]
        .push(mention.then(|| ic(icon::AT_SIGN, 10.0).color(fg)))
        .push(ell(text, w - 8.0 - at_w, 10.0, Weight::Normal).color(fg).line_height(Pixels(h - 8.0)))
        .spacing(4)
        .align_y(Center);
    let bubble = container(line).padding(4).width(w).height(h).center_x(w).style(fill_box(bg, 6.0));
    let nub = container(Space::new()).width(6).height(3).style(move |_: &Theme| container::Style {
        background: Some(bg.into()),
        border: iced::Border { radius: iced::border::Radius::default().bottom(3.0), ..Default::default() },
        ..Default::default()
    });
    stack![pin(bubble).x(x).y(y - h / 2.0), pin(nub).x(right - 9.0).y(y + if compact { 9.0 } else { 10.0 })].into()
}

/// The media badge of a chat list preview: whether it is a video, and its label.
/// Desktop's chat list badge for a message's attachments: a `TAG_ICONS` index, the label, and the detail that
/// shows when the message has no text, with whether that detail is italic.
type MediaTag = (usize, &'static str, Option<(String, bool)>);

/// The `TAG_ICONS` index of Desktop's Poll badge, which shows the `poll-options` icon.
const POLL_TAG: usize = 9;

fn message_tag(m: &Message) -> Option<MediaTag> {
    match m.poll {
        Some(_) => Some((POLL_TAG, trx("Poll", "chat list badge for a poll message"), None)),
        None => media_tag(&m.attachments),
    }
}

fn media_tag(attachments: &[Attachment]) -> Option<MediaTag> {
    let [a] = attachments else {
        return (attachments.len() > 1).then(|| (8, tr("Media"), None));
    };
    let file = a.file_name.as_deref().unwrap_or_default().to_lowercase();
    let mime = a.mime_type.as_deref().unwrap_or_default().to_lowercase();
    let name = a.file_name.clone().map(|f| (f, true));
    let length = a.duration.map(|d| (voice::clock(d as f32), false));
    let said = a.transcript.as_ref().map(|t| (format!("“{t}”"), false));
    let image = a.attachment_type == "img" || mime.starts_with("image/");
    Some(match a.attachment_type.as_str() {
        _ if a.is_sticker == Some(true) => (0, tr("Sticker"), None),
        _ if a.is_gif == Some(true) => (0, tr("GIF"), None),
        _ if image => (0, tr("Image"), None),
        "video" => (1, tr("Video"), length),
        "audio" if a.is_voice_note == Some(true) => (2, tr("Voice Note"), said.or(length)),
        "audio" => (3, tr("Audio File"), said.or(name)),
        _ if file.ends_with(".vcf") || mime == "text/x-vcard" => {
            let contact = contact_name(a.file_name.as_deref().unwrap_or_default());
            (4, tr("Contact"), Some((contact, true)))
        }
        _ if mime == "text/csv" || file.ends_with(".csv") => (5, tr("CSV"), name),
        _ if ["application/zip", "application/x-zip-compressed", "multipart/x-zip"].contains(&mime.as_str())
            || file.ends_with(".zip") =>
        {
            (6, tr("ZIP"), name)
        }
        _ => (7, tr("Document"), name),
    })
}

/// Desktop's contact name from a vCard file name, without a leading `123.ext.` and the `.vcf`.
fn contact_name(f: &str) -> String {
    let rest = f.split_once('.').filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let rest = rest
        .and_then(|(_, r)| r.split_once('.'))
        .filter(|(w, _)| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    rest.map_or(f, |(_, r)| r).replacen(".vcf", "", 1)
}

/// Desktop's chat list badge icons: image, videocam, volume_up, audio_file, account_circle, csv, folder_zip,
/// draft and photo_library, at its `scale(1.2)` in a 12 px box.
static TAG_ICONS: std::sync::LazyLock<[svg::Handle; 9]> = std::sync::LazyLock::new(|| {
    [
        "M200-120q-33 0-56.5-23.5T120-200v-560q0-33 23.5-56.5T200-840h560q33 0 56.5 23.5T840-760v560q0 33-23.5 56.5T760-120H200Zm0-80h560v-560H200v560Zm40-80h480L570-480 450-320l-90-120-120 160Zm-40 80v-560 560Z",
        "M160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h480q33 0 56.5 23.5T720-720v180l160-160v440L720-420v180q0 33-23.5 56.5T640-160H160Zm0-80h480v-480H160v480Zm0 0v-480 480Z",
        "M560-131v-82q90-26 145-100t55-168q0-94-55-168T560-749v-82q124 28 202 125.5T840-481q0 127-78 224.5T560-131ZM120-360v-240h160l200-200v640L280-360H120Zm440 40v-322q47 22 73.5 66t26.5 96q0 51-26.5 94.5T560-320ZM400-606l-86 86H200v80h114l86 86v-252ZM300-480Z",
        "M430-200q38 0 64-26t26-64v-150h120v-80H480v155q-11-8-23.5-11.5T430-380q-38 0-64 26t-26 64q0 38 26 64t64 26ZM240-80q-33 0-56.5-23.5T160-160v-640q0-33 23.5-56.5T240-880h320l240 240v480q0 33-23.5 56.5T720-80H240Zm280-520v-200H240v640h480v-440H520ZM240-800v200-200 640-640Z",
        "M234-276q51-39 114-61.5T480-360q69 0 132 22.5T726-276q35-41 54.5-93T800-480q0-133-93.5-226.5T480-800q-133 0-226.5 93.5T160-480q0 59 19.5 111t54.5 93Zm246-164q-59 0-99.5-40.5T340-580q0-59 40.5-99.5T480-720q59 0 99.5 40.5T620-580q0 59-40.5 99.5T480-440Zm0 360q-83 0-156-31.5T197-197q-54-54-85.5-127T80-480q0-83 31.5-156T197-763q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480q0 83-31.5 156T763-197q-54 54-127 85.5T480-80Zm0-80q53 0 100-15.5t86-44.5q-39-29-86-44.5T480-280q-53 0-100 15.5T294-220q39 29 86 44.5T480-160Zm0-360q26 0 43-17t17-43q0-26-17-43t-43-17q-26 0-43 17t-17 43q0 26 17 43t43 17Zm0-60Zm0 360Z",
        "M230-360h120v-60H250v-120h100v-60H230q-17 0-28.5 11.5T190-560v160q0 17 11.5 28.5T230-360Zm156 0h120q17 0 28.5-11.5T546-400v-60q0-17-11.5-31.5T506-506h-60v-34h100v-60H426q-17 0-28.5 11.5T386-560v60q0 17 11.5 30.5T426-456h60v36H386v60Zm264 0h60l70-240h-60l-40 138-40-138h-60l70 240ZM160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h640q33 0 56.5 23.5T880-720v480q0 33-23.5 56.5T800-160H160Zm0-80h640v-480H160v480Zm0 0v-480 480Z",
        "M640-480v-80h80v80h-80Zm0 80h-80v-80h80v80Zm0 80v-80h80v80h-80ZM447-640l-80-80H160v480h400v-80h80v80h160v-400H640v80h-80v-80H447ZM160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h240l80 80h320q33 0 56.5 23.5T880-640v400q0 33-23.5 56.5T800-160H160Zm0-80v-480 480Z",
        "M240-80q-33 0-56.5-23.5T160-160v-640q0-33 23.5-56.5T240-880h320l240 240v480q0 33-23.5 56.5T720-80H240Zm280-520v-200H240v640h480v-440H520ZM240-800v200-200 640-640Z",
        "M360-400h400L622-580l-92 120-62-80-108 140Zm-40 160q-33 0-56.5-23.5T240-320v-480q0-33 23.5-56.5T320-880h480q33 0 56.5 23.5T880-800v480q0 33-23.5 56.5T800-240H320Zm0-80h480v-480H320v480ZM160-80q-33 0-56.5-23.5T80-160v-560h80v560h560v80H160Zm160-720v480-480Z",
    ]
    .map(|d| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 -880 800 800"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});

fn preview_sender<'a>(chat: &'a Chat, m: &Message) -> Option<&'a User> {
    let shown = (chat.chat_type == "group" || network_bot(chat, &m.sender_id))
        && m.is_sender != Some(true)
        && m.message_type.as_deref() != Some("REACTION");
    chat.participants.items.iter().find(|u| shown && u.id == m.sender_id)
}

/// The 2 px cutout around a 12 or 16 px avatar badge, clipped at its right and bottom where no avatar is.
static BADGE_RINGS: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    [12, 16].map(|b| {
        let (r, s) = (b / 2 + 2, b + 2);
        let ring = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}"><circle cx="{r}" cy="{r}" r="{r}" fill="currentColor"/></svg>"#
        );
        svg::Handle::from_memory(ring.into_bytes())
    })
});

/// `ring` is the row color, which Desktop shows in a cutout of the avatar around the badge.
fn badged<'a>(inner: El<'a>, account_id: &str, size: f32, badge: f32, mono: Option<&Palette>, ring: Color) -> El<'a> {
    let icon: El<'a> = match mono {
        None => svg(crate::style::brand_round(brand_key(account_id))).width(badge).height(badge).into(),
        Some(p) => {
            let fg = p.strong;
            let glyph = svg(crate::style::brand_badge(brand_key(account_id)))
                .style(move |_, _| svg::Style { color: Some(fg) })
                .width(badge)
                .height(badge);
            container(glyph).style(fill_box(p.chip, badge / 2.0)).into()
        }
    };
    let cutout = tinted(&BADGE_RINGS[usize::from(badge > 12.0)], badge + 2.0, ring);
    let w = size + badge / 8.0;
    let corner = |e: El<'a>| container(e).width(w).height(size).align_right(w).align_bottom(size);
    stack![container(inner), corner(cutout), corner(icon)].width(w).height(size).into()
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
    sized_select(p, options, selected, f, 11.0, Padding::from([4, 8]))
}

/// A select with the arrow of Chromium's native select over its right side.
fn sized_select<'a, T: ToString + PartialEq + Clone + 'a>(
    p: &Palette,
    options: impl std::borrow::Borrow<[T]> + 'a,
    selected: T,
    f: impl Fn(T) -> UiMessage + 'a,
    size: f32,
    pad: Padding,
) -> El<'a> {
    let list = pick_list(options, Some(selected), f)
        .text_size(size)
        .text_line_height(Pixels(16.0))
        .font(fw(Weight::Semibold))
        .handle(pick_list::Handle::None)
        .padding(pad)
        .style(pick_style(p.chip, p.chip, p.strong, p.strong))
        .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border));
    let chevron = container(tinted(&SELECT_ARROW, 10.0, p.strong)).align_right(Fill).center_y(Fill).padding([0, 8]);
    stack![list, chevron].into()
}

/// The bold arrow of Chromium's native select, which Desktop's Settings use.
/// A static icon from SVG bytes.
macro_rules! svg_static {
    ($name:ident, $svg:expr) => {
        static $name: std::sync::LazyLock<svg::Handle> =
            std::sync::LazyLock::new(|| svg::Handle::from_memory(&$svg[..]));
    };
}

svg_static!(CHEVRON_LEFT, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="m15 18-6-6 6-6" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##);
svg_static!(CLAUDE_LOGO, include_bytes!("../assets/integrations/claude.svg"));
svg_static!(CURSOR_LOGO, include_bytes!("../assets/integrations/cursor.svg"));
svg_static!(VSCODE_LOGO, include_bytes!("../assets/integrations/vscode.svg"));
svg_static!(COPY_ICON, include_bytes!("../assets/integrations/copy.svg"));
svg_static!(COPIED_ICON, include_bytes!("../assets/integrations/done.svg"));
svg_static!(ADD_ICON, include_bytes!("../assets/integrations/add.svg"));

/// date-fns `formatDistance` in English, as Desktop shows the age and the expiry of a connection.
fn distance(ms: i64) -> String {
    let n = |v: i64, unit: &str| format!("{v} {unit}{}", if v == 1 { "" } else { "s" });
    let m = (ms.abs() as f64 / 60_000.0).round() as i64;
    let months = (m as f64 / 43_200.0).round() as i64;
    match m {
        0 => "less than a minute".into(),
        1 => "1 minute".into(),
        2..45 => n(m, "minute"),
        45..90 => "about 1 hour".into(),
        90..1440 => format!("about {}", n((m as f64 / 60.0).round() as i64, "hour")),
        1440..2520 => "1 day".into(),
        2520..43_200 => n((m as f64 / 1440.0).round() as i64, "day"),
        43_200..86_400 => format!("about {}", n(months, "month")),
        _ if months < 12 => n(months, "month"),
        _ => {
            let (years, rest) = (months / 12, months % 12);
            match rest {
                0..3 => format!("about {}", n(years, "year")),
                3..9 => format!("over {}", n(years, "year")),
                _ => format!("almost {}", n(years + 1, "year")),
            }
        }
    }
}
svg_static!(SELECT_ARROW, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M1 3.25 5 7.25 9 3.25" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##);

/// Desktop's "Feedback?" button, which opens the feedback form.
fn feedback_btn<'a>(p: &Palette) -> El<'a> {
    let rim = Color { a: 0.1, ..p.strong };
    button(container(txt(tr("Feedback?"), 11.0)).center(Fill))
        .width(74)
        .height(24)
        .padding(0)
        .style(bordered_btn([p.card, p.hover, p.card], [Color { a: 0.9, ..rim }; 3], rim, 1.0, 4.0))
        .on_press(UiMessage::OpenQuickLook("Feedback".into(), FEEDBACK.into()))
        .into()
}

fn pill_btn<'a>(p: &Palette, label: impl ToString, msg: UiMessage) -> El<'a> {
    let label = txt(label.to_string(), 13.0).line_height(1.0).wrapping(iced::widget::text::Wrapping::None);
    button(label).padding([4, 6]).style(pill_style(p)).on_press(msg).into()
}

fn pill_style(p: &Palette) -> impl Fn(&Theme, button::Status) -> button::Style + use<> {
    bordered_btn([p.button; 3], [p.strong; 3], p.button_border, 1.0, 6.0)
}

/// Desktop's `.styled-inputs input`.
fn outline_input(p: &Palette) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + use<> {
    let (edge, text, muted, accent) = (p.neutral_border, p.text, p.muted, p.accent);
    move |_, _| text_input::Style {
        background: CLEAR.into(),
        border: iced::Border { color: edge, width: 1.0, radius: 6.0.into() },
        icon: muted,
        placeholder: muted,
        value: text,
        selection: accent.scale_alpha(0.45),
    }
}

/// The groups of `items` in the order of their first item, as Desktop's `groupBy` gives them.
fn first_groups<T>(items: &[T], group: impl Fn(&T) -> &'static str) -> Vec<&'static str> {
    items.iter().map(group).fold(Vec::new(), |mut v, g| {
        if !v.contains(&g) {
            v.push(g);
        }
        v
    })
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
        prefs::Badge::PlusLabs => {
            row![badge(p, prefs::Badge::Plus), badge(p, prefs::Badge::Labs)].spacing(6).align_y(Center).into()
        }
    }
}

const THEME_WINDOW: &str = r##"<rect x="10" y="7" width="76" height="47" rx="4" fill="WIN"/><circle cx="15" cy="12" r="1.4" fill="#ff5f57"/><circle cx="19.5" cy="12" r="1.4" fill="#febc2e"/><circle cx="24" cy="12" r="1.4" fill="#28c840"/><rect x="36" y="16" width="24" height="3" rx="1.5" fill="LINE"/><rect x="14" y="23" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="28" width="68" height="11" rx="2.5" fill="#0d4ffb"/><circle cx="19.5" cy="33.5" r="3" fill="#fff"/><rect x="25" y="31" width="28" height="1.8" rx=".9" fill="#fff"/><rect x="25" y="34.6" width="48" height="1.5" rx=".75" fill="#bcd2ff"/><rect x="14" y="43" width="68" height="2" rx="1" fill="LINE"/><rect x="14" y="48" width="48" height="2" rx="1" fill="LINE"/>"##;

svg_static!(DASHED_RING, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="1" y="1" width="30" height="30" rx="7" fill="none" stroke="#000" stroke-width="2" stroke-dasharray="6 3" stroke-dashoffset="8.5"/></svg>"##);

static SENT: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    [
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2" stroke-dasharray="2.4 1.84" transform="rotate(-79 6 6)"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
    ]
    .map(svg::Handle::from_memory)
});

svg_static!(SEND_ARROW, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path transform="rotate(90 10 10)" d="m6.52 10.833 4.668 4.667L10 16.666 3.333 10 10 3.333 11.188 4.5 6.52 9.166h10.146v1.667z"/></svg>"##);
svg_static!(EYES, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 46" fill="#fff"><path d="M22.1197 25.5092C23.0085 30.1093 20.937 33.1 16.2324 34.009C11.5277 34.918 8.49126 32.9143 7.60244 28.3142L6.68822 23.5827C5.7994 18.9826 7.87085 15.9919 12.5755 15.0829C17.2802 14.1738 20.3166 16.1776 21.2054 20.7777L22.1197 25.5092ZM14.8746 30.5902C15.1743 32.1411 16.6446 33.1113 18.2307 32.8049C19.8168 32.4984 20.8199 31.0503 20.5202 29.4994C20.2205 27.9485 18.7503 26.9783 17.1641 27.2847C15.578 27.5912 14.575 29.0393 14.8746 30.5902Z"/><path d="M39.1455 22.1235C40.0344 26.7236 37.9629 29.7142 33.2583 30.6233C28.5536 31.5323 25.5171 29.5286 24.6283 24.9284L23.7141 20.1969C22.8253 15.5968 24.8967 12.6061 29.6014 11.6971C34.306 10.7881 37.3425 12.7918 38.2313 17.3919L39.1455 22.1235ZM31.9005 27.2045C32.2002 28.7554 33.6704 29.7256 35.2566 29.4191C36.8427 29.1126 37.8457 27.6645 37.5461 26.1136C37.2464 24.5627 35.7761 23.5925 34.19 23.899C32.6039 24.2055 31.6008 25.6536 31.9005 27.2045Z"/></svg>"##);

static REPLY_CURVE: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    ["", r#" transform="matrix(-1 0 0 1 26 0)""#].map(|flip| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0.5 0.5 25 19"><path{flip} fill="currentColor" fill-rule="evenodd" d="M17 15.5C9.544 15.5 3.5 9.456 3.5 2V.5h-3V2c0 9.113 7.387 16.5 16.5 16.5h6a1.5 1.5 0 0 0 0-3z"/></svg>"#).into_bytes())
    })
});

svg_static!(TUNE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M9.167 17.5v-5h1.666v1.667H17.5v1.666h-6.667V17.5zM2.5 15.833v-1.666h5v1.666zM5.833 12.5v-1.667H2.5V9.167h3.333V7.5H7.5v5zm3.334-1.667V9.167H17.5v1.666zM12.5 7.5v-5h1.667v1.667H17.5v1.666h-3.333V7.5zm-10-1.667V4.167h8.333v1.666z"/></svg>"##);

svg_static!(PIN, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21.436 7.586l-3.998-4.02c-.752-.756-2.063-.764-2.83-.006-.196.196-.35.436-.418.629-.653 1.362-1.354 2.215-2.254 2.727l-.217.105c-.968.485-2.285.979-4.719.979-.266 0-.521.052-.766.152-.484.202-.879.595-1.082 1.084-.199.484-.199 1.041 0 1.525.104.249.25.471.435.651l3.235 3.235-3.822 5.353 5.352-3.822 3.227 3.227c.186.189.406.339.656.441.247.103.503.154.766.154s.519-.052.765-.154c.498-.205.883-.592 1.08-1.078.103-.242.155-.507.155-.768 0-2.436.494-3.752.978-4.721.496-.992 1.369-1.748 2.754-2.414.271-.104.51-.256.711-.457.772-.782.768-2.051-.008-2.822zm-5.248 4.801c-.819 1.643-1.188 3.37-1.195 5.604l-7.993-7.991c2.139 0 3.814-.335 5.396-1.084l.235-.105c1.399-.699 2.468-1.893 3.388-3.834l3.924 4.051c-1.863.893-3.056 1.96-3.755 3.359z"/></svg>"##);

/// Desktop's `TabletIcon`, `DesktopIcon` and `MobileIcon`.
static DEVICE_ICONS: std::sync::LazyLock<[svg::Handle; 3]> = std::sync::LazyLock::new(|| {
    [
        ("0 0 14 19", "M0.0605469 16.6621V2.35156C0.0605469 1.69303 0.270833 1.16178 0.691406 0.757812C1.11751 0.353841 1.67367 0.151855 2.35986 0.151855H11.6318C12.318 0.151855 12.8714 0.353841 13.292 0.757812C13.7181 1.16178 13.9312 1.69303 13.9312 2.35156V16.6621C13.9312 17.3151 13.7181 17.8436 13.292 18.2476C12.8714 18.6515 12.318 18.8535 11.6318 18.8535H2.35986C1.67367 18.8535 1.11751 18.6515 0.691406 18.2476C0.270833 17.8436 0.0605469 17.3151 0.0605469 16.6621ZM1.39697 16.438C1.39697 16.7756 1.49658 17.0384 1.6958 17.2266C1.89502 17.4202 2.17171 17.5171 2.52588 17.5171H11.4658C11.82 17.5171 12.0967 17.4202 12.2959 17.2266C12.4951 17.0384 12.5947 16.7756 12.5947 16.438V2.57568C12.5947 2.23258 12.4951 1.96696 12.2959 1.77881C12.0967 1.58512 11.82 1.48828 11.4658 1.48828H2.52588C2.17171 1.48828 1.89502 1.58512 1.6958 1.77881C1.49658 1.96696 1.39697 2.23258 1.39697 2.57568V16.438ZM4.64258 16.8862C4.5319 16.8862 4.44059 16.8503 4.36865 16.7783C4.29671 16.7064 4.26074 16.6123 4.26074 16.4961C4.26074 16.3799 4.29671 16.2858 4.36865 16.2139C4.44059 16.1475 4.5319 16.1143 4.64258 16.1143H9.34912C9.46533 16.1143 9.55941 16.1475 9.63135 16.2139C9.70329 16.2858 9.73926 16.3799 9.73926 16.4961C9.73926 16.6123 9.70329 16.7064 9.63135 16.7783C9.55941 16.8503 9.46533 16.8862 9.34912 16.8862H4.64258Z"),
        ("0 0 17 10", "M0.570801 8.96777C0.570801 8.76725 0.642415 8.59538 0.785645 8.45215C0.932454 8.30534 1.10612 8.23193 1.30664 8.23193H2.41309V1.92627C2.41309 1.47868 2.53662 1.13672 2.78369 0.900391C3.03434 0.664062 3.3763 0.545898 3.80957 0.545898H13.3916C13.8392 0.545898 14.1829 0.664062 14.4229 0.900391C14.6628 1.13672 14.7827 1.47868 14.7827 1.92627V8.23193H15.8892C16.0933 8.23193 16.2669 8.30534 16.4102 8.45215C16.557 8.59538 16.6304 8.76725 16.6304 8.96777C16.6304 9.17188 16.557 9.34554 16.4102 9.48877C16.2669 9.63558 16.0933 9.70898 15.8892 9.70898H1.30664C1.10612 9.70898 0.932454 9.63558 0.785645 9.48877C0.642415 9.34554 0.570801 9.17188 0.570801 8.96777ZM3.61084 8.23193H13.5903V2.22705C13.5903 2.06234 13.5492 1.94059 13.4668 1.86182C13.3844 1.77946 13.2627 1.73828 13.1016 1.73828H4.09961C3.93848 1.73828 3.81673 1.77946 3.73438 1.86182C3.65202 1.94059 3.61084 2.06234 3.61084 2.22705V8.23193Z"),
        ("0 0 12 19", "M2.92041 18.2891C2.24528 18.2891 1.7002 18.0898 1.28516 17.6914C0.870117 17.2985 0.662598 16.7783 0.662598 16.1309V2.87451C0.662598 2.22705 0.870117 1.70687 1.28516 1.31396C1.7002 0.915527 2.24528 0.716309 2.92041 0.716309H9.07129C9.75195 0.716309 10.297 0.915527 10.7065 1.31396C11.1216 1.70687 11.3291 2.22705 11.3291 2.87451V16.1309C11.3291 16.7783 11.1216 17.2985 10.7065 17.6914C10.297 18.0898 9.75195 18.2891 9.07129 18.2891H2.92041ZM3.09473 16.9526H8.90527C9.25391 16.9526 9.5223 16.8613 9.71045 16.6787C9.8986 16.5016 9.99268 16.2471 9.99268 15.915V3.09033C9.99268 2.7583 9.8986 2.50374 9.71045 2.32666C9.5223 2.14404 9.25391 2.05273 8.90527 2.05273H3.09473C2.74056 2.05273 2.4694 2.14404 2.28125 2.32666C2.0931 2.50374 1.99902 2.7583 1.99902 3.09033V15.915C1.99902 16.2471 2.0931 16.5016 2.28125 16.6787C2.4694 16.8613 2.74056 16.9526 3.09473 16.9526ZM4.24023 16.3467C4.12956 16.3467 4.03548 16.3107 3.95801 16.2388C3.88607 16.1668 3.8501 16.0728 3.8501 15.9565C3.8501 15.8403 3.88607 15.7463 3.95801 15.6743C4.03548 15.6079 4.12956 15.5747 4.24023 15.5747H7.76807C7.87874 15.5747 7.97005 15.6079 8.04199 15.6743C8.11393 15.7463 8.1499 15.8403 8.1499 15.9565C8.1499 16.0728 8.11393 16.1668 8.04199 16.2388C7.97005 16.3107 7.87874 16.3467 7.76807 16.3467H4.24023ZM4.99561 3.8125C4.82959 3.8125 4.68848 3.75716 4.57227 3.64648C4.46159 3.53027 4.40625 3.38916 4.40625 3.22314C4.40625 3.06266 4.46159 2.92708 4.57227 2.81641C4.68848 2.7002 4.82959 2.64209 4.99561 2.64209H7.00439C7.16488 2.64209 7.30046 2.7002 7.41113 2.81641C7.52734 2.92708 7.58545 3.06266 7.58545 3.22314C7.58545 3.38916 7.52734 3.53027 7.41113 3.64648C7.30046 3.75716 7.16488 3.8125 7.00439 3.8125H4.99561Z"),
    ]
    .map(|(view, d)| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path d="{d}"/></svg>"#).into_bytes())
    })
});
svg_static!(QR_CODE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="5" height="5" x="3" y="3" rx="1"/><rect width="5" height="5" x="16" y="3" rx="1"/><rect width="5" height="5" x="3" y="16" rx="1"/><path d="M21 16h-3a2 2 0 0 0-2 2v3"/><path d="M21 21v.01"/><path d="M12 7v3a2 2 0 0 1-2 2H7"/><path d="M3 12h.01"/><path d="M12 3h.01"/><path d="M12 16v.01"/><path d="M16 12h1"/><path d="M21 12v.01"/><path d="M12 21v-1"/></svg>"##);
svg_static!(BELL_OFF, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742"/><path d="m2 2 20 20"/><path d="M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"/></svg>"##);

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

svg_static!(DRAG_HANDLE, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path fill="currentColor" d="M160-360v-80h640v80H160Zm0-160v-80h640v80H160Z"/></svg>"#);
svg_static!(DELETE, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path fill="currentColor" d="M280-120q-33 0-56.5-23.5T200-200v-520h-40v-80h200v-40h240v40h200v80h-40v520q0 33-23.5 56.5T680-120H280Zm400-600H280v520h400v-520ZM360-280h80v-360h-80v360Zm160 0h80v-360h-80v360ZM280-720v520-520Z"/></svg>"#);
svg_static!(CHEVRON_RIGHT, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M10.5 10 6.667 6.167 7.833 5l5 5-5 5-1.166-1.167z"/></svg>"#);
svg_static!(POLL_ENDED, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M4 16V4h12v12z"/></svg>"#);
svg_static!(POLL_CHECK, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="none" stroke="#000" stroke-width="1.5" d="M4.64 7.83 6.94 10.13 11.36 5.71"/></svg>"##);
svg_static!(CHEVRON_DOWN, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="m10 10.5 3.833-3.833L15 7.834l-5 5-5-5 1.167-1.167z"/></svg>"#);
svg_static!(TRASH, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><path fill="currentColor" d="M432 32H312l-9.4-18.7A24 24 0 0 0 281.1 0H166.8a23.72 23.72 0 0 0-21.4 13.3L136 32H16A16 16 0 0 0 0 48v32a16 16 0 0 0 16 16h416a16 16 0 0 0 16-16V48a16 16 0 0 0-16-16zM53.2 467a48 48 0 0 0 47.9 45h245.8a48 48 0 0 0 47.9-45L416 128H32z"/></svg>"#);

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

svg_static!(DROP_ARROW, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M12.625 14.375 17 10l-4.375-4.375-1.062 1.063 2.562 2.562H4.5V7H3v3.75h11.125l-2.562 2.563z"/></svg>"##);

svg_static!(SYS_ARROW_BACK, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="m6.52 10.833 4.668 4.667L10 16.666 3.333 10 10 3.333 11.188 4.5 6.52 9.166h10.146v1.667z"/></svg>"##);

svg_static!(SYS_ROTATE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M10 18.25q-1.083 0-2.052-.302a6.9 6.9 0 0 1-1.802-.865L7.229 16a5.5 5.5 0 0 0 2.771.75q2.271 0 3.885-1.615Q15.5 13.521 15.5 11.25t-1.615-3.906-4.01-1.594l1.188 1.188L10 8 7 5l3-3 1.063 1.063L9.875 4.25q3-.063 5.063 2.01T17 11.25q0 1.458-.552 2.73a7.1 7.1 0 0 1-1.5 2.218 7.1 7.1 0 0 1-2.219 1.5A6.8 6.8 0 0 1 10 18.25m-5-2-5-5 5-5 5 5zm0-2.125 2.875-2.875L5 8.375 2.125 11.25z"/></svg>"##);

svg_static!(SYS_DOWNLOAD, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M10 13.333 5.833 9.166 7 7.958l2.167 2.167V3.333h1.666v6.792L13 7.958l1.167 1.208zm-5 3.333q-.687 0-1.177-.49A1.6 1.6 0 0 1 3.333 15v-2.5H5V15h10v-2.5h1.667V15q0 .687-.49 1.177t-1.177.49z"/></svg>"##);

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

svg_static!(MAT_FORWARD, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="m640-280-57-56 184-184-184-184 57-56 240 240-240 240ZM80-200v-160q0-83 58.5-141.5T280-560h247L383-704l57-56 240 240-240 240-57-56 144-144H280q-50 0-85 35t-35 85v160H80Z"/></svg>"##);

svg_static!(ZOOM_LEVEL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 22 15" fill="#000"><path d="M20.64 14.343C21.01 14.343 21.303 14.237 21.52 14.026C21.736 13.816 21.845 13.534 21.845 13.183C21.845 12.937 21.798 12.679 21.705 12.409L17.995 2.346C17.837 1.877 17.603 1.53 17.292 1.304C16.982 1.079 16.583 0.966 16.097 0.966C15.177 0.966 14.562 1.423 14.251 2.336L10.498 12.488C10.4122 12.7219 10.3675 12.9689 10.366 13.218C10.366 13.563 10.469 13.838 10.674 14.04C10.879 14.242 11.16 14.343 11.518 14.343C11.863 14.343 12.134 14.261 12.331 14.097C12.527 13.933 12.675 13.666 12.774 13.297L13.636 10.915H18.531L19.393 13.297C19.493 13.666 19.64 13.933 19.836 14.097C20.033 14.261 20.301 14.343 20.641 14.343H20.64ZM17.995 8.999H14.198L16.07 3.479H16.123L17.995 9V8.999ZM7.87 14.325C8.175 14.325 8.42 14.233 8.608 14.048C8.7018 13.9545 8.77527 13.8427 8.8238 13.7195C8.87232 13.5962 8.89486 13.4643 8.89 13.332C8.89 13.209 8.878 13.09 8.854 12.976C8.8266 12.8487 8.78846 12.7239 8.74 12.603L6.14 5.606C5.987 5.19 5.782 4.883 5.524 4.684C5.267 4.484 4.939 4.384 4.54 4.384C4.165 4.384 3.85 4.484 3.595 4.684C3.34 4.883 3.137 5.19 2.985 5.606L0.381 12.603C0.292023 12.8359 0.244637 13.0827 0.241 13.332C0.241 13.642 0.332 13.886 0.517 14.062C0.702 14.237 0.952 14.325 1.269 14.325C1.532 14.325 1.746 14.25 1.91 14.101C2.074 13.951 2.203 13.707 2.297 13.367L2.851 11.741H6.261L6.814 13.385C6.902 13.725 7.031 13.966 7.201 14.11C7.371 14.253 7.594 14.325 7.869 14.325H7.87ZM5.787 10.16H3.31L4.45 6.494H4.626L5.786 10.159L5.787 10.16Z"/></svg>"##);
svg_static!(MAT_LINK, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M440-280H280q-83 0-141.5-58.5T80-480q0-83 58.5-141.5T280-680h160v80H280q-50 0-85 35t-35 85q0 50 35 85t85 35h160v80ZM320-440v-80h320v80H320Zm200 160v-80h160q50 0 85-35t35-85q0-50-35-85t-85-35H520v-80h160q83 0 141.5 58.5T880-480q0 83-58.5 141.5T680-280H520Z"/></svg>"##);

/// Desktop's system icon arrow-back.
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
    if st.feature("labels-v2").on {
        let mut labels: Vec<&Label> = st.labels.iter().collect();
        labels.sort_by_cached_key(|l| l.name.to_lowercase());
        accounts.extend(labels.into_iter().map(|l| format!("label:{}", l.id)));
    }
    accounts
}

/// Finds the label of a rail entry.
fn rail_label<'a>(st: &'a BeeperStore, id: &str) -> Option<&'a Label> {
    let id = id.strip_prefix("label:")?;
    st.labels.iter().find(|l| l.id == id)
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
    if let Some(l) = rail_label(st, id) {
        return c.label_ids.contains(&l.id);
    }
    match network_space(st, id) {
        Some(n) => c.account_id == n.account_id && c.network_id.as_ref() == Some(&n.id),
        None => c.account_id == id && c.network_id.as_ref().is_none_or(|n| network_space(st, n).is_none()),
    }
}

/// Desktop's `rootMessageIDs`: the first message of the reply chain of `m`, which can be a message that is not loaded.
fn reply_root<'s>(st: &'s BeeperStore, mut m: &'s Message) -> &'s str {
    let mut seen = HashSet::new();
    while let Some(r) = m.linked_message_id.as_deref().filter(|r| *r != m.id && seen.insert(&m.id)) {
        match st.msg_idx_by_id.get(r) {
            Some(&i) => m = &st.messages[i],
            None => return r,
        }
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
                many => Some(trn("%d Account", "%d Accounts", many.len(), &[&many.len()])),
            },
            1 => {
                let label = match (self.sent, self.group) {
                    (None, None) => return None,
                    (Some(true), None) => "Sent messages",
                    (Some(false), None) => "Received messages",
                    (None, Some(true)) => "Group messages",
                    (None, Some(false)) => "Direct messages",
                    (Some(true), Some(true)) => "Sent group messages",
                    (Some(true), Some(false)) => "Sent direct messages",
                    (Some(false), Some(true)) => "Received group messages",
                    (Some(false), Some(false)) => "Received direct messages",
                };
                Some(tr(label).into())
            }
            2 => self.period.map(|i| tr(PERIODS[i]).to_string()),
            _ => (!self.media.is_empty())
                .then(|| self.media.iter().map(|&k| tr(MEDIA_KINDS[k])).collect::<Vec<_>>().join(", ")),
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

/// The `width` and `height` attributes of an SVG document.
fn svg_size(doc: &[u8]) -> (f32, f32) {
    let doc = std::str::from_utf8(doc).expect("UTF-8 SVG");
    let attr = |name: &str| -> f32 {
        let at = doc.find(&format!(" {name}=\"")).unwrap_or_else(|| panic!("the SVG has a {name}")) + name.len() + 3;
        doc[at..].split('"').next().and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("the SVG {name} is a number"))
    };
    (attr("width"), attr("height"))
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

/// Finds the offset of a scrollable that shows a container: in the middle, or with the least scroll when `nearest`.
/// The offset of the timeline counts from its bottom, because it is anchored there.
struct Locate(
    iced::widget::Id,
    Option<(iced::Rectangle, iced::Rectangle, f32)>,
    Option<iced::Rectangle>,
    bool,
    &'static str,
);

fn locate(target: impl Into<iced::widget::Id>, scroll: &'static str, nearest: bool) -> Task<UiMessage> {
    iced::advanced::widget::operate(Locate(target.into(), None, None, nearest, scroll))
        .then(move |y| operation::scroll_to(scroll, operation::AbsoluteOffset { x: 0.0, y }))
}

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
        if id == Some(&iced::widget::Id::new(self.4)) {
            self.1 = Some((bounds, content, at.y));
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<f32> {
        let (view, content, at) = self.1.unwrap_or_else(|| panic!("{} is shown", self.4));
        let msg = self.2.unwrap_or_else(|| panic!("{:?} is in {}", self.0, self.4));
        let max = (content.height - view.height).max(0.0);
        let y = msg.y - content.y;
        let top = match self.3 {
            true => at.max(y + msg.height - view.height).min(y),
            false => y - (view.height - msg.height) / 2.0,
        };
        let top = top.clamp(0.0, max);
        iced::advanced::widget::operation::Outcome::Some(if self.4 == "timeline" { max - top } else { top })
    }
}

/// The forward dialog: the messages in timeline order, the picked chats, and the search text.
struct Forward {
    mids: Vec<String>,
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
    let ymd = civil_from_days(d);
    match t - d {
        0 => tr("Today").into(),
        1 => tr("Yesterday").into(),
        _ if d >= week => tr("This week").into(),
        _ if d >= week - 7 => tr("Last week").into(),
        _ if ymd.0 == civil_from_days(t).0 => i18n::date(i18n::Date::Month, ymd, (0, 0)),
        _ => i18n::date(i18n::Date::MonthYear, ymd, (0, 0)),
    }
}

/// The en-US medium date of a day, like "Oct 17, 2026".
fn medium_date(d: i64) -> String {
    i18n::en_date(i18n::Date::Medium, civil_from_days(d), (0, 0))
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
        m if m.starts_with("audio/") => tr("Audio"),
        "application/pdf" => tr("PDF"),
        m if m.starts_with("text/") => tr("Text"),
        m if m.starts_with("application/") => tr("Document"),
        _ => tr("File"),
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
svg_static!(TAKE_BEEPER, include_bytes!("../assets/take_beeper.svg"));
// Desktop's QR code of `https://www.beeper.com/download`.
svg_static!(APP_QR, include_bytes!("../assets/app_qr.svg"));

pub fn short_title(c: &Chat) -> &str {
    let first_word = c.chat_type != "group" && c.title.chars().any(char::is_alphabetic);
    if first_word {
        c.title.split_whitespace().next().expect("a letter")
    } else {
        &c.title
    }
}

fn tinted<'a>(h: &svg::Handle, size: f32, color: Color) -> El<'a> {
    svg(h.clone()).width(size).height(size).opacity(color.a).style(move |_, _| svg::Style { color: Some(color) }).into()
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

/// Desktop's label of a duration option such as "10 seconds".
fn duration(n: usize, unit: &str) -> String {
    match unit {
        "second" | "seconds" => trn("%d second", "%d seconds", n, &[&n]),
        "seconds (disabled)" => trn("%d second (disabled)", "%d seconds (disabled)", n, &[&n]),
        "ms" => trf("%d ms", &[&n]),
        "ms (disabled)" => trf("%d ms (disabled)", &[&n]),
        "minute" | "minutes" => trn("%d minute", "%d minutes", n, &[&n]),
        "day" | "days" => trn("%d day", "%d days", n, &[&n]),
        "month" | "months" => trn("%d month", "%d months", n, &[&n]),
        "year" | "years" => trn("%d year", "%d years", n, &[&n]),
        _ => panic!("no unit {unit}"),
    }
}

/// Saves the composer text of the chat open in `pane` as its draft, unless the composer edits a message.
fn save_draft(st: &mut BeeperStore, pane: &Pane) {
    let Some(chat) = pane.selected_chat_id.as_deref().filter(|_| pane.editing_message_id.is_none()) else { return };
    let text = pane.composer.text().trim().to_string();
    let draft = (!text.is_empty()).then(|| ChatDraft {
        text: Some(text),
        reply_to_message_id: pane.reply_to_message_id.clone(),
        attachments: Vec::new(),
    });
    st.patch_chat(chat, true, None, None).expect("clear draft");
    st.patch_chat(chat, false, draft, None).expect("save draft");
}

fn pref_row<'a>(
    p: &Palette,
    title: impl ToString,
    desc: Option<&str>,
    extra: Option<El<'a>>,
    control: El<'a>,
) -> El<'a> {
    let title = title.to_string();
    let label: El<'a> = match extra {
        None => txt(title, 13.0).color(p.text).into(),
        Some(extra) => {
            let space = text_width(" ", 13.0, Weight::Normal);
            let words = title.split(' ').map(|w| txt(w.to_string(), 13.0).color(p.text).into());
            let extra = container(extra).padding(Padding::ZERO.left(6.0 - space));
            Row::with_children(words).push(extra).spacing(space).align_y(Center).wrap().into()
        }
    };
    let head = row![container(label).width(Fill), control];
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

/// ProseMirror's history of the composer: the texts before the edit groups, and the undone texts.
#[derive(Default)]
pub struct History {
    undo: Vec<String>,
    redo: Vec<String>,
    /// The time and new range of the last edit; an edit later than 100 ms or not next to it starts a new group.
    last: Option<(std::time::Instant, std::ops::Range<usize>)>,
}

/// Desktop's composer pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picker {
    Gif,
    Sticker,
    Audio,
}

/// The open chat of a window, with its composer and its menus.
#[derive(Default)]
pub struct Pane {
    pub selected_chat_id: Option<String>,
    /// Desktop's composer `attachments`.
    pub attachments: Vec<PendingMedia>,
    pub composer_links: Vec<(String, LinkState)>,
    pub message_menu: Option<(String, Point)>,
    /// The composer context menu, with the misspelled word and its suggestions.
    pub edit_menu: Option<(Point, Option<Typo>)>,
    /// The selected messages, and the message that a Shift click selects from.
    pub selected_msgs: Vec<String>,
    pub msg_anchor: Option<String>,
    pub show_attach_menu: bool,
    /// Desktop's `visiblePicker`.
    pub picker: Option<Picker>,
    /// The URL and text of the formatting menu's link dialog.
    pub link_dialog: Option<(String, String)>,
    pub emoji: Option<emoji::Picker>,
    pub composer: text_editor::Content,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub expanded_history_msg_id: Option<String>,
    pub hovered_msg: Option<String>,
    /// The selected emoji autocomplete row.
    pub suggest: usize,
    pub history: History,
    /// The composer text where Escape closed the emoji autocomplete.
    pub suggest_hidden: Option<String>,
    /// After an emoticon replacement: the new text, and the typed text and caret that Backspace restores.
    pub emoticon: Option<(String, String, usize)>,
    /// The first message IDs of the expanded membership event groups.
    pub open_members: HashSet<String>,
    pub size: Size,
    pub send_later: Option<(i64, bool)>,
    /// Desktop's `ScheduledMessages` state: collapsed and collapsible.
    pub sched_fold: (bool, bool),
    pub sched_menu: Option<(Vec<String>, bool, Point)>,
    /// Desktop's `openReplyThread`: the message whose reply chain shows over the timeline.
    pub reply_thread: Option<String>,
}

impl Pane {
    /// Desktop's `getMessageSendOptions`: the quoted message, else the message of the open reply chain.
    fn reply_target(&mut self) -> Option<String> {
        self.reply_to_message_id.take().or_else(|| self.reply_thread.clone())
    }
}

pub struct BeeperGuiApp {
    pub pane: Pane,
    /// The chat windows and their panes.
    chat_windows: Vec<(window::Id, Pane)>,
    /// The chat window that `view` is drawing, so `pane` gives its pane.
    drawing: std::cell::Cell<Option<window::Id>>,
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
    zoom_drag: Option<f32>,
    pub pal: Palette,
    pub active_tab: InboxTab,
    pub selected_account_filter: Option<String>,
    pub hotkey_query: String,
    /// The search text of the open Keyboard Shortcuts list, and whether the list scrolled down.
    pub hotkeys_modal: Option<String>,
    /// Desktop's `FileDropOverlay` shows, and Shift was down at a drop, so the dropped files are sent.
    dropping: bool,
    drop_send: bool,
    hotkeys_scrolled: bool,
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
    cursor: std::cell::Cell<Point>,
    forward: Option<Forward>,
    carousel: Option<Carousel>,
    media_window: Option<MediaWindow>,
    /// On X11 the copied image stays available only while its clipboard lives.
    image_clipboard: Option<arboard::Clipboard>,
    pub filters: u8,
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
    pub cmd_submenu: Option<commands::SubMenu>,
    /// The Remind Me submenu's "If no reply" choice.
    pub cmd_no_reply: bool,
    /// The highlighted command.
    pub cmd_index: usize,
    pub search_expanded: bool,
    pub creating_chat: Option<u32>,
    pub new_chats: Vec<u32>,
    pub new_chat_query: String,
    pub new_chat_expanded: bool,
    pub join_matrix_mode: bool,
    pub nickname_edit_buffer: String,
    pub recovery_code_input: String,
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
    pub confirm: Option<u8>,
    delete_target: Option<String>,
    mods: keyboard::Modifiers,
    reminder_menu: Option<Point>,
    audio: voice::Player,
    recorder: Option<record::Recorder>,
    queued: Vec<Queued>,
    /// The system microphone and the other inputs, listed when General settings open.
    mics: Option<Result<(String, Vec<String>), String>>,
    pub show_command_palette: bool,
    pub active_settings_tab: SettingsTab,
    pub settings_query: String,
    /// The focused row of the settings search results, and the last opened one.
    settings_focus: usize,
    settings_pick: Option<SettingsHit>,
    /// The pref that a search result opened, which flashes for 2 s.
    pref_flash: Option<(&'static str, std::time::Instant)>,
    pub adding_account: bool,
    /// "Get the app" in Settings > Devices shows the QR code.
    app_qr: bool,
    storage: StoragePage,
    integrations: IntegrationsPage,
    token_ask: Option<TokenAsk>,
    /// The account of Desktop's account page in Settings > Accounts.
    pub account_page: Option<String>,
    pub show_right_panel: bool,
    pub quick_look_target: Option<(String, String)>,
    quick_open: bool,
    pub hovered_chat: Option<String>,
    pub flash: Option<(String, std::time::Instant)>,
    last_click: Option<(String, std::time::Instant)>,
    /// The opened chats, the most recent first. While `cycling`, a selection does not reorder them.
    chat_history: Vec<String>,
    cycling: bool,
    pub status_banner: String,
    /// The text of Desktop's toast with a "Relaunch app" button, after a change that needs a relaunch.
    relaunch_toast: Option<&'static str>,
    /// A button of the toast, shown only while the banner still is the text it was made for.
    banner_action: Option<(String, &'static str, UiMessage)>,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    /// Desktop's local `sidebar-hidden`: hides the rail and the chat list.
    pub sidebar_hidden: bool,
    pub dragging: bool,
    pub main_window: Option<window::Id>,
    pub settings_window: Option<window::Id>,
    pub login_window: Option<window::Id>,
    /// Desktop's `ChatThemeBrowserWindow`s and their chats.
    pub theme_windows: Vec<(window::Id, String)>,
    pub poll_windows: Vec<PollDraft>,
    /// The poll votes windows with their chats and poll messages.
    pub vote_windows: Vec<(window::Id, String, String)>,
    /// The window that `BEEPER_SCREENSHOT_EXIT` or `BEEPER_SCREENSHOT_SETTINGS` captures after its next frame.
    shot: Option<window::Id>,
    pub login: Login,
    pub cloud: Arc<Cloud>,
    pub avatars: HashMap<String, Option<image::Handle>>,
    pub media: HashMap<String, Option<Result<Frames, String>>>,
    pub resolving_media: HashMap<String, bool>,
    video: Option<video::Video>,
    started: std::time::Instant,
}

impl BeeperGuiApp {
    pub fn new(store: SharedStore) -> Self {
        ensure_fonts_loaded();
        let os = OsKind::current();
        let display_server = os.detect_display_server(&std::env::vars().collect());
        let (theme, panel, first_chat, cloud, source) = {
            let st = store.read();
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
        let pending = notify::Pending::new(&store.read().chats);
        let sidebar_width = match std::fs::read_to_string(store.read().data_dir.join("sidebar-width")) {
            Ok(w) => w.parse().expect("a numeric sidebar width"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 252.0,
            Err(e) => panic!("read the sidebar width: {e}"),
        };
        let sidebar_hidden = match std::fs::read_to_string(store.read().data_dir.join("sidebar-hidden")) {
            Ok(h) => h.parse().expect("a boolean sidebar-hidden"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("read sidebar-hidden: {e}"),
        };
        let mut app = Self {
            pane: Pane { size: Size::new(1264.0, 893.0), ..Pane::default() },
            chat_windows: Vec::new(),
            drawing: Default::default(),
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
            zoom_drag: None,
            active_tab: InboxTab::AllChats,
            selected_account_filter: None,
            hotkey_query: String::new(),
            hotkeys_modal: None,
            dropping: false,
            drop_send: false,
            hotkeys_scrolled: false,
            show_filter_menu: false,
            chat_menu: None,
            chat_sub: None,
            reminded: HashMap::new(),
            search_room: None,
            search_filters: SearchFilters::default(),
            search_select: None,
            cursor: Default::default(),
            image_clipboard: None,
            forward: None,
            carousel: None,
            media_window: None,
            mods: keyboard::Modifiers::empty(),
            filters: 0,
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
            cmd_submenu: None,
            cmd_no_reply: true,
            cmd_index: 0,
            search_expanded: false,
            creating_chat: None,
            new_chats: Vec::new(),
            new_chat_query: String::new(),
            new_chat_expanded: false,
            join_matrix_mode: false,
            nickname_edit_buffer: String::new(),
            recovery_code_input: String::new(),
            devices: None,
            mics: None,
            confirm: None,
            delete_target: None,
            reminder_menu: None,
            audio: Default::default(),
            recorder: None,
            queued: Vec::new(),
            selection: Vec::new(),
            selection_menu: None,
            show_command_palette: false,
            active_settings_tab: SettingsTab::General,
            settings_query: String::new(),
            settings_focus: 0,
            settings_pick: None,
            pref_flash: None,
            adding_account: false,
            app_qr: false,
            storage: StoragePage { ages: [1; 2], kinds: [0b1111, 0b11111], ..Default::default() },
            integrations: IntegrationsPage::default(),
            token_ask: None,
            account_page: None,
            show_right_panel: panel,
            quick_look_target: None,
            quick_open: false,
            hovered_chat: None,
            flash: None,
            status_banner: String::new(),
            relaunch_toast: None,
            banner_action: None,
            dnd_muted: false,
            sidebar_width,
            sidebar_hidden,
            dragging: false,
            main_window: None,
            settings_window: None,
            login_window: None,
            theme_windows: Vec::new(),
            poll_windows: Vec::new(),
            vote_windows: Vec::new(),
            shot: None,
            login: Login::new(source),
            cloud,
            avatars: HashMap::new(),
            media: HashMap::new(),
            resolving_media: HashMap::new(),
            video: None,
            started: std::time::Instant::now(),
            last_click: None,
            chat_history: Vec::new(),
            cycling: false,
        };
        if let Some(id) = first_chat {
            app.select_chat(&id);
        }
        app
    }

    pub fn boot(store: SharedStore) -> (Self, Task<UiMessage>) {
        let mut app = Self::new(store);
        let signed_in = app.pane.selected_chat_id.is_some();
        let mut settings =
            if signed_in { Self::window_settings(Size::new(1264.0, 893.0)) } else { Self::login_window_settings() };
        let st = app.store.read();
        app.hidden = signed_in && st.settings.on("startMinimized") && st.settings.on("showTrayIcon");
        drop(st);
        settings.visible = !app.hidden;
        let (id, open) = window::open(settings);
        *(if signed_in { &mut app.main_window } else { &mut app.login_window }) = Some(id);
        let cloud = (signed_in && !app.store.read().config.demo).then(|| app.refresh_cloud_accounts());
        let restore = (signed_in && !app.hidden).then(|| app.restore_subwindows());
        let tasks =
            [open.map(UiMessage::WindowOpened), iced::system::theme().map(UiMessage::SystemTheme), app.load_avatars()];
        (app, Task::batch(tasks.into_iter().chain(cloud).chain(restore)))
    }

    fn load_avatars(&mut self) -> Task<UiMessage> {
        let st = self.store.read();
        let Some(client) = st.matrix.clone() else { return Task::none() };
        let selected =
            self.pane.selected_chat_id.as_ref().and_then(|id| st.chat_idx_by_id.get(id)).map(|&i| &st.chats[i]);
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
            .chain(senders)
            .chain(st.me().img_url.as_deref());
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

    fn face<'a>(&self, url: Option<&str>, name: &str, size: f32, ls: f32) -> El<'a> {
        match url.and_then(|u| self.avatars.get(u)?.clone()) {
            Some(h) => image(h).width(size).height(size).into(),
            None => avatar(name, size, ls, &self.pal),
        }
    }

    fn typing_bubble<'a>(&self, chat: &Chat, ids: &[String]) -> El<'a> {
        let p = &self.pal;
        let dot = |delay| anim::dot(6.0, p.text, self.started, delay, |t| 0.2 + 0.2 * (1.0 - (2.0 * t - 1.0).abs()));
        let bubble =
            container(row![dot(0), dot(340), dot(670)].spacing(4)).padding([12, 11]).style(fill_box(p.received, 16.0));
        let faces = (chat.chat_type == "group").then(|| {
            let users = ids.iter().filter_map(|id| chat.participants.items.iter().find(|u| u.id == *id));
            let layers = users.enumerate().map(|(i, u)| {
                pin(self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 20.0, 0.0))
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
            anim::dot(2.0, p.text, self.started, delay, |t| match t {
                ..0.15 => 0.2 + 0.2 * t / 0.15,
                ..0.3 => 0.4 - 0.2 * (t - 0.15) / 0.15,
                _ => 0.2,
            })
        };
        container(row![dot(0), dot(100), dot(200)].spacing(2))
            .center_y(12)
            .padding([0, 3])
            .style(fill_box(p.card, 6.0))
            .into()
    }

    fn chat_avatar<'a>(&self, chat: &Chat, size: f32, ls: f32) -> El<'a> {
        fn user(u: &User) -> (Option<&str>, &str) {
            (u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default())
        }
        let (shown, more) = faces(chat);
        let face = if chat.img_url.is_some() {
            self.face(chat.img_url.as_deref(), &chat.title, size, ls)
        } else if chat.chat_type != "group" || shown.is_empty() {
            let (url, name) = shown.first().map_or((None, ""), |u| user(u));
            self.face(url, name, size, ls)
        } else {
            const SLOTS: [(f32, f32, f32); 5] = [
                (-0.14, -0.135, 0.48),
                (0.26, 0.08, 0.35),
                (-0.06, 0.28, 0.27),
                (0.25, -0.24, 0.215),
                (-0.33, 0.2, 0.2),
            ];
            let layers = shown.into_iter().zip(SLOTS).map(|(u, (x, y, k))| {
                let (url, name) = user(u);
                let d = size * k;
                pin(self.face(url, name, d, 0.0)).x(size * (0.5 + x) - d / 2.0).y(size * (0.5 + y) - d / 2.0).into()
            });
            let label = if more > 98 { "99+".to_string() } else { format!("+{more}") };
            let font = size * 0.15 * if more > 9 { 0.85 } else { 1.0 };
            let count = (more > 0).then(|| {
                let t = container(med(label, font).color(self.pal.sub)).center_x(size * 0.3);
                pin(t).x(size * 0.027).y(size * 0.484)
            });
            Stack::with_children(layers).push(count).width(size).height(size).into()
        };
        container(face).style(fill_box(Color { a: 0.1, ..self.pal.strong }, size / 2.0)).into()
    }

    /// A window of a fixed `size` over the middle of the main window, like Desktop's `center: "parent"`.
    fn open_child(&self, size: Size) -> (window::Id, Task<UiMessage>) {
        let (id, open) =
            window::open(window::Settings { resizable: false, min_size: None, ..Self::window_settings(size) });
        let main = self.main_window.expect("the main window is open");
        let offset =
            Vector::new((self.pane.size.width - size.width) / 2.0, (self.pane.size.height - size.height) / 2.0);
        let center = move |_| {
            window::position(main).then(move |at| match at {
                Some(at) => window::move_to(id, at + offset),
                None => Task::none(),
            })
        };
        (id, open.then(center))
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
        let load = self.update(UiMessage::SelectSettingsTab(tab));
        if let Some(id) = self.settings_window {
            return Task::batch([load, window::gain_focus(id)]);
        }
        let (id, open) = window::open(Self::window_settings(Size::new(793.0, 705.0)));
        self.settings_window = Some(id);
        self.save_subwindows();
        Task::batch([load, open.map(UiMessage::WindowOpened)])
    }

    /// Desktop's `subwindow-state`: the chat and settings windows to open again at the next launch.
    fn save_subwindows(&self) {
        let threads = self.chat_windows.iter().filter_map(|w| w.1.selected_chat_id.as_deref());
        let threads =
            threads.map(|t| serde_json::json!({"windowId": format!("thread-{t}"), "type": "thread", "threadId": t}));
        let prefs = self.settings_window.map(|_| serde_json::json!({"windowId": "prefs", "type": "prefs"}));
        let state = serde_json::json!({"openSubwindows": threads.chain(prefs).collect::<Vec<_>>()});
        let path = self.store.read().data_dir.join("subwindow-state.json");
        std::fs::write(path, state.to_string()).expect("write the subwindow state");
    }

    /// Opens the windows of the last session, like Desktop's `setupSubwindowRestoreHandler`.
    fn restore_subwindows(&mut self) -> Task<UiMessage> {
        let path = self.store.read().data_dir.join("subwindow-state.json");
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Task::none(),
            Err(e) => panic!("read the subwindow state: {e}"),
        };
        let state: serde_json::Value = serde_json::from_str(&json).expect("a valid subwindow state");
        let open = state["openSubwindows"].as_array().expect("a list of subwindows").clone();
        let tasks = open.into_iter().filter_map(|w| match w["type"].as_str().expect("a subwindow type") {
            "thread" => {
                let id = w["threadId"].as_str().expect("a thread window has a chat").to_owned();
                let known = self.store.read().resolve_chat_idx(&id).is_some();
                known.then(|| self.update(UiMessage::OpenChatWindow(id)))
            }
            _ => Some(self.open_settings(self.active_settings_tab)),
        });
        Task::batch(tasks.collect::<Vec<_>>())
    }

    /// Desktop's keys of the settings window: Ctrl F focuses the search, ↑ and ↓ move through the results, and Enter is
    /// the submit of the input. Escape in an input clears the search, and Escape elsewhere closes the window.
    fn settings_key(&mut self, key: Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
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

    fn scan_storage(&self) -> Task<UiMessage> {
        let store = self.store.clone();
        let scan =
            async { tokio::task::spawn_blocking(move || storage::scan(&store)).await.expect("storage scan thread") };
        Task::perform(scan, UiMessage::StorageScanned)
    }

    /// The entries that [`UiMessage::ClearStorage`] deletes, without the cache of the clear for all chats.
    fn storage_pick(&self, which: u8) -> Vec<&storage::Entry> {
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

    fn load_devices(&mut self) -> Task<UiMessage> {
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
                let me = DeviceInfo { name, model: None, kind: None, last_seen_ms: None, verified, current: true };
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
            _ if Some(id) == self.settings_window => tr("Settings"),
            _ if self.theme_windows.iter().any(|w| w.0 == id) => tr("Chat Theme"),
            _ if self.poll_windows.iter().any(|w| w.id == id) => tr("New Poll"),
            _ if let Some((_, _, mid)) = self.vote_windows.iter().find(|w| w.0 == id) => {
                let st = self.store.read();
                let total =
                    st.msg_idx_by_id.get(mid).and_then(|&i| st.messages[i].poll.as_ref()).map_or(0, PollData::voters);
                return trn("%d vote", "%d votes", total, &[&total]);
            }
            _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => tr("Media"),
            _ => match self.chat_windows.iter().find(|w| w.0 == id) {
                Some((_, pane)) => {
                    let st = self.store.read();
                    let chat = pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c));
                    return chat.map_or("Beeper".into(), |i| st.chats[i].title.clone());
                }
                None => "Beeper",
            },
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
        let st = self.store.read();
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
                        InboxTab::SendLater => st.scheduled.iter().any(|s| s.chat_id == c.id),
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
            (InboxTab::SendLater, _) => list.sort_by_key(|c| next_send(st, &c.id)),
            _ => list.sort_by(|a, b| b.last_activity.cmp(&a.last_activity)),
        }
        list
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        self.creating_chat = None;
        self.join_matrix_mode = false;
        if !self.cycling {
            self.push_history(chat_id);
        }
        let mut st = self.store.write();
        self.leave_chat(&mut st, Some(chat_id));
        let incognito = st.incognito();
        let idx = st.resolve_chat_idx(chat_id).expect("selected chat exists");
        self.pane.selected_chat_id = Some(chat_id.to_string());
        if let Some(ev) = (!incognito).then(|| st.mark_chat_read(chat_id, false)).flatten() {
            let cid = st.chats[idx].id.clone();
            st.queue(&cid, Outgoing::Read(ev));
        }
        let chat = st.chats[idx].clone();
        self.pane.composer = composer_with(chat.draft.as_ref().and_then(|d| d.text.as_deref()).unwrap_or_default());
        self.pane.history = History::default();
        self.nickname_edit_buffer = chat
            .participants
            .items
            .iter()
            .find(|u| u.is_self != Some(true))
            .and_then(|u| u.custom_nickname.clone().or_else(|| u.full_name.clone()))
            .unwrap_or_default();
        self.pane.reply_to_message_id = None;
        self.pane.editing_message_id = None;
        self.pane.selected_msgs.clear();
        self.pane.msg_anchor = None;
        let pending = st.scheduled.iter().any(|s| s.chat_id == chat.id && !s.canceled);
        (self.pane.send_later, self.pane.sched_fold, self.pane.sched_menu) = (None, (pending, pending), None);
        self.pane.reply_thread = None;
        self.video = None;
        drop(st);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `setPicker` checks: shows a toast when the network of the open chat does not accept the picker.
    fn supports(&mut self, kind: Picker) -> bool {
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane().selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let (ok, toast) = match kind {
            Picker::Gif => (chat.files & FILE_GIF != 0, "GIFs aren't supported on %s"),
            Picker::Audio => (chat.files & FILE_VOICE != 0, "Recording audio isn't supported on %s"),
            // The bridges with Desktop's `supportsStickers`.
            Picker::Sticker => (
                matches!(account_network(&chat.account_id).kind, NetworkKind::WhatsApp | NetworkKind::Telegram),
                "Stickers aren't supported on %s",
            ),
        };
        let net = network_name(&chat.account_id);
        drop(st);
        if !ok {
            self.status_banner = trf(toast, &[&net]);
        }
        ok
    }

    /// The selected message when only one message is selected.
    fn single_msg(&self) -> Option<&String> {
        Some(&self.pane().selected_msgs).filter(|s| s.len() == 1).map(|s| &s[0])
    }

    fn selected_message(&self) -> Option<Message> {
        let st = self.store.read();
        self.single_msg().map(|id| st.messages[st.msg_idx_by_id[id]].clone())
    }

    /// The messages of the open chat in timeline order, and which of them Desktop lets you select.
    fn timeline<'a>(&self, st: &'a BeeperStore) -> (Vec<&'a Message>, Vec<bool>) {
        let all = &st.chat_msg_indices[self.pane().selected_chat_id.as_deref().expect("an open chat")];
        let msgs: Vec<&Message> = all.iter().map(|&i| &st.messages[i]).collect();
        let members = st.settings.on("membershipEvents");
        let live = msgs.iter().map(|m| m.is_deleted != Some(true) && (members || !membership(m))).collect();
        (msgs, live)
    }

    /// The arrow keys select messages when no panel is open and the composer is empty.
    fn quick_react(&self, mid: String) -> UiMessage {
        let chat = self.pane().selected_chat_id.clone().expect("an open chat");
        let st = self.store.read();
        UiMessage::ToggleReaction(chat, mid, st.settings.choice("quickReactionEmoji").into())
    }

    /// Desktop's emoji autocomplete for the ":" word at the end of the composer.
    fn emoji_suggestions(&self, st: &BeeperStore) -> Vec<(String, &'static str)> {
        let text = &self.pane().composer.text();
        let on = st.settings.on("emojiAutocomplete") && self.pane().suggest_hidden.as_ref() != Some(text);
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
        let at = self.pane().suggest.min(picks.len().checked_sub(1)?);
        let rows: Vec<String> = picks.iter().map(|(c, name)| format!("{c} {name}")).collect();
        let width = rows.iter().map(|r| text_width(r, 13.0, Weight::Normal)).fold(0.0, f32::max) + 12.0;
        let items = rows.into_iter().enumerate().map(|(i, r)| {
            let (bg, fg) = if i == at { (p.accent, Color::WHITE) } else { (CLEAR, p.text) };
            let b = button(txt(r, 13.0).wrapping(iced::widget::text::Wrapping::None)).padding([4, 6]).width(width);
            b.style(btn_style([bg; 3], [fg; 3], 4.0)).on_press(UiMessage::PickEmoji(i)).into()
        });
        let list =
            container(Column::with_children(items).padding([4, 4])).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let unveiled = hidden_actions(st).len() as f32;
        let plus = if self.pane().show_attach_menu { 36.0 + 6.0 + 2.0 + unveiled * 30.0 - 2.0 } else { 36.0 };
        let text = self.pane().composer.text();
        let line = &text[text.rfind('\n').map_or(0, |i| i + 1)..text.rfind(':').expect("an emoji query")];
        let x = 8.0 + plus + 8.0 + 14.0 + 2.0 + text_width(line, 13.0, Weight::Normal);
        let x = x.min(self.pane().size.width / self.zoom - self.left_width(st) - width - 36.0);
        Some(container(list).align_bottom(Fill).padding(Padding::ZERO.left(x).bottom(34.0)).into())
    }

    /// The width of the rail and the chat list.
    fn left_width(&self, st: &BeeperStore) -> f32 {
        let rail = if st.settings.on("showSidebar") { 68.0 } else { 0.0 };
        if self.sidebar_hidden {
            0.0
        } else {
            rail + self.sidebar_width
        }
    }

    /// The newest message you can edit in the open chat.
    fn last_own_message(&self) -> Option<Message> {
        let st = self.store.read();
        let all = &st.chat_msg_indices[self.pane().selected_chat_id.as_deref().expect("an open chat")];
        let own = all
            .iter()
            .map(|&i| &st.messages[i])
            .rfind(|m| editable(m) && m.is_deleted != Some(true) && m.attachments.is_empty());
        own.cloned()
    }

    fn can_select_messages(&self) -> bool {
        self.pane().selected_chat_id.is_some()
            && self.creating_chat.is_none()
            && !self.show_command_palette
            && self.forward.is_none()
            && self.pane().emoji.is_none()
            && (!self.pane().selected_msgs.is_empty() || self.pane().composer.is_empty())
    }

    /// Leave the selected chat for `next`: save its draft, mark it read, and drop its elapsed reminder.
    fn leave_chat(&self, st: &mut BeeperStore, next: Option<&str>) {
        let Some(prev) = self.pane().selected_chat_id.as_deref().filter(|&p| Some(p) != next) else { return };
        save_draft(st, self.pane());
        let idx = st.resolve_chat_idx(prev).expect("selected chat exists");
        if !st.incognito() && st.chats[idx].is_marked_unread != Some(true) {
            st.mark_chat_read(prev, true);
        }
        if self.expired(&st.chats[idx]) {
            st.set_reminder(idx, None);
        }
    }

    /// Quits the app; the open chats keep their drafts.
    fn quit(&self) -> Task<UiMessage> {
        let mut st = self.store.write();
        for pane in std::iter::once(&self.pane).chain(self.chat_windows.iter().map(|w| &w.1)) {
            save_draft(&mut st, pane);
        }
        drop(st);
        self.save_subwindows();
        iced::exit()
    }

    /// Adds an edit from `old` to the history, and reacts to the new text.
    fn composer_changed(&mut self, old: String) -> Task<UiMessage> {
        let h = &mut self.pane.history;
        let new = self.pane.composer.text();
        if old != new {
            let (head, old_end, new_end) = format::diff(&old, &new);
            let near = |(t, r): &(std::time::Instant, std::ops::Range<usize>)| {
                t.elapsed().as_millis() < 100 && head <= r.end && old_end >= r.start
            };
            if !h.last.as_ref().is_some_and(near) {
                h.undo.push(old);
                if h.undo.len() > 100 {
                    h.undo.remove(0);
                }
            }
            h.redo.clear();
            h.last = Some((std::time::Instant::now(), head..new_end));
        }
        self.composer_effects()
    }

    /// Desktop's reaction to new composer text: the typing notice and link previews.
    /// Whether the open chat has sends that wait for "Allow undo send".
    fn queued_here(&self) -> bool {
        self.queued.iter().any(|q| Some(&q.chat) == self.pane().selected_chat_id.as_ref())
    }

    fn composer_effects(&mut self) -> Task<UiMessage> {
        self.pane.selected_msgs.clear();
        self.pane.link_dialog = None;
        self.pane.suggest = 0;
        let st = self.store.read();
        let new = self.pane.composer.text();
        let notice = st.matrix.clone().filter(|_| st.typing_notices());
        let task = notice.zip(self.pane.selected_chat_id.clone()).map(|(client, chat)| {
            Task::perform(beeper_core::matrix::typing(client, chat, !new.is_empty()), UiMessage::Pushed)
        });
        drop(st);
        Task::batch([task.unwrap_or_else(Task::none), self.fetch_link_previews()])
    }

    /// Desktop's `sendMessage`: a new message moves an archived chat to the inbox, unless `archive` is set.
    pub fn submit_composer(&mut self, archive: bool) {
        let chat_id = self.pane.selected_chat_id.clone().expect("a chat is selected");
        let text = self.pane.composer.text().trim().to_string();
        if text.is_empty() && self.pane.attachments.is_empty() {
            return;
        }
        let mut st = self.store.write();
        let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
        let later = self.pane.send_later.take();
        if self.pane.editing_message_id.is_none()
            && later.is_none()
            && has_flag(&st.chats[idx], ChatFlag::Archived) != archive
        {
            self.set_archived(&mut st, idx, archive);
        }
        if let Some(id) = self.pane.editing_message_id.take_if(|id| st.scheduled.iter().any(|s| s.id == *id)) {
            st.update_scheduled(&id, |s| {
                s.text = text;
                true
            });
        } else if let Some((at, no_reply)) = later {
            let cid = st.chats[idx].id.clone();
            if !st.scheduled.iter().any(|s| s.chat_id == cid && !s.canceled) {
                self.pane.sched_fold = (false, false);
            }
            st.schedule(&chat_id, text, self.pane.reply_target(), at, no_reply);
        } else if let Some(target) = self.pane.editing_message_id.take() {
            st.edit_message(&chat_id, &target, text.clone()).expect("edit message");
            st.queue(&chat_id, Outgoing::Edit { target, text });
        } else {
            let mut reply_to = self.pane.reply_target();
            let files = std::mem::take(&mut self.pane.attachments);
            let delay =
                st.feature("undo-send").on.then(|| prefs::UNDO_SEND_MS[usize::from(st.settings.pref("undoSendDelay"))]);
            let delay = delay.filter(|&ms| ms > 0);
            let restore = delay.map(|_| (text.clone(), reply_to.clone(), files.clone()));
            let mut sends = Vec::new();
            let caption = (!text.is_empty() && files.len() == 1).then(|| text.clone());
            if !text.is_empty() && caption.is_none() {
                let urls = text_urls(&text);
                let preview = self.pane.composer_links.iter().find_map(|(u, s)| match s {
                    LinkState::Loaded(v, img) if urls.contains(&u.as_str()) => {
                        Some((v.clone(), img.as_ref().map(|(b, _)| b.clone())))
                    }
                    _ => None,
                });
                let (_, txn, _) =
                    st.send_message(&chat_id, Some(text.clone()), reply_to.clone(), None).expect("send message");
                if let Some(link) = preview.as_ref().and_then(|(v, _)| beeper_core::matrix::link(v)) {
                    let i = st.msg_idx_by_id[&txn];
                    st.update_message(i, |m| m.links = vec![link]);
                }
                sends.push((txn.clone(), Outgoing::Send { txn, text, reply_to: reply_to.take(), preview }));
            }
            for PendingMedia { bytes, name, mime, is_gif, size, frames } in files {
                let image = !frames.is_empty();
                let key = format!("pasted:{}", st.next_id("img"));
                if image {
                    self.media.insert(key.clone(), Some(Ok(frames)));
                }
                let att = Attachment {
                    attachment_type: if image { "img" } else { "file" }.into(),
                    file_name: Some(name.clone()),
                    file_size: Some(bytes.len() as u64),
                    is_gif: Some(is_gif),
                    mime_type: Some(mime.clone()),
                    size: image.then_some(AttachmentSize { width: Some(size.0), height: Some(size.1) }),
                    src_url: image.then_some(key),
                    ..Default::default()
                };
                let (_, txn, _) =
                    st.send_message(&chat_id, caption.clone(), reply_to.clone(), Some(att)).expect("send media");
                let info = if image { MediaInfo::Image(size.0, size.1) } else { MediaInfo::File };
                let caption = caption.clone();
                let out =
                    Outgoing::Media { txn: txn.clone(), bytes, name, mime, info, caption, reply_to: reply_to.take() };
                sends.push((txn, out));
            }
            // Desktop's `queuePendingMessages` sends the earlier queue of the chat first.
            if let Some(i) = self.queued.iter().position(|q| q.chat == chat_id) {
                for (_, out) in self.queued.remove(i).sends {
                    st.queue(&chat_id, out);
                }
            }
            match (delay, restore) {
                (Some(ms), Some((text, reply_to, files))) => {
                    let due = now_unix_millis() + ms;
                    self.queued.push(Queued { chat: chat_id.clone(), due, sends, text, reply_to, files });
                }
                _ => sends.into_iter().for_each(|(_, out)| st.queue(&chat_id, out)),
            }
            st.mark_chat_read(&chat_id, true);
        }
        self.pane.composer = composer_with("");
        self.pane.composer_links.clear();
        drop(st);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `ScheduledStore.sendMessage`: a typed message, which moves an archived chat to the inbox.
    fn send_scheduled(&mut self, due: Vec<Scheduled>) {
        let mut st = self.store.write();
        for s in due {
            let Some(idx) = st.resolve_chat_idx(&s.chat_id) else { continue };
            if has_flag(&st.chats[idx], ChatFlag::Archived) {
                self.set_archived(&mut st, idx, false);
            }
            let (_, txn, _) =
                st.send_message(&s.chat_id, Some(s.text.clone()), s.reply_to.clone(), None).expect("send scheduled");
            st.queue(&s.chat_id, Outgoing::Send { txn, text: s.text, reply_to: s.reply_to, preview: None });
        }
        drop(st);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `fetchLinkPreviews`: forget the links that left the composer, and load the new ones unless "Disable
    /// link previews" is on or the network cannot send previews.
    fn fetch_link_previews(&mut self) -> Task<UiMessage> {
        use beeper_core::models::NetworkKind::*;
        let urls: Vec<String> = text_urls(&self.pane.composer.text()).into_iter().map(str::to_string).collect();
        self.pane.composer_links.retain(|(u, _)| urls.contains(u));
        let st = self.store.read();
        let chat = self.pane.selected_chat_id.as_ref().and_then(|c| st.resolve_chat_idx(c)).map(|i| &st.chats[i]);
        let kind = chat.map(|c| account_network(&c.account_id).kind);
        let can =
            matches!(kind, Some(Beeper | WhatsApp | TwitterX | Telegram | Slack | GoogleMessages | Signal | Line));
        if !can || st.settings.on("noOutgoingLinkPreviews") {
            return Task::none();
        }
        drop(st);
        let mut tasks = Vec::new();
        for url in urls {
            if self.pane.composer_links.iter().all(|(u, _)| *u != url) {
                self.pane.composer_links.push((url.clone(), LinkState::Loading));
                let load = beeper_core::cloud::link_preview(url.clone());
                tasks.push(Task::perform(load, move |r| UiMessage::LinkPreviewLoaded(url, r)));
            }
        }
        Task::batch(tasks)
    }

    fn push_outbox(&mut self) -> Task<UiMessage> {
        match self.store.write().start_flush() {
            true => Task::perform(beeper_core::matrix::flush(self.store.clone()), UiMessage::Pushed),
            false => Task::none(),
        }
    }

    /// The hotkey with the modifiers and the key, or with the modifiers alone before the digit 1 to 9.
    fn find_hotkey(&self, key: &Key, mods: keyboard::Modifiers) -> Option<(&'static str, usize)> {
        let name = hotkey_key(key)?;
        let digit = name.parse::<usize>().ok().filter(|d| (1..=9).contains(d));
        let mods = [mods.control() || mods.command(), mods.shift(), mods.alt()];
        let st = self.store.read();
        // Show Replies shares Ctrl Down with the scroll, and takes it when messages have the focus.
        let selecting = !self.pane().selected_msgs.is_empty();
        let hit = |mods, key: &str| {
            let hits = prefs::HOTKEYS.iter().filter(|h| prefs::hotkey_has(st.settings.hotkey_keys(h.0), mods, key));
            hits.min_by_key(|h| selecting && h.0 == "Scroll to the bottom of a chat").map(|h| h.0)
        };
        let unshifted = || hit([mods[0], false, mods[2]], &name).filter(|h| *h == CYCLE && mods[1]);
        let hit_digit = || Some((hit(mods, "")?, digit? - 1));
        hit(mods, &name).or_else(unshifted).map(|h| (h, 0)).or_else(hit_digit)
    }

    fn set_sidebar_hidden(&mut self, hidden: bool) {
        if self.sidebar_hidden != hidden {
            self.sidebar_hidden = hidden;
            let path = self.store.read().data_dir.join("sidebar-hidden");
            std::fs::write(path, hidden.to_string()).expect("write sidebar-hidden");
        }
    }

    fn push_history(&mut self, chat_id: &str) {
        self.chat_history.retain(|c| c != chat_id);
        self.chat_history.insert(0, chat_id.to_string());
        self.chat_history.truncate(50);
    }

    /// Selects the chat `by` steps older in the history, or the most recent chat when no chat is open.
    fn step_history(&mut self, by: isize) -> Task<UiMessage> {
        let st = self.store.read();
        let list: Vec<&String> = self.chat_history.iter().filter(|c| st.resolve_chat_idx(c).is_some()).collect();
        let at = self.pane.selected_chat_id.as_ref().map(|s| list.iter().position(|c| *c == s));
        let to = match at {
            Some(at) => (at.map_or(-1, |a| a as isize) + by).rem_euclid(list.len().max(1) as isize) as usize,
            None => 0,
        };
        let Some(id) = list.get(to).map(|c| c.to_string()) else { return Task::none() };
        drop(st);
        let cycling = std::mem::replace(&mut self.cycling, true);
        let task = self.update(UiMessage::SelectChat(id));
        self.cycling = cycling;
        task
    }

    fn run_hotkey(&mut self, name: &str, digit: usize) -> Task<UiMessage> {
        let chat = self.pane.selected_chat_id.clone();
        let flag = |f: ChatFlag| chat.clone().map(|c| UiMessage::ToggleFlag(c, f));
        let selected = self.single_msg().cloned().filter(|_| chat.is_some());
        let msg = match name {
            "Chat: Archive All Read" => Some(UiMessage::Ask(Some(ASK_ARCHIVE_READ))),
            "Chat: Start New" => Some(UiMessage::NewChat),
            "Chat: Toggle Archive" => flag(ChatFlag::Archived),
            "Chat: Toggle Mark As Read" => chat.map(UiMessage::ToggleRead),
            "Chat: Toggle Mute" => flag(ChatFlag::Muted),
            "Chat: Toggle Pin" => flag(ChatFlag::Pinned),
            CYCLE => {
                let task = self.step_history(if self.mods.shift() { -1 } else { 1 });
                self.cycling = true;
                return task;
            }
            "History: Next Chat" => return self.step_history(-1),
            "History: Previous Chat" => return self.step_history(1),
            "Select Next Chat" | "Select Previous Chat" => {
                self.step_chat(if name == "Select Next Chat" { 1 } else { -1 });
                None
            }
            "Scroll to the bottom of a chat" => {
                let text = self.pane.composer.text();
                let end = format::offset(&text, self.pane.composer.cursor().position) == text.len();
                return match chat.is_some() && end {
                    true => operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y: 0.0 }),
                    false => Task::none(),
                };
            }
            "Select Next Unread Chat" => {
                let st = self.store.read();
                let next = self
                    .visible_chats(&st)
                    .into_iter()
                    .find(|c| c.flags & CHAT_UNREAD != 0 && Some(&c.id) != chat.as_ref());
                next.map(|c| UiMessage::SelectChat(c.id.clone()))
            }
            "Switch First 9 Chats Modifier" => {
                let id = self.visible_chats(&self.store.read()).get(digit).map(|c| c.id.clone());
                id.map(UiMessage::SelectChat)
            }
            "Toggle Chat Info" => Some(UiMessage::ToggleRightPanel),
            "Command Bar" if self.show_command_palette => Some(UiMessage::ToggleCommandMode),
            "Command Bar" => {
                let open = self.update(UiMessage::ToggleCommandPalette);
                self.cmd_bar_commands_mode = true;
                return open;
            }
            "Filter By Account" => Some(UiMessage::OpenSubMenu(commands::SubMenu::FilterAccount)),
            "Remind Later" => chat.map(|id| UiMessage::OpenSubMenu(commands::SubMenu::Remind(id))),
            "Cycle Filters" => {
                self.filters = match self.filters {
                    f if f == 1 << 4 => 0,
                    f if f.count_ones() == 1 && f < 1 << 4 => f << 1,
                    _ => 1,
                };
                None
            }
            "Keyboard Shortcuts" => Some(UiMessage::HotkeysModal(self.hotkeys_modal.is_none().then(String::new))),
            "Filter: All" => {
                self.filters = 0;
                None
            }
            f if FILTER_HOTKEYS.contains(&f) => {
                self.filters ^= 1 << FILTER_HOTKEYS.iter().position(|h| *h == f).expect("a filter hotkey");
                None
            }
            "Chat: Send File" => chat.map(|_| UiMessage::PickFiles),
            "Open User Profile" => {
                let st = self.store.read();
                let c = chat.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]);
                c.and_then(profile_link).map(UiMessage::OpenUrl)
            }
            "Message: Send Message and Archive" => chat.map(|_| UiMessage::SendAndArchive),
            "Message: Send Later" => chat.map(|_| UiMessage::OpenSendLater),
            "Send Message" | "Insert New Line" => None,
            "Minimize To Tray" => return self.set_hidden(true),
            "Quit App" => return self.quit(),
            "Reload App" => Some(UiMessage::Relaunch),
            "Report a Problem" => Some(UiMessage::OpenUrl(ISSUES_URL.into())),
            "Reset Zoom" => Some(UiMessage::Zoom(0)),
            "Search" => Some(UiMessage::ToggleCommandPalette),
            "Search: Room" => chat.map(UiMessage::SearchRoom),
            "Select Next Account" | "Select Previous Account" => {
                let rail = rail_accounts(&self.store.read());
                let at = self.selected_account_filter.as_ref().and_then(|a| rail.iter().position(|r| r == a));
                let to = match (name == "Select Next Account", at) {
                    (true, None) => rail.first().cloned().map(Some),
                    (true, Some(i)) => rail.get(i + 1).cloned().map(Some),
                    (false, Some(i)) => Some(i.checked_sub(1).map(|i| rail[i].clone())),
                    (false, None) => None,
                };
                to.map(UiMessage::SelectAccount)
            }
            "Send Feedback" => chat.map(|_| UiMessage::OpenQuickLook("Feedback".into(), FEEDBACK.into())),
            "Settings" => Some(UiMessage::OpenSettings(self.active_settings_tab)),
            "Switch First 9 Spaces Modifier" => match RAIL_VIEWS.get(digit) {
                Some(view) => view.map(UiMessage::SelectTab),
                None => rail_accounts(&self.store.read())
                    .get(digit - RAIL_VIEWS.len())
                    .map(|a| UiMessage::SelectAccount(Some(a.clone()))),
            },
            "Toggle Incognito Mode" => {
                let st = self.store.read();
                let on = st.settings.on("incognito");
                st.feature("stealth-mode").on.then_some(UiMessage::SetPref("incognito", u8::from(!on)))
            }
            "Toggle Sidebar" => {
                self.set_sidebar_hidden(!self.sidebar_hidden);
                None
            }
            "Toggle Space Bar" => {
                let on = self.store.read().settings.on("showSidebar");
                Some(UiMessage::SetPref("showSidebar", u8::from(!on)))
            }
            "Zoom In" => Some(UiMessage::Zoom(1)),
            "Zoom Out" => Some(UiMessage::Zoom(-1)),
            "Download Attachments" => selected
                .filter(|_| self.selected_message().is_some_and(|m| !m.attachments.is_empty()))
                .map(|m| UiMessage::Download(m, None)),
            "Message: Edit" => {
                let edit = self.selected_message().filter(editable);
                let m = edit.or_else(|| self.last_own_message()).filter(|_| chat.is_some());
                m.map(|m| {
                    self.pane.selected_msgs = vec![m.id.clone()];
                    UiMessage::StartEdit(m.id, m.text.unwrap_or_default())
                })
            }
            "Message: Forward" => {
                let st = self.store.read();
                let msgs = self.timeline(&st).0.into_iter().filter(|m| self.pane.selected_msgs.contains(&m.id));
                let msgs: Vec<&Message> = msgs.filter(|_| chat.is_some()).collect();
                let mids = msgs.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
                let ok = !mids.is_empty() && msgs.iter().all(|m| m.is_deleted != Some(true));
                drop(st);
                if !ok {
                    self.status_banner = tr("Only works when messages are selected").into();
                }
                ok.then_some(UiMessage::Forward(mids))
            }
            "Message: Open First Link" => {
                let text = self.selected_message().and_then(|m| m.text);
                text.and_then(|t| {
                    t.split_whitespace().find(|w| w.starts_with("http")).map(|u| UiMessage::OpenUrl(u.into()))
                })
            }
            "Message: Quick React" => selected.map(|m| self.quick_react(m)),
            "Message: Quote & Reply" => selected.map(UiMessage::StartReply),
            "Message: Quote or Edit Selected Message" => {
                let empty = self.pane.composer.text().is_empty();
                self.selected_message().and_then(|m| match m.is_sender == Some(true) {
                    true => (empty && editable(&m)).then(|| UiMessage::StartEdit(m.id, m.text.unwrap_or_default())),
                    false => Some(UiMessage::StartReply(m.id)),
                })
            }
            "Message: Record Audio" => chat.map(|_| UiMessage::Record(record::Msg::Start)),
            "Message: Send GIF" => chat.map(|_| UiMessage::TogglePicker(Picker::Gif)),
            "Message: Send Sticker" => chat.map(|_| UiMessage::TogglePicker(Picker::Sticker)),
            "Show Replies" => chat.and_then(|_| {
                let st = self.store.read();
                let msgs = self.timeline(&st).0;
                self.pane.selected_msgs.iter().find_map(|id| {
                    let m = &st.messages[st.msg_idx_by_id[id]];
                    let replied = || msgs.iter().any(|r| r.id != m.id && reply_root(&st, r) == m.id);
                    m.linked_message_id.clone().or_else(|| replied().then(|| m.id.clone())).map(UiMessage::ShowReplies)
                })
            }),
            _ => unreachable!("{name} has no action"),
        };
        msg.map_or_else(Task::none, |m| self.update(m))
    }

    pub fn step_chat(&mut self, delta: isize) {
        let next = {
            let st = self.store.read();
            let chats = self.visible_chats(&st);
            if chats.is_empty() {
                return;
            }
            let cur = self.pane.selected_chat_id.as_ref().and_then(|id| chats.iter().position(|c| &c.id == id));
            chats[(cur.unwrap_or(0) as isize + delta).rem_euclid(chats.len() as isize) as usize].id.clone()
        };
        self.select_chat(&next);
    }

    /// The pane of the window that `view` draws; in `update`, the pane of the window that sent the message.
    fn pane(&self) -> &Pane {
        match self.drawing.get() {
            Some(id) => &self.chat_windows.iter().find(|w| w.0 == id).expect("a drawn chat window").1,
            None => &self.pane,
        }
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
        let st = self.store.read();
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
            UiMessage::SelectChat(_) if self.recorder.is_some() => {
                self.status_banner = tr("Stop recording to switch chats").into();
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
                let msg = match self.store.read().settings.pref("chatDoubleClick") {
                    1 => UiMessage::ToggleFlag(id, ChatFlag::Archived),
                    2 => UiMessage::ToggleRead(id),
                    3 => UiMessage::ToggleFlag(id, ChatFlag::Muted),
                    4 => UiMessage::ToggleFlag(id, ChatFlag::LowPriority),
                    _ => UiMessage::OpenChatWindow(id),
                };
                return self.update(msg);
            }
            UiMessage::EditMenu(word) => {
                let text = self.pane.composer.text();
                if let Some(r) = &word {
                    self.pane.composer = format::content(&text, r.clone());
                }
                let word = word.map(|r| (r.clone(), spell::suggest(&text[r])));
                self.pane.edit_menu = Some((self.cursor.get(), word));
            }
            UiMessage::MenuEdit(edit) => {
                let (_, word) = self.pane.edit_menu.take().expect("the edit menu is open");
                let old = self.pane.composer.text();
                let sel = format::selection(&self.pane.composer).map_or("", |r| &old[r]).to_owned();
                let focus = operation::focus("composer-input");
                return match edit {
                    MenuEdit::Replace(fix) => {
                        let (r, _) = word.expect("a suggestion has a word");
                        let text = format!("{}{fix}{}", &old[..r.start], &old[r.end..]);
                        self.pane.composer = format::content(&text, r.start + fix.len()..r.start + fix.len());
                        Task::batch([self.composer_changed(old), focus])
                    }
                    MenuEdit::Learn => match spell::learn(&old[word.expect("Learn Spelling has a word").0]) {
                        Ok(()) => focus,
                        Err(e) => {
                            self.status_banner = e.to_string();
                            focus
                        }
                    },
                    MenuEdit::Copy => Task::batch([iced::clipboard::write(sel), focus]),
                    MenuEdit::Cut => {
                        self.pane.composer.perform(text_editor::Action::Edit(text_editor::Edit::Delete));
                        Task::batch([iced::clipboard::write(sel), self.composer_changed(old), focus])
                    }
                    MenuEdit::Paste => Task::batch([
                        iced::clipboard::read().map(|t| match t {
                            Some(t) => {
                                UiMessage::ComposerEdit(text_editor::Action::Edit(text_editor::Edit::Paste(t.into())))
                            }
                            None => UiMessage::Noop,
                        }),
                        Task::perform(clipboard_image(), UiMessage::Pasted),
                        focus,
                    ]),
                };
            }
            UiMessage::ComposerEdit(action) if !action.is_edit() => {
                self.pane.emoticon = None;
                self.pane.composer.perform(action);
            }
            UiMessage::ComposerEdit(action) => {
                use text_editor::{Action, Edit};
                let old = self.pane.composer.text();
                match (action, self.pane.emoticon.take().filter(|(after, ..)| *after == old)) {
                    (Action::Edit(Edit::Backspace), Some((_, typed, at))) => {
                        self.pane.composer = format::content(&typed, at..at);
                    }
                    (action, _) => {
                        let typed = matches!(action, Action::Edit(Edit::Insert(_)));
                        self.pane.composer.perform(action);
                        let text = self.pane.composer.text();
                        let at = format::offset(&text, self.pane.composer.cursor().position);
                        let rules = typed && !self.store.read().settings.on("keepEmoticons");
                        if let Some((new, caret)) = rules.then(|| emoji::replace_emoticon(&text, at)).flatten() {
                            self.pane.composer = format::content(&new, caret..caret);
                            self.pane.emoticon = Some((new, text, at));
                        }
                    }
                }
                return self.composer_changed(old);
            }
            UiMessage::DeleteTo(to, back) => {
                if self.pane.composer.selection().is_none() {
                    self.pane.composer.perform(text_editor::Action::Select(to));
                }
                let edit = if back { text_editor::Edit::Backspace } else { text_editor::Edit::Delete };
                return self.update(UiMessage::ComposerEdit(text_editor::Action::Edit(edit)));
            }
            UiMessage::Undo(false) if self.queued_here() => return self.update(UiMessage::UndoSend),
            UiMessage::Undo(true) if self.queued_here() => {
                return self.update(UiMessage::SendQueued(self.pane.selected_chat_id.clone()));
            }
            UiMessage::Undo(redo) => {
                let h = &mut self.pane.history;
                let (from, to) = if redo { (&mut h.redo, &mut h.undo) } else { (&mut h.undo, &mut h.redo) };
                let Some(text) = from.pop() else { return Task::none() };
                let now = self.pane.composer.text();
                let at = format::diff(&text, &now).1;
                to.push(now);
                h.last = None;
                self.pane.emoticon = None;
                self.pane.composer = format::content(&text, at..at);
                return self.composer_effects();
            }
            UiMessage::ComposerChanged(v) => {
                let old = self.pane.composer.text();
                self.pane.composer = composer_with(&v);
                return self.composer_changed(old);
            }
            UiMessage::Format(format::CODE) if format::selection(&self.pane.composer).is_none() => {
                let insert = text_editor::Action::Edit(text_editor::Edit::Insert('`'));
                return self.update(UiMessage::ComposerEdit(insert));
            }
            UiMessage::Format(i) => {
                let old = self.pane.composer.text();
                let caret = format::offset(&old, self.pane.composer.cursor().position);
                let r = format::selection(&self.pane.composer).unwrap_or(caret..caret);
                if i == format::LINK && !format::active(i, &old, &r) {
                    self.pane.link_dialog = Some((String::new(), old[r].to_owned()));
                    return operation::focus("link-url");
                }
                let (text, r) = format::toggle(i, &old, r);
                self.pane.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            UiMessage::LinkDialog(d) => {
                let closed = d.is_none();
                self.pane.link_dialog = d;
                if closed {
                    return operation::focus("composer-input");
                }
            }
            UiMessage::SaveLink => {
                let (url, label) = self.pane.link_dialog.take().expect("the link dialog is open");
                let old = self.pane.composer.text();
                let caret = format::offset(&old, self.pane.composer.cursor().position);
                let r = format::selection(&self.pane.composer).unwrap_or(caret..caret);
                let (text, r) = format::link(&old, r, url.trim(), label.trim());
                self.pane.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            UiMessage::SubmitComposer if !self.emoji_suggestions(&self.store.read()).is_empty() => {
                return self.update(UiMessage::PickEmoji(self.pane.suggest));
            }
            UiMessage::SubmitComposer if self.recorder.is_some() => return self.send_take(),
            UiMessage::SubmitComposer => self.submit_composer(false),
            UiMessage::SendAndArchive => {
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                let st = self.store.read();
                let at = self.list_ids(&st).iter().position(|c| *c == cid);
                drop(st);
                let draft = !self.pane.composer.text().trim().is_empty() || !self.pane.attachments.is_empty();
                self.submit_composer(true);
                let list = self.list_ids(&self.store.read());
                if let Some(at) = at.filter(|_| draft && !list.contains(&cid)) {
                    return self.after_action("afterArchive", &list, at, true);
                }
            }
            UiMessage::PickEmoji(i) => {
                let picks = self.emoji_suggestions(&self.store.read());
                let mut text = self.pane.composer.text();
                let at = text.rfind(':').expect("an emoji query");
                text.replace_range(at.., &format!("{} ", picks[i.min(picks.len() - 1)].0));
                self.pane.composer = composer_with(&text);
                return operation::focus("composer-input");
            }
            UiMessage::CancelComposeMode => {
                self.pane.reply_to_message_id = None;
                self.pane.editing_message_id = None;
                self.pane.composer = composer_with("");
            }
            UiMessage::StartReply(mid) => {
                self.pane.editing_message_id = None;
                self.pane.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            UiMessage::StartEdit(mid, old_txt) => {
                self.pane.reply_to_message_id = None;
                self.pane.editing_message_id = Some(mid);
                self.pane.composer = composer_with(&old_txt);
                return operation::focus("composer-input");
            }
            UiMessage::DeleteMessage(cid, mid) => {
                self.confirm = None;
                let mut st = self.store.write();
                st.delete_message(&cid, &mid).expect("delete message");
                st.queue(&cid, Outgoing::Redact { target: mid });
            }
            UiMessage::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write();
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
                self.store.write().toggle_bookmark(&mid);
            }
            UiMessage::MarkUnreadFromHere(cid, mid) => {
                self.store.write().mark_unread_from_message(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            UiMessage::RetranscribeVoice(mid) => {
                self.status_banner = self.store.write().retranscribe_voice_note(&mid);
            }
            UiMessage::VotePoll(mid, answers) => {
                let mut st = self.store.write();
                let chat = st.messages[st.msg_idx_by_id[&mid]].chat_id.clone();
                st.vote_poll(&chat, &mid, answers);
            }
            UiMessage::SetReminder(cid, at) => self.set_reminder(&cid, at, None),
            UiMessage::RemindIf(cid, at, no_reply) => self.set_reminder(&cid, Some(at), Some(no_reply)),
            UiMessage::ChatSub(sub) => self.chat_sub = sub,
            UiMessage::CheckReminders => {
                let now = now_unix_millis();
                let mut st = self.store.write();
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
            UiMessage::OpenSendLater if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Send Later isn't available while editing a message").into();
            }
            UiMessage::OpenSendLater => {
                if self.pane.selected_chat_id.is_some() && self.pane.attachments.is_empty() {
                    return self.update(UiMessage::OpenSubMenu(commands::SubMenu::Schedule(Vec::new())));
                }
            }
            UiMessage::SendLater(config) => {
                self.pane.send_later = config;
                return operation::focus("composer-input");
            }
            UiMessage::ScheduledMenu(ids, items) => self.pane.sched_menu = Some((ids, items, self.cursor.get())),
            UiMessage::SendNow(ids) => {
                self.pane.sched_menu = None;
                let mut st = self.store.write();
                let due: Vec<Scheduled> =
                    ids.iter().filter_map(|id| st.scheduled.iter().find(|s| s.id == *id)).cloned().collect();
                ids.iter().for_each(|id| st.update_scheduled(id, |_| false));
                drop(st);
                self.send_scheduled(due);
            }
            UiMessage::Reschedule(ids) => {
                self.pane.sched_menu = None;
                return self.update(UiMessage::OpenSubMenu(commands::SubMenu::Schedule(ids)));
            }
            UiMessage::RescheduleTo(ids, at, no_reply) => {
                let mut st = self.store.write();
                let now = now_unix_millis();
                for id in &ids {
                    st.update_scheduled(id, |s| {
                        (s.canceled, s.added_on, s.send_on, s.send_only_on_no_response) = (false, now, at, no_reply);
                        true
                    });
                }
            }
            UiMessage::Unschedule(ids) => {
                self.pane.sched_menu = None;
                let mut st = self.store.write();
                ids.iter().for_each(|id| st.update_scheduled(id, |_| false));
            }
            UiMessage::EditScheduled(id) => {
                self.pane.sched_menu = None;
                let st = self.store.read();
                let text = st.scheduled.iter().find(|s| s.id == id).expect("a scheduled message").text.clone();
                drop(st);
                return self.update(UiMessage::StartEdit(id, text));
            }
            UiMessage::ShowReplies(id) => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), Some(id)),
            UiMessage::HideReplies => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), None),
            UiMessage::FoldScheduled(collapsed) => {
                self.pane.sched_fold.0 = collapsed;
                return operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            UiMessage::CheckScheduled => {
                let now = now_unix_millis();
                if let Some((_, no_reply)) = self.pane.send_later.filter(|c| c.0 <= now) {
                    self.pane.send_later = Some((now + 60_000, no_reply));
                }
                let due = self.store.write().take_due_scheduled(now);
                self.send_scheduled(due);
            }
            m @ (UiMessage::ArchiveStale | UiMessage::ArchiveAll(..)) => {
                self.confirm = None;
                {
                    let mut st = self.store.write();
                    let chats = match m {
                        UiMessage::ArchiveAll(read, days) => {
                            st.inbox_chats_before(read, now_unix_millis() - days * 86_400_000)
                        }
                        _ => st.stale_inbox_chats(),
                    };
                    for idx in chats {
                        assert!(st.set_chat_flag(idx, ChatFlag::Archived, Some(true)));
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::ToggleFlag(cid, flag) => {
                let mut st = self.store.write();
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
                    self.status_banner = tr("Pinned chats can't be archived").into();
                }
                self.chat_menu = None;
                self.refresh_theme_and_tray();
                let selected = self.pane.selected_chat_id.as_deref() == Some(cid.as_str());
                if let Some(at) = at.filter(|_| flag == ChatFlag::Archived && moved && selected) {
                    return self.after_action("afterArchive", &list, at, true);
                }
            }
            UiMessage::MarkAllRead => {
                self.confirm = None;
                let mut st = self.store.write();
                let unread: Vec<String> = self
                    .visible_chats(&st)
                    .iter()
                    .filter(|c| c.flags & CHAT_UNREAD != 0)
                    .map(|c| c.id.clone())
                    .collect();
                for id in unread {
                    let idx = st.resolve_chat_idx(&id).expect("a listed chat");
                    self.mark_read(&mut st, idx);
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::ChatWallpaper(chat, w) => {
                self.patch_settings(|s| s.chat_appearance.patch(chat.as_deref(), |t| t.wallpaper = Some(w)));
                self.prune_wallpapers();
            }
            UiMessage::ChatBubble(chat, b) => {
                self.patch_settings(|s| s.chat_appearance.patch(chat.as_deref(), |t| t.bubble = b))
            }
            UiMessage::ClearChatTheme(chat) => {
                self.patch_settings(|s| _ = s.chat_appearance.per_chat.remove(&chat));
                self.prune_wallpapers();
            }
            UiMessage::PickWallpaper(chat) => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), true);
                return Task::perform(pick, move |r| UiMessage::WallpaperPicked(chat.clone(), r));
            }
            UiMessage::WallpaperPicked(chat, r) => match r.map(|paths| paths.into_iter().next()) {
                Ok(Some(path)) => {
                    let dir = chat_theme::photo_dir(&self.store.read().data_dir);
                    let ext =
                        path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy().to_lowercase()));
                    let file_name = format!("{}{ext}", beeper_core::connect::random_id());
                    std::fs::create_dir_all(&dir).expect("create the chat wallpapers directory");
                    match std::fs::copy(&path, dir.join(&file_name)) {
                        Ok(_) => return self.update(UiMessage::ChatWallpaper(chat, Wallpaper::Photo { file_name })),
                        Err(e) => self.status_banner = format!("Could not import {}: {e}", path.display()),
                    }
                }
                Ok(None) => {}
                Err(e) => self.status_banner = e,
            },
            UiMessage::OpenChatTheme(chat) => {
                if let Some(&(id, _)) = self.theme_windows.iter().find(|w| w.1 == chat) {
                    return window::gain_focus(id);
                }
                let (id, open) = self.open_child(Size::new(380.0, 620.0));
                self.theme_windows.push((id, chat));
                return open;
            }
            UiMessage::OpenPollCreator(chat) => {
                if let Some(d) = self.poll_windows.iter().find(|d| d.chat == chat) {
                    return window::gain_focus(d.id);
                }
                let (id, open) = self.open_child(Size::new(400.0, 620.0));
                let question = format!("poll-{chat}");
                self.poll_windows.push(PollDraft::new(id, chat));
                return open.chain(operation::focus(question));
            }
            UiMessage::Poll(id, edit) => return self.poll_edit(id, edit),
            UiMessage::EndPoll(chat, mid) => self.store.write().end_poll(&chat, &mid),
            UiMessage::OpenPollVotes(chat, mid) => {
                if let Some(w) = self.vote_windows.iter().find(|w| w.2 == mid) {
                    return window::gain_focus(w.0);
                }
                let (id, open) = self.open_child(Size::new(342.0, 563.0));
                self.vote_windows.push((id, chat, mid));
                return open;
            }
            UiMessage::MarkRead(cid) => {
                let mut st = self.store.write();
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                self.mark_read(&mut st, idx);
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::ToggleRead(cid) => {
                let mut st = self.store.write();
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
                if let Some(at) = at.filter(|_| self.pane.selected_chat_id.as_deref() == Some(cid.as_str())) {
                    return self.after_action("afterMarkRead", &list, at, false);
                }
            }
            UiMessage::CopyChatLink(cid) => {
                self.chat_menu = None;
                let st = self.store.read();
                let c = &st.chats[st.resolve_chat_idx(&cid).expect("chat exists")];
                return iced::clipboard::write(format!(
                    "beeper://select-thread/{}/{}",
                    c.account_id,
                    encode_uri_component(&c.id)
                ));
            }
            UiMessage::OpenChatMenu(cid) => {
                self.chat_menu = Some((cid, self.cursor.get()));
                self.chat_sub = None;
            }
            UiMessage::OpenMessageMenu(mid) => {
                self.video.iter_mut().for_each(|v| v.sub = false);
                self.pane.message_menu = Some((mid, self.cursor.get()));
            }
            UiMessage::SelectMessages(mids) => self.pane.selected_msgs = mids,
            UiMessage::CopySelected(stamp) => {
                let st = self.store.read();
                let (msgs, _) = self.timeline(&st);
                let picked: Vec<&Message> =
                    msgs.into_iter().filter(|m| self.pane.selected_msgs.contains(&m.id)).collect();
                let many = picked.len() > 1;
                let text: Vec<String> = picked.iter().map(|m| copy_text(m, stamp, many)).collect();
                drop(st);
                return self.update(UiMessage::CopyText(text.join("\n")));
            }
            UiMessage::ClickMessage(mid)
                if !self.mods.shift()
                    && self.store.read().settings.on("undoOnPendingClick")
                    && self.queued.iter().any(|q| q.sends.iter().any(|s| s.0 == mid)) =>
            {
                return self.update(UiMessage::UndoSend);
            }
            UiMessage::ClickMessage(mid) => {
                let picked = self.pane.selected_msgs.contains(&mid);
                if self.mods.shift() {
                    let st = self.store.read();
                    let (msgs, live) = self.timeline(&st);
                    let at = |id: &str| msgs.iter().position(|m| m.id == id).expect("the message is in the open chat");
                    let (a, b) = (at(self.pane.msg_anchor.as_deref().unwrap_or(&mid)), at(&mid));
                    let range = (a.min(b)..=a.max(b)).filter(|&i| live[i]).map(|i| msgs[i].id.clone());
                    let range: Vec<String> = range.filter(|id| !self.pane.selected_msgs.contains(id)).collect();
                    drop(st);
                    self.pane.selected_msgs.extend(range);
                } else {
                    match (self.mods.command(), picked) {
                        (true, false) => self.pane.selected_msgs.push(mid.clone()),
                        (false, false) => self.pane.selected_msgs = vec![mid.clone()],
                        (_, true) => self.pane.selected_msgs.retain(|m| *m != mid),
                    }
                }
                self.pane.msg_anchor = Some(mid);
            }
            UiMessage::DoubleClickMessage(mid) => {
                let action = self.store.read().settings.pref("messageDoubleClick");
                return match action {
                    0 => Task::none(),
                    1 => self.update(UiMessage::React(mid)),
                    3 => self.update(UiMessage::Forward(vec![mid])),
                    2 => {
                        self.pane.selected_msgs = vec![mid.clone()];
                        self.update(UiMessage::StartReply(mid))
                    }
                    4 => self.update(UiMessage::OpenMessageMenu(mid)),
                    5 => self.update(self.quick_react(mid)),
                    n => unreachable!("messageDoubleClick option {n}"),
                };
            }
            UiMessage::StepMessage(down, extend) => {
                let st = self.store.read();
                let (msgs, live) = self.timeline(&st);
                let live = |i: &usize| live[*i];
                let picked = |m: &&Message| self.pane.selected_msgs.contains(&m.id);
                let (first, last) = (msgs.iter().position(picked), msgs.iter().rposition(picked));
                let extend = extend && last.is_some();
                let next = match (first.zip(last), down) {
                    (None, false) => (0..msgs.len()).rev().find(live),
                    (None, true) => None,
                    (Some((_, l)), true) => (l + 1..msgs.len()).find(live),
                    (Some((f, _)), false) if extend => (0..f).rev().find(live),
                    (Some((_, l)), false) => (0..l).rev().find(live).or(Some(l).filter(live)),
                };
                let next = next.map(|i| msgs[i].id.clone());
                drop(st);
                match (extend, next) {
                    (true, Some(mid)) => self.pane.selected_msgs.push(mid),
                    (true, None) => return Task::none(),
                    (false, next) => self.pane.selected_msgs = next.into_iter().collect(),
                }
                return match self.pane.selected_msgs.last() {
                    Some(mid) => locate(format!("msg-{mid}"), "timeline", true),
                    None => operation::focus("composer-input"),
                };
            }
            UiMessage::React(mid) => return self.open_emoji(Some((mid, self.cursor.get()))),
            UiMessage::MessageMenu(msg) => {
                self.pane.message_menu = None;
                return self.update(*msg);
            }
            UiMessage::CopyText(s) => return iced::clipboard::write(s),
            UiMessage::CopyImage(mid, ai) => {
                let st = self.store.read();
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
                    self.status_banner = tr("Couldn't copy the image").into();
                }
            }
            UiMessage::OpenCarousel(mid, ai, in_media) => {
                self.carousel = Some(Carousel { mid, ai, turns: 0, in_media });
                return self.load_media();
            }
            UiMessage::CarouselStep(next) => {
                let (items, at) = self.carousel_items(&self.store.read());
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
            UiMessage::Modifiers(mods) => {
                self.mods = mods;
                if self.cycling && !mods.control() {
                    self.cycling = false;
                    if let Some(c) = self.pane.selected_chat_id.clone() {
                        self.push_history(&c);
                    }
                }
            }
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
                let st = self.store.read();
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
                let st = self.store.read();
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
                    true => self.status_banner = tr("These messages can’t be deleted").into(),
                    false => self.media_mut().deleting = Some((pairs, skipped)),
                }
            }
            UiMessage::MediaDelete(go) => {
                let (pairs, skipped) = self.media_mut().deleting.take().expect("the delete dialog is open");
                if go {
                    let mut st = self.store.write();
                    for (cid, mid) in &pairs {
                        st.delete_message(cid, mid).expect("delete message");
                        st.queue(cid, Outgoing::Redact { target: mid.clone() });
                    }
                    drop(st);
                    self.media_mut().picked.retain(|(m, _)| pairs.iter().all(|(_, x)| x != m));
                    if skipped > 0 {
                        self.status_banner = trn(
                            "Skipped %d item that can’t be deleted",
                            "Skipped %d items that can’t be deleted",
                            skipped,
                            &[&skipped],
                        );
                    }
                }
            }
            UiMessage::MediaItemMenu(item) => {
                let at = self.cursor.get();
                self.media_mut().item_menu = item.map(|(mid, k)| (mid, k, at));
            }
            UiMessage::MediaItemAct(msg) => {
                self.media_mut().item_menu = None;
                return self.update(*msg);
            }
            UiMessage::CopyLink(url) => {
                self.status_banner = tr("Copied to clipboard!").into();
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
            UiMessage::InputEscape(id) if Some(id) == self.settings_window && self.integrations.add.is_some() => {
                self.integrations.add = None;
            }
            UiMessage::InputEscape(id) if Some(id) == self.settings_window => {
                return self.update(UiMessage::SettingsQueryChanged(String::new()));
            }
            UiMessage::InputEscape(id) => {
                return self.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), keyboard::Modifiers::empty()));
            }
            UiMessage::WindowKey(id, key, mods) => {
                if Some(id) == self.settings_window {
                    return self.settings_key(key, mods);
                }
                if self.theme_windows.iter().any(|w| w.0 == id) || self.vote_windows.iter().any(|w| w.0 == id) {
                    return if key == Key::Named(Named::Escape) { window::close(id) } else { Task::none() };
                }
                if self.poll_windows.iter().any(|w| w.id == id) {
                    return match key {
                        Key::Named(Named::Escape) => self.poll_edit(id, PollEdit::Close),
                        Key::Named(Named::Enter) if mods.command() => self.poll_edit(id, PollEdit::Send),
                        _ => Task::none(),
                    };
                }
                if self.media_window.as_ref().is_none_or(|w| w.id != id) {
                    let key = UiMessage::KeyPressed(key, mods);
                    return match self.chat_windows.iter().any(|w| w.0 == id) {
                        true => self.update(UiMessage::InWindow(id, Box::new(key))),
                        false => self.update(key),
                    };
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
            UiMessage::Forward(mids) => {
                self.forward = Some(Forward { mids, picked: Vec::new(), query: String::new() });
                return operation::focus("forward-input");
            }
            UiMessage::ForwardQuery(q) => self.forward.as_mut().expect("the forward dialog is open").query = q,
            UiMessage::ForwardSubmit => {
                let st = self.store.read();
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
                let Forward { mids, picked, .. } = self.forward.take().expect("the forward dialog is open");
                assert!(!picked.is_empty() && !mids.is_empty(), "forward needs a chat and a message");
                let mut st = self.store.write();
                for cid in &picked {
                    for mid in &mids {
                        let m = st.messages[st.msg_idx_by_id[mid]].clone();
                        let (_, txn, _) = st
                            .send_message(cid, m.text.clone(), None, m.attachments.first().cloned())
                            .expect("forward message");
                        let (from, event, text, html) = (m.chat_id, m.id, m.text, m.html);
                        st.queue(cid, Outgoing::Forward { txn, from, event, text, html });
                    }
                }
                drop(st);
                if let [one] = picked.as_slice() {
                    self.select_chat(one);
                }
            }
            UiMessage::Download(mid, only) => {
                let st = self.store.read();
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
                self.status_banner = trf("Downloading %s", &[&files[0].1]);
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
                    Ok(name) => trf("Downloaded %s", &[&name]),
                    Err(e) => trf("Download failed: %s", &[&e]),
                }
            }
            UiMessage::AskDeleteMessage(mid) => {
                (self.confirm, self.delete_target) = (Some(ASK_DELETE_MESSAGE), Some(mid))
            }
            UiMessage::NicknameChanged(v) => self.nickname_edit_buffer = v,
            UiMessage::SaveNickname => {
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                let mut st = self.store.write();
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
                (self.cmd_bar_commands_mode, self.cmd_index, self.cmd_submenu) = (false, 0, None);
                self.search_expanded = false;
                if self.show_command_palette {
                    return operation::focus("cmd-palette-input");
                }
            }
            UiMessage::SearchRoom(cid) => {
                (self.show_command_palette, self.cmd_bar_commands_mode, self.cmd_submenu) = (true, false, None);
                (self.search_room, self.chat_menu) = (Some(cid), None);
                self.search_filters.accounts.clear();
                return operation::focus("cmd-palette-input");
            }
            UiMessage::ToggleCommandMode => {
                (self.cmd_bar_commands_mode, self.cmd_index) = (!self.cmd_bar_commands_mode, 0);
                return operation::focus("cmd-palette-input");
            }
            UiMessage::CmdNoReply(on) => self.cmd_no_reply = on,
            UiMessage::LastTime(phrase) => self.patch_settings(|s| s.last_time = phrase),
            UiMessage::OpenSubMenu(menu) => {
                let st = self.store.read();
                self.cmd_no_reply = match &menu {
                    commands::SubMenu::Schedule(ids) => {
                        let initial = match ids.first() {
                            Some(id) => st.scheduled.iter().find(|s| s.id == *id).map(|s| s.send_only_on_no_response),
                            None => self.pane.send_later.map(|c| c.1),
                        };
                        initial.unwrap_or(st.settings.pref("sendLaterDefault") == 0)
                    }
                    _ => st.settings.pref("remindLaterDefault") == 0,
                };
                drop(st);
                (self.show_command_palette, self.cmd_bar_commands_mode, self.cmd_submenu) = (true, true, Some(menu));
                (self.search_room, self.chat_menu, self.cmd_index) = (None, None, 0);
                self.command_palette_query.clear();
                return operation::focus("cmd-palette-input");
            }
            UiMessage::ToggleSearchExpanded => self.search_expanded = !self.search_expanded,
            UiMessage::CommandQueryChanged(q) => (self.command_palette_query, self.cmd_index) = (q, 0),
            UiMessage::CommandHighlight(i) => self.cmd_index = i,
            UiMessage::Commands(msgs) => {
                return Task::batch(msgs.into_iter().map(|m| self.update(m)).collect::<Vec<_>>())
            }
            UiMessage::RunHotkey(name) => return self.run_hotkey(name, 0),
            UiMessage::ToggleMaximize => return window::toggle_maximize(self.main_window.expect("the main window")),
            UiMessage::JumpTo(cid, mid) => {
                (self.show_command_palette, self.command_palette_query) = (false, String::new());
                self.select_chat(&cid);
                let msg = format!("msg-{mid}");
                self.flash = Some((mid, std::time::Instant::now()));
                return locate(msg, "timeline", false);
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
                let st = self.store.read();
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
            UiMessage::SelectSettingsTab(SettingsTab::Help) | UiMessage::OpenSettings(SettingsTab::Help) => {
                return Task::done(UiMessage::OpenUrl("https://help.beeper.com/?ref=desktop-app".into()));
            }
            UiMessage::SelectSettingsTab(t) => {
                let top = operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                self.active_settings_tab = t;
                (self.account_page, self.app_qr) = (None, false);
                return Task::batch([top, self.load_devices()]);
            }
            UiMessage::DevicesLoaded(r) => self.devices = Some(r),
            UiMessage::MicsLoaded(r) => {
                if let Err(e) = &r {
                    eprintln!("Unable to load audio input devices: {e}");
                }
                self.mics = Some(r);
            }
            UiMessage::SetMicrophone(name) => self.patch_settings(|s| s.microphone = name),
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
                    let mut st = self.store.write();
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
                            self.status_banner = tr("Pinned chats can't be archived").into();
                        }
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectionRemind(at) => {
                self.selection_menu = None;
                {
                    let mut st = self.store.write();
                    for cid in std::mem::take(&mut self.selection) {
                        let idx = st.resolve_chat_idx(&cid).expect("selected chat exists");
                        if at.is_some() || st.chats[idx].reminder.is_some() {
                            self.remind(&mut st, idx, at, None);
                        }
                    }
                }
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectionMenu(menu, x) => self.selection_menu = Some((menu, x)),
            UiMessage::DeleteChat(id, everyone) => {
                self.confirm = None;
                self.store.write().delete_chat(&id, everyone);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::Voice(msg) => return self.voice(msg),
            UiMessage::Record(msg) => return self.record(msg),
            UiMessage::ReminderMenu => {
                self.reminder_menu = Some(self.cursor.get());
            }
            UiMessage::OrderReminders(i) => {
                self.reminder_menu = None;
                self.patch_settings(|s| s.reminders_order = i);
            }
            UiMessage::DismissAllReminders => {
                self.confirm = None;
                let mut st = self.store.write();
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
                if !self.pane.open_members.remove(&id) {
                    self.pane.open_members.insert(id);
                }
            }
            UiMessage::ToggleDnd => {
                self.dnd_muted = !self.dnd_muted;
                self.refresh_theme_and_tray();
            }
            UiMessage::CycleTheme => {
                let cur = self.store.read().settings.theme;
                let next = ThemeChoice::ALL[(ThemeChoice::ALL.iter().position(|t| *t == cur).expect("listed") + 1) % 3];
                return self.update(UiMessage::SelectTheme(next));
            }
            UiMessage::SystemTheme(mode) => {
                self.system_dark = mode == iced::theme::Mode::Dark;
                self.refresh_theme_and_tray();
            }
            UiMessage::SelectTheme(choice) => self.patch_settings(|s| s.theme = choice),
            UiMessage::ToggleSetting(flip) => self.patch_settings(flip),
            UiMessage::CloudAccounts(Ok((email, subscribed, accounts))) => {
                let (token, uid, demo) = {
                    let mut st = self.store.write();
                    st.features.subscribed = subscribed;
                    let info = (st.beeper_state.access_token.clone(), st.beeper_state.user_id.clone(), st.config.demo);
                    if st.me().email != email {
                        let i = st.chat_idx_by_id[NOTE_CHAT_ID];
                        st.update_chat(i, |c| c.participants.items[0].email = email);
                    }
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
                    self.store.write().set_contacts(contacts);
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
                    let st = self.store.read();
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
                // Like Desktop, a resolve whose edit does not arrive in 30 s fails.
                let resolve = async move {
                    beeper_core::matrix::resolve_media(
                        homeserver,
                        access_token,
                        user_id,
                        bridge_id,
                        login_id,
                        eid,
                        raw,
                    )
                    .await?;
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    Ok(())
                };
                return Task::perform(resolve, move |r| UiMessage::MediaResolved(event_id, r));
            }
            UiMessage::MediaResolved(event_id, r) => {
                self.resolving_media.insert(event_id, false);
                if let Err(e) = r {
                    eprintln!("resolve media failed: {e}");
                    self.status_banner = tr("Could not load this media.").into();
                }
            }
            UiMessage::Video(msg) => return self.video(msg),
            UiMessage::Pasted(r) => match r {
                Ok(img) => {
                    self.pane.send_later = self.pane.send_later.filter(|_| img.is_none());
                    self.pane.attachments.extend(img.map(|(png, size)| PendingMedia {
                        name: "image.png".into(),
                        mime: "image/png".into(),
                        is_gif: false,
                        size,
                        frames: vec![(image::Handle::from_bytes(png.clone()), 0)],
                        bytes: png,
                    }));
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::DiscardAttachment(i) => drop(self.pane.attachments.remove(i)),
            UiMessage::PickFiles if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            UiMessage::PickFiles => {
                let title = tr("Open").to_string();
                return Task::perform(portal::open_files(portal::PORTAL.into(), title, false), UiMessage::FilesPicked);
            }
            UiMessage::FilesPicked(r) => match r {
                Ok(paths) => {
                    let load = move || paths.iter().map(|p| portal::load_file(p)).collect();
                    let load = async { tokio::task::spawn_blocking(load).await.expect("file load thread") };
                    return Task::perform(load, UiMessage::FilesLoaded);
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::FilesLoaded(Ok(_)) if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            UiMessage::FilesLoaded(r) => match r {
                Ok(files) => {
                    self.pane.send_later = self.pane.send_later.filter(|_| files.is_empty());
                    self.pane.attachments.extend(files);
                    if self.drop_send {
                        self.submit_composer(false);
                    }
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::FileHover(id, on) if Some(id) == self.main_window => {
                self.dropping = on && self.pane.selected_chat_id.is_some();
                self.drop_send &= !on;
            }
            UiMessage::FileDropped(id, path)
                if Some(id) == self.main_window && self.pane.selected_chat_id.is_some() =>
            {
                self.dropping = false;
                self.drop_send |= self.mods.shift();
                return self.update(UiMessage::FilesPicked(Ok(vec![path])));
            }
            UiMessage::FileHover(..) | UiMessage::FileDropped(..) => {}
            UiMessage::LinkPreviewLoaded(url, r) => {
                let entry =
                    self.pane.composer_links.iter_mut().find(|(u, s)| *u == url && matches!(s, LinkState::Loading));
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
                self.pane
                    .composer_links
                    .iter_mut()
                    .filter(|(u, _)| *u == url)
                    .for_each(|(_, s)| *s = LinkState::Discarded);
            }
            UiMessage::Emoji(m) => return self.update_emoji(m),
            UiMessage::Upserted(m) => {
                let mut st = self.store.write();
                let due = st.take_due_scheduled(now_unix_millis());
                let open = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_canonical_chat_id(c));
                if open.as_ref() == Some(&m.chat_id)
                    && self.pane.sched_fold == (false, false)
                    && st.scheduled.iter().any(|s| s.chat_id == m.chat_id && !s.canceled)
                {
                    self.pane.sched_fold = (true, true);
                }
                drop(st);
                self.send_scheduled(due);
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
            UiMessage::UndoSend => {
                let open = self.pane.selected_chat_id.as_ref();
                let Some(i) = self.queued.iter().position(|q| Some(&q.chat) == open) else { return Task::none() };
                let q = self.queued.remove(i);
                let mut st = self.store.write();
                for (id, _) in &q.sends {
                    st.remove_message(&q.chat, id).expect("a queued message is in its chat");
                }
                drop(st);
                self.pane.composer = composer_with(&q.text);
                (self.pane.reply_to_message_id, self.pane.attachments) = (q.reply_to, q.files);
                return Task::batch([self.composer_effects(), operation::focus("composer-input")]);
            }
            UiMessage::SendQueued(chat) => {
                let now = now_unix_millis();
                let (send, keep) = std::mem::take(&mut self.queued)
                    .into_iter()
                    .partition(|q| chat.as_ref().map_or(q.due <= now, |c| q.chat == *c));
                self.queued = keep;
                let mut st = self.store.write();
                for q in send.into_iter().flat_map(|q: Queued| q.sends.into_iter().map(move |s| (q.chat.clone(), s.1)))
                {
                    st.queue(&q.0, q.1);
                }
            }
            UiMessage::TogglePicker(kind) => {
                let open = self.pane.picker != Some(kind) && self.supports(kind);
                self.pane.picker = open.then_some(kind);
                if open && kind == Picker::Gif {
                    self.pane.show_attach_menu = true;
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
                self.pane.picker = None;
                let url = g.gif_url.clone();
                return Task::perform(
                    async move {
                        let bytes = beeper_core::cloud::http_bytes(&url).await?;
                        let b = bytes.clone();
                        let frames = tokio::task::spawn_blocking(move || decode_media(b)).await;
                        Ok((bytes, frames.expect("media decode task")?))
                    },
                    move |r| UiMessage::GifDownloaded(g, r),
                );
            }
            UiMessage::GifDownloaded(g, r) => match r {
                Ok((bytes, frames)) => {
                    self.pane.send_later = None;
                    self.pane.attachments.push(PendingMedia {
                        bytes,
                        name: format!("{}.gif", g.id),
                        mime: "image/gif".into(),
                        is_gif: true,
                        size: g.gif_size,
                        frames,
                    });
                    if self.store.read().settings.on("autoSendGifs") {
                        self.submit_composer(false);
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
            UiMessage::Verified(Ok(())) => self.status_banner = tr("This device is verified.").into(),
            UiMessage::Verified(Err(e)) => self.status_banner = format!("Could not verify this device: {e}"),
            UiMessage::Pushed(Ok(())) => {}
            UiMessage::DismissBanner => (self.status_banner, self.relaunch_toast) = (String::new(), None),
            UiMessage::Relaunch => {
                RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                return self.quit();
            }
            UiMessage::Pushed(Err(e)) => self.status_banner = format!("Could not send to Matrix: {e}"),
            UiMessage::VerifySasNow => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.store.write().sas_confirm = Some(tx);
                return Task::perform(beeper_core::matrix::verify(self.store.clone(), rx), UiMessage::Verified);
            }
            UiMessage::SasMatch(same) => {
                let tx = self.store.write().sas_confirm.take().expect("emojis are shown once");
                tx.send(same).ok();
            }
            UiMessage::StorageScanned(all) => {
                self.storage.chart = Some(donut(&storage_totals(&all)));
                self.storage.all = Some(all);
            }
            UiMessage::StorageAge(which, i) => self.storage.ages[which] = i,
            UiMessage::StorageKind(which, i) => self.storage.kinds[which] ^= 1 << i,
            UiMessage::StorageChat(id) => {
                let s = &mut self.storage;
                (s.chat, s.ages[1], s.kinds[1]) = ((s.chat.as_ref() != Some(&id)).then_some(id), 1, 0b11111);
            }
            UiMessage::ClearStorage(which) => {
                let mut picked: Vec<storage::Entry> = self.storage_pick(which).into_iter().cloned().collect();
                if which == 0 {
                    picked.extend(self.storage_pick(1).into_iter().cloned());
                }
                (self.confirm, self.storage.clearing) = (None, Some(which));
                return Task::perform(storage::clear(self.store.clone(), picked), UiMessage::StorageCleared);
            }
            UiMessage::StorageCleared(r) => {
                self.storage.clearing = None;
                if let Err(e) = r {
                    eprintln!("Unable to clear storage: {e}");
                    self.status_banner = tr("Failed to clear storage").into();
                }
                return self.scan_storage();
            }
            UiMessage::McpTarget(i) => self.integrations.target = i,
            UiMessage::McpInstall(client) => {
                let value = mcp_value(client, &self.store.read().config.base_url());
                let app = match client {
                    "Claude Desktop" => return self.update(UiMessage::OpenUrl(value)),
                    "Cursor" => "Cursor",
                    "VS Code" => "Visual Studio Code",
                    _ => {
                        self.status_banner = tr("Command copied to clipboard! Run it in your terminal.").into();
                        return iced::clipboard::write(value);
                    }
                };
                self.status_banner = trf("Opening %s to install Beeper MCP...", &[&app]);
                return self.update(UiMessage::OpenUrl(value));
            }
            UiMessage::CopyField(i, value) => {
                self.integrations.copied = Some(i);
                let done = Task::perform(tokio::time::sleep(std::time::Duration::from_secs(2)), move |()| {
                    UiMessage::CopyDone(i)
                });
                return Task::batch([iced::clipboard::write(value), done]);
            }
            UiMessage::CopyDone(i) => {
                if self.integrations.copied == Some(i) {
                    self.integrations.copied = None;
                }
            }
            UiMessage::JoinCommunity => {
                if self.store.read().resolve_chat_idx(COMMUNITY_ROOM).is_some() {
                    let close = window::close(self.settings_window.expect("the settings window is open"));
                    return Task::batch([self.update(UiMessage::SelectChat(COMMUNITY_ROOM.into())), close]);
                }
                if !self.integrations.joining {
                    self.integrations.joining = true;
                    let client = self.store.read().matrix.clone();
                    return Task::perform(
                        beeper_core::matrix::join(client, COMMUNITY_ROOM),
                        UiMessage::CommunityJoined,
                    );
                }
            }
            UiMessage::TokenRequest(id, None) => {
                let st = self.store.read();
                if let Some(r) = st.connect.requests.iter().find(|r| r.id == id) {
                    let name = r.client.name.clone().filter(|n| !n.is_empty());
                    let name = name.unwrap_or_else(|| tr("Unknown Client").into());
                    let version = r.client.version.as_ref().map(|v| format!("v{v}")).unwrap_or_default();
                    let write = r.scopes.iter().any(|s| s == beeper_core::connect::WRITE);
                    let app = format!("{name} {version}").trim_end().to_string();
                    self.token_ask = Some(TokenAsk { id, app, write, expiry: Expiry(2_592_000), timed_out: false });
                }
            }
            UiMessage::TokenRequest(id, Some(outcome)) => {
                if let Some(a) = self.token_ask.as_mut().filter(|a| a.id == id) {
                    a.timed_out = outcome == beeper_core::connect::Outcome::Timeout;
                }
            }
            UiMessage::TokenWrite => self.token_ask.as_mut().expect("an open token request").write ^= true,
            UiMessage::TokenExpiry(e) => self.token_ask.as_mut().expect("an open token request").expiry = e,
            UiMessage::TokenAnswer(approve) => {
                let a = self.token_ask.take().expect("an open token request");
                let scopes = match a.write {
                    true => vec![beeper_core::connect::READ.into(), beeper_core::connect::WRITE.into()],
                    false => vec![beeper_core::connect::READ.into()],
                };
                self.store.write().answer_token_request(&a.id, approve.then_some((scopes, a.expiry.0)));
            }
            UiMessage::TokenClose => self.token_ask = None,
            UiMessage::RevokeConnection(id) => {
                (self.integrations.revoke, self.confirm) = (Some(id), Some(ASK_REVOKE_CONNECTION))
            }
            UiMessage::RemoteAccess(true) if self.confirm != Some(ASK_REMOTE_ACCESS) => {
                self.confirm = Some(ASK_REMOTE_ACCESS);
            }
            UiMessage::RemoteAccess(on) => {
                self.confirm = None;
                self.patch_settings(|s| s.remote_access = on);
                self.relaunch_toast = Some(match on {
                    true => "Remote Access will be enabled after relaunch",
                    false => "Remote Access will be disabled after relaunch",
                });
            }
            UiMessage::RevokeConfirmed => {
                self.confirm = None;
                self.store.write().revoke_token(&self.integrations.revoke.take().expect("a connection to revoke"));
            }
            UiMessage::AddToken(open) => {
                self.integrations.add = open.then(|| AddToken {
                    name: String::new(),
                    expiry: Expiry(2_592_000),
                    write: false,
                    token: None,
                    copied: false,
                });
                return operation::focus("token-name");
            }
            UiMessage::AddTokenName(name) => self.integrations.add.as_mut().expect("the token dialog").name = name,
            UiMessage::AddTokenExpiry(e) => self.integrations.add.as_mut().expect("the token dialog").expiry = e,
            UiMessage::AddTokenWrite => self.integrations.add.as_mut().expect("the token dialog").write ^= true,
            UiMessage::CreateToken => {
                let a = self.integrations.add.as_mut().expect("the token dialog");
                let mut scopes = vec![beeper_core::connect::READ.to_string()];
                scopes.extend(a.write.then(|| beeper_core::connect::WRITE.to_string()));
                let client = (!a.name.is_empty())
                    .then(|| beeper_core::connect::ClientInfo { name: Some(a.name.clone()), ..Default::default() });
                a.token = Some(self.store.write().create_token(scopes, a.expiry.0, client));
            }
            UiMessage::CopyToken => {
                let a = self.integrations.add.as_mut().expect("the token dialog");
                a.copied = true;
                return iced::clipboard::write(a.token.clone().expect("a created token"));
            }
            UiMessage::CommunityJoined(r) => {
                self.integrations.joining = false;
                match r {
                    Ok(()) => {
                        self.status_banner = tr("Successfully joined the Beeper community room!").into();
                        self.banner_action = Some((self.status_banner.clone(), tr("Open"), UiMessage::JoinCommunity));
                    }
                    Err(e) => {
                        eprintln!("Failed to join community room: {e}");
                        self.status_banner = tr(
                            "Something went wrong while joining the Beeper Developer Community chat. Please try again.",
                        )
                        .into();
                    }
                }
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
            UiMessage::CloseModals
                if self.pane.message_menu.is_none() && self.video.as_ref().is_some_and(video::Video::is_full) =>
            {
                return self.video(video::Msg::Fullscreen);
            }
            UiMessage::CloseModals => {
                if self.show_command_palette
                    || self.forward.is_some()
                    || self.show_filter_menu
                    || self.chat_menu.is_some()
                    || self.pane.message_menu.is_some()
                    || self.pane.edit_menu.is_some()
                    || self.pane.sched_menu.is_some()
                    || self.reminder_menu.is_some()
                    || self.audio.menu.is_some()
                    || self.selection_menu.is_some()
                    || self.pane.show_attach_menu
                    || self.pane.picker.is_some()
                    || self.pane.emoji.is_some()
                    || self.quick_look_target.is_some()
                    || self.hotkeys_modal.is_some()
                {
                    self.hotkeys_modal = None;
                    self.show_command_palette = false;
                    self.forward = None;
                    self.show_filter_menu = false;
                    self.chat_menu = None;
                    self.pane.message_menu = None;
                    self.pane.edit_menu = None;
                    self.pane.sched_menu = None;
                    self.reminder_menu = None;
                    self.audio.menu = None;
                    self.selection_menu = None;
                    self.pane.show_attach_menu = false;
                    self.pane.picker = None;
                    self.pane.emoji = None;
                    self.quick_look_target = None;
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                } else if self.join_matrix_mode {
                    self.join_matrix_mode = false;
                } else if self.creating_chat.is_some() {
                    (self.creating_chat, self.pane.selected_chat_id) = (None, None);
                } else if self.pane.reply_to_message_id.is_some() {
                    self.pane.reply_to_message_id = None;
                } else if self.pane.editing_message_id.is_some() {
                    return self.update(UiMessage::CancelComposeMode);
                } else if !self.pane.selected_msgs.is_empty() {
                    self.pane.selected_msgs.clear();
                } else if self.pane.reply_thread.is_some() {
                    self.pane.reply_thread = None;
                } else if self.pane.selected_chat_id.is_some() {
                    self.leave_chat(&mut self.store.write(), None);
                    self.pane.selected_chat_id = None;
                    self.pane.composer = composer_with("");
                    self.refresh_theme_and_tray();
                } else if self.active_tab != InboxTab::AllChats {
                    self.active_tab = InboxTab::AllChats;
                } else {
                    self.status_banner.clear();
                }
            }
            UiMessage::ToggleEditHistory(mid) => {
                self.pane.expanded_history_msg_id =
                    (self.pane.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            UiMessage::Noop => {}
            UiMessage::OpenUrl(url) => {
                let (cmd, args) = open_url_or_path_command(self.os, &url);
                let open = move || match std::process::Command::new(&cmd).args(args).status() {
                    Ok(s) if s.success() => Ok(()),
                    Ok(s) => Err(format!("Could not open {url}: {cmd} {s}")),
                    Err(e) => Err(format!("Could not open {url}: {cmd}: {e}")),
                };
                return Task::perform(
                    async { tokio::task::spawn_blocking(open).await.expect("URL opener thread") },
                    UiMessage::Opened,
                );
            }
            UiMessage::Opened(r) => {
                if let Err(e) = r {
                    self.status_banner = e;
                }
            }
            UiMessage::ToggleAttachMenu => {
                self.pane.show_attach_menu = !self.pane.show_attach_menu;
                if !self.pane.show_attach_menu {
                    self.pane.picker = None;
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
            UiMessage::ShowAppQr => self.app_qr = true,
            UiMessage::OpenAccount(id) => {
                self.account_page = id;
                return operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            UiMessage::SetAccountProps(id, props) => {
                self.chat_menu = None;
                self.patch_settings(|s| {
                    s.accounts.insert(id, props);
                })
            }
            UiMessage::SetInboxVisibility(key, option) => {
                self.chat_menu = None;
                let st = self.store.read();
                let client = st.matrix.clone().expect("Show in Inbox needs a Matrix client");
                let rules = beeper_core::matrix::with_inbox_visibility(st.inbox_rules.clone(), &key, option);
                return Task::perform(
                    beeper_core::matrix::set_global(client, beeper_core::matrix::RULES, rules),
                    UiMessage::Pushed,
                );
            }
            UiMessage::SettingsQueryChanged(q) => {
                let st = self.store.read();
                let n = search_settings(&st, &q).len();
                if n != search_settings(&st, &self.settings_query).len() {
                    self.settings_focus = 0;
                    self.settings_pick = self.settings_pick.filter(|_| n > 0);
                }
                self.settings_query = q;
            }
            UiMessage::SettingsFocus(i) => self.settings_focus = i,
            UiMessage::SettingsStep(step) => {
                self.settings_focus =
                    (self.settings_focus + step) % search_settings(&self.store.read(), &self.settings_query).len();
                return locate(format!("hit-{}", self.settings_focus), "settings-nav", true);
            }
            UiMessage::PickSetting(hit) => {
                self.settings_pick = Some(hit);
                let tab = match hit {
                    SettingsHit::Section(t, _) | SettingsHit::Pref(t, _) | SettingsHit::Custom(t, _) => t,
                    SettingsHit::Network(_) => SettingsTab::Accounts,
                };
                let load =
                    Task::batch([self.update(UiMessage::SelectSettingsTab(tab)), operation::focus("settings-search")]);
                return match hit {
                    SettingsHit::Pref(_, &prefs::Pref { key, .. }) | SettingsHit::Custom(_, key) => {
                        self.pref_flash = Some((key, std::time::Instant::now()));
                        Task::batch([load, locate(key, "settings-pane", false)])
                    }
                    SettingsHit::Network(_) => {
                        self.adding_account = true;
                        load
                    }
                    SettingsHit::Section(..) => load,
                };
            }
            UiMessage::SetPref(key, v) => {
                self.patch_settings(|s| s.set_pref(key, v));
                self.relaunch_toast = match key {
                    "spellcheck" | "quitOnClose" => Some("Preference changed"),
                    _ => self.relaunch_toast,
                };
                if key == "incognito" {
                    let typing = self.store.read().settings.on("typingIndicators");
                    let (text, action) = match (v != 0, typing) {
                        (true, true) => {
                            ("Incognito mode on. Also hide when you're typing from recipients?", "Hide typing")
                        }
                        (false, false) => {
                            ("Incognito mode off. Also show when you're typing to recipients?", "Show typing")
                        }
                        (true, false) => ("Incognito mode on.", ""),
                        (false, true) => ("Incognito mode off.", ""),
                    };
                    self.status_banner = tr(text).into();
                    let typing = UiMessage::SetPref("typingIndicators", u8::from(!typing));
                    let typing = UiMessage::Commands(vec![UiMessage::DismissBanner, typing]);
                    self.banner_action = (!action.is_empty()).then(|| (self.status_banner.clone(), tr(action), typing));
                }
                if key == "appLanguage" {
                    i18n::set(v);
                }
                let mut st = self.store.write();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::pref_write(&mut st, key).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed);
                }
            }
            UiMessage::ToggleHotkey(name) => self.patch_settings(|s| {
                match s.hotkey_keys(name) {
                    "disabled" if prefs::hotkey_default(name) == "disabled" => {
                        s.keybindings.insert(name.into(), "".into())
                    }
                    "disabled" => s.keybindings.remove(name),
                    _ => s.keybindings.insert(name.into(), "disabled".into()),
                };
            }),
            UiMessage::RevertHotkeys => self.patch_settings(|s| s.keybindings.clear()),
            UiMessage::HotkeyQuery(q) => self.hotkey_query = q,
            UiMessage::HotkeysModal(q) => {
                self.hotkeys_scrolled &= q.is_some();
                let open = self.hotkeys_modal.is_none() && q.is_some();
                self.hotkeys_modal = q;
                if open {
                    return operation::focus("hotkeys-search");
                }
            }
            UiMessage::HotkeysScrolled(down) => self.hotkeys_scrolled = down,
            UiMessage::Zoom(step) => self.patch_settings(|s| {
                s.zoom_percent = if step == 0 {
                    100
                } else {
                    s.zoom_percent.saturating_add_signed(10 * i16::from(step)).clamp(40, 300)
                };
            }),
            UiMessage::ZoomDrag(level) => self.zoom_drag = Some(level),
            UiMessage::ZoomRelease => {
                let level = self.zoom_drag.take().expect("the zoom slider was dragged");
                self.patch_settings(|s| s.zoom_percent = (100.0 + 10.0 * level) as u16);
            }
            UiMessage::HoverEnter(id) => {
                self.quick_open &= self.pane.hovered_msg.as_ref() == Some(&id);
                self.pane.hovered_msg = Some(id);
            }
            UiMessage::ShowQuickReactions => self.quick_open = true,
            UiMessage::HoverChat(id, true) => self.hovered_chat = Some(id),
            UiMessage::HoverChat(id, false) => {
                if self.hovered_chat.as_ref() == Some(&id) {
                    self.hovered_chat = None;
                }
            }
            UiMessage::DeleteDraft(cid) => {
                self.store.write().patch_chat(&cid, true, None, None).expect("chat exists");
                if self.pane.selected_chat_id.as_ref() == Some(&cid) {
                    self.pane.composer = composer_with("");
                    self.pane.reply_to_message_id = None;
                }
            }
            UiMessage::HoverExit(id) => {
                if self.pane.hovered_msg.as_ref() == Some(&id) {
                    self.pane.hovered_msg = None;
                }
            }
            UiMessage::DragStart => self.dragging = true,
            UiMessage::DragMoved(pt) => {
                let rail = if self.store.read().settings.on("showSidebar") { 68.0 } else { 0.0 };
                let width = pt.x - rail - 2.0;
                if width >= 172.0 {
                    self.sidebar_width = width.min(732.0);
                }
                self.set_sidebar_hidden(width < 172.0);
            }
            UiMessage::DragEnd => {
                self.dragging = false;
                let path = self.store.read().data_dir.join("sidebar-width");
                std::fs::write(path, self.sidebar_width.to_string()).expect("write the sidebar width");
            }
            UiMessage::WindowOpened(id) => {
                let key = if Some(id) == self.settings_window {
                    "BEEPER_SCREENSHOT_SETTINGS"
                } else {
                    "BEEPER_SCREENSHOT_EXIT"
                };
                if std::env::var(key).is_ok() {
                    self.shot = Some(id);
                }
            }
            UiMessage::Drawn(id) if self.shot == Some(id) => {
                self.shot = None;
                return window::screenshot(id).map(move |s| UiMessage::ScreenshotCaptured(id, s));
            }
            UiMessage::Drawn(_) => {}
            UiMessage::WindowResized(id, size) => {
                if self.main_window == Some(id) {
                    let narrow = |s: Size| s.width / self.zoom < 600.0;
                    if narrow(self.pane.size) != narrow(size) {
                        self.set_sidebar_hidden(narrow(size));
                    }
                    self.pane.size = size;
                }
                if let Some(w) = self.chat_windows.iter_mut().find(|w| w.0 == id) {
                    w.1.size = size;
                }
                if let Some(w) = self.media_window.as_mut().filter(|w| w.id == id) {
                    w.size = size;
                }
            }
            UiMessage::Tray(Ok(tray::Event::Up)) => {
                self.tray_up = true;
                self.tray_sent = (None, true);
            }
            // Electron cannot pop up the tray menu on Linux, so Desktop's click on a focused window does nothing.
            UiMessage::Tray(Ok(tray::Event::Activate)) if !self.hidden && self.focused => {}
            UiMessage::Tray(Ok(tray::Event::Show | tray::Event::Activate)) => return self.set_hidden(false),
            UiMessage::Tray(Ok(tray::Event::Hide)) => return self.set_hidden(true),
            UiMessage::Tray(Ok(tray::Event::Quit)) => return self.quit(),
            UiMessage::Tray(Err(e)) | UiMessage::TrayFailed(e) => {
                eprintln!("tray icon failed: {e}");
                self.tray_up = false;
                return self.set_hidden(false);
            }
            UiMessage::CloseRequested(id) if self.poll_windows.iter().any(|w| w.id == id) => {
                return self.poll_edit(id, PollEdit::Close);
            }
            UiMessage::CloseRequested(id) => {
                let st = self.store.read();
                let quit = st.settings.on("quitOnClose") || !(self.tray_up && st.settings.on("showTrayIcon"));
                drop(st);
                return match Some(id) == self.main_window && !quit {
                    true => self.set_hidden(true),
                    false => window::close(id),
                };
            }
            UiMessage::InWindow(id, msg) => {
                let Some(i) = self.chat_windows.iter().position(|w| w.0 == id) else { return self.handle(*msg) };
                std::mem::swap(&mut self.pane, &mut self.chat_windows[i].1);
                let task = self.handle(*msg);
                let i = self.chat_windows.iter().position(|w| w.0 == id).expect("handling keeps the window");
                std::mem::swap(&mut self.pane, &mut self.chat_windows[i].1);
                let task = task.map(move |m| UiMessage::InWindow(id, Box::new(m)));
                return match self.chat_windows[i].1.selected_chat_id {
                    Some(_) => task,
                    None => Task::batch([task, window::close(id)]),
                };
            }
            UiMessage::OpenChatWindow(cid) => {
                self.chat_menu = None;
                let open = self.chat_windows.iter().find(|w| w.1.selected_chat_id.as_deref() == Some(&cid));
                if let Some(&(id, _)) = open {
                    return window::gain_focus(id);
                }
                let size = Size::new(800.0, 600.0);
                let (id, open) = window::open(Self::window_settings(size));
                self.chat_windows.push((id, Pane { size, ..Pane::default() }));
                let main = self.main_window.expect("the chat list is in the main window");
                let near = window::position(main).then(move |at| match at {
                    Some(at) => window::move_to(id, at + Vector::new(20.0, 20.0)),
                    None => Task::none(),
                });
                let select = self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(cid))));
                self.save_subwindows();
                return Task::batch([open.discard(), near, select]);
            }
            UiMessage::WindowClosed(id) => {
                if let Some(w) = self.chat_windows.iter().find(|w| w.0 == id) {
                    save_draft(&mut self.store.write(), &w.1);
                }
                self.chat_windows.retain(|w| w.0 != id);
                self.theme_windows.retain(|w| w.0 != id);
                self.poll_windows.retain(|w| w.id != id);
                self.vote_windows.retain(|w| w.0 != id);
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                } else if self.media_window.as_ref().is_some_and(|w| w.id == id) {
                    self.media_window = None;
                    self.carousel = self.carousel.take().filter(|c| !c.in_media);
                } else if [self.main_window, self.login_window].contains(&Some(id)) {
                    return self.quit();
                }
                self.save_subwindows();
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
                    && self.store.read().signed_in() =>
            {
                self.login.screen = login::Screen::Accounts;
            }
            UiMessage::StoreChanged => {
                let st = self.store.read();
                let left = self.pane.selected_chat_id.as_deref().is_some_and(|c| st.resolve_chat_idx(c).is_none());
                self.selection.retain(|c| st.resolve_chat_idx(c).is_some());
                drop(st);
                if left {
                    (self.pane.selected_chat_id, self.chat_menu) = (None, None);
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
            UiMessage::KeyPressed(Key::Named(k @ (Named::Enter | Named::Escape)), m)
                if self.recorder.is_some() && m.is_empty() =>
            {
                return self.record(record::Msg::Key(k));
            }
            UiMessage::KeyPressed(key, mods) if self.carousel.as_ref().is_some_and(|c| !c.in_media) => {
                return self.carousel_key(&key, mods);
            }
            UiMessage::KeyPressed(key, mods) => {
                if self.queued_here()
                    && mods.command()
                    && matches!(key.as_ref(), Key::Character(c) if c.eq_ignore_ascii_case("z"))
                {
                    return self.update(UiMessage::Undo(mods.shift()));
                }
                let n = self.emoji_suggestions(&self.store.read()).len();
                if n > 0 && mods.is_empty() {
                    let at = self.pane.suggest.min(n - 1);
                    match key.as_ref() {
                        Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                            let down = key == Key::Named(Named::ArrowDown);
                            self.pane.suggest = if down { (at + 1) % n } else { (at + n - 1) % n };
                            return Task::none();
                        }
                        Key::Named(Named::Tab) => return self.update(UiMessage::PickEmoji(at)),
                        Key::Named(Named::Escape) => {
                            self.pane.suggest_hidden = Some(self.pane.composer.text());
                            return Task::none();
                        }
                        _ => {}
                    }
                }
                if self.show_command_palette && self.cmd_bar_commands_mode && mods.is_empty() {
                    let cmds = self.commands(&self.store.read(), self.command_palette_query.trim());
                    let n = cmds.len().max(1);
                    let step = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(1),
                        Key::Named(Named::ArrowUp) => Some(n - 1),
                        Key::Named(Named::Enter) => {
                            let Some(m) = cmds.into_iter().nth(self.cmd_index).and_then(|c| c.msg) else {
                                return Task::none();
                            };
                            return self.update(UiMessage::Commands(vec![UiMessage::CloseModals, m]));
                        }
                        _ => None,
                    };
                    if let Some(by) = step {
                        self.cmd_index = (self.cmd_index + by) % n;
                        return locate(format!("cmd-{}", self.cmd_index), commands::COMMANDS, true);
                    }
                }
                match key.as_ref() {
                    Key::Named(Named::Escape) => return self.update(UiMessage::CloseModals),
                    Key::Named(Named::ArrowUp | Named::ArrowDown)
                        if (mods.is_empty() || mods == keyboard::Modifiers::SHIFT) && self.can_select_messages() =>
                    {
                        let down = key == Key::Named(Named::ArrowDown);
                        let edit = !down
                            && self.pane.selected_msgs.is_empty()
                            && self.store.read().settings.pref("upArrow") == 1;
                        let step = self.update(UiMessage::StepMessage(down, mods.shift()));
                        let Some(m) = edit.then(|| self.last_own_message()).flatten() else { return step };
                        self.pane.selected_msgs = vec![m.id.clone()];
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
                    Key::Named(Named::ArrowRight)
                        if mods.is_empty() && self.single_msg().is_some() && self.can_select_messages() =>
                    {
                        return self.update(UiMessage::React(self.single_msg().cloned().expect("a selected message")));
                    }
                    Key::Character("c") if mods.command() && !mods.shift() && !self.pane.selected_msgs.is_empty() => {
                        let stamp = self.store.read().settings.on("copyTimestamps");
                        return self.update(UiMessage::CopySelected(stamp));
                    }
                    Key::Character("a")
                        if mods.command() && !mods.shift() && !mods.alt() && self.can_select_messages() =>
                    {
                        let st = self.store.read();
                        let (msgs, live) = self.timeline(&st);
                        let all = msgs.iter().zip(live).filter(|(_, l)| *l).map(|(m, _)| m.id.clone()).collect();
                        drop(st);
                        self.pane.selected_msgs = all;
                        return Task::none();
                    }
                    Key::Character("a")
                        if mods.command()
                            && !mods.shift()
                            && !mods.alt()
                            && self.pane.selected_chat_id.is_none()
                            && !self.show_command_palette
                            && self.active_tab != InboxTab::Requests =>
                    {
                        self.selection = self.list_ids(&self.store.read());
                        if let Some(first) = self.selection.first().cloned() {
                            self.select_chat(&first);
                        }
                        return Task::none();
                    }
                    Key::Character("v") if mods.command() && self.pane.selected_chat_id.is_some() => {
                        return Task::perform(clipboard_image(), UiMessage::Pasted);
                    }
                    _ => {}
                }
                if let Some((h, digit)) = self.find_hotkey(&key, mods) {
                    return self.run_hotkey(h, digit);
                }
            }
        }
        if let Some(e) = self.store.write().matrix_error.take() {
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
        let st = self.store.read();
        let client = st.matrix.clone();
        let images = self
            .pane
            .selected_chat_id
            .as_ref()
            .and_then(|chat| st.chat_msg_indices.get(chat))
            .into_iter()
            .flatten()
            .flat_map(|&i| &st.messages[i].attachments)
            .filter(|a| matches!(a.attachment_type.as_str(), "img" | "video"))
            .filter_map(|a| a.poster_img.clone().or_else(|| a.src_url.clone()));
        let gif_urls = (self.pane.picker == Some(Picker::Gif))
            .then(|| self.gifs.iter().map(|g| g.preview_url.clone()))
            .into_iter()
            .flatten();
        let hits = self.media_hits(&st);
        let found =
            hits.iter().flat_map(|m| &m.attachments).filter(|a| matches!(a.attachment_type.as_str(), "img" | "video"));
        let found = found.filter_map(|a| a.poster_img.clone().or_else(|| a.src_url.clone()));
        let tiles = self.media_window.iter().filter(|w| w.tab == 0);
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
        let loadable = |u: &String| !self.media.contains_key(u) && (client.is_some() || u.starts_with("https://"));
        let urls: HashSet<String> = images.chain(gif_urls).chain(found).chain(viewer).filter(loadable).collect();
        drop(st);
        self.media.extend(urls.iter().map(|u| (u.clone(), None)));
        let audio = self.load_audio();
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (client.clone(), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the media semaphore stays open");
                    let bytes = beeper_core::matrix::media(client, u).await?;
                    tokio::task::spawn_blocking(move || decode_media(bytes)).await.expect("media decode task")
                },
                move |r| UiMessage::MediaLoaded(url, r),
            )
        }))
        .chain(audio)
    }

    fn patch_settings(&mut self, f: impl FnOnce(&mut BeeperSettings)) {
        {
            let mut st = self.store.write();
            f(&mut st.settings);
            st.persist_state_to_sqlite();
            let on = st.settings.enable_pas_api;
            st.api_on.send_if_modified(|v| std::mem::replace(v, on) != on);
        }
        self.refresh_theme_and_tray();
    }

    pub fn subscription(&self) -> Subscription<UiMessage> {
        let changes = Subscription::run_with(StoreFeed(self.store.clone(), 0), |f| {
            feed(f.0.read().ws_tx.subscribe(), |e| match (e.event_type.as_str(), e.message, e.chat) {
                ("message.upserted", Some(m), _) => UiMessage::Upserted(Box::new(m)),
                ("chat.upserted", _, Some(c)) => UiMessage::ChatUpserted(Box::new(c)),
                ("chat.typing", ..) => UiMessage::Typing(e.chat_id.expect("typing events name the chat")),
                _ => UiMessage::StoreChanged,
            })
        });
        let focus = Subscription::run_with(StoreFeed(self.store.clone(), 1), |f| {
            feed(f.0.read().focus_tx.subscribe(), UiMessage::FocusRequested)
        });
        let tokens = Subscription::run_with(StoreFeed(self.store.clone(), 2), |f| {
            feed(f.0.read().connect_tx.subscribe(), |(id, outcome)| UiMessage::TokenRequest(id, outcome))
        });
        let keys = iced::event::listen_with(|e, status, id| match e {
            Event::Window(window::Event::Closed) => Some(UiMessage::WindowClosed(id)),
            Event::Window(window::Event::CloseRequested) => Some(UiMessage::CloseRequested(id)),
            Event::Window(window::Event::Resized(size)) => Some(UiMessage::WindowResized(id, size)),
            Event::Window(window::Event::Focused) => Some(UiMessage::WindowFocused(true)),
            Event::Window(window::Event::Unfocused) => Some(UiMessage::WindowFocused(false)),
            Event::Window(window::Event::FileHovered(_)) => Some(UiMessage::FileHover(id, true)),
            Event::Window(window::Event::FilesHoveredLeft) => Some(UiMessage::FileHover(id, false)),
            Event::Window(window::Event::FileDropped(path)) => Some(UiMessage::FileDropped(id, path)),
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(UiMessage::Modifiers(m)),
            Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::Escape), modifiers, .. })
                if status == iced::event::Status::Captured && modifiers.is_empty() =>
            {
                Some(UiMessage::InputEscape(id))
            }
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
        let poll_drag = self.poll_windows.iter().any(|d| d.dragging.is_some()).then(|| {
            iced::event::listen_with(|e, _, id| match e {
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Poll(id, PollEdit::Grab(None))),
                _ => None,
            })
        });
        let (rising, due) = {
            let st = self.store.read();
            let last = self.pane().selected_chat_id.as_ref().and_then(|c| st.chat_msg_indices.get(c)?.last());
            let rising = last.is_some_and(|&i| send_rise(&st, &st.messages[i]).is_some());
            let pending = st.chats.iter().filter(|c| !self.expired(c));
            let send = st.scheduled.iter().filter(|s| !s.canceled).map(|s| s.send_on);
            let send = send.chain(self.pane.send_later.map(|c| c.0)).min();
            (rising, (pending.filter_map(|c| Some(c.reminder.as_ref()?.remind_at_ms)).min(), send))
        };
        let wait = |at: i64| std::time::Duration::from_millis((at - now_unix_millis()).max(1) as u64);
        let remind = due.0.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::CheckReminders));
        let send = due.1.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::CheckScheduled));
        let queued = self.queued.iter().map(|q| q.due).min();
        let queued = queued.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::SendQueued(None)));
        let animated = rising
            || self.flash.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 1.5)
            || self.pref_flash.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 2.0);
        let tray = (cfg!(target_os = "linux") && self.store.read().settings.on("showTrayIcon"))
            .then(|| Subscription::run(tray::events).map(UiMessage::Tray));
        let notes = (!self.notifications.is_empty())
            .then(|| Subscription::run(notify::events).map(|(id, clicked)| UiMessage::NotificationEvent(id, clicked)));
        let tick =
            animated.then(|| iced::time::every(std::time::Duration::from_millis(20)).map(|_| UiMessage::AnimationTick));
        let shot = self.shot.map(|_| {
            iced::event::listen_raw(|e, _, id| {
                matches!(e, Event::Window(window::Event::RedrawRequested(_))).then_some(UiMessage::Drawn(id))
            })
        });
        Subscription::batch(
            [changes, focus, tokens, keys, system]
                .into_iter()
                .chain(drag)
                .chain(poll_drag)
                .chain(carousel)
                .chain(tick)
                .chain(shot)
                .chain(remind)
                .chain(send)
                .chain(queued)
                .chain(notes)
                .chain(tray)
                .chain(self.audio_ticks())
                .chain(self.record_events())
                .chain(self.video_ticks()),
        )
    }

    pub fn view(&self, id: window::Id) -> El<'_> {
        let arrow = !self.store.read().settings.on("pointerCursor");
        self.drawing.set(self.chat_windows.iter().any(|w| w.0 == id).then_some(id));
        let el = cursor::root(self.view_window(id), arrow, &self.cursor);
        self.drawing.set(None);
        match self.chat_windows.iter().any(|w| w.0 == id) {
            true => Element::from(el).map(move |m| UiMessage::InWindow(id, Box::new(m))),
            false => el.into(),
        }
    }

    fn view_window(&self, id: window::Id) -> El<'_> {
        if Some(id) == self.settings_window {
            self.view_settings()
        } else if let Some(w) = self.media_window.as_ref().filter(|w| w.id == id) {
            self.view_media(w)
        } else if Some(id) == self.login_window {
            self.view_login()
        } else if let Some((_, chat)) = self.theme_windows.iter().find(|w| w.0 == id) {
            self.view_chat_theme(chat)
        } else if let Some(d) = self.poll_windows.iter().find(|w| w.id == id) {
            self.view_poll_creator(d)
        } else if let Some((_, chat, mid)) = self.vote_windows.iter().find(|w| w.0 == id) {
            self.view_poll_votes(chat, mid)
        } else {
            self.view_main()
        }
    }

    /// The rail, the chat list and the open chat.
    fn view_body(&self, open: bool) -> El<'_> {
        let p = &self.pal;
        let grip = mouse_area(container(Space::new()).width(5).height(Fill))
            .on_press(UiMessage::DragStart)
            .interaction(mouse::Interaction::ResizingHorizontally);
        let pane = match self.creating_chat {
            Some(_) => self.view_new_chat(),
            None if open => self.view_conversation(),
            None => container(Space::new()).width(Fill).height(Fill).style(fill_box(p.card, 10.0)).into(),
        };
        let info = (self.show_right_panel && open).then(|| self.view_info());
        let pad = Padding::new(8.0).left(if self.sidebar_hidden { 8.0 } else { 0.0 });
        let right: El<'_> = if self.selection.is_empty() {
            row![container(pane).padding(pad).width(Fill)].push(info).into()
        } else {
            let content = container(row![pane].push(info)).padding(8).style(fill_box(p.selected, 18.0));
            column![self.view_selection_head(), content, self.view_selection_bar()].spacing(8).padding(pad).into()
        };
        let rail = self.store.read().settings.on("showSidebar");
        let rail = rail.then(|| self.view_rail()).filter(|_| !self.sidebar_hidden);
        let sidebar = (!self.sidebar_hidden).then(|| self.view_sidebar());
        let body = Row::new().push(rail).push(sidebar).push(stack![right, grip]);
        container(body).width(Fill).height(Fill).style(fill_box(p.window, 0.0)).into()
    }

    pub fn view_main(&self) -> El<'_> {
        let st = self.store.read();
        // The sync task can drop a chat before StoreChanged reaches update.
        let known = |c: &str| c.starts_with("draft:") || c.starts_with("account:") || st.resolve_chat_idx(c).is_some();
        let shown = self.pane().selected_chat_id.as_deref().is_some_and(known);
        let open = self.creating_chat.is_none() && shown;
        let menu_open = self.chat_menu.as_ref().is_some_and(|(c, _)| known(c));
        drop(st);
        let in_main = self.drawing.get().is_none();
        let base = match in_main {
            true => self.view_full_video().unwrap_or_else(|| self.view_body(open)),
            false => container(match shown {
                true => self.view_conversation(),
                false => Space::new().into(),
            })
            .padding(8)
            .width(Fill)
            .height(Fill)
            .style(fill_box(self.pal.window, 0.0))
            .into(),
        };
        let modal = match (&self.quick_look_target, self.show_command_palette) {
            (Some((title, detail)), _) => Some(self.view_quick_look(title, detail)),
            (None, true) => Some(self.view_command_palette()),
            (None, false) => self.forward.as_ref().map(|f| self.view_forward(f)),
        };
        let scrim = modal.filter(|_| in_main).map(|m| {
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
        let filter_menu = self.show_filter_menu.then(|| (self.view_filter_menu(), self.sidebar_width - 22.5, 26.0));
        let reminder_menu = self.reminder_menu.map(|at| (self.view_reminder_menu(), at.x, at.y));
        let rate_menu = self.audio.menu.map(|at| (self.view_rate_menu(), at.x, at.y));
        let chat_menu = self.chat_menu.as_ref().filter(|_| menu_open).map(|(cid, at)| {
            let (w, h) = if cid.starts_with("draft:") {
                (124.0, 31.0)
            } else if cid.starts_with("account:") {
                (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + 3.0 * MENU_ITEM)
            } else {
                (
                    233.0,
                    8.0 + 10.0 * MENU_ITEM
                        + 44.0
                        + if self.selection.is_empty() { 2.0 * MENU_ITEM + 11.0 } else { 0.0 },
                )
            };
            let x = if at.x + w > self.pane().size.width { at.x - w } else { at.x };
            let sub_w =
                self.chat_sub.filter(|_| !cid.starts_with("account:")).map_or(0.0, |i| SUB_MENU_W[usize::from(i)]);
            let left = x + w + sub_w > self.pane().size.width;
            let y = at.y.min(self.pane().size.height - h - 8.0);
            (self.view_chat_menu(cid, left, self.pane().size.height - 8.0 - y), if left { x - sub_w } else { x }, y)
        });
        let message_menu = self.pane().message_menu.as_ref().map(|(mid, at)| {
            let (menu, h) = self.view_message_menu(mid);
            let x = if at.x + MESSAGE_MENU_W > self.pane().size.width { at.x - MESSAGE_MENU_W } else { at.x };
            let sub = self.view_speed_menu(mid);
            let left = sub.is_some() && x + MESSAGE_MENU_W + video::SPEED_MENU_W > self.pane().size.width;
            let (x, menu) = match left {
                true => (x - video::SPEED_MENU_W, row![].push(sub).push(menu).into()),
                false => (x, row![menu].push(sub).into()),
            };
            (menu, x, at.y.min(self.pane().size.height - h - 8.0))
        });
        let edit_menu = self.pane().edit_menu.as_ref().map(|(at, word)| {
            let (menu, h) = self.view_edit_menu(word.as_ref());
            let x = if at.x + MESSAGE_MENU_W > self.pane().size.width { at.x - MESSAGE_MENU_W } else { at.x };
            (menu, x, at.y.min(self.pane().size.height - h - 8.0))
        });
        let edit_menu = edit_menu.or_else(|| {
            let (ids, items, at) = self.pane().sched_menu.as_ref()?;
            let (menu, h) = self.view_scheduled_menu(ids, *items);
            let x = if at.x + MESSAGE_MENU_W > self.pane().size.width { at.x - MESSAGE_MENU_W } else { at.x };
            Some((menu, x, at.y.min(self.pane().size.height - h - 8.0)))
        });
        let react = self.pane().emoji.as_ref().and_then(|pk| pk.react.as_ref()).map(|(_, at)| {
            let x = if at.x + 352.0 > self.pane().size.width { at.x - 352.0 } else { at.x };
            (
                self.view_emoji_picker(&self.store.read()).expect("the picker is open"),
                x,
                at.y.min(self.pane().size.height - 435.0 - 8.0),
            )
        });
        let selection_menu = self.selection_menu.map(|(i, x)| {
            let (menu, h) = self.view_selection_menu(i);
            let w = if i == 0 { SUB_MENU_W[1] } else { 233.0 };
            (menu, x.min(self.pane().size.width - w - 8.0), self.pane().size.height - 60.0 - h)
        });
        let main_menu = filter_menu.or(reminder_menu).or(rate_menu).or(chat_menu).filter(|_| in_main);
        let menu = main_menu.or(message_menu).or(edit_menu).or(react).or(selection_menu.filter(|_| in_main)).map(
            |(m, x, y)| {
                mouse_area(
                    container(mouse_area(m).on_press(UiMessage::Noop))
                        .padding(Padding::ZERO.left(x).top(y))
                        .width(Fill)
                        .height(Fill),
                )
                .on_press(UiMessage::CloseModals)
                .on_right_press(UiMessage::CloseModals)
            },
        );
        let toast = self.view_toast(76.0);
        let confirm = self.confirm.filter(|d| !SETTINGS_DIALOGS.contains(d)).map(|d| self.view_confirm(d));
        let shows = |c: &&Carousel| {
            let st = self.store.read();
            !c.in_media && Some(&st.messages[st.msg_idx_by_id[&c.mid]].chat_id) == self.pane().selected_chat_id.as_ref()
        };
        let carousel = self.carousel.as_ref().filter(shows).map(|c| self.view_carousel(c));
        let hotkeys = self.hotkeys_modal.as_deref().filter(|_| in_main).map(|q| self.view_hotkeys_modal(q));
        let layers = stack![base].push(toast).push(menu).push(scrim).push(select).push(confirm);
        let token = self.token_ask.as_ref().map(|a| self.view_token_ask(a));
        layers.push(carousel).push(hotkeys).push(token).into()
    }

    /// Desktop's `BeeperConnectAccessTokenRequestModal`, over the main window.
    fn view_token_ask<'a>(&'a self, a: &'a TokenAsk) -> El<'a> {
        let p = &self.pal;
        let sentence = tr("A third-party app <app/> wants to access your chats through Beeper Desktop API.");
        let (before, after) = sentence.split_once("<app/>").expect("the sentence names the app");
        let main =
            rich_text![span::<(), _>(before), span(a.app.as_str()).font(fw(Weight::Bold)).color(p.strong), span(after)]
                .size(15.0)
                .line_height(Pixels(20.0))
                .color(p.text)
                .align_x(Center)
                .width(Fill);
        let button = |label: &'static str, primary: bool, msg: UiMessage| -> El<'a> {
            let (bg, fg) = if primary { (p.accent, Color::WHITE) } else { (p.button, p.strong) };
            let label = txt(tr(label), 13.0).font(fw(Weight::Medium)).line_height(Pixels(16.0)).color(fg);
            button(container(label).center_y(Fill))
                .height(32)
                .padding([0, 12])
                .style(bordered_btn([bg; 3], [fg; 3], if primary { bg } else { p.button_border }, 1.0, 6.0))
                .on_press(msg)
                .into()
        };
        let body: El<'a> = match a.timed_out {
            true => {
                let notice = column![
                    semi(tr("The request has timed out."), 15.0).line_height(Pixels(20.0)).color(p.strong),
                    txt(tr("To try again you will need to relaunch the third-party app or re-request login in its MCP settings."), 13.0)
                        .line_height(Pixels(16.0))
                        .color(p.text)
                ]
                .spacing(8);
                let notice = container(notice).width(Fill).padding([8, 16]).style(fill_box(p.chip, 8.0));
                column![notice, row![button("Close", false, UiMessage::TokenClose)]].spacing(16).into()
            }
            false => {
                let label = |t: &'static str| txt(tr(t), 12.0).line_height(Pixels(15.0)).color(p.text);
                let sensitive = column![
                    row![label("Allow sensitive actions").width(Fill), toggle(p, a.write, Some(UiMessage::TokenWrite))]
                        .align_y(Center)
                        .height(38)
                        .padding([0, 8]),
                    container(
                        txt(tr("Grant permission to send messages and modify your account"), 11.0)
                            .line_height(Pixels(14.0))
                            .color(p.sub)
                    )
                    .padding(Padding::new(8.0).top(0.0))
                ];
                let expiry = select(p, EXPIRIES, a.expiry, UiMessage::TokenExpiry);
                let expiry = row![label("Expires in").width(Fill), expiry].align_y(Center).height(38).padding([0, 8]);
                let rows = column![sensitive, divider(p.prefs_border, 0.0), expiry, divider(p.prefs_border, 0.0)];
                let group =
                    container(rows).width(Fill).padding(1).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
                let buttons = row![
                    button("Decline", false, UiMessage::TokenAnswer(false)),
                    button("Approve", true, UiMessage::TokenAnswer(true))
                ]
                .spacing(8);
                let fine = txt(tr("Only approve if you initiated this connection and trust the application."), 12.0)
                    .line_height(Pixels(15.0))
                    .color(p.sub)
                    .align_x(Center)
                    .width(Fill);
                column![
                    container(group).padding([8, 0]),
                    container(buttons).padding(Padding::new(0.0).top(16.0).bottom(8.0)),
                    fine
                ]
                .into()
            }
        };
        let title = container(semi(tr("Approve connection"), 13.0).line_height(Pixels(16.0)).color(p.strong))
            .center_x(Fill)
            .padding([8, 0]);
        let content = column![container(main).padding(Padding::new(0.0).top(32.0).bottom(16.0)), body];
        let pane = column![title, container(content).padding([0, 16])];
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.prefs_pane, 10.0));
        mouse_area(container(pane).padding(8)).on_press(UiMessage::Noop).into()
    }

    /// The selected chats that still exist: the sync task can drop one before StoreChanged reaches update.
    fn selected_chats<'s>(&self, st: &'s BeeperStore) -> Vec<&'s Chat> {
        self.selection.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect()
    }

    fn view_selection_head(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let chats = self.selected_chats(&st);
        let n = chats.len();
        let step = 28.0 / 3.0;
        let avatars = chats.iter().enumerate().fold(Stack::new(), |s, (i, c)| {
            s.push(pin(container(self.chat_avatar(c, 28.0, 0.0)).style(fill_box(p.card, 14.0))).x(i as f32 * step))
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
        let st = self.store.read();
        let chats = self.selected_chats(&st);
        let all = |f: ChatFlag| chats.iter().all(|c| has_flag(c, f));
        let x0 = self.left_width(&st) + 8.0;
        let cell = (self.pane().size.width - x0 - 16.0) / 6.0;
        let toggles = [
            (
                ChatFlag::Archived,
                [trx("Archive", "button label to archive selected chats"), tr("Unarchive")],
                [trx("Archive", "button label to archive selected chats"), tr("Unarchive")],
                [&SELECTION_ICONS[0], &SELECTION_ICONS[1]],
                vec!["Ctrl", "E"],
            ),
            (
                ChatFlag::MarkedUnread,
                [tr("Mark Unread"), tr("Mark Read")],
                [tr("Mark as Unread"), tr("Mark as Read")],
                [&ROW_ACTIONS[3], &ROW_ACTIONS[2]],
                vec!["Ctrl", "Shift", "U"],
            ),
            (
                ChatFlag::Muted,
                [tr("Mute"), tr("Unmute")],
                [tr("Mute"), tr("Unmute")],
                [&SELECTION_ICONS[2], &SELECTION_ICONS[3]],
                vec!["Ctrl", "Shift", "M"],
            ),
            (
                ChatFlag::LowPriority,
                [tr("Low Priority"), tr("Move to Inbox")],
                [tr("Move to Low Priority"), tr("Move to Inbox")],
                [&SELECTION_ICONS[4], &SELECTION_ICONS[5]],
                vec![],
            ),
        ]
        .map(|(f, labels, tips, glyphs, keys)| {
            let on = usize::from(all(f));
            (labels[on], tips[on], glyphs[on], UiMessage::SelectionSet(f, on == 0), keys)
        });
        let menus = [(tr("Remind Me"), &SELECTION_ICONS[6]), (tr("More"), &SELECTION_ICONS[7])]
            .into_iter()
            .zip(0u8..)
            .map(|((label, glyph), i)| {
                let x = x0 + cell * (4.5 + f32::from(i));
                (label, label, glyph, UiMessage::SelectionMenu(i, x), vec![])
            });
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
        let st = self.store.read();
        let chats = self.selected_chats(&st);
        let item = |label: String, at: String, msg| menu_item(p, None, label, at, msg);
        let items: Vec<El> = if which == 0 {
            let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
            let last = last_used(&st, now).map(|at| {
                item(tr("Last Used").into(), last_used_label(at, h24), Some(UiMessage::SelectionRemind(Some(at))))
            });
            let presets = remind_presets(now).map(|(label, at)| {
                let msg =
                    UiMessage::Commands(vec![UiMessage::LastTime(label.into()), UiMessage::SelectionRemind(Some(at))]);
                item(tr(label).into(), remind_label(at, h24), Some(msg))
            });
            let presets = last.into_iter().chain(presets);
            let set = chats.iter().filter(|c| c.reminder.is_some()).count();
            let dismiss = match set {
                0 => vec![],
                1 => vec![
                    menu_rule(p),
                    item(tr("Dismiss Reminder").into(), String::new(), Some(UiMessage::SelectionRemind(None))),
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
            let entry = |label: &'static str, keys: &str, f: ChatFlag, on: bool| {
                item(tr(label).into(), keys.into(), Some(UiMessage::SelectionSet(f, on)))
            };
            let pinned = count(ChatFlag::Pinned);
            [Some(item(trn("%d chat selected", "%d chats selected", n, &[&n]), String::new(), None))]
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
        let rules = if which == 0 && chats.iter().any(|c| c.reminder.is_some()) { 11.0 - MENU_ITEM } else { 0.0 };
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
        let st = self.store.read();
        let item = |checked: bool, glyph: char, label: String, hotkey: String, bit: u8| -> El<'_> {
            let lead = row![
                container(checked.then(|| ic(icon::CHECK, 14.0))).center_x(14),
                container(ic(glyph, 14.0)).padding(Padding::ZERO.left(9.0).right(4.0)),
            ];
            menu_item(p, Some(lead.into()), label, hotkey, Some(UiMessage::PickFilter(bit)))
        };
        let counted = (0..F_ARCHIVED).map(|i| {
            let n = self.chats_with(&st, 1 << i).len();
            let hotkey = prefs::hotkey_label(st.settings.hotkey_keys(FILTER_HOTKEYS[i]));
            item(self.filters & 1 << i != 0, FILTERS[i].1, format!("{} ({n})", tr(FILTERS[i].0)), hotkey, 1 << i)
        });
        let views = (F_ARCHIVED..FILTERS.len())
            .map(|i| item(self.filters & 1 << i != 0, FILTERS[i].1, tr(FILTERS[i].0).into(), String::new(), 1 << i));
        let col = Column::with_children(
            std::iter::once(item(
                self.filters == 0,
                icon::INBOX,
                tr("All").into(),
                prefs::hotkey_label(st.settings.hotkey_keys("Filter: All")),
                0,
            ))
            .chain(counted)
            .chain(std::iter::once(menu_rule(p)))
            .chain(views),
        );
        container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    fn view_reminder_menu(&self) -> El<'_> {
        let p = &self.pal;
        let order = self.store.read().settings.reminders_order;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let orders = REMINDER_ORDERS
            .iter()
            .zip(0..)
            .map(|(label, i)| menu_item(p, lead(order == i), tr(label).into(), "", Some(UiMessage::OrderReminders(i))));
        let ask = Some(UiMessage::Ask(Some(ASK_DISMISS_REMINDERS)));
        let col = Column::with_children(
            orders.chain([menu_rule(p), menu_item(p, lead(false), tr("Dismiss All Reminders").into(), "", ask)]),
        );
        container(col.padding([4, 0])).width(291).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    /// Desktop's message context menu, with its height.
    /// Electron's context menu of the composer: the suggestions for a misspelled word, "Learn Spelling", "Search with
    /// Google" for the selection, and Cut, Copy and Paste.
    fn view_edit_menu(&self, word: Option<&Typo>) -> (El<'_>, f32) {
        let p = &self.pal;
        let text = self.pane().composer.text();
        let sel = format::selection(&self.pane().composer).map(|r| text[r].trim().to_owned()).filter(|s| !s.is_empty());
        let item = |label: &str, msg: Option<UiMessage>| Some(menu_item(p, None, label.into(), "", msg));
        let edit = |e: MenuEdit| Some(UiMessage::MenuEdit(e));
        let mut items: Vec<Option<El<'_>>> = Vec::new();
        if let Some((_, fixes)) = word {
            items.extend(fixes.iter().map(|f| item(f, edit(MenuEdit::Replace(f.clone())))));
            items.extend(fixes.is_empty().then(|| item("No Guesses Found", None)));
            items.extend([Some(menu_rule(p)), item("Learn Spelling", edit(MenuEdit::Learn)), Some(menu_rule(p))]);
        }
        if let Some(q) = &sel {
            let q: String = q
                .bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => char::from(b).to_string(),
                    b' ' => "+".into(),
                    b => format!("%{b:02X}"),
                })
                .collect();
            let google = UiMessage::OpenUrl(format!("https://www.google.com/search?q={q}"));
            items.extend([item("Search with Google", Some(google)), Some(menu_rule(p))]);
        }
        let has = sel.is_some();
        items.extend([
            item("Cut", has.then(|| UiMessage::MenuEdit(MenuEdit::Cut))),
            item("Copy", has.then(|| UiMessage::MenuEdit(MenuEdit::Copy))),
            item("Paste", edit(MenuEdit::Paste)),
        ]);
        let rules = word.map_or(0, |_| 2) + usize::from(has);
        let h = 8.0 + (items.len() - rules) as f32 * MENU_ITEM + rules as f32 * 11.0;
        let menu = container(Column::with_children(items.into_iter().flatten()).padding([4, 0]))
            .width(MESSAGE_MENU_W)
            .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        (menu.into(), h)
    }

    /// Desktop's scheduled message menu: with `items`, Edit and Copy, then `Mhe`.
    fn view_scheduled_menu(&self, ids: &[String], items: bool) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let first = st.scheduled.iter().find(|s| s.id == ids[0]).expect("a scheduled message");
        let (canceled, n, ids) = (first.canceled, ids.len(), ids.to_vec());
        let item = |label: String, hotkey, msg: UiMessage| Some(menu_item(p, None, label, hotkey, Some(msg)));
        let ctx = "scheduled message menu action";
        let rows = [
            items.then(|| item(trx("Edit...", ctx).into(), "Ctrl T", UiMessage::EditScheduled(ids[0].clone()))),
            items.then(|| item(trx("Copy", ctx).into(), "Ctrl C", UiMessage::CopyText(first.text.clone()))),
            items.then(|| Some(menu_rule(p))),
            (!canceled).then(|| match n {
                1 => item(tr("Send Message Now").into(), "", UiMessage::SendNow(ids.clone())),
                _ => item(
                    trn("Send %d Message Now", "Send %d Messages Now", n, &[&n]),
                    "",
                    UiMessage::SendNow(ids.clone()),
                ),
            }),
            Some(item(tr("Reschedule").into(), "", UiMessage::Reschedule(ids.clone()))),
            Some(item(
                match (canceled, n) {
                    (true, _) => trn("Remove Unsent Message", "Remove Unsent Messages", n, &[]),
                    (false, 1) => tr("Delete Without Sending").into(),
                    (false, _) => trn("Delete %d Message", "Delete %d Messages", n, &[&n]),
                },
                "",
                UiMessage::Unschedule(ids),
            )),
        ];
        let rows: Vec<El> = rows.into_iter().flatten().flatten().collect();
        let h = 8.0 + (rows.len() - usize::from(items)) as f32 * MENU_ITEM + if items { 11.0 } else { 0.0 };
        let menu = container(Column::with_children(rows).padding([4, 0])).width(MESSAGE_MENU_W).style(shadow_box(
            p.menu,
            p.menu_border,
            6.0,
            16.0,
        ));
        (menu.into(), h)
    }

    /// Desktop's `ScheduledMessages`: the canceled messages, the "Send Later" toggle, then the messages to send.
    /// Desktop's `Poll` message, for a chat that can vote.
    fn view_poll<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        m: &Message,
        poll: &PollData,
        (bg, fg): (Color, Color),
    ) -> El<'a> {
        let p = &self.pal;
        let mine = m.is_sender == Some(true);
        let (sub, footer, fill, fill_mine, ring) = match mine {
            true => (fg.scale_alpha(0.6), fg, fg.scale_alpha(0.22), fg.scale_alpha(0.38), fg.scale_alpha(0.85)),
            false => (p.on_secondary, p.accent, fg.scale_alpha(0.14), fg.scale_alpha(0.24), fg.scale_alpha(0.4)),
        };
        let total = poll.voters();
        let picks = poll.picks(&st.me().id);
        let shown = !poll.undisclosed || poll.ended;
        let multi = poll.max_selections != 1;
        let full = multi && picks.len() >= poll.max_selections as usize;
        let spread = if shown { total > 0 } else { !picks.is_empty() };
        let label_alpha = if poll.ended { 0.55 } else { 1.0 };
        let votes = trn("%d vote", "%d votes", total, &[&total]);
        let status = match (poll.ended, multi) {
            (true, _) => tr("Poll ended"),
            (_, true) => tr("Select one or more options"),
            _ => tr("Select one option"),
        };
        let question = if poll.question.is_empty() { tr("Poll").into() } else { poll.question.clone() };
        let head = column![
            semi(question, 15.0).line_height(Pixels(20.0)).color(fg),
            row![
                tinted(if poll.ended { &POLL_ENDED } else { &COMPOSER_ICONS[8] }, 16.0, sub),
                txt(status, 13.0).line_height(Pixels(16.0)).color(sub)
            ]
            .spacing(6)
            .align_y(Center),
        ]
        .spacing(4);
        let options = poll.options.iter().map(|o| -> El<'a> {
            let n = o.voter_ids.len();
            let voted = picks.contains(&o.id);
            let width = match (shown, if shown { n > 0 } else { voted }) {
                (_, false) => 0,
                (true, true) => u16::try_from(n * 100 / total).expect("a percentage"),
                (false, true) => 100,
            };
            let bar = container(Space::new()).height(Fill).style(fill_box(if voted { fill_mine } else { fill }, 16.0));
            let bar: El<'a> = match width {
                0 => Space::new().into(),
                100 => bar.width(Fill).into(),
                w => row![bar.width(Length::FillPortion(w)), Space::new().width(Length::FillPortion(100 - w))].into(),
            };
            let (lf, ring) = (fg.scale_alpha(label_alpha), ring.scale_alpha(label_alpha));
            let radius = if multi { 4.0 } else { 8.0 };
            let check: El<'a> = match voted {
                true => container(tinted(&POLL_CHECK, 16.0, bg)).style(fill_box(lf, radius)).into(),
                false => {
                    container(Space::new().width(16).height(16)).style(bordered_box(CLEAR, ring, 1.5, radius)).into()
                }
            };
            let name = txt(o.label.clone(), 13.0).line_height(Pixels(16.0)).color(lf);
            let faces = o.voter_ids.iter().rev().take(3).rev().enumerate().fold(Stack::new(), |s, (i, id)| {
                let u = chat.participants.items.iter().find(|u| u.id == *id);
                let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(id);
                let face = container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                    .padding(1.5)
                    .style(fill_box(bg, 11.5));
                s.push(container(face).padding(Padding::ZERO.left(14.0 * i as f32)))
            });
            let right = (shown && n > 0).then(|| {
                row![faces, semi(n.to_string(), 13.0).line_height(Pixels(16.0)).color(fg)].spacing(6).align_y(Center)
            });
            let line =
                row![row![check, name].spacing(8).align_y(Center).width(Fill)].push(right).spacing(10).align_y(Center);
            let answers = match (multi, voted) {
                (true, true) => picks.iter().filter(|x| **x != o.id).cloned().collect(),
                (true, false) => picks.iter().cloned().chain([o.id.clone()]).collect(),
                (false, true) => Vec::new(),
                (false, false) => vec![o.id.clone()],
            };
            let vote = (!poll.ended && !(full && !voted)).then(|| UiMessage::VotePoll(m.id.clone(), answers));
            let body = container(line).padding([8.0, if spread { 8.0 } else { 4.0 }]);
            button(stack![body].push_under(bar))
                .padding(0)
                .width(Fill)
                .style(btn_style([CLEAR; 3], [fg; 3], 16.0))
                .on_press_maybe(vote)
                .into()
        });
        let options = Column::with_children(options).spacing(if spread { 6 } else { 2 });
        let foot: El<'a> = match shown {
            true if total > 0 => button(
                row![semi(votes, 13.0).line_height(Pixels(16.0)), tinted(&CHEVRON_RIGHT, 16.0, footer)].align_y(Center),
            )
            .padding(0)
            .style(btn_style([CLEAR; 3], [footer; 3], 0.0))
            .on_press(UiMessage::OpenPollVotes(chat.id.clone(), m.id.clone()))
            .into(),
            true => Space::new().into(),
            false => column![]
                .push((total > 0).then(|| semi(votes, 13.0).line_height(Pixels(16.0)).color(fg)))
                .push(
                    txt(tr("Results will be displayed when the poll ends"), 11.0).line_height(Pixels(14.0)).color(sub),
                )
                .spacing(4)
                .into(),
        };
        let foot = (!shown || total > 0).then(|| container(foot).padding(Padding::ZERO.top(10.0)));
        let body = column![head, Space::new().height(10), options].push(foot).push(Space::new().width(240));
        container(body).padding(Padding::new(14.0).bottom(12.0)).max_width(380).style(fill_box(bg, 17.0)).into()
    }

    fn view_scheduled<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let mut all: Vec<&Scheduled> = st.scheduled.iter().filter(|s| s.chat_id == chat.id).collect();
        if all.is_empty() {
            return None;
        }
        all.sort_by_key(|s| s.send_on);
        let (canceled, pending): (Vec<&Scheduled>, Vec<&Scheduled>) = all.into_iter().partition(|s| s.canceled);
        let (collapsed, collapsible) = if pending.is_empty() { (false, false) } else { self.pane().sched_fold };
        let h24 = st.settings.pref("timeFormat") == 1;
        let fg = p.on_secondary;
        let bg = if self.wallpaper_fg(st, chat, false).is_some() { p.card.scale_alpha(0.85) } else { p.card };
        let minute = |s: &Scheduled| s.send_on.div_euclid(60_000);
        let list = |v: &[&Scheduled]| -> Vec<El<'a>> {
            let rows = v.iter().enumerate().map(|(i, s)| {
                let first = i == 0 || (!s.canceled && minute(v[i - 1]) != minute(s));
                let date = first.then(|| {
                    let group = v.iter().filter(|o| s.canceled || minute(o) == minute(s));
                    let ids = group.map(|o| o.id.clone()).collect();
                    let (text, edit, weight) = match s.canceled {
                        true => (tr("Send Later message canceled by reply.").into(), tr("Options"), Weight::Normal),
                        false => (trf("Send %s", &[&send_day(s.send_on, h24)]), tr("Edit"), Weight::Semibold),
                    };
                    let edit = button(semi(edit, 11.0).line_height(Pixels(14.0)).color(p.accent))
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                        .on_press(UiMessage::ScheduledMenu(ids, false));
                    let line = row![]
                        .push((!s.canceled).then(|| tinted(&COMPOSER_ICONS[6], 12.0, fg)))
                        .push(txt(text, 11.0).line_height(Pixels(14.0)).font(fw(weight)).color(fg))
                        .push(edit);
                    container(line.spacing(4).align_y(Center)).center_x(Fill).padding(Padding {
                        top: 4.0,
                        right: 8.0,
                        bottom: 8.0,
                        left: 8.0,
                    })
                });
                column![].push(date).push(self.view_scheduled_bubble(s, bg)).into()
            });
            rows.collect()
        };
        let toggle = (collapsed || collapsible).then(|| {
            let n = pending.len();
            let text = match collapsed && n > 1 {
                true => trn("%d Message to Send Later", "%d Messages to Send Later", n, &[&n]),
                false => tr("Send Later").into(),
            };
            let chevron = svg(CHEVRON_DOWN.clone())
                .width(12)
                .height(12)
                .rotation(Radians(if collapsed { 0.0 } else { std::f32::consts::PI }))
                .style(move |_, _| svg::Style { color: Some(fg) });
            let label = row![
                tinted(&COMPOSER_ICONS[6], 12.0, fg),
                semi(text, 11.0).line_height(Pixels(14.0)).color(fg),
                chevron
            ];
            let toggle = button(label.spacing(4).align_y(Center))
                .padding([4, 8])
                .style(bordered_btn([p.card; 3], [fg; 3], p.chip, 1.0, 100.0))
                .on_press(UiMessage::FoldScheduled(!collapsed));
            container(toggle).center_x(Fill).padding(Padding::ZERO.bottom(4.0))
        });
        let shown = if collapsed { Vec::new() } else { list(&pending) };
        let col = Column::with_children(list(&canceled)).push(toggle).extend(shown).spacing(2);
        Some(col.padding(Padding::ZERO.top(6.0)).into())
    }

    /// A scheduled message like Desktop's `is-scheduled` bubble: `bg`, with a dashed border in the text color.
    fn view_scheduled_bubble(&self, s: &Scheduled, bg: Color) -> El<'_> {
        let p = &self.pal;
        let fg = if s.canceled { hex(0x808080) } else { p.accent };
        let text = txt(s.text.clone(), 13.0)
            .line_height(Pixels(16.0))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .color(fg);
        let bubble = container(text)
            .padding([9.0, BUBBLE_PAD_X])
            .max_width(0.45 * self.pane().size.width)
            .style(fill_box(bg, 17.0));
        let bubble = mouse_area(stack![bubble, dashed::dashes(fg, 17.0)])
            .on_right_press(UiMessage::ScheduledMenu(vec![s.id.clone()], true));
        row![Space::new().width(Fill), bubble].into()
    }

    fn view_message_menu(&self, mid: &str) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let m = &st.messages[st.msg_idx_by_id[mid]];
        let (cid, mid, mine) = (m.chat_id.clone(), m.id.clone(), m.is_sender == Some(true));
        let text = m.text.clone().unwrap_or_default();
        let link = m.links.first().map(|l| l.url.clone());
        let entire = link.is_some();
        let rule = || (menu_rule(p), 11.0);
        let item = |label: String, hotkey, msg: Option<UiMessage>| {
            (menu_item(p, None, label, hotkey, msg.map(|m| UiMessage::MessageMenu(Box::new(m)))), MENU_ITEM)
        };
        let bookmark = tr(if m.is_bookmarked == Some(true) { "Remove Bookmark" } else { "Bookmark" });
        let at = trf(if mine { "Sent at %s" } else { "Received at %s" }, &[&locale_string(&m.timestamp)]);
        let stamp = st.settings.on("copyTimestamps");
        let files = m.attachments.len();
        let image = m.attachments.first().is_some_and(|a| a.attachment_type == "img");
        let only_image = image && m.text.is_none();
        let download = match m.attachments.first().and_then(|a| a.file_size).filter(|_| files == 1) {
            Some(n) => trf(if image { "Download Image (%s)" } else { "Download Attachment (%s)" }, &[&human_size(n)]),
            None => tr(match (image, files == 1) {
                (true, true) => "Download Image",
                (true, false) => "Download Images",
                (false, true) => "Download Attachment",
                (false, false) => "Download Attachments",
            })
            .into(),
        };
        let rows: Vec<(El, f32)> = [
            self.video_menu_rows(&mid),
            link.map(|u| vec![item(tr("Copy Link").into(), "", Some(UiMessage::CopyText(u))), rule()]),
            (editable(m) && !text.is_empty())
                .then(|| vec![item(tr("Edit…").into(), "Ctrl T", Some(UiMessage::StartEdit(mid.clone(), text)))]),
            Some(vec![
                item(tr("Reply…").into(), "Ctrl R", Some(UiMessage::StartReply(mid.clone()))),
                item(tr("React…").into(), "", Some(UiMessage::React(mid.clone()))),
                item(bookmark.into(), "", Some(UiMessage::ToggleBookmark(mid.clone()))),
            ]),
            (m.is_unread != Some(true) && m.account_id != "hungryserv").then(|| {
                vec![item(
                    tr("Mark as Unread").into(),
                    "",
                    Some(UiMessage::MarkUnreadFromHere(cid.clone(), mid.clone())),
                )]
            }),
            (mine
                && m.poll.as_ref().is_some_and(|p| !p.ended)
                && m.is_deleted != Some(true)
                && st.feature("enable-polls").on
                && st.chats[st.chat_idx_by_id[&cid]].polls.is_some_and(|r| r.end))
            .then(|| vec![item("End Poll".into(), "", Some(UiMessage::EndPoll(cid.clone(), mid.clone())))]),
            self.pane().reply_thread.is_some().then(|| {
                let jump = UiMessage::Commands(vec![
                    UiMessage::JumpTo(cid, mid.clone()),
                    UiMessage::SelectMessages(vec![mid.clone()]),
                ]);
                vec![item(tr("Jump to Message").into(), "", Some(jump))]
            }),
            mine.then(|| {
                vec![item(tr("Delete for Everyone…").into(), "", Some(UiMessage::AskDeleteMessage(mid.clone())))]
            }),
            Some(vec![
                rule(),
                item(
                    tr(if entire { "Copy Entire Message" } else { "Copy" }).into(),
                    if stamp { "" } else { "Ctrl C" },
                    Some(match only_image {
                        true => UiMessage::CopyImage(mid.clone(), 0),
                        false => UiMessage::CopyText(copy_text(m, false, false)),
                    }),
                ),
            ]),
            (!image).then(|| {
                let copy = UiMessage::CopyText(copy_text(m, true, false));
                let label = tr(if entire { "Copy Entire Message with Timestamp" } else { "Copy with Timestamp" });
                vec![item(label.into(), if stamp { "Ctrl C" } else { "" }, Some(copy))]
            }),
            (image && !only_image)
                .then(|| vec![item(tr("Copy Image").into(), "", Some(UiMessage::CopyImage(mid.clone(), 0)))]),
            (m.is_deleted != Some(true)).then(|| {
                vec![item(tr("Forward…").into(), "Ctrl Shift F", Some(UiMessage::Forward(vec![mid.clone()])))]
            }),
            (files > 0).then(|| vec![item(download, "Ctrl D", Some(UiMessage::Download(mid.clone(), None)))]),
            Some(vec![
                item(at, "", None),
                rule(),
                item(tr("Report a Problem").into(), "Ctrl Shift P", Some(UiMessage::OpenUrl(ISSUES_URL.into()))),
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
                semi(f.label(chip).unwrap_or_else(|| tr(SEARCH_CHIPS[chip]).into()), 13.0).color(p.text),
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
                let st = self.store.read();
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
                radio(f.sent == Some(true), tr("Sent"), SearchPick::Sent(true)),
                radio(f.sent == Some(false), tr("Received"), SearchPick::Sent(false)),
                menu_rule(p),
                radio(f.group == Some(true), trx("Group", "general search chat type filter"), SearchPick::Group(true)),
                radio(f.group == Some(false), tr("Direct message"), SearchPick::Group(false)),
            ]),
            2 => menu(
                PERIODS
                    .iter()
                    .enumerate()
                    .map(|(i, l)| radio(f.period == Some(i), tr(l), SearchPick::Period(i)))
                    .collect(),
            ),
            3 => brand(
                MEDIA_KINDS
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (tr(l).to_string(), f.media.contains(&i), SearchPick::Media(i)))
                    .collect(),
            ),
            _ => unreachable!("chip {chip} has no menu"),
        }
    }

    /// Desktop's account context menu (`gYe`) of a sidebar account or network space.
    fn view_account_menu(&self, entry: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let space = network_space(&st, entry);
        let account = space.map_or(entry, |n| n.account_id.as_str());
        let bridge = match account {
            "hungryserv" => "hungryserv",
            _ => &st.bridge_accounts.iter().find(|b| b.account_id == account).expect("a rail account").bridge_id,
        };
        let (key, now) =
            beeper_core::matrix::inbox_rule(&st.inbox_rules, account, bridge, space.map(|n| n.id.as_str()));
        let props = st.settings.account(account);
        let muted = !props.notifications;
        let write = st.matrix.is_some();
        let options = INBOX_OPTIONS.iter().filter(|o| space.is_none() || o.0 != "dmOnly");
        let sub = (self.chat_sub == Some(0)).then(|| {
            let items = options.map(|&(o, label)| {
                let lead = container((o == now).then(|| ic(icon::CHECK, 14.0))).width(22).into();
                let msg = write.then(|| UiMessage::SetInboxVisibility(key.clone(), o));
                menu_item(p, Some(lead), inbox_label(label).into(), "", msg)
            });
            let body = container(Column::with_children(items).padding([4, 0])).width(ACCOUNT_SUB_W);
            body.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
        });
        let inbox = menu_item(p, None, tr("Show in Inbox").into(), "›", Some(UiMessage::ChatSub(Some(0))));
        let mute = beeper_core::models::AccountProps { notifications: muted, ..props };
        let mute = menu_item(
            p,
            None,
            trx(if muted { "Unmute" } else { "Mute" }, "account context menu action").into(),
            "",
            Some(UiMessage::SetAccountProps(account.into(), mute)),
        );
        let col = column![
            mouse_area(inbox).on_enter(UiMessage::ChatSub(Some(0))),
            mouse_area(mute).on_enter(UiMessage::ChatSub(None))
        ];
        let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        row![menu].push(sub).into()
    }

    /// The chat menu, with `room` pixels from its top to the bottom of the window.
    fn view_chat_menu(&self, cid: &str, sub_left: bool, room: f32) -> El<'_> {
        let p = &self.pal;
        if let Some(entry) = cid.strip_prefix("account:") {
            return self.view_account_menu(entry);
        }
        if let Some(n) = cid.strip_prefix("draft:") {
            let delete =
                menu_item(p, None, tr("Delete Chat").into(), "", Some(UiMessage::DeleteNewChat(n.parse().unwrap())));
            return container(column![delete].padding([4, 0]))
                .width(124)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into();
        }
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let on = |v: Option<bool>| v == Some(true);
        let flag = |f: ChatFlag| Some(UiMessage::ToggleFlag(c.id.clone(), f));
        let archived = on(c.is_archived);
        let unread = c.unread_count > 0 || on(c.is_marked_unread);
        let close_sub = |items: Column<'static, UiMessage>| mouse_area(items).on_enter(UiMessage::ChatSub(None));
        let item = |label: &str, key: &'static str, msg| menu_item(p, None, label.into(), key, msg);
        let open_sub = |i: u8, label: &str| {
            mouse_area(menu_item(p, None, label.into(), "›", Some(UiMessage::ChatSub(Some(i)))))
                .on_enter(UiMessage::ChatSub(Some(i)))
        };
        let pick = |on: bool, yes, no| if on { yes } else { no };
        let selecting = self.selection.is_empty();
        let col = column![
            close_sub(column![
                item(
                    pick(archived, tr("Unarchive"), trx("Archive", "thread context menu action")),
                    "Ctrl E",
                    flag(ChatFlag::Archived).filter(|_| archived || !on(c.is_pinned))
                ),
                item(tr(pick(on(c.is_muted), "Unmute", "Mute")), "Ctrl Shift M", flag(ChatFlag::Muted)),
                item(
                    tr(pick(unread, "Mark as Read", "Mark as Unread")),
                    "Ctrl Shift U",
                    Some(UiMessage::ToggleRead(c.id.clone()))
                ),
                item(tr(pick(on(c.is_pinned), "Unpin", "Pin")), "Ctrl P", flag(ChatFlag::Pinned)),
            ]),
            open_sub(0, tr("Copy")),
            close_sub(column![
                menu_rule(p),
                item(tr("Search…"), "Ctrl F", Some(UiMessage::SearchRoom(c.id.clone()))),
                menu_rule(p),
                item(
                    tr(pick(on(c.is_low_priority), "Move to Inbox", "Move to Low Priority")),
                    "",
                    flag(ChatFlag::LowPriority)
                ),
            ]),
            open_sub(1, tr("Remind Me")),
            close_sub(
                column![
                    menu_rule(p),
                    item(tr("Delete Chat"), "", (c.delete_modes != 0).then(|| UiMessage::AskDeleteChat(c.id.clone()))),
                ]
                .push(selecting.then(|| menu_rule(p)))
                .push(selecting.then(|| item(
                    tr("Open in New Window"),
                    "",
                    Some(UiMessage::OpenChatWindow(c.id.clone()))
                )))
                .push(selecting.then(|| item(tr("Begin Selecting"), "", Some(UiMessage::BeginSelecting(c.id.clone())))))
                .push(menu_rule(p))
                .push(item(
                    tr("Report a Problem"),
                    "Ctrl Shift P",
                    Some(UiMessage::OpenUrl(ISSUES_URL.into()))
                ))
            ),
        ];
        let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let sub = self.chat_sub.map(|i| {
            let item = |label: &str, at: String, msg| menu_item(p, None, label.into(), at, msg);
            let (items, rules): (Vec<El>, f32) = match i {
                0 => (vec![item(tr("Chat Link"), String::new(), Some(UiMessage::CopyChatLink(c.id.clone())))], 0.0),
                _ => {
                    let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
                    let set = |at| UiMessage::SetReminder(c.id.clone(), Some(at));
                    let last =
                        last_used(&st, now).map(|at| item(tr("Last Used"), last_used_label(at, h24), Some(set(at))));
                    let presets = remind_presets(now).map(|(label, at)| {
                        let msg = UiMessage::Commands(vec![UiMessage::LastTime(label.into()), set(at)]);
                        item(tr(label), remind_label(at, h24), Some(msg))
                    });
                    let presets = last.into_iter().chain(presets);
                    let dismiss = c.reminder.is_some().then(|| UiMessage::SetReminder(c.id.clone(), None));
                    let custom = UiMessage::OpenSubMenu(commands::SubMenu::Remind(c.id.clone()));
                    let custom = item(tr("Custom…"), String::new(), Some(custom));
                    let dismiss = item(tr("Dismiss Reminder"), String::new(), dismiss);
                    (presets.chain([menu_rule(p), custom, menu_rule(p), dismiss]).collect(), 2.0)
                }
            };
            let h = 8.0 + items.len() as f32 * MENU_ITEM - rules * (MENU_ITEM - 11.0);
            let top = if i == 0 { 4.0 * MENU_ITEM } else { 7.0 * MENU_ITEM + 22.0 };
            let top = top.min(room - h).max(0.0);
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

    fn set_reminder(&mut self, cid: &str, at: Option<i64>, no_reply: Option<bool>) {
        {
            let mut st = self.store.write();
            let idx = st.resolve_chat_idx(cid).expect("chat exists");
            self.remind(&mut st, idx, at, no_reply);
        }
        self.chat_menu = None;
        self.refresh_theme_and_tray();
    }

    /// Desktop's `setThreadReminder`: the reminder ends on a reply unless `no_reply` or else "Default for remind later" is "Always",
    /// and the chat is archived when "Archive after setting reminder on chat" is on.
    fn remind(&self, st: &mut BeeperStore, idx: usize, at: Option<i64>, no_reply: Option<bool>) {
        let dismiss_on_incoming_message = no_reply.unwrap_or(st.settings.pref("remindLaterDefault") == 0);
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
        let pick = match self.store.read().settings.pref(key) {
            0 => next,
            1 => prev,
            2 => None,
            _ => return Task::none(),
        };
        match pick {
            Some(id) => self.update(UiMessage::SelectChat(id.clone())),
            None => {
                self.leave_chat(&mut self.store.write(), None);
                self.pane.selected_chat_id = None;
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
                (6, 20.0, tr("Dismiss Reminder"), vec![], UiMessage::SetReminder(c.id.clone(), None))
            }
            _ if c.draft.is_some() => (5, 20.0, tr("Delete draft"), vec![], UiMessage::DeleteDraft(c.id.clone())),
            0 if archived || c.flags & CHAT_PINNED == 0 => {
                let msg = UiMessage::ToggleFlag(c.id.clone(), ChatFlag::Archived);
                let label = if archived { tr("Unarchive") } else { trx("Archive", "button label to archive chat") };
                (usize::from(archived), 14.0, label, vec!["Ctrl", "E"], msg)
            }
            1 => {
                let label = tr(if unread { "Mark Read" } else { "Mark Unread" });
                (2 + usize::from(!unread), 20.0, label, vec!["Ctrl", "Shift", "U"], UiMessage::ToggleRead(c.id.clone()))
            }
            2 => (4, 20.0, tr("Open Context Menu"), vec![], UiMessage::OpenChatMenu(c.id.clone())),
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
        let st = self.store.read();
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
                let label = rail_label(&st, &acc);
                let logo: El<'static> = match label {
                    Some(l) => label_icon(l, 28.0, p.strong),
                    None => svg(brand_svg(brand_key(owner))).width(28).height(28).into(),
                };
                let n = tally(&|c| {
                    in_rail_entry(&st, &acc, c) && c.flags & CHAT_HIDDEN != 0 && c.flags & CHAT_LOW_PRIORITY == 0
                });
                let label = match (label, space) {
                    (Some(l), _) => l.name.clone(),
                    (None, Some(n)) => format!("{} ({})", n.name, network_label(owner)),
                    (None, None) => network_label(owner).to_string(),
                };
                let btn =
                    badge(rail_btn(logo, on, UiMessage::SelectAccount((!on).then_some(acc.clone()))).into(), n, true);
                let btn: El = match acc.starts_with("label:") {
                    true => btn,
                    false => mouse_area(btn).on_right_press(UiMessage::OpenChatMenu(format!("account:{acc}"))).into(),
                };
                match self.chat_menu {
                    Some(_) => c.push(btn),
                    None => c.push(self.tip(btn, label, hotkey(7 + i), tooltip::Position::Right)),
                }
            });
        let add_label = button(add_space).padding(0).style(ghost(p, 12.0)).on_press(UiMessage::ToggleCommandPalette);
        let labels = st.feature("labels-v2");
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
            (labels.show && st.labels.is_empty()).then(|| tip(add_label.into(), tr("Create Label"), vec![])),
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
        let me = st.me();
        let media = UiMessage::OpenMedia;
        let me_btn = button(self.face(me.img_url.as_deref(), me.full_name.as_deref().expect("self name"), 36.0, 0.0))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.text; 3], 18.0))
            .on_press(UiMessage::OpenSettings(SettingsTab::General));
        let footer = column![
            tip(rail_btn(g(icon::PHOTO_LIBRARY), false, media).into(), tr("Media"), vec![]),
            tip(
                rail_btn(g(icon::LIBRARY_ADD), false, UiMessage::OpenSettings(SettingsTab::Accounts)).into(),
                tr("Add an account"),
                vec!["Ctrl", ","]
            ),
            tip(me_btn.into(), tr("Settings"), vec!["Ctrl", ","]),
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
        let st = self.store.read();
        let chats = self.visible_chats(&st);
        let h24 = st.settings.pref("timeFormat") == 1;
        let compact = st.settings.on("compactChatList");
        let (av, lines, title_size, ts_size, pin_av) =
            if compact { (28.0, 1, 13.0, 10.0, 40.0) } else { (46.0, 2, 14.0, 11.0, 52.0) };
        let names: Vec<&str> =
            (0..FILTERS.len()).filter(|i| self.filters & 1 << i != 0).map(|i| tr(FILTERS[i].0)).collect();
        let heading =
            self.selected_account_filter.as_deref().map_or(self.active_tab.label().to_string(), |id| {
                match (rail_label(&st, id), network_space(&st, id)) {
                    (Some(l), _) => l.name.clone(),
                    (None, Some(n)) => n.name.clone(),
                    (None, None) => network_label(id).to_string(),
                }
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
            self.tip(el.into(), tr("Filter"), vec![], tooltip::Position::Bottom)
        });
        let below = |el: El<'static>, label, keys| self.tip(el, label, keys, tooltip::Position::Bottom);
        let options = (self.active_tab == InboxTab::Reminders).then(|| {
            let btn = button(container(tinted(&TUNE, 18.0, p.strong)).center_x(30).center_y(26))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 8.0))
                .on_press(UiMessage::ReminderMenu);
            below(btn.into(), tr("Display Options"), vec![])
        });
        let header = row![title]
            .push(filter)
            .push(options)
            .push(below(
                head_btn(icon::SEARCH, 23.0, [7.5, 10.0], UiMessage::ToggleCommandPalette).into(),
                tr("Search Chats"),
                vec!["Ctrl", "K"],
            ))
            .push(below(
                head_btn(icon::EDIT_SQUARE, 20.0, [0.0, 3.0], UiMessage::NewChat).into(),
                tr("New Chat"),
                vec!["Ctrl", "N"],
            ))
            .align_y(Center)
            .height(52)
            .padding(Padding::ZERO.right(8.0));
        let selected = self.creating_chat.is_none().then_some(self.pane().selected_chat_id.as_deref()).flatten();
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
        let bar_pad = if SOLID_BARS { 0.0 } else { 4.0 };
        let inner = w - 8.0 - bar_pad - GUTTER;
        let cols = (((w - 16.0) / 80.0).floor() as usize).max(1);
        let cell = inner / cols as f32;
        let previews = st.settings.on("showMessagePreviews");
        let now = now_unix_millis();
        let launched = now - self.started.elapsed().as_millis() as i64;
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
                let bubble = bubble_text(&st, c, launched, now).filter(|_| previews && !on && !selecting);
                let unread = unread.filter(|_| bubble.is_none());
                let face = stack![container(self.chat_avatar(c, pin_av, 1.25)).center_x(pin_av + 4.0)]
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
                let colors = if muted { (p.frame, p.text) } else { (p.accent, Color::WHITE) };
                let bubble = bubble.map(|t| pin_bubble(&t, c.unread_mentions > 0, colors, cell, pin_av, compact));
                let item = stack![item].push(bubble);
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
        let text_w = inner - 16.0 - av - 10.0 - lead;
        let frame = |key: String, dot: Option<El<'static>>, face: El<'static>, lines: [El<'static>; 2], on, select| {
            let [top, preview] = lines;
            let item = row![]
                .push(dot)
                .push(container(face).width(av + 10.0))
                .push(column![top, preview].width(Fill))
                .align_y(Center)
                .padding(Padding::new(8.0).top(9.0).bottom(10.0).left(if selecting { 0.0 } else { 8.0 }));
            let action =
                (!selecting && self.hovered_chat.as_ref() == Some(&key)).then(|| match key.strip_prefix("draft:") {
                    Some(n) => Some(self.hover_btn(
                        5,
                        20.0,
                        tr("Delete Chat"),
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
            let title = ell(tr("New Chat"), text_w, title_size, if counts { Weight::Medium } else { Weight::Semibold });
            let rows =
                [container(title.color(p.strong)).height(16).into(), Space::new().height(16 * lines as u32).into()];
            let face = self.face(None, "", av, 1.25);
            frame(format!("draft:{n}"), dot, face, rows, self.creating_chat == Some(n), UiMessage::SelectNewChat(n))
        });
        let rows = rest.iter().fold(Column::with_children(drafts.map(El::from)), |col, c| {
            let on = selected == Some(c.id.as_str());
            let unread = c.unread_count > 0 || c.is_marked_unread == Some(true);
            let muted = c.is_muted == Some(true);
            let draft = c
                .draft
                .as_ref()
                .and_then(|d| d.text.as_deref())
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
            let m = preview_message(&st, c).filter(|_| previews && draft.is_none());
            let reaction = m.filter(|m| m.message_type.as_deref() == Some("REACTION"));
            let next = next_send(&st, &c.id);
            let ts = match (self.active_tab, &c.reminder, next) {
                (InboxTab::Reminders, Some(r), _) => {
                    format!("{} {}", reminder_day(r.remind_at_ms), clock(r.remind_at_ms, h24))
                }
                (InboxTab::SendLater, _, Some(at)) => format!("{} {}", reminder_day(at), clock(at, h24)),
                _ if draft.is_some() => String::new(),
                _ => c.last_activity.as_deref().map(|t| list_ts(t, h24)).unwrap_or_default(),
            };
            let later = next.is_some()
                && (draft.is_none() || matches!(self.active_tab, InboxTab::Reminders | InboxTab::SendLater));
            let ts_w = crate::style::text_width(&ts, ts_size, Weight::Normal) + 8.0 + if later { 12.0 } else { 0.0 };
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
                .push(ell(&title, text_w - ts_w - bell - pin_w, title_size, weight).color(p.fg.scale_alpha(0.9)))
                .push(muted.then(|| container(tinted(&BELL_OFF, 12.0, p.strong)).padding([0, 2])))
                .push(Space::new().width(Fill))
                .push(later.then(|| tinted(&COMPOSER_ICONS[6], 12.0, p.strong.scale_alpha(0.7))))
                .push(
                    txt(ts, ts_size).color(p.strong.scale_alpha(0.7)).font(iced::Font { style, ..fw(Weight::Normal) }),
                )
                .height(16)
                .align_y(Center);
            let mine = m.is_some_and(|m| m.is_sender == Some(true));
            let receipt = mine && reaction.is_none();
            let media_tag = m.and_then(message_tag);
            let (body, italic) = match m.and_then(preview_body) {
                Some(t) => (Some(t), false),
                None => media_tag.as_ref().and_then(|t| t.2.clone()).map_or((None, false), |(d, i)| (Some(d), i)),
            };
            let show_sender = !mine
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
            let base = 16.0 + if counts { 8.0 + 6.0 * digits + at } else { at }.max(18.0);
            let indent = if receipt { 16.0 } else { 0.0 }
                + if sender.is_some() { 16.0 } else { 0.0 }
                + match &media_tag {
                    Some((_, label, _)) if expired => crate::style::text_width(label, 13.0, Weight::Normal) + 4.0,
                    Some((_, label, _)) => crate::style::text_width(label, 10.0, Weight::Medium) + 24.0,
                    None => 0.0,
                }
                + if expired { crate::style::text_width(tr("Reminder"), 11.0, Weight::Semibold) + 10.0 } else { 0.0 }
                + prefix_w;
            let reserve = base + indent;
            let flowed = lines > 1 && indent > 0.0;
            let (body, second) = match body {
                Some(b) if flowed => {
                    let (first, rest) = crate::style::flow(&b, indent, text_w - base, lines, 13.0, Weight::Normal);
                    (Some(first), Some(rest).filter(|r| !r.is_empty()))
                }
                b => (b, None),
            };
            let style = if italic { iced::font::Style::Italic } else { iced::font::Style::Normal };
            let body_font = iced::Font { style, ..fw(Weight::Normal) };
            let read = m.is_some_and(|m| m.seen.keys().any(|u| !st.is_me(u) && u != &m.sender_id));
            let fg = p.fg.scale_alpha(0.63);
            let face =
                sender.map(|u| self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 12.0, 0.5));
            let badge = (expired && previews).then(|| {
                let label = semi(tr("Reminder"), 11.0).line_height(Pixels(14.0)).color(Color::WHITE.scale_alpha(0.7));
                container(label).padding([0, 3]).style(fill_box(ORANGE.scale_alpha(0.7), 4.0))
            });
            let preview = row![]
                .push(badge)
                .push(receipt.then(|| tinted(&SENT[usize::from(read)], 12.0, fg)))
                .push(face)
                .push(prefix.map(|s| txt(s, 13.0).color(fg)))
                .push(media_tag.map(|(icon, label, _)| -> El<'_> {
                    if expired {
                        return txt(label, 13.0).color(fg).into();
                    }
                    let glyph = tinted(if icon == POLL_TAG { &COMPOSER_ICONS[8] } else { &TAG_ICONS[icon] }, 12.0, fg);
                    let label = med(label, 10.0).line_height(Pixels(12.0)).color(fg);
                    container(row![glyph, container(label).padding([0, 2])].align_y(Center))
                        .padding([1, 2])
                        .style(fill_box(p.strong.scale_alpha(0.105), 4.0))
                        .into()
                }))
                .push(body.map(|b| {
                    clamp(&b, text_w - reserve, if flowed { 1 } else { lines }, 13.0, Weight::Normal)
                        .width(text_w - reserve)
                        .line_height(Pixels(16.0))
                        .font(body_font)
                        .color(fg)
                }))
                .push(draft.filter(|_| previews).map(|t| {
                    let label = trx("Draft:", "thread list draft preview prefix");
                    let s =
                        crate::style::clamped(&format!("{label} {t}"), text_w - reserve, lines, 13.0, Weight::Normal);
                    let rest = s.strip_prefix(label).expect("the draft label fits").to_string();
                    rich_text![span::<(), _>(label).font(fw(Weight::Semibold)).color(p.accent), span(rest).color(fg)]
                        .size(13)
                        .line_height(Pixels(16.0))
                        .width(text_w - reserve)
                }))
                .push(Space::new().width(Fill))
                .push(count)
                .spacing(4)
                .align_y(if compact { Center } else { iced::Alignment::Start });
            let second = second.map(|r| txt(r.to_string(), 13.0).line_height(Pixels(16.0)).font(body_font).color(fg));
            let preview = column![preview].push(second);
            let avatar = self.chat_avatar(c, av, 1.25);
            let avatar = match network_icons {
                true => {
                    let ring = match (on, self.hovered_chat.as_ref() == Some(&c.id)) {
                        (true, _) => p.selected,
                        (false, true) => crate::style::mix(p.window, Color { a: 1.0, ..p.hover }, p.hover.a),
                        (false, false) => p.window,
                    };
                    badged(avatar, &c.account_id, av, if compact { 12.0 } else { 16.0 }, (!colorize).then_some(p), ring)
                }
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
                let line =
                    Row::with_children(words.chain([pill(views[0]), txt(tr("or"), 13.0).into(), pill(views[1])]))
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
            scrollable(column![pins, rows].padding(Padding::ZERO.right(bar_pad + GUTTER)))
                .direction(stable_bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .into()
        };
        container(column![header, list].padding(Padding::ZERO.left(8.0))).width(w).height(Fill).into()
    }

    /// Desktop's `IncognitoMarkAsRead`: a link below an unread chat that incognito mode keeps unread.
    fn incognito_mark_read<'a>(&self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let unread = chat.unread_count > 0 || chat.is_marked_unread == Some(true);
        (unread && st.incognito_indicator()).then(|| {
            let p = &self.pal;
            let link = button(semi(tr("Mark as Read"), 11.0).line_height(Pixels(14.0)).color(p.accent))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                .on_press(UiMessage::MarkRead(chat.id.clone()));
            let tip = self.tip(link.into(), tr("Mark as Read"), vec!["Ctrl", "Shift", "U"], tooltip::Position::Top);
            container(tip).center_x(Fill).padding(Padding::new(10.0).bottom(6.5).left(0.0).right(0.0)).into()
        })
    }

    fn view_conversation(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.as_deref().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
        let (hide, members) = (st.settings.on("hideDeleted"), st.settings.on("membershipEvents"));
        let msgs: Vec<&Message> = st.chat_msg_indices.get(cid).map_or_else(Vec::new, |v| {
            v.iter()
                .map(|&i| &st.messages[i])
                .filter(|m| !(hide && m.is_deleted == Some(true)) && (members || !membership(m)))
                .collect()
        });
        let col = self.view_messages(&st, chat, &msgs, false);
        let col = col.push(st.typing.get(cid).map(|ids| self.typing_bubble(chat, ids)));
        let col = col.push(self.incognito_mark_read(&st, chat));
        let col = col.push(self.view_scheduled(&st, chat));
        // Desktop's `messages-list` keeps the messages at the bottom when they do not fill the view.
        let scroller = |col| {
            let col = container(col).align_y(Bottom);
            scrollable(col).direction(stable_bar()).anchor_bottom().height(Fill).style(scroll_style(p.scroll))
        };
        let list = lift::tight(scroller(col).id("timeline"));
        let replies = self.pane().reply_thread.as_ref().map(|id| {
            let root = st.msg_idx_by_id.get(id).map_or(id.as_str(), |&i| reply_root(&st, &st.messages[i]));
            let chain: Vec<&Message> = msgs.iter().copied().filter(|m| reply_root(&st, m) == root).collect();
            let list = lift::tight(scroller(self.view_messages(&st, chat, &chain, true)));
            // Desktop blurs the chat 8px under a 0.6 background; iced has no backdrop blur.
            mouse_area(container(list).width(Fill).height(Fill).style(fill_box(p.card.scale_alpha(0.85), 0.0)))
                .on_press(UiMessage::HideReplies)
        });
        let close = replies.is_some().then(|| -> El<'_> {
            let close = button(ic(icon::CLOSE, 20.0).color(p.weak))
                .padding(10)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::HideReplies);
            container(close).align_right(Fill).padding(16).into()
        });
        let name_row = row![
            semi(short_title(chat).to_string(), 13.0).line_height(Pixels(16.0)).color(p.fg),
            ic(icon::CHEVRON_RIGHT, 16.0).color(p.tertiary)
        ]
        .spacing(2)
        .align_y(Center);
        let pill = button(name_row)
            .padding(Padding::new(6.0).left(10.5).right(2.5))
            .style(btn_style([p.window, p.selected, p.selected], [p.text; 3], 14.0))
            .on_press(UiMessage::ToggleRightPanel);
        let ring = container(self.chat_avatar(chat, 40.0, 0.0)).padding(1).style(fill_box(p.card, 21.0));
        let header = stack![
            container(pill).center_x(Fill).padding(Padding::ZERO.top(44.0)),
            container(ring).center_x(Fill).padding(Padding::ZERO.top(7.0))
        ];
        let feedback = feedback_btn(p);
        let fade = Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, p.card)
            .add_stop(0.75, p.card.scale_alpha(0.85))
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let fade = container(Space::new()).width(Fill).height(chat_theme::FADE).style(move |_: &Theme| {
            container::Style { background: Some(Background::Gradient(Gradient::Linear(fade))), ..Default::default() }
        });
        let wallpaper = chat_theme::resolve(&st.settings.chat_appearance, Some(cid), self.dark, &st.data_dir)
            .0
            .map(|(paint, _)| paint);
        let fade: El<'_> = match wallpaper.clone() {
            Some(paint) => iced::widget::responsive(move |size| {
                container(chat_theme::fade(paint.clone(), size)).height(chat_theme::FADE).into()
            })
            .into(),
            None => fade.into(),
        };
        let reminder = chat.reminder.as_ref().filter(|_| !self.expired(chat)).map(|r| {
            let (day, time) =
                (reminder_day(r.remind_at_ms), clock(r.remind_at_ms, st.settings.pref("timeFormat") == 1));
            let at = if day_of(r.remind_at_ms) == today() { time.clone() } else { format!("{time} {day}") };
            let tip = match r.dismiss_on_incoming_message {
                true => trf("Set to remind at %s if no reply", &[&at]),
                false => trf("Set to remind at %s", &[&at]),
            };
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
            let go = pill_btn(p, tr("Verify"), UiMessage::OpenSettings(SettingsTab::Devices));
            let bar = row![note, Space::new().width(Fill), go].align_y(Center);
            container(bar).padding([8, 14]).style(bordered_box(p.elevated, p.outline, 1.0, 10.0))
        });
        let verify = verify.map(|v| container(v).padding([0, 12]));
        let pane = column![stack![list, overlay].push(replies).push(close).width(Fill).height(Fill)]
            .push(verify)
            .push(self.view_composer(&st, chat));
        let secondary = st.settings.pref("secondaryComposerButton") != 6;
        let picker = self
            .view_emoji_picker(&st)
            .filter(|_| self.pane().emoji.as_ref().is_some_and(|pk| pk.react.is_none()))
            .map(|e| {
                let right = if secondary { 112.0 } else { 82.0 };
                container(e).align_right(Fill).align_bottom(Fill).padding(Padding::ZERO.right(right).bottom(36.0))
            });
        let dropping = self.dropping.then(|| {
            let hint =
                column![bold(tr("Drop files here to attach"), 15.0), bold(tr("Hold Shift to send immediately"), 13.0)];
            // Desktop blurs the chat 10px under a 0.2 background; iced has no backdrop blur.
            let zone = container(hint.spacing(12).align_x(Center)).center(Fill).style(bordered_box(
                Color { a: 0.85, ..p.card },
                Color { a: 0.2, ..p.text },
                10.0,
                0.0,
            ));
            container(zone).padding(Padding { top: 39.0, right: 13.0, bottom: 6.5, left: 6.5 })
        });
        let wallpaper =
            wallpaper.map(|paint| iced::widget::responsive(move |size| chat_theme::fill(paint.clone(), size, 10.0)));
        let pane = stack![].push(wallpaper).push(pane).push(picker).push(self.view_suggestions(&st)).push(dropping);
        let shadow = Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.18), offset: Vector::ZERO, blur_radius: 20.0 };
        let card = fill_box(p.card, 10.0);
        container(pane)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(move |t| container::Style { shadow, ..card(t) })
            .into()
    }

    /// Desktop's `UnreadDivider`: "New", or the incognito line with the days left of a free trial.
    fn unread_divider<'a>(&self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let hush = st.incognito_indicator();
        let color = if hush { p.rule } else { p.accent.scale_alpha(0.35) };
        let line = || container(Space::new()).width(Fill).height(1).style(fill_box(color, 0.0));
        if !hush {
            return row![line(), semi(tr("New"), 11.0).color(p.accent), line()]
                .spacing(8)
                .align_y(Center)
                .padding([6, 0])
                .into();
        }
        let label = txt(format!("🫣 {}", tr("Unread until you reply")), 11.0).line_height(Pixels(14.0));
        let days = st.features.trial_days_left().map(|n| {
            let n = n as usize;
            let days = trn("%d more day of free incognito mode", "%d more days of free incognito mode", n, &[&n]);
            txt(days, 11.0).line_height(Pixels(14.0)).color(p.sub)
        });
        let label = column![label.color(p.on_secondary)].push(days).spacing(2).align_x(Center);
        row![line(), label, line()].spacing(10).align_y(Center).padding([12, 0]).into()
    }

    /// Desktop's `MessageList`: the messages in timeline order. In a reply chain, the quotes, the reply lines,
    /// the replies buttons and the unread divider do not show.
    fn view_messages<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        msgs: &[&Message],
        thread: bool,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        let mut run_start: Vec<usize> = (0..msgs.len()).collect();
        for i in 1..msgs.len() {
            let (a, b) = (msgs[i - 1], msgs[i]);
            if membership(a) && membership(b) && day_num(&a.timestamp) == day_num(&b.timestamp) {
                run_start[i] = run_start[i - 1];
            }
        }
        let mut col = Column::new().padding(Padding::new(8.0).right(8.0 + GUTTER).top(96.0).bottom(0.0));
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
        let first_unread = first_unread.filter(|_| !thread);
        let (h24, group) = (st.settings.pref("timeFormat") == 1, chat.chat_type == "group");
        let roots: Vec<&str> = msgs.iter().map(|m| reply_root(st, m)).collect();
        // Desktop's `replyCounts`: the messages of the same reply chain after each message.
        let mut later: HashMap<&str, usize> = HashMap::new();
        let mut counts = vec![0; msgs.len()];
        for i in (0..msgs.len()).rev() {
            let n = later.entry(roots[i]).or_insert(0);
            (counts[i], *n) = (*n, *n + 1);
        }
        for (i, m) in msgs.iter().enumerate() {
            let day_changed = prev.is_none_or(|q| day_num(&q.timestamp) != day_num(&m.timestamp));
            let mut head = Column::new().push(prev.map(|q| Space::new().height(gap_below(q, m))));
            if day_changed {
                let label = semi(day_label(&m.timestamp), 11.0).line_height(Pixels(14.0)).color(p.date_text);
                let pill = container(label).padding([5, 9]).style(bordered_box(p.card, p.outline, 1.0, 12.0));
                head = head.push(container(pill).center_x(Fill).padding(Padding::new(4.0).left(GUTTER).right(0.0)));
            }
            if Some(i) == first_unread {
                head = head.push(self.unread_divider(st));
            }
            if membership(m) {
                let s = run_start[i];
                let size = run_start[s..].iter().take_while(|&&r| r == s).count();
                let open = self.pane().open_members.contains(&msgs[s].id);
                let dim = self.wallpaper_fg(st, chat, thread).unwrap_or(p.muted).scale_alpha(0.75);
                if size >= 3 && i == s {
                    let label = row![
                        txt(format!("{size} Membership Events"), 11.0).line_height(Pixels(14.0)).color(dim),
                        txt(if open { tr("Collapse") } else { tr("Expand") }, 11.0).color(p.accent.scale_alpha(0.75))
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
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id || membership(q));
            let last = msgs.get(i + 1).is_none_or(|n| {
                n.sender_id != m.sender_id || membership(n) || day_num(&n.timestamp) != day_num(&m.timestamp)
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
                        trf("Seen by %s", &[&names.join(", ")])
                    } else {
                        let (first, more) = (names[..3].join(", "), names.len() - 3);
                        trn("Seen by %1$s & %2$d other", "Seen by %1$s & %2$d others", more, &[&first, &more])
                    })
                } else {
                    let played = matches!(m.message_type.as_deref(), Some("VOICE" | "AUDIO"));
                    Some(match day_num(ts) == today() {
                        true => trf(if played { "Played at %s" } else { "Seen at %s" }, &[&format_short_ts(ts, h24)]),
                        false => trf(if played { "Played %s" } else { "Seen %s" }, &[&day_label(ts)]),
                    })
                }
            } else {
                (Some(i) == last_live && m.is_sender == Some(true)).then(|| tr("Sent").into())
            };
            let chained = i > 0 && roots[i - 1] == roots[i];
            let split = roots.get(i + 1).is_some_and(|r| *r != roots[i]);
            let replies = if !thread && split { counts[i] } else { 0 };
            let view = self.view_message(st, chat, m, (first, last, chained, thread), (status, replies), head);
            let view = match send_rise(st, m) {
                Some(t) => lift::lift(view, 14.0 * (1.0 - t), 0.97 + 0.03 * t).into(),
                None => view,
            };
            let flash = self.flash.as_ref().filter(|(id, _)| *id == m.id).map(|(_, at)| {
                let t = (at.elapsed().as_secs_f32() / 1.5).min(1.0);
                let a = if t < 0.2 { ease_out(t / 0.2) } else { 1.0 - ease_out((t - 0.2) / 0.8) };
                container(Space::new()).width(Fill).height(Fill).style(fill_box(p.accent.scale_alpha(0.4 * a), 0.0))
            });
            let cell = container(stack![view].push(flash));
            col = col.push(if thread { cell } else { cell.id(format!("msg-{}", m.id)) });
            prev = Some(m);
        }
        col
    }

    fn view_message<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        m: &Message,
        (first, last, chained, thread): (bool, bool, bool, bool),
        (status, replies): (Option<String>, usize),
        head: Column<'a, UiMessage>,
    ) -> El<'a> {
        let p = &self.pal;
        let (sent, sent_fg, sent_meta) = self.sent_colors(st, Some(&chat.id));
        let on_wall = self.wallpaper_fg(st, chat, thread);
        let muted = on_wall.unwrap_or(p.muted);
        let mine = m.is_sender == Some(true);
        if m.is_deleted == Some(true) {
            let fg = on_wall.unwrap_or(mix(p.card, p.text, 0.8));
            let italic = iced::Font { style: iced::font::Style::Italic, ..fw(Weight::Normal) };
            let icon = svg(TRASH.clone())
                .width(13.0 * 448.0 / 512.0)
                .height(13)
                .style(move |_, _| svg::Style { color: Some(fg) });
            let body = row![
                icon,
                txt(tr("This message has been deleted"), 13.0).line_height(Pixels(16.0)).font(italic).color(fg)
            ];
            let pill = container(body.spacing(5).align_y(iced::Bottom))
                .padding(Padding::new(13.0).top(9.0).bottom(7.0))
                .style(bordered_box(CLEAR, mix(p.card, p.received, 0.8), 1.0, 17.0));
            let side = if mine { Right } else { iced::Left };
            let edited = m.edited().then(|| {
                let label = txt(tr("Edited"), 11.0).line_height(Pixels(15.0)).color(p.muted);
                container(label).padding([0.0, BUBBLE_PAD_X]).align_x(side).width(Fill)
            });
            return head.push(container(pill).align_x(side).width(Fill)).push(edited).into();
        }
        let group = chat.chat_type == "group";
        let selected = self.pane().selected_msgs.contains(&m.id);
        let rise = send_rise(st, m).unwrap_or(1.0);
        let dim = move |c: Color| (if selected { brightness(c, 0.7) } else { c }).scale_alpha(rise);
        let (bg, fg) = if mine { (dim(sent), dim(sent_fg)) } else { (dim(p.received), dim(p.strong)) };
        let poll = m.poll.as_ref().filter(|_| st.feature("enable-polls").on && chat.polls.is_some());
        let text = match (&m.poll, poll) {
            (Some(p), None) => p.fallback.clone(),
            _ => m.text.clone().unwrap_or_default(),
        };
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
        let linked = m.linked_message_id.as_ref().filter(|_| !thread);
        let quote = linked.and_then(|r| st.msg_idx_by_id.get(r)).map(|&i| {
            let q = &st.messages[i];
            let q_mine = q.is_sender == Some(true);
            let (edge, fill) = if q_mine { (sent, mix(p.card, sent, 0.3)) } else { (p.received, p.card) };
            let body = ell(q.text.as_deref().unwrap_or(""), 300.0, 12.0, Weight::Medium).color(p.text);
            let pill = container(body).padding(8).style(bordered_box(fill, edge, 2.0, 17.0));
            let pill = mouse_area(pill).on_press(UiMessage::ShowReplies(q.id.clone()));
            let name = (group && !q_mine).then(|| {
                container(semi(q.sender_name.clone().expect("sender name"), 12.0).color(on_wall.unwrap_or(p.strong)))
                    .padding([0, 11])
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
                        let k = (0.45 * self.pane().size.width / w).min(400.0 / h).min(1.0);
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
                    let base: El<'_> = match thumb_url.as_ref().and_then(|u| self.media.get(u)?.as_ref()) {
                        Some(Ok(frames)) => {
                            anim::gif(frames, self.started, move |f| image(f).width(w).height(h).border_radius(17.0))
                        }
                        Some(Err(_)) if m.unresolved_media.is_none() => {
                            pill(tr("Could not load. Click to try again"), 0.45)
                        }
                        _ => container(ic(icon::IMAGE, 28.0).color(p.sub))
                            .center_x(w)
                            .center_y(h)
                            .style(fill_box(p.link_card, 17.0))
                            .into(),
                    };
                    let (slot, press): (El<'_>, _) = if let Some(raw) =
                        m.unresolved_media.as_ref().filter(|_| !is_video)
                    {
                        let failed = self.resolving_media.get(&m.id) == Some(&false);
                        let (label, alpha) = match failed {
                            true => (tr("Could not load. Click to try again"), 0.45),
                            false => (tr("Click to load"), 0.25),
                        };
                        (
                            stack![base, pill(label, alpha)].into(),
                            UiMessage::ResolveMedia(m.account_id.clone(), m.id.clone(), raw.clone()),
                        )
                    } else if thumb_url.as_ref().and_then(|u| self.media.get(u)?.as_ref()).is_some_and(Result::is_err) {
                        (base, UiMessage::RetryMedia(thumb_url.expect("a failed media has a URL")))
                    } else if let Some(src) = a.src_url.as_ref().filter(|_| is_video && a.is_gif != Some(true)) {
                        parts = parts.push(self.view_video(&m.id, src, base, w, h));
                        continue;
                    } else if is_video && a.is_gif != Some(true) {
                        (base, UiMessage::OpenQuickLook(name, "Video attachment".into()))
                    } else {
                        (base, UiMessage::OpenCarousel(m.id.clone(), ai, false))
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
        if let Some(poll) = poll {
            parts = parts.push(self.view_poll(st, chat, m, poll, (bg, fg)));
        }
        if m.message_type.as_deref() == Some("HIDDEN") {
            let bold = |s: String| span::<(), _>(s).font(fw(Weight::Bold));
            let head = container(txt(tr("Hidden event"), 13.0).font(fw(Weight::Bold)))
                .padding([2.6, 5.2])
                .style(fill_box(p.chip, 0.0));
            let why = tr("Shown because you enabled Settings > General > Show hidden events in timeline");
            let body = rich_text![bold(text.clone()), span::<(), _>(" from "), bold(m.sender_id.clone())]
                .size(13.0)
                .color(p.text);
            let row =
                row![self.tip(head.into(), why, vec![], tooltip::Position::Top), body].spacing(5.2).align_y(Center);
            let section = container(row.padding(Padding::ZERO.right(6.5))).clip(true);
            parts = parts.push(section.style(bordered_box(CLEAR, p.chip, 2.0, 4.0)));
        } else if jumbo(&text) {
            parts = parts.push(container(txt(text.clone(), 48.0)).padding([0, 12]));
        } else if !text.is_empty() && poll.is_none() && m.message_type.as_deref() != Some("VOICE") {
            let ts = st
                .settings
                .on("showTimestamps")
                .then(|| format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1));
            let room = ts.as_deref().map_or(0.0, |t| crate::style::text_width(t, 11.0, Weight::Normal) + 4.0);
            let time = ts.map(|t| txt(t, 11.0).color(if mine { sent_meta } else { p.sub }));
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
            let max = 0.45 * self.pane().size.width - 2.0 * BUBBLE_PAD_X;
            let full = !st.settings.on("balanceText") && crate::style::wraps(&format!("{text}{pad}"), max, 13.0);
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
            .width(if full { Length::Fixed(max) } else { Length::Shrink });
            parts = parts.push(
                container(stack![body].push(time.map(|t| container(t).align_right(Fill).align_bottom(Fill))))
                    .padding([9.0, BUBBLE_PAD_X])
                    .max_width(0.45 * self.pane().size.width)
                    .style(fill_box(bg, 17.0)),
            );
        }
        let open = self.pane().expanded_history_msg_id.as_ref() == Some(&m.id);
        let label = |t: String| txt(t, 12.0).line_height(Pixels(15.0)).color(muted);
        let edited = m.edited().then(|| -> El<'a> {
            let small = |t: &str| txt(t.to_string(), 11.0).line_height(Pixels(14.0));
            if m.edit_history.is_empty() {
                return small(tr("Edited")).color(muted).into();
            }
            let toggle = small(if open { tr("Hide Edits") } else { tr("Edited") }).font(fw(Weight::Semibold));
            button(toggle.color(p.accent))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::ToggleEditHistory(m.id.clone()))
                .into()
        });
        let replies = (replies > 0).then(|| -> El<'a> {
            let n = trnx("%d Reply", "%d Replies", replies, "button label for replies to a message", &[&replies]);
            button(semi(n, 10.0).line_height(Pixels(12.0)).color(muted))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::ShowReplies(m.id.clone()))
                .into()
        });
        let mut footer: Vec<El<'a>> =
            [status.map(|t| label(t).into()), replies, edited].into_iter().flatten().collect();
        if !mine {
            footer.reverse();
        }
        let footer = (!footer.is_empty()).then(|| {
            let line = footer.into_iter().enumerate().fold(Row::new().align_y(Center), |r, (i, t)| {
                r.push((i > 0).then(|| container(txt("・", 12.0).color(muted.scale_alpha(0.8))).padding([0, 4])))
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
                    container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                        .padding(Padding::ZERO.right(8.0)),
                    med(name.to_string(), 13.0).color(p.text),
                ]
                .align_y(Center);
                col.push(container(line).padding([4, 8]))
            });
            let list = list.push((more > 0).then(|| container(txt(format!("and {more} more"), 13.0)).padding([4, 8])));
            let list = container(list).padding([2, 4]).style(bordered_box(p.menu, p.menu_border, 1.0, 6.0));
            let ring = if on_wall.is_some() { CLEAR } else { p.card };
            let pills = keys.into_iter().fold(Row::new().spacing(2), |r, k| {
                let reactors: Vec<_> = m.reactions.iter().filter(|x| x.reaction_key == k).collect();
                let n = reactors.len();
                let own = reactors.iter().any(|x| st.is_me(&x.participant_id));
                let mut pill = row![txt(k.to_string(), 16.0).line_height(Pixels(15.0))].spacing(4).align_y(Center);
                if group || n > 1 {
                    for x in reactors.iter().take(3) {
                        let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
                        let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
                        pill = pill.push(self.face(u.and_then(|u| u.img_url.as_deref()), name, 16.0, 0.5));
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
                        .style(bordered_btn([fill, p.selected, p.selected], [p.text; 3], ring, 2.0, 14.0))
                        .on_press(UiMessage::ToggleReaction(chat.id.clone(), m.id.clone(), k.to_string())),
                )
            });
            stack![
                container(column![parts, Space::new().width(min_w)].align_x(if mine { Right } else { iced::Left }))
                    .padding(Padding::ZERO.bottom(21.0)),
                container(crate::popover::popover(
                    pills,
                    list,
                    self.single_msg() == Some(&m.id) && st.settings.on("reactionsOnSelect"),
                ))
                .align_bottom(Fill)
                .width(Fill)
                .align_x(if mine { Right } else { iced::Left })
                .padding([0.0, BUBBLE_PAD_X]),
            ]
            .into()
        };
        let parts = column![parts].push(edits).push(footer).align_x(if mine { Right } else { iced::Left });
        let tools = (self.pane().hovered_msg.as_ref() == Some(&m.id)).then(|| {
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
                self.face(url, m.sender_name.as_deref().expect("sender name"), 28.0, 0.0)
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
                let color = on_wall.unwrap_or(p.reply_line);
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

    /// Desktop's `composeTextareaPlaceholder`.
    fn composer_placeholder(&self, chat: &Chat) -> String {
        let pane = self.pane();
        let to = match (pane.send_later, pane.reply_thread.is_some() || pane.reply_to_message_id.is_some()) {
            (Some(_), _) => "Send Later".into(),
            (None, reply) => format!("{} {}", if reply { "Reply to" } else { "Message" }, chat.title),
        };
        format!("{to} on {}", network_name(&chat.account_id))
    }

    fn view_composer<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        let banner = match (&self.pane().editing_message_id, &self.pane().reply_to_message_id) {
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
        let gif_picker = (self.pane().picker == Some(Picker::Gif)).then(|| {
            let search = text_input(tr("Search KLIPY..."), &self.gif_query)
                .id("gif-search-input")
                .on_input(UiMessage::GifQueryChanged)
                .size(13)
                .padding([6, 12])
                .style(input_style(p.input, p.strong, p.sub, p.accent, 8.0));
            let carousel: El<'a> = if let Some(err) = &self.gif_error {
                container(txt(err.clone(), 12.0).color(p.danger)).center_y(215).padding([0, 14]).into()
            } else if self.gif_loading && self.gifs.is_empty() {
                container(txt("Loading GIFs...", 12.0).color(p.sub)).center_y(215).padding([0, 14]).into()
            } else {
                let items = self.gifs.iter().map(|g| {
                    let (pw, ph) = g.preview_size;
                    let w = (215.0 * pw as f32 / ph.max(1) as f32).round().max(48.0);
                    let cell: El<'a> = match self.media.get(&g.preview_url).and_then(|v| v.as_ref()) {
                        Some(Ok(frames)) => anim::gif(frames, self.started, move |f| {
                            image(f).width(w).height(215).content_fit(iced::ContentFit::Cover)
                        }),
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
                2 => Some(UiMessage::TogglePicker(Picker::Gif)),
                3 => Some(UiMessage::PickFiles),
                4 => Some(UiMessage::OpenSendLater),
                5 => Some(UiMessage::Record(record::Msg::Start)),
                6 => chat.polls.map(|_| UiMessage::OpenPollCreator(chat.id.clone())),
                _ => None,
            };
            let active = (id == 2 && self.pane().picker == Some(Picker::Gif))
                || (id == 0 && self.pane().emoji.as_ref().is_some_and(|pk| pk.react.is_none()));
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
        let placeholder = self.composer_placeholder(chat);
        let suggesting = !self.emoji_suggestions(st).is_empty();
        let formats = chat.formatting;
        let keys = ["Message: Send Message and Archive", "Send Message", "Insert New Line"]
            .map(|h| st.settings.hotkey_keys(h).to_string());
        let undo = self.queued_here().then(|| st.settings.hotkey_keys("Undo Pending Send").to_string());
        let input = text_editor(&self.pane().composer)
            .id("composer-input")
            .placeholder(placeholder)
            .on_action(UiMessage::ComposerEdit)
            .key_binding(move |k| match k.key.as_ref() {
                _ if !matches!(k.status, text_editor::Status::Focused { .. }) => None,
                _ if undo.as_deref().zip(hotkey_key(&k.key)).is_some_and(|(keys, key)| {
                    let m = k.modifiers;
                    prefs::hotkey_has(keys, [m.control() || m.command(), m.shift(), m.alt()], &key)
                }) =>
                {
                    Some(text_editor::Binding::Custom(UiMessage::UndoSend))
                }
                Key::Named(Named::Enter) => {
                    let m = k.modifiers;
                    let mods = [m.control() || m.command(), m.shift(), m.alt()];
                    match keys.iter().position(|keys| prefs::hotkey_has(keys, mods, "enter")) {
                        Some(0) => Some(text_editor::Binding::Custom(UiMessage::SendAndArchive)),
                        Some(1) => Some(text_editor::Binding::Custom(UiMessage::SubmitComposer)),
                        _ => Some(text_editor::Binding::Enter),
                    }
                }
                Key::Character("`") if !k.modifiers.command() => {
                    Some(text_editor::Binding::Custom(UiMessage::Format(format::CODE)))
                }
                Key::Named(key @ (Named::Backspace | Named::Delete))
                    if k.modifiers.jump() || k.modifiers.macos_command() =>
                {
                    use text_editor::Motion;
                    let back = key == Named::Backspace;
                    let to = match (back, k.modifiers.macos_command()) {
                        (true, false) => Motion::WordLeft,
                        (false, false) => Motion::WordRight,
                        (true, true) => Motion::Home,
                        (false, true) => Motion::End,
                    };
                    Some(text_editor::Binding::Custom(UiMessage::DeleteTo(to, back)))
                }
                Key::Named(Named::ArrowUp | Named::ArrowDown) if suggesting => None,
                Key::Named(Named::Escape) => None,
                Key::Character(_) if k.modifiers.command() => {
                    let i = match (k.key.to_latin(k.physical_key).map(|c| c.to_ascii_lowercase()), k.modifiers.shift())
                    {
                        (Some('z'), redo) => return Some(text_editor::Binding::Custom(UiMessage::Undo(redo))),
                        (Some('y'), false) => return Some(text_editor::Binding::Custom(UiMessage::Undo(true))),
                        (Some('b'), false) => 0,
                        (Some('i'), false) => 1,
                        (Some('x'), true) => 2,
                        _ => return text_editor::Binding::from_key_press(k),
                    };
                    Some(text_editor::Binding::Custom(UiMessage::Format(i)))
                }
                _ => text_editor::Binding::from_key_press(k),
            })
            .size(13)
            .padding([9, 2])
            .height(Shrink)
            .max_height(self.pane().size.height / 2.0)
            .style(move |_, _| text_editor::Style {
                background: CLEAR.into(),
                border: iced::Border::default(),
                placeholder: p.subtle,
                value: p.strong,
                selection: p.accent.scale_alpha(0.45),
            });
        let empty = self.pane().composer.is_empty();
        let net = (st.settings.on("showNetworkIcons") && empty)
            .then(|| svg(brand_svg(brand_key(&chat.account_id))).width(14).height(14));
        let inline = Row::with_children([pri, sec].into_iter().flatten().map(&action_btn)).spacing(2).align_y(Center);
        let net = Row::new().push(net.map(|n| container(n).padding(Padding::ZERO.right(2.0)).center_y(36)));
        let inline = container(inline).padding(Padding::ZERO.left(2.0)).center_y(36);
        let input = self.with_formatting(input.into(), if st.settings.on("formattingMenu") { formats } else { 0 });
        let line = row![net, input, inline].align_y(Bottom).padding(Padding::new(0.0).left(14.0).right(6.5));
        let pill: El<'a> = match self.pane().send_later {
            None => container(line).width(Fill).style(fill_box(p.input, 18.0)).into(),
            Some((at, no_reply)) => {
                let when = send_day(at, st.settings.pref("timeFormat") == 1);
                let when = if no_reply { trf("%s if no reply", &[&when]) } else { when };
                let details = row![
                    tinted(&COMPOSER_ICONS[6], 20.0, p.accent),
                    semi(when, 11.0).line_height(Pixels(14.0)).color(p.accent),
                    tinted(&CHEVRON_DOWN, 14.0, p.accent)
                ];
                let details = button(details.spacing(4).align_y(Center))
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                    .on_press(UiMessage::OpenSendLater);
                let close = icon_btn(p, icon::CLOSE, 14.0, 20.0, p.accent, UiMessage::SendLater(None));
                let blue = if p.card.r < 0.5 { hex(0x1b43aa) } else { hex(0xcbd9ff) };
                let banner = container(row![details, Space::new().width(Fill), close].spacing(2).align_y(Center))
                    .padding(2)
                    .style(fill_box(blue, 20.0));
                let body = column![container(banner).padding(Padding::new(8.0).bottom(0.0).left(12.0)), line];
                let body = container(body).width(Fill).style(fill_box(p.card, 18.0));
                stack![body, dashed::dashes(hex(0x808080), 18.0)].into()
            }
        };
        let plus_icon = tinted(&COMPOSER_ICONS[usize::from(self.pane().show_attach_menu)], 20.0, p.muted);
        let toggle = button(container(plus_icon).center(36))
            .padding(0)
            .style(btn_style([p.input, p.hover, p.selected], [p.text; 3], 18.0))
            .on_press(UiMessage::ToggleAttachMenu);
        let unveiled = self.pane().show_attach_menu.then(|| {
            Row::with_children(hidden_actions(st).into_iter().map(&action_btn))
                .spacing(2)
                .padding([0, 1])
                .align_y(Center)
        });
        let plus = row![toggle].push(unveiled).spacing(6).align_y(Center);
        let ready = !self.pane().composer.text().trim().is_empty()
            || !self.pane().attachments.is_empty()
            || self.recorder.is_some();
        let pending = (!self.pane().attachments.is_empty()).then(|| {
            let items = self.pane().attachments.iter().enumerate().map(|(i, pm)| {
                let preview: El<'a> = if pm.frames.is_empty() {
                    let ext = std::path::Path::new(&pm.name).extension().map(|e| e.to_string_lossy().to_uppercase());
                    let meta = ext.into_iter().chain([human_size(pm.bytes.len() as u64)]).collect::<Vec<_>>();
                    let name = clamp(&pm.name, 190.0, 2, 11.0, Weight::Semibold).line_height(Pixels(14.0));
                    let info = column![name, txt(meta.join(" ・ "), 11.0).line_height(Pixels(14.0))].spacing(4);
                    let icon = &FILE_ICONS[if pm.mime.starts_with("audio") { 3 } else { 0 }];
                    let line = row![tinted(icon, 20.0, p.strong), info].spacing(8).align_y(Center);
                    container(column![line, Space::new().width(158)])
                        .max_width(240)
                        .padding([8, 11])
                        .style(fill_box(p.received, 17.0))
                        .into()
                } else {
                    let fg = Color { a: 0.1, ..p.text };
                    let fit = |f| image(f).height(66).content_fit(iced::ContentFit::Contain);
                    let img = container(anim::gif(&pm.frames, self.started, fit)).max_width(120).clip(true);
                    img.style(bordered_box(CLEAR, fg, 1.0, 8.0)).into()
                };
                removable(p, preview, UiMessage::DiscardAttachment(i))
            });
            Row::with_children(items).spacing(8).align_y(iced::Bottom).padding([6, 54])
        });
        let send = button(container(tinted(&SEND_ARROW, 20.0, Color::WHITE)).center(36))
            .padding(0)
            .style(btn_style([if ready { p.accent } else { p.send_btn }, p.accent, p.accent], [p.text; 3], 18.0))
            .on_press(UiMessage::SubmitComposer);
        let text = self.pane().composer.text();
        let urls = text_urls(&text);
        let links =
            self.pane().composer_links.iter().filter(|(u, _)| urls.contains(&u.as_str())).filter_map(|(u, s)| {
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
        // Desktop lists no sticker packs on Matrix accounts: its Matrix API has no `getStickerPacks`.
        let stickers = (self.pane().picker == Some(Picker::Sticker))
            .then(|| container(txt(tr("No sticker packs found"), 14.0).color(p.text)).center_x(Fill).center_y(200));
        let extras = Column::new().push(banner).push(gif_picker).push(stickers).push(pending).push(links);
        let line: Row<'a, UiMessage> = match &self.recorder {
            Some(r) => row![self.view_recorder(r), send],
            None => row![plus, pill, send],
        };
        column![extras, line.spacing(8).align_y(Center).padding(Padding::new(8.0).top(4.0))].into()
    }

    fn view_info(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.clone().expect("a chat is selected");
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
                if chat.is_pinned == Some(true) { tr("Unpin") } else { tr("Pin") },
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Pinned)
            ),
            action(
                icon::NOTIFICATIONS_OFF,
                if chat.is_muted == Some(true) { tr("Unmute") } else { tr("Mute") },
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Muted)
            ),
            action(
                icon::ARCHIVE,
                if chat.is_archived == Some(true) {
                    tr("Unarchive")
                } else {
                    trx("Archive", "button label to archive chat")
                },
                UiMessage::ToggleFlag(cid.clone(), ChatFlag::Archived)
            ),
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
                    self.face(u.img_url.as_deref(), &name, 28.0, 0.0),
                    column![txt(name, 13.0).color(p.text)]
                        .push(u.phone_number.clone().map(|ph| txt(ph, 11.0).color(p.sub)))
                ]
                .spacing(10)
                .align_y(Center),
            )
        });
        let edit = button(semi(trx("Edit", "chat info edit button"), 12.0).color(p.accent))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
            .on_press(UiMessage::OpenChatTheme(cid.clone()));
        let body = column![
            closable_head(p, tr("Chat Info"), 13.0, UiMessage::ToggleRightPanel),
            stack![
                column![
                    self.chat_avatar(chat, 64.0, 1.25),
                    semi(chat.title.clone(), 15.0).color(p.strong),
                    txt(network_name(&chat.account_id), 12.0).color(p.sub)
                ]
                .spacing(6)
                .align_x(Center)
                .width(Fill),
                container(edit).align_right(Fill),
            ],
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
        let st = self.store.read();
        let room = self.search_room.as_deref();
        let placeholder = match (self.cmd_bar_commands_mode, room) {
            _ if self.cmd_submenu.is_some() => self.cmd_submenu.as_ref().map(|m| m.placeholder()).unwrap_or_default(),
            (true, _) => tr("Search commands").into(),
            (false, Some(_)) => tr("Search this chat").into(),
            (false, None) => tr("Search chats").into(),
        };
        let input = text_input(&placeholder, &self.command_palette_query)
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
                txt(if self.cmd_bar_commands_mode { tr("Chat") } else { tr("Commands") }, 13.0).color(p.sub),
                kbd("Ctrl"),
                kbd(if self.cmd_bar_commands_mode { "K" } else { "J" })
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([2, 6])
        .style(ghost(p, 8.0))
        .on_press(UiMessage::ToggleCommandMode);
        let schedule = matches!(self.cmd_submenu, Some(commands::SubMenu::Schedule(_)));
        let no_reply = [tr("If no reply"), if schedule { tr("Regardless") } else { tr("Always") }];
        let inputs = match &self.cmd_submenu {
            Some(commands::SubMenu::Remind(_) | commands::SubMenu::Schedule(_)) => {
                let pick = move |c: &str| UiMessage::CmdNoReply(c == no_reply[0]);
                let pad = Padding::new(4.0).left(6.0).right(22.0);
                row![input, sized_select(p, no_reply, no_reply[usize::from(!self.cmd_no_reply)], pick, 13.0, pad)]
            }
            Some(_) => row![input],
            None => row![ic(icon::SEARCH, 14.0).color(p.sub), input, mode_btn],
        };
        let inputs = inputs.spacing(8).align_y(Center).padding(Padding::new(6.0).left(13.0).right(8.0));
        let q = self.command_palette_query.trim().to_lowercase();
        if self.cmd_bar_commands_mode {
            let cmds = self.commands(&st, self.command_palette_query.trim());
            let header = self.cmd_submenu.as_ref().filter(|m| !m.heading().is_empty()).map(|m| {
                let heading = txt(m.heading(), 13.0).line_height(Pixels(16.0)).color(p.strong);
                let clock = schedule.then(|| tinted(&COMPOSER_ICONS[6], 20.0, p.strong));
                container(row![].push(clock).push(heading).spacing(4).align_y(Center))
                    .width(Fill)
                    .center_y(44)
                    .padding([8, 13])
                    .style(fill_box(p.chip, iced::border::top(18)))
            });
            let footer = (schedule || matches!(self.cmd_submenu, Some(commands::SubMenu::Remind(_)))).then(|| {
                let note = match (schedule, self.cmd_no_reply) {
                    (true, _) => trf(
                        "Send Later messages can only send if %s app is running (and your computer is awake)",
                        &[&"Beeper"],
                    ),
                    (_, true) => {
                        tr("Remind Me marks this chat as new at the scheduled time, if there is no reply.").into()
                    }
                    (_, false) => tr("Remind Me marks this chat as new at the scheduled time").into(),
                };
                let line =
                    container(Space::new()).width(Fill).height(2).style(fill_box(p.strong.scale_alpha(0.1), 0.0));
                let note =
                    container(txt(note, 13.0).line_height(Pixels(16.0)).color(p.sub)).center_y(44).padding([8, 13]);
                column![line, container(note).width(Fill).style(fill_box(p.chip, iced::border::bottom(18)))]
            });
            let body = column![].push(header).push(inputs).push(self.view_commands(&st, &cmds));
            return frame(body.push(footer));
        }
        let hint =
            self.pane().selected_chat_id.as_deref().and_then(|cid| st.resolve_chat_idx(cid).map(|i| &st.chats[i])).map(
                |c| {
                    let prep = if c.chat_type == "group" { "in" } else { "with" };
                    let label = match room {
                        Some(_) => "Press Ctrl K to search all of your conversations".into(),
                        None => format!("Press Ctrl F to search your conversation {prep} {}", c.title),
                    };
                    row![tinted(&DROP_ARROW, 20.0, p.sub), txt(label, 13.0).color(p.text)]
                        .spacing(16)
                        .align_y(Center)
                        .padding([8, 24])
                },
            );
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
                semi(label.unwrap_or_else(|| tr(placeholder).into()), 13.0).color(fg),
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
            chats.retain(|c| self.pane().selected_chat_id.as_deref() != Some(c.id.as_str()));
        }
        let has_more = chats.len() > 5;
        let limit = if self.search_expanded { 12 } else { 5 };
        let cat = row![
            txt(if q.is_empty() { tr("Recent chats") } else { tr("Chats") }, 12.0).color(p.sub),
            Space::new().width(Fill)
        ]
        .push(has_more.then(|| {
            button(semi(if self.search_expanded { tr("Show less") } else { tr("Show more") }, 11.0).color(p.accent))
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
            found.iter().fold(Column::new().push((!found.is_empty()).then(|| heading(tr("Messages")))), |col, m| {
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
                let (bg, fg) = if mine { (p.sent, p.sent_fg) } else { (p.received, p.strong) };
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
                    let name = a.file_name.clone().unwrap_or_else(|| tr("Attachment").into());
                    card(name, semi(kind, 10.0).line_height(Pixels(12.0)).color(p.text).into())
                }
                (None, None, None) => {
                    let text = m.text.clone().expect("a link message has text");
                    card(text, Space::new().into())
                }
            };
            let face = container(self.chat_avatar(c, 24.0, 0.0)).padding(2).style(bordered_box(
                CLEAR,
                Color::WHITE,
                2.0,
                14.0,
            ));
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
        let media_grid = (!media.is_empty()).then(|| column![heading(tr("Media")), grid]);
        let empty = none.then(|| {
            let desc = "Only messages loaded in Beeper can be searched.\nFor more results, scroll through older conversations.";
            column![
                container(semi(tr("No messages matching those terms"), 16.0).color(p.strong)).center(Fill),
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
                self.chat_avatar(c, 28.0, 0.0),
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
                let chat = &st.messages[st.msg_idx_by_id[&c.mid]].chat_id;
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
        let (items, at) = self.carousel_items(&self.store.read());
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
        let st = self.store.read();
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
                (u.map(|u| self.face(u.img_url.as_deref(), name, 28.0, 0.0)), name)
            }
            false => (Some(self.chat_avatar(c, 28.0, 0.0)), c.title.as_str()),
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
            .push(control(&ROW_ACTIONS[6], UiMessage::CloseCarousel))
            .spacing(8);
        let head =
            stack![container(title).center_x(Fill).center_y(34), container(controls).align_right(Fill).center_y(34)];
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        let turn = iced::Radians(-std::f32::consts::FRAC_PI_2 * f32::from(car.turns));
        let shown: El<'a> = match url.and_then(|u| self.media.get(u)?.as_ref()) {
            Some(Ok(frames)) => anim::gif(frames, self.started, move |f| {
                image(f).content_fit(iced::ContentFit::ScaleDown).rotation(iced::Rotation::Solid(turn))
            }),
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
        let banner = match self.relaunch_toast {
            Some(t) => tr(t),
            None => &self.status_banner,
        };
        (!banner.is_empty()).then(|| {
            let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, p.sub, UiMessage::DismissBanner);
            let text = txt(banner.to_string(), 13.0).color(p.strong);
            let action = match (self.relaunch_toast, &self.banner_action) {
                (Some(_), _) => Some((tr("Relaunch app"), UiMessage::Relaunch)),
                (None, Some((text, label, msg))) if text == banner => Some((*label, msg.clone())),
                _ => None,
            };
            let relaunch = action.map(|(label, msg)| pill_btn(p, label, msg));
            let card = row![text.wrapping(iced::widget::text::Wrapping::WordOrGlyph)].push(relaunch).push(close);
            let card = card.spacing(8).align_y(Center);
            let card = container(card).padding(8).style(shadow_box(p.card, p.outline, 10.0, 15.0));
            container(card).width(Fill).align_bottom(Fill).padding(Padding::new(8.0).left(left)).into()
        })
    }

    fn view_media<'a>(&'a self, w: &MediaWindow) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
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
            let label = txt(tr(label), 12.0).line_height(Pixels(15.0)).color(p.strong);
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
                tr("Select"),
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(tune_icon, 18.0, false, UiMessage::MediaMenu(true)),
                tr("Filter"),
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(search_icon, 18.0, w.search.is_some(), search),
                tr("Search"),
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
            let input = text_input(tr("Search by caption"), q)
                .id("media-search")
                .on_input(|q| UiMessage::MediaSearch(Some(q)))
                .size(14)
                .padding(0)
                .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
            let close = self.tip(
                control(&ROW_ACTIONS[6], 16.0, false, UiMessage::MediaSearch(None)),
                tr("Close"),
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
            let full = |d| i18n::date(i18n::Date::Medium, civil_from_days(d), (0, 0));
            let range = if lo == hi { full(lo) } else { format!("{} - {}", full(lo), full(hi)) };
            let head = column![bold(label.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong), sub(range)];
            let col = col.push(container(head).padding(Padding::new(6.0).top(12.0).left(10.0).right(10.0)));
            match w.tab {
                0 => group.chunks(cols).fold(col, |col, chunk| {
                    let tiles = chunk.iter().map(|&(i, k)| -> El<'a> {
                        let m = &st.messages[i];
                        let a = &m.attachments[k];
                        let badge = match (a.is_gif == Some(true), a.attachment_type == "video") {
                            (true, _) => Some(row![semi(tr("GIF"), 12.0)]),
                            (false, true) => {
                                let time = a.duration.map(|d| semi(voice::clock(d as f32), 12.0));
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
                            let name = a.file_name.clone().unwrap_or_else(|| tr("Attachment").into());
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
            true => container(txt(tr("Nothing here"), 14.0).color(p.on_secondary)).center_x(Fill).padding(16).into(),
            false => scrollable(body.push(Space::new().height(8)))
                .direction(bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .on_scroll(
                    move |v| if more && v.relative_offset().y > 0.8 { UiMessage::MediaMore } else { UiMessage::Noop },
                )
                .into(),
        };
        let title = container(semi(tr("Media"), 14.0).color(p.strong)).center_x(Fill).padding([8, 0]);
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
                act(tr("Delete"), (n > 0).then(|| UiMessage::MediaAskDelete(picked))),
                act(tr("Save"), (n > 0 && w.tab != 1).then_some(UiMessage::MediaSave)),
                act(tr("Select all"), Some(UiMessage::MediaPickAll)),
                act(tr("Cancel"), Some(UiMessage::MediaSelect(false))),
            ]
            .spacing(4)
            .align_y(Center);
            column![rule(), container(bar).padding([8, 10]).center_y(44)]
        });
        let pane = column![header, rule()].push(search_row).push(list).push(footer);
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.card, 12.0));
        let base = container(column![title, pane]).padding(8).width(Fill).height(Fill).style(fill_box(p.window, 0.0));
        let item_menu = w.item_menu.as_ref().map(|(mid, k, at)| {
            let m = &st.messages[st.msg_idx_by_id[mid]];
            let act =
                |label: &str, msg| menu_item(p, None, label.into(), "", Some(UiMessage::MediaItemAct(Box::new(msg))));
            let go = act(tr("Go to Message"), UiMessage::GoToMessage(m.chat_id.clone(), mid.clone()));
            let mut actions = match w.tab {
                1 => vec![act(tr("Copy Link"), UiMessage::CopyLink(m.links[*k].url.clone()))],
                _ => std::iter::once(act(tr("Download"), UiMessage::Download(mid.clone(), Some(*k))))
                    .chain(
                        (m.attachments[*k].attachment_type == "img")
                            .then(|| act(tr("Copy Image"), UiMessage::CopyImage(mid.clone(), *k))),
                    )
                    .collect(),
            };
            actions
                .extend(deletable(m).then(|| act(tr("Delete…"), UiMessage::MediaAskDelete(vec![(mid.clone(), *k)]))));
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
                heading(tr("Sent By")),
                radio(tr("All"), w.sent_by == 0, UiMessage::MediaSentBy(0)),
                radio(tr("You"), w.sent_by == 1, UiMessage::MediaSentBy(1)),
                radio(tr("Others"), w.sent_by == 2, UiMessage::MediaSentBy(2)),
                menu_rule(p),
                heading(tr("Sort by")),
                radio(tr("Newest"), !w.oldest, UiMessage::MediaOldest(false)),
                radio(tr("Oldest"), w.oldest, UiMessage::MediaOldest(true)),
            ];
            let menu = container(col.padding([4, 0])).width(180).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let x = (w.size.width - 8.0 - 10.0 - 60.0).min(w.size.width - 8.0 - 180.0);
            let at = Padding::ZERO.top(8.0 + 33.0 + 44.0).left(x);
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MediaMenu(false))
        });
        drop(st);
        let carousel = self.carousel.as_ref().filter(|c| c.in_media).map(|c| self.view_carousel(c));
        let ask = w.deleting.as_ref().map(|(pairs, _)| {
            let n = pairs.len();
            let (title, body) = match n {
                1 => {
                    (tr("Delete message").to_string(), tr("Are you sure you want to delete this message?").to_string())
                }
                _ => ("Delete messages".to_string(), format!("Are you sure you want to delete {n} messages?")),
            };
            self.dialog(
                title,
                body,
                vec![(tr("Delete"), UiMessage::MediaDelete(true))],
                (tr("Cancel"), UiMessage::MediaDelete(false)),
            )
        });
        stack![base].push(self.view_toast(8.0)).push(menu).push(item_menu).push(ask).push(carousel).into()
    }

    fn view_forward<'a>(&'a self, f: &'a Forward) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let shade = Color { a: 0.1, ..p.strong };
        let strip = |content: El<'a>, r| container(content).width(Fill).padding([8, 13]).style(fill_box(p.chip, r));
        let head = row![tinted(&MAT_FORWARD, 16.0, p.strong), txt(tr("Forward message"), 14.0).color(p.strong)];
        let head = strip(head.spacing(12).align_y(Center).into(), border::Radius::default().top(18)).center_y(44);
        let pills = f.picked.iter().map(|cid| -> El<'a> {
            let c = &st.chats[st.chat_idx_by_id[cid]];
            let label = semi(c.title.clone(), 14.0).line_height(Pixels(16.0)).color(p.strong);
            let pill = row![self.chat_avatar(c, 16.0, 0.0), label].spacing(4).align_y(Center);
            container(pill).padding(Padding::new(2.0).right(8.0)).style(fill_box(shade, 20.0)).into()
        });
        let input = text_input(tr("Search name, @, or number"), &f.query)
            .id("forward-input")
            .on_input(UiMessage::ForwardQuery)
            .size(14)
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let inputs = Row::with_children(pills).push(input).spacing(4).align_y(Center).height(24);
        let inputs = container(inputs).padding(Padding::new(18.0).left(13.0).right(20.0));
        let chats = forward_chats(&st, f);
        let cat = (f.query.trim().is_empty() && !chats.is_empty()).then(|| {
            container(txt(tr("Recent chats"), 12.0).color(p.sub))
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
            let line = row![self.chat_avatar(c, 32.0, 0.0), title, Space::new().width(Fill)]
                .push(svg(brand_svg(brand_key(&c.account_id))).width(16).height(16))
                .spacing(8)
                .align_y(Center);
            let bg = if i == 0 { p.chip } else { CLEAR };
            let pick = button(line).height(36).padding([0, 6]).width(Fill);
            let pick = pick.style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0));
            col.push(container(pick.on_press(UiMessage::ForwardPick(c.id.clone()))).padding([0, 6]))
        });
        let ready = !f.picked.is_empty();
        let label = row![semi(tr("Forward"), 12.0), txt("CTRL ENTER", 11.0).color(Color { a: 0.8, ..Color::WHITE })];
        let send = button(label.spacing(4).align_y(Center))
            .padding([6, 8])
            .style(btn_style([if ready { p.accent } else { shade }; 3], [Color::WHITE; 3], 100.0))
            .on_press_maybe(ready.then_some(UiMessage::ForwardSend));
        let note = tr("Forwarding messages through Beeper will not include any attribution on the native apps.");
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
        let st = self.store.read();
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
                row![
                    tinted(&SYS_ARROW_BACK, 18.0, p.strong),
                    txt(tr("Back"), 12.0).line_height(Pixels(15.0)).color(p.strong)
                ]
                .spacing(4)
                .align_y(Center),
            )
            .padding([4, 6])
            .style(ghost(p, 6.0))
            .on_press(UiMessage::ToggleJoinMatrix);
            let head =
                container(row![back, txt(tr("Join a Matrix room"), 13.0).color(p.strong)].spacing(12).align_y(Center))
                    .width(Fill)
                    .padding([8, 13])
                    .style(fill_box(p.chip, 0.0));
            let inputs = row![tinted(&MAT_LINK, 18.0, p.sub), field(tr("Room address or Matrix link"))]
                .spacing(8)
                .align_y(Center)
                .padding(Padding::new(6.0).left(13.0).right(8.0));
            let body = column![head, inputs, Space::new().height(Fill), footer(tr("Join"), "ENTER")];
            return container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0)).into();
        }
        let all_acc = container(
            row![med(tr("All accounts"), 12.0).color(p.strong), ic(icon::EXPAND_MORE, 14.0).color(p.strong)]
                .spacing(4)
                .align_y(Center),
        )
        .center_y(24)
        .padding(Padding::new(0.0).left(8.0).right(6.0))
        .style(fill_box(p.chip, 12.0));
        let inputs = row![ic(icon::SEARCH, 16.0).color(p.sub), field(tr("Search contacts")), all_acc]
            .spacing(8)
            .align_y(Center)
            .padding(Padding::new(6.0).top(10.0).left(13.0).right(8.0));
        let q = self.new_chat_query.trim().to_lowercase();
        let mut col = Column::new().padding(Padding::ZERO.top(3.0));
        if q.is_empty() {
            let group_btn = button(
                row![ic(icon::GROUP, 20.0).color(p.accent), semi(tr("New Group"), 13.0).color(p.strong)]
                    .spacing(7)
                    .align_y(Center),
            )
            .width(Fill)
            .height(36)
            .padding([0, 11])
            .style(btn_style([p.chip, p.chip, p.selected], [p.strong; 3], 6.0))
            .on_press(UiMessage::Noop);
            let join_btn = button(
                row![tinted(&MAT_LINK, 20.0, p.accent), semi(tr("Join a Matrix room"), 13.0).color(p.strong)]
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
                container(txt(tr("In your contacts"), 12.0).color(p.sub))
                    .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0)),
            );
            let limit = if self.new_chat_expanded || !q.is_empty() { contacts.len() } else { 10.min(contacts.len()) };
            let mut last_cat = None;
            for u in contacts.iter().take(limit) {
                let name = u.full_name.as_deref().unwrap_or(&u.id);
                let ch = name.chars().next().unwrap_or(' ').to_ascii_uppercase();
                let cat = if ch.is_ascii_alphabetic() { ch.to_string() } else { tr("Others").to_string() };
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
                    self.face(u.img_url.as_deref(), name, 28.0, 0.0),
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
                let label = if self.new_chat_expanded { tr("Show less contacts") } else { tr("Show more contacts") };
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
            footer(tr("Start chat"), "CTRL ENTER")
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
        let search = container(
            row![ic(icon::SEARCH, 16.0).color(p.text), search_input]
                .push(clear)
                .spacing(8)
                .align_y(Center)
                .padding([6, 8]),
        )
        .style(fill_box(p.search, 38.0));
        // The search results draw the icons at Desktop's 11 px instead of 13 px.
        let tile = |tab: SettingsTab, small: bool, alpha: f32| {
            let (_, glyph, rgb) = tab.meta();
            let em = if small { 11.0 } else { 13.0 };
            let glyph: El = match glyph {
                TabIcon::Glyph(g) => ic(g, em).color(Color::WHITE.scale_alpha(alpha)).into(),
                TabIcon::Svg(doc) => {
                    let (w, h) = svg_size(doc);
                    svg(svg::Handle::from_memory(doc)).width(w * em / 13.0).height(h * em / 13.0).opacity(alpha).into()
                }
            };
            let fill = Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8).scale_alpha(alpha);
            let (size, radius) = if small { (16, 4.0) } else { (20, 6.0) };
            container(glyph).center(size).style(fill_box(fill, radius))
        };
        let free = container(semi("Free", 10.0).color(p.strong)).padding([4, 6]).style(fill_box(p.chip, 25.0));
        // Desktop's 34 px avatar shrinks to 28.6 px beside the full width details.
        let av = 28.6;
        let detail_w = 231.0 - av - 16.0 - text_width("Free", 10.0, Weight::Semibold) - 12.0;
        let profile = row![
            container(self.face(me.img_url.as_deref(), &name, av, 0.0)).height(34),
            column![ell(&name, detail_w, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(p.strong)]
                .push(
                    me.email
                        .as_deref()
                        .map(|e| ell(e, detail_w, 10.0, Weight::Normal).line_height(Pixels(12.0)).color(p.strong))
                )
                .width(Fill),
            free
        ]
        .spacing(8)
        .align_y(Center)
        .padding([6, 8]);
        let nav = SettingsTab::ALL.iter().fold(Column::new(), |c, &tab| {
            let on = tab == self.active_settings_tab;
            let count = (tab == SettingsTab::Accounts).then(|| {
                container(txt(st.accounts.len().to_string(), 11.0).color(p.strong))
                    .center(18)
                    .style(fill_box(p.chip, 9.0))
            });
            let line = row![tile(tab, false, 1.0), txt(tab.name(), 14.0).color(p.strong), Space::new().width(Fill)]
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
                    .style(btn_style([if on { p.rail } else { CLEAR }, p.hover, p.rail], [p.text; 3], 8.0))
                    .on_press(UiMessage::SelectSettingsTab(tab)),
            )
            .push((!on).then(|| {
                let rule = if next_on { CLEAR } else { p.divider };
                container(container(Space::new()).width(Fill).height(1).style(fill_box(rule, 0.0)))
                    .padding(Padding::ZERO.left(36.0))
            }))
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
            true => column![profile, nav].into(),
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
        .style(fill_box(p.prefs_pane, 10.0));
        let base = container(row![left, container(right).padding(8)]).width(Fill).height(Fill);
        drop(st);
        let confirm = self.confirm.filter(|d| SETTINGS_DIALOGS.contains(d)).map(|d| self.view_confirm(d));
        let add = self.integrations.add.as_ref().map(|a| self.view_add_token(a));
        stack![base.style(fill_box(p.window, 0.0))].push(add).push(confirm).into()
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
                        pill_btn(p, tr("They don't match"), UiMessage::SasMatch(false)),
                        pill_btn(p, tr("They match"), UiMessage::SasMatch(true))
                    ]
                    .spacing(8);
                    let title = txt("Compare the emojis on both devices", 13.0).color(p.text);
                    let buttons = row![Space::new().width(Fill), buttons];
                    return column![title, Row::with_children(emojis), buttons].spacing(12).padding([13, 14]).into();
                }
                _ => (None, pill_btn(p, tr("Verify"), UiMessage::VerifySasNow)),
            };
        pref_row(p, "Verify with another device", desc, None, control)
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
            let logout = button(semi(tr("Logout"), 12.0).color(p.strong))
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
                let title = trf("Logout of %s?", &[&name.as_deref().unwrap_or(tr("this device"))]);
                (
                    title,
                    tr("This will disconnect all chat accounts on this device.").into(),
                    vec![(tr("Logout"), UiMessage::Logout)],
                )
            }
            ASK_DISMISS_REMINDERS => (
                tr("Dismiss All Reminders?").into(),
                tr("This will remove Reminders from all of your chats.").into(),
                vec![(tr("Yes"), UiMessage::DismissAllReminders)],
            ),
            ASK_DELETE_CHAT => {
                let st = self.store.read();
                let id = self.delete_target.clone().expect("a chat to delete");
                let c = &st.chats[st.resolve_chat_idx(&id).expect("the chat to delete exists")];
                let delete = |label, everyone| (label, UiMessage::DeleteChat(id.clone(), everyone));
                let (body, actions) = match c.delete_modes {
                    DELETE_FOR_ME => {
                        (tr("This action will permanently delete this chat."), vec![delete(tr("Delete Chat"), false)])
                    }
                    DELETE_FOR_EVERYONE => (
                        tr("This action will permanently delete this chat for all participants."),
                        vec![delete(tr("Delete for Everyone"), true)],
                    ),
                    _ => (
                        tr("Choose to delete permanently for yourself only, or for all participants."),
                        vec![delete(tr("Delete for Me"), false), delete(tr("Delete for Everyone"), true)],
                    ),
                };
                (format!("Delete {}", c.title), body.into(), actions)
            }
            ASK_DELETE_MESSAGE => {
                let mid = self.delete_target.clone().expect("a message to delete");
                let st = self.store.read();
                let cid = st.messages[st.msg_idx_by_id[&mid]].chat_id.clone();
                let delete = vec![(tr("Delete"), UiMessage::DeleteMessage(cid, mid))];
                (tr("Delete message").into(), tr("Are you sure you want to delete this message?").into(), delete)
            }
            d @ (ASK_ARCHIVE_READ | ASK_ARCHIVE_ALL) => {
                let read = d == ASK_ARCHIVE_READ;
                let mask = CHAT_ARCHIVED | CHAT_PINNED | if read { CHAT_UNREAD } else { 0 };
                let st = self.store.read();
                let n = self.visible_chats(&st).iter().filter(|c| c.flags & mask == 0).count();
                let body = trn(
                    "This will archive %d chat and any older chats from this point in time",
                    "This will archive %d chats and any older chats from this point in time",
                    n,
                    &[&n],
                );
                let archive = vec![(trx("Archive", "archive confirmation button"), UiMessage::ArchiveAll(read, 0))];
                (tr(if read { "Archive all read chats" } else { "Archive all chats" }).into(), body, archive)
            }
            ASK_ARCHIVE_WEEK => (
                tr("Archive all chats older than a week").into(),
                tr("This will also archive any other older chats from a week ago").into(),
                vec![(trx("Archive", "archive confirmation button"), UiMessage::ArchiveAll(false, 7))],
            ),
            ASK_MARK_ALL_READ => {
                let n = self.visible_chats(&self.store.read()).iter().filter(|c| c.flags & CHAT_UNREAD != 0).count();
                let title = trn("Mark %d thread as read", "Mark %d threads as read", n, &[&n]);
                let body = tr("Are you sure you want to mark all chats as read?").into();
                (title, body, vec![(tr("Mark as Read"), UiMessage::MarkAllRead)])
            }
            ASK_CLEAR_STORAGE | ASK_CLEAR_CACHE | ASK_CLEAR_CHAT => {
                let which = dialog - ASK_CLEAR_STORAGE;
                let size = human_size(self.storage_pick(which).iter().map(|e| e.bytes).sum());
                let clear = |label| vec![(tr(label), UiMessage::ClearStorage(which))];
                match which {
                    0 => {
                        let days = STORAGE_DAYS[self.storage.ages[0]];
                        let title = match days {
                            0 => tr("Clear selected storage?").into(),
                            _ => trn(
                                "Clear storage older than %d day?",
                                "Clear storage older than %d days?",
                                days as usize,
                                &[&days],
                            ),
                        };
                        let body = "This will delete %s of local data. You can recover content by logging out and back into your accounts.";
                        (title, trf(body, &[&size]), clear("Clear Storage"))
                    }
                    1 => {
                        let body = "This will delete %s of cached files including transcoded media, link previews, and unused drafts.";
                        (tr("Clear cache?").into(), trf(body, &[&size]), clear("Clear Cache"))
                    }
                    _ => {
                        let id = self.storage.chat.as_deref().expect("an open chat");
                        let st = self.store.read();
                        let title = st.resolve_chat_idx(id).map_or(id.to_string(), |i| st.chats[i].title.clone());
                        let body = "This will delete %1$s of local data from \"%2$s\". You can recover content by logging out and back into your accounts.";
                        (trf("Clear storage for %s?", &[&title]), trf(body, &[&size, &title]), clear("Clear Storage"))
                    }
                }
            }
            ASK_REVOKE_CONNECTION => (
                tr("Are you sure you want to revoke this connection?").into(),
                tr("Connected clients will lose access to Beeper Desktop API").into(),
                vec![(tr("Revoke"), UiMessage::RevokeConfirmed)],
            ),
            ASK_REMOTE_ACCESS => (
                tr("Enable Remote Access?").into(),
                tr("This can expose the Beeper Desktop API to the public internet based on your firewall settings. Only enable if you know what you are doing.").into(),
                vec![(tr("Allow Remote Access"), UiMessage::RemoteAccess(true))],
            ),
            _ => unreachable!("dialog {dialog}"),
        };
        self.dialog(title, body, actions, (tr("Cancel"), UiMessage::Ask(None)))
    }

    /// A confirmation dialog over a window, with its actions after the cancel button.
    fn dialog<'a>(
        &'a self,
        title: String,
        body: String,
        actions: Vec<(&'static str, UiMessage)>,
        (no, cancel): (&'static str, UiMessage),
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
            row![Space::new().width(Fill), pill_btn(p, no, cancel.clone())]
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
                    let line = row![
                        container(svg(brand_svg(brand_key(logo))).width(28).height(28)).center(46),
                        column![title].push(sub.map(|s| txt(s, 13.0).color(p.sub))).width(Fill),
                        ic(icon::CHEVRON_RIGHT, 16.0).color(p.strong)
                    ]
                    .spacing(8)
                    .align_y(Center)
                    .padding(Padding::new(4.0).left(8.0).right(16.0));
                    button(line)
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                        .on_press(UiMessage::OpenAccount(Some(logo.into())))
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
                .chain(
                    self.adding_account
                        .then(|| group(p, None, vec![login::network_grid(&st.bridge_accounts, "", p.strong)])),
                )
                .collect()
            }
            SettingsTab::General => self.pref_items(prefs::GENERAL, st),
            SettingsTab::Appearance => self.pref_items(prefs::APPEARANCE, st),
            SettingsTab::Notifications => self.pref_items(prefs::NOTIFICATIONS, st),
            SettingsTab::Hotkeys => vec![self.hotkeys_page(st)],
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
                let lists = match &self.devices {
                    None => Vec::new(),
                    Some(Err(e)) => vec![txt(e.clone(), 13.0).color(p.danger).into()],
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
                vec![page.into(), encryption]
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
        let p = &self.pal;
        let segments = STORAGE_DAYS.iter().enumerate().flat_map(|(i, &d)| {
            let label = txt(trn("%d day", "%d days", d as usize, &[&d]), 11.0).line_height(Pixels(14.0)).color(p.text);
            let bg = if i == self.storage.ages[which] { p.chip } else { CLEAR };
            let segment = button(label)
                .padding([2, 8])
                .style(btn_style([bg; 3], [p.text; 3], 20.0))
                .on_press(UiMessage::StorageAge(which, i));
            [(i > 0).then(|| Space::new().width(Fill).into()), Some(segment.into())].into_iter().flatten()
        });
        let bg = if self.dark { Color::BLACK } else { Color::WHITE };
        let control = container(Row::with_children(segments).spacing(4).align_y(Center))
            .padding(if which == 0 { 8 } else { 4 })
            .width(Fill)
            .style(fill_box(bg, 20.0));
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
        let hotkey = |name: &'static str| -> El<'_> {
            let keys = s.hotkey_keys(name);
            let recorder = (keys != "disabled").then(|| {
                let label: El<'_> = match keys {
                    "" => txt(tr("Record Shortcut"), 11.0).color(p.muted).into(),
                    k => txt(prefs::hotkey_label(k), 13.0).line_height(Pixels(18.0)).color(p.text).into(),
                };
                container(label).center_x(160).center_y(20).style(bordered_box(CLEAR, edge, 1.0, 4.0))
            });
            row![ell(tr(name), 226.0, 13.0, Weight::Normal).color(p.text).width(Fill)]
                .push(recorder)
                .push(toggle(p, keys != "disabled", Some(UiMessage::ToggleHotkey(name))))
                .spacing(6)
                .align_y(Center)
                .padding([8, 0])
                .into()
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
    fn view_hotkeys_modal(&self, query: &str) -> El<'_> {
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
        vec![head.into(), card.into(), group(p, Some(tr("Settings")), rows.collect())]
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

    /// Desktop's `--on-wallpaper-fg` of `chat` outside of a reply chain, when its chat theme has a wallpaper.
    fn wallpaper_fg(&self, st: &BeeperStore, chat: &Chat, thread: bool) -> Option<Color> {
        let wallpaper = chat_theme::resolve(&st.settings.chat_appearance, Some(&chat.id), self.dark, &st.data_dir).0;
        wallpaper.filter(|_| !thread).map(|(_, fg)| fg)
    }

    /// The color, text color and metadata color of sent bubbles in `chat`, or in the default chat theme.
    fn sent_colors(&self, st: &BeeperStore, chat: Option<&str>) -> (Color, Color, Color) {
        let p = &self.pal;
        match chat_theme::resolve(&st.settings.chat_appearance, chat, self.dark, &st.data_dir).1 {
            Some((bg, fg)) => (bg, fg, fg.scale_alpha(p.sent_meta.a)),
            None => (p.sent, p.sent_fg, p.sent_meta),
        }
    }

    /// Desktop's `removeOrphanedWallpapers`: deletes the photos in `chat_wallpapers` that no chat theme uses.
    fn prune_wallpapers(&self) {
        let st = self.store.read();
        let entries = match std::fs::read_dir(chat_theme::photo_dir(&st.data_dir)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => panic!("read the chat wallpapers: {e}"),
        };
        let used: HashSet<&str> = st.settings.chat_appearance.photos().collect();
        for entry in entries {
            let path = entry.expect("a chat wallpaper entry").path();
            if path.file_name().and_then(|n| n.to_str()).is_none_or(|n| !used.contains(n)) {
                std::fs::remove_file(&path).expect("remove an unused chat wallpaper");
            }
        }
    }

    /// Desktop's `ChatThemeWindow`: the title and the chat theme picker of a chat.
    fn poll_edit(&mut self, id: window::Id, edit: PollEdit) -> Task<UiMessage> {
        let i = self.poll_windows.iter().position(|d| d.id == id).expect("the poll window is open");
        let st = self.store.read();
        let rules = st.resolve_chat_idx(&self.poll_windows[i].chat).and_then(|c| st.chats[c].polls);
        drop(st);
        let Some(rules) = rules else { return window::close(id) };
        let d = &mut self.poll_windows[i];
        let n = d.options.len();
        match edit {
            PollEdit::Question(q) => d.question = q,
            PollEdit::Option(o, v) if v.is_empty() && d.options[o].1.is_empty() => {
                if n > 2 {
                    d.options.remove(o);
                    return operation::focus(d.input(o.saturating_sub(1)));
                }
            }
            PollEdit::Option(o, v) => {
                let grow = o + 1 == n && !v.trim().is_empty() && rules.room(n);
                d.options[o].1 = v;
                if grow {
                    d.push();
                }
            }
            PollEdit::Next(_) if self.mods.command() => {}
            PollEdit::Next(o) if o + 1 < n => return operation::focus(d.input(o + 1)),
            PollEdit::Next(_) | PollEdit::Add if rules.room(n) => {
                d.push();
                return operation::focus(d.input(n));
            }
            PollEdit::Next(_) | PollEdit::Add => {}
            PollEdit::Remove(o) => {
                if n > 2 {
                    d.options.remove(o);
                    d.hovered = None;
                }
            }
            PollEdit::Hover(o, true) => {
                if let Some(from) = d.dragging.filter(|&g| g != o) {
                    let moved = d.options.remove(from);
                    d.options.insert(o, moved);
                    d.dragging = Some(o);
                }
                d.hovered = Some(o);
            }
            PollEdit::Hover(o, false) => d.hovered = d.hovered.filter(|&h| h != o),
            PollEdit::Grab(o) => d.dragging = o,
            PollEdit::Multiple(on) => d.multiple = on,
            PollEdit::Hidden(on) => d.hidden = on,
            PollEdit::Send if d.ready(rules) => {
                let d = self.poll_windows.remove(i);
                let answers = d.answers();
                self.store.write().send_poll(&d.chat, d.question.trim().into(), answers, d.multiple, d.hidden);
                return window::close(id);
            }
            PollEdit::Send => {}
            PollEdit::Close if d.discarding => d.discarding = false,
            PollEdit::Close if d.dirty() => d.discarding = true,
            PollEdit::Close | PollEdit::Discard(true) => return window::close(id),
            PollEdit::Discard(false) => d.discarding = false,
        }
        Task::none()
    }

    /// Desktop's `PollVotes`: the voters of each option, at most 100 each.
    fn view_poll_votes(&self, chat: &str, mid: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let poll = st.msg_idx_by_id.get(mid).and_then(|&i| st.messages[i].poll.as_ref());
        let Some(poll) = poll else { return container(Space::new()).style(fill_box(p.window, 0.0)).into() };
        let people = st.resolve_chat_idx(chat).map_or(&[][..], |i| &st.chats[i].participants.items[..]);
        let weak = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.05);
        let total = poll.voters();
        let question = if poll.question.is_empty() { tr("Poll").into() } else { poll.question.clone() };
        let mut body = column![semi(question, 17.0).line_height(Pixels(24.0)).color(p.strong)].spacing(10).padding(16);
        for o in &poll.options {
            let n = o.voter_ids.len();
            let head = row![
                semi(o.label.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong).width(Fill),
                txt(n.to_string(), 17.0).line_height(Pixels(24.0)).color(p.sub)
            ]
            .spacing(10)
            .align_y(Center);
            let mut section = column![head];
            if n > 0 {
                let mut voters = Column::new();
                for (i, id) in o.voter_ids.iter().take(100).enumerate() {
                    let u = people.iter().find(|u| u.id == *id);
                    let name = u.and_then(|u| u.full_name.clone()).unwrap_or_else(|| id.clone());
                    let face = self.face(u.and_then(|u| u.img_url.as_deref()), &name, 28.0, 0.5);
                    let line = row![face, txt(name, 14.0).line_height(Pixels(18.0)).color(p.strong)].spacing(8);
                    let last = i + 1 == n.min(100) && n <= 100;
                    voters = voters.push(container(line.align_y(Center)).padding(Padding::new(6.0).bottom(if last {
                        0.0
                    } else {
                        6.0
                    })));
                }
                if n > 100 {
                    let more = trf("+%d more", &[&(n - 100)]);
                    voters =
                        voters.push(container(txt(more, 14.0).line_height(Pixels(18.0)).color(p.sub)).padding([6, 0]));
                }
                let rule = container(Space::new()).width(Fill).height(1).style(fill_box(weak, 0.0));
                section = section.push(Space::new().height(6)).push(rule).push(container(voters).padding([0, 8]));
            }
            body = body.push(container(section).width(Fill).padding(8).style(fill_box(p.received, 12.0)));
        }
        let card = scrollable(body).direction(bar()).height(Fill).style(scroll_style(p.scroll));
        let card = container(card).style(fill_box(p.card, 12.0));
        let title = trn("%d vote", "%d votes", total, &[&total]);
        let title =
            container(semi(title, 13.0).line_height(Pixels(16.0)).color(p.strong)).center_x(Fill).padding([8, 0]);
        container(column![title, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's `PollCreator`.
    fn view_poll_creator<'a>(&'a self, d: &'a PollDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let rules = st.resolve_chat_idx(&d.chat).and_then(|i| st.chats[i].polls).unwrap_or(BEEPER_POLLS);
        drop(st);
        let id = d.id;
        let edit = move |e| UiMessage::Poll(id, e);
        let weak = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.05);
        let rule = || container(Space::new()).width(Fill).height(1).style(fill_box(weak, 0.0));
        let card = |c: Column<'a, UiMessage>| container(c).width(Fill).clip(true).style(fill_box(p.received, 12.0));
        let section = |label: &'static str, items: Vec<El<'a>>| {
            column![semi(tr(label), 12.0).line_height(Pixels(15.0)).color(p.sub)].extend(items).spacing(8)
        };
        let question = text_input(tr("Ask a question…"), &d.question)
            .id(format!("poll-{}", d.chat))
            .on_input(move |q| edit(PollEdit::Question(q.chars().take(500).collect())))
            .size(15)
            .line_height(Pixels(20.0))
            .padding([10, 11])
            .style(input_style(p.received, p.strong, p.sub, p.accent, 10.0));
        let repeats = d.repeats(rules);
        let length = usize::from(rules.max_length);
        let mut options = Column::new();
        for (i, (_, value)) in d.options.iter().enumerate() {
            let fade = if d.dragging == Some(i) { 0.5 } else { 1.0 };
            let grab = if d.dragging.is_some() { mouse::Interaction::Grabbing } else { mouse::Interaction::Grab };
            let handle =
                mouse_area(tinted(&DRAG_HANDLE, 18.0, p.sub.scale_alpha(fade))).on_press(edit(PollEdit::Grab(Some(i))));
            let input = text_input(&trf("Option %d", &[&(i + 1)]), value)
                .id(d.input(i))
                .on_input(move |v| edit(PollEdit::Option(i, v.chars().take(length).collect())))
                .on_submit(edit(PollEdit::Next(i)))
                .size(14)
                .line_height(Pixels(18.0))
                .padding([8, 0])
                .style(input_style(CLEAR, p.strong.scale_alpha(fade), p.sub, p.accent, 0.0));
            let error = repeats[i]
                .then(|| txt(tr("This option already exists"), 11.0).line_height(Pixels(14.0)).color(p.danger));
            let remove = (d.options.len() > 2).then(|| {
                let red = p.danger.scale_alpha(f32::from(u8::from(d.hovered == Some(i))));
                let hover = Color::from_rgba8(0xff, 0x14, 0x16, 0x20 as f32 / 255.0);
                button(container(tinted(&DELETE, 16.0, red)).center(28))
                    .padding(0)
                    .style(btn_style([CLEAR, hover, hover], [red; 3], 14.0))
                    .on_press(edit(PollEdit::Remove(i)))
            });
            let line = row![handle.interaction(grab), column![input].push(error).spacing(1).width(Fill)]
                .push(remove)
                .spacing(8)
                .align_y(Center)
                .padding([6, 10]);
            let line =
                mouse_area(line).on_enter(edit(PollEdit::Hover(i, true))).on_exit(edit(PollEdit::Hover(i, false)));
            options = options.push((i > 0).then(rule)).push(line);
        }
        if rules.room(d.options.len()) {
            let add = row![txt("+", 18.0).line_height(1.0), txt(tr("Add an option"), 14.0).line_height(Pixels(18.0))];
            let add = button(add.spacing(6).align_y(Center))
                .padding(10)
                .width(Fill)
                .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                .on_press(edit(PollEdit::Add));
            options = options.push(rule()).push(add);
        }
        let hint = (rules.max_options > 0).then(|| {
            let left = usize::from(rules.max_options).saturating_sub(d.answers().len());
            let hint = match left {
                0 => tr("You have added the maximum number of options").into(),
                _ => trn("You can enter %d more option", "You can enter %d more options", left, &[&left]),
            };
            txt(hint, 11.0).line_height(Pixels(14.0)).color(p.sub)
        });
        let setting = |label: &'static str, on: bool, msg: fn(bool) -> PollEdit| {
            let line = row![txt(tr(label), 14.0).line_height(Pixels(18.0)).color(p.strong).width(Fill)]
                .push(toggle(p, on, Some(edit(msg(!on)))))
                .spacing(10)
                .align_y(Center)
                .padding([10, 12]);
            mouse_area(line).on_press(edit(msg(!on)))
        };
        let mut settings = column![setting("Allow multiple answers", d.multiple, PollEdit::Multiple)];
        if rules.hidden_votes {
            settings =
                settings.push(rule()).push(setting("Hide results until end of poll", d.hidden, PollEdit::Hidden));
        }
        let body = column![
            section("Question", vec![question.into()]),
            section("Options", [card(options).into()].into_iter().chain(hint.map(El::from)).collect()),
            card(settings)
        ]
        .spacing(14)
        .padding(Padding { top: 10.0, right: 14.0, bottom: 96.0, left: 14.0 });
        let ready = d.ready(rules);
        let bg = if ready { p.accent } else { mix(p.accent, Color::WHITE, 0.55) };
        let send = button(container(semi(tr("Send"), 14.0).line_height(Pixels(18.0))).center_x(Fill))
            .padding(11)
            .width(Fill)
            .style(btn_style([bg; 3], [Color::WHITE; 3], 999.0))
            .on_press_maybe(ready.then(|| edit(PollEdit::Send)));
        let fade = Linear::new(Radians(0.0))
            .add_stop(0.0, p.card)
            .add_stop(0.55, p.card)
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let footer = container(send).padding(Padding { top: 16.0, right: 14.0, bottom: 14.0, left: 14.0 }).style(
            move |_: &Theme| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(fade))),
                ..Default::default()
            },
        );
        let body = scrollable(body).direction(bar()).height(Fill).style(scroll_style(p.scroll));
        let card = container(stack![body, container(footer).align_bottom(Fill)]).style(fill_box(p.card, 12.0));
        let title = container(semi(tr("New Poll"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let window = container(column![title, card]).padding(8).style(fill_box(p.window, 0.0));
        match d.discarding {
            true => stack![
                window,
                self.dialog(
                    tr("Discard poll?").into(),
                    tr("Your poll draft will be deleted.").into(),
                    vec![(tr("Discard"), edit(PollEdit::Discard(true)))],
                    (tr("Keep editing"), edit(PollEdit::Discard(false))),
                )
            ]
            .into(),
            false => window.into(),
        }
    }

    fn view_chat_theme(&self, chat: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let title = container(semi(tr("Chat Theme"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let card = scrollable(self.chat_theme_picker(&st, Some(chat))).direction(bar()).style(scroll_style(p.scroll));
        let card = container(card).height(Fill).style(fill_box(p.card, 12.0));
        container(column![title, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's `ChatAppearancePicker`: the chat theme of `chat`, or the default chat theme.
    fn chat_theme_picker(&self, st: &BeeperStore, chat: Option<&str>) -> El<'_> {
        let p = &self.pal;
        let a = &st.settings.chat_appearance;
        let stored = a.theme(chat);
        let wallpaper = chat_theme::resolve(a, chat, self.dark, &st.data_dir).0;
        let (sent, sent_fg, _) = self.sent_colors(st, chat);
        let id = chat.map(str::to_string);
        let ring = |el: El<'static>, on: bool, radius: f32, msg: UiMessage| -> El<'static> {
            let edge = if on { p.accent } else { CLEAR };
            let el = container(el).padding(4).style(bordered_box(CLEAR, edge, 2.0, radius + 4.0));
            mouse_area(el).on_press(msg).into()
        };
        let bubble = |text: &'static str, bg: Color, fg: Color| {
            container(txt(tr(text), 13.0).color(fg)).padding([8, 12]).style(fill_box(bg, 17.0))
        };
        let lines = column![
            bubble("Pick a wallpaper and bubble color", p.received, p.strong),
            container(bubble("Only you see your chat theme", sent, sent_fg)).align_right(Fill),
        ]
        .spacing(8);
        let preview: El<'_> = match wallpaper.clone() {
            Some((paint, _)) => {
                stack![chat_theme::fill(paint, Size::new(f32::INFINITY, 106.0), 12.0), container(lines).padding(16)]
                    .into()
            }
            None => container(lines).padding(16).style(fill_box(p.card, 12.0)).into(),
        };
        let preview =
            container(preview).height(106 + 2).padding(1).style(bordered_box(CLEAR, p.neutral_border, 1.0, 12.0));
        let label = |text: &'static str| txt(tr(text), 12.0).font(fw(Weight::Medium)).color(p.sub);
        let default = matches!(stored.bubble, None | Some(Bubble::Default));
        let none = if chat.is_some() { Some(Bubble::Default) } else { None };
        let sub = p.sub;
        let cross =
            svg(chat_theme::NO_BUBBLE.clone()).width(26).height(26).style(move |_, _| svg::Style { color: Some(sub) });
        let mut swatches = vec![ring(
            container(cross).padding(1).style(bordered_box(CLEAR, p.neutral_border, 1.0, 14.0)).into(),
            default,
            14.0,
            UiMessage::ChatBubble(id.clone(), none),
        )];
        swatches.extend(chat_theme::BUBBLES.iter().map(|&(bid, c)| {
            let on = stored.bubble == Some(Bubble::Swatch { id: bid.into() });
            let dot = container(Space::new()).width(28).height(28).style(fill_box(hex(c), 14.0));
            ring(dot.into(), on, 14.0, UiMessage::ChatBubble(id.clone(), Some(Bubble::Swatch { id: bid.into() })))
        }));
        let tile = |fill: El<'static>, w: Wallpaper| {
            let on = stored.wallpaper.as_ref().unwrap_or(&Wallpaper::None) == &w;
            let fill = container(fill).width(Fill).height(100).padding(1).style(bordered_box(
                CLEAR,
                p.neutral_border,
                1.0,
                10.0,
            ));
            ring(fill.into(), on, 10.0, UiMessage::ChatWallpaper(id.clone(), w))
        };
        let blank = |label: &'static str| -> El<'static> {
            container(txt(tr(label), 11.0).color(p.sub)).center(Fill).style(fill_box(p.card, 10.0)).into()
        };
        let photo = matches!(stored.wallpaper, Some(Wallpaper::Photo { .. }));
        let photo_fill = match wallpaper.filter(|_| photo) {
            Some((paint, _)) => chat_theme::fill(paint, Size::new(60.0, 98.0), 9.0),
            None => blank("Photo…"),
        };
        let mut tiles = vec![tile(blank("None"), Wallpaper::None)];
        let photo_fill = container(photo_fill).width(Fill).height(100).padding(1);
        let photo_fill = photo_fill.style(bordered_box(CLEAR, p.neutral_border, 1.0, 10.0));
        tiles.push(ring(photo_fill.into(), photo, 10.0, UiMessage::PickWallpaper(id.clone())));
        let papers = chat_theme::COLORS.iter().map(|c| Wallpaper::Color { id: c.0.into() });
        for w in papers.chain(chat_theme::GRADIENTS.iter().map(|g| Wallpaper::Gradient { id: g.0.into() })) {
            let (paint, _) = chat_theme::paint(&w, self.dark, &st.data_dir).expect("a known wallpaper");
            tiles.push(tile(chat_theme::fill(paint, Size::new(60.0, 98.0), 9.0), w));
        }
        let footer = chat.map(|c| {
            let reset = a
                .per_chat
                .contains_key(c)
                .then(|| pill_btn(p, tr("Use app default"), UiMessage::ClearChatTheme(c.into())));
            row![txt(tr("Only you see your chat themes."), 12.0).color(p.sub), Space::new().width(Fill)]
                .push(reset)
                .spacing(8)
                .align_y(Center)
        });
        column![
            preview,
            label("Bubble color"),
            Row::with_children(swatches).wrap(),
            label("Wallpaper"),
            Grid::with_children(tiles).fluid(74).spacing(2).height(grid::Sizing::EvenlyDistribute(Shrink)),
        ]
        .push(footer)
        .spacing(12)
        .padding(16)
        .into()
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
        let mut press = None;
        let control = match r.control {
            prefs::Control::Toggle(_) => {
                let v = s.pref(r.key);
                let off = r.requires.is_some_and(|k| !s.on(k)) || locked;
                press = (!off).then_some(UiMessage::SetPref(r.key, 1 - v));
                toggle_faded(p, v == 1 && !locked, press.clone(), if off { 0.5 } else { 1.0 })
            }
            prefs::Control::Select(..) if locked => {
                stack![self.pref_select(r, s), iced::widget::opaque(Space::new().width(Fill).height(Fill))].into()
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
                let bar = slider(0..=100u8, s.pref(key), move |x| UiMessage::SetPref(key, x))
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

pub fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let (rgba8, png) = (::image::ExtendedColorType::Rgba8, ::image::ImageFormat::Png);
    ::image::save_buffer_with_format(path, rgba, width, height, rgba8, png).expect("write png");
}

/// "Relaunch app" closed the app; `main` starts it again.
pub static RELAUNCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn run_native_gui(store: SharedStore) -> iced::Result {
    let st = store.read();
    i18n::set(st.settings.pref("appLanguage"));
    if st.settings.on("spellcheck") {
        let custom = st.data_dir.join("Custom Dictionary.txt");
        std::thread::spawn(|| spell::load(custom));
    }
    drop(st);
    let app = iced::daemon(move || BeeperGuiApp::boot(store.clone()), BeeperGuiApp::update, BeeperGuiApp::view)
        .title(BeeperGuiApp::title)
        .subscription(BeeperGuiApp::subscription)
        .theme(|a: &BeeperGuiApp, _| a.iced_theme())
        .scale_factor(|a: &BeeperGuiApp, _| a.zoom)
        .style(|a: &BeeperGuiApp, _| iced::theme::Style { background_color: a.pal.frame, text_color: a.pal.text })
        .default_font(fw(Weight::Normal));
    ALL_FONTS.into_iter().fold(app, |app, f| app.font(f)).run()
}

/// A New Poll window, Desktop's `PollCreator` state.
pub struct PollDraft {
    pub id: window::Id,
    pub chat: String,
    pub question: String,
    /// The options with the keys of their rows.
    pub options: Vec<(u32, String)>,
    pub multiple: bool,
    pub hidden: bool,
    /// The "Discard poll?" confirmation is open.
    pub discarding: bool,
    pub hovered: Option<usize>,
    pub dragging: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum PollEdit {
    Question(String),
    Option(usize, String),
    /// Enter in an option.
    Next(usize),
    Add,
    Remove(usize),
    Hover(usize, bool),
    Grab(Option<usize>),
    Multiple(bool),
    Hidden(bool),
    Send,
    Close,
    Discard(bool),
}

impl PollDraft {
    fn new(id: window::Id, chat: String) -> Self {
        let options = vec![(1, String::new()), (2, String::new())];
        Self {
            id,
            chat,
            question: String::new(),
            options,
            multiple: false,
            hidden: false,
            discarding: false,
            hovered: None,
            dragging: None,
        }
    }

    fn answers(&self) -> Vec<String> {
        self.options.iter().map(|o| o.1.trim()).filter(|a| !a.is_empty()).map(str::to_string).collect()
    }

    /// Desktop's duplicate marks: the options that repeat an earlier one.
    fn repeats(&self, rules: PollRules) -> Vec<bool> {
        let mut seen = HashSet::new();
        self.options.iter().map(|o| !rules.duplicates && !o.1.trim().is_empty() && !seen.insert(o.1.trim())).collect()
    }

    fn ready(&self, rules: PollRules) -> bool {
        !self.question.trim().is_empty() && self.answers().len() >= 2 && !self.repeats(rules).contains(&true)
    }

    fn dirty(&self) -> bool {
        !self.question.trim().is_empty() || !self.answers().is_empty()
    }

    fn push(&mut self) {
        let key = self.options.iter().map(|o| o.0).max().expect("a poll has options") + 1;
        self.options.push((key, String::new()));
    }

    fn input(&self, i: usize) -> String {
        format!("poll-{}-{}", self.chat, self.options[i].0)
    }
}

const ATTACH_BUTTONS: [u8; 7] = [0, 3, 5, 2, 1, 4, 6];

/// Desktop's `ComposerHiddenActions`: the actions behind the plus button. Create Poll needs `enable-polls`.
fn hidden_actions(st: &BeeperStore) -> Vec<u8> {
    let (pri, sec) = composer_buttons(st);
    let polls = st.feature("enable-polls").on;
    ATTACH_BUTTONS.into_iter().filter(|&x| Some(x) != pri && Some(x) != sec && (x != 6 || polls)).collect()
}

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
mod fuzz;

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
        assert_eq!(anim::frame(&frames, 60), (&frames[1].0, 90));
        assert_eq!(anim::frame(&frames, 160), (&frames[0].0, 40));
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
        assert_eq!(copy_text(&m, false, false), "🖼 [Picture] see https://a.b\nhttps://c.d");
        m.sender_name = Some("Jo".into());
        let stamped = copy_text(&m, true, false);
        assert!(stamped.starts_with('[') && stamped.ends_with("] Jo: 🖼 [Picture] see https://a.b\nhttps://c.d"));
        let r = |id: &str, key: &str| serde_json::json!({ "id": id, "participantID": id, "reactionKey": key });
        m.reactions = serde_json::from_value(serde_json::json!([r("1", "👍"), r("2", "❤️"), r("3", "👍")])).unwrap();
        assert_eq!(copy_text(&m, false, false), "🖼 [Picture] see https://a.b\nhttps://c.d");
        assert_eq!(copy_text(&m, false, true), "Jo: 🖼 [Picture] see https://a.b\nhttps://c.d\n(👍 2 · ❤️ 1)");
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
    fn duration_options_have_units() {
        let options = prefs::all().filter_map(|(_, r)| match r.control {
            prefs::Control::Select(options, _) => Some(options),
            _ => None,
        });
        let durations = options.flatten().filter_map(|o| o.split_once(' ')).filter(|(n, _)| n.parse::<usize>().is_ok());
        let labels: Vec<String> = durations.map(|(n, unit)| duration(n.parse().unwrap(), unit)).collect();
        assert!(labels.contains(&"0 seconds (disabled)".to_string()) && labels.contains(&"1 year".to_string()));
    }

    #[test]
    fn chat_list_tags_match_desktop() {
        let a = |t: &str, f: &str, mime: &str| Attachment {
            attachment_type: t.into(),
            file_name: Some(f.into()),
            mime_type: Some(mime.into()),
            duration: Some(65.0),
            ..Attachment::default()
        };
        let tag = |v: &[Attachment]| media_tag(v).map(|(i, l, d)| (i, l, d.map(|d| d.0)));
        let voice = Attachment { is_voice_note: Some(true), ..a("audio", "v.ogg", "audio/ogg") };
        assert_eq!(tag(&[voice]), Some((2, "Voice Note", Some("01:05".into()))));
        assert_eq!(tag(&[a("video", "c.mp4", "video/mp4")]), Some((1, "Video", Some("01:05".into()))));
        assert_eq!(tag(&[a("unknown", "12.abc.Ann.vcf", "")]), Some((4, "Contact", Some("Ann".into()))));
        assert_eq!(tag(&[a("unknown", "d.zip", "")]).map(|t| t.1), Some("ZIP"));
        assert_eq!(tag(&[a("unknown", "d.pdf", "application/pdf")]).map(|t| t.1), Some("Document"));
        assert_eq!(tag(&[a("img", "", ""), a("img", "", "")]), Some((8, "Media", None)));
    }

    #[test]
    fn media_window_formats_like_desktop() {
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

mod add_account;
mod anim;
mod chat_theme;
mod commands;
mod composer;
mod cursor;
mod dashed;
mod deeplink;
mod dialogs;
pub mod emoji;
mod enter_key;
mod format;
mod gallery;
mod i18n;
mod images;
mod integrations;
mod lift;
mod links;
pub mod login;
mod media_view;
mod menubar;
mod message;
mod notify;
mod popover;
mod portal;
mod profile;
mod reauth;
mod record;
mod settings;
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

    /// Electron's `app.badgeCount` on macOS, which Desktop's `setAppBadgeCount` sets: the count on the Dock icon, and
    /// none at 0. AppKit takes it on the main thread only, where the app updates.
    #[cfg(target_os = "macos")]
    pub fn dock_badge(count: u32) {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let label = (count > 0).then(|| objc2_foundation::NSString::from_str(&count.to_string()));
        objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
    }
}
pub mod style;
mod threads;
mod verify;
mod video;
mod vlist;
mod voice;
mod wants;
mod zoom;

use crate::composer::{composer_with, html_pills, load_draft, save_draft, Typo};
pub use crate::composer::{LinkState, Stash, Suggestion};
pub use crate::dialogs::{
    GroupDraft, GroupEdit, LabelDraft, LabelEdit, LabelPane, LabelSettings, LabelStep, PollDraft, PollEdit,
};
use crate::dialogs::{DEFAULT_LABEL_ICON, GROUP_SIZE, LABEL_SETTINGS_SIZE};
use crate::i18n::{tr, trf, trn, trs, trx};
use crate::login::{Login, LoginMsg};
use crate::media_view::{decode_media, media_items, run_ffmpeg, Carousel, MediaWindow, MEDIA_PAGE, TAB_SLIDE};
use crate::message::Rows;
pub use crate::message::TopOfThread;
use crate::settings::{
    donut, hotkey_key, mcp_value, shown, storage_totals, AddToken, IntegrationsPage, StoragePage, COMMUNITY_ROOM,
    EXPIRIES, HOTKEYS_RELAUNCH, SETTINGS_DIALOGS, STORAGE_DAYS,
};
pub use crate::settings::{search_settings, Expiry, HotkeyRec, SettingsHit, SettingsTab, TabIcon};
use crate::threads::{can_thread, Threads, THREAD_FRACTION};

use crate::style::{
    bar, bordered_box, bordered_btn, brand_svg, btn_style, clamp, clamped, ell, ensure_fonts_loaded, fill_box, fw,
    glow_box, hex, ic, icon, initials, input_style, med, pick, pick_menu_style, pick_style, scroll_style, semi,
    shadow_box, stable_bar, text_width, txt, Bare, Palette, AVATAR_GRADIENTS, CLEAR, GUTTER, SOLID_BARS,
};
use beeper_core::cloud::{Cloud, KlipyGif};
use beeper_core::config::{
    account_network, civil_from_days, local_millis, millis_from_rfc3339, now_unix_millis, now_unix_secs,
    parse_when_millis, rfc3339_from_millis,
};
use beeper_core::import::has_beeper_data;
use beeper_core::matrix::{DeviceInfo, Mention, Outgoing};
use beeper_core::models::*;
use beeper_core::platform::{current_original_beeper_data_dir, open_url_or_path_command, DisplayServerKind, OsKind};
use beeper_core::prefs;
use beeper_core::storage;
use beeper_core::store::{BeeperStore, NetworkSpace, Query, SharedStore, NOTE_CHAT_ID};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, grid, image, mouse_area, operation, pick_list, pin, rich_text, row, scrollable, slider,
    span, stack, svg, text_editor, text_input, tooltip, Column, Grid, Row, Space, Stack,
};
use iced::{
    border, mouse, window, Background, Center, Color, Element, Event, Fill, Gradient, Length, Padding, Pixels, Point,
    Radians, Right, Shadow, Shrink, Size, Subscription, Task, Theme, Vector,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Semaphore};

static AVATAR_SLOTS: Semaphore = Semaphore::const_new(8);

pub type El<'a> = Element<'a, UiMessage>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
/// The hotkeys that act on the sign-in window and before sign-in. Desktop binds all hotkeys there, but the others act
/// on chats and on the command bar, which Desktop does not draw before the onboarding ends.
const SIGN_IN_HOTKEYS: [&str; 10] = [
    "Minimize To Tray",
    "Quit App",
    "Reload App",
    "Report a Problem",
    "Reset Zoom",
    "Settings",
    "Toggle Sidebar",
    "Toggle Space Bar",
    "Zoom In",
    "Zoom Out",
];
const F_ARCHIVED: usize = 5;
const F_REQUESTS: usize = 6;
const F_LOW_PRIORITY: usize = 7;

/// Desktop's membership events, which the timeline groups.
fn membership(m: &Message) -> bool {
    m.message_type.as_deref() == Some("MEMBERSHIP")
}

/// Desktop's `getComposerUnavailableNotice`: the reason that the chat has no composer, under the last message. An
/// upgraded chat whose new room is in the list shows a link in place of the composer, and a blocked person that you
/// can unblock shows an Unblock link.
fn unavailable(st: &BeeperStore, chat: &Chat) -> Option<String> {
    match &chat.replacement_room {
        Some(room) => st.resolve_chat_idx(room).is_none().then(|| tr("Chat was upgraded.").into()),
        None if st.user_blocked(chat) => (chat.block & NO_UNBLOCK != 0).then(|| {
            let network = network_name(&chat.account_id);
            trf("You blocked %1$s. Unblock them in %2$s.", &[&chat.title, &network])
        }),
        None => chat.is_read_only.then(|| tr("Messaging isn't available in this chat").into()),
    }
}

/// Desktop's `blockedUsersStore.threads`: the chats whose other person is blocked, by title.
fn blocked_chats(st: &BeeperStore) -> Vec<&Chat> {
    let mut chats: Vec<&Chat> = st.chats.iter().filter(|c| st.user_blocked(c)).collect();
    chats.sort_by_cached_key(|c| c.title.to_lowercase());
    chats
}

/// Desktop's Block (true) or Unblock action of a DM, when the bridge supports it.
fn block_action(st: &BeeperStore, c: &Chat) -> Option<bool> {
    match st.user_blocked(c) {
        true => (c.block & NO_UNBLOCK == 0).then_some(false),
        false => st.can_block(c).then_some(true),
    }
}

fn filter_hit(i: usize, c: &Chat, st: &BeeperStore) -> bool {
    match i {
        0 => c.flags & CHAT_UNREAD != 0,
        1 => c.draft.is_some(),
        2 => c
            .preview
            .as_ref()
            .is_none_or(|m| m.is_sender != Some(true) && !m.reactions.iter().any(|r| r.participant_id == st.me().id)),
        3 => c.chat_type != "group",
        4 => c.chat_type == "group",
        F_ARCHIVED => c.archived_in_list(),
        F_REQUESTS => c.flags & CHAT_REQUEST != 0,
        F_LOW_PRIORITY => c.flags & CHAT_LOW_PRIORITY != 0,
        _ => unreachable!("filter index {i}"),
    }
}

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
    /// A virtual list must build other rows.
    Relist,
    /// A view shows avatars or media that no task loads yet.
    LoadWanted,
    DismissBanner,
    SelectTab(InboxTab),
    SelectAccount(Option<String>),
    SelectChat(String),
    /// The Undo of an archive or Low Priority toast: sets the flag of the chats back, then selects them.
    UndoFlag(ChatFlag, Vec<(String, bool)>, bool),
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
    /// Desktop's `removeFromBeeper` of the messages.
    RemoveFromBeeper(Vec<String>),
    ToggleReaction(String, String, String),
    ToggleBookmark(String),
    MarkUnreadFromHere(String, String),
    /// Desktop's Mark as Read of the message menu.
    MarkReadThrough(String, String),
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
    /// A shown notification, with its chat and message.
    Notified(Result<(u32, (String, String)), String>),
    NotificationEvent(u32, Option<String>),
    WindowFocused(bool),
    /// Desktop's `toggleIncognitoMode` from a hotkey or the command bar, with its toast.
    ToggleIncognito,
    /// The pointer enters or leaves the toast, which pauses its timeout like Desktop.
    ToastHover(bool),
    ToastTick,
    DismissOlderToast(usize),
    /// Desktop's toast button: the toast closes, newest or older, and its action runs.
    ToastAction(Option<usize>, Box<UiMessage>),
    GifQueryChanged(String),
    /// Desktop's GIF search 500 ms after the last key, for the query of that time.
    GifSearch(String),
    /// The GIFs of a query.
    /// A page of GIFs for a query: the first page (false) or the next one, and the position of the page after it.
    GifsLoaded(String, bool, Result<(Vec<KlipyGif>, String), String>),
    /// The GIF carousel scrolled: its offset, its content width and its width.
    GifScrolled(f32, f32, f32),
    PickGif(KlipyGif),
    GifDownloaded(Result<PendingMedia, String>),
    /// Desktop's `imagePackStore.loadStickerPacks` of the open chat.
    StickerPacks(Result<Vec<StickerPack>, String>),
    /// A tab of the sticker picker: Recent (`None`) or a pack.
    StickerTab(Option<String>),
    PickSticker(Sticker),
    StickerLoaded(Result<PendingMedia, String>),
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
    /// Ends the typing notices 3 seconds after the last key.
    TypingTick,
    /// Desktop's throttled draft save, 5 seconds after a composer change.
    SaveDrafts,
    /// Desktop's `showReplyThread`: shows the reply chain of the message over the timeline.
    ShowReplies(String),
    HideReplies,
    /// Desktop's `SwitchChatHoverCard` of the composer: shows the chats of the person on your other accounts.
    SwitchCard,
    /// Desktop's `composeSwitchTargets.onSelect`: the chat of the person on another account.
    SwitchChat(String),
    /// Desktop's `showThreadedReplies`: opens the thread pane of a message, or of the thread of a reply.
    ShowThread(String),
    /// Desktop's `hideThreadedReplies`.
    HideThread,
    /// A message of the thread pane, which the thread pane handles.
    InThread(Box<UiMessage>),
    /// Desktop's `setBroadcastOutsideThread`: "Also send to chat".
    SetBroadcast(bool),
    /// The thread pane edge follows the pointer from a press, to the point, until the release (`None`).
    ThreadDrag(Option<Point>),
    /// Desktop's double click on the thread pane edge: the default width.
    ResetThreadWidth,
    /// Desktop's "More Options" of the thread pane.
    ThreadMenu,
    /// Desktop's `markReplyThreadReadThrough` of the thread pane.
    MarkThreadRead,
    /// Desktop's `showUnreadThreads`: the Unread threads view in place of the chat.
    ShowUnreadThreads,
    /// Desktop's unread thread card: the thread pane of the root of the chat.
    OpenThread(String, String),
    /// The menu of the "Unread threads" pill, or `None` to close it.
    UnreadThreadsMenu(bool),
    /// Desktop's `markAllUnreadThreadsAsRead`.
    MarkAllThreadsRead,
    /// Desktop's `setThreadFollowing` of the chat, the root and the follow.
    FollowThread(String, String, bool),
    /// Desktop's `setFollowAllThreads` of the chat.
    FollowAllThreads(String, bool),
    /// Desktop's "Open in New Window" of a thread: the window "Thread in %s" of the chat and the root.
    OpenThreadWindow(String, String),
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
    /// Desktop's `CreateLabelBrowserWindow`.
    /// Desktop's `EditSidebarBrowserWindow`.
    OpenEditSpaces,
    Spaces(SpacesEdit),
    /// Desktop's `editGroup`: the Edit Group window of a chat.
    OpenEditGroup(String),
    /// Desktop's `editGroup(thread, "personal")`: the Edit Chat for Me window.
    OpenEditForMe(String),
    /// Desktop's `setPersonalRoomName`, which Undo can revert.
    SetPersonalName(String, Option<String>),
    /// Starts editing the Chat Info group description, or edits it.
    InfoDesc(Option<text_editor::Action>),
    SaveInfoDesc,
    InfoMore,
    InfoAll,
    InfoBookmarks,
    /// Desktop's "Remove Bookmark" in Chat Info, for the event ID of a message that may not be loaded.
    RemoveBookmark(String),
    /// A bookmarked message that is not loaded, or None when the server does not give it.
    BookmarkHydrated(String, Option<Box<Message>>),
    InfoMedia(Option<usize>),
    InfoQuery(String),
    InfoNote(text_editor::Action),
    /// Desktop's disappearing messages select: a chat and a timer in ms, or 0 for Off.
    SetDisappearing(String, u64),
    Group(GroupEdit),
    Profile(profile::Msg),
    /// Opens the New label window with these chats picked.
    OpenCreateLabel(Vec<String>),
    /// Desktop's `toggleLabel` for the chats of a menu: the clicked chat, or the selected chats.
    ToggleLabel(String, Vec<String>, bool),
    /// Desktop's Edit Labels window of a chat.
    OpenEditLabels(String),
    /// A switch in the Edit Labels window: `toggleLabel` without the toast.
    SwitchLabel(String, String, bool),
    /// Desktop's `Txe`: undoes the last action, with an "Undo:" toast.
    UndoAction,
    FloatRail(bool),
    /// The pointer came onto (`Some`, "chat user") or left a trigger of Desktop's `BrandUserHoverCard`.
    HoverUser(Option<String>),
    /// The pointer came onto or left the shown user card.
    HoverUserCard(bool),
    /// The wait of this generation of the user card ran out.
    HoverUserDue(u32),
    SetSpacesOrder(Vec<SpaceOrder>),
    /// Puts back the name, color, icon and chats of a label.
    RestoreLabel(Box<Label>),
    /// Desktop's `restoreLabel`: makes a deleted label again, in its old place in the sidebar.
    RestoreDeletedLabel(Box<Label>, Vec<SpaceOrder>),
    /// Desktop's Edit Label sheet (`Lst`) of Label Settings: the name, icon and color of a label.
    EditLabel(String),
    /// Desktop's Chats sheet (`Nst`) of Label Settings: the chats of a label.
    EditLabelChats(String),
    Label(LabelEdit),
    DeleteLabel(String),
    /// Desktop's `setLabelInboxVisibility`.
    SetLabelInbox(String, Option<&'static str>),
    /// Desktop's `toggleMuteLabel`.
    MuteLabel(String, bool),
    /// Desktop's `CO`: opens the Label Settings window of a label.
    OpenLabelSettings(String),
    LabelPane(window::Id, LabelPane),
    /// Desktop's `PollVotesBrowserWindow` for a chat's poll message.
    OpenPollVotes(String, String),
    EndPoll(String, String),
    CopyChatLink(String),
    OpenChatMenu(String),
    AcceptRequest(String),
    /// Desktop's Select Chats: toggles chat selection mode.
    ToggleSelectMode,
    /// Closes the chat menu over the command bar, which stays open.
    CloseChatMenu,
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
    /// Open the emoji picker at a point to react to a message.
    ReactAt(String, Point),
    /// An action picked in the message menu, which closes it.
    MessageMenu(Box<UiMessage>),
    CopyText(String),
    /// Copy a link and say so, like Desktop's Media window.
    CopyLink(String),
    CopyImage(String, usize),
    /// The width, the height and the RGBA pixels of an image to copy.
    ImageFetched(Result<(usize, usize, Vec<u8>), String>),
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
    /// Open the carousel menu at the cursor, or close it.
    CarouselMenu(bool),
    /// Scroll the filmstrip of the Media window by a number of items.
    ScrollFilmstrip(f32),
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
    /// Desktop's "Delete message?" dialog of the messages.
    AskDeleteMessage(Vec<String>),
    ArchiveStale,
    /// Archives the inbox chats, or only the read ones, with no activity in the days, after Desktop's confirmation.
    ArchiveAll(bool, i64),
    MarkAllRead,
    /// Maximizes or restores a chat window, or the main window when `None`.
    ToggleMaximize(Option<window::Id>),
    RunHotkey(&'static str),
    /// Runs the messages in order.
    Commands(Vec<UiMessage>),
    CommandHighlight(usize),
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
    /// A click on a contact in the New Chat view, by contact ID: Desktop shows the networks of the contact.
    ToggleContact(String),
    /// The user that `resolve_identifier` found for a slot of the networks of a contact or a typed identifier.
    ContactNetwork(String, usize, Result<Option<serde_json::Value>, String>),
    /// Desktop's lookup of a typed phone number, email or username, after the user stops typing.
    LookupIdentifier(String),
    /// The users of an account's `search_users` for a New Chat query.
    SearchedUsers(String, String, Result<Vec<serde_json::Value>, String>),
    ToggleNewChatResults,
    /// A click on a network of the expanded contact: the account and its user.
    PickUser(String, Box<User>),
    /// Desktop's `createChatFromDraft` for the open draft.
    StartChat,
    /// The bridge made a direct chat for a New Chat draft or for Reply Privately with this quote.
    DmCreated(Option<u32>, String, Option<String>, Result<String, String>),
    /// The bridge made the group of a New Chat draft, by draft and account.
    GroupCreated(u32, String, Result<String, String>),
    /// Desktop's New Group row: the open draft becomes a group draft.
    NewGroup,
    /// The group name of the open draft.
    GroupTitle(String),
    /// Desktop's `replyPrivately`: the direct chat with the sender of a message, with the message quoted.
    ReplyPrivately(String),
    /// Desktop's member "Send Message": the direct chat with a member of a chat, as (chat, user).
    MessageMember(String, String),
    /// Opens a member confirmation, `ASK_MAKE_ADMIN` or `ASK_REMOVE_MEMBER`, for (chat, user).
    AskMember(u8, String, String),
    /// Desktop's `exportLoadedMessagesAsTextFile`: the loaded messages of the open chat in a text file on the desktop.
    ExportChat,
    /// Desktop's Add Members button of Chat Info: the Add members submenu with the members query.
    AddMembers(String),
    /// Desktop's `addParticipant`: invites the user to the chat.
    AddMember(String, Box<User>),
    /// Desktop's `searchCmdBarUsers` for the Add members query, after the user stops typing.
    SearchMembers(String),
    /// Makes a member of a chat an admin (true) or a regular member, as (chat, user, admin).
    SetAdmin(String, String, bool),
    RemoveMember(String, String),
    AskDeleteChat(String),
    AskLeaveChat(String),
    LeaveRoom(String),
    /// Desktop's `confirmBlockUser` (true) or `confirmUnblockUser` of a DM.
    AskBlock(String, bool),
    /// Desktop's `setUserBlockedOrAlert`, as (chat, block, report spam).
    BlockUser(String, bool, bool),
    /// The "Report spam" checkbox of the Block dialog.
    ReportSpam(bool),
    /// Desktop's `BlockedUsersBrowserWindow` from Settings, and its Done button.
    OpenBlockedUsers,
    CloseBlockedUsers,
    /// A row of the Blocked users window: the chat with its Chat Info in the main window.
    OpenBlockedUser(String),
    BeginSelecting(String),
    ToggleSelected(String),
    SelectionSet(ChatFlag, bool),
    SelectionRemind(Option<i64>),
    SelectionMenu(u8, f32),
    DeleteChat(String, bool),
    NewChatQueryChanged(String),
    ToggleNewChatExpanded,
    ToggleNewChatChats,
    ToggleJoinMatrix,
    /// Join or Enter in the Join a Matrix room view.
    JoinMatrixRoom,
    /// The end of a join of the Join a Matrix room view of a draft, and whether the room had no routing servers.
    MatrixRoomJoined(Option<u32>, Result<String, String>, bool),
    ContactsLoaded(Vec<User>),
    Capabilities(String, Result<serde_json::Value, String>),
    /// The account filter of the New Chat view.
    NewChatAccount(Option<String>),
    ToggleNewChatAccounts,
    OpenSettings(SettingsTab),
    ToggleAddAccount,
    ShowAppQr,
    OpenAccount(Option<String>),
    /// Desktop's `openManageAccountInNewWindow`.
    OpenAccountSettings(String),
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
    /// Desktop's click on a `ShortcutRecorder`, which starts to record keys for the hotkey.
    RecordHotkey(&'static str),
    /// The Save (true) or Cancel (false) button of the recorder.
    HotkeyRecorder(bool),
    /// The Revert button of a hotkey with keys other than the default.
    RevertHotkey(&'static str),
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
    /// The Beeper account profile and the chat accounts from `whoami`.
    CloudAccounts(Result<(Profile, Vec<BridgeAccountInfo>), String>),
    AvatarLoaded(String, Result<image::Handle, String>),
    MediaLoaded(String, Result<Frames, String>),
    TileBaked(String, image::Handle),
    /// A quoted message that the timeline did not have, or why it could not load.
    QuoteLoaded(String, Result<Box<Message>, String>),
    /// The dismiss button of the disconnected banner of an account.
    DismissAccountError(String),
    Voice(voice::Msg),
    Record(record::Msg),
    RetryMedia(String),
    ResolveMedia(String, String, String),
    MediaResolved(String, String, String, Result<(), String>),
    Video(video::Msg),
    Gallery(gallery::Msg),
    /// Ctrl+V and "Paste": Desktop's `paste` handler reads the clipboard.
    Paste,
    Pasted(Result<Clip, String>),
    DiscardAttachment(usize),
    /// Desktop's `openAttachmentPicker`.
    PickFiles,
    FilesPicked(Result<Vec<std::path::PathBuf>, String>),
    /// Desktop's "Change" of "Downloads location", and the picked folder.
    PickDownloads,
    DownloadsPicked(Result<Vec<std::path::PathBuf>, String>),
    /// Opens the emoji picker for a quick reaction (see [`emoji::Picker::pref`]), or closes it.
    QuickEmoji(Option<u8>),
    SetQuickEmoji(u8, String),
    /// The files to attach, or the toast text of each file that did not load.
    FilesLoaded(Vec<Result<PendingMedia, String>>),
    /// A file drag enters (true) or leaves a window.
    FileHover(window::Id, bool),
    FileDropped(window::Id, std::path::PathBuf),
    LinkPreviewLoaded(String, Result<Option<beeper_core::cloud::PagePreview>, String>),
    RemoveLinkPreview(String),
    /// Picks a row of the composer autocomplete.
    PickSuggestion(usize),
    AnimationTick,
    Pushed(Result<(), String>),
    /// Desktop's `retryErroredMessage` and `removeErroredMessage` of a failed send.
    RetrySend(String),
    RemoveUnsent(String),
    /// The timeline moved, and is above its bottom (true) or at it.
    TimelineScrolled(bool),
    ScrollToBottom,
    /// The top of the timeline came on screen (true) or went off screen.
    TopShown(bool),
    LoadOlder,
    /// A load of older messages ended.
    LoadedOlder(Result<(), String>),
    DevicesLoaded(Result<Vec<DeviceInfo>, String>),
    MicsLoaded(Result<(String, Vec<String>), String>),
    SetMicrophone(String),
    /// Desktop's "Share Analytics" toggle.
    ShareAnalytics,
    /// The server saved `analyticsDisabled`, or failed and the toggle goes back to the first value.
    AnalyticsSaved(bool, Result<(), String>),
    Ask(Option<u8>),
    /// "Logout" of another device in Settings > Devices, its confirmation, and the end of the logout.
    AskLogoutDevice(String),
    LogoutDevice(String),
    DeviceLoggedOut(String, Result<(), String>),
    /// The close of Desktop's recovery key reminder (`dismissRecoveryCodeReminder`).
    DismissRecoveryReminder,
    ReminderMenu,
    OrderReminders(u8),
    DismissAllReminders,
    Logout,
    LoggedOut(Result<(), String>),
    StorageScanned(Vec<storage::Entry>),
    /// The age choice of the clear for all chats (0) or for the open chat (1).
    StorageAge(usize, usize),
    /// Toggles a type, as its index in [`storage::Kind::ALL`], for all chats (0) or for the open chat (1).
    StorageKind(usize, usize),
    StorageChat(String),
    /// Clears the chosen storage of all chats (0), the cache (1) or the open chat (2).
    ClearStorage(u8),
    StorageCleared(Result<(), String>),
    /// Settings > Integrations: the Desktop API tokens and connections, Remote Access, and the MCP install.
    Integrations(integrations::Msg),
    JoinCommunity,
    CommunityJoined(Result<(), String>),
    OpenQuickLook(String, String),
    OpenUrl(String),
    /// The result of opening a URL in the browser.
    Opened(Result<(), String>),
    CloseModals,
    ToggleEditHistory(String),
    /// "Show more" or "Show less" of a message over 40 lines.
    ToggleClamp(String),
    HoverEnter(String),
    ShowQuickReactions,
    HoverChat(String, bool),
    DeleteDraft(String),
    HoverExit(String),
    DragStart,
    DragMoved(Point),
    DragEnd,
    /// A double click on the sidebar edge gives the sidebar its first width, like Desktop.
    ResetSidebarWidth,
    /// The cursor is over a rail space, at this point in it.
    RailHover(String, Point),
    RailLeave(String),
    RailPress,
    /// The pointer came onto (true) or left the header card of the chat, or its member list.
    HeaderHover(bool),
    /// The 500 ms after the pointer left the header card of this generation ran out.
    HeaderHoverEnd(u32),
    /// Desktop's menu bar: hidden (`None`), shown, or shown with a menu open.
    MenuBar(Option<Option<usize>>),
    /// The pointer came onto a menu title, which opens its menu while another menu is open.
    MenuBarHover(usize),
    /// An item of the menu bar: the menu bar hides, and the item runs.
    MenuBarRun(Box<UiMessage>),
    /// The Cut (true) and Copy items of the Edit menu, on the composer.
    ComposerClip(bool),
    /// The Minimize item of the Window menu.
    MinimizeWindow(window::Id),
    /// The pointer came onto (`Some`) or left a tile of the pinned grid.
    PinHover(Option<String>),
    /// Desktop's `Drop` of the pinned grid (`lY`): a press, a move and the release of a drag of a pin.
    PinPress,
    PinDragMoved(Point),
    PinDragEnd,
    RailDragMoved(Point),
    RailDragEnd,
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
    AskRemoveAccount(String),
    RemoveAccount(String),
    AccountRemoved(String, Result<(), String>),
    ScreenshotCaptured(window::Id, window::Screenshot),
    KeyPressed(Key, Modifiers),
    /// Text that a key typed outside of a text field in a window.
    WindowText(window::Id, String),
    /// Desktop's type-to-focus: the composer of the pane takes the text at its end.
    TypeInComposer(String),
    StoreChanged,
    FocusRequested(Option<String>),
    /// Desktop's `second-instance` event with the arguments of the later launch.
    SecondInstance(Vec<String>),
    /// A room that a `beeper://join-matrix-room` link joined.
    RoomJoined(Result<String, String>),
    /// The network became available or unavailable.
    Online(bool),
    PillHover(bool),
    Login(LoginMsg),
    Verify(verify::VerifyMsg),
    Reauth(reauth::ReauthMsg),
    EnterKey(enter_key::KeyMsg),
    /// Desktop's `openNewAccountInNewWindow` for a network, or `reauthenticate` for a login of it.
    OpenAddAccount(String, Option<String>),
    AddAccount(window::Id, add_account::AddMsg),
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

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
fn network_name(account_id: &str) -> &'static str {
    account_network(account_id).display_name
}

/// Desktop's mute reason of the chat menu: all labels of the chat are muted, or for a chat without labels,
/// notifications of its account are off.
fn muted_by(st: &BeeperStore, c: &Chat) -> Option<String> {
    if st.feature("labels-v2").on && !c.label_ids.is_empty() {
        let labels: Option<Vec<&Label>> =
            c.label_ids.iter().map(|id| st.labels.iter().find(|l| &l.id == id && l.muted)).collect();
        return Some(match labels?[..] {
            [l] if !l.name.trim().is_empty() => trf("Muted by Label %s", &[&l.name.trim()]),
            [_] => tr("Muted by Label").into(),
            ref all => trn("Muted by %d Label", "Muted by %d Labels", all.len(), &[&all.len()]),
        });
    }
    let off = !st.settings.account(&c.account_id).notifications;
    off.then(|| trf("Muted by %s", &[&network_name(&c.account_id)]))
}

/// Desktop's `userDisplayText`: the user of a chat account, as the account's network shows them.
fn account_user(st: &BeeperStore, account_id: &str) -> String {
    let b = st.bridge_accounts.iter().find(|b| b.account_id == account_id);
    let r = b.and_then(|b| b.remote_profile.clone()).unwrap_or_default();
    [r.name, r.username, r.phone, r.email].into_iter().flatten().find(|s| !s.is_empty()).unwrap_or_default()
}

/// Desktop's `hasMultipleAccountsOnSameBridgeType`: you have another account of the bridge of `account_id`.
fn several_accounts(st: &BeeperStore, account_id: &str) -> bool {
    let kind = st.bridge_accounts.iter().find(|b| b.account_id == account_id).map(|b| &b.bridge_type);
    kind.is_some_and(|k| st.bridge_accounts.iter().filter(|b| &b.bridge_type == k).count() > 1)
}

/// Desktop's `composeSwitchTargets` of the account variant: in a single chat on an account with other accounts of its
/// bridge, the chat and the chats with the same person on those accounts (`getAltAccountThreads`).
fn switch_targets(st: &BeeperStore, chat: &Chat) -> Option<Vec<String>> {
    let other = |c: &Chat| c.participants.items.iter().find(|u| u.is_self != Some(true)).cloned();
    let them = other(chat).filter(|_| chat.chat_type == "single" && several_accounts(st, &chat.account_id))?;
    let kind = st.bridge_accounts.iter().find(|b| b.account_id == chat.account_id).map(|b| &b.bridge_type);
    let alts: Vec<&str> = st
        .bridge_accounts
        .iter()
        .filter(|b| b.account_id != chat.account_id && Some(&b.bridge_type) == kind)
        .map(|b| b.account_id.as_str())
        .collect();
    // Desktop's `findMostRecentSingleThread` of each account, by the username, the phone, the email, then the ID.
    let keys: [fn(&User) -> Option<&str>; 4] =
        [|u| u.username.as_deref(), |u| u.phone_number.as_deref(), |u| u.email.as_deref(), |u| Some(u.id.as_str())];
    let newest = |account: &str, same: &dyn Fn(&User) -> bool| {
        let single = st.chats.iter().filter(|c| c.account_id == account && c.chat_type == "single");
        single.filter(|c| other(c).is_some_and(|u| same(&u))).max_by(|a, b| a.last_activity.cmp(&b.last_activity))
    };
    let mut found: Vec<&Chat> = keys
        .iter()
        .filter_map(|k| Some((k, k(&them).filter(|v| !v.is_empty())?)))
        .map(|(k, v)| alts.iter().filter_map(|a| newest(a, &|u: &User| k(u) == Some(v))).collect())
        .find(|f: &Vec<&Chat>| !f.is_empty())
        .unwrap_or_default();
    found.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    Some(std::iter::once(chat).chain(found).map(|c| c.id.clone()).collect())
}

/// Desktop's `Up`: the network of an account, and its user when another listed account has the same network.
fn account_option(st: &BeeperStore, account_id: &str, listed: &[String]) -> String {
    let n = network_label(account_id);
    if listed.iter().filter(|a| network_label(a) == n).count() < 2 {
        return n.into();
    }
    format!("{n} — {} (cloud)", account_display_name(st, account_id, listed))
}

/// Whether typed text looks like a phone number: 7 to 15 digits with phone punctuation.
fn typed_phone(q: &str) -> bool {
    let digits = q.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits) && q.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c))
}

/// The user of a bridge `resolve_identifier` response, by its Matrix ID.
fn resolved_user(v: &serde_json::Value) -> User {
    let ids: Vec<&str> = v["identifiers"].as_array().into_iter().flatten().filter_map(|i| i.as_str()).collect();
    let find = |scheme: &str| ids.iter().find_map(|i| i.strip_prefix(scheme)).map(str::to_string);
    User {
        id: v["mxid"].as_str().or(v["id"].as_str()).unwrap_or_default().into(),
        full_name: v["name"].as_str().map(Into::into),
        img_url: v["avatar_url"].as_str().map(Into::into),
        phone_number: find("tel:"),
        email: find("mailto:"),
        ..Default::default()
    }
}

/// Desktop's `SD`: a person once per bridge, by phone number, username, email or ID.
fn person_key(account_id: &str, u: &User) -> String {
    let id = u.phone_number.as_deref().map(phone_digits);
    let id = id.or_else(|| u.username.clone()).or_else(|| u.email.clone()).unwrap_or_else(|| u.id.clone());
    format!("{}:{id}", brand_key(account_id))
}

/// Desktop's `pb`: the name that sorts a person.
fn person_name(u: &User) -> &str {
    [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email]
        .into_iter()
        .find_map(|s| s.as_deref())
        .unwrap_or(&u.id)
}

/// Desktop's `Chats` category of New Chat: the people of the chats of the accounts, once per bridge and
/// identifier from their most recent chat, that match the query, by name.
fn chat_people(st: &BeeperStore, accounts: &[String], only: Option<&str>, q: &str) -> Vec<(String, User)> {
    let mut found: HashMap<String, (&Chat, &User)> = HashMap::new();
    let chats = st.chats.iter().filter(|c| accounts.contains(&c.account_id) && only.is_none_or(|a| c.account_id == a));
    for c in chats {
        for u in c.participants.items.iter().filter(|u| u.is_self != Some(true)) {
            let key = person_key(&c.account_id, u);
            if found.get(&key).is_none_or(|(old, _)| c.last_activity > old.last_activity) {
                found.insert(key, (c, u));
            }
        }
    }
    let fields =
        |u: &User| [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email].map(Clone::clone);
    let matches =
        |u: &User| fields(u).into_iter().flatten().chain([u.id.clone()]).any(|f| f.to_lowercase().contains(q));
    let mut people: Vec<(String, User)> =
        found.into_values().filter(|(_, u)| matches(u)).map(|(c, u)| (c.account_id.clone(), u.clone())).collect();
    let rank = |u: &User| {
        let n = person_name(u).to_lowercase();
        (n.is_empty(), !n.starts_with(|c: char| c.is_ascii_alphabetic()), n)
    };
    people.sort_by_cached_key(|(_, u)| rank(u));
    people
}

/// Desktop's `accountDisplayName`: the network, after the user when another account has the same network.
fn account_display_name(st: &BeeperStore, account_id: &str, accounts: &[String]) -> String {
    let n = network_label(account_id);
    let user = account_user(st, account_id);
    match accounts.iter().filter(|a| network_label(a) == n).count() > 1 && !user.is_empty() {
        true => format!("{user} · {n}"),
        false => n.into(),
    }
}

/// Desktop's `createThreadWithPhoneNumber` identifier: the digits and `+` of a phone number.
fn phone_digits(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect()
}

/// The email address that linkifyjs finds in a word, without trailing punctuation.
fn email(word: &str) -> Option<&str> {
    let w = word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']);
    let (local, domain) = w.split_once('@')?;
    let ok = |s: &str, more: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || more.contains(c));
    let (host, tld) = domain.rsplit_once('.')?;
    (ok(local, "._%+-") && ok(host, ".-") && tld.len() > 1 && tld.chars().all(char::is_alphabetic)).then_some(w)
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

/// Desktop's `sanitize-filename`: the name without path characters, control characters and trailing dots or spaces.
fn safe_file_name(name: &str) -> String {
    let name: String = name.chars().filter(|c| !r#"/\?<>:*|""#.contains(*c) && !c.is_control()).collect();
    name.trim_end_matches(['.', ' ']).into()
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

/// Desktop's "Approve connection" dialog of a token request from the Desktop API.
struct TokenAsk {
    id: String,
    /// The client name and version.
    app: String,
    write: bool,
    expiry: Expiry,
    timed_out: bool,
}

/// `dir/name`, or `dir/stem (n).ext` with the first free `n`.
/// Desktop's `downloadsDirPath`: the chosen folder, or the downloads folder of the OS.
fn downloads_dir(s: &BeeperSettings) -> std::path::PathBuf {
    match s.downloads_dir.as_str() {
        "" => beeper_core::platform::resolve_home_dir(&std::env::vars().collect()).join("Downloads"),
        dir => dir.into(),
    }
}

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
        .push(txt(label, 13.0).line_height(Pixels(MENU_LINE)))
        .push(Space::new().width(Fill))
        .push(txt(hotkey, 13.0).line_height(Pixels(MENU_LINE)).color(Color { a: 0.5, ..p.strong }));
    menu_line(p, line, msg)
}

/// A menu row with the content `line`. Without a message it is greyed out.
fn menu_line<'a>(p: &Palette, line: Row<'a, UiMessage>, msg: Option<UiMessage>) -> El<'a> {
    let style = btn_style([CLEAR, p.menu_hover, p.menu_hover], [p.text, Color::WHITE, Color::WHITE], 4.0);
    let line = line.align_y(Center);
    container(button(line).padding([3, 8]).width(Fill).style(style).on_press_maybe(msg)).padding([1, 5]).into()
}

/// Desktop's `menu-item section-header`: a grey heading row of a menu, as high as a menu row.
fn menu_heading<'a>(p: &Palette, label: String) -> El<'a> {
    let label = med(label, 11.0).color(p.weak).wrapping(iced::widget::text::Wrapping::None);
    container(label).padding([0, 13]).center_y(MENU_ITEM).width(Fill).clip(true).into()
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
const ASK_DELETE_LABEL: u8 = 13;
const ASK_LEAVE_CHAT: u8 = 14;
const ASK_MAKE_ADMIN: u8 = 15;
const ASK_REMOVE_MEMBER: u8 = 16;
const ASK_REMOVE_ACCOUNT: u8 = 17;
/// Desktop's `confirmBlockUser` and `confirmUnblockUser`, then their "%s Is Disconnected" dialogs.
const ASK_BLOCK: u8 = 18;
const ASK_UNBLOCK: u8 = 19;
const ASK_BLOCK_OFFLINE: u8 = 20;
const ASK_UNBLOCK_OFFLINE: u8 = 21;
/// Desktop's "Privacy notice" before the first transcription of a voice note.
const ASK_TRANSCRIBE: u8 = 22;
/// Desktop's logout of another device of the account in Settings > Devices.
const ASK_LOGOUT_DEVICE: u8 = 23;
/// Desktop opens its `prefs` window at 800 x 740, in the middle of the screen.
const SETTINGS_SIZE: Size = Size::new(800.0, 740.0);
/// Desktop's default "Quick reaction emoji".
/// Wide enough for "Copy Entire Message with Timestamp" on one line.
const MESSAGE_MENU_W: f32 = 270.0;
/// Desktop's `minSidebarWidth`, the least width of the thread pane and of the chat beside it.
const MIN_PANE_W: f32 = 240.0;
const REMINDER_ORDERS: [&str; 4] = [
    "Order by Least Time Remaining",
    "Order by Most Time Remaining",
    "Order by Oldest Message",
    "Order by Newest Message",
];
/// Desktop's `.menu-item` line: 13 px text at the 1.4 line height of `html`.
const MENU_LINE: f32 = 13.0 * 1.4;
/// The pitch of Desktop's `.menu-item`: the line, 3 px of padding on each side, and a collapsed 2 px margin.
const MENU_ITEM: f32 = MENU_LINE + 8.0;
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
const SUB_MENU_W: [f32; 3] = [124.0, 257.0, 180.0];
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

/// A flag as the chat menus show it: "Mark as Read" is for every unread chat.
fn has_flag(c: &Chat, flag: ChatFlag) -> bool {
    match flag {
        ChatFlag::MarkedUnread => c.is_unread(),
        f => c.flag(f),
    }
}

fn checkbox<'a>(p: &Palette, on: bool) -> El<'a> {
    let (bg, ring) = if on { (CHECKED, CLEAR) } else { (CLEAR, Color { a: 0.3, ..p.strong }) };
    let check = on.then(|| ic(icon::CHECK, 12.0).color(Color::WHITE));
    container(check).center(16).style(bordered_box(bg, ring, 1.5, 8.0)).into()
}

/// CSS `flex-wrap` with `justify-content: space-between`: rows of `items` of `size` in `width`, 10px apart at least.
fn spread(items: Vec<El<'_>>, size: f32, width: f32) -> El<'_> {
    let per = ((width + 10.0) / (size + 10.0)) as usize;
    let mut items = items.into_iter().peekable();
    let mut rows = Column::new().spacing(12);
    while items.peek().is_some() {
        let line: Vec<El> = items.by_ref().take(per).collect();
        let n = line.len() as f32;
        rows = rows.push(Row::with_children(line).spacing((width - n * size) / (n - 1.0).max(1.0)));
    }
    rows.into()
}

/// Desktop's `CheckboxPlus`: a plus until it is checked.
fn checkbox_plus<'a>(p: &Palette, on: bool) -> El<'a> {
    match on {
        true => checkbox(p, true),
        false => {
            container(ic(icon::PLUS, 10.0).color(p.sub)).center(16).style(bordered_box(CLEAR, p.sub, 1.5, 8.0)).into()
        }
    }
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

/// Desktop's `Ng`: an unread count, at most "999+".
fn unread_label(n: usize) -> String {
    if n > 999 {
        "999+".into()
    } else {
        n.to_string()
    }
}

fn count_badge<'a>(c: &Chat, bg: Color, fg: Color, h: f32) -> El<'a> {
    let n = if c.unread_count > 0 { unread_label(c.unread_count as usize) } else { String::new() };
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

/// Desktop's `yD`: the label icons by id, in the order of the picker.
static LABEL_ICONS: std::sync::LazyLock<Vec<(&str, svg::Handle)>> = std::sync::LazyLock::new(|| {
    let icon = |line: &'static str| {
        let [id, _, d] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("a label icon line") };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path fill="currentColor" d="{d}"/></svg>"#
        );
        (id, svg::Handle::from_memory(svg.into_bytes()))
    };
    include_str!("../assets/label_icons.tsv").lines().map(icon).collect()
});

fn label_glyph(id: &str) -> Option<&'static svg::Handle> {
    LABEL_ICONS.iter().find(|i| i.0 == id).map(|i| &i.1)
}

/// The gradient of a label color. Desktop ignores unknown colors.
fn label_fill(color: Option<u8>) -> Option<Background> {
    let (top, bottom) = LABEL_COLORS.get(usize::from(color?)).copied()?;
    let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, hex(top)).add_stop(1.0, hex(bottom));
    Some(Background::Gradient(Gradient::Linear(g)))
}

/// Desktop's `LabelSpaceIcon`: the label color with its icon, or the icon alone.
fn label_icon<'a>(l: &Label, size: f32, fg: Color) -> El<'a> {
    label_tile(l, size, (size * 20.0 / 28.0).round(), 8.0, fg)
}

/// Desktop's `LabelIconTile`: the icon of a label, `glyph` pixels high, on a tile of the label color.
fn label_tile<'a>(l: &Label, size: f32, glyph: f32, radius: f32, fg: Color) -> El<'a> {
    let fill = label_fill(l.color);
    let glyph = (l.icon.is_some() || fill.is_none())
        .then(|| label_glyph(l.icon.as_deref().unwrap_or(DEFAULT_LABEL_ICON)))
        .flatten()
        .map(|h| tinted(h, glyph, if fill.is_some() { Color::WHITE } else { fg }));
    let style =
        move |_: &Theme| container::Style { background: fill, border: border::rounded(radius), ..Default::default() };
    container(glyph).center(size).style(style).into()
}

/// Desktop's grouped rows on the secondary container, with 0.5px rules between them.
fn grouped_rows<'a>(p: &Palette, rows: Vec<El<'a>>) -> El<'a> {
    let rule = p.prefs_border;
    let mut col = column![];
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.push(container(Space::new()).width(Fill).height(0.5).style(fill_box(rule, 0.0)));
        }
        col = col.push(r);
    }
    container(col).width(Fill).clip(true).style(fill_box(p.chip, 8.0)).into()
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
pub type Frames = Vec<anim::Frame>;
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
    /// The length in seconds of a video or an audio file.
    pub duration: Option<f64>,
    /// Desktop's `stickerPackImage` of a sticker from the sticker picker.
    pub sticker: Option<Box<Sticker>>,
}

/// Lodash's `truncate` with the default omission: at most `n` characters, and "..." at the end when it cuts.
fn truncate(s: String, n: usize) -> String {
    match s.chars().count() > n {
        true => s.chars().take(n - 3).collect::<String>() + "...",
        false => s,
    }
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

/// What a paste gives: Desktop's `clipboardData.files` (the copied files, or a copied image), or else the text.
#[derive(Debug, Clone)]
pub enum Clip {
    Files(Vec<std::path::PathBuf>),
    Image(Png),
    Text,
}

async fn clipboard_paste() -> Result<Clip, String> {
    let read = || {
        let mut c = arboard::Clipboard::new().map_err(|e| format!("Could not read the clipboard: {e}"))?;
        // The `text/uri-list` of a file manager. iced's X11 clipboard answers every target with its text, which
        // has no `file://` lines.
        if let Some(files) = c.get().file_list().ok().filter(|f| !f.is_empty()) {
            return Ok(Clip::Files(files));
        }
        let img = match c.get_image() {
            Err(arboard::Error::ContentNotAvailable) => return Ok(Clip::Text),
            Err(arboard::Error::Unknown { .. }) if c.get_text().is_ok() => return Ok(Clip::Text),
            r => r.map_err(|e| format!("Could not read the clipboard: {e}"))?,
        };
        let size = (u32::try_from(img.width).unwrap(), u32::try_from(img.height).unwrap());
        let px = ::image::RgbaImage::from_raw(size.0, size.1, img.bytes.into_owned()).expect("arboard gives RGBA");
        let mut png = Vec::new();
        px.write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).expect("encode PNG");
        Ok(Clip::Image((png, size)))
    };
    tokio::task::spawn_blocking(read).await.expect("clipboard task")
}

fn faces_of(users: &[User], group: bool) -> (Vec<&User>, usize) {
    let mut others: Vec<&User> = users.iter().filter(|u| u.is_self != Some(true)).collect();
    let name = |u: &User| {
        [&u.custom_nickname, &u.full_name, &u.username, &u.email, &u.phone_number]
            .into_iter()
            .find_map(|n| n.as_deref().filter(|n| !n.is_empty()))
            .unwrap_or(&u.id)
            .to_lowercase()
    };
    others.sort_by_cached_key(|u| (u.is_network_bot == Some(true), u.is_admin != Some(true), name(u)));
    let more = if group && others.len() > 5 { others.len() - 4 } else { 0 };
    let shown = if group { others.len() - more } else { others.len().min(1) };
    (others.into_iter().take(shown).collect(), more)
}

/// Desktop's `canEditGroup`: a group whose name, avatar or description the user can change.
fn can_edit_group(c: &Chat) -> bool {
    c.chat_type == "group" && c.edits != 0
}

fn network_bot(chat: &Chat, user_id: &str) -> bool {
    chat.participants.items.iter().any(|u| u.id == user_id && u.is_network_bot == Some(true))
}

/// Desktop's `pendingMessages`: a message of this device that the homeserver did not take yet. The demo has no
/// homeserver.
fn pending(st: &BeeperStore, m: &Message) -> bool {
    !st.config.demo && m.id.starts_with("msg_") && !st.sent.contains_key(&m.id)
}

/// The people other than you and the sender who saw the message.
fn seen_by(st: &BeeperStore, m: &Message) -> usize {
    m.seen.keys().filter(|u| !st.is_me(u) && **u != m.sender_id).count()
}

/// The tooltip of Desktop's `getReadIcon` in the chat list: "Sent", "Seen", or "Seen by %1$d of %2$d" in a group.
/// A pending message has no tooltip.
fn receipt_tip(st: &BeeperStore, c: &Chat, m: &Message) -> Option<String> {
    if pending(st, m) {
        return None;
    }
    let total = c.participants.items.iter().filter(|u| !st.is_me(&u.id) && u.id != m.sender_id).count();
    Some(match seen_by(st, m) {
        0 => tr("Sent").into(),
        n if total > 1 => trf("Seen by %1$d of %2$d", &[&n, &total]),
        _ => tr("Seen").into(),
    })
}

/// Desktop's `Iot`: the columns of the pinned grid for `n` pins. A column is at least 80px wide, there are at most 6,
/// and the rows get the same number of pins.
fn pin_columns(width: f32, n: usize) -> usize {
    let most = ((width / 80.0) as usize).min(n).clamp(1, 6);
    n.max(1).div_ceil(n.max(1).div_ceil(most))
}

/// Desktop's `canStartEditMessage` for text: an own message that is not a poll.
/// Desktop's `canStartEditMessage` for a message of a chat with `rules`.
fn editable(m: &Message, rules: MessageRules) -> bool {
    m.is_sender == Some(true) && m.poll.is_none() && rules.edit
}

/// Desktop's `canEditMessage`: the edit window of the network is still open.
fn edit_open(m: &Message, rules: MessageRules) -> bool {
    m.is_deleted != Some(true) && MessageRules::young(rules.edit_max_age, &m.timestamp)
}

/// The text of a chat list preview, or a label for its links.
fn preview_body(m: &Message) -> Option<String> {
    match m.links.len() {
        _ if m.poll.is_some() || m.call.is_some() => None,
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
fn bubble_text(c: &Chat, launched: i64, now: i64) -> Option<String> {
    let unread = c.is_unread();
    let at = c.last_activity.as_deref().map_or(0, |t| millis_from_rfc3339(t).expect("RFC 3339 timestamp"));
    let limit = match c.participants.total {
        0..=2 => 3,
        3..=6 => 5,
        _ => 10,
    };
    if !unread || c.unread_count == 0 || (now - at.max(launched) >= 60_000 && c.unread_count > limit) {
        return None;
    }
    let m = c.preview.as_ref()?;
    let tag = || message_tag(m).map(|(_, l, d)| d.map_or_else(|| l.to_string(), |(d, _)| format!("{l} {d}")));
    let text = preview_body(m).or_else(tag)?;
    let first = preview_sender(c, m).map(sender_first_name);
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

/// The index of Desktop's Poll badge, which shows the `poll-options` icon of the composer.
const POLL_TAG: usize = 10;
/// The `TAG_ICONS` indices of the Video badge, whose `videocam` icon the video calls share, and of the call badge.
const VIDEO_TAG: usize = 1;
const CALL_TAG: usize = 9;

/// Desktop's `_P`: the badge of a poll, then of a call, then of the attachments.
fn message_tag(m: &Message) -> Option<MediaTag> {
    if m.poll.is_some() {
        return Some((POLL_TAG, trx("Poll", "chat list badge for a poll message"), None));
    }
    match &m.call {
        Some(call) => Some((if call.is_video() { VIDEO_TAG } else { CALL_TAG }, call_label(call), None)),
        None => media_tag(&m.attachments),
    }
}

/// Desktop's `Yle`: the label of a call.
fn call_label(call: &beeper_core::models::CallAction) -> &'static str {
    match call.call_type.as_deref() {
        Some("video") => tr("Video call"),
        Some("voice") => tr("Voice call"),
        _ => trx("Call", "generic call type label"),
    }
}

fn tag_glyph(tag: usize) -> &'static svg::Handle {
    if tag == POLL_TAG {
        &COMPOSER_ICONS[8]
    } else {
        &TAG_ICONS[tag]
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
    let said = a.transcription.as_ref().map(|t| (format!("“{}”", t.transcription), false));
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

/// The parts of a vCard that Desktop's contact card shows.
#[derive(Debug, Default, PartialEq)]
struct VCard {
    name: String,
    org: Option<String>,
    phones: Vec<String>,
    emails: Vec<String>,
}

/// Desktop's `parseVCardAsContact`: unfolds the lines, drops the parameters and groups, and splits each value at the
/// unescaped `;`, or else at the unescaped `,`. The name is `FN`, or else the given and family names of `N`.
fn parse_vcard(text: &str) -> VCard {
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        match lines.last_mut().filter(|_| l.starts_with([' ', '\t'])) {
            Some(last) => last.push_str(l.trim()),
            None => lines.push(l.to_string()),
        }
    }
    let bare = |v: &str, c: char| v.starts_with(c) || v.as_bytes().windows(2).any(|w| w[1] == c as u8 && w[0] != b'\\');
    let split = |v: &str| -> Vec<String> {
        let v = v.replace("\\n", "\n");
        let Some(c) = [';', ','].into_iter().find(|&c| bare(&v, c)) else {
            return vec![v.replace("\\,", ",").replace("\\;", ";")];
        };
        let other = if c == ';' { ',' } else { ';' };
        let v = v.replace(&format!("\\{other}"), &other.to_string()).replace(&format!("\\{c}"), "\0");
        v.split(c).map(|p| p.replace('\0', &c.to_string())).collect()
    };
    let mut fields = HashMap::<String, Vec<Vec<String>>>::new();
    for l in &lines {
        let (key, value) = l.split_once(':').unwrap_or((l, ""));
        let key = key.split(';').next().expect("split gives one part");
        fields.entry(key.split_once('.').map_or(key, |(_, k)| k).to_lowercase()).or_default().push(split(value));
    }
    let first = |k: &str| fields.get(k).and_then(|v| v[0].iter().find(|s| !s.is_empty()).cloned());
    let all = |k: &str| fields.get(k).into_iter().flatten().flatten().filter(|s| !s.is_empty()).cloned().collect();
    let n = fields.get("n").map(|n| n[0].clone()).unwrap_or_default();
    let given = [n.get(1), n.first()].into_iter().flatten().filter(|s| !s.is_empty()).cloned();
    let name = first("fn").unwrap_or_else(|| given.collect::<Vec<_>>().join(" "));
    VCard { name, org: first("org"), phones: all("tel"), emails: all("email") }
}

/// Desktop's pending `ContactAttachment`: the avatar, the name or the organization, and the first phone number and
/// email with the count of the others.
fn contact_card<'a>(p: &Palette, c: &VCard) -> El<'a> {
    let line = |s: String| txt(s, 12.0).line_height(Pixels(16.0)).color(p.text);
    let medium = |s: String| txt(s, 14.0).font(fw(Weight::Medium)).color(p.text);
    let mut meta = Column::new().push((!c.name.is_empty()).then(|| medium(c.name.clone())));
    if let Some(org) = &c.org {
        meta = meta.push(match c.name.is_empty() {
            true => El::from(medium(org.clone())),
            false => container(line(org.clone())).padding(Padding::ZERO.top(4.0)).into(),
        });
    }
    let list = |items: &[String], title: String, more: &'static str, fmt: fn(&str) -> String| {
        let head = semi(title, 12.0).line_height(Pixels(16.0)).color(p.text);
        let rest = items.len().saturating_sub(1);
        let more = (rest > 0).then(|| line(trn(more, more, rest, &[&rest])));
        let col = column![container(head).padding(Padding::ZERO.top(6.0).bottom(4.0))];
        (!items.is_empty()).then(|| col.push(line(fmt(&items[0]))).push(more))
    };
    let n = c.phones.len();
    let phones = list(&c.phones, trn("Phone Number", "Phone Numbers", n, &[]), "+ %d more", fmt_phone);
    let emails = list(&c.emails, trn("Email", "Emails", c.emails.len(), &[]), " + %d more", str::to_string);
    let details = row![avatar(&c.name, 32.0, 0.0, p), meta.push(phones).push(emails)].spacing(8);
    container(column![details, Space::new().width(180)]).padding([8, 10]).style(fill_box(p.received, 0.0)).into()
}

/// Desktop's chat list badge icons: image, videocam, volume_up, audio_file, account_circle, csv, folder_zip,
/// draft and photo_library, at its `scale(1.2)` in a 12 px box.
static TAG_ICONS: std::sync::LazyLock<[svg::Handle; 10]> = std::sync::LazyLock::new(|| {
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
        "M798-120q-125 0-247-54.5T329-329Q229-429 174.5-551T120-798q0-18 12-30t30-12h162q14 0 25 9.5t13 22.5l26 140q2 16-1 27t-11 19l-97 98q20 37 47.5 71.5T387-386q31 31 65 57.5t72 48.5l94-94q9-9 23.5-13.5T670-390l138 28q14 4 23 14.5t9 23.5v162q0 18-12 30t-30 12ZM241-600l66-66-17-94h-89q5 41 14 81t26 79Zm358 358q39 17 79.5 27t81.5 13v-88l-94-19-67 67ZM241-600Zm358 358Z",
    ]
    .map(|d| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 -880 800 800"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});

fn preview_sender<'a>(chat: &'a Chat, m: &Message) -> Option<&'a User> {
    // Desktop's `getLastParticipantName` names no one on an action, like a member change.
    let shown = (chat.chat_type == "group" || network_bot(chat, &m.sender_id))
        && m.is_sender != Some(true)
        && !m.silent()
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
svg_static!(TIMER, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path fill="currentColor" d="M6.5 1.553a.58.58 0 0 1-.426-.173A.57.57 0 0 1 5.9.954q0-.252.174-.427a.57.57 0 0 1 .425-.175q.251 0 .426.174A.57.57 0 0 1 7.1.952q0 .251-.173.426a.57.57 0 0 1-.425.175m0 8.1a.58.58 0 0 1-.426-.174.57.57 0 0 1-.175-.424q0-.252.174-.427a.57.57 0 0 1 .425-.175q.251 0 .426.174a.57.57 0 0 1 .175.425q0 .251-.173.426a.57.57 0 0 1-.425.175m2.1-6.3a.58.58 0 0 1-.426-.174A.57.57 0 0 1 8 2.756q0-.252.174-.427a.57.57 0 0 1 .425-.175q.251 0 .426.174a.57.57 0 0 1 .175.425q0 .251-.173.426a.57.57 0 0 1-.425.175m0 4.5a.58.58 0 0 1-.426-.173A.57.57 0 0 1 8 7.255q0-.252.174-.427a.57.57 0 0 1 .425-.175q.251 0 .426.174a.57.57 0 0 1 .175.425q0 .251-.173.426a.57.57 0 0 1-.425.175m.6-2.25a.58.58 0 0 1-.426-.173.57.57 0 0 1-.175-.425q0-.252.174-.427a.57.57 0 0 1 .425-.175q.251 0 .426.174a.57.57 0 0 1 .175.425q0 .251-.173.426a.57.57 0 0 1-.425.175M5 9.803a4.7 4.7 0 0 1-1.866-.375 4.84 4.84 0 0 1-2.56-2.562A4.7 4.7 0 0 1 .2 4.997q0-.993.375-1.863A4.86 4.86 0 0 1 3.134.578 4.7 4.7 0 0 1 5 .203v.9q-1.625 0-2.762 1.138T1.099 5.003t1.138 2.763 2.762 1.137zm1.674-2.675L4.55 5.003v-3h.9v2.625l1.863 1.863z"/></svg>"##);
svg_static!(CALL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M16.193 17.842q-2.605 0-5.146-1.136t-4.625-3.218q-2.083-2.084-3.219-4.625T2.068 3.717q0-.375.25-.625a.85.85 0 0 1 .625-.25h3.375q.292 0 .52.198a.76.76 0 0 1 .272.468l.541 2.917q.042.333-.02.563a.9.9 0 0 1-.23.395l-2.02 2.042q.416.77.989 1.49.573.718 1.26 1.385.645.646 1.355 1.198.708.552 1.5 1.01l1.958-1.958q.187-.188.49-.281.301-.094.593-.052l2.875.583a.97.97 0 0 1 .48.302.73.73 0 0 1 .187.49v3.375q0 .375-.25.625a.85.85 0 0 1-.625.25"/></svg>"##);
svg_static!(ADD_ACCOUNT, include_bytes!("../assets/add_account.svg"));
// Desktop's `reply-in-thread` and `chevron-compact-right` icons, the Material `view_sidebar` and `chevron_forward`.
svg_static!(VIEW_SIDEBAR, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M16,20H2V4h14V20z M18,8h4V4h-4V8z M18,20h4v-4h-4V20z M18,14h4v-4h-4V14z"/></svg>"##);
svg_static!(CHEVRON_FORWARD, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M504-480 320-664l56-56 240 240-240 240-56-56 184-184Z"/></svg>"##);
// Desktop's `undo` icon of the Revert button of a hotkey.
svg_static!(REVERT, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M212.333 224.333H12c-6.627 0-12-5.373-12-12V12C0 5.373 5.373 0 12 0h48c6.627 0 12 5.373 12 12v78.112C117.773 39.279 184.26 7.47 258.175 8.007c136.906.994 246.448 111.623 246.157 248.532C504.041 393.258 393.12 504 256.333 504c-64.089 0-122.496-24.313-166.51-64.215-5.099-4.622-5.334-12.554-.467-17.42l33.967-33.967c4.474-4.474 11.662-4.717 16.401-.525C170.76 415.336 211.58 432 256.333 432c97.268 0 176-78.716 176-176 0-97.267-78.716-176-176-176-58.496 0-110.28 28.476-142.274 72.333h98.274c6.627 0 12 5.373 12 12v48c0 6.627-5.373 12-12 12z"/></svg>"##);
svg_static!(CLAUDE_LOGO, include_bytes!("../assets/integrations/claude.svg"));
svg_static!(ERROR_CIRCLE, include_bytes!("../assets/settings/error_circle.svg"));
svg_static!(CURSOR_LOGO, include_bytes!("../assets/integrations/cursor.svg"));
svg_static!(VSCODE_LOGO, include_bytes!("../assets/integrations/vscode.svg"));
svg_static!(COPY_ICON, include_bytes!("../assets/integrations/copy.svg"));
svg_static!(COPIED_ICON, include_bytes!("../assets/integrations/done.svg"));
svg_static!(ADD_ICON, include_bytes!("../assets/integrations/add.svg"));

/// Desktop's `KJ`: the timer, or "Off".
fn timer_label(ms: u64) -> String {
    if ms == 0 {
        trx("Off", "disappearing messages timer option").into()
    } else {
        timer_distance(ms)
    }
}

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

/// Desktop's compact `PlusPill` of a plan, and its width.
fn plan_pill<'a>(p: &Palette, plan: Plan) -> (El<'a>, f32) {
    let (symbol, name) = match plan {
        Plan::Free => (None, "Free"),
        Plan::Plus => (Some("+"), "Plus"),
        Plan::PlusPlus => (Some("++"), "PlusPlus"),
    };
    let w = text_width(name, 10.0, Weight::Semibold)
        + symbol.map_or(0.0, |s| text_width(s, 12.0, Weight::Semibold) + 2.0)
        + 12.0;
    let symbol = symbol.map(|s| semi(s, 12.0).line_height(1.0).color(p.strong));
    let label = row![].push(symbol).push(semi(name, 10.0).color(p.strong)).spacing(2).align_y(Center);
    (container(label).padding([4, 6]).style(fill_box(p.chip, 25.0)).into(), w)
}

fn badge<'a>(p: &Palette, b: prefs::Badge) -> El<'a> {
    match b {
        prefs::Badge::Plus => {
            let label = row![semi("+", 12.0).line_height(1.0), semi("Plus", 10.0)];
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

// Desktop's `new_set` icon of the New Label button.
svg_static!(NEW_LABEL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M4.3 13.953q-1.15 0-1.729-.564-.57-.571-.571-1.706V4.27q0-1.136.571-1.7Q3.15 2.001 4.3 2h7.353q1.143 0 1.714.571.579.572.579 1.7v1.67h-1.18V4.335q0-.57-.3-.864-.292-.293-.835-.293H4.314q-.55 0-.842.293c-.292.293-.293.484-.293.864v7.28q0 .573.293.865t.842.293h1.839v1.18zm3.552 3.391q-1.15 0-1.728-.571-.572-.564-.572-1.7V7.663q0-1.136.572-1.7.578-.57 1.728-.57h7.354q1.142 0 1.713.57.579.572.579 1.7v7.412q0 1.128-.579 1.699-.57.57-1.713.571zm.015-1.179h7.317q.541 0 .835-.293.3-.293.3-.864v-7.28q0-.572-.3-.865-.294-.293-.835-.293H7.867q-.55 0-.843.293-.293.294-.293.865v7.28q0 .57.293.864.293.293.843.293m4.277-2.036a.62.62 0 0 1-.176.454.6.6 0 0 1-.447.169.59.59 0 0 1-.432-.169.63.63 0 0 1-.168-.454V8.592a.63.63 0 0 1 .168-.454.57.57 0 0 1 .432-.176.6.6 0 0 1 .623.63zm-3.376-2.16a.63.63 0 0 1-.455-.17.57.57 0 0 1-.175-.431q0-.279.168-.447a.62.62 0 0 1 .462-.176h5.537a.6.6 0 0 1 .447.176.6.6 0 0 1 .175.447q0 .263-.175.432a.6.6 0 0 1-.447.168z"/></svg>"##);
svg_static!(DRAFT_RING, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 30 30"><rect x=".5" y=".5" width="29" height="29" rx="7.5" fill="none" stroke="#000" stroke-dasharray="3 2"/></svg>"##);
svg_static!(DASHED_RING, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="1" y="1" width="30" height="30" rx="7" fill="none" stroke="#000" stroke-width="2" stroke-dasharray="6 3" stroke-dashoffset="8.5"/></svg>"##);

static SENT: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    [
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2" stroke-dasharray="2.4 1.84" transform="rotate(-79 6 6)"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="#000"><circle cx="6" cy="6" r="5.4" stroke-width="1.2"/><path d="M3.4 6.1 5.2 7.9 8.6 4.4" stroke-width="1.1"/></svg>"##[..],
    ]
    .map(svg::Handle::from_memory)
});

// Desktop's `MessagePendingIcon`.
svg_static!(PENDING, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><circle cx="6" cy="6" r="5.375" fill="none" stroke="#000" stroke-width="1.25"/><circle cx="3.3" cy="6" r=".9"/><circle cx="6" cy="6" r=".9"/><circle cx="8.7" cy="6" r=".9"/></svg>"##);

svg_static!(SEND_ARROW, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path transform="rotate(90 10 10)" d="m6.52 10.833 4.668 4.667L10 16.666 3.333 10 10 3.333 11.188 4.5 6.52 9.166h10.146v1.667z"/></svg>"##);
svg_static!(EYES, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 46 46" fill="#fff"><path d="M22.1197 25.5092C23.0085 30.1093 20.937 33.1 16.2324 34.009C11.5277 34.918 8.49126 32.9143 7.60244 28.3142L6.68822 23.5827C5.7994 18.9826 7.87085 15.9919 12.5755 15.0829C17.2802 14.1738 20.3166 16.1776 21.2054 20.7777L22.1197 25.5092ZM14.8746 30.5902C15.1743 32.1411 16.6446 33.1113 18.2307 32.8049C19.8168 32.4984 20.8199 31.0503 20.5202 29.4994C20.2205 27.9485 18.7503 26.9783 17.1641 27.2847C15.578 27.5912 14.575 29.0393 14.8746 30.5902Z"/><path d="M39.1455 22.1235C40.0344 26.7236 37.9629 29.7142 33.2583 30.6233C28.5536 31.5323 25.5171 29.5286 24.6283 24.9284L23.7141 20.1969C22.8253 15.5968 24.8967 12.6061 29.6014 11.6971C34.306 10.7881 37.3425 12.7918 38.2313 17.3919L39.1455 22.1235ZM31.9005 27.2045C32.2002 28.7554 33.6704 29.7256 35.2566 29.4191C36.8427 29.1126 37.8457 27.6645 37.5461 26.1136C37.2464 24.5627 35.7761 23.5925 34.19 23.899C32.6039 24.2055 31.6008 25.6536 31.9005 27.2045Z"/></svg>"##);

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

static COMPOSER_ICONS: std::sync::LazyLock<[svg::Handle; 10]> = std::sync::LazyLock::new(|| {
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
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M10.417 2.084A8.333 8.333 0 0 0 9.129 18.65a1.62 1.62 0 0 0 1.39-.464l7.668-7.672c.348-.348.546-.857.463-1.39a8.335 8.335 0 0 0-8.233-7.041M3.75 10.417a6.667 6.667 0 0 1 12.852-2.493 8.333 8.333 0 0 0-8.678 8.678 6.67 6.67 0 0 1-4.174-6.185m5.852 6.33 7.142-7.145a6.667 6.667 0 0 0-7.142 7.146"/></svg>"##[..],
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

svg_static!(SYS_EMOJI, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M12.917 9.167q.52 0 .885-.365t.365-.885-.365-.885a1.2 1.2 0 0 0-.885-.365q-.522 0-.886.365a1.2 1.2 0 0 0-.364.885q0 .52.364.885t.886.365m-5.834 0q.522 0 .886-.365t.364-.885q0-.52-.364-.885a1.2 1.2 0 0 0-.886-.365q-.52 0-.885.365a1.2 1.2 0 0 0-.365.885q0 .52.365.885t.885.365M10 14.584q1.416 0 2.573-.802a4.57 4.57 0 0 0 1.677-2.115h-8.5a4.57 4.57 0 0 0 1.677 2.115q1.156.802 2.573.802m0 3.75a8.1 8.1 0 0 1-3.25-.657 8.4 8.4 0 0 1-2.646-1.78 8.4 8.4 0 0 1-1.781-2.647A8.1 8.1 0 0 1 1.667 10q0-1.729.656-3.25a8.4 8.4 0 0 1 1.781-2.646 8.4 8.4 0 0 1 2.646-1.78A8.1 8.1 0 0 1 10 1.666q1.73 0 3.25.656a8.4 8.4 0 0 1 2.646 1.781 8.4 8.4 0 0 1 1.781 2.646 8.1 8.1 0 0 1 .656 3.25 8.1 8.1 0 0 1-.656 3.25 8.4 8.4 0 0 1-1.781 2.646 8.4 8.4 0 0 1-2.646 1.781 8.1 8.1 0 0 1-3.25.657m0-1.667q2.792 0 4.73-1.938 1.936-1.937 1.937-4.729 0-2.79-1.938-4.729Q12.792 3.334 10 3.334T5.27 5.27t-1.937 4.73q0 2.79 1.938 4.729Q7.209 16.667 10 16.667"/></svg>"##);
svg_static!(SYS_REPLY, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M15.833 15.834V12.5a2.4 2.4 0 0 0-.729-1.77 2.4 2.4 0 0 0-1.77-.73H5.687l3 3L7.5 14.167l-5-5 5-5 1.188 1.167-3 3h7.645q1.73 0 2.948 1.218 1.22 1.22 1.219 2.948v3.334z"/></svg>"##);
svg_static!(SYS_DELETE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M5.833 17.5q-.687 0-1.177-.49a1.6 1.6 0 0 1-.49-1.177V5h-.833V3.333H7.5V2.5h5v.833h4.167V5h-.834v10.833q0 .688-.49 1.177-.489.49-1.176.49zM14.167 5H5.833v10.833h8.334zM7.5 14.167h1.667v-7.5H7.5zm3.333 0H12.5v-7.5h-1.667z"/></svg>"##);
svg_static!(SYS_DOWNLOAD,br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M10 13.333 5.833 9.166 7 7.958l2.167 2.167V3.333h1.666v6.792L13 7.958l1.167 1.208zm-5 3.333q-.687 0-1.177-.49A1.6 1.6 0 0 1 3.333 15v-2.5H5V15h10v-2.5h1.667V15q0 .687-.49 1.177t-1.177.49z"/></svg>"##);

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
svg_static!(MAT_SYNC, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M160-160v-80h110l-16-14q-52-46-73-105t-21-119q0-111 66.5-197.5T400-790v84q-72 26-116 88.5T240-478q0 45 17 87.5t53 78.5l10 10v-98h80v240H160Zm400-10v-84q72-26 116-88.5T720-482q0-45-17-87.5T650-648l-10-10v98h-80v-240h240v80H690l16 14q49 49 71.5 106.5T800-482q0 111-66.5 197.5T560-170Z"/></svg>"##);
svg_static!(MAT_EXPAND_LESS, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M480-528 296-344l-56-56 240-240 240 240-56 56-184-184Z"/></svg>"##);
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

/// Desktop's custom spaces by their space IDs, in the default order.
const RAIL_VIEWS: [(&str, Option<InboxTab>); 7] = [
    ("inbox", Some(InboxTab::AllChats)),
    ("archive", Some(InboxTab::Archived)),
    (LABEL_NUDGE, None),
    ("reminders", Some(InboxTab::Reminders)),
    ("scheduled", Some(InboxTab::SendLater)),
    ("requests", Some(InboxTab::Requests)),
    ("low_priority", Some(InboxTab::LowPriority)),
];
const LABEL_NUDGE: &str = "create-label-nudge";

/// Desktop's `allSpaces` in the default order.
fn all_spaces(st: &BeeperStore) -> Vec<String> {
    let nudge = st.feature("labels-v2").show && st.labels.is_empty() && !st.settings.ever_labeled;
    let views = RAIL_VIEWS.iter().filter(|v| nudge || v.0 != LABEL_NUDGE).map(|v| v.0.to_string());
    views.chain(rail_accounts(st)).collect()
}

/// Desktop's `orderedSidebarSpaces`: the spaces of the `spaces_order` setting in its order, without the hidden
/// ones, then the other spaces in the default order.
fn rail_spaces(st: &BeeperStore) -> Vec<String> {
    ordered_spaces(all_spaces(st), &st.settings.spaces_order)
}

/// The rail ID of Desktop's draft label, which shows while Create Label is open.
/// Desktop's `Jee` sidebar width of 320px, less the 68px space bar.
const SIDEBAR_WIDTH: f32 = 252.0;
/// Desktop's `wxe`: the number of actions that Undo remembers.
const UNDO_DEPTH: usize = 100;
/// Desktop's default toast timeout, counted in ticks while the toast is not paused.
const TOAST_TIMEOUT: Duration = Duration::from_secs(5);
/// Desktop's `Txe` shows its "Undo:" toast for a second.
const UNDO_NOTE_TIMEOUT: Duration = Duration::from_secs(1);
const TOAST_TICK: Duration = Duration::from_millis(250);

/// Desktop's `Oct`: a typing notice ends 3 seconds after the last key.
const TYPING_STOP: Duration = Duration::from_secs(3);
/// Desktop's `toasts-list` shows the last 10 toasts.
const TOASTS_SHOWN: usize = 10;
const LABEL_DRAFT: &str = "label:draft";

/// Desktop's `setDraftLabel`: the draft label named `name` is a label space, and it takes the order slot of the nudge.
fn draft_rail_spaces(st: &BeeperStore, name: &str) -> Vec<String> {
    let mut all: Vec<String> = all_spaces(st).into_iter().filter(|s| s != LABEL_NUDGE).collect();
    let key = name.to_lowercase();
    let after = |s: &String| rail_label(st, s).is_some_and(|l| l.name.to_lowercase() > key);
    all.insert(all.iter().position(after).unwrap_or(all.len()), LABEL_DRAFT.into());
    let order = st.settings.spaces_order.iter().map(|o| match o.id == LABEL_NUDGE {
        true => SpaceOrder { id: LABEL_DRAFT.into(), ..o.clone() },
        false => o.clone(),
    });
    ordered_spaces(all, &order.collect::<Vec<_>>())
}

fn ordered_spaces(mut all: Vec<String>, order: &[SpaceOrder]) -> Vec<String> {
    let at = |id: &str| order.iter().position(|o| o.id == id);
    all.retain(|id| at(id).is_none_or(|i| order[i].visible));
    all.sort_by_key(|id| at(id).unwrap_or(usize::MAX));
    all
}

/// The spaces that Edit Spaces hides, in the `spaces_order` order.
fn hidden_spaces(st: &BeeperStore) -> Vec<String> {
    let all = all_spaces(st);
    let hidden = st.settings.spaces_order.iter().filter(|o| !o.visible && all.contains(&o.id));
    hidden.map(|o| o.id.clone()).collect()
}

/// The `chat_menu` key of the sidebar overflow menu.
const RAIL_MORE: &str = "rail:more";

/// Where `account_menu` shows: on a sidebar space, in the Edit Spaces gear menu, or on the sidebar header.
#[derive(Clone, Copy, PartialEq)]
enum SpaceMenu {
    Rail,
    Gear,
    Header,
    /// The rows of the Accounts settings, like Desktop's `PL` with `hideSettings` and `hideAddRemoveAccounts`.
    Prefs,
    /// The Space Options submenu of the space sheet (`sfe`), which keeps `chat_sub` at `Some(1)` while it is open.
    Options,
}

/// The number of rows of Desktop's label items (`kS`): the name heading, Mute or Unmute, and Label Settings.
fn label_rows(l: &Label) -> f32 {
    2.0 + f32::from(u8::from(!l.name.is_empty()))
}

/// The `chat_menu` key of Desktop's space sheet (`zft`), which opens from the sidebar header title.
const SPACE_SHEET: &str = "space:sheet";
/// The `chat_menu` key of the "Switch Chat" hover card of the composer.
const SWITCH_CARD: &str = "switch:card";

/// The `chat_menu` key of Desktop's sidebar background menu (`Gct`).
const SIDEBAR_MENU: &str = "sidebar:menu";

/// Desktop's `getLastParticipantName`: the first word of the current name of the sender of a chat preview, or the
/// whole name when it is the phone number.
fn sender_first_name(u: &User) -> String {
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let full = u.full_name.as_deref().map(str::trim).filter(|f| !f.is_empty());
    match (full, &u.phone_number) {
        (Some(f), Some(ph)) if !digits(ph).is_empty() && digits(f) == digits(ph) => f.into(),
        _ => member_name(u).split_whitespace().next().unwrap_or_default().into(),
    }
}

/// The link of a spoiler in a message: a click shows the spoilers of the message.
pub(crate) const SPOILER: &str = "beeper-spoiler:";

/// Desktop's `messageSender`: the name of the sender's current member, or the name that came with the message.
fn sender_label(chat: &Chat, m: &Message) -> String {
    let member = chat.participants.items.iter().find(|u| u.id == m.sender_id);
    member.map_or_else(|| m.sender_name.clone().expect("sender name"), member_name)
}

/// Desktop's `mD`: a participant has a name of its own, not only a phone number or an email.
fn real_name(u: &User) -> bool {
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let full = u.full_name.as_deref().filter(|f| !f.is_empty());
    u.custom_nickname.is_some()
        || u.username.is_some()
        || full.is_some_and(|f| {
            !u.phone_number.as_deref().is_some_and(|ph| digits(ph) == digits(f)) && u.email.as_deref() != Some(f)
        })
}

/// Desktop's `Fr`: the name of a participant.
fn member_name(u: &User) -> String {
    [&u.custom_nickname, &u.full_name, &u.username, &u.email]
        .into_iter()
        .find_map(Clone::clone)
        .or_else(|| u.phone_number.as_deref().map(fmt_phone))
        .unwrap_or_else(|| u.id.clone())
}

/// Desktop's `getAltAccountThreads`: the newest direct chat with the other member on each other account, by the
/// first of the username, phone number, email and user ID that finds any, newest first.
fn alt_chats<'s>(st: &'s BeeperStore, chat: &Chat, u: &User) -> Vec<&'s Chat> {
    let keys: [fn(&User) -> Option<&str>; 4] =
        [|u| u.username.as_deref(), |u| u.phone_number.as_deref(), |u| u.email.as_deref(), |u| Some(&u.id)];
    let other = |c: &'s Chat| c.participants.items.iter().find(|u| u.is_self != Some(true));
    let accounts = st.accounts.iter().map(|a| &a.account_id).filter(|a| **a != chat.account_id);
    let accounts: Vec<&String> = accounts.collect();
    keys.iter()
        .find_map(|key| {
            let want = key(u)?;
            let newest = |a: &&String| {
                let dms = st.chats.iter().filter(|c| c.account_id == **a && c.chat_type == "single");
                dms.filter(|c| other(c).and_then(key) == Some(want))
                    .max_by(|x, y| x.last_activity.cmp(&y.last_activity))
            };
            let mut found: Vec<&Chat> = accounts.iter().filter_map(newest).collect();
            found.sort_by(|x, y| y.last_activity.cmp(&x.last_activity));
            (!found.is_empty()).then_some(found)
        })
        .unwrap_or_default()
}

/// A username with Desktop's leading `@`.
fn handle(u: &User) -> Option<String> {
    u.username.as_ref().map(|n| if n.starts_with('@') { n.clone() } else { format!("@{n}") })
}

/// Desktop's user card: its width, the part of its 88 px avatar inside the card, and its height with buttons.
const CARD_W: f32 = 200.0;
const CARD_FACE_IN: f32 = 88.0 / 3.0;
/// The height of a user card with only its avatar and name.
const CARD_H: f32 = 88.0 + 8.0 + 18.0 + 8.0;
/// The Chat Info context menus: the identity header, the title, a member, a bookmark and a contact detail.
const INFO_MENUS: [&str; 5] = ["info:", "copy:", "member:", "bookmark:", "contact:"];

/// The Chat Info menus. The header menus of Desktop's `$fe`: `info:<chat>` has the edit actions and
/// `copy:<chat>`, the menu of the clicked title, adds "Copy <title>" above them. `copy:<chat>\n<text>` copies the
/// clicked line instead.
fn info_menu(st: &BeeperStore, key: &str, dm: impl Fn(&str) -> bool) -> Vec<Option<(String, Option<UiMessage>)>> {
    if let Some((cid, kind)) = key.strip_prefix("contact:").and_then(|k| k.split_once(' ')) {
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).expect("the other member");
        let item = |label: String, msg| Some((label, Some(msg)));
        return match kind {
            "phone" => {
                let n = u.phone_number.as_deref().expect("a phone number");
                let shown = fmt_phone(n);
                vec![
                    item(format!("Call {shown}"), UiMessage::OpenUrl(format!("tel:{n}"))),
                    item(format!("Copy {shown}"), UiMessage::CopyText(shown)),
                ]
            }
            "email" => {
                let e = u.email.clone().expect("an email");
                vec![
                    item(format!("Email {e}"), UiMessage::OpenUrl(format!("mailto:{e}"))),
                    item(format!("Copy {e}"), UiMessage::CopyText(e)),
                ]
            }
            _ => vec![item(tr("Copy Username").into(), UiMessage::CopyText(handle(u).expect("a username")))],
        };
    }
    if let Some((cid, mid)) = key.strip_prefix("bookmark:").and_then(|k| k.split_once(' ')) {
        let jump = st.loaded(mid).is_some().then(|| UiMessage::JumpTo(cid.into(), mid.into()));
        let remove = UiMessage::RemoveBookmark(mid.into());
        return vec![
            Some((tr("Jump to Message").into(), jump)),
            None,
            Some((tr("Remove Bookmark").into(), Some(remove))),
        ];
    }
    if let Some((cid, uid)) = key.strip_prefix("member:").and_then(|k| k.split_once(' ')) {
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let u = c.participants.items.iter().find(|u| u.id == uid).expect("a member of the chat");
        let copy = |label: &'static str, text: Option<String>| {
            text.map(|t| Some((tr(label).to_string(), Some(UiMessage::CopyText(t)))))
        };
        let username = handle(u);
        let call = u.phone_number.as_ref().map(|n| {
            Some((
                trx("Call", "button label to start a phone call").to_string(),
                Some(UiMessage::OpenUrl(format!("tel:{n}"))),
            ))
        });
        let send = dm(&c.account_id)
            .then(|| Some((tr("Send Message").to_string(), Some(UiMessage::MessageMember(cid.into(), uid.into())))));
        let ask = |label: &'static str, ask, on: bool| {
            Some((tr(label).to_string(), on.then(|| UiMessage::AskMember(ask, cid.into(), uid.into()))))
        };
        return [
            call,
            send,
            copy("Copy Phone Number", u.phone_number.as_deref().map(fmt_phone)),
            copy("Copy Email", u.email.clone()),
            copy("Copy Username", username),
            copy("Copy Name", u.full_name.clone()),
            Some(None),
            Some(ask("Make Group Admin…", ASK_MAKE_ADMIN, u.is_admin != Some(true) && u.manage & MANAGE_ROLE != 0)),
            Some(ask("Remove from Group…", ASK_REMOVE_MEMBER, u.manage & MANAGE_KICK != 0)),
        ]
        .into_iter()
        .flatten()
        .collect();
    }
    let (copy, cid) = match key.strip_prefix("copy:") {
        Some(cid) => (true, cid),
        None => (false, key.strip_prefix("info:").expect("an info menu key")),
    };
    let (cid, text) = cid.split_once('\n').map_or((cid, None), |(cid, text)| (cid, Some(text)));
    let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
    let group = can_edit_group(c).then(|| {
        (tr("Edit Group").to_string(), (c.is_request != Some(true)).then(|| UiMessage::OpenEditGroup(c.id.clone())))
    });
    let personal =
        can_edit_for_me(st, c).then(|| (tr("Edit for Me").to_string(), Some(UiMessage::OpenEditForMe(c.id.clone()))));
    let theme = Some((tr("Chat Theme").to_string(), Some(UiMessage::OpenChatTheme(c.id.clone()))));
    let text = text.unwrap_or(&c.title);
    let title = copy.then(|| Some((trf("Copy %s", &[&text]), Some(UiMessage::CopyText(text.to_string())))));
    let edits = [group, personal, theme].into_iter().flatten().map(Some);
    title.into_iter().chain(copy.then_some(None)).chain(edits).collect()
}

/// Desktop's `setPersonalRoomName` gate: the feature is on or the chat has a personal name or photo, and the chat
/// is a Matrix room that is not merged (`canSetPersonalOverrides`, `personalOverridesApply`).
fn can_edit_for_me(st: &BeeperStore, c: &Chat) -> bool {
    let room = c.id.starts_with('!') && c.merge.is_none() && c.merged_into_chat_id.is_none();
    room && (st.feature("personal-room-overrides").on || c.personal.is_some())
}

/// Desktop's sidebar overflow menu (`Plt`): the hidden spaces, then New Label…, Edit Spaces… and Add/Remove Accounts…,
/// with `None` for a separator.
fn more_menu(st: &BeeperStore) -> Vec<Option<(String, UiMessage)>> {
    let unread = st.chats.iter().any(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0);
    let hidden = hidden_spaces(st);
    let mut items: Vec<_> = hidden
        .iter()
        .map(|id| {
            let name = space_name(st, id);
            let label = if id == "requests" && unread { format!("• {name}") } else { name };
            Some((label, space_msg(id)))
        })
        .collect();
    if !hidden.is_empty() {
        items.push(None);
    }
    if st.feature("labels-v2").show {
        items.extend([Some((tr("New Label…").into(), UiMessage::OpenCreateLabel(Vec::new()))), None]);
    }
    items.push(Some((tr("Edit Spaces…").into(), UiMessage::OpenEditSpaces)));
    items.push(Some((tr("Add/Remove Accounts…").into(), UiMessage::OpenSettings(SettingsTab::Accounts))));
    items
}

/// Desktop's `selectSpace` for a menu row of the space `id`. The Create Label space opens Create Label.
fn space_msg(id: &str) -> UiMessage {
    match RAIL_VIEWS.iter().find(|v| v.0 == id).map(|v| v.1) {
        Some(Some(tab)) => UiMessage::SelectTab(tab),
        Some(None) => UiMessage::OpenCreateLabel(Vec::new()),
        None => UiMessage::SelectAccount(Some(id.into())),
    }
}

/// Desktop's `canAcceptRequest` and `canDeclineRequest` of a message request chat.
fn request_answers(c: &Chat) -> (bool, bool) {
    let request = c.is_request == Some(true);
    (request && c.can_accept_request, request && c.delete_modes & DELETE_FOR_ME != 0)
}

/// The account of an account or network space.
fn rail_owner(st: &BeeperStore, id: &str) -> String {
    network_space(st, id).map_or(id, |n| n.account_id.as_str()).to_string()
}

/// Desktop's `getSpaceName`.
fn space_name(st: &BeeperStore, id: &str) -> String {
    match id {
        "inbox" => tr("Inbox").into(),
        "archive" => trx("Archive", "archive space name").into(),
        "reminders" => tr("Reminders").into(),
        "requests" => tr("Requests").into(),
        "low_priority" => tr("Low Priority").into(),
        "scheduled" => tr("Send Later").into(),
        LABEL_NUDGE => tr("Create Label").into(),
        _ => match (rail_label(st, id), network_space(st, id)) {
            (Some(l), _) if l.name.is_empty() => trx("Label", "fallback label space name").into(),
            (Some(l), _) => l.name.clone(),
            (None, Some(n)) => format!("{} ({})", n.name, network_label(&n.account_id)),
            // Desktop's `accountDisplayName`: "<user> · <network>" when the bridge has more than one account.
            (None, None) => account_display_name(st, id, &rail_accounts(st)),
        },
    }
}

/// Desktop's `remoteStatusText`: the state of a bridge login that is not connected.
fn status_text(st: &BeeperStore, account_id: &str) -> Option<&'static str> {
    let a = st.bridge_accounts.iter().find(|a| a.account_id == account_id)?;
    match a.state.as_str() {
        "TRANSIENT_DISCONNECT" => Some("Connection Interrupted"),
        "UNKNOWN_ERROR" => Some("Disconnected"),
        "UNCONFIGURED" => Some("Connection Required"),
        "BAD_CREDENTIALS" => Some("Reconnect Required"),
        // Desktop's `remoteStatusText` for any other state with `remoteHasError`.
        _ => a.remote_error.is_some().then_some("Attention Required"),
    }
}

/// Desktop's `reauthenticate` of an account whose bridge asks for a new login (`computedRequiresReauth`): it opens
/// the Add Account window for the login. beeper-rs logs in to the Beeper Cloud bridges only.
fn reconnect(st: &BeeperStore, account: &str) -> Option<UiMessage> {
    let b = st.bridge_accounts.iter().find(|b| b.account_id == account && b.state == "BAD_CREDENTIALS")?;
    let cloud = beeper_core::config::find_network(&b.bridge_id).and_then(beeper_core::config::cloud_bridge);
    let can = !st.config.demo && b.mode == BridgeMode::Cloud && cloud.is_some();
    can.then(|| UiMessage::OpenAddAccount(b.bridge_id.clone(), Some(b.login_id.clone())))
}

/// Desktop's `remoteHasError` of the account of an account or network space: the status that the space shows.
fn space_error(st: &BeeperStore, id: &str) -> Option<&'static str> {
    rail_label(st, id).is_none().then(|| status_text(st, &rail_owner(st, id))).flatten()
}

/// Desktop's `getSpaceSubtitle`: the chats, reminders, requests or scheduled messages of a space.
fn space_subtitle(st: &BeeperStore, id: &str) -> Option<String> {
    let count = |f: &dyn Fn(&Chat) -> bool| st.chats.iter().filter(|c| c.merged_into_chat_id.is_none() && f(c)).count();
    let listed = |c: &Chat| c.flags & (CHAT_ARCHIVED | CHAT_LOW_PRIORITY) == 0;
    let chats = |n: usize| trn("%d Chat", "%d Chats", n, &[&n]);
    Some(match id {
        "inbox" => chats(count(&|c| c.flags & (CHAT_HIDDEN | CHAT_ARCHIVED | CHAT_LOW_PRIORITY | CHAT_REQUEST) == 0)),
        "archive" => chats(count(&|c| c.flags & CHAT_ARCHIVED != 0)),
        "reminders" => {
            let n = count(&|c| c.reminder.is_some());
            trn("%d Reminder", "%d Reminders", n, &[&n])
        }
        "requests" => {
            let n = count(&|c| c.flags & CHAT_REQUEST != 0);
            trn("%d Request", "%d Requests", n, &[&n])
        }
        "low_priority" => {
            let n = count(&|c| c.flags & CHAT_LOW_PRIORITY != 0);
            trn("%d Low Priority Chat", "%d Low Priority Chats", n, &[&n])
        }
        "scheduled" => trn("%d Message", "%d Messages", st.scheduled.len(), &[&st.scheduled.len()]),
        LABEL_NUDGE => return None,
        _ => chats(count(&|c| in_rail_entry(st, id, c) && listed(c))),
    })
}

/// The account, network and label spaces in the default order.
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
    let id = st.sent.get(id).map_or(id, String::as_str);
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

/// The stored message `id`, which can be in a chat that is not loaded.
fn stored(st: &BeeperStore, id: &str) -> Message {
    st.stored_message(id).expect("a stored message")
}

/// Desktop's `rootMessageIDs`: the first message of the reply chain of `m`, which can be a message that is not loaded.
fn reply_root<'s>(st: &'s BeeperStore, mut m: &'s Message) -> &'s str {
    let mut seen = HashSet::new();
    while m.thread_root.is_none() {
        let Some(r) = m.linked_message_id.as_deref().filter(|r| *r != m.id && seen.insert(&m.id)) else { return &m.id };
        match st.loaded(r) {
            Some(next) => m = next,
            None => return r,
        }
    }
    m.thread_root.as_deref().expect("the loop ends at a thread reply")
}

/// Desktop's `canBroadcastOutsideThread`: threads in a Slack (Desktop's `slackgo`) or a Beeper chat.
fn can_broadcast(st: &BeeperStore, chat: &Chat) -> bool {
    st.threaded(chat) && matches!(account_network(&chat.account_id).bridge_type, "slack" | "hungryserv")
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
        let group = || st.chat(&m.chat_id).chat_type == "group";
        let at = || millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp");
        m.is_deleted != Some(true)
            && room.is_none_or(|r| m.chat_id == r)
            && (self.accounts.is_empty() || self.accounts.contains(&m.account_id))
            && self.sent.is_none_or(|s| (m.is_sender == Some(true)) == s)
            && self.group.is_none_or(|g| group() == g)
            && self.period.is_none_or(|_| {
                let (after, before) = self.window();
                (after..before).contains(&at())
            })
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
    cubic_bezier(x, [0.22, 1.0, 0.36, 1.0])
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at `x`.
fn cubic_bezier(x: f32, [x1, y1, x2, y2]: [f32; 4]) -> f32 {
    let bezier =
        |t: f32, p1: f32, p2: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * p1 + 3.0 * (1.0 - t) * t * t * p2 + t.powi(3);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, y1, y2)
}

/// Desktop's `IJ`: when a pending message shows "Still sending...", 5 s after it, or 12 s with attachments.
fn still_sending_at(m: &Message) -> i64 {
    let wait = if m.attachments.is_empty() { 5_000 } else { 12_000 };
    millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp") + wait
}

/// Desktop's `animate-on-insert` for your pending message, with "Enable message animations" on: the eased
/// progress of its 260 ms `message-send-insert` animation.
fn send_rise(st: &BeeperStore, m: &Message) -> Option<f32> {
    let age = now_unix_millis() - millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp");
    let pending = m.is_sender == Some(true) && m.id.starts_with("msg_");
    (pending && age < 260 && st.settings.on("messageAnimations")).then(|| ease_out_quint(age.max(0) as f32 / 260.0))
}

/// Desktop's `FocusScope.activate`: a composer action or a message selection activates the scope of its pane.
fn activates(msg: &UiMessage) -> bool {
    matches!(msg, UiMessage::ComposerEdit(_) | UiMessage::ClickMessage(_) | UiMessage::SelectMessages(_))
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
        let anchored = matches!(self.4, "timeline" | "thread-timeline");
        iced::advanced::widget::operation::Outcome::Some(if anchored { max - top } else { top })
    }
}

/// The forward dialog: the messages in timeline order, the picked chats, and the search text.
struct Forward {
    mids: Vec<String>,
    picked: Vec<String>,
    query: String,
}

/// If you can delete a message for everyone, like Desktop's Media window `G`: the chat fully supports deletion, you
/// sent the message, and it is younger than the `delete_max_age` of the network.
fn deletable(st: &BeeperStore, m: &Message) -> bool {
    let rules = st.chat(&m.chat_id).messages;
    rules.delete
        && m.is_sender == Some(true)
        && m.is_deleted != Some(true)
        && MessageRules::young(rules.delete_max_age, &m.timestamp)
}

/// Desktop's `ghe`: the text and the buttons of the delete dialog of messages, or None without buttons. Your messages
/// get "Delete for Everyone" or "Delete for Me" by the deletion mode of their rooms when no message of others is in
/// `ms`, and the messages that the homeserver took get "Remove from Beeper". A deleted message has no buttons.
fn delete_dialog(st: &BeeperStore, ms: &[&Message]) -> Option<(String, Vec<(&'static str, UiMessage)>)> {
    if ms.iter().any(|m| m.is_deleted == Some(true)) {
        return None;
    }
    let (mine, others): (Vec<&Message>, Vec<&Message>) =
        ms.iter().copied().filter(|m| !m.silent()).partition(|m| m.is_sender == Some(true));
    // Desktop's `Vit`.
    let mode = |m: &Message| {
        let rules = st.chat(&m.chat_id).messages;
        match rules.delete {
            true => MessageRules::young(rules.delete_max_age, &m.timestamp).then_some("Delete for Everyone"),
            false => rules.delete_for_me.then_some("Delete for Me"),
        }
    };
    let modes: Vec<Option<&'static str>> = mine.iter().map(|m| mode(m)).collect();
    let label = modes.first().copied().flatten().filter(|&l| others.is_empty() && modes.iter().all(|&m| m == Some(l)));
    let delete = mine.iter().map(|m| UiMessage::DeleteMessage(m.chat_id.clone(), m.id.clone())).collect();
    let mut buttons: Vec<(&'static str, UiMessage)> =
        label.map(|l| (trx(l, "confirmation button"), UiMessage::Commands(delete))).into_iter().collect();
    let removable: Vec<&Message> = ms.iter().copied().filter(|m| !m.silent() && !pending(st, m)).collect();
    if !removable.is_empty() {
        let ids = removable.iter().map(|m| m.id.clone()).collect();
        buttons.push((trx("Remove from Beeper", "confirmation button"), UiMessage::RemoveFromBeeper(ids)));
    }
    let n = mine.len();
    let body = match (removable.is_empty(), n) {
        (false, _) => removal_note(st, &removable),
        (true, 1) => tr("Are you sure you want to delete this message?").into(),
        (true, n) => {
            trn("Are you sure you want to delete %d message?", "Are you sure you want to delete %d messages?", n, &[&n])
        }
    };
    (!buttons.is_empty()).then_some((body, buttons))
}

/// Desktop's `Git`: where "Remove from Beeper" keeps the messages `ms`, by their chat.
fn removal_note(st: &BeeperStore, ms: &[&Message]) -> String {
    let n = ms.len();
    let c = st.chat(&ms[0].chat_id);
    if ms.iter().any(|m| m.chat_id != c.id) {
        let (one, many) = (
            "Removing it from Beeper keeps it on its original network.",
            "Removing them from Beeper keeps them on their original networks.",
        );
        return trn(one, many, n, &[]);
    }
    let (title, network) = (c.title.trim(), network_name(&c.account_id));
    let ((one, many), args): ((&'static str, &'static str), Vec<&dyn std::fmt::Display>) =
        match (c.account_id == "hungryserv", c.chat_type == "group", title.is_empty()) {
            (true, true, _) => (
                (
                    "Removing it from Beeper keeps it for you and the group.",
                    "Removing them from Beeper keeps them for you and the group.",
                ),
                vec![],
            ),
            (true, false, false) => (
                (
                    "Removing it from Beeper keeps it for you and %s.",
                    "Removing them from Beeper keeps them for you and %s.",
                ),
                vec![&title],
            ),
            (true, false, true) => (
                (
                    "Removing it from Beeper does not delete it for other people.",
                    "Removing them from Beeper does not delete them for other people.",
                ),
                vec![],
            ),
            (false, true, _) => (
                (
                    "Removing it from Beeper keeps it on %s for you and the group.",
                    "Removing them from Beeper keeps them on %s for you and the group.",
                ),
                vec![&network],
            ),
            (false, false, false) => (
                (
                    "Removing it from Beeper keeps it on %1$s for you and %2$s.",
                    "Removing them from Beeper keeps them on %1$s for you and %2$s.",
                ),
                vec![&network, &title],
            ),
            (false, false, true) => (
                (
                    "Removing it from Beeper keeps it on %s for everyone in this chat.",
                    "Removing them from Beeper keeps them on %s for everyone in this chat.",
                ),
                vec![&network],
            ),
        };
    trn(one, many, n, &args)
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
        true => st.recent_chat_ids.iter().filter_map(|id| st.find_chat(id)).collect(),
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

/// The look of reaction pills: faces also for one reaction (in groups), the colors with and without your reaction and
/// on hover (if it changes), the text and "+N" colors, the ring color and width, and the gap between the pills.
struct PillLook {
    group: bool,
    own: Color,
    other: Color,
    hover: Option<Color>,
    fg: (Color, Color),
    ring: (Color, f32),
    gap: f32,
}

static LOGO: std::sync::LazyLock<svg::Handle> =
    std::sync::LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/beeper.svg")));
svg_static!(TAKE_BEEPER, include_bytes!("../assets/take_beeper.svg"));
// Desktop's `MessageErroredIcon`, Material's error_outline.
svg_static!(ERROR_OUTLINE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M11 15h2v2h-2zm0-8h2v6h-2zm.99-5C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8z"/></svg>"##);
// Desktop's `BookmarkIcon` of the general icons.
svg_static!(BOOKMARK_ICON, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 14"><path d="M3.5 0.5H10.5C11.3284 0.5 12 1.17157 12 2V13.0625C12 13.2499 11.8949 13.4215 11.7279 13.5068C11.5608 13.5921 11.36 13.5767 11.2081 13.4669L7 10.4272L2.79188 13.4669C2.64 13.5767 2.43918 13.5921 2.27212 13.5068C2.10506 13.4215 2 13.2499 2 13.0625V2C2 1.17157 2.67157 0.5 3.5 0.5Z"/></svg>"##);
// Desktop's `BeeperIcon` of the networks icons.
svg_static!(BEEPER_ICON, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 15"><path fill-rule="evenodd" clip-rule="evenodd" d="M0.350037 5.29941C0.350037 2.76021 2.33427 0.649414 4.84524 0.649414H8.75483C11.2658 0.649414 13.25 2.76021 13.25 5.29941C13.25 6.61227 12.7229 7.80414 11.8693 8.65264L7.23829 13.8348C6.60047 14.5486 5.42971 13.9544 5.62929 13.0182L6.2835 9.94942H4.84524C2.33427 9.94942 0.350037 7.83862 0.350037 5.29941ZM4.84524 2.14941C3.21937 2.14941 1.85004 3.5308 1.85004 5.29941C1.85004 7.06803 3.21937 8.44941 4.84524 8.44941H8.13699L7.54101 11.245L10.7824 7.61781L10.7997 7.60083C11.382 7.02833 11.75 6.2123 11.75 5.29941C11.75 3.5308 10.3807 2.14941 8.75483 2.14941H4.84524Z"/></svg>"##);

/// Desktop's `hasOlderMessages`: the chat has events before its first loaded page.
fn has_older(st: &BeeperStore, chat: &Chat) -> bool {
    st.matrix.is_some() && chat.older != Older::Start
}
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

/// Desktop's `Toasts-module__container`: no border, and the `--shadow-medium-15` shadow.
fn toast_box(bg: Color, p: &Palette) -> impl Fn(&Theme) -> container::Style + Copy {
    glow_box(bg, p.shadow, 10.0, 1.0, 10.0)
}

fn tinted<'a>(h: &svg::Handle, size: f32, color: Color) -> El<'a> {
    svg(h.clone()).width(size).height(size).opacity(color.a).style(move |_, _| svg::Style { color: Some(color) }).into()
}

/// Desktop's `Archive`, `Inbox` and `LowPriority` assets, which its archive and Low Priority toasts show, its `Undo`
/// icon, and its `Lock` icon of the recovery key reminder.
static TOAST_ICONS: std::sync::LazyLock<[svg::Handle; 5]> = std::sync::LazyLock::new(|| {
    [
        "M4.556 17a1.5 1.5 0 0 1-1.1-.457A1.5 1.5 0 0 1 3 15.444V5.722q0-.292.097-.525.098-.232.253-.447l1.089-1.322a1 1 0 0 1 .389-.321Q5.06 3 5.333 3h9.334q.272 0 .505.107a1 1 0 0 1 .39.32L16.65 4.75q.156.214.253.447t.097.525v9.722a1.5 1.5 0 0 1-.457 1.1 1.5 1.5 0 0 1-1.099.456zm.31-11.667h10.267l-.66-.777H5.527zm-.31 1.556v8.555h10.888V6.89zM10 14.667l3.111-3.111-1.089-1.09-1.244 1.245V8.444H9.222v3.267l-1.244-1.244-1.09 1.089z",
        "M15.833 2.5H4.158c-.925 0-1.65.742-1.65 1.667L2.5 15.833A1.66 1.66 0 0 0 4.158 17.5h11.675c.917 0 1.667-.75 1.667-1.667V4.167c0-.925-.75-1.667-1.667-1.667m0 10H12.5c0 1.383-1.125 2.5-2.5 2.5a2.5 2.5 0 0 1-2.5-2.5H4.158V4.167h11.675z",
        "M11.667 4.167h6.666v1.556h-6.666zm0 4.278h6.666V10h-6.666zm0 4.278h6.666v1.555h-6.666zm-10-3.5c0 2.784 2.433 5.055 5.416 5.055H7.5v1.556L10 13.5l-2.5-2.333v1.556h-.417c-2.066 0-3.75-1.572-3.75-3.5 0-1.93 1.684-3.5 3.75-3.5H10V4.167H7.083c-2.983 0-5.416 2.271-5.416 5.056",
        "M5.833 15.833v-1.667h5.917q1.312 0 2.281-.833.969-.834.969-2.083t-.969-2.084q-.969-.833-2.281-.833H6.5L8.667 10.5 7.5 11.666 3.333 7.5 7.5 3.333 8.667 4.5 6.5 6.666h5.25q2.021 0 3.469 1.313t1.448 3.27-1.448 3.271-3.469 1.313z",
        "M9.996 2.712a3.9 3.9 0 0 1 1.86.461q.865.462 1.392 1.377.535.909.535 2.264v1.745q.569.079.9.423.433.447.433 1.333v5.178q0 .9-.432 1.347t-1.26.448H6.576q-.827 0-1.26-.448-.432-.446-.432-1.347v-5.178q0-.886.432-1.333.333-.344.9-.423V6.814q0-1.355.528-2.264.535-.915 1.4-1.377a3.9 3.9 0 0 1 1.852-.46m0 1.296q-.651 0-1.208.308-.55.308-.879.901-.33.594-.33 1.457v1.86h4.849v-1.86q0-.864-.337-1.457a2.25 2.25 0 0 0-.886-.901 2.43 2.43 0 0 0-1.209-.308",
    ]
    .map(|d| svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="{d}"/></svg>"#).into_bytes()))
});

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

/// The width of a GIF of the carousel, 215 px high like Desktop's `tinymp4` videos.
fn gif_width(g: &KlipyGif) -> f32 {
    let (w, h) = g.preview_size;
    (215.0 * w as f32 / h.max(1) as f32).round().max(48.0)
}

/// Loads the page of GIFs at `pos` for the query `q`.
fn gif_page(q: String, pos: String) -> Task<UiMessage> {
    let more = !pos.is_empty();
    Task::perform(beeper_core::cloud::klipy_gifs(q.clone(), pos), move |r| UiMessage::GifsLoaded(q.clone(), more, r))
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
        .push(title.map(|t| container(txt(t.to_string(), 13.0).color(p.muted)).padding([0, 13])))
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
    /// The messages of the gallery whose menu is open.
    pub gallery_menu: Vec<String>,
    /// The reactions list of the messages of a gallery, at a point, from the gallery menu.
    pub gallery_reactions: Option<(Vec<String>, Point)>,
    /// The composer context menu, with the misspelled word and its suggestions.
    pub edit_menu: Option<(Point, Option<Typo>)>,
    /// The selected messages, and the message that a Shift click selects from.
    pub selected_msgs: Vec<String>,
    pub msg_anchor: Option<String>,
    pub show_attach_menu: bool,
    /// Desktop's `visiblePicker`.
    pub picker: Option<Picker>,
    /// The sticker packs of the open sticker picker, until they load, and its selected tab: Recent (`None`) or a pack.
    pub stickers: Option<Result<Vec<StickerPack>, String>>,
    pub sticker_tab: Option<Option<String>>,
    /// The URL and text of the formatting menu's link dialog.
    pub link_dialog: Option<(String, String)>,
    pub emoji: Option<emoji::Picker>,
    pub composer: text_editor::Content,
    pub reply_to_message_id: Option<String>,
    pub editing_message_id: Option<String>,
    pub expanded_history_msg_id: Option<String>,
    /// Desktop's `unclampedMessages`: the long messages whose "Show more" is open.
    pub unclamped: HashSet<String>,
    /// The messages whose spoilers were clicked open (Desktop's `.spoiler.clicked`).
    pub spoilers: HashSet<String>,
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
    pub top: TopOfThread,
    timeline: vlist::VList,
    thread_list: vlist::VList,
    /// The messages of the conversation and their rows, for a store generation and a chat.
    rows: std::cell::RefCell<Option<(u64, String, Rc<Rows>)>>,
    /// Desktop's `stopTypingTimeout`: the chat with a typing notice, and the time of the last key.
    pub typing: Option<(String, std::time::Instant)>,
    /// The mention pills of the composer text, which Desktop sends as `mentionedUserIDs`.
    pub mentions: Vec<Mention>,
    /// Desktop's `pendingMessageDraft`: the composer state that an edit puts aside. Set while `editing_message_id` is.
    pub stash: Option<Stash>,
    /// The composer changed after the last save of the draft. Desktop saves it at most every 5 seconds.
    pub draft_due: bool,
    /// Desktop's `ScrollToBottom`: whether you scrolled up from the bottom of the timeline, and since when.
    pub scrolled_up: Option<(bool, std::time::Instant)>,
    /// Desktop's `ThreadedRepliesView`: the root of the thread that this pane shows in place of the chat.
    pub thread_root: Option<String>,
    /// Desktop's `openThreadedReplies`: the pane of the open thread, at the right of the chat.
    pub thread: Option<Box<Pane>>,
    /// Desktop's `broadcastOutsideThread` of the thread composer: "Also send to chat".
    pub broadcast: bool,
    /// The "More Options" menu of the thread pane.
    pub thread_menu: Option<Point>,
    /// Desktop's `replyThreadDividerOnOpen`: the read mark of the thread when the pane opened.
    pub thread_divider: Option<String>,
    /// Desktop's active `FocusScope`: the thread pane takes the message keys. The open of the pane, a composer
    /// action or a message selection in the pane activates it, and one in the chat activates the chat.
    pub thread_focus: bool,
}

impl Pane {
    /// The id of the scrollable of the timeline of the pane.
    fn scroll_id(&self) -> &'static str {
        if self.thread_root.is_some() {
            "thread-timeline"
        } else {
            "timeline"
        }
    }

    /// Desktop's `startEditMessageCommon`: puts the composer state aside, unless an edit already did, and edits `mid`.
    fn start_edit(&mut self, mid: String, text: &str, mentions: Vec<Mention>) {
        if self.editing_message_id.is_none() {
            self.stash = Some(Stash {
                text: self.composer.text(),
                reply_to: self.reply_to_message_id.take(),
                attachments: std::mem::take(&mut self.attachments),
                links: std::mem::take(&mut self.composer_links),
                send_later: self.send_later.take(),
                mentions: std::mem::take(&mut self.mentions),
            });
        }
        self.editing_message_id = Some(mid);
        self.composer = composer_with(text);
        self.mentions = mentions;
    }

    /// Desktop's `restoreDraftState`: ends the edit and puts the composer state back.
    fn end_edit(&mut self) {
        let s = self.stash.take().expect("an edit stashed the composer");
        self.editing_message_id = None;
        self.composer = composer_with(&s.text);
        (self.reply_to_message_id, self.attachments, self.composer_links) = (s.reply_to, s.attachments, s.links);
        self.send_later = s.send_later;
        self.mentions = s.mentions;
    }

    /// Desktop's `getMessageSendOptions`: the quoted message, else the message of the open reply chain.
    fn reply_target(&mut self) -> Option<String> {
        self.reply_to_message_id.take().or_else(|| self.reply_thread.clone())
    }

    /// The thread of a send from the pane: the root of a thread pane, and its "Also send to chat".
    fn send_thread(&self) -> Option<(String, bool)> {
        self.thread_root.clone().map(|root| (root, self.broadcast))
    }
}

/// The tab, the filters and the account of a chat list.
type ListKey = (InboxTab, u8, Option<String>);

/// Values that views compute from the store, kept until the next write to the store.
#[derive(Default)]
struct Memo {
    generation: std::cell::Cell<u64>,
    /// The chat list by tab, filters and account, as indices of `chats`.
    lists: std::cell::RefCell<HashMap<ListKey, Rc<[usize]>>>,
    /// The last chat list of each tab, filters and account, from any store generation, where the next sort starts.
    last: std::cell::RefCell<HashMap<ListKey, Rc<[usize]>>>,
    counts: std::cell::RefCell<HashMap<String, usize>>,
    /// The virtual list keys of the chat rows by tab, filters and account.
    keys: std::cell::RefCell<HashMap<ListKey, Rc<[u64]>>>,
    /// All chats newest first, and the lowercase title and search subtitle of each chat.
    words: std::cell::RefCell<Option<Words>>,
    /// The command bar finds for a query, its filters and its room.
    hits: std::cell::RefCell<Option<(HitsKey, Hits)>>,
    /// The stored messages with media, newest first.
    media: std::cell::RefCell<Option<Rc<[Message]>>>,
    /// The unread threads of all chats, as the chat index and the root.
    threads: std::cell::RefCell<Option<Threads>>,
}

/// The query, the filters and the room of a command bar search.
type HitsKey = (String, SearchFilters, Option<String>);

/// The text and the media messages that a command bar search finds.
type Hits = Rc<(Vec<Message>, Vec<Message>)>;

/// All chats newest first, as indices of `chats`, and the lowercase title and search subtitle of each chat.
type Words = (Rc<[usize]>, Rc<[String]>);

pub struct BeeperGuiApp {
    pub pane: Pane,
    /// The chat windows and their panes.
    chat_windows: Vec<(window::Id, Pane)>,
    /// The chat window that `view` is drawing, so `pane` gives its pane.
    drawing: std::cell::Cell<Option<window::Id>>,
    /// `view` draws the thread pane of the drawn pane.
    drawing_thread: std::cell::Cell<bool>,
    /// Desktop's `threadedRepliesPaneFraction`: the part of the chat area that the thread pane takes.
    thread_fraction: f32,
    /// Desktop's `openThreadedReplies` of each chat: the root of the thread pane that the chat shows when you come back.
    open_threads: HashMap<String, String>,
    /// The width of the chat area at the last draw, where a drag of the thread pane edge starts.
    thread_area: std::cell::Cell<f32>,
    /// The thread pane edge follows the pointer.
    thread_drag: bool,
    /// Desktop's `rightPaneView` of `UNREAD_THREADS`.
    unread_view: bool,
    /// The menu of the "Unread threads" pill.
    unread_menu: Option<Point>,
    pub store: SharedStore,
    pub os: OsKind,
    pub display_server: DisplayServerKind,
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
    pub hotkey_rec: Option<HotkeyRec>,
    pub profile: profile::ProfileForm,
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
    /// Desktop's chat selection mode from Select Chats, before the first chat is picked.
    select_mode: bool,
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
    notifications: HashMap<u32, (String, String)>,
    pending: notify::Pending,
    pub gif_query: String,
    pub gifs: Vec<KlipyGif>,
    pub gif_loading: bool,
    pub gif_error: Option<String>,
    /// Desktop's `paginationPos` of the next page, which is empty after the last page.
    pub gif_next: String,
    /// Desktop's selected GIF of the arrow keys.
    pub gif_sel: Option<usize>,
    /// The offset and the width of the GIF carousel.
    gif_view: (f32, f32),
    pub command_palette_query: String,
    pub cmd_bar_commands_mode: bool,
    pub cmd_submenu: Option<commands::SubMenu>,
    /// The Remind Me submenu's "If no reply" choice.
    pub cmd_no_reply: bool,
    /// The highlighted command.
    pub cmd_index: usize,
    pub search_expanded: bool,
    pub creating_chat: Option<u32>,
    pub new_chats: Vec<NewChat>,
    chat_list: vlist::VList,
    command_list: vlist::VList,
    memo: Memo,
    /// The room of a direct chat that the bridge made, to select when it syncs, and the quote for its composer.
    pub pending_dm: Option<(String, Option<String>)>,
    /// The bridge provisioning capabilities of the chat accounts.
    pub caps: HashMap<String, serde_json::Value>,
    /// The expanded New Chat contact and its networks.
    pub contact_networks: Option<(String, Networks)>,
    /// The networks of the typed identifier, by "lookup:" and the query.
    pub new_chat_lookup: Option<(String, Networks)>,
    /// Desktop's `Connections` of New Chat: the query, the accounts that still search, and the users found.
    pub new_chat_search: Option<(String, usize, People)>,
    pub new_chat_results_expanded: bool,
    pub new_chat_account: Option<String>,
    pub new_chat_accounts_open: bool,
    pub new_chat_query: String,
    pub new_chat_expanded: bool,
    pub new_chat_chats_expanded: bool,
    pub join_matrix_mode: bool,
    /// Desktop's `matrixRoomJoin` of the Join a Matrix room view: a join is going on, and the error of the last one.
    join_room: (bool, Option<String>),
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
    pub confirm: Option<u8>,
    delete_target: Option<String>,
    /// Desktop's `loading` of the device list: a logout of another device runs.
    devices_busy: bool,
    /// The "Report spam" checkbox of the Block dialog.
    report_spam: bool,
    mods: keyboard::Modifiers,
    reminder_menu: Option<Point>,
    audio: voice::Player,
    recorder: Option<record::Recorder>,
    talk: Option<record::Talk>,
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
    /// The newest toast with its icon and buttons, shown only while the banner still is the text it was made for.
    toast: Option<Toast>,
    /// The earlier toasts of Desktop's `toasts-list` that still show above the newest, each with the time it showed.
    older_toasts: Vec<(Toast, Duration)>,
    /// How many times the newest toast was dismissed, so that a dismissed toast does not move up.
    toast_dismissals: u32,
    /// Desktop's action stack `Jv`: what each action did and the message that undoes it.
    undo_stack: Vec<(String, UiMessage)>,
    /// What the shown "Undo:" toast undid.
    undo_note: Option<String>,
    /// The toast text the timeout counts for, whether the pointer is on it, and the time it showed.
    toast_clock: (String, bool, Duration),
    /// Chromium's `navigator.onLine`.
    online: bool,
    /// The pointer is on the "Connecting..." status pill, which then names the accounts.
    pill_hover: bool,
    /// A toast that stays until it is replaced, like Desktop's timeout 0.
    sticky_banner: Option<String>,
    pub dnd_muted: bool,
    pub sidebar_width: f32,
    /// Whether the hidden space bar floats over the chat list, like Desktop's `account-switcher--floating`.
    rail_float: bool,
    /// Desktop's local `sidebar-hidden`: hides the rail and the chat list.
    pub sidebar_hidden: bool,
    pub dragging: bool,
    /// The rail space under the cursor, and where the cursor is in it.
    pub rail_hover: Option<(String, Point)>,
    pub user_hover: UserHover,
    pub rail_drag: Option<RailDrag>,
    /// Desktop's header hover (`f` of `Amt`): the member list shows, and the generation of its 500 ms leave delay.
    pub header_hover: (bool, u32),
    /// Desktop's menu bar, which Alt shows: hidden (`None`), shown, or shown with the menu of an index open.
    pub menu_bar: Option<Option<usize>>,
    /// Alt went down with no other key, so that its release toggles the menu bar.
    alt_alone: bool,
    pub pin_hover: Option<String>,
    pub pin_drag: Option<PinDrag>,
    pub main_window: Option<window::Id>,
    pub settings_window: Option<window::Id>,
    settings_size: Size,
    pub login_window: Option<window::Id>,
    /// Desktop's `ChatThemeBrowserWindow`s and their chats.
    pub theme_windows: Vec<(window::Id, String)>,
    pub labels_windows: Vec<(window::Id, String)>,
    pub poll_windows: Vec<PollDraft>,
    pub label_window: Option<LabelDraft>,
    pub label_settings: Vec<LabelSettings>,
    /// The window that shows the open Delete Label confirm, like Desktop's modal in that window.
    pub delete_in: Option<window::Id>,
    pub spaces_window: Option<SpacesDraft>,
    pub group_window: Option<GroupDraft>,
    /// Desktop's Blocked users window.
    blocked_window: Option<window::Id>,
    pub add_accounts: Vec<add_account::AddAccount>,
    /// The Chat Info group description while it is edited, and whether it shows all of its lines.
    pub info_desc: Option<text_editor::Content>,
    pub info_more: bool,
    /// Whether the Chat Info Members section shows all of its members, and its search text.
    pub info_all: bool,
    /// Whether the Chat Info Bookmarks show all bookmarks, not only Desktop's 15.
    pub info_bookmarks: bool,
    /// The bookmarked messages that Chat Info fetches.
    hydrating: HashSet<String>,
    /// The Media tab of the expanded Chat Info Media section, or None while it shows only the newest.
    pub info_media: Option<usize>,
    pub info_query: String,
    /// The Notes of the open chat in the Chat Info panel.
    pub info_note: text_editor::Content,
    /// The poll votes windows with their chats and poll messages.
    pub vote_windows: Vec<(window::Id, String, String)>,
    /// The window that `BEEPER_SCREENSHOT_EXIT` or `BEEPER_SCREENSHOT_SETTINGS` captures after its next frame.
    shot: Option<window::Id>,
    pub login: Login,
    pub verify: verify::Verify,
    pub reauth: reauth::Reauth,
    pub enter_key: enter_key::EnterKey,
    pub cloud: Arc<Cloud>,
    pub avatars: images::Images<image::Handle>,
    wants: wants::Wants,
    pub media: images::Images<Result<Frames, String>>,
    /// The baked gallery tiles (`gallery.rs`).
    tiles: images::Images<image::Handle>,
    /// Desktop's account `messages` cache for quotes: a quoted message out of the timeline, `None` while it loads.
    pub(crate) quotes: HashMap<String, Option<Result<Message, String>>>,
    /// Desktop's `dismissError`: the accounts whose disconnected banner was dismissed this session.
    dismissed_errors: HashSet<String>,
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
            // Desktop's `InnerApp` shows the home page at start only when the onboarding is done.
            let first = (st.signed_in() && st.beeper_state.stage() == "done").then(|| NOTE_CHAT_ID.to_string());
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
        let st = store.read();
        let demo = serde_json::json!({
            "resolve_identifier": { "create_dm": true, "any_phone": true },
            "group_creation": { "group": { "name": { "allowed": true }, "participants": { "min_length": 2 } } },
        });
        let resolve =
            st.bridge_accounts.iter().filter(|_| st.config.demo).map(|b| (b.account_id.clone(), demo.clone()));
        let resolve = resolve.collect();
        drop(st);
        let sidebar_width = match std::fs::read_to_string(store.read().config.data_dir.join("sidebar-width")) {
            Ok(w) => w.parse().expect("a numeric sidebar width"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SIDEBAR_WIDTH,
            Err(e) => panic!("read the sidebar width: {e}"),
        };
        let audio = voice::Player::new(&store.read().config.data_dir);
        let sidebar_hidden = match std::fs::read_to_string(store.read().config.data_dir.join("sidebar-hidden")) {
            Ok(h) => h.parse().expect("a boolean sidebar-hidden"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("read sidebar-hidden: {e}"),
        };
        let mut app = Self {
            pane: Pane { size: MAIN_SIZE, ..Pane::default() },
            chat_windows: Vec::new(),
            drawing: Default::default(),
            drawing_thread: Default::default(),
            thread_fraction: THREAD_FRACTION,
            open_threads: HashMap::new(),
            thread_area: Default::default(),
            thread_drag: false,
            unread_view: false,
            unread_menu: None,
            store,
            os,
            display_server,
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
            hotkey_rec: None,
            profile: profile::ProfileForm::default(),
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
            gif_next: String::new(),
            gif_sel: None,
            gif_view: (0.0, 0.0),
            command_palette_query: String::new(),
            cmd_bar_commands_mode: false,
            cmd_submenu: None,
            cmd_no_reply: true,
            cmd_index: 0,
            search_expanded: false,
            creating_chat: None,
            new_chats: Vec::new(),
            chat_list: vlist::VList::default(),
            command_list: vlist::VList::default(),
            memo: Memo::default(),
            pending_dm: None,
            caps: resolve,
            contact_networks: None,
            new_chat_lookup: None,
            new_chat_search: None,
            new_chat_results_expanded: false,
            new_chat_account: None,
            new_chat_accounts_open: false,
            new_chat_query: String::new(),
            new_chat_expanded: false,
            new_chat_chats_expanded: false,
            join_matrix_mode: false,
            join_room: (false, None),
            devices: None,
            mics: None,
            confirm: None,
            delete_target: None,
            devices_busy: false,
            report_spam: false,
            reminder_menu: None,
            audio,
            recorder: None,
            talk: None,
            queued: Vec::new(),
            selection: Vec::new(),
            select_mode: false,
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
            toast: None,
            older_toasts: Vec::new(),
            toast_dismissals: 0,
            undo_stack: Vec::new(),
            undo_note: None,
            toast_clock: (String::new(), false, Duration::ZERO),
            online: true,
            pill_hover: false,
            sticky_banner: None,
            dnd_muted: false,
            sidebar_width,
            rail_float: false,
            sidebar_hidden,
            dragging: false,
            rail_hover: None,
            header_hover: (false, 0),
            menu_bar: None,
            alt_alone: false,
            pin_hover: None,
            pin_drag: None,
            user_hover: UserHover::default(),
            rail_drag: None,
            main_window: None,
            settings_window: None,
            settings_size: SETTINGS_SIZE,
            login_window: None,
            theme_windows: Vec::new(),
            labels_windows: Vec::new(),
            label_settings: Vec::new(),
            delete_in: None,
            poll_windows: Vec::new(),
            label_window: None,
            group_window: None,
            blocked_window: None,
            add_accounts: Vec::new(),
            info_desc: None,
            info_more: false,
            info_all: false,
            info_bookmarks: false,
            hydrating: HashSet::new(),
            info_media: None,
            info_query: String::new(),
            info_note: text_editor::Content::new(),
            spaces_window: None,
            vote_windows: Vec::new(),
            shot: None,
            login: Login::new(source),
            verify: Default::default(),
            reauth: Default::default(),
            enter_key: Default::default(),
            cloud,
            avatars: images::Images::new(64 << 20),
            wants: wants::Wants::default(),
            media: images::Images::new(512 << 20),
            tiles: images::Images::new(64 << 20),
            quotes: HashMap::new(),
            dismissed_errors: HashSet::new(),
            resolving_media: HashMap::new(),
            video: None,
            started: std::time::Instant::now(),
            last_click: None,
            chat_history: Vec::new(),
            cycling: false,
        };
        app.store.write().load_recent();
        if let Some(id) = first_chat {
            app.select_chat(&id);
        }
        app.keep_shown();
        app
    }

    /// The pane of the main window and those of the chat windows.
    fn panes(&self) -> impl Iterator<Item = &Pane> {
        std::iter::once(&self.pane).chain(self.chat_windows.iter().map(|w| &w.1))
    }

    /// The chats that the windows show stay loaded: the chats of the panes and their threads, of the poll vote
    /// windows, and of the voice note that plays. A chat that a sync removed leaves the windows at `StoreChanged`.
    fn keep_shown(&mut self) {
        let panes = self.panes().flat_map(|p| std::iter::once(p).chain(p.thread.as_deref()));
        let chats = panes.filter_map(|p| p.selected_chat_id.as_ref());
        let chats = chats.chain(self.vote_windows.iter().map(|w| &w.1)).chain(self.track_chat());
        let st = self.store.read();
        let shown: Vec<String> = chats.filter_map(|c| st.resolve_canonical_chat_id(c)).collect();
        if st.shown == shown {
            return;
        }
        drop(st);
        let mut st = self.store.write();
        st.shown = shown.clone();
        shown.iter().for_each(|c| st.load_chat(c));
    }

    pub fn boot(store: SharedStore) -> (Self, Task<UiMessage>) {
        let mut app = Self::new(store);
        let signed_in = app.pane.selected_chat_id.is_some();
        let mut settings = if signed_in { app.main_window_settings() } else { Self::login_window_settings() };
        let st = app.store.read();
        app.hidden = signed_in && st.settings.on("startMinimized");
        drop(st);
        settings.visible = !app.hidden;
        let (id, open) = window::open(settings);
        *(if signed_in { &mut app.main_window } else { &mut app.login_window }) = Some(id);
        let open = open.then(move |id| Task::done(id).chain(Self::keep_on_screen(id)));
        let cloud = (signed_in && !app.store.read().config.demo).then(|| app.refresh_cloud_accounts());
        let restore = (signed_in && !app.hidden).then(|| app.restore_subwindows());
        let link = app.store.write().config.link.take();
        let link = link.map(|l| app.open_link(&l));
        let onboard = (!signed_in && app.store.read().signed_in()).then(|| app.resume_onboarding());
        let tasks =
            [open.map(UiMessage::WindowOpened), iced::system::theme().map(UiMessage::SystemTheme), app.load_avatars()];
        (app, Task::batch(tasks.into_iter().chain(cloud).chain(restore).chain(link).chain(onboard)))
    }

    /// Starts the loads of the avatars that a view asked for, when a client can load them.
    fn load_avatars(&mut self) -> Task<UiMessage> {
        let Some(client) = self.store.read().matrix.clone() else { return Task::none() };
        let urls = std::mem::take(&mut *self.wants.avatars.borrow_mut());
        let urls: Vec<String> = urls.into_iter().filter(|u| !self.avatars.contains_key(u)).collect();
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

    fn face<'a>(&self, url: Option<&str>, name: &str, size: f32, ls: f32) -> El<'a> {
        let loaded = url.and_then(|u| match self.avatars.get(u) {
            Some(h) => h.clone(),
            None => {
                self.wants.ask(&self.wants.avatars, u);
                None
            }
        });
        match loaded {
            Some(h) => image(h).width(size).height(size).into(),
            None => avatar(name, size, ls, &self.pal),
        }
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
        let group = chat.chat_type == "group";
        self.people_avatar(chat.img_url.as_deref(), &chat.title, group, &chat.participants.items, size, ls)
    }

    /// The avatar of a chat with this image and title, or else of its people.
    fn people_avatar<'a>(
        &self,
        img: Option<&str>,
        title: &str,
        group: bool,
        users: &[User],
        size: f32,
        ls: f32,
    ) -> El<'a> {
        fn user(u: &User) -> (Option<&str>, &str) {
            (u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default())
        }
        let (shown, more) = faces_of(users, group);
        let face = if img.is_some() {
            self.face(img, title, size, ls)
        } else if !group || shown.is_empty() {
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
        // The main window, or the sign-in window of the onboarding.
        let (parent, around) = match self.main_window {
            Some(main) => (main, self.pane.size),
            None => (self.login_window.expect("the main or the sign-in window is open"), login::SIZE),
        };
        let offset = Vector::new((around.width - size.width) / 2.0, (around.height - size.height) / 2.0);
        // Desktop keeps the left and the top edge of a modal on the screen.
        let center = move |_| {
            window::position(parent).then(move |at| match at {
                Some(at) => window::move_to(id, Point::new((at.x + offset.x).max(0.0), (at.y + offset.y).max(0.0))),
                None => Task::none(),
            })
        };
        (id, open.then(center))
    }

    /// Desktop's `ga` (`mainWindowSetSize`): window `id` takes `size`, at most its screen, grows or shrinks around its
    /// center, and stays on the screen. Without a position (Wayland), it only takes the size.
    pub(crate) fn resize_around(id: window::Id, size: Size) -> Task<UiMessage> {
        let state = window::position(id).then(move |at| window::size(id).map(move |old| (at, old)));
        let state = state.then(move |(at, old)| window::monitor_size(id).map(move |screen| (at, old, screen)));
        state.then(move |(at, old, screen)| {
            let screen = screen.unwrap_or(Size::new(f32::INFINITY, f32::INFINITY));
            let (to, fit) = grown(at.unwrap_or(Point::ORIGIN), old, size, screen);
            let resize = window::resize(id, fit);
            match at {
                Some(_) => Task::batch([resize, window::move_to(id, to)]),
                None => resize,
            }
        })
    }

    /// Desktop's default place of a window that it opens (`openInNewWindow`, `ao.windowFeatures`): 20 px right of and
    /// below the main window.
    pub(crate) fn cascade<T: Send + 'static>(&self, id: window::Id) -> Task<T> {
        let Some(main) = self.main_window else { return Task::none() };
        window::position(main).then(move |at| match at {
            Some(at) => window::move_to(id, at + Vector::new(20.0, 20.0)),
            None => Task::none(),
        })
    }

    /// Desktop's `windowStateKeeper` check of saved bounds (`Yy`, `Ma`): a window more than 50 px off its screen, like
    /// one saved on a screen that is gone, moves onto the screen and shrinks to fit it.
    fn keep_on_screen<T: Send + 'static>(id: window::Id) -> Task<T> {
        const SLACK: f32 = 50.0;
        let state = window::position(id).then(move |at| window::size(id).map(move |size| (at, size)));
        let state = state.then(move |(at, size)| window::monitor_size(id).map(move |screen| (at, size, screen)));
        state.then(move |(at, size, screen)| {
            let (Some(at), Some(screen)) = (at, screen) else { return Task::none() };
            let off = at.x < -SLACK
                || at.y < -SLACK
                || at.x + size.width > screen.width + SLACK
                || at.y + size.height > screen.height + SLACK;
            if !off {
                return Task::none();
            }
            let fit = Size::new(size.width.min(screen.width), size.height.min(screen.height));
            let to = Point::new(at.x.clamp(0.0, screen.width - fit.width), at.y.clamp(0.0, screen.height - fit.height));
            Task::batch([window::resize(id, fit), window::move_to(id, to)])
        })
    }

    /// Desktop's `windowStateKeeper("main")`: the main window opens with the bounds and the maximized state of
    /// `window.json`, else centered with the default size.
    fn main_window_settings(&self) -> window::Settings {
        self.saved_window_settings("main", MAIN_SIZE)
    }

    /// Desktop's `windowStateKeeper(key)`: a window opens with its bounds and its maximized state in `window.json`,
    /// else centered with the `size` by default.
    fn saved_window_settings(&self, key: &str, size: Size) -> window::Settings {
        let state = read_window_state(&self.store.read().config.data_dir.join("window.json"));
        let saved = &state["windowState"][key];
        let num = |key: &str| saved[key].as_f64().map(|v| v as f32);
        let size = num("width").zip(num("height")).map_or(size, |(w, h)| Size::new(w, h));
        let at = num("x").zip(num("y")).map(|(x, y)| window::Position::Specific(Point::new(x, y)));
        let maximized = saved["isMaximized"] == true;
        window::Settings {
            position: at.unwrap_or(window::Position::Centered),
            maximized,
            ..Self::window_settings(size)
        }
    }

    /// Saves the state of the main window to `window.json`.
    fn save_main_window(&self) -> Task<UiMessage> {
        let Some(id) = self.main_window else { return Task::none() };
        self.save_window(id, "main")
    }

    /// Saves the state of window `id` under `key` in `window.json`. A maximized window keeps the bounds from before.
    fn save_window(&self, id: window::Id, key: &'static str) -> Task<UiMessage> {
        let path = self.store.read().config.data_dir.join("window.json");
        let state = window::is_maximized(id).then(move |max| window::size(id).map(move |size| (max, size)));
        let state = state.then(move |(max, size)| window::position(id).map(move |at| (max, size, at)));
        state.map(move |(max, size, at)| write_window_state(&path, key, max, size, at)).discard()
    }

    fn window_settings(size: Size) -> window::Settings {
        window::Settings {
            size,
            min_size: Some(Size::new(330.0, 400.0)),
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
                application_id: beeper_core::config::INSTALL_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Desktop's `subwindow-state`: the chat and settings windows to open again at the next launch.
    fn save_subwindows(&self) {
        let chats = self.chat_windows.iter().filter(|w| w.1.thread_root.is_none());
        let threads = chats.filter_map(|w| w.1.selected_chat_id.as_deref());
        let threads =
            threads.map(|t| serde_json::json!({"windowId": format!("thread-{t}"), "type": "thread", "threadId": t}));
        let prefs = self.settings_window.map(|_| serde_json::json!({"windowId": "prefs", "type": "prefs"}));
        let state = serde_json::json!({"openSubwindows": threads.chain(prefs).collect::<Vec<_>>()});
        let path = self.store.read().config.data_dir.join("subwindow-state.json");
        std::fs::write(path, state.to_string()).expect("write the subwindow state");
    }

    /// Opens the windows of the last session, like Desktop's `setupSubwindowRestoreHandler`.
    fn restore_subwindows(&mut self) -> Task<UiMessage> {
        let path = self.store.read().config.data_dir.join("subwindow-state.json");
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

    pub fn title(&self, id: window::Id) -> String {
        match () {
            _ if Some(id) == self.settings_window => tr("Settings"),
            _ if self.theme_windows.iter().any(|w| w.0 == id) => tr("Chat Theme"),
            _ if self.labels_windows.iter().any(|w| w.0 == id) => tr("Edit Labels"),
            _ if self.label_settings.iter().any(|w| w.id == id) => tr("Label Settings"),
            _ if self.poll_windows.iter().any(|w| w.id == id) => tr("New Poll"),
            _ if let Some(d) = self.label_window.as_ref().filter(|d| d.id == id) => match (d.label.is_some(), d.step) {
                (false, _) => tr("New Label"),
                (true, LabelStep::Details) => tr("Edit Label"),
                (true, _) => tr("Chats"),
            },
            _ if self.spaces_window.as_ref().is_some_and(|d| d.id == id) => tr("Edit Spaces"),
            _ if Some(id) == self.blocked_window => tr("Blocked users"),
            _ if let Some(d) = self.group_window.as_ref().filter(|d| d.id == id) => {
                return tr(if d.personal { "Edit Chat for Me" } else { "Edit Group" }).into();
            }
            _ if Some(id) == self.verify.window => "Beeper",
            _ if Some(id) == self.reauth.window => tr("Authentication Required"),
            _ if Some(id) == self.enter_key.window => self.enter_key_title(),
            _ if let Some(title) = self.add_account_title(id) => return title,
            _ if let Some((_, _, mid)) = self.vote_windows.iter().find(|w| w.0 == id) => {
                let st = self.store.read();
                let total = st.loaded(mid).and_then(|m| m.poll.as_ref()).map_or(0, PollData::voters);
                return trn("%d vote", "%d votes", total, &[&total]);
            }
            _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => tr("Media"),
            _ => match self.chat_windows.iter().find(|w| w.0 == id) {
                Some((_, pane)) => {
                    let st = self.store.read();
                    let chat = pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c));
                    let title = chat.map_or("Beeper".into(), |i| st.chats[i].title.clone());
                    return match pane.thread_root {
                        Some(_) => trf("Thread in %s", &[&title]),
                        None => title,
                    };
                }
                None => {
                    let st = self.store.read();
                    let chat = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c));
                    let title = chat.map(|i| st.chats[i].title.trim()).filter(|t| !t.is_empty());
                    return title.map_or("Beeper".into(), |t| format!("Beeper | {t}"));
                }
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
        // Desktop's `--custom-font-size` is the font size plus 3 px, and 16 px is the default.
        let font: f32 = st.settings.choice("fontSize").parse().expect("a number option");
        self.zoom = f32::from(st.settings.zoom_percent) / 100.0 * (font + 3.0) / 16.0;
        self.tray_sent.1 = true;
    }

    /// Splits the visible chats into the pinned tiles and the list rows, in display order.
    fn pins_and_rows<'a>(&self, st: &'a BeeperStore) -> (Vec<&'a Chat>, Vec<&'a Chat>) {
        let (mut pinned, rest): (Vec<&Chat>, Vec<&Chat>) = self
            .visible_chats(st)
            .into_iter()
            .partition(|c| self.active_tab.filterable() && c.flags & CHAT_PINNED != 0);
        pinned.sort_by_key(|c| (st.pin_rank.get(&c.id).copied().unwrap_or(usize::MAX), c.last_activity.clone()));
        match st.settings.on("regularPinned") {
            true => (vec![], pinned.into_iter().chain(rest).collect()),
            false => (pinned, rest),
        }
    }

    pub fn visible_chats<'a>(&self, st: &'a BeeperStore) -> Vec<&'a Chat> {
        self.chats_with(st, self.filters)
    }

    /// The chats of the list with `filters`, computed once for each store generation.
    fn chats_with<'a>(&self, st: &'a BeeperStore, filters: u8) -> Vec<&'a Chat> {
        let key = (self.active_tab, filters, self.selected_account_filter.clone());
        let memo = self.memo();
        let known = memo.and_then(|m| m.lists.borrow().get(&key).cloned());
        let order = known.unwrap_or_else(|| {
            let last = self.memo.last.borrow().get(&key).cloned().unwrap_or_else(|| Rc::new([]));
            let order: Rc<[usize]> = self.sorted_chats(st, filters, &last).into();
            if let Some(m) = memo {
                m.lists.borrow_mut().insert(key.clone(), order.clone());
                m.last.borrow_mut().insert(key, order.clone());
            }
            order
        });
        order.iter().map(|&i| &st.chats[i]).collect()
    }

    /// The virtual list keys of the chat rows of the list, made once for each store generation.
    fn row_keys(&self, rest: &[&Chat]) -> Rc<[u64]> {
        let make = || rest.iter().map(|c| vlist::key(&c.id)).collect();
        let Some(memo) = self.memo() else { return make() };
        let key = (self.active_tab, self.filters, self.selected_account_filter.clone());
        let keys = memo.keys.borrow_mut().entry(key).or_insert_with(make).clone();
        assert_eq!(keys.len(), rest.len(), "the memo has the keys of the same rows");
        keys
    }

    /// A count of chats, computed once for each store generation.
    fn count(&self, key: &str, count: impl FnOnce() -> usize) -> usize {
        let Some(memo) = self.memo() else { return count() };
        let known = memo.counts.borrow().get(key).copied();
        known.unwrap_or_else(|| {
            let n = count();
            memo.counts.borrow_mut().insert(key.to_string(), n);
            n
        })
    }

    /// The memo, emptied when the store changed after it, or `None` while a write lock can change the store.
    fn memo(&self) -> Option<&Memo> {
        let generation = self.store.generation()?;
        if self.memo.generation.replace(generation) != generation {
            self.memo.lists.borrow_mut().clear();
            self.memo.counts.borrow_mut().clear();
            self.memo.keys.borrow_mut().clear();
            self.memo.words.take();
            self.memo.hits.take();
            self.memo.media.take();
            self.memo.threads.take();
        }
        Some(&self.memo)
    }

    /// The chat indices of the list with `filters` in list order. The sort starts from `last`, the order of an
    /// older store generation, which it changes little, and its keys end with the chat index, so the order does not
    /// depend on `last`.
    fn sorted_chats(&self, st: &BeeperStore, filters: u8, last: &[usize]) -> Vec<usize> {
        let keep: Vec<bool> = st.chats.iter().map(|c| self.listed(st, filters, c)).collect();
        let mut seen = vec![false; keep.len()];
        let mut list: Vec<usize> = (last.iter().copied())
            .filter(|&i| keep.get(i) == Some(&true) && !std::mem::replace(&mut seen[i], true))
            .collect();
        list.extend((0..keep.len()).filter(|&i| keep[i] && !seen[i]));
        let remind_at = |i: usize| st.chats[i].reminder.as_ref().map_or(0, |r| r.remind_at_ms);
        // Desktop's `activityTimestamp`: the newer of the activity and the draft.
        let activity = |i: usize| beeper_core::store::activity_key(&st.chats[i]).0;
        match (self.active_tab, st.settings.reminders_order) {
            (InboxTab::Reminders, 0) => list.sort_by_key(|&i| (remind_at(i), i)),
            (InboxTab::Reminders, 1) => list.sort_by_key(|&i| (std::cmp::Reverse(remind_at(i)), i)),
            (InboxTab::Reminders, 2) => list.sort_by_key(|&i| (activity(i), i)),
            (InboxTab::SendLater, _) => list.sort_by_key(|&i| (next_send(st, &st.chats[i].id), i)),
            _ => list.sort_by_key(|&i| (std::cmp::Reverse(activity(i)), i)),
        }
        list
    }

    /// Whether the chat list with `filters` shows `c`.
    fn listed(&self, st: &BeeperStore, filters: u8, c: &Chat) -> bool {
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
                InboxTab::Archived => c.archived_in_list() && shown(F_REQUESTS),
                InboxTab::LowPriority => c.flags & CHAT_LOW_PRIORITY != 0 && shown(F_ARCHIVED) && shown(F_REQUESTS),
                InboxTab::Reminders => c.reminder.is_some(),
                InboxTab::SendLater => st.scheduled.iter().any(|s| s.chat_id == c.id),
            }
            && (!self.active_tab.filterable()
                || filters == 0
                || (0..FILTERS.len()).any(|i| on(i) && filter_hit(i, c, st)))
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        self.creating_chat = None;
        self.join_matrix_mode = false;
        if !self.cycling {
            self.push_history(chat_id);
        }
        let mut st = self.store.write();
        self.leave_chat(&mut st, Some(chat_id));
        if let Some(old) = self.pane.selected_chat_id.clone() {
            match self.pane.thread.as_ref().and_then(|t| t.thread_root.clone()) {
                Some(root) => drop(self.open_threads.insert(old, root)),
                None => drop(self.open_threads.remove(&old)),
            }
        }
        let idx = st.resolve_chat_idx(chat_id).expect("selected chat exists");
        self.pane.selected_chat_id = Some(chat_id.to_string());
        if !st.incognito() {
            st.mark_read(idx, false);
        }
        let chat = st.chats[idx].clone();
        let draft = chat.draft.clone().unwrap_or_default();
        self.pane.composer = composer_with(draft.text.as_deref().unwrap_or_default());
        self.pane.attachments = load_draft(&st, &draft).unwrap_or_else(|e| {
            self.status_banner = e;
            Vec::new()
        });
        self.pane.composer_links = draft.discarded_links.into_iter().map(|u| (u, LinkState::Discarded)).collect();
        (self.pane.history, self.pane.draft_due) = (History::default(), false);
        self.pane.timeline.forget_reveal();
        self.pane.scrolled_up = None;
        self.pane.reply_to_message_id = draft.reply_to_message_id;
        (self.pane.editing_message_id, self.pane.stash) = (None, None);
        self.pane.mentions = draft.mentions;
        self.pane.selected_msgs.clear();
        self.pane.msg_anchor = None;
        (self.info_desc, self.info_more, self.info_all, self.info_bookmarks) = (None, false, false, false);
        self.info_media = None;
        self.info_query.clear();
        self.info_note = text_editor::Content::with_text(chat.note.as_deref().unwrap_or_default());
        let pending = st.scheduled.iter().any(|s| s.chat_id == chat.id && !s.canceled);
        let later = draft.send_later.filter(|&(at, _)| at > now_unix_millis());
        (self.pane.send_later, self.pane.sched_fold, self.pane.sched_menu) = (later, (pending, pending), None);
        (self.pane.reply_thread, self.pane.thread, self.pane.thread_menu) = (None, None, None);
        self.unread_view = false;
        // A running load or retry of the last chat goes on to this chat when it ends.
        (self.pane.top.shown, self.pane.top.tries) = (false, 0);
        self.video = None;
        drop(st);
        if let Some(root) = self.open_threads.get(chat_id).cloned() {
            self.open_thread(chat_id.into(), root);
        }
        self.refresh_theme_and_tray();
    }

    /// Desktop's `createThread`: selects the most recent direct chat with the user on the account, or asks the
    /// bridge to make one.
    fn open_dm(&mut self, account: String, user: User, draft: Option<u32>, quote: Option<String>) -> Task<UiMessage> {
        if self.sticky_banner.is_some() {
            return Task::none();
        }
        // Desktop's `identifierForUser` tries the Matrix ID before the phone number.
        let phone = user.phone_number.as_deref().map(phone_digits).filter(|_| !user.id.starts_with('@'));
        let found = {
            let st = self.store.read();
            let same = |u: &User| match &phone {
                Some(p) => u.phone_number.as_deref().map(phone_digits).as_ref() == Some(p),
                None => u.id == user.id,
            };
            let theirs = |c: &&Chat| c.participants.items.iter().any(|u| u.is_self != Some(true) && same(u));
            let dms = st.chats.iter().filter(|c| c.account_id == account && c.chat_type == "single");
            dms.filter(theirs).max_by(|a, b| a.last_activity.cmp(&b.last_activity)).map(|c| c.id.clone())
        };
        if let Some(id) = found {
            self.new_chats.retain(|d| Some(d.id) != draft);
            self.select_dm(&id, quote);
            return Task::none();
        }
        self.status_banner = tr("Composing chat...").into();
        self.sticky_banner = Some(self.status_banner.clone());
        let create = beeper_core::matrix::create_dm(self.provision(&account), phone.unwrap_or(user.id));
        Task::perform(create, move |r| UiMessage::DmCreated(draft, account.clone(), quote.clone(), r))
    }

    /// Selects a direct chat and puts a quote before its draft, like Desktop's `replyPrivately`.
    fn select_dm(&mut self, chat_id: &str, quote: Option<String>) {
        self.select_chat(chat_id);
        if let Some(q) = quote {
            self.pane.composer = composer_with(&(q + self.pane.composer.text().as_str()));
        }
    }

    /// Desktop's `setPicker` checks: shows a toast when the network of the open chat does not accept the picker. The
    /// sticker picker opens only with "Use sticker packs".
    fn supports(&mut self, kind: Picker) -> bool {
        let st = self.store.read();
        if kind == Picker::Sticker && !st.settings.on("stickerPacks") {
            return false;
        }
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane().selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let (ok, toast) = match kind {
            Picker::Gif => (chat.takes(FILE_GIF), "GIFs aren't supported on %s"),
            Picker::Audio => (chat.takes(FILE_VOICE), "Recording audio isn't supported on %s"),
            Picker::Sticker => (chat.takes(FILE_STICKER), "Stickers aren't supported on %s"),
        };
        let net = network_name(&chat.account_id);
        drop(st);
        if !ok {
            self.status_banner = trf(toast, &[&net]);
        }
        ok
    }

    /// Desktop's `GifPicker` keys: Down selects the first GIF, Left and Right move the selection, Up goes back to the
    /// search, Enter sends the selected GIF, and Escape closes the picker and focuses the composer.
    fn gif_key(&mut self, key: &Key) -> Option<Task<UiMessage>> {
        let n = self.gifs.len();
        let select = |app: &mut Self, i: usize| {
            app.gif_sel = Some(i);
            // Desktop's `scrollIntoView` of the selected GIF, to the nearest edge.
            let x = 14.0 + app.gifs[..i].iter().map(|g| gif_width(g) + 4.0).sum::<f32>();
            let (at, width) = app.gif_view;
            let to = match () {
                _ if x < at => x,
                _ if x + gif_width(&app.gifs[i]) > at + width => x + gif_width(&app.gifs[i]) - width,
                _ => return Task::none(),
            };
            operation::scroll_to("gif-carousel", operation::AbsoluteOffset { x: to, y: 0.0 })
        };
        Some(match (key.as_ref(), self.gif_sel) {
            (Key::Named(Named::ArrowUp), _) => {
                self.gif_sel = None;
                Task::batch([operation::focus("gif-search-input"), operation::move_cursor_to_end("gif-search-input")])
            }
            (Key::Named(Named::ArrowDown), None) if n > 0 => select(self, 0),
            (Key::Named(Named::ArrowLeft), Some(i)) => select(self, i.saturating_sub(1).min(n - 1)),
            (Key::Named(Named::ArrowRight), Some(i)) => select(self, (i + 1).min(n - 1)),
            (Key::Named(Named::Enter), Some(i)) if i < n => self.update(UiMessage::PickGif(self.gifs[i].clone())),
            (Key::Named(Named::Escape), _) => {
                self.pane.picker = None;
                operation::focus("composer-input")
            }
            _ => return None,
        })
    }

    /// Desktop's `downloadGif`: downloads the format of the GIF that the open chat takes. An MP4 is measured like a
    /// picked video.
    fn download_gif(&self, g: KlipyGif) -> Task<UiMessage> {
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let Some((url, size, mp4)) = portal::gif_format(&g, &chat.file) else { return Task::none() };
        let (url, name) = (url.to_string(), format!("{}.{}", g.id, if mp4 { "mp4" } else { "gif" }));
        let load = async move {
            let bytes = beeper_core::cloud::http_bytes(&url).await?;
            let load = move || match mp4 {
                true => {
                    let path = std::env::temp_dir().join(format!("beeper-rs-{}-{name}", std::process::id()));
                    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                    let (f, frames) = (portal::load_file(&path), portal::video_frames(&path, true));
                    std::fs::remove_file(&path).expect("remove the downloaded GIF");
                    Ok(PendingMedia { is_gif: true, size, frames: frames?, ..f? })
                }
                false => {
                    let frames = decode_media(bytes.clone())?;
                    let mime = "image/gif".into();
                    Ok(PendingMedia { bytes, name, mime, is_gif: true, size, frames, duration: None, sticker: None })
                }
            };
            tokio::task::spawn_blocking(load).await.expect("GIF load thread")
        };
        Task::perform(load, UiMessage::GifDownloaded)
    }

    /// Desktop's `loadStickerPacks` when the sticker picker opens: the packs of the open chat.
    fn load_sticker_packs(&mut self) -> Task<UiMessage> {
        (self.pane.stickers, self.pane.sticker_tab) = (None, None);
        let room = self.pane.selected_chat_id.clone().expect("an open chat");
        Task::perform(beeper_core::matrix::sticker_packs(self.store.clone(), room), UiMessage::StickerPacks)
    }

    /// Desktop's `onSelect` of the sticker picker: an animated sticker needs a chat that takes it, and the image
    /// comes as an attachment named by its first shortcode.
    fn pick_sticker(&mut self, s: Sticker) -> Task<UiMessage> {
        use beeper_core::media::{animated, takes_sticker};
        self.pane.picker = None;
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let mime = s.info["mimetype"].as_str().map(str::to_string);
        if mime.as_deref().is_some_and(|m| animated(m) && !takes_sticker(&chat.file, m)) {
            drop(st);
            self.status_banner = tr("This animated sticker can't be sent in this chat.").into();
            return Task::none();
        }
        let fetch = beeper_core::matrix::media(st.matrix.clone(), s.url.clone());
        drop(st);
        let load = async move {
            let bytes = fetch.await?;
            let load = move || {
                let reader = ::image::ImageReader::new(std::io::Cursor::new(&bytes));
                let reader = reader.with_guessed_format().expect("in memory");
                let sniffed = reader.format().map(|f| f.to_mime_type().to_string());
                let dims = reader.into_dimensions().map_err(|e| e.to_string())?;
                let mime = mime.or(sniffed).ok_or("The sticker has no image type")?;
                let ext =
                    mime.split_once('/').map(|(_, t)| t.replace("x-tgsticker", "tgs").replace("lottie+json", "json"));
                let code = Some(s.shortcodes[0].as_str()).filter(|c| !c.is_empty()).unwrap_or("sticker");
                let name = format!("{code}.{}", ext.unwrap_or_else(|| "webp".into()));
                let size = match (s.info["w"].as_u64(), s.info["h"].as_u64()) {
                    (Some(w), Some(h)) => (w as u32, h as u32),
                    _ => dims,
                };
                let frames = decode_media(bytes.clone())?;
                let is_gif = mime == "image/gif";
                Ok(PendingMedia { bytes, name, mime, is_gif, size, frames, duration: None, sticker: Some(Box::new(s)) })
            };
            tokio::task::spawn_blocking(load).await.expect("sticker load thread")
        };
        Task::perform(load, UiMessage::StickerLoaded)
    }

    /// The selected message when only one message is selected.
    fn single_msg(&self) -> Option<&String> {
        Some(&self.pane().selected_msgs).filter(|s| s.len() == 1).map(|s| &s[0])
    }

    fn selected_message(&self) -> Option<Message> {
        let st = self.store.read();
        self.single_msg().map(|id| st.message(id).clone())
    }

    /// The messages of the open chat in timeline order, and which of them Desktop lets you select.
    fn timeline<'a>(&self, st: &'a BeeperStore) -> (Vec<&'a Message>, Vec<bool>) {
        let msgs = self.pane_msgs(st);
        let members = st.settings.on("membershipEvents");
        let live = msgs.iter().map(|m| m.is_deleted != Some(true) && (members || !membership(m))).collect();
        (msgs, live)
    }

    /// The messages of the pane: Desktop's `messageListData` of the chat without the replies only in threads, or
    /// Desktop's `getReplyThreadListData` of the thread of the pane: its root and its replies.
    fn pane_msgs<'a>(&self, st: &'a BeeperStore) -> Vec<&'a Message> {
        let cid = self.pane().selected_chat_id.as_deref().expect("an open chat");
        if let Some(root) = &self.pane().thread_root {
            let root_msg = st.loaded(root);
            return root_msg.into_iter().chain(st.thread_replies(root)).collect();
        }
        let threaded = st.threaded(&st.chats[st.resolve_chat_idx(cid).expect("an open chat")]);
        let all = st.timeline(cid);
        all.filter(|m| !m.thread_only(threaded)).collect()
    }

    /// The arrow keys select messages when no panel is open and the composer is empty.
    fn quick_react(&self, mid: String) -> UiMessage {
        let chat = self.pane().selected_chat_id.clone().expect("an open chat");
        let st = self.store.read();
        UiMessage::ToggleReaction(chat, mid, st.settings.quick_reaction.clone())
    }

    /// The width of the rail and the chat list. Desktop keeps it when the space bar hides.
    fn left_width(&self) -> f32 {
        if self.sidebar_hidden {
            0.0
        } else {
            68.0 + self.sidebar_width
        }
    }

    /// The width of the chat list: Desktop's `threads` width, which takes the space of a hidden space bar.
    fn list_width(&self, st: &BeeperStore) -> f32 {
        self.sidebar_width + if st.settings.on("showSidebar") { 0.0 } else { 68.0 }
    }

    /// The message features of the chat of `m`.
    fn rules(&self, m: &Message) -> MessageRules {
        let st = self.store.read();
        st.chat(&m.chat_id).messages
    }

    /// The newest message you can edit in the open chat.
    fn last_own_message(&self) -> Option<Message> {
        let st = self.store.read();
        let id = self.pane().selected_chat_id.as_deref().expect("an open chat");
        let rules = st.chat(id).messages;
        let own = st
            .timeline_indices(id)
            .iter()
            .map(|&i| &st.messages[i])
            .rfind(|m| editable(m, rules) && edit_open(m, rules) && m.attachments.is_empty());
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
        if !st.incognito() && !st.chats[idx].flag(ChatFlag::MarkedUnread) {
            st.mark_chat_read(prev, true);
        }
        if self.expired(&st.chats[idx]) {
            st.set_reminder(idx, None);
        }
    }

    /// Quits the app; the open chats keep their drafts.
    fn quit(&mut self) -> Task<UiMessage> {
        self.save_drafts();
        self.save_subwindows();
        self.save_main_window().chain(iced::exit())
    }

    /// Desktop's `TopOfThread` effect: loads older messages of the open chat while the top of its timeline is on
    /// screen, up to 5 loads that bring no message.
    fn load_older(&mut self) -> Task<UiMessage> {
        let top = &self.pane.top;
        if !top.shown || top.loading.is_some() || top.waiting || top.tries >= 5 {
            return Task::none();
        }
        let st = self.store.read();
        let Some(chat) = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c)) else {
            return Task::none();
        };
        let chat = &st.chats[chat];
        if !has_older(&st, chat) {
            return Task::none();
        }
        let (id, before) = (chat.id.clone(), st.timeline_indices(&chat.id).len());
        drop(st);
        self.pane.top.loading = Some((std::time::Instant::now(), id.clone(), before));
        let load = beeper_core::matrix::load_older(self.store.clone(), id);
        Task::perform(load, UiMessage::LoadedOlder)
    }

    /// Desktop's `TopOfThread` loads again when the chat gets older messages while its top is on screen, as when the
    /// first sync of a joined room brings a page: in the main pane and in each chat window.
    fn load_older_everywhere(&mut self) -> Task<UiMessage> {
        let mut tasks = vec![self.load_older()];
        for i in 0..self.chat_windows.len() {
            let id = self.chat_windows[i].0;
            std::mem::swap(&mut self.pane, &mut self.chat_windows[i].1);
            let task = self.load_older();
            std::mem::swap(&mut self.pane, &mut self.chat_windows[i].1);
            tasks.push(task.map(move |m| UiMessage::InWindow(id, Box::new(m))));
        }
        Task::batch(tasks)
    }

    /// The end of a load of older messages. A retry follows with Desktop's backoff: 100 ms * 2^tries up to 3 s, with
    /// the tries reset when messages came.
    fn loaded_older(&mut self, r: Result<(), String>) -> Task<UiMessage> {
        let (_, chat, before) = self.pane.top.loading.take().expect("a load ran");
        let st = self.store.read();
        let open = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_canonical_chat_id(c));
        let count = open.as_ref().map(|c| st.timeline_indices(c).len());
        drop(st);
        if open != Some(chat) {
            return self.load_older();
        }
        if let Err(e) = r {
            // The next time the top comes on screen, it tries again.
            self.fail(Toast::new(format!("Could not load older messages: {e}")));
            return Task::none();
        }
        let top = &mut self.pane.top;
        (top.tries, top.waiting) = (if count == Some(before) { top.tries + 1 } else { 0 }, true);
        let wait = Duration::from_millis((100 * 2u64.pow(top.tries)).min(3000));
        Task::perform(async move { tokio::time::sleep(wait).await }, |()| UiMessage::LoadOlder)
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
            let path = self.store.read().config.data_dir.join("sidebar-hidden");
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
            "Switch First 9 Spaces Modifier" => {
                let space = rail_spaces(&self.store.read()).into_iter().nth(digit);
                space.and_then(|id| match RAIL_VIEWS.iter().find(|v| v.0 == id) {
                    Some(view) => view.1.map(UiMessage::SelectTab),
                    None => Some(UiMessage::SelectAccount(Some(id))),
                })
            }
            "Toggle Incognito Mode" => {
                let st = self.store.read();
                st.feature("stealth-mode").on.then_some(UiMessage::ToggleIncognito)
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
                let edit =
                    self.selected_message().filter(|m| editable(m, self.rules(m)) && edit_open(m, self.rules(m)));
                let m = chat.and_then(|_| edit.or_else(|| self.last_own_message()));
                m.map(|m| {
                    self.pane.selected_msgs = vec![m.id.clone()];
                    UiMessage::StartEdit(m.id, m.text.unwrap_or_default())
                })
            }
            "Message: Forward" => {
                let st = self.store.read();
                let msgs = chat.iter().flat_map(|_| self.timeline(&st).0);
                let msgs: Vec<&Message> = msgs.filter(|m| self.pane.selected_msgs.contains(&m.id)).collect();
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
                    true => (empty && editable(&m, self.rules(&m)) && edit_open(&m, self.rules(&m)))
                        .then(|| UiMessage::StartEdit(m.id, m.text.unwrap_or_default())),
                    false => Some(UiMessage::StartReply(m.id)),
                })
            }
            "Message: Record Audio" => chat.map(|_| UiMessage::Record(record::Msg::Start)),
            "Message: Talk to Type" => chat.map(|_| UiMessage::Record(record::Msg::Talk)),
            "Message: Send GIF" => chat.map(|_| UiMessage::TogglePicker(Picker::Gif)),
            "Message: Send Sticker" => chat.map(|_| UiMessage::TogglePicker(Picker::Sticker)),
            "Show Replies" => chat.and_then(|_| {
                let st = self.store.read();
                let msgs = self.timeline(&st).0;
                let chat = &st.chats[st
                    .resolve_chat_idx(self.pane.selected_chat_id.as_ref().expect("an open chat"))
                    .expect("a known chat")];
                self.pane.selected_msgs.iter().find_map(|id| {
                    let m = st.message(id);
                    let replied = || msgs.iter().any(|r| r.id != m.id && reply_root(&st, r) == m.id);
                    let thread = || can_thread(&st, chat, m).then(|| UiMessage::ShowThread(m.id.clone()));
                    let chain = || replied().then(|| UiMessage::ShowReplies(m.id.clone()));
                    m.linked_message_id.clone().map(UiMessage::ShowReplies).or_else(thread).or_else(chain)
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
        let pane = match self.drawing.get() {
            Some(id) => &self.chat_windows.iter().find(|w| w.0 == id).expect("a drawn chat window").1,
            None => &self.pane,
        };
        match self.drawing_thread.get() {
            true => pane.thread.as_deref().expect("a drawn thread pane"),
            false => pane,
        }
    }

    pub fn update(&mut self, msg: UiMessage) -> Task<UiMessage> {
        let (shown, dismissals) = (self.stackable_toast(), self.toast_dismissals);
        let task = self.handle(msg);
        self.keep_shown();
        let replaced = |t: &(Toast, Duration)| !self.status_banner.is_empty() && t.0.text != self.status_banner;
        if let Some(t) = shown.filter(|t| dismissals == self.toast_dismissals && replaced(t)) {
            self.older_toasts.push(t);
            let extra = self.older_toasts.len().saturating_sub(TOASTS_SHOWN - 1);
            self.older_toasts.drain(..extra);
        }
        if self.status_banner.is_empty() && self.older_toasts.is_empty() {
            self.toast_clock.1 = false;
        }
        if !std::mem::take(&mut self.tray_sent.1) {
            return task;
        }
        let now = (self.badge_count(), !self.hidden);
        if self.tray_sent.0.replace(now) == Some(now) {
            return task;
        }
        Task::batch([task, badge_sync(now.0, now.1)])
    }

    /// Desktop's `updateAppBadgeCount`: the unread chats in all chats or in the chat list, by "Badge count".
    /// The open chat shows a composer: it is available, not upgraded, not blocked, and the user did not leave it.
    fn can_compose(&self) -> bool {
        let st = self.store.read();
        let chat = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c)).map(|i| &st.chats[i]);
        chat.is_some_and(|c| {
            unavailable(&st, c).is_none() && c.replacement_room.is_none() && !st.user_blocked(c) && !c.exited()
        })
    }

    fn badge_count(&self) -> u32 {
        let st = self.store.read();
        // Desktop's `updateAppBadgeCount` shows nothing while signed out.
        if !st.signed_in() {
            return 0;
        }
        let counts = |c: &Chat| {
            c.flags & (CHAT_HIDDEN | CHAT_ARCHIVED | CHAT_REQUEST) == 0
                && c.flags & CHAT_UNREAD != 0
                && (c.unread_mentions > 0 || !st.notifications_muted(c))
        };
        let n = match st.settings.pref("badgeCount") {
            0 => return 0,
            1 => st.chats.iter().filter(|c| counts(c)).count(),
            _ => self.visible_chats(&st).into_iter().filter(|c| counts(c)).count(),
        };
        // Desktop counts one more with unread threads.
        n as u32 + u32::from(!self.unread_threads(&st, false).is_empty())
    }

    /// The windows besides the main window, Desktop's `subWindows`.
    fn sub_windows(&self) -> Vec<window::Id> {
        let drafts = [self.label_window.as_ref().map(|d| d.id), self.spaces_window.as_ref().map(|d| d.id)];
        let singles = [self.settings_window, self.media_window.as_ref().map(|w| w.id)].into_iter().chain(drafts);
        let lists = self.chat_windows.iter().map(|w| w.0).chain(self.theme_windows.iter().map(|w| w.0));
        let lists = lists.chain(self.labels_windows.iter().map(|w| w.0)).chain(self.vote_windows.iter().map(|w| w.0));
        let group = self.group_window.as_ref().map(|d| d.id);
        let lists = lists.chain(self.poll_windows.iter().map(|w| w.id)).chain(self.add_accounts.iter().map(|a| a.id));
        singles.chain([group, self.blocked_window]).flatten().chain(lists).collect()
    }

    /// Desktop's `hideToTray` (`Du`), which hides the sub-windows and then the main window, and `showWindow`, which
    /// shows the main window.
    /// Hides or shows the app: the main window, or the sign-in window while signed out (Desktop's single window).
    fn set_hidden(&mut self, hidden: bool) -> Task<UiMessage> {
        let Some(id) = self.main_window.or(self.login_window) else { return Task::none() };
        self.hidden = hidden;
        self.tray_sent.1 = true;
        match hidden {
            true => {
                let subs = self.sub_windows().into_iter().map(|w| window::set_mode(w, window::Mode::Hidden));
                let hide = Task::batch(subs).chain(window::set_mode(id, window::Mode::Hidden));
                self.save_main_window().chain(hide)
            }
            false => window::set_mode(id, window::Mode::Windowed).chain(window::gain_focus(id)),
        }
    }

    /// Desktop's `showAllWindows` (`hm`): the sub-windows, then the main window.
    fn show_all(&mut self) -> Task<UiMessage> {
        let subs = self.sub_windows().into_iter().map(|w| window::set_mode(w, window::Mode::Windowed));
        Task::batch(subs).chain(self.set_hidden(false))
    }

    fn handle(&mut self, msg: UiMessage) -> Task<UiMessage> {
        // Another key while Alt is down keeps the release of Alt from toggling the menu bar.
        if let UiMessage::WindowKey(_, key, _) = &msg {
            self.alt_alone &= *key == Key::Named(Named::Alt);
        }
        // Desktop's `persistDraftRecord` after a change of the attachments, the quote, Send Later or the link previews.
        self.pane.draft_due |= matches!(
            msg,
            UiMessage::FilesLoaded(_)
                | UiMessage::Pasted(_)
                | UiMessage::DiscardAttachment(_)
                | UiMessage::GifDownloaded(..)
                | UiMessage::StickerLoaded(..)
                | UiMessage::StartReply(_)
                | UiMessage::CancelComposeMode
                | UiMessage::SendLater(_)
                | UiMessage::RemoveLinkPreview(_)
        );
        self.pane.thread_focus &= !activates(&msg);
        match msg {
            UiMessage::SelectTab(tab) => {
                self.chat_menu = None;
                self.active_tab = tab;
                self.selected_account_filter = None;
                self.show_filter_menu = false;
                self.creating_chat = None;
            }
            UiMessage::SelectAccount(acc) => {
                self.chat_menu = None;
                self.active_tab = InboxTab::AllChats;
                self.selected_account_filter = acc;
                self.creating_chat = None;
            }
            UiMessage::SelectChat(_) if self.recorder.is_some() => {
                self.status_banner = tr("Stop recording to switch chats").into();
            }
            // Desktop's composer `autofocus = "end"` when a chat opens.
            UiMessage::SelectChat(id) => {
                self.select_chat(&id);
                let previews = self.fetch_link_previews();
                return Task::batch([previews, self.close_notifications(), operation::focus("composer-input")]);
            }
            UiMessage::ClickChat(id) | UiMessage::ToggleSelected(id) if self.mods.shift() => self.select_range(&id),
            UiMessage::ClickChat(id) if self.mods.control() => return self.update(UiMessage::ToggleSelected(id)),
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
                    MenuEdit::Paste => Task::batch([self.update(UiMessage::Paste), focus]),
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
            UiMessage::Undo(false) if let Some(undo) = self.shown_undo() => {
                return self.update(UiMessage::ToastAction(None, Box::new(undo)));
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
            UiMessage::SubmitComposer if self.suggestions(&self.store.read()).is_some() => {
                return self.update(UiMessage::PickSuggestion(self.pane.suggest));
            }
            UiMessage::SubmitComposer if self.recorder.is_some() => return self.send_take(),
            UiMessage::SubmitComposer if !self.can_compose() => {}
            UiMessage::SubmitComposer => {
                self.submit_composer(false);
                return self.typing(false);
            }
            UiMessage::SendAndArchive => {
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                let st = self.store.read();
                let at = self.list_ids(&st).iter().position(|c| *c == cid);
                drop(st);
                let draft = !self.pane.composer.text().trim().is_empty() || !self.pane.attachments.is_empty();
                self.submit_composer(true);
                let stop = self.typing(false);
                let list = self.list_ids(&self.store.read());
                if let Some(at) = at.filter(|_| draft && !list.contains(&cid)) {
                    return stop.chain(self.after_action("afterArchive", &list, at, true));
                }
                return stop;
            }
            UiMessage::PickSuggestion(i) => {
                // The second click of a double click hits the list that the first pick closed.
                let Some((at, picks)) = self.suggestions(&self.store.read()) else { return Task::none() };
                let mut text = self.pane.composer.text();
                let caret = format::offset(&text, self.pane.composer.cursor().position);
                let pick = match &picks[i.min(picks.len() - 1)] {
                    Suggestion::Emoji(c, _) => format!("{c} "),
                    Suggestion::Mention(label, id, _) => {
                        self.pane.mentions.push((label.clone(), id.clone()));
                        format!("@{label} ")
                    }
                };
                text.replace_range(at..caret, &pick);
                self.pane.composer = format::content(&text, at + pick.len()..at + pick.len());
                return operation::focus("composer-input");
            }
            UiMessage::CancelComposeMode if self.pane.editing_message_id.is_some() => self.pane.end_edit(),
            UiMessage::CancelComposeMode => self.pane.reply_to_message_id = None,
            UiMessage::StartReply(mid) => {
                if self.pane.editing_message_id.is_some() {
                    self.pane.end_edit();
                }
                self.pane.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            UiMessage::StartEdit(mid, old_txt) => {
                // The mention pills of the message come back from its `matrix.to` links.
                let st = self.store.read();
                let html = st.loaded(&mid).and_then(|m| m.html.clone());
                drop(st);
                // Desktop scrolls the edited message into view (`scrollIntoView` of the editing target).
                self.pane.timeline.reveal(vlist::key(&mid));
                let shown = locate(format!("msg-{mid}"), self.pane.scroll_id(), false);
                self.pane.start_edit(mid, &old_txt, html.map_or(Vec::new(), |h| html_pills(&h)));
                return Task::batch([operation::focus("composer-input"), shown]);
            }
            UiMessage::DeleteMessage(cid, mid) => {
                self.confirm = None;
                let mut st = self.store.write();
                st.redact_message(&cid, &mid).expect("delete message");
            }
            UiMessage::RemoveFromBeeper(ids) => self.remove_from_beeper(ids),
            UiMessage::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write();
                let mine =
                    stored(&st, &mid).reactions.iter().any(|r| r.reaction_key == key && st.is_me(&r.participant_id));
                if mine {
                    st.remove_reaction(&cid, &mid, &key).expect("remove reaction");
                } else {
                    st.add_reaction(&cid, &mid, &key, None).expect("add reaction");
                    st.add_recent_emoji(&key);
                    if let Some(c) = st.matrix.clone() {
                        let content = serde_json::json!({ "recent_emoji": st.recent_emoji });
                        let write = beeper_core::matrix::set_global(c, "io.element.recent_emoji", content);
                        return Task::perform(write, UiMessage::Pushed);
                    }
                }
            }
            UiMessage::ToggleBookmark(mid) => {
                self.chat_menu = None;
                self.store.write().toggle_bookmark(&mid);
            }
            UiMessage::MarkUnreadFromHere(cid, mid) => {
                self.store.write().mark_unread_from_message(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            UiMessage::MarkReadThrough(cid, mid) => {
                self.store.write().mark_read_through(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            UiMessage::VotePoll(mid, answers) => {
                let mut st = self.store.write();
                let chat = stored(&st, &mid).chat_id;
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
                    let flags = [ChatFlag::LowPriority, ChatFlag::Archived].map(|f| (f, c.flag(f)));
                    let ts = rfc3339_from_millis(at);
                    st.update_chat(i, |c| c.last_activity = c.last_activity.take().max(Some(ts)));
                    for (flag, _) in flags.into_iter().filter(|f| f.1) {
                        st.set_chat_flag(i, flag, Some(false));
                    }
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            UiMessage::OpenSendLater if !self.store.read().feature("scheduled-message").on => {}
            UiMessage::OpenSendLater if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Send Later isn't available while editing a message").into();
            }
            UiMessage::OpenSendLater => {
                if self.pane.selected_chat_id.is_some() {
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
            UiMessage::ShowThread(id) => self.show_thread(&id),
            UiMessage::SwitchCard => self.chat_menu = Some((SWITCH_CARD.into(), self.cursor.get())),
            UiMessage::SwitchChat(id) => self.switch_chat(id),
            UiMessage::HideThread => (self.pane.thread, self.pane.thread_menu) = (None, None),
            UiMessage::InThread(msg) => return self.in_thread(*msg),
            UiMessage::SetBroadcast(on) => self.pane.broadcast = on,
            UiMessage::ThreadDrag(at) => self.drag_thread(at),
            UiMessage::ResetThreadWidth => self.thread_fraction = THREAD_FRACTION,
            UiMessage::ThreadMenu => self.pane.thread_menu = Some(self.cursor.get()),
            UiMessage::MarkThreadRead => self.mark_thread_read(),
            UiMessage::ShowUnreadThreads => self.show_unread_threads(),
            UiMessage::OpenThread(chat, root) => self.open_thread(chat, root),
            UiMessage::UnreadThreadsMenu(open) => self.unread_menu = open.then(|| self.cursor.get()),
            UiMessage::MarkAllThreadsRead => self.mark_all_threads_read(),
            UiMessage::FollowThread(chat, root, on) => self.follow_thread(&chat, &root, on),
            UiMessage::FollowAllThreads(chat, on) => self.follow_all_threads(&chat, on),
            UiMessage::OpenThreadWindow(chat, root) => return self.open_thread_window(chat, root),
            UiMessage::HideReplies => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), None),
            UiMessage::FoldScheduled(collapsed) => {
                self.pane.sched_fold.0 = collapsed;
                return operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            UiMessage::TypingTick => {
                let client = self.store.read().matrix.clone();
                let panes = std::iter::once(&mut self.pane).chain(self.chat_windows.iter_mut().map(|w| &mut w.1));
                let ended: Vec<String> =
                    panes.filter_map(|p| p.typing.take_if(|t| t.1.elapsed() >= TYPING_STOP)).map(|t| t.0).collect();
                let notices = ended.into_iter().filter_map(|chat| {
                    let stop = beeper_core::matrix::typing(client.clone()?, chat, false);
                    Some(Task::perform(stop, UiMessage::Pushed))
                });
                return Task::batch(notices);
            }
            UiMessage::SaveDrafts => self.save_drafts(),
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
            UiMessage::UndoFlag(flag, chats, many) => {
                {
                    let mut st = self.store.write();
                    for (id, on) in &chats {
                        let idx = st.resolve_chat_idx(id).expect("an undone chat exists");
                        match flag {
                            ChatFlag::Archived => drop(self.set_archived(&mut st, idx, *on)),
                            _ => drop(st.set_chat_flag(idx, flag, Some(*on))),
                        }
                    }
                }
                self.refresh_theme_and_tray();
                let ids: Vec<String> = chats.into_iter().map(|c| c.0).collect();
                if flag == ChatFlag::Muted {
                    return Task::none();
                }
                match many {
                    true => (self.selection, self.select_mode) = (ids, false),
                    false => return self.update(UiMessage::SelectChat(ids[0].clone())),
                }
            }
            UiMessage::ToggleFlag(cid, flag) => {
                let at = self.list_ids(&self.store.read()).iter().position(|c| *c == cid);
                let mut st = self.store.write();
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                let was = has_flag(&st.chats[idx], flag);
                let moved = match flag {
                    ChatFlag::Archived => {
                        let on = !has_flag(&st.chats[idx], flag);
                        self.set_archived(&mut st, idx, on)
                    }
                    _ => st.set_chat_flag(idx, flag, None),
                };
                drop(st);
                if !moved {
                    self.status_banner = tr("Pinned chats can't be archived").into();
                } else if matches!(flag, ChatFlag::Archived | ChatFlag::LowPriority) {
                    self.flag_toast(flag, !was, vec![(cid.clone(), was)], false);
                } else if flag == ChatFlag::Muted {
                    let st = self.store.read();
                    let title = st.chats[st.resolve_chat_idx(&cid).expect("a muted chat")].title.clone();
                    drop(st);
                    let what = format!("{} {title}", if was { "Unmute" } else { "Mute" });
                    self.act(what, UiMessage::UndoFlag(flag, vec![(cid.clone(), was)], false), None);
                }
                self.chat_menu = None;
                self.refresh_theme_and_tray();
                let selected = self.pane.selected_chat_id.as_deref() == Some(cid.as_str());
                if let Some(at) = at.filter(|_| flag == ChatFlag::Archived && moved && selected) {
                    let list = self.list_ids(&self.store.read());
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
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Image);
                return Task::perform(pick, move |r| UiMessage::WallpaperPicked(chat.clone(), r));
            }
            UiMessage::WallpaperPicked(chat, r) => match r.map(|paths| paths.into_iter().next()) {
                Ok(Some(path)) => {
                    let dir = chat_theme::photo_dir(&self.store.read().config.data_dir);
                    let ext =
                        path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy().to_lowercase()));
                    let file_name = format!("{}{ext}", beeper_core::connect::random_id());
                    std::fs::create_dir_all(&dir).expect("create the chat wallpapers directory");
                    match std::fs::copy(&path, dir.join(&file_name)) {
                        Ok(_) => return self.update(UiMessage::ChatWallpaper(chat, Wallpaper::Photo { file_name })),
                        Err(e) => self.fail(Toast::new(format!("Could not import {}: {e}", path.display()))),
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
            UiMessage::OpenEditSpaces => {
                self.chat_menu = None;
                if let Some(d) = &self.spaces_window {
                    return window::gain_focus(d.id);
                }
                let (id, open) = self.open_child(SPACES_SIZE);
                let st = self.store.read();
                let order = rail_spaces(&st).into_iter().chain([SPACES_DIVIDER.into()]).chain(hidden_spaces(&st));
                let order = order.collect();
                drop(st);
                self.spaces_window =
                    Some(SpacesDraft { id, order, search: String::new(), drag: None, menu: None, scrolled: false });
                return open;
            }
            UiMessage::Spaces(e) => return self.spaces_edit(e),
            UiMessage::InfoDesc(None) => {
                let st = self.store.read();
                let cid = self.pane.selected_chat_id.as_deref().expect("a chat is selected");
                let c = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
                self.info_desc = Some(composer_with(c.description.as_deref().unwrap_or_default()));
                return operation::focus("info-desc");
            }
            UiMessage::InfoDesc(Some(action)) => self.info_desc.as_mut().expect("editing").perform(action),
            UiMessage::SaveInfoDesc => {
                let text = self.info_desc.take().expect("editing").text();
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                self.store.write().update_group(&cid, None, Some(text), None);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::SetDisappearing(cid, timer) => {
                self.store.write().set_disappearing(&cid, timer);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::InfoMore => self.info_more ^= true,
            UiMessage::InfoAll => self.info_all ^= true,
            UiMessage::InfoBookmarks => self.info_bookmarks ^= true,
            UiMessage::RemoveBookmark(target) => {
                self.chat_menu = None;
                self.store.write().remove_bookmark(&target);
            }
            UiMessage::BookmarkHydrated(target, m) => {
                self.hydrating.remove(&target);
                self.store.write().bookmarked.insert(target, m.map(|m| *m));
            }
            UiMessage::InfoMedia(tab) => self.info_media = tab,
            UiMessage::InfoNote(action) => {
                let edit = action.is_edit();
                self.info_note.perform(action);
                if edit {
                    let note = Some(self.info_note.text()).filter(|n| !n.is_empty());
                    let mut st = self.store.write();
                    let cid = self.pane.selected_chat_id.as_deref().expect("a chat is selected");
                    let idx = st.resolve_chat_idx(cid).expect("selected chat exists");
                    st.update_chat(idx, |c| c.note = note);
                }
            }
            UiMessage::InfoQuery(q) => (self.info_all, self.info_query) = (!q.trim().is_empty(), q),
            UiMessage::OpenEditGroup(chat) => {
                self.chat_menu = None;
                if let Some(d) = &self.group_window {
                    return window::gain_focus(d.id);
                }
                let (id, open) = self.open_child(GROUP_SIZE);
                let st = self.store.read();
                let c = &st.chats[st.resolve_chat_idx(&chat).expect("an edited group")];
                let (title, topic) =
                    (c.room_title().to_string(), composer_with(c.description.as_deref().unwrap_or_default()));
                drop(st);
                self.group_window =
                    Some(GroupDraft { id, chat, title, topic, photo: None, hover: false, personal: false });
                return open;
            }
            UiMessage::OpenEditForMe(chat) => return self.open_edit_for_me(chat),
            UiMessage::SetPersonalName(chat, name) => self.set_personal_name(chat, name),
            UiMessage::Group(e) => return self.group_edit(e),
            UiMessage::Profile(m) => return self.update_profile(m),
            UiMessage::ToggleLabel(label, chat, add) => self.toggle_label(label, chat, add, true),
            UiMessage::SwitchLabel(label, chat, add) => self.toggle_label(label, vec![chat], add, false),
            UiMessage::OpenEditLabels(chat) => {
                self.chat_menu = None;
                if let Some(&(id, _)) = self.labels_windows.iter().find(|w| w.1 == chat) {
                    return window::gain_focus(id);
                }
                let (id, open) = self.open_child(Size::new(303.0, 488.0));
                self.labels_windows.push((id, chat));
                return open;
            }
            UiMessage::UndoAction => {
                let Some((what, undo)) = self.undo_stack.pop() else { return Task::none() };
                let depth = self.undo_stack.len();
                let task = self.update(undo);
                self.undo_stack.truncate(depth);
                self.toast = None;
                self.status_banner = format!("{} {what}", tr("Undo:"));
                self.undo_note = Some(what);
                return task;
            }
            UiMessage::SetSpacesOrder(order) => {
                self.patch_settings(|s| s.spaces_order = order);
                if let Some(d) = self.spaces_window.as_mut() {
                    let st = self.store.read();
                    d.order =
                        rail_spaces(&st).into_iter().chain([SPACES_DIVIDER.into()]).chain(hidden_spaces(&st)).collect();
                }
            }
            UiMessage::RestoreDeletedLabel(l, order) => {
                let mut st = self.store.write();
                let id = st.create_label(&l.name, l.color, l.icon.clone(), l.rooms.clone());
                if l.inbox.is_some() {
                    st.set_label_inbox(&id, l.inbox);
                }
                if l.muted {
                    st.set_label_muted(&id, true);
                }
                drop(st);
                let (old, new) = (format!("label:{}", l.id), format!("label:{id}"));
                let order =
                    order.into_iter().map(|o| if o.id == old { SpaceOrder { id: new.clone(), ..o } } else { o });
                self.patch_settings(|s| s.spaces_order = order.collect());
            }
            UiMessage::RestoreLabel(l) => {
                let id = self.label_id(l.id);
                self.store.write().update_label(&id, &l.name, l.color, l.icon, l.rooms);
            }
            UiMessage::OpenCreateLabel(chats) => {
                self.chat_menu = None;
                if let Some(d) = &self.label_window {
                    return window::gain_focus(d.id);
                }
                let intro = !self.store.read().settings.labels_intro_seen;
                if intro {
                    self.patch_settings(|s| s.labels_intro_seen = true);
                }
                let (id, open) = self.open_child(LABEL_SETTINGS_SIZE);
                let step = if intro { LabelStep::Intro } else { LabelStep::Pick };
                self.label_window = Some(LabelDraft { chats, step, ..LabelDraft::new(id) });
                return if intro { open } else { open.chain(operation::focus("label-search")) };
            }
            UiMessage::EditLabel(label) => return self.open_label_sheet(label, LabelStep::Details),
            UiMessage::EditLabelChats(label) => return self.open_label_sheet(label, LabelStep::Pick),
            UiMessage::MuteLabel(id, muted) => {
                self.chat_menu = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let name = st.labels.iter().find(|l| l.id == id).expect("a muted label").name.clone();
                st.set_label_muted(&id, muted);
                drop(st);
                let what = format!("{} \"{name}\"", if muted { "Mute" } else { "Unmute" });
                self.act(what, UiMessage::MuteLabel(id, !muted), None);
            }
            UiMessage::SetLabelInbox(id, inbox) => {
                self.chat_menu = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let old = st.labels.iter().find(|l| l.id == id).expect("a label").clone();
                st.set_label_inbox(&id, inbox);
                drop(st);
                let to = INBOX_OPTIONS.iter().find(|o| o.0 == inbox.unwrap_or("all")).expect("an inbox option").1;
                let what = format!("Set \"{}\" inbox visibility to {to}", old.name);
                self.act(what, UiMessage::SetLabelInbox(id, old.inbox), None);
            }
            UiMessage::OpenLabelSettings(label) => {
                self.chat_menu = None;
                let label = self.label_id(label);
                if let Some(w) = self.label_settings.iter().find(|w| w.label == label) {
                    return window::gain_focus(w.id);
                }
                let (id, open) = self.open_child(LABEL_SETTINGS_SIZE);
                self.label_settings.push(LabelSettings { id, label, menu: false });
                return open;
            }
            UiMessage::LabelPane(id, e) => {
                let w = self.label_settings.iter_mut().find(|w| w.id == id).expect("an open Label Settings window");
                match e {
                    LabelPane::Menu(open) => w.menu = open,
                    LabelPane::Delete => {
                        (self.confirm, self.delete_target) = (Some(ASK_DELETE_LABEL), Some(w.label.clone()));
                        self.delete_in = Some(id);
                    }
                    LabelPane::Done => return window::close(id),
                }
            }
            UiMessage::DeleteLabel(id) => {
                self.confirm = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let label = st.labels.iter().find(|l| l.id == id).expect("a deleted label").clone();
                let order = st.settings.spaces_order.clone();
                st.delete_label(&id);
                drop(st);
                let (what, text) = (format!("Delete label \"{}\"", label.name), format!("Deleted \"{}\"", label.name));
                self.act(what, UiMessage::RestoreDeletedLabel(Box::new(label), order), Some((text, None)));
                if let Some(d) = self.spaces_window.as_mut() {
                    d.order.retain(|o| *o != format!("label:{id}"));
                }
                if self.selected_account_filter.as_deref() == Some(&format!("label:{id}")) {
                    self.selected_account_filter = None;
                }
                let shown = self.label_settings.iter().filter(|w| w.label == id);
                return Task::batch(shown.map(|w| window::close(w.id)).collect::<Vec<_>>());
            }
            UiMessage::Label(edit) => return self.label_edit(edit),
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
                let list = self.list_ids(&self.store.read());
                let mut st = self.store.write();
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                let c = &st.chats[idx];
                if c.is_unread() {
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
                return iced::clipboard::write(deeplink::chat_link(&c.account_id, &c.id));
            }
            UiMessage::CloseChatMenu => self.chat_menu = None,
            UiMessage::OpenChatMenu(cid) => {
                self.chat_menu = Some((cid, self.cursor.get()));
                self.chat_sub = None;
            }
            UiMessage::OpenMessageMenu(mid) => {
                self.video.iter_mut().for_each(|v| v.sub = false);
                (self.audio.sub, self.audio.lang) = (false, false);
                self.pane.message_menu = Some((mid, self.cursor.get()));
                self.pane.gallery_menu.clear();
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
                let st = self.store.read();
                let action = st.settings.pref("messageDoubleClick");
                let reactable = st.loaded(&mid).is_some_and(Message::reactable);
                drop(st);
                return match action {
                    1 | 5 if !reactable => Task::none(),
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
                    Some(mid) => {
                        self.pane.timeline.reveal(vlist::key(mid));
                        locate(format!("msg-{mid}"), self.pane.scroll_id(), true)
                    }
                    None => operation::focus("composer-input"),
                };
            }
            UiMessage::React(mid) => return self.open_emoji(Some((mid, self.cursor.get()))),
            UiMessage::ReactAt(mid, at) => return self.open_emoji(Some((mid, at))),
            UiMessage::MessageMenu(msg) => {
                self.pane.message_menu = None;
                return self.update(*msg);
            }
            UiMessage::CopyText(s) => return iced::clipboard::write(s),
            UiMessage::CopyImage(mid, ai) => {
                let st = self.store.read();
                let a = &stored(&st, &mid).attachments[ai];
                let url = a.src_url.clone().expect("an image has a source");
                let fetch = beeper_core::matrix::media(st.matrix.clone(), url);
                let decode = async move {
                    let bytes = fetch.await?;
                    let decode = move || {
                        let px = ::image::load_from_memory(&bytes).map_err(|e| e.to_string())?.into_rgba8();
                        Ok((px.width() as usize, px.height() as usize, px.into_raw()))
                    };
                    tokio::task::spawn_blocking(decode).await.expect("image decode task")
                };
                return Task::perform(decode, UiMessage::ImageFetched);
            }
            UiMessage::ImageFetched(r) => {
                let copy = |(width, height, px): (usize, usize, Vec<u8>), c: &mut Option<arboard::Clipboard>| {
                    let img = arboard::ImageData { width, height, bytes: px.into() };
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
                self.leave_carousel_video();
                self.carousel =
                    Some(Carousel { mid, ai, turns: 0, turned: None, in_media, resume: None, menu: None, strip: None });
                self.fit_carousel_video();
                return self.load_media();
            }
            UiMessage::CarouselStep(next) => {
                // The second click of a double click on an arrow can step past the first or the last image.
                let item = self.carousel_items(&self.store.read()).and_then(|(items, at)| {
                    let to = if next { at + 1 } else { at.checked_sub(1)? };
                    items.into_iter().nth(to)
                });
                let Some(item) = item else { return Task::none() };
                self.leave_carousel_video();
                let c = self.carousel.as_mut().expect("the carousel is open");
                (c.mid, c.ai, c.resume, c.menu, c.strip) = (item.0, item.1, None, None, None);
                self.fit_carousel_video();
                return self.load_media();
            }
            UiMessage::ScrollFilmstrip(by) => return self.scroll_filmstrip(by),
            UiMessage::CarouselMenu(open) => {
                let at = self.cursor.get();
                self.carousel.as_mut().expect("the carousel is open").menu = open.then_some(at);
            }
            UiMessage::RotateCarousel => {
                let c = self.carousel.as_mut().expect("the carousel is open");
                (c.turns, c.turned) = ((c.turns + 1) % 4, Some(std::time::Instant::now()));
            }
            UiMessage::CloseCarousel => {
                let task = self.close_carousel_video();
                self.carousel = None;
                return task;
            }
            UiMessage::OpenMedia => {
                if let Some(w) = &self.media_window {
                    return window::gain_focus(w.id);
                }
                // Desktop's `media-viewer` window state.
                let settings = self.saved_window_settings("media-viewer", Size::new(1000.0, 720.0));
                let (size, (id, open)) = (settings.size, window::open(settings));
                self.media_window = Some(MediaWindow {
                    id,
                    tab: 0,
                    size,
                    shown: MEDIA_PAGE,
                    sent_by: 0,
                    oldest: false,
                    search: None,
                    tab_at: None,
                    menu: None,
                    item_menu: None,
                    select: false,
                    picked: Vec::new(),
                    deleting: None,
                });
                return Task::batch([open.map(UiMessage::WindowOpened), self.load_media()]);
            }
            UiMessage::MediaTab(tab) => {
                let w = self.media_mut();
                (w.tab_at, w.tab) = (Some((w.tab, std::time::Instant::now())), tab);
                return self.media_reload();
            }
            UiMessage::MediaSentBy(n) => {
                let w = self.media_mut();
                (w.sent_by, w.menu) = (n, None);
                return self.media_reload();
            }
            UiMessage::MediaOldest(on) => {
                let w = self.media_mut();
                (w.oldest, w.menu) = (on, None);
                return self.media_reload();
            }
            UiMessage::MediaSearch(q) => {
                let w = self.media_mut();
                let open = q.is_some() && w.search.is_none();
                w.search = q;
                let reload = self.media_reload();
                return if open { Task::batch([reload, operation::focus("media-search")]) } else { reload };
            }
            UiMessage::MediaMenu(open) => {
                let at = self.cursor.get();
                self.media_mut().menu = open.then_some(at);
            }
            UiMessage::Modifiers(mods) => {
                // Electron's `autoHideMenuBar`: Alt on its own shows the menu bar, and hides it again.
                let was = std::mem::replace(&mut self.mods, mods);
                match (was == Modifiers::ALT, mods == Modifiers::ALT) {
                    (false, true) if was.is_empty() => self.alt_alone = true,
                    (true, false) if mods.is_empty() && std::mem::take(&mut self.alt_alone) => {
                        if self.main_window.is_some() {
                            self.menu_bar = if self.menu_bar.is_some() { None } else { Some(None) };
                        }
                    }
                    _ => self.alt_alone = false,
                }
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
                let media = self.media_messages(&st);
                let all = media_items(&st, &media, w).into_iter().take(w.shown).map(|(i, k)| (media[i].id.clone(), k));
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
                    let m = &stored(&st, mid);
                    if deletable(&st, m) && !pairs.iter().any(|(_, x)| x == mid) {
                        pairs.push((m.chat_id.clone(), mid.clone()));
                    }
                }
                let skipped = items.iter().filter(|(mid, _)| !deletable(&st, &stored(&st, mid))).count();
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
                        st.redact_message(cid, mid).expect("delete message");
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
            UiMessage::WindowText(id, text) if self.chat_windows.iter().any(|w| w.0 == id) => {
                return self.update(UiMessage::InWindow(id, Box::new(UiMessage::TypeInComposer(text))));
            }
            UiMessage::WindowText(id, text) if Some(id) == self.main_window => {
                return self.update(UiMessage::TypeInComposer(text));
            }
            UiMessage::WindowText(id, text) if Some(id) == self.login_window => return self.login_text(&text),
            UiMessage::WindowText(..) => {}
            UiMessage::TypeInComposer(text) => {
                let open = self.can_compose();
                let quiet = self.confirm.is_none()
                    && !self.show_command_palette
                    && self.forward.is_none()
                    && self.creating_chat.is_none()
                    && self.carousel.is_none()
                    && self.recorder.is_none();
                if !(open && quiet) {
                    return Task::none();
                }
                let old = self.pane.composer.text();
                self.pane.composer.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                for c in text.chars() {
                    self.pane.composer.perform(text_editor::Action::Edit(text_editor::Edit::Insert(c)));
                }
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            // Desktop's menu roles "close" and "minimize", whose accelerators work while the menu bar hides.
            UiMessage::WindowKey(id, Key::Character(c), mods)
                if mods == Modifiers::COMMAND && matches!(c.to_lowercase().as_str(), "w" | "m") =>
            {
                return match c.eq_ignore_ascii_case("w") {
                    true => self.update(UiMessage::CloseRequested(id)),
                    false => window::minimize(id, true),
                };
            }
            UiMessage::WindowKey(id, key, mods) if Some(id) == self.login_window || !self.store.read().signed_in() => {
                return self.login_key(key, mods);
            }
            UiMessage::WindowKey(_, Key::Named(Named::Escape), _) if self.menu_bar.is_some() => self.menu_bar = None,
            UiMessage::WindowKey(id, key, mods) => {
                if Some(id) == self.settings_window {
                    return self.settings_key(key, mods);
                }
                if self.label_asking(id) && key == Key::Named(Named::Escape) {
                    self.confirm = None;
                    return Task::none();
                }
                if self.label_settings.iter().any(|w| w.id == id) {
                    return match key {
                        Key::Named(Named::Escape) => window::close(id),
                        Key::Named(Named::Enter) if mods.command() => window::close(id),
                        _ => Task::none(),
                    };
                }
                if self.theme_windows.iter().any(|w| w.0 == id)
                    || self.labels_windows.iter().any(|w| w.0 == id)
                    || self.vote_windows.iter().any(|w| w.0 == id)
                    || self.spaces_window.as_ref().is_some_and(|d| d.id == id)
                    || self.group_window.as_ref().is_some_and(|d| d.id == id)
                    || Some(id) == self.blocked_window
                    || self.add_accounts.iter().any(|a| a.id == id)
                {
                    return if key == Key::Named(Named::Escape) { window::close(id) } else { Task::none() };
                }
                if self.reauth.window == Some(id) {
                    return self.reauth_key(key);
                }
                if self.enter_key.window == Some(id) {
                    return self.enter_key_key(key);
                }
                if self.label_window.as_ref().is_some_and(|d| d.id == id) {
                    return match key {
                        Key::Named(Named::Escape) => window::close(id),
                        Key::Named(Named::Enter) if mods.command() => self.label_edit(LabelEdit::Advance),
                        _ => Task::none(),
                    };
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
                let asks = self.media_window.as_ref().is_some_and(|w| w.deleting.is_some());
                if self.carousel.as_ref().is_some_and(|c| c.in_media) && !asks {
                    return self.carousel_key(&key, mods);
                }
                if key == Key::Named(Named::Escape) {
                    let w = self.media_mut();
                    if w.item_menu.is_some() {
                        w.item_menu = None;
                    } else if w.menu.is_some() {
                        w.menu = None;
                    } else if w.deleting.is_some() {
                        w.deleting = None;
                    } else if let Some(q) = w.search.as_ref().map(|q| (!q.is_empty()).then(String::new)) {
                        return self.update(UiMessage::MediaSearch(q));
                    } else if w.select {
                        (w.select, w.picked) = (false, Vec::new());
                    } else {
                        return self.update(UiMessage::CloseRequested(id));
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
                // The second click of a double click on Send hits the dialog that the first click closed.
                let Some(Forward { mids, picked, .. }) = self.forward.take() else { return Task::none() };
                assert!(!picked.is_empty() && !mids.is_empty(), "forward needs a chat and a message");
                let mut st = self.store.write();
                for cid in &picked {
                    for mid in &mids {
                        let m = stored(&st, mid);
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
                let m = &stored(&st, &mid);
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
                self.sticky_banner = Some(self.status_banner.clone());
                let (dir, ask) = (downloads_dir(&st.settings), st.settings.on("alwaysAskDownload"));
                return Task::perform(
                    async move {
                        let mut last = String::new();
                        for (url, name) in files {
                            let name = safe_file_name(&name);
                            if name.trim_matches('.').is_empty() {
                                return Err(format!("bad file name {name:?}"));
                            }
                            // Desktop's `always_ask_download_location`: the save dialog picks each path.
                            let path = match ask {
                                true => {
                                    let save = portal::Pick::Save(name.clone());
                                    let picked = portal::open_files(portal::PORTAL.into(), tr("Save").into(), save);
                                    match picked.await?.into_iter().next() {
                                        Some(path) => path,
                                        None => continue,
                                    }
                                }
                                false => {
                                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                                    unused_path(&dir, &name)
                                }
                            };
                            let bytes = beeper_core::matrix::media(client.clone(), url).await?;
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
            UiMessage::AskDeleteMessage(mids) => {
                (self.confirm, self.delete_target) = (Some(ASK_DELETE_MESSAGE), Some(mids.join(" ")))
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
            UiMessage::CommandQueryChanged(q) => {
                (self.command_palette_query, self.cmd_index) = (q.clone(), 0);
                if self.cmd_submenu == Some(commands::SubMenu::AddMembers) {
                    self.new_chat_search = None;
                    let wait = async { tokio::time::sleep(std::time::Duration::from_millis(300)).await };
                    return Task::perform(wait, move |()| UiMessage::SearchMembers(q.clone()));
                }
            }
            UiMessage::SearchMembers(q) => {
                let q = q.trim();
                let open = self.cmd_submenu == Some(commands::SubMenu::AddMembers);
                let chat = self.pane.selected_chat_id.clone().filter(|_| open && !q.is_empty());
                let Some(chat) = chat.filter(|_| q == self.command_palette_query.trim()) else {
                    return Task::none();
                };
                let st = self.store.read();
                let account = st.resolve_chat_idx(&chat).map(|i| st.chats[i].account_id.clone());
                drop(st);
                return self.search_users(q, account);
            }
            UiMessage::AddMembers(q) => {
                let open = self.update(UiMessage::OpenSubMenu(commands::SubMenu::AddMembers));
                return Task::batch([open, self.update(UiMessage::CommandQueryChanged(q))]);
            }
            UiMessage::ExportChat => {
                let st = self.store.read();
                let Some(text) = self.export_text(&st) else { return Task::none() };
                let title = st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().unwrap_or_default());
                let title = safe_file_name(&st.chats[title.expect("an open chat")].title);
                drop(st);
                let home = beeper_core::platform::resolve_home_dir(&std::env::vars().collect());
                let path = home.join(format!("Desktop/Beeper export {title} {}.txt", now_unix_millis()));
                self.status_banner = match std::fs::write(&path, text) {
                    Ok(()) => format!("Exported to {}", path.display()),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
            UiMessage::AddMember(chat, user) => {
                let mut st = self.store.write();
                // The second click of a double click hits the row of the person that the first click added.
                if st.chat(&chat).participants.items.iter().all(|u| u.id != user.id) {
                    st.add_member(&chat, *user);
                }
            }
            UiMessage::CommandHighlight(i) => self.cmd_index = i,
            UiMessage::Commands(msgs) => {
                return Task::batch(msgs.into_iter().map(|m| self.update(m)).collect::<Vec<_>>())
            }
            UiMessage::RunHotkey(name) => {
                self.chat_menu = None;
                return self.run_hotkey(name, 0);
            }
            UiMessage::ToggleMaximize(id) => {
                return window::toggle_maximize(id.or(self.main_window).expect("the main window"));
            }
            UiMessage::JumpTo(cid, mid) => return self.jump_to(cid, mid),
            UiMessage::ExecuteCommandChat(cid) => {
                self.show_command_palette = false;
                self.command_palette_query.clear();
                self.select_chat(&cid);
            }
            UiMessage::NewChat => {
                self.chat_menu = None;
                let id = self.new_chats.iter().map(|d| d.id).max().map_or(0, |n| n + 1);
                self.new_chats.insert(0, NewChat { id, users: Vec::new(), group: false, title: String::new() });
                return self.update(UiMessage::SelectNewChat(id));
            }
            UiMessage::SelectNewChat(id) => {
                self.creating_chat = Some(id);
                self.show_command_palette = false;
                self.new_chat_query.clear();
                (self.new_chat_expanded, self.new_chat_chats_expanded) = (false, false);
                (self.new_chat_account, self.new_chat_accounts_open) = (None, false);
                (self.contact_networks, self.new_chat_lookup, self.new_chat_search) = (None, None, None);
                self.new_chat_results_expanded = false;
                self.join_matrix_mode = false;
                return operation::focus("new-chat-input");
            }
            UiMessage::ToggleContact(id) => {
                if self.contact_networks.take().is_some_and(|(c, _)| c == id) {
                    return Task::none();
                }
                let st = self.store.read();
                let c = st.contacts.iter().find(|u| u.id == id).expect("a listed contact").clone();
                let accounts: Vec<String> =
                    rail_accounts(&st).into_iter().filter(|a| self.can(a, "create_dm")).collect();
                drop(st);
                let (phone, email, username) = (c.phone_number.as_deref(), c.email.as_deref(), c.username.as_deref());
                let (nets, tasks) = self.resolve_identifiers(&id, &accounts, phone, email, username);
                self.contact_networks = Some((id, nets));
                return tasks;
            }
            UiMessage::LookupIdentifier(q) => {
                let q = q.trim();
                if q != self.new_chat_query.trim() || q.is_empty() {
                    return Task::none();
                }
                let only =
                    self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat).and_then(|d| d.users.first());
                let only = only.map(|(a, _)| a.clone()).or_else(|| self.new_chat_account.clone());
                let search = self.search_users(q, only);
                let phone = typed_phone(q).then(|| fmt_phone(&phone_digits(q)));
                let email = (q.contains('@') && !q.starts_with('@')).then_some(q);
                let named = q.as_bytes().windows(2).any(|w| w[0] == b'@' && w[1].is_ascii_alphanumeric());
                let username = named.then_some(q);
                if phone.is_none() && email.is_none() && username.is_none() {
                    return search;
                }
                let st = self.store.read();
                let only =
                    self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat).and_then(|d| d.users.first());
                let only = only.map(|(a, _)| a.clone()).or_else(|| self.new_chat_account.clone());
                let listed = rail_accounts(&st).into_iter().filter(|a| self.can(a, "create_dm"));
                let accounts: Vec<String> = listed.filter(|a| only.as_ref().is_none_or(|o| o == a)).collect();
                drop(st);
                let key = format!("lookup:{q}");
                let (nets, tasks) = self.resolve_identifiers(&key, &accounts, phone.as_deref(), email, username);
                self.new_chat_lookup = Some((key, nets));
                return Task::batch([search, tasks]);
            }
            UiMessage::SearchedUsers(q, account, r) => {
                let Some((_, pending, found)) = self.new_chat_search.as_mut().filter(|(s, ..)| *s == q) else {
                    return Task::none();
                };
                *pending -= 1;
                let users = r.inspect_err(|e| eprintln!("search users failed for {account}: {e}")).unwrap_or_default();
                found.extend(users.iter().map(|v| (account.clone(), resolved_user(v))));
            }
            UiMessage::ToggleNewChatResults => self.new_chat_results_expanded = !self.new_chat_results_expanded,
            UiMessage::ContactNetwork(id, slot, r) => {
                let open = [&mut self.contact_networks, &mut self.new_chat_lookup].into_iter().flatten();
                let Some((_, nets)) = open.into_iter().find(|(c, _)| *c == id) else {
                    return Task::none();
                };
                let found = r.inspect_err(|e| eprintln!("resolve identifier failed: {e}")).ok().flatten();
                nets[slot].1 = Some(found.map(|v| resolved_user(&v)));
            }
            UiMessage::PickUser(account, user) => {
                let n = self.creating_chat.expect("an open draft");
                let group = self.can_group(&account);
                let draft = self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft");
                if !group {
                    draft.users = vec![(account, *user)];
                } else if draft.users.iter().all(|(_, u)| u.id != user.id) {
                    draft.users.push((account, *user));
                } else {
                    return Task::none();
                }
                draft.group |= draft.users.len() > 1;
                self.new_chat_query.clear();
                return operation::focus("new-chat-input");
            }
            UiMessage::NewGroup => {
                let n = self.creating_chat.expect("an open draft");
                self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").group = true;
                return operation::focus("new-chat-input");
            }
            UiMessage::GroupTitle(t) => {
                let n = self.creating_chat.expect("an open draft");
                self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").title = t;
            }
            UiMessage::StartChat => {
                let Some(n) = self.creating_chat else { return Task::none() };
                let draft = self.new_chats.iter().find(|d| d.id == n).expect("the open draft");
                let Some((account, user)) = draft.users.first().cloned() else { return Task::none() };
                if draft.group {
                    let users = draft.users.iter().map(|(_, u)| u.id.clone()).collect();
                    let named = self.caps[&account]["group_creation"]["group"]["name"]["allowed"] == true;
                    let title = if named { draft.title.clone() } else { String::new() };
                    self.status_banner = tr("Creating group chat...").into();
                    self.sticky_banner = Some(self.status_banner.clone());
                    let create =
                        beeper_core::matrix::create_group(self.provision(&account), "group".into(), title, users);
                    return Task::perform(create, move |r| UiMessage::GroupCreated(n, account.clone(), r));
                }
                return self.open_dm(account, user, Some(n), None);
            }
            UiMessage::MessageMember(cid, uid) => {
                self.chat_menu = None;
                self.user_hover = UserHover { r#gen: self.user_hover.r#gen + 1, ..UserHover::default() };
                let st = self.store.read();
                let c = st.chat(&cid);
                let user = c.participants.items.iter().find(|u| u.id == uid).cloned().expect("a member of the chat");
                let account = c.account_id.clone();
                drop(st);
                return self.open_dm(account, user, None, None);
            }
            UiMessage::AskMember(ask, cid, uid) => {
                (self.chat_menu, self.confirm, self.delete_target) = (None, Some(ask), Some(format!("{cid} {uid}")));
            }
            UiMessage::SetAdmin(cid, uid, admin) => {
                self.confirm = None;
                self.store.write().set_admin(&cid, &uid, admin);
            }
            UiMessage::RemoveMember(cid, uid) => {
                self.confirm = None;
                self.store.write().remove_member(&cid, &uid);
            }
            UiMessage::ReplyPrivately(mid) => {
                let st = self.store.read();
                let m = &stored(&st, &mid);
                let c = st.chat(&m.chat_id);
                let sender = c.participants.items.iter().find(|u| u.id == m.sender_id).cloned().expect("the sender");
                let quote = copy_text(m, false, false).lines().map(|l| format!("> {l}\n")).collect();
                let account = c.account_id.clone();
                drop(st);
                return self.open_dm(account, sender, None, Some(quote));
            }
            UiMessage::DmCreated(n, account, quote, r) => self.chat_created(n, &account, quote, r, "creating thread"),
            UiMessage::GroupCreated(n, account, r) => {
                self.chat_created(Some(n), &account, None, r, "creating group chat")
            }
            UiMessage::DeleteNewChat(id) => {
                // The second click of a double click on the close button hits the draft that the first click removed.
                let Some(i) = self.new_chats.iter().position(|d| d.id == id) else { return Task::none() };
                self.new_chats.remove(i);
                self.chat_menu = None;
                if self.creating_chat != Some(id) {
                    return Task::none();
                }
                self.creating_chat = None;
                let st = self.store.read();
                let rest = self.pins_and_rows(&st).1.into_iter().map(|c| UiMessage::SelectChat(c.id.clone()));
                let items: Vec<_> = self.new_chats.iter().map(|d| UiMessage::SelectNewChat(d.id)).chain(rest).collect();
                drop(st);
                if let Some(next) = items.get(i).or(i.checked_sub(1).and_then(|j| items.get(j))) {
                    return self.update(next.clone());
                }
            }
            UiMessage::NewChatQueryChanged(q) => {
                (self.new_chat_query, self.join_room.1) = (q.clone(), None);
                (self.new_chat_lookup, self.new_chat_search) = (None, None);
                let wait = async { tokio::time::sleep(std::time::Duration::from_millis(300)).await };
                return Task::perform(wait, move |()| UiMessage::LookupIdentifier(q.clone()));
            }
            UiMessage::ToggleNewChatExpanded => self.new_chat_expanded = !self.new_chat_expanded,
            UiMessage::ToggleNewChatChats => self.new_chat_chats_expanded = !self.new_chat_chats_expanded,
            UiMessage::JoinMatrixRoom => return self.join_matrix_room(),
            UiMessage::MatrixRoomJoined(draft, r, unrouted) => return self.matrix_room_joined(draft, r, unrouted),
            UiMessage::ToggleJoinMatrix => {
                self.join_matrix_mode = !self.join_matrix_mode;
                self.new_chat_query.clear();
                return operation::focus("new-chat-input");
            }
            UiMessage::SelectSettingsTab(SettingsTab::Help) | UiMessage::OpenSettings(SettingsTab::Help) => {
                return Task::done(UiMessage::OpenUrl("https://help.beeper.com/?ref=desktop-app".into()));
            }
            UiMessage::SelectSettingsTab(t)
                if self.active_settings_tab == SettingsTab::Profile
                    && t != SettingsTab::Profile
                    && self.profile.dirty(self.store.read().me()) =>
            {
                self.profile.leaving = Some(t);
            }
            UiMessage::SelectSettingsTab(t) => {
                let top = operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                self.active_settings_tab = t;
                (self.account_page, self.app_qr) = (None, false);
                return Task::batch([top, self.load_devices()]);
            }
            UiMessage::DevicesLoaded(r) => (self.devices, self.devices_busy) = (Some(r), false),
            UiMessage::AskLogoutDevice(id) => (self.confirm, self.delete_target) = (Some(ASK_LOGOUT_DEVICE), Some(id)),
            UiMessage::LogoutDevice(id) => return self.logout_device(id),
            UiMessage::DeviceLoggedOut(id, r) => return self.device_logged_out(id, r),
            UiMessage::MicsLoaded(r) => {
                if let Err(e) = &r {
                    eprintln!("Unable to load audio input devices: {e}");
                }
                self.mics = Some(r);
            }
            UiMessage::SetMicrophone(name) => self.patch_settings(|s| s.microphone = name),
            UiMessage::ShareAnalytics => {
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
                        move |r| UiMessage::AnalyticsSaved(was, r.map_err(|e| e.message)),
                    );
                }
            }
            UiMessage::AnalyticsSaved(_, Ok(())) => {}
            UiMessage::AnalyticsSaved(was, Err(e)) => {
                self.store.write().whoami.analytics_disabled = was;
                self.fail(Toast::new(trf("Something went wrong: %s", &[&e])));
            }
            UiMessage::Ask(dialog) => (self.confirm, self.reminder_menu) = (dialog, None),
            UiMessage::AskDeleteChat(id) => {
                (self.confirm, self.chat_menu, self.delete_target) = (Some(ASK_DELETE_CHAT), None, Some(id));
            }
            UiMessage::BeginSelecting(id) => {
                (self.selection, self.chat_menu, self.select_mode) = (vec![id.clone()], None, false);
                self.select_chat(&id);
            }
            UiMessage::ToggleSelectMode => {
                self.chat_menu = None;
                if self.active_tab != InboxTab::Requests || self.selected_account_filter.is_some() {
                    self.select_mode = !self.select_mode && self.selection.is_empty();
                    self.selection.clear();
                }
            }
            UiMessage::ToggleSelected(id) => {
                self.select_mode = false;
                match self.selection.iter().position(|c| *c == id) {
                    Some(i) => drop(self.selection.remove(i)),
                    None => self.selection.push(id.clone()),
                }
                self.select_chat(&id);
            }
            UiMessage::SelectionSet(flag, on) => {
                (self.selection_menu, self.chat_menu) = (None, None);
                let many = self.selection.len() > 1;
                let what = match &self.selection[..] {
                    [one] => {
                        let st = self.store.read();
                        st.chats[st.resolve_chat_idx(one).expect("a selected chat")].title.clone()
                    }
                    all => format!("{} chats", all.len()),
                };
                let mut undo = Vec::new();
                {
                    let mut st = self.store.write();
                    for cid in std::mem::take(&mut self.selection) {
                        let idx = st.resolve_chat_idx(&cid).expect("selected chat exists");
                        let moved = has_flag(&st.chats[idx], flag) != on
                            && match flag {
                                ChatFlag::MarkedUnread if !on => {
                                    self.mark_read(&mut st, idx);
                                    true
                                }
                                ChatFlag::Archived => {
                                    self.set_archived(&mut st, idx, on) || {
                                        self.status_banner = tr("Pinned chats can't be archived").into();
                                        false
                                    }
                                }
                                _ => st.set_chat_flag(idx, flag, Some(on)),
                            };
                        if moved || many && flag == ChatFlag::Archived {
                            undo.push((cid, !on));
                        }
                    }
                }
                if matches!(flag, ChatFlag::Archived | ChatFlag::LowPriority) {
                    self.flag_toast(flag, on, undo, many);
                } else if flag == ChatFlag::Muted && !undo.is_empty() {
                    let what = format!("{} {what}", if on { "Mute" } else { "Unmute" });
                    self.act(what, UiMessage::UndoFlag(flag, undo, false), None);
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
            UiMessage::AskLeaveChat(id) => (self.confirm, self.delete_target) = (Some(ASK_LEAVE_CHAT), Some(id)),
            UiMessage::AskBlock(id, block) => self.ask_block(id, block),
            UiMessage::BlockUser(id, block, spam) => return self.block_user(&id, block, spam),
            UiMessage::ReportSpam(on) => self.report_spam = on,
            UiMessage::OpenBlockedUsers => return self.open_blocked_users(),
            UiMessage::CloseBlockedUsers => return window::close(self.blocked_window.expect("Blocked users is open")),
            UiMessage::OpenBlockedUser(id) => return self.open_blocked_user(id),
            UiMessage::LeaveRoom(id) => {
                self.confirm = None;
                self.store.write().leave_room(&id);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::AcceptRequest(id) => {
                self.chat_menu = None;
                self.store.write().accept_request(&id);
                return self.update(UiMessage::StoreChanged);
            }
            UiMessage::DeleteChat(id, everyone) => {
                self.chat_menu = None;
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
            UiMessage::LoggedOut(Err(e)) => self.fail(Toast::new(format!("Could not log out: {e}"))),
            UiMessage::ToggleRightPanel => {
                // Desktop shows Chat Info or the thread pane, not both.
                let closed = self.pane.thread.take().is_some();
                self.show_right_panel = closed || !self.show_right_panel;
            }
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
            UiMessage::CloudAccounts(Ok((profile, accounts))) => {
                let (token, uid, demo) = {
                    let mut st = self.store.write();
                    let email = profile.email.clone();
                    st.features.subscribed = profile.plan != Plan::Free;
                    st.whoami = profile;
                    let info = (st.beeper_state.access_token.clone(), st.beeper_state.user_id.clone(), st.config.demo);
                    if st.me().email != email {
                        let i = st.chat_idx(NOTE_CHAT_ID);
                        st.update_chat(i, |c| c.participants.items[0].email = email);
                    }
                    st.set_cloud_accounts(accounts.clone());
                    info
                };
                if !demo {
                    let connected = accounts.iter().filter(|a| a.state == "CONNECTED");
                    let caps: Vec<_> = connected
                        .map(|a| {
                            let id = a.account_id.clone();
                            Task::perform(beeper_core::matrix::capabilities(self.provision(&id)), move |r| {
                                UiMessage::Capabilities(id.clone(), r)
                            })
                        })
                        .collect();
                    let cloud = self.cloud.clone();
                    let contacts = Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || cloud.contacts(&token, &uid, &accounts))
                                .await
                                .expect("contacts worker thread")
                        },
                        UiMessage::ContactsLoaded,
                    );
                    return Task::batch(caps).chain(contacts);
                }
            }
            UiMessage::Capabilities(account, r) => match r {
                Ok(v) => drop(self.caps.insert(account, v)),
                Err(e) => eprintln!("provisioning capabilities failed: {e}"),
            },
            UiMessage::NewChatAccount(account) => {
                (self.new_chat_account, self.new_chat_accounts_open) = (account, false);
            }
            UiMessage::ToggleNewChatAccounts => self.new_chat_accounts_open = !self.new_chat_accounts_open,
            UiMessage::ContactsLoaded(contacts) => {
                if !contacts.is_empty() {
                    self.store.write().set_contacts(contacts);
                }
            }
            UiMessage::CloudAccounts(Err(e)) => {
                self.fail(Toast::new(format!("Could not load your chat accounts: {e}")))
            }
            UiMessage::AvatarLoaded(url, Ok(h)) => {
                self.avatars.insert(url, Some(h));
            }
            UiMessage::AvatarLoaded(_, Err(_)) => {}
            UiMessage::MediaLoaded(url, r) => {
                self.media.insert(url, Some(r));
            }
            UiMessage::TileBaked(key, h) => self.tiles.insert(key, Some(h)),
            UiMessage::RetryMedia(url) => {
                self.media.remove(&url);
                return self.load_media();
            }
            UiMessage::ResolveMedia(account_id, event_id, raw) => {
                if self.resolving_media.get(&event_id) == Some(&true) {
                    return Task::none();
                }
                self.resolving_media.insert(event_id.clone(), true);
                let p = self.provision(&account_id);
                let (eid, again) = (event_id.clone(), (account_id, raw.clone()));
                // Like Desktop, a resolve whose edit does not arrive in 30 s fails.
                let resolve = async move {
                    beeper_core::matrix::resolve_media(p, eid, raw).await?;
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    Ok(())
                };
                return Task::perform(resolve, move |r| UiMessage::MediaResolved(again.0, event_id, again.1, r));
            }
            UiMessage::MediaResolved(account_id, event_id, raw, r) => {
                self.resolving_media.insert(event_id.clone(), false);
                if let Err(e) = r {
                    eprintln!("resolve media failed: {e}");
                    let again = UiMessage::ResolveMedia(account_id, event_id, raw);
                    self.fail(Toast {
                        buttons: vec![(tr("Try again"), again)],
                        ..Toast::new(tr("Could not load this media.").into())
                    });
                }
            }
            UiMessage::Video(msg) => return self.video(msg),
            UiMessage::Gallery(msg) => return self.gallery(msg),
            UiMessage::Paste => return Task::perform(clipboard_paste(), UiMessage::Pasted),
            // Desktop's `paste` handler: copied files go to the attachments, and a text goes into the composer.
            UiMessage::Pasted(r) => match r {
                Ok(Clip::Files(paths)) => return self.update(UiMessage::FilesPicked(Ok(paths))),
                Ok(Clip::Image((png, size))) => self.attach_files(vec![Ok(PendingMedia {
                    name: "image.png".into(),
                    mime: "image/png".into(),
                    is_gif: false,
                    size,
                    frames: vec![(image::Handle::from_bytes(png.clone()), 0)],
                    bytes: png,
                    duration: None,
                    sticker: None,
                })]),
                Ok(Clip::Text) => {
                    let id = if self.pane.thread_root.is_some() { "thread-composer-input" } else { "composer-input" };
                    let paste = iced::clipboard::read().map(|t| match t {
                        Some(t) => {
                            UiMessage::ComposerEdit(text_editor::Action::Edit(text_editor::Edit::Paste(t.into())))
                        }
                        None => UiMessage::Noop,
                    });
                    return Task::batch([paste, operation::focus(id)]);
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::DiscardAttachment(i) => drop(self.pane.attachments.remove(i)),
            UiMessage::PickFiles if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            UiMessage::PickFiles => {
                let title = tr("Open").to_string();
                return Task::perform(
                    portal::open_files(portal::PORTAL.into(), title, portal::Pick::Files),
                    UiMessage::FilesPicked,
                );
            }
            UiMessage::QuickEmoji(Some(slot)) => {
                let open = self.open_emoji(None);
                self.pane.emoji.as_mut().expect("the picker is open").pref = Some(slot);
                return open;
            }
            UiMessage::QuickEmoji(None) => self.pane.emoji = None,
            UiMessage::SetQuickEmoji(0, e) => {
                self.patch_settings(|s| s.quick_reaction = e);
                let mut st = self.store.write();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::quick_reaction_write(&mut st).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed);
                }
            }
            UiMessage::SetQuickEmoji(slot, e) => {
                self.patch_settings(|s| s.quick_reactions[usize::from(slot) - 1] = e);
            }
            UiMessage::PickDownloads => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Folder);
                return Task::perform(pick, UiMessage::DownloadsPicked);
            }
            UiMessage::DownloadsPicked(Ok(dirs)) => {
                if let Some(dir) = dirs.into_iter().next() {
                    self.patch_settings(|s| s.downloads_dir = dir.display().to_string());
                }
            }
            UiMessage::DownloadsPicked(Err(e)) => self.fail(Toast::new(e)),
            UiMessage::FilesPicked(r) => match r {
                Ok(paths) => {
                    let st = self.store.read();
                    let chat = &st.chats[st
                        .resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat"))
                        .expect("chat exists")];
                    let (rules, resizes) = (chat.file.clone(), chat.room_features);
                    drop(st);
                    // Desktop's `attachFiles` logs the error of a file and shows "Couldn't attach %s". A file that
                    // `TBe` takes goes through the codec check of `_Be` here, away from the window.
                    let load = move || {
                        let load = |p: &std::path::PathBuf| {
                            let name = p.file_name().map_or(p.display().to_string(), |n| n.to_string_lossy().into());
                            let f = portal::load_file(p).map_err(|e| {
                                eprintln!("attachFiles: failed to attach {name}: {e}");
                                format!("Couldn't attach {name}")
                            })?;
                            match portal::check(&f, &rules, resizes) {
                                Ok(()) => portal::decodable(&f, p, &rules).map(|()| f),
                                Err(_) => Ok(f),
                            }
                        };
                        paths.iter().map(load).collect()
                    };
                    let load = async { tokio::task::spawn_blocking(load).await.expect("file load thread") };
                    return Task::perform(load, UiMessage::FilesLoaded);
                }
                Err(e) => self.status_banner = e,
            },
            UiMessage::FilesLoaded(_) if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            UiMessage::FilesLoaded(files) => {
                self.attach_files(files);
                if self.drop_send {
                    self.submit_composer(false);
                }
            }
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
                // Desktop's thread pane reads a new reply that comes into view at the bottom of the thread.
                let thread =
                    self.pane.thread.as_ref().filter(|t| t.thread_root.is_some() && t.thread_root == m.thread_root);
                if self.focused && thread.is_some_and(|t| t.scrolled_up.is_none_or(|(up, _)| !up)) {
                    st.mark_thread_read(&m.chat_id, m.thread_root.as_deref().expect("a thread reply"), None);
                }
                drop(st);
                if open.as_ref() == Some(&m.chat_id) {
                    self.read_open_chat();
                }
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
            UiMessage::Notified(Ok((id, note))) => _ = self.notifications.insert(id, note),
            // Like Desktop, a desktop without a notification server shows no notifications and no error.
            UiMessage::Notified(Err(e)) => eprintln!("notification: {e}"),
            UiMessage::NotificationEvent(id, action) => return self.notification_event(id, action),
            m @ (UiMessage::ToastHover(_) | UiMessage::ToastTick) if self.toast_clock.0 != self.status_banner => {
                self.toast_clock = (self.status_banner.clone(), self.toast_clock.1, Duration::ZERO);
                return self.update(m);
            }
            UiMessage::ToastHover(on) => self.toast_clock.1 = on,
            UiMessage::ToastTick => {
                for t in &mut self.older_toasts {
                    t.1 += TOAST_TICK;
                }
                self.older_toasts.retain(|t| t.1 < TOAST_TIMEOUT);
                if !self.newest_toast_running() {
                    return Task::none();
                }
                self.toast_clock.2 += TOAST_TICK;
                if self.toast_clock.2 >= if self.shown_note() { UNDO_NOTE_TIMEOUT } else { TOAST_TIMEOUT } {
                    return self.update(UiMessage::DismissBanner);
                }
            }
            // The second click of a double click on a close button can come after the first click removed the toast.
            UiMessage::DismissOlderToast(i) if i < self.older_toasts.len() => _ = self.older_toasts.remove(i),
            UiMessage::DismissOlderToast(_) => {}
            UiMessage::ToastAction(older, action) => {
                let _ = match older {
                    Some(i) => self.update(UiMessage::DismissOlderToast(i)),
                    None => self.update(UiMessage::DismissBanner),
                };
                return self.update(*action);
            }
            UiMessage::WindowFocused(focused) => {
                self.focused = focused;
                self.read_open_chat();
                let screen = self.login.screen;
                let clipboard = matches!(screen, login::Screen::RecoveryKey | login::Screen::Code);
                let key = (focused && clipboard && self.login_window.is_some())
                    .then(|| iced::clipboard::read().map(|t| LoginMsg::KeyOnFocus(t).into()));
                let modal = (focused && self.enter_key.window.is_some())
                    .then(|| iced::clipboard::read().map(|t| enter_key::KeyMsg::OnFocus(t).into()));
                return Task::batch([self.close_notifications()].into_iter().chain(key).chain(modal));
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
                if open && kind == Picker::Sticker {
                    return self.load_sticker_packs();
                }
                if open && kind == Picker::Gif {
                    self.pane.show_attach_menu = true;
                    let focus = operation::focus("gif-search-input");
                    if self.gifs.is_empty() && !self.gif_loading {
                        self.gif_loading = true;
                        self.gif_error = None;
                        return Task::batch([focus, self.update(UiMessage::GifSearch(self.gif_query.clone()))]);
                    }
                    return focus;
                }
            }
            UiMessage::GifQueryChanged(q) => {
                (self.gif_query, self.gif_loading, self.gif_error) = (q.clone(), true, None);
                self.gif_next.clear();
                if q.trim().is_empty() {
                    return self.update(UiMessage::GifSearch(q));
                }
                let wait = async { tokio::time::sleep(std::time::Duration::from_millis(500)).await };
                return Task::perform(wait, move |()| UiMessage::GifSearch(q.clone()));
            }
            UiMessage::GifSearch(q) if q == self.gif_query => {
                let reset = operation::scroll_to("gif-carousel", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
                return Task::batch([reset, gif_page(q, String::new())]);
            }
            UiMessage::GifSearch(_) => {}
            UiMessage::GifsLoaded(q, ..) if q != self.gif_query => {}
            UiMessage::GifsLoaded(_, more, r) => {
                self.gif_loading = false;
                match r {
                    // Desktop's `uniqBy(existingGifs.concat(page), "id")`.
                    Ok((page, next)) => {
                        if !more {
                            self.gifs.clear();
                        }
                        for g in page {
                            if self.gifs.iter().all(|x| x.id != g.id) {
                                self.gifs.push(g);
                            }
                        }
                        (self.gif_next, self.gif_error) = (next, None);
                        return self.load_media();
                    }
                    Err(e) => {
                        if !more {
                            self.gifs.clear();
                        }
                        self.gif_error = Some(e);
                    }
                }
            }
            // Desktop loads the next page when the carousel scrolls to 100 px from its end.
            UiMessage::GifScrolled(x, content, width) => {
                self.gif_view = (x, width);
                if x >= content - width - 100.0 && !self.gif_next.is_empty() && !self.gif_loading {
                    self.gif_loading = true;
                    return gif_page(self.gif_query.clone(), self.gif_next.clone());
                }
            }
            UiMessage::PickGif(g) => {
                self.pane.picker = None;
                return self.download_gif(g);
            }
            UiMessage::StickerPacks(r) => self.pane.stickers = Some(r),
            UiMessage::StickerTab(tab) => self.pane.sticker_tab = Some(tab),
            UiMessage::PickSticker(s) => return self.pick_sticker(s),
            UiMessage::StickerLoaded(r) => self.sticker_loaded(r),
            UiMessage::GifDownloaded(Ok(_)) if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            UiMessage::GifDownloaded(r) => match r {
                Ok(gif) => {
                    if !self.store.read().settings.on("autoSendGifs") {
                        self.pane.attachments.push(gif);
                        return Task::none();
                    }
                    // Desktop's `onSendAttachment`: only the GIF goes now. The composer keeps the rest.
                    let pane = &mut self.pane;
                    let text = std::mem::replace(&mut pane.composer, composer_with(""));
                    let files = std::mem::replace(&mut pane.attachments, vec![gif]);
                    let links = std::mem::take(&mut pane.composer_links);
                    let (later, reply) = (pane.send_later.take(), pane.reply_to_message_id.clone());
                    self.submit_composer(false);
                    let pane = &mut self.pane;
                    (pane.composer, pane.attachments, pane.composer_links) = (text, files, links);
                    (pane.send_later, pane.reply_to_message_id) = (later, reply);
                }
                Err(e) => {
                    let text = trf("An error occurred while downloading the GIF: %s", &[&e]);
                    self.fail(Toast::new(text));
                }
            },
            UiMessage::AnimationTick => {}
            UiMessage::Pushed(Ok(())) => {}
            UiMessage::DismissRecoveryReminder => {
                let now = beeper_core::config::now_unix_millis();
                self.patch_settings(|s| s.recovery_reminder_dismissed_at = now);
            }
            UiMessage::DismissBanner => {
                (self.status_banner, self.relaunch_toast) = (String::new(), None);
                self.toast_dismissals += 1;
            }
            UiMessage::Relaunch => {
                RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                return self.quit();
            }
            UiMessage::Pushed(Err(e)) => self.fail(Toast::new(format!("Could not send to Matrix: {e}"))),
            UiMessage::RetrySend(id) => self.store.write().retry_send(&id),
            UiMessage::RemoveUnsent(id) => self.store.write().remove_unsent(&id),
            UiMessage::TimelineScrolled(up) => {
                self.timeline_scrolled(up);
                self.read_open_chat();
            }
            UiMessage::ScrollToBottom => {
                return operation::scroll_to(self.pane.scroll_id(), operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            UiMessage::TopShown(shown) => {
                self.pane.top.shown = shown;
                return self.load_older();
            }
            UiMessage::LoadOlder => {
                self.pane.top.waiting = false;
                return self.load_older();
            }
            UiMessage::LoadedOlder(r) => return self.loaded_older(r),
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
                    self.fail(Toast::new(tr("Failed to clear storage").into()));
                }
                return self.scan_storage();
            }
            UiMessage::Integrations(msg) => return self.integrations(msg),
            UiMessage::JoinCommunity => {
                if self.store.read().resolve_chat_idx(COMMUNITY_ROOM).is_some() {
                    let close = window::close(self.settings_window.expect("the settings window is open"));
                    return Task::batch([self.update(UiMessage::SelectChat(COMMUNITY_ROOM.into())), close]);
                }
                if !self.integrations.joining {
                    self.integrations.joining = true;
                    return Task::perform(beeper_core::matrix::join(self.store.clone(), COMMUNITY_ROOM.into()), |r| {
                        UiMessage::CommunityJoined(r.map(drop))
                    });
                }
            }
            UiMessage::CommunityJoined(r) => {
                self.integrations.joining = false;
                match r {
                    Ok(()) => {
                        let text = tr("Successfully joined the Beeper community room!").into();
                        self.show(Toast { buttons: vec![(tr("Open"), UiMessage::JoinCommunity)], ..Toast::new(text) });
                    }
                    Err(e) => {
                        eprintln!("Failed to join community room: {e}");
                        self.fail(Toast::new(
                            tr("Something went wrong while joining the Beeper Developer Community chat. Please try again.")
                                .into(),
                        ));
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
                self.user_hover.shown = None;
                if self.show_command_palette
                    || self.forward.is_some()
                    || self.show_filter_menu
                    || self.chat_menu.is_some()
                    || self.pane.message_menu.is_some()
                    || self.pane.gallery_reactions.is_some()
                    || self.pane.thread_menu.is_some()
                    || self.unread_menu.is_some()
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
                    // Desktop unselects the gallery when its menu closes without an action.
                    if self.pane.message_menu.take().is_some() && !self.pane.gallery_menu.is_empty() {
                        self.pane.selected_msgs.clear();
                    }
                    self.pane.gallery_reactions = None;
                    self.pane.thread_menu = None;
                    self.unread_menu = None;
                    self.pane.edit_menu = None;
                    self.pane.sched_menu = None;
                    self.reminder_menu = None;
                    self.audio.menu = None;
                    self.selection_menu = None;
                    self.pane.show_attach_menu = false;
                    self.pane.picker = None;
                    self.pane.emoji = None;
                    self.quick_look_target = None;
                } else if self.select_mode || !self.selection.is_empty() {
                    (self.selection, self.select_mode) = (Vec::new(), false);
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
                } else if self.pane.thread_root.is_some() {
                    // Desktop's `onEscapePressed` closes the thread window.
                    self.pane.selected_chat_id = None;
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
            UiMessage::ToggleClamp(mid) => {
                if !self.pane.unclamped.remove(&mid) {
                    self.pane.unclamped.insert(mid);
                }
            }
            UiMessage::ToggleEditHistory(mid) => {
                self.pane.expanded_history_msg_id =
                    (self.pane.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            UiMessage::Noop => {}
            UiMessage::Relist => return Task::none(),
            UiMessage::LoadWanted => return Task::batch([self.load_avatars(), self.load_media(), self.load_quotes()]),
            UiMessage::QuoteLoaded(id, r) => _ = self.quotes.insert(id, Some(r.map(|m| *m))),
            UiMessage::DismissAccountError(id) => _ = self.dismissed_errors.insert(id),
            UiMessage::OpenUrl(url) if url.starts_with(SPOILER) => {
                self.pane.spoilers.insert(url[SPOILER.len()..].to_string());
            }
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
            UiMessage::OpenSettings(tab) => {
                self.chat_menu = None;
                return self.open_settings(tab);
            }
            UiMessage::ToggleAddAccount => self.adding_account ^= true,
            UiMessage::ShowAppQr => self.app_qr = true,
            UiMessage::OpenAccountSettings(id) => {
                self.chat_menu = None;
                let open = self.open_settings(SettingsTab::Accounts);
                self.account_page = Some(id);
                return open;
            }
            UiMessage::OpenAccount(id) => {
                self.account_page = id;
                return operation::scroll_to("settings-pane", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            UiMessage::AskRemoveAccount(id) => {
                (self.chat_menu, self.delete_target, self.confirm) = (None, Some(id), Some(ASK_REMOVE_ACCOUNT));
            }
            UiMessage::RemoveAccount(id) => {
                self.confirm = None;
                let p = self.provision(&id);
                let st = self.store.read();
                let sole = st.bridge_accounts.iter().filter(|b| b.bridge_id == p.bridge_id).all(|b| b.account_id == id);
                let removing = beeper_core::matrix::remove_account(p, st.config.api_url.clone(), sole);
                return Task::perform(removing, move |r| UiMessage::AccountRemoved(id.clone(), r));
            }
            UiMessage::AccountRemoved(_, Ok(())) => return self.refresh_cloud_accounts(),
            UiMessage::AccountRemoved(id, Err(e)) => {
                let text = format!("There was an error disconnecting account {id}. {e}");
                let report = UiMessage::OpenUrl(ISSUES_URL.into());
                self.show(Toast { buttons: vec![("Report a problem", report)], ..Toast::new(text) });
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
            UiMessage::ToggleIncognito => {
                let on = !self.store.read().settings.on("incognito");
                let task = self.update(UiMessage::SetPref("incognito", u8::from(on)));
                let typing = self.store.read().settings.on("typingIndicators");
                let (text, action) = match (on, typing) {
                    (true, true) => ("Incognito mode on. Also hide when you're typing from recipients?", "Hide typing"),
                    (false, false) => {
                        ("Incognito mode off. Also show when you're typing to recipients?", "Show typing")
                    }
                    (true, false) => ("Incognito mode on.", ""),
                    (false, true) => ("Incognito mode off.", ""),
                };
                let undo = (tr("Undo"), UiMessage::UndoAction);
                let typing = UiMessage::Commands(vec![
                    UiMessage::DismissBanner,
                    UiMessage::SetPref("typingIndicators", u8::from(!typing)),
                ]);
                let (buttons, right) =
                    if action.is_empty() { (Vec::new(), Some(undo)) } else { (vec![(tr(action), typing), undo], None) };
                self.show(Toast { buttons, right, ..Toast::new(tr(text).into()) });
                return task;
            }
            UiMessage::SetPref(key, v) => {
                if key == "incognito" {
                    let was = u8::from(self.store.read().settings.on(key));
                    let what = if v != 0 { tr("Enable Incognito Mode") } else { tr("Disable Incognito Mode") };
                    self.act(what.into(), UiMessage::SetPref(key, was), None);
                }
                self.patch_settings(|s| s.set_pref(key, v));
                self.relaunch_toast = match key {
                    "spellcheck" | "quitOnClose" | "hwAcceleration" | "hiddenEvents" => Some("Preference changed"),
                    "appLanguage" => Some("App language will change after relaunch"),
                    _ => self.relaunch_toast,
                };
                let mut st = self.store.write();
                let client = st.matrix.clone();
                if let Some(((kind, content), c)) = beeper_core::matrix::pref_write(&mut st, key).zip(client) {
                    return Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed);
                }
            }
            UiMessage::ToggleHotkey(name) => {
                self.patch_settings(|s| {
                    match s.hotkey_keys(name) {
                        "disabled" if prefs::hotkey_default(name) == "disabled" => {
                            s.keybindings.insert(name.into(), "".into())
                        }
                        "disabled" => s.keybindings.remove(name),
                        _ => s.keybindings.insert(name.into(), "disabled".into()),
                    };
                });
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            UiMessage::RecordHotkey(name) => {
                if self.hotkey_rec.as_ref().is_none_or(|r| r.name != name) {
                    self.hotkey_rec = Some(HotkeyRec { name, mods: [false; 3], key: None, buttons: false });
                }
            }
            UiMessage::HotkeyRecorder(false) => self.hotkey_rec = None,
            UiMessage::HotkeyRecorder(true) => self.save_hotkey(),
            UiMessage::RevertHotkey(name) => {
                self.patch_settings(|s| drop(s.keybindings.remove(name)));
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            UiMessage::RevertHotkeys => {
                self.patch_settings(|s| s.keybindings.clear());
                return self.update(UiMessage::Relaunch);
            }
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
            UiMessage::ResetSidebarWidth => self.sidebar_width = SIDEBAR_WIDTH,
            UiMessage::FloatRail(on) => self.rail_float = on,
            UiMessage::HoverUser(key) => {
                let h = &mut self.user_hover;
                match key {
                    Some(k) if h.key != k => {
                        *h = UserHover { key: k, trigger: true, r#gen: h.r#gen, ..UserHover::default() }
                    }
                    Some(_) => h.trigger = true,
                    None => h.trigger = false,
                }
                return self.settle_user_hover();
            }
            UiMessage::HoverUserCard(on) => {
                self.user_hover.card = on;
                return self.settle_user_hover();
            }
            UiMessage::HoverUserDue(n) if n == self.user_hover.r#gen => {
                let h = &mut self.user_hover;
                h.shown = (h.trigger || h.card).then(|| h.shown.unwrap_or(self.cursor.get()));
            }
            UiMessage::HoverUserDue(_) => {}
            UiMessage::DragMoved(pt) => {
                let width = pt.x - 68.0 - 2.0;
                if width >= 172.0 {
                    self.sidebar_width = width.min(732.0);
                }
                self.set_sidebar_hidden(width < 172.0);
            }
            UiMessage::RailHover(id, at) => {
                if let Some(order) = self.rail_drag.as_mut().filter(|d| d.id != id).and_then(|d| {
                    let order = d.order.as_mut()?;
                    let from = order.iter().position(|o| *o == d.id).expect("the dragged space");
                    Some((order, from))
                }) {
                    let (order, from) = order;
                    let to = order.iter().position(|o| *o == id).expect("a rail space");
                    let item = order.remove(from);
                    order.insert(to, item);
                }
                self.rail_hover = Some((id, at));
            }
            UiMessage::RailLeave(id) => {
                if self.rail_hover.as_ref().is_some_and(|h| h.0 == id) {
                    self.rail_hover = None;
                }
            }
            UiMessage::PinHover(id) => {
                // Desktop's sortable grid moves the dragged pin to the place of the pin under the pointer.
                let over = id.as_ref().and_then(|over| {
                    let d = self.pin_drag.as_mut().filter(|d| d.id != *over)?;
                    Some((over, d.order.as_mut()?, &d.id))
                });
                if let Some((over, order, held)) = over {
                    let from = order.iter().position(|o| o == held).expect("the dragged pin");
                    let to = order.iter().position(|o| o == over).expect("a pin of the grid");
                    let pin = order.remove(from);
                    order.insert(to, pin);
                }
                self.pin_hover = id;
            }
            UiMessage::PinPress => {
                let Some(id) = self.pin_hover.clone() else { return Task::none() };
                self.pin_drag = Some(PinDrag { id, from: None, order: None });
            }
            UiMessage::PinDragMoved(at) => {
                let Some(d) = self.pin_drag.as_mut() else { return Task::none() };
                if d.order.is_none() && at.distance(*d.from.get_or_insert(at)) > 5.0 {
                    let st = self.store.read();
                    let order = self.pins_and_rows(&st).0.iter().map(|c| c.id.clone()).collect();
                    drop(st);
                    self.pin_drag.as_mut().expect("a pin drag").order = Some(order);
                    self.chat_menu = None;
                }
            }
            UiMessage::PinDragEnd => {
                let Some(shown) = self.pin_drag.take().and_then(|d| d.order) else { return Task::none() };
                let order = self.store.write().set_pin_order(&shown);
                let Some(client) = self.store.read().matrix.clone() else { return Task::none() };
                let content = serde_json::json!({ "Inbox": order });
                let write = beeper_core::matrix::set_global(client, "com.beeper.favourites_order", content);
                return Task::perform(write, UiMessage::Pushed);
            }
            UiMessage::HeaderHover(on) => {
                let h = &mut self.header_hover;
                h.1 += 1;
                if on {
                    h.0 = true;
                } else {
                    let n = h.1;
                    let wait = async { tokio::time::sleep(std::time::Duration::from_millis(500)).await };
                    return Task::perform(wait, move |()| UiMessage::HeaderHoverEnd(n));
                }
            }
            UiMessage::HeaderHoverEnd(n) if n == self.header_hover.1 => self.header_hover.0 = false,
            UiMessage::HeaderHoverEnd(_) => {}
            UiMessage::MenuBar(state) => self.menu_bar = state,
            UiMessage::MenuBarHover(i) => {
                if let Some(Some(_)) = self.menu_bar {
                    self.menu_bar = Some(Some(i));
                }
            }
            UiMessage::MenuBarRun(msg) => {
                self.menu_bar = None;
                return self.update(*msg);
            }
            UiMessage::ComposerClip(cut) => {
                let Some(text) = self.pane.composer.selection() else { return Task::none() };
                let copy = iced::clipboard::write(text);
                if !cut {
                    return copy;
                }
                let delete = self.update(UiMessage::ComposerEdit(text_editor::Action::Edit(text_editor::Edit::Delete)));
                return Task::batch([copy, delete]);
            }
            UiMessage::MinimizeWindow(id) => return window::minimize(id, true),
            UiMessage::RailPress => {
                let Some((id, grab)) = self.rail_hover.clone() else { return Task::none() };
                self.rail_drag = Some(RailDrag { id, grab, from: None, at: Point::ORIGIN, order: None });
            }
            UiMessage::RailDragMoved(at) => {
                let Some(d) = self.rail_drag.as_mut() else { return Task::none() };
                d.at = at;
                if d.order.is_none() && at.distance(*d.from.get_or_insert(at)) > 5.0 {
                    let st = self.store.read();
                    let draft = self.label_window.as_ref().filter(|w| w.label.is_none());
                    d.order = Some(draft.map_or_else(|| rail_spaces(&st), |w| draft_rail_spaces(&st, &w.name)));
                    self.chat_menu = None;
                }
            }
            UiMessage::RailDragEnd => {
                let Some(order) = self.rail_drag.take().and_then(|d| d.order) else { return Task::none() };
                let old = self.store.read().settings.spaces_order.clone();
                let order: Vec<String> =
                    order.into_iter().map(|id| if id == LABEL_DRAFT { LABEL_NUDGE.into() } else { id }).collect();
                let shown = order.iter().map(|id| SpaceOrder {
                    id: id.clone(),
                    visible: old.iter().find(|o| o.id == *id).is_none_or(|o| o.visible),
                });
                let rest = old.iter().filter(|o| !order.contains(&o.id)).cloned();
                let order = shown.chain(rest).collect();
                self.patch_settings(|s| s.spaces_order = order);
            }
            UiMessage::DragEnd => {
                self.dragging = false;
                let path = self.store.read().config.data_dir.join("sidebar-width");
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
                    if let Some(t) = self.pane.thread.as_mut() {
                        t.size = size;
                    }
                }
                if let Some(w) = self.chat_windows.iter_mut().find(|w| w.0 == id) {
                    w.1.size = size;
                }
                if let Some(w) = self.media_window.as_mut().filter(|w| w.id == id) {
                    w.size = size;
                }
                if self.settings_window == Some(id) {
                    self.settings_size = size;
                }
            }
            UiMessage::Tray(Ok(tray::Event::Up)) => self.tray_sent = (None, true),
            // Electron cannot pop up the tray menu on Linux, so Desktop's click on a focused window does nothing.
            UiMessage::Tray(Ok(tray::Event::Activate)) if !self.hidden && self.focused => {}
            UiMessage::Tray(Ok(tray::Event::Activate)) => return self.show_all(),
            UiMessage::Tray(Ok(tray::Event::Show)) => return self.set_hidden(false),
            UiMessage::Tray(Ok(tray::Event::Hide)) => return self.set_hidden(true),
            UiMessage::Tray(Ok(tray::Event::Quit)) => return self.quit(),
            UiMessage::Tray(Err(e)) | UiMessage::TrayFailed(e) => {
                eprintln!("tray icon failed: {e}");
                return self.set_hidden(false);
            }
            UiMessage::CloseRequested(id) if self.poll_windows.iter().any(|w| w.id == id) => {
                return self.poll_edit(id, PollEdit::Close);
            }
            // Desktop hides the main window on close, with or without the tray: a later launch shows it again.
            UiMessage::CloseRequested(id) => {
                let quit = self.store.read().settings.on("quitOnClose");
                // Desktop's window `close`: it hides the app unless Quit on Close is on, also while signed out.
                let primary = Some(id) == self.main_window || Some(id) == self.login_window;
                return match (primary, quit) {
                    (true, false) => self.set_hidden(true),
                    (true, true) => self.save_main_window().chain(window::close(id)),
                    _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => {
                        self.save_window(id, "media-viewer").chain(window::close(id))
                    }
                    _ if self.chat_windows.iter().any(|w| w.0 == id && w.1.thread_root.is_none()) => {
                        self.save_window(id, POP_OUT).chain(window::close(id))
                    }
                    (false, _) => window::close(id),
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
                // Desktop's `windowStateKeeper("pop-out-thread")` keeps the size of the last chat window.
                let size = self.saved_window_settings(POP_OUT, Size::new(800.0, 600.0)).size;
                let (id, open) = window::open(Self::window_settings(size));
                self.chat_windows.push((id, Pane { size, ..Pane::default() }));
                let near = self.cascade(id);
                let select = self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(cid))));
                self.save_subwindows();
                return Task::batch([open.discard(), near, select]);
            }
            UiMessage::WindowClosed(id) if self.add_accounts.iter().any(|a| a.id == id) => {
                return self.add_account_closed(id);
            }
            UiMessage::WindowClosed(id) => {
                if let Some(w) = self.chat_windows.iter().find(|w| w.0 == id) {
                    save_draft(&mut self.store.write(), &w.1);
                }
                self.chat_windows.retain(|w| w.0 != id);
                self.theme_windows.retain(|w| w.0 != id);
                self.labels_windows.retain(|w| w.0 != id);
                self.poll_windows.retain(|w| w.id != id);
                self.vote_windows.retain(|w| w.0 != id);
                self.label_window = self.label_window.take().filter(|d| d.id != id);
                if self.label_asking(id) {
                    self.confirm = None;
                }
                self.label_settings.retain(|w| w.id != id);
                self.spaces_window = self.spaces_window.take().filter(|d| d.id != id);
                self.group_window = self.group_window.take().filter(|d| d.id != id);
                self.blocked_window = self.blocked_window.filter(|w| *w != id);
                self.verify.window = self.verify.window.filter(|w| *w != id);
                // Desktop's `ReauthRequiredModal` closed without a code ends the logout of a device.
                if self.reauth.window.take_if(|w| *w == id).is_some() {
                    self.devices_busy = false;
                }
                self.enter_key.window = self.enter_key.window.filter(|w| *w != id);
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                } else if self.media_window.as_ref().is_some_and(|w| w.id == id) {
                    self.media_window = None;
                    self.carousel = self.carousel.take().filter(|c| !c.in_media);
                } else if [self.main_window, self.login_window].contains(&Some(id)) {
                    self.main_window = None;
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
            // A sign-in through the Desktop API goes on to the check of encrypted messaging, like a sign-in here.
            UiMessage::StoreChanged
                if self.login_window.is_some()
                    && !self.login.busy
                    && self.login.signing_in()
                    && self.store.read().signed_in() =>
            {
                return self.check_e2ee();
            }
            UiMessage::StoreChanged => {
                if let Some((room, quote)) =
                    self.pending_dm.take_if(|(r, _)| self.store.read().resolve_chat_idx(r).is_some())
                {
                    self.select_dm(&room, quote);
                }
                let st = self.store.read();
                let left = self.pane.selected_chat_id.as_deref().is_some_and(|c| st.resolve_chat_idx(c).is_none());
                self.selection.retain(|c| st.resolve_chat_idx(c).is_some());
                let gone = self.carousel.is_some() && self.carousel_items(&st).is_none();
                drop(st);
                if gone {
                    self.carousel = None;
                }
                if left {
                    (self.pane.selected_chat_id, self.chat_menu) = (None, None);
                    self.select_chat(NOTE_CHAT_ID);
                }
                self.refresh_theme_and_tray();
                let older = self.load_older_everywhere();
                return Task::batch([self.close_notifications(), self.verification_changed(), self.leave_app(), older]);
            }
            UiMessage::FocusRequested(chat) => {
                if let Some(id) = chat {
                    self.select_chat(&id);
                }
                let shown = if self.hidden { self.set_hidden(false) } else { Task::none() };
                let focus =
                    shown.chain(window::gain_focus(self.main_window.or(self.login_window).expect("a window is open")));
                return Task::batch([focus, self.load_avatars(), self.load_media(), self.close_notifications()]);
            }
            UiMessage::SecondInstance(args) => return self.second_instance(args),
            UiMessage::RoomJoined(r) => self.room_joined(r),
            UiMessage::Online(on) => self.online = on,
            UiMessage::PillHover(on) => self.pill_hover = on,
            UiMessage::Login(m) => {
                let screen = self.login.screen;
                let task = self.update_login(m);
                // Desktop's recovery key fields take the keyboard when their screen opens (`autoFocus`).
                let keyed = matches!(self.login.screen, login::Screen::RecoveryKey | login::Screen::ConfirmKey);
                return match keyed && self.login.screen != screen {
                    true => Task::batch([task, operation::focus(verify::KEY_INPUT)]),
                    false => task,
                };
            }
            UiMessage::Verify(m) => return self.update_verify(m),
            UiMessage::Reauth(m) => return self.update_reauth(m),
            UiMessage::EnterKey(m) => return self.update_enter_key(m),
            UiMessage::OpenAddAccount(network, login) => return self.open_add_account(&network, login),
            UiMessage::AddAccount(id, m) => return self.update_add_account(id, m),
            UiMessage::KeyPressed(Key::Named(k @ (Named::Enter | Named::Escape)), m)
                if (self.recorder.is_some() || self.talk.is_some()) && m.is_empty() =>
            {
                return self.record(record::Msg::Key(k));
            }
            UiMessage::KeyPressed(key, mods) if self.carousel.as_ref().is_some_and(|c| !c.in_media) => {
                return self.carousel_key(&key, mods);
            }
            UiMessage::KeyPressed(key, mods) => {
                let z = mods.command() && matches!(key.as_ref(), Key::Character(c) if c.eq_ignore_ascii_case("z"));
                if z && !mods.shift() && self.shown_undo().is_none() && !self.undo_stack.is_empty() {
                    return self.update(UiMessage::UndoAction);
                }
                if z && (self.queued_here() || self.shown_undo().is_some()) {
                    return self.update(UiMessage::Undo(mods.shift()));
                }
                if self.pane.picker == Some(Picker::Gif) && mods.is_empty() {
                    if let Some(task) = self.gif_key(&key) {
                        return task;
                    }
                }
                let n = self.suggestions(&self.store.read()).map_or(0, |(_, picks)| picks.len());
                if n > 0 && mods.is_empty() {
                    let at = self.pane.suggest.min(n - 1);
                    match key.as_ref() {
                        Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                            let down = key == Key::Named(Named::ArrowDown);
                            self.pane.suggest = if down { (at + 1) % n } else { (at + n - 1) % n };
                            return Task::none();
                        }
                        Key::Named(Named::Tab) => return self.update(UiMessage::PickSuggestion(at)),
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
                        self.command_list.reveal(cmds[self.cmd_index].key());
                        return locate(format!("cmd-{}", self.cmd_index), commands::COMMANDS, true);
                    }
                }
                match key.as_ref() {
                    Key::Named(Named::Escape) => return self.update(UiMessage::CloseModals),
                    Key::Named(Named::ArrowUp | Named::ArrowDown | Named::ArrowRight)
                        if (mods.is_empty() || mods == keyboard::Modifiers::SHIFT)
                            && self.pane.thread.is_some()
                            && self.pane.thread_focus =>
                    {
                        return self.in_thread(UiMessage::KeyPressed(key, mods));
                    }
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
                    Key::Named(Named::Enter)
                        if mods.is_empty() && self.creating_chat.is_some() && !self.join_matrix_mode =>
                    {
                        return self.update(UiMessage::StartChat);
                    }
                    Key::Named(Named::Backspace)
                        if self.creating_chat.is_some() && !self.join_matrix_mode && self.new_chat_query.is_empty() =>
                    {
                        let n = self.creating_chat.expect("an open draft");
                        self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").users.pop();
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
                        (self.selection, self.select_mode) = (self.list_ids(&self.store.read()), false);
                        if let Some(first) = self.selection.first().cloned() {
                            self.select_chat(&first);
                        }
                        return Task::none();
                    }
                    Key::Character("v") if mods.command() && self.pane.selected_chat_id.is_some() => {
                        return self.update(UiMessage::Paste);
                    }
                    _ => {}
                }
                if let Some((h, digit)) = self.find_hotkey(&key, mods) {
                    return self.run_hotkey(h, digit);
                }
            }
        }
        self.settle()
    }

    /// The end of a message that did not return its own task: shows a new Matrix error, and loads the avatars and
    /// the media that the views want.
    fn settle(&mut self) -> Task<UiMessage> {
        // A write lock empties the memo of the views, so take one only for a change.
        if self.store.read().matrix_error.is_some() {
            self.status_banner = self.store.write().matrix_error.take().expect("a Matrix error");
        }
        Task::batch([self.load_avatars(), self.load_media()])
    }

    /// Desktop's `hydrateMissingBookmarks` while Chat Info is open: fetches the bookmarked messages of the open chat
    /// that are not loaded.
    fn load_bookmarks(&mut self) -> Task<UiMessage> {
        let st = self.store.read();
        let (Some(client), Some(chat), true) =
            (st.matrix.clone(), self.pane.selected_chat_id.clone(), self.show_right_panel)
        else {
            return Task::none();
        };
        let missing = st.bookmarks.iter().filter(|(t, b)| b.room_id == chat && st.loaded(t).is_none());
        let missing: Vec<String> = missing
            .filter(|(t, _)| !st.bookmarked.contains_key(*t) && !self.hydrating.contains(*t))
            .map(|(t, _)| t.clone())
            .collect();
        let account = st.resolve_chat_idx(&chat).map(|i| st.chats[i].account_id.clone());
        drop(st);
        let Some(account) = account else { return Task::none() };
        self.hydrating.extend(missing.iter().cloned());
        Task::batch(missing.into_iter().map(|t| {
            let fetch =
                beeper_core::matrix::bookmarked_message(client.clone(), chat.clone(), t.clone(), account.clone());
            Task::perform(fetch, move |m| UiMessage::BookmarkHydrated(t.clone(), m.map(Box::new)))
        }))
    }

    fn patch_settings(&mut self, f: impl FnOnce(&mut BeeperSettings)) {
        self.store.write().update_settings(f);
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
            feed(f.0.read().connect_tx.subscribe(), |(id, outcome)| {
                UiMessage::Integrations(integrations::Msg::Request(id, outcome))
            })
        });
        let launches = Subscription::run_with(StoreFeed(self.store.clone(), 3), |f| {
            feed(f.0.read().launch_tx.subscribe(), UiMessage::SecondInstance)
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
            // A text field pastes itself. The composer sends `Paste` for Desktop's file paste.
            Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Character(c), modifiers, .. })
                if status == iced::event::Status::Captured && modifiers.command() && c.eq_ignore_ascii_case("v") =>
            {
                None
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
            Event::Keyboard(keyboard::Event::KeyPressed { text: Some(text), .. })
                if status == iced::event::Status::Ignored && !text.chars().any(char::is_control) =>
            {
                Some(UiMessage::WindowText(id, text.to_string()))
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
        let thread_drag = self.thread_drag.then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(UiMessage::ThreadDrag(Some(position))),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::ThreadDrag(None)),
                _ => None,
            })
        });
        let rail_press = (self.rail_hover.is_some() && self.rail_drag.is_none()).then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Some(UiMessage::RailPress),
                _ => None,
            })
        });
        let pin_press = (self.pin_hover.is_some() && self.pin_drag.is_none()).then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Some(UiMessage::PinPress),
                _ => None,
            })
        });
        let pin_drag = self.pin_drag.is_some().then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(UiMessage::PinDragMoved(position)),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::PinDragEnd),
                _ => None,
            })
        });
        let rail_drag = self.rail_drag.is_some().then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(UiMessage::RailDragMoved(position)),
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::RailDragEnd),
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
            let last = self.pane().selected_chat_id.as_ref().and_then(|c| st.timeline_indices(c).last());
            let rising = last.is_some_and(|&i| send_rise(&st, &st.messages[i]).is_some());
            let pending = st.chats.iter().filter(|c| !self.expired(c));
            let send = st.scheduled.iter().filter(|s| !s.canceled).map(|s| s.send_on);
            let send = send.chain(self.pane.send_later.map(|c| c.0)).min();
            (rising, (pending.filter_map(|c| Some(c.reminder.as_ref()?.remind_at_ms)).min(), send))
        };
        let wait = |at: i64| std::time::Duration::from_millis((at - now_unix_millis()).max(1) as u64);
        let late = {
            let st = self.store.read();
            let open = self.panes().filter_map(|p| p.selected_chat_id.as_ref()).map(|c| st.timeline_indices(c));
            let echoes = open.flatten().map(|&i| &st.messages[i]);
            let echoes = echoes.filter(|m| m.send_error.is_none() && pending(&st, m));
            echoes.map(still_sending_at).filter(|&at| at > now_unix_millis()).min()
        };
        // "Sending..." turns into "Still sending..." at its time.
        let late = late.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::AnimationTick));
        let remind = due.0.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::CheckReminders));
        let send = due.1.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::CheckScheduled));
        let typed = self.panes().filter_map(|p| p.typing.as_ref().map(|t| t.1)).min();
        let typing = typed.map(|t| {
            let left = TYPING_STOP.saturating_sub(t.elapsed()).max(Duration::from_millis(1));
            iced::time::every(left).map(|_| UiMessage::TypingTick)
        });
        let queued = self.queued.iter().map(|q| q.due).min();
        let queued = queued.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::SendQueued(None)));
        // The loading of older messages and the fade of the scroll button, in each chat window too.
        let moving =
            |p: &Pane| p.top.loading.is_some() || p.scrolled_up.is_some_and(|s| s.1.elapsed().as_secs_f32() < 0.2);
        let animated = rising
            || self.panes().any(moving)
            || self.flash.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 1.5)
            || self.pref_flash.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 2.0)
            || (self.login_window.is_some() && (self.login.busy || self.login.animated()))
            || self.reauth.busy
            || self.enter_key.busy;
        let tab_at = self.media_window.as_ref().and_then(|w| w.tab_at);
        let animated = animated || tab_at.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < TAB_SLIDE);
        let tray = (cfg!(target_os = "linux") && self.store.read().settings.on("showTrayIcon"))
            .then(|| Subscription::run(tray::events).map(UiMessage::Tray));
        let online = cfg!(target_os = "linux").then(|| Subscription::run(portal::online).map(UiMessage::Online));
        let notes = (!self.notifications.is_empty() || self.verify.note.is_some())
            .then(|| Subscription::run(notify::events).map(|(id, action)| UiMessage::NotificationEvent(id, action)));
        let toast = self.toast_running().then(|| iced::time::every(TOAST_TICK).map(|_| UiMessage::ToastTick));
        let tick =
            animated.then(|| iced::time::every(std::time::Duration::from_millis(20)).map(|_| UiMessage::AnimationTick));
        let due = self.panes().any(|p| p.draft_due);
        let drafts = due.then(|| iced::time::every(std::time::Duration::from_secs(5)).map(|_| UiMessage::SaveDrafts));
        let shot = self.shot.map(|_| {
            iced::event::listen_raw(|e, _, id| {
                matches!(e, Event::Window(window::Event::RedrawRequested(_))).then_some(UiMessage::Drawn(id))
            })
        });
        Subscription::batch(
            [changes, focus, tokens, launches, keys, system]
                .into_iter()
                .chain(drag)
                .chain(thread_drag)
                .chain(rail_press)
                .chain(rail_drag)
                .chain(pin_press)
                .chain(pin_drag)
                .chain(poll_drag)
                .chain(carousel)
                .chain(tick)
                .chain(toast)
                .chain(shot)
                .chain(remind)
                .chain(late)
                .chain(send)
                .chain(queued)
                .chain(typing)
                .chain(drafts)
                .chain(notes)
                .chain(tray)
                .chain(online)
                .chain(self.audio_ticks())
                .chain(self.transcribe_ticks())
                .chain(self.audio_drag())
                .chain(self.record_events())
                .chain(self.video_ticks()),
        )
    }

    pub fn view(&self, id: window::Id) -> El<'_> {
        let arrow = !self.store.read().settings.on("pointerCursor");
        self.drawing.set(self.chat_windows.iter().any(|w| w.0 == id).then_some(id));
        let el = cursor::root(self.view_window(id), arrow, &self.cursor);
        self.drawing.set(None);
        let el = match self.chat_windows.iter().any(|w| w.0 == id) {
            true => Element::from(el).map(move |m| UiMessage::InWindow(id, Box::new(m))),
            false => el.into(),
        };
        self.wants.watch(el, UiMessage::LoadWanted)
    }

    fn view_window(&self, id: window::Id) -> El<'_> {
        if Some(id) == self.settings_window && !self.store.read().signed_in() {
            // Desktop's `InnerApp` draws nothing in a secondary window before sign-in.
            Space::new().into()
        } else if Some(id) == self.settings_window {
            self.view_settings()
        } else if let Some(w) = self.media_window.as_ref().filter(|w| w.id == id) {
            self.view_media(w)
        } else if Some(id) == self.login_window {
            self.view_login()
        } else if Some(id) == self.verify.window {
            self.view_verify()
        } else if Some(id) == self.reauth.window {
            self.view_reauth()
        } else if Some(id) == self.enter_key.window {
            self.view_enter_key()
        } else if let Some(a) = self.add_accounts.iter().find(|a| a.id == id) {
            self.view_add_account(a)
        } else if let Some((_, chat)) = self.theme_windows.iter().find(|w| w.0 == id) {
            self.view_chat_theme(chat)
        } else if let Some((_, chat)) = self.labels_windows.iter().find(|w| w.0 == id) {
            self.view_edit_labels(chat)
        } else if let Some(d) = self.poll_windows.iter().find(|w| w.id == id) {
            self.view_poll_creator(d)
        } else if let Some(d) = self.label_window.as_ref().filter(|d| d.id == id) {
            self.view_create_label(d)
        } else if let Some(w) = self.label_settings.iter().find(|w| w.id == id) {
            self.view_label_settings(w)
        } else if let Some(d) = self.spaces_window.as_ref().filter(|d| d.id == id) {
            self.view_edit_spaces(d)
        } else if let Some(d) = self.group_window.as_ref().filter(|d| d.id == id) {
            self.view_edit_group(d)
        } else if Some(id) == self.blocked_window {
            self.view_blocked_users()
        } else if let Some((_, chat, mid)) = self.vote_windows.iter().find(|w| w.0 == id) {
            self.view_poll_votes(chat, mid)
        } else if self.chat_windows.iter().any(|w| w.0 == id && w.1.selected_chat_id.is_none()) {
            // A chat window that left its chat closes, and draws nothing until it is closed.
            Space::new().into()
        } else {
            self.view_main()
        }
    }

    /// The rail, the chat list and the open chat.
    fn view_body(&self, open: bool) -> El<'_> {
        let p = &self.pal;
        let grip = mouse_area(container(Space::new()).width(5).height(Fill))
            .on_press(UiMessage::DragStart)
            .on_double_click(UiMessage::ResetSidebarWidth)
            .interaction(mouse::Interaction::ResizingHorizontally);
        let thread = open && self.creating_chat.is_none() && self.selection.is_empty() && self.pane().thread.is_some();
        let pane = match self.creating_chat {
            Some(_) => self.view_new_chat(),
            None if open && !thread => self.view_conversation(),
            None => container(Space::new()).width(Fill).height(Fill).style(fill_box(p.card, 10.0)).into(),
        };
        let info = (self.show_right_panel && open && !thread).then(|| self.view_info());
        let pad = Padding::new(8.0).left(if self.sidebar_hidden { 8.0 } else { 0.0 });
        let right: El<'_> = if self.unread_view && !open && self.creating_chat.is_none() {
            container(self.view_unread_threads()).padding(pad).into()
        } else if thread {
            iced::widget::responsive(move |size| self.view_split(size.width, pad)).into()
        } else if self.selection.is_empty() {
            row![container(pane).padding(pad).width(Fill)].push(info).into()
        } else {
            let content = container(row![pane].push(info)).padding(8).style(fill_box(p.selected, 18.0));
            column![self.view_selection_head(), content, self.view_selection_bar()].spacing(8).padding(pad).into()
        };
        let shown = self.store.read().settings.on("showSidebar");
        let rail = (shown && !self.sidebar_hidden).then(|| self.view_rail());
        let sidebar = (!self.sidebar_hidden).then(|| self.view_sidebar());
        let body = Row::new().push(rail).push(sidebar).push(stack![right, grip]);
        let body = container(body).width(Fill).height(Fill).style(fill_box(p.window, 0.0));
        // Desktop's `hoveringTrigger`: with the space bar hidden, the 20px at the left edge slide it in over the list.
        let float = (!shown && !self.sidebar_hidden).then(|| -> El<'_> {
            match self.rail_float {
                true => {
                    let rail = container(self.view_rail()).height(Fill).style(|_: &Theme| container::Style {
                        shadow: Shadow {
                            color: Color::BLACK.scale_alpha(0.2),
                            offset: Vector::ZERO,
                            blur_radius: 16.0,
                        },
                        ..Default::default()
                    });
                    // Over the space bar, so its buttons do not hide the pointer from the area.
                    let watch = mouse_area(Space::new().width(68).height(Fill)).on_exit(UiMessage::FloatRail(false));
                    stack![rail, watch].into()
                }
                false => mouse_area(Space::new().width(20).height(Fill)).on_enter(UiMessage::FloatRail(true)).into(),
            }
        });
        stack![body].push(float).into()
    }

    pub fn view_main<'a>(&'a self) -> El<'a> {
        let st = self.store.read();
        // The sync task can drop a chat before StoreChanged reaches update.
        let known = |c: &str| {
            let chat = c.strip_prefix("info:").or(c.strip_prefix("copy:")).unwrap_or(c);
            let chat = chat.split_once('\n').map_or(chat, |k| k.0);
            let pair = ["member:", "bookmark:", "contact:", "card:"].iter().find_map(|k| chat.strip_prefix(k));
            let chat = pair.map_or(chat, |k| k.split_once(' ').map_or(k, |k| k.0));
            c == RAIL_MORE
                || c == SIDEBAR_MENU
                || c == SPACE_SHEET
                || c == SWITCH_CARD
                || c.starts_with("draft:")
                || c.starts_with("account:")
                || c.starts_with("head:")
                || st.resolve_chat_idx(chat).is_some()
        };
        let shown = self.pane().selected_chat_id.as_deref().is_some_and(known);
        let open = self.creating_chat.is_none() && shown;
        let menu_open = self.chat_menu.as_ref().is_some_and(|(c, _)| known(c));
        drop(st);
        let in_main = self.drawing.get().is_none();
        let base = match in_main {
            true => self.view_full_video(false).unwrap_or_else(|| self.view_body(open)),
            false => container(match shown {
                true if self.pane().thread_root.is_some() => self.view_thread_window(),
                true => self.view_conversation(),
                false => Space::new().into(),
            })
            .padding(8)
            .width(Fill)
            .height(Fill)
            .style(fill_box(self.pal.window, 0.0))
            .into(),
        };
        // Desktop's menu bar, which Alt shows over the content of the main window.
        let base = match self.menu_bar.filter(|_| in_main) {
            Some(open) => column![self.view_menu_bar(open), base].into(),
            None => base,
        };
        let menu_drop = self.menu_bar.flatten().filter(|_| in_main).map(|i| {
            let (menu, x) = self.view_menu_dropdown(i);
            let at = Padding::ZERO.left(x).top(menubar::BAR_H);
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MenuBar(None))
        });
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
        let rate_menu = self.audio.menu.map(|at| match self.audio.pill {
            true => {
                let size = self.pane().size;
                let x = at.x.min(size.width - 233.0 - voice::rate_menu_w() - 8.0);
                let y = at.y.min(size.height - 8.0 - MENU_ITEM - 8.0);
                let top = if self.audio.sub { y.min(size.height - 8.0 - 8.0 * MENU_ITEM - 8.0) } else { y };
                (self.view_pill_menu(y - top), x, top)
            }
            false => (self.view_rate_menu(), at.x, at.y),
        });
        let chat_menu = self.chat_menu.as_ref().filter(|_| menu_open).map(|(cid, at)| {
            if let Some(key) = cid.strip_prefix("card:") {
                let (card, h) = self.view_user_card(key);
                let (x, y) = self.card_spot(*at, 30.0, h);
                return (card, x, y);
            }
            if cid == SWITCH_CARD {
                let (card, h) = self.view_switch_card();
                return (card, at.x - 8.0, at.y - h + 8.0);
            }
            if cid == SPACE_SHEET {
                let (sheet, w, h) = self.view_space_sheet();
                let x = if at.x + w > self.pane().size.width { at.x - w } else { at.x };
                return (sheet, x, at.y.min(self.pane().size.height - h - 8.0));
            }
            let (w, h) = if cid.starts_with("draft:") {
                (124.0, 31.0)
            } else if cid == RAIL_MORE {
                let items = more_menu(&self.store.read());
                (233.0, 8.0 + items.iter().map(|i| if i.is_some() { MENU_ITEM } else { 11.0 }).sum::<f32>())
            } else if INFO_MENUS.iter().any(|k| cid.starts_with(k)) {
                let items = info_menu(&self.store.read(), cid, |a| self.can(a, "create_dm"));
                (233.0, 8.0 + items.iter().map(|i| if i.is_some() { MENU_ITEM } else { 11.0 }).sum::<f32>())
            } else if cid == SIDEBAR_MENU || RAIL_VIEWS.iter().any(|v| cid.strip_prefix("account:") == Some(v.0)) {
                (233.0, 8.0 + 4.0 * MENU_ITEM)
            } else if let Some(entry) = cid.strip_prefix("head:") {
                let st = self.store.read();
                let items = match (RAIL_VIEWS.iter().any(|v| v.0 == entry), rail_label(&st, entry)) {
                    (true, _) => 1.0,
                    (false, Some(l)) => label_rows(l) + 1.0,
                    (false, None) => 4.0,
                };
                let rule = if items > 1.0 { 11.0 } else { 0.0 };
                (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + items * MENU_ITEM + rule)
            } else if let Some(n) =
                cid.strip_prefix("account:").and_then(|e| rail_label(&self.store.read(), e).map(label_rows))
            {
                (233.0, 8.0 + (n + 2.0) * MENU_ITEM + 11.0)
            } else if cid.starts_with("account:") {
                (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + 6.0 * MENU_ITEM + 22.0)
            } else {
                let st = self.store.read();
                (
                    233.0,
                    8.0 + 10.0 * MENU_ITEM
                        + 44.0
                        + if self.selection.is_empty() { 2.0 * MENU_ITEM + 11.0 } else { 0.0 }
                        + if st.feature("labels-v2").on { MENU_ITEM } else { 0.0 }
                        + self.selection_rows(&st)
                        + match st.resolve_chat_idx(cid).map(|i| &st.chats[i]) {
                            Some(c)
                                if self.selection.is_empty()
                                    && st.notifications_muted(c)
                                    && muted_by(&st, c).is_some() =>
                            {
                                MENU_ITEM
                            }
                            _ => 0.0,
                        }
                        + if st.resolve_chat_idx(cid).is_some_and(|i| can_edit_group(&st.chats[i])) {
                            MENU_ITEM
                        } else {
                            0.0
                        }
                        + if self.selection.len() <= 1
                            && st.resolve_chat_idx(cid).is_some_and(|i| st.threaded(&st.chats[i]))
                        {
                            MENU_ITEM
                        } else {
                            0.0
                        }
                        + match st.resolve_chat_idx(cid).map(|i| request_answers(&st.chats[i])) {
                            Some((false, false)) | None => 0.0,
                            Some((a, d)) => f32::from(u8::from(a) + u8::from(d)) * MENU_ITEM + 11.0,
                        },
                )
            };
            let x = if at.x + w > self.pane().size.width { at.x - w } else { at.x };
            let sub_w = self
                .chat_sub
                .filter(|_| !cid.starts_with("account:") && !cid.starts_with("head:"))
                .map_or(0.0, |i| SUB_MENU_W[usize::from(i)]);
            let left = x + w + sub_w > self.pane().size.width;
            let y = at.y.min(self.pane().size.height - h - 8.0);
            (self.view_chat_menu(cid, left, self.pane().size.height - 8.0 - y), if left { x - sub_w } else { x }, y)
        });
        let selection_menu = self.selection_menu.map(|(i, x)| {
            let (menu, h) = self.view_selection_menu(i);
            let w = if i == 0 { SUB_MENU_W[1] } else { 233.0 };
            (menu, x.min(self.pane().size.width - w - 8.0), self.pane().size.height - 60.0 - h)
        });
        let main_menu = filter_menu.or(reminder_menu).or(rate_menu).or(chat_menu).filter(|_| in_main);
        let layer = |(m, x, y): (El<'a>, f32, f32), close: UiMessage| -> El<'a> {
            mouse_area(
                container(mouse_area(m).on_press(UiMessage::Noop))
                    .padding(Padding::ZERO.left(x).top(y))
                    .width(Fill)
                    .height(Fill),
            )
            .on_press(close.clone())
            .on_right_press(close)
            .into()
        };
        let main_menu = main_menu.or(self.pane_menu()).or(selection_menu.filter(|_| in_main));
        let (menu, palette_menu) =
            match (self.show_command_palette || self.forward.is_some()) && self.chat_menu.is_some() {
                true => (None, main_menu.map(|m| layer(m, UiMessage::CloseChatMenu))),
                false => (main_menu.map(|m| layer(m, UiMessage::CloseModals)), None),
            };
        // The menus of the thread pane, whose messages the thread pane handles.
        let menu = menu.or_else(|| {
            let open = self.pane().thread.is_some();
            self.drawing_thread.set(open);
            let thread = open.then(|| self.pane_menu()).flatten();
            self.drawing_thread.set(false);
            let thread = thread.map(|(m, x, y)| (Element::from(m).map(|m| UiMessage::InThread(Box::new(m))), x, y));
            thread.map(|m| layer(m, UiMessage::InThread(Box::new(UiMessage::CloseModals))))
        });
        let menu =
            menu.or_else(|| Some(layer(self.view_thread_menu(self.pane().thread_menu?), UiMessage::CloseModals)));
        let menu = menu.or_else(|| Some(layer(self.view_unread_menu(self.unread_menu?), UiMessage::CloseModals)));
        let toast = self.view_toast(true);
        let elsewhere = self.delete_in.is_some_and(|w| self.label_asking(w));
        let confirm = self.confirm.filter(|d| !SETTINGS_DIALOGS.contains(d) && !elsewhere);
        let confirm = confirm.map(|d| self.view_confirm(d));
        let shows = |c: &&Carousel| {
            let st = self.store.read();
            let chat = st.loaded(&c.mid).map(|m| &m.chat_id);
            !c.in_media && chat.is_some() && chat == self.pane().selected_chat_id.as_ref()
        };
        let carousel = self.carousel.as_ref().filter(shows).and_then(|c| self.view_carousel(c));
        let hotkeys = self.hotkeys_modal.as_deref().filter(|_| in_main).map(|q| self.view_hotkeys_modal(q));
        // The menus and the emoji picker of the carousel show over it.
        let hover = self.view_hover_card().filter(|_| in_main);
        let layers = stack![base]
            .push(toast)
            .push(carousel)
            .push(hover)
            .push(menu_drop)
            .push(menu)
            .push(scrim)
            .push(palette_menu)
            .push(select);
        let token = self.token_ask.as_ref().map(|a| self.view_token_ask(a));
        layers.push(confirm).push(hotkeys).push(token).into()
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
                        .color(p.text)
                ]
                .spacing(8);
                let notice = container(notice).width(Fill).padding([8, 16]).style(fill_box(p.chip, 8.0));
                column![notice, row![button("Close", false, UiMessage::Integrations(integrations::Msg::Close))]]
                    .spacing(16)
                    .into()
            }
            false => {
                let label = |t: &'static str| txt(tr(t), 12.0).color(p.text);
                let sensitive = column![
                    row![
                        label("Allow sensitive actions").width(Fill),
                        toggle(p, a.write, Some(UiMessage::Integrations(integrations::Msg::Write)))
                    ]
                    .align_y(Center)
                    .height(38)
                    .padding([0, 8]),
                    container(txt(tr("Grant permission to send messages and modify your account"), 11.0).color(p.sub))
                        .padding(Padding::new(8.0).top(0.0))
                ];
                let expiry = select(p, EXPIRIES, a.expiry, |a0| UiMessage::Integrations(integrations::Msg::Expiry(a0)));
                let expiry = row![label("Expires in").width(Fill), expiry].align_y(Center).height(38).padding([0, 8]);
                let rows = column![sensitive, divider(p.prefs_border, 0.0), expiry, divider(p.prefs_border, 0.0)];
                let group =
                    container(rows).width(Fill).padding(1).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
                let buttons = row![
                    button("Decline", false, UiMessage::Integrations(integrations::Msg::Answer(false))),
                    button("Approve", true, UiMessage::Integrations(integrations::Msg::Answer(true)))
                ]
                .spacing(8);
                let fine = txt(tr("Only approve if you initiated this connection and trust the application."), 12.0)
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
        let title = container(semi(tr("Approve connection"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let content = column![container(main).padding(Padding::new(0.0).top(32.0).bottom(16.0)), body];
        let pane = column![title, container(content).padding([0, 16])];
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.prefs_pane, 10.0));
        mouse_area(container(pane).padding(8)).on_press(UiMessage::Noop).into()
    }

    /// The selected chats that still exist: the sync task can drop one before StoreChanged reaches update.
    /// The chat menu rows that a selection of more chats adds: the count, and both Mark All items.
    fn selection_rows(&self, st: &BeeperStore) -> f32 {
        let picked = self.selected_chats(st);
        let unread = picked.iter().filter(|c| has_flag(c, ChatFlag::MarkedUnread)).count();
        match picked.len() {
            0 | 1 => 0.0,
            n if (1..n).contains(&unread) => 2.0 * MENU_ITEM,
            _ => MENU_ITEM,
        }
    }

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
        let x0 = self.left_width() + 8.0;
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
                [trx("Mute", "thread context menu action"), trx("Unmute", "thread context menu action")],
                [trx("Mute", "thread context menu action"), trx("Unmute", "thread context menu action")],
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
            let entry = |label: &str, keys: &str, f: ChatFlag, on: bool| {
                item(label.into(), keys.into(), Some(UiMessage::SelectionSet(f, on)))
            };
            let pinned = count(ChatFlag::Pinned);
            [Some(item(trn("%d chat selected", "%d chats selected", n, &[&n]), String::new(), None))]
                .into_iter()
                .chain([
                    some(ChatFlag::Archived).then(|| entry(tr("Unarchive"), "Ctrl E", ChatFlag::Archived, false)),
                    some(ChatFlag::MarkedUnread)
                        .then(|| entry(tr("Mark as Read"), "Ctrl Shift U", ChatFlag::MarkedUnread, false)),
                    some(ChatFlag::Muted).then(|| {
                        entry(trx("Unmute", "thread context menu action"), "Ctrl Shift M", ChatFlag::Muted, false)
                    }),
                    some(ChatFlag::LowPriority).then(|| entry(tr("Move to Inbox"), "", ChatFlag::LowPriority, false)),
                    (pinned < n).then(|| entry(tr("Pin"), "Ctrl P", ChatFlag::Pinned, true)),
                    (pinned > 0).then(|| entry(tr("Unpin"), "Ctrl P", ChatFlag::Pinned, false)),
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
        let views: &[usize] = match self.active_tab {
            InboxTab::Archived => &[F_LOW_PRIORITY],
            InboxTab::LowPriority => &[F_ARCHIVED],
            InboxTab::Requests => &[],
            _ => &[F_ARCHIVED, F_REQUESTS, F_LOW_PRIORITY],
        };
        let rule = (!views.is_empty()).then(|| menu_rule(p));
        let views = views
            .iter()
            .map(|&i| item(self.filters & 1 << i != 0, FILTERS[i].1, tr(FILTERS[i].0).into(), String::new(), 1 << i));
        let col = Column::with_children(
            std::iter::once(item(
                self.filters == 0,
                icon::INBOX,
                tr("All").into(),
                prefs::hotkey_label(st.settings.hotkey_keys("Filter: All")),
                0,
            ))
            .chain(counted)
            .chain(rule)
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
        let results = tr("Results will be displayed when the poll ends");
        // Desktop's `.Poll-module__wrapper` is as wide as its content, from 240 to 380 px with its 14 px sides.
        let width = {
            let (semi_w, txt_w) =
                (|s: &str| text_width(s, 13.0, Weight::Semibold), |s| text_width(s, 13.0, Weight::Normal));
            let side = if spread { 16.0 } else { 8.0 };
            let rows = poll.options.iter().map(|o| {
                let n = o.voter_ids.len();
                let faces = 23.0 + 14.0 * n.clamp(1, 3).saturating_sub(1) as f32;
                let right = if shown && n > 0 { 10.0 + faces + 6.0 + semi_w(&n.to_string()) } else { 0.0 };
                side + 16.0 + 8.0 + txt_w(&o.label) + right
            });
            let foot = match shown {
                true if total > 0 => semi_w(&votes) + 16.0,
                true => 0.0,
                false => text_width(results, 11.0, Weight::Normal).max(if total > 0 { semi_w(&votes) } else { 0.0 }),
            };
            let head = [text_width(&question, 15.0, Weight::Semibold), 22.0 + txt_w(status), foot];
            (rows.chain(head).fold(0.0, f32::max) + 28.0).clamp(240.0, 380.0)
        };
        let head = column![
            semi(question, 15.0).line_height(Pixels(20.0)).color(fg),
            row![
                tinted(if poll.ended { &POLL_ENDED } else { &COMPOSER_ICONS[8] }, 16.0, sub),
                txt(status, 13.0).color(sub)
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
            let name = txt(o.label.clone(), 13.0).color(lf);
            let faces = o.voter_ids.iter().rev().take(3).rev().enumerate().fold(Stack::new(), |s, (i, id)| {
                let u = chat.participants.items.iter().find(|u| u.id == *id);
                let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(id);
                let face = container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                    .padding(1.5)
                    .style(fill_box(bg, 11.5));
                s.push(container(face).padding(Padding::ZERO.left(14.0 * i as f32)))
            });
            let right =
                (shown && n > 0).then(|| row![faces, semi(n.to_string(), 13.0).color(fg)].spacing(6).align_y(Center));
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
            true if total > 0 => button(row![semi(votes, 13.0), tinted(&CHEVRON_RIGHT, 16.0, footer)].align_y(Center))
                .bare(footer)
                .on_press(UiMessage::OpenPollVotes(chat.id.clone(), m.id.clone()))
                .into(),
            true => Space::new().into(),
            false => column![]
                .push((total > 0).then(|| semi(votes, 13.0).color(fg)))
                .push(txt(results, 11.0).color(sub))
                .spacing(4)
                .into(),
        };
        let foot = (!shown || total > 0).then(|| container(foot).padding(Padding::ZERO.top(10.0)));
        let body = column![head, Space::new().height(10), options].push(foot);
        container(body).padding(Padding::new(14.0).bottom(12.0)).width(width).style(fill_box(bg, 17.0)).into()
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
                    let edit = button(semi(edit, 11.0).color(p.accent))
                        .bare(p.accent)
                        .on_press(UiMessage::ScheduledMenu(ids, false));
                    let line = row![]
                        .push((!s.canceled).then(|| tinted(&COMPOSER_ICONS[6], 12.0, fg)))
                        .push(txt(text, 11.0).font(fw(weight)).color(fg))
                        .push(edit);
                    container(line.spacing(4).align_y(Center)).center_x(Fill).padding(Padding {
                        top: 4.0,
                        right: 8.0,
                        bottom: 8.0,
                        left: 8.0,
                    })
                });
                column![].push(date).push(self.view_scheduled_bubble(st, s, bg)).into()
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
            let label = row![tinted(&COMPOSER_ICONS[6], 12.0, fg), semi(text, 11.0).color(fg), chevron];
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
    fn view_scheduled_bubble(&self, st: &BeeperStore, s: &Scheduled, bg: Color) -> El<'_> {
        let p = &self.pal;
        let fg = if s.canceled { hex(0x808080) } else { p.accent };
        let line = |t: String| txt(t, 13.0).wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        // A file part shows its file name over its caption.
        let file = s
            .upload
            .as_ref()
            .and_then(|id| st.assets.get(id))
            .map(|a| line(a.file_name.clone()).font(fw(Weight::Semibold)).color(fg));
        let text = column![].push(file).push((!s.text.is_empty()).then(|| line(s.text.clone()).color(fg)));
        let bubble = container(text)
            .padding([9.0, BUBBLE_PAD_X])
            .max_width(0.45 * self.pane().size.width)
            .style(fill_box(bg, 17.0));
        let bubble = mouse_area(stack![bubble, dashed::dashes(fg, 17.0)])
            .on_right_press(UiMessage::ScheduledMenu(vec![s.id.clone()], true));
        row![Space::new().width(Fill), bubble].into()
    }

    /// Desktop's "Remove from Beeper" button of the delete dialog.
    fn remove_from_beeper(&mut self, ids: Vec<String>) {
        self.confirm = None;
        let mut st = self.store.write();
        ids.iter().for_each(|id| st.remove_from_beeper(id));
    }

    /// The message menu, its height and the top of its Playback Speed row.
    /// The message menu, the composer menus or the reaction picker of the drawn pane, with their place.
    fn pane_menu(&self) -> Option<(El<'_>, f32, f32)> {
        let message_menu = self.pane().message_menu.as_ref().map(|(mid, at)| {
            let (menu, h, top) = self.view_message_menu(mid);
            let x = if at.x + MESSAGE_MENU_W > self.pane().size.width { at.x - MESSAGE_MENU_W } else { at.x };
            let y = at.y.min(self.pane().size.height - h - 8.0);
            let (sub, sub_w) = match self.view_speed_menu(mid) {
                Some(sub) => (Some(sub), video::SPEED_MENU_W),
                None => {
                    let st = self.store.read();
                    let room = self.pane().size.height - y - 8.0;
                    self.view_voice_sub(&st, &stored(&st, mid), top, room).map_or((None, 0.0), |(e, w)| (Some(e), w))
                }
            };
            let left = sub.is_some() && x + MESSAGE_MENU_W + sub_w > self.pane().size.width;
            let (x, menu) = match left {
                true => (x - sub_w, row![].push(sub).push(menu).into()),
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
        let gallery =
            self.pane().gallery_reactions.as_ref().map(|(ids, at)| (self.view_gallery_reactions(ids), at.x, at.y));
        message_menu.or(edit_menu).or(react).or(gallery)
    }

    /// The command bar finds: the 20 newest text messages, and the 50 newest messages for the media grid. The memo
    /// keeps them for the query, the filters and the room until the store changes, so that only a key or a change of
    /// the store searches again.
    fn search_hits(&self, st: &BeeperStore, q: &str) -> Hits {
        let (f, room) = (&self.search_filters, self.search_room.as_deref());
        let key = (q.to_string(), f.clone(), self.search_room.clone());
        // A period filter moves with the clock, so its finds stay only for one view.
        let memo = self.memo().filter(|_| f.period.is_none());
        if let Some((_, hits)) = memo.and_then(|m| m.hits.borrow().clone()).filter(|(k, _)| *k == key) {
            return hits;
        }
        let words = q.chars().count() > 1 && !q.trim().is_empty();
        let newest = |query: Query, n: usize, keep: &dyn Fn(&Message) -> bool| {
            let mut found = Vec::new();
            st.scan(&query, |m| {
                if keep(&m) && f.keeps(st, room, &m) {
                    found.push(m);
                }
                found.len() < n
            });
            found
        };
        let found = match words && f.media.is_empty() {
            true => newest(Query::default().text(q), 20, &|m| m.text.as_deref().is_some_and(|t| !t.is_empty())),
            false => Vec::new(),
        };
        let media = match f.media.is_empty() {
            true => Vec::new(),
            false => {
                let text: Option<HashSet<String>> = words.then(|| {
                    let all = newest(Query::default().media().text(q), usize::MAX, &|_| true);
                    all.into_iter().map(|m| m.id).collect()
                });
                let titled: Vec<bool> = st.chats.iter().map(|c| c.title.to_lowercase().contains(q)).collect();
                let hit =
                    |m: &Message| text.as_ref().is_none_or(|t| t.contains(&m.id) || titled[st.chat_idx(&m.chat_id)]);
                newest(Query::default().media(), 50, &hit)
            }
        };
        let hits = Rc::new((found, media));
        if let Some(m) = memo {
            *m.hits.borrow_mut() = Some((key, hits.clone()));
        }
        hits
    }

    /// All chats newest first, as indices of `chats`, and the lowercase title and search subtitle of each chat.
    fn chat_words(&self, st: &BeeperStore) -> Words {
        let memo = self.memo();
        if let Some(words) = memo.and_then(|m| m.words.borrow().clone()) {
            return words;
        }
        let mut order: Vec<usize> = (0..st.chats.len()).collect();
        order.sort_by(|&a, &b| st.chats[b].last_activity.cmp(&st.chats[a].last_activity));
        let words = st.chats.iter().map(|c| format!("{}\0{}", c.title, chat_search_subtitle(c)).to_lowercase());
        let words = (order.into(), words.collect());
        if let Some(m) = memo {
            *m.words.borrow_mut() = Some(words.clone());
        }
        words
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
                    semi(label, 11.0).color(fg),
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
        let (menu, sub) = self.account_menu(entry, SpaceMenu::Rail);
        row![menu].push(sub).into()
    }

    /// Desktop's `kS`: the items of a label space menu, after an empty `lead` of that width.
    fn label_items<'a>(&self, l: &Label, lead: f32) -> Vec<El<'a>> {
        let p = &self.pal;
        let blank = || Some(Space::new().width(lead).into());
        let mute = trx(if l.muted { "Unmute" } else { "Mute" }, "label context menu action");
        let heading = (!l.name.is_empty()).then(|| menu_heading(p, l.name.clone()));
        let items = [
            menu_item(p, blank(), mute.into(), "", Some(UiMessage::MuteLabel(l.id.clone(), !l.muted))),
            menu_item(p, blank(), tr("Label Settings").into(), "", Some(UiMessage::OpenLabelSettings(l.id.clone()))),
        ];
        heading.into_iter().chain(items).collect()
    }

    /// Desktop's space sheet (`zft`): the shown spaces with a check on the selected one, then the hidden spaces. The
    /// Space Options of an account space or the items of a label space follow, then the space actions. Returns the
    /// sheet, its width and its height.
    fn view_space_sheet(&self) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let requests = st.chats.iter().any(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0);
        let blank = || Some(Space::new().width(22).into());
        let space = |id: &String| -> El<'_> {
            let view = RAIL_VIEWS.iter().find(|v| v.0 == id);
            let label = rail_label(&st, id);
            let owner = rail_owner(&st, id);
            let on = match view {
                Some(v) => self.selected_account_filter.is_none() && v.1 == Some(self.active_tab),
                None => self.selected_account_filter.as_ref() == Some(id),
            };
            let glyph: El = match (view, label) {
                (Some(v), _) => {
                    let (g, size) = match v.0 {
                        "inbox" => (icon::INBOX, 13.0),
                        "archive" => (icon::ARCHIVE, 12.0),
                        "reminders" => (icon::SCHEDULE, 12.0),
                        "scheduled" => (icon::SCHEDULE_SEND, 12.0),
                        "requests" => (icon::EMAIL, 12.0),
                        "low_priority" => (icon::LOW_PRIORITY_SPACE, 12.0),
                        _ => (icon::MAPS_UGC, 12.0),
                    };
                    let dot = (v.0 == "requests" && requests).then(|| {
                        let dot = container(Space::new()).width(6).height(6);
                        container(dot.style(bordered_box(p.accent, p.text, 1.0, 3.0))).align_right(16)
                    });
                    stack![container(ic(g, size)).center(14)].push(dot).into()
                }
                (None, Some(l)) => label_icon(l, 14.0, p.strong),
                (None, None) => svg(brand_svg(brand_key(&owner))).width(14).height(14).into(),
            };
            let muted = match (view, label) {
                (Some(_), _) => false,
                (None, Some(l)) => l.muted,
                (None, None) => !st.settings.account(&owner).notifications,
            };
            let status = space_error(&st, id);
            let badge = status.map(|s| container(semi(tr(s), 10.0)).padding([2, 6]).style(fill_box(p.chip, 9.0)));
            let line = row![
                container(on.then(|| ic(icon::CHECK, 14.0))).width(22),
                glyph,
                Space::new().width(6),
                txt(space_name(&st, id), 13.0).line_height(Pixels(MENU_LINE)),
                Space::new().width(4)
            ]
            .push(muted.then(|| ic(icon::NOTIFICATIONS_OFF, 12.0)))
            .push(Space::new().width(Fill))
            .push(badge);
            menu_line(p, line, Some(space_msg(id)))
        };
        let shown: Vec<El> = rail_spaces(&st).iter().map(space).collect();
        let hidden: Vec<El> = hidden_spaces(&st).iter().map(space).collect();
        let rules = 1 + usize::from(!shown.is_empty() && !hidden.is_empty());
        let mut top: Vec<El> = shown;
        if rules == 2 {
            top.push(menu_rule(p));
        }
        top.extend(hidden);
        top.push(menu_rule(p));
        let mut h = (top.len() - rules) as f32 * MENU_ITEM + rules as f32 * 11.0;
        let entry = self.selected_account_filter.as_deref();
        let options = match entry.map(|e| (e, rail_label(&st, e))) {
            Some((_, Some(l))) => {
                let items = self.label_items(l, 22.0);
                h += items.len() as f32 * MENU_ITEM + 11.0;
                top.extend(items.into_iter().chain([menu_rule(p)]));
                None
            }
            Some((e, None)) => Some(e),
            None => None,
        };
        let open = UiMessage::ChatSub(Some(1));
        let item = menu_item(p, blank(), tr("Space Options").into(), "›", Some(open.clone()));
        let options_item = options.map(|_| mouse_area(item).on_enter(open));
        let options_top = h;
        let labels = st.feature("labels-v2").show;
        let rest = column![menu_item(p, blank(), tr("Select Chats").into(), "", Some(UiMessage::ToggleSelectMode))]
            .push(labels.then(|| {
                menu_item(p, blank(), tr("New Label").into(), "", Some(UiMessage::OpenCreateLabel(Vec::new())))
            }))
            .push(menu_rule(p))
            .push(menu_item(p, blank(), tr("Edit Spaces...").into(), "", Some(UiMessage::OpenEditSpaces)))
            .push(menu_item(
                p,
                blank(),
                tr("Add/Remove Accounts...").into(),
                "",
                Some(UiMessage::OpenSettings(SettingsTab::Accounts)),
            ));
        h += 8.0 + f32::from(u8::from(options.is_some()) + 3 + u8::from(labels)) * MENU_ITEM + 11.0;
        let top = mouse_area(Column::with_children(top)).on_enter(UiMessage::ChatSub(None));
        let rest = mouse_area(rest).on_enter(UiMessage::ChatSub(None));
        let sheet = column![top].push(options_item).push(rest).padding([4, 0]);
        let sheet = container(sheet).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        drop(st);
        let sub = options.filter(|_| self.chat_sub.is_some()).map(|e| {
            let (menu, sub) = self.account_menu(e, SpaceMenu::Options);
            column![Space::new().height(options_top), row![menu].push(sub)]
        });
        let w =
            233.0 + sub.as_ref().map_or(0.0, |_| 233.0 + if self.chat_sub == Some(0) { ACCOUNT_SUB_W } else { 0.0 });
        (row![sheet].push(sub).into(), w, h)
    }

    /// The menu of a sidebar space and its Show in Inbox submenu. The gear menu of the Edit Spaces window
    /// (`mYe` and `gYe`) has fewer items, and the sidebar header menu (`Ede` with `Ide` or `Ade`) has Select Chats.
    /// A label space has the label items (`kS`) instead.
    fn account_menu(&self, entry: &str, kind: SpaceMenu) -> (El<'_>, Option<El<'_>>) {
        let p = &self.pal;
        let st = self.store.read();
        let select = (kind == SpaceMenu::Header).then(|| {
            let item = menu_item(p, None, tr("Select Chats").into(), "", Some(UiMessage::ToggleSelectMode));
            mouse_area(item).on_enter(UiMessage::ChatSub(None))
        });
        if kind == SpaceMenu::Header && RAIL_VIEWS.iter().any(|v| v.0 == entry) {
            let menu = container(column![select].padding([4, 0])).width(233);
            return (menu.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into(), None);
        }
        if RAIL_VIEWS.iter().any(|v| v.0 == entry) {
            let keys = |name| prefs::hotkey_label(st.settings.hotkey_keys(name));
            let bar = if st.settings.on("showSidebar") { "Hide Space Bar" } else { "Show Space Bar" };
            let col = column![
                menu_item(
                    p,
                    None,
                    tr("Add/Remove Accounts…").into(),
                    keys("Settings"),
                    Some(UiMessage::OpenSettings(SettingsTab::Accounts))
                ),
                menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)),
                menu_item(
                    p,
                    None,
                    tr(bar).into(),
                    keys("Toggle Space Bar"),
                    Some(UiMessage::RunHotkey("Toggle Space Bar"))
                ),
                menu_item(
                    p,
                    None,
                    tr("Settings").into(),
                    keys("Settings"),
                    Some(UiMessage::OpenSettings(SettingsTab::General))
                ),
            ];
            let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            return (menu.into(), None);
        }
        if let Some(l) = rail_label(&st, entry) {
            let item = |label: &str, msg| menu_item(p, None, label.into(), "", Some(msg));
            let tail = match kind {
                SpaceMenu::Rail => vec![
                    menu_rule(p),
                    item(tr("Edit Spaces…"), UiMessage::OpenEditSpaces),
                    item(tr("Add/Remove Accounts…"), UiMessage::OpenSettings(SettingsTab::Accounts)),
                ],
                SpaceMenu::Header => vec![menu_rule(p), select.expect("the header menu has Select Chats").into()],
                SpaceMenu::Gear => vec![],
                SpaceMenu::Prefs | SpaceMenu::Options => unreachable!("only account spaces have these menus"),
            };
            let col = Column::with_children(self.label_items(l, 0.0).into_iter().chain(tail));
            let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            return (menu.into(), None);
        }
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
        let gear = kind == SpaceMenu::Gear;
        let label = match (gear, muted) {
            (true, _) => trx(if muted { "Unmute" } else { "Mute" }, "account context menu action"),
            (false, true) => tr("Unmute Space"),
            (false, false) => tr("Mute Space"),
        };
        let mute = menu_item(p, None, label.into(), "", Some(UiMessage::SetAccountProps(account.into(), mute)));
        let accounts = UiMessage::OpenSettings(SettingsTab::Accounts);
        let settings = UiMessage::OpenAccountSettings(account.into());
        let rest = match kind {
            SpaceMenu::Gear => column![mute],
            SpaceMenu::Header | SpaceMenu::Options => {
                column![mute, menu_rule(p), menu_item(p, None, tr("Account Settings...").into(), "", Some(settings))]
            }
            SpaceMenu::Prefs => {
                let remove = match account {
                    "hungryserv" => UiMessage::CloseChatMenu,
                    _ => UiMessage::AskRemoveAccount(account.into()),
                };
                column![
                    mute,
                    menu_rule(p),
                    menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)),
                    menu_item(p, None, tr("Remove Account…").into(), "", Some(remove)),
                ]
            }
            SpaceMenu::Rail => column![
                mute,
                menu_rule(p),
                menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)),
                menu_item(p, None, tr("Add/Remove Accounts…").into(), "", Some(accounts)),
                menu_rule(p),
                menu_item(p, None, tr("Account Settings…").into(), "", Some(settings)),
            ],
        };
        // Desktop's Reconnect item (`App:89486`) of an account whose bridge asks for a new login.
        let again = reconnect(&st, account).filter(|_| kind != SpaceMenu::Gear);
        let again = again.map(|m| column![menu_rule(p), menu_item(p, None, tr("Reconnect").into(), "", Some(m))]);
        let col = column![
            select,
            mouse_area(inbox).on_enter(UiMessage::ChatSub(Some(0))),
            mouse_area(rest.push(again)).on_enter(UiMessage::ChatSub((kind == SpaceMenu::Options).then_some(1)))
        ];
        let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        (menu.into(), sub.map(Into::into))
    }

    /// Desktop's `SwitchChatHoverCard`: "Switch Chat" and a row for each chat of the person, with the avatar, the
    /// network and the user of its account, and a check on the open chat. The card closes when the pointer leaves it.
    fn view_switch_card(&self) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let open = self.pane.selected_chat_id.as_deref().expect("an open chat");
        let chat = &st.chats[st.resolve_chat_idx(open).expect("chat exists")];
        let targets = switch_targets(&st, chat).expect("a chat with switch targets");
        let line = || container(Space::new().width(Fill).height(1)).style(fill_box(p.received, 0.0));
        let header =
            container(semi(tr("Switch Chat"), 13.0).color(p.strong)).padding(Padding::new(12.0).left(27.0).right(8.0));
        // Desktop's card is as wide as its rows, up to 300 px.
        let wide = |c: &Chat| {
            let name = text_width(network_name(&c.account_id), 16.0, Weight::Normal);
            29.0 + 28.0 + 8.0 + name.max(text_width(&account_user(&st, &c.account_id), 12.0, Weight::Normal)) + 8.0
        };
        let chats = targets.iter().map(|id| &st.chats[st.resolve_chat_idx(id).expect("a switch target exists")]);
        let w = chats.map(wide).fold(35.0 + text_width(tr("Switch Chat"), 13.0, Weight::Semibold), f32::max).min(300.0);
        let rows = targets.iter().map(|id| {
            let c = &st.chats[st.resolve_chat_idx(id).expect("a switch target exists")];
            let on = id == open;
            let r = st.bridge_accounts.iter().find(|b| b.account_id == c.account_id);
            let face = self.face(
                r.and_then(|b| b.remote_profile.as_ref()?.avatar.as_deref()),
                &account_user(&st, &c.account_id),
                28.0,
                0.0,
            );
            let info = column![
                txt(network_name(&c.account_id).to_string(), 16.0).color(p.strong),
                txt(account_user(&st, &c.account_id), 12.0).line_height(Pixels(16.0)).color(p.strong.scale_alpha(0.4)),
            ];
            let check = on.then(|| ic(icon::CHECK, 13.0).color(p.strong));
            let row = row![].push(check).push(face).push(info).spacing(8).align_y(Center);
            let pad = Padding::new(8.0).left(if on { 8.0 } else { 29.0 });
            let b =
                button(row).padding(pad).width(Fill).style(btn_style([CLEAR, p.hover, p.selected], [p.text; 3], 0.0));
            column![b.on_press(UiMessage::SwitchChat(id.clone()))]
                .push((id != targets.last().expect("a target")).then(line))
                .into()
        });
        let h = 41.0 + targets.len() as f32 * 53.0;
        let card = column![header, line()].extend(rows).width(w.ceil());
        let card = container(card).style(shadow_box(p.menu, p.menu_border, 8.0, 16.0));
        (mouse_area(card).on_exit(UiMessage::CloseChatMenu).into(), h)
    }

    /// Desktop's `composeSwitchTargets.onSelect`: opens the chat on the other account, and moves the draft there
    /// when that chat has none.
    fn switch_chat(&mut self, id: String) {
        self.chat_menu = None;
        if self.pane.selected_chat_id.as_deref() == Some(id.as_str()) {
            return;
        }
        let empty = {
            let st = self.store.read();
            let d =
                st.chats[st.resolve_chat_idx(&id).expect("a switch target exists")].draft.clone().unwrap_or_default();
            d.text.as_deref().unwrap_or_default().trim().is_empty() && d.attachments.is_empty()
        };
        let pane = &mut self.pane;
        let carry = empty.then(|| {
            let text = std::mem::replace(&mut pane.composer, composer_with(""));
            let (files, mentions) = (std::mem::take(&mut pane.attachments), std::mem::take(&mut pane.mentions));
            (text, files, mentions, std::mem::take(&mut pane.composer_links), pane.send_later.take())
        });
        self.select_chat(&id);
        if let Some(draft) = carry {
            let pane = &mut self.pane;
            (pane.composer, pane.attachments, pane.mentions, pane.composer_links, pane.send_later) = draft;
            pane.draft_due = true;
        }
    }

    /// Waits Desktop's 300 ms before the user card shows, or 150 ms before it hides, which lets the pointer cross from
    /// the trigger to the card.
    fn settle_user_hover(&mut self) -> Task<UiMessage> {
        let h = &mut self.user_hover;
        h.r#gen += 1;
        let ms = match (h.trigger || h.card, h.shown.is_some()) {
            (true, false) => 300,
            (false, true) => 150,
            _ => return Task::none(),
        };
        let n = h.r#gen;
        let wait = async move { tokio::time::sleep(std::time::Duration::from_millis(ms)).await };
        Task::perform(wait, move |()| UiMessage::HoverUserDue(n))
    }

    /// Where a user card `h` high shows for the pointer at `at`: centered on it, `gap` below it or above it near the
    /// bottom.
    fn card_spot(&self, at: Point, gap: f32, h: f32) -> (f32, f32) {
        let size = self.pane().size;
        let x = (at.x - CARD_W / 2.0).clamp(8.0, size.width - CARD_W - 8.0);
        let below = at.y + gap + h <= size.height;
        (x, if below { at.y + gap } else { at.y - gap - h })
    }

    /// A trigger of Desktop's `BrandUserHoverCard`: the card of `uid`, a member of `chat`, shows while the pointer
    /// rests on `el`.
    pub(crate) fn user_trigger<'a>(&self, chat: &Chat, uid: &str, el: impl Into<El<'a>>) -> El<'a> {
        if !chat.participants.items.iter().any(|u| u.id == uid) {
            return el.into();
        }
        mouse_area(el)
            .on_enter(UiMessage::HoverUser(Some(format!("{} {uid}", chat.id))))
            .on_exit(UiMessage::HoverUser(None))
            .into()
    }

    /// The user card of `user_hover` over the open chat, which keeps the pointer from the chat under it.
    fn view_hover_card(&self) -> Option<El<'_>> {
        let h = &self.user_hover;
        let at = h.shown?;
        let (cid, uid) = h.key.split_once(' ')?;
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(cid)?];
        let shown = self.pane().selected_chat_id.as_deref() == Some(c.id.as_str());
        (shown && c.participants.items.iter().any(|u| u.id == uid)).then_some(())?;
        drop(st);
        let (card, high) = self.view_user_card(&h.key);
        let (x, y) = self.card_spot(at, 12.0, high);
        let card = mouse_area(card)
            .on_enter(UiMessage::HoverUserCard(true))
            .on_exit(UiMessage::HoverUserCard(false))
            .interaction(mouse::Interaction::Idle);
        Some(container(card).padding(Padding::ZERO.left(x).top(y)).into())
    }

    /// Desktop's `BrandUserHoverCard` of the member in "chat user": the avatar over the card, the name, the
    /// username, phone number or email, and the Send Message and call buttons; and the height of the card.
    fn view_user_card(&self, key: &str) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let (cid, uid) = key.split_once(' ').expect("a chat and a user");
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let u = c.participants.items.iter().find(|u| u.id == uid).expect("a member of the chat");
        let bg = if self.dark { Color::BLACK } else { Color::WHITE };
        let shade = Color { a: 0.1, ..p.strong };
        let lifted = move |r: f32| {
            move |_: &Theme| container::Style {
                background: Some(bg.into()),
                border: iced::border::rounded(r),
                shadow: iced::Shadow { color: shade, offset: iced::Vector::new(0.0, 1.0), blur_radius: 10.0 },
                ..Default::default()
            }
        };
        let name = member_name(u);
        let id = handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone());
        let face = container(self.face(u.img_url.as_deref(), &name, 80.0, 0.0)).padding(4).style(lifted(44.0));
        let high = CARD_H + if id.is_some() { 15.0 } else { 0.0 };
        let info = column![semi(name, 14.0).line_height(Pixels(18.0)).color(p.strong).width(Fill).align_x(Center)]
            .push(id.map(|i| txt(i, 12.0).color(p.sub).width(Fill).align_x(Center)));
        let muted = btn_style([p.chip; 3], [p.strong; 3], 9999.0);
        let actions = (u.is_self != Some(true) && self.can(&c.account_id, "create_dm")).then(|| {
            let label = txt(tr("Send Message"), 11.0);
            let send = button(container(label).center_x(Fill)).padding([4, 8]).width(Fill).style(muted);
            let call = u.phone_number.as_ref().map(|n| {
                let open = UiMessage::Commands(vec![UiMessage::CloseModals, UiMessage::OpenUrl(format!("tel:{n}"))]);
                button(tinted(&CALL, 20.0, p.strong)).padding(4).style(muted).on_press(open)
            });
            row![send.on_press(UiMessage::MessageMember(cid.into(), uid.into()))].push(call).spacing(8).align_y(Center)
        });
        let high = high + if actions.is_some() { 36.0 } else { 0.0 };
        let card = column![Space::new().height(CARD_FACE_IN), info].push(actions).spacing(8).padding([8, 11]);
        let card = container(card).width(CARD_W).style(lifted(17.0));
        let face = container(face).center_x(CARD_W);
        (stack![column![Space::new().height(88.0 - 8.0 - CARD_FACE_IN), card], face].into(), high)
    }

    /// The chat menu, with `room` pixels from its top to the bottom of the window.
    fn view_chat_menu(&self, cid: &str, sub_left: bool, room: f32) -> El<'_> {
        let p = &self.pal;
        if cid == SIDEBAR_MENU {
            let st = self.store.read();
            let keys = |name| prefs::hotkey_label(st.settings.hotkey_keys(name));
            let bar = if st.settings.on("showSidebar") { "Hide Space Bar" } else { "Show Space Bar" };
            let col = column![
                menu_item(p, None, tr("Start New Chat").into(), keys("Chat: Start New"), Some(UiMessage::NewChat)),
                menu_item(
                    p,
                    None,
                    tr("Add/Remove Accounts").into(),
                    "",
                    Some(UiMessage::OpenSettings(SettingsTab::Accounts))
                ),
                menu_item(
                    p,
                    None,
                    tr(bar).into(),
                    keys("Toggle Space Bar"),
                    Some(UiMessage::RunHotkey("Toggle Space Bar"))
                ),
                menu_item(p, None, tr("Settings").into(), "", Some(UiMessage::OpenSettings(SettingsTab::General))),
            ];
            return container(col.padding([4, 0]))
                .width(233)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into();
        }
        if cid == RAIL_MORE {
            let items = more_menu(&self.store.read()).into_iter().map(|i| match i {
                Some((label, msg)) => menu_item(p, None, label, "", Some(msg)),
                None => menu_rule(p),
            });
            let col = Column::with_children(items).padding([4, 0]);
            return container(col).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into();
        }
        if INFO_MENUS.iter().any(|k| cid.starts_with(k)) {
            let items = info_menu(&self.store.read(), cid, |a| self.can(a, "create_dm")).into_iter().map(|i| match i {
                Some((label, msg)) => menu_item(p, None, label, "", msg),
                None => menu_rule(p),
            });
            let col = Column::with_children(items).padding([4, 0]);
            return container(col).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into();
        }
        if let Some(entry) = cid.strip_prefix("account:") {
            return self.view_account_menu(entry);
        }
        if let Some(entry) = cid.strip_prefix("head:") {
            let (menu, sub) = self.account_menu(entry, SpaceMenu::Header);
            return row![menu].push(sub).into();
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
        let flag = |f: ChatFlag| Some(UiMessage::ToggleFlag(c.id.clone(), f));
        let archived = c.flag(ChatFlag::Archived);
        let unread = c.is_unread();
        let close_sub = |items: Column<'static, UiMessage>| mouse_area(items).on_enter(UiMessage::ChatSub(None));
        let item = |label: &str, key: &'static str, msg| menu_item(p, None, label.into(), key, msg);
        let open_sub = |i: u8, label: &str| {
            mouse_area(menu_item(p, None, label.into(), "›", Some(UiMessage::ChatSub(Some(i)))))
                .on_enter(UiMessage::ChatSub(Some(i)))
        };
        let pick = |on: bool, yes, no| if on { yes } else { no };
        let selecting = self.selection.is_empty();
        let labels = st.feature("labels-v2").on;
        let reason = st.notifications_muted(c).then(|| muted_by(&st, c)).flatten();
        let picked = self.selected_chats(&st);
        let n = picked.len();
        let reason = reason.filter(|_| n == 0);
        let every = |f: ChatFlag| picked.iter().all(|c| has_flag(c, f));
        let some = |f: ChatFlag| picked.iter().any(|c| has_flag(c, f));
        let set = |f: ChatFlag, on: bool| Some(UiMessage::SelectionSet(f, on));
        let can_archive = archived || !c.flag(ChatFlag::Pinned);
        let archive = match n {
            0 => item(
                pick(
                    archived,
                    trx("Unarchive", "thread context menu action"),
                    trx("Archive", "thread context menu action"),
                ),
                "Ctrl E",
                flag(ChatFlag::Archived).filter(|_| can_archive),
            ),
            _ => item(
                match every(ChatFlag::Archived) {
                    true => trx("Unarchive", "button label to unarchive selected chats"),
                    false => trx("Archive", "button label to archive selected chats"),
                },
                "Ctrl E",
                set(ChatFlag::Archived, !every(ChatFlag::Archived)).filter(|_| can_archive),
            ),
        };
        let mute = match (n, st.notifications_muted(c)) {
            (0, false) => item(trx("Mute", "thread context menu action"), "Ctrl Shift M", flag(ChatFlag::Muted)),
            (0, true) => item(
                trx("Unmute", "thread context menu action"),
                "Ctrl Shift M",
                flag(ChatFlag::Muted).filter(|_| c.flag(ChatFlag::Muted) && reason.is_none()),
            ),
            _ => item(
                pick(
                    every(ChatFlag::Muted),
                    trx("Unmute", "thread context menu action"),
                    trx("Mute", "thread context menu action"),
                ),
                "Ctrl Shift M",
                set(ChatFlag::Muted, !every(ChatFlag::Muted)),
            ),
        };
        let read = |all: bool| match all {
            true => item(tr("Mark as Read"), "Ctrl Shift U", set(ChatFlag::MarkedUnread, false)),
            false => item(tr("Mark as Unread"), "Ctrl Shift U", set(ChatFlag::MarkedUnread, true)),
        };
        let reads: Vec<El> = match n {
            0 => vec![item(
                tr(pick(unread, "Mark as Read", "Mark as Unread")),
                "Ctrl Shift U",
                Some(UiMessage::ToggleRead(c.id.clone())),
            )],
            1 => vec![read(every(ChatFlag::MarkedUnread))],
            _ => [
                some(ChatFlag::MarkedUnread)
                    .then(|| item(tr("Mark All as Read"), "", set(ChatFlag::MarkedUnread, false))),
                picked
                    .iter()
                    .any(|c| !has_flag(c, ChatFlag::MarkedUnread))
                    .then(|| item(tr("Mark All as Unread"), "", set(ChatFlag::MarkedUnread, true))),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        let pinned = if n == 0 { c.flag(ChatFlag::Pinned) } else { every(ChatFlag::Pinned) };
        let pin = match n {
            0 => flag(ChatFlag::Pinned),
            _ => set(ChatFlag::Pinned, !pinned),
        };
        let low = if n == 0 { c.flag(ChatFlag::LowPriority) } else { every(ChatFlag::LowPriority) };
        let low = item(
            tr(pick(low, "Move to Inbox", "Move to Low Priority")),
            "",
            if n == 0 { flag(ChatFlag::LowPriority) } else { set(ChatFlag::LowPriority, !low) },
        );
        let heading = (n > 1).then(|| item(&trn("%d chat selected", "%d chats selected", n, &[&n]), "", None));
        let (accept, decline) = request_answers(c);
        let group = c.chat_type == "group";
        let accept = accept.then(|| {
            let label = tr(pick(group, "Accept group invitation", "Accept message request"));
            item(label, "", Some(UiMessage::AcceptRequest(c.id.clone())))
        });
        let decline = decline.then(|| {
            let label = tr(pick(group, "Decline group invitation", "Decline message request"));
            item(label, "", Some(UiMessage::DeleteChat(c.id.clone(), false)))
        });
        let answered = (accept.is_some() || decline.is_some()).then(|| menu_rule(p));
        let col = column![
            close_sub(
                column![]
                    .push(accept)
                    .push(decline)
                    .push(answered)
                    .push(heading)
                    .push(archive)
                    .push(mute)
                    .push(reason.as_deref().map(|r| item(r, "", None)))
                    .extend(reads)
                    .push(item(
                        pick(
                            pinned,
                            trx("Unpin", "thread context menu action"),
                            trx("Pin", "thread context menu action")
                        ),
                        "Ctrl P",
                        pin
                    ))
                    .push((n <= 1 && st.threaded(c)).then(|| {
                        let on = c.follows.follows_all(c.participants.total);
                        let label = tr(if on { "Stop Following All Threads" } else { "Follow All Threads" });
                        item(label, "", Some(UiMessage::FollowAllThreads(c.id.clone(), !on)))
                    }))
                    .push(can_edit_group(c).then(|| {
                        let open = UiMessage::OpenEditGroup(c.id.clone());
                        item(tr("Edit Group..."), "", (c.is_request != Some(true)).then_some(open))
                    }))
            ),
            open_sub(0, tr("Copy")),
            close_sub(column![
                menu_rule(p),
                item(tr("Search…"), "Ctrl F", Some(UiMessage::SearchRoom(c.id.clone()))),
                menu_rule(p),
                low,
            ]),
            labels.then(|| open_sub(2, trx("Label", "thread context menu submenu label"))),
            open_sub(1, tr("Remind Me")),
            close_sub({
                let delete = st.feature("delete-chats").on.then(|| {
                    let ask = (c.delete_modes != 0).then(|| UiMessage::AskDeleteChat(c.id.clone()));
                    item(tr("Delete Chat"), "", ask)
                });
                let block = block_action(&st, c).filter(|_| n == 0).map(|block| {
                    let label = match block {
                        true => trx("Block…", "action to block the other person in a DM"),
                        false => trx("Unblock…", "action to unblock the other person in a DM"),
                    };
                    item(label, "", Some(UiMessage::AskBlock(c.id.clone(), block)))
                });
                // Like Electron's menus, a group without items has no separator.
                column![]
                    .push((delete.is_some() || block.is_some()).then(|| menu_rule(p)))
                    .push(delete)
                    .push(block)
                    .push(selecting.then(|| menu_rule(p)))
                    .push(
                        selecting
                            .then(|| item(tr("Open in New Window"), "", Some(UiMessage::OpenChatWindow(c.id.clone())))),
                    )
                    .push(
                        selecting
                            .then(|| item(tr("Begin Selecting"), "", Some(UiMessage::BeginSelecting(c.id.clone())))),
                    )
                    .push(menu_rule(p))
                    .push(item(tr("Report a Problem"), "Ctrl Shift P", Some(UiMessage::OpenUrl(ISSUES_URL.into()))))
            }),
        ];
        let menu = container(col.padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let sub = self.chat_sub.map(|i| {
            let item = |label: &str, at: String, msg| menu_item(p, None, label.into(), at, msg);
            let (items, rules): (Vec<El>, f32) = match i {
                0 => (vec![item(tr("Chat Link"), String::new(), Some(UiMessage::CopyChatLink(c.id.clone())))], 0.0),
                2 => {
                    let targets: Vec<String> = match picked.is_empty() {
                        true => vec![c.id.clone()],
                        false => picked.iter().map(|c| c.id.clone()).collect(),
                    };
                    let mut sorted: Vec<&Label> = st.labels.iter().collect();
                    sorted.sort_by_cached_key(|l| l.name.to_lowercase());
                    let rows = sorted.into_iter().map(|l| {
                        let on = c.label_ids.contains(&l.id);
                        let lead = container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into();
                        let msg = UiMessage::ToggleLabel(l.id.clone(), targets.clone(), !on);
                        menu_item(p, Some(lead), l.name.clone(), "", Some(msg))
                    });
                    let new = item(tr("New Label…"), String::new(), Some(UiMessage::OpenCreateLabel(targets.clone())));
                    (rows.chain([menu_rule(p), new]).collect(), 1.0)
                }
                _ => {
                    let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
                    // Desktop's `canSetReminderFor`: Beeper Plus, a free use, or a reminder to change.
                    let can = st.feature("remind-later").on || c.reminder.is_some();
                    let set = |at| can.then(|| UiMessage::SetReminder(c.id.clone(), Some(at)));
                    let last = last_used(&st, now).map(|at| item(tr("Last Used"), last_used_label(at, h24), set(at)));
                    let presets = remind_presets(now).map(|(label, at)| {
                        let msg = set(at).map(|set| UiMessage::Commands(vec![UiMessage::LastTime(label.into()), set]));
                        item(tr(label), remind_label(at, h24), msg)
                    });
                    let upgrade = (!can).then(|| {
                        let open = UiMessage::OpenSettings(SettingsTab::BeeperPlus);
                        [item(tr("Upgrade to Beeper Plus"), String::new(), Some(open)), menu_rule(p)]
                    });
                    let presets = upgrade.into_iter().flatten().chain(last).chain(presets);
                    let dismiss = c.reminder.is_some().then(|| UiMessage::SetReminder(c.id.clone(), None));
                    let custom = UiMessage::OpenSubMenu(commands::SubMenu::Remind(c.id.clone()));
                    let custom = item(tr("Custom…"), String::new(), can.then_some(custom));
                    let dismiss = item(tr("Dismiss Reminder"), String::new(), dismiss);
                    let rules = if can { 2.0 } else { 3.0 };
                    (presets.chain([menu_rule(p), custom, menu_rule(p), dismiss]).collect(), rules)
                }
            };
            let h = 8.0 + items.len() as f32 * MENU_ITEM - rules * (MENU_ITEM - 11.0);
            let top = match i {
                0 => 4.0 * MENU_ITEM,
                1 if labels => 8.0 * MENU_ITEM + 22.0,
                _ => 7.0 * MENU_ITEM + 22.0,
            } + if reason.is_some() { MENU_ITEM } else { 0.0 }
                + if can_edit_group(c) { MENU_ITEM } else { 0.0 }
                + self.selection_rows(&st);
            let top = top.min(room - h).max(0.0);
            let body = container(Column::with_children(items).padding([4, 0])).width(SUB_MENU_W[usize::from(i)]);
            column![Space::new().height(top), body.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))]
        });
        let (l, r) = if sub_left { (sub, None) } else { (None, sub) };
        row![].push(l).push(menu).push(r).into()
    }

    /// Desktop's timeline effect (`markAsRead` as the newest message comes into view): the open chat is read while
    /// its window is focused and the timeline is at its bottom, except in incognito mode.
    fn read_open_chat(&self) {
        let at_bottom = self.pane.scrolled_up.is_none_or(|(up, _)| !up);
        let Some(id) = self.pane.selected_chat_id.as_deref().filter(|_| self.focused && at_bottom) else { return };
        let mut st = self.store.write();
        if let Some(idx) = st.resolve_chat_idx(id).filter(|_| !st.incognito() && self.pane.thread_root.is_none()) {
            st.mark_read(idx, false);
        }
    }

    fn mark_read(&self, st: &mut BeeperStore, idx: usize) {
        if self.expired(&st.chats[idx]) {
            st.set_reminder(idx, None);
        }
        st.mark_read(idx, true);
    }

    /// Desktop's `setIsArchived`: archiving marks the chat read when "Mark unread chats as read on archive" is on.
    /// Returns false for a pinned chat, which stays in the inbox.
    /// Desktop's `showArchiveToast`, `toggleThreadArchive` and `showLowPriorityToast`: `chats` moved to `on`, with the
    /// flag that Undo sets back. `many` is a selection of chats, which Undo selects again.
    fn flag_toast(&mut self, flag: ChatFlag, on: bool, chats: Vec<(String, bool)>, many: bool) {
        let n = chats.len();
        let (text, icon, open) = match flag {
            ChatFlag::Archived => {
                let text = match (many, on) {
                    (false, true) => tr("Archived").into(),
                    (false, false) => tr("Unarchived").into(),
                    (true, true) => trn("Archived %d chat", "Archived %d chats", n, &[&n]),
                    (true, false) => trn("Unarchived %d chat", "Unarchived %d chats", n, &[&n]),
                };
                (text, (!many).then_some(0), if on { InboxTab::Archived } else { InboxTab::AllChats })
            }
            _ => {
                let text = match (n, on) {
                    (1, true) => tr("Moved to Low Priority").into(),
                    (1, false) => tr("Moved to Inbox").into(),
                    (_, true) => trn("Moved %d chat to Low Priority", "Moved %d chats to Low Priority", n, &[&n]),
                    (_, false) => trn("Moved %d chat to Inbox", "Moved %d chats to Inbox", n, &[&n]),
                };
                (text, Some(if on { 2 } else { 1 }), if on { InboxTab::LowPriority } else { InboxTab::AllChats })
            }
        };
        if n > 0 {
            self.status_banner = text.clone();
            let undo = (tr("Undo"), UiMessage::UndoFlag(flag, chats, many));
            let (buttons, right) = if icon.is_some() { (Vec::new(), Some(undo)) } else { (vec![undo], None) };
            let open = Some(UiMessage::SelectTab(open));
            self.toast = Some(Toast { icon, strong: icon.is_some(), buttons, right, open, ..Toast::new(text) });
        }
    }

    /// The ID of a label, after its label space replaced the ID it had before the homeserver made it.
    fn label_id(&self, id: String) -> String {
        self.store.read().sent.get(&id).cloned().unwrap_or(id)
    }

    /// Desktop's `runAction`: keeps the undo of an action, and shows its `successToast` with Undo.
    fn toggle_label(&mut self, label: String, chats: Vec<String>, add: bool, toast: bool) {
        self.chat_menu = None;
        let label = self.label_id(label);
        let mut st = self.store.write();
        let has =
            |st: &BeeperStore, c: &str| st.chats[st.resolve_chat_idx(c).expect("a chat")].label_ids.contains(&label);
        let chats: Vec<String> = chats.into_iter().filter(|c| has(&st, c) != add).collect();
        for chat in &chats {
            st.set_label_room(&label, chat, add);
        }
        let name = st.labels.iter().find(|l| l.id == label).expect("a toggled label").name.clone();
        drop(st);
        let (what, text) = match add {
            true => (format!("Add to {name}"), format!("Added to {name}")),
            false => (format!("Remove from {name}"), format!("Removed from {name}")),
        };
        let open = add.then(|| UiMessage::SelectAccount(Some(format!("label:{label}"))));
        self.act(what, UiMessage::ToggleLabel(label, chats, !add), toast.then_some((text, open)));
    }

    fn act(&mut self, what: String, undo: UiMessage, toast: Option<(String, Option<UiMessage>)>) {
        if self.undo_stack.len() == UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push((what, undo));
        if let Some((text, open)) = toast {
            self.show(Toast {
                strong: true,
                right: Some((tr("Undo"), UiMessage::UndoAction)),
                open,
                ..Toast::new(text)
            });
        }
    }

    /// The text of `ExportChat`: each loaded message with its time, sender, attachments, links and reactions.
    fn export_text(&self, st: &BeeperStore) -> Option<String> {
        let text: Vec<String> = self.timeline(st).0.into_iter().map(|m| copy_text(m, true, true)).collect();
        (!text.is_empty()).then(|| text.join("\n"))
    }

    /// The end of Desktop's `createThread` and `createGroupThread`: the new chat opens, or an error toast shows.
    fn chat_created(
        &mut self,
        n: Option<u32>,
        account: &str,
        quote: Option<String>,
        r: Result<String, String>,
        action: &str,
    ) {
        if self.sticky_banner.take().is_some_and(|t| t == self.status_banner) {
            self.status_banner.clear();
        }
        match r {
            Ok(room) => {
                self.new_chats.retain(|d| Some(d.id) != n);
                let synced = self.store.read().resolve_chat_idx(&room).is_some();
                match synced {
                    true => self.select_dm(&room, quote),
                    false => (self.creating_chat, self.pending_dm) = (None, Some((room, quote))),
                }
            }
            Err(e) => {
                let st = self.store.read();
                let me = account_user(&st, account);
                let text = trf("Something went wrong %1$s (%2$s)\n%3$s", &[&action, &me, &e]);
                drop(st);
                self.show(Toast { buttons: vec![(tr("Copy Error"), UiMessage::CopyText(e))], ..Toast::new(text) });
            }
        }
    }

    fn show(&mut self, t: Toast) {
        self.status_banner = t.text.clone();
        self.toast = Some(t);
    }

    /// Desktop's `searchUsers` on the accounts whose bridge has the `search` capability, or only on `only`.
    fn search_users(&mut self, q: &str, only: Option<String>) -> Task<UiMessage> {
        let accounts = rail_accounts(&self.store.read());
        let accounts: Vec<String> = accounts
            .into_iter()
            .filter(|a| self.can(a, "create_dm") && self.can(a, "search") && only.as_ref().is_none_or(|o| o == a))
            .collect();
        if accounts.is_empty() {
            return Task::none();
        }
        self.new_chat_search = Some((q.into(), accounts.len(), Vec::new()));
        Task::batch(accounts.into_iter().map(|a| {
            let (q, search) = (q.to_string(), beeper_core::matrix::search_users(self.provision(&a), q.into()));
            Task::perform(search, move |r| UiMessage::SearchedUsers(q.clone(), a.clone(), r))
        }))
    }

    /// Desktop's `ix`: the users of a phone number, email or username on the accounts. An account with `any_phone`
    /// reaches a phone number at once; the others ask the bridge with `resolve_identifier`.
    fn resolve_identifiers(
        &self,
        key: &str,
        accounts: &[String],
        phone: Option<&str>,
        email: Option<&str>,
        username: Option<&str>,
    ) -> (Networks, Task<UiMessage>) {
        let (mut nets, mut tasks) = (Vec::new(), Vec::new());
        for a in accounts {
            let direct = phone.filter(|_| self.can(a, "any_phone"));
            if let Some(phone) = direct {
                let user = User { id: phone_digits(phone), phone_number: Some(phone.into()), ..Default::default() };
                nets.push((a.clone(), Some(Some(user))));
            }
            let asks = [
                ("lookup_phone", phone.filter(|_| direct.is_none())),
                ("lookup_email", email),
                ("lookup_username", username),
            ];
            for id in asks.into_iter().filter(|(c, _)| self.can(a, c)).filter_map(|(_, id)| id) {
                let (key, slot) = (key.to_string(), nets.len());
                let resolve = beeper_core::matrix::resolve_identifier(self.provision(a), id.into());
                tasks.push(Task::perform(resolve, move |r| UiMessage::ContactNetwork(key.clone(), slot, r)));
                nets.push((a.clone(), None));
            }
        }
        (nets, Task::batch(tasks))
    }

    /// Whether the bridge of a chat account has a `resolve_identifier` provisioning capability.
    fn can(&self, account_id: &str, capability: &str) -> bool {
        self.caps.get(account_id).is_some_and(|c| c["resolve_identifier"][capability] == true)
    }

    /// Desktop's `canCreateGroupChat`: the bridge of a chat account has `group_creation.group`.
    fn can_group(&self, account_id: &str) -> bool {
        self.caps.get(account_id).is_some_and(|c| c["group_creation"]["group"].is_object())
    }

    /// The bridge login of a chat account, for the bridge provisioning API.
    fn provision(&self, account_id: &str) -> beeper_core::matrix::Provision {
        let st = self.store.read();
        let found = st.bridge_accounts.iter().find(|a| a.account_id == account_id);
        let (bridge_id, login_id) = found.map_or_else(
            || {
                let (b, l) = account_id.split_once('.').unwrap_or((account_id, ""));
                (b.to_string(), l.to_string())
            },
            |a| (a.bridge_id.clone(), a.login_id.clone()),
        );
        beeper_core::matrix::Provision {
            homeserver: st.beeper_state.homeserver.clone(),
            access_token: st.beeper_state.access_token.clone(),
            user_id: st.beeper_state.user_id.clone(),
            bridge_id,
            login_id,
        }
    }

    fn fail(&mut self, t: Toast) {
        self.show(Toast { error: true, ..t });
    }

    /// The toast timeouts run while a window is focused and the pointer is not on the toasts.
    fn toast_running(&self) -> bool {
        let hovered = self.toast_clock.0 == self.status_banner && self.toast_clock.1;
        let shown = self.newest_toast_running() || !self.older_toasts.is_empty();
        self.relaunch_toast.is_none() && self.focused && !hovered && shown
    }

    fn newest_toast_running(&self) -> bool {
        !self.status_banner.is_empty() && self.sticky_banner.as_ref() != Some(&self.status_banner)
    }

    /// The newest toast and the time it showed, which moves up in Desktop's `toasts-list` when a new toast comes.
    /// Sticky toasts and "Undo:" notes are replaced instead.
    fn stackable_toast(&self) -> Option<(Toast, Duration)> {
        if !self.newest_toast_running() || self.relaunch_toast.is_some() || self.shown_note() {
            return None;
        }
        let t = self.shown_toast().cloned().unwrap_or_else(|| Toast::new(self.status_banner.clone()));
        let clock = &self.toast_clock;
        Some((t, if clock.0 == self.status_banner { clock.2 } else { Duration::ZERO }))
    }

    /// The "Undo:" toast of `Txe` is on screen.
    fn shown_note(&self) -> bool {
        let shown = |w: &String| self.status_banner == format!("{} {w}", tr("Undo:"));
        self.relaunch_toast.is_none() && self.undo_note.as_ref().is_some_and(shown)
    }

    /// Desktop's `undoLastToast`: the Undo of the toast on screen.
    fn shown_undo(&self) -> Option<UiMessage> {
        let t = self.shown_toast()?;
        t.buttons.iter().chain(&t.right).find(|b| b.0 == tr("Undo")).map(|b| b.1.clone())
    }

    fn shown_toast(&self) -> Option<&Toast> {
        self.toast.as_ref().filter(|t| self.relaunch_toast.is_none() && t.text == self.status_banner)
    }

    fn set_archived(&self, st: &mut BeeperStore, idx: usize, on: bool) -> bool {
        if on && (!st.feature("archive-as-unread").on || st.settings.on("markReadOnArchive")) {
            self.mark_read(st, idx);
        }
        st.set_chat_flag(idx, ChatFlag::Archived, Some(on))
    }

    /// Desktop's `confirmBlockUser` (true) and `confirmUnblockUser`: the dialog, with "Report spam" off.
    fn ask_block(&mut self, id: String, block: bool) {
        self.chat_menu = None;
        self.show_command_palette = false;
        self.report_spam = false;
        (self.confirm, self.delete_target) = (Some(if block { ASK_BLOCK } else { ASK_UNBLOCK }), Some(id));
    }

    /// Desktop's `setUserBlockedOrAlert` and `setUserBlocked`: a disconnected account asks to reconnect first. Then
    /// the bridge blocks or unblocks, a toast tells it with Undo when it can be undone, and a blocked request closes.
    fn block_user(&mut self, id: &str, block: bool, spam: bool) -> Task<UiMessage> {
        let mut st = self.store.write();
        let c = st.chat(id);
        if st.bridge_accounts.iter().any(|b| b.account_id == c.account_id && b.state == "BAD_CREDENTIALS") {
            (self.confirm, self.delete_target) =
                (Some(if block { ASK_BLOCK_OFFLINE } else { ASK_UNBLOCK_OFFLINE }), Some(id.into()));
            return Task::none();
        }
        let (title, request) = (c.title.clone(), block && c.is_request == Some(true));
        let can_undo = !block || (!spam && c.block & NO_UNBLOCK == 0);
        self.confirm = None;
        st.set_user_blocked(id, block, spam);
        drop(st);
        // The Undo of a blocked request opens it again, so the chat that is open stays open.
        let open = self.pane.selected_chat_id.as_deref() == Some(id);
        let selected = request && open;
        let mut undo = vec![UiMessage::BlockUser(id.into(), !block, false)];
        undo.extend(open.then(|| UiMessage::SelectChat(id.into())));
        let text = trf(if block { "%s blocked" } else { "%s unblocked" }, &[&title]);
        match can_undo {
            true => {
                let what = trf(if block { "Block %s" } else { "Unblock %s" }, &[&title]);
                self.act(what, UiMessage::Commands(undo), Some((text, None)));
            }
            false => self.show(Toast { strong: true, ..Toast::new(text) }),
        }
        if selected {
            self.leave_chat(&mut self.store.write(), None);
            self.pane.selected_chat_id = None;
        }
        self.update(UiMessage::StoreChanged)
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
    /// Desktop's `selectThreadRange` (`yot`): selects the chats from the last selected chat, or the open chat, to `id`.
    fn select_range(&mut self, id: &str) {
        let list = self.list_ids(&self.store.read());
        let from = self.selection.last().or(self.pane.selected_chat_id.as_ref()).map_or(id, String::as_str);
        let at = |c: &str| list.iter().position(|l| l == c).unwrap_or(0);
        let (a, b) = (at(from), at(id));
        let range = &list[a.min(b)..=a.max(b)];
        self.selection.retain(|c| !range.contains(c));
        self.selection.extend_from_slice(range);
        self.select_mode = false;
    }

    fn expired(&self, c: &Chat) -> bool {
        c.reminder.as_ref().is_some_and(|r| self.reminded.get(&c.id) == Some(&r.remind_at_ms))
    }

    /// Whether the chat list draws the chat muted: Desktop's `areNotificationsMuted` of the chat, its labels or its
    /// account, unless a reminder is due.
    fn list_muted(&self, st: &BeeperStore, c: &Chat) -> bool {
        st.notifications_muted(c) && !self.expired(c)
    }

    /// Desktop's `isMuted` row style, with a grey unread dot and count: a muted chat without unread mentions, or an
    /// archived chat.
    fn list_quiet(&self, st: &BeeperStore, c: &Chat) -> bool {
        self.list_muted(st, c) && c.unread_mentions == 0 || c.flags & CHAT_ARCHIVED != 0
    }

    fn row_action<'a>(&self, c: &Chat, pref: u8) -> Option<El<'a>> {
        let unread = c.is_unread();
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
        self.tip_caps(el, label, keys.into_iter().map(String::from).collect(), at)
    }

    /// [`Self::tip`] with the key caps of a hotkey from the settings.
    fn tip_caps<'a>(
        &self,
        el: El<'a>,
        label: impl iced::widget::text::IntoFragment<'a>,
        keys: Vec<String>,
        at: tooltip::Position,
    ) -> iced::widget::Tooltip<'a, UiMessage> {
        let p = &self.pal;
        let kbd = |k: String| -> El<'a> {
            let width = if k.len() == 1 { Length::Fixed(20.0) } else { Shrink };
            container(med(k, 13.0).color(p.strong))
                .center_x(width)
                .center_y(20)
                .padding([0, 2])
                .style(fill_box(p.chip, 4.0))
                .into()
        };
        let keys = (!keys.is_empty()).then(|| Row::with_children(keys.into_iter().map(kbd)).spacing(4));
        let body = row![med(label, 13.0).color(p.strong)].push(keys).spacing(8).align_y(Center);
        tooltip(el, container(body).padding([8, 11]).style(glow_box(p.card, p.shadow, 17.0, 0.0, 20.0)), at)
    }

    fn view_rail(&self) -> El<'_> {
        let p = &self.pal;
        let drag = self.rail_drag.as_ref().filter(|d| d.order.is_some());
        let rail_btn = |glyph: El<'static>, on: bool, msg: UiMessage| {
            button(container(glyph).center(36))
                .padding(0)
                .style(btn_style([if on { p.selected } else { CLEAR }, p.hover, p.selected], [p.strong; 3], 14.0))
                .on_press_maybe(drag.is_none().then_some(msg))
        };
        let quiet = self.chat_menu.is_some() || drag.is_some();
        let tip = |btn, label, keys| self.tip(btn, label, keys, tooltip::Position::Right);
        let hotkey = |n: usize| -> Vec<&'static str> {
            ["1", "2", "3", "4", "5", "6", "7", "8", "9"].get(n).map_or(vec![], |d| vec!["Ctrl", "Alt", d])
        };
        let st = self.store.read();
        let numbers = st.settings.on("showUnreadCount");
        let badge = |btn: El<'static>, n: usize, number: bool| -> El<'static> {
            let dot = match number && numbers {
                true => container(semi(unread_label(n), 10.0).color(Color::WHITE))
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
                && (c.unread_mentions > 0 || !st.notifications_muted(c))
        };
        let tally = |key: &str, f: &dyn Fn(&Chat) -> bool| {
            self.count(key, || st.chats.iter().filter(|c| countable(c) && f(c)).count())
        };
        let tab =
            |id: &'static str, key: usize, glyph: El<'static>, t: InboxTab, n: usize, number: bool| -> El<'static> {
                let on = self.active_tab == t && self.selected_account_filter.is_none();
                let btn = badge(rail_btn(glyph, on, UiMessage::SelectTab(t)).into(), n, number);
                let view = RAIL_VIEWS.iter().find(|v| v.1 == Some(t)).expect("a sidebar view").0;
                let btn: El = mouse_area(btn).on_right_press(UiMessage::OpenChatMenu(format!("account:{view}"))).into();
                let btn = if quiet { btn } else { tip(btn, t.label(), hotkey(key)).into() };
                container(btn).id(id).into()
            };
        let requests = self.count("requests", || {
            st.chats.iter().filter(|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_UNREAD != 0).count()
        });
        let g = |c: char| -> El<'static> { ic(c, 24.0).color(p.strong).into() };
        let ring_color = if self.dark { Color::WHITE } else { Color::BLACK };
        let ring = svg(DASHED_RING.clone())
            .width(32)
            .height(32)
            .opacity(0.15_f32)
            .style(move |_, _| svg::Style { color: Some(ring_color) });
        let add_space =
            container(stack![ring, container(ic(icon::MAPS_UGC, 24.0).color(p.strong)).center(32)]).center(36);
        let account = |i: usize, acc: String| -> El<'_> {
            let on = self.selected_account_filter.as_ref() == Some(&acc);
            let space = network_space(&st, &acc);
            let owner = space.map_or(acc.as_str(), |n| &n.account_id);
            let label = rail_label(&st, &acc);
            let logo: El<'static> = match label {
                Some(l) => label_icon(l, 28.0, p.strong),
                None => svg(brand_svg(brand_key(owner))).width(28).height(28).into(),
            };
            let n = tally(&format!("rail:{acc}"), &|c| {
                in_rail_entry(&st, &acc, c) && c.flags & CHAT_HIDDEN != 0 && c.flags & CHAT_LOW_PRIORITY == 0
            });
            let label = match (label, space) {
                (Some(l), _) => l.name.clone(),
                (None, Some(n)) => format!("{} ({})", n.name, network_label(owner)),
                (None, None) => network_label(owner).to_string(),
            };
            let error = space_error(&st, &acc);
            let label = match error {
                Some(s) => format!("{label} ({s})"),
                None => label,
            };
            let btn = rail_btn(logo, on, UiMessage::SelectAccount((!on).then_some(acc.clone()))).into();
            let btn = match error {
                Some(_) => {
                    let corner = container(svg(ERROR_CIRCLE.clone()).width(14).height(14));
                    stack![btn, corner.align_right(36).padding(Padding::ZERO.top(2.0).right(2.0))].into()
                }
                None => badge(btn, n, true),
            };
            let btn: El = mouse_area(btn).on_right_press(UiMessage::OpenChatMenu(format!("account:{acc}"))).into();
            if quiet {
                btn
            } else {
                self.tip(btn, label, hotkey(i), tooltip::Position::Right).into()
            }
        };
        let labels = st.feature("labels-v2");
        let add_label = button(add_space)
            .padding(0)
            .style(ghost(p, 12.0))
            .on_press_maybe(labels.on.then(|| UiMessage::OpenCreateLabel(Vec::new())));
        let mut add_label = Some(add_label);
        let draft = self.label_window.as_ref().filter(|d| d.label.is_none());
        let draft_btn = |i: usize, d: &LabelDraft| -> El<'static> {
            let (l, fg) =
                (Label { name: d.name.clone(), color: d.color, icon: d.icon.clone(), ..Label::default() }, p.strong);
            let ring = svg(DRAFT_RING.clone()).width(30).height(30).style(move |_, _| svg::Style { color: Some(fg) });
            let logo = container(stack![ring, container(label_icon(&l, 28.0, fg)).center(30)]).into();
            let btn: El = mouse_area(rail_btn(logo, false, UiMessage::OpenCreateLabel(Vec::new())))
                .on_right_press(UiMessage::OpenCreateLabel(Vec::new()))
                .into();
            if quiet {
                btn
            } else {
                self.tip(btn, l.name, hotkey(i), tooltip::Position::Right).into()
            }
        };
        let ids = match drag.and_then(|d| d.order.clone()) {
            Some(order) => order,
            None => draft.map_or_else(|| rail_spaces(&st), |d| draft_rail_spaces(&st, &d.name)),
        };
        let mut lifted = None;
        let spaces = ids.into_iter().enumerate().map(|(i, id)| {
            let el = match id.as_str() {
                "inbox" => tab(
                    "rail-inbox",
                    i,
                    g(icon::INBOX),
                    InboxTab::AllChats,
                    tally("rail:inbox", &|c| c.flags & (CHAT_HIDDEN | CHAT_LOW_PRIORITY) == 0),
                    true,
                ),
                "archive" => tab("rail-archive", i, g(icon::ARCHIVE), InboxTab::Archived, 0, false),
                LABEL_NUDGE => tip(add_label.take().expect("one nudge").into(), tr("Create Label"), vec![]).into(),
                LABEL_DRAFT => draft_btn(i, draft.expect("an open Create Label")),
                "reminders" => tab("rail-reminders", i, g(icon::SCHEDULE), InboxTab::Reminders, 0, false),
                "scheduled" => tab("rail-send-later", i, g(icon::SCHEDULE_SEND), InboxTab::SendLater, 0, false),
                "requests" => tab("rail-requests", i, g(icon::EMAIL), InboxTab::Requests, requests, false),
                "low_priority" => tab(
                    "rail-low-priority",
                    i,
                    g(icon::LOW_PRIORITY_SPACE),
                    InboxTab::LowPriority,
                    tally("rail:low", &|c| c.flags & CHAT_LOW_PRIORITY != 0),
                    true,
                ),
                _ => account(i, id.clone()),
            };
            let el = match drag {
                Some(d) if d.id == id => {
                    lifted = Some(el);
                    Space::new().width(36).height(36).into()
                }
                _ => el,
            };
            let hover = id.clone();
            mouse_area(el)
                .on_move(move |at| UiMessage::RailHover(hover.clone(), at))
                .on_exit(UiMessage::RailLeave(id))
                .into()
        });
        let top: Vec<El> = spaces.collect();
        let lifted = lifted.zip(drag).map(|(el, d)| {
            let (x, y) = ((d.at.x - d.grab.x).clamp(0.0, 32.0), (d.at.y - d.grab.y).max(0.0));
            container(el).padding(Padding::ZERO.left(x).top(y))
        });
        let top = Column::with_children(top)
            .push(badge(
                rail_btn(g(icon::MORE_HORIZ), false, UiMessage::OpenChatMenu(RAIL_MORE.into())).into(),
                if hidden_spaces(&st).iter().any(|s| s == "requests") { requests } else { 0 },
                false,
            ))
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
        let rail = container(column![scroll, footer].align_x(Center)).width(68).height(Fill);
        let rail = mouse_area(rail.style(fill_box(p.hover, 0.0)))
            .on_right_press(UiMessage::OpenChatMenu("account:inbox".into()));
        stack![rail].push(lifted).into()
    }

    fn view_sidebar(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let w = self.list_width(&st);
        let chats = self.visible_chats(&st);
        let h24 = st.settings.pref("timeFormat") == 1;
        let compact = st.settings.on("compactChatList");
        let (av, lines, title_size, ts_size, pin_av) =
            if compact { (28.0, 1, 13.0, 10.0, 40.0) } else { (46.0, 2, 14.0, 11.0, 52.0) };
        let names: Vec<&str> =
            (0..FILTERS.len()).filter(|i| self.filters & 1 << i != 0).map(|i| tr(FILTERS[i].0)).collect();
        let heading = self
            .selected_account_filter
            .as_deref()
            .map_or(self.active_tab.label().to_string(), |id| space_name(&st, id));
        let heading = semi(heading, 13.0).color(p.strong).wrapping(iced::widget::text::Wrapping::None);
        // Desktop's space icon before the title when the space bar is hidden: a label with a color, a network space,
        // or an account with another account on its network.
        let shown = st.settings.on("showSidebar");
        let space_icon = self.selected_account_filter.as_deref().filter(|_| !shown).and_then(|id| -> Option<El<'_>> {
            if let Some(l) = rail_label(&st, id) {
                return l.color.is_some().then(|| label_icon(l, 16.0, p.strong));
            }
            let owner = rail_owner(&st, id);
            let twins = st.accounts.iter().filter(|a| brand_key(&a.account_id) == brand_key(&owner)).count() > 1;
            let icon = svg(brand_svg(brand_key(&owner))).width(16).height(16);
            (network_space(&st, id).is_some() || twins).then(|| icon.into())
        });
        let line = row![].push(space_icon.map(|i| container(i).padding(Padding::ZERO.right(4.0)))).push(heading);
        let title = column![line.push(ic(icon::UNFOLD_MORE, 12.0).color(p.weak)).spacing(1.4).align_y(Center)]
            .push((!names.is_empty()).then(|| txt(names.join(", "), 10.0).color(p.sub)))
            .padding([0, 8]);
        let entry = self.selected_account_filter.clone().unwrap_or_else(|| {
            RAIL_VIEWS.iter().find(|v| v.1 == Some(self.active_tab)).expect("a sidebar view").0.to_string()
        });
        let title = mouse_area(title)
            .on_press(UiMessage::OpenChatMenu(SPACE_SHEET.into()))
            .on_right_press(UiMessage::OpenChatMenu(format!("head:{entry}")));
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
        let (mut pinned, rest) = self.pins_and_rows(&st);
        // The order of a pin drag, which moves the pins under the pointer.
        let dragged = self.pin_drag.as_ref().filter(|d| d.order.is_some()).map(|d| d.id.as_str());
        if let Some(order) = self.pin_drag.as_ref().and_then(|d| d.order.as_ref()) {
            pinned.sort_by_key(|c| order.iter().position(|o| *o == c.id).unwrap_or(usize::MAX));
        }
        let counts = st.settings.on("showUnreadCount");
        let network_icons = st.settings.on("showNetworkIcons")
            && self.selected_account_filter.as_deref().is_none_or(|id| rail_label(&st, id).is_some());
        let colorize = st.settings.on("colorizeNetworkIcons");
        let hover_action = st.settings.pref("chatActionButton");
        let selecting = self.select_mode || !self.selection.is_empty();
        let picked = |c: &Chat| self.selection.contains(&c.id);
        let pick = |c: &Chat| match selecting {
            true => UiMessage::ToggleSelected(c.id.clone()),
            false => UiMessage::ClickChat(c.id.clone()),
        };
        let bar_pad = if SOLID_BARS { 0.0 } else { 4.0 };
        let inner = w - 8.0 - bar_pad - GUTTER;
        let cols = pin_columns(inner, pinned.len());
        let cell = inner / cols as f32;
        let tile = cell.min(120.0);
        let previews = st.settings.on("showMessagePreviews");
        let now = now_unix_millis();
        let launched = now - self.started.elapsed().as_millis() as i64;
        let pin_row = |chunk: &[&Chat]| {
            El::from(chunk.iter().fold(Row::new(), |r, c| {
                let on = selected == Some(c.id.as_str());
                let (muted, quiet) = (self.list_muted(&st, c), self.list_quiet(&st, c));
                let unread = c.is_unread();
                let (bg, fg) = if quiet { (p.card, p.strong) } else { (p.accent, Color::WHITE) };
                let dot =
                    (!counts && unread).then(|| container(Space::new()).width(6).height(6).style(fill_box(bg, 3.0)));
                let title_w = tile - if muted { 35.2 } else { 20.0 } - if dot.is_some() { 9.2 } else { 0.0 };
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
                let bubble = bubble_text(c, launched, now).filter(|_| previews && !on && !selecting);
                let unread = unread.filter(|_| bubble.is_none());
                let face = stack![container(self.chat_avatar(c, pin_av, 1.25)).center_x(pin_av + 4.0)]
                    .push((!selecting).then(|| container(unread).width(pin_av + 4.0).align_right(pin_av + 4.0)))
                    .push(selecting.then(|| pin(checkbox(p, picked(c))).x(pin_av - 11.0).y(-5.0)))
                    .width(pin_av + 4.0)
                    .height(pin_av);
                let item =
                    column![face, label].spacing(if compact { 4 } else { 8 }).align_x(Center).width(tile).padding(
                        match compact {
                            true => Padding::new(10.0).top(8.0).bottom(6.0),
                            false => Padding::new(12.0).bottom(10.0),
                        },
                    );
                let colors = if muted { (p.card, p.strong) } else { (p.accent, Color::WHITE) };
                let bubble = bubble.map(|t| pin_bubble(&t, c.unread_mentions > 0, colors, tile, pin_av, compact));
                let item = stack![item].push(bubble);
                // Desktop's `useDropPlaceholder`: the dragged pin marks the place it would drop at.
                let held = dragged == Some(c.id.as_str());
                let item = button(item)
                    .padding(0)
                    .style(btn_style([if on || held { p.selected } else { CLEAR }; 3], [p.text; 3], 8.0))
                    .on_press(pick(c));
                let item = mouse_area(item)
                    .on_right_press(UiMessage::OpenChatMenu(c.id.clone()))
                    .on_enter(UiMessage::PinHover(Some(c.id.clone())))
                    .on_exit(UiMessage::PinHover(None));
                r.push(container(item).center_x(cell))
            }))
        };
        let lead = match (selecting, counts) {
            (true, _) => 16.0,
            (false, true) => 0.0,
            (false, false) => 12.0,
        };
        let text_w = inner - 16.0 - av - 10.0 - lead;
        // Desktop's order: the regular-style pinned rows, then the new-chat drafts, then the other chats.
        let lead = rest.iter().take_while(|c| c.flags & CHAT_PINNED != 0).count();
        let drafts = self.new_chats.len();
        // Desktop hides the divider of the selected row and of the row above it.
        let draft = self.creating_chat.and_then(|n| self.new_chats.iter().position(|d| d.id == n)).map(|i| lead + i);
        let chat =
            || selected.and_then(|s| rest.iter().position(|c| c.id == s)).map(|i| i + drafts * usize::from(i >= lead));
        let above = draft.or_else(chat).and_then(|i| i.checked_sub(1)).map(|i| match i.checked_sub(lead) {
            Some(d) if d < drafts => std::borrow::Cow::Owned(format!("draft:{}", self.new_chats[d].id)),
            Some(d) => std::borrow::Cow::Borrowed(rest[lead + d - drafts].id.as_str()),
            None => std::borrow::Cow::Borrowed(rest[i].id.as_str()),
        });
        let frame =
            |key: String, dot: Option<El<'static>>, face: El<'static>, lines: [El<'static>; 2], on: bool, select| {
                let [top, preview] = lines;
                let item = row![]
                    .push(dot)
                    .push(container(face).width(av + 10.0))
                    .push(column![top, preview].width(Fill))
                    .align_y(Center)
                    .padding(Padding::new(8.0).top(9.0).bottom(11.0).left(if selecting { 0.0 } else { 8.0 }));
                let action = (!selecting && self.hovered_chat.as_ref() == Some(&key)).then(|| {
                    match key.strip_prefix("draft:") {
                        Some(n) => Some(self.hover_btn(
                            5,
                            20.0,
                            tr("Delete Chat"),
                            vec![],
                            UiMessage::DeleteNewChat(n.parse().unwrap()),
                        )),
                        None => {
                            self.row_action(&st.chats[st.resolve_chat_idx(&key).expect("listed chat")], hover_action)
                        }
                    }
                });
                let rule = (!on && above.as_deref() != Some(key.as_str()))
                    .then(|| container(divider(p.divider, av + 18.0)).align_bottom(Fill));
                let row = mouse_area(
                    button(stack![item].push(action.flatten()).push(rule))
                        .padding(0)
                        .width(Fill)
                        .style(btn_style([if on { p.selected } else { CLEAR }; 3], [p.text; 3], 8.0))
                        .on_press(select),
                )
                .on_enter(UiMessage::HoverChat(key.clone(), true))
                .on_exit(UiMessage::HoverChat(key.clone(), false));
                let inbox = st.resolve_chat_idx(&key).is_some_and(|i| st.chats[i].flags & CHAT_ARCHIVED == 0);
                match inbox {
                    true => row.on_middle_press(UiMessage::ToggleFlag(key.clone(), ChatFlag::Archived)),
                    false => row,
                }
                .on_right_press(UiMessage::OpenChatMenu(key))
            };
        let draft_row = |d: &NewChat| {
            let n = d.id;
            let dot = (!counts).then(|| container(Space::new()).width(12).into());
            let names: Vec<&str> = d.users.iter().map(|(_, u)| u.full_name.as_deref().unwrap_or(&u.id)).collect();
            let title = if names.is_empty() { tr("New Chat").into() } else { names.join(", ") };
            let title = ell(&title, text_w, title_size, if counts { Weight::Medium } else { Weight::Semibold });
            let rows =
                [container(title.color(p.strong)).height(16).into(), Space::new().height(16 * lines as u32).into()];
            let face = self.face(
                d.users.first().and_then(|(_, u)| u.img_url.as_deref()),
                names.first().unwrap_or(&""),
                av,
                1.25,
            );
            let (key, on) = (format!("draft:{n}"), self.creating_chat == Some(n));
            let picked = self.new_chat_account.as_ref().filter(|_| on);
            let face = match d.users.first().map(|(a, _)| a).or(picked).filter(|_| network_icons) {
                Some(account) => {
                    let ring = if on { crate::style::flat(p.window, p.selected) } else { p.window };
                    badged(face, account, av, if compact { 12.0 } else { 16.0 }, (!colorize).then_some(p), ring)
                }
                None => face,
            };
            frame(key, dot, face, rows, on, UiMessage::SelectNewChat(n))
        };
        let chat_row = |c: &Chat| {
            let on = selected == Some(c.id.as_str());
            let unread = c.is_unread();
            let muted = self.list_muted(&st, c);
            let draft = c
                .draft
                .as_ref()
                .and_then(|d| d.text.as_deref())
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
            let m = c.preview.as_ref().filter(|_| previews && draft.is_none());
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
            let pinned = c.flag(ChatFlag::Pinned);
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
                && !m.is_some_and(Message::silent)
                && (media_tag.is_some() || body.is_some())
                && (c.chat_type == "group" || m.is_some_and(|m| network_bot(c, &m.sender_id)));
            let first =
                m.filter(|_| show_sender).map(|m| match c.participants.items.iter().find(|u| u.id == m.sender_id) {
                    Some(u) => sender_first_name(u),
                    None => m.sender_name.as_deref().and_then(|n| n.split(' ').next()).unwrap_or(&m.sender_id).into(),
                });
            let body = body.map(|t| match (media_tag.is_none(), &first) {
                (true, Some(s)) => format!("{s}: {t}"),
                _ => t,
            });
            let body = match (c.is_request == Some(true), c.chat_type == "group") {
                (false, _) => body,
                (true, false) => body.or_else(|| Some(tr("Message Request").into())),
                (true, true) => {
                    let head = match c.inviter() {
                        Some(u) => trs("Invited by %s", &[&member_name(u)]),
                        None => tr("Group Invitation").into(),
                    };
                    Some(body.map_or(head.clone(), |b| format!("{head}\n{b}")))
                }
            };
            let dot_bg = if self.list_quiet(&st, c) { Color { a: 0.3, ..p.strong } } else { p.accent };
            let count = match counts {
                true => unread.then(|| count_badge(c, dot_bg, Color::WHITE, 12.0)),
                false => (c.unread_mentions > 0).then(|| ic(icon::AT_SIGN, 12.0).color(p.accent).into()),
            };
            // Desktop's `indicators`: the unread dot, and a red dot while a send of the chat waits for Retry.
            let dot = (!counts).then(|| {
                let dot = |bg| container(Space::new()).width(6).height(6).style(fill_box(bg, 3.0));
                let errored = st.has_errored(&c.id).then(|| dot(hex(0xf44336)));
                container(column![].push(unread.then(|| dot(dot_bg))).push(errored).spacing(4)).width(12)
            });
            let count = count
                .filter(|_| self.hovered_chat.as_ref() != Some(&c.id) || self.row_action(c, hover_action).is_none());
            let sender = show_sender.then(|| preview_sender(c, m?)).flatten();
            let digits =
                if counts && c.unread_count > 0 { unread_label(c.unread_count as usize).len() as f32 } else { 0.0 };
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
            let fg = p.fg.scale_alpha(0.63);
            let receipt = m.filter(|_| receipt).map(|m| {
                let glyph = if pending(&st, m) { &PENDING } else { &SENT[usize::from(seen_by(&st, m) > 0)] };
                let icon = tinted(glyph, 12.0, fg);
                match receipt_tip(&st, c, m) {
                    Some(tip) => self.tip(icon, tip, vec![], tooltip::Position::Top).into(),
                    None => icon,
                }
            });
            let face =
                sender.map(|u| self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 12.0, 0.5));
            let badge = (expired && previews).then(|| {
                let label = semi(tr("Reminder"), 11.0).color(Color::WHITE.scale_alpha(0.7));
                container(label).padding([0, 3]).style(fill_box(ORANGE.scale_alpha(0.7), 4.0))
            });
            let preview = row![]
                .push(badge)
                .push(receipt)
                .push(face)
                .push(prefix.map(|s| txt(s, 13.0).color(fg)))
                .push(media_tag.map(|(icon, label, _)| -> El<'_> {
                    if expired {
                        return txt(label, 13.0).color(fg).into();
                    }
                    let glyph = tinted(tag_glyph(icon), 12.0, fg);
                    let label = med(label, 10.0).color(fg);
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
            let second = second.map(|r| txt(r.to_string(), 13.0).font(body_font).color(fg));
            let preview = column![preview].push(second);
            let avatar = self.chat_avatar(c, av, 1.25);
            let avatar = match network_icons {
                true => {
                    let ring = if on { crate::style::flat(p.window, p.selected) } else { p.window };
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
            frame(c.id.clone(), dot, face.into(), [top.into(), preview.height(16 * lines as u32).into()], on, pick(c))
        };
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
            let tiles = pinned.len().div_ceil(cols);
            let row_keys = self.row_keys(&rest);
            // Desktop's "Unread threads" pill comes first.
            let threads = self.unread_threads(&st, true);
            let pre = usize::from(!threads.is_empty());
            let keys: Vec<u64> = (0..pre)
                .map(|_| vlist::key("unread-threads"))
                .chain((0..tiles).map(|i| vlist::key(("pins", i))))
                .chain(row_keys[..lead].iter().copied())
                .chain(self.new_chats.iter().map(|d| vlist::key(("draft", d.id))))
                .chain(row_keys[lead..].iter().copied())
                .collect();
            let estimate = if compact { 52.0 } else { 68.0 };
            let rows = self.chat_list.view(&keys, false, estimate, UiMessage::Relist, |i| match i.checked_sub(pre) {
                None => self.unread_threads_pill(&st, &threads),
                Some(i) if i < tiles => pin_row(&pinned[i * cols..(i * cols + cols).min(pinned.len())]),
                Some(i) if i < tiles + lead => chat_row(rest[i - tiles]).into(),
                Some(i) if i < tiles + lead + drafts => draft_row(&self.new_chats[i - tiles - lead]).into(),
                Some(i) => chat_row(rest[i - tiles - drafts]).into(),
            });
            scrollable(container(rows).padding(Padding::ZERO.right(bar_pad + GUTTER)))
                .direction(stable_bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .into()
        };
        let pill = self.view_global_audio(&st, w - 18.0).map(|pill| {
            let at = Padding::ZERO.left(8.0).right(10.0).bottom(self.audio.bottom);
            container(pill).padding(at).height(Fill).align_bottom(Fill)
        });
        let status = self.view_network_status(&st);
        let sidebar = stack![column![header].push(status).push(list).padding(Padding::ZERO.left(8.0))].push(pill);
        let sidebar = container(sidebar).width(w).height(Fill);
        mouse_area(sidebar).on_right_press(UiMessage::OpenChatMenu(SIDEBAR_MENU.into())).into()
    }

    /// Desktop's `AppStatus` over the chat list: a `network-status` banner for each disconnected account of the
    /// selected space (`mpt`: the account of an account space, the accounts of a label's chats, or every account). A
    /// click logs it in again when its bridge asks for that, or else opens its settings. In its own space it offers
    /// "Reconnect", and elsewhere a dismiss button for the session.
    fn view_network_status(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        if !self.online {
            return None;
        }
        let space = self.selected_account_filter.as_deref();
        let own = space.filter(|id| rail_label(st, id).is_none()).map(|id| rail_owner(st, id));
        let label = space.and_then(|id| rail_label(st, id));
        let shown = |account: &str| match (&own, label) {
            (Some(o), _) => o == account,
            (None, Some(l)) => l.rooms.iter().any(|r| st.find_chat(r).is_some_and(|c| c.account_id == account)),
            (None, None) => true,
        };
        let banners: Vec<El<'_>> = (st.bridge_accounts.iter())
            .filter(|b| b.state == "BAD_CREDENTIALS" || b.remote_error.is_some())
            .filter(|b| shown(&b.account_id) && !self.dismissed_errors.contains(&b.account_id))
            .map(|b| {
                let again = reconnect(st, &b.account_id);
                let name = account_network(&b.bridge_id).display_name;
                let text = txt(format!("{name} {}", tr("disconnected")), 12.0).color(p.fg).width(Fill);
                let end: Option<El<'_>> = match own.is_some() {
                    true => again.is_some().then(|| semi(tr("Reconnect"), 11.0).color(p.accent).into()),
                    false => Some(
                        button(ic(icon::CLOSE, 14.0).color(p.sub))
                            .bare(p.sub)
                            .on_press(UiMessage::DismissAccountError(b.account_id.clone()))
                            .into(),
                    ),
                };
                let icon = svg(brand_svg(brand_key(&b.account_id))).width(22).height(22);
                let line = row![icon, text].push(end).spacing(8).align_y(Center);
                let msg = again.unwrap_or(UiMessage::OpenAccountSettings(b.account_id.clone()));
                let banner = button(line).padding([0, 8]).height(40).style(btn_style([p.hover; 3], [p.fg; 3], 12.0));
                container(banner.on_press(msg)).padding(Padding::new(8.0).top(0.0).left(0.0)).into()
            })
            .collect();
        (!banners.is_empty()).then(|| Column::with_children(banners).into())
    }

    /// Desktop's message request box (`T2t`) under the messages, with its Block, Decline and Accept buttons.
    fn view_request<'a>(&self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        if chat.is_request != Some(true) {
            return None;
        }
        let p = &self.pal;
        let (accept, decline) = request_answers(chat);
        let group = chat.chat_type == "group";
        let inviter = chat.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| !group).map(|u| {
            let id = handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone());
            id.map_or_else(|| member_name(u), |id| format!("{} ({id})", member_name(u)))
        });
        let heading = match inviter {
            _ if group => tr("Accept group invitation for \"__CHAT_TITLE__\"?").replace("__CHAT_TITLE__", &chat.title),
            Some(who) => tr("Accept message request from __INVITER__?").replace("__INVITER__", &who),
            None => tr("Accept message request?").into(),
        };
        let pill = |label: &str, fg: Color, msg| {
            let label = semi(label.to_string(), 12.0);
            button(label).padding([4, 8]).style(btn_style([p.card; 3], [fg; 3], 100.0)).on_press(msg)
        };
        let id = &chat.id;
        let block = (st.can_block(chat) && !st.user_blocked(chat)).then(|| {
            let label = trx("Block", "action to block the other person in a DM");
            pill(label, hex(0xdc362e), UiMessage::AskBlock(id.clone(), true))
        });
        let buttons = row![]
            .push(block)
            .push(decline.then(|| pill(tr("Decline"), hex(0xdc362e), UiMessage::DeleteChat(id.clone(), false))))
            .push(accept.then(|| pill(tr("Accept"), p.strong, UiMessage::AcceptRequest(id.clone()))))
            .spacing(8);
        let heading = txt(heading, 13.0).color(p.strong).width(Fill).align_x(Center);
        let body = column![heading, buttons].spacing(8).align_x(Center);
        let body = container(body).padding(12).width(Fill).max_width(388).style(fill_box(p.chip, 8.0));
        Some(container(body).padding([0, 8]).center_x(Fill).into())
    }

    /// Desktop's `StickerPicker`: the stickers of the selected tab in 4 columns, and the tabs of Recent and of each
    /// pack. Without packs and recent stickers it shows "No sticker packs yet.".
    fn view_sticker_picker<'a>(&'a self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let recent = &st.settings.recent_stickers;
        let packs = match &self.pane().stickers {
            Some(Ok(packs)) => &packs[..],
            _ => &[],
        };
        let failed = matches!(self.pane().stickers, Some(Err(_)));
        let tab = self.pane().sticker_tab.clone().or_else(|| match recent.is_empty() {
            false => Some(None),
            true => packs.first().map(|x| Some(x.id.clone())),
        });
        let shown = match &tab {
            Some(None) => &recent[..],
            Some(Some(id)) => packs.iter().find(|x| &x.id == id).map_or(&[][..], |x| &x.stickers[..]),
            None => &[],
        };
        let picture = |url: &str, w: Length, h: f32| -> El<'a> {
            match self.media_at(url) {
                Some(Ok(frames)) => anim::gif(frames, self.started, move |f| {
                    image(f).width(w).height(h).content_fit(iced::ContentFit::Contain)
                }),
                _ => Space::new().width(w).height(h).into(),
            }
        };
        // Desktop's grid: 4 columns in 393 px, less the padding of 6 and 8 px and the gaps of 8 px.
        let cell = (393.0 - 12.0 - 16.0 - 24.0) / 4.0;
        let body: El<'a> = match (self.pane().stickers.as_ref().is_some_and(|_| packs.is_empty()) || failed)
            && recent.is_empty()
        {
            true => {
                container(txt(tr("No sticker packs yet."), 13.0).color(p.muted)).padding([16, 12]).center_x(Fill).into()
            }
            false => {
                let rows = shown.chunks(4).map(|row| {
                    let cells = row.iter().map(|x| {
                        button(picture(&x.url, Fill, cell))
                            .padding(0)
                            .width(Fill)
                            .style(btn_style([CLEAR; 3], [p.text; 3], 0.0))
                            .on_press(UiMessage::PickSticker(x.clone()))
                            .into()
                    });
                    let rest = (row.len()..4).map(|_| Space::new().width(Fill).into());
                    Row::with_children(cells.chain(rest)).spacing(8).into()
                });
                let n = shown.len().div_ceil(4) as f32;
                let h = (n * cell + (n - 1.0).max(0.0) * 8.0 + 16.0).min(232.0);
                let grid = Column::with_children(rows).spacing(8).padding(8);
                scrollable(grid).height(h).width(Fill).style(scroll_style(p.scroll)).into()
            }
        };
        let tab_btn = |icon: El<'a>, on: bool, msg: UiMessage| -> El<'a> {
            let bg = if on { p.received } else { CLEAR };
            button(container(icon).center(32))
                .padding(2)
                .style(btn_style([bg, p.hover, p.selected], [p.muted; 3], 18.0))
                .on_press(msg)
                .into()
        };
        let recent_tab = (!recent.is_empty())
            .then(|| tab_btn(tinted(&TIMER, 20.0, p.muted), tab == Some(None), UiMessage::StickerTab(None)));
        let pack_tabs = packs.iter().map(|x| {
            let avatar = x.avatar.as_deref().unwrap_or(&x.stickers[0].url);
            let on = tab.as_ref().is_some_and(|t| t.as_ref() == Some(&x.id));
            tab_btn(picture(avatar, Length::Fixed(32.0), 32.0), on, UiMessage::StickerTab(Some(x.id.clone())))
        });
        let tabs = Row::new().push(recent_tab).extend(pack_tabs).spacing(4).padding([4, 6]);
        let rule = container(Space::new().width(Fill).height(1)).style(fill_box(p.border_weak, 0.0));
        let card = column![body, rule, tabs].width(393).padding(6);
        container(container(card).style(shadow_box(p.menu, p.menu_border, 8.0, 16.0))).padding([0, 8]).into()
    }

    fn view_info(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.clone().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(&cid).expect("selected chat exists")];
        let on = |v: Option<bool>| v == Some(true);
        let flag = |f: ChatFlag| UiMessage::ToggleFlag(cid.clone(), f);
        let pane_btn = |glyph: char, pressed: bool, label: &'static str, msg: Option<UiMessage>| -> El<'_> {
            let (bg, fg) = if pressed { (p.accent, Color::WHITE) } else { (CLEAR, p.strong) };
            let btn = button(container(ic(glyph, 16.0)).center(28))
                .padding(0)
                .style(btn_style([bg, if pressed { bg } else { p.chip }, bg], [fg; 3], 8.0))
                .on_press_maybe(msg);
            self.tip(btn.into(), label, vec![], tooltip::Position::Bottom).into()
        };
        let locked = on(chat.is_request);
        let pane_head = row![
            pane_btn(
                icon::NOTIFICATIONS_OFF,
                chat.flag(ChatFlag::Muted),
                tr("Mute Chat"),
                (!locked).then(|| flag(ChatFlag::Muted))
            ),
            pane_btn(icon::KEEP, chat.flag(ChatFlag::Pinned), tr("Pin Chat"), Some(flag(ChatFlag::Pinned))),
            Space::new().width(Fill),
            pane_btn(icon::CLOSE, false, tr("Close Chat Info"), Some(UiMessage::ToggleRightPanel)),
        ]
        .spacing(4)
        .padding([0, 4])
        .height(36)
        .align_y(Center);
        let edits = info_menu(&st, &format!("info:{cid}"), |a| self.can(a, "create_dm"));
        let edit = match &edits[..] {
            [Some((_, msg))] => msg.clone(),
            _ => edits.iter().flatten().any(|i| i.1.is_some()).then(|| UiMessage::OpenChatMenu(format!("info:{cid}"))),
        };
        let edit = button(semi(trx("Edit", "chat info edit button"), 11.0).color(p.accent))
            .bare(p.accent)
            .on_press_maybe(edit);
        let copy = UiMessage::OpenChatMenu(format!("copy:{cid}"));
        let title = mouse_area(
            button(semi(chat.title.clone(), 15.0).color(p.strong).center()).bare(p.strong).on_press(copy.clone()),
        )
        .on_right_press(copy);
        let meta = |s: String| {
            let copy = UiMessage::OpenChatMenu(format!("copy:{cid}\n{s}"));
            mouse_area(txt(s, 11.0).color(Color { a: 0.6, ..p.strong })).on_right_press(copy)
        };
        let other =
            chat.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| chat.chat_type == "single");
        let contact = other.map(|u| {
            row![]
                .push(handle(u).map(meta))
                .push(u.phone_number.as_deref().map(|n| meta(fmt_phone(n))))
                .push(u.email.clone().map(meta))
        });
        let originally = chat
            .personal
            .as_ref()
            .filter(|p| p.title.as_ref().is_some_and(|t| *t != p.room_title) && !p.room_title.is_empty())
            .map(|p| meta(trf("Originally %s", &[&p.room_title])));
        let details = column![self.chat_avatar(chat, 64.0, 1.25), title]
            .push(originally)
            .push(contact)
            .push(meta(network_name(&chat.account_id).to_string()))
            .spacing(4)
            .align_x(Center)
            .width(Fill);
        let can_desc = chat.edits & 1 << 2 != 0;
        let small =
            |s: &'static str, w: Weight, c: Color| crate::style::t(s, 11.0, w).line_height(Pixels(14.0)).color(c);
        let link = |label: &'static str, w: Weight, msg: UiMessage| {
            button(small(label, w, p.accent)).bare(p.accent).on_press(msg)
        };
        let desc_head = |btn: Option<_>| {
            row![small(tr("Group Description"), Weight::Normal, p.strong), Space::new().width(Fill)]
                .push(btn)
                .padding([0, 8])
                .align_y(Center)
        };
        let desc: Option<El> = match (&self.info_desc, chat.description.as_deref()) {
            _ if chat.chat_type != "group" => None,
            (Some(content), _) => {
                let done =
                    link(trx("Done", "group description submit button"), Weight::Semibold, UiMessage::SaveInfoDesc);
                let input = text_editor(content)
                    .id("info-desc")
                    .placeholder(tr("This description is public to the group"))
                    .size(11)
                    .line_height(Pixels(14.0))
                    .padding(8)
                    .style(move |_: &Theme, _| text_editor::Style {
                        background: p.chip.into(),
                        border: border::rounded(8),
                        placeholder: p.sub,
                        value: p.strong,
                        selection: p.accent.scale_alpha(0.3),
                    });
                let input = if can_desc { input.on_action(|a| UiMessage::InfoDesc(Some(a))) } else { input };
                Some(
                    column![desc_head(can_desc.then_some(done)), input]
                        .spacing(8)
                        .padding(Padding::ZERO.top(8.0))
                        .into(),
                )
            }
            (None, None) if can_desc => Some(
                container(link(tr("Add a description"), Weight::Semibold, UiMessage::InfoDesc(None)))
                    .center_x(Fill)
                    .padding(Padding::ZERO.bottom(8.0))
                    .into(),
            ),
            (None, Some(d)) => {
                let full = d.split_whitespace().collect::<Vec<_>>().join(" ");
                let clamped = crate::style::clamped(&full, 268.0, 3, 11.0, Weight::Normal);
                let more = *clamped != *full;
                let shown = if self.info_more { full.clone() } else { clamped.to_string() };
                let spans = shown.split_inclusive(' ').map(|w| match text_urls(w).first() {
                    Some(&url) => span(w.to_string()).color(p.accent).link(url.to_string()),
                    None => span(w.to_string()).color(p.strong),
                });
                let text = rich_text(spans.collect::<Vec<_>>())
                    .size(11)
                    .line_height(Pixels(14.0))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .on_link_click(UiMessage::OpenUrl);
                let edit =
                    link(trx("Edit", "group description edit button"), Weight::Semibold, UiMessage::InfoDesc(None));
                let more = more
                    .then(|| link(trx("more", "expand group description button"), Weight::Normal, UiMessage::InfoMore));
                let body =
                    container(column![text].push(more).spacing(4)).padding(8).width(Fill).style(fill_box(p.chip, 8.0));
                Some(
                    column![desc_head(can_desc.then_some(edit)), body]
                        .spacing(8)
                        .padding(Padding::ZERO.top(8.0))
                        .into(),
                )
            }
            (None, None) => None,
        };
        let adding = can_desc && chat.chat_type == "group" && self.info_desc.is_none() && chat.description.is_none();
        let header = mouse_area(stack![
            container(details).padding(Padding::new(16.0).bottom(if adding { 0.0 } else { 8.0 })),
            container(edit).align_right(Fill).padding(Padding::ZERO.top(12.0).right(8.0)),
        ])
        .on_right_press(UiMessage::OpenChatMenu(format!("info:{cid}")));
        let archived = chat.flag(ChatFlag::Archived);
        let pinned = !archived && chat.flag(ChatFlag::Pinned);
        let tool = |glyph: char, fg: Color, label: &'static str, keys, msg: Option<UiMessage>| -> El<'_> {
            let style = move |_: &Theme, s: button::Status| button::Style {
                text_color: if matches!(s, button::Status::Disabled) { fg.scale_alpha(0.2) } else { fg },
                ..Default::default()
            };
            let btn = button(container(ic(glyph, 20.0)).center_x(Fill)).padding(6).width(Fill).style(style);
            let tip = self.tip(btn.on_press_maybe(msg).into(), label, keys, tooltip::Position::Top);
            container(tip).center_x(Fill).into()
        };
        let toolbar = row![
            tool(
                if archived { icon::INBOX } else { icon::ARCHIVE },
                p.strong,
                match (pinned, archived) {
                    (true, _) => tr("Pinned chats can't be archived"),
                    (false, true) => trx("Unarchive", "button label to unarchive chat"),
                    (false, false) => trx("Archive", "button label to archive chat"),
                },
                vec!["Ctrl", "E"],
                (!pinned && !locked).then(|| flag(ChatFlag::Archived)),
            ),
            match chat.reminder.is_some() {
                true => tool(
                    icon::SCHEDULE,
                    ORANGE,
                    tr("Dismiss Reminder"),
                    vec!["Ctrl", "L"],
                    Some(UiMessage::SetReminder(cid.clone(), None)),
                ),
                false => tool(
                    icon::SCHEDULE,
                    p.strong,
                    tr("Remind Me"),
                    vec!["Ctrl", "L"],
                    Some(UiMessage::OpenChatMenu(cid.clone())),
                ),
            },
            tool(icon::SEARCH, p.strong, tr("Search"), vec!["Ctrl", "F"], Some(UiMessage::SearchRoom(cid.clone()))),
            tool(icon::EXPAND_MORE, p.strong, tr("More Options"), vec![], Some(UiMessage::OpenChatMenu(cid.clone()))),
        ]
        .align_y(Center)
        .padding([8, 12]);
        let toolbar = column![
            container(Space::new().width(Fill).height(2)).style(fill_box(Color { a: 0.1, ..p.strong }, 0.0)),
            container(toolbar).center_y(46),
        ];
        let members = (chat.chat_type == "group").then(|| self.view_members(chat));
        let media = self.view_info_media(&st, chat);
        let bookmarks = self.view_bookmarks(&st, chat);
        let feature = st.feature("labels-v2");
        let labels = feature.show.then(|| {
            let open = feature.on.then(|| UiMessage::OpenEditLabels(cid.clone()));
            let mine: Vec<&Label> =
                chat.label_ids.iter().filter_map(|id| st.labels.iter().find(|l| l.id == *id)).collect();
            let edit = (!mine.is_empty()).then(|| {
                button(semi(trx("Edit", "labels section button"), 11.0).color(p.accent))
                    .bare(p.accent)
                    .on_press_maybe(open.clone())
            });
            let head = row![txt(tr("Labels"), 11.0).color(p.strong), Space::new().width(Fill)]
                .push(edit)
                .align_y(iced::Bottom)
                .padding(8);
            let body: El = match mine.is_empty() {
                true => button(
                    container(row![ic(icon::PLUS, 16.0), txt(tr("Add to a Label"), 13.0)].spacing(4).align_y(Center))
                        .center_x(Fill),
                )
                .padding(8)
                .width(Fill)
                .style(btn_style([p.chip; 3], [p.strong; 3], 26.0))
                .on_press_maybe(open)
                .into(),
                false => grouped_rows(
                    p,
                    mine.iter()
                        .map(|l| {
                            let name = ell(&l.name, 240.0, 13.0, Weight::Normal).line_height(Pixels(16.0));
                            row![label_icon(l, 20.0, p.strong), name.color(p.strong)]
                                .spacing(8)
                                .padding(8)
                                .align_y(Center)
                                .into()
                        })
                        .collect(),
                ),
            };
            column![head, body]
        });
        let timers =
            chat.timers.as_ref().filter(|_| st.feature("enable-disappearing-message-timer-config").on).map(|t| {
                let can = t.can_set && !locked;
                let options: Vec<String> = [0].iter().chain(&t.timers).map(|&ms| timer_label(ms)).collect();
                let values: Vec<u64> = [0].into_iter().chain(t.timers.iter().copied()).collect();
                let current = timer_label(chat.disappearing.unwrap_or(0));
                let (id, fg) = (cid.clone(), if can { p.strong } else { p.sub });
                let pick = pick_list(options.clone(), Some(current), move |l| {
                    let i = options.iter().position(|o| *o == l).expect("a listed timer");
                    UiMessage::SetDisappearing(id.clone(), values[i])
                })
                .text_size(13)
                .text_line_height(Pixels(16.0))
                .handle(pick_list::Handle::None)
                .padding([8, 12])
                .width(Fill)
                .style(pick_style(p.chip, p.chip, fg, fg))
                .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border));
                let pick: El = match can {
                    true => pick.into(),
                    false => container(txt(timer_label(chat.disappearing.unwrap_or(0)), 13.0).color(fg))
                        .padding([8, 12])
                        .width(Fill)
                        .style(fill_box(p.chip, 8.0))
                        .into(),
                };
                let arrow =
                    container(tinted(&SELECT_ARROW, 10.0, fg)).align_right(Fill).center_y(Fill).padding([0, 12]);
                let pick = stack![pick, arrow];
                let about =
                    txt(tr("For added privacy, new messages disappear after the set time."), 10.0).color(p.strong);
                let head = container(txt(tr("Disappearing messages"), 11.0).color(p.strong)).padding(8);
                column![head, column![pick, container(about).padding([0, 8])].spacing(4)]
            });
        let note = text_editor(&self.info_note)
            .placeholder("Notes")
            .on_action(UiMessage::InfoNote)
            .size(13)
            .line_height(Pixels(16.0))
            .padding([8, 11])
            .min_height(128)
            .max_height(self.pane().size.height / 2.0)
            .style(move |_: &Theme, _| text_editor::Style {
                background: p.chip.into(),
                border: border::rounded(10),
                placeholder: p.sub,
                value: p.strong,
                selection: p.accent.scale_alpha(0.3),
            });
        let body = column![header]
            .push(desc)
            .push(members)
            .push(media)
            .push(bookmarks)
            .push(labels)
            .push(other.and_then(|u| self.view_contact(&st, chat, u)))
            .push(timers)
            .push(container(note).padding(Padding::ZERO.top(8.0)))
            .push(block_action(&st, chat).map(|block| {
                // Desktop's `BrandChatInfoBlockUser` row.
                let (label, fg) = match block {
                    true => (trx("Block", "action to block the other person in a DM"), p.text_danger),
                    false => (trx("Unblock", "action to unblock the other person in a DM"), p.accent),
                };
                let row = button(txt(label, 13.0).color(fg))
                    .padding([8, 11])
                    .width(Fill)
                    .style(btn_style([p.chip; 3], [fg; 3], 10.0))
                    .on_press(UiMessage::AskBlock(cid.clone(), block));
                container(row).padding(Padding::ZERO.top(8.0))
            }))
            .push((chat.member_actions & MEMBER_LEAVE != 0).then(|| {
                let leave = button(semi(tr("Leave Chat"), 14.0).line_height(Pixels(18.0)).color(p.strong))
                    .padding([0, 8])
                    .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                    .on_press(UiMessage::AskLeaveChat(cid.clone()));
                container(leave).padding(Padding::ZERO.top(8.0))
            }))
            .padding(Padding::new(8.0).top(0.0));
        let scroll = scrollable(body).direction(bar()).style(scroll_style(p.scroll)).height(Fill);
        container(container(column![pane_head, scroll, toolbar]).width(300).height(Fill).style(fill_box(p.card, 10.0)))
            .padding(Padding::new(8.0).left(0.0))
            .into()
    }

    /// Desktop's `n0e` Media section: the newest image, video and link messages, and when expanded all of them
    /// by type.
    fn view_info_media<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        const TILE: f32 = 130.0;
        let p = &self.pal;
        let mut found: Vec<&Message> = st
            .timeline(&chat.id)
            .filter(|m| m.is_deleted != Some(true))
            .filter(|m| {
                !m.links.is_empty()
                    || m.attachments.iter().any(|a| matches!(a.attachment_type.as_str(), "img" | "video"))
            })
            .collect();
        if found.is_empty() {
            return None;
        }
        found.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
        let limit = if chat.chat_type == "single" { 4 } else { 6 };
        let expandable = found.len() > limit;
        let shown = if self.info_media.is_some() { &found[..] } else { &found[..found.len().min(limit)] };
        let kinds: &[&str] = match self.info_media {
            None => &["img", "video", "audio", "unknown"],
            Some(0) => &["img"],
            Some(1) => &["video"],
            Some(2) => &["audio"],
            Some(3) => &[],
            Some(4) => &["unknown"],
            Some(n) => unreachable!("media tab {n}"),
        };
        let links = matches!(self.info_media, None | Some(3));
        let white = Color::WHITE;
        let sender = |m: &Message| -> El<'a> {
            let u = chat.participants.items.iter().find(|u| u.id == m.sender_id);
            let name = u.map(member_name).or_else(|| m.sender_name.clone()).unwrap_or_else(|| m.sender_id.clone());
            let face = container(self.face(u.and_then(|u| u.img_url.as_deref()), &name, 20.0, 0.0))
                .padding(2)
                .style(shadow_box(white, CLEAR, 12.0, 8.0));
            container(face).padding(8).into()
        };
        let file = |title: String, sub: El<'a>| -> El<'a> {
            let title = txt(crate::style::clamped(&title, TILE - 16.0, 3, 10.0, Weight::Normal).to_string(), 10.0)
                .color(p.strong);
            let bg = if self.dark { Color::BLACK } else { Color::WHITE };
            container(column![title, sub]).padding(8).align_bottom(TILE).width(TILE).style(fill_box(bg, 8.0)).into()
        };
        let tiles: Vec<El> = shown
            .iter()
            .flat_map(|m| {
                let atts = m.attachments.iter().filter(|a| kinds.contains(&a.attachment_type.as_str()));
                let atts = atts.map(|a| match a.attachment_type.as_str() {
                    "img" | "video" => self.thumb(a, TILE, 4.0),
                    _ => {
                        let kind = ell(file_kind(a.mime_type.as_deref()), TILE - 16.0, 10.0, Weight::Semibold);
                        file(
                            a.file_name.clone().unwrap_or_default(),
                            kind.line_height(Pixels(12.0)).color(p.strong).into(),
                        )
                    }
                });
                let urls = m.links.iter().filter(|_| links).map(|l| {
                    let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
                    let title = if l.title.is_empty() { host.to_string() } else { l.title.clone() };
                    let url =
                        ell(&l.url, TILE - 29.0, 10.0, Weight::Semibold).line_height(Pixels(12.0)).color(p.strong);
                    file(title, row![tinted(&MEDIA_ICONS[2], 12.0, p.strong), url].align_y(Center).into())
                });
                atts.chain(urls).map(|tile| {
                    let face = stack![tile, sender(m)].width(TILE).height(TILE);
                    button(face)
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [p.strong; 3], 4.0))
                        .on_press(UiMessage::JumpTo(chat.id.clone(), m.id.clone()))
                        .into()
                })
            })
            .collect();
        let grid: El = match tiles.is_empty() {
            true => container(txt(tr("Nothing here"), 13.0).color(p.sub)).center_x(Fill).padding([10, 0]).into(),
            false => {
                let mut tiles = tiles.into_iter();
                let mut rows = Column::new().spacing(8);
                while let Some(a) = tiles.next() {
                    rows = rows.push(row![a].push(tiles.next()).spacing(8));
                }
                rows.into()
            }
        };
        let tabs = self.info_media.map(|at| {
            let labels = MEDIA_KINDS.iter().map(|k| tr(k).to_string());
            self.segments(labels, at, |i| UiMessage::InfoMedia(Some(i))).padding(Padding::new(8.0).top(0.0))
        });
        let toggle = if self.info_media.is_some() { None } else { Some(0) };
        let link = expandable.then(|| {
            let label = tr(if self.info_media.is_some() { "Show less" } else { "Show all" });
            button(semi(label, 11.0).color(p.accent)).bare(p.accent).on_press(UiMessage::InfoMedia(toggle))
        });
        let head = row![txt(tr("Media"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(link)
            .align_y(Center)
            .padding(8);
        let head: El = match expandable {
            true => mouse_area(head).on_press(UiMessage::InfoMedia(toggle)).into(),
            false => head.into(),
        };
        let body = column![].push(tabs).push(grid).spacing(4).padding(8);
        Some(column![head, container(body).width(Fill).style(fill_box(Color { a: 0.1, ..p.strong }, 10.0))].into())
    }

    /// Desktop's `oPt` Contact section of a single chat: the phone number, email and username of the other member.
    fn view_contact<'a>(&'a self, st: &BeeperStore, chat: &Chat, u: &User) -> Option<El<'a>> {
        let p = &self.pal;
        let details = [
            ("Phone", u.phone_number.as_deref().map(fmt_phone), "phone"),
            ("Email", u.email.clone(), "email"),
            ("Username", handle(u), "username"),
        ];
        let mut items: Vec<El> = details
            .into_iter()
            .filter_map(|(title, value, kind)| {
                let title = txt(tr(title), 13.0).color(p.sub);
                let menu = UiMessage::OpenChatMenu(format!("contact:{} {kind}", chat.id));
                let value = button(txt(value?, 13.0).color(p.strong)).bare(p.strong).on_press(menu.clone());
                let value = mouse_area(value).on_right_press(menu);
                Some(column![container(title).padding([4, 0]), container(value).padding([4, 0])].into())
            })
            .collect();
        if items.is_empty() {
            return None;
        }
        let alts = if u.phone_number.is_some() { alt_chats(st, chat, u) } else { Vec::new() };
        if !alts.is_empty() {
            let accounts: Vec<String> = st.accounts.iter().map(|a| a.account_id.clone()).collect();
            let title = txt(tr("Message Other Accounts"), 13.0).color(p.sub);
            let rows = alts.iter().map(|c| {
                let icon = svg(crate::style::brand_svg(brand_key(&c.account_id))).width(16).height(16);
                let name = txt(account_display_name(st, &c.account_id, &accounts), 13.0);
                button(row![icon, name.color(p.strong)].spacing(4).align_y(Center))
                    .padding([4, 0])
                    .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                    .on_press(UiMessage::SelectChat(c.id.clone()))
                    .into()
            });
            items.push(column![container(title).padding([4, 0])].extend(rows).into());
        }
        let content =
            container(Column::with_children(items).spacing(4)).padding(8).width(Fill).style(fill_box(p.chip, 10.0));
        let head = container(txt(tr("Contact"), 11.0).color(p.strong)).padding(8);
        Some(column![head, content].into())
    }

    /// Desktop's `Kfe` Bookmarks section: the newest bookmarks of the chat first, 15 until "Show all".
    fn view_bookmarks<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        const SHOWN: usize = 15;
        let p = &self.pal;
        let loaded = st.timeline(&chat.id);
        let loaded = loaded.filter(|m| m.is_bookmarked == Some(true) && m.is_deleted != Some(true));
        let others = st.bookmarks.iter().filter(|(t, b)| b.room_id == chat.id && st.loaded(t).is_none());
        let others = others.filter_map(|(t, _)| match st.bookmarked.get(t) {
            Some(Some(m)) if m.is_deleted == Some(true) => None,
            Some(m) => Some((t.as_str(), m.as_ref())),
            None => Some((t.as_str(), None)),
        });
        let mut marked: Vec<(&str, Option<&Message>)> =
            loaded.map(|m| (m.id.as_str(), Some(m))).chain(others).collect();
        marked.sort_by_cached_key(|(t, m)| {
            std::cmp::Reverse((st.bookmarks.get(*t).map_or(i64::MAX, |b| b.order), m.map(|m| m.sort_key.clone())))
        });
        let expandable = marked.len() > SHOWN;
        let shown = if self.info_bookmarks { marked.len() } else { marked.len().min(SHOWN) };
        let (h24, dim) = (st.settings.pref("timeFormat") == 1, Color { a: 0.7, ..p.strong });
        let rows: Vec<El> = marked[..shown]
            .iter()
            .map(|&(target, m)| {
                let menu = UiMessage::OpenChatMenu(format!("bookmark:{} {target}", chat.id));
                let blank = || Space::new().width(24).height(24);
                let lines = |title: El<'a>, text: &str| {
                    let preview = ell(text, 220.0, 11.0, Weight::Normal).line_height(Pixels(14.0)).color(dim);
                    column![title, preview].spacing(2).width(Fill)
                };
                let (line, jump) = match m {
                    Some(m) => {
                        let u = chat.participants.items.iter().find(|u| u.id == m.sender_id);
                        let name =
                            u.map(member_name).or_else(|| m.sender_name.clone()).unwrap_or_else(|| m.sender_id.clone());
                        let title = row![
                            ell(&name, 160.0, 11.0, Weight::Semibold).line_height(Pixels(14.0)).color(p.strong),
                            txt(list_ts(&m.timestamp, h24), 10.0).color(dim)
                        ]
                        .spacing(6)
                        .align_y(iced::Bottom);
                        let text = preview_body(m).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                        let face = self.face(u.and_then(|u| u.img_url.as_deref()), &name, 24.0, 0.0);
                        let jump =
                            st.loaded(target).is_some().then(|| UiMessage::JumpTo(chat.id.clone(), m.id.clone()));
                        (row![face, lines(title.into(), &text)], jump)
                    }
                    None if st.bookmarked.contains_key(target) => {
                        let title = semi(tr("Message unavailable"), 11.0).color(p.strong);
                        let remove = button(ic(icon::CLOSE, 16.0).color(p.sub))
                            .padding(4)
                            .style(btn_style([CLEAR; 3], [p.sub; 3], 8.0))
                            .on_press(UiMessage::RemoveBookmark(target.into()));
                        let text = tr("It was deleted or is no longer available.");
                        (row![blank(), lines(title.into(), text), remove], None)
                    }
                    None => {
                        let title = semi(tr("Loading…"), 11.0).color(p.strong.scale_alpha(0.5));
                        (row![blank(), lines(title.into(), "")], None)
                    }
                };
                let btn = button(line.spacing(8).align_y(Center))
                    .padding(8)
                    .width(Fill)
                    .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                    .on_press_maybe(jump);
                mouse_area(btn).on_right_press(menu).into()
            })
            .collect();
        let body: El = match rows.is_empty() {
            true => container(
                column![
                    semi(tr("No Bookmarks"), 11.0).color(p.strong),
                    txt(tr("Bookmarked messages will appear here."), 10.0).color(p.sub)
                ]
                .spacing(4)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding([12, 8])
            .style(fill_box(p.chip, 8.0))
            .into(),
            false => grouped_rows(p, rows),
        };
        let toggle = expandable.then(|| {
            button(semi(tr(if self.info_bookmarks { "Show less" } else { "Show all" }), 11.0).color(p.accent))
                .bare(p.accent)
                .on_press(UiMessage::InfoBookmarks)
        });
        let head = row![txt(tr("Bookmarks"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(toggle)
            .align_y(Center)
            .padding(8);
        column![head, body].into()
    }

    /// Desktop's `YNt` Members section of a group, with its first six members and a search above more than six.
    fn view_members<'a>(&'a self, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        let shade = Color { a: 0.1, ..p.strong };
        let named = |u: &User| {
            let full = u.full_name.as_deref();
            u.custom_nickname.is_some()
                || u.username.is_some()
                || full.is_some_and(|f| {
                    !u.phone_number.as_deref().is_some_and(|n| phone_digits(f) == phone_digits(n))
                        && u.email.as_deref() != Some(f)
                })
        };
        let mut all: Vec<&User> = chat.participants.items.iter().collect();
        all.sort_by_cached_key(|u| {
            (u.is_self != Some(true), u.is_admin != Some(true), !named(u), member_name(u).to_lowercase())
        });
        let q = self.info_query.trim().to_lowercase();
        let hit = |u: &&User| {
            let fields = [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email];
            q.is_empty() || fields.iter().filter_map(|f| f.as_deref()).any(|f| f.to_lowercase().contains(&q))
        };
        let rows: Vec<&User> = all.iter().copied().filter(hit).collect();
        let invite = chat.member_actions & MEMBER_INVITE != 0;
        let expandable = rows.len() + usize::from(invite) > 6;
        let searchable = all.len() > 6;
        let shown = if self.info_all { rows.len() } else { rows.len().min(6 - usize::from(invite)) };
        let add = invite.then(|| {
            let face = row![tinted(&ADD_ACCOUNT, 28.0, p.strong), semi(tr("Add Members"), 13.0)];
            button(face.spacing(8).align_y(Center))
                .padding(4)
                .width(Fill)
                .style(btn_style([CLEAR, shade, shade], [p.strong; 3], 8.0))
                .on_press(UiMessage::AddMembers(self.info_query.clone()))
        });
        let row = |u: &User| -> El<'a> {
            let you = (u.is_self == Some(true))
                .then(|| format!(" {}", trx("(You)", "tag shown next to the current user in participant list")));
            // Desktop's tags: Exited, Invited and Admin, and an invited member's name at half opacity.
            let tags: Vec<&str> = [
                (u.has_exited == Some(true)).then(|| trx("Exited", "group participant status tag")),
                (u.is_pending == Some(true)).then(|| trx("Invited", "group participant status tag")),
                (u.is_admin == Some(true)).then(|| trx("Admin", "group participant status tag")),
            ]
            .into_iter()
            .flatten()
            .collect();
            let tag_w: f32 = tags.iter().map(|t| crate::style::text_width(t, 10.0, Weight::Semibold) + 14.0).sum();
            let fade = if u.is_pending == Some(true) { 0.5 } else { 1.0 };
            let you_w = you.as_deref().map_or(0.0, |y| crate::style::text_width(y, 13.0, Weight::Normal));
            let name = member_name(u);
            let strong = p.strong.scale_alpha(fade);
            let label =
                row![ell(&name, 224.0 - tag_w - you_w, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(strong)]
                    .push(you.map(|y| txt(y, 13.0).color(strong)));
            let tags = tags.into_iter().map(|t| -> El<'a> {
                container(crate::style::t(t, 10.0, Weight::Semibold).line_height(Pixels(12.0)).color(p.strong))
                    .padding([2, 4])
                    .style(fill_box(p.card, 4.0))
                    .into()
            });
            // iced draws no widget at half opacity, so only the name of an invited member fades.
            let line = row![self.face(u.img_url.as_deref(), &name, 28.0, 0.0), label]
                .push(Space::new().width(Fill))
                .push(Row::with_children(tags).spacing(4))
                .spacing(8)
                .align_y(Center);
            let menu = UiMessage::OpenChatMenu(format!("member:{} {}", chat.id, u.id));
            let card = UiMessage::OpenChatMenu(format!("card:{} {}", chat.id, u.id));
            let btn = button(line).padding(4).width(Fill).style(btn_style([CLEAR, shade, shade], [p.strong; 3], 8.0));
            mouse_area(btn.on_press(card)).on_right_press(menu).into()
        };
        let search = searchable.then(|| {
            let n = all.len();
            let input =
                text_input(&trn("Search %d group member", "Search %d group members", n, &[&n]), &self.info_query)
                    .on_input(UiMessage::InfoQuery)
                    .size(13)
                    .line_height(Pixels(16.0))
                    .padding([10, 12])
                    .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
            let clear = (!self.info_query.is_empty()).then(|| {
                button(ic(icon::CLOSE, 20.0).color(p.sub)).bare(p.sub).on_press(UiMessage::InfoQuery(String::new()))
            });
            let bar = row![ic(icon::SEARCH, 20.0).color(p.sub), input].push(clear).align_y(Center).padding([0, 8]);
            container(bar).padding(Padding::ZERO.top(8.0))
        });
        let toggle_label = if self.info_all { tr("Show less") } else { tr("Show all") };
        let toggle = expandable.then(|| {
            let chevron: El = match self.info_all {
                true => tinted(&MAT_EXPAND_LESS, 20.0, p.strong),
                false => ic(icon::EXPAND_MORE, 20.0).color(p.strong).into(),
            };
            let face = row![semi(toggle_label, 13.0), chevron].align_y(Center);
            button(container(face).center_x(Fill))
                .padding(2)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 9999.0))
                .on_press(UiMessage::InfoAll)
        });
        let list = Column::new()
            .push(search)
            .push(add)
            .extend(rows[..shown].iter().map(|u| row(u)))
            .push(toggle)
            .spacing(6)
            .padding(if searchable { Padding::new(8.0).top(0.0) } else { Padding::new(8.0) });
        let less = (expandable && self.info_all)
            .then(|| button(semi(tr("Show less"), 11.0).color(p.accent)).bare(p.accent).on_press(UiMessage::InfoAll));
        let head = row![txt(tr("Members"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(less)
            .align_y(Center)
            .padding(8);
        let head: El = match expandable {
            true => mouse_area(head).on_press(UiMessage::InfoAll).into(),
            false => head.into(),
        };
        column![head, container(list).style(fill_box(p.chip, 10.0))].into()
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
                .style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 4.0, 84.0))
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
                let heading = txt(m.heading(), 13.0).color(p.strong);
                let clock = match m {
                    commands::SubMenu::AddMembers => Some(tinted(&ADD_ACCOUNT, 20.0, p.strong)),
                    _ => schedule.then(|| tinted(&COMPOSER_ICONS[6], 20.0, p.strong)),
                };
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
                let note = container(txt(note, 13.0).color(p.sub)).center_y(44).padding([8, 13]);
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
            st.recent_chat_ids.iter().filter_map(|id| st.find_chat(id)).filter(|c| c.flags & CHAT_HIDDEN == 0).collect()
        } else {
            // The view shows 12 chats at most, and one more tells that there are more.
            let (order, words) = self.chat_words(&st);
            let hit = |&&i: &&usize| {
                let c = &st.chats[i];
                c.flags & CHAT_HIDDEN == 0
                    && (f.accounts.is_empty() || f.accounts.contains(&c.account_id))
                    && words[i].contains(&q)
            };
            order.iter().filter(hit).take(13).map(|&i| &st.chats[i]).collect()
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
        let hits = self.search_hits(&st, &q);
        let found: Vec<&Message> = hits.0.iter().collect();
        let media: Vec<&Message> = hits.1.iter().collect();
        let none =
            found.is_empty() && chats.is_empty() && media.is_empty() && (q.chars().count() > 1 || !f.media.is_empty());
        let heading = |label: &'static str| {
            container(txt(label, 12.0).color(p.sub)).padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        };
        let h24 = st.settings.pref("timeFormat") == 1;
        let needle = q.to_ascii_lowercase();
        let messages =
            found.iter().fold(Column::new().push((!found.is_empty()).then(|| heading(tr("Messages")))), |col, m| {
                let c = st.chat(&m.chat_id);
                let mine = m.is_sender == Some(true);
                let label = match (c.chat_type == "group", mine) {
                    (false, _) => c.title.clone(),
                    (true, true) => format!("{} • {}", c.title, chat_network(c)),
                    (true, false) => {
                        format!("{} in {} • {}", sender_label(c, m), c.title, chat_network(c))
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
            let c = st.chat(&m.chat_id);
            let card = |title: String, sub: El<'static>| -> El<'static> {
                let body =
                    column![clamp(&title, 94.0, 3, 10.0, Weight::Normal).line_height(Pixels(12.0)).color(p.text), sub];
                container(body).align_bottom(Fill).width(Fill).padding(8).style(fill_box(p.window, 8.0)).into()
            };
            let thumb = m.attachments.iter().find(|a| matches!(a.attachment_type.as_str(), "img" | "video"));
            let body: El<'_> = match (thumb, m.links.first(), m.attachments.first()) {
                (Some(a), _, _) => match a.poster_img.as_ref().or(a.src_url.as_ref()).and_then(|u| self.media_at(u)) {
                    Some(Ok(frames)) => image(frames[0].0.clone())
                        .width(Fill)
                        .height(Fill)
                        .content_fit(iced::ContentFit::Cover)
                        .border_radius(8.0)
                        .into(),
                    _ => Space::new().into(),
                },
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
                    card(name, semi(kind, 10.0).color(p.text).into())
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
            let tile = stack![container(body).width(110).height(110).style(fill_box(p.chip, 8.0))]
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
                    mouse_area(
                        button(line)
                            .height(36)
                            .padding([0, 6])
                            .width(Fill)
                            .style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0))
                            .on_press(UiMessage::ExecuteCommandChat(c.id.clone())),
                    )
                    .on_right_press(UiMessage::OpenChatMenu(c.id.clone())),
                )
                .padding([0, 6]),
            )
        });
        frame(column![inputs].push(hint).push(chips).push(empty).push((!none).then(|| {
            scrollable(hits.push(media_grid).push(messages)).direction(bar()).height(Fill).style(scroll_style(p.scroll))
        })))
    }

    /// The status toast at the bottom left of the `main` window or of the Media window.
    fn view_toast(&self, main: bool) -> Option<El<'_>> {
        let p = &self.pal;
        let left = if main { 76.0 } else { 8.0 };
        let banner = match self.relaunch_toast {
            Some(t) => trs(t, &["Beeper"]),
            None => self.status_banner.clone(),
        };
        let pill = |(kind, text): (Pill, String)| -> El<'_> {
            let icon = (kind == Pill::Syncing).then(|| tinted(&MAT_SYNC, 12.0, p.strong));
            let body = row![].push(icon).push(txt(text, 12.0).color(p.strong)).spacing(4).align_y(Center);
            let style = move |_: &Theme| container::Style {
                background: Some(p.card.into()),
                border: border::rounded(17.0),
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                    offset: Vector::ZERO,
                    blur_radius: 20.0,
                },
                ..container::Style::default()
            };
            let pill = mouse_area(container(body).padding([4, 8]).style(style));
            match kind {
                Pill::Connecting => {
                    pill.on_enter(UiMessage::PillHover(true)).on_exit(UiMessage::PillHover(false)).into()
                }
                Pill::Offline | Pill::Syncing => pill.into(),
            }
        };
        let pills = self.status_pills().into_iter().map(pill);
        let older = self.older_toasts.iter().enumerate().map(|(i, t)| self.toast_card(&t.0, Some(i)));
        let reminder = (main && self.recovery_reminder()).then(|| {
            let toast = Toast {
                icon: Some(4),
                strong: true,
                right: Some((trx("Verify", "recovery key reminder"), enter_key::KeyMsg::Open.into())),
                dismiss: Some(UiMessage::DismissRecoveryReminder),
                ..Toast::new(tr("Check if you still have access to a recovery key.").into())
            };
            self.toast_card(&toast, None)
        });
        let older = pills.chain(reminder).chain(older);
        let newest = (!banner.is_empty()).then(|| -> El<'_> {
            let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, p.sub, UiMessage::DismissBanner);
            if let Some(what) = self.undo_note.as_ref().filter(|_| self.shown_note()) {
                let glyph = container(tinted(&TOAST_ICONS[3], 16.0, p.strong)).center_x(16).center_y(20);
                let text = rich_text![span::<(), _>(tr("Undo:")).font(fw(Weight::Bold)), span(format!(" {what}"))]
                    .size(13.0)
                    .color(p.strong);
                let card = row![glyph, text, close].spacing(8).align_y(Center);
                return container(card).padding([6, 8]).style(toast_box(p.card, p)).into();
            }
            if let Some(t) = self.shown_toast() {
                return self.toast_card(t, None);
            }
            let text = txt(banner.to_string(), 13.0).color(p.strong);
            let relaunch = self.relaunch_toast.map(|_| pill_btn(p, tr("Relaunch app"), UiMessage::Relaunch));
            let card = row![text.wrapping(iced::widget::text::Wrapping::WordOrGlyph)].push(relaunch).push(close);
            let card = card.spacing(8).align_y(Center);
            container(card).padding(8).style(toast_box(p.card, p)).into()
        });
        let toasts: Vec<El> = older.chain(newest).collect();
        (!toasts.is_empty()).then(|| {
            let list = mouse_area(Column::with_children(toasts).spacing(8))
                .on_enter(UiMessage::ToastHover(true))
                .on_exit(UiMessage::ToastHover(false));
            container(list).width(Fill).align_bottom(Fill).padding(Padding::new(8.0).left(left)).into()
        })
    }

    /// Desktop's `RecoveryCodeReminder`, which the flag `personal-room-overrides` or `drafts-sync` turns on: a verified
    /// device without the recovery key asks to check it, unless the user closed the reminder in the last 7 days.
    fn recovery_reminder(&self) -> bool {
        const WEEK_MS: i64 = 7 * 86_400_000;
        let st = self.store.read();
        let (s, closed) = (&st.beeper_state, st.settings.recovery_reminder_dismissed_at);
        let flag = st.feature("personal-room-overrides").on || st.feature("drafts-sync").on;
        let recent = beeper_core::config::now_unix_millis() - closed < WEEK_MS;
        flag && s.verified && s.secrets.recovery_code.is_empty() && !recent
    }

    /// Desktop's `StatusPill` toasts, which stay while their state lasts: "Offline", "Syncing..." and "Connecting...",
    /// with each sync icon. "Connecting..." names the accounts in a transient disconnect while it has the pointer.
    fn status_pills(&self) -> Vec<(Pill, String)> {
        let st = self.store.read();
        let shown = |account: &str| match &self.selected_account_filter {
            Some(f) => rail_owner(&st, f) == account,
            None => {
                self.active_tab == InboxTab::AllChats
                    && st.chats.iter().any(|c| c.account_id == account && c.is_hidden_from_inbox != Some(true))
            }
        };
        let down = st.bridge_accounts.iter().filter(|a| a.state == "TRANSIENT_DISCONNECT" && shown(&a.account_id));
        let names: Vec<&str> = down.map(|a| network_label(&a.account_id)).filter(|_| self.online).collect();
        let connecting = match names[..] {
            [] => None,
            _ if !self.pill_hover => Some(tr("Connecting...").to_string()),
            [a, b] => Some(trs("Connecting... (%1$s & %2$s)", &[a, b])),
            _ => {
                let more = names.len().saturating_sub(3);
                let more = if more > 0 { trn(", +%d more", ", +%d more", more, &[&more]) } else { String::new() };
                Some(trs("Connecting... (%s)", &[&format!("{}{more}", names[..names.len().min(3)].join(", "))]))
            }
        };
        let offline = (!self.online).then(|| (Pill::Offline, tr("Offline").to_string()));
        let syncing = st.syncing.then(|| (Pill::Syncing, tr("Syncing...").to_string()));
        offline.into_iter().chain(syncing).chain(connecting.map(|c| (Pill::Connecting, c))).collect()
    }

    /// Desktop's `Toast`: the newest toast, or the toast at an index of `older_toasts`.
    fn toast_card<'a>(&'a self, t: &Toast, older: Option<usize>) -> El<'a> {
        let p = &self.pal;
        let (bg, fg) = match t.error {
            true => (hex(0xdc362e), Color::WHITE),
            false => (p.card, p.strong),
        };
        let btn = |(label, msg): &(&'static str, UiMessage), fg: Color| {
            let label = semi(*label, 13.0).line_height(Pixels(18.0)).color(fg);
            let action = match t.dismiss {
                Some(_) => msg.clone(),
                None => UiMessage::ToastAction(older, Box::new(msg.clone())),
            };
            button(label).bare(fg).on_press(action)
        };
        // Desktop's 20 px icon sits in a 2rem box with a -8 px margin, so the toast stays 2rem tall.
        let glyph = t.icon.map(|i| container(tinted(&TOAST_ICONS[i], 20.0, fg)).center(20));
        let text = match t.strong {
            true => semi(t.text.clone(), 13.0),
            false => txt(t.text.clone(), 13.0),
        };
        let below = (!t.buttons.is_empty()).then(|| {
            let dot = || -> El { semi(" • ", 13.0).line_height(Pixels(18.0)).color(fg).into() };
            let mut buttons = t.buttons.iter().map(|b| -> El { btn(b, fg).into() });
            let first = buttons.next().expect("a button");
            Row::with_children(std::iter::once(first).chain(buttons.flat_map(|b| [dot(), b])))
        });
        let body = column![text.color(fg).wrapping(iced::widget::text::Wrapping::WordOrGlyph)].push(below).spacing(16);
        let right = t.right.as_ref().map(|b| btn(b, p.accent));
        let dismiss = match &t.dismiss {
            Some(m) => m.clone(),
            None => older.map_or(UiMessage::DismissBanner, UiMessage::DismissOlderToast),
        };
        let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, if t.error { fg } else { p.sub }, dismiss);
        let pad = if glyph.is_some() { Padding::new(6.0).right(8.0) } else { Padding::new(8.0) };
        let lead = row![].push(glyph).push(body).spacing(6);
        let card = row![lead].push(right).push(close).spacing(8);
        let card = card.align_y(if t.buttons.is_empty() { Center } else { iced::Alignment::Start });
        let card = container(card).padding(pad).style(toast_box(bg, p));
        match t.open.clone() {
            Some(open) => mouse_area(card).on_press(open).into(),
            None => card.into(),
        }
    }

    fn view_forward<'a>(&'a self, f: &'a Forward) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let shade = Color { a: 0.1, ..p.strong };
        let strip = |content: El<'a>, r| container(content).width(Fill).padding([8, 13]).style(fill_box(p.chip, r));
        let head = row![tinted(&MAT_FORWARD, 16.0, p.strong), txt(tr("Forward message"), 14.0).color(p.strong)];
        let head = strip(head.spacing(12).align_y(Center).into(), border::Radius::default().top(18)).center_y(44);
        let pills = f.picked.iter().map(|cid| -> El<'a> {
            let c = st.chat(cid);
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
            let pick = mouse_area(pick.on_press(UiMessage::ForwardPick(c.id.clone())));
            col.push(container(pick.on_right_press(UiMessage::OpenChatMenu(c.id.clone()))).padding([0, 6]))
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
        container(body).width(602).style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 4.0, 84.0)).into()
    }

    fn view_new_chat<'a>(&'a self) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let footer = |label: &'static str, key: &'static str, press: Option<UiMessage>| {
            let a = if press.is_some() { 1.0 } else { 0.4 };
            let label = row![med(label, 13.0), txt(key, 10.0).color(Color { a: 0.8 * a, ..Color::WHITE })];
            let pill = button(label.spacing(4).align_y(Center))
                .padding([4, 8])
                .style(btn_style([Color { a, ..p.accent }; 3], [Color { a, ..Color::WHITE }; 3], 20.0))
                .on_press_maybe(press);
            row![Space::new().width(Fill), pill].padding(Padding::new(8.0).left(13.0)).align_y(Center)
        };
        let draft = self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat);
        let users = draft.map_or(&[][..], |d| &d.users);
        let group = draft.is_some_and(|d| d.group);
        let account = users.first().map(|(a, _)| a.as_str());
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
                row![tinted(&SYS_ARROW_BACK, 18.0, p.strong), txt(tr("Back"), 12.0).color(p.strong)]
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
            let (joining, error) = (self.join_room.0, self.join_room.1.as_deref());
            let input = field(tr("Room address or Matrix link"));
            let input = if joining { input } else { input.on_submit(UiMessage::JoinMatrixRoom) };
            let inputs = row![tinted(&MAT_LINK, 18.0, p.sub), input]
                .spacing(8)
                .align_y(Center)
                .padding(Padding::new(6.0).left(13.0).right(8.0));
            let error = error.map(|e| container(txt(e.to_string(), 12.0).color(p.text_danger)).padding([0, 13]));
            let ready = !joining && !self.new_chat_query.trim().is_empty();
            let label = if joining { tr("Joining...") } else { tr("Join") };
            let join = footer(label, "ENTER", ready.then_some(UiMessage::JoinMatrixRoom));
            let body = column![head, inputs].push(error).push(Space::new().height(Fill)).push(join);
            return container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0)).into();
        }
        let all = rail_accounts(&st);
        let listed: Vec<String> = match group {
            true => all.iter().filter(|a| self.can_group(a)).cloned().collect(),
            false => all.iter().filter(|a| self.can(a, "create_dm")).cloned().collect(),
        };
        let logo = |a: &str| -> El<'_> { svg(crate::style::brand_svg(brand_key(a))).width(16).height(16).into() };
        let picked = self.new_chat_account.as_deref();
        let label = picked.map_or_else(|| tr("All accounts").into(), |a| account_option(&st, a, &listed));
        let all_acc = button(
            row![]
                .push(picked.map(logo))
                .push(txt(label, 12.0).color(p.strong))
                .push(ic(icon::EXPAND_MORE, 16.0).color(p.strong))
                .spacing(4)
                .align_y(Center),
        )
        .height(24)
        .padding(Padding { top: 2.0, right: 4.0, bottom: 2.0, left: 6.0 })
        .style(btn_style([p.chip; 3], [p.strong; 3], 12.0))
        .on_press(UiMessage::ToggleNewChatAccounts);
        let options = std::iter::once(None).chain(listed.iter().map(|a| Some(a.as_str()))).map(|a| -> El<'_> {
            let label = a.map_or_else(|| tr("All accounts").into(), |a| account_option(&st, a, &listed));
            let check = (a == picked).then(|| ic(icon::CHECK, 16.0).color(p.strong));
            let line = row![]
                .push(a.map(logo))
                .push(ell(&label, 160.0, 13.0, Weight::Normal).color(p.strong).width(Fill))
                .push(check)
                .spacing(8)
                .align_y(Center);
            button(line)
                .width(Fill)
                .height(22)
                .padding([0, 8])
                .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 4.0))
                .on_press(UiMessage::NewChatAccount(a.map(str::to_string)))
                .into()
        });
        let dropdown = self.new_chat_accounts_open.then(|| {
            let menu = scrollable(Column::with_children(options.collect::<Vec<_>>())).height(Shrink);
            let menu = container(menu).width(216).max_height(300).padding(5).style(shadow_box(
                p.menu,
                p.menu_border,
                6.0,
                22.0,
            ));
            stack![
                mouse_area(Space::new().width(Fill).height(Fill)).on_press(UiMessage::ToggleNewChatAccounts),
                container(menu).align_right(Fill).padding(Padding::new(8.0).top(38.0)),
            ]
        });
        let shade = Color { a: 0.1, ..p.strong };
        let pills = users.iter().map(|(_, u)| -> El<'_> {
            let name = u.full_name.as_deref().unwrap_or(&u.id);
            let pill =
                row![self.face(u.img_url.as_deref(), name, 16.0, 0.0), semi(name.to_string(), 13.0).color(p.strong)];
            container(pill.spacing(4).align_y(Center))
                .padding(Padding::new(2.0).right(8.0))
                .style(fill_box(shade, 20.0))
                .into()
        });
        let placeholder = if users.is_empty() { tr("Search contacts") } else { "" };
        let inputs = Row::with_children(pills).push(field(placeholder)).spacing(4).align_y(Center);
        let inputs = row![ic(icon::SEARCH, 16.0).color(p.sub), inputs]
            .push(users.is_empty().then_some(all_acc))
            .spacing(8)
            .align_y(Center)
            .padding(Padding::new(6.0).top(10.0).left(13.0).right(8.0));
        let q = self.new_chat_query.trim().to_lowercase();
        let mut col = Column::new().padding(Padding::ZERO.top(3.0));
        if q.is_empty() && !group {
            let group_btn = button(
                row![ic(icon::GROUP, 20.0).color(p.accent), semi(tr("New Group"), 13.0).color(p.strong)]
                    .spacing(7)
                    .align_y(Center),
            )
            .width(Fill)
            .height(36)
            .padding([0, 11])
            .style(btn_style([p.chip, p.chip, p.selected], [p.strong; 3], 6.0))
            .on_press(UiMessage::NewGroup);
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
            let group_btn = listed.iter().any(|a| self.can_group(a)).then(|| container(group_btn).padding([0, 8]));
            col = col.push(group_btn).push(container(join_btn).padding([0, 8]));
        }
        let heading = |label: &str| {
            container(txt(label.to_string(), 12.0).color(p.sub))
                .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        };
        let item = |content: El<'a>, press: Option<UiMessage>| -> El<'a> {
            let b = button(content).height(36).padding([0, 6]).width(Fill).style(ghost(p, 8.0));
            container(b.on_press_maybe(press)).padding([0, 6]).into()
        };
        // Desktop's `CommandBarUser`.
        let person = |a: &String, u: &User| -> El<'a> {
            let small = |s: String| txt(s, 12.0).color(Color { a: 0.7, ..p.strong });
            let bare = [&u.full_name, &u.username, &u.phone_number, &u.email].iter().all(|f| f.is_none());
            let line = row![self.face(u.img_url.as_deref(), person_name(u), 28.0, 0.0)]
                .push(u.full_name.clone().or_else(|| bare.then(|| u.id.clone())).map(|n| med(n, 13.0).color(p.strong)))
                .push(u.username.as_ref().map(|n| small(format!("@{}", n.trim_start_matches('@')))))
                .push(u.phone_number.as_deref().map(|n| small(fmt_phone(n))))
                .push(u.email.clone().map(small))
                .push(Space::new().width(Fill))
                .push(svg(crate::style::brand_svg(brand_key(a))).width(16).height(16))
                .spacing(6)
                .align_y(Center);
            item(line.into(), Some(UiMessage::PickUser(a.clone(), Box::new(u.clone()))))
        };
        let lookups: Vec<(&String, &User)> = self
            .new_chat_lookup
            .iter()
            .filter(|(k, nets)| {
                k.strip_prefix("lookup:") == Some(self.new_chat_query.trim()) && nets.iter().all(|n| n.1.is_some())
            })
            .flat_map(|(_, nets)| nets.iter().filter_map(|(a, n)| Some((a, n.as_ref()?.as_ref()?))))
            .collect();
        for (a, u) in &lookups {
            col = col.push(person(a, u));
        }
        let people = if q.is_empty() { Vec::new() } else { chat_people(&st, &listed, account.or(picked), &q) };
        if !people.is_empty() {
            col = col.push(heading(tr("Chats")));
            let limit = if self.new_chat_chats_expanded { people.len() } else { 10.min(people.len()) };
            for (a, u) in people.iter().take(limit) {
                col = col.push(person(a, u));
            }
            if people.len() > 10 {
                let label = if self.new_chat_chats_expanded { tr("Show less chats") } else { tr("Show more chats") };
                col = col.push(item(txt(label, 13.0).color(p.text).into(), Some(UiMessage::ToggleNewChatChats)));
            }
        }
        let search = self.new_chat_search.as_ref().filter(|(s, ..)| !q.is_empty() && *s == self.new_chat_query.trim());
        if let Some((_, pending, found)) = search {
            col = col.push(heading(tr("Connections")));
            let mut seen = HashSet::new();
            let mut results: Vec<&(String, User)> =
                found.iter().filter(|(a, u)| seen.insert(person_key(a, u))).collect();
            results.sort_by_cached_key(|(_, u)| {
                let n = person_name(u).to_lowercase();
                (n.is_empty(), n)
            });
            let limit = if self.new_chat_results_expanded { results.len() } else { 10.min(results.len()) };
            for (a, u) in results.iter().take(limit).filter(|_| *pending == 0) {
                col = col.push(person(a, u));
            }
            if *pending > 0 {
                col = col.push(item(txt(tr("Searching..."), 13.0).color(p.sub).into(), None));
            } else if results.len() > 10 {
                let label =
                    if self.new_chat_results_expanded { tr("Show less results") } else { tr("Show more results") };
                col = col.push(item(txt(label, 13.0).color(p.text).into(), Some(UiMessage::ToggleNewChatResults)));
            }
        }
        let any = listed.iter().any(|a| self.can(a, "contact_list") || self.can(a, "any_phone"));
        let contacts: Vec<&User> = st
            .contacts
            .iter()
            .filter(|_| any)
            .filter(|u| u.full_name.as_deref().unwrap_or_default().to_lowercase().contains(&q))
            .collect();
        if lookups.is_empty()
            && people.is_empty()
            && search.is_none_or(|(_, n, f)| *n == 0 && f.is_empty())
            && contacts.is_empty()
        {
            let none = if group { tr("Start typing to search for people to add") } else { tr("No results found") };
            col = col.push(heading(none));
        }
        if !contacts.is_empty() {
            col = col.push(heading(tr("In your contacts")));
            let limit = if self.new_chat_expanded || !q.is_empty() { contacts.len() } else { 10.min(contacts.len()) };
            let mut last_cat = None;
            for u in contacts.iter().take(limit) {
                let name = u.full_name.as_deref().unwrap_or(&u.id);
                let ch = name.chars().next().unwrap_or(' ').to_ascii_uppercase();
                let cat = if ch.is_ascii_alphabetic() { ch.to_string() } else { tr("Others").to_string() };
                if last_cat.as_deref() != Some(cat.as_str()) {
                    col = col.push(heading(&cat));
                    last_cat = Some(cat);
                }
                let nets = self.contact_networks.as_ref().filter(|(c, _)| *c == u.id).map(|(_, n)| n);
                let line = row![
                    self.face(u.img_url.as_deref(), name, 28.0, 0.0),
                    semi(name.to_string(), 13.0).color(p.strong),
                    Space::new().width(Fill),
                    match nets {
                        Some(_) => tinted(&MAT_EXPAND_LESS, 20.0, p.strong),
                        None => ic(icon::EXPAND_MORE, 20.0).color(p.strong).into(),
                    }
                ]
                .spacing(8)
                .align_y(Center);
                col = col.push(item(line.into(), Some(UiMessage::ToggleContact(u.id.clone()))));
                let Some(nets) = nets else { continue };
                if nets.iter().any(|(_, n)| n.is_none()) {
                    let bone = |w: f32, r: f32| container(Space::new().width(w).height(if r > 8.0 { w } else { 16.0 }));
                    let skeleton = row![
                        bone(22.0, 11.0).style(fill_box(p.chip, 11.0)),
                        bone(100.0, 5.0).style(fill_box(p.chip, 5.0))
                    ];
                    col = col.push(item(skeleton.spacing(8).align_y(Center).into(), None));
                    continue;
                }
                let found: Vec<(&String, &User)> = nets
                    .iter()
                    .filter_map(|(a, n)| Some((a, n.as_ref()?.as_ref()?)))
                    .filter(|(a, _)| account.or(picked).is_none_or(|b| *a == b))
                    .collect();
                if found.is_empty() {
                    let none = semi(tr("No networks found"), 13.0).color(p.strong).width(Fill).align_x(Center);
                    col = col.push(item(none.into(), None));
                }
                for (a, user) in found {
                    let icon = container(svg(crate::style::brand_svg(brand_key(a))).width(20).height(20)).center(28);
                    let line = row![icon, med(account_display_name(&st, a, &all), 13.0).color(p.strong)];
                    let pick = UiMessage::PickUser(a.clone(), Box::new(user.clone()));
                    col = col.push(item(line.spacing(8).align_y(Center).into(), Some(pick)));
                }
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
        let caps = account.map(|a| &self.caps[a]["group_creation"]["group"]);
        let ready = match (group, caps) {
            (false, _) => users.len() == 1,
            (true, None) => false,
            (true, Some(g)) => {
                let fewest = g["participants"]["min_length"]
                    .as_u64()
                    .unwrap_or(u64::from(g["participants"]["required"] == true));
                let named = g["name"]["required"] != true || draft.is_some_and(|d| !d.title.trim().is_empty());
                users.len() as u64 >= fewest && named && g["avatar"]["required"] != true
            }
        };
        let start = if group { tr("Start group chat") } else { tr("Start chat") };
        let body = column![
            inputs,
            scrollable(col).direction(bar()).height(Fill).style(scroll_style(p.scroll)),
            footer(start, "ENTER", ready.then_some(UiMessage::StartChat))
        ];
        let panel = container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0));
        let named = caps.is_some_and(|g| g["name"]["allowed"] == true);
        let title = draft.filter(|_| group && named).map(|d| {
            let members: Vec<User> = d.users.iter().map(|(_, u)| u.clone()).collect();
            let face = self.people_avatar(None, "", true, &members, 28.0, 0.0);
            let name = text_input(tr("New Group Chat"), &d.title)
                .on_input(UiMessage::GroupTitle)
                .width(200)
                .size(13)
                .padding(0)
                .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
            let bg = if self.dark { Color::BLACK } else { Color::WHITE };
            container(row![face, name].spacing(8).align_y(Center)).center_x(Fill).center_y(38).style(move |_| {
                container::Style {
                    background: Some(bg.into()),
                    border: iced::border::rounded(10),
                    shadow: iced::Shadow {
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.12),
                        blur_radius: 10.0,
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
        });
        stack![column![].push(title).push(panel).spacing(8)].push(dropdown).into()
    }

    /// Desktop's `BrandSegmentControl`: the segments spread across the width, with the selected one highlighted.
    fn segments<'a>(
        &self,
        labels: impl Iterator<Item = String>,
        at: usize,
        msg: impl Fn(usize) -> UiMessage,
    ) -> container::Container<'a, UiMessage> {
        let p = &self.pal;
        let segments = labels.enumerate().flat_map(|(i, label)| {
            let label = txt(label, 11.0).color(p.text);
            let bg = if i == at { p.chip } else { CLEAR };
            let segment = button(label).padding([2, 8]).style(btn_style([bg; 3], [p.text; 3], 20.0)).on_press(msg(i));
            [(i > 0).then(|| Space::new().width(Fill).into()), Some(segment.into())].into_iter().flatten()
        });
        let bg = if self.dark { Color::BLACK } else { Color::WHITE };
        container(Row::with_children(segments).spacing(4).align_y(Center))
            .padding(4)
            .width(Fill)
            .style(fill_box(bg, 20.0))
    }

    /// Desktop's `--on-wallpaper-fg` of `chat` outside of a reply chain and of a thread pane, when its chat theme has
    /// a wallpaper.
    fn wallpaper_fg(&self, st: &BeeperStore, chat: &Chat, thread: bool) -> Option<Color> {
        let wallpaper =
            chat_theme::resolve(&st.settings.chat_appearance, Some(&chat.id), self.dark, &st.config.data_dir).0;
        wallpaper.filter(|_| !thread && self.pane().thread_root.is_none()).map(|(_, fg)| fg)
    }

    /// The color, text color and metadata color of sent bubbles in `chat`, or in the default chat theme.
    fn sent_colors(&self, st: &BeeperStore, chat: Option<&str>) -> (Color, Color, Color) {
        let p = &self.pal;
        match chat_theme::resolve(&st.settings.chat_appearance, chat, self.dark, &st.config.data_dir).1 {
            Some((bg, fg)) => (bg, fg, fg.scale_alpha(p.sent_meta.a)),
            None => (p.sent, p.sent_fg, p.sent_meta),
        }
    }

    /// Desktop's `removeOrphanedWallpapers`: deletes the photos in `chat_wallpapers` that no chat theme uses.
    fn prune_wallpapers(&self) {
        let st = self.store.read();
        let entries = match std::fs::read_dir(chat_theme::photo_dir(&st.config.data_dir)) {
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

    fn spaces_edit(&mut self, e: SpacesEdit) -> Task<UiMessage> {
        let d = self.spaces_window.as_mut().expect("the Edit Spaces window is open");
        match e {
            SpacesEdit::Search(q) => d.search = q,
            SpacesEdit::Scrolled(down) => d.scrolled = down,
            SpacesEdit::Drag(id) => d.drag = Some((id, d.order.clone())),
            SpacesEdit::Over(to) => {
                let Some((id, _)) = &d.drag else { return Task::none() };
                let from = d.order.iter().position(|o| o == id).expect("a dragged space");
                let to = d.order.iter().position(|o| *o == to).unwrap_or(d.order.len() - 1);
                let item = d.order.remove(from);
                d.order.insert(to, item);
            }
            SpacesEdit::Drop => {
                let Some((_, before)) = d.drag.take() else { return Task::none() };
                if d.order[0] == SPACES_DIVIDER && d.order.len() > 1 {
                    let inbox = d.order.iter().position(|o| o == "inbox").unwrap_or(1);
                    let first = d.order.remove(inbox);
                    d.order.insert(0, first);
                }
                if d.order == before {
                    return Task::none();
                }
                let split = d.order.iter().position(|o| o == SPACES_DIVIDER).expect("the divider");
                let order = d.order.iter().enumerate().filter(|(i, _)| *i != split);
                let order = order.map(|(i, id)| SpaceOrder { id: id.clone(), visible: i < split }).collect();
                let old = self.store.read().settings.spaces_order.clone();
                self.patch_settings(|s| s.spaces_order = order);
                self.act(tr("Reorder sidebar").into(), UiMessage::SetSpacesOrder(old), None);
            }
            SpacesEdit::Menu(id) => (d.menu, self.chat_sub) = (id.map(|id| (id, self.cursor.get())), None),
            SpacesEdit::Act(m) => {
                if !matches!(*m, UiMessage::ChatSub(_)) {
                    d.menu = None;
                }
                return self.update(*m);
            }
            SpacesEdit::NewLabel => {
                let close = window::close(d.id);
                return Task::batch([close, self.update(UiMessage::OpenCreateLabel(Vec::new()))]);
            }
        }
        Task::none()
    }

    fn view_edit_spaces<'a>(&'a self, d: &'a SpacesDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = UiMessage::Spaces;
        let search = text_input(tr("Search Spaces"), &d.search)
            .on_input(move |q| edit(SpacesEdit::Search(q)))
            .size(13)
            .line_height(Pixels(16.0))
            .padding([14, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let clear = (!d.search.trim().is_empty())
            .then(|| icon_btn(p, icon::CLOSE, 16.0, 28.0, p.sub, edit(SpacesEdit::Search(String::new()))));
        let search = row![ic(icon::SEARCH, 16.0).color(p.sub), search]
            .push(clear)
            .spacing(10)
            .align_y(Center)
            .padding(Padding::ZERO.left(16.0).right(8.0));
        let q = d.search.trim().to_lowercase();
        let split = d.order.iter().position(|o| o == SPACES_DIVIDER).expect("the divider");
        let found = |ids: &'a [String]| -> Vec<&'a String> {
            ids.iter().filter(|id| q.is_empty() || space_name(&st, id).to_lowercase().contains(&q)).collect()
        };
        let line = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.15);
        let dragged = d.drag.as_ref().map(|(id, _)| id.as_str());
        let group = |ids: Vec<&'a String>| -> Vec<El<'a>> {
            let last = ids.len().saturating_sub(1);
            ids.into_iter()
                .enumerate()
                .map(|(i, id)| {
                    let (top, bottom) = (if i == 0 { 12.0 } else { 0.0 }, if i == last { 12.0 } else { 0.0 });
                    let muted = match rail_label(&st, id) {
                        Some(l) => l.muted,
                        None => {
                            !RAIL_VIEWS.iter().any(|v| v.0 == id)
                                && !st.settings.account(&rail_owner(&st, id)).notifications
                        }
                    };
                    let gear = !RAIL_VIEWS.iter().any(|v| v.0 == id) && id != LABEL_NUDGE;
                    let room = if gear { 190.0 } else { 216.0 };
                    let name = row![ell(&space_name(&st, id), room, 13.0, Weight::Semibold).color(p.strong)]
                        .push(muted.then(|| ic(icon::NOTIFICATIONS_OFF, 12.0).color(p.strong)))
                        .spacing(2)
                        .align_y(Center);
                    let sub = space_subtitle(&st, id).map(|t| txt(t, 11.0).color(p.sub));
                    let handle = mouse_area(container(tinted(&DRAG_HANDLE, 20.0, p.sub)).center(28))
                        .interaction(iced::mouse::Interaction::Grab)
                        .on_press(edit(SpacesEdit::Drag(id.clone())));
                    let lift = dragged == Some(id.as_str());
                    let body =
                        row![container(self.space_icon(&st, id)).center(20), column![name].push(sub).width(Fill),]
                            .push(gear.then(|| {
                                mouse_area(container(ic(icon::MORE_HORIZ, 16.0).color(p.sub)).center_x(16))
                                    .interaction(mouse::Interaction::Pointer)
                                    .on_press(edit(SpacesEdit::Menu(Some(id.clone()))))
                            }))
                            .push(handle)
                            .spacing(10)
                            .align_y(Center)
                            .padding([6, 12]);
                    let fill = p.received;
                    let body = container(body).style(move |_: &Theme| container::Style {
                        background: Some(fill.into()),
                        border: iced::Border {
                            radius: border::Radius {
                                top_left: top,
                                top_right: top,
                                bottom_right: bottom,
                                bottom_left: bottom,
                            },
                            ..iced::Border::default()
                        },
                        shadow: if lift {
                            Shadow {
                                color: Color::BLACK.scale_alpha(0.15),
                                offset: Vector::new(0.0, 4.0),
                                blur_radius: 12.0,
                            }
                        } else {
                            Shadow::default()
                        },
                        ..Default::default()
                    });
                    let rule =
                        (i != last).then(|| container(Space::new()).width(Fill).height(1).style(fill_box(line, 0.0)));
                    mouse_area(column![body].push(rule)).on_enter(edit(SpacesEdit::Over(id.clone()))).into()
                })
                .collect()
        };
        let header = |label: &'static str| container(txt(label, 12.0).color(p.sub)).padding([8, 0]);
        let hidden = found(&d.order[split + 1..]);
        let empty = hidden.is_empty().then(|| {
            let fill = if dragged.is_some() { CLEAR } else { p.received };
            let msg = container(txt(tr("No Hidden Spaces"), 13.0).color(p.sub)).center_x(Fill).center_y(56).style(
                move |_: &Theme| container::Style {
                    background: Some(fill.into()),
                    border: iced::Border {
                        color: p.received,
                        width: if fill == CLEAR { 1.0 } else { 0.0 },
                        radius: 12.0.into(),
                    },
                    ..Default::default()
                },
            );
            mouse_area(msg).on_enter(edit(SpacesEdit::Over(String::new())))
        });
        let list = column![header(trx("Pinned", "edit spaces section header"))]
            .extend(group(found(&d.order[..split])))
            .push(mouse_area(header(tr("Hidden")).width(Fill)).on_enter(edit(SpacesEdit::Over(SPACES_DIVIDER.into()))))
            .push(empty)
            .extend(group(hidden))
            .padding(Padding { top: 0.0, right: 13.0, bottom: 30.0, left: 13.0 });
        let list = scrollable(list)
            .direction(Direction::Vertical(Scrollbar::hidden()))
            .height(Fill)
            .on_scroll(move |v| edit(SpacesEdit::Scrolled(v.absolute_offset().y > 0.0)));
        let shade = d
            .scrolled
            .then(|| container(Space::new()).width(Fill).height(2).style(fill_box(p.strong.scale_alpha(0.1), 0.0)));
        let card =
            container(column![search, stack![list].push(shade)]).height(Fill).clip(true).style(fill_box(p.card, 12.0));
        let new = st.feature("labels-v2").show.then(|| {
            let link = button(txt(tr("New Label"), 13.0).color(p.accent))
                .padding([4, 8])
                .style(btn_style([CLEAR; 3], [p.accent; 3], 8.0))
                .on_press(edit(SpacesEdit::NewLabel));
            container(link).center_x(Fill).padding(Padding { top: 8.0, right: 0.0, bottom: 4.0, left: 0.0 })
        });
        let head = container(semi(tr("Edit Spaces"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let window = container(column![head, card].push(new)).padding(8).style(fill_box(p.window, 0.0));
        let window = mouse_area(window).on_release(edit(SpacesEdit::Drop));
        let Some((id, at)) = &d.menu else {
            drop(st);
            return window.into();
        };
        let h = 8.0 + MENU_ITEM * rail_label(&st, id).map_or(2.0, label_rows);
        drop(st);
        let (menu, sub) = self.account_menu(id, SpaceMenu::Gear);
        let x = if at.x + 233.0 > SPACES_SIZE.width { at.x - 233.0 } else { at.x }.max(4.0);
        let y = at.y.min(SPACES_SIZE.height - h - 8.0);
        let sub_x = (x + 233.0).min(SPACES_SIZE.width - ACCOUNT_SUB_W - 4.0);
        let act = |m| UiMessage::Spaces(SpacesEdit::Act(Box::new(m)));
        let place = |m: El<'a>, x: f32| -> El<'a> {
            let m = mouse_area(m.map(act)).on_press(UiMessage::Noop);
            container(m).padding(Padding::ZERO.left(x).top(y)).into()
        };
        let close = mouse_area(container(Space::new()).width(Fill).height(Fill))
            .on_press(edit(SpacesEdit::Menu(None)))
            .on_right_press(edit(SpacesEdit::Menu(None)));
        stack![window, close, place(menu, x)].push(sub.map(|s| place(s, sub_x))).into()
    }

    /// Desktop's `getSpaceIcon`.
    fn space_icon(&self, st: &BeeperStore, id: &str) -> El<'static> {
        let p = &self.pal;
        let glyph = |c: char, size: f32| -> El<'static> { ic(c, size).color(p.strong).into() };
        match id {
            "inbox" => glyph(icon::INBOX, 16.0),
            "archive" => glyph(icon::ARCHIVE, 14.0),
            "reminders" => glyph(icon::SCHEDULE, 14.0),
            "requests" => glyph(icon::EMAIL, 16.0),
            "low_priority" => glyph(icon::LOW_PRIORITY_SPACE, 12.0),
            "scheduled" => glyph(icon::SCHEDULE_SEND, 14.0),
            LABEL_NUDGE => glyph(icon::MAPS_UGC, 16.0),
            _ => match rail_label(st, id) {
                Some(l) => label_icon(l, 20.0, p.strong),
                None => svg(brand_svg(brand_key(&rail_owner(st, id)))).width(20).height(20).into(),
            },
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
        let wallpaper = chat_theme::resolve(a, chat, self.dark, &st.config.data_dir).0;
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
            let (paint, _) = chat_theme::paint(&w, self.dark, &st.config.data_dir).expect("a known wallpaper");
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
}

pub fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let (rgba8, png) = (::image::ExtendedColorType::Rgba8, ::image::ImageFormat::Png);
    ::image::save_buffer_with_format(path, rgba, width, height, rgba8, png).expect("write png");
}

/// Desktop's default size of the main window (`Wg` and `jg`).
const MAIN_SIZE: Size = Size::new(1200.0, 800.0);
/// Desktop's `windowStateKeeper` key of the chat windows (`w2` of a window ID that starts with `thread-`).
const POP_OUT: &str = "pop-out-thread";

/// Desktop's `window.json`, or an empty state before the first save.
fn read_window_state(path: &std::path::Path) -> serde_json::Value {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).expect("a valid window state"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => panic!("read {}: {e}", path.display()),
    }
}

fn write_window_state(path: &std::path::Path, key: &str, maximized: bool, size: Size, at: Option<Point>) {
    let mut state = read_window_state(path);
    let main = &mut state["windowState"][key];
    if !maximized {
        *main = serde_json::json!({ "width": size.width, "height": size.height });
        if let Some(at) = at {
            (main["x"], main["y"]) = (at.x.into(), at.y.into());
        }
    }
    main["isMaximized"] = maximized.into();
    std::fs::write(path, state.to_string()).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Desktop's `StatusPill` toasts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pill {
    Offline,
    Syncing,
    Connecting,
}

/// "Relaunch app" closed the app; `main` starts it again.
pub static RELAUNCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The iced executor on the tokio runtime of the caller, so the GUI does not start a second runtime.
struct Executor(tokio::runtime::Handle);

impl iced::Executor for Executor {
    fn new() -> std::io::Result<Self> {
        Ok(Self(tokio::runtime::Handle::current()))
    }

    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.0.block_on(future)
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _rt = self.0.enter();
        f()
    }
}

/// Runs the GUI on the tokio runtime that the calling thread entered.
pub fn run_native_gui(store: SharedStore) -> iced::Result {
    let st = store.read();
    i18n::set(st.settings.pref("appLanguage"));
    if !st.settings.on("hwAcceleration") {
        // Desktop's `app.disableHardwareAcceleration()`: draw with the CPU renderer.
        std::env::set_var("ICED_BACKEND", "tiny-skia");
    }
    if st.settings.on("spellcheck") {
        let custom = st.config.data_dir.join("Custom Dictionary.txt");
        std::thread::spawn(|| spell::load(custom));
    }
    // Only the default data dir registers, so that test and profile launches do not take the links.
    #[cfg(target_os = "linux")]
    if st.config.profile.is_none() && st.config.data_dir == beeper_core::platform::current_desktop_data_dir(None) {
        std::thread::spawn(|| deeplink::register().unwrap_or_else(|e| eprintln!("register beeper:// links: {e}")));
    }
    drop(st);
    let app = iced::daemon(move || BeeperGuiApp::boot(store.clone()), BeeperGuiApp::update, BeeperGuiApp::view)
        .executor::<Executor>()
        .title(BeeperGuiApp::title)
        .subscription(BeeperGuiApp::subscription)
        .theme(|a: &BeeperGuiApp, _| a.iced_theme())
        .scale_factor(|a: &BeeperGuiApp, _| a.zoom)
        .style(|a: &BeeperGuiApp, _| iced::theme::Style { background_color: a.pal.window, text_color: a.pal.text })
        .default_font(fw(Weight::Normal));
    app.run()
}

/// People, each with the account to reach them on.
pub type People = Vec<(String, User)>;

/// The accounts of a New Chat contact in rail order, each with the user found on it, or `None` while it resolves.
pub type Networks = Vec<(String, Option<Option<User>>)>;

/// Desktop's draft chat: a chat in the sidebar that the New Chat view makes, with the people it picked.
pub struct NewChat {
    pub id: u32,
    /// The picked people, each with the account to reach them on.
    pub users: Vec<(String, User)>,
    /// Desktop's draft `type: "group"`: from New Group, or more than one person.
    pub group: bool,
    /// The group name of Desktop's `GroupTitleBar`.
    pub title: String,
}

/// Desktop's `Toast`, which shows while `status_banner` is its text.
#[derive(Clone)]
pub struct Toast {
    pub text: String,
    /// The index of the icon in `TOAST_ICONS`.
    pub icon: Option<usize>,
    /// Desktop's `textStyle: "strong"`.
    pub strong: bool,
    /// Desktop's `buttons`, in a row below the text.
    pub buttons: Vec<(&'static str, UiMessage)>,
    /// Desktop's `rightButton`.
    pub right: Option<(&'static str, UiMessage)>,
    /// Desktop's `onClick`.
    pub open: Option<UiMessage>,
    /// Desktop's `type: "error"`.
    pub error: bool,
    /// Desktop's `onDismiss` of a toast with `keepToastOpen` buttons: the close runs it, and the buttons do not close
    /// the toast.
    pub dismiss: Option<UiMessage>,
}

impl Toast {
    pub fn new(text: String) -> Self {
        Toast {
            text,
            icon: None,
            strong: false,
            buttons: Vec::new(),
            right: None,
            open: None,
            error: false,
            dismiss: None,
        }
    }
}

/// Desktop's `HoverCard` of a user (`hoverDelay: 300`): the card of "chat user" `key` shows where the pointer is 300 ms
/// after it rests on a trigger, and stays while the pointer is on the trigger or the card.
#[derive(Default)]
pub struct UserHover {
    pub key: String,
    pub trigger: bool,
    pub card: bool,
    pub shown: Option<Point>,
    /// Counts the changes, so that only the last wait acts.
    pub r#gen: u32,
}

/// Desktop's `setAppBadgeCount` and `setTrayUnreadStatus`: the unread count and the window state on the tray icon and
/// the launcher badge of Linux.
#[cfg(target_os = "linux")]
fn badge_sync(unread: u32, visible: bool) -> Task<UiMessage> {
    Task::future(tray::update(unread, visible)).map(Result::err).and_then(|e| Task::done(UiMessage::TrayFailed(e)))
}

/// Desktop's `setAppBadgeCount`: the unread count on the Dock icon of macOS.
#[cfg(target_os = "macos")]
fn badge_sync(unread: u32, _visible: bool) -> Task<UiMessage> {
    tray::dock_badge(unread);
    Task::none()
}

/// Electron's `app.badgeCount` shows nothing on Windows.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn badge_sync(_unread: u32, _visible: bool) -> Task<UiMessage> {
    Task::none()
}

/// Desktop's `ga`: the bounds of a window at `at` and `old` size that takes `size`, at most the `screen`, around its
/// center and on the screen.
fn grown(at: Point, old: Size, size: Size, screen: Size) -> (Point, Size) {
    let fit = Size::new(size.width.min(screen.width), size.height.min(screen.height));
    let place = |at: f32, old: f32, new: f32, room: f32| (at - (new - old) / 2.0).min(room - new).max(0.0).floor();
    (
        Point::new(place(at.x, old.width, fit.width, screen.width), place(at.y, old.height, fit.height, screen.height)),
        fit,
    )
}

/// A press on a pin of the pinned grid, which Desktop's `Drop` sorts after the cursor moves 5px: the pin, where the
/// cursor first moved, and the order of the grid while the pin moves.
pub struct PinDrag {
    pub id: String,
    pub from: Option<Point>,
    pub order: Option<Vec<String>>,
}

/// A press on a rail space, which Desktop's `Drop` sorts after the cursor moves 5px.
pub struct RailDrag {
    pub id: String,
    /// Where the press was in the space, where the cursor first moved in the window, and where it is.
    pub grab: Point,
    pub from: Option<Point>,
    pub at: Point,
    /// The rail order while the space moves, or none before the drag starts.
    pub order: Option<Vec<String>>,
}

/// The Edit Spaces window: the shown spaces, then `SPACES_DIVIDER`, then the hidden spaces.
pub struct SpacesDraft {
    pub id: window::Id,
    pub order: Vec<String>,
    pub search: String,
    /// The space that the drag handle moves, and the order before the drag.
    pub drag: Option<(String, Vec<String>)>,
    /// The space of the open gear menu, and where it opened.
    pub menu: Option<(String, Point)>,
    /// Whether the list scrolled down, so the search row casts a shadow.
    pub scrolled: bool,
}

/// The size of Desktop's Edit Spaces window.
const SPACES_SIZE: Size = Size::new(360.0, 560.0);

/// Desktop's `HIDDEN_DIVIDER_ID`.
const SPACES_DIVIDER: &str = "__hidden_divider__";

#[derive(Debug, Clone)]
pub enum SpacesEdit {
    Search(String),
    Drag(String),
    /// The dragged space moves to this space, or to the end for an empty ID.
    Over(String),
    Drop,
    NewLabel,
    /// The gear menu of a space, or no menu.
    Menu(Option<String>),
    /// A gear menu item.
    Act(Box<UiMessage>),
    Scrolled(bool),
}

#[cfg(test)]
mod fuzz;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_view::{filmstrip, slide};

    #[test]
    fn gui_tasks_run_on_the_runtime_of_the_caller() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).build().expect("build a runtime");
        let _rt = rt.enter();
        let executor = <Executor as iced::Executor>::new().expect("an executor on the entered runtime");
        let (tx, rx) = std::sync::mpsc::channel();
        iced::Executor::spawn(&executor, async move {
            tx.send(tokio::runtime::Handle::current().metrics().num_workers()).expect("the test waits");
        });
        assert_eq!(rx.recv().expect("the task runs"), 1);
    }

    #[test]
    fn ease_out_quint_matches_css() {
        assert!(ease_out_quint(0.0).abs() < 1e-3 && (ease_out_quint(1.0) - 1.0).abs() < 1e-3);
        assert!((ease_out_quint(0.5) - 0.963).abs() < 5e-3);
    }

    #[test]
    fn app_icon_is_the_desktop_linux_icon() {
        let icon = ::image::load_from_memory(include_bytes!("../assets/icon.png")).unwrap().into_rgba8();
        assert_eq!(icon.dimensions(), (512, 512));
        assert_eq!(icon.get_pixel(256, 256).0, [59, 82, 245, 255], "the blue Beeper B");
    }

    #[test]
    fn window_and_menu_sizes_match_desktop() {
        let min = BeeperGuiApp::window_settings(Size::new(1200.0, 800.0)).min_size;
        assert_eq!(min, Some(Size::new(330.0, 400.0)), "main-entry `minWidth` and `minHeight`");
        assert!((MENU_ITEM - 26.2).abs() < 1e-4, "a 24.2 px `.menu-item` and its collapsed 2 px margin");
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
        let mut png = std::io::Cursor::new(Vec::new());
        ::image::RgbaImage::new(3, 2).write_to(&mut png, ::image::ImageFormat::Png).unwrap();
        let still = decode_media(png.into_inner()).unwrap();
        assert!(matches!(still[..], [(image::Handle::Rgba { width: 3, height: 2, .. }, 0)]), "a still is decoded");
        assert!(decode_media(b"\x89PNG".to_vec()).is_err(), "a broken still fails to load");
    }

    /// A video that plays as a GIF decodes to at most 15 frames a second, at most 320 px on its long side.
    #[test]
    fn video_gifs_decode_to_frames() {
        let path = beeper_core::testing::temp_dir("video-gif").join("clip.mp4");
        let lavfi = ["-f", "lavfi", "-i", "testsrc=size=640x360:rate=30", "-t", "1", "-pix_fmt", "yuv420p", "-y"];
        assert!(std::process::Command::new("ffmpeg").args(lavfi).arg(&path).output().unwrap().status.success());
        let frames = crate::media_view::decode_loop(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(frames.len(), 15, "15 frames for one second");
        assert!(matches!(frames[0], (image::Handle::Rgba { width: 320, height: 180, .. }, 66)));
        assert!(crate::media_view::decode_loop(b"not a video").is_err());
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
        let dir = beeper_core::testing::temp_dir("unused-path");
        assert_eq!(unused_path(&dir, "a.png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), b"").unwrap();
        assert_eq!(unused_path(&dir, "a.png"), dir.join("a (1).png"));
        assert_eq!(unused_path(&dir, ".env"), dir.join(".env"));
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
        let card =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Doe;Jo;;;\r\nORG:Acme\\, Inc.\r\nitem1.TEL;type=CELL:+1555\r\n 0100\r\n\
            TEL:+15550100200\r\nEMAIL:jo@x.y\r\nEND:VCARD\r\n";
        let want = VCard {
            name: "Jo Doe".into(),
            org: Some("Acme, Inc.".into()),
            phones: vec!["+15550100".into(), "+15550100200".into()],
            emails: vec!["jo@x.y".into()],
        };
        assert_eq!(parse_vcard(card), want, "Desktop's `parseVCardAsContact`");
        assert_eq!(parse_vcard("FN:Ann\nN:B;C").name, "Ann");
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
        assert_eq!([0, 9, 29].map(|at| filmstrip(&size, at, 30, None)), [(0, 10), (4, 14), (20, 30)]);
        assert_eq!(filmstrip(&size, 1, 3, None), (0, 3));
        assert_eq!([Some(2), Some(25)].map(|from| filmstrip(&size, 0, 30, from)), [(2, 12), (20, 30)], "a scroll");
        assert_eq!(medium_date(20_743), "Oct 17, 2026");
        let widths = [50.0, 40.0, 30.0];
        let round = |(x, w): (f32, f32)| (x.round(), w.round());
        assert_eq!(
            [0.0, 1.0, 2.0].map(|t| round(slide(&widths, (0, 2), t))),
            [(0.0, 50.0), (98.0, 30.0), (98.0, 30.0)]
        );
        let (x, w) = slide(&widths, (0, 2), 0.5);
        assert!(x > 49.0 && w < 40.0, "the highlight is past the middle at half the time: {x} {w}");
    }
}

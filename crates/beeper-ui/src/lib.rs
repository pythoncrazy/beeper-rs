pub mod accounts;
mod add_account;
mod anim;
mod behavior;
mod blocking;
pub mod chat_list;
mod chat_theme;
mod chat_theme_window;
mod click;
mod command_palette;
pub mod commands;
pub mod composer;
mod cursor;
mod dashed;
mod deeplink;
mod dialogs;
pub mod emoji;
mod enter_key;
mod format;
mod forward;
mod gallery;
mod gifs;
pub mod hotkeys;
mod i18n;
mod images;
mod info;
mod integrations;
mod labels;
mod lift;
mod linkify;
mod links;
mod loads;
pub mod login;
mod media_view;
mod members;
mod menubar;
mod merged;
mod message;
pub mod networks;
pub mod new_chat;
mod nickname;
mod notify;
mod pdf;
mod pins;
mod popover;
mod portal;
mod profile;
pub mod rail;
mod reauth;
mod record;
pub mod reminders;
mod reply_line;
mod ring;
mod scheduled;
mod selectable;
pub mod settings;
mod sound;
mod spell;
mod time;
mod time_suggestions;
mod timeline;
pub mod toasts;
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
pub mod threads;
mod units;
mod user_card;
mod vcard;
mod verify;
mod video;
mod view_once;
mod vlist;
mod voice;
mod wants;
pub mod windows;
pub mod zoom;

use crate::composer::{composer_with, load_draft, save_draft, Typo};
pub use crate::composer::{LinkState, Stash, Suggestion};
pub use crate::dialogs::{
    GroupDraft, GroupEdit, LabelDraft, LabelEdit, LabelPane, LabelSettings, LabelStep, PollDraft, PollEdit,
};
use crate::dialogs::{DEFAULT_LABEL_ICON, GROUP_SIZE, LABEL_SETTINGS_SIZE};
use crate::i18n::{tr, trf, trn, trs, trx};
use crate::login::{Login, LoginMsg};
use crate::media_view::{decode_media, run_ffmpeg, Carousel, MediaWindow, TAB_SLIDE};
use crate::message::Rows;
pub use crate::message::TopOfThread;
use crate::settings::{
    hotkey_key, mcp_value, shown, AddToken, IntegrationsPage, StoragePage, EXPIRIES, HOTKEYS_RELAUNCH,
    SETTINGS_DIALOGS, STORAGE_DAYS,
};
pub use crate::settings::{search_settings, Expiry, HotkeyRec, SettingsHit, SettingsTab, TabIcon};
use crate::threads::{can_thread, Threads, THREAD_FRACTION};
use blocking::block_action;
use chat_list::{filter_hit, FILTERS, F_ARCHIVED, F_LOW_PRIORITY, F_REQUESTS};
use forward::Forward;
use info::info_menu;
use integrations::TokenAsk;
use labels::{label_fill, label_glyph, label_rows};
use new_chat::{account_display_name, phone_digits, NewChat};
use pins::{bubble_text, pin_bubble, PinDrag};
use rail::{all_spaces, RailDrag, LABEL_NUDGE, RAIL_MENU};
use rail::{
    in_rail_entry, network_space, rail_accounts, rail_label, rail_owner, rail_spaces, SpaceMenu, RAIL_MORE, RAIL_VIEWS,
};
use rail::{SpacesDraft, SpacesEdit, SPACES_DIVIDER, SPACES_SIZE};
use reminders::{remind_label, reminder_day};
use scheduled::next_send;
use time::{clock, day_label, day_num, day_of, locale_string, today, WEEKDAYS};
use toasts::{Toast, TOASTS_SHOWN, TOAST_TICK};
use units::human_size;
use user_card::UserHover;
use vcard::VCard;
use windows::{badge_sync, grown, read_window_state, write_window_state, MAIN_SIZE};

use crate::style::{
    bar, bordered_box, bordered_btn, brand_gray, brand_svg, btn_style, clamp, clamped, ell, ensure_fonts_loaded,
    fill_box, fw, glow_box, hex, ic, icon, initials, input_style, med, pick, pick_menu_style, pick_style, scroll_style,
    semi, shadow_box, stable_bar, text_width, txt, Bare, Palette, AVATAR_GRADIENTS, CLEAR, GUTTER, SOLID_BARS,
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
use beeper_core::search::{self, Search};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InboxTab {
    #[default]
    AllChats,
    LowPriority,
    Archived,
    Reminders,
    SendLater,
    Requests,
    /// Desktop's Snoozed space, while snoozing is on.
    Snoozed,
}

impl InboxTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Archived => trx("Archive", "archive space name"),
            Self::Snoozed => trx("Snoozed", "space name for snoozed chats"),
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
            ("You don't have any snoozed chats", ""),
        ][self as usize];
        (tr(head), tr(hint))
    }
}

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
    /// A virtual list must build other rows, or its scroll stopped.
    Relist,
    /// A view shows avatars or media that no task loads yet.
    LoadWanted,
    /// Desktop's toasts: the status banner and the older toasts above it, their timeout, and their buttons.
    Toast(toasts::Msg),
    /// Desktop's chat list (ThreadList): its tabs and filters, the chat menu and flags, archive and read, multi-select, and the chat actions.
    ChatList(chat_list::Msg),
    /// Desktop's space bar (rail): its spaces, their order and drags, the floating rail, and the width of the sidebar.
    Rail(rail::Msg),
    SelectChat(String),
    /// Desktop's composer.
    Composer(composer::Msg),
    /// The timeline of a chat.
    Message(message::Msg),
    Emoji(emoji::EmojiMsg),
    Upserted(Box<Message>),
    ChatUpserted(Box<Chat>),
    Typing(String),
    /// Desktop notifications.
    Notify(notify::Msg),
    /// The windows: their open, close, focus, resize and maximize, the tray, the relaunch, and a second launch of the app.
    Window(windows::Msg),
    /// Desktop's `toggleIncognitoMode` from a hotkey or the command bar, with its toast.
    ToggleIncognito,
    /// Desktop's GIF and sticker picker: the GIF search, the sticker packs, and the picked media for the composer.
    Gifs(gifs::Msg),
    /// Desktop's chat reminders: Remind me, If no reply, the due reminders and the Reminders list.
    Reminders(reminders::Msg),
    /// Desktop's `TimeSuggestionsPreference` in General settings.
    TimeSuggestions(time_suggestions::Msg),
    /// Desktop's command bar.
    CommandBar(commands::Msg),
    /// Desktop's Send Later: the time of the composer, and the scheduled messages with their menu.
    Scheduled(scheduled::Msg),
    /// Ends the typing notices 3 seconds after the last key.
    TypingTick,
    /// Desktop's throttled draft save, 5 seconds after a composer change.
    SaveDrafts,
    /// Desktop's `SwitchChatHoverCard` of the composer: shows the chats of the person on your other accounts.
    SwitchCard,
    /// Desktop's `composeSwitchTargets.onSelect`: the chat of the person on another account.
    SwitchChat(String),
    /// Desktop's threaded replies.
    Thread(threads::Msg),
    /// A message of the thread pane, which the thread pane handles.
    InThread(Box<UiMessage>),
    /// Desktop's chat themes: the wallpaper and bubble color of a chat or of the default theme.
    ChatTheme(chat_theme::Msg),
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
    /// Desktop's Chat Info panel: the description, the note, the media and bookmarks lists, the personal name and disappearing messages.
    Info(info::Msg),
    Group(GroupEdit),
    Profile(profile::Msg),
    /// Desktop's labels: their windows, the Edit Labels switches, mute, Show in Inbox, and the undo of a change or a deletion.
    Labels(labels::Msg),
    /// Desktop's `Txe`: undoes the last action, with an "Undo:" toast.
    UndoAction,
    /// Desktop's BrandUserHoverCard: the card of a user that shows after the pointer rests on their name or avatar.
    UserCard(user_card::Msg),
    /// Desktop's forward dialog: the chats to forward the selected messages to.
    Forward(forward::Msg),
    /// Desktop's Media window and full-screen carousel: the tabs, filters, selection and menus of the media of a chat.
    Media(media_view::Msg),
    Modifiers(keyboard::Modifiers),
    WindowKey(window::Id, Key, keyboard::Modifiers),
    /// Escape in a text input, which unfocuses it.
    InputEscape(window::Id),
    RunHotkey(&'static str),
    /// Runs the messages in order.
    Commands(Vec<UiMessage>),
    /// Desktop's New Chat: its drafts, the contact and user search, the networks of a contact, and the start of a chat or group.
    NewChat(new_chat::Msg),
    /// The members of a chat in Chat Info: Add members, admins, removal, and the direct chat with a member.
    Members(members::Msg),
    /// Desktop's block and unblock of a direct chat, with Report spam, and the Blocked users window.
    Blocking(blocking::Msg),
    /// Desktop's merged chats: Unmerge.
    Merged(merged::Msg),
    /// Desktop's account nicknames and the Account Nickname window.
    Nickname(nickname::Msg),
    Capabilities(String, Result<serde_json::Value, String>),
    /// Desktop's Settings window.
    Settings(settings::Msg),
    /// The chat accounts: Settings > Accounts, an account's settings and Show in Inbox, its removal, and its error banner.
    Accounts(accounts::Msg),
    /// The networks of the Add an Account window and of the onboarding, and their accounts menu.
    Networks(networks::Msg),
    ToggleRightPanel,
    ToggleDnd,
    /// Settings > Hotkeys: the hotkey switches, the shortcut recorder, and the Keyboard Shortcuts list.
    Hotkeys(hotkeys::Msg),
    /// Desktop's zoom.
    Zoom(zoom::Msg),
    /// The avatars, media, quotes and media resolves that tasks load for the views.
    Loaded(loads::Msg),
    Voice(voice::Msg),
    Record(record::Msg),
    Video(video::Msg),
    /// Desktop's view-once media: its viewer, its views and the Play queue of a chat.
    ViewOnce(view_once::Msg),
    Gallery(gallery::Msg),
    /// Opens the emoji picker for a quick reaction (see [`emoji::Picker::pref`]), or closes it.
    QuickEmoji(Option<u8>),
    AnimationTick,
    Pushed(Result<(), String>),
    /// The scroll of the timeline: the jump to its bottom, and the load of older messages at its top.
    Timeline(timeline::Msg),
    Ask(Option<u8>),
    /// The close of Desktop's recovery key reminder (`dismissRecoveryCodeReminder`).
    DismissRecoveryReminder,
    Logout,
    LoggedOut(Result<(), String>),
    /// Settings > Integrations: the Desktop API tokens and connections, Remote Access, and the MCP install.
    Integrations(integrations::Msg),
    OpenQuickLook(String, String),
    OpenUrl(String),
    /// The result of opening a URL in the browser.
    Opened(Result<(), String>),
    CloseModals,
    /// The pointer came onto (true) or left the header card of the chat, or its member list.
    HeaderHover(bool),
    /// The 500 ms after the pointer left the header card of this generation ran out.
    HeaderHoverEnd(u32),
    /// Desktop's menu bar.
    MenuBar(menubar::Msg),
    /// Desktop's pinned chats grid: the hover of a tile, and the drag that orders the pins.
    Pins(pins::Msg),
    /// A message from a chat window, handled with its pane.
    InWindow(window::Id, Box<UiMessage>),
    KeyPressed(Key, Modifiers),
    /// Text that a key typed outside of a text field in a window.
    WindowText(window::Id, String),
    StoreChanged,
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

/// Starts beeper-rs again with the same arguments and ends this one: after a sign-out, the store opens on clean data.
fn restart() -> ! {
    let exe = std::env::current_exe().expect("path of the running beeper-rs");
    std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn().expect("restart beeper-rs");
    std::process::exit(0);
}

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
    off.then(|| trf("Muted by %s", &[&behavior::account_name(st, &c.account_id)]))
}

/// Desktop's `userDisplayText`: the nickname of a chat account (`customName`), or its user.
fn account_user(st: &BeeperStore, account_id: &str) -> String {
    st.custom_name(account_id).map_or_else(|| default_account_user(st, account_id), str::to_string)
}

/// Desktop's `defaultUserDisplayText`: the user of a chat account, as the account's network shows them.
fn default_account_user(st: &BeeperStore, account_id: &str) -> String {
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

/// Desktop's `Up`: the network of an account, or its nickname, and its user when another listed account has the same
/// network.
fn account_option(st: &BeeperStore, account_id: &str, listed: &[String]) -> String {
    let n = network_label(account_id);
    if listed.iter().filter(|a| network_label(a) == n).count() < 2 {
        return st.custom_name(account_id).unwrap_or(n).into();
    }
    format!("{n} — {} (cloud)", account_display_name(st, account_id, listed))
}

/// Desktop's `userProfileLink`: the web profile of the other person of a chat, on the networks that have one.
fn profile_link(c: &Chat) -> Option<String> {
    let u = c.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| c.chat_type == "single")?;
    user_profile_link(&c.account_id, u)
}

/// Desktop's `getUserProfileLink` of user `u` of account `account_id`.
fn user_profile_link(account_id: &str, u: &User) -> Option<String> {
    use beeper_core::models::NetworkKind::*;
    let name = u.username.as_deref().unwrap_or_default();
    Some(match account_network(account_id).kind {
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
/// How long Desktop's flyover takes to reveal all its items, one after another.
const FLYOVER_REVEAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Desktop's `--animation-timing`, in seconds.
const HOVER_FADE: f32 = 0.1;

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
/// Desktop's "Room encryption" step of New Chat before it creates a Matrix group of more than one person.
const ASK_ROOM_ENCRYPTION: u8 = 24;
/// Desktop's `confirmUnmergeUnion` of a merged chat.
const ASK_UNMERGE: u8 = 25;
/// Desktop opens its `prefs` window at 800 x 740, in the middle of the screen.
const SETTINGS_SIZE: Size = Size::new(800.0, 740.0);
/// Desktop's default "Quick reaction emoji".
/// Wide enough for "Copy Entire Message with Timestamp" on one line.
const MESSAGE_MENU_W: f32 = 270.0;
/// Desktop's `minSidebarWidth`, the least width of the thread pane and of the chat beside it.
const MIN_PANE_W: f32 = 240.0;
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
/// Desktop's `--chat-view-bubble-padding-y`.
const BUBBLE_PAD_Y: f32 = 8.0;
/// The widths of the Copy, Remind Me, Label and Snooze submenus of the chat menu.
const SUB_MENU_W: [f32; 4] = [124.0, 257.0, 180.0, 257.0];

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

/// Desktop's `fD`: the Material icons by name, which labels and action buttons show. Each icon keeps the middle
/// `share` of its view box, as a box that clips the icon drawn larger.
fn material_icons(share: f32) -> HashMap<&'static str, svg::Handle> {
    let icon = |line: &'static str| {
        let [name, view, d] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("a Material icon line") };
        let box_of = view.split(' ').map(|v| v.parse::<f32>().expect("a view box number")).collect::<Vec<_>>();
        let [x, y, w, h] = box_of[..] else { panic!("a view box") };
        let (x, y, w, h) = (x + w * (1.0 - share) / 2.0, y + h * (1.0 - share) / 2.0, w * share, h * share);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x} {y} {w} {h}"><path fill="currentColor" d="{d}"/></svg>"#
        );
        (name, svg::Handle::from_memory(svg.into_bytes()))
    };
    include_str!("../assets/material_icons.tsv").lines().map(icon).collect()
}

/// Desktop's `yD`: the label icons by id, in the order of the picker.
static LABEL_ICONS: std::sync::LazyLock<Vec<(&str, svg::Handle)>> = std::sync::LazyLock::new(|| {
    let icons = material_icons(1.0);
    let icon = |line: &'static str| {
        let (id, name) = line.split_once('\t').expect("a label icon line");
        (id, icons[name].clone())
    };
    include_str!("../assets/label_icons.tsv").lines().map(icon).collect()
});

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

/// `ls` is Desktop's `--letter-spacing`; its initials also have `margin-right: -1px`. The avatar is drawn at `alpha`.
fn avatar<'a>(name: &str, size: f32, ls: f32, alpha: f32, p: &Palette) -> El<'a> {
    let Some(label) = initials(name) else {
        let shade = fill_box(Color { a: 0.1 * alpha, ..p.strong }, size / 2.0);
        return container(svg(EYES.clone()).width(size).height(size).opacity(alpha)).style(shade).into();
    };
    let &(bottom, top) = pick(name, &AVATAR_GRADIENTS);
    let (top, bottom) = (hex(top).scale_alpha(alpha), hex(bottom).scale_alpha(alpha));
    let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, top).add_stop(1.0, bottom);
    let font = crate::style::initials_font();
    let white = Color::WHITE.scale_alpha(alpha);
    let letter = |c: char| txt(c.to_string(), size * 0.5).line_height(1.0).font(font).color(white).into();
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
    let img = beeper_core::media::decode_upright(bytes)?;
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
/// The links of `text`, as Desktop's linkify finds them.
fn text_urls(text: &str) -> Vec<String> {
    linkify::links(text).into_iter().map(|(_, href)| href).collect()
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

/// Desktop's `canStartEditMessage`: an own text message, or an own message of one file whose caption the chat edits.
fn editable(m: &Message, chat: &Chat) -> bool {
    if m.is_sender != Some(true) || !chat.messages.edit {
        return false;
    }
    let kind = m.message_type.as_deref().unwrap_or("TEXT");
    match &m.attachments[..] {
        [] => kind == "TEXT",
        [a] if matches!(kind, "IMAGE" | "VIDEO" | "AUDIO" | "VOICE" | "FILE") && m.unresolved_media.is_none() => {
            let voice = a.is_voice_note == Some(true) || kind == "VOICE";
            let mime = a.mime_type.as_deref().unwrap_or_default();
            beeper_core::media::takes_caption(&chat.file, mime, a.is_gif == Some(true), voice)
        }
        _ => false,
    }
}

/// Desktop's `canEditMessage`: the edit window of the network is still open.
fn edit_open(m: &Message, rules: MessageRules) -> bool {
    m.is_deleted != Some(true) && MessageRules::young(rules.edit_max_age, &m.timestamp)
}

/// Desktop's pending `ContactAttachment`: the avatar, the name or the organization, and the first phone number and
/// email with the count of the others.
/// Desktop's `Wfe` contact card of a vCard: the 32 px avatar, the name and the organization, then `limit` of its phone
/// numbers and of its emails through `entry` (`true` for a phone number), with "+ %d more" after them.
pub(crate) fn contact_card<'a>(
    p: &Palette,
    c: &VCard,
    fg: Color,
    limit: usize,
    entry: &dyn Fn(bool, &str) -> El<'a>,
) -> Column<'a, UiMessage> {
    let line = |s: String| txt(s, 12.0).line_height(Pixels(16.0)).color(fg);
    let medium = |s: String| txt(s, 14.0).font(fw(Weight::Medium)).color(fg);
    let mut meta = Column::new().push((!c.name.is_empty()).then(|| medium(c.name.clone())));
    if let Some(org) = &c.org {
        meta = meta.push(match c.name.is_empty() {
            true => El::from(medium(org.clone())),
            false => container(line(org.clone())).padding(Padding::ZERO.top(4.0)).into(),
        });
    }
    let list = |items: &[String], title: String, more: &'static str, phone: bool| {
        let head = semi(title, 12.0).line_height(Pixels(16.0)).color(fg);
        let rest = items.len().saturating_sub(limit);
        let more = (rest > 0).then(|| line(trn(more, more, rest, &[&rest])));
        let col = column![container(head).padding(Padding::ZERO.top(6.0).bottom(4.0))];
        let shown = items.iter().take(limit).map(|i| entry(phone, i));
        (!items.is_empty()).then(|| col.extend(shown).push(more))
    };
    let n = c.phones.len();
    let phones = list(&c.phones, trn("Phone Number", "Phone Numbers", n, &[]), "+ %d more", true);
    let emails = list(&c.emails, trn("Email", "Emails", c.emails.len(), &[]), " + %d more", false);
    let details = row![avatar(&c.name, 32.0, 0.0, 1.0, p), meta.push(phones).push(emails)].spacing(8);
    column![details, Space::new().width(180)]
}

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
fn badged<'a>(inner: El<'a>, brand: &str, size: f32, badge: f32, mono: Option<&Palette>, ring: Color) -> El<'a> {
    let icon: El<'a> = match mono {
        None => svg(crate::style::brand_round(brand)).width(badge).height(badge).into(),
        Some(p) => {
            let fg = p.strong;
            let glyph = svg(crate::style::brand_badge(brand))
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
// Desktop's `CircleInfoIcon` of the Chat Info toggle.
svg_static!(INFO_CIRCLE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M9.167 14.164h1.667v-5H9.167zm.833-6.667q.354 0 .594-.24.24-.239.24-.593t-.24-.594A.8.8 0 0 0 10 5.83a.8.8 0 0 0-.594.24.8.8 0 0 0-.24.594q0 .354.24.594.24.24.594.24m0 10.834a8.1 8.1 0 0 1-3.25-.657 8.4 8.4 0 0 1-2.646-1.78 8.4 8.4 0 0 1-1.78-2.647A8.1 8.1 0 0 1 1.666 10q0-1.729.656-3.25a8.4 8.4 0 0 1 1.781-2.646 8.4 8.4 0 0 1 2.646-1.781A8.1 8.1 0 0 1 10 1.664q1.73 0 3.25.656a8.4 8.4 0 0 1 2.646 1.781 8.4 8.4 0 0 1 1.781 2.646q.656 1.52.656 3.25t-.656 3.25a8.4 8.4 0 0 1-1.781 2.646 8.4 8.4 0 0 1-2.646 1.781 8.1 8.1 0 0 1-3.25.657m0-1.667q2.79 0 4.729-1.937Q16.667 12.79 16.667 10t-1.938-4.729T10 3.331 5.271 5.27 3.334 10t1.937 4.73T10 16.663"/></svg>"##);
// Desktop's `chevron-compact-right` icon, the Material `chevron_forward`.
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
// Desktop's Material `info`, `account_circle`, `sell`, `event_busy`, `alternate_email` and `forum` icons of the
// Chat Info behavior line, its menu and the How It Works window.
svg_static!(INFO_ICON, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M440-280h80v-240h-80v240Zm40-320q17 0 28.5-11.5T520-640q0-17-11.5-28.5T480-680q-17 0-28.5 11.5T440-640q0 17 11.5 28.5T480-600Zm0 520q-83 0-156-31.5T197-197q-54-54-85.5-127T80-480q0-83 31.5-156T197-763q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480q0 83-31.5 156T763-197q-54 54-127 85.5T480-80Zm0-80q134 0 227-93t93-227q0-134-93-227t-227-93q-134 0-227 93t-93 227q0 134 93 227t227 93Zm0-320Z"/></svg>"##);
svg_static!(ACCOUNT_CIRCLE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M234-276q51-39 114-61.5T480-360q69 0 132 22.5T726-276q35-41 54.5-93T800-480q0-133-93.5-226.5T480-800q-133 0-226.5 93.5T160-480q0 59 19.5 111t54.5 93Zm246-164q-59 0-99.5-40.5T340-580q0-59 40.5-99.5T480-720q59 0 99.5 40.5T620-580q0 59-40.5 99.5T480-440Zm0 360q-83 0-156-31.5T197-197q-54-54-85.5-127T80-480q0-83 31.5-156T197-763q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480q0 83-31.5 156T763-197q-54 54-127 85.5T480-80Zm0-80q53 0 100-15.5t86-44.5q-39-29-86-44.5T480-280q-53 0-100 15.5T294-220q39 29 86 44.5T480-160Zm0-360q26 0 43-17t17-43q0-26-17-43t-43-17q-26 0-43 17t-17 43q0 26 17 43t43 17Zm0-60Zm0 360Z"/></svg>"##);
svg_static!(SELL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M856-390 570-104q-12 12-27 18t-30 6q-15 0-30-6t-27-18L103-457q-11-11-17-25.5T80-513v-287q0-33 23.5-56.5T160-880h287q16 0 31 6.5t26 17.5l352 353q12 12 17.5 27t5.5 30q0 15-5.5 29.5T856-390ZM513-160l286-286-353-354H160v286l353 354ZM260-640q25 0 42.5-17.5T320-700q0-25-17.5-42.5T260-760q-25 0-42.5 17.5T200-700q0 25 17.5 42.5T260-640Zm220 160Z"/></svg>"##);
svg_static!(EVENT_BUSY, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="m388-212-56-56 92-92-92-92 56-56 92 92 92-92 56 56-92 92 92 92-56 56-92-92-92 92ZM200-80q-33 0-56.5-23.5T120-160v-560q0-33 23.5-56.5T200-800h40v-80h80v80h320v-80h80v80h40q33 0 56.5 23.5T840-720v560q0 33-23.5 56.5T760-80H200Zm0-80h560v-400H200v400Zm0-480h560v-80H200v80Zm0 0v-80 80Z"/></svg>"##);
svg_static!(ALTERNATE_EMAIL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M480-80q-83 0-156-31.5T197-197q-54-54-85.5-127T80-480q0-83 31.5-156T197-763q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480v58q0 59-40.5 100.5T740-280q-35 0-66-15t-52-43q-29 29-65.5 43.5T480-280q-83 0-141.5-58.5T280-480q0-83 58.5-141.5T480-680q83 0 141.5 58.5T680-480v58q0 26 17 44t43 18q26 0 43-18t17-44v-58q0-134-93-227t-227-93q-134 0-227 93t-93 227q0 134 93 227t227 93h200v80H480Zm0-280q50 0 85-35t35-85q0-50-35-85t-85-35q-50 0-85 35t-35 85q0 50 35 85t85 35Z"/></svg>"##);
svg_static!(FORUM, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M280-240q-17 0-28.5-11.5T240-280v-80h520v-360h80q17 0 28.5 11.5T880-680v600L720-240H280ZM80-280v-560q0-17 11.5-28.5T120-880h520q17 0 28.5 11.5T680-840v360q0 17-11.5 28.5T640-440H240L80-280Zm520-240v-280H160v280h440Zm-440 0v-280 280Z"/></svg>"##);
// Desktop's `FullScreenClose` icon of the zoomed Chat Info avatar.
svg_static!(FULL_SCREEN_CLOSE, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 18 18 6M6 6l12 12" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##);
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

// Desktop's `thumbnail-bar` system icon of "Show Sidebar".
svg_static!(THUMBNAIL_BAR, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="#000"><path d="M3.5 16q-.62 0-1.06-.44A1.45 1.45 0 0 1 2 14.5V5.493q0-.62.44-1.056A1.45 1.45 0 0 1 3.5 4h13q.62 0 1.06.44.44.442.44 1.06v9.006q0 .62-.44 1.056A1.45 1.45 0 0 1 16.5 16zM8 14.5h8.5v-9H8zm-1.5 0v-9h-3v9z"/></svg>"##);
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

// Desktop's `download` icon, the Material `download`.
svg_static!(MAT_DOWNLOAD, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M480-320 280-520l56-58 104 104v-326h80v326l104-104 56 58-200 200ZM240-160q-33 0-56.5-23.5T160-240v-120h80v120h480v-120h80v120q0 33-23.5 56.5T720-160H240Z"/></svg>"##);
svg_static!(MAT_FORWARD, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="m640-280-57-56 184-184-184-184 57-56 240 240-240 240ZM80-200v-160q0-83 58.5-141.5T280-560h247L383-704l57-56 240 240-240 240-57-56 144-144H280q-50 0-85 35t-35 85v160H80Z"/></svg>"##);

svg_static!(ZOOM_LEVEL, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 22 15" fill="#000"><path d="M20.64 14.343C21.01 14.343 21.303 14.237 21.52 14.026C21.736 13.816 21.845 13.534 21.845 13.183C21.845 12.937 21.798 12.679 21.705 12.409L17.995 2.346C17.837 1.877 17.603 1.53 17.292 1.304C16.982 1.079 16.583 0.966 16.097 0.966C15.177 0.966 14.562 1.423 14.251 2.336L10.498 12.488C10.4122 12.7219 10.3675 12.9689 10.366 13.218C10.366 13.563 10.469 13.838 10.674 14.04C10.879 14.242 11.16 14.343 11.518 14.343C11.863 14.343 12.134 14.261 12.331 14.097C12.527 13.933 12.675 13.666 12.774 13.297L13.636 10.915H18.531L19.393 13.297C19.493 13.666 19.64 13.933 19.836 14.097C20.033 14.261 20.301 14.343 20.641 14.343H20.64ZM17.995 8.999H14.198L16.07 3.479H16.123L17.995 9V8.999ZM7.87 14.325C8.175 14.325 8.42 14.233 8.608 14.048C8.7018 13.9545 8.77527 13.8427 8.8238 13.7195C8.87232 13.5962 8.89486 13.4643 8.89 13.332C8.89 13.209 8.878 13.09 8.854 12.976C8.8266 12.8487 8.78846 12.7239 8.74 12.603L6.14 5.606C5.987 5.19 5.782 4.883 5.524 4.684C5.267 4.484 4.939 4.384 4.54 4.384C4.165 4.384 3.85 4.484 3.595 4.684C3.34 4.883 3.137 5.19 2.985 5.606L0.381 12.603C0.292023 12.8359 0.244637 13.0827 0.241 13.332C0.241 13.642 0.332 13.886 0.517 14.062C0.702 14.237 0.952 14.325 1.269 14.325C1.532 14.325 1.746 14.25 1.91 14.101C2.074 13.951 2.203 13.707 2.297 13.367L2.851 11.741H6.261L6.814 13.385C6.902 13.725 7.031 13.966 7.201 14.11C7.371 14.253 7.594 14.325 7.869 14.325H7.87ZM5.787 10.16H3.31L4.45 6.494H4.626L5.786 10.159L5.787 10.16Z"/></svg>"##);
svg_static!(MAT_SYNC, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M160-160v-80h110l-16-14q-52-46-73-105t-21-119q0-111 66.5-197.5T400-790v84q-72 26-116 88.5T240-478q0 45 17 87.5t53 78.5l10 10v-98h80v240H160Zm400-10v-84q72-26 116-88.5T720-482q0-45-17-87.5T650-648l-10-10v98h-80v-240h240v80H690l16 14q49 49 71.5 106.5T800-482q0 111-66.5 197.5T560-170Z"/></svg>"##);
svg_static!(MAT_EXPAND_LESS, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M480-528 296-344l-56-56 240-240 240 240-56 56-184-184Z"/></svg>"##);
svg_static!(MAT_LINK, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#000"><path d="M440-280H280q-83 0-141.5-58.5T80-480q0-83 58.5-141.5T280-680h160v80H280q-50 0-85 35t-35 85q0 50 35 85t85 35h160v80ZM320-440v-80h320v80H320Zm200 160v-80h160q50 0 85-35t35-85q0-50-35-85t-85-35H520v-80h160q83 0 141.5 58.5T880-480q0 83-58.5 141.5T680-280H520Z"/></svg>"##);

/// Desktop's `oo` for a US number: "+1 415-555-0142".
fn fmt_phone(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() == 12 && b.starts_with(b"+1") && b[2..].iter().all(u8::is_ascii_digit) {
        format!("+1 {}-{}-{}", &s[2..5], &s[5..8], &s[8..12])
    } else {
        s.to_string()
    }
}

/// The rail ID of Desktop's draft label, which shows while Create Label is open.
/// Desktop's `Jee` sidebar width of 320px, less the 68px space bar.
const SIDEBAR_WIDTH: f32 = 252.0;
/// Desktop's `wxe`: the number of actions that Undo remembers.
const UNDO_DEPTH: usize = 100;

/// Desktop's `Oct`: a typing notice ends 3 seconds after the last key.
const TYPING_STOP: Duration = Duration::from_secs(3);
const LABEL_DRAFT: &str = "label:draft";

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

/// The `chat_menu` key of Desktop's space sheet (`zft`), which opens from the sidebar header title.
const SPACE_SHEET: &str = "space:sheet";
/// The `chat_menu` key of the "Switch Chat" hover card of the composer.
const SWITCH_CARD: &str = "switch:card";

/// The `chat_menu` key of Desktop's sidebar background menu (`Gct`).
const SIDEBAR_MENU: &str = "sidebar:menu";

/// The `chat_menu` key prefix of the Snooze menu (`Jde`) of the Chat Info toolbar, before the chat ID.
const SNOOZE_MENU: &str = "snooze:";

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
/// The link of a mention of another person in a message: the chat, a space and the user ID.
pub(crate) const MENTION: &str = "beeper-mention:";

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
const INFO_MENUS: [&str; 7] = ["info:", "copy:", "member:", "bookmark:", "contact:", "mention:", "vcard:"];

/// Desktop's `setPersonalRoomName` gate: the feature is on or the chat has a personal name or photo, and the chat
/// is a Matrix room that is not merged (`canSetPersonalOverrides`, `personalOverridesApply`).
fn can_edit_for_me(st: &BeeperStore, c: &Chat) -> bool {
    let room = c.id.starts_with('!') && c.merge.is_none() && c.merged_into_chat_id.is_none();
    room && (st.feature("personal-room-overrides").on || c.personal.is_some())
}

/// Desktop's sidebar overflow menu (`Plt`): the hidden spaces, then New Label…, Edit Spaces… and Add/Remove Accounts…,
/// with `None` for a separator.
fn more_menu(st: &BeeperStore) -> Vec<Option<(String, UiMessage)>> {
    let unread = st.chats.iter().any(|c| st.new_request(c));
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
        items.extend([Some((tr("New Label…").into(), UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())))), None]);
    }
    items.push(Some((tr("Edit Spaces…").into(), UiMessage::OpenEditSpaces)));
    items.push(Some((
        tr("Add/Remove Accounts…").into(),
        UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)),
    )));
    items
}

/// Desktop's `selectSpace` for a menu row of the space `id`. The Create Label space opens Create Label.
fn space_msg(id: &str) -> UiMessage {
    match RAIL_VIEWS.iter().find(|v| v.0 == id).map(|v| v.1) {
        Some(Some(tab)) => UiMessage::ChatList(chat_list::Msg::SelectTab(tab)),
        Some(None) => UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())),
        None => UiMessage::Rail(rail::Msg::Select(Some(id.into()))),
    }
}

/// Desktop's `canAcceptRequest` and `canDeclineRequest` of a message request chat.
fn request_answers(c: &Chat) -> (bool, bool) {
    let request = c.is_request == Some(true);
    (request && c.can_accept_request, request && c.delete_modes & DELETE_FOR_ME != 0)
}

/// Desktop's `getSpaceName`.
fn space_name(st: &BeeperStore, id: &str) -> String {
    match id {
        "inbox" => tr("Inbox").into(),
        "archive" => trx("Archive", "archive space name").into(),
        "reminders" => tr("Reminders").into(),
        "snoozed" => trx("Snoozed", "space name for snoozed chats").into(),
        "requests" => tr("Requests").into(),
        "low_priority" => tr("Low Priority").into(),
        "scheduled" => tr("Send Later").into(),
        LABEL_NUDGE => tr("Create Label").into(),
        _ => match (rail_label(st, id), network_space(st, id)) {
            (Some(l), _) if l.name.is_empty() => trx("Label", "fallback label space name").into(),
            (Some(l), _) => l.name.clone(),
            (None, Some(n)) => format!("{} ({})", n.name, account_display_name(st, &n.account_id, &rail_accounts(st))),
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
            let n = count(&|c| c.reminder.is_some() || !st.message_reminders_of(&c.id).is_empty());
            trn("%d Reminder", "%d Reminders", n, &[&n])
        }
        "snoozed" => chats(count(&|c| {
            crate::reminders::snoozed_until(st, c).is_some() && c.flags & CHAT_HIDDEN == 0 && listed(c)
        })),
        "requests" => {
            let n = count(&|c| c.flags & CHAT_REQUEST != 0 && c.flags & CHAT_HIDDEN == 0);
            trn("%d Request", "%d Requests", n, &[&n])
        }
        "low_priority" => {
            let n = count(&|c| c.flags & CHAT_LOW_PRIORITY != 0 && c.flags & CHAT_HIDDEN == 0);
            trn("%d Low Priority Chat", "%d Low Priority Chats", n, &[&n])
        }
        "scheduled" => trn("%d Message", "%d Messages", st.scheduled.len(), &[&st.scheduled.len()]),
        LABEL_NUDGE => return None,
        _ => chats(count(&|c| in_rail_entry(st, id, c) && listed(c))),
    })
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
    let plat = if c.merge.is_some() { merged::chat_platform(c) } else { network_label(&c.account_id) };
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

    /// Desktop's `UY` without the text: the search operators of the filters and of the searched chat `room`.
    fn operators(&self, room: Option<&str>) -> String {
        let mut ops: Vec<String> = match self.media.len() {
            5 => vec!["has:attachment".into()],
            _ => (0..5).filter(|k| self.media.contains(k)).map(|k| format!("has:{}", search::HAS[k].0)).collect(),
        };
        ops.extend(self.sent.map(|me| if me { "from:me" } else { "from:others" }.to_string()));
        ops.extend(self.group.map(|g| if g { "in:group" } else { "in:dm" }.to_string()));
        ops.extend(room.map(|r| format!("in:{r}")));
        ops.extend(self.accounts.iter().map(|a| format!("network:{a}")));
        match self.period.map(|i| search::DATES[i]) {
            Some("yesterday") => ops.extend(["before:today".into(), "after:yesterday".into()]),
            Some(key) => ops.push(format!("after:{key}")),
            None => {}
        }
        ops.join(" ")
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
    matches!(
        msg,
        UiMessage::Composer(composer::Msg::Edit(_))
            | UiMessage::Message(message::Msg::Click(_))
            | UiMessage::Message(message::Msg::Select(_))
    )
}

/// Where `locate` puts a container in its scrollable.
#[derive(Clone, Copy)]
enum Align {
    /// With the least scroll.
    Nearest,
    Center,
    /// Its top this far under the top of the scrollable, like `virtua`'s `align: "start"` with a negative offset.
    Start(f32),
}

/// Finds the offset of a scrollable that shows a container where `Align` puts it. The offset of the timeline counts
/// from its bottom, because it is anchored there.
struct Locate(
    iced::widget::Id,
    Option<(iced::Rectangle, iced::Rectangle, f32)>,
    Option<iced::Rectangle>,
    Align,
    &'static str,
);

fn locate(target: impl Into<iced::widget::Id>, scroll: &'static str, align: Align) -> Task<UiMessage> {
    iced::advanced::widget::operate(Locate(target.into(), None, None, align, scroll))
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
            Align::Nearest => at.max(y + msg.height - view.height).min(y),
            Align::Center => y - (view.height - msg.height) / 2.0,
            Align::Start(gap) => y - gap,
        };
        let top = top.clamp(0.0, max);
        let anchored = matches!(self.4, "timeline" | "thread-timeline");
        iced::advanced::widget::operation::Outcome::Some(if anchored { max - top } else { top })
    }
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
    let delete =
        mine.iter().map(|m| UiMessage::Message(message::Msg::Delete(m.chat_id.clone(), m.id.clone()))).collect();
    let mut buttons: Vec<(&'static str, UiMessage)> =
        label.map(|l| (trx(l, "confirmation button"), UiMessage::Commands(delete))).into_iter().collect();
    let removable: Vec<&Message> = ms.iter().copied().filter(|m| !m.silent() && !pending(st, m)).collect();
    if !removable.is_empty() {
        let ids = removable.iter().map(|m| m.id.clone()).collect();
        buttons.push((
            trx("Remove from Beeper", "confirmation button"),
            UiMessage::Message(message::Msg::RemoveFromBeeper(ids)),
        ));
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
// Desktop's `OutlineClock`, the message reminder indicator.
svg_static!(OUTLINE_CLOCK, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill-rule="evenodd" clip-rule="evenodd" d="M2.5 10a7.5 7.5 0 1114.999 0A7.5 7.5 0 012.5 10zM10 1a9 9 0 100 17.999A9 9 0 0010 1zm.75 4.5a.75.75 0 00-1.5 0V10c0 .242.116.469.314.61l3.5 2.5a.75.75 0 00.871-1.22L10.75 9.613V5.5z"/></svg>"##);
// Desktop's Material `moon_stars`, its `snoozed-space` icon on Linux.
svg_static!(MOON_STARS, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960"><path d="M600-640 480-760l120-120 120 120-120 120Zm200 120-80-80 80-80 80 80-80 80ZM483-80q-84 0-157.5-32t-128-86.5Q143-253 111-326.5T79-484q0-146 93-257.5T409-880q-18 99 11 193.5T520-521q71 71 165.5 100T879-410q-26 144-138 237T483-80Zm0-80q88 0 163-44t118-121q-86-8-163-43.5T463-465q-61-61-97-138t-43-163q-77 43-120.5 118.5T159-484q0 135 94.5 229.5T483-160Zm-20-305Z"/></svg>"##);
// Desktop's `BeeperIcon` of the networks icons.
svg_static!(BEEPER_ICON, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 15"><path fill-rule="evenodd" clip-rule="evenodd" d="M0.350037 5.29941C0.350037 2.76021 2.33427 0.649414 4.84524 0.649414H8.75483C11.2658 0.649414 13.25 2.76021 13.25 5.29941C13.25 6.61227 12.7229 7.80414 11.8693 8.65264L7.23829 13.8348C6.60047 14.5486 5.42971 13.9544 5.62929 13.0182L6.2835 9.94942H4.84524C2.33427 9.94942 0.350037 7.83862 0.350037 5.29941ZM4.84524 2.14941C3.21937 2.14941 1.85004 3.5308 1.85004 5.29941C1.85004 7.06803 3.21937 8.44941 4.84524 8.44941H8.13699L7.54101 11.245L10.7824 7.61781L10.7997 7.60083C11.382 7.02833 11.75 6.2123 11.75 5.29941C11.75 3.5308 10.3807 2.14941 8.75483 2.14941H4.84524Z"/></svg>"##);

/// Desktop's `hasOlderMessages`: the chat has events before its first loaded page, or a member chat of a merged chat
/// does.
fn has_older(st: &BeeperStore, chat: &Chat) -> bool {
    let members = st.members(st.chat_idx(&chat.id));
    let older = |c: &Chat| c.older != Older::Start;
    st.matrix.is_some() && if members.is_empty() { older(chat) } else { members.iter().any(|&i| older(&st.chats[i])) }
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
            .on_press_maybe((!on).then_some(UiMessage::Settings(settings::Msg::SelectTheme(t))))
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
    /// The pending attachment with the keyboard focus, and the one whose Quick Look shows (Desktop's `C_t`).
    pub att_focus: Option<usize>,
    pub quick_look: Option<usize>,
    pub composer_links: Vec<(String, LinkState)>,
    pub message_menu: Option<(String, Point)>,
    /// The Remind Me submenu of the message menu is open.
    pub remind_sub: bool,
    /// The text selected when the message menu opened, which turns it into the menu for the selection.
    pub menu_selection: Option<String>,
    /// The `href` of the link whose right click opened the message menu: an `https://` or `mailto:` one.
    pub menu_link: Option<String>,
    /// The message menu is Desktop's video menu, from a right click on the video.
    pub video_menu: bool,
    /// The message menu is from a right click on a shown image (Desktop's `selectedImage`).
    pub menu_image: bool,
    /// The quoted message whose right click opened the message menu (Desktop's `selectedLinkedMessage`).
    pub menu_linked: Option<String>,
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
    /// The GIFs that their play button started while "Autoplay GIFs" is off, by message and attachment, from when.
    pub played_gifs: HashMap<(String, usize), std::time::Instant>,
    pub hovered_msg: Option<String>,
    /// When `hovered_msg` began, and when the pointer entered its flyover, for the flyover's staggered reveal.
    pub hover_msg_at: Option<std::time::Instant>,
    pub flyover_at: Option<std::time::Instant>,
    /// The selected emoji autocomplete row.
    pub suggest: usize,
    pub history: History,
    /// The composer text where Escape closed the emoji autocomplete.
    pub suggest_hidden: Option<String>,
    /// After an emoticon replacement: the new text, and the typed text and caret that Backspace restores.
    pub emoticon: Option<(String, String, usize)>,
    /// The first message IDs of the expanded membership event groups.
    pub open_members: HashSet<String>,
    /// The membership event group that opened because the selection went into it.
    pub auto_members: Option<String>,
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
    /// Desktop's `unreadDividerMessageSortKey`: the message after the read position of the chat when it opened,
    /// which keeps the "New" divider in place while the chat is read.
    pub unread_divider: Option<String>,
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

/// The tab, the filters, the space and the space view account of a chat list.
type ListKey = (InboxTab, u8, Option<String>, Option<String>);

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
    /// The unread threads of all chats, as the chat index and the root.
    threads: std::cell::RefCell<Option<Threads>>,
}

/// The query, the filters, the room and the partial matches of a command bar search.
type HitsKey = (String, SearchFilters, Option<String>, bool);

/// What a command bar search finds in Desktop's sections: the chats by title and by member as indices of `chats`,
/// and the bookmarks, the media and the text messages, newest first.
#[derive(Default)]
struct Found {
    chats: Vec<usize>,
    in_groups: Vec<usize>,
    bookmarks: Vec<Message>,
    media: Vec<Message>,
    messages: Vec<Message>,
}

type Hits = Rc<Found>;

/// All chats newest first, as indices of `chats`, and the lowercase title and search subtitle of each chat.
type Words = (Rc<[usize]>, Rc<[String]>);

pub struct BeeperGuiApp {
    pub pane: Pane,
    /// The chat windows and their panes.
    chat_windows: Vec<(window::Id, Pane)>,
    /// The space windows by the rail space they show, with their chat lists. Each also has a pane in
    /// `chat_windows`.
    space_windows: Vec<(window::Id, String, chat_list::ListState)>,
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
    /// The chat list of the main window, or of the space window whose messages `update` handles.
    pub list: chat_list::ListState,
    /// The requests that the Requests view last marked as seen.
    seen_requests: Vec<String>,
    /// The Picture in Picture window and the message of its video, and a video that should go there.
    pip: Option<(window::Id, String)>,
    pip_wanted: bool,
    /// The video went to Picture in Picture from the video menu, so it stays there while its chat is open.
    pip_manual: bool,
    /// Where the video menu of the Picture in Picture window is open.
    pip_menu: Option<Point>,
    pub hotkey_query: String,
    pub hotkey_rec: Option<HotkeyRec>,
    pub profile: profile::ProfileForm,
    /// The search text of the open Keyboard Shortcuts list, and whether the list scrolled down.
    pub hotkeys_modal: Option<String>,
    /// Desktop's `FileDropOverlay` shows, and Shift was down at a drop, so the dropped files are sent.
    dropping: bool,
    drop_send: bool,
    hotkeys_scrolled: bool,
    /// Chats picked by Begin Selecting; selection mode is on while this is not empty.
    pub selection: Vec<String>,
    /// Desktop's chat selection mode from Select Chats, before the first chat is picked.
    select_mode: bool,
    /// The open selection bar menu (Remind Me or More) and the x of its button.
    selection_menu: Option<(u8, f32)>,
    chat_sub: Option<u8>,
    /// The reminder time each chat or message was last reminded of.
    reminded: HashMap<String, i64>,
    /// Desktop's `snoozeStore.clockMs`: when the ended snoozes were last checked.
    snooze_clock: i64,
    pub search_room: Option<String>,
    pub search_filters: SearchFilters,
    pub search_select: Option<(usize, iced::Rectangle)>,
    cursor: std::cell::Cell<Point>,
    forward: Option<Forward>,
    carousel: Option<Carousel>,
    media_window: Option<MediaWindow>,
    /// On X11 the copied image stays available only while its clipboard lives.
    image_clipboard: Option<arboard::Clipboard>,
    /// Desktop's `pendingButtonIDs`: the action buttons whose callback runs, as (message, button).
    pressing: HashSet<(String, String)>,
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
    /// Desktop's `expandedCategories`: the search sections that show more.
    pub search_expanded: HashSet<&'static str>,
    /// Desktop's `partialMatchesEnabled` of the message search.
    pub search_partial: bool,
    /// Desktop's `lastArchivedThread`: the chat that was archived or unarchived last.
    last_archived: std::cell::RefCell<Option<String>>,
    pub creating_chat: Option<u32>,
    pub new_chats: Vec<NewChat>,
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
    /// The Encrypted toggle of the "Room encryption" step of the open New Chat draft.
    pub group_encrypted: bool,
    /// Desktop's `matrixRoomJoin` of the Join a Matrix room view: a join is going on, and the error of the last one.
    join_room: (bool, Option<String>),
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
    pub confirm: Option<u8>,
    delete_target: Option<String>,
    /// The chats of Desktop's `confirmDeleteThread`: the selected chats, or the chat of the menu.
    delete_chats: Vec<String>,
    /// Desktop's `loading` of the device list: a logout of another device runs.
    devices_busy: bool,
    /// The "Report spam" checkbox of the Block dialog.
    report_spam: bool,
    mods: keyboard::Modifiers,
    /// Desktop's `TimeSuggestionsPreference`: the add input and the drag of its list.
    time_edit: time_suggestions::TimeEdit,
    audio: voice::Player,
    recorder: Option<record::Recorder>,
    talk: Option<record::Talk>,
    queued: Vec<Queued>,
    /// The system microphone and the other inputs, listed when General settings open.
    mics: Option<Result<(String, Vec<String>), String>>,
    pub show_command_palette: bool,
    pub active_settings_tab: SettingsTab,
    pub settings_query: String,
    /// A settings slider being dragged, and its value, which Desktop saves when the drag ends.
    pub settings_slider: Option<(&'static str, u8)>,
    /// The focused row of the settings search results, and the last opened one.
    settings_focus: usize,
    settings_pick: Option<SettingsHit>,
    /// The pref that a search result opened, which flashes for 2 s.
    pref_flash: Option<(&'static str, std::time::Instant)>,
    pub networks: networks::Networks,
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
    /// When `hovered_chat` began, and the row the pointer last left with when, for the fade of the hover actions.
    hover_at: Option<std::time::Instant>,
    unhovered: Option<(String, std::time::Instant)>,
    pub flash: Option<(String, std::time::Instant)>,
    /// Desktop's `BrandTypingIndicator` of each chat as it shows (`true`) or hides: who types, and since when.
    typing_shown: HashMap<String, (Vec<String>, bool, std::time::Instant)>,
    last_click: Option<(String, std::time::Instant)>,
    /// The opened chats, the most recent first. While `cycling`, a selection does not reorder them.
    chat_history: Vec<String>,
    cycling: bool,
    pub status_banner: String,
    /// The text of Desktop's toast with a "Relaunch app" button, after a change that needs a relaunch.
    relaunch_toast: Option<&'static str>,
    /// Desktop's backup banner of the recovery key was clicked in this session.
    key_banner_closed: bool,
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
    /// When the sidebar last hid or showed, for its slide.
    sidebar_slide: Option<std::time::Instant>,
    pub dragging: bool,
    /// The rail space under the cursor, and where the cursor is in it.
    pub rail_hover: Option<(String, Point)>,
    pub user_hover: UserHover,
    pub rail_drag: Option<RailDrag>,
    /// The rail space of the last press and when, for Desktop's double click that opens a space window.
    rail_press: Option<(String, std::time::Instant)>,
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
    /// Desktop's How It Works window of the Chat Info behavior menu.
    how_window: Option<window::Id>,
    /// Desktop's Account Nickname windows, one for each account.
    pub nickname_windows: Vec<nickname::NicknameDraft>,
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
    /// Whether Chat Info reads the bookmarks room again after a failed restoration.
    restoring_bookmarks: bool,
    /// The Media tab of the expanded Chat Info Media section, or None while it shows only the newest.
    pub info_media: Option<usize>,
    /// Whether the Chat Info avatar shows zoomed to full screen.
    pub info_zoom: bool,
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
    /// The contact cards of `.vcf` attachments by URL, `None` while they load.
    pub(crate) vcards: HashMap<String, Option<Result<vcard::VCard, String>>>,
    /// The PDFs of the carousel by URL, `None` while they load; their pages are `media` (`pdf.rs`).
    pub(crate) pdfs: HashMap<String, Option<Result<pdf::Doc, String>>>,
    /// The baked gallery tiles (`gallery.rs`).
    tiles: images::Images<image::Handle>,
    /// Desktop's account `messages` cache for quotes: a quoted message out of the timeline, `None` while it loads.
    pub(crate) quotes: HashMap<String, Option<Result<Message, String>>>,
    /// Desktop's `dismissError`: the accounts whose disconnected banner was dismissed this session.
    dismissed_errors: HashSet<String>,
    pub resolving_media: HashMap<String, bool>,
    /// Desktop's `mediaDownloadStore`: the media URLs that a bridge loads only when the user asks (`needsDownload`),
    /// and those whose interactive download runs (`fetching`).
    pub(crate) needs_download: HashSet<String>,
    pub(crate) fetching: HashSet<String>,
    video: Option<video::Video>,
    view_once: view_once::ViewOnce,
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
        // Desktop's `loadProvisioningCapabilities` gives the Matrix account those of the SDK's `matrix` pseudo-bridge.
        let matrix = ("hungryserv".to_string(), beeper_core::matrix::matrix_capabilities());
        let resolve = resolve.chain([matrix]).collect();
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
            space_windows: Vec::new(),
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
            list: chat_list::ListState::default(),
            seen_requests: Vec::new(),
            pip: None,
            pip_wanted: false,
            pip_manual: false,
            pip_menu: None,
            hotkey_query: String::new(),
            hotkey_rec: None,
            profile: profile::ProfileForm::default(),
            hotkeys_modal: None,
            dropping: false,
            drop_send: false,
            hotkeys_scrolled: false,
            chat_sub: None,
            reminded: HashMap::new(),
            snooze_clock: now_unix_millis(),
            search_room: None,
            search_filters: SearchFilters::default(),
            search_select: None,
            cursor: Default::default(),
            image_clipboard: None,
            pressing: HashSet::new(),
            forward: None,
            carousel: None,
            media_window: None,
            mods: keyboard::Modifiers::empty(),
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
            search_expanded: HashSet::new(),
            search_partial: true,
            last_archived: Default::default(),
            creating_chat: None,
            new_chats: Vec::new(),
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
            group_encrypted: true,
            join_room: (false, None),
            devices: None,
            mics: None,
            confirm: None,
            delete_target: None,
            delete_chats: Vec::new(),
            devices_busy: false,
            report_spam: false,
            time_edit: Default::default(),
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
            settings_slider: None,
            settings_focus: 0,
            settings_pick: None,
            pref_flash: None,
            networks: Default::default(),
            app_qr: false,
            storage: StoragePage { ages: [1; 2], kinds: [0b1111, 0b11111], ..Default::default() },
            integrations: IntegrationsPage::default(),
            token_ask: None,
            account_page: None,
            show_right_panel: panel,
            quick_look_target: None,
            quick_open: false,
            hovered_chat: None,
            hover_at: None,
            unhovered: None,
            flash: None,
            typing_shown: HashMap::new(),
            status_banner: String::new(),
            relaunch_toast: None,
            key_banner_closed: false,
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
            sidebar_slide: None,
            dragging: false,
            rail_hover: None,
            header_hover: (false, 0),
            menu_bar: None,
            alt_alone: false,
            pin_hover: None,
            pin_drag: None,
            user_hover: UserHover::default(),
            rail_drag: None,
            rail_press: None,
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
            how_window: None,
            nickname_windows: Vec::new(),
            add_accounts: Vec::new(),
            info_desc: None,
            info_more: false,
            info_all: false,
            info_bookmarks: false,
            hydrating: HashSet::new(),
            restoring_bookmarks: false,
            info_media: None,
            info_zoom: false,
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
            vcards: HashMap::new(),
            pdfs: HashMap::new(),
            tiles: images::Images::new(64 << 20),
            quotes: HashMap::new(),
            dismissed_errors: HashSet::new(),
            resolving_media: HashMap::new(),
            needs_download: HashSet::new(),
            fetching: HashSet::new(),
            video: None,
            view_once: Default::default(),
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
        let tasks = [
            open.map(|a0| UiMessage::Window(windows::Msg::Opened(a0))),
            iced::system::theme().map(|a0| UiMessage::Settings(settings::Msg::SystemTheme(a0))),
            app.load_avatars(),
        ];
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
                move |r| UiMessage::Loaded(loads::Msg::Avatar(url, r)),
            )
        }))
    }

    fn face<'a>(&self, url: Option<&str>, name: &str, size: f32, ls: f32) -> El<'a> {
        self.face_alpha(url, name, size, ls, 1.0)
    }

    /// [`Self::face`] at the opacity `alpha`.
    fn face_alpha<'a>(&self, url: Option<&str>, name: &str, size: f32, ls: f32, alpha: f32) -> El<'a> {
        let loaded = url.and_then(|u| match self.avatars.get(u) {
            Some(h) => h.clone(),
            None => {
                self.wants.ask(&self.wants.avatars, u);
                None
            }
        });
        match loaded {
            Some(h) => image(h).width(size).height(size).opacity(alpha).into(),
            None => avatar(name, size, ls, alpha, &self.pal),
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
        // Desktop's `ThreadAvatar` of a merged chat without a photo: the letter of its title.
        if chat.merge.is_some() {
            return self.face(chat.img_url.as_deref(), &chat.title, size, ls);
        }
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

    /// Desktop's `subwindow-state`: the chat and settings windows to open again at the next launch.
    fn save_subwindows(&self) {
        // Desktop's `zv` keeps no `space-` windows.
        let chats = self.chat_windows.iter().filter(|w| w.1.thread_root.is_none() && !self.is_space_window(w.0));
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
                known.then(|| self.update(UiMessage::Window(windows::Msg::OpenChat(id))))
            }
            _ => Some(self.open_settings(self.active_settings_tab)),
        });
        Task::batch(tasks.collect::<Vec<_>>())
    }

    pub fn title(&self, id: window::Id) -> String {
        match () {
            _ if Some(id) == self.settings_window => tr("Settings"),
            // Desktop's `Gyt` title, which it does not translate.
            _ if self.pip.as_ref().is_some_and(|p| p.0 == id) => "Picture in Picture",
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
            // Desktop's `AddAccountsBrowserWindow` title, which it does not translate.
            _ if Some(id) == self.networks.window => "Add an Account",
            _ if Some(id) == self.how_window => tr("How It Works"),
            _ if self.nickname_windows.iter().any(|d| d.id == id) => tr("Account Nickname"),
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
            _ if let Some((.., list)) = self.space_windows.iter().find(|w| w.0 == id) => {
                let title = list.title(&self.store.read());
                return if title.is_empty() { "Beeper".into() } else { title };
            }
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
            // Desktop's `threadsForThreadList`: Reminders, Send Later and Snoozed have no pinned grid.
            .partition(|c| {
                self.list().active_tab.filterable()
                    && self.list().active_tab != InboxTab::Snoozed
                    && c.flags & CHAT_PINNED != 0
            });
        pinned.sort_by_key(|c| (st.pin_rank.get(&c.id).copied().unwrap_or(usize::MAX), c.last_activity.clone()));
        match st.settings.on("regularPinned") {
            true => (vec![], pinned.into_iter().chain(rest).collect()),
            false => (pinned, rest),
        }
    }

    pub fn visible_chats<'a>(&self, st: &'a BeeperStore) -> Vec<&'a Chat> {
        self.chats_with(st, self.list().filters)
    }

    fn list_key(&self, filters: u8) -> ListKey {
        (
            self.list().active_tab,
            filters,
            self.list().selected_account_filter.clone(),
            self.list().space_view_account.clone(),
        )
    }

    /// The chats of the list with `filters`, computed once for each store generation.
    fn chats_with<'a>(&self, st: &'a BeeperStore, filters: u8) -> Vec<&'a Chat> {
        let key = self.list_key(filters);
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
        let key = self.list_key(self.list().filters);
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
        // Desktop's `nextReminderAtMs`: the sooner of the chat reminder and the earliest message reminder.
        let remind_at = |i: usize| {
            let c = &st.chats[i];
            let chat = c.reminder.as_ref().map(|r| r.remind_at_ms);
            chat.into_iter().chain(self.message_reminder_at(st, c)).min().unwrap_or(0)
        };
        let snoozed = |i: usize| st.chats[i].snoozed_until_ms.unwrap_or(i64::MAX);
        // Desktop's `activityTimestamp`: the newer of the activity and the draft.
        let activity = |i: usize| beeper_core::store::activity_key(&st.chats[i]).0;
        match (self.list().active_tab, st.settings.reminders_order) {
            (InboxTab::Reminders, 0) => list.sort_by_key(|&i| (remind_at(i), i)),
            (InboxTab::Reminders, 1) => list.sort_by_key(|&i| (std::cmp::Reverse(remind_at(i)), i)),
            (InboxTab::Reminders, 2) => list.sort_by_key(|&i| (activity(i), i)),
            (InboxTab::SendLater, _) => list.sort_by_key(|&i| (next_send(st, &st.chats[i].id), i)),
            (InboxTab::Snoozed, _) => list.sort_by_key(|&i| (snoozed(i), i)),
            _ => list.sort_by_key(|&i| (std::cmp::Reverse(activity(i)), i)),
        }
        list
    }

    /// Whether the chat list with `filters` shows `c`.
    fn listed(&self, st: &BeeperStore, filters: u8, c: &Chat) -> bool {
        let on = |i: usize| filters & 1 << i != 0;
        let shown = |i: usize| on(i) || !filter_hit(i, c, st);
        c.merged_into_chat_id.is_none()
            && self.list().selected_account_filter.as_ref().is_none_or(|a| in_rail_entry(st, a, c))
            // Desktop's `inSpaceViewAccountFilter`: an account takes in the chats of its network spaces too.
            && self.list().space_view_account.as_ref().is_none_or(|a| match network_space(st, a) {
                Some(_) => in_rail_entry(st, a, c),
                None => &c.account_id == a,
            })
            && match self.list().active_tab {
                InboxTab::AllChats => {
                    let hidden = if self.list().selected_account_filter.is_some() { 0 } else { CHAT_HIDDEN };
                    // Desktop's `shouldShowInAllChats`: a snoozed chat leaves the inbox until a message reminder is
                    // due.
                    let snoozed = self.list().selected_account_filter.is_none()
                        && reminders::snoozed_until(st, c).is_some()
                        && !self.due_message_reminder(st, c);
                    c.flags & hidden == 0
                        && !snoozed
                        && shown(F_ARCHIVED)
                        && shown(F_LOW_PRIORITY)
                        && shown(F_REQUESTS)
                }
                InboxTab::Requests => c.flags & CHAT_REQUEST != 0,
                InboxTab::Archived => c.archived_in_list() && shown(F_REQUESTS),
                InboxTab::LowPriority => c.flags & CHAT_LOW_PRIORITY != 0 && shown(F_ARCHIVED) && shown(F_REQUESTS),
                InboxTab::Reminders => c.reminder.is_some() || self.message_reminder_at(st, c).is_some(),
                InboxTab::SendLater => st.scheduled.iter().any(|s| s.chat_id == c.id),
                InboxTab::Snoozed => {
                    reminders::snoozed_until(st, c).is_some()
                        && c.flags & CHAT_HIDDEN == 0
                        && shown(F_ARCHIVED)
                        && shown(F_LOW_PRIORITY)
                        && shown(F_REQUESTS)
                }
            }
            && (!self.list().active_tab.filterable()
                || filters == 0
                || (0..FILTERS.len()).any(|i| on(i) && filter_hit(i, c, st)))
    }

    pub fn select_chat(&mut self, chat_id: &str) {
        self.leave_draft();
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
        let id = st.chats[idx].id.clone();
        st.load_chat(&id);
        self.pane.unread_divider = message::divider_at(&st, &st.chats[idx], &self.pane_msgs(&st));
        // Desktop reads a chat with an unread divider from there, through the last message in view (`onRangeChange`).
        if !st.incognito() && self.pane.unread_divider.is_none() {
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
        (self.info_media, self.info_zoom) = (None, false);
        self.info_query.clear();
        self.info_note = text_editor::Content::with_text(chat.note.as_deref().unwrap_or_default());
        let pending = st.scheduled.iter().any(|s| s.chat_id == chat.id && !s.canceled);
        let later = draft.send_later.filter(|&(at, _)| at > now_unix_millis());
        (self.pane.send_later, self.pane.sched_fold, self.pane.sched_menu) = (later, (pending, pending), None);
        (self.pane.reply_thread, self.pane.thread, self.pane.thread_menu) = (None, None, None);
        self.unread_view = false;
        // A running load or retry of the last chat goes on to this chat when it ends.
        (self.pane.top.shown, self.pane.top.tries) = (false, 0);
        // Desktop's `maybeCreatePip`: a playing video goes on in Picture in Picture when its chat closes.
        match self.video.as_ref() {
            _ if self.pip.is_some() => {}
            Some(v) if v.playing() && !v.is_full() => self.pip_wanted = true,
            _ => self.video = None,
        }
        drop(st);
        if let Some(root) = self.open_threads.get(chat_id).cloned() {
            self.open_thread(chat_id.into(), root);
        }
        self.refresh_theme_and_tray();
    }

    /// Desktop's `createThread`: selects the most recent direct chat with the user on the account, or asks the
    /// bridge to make one.
    pub(crate) fn open_dm(
        &mut self,
        account: String,
        user: User,
        draft: Option<u32>,
        quote: Option<String>,
    ) -> Task<UiMessage> {
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
        let bridge = self.bridge(&account);
        let id = match bridge {
            beeper_core::matrix::ChatBridge::Matrix(_) => user.id,
            beeper_core::matrix::ChatBridge::Cloud(_) => phone.unwrap_or(user.id),
        };
        let done = move |r| UiMessage::NewChat(new_chat::Msg::DmCreated(draft, account.clone(), quote.clone(), r));
        Task::perform(bridge.create_dm(id), done)
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
        let cid = &st.chats[st.resolve_chat_idx(cid).expect("an open chat")].id;
        // A merged chat shows the replies of a member chat as that chat does.
        let all = st.chat_timeline(cid).into_iter();
        all.filter(|m| !m.thread_only(st.threaded(st.chat(&m.chat_id)))).collect()
    }

    /// The arrow keys select messages when no panel is open and the composer is empty.
    pub(crate) fn quick_react(&self, mid: String) -> UiMessage {
        let chat = self.pane().selected_chat_id.clone().expect("an open chat");
        let st = self.store.read();
        // Desktop's `REACT_WITH`: only a reaction that the network takes.
        match crate::emoji::allowed_reaction(&st.chat(&chat).allowed_reactions, &st.settings.quick_reaction) {
            Some(key) => UiMessage::Message(message::Msg::ToggleReaction(chat, mid, key)),
            None => UiMessage::Noop,
        }
    }

    /// The width of the rail and the chat list. Desktop keeps it when the space bar hides.
    fn left_width(&self) -> f32 {
        if !self.sidebar_shown() {
            0.0
        } else {
            68.0 + self.sidebar_width
        }
    }

    /// The width of the chat list: Desktop's `threads` width, which takes the space of a hidden space bar.
    fn list_width(&self, st: &BeeperStore) -> f32 {
        self.sidebar_width + if st.settings.on("showSidebar") && !self.list().locked { 0.0 } else { 68.0 }
    }

    /// The chat list shows in the drawn window: Desktop opens a space window with its sidebar shown.
    fn sidebar_shown(&self) -> bool {
        !self.sidebar_hidden || self.list().locked
    }

    /// Desktop's `canStartEditMessage` and `canEditMessage` of `m`.
    fn can_edit(&self, m: &Message) -> bool {
        let st = self.store.read();
        let chat = st.chat(&m.chat_id);
        editable(m, chat) && edit_open(m, chat.messages)
    }

    /// The newest message you can edit in the open chat.
    fn last_own_message(&self) -> Option<Message> {
        let st = self.store.read();
        let id = self.pane().selected_chat_id.as_deref().expect("an open chat");
        let chat = st.chat(id);
        let own = st
            .timeline_indices(id)
            .iter()
            .map(|&i| &st.messages[i])
            .rfind(|m| editable(m, chat) && edit_open(m, chat.messages));
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
        let (id, before) = (chat.id.clone(), st.chat_timeline(&chat.id).len());
        drop(st);
        self.pane.top.loading = Some((std::time::Instant::now(), id.clone(), before));
        let load = beeper_core::matrix::load_older(self.store.clone(), id);
        Task::perform(load, |a0| UiMessage::Timeline(timeline::Msg::LoadedOlder(a0)))
    }

    /// Desktop's `TopOfThread` loads again when the chat gets older messages while its top is on screen, as when the
    /// first sync of a joined room brings a page: in the main pane and in each chat window.
    fn load_older_everywhere(&mut self) -> Task<UiMessage> {
        let mut tasks = vec![self.load_older()];
        for id in self.chat_windows.iter().map(|w| w.0).collect::<Vec<_>>() {
            self.swap_window(id);
            let task = self.load_older();
            self.swap_window(id);
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
        let count = open.as_ref().map(|c| st.chat_timeline(c).len());
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
        Task::perform(async move { tokio::time::sleep(wait).await }, |()| UiMessage::Timeline(timeline::Msg::LoadOlder))
    }

    /// How far the sidebar's slide is, `transition: all 0.2s cubic-bezier(0.165, 0.84, 0.44, 1)`, while it runs.
    pub(crate) fn sidebar_progress(&self) -> Option<f32> {
        let t = self.sidebar_slide?.elapsed().as_secs_f32() / 0.2;
        (t < 1.0).then(|| crate::style::cubic_bezier(t, [0.165, 0.84, 0.44, 1.0]))
    }

    fn set_sidebar_hidden(&mut self, hidden: bool) {
        if self.sidebar_hidden != hidden {
            self.sidebar_hidden = hidden;
            self.sidebar_slide = Some(std::time::Instant::now());
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

    /// The chat list of the space window that `view` draws, else that of the main window; in `update`, the list of
    /// the window that sent the message.
    fn list(&self) -> &chat_list::ListState {
        let drawn = self.drawing.get().and_then(|id| self.space_windows.iter().find(|w| w.0 == id));
        drawn.map_or(&self.list, |w| &w.2)
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
        let task = Task::batch([task, self.view_requests(), self.pip()]);
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

    fn handle(&mut self, msg: UiMessage) -> Task<UiMessage> {
        // Another key while Alt is down keeps the release of Alt from toggling the menu bar.
        if let UiMessage::WindowKey(_, key, _) = &msg {
            self.alt_alone &= *key == Key::Named(Named::Alt);
        }
        // Desktop's `persistDraftRecord` after a change of the attachments, the quote, Send Later or the link previews.
        self.pane.draft_due |= matches!(
            msg,
            UiMessage::Composer(composer::Msg::FilesLoaded(_))
                | UiMessage::Composer(composer::Msg::Pasted(_))
                | UiMessage::Composer(composer::Msg::DiscardAttachment(_))
                | UiMessage::Gifs(gifs::Msg::Downloaded(..))
                | UiMessage::Gifs(gifs::Msg::StickerLoaded(..))
                | UiMessage::Composer(composer::Msg::StartReply(_))
                | UiMessage::Composer(composer::Msg::CancelMode)
                | UiMessage::Scheduled(scheduled::Msg::Set(_))
                | UiMessage::Composer(composer::Msg::RemoveLinkPreview(_))
        );
        self.pane.thread_focus &= !activates(&msg);
        match msg {
            // Each domain handles its own messages in its module.
            UiMessage::Accounts(msg) => return self.accounts(msg),
            UiMessage::Networks(msg) => return self.networks(msg),
            UiMessage::Blocking(msg) => return self.blocking(msg),
            UiMessage::Merged(msg) => return self.merged(msg),
            UiMessage::Nickname(msg) => return self.nickname(msg),
            UiMessage::ChatList(msg) => return self.chat_list(msg),
            UiMessage::ChatTheme(msg) => return self.chat_theme(msg),
            UiMessage::CommandBar(msg) => return self.command_bar(msg),
            UiMessage::Composer(msg) => return self.composer(msg),
            UiMessage::Forward(msg) => return self.forward(msg),
            UiMessage::Gallery(msg) => return self.gallery(msg),
            UiMessage::Gifs(msg) => return self.gifs(msg),
            UiMessage::Hotkeys(msg) => return self.hotkeys(msg),
            UiMessage::Info(msg) => return self.info(msg),
            UiMessage::Integrations(msg) => return self.integrations(msg),
            UiMessage::Labels(msg) => return self.labels(msg),
            UiMessage::Loaded(msg) => return self.loaded(msg),
            UiMessage::Media(msg) => return self.media_view(msg),
            UiMessage::Members(msg) => return self.members(msg),
            UiMessage::MenuBar(msg) => return self.menu_bar(msg),
            UiMessage::Message(msg) => return self.message(msg),
            UiMessage::NewChat(msg) => return self.new_chat(msg),
            UiMessage::Notify(msg) => return self.notify_event(msg),
            UiMessage::Pins(msg) => return self.pins(msg),
            UiMessage::Rail(msg) => return self.rail(msg),
            UiMessage::Record(msg) => return self.record(msg),
            UiMessage::Reminders(msg) => return self.reminders(msg),
            UiMessage::TimeSuggestions(msg) => return self.time_suggestions(msg),
            UiMessage::Scheduled(msg) => return self.scheduled(msg),
            UiMessage::Settings(msg) => return self.settings(msg),
            UiMessage::Thread(msg) => return self.threads(msg),
            UiMessage::Timeline(msg) => return self.timeline_scroll(msg),
            UiMessage::Toast(msg) => return self.toasts(msg),
            UiMessage::UserCard(msg) => return self.user_card(msg),
            UiMessage::Video(msg) => return self.video(msg),
            UiMessage::ViewOnce(msg) => return self.view_once(msg),
            UiMessage::Voice(msg) => return self.voice(msg),
            UiMessage::Window(msg) => return self.windows(msg),
            UiMessage::Zoom(msg) => return self.set_zoom(msg),
            UiMessage::SelectChat(_) if self.recorder.is_some() => {
                self.status_banner = tr("Stop recording to switch chats").into();
            }
            // Desktop's composer `autofocus = "end"` when a chat opens.
            UiMessage::SelectChat(id) => {
                self.select_chat(&id);
                let (previews, unread) = (self.fetch_link_previews(), self.scroll_to_unread());
                let bookmarks = self.retry_bookmarks();
                return Task::batch([
                    previews,
                    bookmarks,
                    self.close_notifications(),
                    operation::focus("composer-input"),
                    unread,
                ]);
            }
            UiMessage::SwitchCard => self.list.chat_menu = Some((SWITCH_CARD.into(), self.cursor.get())),
            UiMessage::SwitchChat(id) => self.switch_chat(id),
            UiMessage::InThread(msg) => return self.in_thread(*msg),
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
                self.list.chat_menu = None;
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
            UiMessage::OpenEditGroup(chat) => {
                self.list.chat_menu = None;
                if let Some(d) = &self.group_window {
                    return window::gain_focus(d.id);
                }
                let (id, open) = self.open_child(GROUP_SIZE);
                let st = self.store.read();
                let c = &st.chats[st.resolve_chat_idx(&chat).expect("an edited group")];
                let (title, topic) =
                    (c.room_title().to_string(), composer_with(c.description.as_deref().unwrap_or_default()));
                drop(st);
                self.group_window = Some(GroupDraft {
                    id,
                    chat,
                    title,
                    topic,
                    photo: None,
                    hover: false,
                    personal: false,
                    saving: false,
                    dropping: false,
                });
                return open;
            }
            UiMessage::OpenEditForMe(chat) => return self.open_edit_for_me(chat),
            UiMessage::Group(e) => return self.group_edit(e),
            UiMessage::Profile(m) => return self.update_profile(m),
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
            UiMessage::InputEscape(id) if Some(id) == self.settings_window && self.integrations.add.is_some() => {
                self.integrations.add = None;
            }
            UiMessage::InputEscape(id) if Some(id) == self.settings_window => {
                return self.update(UiMessage::Settings(settings::Msg::Query(String::new())));
            }
            UiMessage::InputEscape(id) => {
                return self.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), keyboard::Modifiers::empty()));
            }
            UiMessage::WindowText(id, text) if self.chat_windows.iter().any(|w| w.0 == id) => {
                return self.update(UiMessage::InWindow(id, Box::new(UiMessage::Composer(composer::Msg::Type(text)))));
            }
            UiMessage::WindowText(id, text) if Some(id) == self.main_window => {
                return self.update(UiMessage::Composer(composer::Msg::Type(text)));
            }
            UiMessage::WindowText(id, text) if Some(id) == self.login_window => return self.login_text(&text),
            UiMessage::WindowText(..) => {}
            // Desktop's menu roles "close" and "minimize", whose accelerators work while the menu bar hides.
            UiMessage::WindowKey(id, Key::Character(c), mods)
                if mods == Modifiers::COMMAND && matches!(c.to_lowercase().as_str(), "w" | "m") =>
            {
                return match c.eq_ignore_ascii_case("w") {
                    true => self.update(UiMessage::Window(windows::Msg::CloseRequested(id))),
                    false => window::minimize(id, true),
                };
            }
            UiMessage::WindowKey(id, key, mods) if Some(id) == self.login_window || !self.store.read().signed_in() => {
                return self.login_key(key, mods);
            }
            UiMessage::WindowKey(_, Key::Named(Named::Escape), _) if self.menu_bar.is_some() => self.menu_bar = None,
            // Desktop's `onEscapePressed` of the Picture in Picture window.
            UiMessage::WindowKey(id, Key::Named(Named::Escape), _) if self.pip.as_ref().is_some_and(|p| p.0 == id) => {
                if self.pip_menu.take().is_none() {
                    return window::close(id);
                }
            }
            UiMessage::WindowKey(id, key, mods) => {
                if Some(id) == self.settings_window {
                    return self.settings_key(key, mods);
                }
                if self.label_asking(id) && key == Key::Named(Named::Escape) {
                    self.confirm = None;
                    return Task::none();
                }
                if self.label_settings.iter().any(|w| w.id == id) || Some(id) == self.how_window {
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
                    || self.nickname_windows.iter().any(|d| d.id == id)
                    || self.add_accounts.iter().any(|a| a.id == id)
                {
                    return if key == Key::Named(Named::Escape) { window::close(id) } else { Task::none() };
                }
                // Desktop's `onEscapePressed` closes the Add an Account window, after the menu of a network.
                if self.networks.window == Some(id) {
                    if key == Key::Named(Named::Escape) && self.networks.menu.take().is_none() {
                        return window::close(id);
                    }
                    return Task::none();
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
                        return self.update(UiMessage::Media(media_view::Msg::Search(q)));
                    } else if w.select {
                        (w.select, w.picked) = (false, Vec::new());
                    } else {
                        return self.update(UiMessage::Window(windows::Msg::CloseRequested(id)));
                    }
                }
            }
            UiMessage::Commands(msgs) => {
                return Task::batch(msgs.into_iter().map(|m| self.update(m)).collect::<Vec<_>>())
            }
            UiMessage::RunHotkey(name) => {
                self.list.chat_menu = None;
                return self.run_hotkey(name, 0);
            }
            UiMessage::Ask(dialog) => (self.confirm, self.list.reminder_menu) = (dialog, None),
            UiMessage::Logout => {
                self.confirm = None;
                return Task::perform(beeper_core::matrix::logout(self.store.clone()), UiMessage::LoggedOut);
            }
            UiMessage::LoggedOut(Ok(())) => restart(),
            UiMessage::LoggedOut(Err(e)) => self.fail(Toast::new(format!("Could not log out: {e}"))),
            UiMessage::ToggleRightPanel => {
                // Desktop shows Chat Info or the thread pane, not both.
                let closed = self.pane.thread.take().is_some();
                self.show_right_panel = closed || !self.show_right_panel;
                let retry = self.retry_bookmarks();
                return Task::batch([retry, self.settle()]);
            }
            UiMessage::ToggleDnd => {
                self.dnd_muted = !self.dnd_muted;
                self.refresh_theme_and_tray();
            }
            UiMessage::Capabilities(account, r) => match r {
                Ok(v) => drop(self.caps.insert(account, v)),
                Err(e) => eprintln!("provisioning capabilities failed: {e}"),
            },
            UiMessage::QuickEmoji(Some(slot)) => {
                let open = self.open_emoji(None);
                self.pane.emoji.as_mut().expect("the picker is open").pref = Some(slot);
                return open;
            }
            UiMessage::QuickEmoji(None) => self.pane.emoji = None,
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
                let failed = self.notify_failed(&m);
                return Task::batch([self.update(UiMessage::StoreChanged), note, failed]);
            }
            UiMessage::ChatUpserted(c) => {
                let note = self.notify_chat(&c);
                return Task::batch([self.update(UiMessage::StoreChanged), note]);
            }
            UiMessage::Typing(chat) => {
                self.typing_changed(&chat);
                let note = self.notify_typing(chat);
                return Task::batch([self.update(UiMessage::StoreChanged), note]);
            }
            UiMessage::AnimationTick => {}
            UiMessage::Pushed(Ok(())) => {}
            UiMessage::DismissRecoveryReminder => {
                let now = beeper_core::config::now_unix_millis();
                self.patch_settings(|s| s.recovery_reminder_dismissed_at = now);
            }
            UiMessage::Pushed(Err(e)) => self.fail(Toast::new(format!("Could not send to Matrix: {e}"))),
            UiMessage::OpenQuickLook(title, detail) => {
                self.quick_look_target = Some((title, detail));
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
                    || self.menu_open()
                    || self.pane.gallery_reactions.is_some()
                    || self.pane.picker.is_some()
                    || self.pane.emoji.is_some()
                    || self.quick_look_target.is_some()
                    || self.hotkeys_modal.is_some()
                {
                    self.hotkeys_modal = None;
                    self.show_command_palette = false;
                    self.forward = None;
                    self.list.show_filter_menu = false;
                    self.list.chat_menu = None;
                    // Desktop unselects the gallery when its menu closes without an action.
                    if self.pane.message_menu.take().is_some() && !self.pane.gallery_menu.is_empty() {
                        self.pane.selected_msgs.clear();
                    }
                    self.pane.gallery_reactions = None;
                    self.pane.thread_menu = None;
                    self.unread_menu = None;
                    self.pane.edit_menu = None;
                    self.pane.sched_menu = None;
                    self.list.reminder_menu = None;
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
                } else if self.confirm == Some(ASK_ROOM_ENCRYPTION) {
                    self.confirm = None;
                } else if self.creating_chat.is_some() {
                    self.leave_draft();
                    self.pane.selected_chat_id = None;
                } else if self.pane.reply_to_message_id.is_some() {
                    self.pane.reply_to_message_id = None;
                } else if self.pane.editing_message_id.is_some() {
                    return self.update(UiMessage::Composer(composer::Msg::CancelMode));
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
                } else if self.list().active_tab != InboxTab::AllChats {
                    self.list.active_tab = InboxTab::AllChats;
                    self.list.space_view_account = None;
                } else {
                    self.status_banner.clear();
                }
            }
            UiMessage::Noop => {}
            UiMessage::Relist => {
                // Above its bottom, the timeline reads through its last row in view; at the bottom its sensor reads.
                if self.pane.scrolled_up.is_some_and(|(up, _)| up) {
                    self.read_open_chat();
                }
                return Task::none();
            }
            UiMessage::LoadWanted => return Task::batch([self.load_avatars(), self.load_media(), self.load_quotes()]),
            UiMessage::OpenUrl(url) if url.starts_with(SPOILER) => {
                self.pane.spoilers.insert(url[SPOILER.len()..].to_string());
            }
            // Desktop's `b2t`: a click on a mention opens its menu.
            UiMessage::OpenUrl(url) if url.starts_with(MENTION) => {
                self.list.chat_menu = Some((format!("mention:{}", &url[MENTION.len()..]), self.cursor.get()));
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
            UiMessage::ToggleIncognito => {
                let on = !self.store.read().settings.on("incognito");
                let task = self.update(UiMessage::Settings(settings::Msg::SetPref("incognito", u8::from(on))));
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
                    UiMessage::Toast(toasts::Msg::DismissBanner),
                    UiMessage::Settings(settings::Msg::SetPref("typingIndicators", u8::from(!typing))),
                ]);
                let (buttons, right) =
                    if action.is_empty() { (Vec::new(), Some(undo)) } else { (vec![(tr(action), typing), undo], None) };
                self.show(Toast { buttons, right, ..Toast::new(tr(text).into()) });
                return task;
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
            UiMessage::InWindow(id, msg) => {
                if self.chat_windows.iter().all(|w| w.0 != id) {
                    return self.handle(*msg);
                }
                self.swap_window(id);
                let task = self.handle(*msg);
                self.swap_window(id);
                let task = task.map(move |m| UiMessage::InWindow(id, Box::new(m)));
                // A chat window closes when it leaves its chat; a space window stays on its space.
                let left = self.chat_windows.iter().any(|w| w.0 == id && w.1.selected_chat_id.is_none());
                return match left && !self.is_space_window(id) {
                    false => task,
                    true => Task::batch([task, window::close(id)]),
                };
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
            UiMessage::StoreChanged if self.store.read().session_ended => restart(),
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
                    (self.pane.selected_chat_id, self.list.chat_menu) = (None, None);
                    self.select_chat(NOTE_CHAT_ID);
                }
                self.refresh_theme_and_tray();
                let older = self.load_older_everywhere();
                return Task::batch([self.close_notifications(), self.verification_changed(), self.leave_app(), older]);
            }
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
            UiMessage::KeyPressed(Key::Named(Named::Escape), _) if self.view_once.is_open() => {
                return self.view_once(view_once::Msg::Dismiss);
            }
            UiMessage::KeyPressed(key, mods) if self.carousel.as_ref().is_some_and(|c| !c.in_media) => {
                return self.carousel_key(&key, mods);
            }
            // Desktop's `C_t` takes every key while a pending attachment has the focus.
            UiMessage::KeyPressed(key, _) if self.pane.att_focus.is_some() => {
                return self.update(UiMessage::Composer(composer::Msg::AttachmentKey(key)));
            }
            UiMessage::KeyPressed(key, mods) => {
                let z = mods.command() && matches!(key.as_ref(), Key::Character(c) if c.eq_ignore_ascii_case("z"));
                if z && !mods.shift() && self.shown_undo().is_none() && !self.undo_stack.is_empty() {
                    return self.update(UiMessage::UndoAction);
                }
                if z && (self.queued_here() || self.shown_undo().is_some()) {
                    return self.update(UiMessage::Composer(composer::Msg::Undo(mods.shift())));
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
                        Key::Named(Named::Tab) => {
                            return self.update(UiMessage::Composer(composer::Msg::PickSuggestion(at)))
                        }
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
                        return locate(format!("cmd-{}", self.cmd_index), commands::COMMANDS, Align::Nearest);
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
                        let step = self.update(UiMessage::Message(message::Msg::Step(down, mods.shift())));
                        let Some(m) = edit.then(|| self.last_own_message()).flatten() else { return step };
                        self.pane.selected_msgs = vec![m.id.clone()];
                        return step.chain(
                            self.update(UiMessage::Composer(composer::Msg::StartEdit(
                                m.id,
                                m.text.unwrap_or_default(),
                            ))),
                        );
                    }
                    Key::Named(Named::Enter) if self.forward.is_some() => {
                        let picked = !self.forward.as_ref().expect("forward").picked.is_empty();
                        return match mods.control() {
                            false => self.update(UiMessage::Forward(forward::Msg::Submit)),
                            true if picked => self.update(UiMessage::Forward(forward::Msg::Send)),
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
                        return self.update(UiMessage::NewChat(new_chat::Msg::Start));
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
                        return self.update(UiMessage::Message(message::Msg::React(
                            self.single_msg().cloned().expect("a selected message"),
                        )));
                    }
                    Key::Character("c") if mods.command() && !mods.shift() && selectable::has_selection() => {
                        return iced::clipboard::write(selectable::text());
                    }
                    Key::Character("c") if mods.command() && !mods.shift() && !self.pane.selected_msgs.is_empty() => {
                        let stamp = self.store.read().settings.on("copyTimestamps");
                        return self.update(UiMessage::Message(message::Msg::CopySelected(stamp)));
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
                            && self.list().active_tab != InboxTab::Requests =>
                    {
                        (self.selection, self.select_mode) = (self.list_ids(&self.store.read()), false);
                        if let Some(first) = self.selection.first().cloned() {
                            self.select_chat(&first);
                        }
                        return Task::none();
                    }
                    Key::Character("v") if mods.command() && self.pane.selected_chat_id.is_some() => {
                        return self.update(UiMessage::Composer(composer::Msg::Paste));
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
            Task::perform(fetch, move |m| UiMessage::Info(info::Msg::BookmarkHydrated(t.clone(), m.map(Box::new))))
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
            feed(f.0.read().focus_tx.subscribe(), |a0| UiMessage::Window(windows::Msg::FocusRequested(a0)))
        });
        let tokens = Subscription::run_with(StoreFeed(self.store.clone(), 2), |f| {
            feed(f.0.read().connect_tx.subscribe(), |(id, outcome)| {
                UiMessage::Integrations(integrations::Msg::Request(id, outcome))
            })
        });
        let launches = Subscription::run_with(StoreFeed(self.store.clone(), 3), |f| {
            feed(f.0.read().launch_tx.subscribe(), |a0| UiMessage::Window(windows::Msg::SecondInstance(a0)))
        });
        let keys = iced::event::listen_with(|e, status, id| match e {
            Event::Window(window::Event::Closed) => Some(UiMessage::Window(windows::Msg::Closed(id))),
            Event::Window(window::Event::CloseRequested) => Some(UiMessage::Window(windows::Msg::CloseRequested(id))),
            Event::Window(window::Event::Resized(size)) => Some(UiMessage::Window(windows::Msg::Resized(id, size))),
            Event::Window(window::Event::Focused) => Some(UiMessage::Window(windows::Msg::Focused(true))),
            Event::Window(window::Event::Unfocused) => Some(UiMessage::Window(windows::Msg::Focused(false))),
            Event::Window(window::Event::FileHovered(_)) => {
                Some(UiMessage::Composer(composer::Msg::FileHover(id, true)))
            }
            Event::Window(window::Event::FilesHoveredLeft) => {
                Some(UiMessage::Composer(composer::Msg::FileHover(id, false)))
            }
            Event::Window(window::Event::FileDropped(path)) => {
                Some(UiMessage::Composer(composer::Msg::FileDropped(id, path)))
            }
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
        let system = iced::system::theme_changes().map(|a0| UiMessage::Settings(settings::Msg::SystemTheme(a0)));
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
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(UiMessage::Rail(rail::Msg::ResizeMoved(position)))
                }
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Rail(rail::Msg::ResizeEnd)),
                _ => None,
            })
        });
        let thread_drag = self.thread_drag.then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(UiMessage::Thread(threads::Msg::Drag(Some(position))))
                }
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Thread(threads::Msg::Drag(None))),
                _ => None,
            })
        });
        let rail_press = (self.rail_hover.is_some() && self.rail_drag.is_none()).then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    Some(UiMessage::Rail(rail::Msg::Press))
                }
                _ => None,
            })
        });
        let pin_press = (self.pin_hover.is_some() && self.pin_drag.is_none()).then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    Some(UiMessage::Pins(pins::Msg::Press))
                }
                _ => None,
            })
        });
        let pin_drag = self.pin_drag.is_some().then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(UiMessage::Pins(pins::Msg::DragMoved(position)))
                }
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Pins(pins::Msg::DragEnd)),
                _ => None,
            })
        });
        let rail_drag = self.rail_drag.is_some().then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(UiMessage::Rail(rail::Msg::DragMoved(position)))
                }
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Rail(rail::Msg::DragEnd)),
                _ => None,
            })
        });
        let poll_drag = self.poll_windows.iter().any(|d| d.dragging.is_some()).then(|| {
            iced::event::listen_with(|e, _, id| match e {
                Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(UiMessage::Poll(id, PollEdit::Grab(None))),
                _ => None,
            })
        });
        let time_drag = self.time_edit.drag.is_some().then(|| {
            iced::event::listen_with(|e, _, _| match e {
                Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                    Some(UiMessage::TimeSuggestions(time_suggestions::Msg::Grab(None)))
                }
                _ => None,
            })
        });
        let (rising, due) = {
            let st = self.store.read();
            let last = self.pane().selected_chat_id.as_ref().and_then(|c| st.chat_timeline(c).last().copied());
            let rising = last.is_some_and(|m| send_rise(&st, m).is_some());
            let pending = st.chats.iter().filter(|c| !self.expired(c));
            let send = st.scheduled.iter().filter(|s| !s.canceled).map(|s| s.send_on);
            let send = send.chain(self.pane.send_later.map(|c| c.0)).min();
            let chats = pending.filter_map(|c| Some(c.reminder.as_ref()?.remind_at_ms));
            let messages = (st.message_reminders.iter())
                .filter(|(id, r)| {
                    self.reminded.get(*id) != Some(&r.remind_at_ms) && reminders::message_reminders_on(&st)
                })
                .map(|(_, r)| r.remind_at_ms);
            let snoozes = st.chats.iter().filter_map(|c| c.snoozed_until_ms).filter(|&at| at > self.snooze_clock);
            (rising, (chats.chain(messages).chain(snoozes).min(), send))
        };
        let wait = |at: i64| std::time::Duration::from_millis((at - now_unix_millis()).max(1) as u64);
        let late = {
            let st = self.store.read();
            let open = self.panes().filter_map(|p| p.selected_chat_id.as_ref()).map(|c| st.chat_timeline(c));
            let echoes = open.flatten();
            let echoes = echoes.filter(|m| m.send_error.is_none() && pending(&st, m));
            echoes.map(still_sending_at).filter(|&at| at > now_unix_millis()).min()
        };
        // "Sending..." turns into "Still sending..." at its time.
        let late = late.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::AnimationTick));
        // Desktop's `buildMessageListData` watches the clock until the next timer of the open chats runs out, or for
        // an hour at most.
        let expiry = {
            let (st, now) = (self.store.read(), now_unix_millis());
            let open = self.panes().filter_map(|p| p.selected_chat_id.as_ref()).map(|c| st.chat_timeline(c));
            let left = open.flatten().filter_map(|m| match m.expiry(now) {
                Some(Countdown::Left(left)) if left > 0 => Some(left),
                _ => None,
            });
            left.min().map(|left| iced::time::every(Duration::from_millis(left.min(3_600_000))))
        };
        let expiry = expiry.map(|e| e.map(|_| UiMessage::AnimationTick));
        let remind = due.0.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::Reminders(reminders::Msg::Check)));
        let send = due.1.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::Scheduled(scheduled::Msg::Check)));
        let typed = self.panes().filter_map(|p| p.typing.as_ref().map(|t| t.1)).min();
        let typing = typed.map(|t| {
            let left = TYPING_STOP.saturating_sub(t.elapsed()).max(Duration::from_millis(1));
            iced::time::every(left).map(|_| UiMessage::TypingTick)
        });
        let queued = self.queued.iter().map(|q| q.due).min();
        let queued =
            queued.map(|at| iced::time::every(wait(at)).map(|_| UiMessage::Composer(composer::Msg::SendQueued(None))));
        // The loading of older messages and the fade of the scroll button, in each chat window too.
        let moving =
            |p: &Pane| p.top.loading.is_some() || p.scrolled_up.is_some_and(|s| s.1.elapsed().as_secs_f32() < 0.2);
        let animated = rising
            || self.panes().any(moving)
            || self.flash.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 1.5)
            || self.typing_shown.values().any(|t| t.2.elapsed() < message::TYPING_FADE)
            || self.pref_flash.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 2.0)
            || (self.login_window.is_some() && (self.login.busy || self.login.animated()))
            || self.reauth.busy
            || self.enter_key.busy
            || self.nickname_windows.iter().any(|d| d.saving)
            || !self.pressing.is_empty()
            || !self.fetching.is_empty()
            || self.group_window.as_ref().is_some_and(|d| d.saving)
            || self.hover_fading()
            || self.sidebar_progress().is_some()
            || self
                .panes()
                .any(|p| [p.hover_msg_at, p.flyover_at].iter().flatten().any(|t| t.elapsed() < FLYOVER_REVEAL));
        // The tab slide and the skeleton tiles of the Media window.
        let media = self.media_window.as_ref().is_some_and(|w| {
            w.loading.is_some() || w.tab_at.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < TAB_SLIDE)
        });
        let animated = animated || media;
        let tray = (cfg!(target_os = "linux") && self.store.read().settings.on("showTrayIcon"))
            .then(|| Subscription::run(tray::events).map(|a0| UiMessage::Window(windows::Msg::Tray(a0))));
        let online = cfg!(target_os = "linux").then(|| Subscription::run(portal::online).map(UiMessage::Online));
        let notes = (!self.notifications.is_empty() || self.verify.note.is_some()).then(|| {
            Subscription::run(notify::events).map(|(id, action)| UiMessage::Notify(notify::Msg::Event(id, action)))
        });
        let toast =
            self.toast_running().then(|| iced::time::every(TOAST_TICK).map(|_| UiMessage::Toast(toasts::Msg::Tick)));
        let tick =
            animated.then(|| iced::time::every(std::time::Duration::from_millis(20)).map(|_| UiMessage::AnimationTick));
        let due = self.panes().any(|p| p.draft_due);
        let drafts = due.then(|| iced::time::every(std::time::Duration::from_secs(5)).map(|_| UiMessage::SaveDrafts));
        let shot = self.shot.map(|_| {
            iced::event::listen_raw(|e, _, id| {
                matches!(e, Event::Window(window::Event::RedrawRequested(_)))
                    .then_some(UiMessage::Window(windows::Msg::Drawn(id)))
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
                .chain(time_drag)
                .chain(carousel)
                .chain(tick)
                .chain(toast)
                .chain(shot)
                .chain(remind)
                .chain(late)
                .chain(expiry)
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
                .chain(self.video_ticks())
                .chain(self.view_once_ticks()),
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
        } else if self.pip.as_ref().is_some_and(|p| p.0 == id) {
            self.view_pip()
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
        } else if Some(id) == self.networks.window {
            self.view_add_accounts()
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
        } else if let Some(d) = self.nickname_windows.iter().find(|d| d.id == id) {
            self.view_nickname(d)
        } else if Some(id) == self.blocked_window {
            self.view_blocked_users()
        } else if Some(id) == self.how_window {
            self.view_how_it_works()
        } else if let Some((_, chat, mid)) = self.vote_windows.iter().find(|w| w.0 == id) {
            self.view_poll_votes(chat, mid)
        } else if self.chat_windows.iter().any(|w| w.0 == id && w.1.selected_chat_id.is_none())
            && !self.is_space_window(id)
        {
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
            .on_press(UiMessage::Rail(rail::Msg::ResizeStart))
            .on_double_click(UiMessage::Rail(rail::Msg::ResetWidth))
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
        // Desktop's `.left` transition: the space bar and the chat list slide out to nothing when they hide, at once
        // empty, and slide back in when they show.
        let left: El<'_> = match self.sidebar_progress() {
            Some(t) => {
                let full = self.sidebar_width + 68.0;
                let width = full * if self.sidebar_hidden { 1.0 - t } else { t };
                container(Row::new().push(rail).push(sidebar)).width(width).clip(true).into()
            }
            None => Row::new().push(rail).push(sidebar).into(),
        };
        let body = Row::new().push(left).push(stack![right, grip]);
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
                    let watch = mouse_area(Space::new().width(68).height(Fill))
                        .on_exit(UiMessage::Rail(rail::Msg::Float(false)));
                    stack![rail, watch].into()
                }
                false => mouse_area(Space::new().width(20).height(Fill))
                    .on_enter(UiMessage::Rail(rail::Msg::Float(true)))
                    .into(),
            }
        });
        stack![body].push(float).into()
    }

    pub fn view_main<'a>(&'a self) -> El<'a> {
        let st = self.store.read();
        // The sync task can drop a chat before StoreChanged reaches update.
        let known = |c: &str| {
            let chat = ["info:", "copy:", SNOOZE_MENU, behavior::BEHAVIOR_MENU].iter().find_map(|k| c.strip_prefix(k));
            let chat = chat.unwrap_or(c);
            let chat = chat.split_once('\n').map_or(chat, |k| k.0);
            let pair =
                ["member:", "bookmark:", "contact:", "card:", "mention:"].iter().find_map(|k| chat.strip_prefix(k));
            let chat = pair.map_or(chat, |k| k.split_once(' ').map_or(k, |k| k.0));
            c == RAIL_MORE
                || c == RAIL_MENU
                || c == SIDEBAR_MENU
                || c == SPACE_SHEET
                || c == SWITCH_CARD
                || c.starts_with("draft:")
                || c.starts_with("vcard:")
                || c.starts_with("account:")
                || c.starts_with("head:")
                || st.resolve_chat_idx(chat).is_some()
        };
        let shown = self.pane().selected_chat_id.as_deref().is_some_and(known);
        let open = self.creating_chat.is_none() && shown;
        let menu_open = self.list().chat_menu.as_ref().is_some_and(|(c, _)| known(c));
        drop(st);
        let in_main = self.drawing.get().is_none();
        // The menus of a chat list show in its window: the main window or a space window.
        let in_list = in_main || self.list().locked;
        let base = match (in_main, self.list().locked) {
            (true, _) => self.view_full_video(false).unwrap_or_else(|| self.view_body(open)),
            (false, true) => self.view_space_window(shown),
            (false, false) => container(match shown {
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
                .on_press(UiMessage::MenuBar(menubar::Msg::Show(None)))
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
            .on_press(UiMessage::CommandBar(commands::Msg::SearchSelect(None)))
        });
        let filter_menu =
            self.list().show_filter_menu.then(|| (self.view_filter_menu(), self.sidebar_width - 22.5, 26.0));
        let reminder_menu = self.list().reminder_menu.map(|at| (self.view_reminder_menu(), at.x, at.y));
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
        let chat_menu = self.list().chat_menu.as_ref().filter(|_| menu_open).map(|(cid, at)| {
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
            } else if let Some(id) = cid.strip_prefix(SNOOZE_MENU) {
                let rows = self.snooze_items(&self.store.read(), &[id.to_string()]);
                (SUB_MENU_W[3], 8.0 + rows.iter().map(|r| r.1).sum::<f32>())
            } else if let Some(id) = cid.strip_prefix(behavior::BEHAVIOR_MENU) {
                let (rows, w) = self.behavior_items(&self.store.read(), id);
                (w, 8.0 + rows.iter().map(|r| r.1).sum::<f32>())
            } else if cid == RAIL_MORE {
                let items = more_menu(&self.store.read());
                (233.0, 8.0 + items.iter().map(|i| if i.is_some() { MENU_ITEM } else { 11.0 }).sum::<f32>())
            } else if INFO_MENUS.iter().any(|k| cid.starts_with(k)) {
                let items = info_menu(&self.store.read(), cid, |a| self.can(a, "create_dm"));
                (233.0, 8.0 + items.iter().map(|i| if i.is_some() { MENU_ITEM } else { 11.0 }).sum::<f32>())
            } else if cid == SIDEBAR_MENU || cid == RAIL_MENU {
                (233.0, 8.0 + 4.0 * MENU_ITEM)
            } else if RAIL_VIEWS.iter().any(|v| cid.strip_prefix("account:") == Some(v.0)) {
                (233.0, 8.0 + 5.0 * MENU_ITEM)
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
                (233.0, 8.0 + (n + 3.0) * MENU_ITEM + 11.0)
            } else if let Some(entry) = cid.strip_prefix("account:") {
                let st = self.store.read();
                let nick = if nickname::can_nickname(&st, &rail_owner(&st, entry)) { MENU_ITEM } else { 0.0 };
                (233.0 + self.chat_sub.map_or(0.0, |_| ACCOUNT_SUB_W), 8.0 + 7.0 * MENU_ITEM + 22.0 + nick)
            } else {
                let st = self.store.read();
                (
                    233.0,
                    8.0 + 10.0 * MENU_ITEM
                        + 44.0
                        + if self.selection.is_empty() { 2.0 * MENU_ITEM + 11.0 } else { 0.0 }
                        + if st.feature("labels-v2").on { MENU_ITEM } else { 0.0 }
                        + if st.resolve_chat_idx(cid).is_some_and(|i| {
                            let targets = self.selected_chats(&st);
                            let targets = if targets.is_empty() { vec![&st.chats[i]] } else { targets };
                            reminders::shows_snooze(&st, &targets)
                        }) {
                            MENU_ITEM
                        } else {
                            0.0
                        }
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
                .filter(|_| {
                    !["account:", "head:", SNOOZE_MENU, behavior::BEHAVIOR_MENU].iter().any(|k| cid.starts_with(k))
                })
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
        let main_menu = filter_menu.or(reminder_menu).filter(|_| in_list).or(rate_menu.filter(|_| in_main));
        let main_menu = main_menu.or(chat_menu.filter(|_| in_list));
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
            match (self.show_command_palette || self.forward.is_some()) && self.list().chat_menu.is_some() {
                true => (None, main_menu.map(|m| layer(m, UiMessage::ChatList(chat_list::Msg::CloseMenu)))),
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
        // Desktop portals the zoomed Chat Info avatar over the whole window.
        let thread = self.selection.is_empty() && self.pane().thread.is_some();
        let zoom =
            (self.info_zoom && self.show_right_panel && in_main && open && !thread).then(|| self.view_avatar_zoom());
        let hotkeys = self.hotkeys_modal.as_deref().filter(|_| in_main).map(|q| self.view_hotkeys_modal(q));
        // The menus and the emoji picker of the carousel show over it.
        let hover = self.view_hover_card().filter(|_| in_main);
        let quick_look = self.view_attachment_quick_look().filter(|_| in_main);
        let layers = stack![base]
            .push(toast)
            .push(quick_look)
            .push(carousel)
            .push(zoom)
            .push(self.view_view_once().filter(|_| in_main))
            .push(hover)
            .push(menu_drop)
            .push(menu)
            .push(scrim)
            .push(palette_menu)
            .push(select);
        let token = self.token_ask.as_ref().map(|a| self.view_token_ask(a));
        layers.push(confirm).push(hotkeys).push(token).into()
    }

    /// Desktop's "Remove from Beeper" button of the delete dialog.
    fn remove_from_beeper(&mut self, ids: Vec<String>) {
        self.confirm = None;
        let mut st = self.store.write();
        ids.iter().for_each(|id| st.remove_from_beeper(id));
    }

    /// The message menu, the composer menus or the reaction picker of the drawn pane, with their place.
    fn pane_menu(&self) -> Option<(El<'_>, f32, f32)> {
        let message_menu = self.pane().message_menu.as_ref().map(|(mid, at)| {
            let (menu, h, top) = self.view_message_menu(mid);
            let x = if at.x + MESSAGE_MENU_W > self.pane().size.width { at.x - MESSAGE_MENU_W } else { at.x };
            let y = at.y.min(self.pane().size.height - h - 8.0);
            let (sub, sub_w) = match self.view_speed_menu(mid) {
                // Level with its row, under Fullscreen, Open in PiP, Download and Mute.
                Some(sub) => (Some(column![Space::new().height(4.0 * MENU_ITEM), sub].into()), video::SPEED_MENU_W),
                None if self.pane().remind_sub => {
                    let st = self.store.read();
                    let rows = self.message_remind_items(&st, mid);
                    let body = Column::with_children(rows.into_iter().map(|r| r.0)).padding([4, 0]);
                    let style = shadow_box(self.pal.menu, self.pal.menu_border, 6.0, 16.0);
                    let body = container(body).width(SUB_MENU_W[1]).style(style);
                    (Some(column![Space::new().height(top), body].into()), SUB_MENU_W[1])
                }
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

    /// Desktop's `searchStore.results` for the trimmed query `raw`: the chats by title and the chats by member, 13 at
    /// most to tell that there are more than 12; the 100 newest bookmarks and the 20 newest text messages; and the
    /// media, the 50 newest of the media filter or else the 10 newest images and videos. The query and the filters
    /// search with Desktop's operators. The memo keeps the finds for the query, the filters, the room and the
    /// partial matches until the store changes, so that only a key or a change of the store searches again.
    fn search_hits(&self, st: &BeeperStore, raw: &str) -> Hits {
        let (f, room) = (&self.search_filters, self.search_room.as_deref());
        let key = (raw.to_string(), f.clone(), self.search_room.clone(), self.search_partial);
        // A period filter moves with the clock, so its finds stay only for one view.
        let memo = self.memo().filter(|_| f.period.is_none());
        if let Some((_, hits)) = memo.and_then(|m| m.hits.borrow().clone()).filter(|(k, _)| *k == key) {
            return hits;
        }
        let q = raw.to_lowercase();
        let (order, words) = self.chat_words(st);
        let low = |i: usize| st.chats[i].flags & CHAT_LOW_PRIORITY != 0;
        let shown = |i: &usize| {
            let c = &st.chats[*i];
            c.flags & CHAT_HIDDEN == 0 && (f.accounts.is_empty() || f.accounts.contains(&c.account_id))
        };
        // Desktop's `sortResults`: a chat that matches by its network only comes last, then a low priority chat.
        let mut titled: Vec<usize> = match q.is_empty() {
            true => Vec::new(),
            false => order.iter().copied().filter(shown).filter(|&i| words[i].contains(&q)).collect(),
        };
        titled.sort_by_key(|&i| (!words[i].split('\0').next().is_some_and(|t| t.contains(&q)), low(i)));
        let chats_shown = !q.is_empty() && room.is_none() && f.media.is_empty();
        let people = |i: &usize| chats_shown && shown(i) && search::participants_match(&st.chats[*i], raw);
        let mut in_groups: Vec<usize> = order.iter().copied().filter(people).collect();
        in_groups.sort_by_key(|&i| low(i));
        // Desktop's `hasTextSearchQuery`.
        let text_query = raw.chars().count() > 1;
        let now = now_unix_millis();
        let ops = format!("{} {}", if text_query { raw } else { "" }, f.operators(room));
        let (opts, typed) = Search::parse(&ops, now);
        let query = |s: &Search, text: &str| match text.trim().is_empty() {
            true => s.query(st),
            false => s.query(st).phrase(text, self.search_partial),
        };
        let newest = |query: Query, n: usize, keep: &dyn Fn(&Message) -> bool| {
            let mut found = Vec::new();
            st.scan(&query, |m| {
                if m.is_deleted != Some(true) && keep(&m) {
                    found.push(m);
                }
                found.len() < n
            });
            found
        };
        let text = |m: &Message| m.text.as_deref().is_some_and(|t| !t.is_empty());
        let (bookmarks, messages) = match text_query && f.media.is_empty() {
            true => (newest(query(&opts, &typed).bookmarked(), 100, &text), newest(query(&opts, &typed), 20, &text)),
            false => Default::default(),
        };
        let media = match (f.media.is_empty(), text_query) {
            (true, false) => Vec::new(),
            (true, true) => {
                let (s, typed) = Search::parse(&format!("has:image has:video {ops}"), now);
                newest(query(&s, &typed), 10, &|_| true)
            }
            (false, _) => {
                let mut all = newest(query(&opts, &typed), 50, &|_| true);
                // Desktop's `matchedMediaRoomIDs`: the media of up to 20 chats whose titles match too.
                if text_query && room.is_none() && (1..=20).contains(&titled.len()) {
                    let mut s = Search::parse(&f.operators(None), now).0;
                    s.chats = titled.iter().map(|&i| st.chats[i].id.clone()).collect();
                    let ids: HashSet<String> = all.iter().map(|m| m.id.clone()).collect();
                    all.extend(newest(s.query(st), 50, &|m| !ids.contains(&m.id)));
                    all.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
                    all.truncate(50);
                }
                all
            }
        };
        let chats = if chats_shown { titled.into_iter().take(13).collect() } else { Vec::new() };
        in_groups.truncate(13);
        let hits = Rc::new(Found { chats, in_groups, bookmarks, media, messages });
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
                &default_account_user(&st, &c.account_id),
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
        (mouse_area(card).on_exit(UiMessage::ChatList(chat_list::Msg::CloseMenu)).into(), h)
    }

    /// Desktop's `composeSwitchTargets.onSelect`: opens the chat on the other account, and moves the draft there
    /// when that chat has none.
    fn switch_chat(&mut self, id: String) {
        self.list.chat_menu = None;
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
        Task::perform(wait, move |()| UiMessage::UserCard(user_card::Msg::Due(n)))
    }

    /// Where a user card `h` high shows for the pointer at `at`: centered on it, `gap` below it or above it near the
    /// bottom.
    fn card_spot(&self, at: Point, gap: f32, h: f32) -> (f32, f32) {
        let size = self.pane().size;
        let x = (at.x - CARD_W / 2.0).clamp(8.0, size.width - CARD_W - 8.0);
        let below = at.y + gap + h <= size.height;
        (x, if below { at.y + gap } else { at.y - gap - h })
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
            row![send.on_press(UiMessage::Members(members::Msg::Message(cid.into(), uid.into())))]
                .push(call)
                .spacing(8)
                .align_y(Center)
        });
        let high = high + if actions.is_some() { 36.0 } else { 0.0 };
        let card = column![Space::new().height(CARD_FACE_IN), info].push(actions).spacing(8).padding([8, 11]);
        let card = container(card).width(CARD_W).style(lifted(17.0));
        let face = container(face).center_x(CARD_W);
        (stack![column![Space::new().height(88.0 - 8.0 - CARD_FACE_IN), card], face].into(), high)
    }

    /// Desktop's timeline effect (`markAsRead` through the last message in view, from `onRangeChange`): while its
    /// window is focused, the open chat is read through the bottom message on screen, all of it at the bottom of the
    /// timeline, except in incognito mode.
    fn read_open_chat(&self) {
        let at_bottom = self.pane.scrolled_up.is_none_or(|(up, _)| !up);
        let Some(id) = self.pane.selected_chat_id.clone().filter(|_| self.focused && self.pane.thread_root.is_none())
        else {
            return;
        };
        if self.store.read().incognito() {
            return;
        }
        if at_bottom {
            let mut st = self.store.write();
            if let Some(idx) = st.resolve_chat_idx(&id) {
                st.mark_read(idx, false);
            }
            return;
        }
        let Some(seen) = self.pane.timeline.last_seen() else { return };
        let through = {
            let st = self.store.read();
            let chat = st.resolve_chat_idx(&id).filter(|&i| st.chats[i].is_unread());
            chat.and_then(|i| {
                let (c, merged) = (&st.chats[i], !st.members(i).is_empty());
                let read = c.last_read_message_sort_key.as_ref();
                let m = st.chat_timeline(&c.id).into_iter().find(|m| vlist::key(&m.id) == seen);
                let m = m.filter(|m| merged || read.is_none_or(|r| m.sort_key > *r));
                m.map(|m| (i, merged, m.chat_id.clone(), m.id.clone()))
            })
        };
        let mut st = self.store.write();
        match through {
            Some((i, true, _, mid)) => st.mark_union_read_through(i, &mid),
            Some((_, false, chat, mid)) => st.mark_read_through(&chat, &mid),
            None => {}
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
            let undo = (tr("Undo"), UiMessage::ChatList(chat_list::Msg::UndoFlag(flag, chats, many)));
            let (buttons, right) = if icon.is_some() { (Vec::new(), Some(undo)) } else { (vec![undo], None) };
            let open = Some(UiMessage::ChatList(chat_list::Msg::SelectTab(open)));
            self.toast = Some(Toast { icon, strong: icon.is_some(), buttons, right, open, ..Toast::new(text) });
        }
    }

    /// The ID of a label, after its label space replaced the ID it had before the homeserver made it.
    fn label_id(&self, id: String) -> String {
        self.store.read().sent.get(&id).cloned().unwrap_or(id)
    }

    /// Desktop's `runAction`: keeps the undo of an action, and shows its `successToast` with Undo.
    fn toggle_label(&mut self, label: String, chats: Vec<String>, add: bool, toast: bool) {
        self.list.chat_menu = None;
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
        let open = add.then(|| UiMessage::Rail(rail::Msg::Select(Some(format!("label:{label}")))));
        self.act(what, UiMessage::Labels(labels::Msg::Toggle(label, chats, !add)), toast.then_some((text, open)));
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
                self.show(Toast {
                    buttons: vec![(tr("Copy Error"), UiMessage::Message(message::Msg::CopyText(e)))],
                    ..Toast::new(text)
                });
            }
        }
    }

    fn show(&mut self, t: Toast) {
        self.status_banner = t.text.clone();
        self.toast = Some(t);
    }

    /// Desktop's `searchUsers` on the accounts whose bridge has the `search` capability, or only on `only`. Desktop's
    /// `canSearchUsersByName` leaves out the Matrix account unless "Include Matrix results in Start New Chat" is on.
    fn search_users(&mut self, q: &str, only: Option<String>) -> Task<UiMessage> {
        let st = self.store.read();
        let prefs = st.synced_prefs.get(beeper_core::prefs::DESKTOP_PREFS);
        let matrix = prefs.is_some_and(|p| p[beeper_core::prefs::INCLUDE_MATRIX_ACCOUNT_SEARCH] == true);
        let accounts: Vec<String> = rail_accounts(&st)
            .into_iter()
            .filter(|a| self.can(a, "create_dm") && self.can(a, "search") && only.as_ref().is_none_or(|o| o == a))
            .filter(|a| matrix || a != "hungryserv")
            .collect();
        drop(st);
        if accounts.is_empty() {
            return Task::none();
        }
        self.new_chat_search = Some((q.into(), accounts.len(), Vec::new()));
        Task::batch(accounts.into_iter().map(|a| {
            let (found, q) = (self.bridge(&a).search_users(q.into()), q.to_string());
            Task::perform(found, move |r| UiMessage::NewChat(new_chat::Msg::SearchedUsers(q.clone(), a.clone(), r)))
        }))
    }

    /// Desktop's `ix`: the users of a phone number, email or username on the accounts. An account with `any_phone`
    /// reaches a phone number at once; the others ask the bridge with `resolve_identifier`.
    /// Desktop's `sit`: on each account, the user of a contact of its contact list (`known`, as its account and user),
    /// a phone number that the network takes as is, and the lookups of the phone number, email and username.
    fn resolve_identifiers(
        &self,
        key: &str,
        accounts: &[String],
        (phone, email, username): (Option<&str>, Option<&str>, Option<&str>),
        known: Option<(&str, User)>,
    ) -> (Networks, Task<UiMessage>) {
        let (mut nets, mut tasks) = (Vec::new(), Vec::new());
        for a in accounts {
            if let Some((_, user)) = known.as_ref().filter(|(k, _)| k == a) {
                nets.push((a.clone(), Some(Some(user.clone()))));
                continue;
            }
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
                let done = move |r| UiMessage::NewChat(new_chat::Msg::ContactNetwork(key.clone(), slot, r));
                tasks.push(Task::perform(self.bridge(a).resolve_identifier(id.into()), done));
                nets.push((a.clone(), None));
            }
        }
        (nets, Task::batch(tasks))
    }

    /// The bridge that does the `PlatformAPI` work of a chat account.
    fn bridge(&self, account_id: &str) -> beeper_core::matrix::ChatBridge {
        beeper_core::matrix::ChatBridge::of(&self.store, account_id)
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
        self.store.read().provision(account_id)
    }

    fn fail(&mut self, t: Toast) {
        self.show(Toast { error: true, ..t });
    }

    /// Desktop's `undoLastToast`: the Undo of the toast on screen.
    fn shown_undo(&self) -> Option<UiMessage> {
        let t = self.shown_toast()?;
        t.buttons.iter().chain(&t.right).find(|b| b.0 == tr("Undo")).map(|b| b.1.clone())
    }

    fn set_archived(&self, st: &mut BeeperStore, idx: usize, on: bool) -> bool {
        if on && (!st.feature("archive-as-unread").on || st.settings.on("markReadOnArchive")) {
            self.mark_read(st, idx);
        }
        let moved = st.set_chat_flag(idx, ChatFlag::Archived, Some(on));
        if moved {
            self.last_archived.replace(Some(st.chats[idx].id.clone()));
        }
        moved
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

    fn expired(&self, c: &Chat) -> bool {
        c.reminder.as_ref().is_some_and(|r| self.reminded.get(&c.id) == Some(&r.remind_at_ms))
    }

    /// Desktop's `earliestMessageReminder` of the chat, while message reminders are on.
    fn message_reminder_at(&self, st: &BeeperStore, c: &Chat) -> Option<i64> {
        let first = st.message_reminders_of(&c.id).first().map(|(_, r)| r.remind_at_ms);
        first.filter(|_| reminders::message_reminders_on(st))
    }

    /// Desktop's `hasDueMessageReminder`.
    fn due_message_reminder(&self, st: &BeeperStore, c: &Chat) -> bool {
        self.message_reminder_at(st, c).is_some_and(|at| at <= now_unix_millis())
    }

    /// Whether the chat list draws the chat muted: Desktop's `areNotificationsMuted` of the chat, its labels or its
    /// account, or its snooze, unless a reminder is due.
    fn list_muted(&self, st: &BeeperStore, c: &Chat) -> bool {
        (st.notifications_muted(c) || reminders::snoozed_until(st, c).is_some())
            && !self.expired(c)
            && !self.due_message_reminder(st, c)
    }

    /// Desktop's `isMuted` row style, with a grey unread dot and count: a muted chat without unread mentions, or an
    /// archived chat.
    fn list_quiet(&self, st: &BeeperStore, c: &Chat) -> bool {
        self.list_muted(st, c) && c.unread_mentions == 0 || c.flags & CHAT_ARCHIVED != 0
    }

    /// Desktop's `.ThreadListItem-module__hoverActions` transition: the hover actions of a row fade and grow in over
    /// `--animation-timing` (0.1 s, ease-in-out) while the pointer is on it, and back out after it leaves. Gives how far
    /// in they are, or `None` when they do not show.
    pub(crate) fn hover_progress(&self, key: &str) -> Option<f32> {
        let gone = |at: std::time::Instant| (at.elapsed().as_secs_f32() / HOVER_FADE).min(1.0);
        match (&self.hovered_chat, &self.unhovered) {
            (Some(k), _) if k == key => Some(crate::login::ease_in_out(self.hover_at.map_or(1.0, gone))),
            (_, Some((k, at))) if k == key && gone(*at) < 1.0 => Some(1.0 - crate::login::ease_in_out(gone(*at))),
            _ => None,
        }
    }

    /// Whether a row's hover actions are fading in or out.
    fn hover_fading(&self) -> bool {
        let fading = |at: &std::time::Instant| at.elapsed().as_secs_f32() < HOVER_FADE;
        self.hover_at.as_ref().is_some_and(fading) || self.unhovered.as_ref().is_some_and(|(_, at)| fading(at))
    }

    fn row_action<'a>(&self, c: &Chat, pref: u8, t: f32) -> Option<El<'a>> {
        let unread = c.is_unread();
        let archived = c.flags & CHAT_ARCHIVED != 0;
        let (glyph, size, label, keys, msg) = match pref {
            _ if self.list().active_tab == InboxTab::Reminders => {
                (6, 20.0, tr("Dismiss Reminder"), vec![], UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), None)))
            }
            _ if c.draft.is_some() => {
                (5, 20.0, tr("Delete draft"), vec![], UiMessage::ChatList(chat_list::Msg::DeleteDraft(c.id.clone())))
            }
            0 if archived || c.flags & CHAT_PINNED == 0 => {
                let msg = UiMessage::ChatList(chat_list::Msg::ToggleFlag(c.id.clone(), ChatFlag::Archived));
                let label = if archived { tr("Unarchive") } else { trx("Archive", "button label to archive chat") };
                (usize::from(archived), 14.0, label, vec!["Ctrl", "E"], msg)
            }
            1 => {
                let label = tr(if unread { "Mark Read" } else { "Mark Unread" });
                (
                    2 + usize::from(!unread),
                    20.0,
                    label,
                    vec!["Ctrl", "Shift", "U"],
                    UiMessage::ChatList(chat_list::Msg::ToggleRead(c.id.clone())),
                )
            }
            2 => {
                (4, 20.0, tr("Open Context Menu"), vec![], UiMessage::ChatList(chat_list::Msg::OpenMenu(c.id.clone())))
            }
            _ => return None,
        };
        Some(self.hover_btn(glyph, size, label, keys, msg, t))
    }

    /// A hover action `t` of the way into its fade, from 30% of its size and clear to its full size and color.
    fn hover_btn<'a>(
        &self,
        glyph: usize,
        size: f32,
        label: &'a str,
        keys: Vec<&'static str>,
        msg: UiMessage,
        t: f32,
    ) -> El<'a> {
        let p = &self.pal;
        let icon = tinted(&ROW_ACTIONS[glyph], size * (0.3 + 0.7 * t), p.strong.scale_alpha(t));
        let btn = button(container(icon).center(size)).padding(6).style(btn_style([CLEAR; 3], [p.strong; 3], 0.0));
        let tip = self.tip(btn.on_press(msg).into(), label, keys, tooltip::Position::Right);
        let top = 22f32.min(51.0 - 12.0 - size);
        container(tip.delay(std::time::Duration::from_secs(1)))
            .align_right(Fill)
            .padding(Padding::ZERO.top(top).right(4.0))
            .into()
    }

    /// Desktop's Picture in Picture window (`Gyt`): frameless and on top, 16 px from the top left corner, while the
    /// video that went on plays out of its chat or out of view. It closes when the video stops or another video
    /// plays, and when the video goes back inline (`restore_pip`). Desktop's main process makes it at least 150 px
    /// on each side and without a shadow; iced has no shadow setting on Linux, where an undecorated window has none.
    fn pip(&mut self) -> Task<UiMessage> {
        if let Some((id, mid)) = &self.pip {
            if self.video.as_ref().is_some_and(|v| &v.mid == mid) {
                return Task::none();
            }
            let id = *id;
            (self.pip, self.pip_manual, self.pip_menu) = (None, false, None);
            return window::close(id);
        }
        let Some(v) = self.video.as_mut().filter(|_| std::mem::take(&mut self.pip_wanted)) else {
            return Task::none();
        };
        v.set_pip(true);
        let (w, h) = v.pip_size();
        let (id, open) = window::open(window::Settings {
            size: Size::new(w as f32, h as f32),
            position: window::Position::Specific(Point::new(16.0, 16.0)),
            decorations: false,
            resizable: false,
            transparent: true,
            level: window::Level::AlwaysOnTop,
            min_size: Some(Size::new(150.0, 150.0)),
            ..Self::window_settings(Size::new(w as f32, h as f32))
        });
        self.pip = Some((id, v.mid.clone()));
        open.map(|a0| UiMessage::Window(windows::Msg::Opened(a0)))
    }

    /// Desktop's `updateLastViewedRequestsAt` when the Requests view shows, and when its requests change while it
    /// shows: it sets `com.beeper.message_requests.last_viewed_at`, which clears the dot of new requests.
    fn view_requests(&mut self) -> Task<UiMessage> {
        if self.list().active_tab != InboxTab::Requests || self.list().selected_account_filter.is_some() {
            self.seen_requests.clear();
            return Task::none();
        }
        let ids: Vec<String> = {
            let st = self.store.read();
            st.chats.iter().filter(|c| c.flags & CHAT_REQUEST != 0).map(|c| c.id.clone()).collect()
        };
        if ids.is_empty() || ids == self.seen_requests {
            return Task::none();
        }
        self.seen_requests = ids;
        let mut st = self.store.write();
        match (st.view_requests(now_unix_millis()), st.matrix.clone()) {
            (Some(content), Some(c)) => {
                let write = beeper_core::matrix::set_global(c, beeper_core::matrix::REQUESTS_VIEWED, content);
                Task::perform(write, UiMessage::Pushed)
            }
            _ => Task::none(),
        }
    }

    /// Whether one of the main window's menus is open.
    fn menu_open(&self) -> bool {
        self.list().show_filter_menu
            || self.list().chat_menu.is_some()
            || self.pane.message_menu.is_some()
            || self.pane.thread_menu.is_some()
            || self.unread_menu.is_some()
            || self.pane.edit_menu.is_some()
            || self.pane.sched_menu.is_some()
            || self.list().reminder_menu.is_some()
            || self.audio.menu.is_some()
            || self.selection_menu.is_some()
            || self.pane.show_attach_menu
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
        // Desktop's press on a trigger dismisses its tooltip, so none shows over the menu that the press opened.
        if self.menu_open() {
            return tooltip(el, Space::new(), at);
        }
        let body = row![med(label, 13.0).color(p.strong)].push(keys).spacing(8).align_y(Center);
        tooltip(el, self.tip_card(body), at)
    }

    /// The box of Desktop's `BrandTooltip`: `8px 11px` inside, the bubble radius and `--shadow-large-15`.
    fn tip_card<'a>(&self, body: impl Into<El<'a>>) -> iced::widget::Container<'a, UiMessage> {
        container(body).padding([8, 11]).style(glow_box(self.pal.card, self.pal.shadow, 17.0, 0.0, 20.0))
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
        let space = self.list().selected_account_filter.as_deref();
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
                            .on_press(UiMessage::Accounts(accounts::Msg::DismissError(b.account_id.clone())))
                            .into(),
                    ),
                };
                let icon = svg(brand_svg(brand_key(&b.account_id))).width(22).height(22);
                let line = row![icon, text].push(end).spacing(8).align_y(Center);
                let msg = again.unwrap_or(UiMessage::Accounts(accounts::Msg::OpenSettings(b.account_id.clone())));
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
        // Desktop's `RequestNotice`: the person who added you, else the other person of a direct chat, which opens
        // their card.
        let other = chat.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| !group);
        let inviter = chat.inviter().or(other).map(|u| {
            let id = handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone());
            (u.id.clone(), id.map_or_else(|| member_name(u), |id| format!("{} ({id})", member_name(u))))
        });
        let heading = tr(match (group, inviter.is_some()) {
            (true, true) => "Accept group invitation for \"__CHAT_TITLE__\" from __INVITER__?",
            (true, false) => "Accept group invitation for \"__CHAT_TITLE__\"?",
            (false, true) => "Accept message request from __INVITER__?",
            (false, false) => "Accept message request?",
        })
        .replace("__CHAT_TITLE__", &chat.title);
        let line = |t: &str| txt(t.to_string(), 13.0).color(p.strong);
        let heading: El = match inviter.as_ref().zip(heading.split_once("__INVITER__")) {
            Some(((uid, who), (before, after))) => {
                let who = self.user_trigger(chat, uid, line(who));
                container(row![line(before), who, line(after)].align_y(Center).wrap()).center_x(Fill).into()
            }
            None => line(&heading).width(Fill).align_x(Center).into(),
        };
        let pill = |label: &str, fg: Color, msg| {
            let label = semi(label.to_string(), 12.0);
            button(label).padding([4, 8]).style(btn_style([p.card; 3], [fg; 3], 100.0)).on_press(msg)
        };
        let id = &chat.id;
        let block = (st.can_block(chat) && !st.user_blocked(chat)).then(|| {
            let label = trx("Block", "action to block the other person in a DM");
            pill(label, hex(0xdc362e), UiMessage::Blocking(blocking::Msg::Ask(id.clone(), true)))
        });
        let buttons =
            row![]
                .push(block)
                .push(decline.then(|| {
                    pill(tr("Decline"), hex(0xdc362e), UiMessage::ChatList(chat_list::Msg::Delete(id.clone(), false)))
                }))
                .push(accept.then(|| {
                    pill(tr("Accept"), p.strong, UiMessage::ChatList(chat_list::Msg::AcceptRequest(id.clone())))
                }))
                .spacing(8);
        let body = column![heading, buttons].spacing(8).align_x(Center);
        let body = container(body).padding(12).width(Fill).max_width(388).style(fill_box(p.chip, 8.0));
        Some(container(body).padding([0, 8]).center_x(Fill).into())
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
                let menu = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("contact:{} {kind}", chat.id)));
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
        let shown = |account: &str| match &self.list().selected_account_filter {
            Some(f) => rail_owner(&st, f) == account,
            None => {
                self.list().active_tab == InboxTab::AllChats
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

    /// Desktop's `getSpaceIcon`.
    fn space_icon(&self, st: &BeeperStore, id: &str) -> El<'static> {
        let p = &self.pal;
        let glyph = |c: char, size: f32| -> El<'static> { ic(c, size).color(p.strong).into() };
        match id {
            "inbox" => glyph(icon::INBOX, 16.0),
            "archive" => glyph(icon::ARCHIVE, 14.0),
            "reminders" => glyph(icon::SCHEDULE, 14.0),
            "snoozed" => tinted(&MOON_STARS, 14.0, p.strong),
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
}

pub fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let (rgba8, png) = (::image::ExtendedColorType::Rgba8, ::image::ImageFormat::Png);
    ::image::save_buffer_with_format(path, rgba, width, height, rgba8, png).expect("write png");
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
        let tag = |v: &[Attachment]| crate::chat_list::media_tag(v).map(|(i, l, d)| (i, l, d.map(|d| d.0)));
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

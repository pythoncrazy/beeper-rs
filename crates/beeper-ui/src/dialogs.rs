//! The confirmation dialogs, and the Label, Edit Labels, Edit Group, Blocked users and New Poll windows.

use crate::accounts;
use crate::blocking;
use crate::blocking::blocked_chats;
use crate::chat_list;
use crate::focus::{self, Keys, Look};
use crate::i18n::{tr, trf, trn, trx};
use crate::info;
use crate::integrations;
use crate::labels;
use crate::labels::label_fill;
use crate::members;
use crate::merged;
use crate::rail;
use crate::reminders;
use crate::settings;
use crate::style::rem;
use crate::style::{
    bar, bold, bordered_box, btn_style, done_btn, editor_style, ell, fill_box, fw, ic, icon, input_style, lh, med, mix,
    pad, scroll_style, semi, shadow_box, text_width, txt, Bare, CLEAR,
};
use crate::units::human_size;
use crate::{
    badge, chat_search_subtitle, checkbox, checkbox_plus, delete_dialog, grouped_rows, icon_btn, inbox_label,
    label_icon, label_tile, login, member_name, menu_item, network_name, pill_btn, portal, reconnect, round_avatar,
    spread, stored, tinted, toggle, voice, BeeperGuiApp, El, PendingMedia, Toast, UiMessage, ACCOUNT_SUB_W,
    ASK_ARCHIVE_ALL, ASK_ARCHIVE_READ, ASK_ARCHIVE_WEEK, ASK_BLOCK, ASK_BLOCK_OFFLINE, ASK_CLEAR_CACHE, ASK_CLEAR_CHAT,
    ASK_CLEAR_STORAGE, ASK_DELETE_CHAT, ASK_DELETE_LABEL, ASK_DELETE_MESSAGE, ASK_DISCONNECT_SERVER,
    ASK_DISMISS_REMINDERS, ASK_LEAVE_CHAT, ASK_LOGOUT, ASK_LOGOUT_DEVICE, ASK_MAKE_ADMIN, ASK_MARK_ALL_READ,
    ASK_REMOTE_ACCESS, ASK_REMOVE_ACCOUNT, ASK_REMOVE_MEMBER, ASK_REMOVE_PACK, ASK_REVOKE_CONNECTION,
    ASK_ROOM_ENCRYPTION, ASK_TRANSCRIBE, ASK_UNBLOCK, ASK_UNBLOCK_OFFLINE, ASK_UNMERGE, DELETE, DRAG_HANDLE,
    INBOX_OPTIONS, LABEL_COLORS, LABEL_ICONS, NEW_LABEL, STORAGE_DAYS,
};
use beeper_core::models::*;
use beeper_core::prefs;
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::keyboard::key::Named;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, image, mouse_area, operation, row, scrollable, stack, text_editor, text_input, Column,
    Space, Stack,
};
use iced::{border, mouse, window, Background, Center, Color, Fill, Gradient, Padding, Radians, Size, Task, Theme};
use std::collections::HashSet;

/// Desktop's `cR`: the icon of a new label.
pub(crate) const DEFAULT_LABEL_ICON: &str = "3257-5964";

/// Desktop's Label Settings window (`CO`) of the label `label`. `menu` is the open Show in Inbox menu.
pub struct LabelSettings {
    pub id: window::Id,
    pub label: String,
    pub menu: bool,
}

/// An action in a Label Settings window.
#[derive(Debug, Clone)]
pub enum LabelPane {
    /// Opens or closes the Show in Inbox menu.
    Menu(bool),
    /// Asks to delete the label, with the confirm in the window.
    Delete,
    Done,
}

/// The size of Desktop's Label Settings window.
pub(crate) const LABEL_SETTINGS_SIZE: Size = Size::new(341.0, 563.0);

/// The top of the Show in Inbox menu in the Label Settings window, under its pill.
const LABEL_INBOX_MENU_Y: f32 = 326.0;

/// The New label window, Desktop's `CreateLabel` state.
pub struct LabelDraft {
    pub id: window::Id,
    /// The edited label, or none for a new label.
    pub label: Option<String>,
    pub name: String,
    pub search: String,
    /// The chats of the label, in the order they were picked.
    pub chats: Vec<String>,
    pub color: Option<u8>,
    pub icon: Option<String>,
    /// Whether "Recent chats" shows all of its chats.
    pub more: bool,
    /// The step of the New Label wizard, or the sheet of an existing label: Pick for Chats, Details for Edit Label.
    pub step: LabelStep,
    /// Desktop's `focusedIndex` of the `IconColorGrid`: the cell that the arrows move and Enter and Space pick.
    pub cell: Option<usize>,
}

/// A step of Desktop's New Label wizard (`krt`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LabelStep {
    Intro,
    Pick,
    Details,
}

/// The size of Desktop's Chats and Edit Label sheets of Label Settings.
const LABEL_SHEET_SIZE: Size = Size::new(395.0, 504.0);

/// Desktop's arrows in the `IconColorGrid` (`KZ` and `GZ`): ← and → move along the row, and ↑ and ↓ to the cell of
/// the row above or below whose middle is nearest, in the rows that `spread` makes `width` wide.
fn grid_move(at: usize, key: Named, width: f32) -> usize {
    let per = ((width + 10.0) / 42.0) as usize;
    let sections = [(0, LABEL_COLORS.len()), (LABEL_COLORS.len(), LABEL_ICONS.len())];
    let rows: Vec<(usize, usize)> = sections
        .into_iter()
        .flat_map(|(start, n)| (0..n).step_by(per).map(move |i| (start + i, per.min(n - i))))
        .collect();
    let r = rows.iter().position(|&(s, n)| (s..s + n).contains(&at)).expect("a cell of the grid");
    let (start, n) = rows[r];
    let left = |(_, n): (usize, usize), j: usize| if n < 2 { 0.0 } else { j as f32 * (width - 32.0) / (n - 1) as f32 };
    let to = match key {
        Named::ArrowLeft => return at.saturating_sub(1).max(start),
        Named::ArrowRight => return (at + 1).min(start + n - 1),
        Named::ArrowUp => r.checked_sub(1),
        _ => Some(r + 1).filter(|&r| r < rows.len()),
    };
    let Some(to) = to else { return at };
    let x = left(rows[r], at - start);
    let far = |j: usize| (left(rows[to], j) - x).abs();
    (0..rows[to].1).min_by(|&a, &b| far(a).total_cmp(&far(b))).map_or(at, |j| rows[to].0 + j)
}

/// Desktop's `intro-avatar` faces of the New Label intro.
pub(crate) static LABEL_INTRO_FACES: std::sync::LazyLock<[image::Handle; 3]> = std::sync::LazyLock::new(|| {
    [
        include_bytes!("../assets/label_intro/1.png").as_slice(),
        include_bytes!("../assets/label_intro/2.png").as_slice(),
        include_bytes!("../assets/label_intro/3.png").as_slice(),
    ]
    .map(image::Handle::from_bytes)
});

impl LabelDraft {
    pub(crate) fn new(id: window::Id) -> Self {
        let (name, search, chats) = (String::new(), String::new(), Vec::new());
        let icon = Some(DEFAULT_LABEL_ICON.into());
        LabelDraft {
            id,
            label: None,
            name,
            search,
            chats,
            color: None,
            icon,
            more: false,
            step: LabelStep::Pick,
            cell: None,
        }
    }

    fn edit(id: window::Id, l: &Label) -> Self {
        let (name, chats, color, icon) = (l.name.clone(), l.rooms.clone(), l.color, l.icon.clone());
        LabelDraft { label: Some(l.id.clone()), name, chats, color, icon, ..Self::new(id) }
    }

    /// Whether the step can go on: the pick step needs a chat and the details step needs a name. A new label
    /// needs both.
    pub fn ready(&self) -> bool {
        let named = !self.name.trim().is_empty();
        match self.step {
            LabelStep::Intro => true,
            LabelStep::Pick => !self.chats.is_empty(),
            LabelStep::Details => named && (self.label.is_some() || !self.chats.is_empty()),
        }
    }

    /// The size of the window: Desktop's New Label wizard, or a sheet of Label Settings.
    fn size(&self) -> Size {
        if self.label.is_some() {
            LABEL_SHEET_SIZE
        } else {
            LABEL_SETTINGS_SIZE
        }
    }
}

/// A line of the New label list.
enum LabelRow<'a> {
    /// "Clear search to show all selected chats".
    Note,
    /// A section, with whether it is expanded when it has more chats.
    Header(&'static str, Option<bool>),
    Chat(&'a Chat),
}

#[derive(Debug, Clone)]
pub enum LabelEdit {
    Name(String),
    Search(String),
    Toggle(String),
    More,
    /// Picks a color, or clears the picked color.
    Color(u8),
    /// Picks an icon, or clears the picked icon.
    Icon(&'static str),
    /// An arrow in the color and icon grid, or the focus coming to the grid, which forgets its cell.
    Grid(Option<Named>),
    /// Enter or Space in the grid: picks or clears the color or icon of its cell.
    GridPick,
    /// Get Started, Next or Done: the next step, or creates or saves the label.
    Advance,
    Back,
    Cancel,
}

/// Desktop's Edit Group window, the `EditGroupModal` state.
pub struct GroupDraft {
    pub id: window::Id,
    pub chat: String,
    pub title: String,
    pub topic: text_editor::Content,
    /// The new avatar: `Some(None)` removes it, and `Some(Some(_))` is a picked image with its file URL.
    pub photo: Option<Option<(String, PendingMedia, crate::Frames)>>,
    /// The pointer is on the avatar, so it shows "Edit".
    pub hover: bool,
    /// Desktop's `mode === "personal"`: the Edit Chat for Me window, which sets the personal name and photo.
    pub personal: bool,
    /// Desktop's `isSaving`: the homeserver has not answered the save yet.
    pub saving: bool,
    /// Desktop's `isDragActive`: a file is dragged over the window.
    pub dropping: bool,
}

impl GroupDraft {
    /// The name and photo that the window starts with.
    fn initial(&self, c: &Chat) -> (String, bool) {
        match self.personal {
            true => {
                let (title, img) = c.personal_parts();
                (title.unwrap_or_default(), img.is_some())
            }
            false => (c.room_title().to_string(), c.room_img_url().is_some()),
        }
    }

    /// The new title and description, and whether the avatar changes.
    pub(crate) fn changes(&self, st: &BeeperStore) -> (Option<String>, Option<String>, bool) {
        let c = &st.chats[st.resolve_chat_idx(&self.chat).expect("an edited group")];
        let (title, img) = self.initial(c);
        let topic = (!self.personal).then(|| self.topic.text());
        let photo = self.photo.as_ref().is_some_and(|p| p.is_some() || img);
        (
            (self.title != title).then(|| self.title.clone()),
            topic.filter(|t| t != c.description.as_deref().unwrap_or_default()),
            photo,
        )
    }
}

/// The size of Desktop's Edit Group window.
pub(crate) const GROUP_SIZE: Size = Size::new(313.0, 447.0);

/// The size of Desktop's Edit Chat for Me window.
const PERSONAL_SIZE: Size = Size::new(313.0, 358.0);

const PERSONAL_INPUT: &str = "personal-title";

#[derive(Clone)]
pub enum GroupEdit {
    Title(String),
    Topic(text_editor::Action),
    Pick,
    Picked(Result<Vec<std::path::PathBuf>, String>),
    Loaded(String, Result<PendingMedia, String>),
    Remove,
    Hover(bool),
    Cancel,
    Save,
    /// The homeserver took the changes of the group, or failed with the error.
    Saved(Result<(), String>),
    /// A file dropped on the window, which becomes the avatar when the avatar can change.
    Dropped(std::path::PathBuf),
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
    pub(crate) fn new(id: window::Id, chat: String) -> Self {
        let options = vec![(1, String::new()), (2, String::new())];
        beeper_core::with_defaults!(Self {
            id,
            chat,
            options,
        }, default: question, multiple, hidden, discarding, hovered, dragging)
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

impl BeeperGuiApp {
    pub(crate) fn view_confirm(&self, dialog: u8) -> El<'_> {
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
            ASK_LOGOUT_DEVICE => {
                let id = self.delete_target.clone().expect("a device to log out");
                let name = self.devices.iter().flatten().flatten().find(|d| d.id == id).map(|d| d.name.clone());
                (
                    trf("Logout of %s?", &[&name.as_deref().unwrap_or(tr("this device"))]),
                    tr("This will disconnect all chat accounts on this device.").into(),
                    vec![(tr("Logout"), UiMessage::Settings(settings::Msg::LogoutDevice(id)))],
                )
            }
            ASK_DISMISS_REMINDERS => (
                tr("Dismiss All Reminders?").into(),
                tr("This will remove Reminders from all of your chats.").into(),
                vec![(tr("Yes"), UiMessage::Reminders(reminders::Msg::DismissAll))],
            ),
            // Desktop's `confirmDeleteThread`: the delete actions that all the chats take.
            ASK_DELETE_CHAT => {
                let st = self.store.read();
                let chats: Vec<&Chat> =
                    self.delete_chats.iter().filter_map(|c| st.resolve_chat_idx(c)).map(|i| &st.chats[i]).collect();
                let modes = chats.iter().fold(DELETE_FOR_ME | DELETE_FOR_EVERYONE, |all, c| all & c.delete_modes);
                let many = chats.len() > 1;
                let ids = self.delete_chats.clone();
                let delete = |label, everyone| (label, UiMessage::ChatList(chat_list::Msg::DeleteAll(ids.clone(), everyone)));
                let (body, actions) = match modes {
                    DELETE_FOR_ME => (
                        tr(if many { "This action will permanently delete these chats." } else { "This action will permanently delete this chat." }),
                        vec![delete(tr("Delete Chat"), false)],
                    ),
                    DELETE_FOR_EVERYONE => (
                        tr(match many {
                            true => "This action will permanently delete these chats for all participants.",
                            false => "This action will permanently delete this chat for all participants.",
                        }),
                        vec![delete(tr("Delete for Everyone"), true)],
                    ),
                    _ => (
                        tr("Choose to delete permanently for yourself only, or for all participants."),
                        vec![delete(tr("Delete for Me"), false), delete(tr("Delete for Everyone"), true)],
                    ),
                };
                let title = match &chats[..] {
                    [one] => trf("Delete %s", &[&one.combined_title()]),
                    _ => tr("Delete Chats").into(),
                };
                (title, body.into(), actions)
            }
            ASK_LEAVE_CHAT => {
                let id = self.delete_target.clone().expect("a chat to leave");
                let st = self.store.read();
                let title = &st.chats[st.resolve_chat_idx(&id).expect("the chat to leave exists")].title;
                let body = trf("Are you sure you want to leave %s?", &[title]);
                (tr("Leave chat").into(), body, vec![(tr("Leave"), UiMessage::ChatList(chat_list::Msg::Leave(id)))])
            }
            ASK_MAKE_ADMIN | ASK_REMOVE_MEMBER => {
                let target = self.delete_target.as_deref().expect("a member");
                let (cid, uid) = target.split_once(' ').map(|(c, u)| (c.to_string(), u.to_string())).expect("chat and user");
                let st = self.store.read();
                let c = st.chat(&cid);
                let name = member_name(c.participants.items.iter().find(|u| u.id == uid).expect("a member of the chat"));
                match dialog {
                    ASK_MAKE_ADMIN => (
                        tr("Make Group Admin").into(),
                        trf("Are you sure you want to make %s an admin of this group?", &[&name]),
                        vec![(trx("Confirm", "make group admin confirmation button"), UiMessage::Members(members::Msg::SetAdmin(cid, uid, true)))],
                    ),
                    _ => (
                        tr("Remove from Group").into(),
                        trf("Are you sure you want to remove %s from this group?", &[&name]),
                        vec![(trx("Remove", "remove group participant confirmation button"), UiMessage::Members(members::Msg::Remove(cid, uid)))],
                    ),
                }
            }
            ASK_DELETE_LABEL => {
                let id = self.delete_target.clone().expect("a label to delete");
                let st = self.store.read();
                let name = st.labels.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
                let body = trf(
                    "Are you sure you want to delete \"%s\"? This will remove the label from all chats.",
                    &[&name],
                );
                (tr("Delete Label").into(), body, vec![(tr("Delete"), UiMessage::Labels(labels::Msg::Delete(id)))])
            }
            ASK_DELETE_MESSAGE => {
                let st = self.store.read();
                let ids = self.delete_target.as_deref().expect("messages to delete").split(' ');
                let ms: Vec<Message> = ids.map(|id| stored(&st, id)).collect();
                let (body, buttons) = delete_dialog(&st, &ms.iter().collect::<Vec<_>>()).expect("the menu checks");
                (trn("Delete message?", "Delete messages?", ms.len(), &[]), body, buttons)
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
                let archive = vec![(trx("Archive", "archive confirmation button"), UiMessage::ChatList(chat_list::Msg::ArchiveAll(read, 0)))];
                (tr(if read { "Archive all read chats" } else { "Archive all chats" }).into(), body, archive)
            }
            ASK_ARCHIVE_WEEK => (
                tr("Archive all chats older than a week").into(),
                tr("This will also archive any other older chats from a week ago").into(),
                vec![(trx("Archive", "archive confirmation button"), UiMessage::ChatList(chat_list::Msg::ArchiveAll(false, 7)))],
            ),
            ASK_MARK_ALL_READ => {
                let n = self.visible_chats(&self.store.read()).iter().filter(|c| c.flags & CHAT_UNREAD != 0).count();
                let title = trn("Mark %d thread as read", "Mark %d threads as read", n, &[&n]);
                let body = tr("Are you sure you want to mark all chats as read?").into();
                (title, body, vec![(tr("Mark as Read"), UiMessage::ChatList(chat_list::Msg::MarkAllRead))])
            }
            ASK_CLEAR_STORAGE | ASK_CLEAR_CACHE | ASK_CLEAR_CHAT => {
                let which = dialog - ASK_CLEAR_STORAGE;
                let size = human_size(self.storage_pick(which).iter().map(|e| e.bytes).sum());
                let clear = |label| vec![(tr(label), UiMessage::Settings(settings::Msg::ClearStorage(which)))];
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
                vec![(tr("Revoke"), UiMessage::Integrations(integrations::Msg::RevokeConfirmed))],
            ),
            ASK_REMOVE_ACCOUNT => {
                let id = self.delete_target.clone().expect("an account to remove");
                (
                    "Remove Account".into(),
                    "Disconnect and remove the data from this account on Beeper".into(),
                    vec![("Remove from All Devices", UiMessage::Accounts(accounts::Msg::Remove(id)))],
                )
            }
            ASK_REMOTE_ACCESS => (
                tr("Enable Remote Access?").into(),
                tr("This can expose the Beeper Desktop API to the public internet based on your firewall settings. Only enable if you know what you are doing.").into(),
                vec![(tr("Allow Remote Access"), UiMessage::Integrations(integrations::Msg::RemoteAccess(true)))],
            ),
            ASK_BLOCK..=ASK_UNBLOCK_OFFLINE => {
                let id = self.delete_target.clone().expect("a DM to block");
                let st = self.store.read();
                let c = st.chat(&id);
                let network = network_name(&c.account_id);
                match dialog {
                    ASK_BLOCK => {
                        let body = match c.block & NO_UNBLOCK {
                            0 => trf("They'll also be blocked on %s.", &[&network]),
                            _ => trf(
                                "They'll also be blocked on %1$s. You'll need %2$s to unblock them later.",
                                &[&network, &network],
                            ),
                        };
                        let block = UiMessage::Blocking(blocking::Msg::Block(id, true, self.report_spam && c.block & REPORT_SPAM != 0));
                        let label = trx("Block", "action to block the other person in a DM");
                        (trf("Block %s?", &[&c.title]), body, vec![(label, block)])
                    }
                    ASK_UNBLOCK => (
                        trf("Unblock %s?", &[&c.title]),
                        trf("They'll also be unblocked on %s.", &[&network]),
                        vec![(
                            trx("Unblock", "action to unblock the other person in a DM"),
                            UiMessage::Blocking(blocking::Msg::Block(id, false, false)),
                        )],
                    ),
                    _ => {
                        let body = match dialog {
                            ASK_BLOCK_OFFLINE => "Reconnect %1$s to block %2$s.",
                            _ => "Reconnect %1$s to unblock %2$s.",
                        };
                        let again = reconnect(&st, &c.account_id);
                        let again = again.unwrap_or_else(|| UiMessage::Accounts(accounts::Msg::OpenSettings(c.account_id.clone())));
                        let again = vec![(tr("Reconnect"), UiMessage::Commands(vec![UiMessage::Ask(None), again]))];
                        (trf("%s Is Disconnected", &[&network]), trf(body, &[&network, &c.title]), again)
                    }
                }
            }
            ASK_TRANSCRIBE => {
                let body = trf(
                    "%s works by sharing audio clips with OpenAI via our servers. You can always change this in Settings.",
                    &[&tr(if self.audio.asked.is_some() { "Transcribing voice notes" } else { "Talk to Type" })],
                );
                let allow = vec![(tr("Allow Access"), UiMessage::Voice(voice::Msg::Allow(true)))];
                let disable = (tr("Disable"), UiMessage::Voice(voice::Msg::Allow(false)));
                return self.dialog_with(self.asking_in(), tr("Privacy notice").into(), body, None, allow, disable);
            }
            // Desktop's "Room encryption" `PrefsActionPrompt` of New Chat.
            ASK_ROOM_ENCRYPTION => {
                let (p, on) = (&self.pal, self.group_encrypted);
                let label = txt(trx("Encrypted", "new Matrix group encryption toggle label"), 13.0).color(p.strong);
                // Desktop's `<label>`: a click anywhere on the row flips the toggle.
                let option = container(row![label.width(Fill), toggle(p, on, None)].align_y(Center))
                    .padding(10)
                    .style(fill_box(p.chip, 12.0));
                let option = mouse_area(option)
                    .on_press(UiMessage::NewChat(crate::new_chat::Msg::Encrypted(!on)))
                    .interaction(mouse::Interaction::Pointer);
                let note = match on {
                    true => tr("People invited later cannot see earlier messages."),
                    false => tr("People invited later can see earlier messages."),
                };
                let extra = column![option, txt(note, 12.0).color(p.text.scale_alpha(0.72))].spacing(10);
                let start = vec![(tr("Start group chat"), UiMessage::NewChat(crate::new_chat::Msg::Start))];
                let cancel = (trx("Cancel", "button label"), UiMessage::Ask(None));
                let title = tr("Room encryption").into();
                return self.dialog_with(self.asking_in(), title, String::new(), Some(extra.into()), start, cancel);
            }
            ASK_DISCONNECT_SERVER => {
                let (id, server) = self.server_target.clone().expect("a server to disconnect");
                let body = "Turning this off will permanently remove your Discord server from Beeper, including all its channels and messages.";
                (
                    tr("Disconnect Discord Server?").into(),
                    tr(body).into(),
                    vec![(tr("Disconnect"), UiMessage::Accounts(accounts::Msg::SetServer(id, server)))],
                )
            }
            ASK_REMOVE_PACK => {
                let id = self.delete_target.clone().expect("a sticker pack to remove");
                let pack = self.installed_packs.iter().flatten().find(|p| p.id == id);
                let name = pack.map_or("", |p| p.name.as_str());
                let name = if name.is_empty() { tr("Sticker pack").to_string() } else { name.to_string() };
                (
                    trf("Remove %s?", &[&name]),
                    tr("This removes the pack from your account on every device.").into(),
                    vec![(tr("Remove"), UiMessage::Settings(settings::Msg::RemovePack(id)))],
                )
            }
            ASK_UNMERGE => {
                let id = self.delete_target.clone().expect("a merged chat to unmerge");
                let unmerge = vec![(
                    trx("Unmerge", "merged chat unmerge confirm button"),
                    UiMessage::Merged(merged::Msg::Unmerge(id)),
                )];
                let body = tr("This will split this merged chat back into individual chats.").into();
                let keep = (tr("Keep chats merged"), UiMessage::Ask(None));
                return self.dialog_with(self.asking_in(), tr("Unmerge Chat?").into(), body, None, unmerge, keep);
            }
            _ => unreachable!("dialog {dialog}"),
        };
        let spam = (dialog == ASK_BLOCK).then(|| self.report_spam_box()).flatten();
        self.dialog_with(self.asking_in(), title, body, spam, actions, (tr("Cancel"), UiMessage::Ask(None)))
    }

    /// The "Report spam" checkbox of the Block dialog, when the bridge reports spam.
    fn report_spam_box(&self) -> Option<El<'_>> {
        let st = self.store.read();
        let c = st.chat(self.delete_target.as_deref().expect("a DM to block"));
        (c.block & REPORT_SPAM != 0).then(|| {
            let label = txt(tr("Report spam"), 13.0).color(self.pal.text);
            let line = row![checkbox(&self.pal, self.report_spam), label].spacing(6).align_y(Center);
            mouse_area(line).on_press(UiMessage::Blocking(blocking::Msg::ReportSpam(!self.report_spam))).into()
        })
    }

    /// A confirmation dialog over `window`, with its actions after the cancel button.
    pub(crate) fn dialog<'a>(
        &'a self, window: Option<window::Id>, title: String, body: String, actions: Vec<(&'static str, UiMessage)>,
        cancel: (&'static str, UiMessage),
    ) -> El<'a> {
        self.dialog_with(window, title, body, None, actions, cancel)
    }

    /// A confirmation dialog with `extra` under its text, like the checkbox of Electron's `showMessageBox`. An empty
    /// text leaves only `extra`.
    fn dialog_with<'a>(
        &'a self, window: Option<window::Id>, title: String, body: String, extra: Option<El<'a>>,
        actions: Vec<(&'static str, UiMessage)>, (no, cancel): (&'static str, UiMessage),
    ) -> El<'a> {
        let p = &self.pal;
        // GTK's buttons of Electron's `showMessageBox`: Tab goes through them with Adwaita's dashed
        // focus line, and the last one is the default (`defaultId`), which has the focus when the dialog opens.
        let last = actions.len().saturating_sub(1);
        let action = |(i, (label, msg)): (usize, (&'static str, UiMessage))| {
            let b = button(med(label, 13.0).color(Color::WHITE).wrapping(iced::widget::text::Wrapping::None))
                .padding([5, 12])
                .style(btn_style([p.accent; 3], [Color::WHITE; 3], 6.0))
                .on_press(msg.clone());
            let keys = if i == last { Keys::press(msg).default_focus() } else { Keys::press(msg) };
            self.target(window, format!("ask-{i}"), b, keys, Look::Dashed(Color::WHITE))
        };
        // Like a native message box, the dialog is 340 wide and grows to keep its buttons on one line.
        let buttons: f32 = std::iter::once((no, Weight::Normal, 12.0))
            .chain(actions.iter().map(|a| (a.0, Weight::Medium, 24.0)))
            .map(|(label, weight, pad)| text_width(label, 13.0, weight) + pad + 8.0)
            .sum();
        let card = column![bold(title, 14.0).color(p.strong)]
            .push((!body.is_empty()).then(|| txt(body, 13.0).color(p.text)))
            .push(extra)
            .push(
                row![
                    Space::new().width(Fill),
                    self.target(
                        window,
                        "ask-cancel",
                        pill_btn(p, no, cancel.clone()),
                        Keys::press(cancel.clone()),
                        Look::Dashed(p.text)
                    )
                ]
                .extend(actions.into_iter().enumerate().map(action))
                .spacing(8),
            )
            .spacing(10)
            .padding(16)
            .width(buttons.max(308.0) + 40.0);
        crate::backdrop(
            container(focus::trap(
                mouse_area(container(card).style(shadow_box(p.elevated, p.menu_border, 12.0, 24.0)))
                    .on_press(UiMessage::Noop),
            ))
            .center(Fill)
            .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.45), 0.0)),
        )
        .on_press(cancel)
        .into()
    }

    /// Desktop's Edit Label (`Lst`) and Chats (`Nst`) sheets of Label Settings, by their step.
    pub(crate) fn open_label_sheet(&mut self, label: String, step: LabelStep) -> Task<UiMessage> {
        self.list.chat_menu = None;
        if let Some(d) = &self.label_window {
            return self.raise(d.id);
        }
        let (id, open) = self.open_child(LABEL_SHEET_SIZE);
        let label = self.label_id(label);
        let st = self.store.read();
        let label = st.labels.iter().find(|l| l.id == label).expect("an edited label");
        self.label_window = Some(LabelDraft { step, ..LabelDraft::edit(id, label) });
        drop(st);
        open.chain(operation::focus(if step == LabelStep::Details { "label-title" } else { "label-search" }))
    }

    pub(crate) fn label_edit(&mut self, edit: LabelEdit) -> Task<UiMessage> {
        let d = self.label_window.as_mut().expect("the label window is open");
        match edit {
            LabelEdit::Name(name) => d.name = name,
            LabelEdit::Search(search) => d.search = search,
            LabelEdit::Toggle(chat) => match d.chats.iter().position(|c| *c == chat) {
                Some(i) => drop(d.chats.remove(i)),
                None => d.chats.push(chat),
            },
            LabelEdit::More => d.more = !d.more,
            LabelEdit::Color(c) => d.color = (d.color != Some(c)).then_some(c),
            LabelEdit::Icon(i) => d.icon = (d.icon.as_deref() != Some(i)).then(|| i.into()),
            LabelEdit::Grid(None) => d.cell = None,
            LabelEdit::Grid(Some(k)) => d.cell = Some(d.cell.map_or(0, |at| grid_move(at, k, d.size().width - 48.0))),
            LabelEdit::GridPick => match d.cell {
                Some(c) if c < LABEL_COLORS.len() => return self.label_edit(LabelEdit::Color(c as u8)),
                Some(c) => return self.label_edit(LabelEdit::Icon(LABEL_ICONS[c - LABEL_COLORS.len()].0)),
                None => {}
            },
            LabelEdit::Cancel => {
                let id = d.id;
                return self.close_window(id);
            }
            LabelEdit::Back => d.step = LabelStep::Pick,
            LabelEdit::Advance if !d.ready() => {}
            LabelEdit::Advance if d.label.is_none() && d.step != LabelStep::Details => {
                let (step, focus) = match d.step {
                    LabelStep::Intro => (LabelStep::Pick, "label-search"),
                    _ => (LabelStep::Details, "label-title"),
                };
                d.step = step;
                return operation::focus(focus);
            }
            LabelEdit::Advance => {
                let (id, chats) = (d.id, std::mem::take(&mut d.chats));
                let (name, color, icon) = (d.name.trim().to_string(), d.color, d.icon.clone());
                if let Some(label) = d.label.clone() {
                    let mut st = self.store.write();
                    let old = st.labels.iter().find(|l| l.id == label).expect("an edited label").clone();
                    let same = old.name == name
                        && (old.color, &old.icon) == (color, &icon)
                        && old.rooms.len() == chats.len()
                        && chats.iter().all(|c| old.rooms.contains(c));
                    if !same {
                        st.update_label(&label, &name, color, icon, chats);
                        drop(st);
                        let (what, text) = (format!("Update label \"{}\"", old.name), format!("Updated {name}"));
                        self.act(what, UiMessage::Labels(labels::Msg::Restore(Box::new(old))), Some((text, None)));
                    }
                    return self.close_window(id);
                }
                let label = self.store.write().create_label(&name, color, icon, chats);
                let open = UiMessage::Rail(rail::Msg::Select(Some(format!("label:{label}"))));
                let _ = self.update(open.clone());
                let (what, text) = (format!("Create label \"{name}\""), format!("Created {name}"));
                self.act(what, UiMessage::Labels(labels::Msg::Delete(label.clone())), Some((text, Some(open))));
                return Task::batch([
                    self.close_window(id),
                    self.update(UiMessage::Labels(labels::Msg::OpenSettings(label))),
                ]);
            }
        }
        Task::none()
    }

    /// Desktop's `CreateLabel` list: the search results, or the selected, recent and other chats.
    fn label_rows<'s>(&self, st: &'s BeeperStore, d: &LabelDraft) -> Vec<LabelRow<'s>> {
        let picked = |c: &Chat| d.chats.contains(&c.id);
        let shown = |c: &&Chat| c.listed() && (c.flags & CHAT_LOW_PRIORITY == 0 || picked(c));
        let mut all: Vec<&Chat> = st.chats.iter().filter(shown).collect();
        all.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        let q = d.search.trim().to_lowercase();
        if !q.is_empty() {
            let hits = all.into_iter().filter(|c| c.title.to_lowercase().contains(&q)).map(LabelRow::Chat);
            return std::iter::once(LabelRow::Note).chain(hits).collect();
        }
        let recent: Vec<&Chat> = st
            .recent_chat_ids
            .iter()
            .filter_map(|id| st.resolve_chat_idx(id).map(|i| &st.chats[i]))
            .filter(shown)
            .collect();
        let mut rows = Vec::new();
        let selected: Vec<&Chat> = all.iter().copied().filter(|c| picked(c)).collect();
        if !selected.is_empty() {
            rows.push(LabelRow::Header("Selected", None));
            rows.extend(selected.into_iter().map(LabelRow::Chat));
        }
        let fresh: Vec<&Chat> = recent.iter().copied().filter(|c| !picked(c)).collect();
        if !fresh.is_empty() {
            rows.push(LabelRow::Header("Recent chats", (fresh.len() > 5).then_some(d.more)));
            rows.extend(fresh.into_iter().take(if d.more { usize::MAX } else { 5 }).map(LabelRow::Chat));
        }
        let rest: Vec<&Chat> = all.into_iter().filter(|c| !picked(c) && !recent.iter().any(|r| r.id == c.id)).collect();
        if !rest.is_empty() {
            rows.push(LabelRow::Header("Chats", None));
            rows.extend(rest.into_iter().map(LabelRow::Chat));
        }
        rows
    }

    /// Desktop's Label Settings window (`grt`): the label tile and name with an Edit link, the chats of the label,
    /// the Show in Inbox and Mute settings, Delete Label and Done.
    pub(crate) fn view_label_settings<'a>(&'a self, w: &'a LabelSettings) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let id = st.sent.get(&w.label).unwrap_or(&w.label);
        let Some(l) = st.labels.iter().find(|l| l.id == *id) else { return Space::new().into() };
        let pane = |e| UiMessage::Labels(labels::Msg::Pane(w.id, e));
        // The `:focus-visible` outlines of Label Settings: Desktop's `Button`s, and `--app-focus-ring` rings inside
        // the rows of `.LabelSettings-module__group` and `__settingRow`, and on `__menuPill`.
        let ring = |offset, radius| Look::Outline { color: p.focus_ring, width: 2.0, offset, radius };
        let target = |key: &str, b: El<'a>, keys, look| self.target(Some(w.id), key, b, keys, look);
        let link = btn_style([CLEAR; 3], [p.accent; 3], 0.0);
        let edit = trx("Edit", "edit a label name, icon and color");
        let open = UiMessage::Labels(labels::Msg::Edit(l.id.clone()));
        let edit = button(txt(edit, 13.0).color(p.accent)).padding(0).style(link).on_press(open.clone());
        let edit = target("edit", edit.into(), Keys::press(open), Look::button(p, 4.0));
        let head = column![
            container(label_tile(l, 54.0, 28.0, 20.0, p.strong)).padding(Padding::ZERO.bottom(16.0)),
            semi(l.name.clone(), 18.0).line_height(lh(24.0)).color(p.strong).align_x(Center),
            edit,
        ]
        .align_x(Center)
        .width(Fill)
        .padding(Padding::ZERO.top(8.0));
        let title = |s: &str| container(med(s.to_string(), 13.0).color(p.sub)).padding([8, 12]);
        let chats: Vec<&Chat> = l.rooms.iter().filter_map(|r| st.resolve_chat_idx(r).map(|i| &st.chats[i])).collect();
        let fit = 1 + ((LABEL_SETTINGS_SIZE.width - 16.0 - 32.0 - 20.0 - 24.0 - 36.0) / 28.0) as usize;
        let shown = match l.rooms.len() <= fit && chats.len() >= l.rooms.len() {
            true => chats.len(),
            false => chats.len().min(fit - 1),
        };
        let more = l.rooms.len() - shown;
        let faces = chats[..shown].iter().map(|c| self.chat_avatar(c, 36.0, 1.25)).chain((more > 0).then(|| {
            // Desktop translates "+%d" ("count of avatars not shown in a pile") in no language.
            let more = format!("+{more}");
            container(semi(more, 13.0).color(p.strong)).center(36).style(fill_box(p.selected, 18.0)).into()
        }));
        let faces = faces.enumerate().fold(Stack::new(), |s, (i, face)| {
            let ring = container(face).padding(2).style(fill_box(p.chip, 20.0));
            s.push(container(ring).padding(Padding::ZERO.left(i as f32 * 28.0)))
        });
        let group: El = match l.rooms.is_empty() {
            true => container(txt(tr("Add Chats"), 12.0).color(p.sub)).center_y(36).width(Fill).into(),
            false => container(faces).width(Fill).into(),
        };
        let group = row![group, ic(icon::CHEVRON_RIGHT, 16.0).color(p.sub)].spacing(8).align_y(Center);
        let chats = UiMessage::Labels(labels::Msg::EditChats(l.id.clone()));
        let group = button(group)
            .padding(Padding::new(8.0).left(12.0))
            .width(Fill)
            .style(btn_style([p.chip; 3], [p.text; 3], 16.0))
            .on_press(chats.clone());
        let group = target("chats", group.into(), Keys::press(chats), ring(-2.0, 16.0));
        let now = l.inbox.unwrap_or("all");
        let inbox_name = |o: &str, label: &'static str| match o {
            "none" => trx(label, "label inbox visibility option"),
            _ => inbox_label(label),
        };
        let current = INBOX_OPTIONS.iter().find(|o| o.0 == now).expect("an inbox option");
        let pill =
            row![txt(inbox_name(current.0, current.1), 12.0), ic(icon::EXPAND_MORE, 12.0)].spacing(2).align_y(Center);
        let pill = button(pill)
            .padding(Padding::new(2.0).right(6.0).left(10.0))
            .height(24)
            .style(btn_style([p.selected; 3], [p.sub; 3], 12.0))
            .on_press(pane(LabelPane::Menu(!w.menu)));
        let pill = target("inbox", pill.into(), Keys::press(pane(LabelPane::Menu(!w.menu))), ring(0.0, 12.0));
        let setting = |glyph: char, label: &str, end: El<'a>| -> El<'a> {
            row![container(ic(glyph, 18.0).color(p.strong)).center_x(28), txt(label.to_string(), 13.0).width(Fill), end]
                .spacing(8)
                .align_y(Center)
                .height(40)
                .padding(Padding::new(8.0).left(8.0))
                .into()
        };
        let rule = container(container(Space::new()).width(Fill).height(0.5).style(fill_box(p.prefs_border, 0.0)))
            .padding(Padding::ZERO.left(40.0).right(8.0));
        let flip = UiMessage::Labels(labels::Msg::Mute(l.id.clone(), !l.muted));
        let mute = toggle(p, l.muted, Some(flip.clone()));
        // The `<label>` row of Mute shows the ring of its switch, which Space flips.
        let mute = setting(icon::NOTIFICATIONS_OFF, trx("Mute", "label setting"), mute);
        let settings = column![
            setting(icon::INBOX, tr("Show in Inbox"), pill),
            rule,
            target("mute", mute, Keys::checkbox(flip), ring(-2.0, 16.0))
        ];
        let delete = button(container(txt(tr("Delete Label"), 13.0).color(p.danger)).center_y(24))
            .padding([8, 12])
            .width(Fill)
            .style(btn_style([p.chip; 3], [p.danger; 3], 16.0))
            .on_press(pane(LabelPane::Delete));
        let delete = target("delete", delete.into(), Keys::press(pane(LabelPane::Delete)), ring(-2.0, 16.0));
        let body = column![
            head,
            column![title(trx("Chats", "label settings section")), group].padding(Padding::ZERO.top(16.0)),
            container(container(settings).style(fill_box(p.chip, 16.0))).padding(Padding::ZERO.top(16.0)),
            container(delete).padding(Padding::ZERO.top(16.0)),
        ]
        .padding(Padding::new(16.0).bottom(80.0));
        let done = done_btn(p).on_press(pane(LabelPane::Done));
        let done = target("done", done.into(), Keys::press(pane(LabelPane::Done)), Look::button(p, 999.0));
        let footer = container(done).align_right(Fill).align_bottom(Fill).padding(Padding::new(16.0).top(12.0));
        let card = stack![scrollable(body).direction(bar()).style(scroll_style(p.scroll)).height(Fill), footer];
        let card = container(card).height(Fill).clip(true).style(fill_box(p.card, 12.0));
        let head = container(semi(tr("Label Settings"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let window = container(column![head, card]).padding(8).style(fill_box(p.window, 0.0));
        let menu = w.menu.then(|| {
            let items = INBOX_OPTIONS.iter().map(|&(o, label)| {
                let lead = container((o == now).then(|| ic(icon::CHECK, 14.0))).width(22).into();
                let msg = UiMessage::Commands(vec![
                    pane(LabelPane::Menu(false)),
                    UiMessage::Labels(labels::Msg::SetInbox(l.id.clone(), (o != "all").then_some(o))),
                ]);
                menu_item(p, Some(lead), inbox_name(o, label).into(), "", Some(msg))
            });
            let menu = crate::menu_card(p, Column::with_children(items), ACCOUNT_SUB_W);
            stack![
                crate::backdrop(Space::new().width(Fill).height(Fill)).on_press(pane(LabelPane::Menu(false))),
                container(menu).align_right(Fill).padding(Padding::ZERO.top(LABEL_INBOX_MENU_Y).right(24.0)),
            ]
        });
        drop(st);
        let confirm = self.label_asking(w.id).then(|| self.view_confirm(ASK_DELETE_LABEL));
        stack![window].push(menu).push(confirm).into()
    }

    /// Desktop's label sheets: the New Label wizard (`krt`) with its intro, chats and details steps, and the Chats
    /// (`Cst`) and Edit Label (`Tst`) sheets of Label Settings.
    pub(crate) fn view_create_label<'a>(&'a self, d: &'a LabelDraft) -> El<'a> {
        let p = &self.pal;
        let edit = |a0| UiMessage::Labels(labels::Msg::Change(a0));
        let body = match d.step {
            LabelStep::Intro => self.view_label_intro(d),
            LabelStep::Pick => self.view_label_pick(d),
            LabelStep::Details => self.view_label_details(d),
        };
        let ((back, undo), next) = match (d.label.is_some(), d.step) {
            (false, LabelStep::Intro) => ((tr("Cancel"), LabelEdit::Cancel), tr("Get Started")),
            (false, LabelStep::Pick) => ((tr("Cancel"), LabelEdit::Cancel), tr("Next")),
            (false, LabelStep::Details) => ((tr("Back"), LabelEdit::Back), tr("Next")),
            (true, _) => ((tr("Cancel"), LabelEdit::Cancel), tr("Done")),
        };
        // Desktop's `Button`s; Get Started has `autoFocus`, and a disabled Next takes no focus.
        let back = button(container(med(back, 13.0)).center_y(32))
            .padding([0, 12])
            .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 16.0))
            .on_press(edit(undo.clone()));
        let back = self.target(Some(d.id), "back", back, Keys::press(edit(undo)), Look::button(p, 16.0));
        let ready = d.ready();
        let bg = if ready { p.accent } else { p.accent.scale_alpha(0.4) };
        let next = button(container(med(next, 13.0)).center_y(32))
            .padding([0, 14])
            .style(btn_style([bg; 3], [Color::WHITE.scale_alpha(if ready { 1.0 } else { 0.6 }); 3], 16.0))
            .on_press_maybe(ready.then_some(edit(LabelEdit::Advance)));
        let next: El<'a> = match ready {
            true => {
                let keys = Keys::press(edit(LabelEdit::Advance));
                let keys = if d.step == LabelStep::Intro { keys.default_focus() } else { keys };
                self.target(Some(d.id), "next", next, keys, Look::button(p, 16.0))
            }
            false => next.into(),
        };
        let footer = row![Space::new().width(Fill), back, next].spacing(8).padding(Padding::new(16.0).top(12.0));
        let card = container(column![container(body).height(Fill), footer]).height(Fill).clip(true);
        let head = semi(self.title(d.id), 13.0).color(p.strong);
        let head = container(head).center_x(Fill).padding([8, 0]);
        let window = column![head, card.style(fill_box(p.card, 12.0))];
        container(window).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// The intro step of Desktop's New Label wizard: the picked chats or three sample faces with a label badge.
    fn view_label_intro<'a>(&'a self, d: &'a LabelDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let picked = d.chats.iter().filter_map(|c| st.resolve_chat_idx(c)).take(5);
        let mut faces: Vec<El> = picked.map(|i| self.chat_avatar(&st.chats[i], 48.0, 1.25)).collect();
        if faces.is_empty() {
            faces = LABEL_INTRO_FACES.iter().map(|h| image(h.clone()).width(48).height(48).into()).collect();
        }
        let n = faces.len();
        let faces = faces.into_iter().enumerate().fold(Stack::new(), |s, (i, face)| {
            let ring = container(face).padding(4).style(fill_box(p.card, 28.0));
            s.push(container(ring).padding(Padding::ZERO.left(i as f32 * 33.0)))
        });
        let tile = Label {
            color: Some(LABEL_COLORS.len() as u8 - 1),
            icon: Some(DEFAULT_LABEL_ICON.into()),
            ..Label::default()
        };
        let tile = container(label_tile(&tile, 26.0, 18.0, 13.0, p.strong)).padding(2).style(fill_box(p.card, 15.0));
        let width = 56.0 + (n - 1) as f32 * 33.0;
        let stack =
            stack![container(faces).padding(Padding::ZERO.top(13.0)), container(tile).align_right(width + 13.0),];
        let headline = semi(tr("Organize chats in custom spaces."), 13.0).color(p.strong).align_x(Center);
        let intro = column![stack, headline].spacing(16).align_x(Center);
        let plus = container(badge(p, prefs::Badge::Plus)).align_right(Fill).padding(Padding::new(12.0).right(14.0));
        stack![container(intro).center(Fill), plus].into()
    }

    /// Desktop's `LabelDetailsFields` (`Ide`): the label tile, the name field and the icon and color grid.
    fn view_label_details<'a>(&'a self, d: &'a LabelDraft) -> El<'a> {
        let p = &self.pal;
        let edit = |a0| UiMessage::Labels(labels::Msg::Change(a0));
        let tile = Label { color: d.color, icon: d.icon.clone(), ..Label::default() };
        let name = text_input(tr("Label name"), &d.name)
            .id("label-title")
            .on_input(move |n| edit(LabelEdit::Name(n)))
            .on_submit(edit(LabelEdit::Advance))
            .size(rem(12.0))
            .line_height(lh(16.0))
            .padding([8, 12])
            .style(input_style(p.chip, p.strong, p.sub, p.accent, 16.0));
        let body = column![
            container(label_tile(&tile, 54.0, 28.0, 20.0, p.strong)).center_x(Fill),
            container(name).padding(Padding::ZERO.top(20.0)),
            container(self.label_grid(d, d.size().width - 48.0)).padding(Padding::ZERO.top(20.0)),
        ];
        scrollable(container(body).padding(16)).direction(Direction::Vertical(Scrollbar::hidden())).into()
    }

    /// Desktop's `LabelChatPicker` (`_de`): the search row, then the selected, recent and other chats.
    fn view_label_pick<'a>(&'a self, d: &'a LabelDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = |a0| UiMessage::Labels(labels::Msg::Change(a0));
        let search = text_input(tr("Search name, @, number, or network"), &d.search)
            .id("label-search")
            .on_input(move |q| edit(LabelEdit::Search(q)))
            .size(rem(13.0))
            .line_height(lh(16.0))
            .padding([14, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let clear = (!d.search.trim().is_empty()).then(|| {
            let empty = edit(LabelEdit::Search(String::new()));
            let clear = icon_btn(p, icon::CLOSE, 16.0, 28.0, p.sub, empty.clone());
            self.target(Some(d.id), "clear", clear, Keys::press(empty), Look::Hidden)
        });
        let search = row![ic(icon::SEARCH, 16.0).color(p.sub), search]
            .push(clear)
            .spacing(10)
            .align_y(Center)
            .padding(Padding::ZERO.left(16.0).right(8.0));
        let rows = self.label_rows(&st, d).into_iter().map(|r| -> El<'a> {
            match r {
                LabelRow::Note => txt(tr("Clear search to show all selected chats"), 12.0)
                    .color(p.sub)
                    .align_x(Center)
                    .width(Fill)
                    .into(),
                LabelRow::Header(name, more) => {
                    let toggle = more.map(|open| {
                        let label = semi(if open { tr("Show less") } else { tr("Show more") }, 10.0);
                        button(label.line_height(lh(12.0)).color(p.accent))
                            .bare(p.accent)
                            .on_press(edit(LabelEdit::More))
                    });
                    row![txt(tr(name), 12.0).color(p.sub).width(Fill)]
                        .push(toggle)
                        .align_y(Center)
                        .padding(pad(8.0, 13.0, 4.0, 13.0))
                        .into()
                }
                LabelRow::Chat(c) => {
                    let sub = chat_search_subtitle(c);
                    let line = row![
                        checkbox_plus(p, d.chats.contains(&c.id)),
                        self.chat_avatar(c, 32.0, 0.0),
                        ell(&c.title, 260.0, 13.0, Weight::Semibold).color(p.strong),
                        ell(&sub, 200.0, 13.0, Weight::Normal).color(p.sub),
                    ]
                    .spacing(8)
                    .align_y(Center);
                    let line = button(line)
                        .padding([0, 5])
                        .height(44)
                        .width(Fill)
                        .style(btn_style([CLEAR, p.received, p.received], [p.text; 3], 6.0))
                        .on_press(edit(LabelEdit::Toggle(c.id.clone())));
                    container(line).padding([0, 8]).into()
                }
            }
        });
        let list = scrollable(Column::with_children(rows)).direction(Direction::Vertical(Scrollbar::hidden()));
        column![search, container(list).height(Fill)].into()
    }

    /// Desktop's `IconColorGrid`: the label colors, then the label icons, in rows `width` wide.
    fn label_grid<'a>(&self, d: &LabelDraft, width: f32) -> El<'a> {
        let p = &self.pal;
        let edit = |a0| UiMessage::Labels(labels::Msg::Change(a0));
        // Desktop's grid is one stop: its arrows move the focused cell, which has `box-shadow: 0 0 0 2px
        // var(--app-focus-ring)`, and Enter and Space pick it.
        let held = self.key_held(Some(d.id), "grid");
        let cell = d.cell.filter(|_| held);
        let focused = move |i: usize, b: El<'a>, radius: f32| -> El<'a> {
            crate::ring::ring(b, if cell == Some(i) { p.focus_ring } else { CLEAR }, 2.0, radius).into()
        };
        let shade = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.05);
        let colors = (0..LABEL_COLORS.len() as u8).map(|c| -> El<'a> {
            let on = d.color == Some(c);
            let fill = label_fill(Some(c));
            let dot = container(Space::new()).width(24).height(24).style(move |_: &Theme| container::Style {
                background: fill,
                border: border::rounded(12.0),
                ..Default::default()
            });
            let ring = if on { (CLEAR, p.accent) } else { (shade, CLEAR) };
            let dot = container(dot).center(32).style(bordered_box(ring.0, ring.1, 2.0, 16.0));
            let b = button(dot).bare(p.strong);
            focused(usize::from(c), b.on_press(edit(LabelEdit::Color(c))).into(), 16.0)
        });
        let icons = LABEL_ICONS.iter().enumerate().map(|(i, (id, h))| -> El<'a> {
            let on = d.icon.as_deref() == Some(*id);
            let ring = if on { (p.chip, p.accent) } else { (CLEAR, CLEAR) };
            let cell = container(tinted(h, 16.0, p.strong)).center(32).style(bordered_box(ring.0, ring.1, 2.0, 18.0));
            let b = button(cell).padding(0).style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 18.0));
            focused(LABEL_COLORS.len() + i, b.on_press(edit(LabelEdit::Icon(id))).into(), 18.0)
        });
        let grid = column![spread(colors.collect(), 32.0, width), spread(icons.collect(), 32.0, width)].spacing(20);
        let keys = Keys::arrows(move |k| Some(edit(LabelEdit::Grid(Some(k)))))
            .pressed(edit(LabelEdit::GridPick))
            .on_focus(edit(LabelEdit::Grid(None)));
        self.target(Some(d.id), "grid", grid, keys, Look::Hidden)
    }

    /// Desktop's `XLt` Edit Labels window: the chat, a New Label button and a switch for each label.
    pub(crate) fn view_edit_labels<'a>(&'a self, cid: &str) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let Some(c) = st.resolve_chat_idx(cid).map(|i| &st.chats[i]) else { return Space::new().into() };
        let sub = |s: String| txt(s, 11.0).color(Color { a: 0.6, ..p.strong });
        let members = (c.chat_type == "group").then(|| {
            let n = c.participants.total;
            sub(trn("Group · %d Member", "Group · %d Members", n, &[&n]))
        });
        let head = column![
            self.chat_avatar(c, 64.0, 1.25),
            ell(&c.title, 287.0, 15.0, Weight::Semibold).line_height(lh(20.0)).color(p.strong)
        ]
        .push(members)
        .push(sub(network_name(&c.account_id).to_string()))
        .spacing(4)
        .padding([8, 0])
        .align_x(Center)
        .width(Fill);
        let glyph = tinted(&NEW_LABEL, 18.0, p.strong);
        let new = button(row![glyph, txt(tr("New Label"), 13.0)].spacing(6).align_y(Center))
            .padding([6, 10])
            .style(btn_style([p.chip; 3], [p.strong; 3], 20.0))
            .on_press(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())));
        let mut labels: Vec<&Label> = st.labels.iter().collect();
        labels.sort_by_cached_key(|l| l.name.to_lowercase());
        let rows = labels.iter().map(|l| {
            let on = c.label_ids.contains(&l.id);
            let name = ell(&l.name, 183.0, 13.0, Weight::Normal).line_height(lh(16.0)).color(p.strong);
            let msg = UiMessage::Labels(labels::Msg::Switch(l.id.clone(), c.id.clone(), !on));
            row![label_icon(l, 28.0, p.strong), container(name).width(Fill), toggle(p, on, Some(msg))]
                .spacing(4)
                .padding([10, 8])
                .align_y(Center)
                .into()
        });
        let list = (!labels.is_empty()).then(|| {
            column![
                container(txt(tr("Labels"), 12.0).color(p.sub)).padding([0, 16]),
                container(grouped_rows(p, rows.collect())).padding([0, 8]),
            ]
            .spacing(8)
        });
        let body = column![head, container(new).center_x(Fill)].push(list).spacing(8);
        let card = container(scrollable(body).direction(bar()).style(scroll_style(p.scroll)))
            .height(Fill)
            .clip(true)
            .style(fill_box(p.card, 12.0));
        let title = container(semi(tr("Edit Labels"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        container(column![title, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Whether the open Delete Label confirm shows in the window `id`.
    pub(crate) fn label_asking(&self, id: window::Id) -> bool {
        self.confirm == Some(ASK_DELETE_LABEL) && self.delete_in == Some(id)
    }

    /// The window that shows the confirmation dialog, if one shows.
    fn asking_in(&self) -> Option<window::Id> {
        match self.confirm? {
            d if crate::SETTINGS_DIALOGS.contains(&d) => self.settings_window,
            _ => self.delete_in.filter(|w| self.label_asking(*w)).or(self.main_window),
        }
    }

    /// Whether a confirmation dialog shows in the window: the confirmation of the app, or Discard poll.
    pub(crate) fn asking(&self, id: window::Id) -> bool {
        self.asking_in() == Some(id) || self.poll_windows.iter().any(|d| d.id == id && d.discarding)
    }

    /// Desktop's `editGroup(thread, "personal")`.
    pub(crate) fn open_edit_for_me(&mut self, chat: String) -> Task<UiMessage> {
        self.list.chat_menu = None;
        if let Some(d) = &self.group_window {
            return self.raise(d.id);
        }
        let (id, open) = self.open_child(PERSONAL_SIZE);
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(&chat).expect("a chat to edit for me")];
        let title = c.personal_parts().0.unwrap_or_default();
        drop(st);
        let topic = text_editor::Content::new();
        let personal = true;
        self.group_window = Some(beeper_core::with_defaults!(GroupDraft {
            id,
            chat,
            title,
            topic,
            personal,
        }, default: photo, hover, saving, dropping));
        open.chain(operation::focus(PERSONAL_INPUT)).chain(operation::select_all(PERSONAL_INPUT))
    }

    /// Desktop's `setPersonalRoomName`, with its toast and Undo.
    pub(crate) fn set_personal_name(&mut self, chat: String, name: Option<String>) {
        let mut st = self.store.write();
        let c = st.chat(&chat);
        let old = c.personal_parts().0;
        let what = match &name {
            Some(_) => trf("Rename %s for you", &[&c.room_title()]),
            None => trf("Remove your name for %s", &[&c.title]),
        };
        st.set_personal(&chat, Some(name.clone()), None);
        drop(st);
        let text = match &name {
            Some(n) => trf("Renamed to %s for you", &[n]),
            None => tr("Removed your name for this chat").into(),
        };
        self.act(what, UiMessage::Info(info::Msg::SetPersonalName(chat, old)), Some((text, None)));
    }

    pub(crate) fn group_edit(&mut self, edit: GroupEdit) -> Task<UiMessage> {
        match edit {
            GroupEdit::Pick => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Image);
                return Task::perform(pick, |r| UiMessage::Group(GroupEdit::Picked(r)));
            }
            GroupEdit::Dropped(path) => {
                let d = self.group_window.as_ref().expect("the Edit Group window is open");
                let st = self.store.read();
                let edits =
                    d.personal || st.chats[st.resolve_chat_idx(&d.chat).expect("an edited group")].edits & 2 != 0;
                drop(st);
                return match edits && !d.saving {
                    true => self.group_edit(GroupEdit::Picked(Ok(vec![path]))),
                    false => Task::none(),
                };
            }
            GroupEdit::Picked(Ok(paths)) => {
                let Some(path) = paths.into_iter().next() else { return Task::none() };
                let url = format!("file://{}", path.display());
                return Task::perform(portal::load_off_thread(path), move |r| {
                    UiMessage::Group(GroupEdit::Loaded(url.clone(), r))
                });
            }
            GroupEdit::Picked(Err(e)) | GroupEdit::Loaded(_, Err(e)) => self.fail(Toast::new(e)),
            GroupEdit::Loaded(url, Ok(m)) => match (round_avatar(&m.bytes), self.group_window.as_mut()) {
                (Ok(h), Some(d)) => d.photo = Some(Some((url, m, h))),
                (Ok(_), None) => {}
                (Err(_), _) => self.fail(Toast::new(tr("Failed to upload image").into())),
            },
            GroupEdit::Saved(r) => {
                let Some(d) = self.group_window.as_mut() else { return Task::none() };
                d.saving = false;
                if let Err(e) = r {
                    eprintln!("Could not update the group: {e}");
                    self.fail(Toast::new(tr("Failed to update group").into()));
                    return Task::none();
                }
                let (title, topic, photo) = d.changes(&self.store.read());
                let photo = d.photo.take().filter(|_| photo);
                if let Some(Some((url, _, h))) = &photo {
                    self.avatars.insert(url.clone(), Some(h.clone()));
                }
                let url = photo.map(|p| p.map(|(url, ..)| url));
                let (id, chat) = (d.id, d.chat.clone());
                let avatar = url.as_ref().map(Option::as_deref);
                self.store.write().apply_group(&chat, title.as_deref(), topic.as_deref(), avatar);
                return self.close_window(id);
            }
            GroupEdit::Cancel => {
                return self.close_window(self.group_window.as_ref().expect("the Edit Group window is open").id)
            }
            GroupEdit::Save => {
                let d = self.group_window.as_mut().expect("the Edit Group window is open");
                let (title, topic, photo) = d.changes(&self.store.read());
                if title.is_none() && topic.is_none() && !photo {
                    return Task::none();
                }
                let (id, chat, personal) = (d.id, d.chat.clone(), d.personal);
                // Desktop's `updateGroup` keeps the window open with "Saving" until the homeserver takes the changes,
                // and leaves them unsaved after an error.
                if !personal && self.store.read().matrix.is_some() {
                    d.saving = true;
                    let avatar = d.photo.as_ref().filter(|_| photo);
                    let avatar = avatar.map(|p| p.as_ref().map(|(_, m, _)| (m.bytes.clone(), m.mime.clone())));
                    let save = beeper_core::matrix::update_group(self.store.clone(), chat, title, topic, avatar);
                    return Task::perform(save, |r| UiMessage::Group(GroupEdit::Saved(r)));
                }
                let photo = d.photo.take().filter(|_| photo);
                if let Some(Some((url, _, h))) = &photo {
                    self.avatars.insert(url.clone(), Some(h.clone()));
                }
                let photo = photo.map(|p| p.map(|(url, m, _)| (url, m.bytes, m.mime)));
                if !personal {
                    self.store.write().update_group(&chat, title, topic, photo);
                    return self.close_window(id);
                }
                if let Some(t) = title {
                    let name =
                        Some(t.trim().chars().take(PERSONAL_NAME_MAX).collect()).filter(|t: &String| !t.is_empty());
                    let _ = self.update(UiMessage::Info(info::Msg::SetPersonalName(chat.clone(), name)));
                }
                if let Some(photo) = photo {
                    let text = match photo {
                        Some(_) => tr("Changed your photo for this chat"),
                        None => tr("Removed your photo for this chat"),
                    };
                    self.store.write().set_personal(&chat, None, Some(photo));
                    self.show(Toast { strong: true, ..Toast::new(text.into()) });
                }
                return self.close_window(id);
            }
            edit => {
                let d = self.group_window.as_mut().expect("the Edit Group window is open");
                match edit {
                    GroupEdit::Title(t) if d.personal => d.title = t.chars().take(PERSONAL_NAME_MAX).collect(),
                    GroupEdit::Title(t) => d.title = t,
                    GroupEdit::Topic(a) => d.topic.perform(a),
                    GroupEdit::Remove => d.photo = Some(None),
                    GroupEdit::Hover(on) => d.hover = on,
                    _ => unreachable!("handled above"),
                }
            }
        }
        Task::none()
    }

    pub(crate) fn view_edit_group<'a>(&'a self, d: &'a GroupDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(&d.chat).expect("an edited group")];
        let can = |i: usize| d.personal || c.edits & (1 << i) != 0;
        let edit = UiMessage::Group;
        let face = match &d.photo {
            Some(Some((_, _, f))) => self.emoji(f, |h| image(h).width(64).height(64)),
            Some(None) => {
                let img_url = c.room_img_url().filter(|_| d.personal).map(str::to_string);
                self.chat_avatar(&Chat { img_url, personal: None, ..c.clone() }, 64.0, 0.0)
            }
            None => self.chat_avatar(c, 64.0, 0.0),
        };
        let shown = d.photo.as_ref().map_or(d.initial(c).1, Option::is_some);
        let face: El<'a> = match can(1) {
            true => {
                let veil = d.hover.then(|| {
                    let label = txt(trx("Edit", "group avatar edit action"), 13.0);
                    container(label.color(Color::WHITE)).center(64).style(fill_box(p.strong.scale_alpha(0.7), 32.0))
                });
                mouse_area(stack![face].push(veil))
                    .on_enter(edit(GroupEdit::Hover(true)))
                    .on_exit(edit(GroupEdit::Hover(false)))
                    .on_press(edit(GroupEdit::Pick))
                    .interaction(mouse::Interaction::Pointer)
                    .into()
            }
            false => face,
        };
        let remove = (can(1) && shown).then(|| {
            let label = txt(tr("Remove Photo"), 11.0).color(p.accent);
            container(button(label).bare(p.accent).on_press(edit(GroupEdit::Remove))).center_x(Fill)
        });
        let hint = match d.personal {
            true => Some(c.room_title()).filter(|t| !t.is_empty()).unwrap_or(tr("Chat Name")),
            false => tr("Group Title"),
        };
        let title = text_input(hint, &d.title)
            .id(PERSONAL_INPUT)
            .on_input_maybe(can(0).then_some(move |t| edit(GroupEdit::Title(t))))
            .font(fw(Weight::Semibold))
            .size(rem(15.0))
            .line_height(lh(20.0))
            .padding([8, 11])
            .style(input_style(p.chip, p.strong, p.sub, p.accent, 10.0));
        let topic = text_editor(&d.topic)
            .placeholder(tr("Group Description"))
            .size(rem(13.0))
            .line_height(lh(16.0))
            .padding([8, 11])
            .height(120)
            .style(editor_style(p, 10.0));
        let topic = if can(2) { topic.on_action(move |a| edit(GroupEdit::Topic(a))) } else { topic };
        let topic = (!d.personal).then_some(topic);
        let note = match d.personal {
            true => tr("This name and photo are only visible to you, on all your devices."),
            false => tr("The group description is visible to members of this group and people invited to this group."),
        };
        let note = txt(note, 10.0).color(p.sub);
        let (t, s, ph) = d.changes(&st);
        let ready = t.is_some() || s.is_some() || ph;
        let brand = |label: &'static str, primary: bool, msg: Option<UiMessage>| -> El<'a> {
            let fg = match (msg.is_some(), primary) {
                (false, _) => p.on_secondary,
                (true, true) => Color::WHITE,
                (true, false) => p.strong,
            };
            let gradient = primary && msg.is_some();
            button(container(med(label, 14.0).line_height(lh(20.0)).color(fg)).center_x(Fill))
                .width(Fill)
                .padding([10, 12])
                .style(move |_: &Theme, _| button::Style {
                    background: Some(if gradient { login::brand_gradient(p) } else { p.chip.into() }),
                    border: border::rounded(9999),
                    ..Default::default()
                })
                .on_press_maybe(msg)
                .into()
        };
        // Desktop's `SavingButton`: "Saving" and a spinner, with Cancel disabled too.
        let save: El<'a> = match d.saving {
            true => {
                let label = row![med(trx("Saving", "edit group modal button"), 14.0).color(p.on_secondary)]
                    .push(crate::verify::spinner(14.0, p.on_secondary))
                    .spacing(4)
                    .align_y(Center);
                container(label).center_x(Fill).padding([10, 12]).style(fill_box(p.chip, 9999.0)).into()
            }
            false => brand(trx("Save", "edit group modal button"), true, ready.then_some(edit(GroupEdit::Save))),
        };
        let cancel = (!d.saving).then_some(edit(GroupEdit::Cancel));
        let buttons = row![brand(trx("Cancel", "edit group modal button"), false, cancel), save].spacing(8);
        let body = column![container(face).center_x(Fill)].push(remove).push(title).push(topic).push(note).spacing(8);
        let body = column![body, Space::new().height(Fill), buttons];
        let body = container(body).padding(16).width(Fill).height(Fill).style(fill_box(p.card, 0.0));
        // Desktop's `dropzone` while a file is dragged over a window whose avatar can change.
        let zone = (d.dropping && can(1)).then(|| {
            let text = med(tr("Drop image here to upload"), 14.0).color(p.strong);
            container(text).center(Fill).style(bordered_box(p.card.scale_alpha(0.9), p.accent, 2.0, 12.0))
        });
        stack![body].push(zone).into()
    }

    /// Desktop's `kCt`: the Blocked users window, once.
    pub(crate) fn open_blocked_users(&mut self) -> Task<UiMessage> {
        if let Some(id) = self.blocked_window {
            return self.raise(id);
        }
        let (id, open) = self.open_child(Size::new(342.0, 488.0));
        self.blocked_window = Some(id);
        open
    }

    /// A blocked person of the Blocked users window: their chat opens with its Chat Info in the main window.
    pub(crate) fn open_blocked_user(&mut self, id: String) -> Task<UiMessage> {
        self.show_right_panel = true;
        let main = self.main_window.expect("the main window is open");
        self.update(UiMessage::SelectChat(id)).chain(self.raise(main))
    }

    /// Desktop's `ICt`: the blocked people by name, or "No Blocked Users", and Done.
    pub(crate) fn view_blocked_users(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let chats = blocked_chats(&st);
        let title = semi(tr("Blocked users"), 13.0).color(p.strong);
        let list: El<'_> = match chats.is_empty() {
            true => column![
                semi(tr("No Blocked Users"), 18.0).line_height(lh(26.0)).color(p.strong),
                txt(tr("When you block someone from a chat, they’ll show up here."), 15.0)
                    .line_height(lh(20.0))
                    .color(p.sub)
                    .align_x(Center),
            ]
            .spacing(6)
            .align_x(Center)
            .padding([0, 12])
            .into(),
            false => {
                let rows = chats.iter().enumerate().map(|(i, c)| -> El<'_> {
                    let line = row![
                        self.chat_avatar(c, 24.0, 0.0),
                        txt(c.title.clone(), 13.0).color(p.strong).width(Fill),
                        ic(icon::CHEVRON_RIGHT, 16.0).color(p.muted),
                    ]
                    .spacing(8)
                    .align_y(Center);
                    let open = UiMessage::Blocking(blocking::Msg::OpenUser(c.id.clone()));
                    let row = button(container(line).center_y(24))
                        .bare(p.strong)
                        .padding(8)
                        .width(Fill)
                        .on_press(open.clone());
                    // Desktop's rows are plain `<button>`s, whose focus does not show.
                    let row = self.target(
                        self.blocked_window,
                        format!("blocked-{}", c.id),
                        row,
                        Keys::press(open),
                        Look::Hidden,
                    );
                    let rule = container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0));
                    column![].push((i > 0).then_some(rule)).push(row).into()
                });
                let list = container(Column::with_children(rows)).clip(true).style(fill_box(p.chip, 12.0));
                scrollable(list).direction(bar()).style(scroll_style(p.scroll)).into()
            }
        };
        let body = column![title, list].spacing(10).padding(Padding::new(12.0).bottom(0.0)).height(Fill);
        let close = UiMessage::Blocking(blocking::Msg::CloseList);
        let done = done_btn(p).on_press(close.clone());
        let done = self.target(self.blocked_window, "done", done, Keys::press(close), Look::button(p, 999.0));
        let footer = container(done).align_right(Fill).padding(Padding::new(12.0).top(10.0));
        container(column![body, footer]).width(Fill).height(Fill).style(fill_box(p.elevated, 0.0)).into()
    }

    /// An edit in the New Poll window `id`.
    pub(crate) fn poll_edit(&mut self, id: window::Id, edit: PollEdit) -> Task<UiMessage> {
        let i = self.poll_windows.iter().position(|d| d.id == id).expect("the poll window is open");
        let st = self.store.read();
        let rules = st.resolve_chat_idx(&self.poll_windows[i].chat).and_then(|c| st.chats[c].polls);
        drop(st);
        let Some(rules) = rules else { return self.close_window(id) };
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
                return self.close_window(id);
            }
            PollEdit::Send => {}
            PollEdit::Close if d.discarding => d.discarding = false,
            PollEdit::Close if d.dirty() => d.discarding = true,
            PollEdit::Close | PollEdit::Discard(true) => return self.close_window(id),
            PollEdit::Discard(false) => d.discarding = false,
        }
        Task::none()
    }

    /// Desktop's `PollVotes`: the voters of each option, at most 100 each.
    pub(crate) fn view_poll_votes(&self, chat: &str, mid: &str) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let poll = st.loaded(mid).and_then(|m| m.poll.as_ref());
        let Some(poll) = poll else { return container(Space::new()).style(fill_box(p.window, 0.0)).into() };
        let people = st.resolve_chat_idx(chat).map_or(&[][..], |i| &st.chats[i].participants.items[..]);
        let weak = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.05);
        let total = poll.voters();
        let question = if poll.question.is_empty() { tr("Poll").into() } else { poll.question.clone() };
        let mut body = column![semi(question, 17.0).line_height(lh(24.0)).color(p.strong)].spacing(10).padding(16);
        for o in &poll.options {
            let n = o.voter_ids.len();
            let head = row![
                semi(o.label.clone(), 15.0).line_height(lh(20.0)).color(p.strong).width(Fill),
                txt(n.to_string(), 17.0).line_height(lh(24.0)).color(p.sub)
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
                    let line = row![face, txt(name, 14.0).line_height(lh(18.0)).color(p.strong)].spacing(8);
                    let last = i + 1 == n.min(100) && n <= 100;
                    voters = voters.push(container(line.align_y(Center)).padding(Padding::new(6.0).bottom(if last {
                        0.0
                    } else {
                        6.0
                    })));
                }
                if n > 100 {
                    let more = trf("+%d more", &[&(n - 100)]);
                    voters = voters.push(container(txt(more, 14.0).line_height(lh(18.0)).color(p.sub)).padding([6, 0]));
                }
                let rule = container(Space::new()).width(Fill).height(1).style(fill_box(weak, 0.0));
                section = section.push(Space::new().height(6)).push(rule).push(container(voters).padding([0, 8]));
            }
            body = body.push(container(section).width(Fill).padding(8).style(fill_box(p.received, 12.0)));
        }
        let card = scrollable(body).direction(bar()).height(Fill).style(scroll_style(p.scroll));
        let card = container(card).style(fill_box(p.card, 12.0));
        let title = trn("%d vote", "%d votes", total, &[&total]);
        let title = container(semi(title, 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        container(column![title, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's `PollCreator`.
    pub(crate) fn view_poll_creator<'a>(&'a self, d: &'a PollDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let rules = st.resolve_chat_idx(&d.chat).and_then(|i| st.chats[i].polls).unwrap_or(BEEPER_POLLS);
        drop(st);
        let id = d.id;
        let edit = move |e| UiMessage::Poll(id, e);
        // The `:focus-visible` rings of the Poll Creator: 2 px of `--color-primary` at an `outline-offset`.
        let ring = |offset, radius| Look::Outline { color: p.accent, width: 2.0, offset, radius };
        let weak = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.05);
        let rule = || container(Space::new()).width(Fill).height(1).style(fill_box(weak, 0.0));
        let card = |c: Column<'a, UiMessage>| container(c).width(Fill).clip(true).style(fill_box(p.received, 12.0));
        let section = |label: &'static str, items: Vec<El<'a>>| {
            column![semi(tr(label), 12.0).color(p.sub)].extend(items).spacing(8)
        };
        let question = text_input(tr("Ask a question…"), &d.question)
            .id(format!("poll-{}", d.chat))
            .on_input(move |q| edit(PollEdit::Question(q.chars().take(500).collect())))
            .size(rem(15.0))
            .line_height(lh(20.0))
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
                .size(rem(14.0))
                .line_height(lh(18.0))
                .padding([8, 0])
                .style(input_style(CLEAR, p.strong.scale_alpha(fade), p.sub, p.accent, 0.0));
            let error = repeats[i].then(|| txt(tr("This option already exists"), 11.0).color(p.danger));
            // `.PollCreator-module__removeOption:focus-visible` shows the button with a 2 px ring 2 px outside it.
            let remove = (d.options.len() > 2).then(|| {
                let key = format!("remove-{i}");
                let shown = d.hovered == Some(i) || self.key_held(Some(id), &key);
                let red = p.danger.scale_alpha(f32::from(u8::from(shown)));
                let hover = Color::from_rgba8(0xff, 0x14, 0x16, 0x20 as f32 / 255.0);
                let b = button(container(tinted(&DELETE, 16.0, red)).center(28))
                    .padding(0)
                    .style(btn_style([CLEAR, hover, hover], [red; 3], 14.0))
                    .on_press(edit(PollEdit::Remove(i)));
                self.target(Some(id), key, b, Keys::press(edit(PollEdit::Remove(i))), ring(2.0, 14.0))
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
            let add = row![txt("+", 18.0).line_height(1.0), txt(tr("Add an option"), 14.0).line_height(lh(18.0))];
            let add = button(add.spacing(6).align_y(Center))
                .bare(p.accent)
                .padding(10)
                .width(Fill)
                .on_press(edit(PollEdit::Add));
            let add = self.target(Some(id), "add", add, Keys::press(edit(PollEdit::Add)), ring(-2.0, 0.0));
            options = options.push(rule()).push(add);
        }
        let hint = (rules.max_options > 0).then(|| {
            let left = usize::from(rules.max_options).saturating_sub(d.answers().len());
            let hint = match left {
                0 => tr("You have added the maximum number of options").into(),
                _ => trn("You can enter %d more option", "You can enter %d more options", left, &[&left]),
            };
            txt(hint, 11.0).color(p.sub)
        });
        // Desktop's `<label>` rows around a `Toggle`, which Space flips, with `.PollCreator-module__settingRow:has(
        // :focus-visible)`: a 2 px `--app-focus-ring` outline 2 px inside the row.
        let setting = |label: &'static str, on: bool, msg: fn(bool) -> PollEdit| {
            let line = row![txt(tr(label), 14.0).line_height(lh(18.0)).color(p.strong).width(Fill)]
                .push(toggle(p, on, Some(edit(msg(!on)))))
                .spacing(10)
                .align_y(Center)
                .padding([10, 12]);
            let look = Look::Outline { color: p.focus_ring, width: 2.0, offset: -2.0, radius: 0.0 };
            let line = mouse_area(line).on_press(edit(msg(!on)));
            self.target(Some(id), label, line, Keys::checkbox(edit(msg(!on))), look)
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
        .padding(pad(10.0, 14.0, 96.0, 14.0));
        let ready = d.ready(rules);
        let bg = if ready { p.accent } else { mix(p.accent, Color::WHITE, 0.55) };
        let send = button(container(semi(tr("Send"), 14.0).line_height(lh(18.0))).center_x(Fill))
            .padding(11)
            .width(Fill)
            .style(btn_style([bg; 3], [Color::WHITE; 3], 999.0))
            .on_press_maybe(ready.then(|| edit(PollEdit::Send)));
        let send: El<'a> = match ready {
            true => self.target(Some(id), "send", send, Keys::press(edit(PollEdit::Send)), ring(2.0, 999.0)),
            false => send.into(),
        };
        let fade = Linear::new(Radians(0.0))
            .add_stop(0.0, p.card)
            .add_stop(0.55, p.card)
            .add_stop(1.0, p.card.scale_alpha(0.0));
        let footer = container(send).padding(pad(16.0, 14.0, 14.0, 14.0)).style(move |_: &Theme| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fade))),
            ..Default::default()
        });
        let body = scrollable(body).direction(bar()).height(Fill).style(scroll_style(p.scroll));
        let card = container(stack![body, container(footer).align_bottom(Fill)]).style(fill_box(p.card, 12.0));
        let title = container(semi(tr("New Poll"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let window = container(column![title, card]).padding(8).style(fill_box(p.window, 0.0));
        match d.discarding {
            true => stack![
                window,
                self.dialog(
                    Some(id),
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
}

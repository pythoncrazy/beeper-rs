//! Desktop's chat list (ThreadList): its tabs and filters, the chat menu and flags, archive and read, multi-select, and the chat actions.

use super::*;
use crate::i18n::trnx;
use crate::prelude::*;
use crate::reminders::{last_used, last_used_label, remind_presets};
use crate::vcard::contact_name;
use iced::widget::column;
use std::borrow::Cow;

/// The chat list and the context menus of a window: the main window's, or a space window's. Desktop keeps them in the
/// `WindowStateStore` of each window.
#[derive(Default)]
pub struct ListState {
    pub active_tab: InboxTab,
    pub selected_account_filter: Option<String>,
    /// Desktop's `spaceViewAccountFilter`: the account or network space that the Archive, Low Priority or Requests
    /// view shows the chats of, from its filter menu.
    pub space_view_account: Option<String>,
    pub filters: u8,
    pub show_filter_menu: bool,
    /// The chat whose context menu is open, and where it opens in the window.
    pub chat_menu: Option<(String, Point)>,
    pub(crate) reminder_menu: Option<Point>,
    pub(crate) rows: vlist::VList,
    /// Desktop's `disableSpaceSwitching`: a space window shows its space without the space bar, and its header does
    /// not switch spaces.
    pub(crate) locked: bool,
    /// Desktop's `threadCursorID`: the chat that ↑, ↓, J and K point at while no chat is open, which shows as
    /// selected and which Enter opens.
    pub(crate) cursor: Option<String>,
    /// The id of the list's scrollable, its own in each window.
    pub(crate) scroll: ListScroll,
}

pub(crate) struct ListScroll(pub(crate) iced::widget::Id);

impl Default for ListScroll {
    fn default() -> Self {
        Self(iced::widget::Id::unique())
    }
}

impl ListState {
    /// The list of a space window of rail space `entry`, Desktop's `setSpaceFilters([space])`.
    pub(crate) fn space(entry: &str) -> Self {
        let tab = RAIL_VIEWS.iter().find(|v| v.0 == entry).and_then(|v| v.1);
        let filter = tab.is_none().then(|| entry.to_string());
        Self { active_tab: tab.unwrap_or_default(), selected_account_filter: filter, locked: true, ..Self::default() }
    }

    /// The rail space that the list shows: the space of its account filter, or the space of its tab.
    pub(crate) fn entry(&self) -> String {
        self.selected_account_filter
            .clone()
            .unwrap_or_else(|| RAIL_VIEWS.iter().find(|v| v.1 == Some(self.active_tab)).expect("a sidebar view").0.to_string())
    }

    /// Desktop's `sc`: the name of the space that the list shows, its header and the title of a space window.
    pub(crate) fn title(&self, st: &BeeperStore) -> String {
        self.selected_account_filter.as_deref().map_or(self.active_tab.label().to_string(), |id| space_name(st, id))
    }
}

#[derive(Clone)]
pub enum Msg {
    SelectTab(InboxTab),
    /// The Undo of an archive or Low Priority toast: sets the flag of the chats back, then selects them.
    UndoFlag(ChatFlag, Vec<(String, bool)>, bool),
    Click(String),
    ToggleFlag(String, ChatFlag),
    ToggleRead(String),
    MarkRead(String),
    CopyLink(String),
    OpenMenu(String),
    AcceptRequest(String),
    /// Desktop's Select Chats: toggles chat selection mode.
    ToggleSelectMode,
    /// Closes the chat menu over the command bar, which stays open.
    CloseMenu,
    ArchiveStale,
    /// Archives the inbox chats, or only the read ones, with no activity in the days, after Desktop's confirmation.
    ArchiveAll(bool, i64),
    MarkAllRead,
    ToggleFilterMenu,
    FilterButton,
    PickFilter(u8),
    /// Desktop's `setSpaceViewAccountFilter`: shows only the chats of an account or network space in a space view,
    /// or every chat again when it is the one shown.
    PickAccount(String),
    /// Desktop's `clearThreadsFilters`: the filters and the space view account.
    ClearFilters,
    /// Desktop's `replyPrivately`: the direct chat with the sender of a message, with the message quoted.
    ReplyPrivately(String),
    /// Desktop's `exportLoadedMessagesAsTextFile`: the loaded messages of the open chat in a text file on the desktop.
    Export,
    AskDelete(String),
    AskLeave(String),
    Leave(String),
    BeginSelecting(String),
    ToggleSelected(String),
    SelectionSet(ChatFlag, bool),
    SelectionRemind(Option<i64>),
    SelectionMenu(u8, f32),
    Delete(String, bool),
    /// Deletes the chats of a confirmation, for everyone (true) or for you.
    DeleteAll(Vec<String>, bool),
    Hover(String, bool),
    DeleteDraft(String),
}

impl BeeperGuiApp {
    pub(crate) fn chat_list(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::SelectTab(tab) => {
                self.list.chat_menu = None;
                self.list.active_tab = tab;
                self.list.selected_account_filter = None;
                self.list.space_view_account = None;
                self.list.show_filter_menu = false;
                self.leave_draft();
            }
            Msg::Click(id) | Msg::ToggleSelected(id) if self.mods.shift() => self.select_range(&id),
            Msg::Click(id) if self.mods.control() => return self.update(UiMessage::ChatList(Msg::ToggleSelected(id))),
            Msg::Click(id) => {
                let last = self.last_click.take();
                if last.is_none_or(|(c, at)| c != id || at.elapsed() > std::time::Duration::from_millis(500)) {
                    self.last_click = Some((id.clone(), std::time::Instant::now()));
                    return self.update(UiMessage::SelectChat(id));
                }
                let msg = match self.store.read().settings.pref("chatDoubleClick") {
                    1 => UiMessage::ChatList(Msg::ToggleFlag(id, ChatFlag::Archived)),
                    2 => UiMessage::ChatList(Msg::ToggleRead(id)),
                    3 => UiMessage::ChatList(Msg::ToggleFlag(id, ChatFlag::Muted)),
                    4 => UiMessage::ChatList(Msg::ToggleFlag(id, ChatFlag::LowPriority)),
                    _ => UiMessage::Window(windows::Msg::OpenChat(id)),
                };
                return self.update(msg);
            }
            m @ (Msg::ArchiveStale | Msg::ArchiveAll(..)) => {
                self.confirm = None;
                {
                    let mut st = self.store.write();
                    let chats = match m {
                        Msg::ArchiveAll(read, days) => st.inbox_chats_before(read, now_unix_millis() - days * 86_400_000),
                        _ => st.stale_inbox_chats(),
                    };
                    for idx in chats {
                        assert!(st.set_chat_flag(idx, ChatFlag::Archived, Some(true)));
                    }
                }
                self.refresh_theme_and_tray();
            }
            Msg::UndoFlag(flag, chats, many) => {
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
            Msg::ToggleFlag(cid, flag) => {
                // Desktop's `toggleThreadArchive` first moves the cursor from the chat to the next one, also when the
                // chat is pinned and stays.
                if flag == ChatFlag::Archived && self.list.cursor.as_ref() == Some(&cid) {
                    self.point_chat(true);
                }
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
                    self.act(what, UiMessage::ChatList(Msg::UndoFlag(flag, vec![(cid.clone(), was)], false)), None);
                }
                self.list.chat_menu = None;
                self.refresh_theme_and_tray();
                let selected = self.pane.selected_chat_id.as_deref() == Some(cid.as_str());
                if let Some(at) = at.filter(|_| flag == ChatFlag::Archived && moved && selected) {
                    let list = self.list_ids(&self.store.read());
                    return self.after_action("afterArchive", &list, at, true);
                }
            }
            Msg::MarkAllRead => {
                self.confirm = None;
                let mut st = self.store.write();
                let unread: Vec<String> =
                    self.visible_chats(&st).iter().filter(|c| c.flags & CHAT_UNREAD != 0).map(|c| c.id.clone()).collect();
                for id in unread {
                    let idx = st.resolve_chat_idx(&id).expect("a listed chat");
                    self.mark_read(&mut st, idx);
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            Msg::MarkRead(cid) => {
                let mut st = self.store.write();
                let idx = st.resolve_chat_idx(&cid).expect("chat exists");
                self.mark_read(&mut st, idx);
                drop(st);
                self.refresh_theme_and_tray();
            }
            Msg::ToggleRead(cid) => {
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
                self.list.chat_menu = None;
                self.refresh_theme_and_tray();
                let at = list.iter().position(|c| *c == cid);
                if let Some(at) = at.filter(|_| self.pane.selected_chat_id.as_deref() == Some(cid.as_str())) {
                    return self.after_action("afterMarkRead", &list, at, false);
                }
            }
            Msg::CopyLink(cid) => {
                self.list.chat_menu = None;
                let st = self.store.read();
                let c = &st.chats[st.resolve_chat_idx(&cid).expect("chat exists")];
                return iced::clipboard::write(deeplink::chat_link(&c.account_id, &c.id));
            }
            Msg::CloseMenu => self.list.chat_menu = None,
            Msg::OpenMenu(cid) => {
                self.list.chat_menu = Some((cid, self.cursor.get()));
                self.chat_sub = None;
            }
            Msg::Export => {
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
            Msg::ReplyPrivately(mid) => {
                let st = self.store.read();
                let m = &stored(&st, &mid);
                let c = st.chat(&m.chat_id);
                let sender = c.participants.items.iter().find(|u| u.id == m.sender_id).cloned().expect("the sender");
                let quote = copy_text(m, false, false).lines().map(|l| format!("> {l}\n")).collect();
                let account = c.account_id.clone();
                drop(st);
                return self.open_dm(account, sender, None, Some(quote));
            }
            // Desktop's `confirmDeleteThread`: the selected chats, which must share a delete action.
            Msg::AskDelete(id) => {
                self.list.chat_menu = None;
                let chats = if self.selection.is_empty() { vec![id.clone()] } else { self.selection.clone() };
                let st = self.store.read();
                let modes = chats.iter().filter_map(|c| st.resolve_chat_idx(c)).map(|i| st.chats[i].delete_modes);
                let common = modes.fold(DELETE_FOR_ME | DELETE_FOR_EVERYONE, |all, m| all & m);
                drop(st);
                if common == 0 {
                    self.status_banner = match chats.len() > 1 {
                        true => tr("Selected chats do not support the same delete action"),
                        false => tr("Selected chat does not support this delete action"),
                    }
                    .into();
                    return Task::none();
                }
                (self.confirm, self.delete_target, self.delete_chats) = (Some(ASK_DELETE_CHAT), Some(id), chats);
            }
            msg @ (Msg::BeginSelecting(..)
            | Msg::ToggleSelectMode
            | Msg::ToggleSelected(..)
            | Msg::SelectionSet(..)
            | Msg::SelectionRemind(..)
            | Msg::SelectionMenu(..)) => return self.chat_list_selection(msg),
            Msg::AskLeave(id) => (self.confirm, self.delete_target) = (Some(ASK_LEAVE_CHAT), Some(id)),
            Msg::Leave(id) => {
                self.confirm = None;
                self.store.write().leave_room(&id);
                return self.update(UiMessage::StoreChanged);
            }
            Msg::AcceptRequest(id) => {
                self.list.chat_menu = None;
                self.store.write().accept_request(&id);
                return self.update(UiMessage::StoreChanged);
            }
            Msg::Delete(id, everyone) => {
                self.list.chat_menu = None;
                self.confirm = None;
                self.store.write().delete_chat(&id, everyone);
                return self.update(UiMessage::StoreChanged);
            }
            Msg::DeleteAll(ids, everyone) => {
                self.confirm = None;
                self.selection.retain(|c| !ids.contains(c));
                let mut st = self.store.write();
                for id in &ids {
                    if st.resolve_chat_idx(id).is_some() {
                        st.delete_chat(id, everyone);
                    }
                }
                drop(st);
                return self.update(UiMessage::StoreChanged);
            }
            msg @ (Msg::ToggleFilterMenu | Msg::FilterButton | Msg::PickFilter(..) | Msg::PickAccount(..) | Msg::ClearFilters) => {
                return self.chat_list_filters(msg)
            }
            Msg::Hover(id, true) => {
                if self.hovered_chat.as_ref() != Some(&id) {
                    let now = std::time::Instant::now();
                    if let Some(left) = self.hovered_chat.replace(id) {
                        self.unhovered = Some((left, now));
                    }
                    self.hover_at = Some(now);
                }
            }
            Msg::Hover(id, false) => {
                if self.hovered_chat.as_ref() == Some(&id) {
                    self.hovered_chat = None;
                    self.unhovered = Some((id, std::time::Instant::now()));
                }
            }
            Msg::DeleteDraft(cid) => {
                self.store.write().patch_chat(&cid, true, None, None).expect("chat exists");
                if self.pane.selected_chat_id.as_ref() == Some(&cid) {
                    self.pane.composer = composer_with("");
                    self.pane.reply_to_message_id = None;
                }
            }
        }
        self.settle()
    }

    /// The filters of the chat list: the filter menu, its accounts and clearing them.
    fn chat_list_filters(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::ToggleFilterMenu => self.list.show_filter_menu = !self.list().show_filter_menu,
            Msg::FilterButton if self.list().filters != 0 || self.list().space_view_account.is_some() => {
                return self.update(UiMessage::ChatList(Msg::ClearFilters));
            }
            Msg::FilterButton => self.set_filters(1),
            Msg::PickFilter(bit) => {
                self.set_filters(if self.list().filters == bit { 0 } else { bit });
                self.list.show_filter_menu = false;
            }
            Msg::PickAccount(id) => {
                self.list.space_view_account = (self.list().space_view_account.as_ref() != Some(&id)).then_some(id);
                self.list.show_filter_menu = false;
            }
            Msg::ClearFilters => {
                self.set_filters(0);
                self.list.space_view_account = None;
            }
            _ => unreachable!("not a chat_list_filters message"),
        }
        self.settle()
    }

    /// Selecting several chats, and what the selection does.
    fn chat_list_selection(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::BeginSelecting(id) => {
                (self.selection, self.list.chat_menu, self.select_mode) = (vec![id.clone()], None, false);
                self.select_chat(&id);
            }
            Msg::ToggleSelectMode => {
                self.list.chat_menu = None;
                if self.list().active_tab != InboxTab::Requests || self.list().selected_account_filter.is_some() {
                    self.select_mode = !self.select_mode && self.selection.is_empty();
                    self.selection.clear();
                }
            }
            Msg::ToggleSelected(id) => {
                self.select_mode = false;
                match self.selection.iter().position(|c| *c == id) {
                    Some(i) => drop(self.selection.remove(i)),
                    None => self.selection.push(id.clone()),
                }
                self.select_chat(&id);
            }
            Msg::SelectionSet(flag, on) => {
                (self.selection_menu, self.list.chat_menu) = (None, None);
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
                    self.act(what, UiMessage::ChatList(Msg::UndoFlag(flag, undo, false)), None);
                }
                self.refresh_theme_and_tray();
            }
            Msg::SelectionRemind(at) => {
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
            Msg::SelectionMenu(menu, x) => self.selection_menu = Some((menu, x)),
            _ => unreachable!("not a chat_list_selection message"),
        }
        self.settle()
    }
}

pub(crate) const FILTERS: [(&str, char); 8] = [
    ("Unread", icon::MARK_EMAIL_UNREAD),
    ("Drafts", icon::SMS),
    ("Unanswered", icon::MODE_COMMENT),
    ("DMs", icon::PERSON),
    ("Groups", icon::GROUP),
    ("Archived", icon::ARCHIVE),
    ("Requests", icon::EMAIL),
    ("Low Priority", icon::LOW_PRIORITY_SPACE),
];

/// Desktop's `ThreadsFilter` values of [`FILTERS`], which its `threadsFilters` keeps.
const FILTER_IDS: [&str; 8] = ["unread", "drafts", "unresponded", "dm", "group", "archived", "requests", "low_priority"];

/// The file of Desktop's `threadsFilters`: the filters of the chat list, as a JSON array of their values.
const FILTERS_FILE: &str = "threadsFilters";

/// Desktop's `Put`: the filters of the `threadsFilters` that the last launch kept, without unknown values.
pub(crate) fn saved_filters(data_dir: &std::path::Path) -> u8 {
    let Ok(json) = std::fs::read_to_string(data_dir.join(FILTERS_FILE)) else { return 0 };
    let ids: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
    FILTER_IDS.iter().enumerate().filter(|(_, id)| ids.iter().any(|i| i == *id)).fold(0, |f, (i, _)| f | 1 << i)
}

impl BeeperGuiApp {
    /// Desktop's `persistThreadsFilters`: the chat list shows `filters`, which the next launch restores.
    pub(crate) fn set_filters(&mut self, filters: u8) {
        self.list.filters = filters;
        let ids: Vec<&str> = (0..FILTER_IDS.len()).filter(|i| filters & 1 << i != 0).map(|i| FILTER_IDS[i]).collect();
        let path = self.store.read().config.data_dir.join(FILTERS_FILE);
        std::fs::write(path, serde_json::json!(ids).to_string()).expect("write the chat list filters");
    }
}

pub(crate) const F_ARCHIVED: usize = 5;

pub(crate) const F_REQUESTS: usize = 6;

pub(crate) const F_LOW_PRIORITY: usize = 7;

pub(crate) fn filter_hit(i: usize, c: &Chat, st: &BeeperStore) -> bool {
    match i {
        0 => c.flags & CHAT_UNREAD != 0,
        1 => c.draft.is_some(),
        2 => c.preview.as_ref().is_none_or(|m| m.is_sender != Some(true) && !m.reactions.iter().any(|r| r.participant_id == st.me().id)),
        3 => c.chat_type != "group",
        4 => c.chat_type == "group",
        F_ARCHIVED => c.archived_in_list(),
        F_REQUESTS => c.flags & CHAT_REQUEST != 0,
        F_LOW_PRIORITY => c.flags & CHAT_LOW_PRIORITY != 0,
        _ => unreachable!("filter index {i}"),
    }
}

/// Desktop's `pot`: the text of the draft of a chat on one line.
pub(crate) fn draft_line(c: &Chat) -> Option<String> {
    c.draft.as_ref()?.text.as_deref().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Desktop's `getReadIcon` of a chat list preview: an own message shows its send state, but an own reaction does not,
/// nor an action like "You left the chat" (`isAction`).
pub(crate) fn read_icon(m: &Message) -> bool {
    m.is_sender == Some(true) && m.message_type.as_deref() != Some("REACTION") && !m.silent()
}

/// The text of a chat list preview, or a label for its links.
pub(crate) fn preview_body(m: &Message) -> Option<String> {
    // Desktop's `CP`: view-once media previews as what it is.
    if let Some(text) = crate::view_once::preview(m) {
        return Some(text.into());
    }
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

/// The media badge of a chat list preview: whether it is a video, and its label.
/// Desktop's chat list badge for a message's attachments: a `TAG_ICONS` index, the label, and the detail that
/// shows when the message has no text, with whether that detail is italic.
pub(crate) type MediaTag = (usize, &'static str, Option<(String, bool)>);

/// The index of Desktop's Poll badge, which shows the `poll-options` icon of the composer.
pub(crate) const POLL_TAG: usize = 10;

/// The `TAG_ICONS` indices of the Video badge, whose `videocam` icon the video calls share, and of the call badge.
pub(crate) const VIDEO_TAG: usize = 1;

pub(crate) const CALL_TAG: usize = 9;

/// Desktop's `_P`: the badge of a poll, then of a call, then of the attachments.
pub(crate) fn message_tag(m: &Message) -> Option<MediaTag> {
    if m.view_once() {
        return None;
    }
    if m.poll.is_some() {
        return Some((POLL_TAG, trx("Poll", "chat list badge for a poll message"), None));
    }
    match &m.call {
        Some(call) => Some((if call.is_video() { VIDEO_TAG } else { CALL_TAG }, call_label(call), None)),
        None => media_tag(&m.attachments),
    }
}

/// Desktop's `Yle`: the label of a call.
pub(crate) fn call_label(call: &beeper_core::models::CallAction) -> &'static str {
    match call.call_type.as_deref() {
        Some("video") => tr("Video call"),
        Some("voice") => tr("Voice call"),
        _ => trx("Call", "generic call type label"),
    }
}

pub(crate) fn tag_glyph(tag: usize) -> &'static svg::Handle {
    if tag == POLL_TAG {
        &COMPOSER_ICONS[8]
    } else {
        &TAG_ICONS[tag]
    }
}

pub(crate) fn media_tag(attachments: &[Attachment]) -> Option<MediaTag> {
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
        _ if ["application/zip", "application/x-zip-compressed", "multipart/x-zip"].contains(&mime.as_str()) || file.ends_with(".zip") => {
            (6, tr("ZIP"), name)
        }
        _ => (7, tr("Document"), name),
    })
}

/// Desktop's chat list badge icons: image, videocam, volume_up, audio_file, account_circle, csv, folder_zip,
/// draft and photo_library, at its `scale(1.2)` in a 12 px box.
pub(crate) static TAG_ICONS: std::sync::LazyLock<[svg::Handle; 10]> = std::sync::LazyLock::new(|| {
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
    .map(|d| crate::path_svg("80 -880 800 800", d))
});

impl BeeperGuiApp {
    /// Desktop's `f` of `defaultOnKeyDown`: no chat or draft is open and nothing else takes the keys, so ↑, ↓, J, K
    /// and Enter work on the cursor of the chat list.
    pub(crate) fn points_chats(&self) -> bool {
        self.pane().selected_chat_id.is_none()
            && self.creating_chat.is_none()
            && !self.show_command_palette
            && self.forward.is_none()
            && self.hotkeys_modal.is_none()
            && !self.menu_open()
            && !self.select_mode
            && self.selection.is_empty()
    }

    /// [`Self::points_chats`] in the main window or in a space window `id`.
    pub(crate) fn window_points_chats(&mut self, id: window::Id) -> bool {
        if Some(id) == self.main_window {
            return self.points_chats();
        }
        if !self.space_windows.iter().any(|w| w.0 == id) {
            return false;
        }
        self.swap_window(id);
        let on = self.points_chats();
        self.swap_window(id);
        on
    }

    /// Desktop's `pointNextThread` and `pointPrevThread`: the cursor moves to the next or the previous chat of the
    /// list, and from a chat that the list does not show, to the first chat.
    pub(crate) fn point_chat(&mut self, down: bool) {
        let ids = self.list_ids(&self.store.read());
        let at = self.list.cursor.as_ref().and_then(|c| ids.iter().position(|i| i == c));
        let to = match (at, down) {
            (None, _) => Some(0),
            (Some(i), true) => Some(i + 1),
            (Some(i), false) => i.checked_sub(1),
        };
        if let Some(id) = to.and_then(|i| ids.get(i)) {
            self.list.cursor = Some(id.clone());
        }
    }

    /// Whether the chat list is in Desktop's `threadSelectionMode`, with chats selected or the checkboxes shown.
    pub(crate) fn selecting_chats(&self) -> bool {
        self.select_mode || !self.selection.is_empty()
    }

    /// Whether the main window is in Desktop's selection mode (`threadSelectionMode`), outside the chat windows,
    /// which have their own: its composer is gone and its open chat is not read.
    pub(crate) fn main_selecting(&self) -> bool {
        self.swapped.is_none() && self.drawing.get().is_none() && self.selecting_chats()
    }

    /// Whether the keys of the selection mode move the selection: not while the command bar or a dialog is open.
    pub(crate) fn selection_keys(&self) -> bool {
        self.selecting_chats()
            && !self.show_command_palette
            && self.forward.is_none()
            && self.creating_chat.is_none()
            && self.confirm.is_none()
            && !self.menu_open()
    }

    /// Desktop's `u` of the keyboard: X or Shift+↑/↓ with no chat open selects the chat of the cursor, which starts
    /// the selection mode.
    pub(crate) fn begin_chat_selection(&mut self) {
        // `toggleThreadSelectionMode` drops the cursor.
        let pointed = self.list.cursor.take().filter(|c| self.list_ids(&self.store.read()).contains(c));
        match pointed {
            Some(id) => (self.selection, self.select_mode) = (vec![id], false),
            None => (self.selection, self.select_mode) = (Vec::new(), true),
        }
    }

    /// Desktop's `selectThreadKeyboard` in the selection mode: ↓ or ↑ selects only the chat after or before the
    /// lowest selected chat, and with Shift adds the chat after the lowest or before the highest selected chat. With
    /// nothing selected, it selects the chat after the first. The list scrolls to the chat.
    pub(crate) fn select_chat_key(&mut self, shift: bool, down: bool) -> Task<UiMessage> {
        let ids = self.list_ids(&self.store.read());
        let first = ids.iter().position(|c| self.selection.contains(c));
        let last = ids.iter().rposition(|c| self.selection.contains(c));
        let from = match (first, last) {
            (Some(f), Some(_)) if shift && !down => f,
            (Some(_), Some(l)) => l,
            _ => 0,
        };
        let Some(id) = (if down { from.checked_add(1) } else { from.checked_sub(1) }).and_then(|i| ids.get(i)) else {
            return Task::none();
        };
        if !shift && last.is_some() {
            self.selection.clear();
        }
        self.selection.retain(|c| c != id);
        self.selection.push(id.clone());
        self.select_mode = false;
        // Desktop's `scrollThreadsListToItem`.
        let pinned = self.pins_and_rows(&self.store.read()).0.iter().any(|c| c.id == *id);
        let to = self.list.rows.nearest((!pinned).then(|| vlist::key(id)));
        let scroll = self.list.scroll.0.clone();
        to.map_or_else(Task::none, |y| operation::scroll_to(scroll, operation::AbsoluteOffset { x: 0.0, y }))
    }

    /// The open chat and the chat of the cursor, which [`Self::reveal_pointed`] follows.
    pub(crate) fn pointed(&self) -> (Option<String>, Option<String>) {
        (self.pane.selected_chat_id.clone(), self.list.cursor.clone())
    }

    /// Desktop's `scrollThreadsListToItem`, which `bringThreadToViewOnSelection` and `setThreadCursorID` call: when the
    /// open chat or the cursor moved from `before` to another chat, the list scrolls that chat into view with the
    /// least scroll (`align: "nearest"`). A pinned tile brings the top of the list, where Desktop's first item holds
    /// the whole pinned grid.
    pub(crate) fn reveal_pointed(&self, before: (Option<String>, Option<String>)) -> Task<UiMessage> {
        let (open, cursor) = self.pointed();
        let moved = |now: Option<String>, was: Option<String>| now.filter(|n| was.as_ref() != Some(n));
        let Some(id) = moved(open, before.0).or_else(|| moved(cursor, before.1)) else { return Task::none() };
        let pinned = self.pins_and_rows(&self.store.read()).0.iter().any(|c| c.id == id);
        let to = self.list.rows.nearest((!pinned).then(|| vlist::key(&id)));
        let scroll = self.list.scroll.0.clone();
        to.map_or_else(Task::none, |y| operation::scroll_to(scroll, operation::AbsoluteOffset { x: 0.0, y }))
    }

    /// The selected chats that still exist: the sync task can drop one before StoreChanged reaches update.
    /// The chat menu rows that a selection of more chats adds: the count, and both Mark All items.
    pub(crate) fn selection_rows(&self, st: &BeeperStore) -> f32 {
        let picked = self.selected_chats(st);
        let unread = picked.iter().filter(|c| has_flag(c, ChatFlag::MarkedUnread)).count();
        match picked.len() {
            0 | 1 => 0.0,
            n if (1..n).contains(&unread) => 2.0 * menu_pitch(),
            _ => menu_pitch(),
        }
    }

    pub(crate) fn selected_chats<'s>(&self, st: &'s BeeperStore) -> Vec<&'s Chat> {
        self.selection.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect()
    }

    pub(crate) fn view_selection_head(&self) -> El<'_> {
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

    pub(crate) fn view_selection_bar(&self) -> El<'_> {
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
                hotkey_caps(st.settings.hotkey_keys("Chat: Toggle Archive")),
            ),
            (
                ChatFlag::MarkedUnread,
                [tr("Mark Unread"), tr("Mark Read")],
                [tr("Mark as Unread"), tr("Mark as Read")],
                [&ROW_ACTIONS[3], &ROW_ACTIONS[2]],
                hotkey_caps(st.settings.hotkey_keys("Chat: Toggle Mark As Read")),
            ),
            (
                ChatFlag::Muted,
                [trx("Mute", "thread context menu action"), trx("Unmute", "thread context menu action")],
                [trx("Mute", "thread context menu action"), trx("Unmute", "thread context menu action")],
                [&SELECTION_ICONS[2], &SELECTION_ICONS[3]],
                hotkey_caps(st.settings.hotkey_keys("Chat: Toggle Mute")),
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
            (labels[on], tips[on], glyphs[on], UiMessage::ChatList(chat_list::Msg::SelectionSet(f, on == 0)), keys)
        });
        let menus = [(tr("Remind Me"), &SELECTION_ICONS[6]), (tr("More"), &SELECTION_ICONS[7])].into_iter().zip(0u8..).map(
            |((label, glyph), i)| {
                let x = x0 + cell * (4.5 + f32::from(i));
                (label, label, glyph, UiMessage::ChatList(chat_list::Msg::SelectionMenu(i, x)), vec![])
            },
        );
        let wide = cell * 6.0 >= 800.0;
        let buttons = toggles.into_iter().chain(menus).map(|(label, tip, glyph, msg, keys)| {
            let face = row![]
                .push(wide.then(|| semi(label, 14.0).color(p.strong)))
                .push(tinted(glyph, 24.0, p.strong))
                .spacing(10)
                .align_y(Center);
            let btn = button(container(face).center_x(Fill)).padding([6, 0]).width(Fill).style(btn_style([CLEAR; 3], [p.strong; 3], 0.0));
            let btn = btn.on_press(msg).into();
            match self.selection_menu {
                Some(_) => btn,
                None => self.tip_caps(btn, tip, keys, tooltip::Position::Top).into(),
            }
        });
        let bar = container(Row::with_children(buttons).align_y(Center)).center_y(36).width(Fill).style(fill_box(p.card, 18.0));
        container(bar).padding(8).style(fill_box(p.selected, 26.0)).into()
    }

    /// The Remind Me or More menu of the selection bar, and its height.
    pub(crate) fn view_selection_menu(&self, which: u8) -> (El<'_>, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let chats = self.selected_chats(&st);
        let item = |label: String, at: String, msg| menu_item(p, None, label, at, msg);
        let items: Vec<El> = if which == 0 {
            let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
            let last = last_used(&st, now).map(|at| {
                item(tr("Last Used").into(), last_used_label(at, h24), Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(Some(at)))))
            });
            let presets = remind_presets(&st.settings, now).into_iter().map(|(label, at)| {
                let msg = UiMessage::Commands(vec![
                    UiMessage::CommandBar(commands::Msg::LastTime(label.clone())),
                    UiMessage::ChatList(chat_list::Msg::SelectionRemind(Some(at))),
                ]);
                item(label, remind_label(at, h24), Some(msg))
            });
            let presets = last.into_iter().chain(presets);
            let set = chats.iter().filter(|c| c.reminder.is_some()).count();
            let dismiss = match set {
                0 => vec![],
                1 => vec![
                    menu_rule(p),
                    item(tr("Dismiss Reminder").into(), String::new(), Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(None)))),
                ],
                n => vec![
                    menu_rule(p),
                    item(format!("Dismiss {n} Reminders"), String::new(), Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(None)))),
                ],
            };
            presets.chain(dismiss).collect()
        } else {
            let n = chats.len();
            let count = |f: ChatFlag| chats.iter().filter(|c| has_flag(c, f)).count();
            let some = |f: ChatFlag| (1..n).contains(&count(f));
            let entry = |label: &str, f: ChatFlag, on: bool| {
                let hotkey = match f {
                    ChatFlag::Archived => Some("Chat: Toggle Archive"),
                    ChatFlag::MarkedUnread => Some("Chat: Toggle Mark As Read"),
                    ChatFlag::Muted => Some("Chat: Toggle Mute"),
                    ChatFlag::Pinned => Some("Chat: Toggle Pin"),
                    _ => None,
                };
                let keys = hotkey.map(|h| hotkey_text(st.settings.hotkey_keys(h))).unwrap_or_default();
                item(label.into(), keys, Some(UiMessage::ChatList(chat_list::Msg::SelectionSet(f, on))))
            };
            let pinned = count(ChatFlag::Pinned);
            [Some(item(trn("%d chat selected", "%d chats selected", n, &[&n]), String::new(), None))]
                .into_iter()
                .chain([
                    some(ChatFlag::Archived).then(|| entry(tr("Unarchive"), ChatFlag::Archived, false)),
                    some(ChatFlag::MarkedUnread).then(|| entry(tr("Mark as Read"), ChatFlag::MarkedUnread, false)),
                    some(ChatFlag::Muted).then(|| entry(trx("Unmute", "thread context menu action"), ChatFlag::Muted, false)),
                    some(ChatFlag::LowPriority).then(|| entry(tr("Move to Inbox"), ChatFlag::LowPriority, false)),
                    (pinned < n).then(|| entry(tr("Pin"), ChatFlag::Pinned, true)),
                    (pinned > 0).then(|| entry(tr("Unpin"), ChatFlag::Pinned, false)),
                ])
                .flatten()
                .collect()
        };
        let rules = if which == 0 && chats.iter().any(|c| c.reminder.is_some()) { MENU_RULE - menu_pitch() } else { 0.0 };
        let h = 8.0 + items.len() as f32 * menu_pitch() + rules;
        let width = if which == 0 { SUB_MENU_W[1] } else { 233.0 };
        let menu = container(Column::with_children(items).padding([4, 0])).width(width).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        (menu.into(), h)
    }

    pub(crate) fn view_filter_menu(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let item = |checked: bool, glyph: char, label: String, hotkey: String, bit: u8| -> El<'_> {
            let lead = row![
                container(checked.then(|| ic(icon::CHECK, 14.0))).center_x(14),
                container(ic(glyph, 14.0)).padding(Padding::ZERO.left(9.0).right(4.0)),
            ];
            menu_item(p, Some(lead.into()), label, hotkey, Some(UiMessage::ChatList(chat_list::Msg::PickFilter(bit))))
        };
        let counted = (0..F_ARCHIVED).map(|i| {
            let n = self.chats_with(&st, 1 << i).len();
            let hotkey = prefs::hotkey_label(st.settings.hotkey_keys(FILTER_HOTKEYS[i]));
            item(self.list().filters & 1 << i != 0, FILTERS[i].1, format!("{} ({n})", tr(FILTERS[i].0)), hotkey, 1 << i)
        });
        let in_view = matches!(self.list().active_tab, InboxTab::Archived | InboxTab::LowPriority | InboxTab::Requests);
        let views: &[usize] = match self.list().active_tab {
            InboxTab::Archived => &[F_LOW_PRIORITY],
            InboxTab::LowPriority => &[F_ARCHIVED],
            InboxTab::Requests => &[],
            // Desktop's `anySelectedSpaceSupportsRequests`.
            _ if self.list().selected_account_filter.as_deref().is_none_or(|id| takes_requests(&st, id)) => {
                &[F_ARCHIVED, F_REQUESTS, F_LOW_PRIORITY]
            }
            _ => &[F_ARCHIVED, F_LOW_PRIORITY],
        };
        let rule = (!views.is_empty()).then(|| menu_rule(p));
        let views =
            views.iter().map(|&i| item(self.list().filters & 1 << i != 0, FILTERS[i].1, tr(FILTERS[i].0).into(), String::new(), 1 << i));
        // Desktop's account spaces of a space view, which in Requests are the ones whose bridge takes requests.
        let accounts: Vec<String> = match in_view {
            true => rail_spaces(&st)
                .into_iter()
                .filter(|id| rail_label(&st, id).is_none() && !RAIL_VIEWS.iter().any(|v| v.0 == id))
                .filter(|id| self.list().active_tab != InboxTab::Requests || takes_requests(&st, id))
                .collect(),
            false => Vec::new(),
        };
        let account_rule = (!accounts.is_empty()).then(|| menu_rule(p));
        let names: Vec<String> = accounts.iter().map(|id| space_name(&st, id)).collect();
        // Desktop's menu grows past its least width for a long account name.
        let width = names.iter().map(|n| 104.0 + text_width(n, 13.0, Weight::Normal)).fold(233.0, f32::max);
        let accounts = accounts.into_iter().zip(names).map(|(id, name)| {
            let lead = row![
                container((self.list().space_view_account.as_ref() == Some(&id)).then(|| ic(icon::CHECK, 14.0))).center_x(14),
                container(svg(brand_svg(brand_key(&rail_owner(&st, &id)))).width(14).height(14))
                    .padding(Padding::ZERO.left(9.0).right(4.0)),
            ];
            let msg = UiMessage::ChatList(chat_list::Msg::PickAccount(id.clone()));
            menu_item(p, Some(lead.into()), name, "", Some(msg))
        });
        let col = Column::with_children(
            std::iter::once(item(
                self.list().filters == 0,
                icon::INBOX,
                tr("All").into(),
                prefs::hotkey_label(st.settings.hotkey_keys("Filter: All")),
                0,
            ))
            .chain(counted)
            .chain(rule)
            .chain(views)
            .chain(account_rule)
            .chain(accounts),
        );
        crate::menu_card(p, col, width).into()
    }

    /// The chat menu, with `room` pixels from its top to the bottom of the window.
    pub(crate) fn view_chat_menu(&self, cid: &str, sub_left: bool, room: f32) -> El<'_> {
        if let Some(menu) = self.view_other_menu(cid) {
            return menu;
        }
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let m = MenuChats { st: &st, c, picked: self.selected_chats(&st) };
        let menu = crate::menu_card(&self.pal, self.chat_menu_items(&m), chat_menu_w(&st));
        let sub = self.chat_sub.map(|i| self.view_chat_sub(&m, i, room));
        let (l, r) = if sub_left { (sub, None) } else { (None, sub) };
        row![].push(l).push(menu).push(r).into()
    }

    /// The menus that share the chat menu's slot but are not a chat's: the chat list's, the rail's, Chat Info's, a
    /// space's, the Snooze and behavior menus of Chat Info, and a new-chat draft's.
    fn view_other_menu(&self, cid: &str) -> Option<El<'_>> {
        let p = &self.pal;
        if cid == SIDEBAR_MENU {
            let st = self.store.read();
            let keys = |name| prefs::hotkey_label(st.settings.hotkey_keys(name));
            let bar = if st.settings.on("showSidebar") { "Hide Space Bar" } else { "Show Space Bar" };
            let settings = |tab| UiMessage::Settings(settings::Msg::Open(tab));
            let items = [
                (tr("Start New Chat"), keys("Chat: Start New"), UiMessage::NewChat(new_chat::Msg::Open)),
                (tr("Add/Remove Accounts"), String::new(), settings(SettingsTab::Accounts)),
                (tr(bar), keys("Toggle Space Bar"), UiMessage::RunHotkey("Toggle Space Bar")),
                (tr("Settings"), String::new(), settings(SettingsTab::General)),
            ]
            .map(|(label, key, msg)| menu_item(p, None, label.into(), key, Some(msg)));
            return Some(crate::menu_card(p, Column::with_children(items), 233).into());
        }
        if cid == RAIL_MORE {
            let items = more_menu(&self.store.read()).into_iter().map(|i| match i {
                Some((label, msg)) => menu_item(p, None, label, "", Some(msg)),
                None => menu_rule(p),
            });
            return Some(crate::menu_card(p, Column::with_children(items), 233).into());
        }
        if INFO_MENUS.iter().any(|k| cid.starts_with(k)) {
            let items = info_menu(&self.store.read(), cid, |a| self.can(a, "create_dm")).into_iter().map(|i| match i {
                Some((label, msg)) => menu_item(p, None, label, "", msg),
                None => menu_rule(p),
            });
            return Some(crate::menu_card(p, Column::with_children(items), 233).into());
        }
        if let Some(entry) = cid.strip_prefix("account:") {
            return Some(self.view_account_menu(entry));
        }
        if cid == RAIL_MENU {
            return Some(self.rail_menu(&self.store.read(), None));
        }
        if let Some(entry) = cid.strip_prefix("head:") {
            let (menu, sub) = self.account_menu(entry, SpaceMenu::Header);
            return Some(row![menu].push(sub).into());
        }
        // Desktop's Snooze menu of the Chat Info toolbar.
        if let Some(id) = cid.strip_prefix(SNOOZE_MENU) {
            let rows = self.snooze_items(&self.store.read(), &[id.to_string()]);
            return Some(crate::menu_rows(p, rows, SUB_MENU_W[3]).0.into());
        }
        if let Some(id) = cid.strip_prefix(behavior::BEHAVIOR_MENU) {
            let (rows, w) = self.behavior_items(&self.store.read(), id);
            return Some(crate::menu_rows(p, rows, w).0.into());
        }
        let n = cid.strip_prefix("draft:")?;
        let delete = UiMessage::NewChat(new_chat::Msg::Delete(n.parse().unwrap()));
        Some(crate::menu_card(p, column![chat_item(p, tr("Delete Chat"), "", Some(delete))], 124).into())
    }

    /// The items of a chat's menu, in the groups that close an open submenu on hover, and the submenu openers.
    fn chat_menu_items<'a>(&'a self, m: &MenuChats) -> Column<'a, UiMessage> {
        let (p, st, c, n) = (&self.pal, m.st, m.c, m.picked.len());
        let close_sub = |items: Column<'a, UiMessage>| mouse_area(items).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(None)));
        let open_sub = |i: u8, label: &str| {
            let open = UiMessage::CommandBar(commands::Msg::ChatSub(Some(i)));
            mouse_area(menu_item(p, None, label.into(), "›", Some(open.clone()))).on_enter(open)
        };
        let low = if n == 0 { c.flag(ChatFlag::LowPriority) } else { m.every(ChatFlag::LowPriority) };
        let low = chat_item(
            p,
            tr(if low { "Move to Inbox" } else { "Move to Low Priority" }),
            "",
            if n == 0 { flag_msg(c, ChatFlag::LowPriority) } else { set_msg(ChatFlag::LowPriority, !low) },
        );
        let search = UiMessage::CommandBar(commands::Msg::Search(Some(c.id.clone())));
        column![
            close_sub(self.chat_menu_head(m)),
            open_sub(0, tr("Copy")),
            close_sub(column![menu_rule(p), chat_item(p, tr("Search…"), &m.key("Search: Room"), Some(search)), menu_rule(p), low,]),
            st.feature("labels-v2").on.then(|| open_sub(2, trx("Label", "thread context menu submenu label"))),
            open_sub(1, tr("Remind Me")),
            reminders::shows_snooze(st, &m.chats()).then(|| open_sub(3, trx("Snooze", "thread context menu submenu label"))),
            close_sub(self.chat_menu_tail(m)),
        ]
    }

    /// The first group of a chat's menu: the answers to a request, the count of the selected chats, Archive, Mute,
    /// the reason of a mute, the read items, Pin, Follow All Threads and Edit Group.
    fn chat_menu_head<'a>(&'a self, m: &MenuChats) -> Column<'a, UiMessage> {
        let (p, st, c, n) = (&self.pal, m.st, m.c, m.picked.len());
        let reason = m.reason();
        let heading = (n > 1).then(|| chat_item(p, &trn("%d chat selected", "%d chats selected", n, &[&n]), "", None));
        let (accept, decline) = request_answers(c);
        let group = c.chat_type == "group";
        let accept = accept.then(|| {
            let label = tr(if group { "Accept group invitation" } else { "Accept message request" });
            chat_item(p, label, "", Some(UiMessage::ChatList(chat_list::Msg::AcceptRequest(c.id.clone()))))
        });
        let decline = decline.then(|| {
            let label = tr(if group { "Decline group invitation" } else { "Decline message request" });
            chat_item(p, label, "", Some(UiMessage::ChatList(chat_list::Msg::Delete(c.id.clone(), false))))
        });
        let answered = (accept.is_some() || decline.is_some()).then(|| menu_rule(p));
        let pinned = if n == 0 { c.flag(ChatFlag::Pinned) } else { m.every(ChatFlag::Pinned) };
        let pin = if n == 0 { flag_msg(c, ChatFlag::Pinned) } else { set_msg(ChatFlag::Pinned, !pinned) };
        let pin_label = if pinned { trx("Unpin", "thread context menu action") } else { trx("Pin", "thread context menu action") };
        column![]
            .push(accept)
            .push(decline)
            .push(answered)
            .push(heading)
            .push(m.archive_item(p))
            .push(m.mute_item(p, reason.is_some()))
            .push(reason.as_deref().map(|r| chat_item(p, r, "", None)))
            .extend(m.read_items(p))
            .push(chat_item(p, pin_label, &m.key("Chat: Toggle Pin"), pin))
            .push((n <= 1 && st.threaded(c)).then(|| {
                let on = c.follows.follows_all(c.participants.total);
                let label = tr(if on { "Stop Following All Threads" } else { "Follow All Threads" });
                chat_item(p, label, "", Some(UiMessage::Thread(threads::Msg::FollowAll(c.id.clone(), !on))))
            }))
            .push(can_edit_group(c).then(|| {
                let open = UiMessage::OpenEditGroup(c.id.clone());
                chat_item(p, tr("Edit Group..."), "", (c.is_request != Some(true)).then_some(open))
            }))
    }

    /// The last group of a chat's menu: Delete Chat or Unmerge, Block, Open in New Window, Begin Selecting and
    /// Report a Problem.
    fn chat_menu_tail<'a>(&'a self, m: &MenuChats) -> Column<'a, UiMessage> {
        let (p, st, c, n) = (&self.pal, m.st, m.c, m.picked.len());
        let selecting = self.selection.is_empty();
        // Desktop's Unmerge of one merged chat, in the place of Delete Chat.
        let delete = match n <= 1 && c.merge.is_some() {
            true => Some(chat_item(
                p,
                trx("Unmerge", "thread context menu action to split a merged chat"),
                "",
                Some(UiMessage::Merged(merged::Msg::Ask(c.id.clone()))),
            )),
            false => st.feature("delete-chats").on.then(|| {
                let ask = (c.delete_modes != 0).then(|| UiMessage::ChatList(chat_list::Msg::AskDelete(c.id.clone())));
                chat_item(p, tr("Delete Chat"), "", ask)
            }),
        };
        let block = block_action(st, c).filter(|_| n == 0).map(|block| {
            let label = match block {
                true => trx("Block…", "action to block the other person in a DM"),
                false => trx("Unblock…", "action to unblock the other person in a DM"),
            };
            chat_item(p, label, "", Some(UiMessage::Blocking(blocking::Msg::Ask(c.id.clone(), block))))
        });
        let window = UiMessage::Window(windows::Msg::OpenChat(c.id.clone()));
        let select = UiMessage::ChatList(chat_list::Msg::BeginSelecting(c.id.clone()));
        // Like Electron's menus, a group without items has no separator.
        column![]
            .push((delete.is_some() || block.is_some()).then(|| menu_rule(p)))
            .push(delete)
            .push(block)
            .push(selecting.then(|| menu_rule(p)))
            .push(selecting.then(|| chat_item(p, tr("Open in New Window"), "", Some(window))))
            .push(selecting.then(|| chat_item(p, tr("Begin Selecting"), "", Some(select))))
            .push(menu_rule(p))
            .push(chat_item(p, tr("Report a Problem"), &m.key("Report a Problem"), Some(UiMessage::OpenUrl(ISSUES_URL.into()))))
    }

    /// The open submenu `i` of a chat's menu: Copy, Remind Me, Label or Snooze, next to the item that opens it and
    /// inside the window.
    fn view_chat_sub<'a>(&'a self, m: &MenuChats, i: u8, room: f32) -> Column<'a, UiMessage> {
        let (p, st, c) = (&self.pal, m.st, m.c);
        let (items, rules): (Vec<El>, f32) = match i {
            0 => {
                let copy = UiMessage::ChatList(chat_list::Msg::CopyLink(c.id.clone()));
                (vec![chat_item(p, tr("Chat Link"), "", Some(copy))], 0.0)
            }
            2 => m.label_items(p),
            1 => m.remind_items(p),
            _ => {
                let rows = self.snooze_items(st, &m.targets());
                let rules = rows.iter().filter(|r| r.1 < menu_pitch()).count() as f32;
                (rows.into_iter().map(|r| r.0).collect(), rules)
            }
        };
        let labels = st.feature("labels-v2").on;
        let h = 8.0 + items.len() as f32 * menu_pitch() - rules * (menu_pitch() - MENU_RULE);
        let top = match i {
            0 => 4.0 * menu_pitch(),
            1 if labels => 8.0 * menu_pitch() + 22.0,
            3 if labels => 9.0 * menu_pitch() + 22.0,
            3 => 8.0 * menu_pitch() + 22.0,
            _ => 7.0 * menu_pitch() + 22.0,
        } + if m.reason().is_some() { menu_pitch() } else { 0.0 }
            + if can_edit_group(c) { menu_pitch() } else { 0.0 }
            + self.selection_rows(st);
        let top = top.min(room - h).max(0.0);
        let body = crate::menu_card(p, Column::with_children(items), SUB_MENU_W[usize::from(i)]);
        column![Space::new().height(top), body]
    }

    /// Desktop's `hasExpiredReminder`: the reminder time came while the reminder was set.
    /// Desktop's `selectThreadRange` (`yot`): selects the chats from the last selected chat, or the open chat, to `id`.
    pub(crate) fn select_range(&mut self, id: &str) {
        let list = self.list_ids(&self.store.read());
        let from = self.selection.last().or(self.pane.selected_chat_id.as_ref()).map_or(id, String::as_str);
        let at = |c: &str| list.iter().position(|l| l == c).unwrap_or(0);
        let (a, b) = (at(from), at(id));
        let range = &list[a.min(b)..=a.max(b)];
        self.selection.retain(|c| !range.contains(c));
        self.selection.extend_from_slice(range);
        self.select_mode = false;
    }

    pub(crate) fn view_sidebar(&self) -> El<'_> {
        let st = self.store.read();
        let w = self.list_width(&st);
        let header = self.view_list_header(&st);
        let list = self.view_list(&st, w);
        let pill = self.view_global_audio(&st, w - 18.0).map(|pill| {
            let at = Padding::ZERO.left(8.0).right(10.0).bottom(self.audio.bottom);
            container(pill).padding(at).height(Fill).align_bottom(Fill)
        });
        let status = self.view_network_status(&st);
        let banner = self.view_key_banner(&st);
        let sidebar = stack![column![header].push(status).push(banner).push(list).padding(Padding::ZERO.left(8.0))].push(pill);
        let sidebar = container(sidebar).width(w).height(Fill);
        mouse_area(sidebar).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(SIDEBAR_MENU.into()))).into()
    }

    /// A button of the chat list header, which takes the focus without showing it, like Desktop's
    /// (`button { outline: 0 }`).
    fn header_target<'a>(&self, key: &str, b: impl Into<El<'a>>, msg: UiMessage) -> El<'a> {
        self.target(self.drawing.get().or(self.main_window), key, b, Keys::press(msg), Look::Hidden)
    }

    /// The header of the chat list: the space's title, then the Filter, Display Options, Search and New Chat buttons.
    fn view_list_header(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let head_btn = |key: &str, c: char, size: f32, [l, r]: [f32; 2], msg: UiMessage| {
            let b = button(container(ic(c, size).color(p.strong)).padding(Padding::ZERO.left(l).right(r).top(2.0)).center_y(26))
                .bare(p.strong)
                .on_press(msg.clone());
            self.header_target(key, b, msg)
        };
        let below = |el: El<'static>, label, keys| self.tip_caps(el, label, keys, tooltip::Position::Bottom);
        let options = (self.list().active_tab == InboxTab::Reminders).then(|| {
            let menu = UiMessage::Reminders(reminders::Msg::Menu);
            let btn = button(container(tinted(&TUNE, 18.0, p.strong)).center_x(30).center_y(26)).bare(p.strong).on_press(menu.clone());
            below(self.header_target("list-options", btn, menu), tr("Display Options"), vec![])
        });
        let search = UiMessage::CommandBar(commands::Msg::TogglePalette);
        let search = head_btn("list-search", icon::SEARCH, 23.0, [7.5, 10.0], search);
        let new_chat = head_btn("list-new-chat", icon::EDIT_SQUARE, 20.0, [0.0, 3.0], UiMessage::NewChat(new_chat::Msg::Open));
        row![self.view_list_title(st)]
            .push(self.view_filter_button())
            .push(options)
            .push(below(search, tr("Search Chats"), hotkey_caps(st.settings.hotkey_keys("Search"))))
            .push(below(new_chat, tr("New Chat"), hotkey_caps(st.settings.hotkey_keys("Chat: Start New"))))
            .align_y(Center)
            .height(52)
            .padding(Padding::ZERO.right(8.0))
            .into()
    }

    /// The title of the chat list: the space's name and icon, and the names of the filters in use.
    fn view_list_title(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let list = self.list();
        let names: Vec<String> = (0..FILTERS.len())
            .filter(|i| list.filters & 1 << i != 0)
            .map(|i| tr(FILTERS[i].0).to_string())
            .chain(list.space_view_account.as_deref().map(|id| space_name(st, id)))
            .collect();
        let heading = semi(list.title(st), 13.0).color(p.strong).wrapping(iced::widget::text::Wrapping::None);
        // Desktop's space icon before the title when the space bar is hidden: a label with a color, a network space,
        // or an account with another account on its network.
        let shown = st.settings.on("showSidebar") && !list.locked;
        let space_icon = list.selected_account_filter.as_deref().filter(|_| !shown).and_then(|id| -> Option<El<'_>> {
            if let Some(l) = rail_label(st, id) {
                return l.color.is_some().then(|| label_icon(l, 16.0, p.strong));
            }
            let owner = rail_owner(st, id);
            let twins = st.accounts.iter().filter(|a| brand_key(&a.account_id) == brand_key(&owner)).count() > 1;
            let icon = svg(brand_svg(brand_key(&owner))).width(16).height(16);
            (network_space(st, id).is_some() || twins).then(|| icon.into())
        });
        let line = row![].push(space_icon.map(|i| container(i).padding(Padding::ZERO.right(4.0)))).push(heading);
        // Desktop's header of a space window neither opens the space sheet nor has a menu, and a list of at most
        // 10.625rem inside its 8 px padding (`@container threads`) hides the chevron.
        let roomy = self.list_width(st) - 8.0 > rem(170.0);
        let chevron = (!list.locked && roomy).then(|| ic(icon::UNFOLD_MORE, 12.0).color(p.sub));
        let title = column![line.push(chevron).spacing(1.4).align_y(Center)]
            .push((!names.is_empty()).then(|| txt(names.join(", "), 10.0).color(p.muted)))
            .padding([0, 8]);
        let title: El<'_> = match list.locked {
            true => title.into(),
            false => {
                let sheet = UiMessage::ChatList(chat_list::Msg::OpenMenu(SPACE_SHEET.into()));
                let title = mouse_area(title)
                    .on_press(sheet.clone())
                    .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("head:{}", list.entry()))));
                self.header_target("list-title", title, sheet)
            }
        };
        container(title).width(Fill).clip(true).into()
    }

    /// Desktop's `FilterButton`, with the count of the filters in use, in the tabs that take filters.
    fn view_filter_button(&self) -> Option<El<'_>> {
        let p = &self.pal;
        self.list().active_tab.filterable().then(|| {
            let count = self.list().filters.count_ones() + u32::from(self.list().space_view_account.is_some());
            let active = count > 0;
            let fg = if active { p.accent } else { p.strong };
            let trigger: El<'_> = match count {
                0 => Space::new().into(),
                1 => ic(icon::EXPAND_MORE, 10.0).color(fg).into(),
                // `.FilterButton-module__count` is 11 px.
                n => crate::style::fixed(n.to_string(), 11.0, Weight::Normal).line_height(1.0).color(fg).into(),
            };
            let trigger = button(container(trigger).padding(Padding::ZERO.left(6.0)).center_y(20))
                .bare(fg)
                .width(if active { 16 } else { 0 })
                .on_press(UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu));
            let pill = row![trigger, container(ic(icon::FILTER_LIST, 16.8).color(fg)).center_x(27).center_y(20)];
            let ring = if active { p.accent } else { CLEAR };
            // `.tab-bar-btn button:hover` keeps it clear.
            let face = bordered_btn([CLEAR; 3], [fg; 3], ring, 1.0, 12.0);
            let press = UiMessage::ChatList(chat_list::Msg::FilterButton);
            let el = mouse_area(button(pill).padding(0).style(face).on_press(press.clone()))
                .on_right_press(UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu));
            self.tip(self.header_target("list-filter", el, press), tr("Filter"), vec![], tooltip::Position::Bottom).into()
        })
    }

    /// The rows of the chat list: the unread threads pill, the grid of pins, then the chat rows and the new-chat drafts.
    fn view_list(&self, st: &BeeperStore, w: f32) -> El<'_> {
        let (pinned, rest) = self.pins_and_rows(st);
        let bar_pad = if SOLID_BARS { 0.0 } else { 4.0 };
        let inner = w - 8.0 - bar_pad - GUTTER;
        let grid = self.pin_grid_of(st, pinned, inner);
        let look = self.row_look(st, &rest, inner);
        if self.visible_chats(st).is_empty() && self.new_chats.is_empty() {
            return self.view_empty_list();
        }
        let (cols, pinned) = (grid.cols, &grid.pinned);
        let tiles = pinned.len().div_ceil(cols);
        let row_keys = self.row_keys(&rest);
        let lead = pinned_rows(&rest);
        let drafts = self.new_chats.len();
        // Desktop's "Unread threads" pill comes first.
        let threads = self.unread_threads(st, true);
        let pre = usize::from(!threads.is_empty());
        let keys: Vec<u64> = (0..pre)
            .map(|_| vlist::key("unread-threads"))
            .chain((0..tiles).map(|i| vlist::key(("pins", i))))
            .chain(row_keys[..lead].iter().copied())
            .chain(self.new_chats.iter().map(|d| vlist::key(("draft", d.id))))
            .chain(row_keys[lead..].iter().copied())
            .collect();
        let estimate = if look.compact { 52.0 } else { 68.0 };
        // Desktop's `threads` list is a `.redacted-region`.
        let rows = self.list().rows.view(&keys, false, estimate, UiMessage::Relist, |i| {
            crate::style::region(true, || match i.checked_sub(pre) {
                None => self.unread_threads_pill(st, &threads),
                Some(i) if i < tiles => self.view_pin_row(&look, &grid, &pinned[i * cols..(i * cols + cols).min(pinned.len())]),
                Some(i) if i < tiles + lead => self.view_chat_row(&look, rest[i - tiles]),
                Some(i) if i < tiles + lead + drafts => self.view_draft_row(&look, &self.new_chats[i - tiles - lead]),
                Some(i) => self.view_chat_row(&look, rest[i - tiles - drafts]),
            })
        });
        scrollable(container(rows).padding(Padding::ZERO.right(bar_pad + GUTTER)))
            .id(self.list().scroll.0.clone())
            .direction(stable_bar())
            .height(Fill)
            .style(scroll_style(self.pal.scroll))
            .into()
    }

    /// The text of an empty chat list, with the buttons of the views to look in instead.
    fn view_empty_list(&self) -> El<'_> {
        let p = &self.pal;
        let tab = if self.list().selected_account_filter.is_some() { InboxTab::Requests } else { self.list().active_tab };
        let (head, hint) = tab.empty();
        let header = column![semi(head, 16.0).color(p.strong).align_x(Center).line_height(1.25)]
            .push((!hint.is_empty()).then(|| txt(hint, 13.0).color(p.strong).align_x(Center)))
            .spacing(12)
            .align_x(Center);
        let fallback = self.list().active_tab.fallback().map(|views| {
            let words = "If you're looking for something, check the".split(' ').map(|w| txt(w, 13.0).into());
            let pill = |t: InboxTab| -> El<'_> {
                button(semi(t.label(), 12.0))
                    .padding([6, 8])
                    .style(btn_style([p.selected; 3], [p.strong; 3], 999.0))
                    .on_press(UiMessage::ChatList(chat_list::Msg::SelectTab(t)))
                    .into()
            };
            let line = Row::with_children(words.chain([pill(views[0]), txt(tr("or"), 13.0).into(), pill(views[1])]))
                .spacing(3.5)
                .align_y(Center)
                .wrap()
                .vertical_spacing(5.5)
                .align_x(Center);
            container(line).center_x(Fill).padding([24, 0])
        });
        column![Space::new().height(Fill), header, Space::new().height(Fill)].push(fallback).padding([0, 22]).width(Fill).into()
    }

    /// The settings, sizes and selection that every row of the chat list reads.
    fn row_look<'s>(&'s self, st: &'s BeeperStore, rest: &[&'s Chat], inner: f32) -> RowLook<'s> {
        let compact = st.settings.on("compactChatList");
        let (av, lines, title_size, ts_size) = if compact { (28.0, 1, 13.0, 10.0) } else { (46.0, 2, 14.0, 11.0) };
        let counts = st.settings.on("showUnreadCount");
        let selecting = self.select_mode || !self.selection.is_empty();
        // Desktop's `isSelected` row is the open chat, or the chat of the cursor (`threadCursorID`).
        let open = self.pane().selected_chat_id.as_deref();
        let selected = self.creating_chat.is_none().then(|| open.or(self.list().cursor.as_deref())).flatten();
        let lead = match (selecting, counts) {
            (true, _) => 16.0,
            (false, true) => 0.0,
            (false, false) => 12.0,
        };
        RowLook {
            st,
            selected,
            above: self.row_above(rest, selected),
            compact,
            av,
            lines,
            title_size,
            ts_size,
            text_w: inner - 16.0 - av - 10.0 - lead,
            counts,
            network_icons: st.settings.on("showNetworkIcons")
                && self.list().selected_account_filter.as_deref().is_none_or(|id| rail_label(st, id).is_some()),
            colorize: st.settings.on("colorizeNetworkIcons"),
            hover_action: st.settings.pref("chatActionButton"),
            selecting,
            previews: st.settings.on("showMessagePreviews"),
            h24: st.settings.pref("timeFormat") == 1,
        }
    }

    /// The key of the row above the selected row or draft, whose divider Desktop hides too.
    fn row_above<'s>(&'s self, rest: &[&'s Chat], selected: Option<&str>) -> Option<Cow<'s, str>> {
        let (lead, drafts) = (pinned_rows(rest), self.new_chats.len());
        let draft = self.creating_chat.and_then(|n| self.new_chats.iter().position(|d| d.id == n)).map(|i| lead + i);
        let chat = || selected.and_then(|s| rest.iter().position(|c| c.id == s)).map(|i| i + drafts * usize::from(i >= lead));
        draft.or_else(chat).and_then(|i| i.checked_sub(1)).map(|i| match i.checked_sub(lead) {
            Some(d) if d < drafts => Cow::Owned(format!("draft:{}", self.new_chats[d].id)),
            Some(d) => Cow::Borrowed(rest[lead + d - drafts].id.as_str()),
            None => Cow::Borrowed(rest[i].id.as_str()),
        })
    }

    /// The pins in the order that a drag shows, and the sizes of their grid, which the drags read.
    fn pin_grid_of<'s>(&'s self, st: &BeeperStore, mut pinned: Vec<&'s Chat>, inner: f32) -> PinGrid<'s> {
        // The order of a pin drag, which moves the pins under the pointer.
        let dragged = self.pin_drag.as_ref().filter(|d| d.order.is_some()).map(|d| d.id.as_str());
        if let Some(order) = self.pin_drag.as_ref().and_then(|d| d.order.as_ref()) {
            pinned.sort_by_key(|c| order.iter().position(|o| *o == c.id).unwrap_or(usize::MAX));
        }
        // A pin that the keyboard moves shows in the place it drops at, which it marks like a drag of the mouse.
        let items: std::sync::Arc<[String]> = pinned.iter().map(|c| c.id.clone()).collect();
        let sort = self.key_sort_of(self.drawing.get().or(self.main_window), sortable::List::Pins);
        let dragged = dragged.or(sort.map(|s| s.id.as_str()));
        if let Some(order) = sort.and_then(|s| s.moved(&items)) {
            pinned.sort_by_key(|c| order.iter().position(|o| *o == c.id).unwrap_or(usize::MAX));
        }
        let compact = st.settings.on("compactChatList");
        let av = if compact { 40.0 } else { 52.0 };
        let cols = pin_columns(inner, pinned.len());
        let cell = inner / cols as f32;
        // The height of a row of the grid: the padding, the avatar and the title of a pin.
        let pin_h = if compact { 8.0 + 4.0 + 6.0 } else { 12.0 + 8.0 + 10.0 } + av + rem(15.0);
        self.pin_grid.set((cols, cell, pin_h));
        let now = now_unix_millis();
        let launched = now - self.started.elapsed().as_millis() as i64;
        let held = self.pin_drag.as_ref().filter(|d| d.order.is_some());
        PinGrid { pinned, items, dragged, held, ghost: self.pin_ghost(), cols, cell, tile: cell.min(120.0), av, now, launched }
    }

    /// A row of the grid of pins.
    fn view_pin_row(&self, l: &RowLook, g: &PinGrid, chunk: &[&Chat]) -> El<'_> {
        El::from(chunk.iter().fold(Row::new(), |r, c| r.push(container(self.view_pin(l, g, c)).center_x(g.cell))))
    }

    /// A pin: the chat's avatar with its unread badge or the bubble of its last message, over its title.
    fn view_pin(&self, l: &RowLook, g: &PinGrid, c: &Chat) -> El<'_> {
        let (p, st, tile, pin_av, window) = (&self.pal, l.st, g.tile, g.av, self.drawing.get().or(self.main_window));
        let on = l.selected == Some(c.id.as_str());
        let (muted, quiet) = (self.list_muted(st, c), self.list_quiet(st, c));
        let unread = c.is_unread();
        let loud = p.badge.map_or((p.accent, Color::WHITE), |from| (Color::WHITE, from));
        let (bg, fg) = if quiet { (p.card, p.strong) } else { loud };
        let dot = (!l.counts && unread).then(|| container(Space::new()).width(6).height(6).style(fill_box(bg, 3.0)));
        // The padding of the pin, then the bell and the dot with the heading's 0.2rem gaps.
        let title_w = tile - if l.compact { 20.0 } else { 24.0 } - if muted { 15.2 } else { 0.0 } - if dot.is_some() { 9.2 } else { 0.0 };
        let title = ell(short_title(c), title_w, 12.0, Weight::Semibold);
        let label = row![]
            .push(dot)
            .push(title.color(p.strong).line_height(iced::widget::text::LineHeight::Absolute(rem(15.0).into())))
            .push(muted.then(|| tinted(&BELL_OFF, 12.0, p.strong)))
            .spacing(3.2)
            .align_y(Center);
        let unread = match (st.typing.contains_key(&c.id), l.counts) {
            (true, _) => Some(self.typing_badge()),
            (false, true) => unread.then(|| count_badge(c, bg, fg, 14.0)),
            (false, false) => {
                (c.unread_mentions > 0).then(|| container(ic(icon::AT_SIGN, 10.0).color(fg)).center(16).style(fill_box(bg, 8.0)).into())
            }
        };
        // Desktop's 14 px indicators center on the line of the bubbles, 5 px below the avatar's top (2 px in the
        // compact list), and its 16 px `mentionIndicator` 2 px lower.
        let mention = !l.counts && !st.typing.contains_key(&c.id);
        let lift = if l.compact { 5.0 } else { 2.0 } - if mention { 1.0 } else { 0.0 };
        let bubble = bubble_text(c, g.launched, g.now).filter(|_| l.previews && !on && !l.selecting);
        let unread = unread.filter(|_| bubble.is_none());
        let face = stack![container(self.chat_avatar(c, pin_av, 1.25)).center_x(pin_av + 4.0)]
            .push((!l.selecting).then(|| pin(container(unread).width(pin_av + 4.0).align_right(pin_av + 4.0)).y(-lift)))
            .push(l.selecting.then(|| pin(checkbox(p, self.selection.contains(&c.id))).x(pin_av - 11.0).y(-5.0)))
            .width(pin_av + 4.0)
            .height(pin_av);
        let pad = if l.compact { Padding::new(10.0).top(8.0).bottom(6.0) } else { Padding::new(12.0).bottom(10.0) };
        let item = column![face, label].spacing(if l.compact { 4 } else { 8 }).align_x(Center).width(tile).padding(pad);
        let colors = if muted { (p.card, p.strong) } else { loud };
        let bubble = bubble.map(|t| pin_bubble(&t, c.unread_mentions > 0, colors, tile, pin_av, l.compact));
        let item = stack![item].push(bubble);
        if let Some(d) = g.held.filter(|d| d.id == c.id) {
            return self.view_held_pin(g, item.into(), d, on);
        }
        // Desktop's `useDropPlaceholder`: the dragged pin marks the place it would drop at.
        let held = g.dragged == Some(c.id.as_str());
        let item = button(item)
            .padding(0)
            .style(btn_style([if on || held { p.selected } else { CLEAR }; 3], [p.text; 3], 8.0))
            .on_press(l.pick(c));
        let item = mouse_area(item)
            .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(c.id.clone())))
            .on_enter(UiMessage::Pins(pins::Msg::Hover(Some(c.id.clone()))))
            .on_exit(UiMessage::Pins(pins::Msg::Hover(None)));
        // Desktop's pins are `dnd-kit` sortables, which show Chromium's ring, and whose Enter and Space pick the pin
        // up for the arrows (see `sortable`).
        let pick = Self::pick(window, sortable::List::Pins, &c.id, &g.items, g.cols);
        let item = self.target(window, format!("pin-{}", c.id), item, Keys::press(pick), Look::Auto);
        match self.pin_offset(&c.id) {
            Some(by) => iced::widget::float(item).translate(move |_, _| by).into(),
            None => item,
        }
    }

    /// The pin that the mouse drags: it follows the pointer at `opacity: 0.4`, inside the list, over the
    /// `dropPlaceholder` of the cell that it would drop in, unless that is its own cell.
    fn view_held_pin<'a>(&self, g: &PinGrid, item: El<'a>, d: &PinDrag, on: bool) -> El<'a> {
        let p = &self.pal;
        let face = container(item).width(g.tile).style(fill_box(if on { p.selected } else { CLEAR }, 8.0));
        let veil = container(Space::new())
            .width(Fill)
            .height(Fill)
            .style(fill_box(p.backdrop.map_or(p.window, |b| b.middle()).scale_alpha(0.6), 8.0));
        let by = g.ghost.unwrap_or(Vector::ZERO);
        let face = iced::widget::float(stack![face, veil]).translate(move |b, view| {
            let x = (b.x + by.x).clamp(view.x, (view.x + view.width - b.width).max(view.x));
            let y = (b.y + by.y).clamp(view.y, (view.y + view.height - b.height).max(view.y));
            Vector::new(x - b.x, y - b.y)
        });
        let to = g.pinned.iter().position(|o| o.id == d.id);
        let mark = (to != Some(d.start))
            .then(|| container(Space::new()).width(Fill).height(Fill).style(fill_box(Color::BLACK.scale_alpha(0.1), 8.0)));
        stack![face].push(mark).into()
    }

    /// A row of the chat list, a chat's or a new-chat draft's, with its hover action, divider and menu.
    fn view_list_row<'a>(
        &'a self, l: &RowLook, (key, on): (String, bool), dot: Option<El<'a>>, face: El<'a>, [top, preview]: [El<'a>; 2], select: UiMessage,
    ) -> El<'a> {
        let (p, st) = (&self.pal, l.st);
        let item = row![]
            .push(dot)
            .push(container(face).width(l.av + 10.0))
            // Desktop's 10 px padding, but iced draws a 14 px title 1 px lower than Chromium, so the regular row moves
            // its title up and keeps the preview in place.
            .push(column![top, preview].spacing(if l.compact { 0 } else { 1 }).width(Fill))
            .align_y(Center)
            .padding(Padding::new(8.0).top(if l.compact { 10.0 } else { 9.0 }).bottom(10.0).left(if l.selecting { 0.0 } else { 8.0 }));
        // Desktop's Play button takes the place of the hover actions.
        let plays = st.resolve_chat_idx(&key).is_some_and(|i| self.plays(&st.chats[i]));
        let hover = !l.selecting && !plays;
        let action = self.hover_progress(&key).filter(|_| hover).map(|t| match key.strip_prefix("draft:") {
            Some(n) => {
                Some(self.hover_btn(5, 20.0, tr("Delete Chat"), vec![], UiMessage::NewChat(new_chat::Msg::Delete(n.parse().unwrap())), t))
            }
            None => {
                let c = &st.chats[st.resolve_chat_idx(&key).expect("listed chat")];
                self.row_action(&st.settings, c, l.hover_action, t)
            }
        });
        let rule = (!on && l.above.as_deref() != Some(key.as_str())).then(|| container(divider(p.divider, l.av + 18.0)).align_bottom(Fill));
        let row = mouse_area(
            button(stack![item].push(action.flatten()).push(rule))
                .padding(0)
                .width(Fill)
                .style(btn_style([if on { p.selected } else { CLEAR }; 3], [p.text; 3], 8.0))
                .on_press(select),
        )
        .on_enter(UiMessage::ChatList(chat_list::Msg::Hover(key.clone(), true)))
        .on_exit(UiMessage::ChatList(chat_list::Msg::Hover(key.clone(), false)));
        let inbox = st.resolve_chat_idx(&key).is_some_and(|i| st.chats[i].flags & CHAT_ARCHIVED == 0);
        let row = match inbox {
            true => row.on_middle_press(UiMessage::ChatList(chat_list::Msg::ToggleFlag(key.clone(), ChatFlag::Archived))),
            false => row,
        };
        row.on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(key))).into()
    }

    /// The row of a new-chat draft: the names of its people, or "New Chat".
    fn view_draft_row(&self, l: &RowLook, d: &NewChat) -> El<'_> {
        let (p, n) = (&self.pal, d.id);
        let dot = (!l.counts).then(|| container(Space::new()).width(12).into());
        let names: Vec<&str> = d.users.iter().map(|(_, u)| u.full_name.as_deref().unwrap_or(&u.id)).collect();
        let title = if names.is_empty() { tr("New Chat").into() } else { names.join(", ") };
        let title = ell(&title, l.text_w, l.title_size, if l.counts { Weight::Medium } else { Weight::Semibold });
        let rows = [container(title.color(p.strong)).height(rem(16.0)).into(), Space::new().height(rem(16.0) * l.lines as f32).into()];
        let face = self.face(d.users.first().and_then(|(_, u)| u.img_url.as_deref()), names.first().unwrap_or(&""), l.av, 1.25);
        let on = self.creating_chat == Some(n);
        let picked = self.new_chat_account.as_ref().filter(|_| on);
        let face = match d.users.first().map(|(a, _)| a).or(picked).filter(|_| l.network_icons) {
            Some(account) => self.list_badged(l, face, brand_key(account), on),
            None => face,
        };
        self.view_list_row(l, (format!("draft:{n}"), on), dot, face, rows, UiMessage::NewChat(new_chat::Msg::Select(n)))
    }

    /// `face` with the network icon of `brand` at its corner, ringed in the color of the row.
    fn list_badged<'a>(&self, l: &RowLook, face: El<'a>, brand: &str, on: bool) -> El<'a> {
        let p = &self.pal;
        // Over a theme color's backdrop the cutout shows the middle of its gradient.
        let under = p.backdrop.map_or(p.window, |b| b.middle());
        let ring = if on { crate::style::flat(under, p.selected) } else { under };
        badged(face, brand, l.av, if l.compact { 12.0 } else { 16.0 }, (!l.colorize).then_some(p), ring)
    }

    /// The draft that the row of a chat shows: Desktop's `rce` keeps that of the open chat as it was when it opened.
    pub(crate) fn listed_draft(&self, c: &Chat) -> Option<String> {
        let open = self.pane().selected_chat_id.as_ref() == Some(&c.id);
        draft_line(c).and_then(|t| if open { self.pane().listed_draft.clone() } else { Some(t) })
    }

    /// The row of a chat: its avatar, its title line, and the preview of its last message or draft.
    fn view_chat_row(&self, l: &RowLook, c: &Chat) -> El<'_> {
        let (p, st) = (&self.pal, l.st);
        let on = l.selected == Some(c.id.as_str());
        let unread = c.is_unread();
        let draft = self.listed_draft(c);
        let top = self.view_chat_top(l, c, draft.is_some());
        // A dark rich theme's `unreadCount` is white with the text in `--color-from`.
        let (dot_bg, dot_fg) = match (self.list_quiet(st, c), p.badge) {
            (true, _) => (Color { a: 0.3, ..p.strong }, Color::WHITE),
            (false, Some(from)) => (Color::WHITE, from),
            (false, None) => (p.accent, Color::WHITE),
        };
        // Desktop's `proIndicators` fade and shrink away in `.1s ease-in-out` while a hover action grows in their place.
        let acts = || self.row_action(&st.settings, c, l.hover_action, 1.0).is_some();
        let keep = 1.0 - self.hover_progress(&c.id).filter(|_| acts()).unwrap_or(0.0);
        let count = match l.counts {
            // `.unreadCount` is `1lh` of 14 px, or 12 px in the compact list.
            true => unread.then(|| count_badge(c, dot_bg.scale_alpha(keep), dot_fg.scale_alpha(keep), if l.compact { 12.0 } else { 14.0 })),
            false => (c.unread_mentions > 0).then(|| ic(icon::AT_SIGN, 12.0).color(p.accent.scale_alpha(keep)).into()),
        };
        // Desktop's `indicators`: the unread dot, and a red dot while a send of the chat waits for Retry.
        let dot = (!l.counts).then(|| {
            let dot = |bg| container(Space::new()).width(6).height(6).style(fill_box(bg, 3.0));
            let errored = st.has_errored(&c.id).then(|| dot(hex(0xf44336)));
            container(column![].push(unread.then(|| dot(dot_bg))).push(errored).spacing(4)).width(12)
        });
        let count = count.map(|b| crate::lift::shrink(b, keep).into());
        let count = if self.plays(c) { Some(self.view_once_play(&c.id)) } else { count };
        let preview = self.view_chat_preview(l, c, draft, count);
        let avatar = self.chat_avatar(c, l.av, 1.25);
        let avatar = if l.network_icons { self.list_badged(l, avatar, merged::chat_brand(c), on) } else { avatar };
        let face = stack![avatar].push(st.typing.contains_key(&c.id).then(|| pin(self.typing_badge()).x(l.av - 14.0).y(-2.0)));
        let dot = match l.selecting {
            true => Some(container(checkbox(p, self.selection.contains(&c.id))).padding(Padding::ZERO.right(8.0)).into()),
            false => dot.map(El::from),
        };
        let lines = [top, preview.height(rem(16.0) * l.lines as f32).into()];
        self.view_list_row(l, (c.id.clone(), on), dot, face.into(), lines, l.pick(c))
    }

    /// The title line of a chat row: the pin, title and mute icons, and the time of its last activity, reminder,
    /// scheduled send or snooze.
    fn view_chat_top(&self, l: &RowLook, c: &Chat, drafted: bool) -> El<'_> {
        let (p, st) = (&self.pal, l.st);
        let muted = self.list_muted(st, c);
        let next = next_send(st, &c.id);
        // Desktop's `quietReason` "snoozed", and the time of the Reminders, Send Later or Snoozed row.
        let snoozed = reminders::snoozed_until(st, c);
        let remind = c.reminder.as_ref().map(|r| r.remind_at_ms).into_iter().chain(self.message_reminder_at(st, c));
        let ts = match (self.list().active_tab, remind.min(), next, snoozed) {
            (InboxTab::Reminders, Some(at), _, _) | (InboxTab::SendLater, _, Some(at), _) | (InboxTab::Snoozed, _, _, Some(at)) => {
                format!("{} {}", reminder_day(at), clock(at, l.h24))
            }
            _ if drafted => String::new(),
            _ => c.last_activity.as_deref().map(|t| list_ts(t, l.h24)).unwrap_or_default(),
        };
        let later = next.is_some() && (!drafted || matches!(self.list().active_tab, InboxTab::Reminders | InboxTab::SendLater));
        let ts_w = crate::style::text_width(&ts, l.ts_size, Weight::Normal) + 8.0 + if later { 12.0 } else { 0.0 };
        let reminded_last = c
            .reminder
            .as_ref()
            .zip(c.last_activity.as_deref())
            .is_some_and(|(r, t)| millis_from_rfc3339(t).expect("RFC 3339 timestamp") <= r.remind_at_ms);
        let style = match self.expired(c) && reminded_last {
            true => iced::font::Style::Italic,
            false => iced::font::Style::Normal,
        };
        let title = c.network_name.as_ref().map_or_else(|| c.title.clone(), |n| format!("{} • {n}", c.title));
        let weight = if l.counts && !c.is_unread() { Weight::Medium } else { Weight::Semibold };
        let bell = if muted { 16.0 } else { 0.0 } + if snoozed.is_some() { 16.0 } else { 0.0 };
        let pinned = c.flag(ChatFlag::Pinned);
        let pin_icon = pinned.then(|| container(tinted(&PIN, 13.0, p.strong)).padding(Padding::ZERO.right(2.0)));
        let pin_w = if pinned { 15.0 } else { 0.0 };
        row![]
            .push(pin_icon)
            .push(ell(&title, l.text_w - ts_w - bell - pin_w, l.title_size, weight).color(p.fg.scale_alpha(0.9)))
            .push(muted.then(|| container(tinted(&BELL_OFF, 12.0, p.strong)).padding([0, 2])))
            .push(snoozed.map(|_| container(tinted(&MOON_STARS, 12.0, p.strong)).padding([0, 2])))
            .push(Space::new().width(Fill))
            .push(later.then(|| tinted(&COMPOSER_ICONS[6], 12.0, p.strong.scale_alpha(0.7))))
            .push(
                container(
                    txt(ts, l.ts_size)
                        .color(p.strong.scale_alpha(0.7))
                        .font(crate::style::redact(iced::Font { style, ..fw(Weight::Normal) })),
                )
                .height(Fill)
                .padding(Padding::ZERO.top(2.0)),
            )
            .height(rem(16.0))
            .align_y(Center)
            .into()
    }

    /// The text of a chat row's preview, whether it is italic, and the first name of its sender when the row names
    /// them: the count of the new messages of unopened view-once media, the last message, its media label, or the
    /// invitation of a request.
    fn preview_text(&self, c: &Chat, m: Option<&Message>, tag: Option<&MediaTag>) -> (Option<String>, bool, Option<String>) {
        let mine = m.is_some_and(|m| m.is_sender == Some(true));
        let reaction = m.filter(|m| m.message_type.as_deref() == Some("REACTION"));
        // Desktop counts the new messages of a chat whose view-once media nobody opened.
        let fresh = (self.plays(c) && c.unread_count > 0 && c.preview.as_ref().is_some_and(|m| self.view_once.unopened(m))).then(|| {
            let n = c.unread_count as usize;
            trnx("%d New message", "%d New messages", n, "view-limited media chat list preview", &[&n])
        });
        let (body, italic) = match (fresh.clone(), m.and_then(preview_body)) {
            (Some(t), _) | (None, Some(t)) => (Some(t), false),
            (None, None) => tag.and_then(|t| t.2.clone()).map_or((None, false), |(d, i)| (Some(d), i)),
        };
        let show_sender = fresh.is_none()
            && !mine
            && reaction.is_none()
            && !m.is_some_and(Message::silent)
            && (tag.is_some() || body.is_some())
            && (c.chat_type == "group" || m.is_some_and(|m| network_bot(c, &m.sender_id)));
        let first = m.filter(|_| show_sender).map(|m| match c.participants.items.iter().find(|u| u.id == m.sender_id) {
            Some(u) => sender_first_name(u),
            None => m.sender_name.as_deref().and_then(|n| n.split(' ').next()).unwrap_or(&m.sender_id).into(),
        });
        let body = body.map(|t| match (tag.is_none(), &first) {
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
        (body, italic, first)
    }

    /// The preview line of a chat row: the Reminder badge, the receipt, the sender's face, the media label and the
    /// text of the last message or of the draft, then the unread count.
    fn view_chat_preview<'a>(&'a self, l: &RowLook, c: &Chat, draft: Option<String>, count: Option<El<'a>>) -> Column<'a, UiMessage> {
        let (p, st, lines, text_w) = (&self.pal, l.st, l.lines, l.text_w);
        let m = c.preview.as_ref().filter(|_| l.previews && draft.is_none());
        let expired = self.expired(c);
        let media_tag = m.and_then(message_tag);
        let (body, italic, first) = self.preview_text(c, m, media_tag.as_ref());
        let sender = first.is_some().then(|| preview_sender(c, m?)).flatten();
        let digits = if l.counts && c.unread_count > 0 { unread_label(c.unread_count as usize).len() as f32 } else { 0.0 };
        let at = if c.unread_mentions > 0 { 12.0 } else { 0.0 };
        let prefix = first.filter(|_| media_tag.is_some()).map(|s| format!("{s}:"));
        let prefix_w = prefix.as_deref().map_or(0.0, |s| crate::style::text_width(s, 13.0, Weight::Normal) + 4.0);
        let base = 16.0 + if l.counts { 8.0 + 6.0 * digits + at } else { at }.max(18.0);
        let receipt = m.is_some_and(read_icon);
        let indent = match (receipt, m.is_some_and(|m| m.send_error.is_some())) {
            (true, true) => 21.0,
            (true, false) => 16.0,
            (false, _) => 0.0,
        } + if sender.is_some() { 16.0 } else { 0.0 }
            + match &media_tag {
                Some((_, label, _)) if expired => crate::style::text_width(label, 13.0, Weight::Normal) + 4.0,
                Some((_, label, _)) => crate::style::text_width(label, 10.0, Weight::Medium) + 24.0,
                None => 0.0,
            }
            + if expired { crate::style::text_width(tr("Reminder"), 11.0, Weight::Semibold) + 10.0 } else { 0.0 }
            + prefix_w;
        let width = text_w - (base + indent);
        let flowed = lines > 1 && indent > 0.0;
        let (body, second) = match body {
            Some(b) if flowed => {
                let (first, rest) = crate::style::flow(&b, indent, text_w - base, lines, 13.0, Weight::Normal);
                (Some(first), Some(rest).filter(|r| !r.is_empty()))
            }
            b => (b, None),
        };
        let style = if italic { iced::font::Style::Italic } else { iced::font::Style::Normal };
        let font = crate::style::redact(iced::Font { style, ..fw(Weight::Normal) });
        let fg = p.fg.scale_alpha(0.63);
        let receipt = m.filter(|_| receipt).map(|m| self.preview_receipt(st, c, m, fg));
        let face = sender.map(|u| self.face(crate::user_img(u).as_deref(), u.full_name.as_deref().unwrap_or_default(), 12.0, 0.5));
        let badge = (expired && l.previews).then(|| {
            let label = semi(tr("Reminder"), 11.0).color(Color::WHITE.scale_alpha(0.7));
            container(label).padding([0, 3]).style(fill_box(ORANGE.scale_alpha(0.7), 4.0))
        });
        let preview = row![]
            .push(badge)
            .push(receipt)
            .push(face)
            .push(prefix.map(|s| txt(s, 13.0).color(fg)))
            .push(media_tag.map(|(icon, label, _)| media_chip(p, icon, label, expired, fg)))
            .push(body.map(|b| preview_line(&b, width, if flowed { 1 } else { lines }, font, fg)))
            .push(draft.filter(|_| l.previews).map(|t| {
                let label = trx("Draft:", "thread list draft preview prefix");
                let s = crate::style::clamped(&format!("{label} {t}"), width, lines, 13.0, Weight::Normal);
                let rest = s.strip_prefix(label).expect("the draft label fits").to_string();
                rich_text![
                    span::<(), _>(label).font(crate::style::redact(fw(Weight::Semibold))).color(p.accent),
                    span(rest).font(crate::style::redact(fw(Weight::Normal))).color(fg)
                ]
                .size(rem(13.0))
                .line_height(lh(16.0))
                .width(width)
            }))
            .push(Space::new().width(Fill))
            // `.actions` has a 2 px top margin.
            .push(count.map(|c| container(c).padding(Padding::ZERO.top(if l.compact { 0.0 } else { 2.0 }))))
            .spacing(4)
            .align_y(if l.compact { Center } else { iced::Alignment::Start });
        column![preview].push(second.map(|r| txt(r.to_string(), 13.0).font(font).color(fg)))
    }

    /// The receipt of a chat's last message when it is the user's: sending, sent, read, or failed.
    fn preview_receipt<'a>(&self, st: &BeeperStore, c: &Chat, m: &Message, fg: Color) -> El<'a> {
        // Desktop's `ZGe`: a message that failed to send shows `ErroredIcon`, without a tooltip.
        if m.send_error.is_some() {
            // `.messageErroredIcon`: 16 by 11 with a 1 px margin, where the read icons are 12 px wide.
            let icon = container(tinted(&ERRORED, 11.0, ERROR_COLOR)).center_x(16);
            return container(icon).padding(Padding::ZERO.right(1.0)).into();
        }
        let glyph = if pending(st, m) { &PENDING } else { &SENT[usize::from(seen_by(st, m) > 0)] };
        let icon = tinted(glyph, 12.0, fg);
        match receipt_tip(st, c, m) {
            Some(tip) => self.tip(icon, tip, vec![], tooltip::Position::Top).into(),
            None => icon,
        }
    }
}

/// The count of the pinned chats at the top of the rows of a list without a grid of pins (`regularPinned`).
fn pinned_rows(rest: &[&Chat]) -> usize {
    rest.iter().take_while(|c| c.flags & CHAT_PINNED != 0).count()
}

/// The media label of a chat row's preview: the icon and label of the attachment, or only its label in italic text
/// while the chat's reminder is due.
fn media_chip<'a>(p: &Palette, icon: usize, label: &'a str, expired: bool, fg: Color) -> El<'a> {
    if expired {
        return txt(label, 13.0).color(fg).into();
    }
    let glyph = tinted(tag_glyph(icon), 12.0, fg);
    let label = med(label, 10.0).color(fg);
    container(row![glyph, container(label).padding([0, 2])].align_y(Center))
        .padding([1, 2])
        .style(fill_box(p.strong.scale_alpha(0.105), 4.0))
        .into()
}

/// The text of a chat row's preview in `lines` lines of `width`, with the emoji font when it has emoji.
fn preview_line<'a>(b: &str, width: f32, lines: usize, font: iced::Font, fg: Color) -> El<'a> {
    let shown = crate::style::clamped(b, width, lines, 13.0, Weight::Normal);
    if !shown.contains('\u{fe0f}') {
        let text = clamp(b, width, lines, 13.0, Weight::Normal);
        return text.width(width).line_height(lh(16.0)).font(font).color(fg).into();
    }
    let spans = crate::emoji::with_emoji_font(vec![span::<(), _>(shown.to_string()).font(font)]);
    let wrap = if lines == 1 { iced::widget::text::Wrapping::None } else { iced::widget::text::Wrapping::WordOrGlyph };
    rich_text(spans).size(rem(13.0)).line_height(lh(16.0)).color(fg).width(width).wrapping(wrap).into()
}

/// The settings, sizes and selection that every row of the chat list reads.
struct RowLook<'s> {
    st: &'s BeeperStore,
    /// Desktop's `isSelected` row.
    selected: Option<&'s str>,
    /// The row above the selected one.
    above: Option<Cow<'s, str>>,
    compact: bool,
    av: f32,
    lines: usize,
    title_size: f32,
    ts_size: f32,
    /// The width of the title and preview lines.
    text_w: f32,
    counts: bool,
    network_icons: bool,
    colorize: bool,
    hover_action: u8,
    selecting: bool,
    previews: bool,
    h24: bool,
}

impl RowLook<'_> {
    /// The message of a click on the row of `c`: it opens the chat, or picks it while chats are selected.
    fn pick(&self, c: &Chat) -> UiMessage {
        match self.selecting {
            true => UiMessage::ChatList(chat_list::Msg::ToggleSelected(c.id.clone())),
            false => UiMessage::ChatList(chat_list::Msg::Click(c.id.clone())),
        }
    }
}

/// The grid of pins at the top of the chat list.
struct PinGrid<'s> {
    pinned: Vec<&'s Chat>,
    items: std::sync::Arc<[String]>,
    /// The pin that a drag of the mouse or the keyboard moves.
    dragged: Option<&'s str>,
    /// The drag of the mouse, and where its ghost is from the pin's cell.
    held: Option<&'s PinDrag>,
    ghost: Option<Vector>,
    cols: usize,
    cell: f32,
    tile: f32,
    av: f32,
    now: i64,
    launched: i64,
}

/// Desktop's `ope`: whether the bridge of an account or network space takes message requests (the
/// `message_request` of its `defaultFeatures`). Other spaces do.
pub(crate) fn takes_requests(st: &BeeperStore, id: &str) -> bool {
    if rail_label(st, id).is_some() || RAIL_VIEWS.iter().any(|v| v.0 == id) {
        return true;
    }
    let network = beeper_core::config::find_network(&rail_owner(st, id));
    network.is_some_and(|n| matches!(n.kind, NetworkKind::Beeper | NetworkKind::Signal | NetworkKind::TwitterX))
}

/// The width of the chat menu: Desktop's 233 px, or more when a label and its hotkey in the app language need it,
/// like the native menus of Electron, which fit their items. Each row has 13 px of padding on each side, and the
/// hotkey keeps 16 px from the label.
pub(crate) fn chat_menu_w(st: &BeeperStore) -> f32 {
    let key = |name| hotkey_text(st.settings.hotkey_keys(name));
    let action = |label| trx(label, "thread context menu action");
    let rows = [
        (action("Unarchive"), key("Chat: Toggle Archive")),
        (action("Archive"), key("Chat: Toggle Archive")),
        (action("Unmute"), key("Chat: Toggle Mute")),
        (action("Mute"), key("Chat: Toggle Mute")),
        (tr("Mark as Unread"), key("Chat: Toggle Mark As Read")),
        (tr("Mark as Read"), key("Chat: Toggle Mark As Read")),
        (action("Unpin"), key("Chat: Toggle Pin")),
        (action("Pin"), key("Chat: Toggle Pin")),
        (tr("Search…"), key("Search: Room")),
        (tr("Report a Problem"), key("Report a Problem")),
        (tr("Copy"), "›".into()),
        (tr("Remind Me"), "›".into()),
        (trx("Label", "thread context menu submenu label"), "›".into()),
        (trx("Snooze", "thread context menu submenu label"), "›".into()),
    ];
    let plain = [
        "Mark All as Read",
        "Mark All as Unread",
        "Stop Following All Threads",
        "Follow All Threads",
        "Edit Group...",
        "Move to Inbox",
        "Move to Low Priority",
        "Delete Chat",
        "Open in New Window",
        "Begin Selecting",
        "Accept group invitation",
        "Accept message request",
        "Decline group invitation",
        "Decline message request",
    ];
    let width = |label: &str, key: &str| {
        let key = if key.is_empty() { 0.0 } else { 16.0 + text_width(key, 13.0, Weight::Normal) };
        26.0 + text_width(label, 13.0, Weight::Normal) + key
    };
    let rows = rows.iter().map(|(label, key)| width(label, key));
    rows.chain(plain.iter().map(|label| width(tr(label), ""))).fold(233.0, f32::max).ceil()
}

/// An item of the chat menu.
fn chat_item<'a>(p: &Palette, label: &str, key: &str, msg: Option<UiMessage>) -> El<'a> {
    menu_item(p, None, label.into(), key.to_string(), msg)
}

/// The message that turns flag `f` of the chat on or off.
fn flag_msg(c: &Chat, f: ChatFlag) -> Option<UiMessage> {
    Some(UiMessage::ChatList(chat_list::Msg::ToggleFlag(c.id.clone(), f)))
}

/// The message that sets flag `f` of the selected chats.
fn set_msg(f: ChatFlag, on: bool) -> Option<UiMessage> {
    Some(UiMessage::ChatList(chat_list::Msg::SelectionSet(f, on)))
}

/// The chat of a chat menu, and the selected chats, which the menu acts on when there are some.
struct MenuChats<'s> {
    st: &'s BeeperStore,
    c: &'s Chat,
    picked: Vec<&'s Chat>,
}

impl MenuChats<'_> {
    /// Desktop's `l`: the selected chats, or the chat of the menu.
    fn chats(&self) -> Vec<&Chat> {
        if self.picked.is_empty() {
            vec![self.c]
        } else {
            self.picked.clone()
        }
    }

    fn targets(&self) -> Vec<String> {
        self.chats().iter().map(|c| c.id.clone()).collect()
    }

    fn every(&self, f: ChatFlag) -> bool {
        self.picked.iter().all(|c| has_flag(c, f))
    }

    /// The keys of hotkey `name`.
    fn key(&self, name: &str) -> String {
        hotkey_text(self.st.settings.hotkey_keys(name))
    }

    /// Why the chat is muted when something other than its own mute mutes it, without a selection.
    fn reason(&self) -> Option<String> {
        let (st, c) = (self.st, self.c);
        st.notifications_muted(c).then(|| muted_by(st, c)).flatten().filter(|_| self.picked.is_empty())
    }

    /// Archive or Unarchive, which a pinned chat does not take.
    fn archive_item<'a>(&self, p: &Palette) -> El<'a> {
        let c = self.c;
        let archived = c.flag(ChatFlag::Archived);
        let can_archive = archived || !c.flag(ChatFlag::Pinned);
        let key = self.key("Chat: Toggle Archive");
        match self.picked.len() {
            0 => chat_item(
                p,
                match archived {
                    true => trx("Unarchive", "thread context menu action"),
                    false => trx("Archive", "thread context menu action"),
                },
                &key,
                flag_msg(c, ChatFlag::Archived).filter(|_| can_archive),
            ),
            _ => chat_item(
                p,
                match self.every(ChatFlag::Archived) {
                    true => trx("Unarchive", "button label to unarchive selected chats"),
                    false => trx("Archive", "button label to archive selected chats"),
                },
                &key,
                set_msg(ChatFlag::Archived, !self.every(ChatFlag::Archived)).filter(|_| can_archive),
            ),
        }
    }

    /// Mute or Unmute. A chat that something else mutes (`reason`) has no Unmute.
    fn mute_item<'a>(&self, p: &Palette, reason: bool) -> El<'a> {
        let c = self.c;
        let key = self.key("Chat: Toggle Mute");
        let (mute, unmute) = (trx("Mute", "thread context menu action"), trx("Unmute", "thread context menu action"));
        match (self.picked.len(), self.st.notifications_muted(c)) {
            (0, false) => chat_item(p, mute, &key, flag_msg(c, ChatFlag::Muted)),
            (0, true) => chat_item(p, unmute, &key, flag_msg(c, ChatFlag::Muted).filter(|_| c.flag(ChatFlag::Muted) && !reason)),
            _ => {
                let all = self.every(ChatFlag::Muted);
                chat_item(p, if all { unmute } else { mute }, &key, set_msg(ChatFlag::Muted, !all))
            }
        }
    }

    /// Mark as Read or Unread, or Mark All as Read and Unread for a selection of several chats.
    fn read_items<'a>(&self, p: &Palette) -> Vec<El<'a>> {
        let key = self.key("Chat: Toggle Mark As Read");
        let marked = |f: &dyn Fn(&&Chat) -> bool| self.picked.iter().any(f);
        match self.picked.len() {
            0 => {
                let label = tr(if self.c.is_unread() { "Mark as Read" } else { "Mark as Unread" });
                vec![chat_item(p, label, &key, Some(UiMessage::ChatList(chat_list::Msg::ToggleRead(self.c.id.clone()))))]
            }
            1 => vec![match self.every(ChatFlag::MarkedUnread) {
                true => chat_item(p, tr("Mark as Read"), &key, set_msg(ChatFlag::MarkedUnread, false)),
                false => chat_item(p, tr("Mark as Unread"), &key, set_msg(ChatFlag::MarkedUnread, true)),
            }],
            _ => [
                marked(&|c| has_flag(c, ChatFlag::MarkedUnread))
                    .then(|| chat_item(p, tr("Mark All as Read"), "", set_msg(ChatFlag::MarkedUnread, false))),
                marked(&|c| !has_flag(c, ChatFlag::MarkedUnread))
                    .then(|| chat_item(p, tr("Mark All as Unread"), "", set_msg(ChatFlag::MarkedUnread, true))),
            ]
            .into_iter()
            .flatten()
            .collect(),
        }
    }

    /// The Label submenu: the labels by name, checked when the chat has them, then New Label….
    fn label_items<'a>(&self, p: &Palette) -> (Vec<El<'a>>, f32) {
        let targets = self.targets();
        let mut sorted: Vec<&Label> = self.st.labels.iter().collect();
        sorted.sort_by_cached_key(|l| l.name.to_lowercase());
        let rows = sorted.into_iter().map(|l| {
            let on = self.c.label_ids.contains(&l.id);
            let lead = container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into();
            let msg = UiMessage::Labels(labels::Msg::Toggle(l.id.clone(), targets.clone(), !on));
            menu_item(p, Some(lead), l.name.clone(), "", Some(msg))
        });
        let new = chat_item(p, tr("New Label…"), "", Some(UiMessage::Labels(labels::Msg::OpenCreate(targets.clone()))));
        (rows.chain([menu_rule(p), new]).collect(), 1.0)
    }

    /// The Remind Me submenu: Last Used and the presets with their times, Custom… and Dismiss Reminder.
    fn remind_items<'a>(&self, p: &Palette) -> (Vec<El<'a>>, f32) {
        let (st, c) = (self.st, self.c);
        let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
        // Desktop's `canSetReminderFor`: Beeper Plus, a free use, or a reminder to change.
        let can = st.feature("remind-later").on || c.reminder.is_some();
        let set = |at| can.then(|| UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), Some(at))));
        let last = last_used(st, now).map(|at| chat_item(p, tr("Last Used"), &last_used_label(at, h24), set(at)));
        let presets = remind_presets(&st.settings, now).into_iter().map(|(label, at)| {
            let last = UiMessage::CommandBar(commands::Msg::LastTime(label.clone()));
            let msg = set(at).map(|set| UiMessage::Commands(vec![last, set]));
            chat_item(p, &label, &remind_label(at, h24), msg)
        });
        let upgrade = (!can).then(|| {
            let open = UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus));
            [chat_item(p, tr("Upgrade to Beeper Plus"), "", Some(open)), menu_rule(p)]
        });
        let presets = upgrade.into_iter().flatten().chain(last).chain(presets);
        let dismiss = c.reminder.is_some().then(|| UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), None)));
        let custom = UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(c.id.clone())));
        let custom = chat_item(p, tr("Custom…"), "", can.then_some(custom));
        let dismiss = chat_item(p, tr("Dismiss Reminder"), "", dismiss);
        let rules = if can { 2.0 } else { 3.0 };
        (presets.chain([menu_rule(p), custom, menu_rule(p), dismiss]).collect(), rules)
    }
}

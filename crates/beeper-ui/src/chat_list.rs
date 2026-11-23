//! Desktop's chat list (ThreadList): its tabs and filters, the chat menu and flags, archive and read, multi-select, and the chat actions.

use super::*;
use crate::focus::{Keys, Look};
use crate::i18n::trnx;
use crate::reminders::{last_used, last_used_label, remind_presets};
use crate::time::list_ts;
use crate::vcard::contact_name;
use iced::widget::column;

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
        self.selected_account_filter.clone().unwrap_or_else(|| {
            RAIL_VIEWS.iter().find(|v| v.1 == Some(self.active_tab)).expect("a sidebar view").0.to_string()
        })
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
                        Msg::ArchiveAll(read, days) => {
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
const FILTER_IDS: [&str; 8] =
    ["unread", "drafts", "unresponded", "dm", "group", "archived", "requests", "low_priority"];

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
        _ if ["application/zip", "application/x-zip-compressed", "multipart/x-zip"].contains(&mime.as_str())
            || file.ends_with(".zip") =>
        {
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
    .map(|d| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 -880 800 800"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
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
        let menus = [(tr("Remind Me"), &SELECTION_ICONS[6]), (tr("More"), &SELECTION_ICONS[7])]
            .into_iter()
            .zip(0u8..)
            .map(|((label, glyph), i)| {
                let x = x0 + cell * (4.5 + f32::from(i));
                (label, label, glyph, UiMessage::ChatList(chat_list::Msg::SelectionMenu(i, x)), vec![])
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
                None => self.tip_caps(btn, tip, keys, tooltip::Position::Top).into(),
            }
        });
        let bar = container(Row::with_children(buttons).align_y(Center))
            .center_y(36)
            .width(Fill)
            .style(fill_box(p.card, 18.0));
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
                item(
                    tr("Last Used").into(),
                    last_used_label(at, h24),
                    Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(Some(at)))),
                )
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
                    item(
                        tr("Dismiss Reminder").into(),
                        String::new(),
                        Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(None))),
                    ),
                ],
                n => vec![
                    menu_rule(p),
                    item(
                        format!("Dismiss {n} Reminders"),
                        String::new(),
                        Some(UiMessage::ChatList(chat_list::Msg::SelectionRemind(None))),
                    ),
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
                    some(ChatFlag::Muted)
                        .then(|| entry(trx("Unmute", "thread context menu action"), ChatFlag::Muted, false)),
                    some(ChatFlag::LowPriority).then(|| entry(tr("Move to Inbox"), ChatFlag::LowPriority, false)),
                    (pinned < n).then(|| entry(tr("Pin"), ChatFlag::Pinned, true)),
                    (pinned > 0).then(|| entry(tr("Unpin"), ChatFlag::Pinned, false)),
                ])
                .flatten()
                .collect()
        };
        let rules = if which == 0 && chats.iter().any(|c| c.reminder.is_some()) { 11.0 - menu_pitch() } else { 0.0 };
        let h = 8.0 + items.len() as f32 * menu_pitch() + rules;
        let width = if which == 0 { SUB_MENU_W[1] } else { 233.0 };
        let menu = container(Column::with_children(items).padding([4, 0])).width(width).style(shadow_box(
            p.menu,
            p.menu_border,
            6.0,
            16.0,
        ));
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
        let views = views.iter().map(|&i| {
            item(self.list().filters & 1 << i != 0, FILTERS[i].1, tr(FILTERS[i].0).into(), String::new(), 1 << i)
        });
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
                container((self.list().space_view_account.as_ref() == Some(&id)).then(|| ic(icon::CHECK, 14.0)))
                    .center_x(14),
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
        container(col.padding([4, 0])).width(width).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    /// The chat menu, with `room` pixels from its top to the bottom of the window.
    pub(crate) fn view_chat_menu(&self, cid: &str, sub_left: bool, room: f32) -> El<'_> {
        let p = &self.pal;
        if cid == SIDEBAR_MENU {
            let st = self.store.read();
            let keys = |name| prefs::hotkey_label(st.settings.hotkey_keys(name));
            let bar = if st.settings.on("showSidebar") { "Hide Space Bar" } else { "Show Space Bar" };
            let col = column![
                menu_item(
                    p,
                    None,
                    tr("Start New Chat").into(),
                    keys("Chat: Start New"),
                    Some(UiMessage::NewChat(new_chat::Msg::Open))
                ),
                menu_item(
                    p,
                    None,
                    tr("Add/Remove Accounts").into(),
                    "",
                    Some(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)))
                ),
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
                    "",
                    Some(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)))
                ),
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
        if cid == RAIL_MENU {
            return self.rail_menu(&self.store.read(), None);
        }
        if let Some(entry) = cid.strip_prefix("head:") {
            let (menu, sub) = self.account_menu(entry, SpaceMenu::Header);
            return row![menu].push(sub).into();
        }
        // Desktop's Snooze menu of the Chat Info toolbar.
        if let Some(id) = cid.strip_prefix(SNOOZE_MENU) {
            let st = self.store.read();
            let rows = self.snooze_items(&st, &[id.to_string()]);
            let col = Column::with_children(rows.into_iter().map(|r| r.0)).padding([4, 0]);
            return container(col).width(SUB_MENU_W[3]).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into();
        }
        if let Some(id) = cid.strip_prefix(behavior::BEHAVIOR_MENU) {
            let (rows, w) = self.behavior_items(&self.store.read(), id);
            let col = Column::with_children(rows.into_iter().map(|r| r.0)).padding([4, 0]);
            return container(col).width(w).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into();
        }
        if let Some(n) = cid.strip_prefix("draft:") {
            let delete = menu_item(
                p,
                None,
                tr("Delete Chat").into(),
                "",
                Some(UiMessage::NewChat(new_chat::Msg::Delete(n.parse().unwrap()))),
            );
            return container(column![delete].padding([4, 0]))
                .width(124)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into();
        }
        let st = self.store.read();
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let flag = |f: ChatFlag| Some(UiMessage::ChatList(chat_list::Msg::ToggleFlag(c.id.clone(), f)));
        let archived = c.flag(ChatFlag::Archived);
        let unread = c.is_unread();
        let close_sub = |items: Column<'static, UiMessage>| {
            mouse_area(items).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(None)))
        };
        let key = |name| hotkey_text(st.settings.hotkey_keys(name));
        let item = |label: &str, key: &str, msg| menu_item(p, None, label.into(), key.to_string(), msg);
        let open_sub = |i: u8, label: &str| {
            mouse_area(menu_item(
                p,
                None,
                label.into(),
                "›",
                Some(UiMessage::CommandBar(commands::Msg::ChatSub(Some(i)))),
            ))
            .on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(Some(i))))
        };
        let pick = |on: bool, yes, no| if on { yes } else { no };
        let selecting = self.selection.is_empty();
        let labels = st.feature("labels-v2").on;
        let reason = st.notifications_muted(c).then(|| muted_by(&st, c)).flatten();
        let picked = self.selected_chats(&st);
        let n = picked.len();
        // Desktop's `l`: the selected chats, or the chat of the menu.
        let chats = if n == 0 { vec![c] } else { picked.clone() };
        let targets: Vec<String> = chats.iter().map(|c| c.id.clone()).collect();
        let snooze = reminders::shows_snooze(&st, &chats);
        let reason = reason.filter(|_| n == 0);
        let every = |f: ChatFlag| picked.iter().all(|c| has_flag(c, f));
        let some = |f: ChatFlag| picked.iter().any(|c| has_flag(c, f));
        let set = |f: ChatFlag, on: bool| Some(UiMessage::ChatList(chat_list::Msg::SelectionSet(f, on)));
        let can_archive = archived || !c.flag(ChatFlag::Pinned);
        let archive = match n {
            0 => item(
                pick(
                    archived,
                    trx("Unarchive", "thread context menu action"),
                    trx("Archive", "thread context menu action"),
                ),
                &key("Chat: Toggle Archive"),
                flag(ChatFlag::Archived).filter(|_| can_archive),
            ),
            _ => item(
                match every(ChatFlag::Archived) {
                    true => trx("Unarchive", "button label to unarchive selected chats"),
                    false => trx("Archive", "button label to archive selected chats"),
                },
                &key("Chat: Toggle Archive"),
                set(ChatFlag::Archived, !every(ChatFlag::Archived)).filter(|_| can_archive),
            ),
        };
        let mute = match (n, st.notifications_muted(c)) {
            (0, false) => {
                item(trx("Mute", "thread context menu action"), &key("Chat: Toggle Mute"), flag(ChatFlag::Muted))
            }
            (0, true) => item(
                trx("Unmute", "thread context menu action"),
                &key("Chat: Toggle Mute"),
                flag(ChatFlag::Muted).filter(|_| c.flag(ChatFlag::Muted) && reason.is_none()),
            ),
            _ => item(
                pick(
                    every(ChatFlag::Muted),
                    trx("Unmute", "thread context menu action"),
                    trx("Mute", "thread context menu action"),
                ),
                &key("Chat: Toggle Mute"),
                set(ChatFlag::Muted, !every(ChatFlag::Muted)),
            ),
        };
        let read = |all: bool| match all {
            true => item(tr("Mark as Read"), &key("Chat: Toggle Mark As Read"), set(ChatFlag::MarkedUnread, false)),
            false => item(tr("Mark as Unread"), &key("Chat: Toggle Mark As Read"), set(ChatFlag::MarkedUnread, true)),
        };
        let reads: Vec<El> = match n {
            0 => vec![item(
                tr(pick(unread, "Mark as Read", "Mark as Unread")),
                &key("Chat: Toggle Mark As Read"),
                Some(UiMessage::ChatList(chat_list::Msg::ToggleRead(c.id.clone()))),
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
            item(label, "", Some(UiMessage::ChatList(chat_list::Msg::AcceptRequest(c.id.clone()))))
        });
        let decline = decline.then(|| {
            let label = tr(pick(group, "Decline group invitation", "Decline message request"));
            item(label, "", Some(UiMessage::ChatList(chat_list::Msg::Delete(c.id.clone(), false))))
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
                        &key("Chat: Toggle Pin"),
                        pin
                    ))
                    .push((n <= 1 && st.threaded(c)).then(|| {
                        let on = c.follows.follows_all(c.participants.total);
                        let label = tr(if on { "Stop Following All Threads" } else { "Follow All Threads" });
                        item(label, "", Some(UiMessage::Thread(threads::Msg::FollowAll(c.id.clone(), !on))))
                    }))
                    .push(can_edit_group(c).then(|| {
                        let open = UiMessage::OpenEditGroup(c.id.clone());
                        item(tr("Edit Group..."), "", (c.is_request != Some(true)).then_some(open))
                    }))
            ),
            open_sub(0, tr("Copy")),
            close_sub(column![
                menu_rule(p),
                item(
                    tr("Search…"),
                    &key("Search: Room"),
                    Some(UiMessage::CommandBar(commands::Msg::SearchRoom(c.id.clone())))
                ),
                menu_rule(p),
                low,
            ]),
            labels.then(|| open_sub(2, trx("Label", "thread context menu submenu label"))),
            open_sub(1, tr("Remind Me")),
            snooze.then(|| open_sub(3, trx("Snooze", "thread context menu submenu label"))),
            close_sub({
                // Desktop's Unmerge of one merged chat, in the place of Delete Chat.
                let delete = match n <= 1 && c.merge.is_some() {
                    true => Some(item(
                        trx("Unmerge", "thread context menu action to split a merged chat"),
                        "",
                        Some(UiMessage::Merged(merged::Msg::Ask(c.id.clone()))),
                    )),
                    false => st.feature("delete-chats").on.then(|| {
                        let ask =
                            (c.delete_modes != 0).then(|| UiMessage::ChatList(chat_list::Msg::AskDelete(c.id.clone())));
                        item(tr("Delete Chat"), "", ask)
                    }),
                };
                let block = block_action(&st, c).filter(|_| n == 0).map(|block| {
                    let label = match block {
                        true => trx("Block…", "action to block the other person in a DM"),
                        false => trx("Unblock…", "action to unblock the other person in a DM"),
                    };
                    item(label, "", Some(UiMessage::Blocking(blocking::Msg::Ask(c.id.clone(), block))))
                });
                // Like Electron's menus, a group without items has no separator.
                column![]
                    .push((delete.is_some() || block.is_some()).then(|| menu_rule(p)))
                    .push(delete)
                    .push(block)
                    .push(selecting.then(|| menu_rule(p)))
                    .push(selecting.then(|| {
                        item(
                            tr("Open in New Window"),
                            "",
                            Some(UiMessage::Window(windows::Msg::OpenChat(c.id.clone()))),
                        )
                    }))
                    .push(selecting.then(|| {
                        item(
                            tr("Begin Selecting"),
                            "",
                            Some(UiMessage::ChatList(chat_list::Msg::BeginSelecting(c.id.clone()))),
                        )
                    }))
                    .push(menu_rule(p))
                    .push(item(
                        tr("Report a Problem"),
                        &key("Report a Problem"),
                        Some(UiMessage::OpenUrl(ISSUES_URL.into())),
                    ))
            }),
        ];
        let menu =
            container(col.padding([4, 0])).width(chat_menu_w(&st)).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let sub = self.chat_sub.map(|i| {
            let item = |label: &str, at: String, msg| menu_item(p, None, label.into(), at, msg);
            let (items, rules): (Vec<El>, f32) = match i {
                0 => (
                    vec![item(
                        tr("Chat Link"),
                        String::new(),
                        Some(UiMessage::ChatList(chat_list::Msg::CopyLink(c.id.clone()))),
                    )],
                    0.0,
                ),
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
                        let msg = UiMessage::Labels(labels::Msg::Toggle(l.id.clone(), targets.clone(), !on));
                        menu_item(p, Some(lead), l.name.clone(), "", Some(msg))
                    });
                    let new = item(
                        tr("New Label…"),
                        String::new(),
                        Some(UiMessage::Labels(labels::Msg::OpenCreate(targets.clone()))),
                    );
                    (rows.chain([menu_rule(p), new]).collect(), 1.0)
                }
                1 => {
                    let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
                    // Desktop's `canSetReminderFor`: Beeper Plus, a free use, or a reminder to change.
                    let can = st.feature("remind-later").on || c.reminder.is_some();
                    let set = |at| can.then(|| UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), Some(at))));
                    let last = last_used(&st, now).map(|at| item(tr("Last Used"), last_used_label(at, h24), set(at)));
                    let presets = remind_presets(&st.settings, now).into_iter().map(|(label, at)| {
                        let last = UiMessage::CommandBar(commands::Msg::LastTime(label.clone()));
                        let msg = set(at).map(|set| UiMessage::Commands(vec![last, set]));
                        item(&label, remind_label(at, h24), msg)
                    });
                    let upgrade = (!can).then(|| {
                        let open = UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus));
                        [item(tr("Upgrade to Beeper Plus"), String::new(), Some(open)), menu_rule(p)]
                    });
                    let presets = upgrade.into_iter().flatten().chain(last).chain(presets);
                    let dismiss =
                        c.reminder.is_some().then(|| UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), None)));
                    let custom =
                        UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(c.id.clone())));
                    let custom = item(tr("Custom…"), String::new(), can.then_some(custom));
                    let dismiss = item(tr("Dismiss Reminder"), String::new(), dismiss);
                    let rules = if can { 2.0 } else { 3.0 };
                    (presets.chain([menu_rule(p), custom, menu_rule(p), dismiss]).collect(), rules)
                }
                _ => {
                    let rows = self.snooze_items(&st, &targets);
                    let rules = rows.iter().filter(|r| r.1 < menu_pitch()).count() as f32;
                    (rows.into_iter().map(|r| r.0).collect(), rules)
                }
            };
            let h = 8.0 + items.len() as f32 * menu_pitch() - rules * (menu_pitch() - 11.0);
            let top = match i {
                0 => 4.0 * menu_pitch(),
                1 if labels => 8.0 * menu_pitch() + 22.0,
                3 if labels => 9.0 * menu_pitch() + 22.0,
                3 => 8.0 * menu_pitch() + 22.0,
                _ => 7.0 * menu_pitch() + 22.0,
            } + if reason.is_some() { menu_pitch() } else { 0.0 }
                + if can_edit_group(c) { menu_pitch() } else { 0.0 }
                + self.selection_rows(&st);
            let top = top.min(room - h).max(0.0);
            let body = container(Column::with_children(items).padding([4, 0])).width(SUB_MENU_W[usize::from(i)]);
            column![Space::new().height(top), body.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))]
        });
        let (l, r) = if sub_left { (sub, None) } else { (None, sub) };
        row![].push(l).push(menu).push(r).into()
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
        let p = &self.pal;
        let st = self.store.read();
        let w = self.list_width(&st);
        let chats = self.visible_chats(&st);
        let h24 = st.settings.pref("timeFormat") == 1;
        let compact = st.settings.on("compactChatList");
        let (av, lines, title_size, ts_size, pin_av) =
            if compact { (28.0, 1, 13.0, 10.0, 40.0) } else { (46.0, 2, 14.0, 11.0, 52.0) };
        let names: Vec<String> = (0..FILTERS.len())
            .filter(|i| self.list().filters & 1 << i != 0)
            .map(|i| tr(FILTERS[i].0).to_string())
            .chain(self.list().space_view_account.as_deref().map(|id| space_name(&st, id)))
            .collect();
        let heading = semi(self.list().title(&st), 13.0).color(p.strong).wrapping(iced::widget::text::Wrapping::None);
        // Desktop's space icon before the title when the space bar is hidden: a label with a color, a network space,
        // or an account with another account on its network.
        let list = self.list();
        let shown = st.settings.on("showSidebar") && !list.locked;
        let space_icon = list.selected_account_filter.as_deref().filter(|_| !shown).and_then(|id| -> Option<El<'_>> {
            if let Some(l) = rail_label(&st, id) {
                return l.color.is_some().then(|| label_icon(l, 16.0, p.strong));
            }
            let owner = rail_owner(&st, id);
            let twins = st.accounts.iter().filter(|a| brand_key(&a.account_id) == brand_key(&owner)).count() > 1;
            let icon = svg(brand_svg(brand_key(&owner))).width(16).height(16);
            (network_space(&st, id).is_some() || twins).then(|| icon.into())
        });
        let line = row![].push(space_icon.map(|i| container(i).padding(Padding::ZERO.right(4.0)))).push(heading);
        // Desktop's header of a space window neither opens the space sheet nor has a menu.
        let chevron = (!list.locked).then(|| ic(icon::UNFOLD_MORE, 12.0).color(p.weak));
        let title = column![line.push(chevron).spacing(1.4).align_y(Center)]
            .push((!names.is_empty()).then(|| txt(names.join(", "), 10.0).color(p.sub)))
            .padding([0, 8]);
        // Desktop's buttons of the chat list header take the focus without showing it (`button { outline: 0 }`).
        let window = self.drawing.get().or(self.main_window);
        let plain =
            |key: &str, b: El<'static>, msg: UiMessage| self.target(window, key, b, Keys::press(msg), Look::Hidden);
        let title: El<'_> = match list.locked {
            true => title.into(),
            false => {
                let sheet = UiMessage::ChatList(chat_list::Msg::OpenMenu(SPACE_SHEET.into()));
                let title = mouse_area(title)
                    .on_press(sheet.clone())
                    .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("head:{}", list.entry()))));
                self.target(window, "list-title", title, Keys::press(sheet), Look::Hidden)
            }
        };
        let title = container(title).width(Fill).clip(true);
        let head_btn = |c: char, size: f32, [l, r]: [f32; 2], msg: UiMessage| {
            button(container(ic(c, size).color(p.strong)).padding(Padding::ZERO.left(l).right(r).top(2.0)).center_y(26))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 8.0))
                .on_press(msg)
        };
        let filter = self.list().active_tab.filterable().then(|| {
            let count = self.list().filters.count_ones() + u32::from(self.list().space_view_account.is_some());
            let active = count > 0;
            let fg = if active { p.accent } else { p.strong };
            let trigger: El<'_> = match count {
                0 => Space::new().into(),
                1 => ic(icon::EXPAND_MORE, 10.0).color(fg).into(),
                n => txt(n.to_string(), 11.0).color(fg).into(),
            };
            let trigger = button(container(trigger).padding(Padding::ZERO.left(6.0)).center_y(20))
                .padding(0)
                .width(if active { 16 } else { 0 })
                .style(btn_style([CLEAR; 3], [fg; 3], 0.0))
                .on_press(UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu));
            let pill = row![trigger, container(ic(icon::FILTER_LIST, 16.8).color(fg)).center_x(27).center_y(20)];
            let ring = if active { p.accent } else { CLEAR };
            let face = bordered_btn([CLEAR, p.hover, p.hover], [fg; 3], ring, 1.0, 12.0);
            let press = UiMessage::ChatList(chat_list::Msg::FilterButton);
            let el = mouse_area(button(pill).padding(0).style(face).on_press(press.clone()))
                .on_right_press(UiMessage::ChatList(chat_list::Msg::ToggleFilterMenu));
            self.tip(plain("list-filter", el.into(), press), tr("Filter"), vec![], tooltip::Position::Bottom)
        });
        let below = |el: El<'static>, label, keys| self.tip_caps(el, label, keys, tooltip::Position::Bottom);
        let options = (self.list().active_tab == InboxTab::Reminders).then(|| {
            let menu = UiMessage::Reminders(reminders::Msg::Menu);
            let btn = button(container(tinted(&TUNE, 18.0, p.strong)).center_x(30).center_y(26))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.strong; 3], 8.0))
                .on_press(menu.clone());
            below(plain("list-options", btn.into(), menu), tr("Display Options"), vec![])
        });
        let header = row![title]
            .push(filter)
            .push(options)
            .push(below(
                plain(
                    "list-search",
                    head_btn(icon::SEARCH, 23.0, [7.5, 10.0], UiMessage::CommandBar(commands::Msg::TogglePalette))
                        .into(),
                    UiMessage::CommandBar(commands::Msg::TogglePalette),
                ),
                tr("Search Chats"),
                hotkey_caps(st.settings.hotkey_keys("Search")),
            ))
            .push(below(
                plain(
                    "list-new-chat",
                    head_btn(icon::EDIT_SQUARE, 20.0, [0.0, 3.0], UiMessage::NewChat(new_chat::Msg::Open)).into(),
                    UiMessage::NewChat(new_chat::Msg::Open),
                ),
                tr("New Chat"),
                hotkey_caps(st.settings.hotkey_keys("Chat: Start New")),
            ))
            .align_y(Center)
            .height(52)
            .padding(Padding::ZERO.right(8.0));
        // Desktop's `isSelected` row is the open chat, or the chat of the cursor (`threadCursorID`).
        let open = self.pane().selected_chat_id.as_deref();
        let selected = self.creating_chat.is_none().then(|| open.or(self.list().cursor.as_deref())).flatten();
        let (mut pinned, rest) = self.pins_and_rows(&st);
        // The order of a pin drag, which moves the pins under the pointer.
        let dragged = self.pin_drag.as_ref().filter(|d| d.order.is_some()).map(|d| d.id.as_str());
        if let Some(order) = self.pin_drag.as_ref().and_then(|d| d.order.as_ref()) {
            pinned.sort_by_key(|c| order.iter().position(|o| *o == c.id).unwrap_or(usize::MAX));
        }
        // A pin that the keyboard moves shows in the place it drops at, which it marks like a drag of the mouse.
        let items: std::sync::Arc<[String]> = pinned.iter().map(|c| c.id.clone()).collect();
        let sort = self.key_sort_of(window, sortable::List::Pins);
        let dragged = dragged.or(sort.map(|s| s.id.as_str()));
        if let Some(order) = sort.and_then(|s| s.moved(&items)) {
            pinned.sort_by_key(|c| order.iter().position(|o| *o == c.id).unwrap_or(usize::MAX));
        }
        let counts = st.settings.on("showUnreadCount");
        let network_icons = st.settings.on("showNetworkIcons")
            && self.list().selected_account_filter.as_deref().is_none_or(|id| rail_label(&st, id).is_some());
        let colorize = st.settings.on("colorizeNetworkIcons");
        let hover_action = st.settings.pref("chatActionButton");
        let selecting = self.select_mode || !self.selection.is_empty();
        let picked = |c: &Chat| self.selection.contains(&c.id);
        let pick = |c: &Chat| match selecting {
            true => UiMessage::ChatList(chat_list::Msg::ToggleSelected(c.id.clone())),
            false => UiMessage::ChatList(chat_list::Msg::Click(c.id.clone())),
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
                    .push(title.color(p.strong).line_height(iced::widget::text::LineHeight::Absolute(rem(15.0).into())))
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
                    .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(c.id.clone())))
                    .on_enter(UiMessage::Pins(pins::Msg::Hover(Some(c.id.clone()))))
                    .on_exit(UiMessage::Pins(pins::Msg::Hover(None)));
                // Desktop's pins are `dnd-kit` sortables, which show Chromium's ring, and whose Enter and Space pick
                // the pin up for the arrows (see `sortable`).
                let pick = Self::pick(window, sortable::List::Pins, &c.id, &items, cols);
                let item = self.target(window, format!("pin-{}", c.id), item, Keys::press(pick), Look::Auto);
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
                // Desktop's Play button takes the place of the hover actions.
                let plays = st.resolve_chat_idx(&key).is_some_and(|i| self.plays(&st.chats[i]));
                let hover = !selecting && !plays;
                let action = self.hover_progress(&key).filter(|_| hover).map(|t| match key.strip_prefix("draft:") {
                    Some(n) => Some(self.hover_btn(
                        5,
                        20.0,
                        tr("Delete Chat"),
                        vec![],
                        UiMessage::NewChat(new_chat::Msg::Delete(n.parse().unwrap())),
                        t,
                    )),
                    None => self.row_action(
                        &st.settings,
                        &st.chats[st.resolve_chat_idx(&key).expect("listed chat")],
                        hover_action,
                        t,
                    ),
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
                .on_enter(UiMessage::ChatList(chat_list::Msg::Hover(key.clone(), true)))
                .on_exit(UiMessage::ChatList(chat_list::Msg::Hover(key.clone(), false)));
                let inbox = st.resolve_chat_idx(&key).is_some_and(|i| st.chats[i].flags & CHAT_ARCHIVED == 0);
                match inbox {
                    true => row.on_middle_press(UiMessage::ChatList(chat_list::Msg::ToggleFlag(
                        key.clone(),
                        ChatFlag::Archived,
                    ))),
                    false => row,
                }
                .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(key)))
            };
        let draft_row = |d: &NewChat| {
            let n = d.id;
            let dot = (!counts).then(|| container(Space::new()).width(12).into());
            let names: Vec<&str> = d.users.iter().map(|(_, u)| u.full_name.as_deref().unwrap_or(&u.id)).collect();
            let title = if names.is_empty() { tr("New Chat").into() } else { names.join(", ") };
            let title = ell(&title, text_w, title_size, if counts { Weight::Medium } else { Weight::Semibold });
            let rows = [
                container(title.color(p.strong)).height(rem(16.0)).into(),
                Space::new().height(rem(16.0) * lines as f32).into(),
            ];
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
                    badged(
                        face,
                        brand_key(account),
                        av,
                        if compact { 12.0 } else { 16.0 },
                        (!colorize).then_some(p),
                        ring,
                    )
                }
                None => face,
            };
            frame(key, dot, face, rows, on, UiMessage::NewChat(new_chat::Msg::Select(n)))
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
            // Desktop's `quietReason` "snoozed", and the time of the Reminders, Send Later or Snoozed row.
            let snoozed = reminders::snoozed_until(&st, c);
            let remind =
                c.reminder.as_ref().map(|r| r.remind_at_ms).into_iter().chain(self.message_reminder_at(&st, c));
            let ts = match (self.list().active_tab, remind.min(), next, snoozed) {
                (InboxTab::Reminders, Some(at), _, _)
                | (InboxTab::SendLater, _, Some(at), _)
                | (InboxTab::Snoozed, _, _, Some(at)) => format!("{} {}", reminder_day(at), clock(at, h24)),
                _ if draft.is_some() => String::new(),
                _ => c.last_activity.as_deref().map(|t| list_ts(t, h24)).unwrap_or_default(),
            };
            let later = next.is_some()
                && (draft.is_none() || matches!(self.list().active_tab, InboxTab::Reminders | InboxTab::SendLater));
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
            let bell = if muted { 16.0 } else { 0.0 } + if snoozed.is_some() { 16.0 } else { 0.0 };
            let pinned = c.flag(ChatFlag::Pinned);
            let pin_icon = pinned.then(|| container(tinted(&PIN, 13.0, p.strong)).padding(Padding::ZERO.right(2.0)));
            let pin_w = if pinned { 15.0 } else { 0.0 };
            let top = row![]
                .push(pin_icon)
                .push(ell(&title, text_w - ts_w - bell - pin_w, title_size, weight).color(p.fg.scale_alpha(0.9)))
                .push(muted.then(|| container(tinted(&BELL_OFF, 12.0, p.strong)).padding([0, 2])))
                .push(snoozed.map(|_| container(tinted(&MOON_STARS, 12.0, p.strong)).padding([0, 2])))
                .push(Space::new().width(Fill))
                .push(later.then(|| tinted(&COMPOSER_ICONS[6], 12.0, p.strong.scale_alpha(0.7))))
                .push(
                    txt(ts, ts_size).color(p.strong.scale_alpha(0.7)).font(iced::Font { style, ..fw(Weight::Normal) }),
                )
                .height(rem(16.0))
                .align_y(Center);
            let mine = m.is_some_and(|m| m.is_sender == Some(true));
            let receipt = m.is_some_and(read_icon);
            let media_tag = m.and_then(message_tag);
            let plays = self.plays(c);
            // Desktop counts the new messages of a chat whose view-once media nobody opened.
            let fresh = (plays && c.unread_count > 0 && c.preview.as_ref().is_some_and(|m| self.view_once.unopened(m)))
                .then(|| {
                    let n = c.unread_count as usize;
                    trnx("%d New message", "%d New messages", n, "view-limited media chat list preview", &[&n])
                });
            let (body, italic) = match (fresh.clone(), m.and_then(preview_body)) {
                (Some(t), _) | (None, Some(t)) => (Some(t), false),
                (None, None) => {
                    media_tag.as_ref().and_then(|t| t.2.clone()).map_or((None, false), |(d, i)| (Some(d), i))
                }
            };
            let show_sender = fresh.is_none()
                && !mine
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
            let count = count.filter(|_| {
                self.hovered_chat.as_ref() != Some(&c.id)
                    || self.row_action(&st.settings, c, hover_action, 1.0).is_none()
            });
            let count = match plays {
                true => Some(self.view_once_play(&c.id)),
                false => count,
            };
            let sender = show_sender.then(|| preview_sender(c, m?)).flatten();
            let digits =
                if counts && c.unread_count > 0 { unread_label(c.unread_count as usize).len() as f32 } else { 0.0 };
            let at = if c.unread_mentions > 0 { 12.0 } else { 0.0 };
            let prefix = first.filter(|_| media_tag.is_some()).map(|s| format!("{s}:"));
            let prefix_w = prefix.as_deref().map_or(0.0, |s| crate::style::text_width(s, 13.0, Weight::Normal) + 4.0);
            let base = 16.0 + if counts { 8.0 + 6.0 * digits + at } else { at }.max(18.0);
            let errored = m.is_some_and(|m| m.send_error.is_some());
            let indent = match (receipt, errored) {
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
                // Desktop's `ZGe`: a message that failed to send shows `ErroredIcon`, without a tooltip.
                if m.send_error.is_some() {
                    // `.messageErroredIcon`: 16 by 11 with a 1 px margin, where the read icons are 12 px wide.
                    let icon = container(tinted(&ERRORED, 11.0, ERROR_COLOR)).center_x(16);
                    return container(icon).padding(Padding::ZERO.right(1.0)).into();
                }
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
                .push(body.map(|b| -> El<'_> {
                    let lines = if flowed { 1 } else { lines };
                    let shown = crate::style::clamped(&b, text_w - reserve, lines, 13.0, Weight::Normal);
                    if !shown.contains('\u{fe0f}') {
                        let text = clamp(&b, text_w - reserve, lines, 13.0, Weight::Normal);
                        return text
                            .width(text_w - reserve)
                            .line_height(Pixels(rem(16.0)))
                            .font(body_font)
                            .color(fg)
                            .into();
                    }
                    let spans = crate::emoji::with_emoji_font(vec![span::<(), _>(shown.to_string()).font(body_font)]);
                    let wrap = if lines == 1 {
                        iced::widget::text::Wrapping::None
                    } else {
                        iced::widget::text::Wrapping::WordOrGlyph
                    };
                    rich_text(spans)
                        .size(rem(13.0))
                        .line_height(Pixels(rem(16.0)))
                        .color(fg)
                        .width(text_w - reserve)
                        .wrapping(wrap)
                        .into()
                }))
                .push(draft.filter(|_| previews).map(|t| {
                    let label = trx("Draft:", "thread list draft preview prefix");
                    let s =
                        crate::style::clamped(&format!("{label} {t}"), text_w - reserve, lines, 13.0, Weight::Normal);
                    let rest = s.strip_prefix(label).expect("the draft label fits").to_string();
                    rich_text![span::<(), _>(label).font(fw(Weight::Semibold)).color(p.accent), span(rest).color(fg)]
                        .size(rem(13.0))
                        .line_height(Pixels(rem(16.0)))
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
                    badged(
                        avatar,
                        merged::chat_brand(c),
                        av,
                        if compact { 12.0 } else { 16.0 },
                        (!colorize).then_some(p),
                        ring,
                    )
                }
                false => avatar,
            };
            let face = stack![avatar]
                .push(st.typing.contains_key(&c.id).then(|| pin(self.typing_badge()).x(av - 14.0).y(-2.0)));
            let dot = match selecting {
                true => Some(container(checkbox(p, picked(c))).padding(Padding::ZERO.right(8.0)).into()),
                false => dot.map(El::from),
            };
            frame(
                c.id.clone(),
                dot,
                face.into(),
                [top.into(), preview.height(rem(16.0) * lines as f32).into()],
                on,
                pick(c),
            )
        };
        let list: El<'_> = if chats.is_empty() && self.new_chats.is_empty() {
            let tab =
                if self.list().selected_account_filter.is_some() { InboxTab::Requests } else { self.list().active_tab };
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
            let rows = self.list().rows.view(&keys, false, estimate, UiMessage::Relist, |i| match i.checked_sub(pre) {
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
        let banner = self.view_key_banner(&st);
        let sidebar =
            stack![column![header].push(status).push(banner).push(list).padding(Padding::ZERO.left(8.0))].push(pill);
        let sidebar = container(sidebar).width(w).height(Fill);
        mouse_area(sidebar).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(SIDEBAR_MENU.into()))).into()
    }
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

//! Desktop's command bar commands (`Efe`): the categories, the titles, the search keywords and the hotkeys.
use crate::i18n::{tr, trx};
use crate::style::{bar, brand_svg, ell, fill_box, scroll_style, semi, text_width, txt, Palette, CLEAR};
use crate::{
    has_flag, network_name, shown, BeeperGuiApp, El, InboxTab, SettingsHit, SettingsTab, UiMessage, ASK_LOGOUT,
};
use crate::{ASK_ARCHIVE_ALL, ASK_ARCHIVE_READ, ASK_ARCHIVE_WEEK, ASK_MARK_ALL_READ};
use beeper_core::matrix::ChatFlag;
use beeper_core::models::{Chat, Message, ThemeChoice, CHAT_UNREAD};
use beeper_core::prefs;
use beeper_core::store::BeeperStore;
use iced::widget::{column, container, mouse_area, row, scrollable, svg, Column, Row, Space};
use iced::{Center, Fill, Padding, Pixels};

/// A command: Desktop shows a disabled command, which has no message, at 70% opacity.
pub(crate) struct Command {
    pub category: &'static str,
    pub title: String,
    pub subtitle: Option<&'static str>,
    pub keywords: Vec<&'static str>,
    pub hotkey: Option<&'static str>,
    /// The digit after the hotkey modifier, as in Desktop's account list.
    pub digit: Option<usize>,
    /// The account that Desktop draws with its network icon before the title.
    pub account: Option<String>,
    /// Desktop's `time-suggestion`: the words on the left and the time on the right, in place of the title.
    pub render: Option<(String, String)>,
    /// The chat that Desktop draws in place of the title.
    pub chat: Option<String>,
    pub msg: Option<UiMessage>,
}

fn cmd(category: &'static str, title: impl Into<String>, msg: Option<UiMessage>) -> Command {
    Command {
        category,
        title: title.into(),
        subtitle: None,
        keywords: Vec::new(),
        hotkey: None,
        digit: None,
        account: None,
        render: None,
        chat: None,
        msg,
    }
}

impl Command {
    fn keys(self, keywords: &[&'static str]) -> Self {
        Self { keywords: keywords.to_vec(), ..self }
    }

    fn hotkey(self, name: &'static str) -> Self {
        Self { hotkey: Some(name), ..self }
    }

    fn sub(self, subtitle: Option<&'static str>) -> Self {
        Self { subtitle, ..self }
    }
}

const CHATS: &str = "Chats";
pub(crate) const COMMANDS: &str = "commands";
const SPACES: &str = "Switch First 9 Spaces Modifier";
/// Desktop's `searchKeywords` of the Integrations section, after its `Preferences`.
const INTEGRATIONS: &[&str] = &[
    "Preferences",
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
];

/// A command bar submenu of Desktop (`showWithSubMenu`).
#[derive(Debug, Clone, PartialEq)]
pub enum SubMenu {
    FilterAccount,
    /// The times for a reminder on a chat.
    Remind(String),
    /// The times for Send Later: the composer's, or the scheduled messages' to reschedule.
    Schedule(Vec<String>),
    /// The recently viewed chats.
    History,
    /// The folders other than the open one.
    Folders,
    /// The links of the selected messages, each an address and a title.
    Links(Vec<(String, String)>),
}

impl SubMenu {
    pub(crate) fn heading(&self) -> &'static str {
        match self {
            SubMenu::FilterAccount => tr("Filter account"),
            SubMenu::Remind(_) => tr("Remind Me"),
            SubMenu::Schedule(ids) if ids.is_empty() => tr("Send Later"),
            SubMenu::Schedule(_) => tr("Reschedule message"),
            SubMenu::History => tr("Recently viewed chats"),
            SubMenu::Folders => tr("Filter chat folders"),
            SubMenu::Links(_) => "",
        }
    }

    pub(crate) fn placeholder(&self) -> String {
        match self {
            SubMenu::FilterAccount | SubMenu::Folders | SubMenu::Links(_) => String::new(),
            SubMenu::History => tr("Search chats...").into(),
            SubMenu::Remind(_) => {
                tr("When? (try typing something like %s)").replacen("%s", "\"Monday morning at 9\"", 1)
            }
            SubMenu::Schedule(_) => tr("When? (try typing something like %s)").replacen("%s", "“Monday morning”", 1),
        }
    }
}

/// Desktop's `tGe`: the phrases that the Remind Me submenu also offers.
const MORE_TIMES: [&str; 23] = [
    "in an hour",
    "tomorrow",
    "this weekend",
    "next monday",
    "next week",
    "next month",
    "in a minute",
    "in an hour",
    "in a day",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "next year",
    "next tuesday",
    "next wednesday",
    "next thursday",
    "next friday",
    "next saturday",
    "next sunday",
];

/// Desktop's `toLocaleDateString()` and `toLocaleString()` in en-US, and the `hh:mm a` time.
fn locale_times(at: i64) -> (String, String) {
    let local = beeper_core::config::local_millis(at);
    let (y, m, d) = beeper_core::config::civil_from_days(local.div_euclid(86_400_000));
    let secs = local.div_euclid(1000).rem_euclid(86_400);
    let (h, min, sec) = (secs / 3600, secs / 60 % 60, secs % 60);
    let (h12, half) = ((h + 11) % 12 + 1, if h < 12 { "AM" } else { "PM" });
    let date = format!("{m}/{d}/{y}");
    (format!("{date}, {h12}:{min:02}:{sec:02} {half}"), format!("{date} {h12:02}:{min:02} {half}"))
}

fn search(v: Vec<Command>, query: &str) -> Vec<Command> {
    match query.is_empty() {
        true => v,
        false => prefs::match_sort_keys(v, query, |c| {
            std::iter::once(c.title.clone()).chain(c.keywords.iter().map(|k| k.to_string())).collect()
        }),
    }
}

/// Desktop's `yle`: "last used", then the suggested and typed times that are still ahead, each once.
fn time_commands(st: &BeeperStore, query: &str, pick: impl Fn(i64) -> UiMessage) -> Vec<Command> {
    let now = beeper_core::config::now_unix_millis();
    let count = query.parse::<u32>().ok().filter(|n| *n > 0);
    let counted = [("minute", 100_000_000), ("hour", 1_000_000), ("day", 100_000), ("week", 10_000)];
    let counted = counted.into_iter().filter_map(|(unit, max)| {
        let n = count.filter(|n| *n < max)?;
        Some(format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
    });
    let phrases = crate::TIME_SUGGESTIONS.iter().map(|s| s.to_string()).chain(counted);
    let phrases = phrases.chain(MORE_TIMES.iter().map(|s| s.to_string())).chain([query.to_string()]);
    let row = |label: String, at: i64, msg: UiMessage| {
        let (long, short) = locale_times(at);
        Command { render: Some((label.clone(), short)), ..cmd("", format!("{label} — {long}"), Some(msg)) }
    };
    let last = crate::last_used(st, now).map(|at| row(tr("last used").into(), at, pick(at)));
    let times = phrases.filter_map(|phrase| {
        let at = beeper_core::config::parse_when_millis(&phrase, now).filter(|at| *at > now)?;
        let label = phrase.trim().to_lowercase();
        Some(row(label, at, UiMessage::Commands(vec![UiMessage::LastTime(phrase), pick(at)])))
    });
    let mut seen = std::collections::HashSet::new();
    let mut v = search(last.into_iter().chain(times).collect(), query);
    v.retain(|c| seen.insert(c.render.as_ref().map(|r| r.0.clone())));
    v
}

impl BeeperGuiApp {
    /// Desktop's `zXe`, all accounts and then each account with its network icon, and `qXe`, the times for a reminder.
    fn submenu_commands(&self, st: &BeeperStore, menu: &SubMenu, query: &str) -> Vec<Command> {
        let search = |v: Vec<Command>| search(v, query);
        match menu {
            SubMenu::Remind(id) => {
                let Some(c) = st.resolve_chat_idx(id).map(|i| &st.chats[i]) else { return Vec::new() };
                let mut v = time_commands(st, query, |at| UiMessage::RemindIf(c.id.clone(), at, self.cmd_no_reply));
                v.extend(c.reminder.as_ref().map(|r| {
                    let dismiss = cmd("", tr("dismiss reminder"), Some(UiMessage::SetReminder(c.id.clone(), None)));
                    Command { render: Some((tr("dismiss reminder").into(), locale_times(r.remind_at_ms).1)), ..dismiss }
                }));
                v
            }
            SubMenu::Schedule(ids) => time_commands(st, query, |at| match ids.is_empty() {
                true => UiMessage::SendLater(Some((at, self.cmd_no_reply))),
                false => UiMessage::RescheduleTo(ids.clone(), at, self.cmd_no_reply),
            }),
            SubMenu::FilterAccount => {
                let all = cmd("", tr("All Accounts"), Some(UiMessage::SelectTab(InboxTab::AllChats)));
                let accounts = crate::rail_accounts(st).into_iter().filter(|a| crate::network_space(st, a).is_none());
                let accounts = accounts.enumerate().map(|(i, a)| {
                    let title = match a.as_str() {
                        "hungryserv" => st.me().full_name.clone(),
                        _ => st.bridge_accounts.iter().find(|b| b.account_id == a).and_then(|b| b.display_text()),
                    };
                    let title = title.unwrap_or_else(|| network_name(&a).into());
                    let pick = cmd("", title, Some(UiMessage::SelectAccount(Some(a.clone()))));
                    let pick = Command { digit: (i < 8).then_some(i + 2), ..pick.keys(&[network_name(&a)]) };
                    Command { account: Some(a), ..pick.hotkey(SPACES) }
                });
                search(std::iter::once(Command { digit: Some(1), ..all.hotkey(SPACES) }).chain(accounts).collect())
            }
            SubMenu::Folders => {
                let folders = [
                    (InboxTab::AllChats, "Inbox"),
                    (InboxTab::Archived, "Archived"),
                    (InboxTab::LowPriority, "Low Priority"),
                    (InboxTab::Requests, "Requests"),
                ];
                let open = |t: &InboxTab| *t == self.active_tab && self.selected_account_filter.is_none();
                let folders = folders.into_iter().filter(|(t, _)| !open(t));
                search(folders.map(|(t, name)| cmd("", tr(name), Some(UiMessage::SelectTab(t)))).collect())
            }
            SubMenu::Links(links) => links
                .iter()
                .map(|(url, title)| {
                    let label = if title.is_empty() { url.clone() } else { format!("{title} — {url}") };
                    cmd("", label, Some(UiMessage::OpenUrl(url.clone())))
                })
                .collect(),
            SubMenu::History => {
                let chats: Vec<&Chat> =
                    self.chat_history.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect();
                let at = self.pane.selected_chat_id.as_ref().and_then(|id| chats.iter().position(|c| &c.id == id));
                let mut v: Vec<Command> = chats
                    .iter()
                    .map(|c| Command {
                        chat: Some(c.id.clone()),
                        ..cmd("", c.title.clone(), Some(UiMessage::SelectChat(c.id.clone())))
                    })
                    .collect();
                let neighbor =
                    |by: isize| at.map_or(if by > 0 { 0 } else { usize::MAX }, |a| (a as isize + by) as usize);
                if let Some(c) = v.get_mut(neighbor(1)) {
                    c.hotkey = Some("History: Next Chat");
                }
                if let Some(c) = v.get_mut(neighbor(-1)) {
                    c.hotkey = Some("History: Previous Chat");
                }
                if query.is_empty() {
                    return v;
                }
                prefs::match_sort_keys(v, query, |cmd| {
                    let c = chats.iter().find(|c| Some(&c.id) == cmd.chat.as_ref()).expect("a listed chat");
                    let people = c.participants.items.iter().take(3).filter(|u| u.is_self != Some(true));
                    let people = people.flat_map(|u| [&u.full_name, &u.email, &u.phone_number, &u.username]);
                    let mut keys = vec![c.title.clone(), network_name(&c.account_id).to_string()];
                    keys.extend(people.flatten().cloned());
                    keys
                })
            }
        }
    }

    /// The commands in Desktop's order, or the ones that match `query` with the best match first.
    pub(crate) fn commands(&self, st: &BeeperStore, query: &str) -> Vec<Command> {
        if let Some(menu) = &self.cmd_submenu {
            return self.submenu_commands(st, menu, query);
        }
        let hot = |name: &'static str| Some(UiMessage::RunHotkey(name));
        let chat = self.pane.selected_chat_id.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]);
        let chat = chat.filter(|c| !c.id.starts_with("draft:") && !c.id.starts_with("account:"));
        let mut v = Vec::new();
        let picked: Vec<&Message> = match chat {
            Some(_) => self.timeline(st).0.into_iter().filter(|m| self.pane.selected_msgs.contains(&m.id)).collect(),
            None => Vec::new(),
        };
        if !picked.is_empty() {
            let e = if picked.len() == 1 { tr("Message") } else { tr("Messages") };
            let live = picked.iter().all(|m| m.is_deleted != Some(true));
            if let [m] = picked[..] {
                if live && m.is_sender == Some(true) {
                    let edit = cmd(e, trx("Edit", "message command"), hot("Message: Edit"));
                    v.push(edit.hotkey("Message: Edit"));
                }
            }
            if live {
                v.push(cmd(e, tr("Forward"), hot("Message: Forward")).hotkey("Message: Forward"));
            }
            v.push(cmd(e, trx("Copy", "message command"), Some(UiMessage::CopySelected(false))));
            v.push(cmd(e, tr("Copy with timestamps"), Some(UiMessage::CopySelected(true))));
            let files: Vec<UiMessage> = picked
                .iter()
                .filter(|m| !m.attachments.is_empty() && m.is_deleted != Some(true))
                .map(|m| UiMessage::Download(m.id.clone(), None))
                .collect();
            if !files.is_empty() {
                let download = cmd(e, tr("Download attachments"), Some(UiMessage::Commands(files)));
                v.push(download.hotkey("Download Attachments"));
            }
            let links: Vec<(String, String)> =
                picked.iter().flat_map(|m| m.links.iter().map(|l| (l.url.clone(), l.title.clone()))).collect();
            let open = match &links[..] {
                [] => None,
                [(url, _)] => Some(UiMessage::OpenUrl(url.clone())),
                _ => Some(UiMessage::OpenSubMenu(SubMenu::Links(links.clone()))),
            };
            v.extend(open.map(|open| cmd(e, tr("Open link"), Some(open))));
            v.push(cmd(e, tr("Unselect"), Some(UiMessage::SelectMessages(Vec::new()))));
            if let [m] = picked[..] {
                let reply = cmd(e, trx("Reply", "message command"), Some(UiMessage::StartReply(m.id.clone())));
                v.push(reply.hotkey("Message: Quote & Reply"));
            }
        }
        if let Some(c) = chat {
            let id = || c.id.clone();
            let e = trx("Chat", "Command bar category");
            let flag = |f| Some(UiMessage::ToggleFlag(id(), f));
            let remind = UiMessage::OpenSubMenu(SubMenu::Remind(id()));
            v.push(cmd(e, tr("Remind me later"), Some(remind)).keys(&["reminder"]).hotkey("Remind Later"));
            let low = has_flag(c, ChatFlag::LowPriority);
            let low_title = if low { tr("Move to Inbox") } else { tr("Move to Low Priority") };
            v.push(
                cmd(e, low_title, flag(ChatFlag::LowPriority)).sub((!low).then(|| tr("Hides chat from your inbox"))),
            );
            let read = if c.flags & CHAT_UNREAD != 0 { tr("Mark as read") } else { tr("Mark as unread") };
            v.push(cmd(e, read, Some(UiMessage::ToggleRead(id()))).hotkey("Chat: Toggle Mark As Read"));
            let pin = if has_flag(c, ChatFlag::Pinned) { "Unpin" } else { "Pin" };
            v.push(cmd(e, trx(pin, "chat command"), flag(ChatFlag::Pinned)).hotkey("Chat: Toggle Pin"));
            let mute = if has_flag(c, ChatFlag::Muted) { "Unmute" } else { "Mute" };
            v.push(cmd(e, trx(mute, "chat command"), flag(ChatFlag::Muted)).hotkey("Chat: Toggle Mute"));
            if self.active_tab != InboxTab::Requests {
                let archived = has_flag(c, ChatFlag::Archived);
                let title = trx(if archived { "Unarchive" } else { "Archive" }, "chat command");
                let sub = (!archived).then(|| tr("Hides chat until new messages received"));
                v.push(cmd(e, title, flag(ChatFlag::Archived)).sub(sub).hotkey("Chat: Toggle Archive"));
            }
            let delete = (c.delete_modes != 0).then(|| UiMessage::AskDeleteChat(id()));
            v.push(cmd(e, trx("Delete", "chat command"), delete).keys(&["delete", "leave"]));
            v.push(cmd(e, tr("Report as spam"), None));
            v.push(cmd(e, tr("Open in new window"), Some(UiMessage::OpenChatWindow(id()))));
        }
        v.push(
            cmd(CHATS, tr("Search chats"), Some(UiMessage::ToggleCommandPalette)).keys(&["search", "find", "filter"]),
        );
        let list = self.visible_chats(st);
        let at = self.pane.selected_chat_id.as_ref().and_then(|id| list.iter().position(|c| &c.id == id));
        let step =
            |by: isize| list.get((at.unwrap_or(0) as isize + by).rem_euclid(list.len().max(1) as isize) as usize);
        let steps = [
            ("Next Chat", "Select Next Chat", tr("Select next: %1$s %2$s"), 1),
            ("Previous Chat", "Select Previous Chat", tr("Select previous: %1$s %2$s"), -1),
        ];
        for (category, name, template, by) in steps {
            if let Some(n) = step(by).filter(|_| at.is_some() || by == 1) {
                let title = template.replacen("%1$s", network_name(&n.account_id), 1);
                let title = title.replacen("%2$s", &n.title, 1);
                v.push(Command { chat: Some(n.id.clone()), ..cmd(category, title, hot(name)).hotkey(name) });
            }
        }
        if let Some(other) = chat
            .filter(|c| c.chat_type == "single")
            .and_then(|c| c.participants.items.iter().find(|u| u.is_self != Some(true)))
        {
            let copy = |label: &'static str, value: &str| {
                cmd("User", tr(label).replacen("%s", value, 1), Some(UiMessage::CopyText(value.to_string())))
            };
            v.extend(other.full_name.as_deref().map(|n| copy("Copy name: %s", n)));
            v.extend(other.username.as_deref().map(|n| copy("Copy username: %s", n)));
            if let Some(phone) = other.phone_number.as_deref() {
                v.push(copy("Copy phone number: %s", &crate::fmt_phone(phone)));
            }
            v.extend(other.email.as_deref().map(|n| copy("Copy email: %s", n)));
            if let Some(phone) = other.phone_number.as_deref() {
                let call = tr("Call %s").replacen("%s", &crate::fmt_phone(phone), 1);
                v.push(cmd("User", call, Some(UiMessage::OpenUrl(format!("tel:{phone}")))).keys(&["phone"]));
            }
            if let Some((c, url)) = chat.and_then(|c| Some((c, crate::profile_link(c)?))) {
                let network = network_name(&c.account_id);
                let title = match other.username.as_deref() {
                    Some(u) => tr("Open %1$s on %2$s")
                        .replacen("%1$s", &if u.starts_with('@') { u.to_string() } else { format!("@{u}") }, 1)
                        .replacen("%2$s", network, 1),
                    None => tr("Open in %s").replacen("%s", network, 1),
                };
                let open = cmd("User", title, Some(UiMessage::OpenUrl(url))).keys(&["user", "profile"]);
                v.push(open.hotkey("Open User Profile"));
            }
        }
        if chat.is_some() {
            let e = "Chat";
            let info = if self.show_right_panel { tr("Hide chat info") } else { tr("Show chat info") };
            v.push(cmd(e, info, Some(UiMessage::ToggleRightPanel)).hotkey("Toggle Chat Info"));
            let file = self.pane.editing_message_id.is_none().then_some(UiMessage::PickFiles);
            let file_keys = &["attachment", "image", "gif", "video", "document", "pdf"];
            v.push(cmd(e, tr("Send file"), file).keys(file_keys).hotkey("Chat: Send File"));
            let gif = (self.pane.editing_message_id.is_none()).then_some(UiMessage::TogglePicker(crate::Picker::Gif));
            v.push(cmd(e, tr("Send GIF"), gif).keys(&["attachment", "image", "video"]).hotkey("Message: Send GIF"));
            let later = self.pane.editing_message_id.is_none().then_some(UiMessage::OpenSendLater);
            let later = cmd(trx("Message", "Command bar category"), tr("Send later"), later);
            v.push(later.keys(&["schedule", "past", "future", "time"]).hotkey("Message: Send Later"));
        }
        let history = cmd(CHATS, tr("Open recently viewed chats"), Some(UiMessage::OpenSubMenu(SubMenu::History)));
        v.push(history.keys(&["history"]));
        let new_chat = cmd("", tr("Create new chat"), Some(UiMessage::NewChat));
        v.push(new_chat.keys(&["compose", "create", "person", "chat"]).hotkey("Chat: Start New"));
        let join = UiMessage::Commands(vec![UiMessage::NewChat, UiMessage::ToggleJoinMatrix]);
        v.push(cmd("", tr("Join a Matrix room"), Some(join)).keys(&["matrix", "room", "join", "address", "alias"]));
        let all = cmd("", tr("Search messages in all chats"), Some(UiMessage::ToggleCommandPalette));
        v.push(all.keys(&["find", "grep"]).hotkey("Search"));
        if let Some(c) = chat {
            let here = cmd("", tr("Search messages in current chat"), Some(UiMessage::SearchRoom(c.id.clone())));
            v.push(here.keys(&["find", "grep"]).hotkey("Search: Room"));
        }
        let folders = (self.active_tab != InboxTab::Requests).then_some(UiMessage::OpenSubMenu(SubMenu::Folders));
        v.push(cmd("Filter Chats", tr("Filter by folder type"), folders));
        if crate::rail_accounts(st).len() > 1 {
            let filter =
                cmd("Filter Chats", tr("Filter by account"), Some(UiMessage::OpenSubMenu(SubMenu::FilterAccount)));
            v.push(filter.hotkey("Filter By Account"));
        }
        let requests = self.active_tab == InboxTab::Requests;
        let (inbox, tab) = match requests {
            true => (tr("Show default inbox"), InboxTab::AllChats),
            false => (tr("Show message requests"), InboxTab::Requests),
        };
        v.push(cmd("", inbox, Some(UiMessage::SelectTab(tab))).keys(&["dm", "unknown"]));
        let prefs = &["Preferences"];
        let settings = tr("Settings");
        let open = |tab, then: UiMessage| Some(UiMessage::Commands(vec![UiMessage::OpenSettings(tab), then]));
        let add = match self.adding_account {
            true => UiMessage::SelectSettingsTab(SettingsTab::Accounts),
            false => UiMessage::ToggleAddAccount,
        };
        v.push(cmd(settings, tr("Add Account"), open(SettingsTab::Accounts, add)).keys(prefs));
        use SettingsTab as T;
        let tabs = [T::Accounts, T::BeeperPlus, T::General, T::Ai, T::Appearance, T::Notifications, T::Hotkeys];
        for tab in tabs.into_iter().chain([T::Storage, T::Devices, T::About, T::Help, T::Integrations]) {
            let keys = if tab == T::Integrations { INTEGRATIONS } else { prefs };
            let section = open(tab, UiMessage::PickSetting(SettingsHit::Section(tab, true)));
            v.push(cmd(settings, tab.name(), section).keys(keys));
            let rows = prefs::rows().filter(|(page, r)| *page == tab.meta().0 && !r.label.is_empty() && shown(st, r));
            v.extend(rows.map(|(_, r)| {
                let title = format!("{} → {}", tab.name(), crate::trs(r.label, r.split_args().0));
                let pick = open(tab, UiMessage::PickSetting(SettingsHit::Pref(tab, r)));
                cmd(settings, title, pick).keys(&[r.desc.unwrap_or_default(), "Preferences"])
            }));
        }
        v.extend(crate::login::addable().map(|n| {
            let title = format!("{} → {}", tr("Accounts"), n.display_name);
            cmd(settings, title, open(T::Accounts, UiMessage::PickSetting(SettingsHit::Network(n.display_name))))
                .keys(prefs)
        }));
        v.push(cmd("", tr("Show settings"), hot("Settings")).keys(prefs).hotkey("Settings"));
        let incognito = st.settings.on("incognito");
        let title = tr(if incognito { "Turn off incognito mode" } else { "Turn on incognito mode" });
        let toggle = st.feature("stealth-mode").on.then_some(UiMessage::SetPref("incognito", u8::from(!incognito)));
        v.push(cmd(tr("Settings"), title, toggle).keys(&["Preferences", "stealth"]));
        if self.hotkeys_modal.is_none() {
            let shortcuts = cmd("", tr("Show keyboard shortcuts"), hot("Keyboard Shortcuts"));
            v.push(shortcuts.keys(&["hotkeys"]).hotkey("Keyboard Shortcuts"));
        }
        v.push(cmd("", tr("Toggle sidebar"), hot("Toggle Sidebar")).keys(&["hide", "show"]).hotkey("Toggle Sidebar"));
        let space = cmd("", tr("Toggle space bar"), hot("Toggle Space Bar"));
        v.push(space.keys(&["hide", "show"]).hotkey("Toggle Space Bar"));
        let media = &["media", "gallery", "photos", "images", "videos", "files", "links", "docs"];
        v.push(cmd("Window", tr("Media viewer"), Some(UiMessage::OpenMedia)).keys(media));
        v.push(cmd("Window", tr("Maximize/unmaximize"), Some(UiMessage::ToggleMaximize)));
        v.push(cmd("View", tr("Zoom in"), hot("Zoom In")).keys(&["magnify"]).hotkey("Zoom In"));
        v.push(cmd("View", tr("Zoom out"), hot("Zoom Out")).keys(&["magnify"]).hotkey("Zoom Out"));
        let reset = hot("Reset Zoom").filter(|_| st.settings.zoom_percent != 100);
        v.push(cmd("View", tr("Reset zoom"), reset).keys(&["magnify"]).hotkey("Reset Zoom"));
        let feedback = Some(UiMessage::OpenUrl("mailto:help+feedback@beeper.com".into()));
        v.push(cmd("", tr("Email feedback"), feedback).keys(&[
            "send",
            "report",
            "suggestion",
            "criticism",
            "bug",
            "help",
        ]));
        let changelog = Some(UiMessage::OpenUrl(crate::CHANGELOG_URL.into()));
        v.push(cmd("", tr("View changelog"), changelog).keys(&["whats new", "updates"]));
        for theme in ThemeChoice::ALL.into_iter().filter(|t| *t != st.settings.theme) {
            let title = match theme {
                ThemeChoice::System => tr("Set theme to match system"),
                ThemeChoice::Light => tr("Set theme to light mode"),
                ThemeChoice::Dark => tr("Set theme to dark mode"),
            };
            v.push(cmd("UI", title, Some(UiMessage::SelectTheme(theme))).keys(&["dark", "light", "color"]));
        }
        if self.selection.is_empty() {
            v.push(cmd(CHATS, tr("Archive all chats"), Some(UiMessage::Ask(Some(ASK_ARCHIVE_ALL)))));
            v.push(cmd(CHATS, tr("Archive all chats older than a week"), Some(UiMessage::Ask(Some(ASK_ARCHIVE_WEEK)))));
            let read = cmd(CHATS, tr("Archive all read chats"), Some(UiMessage::Ask(Some(ASK_ARCHIVE_READ))));
            v.push(read.hotkey("Chat: Archive All Read"));
            v.push(cmd(CHATS, tr("Mark all chats as read"), Some(UiMessage::Ask(Some(ASK_MARK_ALL_READ)))));
        }
        let logout = match st.me().full_name.as_deref() {
            Some(name) => {
                tr("Logout of your %1$s account (%2$s)").replacen("%1$s", "Beeper", 1).replacen("%2$s", name, 1)
            }
            None => tr("Logout of your %s account").replacen("%s", "Beeper", 1),
        };
        v.push(cmd("", logout, Some(UiMessage::Ask(Some(ASK_LOGOUT)))));
        let problem = cmd("", tr("Report a problem"), hot("Report a Problem"));
        v.push(problem.keys(&["bug", "issue", "feedback"]).hotkey("Report a Problem"));
        if let Some(c) = chat {
            v.push(cmd("Developer", "Copy current chat ID", Some(UiMessage::CopyText(c.id.clone()))));
        }
        v.push(cmd("Developer", "Reload app", hot("Reload App")).hotkey("Reload App"));
        v.push(cmd("Developer", "Relaunch app", Some(UiMessage::Relaunch)));
        if !query.is_empty() {
            v = prefs::match_sort_keys(v, query, |c| {
                let mut keys = vec![c.title.clone()];
                keys.extend(c.keywords.iter().map(|k| k.to_string()));
                keys.push(c.category.into());
                keys
            });
        }
        let groups = crate::first_groups(&v, |c| c.category);
        v.sort_by_key(|c| groups.iter().position(|g| *g == c.category));
        v
    }

    /// The commands under Desktop's category headings.
    pub(crate) fn view_commands(&self, st: &BeeperStore, cmds: &[Command]) -> El<'_> {
        let p = &self.pal;
        let list = cmds.iter().enumerate().fold(Column::new(), |col, (i, c)| {
            let head = (self.cmd_submenu.is_none() && (i == 0 || cmds[i - 1].category != c.category)).then(|| {
                let head = semi(if c.category.is_empty() { "Commands" } else { c.category }, 12.0);
                container(head.line_height(Pixels(15.0)).color(p.sub))
                    .center_y(32)
                    .padding(Padding::ZERO.left(12.0).right(4.0))
            });
            col.push(head).push(self.view_command(st, c, i == self.cmd_index, i))
        });
        let list = scrollable(list.padding(Padding::ZERO.bottom(6.0))).id(COMMANDS).direction(bar());
        list.height(Fill).style(scroll_style(p.scroll)).into()
    }

    fn view_command(&self, st: &BeeperStore, c: &Command, lit: bool, i: usize) -> El<'_> {
        let p = &self.pal;
        let alpha = if c.msg.is_some() { 1.0 } else { 0.7 };
        let title: El<'_> = match c.chat.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]) {
            Some(chat) => self.command_chat(p, chat),
            None if c.render.is_some() => {
                let (left, right) = c.render.clone().unwrap_or_default();
                let words = semi(left, 13.0).line_height(Pixels(16.0)).color(p.strong);
                let right = semi(right, 13.0).line_height(Pixels(16.0)).color(p.strong);
                row![words, Space::new().width(Fill), right].width(Fill).into()
            }
            None if c.account.is_some() => {
                let logo =
                    svg(brand_svg(crate::brand_key(c.account.as_deref().unwrap_or_default()))).width(20).height(20);
                let logo = container(logo).padding(Padding::ZERO.left(6.0).right(14.0));
                row![logo, semi(c.title.clone(), 13.0).line_height(Pixels(16.0)).color(p.strong)].align_y(Center).into()
            }
            None => {
                let title = semi(c.title.clone(), 13.0).line_height(Pixels(16.0)).color(p.strong.scale_alpha(alpha));
                let sub = c.subtitle.map(|s| txt(s, 13.0).line_height(Pixels(16.0)).color(p.sub.scale_alpha(alpha)));
                row![title].push(sub).spacing(4).align_y(Center).into()
            }
        };
        let keys = c.hotkey.map_or("disabled", |h| st.settings.hotkey_keys(h)).split(", ").next().unwrap_or("");
        let keys = match c.digit {
            Some(d) if keys != "disabled" => format!("{keys}+{d}"),
            _ => keys.to_string(),
        };
        let label = prefs::hotkey_label(&keys);
        let caps = (!label.is_empty()).then(|| {
            Row::with_children(label.split(' ').map(|k| {
                let w = (text_width(k, 11.0, iced::font::Weight::Semibold) + 6.0).max(20.0);
                let key = semi(k.to_string(), 11.0).line_height(Pixels(14.0)).color(p.strong.scale_alpha(alpha));
                container(key).center_x(w).center_y(20).style(fill_box(p.strong.scale_alpha(0.1), 4.0)).into()
            }))
            .spacing(4)
        });
        let line = row![title].push(c.render.is_none().then(|| Space::new().width(Fill))).push(caps);
        let line = line.spacing(8).align_y(Center);
        let bg = if lit { p.chip } else { CLEAR };
        let body = container(line).center_y(36).padding([0, 6]).style(fill_box(bg, 8.0));
        let body = container(body).padding([0, 6]).id(format!("cmd-{i}"));
        let hit = mouse_area(body).on_enter(UiMessage::CommandHighlight(i));
        match c.msg.clone() {
            Some(m) => hit.on_press(UiMessage::Commands(vec![UiMessage::CloseModals, m])).into(),
            None => hit.into(),
        }
    }

    /// Desktop's `Ed` chat line: the avatar, the name, the network and the brand icon.
    fn command_chat<'a>(&'a self, p: &Palette, c: &Chat) -> El<'a> {
        let title = row![
            ell(&c.title, 260.0, 13.0, iced::font::Weight::Semibold).color(p.strong),
            ell(&crate::chat_search_subtitle(c), 220.0, 13.0, iced::font::Weight::Normal).color(p.sub),
        ]
        .spacing(4)
        .align_y(Center);
        column![row![self.chat_avatar(c, 28.0, 0.0), title].spacing(8).align_y(Center)].into()
    }
}

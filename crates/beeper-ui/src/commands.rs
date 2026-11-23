//! Desktop's command bar commands (`Efe`): the categories, the titles, the search keywords and the hotkeys.
use crate::accounts;
use crate::blocking;
use crate::chat_list;
use crate::composer;
use crate::i18n::{tr, trf, trx};
use crate::media_view;
use crate::members;
use crate::message;
use crate::new_chat;
use crate::rail;
use crate::reminders;
use crate::scheduled;
use crate::settings;
use crate::style::rem;
use crate::style::{bar, brand_svg, ell, fill_box, scroll_style, semi, text_width, txt, Palette, CLEAR};
use crate::threads;
use crate::windows;
use crate::{
    has_flag, network_name, shown, BeeperGuiApp, El, InboxTab, SettingsHit, SettingsTab, UiMessage, ASK_LOGOUT,
};
use crate::{Find, SearchFilters, SearchPick, ASK_ARCHIVE_ALL, ASK_ARCHIVE_READ, ASK_ARCHIVE_WEEK, ASK_MARK_ALL_READ};
use beeper_core::models::{
    AccountProps, Chat, ChatFlag, Message, ThemeChoice, User, CHAT_UNREAD, MANAGE_KICK, MANAGE_ROLE, MEMBER_INVITE,
    MEMBER_KICK, MEMBER_LEAVE,
};
use beeper_core::prefs;
use beeper_core::store::BeeperStore;
use iced::widget::operation;
use iced::widget::{column, container, mouse_area, row, scrollable, svg, Row, Space};
use iced::{Center, Fill, Padding, Pixels, Task};

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
    /// Desktop's `CommandBarUser` in place of the title.
    pub user: Option<User>,
    pub msg: Option<UiMessage>,
}

/// A category name made at run time, kept once per text for the life of the app.
fn interned(s: String) -> &'static str {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().expect("the category list");
    match seen.iter().find(|t| **t == s) {
        Some(t) => t,
        None => {
            let t: &'static str = Box::leak(s.into_boxed_str());
            seen.push(t);
            t
        }
    }
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
        user: None,
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

    /// The row key of the command in the virtual list.
    pub(crate) fn key(&self) -> u64 {
        crate::vlist::key((self.category, &self.title, self.user.as_ref().map(|u| &u.id)))
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
    /// Desktop's `remindLaterMessage`: the times for a reminder on a message.
    RemindMessage(String),
    /// Desktop's `snoozeThread`: the times to snooze the chats until.
    Snooze(Vec<String>),
    /// The times for Send Later: the composer's, or the scheduled messages' to reschedule.
    Schedule(Vec<String>),
    /// The recently viewed chats.
    History,
    /// The folders other than the open one.
    Folders,
    /// The links of the selected messages, each an address and a title.
    Links(Vec<(String, String)>),
    /// The members of the open chat to remove (`MEMBERS_KICK`), to make admins or to make regular members.
    Members(u8),
    /// Desktop's `ADD_THREAD_PARTICIPANT`: the people to invite to the open chat.
    AddMembers,
    /// Desktop's `THREAD_FILTERS`: the chat filters.
    Filters,
}

pub const MEMBERS_KICK: u8 = 0;
pub const MEMBERS_ADMIN: u8 = 1;
pub const MEMBERS_DISMISS: u8 = 2;

impl SubMenu {
    pub(crate) fn heading(&self) -> &'static str {
        match self {
            SubMenu::FilterAccount => tr("Filter account"),
            SubMenu::Remind(_) | SubMenu::RemindMessage(_) => tr("Remind Me"),
            SubMenu::Snooze(_) => trx("Snooze", "command bar heading for snoozing chats"),
            SubMenu::Schedule(ids) if ids.is_empty() => tr("Send Later"),
            SubMenu::Schedule(_) => tr("Reschedule message"),
            SubMenu::History => tr("Recently viewed chats"),
            SubMenu::Folders => tr("Filter chat folders"),
            SubMenu::Filters => tr("Filter chats"),
            SubMenu::Links(_) => "",
            SubMenu::Members(MEMBERS_KICK) => tr("Remove member from group"),
            SubMenu::Members(MEMBERS_ADMIN) => tr("Make group admin"),
            SubMenu::Members(_) => tr("Dismiss group admin"),
            SubMenu::AddMembers => tr("Add members to chat"),
        }
    }

    pub(crate) fn placeholder(&self) -> String {
        match self {
            SubMenu::FilterAccount | SubMenu::Folders | SubMenu::Filters | SubMenu::Links(_) => String::new(),
            SubMenu::History => tr("Search chats...").into(),
            SubMenu::Members(_) | SubMenu::AddMembers => tr("Search users...").into(),
            SubMenu::Remind(_) | SubMenu::RemindMessage(_) => {
                tr("When? (try typing something like %s)").replacen("%s", "\"Monday morning at 9\"", 1)
            }
            SubMenu::Snooze(_) => trf("Until when? (try typing something like %s)", &[&"\"tomorrow at 9\""]),
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

/// Desktop's `cy`: "last used", then the `timeSuggestionOptions`, counted and typed times that are still ahead, each
/// once. A snooze (`yle`) has its own last used phrase.
fn time_commands(st: &BeeperStore, query: &str, snooze: bool, pick: impl Fn(i64) -> UiMessage) -> Vec<Command> {
    let now = beeper_core::config::now_unix_millis();
    let count = query.parse::<u32>().ok().filter(|n| *n > 0);
    let counted = [("minute", 100_000_000), ("hour", 1_000_000), ("day", 100_000), ("week", 10_000)];
    let counted = counted.into_iter().filter_map(|(unit, max)| {
        let n = count.filter(|n| *n < max)?;
        Some(format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
    });
    let phrases = st.settings.time_suggestions().into_iter().map(String::from).chain(counted);
    let phrases = phrases.chain(MORE_TIMES.iter().map(|s| s.to_string())).chain([query.to_string()]);
    let row = |label: String, at: i64, msg: UiMessage| {
        let (long, short) = locale_times(at);
        Command { render: Some((label.clone(), short)), ..cmd("", format!("{label} — {long}"), Some(msg)) }
    };
    let last = if snooze { &st.settings.snooze_last_time } else { &st.settings.last_time };
    let last = crate::reminders::last_used_of(last, now).map(|at| row(tr("last used").into(), at, pick(at)));
    let times = phrases.filter_map(|phrase| {
        let at = beeper_core::config::parse_when_millis(&phrase, now).filter(|at| *at > now)?;
        let label = phrase.trim().to_lowercase();
        let used = match snooze {
            true => UiMessage::Reminders(reminders::Msg::SnoozeTime(phrase)),
            false => UiMessage::CommandBar(Msg::LastTime(phrase)),
        };
        Some(row(label, at, UiMessage::Commands(vec![used, pick(at)])))
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
                let mut v = time_commands(st, query, false, |at| {
                    UiMessage::Reminders(reminders::Msg::If(c.id.clone(), at, self.cmd_no_reply))
                });
                v.extend(c.reminder.as_ref().map(|r| {
                    let dismiss = cmd(
                        "",
                        tr("dismiss reminder"),
                        Some(UiMessage::Reminders(reminders::Msg::Set(c.id.clone(), None))),
                    );
                    Command { render: Some((tr("dismiss reminder").into(), locale_times(r.remind_at_ms).1)), ..dismiss }
                }));
                v
            }
            SubMenu::RemindMessage(mid) => time_commands(st, query, false, |at| {
                UiMessage::Reminders(reminders::Msg::Message(mid.clone(), Some(at)))
            }),
            // Desktop's `Eit`: the times, and "dismiss snooze" with the end of the snooze of one chat.
            SubMenu::Snooze(ids) => {
                let chats: Vec<&Chat> =
                    ids.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect();
                let snooze = |at| UiMessage::Reminders(reminders::Msg::Snooze(ids.clone(), at));
                let mut v = time_commands(st, query, true, |at| snooze(Some(at)));
                v.extend(crate::reminders::can_dismiss_snooze(st, &chats).then(|| {
                    let until =
                        chats.first().filter(|_| chats.len() == 1).and_then(|c| crate::reminders::snoozed_until(st, c));
                    let render = (tr("dismiss snooze").into(), until.map(|at| locale_times(at).1).unwrap_or_default());
                    Command { render: Some(render), ..cmd("", tr("dismiss snooze"), Some(snooze(None))) }
                }));
                v
            }
            SubMenu::Schedule(ids) => time_commands(st, query, false, |at| match ids.is_empty() {
                true => UiMessage::Scheduled(scheduled::Msg::Set(Some((at, self.cmd_no_reply)))),
                false => UiMessage::Scheduled(scheduled::Msg::RescheduleTo(ids.clone(), at, self.cmd_no_reply)),
            }),
            SubMenu::FilterAccount => {
                let all = cmd(
                    "",
                    tr("All Accounts"),
                    Some(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::AllChats))),
                );
                let accounts = crate::rail_accounts(st).into_iter().filter(|a| crate::network_space(st, a).is_none());
                let accounts = accounts.enumerate().map(|(i, a)| {
                    let title = match a.as_str() {
                        "hungryserv" => st.me().full_name.clone(),
                        // Desktop's `userDisplayText`: the nickname, or the user.
                        _ => st
                            .bridge_accounts
                            .iter()
                            .find(|b| b.account_id == a)
                            .and_then(|b| st.custom_name(&a).map(str::to_string).or_else(|| b.display_text())),
                    };
                    let title = title.unwrap_or_else(|| network_name(&a).into());
                    let pick = cmd("", title, Some(UiMessage::Rail(rail::Msg::Select(Some(a.clone())))));
                    let pick = Command { digit: (i < 8).then_some(i + 2), ..pick.keys(&[network_name(&a)]) };
                    Command { account: Some(a), ..pick.hotkey(SPACES) }
                });
                search(std::iter::once(Command { digit: Some(1), ..all.hotkey(SPACES) }).chain(accounts).collect())
            }
            // Desktop's `Ait`: Clear all filters while one is on, then each filter.
            SubMenu::Filters => {
                let pick = |bit| Some(UiMessage::ChatList(chat_list::Msg::PickFilter(bit)));
                let clear = UiMessage::ChatList(chat_list::Msg::ClearFilters);
                let clear = (self.list().filters != 0).then(|| cmd("", tr("Clear all filters"), Some(clear)));
                let each = chat_list::FILTERS.iter().enumerate().map(|(i, f)| cmd("", tr(f.0), pick(1 << i)));
                search(clear.into_iter().chain(each).collect())
            }
            SubMenu::Folders => {
                let folders = [
                    (InboxTab::AllChats, "Inbox"),
                    (InboxTab::Archived, "Archived"),
                    (InboxTab::LowPriority, "Low Priority"),
                    (InboxTab::Requests, "Requests"),
                ];
                let open = |t: &InboxTab| *t == self.list().active_tab && self.list().selected_account_filter.is_none();
                let folders = folders.into_iter().filter(|(t, _)| !open(t));
                search(
                    folders
                        .map(|(t, name)| cmd("", tr(name), Some(UiMessage::ChatList(chat_list::Msg::SelectTab(t)))))
                        .collect(),
                )
            }
            SubMenu::Members(kind) => {
                let Some(c) = self.pane.selected_chat_id.as_deref().and_then(|id| st.resolve_chat_idx(id)) else {
                    return Vec::new();
                };
                let c = &st.chats[c];
                let listed = c.participants.items.iter().filter(|u| match *kind {
                    MEMBERS_KICK => u.manage & MANAGE_KICK != 0,
                    MEMBERS_ADMIN => u.manage & MANAGE_ROLE != 0 && u.is_admin != Some(true),
                    _ => u.manage & MANAGE_ROLE != 0 && u.is_admin == Some(true),
                });
                let listed = listed.map(|u| {
                    let (chat, user) = (c.id.clone(), u.id.clone());
                    let msg = match *kind {
                        MEMBERS_KICK => UiMessage::Members(members::Msg::Remove(chat, user)),
                        k => UiMessage::Members(members::Msg::SetAdmin(chat, user, k == MEMBERS_ADMIN)),
                    };
                    Command { user: Some(u.clone()), ..cmd("", crate::member_name(u), Some(msg)) }
                });
                search(listed.collect())
            }
            SubMenu::AddMembers => self.add_member_commands(st, query),
            SubMenu::Links(links) => links
                .iter()
                .map(|(url, title)| {
                    let label = if title.is_empty() { url.clone() } else { format!("{title} — {url}") };
                    cmd("", label, Some(UiMessage::OpenUrl(url.clone())))
                })
                .collect(),
            SubMenu::History => {
                let chats: Vec<&Chat> =
                    st.recent_chat_ids.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect();
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

    /// Desktop's `xXe`: the people of the account's other chats (`SXe`), then the users of the bridge search.
    fn add_member_commands(&self, st: &BeeperStore, query: &str) -> Vec<Command> {
        let Some(c) = self.pane.selected_chat_id.as_deref().and_then(|id| st.resolve_chat_idx(id)) else {
            return Vec::new();
        };
        let c = &st.chats[c];
        let row = |category, u: &User| Command {
            user: Some(u.clone()),
            account: Some(c.account_id.clone()),
            ..cmd(
                category,
                crate::member_name(u),
                Some(UiMessage::Members(members::Msg::Add(c.id.clone(), Box::new(u.clone())))),
            )
        };
        let q = query.trim().to_lowercase();
        let inbox = st
            .chats
            .iter()
            .filter(|o| o.account_id == c.account_id && o.is_request != Some(true) && !o.flag(ChatFlag::LowPriority));
        let mut seen: std::collections::HashSet<&str> = c.participants.items.iter().map(|u| u.id.as_str()).collect();
        let mut people: Vec<&User> = inbox
            .flat_map(|o| &o.participants.items)
            .filter(|u| u.is_self != Some(true) && seen.insert(&u.id))
            .filter(|u| {
                let fields = [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email];
                let fields = fields.into_iter().flatten().chain(std::iter::once(&u.id));
                q.is_empty() || fields.map(|f| f.to_lowercase()).any(|f| f.contains(&q))
            })
            .collect();
        let name = |u: &User| {
            let names = [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email];
            names.into_iter().flatten().next().map(|n| n.to_lowercase())
        };
        people.sort_by_cached_key(|u| (name(u).is_none(), name(u)));
        let mut v: Vec<Command> = people.into_iter().map(|u| row(tr("Chats"), u)).collect();
        let found = self.new_chat_search.as_ref().filter(|(s, ..)| !q.is_empty() && s == query.trim());
        let found = found.into_iter().flat_map(|(_, _, f)| f).filter(|(a, _)| *a == c.account_id);
        v.extend(found.map(|(_, u)| {
            let add = row(tr("Connections"), u);
            Command { msg: add.msg.filter(|_| u.cannot_message != Some(true)), ..add }
        }));
        v
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
            let rules = chat.expect("a chat of the picked messages").messages;
            let sent = picked.iter().all(|m| !crate::pending(st, m));
            // Desktop's `KFt` reactions: type an emoji to react with it, or to remove your reaction of it.
            if live && sent && rules.react && picked.iter().all(|m| m.reactable()) {
                let key = query.trim();
                let own =
                    |m: &&Message| m.reactions.iter().any(|r| r.reaction_key == key && st.is_me(&r.participant_id));
                if crate::emoji::is_emoji(key) {
                    let mine = picked.iter().any(own);
                    let chat_id = chat.expect("a chat of the picked messages").id.clone();
                    let toggles = picked
                        .iter()
                        .filter(|m| own(m) == mine)
                        .map(|m| {
                            UiMessage::Message(message::Msg::ToggleReaction(chat_id.clone(), m.id.clone(), key.into()))
                        })
                        .collect();
                    let title = if mine { trf("Remove %s reaction", &[&key]) } else { trf("React with %s", &[&key]) };
                    v.push(cmd(e, title, Some(UiMessage::Commands(toggles))));
                } else if key.is_empty() {
                    v.push(cmd(e, tr("Type any emoji to react with it"), None));
                }
            }
            if let [m] = picked[..] {
                if crate::editable(m, chat.expect("a chat of the picked messages")) {
                    let edit = crate::edit_open(m, rules).then(|| UiMessage::RunHotkey("Message: Edit"));
                    v.push(cmd(e, trx("Edit", "message command"), edit).hotkey("Message: Edit"));
                }
            }
            // Desktop's `ghe` with the source "Command Bar", only without pending messages.
            if sent && crate::delete_dialog(st, &picked).is_some() {
                let ask = UiMessage::Message(message::Msg::AskDelete(picked.iter().map(|m| m.id.clone()).collect()));
                v.push(cmd(e, tr("Delete…"), Some(ask)).keys(&["unsend", "remove"]));
            } else if !rules.delete && !rules.delete_for_me {
                let name = crate::network_name(&picked[0].account_id);
                v.push(cmd(e, trf("Delete message is not supported for %s yet", &[&name]), None));
            }
            if live {
                v.push(cmd(e, tr("Forward"), hot("Message: Forward")).hotkey("Message: Forward"));
            }
            v.push(cmd(e, trx("Copy", "message command"), Some(UiMessage::Message(message::Msg::CopySelected(false)))));
            v.push(cmd(e, tr("Copy with timestamps"), Some(UiMessage::Message(message::Msg::CopySelected(true)))));
            let files: Vec<UiMessage> = picked
                .iter()
                .filter(|m| !m.attachments.is_empty() && m.is_deleted != Some(true))
                .map(|m| UiMessage::Message(message::Msg::Download(m.id.clone(), None)))
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
                _ => Some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::Links(links.clone())))),
            };
            v.extend(open.map(|open| cmd(e, tr("Open link"), Some(open))));
            v.push(cmd(e, tr("Unselect"), Some(UiMessage::Message(message::Msg::Select(Vec::new())))));
            if let ([m], true) = (&picked[..], rules.reply && live) {
                let reply = cmd(
                    e,
                    trx("Reply", "message command"),
                    Some(UiMessage::Composer(composer::Msg::StartReply(m.id.clone()))),
                );
                v.push(reply.hotkey("Message: Quote & Reply"));
            }
            if let [m] = picked[..] {
                if crate::can_thread(st, chat.expect("a chat of the picked messages"), m) {
                    let thread = Some(UiMessage::Thread(threads::Msg::Show(m.id.clone())));
                    v.push(cmd(e, trx("Reply in thread", "message command"), thread).hotkey("Show Replies"));
                }
            }
        }
        if let Some(c) = chat {
            let id = || c.id.clone();
            let e = trx("Chat", "Command bar category");
            let flag = |f| Some(UiMessage::ChatList(chat_list::Msg::ToggleFlag(id(), f)));
            let remind = UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::Remind(id())));
            v.push(cmd(e, tr("Remind me later"), Some(remind)).keys(&["reminder"]).hotkey("Remind Later"));
            let low = has_flag(c, ChatFlag::LowPriority);
            let low_title = if low { tr("Move to Inbox") } else { tr("Move to Low Priority") };
            v.push(
                cmd(e, low_title, flag(ChatFlag::LowPriority)).sub((!low).then(|| tr("Hides chat from your inbox"))),
            );
            let read = if c.flags & CHAT_UNREAD != 0 { tr("Mark as read") } else { tr("Mark as unread") };
            v.push(
                cmd(e, read, Some(UiMessage::ChatList(chat_list::Msg::ToggleRead(id()))))
                    .hotkey("Chat: Toggle Mark As Read"),
            );
            let pin = if has_flag(c, ChatFlag::Pinned) { "Unpin" } else { "Pin" };
            v.push(cmd(e, trx(pin, "chat command"), flag(ChatFlag::Pinned)).hotkey("Chat: Toggle Pin"));
            let mute = if has_flag(c, ChatFlag::Muted) { "Unmute" } else { "Mute" };
            v.push(cmd(e, trx(mute, "chat command"), flag(ChatFlag::Muted)).hotkey("Chat: Toggle Mute"));
            if self.list().active_tab != InboxTab::Requests {
                let archived = has_flag(c, ChatFlag::Archived);
                let title = trx(if archived { "Unarchive" } else { "Archive" }, "chat command");
                let sub = (!archived).then(|| tr("Hides chat until new messages received"));
                v.push(cmd(e, title, flag(ChatFlag::Archived)).sub(sub).hotkey("Chat: Toggle Archive"));
            }
            if st.feature("delete-chats").on {
                let delete = (c.delete_modes != 0).then(|| UiMessage::ChatList(chat_list::Msg::AskDelete(id())));
                v.push(cmd(e, trx("Delete", "chat command"), delete).keys(&["delete", "leave"]));
            }
            v.extend(crate::block_action(st, c).map(|block| {
                let msg = Some(UiMessage::Blocking(blocking::Msg::Ask(id(), block)));
                match block {
                    true => cmd(e, trx("Block", "chat command"), msg).keys(&["spam", "report"]),
                    false => cmd(e, trx("Unblock", "chat command"), msg),
                }
            }));
            v.push(cmd(e, tr("Open in new window"), Some(UiMessage::Window(windows::Msg::OpenChat(id())))));
        }
        v.push(
            cmd(CHATS, tr("Search chats"), Some(UiMessage::CommandBar(Msg::TogglePalette)))
                .keys(&["search", "find", "filter"])
                .hotkey("Search"),
        );
        let merge = UiMessage::Merged(crate::merged::Msg::Open(None, Vec::new()));
        v.push(cmd(CHATS, tr("Merge chats"), Some(merge)).keys(&["combine"]));
        let steps = [
            ("Next Chat", "Select Next Chat", tr("Select next: %1$s %2$s"), 1),
            ("Previous Chat", "Select Previous Chat", tr("Select previous: %1$s %2$s"), -1),
        ];
        for (category, name, template, by) in steps {
            if let Some(n) = self.chat_by(st, by) {
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
                cmd(
                    "User",
                    tr(label).replacen("%s", value, 1),
                    Some(UiMessage::Message(message::Msg::CopyText(value.to_string()))),
                )
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
        if let Some(c) = chat {
            let e = "Chat";
            let members = |kind| Some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::Members(kind))));
            let can = |bit| c.participants.items.iter().any(|u| u.manage & bit != 0);
            if c.member_actions & MEMBER_INVITE != 0 {
                let add = cmd(e, tr("Add members"), Some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::AddMembers))));
                v.push(add.keys(&["participant", "user", "invite"]));
            }
            if c.member_actions & MEMBER_KICK != 0 {
                let kick = cmd(e, tr("Remove member from group"), members(MEMBERS_KICK).filter(|_| can(MANAGE_KICK)));
                v.push(kick.keys(&["delete", "kick", "participant", "user"]));
            }
            if c.member_actions & MEMBER_LEAVE != 0 {
                let leave = cmd(e, tr("Leave chat"), Some(UiMessage::ChatList(chat_list::Msg::AskLeave(c.id.clone()))));
                v.push(leave.keys(&["delete", "leave"]));
            }
            if c.chat_type == "group" && can(MANAGE_ROLE) {
                let keys = &["promote", "demote", "convert"];
                v.push(cmd(e, tr("Make group admin"), members(MEMBERS_ADMIN)).keys(keys));
                v.push(cmd(e, tr("Dismiss group admin"), members(MEMBERS_DISMISS)).keys(keys));
            }
            if crate::can_edit_group(c) && c.chat_type == "group" {
                let edit = (c.is_request != Some(true)).then(|| UiMessage::OpenEditGroup(c.id.clone()));
                let keys =
                    &["change", "edit", "set", "update", "rename", "name", "title", "avatar", "image", "photo", "icon"];
                v.push(cmd(e, tr("Edit group"), edit).keys(keys));
            }
            let info = if self.show_right_panel { tr("Hide chat info") } else { tr("Show chat info") };
            v.push(cmd(e, info, Some(UiMessage::ToggleRightPanel)).hotkey("Toggle Chat Info"));
            let file = self.pane.editing_message_id.is_none().then_some(UiMessage::Composer(composer::Msg::PickFiles));
            let file_keys = &["attachment", "image", "gif", "video", "document", "pdf"];
            v.push(cmd(e, tr("Send file"), file).keys(file_keys).hotkey("Chat: Send File"));
            let gif = (self.pane.editing_message_id.is_none())
                .then_some(UiMessage::Composer(composer::Msg::TogglePicker(crate::Picker::Gif)));
            v.push(cmd(e, tr("Send GIF"), gif).keys(&["attachment", "image", "video"]).hotkey("Message: Send GIF"));
            if st.settings.on("stickerPacks") {
                let sticker = self
                    .pane
                    .editing_message_id
                    .is_none()
                    .then_some(UiMessage::Composer(composer::Msg::TogglePicker(crate::Picker::Sticker)));
                let keys = &["attachment", "image", "video"];
                v.push(cmd(e, tr("Send Sticker"), sticker).keys(keys).hotkey("Message: Send Sticker"));
            }
            v.push(cmd(
                e,
                tr("Export all loaded messages to .txt file"),
                Some(UiMessage::ChatList(chat_list::Msg::Export)),
            ));
            let a = c.account_id.as_str();
            let props = st.settings.account(a);
            let user = match a {
                "hungryserv" => st.me().full_name.clone().unwrap_or_default(),
                _ => crate::account_user(st, a),
            };
            let mute_title = if props.notifications { tr("Mute account: %s") } else { tr("Unmute account: %s") };
            let mute = Some(UiMessage::Accounts(accounts::Msg::SetProps(
                a.into(),
                AccountProps { notifications: !props.notifications, ..props },
            )));
            let category = interned(trf("%s Account", &[&crate::network_label(a)]));
            v.push(cmd(category, mute_title.replacen("%s", &user, 1), mute));
            let later = self.pane.editing_message_id.is_none().then_some(UiMessage::Scheduled(scheduled::Msg::Open));
            let later = cmd(trx("Message", "Command bar category"), tr("Send later"), later);
            v.push(later.keys(&["schedule", "past", "future", "time"]).hotkey("Message: Send Later"));
        }
        let history = cmd(
            CHATS,
            tr("Open recently viewed chats"),
            Some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::History))),
        );
        v.push(history.keys(&["history"]));
        let new_chat = cmd("", tr("Create new chat"), Some(UiMessage::NewChat(new_chat::Msg::Open)));
        v.push(new_chat.keys(&["compose", "create", "person", "chat"]).hotkey("Chat: Start New"));
        let group = UiMessage::Commands(vec![
            UiMessage::NewChat(new_chat::Msg::Open),
            UiMessage::NewChat(new_chat::Msg::NewGroup),
        ]);
        v.push(cmd("", tr("Create new group"), Some(group)).keys(&["compose", "thread", "chat"]));
        let join = UiMessage::Commands(vec![
            UiMessage::NewChat(new_chat::Msg::Open),
            UiMessage::NewChat(new_chat::Msg::ToggleJoinMatrix),
        ]);
        v.push(cmd("", tr("Join a Matrix room"), Some(join)).keys(&["matrix", "room", "join", "address", "alias"]));
        let all = cmd("", tr("Search messages in all chats"), Some(UiMessage::CommandBar(Msg::TogglePalette)));
        v.push(all.keys(&["find", "grep"]).hotkey("Search"));
        if let Some(c) = chat {
            let here = cmd(
                "",
                tr("Search messages in current chat"),
                Some(UiMessage::CommandBar(Msg::SearchRoom(c.id.clone()))),
            );
            v.push(here.keys(&["find", "grep"]).hotkey("Search: Room"));
        }
        let folders = (self.list().active_tab != InboxTab::Requests)
            .then_some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::Folders)));
        v.push(cmd("Filter Chats", tr("Filter by folder type"), folders));
        let filters = (self.list().active_tab != InboxTab::Requests)
            .then_some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::Filters)));
        v.push(cmd("Filter Chats", tr("Filter by type"), filters));
        if crate::rail_accounts(st).len() > 1 {
            let filter = cmd(
                "Filter Chats",
                tr("Filter by account"),
                Some(UiMessage::CommandBar(Msg::OpenSubMenu(SubMenu::FilterAccount))),
            );
            v.push(filter.hotkey("Filter By Account"));
        }
        let requests = self.list().active_tab == InboxTab::Requests;
        let (inbox, tab) = match requests {
            true => (tr("Show default inbox"), InboxTab::AllChats),
            false => (tr("Show message requests"), InboxTab::Requests),
        };
        v.push(cmd("", inbox, Some(UiMessage::ChatList(chat_list::Msg::SelectTab(tab)))).keys(&["dm", "unknown"]));
        let prefs = &["Preferences"];
        let settings = tr("Settings");
        let open =
            |tab, then: UiMessage| Some(UiMessage::Commands(vec![UiMessage::Settings(settings::Msg::Open(tab)), then]));
        // Desktop's `ADD_ACCOUNT` section opens the Add an Account window (`showAddAccountModal`).
        let add = Some(UiMessage::Networks(crate::networks::Msg::Open));
        v.push(cmd(settings, tr("Add Account"), add).keys(prefs));
        use SettingsTab as T;
        let tabs = [T::Accounts, T::BeeperPlus, T::General, T::Ai, T::Appearance, T::Notifications, T::Hotkeys];
        for tab in tabs.into_iter().chain([T::Storage, T::Devices, T::About, T::Help, T::Integrations]) {
            let keys = if tab == T::Integrations { INTEGRATIONS } else { prefs };
            let section = open(tab, UiMessage::Settings(settings::Msg::Pick(SettingsHit::Section(tab, true))));
            v.push(cmd(settings, tab.name(), section).keys(keys));
            let rows = prefs::rows().filter(|(page, r)| *page == tab.meta().0 && !r.label.is_empty() && shown(st, r));
            v.extend(rows.map(|(_, r)| {
                let title = format!("{} → {}", tab.name(), crate::trs(r.label, r.split_args().0));
                let pick = open(tab, UiMessage::Settings(settings::Msg::Pick(SettingsHit::Pref(tab, r))));
                cmd(settings, title, pick).keys(&[r.desc.unwrap_or_default(), "Preferences"])
            }));
        }
        v.extend(crate::networks::addable(&st.features).map(|n| {
            let title = format!("{} → {}", tr("Accounts"), n.display_name);
            cmd(
                settings,
                title,
                open(T::Accounts, UiMessage::Settings(settings::Msg::Pick(SettingsHit::Network(n.display_name)))),
            )
            .keys(prefs)
        }));
        v.push(cmd("", tr("Show settings"), hot("Settings")).keys(prefs).hotkey("Settings"));
        let incognito = st.settings.on("incognito");
        let title = tr(if incognito { "Turn off incognito mode" } else { "Turn on incognito mode" });
        let toggle = st.feature("stealth-mode").on.then_some(UiMessage::ToggleIncognito);
        v.push(cmd(tr("Settings"), title, toggle).keys(&["Preferences", "stealth"]));
        if self.hotkeys_modal.is_none() {
            let shortcuts = cmd("", tr("Show keyboard shortcuts"), hot("Keyboard Shortcuts"));
            v.push(shortcuts.keys(&["hotkeys"]).hotkey("Keyboard Shortcuts"));
        }
        v.push(cmd("", tr("Toggle sidebar"), hot("Toggle Sidebar")).keys(&["hide", "show"]).hotkey("Toggle Sidebar"));
        let space = cmd("", tr("Toggle space bar"), hot("Toggle Space Bar"));
        v.push(space.keys(&["hide", "show"]).hotkey("Toggle Space Bar"));
        let media = &["media", "gallery", "photos", "images", "videos", "files", "links", "docs"];
        v.push(cmd("Window", tr("Media viewer"), Some(UiMessage::Media(media_view::Msg::Open))).keys(media));
        let space = Some(UiMessage::Window(windows::Msg::OpenSpace(self.list().entry())));
        v.push(cmd("Window", tr("Open space in new window"), space).keys(&["pop out", "detach"]));
        v.push(cmd("Window", tr("Maximize/unmaximize"), Some(UiMessage::Window(windows::Msg::ToggleMaximize(None)))));
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
        let select = (self.list().active_tab != InboxTab::Requests)
            .then_some(UiMessage::ChatList(chat_list::Msg::ToggleSelectMode));
        v.push(cmd("UI", tr("Toggle chat selection mode"), select));
        for theme in ThemeChoice::ALL.into_iter().filter(|t| *t != st.settings.theme) {
            let title = match theme {
                ThemeChoice::System => tr("Set theme to match system"),
                ThemeChoice::Light => tr("Set theme to light mode"),
                ThemeChoice::Dark => tr("Set theme to dark mode"),
            };
            v.push(
                cmd("UI", title, Some(UiMessage::Settings(settings::Msg::SelectTheme(theme))))
                    .keys(&["dark", "light", "color"]),
            );
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
            v.push(cmd(
                "Developer",
                "Copy current chat ID",
                Some(UiMessage::Message(message::Msg::CopyText(c.id.clone()))),
            ));
        }
        v.push(cmd("Developer", "Reload app", hot("Reload App")).hotkey("Reload App"));
        v.push(cmd("Developer", "Relaunch app", Some(UiMessage::Window(windows::Msg::Relaunch))));
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
        // Desktop's empty command bar: "No results", and in the command list the hint for people search.
        if cmds.is_empty() && !self.command_palette_query.trim().is_empty() {
            let key = prefs::hotkey_label(st.settings.hotkey_keys("Search").split(", ").next().unwrap_or(""));
            let hint = self.cmd_submenu.is_none().then(|| {
                let text =
                    trf("Are you trying to search for people? Press %s to search all of your conversations", &[&key]);
                row![crate::tinted(&crate::DROP_ARROW, 20.0, p.sub), txt(text, 13.0).color(p.text)]
                    .spacing(16)
                    .align_y(Center)
                    .padding([8, 24])
            });
            let empty =
                column![crate::ic(crate::icon::SEARCH, 24.0).color(p.sub), txt(tr("No results"), 14.0).color(p.sub)]
                    .spacing(8)
                    .align_x(Center);
            return column![].push(hint).push(container(empty).center(Fill)).height(Fill).into();
        }
        let item = |i: usize| {
            let c = &cmds[i];
            let head = ((self.cmd_submenu.is_none() || !c.category.is_empty())
                && (i == 0 || cmds[i - 1].category != c.category))
                .then(|| {
                    let head = semi(if c.category.is_empty() { "Commands" } else { c.category }, 12.0);
                    container(head.line_height(Pixels(rem(15.0))).color(p.sub))
                        .center_y(32)
                        .padding(Padding::ZERO.left(12.0).right(4.0))
                });
            column![].push(head).push(self.view_command(st, c, i == self.cmd_index, i)).into()
        };
        let keys: Vec<u64> = cmds.iter().map(Command::key).collect();
        let list = self.command_list.view(&keys, false, 36.0, UiMessage::Relist, item);
        let list = scrollable(container(list).padding(Padding::ZERO.bottom(6.0))).id(COMMANDS).direction(bar());
        list.height(Fill).style(scroll_style(p.scroll)).into()
    }

    fn view_command(&self, st: &BeeperStore, c: &Command, lit: bool, i: usize) -> El<'_> {
        let p = &self.pal;
        let alpha = if c.msg.is_some() { 1.0 } else { 0.7 };
        let title: El<'_> = match c.chat.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]) {
            Some(chat) => self.command_chat(p, chat),
            None if c.user.is_some() => self.command_user(p, c, lit),
            None if c.render.is_some() => {
                let (left, right) = c.render.clone().unwrap_or_default();
                let words = semi(left, 13.0).color(p.strong);
                let right = semi(right, 13.0).color(p.strong);
                row![words, Space::new().width(Fill), right].width(Fill).into()
            }
            None if c.account.is_some() => {
                let logo =
                    svg(brand_svg(crate::brand_key(c.account.as_deref().unwrap_or_default()))).width(20).height(20);
                let logo = container(logo).padding(Padding::ZERO.left(6.0).right(14.0));
                row![logo, semi(c.title.clone(), 13.0).color(p.strong)].align_y(Center).into()
            }
            None => {
                let title = semi(c.title.clone(), 13.0).color(p.strong.scale_alpha(alpha));
                let sub = c.subtitle.map(|s| txt(s, 13.0).color(p.sub.scale_alpha(alpha)));
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
                let key = semi(k.to_string(), 11.0).color(p.strong.scale_alpha(alpha));
                container(key).center_x(w).center_y(20).style(fill_box(p.strong.scale_alpha(0.1), 4.0)).into()
            }))
            .spacing(4)
        });
        let line =
            row![title].push((c.render.is_none() && c.user.is_none()).then(|| Space::new().width(Fill))).push(caps);
        let line = line.spacing(8).align_y(Center);
        let bg = if lit { p.chip } else { CLEAR };
        let body = container(line).center_y(36).padding([0, 6]).style(fill_box(bg, 8.0));
        let body = container(body).padding([0, 6]).id(format!("cmd-{i}"));
        let hit = mouse_area(body).on_enter(UiMessage::CommandBar(Msg::Highlight(i)));
        let hit = match c.chat.as_ref().filter(|id| st.resolve_chat_idx(id).is_some()) {
            Some(id) => hit.on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone()))),
            None => hit,
        };
        match c.msg.clone() {
            Some(m) => hit.on_press(UiMessage::Commands(vec![UiMessage::CloseModals, m])).into(),
            None => hit.into(),
        }
    }

    /// Desktop's `CommandBarUser`: the avatar, the name, and the username, phone number and email in small text.
    fn command_user<'a>(&'a self, p: &Palette, c: &Command, lit: bool) -> El<'a> {
        let u = c.user.as_ref().expect("a user");
        let name = u.full_name.clone().unwrap_or_else(|| u.id.clone());
        let small = [crate::handle(u), u.phone_number.as_deref().map(crate::fmt_phone), u.email.clone()];
        let small = small.into_iter().flatten().map(|s| txt(s, 12.0).color(p.strong.scale_alpha(0.7)).into());
        let title = row![semi(name.clone(), 13.0).color(p.strong)].extend(small);
        let face = self.face(u.img_url.as_deref(), &name, 28.0, 0.0);
        let right = c.account.as_deref().map(|a| {
            let hint = lit.then(|| {
                let hint = semi(tr("Add to chat"), 11.0).color(p.strong);
                container(hint).padding(Padding::ZERO.right(6.0))
            });
            row![].push(hint).push(svg(brand_svg(crate::brand_key(a))).width(16).height(16)).spacing(8).align_y(Center)
        });
        let main = row![face, title.spacing(8).align_y(Center)].spacing(8).align_y(Center);
        main.push(right.map(|r| row![Space::new().width(Fill), r])).width(Fill).align_y(Center).into()
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

#[derive(Clone)]
pub enum Msg {
    OpenSubMenu(SubMenu),
    NoReply(bool),
    /// Keeps the time phrase for Desktop's "Last Used".
    LastTime(String),
    ChatSub(Option<u8>),
    Highlight(usize),
    TogglePalette,
    SearchRoom(String),
    ToggleMode,
    /// Desktop's `TOGGLE_EXPANDED_MORE` of a search section.
    ToggleSearchExpanded(&'static str),
    /// Desktop's `TOGGLE_PARTIAL_MATCHES` of the Messages section.
    TogglePartialMatches,
    Query(String),
    ExecuteChat(String),
    JumpTo(String, String),
    OpenSearchSelect(usize),
    SearchSelect(Option<(usize, iced::Rectangle)>),
    PickSearchFilter(SearchPick),
}

impl BeeperGuiApp {
    pub(crate) fn command_bar(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::ChatSub(sub) => self.chat_sub = sub,
            Msg::TogglePalette => {
                // Desktop's `reset` empties the input each time the bar closes, so it opens empty.
                if !self.show_command_palette {
                    self.search_filters = SearchFilters::default();
                    self.command_palette_query.clear();
                }
                self.show_command_palette ^= self.search_room.take().is_none() || !self.show_command_palette;
                (self.cmd_bar_commands_mode, self.cmd_index, self.cmd_submenu) = (false, 0, None);
                self.search_expanded.clear();
                self.search_partial = true;
                if self.show_command_palette {
                    return operation::focus("cmd-palette-input");
                }
            }
            Msg::SearchRoom(cid) => {
                // Desktop's `showSearch` keeps the text of an open bar, but resets a submenu, and `reset` emptied a
                // closed bar.
                if !self.show_command_palette || self.cmd_submenu.is_some() {
                    self.command_palette_query.clear();
                }
                (self.show_command_palette, self.cmd_bar_commands_mode, self.cmd_submenu) = (true, false, None);
                (self.search_room, self.list.chat_menu) = (Some(cid), None);
                self.search_filters.accounts.clear();
                return operation::focus("cmd-palette-input");
            }
            Msg::ToggleMode => {
                (self.cmd_bar_commands_mode, self.cmd_index) = (!self.cmd_bar_commands_mode, 0);
                return operation::focus("cmd-palette-input");
            }
            Msg::NoReply(on) => self.cmd_no_reply = on,
            Msg::LastTime(phrase) => self.patch_settings(|s| s.last_time = phrase),
            Msg::OpenSubMenu(menu) => {
                let st = self.store.read();
                self.cmd_no_reply = match &menu {
                    SubMenu::Schedule(ids) => {
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
                (self.search_room, self.list.chat_menu, self.cmd_index) = (None, None, 0);
                self.command_palette_query.clear();
                return operation::focus("cmd-palette-input");
            }
            Msg::ToggleSearchExpanded(section) => {
                if !self.search_expanded.remove(section) {
                    self.search_expanded.insert(section);
                }
            }
            Msg::TogglePartialMatches => self.search_partial = !self.search_partial,
            Msg::Query(q) => {
                (self.command_palette_query, self.cmd_index) = (q.clone(), 0);
                if self.cmd_submenu == Some(SubMenu::AddMembers) {
                    self.new_chat_search = None;
                    let wait = async { tokio::time::sleep(std::time::Duration::from_millis(300)).await };
                    return Task::perform(wait, move |()| UiMessage::Members(members::Msg::Search(q.clone())));
                }
            }
            Msg::Highlight(i) => self.cmd_index = i,
            Msg::JumpTo(cid, mid) => return self.jump_to(cid, mid),
            Msg::ExecuteChat(cid) => {
                self.show_command_palette = false;
                self.command_palette_query.clear();
                self.select_chat(&cid);
            }
            Msg::OpenSearchSelect(chip) => {
                let find = Find(format!("search-chip-{chip}").into(), None);
                return iced::advanced::widget::operate(find)
                    .map(move |r| UiMessage::CommandBar(Msg::SearchSelect(Some((chip, r)))));
            }
            Msg::SearchSelect(open) => self.search_select = open,
            Msg::PickSearchFilter(pick) => {
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
        }
        self.settle()
    }
}

impl BeeperGuiApp {}

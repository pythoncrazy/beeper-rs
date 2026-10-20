//! Desktop's command bar commands (`Efe`): the categories, the titles, the search keywords and the hotkeys.
use crate::i18n::{tr, trx};
use crate::style::{bar, brand_svg, ell, fill_box, scroll_style, semi, text_width, txt, Palette, CLEAR};
use crate::{has_flag, network_name, BeeperGuiApp, El, InboxTab, UiMessage, ASK_LOGOUT};
use crate::{ASK_ARCHIVE_ALL, ASK_ARCHIVE_READ, ASK_ARCHIVE_WEEK, ASK_MARK_ALL_READ};
use beeper_core::matrix::ChatFlag;
use beeper_core::models::{Chat, ThemeChoice, CHAT_UNREAD};
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

/// A command bar submenu of Desktop (`showWithSubMenu`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubMenu {
    FilterAccount,
}

impl SubMenu {
    pub(crate) fn heading(self) -> &'static str {
        match self {
            SubMenu::FilterAccount => tr("Filter account"),
        }
    }
}

impl BeeperGuiApp {
    /// Desktop's `zXe`: all accounts, then each account with its network icon.
    fn submenu_commands(&self, st: &BeeperStore, menu: SubMenu) -> Vec<Command> {
        match menu {
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
                std::iter::once(Command { digit: Some(1), ..all.hotkey(SPACES) }).chain(accounts).collect()
            }
        }
    }

    /// The commands in Desktop's order, or the ones that match `query` with the best match first.
    pub(crate) fn commands(&self, st: &BeeperStore, query: &str) -> Vec<Command> {
        if let Some(menu) = self.cmd_submenu {
            let v = self.submenu_commands(st, menu);
            return match query.is_empty() {
                true => v,
                false => prefs::match_sort_keys(v, query, |c| {
                    std::iter::once(c.title.clone()).chain(c.keywords.iter().map(|k| k.to_string())).collect()
                }),
            };
        }
        let hot = |name: &'static str| Some(UiMessage::RunHotkey(name));
        let chat = self.pane.selected_chat_id.as_deref().and_then(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]);
        let chat = chat.filter(|c| !c.id.starts_with("draft:") && !c.id.starts_with("account:"));
        let mut v = Vec::new();
        if let Some(c) = chat {
            let id = || c.id.clone();
            let e = trx("Chat", "Command bar category");
            let flag = |f| Some(UiMessage::ToggleFlag(id(), f));
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
        }
        if chat.is_some() {
            let e = "Chat";
            let info = if self.show_right_panel { tr("Hide chat info") } else { tr("Show chat info") };
            v.push(cmd(e, info, Some(UiMessage::ToggleRightPanel)).hotkey("Toggle Chat Info"));
            let gif = (self.pane.editing_message_id.is_none()).then_some(UiMessage::ToggleGifPicker);
            v.push(cmd(e, tr("Send GIF"), gif).keys(&["attachment", "image", "video"]).hotkey("Message: Send GIF"));
        }
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
        let requests = self.active_tab == InboxTab::Requests;
        let (inbox, tab) = match requests {
            true => (tr("Show default inbox"), InboxTab::AllChats),
            false => (tr("Show message requests"), InboxTab::Requests),
        };
        v.push(cmd("", inbox, Some(UiMessage::SelectTab(tab))).keys(&["dm", "unknown"]));
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
        let line = row![title, Space::new().width(Fill)].push(caps).spacing(8).align_y(Center);
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

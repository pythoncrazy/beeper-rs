//! The members of a chat in Chat Info: Add members, admins, removal, and the direct chat with a member.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// Desktop's member "Send Message": the direct chat with a member of a chat, as (chat, user).
    Message(String, String),
    /// Opens a member confirmation, `ASK_MAKE_ADMIN` or `ASK_REMOVE_MEMBER`, for (chat, user).
    Ask(u8, String, String),
    /// Desktop's Add Members button of Chat Info: the Add members submenu with the members query.
    OpenAdd(String),
    /// Desktop's `addParticipant`: invites the user to the chat.
    Add(String, Box<User>),
    /// Desktop's `searchCmdBarUsers` for the Add members query, after the user stops typing.
    Search(String),
    /// Makes a member of a chat an admin (true) or a regular member, as (chat, user, admin).
    SetAdmin(String, String, bool),
    Remove(String, String),
    Toggle(String),
}

impl BeeperGuiApp {
    pub(crate) fn members(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Search(q) => {
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
            Msg::OpenAdd(q) => {
                let open =
                    self.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::AddMembers)));
                return Task::batch([open, self.update(UiMessage::CommandBar(commands::Msg::Query(q)))]);
            }
            Msg::Add(chat, user) => {
                let mut st = self.store.write();
                // The second click of a double click hits the row of the person that the first click added.
                if st.chat(&chat).participants.items.iter().all(|u| u.id != user.id) {
                    st.add_member(&chat, *user);
                }
            }
            Msg::Message(cid, uid) => {
                self.list.chat_menu = None;
                self.user_hover = UserHover { r#gen: self.user_hover.r#gen + 1, ..UserHover::default() };
                let st = self.store.read();
                let c = st.chat(&cid);
                let user = c.participants.items.iter().find(|u| u.id == uid).cloned().expect("a member of the chat");
                let account = c.account_id.clone();
                drop(st);
                return self.open_dm(account, user, None, None);
            }
            Msg::Ask(ask, cid, uid) => {
                (self.list.chat_menu, self.confirm, self.delete_target) =
                    (None, Some(ask), Some(format!("{cid} {uid}")));
            }
            Msg::SetAdmin(cid, uid, admin) => {
                self.confirm = None;
                self.store.write().set_admin(&cid, &uid, admin);
            }
            Msg::Remove(cid, uid) => {
                self.confirm = None;
                self.store.write().remove_member(&cid, &uid);
            }
            Msg::Toggle(id) => {
                if self.pane.open_members.insert(id.clone()) {
                    return self.settle();
                }
                self.pane.open_members.remove(&id);
                self.pane.auto_members.take_if(|a| *a == id);
                // Desktop's `MembershipGrouper` unselects the events of a group that closes.
                let st = self.store.read();
                let (msgs, _) = self.timeline(&st);
                let runs = message::member_runs(&msgs);
                let start = msgs.iter().position(|m| m.id == id);
                let group = start
                    .map_or(0..0, |s| s..s + msgs[s..].iter().zip(&runs[s..]).take_while(|(_, &r)| r == s).count());
                let ids: Vec<String> = msgs[group].iter().map(|m| m.id.clone()).collect();
                drop(st);
                self.pane.selected_msgs.retain(|m| !ids.contains(m));
            }
        }
        self.settle()
    }
}

impl BeeperGuiApp {
    /// Desktop's `YNt` Members section of a group, with its first six members and a search above more than six.
    pub(crate) fn view_members<'a>(&'a self, chat: &Chat) -> El<'a> {
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
                .on_press(UiMessage::Members(members::Msg::OpenAdd(self.info_query.clone())))
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
            let label = row![ell(&name, 224.0 - tag_w - you_w, 13.0, Weight::Semibold)
                .line_height(Pixels(rem(16.0)))
                .color(strong)]
            .push(you.map(|y| txt(y, 13.0).color(strong)));
            let tags = tags.into_iter().map(|t| -> El<'a> {
                container(crate::style::t(t, 10.0, Weight::Semibold).line_height(Pixels(rem(12.0))).color(p.strong))
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
            let menu = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("member:{} {}", chat.id, u.id)));
            let card = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("card:{} {}", chat.id, u.id)));
            let btn = button(line).padding(4).width(Fill).style(btn_style([CLEAR, shade, shade], [p.strong; 3], 8.0));
            mouse_area(btn.on_press(card)).on_right_press(menu).into()
        };
        let search = searchable.then(|| {
            let n = all.len();
            let input =
                text_input(&trn("Search %d group member", "Search %d group members", n, &[&n]), &self.info_query)
                    .on_input(|a0| UiMessage::Info(info::Msg::Query(a0)))
                    .size(rem(13.0))
                    .line_height(Pixels(rem(16.0)))
                    .padding([10, 12])
                    .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
            let clear = (!self.info_query.is_empty()).then(|| {
                button(ic(icon::CLOSE, 20.0).color(p.sub))
                    .bare(p.sub)
                    .on_press(UiMessage::Info(info::Msg::Query(String::new())))
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
                .on_press(UiMessage::Info(info::Msg::All))
        });
        let list = Column::new()
            .push(search)
            .push(add)
            .extend(rows[..shown].iter().map(|u| row(u)))
            .push(toggle)
            .spacing(6)
            .padding(if searchable { Padding::new(8.0).top(0.0) } else { Padding::new(8.0) });
        let less = (expandable && self.info_all).then(|| {
            button(semi(tr("Show less"), 11.0).color(p.accent)).bare(p.accent).on_press(UiMessage::Info(info::Msg::All))
        });
        let head = row![txt(tr("Members"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(less)
            .align_y(Center)
            .padding(8);
        let head: El = match expandable {
            true => mouse_area(head).on_press(UiMessage::Info(info::Msg::All)).into(),
            false => head.into(),
        };
        column![head, container(list).style(fill_box(p.chip, 10.0))].into()
    }
}

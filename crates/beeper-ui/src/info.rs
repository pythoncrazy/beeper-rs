//! Desktop's Chat Info panel: the description, the note, the media and bookmarks lists, the personal name and disappearing messages.

use super::*;
use crate::prelude::*;
use crate::time::timer_label;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `setPersonalRoomName`, which Undo can revert.
    SetPersonalName(String, Option<String>),
    /// Starts editing the Chat Info group description, or edits it.
    Desc(Option<text_editor::Action>),
    SaveDesc,
    More,
    All,
    Bookmarks,
    /// Desktop's "Remove Bookmark" in Chat Info, for the event ID of a message that may not be loaded.
    RemoveBookmark(String),
    /// A bookmarked message that is not loaded, or None when the server does not give it.
    BookmarkHydrated(String, Option<Box<Message>>),
    /// The end of a restoration of the bookmarks room that Chat Info started.
    BookmarksRestored,
    Media(Option<usize>),
    Query(String),
    Note(text_editor::Action),
    /// Desktop's disappearing messages select: a chat and a timer in ms, or 0 for Off.
    SetDisappearing(String, u64),
    /// Desktop's "Notification sound" select: a chat and a sound of `CHAT_SOUNDS`, or `None` for Default.
    SetSound(String, Option<String>),
    /// Opens Desktop's How It Works window of the behavior menu with `true`, and its Done button closes it.
    HowItWorks(bool),
    /// Desktop's `$8t`: a click on the Chat Info avatar zooms it to full screen, and a click on it there zooms it out.
    Zoom,
}

impl BeeperGuiApp {
    pub(crate) fn info(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Desc(None) => {
                let st = self.store.read();
                let cid = self.pane.selected_chat_id.as_deref().expect("a chat is selected");
                let c = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
                self.info_desc = Some(composer_with(c.description.as_deref().unwrap_or_default()));
                return operation::focus("info-desc");
            }
            Msg::Desc(Some(action)) => self.info_desc.as_mut().expect("editing").perform(action),
            Msg::SaveDesc => {
                let text = self.info_desc.take().expect("editing").text();
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                self.store.write().update_group(&cid, None, Some(text), None);
                return self.update(UiMessage::StoreChanged);
            }
            Msg::SetSound(chat, sound) => self.store.write().set_notification_sound(&chat, sound),
            Msg::SetDisappearing(cid, timer) => {
                self.store.write().set_disappearing(&cid, timer);
                return self.update(UiMessage::StoreChanged);
            }
            Msg::More => self.info_more ^= true,
            Msg::All => self.info_all ^= true,
            Msg::Bookmarks => self.info_bookmarks ^= true,
            Msg::RemoveBookmark(target) => {
                self.list.chat_menu = None;
                self.store.write().remove_bookmark(&target);
            }
            Msg::BookmarkHydrated(target, m) => {
                self.hydrating.remove(&target);
                self.store.write().bookmarked.insert(target, m.map(|m| *m));
            }
            Msg::BookmarksRestored => self.restoring_bookmarks = false,
            Msg::Media(tab) => self.info_media = tab,
            Msg::Zoom => self.info_zoom ^= true,
            Msg::HowItWorks(true) => {
                self.list.chat_menu = None;
                if let Some(id) = self.how_window {
                    return self.raise(id);
                }
                let (id, open) = self.open_sheet(behavior::HOW_SIZE);
                self.how_window = Some(id);
                return open;
            }
            Msg::HowItWorks(false) => return self.close_window(self.how_window.expect("How It Works is open")),
            Msg::Note(action) => {
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
            Msg::Query(q) => (self.info_all, self.info_query) = (!q.trim().is_empty(), q),
            Msg::SetPersonalName(chat, name) => self.set_personal_name(chat, name),
        }
        self.settle()
    }

    /// Desktop's Bookmarks section calls `restore()` when it shows: Chat Info that opens after a failed restoration
    /// reads the bookmarks room again.
    pub(crate) fn retry_bookmarks(&mut self) -> Task<UiMessage> {
        let failed = {
            let st = self.store.read();
            st.bookmarks_failed && st.matrix.is_some()
        };
        if !self.show_right_panel || !failed || self.restoring_bookmarks {
            return Task::none();
        }
        self.restoring_bookmarks = true;
        let restore = beeper_core::matrix::restore_bookmarks(self.store.clone());
        Task::perform(restore, |()| UiMessage::Info(info::Msg::BookmarksRestored))
    }
}

/// The Chat Info menus. The header menus of Desktop's `$fe`: `info:<chat>` has the edit actions and
/// `copy:<chat>`, the menu of the clicked title, adds "Copy <title>" above them. `copy:<chat>\n<text>` copies the
/// clicked line instead.
pub(crate) fn info_menu(st: &BeeperStore, key: &str, dm: impl Fn(&str) -> bool) -> Vec<Option<(String, Option<UiMessage>)>> {
    // Desktop's `hj` and `pj` menus of a phone number or an email of a contact card.
    if let Some((kind, v)) = key.strip_prefix("vcard:").and_then(|k| k.split_once(' ')) {
        let item = |label: String, msg| Some((label, Some(msg)));
        return match kind {
            "phone" => {
                let shown = fmt_phone(v);
                vec![
                    item(format!("Call {shown}"), UiMessage::OpenUrl(format!("tel:{}", phone_digits(v)))),
                    item(format!("Copy {shown}"), UiMessage::Message(message::Msg::CopyText(shown))),
                ]
            }
            _ => vec![
                item(format!("Email {v}"), UiMessage::OpenUrl(format!("mailto:{v}"))),
                item(format!("Copy {v}"), UiMessage::Message(message::Msg::CopyText(v.into()))),
            ],
        };
    }
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
                    item(format!("Copy {shown}"), UiMessage::Message(message::Msg::CopyText(shown))),
                ]
            }
            "email" => {
                let e = u.email.clone().expect("an email");
                vec![
                    item(format!("Email {e}"), UiMessage::OpenUrl(format!("mailto:{e}"))),
                    item(format!("Copy {e}"), UiMessage::Message(message::Msg::CopyText(e))),
                ]
            }
            _ => vec![item(tr("Copy Username").into(), UiMessage::Message(message::Msg::CopyText(handle(u).expect("a username"))))],
        };
    }
    // Desktop's Manage menu of the Merged Chats section (`R7t`).
    if let Some(cid) = key.strip_prefix("merged:") {
        let edit = UiMessage::Merged(merged::Msg::Open(Some(cid.into()), Vec::new()));
        let unmerge = UiMessage::Merged(merged::Msg::Ask(cid.into()));
        return vec![
            Some((trx("Edit", "merged chat manage menu").into(), Some(edit))),
            None,
            Some((trx("Unmerge", "merged chat manage menu").into(), Some(unmerge))),
        ];
    }
    if let Some((cid, mid)) = key.strip_prefix("bookmark:").and_then(|k| k.split_once(' ')) {
        let jump = st.loaded(mid).is_some().then(|| UiMessage::CommandBar(commands::Msg::JumpTo(cid.into(), mid.into())));
        // Desktop's "Remove Bookmark" waits for the restoration of the bookmarks.
        let remove = st.bookmarks_restored().then(|| UiMessage::Info(info::Msg::RemoveBookmark(mid.into())));
        return vec![Some((tr("Jump to Message").into(), jump)), None, Some((tr("Remove Bookmark").into(), remove))];
    }
    // Desktop's `b2t` menu of a mention: message the person, and open their profile on the network.
    if let Some((cid, uid)) = key.strip_prefix("mention:").and_then(|k| k.split_once(' ')) {
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let u = c.participants.items.iter().find(|u| u.id == uid);
        let username = u.and_then(|u| u.username.as_deref()).map(|n| n.trim_start_matches('@'));
        let name = username.or(u.and_then(|u| u.full_name.as_deref())).unwrap_or(uid);
        let send = u.map(|_| UiMessage::Members(members::Msg::Message(cid.into(), uid.into())));
        let network = crate::network_name(&c.account_id);
        let open = u.and_then(|u| crate::user_profile_link(&c.account_id, u)).map(|url| {
            let label = match username {
                Some(n) => trf("Open %1$s on %2$s", &[&format!("@{n}"), &network]),
                None => trf("Open in %s", &[&network]),
            };
            Some((label, Some(UiMessage::OpenUrl(url))))
        });
        return [Some(Some((format!("Message {name}"), send))), open].into_iter().flatten().collect();
    }
    if let Some((cid, uid)) = key.strip_prefix("member:").and_then(|k| k.split_once(' ')) {
        let c = &st.chats[st.resolve_chat_idx(cid).expect("chat exists")];
        let u = c.participants.items.iter().find(|u| u.id == uid).expect("a member of the chat");
        let copy = |label: &'static str, text: Option<String>| {
            text.map(|t| Some((tr(label).to_string(), Some(UiMessage::Message(message::Msg::CopyText(t))))))
        };
        let username = handle(u);
        let call = u
            .phone_number
            .as_ref()
            .map(|n| Some((trx("Call", "button label to start a phone call").to_string(), Some(UiMessage::OpenUrl(format!("tel:{n}"))))));
        let send = dm(&c.account_id)
            .then(|| Some((tr("Send Message").to_string(), Some(UiMessage::Members(members::Msg::Message(cid.into(), uid.into()))))));
        let ask = |label: &'static str, ask, on: bool| {
            Some((tr(label).to_string(), on.then(|| UiMessage::Members(members::Msg::Ask(ask, cid.into(), uid.into())))))
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
    let group = can_edit_group(c)
        .then(|| (tr("Edit Group").to_string(), (c.is_request != Some(true)).then(|| UiMessage::OpenEditGroup(c.id.clone()))));
    let personal = can_edit_for_me(st, c).then(|| (tr("Edit for Me").to_string(), Some(UiMessage::OpenEditForMe(c.id.clone()))));
    let theme = Some((tr("Chat Theme").to_string(), Some(UiMessage::ChatTheme(chat_theme::Msg::Open(c.id.clone())))));
    let text = text.unwrap_or(&c.title);
    let title = copy.then(|| Some((trf("Copy %s", &[&text]), Some(UiMessage::Message(message::Msg::CopyText(text.to_string()))))));
    let edits = [group, personal, theme].into_iter().flatten().map(Some);
    title.into_iter().chain(copy.then_some(None)).chain(edits).collect()
}

impl BeeperGuiApp {
    pub(crate) fn view_info(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.clone().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(&cid).expect("selected chat exists")];
        let pane_head = self.view_info_pane_head(chat, &cid);
        let other = chat.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| chat.chat_type == "single");
        let header = self.view_info_header(&st, chat, &cid, other);
        let behavior = self.view_behavior_line(&st, chat);
        let desc = self.view_info_desc(chat);
        let toolbar = self.view_info_toolbar(&st, chat, &cid);
        let members = (chat.chat_type == "group").then(|| self.view_members(chat));
        let media = self.view_info_media(&st, chat);
        let bookmarks = self.view_bookmarks(&st, chat);
        let labels = self.view_info_labels(&st, chat, &cid);
        let timers = self.view_info_timers(&st, chat, &cid);
        let sound = self.view_info_sound(chat, &cid);
        let [block, leave] = self.view_info_actions(&st, chat, &cid);
        let note = text_editor(&self.info_note)
            .placeholder("Notes")
            .on_action(|a0| UiMessage::Info(info::Msg::Note(a0)))
            .size(rem(13.0))
            .line_height(lh(16.0))
            .padding([8, 11])
            .min_height(128)
            .max_height(self.pane().size.height / 2.0)
            .style(info_editor(p, 10.0));
        let body = column![header]
            .push(behavior)
            .push(self.view_merge_promo(&st, chat))
            .push(self.view_merged_section(&st, chat))
            .push(desc)
            .push(members)
            .push(media)
            .push(bookmarks)
            .push(labels)
            .push(self.view_merge_card(&st, chat))
            .push(other.and_then(|u| self.view_contact(&st, chat, u)))
            .push(timers)
            .push(sound)
            .push(container(note).padding(Padding::ZERO.top(8.0)))
            .push(block)
            .push(leave)
            .padding(Padding::new(8.0).top(0.0));
        let scroll = scrollable(body).direction(bar()).style(scroll_style(p.scroll)).height(Fill);
        let card = |body| container(body).height(Fill).style(fill_box(p.card, 10.0));
        match self.pane().info {
            // Desktop's `thread-info--detached` fills its `--left-pane-bg` window inside 8 px: the rounded card is
            // `thread-info-inner`, and the toolbar, its sibling, is a square strip under it.
            true => {
                let body = column![card(column![pane_head, scroll]).width(Fill), container(toolbar).style(fill_box(p.card, 0.0))];
                crate::ui_theme::window_bg(container(body).padding(8), p.backdrop, p.window).into()
            }
            false => {
                container(card(column![pane_head, scroll, toolbar]).width(self.info_width())).padding(Padding::new(8.0).left(0.0)).into()
            }
        }
    }

    /// Desktop's `BrandChatInfoBlockUser` row and the Leave Chat button at the end of Chat Info.
    fn view_info_actions(&self, st: &BeeperStore, chat: &Chat, cid: &str) -> [Option<El<'_>>; 2] {
        let p = &self.pal;
        let block = block_action(st, chat).map(|block| {
            let (label, fg) = match block {
                true => (trx("Block", "action to block the other person in a DM"), p.text_danger),
                false => (trx("Unblock", "action to unblock the other person in a DM"), p.accent),
            };
            let row = button(txt(label, 13.0).color(fg))
                .padding([8, 11])
                .width(Fill)
                .style(btn_style([p.chip; 3], [fg; 3], 10.0))
                .on_press(UiMessage::Blocking(blocking::Msg::Ask(cid.into(), block)));
            container(row).padding(Padding::ZERO.top(8.0)).into()
        });
        // A merged chat has no Leave Chat (`F0e`); Unmerge splits it.
        let leave = (chat.member_actions & MEMBER_LEAVE != 0 && chat.merge.is_none()).then(|| {
            // `.leave-chat-button`, a quaternary `xs` button: 8 px inside its 1 px clear border.
            let leave = button(semi(tr("Leave Chat"), 14.0).line_height(lh(18.0)).color(p.strong))
                .bare(p.strong)
                .padding([1, 9])
                .on_press(UiMessage::ChatList(chat_list::Msg::AskLeave(cid.into())));
            container(leave).padding(Padding::ZERO.top(8.0)).into()
        });
        [block, leave]
    }

    /// The top row of Chat Info: Mute Chat and Pin Chat, and Close Chat Info.
    fn view_info_pane_head(&self, chat: &Chat, cid: &str) -> El<'_> {
        let p = &self.pal;
        let flag = |f: ChatFlag| UiMessage::ChatList(chat_list::Msg::ToggleFlag(cid.into(), f));
        let pane_btn = |glyph: char, pressed: bool, label: &'static str, msg: Option<UiMessage>| -> El<'_> {
            let (bg, fg) = if pressed { (p.accent, Color::WHITE) } else { (CLEAR, p.strong) };
            let btn = button(container(ic(glyph, 16.0)).center(28))
                .padding(0)
                .style(btn_style([bg, if pressed { bg } else { p.chip }, bg], [fg; 3], 8.0))
                .on_press_maybe(msg);
            self.tip(btn.into(), label, vec![], tooltip::Position::Bottom).into()
        };
        let locked = chat.is_request == Some(true);
        row![
            pane_btn(icon::NOTIFICATIONS_OFF, chat.flag(ChatFlag::Muted), tr("Mute Chat"), (!locked).then(|| flag(ChatFlag::Muted))),
            pane_btn(icon::KEEP, chat.flag(ChatFlag::Pinned), tr("Pin Chat"), Some(flag(ChatFlag::Pinned))),
            Space::new().width(Fill),
            pane_btn(icon::CLOSE, false, tr("Close Chat Info"), Some(UiMessage::ToggleRightPanel)),
        ]
        .spacing(4)
        .padding([0, 4])
        .height(36)
        .align_y(Center)
        .into()
    }

    /// The head of Chat Info: the avatar, the title, the original title, the handles and the network of the chat, with
    /// the Edit button.
    fn view_info_header(&self, st: &BeeperStore, chat: &Chat, cid: &str, other: Option<&User>) -> El<'_> {
        let p = &self.pal;
        let edits = info_menu(st, &format!("info:{cid}"), |a| self.can(a, "create_dm"));
        let edit = match &edits[..] {
            [Some((_, msg))] => msg.clone(),
            _ => {
                edits.iter().flatten().any(|i| i.1.is_some()).then(|| UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("info:{cid}"))))
            }
        };
        let edit = section_btn(p, trx("Edit", "chat info edit button"), p.accent).on_press_maybe(edit);
        let copy = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("copy:{cid}")));
        let title = mouse_area(
            button(semi(chat.title.clone(), 15.0).line_height(lh(20.0)).color(p.strong).center()).bare(p.strong).on_press(copy.clone()),
        )
        .on_right_press(copy);
        let meta = |s: String| self.info_meta(cid, s);
        // Desktop's identity `metadata` of a DM that is not merged: the inline username on the line of the 13 px
        // `div`, then the phone number and the email in their own `div`s on 14 px lines, so each starts a line. A
        // click on either opens its `PhoneNumber` or `EmailAddress` menu. The `.phone` div is a flex row, so its number
        // starts at the left of the block.
        let contact = other.filter(|_| chat.merge.is_none()).map(|u| {
            let lines = [(handle(u), ""), (u.phone_number.as_deref().map(fmt_phone), "phone"), (u.email.clone(), "email")];
            let lines: Vec<(String, &str)> = lines.into_iter().filter_map(|(s, kind)| Some((s?, kind))).collect();
            let wide = lines.iter().map(|(s, _)| text_width(s, 11.0, Weight::Normal)).fold(0.0, f32::max).ceil();
            Column::with_children(lines.into_iter().map(|(s, kind)| -> El<'_> {
                let line = meta(s);
                match kind {
                    "" => meta_line(line),
                    _ => {
                        let menu = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("contact:{cid} {kind}")));
                        let line = line.on_press(menu).interaction(mouse::Interaction::Pointer);
                        let left = if kind == "phone" { iced::Alignment::Start } else { Center };
                        container(line).width(wide).align_x(left).into()
                    }
                }
            }))
            .align_x(Center)
        });
        let originally = chat
            .personal
            .as_ref()
            .filter(|p| p.title.as_ref().is_some_and(|t| *t != p.room_title) && !p.room_title.is_empty())
            .map(|p| meta_line(meta(trf("Originally %s", &[&p.room_title]))));
        let network = self.view_info_network(st, chat, cid, other);
        // Desktop's `.thread-info-avatar img` has the `zoom-in` cursor.
        let group = chat.chat_type == "group";
        let image = chat.img_url.is_some() || faces_of(&chat.participants.items, group).0.iter().any(|u| u.img_url.is_some());
        let avatar = mouse_area(self.chat_avatar(chat, 64.0, 1.25)).on_press(UiMessage::Info(info::Msg::Zoom));
        let avatar = if image { avatar.interaction(mouse::Interaction::ZoomIn) } else { avatar };
        let details = column![avatar, title].push(originally).push(contact).push(network).spacing(4).align_x(Center).width(Fill);
        let can_desc = chat.edits & 1 << 2 != 0;
        let adding = can_desc && group && self.info_desc.is_none() && chat.description.is_none();
        mouse_area(stack![
            container(details).padding(Padding::new(16.0).bottom(if adding { 0.0 } else { 8.0 })),
            container(edit).align_right(Fill).padding(Padding::ZERO.top(12.0).right(8.0)),
        ])
        .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("info:{cid}"))))
        .into()
    }

    /// A `metadata` row of Chat Info, whose right click copies it.
    fn info_meta(&self, cid: &str, s: String) -> iced::widget::MouseArea<'_, UiMessage> {
        let copy = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("copy:{cid}\n{s}")));
        mouse_area(txt(s, 11.0).color(Color { a: 0.6, ..self.pal.strong })).on_right_press(copy)
    }

    /// The network of the chat in Chat Info. Desktop's `profileLink`: the network of a DM with a profile URL opens the
    /// profile.
    fn view_info_network(&self, st: &BeeperStore, chat: &Chat, cid: &str, other: Option<&User>) -> El<'_> {
        let network = merged::chat_platform(chat);
        let Some(url) = profile_link(chat) else { return meta_line(self.info_meta(cid, network.to_string())) };
        let fg = Color { a: 0.6, ..self.pal.strong };
        let label = row![txt(network, 11.0).color(fg), tinted(&EXTERNAL_LINK, 8.0, fg)].spacing(4).align_y(Center);
        let label = meta_line(label);
        let open = button(label).bare(fg).on_press(UiMessage::OpenUrl(url));
        let tip = match other.and_then(|u| u.username.as_deref()) {
            Some(u) => trf("Open @%1$s on %2$s", &[&u.trim_start_matches('@'), &network]),
            None => trf("Open in %s", &[&network]),
        };
        let keys = hotkey_caps(st.settings.hotkey_keys("Open User Profile"));
        self.tip_caps(open.into(), tip, keys, tooltip::Position::Bottom).into()
    }

    /// Desktop's group description: its editor, an "Add a description" link, or the description with its Edit and
    /// "more" links.
    fn view_info_desc(&self, chat: &Chat) -> Option<El<'_>> {
        let p = &self.pal;
        let can_desc = chat.edits & 1 << 2 != 0;
        let link = |label: &'static str, msg: UiMessage| section_btn(p, label, p.accent).on_press(msg);
        let (btn, body): (_, El) = match (&self.info_desc, chat.description.as_deref()) {
            _ if chat.chat_type != "group" => return None,
            (Some(content), _) => {
                let done = trx("Done", "group description submit button");
                let input = text_editor(content)
                    .id("info-desc")
                    .placeholder(tr("This description is public to the group"))
                    .size(rem(11.0))
                    .line_height(lh(14.0))
                    .padding(8)
                    .style(info_editor(p, 8.0));
                let input = if can_desc { input.on_action(|a| UiMessage::Info(info::Msg::Desc(Some(a)))) } else { input };
                (link(done, UiMessage::Info(info::Msg::SaveDesc)), input.into())
            }
            (None, None) if can_desc => {
                let add = link(tr("Add a description"), UiMessage::Info(info::Msg::Desc(None)));
                return Some(container(add).center_x(Fill).padding(Padding::ZERO.bottom(8.0)).into());
            }
            (None, Some(d)) => {
                // Desktop's `description-toggle` ends the last line of the clamped text, at its right.
                let label = trx("more", "expand group description button");
                let (text, more) = self.view_desc_text(d, text_width(label, 11.0, Weight::Normal) + 4.0);
                let edit = trx("Edit", "group description edit button");
                let more = more.then(|| button(txt(label, 11.0).color(p.accent)).bare(p.accent).on_press(UiMessage::Info(info::Msg::More)));
                let body = row![text].push(more).spacing(4).align_y(iced::Bottom);
                let body = container(body).padding(8).width(Fill).style(fill_box(p.chip, 8.0));
                (link(edit, UiMessage::Info(info::Msg::Desc(None))), body.into())
            }
            (None, None) => return None,
        };
        let head = row![txt(tr("Group Description"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(can_desc.then_some(btn))
            .padding([0, 8])
            .align_y(Center);
        Some(column![head, body].spacing(8).padding(Padding::ZERO.top(8.0)).into())
    }

    /// The text of the group description with its links, clamped to three lines until "more", and whether it is
    /// longer than three lines. Next to "more", which is `toggle` px wide with its gap, the lines are narrower.
    fn view_desc_text(&self, d: &str, toggle: f32) -> (El<'_>, bool) {
        let p = &self.pal;
        let full = d.split_whitespace().collect::<Vec<_>>().join(" ");
        let clamp = |w| crate::style::clamped(&full, w, 3, 11.0, Weight::Normal);
        let more = *clamp(268.0) != *full;
        let shown = if self.info_more { full.clone() } else { clamp(268.0 - if more { toggle } else { 0.0 }).to_string() };
        let spans = shown.split_inclusive(' ').map(|w| match text_urls(w).first() {
            Some(url) => span(w.to_string()).color(p.accent).link(url.clone()),
            None => span(w.to_string()).color(p.strong),
        });
        let text = rich_text(spans.collect::<Vec<_>>())
            .size(rem(11.0))
            .line_height(lh(14.0))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Fill)
            .on_link_click(UiMessage::OpenUrl);
        (text.into(), more)
    }

    /// The toolbar at the bottom of Chat Info: Archive, the reminder, Snooze, Search and More Options.
    fn view_info_toolbar(&self, st: &BeeperStore, chat: &Chat, cid: &str) -> El<'_> {
        let p = &self.pal;
        let locked = chat.is_request == Some(true);
        let archived = chat.flag(ChatFlag::Archived);
        let pinned = !archived && chat.flag(ChatFlag::Pinned);
        let keys = |name| hotkey_caps(st.settings.hotkey_keys(name));
        let tool = |glyph: char, fg: Color, label: &'static str, keys, msg: Option<UiMessage>| -> El<'_> {
            let style = move |_: &Theme, s: button::Status| button::Style {
                text_color: if matches!(s, button::Status::Disabled) { fg.scale_alpha(0.2) } else { fg },
                ..Default::default()
            };
            let btn = button(container(ic(glyph, 20.0)).center_x(Fill)).padding(6).width(Fill).style(style);
            let tip = self.tip_caps(btn.on_press_maybe(msg).into(), label, keys, tooltip::Position::Top);
            container(tip).center_x(Fill).into()
        };
        let label = match (pinned, archived) {
            (true, _) => tr("Pinned chats can't be archived"),
            (false, true) => trx("Unarchive", "button label to unarchive chat"),
            (false, false) => trx("Archive", "button label to archive chat"),
        };
        let toggle = UiMessage::ChatList(chat_list::Msg::ToggleFlag(cid.into(), ChatFlag::Archived));
        let glyph = if archived { icon::INBOX } else { icon::ARCHIVE };
        let archive = tool(glyph, p.strong, label, keys("Chat: Toggle Archive"), (!pinned && !locked).then_some(toggle));
        // Desktop's `hasReminder` and `dismissThreadReminder`: the soonest of the message reminders and the chat
        // reminder is dismissed.
        let (fg, label, remind) = match (self.soonest_message_reminder(st, chat), chat.reminder.is_some()) {
            (Some(mid), _) => (ORANGE, tr("Dismiss Reminder"), UiMessage::Reminders(reminders::Msg::Message(mid, None))),
            (None, true) => (ORANGE, tr("Dismiss Reminder"), UiMessage::Reminders(reminders::Msg::Set(cid.into(), None))),
            (None, false) => (p.strong, tr("Remind Me"), UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.into()))),
        };
        let remind = tool(icon::SCHEDULE, fg, label, keys("Remind Later"), Some(remind));
        let snooze = self.view_info_snooze(st, chat, cid);
        let search = Some(UiMessage::CommandBar(commands::Msg::Search(Some(cid.into()))));
        let search = tool(icon::SEARCH, p.strong, tr("Search"), keys("Search: Room"), search);
        let more = Some(UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.into())));
        let more = tool(icon::EXPAND_MORE, p.strong, tr("More Options"), vec![], more);
        let toolbar = row![archive, remind].push(snooze).push(search).push(more).align_y(Center).padding([8, 12]);
        column![
            container(Space::new().width(Fill).height(2)).style(fill_box(Color { a: 0.1, ..p.strong }, 0.0)),
            container(toolbar).center_y(48),
        ]
        .into()
    }

    /// Desktop's Snooze button of the Chat Info toolbar shows when the chat can snooze, is snoozed or needs Beeper
    /// Plus to snooze; it is highlighted with the end of the snooze while the chat is snoozed.
    fn view_info_snooze(&self, st: &BeeperStore, chat: &Chat, cid: &str) -> Option<El<'_>> {
        let p = &self.pal;
        reminders::shows_snooze(st, &[chat]).then(|| {
            let until = reminders::snoozed_until(st, chat);
            let label = match until {
                Some(at) => reminders::snoozed_label(at, st.settings.pref("timeFormat") == 1),
                None => trx("Snooze", "chat info toolbar button").into(),
            };
            let icon = tinted(&MOON_STARS, 20.0, if until.is_some() { p.accent } else { p.strong });
            let open = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("{SNOOZE_MENU}{cid}")));
            let btn = button(container(icon).center_x(Fill)).padding(6).width(Fill).style(btn_style([CLEAR; 3], [p.strong; 3], 0.0));
            container(self.tip(btn.on_press(open).into(), label, vec![], tooltip::Position::Top)).center_x(Fill).into()
        })
    }

    /// Desktop's Labels section of Chat Info: the labels of the chat with Edit, or "Add to a Label".
    fn view_info_labels(&self, st: &BeeperStore, chat: &Chat, cid: &str) -> Option<El<'_>> {
        let p = &self.pal;
        let feature = st.feature("labels-v2");
        feature.show.then(|| {
            let open = feature.on.then(|| UiMessage::Labels(labels::Msg::OpenEdit(cid.into())));
            let mine: Vec<&Label> = chat.label_ids.iter().filter_map(|id| st.labels.iter().find(|l| l.id == *id)).collect();
            let edit =
                (!mine.is_empty()).then(|| section_btn(p, trx("Edit", "labels section button"), p.main_text).on_press_maybe(open.clone()));
            let head = row![txt(tr("Labels"), 11.0).color(p.strong), Space::new().width(Fill)].push(edit).align_y(iced::Bottom).padding(8);
            let body: El = match mine.is_empty() {
                true => {
                    button(container(row![ic(icon::PLUS, 16.0), txt(tr("Add to a Label"), 13.0)].spacing(4).align_y(Center)).center_x(Fill))
                        .padding(8)
                        .width(Fill)
                        .style(btn_style([p.chip; 3], [p.strong; 3], 26.0))
                        .on_press_maybe(open)
                        .into()
                }
                false => grouped_rows(
                    p,
                    mine.iter()
                        .map(|l| {
                            let name = ell(&l.name, 240.0, 13.0, Weight::Normal).line_height(lh(16.0));
                            row![label_icon(l, 20.0, p.strong), name.color(p.strong)].spacing(8).padding(8).align_y(Center).into()
                        })
                        .collect(),
                ),
            };
            column![head, body].into()
        })
    }

    /// Desktop's disappearing messages select of Chat Info, which only shows the timer when the chat cannot set it.
    fn view_info_timers(&self, st: &BeeperStore, chat: &Chat, cid: &str) -> Option<El<'_>> {
        let locked = chat.is_request == Some(true);
        chat.timers.as_ref().filter(|_| st.feature("enable-disappearing-message-timer-config").on).map(|t| {
            let values: Vec<u64> = [0].into_iter().chain(t.timers.iter().copied()).collect();
            let options = values.iter().map(|&ms| timer_label(ms)).collect();
            let id = cid.to_string();
            let pick = (t.can_set && !locked).then_some(move |i| UiMessage::Info(info::Msg::SetDisappearing(id.clone(), values[i])));
            let at = timer_label(chat.disappearing.unwrap_or(0));
            self.info_select(
                tr("Disappearing messages"),
                tr("For added privacy, new messages disappear after the set time."),
                options,
                at,
                pick,
            )
        })
    }

    /// Desktop's `BrandChatInfoNotificationSound`: Default, Silent, and the sound the chat has if it is another one.
    fn view_info_sound(&self, chat: &Chat, cid: &str) -> El<'_> {
        let current = chat.notification_sound.clone();
        let mut sounds: Vec<Option<String>> = vec![None, Some("silent".into())];
        sounds.extend(current.clone().filter(|s| s != "silent").map(Some));
        let options = sounds.iter().map(|s| sound_label(s.as_deref()).to_string()).collect();
        let id = cid.to_string();
        let pick = move |i: usize| UiMessage::Info(info::Msg::SetSound(id.clone(), sounds[i].clone()));
        let about = tr("Silent chats still show notifications, just without a sound.");
        self.info_select(tr("Notification sound"), about, options, sound_label(current.as_deref()).into(), Some(pick))
    }

    /// Desktop's `native` `BrandSelect` of a Chat Info section under its `title`: the 400 13/16 px value `at` and a
    /// 16 px chevron 12 px inside the 1 px clear border of a 26 px `--color-secondary-container` box with an 8 px
    /// radius, which picks one of `options`, over the 10/12 px `about`. Without `pick`, the value is
    /// `--color-on-surface-variant`.
    fn info_select<'a>(
        &self, title: &'a str, about: &'a str, options: Vec<String>, at: String, pick: Option<impl Fn(usize) -> UiMessage + 'a>,
    ) -> El<'a> {
        let p = &self.pal;
        let fg = if pick.is_some() { p.strong } else { p.muted };
        let value: El = match pick {
            Some(pick) => {
                let listed = options.clone();
                let (bg, style) = (p.chip, pick_style(p.chip, p.chip, fg, fg));
                pick_list(options, Some(at), move |l| pick(listed.iter().position(|o| *o == l).expect("a listed option")))
                    .text_size(rem(13.0))
                    .text_line_height(Pixels(rem(16.0)))
                    .handle(pick_list::Handle::None)
                    .padding([5, 13])
                    .width(Fill)
                    .style(move |t, s| pick_list::Style { background: bg.into(), border: border::rounded(8), ..style(t, s) })
                    .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border))
                    .into()
            }
            None => container(txt(at, 13.0).color(fg)).padding([5, 13]).width(Fill).style(fill_box(p.chip, 8.0)).into(),
        };
        let arrow = container(tinted(&CHEVRON_DOWN, 16.0, fg)).align_right(Fill).center_y(Fill).padding([0, 13]);
        // The description `small` takes the `--color-text-neutrals` of the pane.
        let about = container(txt(about, 10.0).color(p.text)).padding([0, 8]);
        let head = container(txt(title, 11.0).color(p.strong)).padding(8);
        column![head, column![stack![value, arrow], about].spacing(4)].into()
    }

    /// Desktop's zoomed `thread-info-avatar`: over a `#000000e6` backdrop, the avatar of the chat with its full-size
    /// image fills 90% of the smaller side of the window, with the close icon at the top right. A click zooms it out.
    pub(crate) fn view_avatar_zoom(&self) -> El<'_> {
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.as_deref().expect("a chat is selected");
        let chat = st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")].clone();
        let group = chat.chat_type == "group";
        let url = chat.img_url.clone().or_else(|| {
            let others = (!group).then(|| faces_of(&chat.participants.items, false).0);
            others.and_then(|o| o.first().and_then(|u| u.img_url.clone()))
        });
        drop(st);
        let full = url.as_deref().and_then(|u| self.frames(u));
        let shown = iced::widget::responsive(move |area| {
            let size = 0.9 * area.width.min(area.height);
            let avatar: El = match &full {
                Some(f) => {
                    self.emoji(f, move |h| image(h).width(size).height(size).content_fit(iced::ContentFit::Cover).border_radius(size / 2.0))
                }
                None => self.chat_avatar(&chat, size, 1.25),
            };
            let avatar = mouse_area(avatar).interaction(mouse::Interaction::ZoomOut);
            container(avatar).center(Fill).into()
        });
        let close = container(tinted(&FULL_SCREEN_CLOSE, 32.0, Color::WHITE)).align_right(Fill).padding(18);
        let backdrop = container(stack![shown, close]).width(Fill).height(Fill).style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.9), 0.0));
        crate::backdrop(backdrop).on_press(UiMessage::Info(info::Msg::Zoom)).into()
    }

    /// Desktop's `C0e` Media section: the newest messages with attachments or links, and when expanded all of them
    /// by type. Desktop's `attachmentMessages` takes the image, video and link messages of `populateAttachmentMessages`
    /// and every loaded message with an attachment or a link (`pushMessages`); the open chat has all its stored
    /// messages loaded, so its audio and file messages show too.
    pub(crate) fn view_info_media<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let mut found: Vec<&Message> = st
            .chat_timeline(&chat.id)
            .into_iter()
            .filter(|m| m.is_deleted != Some(true) && m.view_limit.is_none())
            .filter(|m| !m.links.is_empty() || !m.attachments.is_empty())
            .collect();
        if found.is_empty() {
            return None;
        }
        found.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
        let limit = if chat.chat_type == "single" { 4 } else { 6 };
        let expandable = found.len() > limit;
        let shown = if self.info_media.is_some() { &found[..] } else { &found[..found.len().min(limit)] };
        let tiles = self.view_info_media_tiles(chat, shown);
        let grid: El = match tiles.is_empty() {
            true => container(txt(tr("Nothing here"), 13.0).color(p.muted)).center_x(Fill).padding([10, 0]).into(),
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
            self.segments(labels, at, |i| UiMessage::Info(info::Msg::Media(Some(i)))).padding(Padding::new(8.0).top(0.0))
        });
        let toggle = if self.info_media.is_some() { None } else { Some(0) };
        let link = expandable.then(|| {
            let label = tr(if self.info_media.is_some() { "Show less" } else { "Show all" });
            section_btn(p, label, p.main_text).on_press(UiMessage::Info(info::Msg::Media(toggle)))
        });
        let head = row![txt(tr("Media"), 11.0).color(p.strong), Space::new().width(Fill)].push(link).align_y(Center).padding(8);
        let head: El = match expandable {
            true => mouse_area(head).on_press(UiMessage::Info(info::Msg::Media(toggle))).into(),
            false => head.into(),
        };
        let body = column![].push(tabs).push(grid).spacing(4).padding(8);
        Some(column![head, container(body).width(Fill).style(fill_box(Color { a: 0.1, ..p.strong }, 10.0))].into())
    }

    /// The tiles of the Media section: the images, videos and files of the messages of the tab, and their links,
    /// with the avatar of the sender.
    fn view_info_media_tiles<'a>(&'a self, chat: &Chat, shown: &[&Message]) -> Vec<El<'a>> {
        // Desktop's `mediaGrid`: two square columns (`repeat(2, minmax(auto, 1fr))`) 8 px apart, 124 px in the 8 px
        // padding of the 272 px pane, and narrower with Chat Info in a narrow window, down to `--min-media-width`.
        let narrower = if self.pane().info { 0.0 } else { 300.0 - self.info_width() };
        let tile = (124.0 - narrower / 2.0).max(40.0);
        let p = &self.pal;
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
        let file = |title: String, sub: El<'a>| -> El<'a> {
            // `FilePlaceholder` takes the `--color-text-neutrals` of the pane.
            let title = txt(crate::style::clamped(&title, tile - 16.0, 3, 10.0, Weight::Normal).to_string(), 10.0).color(p.text);
            container(column![title, sub]).padding(8).align_bottom(tile).width(tile).style(fill_box(p.card, 8.0)).into()
        };
        shown
            .iter()
            .flat_map(|m| {
                let atts = m.attachments.iter().filter(|a| kinds.contains(&a.attachment_type.as_str()));
                let atts = atts.map(|a| match a.attachment_type.as_str() {
                    "img" => self.thumb(a, tile, 4.0),
                    // Desktop's `BrandVideo` play button: a 36 px `secondary-container` disc in the middle.
                    "video" => {
                        let play = container(ic(icon::PLAY, 14.0).color(p.strong)).center(36).style(fill_box(p.chip, 18.0));
                        stack![self.thumb(a, tile, 4.0), container(play).center(tile)].into()
                    }
                    _ => {
                        let kind = ell(file_kind(a.mime_type.as_deref()), tile - 16.0, 10.0, Weight::Semibold);
                        file(a.file_name.clone().unwrap_or_default(), kind.line_height(lh(12.0)).color(p.text).into())
                    }
                });
                let urls = m.links.iter().filter(|_| links).map(|l| {
                    let title = crate::links::shown_title(l);
                    let url = ell(&l.url, tile - 29.0, 10.0, Weight::Semibold).line_height(lh(12.0)).color(p.text);
                    file(title, row![tinted(&MEDIA_ICONS[2], 12.0, p.text), url].align_y(Center).into())
                });
                atts.chain(urls).map(|face| {
                    let face = stack![face].push(self.media_sender(chat, m)).width(tile).height(tile);
                    button(face).bare(p.strong).on_press(UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), m.id.clone()))).into()
                })
            })
            .collect()
    }

    /// Desktop's `MediaWithSender`: the avatar of a member sender, with their full name in a tooltip on the left.
    fn media_sender(&self, chat: &Chat, m: &Message) -> Option<El<'_>> {
        let u = chat.participants.items.iter().find(|u| u.id == m.sender_id)?;
        // `MediaWithSender-module__avatar`: a 2 px white ring and `box-shadow: 0 0 8px #00000040`.
        let face = self.face(crate::user_img(u).as_deref(), &member_name(u), 20.0, 0.0);
        let face = container(face).padding(2).style(glow_box(Color::WHITE, Color::BLACK.scale_alpha(0.25), 12.0, 0.0, 8.0));
        let face: El = match u.full_name.clone() {
            Some(name) => self.tip(face.into(), name, vec![], tooltip::Position::Left).into(),
            None => face.into(),
        };
        Some(container(face).padding(8).into())
    }

    /// Desktop's `Kfe` Bookmarks section: the newest bookmarks of the chat first, 15 until "Show all".
    pub(crate) fn view_bookmarks<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        const SHOWN: usize = 15;
        let p = &self.pal;
        // Desktop's `syncMergedBookmarks`: a merged chat lists the bookmarks of its member chats.
        let idx = st.chat_idx(&chat.id);
        let loaded = st.chat_timeline(&chat.id).into_iter();
        let loaded = loaded.filter(|m| m.is_bookmarked == Some(true) && m.is_deleted != Some(true));
        let others = st.bookmarks.iter().filter(|(t, b)| st.in_chat(idx, &b.room_id) && st.loaded(t).is_none());
        let others = others.filter_map(|(t, _)| match st.bookmarked.get(t) {
            Some(Some(m)) if m.is_deleted == Some(true) => None,
            Some(m) => Some((t.as_str(), m.as_ref())),
            None => Some((t.as_str(), None)),
        });
        let mut marked: Vec<(&str, Option<&Message>)> = loaded.map(|m| (m.id.as_str(), Some(m))).chain(others).collect();
        marked.sort_by_cached_key(|(t, m)| {
            std::cmp::Reverse((st.bookmarks.get(*t).map_or(i64::MAX, |b| b.order), m.map(|m| m.sort_key.clone())))
        });
        let expandable = marked.len() > SHOWN;
        let shown = if self.info_bookmarks { marked.len() } else { marked.len().min(SHOWN) };
        let restored = st.bookmarks_restored();
        let rows: Vec<El> = marked[..shown].iter().map(|&(target, m)| self.view_bookmark_row(st, chat, target, m)).collect();
        // Desktop's empty Bookmarks by `bookmarkStore.status`: restored, failed, or still loading.
        let (title, about) = match (restored, st.bookmarks_failed) {
            (true, _) => (tr("No Bookmarks"), Some(tr("Bookmarked messages will appear here."))),
            (false, true) => (tr("Bookmarks unavailable"), Some(tr("Reopen this pane to try again."))),
            (false, false) => (tr("Loading…"), None),
        };
        let body: El = match rows.is_empty() {
            true => container(
                column![semi(title, 11.0).color(p.strong)].push(about.map(|a| txt(a, 10.0).color(p.muted))).spacing(4).align_x(Center),
            )
            .center_x(Fill)
            .padding([12, 8])
            .style(fill_box(p.chip, 8.0))
            .into(),
            false => grouped_rows(p, rows),
        };
        let toggle = expandable.then(|| {
            section_btn(p, tr(if self.info_bookmarks { "Show less" } else { "Show all" }), p.main_text)
                .on_press(UiMessage::Info(info::Msg::Bookmarks))
        });
        let head = row![txt(tr("Bookmarks"), 11.0).color(p.strong), Space::new().width(Fill)].push(toggle).align_y(Center).padding(8);
        column![head, body].into()
    }

    /// A row of the Bookmarks section: the sender, the time and the text of a bookmarked message, which jumps to it,
    /// or a message that is unavailable or still loading.
    fn view_bookmark_row<'a>(&'a self, st: &BeeperStore, chat: &Chat, target: &str, m: Option<&Message>) -> El<'a> {
        let p = &self.pal;
        let (h24, dim) = (st.settings.pref("timeFormat") == 1, Color { a: 0.7, ..p.strong });
        let restored = st.bookmarks_restored();
        let menu = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("bookmark:{} {target}", chat.id)));
        let blank = || Space::new().width(24).height(24);
        let lines = |title: El<'a>, text: &str| {
            let preview = ell(text, 220.0, 11.0, Weight::Normal).line_height(lh(14.0)).color(dim);
            column![title, preview].spacing(2).width(Fill)
        };
        let (line, jump) = match m {
            Some(m) => {
                let u = chat.participants.items.iter().find(|u| u.id == m.sender_id);
                let name = u.map(member_name).or_else(|| m.sender_name.clone()).unwrap_or_else(|| m.sender_id.clone());
                let title = row![
                    ell(&name, 160.0, 11.0, Weight::Semibold).line_height(lh(14.0)).color(p.strong),
                    txt(list_ts(&m.timestamp, h24), 10.0).color(dim)
                ]
                .spacing(6)
                .align_y(iced::Bottom);
                let text = preview_body(m).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                let face = self.face(u.and_then(crate::user_img).as_deref(), &name, 24.0, 0.0);
                let jump = st.loaded(target).is_some().then(|| UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), m.id.clone())));
                (row![face, lines(title.into(), &text)], jump)
            }
            None if st.bookmarked.contains_key(target) => {
                let title = semi(tr("Message unavailable"), 11.0).color(p.strong);
                let remove = button(ic(icon::CLOSE, 16.0).color(p.muted))
                    .bare(p.muted)
                    .padding(4)
                    .on_press_maybe(restored.then(|| UiMessage::Info(info::Msg::RemoveBookmark(target.into()))));
                let text = tr("It was deleted or is no longer available.");
                (row![blank(), lines(title.into(), text), remove], None)
            }
            None => {
                let title = semi(tr("Loading…"), 11.0).color(p.strong.scale_alpha(0.5));
                (row![blank(), lines(title.into(), "")], None)
            }
        };
        let btn = button(line.spacing(8).align_y(Center)).bare(p.strong).padding(8).width(Fill).on_press_maybe(jump);
        mouse_area(btn).on_right_press(menu).into()
    }
}

/// The style of the notes and the group description editors, whose placeholder is `--color-on-surface-variant`.
fn info_editor(p: &Palette, r: f32) -> impl Fn(&Theme, text_editor::Status) -> text_editor::Style + Copy {
    let (style, placeholder) = (editor_style(p, r), p.muted);
    move |t, s| text_editor::Style { placeholder, ..style(t, s) }
}

/// A `metadata` row of the Chat Info header: its 11/14 px text sits on the baseline of the 18.2 px line of the 13 px
/// header, 2.8 px from its top.
fn meta_line<'a>(line: impl Into<El<'a>>) -> El<'a> {
    container(line).padding(Padding::ZERO.top(rem(2.8)).bottom(rem(1.4))).into()
}

/// Desktop's `aX`: the name of a chat sound, and Default for none.
fn sound_label(sound: Option<&str>) -> &'static str {
    match sound {
        None => tr("Default"),
        Some("silent") => tr("Silent"),
        Some("double") => tr("Double"),
        Some("dreams") => tr("Dreams"),
        Some("not_marimba") => tr("Not Marimba"),
        Some("rain_harp") => tr("Rain Harp"),
        Some("record") => tr("Record"),
        Some("spirit") => tr("Spirit"),
        Some(other) => unreachable!("the chat keeps only known sounds, not {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use iced_test::simulator::Simulator;

    const JORDAN: &str = "!im_jordan:beeper.com";

    fn tall(el: El<'_>) -> Simulator<'_, UiMessage> {
        Simulator::with_size(Default::default(), Size::new(1024.0, 2000.0), el)
    }

    /// A press and a release of the mouse at `at`.
    fn click_at(sim: &mut Simulator<UiMessage>, at: Point) {
        sim.point_at(at);
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
        let _ = sim.simulate(iced_test::simulator::click());
    }

    /// A demo app with the Chat Info of `chat` open.
    fn info_app(name: &str, chat: &str) -> BeeperGuiApp {
        let mut app = demo_app(name);
        let _ = app.update(UiMessage::SelectChat(chat.into()));
        app.show_right_panel = true;
        app
    }

    /// Adds a message from `sender` with an attachment of `kind` named `name` to the chat, newest.
    fn attach(app: &BeeperGuiApp, chat: &str, sender: &str, kind: &str, name: &str) {
        let mut st = app.store.write();
        let base = st.messages[*st.timeline_indices(chat).last().expect("a message")].clone();
        let a = Attachment {
            attachment_type: kind.into(),
            file_name: Some(name.into()),
            mime_type: Some(if kind == "audio" { "audio/mpeg" } else { "application/pdf" }.into()),
            src_url: Some(format!("https://example.com/{name}")),
            ..Default::default()
        };
        let sort_key = format!("{:016}", 9_000_000 + st.timeline_indices(chat).len());
        let id = format!("${name}");
        let m = Message { id, sender_id: sender.into(), attachments: vec![a], links: Vec::new(), sort_key, ..base };
        st.upsert_message(m);
    }

    /// Desktop's `$8t`: a click on the Chat Info avatar portals it over the whole window, and a click anywhere there
    /// zooms it out. Another chat opens with the avatar in place.
    #[test]
    fn chat_info_avatar_zooms_like_desktop() {
        let mut app = info_app("chat-info-zoom", JORDAN);
        let msgs: Vec<UiMessage> = {
            let mut sim = tall(app.view_info());
            click_at(&mut sim, Point::new(150.0, 92.0));
            sim.into_messages().collect()
        };
        assert!(matches!(&msgs[..], [UiMessage::Info(Msg::Zoom)]), "the avatar zooms");
        let corner = |app: &BeeperGuiApp| -> Vec<UiMessage> {
            let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
            click_at(&mut sim, Point::new(4.0, 700.0));
            sim.into_messages().collect()
        };
        assert!(!corner(&app).iter().any(|m| matches!(m, UiMessage::Info(Msg::Zoom))));
        let _ = app.update(UiMessage::Info(Msg::Zoom));
        assert!(matches!(&corner(&app)[..], [UiMessage::Info(Msg::Zoom)]), "the backdrop covers the window");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        assert!(!app.info_zoom);
    }

    /// Desktop's Media section takes the loaded audio and file messages too, so its Audio and Files tabs list them.
    #[test]
    fn chat_info_media_lists_audio_and_files_like_desktop() {
        let mut app = info_app("chat-info-media-files", JORDAN);
        let me = app.store.read().me().id.clone();
        attach(&app, JORDAN, &me, "audio", "song.mp3");
        attach(&app, JORDAN, "@stranger:beeper.com", "unknown", "notes.pdf");
        {
            let mut sim = tall(app.view_info());
            assert!(sim.find("notes.pdf").is_ok() && sim.find("PDF").is_ok(), "the newest tiles show the file");
            assert!(sim.find("song.mp3").is_ok() && sim.find("Audio").is_ok());
        }
        let _ = app.update(UiMessage::Info(Msg::Media(Some(2))));
        {
            let mut sim = tall(app.view_info());
            assert!(sim.find("song.mp3").is_ok() && sim.find("notes.pdf").is_err(), "the Audio tab");
        }
        let _ = app.update(UiMessage::Info(Msg::Media(Some(4))));
        let mut sim = tall(app.view_info());
        assert!(sim.find("notes.pdf").is_ok() && sim.find("song.mp3").is_err(), "the Files tab");
    }

    /// Desktop's empty Bookmarks by `bookmarkStore.status`, and its Remove Bookmark only once they are restored.
    #[test]
    fn chat_info_bookmarks_wait_for_the_restoration_like_desktop() {
        let app = info_app("chat-info-bookmarks-status", "!tg_rust:beeper.com");
        app.store.write().config.demo = false;
        {
            let mut sim = tall(app.view_info());
            assert!(sim.find("Loading…").is_ok() && sim.find("No Bookmarks").is_err());
        }
        app.store.write().bookmarks_failed = true;
        {
            let mut sim = tall(app.view_info());
            assert!(sim.find("Bookmarks unavailable").is_ok());
            assert!(sim.find("Reopen this pane to try again.").is_ok());
        }
        let menu = info_menu(&app.store.read(), "bookmark:!tg_rust:beeper.com $m", |_| true);
        assert!(matches!(&menu[2], Some((_, None))), "Remove Bookmark waits for the restoration");
        app.store.write().apply_bookmarks("!bookmarks", Vec::new());
        assert!(!app.store.read().bookmarks_failed);
        assert!(tall(app.view_info()).find("No Bookmarks").is_ok());
    }

    /// The sections of a group's Chat Info sit where Desktop's CSS puts them, as measured in Desktop with the same
    /// sections: the metadata on the baseline of its line, 16 px header buttons, 36 px member rows and 26 px selects.
    #[test]
    fn chat_info_sections_sit_like_desktop() {
        let app = info_app("chat-info-layout", "!tg_rust:beeper.com");
        let mut sim = tall(app.view_info());
        let mut y = |label: &str| sim.find(label).expect(label).bounds().y;
        let title = "Beeper Rust Core Team";
        for (label, above, below) in [("Telegram", title, 26.8), ("Group Description", title, 59.2), ("Media", "Members", 208.0)] {
            let at = y(label) - y(above);
            assert!((at - below).abs() < 0.5, "{label}: {at} below {above}");
        }
        let (head, about) = (y("Notification sound"), y("Silent chats still show notifications, just without a sound."));
        assert!((about - head - 52.0).abs() < 0.5, "a 26 px select: {}", about - head);
        // The email of a DM is a `div` in the `metadata` span, on its own 14 px line.
        let app = info_app("chat-info-dm-layout", JORDAN);
        let mut sim = tall(app.view_info());
        let mut y = |label: &str| sim.find(label).expect(label).bounds().y;
        for (label, above, below) in
            [("@jordan", "Jordan Park", 26.8), ("jordan@example.com", "@jordan", 15.4), ("iMessage", "jordan@example.com", 20.8)]
        {
            let at = y(label) - y(above);
            assert!((at - below).abs() < 0.5, "{label}: {at} below {above}");
        }
    }

    /// Desktop's 330 by 560 Chat Info window, measured: the rounded `thread-info-inner` card at (8, 8, 314, 494), and
    /// the toolbar under it at (8, 502, 314, 50).
    #[test]
    fn detached_chat_info_puts_the_toolbar_under_the_card_like_desktop() {
        let mut app = info_app("chat-info-detached", JORDAN);
        app.pane.info = true;
        let mut sim = Simulator::with_size(Default::default(), Size::new(330.0, 560.0), app.view_info());
        let mut boxes = Vec::new();
        let _ = sim.find(|c: iced_test::selector::Candidate<'_>| -> Option<()> {
            if let iced_test::selector::Candidate::Container { bounds, .. } = c {
                boxes.push(bounds);
            }
            None
        });
        for want in [
            iced::Rectangle::new(Point::new(8.0, 8.0), Size::new(314.0, 494.0)),
            iced::Rectangle::new(Point::new(8.0, 502.0), Size::new(314.0, 50.0)),
        ] {
            assert!(boxes.contains(&want), "{want:?} in {boxes:?}");
        }
    }

    /// A demo app where Desktop's `snoozeOrSchedulesEnabled` is on: the internal channel with labels.
    fn behavior_app(name: &str) -> BeeperGuiApp {
        let app = info_app(name, JORDAN);
        app.store.write().whoami.internal = true;
        app
    }

    fn line(app: &BeeperGuiApp) -> Option<String> {
        let st = app.store.read();
        behavior::behavior(&st, st.chat(JORDAN)).and_then(|b| b.line(false))
    }

    /// Desktop's behavior line (`P0e`): what mutes or hides the chat, its Visibility and Notifications menu with what
    /// each source does, and How It Works.
    #[test]
    fn chat_info_behavior_line_like_desktop() {
        let mut app = behavior_app("chat-info-behavior");
        assert_eq!(line(&app), None, "a chat that shows and notifies has no line");
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(JORDAN.into(), ChatFlag::Muted)));
        assert_eq!(line(&app).as_deref(), Some("Muted by you"));
        let msgs: Vec<UiMessage> = {
            let mut sim = tall(app.view_info());
            let _ = sim.click("Muted by you").expect("the behavior line");
            sim.into_messages().collect()
        };
        let menu = format!("{}{JORDAN}", behavior::BEHAVIOR_MENU);
        assert!(matches!(&msgs[..], [UiMessage::ChatList(chat_list::Msg::OpenMenu(k))] if *k == menu));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(menu.clone())));
        let msgs: Vec<UiMessage> = {
            let mut sim = Simulator::with_size(Default::default(), Size::new(1000.0, 720.0), app.view_main());
            for text in ["Visibility and Notifications", "Chat Settings", "Mutes this chat"] {
                assert!(sim.find(text).is_ok(), "the menu has {text}");
            }
            let _ = sim.click("How It Works").expect("How It Works");
            sim.into_messages().collect()
        };
        assert!(matches!(&msgs[..], [UiMessage::Commands(c)] if matches!(&c[1], UiMessage::Info(Msg::HowItWorks(true)))));
        let _ = app.update(UiMessage::Info(Msg::HowItWorks(true)));
        assert!(app.how_window.is_some() && app.list.chat_menu.is_none());
        let msgs: Vec<UiMessage> = {
            let mut sim = tall(app.view_how_it_works());
            assert!(sim.find("You Come First").is_ok() && sim.find("Chats Without a Label").is_ok());
            let _ = sim.click("Done").expect("Done");
            sim.into_messages().collect()
        };
        assert!(matches!(&msgs[..], [UiMessage::Info(Msg::HowItWorks(false))]));
        // A muted label of the chat mutes it too, but the chat's own mute decides.
        let _ = app.update(UiMessage::Labels(labels::Msg::Switch("lbl_work".into(), JORDAN.into(), true)));
        let _ = app.update(UiMessage::Labels(labels::Msg::Mute("lbl_work".into(), true)));
        assert_eq!(line(&app).as_deref(), Some("Muted by you"));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::ToggleFlag(JORDAN.into(), ChatFlag::Muted)));
        assert_eq!(line(&app).as_deref(), Some("Muted by Work"));
        let _ = app.update(UiMessage::Labels(labels::Msg::SetInbox("lbl_work".into(), Some("none"))));
        assert_eq!(line(&app).as_deref(), Some("Hidden and muted by Work"));
        let rows = |app: &BeeperGuiApp| {
            let st = app.store.read();
            let mut sim = tall(Column::with_children(app.behavior_items(&st, JORDAN).0.into_iter().map(|r| r.0)).into());
            sim.find("Work").is_ok() && sim.find("Hides and mutes this chat").is_ok()
        };
        assert!(rows(&app), "the label hides and mutes the chat");
        app.store.write().whoami.internal = false;
        assert_eq!(line(&app), None, "only with snoozing or label schedules");
    }

    /// Desktop's "Snoozed until %s" line of a snoozed chat, with the time of a snooze that ends today.
    #[test]
    fn chat_info_behavior_line_of_a_snoozed_chat() {
        let mut app = behavior_app("chat-info-behavior-snooze");
        {
            let mut st = app.store.write();
            st.features.internal = true;
            st.features.flags["features"]["*"]["snooze"] = serde_json::json!(true);
        }
        let at = now_unix_millis() + 60_000;
        let _ = app.update(UiMessage::Reminders(reminders::Msg::Snooze(vec![JORDAN.into()], Some(at))));
        let shown = match day_of(at) == today() {
            true => clock(at, false),
            false => scheduled::send_day(at, false),
        };
        assert_eq!(line(&app), Some(format!("Snoozed until {shown}")));
        let st = app.store.read();
        let mut sim = tall(Column::with_children(app.behavior_items(&st, JORDAN).0.into_iter().map(|r| r.0)).into());
        assert!(sim.find("Chat Snooze").is_ok() && sim.find(format!("Snoozes until {shown}").as_str()).is_ok());
    }
}

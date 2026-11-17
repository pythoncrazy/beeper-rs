//! Desktop's Chat Info panel: the description, the note, the media and bookmarks lists, the personal name and disappearing messages.

use super::*;
use crate::chat_list::preview_body;
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
    Media(Option<usize>),
    Query(String),
    Note(text_editor::Action),
    /// Desktop's disappearing messages select: a chat and a timer in ms, or 0 for Off.
    SetDisappearing(String, u64),
    /// Desktop's "Notification sound" select: a chat and a sound of `CHAT_SOUNDS`, or `None` for Default.
    SetSound(String, Option<String>),
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
                self.chat_menu = None;
                self.store.write().remove_bookmark(&target);
            }
            Msg::BookmarkHydrated(target, m) => {
                self.hydrating.remove(&target);
                self.store.write().bookmarked.insert(target, m.map(|m| *m));
            }
            Msg::Media(tab) => self.info_media = tab,
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
}

/// The Chat Info menus. The header menus of Desktop's `$fe`: `info:<chat>` has the edit actions and
/// `copy:<chat>`, the menu of the clicked title, adds "Copy <title>" above them. `copy:<chat>\n<text>` copies the
/// clicked line instead.
pub(crate) fn info_menu(
    st: &BeeperStore,
    key: &str,
    dm: impl Fn(&str) -> bool,
) -> Vec<Option<(String, Option<UiMessage>)>> {
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
            _ => vec![item(
                tr("Copy Username").into(),
                UiMessage::Message(message::Msg::CopyText(handle(u).expect("a username"))),
            )],
        };
    }
    if let Some((cid, mid)) = key.strip_prefix("bookmark:").and_then(|k| k.split_once(' ')) {
        let jump =
            st.loaded(mid).is_some().then(|| UiMessage::CommandBar(commands::Msg::JumpTo(cid.into(), mid.into())));
        let remove = UiMessage::Info(info::Msg::RemoveBookmark(mid.into()));
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
            text.map(|t| Some((tr(label).to_string(), Some(UiMessage::Message(message::Msg::CopyText(t))))))
        };
        let username = handle(u);
        let call = u.phone_number.as_ref().map(|n| {
            Some((
                trx("Call", "button label to start a phone call").to_string(),
                Some(UiMessage::OpenUrl(format!("tel:{n}"))),
            ))
        });
        let send = dm(&c.account_id).then(|| {
            Some((
                tr("Send Message").to_string(),
                Some(UiMessage::Members(members::Msg::Message(cid.into(), uid.into()))),
            ))
        });
        let ask = |label: &'static str, ask, on: bool| {
            Some((
                tr(label).to_string(),
                on.then(|| UiMessage::Members(members::Msg::Ask(ask, cid.into(), uid.into()))),
            ))
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
    let theme = Some((tr("Chat Theme").to_string(), Some(UiMessage::ChatTheme(chat_theme::Msg::Open(c.id.clone())))));
    let text = text.unwrap_or(&c.title);
    let title = copy
        .then(|| Some((trf("Copy %s", &[&text]), Some(UiMessage::Message(message::Msg::CopyText(text.to_string()))))));
    let edits = [group, personal, theme].into_iter().flatten().map(Some);
    title.into_iter().chain(copy.then_some(None)).chain(edits).collect()
}

impl BeeperGuiApp {
    pub(crate) fn view_info(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.clone().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(&cid).expect("selected chat exists")];
        let on = |v: Option<bool>| v == Some(true);
        let flag = |f: ChatFlag| UiMessage::ChatList(chat_list::Msg::ToggleFlag(cid.clone(), f));
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
            _ => edits
                .iter()
                .flatten()
                .any(|i| i.1.is_some())
                .then(|| UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("info:{cid}")))),
        };
        let edit = button(semi(trx("Edit", "chat info edit button"), 11.0).color(p.accent))
            .bare(p.accent)
            .on_press_maybe(edit);
        let copy = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("copy:{cid}")));
        let title = mouse_area(
            button(semi(chat.title.clone(), 15.0).color(p.strong).center()).bare(p.strong).on_press(copy.clone()),
        )
        .on_right_press(copy);
        let meta = |s: String| {
            let copy = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("copy:{cid}\n{s}")));
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
                let done = link(
                    trx("Done", "group description submit button"),
                    Weight::Semibold,
                    UiMessage::Info(info::Msg::SaveDesc),
                );
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
                let input =
                    if can_desc { input.on_action(|a| UiMessage::Info(info::Msg::Desc(Some(a)))) } else { input };
                Some(
                    column![desc_head(can_desc.then_some(done)), input]
                        .spacing(8)
                        .padding(Padding::ZERO.top(8.0))
                        .into(),
                )
            }
            (None, None) if can_desc => Some(
                container(link(tr("Add a description"), Weight::Semibold, UiMessage::Info(info::Msg::Desc(None))))
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
                let edit = link(
                    trx("Edit", "group description edit button"),
                    Weight::Semibold,
                    UiMessage::Info(info::Msg::Desc(None)),
                );
                let more = more.then(|| {
                    link(
                        trx("more", "expand group description button"),
                        Weight::Normal,
                        UiMessage::Info(info::Msg::More),
                    )
                });
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
        .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("info:{cid}"))));
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
                    Some(UiMessage::Reminders(reminders::Msg::Set(cid.clone(), None))),
                ),
                false => tool(
                    icon::SCHEDULE,
                    p.strong,
                    tr("Remind Me"),
                    vec!["Ctrl", "L"],
                    Some(UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.clone()))),
                ),
            },
            tool(
                icon::SEARCH,
                p.strong,
                tr("Search"),
                vec!["Ctrl", "F"],
                Some(UiMessage::CommandBar(commands::Msg::SearchRoom(cid.clone())))
            ),
            tool(
                icon::EXPAND_MORE,
                p.strong,
                tr("More Options"),
                vec![],
                Some(UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.clone())))
            ),
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
            let open = feature.on.then(|| UiMessage::Labels(labels::Msg::OpenEdit(cid.clone())));
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
                    UiMessage::Info(info::Msg::SetDisappearing(id.clone(), values[i]))
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
        // Desktop's `BrandChatInfoNotificationSound`: Default, Silent, and the sound the chat has if it is another one.
        let sound = {
            let current = chat.notification_sound.clone();
            let mut sounds: Vec<Option<String>> = vec![None, Some("silent".into())];
            sounds.extend(current.clone().filter(|s| s != "silent").map(Some));
            let labels: Vec<&'static str> = sounds.iter().map(|s| sound_label(s.as_deref())).collect();
            let id = cid.clone();
            let pick = pick_list(labels.clone(), Some(sound_label(current.as_deref())), move |l| {
                let i = labels.iter().position(|o| *o == l).expect("a listed sound");
                UiMessage::Info(info::Msg::SetSound(id.clone(), sounds[i].clone()))
            })
            .text_size(13)
            .text_line_height(Pixels(16.0))
            .handle(pick_list::Handle::None)
            .padding([8, 12])
            .width(Fill)
            .style(pick_style(p.chip, p.chip, p.strong, p.strong))
            .menu_style(pick_menu_style(p.elevated, p.selected, p.text, p.prefs_border));
            let arrow =
                container(tinted(&SELECT_ARROW, 10.0, p.strong)).align_right(Fill).center_y(Fill).padding([0, 12]);
            let about = txt(tr("Silent chats still show notifications, just without a sound."), 10.0).color(p.strong);
            let head = container(txt(tr("Notification sound"), 11.0).color(p.strong)).padding(8);
            column![head, column![stack![pick, arrow], container(about).padding([0, 8])].spacing(4)]
        };
        let note = text_editor(&self.info_note)
            .placeholder("Notes")
            .on_action(|a0| UiMessage::Info(info::Msg::Note(a0)))
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
            .push(sound)
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
                    .on_press(UiMessage::Blocking(blocking::Msg::Ask(cid.clone(), block)));
                container(row).padding(Padding::ZERO.top(8.0))
            }))
            .push((chat.member_actions & MEMBER_LEAVE != 0).then(|| {
                let leave = button(semi(tr("Leave Chat"), 14.0).line_height(Pixels(18.0)).color(p.strong))
                    .padding([0, 8])
                    .style(btn_style([CLEAR; 3], [p.strong; 3], 0.0))
                    .on_press(UiMessage::ChatList(chat_list::Msg::AskLeave(cid.clone())));
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
    pub(crate) fn view_info_media<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
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
                        .on_press(UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), m.id.clone())))
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
            self.segments(labels, at, |i| UiMessage::Info(info::Msg::Media(Some(i))))
                .padding(Padding::new(8.0).top(0.0))
        });
        let toggle = if self.info_media.is_some() { None } else { Some(0) };
        let link = expandable.then(|| {
            let label = tr(if self.info_media.is_some() { "Show less" } else { "Show all" });
            button(semi(label, 11.0).color(p.accent)).bare(p.accent).on_press(UiMessage::Info(info::Msg::Media(toggle)))
        });
        let head = row![txt(tr("Media"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(link)
            .align_y(Center)
            .padding(8);
        let head: El = match expandable {
            true => mouse_area(head).on_press(UiMessage::Info(info::Msg::Media(toggle))).into(),
            false => head.into(),
        };
        let body = column![].push(tabs).push(grid).spacing(4).padding(8);
        Some(column![head, container(body).width(Fill).style(fill_box(Color { a: 0.1, ..p.strong }, 10.0))].into())
    }

    /// Desktop's `Kfe` Bookmarks section: the newest bookmarks of the chat first, 15 until "Show all".
    pub(crate) fn view_bookmarks<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
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
                let menu = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("bookmark:{} {target}", chat.id)));
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
                        let jump = st
                            .loaded(target)
                            .is_some()
                            .then(|| UiMessage::CommandBar(commands::Msg::JumpTo(chat.id.clone(), m.id.clone())));
                        (row![face, lines(title.into(), &text)], jump)
                    }
                    None if st.bookmarked.contains_key(target) => {
                        let title = semi(tr("Message unavailable"), 11.0).color(p.strong);
                        let remove = button(ic(icon::CLOSE, 16.0).color(p.sub))
                            .padding(4)
                            .style(btn_style([CLEAR; 3], [p.sub; 3], 8.0))
                            .on_press(UiMessage::Info(info::Msg::RemoveBookmark(target.into())));
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
                .on_press(UiMessage::Info(info::Msg::Bookmarks))
        });
        let head = row![txt(tr("Bookmarks"), 11.0).color(p.strong), Space::new().width(Fill)]
            .push(toggle)
            .align_y(Center)
            .padding(8);
        column![head, body].into()
    }
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

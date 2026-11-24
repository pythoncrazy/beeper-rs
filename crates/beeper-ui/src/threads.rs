//! Desktop's threaded replies: the thread pane and window, the unread threads list, and the thread parts of the
//! timeline.

use crate::i18n::{tr, trf, trn};
use crate::message;
use crate::prelude::*;
use crate::{activates, MIN_PANE_W};
use iced::mouse;
use iced::widget::column;

/// Desktop's default `threadedRepliesPaneFraction`.
pub(crate) const THREAD_FRACTION: f32 = 0.5;

/// The file of Desktop's `threaded-replies-pane-fraction`.
const THREAD_FRACTION_FILE: &str = "threaded-replies-pane-fraction";

/// Desktop's saved `threadedRepliesPaneFraction`, between 0.1 and 0.9, or the default.
pub(crate) fn saved_thread_fraction(data_dir: &std::path::Path) -> f32 {
    let saved = std::fs::read_to_string(data_dir.join(THREAD_FRACTION_FILE)).ok().and_then(|f| f.parse::<f32>().ok());
    saved.filter(|f| *f > 0.0).map_or(THREAD_FRACTION, |f| f.clamp(0.1, 0.9))
}

/// Desktop's width of the edge that resizes the thread pane.
const THREAD_GRIP: f32 = 8.0;

/// Desktop's `canReplyInThread`: a message of the chat of a room with threads, which is not a thread reply, not
/// deleted and not an action, in a chat that you can write to.
pub(crate) fn can_thread(st: &BeeperStore, chat: &Chat, m: &Message) -> bool {
    let action = membership(m) || m.timer.is_some();
    !chat.is_read_only && st.threaded(chat) && m.thread_root.is_none() && m.is_deleted != Some(true) && !action
}

/// The unread threads of the chats, as the chat index and the root.
pub(crate) type Threads = Rc<[(usize, String)]>;

impl BeeperGuiApp {
    /// Desktop's `showThreadedReplies`: the thread pane of the thread of message `id`, in place of Chat Info.
    pub(crate) fn show_thread(&mut self, id: &str) {
        let root = {
            let st = self.store.read();
            st.loaded(id).and_then(|m| m.thread_root.clone()).unwrap_or_else(|| id.into())
        };
        let chat = self.pane.selected_chat_id.clone().expect("a thread opens from a chat");
        self.open_thread(chat, root);
    }

    /// The thread pane of `root` in `chat`, which reads the thread and keeps its read mark for the unread divider.
    pub(crate) fn open_thread(&mut self, chat: String, root: String) {
        if self.pane.thread.as_ref().is_none_or(|t| t.thread_root.as_ref() != Some(&root)) {
            let mut st = self.store.write();
            let idx = st.resolve_chat_idx(&chat).expect("a known chat");
            let (cid, unread) = (st.chats[idx].id.clone(), st.thread_unread(&st.chats[idx], &root).0 > 0);
            let divider = st.chats[idx].thread_reads.get(&root).cloned().filter(|_| unread);
            st.mark_thread_read(&cid, &root, None);
            let pane = Pane {
                selected_chat_id: Some(chat),
                thread_root: Some(root),
                thread_divider: divider,
                size: self.pane.size,
                ..Pane::default()
            };
            (self.pane.thread, self.pane.thread_focus) = (Some(Box::new(pane)), true);
        }
        (self.show_right_panel, self.pane.thread_menu) = (false, None);
    }

    /// Desktop's `markReplyThreadReadThrough` of "Mark as Read" in the thread pane.
    pub(crate) fn mark_thread_read(&mut self) {
        self.pane.thread_menu = None;
        let thread = self.pane.thread.as_ref().expect("a thread pane");
        let (chat, root) = (thread.selected_chat_id.clone().expect("a chat"), thread.thread_root.clone().expect("a root"));
        let mut st = self.store.write();
        let cid = st.chats[st.resolve_chat_idx(&chat).expect("a known chat")].id.clone();
        st.mark_thread_read(&cid, &root, None);
    }

    /// Desktop's `setThreadFollowing`.
    pub(crate) fn follow_thread(&mut self, chat: &str, root: &str, on: bool) {
        (self.pane.message_menu, self.pane.thread_menu) = (None, None);
        let mut st = self.store.write();
        let cid = st.chats[st.resolve_chat_idx(chat).expect("a known chat")].id.clone();
        st.follow_thread(&cid, root, on);
    }

    /// Desktop's `setFollowAllThreads`.
    pub(crate) fn follow_all_threads(&mut self, chat: &str, on: bool) {
        self.list.chat_menu = None;
        let mut st = self.store.write();
        let cid = st.chats[st.resolve_chat_idx(chat).expect("a known chat")].id.clone();
        st.follow_all_threads(&cid, on);
    }

    /// Desktop's `fhe`: "Follow Thread" or "Unfollow Thread" of the thread of `root` in `chat`.
    pub(crate) fn follow_item<'a>(&self, st: &BeeperStore, chat: &Chat, root: &str) -> El<'a> {
        let on = st.follows_thread(chat, root);
        let label = tr(if on { "Unfollow Thread" } else { "Follow Thread" }).to_string();
        menu_item(&self.pal, None, label, "", Some(UiMessage::Thread(Msg::Follow(chat.id.clone(), root.into(), !on))))
    }

    /// Desktop's `ThreadedRepliesBrowserWindow`: a window of 380 by 700 with the thread of `root` in `chat`.
    pub(crate) fn open_thread_window(&mut self, chat: String, root: String) -> Task<UiMessage> {
        let size = Size::new(380.0, 700.0);
        let (id, open) = self.open_window(Self::window_settings(size));
        let pane = Pane { selected_chat_id: Some(chat), thread_root: Some(root), size, ..Pane::default() };
        self.chat_windows.push((id, pane));
        // `ThreadedRepliesBrowserWindow` keeps the place of `ao.windowFeatures`.
        Task::batch([open.discard(), self.cascade(id)])
    }

    /// The thread window: Desktop's `ThreadedRepliesHeader` with the title over the thread.
    pub(crate) fn view_thread_window(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.as_deref().expect("a thread window has a chat");
        let title = st.resolve_chat_idx(cid).map_or("Beeper".into(), |i| st.chats[i].title.clone());
        drop(st);
        let title = semi(trf("Thread in %s", &[&title]), 14.0).color(p.text).wrapping(iced::widget::text::Wrapping::None);
        let header = container(title).center_x(Fill).center_y(36).padding([0, 4]).clip(true);
        stack![self.view_conversation(), header].into()
    }

    /// Desktop's `showUnreadThreads`: leaves the chat for the Unread threads view.
    pub(crate) fn show_unread_threads(&mut self) {
        if self.pane.selected_chat_id.is_some() {
            self.leave_chat(&mut self.store.write(), None);
            (self.pane.selected_chat_id, self.pane.composer) = (None, composer_with(""));
        }
        (self.unread_view, self.pane.thread, self.creating_chat) = (true, None, None);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `unreadThreadEntries` of the chats in the chat list when `listed`, or of all chats. Each thread reads SQLite, so the memo
    /// keeps the unread threads of all chats until the store changes.
    pub(crate) fn unread_threads(&self, st: &BeeperStore, listed: bool) -> Vec<(usize, String)> {
        let memo = self.memo();
        let all = memo.and_then(|m| m.threads.borrow().clone()).unwrap_or_else(|| {
            let all: Threads = st.unread_threads(0..st.chats.len()).into();
            if let Some(m) = memo {
                *m.threads.borrow_mut() = Some(all.clone());
            }
            all
        });
        all.iter().filter(|(i, _)| !listed || self.listed(st, self.list().filters, &st.chats[*i])).cloned().collect()
    }

    /// Desktop's `markAllUnreadThreadsAsRead`.
    pub(crate) fn mark_all_threads_read(&mut self) {
        self.unread_menu = None;
        let mut st = self.store.write();
        let chats: Vec<usize> = self.visible_chats(&st).iter().map(|c| st.chat_idx(&c.id)).collect();
        for (i, root) in st.unread_threads(chats) {
            let cid = st.chats[i].id.clone();
            st.mark_thread_read(&cid, &root, None);
        }
    }

    /// Handles a message of the thread pane with the thread pane in place of the pane of the chat.
    pub(crate) fn in_thread(&mut self, msg: UiMessage) -> Task<UiMessage> {
        let Some(mut thread) = self.pane.thread.take() else { return Task::none() };
        let focus = activates(&msg);
        std::mem::swap(&mut self.pane, &mut thread);
        // The carousel is that of the window, also for the messages of the thread pane.
        std::mem::swap(&mut self.pane.carousel, &mut thread.carousel);
        let task = self.handle(msg);
        std::mem::swap(&mut self.pane.carousel, &mut thread.carousel);
        std::mem::swap(&mut self.pane, &mut thread);
        (self.pane.thread, self.pane.thread_focus) = (Some(thread), self.pane.thread_focus || focus);
        task.map(|m| UiMessage::InThread(Box::new(m)))
    }

    /// Desktop's `useResizeHandle` of the thread pane: a press or a release toggles the drag, and the edge follows the
    /// pointer, which leaves at least `MIN_PANE_W` to the pane and to the chat.
    pub(crate) fn drag_thread(&mut self, at: Option<Point>) {
        let Some(at) = at else {
            self.thread_drag = !self.thread_drag;
            if !self.thread_drag {
                self.set_thread_fraction(self.thread_fraction);
            }
            return;
        };
        let area = self.thread_area.get() - THREAD_GRIP;
        let w = self.pane.size.width - 8.0 - at.x;
        self.thread_fraction = w.min(area - MIN_PANE_W).max(MIN_PANE_W) / area;
    }

    /// Desktop's `setThreadedRepliesPaneFraction`, which the next launch restores.
    fn set_thread_fraction(&mut self, fraction: f32) {
        self.thread_fraction = fraction;
        let path = self.store.read().config.data_dir.join(THREAD_FRACTION_FILE);
        std::fs::write(path, fraction.to_string()).expect("write the thread pane fraction");
    }

    /// The chat and Desktop's `ThreadedRepliesPane` at its right, which takes `thread_fraction` of the `area`.
    pub(crate) fn view_split(&self, area: f32, pad: Padding) -> El<'_> {
        self.thread_area.set(area);
        let w = (self.thread_fraction * (area - THREAD_GRIP)).min(area - THREAD_GRIP - MIN_PANE_W).max(MIN_PANE_W);
        let line = self.thread_drag.then(|| container(Space::new()).width(2).height(Fill).style(fill_box(self.pal.accent, 0.0)));
        let grip = mouse_area(container(row![Space::new().width(THREAD_GRIP - 2.0)].push(line)).width(THREAD_GRIP).height(Fill))
            .on_press(UiMessage::Thread(Msg::Drag(None)))
            .on_double_click(UiMessage::Thread(Msg::ResetWidth))
            .interaction(mouse::Interaction::ResizingHorizontally);
        let chat = container(self.view_conversation()).padding(pad.right(0.0)).width(Fill);
        row![chat, grip, container(self.view_thread_pane()).width(w + 8.0).padding(Padding::new(8.0).left(0.0))].into()
    }

    /// Desktop's `ThreadedRepliesPane`: the thread with its composer, which handles its own messages, under the
    /// header with "More Options" and "Close".
    fn view_thread_pane(&self) -> El<'_> {
        let p = &self.pal;
        self.drawing_thread.set(true);
        let view = Element::from(self.view_conversation()).map(|m| UiMessage::InThread(Box::new(m)));
        self.drawing_thread.set(false);
        let button = |glyph: char, label: &'static str, msg: UiMessage| -> El<'_> {
            let b = button(container(ic(glyph, 16.0).color(p.strong)).center(28))
                .padding(0)
                .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 8.0))
                .on_press(msg);
            self.tip(b.into(), tr(label), vec![], tooltip::Position::Bottom).into()
        };
        // `ThreadedRepliesHeader-module__title` and its buttons are `--color-on-surface`.
        let title = container(semi(tr("Thread"), 14.0).line_height(lh(18.0)).color(p.strong)).center_x(Fill);
        let header = row![button(icon::MORE_HORIZ, "More Options", UiMessage::Thread(Msg::Menu)), title]
            .push(button(icon::CLOSE, "Close", UiMessage::Thread(Msg::Hide)))
            .spacing(2)
            .padding([0, 4])
            .height(36)
            .align_y(Center);
        let header = mouse_area(header).on_right_press(UiMessage::Thread(Msg::Menu));
        stack![view, header].into()
    }

    /// Desktop's `threadedRepliesMenu` of the thread pane at `at`.
    pub(crate) fn view_thread_menu(&self, at: Point) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let thread = self.pane().thread.as_deref().expect("a thread pane");
        let (cid, root) = (thread.selected_chat_id.clone().expect("a chat"), thread.thread_root.clone().expect("a root"));
        let st = self.store.read();
        let chat = &st.chats[st.resolve_chat_idx(&cid).expect("a known chat")];
        let (unread, follow) = (st.thread_unread(chat, &root).0 > 0, self.follow_item(&st, chat, &root));
        drop(st);
        let view = UiMessage::Commands(vec![
            UiMessage::CloseModals,
            UiMessage::CommandBar(commands::Msg::JumpTo(cid.clone(), root.clone())),
            UiMessage::Message(message::Msg::Select(vec![root.clone()])),
        ]);
        let read = unread.then(|| {
            [
                (menu_item(p, None, tr("Mark as Read").into(), "", Some(UiMessage::Thread(Msg::MarkRead))), menu_pitch()),
                (menu_rule(p), MENU_RULE),
            ]
        });
        let window = UiMessage::Commands(vec![UiMessage::Thread(Msg::Hide), UiMessage::Thread(Msg::OpenWindow(cid.clone(), root.clone()))]);
        let rows: Vec<(El, f32)> = read
            .into_iter()
            .flatten()
            .chain([
                (follow, menu_pitch()),
                (menu_item(p, None, tr("Open in New Window").into(), "", Some(window)), menu_pitch()),
                (menu_item(p, None, tr("View in Chat").into(), "", Some(view)), menu_pitch()),
            ])
            .collect();
        let (menu, h) = crate::menu_rows(p, rows, 233);
        let menu = menu;
        let (x, y) = crate::menu_spot(at, 233.0, h, self.pane().size);
        (menu.into(), x, y)
    }

    /// Desktop's "Unread threads" pill at the top of the chat list: the count, with "@" for a mention.
    pub(crate) fn unread_threads_pill<'a>(&'a self, st: &BeeperStore, threads: &[(usize, String)]) -> El<'a> {
        let p = &self.pal;
        // The mentions of a thread read SQLite, so they are kept until the store changes.
        let mentions = |i: usize, r: &str| self.count(&format!("thread mentions {i} {r}"), || st.thread_unread(&st.chats[i], r).1);
        let at = threads.iter().any(|(i, r)| mentions(*i, r) > 0);
        // `.pillCount` is `1lh` (12 px) high.
        let count = crate::count_pill(crate::unread_label(threads.len()), at, (p.accent, Color::WHITE), 12.0);
        let label = container(semi(tr("Unread threads"), 13.0).color(p.strong)).width(Fill);
        let bg = if self.unread_view { p.selected } else { CLEAR };
        let edge = if self.unread_view { CLEAR } else { p.border_tertiary };
        // The padding of `.pill` inside its 1 px border, which iced draws over the padding; it has no hover style.
        let pill = button(row![label, count].spacing(8).align_y(Center))
            .padding(pad(7.0, 9.0, 7.0, 13.0))
            .width(Fill)
            .style(bordered_btn([bg; 3], [p.strong; 3], edge, 1.0, 999.0))
            .on_press(UiMessage::Thread(Msg::ShowUnread));
        let pill = mouse_area(pill).on_right_press(UiMessage::Thread(Msg::UnreadMenu(true)));
        // A `<button>` of Desktop, whose focus does not show.
        let keys = crate::focus::Keys::press(UiMessage::Thread(Msg::ShowUnread));
        let pill = self.target(self.drawing.get().or(self.main_window), "unread-threads", pill, keys, crate::focus::Look::Hidden);
        container(pill).padding([4, 0]).into()
    }

    /// The menu of the "Unread threads" pill at `at`.
    pub(crate) fn view_unread_menu(&self, at: Point) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let n = self.unread_threads(&st, true).len();
        let label = trn("Mark %d Thread as Read", "Mark %d Threads as Read", n, &[&n]);
        let col = column![menu_item(p, None, label, "", Some(UiMessage::Thread(Msg::MarkAllRead)))];
        let menu = crate::menu_card(p, col, 233);
        (menu.into(), at.x, at.y.min(self.pane().size.height - 8.0 - menu_pitch() - 8.0))
    }

    /// Desktop's `UnreadThreads` view: the unread followed threads as cards, newest first, and the thread pane of the
    /// open one.
    pub(crate) fn view_unread_threads(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let threads = self.unread_threads(&st, true);
        let open = self.pane.thread.as_ref().map(|t| (t.selected_chat_id.clone(), t.thread_root.clone()));
        let title = semi(tr("Unread threads"), 14.0).line_height(lh(18.0)).color(p.text);
        let count = (!threads.is_empty()).then(|| container(semi(threads.len().to_string(), 14.0).color(p.tertiary)).padding([0, 6]));
        let header = row![container(title).width(Fill)].push(count).align_y(Center).padding([12, 16]);
        let empty = threads.is_empty().then(|| container(txt(tr("You’re all caught up."), 13.0).color(p.sub)).padding([0, 16]));
        let cards = threads.iter().map(|(i, root)| {
            let chat = &st.chats[*i];
            let selected = open.as_ref().is_some_and(|(c, r)| c.as_deref() == Some(&chat.id) && r.as_deref() == Some(root));
            self.unread_thread_card(&st, chat, root, selected)
        });
        let list = scrollable(Column::with_children(cards).padding(Padding::new(8.0).top(0.0))).height(Fill).style(scroll_style(p.scroll));
        drop(st);
        let view = column![header].push(empty).push(list);
        let view = container(view).width(Fill).height(Fill).style(fill_box(p.card, 10.0));
        let pane: El<'_> = match self.pane.thread.is_some() {
            true => self.view_thread_pane(),
            false => container(txt(tr("Select a thread"), 13.0).color(p.sub)).center(Fill).style(fill_box(p.card, 10.0)).into(),
        };
        row![view, container(pane).width(Fill)].spacing(THREAD_GRIP).into()
    }

    /// Desktop's `UnreadThreadCard`: the chat and the first replier, the repliers and the time of the newest reply,
    /// the chat and the root, the unread count, and the newest reply.
    fn unread_thread_card<'a>(&'a self, st: &BeeperStore, chat: &Chat, root: &str, selected: bool) -> El<'a> {
        let p = &self.pal;
        let replies = st.stored_replies(&chat.id, root);
        let read = chat.thread_reads.get(root).cloned().unwrap_or_default();
        // Desktop's `unreadSenderIDs`: the senders of the unread replies, the newest first.
        let mut senders: Vec<&str> = Vec::new();
        for m in replies.iter().rev().filter(|m| m.sort_key > read && m.is_sender != Some(true) && m.is_deleted != Some(true)) {
            if !senders.contains(&m.sender_id.as_str()) {
                senders.push(&m.sender_id);
            }
        }
        let (unread, at) = st.thread_unread(chat, root);
        let user = |id: &str| chat.participants.items.iter().find(|u| u.id == id);
        let name = |id: &str| user(id).and_then(|u| u.full_name.clone()).unwrap_or_else(|| id.to_string());
        let repliers = match senders[..] {
            [] => trn("%d new reply", "%d new replies", unread, &[&unread]),
            [a] => name(a),
            [a, b] => trf("%1$s & %2$s", &[&name(a), &name(b)]),
            [a, ..] => {
                let n = senders.len() - 1;
                trn("%1$s & %2$d other", "%1$s & %2$d others", n, &[&name(a), &n])
            }
        };
        let newest = replies.last().expect("an unread thread has replies");
        let first = senders
            .first()
            .and_then(|id| user(id))
            .map(|u| self.face(crate::user_img(u).as_deref(), u.full_name.as_deref().unwrap_or(&u.id), 32.0, 0.5));
        let avatars = row![self.chat_avatar(chat, 32.0, 0.0)].push(first.map(|f| container(f).padding(Padding::ZERO.left(-8.0)))).width(56);
        let message = |m: Option<&Message>| m.and_then(preview_body).unwrap_or_else(|| tr("Message").into());
        let root_text = message(st.stored_message(root).as_ref());
        let time = txt(format_short_ts(&newest.timestamp, st.settings.pref("timeFormat") == 1), 11.0).color(p.sub);
        let top = row![ell(&repliers, 180.0, 13.0, Weight::Normal).color(p.sub), Space::new().width(Fill), time].spacing(8);
        let badge = thread_badge(p.accent, unread, at);
        let icon = svg(brand_svg(brand_key(&chat.account_id))).width(16).height(16);
        let line = row![
            icon,
            ell(&format!("{}:", chat.title), 120.0, 13.0, Weight::Normal).color(p.sub),
            ell(&root_text, 140.0, 13.0, Weight::Normal).color(p.sub)
        ]
        .spacing(4)
        .align_y(Center);
        let middle = row![container(line).width(Fill).clip(true)].push(badge).spacing(8).align_y(Center);
        let preview = ell(&message(Some(newest)), 300.0, 13.0, Weight::Normal).color(p.text);
        let content = column![top, middle, container(preview).padding(Padding::ZERO.top(8.0))].spacing(2).width(Fill);
        let bg = if selected { p.selected } else { CLEAR };
        button(row![avatars, content].spacing(12).align_y(Center))
            .padding(8)
            .width(Fill)
            .style(btn_style([bg, p.hover, p.hover], [p.text; 3], 10.0))
            .on_press(UiMessage::Thread(Msg::Open(chat.id.clone(), root.into())))
            .into()
    }

    /// Desktop's `ThreadRepliesButton` under the root of a thread: up to 3 repliers, the reply count, the unread count
    /// of a followed thread with a mention mark, and a chevron.
    pub(crate) fn thread_replies_button<'a>(&'a self, st: &BeeperStore, chat: &Chat, root: &str) -> Option<El<'a>> {
        let p = &self.pal;
        let replies: Vec<&Message> = st.thread_replies(root).collect();
        if replies.is_empty() {
            return None;
        }
        let mut seen = HashSet::new();
        let faces = replies
            .iter()
            .filter(|m| seen.insert(m.sender_id.as_str()))
            .filter_map(|m| chat.participants.items.iter().find(|u| u.id == m.sender_id));
        let faces = faces.take(3).map(|u| self.face(crate::user_img(u).as_deref(), u.full_name.as_deref().unwrap_or(&u.id), 16.0, 0.5));
        let n = replies.len();
        let label = semi(trn("%d reply", "%d replies", n, &[&n]), 11.0).line_height(lh(15.0)).color(p.text);
        let (unread, at) = if st.follows_thread(chat, root) { st.thread_unread(chat, root) } else { (0, 0) };
        let badge = thread_badge(p.accent, unread, at);
        let chevron = container(tinted(&CHEVRON_FORWARD, 14.0, p.tertiary)).padding(Padding::ZERO.right(2.0));
        let line = Row::with_children(faces).push(label).push(badge).push(chevron).spacing(4).align_y(Center);
        let ring = if self.wallpaper_fg(st, chat, false).is_some() { CLEAR } else { p.card };
        let b = button(line)
            .padding([6, 7])
            .style(bordered_btn([p.received, p.selected, p.selected], [p.text; 3], ring, 2.0, 14.0))
            .on_press(UiMessage::Thread(Msg::Show(root.into())));
        Some(b.into())
    }

    /// Desktop's `ThreadRootLine` above a reply that is also sent to the chat: "Replying to:" and the root.
    pub(crate) fn thread_root_line<'a>(&'a self, st: &BeeperStore, root: &str, mine: bool) -> El<'a> {
        let p = &self.pal;
        let preview = st.loaded_idx(root).and_then(|i| preview_body(&st.messages[i])).unwrap_or_default();
        let label = format!("{} ", tr("Replying to:"));
        let room = 300.0 - 2.0 * BUBBLE_PAD_X - crate::style::text_width(&label, 11.0, Weight::Semibold);
        let line = row![semi(label, 11.0).color(p.sub), ell(&preview, room, 11.0, Weight::Normal).line_height(lh(14.0)).color(p.sub)];
        let line = container(line).max_width(300).clip(true).padding(Padding::new(0.0).left(BUBBLE_PAD_X).right(BUBBLE_PAD_X).bottom(2.0));
        let line = mouse_area(line).on_press(UiMessage::Thread(Msg::Show(root.into())));
        container(line).width(Fill).align_x(if mine { Right } else { iced::Left }).into()
    }

    /// Desktop's `ThreadRepliesDivider` under the root in the thread pane: "%d reply" between two lines.
    pub(crate) fn thread_divider<'a>(&self, n: usize) -> El<'a> {
        let p = &self.pal;
        let line = || container(Space::new()).width(Fill).height(1).style(fill_box(p.rule, 0.0));
        let label = semi(trn("%d reply", "%d replies", n, &[&n]), 11.0).color(p.sub);
        row![line(), label, line()].spacing(6).align_y(Center).padding([8, 0]).into()
    }
}

/// The unread count of a thread on an accent pill, with an "@" when a reply mentions the user.
fn thread_badge<'a>(accent: Color, unread: usize, at: usize) -> Option<El<'a>> {
    let mark = (at > 0).then(|| semi("@", 10.0).color(Color::WHITE));
    let count = semi(unread.to_string(), 10.0).color(Color::WHITE);
    let pill = row![].push(mark).push(count).spacing(2).align_y(Center);
    (unread > 0).then(|| container(pill).center_y(16).padding([0, 5]).style(fill_box(accent, 16.0)).into())
}

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `showThreadedReplies`: opens the thread pane of a message, or of the thread of a reply.
    Show(String),
    /// Desktop's `hideThreadedReplies`.
    Hide,
    /// Desktop's `setBroadcastOutsideThread`: "Also send to chat".
    SetBroadcast(bool),
    /// The thread pane edge follows the pointer from a press, to the point, until the release (`None`).
    Drag(Option<Point>),
    /// Desktop's double click on the thread pane edge: the default width.
    ResetWidth,
    /// Desktop's "More Options" of the thread pane.
    Menu,
    /// Desktop's `markReplyThreadReadThrough` of the thread pane.
    MarkRead,
    /// Desktop's `showUnreadThreads`: the Unread threads view in place of the chat.
    ShowUnread,
    /// Desktop's unread thread card: the thread pane of the root of the chat.
    Open(String, String),
    /// The menu of the "Unread threads" pill, or `None` to close it.
    UnreadMenu(bool),
    /// Desktop's `markAllUnreadThreadsAsRead`.
    MarkAllRead,
    /// Desktop's `setThreadFollowing` of the chat, the root and the follow.
    Follow(String, String, bool),
    /// Desktop's `setFollowAllThreads` of the chat.
    FollowAll(String, bool),
    /// Desktop's "Open in New Window" of a thread: the window "Thread in %s" of the chat and the root.
    OpenWindow(String, String),
}

impl BeeperGuiApp {
    pub(crate) fn threads(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Show(id) => self.show_thread(&id),
            Msg::Hide => (self.pane.thread, self.pane.thread_menu) = (None, None),
            Msg::SetBroadcast(on) => self.pane.broadcast = on,
            Msg::Drag(at) => self.drag_thread(at),
            Msg::ResetWidth => self.set_thread_fraction(THREAD_FRACTION),
            Msg::Menu => self.pane.thread_menu = Some(self.cursor.get()),
            Msg::MarkRead => self.mark_thread_read(),
            Msg::ShowUnread => self.show_unread_threads(),
            Msg::Open(chat, root) => self.open_thread(chat, root),
            Msg::UnreadMenu(open) => self.unread_menu = open.then(|| self.cursor.get()),
            Msg::MarkAllRead => self.mark_all_threads_read(),
            Msg::Follow(chat, root, on) => self.follow_thread(&chat, &root, on),
            Msg::FollowAll(chat, on) => self.follow_all_threads(&chat, on),
            Msg::OpenWindow(chat, root) => return self.open_thread_window(chat, root),
        }
        self.settle()
    }
}

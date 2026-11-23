//! Desktop's forward dialog: the chats to forward the selected messages to.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// Open the forward dialog for messages, in timeline order.
    Open(Vec<String>),
    Query(String),
    /// Add the first chat that matches the query.
    Submit,
    Pick(String),
    Send,
}

impl BeeperGuiApp {
    pub(crate) fn forward(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open(mids) => {
                self.forward = Some(Forward { mids, picked: Vec::new(), query: String::new() });
                return operation::focus("forward-input");
            }
            Msg::Query(q) => self.forward.as_mut().expect("the forward dialog is open").query = q,
            Msg::Submit => {
                let st = self.store.read();
                let first = forward_chats(&st, self.forward.as_ref().expect("the forward dialog is open"));
                let first = first.first().map(|c| c.id.clone());
                drop(st);
                return first.map_or_else(Task::none, |c| self.update(UiMessage::Forward(Msg::Pick(c))));
            }
            Msg::Pick(cid) => {
                let f = self.forward.as_mut().expect("the forward dialog is open");
                f.picked.push(cid);
                f.query.clear();
                return operation::focus("forward-input");
            }
            Msg::Send => {
                // The second click of a double click on Send hits the dialog that the first click closed.
                let Some(Forward { mids, picked, .. }) = self.forward.take() else { return Task::none() };
                assert!(!picked.is_empty() && !mids.is_empty(), "forward needs a chat and a message");
                let mut st = self.store.write();
                let mut failed = Vec::new();
                for cid in &picked {
                    for mid in &mids {
                        // Desktop forwards a copy of a scheduled message, without its reply.
                        if let Some(s) = st.scheduled.iter().find(|s| s.id == *mid).cloned() {
                            match crate::composer::scheduled_draft(&mut self.media, &mut st, &s, None) {
                                Ok(draft) => drop(st.send(cid, draft).expect("forward a scheduled message")),
                                Err(e) => failed.push(e),
                            }
                            continue;
                        }
                        let m = stored(&st, mid);
                        let (_, txn, _) = st
                            .send_message(cid, m.text.clone(), None, m.attachments.first().cloned())
                            .expect("forward message");
                        let (from, event, text, html) = (m.chat_id, m.id, m.text, m.html);
                        st.queue(cid, Outgoing::Forward { txn, from, event, text, html });
                    }
                }
                drop(st);
                self.fail_all(failed);
                if let [one] = picked.as_slice() {
                    self.select_chat(one);
                }
            }
        }
        self.settle()
    }
}

/// The forward dialog: the messages in timeline order, the picked chats, and the search text.
pub(crate) struct Forward {
    pub(crate) mids: Vec<String>,
    pub(crate) picked: Vec<String>,
    pub(crate) query: String,
}

/// The chats the forward dialog offers: recent chats, or all chats that match the query.
pub(crate) fn forward_chats<'s>(st: &'s BeeperStore, f: &Forward) -> Vec<&'s Chat> {
    let q = f.query.trim().to_lowercase();
    let mut v: Vec<&Chat> = match q.is_empty() && !st.recent_chat_ids.is_empty() {
        true => st.recent_chat_ids.iter().filter_map(|id| st.find_chat(id)).collect(),
        false => {
            let hit =
                |c: &&Chat| c.title.to_lowercase().contains(&q) || chat_search_subtitle(c).to_lowercase().contains(&q);
            let mut v: Vec<&Chat> = st.chats.iter().filter(hit).collect();
            v.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
            v
        }
    };
    v.retain(|c| c.flags & CHAT_HIDDEN == 0 && !f.picked.contains(&c.id));
    v
}

impl BeeperGuiApp {
    pub(crate) fn view_forward<'a>(&'a self, f: &'a Forward) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let shade = Color { a: 0.1, ..p.strong };
        let strip = |content: El<'a>, r| container(content).width(Fill).padding([8, 13]).style(fill_box(p.chip, r));
        let head = row![tinted(&MAT_FORWARD, 16.0, p.strong), txt(tr("Forward message"), 14.0).color(p.strong)];
        let head = strip(head.spacing(12).align_y(Center).into(), border::Radius::default().top(18)).center_y(44);
        let pills = f.picked.iter().map(|cid| -> El<'a> {
            let c = st.chat(cid);
            let label = semi(c.title.clone(), 14.0).line_height(Pixels(rem(16.0))).color(p.strong);
            let pill = row![self.chat_avatar(c, 16.0, 0.0), label].spacing(4).align_y(Center);
            container(pill).padding(Padding::new(2.0).right(8.0)).style(fill_box(shade, 20.0)).into()
        });
        let input = text_input(tr("Search name, @, or number"), &f.query)
            .id("forward-input")
            .on_input(|a0| UiMessage::Forward(forward::Msg::Query(a0)))
            .size(rem(14.0))
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let inputs = Row::with_children(pills).push(input).spacing(4).align_y(Center).height(24);
        let inputs = container(inputs).padding(Padding::new(18.0).left(13.0).right(20.0));
        let chats = forward_chats(&st, f);
        let cat = (f.query.trim().is_empty() && !chats.is_empty()).then(|| {
            container(txt(tr("Recent chats"), 12.0).color(p.sub))
                .padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
        });
        let rows = chats.iter().enumerate().fold(Column::new().push(cat), |col, (i, c)| {
            let title = crate::chat_search_title(p, c);
            let line = row![self.chat_avatar(c, 32.0, 0.0), title, Space::new().width(Fill)]
                .push(svg(brand_svg(brand_key(&c.account_id))).width(16).height(16))
                .spacing(8)
                .align_y(Center);
            let bg = if i == 0 { p.chip } else { CLEAR };
            let pick = button(line).height(36).padding([0, 6]).width(Fill);
            let pick = pick.style(btn_style([bg, p.chip, p.selected], [p.text; 3], 8.0));
            let pick = mouse_area(pick.on_press(UiMessage::Forward(forward::Msg::Pick(c.id.clone()))));
            col.push(
                container(pick.on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(c.id.clone()))))
                    .padding([0, 6]),
            )
        });
        let ready = !f.picked.is_empty();
        let label = row![semi(tr("Forward"), 12.0), txt("CTRL ENTER", 11.0).color(Color { a: 0.8, ..Color::WHITE })];
        let send = button(label.spacing(4).align_y(Center))
            .padding([6, 8])
            .style(btn_style([if ready { p.accent } else { shade }; 3], [Color::WHITE; 3], 100.0))
            .on_press_maybe(ready.then_some(UiMessage::Forward(forward::Msg::Send)));
        let note = tr("Forwarding messages through Beeper will not include any attribution on the native apps.");
        let foot = row![txt(note, 14.0).color(p.sub).width(Fill), send].spacing(12).align_y(Center);
        let body = column![
            head,
            inputs,
            scrollable(rows).direction(bar()).height(300).style(scroll_style(p.scroll)),
            container(Space::new()).width(Fill).height(2).style(fill_box(shade, 0.0)),
            strip(foot.into(), border::Radius::default().bottom(18)),
        ];
        container(body).width(602).style(glow_box(p.card, Color { a: 0.18, ..p.strong }, 18.0, 4.0, 84.0)).into()
    }
}

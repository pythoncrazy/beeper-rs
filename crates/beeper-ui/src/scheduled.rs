//! Desktop's Send Later: the time of the composer, and the scheduled messages with their menu.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `showWithSubMenu(SCHEDULE_MESSAGE)` for the composer.
    Open,
    /// The composer's `sendLaterConfig`: the time and "If no reply".
    Set(Option<(i64, bool)>),
    /// The menu of scheduled messages at the cursor; true adds the message items.
    Menu(Vec<String>, bool),
    SendNow(Vec<String>),
    Reschedule(Vec<String>),
    RescheduleTo(Vec<String>, i64, bool),
    Unschedule(Vec<String>),
    Edit(String),
    /// Desktop's Forward... of a scheduled message, which forwards a copy of it.
    Forward(String),
    Fold(bool),
    Check,
}

impl BeeperGuiApp {
    pub(crate) fn scheduled(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open if !self.store.read().feature("scheduled-message").on => {}
            Msg::Open if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Send Later isn't available while editing a message").into();
            }
            Msg::Open => {
                if self.pane.selected_chat_id.is_some() {
                    return self.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Schedule(Vec::new()))));
                }
            }
            Msg::Set(config) => {
                self.pane.send_later = config;
                return operation::focus("composer-input");
            }
            Msg::Menu(ids, items) => self.pane.sched_menu = Some((ids, items, self.cursor.get())),
            Msg::SendNow(ids) => {
                self.pane.sched_menu = None;
                let mut st = self.store.write();
                let due: Vec<Scheduled> = ids.iter().filter_map(|id| st.scheduled.iter().find(|s| s.id == *id)).cloned().collect();
                ids.iter().for_each(|id| st.update_scheduled(id, |_| false));
                drop(st);
                self.send_scheduled(due);
            }
            Msg::Reschedule(ids) => {
                self.pane.sched_menu = None;
                return self.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Schedule(ids))));
            }
            Msg::RescheduleTo(ids, at, no_reply) => {
                let mut st = self.store.write();
                let now = now_unix_millis();
                for id in &ids {
                    st.update_scheduled(id, |s| {
                        (s.canceled, s.added_on, s.send_on, s.send_only_on_no_response) = (false, now, at, no_reply);
                        true
                    });
                }
            }
            Msg::Unschedule(ids) => {
                self.pane.sched_menu = None;
                let mut st = self.store.write();
                ids.iter().for_each(|id| st.update_scheduled(id, |_| false));
            }
            Msg::Edit(id) => {
                self.pane.sched_menu = None;
                let st = self.store.read();
                let text = st.scheduled.iter().find(|s| s.id == id).expect("a scheduled message").text.clone();
                drop(st);
                return self.update(UiMessage::Composer(composer::Msg::StartEdit(id, text)));
            }
            Msg::Forward(id) => {
                self.pane.sched_menu = None;
                return self.update(UiMessage::Forward(forward::Msg::Open(vec![id])));
            }
            Msg::Fold(collapsed) => {
                self.pane.sched_fold.0 = collapsed;
                return operation::scroll_to("timeline", operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            Msg::Check => {
                let now = now_unix_millis();
                if let Some((_, no_reply)) = self.pane.send_later.filter(|c| c.0 <= now) {
                    self.pane.send_later = Some((now + 60_000, no_reply));
                }
                let due = self.store.write().take_due_scheduled(now);
                self.send_scheduled(due);
            }
        }
        self.settle()
    }
}

/// Desktop's `nextScheduledMessageTime`.
pub(crate) fn next_send(st: &BeeperStore, chat_id: &str) -> Option<i64> {
    st.scheduled.iter().filter(|s| s.chat_id == chat_id).map(|s| s.send_on).min()
}

/// Desktop's `qN`: when a Send Later message sends.
pub(crate) fn send_day(at: i64, h24: bool) -> String {
    let (d, t, time) = (day_of(at), today(), clock(at, h24));
    match d - t {
        0 => trf("Today at %s", &[&time]),
        1 => trf("Tomorrow at %s", &[&time]),
        _ if (d + 3).div_euclid(7) == (t + 3).div_euclid(7) => trf("%1$s at %2$s", &[&tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]), &time]),
        _ => {
            let (y, m, day) = civil_from_days(d);
            trf("%1$s at %2$s", &[&format!("{m}/{day}/{y}"), &time])
        }
    }
}

impl BeeperGuiApp {
    /// Desktop's scheduled message menu: with `items`, Edit, Copy and Forward, then `r0e`.
    pub(crate) fn view_scheduled_menu(&self, ids: &[String], items: bool) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let first = st.scheduled.iter().find(|s| s.id == ids[0]).expect("a scheduled message");
        let (canceled, n, ids) = (first.canceled, ids.len(), ids.to_vec());
        let key = |name| hotkey_text(st.settings.hotkey_keys(name));
        let item = |label: String, hotkey: &str, msg: UiMessage| Some(menu_item(p, None, label, hotkey.to_string(), Some(msg)));
        let ctx = "scheduled message menu action";
        let rows = [
            items.then(|| {
                item(trx("Edit...", ctx).into(), &key("Message: Edit"), UiMessage::Scheduled(scheduled::Msg::Edit(ids[0].clone())))
            }),
            items.then(|| item(trx("Copy", ctx).into(), "Ctrl C", UiMessage::Message(message::Msg::CopyText(first.text.clone())))),
            items.then(|| {
                let forward = UiMessage::Scheduled(scheduled::Msg::Forward(ids[0].clone()));
                item(trx("Forward...", "scheduled message menu action").into(), &key("Message: Forward"), forward)
            }),
            items.then(|| Some(menu_rule(p))),
            (!canceled).then(|| match n {
                1 => item(tr("Send Message Now").into(), "", UiMessage::Scheduled(scheduled::Msg::SendNow(ids.clone()))),
                _ => item(
                    trn("Send %d Message Now", "Send %d Messages Now", n, &[&n]),
                    "",
                    UiMessage::Scheduled(scheduled::Msg::SendNow(ids.clone())),
                ),
            }),
            // Desktop's Reschedule needs the Send Later feature.
            Some(Some(menu_item(
                p,
                None,
                tr("Reschedule").into(),
                "",
                st.feature("scheduled-message").on.then(|| UiMessage::Scheduled(scheduled::Msg::Reschedule(ids.clone()))),
            ))),
            Some(item(
                match (canceled, n) {
                    (true, _) => trn("Remove Unsent Message", "Remove Unsent Messages", n, &[]),
                    (false, 1) => tr("Delete Without Sending").into(),
                    (false, _) => trn("Delete %d Message", "Delete %d Messages", n, &[&n]),
                },
                "",
                UiMessage::Scheduled(scheduled::Msg::Unschedule(ids)),
            )),
        ];
        crate::menu_card(p, Column::with_children(rows.into_iter().flatten().flatten()), MESSAGE_MENU_W).into()
    }

    pub(crate) fn view_scheduled<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let mut all: Vec<&Scheduled> = st.scheduled.iter().filter(|s| s.chat_id == chat.id).collect();
        if all.is_empty() {
            return None;
        }
        all.sort_by_key(|s| s.send_on);
        let (canceled, pending): (Vec<&Scheduled>, Vec<&Scheduled>) = all.into_iter().partition(|s| s.canceled);
        let (collapsed, collapsible) = if pending.is_empty() { (false, false) } else { self.pane().sched_fold };
        let h24 = st.settings.pref("timeFormat") == 1;
        let fg = p.on_secondary;
        let bg = if self.wallpaper_fg(st, chat, false).is_some() { p.card.scale_alpha(0.85) } else { p.card };
        let minute = |s: &Scheduled| s.send_on.div_euclid(60_000);
        // Desktop's `gapBelow` of a scheduled message is 8 px.
        let list = |v: &[&Scheduled]| -> El<'a> {
            let rows = v.iter().enumerate().map(|(i, s)| {
                let first = i == 0 || (!s.canceled && minute(v[i - 1]) != minute(s));
                let date = first.then(|| {
                    let group = v.iter().filter(|o| s.canceled || minute(o) == minute(s));
                    let ids = group.map(|o| o.id.clone()).collect();
                    let (text, edit, weight) = match s.canceled {
                        true => (tr("Send Later message canceled by reply.").into(), tr("Options"), Weight::Normal),
                        false => (trf("Send %s", &[&send_day(s.send_on, h24)]), tr("Edit"), Weight::Semibold),
                    };
                    let edit = button(semi(edit, 11.0).color(p.accent))
                        .bare(p.accent)
                        .on_press(UiMessage::Scheduled(scheduled::Msg::Menu(ids, false)));
                    let line = row![]
                        .push((!s.canceled).then(|| tinted(&COMPOSER_ICONS[6], 12.0, fg)))
                        .push(txt(text, 11.0).font(fw(weight)).color(fg))
                        .push(edit);
                    container(line.spacing(4).align_y(Center)).center_x(Fill).padding(pad(4.0, 8.0, 8.0, 8.0))
                });
                column![].push(date).push(self.view_scheduled_bubble(st, s, bg)).into()
            });
            Column::with_children(rows).spacing(8).into()
        };
        let toggle = (collapsed || collapsible).then(|| {
            let n = pending.len();
            let text = match collapsed && n > 1 {
                true => trn("%d Message to Send Later", "%d Messages to Send Later", n, &[&n]),
                false => tr("Send Later").into(),
            };
            let chevron = svg(CHEVRON_DOWN.clone())
                .width(12)
                .height(12)
                .rotation(Radians(if collapsed { 0.0 } else { std::f32::consts::PI }))
                .style(tint(fg));
            let label = row![tinted(&COMPOSER_ICONS[6], 12.0, fg), semi(text, 11.0).color(fg), chevron];
            let toggle = button(label.spacing(4).align_y(Center))
                .padding([4, 8])
                .style(bordered_btn([p.card; 3], [fg; 3], p.chip, 1.0, 100.0))
                .on_press(UiMessage::Scheduled(scheduled::Msg::Fold(!collapsed)));
            container(toggle).center_x(Fill).padding(Padding::ZERO.bottom(4.0))
        });
        let shown = (!collapsed).then(|| list(&pending));
        let canceled = (!canceled.is_empty()).then(|| list(&canceled));
        let col = column![].push(canceled).push(toggle).push(shown).spacing(2);
        Some(col.padding(Padding::ZERO.top(6.0)).into())
    }

    /// A scheduled message like Desktop's `is-scheduled` bubble: `bg`, with a dashed border in the text color.
    pub(crate) fn view_scheduled_bubble(&self, st: &BeeperStore, s: &Scheduled, bg: Color) -> El<'_> {
        let p = &self.pal;
        let fg = if s.canceled { p.muted } else { p.accent };
        let line = |t: String| txt(t, 13.0).wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        // A file part shows its file name over its caption.
        let file =
            s.upload.as_ref().and_then(|id| st.assets.get(id)).map(|a| line(a.file_name.clone()).font(fw(Weight::Semibold)).color(fg));
        let text = column![].push(file).push((!s.text.is_empty()).then(|| line(s.text.clone()).color(fg)));
        let bubble = container(text).padding([9.0, BUBBLE_PAD_X]).max_width(0.45 * self.pane().size.width).style(fill_box(bg, 17.0));
        let bubble = mouse_area(stack![bubble, dashed::dashes(fg, 17.0, 1.0)])
            .on_right_press(UiMessage::Scheduled(scheduled::Msg::Menu(vec![s.id.clone()], true)));
        row![Space::new().width(Fill), bubble].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::clicks;

    /// The messages of a click on `label` in the menu of the scheduled message `id`.
    fn click(app: &BeeperGuiApp, id: &str, label: &str) -> Vec<UiMessage> {
        clicks(app.view_scheduled_menu(&[id.to_string()], true), label)
    }

    /// Desktop's scheduled message menu: Forward... after Copy, which forwards a copy of the message, and Reschedule
    /// only with the Send Later feature.
    #[test]
    fn scheduled_menu_like_desktop() {
        let mut app = crate::fuzz::demo_app("scheduled-menu");
        let (chat, other) = {
            let st = app.store.read();
            let ids: Vec<String> = st.chats.iter().take(2).map(|c| c.id.clone()).collect();
            (ids[0].clone(), ids[1].clone())
        };
        app.store.write().schedule(&chat, "later".into(), None, (now_unix_millis() + 3_600_000, false), None);
        let id = app.store.read().scheduled[0].id.clone();
        let forward = click(&app, &id, "Forward...");
        assert!(matches!(&forward[..], [UiMessage::Scheduled(Msg::Forward(f))] if *f == id));
        forward.into_iter().for_each(|m| drop(app.update(m)));
        assert_eq!(app.forward.as_ref().map(|f| f.mids.clone()), Some(vec![id.clone()]));
        let _ = app.update(UiMessage::Forward(forward::Msg::Pick(other.clone())));
        let _ = app.update(UiMessage::Forward(forward::Msg::Send));
        let st = app.store.read();
        let copy = st.messages.iter().rfind(|m| m.chat_id == other).expect("the forwarded copy");
        assert_eq!(copy.text.as_deref(), Some("later"));
        assert_eq!(st.scheduled.len(), 1, "the scheduled message stays");
        drop(st);
        assert!(matches!(&click(&app, &id, "Reschedule")[..], [UiMessage::Scheduled(Msg::Reschedule(_))]));
        {
            let mut st = app.store.write();
            st.features.flags["features"]["*"]["scheduled-message"] = false.into();
            st.features.usage = serde_json::json!({ "scheduled-message": { "used": 5 } });
        }
        assert!(click(&app, &id, "Reschedule").is_empty(), "the spent free plan disables Reschedule");
    }
}

//! Desktop's chat reminders: Remind me, If no reply, the due reminders and the Reminders list.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Set(String, Option<i64>),
    If(String, i64, bool),
    Check,
    Menu,
    Order(u8),
    DismissAll,
}

impl BeeperGuiApp {
    pub(crate) fn reminders(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Set(cid, at) => self.set_reminder(&cid, at, None),
            Msg::If(cid, at, no_reply) => self.set_reminder(&cid, Some(at), Some(no_reply)),
            Msg::Check => {
                let now = now_unix_millis();
                let mut st = self.store.write();
                for i in 0..st.chats.len() {
                    let c = &st.chats[i];
                    let Some(at) = c.reminder.as_ref().map(|r| r.remind_at_ms).filter(|&at| at <= now) else {
                        continue;
                    };
                    if self.reminded.insert(c.id.clone(), at) == Some(at) {
                        continue;
                    }
                    let flags = [ChatFlag::LowPriority, ChatFlag::Archived].map(|f| (f, c.flag(f)));
                    let ts = rfc3339_from_millis(at);
                    st.update_chat(i, |c| c.last_activity = c.last_activity.take().max(Some(ts)));
                    for (flag, _) in flags.into_iter().filter(|f| f.1) {
                        st.set_chat_flag(i, flag, Some(false));
                    }
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
            Msg::Menu => {
                self.reminder_menu = Some(self.cursor.get());
            }
            Msg::Order(i) => {
                self.reminder_menu = None;
                self.patch_settings(|s| s.reminders_order = i);
            }
            Msg::DismissAll => {
                self.confirm = None;
                let mut st = self.store.write();
                for i in 0..st.chats.len() {
                    if st.chats[i].reminder.is_some() {
                        st.set_reminder(i, None);
                    }
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
        }
        self.settle()
    }
}

pub(crate) const REMINDER_ORDERS: [&str; 4] = [
    "Order by Least Time Remaining",
    "Order by Most Time Remaining",
    "Order by Oldest Message",
    "Order by Newest Message",
];

pub(crate) fn remind_label(at: i64, h24: bool) -> String {
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{}, {:0>8}", &wd[..3], clock(at, h24))
}

/// The day of a reminder time, like Desktop's `Today` or `Tomorrow`.
pub(crate) fn reminder_day(at: i64) -> String {
    let (d, t) = (day_of(at), today());
    match d - t {
        -1 => tr("Yesterday").into(),
        0 => tr("Today").into(),
        1 => tr("Tomorrow").into(),
        _ if (d + 3).div_euclid(7) == (t + 3).div_euclid(7) => tr(WEEKDAYS[(d + 4).rem_euclid(7) as usize]).into(),
        _ => i18n::date(i18n::Date::Numeric, civil_from_days(d), (0, 0)),
    }
}

impl BeeperGuiApp {
    pub(crate) fn view_reminder_menu(&self) -> El<'_> {
        let p = &self.pal;
        let order = self.store.read().settings.reminders_order;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let orders = REMINDER_ORDERS.iter().zip(0..).map(|(label, i)| {
            menu_item(p, lead(order == i), tr(label).into(), "", Some(UiMessage::Reminders(reminders::Msg::Order(i))))
        });
        let ask = Some(UiMessage::Ask(Some(ASK_DISMISS_REMINDERS)));
        let col = Column::with_children(
            orders.chain([menu_rule(p), menu_item(p, lead(false), tr("Dismiss All Reminders").into(), "", ask)]),
        );
        container(col.padding([4, 0])).width(291).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into()
    }

    pub(crate) fn set_reminder(&mut self, cid: &str, at: Option<i64>, no_reply: Option<bool>) {
        {
            let mut st = self.store.write();
            let idx = st.resolve_chat_idx(cid).expect("chat exists");
            self.remind(&mut st, idx, at, no_reply);
        }
        self.chat_menu = None;
        self.refresh_theme_and_tray();
    }

    /// Desktop's `setThreadReminder`: the reminder ends on a reply unless `no_reply` or else "Default for remind later" is "Always",
    /// and the chat is archived when "Archive after setting reminder on chat" is on.
    pub(crate) fn remind(&self, st: &mut BeeperStore, idx: usize, at: Option<i64>, no_reply: Option<bool>) {
        let dismiss_on_incoming_message = no_reply.unwrap_or(st.settings.pref("remindLaterDefault") == 0);
        st.set_reminder(idx, at.map(|remind_at_ms| ChatReminder { remind_at_ms, dismiss_on_incoming_message }));
        let c = &st.chats[idx];
        if at.is_some() && st.settings.on("archiveOnReminder") && c.flags & (CHAT_PINNED | CHAT_REQUEST) == 0 {
            self.set_archived(st, idx, true);
        }
    }
}

/// Desktop's default `time_suggestion_options`.
pub(crate) const TIME_SUGGESTIONS: [&str; 6] =
    ["In an Hour", "In Two Hours", "This Evening", "Tomorrow", "This Weekend", "Next Week"];

/// Desktop's `wJe`: the time suggestions that fall in the future, with their times.
pub(crate) fn remind_presets(now: i64) -> impl Iterator<Item = (&'static str, i64)> {
    TIME_SUGGESTIONS.into_iter().filter_map(move |s| parse_when_millis(s, now).filter(|at| *at > now).map(|at| (s, at)))
}

/// Desktop's `dD`: the last time phrase that set a reminder, read again now, when it falls in the future.
pub(crate) fn last_used(st: &BeeperStore, now: i64) -> Option<i64> {
    Some(&st.settings.last_time)
        .filter(|t| !t.is_empty())
        .and_then(|t| parse_when_millis(t, now))
        .filter(|at| *at > now)
}

/// Desktop's "Last Used" time: `EEE dd MMM`, with the year when it is not this year, and the time.
pub(crate) fn last_used_label(at: i64, h24: bool) -> String {
    let (y, m, d) = civil_from_days(day_of(at));
    let month = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(' ').nth(m as usize - 1).expect("a month");
    let year = if y == civil_from_days(today()).0 { String::new() } else { format!(" {y}") };
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{} {d:02} {month}{year}, {:0>8}", &wd[..3], clock(at, h24))
}

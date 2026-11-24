//! Desktop's reminders: Remind me, If no reply, the message reminders, the due reminders and the Reminders list, and
//! the snoozed chats.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Set(String, Option<i64>),
    If(String, i64, bool),
    /// Desktop's `wD`: sets the reminder of a message, or marks it as done.
    Message(String, Option<i64>),
    /// The Remind Me submenu of the message menu opens.
    MessageSub,
    /// Desktop's `yb`: snoozes the chats until a time, or dismisses their snooze.
    Snooze(Vec<String>, Option<i64>),
    /// Desktop's `snooze_time_suggestion_last_used`: the time phrase that snoozed a chat.
    SnoozeTime(String),
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
            Msg::Message(mid, at) => {
                let mut st = self.store.write();
                let chat = st.stored_message(&mid).map(|m| m.chat_id);
                let chat = chat.or_else(|| st.message_reminders.get(&mid).map(|r| r.room_id.clone()));
                st.set_message_reminder(&chat.expect("the message of a reminder is known"), &mid, at);
            }
            Msg::MessageSub => {
                self.pane.remind_sub = true;
                self.video.iter_mut().for_each(|v| v.sub = false);
                (self.audio.sub, self.audio.lang) = (false, false);
            }
            Msg::Snooze(ids, until) => {
                let mut st = self.store.write();
                let chats: Vec<usize> = ids.iter().filter_map(|id| st.resolve_chat_idx(id)).collect();
                for i in chats {
                    // Desktop's dismissal leaves the chats that are not snoozed.
                    if until.is_some() || st.chats[i].snoozed_until_ms.is_some() {
                        st.set_snooze(i, until);
                    }
                }
                drop(st);
                self.list.chat_menu = None;
                self.refresh_theme_and_tray();
            }
            Msg::SnoozeTime(phrase) => self.patch_settings(|s| s.snooze_last_time = phrase),
            Msg::Check => {
                let now = now_unix_millis();
                let mut notes = self.message_reminders_due(now);
                notes.extend(self.snoozes_ended(now));
                let due = self.chat_reminders_due(now);
                let body = || trx("Reminder", "thread reminder notification body").to_string();
                notes.extend(due.iter().map(|c| self.notify_thread(c, "", body())));
                self.refresh_theme_and_tray();
                return Task::batch(notes.into_iter().chain([self.settle()]));
            }
            Msg::Menu => self.list.reminder_menu = Some(self.cursor.get()),
            Msg::Order(i) => {
                self.list.reminder_menu = None;
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
                let on = message_reminders_on(&st);
                let messages: Vec<(String, String)> = st.message_reminders.iter().map(|(id, r)| (r.room_id.clone(), id.clone())).collect();
                for (chat, id) in messages.into_iter().filter(|_| on) {
                    st.set_message_reminder(&chat, &id, None);
                }
                drop(st);
                self.refresh_theme_and_tray();
            }
        }
        self.settle()
    }

    /// Desktop's `onReminderElapsed`: each chat reminder that came due, also while the app was closed, moves its chat
    /// back to the inbox once, and is the chat to notify "Reminder" for. Its `userRemindedAt` stays in the chat.
    fn chat_reminders_due(&mut self, now: i64) -> Vec<Chat> {
        let mut st = self.store.write();
        let mut due = Vec::new();
        for i in 0..st.chats.len() {
            let c = &st.chats[i];
            let Some(r) = c.reminder.clone().filter(|r| r.remind_at_ms <= now && r.user_reminded_at.is_none()) else {
                continue;
            };
            let flags = [ChatFlag::LowPriority, ChatFlag::Archived].map(|f| (f, c.flag(f)));
            let ts = rfc3339_from_millis(r.remind_at_ms);
            let reminded = ChatReminder { user_reminded_at: Some(now), ..r };
            let c = st.update_chat(i, |c| {
                c.last_activity = c.last_activity.take().max(Some(ts));
                c.reminder = Some(reminded);
            });
            for (flag, _) in flags.into_iter().filter(|f| f.1) {
                st.set_chat_flag(i, flag, Some(false));
            }
            due.push(c);
        }
        due
    }

    /// Desktop's `onMessageReminderDue`: each message reminder that comes due unarchives its chat and notifies once.
    fn message_reminders_due(&mut self, now: i64) -> Vec<Task<UiMessage>> {
        let mut st = self.store.write();
        if !message_reminders_on(&st) {
            return Vec::new();
        }
        let due: Vec<(String, String, i64)> = (st.message_reminders.iter())
            .filter(|(id, r)| r.remind_at_ms <= now && self.reminded.get(*id) != Some(&r.remind_at_ms))
            .map(|(id, r)| (id.clone(), r.room_id.clone(), r.remind_at_ms))
            .collect();
        let mut notes = Vec::new();
        for (id, room, at) in due {
            self.reminded.insert(id.clone(), at);
            let Some(i) = st.resolve_chat_idx(&room) else { continue };
            let i = st.chats[i].merged_into_chat_id.clone().and_then(|p| st.resolve_chat_idx(&p)).unwrap_or(i);
            if st.chats[i].flag(ChatFlag::Archived) {
                st.set_chat_flag(i, ChatFlag::Archived, Some(false));
            }
            let c = st.chats[i].clone();
            let previews = st.settings.account(&c.account_id).previews;
            let body = notify::reminder_body(previews, &c, st.stored_message(&id).as_ref());
            notes.push((c, id, body));
        }
        drop(st);
        notes.into_iter().map(|(c, id, body)| self.notify_thread(&c, &id, body)).collect()
    }

    /// Desktop's `notifyEndedSnoozes`: a snooze that ended since the last check notifies when its chat has unread
    /// activity that is not muted.
    fn snoozes_ended(&mut self, now: i64) -> Vec<Task<UiMessage>> {
        let since = std::mem::replace(&mut self.snooze_clock, now);
        let st = self.store.read();
        if !st.feature("snooze").on {
            return Vec::new();
        }
        let ended = |c: &&Chat| {
            c.snoozed_until_ms.is_some_and(|at| since < at && at <= now)
                && c.listed()
                && (c.is_unread() || c.unread_mentions > 0)
                && (!st.notifications_muted(c) || c.unread_mentions > 0)
        };
        let chats: Vec<Chat> = st.chats.iter().filter(ended).cloned().collect();
        drop(st);
        let body = || trx("Had activity while snoozed", "notification body when a snoozed chat returns").to_string();
        chats.iter().map(|c| self.notify_thread(c, "", body())).collect()
    }
}

/// Desktop's `usesMessageReminders`: message reminders are for the internal channel (`messageRemindersEnabled`), once
/// the reminders room is read.
pub(crate) fn message_reminders_on(st: &BeeperStore) -> bool {
    st.whoami.internal && st.feature("remind-later").show && st.reminders_room.is_some()
}

/// Desktop's `canSetMessageReminder`: a sent message that is not an action and not deleted.
pub(crate) fn can_remind(st: &BeeperStore, m: &Message) -> bool {
    message_reminders_on(st) && m.id.starts_with('$') && !m.silent() && m.is_deleted != Some(true)
}

/// Desktop's `directSnoozedUntilMs`: the end of the chat's own snooze, while snoozing is on and it lasts.
pub(crate) fn snoozed_until(st: &BeeperStore, c: &Chat) -> Option<i64> {
    c.snoozed_until_ms.filter(|&at| at > now_unix_millis() && st.feature("snooze").on)
}

/// Desktop's `areSnoozable`: chats of bridged accounts.
fn snoozable(chats: &[&Chat]) -> bool {
    !chats.is_empty() && chats.iter().all(|c| c.account_id != "hungryserv")
}

/// Desktop's `canSnooze`: snoozing is on, and a free snooze is left unless the chats are snoozed already.
pub(crate) fn can_snooze(st: &BeeperStore, chats: &[&Chat]) -> bool {
    let out = st.features.free_uses("snooze").is_some_and(|(used, limit)| used >= limit);
    st.feature("snooze").on && snoozable(chats) && (!out || chats.iter().all(|c| snoozed_until(st, c).is_some()))
}

/// Desktop's `canDismiss`: a chat is snoozed.
pub(crate) fn can_dismiss_snooze(st: &BeeperStore, chats: &[&Chat]) -> bool {
    chats.iter().any(|c| snoozed_until(st, c).is_some() && c.account_id != "hungryserv")
}

/// Desktop's `requiresUpgradeToSnooze`.
fn snooze_upgrade(st: &BeeperStore, chats: &[&Chat]) -> bool {
    let f = st.feature("snooze");
    f.show && f.upgrade && snoozable(chats) && !can_snooze(st, chats)
}

/// Desktop's `showsSnooze`: the Snooze menu shows for the chats.
pub(crate) fn shows_snooze(st: &BeeperStore, chats: &[&Chat]) -> bool {
    can_snooze(st, chats) || can_dismiss_snooze(st, chats) || snooze_upgrade(st, chats)
}

/// Desktop's `jx`: "Snoozed until" the day and time.
pub(crate) fn snoozed_label(at: i64, h24: bool) -> String {
    trf("Snoozed until %1$s %2$s", &[&reminder_day(at), &clock(at, h24)])
}

impl BeeperGuiApp {
    /// Desktop's `soonestMessageReminderToDismiss`: the message of the earliest message reminder of the chat, when it
    /// comes before the chat reminder.
    pub(crate) fn soonest_message_reminder(&self, st: &BeeperStore, c: &Chat) -> Option<String> {
        let (id, r) = st.message_reminders_of(&c.id).into_iter().next().filter(|_| message_reminders_on(st))?;
        c.reminder.as_ref().is_none_or(|own| r.remind_at_ms < own.remind_at_ms).then(|| id.to_string())
    }

    /// Desktop's `zit` submenu of the message menu: Upgrade to Beeper Plus without Beeper Plus or a free use, Mark as
    /// Done for a set reminder, the times (`Hx`) and Custom…. Each row comes with its height.
    pub(crate) fn message_remind_items(&self, st: &BeeperStore, mid: &str) -> Vec<(El<'_>, f32)> {
        let p = &self.pal;
        let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
        let can = st.feature("remind-later").on;
        let close = |msg: UiMessage| UiMessage::Message(message::Msg::Menu(Box::new(msg)));
        let item = |label: String, at: String, msg: Option<UiMessage>| (menu_item(p, None, label, at, msg), menu_pitch());
        let set = |at: i64| can.then(|| close(UiMessage::Reminders(Msg::Message(mid.into(), Some(at)))));
        let upgrade = (!can).then(|| {
            let open = close(UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus)));
            [item(tr("Upgrade to Beeper Plus").into(), String::new(), Some(open)), (menu_rule(p), MENU_RULE)]
        });
        let done = st.message_reminders.contains_key(mid).then(|| {
            let done = close(UiMessage::Reminders(Msg::Message(mid.into(), None)));
            [item(trx("Mark as Done", "message reminder menu action").into(), String::new(), Some(done)), (menu_rule(p), MENU_RULE)]
        });
        let last = last_used(st, now).map(|at| item(tr("Last Used").into(), last_used_label(at, h24), set(at)));
        let presets = remind_presets(&st.settings, now).into_iter().map(|(label, at)| {
            let msg = set(at).map(|set| UiMessage::Commands(vec![UiMessage::CommandBar(commands::Msg::LastTime(label.clone())), set]));
            item(label, remind_label(at, h24), msg)
        });
        let custom = close(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::RemindMessage(mid.into()))));
        let custom = item(tr("Custom…").into(), String::new(), can.then_some(custom));
        (upgrade.into_iter().flatten().chain(done.into_iter().flatten()).chain(last).chain(presets))
            .chain([(menu_rule(p), MENU_RULE), custom])
            .collect()
    }

    /// Desktop's Snooze menu `Jde` of the chats `ids`: Upgrade to Beeper Plus, the end of the snooze of one snoozed chat,
    /// the times (`xle`), Custom… and Dismiss Snooze. Each row comes with its height.
    pub(crate) fn snooze_items(&self, st: &BeeperStore, ids: &[String]) -> Vec<(El<'_>, f32)> {
        let p = &self.pal;
        let chats: Vec<&Chat> = ids.iter().filter_map(|id| st.resolve_chat_idx(id)).map(|i| &st.chats[i]).collect();
        let (now, h24) = (now_unix_millis(), st.settings.pref("timeFormat") == 1);
        let can = can_snooze(st, &chats);
        let item = |label: String, at: String, msg: Option<UiMessage>| (menu_item(p, None, label, at, msg), menu_pitch());
        let snooze = |at: Option<i64>| UiMessage::Reminders(Msg::Snooze(ids.to_vec(), at));
        let upgrade = snooze_upgrade(st, &chats).then(|| {
            let open = UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus));
            [item(tr("Upgrade to Beeper Plus").into(), String::new(), Some(open)), (menu_rule(p), MENU_RULE)]
        });
        let until = chats.first().filter(|_| chats.len() == 1).and_then(|c| snoozed_until(st, c));
        let heading = until.map(|at| (menu_heading(p, snoozed_label(at, h24)), menu_pitch()));
        let last = last_used_of(&st.settings.snooze_last_time, now)
            .map(|at| item(tr("Last Used").into(), last_used_label(at, h24), can.then(|| snooze(Some(at)))));
        let presets = remind_presets(&st.settings, now).into_iter().map(|(label, at)| {
            let msg = UiMessage::Commands(vec![UiMessage::Reminders(Msg::SnoozeTime(label.clone())), snooze(Some(at))]);
            item(label, remind_label(at, h24), can.then_some(msg))
        });
        let custom = UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Snooze(ids.to_vec())));
        let dismiss = can_dismiss_snooze(st, &chats).then(|| snooze(None));
        (upgrade.into_iter().flatten().chain(heading).chain(last).chain(presets))
            .chain([(menu_rule(p), MENU_RULE), item(tr("Custom…").into(), String::new(), can.then_some(custom))])
            .chain([(menu_rule(p), MENU_RULE), item(tr("Dismiss Snooze").into(), String::new(), dismiss)])
            .collect()
    }
}

pub(crate) const REMINDER_ORDERS: [&str; 4] =
    ["Order by Least Time Remaining", "Order by Most Time Remaining", "Order by Oldest Message", "Order by Newest Message"];

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
        let orders = REMINDER_ORDERS
            .iter()
            .zip(0..)
            .map(|(label, i)| menu_item(p, lead(order == i), tr(label).into(), "", Some(UiMessage::Reminders(reminders::Msg::Order(i)))));
        let ask = Some(UiMessage::Ask(Some(ASK_DISMISS_REMINDERS)));
        let col =
            Column::with_children(orders.chain([menu_rule(p), menu_item(p, lead(false), tr("Dismiss All Reminders").into(), "", ask)]));
        crate::menu_card(p, col, 291).into()
    }

    pub(crate) fn set_reminder(&mut self, cid: &str, at: Option<i64>, no_reply: Option<bool>) {
        {
            let mut st = self.store.write();
            let idx = st.resolve_chat_idx(cid).expect("chat exists");
            self.remind(&mut st, idx, at, no_reply);
        }
        self.list.chat_menu = None;
        self.refresh_theme_and_tray();
    }

    /// Desktop's `setThreadReminder`: the reminder ends on a reply unless `no_reply` or else "Default for remind later" is "Always",
    /// and the chat is archived when "Archive after setting reminder on chat" is on.
    pub(crate) fn remind(&self, st: &mut BeeperStore, idx: usize, at: Option<i64>, no_reply: Option<bool>) {
        let dismiss_on_incoming_message = no_reply.unwrap_or(st.settings.pref("remindLaterDefault") == 0);
        st.set_reminder(idx, at.map(|at| ChatReminder::new(at, dismiss_on_incoming_message)));
        let c = &st.chats[idx];
        if at.is_some() && st.settings.on("archiveOnReminder") && c.flags & (CHAT_PINNED | CHAT_REQUEST) == 0 {
            self.set_archived(st, idx, true);
        }
    }
}

/// Desktop's `Cze`: the time suggestions of `timeSuggestionOptions` that fall in the future, with their times.
pub(crate) fn remind_presets(s: &BeeperSettings, now: i64) -> Vec<(String, i64)> {
    let at = |t: &str| parse_when_millis(t, now).filter(|at| *at > now);
    s.time_suggestions().into_iter().filter_map(|t| Some((t.to_string(), at(t)?))).collect()
}

/// Desktop's `dD`: the last time phrase that set a reminder, read again now, when it falls in the future.
pub(crate) fn last_used(st: &BeeperStore, now: i64) -> Option<i64> {
    last_used_of(&st.settings.last_time, now)
}

/// Desktop's `iM`: the time phrase `phrase` read again now, when it falls in the future.
pub(crate) fn last_used_of(phrase: &str, now: i64) -> Option<i64> {
    Some(phrase).filter(|t| !t.is_empty()).and_then(|t| parse_when_millis(t, now)).filter(|at| *at > now)
}

/// Desktop's "Last Used" time: `EEE dd MMM`, with the year when it is not this year, and the time.
pub(crate) fn last_used_label(at: i64, h24: bool) -> String {
    let (y, m, d) = civil_from_days(day_of(at));
    let month = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(' ').nth(m as usize - 1).expect("a month");
    let year = if y == civil_from_days(today()).0 { String::new() } else { format!(" {y}") };
    let wd = WEEKDAYS[(day_of(at) + 4).rem_euclid(7) as usize];
    format!("{} {d:02} {month}{year}, {:0>8}", &wd[..3], clock(at, h24))
}

#[cfg(test)]
mod tests {
    use super::*;
    use beeper_core::models::REMINDERS_ROOM;
    use iced_test::simulator::simulator;

    /// A demo app of the internal channel, where Desktop has message reminders and snoozing, with the reminders room
    /// read and the last message of a bridged chat as a sent event.
    fn internal_app(name: &str) -> (BeeperGuiApp, String, String) {
        let app = crate::fuzz::demo_app(name);
        let chat = {
            let mut st = app.store.write();
            st.whoami.internal = true;
            st.features.internal = true;
            st.features.flags["features"]["*"]["snooze"] = serde_json::json!(true);
            st.apply_message_reminders(REMINDERS_ROOM, Vec::new());
            let chat = st.chats.iter().find(|c| c.account_id != "hungryserv" && !c.flag(ChatFlag::Archived));
            let chat = chat.expect("a bridged chat").id.clone();
            let m = st.messages[*st.timeline_indices(&chat).last().expect("a message")].clone();
            st.upsert_message(Message { id: "$remind".into(), ..m });
            chat
        };
        (app, chat, "$remind".into())
    }

    fn items(rows: Vec<(El<'_>, f32)>) -> El<'_> {
        Column::with_children(rows.into_iter().map(|r| r.0)).into()
    }

    /// Desktop's message reminders (`zit`): Remind Me opens the times and Custom…, a set reminder turns it into Edit
    /// Reminder with Mark as Done, goes to the reminders room, puts the chat in Reminders and is the header pill. Only
    /// the internal channel has them.
    #[test]
    fn message_reminders_like_desktop() {
        let (mut app, chat, mid) = internal_app("message-reminders");
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(mid.clone())));
        assert!(simulator(app.view_message_menu(&mid).0).find("Remind Me").is_ok());
        let _ = app.update(UiMessage::Reminders(Msg::MessageSub));
        assert!(app.pane.remind_sub);
        {
            let st = app.store.read();
            let mut sim = simulator(items(app.message_remind_items(&st, &mid)));
            assert!(sim.find("Custom…").is_ok() && sim.find("Mark as Done").is_err());
            assert!(sim.find("Upgrade to Beeper Plus").is_err(), "free uses are left");
        }
        let at = now_unix_millis() + 3_600_000;
        let _ = app.update(UiMessage::Reminders(Msg::Message(mid.clone(), Some(at))));
        {
            let st = app.store.read();
            let first = st.message_reminders_of(&chat).first().map(|(id, r)| (id.to_string(), r.remind_at_ms));
            assert_eq!(first, Some((mid.clone(), at)));
            let sent = st.outbox.last().map(|o| (o.1.as_str(), &o.2));
            let sends = matches!(sent, Some((REMINDERS_ROOM, beeper_core::matrix::Outgoing::MessageReminder { room, target, at: Some(t) }))
                if *room == chat && *target == mid && *t == at);
            assert!(sends, "the reminder goes to the reminders room");
            assert!(simulator(items(app.message_remind_items(&st, &mid))).find("Mark as Done").is_ok());
        }
        assert!(simulator(app.view_message_menu(&mid).0).find("Edit Reminder").is_ok());
        let pill = format!("{} {}", reminder_day(at), clock(at, false));
        assert!(simulator(app.view_conversation()).find(pill.as_str()).is_ok(), "the header pill shows the reminder");
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Reminders)));
        assert!(app.visible_chats(&app.store.read()).iter().any(|c| c.id == chat), "Reminders lists the chat");
        let st = app.store.read();
        let soonest = app.soonest_message_reminder(&st, st.chat(&chat));
        assert_eq!(soonest, Some(mid.clone()), "the Chat Info toolbar dismisses it");
        drop(st);
        app.store.write().whoami.internal = false;
        assert!(simulator(app.view_message_menu(&mid).0).find("Edit Reminder").is_err(), "only the internal channel");
        app.store.write().whoami.internal = true;
        let _ = app.update(UiMessage::Reminders(Msg::Message(mid.clone(), None)));
        assert!(app.store.read().message_reminders.is_empty(), "Mark as Done dismisses it");
    }

    /// Desktop's `onReminderElapsed`: a chat reminder that comes due notifies and moves its chat back to the inbox
    /// once. Its `userRemindedAt` stays in the stored chat through a sync of the same reminder, so a restart does not
    /// remind again or unarchive the chat again.
    #[test]
    fn a_due_chat_reminder_notifies_once_like_desktop() {
        use beeper_core::models::ChatReminder;
        let mut app = crate::fuzz::demo_app("due-chat-reminder");
        let now = now_unix_millis();
        let at = now - 1_000;
        let chat = {
            let mut st = app.store.write();
            let c = st.chats.iter().find(|c| c.account_id != "hungryserv" && !c.flag(ChatFlag::Pinned));
            let chat = c.expect("a bridged chat").id.clone();
            let i = st.resolve_chat_idx(&chat).expect("the chat");
            st.set_chat_flag(i, ChatFlag::Archived, Some(true));
            st.set_reminder(i, Some(ChatReminder::new(at, true)));
            st.outbox.clear();
            chat
        };
        let due: Vec<String> = app.chat_reminders_due(now).into_iter().map(|c| c.id).collect();
        assert_eq!(due, std::slice::from_ref(&chat), "the chat to notify");
        {
            let st = app.store.read();
            let c = st.chat(&chat);
            assert!(!c.flag(ChatFlag::Archived), "the chat leaves the archive");
            assert_eq!(c.reminder.as_ref().and_then(|r| r.user_reminded_at), Some(now));
            let stored = serde_json::to_value(c).expect("chat JSON");
            assert_eq!(stored["reminder"]["userRemindedAt"], now, "the time is stored with the chat");
            let sent = st.outbox.iter().any(|o| matches!(o.2, beeper_core::matrix::Outgoing::Reminder(_)));
            assert!(!sent, "the time stays on this device");
            assert!(app.expired(c), "leaving the chat dismisses the reminder");
        }
        assert!(app.chat_reminders_due(now + 1).is_empty(), "it reminds once");
        {
            let mut st = app.store.write();
            let mut synced = st.chat(&chat).clone();
            synced.reminder = Some(ChatReminder::new(at, true));
            st.upsert_chat(synced.clone());
            assert_eq!(st.chat(&chat).reminder.as_ref().and_then(|r| r.user_reminded_at), Some(now), "a sync keeps it");
            let i = st.resolve_chat_idx(&chat).expect("the chat");
            st.set_chat_flag(i, ChatFlag::Archived, Some(true));
            synced.reminder = Some(ChatReminder::new(at + 1, true));
            st.upsert_chat(synced);
        }
        let due: Vec<String> = app.chat_reminders_due(now).into_iter().map(|c| c.id).collect();
        assert_eq!(due, [chat], "a new reminder from another device reminds again");
    }

    /// Desktop's `onMessageReminderDue`: a due message reminder unarchives its chat once, and keeps the chat in the
    /// inbox while it is snoozed.
    #[test]
    fn a_due_message_reminder_unarchives_its_chat_like_desktop() {
        let (mut app, chat, mid) = internal_app("due-message-reminder");
        let now = now_unix_millis();
        {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(&chat).expect("the chat");
            st.set_chat_flag(i, ChatFlag::Archived, Some(true));
            st.set_snooze(i, Some(now + 3_600_000));
            let r = beeper_core::models::MessageReminder { room_id: chat.clone(), remind_at_ms: now - 1 };
            st.apply_message_reminders(REMINDERS_ROOM, vec![(mid.clone(), Some(("$r".into(), r)))]);
        }
        let _ = app.update(UiMessage::Reminders(Msg::Check));
        let st = app.store.read();
        assert!(!st.chat(&chat).flag(ChatFlag::Archived), "the chat leaves the archive");
        assert_eq!(app.reminded.get(&mid), Some(&(now - 1)), "it reminds once");
        assert!(app.visible_chats(&st).iter().any(|c| c.id == chat), "a due reminder shows the snoozed chat");
    }

    /// Desktop's snooze: the chat menu's Snooze and the Chat Info toolbar's Snooze menu (`Jde`) snooze a chat until a
    /// time, which takes it out of the inbox into the Snoozed space with its end as the time, and Dismiss Snooze
    /// brings it back. Only the internal channel snoozes.
    #[test]
    fn snooze_like_desktop() {
        let (mut app, chat, _) = internal_app("snooze");
        assert!(rail::all_spaces(&app.store.read()).contains(&"snoozed".to_string()), "the Snoozed space");
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(chat.clone())));
        assert!(simulator(app.view_main()).find("Snooze").is_ok(), "the chat menu has Snooze");
        let _ = app.update(UiMessage::CommandBar(commands::Msg::ChatSub(Some(3))));
        assert!(simulator(app.view_main()).find("Dismiss Snooze").is_ok());
        let until = now_unix_millis() + 7_200_000;
        let _ = app.update(UiMessage::Reminders(Msg::Snooze(vec![chat.clone()], Some(until))));
        {
            let st = app.store.read();
            assert_eq!(st.chat(&chat).snoozed_until_ms, Some(until));
            assert!(app.visible_chats(&st).iter().all(|c| c.id != chat), "a snoozed chat leaves the inbox");
        }
        let menu = format!("{SNOOZE_MENU}{chat}");
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(menu.clone())));
        {
            let mut sim = simulator(app.view_chat_menu(&menu, false, 600.0));
            assert!(sim.find(snoozed_label(until, false).as_str()).is_ok(), "the end of the snooze heads the menu");
            let _ = sim.click("Dismiss Snooze").expect("Dismiss Snooze");
            let msgs: Vec<UiMessage> = sim.into_messages().collect();
            assert!(matches!(&msgs[..], [UiMessage::Reminders(Msg::Snooze(ids, None))] if *ids == [chat.clone()]));
        }
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Snoozed)));
        assert!(app.visible_chats(&app.store.read()).iter().any(|c| c.id == chat), "the Snoozed space lists it");
        let time = format!("{} {}", reminder_day(until), clock(until, false));
        assert!(simulator(app.view_main()).find(time.as_str()).is_ok(), "with the end of the snooze");
        let _ = app.update(UiMessage::Reminders(Msg::Snooze(vec![chat.clone()], None)));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::AllChats)));
        assert!(app.visible_chats(&app.store.read()).iter().any(|c| c.id == chat), "Dismiss Snooze brings it back");
        app.store.write().features.internal = false;
        assert!(!rail::all_spaces(&app.store.read()).contains(&"snoozed".to_string()));
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(chat.clone())));
        assert!(simulator(app.view_main()).find("Snooze").is_err(), "only the internal channel");
    }

    /// Desktop's `snoozeThread` command bar: the times with their own Last Used phrase, and "dismiss snooze".
    #[test]
    fn snooze_command_bar_like_desktop() {
        let (mut app, chat, _) = internal_app("snooze-commands");
        let _ = app.update(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Snooze(vec![chat.clone()]))));
        let footer = "Snoozed chats leave your inbox and stay quiet until the time you pick";
        assert!(simulator(app.view_main()).find(footer).is_ok());
        let labels = |app: &BeeperGuiApp| -> Vec<String> {
            app.commands(&app.store.read(), "").into_iter().filter_map(|c| c.render.map(|r| r.0)).collect()
        };
        assert!(!labels(&app).contains(&"dismiss snooze".into()), "nothing to dismiss");
        let at = parse_when_millis("tomorrow", now_unix_millis()).expect("tomorrow");
        let _ = app.update(UiMessage::Commands(vec![
            UiMessage::Reminders(Msg::SnoozeTime("tomorrow".into())),
            UiMessage::Reminders(Msg::Snooze(vec![chat.clone()], Some(at))),
        ]));
        let st = app.store.read();
        assert_eq!((st.settings.snooze_last_time.as_str(), st.settings.last_time.as_str()), ("tomorrow", ""));
        drop(st);
        assert_eq!(labels(&app).first().map(String::as_str), Some("last used"));
        assert_eq!(labels(&app).last().map(String::as_str), Some("dismiss snooze"));
    }

    /// Desktop's reminder orders by message take the time of the last message of each chat (`sortTimestamp`), not
    /// of its draft (`activityTimestamp`), oldest or newest first.
    #[test]
    fn reminders_order_by_message_like_desktop() {
        let mut app = crate::fuzz::demo_app("reminder-orders");
        let (old, new) = {
            let st = app.store.read();
            let mut chats: Vec<&Chat> = st.chats.iter().filter(|c| c.last_activity.is_some() && c.listed()).collect();
            chats.sort_by(|a, b| a.last_activity.cmp(&b.last_activity));
            (chats[0].id.clone(), chats[chats.len() - 1].id.clone())
        };
        let at = now_unix_millis() + 3_600_000;
        app.set_reminder(&old, Some(at), None);
        app.set_reminder(&new, Some(at + 60_000), None);
        {
            let mut st = app.store.write();
            let i = st.chat_idx(&old);
            let draft = ChatDraft { text: Some("later".into()), ts: Some(now_unix_millis()), ..Default::default() };
            st.update_chat(i, |c| c.draft = Some(draft));
        }
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Reminders)));
        let mut order = |i: u8| {
            let _ = app.update(UiMessage::Reminders(Msg::Order(i)));
            let st = app.store.read();
            let ids = app.visible_chats(&st).into_iter().map(|c| c.id.clone());
            ids.filter(|id| *id == old || *id == new).collect::<Vec<_>>()
        };
        assert_eq!(order(2), [old.clone(), new.clone()], "the oldest message first, whatever the draft");
        assert_eq!(order(3), [new.clone(), old.clone()], "the newest message first");
        assert_eq!(order(0), [old.clone(), new.clone()], "the least time remaining first");
    }
}

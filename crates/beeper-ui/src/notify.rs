//! Desktop's message notifications (`maybeSendNotification` and `sendNotification`).
use crate::chat_list;
use crate::commands;
use crate::message;
use crate::reminders;
use crate::windows;
use crate::{BeeperGuiApp, UiMessage};
use beeper_core::config::{account_network, millis_from_rfc3339, now_unix_millis};
use beeper_core::models::NetworkKind::*;
use beeper_core::models::{Attachment, Chat, ChatFlag, Message, MSG_MENTION};
use iced::Task;
use std::collections::{HashMap, HashSet};

/// Desktop's attachment text: the verbose form when the message has no text.
pub(crate) fn attachment_text(all: &[Attachment], verbose: bool) -> Option<String> {
    let pick = |long: &str, short: &str| Some(if verbose { long } else { short }.to_string());
    let [a] = all else {
        return (!all.is_empty()).then(|| match verbose {
            true => format!("Sent {} attachments", all.len()),
            false => format!("📄 [{} attachments]", all.len()),
        });
    };
    let name = a.file_name.as_deref().unwrap_or_default();
    let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let contact = (name.ends_with(".vcf") || a.mime_type.as_deref() == Some("text/x-vcard")).then(|| {
        let mut parts = name.splitn(3, '.');
        let stripped = match (parts.next(), parts.next(), parts.next()) {
            (Some(d), Some(w), Some(rest)) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && word(w) => rest,
            _ => name,
        };
        stripped.replacen(".vcf", "", 1)
    });
    match (a.attachment_type.as_str(), contact) {
        _ if a.is_sticker == Some(true) => pick("Sent a sticker", "🖼 [Sticker]"),
        _ if a.is_gif == Some(true) => pick("Sent a GIF", "🎬 [GIF]"),
        ("img", _) => pick("Sent a picture", "🖼 [Picture]"),
        ("video", _) => pick("Sent a video", "🎥 [Video]"),
        ("audio", _) => match &a.transcription {
            Some(t) => Some(format!("“{}”", t.transcription)),
            None if a.is_voice_note == Some(true) => pick("Sent a voice note", "🎤 [Voice Note]"),
            None => pick("Sent an audio", "🎵 [Audio]"),
        },
        (_, Some(c)) => Some(if verbose { format!("Sent a contact: {c}") } else { format!("👤 [Contact: {c}]") }),
        ("unknown", None) => Some(if verbose { format!("Sent a file: {name}") } else { format!("📄 [{name}]") }),
        _ => None,
    }
}

/// Desktop's notification body: the attachment text, the message text, then the link text.
/// Desktop's `messageReminderNotificationBody`: "Reminder: " and the message with its sender in a group, or
/// "Reminder" when the message is not stored or the account hides previews.
pub(crate) fn reminder_body(previews: bool, chat: &Chat, m: Option<&Message>) -> String {
    match m.filter(|_| previews) {
        Some(m) => {
            let sender = m.sender_name.as_deref().filter(|_| chat.chat_type == "group");
            let text = sender.map_or_else(|| body(m), |s| format!("{s}: {}", body(m)));
            crate::i18n::trxf("Reminder: %s", "message reminder notification body", &[&text])
        }
        None => crate::i18n::trx("Reminder", "thread reminder notification body").into(),
    }
}

pub fn body(m: &Message) -> String {
    let text = m.text.as_deref().filter(|t| !t.is_empty());
    let attachment = attachment_text(&m.attachments, text.is_none());
    let link = match m.links.len() {
        _ if attachment.is_some() || text.is_some() => None,
        0 => None,
        1 => Some("Sent a link".to_string()),
        n => Some(format!("Sent {n} links")),
    };
    [attachment, text.map(str::to_string), link].into_iter().flatten().collect::<Vec<_>>().join(" ")
}

/// Desktop's `extractOTP` without its natural language pass that skips money, percentages, and dates:
/// the first `\d{3,4}[-\s]\d{3}` or `\d{4,8}` code that is not part of a word, a number, a file name, or an email.
pub(crate) fn otp_code(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let digits = |i: usize| b[i.min(b.len())..].iter().take_while(|c| c.is_ascii_digit()).count();
    let free = |p: usize| {
        let c = at(p);
        let more = b.get(p + 1..).unwrap_or_default().iter().find(|c| !c.is_ascii_whitespace());
        !(c.is_ascii_alphanumeric()
            || b"-'".contains(&c)
            || b",.;:".contains(&c) && more.is_some_and(u8::is_ascii_digit))
    };
    let ends = |e: usize| (!word(at(e)) && free(e)) || (at(e) == b'.' && free(e + 1));
    let found = (0..b.len()).find_map(|i| {
        let run = digits(i);
        if run < 3 || i > 0 && (b[i - 1].is_ascii_alphanumeric() || b".-".contains(&b[i - 1])) {
            return None;
        }
        let split = run <= 4 && (at(i + run) == b'-' || at(i + run).is_ascii_whitespace()) && digits(i + run + 1) == 3;
        [(split, i + run + 4), ((4..=8).contains(&run), i + run)]
            .into_iter()
            .find(|&(ok, e)| ok && ends(e))
            .map(|(_, e)| &text[i..e])
    })?;
    let lower = text.to_ascii_lowercase();
    let handle = lower.match_indices('@').any(|(p, _)| {
        let name = lower[p + 1..].bytes().take_while(|c| c.is_ascii_lowercase() || *c == b'.').collect::<Vec<_>>();
        let rest = &lower.as_bytes()[p + 1 + name.len()..];
        name.iter().filter(|c| c.is_ascii_lowercase()).count() >= 3
            && rest.first().is_some_and(u8::is_ascii_whitespace)
            && rest.get(1) == Some(&b'#')
            && rest.get(2).is_some_and(u8::is_ascii_digit)
    });
    if text.contains('@') && !handle {
        return None;
    }
    let file = text.match_indices(found).any(|(p, _)| {
        let mut before = b[..p].iter().rev().take_while(|&&c| word(c) || c == b'-');
        let (next, ext) = (at(p + found.len()), at(p + found.len() + 1));
        before.any(u8::is_ascii_alphabetic) && (!word(next) || next == b'.')
            || (p == 0 || !word(b[p - 1])) && next == b'.' && ext.is_ascii_alphanumeric()
    });
    (!file).then_some(found)
}

/// A notification of a chat, for the message with the ID `message`.
#[derive(Clone)]
pub(crate) struct Note {
    pub(crate) chat: String,
    message: String,
    pub(crate) toast: Toast,
}

/// Desktop's notification state of each chat.
#[derive(Default)]
pub(crate) struct Pending {
    timer: u64,
    /// `notificationsQueue`: the timer of the last delayed message, and the delayed notifications.
    queued: HashMap<String, (u64, Vec<Note>)>,
    /// `renotifies`: the timer, and the notification to repeat.
    repeat: HashMap<String, (u64, Note)>,
    /// The users that type, and `lastTypingNotificationParticipantID`.
    typing: HashMap<String, (Vec<String>, Option<String>)>,
    /// The chats of the store at boot, which are not new message requests.
    old_chats: HashSet<String>,
}

impl Pending {
    pub(crate) fn new(chats: &[Chat]) -> Self {
        Self { old_chats: chats.iter().map(|c| c.id.clone()).collect(), ..Self::default() }
    }
}

fn wait(secs: u64, then: UiMessage) -> Task<UiMessage> {
    Task::perform(tokio::time::sleep(std::time::Duration::from_secs(secs)), move |()| then)
}

impl BeeperGuiApp {
    /// Desktop's `maybeSendNotification`: show the notification now, or after "Delay notifications for".
    /// Desktop's `playMessageSendSound` for your local echo, and its receive sound for a new message from the last hour,
    /// both only in the selected chat of a focused window and when the chat is not muted.
    pub(crate) fn message_sound(&self, m: &Message) {
        use crate::sound::{play, Sound};
        let st = self.store.read();
        let Some(i) = st.resolve_chat_idx(&m.chat_id) else { return };
        let mine = m.is_sender == Some(true);
        let hour_old = now_unix_millis() - millis_from_rfc3339(&m.timestamp).expect("RFC 3339 timestamp") > 3_600_000;
        let sound = match (mine, st.settings.pref("soundEffects")) {
            (true, 1 | 3) if m.id.starts_with("msg_") => Sound::Send,
            (false, 2 | 3)
                if !hour_old
                    && m.timestamp > self.booted
                    && !m.silent()
                    && m.message_type.as_deref() != Some("REACTION") =>
            {
                Sound::Receive
            }
            _ => return,
        };
        if self.focused
            && self.pane().selected_chat_id.as_ref() == Some(&m.chat_id)
            && !st.chats[i].flag(ChatFlag::Muted)
        {
            play(sound, st.settings.pref("soundVolume") as f32 / 100.0);
        }
    }

    /// Desktop's notifications of an own message that failed: "Message failed to send" for a send of this device
    /// that waits for Retry (`onNotSent`), else "Message may have failed to send" (`notifyErroredMessage`). Once a
    /// message, and not for messages from before the start.
    pub(crate) fn notify_failed(&mut self, m: &Message) -> Task<UiMessage> {
        let failed = m.is_sender == Some(true) && m.send_error.as_ref().is_some_and(|e| !e.retrying);
        if !failed {
            return Task::none();
        }
        // Desktop notifies at each failed attempt: the first one of a new send, and each one after a Retry.
        let retried = self.retried.remove(&m.id);
        if !retried && (m.timestamp <= self.booted || !self.notified.insert(format!("failed:{}", m.id))) {
            return Task::none();
        }
        let st = self.store.read();
        let Some(chat) = st.find_chat(&m.chat_id) else { return Task::none() };
        let here = st.outbox.iter().any(|o| o.2.message() == Some(m.id.as_str()));
        let body = if here { "Message failed to send" } else { "Message may have failed to send" };
        let toast = Toast { title: chat.title.clone(), body: body.into(), ..Toast::default() };
        let note = Note { chat: chat.id.clone(), message: m.id.clone(), toast };
        // Desktop's `sendQueuedMessages` also shows the error of a send of this device in a toast that stays.
        let error = here.then(|| {
            let e = m.send_error.as_ref().and_then(|e| e.message.clone()).unwrap_or_default();
            let at: [&dyn std::fmt::Display; 3] = [&chat.title, &crate::network_name(&chat.account_id), &e];
            (crate::trf("Something went wrong sending message to %1$s (%2$s)\n%3$s", &at), e)
        });
        drop(st);
        if let Some((text, e)) = error {
            let buttons = vec![
                (crate::tr("Copy Error"), UiMessage::Message(message::Msg::CopyText(e))),
                (crate::tr("Show"), UiMessage::SelectChat(note.chat.clone())),
                (crate::tr("Retry"), UiMessage::Message(message::Msg::RetrySend(m.id.clone()))),
            ];
            self.show(crate::toasts::Toast { buttons, ..crate::toasts::Toast::new(text.clone()) });
            self.sticky_banner = Some(text.clone());
            self.send_toast = Some((m.id.clone(), text));
        }
        self.show_note(note, false)
    }

    pub(crate) fn notify_message(&mut self, m: &Message) -> Task<UiMessage> {
        if let Some(t) = self.pending.typing.get_mut(&m.chat_id) {
            t.1 = None;
        }
        let Some((note, otp)) = self.notification(m) else { return Task::none() };
        if let Some(code) = otp {
            return Task::batch([self.show_note(note, false), iced::clipboard::write(code)]);
        }
        let delay = self.store.read().settings.secs("notificationDelay");
        if delay == 0 {
            return self.show_note(note, true);
        }
        self.pending.timer += 1;
        let chat = note.chat.clone();
        let queued = self.pending.queued.entry(chat.clone()).or_default();
        queued.0 = self.pending.timer;
        queued.1.push(note);
        wait(delay, UiMessage::Notify(Msg::Timer(chat, self.pending.timer)))
    }

    /// Show a notification, and with `repeat` show it again after "Repeat notification if still unread after".
    fn show_note(&mut self, note: Note, repeat: bool) -> Task<UiMessage> {
        let secs = self.store.read().settings.secs("repeatNotification");
        let (chat, message) = (note.chat.clone(), note.message.clone());
        let again = (repeat && secs > 0).then(|| {
            self.pending.timer += 1;
            self.pending.repeat.insert(chat.clone(), (self.pending.timer, note.clone()));
            wait(secs, UiMessage::Notify(Msg::Timer(chat.clone(), self.pending.timer)))
        });
        let show = Task::perform(show(note.toast), move |r| {
            UiMessage::Notify(Msg::Notified(r.map(|id| (id, (chat, message)))))
        });
        Task::batch([show].into_iter().chain(again))
    }

    /// Desktop's `showThreadNotification` of a due reminder or an ended snooze: `body` under the chat title.
    pub(crate) fn notify_thread(&mut self, chat: &Chat, message: &str, body: String) -> Task<UiMessage> {
        let toast = Toast { title: chat.title.clone(), body, ..Toast::default() };
        self.show_note(Note { chat: chat.id.clone(), message: message.into(), toast }, false)
    }

    /// Desktop's `sendQueuedNotifications` and `renotifyIfUnread`.
    pub(crate) fn notify_timer(&mut self, chat: String, timer: u64) -> Task<UiMessage> {
        let st = self.store.read();
        let unread = st.find_chat(&chat).is_some_and(|c| {
            c.unread_count > 0
                && !st.notifications_muted(c)
                && c.is_hidden_from_inbox != Some(true)
                && c.is_request != Some(true)
        });
        let (notes, repeat) = match (self.pending.queued.get(&chat), self.pending.repeat.get(&chat)) {
            (Some(q), _) if q.0 == timer => (self.pending.queued.remove(&chat).expect("queued").1, true),
            (_, Some(r)) if r.0 == timer => {
                let note = self.pending.repeat.remove(&chat).expect("repeated").1;
                let kept = st.stored_message(&note.message).is_some_and(|m| m.is_deleted != Some(true));
                let seen = self.focused && self.pane.selected_chat_id.as_ref() == Some(&chat);
                let on = kept && !seen && st.settings.on("messageNotifications");
                (on.then_some(note).into_iter().collect(), false)
            }
            _ => (Vec::new(), false),
        };
        drop(st);
        Task::batch(notes.into_iter().filter(|_| unread).map(|n| self.show_note(n, repeat)).collect::<Vec<_>>())
    }

    /// Desktop's `showTypingNotification` for each user that starts to type.
    pub(crate) fn notify_typing(&mut self, chat: String) -> Task<UiMessage> {
        let st = self.store.read();
        let now = st.typing.get(&chat).cloned().unwrap_or_default();
        let (before, last) = self.pending.typing.entry(chat.clone()).or_default();
        let started: Vec<String> = now.iter().filter(|u| !before.contains(u)).cloned().collect();
        *before = now;
        let Some(c) = st.find_chat(&chat) else { return Task::none() };
        let shown = self.pane.selected_chat_id.as_ref() != Some(&chat)
            && !st.notifications_muted(c)
            && c.is_hidden_from_inbox != Some(true)
            && c.is_request != Some(true)
            && st.settings.on("notifyTyping");
        let mut notes = Vec::new();
        for user in started.into_iter().filter(|_| shown) {
            if last.as_ref() == Some(&user) {
                continue;
            }
            *last = Some(user.clone());
            let Some(p) = c.participants.items.iter().find(|p| p.id == user) else { continue };
            let name = p.full_name.as_deref().expect("members have a name");
            let body = if c.chat_type == "group" { format!("{name} is typing...") } else { "typing...".into() };
            let toast = Toast { title: c.title.clone(), body, ..Toast::default() };
            notes.push(Note { chat: chat.clone(), message: String::new(), toast });
        }
        drop(st);
        Task::batch(notes.into_iter().map(|n| self.show_note(n, false)).collect::<Vec<_>>())
    }

    /// The notifications of a changed chat: Desktop's reaction notification, and its message request notification
    /// in `pushThread`.
    pub(crate) fn notify_chat(&mut self, c: &Chat) -> Task<UiMessage> {
        let reaction = c.preview.as_ref().filter(|p| p.message_type.as_deref() == Some("REACTION"));
        let reaction = reaction.map(|p| self.notify_message(p));
        let st = self.store.read();
        let request = c.is_request == Some(true)
            && !self.pending.old_chats.contains(&c.id)
            && c.last_activity.as_ref().is_some_and(|t| *t > self.booted)
            && st.settings.on("requestNotifications")
            && st.settings.account(&c.account_id).notifications
            && self.notified.insert(c.id.clone());
        drop(st);
        let request = request.then(|| {
            let body = crate::i18n::tr(if c.chat_type == "group" { "Group Invitation" } else { "Chat Invitation" });
            let toast = Toast { title: c.title.clone(), body: body.into(), ..Toast::default() };
            let note = Note { chat: c.id.clone(), message: String::new(), toast };
            self.show_note(note, false)
        });
        Task::batch(reaction.into_iter().chain(request))
    }

    /// The notification of a new message, and the code to copy, if Desktop shows one. A message with a code always
    /// shows one when "Auto copy OTP/2FA codes" is on.
    pub(crate) fn notification(&mut self, m: &Message) -> Option<(Note, Option<String>)> {
        let st = self.store.read();
        let chat = st.find_chat(&m.chat_id)?;
        // Desktop's `notificationSender` of a merged chat: a member chat notifies as its merged chat.
        let shown_as = chat.merged_into_chat_id.as_deref().and_then(|p| st.find_chat(p)).unwrap_or(chat);
        let group = chat.chat_type == "group";
        let prefs = &st.settings;
        let otp_network = matches!(account_network(&chat.account_id).kind, IMessage | GoogleMessages | GoogleVoice);
        let otp = (m.text.as_deref())
            .filter(|_| otp_network && prefs.on("autoCopyOtp"))
            .and_then(otp_code)
            .map(str::to_string);
        let shown = m.is_sender != Some(true)
            && !m.silent()
            && m.timestamp > self.booted
            && m.text.as_deref() != Some(beeper_core::matrix::UTD_TEXT);
        // Desktop's `shouldSuppressNotification`: a snoozed chat stays quiet, also for mentions.
        let shown = shown && otp.is_some()
            || shown
                && (!st.notifications_muted(chat) || m.flags & MSG_MENTION != 0)
                && reminders::snoozed_until(&st, chat).is_none()
                && chat.is_hidden_from_inbox != Some(true)
                && chat.is_request != Some(true)
                && prefs.on("messageNotifications")
                && (prefs.on("groupNotifications") || !group)
                && (m.message_type.as_deref() != Some("REACTION") || prefs.on("notifyReactions"))
                // Desktop notifies a reply only in a thread when you follow the thread or it mentions you.
                && m.thread_root.as_deref().is_none_or(|r| {
                    !m.thread_only(st.threaded(chat)) || m.flags & MSG_MENTION != 0 || st.follows_thread(chat, r)
                })
                && !(self.focused
                    && (self.pane.selected_chat_id.as_ref() == Some(&shown_as.id) || !prefs.on("notifyInFocus")));
        if !shown || !self.notified.insert(m.id.clone()) {
            return None;
        }
        let account = prefs.account(&chat.account_id);
        let text = body(m) + if otp.is_some() { " (OTP auto copied to clipboard)" } else { "" };
        let sender = m.sender_name.as_deref().filter(|s| group && !text.starts_with(&format!("{s} ")));
        // Desktop's `notificationSound`: the sound of the chat, else of the account, else of the preference.
        let silent = match chat.notification_sound.as_deref() {
            Some(sound) => sound == "silent",
            None => account.silent || prefs.pref(if group { "groupSound" } else { "messageSound" }) == 1,
        };
        let body = match account.previews {
            true => sender.map_or(text.clone(), |s| format!("{s}: {text}")),
            false => "You have a new message".into(),
        };
        // Desktop's actions of a message notification, which only its macOS notifications show: Mark as Read and the
        // reminders, or none with a code, and a reply field where the chat takes replies and previews show.
        let (mut buttons, mut reply) = (Vec::new(), None);
        if cfg!(target_os = "macos") && otp.is_none() {
            let action = |key: &str, label: &'static str| {
                (key.to_string(), crate::i18n::trx(label, "notification action button").to_string())
            };
            buttons.push(action("markRead", "Mark as Read"));
            if st.feature("remind-later").on {
                buttons.extend([action("remind1h", "Remind in 1 Hour"), action("remind8h", "Remind in 8 Hours")]);
            }
            let replies = chat.messages.reply || st.threaded(chat);
            reply = (account.previews && !chat.is_read_only && replies).then(|| crate::i18n::tr("Reply").to_string());
        }
        let toast = Toast { title: shown_as.title.clone(), body, silent, buttons, reply };
        Some((Note { chat: shown_as.id.clone(), message: m.id.clone(), toast }, otp))
    }

    /// Desktop's notification click: jumps to the message, then shows the window.
    pub(crate) fn notification_event(&mut self, id: u32, action: Option<String>) -> Task<UiMessage> {
        if self.verify.note.as_ref().is_some_and(|n| n.0 == id) {
            return self.verification_note_event(action.as_deref());
        }
        let Some((chat, msg)) = self.notifications.remove(&id) else { return Task::none() };
        // Desktop's `markRead`, `remind1h` and `remind8h` actions, and its reply from the notification.
        let hours =
            |h: i64| UiMessage::Reminders(reminders::Msg::Set(chat.clone(), Some(now_unix_millis() + h * 3_600_000)));
        match action.as_deref() {
            Some("markRead") if msg.is_empty() => self.update(UiMessage::ChatList(chat_list::Msg::MarkRead(chat))),
            Some("markRead") => self.update(UiMessage::Message(message::Msg::MarkReadThrough(chat, msg))),
            Some("remind1h") => self.update(hours(1)),
            Some("remind8h") => self.update(hours(8)),
            Some(a) if a.starts_with(REPLY) => {
                let text = a[REPLY.len()..].to_string();
                let draft = beeper_core::store::Draft { text: Some(text), ..Default::default() };
                let mut st = self.store.write();
                // Desktop's `threadForNotificationReply`: a merged chat answers in the chat of the message.
                let to = st.find_message(&msg).map_or(chat, |i| st.messages[i].chat_id.clone());
                let sent = st.send(&to, draft);
                drop(st);
                if let Err(e) = sent {
                    self.status_banner = e.to_string();
                }
                self.refresh_theme_and_tray();
                Task::none()
            }
            Some("default") => {
                let focus = self.update(UiMessage::Window(windows::Msg::FocusRequested(Some(chat.clone()))));
                let jump =
                    (!msg.is_empty()).then(|| self.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat, msg))));
                Task::batch([focus, self.set_hidden(false)].into_iter().chain(jump))
            }
            _ => Task::none(),
        }
    }

    /// Desktop's `clearShownNotifications`: close the notifications of read chats without a reminder, and of the
    /// selected chat in a focused window.
    pub(crate) fn close_notifications(&mut self) -> iced::Task<crate::UiMessage> {
        let st = self.store.read();
        let stale: Vec<u32> = (self.notifications.iter())
            .filter(|(_, (c, _))| {
                let read = |i: usize| {
                    let c = &st.chats[i];
                    !c.is_unread() && c.reminder.is_none()
                };
                (self.focused && self.pane.selected_chat_id.as_ref() == Some(c))
                    || st.resolve_chat_idx(c).is_none_or(read)
            })
            .map(|(&id, _)| id)
            .collect();
        drop(st);
        if stale.is_empty() {
            return iced::Task::none();
        }
        stale.iter().for_each(|id| _ = self.notifications.remove(id));
        iced::Task::future(close(stale))
            .map(Result::err)
            .and_then(|e| iced::Task::done(crate::UiMessage::Notify(Msg::Notified(Err(e)))))
    }
}

/// A notification to show: its title and text, without its sound when `silent`, with `buttons` of an action key and
/// a label, and with a reply field of a placeholder where the platform has one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Toast {
    pub title: String,
    pub body: String,
    pub silent: bool,
    pub buttons: Vec<(String, String)>,
    /// Only macOS notifications have a reply field, like Desktop's (Electron's `hasReply`).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub reply: Option<String>,
}

/// The action of a reply to a notification, before the text of the reply.
pub(crate) const REPLY: &str = "reply:";

#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
mod mac;

/// A listener of notification events: the notification ID and the action.
#[cfg(not(target_os = "linux"))]
type Listener = iced::futures::channel::mpsc::UnboundedSender<(u32, Option<String>)>;

/// The listeners of the notification events of the platforms that call back on threads of their own.
#[cfg(not(target_os = "linux"))]
static LISTENERS: std::sync::Mutex<Vec<Listener>> = std::sync::Mutex::new(Vec::new());

#[cfg(not(target_os = "linux"))]
static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[cfg(not(target_os = "linux"))]
fn emit(id: u32, action: Option<String>) {
    let mut listeners = LISTENERS.lock().expect("the listener lock is not poisoned");
    listeners.retain(|tx| tx.unbounded_send((id, action.clone())).is_ok());
}

#[cfg(target_os = "linux")]
static BUS: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();

/// Notification servers send the click signals to the connection that showed the notification.
#[cfg(target_os = "linux")]
pub(crate) async fn bus() -> Result<&'static zbus::Connection, String> {
    BUS.get_or_try_init(zbus::Connection::session).await.map_err(|e| e.to_string())
}

/// Shows a notification that a click opens, and returns its ID.
#[cfg(target_os = "linux")]
pub(crate) async fn show(t: Toast) -> Result<u32, String> {
    use beeper_core::config::{INSTALL_ID, INSTALL_NAME};
    // The `desktop-entry` hint, which Electron's notifications send too, names the entry whose icon the server shows.
    let hints = std::collections::HashMap::from([
        ("suppress-sound", zbus::zvariant::Value::from(t.silent)),
        ("desktop-entry", zbus::zvariant::Value::from(INSTALL_ID)),
    ]);
    let buttons = t.buttons.iter().map(|(key, label)| (key.as_str(), label.as_str()));
    let actions: Vec<&str> =
        [("default", "")].into_iter().chain(buttons).flat_map(|(key, label)| [key, label]).collect();
    let args = (INSTALL_NAME, 0u32, "", &t.title, &t.body, actions, hints, -1i32);
    let path = "/org/freedesktop/Notifications";
    let iface = Some("org.freedesktop.Notifications");
    let reply = bus().await?.call_method(iface, path, iface, "Notify", &args).await.map_err(|e| e.to_string())?;
    Ok(reply.body().deserialize().expect("Notify returns the notification id"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn show(t: Toast) -> Result<u32, String> {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "windows")]
    win::show(id, &t)?;
    #[cfg(target_os = "macos")]
    mac::show(id, t)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    drop(t);
    Ok(id)
}

#[cfg(target_os = "linux")]
pub(crate) async fn close(ids: Vec<u32>) -> Result<(), String> {
    let (path, iface) = ("/org/freedesktop/Notifications", Some("org.freedesktop.Notifications"));
    for id in ids {
        bus().await?.call_method(iface, path, iface, "CloseNotification", &id).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// macOS keeps a shown notification until the user dismisses it.
#[cfg(not(target_os = "linux"))]
pub(crate) async fn close(ids: Vec<u32>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return win::close(&ids);
    #[cfg(not(target_os = "windows"))]
    {
        drop(ids);
        Ok(())
    }
}

/// A notification id, and the action that the user invoked, or `None` when the server closed it.
#[cfg(target_os = "linux")]
pub fn events() -> impl iced::futures::Stream<Item = (u32, Option<String>)> {
    use iced::futures::StreamExt;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.Notifications")
        .expect("a valid match rule")
        .build();
    let signals = async move {
        let bus = bus().await.expect("a notification was shown, so the session bus works");
        zbus::MessageStream::for_match_rule(rule, bus, None).await.expect("subscribe to notification signals")
    };
    iced::futures::stream::once(signals).flatten().filter_map(|m| async move {
        let m = m.expect("a D-Bus signal");
        let body = m.body();
        match m.header().member().expect("signals have a member").as_str() {
            "ActionInvoked" => {
                let (id, action): (u32, String) =
                    body.deserialize().expect("ActionInvoked carries an id and an action");
                Some((id, Some(action)))
            }
            "NotificationClosed" => {
                Some((body.deserialize::<(u32, u32)>().expect("NotificationClosed carries an id").0, None))
            }
            _ => None,
        }
    })
}

/// A notification id, and the action that the user invoked.
#[cfg(not(target_os = "linux"))]
pub fn events() -> iced::futures::channel::mpsc::UnboundedReceiver<(u32, Option<String>)> {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    LISTENERS.lock().expect("the listener lock is not poisoned").push(tx);
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_settings_change_notifications() {
        use beeper_core::models::AccountProps;
        let mut app = crate::fuzz::demo_app("account-notes");
        let (chat, mut m) = {
            let st = app.store.read();
            let c = st
                .chats
                .iter()
                .find(|c| c.chat_type == "single" && c.account_id != "hungryserv" && c.label_ids.is_empty());
            let c = c.expect("a demo DM without labels").clone();
            let m = st.timeline(&c.id).find(|m| m.is_sender != Some(true));
            (c, m.expect("a received message").clone())
        };
        (m.timestamp, m.text, m.attachments) = ("2099-01-01T00:00:00.000Z".into(), Some("hi".into()), Vec::new());
        let note = |app: &mut BeeperGuiApp, props: AccountProps, m: &mut Message, id: &str| {
            let _ = app.update(UiMessage::Accounts(crate::accounts::Msg::SetProps(chat.account_id.clone(), props)));
            m.id = id.into();
            app.notification(m).map(|(n, _)| (n.toast.body, n.toast.silent))
        };
        let on = AccountProps::default();
        assert_eq!(note(&mut app, on, &mut m, "a"), Some(("hi".into(), false)));
        let hidden = note(&mut app, AccountProps { previews: false, silent: true, ..on }, &mut m, "b");
        assert_eq!(hidden, Some(("You have a new message".into(), true)));
        assert_eq!(note(&mut app, AccountProps { notifications: false, ..on }, &mut m, "c"), None);
        m.flags |= MSG_MENTION;
        let mention = note(&mut app, AccountProps { notifications: false, ..on }, &mut m, "d");
        assert_eq!(mention, Some(("hi".into(), false)), "a mention notifies through the mute");
        // Desktop's `notificationSound`: the sound of the chat comes before the account and the preference.
        let sound = |app: &mut BeeperGuiApp, s: Option<&str>| {
            let _ = app.update(UiMessage::Info(crate::info::Msg::SetSound(chat.id.clone(), s.map(String::from))));
        };
        sound(&mut app, Some("silent"));
        assert_eq!(note(&mut app, on, &mut m, "e").map(|n| n.1), Some(true), "a silent chat has no sound");
        sound(&mut app, Some("double"));
        let loud = note(&mut app, AccountProps { silent: true, ..on }, &mut m, "f");
        assert_eq!(loud.map(|n| n.1), Some(false), "a chat sound plays in a silent account");
        sound(&mut app, None);
        let mid = {
            let st = app.store.read();
            st.messages[st.timeline_indices(&chat.id)[0]].id.clone()
        };
        app.notifications.insert(7, (chat.id.clone(), mid.clone()));
        let _ = app.update(UiMessage::Notify(Msg::Event(7, Some("default".into()))));
        assert_eq!(app.pane.selected_chat_id.as_ref(), Some(&chat.id));
        assert_eq!(app.flash.as_ref().map(|f| &f.0), Some(&mid), "a click jumps to the message");
        // Desktop's actions of its macOS notifications: Mark as Read, a reminder, and a reply.
        let event = |app: &mut BeeperGuiApp, action: String| {
            app.notifications.insert(8, (chat.id.clone(), mid.clone()));
            let _ = app.update(UiMessage::Notify(Msg::Event(8, Some(action))));
        };
        app.store.write().mark_unread_from_message(&chat.id, &mid);
        event(&mut app, "markRead".into());
        assert!(!app.store.read().chat(&chat.id).is_unread(), "Mark as Read reads the chat");
        event(&mut app, "remind1h".into());
        let at = app.store.read().chat(&chat.id).reminder.as_ref().map(|r| r.remind_at_ms);
        assert!(at.is_some_and(|at| (at - now_unix_millis() - 3_600_000).abs() < 60_000), "reminded in an hour");
        event(&mut app, format!("{REPLY}on my way"));
        let sent = app.store.read().timeline(&chat.id).any(|m| m.text.as_deref() == Some("on my way"));
        assert!(sent, "a reply sends to the chat");
    }

    fn attachment(kind: &str, file: Option<&str>) -> Attachment {
        serde_json::from_value(serde_json::json!({ "type": kind, "fileName": file })).expect("attachment JSON")
    }

    #[test]
    fn body_matches_desktop() {
        let mut m: Message = serde_json::from_value(serde_json::json!({
            "id": "m", "accountID": "a", "chatID": "c", "senderID": "s", "sortKey": "1", "timestamp": "t",
        }))
        .expect("message JSON");
        m.attachments = vec![attachment("img", None)];
        assert_eq!(body(&m), "Sent a picture");
        m.text = Some("look".into());
        assert_eq!(body(&m), "🖼 [Picture] look");
        m.attachments = vec![attachment("unknown", Some("1.abc.Jo Doe.vcf"))];
        m.text = None;
        assert_eq!(body(&m), "Sent a contact: Jo Doe");
        m.attachments.push(attachment("video", None));
        assert_eq!(body(&m), "Sent 2 attachments");
    }

    #[test]
    fn otp_code_matches_desktop() {
        assert_eq!(otp_code("Your code is 123456."), Some("123456"));
        assert_eq!(otp_code("Code: 123-456"), Some("123-456"));
        assert_eq!(otp_code("G-482913 is your Google verification code."), None);
        assert_eq!(otp_code("Use 4821 to log in.\n\n@example.com #4821"), Some("4821"));
        assert_eq!(otp_code("Mail me at a@b.com, code 4821"), None);
        assert_eq!(otp_code("See IMG_4821.jpg"), None);
        assert_eq!(otp_code("Open 4821.pdf"), None);
        assert_eq!(otp_code("Total 1,234.56 today"), None);
        assert_eq!(otp_code("Call 555"), None);
        assert_eq!(otp_code("abc123456"), None);
    }
}

#[derive(Clone)]
pub enum Msg {
    Timer(String, u64),
    /// A shown notification, with its chat and message.
    Notified(Result<(u32, (String, String)), String>),
    Event(u32, Option<String>),
}

impl BeeperGuiApp {
    pub(crate) fn notify_event(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Timer(chat, timer) => return self.notify_timer(chat, timer),
            Msg::Notified(Ok((id, note))) => _ = self.notifications.insert(id, note),
            // Like Desktop, a desktop without a notification server shows no notifications and no error.
            Msg::Notified(Err(e)) => eprintln!("notification: {e}"),
            Msg::Event(id, action) => return self.notification_event(id, action),
        }
        self.settle()
    }
}

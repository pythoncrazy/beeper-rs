//! Desktop's message notifications (`maybeSendNotification` and `sendNotification`).
use crate::{BeeperGuiApp, UiMessage};
use beeper_core::config::{account_network, millis_from_rfc3339, now_unix_millis};
use beeper_core::models::NetworkKind::*;
use beeper_core::models::{Attachment, Chat, Message};
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
        ("audio", _) => match &a.transcript {
            Some(t) => Some(format!("“{t}”")),
            None if a.is_voice_note == Some(true) => pick("Sent a voice note", "🎤 [Voice Note]"),
            None => pick("Sent an audio", "🎵 [Audio]"),
        },
        (_, Some(c)) => Some(if verbose { format!("Sent a contact: {c}") } else { format!("👤 [Contact: {c}]") }),
        ("unknown", None) => Some(if verbose { format!("Sent a file: {name}") } else { format!("📄 [{name}]") }),
        _ => None,
    }
}

/// Desktop's notification body: the attachment text, the message text, then the link text.
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
    chat: String,
    message: String,
    title: String,
    body: String,
    silent: bool,
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
        let st = self.store.read().unwrap();
        let Some(&i) = st.chat_idx_by_id.get(&m.chat_id) else { return };
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
        if self.focused && self.selected_chat_id.as_ref() == Some(&m.chat_id) && st.chats[i].is_muted != Some(true) {
            play(sound, st.settings.pref("soundVolume") as f32 / 100.0);
        }
    }

    pub(crate) fn notify_message(&mut self, m: &Message) -> Task<UiMessage> {
        if let Some(t) = self.pending.typing.get_mut(&m.chat_id) {
            t.1 = None;
        }
        let Some((note, otp)) = self.notification(m) else { return Task::none() };
        if let Some(code) = otp {
            return Task::batch([self.show_note(note, false), iced::clipboard::write(code)]);
        }
        let delay = self.store.read().unwrap().settings.secs("notificationDelay");
        if delay == 0 {
            return self.show_note(note, true);
        }
        self.pending.timer += 1;
        let chat = note.chat.clone();
        let queued = self.pending.queued.entry(chat.clone()).or_default();
        queued.0 = self.pending.timer;
        queued.1.push(note);
        wait(delay, UiMessage::NotifyTimer(chat, self.pending.timer))
    }

    /// Show a notification, and with `repeat` show it again after "Repeat notification if still unread after".
    fn show_note(&mut self, note: Note, repeat: bool) -> Task<UiMessage> {
        let secs = self.store.read().unwrap().settings.secs("repeatNotification");
        let chat = note.chat.clone();
        let again = (repeat && secs > 0).then(|| {
            self.pending.timer += 1;
            self.pending.repeat.insert(chat.clone(), (self.pending.timer, note.clone()));
            wait(secs, UiMessage::NotifyTimer(chat.clone(), self.pending.timer))
        });
        let show = Task::perform(show(note.title, note.body, note.silent), move |r| {
            UiMessage::Notified(r.map(|id| (id, chat)))
        });
        Task::batch([show].into_iter().chain(again))
    }

    /// Desktop's `sendQueuedNotifications` and `renotifyIfUnread`.
    pub(crate) fn notify_timer(&mut self, chat: String, timer: u64) -> Task<UiMessage> {
        let st = self.store.read().unwrap();
        let unread = st.chat_idx_by_id.get(&chat).is_some_and(|&i| {
            let c = &st.chats[i];
            c.unread_count > 0
                && c.is_muted != Some(true)
                && c.is_hidden_from_inbox != Some(true)
                && c.is_request != Some(true)
        });
        let (notes, repeat) = match (self.pending.queued.get(&chat), self.pending.repeat.get(&chat)) {
            (Some(q), _) if q.0 == timer => (self.pending.queued.remove(&chat).expect("queued").1, true),
            (_, Some(r)) if r.0 == timer => {
                let note = self.pending.repeat.remove(&chat).expect("repeated").1;
                let kept =
                    st.msg_idx_by_id.get(&note.message).is_some_and(|&i| st.messages[i].is_deleted != Some(true));
                let seen = self.focused && self.selected_chat_id.as_ref() == Some(&chat);
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
        let st = self.store.read().unwrap();
        let now = st.typing.get(&chat).cloned().unwrap_or_default();
        let (before, last) = self.pending.typing.entry(chat.clone()).or_default();
        let started: Vec<String> = now.iter().filter(|u| !before.contains(u)).cloned().collect();
        *before = now;
        let Some(c) = st.chat_idx_by_id.get(&chat).map(|&i| &st.chats[i]) else { return Task::none() };
        let shown = cfg!(target_os = "linux")
            && self.selected_chat_id.as_ref() != Some(&chat)
            && c.is_muted != Some(true)
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
            notes.push(Note {
                chat: chat.clone(),
                message: String::new(),
                title: c.title.clone(),
                body,
                silent: false,
            });
        }
        drop(st);
        Task::batch(notes.into_iter().map(|n| self.show_note(n, false)).collect::<Vec<_>>())
    }

    /// The notifications of a changed chat: Desktop's reaction notification, and its message request notification
    /// in `pushThread`.
    pub(crate) fn notify_chat(&mut self, c: &Chat) -> Task<UiMessage> {
        let reaction = c.preview.as_ref().filter(|p| p.message_type.as_deref() == Some("REACTION"));
        let reaction = reaction.map(|p| self.notify_message(p));
        let request = cfg!(target_os = "linux")
            && c.is_request == Some(true)
            && !self.pending.old_chats.contains(&c.id)
            && c.last_activity.as_ref().is_some_and(|t| *t > self.booted)
            && self.store.read().unwrap().settings.on("requestNotifications")
            && self.notified.insert(c.id.clone());
        let request = request.then(|| {
            let body = if c.chat_type == "group" { "Group Invitation" } else { "Chat Invitation" };
            let note = Note {
                chat: c.id.clone(),
                message: String::new(),
                title: c.title.clone(),
                body: body.into(),
                silent: false,
            };
            self.show_note(note, false)
        });
        Task::batch(reaction.into_iter().chain(request))
    }

    /// The notification of a new message, and the code to copy, if Desktop shows one. A message with a code always
    /// shows one when "Auto copy OTP/2FA codes" is on.
    fn notification(&mut self, m: &Message) -> Option<(Note, Option<String>)> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let st = self.store.read().unwrap();
        let chat = &st.chats[*st.chat_idx_by_id.get(&m.chat_id)?];
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
        let shown = shown && otp.is_some()
            || shown
                && chat.is_muted != Some(true)
                && chat.is_hidden_from_inbox != Some(true)
                && chat.is_request != Some(true)
                && prefs.on("messageNotifications")
                && (prefs.on("groupNotifications") || !group)
                && (m.message_type.as_deref() != Some("REACTION") || prefs.on("notifyReactions"))
                && !(self.focused && (self.selected_chat_id.as_ref() == Some(&chat.id) || !prefs.on("notifyInFocus")));
        if !shown || !self.notified.insert(m.id.clone()) {
            return None;
        }
        let text = body(m) + if otp.is_some() { " (OTP auto copied to clipboard)" } else { "" };
        let sender = m.sender_name.as_deref().filter(|s| group && !text.starts_with(&format!("{s} ")));
        let silent = prefs.pref(if group { "groupSound" } else { "messageSound" }) == 1;
        let body = sender.map_or(text.clone(), |s| format!("{s}: {text}"));
        Some((Note { chat: chat.id.clone(), message: m.id.clone(), title: chat.title.clone(), body, silent }, otp))
    }

    /// Desktop's `clearShownNotifications`: close the notifications of read chats without a reminder, and of the
    /// selected chat in a focused window.
    pub(crate) fn close_notifications(&mut self) -> iced::Task<crate::UiMessage> {
        let st = self.store.read().unwrap();
        let stale: Vec<u32> = (self.notifications.iter())
            .filter(|(_, c)| {
                let read = |i: usize| {
                    let c = &st.chats[i];
                    c.unread_count == 0 && c.is_marked_unread != Some(true) && c.reminder.is_none()
                };
                (self.focused && self.selected_chat_id.as_ref() == Some(c)) || st.resolve_chat_idx(c).is_none_or(read)
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
            .and_then(|e| iced::Task::done(crate::UiMessage::Notified(Err(e))))
    }
}

#[cfg(target_os = "linux")]
static BUS: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();

/// Notification servers send the click signals to the connection that showed the notification.
#[cfg(target_os = "linux")]
pub(crate) async fn bus() -> Result<&'static zbus::Connection, String> {
    BUS.get_or_try_init(zbus::Connection::session).await.map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub async fn show(title: String, body: String, silent: bool) -> Result<u32, String> {
    let hints = std::collections::HashMap::from([("suppress-sound", zbus::zvariant::Value::from(silent))]);
    let args = ("Beeper", 0u32, "", title, body, vec!["default", ""], hints, -1i32);
    let path = "/org/freedesktop/Notifications";
    let iface = Some("org.freedesktop.Notifications");
    let reply = bus().await?.call_method(iface, path, iface, "Notify", &args).await.map_err(|e| e.to_string())?;
    Ok(reply.body().deserialize().expect("Notify returns the notification id"))
}

#[cfg(target_os = "linux")]
async fn close(ids: Vec<u32>) -> Result<(), String> {
    let (path, iface) = ("/org/freedesktop/Notifications", Some("org.freedesktop.Notifications"));
    for id in ids {
        bus().await?.call_method(iface, path, iface, "CloseNotification", &id).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A notification id, and whether the user clicked it or the server closed it.
#[cfg(target_os = "linux")]
pub fn events() -> impl iced::futures::Stream<Item = (u32, bool)> {
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
                (action == "default").then_some((id, true))
            }
            "NotificationClosed" => {
                Some((body.deserialize::<(u32, u32)>().expect("NotificationClosed carries an id").0, false))
            }
            _ => None,
        }
    })
}

#[cfg(not(target_os = "linux"))]
pub async fn show(_title: String, _body: String, _silent: bool) -> Result<u32, String> {
    unreachable!("only the Linux build shows notifications")
}

#[cfg(not(target_os = "linux"))]
async fn close(_ids: Vec<u32>) -> Result<(), String> {
    unreachable!("only the Linux build shows notifications")
}

#[cfg(not(target_os = "linux"))]
pub fn events() -> iced::futures::stream::Empty<(u32, bool)> {
    unreachable!("only the Linux build shows notifications")
}

#[cfg(test)]
mod tests {
    use super::*;

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

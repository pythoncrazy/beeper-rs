//! Desktop's message notifications (`maybeSendNotification` and `sendNotification`).
use crate::BeeperGuiApp;
use beeper_core::models::{Attachment, Message};

/// Desktop's attachment text: the verbose form when the message has no text.
fn attachment_text(all: &[Attachment], verbose: bool) -> Option<String> {
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

impl BeeperGuiApp {
    /// The title and body of the notification for a new message, if Desktop shows one.
    pub(crate) fn notification(&mut self, m: &Message) -> Option<(String, String)> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let st = self.store.read().unwrap();
        let chat = &st.chats[*st.chat_idx_by_id.get(&m.chat_id)?];
        let group = chat.chat_type == "group";
        let prefs = &st.settings;
        let shown = m.is_sender != Some(true)
            && m.timestamp > self.booted
            && m.text.as_deref() != Some(beeper_core::matrix::UTD_TEXT)
            && chat.is_muted != Some(true)
            && chat.is_hidden_from_inbox != Some(true)
            && chat.is_request != Some(true)
            && prefs.on("messageNotifications")
            && (prefs.on("groupNotifications") || !group)
            && !(self.focused && (self.selected_chat_id.as_ref() == Some(&chat.id) || !prefs.on("notifyInFocus")));
        if !shown || !self.notified.insert(m.id.clone()) {
            return None;
        }
        let text = body(m);
        let sender = m.sender_name.as_deref().filter(|s| group && !text.starts_with(&format!("{s} ")));
        Some((chat.title.clone(), sender.map_or(text.clone(), |s| format!("{s}: {text}"))))
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
async fn bus() -> Result<&'static zbus::Connection, String> {
    BUS.get_or_try_init(zbus::Connection::session).await.map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub async fn show(title: String, body: String) -> Result<u32, String> {
    let hints = std::collections::HashMap::<&str, zbus::zvariant::Value>::new();
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
pub async fn show(_title: String, _body: String) -> Result<u32, String> {
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
}

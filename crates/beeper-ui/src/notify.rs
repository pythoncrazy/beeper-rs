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
}

#[cfg(target_os = "linux")]
pub async fn show(title: String, body: String) -> Result<(), String> {
    let bus = zbus::Connection::session().await.map_err(|e| e.to_string())?;
    let hints = std::collections::HashMap::<&str, zbus::zvariant::Value>::new();
    let args = ("Beeper", 0u32, "", title, body, Vec::<&str>::new(), hints, -1i32);
    let path = "/org/freedesktop/Notifications";
    let iface = Some("org.freedesktop.Notifications");
    bus.call_method(iface, path, iface, "Notify", &args).await.map(drop).map_err(|e| e.to_string())
}

/// Only the Linux build shows notifications so far.
#[cfg(not(target_os = "linux"))]
pub async fn show(_title: String, _body: String) -> Result<(), String> {
    Ok(())
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

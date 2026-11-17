//! macOS notifications through `NSUserNotificationCenter`, like Desktop's native notifications on macOS: a click, the
//! action buttons in one menu, or a reply field. Each notification waits for its answer on a thread of its own, and a
//! shown notification cannot be taken back.
use super::Toast;
use mac_notification_sys::{get_bundle_identifier_or_default, set_application, MainButton, Notification};
use mac_notification_sys::{NotificationResponse, Sound};

pub(super) fn show(id: u32, t: Toast) -> Result<(), String> {
    static APP: std::sync::Once = std::sync::Once::new();
    APP.call_once(|| {
        // The notifications of an app bundle named Beeper-RS show as its own, else as the default app's.
        if let Err(e) = set_application(&get_bundle_identifier_or_default(beeper_core::config::INSTALL_NAME)) {
            eprintln!("set the notification app: {e}");
        }
    });
    std::thread::Builder::new()
        .name("notification".into())
        .spawn(move || {
            let labels: Vec<&str> = t.buttons.iter().map(|b| b.1.as_str()).collect();
            let more = crate::i18n::tr("More");
            let mut n = Notification::new();
            n.title(&t.title).message(&t.body).wait_for_click(true);
            if !t.silent {
                n.sound(Sound::Default);
            }
            match (&t.reply, labels.as_slice()) {
                (Some(placeholder), _) => _ = n.main_button(MainButton::Response(placeholder)),
                (None, []) => {}
                (None, [one]) => _ = n.main_button(MainButton::SingleAction(one)),
                (None, many) => _ = n.main_button(MainButton::DropdownActions(more, many)),
            }
            let action = match n.send() {
                Ok(NotificationResponse::Click) => Some("default".to_string()),
                Ok(NotificationResponse::ActionButton(label)) => {
                    t.buttons.iter().find(|b| b.1 == label).map(|b| b.0.clone())
                }
                Ok(NotificationResponse::Reply(text)) => Some(format!("{}{text}", super::REPLY)),
                Ok(_) => None,
                Err(e) => return eprintln!("show a notification: {e}"),
            };
            super::emit(id, action);
        })
        .map(drop)
        .map_err(|e| e.to_string())
}

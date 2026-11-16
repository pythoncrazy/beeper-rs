//! Windows toasts through WinRT, like Electron's `Notification` on Windows: the title, the text, the sound, and a
//! click, with a tag so that a read chat can take its toasts back out of the Action Center.
use super::Toast;
use windows::core::{Interface, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};

/// The toast group of the app, which `close` removes the toasts from.
const GROUP: &str = "beeper-rs";

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Windows shows the toasts of an app without a Start menu shortcut only for an app ID that the registry names, so the
/// app registers its ID as Beeper-RS once, and uses it for the windows of the process too.
fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        use beeper_core::config::{INSTALL_ID, INSTALL_NAME};
        let key = windows_registry::CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{INSTALL_ID}"));
        if let Err(e) = key.and_then(|k| k.set_string("DisplayName", INSTALL_NAME)) {
            eprintln!("register the toast app ID: {e}");
        }
        // SAFETY: the call reads the string for its duration only.
        let set =
            unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(INSTALL_ID)) };
        if let Err(e) = set {
            eprintln!("set the app ID of the process: {e}");
        }
    });
}

/// The toast XML: the title and the text, silent or with the default sound, and the buttons with their action keys.
pub(super) fn xml(t: &Toast) -> String {
    let audio = if t.silent { r#"<audio silent="true"/>"# } else { "" };
    let actions: String = (t.buttons.iter())
        .map(|(key, label)| format!(r#"<action content="{}" arguments="{}"/>"#, escape(label), escape(key)))
        .collect();
    format!(
        r#"<toast launch="default"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>{audio}<actions>{actions}</actions></toast>"#,
        escape(&t.title),
        escape(&t.body)
    )
}

pub(super) fn show(id: u32, t: &Toast) -> Result<(), String> {
    register();
    let e = |e: windows::core::Error| e.to_string();
    let doc = XmlDocument::new().map_err(e)?;
    doc.LoadXml(&HSTRING::from(xml(t))).map_err(e)?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(e)?;
    toast.SetTag(&HSTRING::from(id.to_string())).map_err(e)?;
    toast.SetGroup(&HSTRING::from(GROUP)).map_err(e)?;
    let activated = TypedEventHandler::new(move |_, args: windows::core::Ref<windows::core::IInspectable>| {
        let args = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok());
        let action = args.and_then(|a| a.Arguments().ok()).map(|a| a.to_string()).filter(|a| !a.is_empty());
        super::emit(id, Some(action.unwrap_or_else(|| "default".into())));
        Ok(())
    });
    toast.Activated(&activated).map_err(e)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(beeper_core::config::INSTALL_ID));
    notifier.and_then(|n| n.Show(&toast)).map_err(e)
}

pub(super) fn close(ids: &[u32]) -> Result<(), String> {
    let e = |e: windows::core::Error| e.to_string();
    let history = ToastNotificationManager::History().map_err(e)?;
    let app = HSTRING::from(beeper_core::config::INSTALL_ID);
    for id in ids {
        history.RemoveGroupedTagWithId(&HSTRING::from(id.to_string()), &HSTRING::from(GROUP), &app).map_err(e)?;
    }
    Ok(())
}

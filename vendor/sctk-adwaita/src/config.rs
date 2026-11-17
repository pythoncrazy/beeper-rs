//! System configuration.
use std::process::Command;

/// Whether the desktop prefers dark colors, asked once per process.
pub(crate) fn prefer_dark() -> bool {
    static DARK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DARK.get_or_init(query_prefer_dark)
}

/// Query system to see if dark theming should be preferred.
fn query_prefer_dark() -> bool {
    // outputs something like: `variant       variant          uint32 1`
    let stdout = Command::new("dbus-send")
        .arg("--reply-timeout=100")
        .arg("--print-reply=literal")
        .arg("--dest=org.freedesktop.portal.Desktop")
        .arg("/org/freedesktop/portal/desktop")
        .arg("org.freedesktop.portal.Settings.Read")
        .arg("string:org.freedesktop.appearance")
        .arg("string:color-scheme")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok());

    if matches!(stdout, Some(ref s) if s.is_empty()) {
        log::error!("XDG Settings Portal did not return response in time: timeout: 100ms, key: color-scheme");
    }

    matches!(stdout, Some(s) if s.trim().ends_with("uint32 1"))
}

/// Query system configuration for buttons layout.
/// Should be updated to use standard xdg-desktop-portal specs once available
/// https://github.com/flatpak/xdg-desktop-portal/pull/996
pub(crate) fn get_button_layout_config() -> Option<(String, String)> {
    let config_string = Command::new("dbus-send")
        .arg("--reply-timeout=100")
        .arg("--print-reply=literal")
        .arg("--dest=org.freedesktop.portal.Desktop")
        .arg("/org/freedesktop/portal/desktop")
        .arg("org.freedesktop.portal.Settings.Read")
        .arg("string:org.gnome.desktop.wm.preferences")
        .arg("string:button-layout")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())?;

    let sides_split: Vec<_> = config_string
        // Taking last word
        .rsplit(' ')
        .next()?
        // Split by left/right side
        .split(':')
        // Only two sides
        .take(2)
        .collect();

    match sides_split.as_slice() {
        [left, right] => Some((left.to_string(), right.to_string())),
        _ => None,
    }
}

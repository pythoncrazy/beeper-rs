//! Desktop's global hotkeys (`isGlobal`): "Focus App" and "Focus App and Command Bar", which Electron's
//! `globalShortcut` takes from the whole desktop. On X11 the app grabs their keys on the root window, as Chromium's
//! `GlobalShortcutListenerX11` does. A Wayland session has no such grab without the GlobalShortcuts portal, so there
//! the hotkeys work only inside the app.
use crate::UiMessage;
use iced::Subscription;

/// The names of Desktop's global hotkeys.
pub(crate) const GLOBAL: [&str; 2] = ["Focus App", "Focus App and Command Bar"];

/// The X11 keysym of a key of Desktop's hotkey strings, which the key recorder writes in lowercase.
fn keysym(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    Some(match (chars.next()?, chars.next()) {
        (c, None) if c.is_ascii_graphic() => u32::from(c.to_ascii_lowercase()),
        _ => match key {
            "space" => 0x20,
            "enter" => 0xff0d,
            "tab" => 0xff09,
            "escape" => 0xff1b,
            "backspace" => 0xff08,
            "delete" => 0xffff,
            "insert" => 0xff63,
            "home" => 0xff50,
            "end" => 0xff57,
            "pageup" => 0xff55,
            "pagedown" => 0xff56,
            "left" => 0xff51,
            "up" => 0xff52,
            "right" => 0xff53,
            "down" => 0xff54,
            f => match f.strip_prefix('f')?.parse::<u32>().ok()? {
                n @ 1..=24 => 0xffbe + n - 1,
                _ => return None,
            },
        },
    })
}

/// A combination of a hotkey string ("ctrl+shift+b") as X11 modifiers and a keysym.
pub(crate) fn combination(keys: &str) -> Option<(u16, u32)> {
    use x11rb::protocol::xproto::ModMask;
    let (mut mods, mut sym) = (0u16, None);
    for part in keys.split('+') {
        match part {
            "ctrl" => mods |= u16::from(ModMask::CONTROL),
            "shift" => mods |= u16::from(ModMask::SHIFT),
            "alt" => mods |= u16::from(ModMask::M1),
            "command" | "meta" | "super" => mods |= u16::from(ModMask::M4),
            key => sym = Some(keysym(key)?),
        }
    }
    Some((mods, sym?))
}

/// The global hotkeys of `bindings`, each a name and its keys, while the app runs on X11. Each press sends
/// [`UiMessage::RunHotkey`] of the hotkey.
pub(crate) fn subscription(bindings: Vec<(&'static str, String)>) -> Subscription<UiMessage> {
    let x11 = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    let keys: Vec<(&'static str, u16, u32)> =
        bindings.iter().flat_map(|(name, keys)| keys.split(", ").filter_map(combination).map(move |(m, s)| (*name, m, s))).collect();
    if !x11 || keys.is_empty() {
        return Subscription::none();
    }
    Subscription::run_with(keys, |keys| {
        let keys = keys.clone();
        iced::stream::channel(8, move |output| async move {
            let _ = tokio::task::spawn_blocking(move || grab(&keys, output)).await;
        })
    })
}

/// Grabs the keys on the root window of a connection of their own, and sends their presses until the subscription
/// ends. Closing the connection ends the grabs.
fn grab(keys: &[(&'static str, u16, u32)], mut output: iced::futures::channel::mpsc::Sender<UiMessage>) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    use x11rb::protocol::Event;
    let Ok((conn, screen)) = x11rb::connect(None) else { return };
    let root = conn.setup().roots[screen].root;
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let Some(map) = conn.get_keyboard_mapping(min, max - min + 1).ok().and_then(|c| c.reply().ok()) else { return };
    let per = usize::from(map.keysyms_per_keycode).max(1);
    let code = |sym: u32| map.keysyms.chunks(per).position(|c| c.contains(&sym)).map(|i| min + i as u8);
    // The grab of each key also takes it with Caps Lock and Num Lock on.
    let locks = [0, u16::from(ModMask::LOCK), u16::from(ModMask::M2), u16::from(ModMask::LOCK | ModMask::M2)];
    let mut grabbed = Vec::new();
    for &(name, mods, sym) in keys {
        let Some(code) = code(sym) else { continue };
        for lock in locks {
            let grab = conn.grab_key(true, root, ModMask::from(mods | lock), code, GrabMode::ASYNC, GrabMode::ASYNC);
            if grab.map_or(true, |c| c.check().is_err()) {
                eprintln!("global hotkey {name}: the key is not free");
            }
        }
        grabbed.push((name, mods, code));
    }
    if conn.flush().is_err() {
        return;
    }
    let held = u16::from(ModMask::CONTROL | ModMask::SHIFT | ModMask::M1 | ModMask::M4);
    while !output.is_closed() {
        match conn.poll_for_event() {
            Ok(Some(Event::KeyPress(e))) => {
                let hit = grabbed.iter().find(|(_, mods, code)| *code == e.detail && *mods == u16::from(e.state) & held);
                if let Some((name, ..)) = hit {
                    let _ = output.try_send(UiMessage::RunHotkey(name));
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder's key strings as X11 modifiers and keysyms.
    #[test]
    fn hotkeys_become_x11_grabs() {
        assert_eq!(combination("ctrl+shift+b"), Some((4 | 1, u32::from(b'b'))));
        assert_eq!(combination("ctrl+alt+space"), Some((4 | 8, 0x20)));
        assert_eq!(combination("command+f5"), Some((64, 0xffc2)));
        assert_eq!(combination("ctrl+alt"), None, "a modifier alone has no key");
        assert_eq!(combination("ctrl+nosuchkey"), None);
    }
}

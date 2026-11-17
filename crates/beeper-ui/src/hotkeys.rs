//! Settings > Hotkeys: the hotkey switches, the shortcut recorder, and the Keyboard Shortcuts list.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Toggle(&'static str),
    /// Desktop's click on a `ShortcutRecorder`, which starts to record keys for the hotkey.
    Record(&'static str),
    /// The Save (true) or Cancel (false) button of the recorder.
    Recorder(bool),
    /// The Revert button of a hotkey with keys other than the default.
    Revert(&'static str),
    RevertAll,
    Query(String),
    /// Opens Desktop's Keyboard Shortcuts list with the search text, or closes it.
    Modal(Option<String>),
    Scrolled(bool),
}

impl BeeperGuiApp {
    pub(crate) fn hotkeys(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Toggle(name) => {
                self.patch_settings(|s| {
                    match s.hotkey_keys(name) {
                        "disabled" if prefs::hotkey_default(name) == "disabled" => {
                            s.keybindings.insert(name.into(), "".into())
                        }
                        "disabled" => s.keybindings.remove(name),
                        _ => s.keybindings.insert(name.into(), "disabled".into()),
                    };
                });
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            Msg::Record(name) => {
                if self.hotkey_rec.as_ref().is_none_or(|r| r.name != name) {
                    self.hotkey_rec = Some(HotkeyRec { name, mods: [false; 3], key: None, buttons: false });
                }
            }
            Msg::Recorder(false) => self.hotkey_rec = None,
            Msg::Recorder(true) => self.save_hotkey(),
            Msg::Revert(name) => {
                self.patch_settings(|s| drop(s.keybindings.remove(name)));
                (self.hotkey_rec, self.relaunch_toast) = (None, Some(HOTKEYS_RELAUNCH));
            }
            Msg::RevertAll => {
                self.patch_settings(|s| s.keybindings.clear());
                return self.update(UiMessage::Relaunch);
            }
            Msg::Query(q) => self.hotkey_query = q,
            Msg::Modal(q) => {
                self.hotkeys_scrolled &= q.is_some();
                let open = self.hotkeys_modal.is_none() && q.is_some();
                self.hotkeys_modal = q;
                if open {
                    return operation::focus("hotkeys-search");
                }
            }
            Msg::Scrolled(down) => self.hotkeys_scrolled = down,
        }
        self.settle()
    }
}

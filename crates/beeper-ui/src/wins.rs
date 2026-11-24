//! The child windows of the app: each is one of [`Win`] in [`BeeperGuiApp::children`], which gives its title, its view,
//! its keys and what it forgets as it closes. The chat, space and "Media in %s" windows stay apart, because their panes
//! swap with the main window's.

use super::*;
use crate::modals::Modal;

/// A child window and what it shows.
pub(crate) enum Win {
    /// Desktop's Blocked users window.
    Blocked,
    /// Desktop's How It Works window of the Chat Info behavior menu.
    How,
    /// Desktop's `AddAccountsBrowserWindow`.
    Networks,
    /// Desktop's `VERIFICATION` modal window.
    Verify,
    /// Desktop's `ReauthRequiredModal`.
    Reauth,
    /// Desktop's Enter your Recovery Key modal.
    EnterKey,
    /// Desktop's windows that open once each, like its modal windows.
    Modal(Modal),
}

/// The child windows and what they show, in the order that they opened.
#[derive(Default)]
pub(crate) struct Children(Vec<(window::Id, Win)>);

impl Children {
    /// What child window `id` shows.
    pub(crate) fn at(&self, id: window::Id) -> Option<&Win> {
        self.0.iter().find(|w| w.0 == id).map(|w| &w.1)
    }

    /// The first open child window that `is` picks.
    pub(crate) fn find(&self, is: impl Fn(&Win) -> bool) -> Option<window::Id> {
        self.0.iter().find(|w| is(&w.1)).map(|w| w.0)
    }

    pub(crate) fn push(&mut self, id: window::Id, w: Win) {
        self.0.push((id, w));
    }

    /// Forgets the child window that `is` picks before it closes, and gives its ID.
    pub(crate) fn take(&mut self, is: impl Fn(&Win) -> bool) -> Option<window::Id> {
        let i = self.0.iter().position(|w| is(&w.1))?;
        Some(self.0.remove(i).0)
    }

    /// The open child windows.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(window::Id, Win)> {
        self.0.iter()
    }
}

impl BeeperGuiApp {
    /// Raises the child window that `is` picks, else opens a window with `open` that shows `win`.
    pub(crate) fn open_once(
        &mut self, is: impl Fn(&Win) -> bool, open: impl FnOnce(&mut Self) -> (window::Id, Task<UiMessage>),
        win: impl FnOnce(window::Id) -> Win,
    ) -> Task<UiMessage> {
        if let Some(id) = self.children.find(is) {
            return self.raise(id);
        }
        let (id, open) = open(self);
        self.children.push(id, win(id));
        open
    }

    /// The title of child window `w`.
    pub(crate) fn child_title(&self, w: &Win) -> String {
        match w {
            Win::Blocked => tr("Blocked users"),
            // Desktop's `AddAccountsBrowserWindow` title, which it does not translate.
            Win::Networks => "Add an Account",
            Win::How => tr("How It Works"),
            Win::Verify => "Beeper",
            Win::Reauth => tr("Authentication Required"),
            Win::EnterKey => self.enter_key_title(),
            Win::Modal(m) => m.title(),
        }
        .into()
    }

    pub(crate) fn view_child(&self, id: window::Id, w: &Win) -> El<'_> {
        match w {
            Win::Blocked => self.view_blocked_users(id),
            Win::How => self.view_how_it_works(),
            Win::Networks => self.view_add_accounts(),
            Win::Verify => self.view_verify(),
            Win::Reauth => self.view_reauth(),
            Win::EnterKey => self.view_enter_key(),
            Win::Modal(m) => self.view_modal(*m),
        }
    }

    /// Forgets child window `id`, which closed, and what it showed.
    pub(crate) fn child_closed(&mut self, id: window::Id) {
        let Some(i) = self.children.0.iter().position(|w| w.0 == id) else { return };
        match self.children.0.remove(i).1 {
            Win::Networks => self.networks.menu = None,
            // Desktop's `ReauthRequiredModal` closed without a code ends the logout of a device.
            Win::Reauth => self.devices_busy = false,
            Win::Modal(m) => self.modal_closed(m),
            Win::Blocked | Win::How | Win::Verify | Win::EnterKey => {}
        }
    }

    /// The keys of child window `id` that closes on Escape or confirms on Ctrl+Enter, or `None` for the keys of the
    /// app.
    pub(crate) fn child_key(&mut self, id: window::Id, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let escape = *key == Key::Named(Named::Escape);
        let close = |app: &Self, closes: bool| if closes { app.close_window(id) } else { Task::none() };
        Some(match self.children.at(id)? {
            Win::How => close(self, escape || (*key == Key::Named(Named::Enter) && mods.command())),
            Win::Blocked => close(self, escape),
            // Desktop's `onEscapePressed` closes the Add an Account window, after the menu of a network.
            Win::Networks => {
                let closes = escape && self.networks.menu.take().is_none();
                close(self, closes)
            }
            Win::Reauth => self.reauth_key(id, key),
            &Win::Modal(m) => close(self, escape && m.escape_closes()),
            Win::EnterKey => close(self, escape),
            Win::Verify => return None,
        })
    }
}

//! Desktop's windows that open once each from a menu or a prompt: its modal windows (`gm`, the `modal-N` windows of
//! `setVisibleModal`), and the windows of its classes that keep no list, like Change Email.

use super::*;
use crate::windows::MIN_SIZE;

/// The windows of [`BeeperGuiApp::modals`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    /// `SHOW_RECOVERY_KEY`, from the Profile menu and the backup banner.
    RecoveryKey,
    /// `ChangeEmailBrowserWindow` (`TNt`), which the Save of a new email on the Profile page opens.
    ChangeEmail,
}

/// How Desktop opens the window of a [`Modal`]: its size before Electron grows it to its minimum, whether it resizes,
/// and whether it has the title bar of the system.
struct Shape {
    size: Size,
    resizable: bool,
    frame: bool,
}

impl Modal {
    /// `gm` of a modal window, and `windowFeatures` of a class.
    fn shape(self) -> Shape {
        match self {
            // `gm` asks 350 by 330 at least 175 high, but Electron's `minHeight` of 400 (`ji`) wins.
            Modal::RecoveryKey => Shape { size: Size::new(350.0, 330.0), resizable: true, frame: true },
            // `frame: !1`, so it has no title bar.
            Modal::ChangeEmail => Shape { size: Size::new(350.0, 380.0), resizable: false, frame: false },
        }
    }

    /// The window title: `gm`'s `title`, and the class's `title`, which Desktop does not translate.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Modal::RecoveryKey => "Recovery Key",
            Modal::ChangeEmail => "Change Email",
        }
    }
}

/// `gm`'s `maxWidth` of a modal window that resizes, `Math.max(700, width)`. Its height has no limit.
const MAX_SIZE: Size = Size::new(700.0, 100_000.0);

impl BeeperGuiApp {
    /// The open window of `m`.
    pub(crate) fn modal_window(&self, m: Modal) -> Option<window::Id> {
        self.modals.iter().find(|w| w.1 == m).map(|w| w.0)
    }

    /// What window `id` is, when it is one of [`Modal`].
    pub(crate) fn modal_at(&self, id: window::Id) -> Option<Modal> {
        self.modals.iter().find(|w| w.0 == id).map(|w| w.1)
    }

    /// Opens the window of `m`, or raises it when it is open. Desktop's `gm` centers a modal window over the window
    /// whose `setVisibleModal` opened it, `from`; the classes center over the main window.
    pub(crate) fn open_modal(&mut self, m: Modal, from: Option<window::Id>) -> Task<UiMessage> {
        if let Some(id) = self.modal_window(m) {
            return self.raise(id);
        }
        let Shape { size, resizable, frame } = m.shape();
        let settings = window::Settings {
            resizable,
            decorations: frame,
            min_size: resizable.then_some(MIN_SIZE),
            max_size: (resizable && frame).then_some(MAX_SIZE),
            ..Self::window_settings(size)
        };
        let parent = from.and_then(|id| self.screen_size(id).map(|size| (id, size)));
        let (id, open) = self.open_over(settings, parent);
        self.modals.push((id, m));
        open
    }

    /// Closes the window of `m` when it is open.
    pub(crate) fn close_modal(&self, m: Modal) -> Task<UiMessage> {
        self.modal_window(m).map_or_else(Task::none, |id| self.close_window(id))
    }

    /// Forgets window `id` of `m`, which closed, and what it showed.
    pub(crate) fn modal_closed(&mut self, id: window::Id) {
        let Some(m) = self.modal_at(id) else { return };
        self.modals.retain(|w| w.0 != id);
        match m {
            Modal::RecoveryKey => self.profile.recovery_key = None,
            Modal::ChangeEmail => self.profile.change_email = None,
        }
    }

    /// The size in screen pixels of the main window, the Settings window or a chat window `id`.
    fn screen_size(&self, id: window::Id) -> Option<Size> {
        let size = match () {
            () if Some(id) == self.main_window => self.pane.size,
            () if Some(id) == self.settings_window => self.settings_size,
            () => self.chat_windows.iter().find(|w| w.0 == id)?.1.size,
        };
        Some(size * self.zoom)
    }

    pub(crate) fn view_modal(&self, m: Modal) -> El<'_> {
        match m {
            Modal::RecoveryKey => self.view_recovery_key(),
            Modal::ChangeEmail => self.view_change_email(),
        }
    }

    /// A key in the window of `m`: Escape closes it, like `onEscapePressed` of Change Email and Desktop's
    /// `handleEscape`, which hides `SHOW_RECOVERY_KEY`.
    pub(crate) fn modal_key(&mut self, id: window::Id, key: &Key) -> Task<UiMessage> {
        match key {
            Key::Named(keyboard::key::Named::Escape) => self.close_window(id),
            _ => Task::none(),
        }
    }

    /// A press on the background of frameless window `id`, which Desktop's `-webkit-app-region: drag` moves.
    pub(crate) fn drag_area<'a>(&self, id: Option<window::Id>, content: impl Into<El<'a>>) -> El<'a> {
        match id {
            Some(id) => mouse_area(content).on_press(UiMessage::Window(windows::Msg::Drag(id))).into(),
            None => content.into(),
        }
    }
}

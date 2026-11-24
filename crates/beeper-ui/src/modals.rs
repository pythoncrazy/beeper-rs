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
    /// `BEEPER_CONNECT_ACCESS_TOKEN_REQUEST`, which a token request of the Desktop API opens over the main window.
    TokenRequest,
    /// `ADD_BEEPER_CONNECT_TOKEN`, the "+" of Settings > Integrations.
    AddToken,
    /// `LEGAL_NOTICES`, from Settings > About.
    Legal,
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
            Modal::TokenRequest => Shape { size: Size::new(460.0, 330.0), resizable: false, frame: true },
            // Its `minHeight` of 320 loses to Electron's 400 too.
            Modal::AddToken => Shape { size: Size::new(760.0, 360.0), resizable: true, frame: true },
            // `gm`'s default: 500 by 450.
            Modal::Legal => Shape { size: Size::new(500.0, 450.0), resizable: false, frame: true },
        }
    }

    /// The window title: `gm`'s `title`, else the app's name, and the class's `title`, which Desktop does not
    /// translate.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Modal::RecoveryKey => "Recovery Key",
            Modal::ChangeEmail => "Change Email",
            // The `<title>` that `LegalNotices` renders stays in the body, after the head's, under React 18.
            Modal::TokenRequest | Modal::AddToken | Modal::Legal => "Beeper",
        }
    }

    /// Whether Escape closes the window: `onEscapePressed` of a class, and the modals of Desktop's `handleEscape`.
    fn escape_closes(self) -> bool {
        matches!(self, Modal::RecoveryKey | Modal::ChangeEmail)
    }
}

/// beeper-rs's counterpart of Desktop's `dependencies.txt`: its license and the notices of what it bundles.
const LEGAL_NOTICES: &str = concat!(include_str!("../../../LICENSE"), "\n", include_str!("../../../NOTICE"));

/// `gm`'s `maxWidth` of a modal window that resizes, at least 700. Its height has no limit.
fn max_size(width: f32) -> Size {
    Size::new(width.max(700.0), 100_000.0)
}

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
            max_size: (resizable && frame).then(|| max_size(size.width)),
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
            Modal::TokenRequest => self.token_ask = None,
            Modal::AddToken => self.integrations.add = None,
            Modal::Legal => {}
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
            Modal::TokenRequest => self.token_ask.as_ref().map_or_else(|| Space::new().into(), |a| self.view_token_ask(a)),
            Modal::AddToken => self.integrations.add.as_ref().map_or_else(|| Space::new().into(), |a| self.view_add_token(a)),
            Modal::Legal => self.view_legal_notices(),
        }
    }

    /// Desktop's `LegalNotices` (`_Ht`): its `dependencies.txt` in a `<pre>`, which wraps at any character, in
    /// 16 px monospace inside 16 by 32 px on the elevated color, and scrolls. beeper-rs's notices are its license and
    /// its NOTICE file.
    fn view_legal_notices(&self) -> El<'_> {
        let p = &self.pal;
        let notes = txt(LEGAL_NOTICES, 16.0).font(iced::Font::MONOSPACE).wrapping(iced::widget::text::Wrapping::Glyph).color(p.text);
        let page = container(notes).padding([16, 32]).width(Fill);
        let page = scrollable(page).direction(bar()).style(scroll_style(p.scroll)).height(Fill);
        container(page).width(Fill).height(Fill).style(fill_box(p.elevated, 0.0)).into()
    }

    /// A key in a window of [`Modal`], where Escape closes some.
    pub(crate) fn modal_key(&mut self, id: window::Id, key: &Key) -> Task<UiMessage> {
        let escape = *key == Key::Named(keyboard::key::Named::Escape);
        match escape && self.modal_at(id).is_some_and(Modal::escape_closes) {
            true => self.close_window(id),
            false => Task::none(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::{clicks, demo_app};
    use iced_test::simulator::simulator;

    /// Desktop's About > Legal Notices: the `LEGAL_NOTICES` modal window, 500 by 450 and fixed, titled with the app's
    /// name, with the notices in a `<pre>`; Escape does not close it.
    #[test]
    fn legal_notices_open_their_window_like_desktop() {
        let mut app = demo_app("legal-notices");
        let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::About)));
        for m in clicks(app.view_settings(), "Legal Notices") {
            let _ = app.update(m);
        }
        let id = app.modal_window(Modal::Legal).expect("Legal Notices opens its window");
        assert_eq!(app.title(id), "Beeper");
        let shape = Modal::Legal.shape();
        assert!(shape.size == Size::new(500.0, 450.0) && !shape.resizable && shape.frame);
        let mut sim = simulator(app.view_modal(Modal::Legal));
        assert!(sim.find(LEGAL_NOTICES).is_ok() && LEGAL_NOTICES.starts_with("MIT License") && LEGAL_NOTICES.contains("Third-Party"));
        drop(sim);
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(keyboard::key::Named::Escape), keyboard::Modifiers::empty()));
        assert_eq!(app.modal_window(Modal::Legal), Some(id), "Escape keeps it, like Desktop's `handleEscape`");
        let _ = app.update(UiMessage::OpenModal(Modal::Legal, app.settings_window));
        assert_eq!(app.modals.len(), 1, "it opens once");
    }
}

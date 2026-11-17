//! Desktop's keyboard focus outside text fields, which Chromium gives it: Tab and Shift+Tab move the focus through
//! the controls of a window in their DOM order and wrap around, Enter and Space press the focused control, and a
//! mouse press takes the focus away. A control shows its focus only when a key moved it there (`:focus-visible`).

use super::*;
use iced::advanced::widget::operation::{Focusable, Operation, Outcome};

/// The control of a window that has the keyboard focus, out of the window's stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focus<S>(Option<S>);

impl<S> Default for Focus<S> {
    fn default() -> Self {
        Self(None)
    }
}

impl<S: Copy + PartialEq> Focus<S> {
    /// Chromium's sequential focus navigation: Tab (Shift+Tab with `back`) moves to the stop after (before) the
    /// focused one and wraps around, and starts at the first (last) stop. A window without stops keeps no focus.
    pub fn step(&mut self, stops: &[S], back: bool) -> Option<S> {
        let n = stops.len();
        let at = self.0.and_then(|s| stops.iter().position(|t| *t == s));
        self.0 = match (at, back) {
            _ if n == 0 => None,
            (None, false) => Some(stops[0]),
            (None, true) => Some(stops[n - 1]),
            (Some(i), false) => Some(stops[(i + 1) % n]),
            (Some(i), true) => Some(stops[(i + n - 1) % n]),
        };
        self.0
    }

    pub fn get(self) -> Option<S> {
        self.0
    }

    pub fn is(self, stop: S) -> bool {
        self.0 == Some(stop)
    }

    pub fn set(&mut self, stop: Option<S>) {
        self.0 = stop;
    }
}

/// Where the caret is when a key comes: in the text field of the window, in another text field, or in none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caret {
    None,
    Field,
    Other,
}

/// The mark that the root of the view of a window leaves for the operations, which go through the widgets of all
/// the windows: the widgets after it belong to the window.
pub struct WindowMark(pub window::Id);

/// Finds where the caret is in `window`, for `field` the ID of its text field.
pub fn caret(window: window::Id, field: &'static str) -> Task<Caret> {
    iced::advanced::widget::operate(FindCaret::new(window, field))
}

/// The operation of [`caret`].
struct FindCaret {
    window: window::Id,
    field: iced::widget::Id,
    inside: bool,
    caret: Caret,
}

impl FindCaret {
    fn new(window: window::Id, field: &'static str) -> Self {
        Self { window, field: iced::widget::Id::new(field), inside: false, caret: Caret::None }
    }
}

impl Operation<Caret> for FindCaret {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Caret>)) {
        operate(self)
    }

    fn custom(&mut self, _: Option<&iced::widget::Id>, _: iced::Rectangle, state: &mut dyn std::any::Any) {
        if let Some(WindowMark(w)) = state.downcast_ref() {
            self.inside = *w == self.window;
        }
    }

    fn focusable(&mut self, id: Option<&iced::widget::Id>, _: iced::Rectangle, state: &mut dyn Focusable) {
        if self.inside && state.is_focused() && self.caret != Caret::Field {
            self.caret = if id == Some(&self.field) { Caret::Field } else { Caret::Other };
        }
    }

    fn finish(&self) -> Outcome<Caret> {
        Outcome::Some(self.caret)
    }
}

/// Takes the keyboard focus from the text fields, as Tab does when it leaves one: no widget has the empty ID.
pub fn blur<T>() -> Task<T> {
    operation::focus("")
}

/// Desktop's `:focus-visible` outline of its `Button`, `Toggle` and `Checkbox` (`outline: 2px solid
/// var(--app-focus-ring)` with `outline-offset: 1px`) around `content`, whose corners have `radius`.
pub fn outline<'a>(content: impl Into<El<'a>>, on: bool, p: &Palette, radius: f32) -> El<'a> {
    ring::ring(content, if on { p.focus_ring } else { CLEAR }, 2.0, radius).offset(1.0).into()
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// Tab (Shift+Tab with `back`) in a window, after a look for the caret.
    Tab(window::Id, bool, Caret),
    /// A mouse press in a window: Chromium moves the focus away from the focused control.
    Press(window::Id),
}

impl BeeperGuiApp {
    pub(crate) fn key_focus(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            // Chromium's Tab in another text field of the window goes to controls that beeper-rs does not stop at.
            Msg::Tab(_, _, Caret::Other) => Task::none(),
            Msg::Tab(id, back, caret) if Some(id) == self.settings_window => self.settings_tab(back, caret),
            Msg::Tab(id, back, caret) => self.nickname_tab(id, back, caret),
            Msg::Press(id) => {
                if Some(id) == self.main_window {
                    self.pane.key_focus.set(None);
                }
                if let Some(w) = self.chat_windows.iter_mut().find(|w| w.0 == id) {
                    w.1.key_focus.set(None);
                }
                if Some(id) == self.settings_window {
                    self.settings_keys.set(None);
                }
                if let Some(d) = self.nickname_windows.iter_mut().find(|d| d.id == id) {
                    d.focus.set(None);
                }
                Task::none()
            }
        }
    }

    /// Whether a control of a window has the keyboard focus, which the next mouse press takes.
    pub(crate) fn key_focused(&self) -> bool {
        self.pane.key_focus.get().is_some()
            || self.chat_windows.iter().any(|w| w.1.key_focus.get().is_some())
            || self.settings_keys.get().is_some()
            || self.nickname_windows.iter().any(|d| d.focus.get().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use iced_test::simulator::Simulator;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Stop {
        A,
        B,
        C,
    }

    /// Chromium's Tab order: the first stop, then each next one and back to the first; Shift+Tab goes the other way
    /// and starts at the last stop. A stop that left the window counts as no focus.
    #[test]
    fn tab_wraps_around_like_chromium() {
        let stops = [Stop::A, Stop::B, Stop::C];
        let mut f = Focus::default();
        let tabs: Vec<_> = (0..4).map(|_| f.step(&stops, false)).collect();
        assert_eq!(tabs, [Stop::A, Stop::B, Stop::C, Stop::A].map(Some));
        assert_eq!(f.step(&stops, true), Some(Stop::C), "Shift+Tab wraps back");
        assert_eq!(Focus::default().step(&stops, true), Some(Stop::C), "Shift+Tab starts at the last stop");
        f.set(Some(Stop::B));
        assert_eq!(f.step(&[Stop::A, Stop::C], false), Some(Stop::A), "a stop that left starts over");
        assert_eq!(f.step(&[], false), None, "no stops keep no focus");
    }

    /// The caret of a window: in its text field, in another one, or in none. The caret of another window, which
    /// iced keeps focused, does not count.
    #[test]
    fn the_caret_is_looked_for_in_its_window() {
        use iced::advanced::renderer::Headless;
        let mut app = demo_app("focus-caret");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        let main = app.main_window.expect("the main window");
        let mut renderer =
            iced::futures::executor::block_on(iced::Renderer::new(iced::Font::DEFAULT, Pixels(13.0), None))
                .expect("a headless renderer");
        let size = Size::new(1200.0, 800.0);
        let mut ui = iced_runtime::UserInterface::build(app.view(main), size, Default::default(), &mut renderer);
        let mut find = |focus: Option<&'static str>, window, field| {
            if let Some(id) = focus {
                ui.operate(&renderer, &mut iced::advanced::widget::operation::focusable::focus(id.into()));
            }
            let mut op = FindCaret::new(window, field);
            ui.operate(&renderer, &mut iced::advanced::widget::operation::black_box(&mut op));
            op.caret
        };
        assert_eq!(find(None, main, "composer-input"), Caret::None, "no text field has the caret");
        assert_eq!(find(Some("composer-input"), main, "composer-input"), Caret::Field, "the composer has it");
        assert_eq!(find(None, main, "settings-search"), Caret::Other, "a field other than the window's");
        assert_eq!(find(None, window::Id::unique(), "settings-search"), Caret::None, "another window's caret");
    }

    /// The pixels of `view` at `size`, from the simulator's snapshot.
    fn shot(view: El<'_>, size: Size, name: &str) -> ::image::RgbaImage {
        let mut sim = Simulator::with_size(Default::default(), size, view);
        let dir = std::env::temp_dir().join(format!("beeper-rs-focus-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = sim.snapshot(&iced::Theme::Light).unwrap().matches_image(dir.join("shot"));
        let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let shot = ::image::open(&png).unwrap().into_rgba8();
        std::fs::remove_dir_all(&dir).unwrap();
        shot
    }

    fn key(app: &mut BeeperGuiApp, id: window::Id, k: Named) {
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(k), Modifiers::empty()));
    }

    /// Desktop's `RenameAccount` form: Tab goes from the name field to Cancel and Save and back to the field, and
    /// Shift+Tab goes back. Only the focused button draws the 2 px `--app-focus-ring` outline, 1 px outside it, and a
    /// mouse press takes the focus and the ring. Enter presses the focused button.
    #[test]
    fn the_nickname_window_tabs_through_its_buttons_like_desktop() {
        use crate::nickname::{Msg as Nick, Stop as N, NICKNAME_SIZE};
        let mut app = demo_app("focus-nickname");
        let account = app.store.read().bridge_accounts[0].account_id.clone();
        let _ = app.update(UiMessage::Nickname(Nick::Open(account.clone())));
        let id = app.nickname_windows[0].id;
        let tab = |app: &mut BeeperGuiApp, back: bool, in_field: bool| {
            let caret = if in_field { Caret::Field } else { Caret::None };
            let _ = app.update(UiMessage::Focus(Msg::Tab(id, back, caret)));
            app.nickname_windows[0].focus.get()
        };
        assert_eq!(tab(&mut app, false, true), Some(N::Cancel), "Tab leaves the field for Cancel");
        assert_eq!(tab(&mut app, false, false), Some(N::Save));
        assert_eq!(tab(&mut app, false, false), None, "and wraps around to the field");
        assert_eq!(tab(&mut app, true, true), Some(N::Save), "Shift+Tab goes back");
        // The buttons are 28 px high, 8 + 10 px above the bottom: their rings run along the row 2 px above them.
        let rings = |app: &BeeperGuiApp, name: &str| {
            let shot = shot(app.view_nickname(&app.nickname_windows[0]), NICKNAME_SIZE, name);
            let (w, h) = (NICKNAME_SIZE.width as u32, NICKNAME_SIZE.height as u32);
            let k = shot.width() / w;
            let ring = |x: u32| shot.get_pixel(x * k, (h - 18 - 28 - 2) * k).0 == [0x0d, 0x4f, 0xfb, 255];
            ((0..w / 2).filter(|&x| ring(x)).count(), (w / 2..w).filter(|&x| ring(x)).count())
        };
        let (cancel, save) = rings(&app, "save");
        assert!(cancel == 0 && save > 10, "Save draws the ring: {cancel} {save}");
        tab(&mut app, true, false);
        let (cancel, save) = rings(&app, "cancel");
        assert!(cancel > 10 && save == 0, "Cancel draws it: {cancel} {save}");
        let _ = app.update(UiMessage::Focus(Msg::Press(id)));
        assert_eq!(app.nickname_windows[0].focus.get(), None, "a mouse press takes the focus");
        assert_eq!(rings(&app, "none"), (0, 0), "and the ring");
        let _ = app.update(UiMessage::Nickname(Nick::Name(id, "Work".into())));
        tab(&mut app, true, true);
        key(&mut app, id, Named::Enter);
        assert_eq!(app.store.read().custom_name(&account), Some("Work"), "Enter presses Save");
    }

    /// Desktop's `KeyboardHandlers` give any key outside a text field to the composer: Tab in the composer goes to
    /// the button after it and Shift+Tab to the button before it, and Tab on a button goes back to the composer.
    /// The buttons draw no ring (`button { outline: 0 }`). Enter presses the focused button, and typing goes to the
    /// composer.
    #[test]
    fn the_composer_row_tabs_like_desktop() {
        use crate::composer::Stop as C;
        let mut app = demo_app("focus-composer");
        let main = app.main_window.expect("the main window");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        let tab = |app: &mut BeeperGuiApp, back: bool, in_field: bool| {
            let caret = if in_field { Caret::Field } else { Caret::None };
            let _ = app.update(UiMessage::Composer(composer::Msg::Tab(back, caret)));
            app.pane.key_focus.get()
        };
        let stops = app.composer_stops();
        assert_eq!(stops, [C::Toggle, C::Field, C::Action(5)], "the toggle, the composer and Record Audio");
        assert_eq!(tab(&mut app, false, true), Some(C::Action(5)), "Tab goes to the button after the composer");
        let size = Size::new(1200.0, 800.0);
        let focused = shot(app.view_main(), size, "composer-focused");
        assert_eq!(tab(&mut app, false, false), None, "Tab on a button goes back to the composer");
        assert!(focused == shot(app.view_main(), size, "composer-none"), "a focused button looks the same");
        assert_eq!(tab(&mut app, true, true), Some(C::Toggle), "Shift+Tab goes to the toggle");
        key(&mut app, main, Named::Enter);
        assert!(app.pane.show_attach_menu, "Enter presses the toggle");
        assert_eq!(app.pane.key_focus.get(), None, "and the keyboard goes back to the composer");
        let unveiled = app.composer_stops();
        let before = unveiled[unveiled.iter().position(|s| *s == C::Field).expect("the composer") - 1];
        assert!(unveiled.len() > stops.len() && matches!(before, C::Action(_)), "the unveiled actions take focus");
        assert_eq!(tab(&mut app, true, true), Some(before), "Shift+Tab goes to the last of them");
        let _ = app.update(UiMessage::WindowText(main, "h".into()));
        assert_eq!(app.pane.key_focus.get(), None, "typing goes to the composer");
        assert_eq!(app.composer_stops().last(), Some(&C::Send), "and a draft turns Send on");
    }

    /// Desktop's Settings window: Tab goes from the search field to the `label-wrapper` rows of the pane, and the
    /// focused row takes 10 % of the text color. Space and Enter flip its switch, or offer Beeper Plus for a locked
    /// feature. Shift+Tab goes back to the search.
    #[test]
    fn settings_rows_take_the_keyboard_like_desktop() {
        let mut app = demo_app("focus-settings");
        let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
        let id = app.settings_window.expect("the Settings window");
        let tab = |app: &mut BeeperGuiApp, back: bool, in_field: bool| {
            let caret = if in_field { Caret::Field } else { Caret::None };
            let _ = app.update(UiMessage::Focus(Msg::Tab(id, back, caret)));
            app.settings_keys.get()
        };
        let plain = shot(app.view_settings(), SETTINGS_SIZE, "settings-plain");
        let Some(settings::Stop::Pref(row)) = tab(&mut app, false, true) else {
            panic!("Tab leaves the search for the first switch row")
        };
        let tinted = shot(app.view_settings(), SETTINGS_SIZE, "settings-tinted");
        let changed: Vec<_> = plain.pixels().zip(tinted.pixels()).filter(|(a, b)| a != b).collect();
        assert!(changed.len() > 10_000, "the row is tinted: {}", changed.len());
        // `rgba(0 0 0 / 0.1)` over the white group.
        let tint = |c: &u8| (229..=230).contains(c);
        let white: Vec<_> = changed.iter().filter(|(a, _)| a.0 == [255; 4]).collect();
        assert!(white.len() > 10_000 && white.iter().all(|(_, b)| b.0[..3].iter().all(tint)), "10 % black on white");
        assert_eq!(row.key, "incognito", "the first switch row");
        let _ = app.update(UiMessage::WindowText(id, " ".into()));
        assert_eq!(app.active_settings_tab, SettingsTab::BeeperPlus, "Space on a locked row offers Beeper Plus");
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Notifications)));
        let Some(settings::Stop::Pref(row)) = tab(&mut app, false, true) else { panic!("a switch row") };
        let before = app.store.read().settings.pref(row.key);
        let _ = app.update(UiMessage::WindowText(id, " ".into()));
        assert_ne!(app.store.read().settings.pref(row.key), before, "Space flips the switch of {}", row.key);
        key(&mut app, id, Named::Enter);
        assert_eq!(app.store.read().settings.pref(row.key), before, "and so does Enter");
        assert!(matches!(tab(&mut app, false, false), Some(settings::Stop::Pref(r)) if r != row), "Tab goes on");
        tab(&mut app, true, false);
        assert_eq!(tab(&mut app, true, false), None, "Shift+Tab goes back to the search");
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Accounts)));
        assert_eq!(tab(&mut app, false, true), Some(settings::Stop::Account(0)), "the Beeper account comes first");
    }
}

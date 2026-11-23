//! Desktop's keyboard focus outside text fields, which Chromium gives it: Tab and Shift+Tab move the focus through
//! the controls of a window in their DOM order and wrap around, Enter and Space press the focused control, and a
//! mouse press takes the focus away. A control shows its focus only when a key moved it there (`:focus-visible`).
//!
//! The views mark the controls that Tab reaches with [`BeeperGuiApp::target`]; a Tab collects the marks and the
//! text fields of the window in their layout order, which is Desktop's DOM order.

use super::*;
use crate::prelude::*;
use iced::advanced::widget::operation::{Focusable, Operation, Outcome, Scrollable};
use iced::advanced::widget::{self as advanced};

/// Tells when the text field inside `content` takes or loses the keyboard focus, like the DOM's `focus` and `blur`
/// events: `on_change` gets whether it has the focus now.
pub fn watch<'a>(content: impl Into<El<'a>>, on_change: fn(bool) -> UiMessage) -> El<'a> {
    El::new(Watch { content: content.into(), on_change })
}

/// The widget of [`watch`].
struct Watch<'a> {
    content: El<'a>,
    on_change: fn(bool) -> UiMessage,
}

/// Whether the text field had the focus at the last event.
#[derive(Default)]
struct Watched(bool);

/// The operation that finds whether a text field has the focus.
struct HasFocus(bool);

impl Operation for HasFocus {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self)
    }

    fn focusable(&mut self, _: Option<&iced::widget::Id>, _: Rectangle, state: &mut dyn Focusable) {
        self.0 |= state.is_focused();
    }
}

impl Widget<UiMessage, Theme, Renderer> for Watch<'_> {
    crate::delegate::delegate!(state(Watched), children, diff, size, layout, operate, mouse_interaction, draw, overlay(UiMessage));

    fn update(
        &mut self, tree: &mut advanced::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, UiMessage>, viewport: &Rectangle,
    ) {
        let content = &mut tree.children[0];
        self.content.as_widget_mut().update(content, event, layout, cursor, renderer, clipboard, shell, viewport);
        let mut focus = HasFocus(false);
        self.content.as_widget_mut().operate(content, layout, renderer, &mut focus);
        let was = tree.state.downcast_mut::<Watched>();
        if was.0 != focus.0 {
            was.0 = focus.0;
            shell.publish((self.on_change)(focus.0));
        }
    }
}

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
        let at = self.0.and_then(|s| stops.iter().position(|t| *t == s));
        self.0 = next(at, stops.len(), back).map(|i| stops[i]);
        self.0
    }

    pub fn get(self) -> Option<S> {
        self.0
    }

    pub fn set(&mut self, stop: Option<S>) {
        self.0 = stop;
    }
}

/// The index of the stop after (before, with `back`) the stop at `at` out of `n`, wrapping around; with no stop at
/// `at`, the first (last) one.
fn next(at: Option<usize>, n: usize, back: bool) -> Option<usize> {
    match (at, back) {
        _ if n == 0 => None,
        (None, false) => Some(0),
        (None, true) => Some(n - 1),
        (Some(i), false) => Some((i + 1) % n),
        (Some(i), true) => Some((i + n - 1) % n),
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
    advanced::operate(FindCaret::new(window, field))
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

    fn custom(&mut self, _: Option<&iced::widget::Id>, _: Rectangle, state: &mut dyn std::any::Any) {
        if let Some(WindowMark(w)) = state.downcast_ref() {
            self.inside = *w == self.window;
        }
    }

    fn focusable(&mut self, id: Option<&iced::widget::Id>, _: Rectangle, state: &mut dyn Focusable) {
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

/// What a control does with the keys while it has the focus.
#[derive(Clone, Default)]
pub struct Keys {
    /// Enter and Space press it.
    press: Option<UiMessage>,
    /// Only Space presses it, like a checkbox.
    space_only: bool,
    /// The arrows: a select or a range input steps its value, and a grid moves its focused cell.
    arrows: Option<std::sync::Arc<dyn Fn(Named) -> Option<UiMessage> + Send + Sync>>,
    /// The message when Tab gives it the focus.
    focused: Option<UiMessage>,
    /// The control has the focus without showing it when the window opens (`autoFocus`, or the default button of a
    /// GTK dialog): Tab starts from it, and Enter and Space press it.
    default: bool,
}

impl Keys {
    /// A button, or a `role="button"` element with Desktop's `onKeyDown` for Enter and Space.
    pub fn press(msg: impl Into<Option<UiMessage>>) -> Self {
        Self { press: msg.into(), ..Self::default() }
    }

    /// A checkbox or a switch input, which Space flips and Enter does not.
    pub fn checkbox(msg: UiMessage) -> Self {
        Self { press: Some(msg), space_only: true, ..Self::default() }
    }

    /// A select or a range input, whose ← and ↑ step its value back, and → and ↓ forth.
    pub fn step(back: Option<UiMessage>, forth: Option<UiMessage>) -> Self {
        Self::arrows(move |k| match k {
            Named::ArrowLeft | Named::ArrowUp => back.clone(),
            _ => forth.clone(),
        })
    }

    /// A control whose arrows send the message for the arrow key.
    pub fn arrows(f: impl Fn(Named) -> Option<UiMessage> + Send + Sync + 'static) -> Self {
        Self { arrows: Some(std::sync::Arc::new(f)), ..Self::default() }
    }

    /// Enter and Space also press the control with `msg`.
    pub fn pressed(self, msg: UiMessage) -> Self {
        Self { press: Some(msg), ..self }
    }

    /// The control also sends `msg` when Tab gives it the focus.
    pub fn on_focus(self, msg: UiMessage) -> Self {
        Self { focused: Some(msg), ..self }
    }

    /// The control has the focus when the window opens, without showing it.
    pub fn default_focus(self) -> Self {
        Self { default: true, ..self }
    }
}

/// How a control shows the keyboard focus.
#[derive(Clone, Copy)]
pub enum Look {
    /// Not at all: Desktop's `button { outline: 0 }`, or a control without focus styles over an `outline: none`.
    Hidden,
    /// `outline: <width>px solid <color>` with `outline-offset: <offset>px`, around corners of `radius`.
    Outline { color: Color, width: f32, offset: f32, radius: f32 },
    /// Chromium's `outline: auto` of an element without focus styles, like Desktop's `dnd-kit` sortables: a 2 px
    /// `-webkit-focus-ring-color` ring on its edge, in a 1 px white halo.
    Auto,
    /// A fill behind the control, like `.label-wrapper:focus-visible`, with corners of the radius.
    Fill(Color, f32),
    /// GTK's focus line of a button in a native dialog: Adwaita's 1 px `dashed` outline at `outline-offset: -3px` in
    /// 30 % of the text color.
    Dashed(Color),
}

impl Look {
    /// The `:focus-visible` outline of Desktop's `Button`, `Toggle` and `Checkbox`: `outline: 2px solid
    /// var(--app-focus-ring)` with `outline-offset: 1px`.
    pub fn button(p: &Palette, radius: f32) -> Self {
        Look::Outline { color: p.focus_ring, width: 2.0, offset: 1.0, radius }
    }
}

/// Chromium's `-webkit-focus-ring-color` on Linux, which Desktop's sign-in links and sortables show.
pub const CHROMIUM_RING: Color = Color::from_rgb(229.0 / 255.0, 151.0 / 255.0, 0.0);

/// A control that Tab reaches: the content, and its mark for [`survey`].
struct Target<'a> {
    content: El<'a>,
    id: iced::widget::Id,
    keys: Keys,
}

/// The mark of a [`Target`] for the operations.
struct Mark(Keys);

/// The mark of a dialog in a window, which keeps the focus inside it like Desktop's `react-focus-lock`: the controls
/// before it do not count.
struct Trap;

impl Widget<UiMessage, Theme, Renderer> for Target<'_> {
    crate::delegate::delegate!(children, diff, size, layout, update(UiMessage), mouse_interaction, draw, overlay(UiMessage));

    fn operate(&mut self, tree: &mut advanced::Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn advanced::Operation) {
        operation.container(Some(&self.id), layout.bounds());
        operation.custom(Some(&self.id), layout.bounds(), &mut Mark(self.keys.clone()));
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }
}

/// A dialog in a window that keeps the focus inside it (see [`Trap`]).
pub fn trap<'a>(content: impl Into<El<'a>>) -> El<'a> {
    struct Trapped<'a> {
        content: El<'a>,
    }
    impl Widget<UiMessage, Theme, Renderer> for Trapped<'_> {
        crate::delegate::delegate!(children, diff, size, layout, update(UiMessage), mouse_interaction, draw, overlay(UiMessage));

        fn operate(&mut self, tree: &mut advanced::Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn advanced::Operation) {
            operation.custom(None, layout.bounds(), &mut Trap);
            self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
        }
    }
    Element::new(Trapped { content: content.into() })
}

/// A stop of a window: a text field, or a control with its keys.
#[derive(Clone)]
pub enum Stop {
    Field(iced::widget::Id),
    Control(iced::widget::Id, Box<Keys>),
}

impl Stop {
    fn id(&self) -> &iced::widget::Id {
        match self {
            Stop::Field(id) | Stop::Control(id, _) => id,
        }
    }
}

/// A scrollable of a window: its ID, its bounds, the bounds of its content, and its scroll offset.
#[derive(Clone)]
struct Scroll(iced::widget::Id, Rectangle, Rectangle, f32);

/// The stops of a window in their order with their bounds, the stop with the caret, a caret in a text field that is
/// no stop, the stop that has the focus when the window opens, and the scrollables.
#[derive(Clone, Default)]
pub struct Survey {
    stops: Vec<(Stop, Rectangle)>,
    caret: Option<usize>,
    other: bool,
    default: Option<usize>,
    scrolls: Vec<Scroll>,
}

impl Survey {
    fn position(&self, id: &iced::widget::Id) -> Option<usize> {
        self.stops.iter().position(|(s, _)| s.id() == id)
    }

    /// Chromium's `scrollIntoView({ block: "nearest" })` of the stop: the scroll of the innermost scrollable around it
    /// that shows it with the least move, if it moves.
    fn reveal(&self, i: usize) -> Option<(iced::widget::Id, f32)> {
        let b = self.stops[i].1;
        let around = self.scrolls.iter().filter(|s| s.2.y <= b.y && b.y + b.height <= s.2.y + s.2.height);
        let Scroll(id, view, content, at) = around.min_by(|a, c| a.2.height.total_cmp(&c.2.height))?;
        let y = b.y - content.y;
        let top = at.max(y + b.height - view.height).min(y).clamp(0.0, (content.height - view.height).max(0.0));
        (top != *at).then(|| (id.clone(), top))
    }
}

/// Collects the [`Survey`] of a window.
pub fn survey(window: window::Id) -> Task<Survey> {
    advanced::operate(Surveying { window, inside: false, survey: Survey::default() })
}

struct Surveying {
    window: window::Id,
    inside: bool,
    survey: Survey,
}

impl Operation<Survey> for Surveying {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Survey>)) {
        operate(self)
    }

    fn custom(&mut self, id: Option<&iced::widget::Id>, bounds: Rectangle, state: &mut dyn std::any::Any) {
        if let Some(WindowMark(w)) = state.downcast_ref() {
            self.inside = *w == self.window;
        }
        if !self.inside {
            return;
        }
        let s = &mut self.survey;
        if state.is::<Trap>() {
            (s.stops, s.caret, s.other, s.default) = (Vec::new(), None, false, None);
        }
        if let (Some(Mark(keys)), Some(id)) = (state.downcast_ref::<Mark>(), id) {
            if keys.default {
                s.default = Some(s.stops.len());
            }
            s.stops.push((Stop::Control(id.clone(), Box::new(keys.clone())), bounds));
        }
    }

    fn focusable(&mut self, id: Option<&iced::widget::Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        if !self.inside {
            return;
        }
        let s = &mut self.survey;
        match id {
            Some(id) => {
                if state.is_focused() {
                    s.caret = Some(s.stops.len());
                }
                s.stops.push((Stop::Field(id.clone()), bounds));
            }
            None => s.other |= state.is_focused(),
        }
    }

    fn scrollable(&mut self, id: Option<&iced::widget::Id>, bounds: Rectangle, content: Rectangle, at: Vector, _: &mut dyn Scrollable) {
        if let (true, Some(id)) = (self.inside, id) {
            self.survey.scrolls.push(Scroll(id.clone(), bounds, content, at.y));
        }
    }

    fn finish(&self) -> Outcome<Survey> {
        Outcome::Some(self.survey.clone())
    }
}

/// What a key asks of the focus of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Tab, or Shift+Tab with `true`.
    Tab(bool),
    /// Enter (`true`) or Space on the focused control.
    Press(bool),
    /// An arrow on the focused control.
    Arrow(Named),
}

#[derive(Clone)]
pub enum Msg {
    /// A key of the focus in a window, with the survey of the window.
    Surveyed(window::Id, Ask, Survey),
    /// A mouse press in a window: Chromium moves the focus away from the focused control.
    Press(window::Id),
}

impl BeeperGuiApp {
    pub(crate) fn key_focus(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Surveyed(window, ask, survey) => self.surveyed(window, ask, &survey),
            Msg::Press(id) => {
                if Some(id) == self.main_window {
                    self.pane.key_focus.set(None);
                }
                if let Some(w) = self.chat_windows.iter_mut().find(|w| w.0 == id) {
                    w.1.key_focus.set(None);
                }
                self.key_targets.remove(&id);
                Task::none()
            }
        }
    }

    fn surveyed(&mut self, window: window::Id, ask: Ask, survey: &Survey) -> Task<UiMessage> {
        let held = self.key_targets.get(&window).and_then(|id| survey.position(id));
        // A Tab in a text field that is no stop goes to controls that beeper-rs does not stop at.
        if survey.other {
            return Task::none();
        }
        let at = survey.caret.or(held).or(survey.default);
        let keys = || match &survey.stops[at?] {
            (Stop::Control(_, keys), _) if survey.caret.is_none() => Some(keys),
            _ => None,
        };
        let msg = match ask {
            Ask::Tab(back) => {
                let Some(i) = next(at, survey.stops.len(), back) else { return Task::none() };
                return match &survey.stops[i].0 {
                    Stop::Field(id) => {
                        self.key_targets.remove(&window);
                        operation::focus(id.clone())
                    }
                    Stop::Control(id, keys) => {
                        self.key_targets.insert(window, id.clone());
                        let focused = keys.focused.clone().map(|m| self.update(m));
                        // iced draws the scroll of an operation at the next message.
                        let shown = survey.reveal(i).map(|(scroll, y)| {
                            let to = operation::scroll_to(scroll, operation::AbsoluteOffset { x: None, y: Some(y) });
                            to.chain(Task::done(UiMessage::Noop))
                        });
                        Task::batch([blur()].into_iter().chain(shown).chain(focused))
                    }
                };
            }
            Ask::Press(enter) => keys().filter(|k| !(enter && k.space_only)).and_then(|k| k.press.clone()),
            Ask::Arrow(k) => keys().and_then(|keys| keys.arrows.as_ref()?(k)),
        };
        msg.map_or_else(Task::none, |m| self.update(m))
    }

    /// Chromium's keys of the focus in `window`: Tab and Shift+Tab, and Enter and the arrows on the focused control,
    /// or on the control with the focus when the window opens with `defaults`. Other keys are not the focus's.
    pub(crate) fn focus_key(&self, window: window::Id, key: &Key, mods: Modifiers, defaults: bool) -> Option<Task<UiMessage>> {
        let held = defaults || self.key_targets.contains_key(&window);
        let ask = match key.as_ref() {
            Key::Named(Named::Tab) if (mods - Modifiers::SHIFT).is_empty() => Ask::Tab(mods.shift()),
            Key::Named(Named::Enter) if mods.is_empty() && held => Ask::Press(true),
            Key::Named(k @ (Named::ArrowUp | Named::ArrowLeft | Named::ArrowDown | Named::ArrowRight))
                if mods.is_empty() && self.key_targets.contains_key(&window) =>
            {
                Ask::Arrow(k)
            }
            _ => return None,
        };
        Some(self.ask_focus(window, ask))
    }

    /// Space typed outside a text field of `window`, which presses its focused control.
    pub(crate) fn focus_space(&self, window: window::Id, defaults: bool) -> Option<Task<UiMessage>> {
        (defaults || self.key_targets.contains_key(&window)).then(|| self.ask_focus(window, Ask::Press(false)))
    }

    fn ask_focus(&self, window: window::Id, ask: Ask) -> Task<UiMessage> {
        survey(window).map(move |s| UiMessage::Focus(Msg::Surveyed(window, ask, s)))
    }

    /// A control of `window` that Tab reaches as `key`, with what the keys do on it and how it shows the focus.
    pub(crate) fn target<'a>(
        &self, window: Option<window::Id>, key: impl Into<String>, content: impl Into<El<'a>>, keys: Keys, look: Look,
    ) -> El<'a> {
        let key = key.into();
        let lifted = self.key_sort.as_ref().is_some_and(|s| Some(s.window) == window && s.hides(&key));
        let id = iced::widget::Id::from(key);
        let on = window.and_then(|w| self.key_targets.get(&w)) == Some(&id) && !lifted;
        let content: El<'a> = match look {
            Look::Hidden => content.into(),
            Look::Outline { color, width, offset, radius } => {
                ring::ring(content, if on { color } else { CLEAR }, width, radius).offset(offset).into()
            }
            Look::Auto => {
                let (edge, halo) = if on { (CHROMIUM_RING, Color::WHITE) } else { (CLEAR, CLEAR) };
                ring::ring(ring::ring(content, edge, 2.0, 2.0).offset(-1.0), halo, 1.0, 2.0).offset(1.0).into()
            }
            Look::Fill(color, radius) => container(content).style(fill_box(if on { color } else { CLEAR }, radius)).into(),
            Look::Dashed(color) => {
                let line = container(crate::dashed::dashes(color.scale_alpha(0.3), 2.0, 1.0)).padding(3);
                stack![content.into()].push(on.then_some(line)).into()
            }
        };
        Element::new(Target { content, id, keys })
    }

    /// Whether a control of a window has the keyboard focus, which the next mouse press takes.
    pub(crate) fn key_focused(&self) -> bool {
        self.pane.key_focus.get().is_some()
            || self.chat_windows.iter().any(|w| w.1.key_focus.get().is_some())
            || !self.key_targets.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use iced::advanced::renderer::Headless;
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

    fn renderer() -> iced::Renderer {
        iced::futures::executor::block_on(iced::Renderer::new(iced::Font::DEFAULT, Pixels(13.0), Some("tiny-skia")))
            .expect("a headless renderer")
    }

    /// The caret of a window: in its text field, in another one, or in none. The caret of another window, which
    /// iced keeps focused, does not count.
    #[test]
    fn the_caret_is_looked_for_in_its_window() {
        let mut app = demo_app("focus-caret");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        let main = app.main_window.expect("the main window");
        let mut renderer = renderer();
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

    /// The survey of `window` at `size`, as a key collects it, with the caret in the text field `caret`.
    pub(crate) fn survey_of(app: &BeeperGuiApp, window: window::Id, size: Size, caret: Option<&'static str>) -> Survey {
        let mut renderer = renderer();
        let mut ui = iced_runtime::UserInterface::build(app.view(window), size, Default::default(), &mut renderer);
        if let Some(id) = caret {
            ui.operate(&renderer, &mut iced::advanced::widget::operation::focusable::focus(id.into()));
        }
        let mut op = Surveying { window, inside: false, survey: Survey::default() };
        ui.operate(&renderer, &mut iced::advanced::widget::operation::black_box(&mut op));
        op.survey
    }

    /// The IDs of the stops of a survey.
    pub(crate) fn ids(s: &Survey) -> Vec<String> {
        s.stops.iter().map(|(stop, _)| format!("{:?}", stop.id()).split('"').nth(1).unwrap_or("").to_string()).collect()
    }

    /// A key of the focus in `window`, with the survey of its view.
    pub(crate) fn ask(app: &mut BeeperGuiApp, window: window::Id, size: Size, caret: Option<&'static str>, ask: Ask) {
        let survey = survey_of(app, window, size, caret);
        let _ = app.update(UiMessage::Focus(Msg::Surveyed(window, ask, survey)));
    }

    /// The ID of the control of `window` with the keyboard focus.
    pub(crate) fn held(app: &BeeperGuiApp, window: window::Id) -> Option<String> {
        app.key_targets.get(&window).map(|id| format!("{id:?}").split('"').nth(1).unwrap_or("").to_string())
    }

    /// The pixels of `view` at `size`, from the simulator's snapshot.
    pub(crate) fn shot(view: El<'_>, size: Size, name: &str) -> ::image::RgbaImage {
        pixels(&mut Simulator::with_size(Default::default(), size, view), &iced::Theme::Light, name)
    }

    /// The pixels of the snapshot of `sim` in `theme`. The snapshot goes through a PNG in a folder of its own, also
    /// for a `name` that another test of the process takes at the same time.
    pub(crate) fn pixels(sim: &mut Simulator<'_, UiMessage>, theme: &iced::Theme, name: &str) -> ::image::RgbaImage {
        static SHOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SHOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("beeper-rs-shot-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = sim.snapshot(theme).unwrap().matches_image(dir.join("shot"));
        let png = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        let shot = ::image::open(&png).unwrap().into_rgba8();
        std::fs::remove_dir_all(&dir).unwrap();
        shot
    }

    /// The tests of a process run at once, and two of them may snapshot under the same name, like the space bar
    /// and Edit Spaces tests with "held" and "lifted". Each snapshot reads its own pixels.
    #[test]
    fn snapshots_of_one_name_do_not_collide() {
        let size = Size::new(40.0, 20.0);
        let widths: Vec<u32> = std::thread::scope(|s| {
            let runs: Vec<_> =
                (0..8).map(|_| s.spawn(move || (0..4).map(|_| shot(iced::widget::text("x").into(), size, "same").width()).sum())).collect();
            runs.into_iter().map(|r| r.join().expect("a snapshot of its own")).collect()
        });
        assert!(widths.iter().all(|&w| w >= 4 * 40), "{widths:?}");
    }

    /// Desktop's `RenameAccount` form: Tab goes from the name field to Cancel and Save and back to the field, and
    /// Shift+Tab goes back. Only the focused button draws the 2 px `--app-focus-ring` outline, 1 px outside it, and a
    /// mouse press takes the focus and the ring. Enter presses the focused button.
    #[test]
    fn the_nickname_window_tabs_through_its_buttons_like_desktop() {
        use crate::nickname::{Msg as Nick, NICKNAME_SIZE};
        let mut app = demo_app("focus-nickname");
        let account = app.store.read().bridge_accounts[0].account_id.clone();
        let _ = app.update(UiMessage::Nickname(Nick::Open(account.clone())));
        let id = app.nickname_windows[0].id;
        let field = Some("account-nickname");
        let survey = survey_of(&app, id, NICKNAME_SIZE, None);
        assert_eq!(ids(&survey), ["account-nickname", "cancel", "save"], "the field, Cancel and Save");
        ask(&mut app, id, NICKNAME_SIZE, field, Ask::Tab(false));
        assert_eq!(held(&app, id).as_deref(), Some("cancel"), "Tab leaves the field for Cancel");
        ask(&mut app, id, NICKNAME_SIZE, None, Ask::Tab(false));
        assert_eq!(held(&app, id).as_deref(), Some("save"));
        ask(&mut app, id, NICKNAME_SIZE, None, Ask::Tab(false));
        assert_eq!(held(&app, id), None, "and wraps around to the field");
        ask(&mut app, id, NICKNAME_SIZE, field, Ask::Tab(true));
        assert_eq!(held(&app, id).as_deref(), Some("save"), "Shift+Tab goes back");
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
        ask(&mut app, id, NICKNAME_SIZE, None, Ask::Tab(true));
        let (cancel, save) = rings(&app, "cancel");
        assert!(cancel > 10 && save == 0, "Cancel draws it: {cancel} {save}");
        let _ = app.update(UiMessage::Focus(Msg::Press(id)));
        assert_eq!(held(&app, id), None, "a mouse press takes the focus");
        assert_eq!(rings(&app, "none"), (0, 0), "and the ring");
        let _ = app.update(UiMessage::Nickname(Nick::Name(id, "Work".into())));
        ask(&mut app, id, NICKNAME_SIZE, field, Ask::Tab(true));
        ask(&mut app, id, NICKNAME_SIZE, None, Ask::Press(true));
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
        // The first frame finds the day under the header, whose date the next frames float.
        let _ = shot(app.view_main(), size, "composer-first");
        let focused = shot(app.view_main(), size, "composer-focused");
        assert_eq!(tab(&mut app, false, false), None, "Tab on a button goes back to the composer");
        assert!(focused == shot(app.view_main(), size, "composer-none"), "a focused button looks the same");
        assert_eq!(tab(&mut app, true, true), Some(C::Toggle), "Shift+Tab goes to the toggle");
        let _ = app.update(UiMessage::WindowKey(main, Key::Named(Named::Enter), Modifiers::empty()));
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

    /// The actions of `task` that are ready at once; its timers and sends never end here.
    fn ready_actions(task: Task<UiMessage>) -> Vec<iced_runtime::Action<UiMessage>> {
        use iced::futures::{FutureExt, StreamExt};
        let Some(mut stream) = iced_runtime::task::into_stream(task) else { return Vec::new() };
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("a runtime");
        let _in = runtime.enter();
        std::iter::from_fn(|| stream.next().now_or_never().flatten()).collect()
    }

    /// Whether the actions of `task` focus the text field `id`.
    fn focuses(task: Task<UiMessage>, id: &'static str) -> bool {
        use iced::advanced::widget::operation::Focusable;
        struct Field(bool);
        impl Focusable for Field {
            fn is_focused(&self) -> bool {
                self.0
            }
            fn focus(&mut self) {
                self.0 = true;
            }
            fn unfocus(&mut self) {
                self.0 = false;
            }
        }
        let mut field = Field(false);
        for a in ready_actions(task) {
            if let iced_runtime::Action::Widget(mut op) = a {
                op.focusable(Some(&iced::widget::Id::new(id)), Rectangle::default(), &mut field);
            }
        }
        field.0
    }

    /// After a click on the attachment button or on a sticker no text field has the keyboard. Desktop's
    /// `KeyboardHandlers` give Enter there to the composer, whose Enter sends. A drop of files focuses the composer
    /// (`focusComposerFor`) before they attach.
    #[test]
    fn enter_after_attaching_sends_like_desktop() {
        let mut app = demo_app("focus-enter");
        let main = app.main_window.expect("the main window");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        let png = vec![(iced::widget::image::Handle::from_rgba(1, 1, vec![255; 4]), 0)];
        let file = PendingMedia {
            name: "image.png".into(),
            mime: "image/png".into(),
            size: (1, 1),
            frames: png.into(),
            bytes: vec![0; 8],
            ..Default::default()
        };
        let attach = |app: &mut BeeperGuiApp| {
            let _ = app.update(UiMessage::Composer(composer::Msg::FilesLoaded(vec![Ok(file.clone())])));
            assert_eq!(app.pane.attachments.len(), 1);
        };
        let count = |app: &BeeperGuiApp| app.store.read().messages.iter().filter(|m| m.chat_id == NOTE_CHAT_ID).count();
        attach(&mut app);
        let before = count(&app);
        // The attachment button is among the unveiled actions of the composer row.
        let _ = app.update(UiMessage::Composer(composer::Msg::ToggleAttachMenu));
        assert!(app.pane.show_attach_menu);
        let task = app.update(UiMessage::WindowKey(main, Key::Named(Named::Enter), Modifiers::empty()));
        let looks = ready_actions(task);
        assert!(looks.iter().any(|a| matches!(a, iced_runtime::Action::Widget(_))), "Enter looks for the caret");
        let _ = app.update(UiMessage::Composer(composer::Msg::Enter(Caret::Other)));
        assert_eq!(app.pane.attachments.len(), 1, "Enter in another text field is its own");
        let _ = app.update(UiMessage::Composer(composer::Msg::Enter(Caret::Field)));
        assert_eq!(app.pane.attachments.len(), 1, "the composer sends its own Enter");
        let task = app.update(UiMessage::Composer(composer::Msg::Enter(Caret::None)));
        assert!(focuses(task, "composer-input"), "Enter outside a text field focuses the composer");
        assert!(app.pane.attachments.is_empty() && count(&app) == before + 1, "and sends the attachment");
        let _ = app.update(UiMessage::Composer(composer::Msg::FileHover(main, true)));
        let _ = app.update(UiMessage::Composer(composer::Msg::FileDropped(main, "/a".into())));
        let task = app.update(UiMessage::Composer(composer::Msg::DropEnded));
        assert!(focuses(task, "composer-input"), "a drop focuses the composer");
    }

    /// Desktop's Settings window: Tab goes from the search field to the controls of the pane in their order. A
    /// `label-wrapper` row takes 10 % of the text color, and Space and Enter flip its switch, or offer Beeper Plus for
    /// a locked feature. The arrows of a select pick the option after or before it. Shift+Tab goes back.
    #[test]
    fn settings_controls_take_the_keyboard_like_desktop() {
        let mut app = demo_app("focus-settings");
        let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::General)));
        let id = app.settings_window.expect("the Settings window");
        let search = Some("settings-search");
        let survey = survey_of(&app, id, SETTINGS_SIZE, None);
        let order = ids(&survey);
        assert_eq!(order[..3], ["settings-search", "pref-incognito", "pref-typingIndicators"], "{order:?}");
        let select = order.iter().position(|s| s == "select-afterArchive").expect("the After archive select");
        let remind = order.iter().position(|s| s == "select-remindLaterDefault").expect("the remind later select");
        assert!(3 < remind && remind < select, "the controls in their order: {order:?}");
        let plain = shot(app.view_settings(), SETTINGS_SIZE, "settings-plain");
        ask(&mut app, id, SETTINGS_SIZE, search, Ask::Tab(false));
        assert_eq!(held(&app, id).as_deref(), Some("pref-incognito"), "Tab leaves the search for the first row");
        let tinted = shot(app.view_settings(), SETTINGS_SIZE, "settings-tinted");
        let changed: Vec<_> = plain.pixels().zip(tinted.pixels()).filter(|(a, b)| a != b).collect();
        let tint = |c: &u8| (229..=230).contains(c);
        let white: Vec<_> = changed.iter().filter(|(a, _)| a.0 == [255; 4]).collect();
        assert!(white.len() > 10_000 && white.iter().all(|(_, b)| b.0[..3].iter().all(tint)), "10 % black on white");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Press(false));
        assert_eq!(app.active_settings_tab, SettingsTab::BeeperPlus, "Space on a locked row offers Beeper Plus");
        assert_eq!(held(&app, id), None, "and the new pane starts over");
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::General)));
        app.key_targets.insert(id, iced::widget::Id::from("select-afterArchive".to_string()));
        let before = app.store.read().settings.pref("afterArchive");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Arrow(Named::ArrowDown));
        assert_eq!(app.store.read().settings.pref("afterArchive"), before + 1, "↓ picks the next option");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Arrow(Named::ArrowUp));
        assert_eq!(app.store.read().settings.pref("afterArchive"), before, "↑ the one before");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Tab(true));
        assert_eq!(held(&app, id).as_deref(), Some(order[select - 1].as_str()), "Shift+Tab goes back");
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Notifications)));
        ask(&mut app, id, SETTINGS_SIZE, search, Ask::Tab(false));
        while !held(&app, id).expect("a control of Notifications").starts_with("pref-") {
            ask(&mut app, id, SETTINGS_SIZE, None, Ask::Tab(false));
        }
        let key = held(&app, id).expect("a switch row")["pref-".len()..].to_string();
        let before = app.store.read().settings.pref(&key);
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Press(false));
        assert_ne!(app.store.read().settings.pref(&key), before, "Space flips the switch of {key}");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Press(true));
        assert_eq!(app.store.read().settings.pref(&key), before, "and so does Enter");
        let _ = app.update(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::Accounts)));
        ask(&mut app, id, SETTINGS_SIZE, search, Ask::Tab(false));
        assert_eq!(held(&app, id).as_deref(), Some("account-hungryserv"), "the Beeper account comes first");
    }

    /// Desktop's main window without an open chat leaves Tab to Chromium: each space of the space bar shows the
    /// orange `outline: auto` ring of a `dnd-kit` sortable, then its button takes the focus without a ring, and Enter
    /// presses it. Shift+Tab starts at the end.
    #[test]
    fn the_main_window_without_a_chat_tabs_through_the_space_bar_like_desktop() {
        let mut app = demo_app("focus-main");
        let main = app.main_window.expect("the main window");
        app.pane.selected_chat_id = None;
        let size = Size::new(1200.0, 800.0);
        let order = ids(&survey_of(&app, main, size, None));
        assert_eq!(order[..2], ["space-inbox", "rail-inbox-button"], "{order:?}");
        let header = ["list-title", "list-filter", "list-search", "list-new-chat"];
        let at = order.iter().position(|s| s == "list-title").expect("the chat list header");
        assert_eq!(order[at..at + 4], header, "the header after the space bar: {order:?}");
        assert!(order[..at].contains(&"rail-settings".to_string()), "the settings button ends the space bar");
        ask(&mut app, main, size, None, Ask::Tab(false));
        assert_eq!(held(&app, main).as_deref(), Some("space-inbox"));
        // The space is 36 px wide at x 16 and y 14: the ring is 1 px outside it, in a white halo.
        let shot = shot(app.view_main(), size, "rail");
        let k = shot.width() / 1200;
        assert_eq!(shot.get_pixel(15 * k, 30 * k).0, [0xe5, 0x97, 0x00, 255], "Chromium's ring");
        assert_eq!(shot.get_pixel(14 * k, 30 * k).0, [255; 4], "its halo");
        app.key_targets.insert(main, iced::widget::Id::from("space-archive".to_string()));
        ask(&mut app, main, size, None, Ask::Tab(false));
        assert_eq!(held(&app, main).as_deref(), Some("rail-archive-button"));
        ask(&mut app, main, size, None, Ask::Press(true));
        assert_eq!(app.list().active_tab, InboxTab::Archived, "Enter presses the button of the space");
        app.key_targets.clear();
        let last = ids(&survey_of(&app, main, size, None)).pop();
        ask(&mut app, main, size, None, Ask::Tab(true));
        assert_eq!(held(&app, main), last, "Shift+Tab starts at the end");
    }

    /// A confirmation dialog keeps the focus inside it, like GTK's message box of Electron: Enter presses the default
    /// button, and Tab goes from it to Cancel.
    #[test]
    fn a_confirmation_dialog_keeps_the_focus_like_desktop() {
        let mut app = demo_app("focus-ask");
        let main = app.main_window.expect("the main window");
        let _ = app.update(UiMessage::Ask(Some(ASK_DISMISS_REMINDERS)));
        let size = Size::new(1200.0, 800.0);
        let survey = survey_of(&app, main, size, None);
        assert_eq!(ids(&survey), ["ask-cancel", "ask-0"], "only the buttons of the dialog");
        ask(&mut app, main, size, None, Ask::Tab(false));
        assert_eq!(held(&app, main).as_deref(), Some("ask-cancel"), "Tab goes from the default button to Cancel");
        app.key_targets.clear();
        ask(&mut app, main, size, None, Ask::Press(true));
        assert_eq!(app.confirm, None, "Enter presses the default button");
    }

    /// Desktop's `IconColorGrid` is one stop: the arrows move its focused cell, which shows the ring, and Enter picks
    /// the color or icon of the cell.
    #[test]
    fn the_label_grid_moves_its_cell_like_desktop() {
        use crate::dialogs::{LabelEdit, LabelStep};
        let mut app = demo_app("focus-grid");
        let _ = app.update(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())));
        let d = app.label_window.as_mut().expect("the New Label window");
        (d.step, d.name) = (LabelStep::Details, "Work".into());
        let (id, size) = (d.id, LABEL_SETTINGS_SIZE);
        let order = ids(&survey_of(&app, id, size, None));
        assert_eq!(order[..2], ["label-title", "grid"], "the name, then the grid: {order:?}");
        ask(&mut app, id, size, Some("label-title"), Ask::Tab(false));
        assert_eq!(held(&app, id).as_deref(), Some("grid"));
        let cell = |app: &BeeperGuiApp| app.label_window.as_ref().unwrap().cell;
        assert_eq!(cell(&app), None, "the grid starts without a cell");
        for (k, at) in [(Named::ArrowRight, 0), (Named::ArrowRight, 1), (Named::ArrowLeft, 0), (Named::ArrowUp, 0)] {
            ask(&mut app, id, size, None, Ask::Arrow(k));
            assert_eq!(cell(&app), Some(at), "{k:?}");
        }
        ask(&mut app, id, size, None, Ask::Arrow(Named::ArrowDown));
        let below = cell(&app).expect("a cell");
        assert!(below > 1, "↓ goes to the next row");
        ask(&mut app, id, size, None, Ask::Press(true));
        let d = app.label_window.as_ref().unwrap();
        assert!(d.color.is_some() || d.icon.is_some(), "Enter picks the cell");
        let _ = app.update(UiMessage::Labels(labels::Msg::Change(LabelEdit::Grid(None))));
        assert_eq!(cell(&app), None);
    }

    /// Desktop's New Poll and Edit Spaces sheets: Tab goes through their fields, buttons, switch rows and drag
    /// handles in their order, and the Poll Creator's Send shows a 2 px `--color-primary` ring 2 px outside it.
    #[test]
    fn the_poll_and_spaces_sheets_tab_like_desktop() {
        let mut app = demo_app("focus-sheets");
        let chat = app.store.read().chats.iter().find(|c| c.polls.is_some()).expect("a chat with polls").id.clone();
        let _ = app.update(UiMessage::OpenPollCreator(chat.clone()));
        let (poll, size) = (app.poll_windows[0].id, Size::new(400.0, 620.0));
        let order = ids(&survey_of(&app, poll, size, None));
        let question = format!("poll-{chat}");
        assert_eq!(order[0], question, "the question first: {order:?}");
        let add = order.iter().position(|s| s == "add").expect("Add an option");
        assert_eq!(order[add + 1], "Allow multiple answers", "Add an option, then the switch rows: {order:?}");
        let _ = app.update(UiMessage::Poll(poll, PollEdit::Question("Lunch?".into())));
        for (i, o) in ["Pizza", "Soup"].into_iter().enumerate() {
            let _ = app.update(UiMessage::Poll(poll, PollEdit::Option(i, o.into())));
        }
        assert_eq!(ids(&survey_of(&app, poll, size, None)).last().map(String::as_str), Some("send"), "Send works");
        app.key_targets.insert(poll, iced::widget::Id::from("send".to_string()));
        let shot = shot(app.view_poll_creator(&app.poll_windows[0]), size, "poll-send");
        let k = shot.width() / 400;
        // Send is 40 px high, 8 + 14 px above the bottom; its ring is 2 px wide, 2 px above it.
        let ring = (0..400).filter(|&x| shot.get_pixel(x * k, (620 - 22 - 40 - 3) * k).0[..3] == [0x0d, 0x4f, 0xfb]);
        assert!(ring.count() > 100, "Send draws the ring");
        let _ = app.update(UiMessage::OpenEditSpaces);
        let spaces = app.spaces_window.as_ref().expect("the Edit Spaces window").id;
        let order = ids(&survey_of(&app, spaces, rail::SPACES_SIZE, None));
        assert_eq!(order[..2], ["spaces-search", "handle-inbox"], "the search, then the handles: {order:?}");
        assert!(order.iter().any(|s| s.starts_with("gear-")), "the menus of the accounts: {order:?}");
    }
}

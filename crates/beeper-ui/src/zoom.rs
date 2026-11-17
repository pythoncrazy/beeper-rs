//! Desktop's carousel zoom (`react-zoom-pan-pinch` in `AttachmentCarousel.tsx`): scale 1 to 8, Ctrl + wheel (the
//! touchpad pinch of Chromium) zooms at the cursor, a double click zooms in or out, `+`, `=` and `-` zoom at the
//! middle, and a drag pans inside the bounds of the zoomed area. A click outside the content publishes `outside`.
use crate::{BeeperGuiApp, UiMessage};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Shell};
use iced::keyboard::{self, Key};
use iced::Task;
use iced::{window, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Transformation, Vector};
use std::time::Instant;

/// `minScale` and `maxScale`.
const MIN: f32 = 1.0;
const MAX: f32 = 8.0;
/// The `step` of `zoomIn`, `zoomOut` and `doubleClick`, and `wheel.smoothStep` of the carousel.
const KEY_STEP: f32 = 0.5;
const CLICK_STEP: f32 = 0.7;
const WHEEL_STEP: f32 = 0.02;
/// The `animationTime` of `zoomIn` and of `doubleClick`, in ms.
const KEY_MS: f32 = 300.0;
const CLICK_MS: f32 = 200.0;
/// The pixels of a wheel line, like Chromium.
const LINE: f32 = 100.0;

pub struct Zoom<'a, M> {
    content: Element<'a, M>,
    /// The item: a new item starts at scale 1.
    key: u64,
    outside: M,
}

/// The scale and the position of the top left corner of the scaled area.
type View = (f32, Vector);

#[derive(Default)]
struct State {
    key: u64,
    view: Option<View>,
    /// The view at the start of an animation, its target, its length in ms and its start.
    anim: Option<(View, View, f32, Instant)>,
    /// The cursor minus the position at the press, and if the cursor moved since.
    drag: Option<(Vector, bool)>,
    click: Option<mouse::Click>,
    cursor: Point,
    ctrl: bool,
}

impl State {
    fn view(&self) -> View {
        self.view.unwrap_or((MIN, Vector::ZERO))
    }
}

pub fn zoom<'a, M>(content: impl Into<Element<'a, M>>, key: u64, outside: M) -> Zoom<'a, M> {
    Zoom { content: content.into(), key, outside }
}

/// The library's `easeOut`.
fn ease(t: f32) -> f32 {
    0.5 - (t * std::f32::consts::PI).cos() / 2.0
}

/// `limitToBounds`: the scaled area always covers the bounds.
fn limit(s: f32, at: Vector, size: Size) -> Vector {
    Vector::new(at.x.clamp(size.width * (1.0 - s), 0.0), at.y.clamp(size.height * (1.0 - s), 0.0))
}

/// The view at scale `to` that keeps the point `p` of the bounds in place, with the scale rounded to 3 decimals.
fn zoom_at((s, at): View, to: f32, p: Point, size: Size) -> View {
    let to = ((to * 1000.0).round() / 1000.0).clamp(MIN, MAX);
    let p = Vector::new(p.x, p.y);
    (to, limit(to, p - (p - at) * (to / s), size))
}

impl<M: Clone> Widget<M, Theme, Renderer> for Zoom<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State { key: self.key, ..State::default() })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.key != self.key {
            *state = State { key: self.key, ..State::default() };
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let size = limits.max();
        let child = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &limits.loose());
        let at = Point::new((size.width - child.size().width) / 2.0, (size.height - child.size().height) / 2.0);
        layout::Node::with_children(size, vec![child.move_to(at)])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let size = bounds.size();
        let local = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        let animate = |state: &mut State, to: View, ms: f32| {
            state.anim = Some((state.view(), to, ms, Instant::now()));
        };
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some((from, to, ms, start)) = state.anim {
                    let t = (now.saturating_duration_since(start).as_secs_f32() * 1000.0 / ms).min(1.0);
                    let k = ease(t);
                    state.view = Some((from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k));
                    match t < 1.0 {
                        true => shell.request_redraw(),
                        false => state.anim = None,
                    }
                }
                return;
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Character(c), modifiers, .. })
                if modifiers.is_empty() || *modifiers == keyboard::Modifiers::SHIFT =>
            {
                let dir = match c.as_str() {
                    "+" | "=" => 1.0,
                    "-" => -1.0,
                    _ => return,
                };
                let (s, at) = state.view();
                let middle = Point::new(size.width / 2.0, size.height / 2.0);
                animate(state, zoom_at((s, at), s * (dir * KEY_STEP).exp(), middle, size), KEY_MS);
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                state.cursor = *position;
                if let Some((grab, _)) = state.drag {
                    let (s, _) = state.view();
                    let at = limit(s, Vector::new(position.x, position.y) - grab, size);
                    (state.view, state.anim, state.drag) = (Some((s, at)), None, Some((grab, true)));
                    shell.capture_event();
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.ctrl = m.control();
                return;
            }
            // `wheelDisabled`: only Ctrl + wheel zooms.
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if state.ctrl => {
                let Some(p) = cursor.position_over(bounds) else { return };
                let y = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                let (s, at) = state.view();
                (state.view, state.anim) =
                    (Some(zoom_at((s, at), s + y.signum() * WHEEL_STEP * y.abs(), local(p), size)), None);
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_over(bounds) else { return };
                let click = mouse::Click::new(p, mouse::Button::Left, state.click);
                state.click = Some(click);
                let (s, at) = state.view();
                if click.kind() == mouse::click::Kind::Double {
                    let dir = if s == MIN { 1.0 } else { -1.0 };
                    animate(state, zoom_at((s, at), s * (dir * CLICK_STEP).exp(), local(p), size), CLICK_MS);
                } else {
                    state.drag = Some((Vector::new(p.x, p.y) - at, false));
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let Some((_, moved)) = state.drag.take() else { return };
                let (s, at) = state.view();
                let content = layout.children().next().expect("the content").bounds();
                let shown = Rectangle::new(
                    Point::new(
                        bounds.x + at.x + (content.x - bounds.x) * s,
                        bounds.y + at.y + (content.y - bounds.y) * s,
                    ),
                    content.size() * s,
                );
                if !moved && !shown.contains(state.cursor) {
                    shell.publish(self.outside.clone());
                }
                shell.capture_event();
            }
            _ => return,
        }
        shell.request_redraw();
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        mouse::Interaction::None
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let (s, at) = tree.state.downcast_ref::<State>().view();
        let bounds = layout.bounds();
        let t = Transformation::translate(bounds.x + at.x, bounds.y + at.y)
            * Transformation::scale(s)
            * Transformation::translate(-bounds.x, -bounds.y);
        let content = layout.children().next().expect("the content");
        renderer.with_layer(bounds, |r| {
            r.with_transformation(t, |r| {
                let child = &tree.children[0];
                let shown = *viewport * t.inverse();
                self.content.as_widget().draw(child, r, theme, style, content, mouse::Cursor::Unavailable, &shown);
            });
        });
    }
}

impl<'a, M: Clone + 'a> From<Zoom<'a, M>> for Element<'a, M> {
    fn from(z: Zoom<'a, M>) -> Self {
        Element::new(z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_point_and_the_bounds() {
        let size = Size::new(100.0, 100.0);
        let (s, at) = zoom_at((1.0, Vector::ZERO), 2.0, Point::new(0.0, 0.0), size);
        assert_eq!((s, at), (2.0, Vector::ZERO), "a zoom at the corner keeps the corner");
        let (s, at) = zoom_at((1.0, Vector::ZERO), 2.0, Point::new(50.0, 50.0), size);
        assert_eq!((s, at), (2.0, Vector::new(-50.0, -50.0)), "a zoom at the middle keeps the middle");
        assert_eq!(zoom_at((2.0, at), 20.0, Point::ORIGIN, size).0, MAX, "the scale stops at 8");
        assert_eq!(zoom_at((2.0, at), 0.5, Point::ORIGIN, size), (MIN, Vector::ZERO), "scale 1 fills the bounds");
        assert_eq!(limit(2.0, Vector::new(30.0, -300.0), size), Vector::new(0.0, -100.0), "a pan stops at the edges");
        assert_eq!(zoom_at((1.0, Vector::ZERO), (CLICK_STEP).exp(), Point::ORIGIN, size).0, 2.014);
    }
}

#[derive(Clone)]
pub enum Msg {
    Step(i8),
    /// The zoom slider moved to a level from -4 to 4, in steps of 10%.
    Drag(f32),
    Release,
}

impl BeeperGuiApp {
    pub(crate) fn set_zoom(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Step(step) => self.patch_settings(|s| {
                s.zoom_percent = if step == 0 {
                    100
                } else {
                    s.zoom_percent.saturating_add_signed(10 * i16::from(step)).clamp(40, 300)
                };
            }),
            Msg::Drag(level) => self.zoom_drag = Some(level),
            Msg::Release => {
                let level = self.zoom_drag.take().expect("the zoom slider was dragged");
                self.patch_settings(|s| s.zoom_percent = (100.0 + 10.0 * level) as u16);
            }
        }
        self.settle()
    }
}

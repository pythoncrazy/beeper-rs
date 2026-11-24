use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget;
use iced::advanced::{mouse, renderer, Renderer as _};
use iced::{border, window};

/// Desktop's `Toggle-module__track`: 36 by 24 px with a 2 px border, whose 16 px knob slides 12 px and takes its color
/// in `.15s ease` when the toggle turns on or off.
pub struct Toggle {
    on: bool,
    /// The colors of the knob, off and on, and of the border.
    colors: [Color; 3],
}

const SLIDE: f32 = 0.15;

pub fn toggle<'a, Message>(on: bool, colors: [Color; 3]) -> Element<'a, Message> {
    Element::new(Toggle { on, colors })
}

/// Whether the toggle is on, and since when it slides.
struct State(bool, Option<Instant>);

impl State {
    /// Starts the slide when the toggle turned on or off since the last frame, and whether it slides at `now`.
    fn step(&mut self, on: bool, now: Instant) -> bool {
        if self.0 != on {
            *self = State(on, Some(now));
        }
        self.1.is_some_and(|at| now.saturating_duration_since(at).as_secs_f32() < SLIDE)
    }

    /// How far the knob is from off to on at `now`.
    fn at(&self, now: Instant) -> f32 {
        let t = self.1.map_or(1.0, |at| (now.saturating_duration_since(at).as_secs_f32() / SLIDE).min(1.0));
        let e = crate::cubic_bezier(t, [0.25, 0.1, 0.25, 1.0]);
        if self.0 {
            e
        } else {
            1.0 - e
        }
    }
}

impl<M> Widget<M, Theme, Renderer> for Toggle {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State(self.on, None))
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(36.0), Length::Fixed(24.0))
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, _: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(36.0, 24.0))
    }

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, _: Layout<'_>, _: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>, _: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return };
        if tree.state.downcast_mut::<State>().step(self.on, *now) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, _: &Theme, _: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let [off, on, edge] = self.colors;
        let b = layout.bounds();
        let k = tree.state.downcast_ref::<State>().at(Instant::now());
        let track = renderer::Quad { bounds: b, border: border::rounded(12).color(edge).width(2), ..Default::default() };
        renderer.fill_quad(track, CLEAR);
        let knob = Rectangle::new(Point::new(b.x + 4.0 + 12.0 * k, b.y + 4.0), Size::new(16.0, 16.0));
        renderer
            .fill_quad(renderer::Quad { bounds: knob, border: border::rounded(8), ..Default::default() }, crate::style::mix(off, on, k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob slides from off to on in `.15s ease`, 58 % of the way at 50 ms and 80 % at 75 ms, and back.
    #[test]
    fn the_knob_slides_like_desktop() {
        let start = Instant::now();
        let mut s = State(false, None);
        assert!(!s.step(false, start) && s.at(start) == 0.0, "an unchanged toggle stays put");
        assert!(s.step(true, start), "turning on starts the slide");
        let ms = |n| start + Duration::from_millis(n);
        assert!((s.at(ms(50)) - 0.576).abs() < 0.01 && (s.at(ms(75)) - 0.802).abs() < 0.01);
        assert!(!s.step(true, ms(150)) && s.at(ms(150)) == 1.0, "and ends after 150 ms");
        assert!(s.step(false, ms(200)) && (s.at(ms(275)) - 0.198).abs() < 0.01, "turning off slides back");
    }

    /// The track keeps Desktop's 2 px border, and the knob shows at the start when off and 12 px on when on.
    #[test]
    fn the_toggle_draws_like_desktop() {
        let colors = [Color::from_rgb8(255, 0, 0), Color::from_rgb8(0, 0, 255), Color::from_rgb8(0, 255, 0)];
        for (on, x) in [(false, 12), (true, 24)] {
            let view = container(toggle(on, colors)).padding(10).style(crate::style::fill_box(Color::WHITE, 0.0));
            let mut sim = iced_test::Simulator::with_size(Default::default(), Size::new(56.0, 44.0), view);
            let shot = crate::focus::tests::pixels(&mut sim, &iced::Theme::Light, "toggle");
            let k = shot.width() / 56;
            let px = |x: u32, y: u32| shot.get_pixel((10 + x) * k, (10 + y) * k).0;
            assert_eq!(px(x, 12), if on { [0, 0, 255, 255] } else { [255, 0, 0, 255] }, "the knob, on {on}");
            assert_eq!((px(18, 1), px(18, 3)), ([0, 255, 0, 255], [255; 4]), "the border, on {on}");
        }
    }
}

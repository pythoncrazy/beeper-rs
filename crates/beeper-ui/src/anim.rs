use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Renderer as _, Shell};
use iced::widget::{image, Image};
use iced::{window, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A frame of an animation and its delay in milliseconds.
pub type Frame = (image::Handle, u128);

/// Draws the frame of `frames` that is due since `start`, and asks for a redraw at the next frame while it is
/// visible. The view does not change, so the animation needs no view or layout. An animation that does not play
/// shows its first frame and asks for no redraws.
pub struct Gif<'a> {
    frames: Frames,
    play: bool,
    start: Instant,
    make: Box<dyn Fn(image::Handle) -> Image + 'a>,
    /// The turn in radians from the first angle to the second one in `TURN_MS` from the time.
    turn: (f32, f32, Instant),
}

/// The time of Desktop's `.2s transform` transition.
const TURN_MS: f32 = 200.0;

/// The frames of an image, shared with the cache that decoded them: a still image has one frame.
pub type Frames = Arc<[Frame]>;

/// `frames` as `make` draws them, playing from `start` when `play`. A still image, or an animation that does not
/// play, is the plain image of its first frame, so it adds no work to the frames of the window.
pub fn gif<'a, M: 'a>(
    frames: &Frames,
    play: bool,
    start: Instant,
    make: impl Fn(image::Handle) -> Image + 'a,
) -> Element<'a, M> {
    match plays(frames, play) {
        false => make(frames[0].0.clone()).into(),
        true => {
            let turn = (0.0, 0.0, start);
            Element::new(Gif { frames: frames.clone(), play, start, make: Box::new(make), turn })
        }
    }
}

/// `gif`, turned from the first angle to the second one in radians, from the time on, with the CSS `ease` curve.
/// The turned image fits in its bounds.
pub fn turning<'a, M: 'a>(
    frames: &Frames,
    play: bool,
    start: Instant,
    turn: (f32, f32, Instant),
    make: impl Fn(image::Handle) -> Image + 'a,
) -> Element<'a, M> {
    Element::new(Gif { frames: frames.clone(), play, start, make: Box::new(make), turn })
}

/// Whether an animation of `frames` moves when it is to `play`: it has more than one frame.
pub fn plays(frames: &[Frame], play: bool) -> bool {
    play && frames.len() > 1
}

/// The time of the frame after the one that is due at `now`, of an animation that plays from `start`.
pub fn next(frames: &[Frame], start: Instant, now: Instant) -> Instant {
    let (_, left) = frame(frames, now.saturating_duration_since(start).as_millis());
    now + Duration::from_millis(left as u64)
}

impl Gif<'_> {
    /// The image of frame `h` at the angle of the turn now, and if the turn goes on.
    fn image(&self, h: image::Handle) -> (Image, bool) {
        let (from, to, at) = self.turn;
        let t = (at.elapsed().as_secs_f32() * 1000.0 / TURN_MS).min(1.0);
        let angle = from + (to - from) * crate::cubic_bezier(t, [0.25, 0.1, 0.25, 1.0]);
        ((self.make)(h).rotation(iced::Rotation::Solid(iced::Radians(angle))), t < 1.0 && from != to)
    }
}

/// The frame at `ms` into the loop, and the time until the next frame.
pub fn frame(frames: &[Frame], ms: u128) -> (&image::Handle, u128) {
    let mut t = ms % frames.iter().map(|f| f.1).sum::<u128>();
    frames
        .iter()
        .find_map(|(h, d)| match t.checked_sub(*d) {
            Some(rest) => {
                t = rest;
                None
            }
            None => Some((h, d - t)),
        })
        .expect("the time is below the total delay")
}

impl<M> Widget<M, Theme, Renderer> for Gif<'_> {
    fn size(&self) -> Size<Length> {
        Widget::<M, Theme, Renderer>::size(&(self.make)(self.frames[0].0.clone()))
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        Widget::<M, Theme, Renderer>::layout(&mut (self.make)(self.frames[0].0.clone()), tree, renderer, limits)
    }

    fn update(
        &mut self,
        _: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return };
        if !layout.bounds().intersects(viewport) {
            return;
        }
        if self.image(self.frames[0].0.clone()).1 {
            shell.request_redraw();
        } else if plays(&self.frames, self.play) {
            shell.request_redraw_at(next(&self.frames, self.start, *now));
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        // A still image has one frame without a delay.
        let h = match plays(&self.frames, self.play) {
            false => &self.frames[0].0,
            true => frame(&self.frames, self.start.elapsed().as_millis()).0,
        };
        let (image, _) = self.image(h.clone());
        Widget::<M, Theme, Renderer>::draw(&image, tree, renderer, theme, style, layout, cursor, viewport);
    }
}

/// A round dot of `color` whose alpha is `alpha(t)`, with `t` the part of a 1 s loop that starts `delay` ms
/// after `start`. It redraws each frame while it is visible, without view or layout.
pub struct Dot {
    size: f32,
    color: Color,
    start: Instant,
    delay: u128,
    alpha: fn(f32) -> f32,
}

pub fn dot<'a, M: 'a>(size: f32, color: Color, start: Instant, delay: u128, alpha: fn(f32) -> f32) -> Element<'a, M> {
    Element::new(Dot { size, color, start, delay, alpha })
}

impl<M> Widget<M, Theme, Renderer> for Dot {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, _: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }

    fn update(
        &mut self,
        _: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) && layout.bounds().intersects(viewport) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _: &widget::Tree,
        renderer: &mut Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let t = ((self.start.elapsed().as_millis() + 1000 - self.delay) % 1000) as f32 / 1000.0;
        let quad = renderer::Quad {
            bounds: layout.bounds(),
            border: iced::border::rounded(self.size / 2.0),
            ..Default::default()
        };
        renderer.fill_quad(quad, self.color.scale_alpha((self.alpha)(t)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `.2s transform` turn with the CSS `ease` curve: it goes on for 200 ms, and only for a change.
    #[test]
    fn turns_like_desktop() {
        assert!((crate::cubic_bezier(0.5, [0.25, 0.1, 0.25, 1.0]) - 0.802).abs() < 5e-3, "the CSS ease curve");
        let frames: Frames = Arc::new([(image::Handle::from_rgba(1, 1, vec![0; 4]), 0)]);
        let start = Instant::now();
        let gif = |turn| Gif { frames: frames.clone(), play: true, start, make: Box::new(image), turn };
        let h = || frames[0].0.clone();
        assert!(gif((0.0, -1.0, Instant::now())).image(h()).1, "a new turn goes on");
        assert!(!gif((0.0, -1.0, start - Duration::from_millis(300))).image(h()).1, "a turn ends after 200 ms");
        assert!(!gif((-1.0, -1.0, Instant::now())).image(h()).1, "no turn without a change");
    }
}

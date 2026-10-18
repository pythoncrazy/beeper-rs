use crate::Frames;
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Shell};
use iced::widget::{image, Image};
use iced::{window, Element, Event, Length, Rectangle, Renderer, Size, Theme};
use std::time::{Duration, Instant};

/// Draws the frame of `frames` that is due since `start`, and asks for a redraw at the next frame while it is
/// visible. The view does not change, so the animation needs no view or layout.
pub struct Gif<'a> {
    frames: &'a Frames,
    start: Instant,
    make: Box<dyn Fn(image::Handle) -> Image + 'a>,
}

pub fn gif<'a, M: 'a>(
    frames: &'a Frames,
    start: Instant,
    make: impl Fn(image::Handle) -> Image + 'a,
) -> Element<'a, M> {
    match frames.len() {
        1 => make(frames[0].0.clone()).into(),
        _ => Element::new(Gif { frames, start, make: Box::new(make) }),
    }
}

/// The frame at `ms` into the loop, and the time until the next frame.
pub fn frame(frames: &Frames, ms: u128) -> (&image::Handle, u128) {
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
        if layout.bounds().intersects(viewport) {
            let (_, left) = frame(self.frames, now.duration_since(self.start).as_millis());
            shell.request_redraw_at(*now + Duration::from_millis(left as u64));
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
        let (h, _) = frame(self.frames, self.start.elapsed().as_millis());
        let image = (self.make)(h.clone());
        Widget::<M, Theme, Renderer>::draw(&image, tree, renderer, theme, style, layout, cursor, viewport);
    }
}

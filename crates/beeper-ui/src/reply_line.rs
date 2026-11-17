//! Desktop's `BrandReplyLineSegment`: the 3 px line of a reply chain, 15 px from the left of the timeline, which
//! starts with a curve, ends with one, loops, or goes straight.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, svg, Renderer as _};
use iced::{border, Color, Element, Length, Rectangle, Renderer, Size, Theme};
use std::sync::LazyLock;

/// Desktop's `So`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    Line,
    /// A curve in from the right at the top, then the line down.
    Start,
    /// The line down, then a curve out to the right.
    End,
    /// The line, a loop, and the line again.
    Loop,
}

/// The props of a segment: its shape, and its 2 px round ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    pub(crate) shape: Shape,
    pub(crate) round_top: bool,
    pub(crate) round_bottom: bool,
}

impl Segment {
    pub(crate) const fn new(shape: Shape, round_top: bool, round_bottom: bool) -> Self {
        Self { shape, round_top, round_bottom }
    }
}

/// Desktop's `I4t`, `k4t` and `E4t`: the start curve, the end curve and the loop, each `(width, height)`.
static SHAPES: LazyLock<[(svg::Handle, f32, f32); 3]> = LazyLock::new(|| {
    let shape = |w: f32, h: f32, view: &str, d: &str| {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{view}"><path fill="currentColor" fill-rule="evenodd" d="{d}"/></svg>"#
        );
        (svg::Handle::from_memory(svg.into_bytes()), w, h)
    };
    [
        shape(25.0, 19.0, "0.5 0 25 18", "M17 3.5C9.544 3.5 3.5 9.544 3.5 17v1.5h-3V17C.5 7.887 7.887.5 17 .5h6a1.5 1.5 0 0 1 0 3z"),
        shape(25.0, 19.0, "0.5 0.5 25 19", "M17 15.5C9.544 15.5 3.5 9.456 3.5 2V.5h-3V2c0 9.113 7.387 16.5 16.5 16.5h6a1.5 1.5 0 0 0 0-3z"),
        shape(17.0, 15.0, "0.5 0 17 15", "M3.66 11.689c-.106.772-.16 2.55-.16 3.311h-3c0-2.063.339-5.275 1.024-7.298C.804 5.555.501 2.177.5.001l3-.002c0 .862.055 2.74.166 3.6C5.07 1.81 7.02.5 9.542.5c2.203 0 3.97.862 5.175 2.22 1.18 1.33 1.76 3.062 1.782 4.751.023 1.693-.512 3.446-1.695 4.796-1.209 1.379-3.003 2.233-5.26 2.233-2.563 0-4.496-1.15-5.883-2.811m1.072-3.987q.15.346.317.67c1.033 1.99 2.487 3.128 4.495 3.128 1.45 0 2.401-.523 3.003-1.21.628-.717.967-1.719.953-2.778s-.38-2.072-1.027-2.8C11.851 4.01 10.907 3.5 9.542 3.5c-1.7 0-3.175 1.098-4.306 3.159q-.273.497-.504 1.043"),
    ]
});

/// The left of the line in the box of a message (`margin-left: 15px`).
const LEFT: f32 = 15.0;

/// `--reply-line-width`.
const WIDTH: f32 = 3.0;

/// A segment in the full box it is laid over, with `top` and `bottom` px of it left out.
pub(crate) struct ReplyLine {
    segment: Segment,
    color: Color,
    top: f32,
    bottom: f32,
}

pub(crate) fn reply_line<'a, M: 'a>(segment: Segment, color: Color, top: f32, bottom: f32) -> Element<'a, M> {
    Element::new(ReplyLine { segment, color, top, bottom })
}

impl ReplyLine {
    /// A straight part from `y` down `h` px, with the round ends it has.
    fn line(&self, renderer: &mut Renderer, x: f32, y: f32, h: f32, (top, bottom): (bool, bool)) {
        if h <= 0.0 {
            return;
        }
        let (t, b) = (if top { 2.0 } else { 0.0 }, if bottom { 2.0 } else { 0.0 });
        let quad = renderer::Quad {
            bounds: Rectangle { x, y, width: WIDTH, height: h },
            border: border::rounded(border::Radius { top_left: t, top_right: t, bottom_right: b, bottom_left: b }),
            ..Default::default()
        };
        renderer.fill_quad(quad, self.color);
    }

    fn shape(&self, renderer: &mut Renderer, k: usize, x: f32, y: f32, clip: Rectangle) {
        use iced::advanced::svg::Renderer as _;
        let (handle, w, h) = &SHAPES[k];
        let bounds = Rectangle { x, y, width: *w, height: *h };
        renderer.draw_svg(svg::Svg::new(handle.clone()).color(self.color), bounds, clip);
    }
}

impl<M> Widget<M, Theme, Renderer> for ReplyLine {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _: &widget::Tree,
        renderer: &mut Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let (x, y, h) = (b.x + LEFT, b.y + self.top, b.height - self.top - self.bottom);
        let Segment { shape, round_top, round_bottom } = self.segment;
        // `.start` and `.end` take `max(50%, 22px)` of the box, at its bottom or its top.
        let part = (h / 2.0).max(22.0);
        match shape {
            Shape::Line => self.line(renderer, x, y, h, (round_top, round_bottom)),
            Shape::Start => {
                let y = (y + h - part).max(y);
                self.shape(renderer, 0, x, y, *viewport);
                self.line(renderer, x, y + 19.0, part - 19.0, (false, round_bottom));
            }
            Shape::End => {
                self.line(renderer, x, y, part - 19.0, (round_top, false));
                self.shape(renderer, 1, x, y + part - 19.0, *viewport);
            }
            Shape::Loop => {
                let half = (h - 15.0) / 2.0;
                self.line(renderer, x, y, half, (round_top, false));
                self.shape(renderer, 2, x, y + half, *viewport);
                self.line(renderer, x, y + half + 15.0, half, (false, round_bottom));
            }
        }
    }
}

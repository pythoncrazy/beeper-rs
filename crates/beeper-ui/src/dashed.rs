use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Renderer as _};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

/// A 1px dashed border with rounded corners, like CSS `border: 1px dashed`. Put it over the content in a stack.
pub struct Dashes {
    color: Color,
    radius: f32,
}

pub fn dashes<'a, Message>(color: Color, radius: f32) -> Element<'a, Message> {
    Element::new(Dashes { color, radius })
}

/// The point at `s` along the rounded rectangle `b` with corner radius `r`, clockwise from the top left corner.
fn along(b: Rectangle, r: f32, mut s: f32) -> Point {
    let (w, h) = (b.width - 2.0 * r, b.height - 2.0 * r);
    let arc = std::f32::consts::FRAC_PI_2 * r;
    let corners = [
        (b.x + b.width - r, b.y + r),
        (b.x + b.width - r, b.y + b.height - r),
        (b.x + r, b.y + b.height - r),
        (b.x + r, b.y + r),
    ];
    let starts =
        [(b.x + r, b.y), (b.x + b.width, b.y + r), (b.x + b.width - r, b.y + b.height), (b.x, b.y + b.height - r)];
    let dirs = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    for i in 0..4 {
        let side = if i % 2 == 0 { w } else { h };
        if s < side {
            return Point::new(starts[i].0 + dirs[i].0 * s, starts[i].1 + dirs[i].1 * s);
        }
        s -= side;
        if s < arc {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * (i as f32 + s / arc);
            return Point::new(corners[i].0 + r * a.cos(), corners[i].1 + r * a.sin());
        }
        s -= arc;
    }
    Point::new(b.x + r, b.y)
}

impl<Message> Widget<Message, Theme, Renderer> for Dashes {
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
        _: &Rectangle,
    ) {
        let b = layout.bounds().shrink(0.5);
        let r = self.radius.min(b.width / 2.0).min(b.height / 2.0);
        let total = 2.0 * (b.width + b.height - 4.0 * r) + 2.0 * std::f32::consts::PI * r;
        // Chrome draws 3px dashes and 3px gaps, stretched so that the pattern fits.
        let period = total / (total / 6.0).round().max(1.0);
        let mut s = 0.0;
        while s < total {
            if s % period < period / 2.0 {
                let p = along(b, r, s);
                let bounds = Rectangle::new(Point::new(p.x - 0.5, p.y - 0.5), Size::new(1.0, 1.0));
                renderer.fill_quad(renderer::Quad { bounds, ..Default::default() }, self.color);
            }
            s += 0.5;
        }
    }
}

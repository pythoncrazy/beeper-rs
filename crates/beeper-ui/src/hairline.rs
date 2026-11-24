use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget::{self};
use iced::advanced::{mouse, renderer, Renderer as _};

/// A CSS rule `thick` px high at the bottom of its box, `left` px in, like Desktop's `section:after` divider of a
/// chat row (`--divider-height: 0.33px`). Chromium snaps it to whole device pixels, at least one: at a scale of 1 it
/// falls on the first pixel under the box, at 2 on the last one inside it. Put it over the row in a stack.
pub struct Hairline {
    color: Color,
    left: f32,
    thick: f32,
    scale: f32,
}

pub fn hairline<'a, Message>(color: Color, left: f32, thick: f32, scale: f32) -> Element<'a, Message> {
    Element::new(Hairline { color, left, thick, scale })
}

/// The top and height of the rule over `bottom`, in layout pixels: Chromium's `PixelSnappedIntRect` of the rule's
/// box in device pixels, whose height is a `LayoutUnit` (1/64 px).
fn snapped(bottom: f32, thick: f32, scale: f32) -> (f32, f32) {
    let h = (thick * scale * 64.0).floor() / 64.0;
    let top = bottom * scale - h;
    let y = top.round();
    (y / scale, ((top + h).round() - y).max(1.0) / scale)
}

impl<Message> Widget<Message, Theme, Renderer> for Hairline {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self, _: &widget::Tree, renderer: &mut Renderer, _: &Theme, _: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let b = layout.bounds();
        let (y, h) = snapped(b.y + b.height, self.thick, self.scale);
        let bounds = Rectangle::new(Point::new(b.x + self.left, y), Size::new(b.width - self.left, h));
        renderer.fill_quad(renderer::Quad { bounds, ..Default::default() }, self.color);
    }
}

#[cfg(test)]
mod tests {
    /// Desktop's 0.33 px rule under a row that ends at y = 104, measured in device pixels with
    /// `--force-device-scale-factor`: 104 at 1, 207 at 2, 156 at 1.5 and 181 at 1.75, one device pixel each.
    #[test]
    fn the_rule_snaps_like_chromium() {
        for (scale, top) in [(1.0, 104.0), (2.0, 207.0), (1.5, 156.0), (1.75, 181.0)] {
            assert_eq!(super::snapped(104.0, 0.33, scale), (top / scale, 1.0 / scale), "at {scale}");
        }
    }
}

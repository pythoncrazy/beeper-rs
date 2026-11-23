use iced::advanced::layout::Layout;
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Widget};
use iced::{border, Background, Color, Element, Rectangle, Renderer, Size, Theme, Vector};

/// CSS `box-shadow: 0 0 0 <width>px <color>`: a ring of `width` around `content`, outside its box like a spread
/// shadow, so the layout does not move. Desktop draws its focus rings this way, or as an `outline` with an
/// `outline-offset`, which leaves a gap between the box and the ring.
pub struct Ring<'a, Message> {
    content: Element<'a, Message>,
    color: Color,
    width: f32,
    radius: f32,
    offset: f32,
}

/// A ring of `width` and `color` around `content`, whose corners have `radius`.
pub fn ring<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    color: Color,
    width: f32,
    radius: f32,
) -> Ring<'a, Message> {
    Ring { content: content.into(), color, width, radius, offset: 0.0 }
}

impl<Message> Ring<'_, Message> {
    /// CSS `outline-offset`: the gap between the box and the ring, whose corners grow by it.
    pub fn offset(self, offset: f32) -> Self {
        Self { offset, ..self }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Ring<'_, Message> {
    crate::delegate::delegate!(
        children,
        diff,
        size,
        layout,
        operate,
        update(Message),
        mouse_interaction,
        overlay(Message)
    );

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
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        let (w, out) = (self.width, self.width + self.offset);
        let b = layout.bounds();
        let bounds =
            Rectangle::new(b.position() - Vector::new(out, out), Size::new(b.width + 2.0 * out, b.height + 2.0 * out));
        let border = border::rounded(self.radius + out).color(self.color).width(w);
        renderer.fill_quad(Quad { bounds, border, ..Quad::default() }, Background::Color(Color::TRANSPARENT));
    }
}

impl<'a, Message: 'a> From<Ring<'a, Message>> for Element<'a, Message> {
    fn from(r: Ring<'a, Message>) -> Self {
        Element::new(r)
    }
}

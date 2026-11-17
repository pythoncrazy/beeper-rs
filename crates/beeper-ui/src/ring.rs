use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::{border, Background, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

/// CSS `box-shadow: 0 0 0 <width>px <color>`: a ring of `width` around `content`, outside its box like a spread
/// shadow, so the layout does not move. Desktop draws its focus rings this way.
pub struct Ring<'a, Message> {
    content: Element<'a, Message>,
    color: Color,
    width: f32,
    radius: f32,
}

/// A ring of `width` and `color` around `content`, whose corners have `radius`.
pub fn ring<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    color: Color,
    width: f32,
    radius: f32,
) -> Ring<'a, Message> {
    Ring { content: content.into(), color, width, radius }
}

impl<Message> Widget<Message, Theme, Renderer> for Ring<'_, Message> {
    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let content = &mut tree.children[0];
        self.content.as_widget_mut().update(content, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        let w = self.width;
        let b = layout.bounds();
        let bounds = Rectangle::new(b.position() - Vector::new(w, w), Size::new(b.width + 2.0 * w, b.height + 2.0 * w));
        let border = border::rounded(self.radius + w).color(self.color).width(w);
        renderer.fill_quad(Quad { bounds, border, ..Quad::default() }, Background::Color(Color::TRANSPARENT));
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Ring<'a, Message>> for Element<'a, Message> {
    fn from(r: Ring<'a, Message>) -> Self {
        Element::new(r)
    }
}

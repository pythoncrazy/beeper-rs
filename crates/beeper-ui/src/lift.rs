use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Renderer as _, Shell};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector};

/// Draws `content` lowered by `dy` and scaled by `scale` from its bottom right corner, like Desktop's
/// `message-send-insert` keyframes. The layout does not change, except that a `tight` lift gives `content`
/// its full size as the minimum size.
pub struct Lift<'a, Message> {
    content: Element<'a, Message>,
    dy: f32,
    scale: f32,
    tight: bool,
}

pub fn lift<'a, Message>(content: impl Into<Element<'a, Message>>, dy: f32, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dy, scale, tight: false }
}

/// Gives a scrollable its viewport as the minimum size of its content, like a CSS `min-height: 100%`.
pub fn tight<'a, Message>(content: impl Into<Element<'a, Message>>) -> Lift<'a, Message> {
    Lift { content: content.into(), dy: 0.0, scale: 1.0, tight: true }
}

impl<Message> Widget<Message, Theme, Renderer> for Lift<'_, Message> {
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
        let tight = layout::Limits::new(limits.max(), limits.max());
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, if self.tight { &tight } else { limits })
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
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
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
        let b = layout.bounds();
        let (x, y) = (b.x + b.width, b.y + b.height);
        let t = Transformation::translate(x, y + self.dy)
            * Transformation::scale(self.scale)
            * Transformation::translate(-x, -y);
        renderer.with_transformation(t, |r| {
            self.content.as_widget().draw(&tree.children[0], r, theme, style, layout, cursor, viewport);
        });
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

impl<'a, Message: 'a> From<Lift<'a, Message>> for Element<'a, Message> {
    fn from(l: Lift<'a, Message>) -> Self {
        Element::new(l)
    }
}

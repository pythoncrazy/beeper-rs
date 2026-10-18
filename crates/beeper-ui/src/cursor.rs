use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

/// Shows the arrow cursor instead of the pointer over `content` and its overlays, like Desktop
/// when "Use pointer cursor for interactive elements" is off.
pub struct Arrow<'a, Message>(Element<'a, Message>);

pub fn arrow<'a, Message>(content: impl Into<Element<'a, Message>>) -> Arrow<'a, Message> {
    Arrow(content.into())
}

fn unpoint(i: mouse::Interaction) -> mouse::Interaction {
    match i {
        mouse::Interaction::Pointer => mouse::Interaction::Idle,
        i => i,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Arrow<'_, Message> {
    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.0)]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.0));
    }

    fn size(&self) -> Size<Length> {
        self.0.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.0.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.0.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
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
        self.0.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        unpoint(self.0.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer))
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
        self.0.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let inner = self.0.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation);
        inner.map(|o| overlay::Element::new(Box::new(ArrowOverlay(o))))
    }
}

struct ArrowOverlay<'a, Message>(overlay::Element<'a, Message, Theme, Renderer>);

impl<Message> overlay::Overlay<Message, Theme, Renderer> for ArrowOverlay<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.0.as_overlay_mut().layout(renderer, bounds)
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.0.as_overlay_mut().operate(layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.0.as_overlay_mut().update(event, layout, cursor, renderer, clipboard, shell);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        unpoint(self.0.as_overlay().mouse_interaction(layout, cursor, renderer))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.0.as_overlay().draw(renderer, theme, style, layout, cursor);
    }

    fn overlay<'b>(
        &'b mut self,
        layout: Layout<'b>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let inner = self.0.as_overlay_mut().overlay(layout, renderer);
        inner.map(|o| overlay::Element::new(Box::new(ArrowOverlay(o))))
    }

    fn index(&self) -> f32 {
        self.0.as_overlay().index()
    }
}

impl<'a, Message: 'a> From<Arrow<'a, Message>> for Element<'a, Message> {
    fn from(a: Arrow<'a, Message>) -> Self {
        Element::new(a)
    }
}

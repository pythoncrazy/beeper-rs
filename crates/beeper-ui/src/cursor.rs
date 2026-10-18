use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use std::cell::Cell;

/// The root of a window. It keeps the cursor position in `at` without a message, so a mouse move does not
/// rebuild the view. With `arrow`, it shows the arrow cursor instead of the pointer over `content` and its
/// overlays, like Desktop when "Use pointer cursor for interactive elements" is off.
pub struct Root<'a, Message>(Element<'a, Message>, bool, &'a Cell<Point>);

pub fn root<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    arrow: bool,
    at: &'a Cell<Point>,
) -> Root<'a, Message> {
    Root(content.into(), arrow, at)
}

fn unpoint(arrow: bool, i: mouse::Interaction) -> mouse::Interaction {
    match i {
        mouse::Interaction::Pointer if arrow => mouse::Interaction::Idle,
        i => i,
    }
}

fn record(at: &Cell<Point>, event: &Event) {
    if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
        at.set(*position);
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Root<'_, Message> {
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
        record(self.2, event);
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
        unpoint(self.1, self.0.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer))
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
        inner.map(|o| overlay::Element::new(Box::new(RootOverlay(o, self.1, self.2))))
    }
}

struct RootOverlay<'a, Message>(overlay::Element<'a, Message, Theme, Renderer>, bool, &'a Cell<Point>);

impl<Message> overlay::Overlay<Message, Theme, Renderer> for RootOverlay<'_, Message> {
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
        record(self.2, event);
        self.0.as_overlay_mut().update(event, layout, cursor, renderer, clipboard, shell);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        unpoint(self.1, self.0.as_overlay().mouse_interaction(layout, cursor, renderer))
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
        inner.map(|o| overlay::Element::new(Box::new(RootOverlay(o, self.1, self.2))))
    }

    fn index(&self) -> f32 {
        self.0.as_overlay().index()
    }
}

impl<'a, Message: 'a> From<Root<'a, Message>> for Element<'a, Message> {
    fn from(a: Root<'a, Message>) -> Self {
        Element::new(a)
    }
}

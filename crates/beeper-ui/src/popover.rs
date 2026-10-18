use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

/// Shows `popup` with its top-left corner at the cursor while the cursor is over `content`,
/// or at the top-left corner of `content` while `open` is set, like Desktop's "right-start" popover.
pub struct Popover<'a, Message> {
    content: Element<'a, Message>,
    popup: Element<'a, Message>,
    open: bool,
}

pub fn popover<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    popup: impl Into<Element<'a, Message>>,
    open: bool,
) -> Popover<'a, Message> {
    Popover { content: content.into(), popup: popup.into(), open }
}

#[derive(Default, Clone, Copy)]
struct Hover(Option<Point>);

impl<Message> Widget<Message, Theme, Renderer> for Popover<'_, Message> {
    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.content), widget::Tree::new(&self.popup)]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(&[self.content.as_widget(), self.popup.as_widget()]);
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Hover::default())
    }

    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Hover>()
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
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
        if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            let hover = tree.state.downcast_mut::<Hover>();
            let over = cursor.position_over(layout.bounds());
            if hover.0.is_some() != over.is_some() {
                hover.0 = over;
                shell.invalidate_layout();
                shell.request_redraw();
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
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
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let hover = *tree.state.downcast_ref::<Hover>();
        let [content, popup] = tree.children.as_mut_slice() else { unreachable!("a popover has two children") };
        let inner = self.content.as_widget_mut().overlay(content, layout, renderer, viewport, translation);
        let at = hover.0.or(self.open.then(|| layout.position())).map(|p| p + translation);
        let popup = at.map(|at| overlay::Element::new(Box::new(Popup { popup: &mut self.popup, tree: popup, at })));
        let all: Vec<_> = inner.into_iter().chain(popup).collect();
        (!all.is_empty()).then(|| overlay::Group::with_children(all).overlay())
    }
}

struct Popup<'a, 'b, Message> {
    popup: &'b mut Element<'a, Message>,
    tree: &'b mut widget::Tree,
    at: Point,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Popup<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        const MARGIN: f32 = 8.0;
        let node = self.popup.as_widget_mut().layout(self.tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        let size = node.size();
        let x = self.at.x.min(bounds.width - size.width - MARGIN).max(MARGIN);
        let y = self.at.y.min(bounds.height - size.height - MARGIN).max(MARGIN);
        node.move_to(Point::new(x, y))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let viewport = Rectangle::with_size(Size::INFINITE);
        self.popup.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &viewport);
    }
}

impl<'a, Message: 'a> From<Popover<'a, Message>> for Element<'a, Message> {
    fn from(p: Popover<'a, Message>) -> Self {
        Element::new(p)
    }
}

/// Shows `popup` over the window with its bottom center at `anchor(content size)` from the top-left of `content`,
/// like Desktop's "top" popover.
pub struct Above<'a, Message> {
    content: Element<'a, Message>,
    popup: Option<Element<'a, Message>>,
    anchor: Box<dyn Fn(Size) -> Vector + 'a>,
}

pub fn above<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    popup: Option<Element<'a, Message>>,
    anchor: impl Fn(Size) -> Vector + 'a,
) -> Above<'a, Message> {
    Above { content: content.into(), popup, anchor: Box::new(anchor) }
}

impl<Message> Widget<Message, Theme, Renderer> for Above<'_, Message> {
    fn children(&self) -> Vec<widget::Tree> {
        std::iter::once(&self.content).chain(&self.popup).map(widget::Tree::new).collect()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        let all: Vec<_> = std::iter::once(&self.content).chain(&self.popup).map(|e| e.as_widget()).collect();
        tree.diff_children(&all);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
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
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
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

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let (content, rest) = tree.children.split_first_mut().expect("the content has a tree");
        let inner = self.content.as_widget_mut().overlay(content, layout, renderer, viewport, translation);
        let at = layout.position() + (self.anchor)(layout.bounds().size()) + translation;
        let popup = self.popup.as_mut().map(|popup| {
            let tree = rest.first_mut().expect("the popup has a tree");
            overlay::Element::new(Box::new(Bubble { popup, tree, at }))
        });
        let all: Vec<_> = inner.into_iter().chain(popup).collect();
        (!all.is_empty()).then(|| overlay::Group::with_children(all).overlay())
    }
}

struct Bubble<'a, 'b, Message> {
    popup: &'b mut Element<'a, Message>,
    tree: &'b mut widget::Tree,
    at: Point,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Bubble<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.popup.as_widget_mut().layout(self.tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        let size = node.size();
        node.move_to(Point::new(
            (self.at.x - size.width / 2.0).clamp(0.0, bounds.width - size.width),
            self.at.y - size.height,
        ))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.popup.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &layout.bounds());
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.popup.as_widget_mut().operate(self.tree, layout, renderer, operation);
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
        let viewport = layout.bounds();
        self.popup.as_widget_mut().update(self.tree, event, layout, cursor, renderer, clipboard, shell, &viewport);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.popup.as_widget().mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.popup.as_widget_mut().overlay(self.tree, layout, renderer, &layout.bounds(), Vector::ZERO)
    }
}

impl<'a, Message: 'a> From<Above<'a, Message>> for Element<'a, Message> {
    fn from(a: Above<'a, Message>) -> Self {
        Element::new(a)
    }
}

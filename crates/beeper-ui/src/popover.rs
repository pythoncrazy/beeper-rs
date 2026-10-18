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

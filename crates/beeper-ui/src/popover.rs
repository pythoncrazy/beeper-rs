use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget::{self};
use iced::advanced::Renderer as _;
use iced::advanced::{mouse, overlay, renderer};

/// Shows `popup` with its top-left corner at the cursor while the cursor is over `content`,
/// or at the top-left corner of `content` while `open` is set, like Desktop's "right-start" popover.
pub struct Popover<'a, Message> {
    content: Element<'a, Message>,
    popup: Element<'a, Message>,
    open: bool,
}

pub fn popover<'a, Message>(
    content: impl Into<Element<'a, Message>>, popup: impl Into<Element<'a, Message>>, open: bool,
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

    crate::delegate::delegate!(state(Hover), size, layout, mouse_interaction, draw);

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle,
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
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn overlay<'b>(
        &'b mut self, tree: &'b mut widget::Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector,
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

    fn draw(&self, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor) {
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
/// like Desktop's "top" popover, or as a dropdown `below` the first child of `content`.
pub struct Above<'a, Message> {
    content: Element<'a, Message>,
    popup: Option<Element<'a, Message>>,
    anchor: Box<dyn Fn(Size) -> Vector + 'a>,
    squiggles: Box<dyn Fn(Size) -> Vec<(Point, f32)> + 'a>,
    on_right: Option<Box<dyn Fn(Size, Point) -> Message + 'a>>,
    blur: Option<Message>,
}

pub fn above<'a, Message>(
    content: impl Into<Element<'a, Message>>, popup: Option<Element<'a, Message>>, anchor: impl Fn(Size) -> Vector + 'a,
) -> Above<'a, Message> {
    Above { content: content.into(), popup, anchor: Box::new(anchor), squiggles: Box::new(|_| Vec::new()), on_right: None, blur: None }
}

impl<'a, Message> Above<'a, Message> {
    /// Chromium's red wavy spelling underlines, as (left end, width) from the top-left of `content`.
    pub fn squiggles(mut self, at: impl Fn(Size) -> Vec<(Point, f32)> + 'a) -> Self {
        self.squiggles = Box::new(at);
        self
    }

    /// The message of a right click, from the size of `content` and the cursor from its top-left.
    pub fn on_right(mut self, f: impl Fn(Size, Point) -> Message + 'a) -> Self {
        self.on_right = Some(Box::new(f));
        self
    }

    /// A dropdown: the popup's top-left corner at `anchor` from the first child of `content`, and a press outside the
    /// popup and that child publishes `blur` and goes on to what is under it, like a blur.
    pub fn below(mut self, blur: Message) -> Self {
        self.blur = Some(blur);
        self
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for Above<'_, Message> {
    fn children(&self) -> Vec<widget::Tree> {
        std::iter::once(&self.content).chain(&self.popup).map(widget::Tree::new).collect()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        let all: Vec<_> = std::iter::once(&self.content).chain(&self.popup).map(|e| e.as_widget()).collect();
        tree.diff_children(&all);
    }

    crate::delegate::delegate!(size, layout, mouse_interaction, operate);

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        if let (Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)), Some(f), Some(at)) =
            (event, &self.on_right, cursor.position_in(b))
        {
            shell.publish(f(b.size(), at));
        }
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        let b = layout.bounds();
        for (at, width) in (self.squiggles)(b.size()) {
            for x in 0..width as u16 {
                let dot = Point::new(b.x + at.x + f32::from(x), b.y + at.y + [0.0, 1.0, 2.0, 1.0][usize::from(x % 4)]);
                let quad = renderer::Quad { bounds: Rectangle::new(dot, Size::new(1.0, 1.0)), ..Default::default() };
                renderer.fill_quad(quad, Color::from_rgb8(255, 0, 0));
            }
        }
    }

    fn overlay<'b>(
        &'b mut self, tree: &'b mut widget::Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let (content, rest) = tree.children.split_first_mut().expect("the content has a tree");
        let inner = self.content.as_widget_mut().overlay(content, layout, renderer, viewport, translation);
        let first = layout.children().next().map_or(layout.bounds(), |l| l.bounds()) + translation;
        let blur = self.blur.clone().map(|m| (first, m));
        let from = if blur.is_some() { first.position() } else { layout.position() + translation };
        let at = from + (self.anchor)(layout.bounds().size());
        let popup = self.popup.as_mut().map(|popup| {
            let tree = rest.first_mut().expect("the popup has a tree");
            overlay::Element::new(Box::new(Bubble { popup, tree, at, below: blur.is_some(), blur }))
        });
        let all: Vec<_> = inner.into_iter().chain(popup).collect();
        (!all.is_empty()).then(|| overlay::Group::with_children(all).overlay())
    }
}

/// A popup over the window: its bottom center at `at`, or its top-left corner `below` it. With `blur`, a press
/// outside the popup and the rectangle publishes the message and goes on to what is under it, like a blur.
struct Bubble<'a, 'b, Message> {
    popup: &'b mut Element<'a, Message>,
    tree: &'b mut widget::Tree,
    at: Point,
    below: bool,
    blur: Option<(Rectangle, Message)>,
}

impl<Message: Clone> overlay::Overlay<Message, Theme, Renderer> for Bubble<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.popup.as_widget_mut().layout(self.tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        let size = node.size();
        match self.below {
            true => node.move_to(self.at),
            false => {
                node.move_to(Point::new((self.at.x - size.width / 2.0).clamp(0.0, bounds.width - size.width), self.at.y - size.height))
            }
        }
    }

    fn draw(&self, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor) {
        self.popup.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &layout.bounds());
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.popup.as_widget_mut().operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if let (Event::Mouse(mouse::Event::ButtonPressed(_)), Some((keep, blur))) = (event, &self.blur) {
            if !cursor.is_over(layout.bounds()) && !cursor.is_over(*keep) {
                shell.publish(blur.clone());
            }
        }
        let viewport = layout.bounds();
        self.popup.as_widget_mut().update(self.tree, event, layout, cursor, renderer, clipboard, shell, &viewport);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.popup.as_widget().mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }

    fn overlay<'c>(&'c mut self, layout: Layout<'c>, renderer: &Renderer) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.popup.as_widget_mut().overlay(self.tree, layout, renderer, &layout.bounds(), Vector::ZERO)
    }
}

impl<'a, Message: Clone + 'a> From<Above<'a, Message>> for Element<'a, Message> {
    fn from(a: Above<'a, Message>) -> Self {
        Element::new(a)
    }
}

/// Desktop's `BrandTooltip` with its `position-try: flip-block` (`flip-inline` beside the trigger): `tooltip`, which
/// iced puts on the side `at` and keeps inside the window, goes to the other side of the trigger when its margin box
/// does not fit on its own side and fits there.
pub struct Tip<'a, Message> {
    tooltip: iced::widget::Tooltip<'a, Message>,
    at: iced::widget::tooltip::Position,
}

pub fn tip<'a, Message>(tooltip: iced::widget::Tooltip<'a, Message>, at: iced::widget::tooltip::Position) -> Tip<'a, Message> {
    Tip { tooltip, at }
}

impl<Message> Tip<'_, Message> {
    /// Desktop's `delayTime`.
    pub fn delay(self, delay: std::time::Duration) -> Self {
        Self { tooltip: self.tooltip.delay(delay), ..self }
    }
}

impl<'a, Message: 'a> From<Tip<'a, Message>> for Element<'a, Message> {
    fn from(t: Tip<'a, Message>) -> Self {
        Element::new(Flip { content: t.tooltip.into(), at: t.at })
    }
}

struct Flip<'a, Message> {
    content: Element<'a, Message>,
    at: iced::widget::tooltip::Position,
}

impl<Message> Widget<Message, Theme, Renderer> for Flip<'_, Message> {
    crate::delegate::delegate!(children, diff, size, layout, operate, update(Message), mouse_interaction, draw);

    fn overlay<'b>(
        &'b mut self, tree: &'b mut widget::Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let trigger = layout.bounds() + translation;
        let inner = self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)?;
        Some(overlay::Element::new(Box::new(Flipped { inner, trigger, at: self.at })))
    }
}

/// The overlays of a tooltip, whose last one is its card.
struct Flipped<'b, Message> {
    inner: overlay::Element<'b, Message, Theme, Renderer>,
    trigger: Rectangle,
    at: iced::widget::tooltip::Position,
}

impl<Message> Flipped<'_, Message> {
    /// Where the card `card` goes in a window of `size`: on the other side of the trigger when it does not fit on its
    /// own and fits there, else where iced put it.
    fn place(&self, card: Rectangle, size: Size) -> Point {
        use iced::widget::tooltip::Position::*;
        let t = self.trigger;
        let (above, below) = (t.y - card.height, t.y + t.height);
        let (before, after) = (t.x - card.width, t.x + t.width);
        let fits_y = |y: f32| y >= 0.0 && y + card.height <= size.height;
        let fits_x = |x: f32| x >= 0.0 && x + card.width <= size.width;
        match self.at {
            Top if !fits_y(above) && fits_y(below) => Point::new(card.x, below),
            Bottom if !fits_y(below) && fits_y(above) => Point::new(card.x, above),
            Left if !fits_x(before) && fits_x(after) => Point::new(after, card.y),
            Right if !fits_x(after) && fits_x(before) => Point::new(before, card.y),
            _ => card.position(),
        }
    }
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Flipped<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.inner.as_overlay_mut().layout(renderer, bounds);
        let mut children = node.children().to_vec();
        if let Some(card) = children.last_mut() {
            let at = self.place(card.bounds(), bounds);
            card.move_to_mut(at);
        }
        layout::Node::with_children(node.size(), children).move_to(node.bounds().position())
    }

    fn draw(&self, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor) {
        self.inner.as_overlay().draw(renderer, theme, style, layout, cursor);
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.inner.as_overlay_mut().operate(layout, renderer, operation);
    }

    fn update(
        &mut self, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.inner.as_overlay_mut().update(event, layout, cursor, renderer, clipboard, shell);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.inner.as_overlay().mouse_interaction(layout, cursor, renderer)
    }

    fn overlay<'c>(&'c mut self, layout: Layout<'c>, renderer: &Renderer) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.inner.as_overlay_mut().overlay(layout, renderer)
    }

    fn index(&self) -> f32 {
        self.inner.as_overlay().index()
    }
}

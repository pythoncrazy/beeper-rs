use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Renderer as _, Shell};
use iced::{Element, Event, Rectangle, Renderer, Theme, Transformation};

/// Draws `content` lowered by `dy` and scaled by `scale` from its bottom right corner, like Desktop's
/// `message-send-insert` keyframes. The layout does not change, except that a `tight` lift gives `content`
/// its full size as the minimum size. A `press` lift draws `content` at `scale` from its center while the left
/// button that went down on it stays down, like a CSS `:active` transform.
pub struct Lift<'a, Message> {
    content: Element<'a, Message>,
    dx: f32,
    dy: f32,
    scale: f32,
    tight: bool,
    press: bool,
}

pub fn lift<'a, Message>(content: impl Into<Element<'a, Message>>, dy: f32, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy, scale, tight: false, press: false }
}

/// Gives a scrollable its viewport as the minimum size of its content, like a CSS `min-height: 100%`.
pub fn tight<'a, Message>(content: impl Into<Element<'a, Message>>) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale: 1.0, tight: true, press: false }
}

/// Draws `content` at `scale` while it is pressed, like Desktop's `:active { transform: scale(...) }`.
pub fn press<'a, Message>(content: impl Into<Element<'a, Message>>, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale, tight: false, press: true }
}

impl<Message> Lift<'_, Message> {
    /// Draws the content moved by `dx` and `dy` too, like a CSS `translate`.
    pub fn shift(self, dx: f32, dy: f32) -> Self {
        Lift { dx, dy: self.dy + dy, ..self }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Lift<'_, Message> {
    /// Whether a `press` lift is pressed.
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<bool>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(false)
    }

    crate::delegate::delegate!(children, diff, size, operate, mouse_interaction, overlay(Message));

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let tight = layout::Limits::new(limits.max(), limits.max());
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, if self.tight { &tight } else { limits })
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
        if self.press {
            let pressed = tree.state.downcast_mut::<bool>();
            let now = match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => cursor.is_over(layout.bounds()),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => false,
                _ => *pressed,
            };
            if now != *pressed {
                *pressed = now;
                shell.request_redraw();
            }
        }
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
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
        let (x, y, scale) = match self.press {
            true => (b.center_x(), b.center_y(), if *tree.state.downcast_ref::<bool>() { self.scale } else { 1.0 }),
            false => (b.x + b.width, b.y + b.height, self.scale),
        };
        let t = Transformation::translate(x + self.dx, y + self.dy)
            * Transformation::scale(scale)
            * Transformation::translate(-x, -y);
        renderer.with_transformation(t, |r| {
            self.content.as_widget().draw(&tree.children[0], r, theme, style, layout, cursor, viewport);
        });
    }
}

impl<'a, Message: 'a> From<Lift<'a, Message>> for Element<'a, Message> {
    fn from(l: Lift<'a, Message>) -> Self {
        Element::new(l)
    }
}

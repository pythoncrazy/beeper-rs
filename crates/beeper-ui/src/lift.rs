use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget::{self};
use iced::advanced::{mouse, renderer, Renderer as _};

/// Draws `content` lowered by `dy` and scaled by `scale` from its bottom right corner, like Desktop's
/// `message-send-insert` keyframes. The layout does not change, except that a `tight` lift gives `content` its full
/// size as the minimum size, and a `spill` lift lets it grow taller than its room. A `press` lift draws `content` at
/// `scale` from its center while the left button that went down on it stays down, like a CSS `:active` transform, and a
/// `shrink` lift from its center.
pub struct Lift<'a, Message> {
    content: Element<'a, Message>,
    dx: f32,
    dy: f32,
    scale: f32,
    /// The limits of `content` from the most room, instead of the given limits.
    fit: Option<fn(Size) -> layout::Limits>,
    origin: Origin,
}

/// Where a lift scales `content` from.
#[derive(Clone, Copy, PartialEq)]
enum Origin {
    /// The bottom right corner.
    Corner,
    /// The center, while the left button that went down on it stays down.
    Press,
    /// The center.
    Center,
}

pub fn lift<'a, Message>(content: impl Into<Element<'a, Message>>, dy: f32, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy, scale, fit: None, origin: Origin::Corner }
}

/// Gives a scrollable its viewport as the minimum size of its content, like a CSS `min-height: 100%`.
pub fn tight<'a, Message>(content: impl Into<Element<'a, Message>>) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale: 1.0, fit: Some(|max| layout::Limits::new(max, max)), origin: Origin::Corner }
}

/// Lets `content` be taller than its room, so that it draws over what is under it, like an absolutely positioned
/// CSS box that overflows its parent.
pub fn spill<'a, Message>(content: impl Into<Element<'a, Message>>) -> Lift<'a, Message> {
    let fit = |max: Size| layout::Limits::new(Size::ZERO, Size::new(max.width, f32::INFINITY));
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale: 1.0, fit: Some(fit), origin: Origin::Corner }
}

/// Draws `content` at `scale` while it is pressed, like Desktop's `:active { transform: scale(...) }`.
pub fn press<'a, Message>(content: impl Into<Element<'a, Message>>, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale, fit: None, origin: Origin::Press }
}

/// Draws `content` at `scale` from its center, like a CSS `transform: scale(...)`.
pub fn shrink<'a, Message>(content: impl Into<Element<'a, Message>>, scale: f32) -> Lift<'a, Message> {
    Lift { content: content.into(), dx: 0.0, dy: 0.0, scale, fit: None, origin: Origin::Center }
}

impl<Message> Lift<'_, Message> {
    /// Draws the content moved by `dx` and `dy` too, like a CSS `translate`.
    pub fn shift(self, dx: f32, dy: f32) -> Self {
        Lift { dx, dy: self.dy + dy, ..self }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Lift<'_, Message> {
    // The state: whether a `press` lift is pressed.
    crate::delegate::delegate!(state(bool), children, diff, size, operate, mouse_interaction, overlay(Message));

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let limits = self.fit.map_or(*limits, |fit| fit(limits.max()));
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &limits)
    }

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle,
    ) {
        if self.origin == Origin::Press {
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
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let (x, y, scale) = match self.origin {
            Origin::Corner => (b.x + b.width, b.y + b.height, self.scale),
            Origin::Press if !*tree.state.downcast_ref::<bool>() => (b.center_x(), b.center_y(), 1.0),
            Origin::Press | Origin::Center => (b.center_x(), b.center_y(), self.scale),
        };
        let t = Transformation::translate(x + self.dx, y + self.dy) * Transformation::scale(scale) * Transformation::translate(-x, -y);
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

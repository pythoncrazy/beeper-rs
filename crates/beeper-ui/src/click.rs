use iced::advanced::layout::Layout;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, Clipboard, Shell};
use iced::{Element, Event, Rectangle, Renderer, Theme};

/// A DOM `click` on `content`: `message` when the left button goes down and up over it, unless something inside it
/// (a link, a button, a card) took the press. Desktop selects a message on `click`, so pressing a link card in a
/// message or dragging a text selection out of it does not select it.
pub struct Click<'a, Message> {
    content: Element<'a, Message>,
    message: Message,
}

pub fn click<'a, Message>(content: impl Into<Element<'a, Message>>, message: Message) -> Click<'a, Message> {
    Click { content: content.into(), message }
}

/// Whether the left button went down on the content itself and is still down.
#[derive(Default)]
struct Pressed(bool);

impl<Message: Clone> Widget<Message, Theme, Renderer> for Click<'_, Message> {
    crate::delegate::delegate!(state(Pressed), children, diff, size, layout, operate, mouse_interaction, draw, overlay(Message));

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle,
    ) {
        let content = &mut tree.children[0];
        self.content.as_widget_mut().update(content, event, layout, cursor, renderer, clipboard, shell, viewport);
        let over = cursor.is_over(layout.bounds());
        let pressed = &mut tree.state.downcast_mut::<Pressed>().0;
        let captured = shell.is_event_captured();
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            *pressed = over && !captured;
        } else if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event {
            // The press ends here either way.
            if std::mem::take(pressed) && over && !captured {
                shell.publish(self.message.clone());
            }
        }
    }
}

impl<'a, Message: Clone + 'a> From<Click<'a, Message>> for Element<'a, Message> {
    fn from(c: Click<'a, Message>) -> Self {
        Element::new(c)
    }
}

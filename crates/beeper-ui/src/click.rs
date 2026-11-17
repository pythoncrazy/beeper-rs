use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

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
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Pressed>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Pressed::default())
    }

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
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: Clone + 'a> From<Click<'a, Message>> for Element<'a, Message> {
    fn from(c: Click<'a, Message>) -> Self {
        Element::new(c)
    }
}

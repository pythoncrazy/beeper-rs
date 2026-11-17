//! The avatars and media that a view shows but that no task loads yet. A view asks for them, the next frame
//! publishes a message, and `update` starts the loads. So a load costs work for the shown rows only, and an update
//! does not scan every chat and every message.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{window, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};
use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Wants {
    pub avatars: RefCell<BTreeSet<String>>,
    pub media: RefCell<BTreeSet<String>>,
    /// The tiles to bake, as (media URL, width, height, look).
    pub tiles: RefCell<BTreeSet<(String, u32, u32, crate::gallery::Look)>>,
    /// The quoted messages that the timeline does not have, as (chat, message).
    pub quotes: RefCell<BTreeSet<(String, String)>>,
    /// The shown voice notes to transcribe automatically.
    pub transcribe: RefCell<BTreeSet<String>>,
    /// The URLs of the shown contact cards (`.vcf` attachments) to read.
    pub vcards: RefCell<BTreeSet<String>>,
    /// The URLs of the PDFs that the carousel shows.
    pub pdfs: RefCell<BTreeSet<String>>,
    /// A view asked for a URL after the last frame.
    fresh: Cell<bool>,
}

impl Wants {
    pub fn ask<K: Ord + ToOwned + ?Sized>(&self, set: &RefCell<BTreeSet<K::Owned>>, key: &K)
    where
        K::Owned: Ord + Borrow<K>,
    {
        if !set.borrow().contains(key) {
            set.borrow_mut().insert(key.to_owned());
            self.fresh.set(true);
        }
    }

    /// Wraps the view of a window: the frame after a view that asked for a URL publishes `load`.
    pub fn watch<'a, M: Clone + 'a>(&'a self, content: impl Into<Element<'a, M>>, load: M) -> Element<'a, M> {
        Element::new(Watch { wants: self, content: content.into(), load })
    }
}

struct Watch<'a, M> {
    wants: &'a Wants,
    content: Element<'a, M>,
    load: M,
}

impl<M: Clone> Widget<M, Theme, Renderer> for Watch<'_, M> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) && self.wants.fresh.replace(false) {
            shell.publish(self.load.clone());
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
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
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

//! The avatars and media that a view shows but that no task loads yet. A view asks for them, the next frame
//! publishes a message, and `update` starts the loads. So a load costs work for the shown rows only, and an update
//! does not scan every chat and every message.

use crate::prelude::*;
use iced::advanced::mouse;
use iced::window;
use std::borrow::Borrow;

#[derive(Default)]
pub struct Wants {
    pub avatars: RefCell<BTreeSet<String>>,
    pub media: RefCell<BTreeSet<String>>,
    /// The tiles to bake, as (media URL, width, height, look).
    pub tiles: RefCell<BTreeSet<(String, u32, u32, crate::gallery::Look)>>,
    /// The animations to cut to their inline box or to dim, as (media URL, cut).
    pub cuts: RefCell<BTreeSet<(String, crate::gallery::Cut)>>,
    /// The quoted messages that the timeline does not have, as (chat, message).
    pub quotes: RefCell<BTreeSet<(String, String)>>,
    /// The URLs of the shown audio players whose audio no load fetches yet.
    pub audio: RefCell<BTreeSet<String>>,
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

    /// The loaded value `got` of `key`, or `None` while it loads: a `key` that no task loads yet is asked for.
    pub fn get<'v, K: Ord + ToOwned + ?Sized, V>(
        &self, set: &RefCell<BTreeSet<K::Owned>>, key: &K, got: Option<&'v Option<V>>,
    ) -> Option<&'v V>
    where
        K::Owned: Ord + Borrow<K>,
    {
        if got.is_none() {
            self.ask(set, key);
        }
        got?.as_ref()
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
    crate::delegate::delegate!(children, diff, size, layout, operate, mouse_interaction, draw, overlay(M));

    fn update(
        &mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        let tree = &mut tree.children[0];
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) && self.wants.fresh.replace(false) {
            shell.publish(self.load.clone());
        }
    }
}

//! The scroll of the timeline: the jump to its bottom, the load of older messages at its top, and the hold of the
//! view while it is above its bottom.

use super::*;
use iced::advanced::widget::{operation as op, tree, Operation, Tree};
use iced::advanced::{layout, Layout};
use iced::Rectangle;

#[derive(Clone)]
pub enum Msg {
    /// The timeline moved, and is above its bottom (true) or at it.
    Scrolled(bool),
    ScrollToBottom,
    /// The top of the timeline came on screen (true) or went off screen.
    TopShown(bool),
    LoadOlder,
    /// A load of older messages ended.
    LoadedOlder(Result<(), String>),
}

impl BeeperGuiApp {
    pub(crate) fn timeline_scroll(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Scrolled(up) => {
                self.timeline_scrolled(up);
                self.read_open_chat();
            }
            Msg::ScrollToBottom => {
                return operation::scroll_to(self.pane.scroll_id(), operation::AbsoluteOffset { x: 0.0, y: 0.0 });
            }
            Msg::TopShown(shown) => {
                self.pane.top.shown = shown;
                return self.load_older();
            }
            Msg::LoadOlder => {
                self.pane.top.waiting = false;
                return self.load_older();
            }
            Msg::LoadedOlder(r) => return self.loaded_older(r),
        }
        self.settle()
    }
}

/// Keeps what the timeline shows in place while it is above its bottom, like Desktop's `virtua` list: messages that
/// arrive below the view, the typing indicator and media that loads under it do not move it.
///
/// The timeline is anchored at its bottom, which keeps the view when older messages load above it, and its offset
/// counts from its bottom. So the widget notes how far the first container with an id in view, a message, is from
/// the bottom of the content, and when a layout moves it, it moves the offset by as much. At the bottom (`hold`
/// false) the timeline stays pinned to its bottom instead, like Desktop's `shouldPinToBottom`.
pub(crate) struct Hold<'a> {
    content: Element<'a, UiMessage>,
    hold: bool,
}

/// The scrollable of the timeline, held while `hold`.
pub(crate) fn hold<'a>(content: impl Into<Element<'a, UiMessage>>, hold: bool) -> Hold<'a> {
    Hold { content: content.into(), hold }
}

/// The container of a [`Hold`] that the view follows, and how far its top is from the bottom of the content.
#[derive(Default)]
struct Anchor(Option<(iced::widget::Id, f32)>);

/// What a pass over the timeline finds: its scrollable, with its viewport, content and translation, the bounds of
/// the container `target`, and the first container with an id in the viewport.
struct Measure {
    target: Option<iced::widget::Id>,
    scroll: Option<(Rectangle, Rectangle, f32)>,
    found: Option<Rectangle>,
    first: Option<(iced::widget::Id, Rectangle)>,
}

impl Operation for Measure {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self)
    }

    fn scrollable(
        &mut self,
        _: Option<&iced::widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        at: Vector,
        _: &mut dyn op::Scrollable,
    ) {
        self.scroll.get_or_insert((bounds, content, at.y));
    }

    fn container(&mut self, id: Option<&iced::widget::Id>, bounds: Rectangle) {
        let (Some(id), Some((view, content, at))) = (id, self.scroll) else { return };
        if self.target.as_ref() == Some(id) && self.found.is_none() {
            self.found = Some(bounds);
        }
        let top = bounds.y - content.y - at;
        if self.first.is_none() && top + bounds.height > 0.0 && top < view.height {
            self.first = Some((id.clone(), bounds));
        }
    }
}

/// Sets the offset of the first scrollable, which counts from its bottom.
struct SetOffset(Option<f32>);

impl Operation for SetOffset {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        if self.0.is_some() {
            operate(self)
        }
    }

    fn scrollable(
        &mut self,
        _: Option<&iced::widget::Id>,
        _: Rectangle,
        _: Rectangle,
        _: Vector,
        state: &mut dyn op::Scrollable,
    ) {
        if let Some(y) = self.0.take() {
            state.scroll_to(op::scrollable::AbsoluteOffset { x: None, y: Some(y) });
        }
    }
}

impl Hold<'_> {
    /// Passes over the timeline, for the container `target`.
    fn measure(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        target: Option<iced::widget::Id>,
    ) -> Measure {
        let mut m = Measure { target, scroll: None, found: None, first: None };
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut m);
        m
    }

    /// Notes the first container in view, and how far it is from the bottom of the content.
    fn note(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer) {
        let m = self.measure(tree, layout, renderer, None);
        let noted = m.first.zip(m.scroll).map(|((id, b), (_, content, _))| (id, content.y + content.height - b.y));
        tree.state.downcast_mut::<Anchor>().0 = noted;
    }
}

impl iced::advanced::Widget<UiMessage, Theme, iced::Renderer> for Hold<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Anchor>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Anchor::default())
    }

    crate::delegate::delegate!(children, diff, size, mouse_interaction, draw, overlay(UiMessage));

    /// An operation may scroll the timeline, as a jump to a message: the view that it leaves is the one to hold.
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
        self.note(tree, layout, renderer);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let node = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        let layout = Layout::new(&node);
        let noted = tree.state.downcast_ref::<Anchor>().0.clone();
        if let Some((id, was)) = noted.filter(|_| self.hold) {
            let m = self.measure(tree, layout, renderer, Some(id));
            if let (Some(b), Some((view, content, at))) = (m.found, m.scroll) {
                let moved = content.y + content.height - b.y - was;
                if moved.abs() >= 0.5 {
                    // The offset from the bottom that the translation from the top comes from.
                    let from_bottom = (content.height - view.height).max(0.0) - at;
                    let mut set = SetOffset(Some((from_bottom + moved).max(0.0)));
                    self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut set);
                }
            }
        }
        self.note(tree, layout, renderer);
        node
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, UiMessage>,
        viewport: &Rectangle,
    ) {
        let child = &mut tree.children[0];
        self.content.as_widget_mut().update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
        // The events that scroll: the wheel, the keys, a touch, and a drag of the scrollbar.
        let scrolls = match event {
            Event::Mouse(mouse::Event::WheelScrolled { .. } | mouse::Event::ButtonReleased(_)) => true,
            Event::Mouse(mouse::Event::CursorMoved { .. }) => shell.is_event_captured(),
            Event::Keyboard(_) | Event::Touch(_) => true,
            _ => false,
        };
        if scrolls {
            self.note(tree, layout, renderer);
        }
    }
}

impl<'a> From<Hold<'a>> for Element<'a, UiMessage> {
    fn from(h: Hold<'a>) -> Self {
        Element::new(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::renderer::Headless as _;
    use iced::advanced::widget::operation::black_box;
    use iced_runtime::user_interface::{Cache, UserInterface};

    /// How far under the top of the viewport of the timeline the container `target` is.
    struct ShownAt {
        target: iced::widget::Id,
        scroll: Option<(Rectangle, f32)>,
        at: Option<f32>,
    }

    impl Operation for ShownAt {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self)
        }

        fn scrollable(
            &mut self,
            id: Option<&iced::widget::Id>,
            _: Rectangle,
            content: Rectangle,
            at: Vector,
            _: &mut dyn op::Scrollable,
        ) {
            if id == Some(&iced::widget::Id::new("timeline")) {
                self.scroll = Some((content, at.y));
            }
        }

        fn container(&mut self, id: Option<&iced::widget::Id>, bounds: Rectangle) {
            if let (true, Some((content, at))) = (id == Some(&self.target), self.scroll) {
                self.at.get_or_insert(bounds.y - content.y - at);
            }
        }
    }

    /// A chat of 60 lines, open in `app`, and the main window.
    fn long_chat(app: &mut BeeperGuiApp) -> (String, Message) {
        let (chat, msg) = {
            let st = app.store.read();
            let has_msgs = |c: &&Chat| !st.timeline_indices(&c.id).is_empty();
            let c = st.chats.iter().filter(|c| c.flags & CHAT_PINNED == 0).find(has_msgs).expect("a chat").clone();
            (c.id.clone(), st.messages[st.timeline_indices(&c.id)[0]].clone())
        };
        let mut st = app.store.write();
        for i in 0..60 {
            let (id, text, sort_key) =
                (format!("$line{i:03}"), Some(format!("Line {i:03}")), format!("{:016}", 1000 + i));
            let timestamp = format!("2001-01-01T00:{i:02}:00.000Z");
            st.upsert_message(Message { id, text, sort_key, timestamp, ..msg.clone() });
        }
        drop(st);
        app.select_chat(&chat);
        (chat, msg)
    }

    type Ui<'a> = UserInterface<'a, UiMessage, Theme, iced::Renderer>;

    /// A main window of 1200 by 800 px that keeps its widget state from frame to frame.
    struct Frames {
        renderer: iced::Renderer,
        cache: Option<Cache>,
    }

    impl Frames {
        fn new() -> Self {
            let renderer = iced::futures::executor::block_on(iced::Renderer::new(
                iced::Font::DEFAULT,
                iced::Pixels(13.0),
                Some("tiny-skia"),
            ));
            Self { renderer: renderer.expect("a headless renderer"), cache: Some(Cache::default()) }
        }

        /// A frame of `app` where `then` operates, and how far under the top of the timeline `target` shows after.
        fn shown_at(
            &mut self,
            app: &BeeperGuiApp,
            target: &str,
            then: impl FnOnce(&mut Ui<'_>, &iced::Renderer),
        ) -> Option<f32> {
            let main = app.main_window.expect("the main window");
            let cache = self.cache.take().expect("a cache");
            let size = iced::Size::new(1200.0, 800.0);
            let mut ui = UserInterface::build(app.view(main), size, cache, &mut self.renderer);
            then(&mut ui, &self.renderer);
            let mut find = ShownAt { target: target.to_string().into(), scroll: None, at: None };
            ui.operate(&self.renderer, &mut black_box(&mut find));
            self.cache = Some(ui.into_cache());
            find.at
        }
    }

    /// Scrolls the timeline `y` px above its bottom, the way a wheel leaves it.
    fn scroll(y: f32) -> impl FnOnce(&mut Ui<'_>, &iced::Renderer) {
        move |ui, renderer| {
            let offset = op::scrollable::AbsoluteOffset { x: None, y: Some(y) };
            ui.operate(renderer, &mut black_box(&mut op::scrollable::scroll_to::<()>("timeline".into(), offset)));
        }
    }

    /// Runs the widget operations of `task` like the runtime: each goes through the frame, and the ones that follow
    /// it come after it.
    fn run(task: Task<UiMessage>) -> impl FnOnce(&mut Ui<'_>, &iced::Renderer) {
        use iced::futures::StreamExt;
        move |ui, renderer| {
            let Some(mut stream) = iced_runtime::task::into_stream(task) else { return };
            while let Some(action) = iced::futures::executor::block_on(stream.next()) {
                let iced_runtime::Action::Widget(op) = action else { continue };
                let mut next = Some(op);
                while let Some(mut op) = next.take() {
                    ui.operate(renderer, op.as_mut());
                    if let op::Outcome::Chain(then) = op.finish() {
                        next = Some(then);
                    }
                }
            }
        }
    }

    /// Desktop's `virtua` list keeps what it shows in place while it is not pinned to its bottom: messages that
    /// arrive below the view do not move it. At the bottom the list stays pinned and shows them.
    #[test]
    fn the_timeline_holds_its_view_above_its_bottom_like_desktop() {
        let mut app = crate::fuzz::demo_app("timeline-hold");
        let (_, msg) = long_chat(&mut app);
        let mut frames = Frames::new();
        let line = "msg-$line040";
        frames.shown_at(&app, line, |_, _| {});
        frames.shown_at(&app, line, scroll(500.0));
        app.timeline_scrolled(true);
        let before = frames.shown_at(&app, line, |_, _| {}).expect("the line shows");
        let arrive = |app: &mut BeeperGuiApp, i: usize| {
            let (id, text, sort_key) = (format!("$new{i}"), Some(format!("New {i}")), format!("{:016}", 2000 + i));
            let timestamp = "2001-01-01T01:00:00.000Z".to_string();
            app.store.write().upsert_message(Message { id, text, sort_key, timestamp, ..msg.clone() });
        };
        (0..3).for_each(|i| arrive(&mut app, i));
        let after = frames.shown_at(&app, line, |_, _| {});
        assert_eq!(after, Some(before), "the messages that arrive below do not move the view");
        // At its bottom the timeline follows the new messages up.
        frames.shown_at(&app, line, scroll(0.0));
        app.timeline_scrolled(false);
        let before = frames.shown_at(&app, line, |_, _| {}).expect("the line shows");
        arrive(&mut app, 3);
        let after = frames.shown_at(&app, line, |_, _| {}).expect("the line shows");
        assert!(after < before - 10.0, "{after} {before}");
    }

    /// Desktop's `revealMessage`: a jump shows the message 80 px under the top of the timeline (`scrollToSortKey`)
    /// and selects it (`selectOnlyMessage`). A jump from search and the like flashes it (`highlight`), and the jump
    /// of the media viewer (`jumpToMessage`) does not.
    #[test]
    fn jumps_show_and_select_the_message_like_desktop() {
        let mut app = crate::fuzz::demo_app("timeline-jump");
        let (chat, _) = long_chat(&mut app);
        let mut frames = Frames::new();
        let line = "msg-$line020";
        frames.shown_at(&app, line, |_, _| {});
        let jump = app.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat.clone(), "$line020".into())));
        frames.shown_at(&app, line, run(jump));
        let at = frames.shown_at(&app, line, |_, _| {}).expect("the jump shows the line");
        assert!((at - 80.0).abs() < 1.0, "{at}");
        assert_eq!(app.pane.selected_msgs, ["$line020"]);
        assert_eq!(app.flash.as_ref().map(|f| f.0.as_str()), Some("$line020"), "search and the like flash it");
        app.flash = None;
        let line = "msg-$line030";
        let go = app.update(UiMessage::Message(message::Msg::GoTo(chat, "$line030".into())));
        frames.shown_at(&app, line, run(go));
        let at = frames.shown_at(&app, line, |_, _| {}).expect("the jump shows the line");
        assert!((at - 80.0).abs() < 1.0, "{at}");
        assert_eq!(app.pane.selected_msgs, ["$line030"]);
        assert!(app.flash.is_none(), "the media viewer's jump does not flash it");
    }
}

//! The scroll of the timeline: the jump to its bottom, the load of older messages at its top, and the hold of the
//! view while it is above its bottom.

use super::*;
use crate::prelude::*;
use iced::advanced::widget::operation as op;
use iced::advanced::{layout, Layout};

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
    /// Desktop's "Refresh chat" of a chat.
    Refresh(String),
    /// A refresh ended.
    Refreshed(Result<(), String>),
}

impl BeeperGuiApp {
    pub(crate) fn timeline_scroll(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Scrolled(up) => {
                self.timeline_scrolled(up);
                self.read_open_chat();
            }
            // Desktop's `ScrollToBottom` button and hotkey scroll with `smooth: true`.
            Msg::ScrollToBottom => self.pane.glide = Some(std::time::Instant::now()),
            Msg::TopShown(shown) => {
                self.pane.top.shown = shown;
                return self.load_older();
            }
            Msg::LoadOlder => {
                self.pane.top.waiting = false;
                return self.load_older();
            }
            Msg::LoadedOlder(r) => return self.loaded_older(r),
            Msg::Refresh(chat) => return self.refresh_chat(chat),
            Msg::Refreshed(Err(e)) => {
                let (_, chat, _) = self.pane.top.loading.take().expect("a refresh ran");
                let st = self.store.read();
                let user = st.resolve_chat_idx(&chat).map(|i| account_user(&st, &st.chats[i].account_id));
                let user = user.unwrap_or_default();
                drop(st);
                // Desktop's `showErrorToast` of `wrapPlatformAPICall`.
                self.fail(Toast::new(trf("Something went wrong %1$s (%2$s)\n%3$s", &[&"refreshing chat", &user, &e])));
            }
            Msg::Refreshed(Ok(())) => return self.loaded_older(Ok(())),
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
///
/// It also glides the timeline to its bottom from the instant `glide`, like Chromium's `scrollTo` with `behavior:
/// "smooth"` that Desktop's `scrollToIndex` with `smooth: true` asks for.
pub(crate) struct Hold<'a> {
    content: Element<'a, UiMessage>,
    hold: bool,
    glide: Option<std::time::Instant>,
}

/// The scrollable of the timeline, held while `hold`.
pub(crate) fn hold<'a>(content: impl Into<Element<'a, UiMessage>>, hold: bool) -> Hold<'a> {
    Hold { content: content.into(), hold, glide: None }
}

/// The state of a [`Hold`]: the container that the view follows and how far its top is from the bottom of the
/// content, and the glide to the bottom.
#[derive(Default)]
struct Anchor {
    noted: Option<(iced::widget::Id, f32)>,
    /// The start of the glide, the offset it starts from, and how long it takes in seconds.
    glide: Option<(std::time::Instant, f32, f32)>,
    /// The start of the last glide that ended, or that a wheel or a press stopped.
    ended: Option<std::time::Instant>,
}

/// How long Chromium's programmatic smooth scroll takes for `delta` px (`ScrollOffsetAnimationCurve` with
/// `kDeltaBased`): the square root of the distance in 60ths of a second, at most 0.7 seconds.
pub(crate) fn glide_secs(delta: f32) -> f32 {
    delta.abs().sqrt().min(0.7 * 60.0) / 60.0
}

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

    fn scrollable(&mut self, _: Option<&iced::widget::Id>, bounds: Rectangle, content: Rectangle, at: Vector, _: &mut dyn op::Scrollable) {
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

    fn scrollable(&mut self, _: Option<&iced::widget::Id>, _: Rectangle, _: Rectangle, _: Vector, state: &mut dyn op::Scrollable) {
        if let Some(y) = self.0.take() {
            state.scroll_to(op::scrollable::AbsoluteOffset { x: None, y: Some(y) });
        }
    }
}

impl Hold<'_> {
    /// Glides from the instant `at` (see [`Hold`]).
    pub(crate) fn glide(self, at: Option<std::time::Instant>) -> Self {
        Self { glide: at, ..self }
    }

    /// The frame of the glide at `now`, which asks for the next frame until the glide ends.
    fn glide_frame(
        &mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, now: std::time::Instant,
        shell: &mut iced::advanced::Shell<'_, UiMessage>,
    ) {
        let state = tree.state.downcast_ref::<Anchor>();
        let Some(start) = self.glide.filter(|s| state.ended != Some(*s)) else { return };
        let glide = match state.glide.filter(|g| g.0 == start) {
            Some(g) => g,
            None => {
                let Some((view, content, at)) = self.measure(tree, layout, renderer, None).scroll else { return };
                let from = (content.height - view.height).max(0.0) - at;
                (start, from, glide_secs(from))
            }
        };
        let t = match glide.2 > 0.0 {
            true => (now.saturating_duration_since(start).as_secs_f32() / glide.2).min(1.0),
            false => 1.0,
        };
        let left = if t < 1.0 { glide.1 * (1.0 - crate::login::ease_in_out(t)) } else { 0.0 };
        let mut set = SetOffset(Some(left));
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut set);
        let state = tree.state.downcast_mut::<Anchor>();
        state.glide = Some(glide);
        if t < 1.0 {
            shell.request_redraw();
        } else {
            state.ended = Some(start);
        }
        self.note(tree, layout, renderer);
    }

    /// Passes over the timeline, for the container `target`.
    fn measure(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, target: Option<iced::widget::Id>) -> Measure {
        let mut m = Measure { target, scroll: None, found: None, first: None };
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut m);
        m
    }

    /// Notes the first container in view, and how far it is from the bottom of the content.
    fn note(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer) {
        let m = self.measure(tree, layout, renderer, None);
        let noted = m.first.zip(m.scroll).map(|((id, b), (_, content, _))| (id, content.y + content.height - b.y));
        tree.state.downcast_mut::<Anchor>().noted = noted;
    }
}

impl iced::advanced::Widget<UiMessage, Theme, iced::Renderer> for Hold<'_> {
    crate::delegate::delegate!(state(Anchor), children, diff, size, mouse_interaction, draw, overlay(UiMessage));

    /// An operation may scroll the timeline, as a jump to a message: the view that it leaves is the one to hold.
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
        self.note(tree, layout, renderer);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let node = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        let layout = Layout::new(&node);
        let noted = tree.state.downcast_ref::<Anchor>().noted.clone();
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
        &mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard, shell: &mut iced::advanced::Shell<'_, UiMessage>, viewport: &Rectangle,
    ) {
        let child = &mut tree.children[0];
        self.content.as_widget_mut().update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                return self.glide_frame(tree, layout, renderer, *now, shell);
            }
            // Like Chromium, a scroll of your own stops the glide.
            Event::Mouse(mouse::Event::WheelScrolled { .. } | mouse::Event::ButtonPressed(_)) | Event::Touch(_)
                if cursor.is_over(layout.bounds()) =>
            {
                let state = tree.state.downcast_mut::<Anchor>();
                state.ended = state.ended.max(self.glide);
            }
            _ => {}
        }
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

/// Desktop's `editing-message` of the chat's `panes`: while you edit a message, the timeline is under a backdrop
/// (`.panes:after`, a 4 px blur that fades in over 0.12 s) that only the edited message (`.is-editing`, z-index
/// 99999) is above. The timeline does not scroll (`overflow: hidden`), and a click on the backdrop ends the edit
/// (`clearEditingMessage` of its `onClick`). iced has no backdrop blur, so the backdrop is the chat's background at
/// [`VEIL_ALPHA`], and the messages under it take no pointer.
pub(crate) struct Veil<'a> {
    content: Element<'a, UiMessage>,
    /// The container of the edited message, and when the edit started.
    target: iced::widget::Id,
    since: std::time::Instant,
    color: Color,
    on_press: UiMessage,
}

/// How much of the chat's background covers what the 4 px blur of Desktop's backdrop blurs.
pub(crate) const VEIL_ALPHA: f32 = 0.6;

/// The timeline `content` under a [`Veil`] with a hole for the container `target`, from `since`.
pub(crate) fn veil<'a>(
    content: impl Into<Element<'a, UiMessage>>, target: iced::widget::Id, since: std::time::Instant, color: Color, on_press: UiMessage,
) -> Veil<'a> {
    Veil { content: content.into(), target, since, color, on_press }
}

impl Veil<'_> {
    /// Finds where the edited message shows, in the viewport of the timeline.
    fn find(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer) {
        let mut m = Measure { target: Some(self.target.clone()), scroll: None, found: None, first: None };
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut m);
        let hole = m.found.zip(m.scroll).and_then(|(b, (view, _, at))| Rectangle { y: b.y - at, ..b }.intersection(&view));
        *tree.state.downcast_mut::<Option<Rectangle>>() = hole;
    }

    /// Whether the pointer is on the backdrop, not on the edited message.
    fn on_veil(tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor) -> bool {
        let hole = *tree.state.downcast_ref::<Option<Rectangle>>();
        cursor.is_over(layout.bounds()) && !hole.is_some_and(|h| cursor.is_over(h))
    }
}

impl iced::advanced::Widget<UiMessage, Theme, iced::Renderer> for Veil<'_> {
    crate::delegate::delegate!(state(Option<Rectangle>), children, diff, size, operate, overlay(UiMessage));

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let node = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        self.find(tree, Layout::new(&node), renderer);
        node
    }

    fn update(
        &mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard, shell: &mut iced::advanced::Shell<'_, UiMessage>, viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            // A jump to the edited message scrolls the timeline before the frame.
            self.find(tree, layout, renderer);
            if self.since.elapsed() < std::time::Duration::from_millis(120) {
                shell.request_redraw();
            }
        }
        let on_veil = Self::on_veil(tree, layout, cursor);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(_)) if on_veil => {
                shell.publish(self.on_press.clone());
                return shell.capture_event();
            }
            Event::Mouse(mouse::Event::WheelScrolled { .. }) if cursor.is_over(layout.bounds()) => {
                return shell.capture_event();
            }
            _ => {}
        }
        let cursor = if on_veil { mouse::Cursor::Unavailable } else { cursor };
        let child = &mut tree.children[0];
        self.content.as_widget_mut().update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        match Self::on_veil(tree, layout, cursor) {
            true => mouse::Interaction::None,
            false => self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer),
        }
    }

    fn draw(
        &self, tree: &Tree, renderer: &mut iced::Renderer, theme: &Theme, style: &iced::advanced::renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let on_veil = Self::on_veil(tree, layout, cursor);
        let inner = if on_veil { mouse::Cursor::Unavailable } else { cursor };
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, inner, viewport);
        let b = layout.bounds();
        // `backdrop-blur-in`: 0.12 s, `ease`.
        let t = (self.since.elapsed().as_secs_f32() / 0.12).min(1.0);
        let color = self.color.scale_alpha(VEIL_ALPHA * crate::style::cubic_bezier(t, [0.25, 0.1, 0.25, 1.0]));
        let hole = *tree.state.downcast_ref::<Option<Rectangle>>();
        let parts = match hole {
            Some(h) => vec![
                Rectangle { height: h.y - b.y, ..b },
                Rectangle { y: h.y + h.height, height: b.y + b.height - (h.y + h.height), ..b },
                Rectangle { x: b.x, y: h.y, width: h.x - b.x, height: h.height },
                Rectangle { x: h.x + h.width, y: h.y, width: b.x + b.width - (h.x + h.width), height: h.height },
            ],
            None => vec![b],
        };
        renderer.with_layer(b, |r| {
            for bounds in parts.into_iter().filter(|r| r.width > 0.0 && r.height > 0.0) {
                r.fill_quad(iced::advanced::renderer::Quad { bounds, ..Default::default() }, color);
            }
        });
    }
}

impl<'a> From<Veil<'a>> for Element<'a, UiMessage> {
    fn from(v: Veil<'a>) -> Self {
        Element::new(v)
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
        /// The offset of the timeline from its bottom.
        offset: f32,
        at: Option<f32>,
    }

    impl Operation for ShownAt {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self)
        }

        fn scrollable(
            &mut self, id: Option<&iced::widget::Id>, view: Rectangle, content: Rectangle, at: Vector, _: &mut dyn op::Scrollable,
        ) {
            if id == Some(&iced::widget::Id::new("timeline")) {
                (self.scroll, self.offset) = (Some((content, at.y)), (content.height - view.height).max(0.0) - at.y);
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
            let (id, text, sort_key) = (format!("$line{i:03}"), Some(format!("Line {i:03}")), format!("{:016}", 1000 + i));
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
        /// The instant of the frame that the next view draws, if it draws one.
        frame: Option<std::time::Instant>,
        /// The offset of the timeline from its bottom in the last frame.
        offset: f32,
    }

    impl Frames {
        fn new() -> Self {
            let renderer =
                iced::futures::executor::block_on(iced::Renderer::new(iced::Font::DEFAULT, iced::Pixels(13.0), Some("tiny-skia")));
            Self { renderer: renderer.expect("a headless renderer"), cache: Some(Cache::default()), frame: None, offset: 0.0 }
        }

        /// A frame of `app` where `then` operates, and how far under the top of the timeline `target` shows after.
        fn shown_at(&mut self, app: &BeeperGuiApp, target: &str, then: impl FnOnce(&mut Ui<'_>, &iced::Renderer)) -> Option<f32> {
            let main = app.main_window.expect("the main window");
            let cache = self.cache.take().expect("a cache");
            let size = iced::Size::new(1200.0, 800.0);
            let mut ui = UserInterface::build(app.view(main), size, cache, &mut self.renderer);
            then(&mut ui, &self.renderer);
            if let Some(now) = self.frame.take() {
                let redraw = [iced::Event::Window(window::Event::RedrawRequested(now))];
                let clipboard = &mut iced::advanced::clipboard::Null;
                let _ = ui.update(&redraw, mouse::Cursor::Unavailable, &mut self.renderer, clipboard, &mut Vec::new());
            }
            let mut find = ShownAt { target: target.to_string().into(), scroll: None, at: None, offset: 0.0 };
            ui.operate(&self.renderer, &mut black_box(&mut find));
            self.cache = Some(ui.into_cache());
            self.offset = find.offset;
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

    /// Desktop's `ScrollToBottom` button and hotkey scroll with `smooth: true`: Chromium glides the timeline down
    /// with `ease-in-out`, in the square root of the distance in 60ths of a second, and at most 0.7 seconds.
    #[test]
    fn the_scroll_to_bottom_glides_like_desktop() {
        assert_eq!(glide_secs(100.0), 10.0 / 60.0);
        assert_eq!(glide_secs(5000.0), 0.7);
        let mut app = crate::fuzz::demo_app("timeline-glide");
        long_chat(&mut app);
        let mut frames = Frames::new();
        offset_at(&mut frames, &app, None);
        frames_scroll(&mut frames, &app, 600.0);
        app.timeline_scrolled(true);
        let _ = app.update(UiMessage::Timeline(Msg::ScrollToBottom));
        let start = app.pane.glide.expect("a glide");
        let secs = glide_secs(600.0);
        let after = |s: f32| Some(start + std::time::Duration::from_secs_f32(s));
        assert_eq!(offset_at(&mut frames, &app, Some(start)), 600.0, "the glide starts where the timeline is");
        // Halfway through the time, `ease-in-out` is halfway down.
        let half = offset_at(&mut frames, &app, after(secs / 2.0));
        assert!((half - 300.0).abs() < 1.0, "{half}");
        let late = offset_at(&mut frames, &app, after(secs * 0.9));
        assert!(late > 0.0 && late < 20.0, "{late}");
        let end = offset_at(&mut frames, &app, after(secs + 0.001));
        assert!(end < 1.0, "the glide ends at the bottom: {end}");
    }

    /// The offset of the timeline from its bottom after a frame of `app`, drawn at `frame` if given.
    fn offset_at(frames: &mut Frames, app: &BeeperGuiApp, frame: Option<std::time::Instant>) -> f32 {
        frames.frame = frame;
        frames.shown_at(app, "msg-$line000", |_, _| {});
        frames.offset
    }

    /// Scrolls the timeline of `app` `y` px above its bottom in a frame of `frames`.
    fn frames_scroll(frames: &mut Frames, app: &BeeperGuiApp, y: f32) {
        frames.shown_at(app, "msg-$line000", scroll(y));
        assert_eq!(frames.offset, y);
    }
}

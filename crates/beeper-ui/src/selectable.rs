//! Rich text the pointer can select, like Desktop's `.message { user-select: text }`.
//!
//! A drag that starts on the text of one message extends over the others, the way a browser selection does: every
//! [`Selectable`] reads the same drag and highlights the part between its two ends in reading order. Ctrl+C copies
//! the selected parts top to bottom, one line each ([`text`]). A double click selects a word and a triple click the
//! paragraph.
//! Selected text is white on Chromium's selection blue.
//!
//! It is iced's `Rich` text with the selection added, so links still work and the layout is the same.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use iced::advanced::graphics::text::cosmic_text::{Affinity, Buffer, Cursor};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::text::{self as core_text, Paragraph as _, Renderer as _, Span};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::alignment;
use iced::mouse;
use iced::widget::text::{Alignment, LineHeight, Shaping, Wrapping};
use iced::{Color, Element, Event, Length, Pixels, Point, Rectangle, Size, Theme, Vector};

type Renderer = iced::Renderer;
type Paragraph = <Renderer as core_text::Renderer>::Paragraph;
type Font = <Renderer as core_text::Renderer>::Font;

/// Chromium's selection on Linux: white text on blue.
const SELECTED_BACKGROUND: Color = Color::from_rgb8(0x30, 0x64, 0xce);
const SELECTED_TEXT: Color = Color::WHITE;
/// GTK's default `gtk-double-click-time`, which Chromium follows.
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Every text sees a press within this long of the one it landed on, so they can tell it from a later one.
const SAME_PRESS: Duration = Duration::from_millis(100);

/// The selection, shared by every selectable text so it can run across them. Each text derives its part when it
/// needs it, so a text the view rebuilt mid-drag still shows its part.
#[derive(Clone, Copy, PartialEq)]
enum Selection {
    /// A drag from `anchor` to `head`, in the coordinates of the scrolled content.
    Drag { anchor: Point, head: Point, held: bool },
    /// A double or triple click's word or paragraph in the text at `at`.
    Within { at: Point, start: Cursor, end: Cursor },
}

impl Selection {
    /// The part of the text with `buffer` at `bounds`, start before end.
    fn part(self, buffer: &Buffer, bounds: Rectangle) -> Option<(Cursor, Cursor)> {
        let (start, end) = match self {
            Self::Drag { anchor, head, .. } => drag_part(buffer, anchor, head, bounds)?,
            Self::Within { at, start, end } if at == bounds.position() => (start, end),
            Self::Within { .. } => return None,
        };
        let (start, end) = (before_padding(buffer, start), before_padding(buffer, end));
        (start != end).then_some((start, end))
    }
}

#[derive(Clone, Copy)]
struct Click {
    at: Instant,
    position: Point,
    count: u8,
}

thread_local! {
    static SELECTION: Cell<Option<Selection>> = const { Cell::new(None) };
    static CLICK: Cell<Option<Click>> = const { Cell::new(None) };
    /// The selected text of each text on screen, by place, so [`text`] joins them in reading order. Drawing fills it.
    static PIECES: RefCell<BTreeMap<(i32, i32), String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Selectable rich text: `iced::widget::rich_text` with a selection.
pub fn selectable<'a, Link: Clone + 'static, Message>(
    spans: impl AsRef<[Span<'a, Link, Font>]> + 'a,
) -> Selectable<'a, Link, Message> {
    Selectable {
        spans: Box::new(spans),
        size: None,
        line_height: LineHeight::default(),
        width: Length::Shrink,
        wrapping: Wrapping::default(),
        color: None,
        hovered_link: None,
        on_link_click: None,
    }
}

pub struct Selectable<'a, Link: Clone + 'static, Message> {
    spans: Box<dyn AsRef<[Span<'a, Link, Font>]> + 'a>,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    wrapping: Wrapping,
    color: Option<Color>,
    hovered_link: Option<usize>,
    on_link_click: Option<Box<dyn Fn(Link) -> Message + 'a>>,
}

impl<'a, Link: Clone + 'static, Message> Selectable<'a, Link, Message> {
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn on_link_click(mut self, on_link_click: impl Fn(Link) -> Message + 'a) -> Self {
        self.on_link_click = Some(Box::new(on_link_click));
        self
    }

    fn spans(&self) -> &[Span<'a, Link, Font>] {
        self.spans.as_ref().as_ref()
    }
}

struct State<Link> {
    spans: Vec<Span<'static, Link, Font>>,
    span_pressed: Option<usize>,
    paragraph: Paragraph,
    /// The same text all in [`SELECTED_TEXT`], drawn over the selection while there is one.
    selected_paragraph: Option<Paragraph>,
}

impl<Link> State<Link> {
    fn part(&self, bounds: Rectangle) -> Option<(Cursor, Cursor)> {
        SELECTION.get()?.part(self.paragraph.buffer(), bounds)
    }
}

/// Sets the selection, and asks every text to lay out again (for its selected paragraph) and redraw when it changed.
fn select<Message>(selection: Option<Selection>, shell: &mut Shell<'_, Message>) {
    if SELECTION.replace(selection) != selection {
        PIECES.with_borrow_mut(BTreeMap::clear);
        shell.invalidate_layout();
        shell.request_redraw();
    }
}

/// Whether some text is selected, as Desktop checks `getSelection().isCollapsed` before a click selects the message
/// or a double click acts on it.
pub fn has_selection() -> bool {
    match SELECTION.get() {
        None => false,
        Some(Selection::Within { .. }) => true,
        Some(Selection::Drag { .. }) => PIECES.with_borrow(|p| !p.is_empty()),
    }
}

/// The selected text: each text's part, top to bottom, one line each, like Chromium's `selectionText`.
pub fn text() -> String {
    PIECES.with_borrow(|p| p.values().cloned().collect::<Vec<_>>().join("\n"))
}

impl<Link: Clone + 'static, Message> Widget<Message, Theme, Renderer> for Selectable<'_, Link, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Link>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Link> {
            spans: Vec::new(),
            span_pressed: None,
            paragraph: Paragraph::default(),
            selected_paragraph: None,
        })
    }

    fn size(&self) -> Size<Length> {
        Size { width: self.width, height: Length::Shrink }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Link>>();
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let font = renderer.default_font();
        let (line_height, wrapping) = (self.line_height, self.wrapping);
        let spans = self.spans.as_ref().as_ref();
        layout::sized(limits, self.width, Length::Shrink, |limits| {
            let bounds = limits.max();
            let text = |content| styled(content, bounds, size, line_height, font, wrapping);
            let shape = || styled((), bounds, size, line_height, font, wrapping);
            if state.spans != spans {
                state.paragraph = Paragraph::with_spans(text(spans));
                state.spans = spans.iter().cloned().map(Span::to_static).collect();
                state.selected_paragraph = None;
            } else {
                match state.paragraph.compare(shape()) {
                    core_text::Difference::None => {}
                    core_text::Difference::Bounds => {
                        state.paragraph.resize(bounds);
                        if let Some(selected) = &mut state.selected_paragraph {
                            selected.resize(bounds);
                        }
                    }
                    core_text::Difference::Shape => {
                        state.paragraph = Paragraph::with_spans(text(spans));
                        state.selected_paragraph = None;
                    }
                }
            }
            if SELECTION.get().is_none() {
                state.selected_paragraph = None;
            } else if state.selected_paragraph.is_none() {
                let white: Vec<_> = spans.iter().cloned().map(|s| s.color(SELECTED_TEXT)).collect();
                state.selected_paragraph = Some(Paragraph::with_spans(text(white.as_slice())));
            }
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<State<Link>>();
        let part = state.part(bounds);
        let key = (bounds.y.round() as i32, bounds.x.round() as i32);
        let piece =
            part.map(|(start, end)| selected_text(state.paragraph.buffer(), start, end)).filter(|t| !t.is_empty());
        PIECES.with_borrow_mut(|pieces| match piece {
            Some(piece) => pieces.insert(key, piece),
            None => pieces.remove(&key),
        });
        if !bounds.intersects(viewport) {
            return;
        }
        let translation = bounds.position() - Point::ORIGIN;
        let color = self.color.unwrap_or(defaults.text_color);
        let size = self.size.unwrap_or_else(|| renderer.default_size());

        // Highlights, underlines and strikethroughs of the spans, as iced's `Rich` draws them.
        for (index, span) in self.spans().iter().enumerate() {
            let hovered_link = self.on_link_click.is_some() && Some(index) == self.hovered_link;
            if span.highlight.is_none() && !span.underline && !span.strikethrough && !hovered_link {
                continue;
            }
            let regions = state.paragraph.span_bounds(index);
            if let Some(highlight) = span.highlight {
                for region in &regions {
                    let region = Rectangle::new(
                        region.position() - Vector::new(span.padding.left, span.padding.top),
                        region.size() + Size::new(span.padding.x(), span.padding.y()),
                    );
                    let quad = Quad { bounds: region + translation, border: highlight.border, ..Quad::default() };
                    renderer.fill_quad(quad, highlight.background);
                }
            }
            let size = span.size.unwrap_or(size);
            let line_height = span.line_height.unwrap_or(self.line_height).to_absolute(size);
            let ink = span.color.unwrap_or(color);
            let baseline = translation + Vector::new(0.0, size.0 + (line_height.0 - size.0) / 2.0);
            let mut line = |offset: f32| {
                for region in &regions {
                    let bounds = Rectangle::new(
                        region.position() + baseline - Vector::new(0.0, offset),
                        Size::new(region.width, 1.0),
                    );
                    renderer.fill_quad(Quad { bounds, ..Quad::default() }, ink);
                }
            };
            if span.underline || hovered_link {
                line(size.0 * 0.08);
            }
            if span.strikethrough {
                line(size.0 / 2.0);
            }
        }

        let selected = part.map(|(start, end)| highlights(state.paragraph.buffer(), start, end));
        for rect in selected.iter().flatten() {
            renderer.fill_quad(Quad { bounds: *rect + translation, ..Quad::default() }, SELECTED_BACKGROUND);
        }
        renderer.fill_paragraph(&state.paragraph, bounds.position(), color, *viewport);
        if let (Some(rects), Some(white)) = (selected, &state.selected_paragraph) {
            for rect in rects {
                if let Some(clip) = (rect + translation).intersection(viewport) {
                    renderer.fill_paragraph(white, bounds.position(), SELECTED_TEXT, clip);
                }
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State<Link>>();
        let inside = cursor.position_in(bounds);

        let was_hovered = self.hovered_link.is_some();
        self.hovered_link = inside
            .and_then(|position| state.paragraph.hit_span(position))
            .filter(|&span| self.spans().get(span).is_some_and(|s| s.link.is_some()))
            .filter(|_| self.on_link_click.is_some());
        if was_hovered != self.hovered_link.is_some() {
            shell.request_redraw();
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if self.hovered_link.is_some() {
                    state.span_pressed = self.hovered_link;
                    shell.capture_event();
                    return;
                }
                // A press on the text starts the next selection; one anywhere else just ends this one.
                let (Some(position), Some(local)) = (cursor.position(), inside) else {
                    // Unless the press is the one that just started a selection on another text.
                    if !CLICK.get().is_some_and(|c| c.at.elapsed() < SAME_PRESS) {
                        select(None, shell);
                    }
                    return;
                };
                let now = Instant::now();
                let count = match CLICK.get() {
                    Some(c) if now - c.at < MULTI_CLICK && c.position.distance(position) < 4.0 => c.count % 3 + 1,
                    _ => 1,
                };
                CLICK.set(Some(Click { at: now, position, count }));
                let buffer = state.paragraph.buffer();
                let at = bounds.position();
                let selection = match count {
                    1 => Some(Selection::Drag { anchor: position, head: position, held: true }),
                    2 => hit(buffer, local, bounds.size()).map(|c| word(buffer, c)),
                    _ => hit(buffer, local, bounds.size()).map(|c| paragraph(buffer, c)),
                };
                let selection = selection.map(|s| match s {
                    Selection::Within { start, end, .. } => Selection::Within { at, start, end },
                    drag => drag,
                });
                select(selection, shell);
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // The cursor, not the event, is in the coordinates of the scrolled content the layout uses.
                let (Some(Selection::Drag { anchor, held: true, .. }), Some(head)) =
                    (SELECTION.get(), cursor.position())
                else {
                    return;
                };
                select(Some(Selection::Drag { anchor, head, held: true }), shell);
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(Selection::Drag { anchor, head, .. }) = SELECTION.get() {
                    SELECTION.set(Some(Selection::Drag { anchor, head, held: false }));
                }
                let pressed = state.span_pressed.take().filter(|span| Some(*span) == self.hovered_link);
                let link = pressed.and_then(|span| self.spans().get(span)?.link.clone());
                if let (Some(link), Some(on_click)) = (link, &self.on_link_click) {
                    shell.publish(on_click(link));
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.hovered_link.is_some() {
            mouse::Interaction::Pointer
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Link: Clone + 'static, Message: 'a> From<Selectable<'a, Link, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(text: Selectable<'a, Link, Message>) -> Self {
        Element::new(text)
    }
}

fn styled<T>(
    content: T,
    bounds: Size,
    size: Pixels,
    line_height: LineHeight,
    font: Font,
    wrapping: Wrapping,
) -> core_text::Text<T, Font> {
    core_text::Text {
        content,
        bounds,
        size,
        line_height,
        font,
        align_x: Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping,
    }
}

/// The cursor at `point`, inside the text's box.
fn hit(buffer: &Buffer, point: Point, size: Size) -> Option<Cursor> {
    buffer.hit(point.x.clamp(0.0, size.width), point.y.clamp(0.0, (size.height - 1.0).max(0.0)))
}

fn text_end(buffer: &Buffer) -> Cursor {
    let line = buffer.lines.len().saturating_sub(1);
    Cursor::new(line, buffer.lines.get(line).map_or(0, |l| l.text().len()))
}

/// This text's part of a drag, in reading order: whole when the drag spans it, from or to the pointer when an end
/// lies on it.
fn drag_part(buffer: &Buffer, anchor: Point, head: Point, bounds: Rectangle) -> Option<(Cursor, Cursor)> {
    let before = |a: Point, b: Point| (a.y, a.x) < (b.y, b.x);
    let (start, end) = if before(head, anchor) { (head, anchor) } else { (anchor, head) };
    if end.y < bounds.y || start.y > bounds.y + bounds.height {
        return None;
    }
    let origin = bounds.position() - Point::ORIGIN;
    let from = if start.y < bounds.y { Cursor::new(0, 0) } else { hit(buffer, start - origin, bounds.size())? };
    let to =
        if end.y > bounds.y + bounds.height { text_end(buffer) } else { hit(buffer, end - origin, bounds.size())? };
    let order = |c: Cursor| (c.line, c.index);
    Some(if order(to) < order(from) { (to, from) } else { (from, to) })
}

/// The word under the pointer at `at`, like a double click in a browser: the run of letters, of spaces or of other
/// characters the clicked character belongs to.
fn word(buffer: &Buffer, at: Cursor) -> Selection {
    let text = buffer.lines.get(at.line).map_or("", |l| l.text());
    let index = at.index.min(text.len());
    // A hit on the right half of a character puts the cursor after it.
    let under = match at.affinity {
        Affinity::Before => text[..index].char_indices().next_back().map_or(index, |(i, _)| i),
        Affinity::After => index,
    };
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let Some(here) = text[under..].chars().next().map(kind) else {
        return Selection::Within { at: Point::ORIGIN, start: at, end: at };
    };
    let start =
        text[..under].char_indices().rev().take_while(|(_, c)| kind(*c) == here).last().map_or(under, |(i, _)| i);
    let end = text[under..].char_indices().find(|(_, c)| kind(*c) != here).map_or(text.len(), |(i, _)| under + i);
    Selection::Within { at: Point::ORIGIN, start: Cursor::new(at.line, start), end: Cursor::new(at.line, end) }
}

/// The paragraph (text line) around `at`, like a triple click.
fn paragraph(buffer: &Buffer, at: Cursor) -> Selection {
    let len = buffer.lines.get(at.line).map_or(0, |l| l.text().len());
    Selection::Within { at: Point::ORIGIN, start: Cursor::new(at.line, 0), end: Cursor::new(at.line, len) }
}

/// The boxes of the selection, one per visual line, in the text's coordinates.
fn highlights(buffer: &Buffer, start: Cursor, end: Cursor) -> Vec<Rectangle> {
    buffer
        .layout_runs()
        .filter_map(|run| {
            let (x, width) = run.highlight(start, end)?;
            // A selection running on past a line's end covers the space of the line break, as Chromium does.
            let breaks = run.line_i < end.line && run.glyphs.last().is_some_and(|g| g.end >= run.text.len());
            let more = if breaks { 4.0 } else { 0.0 };
            Some(Rectangle::new(Point::new(x, run.line_top), Size::new(width + more, run.line_height)))
        })
        .filter(|r| r.width > 0.0)
        .collect()
}

/// `at`, moved back out of the invisible padding message bubbles append for the time, which Desktop does not select.
fn before_padding(buffer: &Buffer, at: Cursor) -> Cursor {
    let text = buffer.lines.get(at.line).map_or("", |l| l.text());
    let end = text.trim_end_matches(['\u{a0}', '\u{200b}', '\u{2060}']).len();
    Cursor::new(at.line, at.index.min(end))
}

/// The selected text, without the invisible padding message bubbles append for the time.
fn selected_text(buffer: &Buffer, start: Cursor, end: Cursor) -> String {
    let lines: Vec<&str> = (start.line..=end.line)
        .filter_map(|i| {
            let text = buffer.lines.get(i)?.text();
            let from = if i == start.line { start.index.min(text.len()) } else { 0 };
            let to = if i == end.line { end.index.min(text.len()) } else { text.len() };
            text.get(from..to.max(from))
        })
        .collect();
    lines.join("\n").trim_end_matches(['\u{a0}', '\u{200b}', '\u{2060}']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::span;

    fn buffer(text: &str, width: f32) -> Paragraph {
        let spans = [span::<(), Font>(text.to_string())];
        let size = Size::new(width, f32::INFINITY);
        Paragraph::with_spans(styled(
            &spans[..],
            size,
            13.0.into(),
            LineHeight::Absolute(16.0.into()),
            Font::DEFAULT,
            Wrapping::WordOrGlyph,
        ))
    }

    fn texts(selection: Selection, paragraph: &Paragraph, bounds: Rectangle) -> String {
        let (start, end) = selection.part(paragraph.buffer(), bounds).expect("a part");
        selected_text(paragraph.buffer(), start, end)
    }

    #[test]
    fn double_click_selects_the_word_and_its_kind() {
        let p = buffer("see you at 7:30, ok?", 400.0);
        let b = p.buffer();
        let at = |i| word(b, Cursor::new_with_affinity(0, i, Affinity::After));
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(400.0, 16.0));
        assert_eq!(texts(at(5), &p, bounds), "you");
        assert_eq!(texts(at(11), &p, bounds), "7");
        assert_eq!(texts(at(12), &p, bounds), ":");
        assert_eq!(texts(at(3), &p, bounds), " ", "a space selects the run of spaces");
        let after = word(b, Cursor::new_with_affinity(0, 7, Affinity::Before));
        assert_eq!(texts(after, &p, bounds), "you", "a hit on the right half of the last letter");
        assert!(at(5).part(b, Rectangle::new(Point::new(0.0, 20.0), bounds.size())).is_none(), "only in its own text");
    }

    #[test]
    fn a_drag_selects_this_texts_part_in_reading_order() {
        let p = buffer("first line of text", 400.0);
        let bounds = Rectangle::new(Point::new(10.0, 100.0), Size::new(400.0, 16.0));
        let drag = |anchor, head| Selection::Drag { anchor, head, held: true };
        // Over the text, backwards: the same part as forwards.
        let (a, b) = (Point::new(10.0, 108.0), Point::new(60.0, 108.0));
        assert_eq!(texts(drag(b, a), &p, bounds), texts(drag(a, b), &p, bounds));
        // From above to below: all of it; from above to inside: up to the pointer.
        assert_eq!(texts(drag(Point::new(0.0, 0.0), Point::new(0.0, 300.0)), &p, bounds), "first line of text");
        assert_eq!(texts(drag(Point::new(500.0, 50.0), Point::new(36.0, 108.0)), &p, bounds), "first");
        assert!(drag(Point::new(0.0, 0.0), Point::new(0.0, 50.0)).part(p.buffer(), bounds).is_none());
    }

    #[test]
    fn the_time_padding_is_not_selected() {
        let p = buffer("hi\u{2060}\u{a0}\u{a0}\u{a0}\u{a0}", 400.0);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(400.0, 16.0));
        let all = Selection::Drag { anchor: Point::new(0.0, -5.0), head: Point::new(0.0, 50.0), held: false };
        assert_eq!(texts(all, &p, bounds), "hi");
        let tail = Selection::Drag { anchor: Point::new(390.0, 8.0), head: Point::new(399.0, 8.0), held: false };
        assert!(tail.part(p.buffer(), bounds).is_none(), "the padding alone is no selection");
    }
}

//! Desktop's `virtua` `VList` with `overscan: 15`: a list that builds only the rows in or near its viewport.
//!
//! The view gives each row a key. A `VList` keeps the measured height of each row and the visible span of the last
//! frame. A view builds the visible rows and `OVERSCAN` more rows on each side, and one spacer above and one spacer
//! below stand for the other rows. A frame that measures a new row height, or that moves the visible span out of
//! the inner half of the overscan, publishes the list message, so that the view builds the rows again.

use crate::prelude::*;
use iced::advanced::layout::{self, flex};
use iced::advanced::{mouse, overlay, renderer, Renderer as _};
use iced::{window, Alignment};
use std::hash::{Hash, Hasher};

const OVERSCAN: usize = 15;
/// How long a list counts as scrolling after its visible span last moved, like `virtua`'s `isScrolling`.
const SCROLL_IDLE: std::time::Duration = std::time::Duration::from_millis(150);
/// The height of a section label, Desktop's 24 px date pill, which its row places 4 px under its top.
const LABEL: f32 = 24.0;
/// The fade of a label that hides or shows: `.sticky-date-container`'s `opacity 0.1s cubic-bezier(.165, .84, .44, 1)`.
const FADE: f32 = 0.1;
/// The key of the label of the section that starts at the row of a key.
const LABEL_KEY: u64 = 0x9e37_79b9_7f4a_7c15;
/// The visible height that a list uses before a frame measures it.
const FIRST_SPAN: f32 = 1600.0;

/// The key of a row: the FNV-1a hash of its id, which is fast for the short ids of chats and messages.
pub fn key(id: impl Hash) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    id.hash(&mut h);
    h.0
}

struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// The hasher of the height map: its keys are hashes already.
#[derive(Default)]
struct Same(u64);

impl Hasher for Same {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _: &[u8]) {
        unreachable!("the height map has u64 keys")
    }

    fn write_u64(&mut self, key: u64) {
        self.0 = key;
    }
}

#[derive(Default)]
pub struct VList(RefCell<Inner>);

/// Desktop's `sticky-date-container`: each section's label is `position: sticky` in a box from its section to the next,
/// so it floats at `line` once its place goes above it, and the next section pushes it up. While the list stands still,
/// a label hides once its section starts more than 20 px above `pad` (`offset + 20 < scrollOffset`, where the offsets
/// leave out the padding of the scrolling container).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Float {
    /// The padding at the top of the scrolling container.
    pub pad: f32,
    /// The sticky top of the label (`.sticky-date-slider`) under the top of the viewport.
    pub line: f32,
}

#[derive(Default)]
struct Inner {
    /// The width of the measured rows.
    width: f32,
    heights: HashMap<u64, f32, std::hash::BuildHasherDefault<Same>>,
    /// The mean height of the rows of the last frame, for the rows that no frame measured.
    typical: Option<f32>,
    /// The visible span of the last frame: its distance from the anchored edge of the list, and its height.
    seen: Option<(f32, f32)>,
    /// The span from the anchored edge that the visible span can move in without a new view.
    safe: (f32, f32),
    /// The row that the next view must put in the middle of the visible span.
    target: Option<u64>,
    /// The keys, the anchor and the unmeasured height of the last view, and where each row starts from the anchored
    /// edge, so that a view of the same rows does not add up all heights again.
    starts: (Vec<u64>, bool, f32, Vec<f32>),
    /// A frame measured a height that `starts` does not have.
    stale: bool,
    /// The rows that start a section, as ascending indices of the keys: the days of Desktop's messages list.
    sections: Vec<usize>,
    /// How the labels of the sections float.
    float: Option<Float>,
    /// Whether the label of the section at the row of each key shows, and since when.
    fades: HashMap<u64, (bool, std::time::Instant)>,
    /// When the visible span last moved.
    moved: Option<std::time::Instant>,
}

impl VList {
    /// The rows of `keys`, in display order, near the visible span. `end` tells that the scrollable anchors the
    /// list at its bottom. `estimate` is the height of a row before the first measurement. `row` builds a row from
    /// its index, and the list publishes `relist` when it must build other rows.
    pub fn view<'a, M: Clone + 'a>(
        &'a self, keys: &[u64], end: bool, estimate: f32, relist: M, row: impl FnMut(usize) -> Element<'a, M>,
    ) -> Element<'a, M> {
        self.labeled(keys, end, estimate, relist, row, |_| None)
    }

    /// [`Self::view`] with the label of each section, which `label` builds from the index of its first row. The list
    /// draws the labels of its sections in view, and that of the last section above them.
    pub fn labeled<'a, M: Clone + 'a>(
        &'a self, keys: &[u64], end: bool, estimate: f32, relist: M, mut row: impl FnMut(usize) -> Element<'a, M>,
        mut label: impl FnMut(usize) -> Option<Element<'a, M>>,
    ) -> Element<'a, M> {
        let mut guard = self.0.borrow_mut();
        let inner = &mut *guard;
        let n = keys.len();
        let index = |k: usize| if end { n - 1 - k } else { k };
        if inner.heights.len() > 2 * n + 256 {
            let live: std::collections::HashSet<u64> = keys.iter().copied().collect();
            inner.heights.retain(|k, _| live.contains(k));
        }
        let target = inner.target.take();
        let unmeasured = inner.typical.unwrap_or(estimate);
        let height = |i: usize| inner.heights.get(&keys[i]).copied().unwrap_or(unmeasured);
        let (old, old_end, old_unmeasured, _) = &inner.starts;
        if inner.stale || old[..] != keys[..] || *old_end != end || *old_unmeasured != unmeasured {
            let mut starts = Vec::with_capacity(n + 1);
            let mut y = 0.0;
            for k in 0..n {
                starts.push(y);
                y += height(index(k));
            }
            starts.push(y);
            inner.starts = (keys.to_vec(), end, unmeasured, starts);
            inner.stale = false;
        }
        let starts = &inner.starts.3;
        let y = starts[n];
        let (mut at, len) = inner.seen.unwrap_or((0.0, FIRST_SPAN));
        // Like [`Self::nearest`], a row that the list does not have keeps the list where it is.
        if let Some(k) = target.and_then(|t| (0..n).find(|&k| keys[index(k)] == t)) {
            at = starts[k] - (len - (starts[k + 1] - starts[k])) / 2.0;
        }
        let at = at.clamp(0.0, (y - len).max(0.0));
        let first = starts[1..].partition_point(|&s| s <= at);
        let last = starts[..n].partition_point(|&s| s < at + len).max(first);
        let (lo, hi) = (first.saturating_sub(OVERSCAN), (last + OVERSCAN).min(n));
        inner.safe = (
            if lo == 0 { f32::NEG_INFINITY } else { starts[(lo + OVERSCAN / 2).min(first)] },
            if hi == n { f32::INFINITY } else { starts[(hi - OVERSCAN / 2).max(last)] },
        );
        let (near, far) = (starts[lo], y - starts[hi]);
        let shown = if end { n - hi..n - lo } else { lo..hi };
        let assumed = shown.clone().map(height).collect();
        // The sections in view, after the last one that starts above them: each with its row and that of the next.
        let first = inner.sections.partition_point(|&s| s < shown.start).saturating_sub(1);
        let sections: Vec<usize> = inner.sections[first..].iter().copied().take_while(|&s| s < shown.end).collect();
        let row_of = |s: usize| s.checked_sub(shown.start).filter(|&j| j < shown.len());
        let next = |k: usize| inner.sections.get(first + k + 1).copied().and_then(row_of);
        let sections: Vec<(Label, usize)> =
            sections.iter().enumerate().map(|(k, &s)| ((row_of(s), next(k), keys[s] ^ LABEL_KEY), s)).collect();
        let float = inner.float;
        drop(guard);
        let (labels, labeled): (Vec<Label>, Vec<Element<'a, M>>) = sections.into_iter().filter_map(|(l, s)| Some((l, label(s)?))).unzip();
        Element::new(Rows {
            list: self,
            end,
            keys: keys[shown.clone()].to_vec(),
            assumed,
            children: shown.map(&mut row).chain(labeled).collect(),
            labels,
            float,
            before: if end { far } else { near },
            after: if end { near } else { far },
            relist,
        })
    }

    /// Sets the rows that start the sections, as ascending indices of the keys of the next views, and how their labels
    /// float: `None` keeps them in place.
    pub fn sections(&self, starts: Vec<usize>, float: Option<Float>) {
        let mut inner = self.0.borrow_mut();
        (inner.sections, inner.float) = (starts, float);
    }

    /// The opacity of the label of the section that starts at the row of `key`, which fades in or out as it shows or
    /// hides.
    pub fn alpha(&self, key: u64) -> f32 {
        self.0.borrow().fades.get(&(key ^ LABEL_KEY)).map_or(1.0, |&(shown, at)| {
            let t = crate::style::cubic_bezier((at.elapsed().as_secs_f32() / FADE).min(1.0), [0.165, 0.84, 0.44, 1.0]);
            if shown {
                t
            } else {
                1.0 - t
            }
        })
    }

    /// The key of the row at the far edge of the last visible span, like the end of virtua's `onRangeChange`: the
    /// bottom row of a list anchored at its bottom, and the last row of one anchored at its top.
    pub fn last_seen(&self) -> Option<u64> {
        let inner = self.0.borrow();
        let (keys, end, _, starts) = &inner.starts;
        let (at, len) = inner.seen?;
        let n = keys.len();
        let at = at.clamp(0.0, (starts.get(n)? - len).max(0.0));
        match end {
            true => keys.get(n.checked_sub(1)?.checked_sub(starts[1..].partition_point(|&s| s <= at))?).copied(),
            false => keys.get(starts[..n].partition_point(|&s| s < at + len).checked_sub(1)?).copied(),
        }
    }

    /// `virtua`'s `scrollToIndex` with `align: "nearest"` in a list anchored at its top: the scroll offset that brings
    /// the row of `key`, or the first row for `None`, into the last visible span with the least scroll. `None` when
    /// the row shows already, or the last view did not have it.
    pub fn nearest(&self, key: Option<u64>) -> Option<f32> {
        let inner = self.0.borrow();
        let (keys, end, _, starts) = &inner.starts;
        assert!(!end, "a list anchored at its top");
        let k = if let Some(key) = key { keys.iter().position(|&k| k == key)? } else { 0 };
        let (top, bottom) = (*starts.get(k)?, *starts.get(k + 1)?);
        let (at, len) = inner.seen?;
        let at = at.clamp(0.0, (starts[keys.len()] - len).max(0.0));
        match (top < at, bottom > at + len) {
            (true, _) => Some(top),
            (false, true) => Some(bottom - len),
            (false, false) => None,
        }
    }

    /// Makes the next view build the row of `key` in the middle of the visible span.
    pub fn reveal(&self, key: u64) {
        self.0.borrow_mut().target = Some(key);
    }

    /// Drops the row that the next view was to reveal, when the list gets other rows.
    pub fn forget_reveal(&self) {
        self.0.borrow_mut().target = None;
    }
}

/// The layouts of the rows in the layout of `Rows`.
fn rows(layout: Layout<'_>) -> impl Iterator<Item = Layout<'_>> {
    layout.children().next().expect("the rows node").children()
}

/// A section's label: the built row of its section, that of the next section, and its key. A section above the built
/// rows starts above them, and one below them starts below the view.
type Label = (Option<usize>, Option<usize>, u64);

/// The rows that a view of a `VList` builds, under a spacer of `before` and over a spacer of `after`.
struct Rows<'a, M> {
    list: &'a VList,
    end: bool,
    keys: Vec<u64>,
    /// The heights that the view gave the rows.
    assumed: Vec<f32>,
    /// The rows, then the labels.
    children: Vec<Element<'a, M>>,
    labels: Vec<Label>,
    float: Option<Float>,
    before: f32,
    after: f32,
    relist: M,
}

impl<M> Rows<'_, M> {
    /// The keys of the rows, then those of the labels.
    fn all_keys(&self) -> Vec<u64> {
        self.keys.iter().copied().chain(self.labels.iter().map(|l| l.2)).collect()
    }

    /// Where each label goes under the top of the viewport, and whether it shows.
    fn placed(&self, layout: Layout<'_>, viewport: &Rectangle, scrolling: bool) -> Vec<(f32, bool)> {
        let tops: Vec<f32> = rows(layout).map(|r| r.bounds().y - viewport.y).collect();
        let top = |j: Option<usize>, none: f32| j.and_then(|j| tops.get(j).copied()).unwrap_or(none);
        let place = |&(row, next, _): &Label| {
            let (t, below) = (top(row, f32::NEG_INFINITY), top(next, f32::INFINITY));
            match self.float {
                Some(f) => ((t + 4.0).max(f.line).min(below - LABEL), scrolling || t >= f.pad - 20.0),
                None => (t + 4.0, true),
            }
        };
        self.labels.iter().map(place).collect()
    }
}

impl<M: Clone> Widget<M, Theme, Renderer> for Rows<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Vec<u64>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(self.all_keys())
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    /// Gives each row the state of the row with the same key, so that a row keeps its state when the rows move.
    fn diff(&self, tree: &mut Tree) {
        let keys = self.all_keys();
        let old = std::mem::replace(tree.state.downcast_mut::<Vec<u64>>(), keys.clone());
        let mut old: HashMap<u64, Tree> = old.into_iter().zip(std::mem::take(&mut tree.children)).collect();
        tree.children = (keys.iter().zip(&self.children))
            .map(|(k, child)| match old.remove(k) {
                Some(mut t) => {
                    t.diff(child.as_widget());
                    t
                }
                None => Tree::new(child.as_widget()),
            })
            .collect();
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let n = self.keys.len();
        let rows = flex::resolve(
            flex::Axis::Vertical,
            renderer,
            limits,
            Length::Fill,
            Length::Shrink,
            Padding::ZERO,
            0.0,
            Alignment::Start,
            &mut self.children[..n],
            &mut tree.children[..n],
        );
        let size = rows.size();
        // Each label in its place, 4 px into the row of its section, or above the rows.
        let wide = layout::Limits::new(Size::ZERO, Size::new(size.width, f32::INFINITY));
        let labels = (self.children[n..].iter_mut().zip(&mut tree.children[n..]).zip(&self.labels))
            .map(|((label, tree), &(row, _, _))| {
                let y = row.map_or(-LABEL, |j| rows.children()[j].bounds().y + 4.0);
                label.as_widget_mut().layout(tree, renderer, &wide).move_to((0.0, self.before + y))
            })
            .collect();
        let size = Size::new(size.width, size.height + self.before + self.after);
        layout::Node::with_children(size, vec![rows.move_to((0.0, self.before)), layout::Node::with_children(size, labels)])
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), layout) in self.children.iter_mut().zip(&mut tree.children).zip(rows(layout)) {
                child.as_widget_mut().operate(state, layout, renderer, operation);
            }
        });
    }

    fn update(
        &mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        for ((child, state), layout) in self.children.iter_mut().zip(&mut tree.children).zip(rows(layout)) {
            child.as_widget_mut().update(state, event, layout, cursor, renderer, clipboard, shell, viewport);
        }
        let Event::Window(window::Event::RedrawRequested(_)) = event else { return };
        let b = layout.bounds();
        let mut inner = self.list.0.borrow_mut();
        if inner.width != b.width {
            inner.width = b.width;
            inner.heights.clear();
            inner.stale = true;
        }
        let mut changed = false;
        let mut sum = 0.0;
        for ((k, assumed), row) in self.keys.iter().zip(&self.assumed).zip(rows(layout)) {
            let h = row.bounds().height;
            changed |= (h - assumed).abs() > 0.5;
            sum += h;
            inner.stale |= inner.heights.insert(*k, h) != Some(h);
        }
        if !self.keys.is_empty() {
            inner.typical = Some(sum / self.keys.len() as f32);
        }
        let top = viewport.y - b.y;
        let at = if self.end { b.height - top - viewport.height } else { top };
        let now = std::time::Instant::now();
        if inner.seen.is_some_and(|(old, _)| (old - at).abs() > 0.5) {
            inner.moved = Some(now);
            shell.request_redraw_at(now + SCROLL_IDLE);
        }
        inner.seen = Some((at, viewport.height));
        let scrolling = inner.moved.is_some_and(|t| now.duration_since(t) < SCROLL_IDLE);
        // A label that starts to show or hide fades, and the view builds it again at each step.
        let mut fading = false;
        for (&(_, _, key), (_, shows)) in self.labels.iter().zip(self.placed(layout, viewport, scrolling)) {
            match inner.fades.get(&key) {
                Some(&(old, at)) if old == shows => fading |= at.elapsed().as_secs_f32() < FADE,
                Some(_) => {
                    inner.fades.insert(key, (shows, now));
                    fading = true;
                }
                // A label that shows up hidden is hidden at once.
                None => {
                    inner.fades.insert(key, (shows, now.checked_sub(std::time::Duration::from_secs(1)).unwrap_or(now)));
                    fading |= !shows;
                }
            }
        }
        if fading {
            shell.request_redraw();
        }
        // `virtua`'s `onRangeChange` once the scroll stops, for the timeline to read through its last row in view.
        let settled = !scrolling && inner.moved.take().is_some();
        if changed || fading || settled || at < inner.safe.0 || at + viewport.height > inner.safe.1 {
            shell.publish(self.relist.clone());
        }
    }

    fn mouse_interaction(
        &self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer,
    ) -> mouse::Interaction {
        (self.children.iter().zip(&tree.children).zip(rows(layout)))
            .map(|((child, state), layout)| child.as_widget().mouse_interaction(state, layout, cursor, viewport, renderer))
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, state), layout) in self.children.iter().zip(&tree.children).zip(rows(layout)) {
            if layout.bounds().intersects(viewport) {
                child.as_widget().draw(state, renderer, theme, style, layout, cursor, viewport);
            }
        }
        let inner = self.list.0.borrow();
        let scrolling = inner.moved.is_some_and(|t| t.elapsed() < SCROLL_IDLE);
        let n = self.keys.len();
        let labels = layout.children().nth(1).expect("the labels node").children();
        let placed = self.labels.iter().zip(self.placed(layout, viewport, scrolling));
        for (((label, state), layout), (&(_, _, key), (y, shows))) in
            self.children[n..].iter().zip(&tree.children[n..]).zip(labels).zip(placed)
        {
            // A label that the view built before the list knew whether it shows waits for the next view.
            if shows || inner.fades.contains_key(&key) {
                let dy = Vector::new(0.0, viewport.y + y - layout.bounds().y);
                renderer.with_translation(dy, |r| label.as_widget().draw(state, r, theme, style, layout, cursor, &(*viewport - dy)));
            }
        }
    }

    fn overlay<'b>(
        &'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        let rows = layout.children().next().expect("the rows node");
        overlay::from_children(&mut self.children, tree, rows, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last row in view: the bottom one of a timeline anchored at its bottom, the last one of a list from the top.
    #[test]
    fn the_last_row_in_view() {
        let keys: Vec<u64> = (0..10).collect();
        for (end, at, want) in [(true, 0.0, 9), (true, 25.0, 8), (true, 45.0, 7), (false, 0.0, 2), (false, 25.0, 3)] {
            let list = VList::default();
            let _: Element<'_, ()> = list.view(&keys, end, 20.0, (), |_| iced::widget::Space::new().into());
            list.0.borrow_mut().seen = Some((at, 50.0));
            assert_eq!(list.last_seen(), Some(want), "anchored at the end: {end}, {at} px in");
        }
    }

    /// Like `virtua`'s `align: "nearest"`, a row above the visible span comes to its top, a row below it to its bottom,
    /// and a row in it does not scroll.
    #[test]
    fn nearest_scrolls_the_least() {
        let keys: Vec<u64> = (0..20).collect();
        let list = VList::default();
        let rows = list.view(&keys, false, 50.0, (), |_| iced::widget::Space::new().height(50).into());
        let view = iced::widget::scrollable(rows).height(Length::Fill);
        let mut sim = iced_test::Simulator::with_size(Default::default(), Size::new(400.0, 300.0), view);
        let _ = sim.simulate([Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
        drop(sim);
        assert_eq!(list.nearest(Some(3)), None, "row 3 shows in the first 300 px");
        assert_eq!(list.nearest(Some(6)), Some(50.0), "row 6 comes to the bottom");
        assert_eq!(list.nearest(Some(19)), Some(700.0), "the last row ends the list");
        assert_eq!(list.nearest(Some(99)), None, "a row that the list does not have");
        list.0.borrow_mut().seen = Some((400.0, 300.0));
        assert_eq!(list.nearest(Some(2)), Some(100.0), "row 2 comes to the top");
        assert_eq!(list.nearest(None), Some(0.0), "the first row comes to the top");
        assert_eq!(list.nearest(Some(10)), None);
        list.reveal(99);
        drop(list.view(&keys, false, 50.0, (), |_| iced::widget::Space::new().height(50).into()));
        assert_eq!(list.nearest(Some(2)), Some(100.0), "a reveal of a row that the list does not have keeps its place");
    }

    /// Desktop's `sticky-date-container`s, measured in Desktop 4.3.308 with a 36 px padding and the 48 px sticky top:
    /// a label sits 4 px into its section, at most 84 px under the top, and above the next section; while the list
    /// stands still, it hides once its section starts more than 20 px above the padding.
    #[test]
    fn labels_stick_and_hide_like_desktop() {
        // 20 rows of 50 px at the bottom of a 300 px viewport: row 14 starts at the top, row 15 50 px under it.
        let keys: Vec<u64> = (0..20).collect();
        fn color(s: usize) -> Color {
            Color::from_rgb8(10 * s as u8, 0, 0)
        }
        fn view<'a>(list: &'a VList, keys: &'a [u64]) -> Element<'a, UiMessage> {
            let bar = |s: usize| {
                let c = color(s).scale_alpha(list.alpha(keys[s]));
                let bar = iced::widget::container(iced::widget::Space::new().width(Length::Fill).height(LABEL));
                Some(bar.style(crate::style::fill_box(c, 0.0)).into())
            };
            let rows = list.labeled(keys, true, 50.0, UiMessage::Relist, |_| iced::widget::Space::new().height(50).into(), bar);
            iced::widget::scrollable(rows).anchor_bottom().height(Length::Fill).into()
        }
        let drawn = |sections: Vec<usize>| -> Vec<Option<u32>> {
            let list = VList::default();
            list.sections(sections.clone(), Some(Float { pad: 36.0, line: 84.0 }));
            let size = Size::new(400.0, 300.0);
            let mut sim = iced_test::Simulator::with_size(Default::default(), size, view(&list, &keys));
            let _ = sim.simulate([Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
            drop(sim);
            // The next view knows which labels hide.
            let mut sim = iced_test::Simulator::with_size(Default::default(), size, view(&list, &keys));
            let shot = crate::focus::tests::pixels(&mut sim, &iced::Theme::Light, "labels");
            let top = |s: usize| (0..shot.height()).find(|&y| shot.get_pixel(20, y).0[..3] == color(s).into_rgba8()[..3]).map(|y| y / 2);
            sections.into_iter().map(top).collect()
        };
        assert_eq!(drawn(vec![0, 15]), [None, Some(84)], "a day that starts 50 px down floats 84 px down, and the one above hides");
        assert_eq!(drawn(vec![0, 15, 16]), [None, Some(76), Some(104)], "the next day pushes it up");
        assert_eq!(drawn(vec![0, 17]), [None, Some(154)], "a day lower down shows in its place");
        assert_eq!(drawn(vec![14, 17]), [None, Some(154)], "a day that starts at the top hides, 16 px under the padding");
    }
}

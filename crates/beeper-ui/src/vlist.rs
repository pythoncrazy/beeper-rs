//! Desktop's `virtua` `VList` with `overscan: 15`: a list that builds only the rows in or near its viewport.
//!
//! The view gives each row a key. A `VList` keeps the measured height of each row and the visible span of the last
//! frame. A view builds the visible rows and `OVERSCAN` more rows on each side, and one spacer above and one spacer
//! below stand for the other rows. A frame that measures a new row height, or that moves the visible span out of
//! the inner half of the overscan, publishes the list message, so that the view builds the rows again.

use crate::prelude::*;
use iced::advanced::layout::{self, flex};
use iced::advanced::{mouse, overlay, renderer};
use iced::{window, Alignment};
use std::hash::{Hash, Hasher};

const OVERSCAN: usize = 15;
/// How long a list counts as scrolling after its visible span last moved, like `virtua`'s `isScrolling`.
const SCROLL_IDLE: std::time::Duration = std::time::Duration::from_millis(150);
/// The height of a section label, Desktop's date pill with its margin: a label floats while the next one is further
/// below the line.
const LABEL: f32 = 32.0;
/// How far above the line a section start may be for its label to float while the list stands still: Desktop hides a
/// date (`i + 20 < scrollOffset`) once its row is 20 px under the 36 px header, 64 px above its 48 px sticky top.
const IDLE: f32 = 64.0;
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
    /// How far below the top of the viewport a floating label goes.
    line: f32,
    /// The last section that starts above the line, whose own label the list covers.
    covered: Option<usize>,
    /// The start of the section whose label floats at the line.
    sticky: Option<usize>,
    /// When the visible span last moved.
    moved: Option<std::time::Instant>,
}

impl VList {
    /// The rows of `keys`, in display order, near the visible span. `end` tells that the scrollable anchors the
    /// list at its bottom. `estimate` is the height of a row before the first measurement. `row` builds a row from
    /// its index, and the list publishes `relist` when it must build other rows.
    pub fn view<'a, M: Clone + 'a>(
        &'a self, keys: &[u64], end: bool, estimate: f32, relist: M, mut row: impl FnMut(usize) -> Element<'a, M>,
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
        drop(guard);
        Element::new(Rows {
            list: self,
            end,
            base: shown.start,
            keys: keys[shown.clone()].to_vec(),
            assumed,
            children: shown.map(&mut row).collect(),
            before: if end { far } else { near },
            after: if end { near } else { far },
            relist,
        })
    }

    /// Sets the rows that start the sections, as ascending indices of the keys of the next views, and how far below
    /// the top of the viewport their labels float.
    pub fn sections(&self, starts: Vec<usize>, line: f32) {
        let mut inner = self.0.borrow_mut();
        (inner.sections, inner.line) = (starts, line);
    }

    /// The last section that starts above the line. Like Desktop, whose dates are all `sticky-date-container`s, its
    /// label shows only at the line, so the view leaves it out of its row.
    pub fn covered(&self) -> Option<usize> {
        self.0.borrow().covered
    }

    /// Desktop's `sticky-date-container`: the start of the covered section, whose label floats at the line while the
    /// list scrolls, or while its start is less than `IDLE` above the line. A frame where it or the covered section
    /// changes publishes the list message.
    pub fn sticky(&self) -> Option<usize> {
        self.0.borrow().sticky
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

/// The rows that a view of a `VList` builds, under a spacer of `before` and over a spacer of `after`.
struct Rows<'a, M> {
    list: &'a VList,
    end: bool,
    /// The index of the first built row in the keys of the view.
    base: usize,
    keys: Vec<u64>,
    /// The heights that the view gave the rows.
    assumed: Vec<f32>,
    children: Vec<Element<'a, M>>,
    before: f32,
    after: f32,
    relist: M,
}

impl<M: Clone> Widget<M, Theme, Renderer> for Rows<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Vec<u64>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(self.keys.clone())
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    /// Gives each row the state of the row with the same key, so that a row keeps its state when the rows move.
    fn diff(&self, tree: &mut Tree) {
        let old = std::mem::replace(tree.state.downcast_mut::<Vec<u64>>(), self.keys.clone());
        let mut old: HashMap<u64, Tree> = old.into_iter().zip(std::mem::take(&mut tree.children)).collect();
        tree.children = (self.keys.iter().zip(&self.children))
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
        let rows = flex::resolve(
            flex::Axis::Vertical,
            renderer,
            limits,
            Length::Fill,
            Length::Shrink,
            Padding::ZERO,
            0.0,
            Alignment::Start,
            &mut self.children,
            &mut tree.children,
        );
        let size = rows.size();
        let size = Size::new(size.width, size.height + self.before + self.after);
        layout::Node::with_children(size, vec![rows.move_to((0.0, self.before))])
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
        // How far below the line each built row starts. A row above the built ones starts above it.
        let tops: Vec<f32> = rows(layout).map(|r| r.bounds().y - viewport.y - inner.line).collect();
        let top = |row: usize| row.checked_sub(self.base).and_then(|j| tops.get(j)).copied();
        let above = |row: usize| top(row).map_or(row < self.base, |y| y < 0.0);
        let covered = inner.sections.iter().rposition(|&s| above(s));
        let sticky = covered.filter(|&k| {
            let next_clear = inner.sections.get(k + 1).and_then(|&s| top(s)).is_none_or(|y| y >= LABEL);
            next_clear && (scrolling || top(inner.sections[k]).is_some_and(|y| y >= -IDLE))
        });
        let (covered, sticky) = (covered.map(|k| inner.sections[k]), sticky.map(|k| inner.sections[k]));
        let sticky_changed =
            (std::mem::replace(&mut inner.sticky, sticky) != sticky) | (std::mem::replace(&mut inner.covered, covered) != covered);
        // `virtua`'s `onRangeChange` once the scroll stops, for the timeline to read through its last row in view.
        let settled = !scrolling && inner.moved.take().is_some();
        if changed || sticky_changed || settled || at < inner.safe.0 || at + viewport.height > inner.safe.1 {
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

    /// Like Desktop's dates, which all are `sticky-date-container`s, the label of a section that starts above the
    /// line shows only at the line: while the list stands still, only when its start is less than 64 px above the
    /// line, and never while the next label is less than a label below the line.
    #[test]
    fn labels_float_at_the_line_like_desktop() {
        // 20 rows of 50 px at the bottom of a 300 px viewport: rows 14 to 19 show, and row 15 starts 50 px down.
        let keys: Vec<u64> = (0..20).collect();
        let float = |sections: Vec<usize>| {
            let list = VList::default();
            list.sections(sections, 76.0);
            let rows = list.view(&keys, true, 50.0, (), |_| iced::widget::Space::new().height(50).into());
            let view = iced::widget::scrollable(rows).anchor_bottom().height(Length::Fill);
            let mut sim = iced_test::Simulator::with_size(Default::default(), Size::new(400.0, 300.0), view);
            let _ = sim.simulate([Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
            drop(sim);
            (list.covered(), list.sticky())
        };
        assert_eq!(float(vec![0, 16]), (Some(0), None), "a day that starts 24 px under the line keeps its label");
        assert_eq!(float(vec![0, 17]), (Some(0), None), "a day that starts far above hides its label");
        assert_eq!(float(vec![0, 15]), (Some(15), Some(15)), "26 px above the line, its label floats");
        assert_eq!(float(vec![0, 15, 16]), (Some(15), None), "the next label pushes it away");
        assert_eq!(float(vec![0, 14]), (Some(14), None), "76 px above the line, it hides");
    }
}

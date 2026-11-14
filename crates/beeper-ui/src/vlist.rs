//! Desktop's `virtua` `VList` with `overscan: 15`: a list that builds only the rows in or near its viewport.
//!
//! The view gives each row a key. A `VList` keeps the measured height of each row and the visible span of the last
//! frame. A view builds the visible rows and `OVERSCAN` more rows on each side, and one spacer above and one spacer
//! below stand for the other rows. A frame that measures a new row height, or that moves the visible span out of
//! the inner half of the overscan, publishes the list message, so that the view builds the rows again.

use iced::advanced::layout::{self, flex, Layout};
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::{window, Alignment, Element, Event, Length, Padding, Rectangle, Renderer, Size, Theme, Vector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const OVERSCAN: usize = 15;
/// The visible height that a list uses before a frame measures it.
const FIRST_SPAN: f32 = 1600.0;

/// The key of a row.
pub fn key(id: impl Hash) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

#[derive(Default)]
pub struct VList(RefCell<Inner>);

#[derive(Default)]
struct Inner {
    /// The width of the measured rows.
    width: f32,
    heights: HashMap<u64, f32>,
    /// The mean height of the rows of the last frame, for the rows that no frame measured.
    typical: Option<f32>,
    /// The visible span of the last frame: its distance from the anchored edge of the list, and its height.
    seen: Option<(f32, f32)>,
    /// The span from the anchored edge that the visible span can move in without a new view.
    safe: (f32, f32),
    /// The row that the next view must put in the middle of the visible span.
    target: Option<u64>,
}

impl VList {
    /// The rows of `keys`, in display order, near the visible span. `end` tells that the scrollable anchors the
    /// list at its bottom. `estimate` is the height of a row before the first measurement. `row` builds a row from
    /// its index, and the list publishes `relist` when it must build other rows.
    pub fn view<'a, M: Clone + 'a>(
        &'a self,
        keys: &[u64],
        end: bool,
        estimate: f32,
        relist: M,
        mut row: impl FnMut(usize) -> Element<'a, M>,
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
        let mut starts = Vec::with_capacity(n + 1);
        let mut y = 0.0;
        for k in 0..n {
            starts.push(y);
            y += height(index(k));
        }
        starts.push(y);
        let (mut at, len) = inner.seen.unwrap_or((0.0, FIRST_SPAN));
        if let Some(t) = target {
            let k = (0..n).find(|&k| keys[index(k)] == t).expect("the revealed row is in the list");
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
            keys: keys[shown.clone()].to_vec(),
            assumed,
            children: shown.map(&mut row).collect(),
            before: if end { far } else { near },
            after: if end { near } else { far },
            relist,
        })
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
        for ((child, state), layout) in self.children.iter_mut().zip(&mut tree.children).zip(rows(layout)) {
            child.as_widget_mut().update(state, event, layout, cursor, renderer, clipboard, shell, viewport);
        }
        let Event::Window(window::Event::RedrawRequested(_)) = event else { return };
        let b = layout.bounds();
        let mut inner = self.list.0.borrow_mut();
        if inner.width != b.width {
            inner.width = b.width;
            inner.heights.clear();
        }
        let mut changed = false;
        let mut sum = 0.0;
        for ((k, assumed), row) in self.keys.iter().zip(&self.assumed).zip(rows(layout)) {
            let h = row.bounds().height;
            changed |= (h - assumed).abs() > 0.5;
            sum += h;
            inner.heights.insert(*k, h);
        }
        if !self.keys.is_empty() {
            inner.typical = Some(sum / self.keys.len() as f32);
        }
        let top = viewport.y - b.y;
        let at = if self.end { b.height - top - viewport.height } else { top };
        inner.seen = Some((at, viewport.height));
        if changed || at < inner.safe.0 || at + viewport.height > inner.safe.1 {
            shell.publish(self.relist.clone());
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
        (self.children.iter().zip(&tree.children).zip(rows(layout)))
            .map(|((child, state), layout)| {
                child.as_widget().mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
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
        for ((child, state), layout) in self.children.iter().zip(&tree.children).zip(rows(layout)) {
            if layout.bounds().intersects(viewport) {
                child.as_widget().draw(state, renderer, theme, style, layout, cursor, viewport);
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        let rows = layout.children().next().expect("the rows node");
        overlay::from_children(&mut self.children, tree, rows, renderer, viewport, translation)
    }
}

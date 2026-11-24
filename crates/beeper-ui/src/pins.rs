//! Desktop's pinned chats grid: the hover of a tile, and the drag that orders the pins.

use super::*;
use crate::prelude::*;

/// `dnd-kit`'s `transform 250ms ease` of a sortable item that moves to another place.
pub(crate) const SLIDE: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub enum Msg {
    /// The pointer came onto (`Some`) or left a tile of the pinned grid.
    Hover(Option<String>),
    /// Desktop's `Drop` of the pinned grid (`lY`): a press, a move and the release of a drag of a pin.
    Press,
    DragMoved(Point),
    DragEnd,
}

impl BeeperGuiApp {
    pub(crate) fn pins(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Hover(id) => {
                // Desktop's sortable grid moves the dragged pin to the place of the pin under the pointer.
                let over = id.as_ref().and_then(|over| {
                    let d = self.pin_drag.as_mut().filter(|d| d.id != *over)?;
                    Some((over, d.order.as_mut()?, &d.id))
                });
                let moved = over.map(|(over, order, held)| {
                    let before = order.clone();
                    let from = order.iter().position(|o| o == held).expect("the dragged pin");
                    let to = order.iter().position(|o| o == over).expect("a pin of the grid");
                    let pin = order.remove(from);
                    order.insert(to, pin);
                    (before, order.clone(), held.clone())
                });
                if let Some((before, after, held)) = moved {
                    self.slide_pins(&before, &after, &held);
                }
                self.pin_hover = id;
            }
            Msg::Press => {
                let Some(id) = self.pin_hover.clone() else { return Task::none() };
                let at = self.cursor.get();
                self.pin_drag = Some(PinDrag { id, from: Some(at), at, order: None, start: 0 });
            }
            Msg::DragMoved(at) => {
                let Some(d) = self.pin_drag.as_mut() else { return Task::none() };
                d.at = at;
                if d.order.is_none() && at.distance(*d.from.get_or_insert(at)) > 5.0 {
                    let st = self.store.read();
                    let order: Vec<String> = self.pins_and_rows(&st).0.iter().map(|c| c.id.clone()).collect();
                    drop(st);
                    let d = self.pin_drag.as_mut().expect("a pin drag");
                    d.start = order.iter().position(|o| *o == d.id).unwrap_or(0);
                    d.order = Some(order);
                    self.list.chat_menu = None;
                }
            }
            Msg::DragEnd => {
                // `useSortable`'s transition: the pin glides from where the pointer left it into its new place.
                if let Some(ghost) = self.pin_ghost() {
                    let id = self.pin_drag.as_ref().expect("a pin drag").id.clone();
                    self.pin_slides.insert(id, (ghost, Instant::now()));
                }
                let Some(shown) = self.pin_drag.take().and_then(|d| d.order) else { return Task::none() };
                let order = self.store.write().set_pin_order(&shown);
                let Some(client) = self.store.read().matrix.clone() else { return Task::none() };
                let content = serde_json::json!({ "Inbox": order });
                let write = beeper_core::matrix::set_global(client, "com.beeper.favourites_order", content);
                return Task::perform(write, UiMessage::Pushed);
            }
        }
        self.settle()
    }
}

/// The text of Desktop's bubble on an unread pinned chat. A new message shows it, and after a minute it hides
/// when the chat has more unread messages than Desktop's limit for its size.
pub(crate) fn bubble_text(c: &Chat, launched: i64, now: i64) -> Option<String> {
    let unread = c.is_unread();
    let at = c.last_activity.as_deref().map_or(0, |t| millis_from_rfc3339(t).expect("RFC 3339 timestamp"));
    let limit = match c.participants.total {
        0..=2 => 3,
        3..=6 => 5,
        _ => 10,
    };
    if !unread || c.unread_count == 0 || (now - at.max(launched) >= 60_000 && c.unread_count > limit) {
        return None;
    }
    let m = c.preview.as_ref()?;
    let tag = || message_tag(m).map(|(_, l, d)| d.map_or_else(|| l.to_string(), |(d, _)| format!("{l} {d}")));
    let text = preview_body(m).or_else(tag)?;
    let first = preview_sender(c, m).map(sender_first_name);
    Some(first.map_or_else(|| text.clone(), |f| format!("{f}: {text}")))
}

/// Desktop's `PinnedThreadListItem` bubble: at most 8em wide, from 3px right of the avatar's top right corner,
/// centered on the avatar when wider than it, with a half disc below.
pub(crate) fn pin_bubble<'a>(text: &str, mention: bool, (bg, fg): (Color, Color), cell: f32, av: f32, compact: bool) -> El<'a> {
    let max = rem(80.0).min(cell - 4.0);
    let at_w = if mention { 14.0 } else { 0.0 };
    let w = (crate::style::text_width(text, 10.0, Weight::Normal) + at_w + 8.0).clamp(22.0, max);
    // `right: calc(50% - var(--pinned-avatar-size) / 2 - 3px)`, and the nub's `+ 3px`.
    let right = (cell + av) / 2.0;
    let x = right + 3.0 + ((w - av - 6.0) / 2.0).max(0.0) - w;
    // Its line of `--semantic-typo-line-height-label-xs` in rem, or 0.66rem in the compact list, in 4 px of padding.
    let (y, h) = if compact { (10.0, 8.0 + rem(10.56)) } else { (17.0, 8.0 + rem(12.0)) };
    let line = row![]
        .push(mention.then(|| ic(icon::AT_SIGN, 10.0).color(fg)))
        .push(ell(text, w - 8.0 - at_w, 10.0, Weight::Normal).color(fg).line_height(Pixels(h - 8.0)))
        .spacing(4)
        .align_y(Center);
    let bubble = container(line).padding(4).width(w).height(h).center_x(w).style(fill_box(bg, 6.0));
    let nub = container(Space::new()).width(6).height(3).style(move |_: &Theme| container::Style {
        background: Some(bg.into()),
        border: iced::Border { radius: iced::border::Radius::default().bottom(3.0), ..Default::default() },
        ..Default::default()
    });
    stack![pin(bubble).x(x).y(y - h / 2.0), pin(nub).x(right - 9.0).y(y + h / 2.0 - if compact { 0.28 } else { 0.0 })].into()
}

/// A press on a pin of the pinned grid, which Desktop's `Drop` sorts after the cursor moves 5px: the pin, where the
/// press was, where the cursor is, the order of the grid while the pin moves, and the place of the pin before it.
pub struct PinDrag {
    pub id: String,
    pub from: Option<Point>,
    pub at: Point,
    pub order: Option<Vec<String>>,
    pub start: usize,
}

/// Where the cell `i` of the pinned grid of `cols` columns of `cell` px and rows of `row` px is, from the first cell.
pub(crate) fn slot(i: usize, (cols, cell, row): (usize, f32, f32)) -> Vector {
    Vector::new((i % cols.max(1)) as f32 * cell, (i / cols.max(1)) as f32 * row)
}

impl BeeperGuiApp {
    /// How far a pin is drawn from its cell while it slides into it: CSS `ease` over [`SLIDE`].
    pub(crate) fn pin_offset(&self, id: &str) -> Option<Vector> {
        let (from, at) = self.pin_slides.get(id)?;
        let t = at.elapsed().as_secs_f32() / SLIDE.as_secs_f32();
        (t < 1.0).then(|| *from * (1.0 - crate::style::cubic_bezier(t, [0.25, 0.1, 0.25, 1.0])))
    }

    /// Whether a pin slides.
    pub(crate) fn pins_sliding(&self) -> bool {
        self.pin_slides.values().any(|(_, at)| at.elapsed() < SLIDE)
    }

    /// Where the dragged pin is drawn from the cell it would drop in: `dnd-kit` moves the dragged item with the
    /// pointer from its own cell.
    pub(crate) fn pin_ghost(&self) -> Option<Vector> {
        let d = self.pin_drag.as_ref()?;
        let order = d.order.as_ref()?;
        let to = order.iter().position(|o| *o == d.id)?;
        let grid = self.pin_grid.get();
        Some(d.at - d.from.unwrap_or(d.at) + slot(d.start, grid) - slot(to, grid))
    }

    /// `rectSortingStrategy`: the pins between the old and the new place of the dragged pin slide to their new cells.
    fn slide_pins(&mut self, before: &[String], after: &[String], held: &str) {
        let grid = self.pin_grid.get();
        let now = Instant::now();
        let moved: Vec<(String, Vector)> = after
            .iter()
            .enumerate()
            .filter(|(_, id)| *id != held)
            .filter_map(|(to, id)| {
                let from = before.iter().position(|b| b == id)?;
                let left = self.pin_offset(id).unwrap_or(Vector::ZERO);
                (from != to).then(|| (id.clone(), left + slot(from, grid) - slot(to, grid)))
            })
            .collect();
        self.pin_slides.retain(|_, (_, at)| at.elapsed() < SLIDE);
        self.pin_slides.extend(moved.into_iter().map(|(id, v)| (id, (v, now))));
    }
}

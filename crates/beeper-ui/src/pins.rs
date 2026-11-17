//! Desktop's pinned chats grid: the hover of a tile, and the drag that orders the pins.

use super::*;
use crate::chat_list::{message_tag, preview_body};

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
                if let Some((over, order, held)) = over {
                    let from = order.iter().position(|o| o == held).expect("the dragged pin");
                    let to = order.iter().position(|o| o == over).expect("a pin of the grid");
                    let pin = order.remove(from);
                    order.insert(to, pin);
                }
                self.pin_hover = id;
            }
            Msg::Press => {
                let Some(id) = self.pin_hover.clone() else { return Task::none() };
                self.pin_drag = Some(PinDrag { id, from: None, order: None });
            }
            Msg::DragMoved(at) => {
                let Some(d) = self.pin_drag.as_mut() else { return Task::none() };
                if d.order.is_none() && at.distance(*d.from.get_or_insert(at)) > 5.0 {
                    let st = self.store.read();
                    let order = self.pins_and_rows(&st).0.iter().map(|c| c.id.clone()).collect();
                    drop(st);
                    self.pin_drag.as_mut().expect("a pin drag").order = Some(order);
                    self.chat_menu = None;
                }
            }
            Msg::DragEnd => {
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
pub(crate) fn pin_bubble<'a>(
    text: &str,
    mention: bool,
    (bg, fg): (Color, Color),
    cell: f32,
    av: f32,
    compact: bool,
) -> El<'a> {
    let max = 80f32.min(cell - 4.0);
    let at_w = if mention { 14.0 } else { 0.0 };
    let w = (crate::style::text_width(text, 10.0, Weight::Normal) + at_w + 8.0).clamp(22.0, max);
    let right = (cell + av - 4.0) / 2.0;
    let x = right + 3.0 + ((w - av - 6.0) / 2.0).max(0.0) - w;
    let (y, h) = if compact { (10.0, 18.56) } else { (17.0, 20.0) };
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
    stack![pin(bubble).x(x).y(y - h / 2.0), pin(nub).x(right - 9.0).y(y + if compact { 9.0 } else { 10.0 })].into()
}

/// A press on a pin of the pinned grid, which Desktop's `Drop` sorts after the cursor moves 5px: the pin, where the
/// cursor first moved, and the order of the grid while the pin moves.
pub struct PinDrag {
    pub id: String,
    pub from: Option<Point>,
    pub order: Option<Vec<String>>,
}

//! Desktop's keyboard reordering of its `dnd-kit` sortables: the `KeyboardSensor` with
//! `sortableKeyboardCoordinates` (App:59318-59460, 61099-61140). Enter or Space on a focused item picks it up, an
//! arrow moves it over the nearest item that way, Enter, Space or Tab drops it, and Escape puts it back. The drop
//! saves the order like a drop of the mouse.
//!
//! Desktop also announces the drag to screen readers ("Picked up draggable item …"); iced has no accessibility
//! tree, so beeper-rs does not.

use super::*;
use std::sync::Arc;

/// The sortable lists of Desktop that the keyboard reorders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    /// The space bar (`y0t`, a `Drop` with a drag overlay).
    Rail,
    /// The grid of pinned chats (`lY`, a `Drop` restricted to its container).
    Pins,
    /// The accounts of Settings (`ZLt`, a `Drop` restricted to its container).
    Accounts,
    /// The handles of Edit Spaces (`Oft`, with the Hidden divider as a sortable that is disabled).
    Spaces,
}

/// A press of Enter or Space on a focused sortable: its window, its list, the item, the items of the list as it
/// shows them, and the columns of the grid (1 for a list).
#[derive(Clone)]
pub struct Pick {
    pub window: window::Id,
    pub list: List,
    pub id: String,
    pub items: Arc<[String]>,
    pub cols: usize,
}

#[derive(Clone)]
pub enum Msg {
    /// Enter or Space on the focused item (`keyboardCodes.start`).
    Pick(Box<Pick>),
    /// An arrow key.
    Move(Named),
    /// Enter, Space or Tab (`keyboardCodes.end`).
    Drop,
    /// Escape (`keyboardCodes.cancel`), a resize of the window, or its close.
    Cancel,
}

/// The item that the keyboard moves: the items of the list at the pick up, the slot where the item is, and the item
/// it is over, which is where it drops.
pub struct KeySort {
    pub window: window::Id,
    pub list: List,
    pub id: String,
    items: Arc<[String]>,
    cols: usize,
    at: usize,
    over: Option<usize>,
}

impl KeySort {
    fn new(p: Pick) -> Option<Self> {
        let at = p.items.iter().position(|i| *i == p.id)?;
        let mut s = KeySort { window: p.window, list: p.list, id: p.id, items: p.items, cols: p.cols.max(1), at, over: None };
        // `closestCenter` over the droppables: the item itself, or the nearest other one when the item is disabled.
        s.over = s.nearest(|_| true).first().copied();
        Some(s)
    }

    /// Whether the item at `i` is a droppable. The space bar disables the dragged space (`disabled: !c && h ===
    /// M.id` of `Drop`), and Edit Spaces its Hidden divider.
    fn droppable(&self, i: usize) -> bool {
        match self.list {
            List::Rail => self.items[i] != self.id,
            List::Spaces => self.items[i] != rail::SPACES_DIVIDER,
            List::Pins | List::Accounts => true,
        }
    }

    fn cell(&self, i: usize) -> (f32, f32) {
        ((i % self.cols) as f32, (i / self.cols) as f32)
    }

    /// The droppables for which `keep` holds, nearest to the slot first, and in their order between equals.
    fn nearest(&self, keep: impl Fn((f32, f32)) -> bool) -> Vec<usize> {
        let (x, y) = self.cell(self.at);
        let mut found: Vec<(usize, f32)> = (0..self.items.len())
            .filter(|&i| self.droppable(i) && keep(self.cell(i)))
            .map(|i| (i, (self.cell(i).0 - x).powi(2) + (self.cell(i).1 - y).powi(2)))
            .collect();
        found.sort_by(|a, b| a.1.total_cmp(&b.1));
        found.into_iter().map(|f| f.0).collect()
    }

    /// `sortableKeyboardCoordinates`: the arrow moves the item onto the nearest droppable whose corner is past the
    /// slot that way (`closestCorners`), or the next nearest when that one is the item it is over.
    fn step(&mut self, key: Named) {
        let (x, y) = self.cell(self.at);
        let ahead = |(cx, cy): (f32, f32)| match key {
            Named::ArrowDown => cy > y,
            Named::ArrowUp => cy < y,
            Named::ArrowLeft => cx < x,
            Named::ArrowRight => cx > x,
            _ => false,
        };
        let found = self.nearest(ahead);
        let to = match found.as_slice() {
            [first, second, ..] if Some(*first) == self.over => Some(*second),
            _ => found.first().copied(),
        };
        if let Some(to) = to {
            (self.at, self.over) = (to, Some(to));
        }
    }

    /// The item it drops over, when that is another item.
    fn target(&self) -> Option<&str> {
        self.over.map(|i| self.items[i].as_str()).filter(|o| *o != self.id)
    }

    /// `arrayMove` of `list` from the item to the item it is over, which the list shows while the item moves and
    /// keeps on the drop. `None` when the item is over itself, or when the list no longer has them.
    pub fn moved(&self, list: &[String]) -> Option<Vec<String>> {
        let over = self.target()?;
        let from = list.iter().position(|i| *i == self.id)?;
        let to = list.iter().position(|i| i == over)?;
        let mut out = list.to_vec();
        let item = out.remove(from);
        out.insert(to, item);
        Some(out)
    }

    /// The control that Desktop's drag overlay hides: the dragged space of the space bar and the handle of Edit
    /// Spaces stay in place with no opacity while a copy moves, so their focus ring does not show.
    pub fn hides(&self, key: &str) -> bool {
        match self.list {
            List::Rail => key.strip_prefix("space-") == Some(self.id.as_str()),
            List::Spaces => key.strip_prefix("handle-") == Some(self.id.as_str()),
            List::Pins | List::Accounts => false,
        }
    }
}

/// What a key does to the item that the keyboard moves: Enter and Tab drop it (Shift+Tab too), Escape puts it back,
/// and the arrows move it. Space comes as text, and drops it too.
pub(crate) fn key_msg(key: &Key, mods: Modifiers) -> Option<Msg> {
    if !(mods - Modifiers::SHIFT).is_empty() {
        return None;
    }
    match key {
        Key::Named(Named::Enter | Named::Tab) => Some(Msg::Drop),
        Key::Named(Named::Escape) => Some(Msg::Cancel),
        Key::Named(k @ (Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight)) => Some(Msg::Move(*k)),
        _ => None,
    }
}

impl BeeperGuiApp {
    /// The keyboard sort of `list` in `window`, if one moves an item.
    pub(crate) fn key_sort_of(&self, window: Option<window::Id>, list: List) -> Option<&KeySort> {
        self.key_sort.as_ref().filter(|s| s.list == list && Some(s.window) == window)
    }

    /// The message of Enter and Space on the item `id` of `list` in `window`, whose items show as `items`.
    pub(crate) fn pick(window: Option<window::Id>, list: List, id: &str, items: &Arc<[String]>, cols: usize) -> Option<UiMessage> {
        let (id, items) = (id.to_string(), items.clone());
        window.map(|window| UiMessage::Sort(Msg::Pick(Box::new(Pick { window, list, id, items, cols }))))
    }

    pub(crate) fn sortable(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Pick(p) => self.key_sort = KeySort::new(*p),
            Msg::Move(k) => {
                if let Some(s) = self.key_sort.as_mut() {
                    s.step(k);
                }
            }
            Msg::Cancel => self.key_sort = None,
            Msg::Drop => {
                let Some(s) = self.key_sort.take() else { return Task::none() };
                return self.drop_sorted(&s);
            }
        }
        self.settle()
    }

    /// Desktop's `onDragEnd` of each list, through the drop of the mouse.
    fn drop_sorted(&mut self, s: &KeySort) -> Task<UiMessage> {
        match s.list {
            // `Drop`'s `onSort` and `onDrop` save only a move.
            List::Rail => {
                let Some(order) = s.moved(&s.items) else { return self.settle() };
                let at = Point::ORIGIN;
                self.rail_drag = Some(RailDrag { id: s.id.clone(), grab: at, from: None, at, order: Some(order) });
                self.rail(rail::Msg::DragEnd)
            }
            List::Pins => {
                let Some(order) = s.moved(&s.items) else { return self.settle() };
                self.pin_drag = Some(PinDrag { id: s.id.clone(), from: None, at: Point::ORIGIN, order: Some(order), start: 0 });
                self.pins(pins::Msg::DragEnd)
            }
            // `ZLt`'s `setAccountsOrder` and `persistAccountsOrder`: the place of each account in the list.
            List::Accounts => {
                if let Some(order) = s.moved(&s.items) {
                    self.patch_settings(|st| st.accounts_order = order.into_iter().zip(0..).collect());
                }
                self.settle()
            }
            // `Oft`'s `onDragEnd` moves the space in the whole order, then saves it like a drop of the mouse, and
            // focuses the window when the order changed.
            List::Spaces => {
                let Some(d) = self.spaces_window.as_mut().filter(|d| d.id == s.window) else { return self.settle() };
                let Some(order) = s.moved(&d.order) else { return self.settle() };
                let before = std::mem::replace(&mut d.order, order);
                d.drag = Some((s.id.clone(), before.clone()));
                let task = self.spaces_edit(SpacesEdit::Drop);
                if self.spaces_window.as_ref().is_some_and(|d| d.order != before) {
                    self.key_targets.remove(&s.window);
                }
                task
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::tests::{ask, held, ids, shot, survey_of};
    use crate::focus::Ask;
    use crate::fuzz::demo_app;

    fn key(app: &mut BeeperGuiApp, window: window::Id, k: Named) {
        let _ = app.update(UiMessage::WindowKey(window, Key::Named(k), Modifiers::empty()));
    }

    fn focus(app: &mut BeeperGuiApp, window: window::Id, key: &str) {
        app.key_targets.insert(window, iced::widget::Id::from(key.to_string()));
    }

    /// The pixels of Chromium's focus ring in `view` left of `x`.
    fn rings(view: El<'_>, size: Size, x: u32, name: &str) -> usize {
        let shot = shot(view, size, name);
        let k = shot.width() / size.width as u32;
        shot.enumerate_pixels().filter(|(px, _, c)| *px < x * k && c.0 == [0xe5, 0x97, 0x00, 255]).count()
    }

    /// `list` with the item at `from` moved to `to`.
    fn moved(list: &[String], from: usize, to: usize) -> Vec<String> {
        let mut out = list.to_vec();
        let item = out.remove(from);
        out.insert(to, item);
        out
    }

    /// Desktop's space bar: Enter on a focused space picks it up, and its ring hides with it under the drag overlay.
    /// ↓ moves it below the next space, and Space drops it there, saves `spaces_order` and keeps the focus on it.
    /// Escape puts a space back. The dragged space is no droppable, so Enter right after the pick up drops it over the
    /// space above it.
    #[test]
    fn the_space_bar_reorders_from_the_keyboard_like_desktop() {
        let mut app = demo_app("sort-rail");
        let main = app.main_window.expect("the main window");
        app.pane.selected_chat_id = None;
        let size = Size::new(1200.0, 800.0);
        let rail = |app: &BeeperGuiApp| rail::rail_spaces(&app.store.read());
        let before = rail(&app);
        let second = format!("space-{}", before[1]);
        focus(&mut app, main, &second);
        assert!(rings(app.view_main(), size, 68, "held") > 0, "the focused space shows the ring");
        ask(&mut app, main, size, None, Ask::Press(true));
        assert!(app.key_sort.is_some(), "Enter picks the space up");
        assert_eq!(rings(app.view_main(), size, 68, "lifted"), 0, "the lifted space hides its ring");
        key(&mut app, main, Named::ArrowDown);
        assert_eq!(rail(&app), before, "the order holds until the drop");
        let _ = app.update(UiMessage::WindowText(main, " ".into()));
        let want = moved(&before, 1, 2);
        assert_eq!((rail(&app), app.key_sort.is_none()), (want.clone(), true), "Space drops it below the next one");
        assert_eq!(held(&app, main), Some(second), "and it keeps the focus");
        assert!(rings(app.view_main(), size, 68, "dropped") > 0, "with its ring");
        ask(&mut app, main, size, None, Ask::Press(false));
        assert!(app.key_sort.is_some(), "Space picks it up too");
        key(&mut app, main, Named::ArrowUp);
        key(&mut app, main, Named::Escape);
        assert_eq!((rail(&app), app.key_sort.is_none()), (want, true), "Escape puts it back");
        ask(&mut app, main, size, None, Ask::Press(true));
        key(&mut app, main, Named::Enter);
        assert_eq!(rail(&app), before, "Enter at once drops it over the space above");
    }

    /// Desktop's pinned grid: → moves a picked pin over the pin right of it, which the drop saves as the pin order.
    #[test]
    fn the_pinned_grid_reorders_from_the_keyboard_like_desktop() {
        let mut app = demo_app("sort-pins");
        let main = app.main_window.expect("the main window");
        app.pane.selected_chat_id = None;
        let size = Size::new(1200.0, 800.0);
        let pins = |app: &BeeperGuiApp| -> Vec<String> {
            let st = app.store.read();
            app.pins_and_rows(&st).0.iter().map(|c| c.id.clone()).collect()
        };
        let before = pins(&app);
        assert!(before.len() >= 2, "the demo has pins: {before:?}");
        let first = format!("pin-{}", before[0]);
        assert!(ids(&survey_of(&app, main, size, None)).contains(&first), "Tab reaches the pins");
        focus(&mut app, main, &first);
        ask(&mut app, main, size, None, Ask::Press(true));
        key(&mut app, main, Named::ArrowRight);
        let order: Vec<String> = ids(&survey_of(&app, main, size, None)).into_iter().filter(|s| s.starts_with("pin-")).collect();
        assert_eq!(order[1], first, "the pin shows in the place it drops at");
        key(&mut app, main, Named::Enter);
        assert_eq!(pins(&app), moved(&before, 0, 1), "Enter drops it after the next pin");
        assert_eq!(held(&app, main), Some(first), "and it keeps the focus");
    }

    /// Desktop's accounts in Settings: ↓ moves a picked account below the next one, at 40 % opacity, and Tab drops it
    /// and saves `accounts_order`, which orders the list.
    #[test]
    fn settings_accounts_reorder_from_the_keyboard_like_desktop() {
        let mut app = demo_app("sort-accounts");
        let _ = app.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts)));
        let id = app.settings_window.expect("the Settings window");
        let accounts = |app: &BeeperGuiApp| -> Vec<String> {
            let stops = ids(&survey_of(app, id, SETTINGS_SIZE, None));
            stops.into_iter().filter(|s| s.starts_with("account-")).collect()
        };
        let before = accounts(&app);
        assert!(before.len() >= 2 && before[0] == "account-hungryserv", "Beeper, then the bridges: {before:?}");
        focus(&mut app, id, &before[0]);
        let plain = shot(app.view_settings(), SETTINGS_SIZE, "accounts-plain");
        ask(&mut app, id, SETTINGS_SIZE, None, Ask::Press(false));
        key(&mut app, id, Named::ArrowDown);
        let want = moved(&before, 0, 1);
        assert_eq!(accounts(&app), want, "the account shows below the next one");
        let lifted = shot(app.view_settings(), SETTINGS_SIZE, "accounts-lifted");
        let faded = plain.pixels().zip(lifted.pixels()).filter(|(a, b)| a != b).count();
        assert!(faded > 1000, "the moved account fades: {faded}");
        key(&mut app, id, Named::Tab);
        assert!(app.key_sort.is_none(), "Tab drops it");
        assert_eq!(accounts(&app), want, "the list keeps the order");
        let order = &app.store.read().settings.accounts_order;
        assert_eq!((order["hungryserv"], order.len()), (1, before.len()), "{order:?}");
        assert_eq!(held(&app, id).as_deref(), Some("account-hungryserv"), "and the account keeps the focus");
    }

    /// Desktop's Edit Spaces: Escape puts a picked space back with the focus on its handle, and a drop that moves it
    /// saves the order and gives the focus to the window.
    #[test]
    fn edit_spaces_reorders_from_the_keyboard_like_desktop() {
        let mut app = demo_app("sort-spaces");
        let _ = app.update(UiMessage::OpenEditSpaces);
        let (sheet, before) = {
            let d = app.spaces_window.as_ref().expect("the Edit Spaces window");
            (d.id, d.order.clone())
        };
        let size = rail::SPACES_SIZE;
        let handles = |app: &BeeperGuiApp| -> Vec<String> {
            let stops = ids(&survey_of(app, sheet, size, None));
            stops.into_iter().filter(|s| s.starts_with("handle-")).collect()
        };
        let view = |app: &BeeperGuiApp, name: &str| {
            rings(app.view_edit_spaces(app.spaces_window.as_ref().unwrap()), size, size.width as u32, name)
        };
        let first = format!("handle-{}", before[0]);
        focus(&mut app, sheet, &first);
        assert!(view(&app, "held") > 0, "the handle shows the ring");
        ask(&mut app, sheet, size, None, Ask::Press(true));
        key(&mut app, sheet, Named::ArrowDown);
        assert_eq!(handles(&app)[1], first, "the space shows below the next one");
        assert_eq!(view(&app, "lifted"), 0, "and its handle hides the ring");
        key(&mut app, sheet, Named::Escape);
        assert_eq!(app.spaces_window.as_ref().unwrap().order, before, "Escape puts it back");
        assert_eq!(held(&app, sheet), Some(first.clone()), "with the focus on its handle");
        ask(&mut app, sheet, size, None, Ask::Press(true));
        key(&mut app, sheet, Named::ArrowDown);
        key(&mut app, sheet, Named::Enter);
        let saved: Vec<String> = app.store.read().settings.spaces_order.iter().map(|o| o.id.clone()).collect();
        assert_eq!(saved[..2], [before[1].clone(), before[0].clone()], "Enter saves the move");
        assert_eq!(held(&app, sheet), None, "and the window takes the focus");
    }

    fn sort(list: List, items: &[&str], id: &str, cols: usize) -> KeySort {
        let items: Arc<[String]> = items.iter().map(|s| s.to_string()).collect();
        let window = window::Id::unique();
        KeySort::new(Pick { window, list, id: id.into(), items, cols }).expect("the item is in the list")
    }

    fn order(s: &KeySort) -> String {
        s.moved(&s.items).unwrap_or_else(|| s.items.to_vec()).concat()
    }

    /// A list restricted to its container keeps the item droppable: ↓ and ↑ move it one place, it can go back to its
    /// own place, and ← and → find nothing in a column.
    #[test]
    fn a_list_moves_its_item_one_place_per_arrow_like_dnd_kit() {
        let mut s = sort(List::Accounts, &["a", "b", "c", "d"], "b", 1);
        assert_eq!(order(&s), "abcd", "over itself at the pick up");
        s.step(Named::ArrowDown);
        assert_eq!(order(&s), "acbd");
        s.step(Named::ArrowDown);
        s.step(Named::ArrowDown);
        assert_eq!(order(&s), "acdb", "the last place holds");
        s.step(Named::ArrowRight);
        assert_eq!(order(&s), "acdb", "→ finds nothing in a column");
        for _ in 0..2 {
            s.step(Named::ArrowUp);
        }
        assert_eq!(order(&s), "abcd", "back over itself");
        s.step(Named::ArrowUp);
        assert_eq!(order(&s), "bacd");
    }

    /// The space bar disables the dragged space, so it starts over the nearest space, the one above it, and the
    /// arrows skip its own place.
    #[test]
    fn the_space_bar_skips_the_dragged_space_like_desktop() {
        let mut s = sort(List::Rail, &["a", "b", "c", "d"], "b", 1);
        assert_eq!(order(&s), "bacd", "over the space above at the pick up");
        s.step(Named::ArrowDown);
        assert_eq!(order(&s), "acbd", "↓ goes over c");
        s.step(Named::ArrowUp);
        assert_eq!(order(&s), "bacd", "↑ skips its own place");
        let first = sort(List::Rail, &["a", "b"], "a", 1);
        assert_eq!(order(&first), "ba", "the first space starts over the one below");
    }

    /// Edit Spaces disables its Hidden divider: ↓ from the last pinned space goes over the first hidden one.
    #[test]
    fn edit_spaces_skips_the_divider_like_desktop() {
        let d = rail::SPACES_DIVIDER;
        let mut s = sort(List::Spaces, &["a", "b", d, "c", "e"], "b", 1);
        s.step(Named::ArrowDown);
        let full: Vec<String> = ["a", "b", d, "c", "e"].map(String::from).to_vec();
        assert_eq!(s.moved(&full).expect("a move"), ["a", d, "c", "b", "e"], "b goes after c");
        let mut alone = sort(List::Spaces, &["a", "b", d], "b", 1);
        alone.step(Named::ArrowDown);
        assert_eq!(alone.moved(&full), None, "nothing to go over below the divider");
    }

    /// The pinned grid moves the pin by rows with ↑ and ↓ and by columns with ← and →, onto the nearest pin.
    #[test]
    fn the_pinned_grid_moves_by_rows_and_columns_like_dnd_kit() {
        let mut s = sort(List::Pins, &["a", "b", "c", "d", "e"], "b", 3);
        s.step(Named::ArrowDown);
        assert_eq!(order(&s), "acdeb", "↓ from the middle goes over e, below b");
        s.step(Named::ArrowLeft);
        assert_eq!(order(&s), "acdbe", "← goes over d");
        s.step(Named::ArrowUp);
        assert_eq!(order(&s), "bacde", "↑ goes over a, above d");
        s.step(Named::ArrowRight);
        assert_eq!(order(&s), "abcde", "→ comes back over itself");
        s.step(Named::ArrowRight);
        assert_eq!(order(&s), "acbde", "→ goes over c");
        s.step(Named::ArrowDown);
        assert_eq!(order(&s), "acdeb", "↓ from c goes over e, the nearest pin below");
        s.step(Named::ArrowRight);
        assert_eq!(order(&s), "acbde", "→ finds c in the row above, the only pin right of e");
    }
}

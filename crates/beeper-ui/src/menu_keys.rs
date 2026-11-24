//! The keyboard of Desktop's native context menus, which are Chromium's views menus on Linux (`MenuController`):
//! ↓ and ↑ move the selection through the enabled items and wrap around, the first ↓ (↑) selects the first (last)
//! item of a menu that the mouse opened, Home and End select the first and the last item, Enter runs the selected
//! item, and the pointer over an item selects it.

use super::*;
use crate::prelude::*;
use iced::advanced::widget::{self as advanced};
use iced::advanced::{mouse, renderer};

/// A row of a menu, whose message is its action, or none when it is greyed out.
pub fn row<'a>(content: impl Into<El<'a>>, msg: Option<UiMessage>) -> El<'a> {
    crate::delegate::marked(content, None, Mark(msg))
}

/// Desktop's `.menu-item .hotkey`: `--color-text-translucent-subtle` (`dim`), and `--color-text-on-accent-weak`
/// on the selected row, which is the row whose button gives its content white text.
pub fn hotkey<'a>(content: impl Into<El<'a>>, dim: Color) -> El<'a> {
    El::new(Hotkey { content: content.into(), dim })
}

struct Hotkey<'a> {
    content: El<'a>,
    dim: Color,
}

impl Widget<UiMessage, Theme, Renderer> for Hotkey<'_> {
    crate::delegate::delegate!(children, diff, size, layout, operate);

    fn draw(
        &self, tree: &advanced::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        let text_color = if style.text_color == Color::WHITE { Color::WHITE.scale_alpha(0.7) } else { self.dim };
        self.content.as_widget().draw(&tree.children[0], renderer, theme, &renderer::Style { text_color }, layout, cursor, viewport);
    }
}

/// A menu whose rows the keyboard selects and runs.
pub fn menu<'a>(content: impl Into<El<'a>>) -> El<'a> {
    El::new(Menu { content: content.into() })
}

/// The mark of a [`row`] for the operations.
#[derive(Clone)]
struct Mark(Option<UiMessage>);

/// The operation that collects the rows of a menu in their order, with their bounds and actions.
#[derive(Default)]
struct Rows(Vec<(Rectangle, Option<UiMessage>)>);

impl Operation for Rows {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self)
    }

    fn custom(&mut self, _: Option<&iced::widget::Id>, bounds: Rectangle, state: &mut dyn std::any::Any) {
        if let Some(Mark(msg)) = state.downcast_ref() {
            self.0.push((bounds, msg.clone()));
        }
    }
}

struct Menu<'a> {
    content: El<'a>,
}

/// The selected row, and the middle of the row that the keyboard selected, which draws it as the pointer would.
#[derive(Default)]
struct Selection {
    at: Option<usize>,
    keyed: Option<Point>,
}

/// The index of the enabled row that `key` selects out of `rows` from the selected one `at`: Chromium's
/// `IncrementSelection`, which wraps around, and `MoveSelectionToFirstOrLastItem` of Home and End.
pub(crate) fn step(enabled: &[bool], at: Option<usize>, key: Named) -> Option<usize> {
    let n = enabled.len();
    let first = || enabled.iter().position(|e| *e);
    let last = || enabled.iter().rposition(|e| *e);
    match (key, at) {
        (Named::Home, _) | (Named::ArrowDown, None) => first(),
        (Named::End, _) | (Named::ArrowUp, None) => last(),
        (Named::ArrowDown, Some(i)) => (1..n).map(|d| (i + d) % n).find(|&j| enabled[j]).or(Some(i)),
        (Named::ArrowUp, Some(i)) => (1..n).map(|d| (i + n - d) % n).find(|&j| enabled[j]).or(Some(i)),
        _ => at,
    }
}

impl Menu<'_> {
    fn rows(&mut self, tree: &mut advanced::Tree, layout: Layout<'_>, renderer: &Renderer) -> Rows {
        let mut rows = Rows::default();
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut rows);
        rows
    }
}

impl Widget<UiMessage, Theme, Renderer> for Menu<'_> {
    crate::delegate::delegate!(state(Selection), children, diff, size, layout, operate, mouse_interaction, overlay(UiMessage));

    fn update(
        &mut self, tree: &mut advanced::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, UiMessage>, viewport: &Rectangle,
    ) {
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(k), modifiers, .. })
                if modifiers.is_empty() && matches!(k, Named::ArrowDown | Named::ArrowUp | Named::Home | Named::End | Named::Enter) =>
            {
                let rows = self.rows(tree, layout, renderer).0;
                if rows.is_empty() {
                    return;
                }
                let sel = tree.state.downcast_mut::<Selection>();
                if *k == Named::Enter {
                    if let Some(msg) = sel.at.and_then(|i| rows.get(i)).and_then(|r| r.1.clone()) {
                        shell.publish(msg);
                    }
                } else {
                    let enabled: Vec<bool> = rows.iter().map(|r| r.1.is_some()).collect();
                    sel.at = step(&enabled, sel.at.filter(|&i| i < rows.len()), *k);
                    sel.keyed = sel.at.map(|i| rows[i].0.center());
                    shell.request_redraw();
                }
                shell.capture_event();
                return;
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let rows = self.rows(tree, layout, renderer).0;
                let over = cursor.position().and_then(|at| rows.iter().position(|r| r.0.contains(at)));
                let sel = tree.state.downcast_mut::<Selection>();
                // Chromium's hot tracking: the pointer selects the enabled row under it.
                if let Some(i) = over.filter(|&i| rows[i].1.is_some()) {
                    (sel.at, sel.keyed) = (Some(i), None);
                } else if sel.keyed.is_none() {
                    sel.at = None;
                }
            }
            _ => {}
        }
        // The buttons take their hover look from the cursor of the redraw, which the keyboard's row gets; the mouse
        // events keep the real cursor.
        let keyed = tree.state.downcast_ref::<Selection>().keyed;
        let redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        let cursor = keyed.filter(|_| redraw).map_or(cursor, mouse::Cursor::Available);
        let content = &mut tree.children[0];
        self.content.as_widget_mut().update(content, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self, tree: &advanced::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        // A row that the keyboard selected draws as the pointer over it draws it.
        let keyed = tree.state.downcast_ref::<Selection>().keyed;
        let cursor = keyed.map_or(cursor, mouse::Cursor::Available);
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator::simulator;

    /// Chromium's `IncrementSelection` wraps past the greyed-out rows, and Home and End go to the ends.
    #[test]
    fn the_arrows_step_like_chromium() {
        let enabled = [true, false, true, true, false];
        assert_eq!(step(&enabled, None, Named::ArrowDown), Some(0), "the first ↓ selects the first row");
        assert_eq!(step(&enabled, None, Named::ArrowUp), Some(3), "the first ↑ selects the last enabled row");
        assert_eq!(step(&enabled, Some(0), Named::ArrowDown), Some(2), "a greyed-out row is skipped");
        assert_eq!(step(&enabled, Some(3), Named::ArrowDown), Some(0), "↓ wraps around");
        assert_eq!(step(&enabled, Some(0), Named::ArrowUp), Some(3), "↑ wraps around");
        assert_eq!(step(&enabled, Some(2), Named::Home), Some(0));
        assert_eq!(step(&enabled, Some(0), Named::End), Some(3));
    }

    /// Desktop's hotkey is `--color-text-translucent-subtle`, and `--color-text-on-accent-weak` on the selected row,
    /// and its submenu chevron is in the color of the row's text: white on the selected row.
    #[test]
    fn menu_asides_take_the_selection_like_desktop() {
        crate::style::ensure_fonts_loaded();
        let p = crate::style::palette(false, None, 0);
        let menu = |at: Option<f32>| {
            let rows = Column::new().push(menu_item(&p, None, "Mute".into(), "Ctrl Shift M", Some(UiMessage::Noop))).push(menu_item(
                &p,
                None,
                "Copy".into(),
                SUBMENU,
                Some(UiMessage::Noop),
            ));
            let size = Size::new(240.0, 2.0 * menu_pitch());
            let mut sim = iced_test::Simulator::with_size(Default::default(), size, menu_card(&p, rows, 240.0));
            if let Some(y) = at {
                sim.point_at(Point::new(20.0, y));
            }
            crate::focus::tests::pixels(&mut sim, &iced::Theme::Light, "menu-asides")
        };
        // The lightest and the darkest pixel of the right end of each row.
        let ends = |shot: &::image::RgbaImage| {
            let k = shot.width() as f32 / 240.0;
            let range = |row: f32| {
                let ys = (k * (row * menu_pitch() + 8.0)) as u32..(k * (row + 1.0) * menu_pitch()) as u32;
                let xs = (k * 150.0) as u32..(k * 228.0) as u32;
                let lum = ys
                    .flat_map(|y| xs.clone().map(move |x| (x, y)))
                    .map(|(x, y)| shot.get_pixel(x, y).0[..3].iter().map(|&c| u32::from(c)).sum::<u32>());
                lum.fold((u32::MAX, 0), |(lo, hi), l| (lo.min(l), hi.max(l)))
            };
            [range(0.0), range(1.0)]
        };
        let [hotkey, chevron] = ends(&menu(None));
        assert!(hotkey.0 > 3 * 100, "an idle hotkey is half transparent: {hotkey:?}");
        assert!(chevron.0 < 3 * 60, "the chevron has the text color: {chevron:?}");
        let [hotkey, _] = ends(&menu(Some(menu_pitch() / 2.0 + 4.0)));
        assert!(hotkey.1 > 3 * 200, "the selected row's hotkey is light: {hotkey:?}");
        let [_, chevron] = ends(&menu(Some(1.5 * menu_pitch() + 4.0)));
        assert!(chevron.1 > 3 * 240, "the selected row's chevron is white: {chevron:?}");
    }

    /// The chat menu takes ↓ and Enter: ↓ ↓ selects its second row, which Enter runs.
    #[test]
    fn the_chat_menu_runs_the_selected_row_like_chromium() {
        let mut app = crate::fuzz::demo_app("menu-keys");
        let id = {
            let st = app.store.read();
            let plain = |c: &&&Chat| c.id != NOTE_CHAT_ID && !c.flag(ChatFlag::Pinned) && !st.notifications_muted(c);
            app.visible_chats(&st).iter().find(plain).expect("a chat to archive and mute").id.clone()
        };
        let _ = app.update(UiMessage::ChatList(chat_list::Msg::OpenMenu(id.clone())));
        let picked: Vec<UiMessage> = {
            let mut sim = simulator(app.view_main());
            sim.tap_key(Named::ArrowDown);
            sim.tap_key(Named::ArrowDown);
            sim.tap_key(Named::Enter);
            sim.into_messages().collect()
        };
        let mute = |m: &UiMessage| matches!(m, UiMessage::ChatList(chat_list::Msg::ToggleFlag(c, ChatFlag::Muted)) if *c == id);
        assert!(picked.iter().any(mute), "Archive, then Mute: {}", picked.len());
        let main = app.main_window.expect("the main window");
        let open = app.pane.selected_chat_id.clone();
        let _ = app.update(UiMessage::WindowKey(main, Key::Named(Named::Enter), Modifiers::empty()));
        assert!(app.list.chat_menu.is_some() && app.pane.selected_chat_id == open, "the app leaves Enter to the menu");
    }
}

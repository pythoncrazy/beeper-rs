//! Desktop's `native` `BrandSelect` (`E0e`): a press opens Electron's context menu of its options, radio items with
//! the chosen one checked, at the select's top left corner (`popupContextMenu` at its `getBoundingClientRect` left and
//! top; the `positioningItem` that puts the checked item over the select works only on macOS). The menu is a native
//! menu: the keyboard of [`crate::menu_keys`], Escape and a press outside close it, and a pick closes it.

use super::*;
use crate::prelude::*;

/// An option of a select's menu, with whether it is the chosen one and its message, or the separator between two
/// `BrandSelect.Group`s.
#[derive(Clone)]
pub enum Row {
    Option(String, bool, Box<UiMessage>),
    Rule,
}

/// The open menu of a select: the select's bounds on the screen and the rows.
#[derive(Clone)]
pub struct Open {
    at: Rectangle,
    rows: Vec<Row>,
}

#[derive(Clone)]
pub enum Msg {
    /// A press on the select with the container `id`, whose menu has the rows.
    Press(iced::widget::Id, Vec<Row>),
    /// The menu opens, or closes.
    Shown(Option<Open>),
    /// A row of the menu: the menu closes and the row's message runs.
    Pick(Box<UiMessage>),
}

/// The message of a press on the select whose container has `id`: it opens a menu of `rows`.
pub fn press(id: &str, rows: Vec<Row>) -> UiMessage {
    UiMessage::Select(Msg::Press(iced::widget::Id::from(id.to_string()), rows))
}

impl BeeperGuiApp {
    pub(crate) fn select(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Press(id, rows) => {
                let shown = move |at| UiMessage::Select(Msg::Shown(Some(Open { at, rows: rows.clone() })));
                return iced::advanced::widget::operate(crate::Find::new(id)).map(shown);
            }
            Msg::Shown(open) => self.native_select = open,
            Msg::Pick(msg) => {
                self.native_select = None;
                return self.update(*msg);
            }
        }
        self.settle()
    }

    /// The menu of the open select, Desktop's `.menu-item`s with a `check-column`, as wide as the longest option
    /// within the `min-width` and `max-width` of `.menu-list`, and where it goes.
    pub(crate) fn view_native_select(&self, open: &Open) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let labels = open.rows.iter().filter_map(|r| if let Row::Option(label, ..) = r { Some(label) } else { None });
        let widest = labels.map(|l| crate::style::text_width(l, 13.0, Weight::Normal)).fold(0.0, f32::max);
        // The 5 px margins and 8 px paddings of the row, the 14 px column and its 9 px gap, and the 1 px borders.
        let w = (widest + 2.0 * (5.0 + 8.0 + 1.0) + 14.0 + 9.0).clamp(160.0, 360.0);
        let rows = open.rows.iter().map(|r| match r {
            Row::Option(label, on, msg) => {
                let check = container(container(on.then(|| ic(icon::CHECK, 10.0))).center_x(14)).padding(Padding::ZERO.right(9.0));
                let pick = UiMessage::Select(Msg::Pick(msg.clone()));
                (menu_item(p, Some(check.into()), label.clone(), "", Some(pick)), menu_pitch())
            }
            Row::Rule => (menu_rule(p), MENU_RULE),
        });
        let (menu, h) = menu_rows(p, rows.collect(), w);
        let (x, y) = menu_spot(open.at.position(), w, h, self.pane().size);
        (menu.into(), x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::Simulator;

    /// Desktop's Notification sound in Chat Info: a press opens Electron's menu of the options with the chosen one
    /// checked, at the select's top left corner. S runs Silent, the only option that starts with it, which closes the
    /// menu, and Escape closes it too.
    #[test]
    fn chat_info_selects_open_a_native_menu_like_desktop() {
        let mut app = crate::fuzz::demo_app("native-select");
        let chat = "!wa_eric:beeper.com";
        let _ = app.update(UiMessage::SelectChat(chat.into()));
        let press = {
            let mut sim = Simulator::with_size(Default::default(), Size::new(1024.0, 2000.0), app.view_info());
            let _ = sim.click(iced::widget::Id::new("info-select-Notification sound")).expect("the select");
            sim.into_messages().find_map(|m| if let UiMessage::Select(Msg::Press(_, rows)) = m { Some(rows) } else { None })
        };
        let rows = press.expect("a press opens the menu");
        let options: Vec<(&str, bool)> =
            rows.iter().map(|r| if let Row::Option(l, on, _) = r { (l.as_str(), *on) } else { ("", false) }).collect();
        assert_eq!(options, [("Default", true), ("Silent", false)]);
        let at = Rectangle::new(Point::new(700.0, 300.0), Size::new(300.0, 26.0));
        let _ = app.update(UiMessage::Select(Msg::Shown(Some(Open { at, rows }))));
        let main = app.main_window.expect("the main window");
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view(main));
        let silent = sim.find("Silent").expect("the menu").bounds();
        // The menu's top left corner is the select's: the label is after the 5 px margin, the 8 px padding, the
        // 14 px check column and its 9 px gap, on the second row.
        assert_eq!((silent.x, silent.y.round()), (700.0 + 5.0 + 8.0 + 14.0 + 9.0, (300.0 + 4.0 + menu_pitch() + 4.0).round()));
        let _ = sim.typewrite("s");
        let picked: Vec<UiMessage> = sim.into_messages().collect();
        assert_eq!(picked.len(), 1, "S runs the only row that starts with it");
        let _ = app.update(picked.into_iter().next().expect("a pick"));
        assert!(app.native_select.is_none(), "the pick closes the menu");
        assert_eq!(app.store.read().chat(chat).notification_sound.as_deref(), Some("silent"));
        let rows = vec![Row::Option("Off".into(), true, Box::new(UiMessage::Noop))];
        let _ = app.update(UiMessage::Select(Msg::Shown(Some(Open { at, rows }))));
        let _ = app.update(UiMessage::CloseModals);
        assert!(app.native_select.is_none(), "Escape closes the menu");
    }

    /// Chromium's `SelectByChar` without mnemonics: one match runs, several cycle from the selected row.
    #[test]
    fn letters_pick_like_chromium() {
        let titles = [Some('o'), Some('1'), Some('8'), Some('1'), None];
        assert_eq!(crate::menu_keys::by_char(&titles, None, 'O'), Some((0, true)), "the only match runs");
        assert_eq!(crate::menu_keys::by_char(&titles, None, '1'), Some((1, false)), "the first of several is selected");
        assert_eq!(crate::menu_keys::by_char(&titles, Some(1), '1'), Some((3, false)), "then the next one");
        assert_eq!(crate::menu_keys::by_char(&titles, Some(3), '1'), Some((1, false)), "and around");
        assert_eq!(crate::menu_keys::by_char(&titles, None, 'x'), None);
    }
}

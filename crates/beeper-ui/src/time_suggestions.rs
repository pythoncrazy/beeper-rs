//! Desktop's `TimeSuggestionsPreference` (`fjt`) under General > Time Suggestions: the editor of the time phrases
//! that the reminder menus and the command bar suggest.

use super::*;
use beeper_core::prefs::TIME_SUGGESTIONS_MAX;
use iced::widget::column;

/// The id of Desktop's `timeSuggestionsInput`.
pub(crate) const INPUT: &str = "timeSuggestionsInput";

/// The state of the editor, which Desktop drops when the settings close.
#[derive(Default)]
pub(crate) struct TimeEdit {
    /// The text of the add input while it shows.
    pub(crate) input: Option<String>,
    /// The hovered row, which shows its drag handle.
    hovered: Option<usize>,
    /// The dragged row and the order that the drag shows, which Desktop's `onSort` saves at the drop.
    pub(crate) drag: Option<(usize, Vec<String>)>,
}

#[derive(Clone)]
pub enum Msg {
    /// "Add" opens the input, and Save, Enter, Cancel or Escape close it.
    Add,
    Input(String),
    Save,
    Cancel,
    Remove(String),
    Reset,
    Hover(usize, bool),
    /// A press on a row starts a drag, and the release drops it.
    Grab(Option<usize>),
}

/// Desktop's `Tee`: the en-US `toLocaleString` with a numeric date, a 2-digit year and the time, like
/// "11/17/26, 3:05 PM".
pub(crate) fn suggestion_date(at: i64) -> String {
    let (y, m, d) = civil_from_days(day_of(at));
    format!("{m}/{d}/{:02}, {}", y.rem_euclid(100), clock(at, false))
}

/// The phrase that Desktop's Save adds: a future time that is not listed yet, while there is room.
fn addable(list: &[&str], text: &str, now: i64) -> Option<String> {
    let t = text.trim();
    let future = parse_when_millis(t, now).is_some_and(|at| at > now);
    (!t.is_empty() && future && !list.contains(&t) && list.len() < TIME_SUGGESTIONS_MAX).then(|| t.to_string())
}

impl BeeperGuiApp {
    pub(crate) fn time_suggestions(&mut self, msg: Msg) -> Task<UiMessage> {
        let list: Vec<String> = self.store.read().settings.time_suggestions().into_iter().map(String::from).collect();
        let e = &mut self.time_edit;
        match msg {
            Msg::Add => {
                e.input = Some(String::new());
                return operation::focus(INPUT);
            }
            Msg::Input(t) => e.input = e.input.take().map(|_| t),
            Msg::Save => {
                let shown: Vec<&str> = list.iter().map(String::as_str).collect();
                let now = now_unix_millis();
                if let Some(t) = e.input.as_deref().and_then(|t| addable(&shown, t, now)) {
                    e.input = None;
                    return self.set_time_suggestions([list, vec![t]].concat());
                }
            }
            Msg::Cancel => e.input = None,
            Msg::Remove(t) if list.len() > 1 => {
                return self.set_time_suggestions(list.into_iter().filter(|x| *x != t).collect())
            }
            Msg::Remove(_) => {}
            Msg::Reset => return self.set_time_suggestions(Vec::new()),
            Msg::Hover(o, true) => {
                if let Some((from, order)) = e.drag.as_mut().filter(|d| d.0 != o && o < d.1.len()) {
                    let moved = order.remove(*from);
                    order.insert(o, moved);
                    *from = o;
                }
                e.hovered = Some(o);
            }
            Msg::Hover(o, false) => e.hovered = e.hovered.filter(|&h| h != o),
            Msg::Grab(Some(i)) => e.drag = Some((i, list)),
            Msg::Grab(None) => {
                if let Some((_, order)) = e.drag.take().filter(|d| d.1 != list) {
                    return self.set_time_suggestions(order);
                }
            }
        }
        Task::none()
    }

    /// Desktop's `setTimeSuggestionOptions`: saves the list through `xpe` and syncs it to Desktop.
    fn set_time_suggestions(&mut self, list: Vec<String>) -> Task<UiMessage> {
        let list = prefs::time_suggestion_options(&list.into());
        self.patch_settings(|s| s.time_suggestions = list);
        let mut st = self.store.write();
        let client = st.matrix.clone();
        match beeper_core::matrix::time_suggestions_write(&mut st).zip(client) {
            Some(((kind, content), c)) => {
                Task::perform(beeper_core::matrix::set_global(c, kind, content), UiMessage::Pushed)
            }
            None => Task::none(),
        }
    }

    /// Desktop's `TimeSuggestionsPreference`: the description, the sortable phrases with their times, and the add
    /// input or the Add and Reset buttons.
    pub(crate) fn view_time_suggestions(&self, st: &BeeperStore) -> El<'_> {
        let p = &self.pal;
        let e = &self.time_edit;
        let now = now_unix_millis();
        let saved = st.settings.time_suggestions();
        let n = saved.len();
        let edit = |m| UiMessage::TimeSuggestions(m);
        let limit = match n >= TIME_SUGGESTIONS_MAX {
            true => trf("You have reached the limit of %d.", &[&TIME_SUGGESTIONS_MAX]),
            false => trf("You can add up to %d.", &[&TIME_SUGGESTIONS_MAX]),
        };
        let about = "Customize time suggestions for quick reminders in the command bar and context menus. Try %1$s, %2$s, %3$s.";
        let about = trf(about, &[&"\"7am Tomorrow\"", &"\"In 3 weeks\"", &"\"Next Monday\""]);
        let header = txt(format!("{about} {limit}"), 13.0).line_height(1.4).color(p.weak);
        let shown: Vec<String> = match &e.drag {
            Some((_, order)) => order.clone(),
            None => saved.iter().map(|s| s.to_string()).collect(),
        };
        // `--color-icon-translucent-weak`.
        let handle_color = if self.dark { Color::WHITE } else { Color::BLACK }.scale_alpha(0.4);
        let mut rows = Column::new().spacing(8);
        for (i, label) in shown.into_iter().enumerate() {
            // Desktop's `Drag` fades the dragged row to 0.4.
            let fade = if e.drag.as_ref().is_some_and(|d| d.0 == i) { 0.4 } else { 1.0 };
            let date = parse_when_millis(&label, now)
                .map(|at| txt(suggestion_date(at), 10.0).line_height(lh(12.0)).color(p.weak.scale_alpha(fade)));
            let name = semi(label.clone(), 13.0).color(p.text.scale_alpha(fade));
            let labels = row![name].push(date).spacing(8).align_y(Center).width(Fill);
            let shows = e.hovered == Some(i) || e.drag.is_some();
            let handle = tinted(&DRAG_HANDLE, 16.0, handle_color.scale_alpha(if shows { fade } else { 0.0 }));
            let handle = mouse_area(handle).interaction(mouse::Interaction::Move);
            let gone = (n > 1).then(|| edit(Msg::Remove(label.clone())));
            let remove = button(ic(icon::CLOSE, 14.0))
                .padding(0)
                .style(btn_style([CLEAR; 3], [p.text.scale_alpha(fade); 3], 0.0))
                .on_press_maybe(gone.clone());
            let remove = self.plain(format!("time-remove-{label}"), remove, gone);
            let icons = row![handle, remove].spacing(6).align_y(Center).padding(Padding::ZERO.left(8.0));
            let line = mouse_area(row![labels, icons].align_y(Center))
                .on_press(edit(Msg::Grab(Some(i))))
                .on_enter(edit(Msg::Hover(i, true)))
                .on_exit(edit(Msg::Hover(i, false)));
            rows = rows.push(line);
        }
        let solid = |key: &str, content: El<'static>, msg: Option<UiMessage>| {
            self.plain(
                format!("time-{key}"),
                button(content).padding([4, 6]).style(pill_style(p)).on_press_maybe(msg.clone()),
                msg,
            )
        };
        let foot: El = match &e.input {
            Some(text) => {
                let hint = trf("Try %1$s or %2$s", &[&"'7am Tomorrow'", &"'In 3w'"]);
                let input = text_input(&hint, text)
                    .id(INPUT)
                    .on_input(move |t| edit(Msg::Input(t)))
                    .on_submit(edit(Msg::Save))
                    .size(rem(13.0))
                    .line_height(lh(16.0))
                    .padding([8, 8])
                    .width(Fill)
                    .style(outline_input(p));
                let save = addable(&saved, text, now).map(|_| edit(Msg::Save));
                let add = row![
                    input,
                    solid("save", ic(icon::CHECK, 16.0).into(), save),
                    solid("cancel", ic(icon::CLOSE, 16.0).into(), Some(edit(Msg::Cancel)))
                ]
                .spacing(6)
                .align_y(Center);
                let at = Some(text.trim()).filter(|t| !t.is_empty()).and_then(|t| parse_when_millis(t, now));
                let preview = at.map(|at| txt(suggestion_date(at), 10.0).line_height(lh(12.0)).color(p.weak));
                let preview = container(preview).height(24).padding([4, 6]).align_y(Center);
                column![add, preview].padding(Padding::ZERO.top(8.0)).into()
            }
            None => {
                let label = |t| txt(tr(t), 13.0).line_height(1.0).into();
                let add = (n < TIME_SUGGESTIONS_MAX).then(|| edit(Msg::Add));
                let buttons =
                    row![solid("add", label("Add"), add), solid("reset", label("Reset"), Some(edit(Msg::Reset)))];
                container(buttons.spacing(6)).width(Fill).align_right(Fill).padding(Padding::ZERO.top(6.0)).into()
            }
        };
        column![container(header).padding(Padding::ZERO.bottom(8.0)), rows, foot].width(Fill).padding(16).into()
    }
}

impl BeeperGuiApp {
    /// A plain `<button>` of the Settings window, which takes the focus without showing it (`button { outline: 0 }`)
    /// while it works.
    fn plain<'a>(&self, key: String, b: impl Into<El<'a>>, press: Option<UiMessage>) -> El<'a> {
        match press {
            Some(m) => self.target(self.settings_window, key, b, focus::Keys::press(m), focus::Look::Hidden),
            None => b.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tee_formats_like_desktop() {
        let at = parse_when_millis("tomorrow at 3:05pm", now_unix_millis()).expect("a time");
        let (y, m, d) = civil_from_days(day_of(at));
        assert_eq!(suggestion_date(at), format!("{m}/{d}/{:02}, 3:05 PM", y % 100));
    }

    #[test]
    fn addable_like_desktop() {
        let now = now_unix_millis();
        let list = prefs::TIME_SUGGESTIONS.to_vec();
        assert_eq!(addable(&list, " In 3w ", now).as_deref(), Some("In 3w"), "the phrase is trimmed");
        assert_eq!(addable(&list, "Tomorrow", now), None, "already listed");
        assert_eq!(addable(&list, "yesterday", now), None, "a past time");
        assert_eq!(addable(&list, "banana", now), None, "not a time");
        assert_eq!(addable(&list, "  ", now), None);
        let full = ["a"; TIME_SUGGESTIONS_MAX];
        assert_eq!(addable(&full, "In 3w", now), None, "the list is full");
    }
}

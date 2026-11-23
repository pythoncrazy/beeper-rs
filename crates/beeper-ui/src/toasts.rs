//! Desktop's toasts: the status banner and the older toasts above it, their timeout, and their buttons.

use super::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    DismissBanner,
    /// The pointer enters or leaves the toast, which pauses its timeout like Desktop.
    Hover(bool),
    Tick,
    DismissOlder(usize),
    /// Desktop's toast button: the toast closes, newest or older, and its action runs.
    Action(Option<usize>, Box<UiMessage>),
    /// Desktop's `DownloadMobileApps` toast after the onboarding shows, or its close button hides it.
    MobileApps(bool),
}

/// Desktop's links of the `DownloadMobileApps` toast (`O8`, `Cye` and `Tye` of sentry-common).
const DOWNLOAD_URL: &str = "https://www.beeper.com/download";
const IOS_URL: &str = "https://www.beeper.com/download/ios";
const ANDROID_URL: &str = "https://www.beeper.com/download/android";

impl BeeperGuiApp {
    pub(crate) fn toasts(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::MobileApps(on) => self.mobile_apps_toast = on,
            m @ (Msg::Hover(_) | Msg::Tick) if self.toast_clock.0 != self.status_banner => {
                self.toast_clock = (self.status_banner.clone(), self.toast_clock.1, Duration::ZERO);
                return self.update(UiMessage::Toast(m));
            }
            Msg::Hover(on) => self.toast_clock.1 = on,
            Msg::Tick => {
                for t in &mut self.older_toasts {
                    t.1 += TOAST_TICK;
                }
                self.older_toasts.retain(|t| t.0.stays || t.1 < TOAST_TIMEOUT);
                if !self.newest_toast_running() {
                    return Task::none();
                }
                self.toast_clock.2 += TOAST_TICK;
                if self.toast_clock.2 >= if self.shown_note() { UNDO_NOTE_TIMEOUT } else { TOAST_TIMEOUT } {
                    return self.update(UiMessage::Toast(Msg::DismissBanner));
                }
            }
            // The second click of a double click on a close button can come after the first click removed the toast.
            Msg::DismissOlder(i) if i < self.older_toasts.len() => _ = self.older_toasts.remove(i),
            Msg::DismissOlder(_) => {}
            Msg::Action(older, action) => {
                let _ = match older {
                    Some(i) => self.update(UiMessage::Toast(Msg::DismissOlder(i))),
                    None => self.update(UiMessage::Toast(Msg::DismissBanner)),
                };
                return self.update(*action);
            }
            Msg::DismissBanner => {
                (self.status_banner, self.relaunch_toast) = (String::new(), None);
                self.toast_dismissals += 1;
            }
        }
        self.settle()
    }
}

/// Desktop's default toast timeout, counted in ticks while the toast is not paused.
pub(crate) const TOAST_TIMEOUT: Duration = Duration::from_secs(5);

/// Desktop's `Txe` shows its "Undo:" toast for a second.
pub(crate) const UNDO_NOTE_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) const TOAST_TICK: Duration = Duration::from_millis(250);

/// Desktop's `toasts-list` shows the last 10 toasts.
pub(crate) const TOASTS_SHOWN: usize = 10;

/// Desktop's `Toasts-module__container`: no border, and the `--shadow-medium-15` shadow.
pub(crate) fn toast_box(bg: Color, p: &Palette) -> impl Fn(&Theme) -> container::Style + Copy {
    glow_box(bg, p.shadow, 10.0, 1.0, 10.0)
}

/// Desktop's `Archive`, `Inbox` and `LowPriority` assets, which its archive and Low Priority toasts show, its `Undo`
/// icon, its `Lock` icon of the recovery key reminder, and its `Phone` icon of the mobile apps toast.
pub(crate) static TOAST_ICONS: std::sync::LazyLock<[svg::Handle; 6]> = std::sync::LazyLock::new(|| {
    [
        ("0 0 20 20", "M4.556 17a1.5 1.5 0 0 1-1.1-.457A1.5 1.5 0 0 1 3 15.444V5.722q0-.292.097-.525.098-.232.253-.447l1.089-1.322a1 1 0 0 1 .389-.321Q5.06 3 5.333 3h9.334q.272 0 .505.107a1 1 0 0 1 .39.32L16.65 4.75q.156.214.253.447t.097.525v9.722a1.5 1.5 0 0 1-.457 1.1 1.5 1.5 0 0 1-1.099.456zm.31-11.667h10.267l-.66-.777H5.527zm-.31 1.556v8.555h10.888V6.89zM10 14.667l3.111-3.111-1.089-1.09-1.244 1.245V8.444H9.222v3.267l-1.244-1.244-1.09 1.089z"),
        ("0 0 20 20", "M15.833 2.5H4.158c-.925 0-1.65.742-1.65 1.667L2.5 15.833A1.66 1.66 0 0 0 4.158 17.5h11.675c.917 0 1.667-.75 1.667-1.667V4.167c0-.925-.75-1.667-1.667-1.667m0 10H12.5c0 1.383-1.125 2.5-2.5 2.5a2.5 2.5 0 0 1-2.5-2.5H4.158V4.167h11.675z"),
        ("0 0 20 20", "M11.667 4.167h6.666v1.556h-6.666zm0 4.278h6.666V10h-6.666zm0 4.278h6.666v1.555h-6.666zm-10-3.5c0 2.784 2.433 5.055 5.416 5.055H7.5v1.556L10 13.5l-2.5-2.333v1.556h-.417c-2.066 0-3.75-1.572-3.75-3.5 0-1.93 1.684-3.5 3.75-3.5H10V4.167H7.083c-2.983 0-5.416 2.271-5.416 5.056"),
        ("0 0 20 20", "M5.833 15.833v-1.667h5.917q1.312 0 2.281-.833.969-.834.969-2.083t-.969-2.084q-.969-.833-2.281-.833H6.5L8.667 10.5 7.5 11.666 3.333 7.5 7.5 3.333 8.667 4.5 6.5 6.666h5.25q2.021 0 3.469 1.313t1.448 3.27-1.448 3.271-3.469 1.313z"),
        ("0 0 20 20", "M9.996 2.712a3.9 3.9 0 0 1 1.86.461q.865.462 1.392 1.377.535.909.535 2.264v1.745q.569.079.9.423.433.447.433 1.333v5.178q0 .9-.432 1.347t-1.26.448H6.576q-.827 0-1.26-.448-.432-.446-.432-1.347v-5.178q0-.886.432-1.333.333-.344.9-.423V6.814q0-1.355.528-2.264.535-.915 1.4-1.377a3.9 3.9 0 0 1 1.852-.46m0 1.296q-.651 0-1.208.308-.55.308-.879.901-.33.594-.33 1.457v1.86h4.849v-1.86q0-.864-.337-1.457a2.25 2.25 0 0 0-.886-.901 2.43 2.43 0 0 0-1.209-.308"),
        // Desktop's `Phone`, in its own 12x19 box.
        ("0 0 12 19", "M3.188 18.102q-1.047 0-1.696-.618-.648-.617-.648-1.625V2.844q0-1.008.648-1.625.649-.617 1.696-.617h6.406q1.031 0 1.656.617t.625 1.625v13.008q0 1.015-.625 1.632-.625.618-1.656.618zm.335-1.813h5.68q.438 0 .68-.242.25-.234.25-.656V3.313q0-.43-.25-.665-.242-.234-.68-.234h-5.68q-.444 0-.695.234-.242.235-.242.665V15.39q0 .429.242.664.25.234.695.234m1.133-.64a.4.4 0 0 1-.414-.415q0-.188.117-.296a.4.4 0 0 1 .297-.118H8.07a.4.4 0 0 1 .414.414q0 .188-.117.297a.4.4 0 0 1-.297.117zM5.352 4.28a.614.614 0 0 1-.64-.633q-.001-.273.179-.453a.63.63 0 0 1 .46-.187h2.024q.266 0 .445.187.188.18.188.453c0 .273-.063.334-.188.454a.6.6 0 0 1-.445.18z"),
    ]
    .map(|(view, d)| svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path d="{d}"/></svg>"#).into_bytes()))
});

/// Desktop's `Toast`, which shows while `status_banner` is its text.
#[derive(Clone, Default)]
pub struct Toast {
    pub text: String,
    /// The index of the icon in `TOAST_ICONS`.
    pub icon: Option<usize>,
    /// Desktop's `textStyle: "strong"`.
    pub strong: bool,
    /// Desktop's `buttons`, in a row below the text.
    pub buttons: Vec<(&'static str, UiMessage)>,
    /// Desktop's `rightButton`.
    pub right: Option<(&'static str, UiMessage)>,
    /// Desktop's `onClick`.
    pub open: Option<UiMessage>,
    /// Desktop's `type: "error"`.
    pub error: bool,
    /// Desktop's `onDismiss` of a toast with `keepToastOpen` buttons: the close runs it, and the buttons do not close
    /// the toast.
    pub dismiss: Option<UiMessage>,
    /// The links in the text, in their order: each word opens its URL, in the color of the text like Desktop's `a`.
    pub links: Vec<(&'static str, &'static str)>,
    /// Desktop's `timeout: -1`: the toast stays until it is closed, also under newer toasts.
    pub stays: bool,
    /// Desktop's toast `id`, which `removeToast` closes.
    pub id: Option<String>,
}

impl Toast {
    pub fn new(text: String) -> Self {
        Toast { text, ..Default::default() }
    }
}

impl BeeperGuiApp {
    /// The chats that the windows show stay loaded: the chats of the panes and their threads, of the poll vote
    /// windows, and of the voice note that plays. A chat that a sync removed leaves the windows at `StoreChanged`.
    pub(crate) fn keep_shown(&mut self) {
        let panes = self.panes().flat_map(|p| std::iter::once(p).chain(p.thread.as_deref()));
        let chats = panes.filter_map(|p| p.selected_chat_id.as_ref());
        let chats = chats.chain(self.vote_windows.iter().map(|w| &w.1)).chain(self.track_chat());
        let st = self.store.read();
        let shown: Vec<String> = chats.filter_map(|c| st.resolve_canonical_chat_id(c)).collect();
        if st.shown == shown {
            return;
        }
        drop(st);
        let mut st = self.store.write();
        st.shown = shown.clone();
        shown.iter().for_each(|c| st.load_chat(c));
    }

    /// The toast timeouts run while a window is focused and the pointer is not on the toasts.
    pub(crate) fn toast_running(&self) -> bool {
        let hovered = self.toast_clock.0 == self.status_banner && self.toast_clock.1;
        let shown = self.newest_toast_running() || self.older_toasts.iter().any(|t| !t.0.stays);
        self.relaunch_toast.is_none() && self.focused && !hovered && shown
    }

    /// Desktop's `toasts-list` keeps the last `TOASTS_SHOWN` toasts, with the newest one.
    pub(crate) fn trim_older_toasts(&mut self) {
        let extra = self.older_toasts.len().saturating_sub(TOASTS_SHOWN - 1);
        self.older_toasts.drain(..extra);
    }

    /// Desktop's `addToast` of each error of one action: every toast shows, the last one newest.
    pub(crate) fn fail_all(&mut self, errors: Vec<String>) {
        let Some((last, rest)) = errors.split_last() else { return };
        for e in rest {
            self.older_toasts.push((Toast { error: true, ..Toast::new(e.clone()) }, Duration::ZERO));
        }
        self.trim_older_toasts();
        self.fail(Toast::new(last.clone()));
    }

    /// Desktop's `removeToast` of the toast with ID `id`, the newest one or an older one.
    pub(crate) fn remove_toast(&mut self, id: &str) {
        self.older_toasts.retain(|t| t.0.id.as_deref() != Some(id));
        if self.shown_toast().is_some_and(|t| t.id.as_deref() == Some(id)) {
            self.sticky_banner.take_if(|t| *t == self.status_banner);
            self.status_banner.clear();
        }
    }

    pub(crate) fn newest_toast_running(&self) -> bool {
        !self.status_banner.is_empty() && self.sticky_banner.as_ref() != Some(&self.status_banner)
    }

    /// The newest toast and the time it showed, which moves up in Desktop's `toasts-list` when a new toast comes.
    /// Sticky toasts that do not stay and "Undo:" notes are replaced instead.
    pub(crate) fn stackable_toast(&self) -> Option<(Toast, Duration)> {
        let stays = self.shown_toast().is_some_and(|t| t.stays);
        if !(self.newest_toast_running() || stays) || self.relaunch_toast.is_some() || self.shown_note() {
            return None;
        }
        let t = self.shown_toast().cloned().unwrap_or_else(|| Toast::new(self.status_banner.clone()));
        let clock = &self.toast_clock;
        Some((t, if clock.0 == self.status_banner { clock.2 } else { Duration::ZERO }))
    }

    /// The "Undo:" toast of `Txe` is on screen.
    pub(crate) fn shown_note(&self) -> bool {
        let shown = |w: &String| self.status_banner == format!("{} {w}", tr("Undo:"));
        self.relaunch_toast.is_none() && self.undo_note.as_ref().is_some_and(shown)
    }

    pub(crate) fn shown_toast(&self) -> Option<&Toast> {
        self.toast.as_ref().filter(|t| self.relaunch_toast.is_none() && t.text == self.status_banner)
    }

    /// The status toast at the bottom left of the `main` window or of the Media window.
    pub(crate) fn view_toast(&self, main: bool) -> Option<El<'_>> {
        let p = &self.pal;
        let left = if main { 76.0 } else { 8.0 };
        let banner = match self.relaunch_toast {
            Some(t) => trs(t, &["Beeper"]),
            None => self.status_banner.clone(),
        };
        let pill = |(kind, text): (Pill, String)| -> El<'_> {
            let icon = (kind == Pill::Syncing).then(|| tinted(&MAT_SYNC, 12.0, p.strong));
            let body = row![].push(icon).push(txt(text, 12.0).color(p.strong)).spacing(4).align_y(Center);
            let style = move |_: &Theme| container::Style {
                background: Some(p.card.into()),
                border: border::rounded(17.0),
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                    offset: Vector::ZERO,
                    blur_radius: 20.0,
                },
                ..container::Style::default()
            };
            let pill = mouse_area(container(body).padding([4, 8]).style(style));
            match kind {
                Pill::Connecting => {
                    pill.on_enter(UiMessage::PillHover(true)).on_exit(UiMessage::PillHover(false)).into()
                }
                Pill::Offline | Pill::Syncing => pill.into(),
            }
        };
        let pills = self.status_pills().into_iter().map(pill);
        let older = self.older_toasts.iter().enumerate().map(|(i, t)| self.toast_card(&t.0, Some(i)));
        let reminder = (main && self.recovery_reminder()).then(|| {
            let toast = Toast {
                icon: Some(4),
                strong: true,
                right: Some((trx("Verify", "recovery key reminder"), enter_key::KeyMsg::Open.into())),
                dismiss: Some(UiMessage::DismissRecoveryReminder),
                ..Toast::new(tr("Check if you still have access to a recovery key.").into())
            };
            self.toast_card(&toast, None)
        });
        // Desktop's `DownloadMobileApps`, which has no timeout and opens the download page.
        let apps = (main && self.mobile_apps_toast).then(|| {
            let toast = Toast {
                icon: Some(5),
                open: Some(UiMessage::OpenUrl(DOWNLOAD_URL.into())),
                dismiss: Some(UiMessage::Toast(Msg::MobileApps(false))),
                links: vec![("iPhone", IOS_URL), ("Android", ANDROID_URL)],
                ..Toast::new("Don't forget to take your chats with you! On your iPhone or Android...".into())
            };
            self.toast_card(&toast, None)
        });
        let older = pills.chain(reminder).chain(apps).chain(older);
        let newest = (!banner.is_empty()).then(|| -> El<'_> {
            let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, p.sub, UiMessage::Toast(toasts::Msg::DismissBanner));
            if let Some(what) = self.undo_note.as_ref().filter(|_| self.shown_note()) {
                let glyph = container(tinted(&TOAST_ICONS[3], 16.0, p.strong)).center_x(16).center_y(20);
                let text = rich_text![span::<(), _>(tr("Undo:")).font(fw(Weight::Bold)), span(format!(" {what}"))]
                    .size(rem(13.0))
                    .color(p.strong);
                let card = row![glyph, text, close].spacing(8).align_y(Center);
                return container(card).padding([6, 8]).style(toast_box(p.card, p)).into();
            }
            if let Some(t) = self.shown_toast() {
                return self.toast_card(t, None);
            }
            let pill_w =
                self.relaunch_toast.map_or(0.0, |_| text_width(tr("Relaunch app"), 13.0, Weight::Normal) + 20.0);
            let text =
                txt(banner.to_string(), 13.0).color(p.strong).wrapping(iced::widget::text::Wrapping::WordOrGlyph);
            let text = container(text).max_width(self.toast_width() - 40.0 - pill_w);
            let relaunch =
                self.relaunch_toast.map(|_| pill_btn(p, tr("Relaunch app"), UiMessage::Window(windows::Msg::Relaunch)));
            let card = row![text].push(relaunch).push(close);
            let card = card.spacing(8).align_y(Center);
            container(card).padding(8).style(toast_box(p.card, p)).into()
        });
        let toasts: Vec<El> = older.chain(newest).collect();
        (!toasts.is_empty()).then(|| {
            let list = mouse_area(Column::with_children(toasts).spacing(8).max_width(self.toast_width()))
                .on_enter(UiMessage::Toast(toasts::Msg::Hover(true)))
                .on_exit(UiMessage::Toast(toasts::Msg::Hover(false)));
            container(list).width(Fill).align_bottom(Fill).padding(Padding::new(8.0).left(left)).into()
        })
    }

    /// The width that a toast takes at most: Desktop's `toasts-list` is `sidebarWidth - accountSwitcherWidth` wide,
    /// at least 300 px, inside 8 px sides.
    fn toast_width(&self) -> f32 {
        self.sidebar_width.max(300.0) - 16.0
    }

    /// Desktop's `Toast`: the newest toast, or the toast at an index of `older_toasts`.
    pub(crate) fn toast_card<'a>(&'a self, t: &Toast, older: Option<usize>) -> El<'a> {
        let p = &self.pal;
        let (bg, fg) = match t.error {
            true => (hex(0xdc362e), Color::WHITE),
            false => (p.card, p.strong),
        };
        let btn = |(label, msg): &(&'static str, UiMessage), fg: Color| {
            let label = semi(*label, 13.0).line_height(lh(18.0)).color(fg);
            let action = match t.dismiss {
                Some(_) => msg.clone(),
                None => UiMessage::Toast(toasts::Msg::Action(older, Box::new(msg.clone()))),
            };
            button(label).bare(fg).on_press(action)
        };
        // Desktop's 20 px icon sits in a 2rem box with a -8 px margin, so the toast stays 2rem tall.
        let glyph = t.icon.map(|i| container(tinted(&TOAST_ICONS[i], 20.0, fg)).center(20));
        let wrap = iced::widget::text::Wrapping::WordOrGlyph;
        let text: El<'a> = match (t.links.is_empty(), t.strong) {
            (true, true) => semi(t.text.clone(), 13.0).color(fg).wrapping(wrap).into(),
            (true, false) => txt(t.text.clone(), 13.0).color(fg).wrapping(wrap).into(),
            (false, _) => {
                let mut rest = t.text.as_str();
                let mut spans = Vec::new();
                for &(word, url) in &t.links {
                    let (before, after) = rest.split_once(word).expect("the toast text has its links in order");
                    spans.push(span(before.to_string()));
                    spans.push(span(word).link(UiMessage::OpenUrl(url.into())));
                    rest = after;
                }
                spans.push(span(rest.to_string()));
                rich_text(spans).on_link_click(|m: UiMessage| m).size(rem(13.0)).color(fg).wrapping(wrap).into()
            }
        };
        let below = (!t.buttons.is_empty()).then(|| {
            let dot = || -> El { semi(" • ", 13.0).line_height(lh(18.0)).color(fg).into() };
            let mut buttons = t.buttons.iter().map(|b| -> El { btn(b, fg).into() });
            let first = buttons.next().expect("a button");
            Row::with_children(std::iter::once(first).chain(buttons.flat_map(|b| [dot(), b])))
        });
        // The text wraps inside Desktop's toast list, beside the icon, the right button and the close button.
        let pad = if t.icon.is_some() { 14.0 } else { 16.0 };
        let icon_w = if t.icon.is_some() { 26.0 } else { 0.0 };
        let right_w = t.right.as_ref().map_or(0.0, |(label, _)| text_width(label, 13.0, Weight::Semibold) + 8.0);
        let body = column![text].push(below).spacing(16).max_width(self.toast_width() - pad - icon_w - right_w - 24.0);
        let right = t.right.as_ref().map(|b| btn(b, p.accent));
        let dismiss = match &t.dismiss {
            Some(m) => m.clone(),
            None => older.map_or(UiMessage::Toast(toasts::Msg::DismissBanner), |a0| {
                UiMessage::Toast(toasts::Msg::DismissOlder(a0))
            }),
        };
        let close = icon_btn(p, icon::CLOSE, 14.0, 16.0, if t.error { fg } else { p.sub }, dismiss);
        let pad = if glyph.is_some() { Padding::new(6.0).right(8.0) } else { Padding::new(8.0) };
        let lead = row![].push(glyph).push(body).spacing(6);
        let card = row![lead].push(right).push(close).spacing(8);
        let card = card.align_y(if t.buttons.is_empty() { Center } else { iced::Alignment::Start });
        let card = container(card).padding(pad).style(toast_box(bg, p));
        match t.open.clone() {
            Some(open) => mouse_area(card).on_press(open).into(),
            None => card.into(),
        }
    }
}

//! The windows: their open, close, focus, resize and maximize, the tray, the relaunch, and a second launch of the app.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Focused(bool),
    /// Maximizes or restores a chat window, or the main window when `None`.
    ToggleMaximize(Option<window::Id>),
    /// The Minimize item of the Window menu.
    Minimize(window::Id),
    /// Whether the main window is minimized, as the window manager says after the window lost the focus.
    Minimized(bool),
    Opened(window::Id),
    /// A window drew a frame, after which its screenshot has content.
    Drawn(window::Id),
    Relaunch,
    OpenChat(String),
    /// Desktop's `openSpaceInNewWindow` of a rail space.
    OpenSpace(String),
    Closed(window::Id),
    CloseRequested(window::Id),
    Tray(Result<tray::Event, String>),
    TrayFailed(String),
    Resized(window::Id, Size),
    ScreenshotCaptured(window::Id, window::Screenshot),
    FocusRequested(Option<String>),
    /// Desktop's `second-instance` event with the arguments of the later launch.
    SecondInstance(Vec<String>),
}

impl BeeperGuiApp {
    pub(crate) fn windows(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::ToggleMaximize(id) => {
                return window::toggle_maximize(id.or(self.main_window).expect("the main window"));
            }
            Msg::Focused(focused) => {
                self.focused = focused;
                // A window that the window manager minimizes loses the focus, and one that it restores gains it.
                let minimized = match (focused, self.main_window) {
                    (false, Some(main)) => {
                        let later = async { tokio::time::sleep(std::time::Duration::from_millis(300)).await };
                        let ask = Task::perform(later, |()| ()).then(move |()| window::is_minimized(main));
                        ask.map(|m| UiMessage::Window(Msg::Minimized(m == Some(true))))
                    }
                    _ => {
                        self.minimized = false;
                        Task::none()
                    }
                };
                self.read_open_chat();
                let screen = self.login.screen;
                let clipboard = matches!(screen, login::Screen::RecoveryKey | login::Screen::Code);
                let key = (focused && clipboard && self.login_window.is_some())
                    .then(|| iced::clipboard::read().map(|t| LoginMsg::KeyOnFocus(t).into()));
                let modal = (focused && self.enter_key.window.is_some())
                    .then(|| iced::clipboard::read().map(|t| enter_key::KeyMsg::OnFocus(t).into()));
                return Task::batch([self.close_notifications(), minimized].into_iter().chain(key).chain(modal));
            }
            Msg::Relaunch => {
                RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                return self.quit();
            }
            Msg::Minimize(id) => {
                self.minimized |= Some(id) == self.main_window;
                return window::minimize(id, true);
            }
            // A focus that came back while the window manager answered wins.
            Msg::Minimized(minimized) => self.minimized = minimized && !self.focused,
            Msg::Opened(id) => {
                let key = if Some(id) == self.settings_window {
                    "BEEPER_SCREENSHOT_SETTINGS"
                } else {
                    "BEEPER_SCREENSHOT_EXIT"
                };
                if std::env::var(key).is_ok() {
                    self.shot = Some(id);
                }
            }
            Msg::Drawn(id) if self.shot == Some(id) => {
                self.shot = None;
                return window::screenshot(id).map(move |s| UiMessage::Window(Msg::ScreenshotCaptured(id, s)));
            }
            Msg::Drawn(_) => {}
            Msg::Resized(id, size) => self.resized(id, size),
            Msg::Tray(Ok(tray::Event::Up)) => self.tray_sent = (None, true),
            // Electron cannot pop up the tray menu on Linux, so Desktop's click on a focused window does nothing.
            Msg::Tray(Ok(tray::Event::Activate)) if !self.hidden && self.focused => {}
            Msg::Tray(Ok(tray::Event::Activate)) => return self.show_all(),
            Msg::Tray(Ok(tray::Event::Show)) => return self.set_hidden(false),
            Msg::Tray(Ok(tray::Event::Hide)) => return self.set_hidden(true),
            Msg::Tray(Ok(tray::Event::Quit)) => return self.quit(),
            Msg::Tray(Err(e)) | Msg::TrayFailed(e) => {
                eprintln!("tray icon failed: {e}");
                return self.set_hidden(false);
            }
            Msg::CloseRequested(id) if self.poll_windows.iter().any(|w| w.id == id) => {
                return self.poll_edit(id, PollEdit::Close);
            }
            // Desktop hides the main window on close, with or without the tray: a later launch shows it again.
            Msg::CloseRequested(id) => {
                // Desktop's window `close`: it hides the app unless Quit on Close was on at launch, also while
                // signed out.
                let primary = Some(id) == self.main_window || Some(id) == self.login_window;
                return match (primary, self.quit_on_close) {
                    (true, false) => self.set_hidden(true),
                    (true, true) => self.save_main_window().chain(window::close(id)),
                    _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => {
                        self.save_window(id, "media-viewer").chain(window::close(id))
                    }
                    _ if self.chat_windows.iter().any(|w| w.0 == id && w.1.thread_root.is_none() && !w.1.info)
                        && !self.is_space_window(id)
                        && self.carousel_opener(id).is_none() =>
                    {
                        self.save_window(id, POP_OUT).chain(window::close(id))
                    }
                    (false, _) => window::close(id),
                };
            }
            Msg::OpenChat(cid) => {
                self.list.chat_menu = None;
                let open = self
                    .chat_windows
                    .iter()
                    .find(|w| w.1.selected_chat_id.as_deref() == Some(&cid) && !self.is_space_window(w.0));
                if let Some(&(id, _)) = open {
                    return window::gain_focus(id);
                }
                // Desktop's `windowStateKeeper("pop-out-thread")` keeps the size of the last chat window.
                let size = self.saved_window_settings(POP_OUT, Size::new(800.0, 600.0)).size;
                let (id, open) = self.open_window(Self::window_settings(size));
                self.chat_windows.push((id, Pane { size, ..Pane::default() }));
                let near = self.cascade(id);
                let select = self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(cid))));
                self.save_subwindows();
                return Task::batch([open.discard(), near, select]);
            }
            Msg::OpenSpace(entry) => {
                self.list.chat_menu = None;
                if let Some(&(id, ..)) = self.space_windows.iter().find(|w| w.1 == entry) {
                    return window::gain_focus(id);
                }
                // Desktop's space window is 1200 by 800 in the middle of the screen, and keeps no window state.
                let settings =
                    window::Settings { position: window::Position::Centered, ..Self::window_settings(MAIN_SIZE) };
                let (id, open) = self.open_window(settings);
                self.chat_windows.push((id, Pane { size: MAIN_SIZE, ..Pane::default() }));
                let list = chat_list::ListState::space(&entry);
                self.space_windows.push((id, entry, list));
                // It opens on the chat of the window it came from when its space has the chat, else on its first chat.
                let from = self.pane.selected_chat_id.clone();
                self.swap_window(id);
                let chat = {
                    let st = self.store.read();
                    let chats = self.listed_chats(&st);
                    let first = chats.first().map(|c| c.id.clone());
                    from.filter(|c| chats.iter().any(|v| v.id == *c)).or(first)
                };
                self.swap_window(id);
                let select = chat.map(|c| self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(c)))));
                return Task::batch([open.discard()].into_iter().chain(select));
            }
            Msg::Closed(id) if self.add_accounts.iter().any(|a| a.id == id) => {
                return self.add_account_closed(id);
            }
            Msg::Closed(id) => {
                let mut tasks = Vec::new();
                self.key_sort.take_if(|s| s.window == id);
                // Desktop's "Media in %s" window closes its carousel when it closes, which plays the inline video
                // again, and focuses its thread window.
                if let Some(opener) = self.carousel_opener(id) {
                    self.swap_window(id);
                    let close = self.carousel().is_some().then(|| self.media_view(media_view::Msg::CloseCarousel));
                    self.swap_window(id);
                    tasks.extend(close.into_iter().chain([window::gain_focus(opener)]));
                }
                // A thread window closes its "Media in %s" window.
                tasks.extend(self.carousel_windows.iter().filter(|w| w.1 == id).map(|w| window::close(w.0)));
                if let Some(w) = self.chat_windows.iter().find(|w| w.0 == id) {
                    save_draft(&mut self.store.write(), &w.1);
                }
                self.chat_windows.retain(|w| w.0 != id);
                self.space_windows.retain(|w| w.0 != id);
                self.carousel_windows.retain(|w| w.0 != id);
                // Desktop's `setPipState(null)`: the video stops with its Picture in Picture window.
                if self.pip.take_if(|p| p.0 == id).is_some() {
                    (self.video, self.pip_manual, self.pip_menu) = (None, false, None);
                }
                self.theme_windows.retain(|w| w.0 != id);
                self.labels_windows.retain(|w| w.0 != id);
                self.poll_windows.retain(|w| w.id != id);
                self.vote_windows.retain(|w| w.0 != id);
                self.label_window = self.label_window.take().filter(|d| d.id != id);
                if self.label_asking(id) {
                    self.confirm = None;
                }
                self.label_settings.retain(|w| w.id != id);
                self.spaces_window = self.spaces_window.take().filter(|d| d.id != id);
                self.merge_window = self.merge_window.take().filter(|d| d.id != id);
                self.group_window = self.group_window.take().filter(|d| d.id != id);
                self.blocked_window = self.blocked_window.filter(|w| *w != id);
                if self.networks.window.take_if(|w| *w == id).is_some() {
                    self.networks.menu = None;
                }
                self.how_window = self.how_window.filter(|w| *w != id);
                self.nickname_windows.retain(|d| d.id != id);
                self.verify.window = self.verify.window.filter(|w| *w != id);
                // Desktop's `ReauthRequiredModal` closed without a code ends the logout of a device.
                if self.reauth.window.take_if(|w| *w == id).is_some() {
                    self.devices_busy = false;
                }
                self.enter_key.window = self.enter_key.window.filter(|w| *w != id);
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                    self.time_edit = Default::default();
                    // Desktop's Settings is a new window each time, so its search starts empty.
                    (self.settings_query, self.settings_focus, self.settings_pick) = (String::new(), 0, None);
                } else if self.media_window.as_ref().is_some_and(|w| w.id == id) {
                    self.media_window = None;
                } else if [self.main_window, self.login_window].contains(&Some(id)) {
                    self.main_window = None;
                    return self.quit();
                }
                self.save_subwindows();
                return Task::batch(tasks.into_iter().chain([self.settle()]));
            }
            Msg::ScreenshotCaptured(id, shot) => {
                let is_settings = Some(id) == self.settings_window;
                let path =
                    std::env::var(if is_settings { "BEEPER_SCREENSHOT_SETTINGS" } else { "BEEPER_SCREENSHOT_EXIT" })
                        .expect("screenshot path");
                write_rgba_png(std::path::Path::new(&path), shot.size.width, shot.size.height, &shot.rgba);
                if !is_settings && std::env::var("BEEPER_SCREENSHOT_SETTINGS").is_ok() {
                    return self.open_settings(SettingsTab::General);
                }
                return iced::exit();
            }
            Msg::FocusRequested(chat) => {
                if let Some(id) = chat {
                    self.select_chat(&id);
                }
                let shown = if self.hidden { self.set_hidden(false) } else { Task::none() };
                let focus =
                    shown.chain(window::gain_focus(self.main_window.or(self.login_window).expect("a window is open")));
                return Task::batch([focus, self.load_avatars(), self.load_media(), self.close_notifications()]);
            }
            Msg::SecondInstance(args) => return self.second_instance(args),
        }
        self.settle()
    }
}

/// Desktop's default size of the main window (`Wg` and `jg`).
pub(crate) const MAIN_SIZE: Size = Size::new(1200.0, 800.0);

/// Desktop's `windowStateKeeper` key of the chat windows (`w2` of a window ID that starts with `thread-`).
pub(crate) const POP_OUT: &str = "pop-out-thread";

/// Desktop's `window.json`, or an empty state before the first save.
pub(crate) fn read_window_state(path: &std::path::Path) -> serde_json::Value {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).expect("a valid window state"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => panic!("read {}: {e}", path.display()),
    }
}

pub(crate) fn write_window_state(path: &std::path::Path, key: &str, maximized: bool, size: Size, at: Option<Point>) {
    let mut state = read_window_state(path);
    let main = &mut state["windowState"][key];
    if !maximized {
        *main = serde_json::json!({ "width": size.width, "height": size.height });
        if let Some(at) = at {
            (main["x"], main["y"]) = (at.x.into(), at.y.into());
        }
    }
    main["isMaximized"] = maximized.into();
    std::fs::write(path, state.to_string()).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Desktop's `setAppBadgeCount` and `setTrayUnreadStatus`: the unread count and the window state on the tray icon and
/// the launcher badge of Linux.
#[cfg(target_os = "linux")]
pub(crate) fn badge_sync(unread: u32, visible: bool) -> Task<UiMessage> {
    Task::future(tray::update(unread, visible))
        .map(Result::err)
        .and_then(|e| Task::done(UiMessage::Window(windows::Msg::TrayFailed(e))))
}

/// Desktop's `setAppBadgeCount`: the unread count on the Dock icon of macOS.
#[cfg(target_os = "macos")]
pub(crate) fn badge_sync(unread: u32, _visible: bool) -> Task<UiMessage> {
    tray::dock_badge(unread);
    Task::none()
}

/// Electron's `app.badgeCount` shows nothing on Windows.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn badge_sync(_unread: u32, _visible: bool) -> Task<UiMessage> {
    Task::none()
}

/// Moves window `id` of `size` over the middle of window `parent` of size `around`, like Desktop's `center: "parent"`,
/// with its left and top edges on the screen.
fn center_over(id: window::Id, parent: window::Id, around: Size, size: Size) -> Task<UiMessage> {
    let offset = Vector::new((around.width - size.width) / 2.0, (around.height - size.height) / 2.0);
    window::position(parent).then(move |at| match at {
        Some(at) => window::move_to(id, Point::new((at.x + offset.x).max(0.0), (at.y + offset.y).max(0.0))),
        None => Task::none(),
    })
}

/// Desktop's `ga`: the bounds of a window at `at` and `old` size that takes `size`, at most the `screen`, around its
/// center and on the screen.
pub(crate) fn grown(at: Point, old: Size, size: Size, screen: Size) -> (Point, Size) {
    let fit = Size::new(size.width.min(screen.width), size.height.min(screen.height));
    let place = |at: f32, old: f32, new: f32, room: f32| (at - (new - old) / 2.0).min(room - new).max(0.0).floor();
    (
        Point::new(place(at.x, old.width, fit.width, screen.width), place(at.y, old.height, fit.height, screen.height)),
        fit,
    )
}

impl BeeperGuiApp {
    /// Opens a window of `settings.size` screen pixels, like Desktop's windows, whose size does not follow the zoom.
    /// iced opens a window at its size times the scale factor, which is the zoom.
    pub(crate) fn open_window(&self, settings: window::Settings) -> (window::Id, Task<window::Id>) {
        window::open(window::Settings { size: settings.size * (1.0 / self.zoom), ..settings })
    }

    /// The size of a window in iced's logical pixels, which the zoom scales like Desktop's CSS pixels.
    fn resized(&mut self, id: window::Id, size: Size) {
        // Desktop's `KeyboardSensor` cancels on a resize of the window.
        self.key_sort.take_if(|s| s.window == id);
        if self.main_window == Some(id) {
            // Desktop's sidebar hides when the viewport gets narrower than 600 CSS pixels, and shows again when it
            // gets wider.
            let narrow = |s: Size| s.width < 600.0;
            if narrow(self.pane.size) != narrow(size) {
                self.set_sidebar_hidden(narrow(size));
            }
            self.pane.size = size;
            if let Some(t) = self.pane.thread.as_mut() {
                t.size = size;
            }
        }
        if let Some(w) = self.chat_windows.iter_mut().find(|w| w.0 == id) {
            w.1.size = size;
        }
        if let Some(w) = self.media_window.as_mut().filter(|w| w.id == id) {
            w.size = size;
        }
        if self.settings_window == Some(id) {
            self.settings_size = size;
        }
    }

    /// iced sends no resize when the zoom changes, but Desktop's viewport follows the zoom: each window takes its
    /// size at the new zoom, `k` times its size at the old one.
    pub(crate) fn rezoom(&mut self, k: f32) {
        let main = self.main_window.map(|id| (id, self.pane.size));
        let chats = self.chat_windows.iter().map(|w| (w.0, w.1.size));
        let media = self.media_window.as_ref().map(|w| (w.id, w.size));
        let settings = self.settings_window.map(|id| (id, self.settings_size));
        let all: Vec<_> = main.into_iter().chain(chats).chain(media).chain(settings).collect();
        for (id, size) in all {
            self.resized(id, size * k);
        }
    }

    /// A window of a fixed `size` over the middle of the main window, like Desktop's `center: "parent"`.
    pub(crate) fn open_child(&self, size: Size) -> (window::Id, Task<UiMessage>) {
        let (id, open) =
            self.open_window(window::Settings { resizable: false, min_size: None, ..Self::window_settings(size) });
        // The main window, or the sign-in window of the onboarding.
        let (parent, around) = match self.main_window {
            Some(main) => (main, self.pane.size * self.zoom),
            None => (self.login_window.expect("the main or the sign-in window is open"), login::SIZE),
        };
        (id, open.then(move |_| center_over(id, parent, around, size)))
    }

    pub(crate) fn window_settings(size: Size) -> window::Settings {
        window::Settings {
            size,
            min_size: Some(Size::new(330.0, 400.0)),
            exit_on_close_request: false,
            icon: Some({
                let img =
                    ::image::load_from_memory(include_bytes!("../assets/icon.png")).expect("the app icon is a PNG");
                let img = img.into_rgba8();
                let (w, h) = img.dimensions();
                window::icon::from_rgba(img.into_raw(), w, h).expect("the app icon is valid RGBA")
            }),
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: beeper_core::config::INSTALL_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Whether window `id` is a space window, whose pane is also among the chat windows.
    pub(crate) fn is_space_window(&self, id: window::Id) -> bool {
        self.space_windows.iter().any(|w| w.0 == id)
    }

    /// The thread window of the "Media in %s" window `id`, whose pane is also among the chat windows.
    pub(crate) fn carousel_opener(&self, id: window::Id) -> Option<window::Id> {
        self.carousel_windows.iter().find(|w| w.0 == id).map(|w| w.1)
    }

    /// The thread window of the "Media in %s" window that `view` draws.
    pub(crate) fn drawn_opener(&self) -> Option<window::Id> {
        self.drawing.get().and_then(|id| self.carousel_opener(id))
    }

    /// Desktop's `AttachmentCarouselBrowserWindow.show`: the carousel of thread window `opener` in its "Media in %s"
    /// window, which opens at least 800 by 600 and the size of the thread window, over its middle.
    pub(crate) fn show_carousel_window(&mut self, opener: window::Id, carousel: Carousel) -> Task<UiMessage> {
        if let Some(&(id, _)) = self.carousel_windows.iter().find(|w| w.1 == opener) {
            let pane = &mut self.chat_windows.iter_mut().find(|w| w.0 == id).expect("an open carousel window").1;
            pane.carousel = Some(carousel);
            return window::gain_focus(id);
        }
        let around =
            self.chat_windows.iter().find(|w| w.0 == opener).expect("an open thread window").1.size * self.zoom;
        let size = Size::new(around.width.max(800.0), around.height.max(600.0));
        let (id, open) = self.open_window(Self::window_settings(size));
        self.chat_windows.push((id, Pane { carousel: Some(carousel), size, ..Pane::default() }));
        self.carousel_windows.push((id, opener));
        open.then(move |_| center_over(id, opener, around, size))
    }

    /// Swaps the pane, and the chat list of a space window, of window `id` with those of the main window, around the
    /// handling of its messages.
    pub(crate) fn swap_window(&mut self, id: window::Id) {
        let pane = &mut self.chat_windows.iter_mut().find(|w| w.0 == id).expect("an open chat window").1;
        std::mem::swap(&mut self.pane, pane);
        if let Some(w) = self.space_windows.iter_mut().find(|w| w.0 == id) {
            std::mem::swap(&mut self.list, &mut w.2);
        }
    }

    /// Desktop's space window: the chat list of its space without the space bar, and the chat open in it.
    pub(crate) fn view_space_window(&self, open: bool) -> El<'_> {
        let pad = Padding::new(8.0).left(0.0);
        let right: El<'_> = match open && self.pane().thread.is_some() {
            true => iced::widget::responsive(move |size| self.view_split(size.width, pad)).into(),
            false => container(match open {
                true => self.view_conversation(),
                false => container(Space::new()).width(Fill).height(Fill).style(fill_box(self.pal.card, 10.0)).into(),
            })
            .padding(pad)
            .width(Fill)
            .into(),
        };
        let body = row![self.view_sidebar(), right];
        container(body).width(Fill).height(Fill).style(fill_box(self.pal.window, 0.0)).into()
    }

    /// The windows besides the main window, Desktop's `subWindows`.
    pub(crate) fn sub_windows(&self) -> Vec<window::Id> {
        let merge = self.merge_window.as_ref().map(|d| d.id);
        let drafts = [self.label_window.as_ref().map(|d| d.id), self.spaces_window.as_ref().map(|d| d.id), merge];
        let singles = [self.settings_window, self.media_window.as_ref().map(|w| w.id)].into_iter().chain(drafts);
        let lists = self.chat_windows.iter().map(|w| w.0).chain(self.theme_windows.iter().map(|w| w.0));
        let lists = lists.chain(self.labels_windows.iter().map(|w| w.0)).chain(self.vote_windows.iter().map(|w| w.0));
        let group = self.group_window.as_ref().map(|d| d.id);
        let lists = lists.chain(self.poll_windows.iter().map(|w| w.id)).chain(self.add_accounts.iter().map(|a| a.id));
        let lists = lists.chain(self.nickname_windows.iter().map(|d| d.id));
        singles
            .chain([group, self.blocked_window, self.how_window, self.networks.window])
            .flatten()
            .chain(lists)
            .collect()
    }

    /// Desktop's `hideToTray` (`Du`), which hides the sub-windows and then the main window, and `showWindow`, which
    /// shows the main window.
    /// Hides or shows the app: the main window, or the sign-in window while signed out (Desktop's single window).
    pub(crate) fn set_hidden(&mut self, hidden: bool) -> Task<UiMessage> {
        let Some(id) = self.main_window.or(self.login_window) else { return Task::none() };
        self.hidden = hidden;
        self.tray_sent.1 = true;
        match hidden {
            true => {
                let subs = self.sub_windows().into_iter().map(|w| window::set_mode(w, window::Mode::Hidden));
                let hide = Task::batch(subs).chain(window::set_mode(id, window::Mode::Hidden));
                self.save_main_window().chain(hide)
            }
            false => window::set_mode(id, window::Mode::Windowed).chain(window::gain_focus(id)),
        }
    }

    /// Desktop's `showAllWindows` (`hm`): the sub-windows, then the main window.
    pub(crate) fn show_all(&mut self) -> Task<UiMessage> {
        let subs = self.sub_windows().into_iter().map(|w| window::set_mode(w, window::Mode::Windowed));
        Task::batch(subs).chain(self.set_hidden(false))
    }
}

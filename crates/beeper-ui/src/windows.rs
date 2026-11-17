//! The windows: their open, close, focus, resize and maximize, the tray, the relaunch, and a second launch of the app.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Focused(bool),
    /// Maximizes or restores a chat window, or the main window when `None`.
    ToggleMaximize(Option<window::Id>),
    /// The Minimize item of the Window menu.
    Minimize(window::Id),
    Opened(window::Id),
    /// A window drew a frame, after which its screenshot has content.
    Drawn(window::Id),
    Relaunch,
    OpenChat(String),
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
                self.read_open_chat();
                let screen = self.login.screen;
                let clipboard = matches!(screen, login::Screen::RecoveryKey | login::Screen::Code);
                let key = (focused && clipboard && self.login_window.is_some())
                    .then(|| iced::clipboard::read().map(|t| LoginMsg::KeyOnFocus(t).into()));
                let modal = (focused && self.enter_key.window.is_some())
                    .then(|| iced::clipboard::read().map(|t| enter_key::KeyMsg::OnFocus(t).into()));
                return Task::batch([self.close_notifications()].into_iter().chain(key).chain(modal));
            }
            Msg::Relaunch => {
                RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                return self.quit();
            }
            Msg::Minimize(id) => return window::minimize(id, true),
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
            Msg::Resized(id, size) => {
                if self.main_window == Some(id) {
                    let narrow = |s: Size| s.width / self.zoom < 600.0;
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
                let quit = self.store.read().settings.on("quitOnClose");
                // Desktop's window `close`: it hides the app unless Quit on Close is on, also while signed out.
                let primary = Some(id) == self.main_window || Some(id) == self.login_window;
                return match (primary, quit) {
                    (true, false) => self.set_hidden(true),
                    (true, true) => self.save_main_window().chain(window::close(id)),
                    _ if self.media_window.as_ref().is_some_and(|w| w.id == id) => {
                        self.save_window(id, "media-viewer").chain(window::close(id))
                    }
                    _ if self.chat_windows.iter().any(|w| w.0 == id && w.1.thread_root.is_none()) => {
                        self.save_window(id, POP_OUT).chain(window::close(id))
                    }
                    (false, _) => window::close(id),
                };
            }
            Msg::OpenChat(cid) => {
                self.chat_menu = None;
                let open = self.chat_windows.iter().find(|w| w.1.selected_chat_id.as_deref() == Some(&cid));
                if let Some(&(id, _)) = open {
                    return window::gain_focus(id);
                }
                // Desktop's `windowStateKeeper("pop-out-thread")` keeps the size of the last chat window.
                let size = self.saved_window_settings(POP_OUT, Size::new(800.0, 600.0)).size;
                let (id, open) = window::open(Self::window_settings(size));
                self.chat_windows.push((id, Pane { size, ..Pane::default() }));
                let near = self.cascade(id);
                let select = self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(cid))));
                self.save_subwindows();
                return Task::batch([open.discard(), near, select]);
            }
            Msg::Closed(id) if self.add_accounts.iter().any(|a| a.id == id) => {
                return self.add_account_closed(id);
            }
            Msg::Closed(id) => {
                if let Some(w) = self.chat_windows.iter().find(|w| w.0 == id) {
                    save_draft(&mut self.store.write(), &w.1);
                }
                self.chat_windows.retain(|w| w.0 != id);
                // Desktop's `setPipState(null)`: the video stops with its Picture in Picture window.
                if self.pip.take_if(|p| p.0 == id).is_some() {
                    (self.video, self.pip_manual) = (None, false);
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
                self.group_window = self.group_window.take().filter(|d| d.id != id);
                self.blocked_window = self.blocked_window.filter(|w| *w != id);
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
                } else if self.media_window.as_ref().is_some_and(|w| w.id == id) {
                    self.media_window = None;
                    self.carousel = self.carousel.take().filter(|c| !c.in_media);
                } else if [self.main_window, self.login_window].contains(&Some(id)) {
                    self.main_window = None;
                    return self.quit();
                }
                self.save_subwindows();
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
    /// A window of a fixed `size` over the middle of the main window, like Desktop's `center: "parent"`.
    pub(crate) fn open_child(&self, size: Size) -> (window::Id, Task<UiMessage>) {
        let (id, open) =
            window::open(window::Settings { resizable: false, min_size: None, ..Self::window_settings(size) });
        // The main window, or the sign-in window of the onboarding.
        let (parent, around) = match self.main_window {
            Some(main) => (main, self.pane.size),
            None => (self.login_window.expect("the main or the sign-in window is open"), login::SIZE),
        };
        let offset = Vector::new((around.width - size.width) / 2.0, (around.height - size.height) / 2.0);
        // Desktop keeps the left and the top edge of a modal on the screen.
        let center = move |_| {
            window::position(parent).then(move |at| match at {
                Some(at) => window::move_to(id, Point::new((at.x + offset.x).max(0.0), (at.y + offset.y).max(0.0))),
                None => Task::none(),
            })
        };
        (id, open.then(center))
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

    /// The windows besides the main window, Desktop's `subWindows`.
    pub(crate) fn sub_windows(&self) -> Vec<window::Id> {
        let drafts = [self.label_window.as_ref().map(|d| d.id), self.spaces_window.as_ref().map(|d| d.id)];
        let singles = [self.settings_window, self.media_window.as_ref().map(|w| w.id)].into_iter().chain(drafts);
        let lists = self.chat_windows.iter().map(|w| w.0).chain(self.theme_windows.iter().map(|w| w.0));
        let lists = lists.chain(self.labels_windows.iter().map(|w| w.0)).chain(self.vote_windows.iter().map(|w| w.0));
        let group = self.group_window.as_ref().map(|d| d.id);
        let lists = lists.chain(self.poll_windows.iter().map(|w| w.id)).chain(self.add_accounts.iter().map(|a| a.id));
        let lists = lists.chain(self.nickname_windows.iter().map(|d| d.id));
        singles.chain([group, self.blocked_window]).flatten().chain(lists).collect()
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

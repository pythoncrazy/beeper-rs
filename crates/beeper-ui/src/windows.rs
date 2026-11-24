//! The windows: their open, close, focus, resize and maximize, the tray, the relaunch, and a second launch of the app.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Focused(bool),
    /// Maximizes or restores a chat window, or the main window when `None`.
    ToggleMaximize(Option<window::Id>),
    /// The Minimize item of the Window menu.
    Minimize(window::Id),
    /// A press on the background of a frameless window, which moves it.
    Drag(window::Id),
    /// Whether the main window is minimized, as the window manager says after the window lost the focus.
    Minimized(bool),
    Opened(window::Id),
    /// The scale factor of the screen of a window, as it opens or moves to another screen.
    Scale(f32),
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
    Moved(window::Id),
    /// Desktop's window state keeper saves a second after the last change, if no later change came (`.1`).
    SaveState(window::Id, u64),
    ScreenshotCaptured(window::Id, window::Screenshot),
    FocusRequested(Option<String>),
    /// Desktop's `second-instance` event with the arguments of the later launch.
    SecondInstance(Vec<String>),
    /// GTK opened to draw the title bars, or its settings changed, among them the text scaling.
    FrameReady,
    /// A window that hid on Wayland had this size in screen pixels, and was maximized or not, as it closed.
    Unmapped(window::Id, Size, bool),
    /// The widget that had the keyboard focus as the main window hid on Wayland.
    UnmappedFocus(iced::widget::Id),
    /// The app closed a window that had hidden on Wayland, which iced no longer has.
    Forget(window::Id),
}

impl BeeperGuiApp {
    pub(crate) fn windows(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::ToggleMaximize(id) => {
                return window::toggle_maximize(id.or(self.main_window).expect("the main window"));
            }
            Msg::Focused(focused) => {
                self.tray_sent.1 = true;
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
                let modal = (focused && self.children.find(|w| matches!(w, Win::EnterKey)).is_some())
                    .then(|| iced::clipboard::read().map(|t| enter_key::KeyMsg::OnFocus(t).into()));
                return Task::batch([self.close_notifications(), minimized].into_iter().chain(key).chain(modal));
            }
            Msg::Relaunch => {
                RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                return self.quit();
            }
            Msg::Drag(id) => return window::drag(id),
            Msg::Minimize(id) => {
                self.tray_sent.1 = true;
                self.minimized |= Some(id) == self.main_window;
                return window::minimize(id, true);
            }
            // A focus that came back while the window manager answered wins.
            Msg::Minimized(minimized) => {
                self.minimized = minimized && !self.focused;
                self.tray_sent.1 = true;
            }
            Msg::Opened(id) => {
                let scale = window::scale_factor(id).map(|s| UiMessage::Window(Msg::Scale(s)));
                // The serials of the input that the window gets from now on let the app raise its windows later.
                #[cfg(target_os = "linux")]
                let track = self.unmaps().then(|| window::run(id, activation::track).discard());
                let key = if Some(id) == self.settings_window { "BEEPER_SCREENSHOT_SETTINGS" } else { "BEEPER_SCREENSHOT_EXIT" };
                if std::env::var(key).is_ok() {
                    self.shot = Some(id);
                }
                #[cfg(target_os = "linux")]
                if let Some(track) = track {
                    return Task::batch([scale, track]);
                }
                return scale;
            }
            Msg::Scale(scale) => self.screen_scale = scale,
            Msg::Drawn(id) if self.shot == Some(id) => {
                self.shot = None;
                return window::screenshot(id).map(move |s| UiMessage::Window(Msg::ScreenshotCaptured(id, s)));
            }
            Msg::Drawn(_) => {}
            Msg::FrameReady => return self.follow_text_scale(),
            Msg::Resized(id, size) => {
                self.resized(id, size);
                return Task::batch([self.maximize_if_placed(id), self.save_soon(id)]);
            }
            Msg::Moved(id) => return Task::batch([self.maximize_if_placed(id), self.save_soon(id)]),
            Msg::SaveState(id, n) if n == self.state_changes => {
                return self.state_key(id).map_or_else(Task::none, |key| self.save_window(id, key));
            }
            Msg::SaveState(..) => {}
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
                    (false, _) => match self.state_key(id) {
                        Some(key) => self.save_window(id, key).chain(window::close(id)),
                        None => window::close(id),
                    },
                };
            }
            Msg::OpenChat(cid) => {
                self.list.chat_menu = None;
                let open = self.chat_windows.iter().find(|w| w.1.selected_chat_id.as_deref() == Some(&cid) && !self.is_space_window(w.0));
                if let Some(&(id, _)) = open {
                    return self.raise(id);
                }
                // Desktop's `windowStateKeeper("pop-out-thread")` keeps the size of the last chat window.
                let settings = self.kept_size(POP_OUT, Size::new(800.0, 600.0));
                let size = settings.size;
                let (id, open) = self.open_window(settings);
                self.chat_windows.push((id, Pane { size, ..Pane::default() }));
                let near = self.cascade(id);
                let select = self.update(UiMessage::InWindow(id, Box::new(UiMessage::SelectChat(cid))));
                self.save_subwindows();
                // The window shows the chat once it opens: the scroll to its unread divider needs the window's widgets.
                return Task::batch([open.discard().chain(select), near]);
            }
            Msg::OpenSpace(entry) => {
                self.list.chat_menu = None;
                if let Some(&(id, ..)) = self.space_windows.iter().find(|w| w.1 == entry) {
                    return self.raise(id);
                }
                // Desktop's space window is 1200 by 800 in the middle of the screen, and keeps no window state.
                let settings = window::Settings { position: window::Position::Centered, ..Self::window_settings(MAIN_SIZE) };
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
                return open.discard().chain(select.unwrap_or_else(Task::none));
            }
            // A window that hid on Wayland closed in iced only: the app keeps it to open it again.
            Msg::Closed(id) if self.unmapped.iter().any(|w| w.0 == id) => {}
            Msg::Forget(id) => {
                self.unmapped.retain(|w| w.0 != id);
                return self.windows(Msg::Closed(id));
            }
            Msg::Unmapped(id, size, maximized) => {
                if let Some(w) = self.unmapped.iter_mut().find(|w| w.0 == id) {
                    // A maximized window keeps the size from before, like Desktop's window state.
                    if !maximized {
                        w.1.size = size;
                    }
                    w.1.maximized = maximized;
                }
            }
            Msg::UnmappedFocus(widget) => self.unmapped_focus = Some(widget),
            Msg::Closed(id) if self.add_accounts.iter().any(|a| a.id == id) => {
                return self.add_account_closed(id);
            }
            Msg::Closed(id) => {
                self.opened.retain(|w| w.0 != id);
                let mut tasks = Vec::new();
                self.key_sort.take_if(|s| s.window == id);
                // Desktop's "Media in %s" window closes its carousel when it closes, which plays the inline video
                // again, and focuses its thread window.
                if let Some(opener) = self.carousel_opener(id) {
                    self.swap_window(id);
                    let close = self.carousel().is_some().then(|| self.media_view(media_view::Msg::CloseCarousel));
                    self.swap_window(id);
                    tasks.extend(close.into_iter().chain([self.raise(opener)]));
                }
                // A thread window closes its "Media in %s" window.
                tasks.extend(self.carousel_windows.iter().filter(|w| w.1 == id).map(|w| self.close_window(w.0)));
                if let Some(w) = self.chat_windows.iter().find(|w| w.0 == id) {
                    let mut st = self.store.write();
                    save_draft(&mut st, &w.1);
                    if let Some(chat) = w.1.selected_chat_id.as_deref() {
                        st.leave_draft(chat);
                    }
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
                self.nickname_windows.retain(|d| d.id != id);
                self.child_closed(id);
                if Some(id) == self.settings_window {
                    self.settings_window = None;
                    self.time_edit = Default::default();
                    // Desktop's Settings is a new window each time, so its search starts empty.
                    (self.settings_query, self.settings_focus, self.settings_pick) = (String::new(), 0, None);
                    self.settings_search_on = false;
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
                let path = std::env::var(if is_settings { "BEEPER_SCREENSHOT_SETTINGS" } else { "BEEPER_SCREENSHOT_EXIT" })
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
                let focus = self.set_hidden(false);
                return Task::batch([focus, self.load_avatars(), self.load_media(), self.close_notifications()]);
            }
            Msg::SecondInstance(args) => return self.second_instance(args),
        }
        self.settle()
    }
}

/// Desktop's default size of the main window (`Wg` and `jg`).
pub(crate) const MAIN_SIZE: Size = Size::new(1200.0, 800.0);

/// Desktop's `minWidth` and `minHeight` of its windows (`ji`).
pub(crate) const MIN_SIZE: Size = Size::new(330.0, 400.0);

/// Desktop's `windowStateKeeper` key of the chat windows (`w2` of a window ID that starts with `thread-`).
pub(crate) const POP_OUT: &str = "pop-out-thread";

/// Desktop's FastConf `getInitialStore`: the JSON object of the store at `path`, or an empty store before the first
/// save. A file that is not a JSON object also reads as an empty store, like FastConf's `clearInvalidConfig`, and the
/// error goes to the log.
pub(crate) fn read_store(path: &std::path::Path) -> serde_json::Value {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return serde_json::json!({}),
        Err(e) => panic!("read {}: {e}", path.display()),
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(store) if store.is_object() => store,
        Ok(_) => {
            eprintln!("{} is not a JSON object, so the app uses the defaults", path.display());
            serde_json::json!({})
        }
        Err(e) => {
            eprintln!("{} is not valid JSON ({e}), so the app uses the defaults", path.display());
            serde_json::json!({})
        }
    }
}

/// Desktop's `window.json`, or an empty state before the first save.
pub(crate) fn read_window_state(path: &std::path::Path) -> serde_json::Value {
    read_store(path)
}

pub(crate) fn write_window_state(path: &std::path::Path, key: &str, maximized: bool, size: Size, at: Option<Point>) {
    let mut state = read_window_state(path);
    // FastConf's `set` of a dotted key replaces what is not an object on its way.
    let object = |v: &mut serde_json::Value| {
        if !v.is_object() {
            *v = serde_json::json!({});
        }
    };
    object(&mut state["windowState"]);
    let main = &mut state["windowState"][key];
    object(main);
    if !maximized {
        *main = serde_json::json!({ "width": size.width, "height": size.height });
        if let Some(at) = at {
            (main["x"], main["y"]) = (at.x.into(), at.y.into());
        }
    }
    main["isMaximized"] = maximized.into();
    std::fs::write(path, state.to_string()).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// A window of Desktop's `subwindow-state.json` to open again at launch.
#[derive(Debug, PartialEq)]
pub(crate) enum Subwindow {
    /// A chat in its own window.
    Thread(String),
    /// The Settings window.
    Prefs,
}

/// Desktop's `restoreWindowsFromState` of the `openSubwindows` in `subwindow-state.json`, which skips the entries it
/// does not know and logs those that are not objects.
pub(crate) fn read_subwindows(path: &std::path::Path) -> Vec<Subwindow> {
    let store = read_store(path);
    let Some(open) = store["openSubwindows"].as_array() else { return Vec::new() };
    let known = |w: &serde_json::Value| match (w["type"].as_str(), w["threadId"].as_str()) {
        (Some("thread"), Some(chat)) if !chat.is_empty() => Some(Subwindow::Thread(chat.to_owned())),
        (Some("thread"), _) => None,
        // Desktop's other windows are parts of its settings: a subview, or an account to add or manage.
        (Some(kind), _) if kind == "prefs" || kind.starts_with("prefs-") => Some(Subwindow::Prefs),
        _ => None,
    };
    let entry = |w: &serde_json::Value| {
        if !w.is_object() {
            eprintln!("{}: skipped a subwindow that is not an object: {w}", path.display());
        }
        known(w)
    };
    open.iter().filter_map(entry).collect()
}

/// Desktop's `setAppBadgeCount` and `setTrayUnreadStatus`: the unread count and the window state on the tray icon and
/// the launcher badge of Linux.
#[cfg(target_os = "linux")]
pub(crate) fn badge_sync(unread: u32, visible: bool) -> Task<UiMessage> {
    Task::future(tray::update(unread, visible)).map(Result::err).and_then(|e| Task::done(UiMessage::Window(windows::Msg::TrayFailed(e))))
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

/// Desktop's `v3`: a sheet's size at the zoom `zoom`, in whole screen pixels.
fn zoomed(size: Size, zoom: f32) -> Size {
    Size::new((size.width * zoom).round(), (size.height * zoom).round())
}

/// Moves window `id` of `size` over the middle of window `parent` of size `around`, like Desktop's `center: "parent"`,
/// with its left and top edges on the screen.
pub(crate) fn center_over(id: window::Id, parent: window::Id, around: Size, size: Size) -> Task<UiMessage> {
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
    (Point::new(place(at.x, old.width, fit.width, screen.width), place(at.y, old.height, fit.height, screen.height)), fit)
}

impl BeeperGuiApp {
    /// Opens a window of `settings.size` screen pixels, like Desktop's windows, whose size does not follow the zoom.
    /// iced opens a window at its size times the scale factor, which is the zoom.
    ///
    /// A window that opens maximized maximizes again once the window manager has it, like Desktop's
    /// `deferMaximize`: iced maps each window only after its first frame, and an X11 window manager ignores the
    /// maximized state that winit asks for before then, so on X11 the window opened at its restored size.
    pub(crate) fn open_window(&mut self, settings: window::Settings) -> (window::Id, Task<window::Id>) {
        let id = window::Id::unique();
        (id, self.open_window_as(id, settings))
    }

    /// Desktop's `windowStateKeeper(key)` with `sizeOnly`, of the chat and Media windows: a window opens with the size,
    /// `size` by default, and the maximized state that the last one had, and its place comes from its features.
    pub(crate) fn kept_size(&self, key: &str, size: Size) -> window::Settings {
        window::Settings { position: window::Position::Default, ..self.saved_window_settings(key, size) }
    }

    /// Opens window `id` with `settings` like [`Self::open_window`]: a new window, or one that hid on Wayland again.
    fn open_window_as(&mut self, id: window::Id, mut settings: window::Settings) -> Task<window::Id> {
        let maximize = settings.maximized && settings.visible;
        if maximize {
            self.maximize_when_placed.push(id);
        }
        let icon = settings.icon.take();
        if self.unmaps() {
            self.opened.push((id, settings.clone()));
        }
        // The title bar takes its height out of the window's, like Electron's on Wayland.
        let bar = if settings.decorations { self.title_bar() } else { 0.0 };
        let page = |s: Size| Size::new(s.width, s.height - bar) * (1.0 / self.zoom);
        let settings = window::Settings { size: page(settings.size), min_size: settings.min_size.map(page), icon, ..settings };
        // iced's `window::open` with a given ID, so that all the state of a window that hid on Wayland follows it.
        iced_runtime::task::oneshot(move |done| iced_runtime::Action::Window(iced_runtime::window::Action::Open(id, settings, done)))
    }

    /// The height in Desktop's pixels of the title bar that winit adds on top of a window on Wayland. Electron draws
    /// its own title bar there, inside the window's size, so Desktop's 800 px high window has 763 px of page under a
    /// 37 px title bar. On X11 the window manager draws the title bar outside the window, for both.
    pub(crate) fn title_bar(&self) -> f32 {
        #[cfg(target_os = "linux")]
        if self.unmaps() {
            return sctk_adwaita::theme::header_size() as f32 / text_scale();
        }
        0.0
    }

    /// Whether the app hides a window by closing it and opening it again: on Wayland, where winit cannot hide a
    /// window (`set_visible` does nothing there) and Electron's `hide` unmaps it.
    pub(crate) fn unmaps(&self) -> bool {
        self.display_server == DisplayServerKind::Wayland
    }

    /// Closes window `id`. A window that hid on Wayland is closed in iced already, so the app forgets it.
    pub(crate) fn close_window(&self, id: window::Id) -> Task<UiMessage> {
        match self.unmapped.iter().any(|w| w.0 == id) {
            true => Task::done(UiMessage::Window(Msg::Forget(id))),
            false => window::close(id),
        }
    }

    /// Hides window `id`. On Wayland it closes the window in iced and keeps it, with its size and maximized state, to
    /// open it again under its ID, like Electron's `hide`, which unmaps the window.
    fn hide_window(&mut self, id: window::Id) -> Task<UiMessage> {
        if !self.unmaps() {
            return window::set_mode(id, window::Mode::Hidden);
        }
        let Some(i) = self.opened.iter().position(|w| w.0 == id) else { return Task::none() };
        let settings = self.opened.remove(i).1;
        self.unmapped.push((id, settings));
        let (zoom, bar) = (self.zoom, self.title_bar());
        let size = move |size: Size| Size::new(size.width * zoom, size.height * zoom + bar);
        let state = window::is_maximized(id).then(move |max| window::size(id).map(move |s| (size(s), max)));
        let state = state.map(move |(size, max)| UiMessage::Window(Msg::Unmapped(id, size, max)));
        // The new window has none of the widget state of the old one, so the focus goes back to the same widget.
        let focus = (Some(id) == self.main_window.or(self.login_window)).then(|| {
            let find = iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::find_focused());
            find.map(|w| UiMessage::Window(Msg::UnmappedFocus(w)))
        });
        if focus.is_some() {
            self.unmapped_focus = None;
        }
        Task::batch([state].into_iter().chain(focus)).chain(window::close(id))
    }

    /// Opens window `id` again after it hid on Wayland, at the size and maximized state that it had. The main window
    /// takes its bounds from `window.json`, like Desktop's `windowStateKeeper("main")`.
    fn map_window(&mut self, id: window::Id) -> Option<Task<UiMessage>> {
        let i = self.unmapped.iter().position(|w| w.0 == id)?;
        let mut settings = self.unmapped.remove(i).1;
        let main = Some(id) == self.main_window.or(self.login_window);
        if Some(id) == self.main_window {
            settings = self.main_window_settings();
        }
        settings = window::Settings { visible: true, icon: Self::window_settings(settings.size).icon, ..settings };
        let open = self.open_window_as(id, settings);
        let focus = self.unmapped_focus.take().filter(|_| main);
        let opened = open.then(move |id| {
            let opened = Task::done(UiMessage::Window(Msg::Opened(id)));
            match main {
                true => opened.chain(Self::keep_on_screen(id)).chain(focus.clone().map_or_else(Task::none, iced::widget::operation::focus)),
                false => opened,
            }
        });
        Some(opened.chain(self.focus_window(id)))
    }

    /// Shows window `id` and gives it the focus. A window that hid on Wayland opens again.
    pub(crate) fn raise(&mut self, id: window::Id) -> Task<UiMessage> {
        if let Some(open) = self.map_window(id) {
            return open;
        }
        let shown = window::mode(id).then(move |mode| match mode {
            window::Mode::Hidden => window::set_mode(id, window::Mode::Windowed),
            _ => Task::none(),
        });
        shown.chain(self.focus_window(id))
    }

    /// Gives window `id` the keyboard focus. winit's `focus_window` does nothing on Wayland, where the window asks the
    /// compositor with `xdg_activation_v1` instead: with the token of the later launch or the notification that shows
    /// it, else with a token for the latest input of the app, like Chromium. GNOME raises the window when a window of
    /// the app has the focus, and else says that it "is ready".
    pub(crate) fn focus_window(&mut self, id: window::Id) -> Task<UiMessage> {
        let token = self.activation_token.take();
        #[cfg(target_os = "linux")]
        if self.unmaps() {
            return window::run(id, move |w| activation::activate(w, token)).discard();
        }
        drop(token);
        window::gain_focus(id)
    }

    /// Maximizes window `id` if it opened maximized and the window manager has just placed it, which it reports with
    /// the first move or resize.
    fn maximize_if_placed(&mut self, id: window::Id) -> Task<UiMessage> {
        let before = self.maximize_when_placed.len();
        self.maximize_when_placed.retain(|w| *w != id);
        match self.maximize_when_placed.len() < before {
            true => window::maximize(id, true),
            false => Task::none(),
        }
    }

    /// The size of a window in iced's logical pixels, which the zoom scales like Desktop's CSS pixels.
    /// The key of window `id` in `window.json`: the main window, the media viewer and a chat in its own window.
    fn state_key(&self, id: window::Id) -> Option<&'static str> {
        let chat = self.chat_windows.iter().any(|w| w.0 == id && w.1.thread_root.is_none() && !w.1.info);
        match () {
            () if Some(id) == self.main_window => Some("main"),
            () if self.media_window.as_ref().is_some_and(|w| w.id == id) => Some("media-viewer"),
            () if chat && !self.is_space_window(id) && self.carousel_opener(id).is_none() => Some(POP_OUT),
            () => None,
        }
    }

    /// Desktop's `windowStateKeeper`, which saves the state of a window a second after it resizes, moves, maximizes or
    /// minimizes, so that a crash or a logout keeps it, not only a close.
    fn save_soon(&mut self, id: window::Id) -> Task<UiMessage> {
        if self.state_key(id).is_none() {
            return Task::none();
        }
        self.state_changes += 1;
        let save = UiMessage::Window(Msg::SaveState(id, self.state_changes));
        self.timers.window_state.after(std::time::Duration::from_secs(1), save)
    }

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

    /// Follows a change of GNOME's text scaling on Wayland, which Chromium makes part of the device scale: Desktop's
    /// page and title bar grow like a zoom, and its windows keep their sizes in its pixels, so they grow with it, but
    /// for maximized ones, which keep the screen's size and lay out again.
    fn follow_text_scale(&mut self) -> Task<UiMessage> {
        let scale = text_scale();
        if scale == self.text_scale {
            return Task::none();
        }
        let k = scale / self.text_scale;
        self.text_scale = scale;
        let all = self.main_window.into_iter().chain(self.login_window).chain(self.sub_windows());
        // The sizes come at the new scale, so a window of the same size at the old one grows by `k` on the screen.
        Task::batch(all.map(move |id| {
            let state = window::is_maximized(id).then(move |max| window::size(id).map(move |size| (max, size)));
            state.then(move |(max, size)| match max {
                // The title bar grew or shrank, but winit takes the content's size out of the screen's only when the
                // compositor configures the window, which GNOME does for a change of its state: restored and
                // maximized again in one go, the window does not show the restored size.
                true => Task::done(UiMessage::Window(Msg::Resized(id, size)))
                    .chain(window::maximize(id, false))
                    .chain(window::maximize(id, true)),
                false => window::resize(id, size * k),
            })
        }))
    }

    /// A window of `size` over the middle of the main window, like Desktop's `center: "parent"`, and `resizable` like
    /// Desktop's windows whose features leave out `resizable`. It is at least Desktop's minimum size: Electron gives
    /// the windows that Desktop's renderer opens its `minWidth` and `minHeight` too, so the 303 px wide Edit Labels
    /// window is 330 px wide, and a fixed size grows to it.
    pub(crate) fn open_child(&mut self, size: Size, resizable: bool) -> (window::Id, Task<UiMessage>) {
        let settings = window::Settings { resizable, min_size: resizable.then_some(MIN_SIZE), ..Self::window_settings(size) };
        self.open_over(settings, None)
    }

    /// A window of `settings` over the middle of window `parent` of its size in screen pixels, else of the main window
    /// or the sign-in window of the onboarding. It grows to Desktop's minimum size, but centers at the size that it
    /// asked for, like Desktop's `center: "parent"` and `gm`, which place the window before Electron grows it.
    pub(crate) fn open_over(&mut self, settings: window::Settings, parent: Option<(window::Id, Size)>) -> (window::Id, Task<UiMessage>) {
        let asked = settings.size;
        let (id, open) = self.open_window(window::Settings { size: asked.max(MIN_SIZE), ..settings });
        let (parent, around) = parent.unwrap_or_else(|| match self.main_window {
            Some(main) => (main, self.pane.size * self.zoom),
            None => (self.login_window.expect("the main or the sign-in window is open"), login::SIZE),
        });
        (id, open.then(move |_| center_over(id, parent, around, asked)))
    }

    /// One of Desktop's sheets that `v3` sizes (New Label, Label Settings, Edit Label, Chats and How It Works): it
    /// opens at its size times the zoom, so its page keeps its layout at any zoom.
    pub(crate) fn open_sheet(&mut self, size: Size) -> (window::Id, Task<UiMessage>) {
        self.open_child(zoomed(size, self.zoom), false)
    }

    /// The open sheets that follow the zoom (`followsZoom`), with their sizes at 100%.
    pub(crate) fn zoomed_sheets(&self) -> Vec<(window::Id, Size)> {
        let draft = self.label_window.as_ref().map(|d| (d.id, if d.label.is_some() { LABEL_SHEET_SIZE } else { LABEL_SETTINGS_SIZE }));
        let settings = self.label_settings.iter().map(|w| (w.id, LABEL_SETTINGS_SIZE));
        let how = self.children.find(|w| matches!(w, Win::How)).map(|id| (id, behavior::HOW_SIZE));
        draft.into_iter().chain(settings).chain(how).collect()
    }

    /// Desktop's `followZoom`: after a change of the zoom, each sheet takes its size at the new zoom around its center.
    pub(crate) fn follow_zoom(&self) -> Task<UiMessage> {
        Task::batch(self.zoomed_sheets().into_iter().map(|(id, size)| self.resize_around(id, zoomed(size, self.zoom))))
    }

    pub(crate) fn window_settings(size: Size) -> window::Settings {
        window::Settings {
            size,
            min_size: Some(MIN_SIZE),
            exit_on_close_request: false,
            icon: Some({
                let img = ::image::load_from_memory(include_bytes!("../assets/icon.png")).expect("the app icon is a PNG");
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
            return self.raise(id);
        }
        let around = self.chat_windows.iter().find(|w| w.0 == opener).expect("an open thread window").1.size * self.zoom;
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
        crate::ui_theme::window_bg(container(body).width(Fill).height(Fill), self.pal.backdrop, self.pal.window).into()
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
        // The verification, sign-in code and Recovery Key modals do not hide with the app.
        let children = self.children.iter().filter(|w| !matches!(w.1, Win::Verify | Win::Reauth | Win::EnterKey)).map(|w| w.0);
        singles.chain([group]).flatten().chain(lists).chain(children).collect()
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
                let subs: Vec<_> = self.sub_windows().into_iter().map(|w| self.hide_window(w)).collect();
                let hide = Task::batch(subs).chain(self.hide_window(id));
                self.save_main_window().chain(hide)
            }
            false => match self.map_window(id) {
                Some(open) => open,
                None => window::set_mode(id, window::Mode::Windowed).chain(self.focus_window(id)),
            },
        }
    }

    /// Desktop's `showAllWindows` (`hm`): the sub-windows, then the main window.
    pub(crate) fn show_all(&mut self) -> Task<UiMessage> {
        let subs: Vec<_> = self.sub_windows().into_iter().collect();
        let subs: Vec<_> =
            subs.into_iter().map(|w| self.map_window(w).unwrap_or_else(|| window::set_mode(w, window::Mode::Windowed))).collect();
        Task::batch(subs).chain(self.set_hidden(false))
    }
}

/// The Wayland handles of a window for `sctk_adwaita::activation`.
#[cfg(target_os = "linux")]
mod activation {
    use iced::window::raw_window_handle::{RawDisplayHandle, RawWindowHandle};

    fn handles(w: &dyn iced::window::Window) -> Option<(std::ptr::NonNull<std::ffi::c_void>, Option<std::ptr::NonNull<std::ffi::c_void>>)> {
        let RawDisplayHandle::Wayland(display) = w.display_handle().ok()?.as_raw() else { return None };
        let surface = match w.window_handle().ok().map(|h| h.as_raw()) {
            Some(RawWindowHandle::Wayland(h)) => Some(h.surface),
            _ => None,
        };
        Some((display.display, surface))
    }

    /// Starts following the input serials of the app's Wayland display, once.
    pub(super) fn track(w: &dyn iced::window::Window) {
        if let Some((display, _)) = handles(w) {
            // Safety: the display of the app's windows stays open while the app runs.
            unsafe { sctk_adwaita::activation::track(display) }
        }
    }

    /// Raises window `w` with `token`, or with a token for the latest input of the app.
    pub(super) fn activate(w: &dyn iced::window::Window, token: Option<String>) {
        if let Some((display, Some(surface))) = handles(w) {
            // Safety: iced runs this while the window and its surface are open.
            unsafe { sctk_adwaita::activation::activate(display, surface, token) }
        }
    }
}

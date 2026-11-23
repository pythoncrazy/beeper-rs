//! The frame painted by the GTK 3 theme, the way Electron paints its Linux window frame.
//!
//! Electron's `ClientFrameViewLinux` styles its title bar from GTK's
//! `window.background.csd > headerbar.default-decoration.titlebar` nodes, and Chromium's
//! `WindowFrameProviderGtk` and `NavButtonProviderGtk` render the `decoration` shadow and the
//! `button.titlebutton` images with `gtk_render_*`. This module does the same, drawing with
//! cairo straight into the frame's `Argb8888` buffers, so the title bar matches the desktop's
//! theme (and Beeper Desktop) instead of approximating libadwaita.
//!
//! GTK is opened at run time: nothing links against it, and a system without it keeps the
//! built-in drawing.

use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_void, CString};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};

use smithay_client_toolkit::reexports::csd_frame::WindowState;

use crate::buttons::ButtonKind;
use crate::pointer::{TitlebarAction, TitlebarActions};
use crate::theme::Metrics;

type Ptr = *mut c_void;

/// `GtkBorder`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct Border {
    left: i16,
    right: i16,
    top: i16,
    bottom: i16,
}

impl Border {
    fn width(self) -> i32 {
        i32::from(self.left) + i32::from(self.right)
    }

    fn height(self) -> i32 {
        i32::from(self.top) + i32::from(self.bottom)
    }
}

macro_rules! api {
    ($($name:ident: fn($($arg:ty),* $(; $va:tt)?) $(-> $ret:ty)?;)*) => {
        /// The GTK, GDK, cairo and pango entry points, resolved from `libgtk-3.so.0` and its
        /// dependencies.
        struct Api {
            $($name: unsafe extern "C" fn($($arg),* $(, $va)?) $(-> $ret)?,)*
        }

        impl Api {
            fn load() -> Option<Self> {
                // SAFETY: GTK's initialisers have no preconditions. The library is leaked
                // because GTK cannot be unloaded once initialised.
                let lib: &'static libloading::Library = unsafe {
                    Box::leak(Box::new(libloading::Library::new("libgtk-3.so.0").ok()?))
                };
                // SAFETY: each symbol has the C signature declared for it.
                unsafe {
                    Some(Self {
                        $($name: *lib.get(concat!(stringify!($name), "\0").as_bytes()).ok()?,)*
                    })
                }
            }
        }
    };
}

api! {
    gtk_disable_setlocale: fn();
    gtk_init_check: fn(*mut c_int, Ptr) -> c_int;
    gtk_settings_get_default: fn() -> Ptr;
    g_object_get: fn(Ptr, *const c_char; ...);
    g_free: fn(Ptr);
    g_object_unref: fn(Ptr);
    g_main_context_iteration: fn(Ptr, c_int) -> c_int;
    g_main_context_wakeup: fn(Ptr);
    gtk_widget_path_new: fn() -> Ptr;
    gtk_widget_path_copy: fn(Ptr) -> Ptr;
    gtk_widget_path_free: fn(Ptr);
    gtk_widget_path_append_type: fn(Ptr, usize) -> c_int;
    gtk_widget_path_iter_set_object_name: fn(Ptr, c_int, *const c_char);
    gtk_widget_path_iter_add_class: fn(Ptr, c_int, *const c_char);
    gtk_style_context_new: fn() -> Ptr;
    gtk_style_context_get_path: fn(Ptr) -> Ptr;
    gtk_style_context_set_path: fn(Ptr, Ptr);
    gtk_style_context_set_parent: fn(Ptr, Ptr);
    gtk_style_context_set_state: fn(Ptr, u32);
    gtk_style_context_get: fn(Ptr, u32; ...);
    gtk_style_context_get_padding: fn(Ptr, u32, *mut Border);
    gtk_style_context_get_border: fn(Ptr, u32, *mut Border);
    gtk_render_background: fn(Ptr, Ptr, c_double, c_double, c_double, c_double);
    gtk_render_frame: fn(Ptr, Ptr, c_double, c_double, c_double, c_double);
    gtk_render_layout: fn(Ptr, Ptr, c_double, c_double, Ptr);
    gtk_render_icon_surface: fn(Ptr, Ptr, Ptr, c_double, c_double);
    gtk_icon_theme_get_default: fn() -> Ptr;
    gtk_icon_theme_lookup_icon_for_scale: fn(Ptr, *const c_char, c_int, c_int, c_int) -> Ptr;
    gtk_icon_info_load_symbolic_for_context: fn(Ptr, Ptr, *mut c_int, *mut Ptr) -> Ptr;
    gdk_cairo_surface_create_from_pixbuf: fn(Ptr, c_int, Ptr) -> Ptr;
    gdk_screen_get_default: fn() -> Ptr;
    gdk_screen_get_resolution: fn(Ptr) -> c_double;
    gdk_screen_get_font_options: fn(Ptr) -> Ptr;
    cairo_image_surface_create_for_data: fn(*mut u8, c_int, c_int, c_int, c_int) -> Ptr;
    cairo_surface_set_device_scale: fn(Ptr, c_double, c_double);
    cairo_surface_flush: fn(Ptr);
    cairo_surface_destroy: fn(Ptr);
    cairo_create: fn(Ptr) -> Ptr;
    cairo_destroy: fn(Ptr);
    cairo_translate: fn(Ptr, c_double, c_double);
    pango_cairo_create_layout: fn(Ptr) -> Ptr;
    pango_cairo_context_set_resolution: fn(Ptr, c_double);
    pango_cairo_context_set_font_options: fn(Ptr, Ptr);
    pango_layout_get_context: fn(Ptr) -> Ptr;
    pango_layout_context_changed: fn(Ptr);
    pango_layout_set_text: fn(Ptr, *const c_char, c_int);
    pango_layout_set_font_description: fn(Ptr, Ptr);
    pango_layout_set_ellipsize: fn(Ptr, c_int);
    pango_layout_set_width: fn(Ptr, c_int);
    pango_layout_get_pixel_size: fn(Ptr, *mut c_int, *mut c_int);
    pango_font_description_free: fn(Ptr);
}

const STATE_ACTIVE: u32 = 1 << 0;
const STATE_PRELIGHT: u32 = 1 << 1;
const STATE_BACKDROP: u32 = 1 << 6;
const G_TYPE_NONE: usize = 1 << 2;
const CAIRO_FORMAT_ARGB32: c_int = 0;
const PANGO_ELLIPSIZE_END: c_int = 3;
const PANGO_SCALE: c_int = 1024;
const ICON_SIZE: c_int = 16;
const HEADER: &str = "headerbar.titlebar.default-decoration";

/// A title bar button's state under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonLook {
    Normal,
    Hover,
    Pressed,
}

/// One button to draw, at `x` logical points from the left of the header.
pub(crate) struct ButtonDraw {
    pub kind: ButtonKind,
    pub x: f32,
    pub look: ButtonLook,
}

/// Everything a header draw needs besides the buffer.
pub(crate) struct HeaderDraw {
    pub state: WindowState,
    pub title: String,
    pub buttons: Vec<ButtonDraw>,
    /// The x range the title may use, between the buttons, in logical points.
    pub title_room: (f32, f32),
    /// The window's height, title bar included, in logical points.
    pub window_height: u32,
}

/// A buffer of the frame: `Argb8888` pixels and its device scale.
pub(crate) struct Canvas<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    pub scale: u32,
}

struct Gtk {
    api: Api,
    /// Style contexts by CSS selector chain and state, kept for the life of the process like
    /// Chromium's `GtkCssContext` caches.
    contexts: HashMap<(String, u32), Ptr>,
    /// Symbolic icon surfaces by name, state and scale.
    icons: HashMap<(&'static str, u32, u32), Ptr>,
}

/// A job for the GTK thread.
type Job = Box<dyn FnOnce(&mut Gtk) + Send>;

/// What the GTK thread reports once GTK is open.
#[derive(Clone)]
struct Opened {
    metrics: Metrics,
    layout: Option<(String, String)>,
    titlebar: TitlebarActions,
    headerbar_rgb: u32,
    border_rgb: u32,
}

/// GTK's settings that the frame follows while it runs, like Chromium's `GtkUi`, which listens
/// to `notify::gtk-decoration-layout` and the theme's settings.
#[derive(Clone, PartialEq)]
struct Settings {
    layout: Option<(String, String)>,
    titlebar: TitlebarActions,
    /// `gtk-theme-name` and `gtk-application-prefer-dark-theme`, whose change redraws the frames.
    theme: (Option<String>, bool),
}

fn cache_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("beeper-rs").join("gtk-frame-v1.cache"));
        }
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".cache").join("beeper-rs").join("gtk-frame-v1.cache"))
}

fn action_to_str(a: TitlebarAction) -> &'static str {
    match a {
        TitlebarAction::None => "none",
        TitlebarAction::ToggleMaximize => "toggle-maximize",
        TitlebarAction::Minimize => "minimize",
        TitlebarAction::Menu => "menu",
    }
}

fn load_cache() -> Option<Opened> {
    let raw = std::fs::read_to_string(cache_path()?).ok()?;
    let p: Vec<&str> = raw.trim().split('\t').collect();
    if p.len() < 13 {
        return None;
    }
    let metrics = Metrics {
        header: p[0].parse().ok()?,
        button: (p[1].parse().ok()?, p[2].parse().ok()?),
        button_top: p[3].parse().ok()?,
        inset: (p[4].parse().ok()?, p[5].parse().ok()?),
        spacing: p[6].parse().ok()?,
    };
    let layout = if p[7].is_empty() && p[8].is_empty() {
        None
    } else {
        Some((p[7].to_owned(), p[8].to_owned()))
    };
    let defaults = TitlebarActions::default();
    let titlebar = TitlebarActions {
        double_click: TitlebarAction::parse(p[9], defaults.double_click),
        right_click: TitlebarAction::parse(p[10], defaults.right_click),
    };
    let headerbar_rgb = u32::from_str_radix(p[11], 16).ok()?;
    let border_rgb = u32::from_str_radix(p[12], 16).ok()?;
    Some(Opened { metrics, layout, titlebar, headerbar_rgb, border_rgb })
}

fn save_cache(o: &Opened) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let (l, r) = o.layout.as_ref().map_or(("", ""), |(a, b)| (a.as_str(), b.as_str()));
    let body = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{l}\t{r}\t{}\t{}\t{:06x}\t{:06x}\n",
        o.metrics.header,
        o.metrics.button.0,
        o.metrics.button.1,
        o.metrics.button_top,
        o.metrics.inset.0,
        o.metrics.inset.1,
        o.metrics.spacing,
        action_to_str(o.titlebar.double_click),
        action_to_str(o.titlebar.right_click),
        o.headerbar_rgb & 0x00ff_ffff,
        o.border_rgb & 0x00ff_ffff,
    );
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&tmp, body).and_then(|()| std::fs::rename(&tmp, &path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn cached() -> Option<&'static Opened> {
    static CACHED: OnceLock<Option<Opened>> = OnceLock::new();
    CACHED.get_or_init(load_cache).as_ref()
}

/// The thread that owns GTK. GTK is not thread safe, so every call happens there; frames hand
/// it their buffers and wait. Opening GTK takes tens of milliseconds, which [`preload`] keeps
/// off the application's startup path.
struct Worker {
    jobs: Mutex<mpsc::Sender<Job>>,
    /// Yields once, when GTK has opened or failed to.
    opening: Mutex<Option<mpsc::Receiver<Opened>>>,
    opened: OnceLock<Option<Opened>>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);
static ON_READY: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);
/// GTK's settings as they are now, once GTK is open.
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
/// Counts the changes of [`SETTINGS`]; a frame whose count is behind lays out and draws again.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// `g_main_context_wakeup`, which other threads call to hand the GTK thread a job.
static WAKEUP: OnceLock<unsafe extern "C" fn(Ptr)> = OnceLock::new();

/// Starts opening GTK in the background, so the first window's frame finds it ready.
pub fn preload() {
    let _ = cached();
    worker();
}

/// Registers a callback invoked when GTK finishes initializing and warming up, and again
/// whenever GTK's settings change (the title bar's buttons, its click actions or the theme), so
/// that the application wakes its event loop and the frames draw again.
pub fn on_ready(cb: impl Fn() + Send + 'static) {
    let ready = READY.load(Ordering::Acquire);
    if let Ok(mut slot) = ON_READY.lock() {
        let cb = slot.insert(Box::new(cb));
        if ready {
            cb();
        }
    }
}

/// Runs the callback of [`on_ready`], if there is one.
fn notify_ready() {
    if let Ok(slot) = ON_READY.lock() {
        if let Some(cb) = slot.as_ref() {
            cb();
        }
    }
}

/// How many times GTK's settings have changed; see [`GENERATION`].
pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Takes GTK's `settings`; the frames lay out and draw again when they changed. Returns the
/// settings from before.
fn publish(settings: Settings) -> Option<Option<Settings>> {
    let mut current = SETTINGS.lock().ok()?;
    if current.as_ref() == Some(&settings) {
        return None;
    }
    let old = current.replace(settings);
    drop(current);
    GENERATION.fetch_add(1, Ordering::AcqRel);
    Some(old)
}

fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let (jobs, queue) = mpsc::channel::<Job>();
        let (opened, opening) = mpsc::sync_channel(1);
        let _ = std::thread::Builder::new().name("gtk-frame".into()).spawn(move || {
            let Some(mut gtk) = Gtk::open() else {
                return;
            };
            let _ = WAKEUP.set(gtk.api.g_main_context_wakeup);
            let metrics = gtk.metrics();
            let settings = gtk.settings();
            let (headerbar_rgb, border_rgb) = gtk.warmup(&metrics);
            let (layout, titlebar) = (settings.layout.clone(), settings.titlebar);
            let mut info = Opened { metrics, layout, titlebar, headerbar_rgb, border_rgb };
            save_cache(&info);
            let _ = opened.send(info.clone());
            // The frames that opened before GTK did took the cached settings, which may be stale.
            publish(settings);
            READY.store(true, Ordering::Release);
            notify_ready();
            // GTK's main context wakes up for jobs (`call` wakes it) and for GDK's events, among
            // them the changes of the desktop's settings, which come over D-Bus.
            loop {
                while let Ok(job) = queue.try_recv() {
                    job(&mut gtk);
                }
                let settings = gtk.settings();
                let theme = settings.theme.clone();
                (info.layout, info.titlebar) = (settings.layout.clone(), settings.titlebar);
                if let Some(old) = publish(settings) {
                    if old.is_some_and(|old| old.theme != theme) {
                        gtk.forget_icons();
                    }
                    save_cache(&info);
                    notify_ready();
                }
                // SAFETY: GTK's thread blocks in its own main context until an event or a wakeup.
                unsafe { (gtk.api.g_main_context_iteration)(null_mut(), 1) };
            }
        });
        Worker { jobs: Mutex::new(jobs), opening: Mutex::new(Some(opening)), opened: OnceLock::new() }
    })
}

impl Worker {
    /// Waits for GTK to open; `None` when it cannot.
    fn opened(&self) -> Option<&Opened> {
        self.opened.get_or_init(|| self.opening.lock().ok()?.take()?.recv().ok()).as_ref()
    }

    /// Non-blocking check for live GTK state, falling back to cached metrics if available.
    fn metadata(&self) -> Option<&Opened> {
        if READY.load(Ordering::Acquire) {
            return self.opened();
        }
        if let Some(c) = cached() {
            return Some(c);
        }
        self.opened()
    }
}

/// Runs `job` on the GTK thread and waits for its result; `None` without GTK.
fn call<R: Send>(job: impl FnOnce(&mut Gtk) -> R + Send) -> Option<R> {
    let worker = worker();
    worker.opened()?;
    let (done, result) = mpsc::sync_channel(1);
    let job: Box<dyn FnOnce(&mut Gtk) + Send + '_> = Box::new(move |gtk| {
        let _ = done.send(job(gtk));
    });
    // SAFETY: only the lifetime changes. The job may borrow from the caller, and it is over
    // before this returns: `result` yields once the job has run or been dropped.
    let job: Job = unsafe { std::mem::transmute(job) };
    worker.jobs.lock().ok()?.send(job).ok()?;
    if let Some(wakeup) = WAKEUP.get() {
        // SAFETY: `g_main_context_wakeup` is thread safe, and a wakeup before GTK's thread
        // blocks makes its next iteration return at once.
        unsafe { wakeup(null_mut()) };
    }
    result.recv().ok()
}

/// The theme's title bar sizes, once GTK has opened (or from cache); `None` without GTK.
pub(crate) fn metrics() -> Option<Metrics> {
    worker().metadata().map(|opened| opened.metrics)
}

/// GTK's `gtk-decoration-layout` (`org.gnome.desktop.wm.preferences button-layout`): the
/// buttons left and right of the title, which Electron's frame shows.
pub(crate) fn decoration_layout() -> Option<(String, String)> {
    match live() {
        Some(settings) => settings.layout,
        None => worker().metadata()?.layout.clone(),
    }
}

/// What clicks on the title bar do, by GTK's settings.
pub(crate) fn titlebar_actions() -> Option<TitlebarActions> {
    match live() {
        Some(settings) => Some(settings.titlebar),
        None => Some(worker().metadata()?.titlebar),
    }
}

/// GTK's settings now, once GTK is open.
fn live() -> Option<Settings> {
    SETTINGS.lock().ok()?.clone()
}

/// Whether GTK has finished opening and warming up and can draw immediately without stalling.
pub(crate) fn ready() -> bool {
    let _ = worker();
    READY.load(Ordering::Acquire)
}

/// Whether frames are expected to draw with GTK (ready now or cached from a previous run).
pub(crate) fn expected() -> bool {
    ready() || cached().is_some()
}

/// Cached GTK headerbar & border colors (`0xRRGGBB`) for instant zero-flash first frame.
pub(crate) fn cached_colors() -> Option<(u32, u32)> {
    worker().metadata().map(|o| (o.headerbar_rgb, o.border_rgb))
}

/// Whether frames draw with GTK (blocks only if no cache exists yet).
pub(crate) fn active() -> bool {
    if ready() {
        return true;
    }
    if cached().is_some() {
        return false;
    }
    worker().opened().is_some()
}

/// Draws the title bar into `canvas`, which covers the header exactly.
pub(crate) fn draw_header(canvas: Canvas, header: &HeaderDraw) {
    call(|gtk| {
        gtk.pump();
        gtk.draw_header(canvas, header);
    });
}

/// Draws the window's shadow into `canvas`, a border part whose top left corner sits at
/// `origin` logical points from the window's top left corner (title bar included).
pub(crate) fn draw_shadow(canvas: Canvas, origin: (i32, i32), window: (u32, u32), state: WindowState) {
    call(|gtk| {
        gtk.with_cairo(canvas, |gtk, cr| {
            // SAFETY: a live cairo context.
            unsafe { (gtk.api.cairo_translate)(cr, f64::from(-origin.0), f64::from(-origin.1)) };
            gtk.decoration(cr, window, state);
        });
    });
}

impl Gtk {
    /// Opens GTK, if it is installed and can reach the display.
    fn open() -> Option<Self> {
        let api = Api::load()?;
        // SAFETY: GTK is initialised once, on this thread, before any other call. GTK must
        // not change the process locale under the application.
        unsafe {
            (api.gtk_disable_setlocale)();
            if (api.gtk_init_check)(null_mut(), null_mut()) == 0 {
                return None;
            }
        }
        Some(Self { api, contexts: HashMap::new(), icons: HashMap::new() })
    }

    /// A string setting of GTK's.
    fn setting(&self, name: &std::ffi::CStr) -> Option<String> {
        let api = &self.api;
        let mut value: *mut c_char = null_mut();
        // SAFETY: a string property, copied out and freed.
        unsafe {
            let settings = (api.gtk_settings_get_default)();
            if settings.is_null() {
                return None;
            }
            (api.g_object_get)(settings, name.as_ptr(), &mut value as *mut *mut c_char, null_mut::<c_void>());
            if value.is_null() {
                return None;
            }
            let text = std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned();
            (api.g_free)(value.cast());
            Some(text)
        }
    }

    /// The settings the frame follows, as GTK has them now.
    fn settings(&self) -> Settings {
        Settings {
            layout: self.setting(c"gtk-decoration-layout").map(|layout| split_layout(&layout)),
            titlebar: self.titlebar_actions(),
            theme: (self.setting(c"gtk-theme-name"), self.flag(c"gtk-application-prefer-dark-theme")),
        }
    }

    /// A boolean setting of GTK's.
    fn flag(&self, name: &std::ffi::CStr) -> bool {
        let mut value: c_int = 0;
        // SAFETY: a boolean property, written to `value`.
        unsafe {
            let settings = (self.api.gtk_settings_get_default)();
            if settings.is_null() {
                return false;
            }
            (self.api.g_object_get)(settings, name.as_ptr(), &mut value as *mut c_int, null_mut::<c_void>());
        }
        value != 0
    }

    /// Drops the icons drawn in the colors of the theme from before.
    fn forget_icons(&mut self) {
        for (_, surface) in self.icons.drain().filter(|(_, surface)| !surface.is_null()) {
            // SAFETY: each surface was made for the cache, which held the only reference.
            unsafe { (self.api.cairo_surface_destroy)(surface) };
        }
    }

    fn titlebar_actions(&self) -> TitlebarActions {
        let defaults = TitlebarActions::default();
        let action = |name, default| self.setting(name).map_or(default, |value| TitlebarAction::parse(&value, default));
        TitlebarActions {
            double_click: action(c"gtk-titlebar-double-click", defaults.double_click),
            right_click: action(c"gtk-titlebar-right-click", defaults.right_click),
        }
    }

    /// Pre-warms style contexts, Pango/FontConfig, and button symbolic icons on the background
    /// thread so the first live `draw_header` takes < 1 ms, and samples the theme's headerbar
    /// background & bottom-border colors (`0xRRGGBB`) for instant zero-flash Frame 0 rendering.
    fn warmup(&mut self, metrics: &Metrics) -> (u32, u32) {
        let h = metrics.header.max(1);
        let w = 64_u32;
        let mut pixels = vec![0_u8; (w * h * 4) as usize];
        let header = HeaderDraw {
            state: WindowState::ACTIVATED,
            title: "W".into(),
            buttons: vec![
                ButtonDraw { kind: ButtonKind::Minimize, x: 0.0, look: ButtonLook::Normal },
                ButtonDraw { kind: ButtonKind::Maximize, x: 20.0, look: ButtonLook::Normal },
                ButtonDraw { kind: ButtonKind::Close, x: 40.0, look: ButtonLook::Normal },
            ],
            title_room: (0.0, 64.0),
            window_height: h + 100,
        };
        let canvas = Canvas { pixels: &mut pixels, width: w, height: h, scale: 1 };
        self.with_cairo(canvas, |gtk, cr| {
            let bar = gtk.context(&format!("{} {HEADER}", window_node(WindowState::ACTIVATED)), 0);
            // SAFETY: rendering into a live cairo context.
            unsafe {
                (gtk.api.gtk_render_background)(bar, cr, 0., 0., f64::from(w), f64::from(h));
                (gtk.api.gtk_render_frame)(bar, cr, 0., 0., f64::from(w), f64::from(h));
            }
        });
        let sample = |row: u32| -> u32 {
            // Cairo ARGB32 on little-endian is BGRA in memory.
            let off = ((row.min(h - 1) * w + (w / 2)) * 4) as usize;
            let (b, g, r) = (pixels[off] as u32, pixels[off + 1] as u32, pixels[off + 2] as u32);
            (r << 16) | (g << 8) | b
        };
        let hdr_rgb = sample(h / 2);
        let border_rgb = sample(h - 1);
        // Warm Pango + button symbolic icons off the main thread.
        let canvas = Canvas { pixels: &mut pixels, width: w, height: h, scale: 1 };
        self.draw_header(canvas, &header);
        (hdr_rgb, border_rgb)
    }

    /// Handles GDK's pending display events (theme and setting changes) without blocking.
    fn pump(&self) {
        // SAFETY: a non-blocking iteration of the default main context.
        unsafe { while (self.api.g_main_context_iteration)(null_mut(), 0) != 0 {} }
    }

    /// A style context for `chain`, CSS nodes separated by spaces (`name.class.class`), each
    /// in `state`. Chromium's `AppendCssNodeToStyleContext`.
    fn context(&mut self, chain: &str, state: u32) -> Ptr {
        if let Some(ctx) = self.contexts.get(&(chain.to_owned(), state)) {
            return *ctx;
        }
        let parent = match chain.rsplit_once(' ') {
            // Ancestors share only the backdrop state: hovering a button is not hovering the bar.
            Some((parent, _)) => self.context(parent, state & STATE_BACKDROP),
            None => null_mut(),
        };
        let node = chain.rsplit(' ').next().unwrap_or(chain);
        let api = &self.api;
        // SAFETY: plain GTK object construction; `parent` is a live context or null.
        let ctx = unsafe {
            let path = if parent.is_null() {
                (api.gtk_widget_path_new)()
            } else {
                (api.gtk_widget_path_copy)((api.gtk_style_context_get_path)(parent))
            };
            let mut parts = node.split('.');
            let pos = (api.gtk_widget_path_append_type)(path, G_TYPE_NONE);
            let name = CString::new(parts.next().unwrap_or_default()).unwrap_or_default();
            (api.gtk_widget_path_iter_set_object_name)(path, pos, name.as_ptr());
            for class in parts {
                let class = CString::new(class).unwrap_or_default();
                (api.gtk_widget_path_iter_add_class)(path, pos, class.as_ptr());
            }
            let ctx = (api.gtk_style_context_new)();
            (api.gtk_style_context_set_path)(ctx, path);
            (api.gtk_widget_path_free)(path);
            if !parent.is_null() {
                (api.gtk_style_context_set_parent)(ctx, parent);
            }
            (api.gtk_style_context_set_state)(ctx, state);
            ctx
        };
        self.contexts.insert((chain.to_owned(), state), ctx);
        ctx
    }

    fn int(&self, ctx: Ptr, state: u32, property: &std::ffi::CStr) -> i32 {
        let mut value: c_int = 0;
        // SAFETY: `property` is an integer CSS property, written to `value`.
        unsafe {
            (self.api.gtk_style_context_get)(
                ctx,
                state,
                property.as_ptr(),
                &mut value as *mut c_int,
                null_mut::<c_void>(),
            );
        }
        value
    }

    fn border(&self, ctx: Ptr, state: u32, get: unsafe extern "C" fn(Ptr, u32, *mut Border)) -> Border {
        let mut border = Border::default();
        // SAFETY: `get` fills a `GtkBorder`.
        unsafe { get(ctx, state, &mut border) };
        border
    }

    /// The box sizes of a node: min size plus padding and border.
    fn size(&self, ctx: Ptr, state: u32) -> (i32, i32) {
        let padding = self.border(ctx, state, self.api.gtk_style_context_get_padding);
        let border = self.border(ctx, state, self.api.gtk_style_context_get_border);
        (
            self.int(ctx, state, c"min-width") + padding.width() + border.width(),
            self.int(ctx, state, c"min-height") + padding.height() + border.height(),
        )
    }

    fn metrics(&mut self) -> Metrics {
        let header = self.context(&format!("{} {HEADER}", window_node(WindowState::ACTIVATED)), 0);
        let padding = self.border(header, 0, self.api.gtk_style_context_get_padding);
        let border = self.border(header, 0, self.api.gtk_style_context_get_border);
        let (_, header_height) = self.size(header, 0);
        let button = self.context(&button_chain(WindowState::ACTIVATED, ButtonKind::Close), 0);
        let (button_w, button_h) = self.size(button, 0);
        let content = header_height - padding.height() - border.height();
        Metrics {
            header: header_height.max(1) as u32,
            button: (button_w.max(ICON_SIZE) as f32, button_h.max(ICON_SIZE) as f32),
            button_top: (i32::from(padding.top) + i32::from(border.top)) as f32
                + (content - button_h).max(0) as f32 / 2.,
            inset: (
                (i32::from(padding.left) + i32::from(border.left)) as f32,
                (i32::from(padding.right) + i32::from(border.right)) as f32,
            ),
            // `GtkHeaderBar`'s default `spacing` style property.
            spacing: 6.,
        }
    }

    /// Runs `draw` with a cairo context over `canvas`, in logical points.
    fn with_cairo(&mut self, canvas: Canvas, draw: impl FnOnce(&mut Self, Ptr)) {
        let api = &self.api;
        // SAFETY: the surface borrows `canvas.pixels` (an `Argb8888` buffer of exactly
        // `width * height` pixels) only until it is destroyed below.
        let (surface, cr) = unsafe {
            let surface = (api.cairo_image_surface_create_for_data)(
                canvas.pixels.as_mut_ptr(),
                CAIRO_FORMAT_ARGB32,
                canvas.width as c_int,
                canvas.height as c_int,
                canvas.width as c_int * 4,
            );
            (api.cairo_surface_set_device_scale)(surface, f64::from(canvas.scale), f64::from(canvas.scale));
            (surface, (api.cairo_create)(surface))
        };
        draw(self, cr);
        let api = &self.api;
        // SAFETY: both were created above.
        unsafe {
            (api.cairo_destroy)(cr);
            (api.cairo_surface_flush)(surface);
            (api.cairo_surface_destroy)(surface);
        }
    }

    /// Chromium's `WindowFrameProviderGtk`: the `decoration` node's shadow and outline around
    /// the window box at the origin.
    fn decoration(&mut self, cr: Ptr, (width, height): (u32, u32), state: WindowState) {
        let flags = backdrop(state);
        let ctx = self.context(&format!("{} decoration", window_node(state)), flags);
        // SAFETY: rendering into a live cairo context.
        unsafe {
            (self.api.gtk_render_background)(ctx, cr, 0., 0., f64::from(width), f64::from(height));
            (self.api.gtk_render_frame)(ctx, cr, 0., 0., f64::from(width), f64::from(height));
        }
    }

    fn draw_header(&mut self, canvas: Canvas, header: &HeaderDraw) {
        let scale = canvas.scale;
        let width = f64::from(canvas.width / scale);
        let height = f64::from(canvas.height / scale);
        self.with_cairo(canvas, |gtk, cr| {
            let state = header.state;
            let flags = backdrop(state);
            let window = window_node(state);
            // The shadow shows through the header's rounded corners.
            if !state.contains(WindowState::MAXIMIZED) {
                gtk.decoration(cr, (width as u32, header.window_height), state);
            }
            let bar = gtk.context(&format!("{window} {HEADER}"), flags);
            // SAFETY: rendering into a live cairo context.
            unsafe {
                (gtk.api.gtk_render_background)(bar, cr, 0., 0., width, height);
                (gtk.api.gtk_render_frame)(bar, cr, 0., 0., width, height);
            }
            gtk.title(cr, &format!("{window} {HEADER} label.title"), flags, header, width, height);
            for button in &header.buttons {
                gtk.button(cr, state, button, height, scale);
            }
        });
    }

    /// The title, centred on the whole bar like `GtkHeaderBar` when it fits between the
    /// buttons, else pushed beside them and ellipsized.
    fn title(&mut self, cr: Ptr, chain: &str, flags: u32, header: &HeaderDraw, width: f64, height: f64) {
        if header.title.is_empty() {
            return;
        }
        let ctx = self.context(chain, flags);
        let api = &self.api;
        let (start, end) = (f64::from(header.title_room.0), f64::from(header.title_room.1));
        if end <= start {
            return;
        }
        let text = CString::new(header.title.replace('\0', "")).unwrap_or_default();
        // SAFETY: a pango layout on a live cairo context, freed before returning.
        unsafe {
            let layout = (api.pango_cairo_create_layout)(cr);
            let context = (api.pango_layout_get_context)(layout);
            let screen = (api.gdk_screen_get_default)();
            if !screen.is_null() {
                (api.pango_cairo_context_set_resolution)(context, (api.gdk_screen_get_resolution)(screen));
                let options = (api.gdk_screen_get_font_options)(screen);
                if !options.is_null() {
                    (api.pango_cairo_context_set_font_options)(context, options);
                }
                (api.pango_layout_context_changed)(layout);
            }
            let mut font: Ptr = null_mut();
            (api.gtk_style_context_get)(ctx, flags, c"font".as_ptr(), &mut font as *mut Ptr, null_mut::<c_void>());
            if !font.is_null() {
                (api.pango_layout_set_font_description)(layout, font);
                (api.pango_font_description_free)(font);
            }
            (api.pango_layout_set_text)(layout, text.as_ptr(), -1);
            (api.pango_layout_set_ellipsize)(layout, PANGO_ELLIPSIZE_END);
            (api.pango_layout_set_width)(layout, ((end - start) as c_int) * PANGO_SCALE);
            let (mut w, mut h) = (0, 0);
            (api.pango_layout_get_pixel_size)(layout, &mut w, &mut h);
            let (w, h) = (f64::from(w), f64::from(h));
            let x = ((width - w) / 2.).clamp(start, (end - w).max(start));
            (api.gtk_render_layout)(ctx, cr, x.round(), ((height - h) / 2.).round(), layout);
            (api.g_object_unref)(layout);
        }
    }

    fn button(&mut self, cr: Ptr, state: WindowState, button: &ButtonDraw, height: f64, scale: u32) {
        let flags = backdrop(state)
            | match button.look {
                ButtonLook::Normal => 0,
                ButtonLook::Hover => STATE_PRELIGHT,
                ButtonLook::Pressed => STATE_PRELIGHT | STATE_ACTIVE,
            };
        let ctx = self.context(&button_chain(state, button.kind), flags);
        let (w, h) = self.size(ctx, flags);
        let (w, h) = (f64::from(w.max(ICON_SIZE)), f64::from(h.max(ICON_SIZE)));
        let x = f64::from(button.x);
        let y = ((height - h) / 2.).floor();
        let name = icon(button.kind, state);
        let icon = self.icon(ctx, name, flags, scale);
        let api = &self.api;
        // SAFETY: rendering into a live cairo context.
        unsafe {
            (api.gtk_render_background)(ctx, cr, x, y, w, h);
            (api.gtk_render_frame)(ctx, cr, x, y, w, h);
            if !icon.is_null() {
                let size = f64::from(ICON_SIZE);
                (api.gtk_render_icon_surface)(
                    ctx,
                    cr,
                    icon,
                    x + ((w - size) / 2.).floor(),
                    y + ((h - size) / 2.).floor(),
                );
            }
        }
    }

    /// The symbolic icon `name`, recoloured for the button's state like
    /// `NavButtonProviderGtk`.
    fn icon(&mut self, ctx: Ptr, name: &'static str, flags: u32, scale: u32) -> Ptr {
        if let Some(icon) = self.icons.get(&(name, flags, scale)) {
            return *icon;
        }
        let api = &self.api;
        let cname = CString::new(name).unwrap_or_default();
        // SAFETY: GTK icon lookup; the pixbuf and info are released once the surface exists.
        let surface = unsafe {
            let info = (api.gtk_icon_theme_lookup_icon_for_scale)(
                (api.gtk_icon_theme_get_default)(),
                cname.as_ptr(),
                ICON_SIZE,
                scale as c_int,
                0,
            );
            if info.is_null() {
                null_mut()
            } else {
                let mut symbolic = 0;
                let mut error = null_mut();
                let pixbuf = (api.gtk_icon_info_load_symbolic_for_context)(info, ctx, &mut symbolic, &mut error);
                (api.g_object_unref)(info);
                if pixbuf.is_null() {
                    null_mut()
                } else {
                    let surface = (api.gdk_cairo_surface_create_from_pixbuf)(pixbuf, scale as c_int, null_mut());
                    (api.g_object_unref)(pixbuf);
                    surface
                }
            }
        };
        self.icons.insert((name, flags, scale), surface);
        surface
    }
}

/// `gtk-decoration-layout`'s buttons left and right of the title.
fn split_layout(layout: &str) -> (String, String) {
    let (left, right) = layout.split_once(':').unwrap_or(("", layout));
    (left.to_owned(), right.to_owned())
}

/// The `window` node with GTK's classes for the window's state.
fn window_node(state: WindowState) -> String {
    let mut node = String::from("window.background.csd");
    if state.contains(WindowState::MAXIMIZED) {
        node.push_str(".maximized");
    }
    if state.intersects(WindowState::TILED) {
        node.push_str(".tiled");
        for (edge, class) in [
            (WindowState::TILED_TOP, ".tiled-top"),
            (WindowState::TILED_RIGHT, ".tiled-right"),
            (WindowState::TILED_BOTTOM, ".tiled-bottom"),
            (WindowState::TILED_LEFT, ".tiled-left"),
        ] {
            if state.contains(edge) {
                node.push_str(class);
            }
        }
    }
    node
}

fn button_chain(state: WindowState, kind: ButtonKind) -> String {
    let class = match kind {
        ButtonKind::Close => "close",
        ButtonKind::Maximize => "maximize",
        ButtonKind::Minimize => "minimize",
    };
    format!("{} {HEADER} button.titlebutton.{class}", window_node(state))
}

fn icon(kind: ButtonKind, state: WindowState) -> &'static str {
    match kind {
        ButtonKind::Close => "window-close-symbolic",
        ButtonKind::Maximize if state.contains(WindowState::MAXIMIZED) => "window-restore-symbolic",
        ButtonKind::Maximize => "window-maximize-symbolic",
        ButtonKind::Minimize => "window-minimize-symbolic",
    }
}

fn backdrop(state: WindowState) -> u32 {
    if state.contains(WindowState::ACTIVATED) {
        0
    } else {
        STATE_BACKDROP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change of GTK's settings counts once, so the frames lay out again once, and the same
    /// settings again change nothing.
    #[test]
    fn publish_counts_each_change_once() {
        let settings = |layout: &str| Settings {
            layout: Some(split_layout(layout)),
            titlebar: TitlebarActions::default(),
            theme: (Some("Adwaita".into()), false),
        };
        let start = generation();
        assert!(publish(settings("menu:close")).is_some());
        assert!(publish(settings("menu:close")).is_none(), "the same settings again are no change");
        assert_eq!(generation(), start + 1);
        let old = publish(settings("close,minimize:")).expect("a new layout is a change");
        assert_eq!(old.and_then(|s| s.layout), Some(("menu".into(), "close".into())));
        assert_eq!(generation(), start + 2);
        assert_eq!(decoration_layout(), Some(("close,minimize".into(), String::new())), "the frames read it");
    }
}

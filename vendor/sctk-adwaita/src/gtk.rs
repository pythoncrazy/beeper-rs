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
use std::ptr::null_mut;
use std::sync::{mpsc, Mutex, OnceLock};

use smithay_client_toolkit::reexports::csd_frame::WindowState;

use crate::buttons::ButtonKind;
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
struct Opened {
    metrics: Metrics,
    layout: Option<(String, String)>,
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

/// Starts opening GTK in the background, so the first window's frame finds it ready.
pub fn preload() {
    worker();
}

fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let (jobs, queue) = mpsc::channel::<Job>();
        let (opened, opening) = mpsc::sync_channel(1);
        let _ = std::thread::Builder::new().name("gtk-frame".into()).spawn(move || {
            let Some(mut gtk) = Gtk::open() else {
                return;
            };
            let _ = opened.send(Opened { metrics: gtk.metrics(), layout: gtk.decoration_layout() });
            for job in queue {
                job(&mut gtk);
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
    result.recv().ok()
}

/// The theme's title bar sizes, once GTK has opened; `None` without GTK.
pub(crate) fn metrics() -> Option<Metrics> {
    worker().opened().map(|opened| opened.metrics)
}

/// GTK's `gtk-decoration-layout` (`org.gnome.desktop.wm.preferences button-layout`): the
/// buttons left and right of the title, which Electron's frame shows.
pub(crate) fn decoration_layout() -> Option<(String, String)> {
    worker().opened()?.layout.clone()
}

/// Whether frames draw with GTK.
pub(crate) fn active() -> bool {
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

    fn decoration_layout(&self) -> Option<(String, String)> {
        let api = &self.api;
        let mut layout: *mut c_char = null_mut();
        // SAFETY: a string property, copied out and freed.
        let layout = unsafe {
            let settings = (api.gtk_settings_get_default)();
            if settings.is_null() {
                return None;
            }
            (api.g_object_get)(
                settings,
                c"gtk-decoration-layout".as_ptr(),
                &mut layout as *mut *mut c_char,
                null_mut::<c_void>(),
            );
            if layout.is_null() {
                return None;
            }
            let text = std::ffi::CStr::from_ptr(layout).to_string_lossy().into_owned();
            (api.g_free)(layout.cast());
            text
        };
        let (left, right) = layout.split_once(':').unwrap_or(("", &layout));
        Some((left.to_owned(), right.to_owned()))
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

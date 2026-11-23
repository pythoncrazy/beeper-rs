use std::error::Error;
use std::mem;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use tiny_skia::{Color, FillRule, Mask, Path, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Point, Rect, Transform};

use smithay_client_toolkit::reexports::client::backend::ObjectId;
use smithay_client_toolkit::reexports::client::protocol::wl_shm;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::csd_frame::{
    CursorIcon, DecorationsFrame, FrameAction, FrameClick, WindowManagerCapabilities, WindowState,
};

use smithay_client_toolkit::compositor::{CompositorState, Region, SurfaceData};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{slot::SlotPool, Shm};
use smithay_client_toolkit::subcompositor::SubcompositorState;
use smithay_client_toolkit::subcompositor::SubsurfaceData;

mod buttons;
mod config;
mod gtk;
mod parts;
mod pointer;
mod shadow;
pub mod theme;

pub use gtk::{on_ready, preload};

static ACTIVATION_TOKEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Saves a Wayland startup activation token (`XDG_ACTIVATION_TOKEN` / `DESKTOP_STARTUP_ID`)
/// so the first created window immediately consumes it via `xdg_activation_v1.activate`.
pub fn set_activation_token(token: String) {
    if !token.is_empty() {
        if let Ok(mut slot) = ACTIVATION_TOKEN.lock() {
            *slot = Some(token);
        }
    }
}

fn consume_activation_token(surface: &WlSurface) {
    use smithay_client_toolkit::reexports::client::{protocol::wl_registry, Connection};
    use smithay_client_toolkit::reexports::protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

    let token = ACTIVATION_TOKEN
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .or_else(|| {
            std::env::var("XDG_ACTIVATION_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| std::env::var("DESKTOP_STARTUP_ID").ok().filter(|s| !s.is_empty()))
        });
    let Some(token) = token else { return };
    let Some(backend) = surface.backend().upgrade() else { return };

    struct ActivationBind {
        activation: Option<XdgActivationV1>,
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for ActivationBind {
        fn event(
            state: &mut Self,
            reg: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global { name, interface, .. } = event {
                if interface == XdgActivationV1::interface().name && state.activation.is_none() {
                    state.activation = Some(reg.bind::<XdgActivationV1, _, _>(name, 1, qh, ()));
                }
            }
        }
    }
    impl Dispatch<XdgActivationV1, ()> for ActivationBind {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: <XdgActivationV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    let conn = Connection::from_backend(backend);
    let mut eq = conn.new_event_queue::<ActivationBind>();
    let qh = eq.handle();
    let _reg = conn.display().get_registry(&qh, ());
    let mut state = ActivationBind { activation: None };
    let _ = eq.roundtrip(&mut state);
    if let Some(act) = state.activation {
        act.activate(token, surface);
        act.destroy();
        let _ = conn.flush();
    }
}
mod title;
mod wl_typed;

use crate::theme::{
    header_size, ColorMap, ColorTheme, BORDER_SIZE, CORNER_RADIUS, RESIZE_HANDLE_CORNER_SIZE, VISIBLE_BORDER_SIZE,
};

use buttons::{ButtonKind, Buttons};
use config::get_button_layout_config;
use parts::DecorationParts;
use pointer::{Location, MouseState};
use shadow::Shadow;
use title::TitleText;
use wl_typed::WlTyped;

/// XXX this is not result, so `must_use` when needed.
type SkiaResult = Option<()>;

/// A simple set of decorations
#[derive(Debug)]
pub struct AdwaitaFrame<State> {
    /// The base surface used to create the window.
    base_surface: WlTyped<WlSurface, SurfaceData>,

    compositor: Arc<CompositorState>,

    /// Subcompositor to create/drop subsurfaces ondemand.
    subcompositor: Arc<SubcompositorState>,

    /// Queue handle to perform object creation.
    queue_handle: QueueHandle<State>,

    /// The drawable decorations, `None` when hidden.
    decorations: Option<DecorationParts>,

    /// Memory pool to allocate the buffers for the decorations.
    pool: SlotPool,

    /// Whether the frame should be redrawn.
    dirty: bool,

    /// Whether the drawing should be synced with the main surface.
    should_sync: bool,

    /// Scale factor used for the surface.
    scale_factor: u32,

    /// Wether the frame is resizable.
    resizable: bool,

    buttons: Buttons,
    state: WindowState,
    wm_capabilities: WindowManagerCapabilities,
    mouse: MouseState,
    theme: ColorTheme,
    title: Option<String>,
    title_text: Option<TitleText>,
    shadow: Shadow,
    gtk_drawn: bool,
}

impl<State> AdwaitaFrame<State>
where
    State: Dispatch<WlSurface, SurfaceData> + Dispatch<WlSubsurface, SubsurfaceData> + 'static,
{
    pub fn new(
        base_surface: &impl WaylandSurface,
        shm: &Shm,
        compositor: Arc<CompositorState>,
        subcompositor: Arc<SubcompositorState>,
        queue_handle: QueueHandle<State>,
        frame_config: FrameConfig,
    ) -> Result<Self, Box<dyn Error>> {
        consume_activation_token(base_surface.wl_surface());
        let base_surface = WlTyped::wrap::<State>(base_surface.wl_surface().clone());
        // Opens GTK, when there is one to draw with, before anything is sized.
        theme::metrics();

        let pool = SlotPool::new(1, shm)?;

        let decorations = Some(DecorationParts::new(&base_surface, &subcompositor, &queue_handle));

        let theme = system_theme(frame_config);

        Ok(AdwaitaFrame {
            base_surface,
            decorations,
            pool,
            compositor,
            subcompositor,
            queue_handle,
            dirty: true,
            scale_factor: 1,
            should_sync: true,
            title: None,
            title_text: (!gtk::expected())
                .then(|| TitleText::new(theme.active.font_color))
                .flatten(),
            theme,
            buttons: Buttons::new(gtk::decoration_layout().or_else(get_button_layout_config)),
            mouse: MouseState::new(gtk::titlebar_actions().unwrap_or_default()),
            state: WindowState::empty(),
            wm_capabilities: WindowManagerCapabilities::all(),
            resizable: true,
            shadow: Shadow::default(),
            gtk_drawn: false,
        })
    }

    /// Update the current frame config.
    pub fn set_config(&mut self, config: FrameConfig) {
        self.theme = system_theme(config);
        self.dirty = true;
    }

    /// What the GTK title bar shows now.
    fn header_draw(&self, width: u32, window_height: u32) -> gtk::HeaderDraw {
        let metrics = theme::metrics();
        let width = width as f32;
        let buttons = self
            .buttons
            .visible(0., width)
            .map(|button| {
                let kind = button.kind();
                let enabled = self.resizable || kind != ButtonKind::Maximize;
                let look = match self.mouse.location {
                    Location::Button(at) if at == kind && enabled => {
                        if self.mouse.pressed == Some(kind) {
                            gtk::ButtonLook::Pressed
                        } else {
                            gtk::ButtonLook::Hover
                        }
                    }
                    _ => gtk::ButtonLook::Normal,
                };
                gtk::ButtonDraw { kind, x: button.x(), look }
            })
            .collect();
        let start = self.buttons.left_buttons_end_x().map_or(metrics.inset.0, |x| x + metrics.spacing);
        let end = self.buttons.right_buttons_start_x().map_or(width - metrics.inset.1, |x| x - metrics.spacing);
        gtk::HeaderDraw {
            state: self.state,
            title: self.title.clone().unwrap_or_default(),
            buttons,
            title_room: (start, end),
            window_height,
        }
    }

    fn precise_location(&self, location: Location, decoration: &DecorationParts, x: f64, y: f64) -> Location {
        let header_width = decoration.header().surface_rect.width;
        let side_height = decoration.side_height();

        let left_corner_x = BORDER_SIZE + RESIZE_HANDLE_CORNER_SIZE;
        let right_corner_x = (header_width + BORDER_SIZE).saturating_sub(RESIZE_HANDLE_CORNER_SIZE);
        let top_corner_y = RESIZE_HANDLE_CORNER_SIZE;
        let bottom_corner_y = side_height.saturating_sub(RESIZE_HANDLE_CORNER_SIZE);
        match location {
            Location::Head | Location::Button(_) => self.buttons.find_button(x, y),
            Location::Top | Location::TopLeft | Location::TopRight => {
                if x <= f64::from(left_corner_x) {
                    Location::TopLeft
                } else if x >= f64::from(right_corner_x) {
                    Location::TopRight
                } else {
                    Location::Top
                }
            }
            Location::Bottom | Location::BottomLeft | Location::BottomRight => {
                if x <= f64::from(left_corner_x) {
                    Location::BottomLeft
                } else if x >= f64::from(right_corner_x) {
                    Location::BottomRight
                } else {
                    Location::Bottom
                }
            }
            Location::Left => {
                if y <= f64::from(top_corner_y) {
                    Location::TopLeft
                } else if y >= f64::from(bottom_corner_y) {
                    Location::BottomLeft
                } else {
                    Location::Left
                }
            }
            Location::Right => {
                if y <= f64::from(top_corner_y) {
                    Location::TopRight
                } else if y >= f64::from(bottom_corner_y) {
                    Location::BottomRight
                } else {
                    Location::Right
                }
            }
            other => other,
        }
    }

    fn redraw_inner(&mut self) -> Option<bool> {
        let decorations = self.decorations.as_ref()?;

        // Reset the dirty bit.
        self.dirty = false;
        let should_sync = mem::take(&mut self.should_sync);

        // Don't draw borders if the frame explicitly hidden or fullscreened.
        if self.state.contains(WindowState::FULLSCREEN) {
            decorations.hide();
            return Some(true);
        }

        let colors =
            if self.state.contains(WindowState::ACTIVATED) { &self.theme.active } else { &self.theme.inactive };

        let draw_borders = if self.state.contains(WindowState::MAXIMIZED) {
            // Don't draw the borders.
            decorations.hide_borders();
            false
        } else {
            true
        };
        let border_paint = colors.border_paint();
        let gtk = gtk::active();
        if gtk {
            self.gtk_drawn = true;
        }
        let window_width = decorations.header().surface_rect.width;
        let window_height = decorations.side_height();
        let header = header_size();
        let header_draw = gtk.then(|| self.header_draw(window_width, window_height));

        // Draw the borders.
        for (idx, part) in decorations.parts().filter(|(idx, _)| *idx == DecorationParts::HEADER || draw_borders) {
            let scale = self.scale_factor;

            let mut rect = part.surface_rect;
            // XXX to perfectly align the visible borders we draw them with
            // the header, otherwise rounded corners won't look 'smooth' at the
            // start. To achieve that, we enlargen the width of the header by
            // 2 * `VISIBLE_BORDER_SIZE`, and move `x` by `VISIBLE_BORDER_SIZE`
            // to the left.
            if idx == DecorationParts::HEADER && draw_borders && !gtk {
                rect.width += 2 * VISIBLE_BORDER_SIZE;
                rect.x -= VISIBLE_BORDER_SIZE as i32;
            }

            rect.width *= scale;
            rect.height *= scale;

            let (buffer, canvas) = match self.pool.create_buffer(
                rect.width as i32,
                rect.height as i32,
                rect.width as i32 * 4,
                wl_shm::Format::Argb8888,
            ) {
                Ok((buffer, canvas)) => (buffer, canvas),
                Err(_) => continue,
            };

            if gtk {
                canvas.fill(0);
                let canvas = gtk::Canvas { pixels: canvas, width: rect.width, height: rect.height, scale };
                if let (DecorationParts::HEADER, Some(header_draw)) = (idx, &header_draw) {
                    gtk::draw_header(canvas, header_draw);
                } else {
                    let origin = (part.surface_rect.x, part.surface_rect.y + header as i32);
                    gtk::draw_shadow(canvas, origin, (window_width, window_height), self.state);
                }
            } else {
                // Create the pixmap and fill with transparent color.
                let mut pixmap = PixmapMut::from_bytes(canvas, rect.width, rect.height)?;

                // Fill everything with transparent background, since we draw rounded corners and
                // do invisible borders to enlarge the input zone.
                pixmap.fill(Color::TRANSPARENT);

                if !self.state.intersects(WindowState::TILED) {
                    self.shadow.draw(&mut pixmap, scale, self.state.contains(WindowState::ACTIVATED), idx);
                }

                match idx {
                    DecorationParts::HEADER => {
                        if let Some(title_text) = self.title_text.as_mut() {
                            title_text.update_scale(scale);
                            title_text.update_color(colors.font_color);
                        }

                        draw_headerbar(
                            &mut pixmap,
                            self.title_text.as_ref().map(|t| t.pixmap()).unwrap_or(None),
                            scale as f32,
                            self.resizable,
                            &self.state,
                            &self.theme,
                            &self.buttons,
                            self.mouse.location,
                        );
                    }
                    border => {
                        // The visible border is one pt.
                        let visible_border_size = VISIBLE_BORDER_SIZE * scale;

                        // XXX we do all the match using integral types and then convert to f32 in the
                        // end to ensure that result is finite.
                        let border_rect = match border {
                            DecorationParts::LEFT => {
                                let x = (rect.x.unsigned_abs() * scale) - visible_border_size;
                                let y = rect.y.unsigned_abs() * scale;
                                Rect::from_xywh(
                                    x as f32,
                                    y as f32,
                                    visible_border_size as f32,
                                    (rect.height - y) as f32,
                                )
                            }
                            DecorationParts::RIGHT => {
                                let y = rect.y.unsigned_abs() * scale;
                                Rect::from_xywh(0., y as f32, visible_border_size as f32, (rect.height - y) as f32)
                            }
                            // We draw small visible border only bellow the window surface, no need to
                            // handle `TOP`.
                            DecorationParts::BOTTOM => {
                                let x = (rect.x.unsigned_abs() * scale) - visible_border_size;
                                Rect::from_xywh(x as f32, 0., (rect.width - 2 * x) as f32, visible_border_size as f32)
                            }
                            _ => None,
                        };

                        // Fill the visible border, if present.
                        if let Some(border_rect) = border_rect {
                            pixmap.fill_rect(border_rect, &border_paint, Transform::identity(), None);
                        }
                    }
                };
            }

            if should_sync {
                part.subsurface.set_sync();
            } else {
                part.subsurface.set_desync();
            }

            part.surface.set_buffer_scale(scale as i32);

            part.subsurface.set_position(rect.x, rect.y);
            buffer.attach_to(&part.surface).ok()?;

            if part.surface.version() >= 4 {
                part.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
            } else {
                part.surface.damage(0, 0, i32::MAX, i32::MAX);
            }

            if let Some(input_rect) = part.input_rect {
                let input_region = Region::new(&*self.compositor).ok()?;
                input_region.add(input_rect.x, input_rect.y, input_rect.width as i32, input_rect.height as i32);

                part.surface.set_input_region(Some(input_region.wl_region()));
            }

            part.surface.commit();
        }

        Some(should_sync)
    }
}

impl<State> DecorationsFrame for AdwaitaFrame<State>
where
    State: Dispatch<WlSurface, SurfaceData> + Dispatch<WlSubsurface, SubsurfaceData> + 'static,
{
    fn update_state(&mut self, state: WindowState) {
        let difference = self.state.symmetric_difference(state);
        self.state = state;
        self.dirty |= difference
            .intersects(WindowState::ACTIVATED | WindowState::FULLSCREEN | WindowState::MAXIMIZED | WindowState::TILED);
    }

    fn update_wm_capabilities(&mut self, wm_capabilities: WindowManagerCapabilities) {
        self.dirty |= self.wm_capabilities != wm_capabilities;
        self.wm_capabilities = wm_capabilities;
        self.buttons.update_wm_capabilities(wm_capabilities);
        if let Some(decorations) = &self.decorations {
            self.buttons.arrange(decorations.header().surface_rect.width, get_margin_h_lp(&self.state));
        }
    }

    fn set_hidden(&mut self, hidden: bool) {
        if hidden {
            self.dirty = false;
            let _ = self.pool.resize(1);
            self.decorations = None;
        } else if self.decorations.is_none() {
            self.decorations = Some(DecorationParts::new(&self.base_surface, &self.subcompositor, &self.queue_handle));
            self.dirty = true;
            self.should_sync = true;
        }
    }

    fn set_resizable(&mut self, resizable: bool) {
        self.dirty |= self.resizable != resizable;
        self.resizable = resizable;
    }

    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        let Some(decorations) = self.decorations.as_mut() else {
            log::error!("trying to resize the hidden frame.");
            return;
        };

        decorations.resize(width.get(), height.get());
        self.buttons.arrange(width.get(), get_margin_h_lp(&self.state));
        self.dirty = true;
        self.should_sync = true;
    }

    fn draw(&mut self) -> bool {
        self.redraw_inner().unwrap_or(true)
    }

    fn subtract_borders(&self, width: NonZeroU32, height: NonZeroU32) -> (Option<NonZeroU32>, Option<NonZeroU32>) {
        if self.decorations.is_none() || self.state.contains(WindowState::FULLSCREEN) {
            (Some(width), Some(height))
        } else {
            (Some(width), NonZeroU32::new(height.get().saturating_sub(header_size())))
        }
    }

    fn add_borders(&self, width: u32, height: u32) -> (u32, u32) {
        if self.decorations.is_none() || self.state.contains(WindowState::FULLSCREEN) {
            (width, height)
        } else {
            (width, height + header_size())
        }
    }

    fn location(&self) -> (i32, i32) {
        if self.decorations.is_none() || self.state.contains(WindowState::FULLSCREEN) {
            (0, 0)
        } else {
            (0, -(header_size() as i32))
        }
    }

    fn set_title(&mut self, title: impl Into<String>) {
        let new_title = title.into();
        if let Some(title_text) = self.title_text.as_mut() {
            title_text.update_title(new_title.clone());
        }

        self.title = Some(new_title);
        self.dirty = true;
    }

    fn on_click(&mut self, timestamp: Duration, click: FrameClick, pressed: bool) -> Option<FrameAction> {
        let was_pressed = self.mouse.pressed;
        let action = match click {
            FrameClick::Normal => {
                self.mouse.click(timestamp, pressed, self.resizable, &self.state, &self.wm_capabilities)
            }
            FrameClick::Alternate => self.mouse.alternate_click(pressed, &self.state, &self.wm_capabilities),
            _ => None,
        };
        self.dirty |= was_pressed != self.mouse.pressed;
        action
    }

    fn set_scaling_factor(&mut self, scale_factor: f64) {
        // NOTE: Clamp it just in case to some ok-ish range.
        self.scale_factor = scale_factor.clamp(0.1, 64.).ceil() as u32;
        self.dirty = true;
        self.should_sync = true;
    }

    fn click_point_moved(&mut self, _timestamp: Duration, surface: &ObjectId, x: f64, y: f64) -> Option<CursorIcon> {
        let decorations = self.decorations.as_ref()?;
        let location = decorations.find_surface(surface);
        if location == Location::None {
            return None;
        }

        let old_location = self.mouse.location;

        let location = self.precise_location(location, decorations, x, y);
        let new_cursor = self.mouse.moved(location, x, y, self.resizable);

        // Set dirty if we moved the cursor between the buttons.
        self.dirty |= (matches!(old_location, Location::Button(_))
            || matches!(self.mouse.location, Location::Button(_)))
            && old_location != self.mouse.location;

        Some(new_cursor)
    }

    fn click_point_left(&mut self) {
        self.dirty |= matches!(self.mouse.location, Location::Button(_));
        self.mouse.left()
    }

    fn is_dirty(&self) -> bool {
        self.dirty || (!self.gtk_drawn && gtk::ready())
    }

    fn is_hidden(&self) -> bool {
        self.decorations.is_none()
    }
}

/// The configuration for the [`AdwaitaFrame`] frame.
#[derive(Debug, Clone)]
pub struct FrameConfig {
    pub theme: ColorTheme,
}

impl FrameConfig {
    /// Create the new configuration with the given `theme`.
    pub fn new(theme: ColorTheme) -> Self {
        Self { theme }
    }

    /// This is equivalent of calling `FrameConfig::new(ColorTheme::auto())`.
    ///
    /// For details see [`ColorTheme::auto`].
    pub fn auto() -> Self {
        Self { theme: ColorTheme::auto() }
    }

    /// This is equivalent of calling `FrameConfig::new(ColorTheme::light())`.
    ///
    /// For details see [`ColorTheme::light`].
    pub fn light() -> Self {
        Self { theme: ColorTheme::light() }
    }

    /// This is equivalent of calling `FrameConfig::new(ColorTheme::dark())`.
    ///
    /// For details see [`ColorTheme::dark`].
    pub fn dark() -> Self {
        Self { theme: ColorTheme::dark() }
    }
}

/// The frame's colors: the desktop's rather than the ones the application asks for, like
/// Electron's frame, which Beeper Desktop never themes. GTK's theme decides them when GTK draws:
/// Electron does not follow the portal's color scheme either, only the GTK theme.
fn system_theme(_config: FrameConfig) -> ColorTheme {
    if let Some((hdr_rgb, border_rgb)) = gtk::cached_colors() {
        let rgb = |v: u32| Color::from_rgba8(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8, 255);
        let luma = ((hdr_rgb >> 16) & 0xff) * 3 + ((hdr_rgb >> 8) & 0xff) * 6 + (hdr_rgb & 0xff);
        let mut t = if luma < 1280 { ColorTheme::dark() } else { ColorTheme::light() };
        t.active.headerbar = rgb(hdr_rgb);
        t.inactive.headerbar = rgb(hdr_rgb);
        t.active.border_color = rgb(border_rgb);
        t.inactive.border_color = rgb(border_rgb);
        t
    } else if gtk::active() {
        // GTK's theme draws the frame; skip asking the desktop again.
        ColorTheme::light()
    } else {
        ColorTheme::auto()
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_headerbar(
    pixmap: &mut PixmapMut,
    text_pixmap: Option<&Pixmap>,
    scale: f32,
    resizable: bool,
    state: &WindowState,
    theme: &ColorTheme,
    buttons: &Buttons,
    mouse: Location,
) {
    let colors = theme.for_state(state.contains(WindowState::ACTIVATED));

    let _ = draw_headerbar_bg(pixmap, scale, colors, state);

    // Horizontal margin.
    let margin_h = get_margin_h_lp(state) * 2.0;

    let canvas_w = pixmap.width() as f32;
    let canvas_h = pixmap.height() as f32;

    let header_w = canvas_w - margin_h * 2.0;
    let header_h = canvas_h;

    if let Some(text_pixmap) = text_pixmap {
        const TEXT_OFFSET: f32 = 10.;
        let offset_x = TEXT_OFFSET * scale;

        let text_w = text_pixmap.width() as f32;
        let text_h = text_pixmap.height() as f32;

        let x = margin_h + header_w / 2. - text_w / 2.;
        let y = header_h / 2. - text_h / 2.;

        let left_buttons_end_x = buttons.left_buttons_end_x().unwrap_or(0.0) * scale;
        let right_buttons_start_x = buttons.right_buttons_start_x().unwrap_or(header_w / scale) * scale;

        {
            // We have enough space to center text
            let (x, y, text_canvas_start_x) =
                if (x + text_w < right_buttons_start_x - offset_x) && (x > left_buttons_end_x + offset_x) {
                    let text_canvas_start_x = x;

                    (x, y, text_canvas_start_x)
                } else {
                    let x = left_buttons_end_x + offset_x;
                    let text_canvas_start_x = left_buttons_end_x + offset_x;

                    (x, y, text_canvas_start_x)
                };

            let text_canvas_end_x = right_buttons_start_x - x - offset_x;
            // Ensure that text start within the bounds.
            let x = x.max(margin_h + offset_x);

            if let Some(clip) = Rect::from_xywh(text_canvas_start_x, 0., text_canvas_end_x, canvas_h) {
                if let Some(mut mask) = Mask::new(canvas_w as u32, canvas_h as u32) {
                    mask.fill_path(&PathBuilder::from_rect(clip), FillRule::Winding, false, Transform::identity());
                    pixmap.draw_pixmap(
                        x.round() as i32,
                        y as i32,
                        text_pixmap.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        Some(&mask),
                    );
                } else {
                    log::error!("Invalid mask width and height: w: {}, h: {}", canvas_w as u32, canvas_h as u32);
                }
            }
        }
    }

    // Draw the buttons.
    buttons.draw(margin_h, header_w, scale, colors, mouse, pixmap, resizable, state);
}

#[must_use]
fn draw_headerbar_bg(pixmap: &mut PixmapMut, scale: f32, colors: &ColorMap, state: &WindowState) -> SkiaResult {
    let w = pixmap.width() as f32;
    let h = pixmap.height() as f32;

    let radius =
        if state.intersects(WindowState::MAXIMIZED | WindowState::TILED) { 0. } else { CORNER_RADIUS as f32 * scale };

    let bg = rounded_headerbar_shape(0., 0., w, h, radius)?;

    pixmap.fill_path(&bg, &colors.headerbar_paint(), FillRule::Winding, Transform::identity(), None);

    pixmap.fill_rect(Rect::from_xywh(0., h - 1., w, h)?, &colors.border_paint(), Transform::identity(), None);

    Some(())
}

fn rounded_headerbar_shape(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<Path> {
    // https://stackoverflow.com/a/27863181
    let cubic_bezier_circle = 0.552_284_8 * radius;

    let mut pb = PathBuilder::new();
    let mut cursor = Point::from_xy(x, y);

    // !!!
    // This code is heavily "inspired" by https://gitlab.com/snakedye/snui/
    // So technically it should be licensed under MPL-2.0, sorry about that 🥺 👉👈
    // !!!

    // Positioning the cursor
    cursor.y += radius;
    pb.move_to(cursor.x, cursor.y);

    // Drawing the outline
    let next = Point::from_xy(cursor.x + radius, cursor.y - radius);
    pb.cubic_to(cursor.x, cursor.y - cubic_bezier_circle, next.x - cubic_bezier_circle, next.y, next.x, next.y);
    cursor = next;
    pb.line_to(
        {
            cursor.x = x + width - radius;
            cursor.x
        },
        cursor.y,
    );
    let next = Point::from_xy(cursor.x + radius, cursor.y + radius);
    pb.cubic_to(cursor.x + cubic_bezier_circle, cursor.y, next.x, next.y - cubic_bezier_circle, next.x, next.y);
    cursor = next;
    pb.line_to(cursor.x, {
        cursor.y = y + height;
        cursor.y
    });
    pb.line_to(
        {
            cursor.x = x;
            cursor.x
        },
        cursor.y,
    );

    pb.close();

    pb.finish()
}

// returns horizontal margin, logical points
fn get_margin_h_lp(state: &WindowState) -> f32 {
    // GTK's header covers exactly the window's width.
    if gtk::active() || state.intersects(WindowState::MAXIMIZED | WindowState::TILED) {
        0.
    } else {
        VISIBLE_BORDER_SIZE as f32
    }
}

pub use tiny_skia::Color;
use tiny_skia::{Paint, Shader};

// https://gitlab.gnome.org/GNOME/gtk/-/blob/1bf88f1d81043fd99740e2f91e56ade7ede7303b/gtk/gtkwindow.c#L165
pub(crate) const RESIZE_HANDLE_SIZE: u32 = 12;
// https://gitlab.gnome.org/GNOME/gtk/-/blob/1bf88f1d81043fd99740e2f91e56ade7ede7303b/gtk/gtkwindow.c#L166
pub(crate) const RESIZE_HANDLE_CORNER_SIZE: u32 = 24;
pub(crate) const BORDER_SIZE: u32 = crate::shadow::SHADOW_SIZE + VISIBLE_BORDER_SIZE;
pub(crate) const CORNER_RADIUS: u32 = 10;
pub(crate) const VISIBLE_BORDER_SIZE: u32 = 1;

/// The title bar's sizes in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Metrics {
    /// The title bar's height.
    pub header: u32,
    /// A button's width and height.
    pub button: (f32, f32),
    /// A button's distance from the top of the bar.
    pub button_top: f32,
    /// The space before the first button on the left and after the last on the right.
    pub inset: (f32, f32),
    /// The space between two buttons.
    pub spacing: f32,
}

/// The built-in libadwaita look's sizes.
pub(crate) const ADWAITA: Metrics =
    Metrics { header: 35, button: (24., 24.), button_top: 5., inset: (5., 5.), spacing: 13. };

static METRICS: std::sync::OnceLock<Metrics> = std::sync::OnceLock::new();

/// The sizes the frames of this process use: the GTK theme's when GTK draws them (see
/// [`crate::gtk`]), else libadwaita's. Fixed by the first frame.
pub(crate) fn metrics() -> Metrics {
    *METRICS.get_or_init(|| crate::gtk::metrics().unwrap_or(ADWAITA))
}

/// The title bar's height in logical points.
pub(crate) fn header_size() -> u32 {
    metrics().header
}

/// The color theme to use with the decorations frame.
#[derive(Debug, Clone)]
pub struct ColorTheme {
    pub active: ColorMap,
    pub inactive: ColorMap,
}

impl ColorTheme {
    /// Automatically choose between light & dark themes based on:
    /// * dbus org.freedesktop.portal.Settings
    ///   <https://flatpak.github.io/xdg-desktop-portal/#gdbus-interface-org-freedesktop-portal-Settings>
    pub fn auto() -> Self {
        match crate::config::prefer_dark() {
            true => Self::dark(),
            false => Self::light(),
        }
    }

    /// Predefined light variant, which aims to replecate Adwaita theme.
    pub fn light() -> Self {
        Self {
            active: ColorMap {
                headerbar: Color::from_rgba8(235, 235, 235, 255),
                button_idle: Color::from_rgba8(216, 216, 216, 255),
                button_hover: Color::from_rgba8(207, 207, 207, 255),
                button_icon: Color::from_rgba8(42, 42, 42, 255),
                border_color: Color::from_rgba8(220, 220, 220, 255),
                font_color: Color::from_rgba8(47, 47, 47, 255),
            },
            inactive: ColorMap {
                headerbar: Color::from_rgba8(250, 250, 250, 255),
                button_idle: Color::from_rgba8(240, 240, 240, 255),
                button_hover: Color::from_rgba8(216, 216, 216, 255),
                button_icon: Color::from_rgba8(148, 148, 148, 255),
                border_color: Color::from_rgba8(220, 220, 220, 255),
                font_color: Color::from_rgba8(150, 150, 150, 255),
            },
        }
    }

    /// Predefined dark variant, which aims to replecate Adwaita-dark theme.
    pub fn dark() -> Self {
        Self {
            active: ColorMap {
                headerbar: Color::from_rgba8(48, 48, 48, 255),
                button_idle: Color::from_rgba8(69, 69, 69, 255),
                button_hover: Color::from_rgba8(79, 79, 79, 255),
                button_icon: Color::from_rgba8(255, 255, 255, 255),
                border_color: Color::from_rgba8(58, 58, 58, 255),
                font_color: Color::from_rgba8(255, 255, 255, 255),
            },
            inactive: ColorMap {
                headerbar: Color::from_rgba8(36, 36, 36, 255),
                button_idle: Color::from_rgba8(47, 47, 47, 255),
                button_hover: Color::from_rgba8(57, 57, 57, 255),
                button_icon: Color::from_rgba8(144, 144, 144, 255),
                border_color: Color::from_rgba8(58, 58, 58, 255),
                font_color: Color::from_rgba8(144, 144, 144, 255),
            },
        }
    }

    pub(crate) fn for_state(&self, active: bool) -> &ColorMap {
        if active {
            &self.active
        } else {
            &self.inactive
        }
    }
}

impl Default for ColorTheme {
    fn default() -> Self {
        Self::auto()
    }
}

/// The color map for various decorcation parts.
#[derive(Debug, Clone)]
pub struct ColorMap {
    pub headerbar: Color,
    pub button_idle: Color,
    pub button_hover: Color,
    pub button_icon: Color,
    pub border_color: Color,
    pub font_color: Color,
}

impl ColorMap {
    pub(crate) fn headerbar_paint(&self) -> Paint<'_> {
        Paint { shader: Shader::SolidColor(self.headerbar), anti_alias: true, ..Default::default() }
    }

    pub(crate) fn button_idle_paint(&self) -> Paint<'_> {
        Paint { shader: Shader::SolidColor(self.button_idle), anti_alias: true, ..Default::default() }
    }

    pub(crate) fn button_hover_paint(&self) -> Paint<'_> {
        Paint { shader: Shader::SolidColor(self.button_hover), anti_alias: true, ..Default::default() }
    }

    pub(crate) fn button_icon_paint(&self) -> Paint<'_> {
        Paint { shader: Shader::SolidColor(self.button_icon), ..Default::default() }
    }

    pub(crate) fn border_paint(&self) -> Paint<'_> {
        Paint { shader: Shader::SolidColor(self.border_color), ..Default::default() }
    }
}

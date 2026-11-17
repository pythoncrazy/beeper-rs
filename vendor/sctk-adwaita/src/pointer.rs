use std::time::Duration;

use smithay_client_toolkit::reexports::csd_frame::{
    CursorIcon, FrameAction, ResizeEdge, WindowManagerCapabilities, WindowState,
};

use crate::{
    buttons::ButtonKind,
    theme::{header_size, BORDER_SIZE},
};

/// Time to register the next click as a double click.
///
/// The value is the same as the default in gtk4.
const DOUBLE_CLICK_DURATION: Duration = Duration::from_millis(400);

/// The state of the mouse input inside the decorations frame.
#[derive(Debug, Default)]
pub(crate) struct MouseState {
    pub location: Location,

    /// The surface local location inside the surface.
    position: (f64, f64),

    /// The instant of the last click.
    last_normal_click: Option<Duration>,

    /// The button under a press that has not been released yet.
    pub pressed: Option<ButtonKind>,

    /// What clicks on the title bar do.
    pub titlebar: TitlebarActions,
}

/// What a double click and a right click on the title bar do: GTK's `gtk-titlebar-double-click`
/// and `gtk-titlebar-right-click`, which Chromium's `GtkUi::GetWindowFrameAction` reads for
/// Electron.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TitlebarActions {
    pub double_click: TitlebarAction,
    pub right_click: TitlebarAction,
}

impl Default for TitlebarActions {
    fn default() -> Self {
        Self { double_click: TitlebarAction::ToggleMaximize, right_click: TitlebarAction::Menu }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitlebarAction {
    None,
    ToggleMaximize,
    Minimize,
    Menu,
}

impl TitlebarAction {
    /// A GTK setting's value, or `default` for one Chromium does not know. Wayland cannot
    /// lower a window, so `lower` does nothing.
    pub fn parse(value: &str, default: Self) -> Self {
        match value {
            "none" | "lower" => Self::None,
            "toggle-maximize" => Self::ToggleMaximize,
            "minimize" => Self::Minimize,
            "menu" => Self::Menu,
            _ => default,
        }
    }

    fn act(
        self,
        position: (f64, f64),
        state: &WindowState,
        wm_capabilities: &WindowManagerCapabilities,
    ) -> Option<FrameAction> {
        let can = |capability| wm_capabilities.contains(capability);
        match self {
            Self::ToggleMaximize if can(WindowManagerCapabilities::MAXIMIZE) => {
                Some(if state.contains(WindowState::MAXIMIZED) {
                    FrameAction::UnMaximize
                } else {
                    FrameAction::Maximize
                })
            }
            Self::Minimize if can(WindowManagerCapabilities::MINIMIZE) => Some(FrameAction::Minimize),
            Self::Menu if can(WindowManagerCapabilities::WINDOW_MENU) => Some(FrameAction::ShowMenu(
                // XXX this could be one 1pt off when the frame is not maximized, but it's not
                // like it really matters in the end.
                position.0 as i32 - BORDER_SIZE as i32,
                // We must offset it by header size for precise position.
                position.1 as i32 - header_size() as i32,
            )),
            _ => None,
        }
    }
}

impl MouseState {
    pub fn new(titlebar: TitlebarActions) -> Self {
        Self { titlebar, ..Self::default() }
    }

    /// The normal click on decorations frame was made.
    pub fn click(
        &mut self,
        timestamp: Duration,
        pressed: bool,
        resizable: bool,
        state: &WindowState,
        wm_capabilities: &WindowManagerCapabilities,
    ) -> Option<FrameAction> {
        let maximized = state.contains(WindowState::MAXIMIZED);
        // Like GTK, a button acts when released over the button it was pressed on.
        let pressed_button = match self.location {
            Location::Button(kind) if pressed => Some(kind),
            _ => None,
        };
        let released_on = |kind| !pressed && self.pressed == Some(kind);
        let action = match self.location {
            Location::Top if resizable => FrameAction::Resize(ResizeEdge::Top),
            Location::TopLeft if resizable => FrameAction::Resize(ResizeEdge::TopLeft),
            Location::Left if resizable => FrameAction::Resize(ResizeEdge::Left),
            Location::BottomLeft if resizable => FrameAction::Resize(ResizeEdge::BottomLeft),
            Location::Bottom if resizable => FrameAction::Resize(ResizeEdge::Bottom),
            Location::BottomRight if resizable => FrameAction::Resize(ResizeEdge::BottomRight),
            Location::Right if resizable => FrameAction::Resize(ResizeEdge::Right),
            Location::TopRight if resizable => FrameAction::Resize(ResizeEdge::TopRight),
            Location::Button(ButtonKind::Close) if released_on(ButtonKind::Close) => FrameAction::Close,
            Location::Button(ButtonKind::Maximize) if released_on(ButtonKind::Maximize) && !maximized => {
                FrameAction::Maximize
            }
            Location::Button(ButtonKind::Maximize) if released_on(ButtonKind::Maximize) && maximized => {
                FrameAction::UnMaximize
            }
            Location::Button(ButtonKind::Minimize) if released_on(ButtonKind::Minimize) => FrameAction::Minimize,
            Location::Head if pressed => match self.last_normal_click.replace(timestamp) {
                Some(last) if timestamp.saturating_sub(last) < DOUBLE_CLICK_DURATION => {
                    self.last_normal_click = None;
                    self.titlebar.double_click.act(self.position, state, wm_capabilities)?
                }
                _ => FrameAction::Move,
            },
            _ => {
                self.pressed = pressed_button;
                return None;
            }
        };
        self.pressed = pressed_button;

        Some(action)
    }

    /// Alternative click on decorations frame was made.
    pub fn alternate_click(
        &mut self,
        pressed: bool,
        state: &WindowState,
        wm_capabilities: &WindowManagerCapabilities,
    ) -> Option<FrameAction> {
        // Invalidate the normal click.
        self.last_normal_click = None;

        match self.location {
            Location::Head | Location::Button(_) if pressed => {
                self.titlebar.right_click.act(self.position, state, wm_capabilities)
            }
            _ => None,
        }
    }

    /// The mouse moved inside the decorations frame.
    pub fn moved(&mut self, location: Location, x: f64, y: f64, resizable: bool) -> CursorIcon {
        self.location = location;
        self.position = (x, y);
        match self.location {
            _ if !resizable => CursorIcon::Default,
            Location::Top => CursorIcon::NResize,
            Location::TopRight => CursorIcon::NeResize,
            Location::Right => CursorIcon::EResize,
            Location::BottomRight => CursorIcon::SeResize,
            Location::Bottom => CursorIcon::SResize,
            Location::BottomLeft => CursorIcon::SwResize,
            Location::Left => CursorIcon::WResize,
            Location::TopLeft => CursorIcon::NwResize,
            _ => CursorIcon::Default,
        }
    }

    /// The mouse left the decorations frame.
    pub fn left(&mut self) {
        // Reset only the location and the pressed button.
        self.location = Location::None;
        self.pressed = None;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum Location {
    #[default]
    None,
    Head,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
    Button(ButtonKind),
}

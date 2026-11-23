//! Raising a window on Wayland with `xdg_activation_v1`, which winit 0.30 does not do: its `focus_window` is empty
//! there.
//!
//! Like GTK and Chromium, the app asks the compositor for a token that carries the serial of the latest input event
//! that one of its windows got, with that window and the seat, and activates the window to raise with the token.
//! winit keeps the serials to itself, so this binds its own `wl_seat` on the app's connection and listens to its own
//! pointer and keyboard, which get the same events as winit's. A compositor raises the window only when the serial is
//! one it knows: mutter wants the serial of the keyboard focus or of a key or button of the focused window, and else
//! only marks the window as demanding attention (GNOME's "is ready" notification).
//!
//! A token from elsewhere, like the launcher's `XDG_ACTIVATION_TOKEN` of a later launch or the `ActivationToken` of a
//! notification, activates the window as it is.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

use smithay_client_toolkit::reexports::client::backend::{Backend, ObjectId};
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::{self, WlKeyboard};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::{self, WlPointer};
use smithay_client_toolkit::reexports::client::protocol::wl_registry::{self, WlRegistry};
use smithay_client_toolkit::reexports::client::protocol::wl_seat::{self, WlSeat};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use smithay_client_toolkit::reexports::protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use smithay_client_toolkit::reexports::protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

/// The input of one seat that a token can name.
struct Seat {
    seat: WlSeat,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    /// The window with the keyboard focus, and the serial of its `enter` or of its latest key.
    key: Option<(WlSurface, u32)>,
    /// The window under the pointer.
    hover: Option<WlSurface>,
    /// The window of the latest button press, and its serial.
    press: Option<(WlSurface, u32)>,
}

#[derive(Default)]
struct Input {
    activation: Option<XdgActivationV1>,
    seats: Vec<Seat>,
}

impl Input {
    /// The seat, the serial and the window of an input event that lets a window of the app take the focus: the
    /// keyboard focus, else the latest button press on the window under the pointer.
    fn serial(&self) -> Option<(WlSeat, u32, WlSurface)> {
        let key = self.seats.iter().find_map(|s| s.key.clone().map(|(w, n)| (s.seat.clone(), n, w)));
        let press = || {
            self.seats.iter().find_map(|s| {
                let (w, n) = s.press.clone()?;
                (s.hover.as_ref() == Some(&w)).then(|| (s.seat.clone(), n, w))
            })
        };
        key.or_else(press)
    }

    fn seat(&mut self, seat: &WlSeat) -> Option<&mut Seat> {
        self.seats.iter_mut().find(|s| &s.seat == seat)
    }
}

/// The handlers of the input on the thread of the event queue.
struct State(Arc<Mutex<Input>>);

impl State {
    fn input(&self) -> std::sync::MutexGuard<'_, Input> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Activation {
    conn: Connection,
    qh: QueueHandle<State>,
    input: Arc<Mutex<Input>>,
}

static ACTIVATION: OnceLock<Option<Activation>> = OnceLock::new();

/// Starts to follow the input serials on the Wayland display `display` (a `wl_display` pointer), once. Call it as
/// soon as a window opens, so that a later activation finds the serial of the input that came since.
///
/// # Safety
///
/// `display` must be the `wl_display` of the app's windows, which stays open while the app runs.
pub unsafe fn track(display: NonNull<c_void>) {
    activation(display);
}

/// Raises the window of `surface` (a `wl_surface` pointer) on `display`, with `token` or else with a new token for the
/// latest input of the app.
///
/// # Safety
///
/// `display` must be the `wl_display` of the app's windows, which stays open while the app runs, and `surface` a live
/// `wl_surface` on it.
pub unsafe fn activate(display: NonNull<c_void>, surface: NonNull<c_void>, token: Option<String>) {
    let Some(a) = activation(display) else { return };
    let id = match unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) } {
        Ok(id) => id,
        Err(e) => return log::warn!("activate a window: not a wl_surface: {e}"),
    };
    let Ok(surface) = WlSurface::from_id(&a.conn, id) else { return };
    let input = a.input.lock().unwrap_or_else(|e| e.into_inner());
    let Some(activation) = input.activation.clone() else { return };
    match token {
        Some(token) => activation.activate(token, &surface),
        None => {
            let request = activation.get_activation_token(&a.qh, surface);
            if let Some((seat, serial, focus)) = input.serial() {
                request.set_serial(serial, &seat);
                request.set_surface(&focus);
            }
            request.commit();
        }
    }
    drop(input);
    if let Err(e) = a.conn.flush() {
        log::warn!("activate a window: {e}");
    }
}

fn activation(display: NonNull<c_void>) -> Option<&'static Activation> {
    ACTIVATION.get_or_init(|| start(display)).as_ref()
}

/// Binds the seats and `xdg_activation_v1` on a queue of its own, which a thread dispatches from then on.
fn start(display: NonNull<c_void>) -> Option<Activation> {
    let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let input = Arc::new(Mutex::new(Input::default()));
    let mut state = State(input.clone());
    let _registry = conn.display().get_registry(&qh, ());
    // The globals, then the capabilities of the seats.
    for _ in 0..2 {
        if let Err(e) = queue.roundtrip(&mut state) {
            log::warn!("follow the input for the window activation: {e}");
            return None;
        }
    }
    if state.input().activation.is_none() {
        log::info!("the compositor has no xdg_activation_v1, so windows cannot raise themselves");
        return None;
    }
    let thread = std::thread::Builder::new().name("wayland-activation".into());
    let spawned = thread.spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {});
    if let Err(e) = spawned {
        log::warn!("follow the input for the window activation: {e}");
        return None;
    }
    Some(Activation { conn, qh, input })
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        reg: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        let mut input = state.input();
        if interface == XdgActivationV1::interface().name && input.activation.is_none() {
            input.activation = Some(reg.bind(name, 1, qh, ()));
        } else if interface == WlSeat::interface().name {
            let seat = reg.bind(name, version.min(5), qh, ());
            input.seats.push(Seat { seat, keyboard: None, pointer: None, key: None, hover: None, press: None });
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event else { return };
        let mut input = state.input();
        let Some(s) = input.seat(seat) else { return };
        if caps.contains(wl_seat::Capability::Keyboard) {
            if s.keyboard.is_none() {
                s.keyboard = Some(seat.get_keyboard(qh, seat.clone()));
            }
        } else if let Some(keyboard) = s.keyboard.take() {
            s.key = None;
            if keyboard.version() >= 3 {
                keyboard.release();
            }
        }
        if caps.contains(wl_seat::Capability::Pointer) {
            if s.pointer.is_none() {
                s.pointer = Some(seat.get_pointer(qh, seat.clone()));
            }
        } else if let Some(pointer) = s.pointer.take() {
            (s.hover, s.press) = (None, None);
            if pointer.version() >= 3 {
                pointer.release();
            }
        }
    }
}

impl Dispatch<WlKeyboard, WlSeat> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        seat: &WlSeat,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut input = state.input();
        let Some(s) = input.seat(seat) else { return };
        match event {
            wl_keyboard::Event::Enter { serial, surface, .. } => s.key = Some((surface, serial)),
            wl_keyboard::Event::Leave { .. } => s.key = None,
            wl_keyboard::Event::Key { serial, .. } => {
                if let Some(key) = s.key.as_mut() {
                    key.1 = serial;
                }
            }
            // The keymap's file closes as it drops.
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, WlSeat> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        seat: &WlSeat,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut input = state.input();
        let Some(s) = input.seat(seat) else { return };
        match event {
            wl_pointer::Event::Enter { surface, .. } => s.hover = Some(surface),
            wl_pointer::Event::Leave { .. } => s.hover = None,
            wl_pointer::Event::Button { serial, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } => {
                s.press = s.hover.clone().map(|w| (w, serial));
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
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

/// A token request, with the window to activate once the token comes.
impl Dispatch<XdgActivationTokenV1, WlSurface> for State {
    fn event(
        state: &mut Self,
        request: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        surface: &WlSurface,
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_activation_token_v1::Event::Done { token } = event else { return };
        if let Some(activation) = state.input().activation.as_ref() {
            activation.activate(token, surface);
        }
        request.destroy();
        let _ = conn.flush();
    }
}

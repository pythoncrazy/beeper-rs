//! Desktop's chat themes: the bubble colors and the wallpapers of `chatAppearanceStore`.
use crate::style::hex;
use beeper_core::models::{Bubble, ChatAppearance, Wallpaper};
use iced::gradient::Linear;
use iced::widget::{container, image, stack, svg, Space};
use iced::{Background, Border, Color, Element, Fill, Gradient, Radians, Size, Theme};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Desktop's `mde`: the id and the color of each bubble color.
pub const BUBBLES: [(&str, u32); 9] = [
    ("blue", 0x3478f6),
    ("indigo", 0x4f5bd5),
    ("purple", 0x734ef3),
    ("pink", 0xc2185b),
    ("red", 0xd32f2f),
    ("orange", 0xe8710a),
    ("green", 0x2e7d32),
    ("teal", 0x00838f),
    ("slate", 0x37474f),
];

/// Desktop's `pde`: the id and the light color of each color wallpaper.
pub const COLORS: [(&str, u32); 12] = [
    ("mist-blue", 0xb8d8de),
    ("seafoam", 0xc7e1d2),
    ("sage", 0xb6d5c6),
    ("frost-teal", 0xaed8d7),
    ("lagoon", 0x6fced4),
    ("skywash", 0x77c3d7),
    ("mint", 0x88d1b0),
    ("meadow", 0x7dcb82),
    ("moss", 0xc5d6a6),
    ("sand", 0xd9d9ae),
    ("oat", 0xe1d9a3),
    ("butter", 0xebdd92),
];

/// Desktop's `fde`: the id, and the primary, secondary and accent colors of each gradient wallpaper.
pub const GRADIENTS: [(&str, [u32; 3]); 9] = [
    ("linen", [0xf2efe8, 0xe6ddcc, 0xd9ccb5]),
    ("petal", [0xe9b6eb, 0xce8fdd, 0xf8ddf4]),
    ("pearl", [0xdcc7ff, 0xbfa7ff, 0xf5ecff]),
    ("orchid", [0xdaa8d1, 0xc381d4, 0xf4daee]),
    ("bloom", [0xf7c7bf, 0xf39abd, 0xffe8d7]),
    ("haze", [0xe6daf7, 0xcdc3e9, 0xf6f0fc]),
    ("violet-night", [0xa79af7, 0x7566e3, 0x392e78]),
    ("apricot", [0xf7c270, 0xf1965b, 0xffd7a0]),
    ("amber", [0xf7d58d, 0xf2b55f, 0xfff0b8]),
];

/// Desktop's `swatchDefault`: a 2px diagonal from the bottom left to the top right of a 28px circle.
pub static NO_BUBBLE: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        &br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 28 28"><clipPath id="c"><circle cx="14" cy="14" r="13"/></clipPath><path d="M0 28L28 0" stroke="#000" stroke-width="2" clip-path="url(#c)"/></svg>"##[..],
    )
});

/// Desktop's `b4`: each channel times `t`, rounded.
fn scale(c: u32, t: f32) -> Color {
    let channel = |shift: u32| (((c >> shift) & 255) as f32 * t).round().min(255.0) / 255.0;
    Color::from_rgb(channel(16), channel(8), channel(0))
}

/// Desktop's `dO`: white, or near black when that has more contrast with `c`.
pub fn on(c: Color) -> Color {
    let lin = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let l = 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    if 1.05 / (l + 0.05) >= (l + 0.05) / 0.05 {
        Color::WHITE
    } else {
        hex(0x1a1a1a)
    }
}

/// A wallpaper: a flat color, or the primary, secondary and glow colors of a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    Flat(Color),
    Gradient([Color; 3]),
}

/// Desktop's `kot` and `Dot`: the paint of a wallpaper and the color of text on it.
pub fn paint(w: &Wallpaper, dark: bool) -> Option<(Paint, Color)> {
    match w {
        Wallpaper::None => None,
        Wallpaper::Color { id } => {
            let c = COLORS.iter().find(|c| c.0 == id)?.1;
            let c = scale(c, if dark { 0.52 } else { 1.0 });
            Some((Paint::Flat(c), on(c)))
        }
        Wallpaper::Gradient { id } => {
            let [a, b, glow] = GRADIENTS.iter().find(|g| g.0 == id)?.1;
            let [ta, tb, tg] = if dark { [0.34, 0.44, 0.58] } else { [1.0; 3] };
            let glow = scale(glow, tg).scale_alpha(if dark { 0x3d } else { 0x59 } as f32 / 255.0);
            let a = scale(a, ta);
            Some((Paint::Gradient([a, scale(b, tb), glow]), on(a)))
        }
    }
}

/// A color and the color of text on it.
pub type Pair = (Color, Color);

/// Desktop's `resolvedThemeFor`: the wallpaper with its text color, and the color and text color of sent bubbles.
pub fn resolve(a: &ChatAppearance, chat: Option<&str>, dark: bool) -> (Option<(Paint, Color)>, Option<Pair>) {
    let theme = a.theme(chat);
    let bubble = match theme.bubble {
        Some(Bubble::Swatch { id }) => BUBBLES.iter().find(|b| b.0 == id).map(|b| (hex(b.1), on(hex(b.1)))),
        Some(Bubble::Default) | None => None,
    };
    (theme.wallpaper.and_then(|w| paint(&w, dark)), bubble)
}

/// An image from `draw`, cached by `key`. Resizes make new keys, so the cache keeps at most 32 images.
fn cached(key: Vec<u32>, (w, h): (u32, u32), draw: impl Fn(f32, f32) -> [u8; 4]) -> image::Handle {
    static CACHE: LazyLock<Mutex<HashMap<Vec<u32>, image::Handle>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().expect("the wallpaper cache");
    if cache.len() >= 32 && !cache.contains_key(&key) {
        cache.clear();
    }
    let handle = cache.entry(key).or_insert_with(|| {
        let px = (0..h).flat_map(|y| (0..w).map(move |x| (x as f32 + 0.5, y as f32 + 0.5)));
        image::Handle::from_rgba(w, h, px.flat_map(|(x, y)| draw(x, y)).collect::<Vec<u8>>())
    });
    handle.clone()
}

/// The alpha of Desktop's glow at `x`, `y`: a 420px radial gradient centered at -120px, -180px.
fn glow_at(x: f32, y: f32) -> f32 {
    1.0 - ((x + 120.0).hypot(y + 180.0) / 420.0).min(1.0)
}

/// The glow of a gradient wallpaper over the at most 300 × 240 pixels at the top left of a `size` box with
/// corners of `radius`.
fn glow(c: Color, size: Size, radius: f32) -> image::Handle {
    let (width, height) = (size.width.min(1e6), size.height.min(1e6));
    let key = vec![c.r, c.g, c.b, c.a, radius, width, height].into_iter().map(f32::to_bits).collect();
    let [r, g, b, _] = c.into_rgba8();
    cached(key, ((width as u32).min(300), (height as u32).min(240)), |x, y| {
        let corner = |p: f32, size: f32| (radius - p).max(p - (size - radius)).max(0.0);
        let inside = (radius + 0.5 - corner(x, width).hypot(corner(y, height))).clamp(0.0, 1.0);
        [r, g, b, (c.a * glow_at(x, y) * inside * 255.0).round() as u8]
    })
}

/// The alpha of the 84px fade at the top of the timeline at `y`.
fn fade_at(y: f32) -> f32 {
    let t = y / FADE;
    if t < 0.75 {
        1.0 - 0.15 * t / 0.75
    } else {
        0.85 * (1.0 - (t - 0.75) / 0.25)
    }
}

/// The height of the fade at the top of the timeline.
pub const FADE: f32 = 84.0;

/// The fade at the top of the timeline over a `size` wallpaper. Desktop masks the messages instead.
pub fn fade<'a, M: 'a>(paint: Paint, size: Size) -> Element<'a, M> {
    let [a, b, g] = match paint {
        Paint::Gradient(colors) => colors,
        Paint::Flat(c) => {
            let fade = Linear::new(Radians(std::f32::consts::PI))
                .add_stop(0.0, c)
                .add_stop(0.75, c.scale_alpha(0.85))
                .add_stop(1.0, c.scale_alpha(0.0));
            let style = move |_: &Theme| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(fade))),
                ..Default::default()
            };
            return container(Space::new()).width(Fill).height(FADE).style(style).into();
        }
    };
    let key = [a, b, g].into_iter().flat_map(|c| [c.r, c.g, c.b, c.a]).chain([size.width, size.height]);
    let mix = |p: f32, q: f32, t: f32| p + (q - p) * t;
    let handle = cached(key.map(f32::to_bits).collect(), (size.width as u32, FADE as u32), |x, y| {
        let t = ((x + y) / (size.width + size.height)).min(1.0);
        let ga = g.a * glow_at(x, y);
        let px = [(a.r, b.r, g.r), (a.g, b.g, g.g), (a.b, b.b, g.b)].map(|(p, q, h)| mix(mix(p, q, t), h, ga));
        let [r, g, b] = px.map(|v| (v * 255.0).round() as u8);
        [r, g, b, (fade_at(y) * 255.0).round() as u8]
    });
    image(handle).into()
}

/// A box filled with a wallpaper. The glow stays inside the rounded corners of a `size` box.
pub fn fill<'a, M: 'a>(paint: Paint, size: Size, radius: f32) -> Element<'a, M> {
    let background = match paint {
        Paint::Flat(c) => Background::Color(c),
        Paint::Gradient([a, b, _]) => Background::Gradient(Gradient::Linear(
            Linear::new(Radians(135f32.to_radians())).add_stop(0.0, a).add_stop(1.0, b),
        )),
    };
    let base = container(Space::new()).width(Fill).height(Fill).style(move |_: &Theme| container::Style {
        background: Some(background),
        border: Border { radius: radius.into(), ..Default::default() },
        ..Default::default()
    });
    match paint {
        Paint::Flat(_) => base.into(),
        Paint::Gradient([_, _, g]) => {
            stack![base, container(image(glow(g, size, radius))).width(Fill).height(Fill).clip(true)].into()
        }
    }
}

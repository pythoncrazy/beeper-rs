//! Desktop's chat themes: the bubble colors and the wallpapers of `chatAppearanceStore`.
use crate::i18n::tr;
use crate::portal;
use crate::prelude::*;
use beeper_core::models::{Bubble, ChatAppearance, Wallpaper};

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

/// A wallpaper: a flat color, the primary, secondary and glow colors of a gradient, or a photo under a dimming
/// color.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Flat(Color),
    Gradient([Color; 3]),
    Photo(image::Handle, Color),
}

/// Desktop's `chat_wallpapers` directory of photo wallpapers.
pub fn photo_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("chat_wallpapers")
}

/// Desktop's `kot`, `Dot` and `wallpaperPhotoURL`: the paint of a wallpaper and the color of text on it. A photo
/// that is not on disk shows no wallpaper.
pub fn paint(w: &Wallpaper, dark: bool, data_dir: &Path) -> Option<(Paint, Color)> {
    match w {
        Wallpaper::None => None,
        Wallpaper::Photo { file_name } => {
            let path = photo_dir(data_dir).join(file_name);
            let dim = Color::from_rgba(0.0, 0.0, 0.0, if dark { 0.35 } else { 0.2 });
            path.exists().then(|| (Paint::Photo(image::Handle::from_path(path), dim), Color::WHITE))
        }
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
pub fn resolve(a: &ChatAppearance, chat: Option<&str>, dark: bool, data_dir: &Path) -> (Option<(Paint, Color)>, Option<Pair>) {
    let theme = a.theme(chat);
    let bubble = match theme.bubble {
        Some(Bubble::Swatch { id }) => BUBBLES.iter().find(|b| b.0 == id).map(|b| (hex(b.1), on(hex(b.1)))),
        Some(Bubble::Default) | None => None,
    };
    (theme.wallpaper.and_then(|w| paint(&w, dark, data_dir)), bubble)
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

/// The cover of the fade at the top of the timeline at `y`: Desktop's `fadeout-top` mask shows the messages from
/// nothing at the top to all at `FADE`.
fn fade_at(y: f32) -> f32 {
    1.0 - (y / FADE).min(1.0)
}

/// Desktop's `5.75rem` of `fadeout-top`.
pub const FADE: f32 = 92.0;

/// The fade at the top of the timeline over a `size` wallpaper, a photo too. Desktop masks the messages instead.
pub fn fade<'a, M: 'a>(paint: Paint, size: Size) -> Element<'a, M> {
    let [a, b, g] = match paint {
        Paint::Gradient(colors) => colors,
        Paint::Photo(photo, dim) => return photo_fade(&photo, dim, size).map_or(Space::new().into(), |h| image(h).into()),
        Paint::Flat(c) => {
            let fade = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, c).add_stop(1.0, c.scale_alpha(0.0));
            // The card clips its corners (`overflow: hidden`), so the fade has them too.
            let style = move |_: &Theme| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(fade))),
                border: iced::border::rounded(iced::border::top(crate::style::PANE_RADIUS)),
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
    image(handle).content_fit(ContentFit::None).into()
}

/// The top `FADE` pixels of a photo wallpaper as it covers a `size` box, under its `dim`, fading out downwards.
fn photo_fade(photo: &image::Handle, dim: Color, size: Size) -> Option<image::Handle> {
    /// A photo, the width and height of the box, and the alpha of the dim.
    type Key = (PathBuf, u32, u32, u32);
    static CACHE: LazyLock<Mutex<HashMap<Key, image::Handle>>> = LazyLock::new(Default::default);
    let image::Handle::Path(_, path) = photo else { return None };
    let (w, h) = (size.width.round() as u32, size.height.round() as u32);
    let key = (path.clone(), w, h, dim.a.to_bits());
    let mut cache = CACHE.lock().expect("the photo fade cache");
    if let Some(handle) = cache.get(&key) {
        return Some(handle.clone());
    }
    let bytes = std::fs::read(path).ok()?;
    let px = beeper_core::media::decode_upright(&bytes).ok()?.into_rgb8();
    let (iw, ih) = (px.width() as f32, px.height() as f32);
    // `ContentFit::Cover`: scaled to cover the box, centered.
    let scale = (size.width / iw).max(size.height / ih);
    let (ox, oy) = ((iw * scale - size.width) / 2.0, (ih * scale - size.height) / 2.0);
    let rows = (FADE as u32).min(h);
    let mut rgba = Vec::with_capacity((w * rows * 4) as usize);
    for y in 0..rows {
        for x in 0..w {
            let sx = (((x as f32 + 0.5 + ox) / scale) as u32).min(px.width() - 1);
            let sy = (((y as f32 + 0.5 + oy) / scale) as u32).min(px.height() - 1);
            let [r, g, b] = px.get_pixel(sx, sy).0;
            let under = |c: u8, d: f32| (f32::from(c) * (1.0 - dim.a) + d * 255.0 * dim.a).round() as u8;
            rgba.extend([under(r, dim.r), under(g, dim.g), under(b, dim.b)]);
            rgba.push((fade_at(y as f32 + 0.5) * 255.0).round() as u8);
        }
    }
    if cache.len() >= 8 {
        cache.clear();
    }
    let handle = image::Handle::from_rgba(w, rows, rgba);
    Some(cache.entry(key).or_insert(handle).clone())
}

/// A box filled with a wallpaper. The glow stays inside the rounded corners of a `size` box.
pub fn fill<'a, M: 'a>(paint: Paint, size: Size, radius: f32) -> Element<'a, M> {
    let background = match &paint {
        Paint::Photo(photo, dim) => {
            let photo = image(photo.clone()).content_fit(ContentFit::Cover).width(Fill).height(Fill);
            let dim = container(Space::new()).width(Fill).height(Fill).style(crate::style::fill_box(*dim, radius));
            return stack![photo.border_radius(radius), dim].into();
        }
        Paint::Flat(c) => Background::Color(*c),
        Paint::Gradient([a, b, _]) => {
            Background::Gradient(Gradient::Linear(Linear::new(Radians(135f32.to_radians())).add_stop(0.0, *a).add_stop(1.0, *b)))
        }
    };
    let base = container(Space::new()).width(Fill).height(Fill).style(move |_: &Theme| container::Style {
        background: Some(background),
        border: Border { radius: radius.into(), ..Default::default() },
        ..Default::default()
    });
    match paint {
        Paint::Flat(_) | Paint::Photo(..) => base.into(),
        Paint::Gradient([_, _, g]) => {
            stack![base, container(image(glow(g, size, radius)).content_fit(ContentFit::None)).width(Fill).height(Fill).clip(true)].into()
        }
    }
}

#[derive(Clone)]
pub enum Msg {
    /// Sets the wallpaper of the chat theme of a chat, or of the default theme.
    Wallpaper(Option<String>, Wallpaper),
    /// Sets the bubble color of the chat theme of a chat, or of the default theme; `None` clears it.
    Bubble(Option<String>, Option<Bubble>),
    /// Desktop's `Choose photo`: picks a photo wallpaper for a chat, or for the default theme.
    PickWallpaper(Option<String>),
    WallpaperPicked(Option<String>, Result<Vec<std::path::PathBuf>, String>),
    /// Desktop's `clearChatTheme`: the chat uses the default chat theme again.
    Clear(String),
    /// Desktop's `editChatTheme`: opens the chat theme window of a chat.
    Open(String),
}

impl BeeperGuiApp {
    pub(crate) fn chat_theme(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Wallpaper(chat, w) => {
                self.patch_settings(|s| s.chat_appearance.patch(chat.as_deref(), |t| t.wallpaper = Some(w)));
                self.prune_wallpapers();
            }
            Msg::Bubble(chat, b) => self.patch_settings(|s| s.chat_appearance.patch(chat.as_deref(), |t| t.bubble = b)),
            Msg::Clear(chat) => {
                self.patch_settings(|s| _ = s.chat_appearance.per_chat.remove(&chat));
                self.prune_wallpapers();
            }
            Msg::PickWallpaper(chat) => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Image);
                return Task::perform(pick, move |r| UiMessage::ChatTheme(Msg::WallpaperPicked(chat.clone(), r)));
            }
            Msg::WallpaperPicked(chat, r) => match r.map(|paths| paths.into_iter().next()) {
                Ok(Some(path)) => {
                    let dir = photo_dir(&self.store.read().config.data_dir);
                    let ext = path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy().to_lowercase()));
                    let file_name = format!("{}{ext}", beeper_core::connect::random_id());
                    std::fs::create_dir_all(&dir).expect("create the chat wallpapers directory");
                    match std::fs::copy(&path, dir.join(&file_name)) {
                        Ok(_) => return self.update(UiMessage::ChatTheme(Msg::Wallpaper(chat, Wallpaper::Photo { file_name }))),
                        Err(e) => self.fail(Toast::new(format!("Could not import {}: {e}", path.display()))),
                    }
                }
                Ok(None) => {}
                Err(e) => self.status_banner = e,
            },
            Msg::Open(chat) => {
                let open = |app: &mut Self| app.open_child(Size::new(380.0, 620.0), false);
                return self.open_once(|w| matches!(w, Win::Theme(c) if *c == chat), open, |_| Win::Theme(chat.clone()));
            }
        }
        self.settle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `fadeout-top` over a photo wallpaper: the photo as it covers the timeline, under its dim, fading out
    /// over `FADE` pixels.
    #[test]
    fn photo_wallpapers_fade_at_the_top_like_desktop() {
        let path = std::env::temp_dir().join(format!("beeper-rs-fade-{}.png", std::process::id()));
        ::image::RgbImage::from_pixel(4, 4, ::image::Rgb([255, 0, 0])).save(&path).expect("write a photo");
        let dim = Color::from_rgba(0.0, 0.0, 0.0, 0.2);
        let fade = photo_fade(&image::Handle::from_path(&path), dim, Size::new(30.0, 300.0));
        std::fs::remove_file(&path).expect("remove the photo");
        let Some(image::Handle::Rgba { width, height, pixels, .. }) = fade else { panic!("a faded strip") };
        assert_eq!((width, height), (30, FADE as u32));
        let at = |y: u32| &pixels[(y * width * 4) as usize..(y * width * 4 + 4) as usize];
        assert_eq!(at(0)[..3], [204, 0, 0], "the photo under its dim");
        assert!(at(0)[3] > 250 && at(height - 1)[3] < 5, "opaque at the top, gone at the bottom");
    }
}

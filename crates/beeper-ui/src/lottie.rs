//! Desktop's `Lottie` player: it loops a Lottie animation at its frame rate while the animation is on screen, from
//! its first frame when it shows. velato reads the animation, and vello_cpu, which hayro brings for PDFs, draws each
//! frame on a worker, at the size that it shows on the screen, so the window draws on while a frame takes tens of ms.
//!
//! The animated stickers of Telegram (TGS, gzipped Lottie) play the same way. Like Desktop's rlottie workers, a few
//! threads draw their frames as they are due, at the size that they show, and keep no other frames.
use hayro::vello_cpu::kurbo::{Affine, Rect, Shape};
use hayro::vello_cpu::peniko::{BlendMode, Brush, Fill, ImageAlphaType};
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Shell};
use iced::widget::image;
use iced::{window, Element, Event, Length, Rectangle, Renderer, Size, Theme};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use velato::model::{fixed, Content, ImageAsset};
use velato::{Composition, RenderSink};

/// The largest side of the frames of Desktop's animations in pixels: its 314 px wide illustrations on a 2x screen.
const PIXELS: u16 = 628;

/// The screen pixels of a layout pixel of the windows: the scale factor of the screen times the zoom. Until a window
/// says, a 2x screen.
static SCALE: AtomicU32 = AtomicU32::new(0x4000_0000);

/// Takes the screen pixels of a layout pixel, [`SCALE`].
pub fn set_scale(scale: f32) {
    if scale.is_finite() && scale > 0.0 {
        SCALE.store(scale.to_bits(), Ordering::Relaxed);
    }
}

fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// The longest side of the first frame that a sticker keeps, which shows until a worker draws the size that shows.
pub const STILL: u16 = 256;

/// The largest unzipped TGS: Telegram's stickers are at most 64 KiB zipped.
const MAX_JSON: u64 = 16 << 20;

/// A Lottie animation and the frame that was drawn last.
pub struct Animation {
    state: Mutex<State>,
    /// The first frame, the number of frames and the frames per second. Desktop's rlottie draws the frames from the
    /// in point to the out point, both included.
    first: f64,
    frames: u64,
    rate: f64,
    /// The width and the height of the composition.
    size: (f64, f64),
    /// The size of the JSON, for the budget of the media cache.
    json: usize,
    /// The frames that views drew last, by size.
    slots: Mutex<Vec<Slot>>,
}

/// The last frame that a worker drew at a size, if a worker draws another one, and when a view last showed it.
struct Slot {
    size: (u16, u16),
    drawn: Option<(u64, image::Handle)>,
    queued: bool,
    used: Instant,
}

struct State {
    comp: Composition,
    /// The subtract flags of the masks of each layer name.
    subtract: HashMap<String, Vec<bool>>,
    renderer: velato::Renderer,
    ctx: Option<((u16, u16), RenderContext)>,
    resources: Resources,
}

impl Animation {
    /// One of Desktop's own animations, whose masks are checked to draw right.
    pub fn new(json: &[u8]) -> Self {
        let v: serde_json::Value = serde_json::from_slice(json).expect("Lottie is JSON");
        for (name, masks) in masks(&v) {
            for m in masks {
                assert!(["a", "s"].contains(&m["mode"].as_str().expect("a mask mode")), "{name}: {m}");
                assert!(m["inv"] != true && m["o"]["k"] == 100, "{name}: a plain mask");
            }
        }
        Self::parse(json).expect("a Lottie animation of Desktop")
    }

    /// Any Lottie animation, like an animated sticker. Masks other than subtract ones add, and images are left out.
    pub fn parse(json: &[u8]) -> Result<Self, String> {
        let comp = Composition::from_slice(json).map_err(|e| format!("Could not read the Lottie animation: {e}"))?;
        let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let subtract = (masks(&v).into_iter()).map(|(name, masks)| (name, masks.iter().map(|m| m["mode"] == "s").collect())).collect();
        let (first, last, rate) = (comp.frames.start, comp.frames.end, comp.frame_rate);
        let size = (comp.width as f64, comp.height as f64);
        if !(last >= first && rate > 0.0 && size.0 >= 1.0 && size.1 >= 1.0) {
            return Err("The Lottie animation has no frames".into());
        }
        let frames = ((last - first) as u64 + 1).min(100_000);
        let state = State { comp, subtract, renderer: velato::Renderer::new(), ctx: None, resources: Resources::new() };
        let slots = Mutex::new(Vec::new());
        Ok(Animation { state: Mutex::new(state), first, frames, rate, size, json: json.len(), slots })
    }

    /// The frame `t` into the loop, and the time until the next one.
    pub fn frame(&self, t: Duration) -> (u64, Duration) {
        let n = (t.as_secs_f64() * self.rate) as u64;
        (n % self.frames, Duration::from_secs_f64((n + 1) as f64 / self.rate).saturating_sub(t))
    }

    /// The width and the height of the composition.
    pub fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }

    /// The pixel size of the frames that fit in `longest` pixels, with the aspect of the composition.
    pub fn fit(&self, longest: u16) -> (u16, u16) {
        let k = f64::from(longest.max(1)) / self.size.0.max(self.size.1);
        ((self.size.0 * k).round().max(1.0) as u16, (self.size.1 * k).round().max(1.0) as u16)
    }

    /// The image of frame `n` at `size`, drawn now.
    pub fn render(&self, n: u64, size: (u16, u16)) -> image::Handle {
        let mut st = self.state.lock().expect("no frame panicked");
        let scale = (f64::from(size.0) / self.size.0).min(f64::from(size.1) / self.size.1);
        st.render(self.first + (n % self.frames) as f64, size, scale)
    }

    /// The bytes that the animation takes, about: its model, and a frame at each size that shows.
    pub fn bytes(&self) -> usize {
        let slots = self.slots.lock().expect("no frame panicked");
        self.json * 4 + slots.iter().map(|s| usize::from(s.size.0) * usize::from(s.size.1) * 4).sum::<usize>()
    }

    /// The last frame that a worker drew at `size`, and its number.
    pub fn drawn(&self, size: (u16, u16)) -> Option<(u64, image::Handle)> {
        let slots = self.slots.lock().expect("no frame panicked");
        slots.iter().find(|s| s.size == size).and_then(|s| s.drawn.clone())
    }

    /// A view shows frame `n` at `size`: a worker draws it, unless it drew it or draws a frame at that size now.
    /// Whether the frame is drawn.
    pub fn want(self: &Arc<Self>, n: u64, size: (u16, u16)) -> bool {
        let now = Instant::now();
        let mut slots = self.slots.lock().expect("no frame panicked");
        slots.retain(|s| s.size == size || now.duration_since(s.used) < Duration::from_secs(10));
        let at = match slots.iter().position(|s| s.size == size) {
            Some(at) => at,
            None => {
                slots.push(Slot { size, drawn: None, queued: false, used: now });
                slots.len() - 1
            }
        };
        let slot = &mut slots[at];
        slot.used = now;
        if slot.drawn.as_ref().is_some_and(|d| d.0 == n) {
            return true;
        }
        if !slot.queued {
            slot.queued = true;
            // The workers live as long as the app.
            let _ = WORKERS.lock().expect("no frame panicked").send((self.clone(), n, size));
        }
        false
    }

    /// No worker draws a frame of the animation, and each size has one.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.slots.lock().expect("no frame panicked").iter().all(|s| !s.queued && s.drawn.is_some())
    }

    /// The frames that the workers drew last, at each size.
    #[cfg(test)]
    pub fn shown(&self) -> Vec<u64> {
        self.slots.lock().expect("no frame panicked").iter().filter_map(|s| Some(s.drawn.as_ref()?.0)).collect()
    }

    fn drew(&self, n: u64, size: (u16, u16), h: image::Handle) {
        let mut slots = self.slots.lock().expect("no frame panicked");
        if let Some(s) = slots.iter_mut().find(|s| s.size == size) {
            (s.drawn, s.queued) = (Some((n, h)), false);
        }
    }
}

/// The name and the masks of each layer that has masks, in the composition and its assets.
fn masks(v: &serde_json::Value) -> Vec<(String, &Vec<serde_json::Value>)> {
    let assets = v["assets"].as_array().into_iter().flatten().filter_map(|a| a["layers"].as_array());
    let layers = v["layers"].as_array().into_iter().chain(assets).flatten();
    layers.filter_map(|l| Some((l["nm"].as_str()?.to_string(), l["masksProperties"].as_array()?))).collect()
}

/// The threads that draw the frames of the animations that show, like Desktop's 4 rlottie workers at most.
type Job = (Arc<Animation>, u64, (u16, u16));
static WORKERS: LazyLock<Mutex<mpsc::Sender<Job>>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel::<Job>();
    let rx = Arc::new(Mutex::new(rx));
    let n = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
    for i in 0..n {
        let rx = rx.clone();
        let work = move || loop {
            let job = rx.lock().expect("no frame panicked").recv();
            let Ok((a, n, size)) = job else { return };
            let h = a.render(n, size);
            a.drew(n, size, h);
        };
        std::thread::Builder::new().name(format!("lottie-{i}")).spawn(work).expect("a Lottie thread");
    }
    Mutex::new(tx)
});

/// The Lottie animation in `bytes`, gzipped as a TGS sticker or not, or `None` when they are no Lottie JSON. Desktop
/// knows them by their type (`video/lottie+json`, `application/x-tgsticker` or `image/tgs`) or a `.tgs` name.
pub fn sniff(bytes: &[u8]) -> Option<Result<Animation, String>> {
    use std::io::Read;
    let json = match bytes.starts_with(&[0x1f, 0x8b]) {
        true => {
            let mut json = Vec::new();
            let unzip = flate2::read::GzDecoder::new(bytes).take(MAX_JSON).read_to_end(&mut json);
            if let Err(e) = unzip {
                return Some(Err(format!("Could not unzip the sticker: {e}")));
            }
            json
        }
        false if bytes.trim_ascii_start().starts_with(b"{") => bytes.to_vec(),
        false => return None,
    };
    // A Lottie animation has its version, frame rate, in and out points and layers.
    let v: serde_json::Value = serde_json::from_slice(&json).ok()?;
    let lottie = ["v", "fr", "ip", "op", "layers"].iter().all(|k| v.get(k).is_some());
    lottie.then(|| Animation::parse(&json))
}

impl State {
    fn render(&mut self, frame: f64, size: (u16, u16), scale: f64) -> image::Handle {
        let State { comp, subtract, renderer, ctx, resources, .. } = self;
        let ctx = match ctx {
            Some((s, ctx)) if *s == size => ctx,
            _ => &mut ctx.insert((size, RenderContext::new(size.0, size.1))).1,
        };
        // velato leaves out the time remap of a precomposition, which plays its frame at `start_frame` before.
        for layer in &mut comp.layers {
            if let Content::Instance { time_remap: Some(remap), .. } = &layer.content {
                layer.start_frame = frame - remap.evaluate(frame) * comp.frame_rate * layer.stretch;
            }
        }
        ctx.reset();
        renderer.append(comp, frame, Affine::scale(scale), 1.0, &mut Sink { ctx, subtract, masks: Vec::new() });
        ctx.flush();
        let mut pixmap = Pixmap::new(size.0, size.1);
        ctx.render(&mut pixmap, resources);
        image::Handle::from_rgba(size.0.into(), size.1.into(), pixmap.take_rgba8(ImageAlphaType::Alpha))
    }
}

/// velato's drawing on a vello_cpu context. velato clips with each mask of a layer, so a subtract mask clips with
/// all but its shape here.
struct Sink<'a> {
    ctx: &'a mut RenderContext,
    subtract: &'a HashMap<String, Vec<bool>>,
    /// The subtract flags of the masks of the layer that velato draws, last first.
    masks: Vec<bool>,
}

impl RenderSink for Sink<'_> {
    fn push_layer(&mut self, blend: impl Into<BlendMode>, alpha: f32, transform: Affine, shape: &impl Shape) {
        self.ctx.set_transform(transform);
        self.ctx.push_layer(Some(&shape.to_path(0.1)), Some(blend.into()), Some(alpha), None, None);
    }

    fn push_clip_layer(&mut self, transform: Affine, shape: &impl Shape) {
        self.ctx.set_transform(transform);
        let path = match self.masks.pop() {
            Some(true) => {
                let mut all = Rect::new(-1e6, -1e6, 1e6, 1e6).to_path(0.1);
                all.extend(shape.path_elements(0.1));
                self.ctx.set_fill_rule(Fill::EvenOdd);
                all
            }
            _ => shape.to_path(0.1),
        };
        self.ctx.push_clip_layer(&path);
        self.ctx.set_fill_rule(Fill::NonZero);
    }

    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }

    fn draw(&mut self, stroke: Option<&fixed::Stroke>, transform: Affine, brush: &fixed::Brush, shape: &impl Shape) {
        self.ctx.set_transform(transform);
        match brush {
            Brush::Solid(c) => self.ctx.set_paint(*c),
            Brush::Gradient(g) => self.ctx.set_paint(g.clone()),
            // Telegram's stickers have no images.
            Brush::Image(_) => return,
        }
        let path = shape.to_path(0.1);
        match stroke {
            Some(s) => {
                self.ctx.set_stroke(s.clone());
                self.ctx.stroke_path(&path);
            }
            None => self.ctx.fill_path(&path),
        }
    }

    fn draw_image(&mut self, _: &ImageAsset, _: Affine, _: f64) {}

    fn begin_layer_group(&mut self, name: &str, _: usize) {
        self.masks = self.subtract.get(name).into_iter().flatten().rev().copied().collect();
    }
}

/// `animation`, `height` high and fit in the width, looping from its first frame when it shows.
pub fn lottie<'a, M: 'a>(animation: &'a Arc<Animation>, height: impl Into<Length>) -> Element<'a, M> {
    Element::new(Lottie { animation, height: height.into() })
}

struct Lottie<'a> {
    animation: &'a Arc<Animation>,
    height: Length,
}

impl Lottie<'_> {
    /// The pixel size of the frames in `bounds` at `scale` screen pixels a layout pixel ([`SCALE`]): the screen pixels
    /// that the animation fits in, in steps of 16 px, and at most [`PIXELS`].
    fn pixels(&self, bounds: Rectangle, scale: f32) -> (u16, u16) {
        let (w, h, scale) = (self.animation.size.0, self.animation.size.1, f64::from(scale));
        let k = (f64::from(bounds.width) / w).min(f64::from(bounds.height) / h);
        let longest = (w.max(h) * k * scale / 16.0).ceil() * 16.0;
        self.animation.fit(longest.clamp(16.0, f64::from(PIXELS)) as u16)
    }
}

impl<M> Widget<M, Theme, Renderer> for Lottie<'_> {
    /// When the animation showed.
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Instant>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Instant::now())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, self.height)
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, Length::Fill, self.height)
    }

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, _: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return };
        if layout.bounds().intersects(viewport) {
            // A worker draws the frame that is due, which shows from the next redraw on.
            let start = *tree.state.downcast_ref::<Instant>();
            let (n, left) = self.animation.frame(now.duration_since(start));
            self.animation.want(n, self.pixels(layout.bounds(), scale()));
            shell.request_redraw_at(*now + left);
        }
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        // Nothing shows until a worker drew the first frame, like an animation that loads.
        let Some((_, frame)) = self.animation.drawn(self.pixels(layout.bounds(), scale())) else { return };
        let image = image(frame).width(Length::Fill).height(self.height);
        Widget::<M, Theme, Renderer>::draw(&image, tree, renderer, theme, style, layout, cursor, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::{Animation, PIXELS};
    use std::time::Duration;

    /// The pixel at `x`, `y` of frame `n`.
    fn pixel(a: &Animation, n: u64, x: u32, y: u32) -> [u8; 4] {
        let iced::advanced::image::Handle::Rgba { pixels, .. } = a.render(n, (PIXELS, PIXELS)) else { panic!("RGBA") };
        let at = ((y * u32::from(PIXELS) + x) * 4) as usize;
        pixels[at..at + 4].try_into().expect("four bytes")
    }

    /// Frames of Desktop's animations that need a subtract mask and a time remap, whose pixels Desktop's rlottie draws
    /// with these colors.
    #[test]
    fn animations_draw_like_desktop() {
        let key = Animation::new(include_bytes!("../assets/onboarding/key.json"));
        assert_eq!((key.first, key.frames, key.rate), (24.0, 74, 24.0), "rlottie plays the in to the out point");
        assert_eq!(pixel(&key, 0, 180, 150), [255, 122, 25, 255], "the orange key shows outside its subtract mask");
        let lock = Animation::new(include_bytes!("../assets/onboarding/lock.json"));
        assert_eq!(pixel(&lock, 25, 310, 286), [255, 255, 255, 255], "the remapped time keeps the logo in front");
        // rlottie draws the out point too, so the loop has no empty frame.
        let email = Animation::new(include_bytes!("../assets/onboarding/email.json"));
        for a in [&key, &lock, &email] {
            let last = a.render(a.frames - 1, (PIXELS, PIXELS));
            let iced::advanced::image::Handle::Rgba { pixels, .. } = last else { panic!("RGBA") };
            assert!(pixels.chunks(4).filter(|p| p[3] > 0).count() > 10_000, "the last frame is drawn");
        }
        let (n, next) = lock.frame(Duration::from_secs(1));
        assert!(n == 24 && (next.as_secs_f64() - 1.0 / 24.0).abs() < 1e-6, "24 frames a second: {n} {next:?}");
        assert_eq!(lock.frame(Duration::from_secs_f64(50.5 / 24.0)).0, 0, "the animation loops after its 50 frames");
    }

    /// Desktop's `Lottie` takes a gzipped TGS (`isTgs`) and Lottie JSON; other images are no Lottie, and a broken
    /// TGS fails like an image that does not decode.
    #[test]
    fn stickers_sniff_like_desktop() {
        use std::io::Write;
        let json = include_bytes!("../assets/onboarding/key.json");
        let mut tgs = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        tgs.write_all(json).unwrap();
        let tgs = tgs.finish().unwrap();
        let a = super::sniff(&tgs).expect("a TGS").expect("it reads");
        assert_eq!((a.frames, a.size()), (74, (1920, 1920)), "the frames and the size of the composition");
        assert_eq!(a.fit(256), (256, 256));
        let h = a.render(0, (128, 128));
        let iced::advanced::image::Handle::Rgba { width, height, .. } = h else { panic!("RGBA") };
        assert_eq!((width, height), (128, 128), "a frame at the size that shows");
        assert!(super::sniff(json).is_some_and(|a| a.is_ok()), "Lottie JSON");
        assert!(super::sniff(b"GIF89a").is_none() && super::sniff(br#"{"a":1}"#).is_none(), "no Lottie");
        assert!(super::sniff(&tgs[..tgs.len() / 2]).is_some_and(|a| a.is_err()), "a cut TGS");
    }

    /// Desktop's illustrations draw at the size that they fit in on the screen, at most 628 px, so a smaller box or
    /// screen costs less.
    #[test]
    fn illustrations_draw_at_their_size_on_the_screen() {
        let a = std::sync::Arc::new(Animation::new(include_bytes!("../assets/onboarding/key.json")));
        let at = |w, h, scale| {
            let bounds = iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(w, h));
            super::Lottie { animation: &a, height: iced::Length::Fixed(h) }.pixels(bounds, scale)
        };
        assert_eq!(at(330.0, 240.0, 2.0), (480, 480), "the key of the recovery key screen");
        assert_eq!(at(330.0, 314.0, 2.0), (628, 628), "the 314 px illustrations");
        assert_eq!(at(800.0, 700.0, 2.0), (628, 628), "at most 628 px");
        assert_eq!(at(330.0, 314.0, 1.0), (320, 320), "on a 1x screen, in steps of 16 px");
    }

    /// Like Desktop's rlottie workers, a worker draws the frame that a view wants at its size, once.
    #[test]
    fn workers_draw_the_frames_that_show() {
        let a = std::sync::Arc::new(Animation::new(include_bytes!("../assets/onboarding/key.json")));
        // The worker may draw the frame at once, so only the ask is checked here.
        assert!(!a.want(3, (64, 64)), "a worker draws it");
        let t = std::time::Instant::now();
        while a.drawn((64, 64)).is_none() {
            assert!(t.elapsed() < Duration::from_secs(10), "the worker draws the frame");
            std::thread::yield_now();
        }
        assert_eq!(a.drawn((64, 64)).unwrap().0, 3);
        assert!(a.want(3, (64, 64)), "a drawn frame draws once");
        assert!(a.drawn((32, 32)).is_none(), "each size has its frame");
    }
}

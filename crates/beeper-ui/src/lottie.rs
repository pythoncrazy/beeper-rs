//! Desktop's `Lottie` player: it loops a Lottie animation at its frame rate while the animation is on screen, from
//! its first frame when it shows. velato reads the animation, and vello_cpu, which hayro brings for PDFs, draws each
//! frame.
use hayro::vello_cpu::kurbo::{Affine, Rect, Shape};
use hayro::vello_cpu::peniko::{BlendMode, Brush, Fill, ImageAlphaType};
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Shell};
use iced::widget::image;
use iced::{window, Element, Event, Length, Rectangle, Renderer, Size, Theme};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use velato::model::{fixed, Content, ImageAsset};
use velato::{Composition, RenderSink};

/// The width and height of the frames in pixels: Desktop's 314 px wide illustrations on a 2x screen.
const PIXELS: u16 = 628;

/// A Lottie animation and the frame that was drawn last.
pub struct Animation {
    state: Mutex<State>,
    /// The first frame, the number of frames and the frames per second. Desktop's rlottie draws the frames from the
    /// in point to the out point, both included.
    first: f64,
    frames: u64,
    rate: f64,
}

struct State {
    comp: Composition,
    /// The subtract flags of the masks of each layer name.
    subtract: HashMap<String, Vec<bool>>,
    renderer: velato::Renderer,
    ctx: RenderContext,
    resources: Resources,
    last: Option<(u64, image::Handle)>,
}

impl Animation {
    pub fn new(json: &[u8]) -> Self {
        let comp = Composition::from_slice(json).expect("a Lottie animation of Desktop");
        let v: serde_json::Value = serde_json::from_slice(json).expect("Lottie is JSON");
        let assets = v["assets"].as_array().into_iter().flatten().filter_map(|a| a["layers"].as_array());
        let layers = v["layers"].as_array().into_iter().chain(assets).flatten();
        let subtract = layers
            .filter_map(|l| Some((l["nm"].as_str()?.to_string(), l["masksProperties"].as_array()?)))
            .map(|(name, masks)| {
                for m in masks {
                    assert!(["a", "s"].contains(&m["mode"].as_str().expect("a mask mode")), "{name}: {m}");
                    assert!(m["inv"] != true && m["o"]["k"] == 100, "{name}: a plain mask");
                }
                (name, masks.iter().map(|m| m["mode"] == "s").collect())
            })
            .collect();
        let (first, frames, rate) =
            (comp.frames.start, (comp.frames.end - comp.frames.start) as u64 + 1, comp.frame_rate);
        let ctx = RenderContext::new(PIXELS, PIXELS);
        let state =
            State { comp, subtract, renderer: velato::Renderer::new(), ctx, resources: Resources::new(), last: None };
        Animation { state: Mutex::new(state), first, frames, rate }
    }

    /// The frame `t` into the loop, and the time until the next one.
    fn frame(&self, t: Duration) -> (u64, Duration) {
        let n = (t.as_secs_f64() * self.rate) as u64;
        (n % self.frames, Duration::from_secs_f64((n + 1) as f64 / self.rate) - t)
    }

    /// The image of frame `n`.
    pub fn image(&self, n: u64) -> image::Handle {
        let mut st = self.state.lock().expect("no frame panicked");
        if let Some((_, h)) = st.last.as_ref().filter(|(at, _)| *at == n) {
            return h.clone();
        }
        let frame = self.first + n as f64;
        let State { comp, subtract, renderer, ctx, resources, .. } = &mut *st;
        // velato leaves out the time remap of a precomposition, which plays its frame at `start_frame` before.
        for layer in &mut comp.layers {
            if let Content::Instance { time_remap: Some(remap), .. } = &layer.content {
                assert_eq!(layer.stretch, 1.0, "{} has no time stretch", layer.name);
                layer.start_frame = frame - remap.evaluate(frame) * comp.frame_rate;
            }
        }
        ctx.reset();
        let scale = f64::from(PIXELS) / comp.width as f64;
        renderer.append(comp, frame, Affine::scale(scale), 1.0, &mut Sink { ctx, subtract, masks: Vec::new() });
        ctx.flush();
        let mut pixmap = Pixmap::new(PIXELS, PIXELS);
        ctx.render(&mut pixmap, resources);
        let h = image::Handle::from_rgba(PIXELS.into(), PIXELS.into(), pixmap.take_rgba8(ImageAlphaType::Alpha));
        st.last = Some((n, h.clone()));
        h
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
            Brush::Image(_) => unreachable!("Desktop's animations have no images"),
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

    fn draw_image(&mut self, _: &ImageAsset, _: Affine, _: f64) {
        unreachable!("Desktop's animations have no images")
    }

    fn begin_layer_group(&mut self, name: &str, _: usize) {
        self.masks = self.subtract.get(name).into_iter().flatten().rev().copied().collect();
    }
}

/// `animation`, `height` high and fit in the width, looping from its first frame when it shows.
pub fn lottie<'a, M: 'a>(animation: &'a Animation, height: impl Into<Length>) -> Element<'a, M> {
    Element::new(Lottie { animation, height: height.into() })
}

struct Lottie<'a> {
    animation: &'a Animation,
    height: Length,
}

impl Lottie<'_> {
    fn image(&self, tree: &widget::Tree) -> image::Image {
        let start = *tree.state.downcast_ref::<Instant>();
        image(self.animation.image(self.animation.frame(start.elapsed()).0)).width(Length::Fill).height(self.height)
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

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        Widget::<M, Theme, Renderer>::layout(&mut self.image(tree), tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return };
        if layout.bounds().intersects(viewport) {
            let start = *tree.state.downcast_ref::<Instant>();
            shell.request_redraw_at(*now + self.animation.frame(now.duration_since(start)).1);
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        Widget::<M, Theme, Renderer>::draw(&self.image(tree), tree, renderer, theme, style, layout, cursor, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::{Animation, PIXELS};
    use std::time::Duration;

    /// The pixel at `x`, `y` of frame `n`.
    fn pixel(a: &Animation, n: u64, x: u32, y: u32) -> [u8; 4] {
        let iced::advanced::image::Handle::Rgba { pixels, .. } = a.image(n) else { panic!("an RGBA frame") };
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
            let iced::advanced::image::Handle::Rgba { pixels, .. } = a.image(a.frames - 1) else { panic!("RGBA") };
            assert!(pixels.chunks(4).filter(|p| p[3] > 0).count() > 10_000, "the last frame is drawn");
        }
        let (n, next) = lock.frame(Duration::from_secs(1));
        assert!(n == 24 && (next.as_secs_f64() - 1.0 / 24.0).abs() < 1e-6, "24 frames a second: {n} {next:?}");
        assert_eq!(lock.frame(Duration::from_secs_f64(50.5 / 24.0)).0, 0, "the animation loops after its 50 frames");
    }
}

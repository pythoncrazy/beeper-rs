use crate::lottie;
use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget::{self};
use iced::advanced::{mouse, renderer, Renderer as _};
use iced::widget::Image;
use iced::window;

/// A frame of an animation and its delay in milliseconds.
pub type Frame = (image::Handle, u128);

/// Draws the frame of `frames` that is due since `start`, and asks for a redraw at the next frame while it is
/// visible. The view does not change, so the animation needs no view or layout. An animation that does not play
/// shows its first frame and asks for no redraws.
pub struct Gif<'a> {
    frames: Frames,
    play: bool,
    start: Instant,
    make: Box<dyn Fn(image::Handle) -> Image + 'a>,
    /// The turn in radians from the first angle to the second one in `TURN_MS` from the time.
    turn: (f32, f32, Instant),
}

#[cfg(test)]
thread_local!(static FROZEN: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) });

/// The time that the animations of this thread draw at: the time of the redraw `at`, or the time that a test froze.
fn at(at: Instant) -> Instant {
    #[cfg(test)]
    if let Some(t) = FROZEN.get() {
        return t;
    }
    at
}

/// How long ago `start` was for the animations of this thread.
pub fn since(start: Instant) -> Duration {
    at(Instant::now()).saturating_duration_since(start)
}

/// Stops the clock of the animations of this thread at `t` until the guard drops, so that a test draws the frame
/// of a time, however long the draw takes.
#[cfg(test)]
pub fn freeze(t: Instant) -> Frozen {
    FROZEN.set(Some(t));
    Frozen
}

/// The clock of [`freeze`], which goes on when it drops.
#[cfg(test)]
pub struct Frozen;

#[cfg(test)]
impl Drop for Frozen {
    fn drop(&mut self) {
        FROZEN.set(None);
    }
}

/// The time of Desktop's `.2s transform` transition.
const TURN_MS: f32 = 200.0;

/// How often a Lottie animation that does not play looks for its frame while a worker draws it.
const POLL: Duration = Duration::from_millis(16);

/// The frames of an image, shared with the cache that decoded them: a still image has one frame. A Lottie
/// animation, like an animated sticker, has its first frame, and workers draw its frames when they are due.
#[derive(Clone, Default)]
pub struct Frames(Arc<[Frame]>, Option<Arc<lottie::Animation>>);

impl Frames {
    /// The Lottie animation `a`, whose first frame is `still`.
    pub fn lottie(still: image::Handle, a: lottie::Animation) -> Self {
        Frames(Arc::new([(still, 0)]), Some(Arc::new(a)))
    }

    /// The Lottie animation of the frames.
    pub fn animation(&self) -> Option<&Arc<lottie::Animation>> {
        self.1.as_ref()
    }

    /// Whether the image moves: it has more than one frame, or it is a Lottie animation.
    pub fn animated(&self) -> bool {
        self.0.len() > 1 || self.1.is_some()
    }
}

impl std::ops::Deref for Frames {
    type Target = [Frame];

    fn deref(&self) -> &[Frame] {
        &self.0
    }
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Frames").field(&self.0.len()).field(&self.1.is_some()).finish()
    }
}

impl<const N: usize> From<[Frame; N]> for Frames {
    fn from(frames: [Frame; N]) -> Self {
        Frames(Arc::new(frames), None)
    }
}

impl From<Vec<Frame>> for Frames {
    fn from(frames: Vec<Frame>) -> Self {
        Frames(frames.into(), None)
    }
}

impl FromIterator<Frame> for Frames {
    fn from_iter<I: IntoIterator<Item = Frame>>(frames: I) -> Self {
        Frames(frames.into_iter().collect(), None)
    }
}

/// `frames` as `make` draws them, playing from `start` when `play`. A still image, or an animation that does not
/// play, is the plain image of its first frame, so it adds no work to the frames of the window. A Lottie animation
/// draws at the size that it shows, so it stays an animation that does not play.
pub fn gif<'a, M: 'a>(frames: &Frames, play: bool, start: Instant, make: impl Fn(image::Handle) -> Image + 'a) -> Element<'a, M> {
    match plays(frames, play) || frames.animation().is_some() {
        false => make(frames[0].0.clone()).into(),
        true => {
            let turn = (0.0, 0.0, start);
            Element::new(Gif { frames: frames.clone(), play, start, make: Box::new(make), turn })
        }
    }
}

/// `gif`, turned from the first angle to the second one in radians, from the time on, with the CSS `ease` curve.
/// The turned image fits in its bounds.
pub fn turning<'a, M: 'a>(
    frames: &Frames, play: bool, start: Instant, turn: (f32, f32, Instant), make: impl Fn(image::Handle) -> Image + 'a,
) -> Element<'a, M> {
    Element::new(Gif { frames: frames.clone(), play, start, make: Box::new(make), turn })
}

/// Whether an animation of `frames` moves when it is to `play`.
pub fn plays(frames: &Frames, play: bool) -> bool {
    play && frames.animated()
}

/// The time of the frame after the one that is due at `now`, of an animation that plays from `start`.
pub fn next(frames: &[Frame], start: Instant, now: Instant) -> Instant {
    let (_, left) = frame(frames, now.saturating_duration_since(start).as_millis());
    now + Duration::from_millis(left as u64)
}

/// The pixel size that a Lottie animation draws at in `bounds`: twice their size for a 2x screen, in steps of 16 px,
/// and at most 512 px, the size of a Telegram sticker.
pub fn lottie_size(a: &lottie::Animation, bounds: Rectangle) -> (u16, u16) {
    let longest = (bounds.width.max(bounds.height) * 2.0 / 16.0).ceil() * 16.0;
    a.fit(longest.clamp(16.0, 512.0) as u16)
}

impl Gif<'_> {
    /// The image of frame `h` at the angle of the turn now, and if the turn goes on.
    fn image(&self, h: image::Handle) -> (Image, bool) {
        let (from, to, at) = self.turn;
        let t = (since(at).as_secs_f32() * 1000.0 / TURN_MS).min(1.0);
        let angle = from + (to - from) * crate::cubic_bezier(t, [0.25, 0.1, 0.25, 1.0]);
        ((self.make)(h).rotation(iced::Rotation::Solid(iced::Radians(angle))), t < 1.0 && from != to)
    }
}

/// The frame at `ms` into the loop, and the time until the next frame.
pub fn frame(frames: &[Frame], ms: u128) -> (&image::Handle, u128) {
    let mut t = ms % frames.iter().map(|f| f.1).sum::<u128>();
    frames
        .iter()
        .find_map(|(h, d)| match t.checked_sub(*d) {
            Some(rest) => {
                t = rest;
                None
            }
            None => Some((h, d - t)),
        })
        .expect("the time is below the total delay")
}

impl<M> Widget<M, Theme, Renderer> for Gif<'_> {
    fn size(&self) -> Size<Length> {
        Widget::<M, Theme, Renderer>::size(&(self.make)(self.frames[0].0.clone()))
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        Widget::<M, Theme, Renderer>::layout(&mut (self.make)(self.frames[0].0.clone()), tree, renderer, limits)
    }

    fn update(
        &mut self, _: &mut widget::Tree, event: &Event, layout: Layout<'_>, _: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return };
        let now = at(*now);
        if !layout.bounds().intersects(viewport) {
            return;
        }
        if self.image(self.frames[0].0.clone()).1 {
            shell.request_redraw();
        }
        if let Some(a) = self.frames.animation() {
            // A worker draws the frame that is due, which shows from the next redraw on. Like Desktop's paused
            // Lottie, an animation that stops keeps the frame that it shows.
            let size = lottie_size(a, layout.bounds());
            match self.play {
                true => {
                    let (n, left) = a.frame(now.saturating_duration_since(self.start));
                    a.want(n, size);
                    shell.request_redraw_at(now + left);
                }
                false if a.drawn(size).is_none() && !a.want(0, size) => shell.request_redraw_at(now + POLL),
                false => {}
            }
        } else if plays(&self.frames, self.play) {
            shell.request_redraw_at(next(&self.frames, self.start, now));
        }
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        // A still image has one frame without a delay. The first frame of a Lottie animation shows until a worker
        // draws its frames at the size that it shows.
        let first = || self.frames[0].0.clone();
        let h = match (self.frames.animation(), plays(&self.frames, self.play)) {
            (Some(a), _) => a.drawn(lottie_size(a, layout.bounds())).map_or_else(first, |d| d.1),
            (None, false) => first(),
            (None, true) => frame(&self.frames, since(self.start).as_millis()).0.clone(),
        };
        let (image, _) = self.image(h);
        Widget::<M, Theme, Renderer>::draw(&image, tree, renderer, theme, style, layout, cursor, viewport);
    }
}

/// A round dot of `color` whose alpha is `alpha(t)`, with `t` the part of a 1 s loop that starts `delay` ms
/// after `start`. It redraws each frame while it is visible, without view or layout.
pub struct Dot {
    size: f32,
    color: Color,
    start: Instant,
    delay: u128,
    alpha: fn(f32) -> f32,
}

pub fn dot<'a, M: 'a>(size: f32, color: Color, start: Instant, delay: u128, alpha: fn(f32) -> f32) -> Element<'a, M> {
    Element::new(Dot { size, color, start, delay, alpha })
}

impl<M> Widget<M, Theme, Renderer> for Dot {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, _: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }

    fn update(
        &mut self, _: &mut widget::Tree, event: &Event, layout: Layout<'_>, _: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) && layout.bounds().intersects(viewport) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self, _: &widget::Tree, renderer: &mut Renderer, _: &Theme, _: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let t = ((since(self.start).as_millis() + 1000 - self.delay) % 1000) as f32 / 1000.0;
        let quad = renderer::Quad { bounds: layout.bounds(), border: iced::border::rounded(self.size / 2.0), ..Default::default() };
        renderer.fill_quad(quad, self.color.scale_alpha((self.alpha)(t)));
    }
}

/// The `--popover-window-fade` of Desktop's command bar, in seconds.
const DIMMER_FADE: f32 = 0.04;

/// Desktop's `.command-bar-dimmer`: the `.modal-dimmer` #0003 under `content`, which fades in with the 40 ms
/// `command-bar-dimmer-in` animation, `ease-out`, from the first frame that shows it.
pub struct Dimmer<'a, M> {
    content: Element<'a, M>,
}

pub fn dimmer<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    Element::new(Dimmer { content: content.into() })
}

/// The times of the first frame of the dimmer and of the frame to draw.
#[derive(Default)]
struct Shown(Option<(Instant, Instant)>);

impl Shown {
    /// How far the dimmer has faded in at the frame, from 0 to 1.
    fn faded(&self) -> f32 {
        let t = self.0.map_or(0.0, |(first, now)| (now.saturating_duration_since(first).as_secs_f32() / DIMMER_FADE).min(1.0));
        crate::cubic_bezier(t, [0.0, 0.0, 0.58, 1.0])
    }
}

impl<M> Widget<M, Theme, Renderer> for Dimmer<'_, M> {
    crate::delegate::delegate!(state(Shown), children, diff, size, layout, operate, mouse_interaction, overlay(M));

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer,
        clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let shown = tree.state.downcast_mut::<Shown>();
            shown.0 = Some((shown.0.map_or(*now, |s| s.0), *now));
            if shown.faded() < 1.0 {
                shell.request_redraw();
            }
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        let k = tree.state.downcast_ref::<Shown>().faded();
        renderer.fill_quad(renderer::Quad { bounds: layout.bounds(), ..Default::default() }, Color::BLACK.scale_alpha(0.2 * k));
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }
}

/// The length of a loop of Desktop's `--skeleton-bg` shimmer (`slide-across 4s ease-in-out infinite`), in seconds.
pub const SHIMMER: f32 = 4.0;

/// The alpha of the shimmer of Desktop's `--skeleton-bg` over its fill, as stops across the box from the left at `t`
/// seconds: the fill is `color`, and its four 400% wide `#flashes` rects of the `gradient` (from clear to `color` at
/// 0.1 at 15%, `color` from 40% to 60%, and back) at 0.5 opacity slide from `-1350%` to `-150%`, so a soft band
/// sweeps across the box three times a loop and rests on it at each end.
pub fn shimmer(t: f32) -> Vec<(f32, f32)> {
    const STOPS: [(f32, f32); 6] = [(0.0, 0.0), (0.15, 0.1), (0.4, 1.0), (0.6, 1.0), (0.85, 0.1), (1.0, 0.0)];
    let e = crate::cubic_bezier((t / SHIMMER).rem_euclid(1.0), [0.42, 0.0, 0.58, 1.0]);
    // The left edge of the box in the units of the box width, from the left edge of the first rect.
    let from = 13.5 - 12.0 * e;
    let flash = |p: f32| {
        let q = ((from + p) / 4.0).rem_euclid(1.0);
        let i = STOPS.windows(2).position(|s| q <= s[1].0).unwrap_or(STOPS.len() - 2);
        let ((a, x), (b, y)) = (STOPS[i], STOPS[i + 1]);
        0.5 * (x + (y - x) * (q - a) / (b - a))
    };
    let mut at: Vec<f32> = vec![0.0, 1.0];
    for k in 0..5 {
        for (s, _) in STOPS {
            let p = 4.0 * (k as f32 + s) - from;
            if p > 0.0 && p < 1.0 {
                at.push(p);
            }
        }
    }
    at.sort_by(f32::total_cmp);
    at.dedup();
    at.into_iter().map(|p| (p, flash(p))).collect()
}

/// Desktop's `img.loading` of a media that loads: its box of `--skeleton-bg`, `color` with the shimmer of
/// [`shimmer`] that loops from `start`, with corners of `radius`. It redraws each frame while it is visible, without
/// view or layout.
pub struct Skeleton {
    width: Length,
    height: Length,
    radius: f32,
    color: Color,
    start: Instant,
}

pub fn skeleton<'a, M: 'a>(
    width: impl Into<Length>, height: impl Into<Length>, radius: f32, color: Color, start: Instant,
) -> Element<'a, M> {
    Element::new(Skeleton { width: width.into(), height: height.into(), radius, color, start })
}

impl Skeleton {
    /// The gradient of the box at `t` seconds: the shimmer over the fill, in the color of both.
    pub fn gradient(color: Color, t: f32) -> iced::gradient::Linear {
        let c = color.a;
        shimmer(t).into_iter().fold(iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2)), |g, (at, flash)| {
            g.add_stop(at, Color { a: c + c * flash * (1.0 - c), ..color })
        })
    }
}

/// The state that a [`Skeleton`] shows to the operations, so that tests find it.
pub struct Loading;

impl<M> Widget<M, Theme, Renderer> for Skeleton {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn operate(&mut self, _: &mut widget::Tree, layout: Layout<'_>, _: &Renderer, op: &mut dyn widget::Operation) {
        op.custom(None, layout.bounds(), &mut Loading);
    }

    fn update(
        &mut self, _: &mut widget::Tree, event: &Event, layout: Layout<'_>, _: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>, viewport: &Rectangle,
    ) {
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) && layout.bounds().intersects(viewport) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self, _: &widget::Tree, renderer: &mut Renderer, _: &Theme, _: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let quad = renderer::Quad { bounds: layout.bounds(), border: iced::border::rounded(self.radius), ..Default::default() };
        let t = since(self.start).as_secs_f32();
        renderer.fill_quad(quad, iced::Background::Gradient(Self::gradient(self.color, t).into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command bar's dimmer fades in `ease-out` over 40 ms: none before its first frame, 69 % of the way at 20 ms.
    #[test]
    fn the_dimmer_fades_in_like_desktop() {
        let start = Instant::now();
        let at = |n| Shown(Some((start, start + Duration::from_millis(n)))).faded();
        assert!(Shown(None).faded() < 1e-6 && at(0) < 1e-6, "clear before and at the first frame");
        assert!((at(20) - 0.69).abs() < 0.01 && at(40) == 1.0);
    }

    /// Desktop's `--skeleton-bg` shimmer rests its band on the box at the ends of its 4 s loop and is away halfway,
    /// with at most the 8 stops of a gradient from the left to the right edge.
    #[test]
    fn skeleton_shimmers_like_desktop() {
        for t in [0.0, 0.7, 1.3, 2.0, 2.9, 3.99] {
            let stops = shimmer(t);
            assert!(stops.len() <= 8 && stops[0].0 == 0.0 && stops.last().unwrap().0 == 1.0, "{t}: {stops:?}");
            assert!(stops.windows(2).all(|s| s[0].0 < s[1].0), "{t}: {stops:?}");
        }
        assert!(shimmer(0.0).iter().all(|s| s.1 > 0.45), "the band is on the box at the start");
        assert!(shimmer(SHIMMER).iter().all(|s| s.1 > 0.45), "and at the end of the loop");
        assert!(shimmer(2.0).iter().all(|s| s.1 < 0.05), "it is away halfway");
        let g = Skeleton::gradient(Color::from_rgba(0.0, 0.0, 0.0, 0.1), 0.0);
        let a = g.stops.iter().flatten().map(|s| s.color.a).fold(0.0, f32::max);
        assert!((a - 0.145).abs() < 1e-3, "the band at half opacity over the fill: {a}");
    }

    /// Desktop's `.2s transform` turn with the CSS `ease` curve: it goes on for 200 ms, and only for a change.
    #[test]
    fn turns_like_desktop() {
        assert!((crate::cubic_bezier(0.5, [0.25, 0.1, 0.25, 1.0]) - 0.802).abs() < 5e-3, "the CSS ease curve");
        let frames = Frames::from([(image::Handle::from_rgba(1, 1, vec![0; 4]), 0)]);
        let start = Instant::now();
        let gif = |turn| Gif { frames: frames.clone(), play: true, start, make: Box::new(image), turn };
        let h = || frames[0].0.clone();
        assert!(gif((0.0, -1.0, Instant::now())).image(h()).1, "a new turn goes on");
        assert!(!gif((0.0, -1.0, start - Duration::from_millis(300))).image(h()).1, "a turn ends after 200 ms");
        assert!(!gif((-1.0, -1.0, Instant::now())).image(h()).1, "no turn without a change");
    }
}

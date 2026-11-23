//! Desktop's "Confetti" command (`fireConfetti`): canvas-confetti bursts from the two bottom corners of a
//! click-through layer over the window, which closes 10.25 s later.
//!
//! The pieces move like canvas-confetti's `updateFetti`, one step per animation frame of 60 Hz. iced draws no rotated
//! shapes, so each square is the box around canvas-confetti's four corners and each circle the pill of its ellipse.
use crate::prelude::*;
use iced::advanced::layout::{self};
use iced::advanced::widget::{self};
use iced::advanced::{mouse, renderer};

/// Desktop's confetti window closes 250 ms after it opens plus 10 s.
pub(crate) const LIFE: Duration = Duration::from_millis(10_250);
/// The bursts start 250 ms after the window opens.
const DELAY: Duration = Duration::from_millis(250);
/// canvas-confetti moves the pieces once per `requestAnimationFrame`.
const STEP: Duration = Duration::from_micros(16_667);
/// canvas-confetti's default colors.
const COLORS: [u32; 7] = [0x26ccff, 0xa25afd, 0xff5e7e, 0x88ff5a, 0xfcff42, 0xffa62d, 0xff36ff];
/// Desktop's bursts from each corner: the part of 100 pieces, the spread, the start velocity and the scale.
const BURSTS: [(f32, f32, f32, f32); 5] =
    [(10.0, 26.0, 80.0, 1.0), (12.0, 60.0, 50.0, 1.0), (15.0, 100.0, 60.0, 0.8), (8.0, 120.0, 80.0, 1.2), (9.0, 120.0, 45.0, 1.0)];

#[derive(Debug, Clone)]
struct Piece {
    x: f32,
    y: f32,
    wobble: f32,
    wobble_speed: f32,
    velocity: f32,
    angle: f32,
    tilt_angle: f32,
    color: Color,
    circle: bool,
    random: f32,
    tilt_sin: f32,
    tilt_cos: f32,
    wobble_x: f32,
    wobble_y: f32,
    scalar: f32,
}

/// The pieces of a confetti that runs, and the frames that they moved.
#[derive(Debug, Clone)]
pub(crate) struct Confetti {
    started: Instant,
    steps: u32,
    size: Size,
    pieces: Vec<Piece>,
    seed: u64,
}

/// Desktop's `S`: larger windows get more and faster pieces.
fn scale(width: f32) -> f64 {
    match width {
        w if w >= 2000.0 => 1.3,
        w if w >= 1600.0 => 1.2,
        w if w >= 1280.0 => 1.1,
        _ => 1.0,
    }
}

impl Confetti {
    /// Fires the bursts of Desktop over an area of `size`.
    pub(crate) fn fire(size: Size) -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self { started: Instant::now(), steps: 0, size, pieces: Vec::new(), seed: seed | 1 }
    }

    /// xorshift64*, as `Math.random()`: a number in [0, 1).
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        (self.seed.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// canvas-confetti's `randomPhysics` for a piece at the bottom corner `x` (0 or 1).
    fn burst(&mut self, corner: f32) {
        let r = scale(self.size.width);
        for (part, spread, velocity, scalar) in BURSTS {
            // JavaScript's `Math.floor(k * R * C)` in doubles.
            let count = (f64::from(part) * r * 100.0).floor() as usize;
            let r = r as f32;
            let (spread, velocity, scalar) = ((spread * r).to_radians(), velocity * r, scalar * r);
            for _ in 0..count {
                let color = COLORS[self.pieces.len() % COLORS.len()];
                let color = Color::from_rgb8((color >> 16) as u8, (color >> 8) as u8, color as u8);
                let circle = self.random() < 0.5;
                let piece = Piece {
                    x: corner * self.size.width,
                    y: self.size.height,
                    wobble: self.random() * 10.0,
                    wobble_speed: (self.random() * 0.1 + 0.05).min(0.11),
                    velocity: velocity * 0.5 + self.random() * velocity,
                    angle: -std::f32::consts::FRAC_PI_2 + (0.5 * spread - self.random() * spread),
                    tilt_angle: (self.random() * 0.5 + 0.25) * std::f32::consts::PI,
                    color,
                    circle,
                    random: self.random() + 2.0,
                    tilt_sin: 0.0,
                    tilt_cos: 0.0,
                    wobble_x: 0.0,
                    wobble_y: 0.0,
                    scalar,
                };
                self.pieces.push(piece);
            }
        }
    }

    /// Whether the layer still shows.
    pub(crate) fn running(&self) -> bool {
        self.started.elapsed() < LIFE
    }

    /// Moves the pieces by the frames since the last move, and drops those that fell off the bottom.
    pub(crate) fn advance(&mut self) {
        self.advance_to(self.started.elapsed());
    }

    /// [`Self::advance`] to `elapsed` after the fire.
    pub(crate) fn advance_to(&mut self, elapsed: Duration) {
        let Some(since) = elapsed.checked_sub(DELAY) else { return };
        if self.steps == 0 && self.pieces.is_empty() {
            self.burst(0.0);
            self.burst(1.0);
        }
        let due = (since.as_micros() / STEP.as_micros()) as u32 + 1;
        while self.steps < due {
            self.steps += 1;
            for i in 0..self.pieces.len() {
                let random = self.random() + 2.0;
                let p = &mut self.pieces[i];
                // canvas-confetti's `updateFetti` with Desktop's gravity of 2, which is 6 px a frame, and decay 0.9.
                p.x += p.angle.cos() * p.velocity;
                p.y += p.angle.sin() * p.velocity + 6.0;
                p.velocity *= 0.9;
                p.wobble += p.wobble_speed;
                p.wobble_x = p.x + 10.0 * p.scalar * p.wobble.cos();
                p.wobble_y = p.y + 10.0 * p.scalar * p.wobble.sin();
                p.tilt_angle += 0.1;
                (p.tilt_sin, p.tilt_cos) = p.tilt_angle.sin_cos();
                p.random = random;
            }
            // A piece below the area moves down from there on.
            let bottom = self.size.height + 40.0;
            self.pieces.retain(|p| p.y < bottom || p.angle.sin() * p.velocity + 6.0 < 0.0);
        }
    }

    /// The pieces as boxes: canvas-confetti's quadrilateral of a square, or the ellipse of a circle.
    fn boxes(&self) -> impl Iterator<Item = (Rectangle, f32, Color)> + '_ {
        self.pieces.iter().filter(|_| self.steps > 0).map(|p| {
            let (t, o) = (p.x + p.random * p.tilt_cos, p.y + p.random * p.tilt_sin);
            let (i, l) = (p.wobble_x + p.random * p.tilt_cos, p.wobble_y + p.random * p.tilt_sin);
            match p.circle {
                true => {
                    let (rx, ry) = ((i - t).abs() * 0.6, (l - o).abs() * 0.6);
                    let r = Rectangle::new([p.x - rx, p.y - ry].into(), Size::new(2.0 * rx, 2.0 * ry));
                    (r, rx.min(ry), p.color)
                }
                false => {
                    let xs = [p.x, p.wobble_x, i, t];
                    let ys = [p.y, o, l, p.wobble_y];
                    let (x0, x1) = xs.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
                    let (y0, y1) = ys.iter().fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
                    (Rectangle::new([x0, y0].into(), Size::new(x1 - x0, y1 - y0)), 0.0, p.color)
                }
            }
        })
    }

    /// The number of pieces that show.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.pieces.len()
    }

    /// The layer that draws the pieces. It takes no pointer events, like Desktop's `setIgnoreMouseEvents`.
    pub(crate) fn view<'a, M: 'a>(&'a self) -> Element<'a, M> {
        Element::new(Layer(self))
    }
}

struct Layer<'a>(&'a Confetti);

impl<M> Widget<M, Theme, iced::Renderer> for Layer<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self, _: &widget::Tree, renderer: &mut iced::Renderer, _: &Theme, _: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let origin = layout.bounds().position();
        for (b, radius, color) in self.0.boxes() {
            let bounds = Rectangle { x: origin.x + b.x, y: origin.y + b.y, ..b };
            if bounds.intersects(viewport) {
                let quad = renderer::Quad { bounds, border: iced::border::rounded(radius), ..Default::default() };
                renderer.fill_quad(quad, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's 5 bursts of 1000, 1200, 1500, 800 and 900 pieces from each bottom corner, 250 ms after the fire,
    /// with more pieces in a wider window, which fall off the bottom within seconds.
    #[test]
    fn the_bursts_follow_desktops_confetti() {
        let mut c = Confetti::fire(Size::new(1000.0, 800.0));
        c.advance_to(Duration::from_millis(200));
        assert_eq!((c.count(), c.boxes().count()), (0, 0), "nothing before 250 ms");
        c.advance_to(Duration::from_millis(270));
        assert_eq!(c.count(), 2 * 5400);
        let high = c.boxes().map(|b| b.0.y).fold(f32::MAX, f32::min);
        c.advance_to(Duration::from_millis(800));
        let higher = c.boxes().map(|b| b.0.y).fold(f32::MAX, f32::min);
        assert!(higher < high - 300.0, "the pieces fly up: {high} then {higher}");
        assert!(c.boxes().all(|(_, _, color)| COLORS.iter().any(|&h| Color::from_rgb8((h >> 16) as u8, (h >> 8) as u8, h as u8) == color)));
        c.advance_to(Duration::from_secs(8));
        assert_eq!(c.count(), 0, "all fell off the bottom");
        let mut wide = Confetti::fire(Size::new(1600.0, 800.0));
        wide.advance_to(Duration::from_millis(270));
        assert_eq!(wide.count(), 2 * 6479, "1.2 times the pieces, rounded down per burst like JavaScript");
    }
}

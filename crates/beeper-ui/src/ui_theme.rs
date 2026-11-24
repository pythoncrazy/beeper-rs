//! Desktop 4.3.308's theme colors (Settings > Appearance > Theme > Theme color, local `ui_theme`): `body.theme-monochrome`
//! with a `--theme-hue`, and `body.theme-rich` with a `data-theme-color`, which override the base colors on `body`
//! (App css 29298-29521) and show a gradient behind the transparent `--left-pane-bg`. The tokens that `:root`
//! computes from the overridden ones keep Desktop's own colors, as CSS custom properties resolve where they are
//! declared.
use crate::delegate::delegate;
use crate::i18n::tr;
use crate::prelude::*;
use crate::settings;
use crate::style::fixed;
use beeper_core::models::UiTheme;
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self};
use iced::border;
use iced::widget::column;

/// Desktop's `f`: the rich gradient moves by its 200% in 2 hours.
const RICH_PERIOD_MS: i64 = 7_200_000;
/// Desktop's `setTimeout(y, f / 200)`: the rich gradient moves every 36 seconds.
pub const RICH_STEP: Duration = Duration::from_millis(RICH_PERIOD_MS as u64 / 200);

/// The gradient behind the window of a theme color, which CSS draws over the whole `body`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backdrop {
    /// `--theme-background-gradient` or `--color-from`.
    pub from: Color,
    /// The end of `--theme-background-gradient` or `--color-to`.
    pub to: Color,
    /// `--theme-bg`: the rich gradient, which repeats and moves by `offset`.
    pub rich: bool,
    /// `--theme-bg-offset` in percent, 0 to 200.
    pub offset: f32,
}

/// CSS `hsl(h, s%, l%)`.
fn hsl(h: f32, s: f32, l: f32) -> Color {
    let (s, l) = (s / 100.0, l / 100.0);
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    Color::from_rgb(f(0.0), f(8.0), f(4.0))
}

/// `color-mix(in srgb, c p%, transparent)`.
fn mixed(c: Color, p: f32) -> Color {
    c.scale_alpha(p / 100.0)
}

/// Desktop's `lightGradient` and `darkGradient`, `--color-from` and `--color-to`.
pub fn rich_colors(t: UiTheme, dark: bool) -> (Color, Color) {
    let (light, dark_) = match t {
        UiTheme::RichRed => ((0xfe9a9a, 0xfecea9), (0x6a0303, 0x3c0000)),
        UiTheme::RichGreen => ((0x80ea8b, 0xeffb81), (0x4a6729, 0x163d2c)),
        UiTheme::RichBlue => ((0x80cbea, 0x81fbae), (0x04409b, 0x001b44)),
        UiTheme::RichPurple => ((0xc6a7f4, 0x7ea1f9), (0x6e16b2, 0x400170)),
        _ => ((0xf9f9f9, 0x6e6e6e), (0x454545, 0x000000)),
    };
    let (a, b) = if dark { dark_ } else { light };
    (hex(a), hex(b))
}

/// The `--color-primary` of a rich theme, of which Rich Blue sets only the light one (so the dark theme keeps it)
/// and Rich Grey none.
fn rich_primary(t: UiTheme, dark: bool) -> Option<Color> {
    match (t, dark) {
        (UiTheme::RichRed, false) => Some(hex(0xfb3939)),
        (UiTheme::RichRed, true) => Some(hex(0xc40b0b)),
        (UiTheme::RichGreen, false) => Some(hex(0x1cc62c)),
        (UiTheme::RichGreen, true) => Some(hex(0x3cb217)),
        (UiTheme::RichBlue, _) => Some(hex(0x089bdb)),
        (UiTheme::RichPurple, false) => Some(hex(0x540dfb)),
        (UiTheme::RichPurple, true) => Some(hex(0x6a4be5)),
        _ => None,
    }
}

/// `--linked-message-bg` of a rich theme. Without it Desktop's `var(--linked-message-bg, --color-primary)` is not a
/// color, so the quote of your own message takes the color of its parent, the text color.
fn linked_message(t: UiTheme, dark: bool) -> Option<Color> {
    match (t, dark) {
        (UiTheme::RichRed, false) => Some(hex(0xfd6868)),
        (UiTheme::RichRed | UiTheme::RichBlue, true) => Some(hex(0xc40b0b)),
        _ => None,
    }
}

/// Desktop's `--theme-bg-offset` at `now_ms`.
pub fn rich_offset(now_ms: i64) -> f32 {
    (now_ms.rem_euclid(RICH_PERIOD_MS)) as f32 / RICH_PERIOD_MS as f32 * 200.0
}

/// The palette of `p` under `theme`.
pub fn apply(p: &mut Palette, theme: UiTheme, dark: bool, now_ms: i64) {
    let primary = match theme.hue() {
        Some(hue) => {
            let h = f32::from(hue);
            let g = |l: f32| hsl(h, 10.0, l);
            // `body.theme-monochrome` and its `prefers-color-scheme: dark`.
            let (primary, strong, muted, received, from, to) = match dark {
                false => (hsl(h, 50.0, 45.0), hsl(h, 69.0, 10.0), g(52.0), hsl(h, 10.0, 96.0), hsl(h, 87.0, 96.0), hsl(h, 100.0, 92.0)),
                true => (hsl(h, 50.0, 40.0), hsl(h, 69.0, 96.0), g(49.0), g(16.0), hsl(h, 40.0, 16.0), hsl(h, 40.0, 10.0)),
            };
            p.strong = strong;
            p.divider = strong.scale_alpha(0.2);
            p.muted = muted;
            (p.received, p.chip, p.link_card) = (received, received, received);
            p.backdrop = Some(Backdrop { from, to, rich: false, offset: 0.0 });
            // `--gradient-brand` and `--color-brand`.
            p.brand_stops = Some(match dark {
                false => [hsl(h, 86.0, 40.0), hsl(h, 75.0, 30.0)],
                true => [hsl(h, 86.0, 30.0), hsl(h, 75.0, 23.0)],
            });
            p.brand = primary;
            primary
        }
        None => {
            let (from, to) = rich_colors(theme, dark);
            p.backdrop = Some(Backdrop { from, to, rich: true, offset: rich_offset(now_ms) });
            p.quote = Some(linked_message(theme, dark).unwrap_or(p.text));
            // `.message-download-btn` and `.Transcribe-module__button`.
            p.action = hex(0x0d4ffb);
            // The dark `PinnedThreadListItem` bubbles, `AccountBadge` dots and unread counts are white.
            p.badge = dark.then_some(from);
            rich_primary(theme, dark).unwrap_or(p.accent)
        }
    };
    (p.accent, p.sent) = (primary, primary);
    if theme.hue().is_some() {
        p.action = primary;
    }
    // `--toggle-style-knob-on`, `--checkbox-style-bg-selected` and `--app-focus-ring`.
    (p.toggle_on, p.focus_ring) = (primary, primary);
    // `.button-sentiment--main`: the primary color, and 14% of it (26% in the dark theme) behind the secondary text.
    p.main = primary;
    p.main_muted = mixed(primary, if dark { 26.0 } else { 14.0 });
    p.main_text = primary;
}

/// The color stops of a CSS `linear-gradient(<angle>deg, ...)` in `bounds`, with the color of the CSS line at `f` of
/// its length and a stop at each of `breaks`. iced draws a [`Linear`] between the points where its line meets the box
/// instead of the corners, so the stops are the CSS colors at the ends of iced's line, and the corners beyond them
/// take their colors; [`css_box`] gives the box whose iced line is the CSS line of `bounds`, where `exact` is true.
fn css_linear(angle: f32, bounds: Size, color_at: impl Fn(f32) -> Color, breaks: &[f32], exact: bool) -> Linear {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (w, h) = (sin.abs() * bounds.width / 2.0, cos.abs() * bounds.height / 2.0);
    let k = match w + h {
        css if css > 0.0 && !exact => w.max(h) / css,
        _ => 1.0,
    };
    // The CSS fraction at iced's `g`, and back.
    let at = |g: f32| 0.5 + (g - 0.5) * k;
    let (lo, hi) = (at(0.0), at(1.0));
    let mut line = Linear::new(Radians(angle.to_radians())).add_stop(0.0, color_at(lo));
    for &f in breaks.iter().filter(|&&f| f > lo && f < hi) {
        line = line.add_stop(0.5 + (f - 0.5) / k, color_at(f));
    }
    line.add_stop(1.0, color_at(hi))
}

/// The box around the middle of `area` whose iced gradient line at `angle` is the CSS one of `area`, which ends at
/// its corners: `area` made wider (or higher) until iced's line is as long.
fn css_box(angle: f32, area: Rectangle) -> Rectangle {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (s, c) = (sin.abs(), cos.abs());
    let half = s * area.width / 2.0 + c * area.height / 2.0;
    let size = match s * area.width >= c * area.height && s > 1e-6 {
        true => Size::new(2.0 * half / s, area.height),
        false => Size::new(area.width, 2.0 * half / c.max(1e-6)),
    };
    let center = area.center();
    Rectangle::new(iced::Point::new(center.x - size.width / 2.0, center.y - size.height / 2.0), size)
}

/// A monochrome `--gradient-brand`: `stops` at -7.24% and 109.66% of the 190 degree line, in a box of `size`, each
/// color through `lit`.
pub fn brand_line(stops: [Color; 2], size: Size, lit: impl Fn(Color) -> Color) -> Linear {
    let at = |f: f32| lit(crate::style::mix(stops[0], stops[1], ((f + 0.0724) / 1.169).clamp(0.0, 1.0)));
    css_linear(190.0, size, at, &[], false)
}

impl Backdrop {
    /// The color in the middle of the gradient, for what Desktop cuts out of the window.
    pub fn middle(&self) -> Color {
        crate::style::mix(self.from, self.to, 0.5)
    }

    /// The gradient in a box of `size`: `--theme-background-gradient` at 135 degrees, or `--theme-bg` at 121.73
    /// degrees, `from` at -200%, 0% and 200% of the line and `to` at -100% and 100%, moved by the offset.
    /// `--theme-background-gradient` at 135 degrees, or `--theme-bg` at 121.73 degrees.
    fn angle(&self) -> f32 {
        if self.rich {
            121.73
        } else {
            135.0
        }
    }

    /// The gradient in a box of `size`: `from` to `to`, or for the rich one `from` at -200%, 0% and 200% of the line
    /// and `to` at -100% and 100%, moved by the offset. `exact` is for the [`css_box`] of the box.
    pub fn gradient(&self, size: Size, exact: bool) -> Background {
        let (a, b) = (self.from, self.to);
        let lerp = |t: f32| crate::style::mix(a, b, t);
        let line = match self.rich {
            false => css_linear(self.angle(), size, lerp, &[], exact),
            true => {
                let o = self.offset / 100.0;
                // From -200% + offset the colors run from, to, from, to, a stop each 100%.
                let color_at = |f: f32| {
                    let t = (f - o + 2.0).clamp(0.0, 3.0);
                    let seg = t.floor().min(2.0);
                    let frac = t - seg;
                    if seg == 1.0 {
                        lerp(1.0 - frac)
                    } else {
                        lerp(frac)
                    }
                };
                css_linear(self.angle(), size, color_at, &[o - 1.0, o], exact)
            }
        };
        Background::Gradient(line.into())
    }
}

/// `content` over the backdrop `b`, which is the size of `frame` (the window) from the top left corner of its box when
/// given, else of its box, and clipped to the box's rounded corners of `radius`; or over `fill` without one.
pub struct WindowBg<'a, Message> {
    content: Element<'a, Message>,
    backdrop: Option<Backdrop>,
    fill: Color,
    radius: f32,
    frame: Option<Size>,
}

/// `content` over the theme's backdrop, or over `fill` without a theme color.
pub fn window_bg<'a, Message>(content: impl Into<Element<'a, Message>>, backdrop: Option<Backdrop>, fill: Color) -> WindowBg<'a, Message> {
    WindowBg { content: content.into(), backdrop, fill, radius: 0.0, frame: None }
}

impl<Message> WindowBg<'_, Message> {
    pub fn radius(self, radius: f32) -> Self {
        Self { radius, ..self }
    }

    /// CSS `background-size: 100vw 100vh`: the gradient is the size of the window.
    pub fn frame(self, frame: Size) -> Self {
        Self { frame: Some(frame), ..self }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for WindowBg<'_, Message> {
    delegate!(children, diff, size, layout, operate, update(Message), mouse_interaction, overlay(Message));

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>,
        cursor: mouse::Cursor, viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let border = border::rounded(self.radius);
        match (self.backdrop, self.frame) {
            (None, _) if self.fill == CLEAR => {}
            (None, _) => renderer.fill_quad(Quad { bounds, border, ..Quad::default() }, self.fill),
            // Rounded corners clip only the quad itself, so its gradient ends where iced's line does.
            (Some(b), _) if self.radius > 0.0 => {
                renderer.fill_quad(Quad { bounds, border, ..Quad::default() }, b.gradient(bounds.size(), false))
            }
            // The wider box of the CSS line is clipped to the bounds in a layer of its own, and the content goes in
            // the next layer, as iced draws the layers in order after the one they started from.
            (Some(b), frame) => {
                renderer.with_layer(bounds, |r| {
                    let area = frame.map_or(bounds, |f| Rectangle::new(bounds.position(), f));
                    let quad = Quad { bounds: css_box(b.angle(), area), ..Quad::default() };
                    r.fill_quad(quad, b.gradient(area.size(), true));
                });
                let content = &self.content;
                return renderer
                    .with_layer(*viewport, |r| content.as_widget().draw(&tree.children[0], r, theme, style, layout, cursor, viewport));
            }
        }
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }
}

impl<'a, Message: 'a> From<WindowBg<'a, Message>> for Element<'a, Message> {
    fn from(b: WindowBg<'a, Message>) -> Self {
        Element::new(b)
    }
}

/// `.theme-hue-button:after`: a 40 px circle of `linear-gradient(135deg, a 50%, b 50%)`.
fn swatch<'a>(a: Color, b: Color) -> El<'a> {
    let split = Linear::new(Radians(135f32.to_radians())).add_stop(0.5, a).add_stop(0.5001, b);
    let fill = Background::Gradient(split.into());
    let style = move |_: &Theme| container::Style { background: Some(fill), border: border::rounded(20), ..Default::default() };
    container(Space::new()).width(40).height(40).style(style).into()
}

/// The id of the swatch of `theme`, or of the reset swatch.
pub(crate) fn swatch_id(theme: Option<UiTheme>) -> String {
    format!("theme-color-{}", theme.map_or("reset", UiTheme::id))
}

impl BeeperGuiApp {
    /// Desktop's Theme Color Selector (`vFt`): "Theme color" with its Labs tag over the reset swatch and a swatch of
    /// each theme, in a `.label-wrapper` of the `.prefs-action-group`. The current one is on
    /// `--color-background-selected`, a square without hover or focus styles (`background: transparent !important`
    /// and `button { outline: 0 }`).
    pub(crate) fn theme_colors(&self, current: Option<UiTheme>) -> El<'_> {
        let p = &self.pal;
        // `.tag`: 12 px on `--color-background-tag`, `margin-left: 0.5em` after a space.
        // Desktop's catalogs have no "Labs" of "preferences experimental feature tag", so it is English everywhere.
        let tag = fixed("🧪 Labs", 12.0, Weight::Normal);
        let tag = container(tag.color(p.text)).padding([1, 4]).style(fill_box(p.fg.scale_alpha(0.1), 4.0));
        let tag = self.tip(tag.into(), tr("Experimental and subject to change"), vec![], tooltip::Position::Top);
        let title = row![txt(tr("Theme color"), 13.0).color(p.text).line_height(lh(18.2)), container(tag)]
            .spacing(crate::style::text_width(" ", 13.0, Weight::Normal) + 6.0)
            .align_y(Center);
        // `.theme-hue-button.monochrome.theme-monochrome-unset` first, then Desktop's `ite`.
        let grey = (hsl(0.0, 0.0, 60.0), hsl(0.0, 0.0, 30.0));
        let swatches = std::iter::once((None, grey)).chain(UiTheme::ALL.into_iter().map(|t| {
            let colors = match t.hue() {
                Some(h) => (hsl(f32::from(h), 60.0, 60.0), hsl(f32::from(h), 70.0, 30.0)),
                None => rich_colors(t, self.dark),
            };
            (Some(t), colors)
        }));
        let window = self.settings_window;
        let buttons = swatches.map(|(t, (a, b))| {
            let pick = UiMessage::Settings(settings::Msg::SelectUiTheme(t));
            let bg = if t == current { p.selected } else { CLEAR };
            let b = button(swatch(a, b)).padding(8).style(btn_style([bg; 3], [p.text; 3], 0.0)).on_press(pick.clone());
            let b = self.target(window, swatch_id(t), b, Keys::press(pick), Look::Hidden);
            // `margin: 4px 4px 0 0`, on a line of the 13 px text, whose descent is under the inline-block.
            container(b).padding(Padding::new(0.0).top(4.0).right(4.0).bottom(4.4)).into()
        });
        let swatches: Vec<El<'_>> = buttons.collect();
        let colors = column![title, Row::with_children(swatches).wrap()];
        container(colors).width(iced::Fill).padding(rem(13.0)).into()
    }

    /// Desktop's `setTimeout` of the rich gradient, every 36 s while a rich theme shows.
    pub(crate) fn theme_ticks(&self) -> Option<Subscription<UiMessage>> {
        let rich = self.pal.backdrop.is_some_and(|b| b.rich);
        rich.then(|| iced::time::every(RICH_STEP).map(|_| UiMessage::Settings(settings::Msg::ThemeTick)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Msg;
    use crate::SettingsTab;
    use iced::Point;
    use iced_test::simulator;

    fn rgb(c: Color) -> u32 {
        let [r, g, b, _] = c.into_rgba8();
        u32::from_be_bytes([0, r, g, b])
    }

    /// Desktop's CSS resolved on `body` for each theme, light then dark: `--color-primary`, `--color-on-surface`,
    /// `--color-on-surface-variant`, `--color-secondary-container`, `--color-background-message-bubble-received`
    /// and the two colors of the backdrop.
    const DESKTOP: [(UiTheme, [[u32; 7]; 2]); 9] = [
        (
            UiTheme::MonochromeRed,
            [
                [0xac3939, 0x2b0808, 0x917878, 0xf6f4f4, 0xf6f4f4, 0xfeecec, 0xffd6d6],
                [0x993333, 0xfceeee, 0x897070, 0x2d2525, 0x2d2525, 0x391818, 0x240f0f],
            ],
        ),
        (
            UiTheme::MonochromeGreen,
            [
                [0x39ac39, 0x082b08, 0x789178, 0xf4f6f4, 0xf4f6f4, 0xecfeec, 0xd6ffd6],
                [0x339933, 0xeefcee, 0x708970, 0x252d25, 0x252d25, 0x183918, 0x0f240f],
            ],
        ),
        (
            UiTheme::MonochromeBlue,
            [
                [0x3973ac, 0x08192b, 0x788591, 0xf4f5f6, 0xf4f5f6, 0xecf5fe, 0xd6ebff],
                [0x336699, 0xeef5fc, 0x707d89, 0x25292d, 0x25292d, 0x182939, 0x0f1924],
            ],
        ),
        (
            UiTheme::MonochromePurple,
            [
                [0x7339ac, 0x19082b, 0x857891, 0xf5f4f6, 0xf5f4f6, 0xf5ecfe, 0xebd6ff],
                [0x663399, 0xf5eefc, 0x7d7089, 0x29252d, 0x29252d, 0x291839, 0x190f24],
            ],
        ),
        (
            UiTheme::RichRed,
            [
                [0xfb3939, 0x202124, 0x808080, 0xefeff0, 0xededed, 0xfe9a9a, 0xfecea9],
                [0xc40b0b, 0xf6f6f6, 0x808080, 0x232326, 0x282828, 0x6a0303, 0x3c0000],
            ],
        ),
        (
            UiTheme::RichGreen,
            [
                [0x1cc62c, 0x202124, 0x808080, 0xefeff0, 0xededed, 0x80ea8b, 0xeffb81],
                [0x3cb217, 0xf6f6f6, 0x808080, 0x232326, 0x282828, 0x4a6729, 0x163d2c],
            ],
        ),
        (
            UiTheme::RichBlue,
            [
                [0x089bdb, 0x202124, 0x808080, 0xefeff0, 0xededed, 0x80cbea, 0x81fbae],
                [0x089bdb, 0xf6f6f6, 0x808080, 0x232326, 0x282828, 0x04409b, 0x001b44],
            ],
        ),
        (
            UiTheme::RichPurple,
            [
                [0x540dfb, 0x202124, 0x808080, 0xefeff0, 0xededed, 0xc6a7f4, 0x7ea1f9],
                [0x6a4be5, 0xf6f6f6, 0x808080, 0x232326, 0x282828, 0x6e16b2, 0x400170],
            ],
        ),
        (
            UiTheme::RichGrey,
            [
                [0x0d4ffb, 0x202124, 0x808080, 0xefeff0, 0xededed, 0xf9f9f9, 0x6e6e6e],
                [0x2561fb, 0xf6f6f6, 0x808080, 0x232326, 0x282828, 0x454545, 0x000000],
            ],
        ),
    ];

    /// Each theme color sets the colors that Desktop's CSS gives `body` in the light and the dark theme: the primary
    /// color of the accent, the sent bubbles, the toggles, checkboxes, focus rings and `Button`s, the monochrome text
    /// and gray colors, and the gradient behind the window. The rest keeps Desktop's own colors.
    #[test]
    fn the_palette_of_each_theme_color_follows_desktops_css() {
        for (theme, modes) in DESKTOP {
            for (dark, [primary, strong, muted, received, link_card, from, to]) in [(false, modes[0]), (true, modes[1])] {
                let at = format!("{theme:?} dark={dark}");
                let p = crate::style::palette(dark, Some(theme), 0);
                let base = crate::style::palette(dark, None, 0);
                let close = |a: u32, b: u32| a.to_be_bytes().iter().zip(b.to_be_bytes()).all(|(x, y)| x.abs_diff(y) <= 1);
                for (name, c) in [
                    ("accent", p.accent),
                    ("sent", p.sent),
                    ("toggle_on", p.toggle_on),
                    ("focus_ring", p.focus_ring),
                    ("main", p.main),
                    ("main_text", p.main_text),
                ] {
                    assert!(close(rgb(c), primary), "{at}: {name}");
                }
                let muted_main = if dark { 0.26 } else { 0.14 };
                assert!((p.main_muted.a - muted_main).abs() < 1e-6 && close(rgb(p.main_muted), primary), "{at}");
                for (name, c, want) in [
                    ("strong", p.strong, strong),
                    ("muted", p.muted, muted),
                    ("received", p.received, received),
                    ("chip", p.chip, received),
                    ("link_card", p.link_card, link_card),
                    ("divider", p.divider, strong),
                ] {
                    assert!(close(rgb(c), want), "{at}: {name} {:06x} is not {want:06x}", rgb(c));
                }
                let b = p.backdrop.expect("a backdrop");
                assert!(close(rgb(b.from), from) && close(rgb(b.to), to) && b.rich == theme.hue().is_none(), "{at}");
                // `:root` resolves these from its own base colors, so the theme keeps them.
                for (name, c, d) in [
                    ("text", p.text, base.text),
                    ("hover", p.hover, base.hover),
                    ("selected", p.selected, base.selected),
                    ("card", p.card, base.card),
                    ("window", p.window, base.window),
                    ("sent_fg", p.sent_fg, base.sent_fg),
                ] {
                    assert_eq!(c, d, "{at}: {name}");
                }
                let rich = theme.hue().is_none();
                assert!(close(rgb(p.action), if rich { 0x0d4ffb } else { primary }), "{at}: the download button");
                assert_eq!(p.badge.is_some(), rich && dark, "{at}: the white badges");
                assert!(p.badge.is_none_or(|c| close(rgb(c), from)), "{at}: their text");
                assert_eq!(p.brand_stops.is_some(), !rich, "{at}: the brand gradient");
            }
        }
        let quote = |t, dark| crate::style::palette(dark, Some(t), 0).quote.map(rgb);
        assert_eq!(quote(UiTheme::RichRed, false), Some(0xfd6868));
        assert_eq!([quote(UiTheme::RichRed, true), quote(UiTheme::RichBlue, true)], [Some(0xc40b0b); 2]);
        let text = rgb(BEEPER_TEXT_LIGHT);
        assert_eq!(quote(UiTheme::RichGreen, false), Some(text), "without --linked-message-bg the quote inherits");
        assert_eq!(quote(UiTheme::MonochromeRed, false), None);
        assert!(crate::style::palette(false, None, 0).backdrop.is_none());
    }

    const BEEPER_TEXT_LIGHT: Color = crate::style::BEEPER_LIGHT.text;

    /// Desktop's `--theme-bg-offset`: 200% over 2 hours, from the clock.
    #[test]
    fn the_rich_gradient_moves_with_the_clock_like_desktop() {
        assert_eq!(rich_offset(0), 0.0);
        assert_eq!(rich_offset(3_600_000), 100.0);
        assert_eq!(rich_offset(7_200_000 + 36_000), 1.0, "one step of 36 s is 1%");
        assert_eq!(RICH_STEP, Duration::from_secs(36));
        let p = crate::style::palette(false, Some(UiTheme::RichBlue), 1_800_000);
        assert_eq!(p.backdrop.map(|b| b.offset), Some(50.0));
        assert_eq!(crate::style::palette(false, Some(UiTheme::MonochromeBlue), 1_800_000).backdrop.unwrap().offset, 0.0);
    }

    /// Desktop's CSS color of the gradient at `at` in a box of `size`: the position on the line through the middle
    /// at `angle`, whose ends meet the corners.
    fn css_at(b: &Backdrop, size: Size, at: Point) -> Color {
        let (angle, colors) = match b.rich {
            false => (135.0f32, vec![(0.0, b.from), (1.0, b.to)]),
            true => {
                let o = b.offset / 100.0;
                (121.73, vec![(o - 2.0, b.from), (o - 1.0, b.to), (o, b.from), (o + 1.0, b.to)])
            }
        };
        let (sin, cos) = angle.to_radians().sin_cos();
        let len = (size.width * sin).abs() + (size.height * cos).abs();
        let f = ((at.x - size.width / 2.0) * sin - (at.y - size.height / 2.0) * cos) / len + 0.5;
        let i = colors.iter().rposition(|(s, _)| *s <= f).unwrap_or(0).min(colors.len() - 2);
        let ((s0, c0), (s1, c1)) = (colors[i], colors[i + 1]);
        crate::style::mix(c0, c1, ((f - s0) / (s1 - s0)).clamp(0.0, 1.0))
    }

    /// The gradient behind the window is Desktop's CSS gradient over the whole window, which reaches the corners,
    /// at each offset of the rich one.
    #[test]
    fn the_backdrop_draws_desktops_gradient() {
        let size = Size::new(1000.0, 600.0);
        for (theme, dark, now) in [
            (UiTheme::MonochromeBlue, false, 0),
            (UiTheme::MonochromePurple, true, 0),
            (UiTheme::RichPurple, false, 0),
            (UiTheme::RichGreen, true, 1_400_000),
            (UiTheme::RichRed, false, 5_000_000),
        ] {
            let p = crate::style::palette(dark, Some(theme), now);
            let b = p.backdrop.expect("a backdrop");
            let view: El<'_> = window_bg(Space::new().width(iced::Fill).height(iced::Fill), p.backdrop, p.window).into();
            let shot = crate::focus::tests::shot(view, size, "theme-backdrop");
            let points = [(2.0, 2.0), (998.0, 598.0), (500.0, 300.0), (150.0, 450.0), (800.0, 100.0), (640.0, 520.0)];
            let off: Vec<_> = points
                .into_iter()
                .map(|(x, y)| {
                    let k = shot.width() as f32 / size.width;
                    let px = shot.get_pixel((x * k) as u32, (y * k) as u32).0;
                    (x, y, px, css_at(&b, size, Point::new(x + 0.5, y + 0.5)).into_rgba8())
                })
                .filter(|(.., px, want)| px.iter().zip(want).any(|(a, b)| a.abs_diff(*b) > 3))
                .collect();
            assert!(off.is_empty(), "{theme:?} dark={dark}: {off:?}");
        }
    }

    fn open(app: &mut BeeperGuiApp) {
        let _ = app.update(UiMessage::Settings(Msg::Open(SettingsTab::Appearance)));
    }

    /// Desktop's Theme Color Selector in the Theme group: "Theme color" with its Labs tag, then the reset swatch and
    /// the nine themes in Desktop's order. A click sets `ui_theme` to the theme's id, which the next start reads,
    /// the reset swatch clears it, and the current one is on `--color-background-selected`.
    #[test]
    fn the_theme_color_selector_sets_ui_theme_like_desktop() {
        let config = beeper_core::testing::demo_config("theme-color-selector");
        let store = beeper_core::store::StoreLock::shared(beeper_core::store::BeeperStore::from_config(config.clone()));
        let mut app = BeeperGuiApp::boot(store).0;
        open(&mut app);
        let swatches: Vec<_> = std::iter::once(None).chain(UiTheme::ALL.map(Some)).collect();
        {
            let mut sim = simulator(app.view_settings());
            let title = sim.find("Theme color").expect("the title").bounds();
            let mut own = simulator(app.theme_colors(None));
            let (title_alone, labs) = (own.find("Theme color").expect("the title").bounds(), own.find("🧪 Labs"));
            let labs = labs.expect("the Labs tag").bounds();
            let (t, l) = (title_alone, labs);
            assert!(l.x > t.x + t.width && (l.center_y() - t.center_y()).abs() < 2.0, "the tag follows: {l:?} {t:?}");
            let mut last = None::<Rectangle>;
            for &t in &swatches {
                let at = sim.find(iced::widget::Id::from(swatch_id(t))).expect("a swatch").bounds();
                assert_eq!((at.width, at.height), (56.0, 56.0), "a 40 px circle with 8 px around it");
                assert!(at.y > title.y + title.height, "{t:?} is under the title");
                if let Some(l) = last.filter(|l| l.y == at.y) {
                    assert_eq!(at.x - (l.x + l.width), 4.0, "{t:?}: `margin-right: 4px`");
                }
                last = Some(at);
            }
        }
        for &t in swatches.iter().skip(1).chain([&None]) {
            let current = app.store.read().settings.ui_theme;
            let msgs = crate::fuzz::clicks(app.theme_colors(current), iced::widget::Id::from(swatch_id(t)));
            assert!(msgs.iter().any(|m| matches!(m, UiMessage::Settings(Msg::SelectUiTheme(x)) if *x == t)), "{t:?}");
            msgs.into_iter().for_each(|m| drop(app.update(m)));
            assert_eq!(app.store.read().settings.ui_theme, t);
            assert_eq!(app.pal.backdrop.is_some(), t.is_some(), "{t:?} recolors the window");
        }
        let _ = app.update(UiMessage::Settings(Msg::SelectUiTheme(Some(UiTheme::RichGrey))));
        drop(app);
        let st = beeper_core::store::BeeperStore::from_config(config);
        assert_eq!(st.settings.ui_theme, Some(UiTheme::RichGrey), "the theme color stays");
        let json = serde_json::to_value(&st.settings).expect("the settings");
        assert_eq!(json["ui_theme"], "rich-grey", "kept under Desktop's id");
    }

    /// The current swatch is on `--color-background-selected` and the others on nothing, and each circle is split
    /// at 135 degrees between its two colors: the reset grays, `hsl(hue, 60%, 60%)` and `hsl(hue, 70%, 30%)`, or
    /// the gradient of a rich theme in the light or dark theme.
    #[test]
    fn the_swatches_look_like_desktop() {
        let mut app = crate::fuzz::demo_app("theme-color-swatches");
        open(&mut app);
        let _ = app.update(UiMessage::Settings(Msg::SelectUiTheme(Some(UiTheme::MonochromeGreen))));
        for dark in [false, true] {
            app.dark = dark;
            let p = crate::style::palette(dark, Some(UiTheme::MonochromeGreen), 0);
            app.pal = p;
            let size = Size::new(900.0, 2400.0);
            let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.theme_colors(Some(UiTheme::MonochromeGreen)));
            let boxes: Vec<_> = std::iter::once(None)
                .chain(UiTheme::ALL.map(Some))
                .map(|t| (t, sim.find(iced::widget::Id::from(swatch_id(t))).expect("a swatch").bounds()))
                .collect();
            let theme = if dark { iced::Theme::Dark } else { iced::Theme::Light };
            let shot = crate::focus::tests::pixels(&mut sim, &theme, "theme-swatches");
            let k = shot.width() as f32 / size.width;
            let px = |x: f32, y: f32| {
                let [r, g, b, _] = shot.get_pixel((x * k) as u32, (y * k) as u32).0;
                Color::from_rgb8(r, g, b)
            };
            let near = |a: Color, b: Color| a.into_rgba8().iter().zip(b.into_rgba8()).all(|(x, y)| x.abs_diff(y) <= 2);
            for (t, b) in boxes {
                let (a, z) = match t.map(UiTheme::hue) {
                    None => (hsl(0.0, 0.0, 60.0), hsl(0.0, 0.0, 30.0)),
                    Some(Some(h)) => (hsl(f32::from(h), 60.0, 60.0), hsl(f32::from(h), 70.0, 30.0)),
                    Some(None) => rich_colors(t.unwrap(), dark),
                };
                let (cx, cy) = (b.x + 28.0, b.y + 28.0);
                let (tl, br) = (px(cx - 10.0, cy - 10.0), px(cx + 10.0, cy + 10.0));
                assert!(near(tl, a), "{t:?} dark={dark}: the top left half {tl:?} {a:?} in {b:?}");
                assert!(near(br, z), "{t:?} dark={dark}: the bottom right half {br:?} {z:?}");
                // The snapshot's own background, where nothing draws.
                let bg = px(1.0, 1.0);
                let corner = match t == Some(UiTheme::MonochromeGreen) {
                    true => crate::style::flat(bg, p.selected),
                    false => bg,
                };
                assert!(near(px(b.x + 2.0, b.y + 2.0), corner), "{t:?} dark={dark}: the square behind the circle");
            }
        }
    }

    /// Only a rich theme color subscribes to the clock, and its tick moves the gradient by the time.
    #[test]
    fn only_a_rich_theme_ticks() {
        let mut app = crate::fuzz::demo_app("theme-color-ticks");
        assert!(app.theme_ticks().is_none());
        let _ = app.update(UiMessage::Settings(Msg::SelectUiTheme(Some(UiTheme::MonochromeRed))));
        assert!(app.theme_ticks().is_none(), "a monochrome gradient stands still");
        let _ = app.update(UiMessage::Settings(Msg::SelectUiTheme(Some(UiTheme::RichRed))));
        assert!(app.theme_ticks().is_some());
        app.pal.backdrop.as_mut().unwrap().offset = -1.0;
        let _ = app.update(UiMessage::Settings(Msg::ThemeTick));
        let now = rich_offset(beeper_core::config::now_unix_millis());
        assert!((app.pal.backdrop.unwrap().offset - now).abs() < 0.1, "the tick reads the clock");
    }
}

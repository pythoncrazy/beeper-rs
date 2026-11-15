//! Desktop's `GalleryGrouper` and `GalleryAttachments`: the photos and videos that one sender sends in a short time
//! show as one gallery of up to four tiles.
use crate::style::{bordered_box, fill_box, semi, Palette};
use crate::{iso_millis, pending, tinted, BeeperGuiApp, El, UiMessage};
use beeper_core::models::{AttachmentSize, Message};
use beeper_core::store::BeeperStore;
use iced::widget::{container, image, mouse_area, stack, Space, Stack};
use iced::{mouse, Color, Fill, Padding, Pixels, Task};

/// Desktop's `rbt`, `obt` and `ibt`: at most 20 messages, at most 2 minutes after the first, at most 60 seconds apart.
const MAX: usize = 20;
const SPAN_MS: i64 = 120_000;
const GAP_MS: i64 = 60_000;
/// Desktop's `zJ`, `h4` and the tile radius.
const WIDTH: f32 = 280.0;
const GAP: f32 = 6.0;
const RADIUS: f32 = 14.0;
/// The scale of a baked tile, so that it stays sharp on a 2x screen.
const BAKE: f32 = 2.0;

/// Desktop's `lbt`: a message that can be in a gallery, with one photo or video and nothing else.
fn member(st: &BeeperStore, m: &Message, replies: usize) -> bool {
    let [a] = &m.attachments[..] else { return false };
    let mime = a.mime_type.as_deref().unwrap_or("").to_lowercase();
    let media = a.attachment_type == "img" || a.attachment_type == "video" || mime.starts_with("image/");
    media
        && a.is_gif != Some(true)
        && a.is_sticker != Some(true)
        && mime != "image/gif"
        && a.is_voice_note != Some(true)
        && m.is_deleted != Some(true)
        && !pending(st, m)
        && m.text.as_deref().is_none_or(|t| t.trim().is_empty())
        && m.linked_message_id.is_none()
        && m.unresolved_media.is_none()
        && m.links.is_empty()
        && m.poll.is_none()
        && replies == 0
}

/// Desktop's `ubt`: `m` joins the gallery that starts at `first`, after `prev`, with `len` messages.
fn joins(first: &Message, prev: &Message, m: &Message, len: usize) -> bool {
    let (t, t0, tp) = (iso_millis(&m.timestamp), iso_millis(&first.timestamp), iso_millis(&prev.timestamp));
    len < MAX && m.sender_id == first.sender_id && (0..=SPAN_MS).contains(&(t - t0)) && (t - tp).abs() <= GAP_MS
}

/// The index of the first message of the gallery of each message, or its own index. A gallery has two messages or
/// more, like Desktop's `minSize`. `replies` counts the replies to each message.
pub(crate) fn starts(st: &BeeperStore, msgs: &[&Message], replies: &[usize]) -> Vec<usize> {
    let mut starts: Vec<usize> = (0..msgs.len()).collect();
    for i in 1..msgs.len() {
        let s = starts[i - 1];
        if member(st, msgs[i - 1], replies[i - 1])
            && member(st, msgs[i], replies[i])
            && joins(msgs[s], msgs[i - 1], msgs[i], i - s)
        {
            starts[i] = s;
        }
    }
    starts
}

/// A tile: its left, top, width, height and turn in radians, clockwise.
type Tile = [f32; 5];

/// Desktop's `r4t`: the height of a `w` wide tile, a little lower for a wide image and a little higher for a tall one.
fn tile_height(size: Option<&AttachmentSize>, w: f32) -> f32 {
    let (x, y) = size.map_or((0, 0), |s| (s.width.unwrap_or(0), s.height.unwrap_or(0)));
    let (x, y) = (x as f32, y as f32);
    match () {
        _ if x <= 0.0 || y <= 0.0 => w,
        _ if x > y * 1.1 => w * 10.0 / 11.0,
        _ if y > x * 1.1 => w * 11.0 / 10.0,
        _ => w,
    }
}

/// Desktop's `o4t`: the width, the height and the tiles of a gallery at most `width` wide. Four or more items show
/// four tiles in a 2×2 grid, and two or three items show as a fan of turned tiles.
fn layout(sizes: &[Option<&AttachmentSize>], width: f32) -> (f32, f32, Vec<Tile>) {
    assert!(sizes.len() >= 2, "a gallery has two items or more");
    let n = width.clamp(0.0, WIDTH);
    let s = sizes.len().min(4);
    if s == 4 {
        let u = ((n - GAP) / 2.0).max(0.0);
        let tiles = (0..s).map(|i| [(i % 2) as f32 * (u + GAP), (i / 2) as f32 * (u + GAP), u, u, 0.0]);
        return (n, u * 2.0 + GAP, tiles.collect());
    }
    let two = s == 2;
    let r = (n * 0.86).min(400.0);
    let o = (n * if two { 0.68 } else { 0.51 }).min(r * 0.86);
    let tiles: Vec<Tile> = (sizes.iter().enumerate())
        .map(|(d, size)| {
            let h = tile_height(*size, o);
            let f = d as f32 - (s - 1) as f32 / 2.0;
            let turn = f * if two { 0.14 } else { 0.1 };
            let x = n / 2.0 + f * n * if two { 0.22 } else { 0.19 };
            let y = r * if two { 0.5 + f * 0.08 } else { 0.5 + f.abs() * 0.04 };
            [x - o / 2.0, y - h / 2.0, o, h, turn]
        })
        .collect();
    let half = |t: &Tile| ((t[2] * t[4].sin()).abs() + (t[3] * t[4].cos()).abs()) / 2.0;
    let top = tiles.iter().map(|t| t[1] + t[3] / 2.0 - half(t)).fold(f32::INFINITY, f32::min);
    let bottom = tiles.iter().map(|t| t[1] + t[3] / 2.0 + half(t)).fold(f32::NEG_INFINITY, f32::max);
    (n, bottom - top, tiles.into_iter().map(|[x, y, w, h, a]| [x, y - top, w, h, a]).collect())
}

/// The size of the box around a `w` by `h` tile turned by `turn`.
fn turned((w, h): (f32, f32), turn: f32) -> (f32, f32) {
    let (sin, cos) = (turn.sin().abs(), turn.cos().abs());
    (w * cos + h * sin, w * sin + h * cos)
}

/// A tile image: `src` cut to cover a `w` by `h` tile, at `BAKE` times its size, with the round corners and the
/// half pixel border of Desktop's `.tile`. iced can turn an image, but it cuts and rounds an image only along the
/// screen axes, so the tile has its corners in its pixels.
fn bake(src: &image::Handle, (w, h): (u32, u32), border: Color) -> image::Handle {
    let image::Handle::Rgba { width, height, pixels, .. } = src else { unreachable!("decoded media is RGBA") };
    let full = ::image::RgbaImage::from_raw(*width, *height, pixels.to_vec()).expect("RGBA pixels fill the image");
    let (tw, th) = ((w as f32 * BAKE) as u32, (h as f32 * BAKE) as u32);
    let k = (*width as f32 / tw as f32).min(*height as f32 / th as f32);
    let (cw, ch) = (((tw as f32 * k) as u32).max(1), ((th as f32 * k) as u32).max(1));
    let cut = ::image::imageops::crop_imm(&full, (width - cw) / 2, (height - ch) / 2, cw, ch).to_image();
    let mut out = ::image::imageops::resize(&cut, tw, th, ::image::imageops::FilterType::Triangle);
    let (r, half) = (RADIUS * BAKE, [tw as f32 / 2.0, th as f32 / 2.0]);
    let edge = border.into_rgba8();
    for (x, y, px) in out.enumerate_pixels_mut() {
        // The signed distance from the rounded edge, negative inside.
        let q = [(x as f32 + 0.5 - half[0]).abs() - half[0] + r, (y as f32 + 0.5 - half[1]).abs() - half[1] + r];
        let d = q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r;
        let ring = (0.5 + d + 0.5 * BAKE).clamp(0.0, 1.0) * f32::from(edge[3]) / 255.0;
        for c in 0..3 {
            px[c] = (f32::from(px[c]) * (1.0 - ring) + f32::from(edge[c]) * ring).round() as u8;
        }
        px[3] = (f32::from(px[3]) * (0.5 - d).clamp(0.0, 1.0)).round() as u8;
    }
    image::Handle::from_rgba(tw, th, out.into_raw())
}

/// The key of a baked tile in `BeeperGuiApp::tiles`.
fn key(url: &str, (w, h): (u32, u32), dark: bool) -> String {
    format!("{w}x{h}{} {url}", u8::from(dark))
}

/// Desktop's `.duration` and `.more` labels: white text on a dark pill.
fn label<'a>(text: String, size: f32, line: f32, pad: [f32; 2], radius: f32) -> El<'a> {
    let t = semi(text, size).line_height(Pixels(line)).color(Color::WHITE);
    container(t).padding(pad).style(fill_box(Color::from_rgba8(0, 0, 0, 0x8c as f32 / 255.0), radius)).into()
}

impl BeeperGuiApp {
    /// The baked tile of `url` once its media loaded, or `None` while a load or a bake runs.
    fn tile(&self, url: &str, size: (u32, u32)) -> Option<&image::Handle> {
        let Some(Ok(_)) = self.media_at(url) else { return None };
        match self.tiles.get(&key(url, size, self.dark)) {
            Some(t) => t.as_ref(),
            None => {
                self.wants.ask(&self.wants.tiles, &(url.to_string(), size.0, size.1));
                None
            }
        }
    }

    /// Bakes the tiles that a view asked for, on worker threads.
    pub(crate) fn bake_tiles(&mut self) -> Task<UiMessage> {
        let wanted = std::mem::take(&mut *self.wants.tiles.borrow_mut());
        let border = self.pal.prefs_border;
        let tasks: Vec<_> = (wanted.into_iter())
            .filter_map(|(url, w, h)| {
                let k = key(&url, (w, h), self.dark);
                // A view asks again for a tile whose media went out of the cache.
                let Some(Some(Ok(frames))) = self.media.get(&url) else { return None };
                let src = frames[0].0.clone();
                (!self.tiles.contains_key(&k)).then(|| {
                    let bake = move || bake(&src, (w, h), border);
                    let task = async move { tokio::task::spawn_blocking(bake).await.expect("tile bake task") };
                    (k, task)
                })
            })
            .collect();
        self.tiles.extend(tasks.iter().map(|(k, _)| (k.clone(), None)));
        Task::batch(tasks.into_iter().map(|(k, task)| Task::perform(task, move |h| UiMessage::TileBaked(k.clone(), h))))
    }

    /// Desktop's `GalleryAttachments` of the messages of a gallery: each tile opens its item in the carousel.
    pub(crate) fn view_gallery<'a>(&self, members: &[&Message]) -> El<'a> {
        let p: &Palette = &self.pal;
        let width = (if self.sidebar_hidden { 0.85 } else { 0.45 }) * self.pane().size.width;
        let sizes: Vec<_> = members.iter().map(|m| m.attachments[0].size.as_ref()).collect();
        let (gw, gh, tiles) = layout(&sizes, width);
        let hidden = members.len() - tiles.len();
        let layers = tiles.iter().zip(members).enumerate().map(|(i, (&[x, y, w, h, turn], m))| -> El<'a> {
            let a = &m.attachments[0];
            let video = a.attachment_type == "video";
            let url = a.poster_img.as_ref().or(a.src_url.as_ref());
            let (bw, bh) = turned((w, h), turn);
            let size = (w.round() as u32, h.round() as u32);
            let base: El<'a> = match url.and_then(|u| self.tile(u, size)) {
                Some(t) => image(t.clone())
                    .width(bw)
                    .height(bh)
                    .content_fit(iced::ContentFit::Contain)
                    .rotation(iced::Rotation::Solid(iced::Radians(turn)))
                    .into(),
                None => {
                    let fill = if video { Color::BLACK } else { p.chip };
                    let tile = container(Space::new()).width(w).height(h);
                    container(tile.style(bordered_box(fill, p.prefs_border, 0.5, RADIUS)))
                        .center(bw)
                        .center_y(bh)
                        .into()
                }
            };
            let play = video.then(|| container(tinted(&crate::voice::ICONS[0], 32.0, Color::WHITE)).center(Fill));
            let time = a.duration.filter(|_| video).map(|d| {
                let pill = label(crate::voice::clock(d as f32), 11.0, 14.0, [1.0, 4.0], 8.0);
                container(pill).align_bottom(Fill).padding(4)
            });
            let more = (hidden > 0 && i + 1 == tiles.len()).then(|| {
                let pill = label(format!("+{hidden}"), 12.0, 15.0, [4.0, 10.0], 100.0);
                container(pill).align_right(Fill).align_bottom(Fill).padding(6)
            });
            let marks = Stack::new().push(play).push(time).push(more).width(w).height(h);
            let marks = container(marks).padding(Padding::ZERO.left((bw - w) / 2.0).top((bh - h) / 2.0));
            let open = UiMessage::OpenCarousel(m.id.clone(), 0, false);
            let tile = mouse_area(stack![base, marks]).on_press(open).interaction(mouse::Interaction::Pointer);
            container(tile).padding(Padding::ZERO.left(x - (bw - w) / 2.0).top(y - (bh - h) / 2.0)).into()
        });
        Stack::with_children(layers).width(gw).height(gh).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> AttachmentSize {
        AttachmentSize { width: Some(width), height: Some(height) }
    }

    /// The numbers of Desktop's `o4t` for a 280 px gallery.
    #[test]
    fn gallery_layout_like_desktop() {
        let (w, h, grid) = layout(&[None; 7], 400.0);
        assert_eq!((w, h), (280.0, 280.0), "a grid is square at the 280 px maximum");
        assert_eq!(grid.len(), 4, "a grid shows four tiles");
        assert_eq!(grid[3], [143.0, 143.0, 137.0, 137.0, 0.0]);
        let (wide, tall) = (size(400, 300), size(300, 400));
        let (_, h, two) = layout(&[Some(&wide), Some(&tall)], 280.0);
        let o = 280.0 * 0.68;
        assert_eq!((two[0][2], two[0][3], two[1][3]), (o, o * 10.0 / 11.0, o * 11.0 / 10.0));
        assert_eq!((two[0][4], two[1][4]), (-0.07, 0.07), "two tiles turn 4° apart from each other");
        assert!((two[0][0] + o / 2.0 - (140.0 - 30.8)).abs() < 1e-3, "the first tile centers left of the middle");
        let (_, _, three) = layout(&[None; 3], 280.0);
        assert_eq!(three[1][4], 0.0, "the middle of three tiles stays straight");
        for t in two.iter().chain(&three) {
            let (bw, bh) = turned((t[2], t[3]), t[4]);
            let (x, y) = (t[0] - (bw - t[2]) / 2.0, t[1] - (bh - t[3]) / 2.0);
            assert!(x >= 0.0 && y >= -1e-3 && x + bw <= 280.0 && y + bh <= h + 1e-3, "{t:?} stays in the gallery");
        }
        assert_eq!(layout(&[None; 2], 200.0).0, 200.0, "a narrow chat makes the gallery narrower");
    }

    #[test]
    fn baked_tiles_have_round_corners() {
        let src = image::Handle::from_rgba(40, 20, vec![255; 40 * 20 * 4]);
        let image::Handle::Rgba { width, height, pixels, .. } = bake(&src, (30, 30), Color::BLACK) else {
            unreachable!()
        };
        assert_eq!((width, height), (60, 60), "a tile bakes at twice its size");
        let at = |x: u32, y: u32| &pixels[((y * width + x) * 4) as usize..][..4];
        assert_eq!(at(0, 0)[3], 0, "a corner is clear");
        assert_eq!(at(30, 30), [255; 4], "the middle keeps the image");
        assert_eq!(at(30, 0), [0, 0, 0, 255], "the edge has the border");
    }
}

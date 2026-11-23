//! Desktop's `GalleryGrouper` and `GalleryAttachments`: the photos and videos that one sender sends in a short time
//! show as one gallery of up to four tiles.
use crate::forward;
use crate::i18n::{tr, trx};
use crate::loads;
use crate::media_view;
use crate::message;
use crate::style::rem;
use crate::style::{bordered_box, btn_style, fill_box, semi, Palette};
use crate::time::iso_millis;
use crate::{deletable, menu_item, menu_rule, pending, tinted, BeeperGuiApp, El, Frames, UiMessage};
use crate::{hotkey_text, menu_pitch, MESSAGE_MENU_W};
use beeper_core::models::{AttachmentSize, Message};
use beeper_core::store::BeeperStore;
use iced::widget::{button, container, image, mouse_area, stack, Space, Stack};
use iced::{mouse, Color, Fill, Padding, Pixels, Point, Task};

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

/// How a baked tile looks: a gallery tile with its ring (selected or not), or an inline image cropped like Desktop's
/// `object-fit: cover` in the 17 px corners of a bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Look {
    Tile(bool),
    Inline,
    /// A square thumbnail with this corner radius, as in Chat Info and the Media window.
    Thumb(u8),
}

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
        && m.view_limit.is_none()
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

/// The selection and the menu of a gallery, like Desktop's `MessageComponent` of a `GalleryGrouper`.
#[derive(Clone)]
pub enum Msg {
    /// A click on a gallery beside its tiles, with the messages of the gallery.
    Click(Vec<String>),
    /// A click on the tile of a message.
    Tile(String),
    /// A right click on a gallery, with the messages of the gallery.
    Menu(Vec<String>),
    /// The "Reactions" item of the gallery menu at a point.
    Reactions(Vec<String>, Point),
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

/// A tile image: `src` cut to cover a `w` by `h` tile, at `scale` times its size, with the round corners and the
/// `ring` of Desktop's `.tile`: its half pixel border, or the 2 px outline of a selected tile. iced can turn an image,
/// but it cuts and rounds an image only along the screen axes, so the tile has its corners in its pixels.
fn bake(
    src: &image::Handle,
    (w, h): (u32, u32),
    (border, thick): (Color, f32),
    radius: f32,
    scale: f32,
) -> image::Handle {
    let image::Handle::Rgba { width, height, pixels, .. } = src else { unreachable!("decoded media is RGBA") };
    let full = ::image::RgbaImage::from_raw(*width, *height, pixels.to_vec()).expect("RGBA pixels fill the image");
    let (tw, th) = (((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1));
    let k = (*width as f32 / tw as f32).min(*height as f32 / th as f32);
    let (cw, ch) = (((tw as f32 * k) as u32).max(1), ((th as f32 * k) as u32).max(1));
    let cut = ::image::imageops::crop_imm(&full, (width - cw) / 2, (height - ch) / 2, cw, ch).to_image();
    let mut out = ::image::imageops::resize(&cut, tw, th, ::image::imageops::FilterType::Triangle);
    let (r, half) = (radius * scale, [tw as f32 / 2.0, th as f32 / 2.0]);
    let edge = border.into_rgba8();
    for (x, y, px) in out.enumerate_pixels_mut() {
        // The signed distance from the rounded edge, negative inside.
        let q = [(x as f32 + 0.5 - half[0]).abs() - half[0] + r, (y as f32 + 0.5 - half[1]).abs() - half[1] + r];
        let d = q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r;
        let ring = (0.5 + d + thick * scale).clamp(0.0, 1.0) * f32::from(edge[3]) / 255.0;
        for c in 0..3 {
            px[c] = (f32::from(px[c]) * (1.0 - ring) + f32::from(edge[c]) * ring).round() as u8;
        }
        px[3] = (f32::from(px[3]) * (0.5 - d).clamp(0.0, 1.0)).round() as u8;
    }
    image::Handle::from_rgba(tw, th, out.into_raw())
}

/// The frames of an animation cut like `bake` cuts an inline image, at `BAKE` times the size of the box or the size
/// of the frames if they are smaller, so that the cut takes no more memory than the frames.
pub(crate) fn cut_frames(frames: &Frames, (w, h): (u32, u32)) -> Frames {
    let (fw, fh) = match &frames[0].0 {
        image::Handle::Rgba { width, height, .. } => (*width as f32, *height as f32),
        _ => unreachable!("decoded media is RGBA"),
    };
    let scale = BAKE.min(fw / w.max(1) as f32).min(fh / h.max(1) as f32).max(0.1);
    frames.iter().map(|(f, ms)| (bake(f, (w, h), (Color::TRANSPARENT, 0.0), 17.0, scale), *ms)).collect()
}

/// The frames of an animation at `k` times their brightness, like CSS `filter: brightness(k)`.
pub(crate) fn dim_frames(frames: &Frames, k: f32) -> Frames {
    let dim = |h: &image::Handle| match h {
        image::Handle::Rgba { width, height, pixels, .. } => {
            let c = |c: u8| (f32::from(c) * k).round() as u8;
            let px: Vec<u8> =
                pixels.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [c(r), c(g), c(b), a]).collect();
            image::Handle::from_rgba(*width, *height, px)
        }
        _ => unreachable!("decoded media is RGBA"),
    };
    frames.iter().map(|(h, ms)| (dim(h), *ms)).collect()
}

/// How a worker changes the frames of an animation: cut to cover an inline box of a width and a height, or at 80%
/// brightness like a selected emoji message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Cut {
    Cover(u32, u32),
    Dim,
}

/// The key of the cut frames of an animation in `BeeperGuiApp::cuts`.
fn cut_key(url: &str, cut: Cut) -> String {
    format!("{cut:?} {url}")
}

/// The key of a baked tile in `BeeperGuiApp::tiles`.
fn key(url: &str, (w, h): (u32, u32), dark: bool, look: Look) -> String {
    format!("{w}x{h}{}{look:?} {url}", u8::from(dark))
}

/// Desktop's `.duration` and `.more` labels: white text on a dark pill.
fn label<'a>(text: String, size: f32, line: f32, pad: [f32; 2], radius: f32) -> El<'a> {
    let t = semi(text, size).line_height(Pixels(rem(line))).color(Color::WHITE);
    container(t).padding(pad).style(fill_box(Color::from_rgba8(0, 0, 0, 0x8c as f32 / 255.0), radius)).into()
}

impl BeeperGuiApp {
    pub(crate) fn gallery(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            // Shift selects up to the last message of the gallery. Ctrl adds the gallery to the selection, and a click
            // selects only the gallery. Both unselect a selected gallery.
            Msg::Click(ids) => {
                let task = match self.mods.shift() {
                    true => self.update(UiMessage::Message(message::Msg::Click(
                        ids.last().expect("a gallery has messages").clone(),
                    ))),
                    false => {
                        let sel = &mut self.pane.selected_msgs;
                        match (ids.iter().all(|i| sel.contains(i)), self.mods.command()) {
                            (true, _) => sel.retain(|m| !ids.contains(m)),
                            (false, true) => {
                                let more: Vec<String> = ids.iter().filter(|i| !sel.contains(i)).cloned().collect();
                                sel.extend(more);
                            }
                            (false, false) => *sel = ids.clone(),
                        }
                        Task::none()
                    }
                };
                self.pane.msg_anchor = Some(ids[0].clone());
                return task;
            }
            // Ctrl and Shift select tiles like messages, and a click opens the carousel.
            Msg::Tile(mid) => {
                return match self.mods.command() || self.mods.shift() {
                    true => self.update(UiMessage::Message(message::Msg::Click(mid))),
                    false => self.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, 0, false))),
                };
            }
            // A right click selects only the gallery, unless all of it is selected.
            Msg::Menu(ids) => {
                if !ids.iter().all(|i| self.pane.selected_msgs.contains(i)) {
                    self.pane.selected_msgs = ids.clone();
                }
                let task = self.update(UiMessage::Message(message::Msg::OpenMenu(ids[0].clone())));
                self.pane.gallery_menu = ids;
                return task;
            }
            Msg::Reactions(ids, at) => self.pane.gallery_reactions = Some((ids, at)),
        }
        Task::none()
    }

    /// Desktop's gallery menu when only the whole gallery of message `mid` is selected: "Reactions", "Download All",
    /// "Forward All…" and "Delete…", as the menu, its height and the top of its voice rows.
    pub(crate) fn view_gallery_menu(&self, mid: &str) -> Option<(El<'_>, f32, f32)> {
        let (ids, sel) = (&self.pane().gallery_menu, &self.pane().selected_msgs);
        if ids.first().is_none_or(|first| first != mid)
            || sel.len() != ids.len()
            || !ids.iter().all(|i| sel.contains(i))
        {
            return None;
        }
        let p = &self.pal;
        let st = self.store.read();
        let ms: Vec<&Message> = ids.iter().map(|i| st.message(i)).collect();
        let at = self.pane().message_menu.as_ref().expect("the menu is open").1;
        let key = |name| hotkey_text(st.settings.hotkey_keys(name));
        let item = |label: String, hotkey: &str, msg| {
            (
                menu_item(
                    p,
                    None,
                    label,
                    hotkey.to_string(),
                    Some(UiMessage::Message(message::Msg::Menu(Box::new(msg)))),
                ),
                menu_pitch(),
            )
        };
        let action = |s| trx(s, "gallery menu action").to_string();
        let download = UiMessage::Commands(
            ids.iter().map(|i| UiMessage::Message(message::Msg::Download(i.clone(), None))).collect(),
        );
        let rows: Vec<(El, f32)> = [
            ms.iter()
                .any(|m| !m.reactions.is_empty())
                .then(|| vec![item(action("Reactions"), "", UiMessage::Gallery(Msg::Reactions(ids.clone(), at)))]),
            Some(vec![
                item(action("Download All"), &key("Download Attachments"), download),
                item(
                    action("Forward All…"),
                    &key("Message: Forward"),
                    UiMessage::Forward(forward::Msg::Open(ids.clone())),
                ),
            ]),
            ms.iter().all(|m| deletable(&st, m)).then(|| {
                let delete = UiMessage::Message(message::Msg::AskDelete(ids.clone()));
                vec![(menu_rule(p), 11.0), item(tr("Delete…").into(), "", delete)]
            }),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let (menu, h) = crate::menu_rows(p, rows, MESSAGE_MENU_W);
        Some((menu.into(), h, 0.0))
    }

    /// Desktop's reactions list of the messages of a gallery, at the point of its anchor.
    pub(crate) fn view_gallery_reactions<'a>(&self, ids: &[String]) -> El<'a> {
        let st = self.store.read();
        let ms: Vec<&Message> = ids.iter().map(|i| st.message(i)).collect();
        let chat = st.chat(&ms[0].chat_id);
        crate::popover::popover(Space::new(), self.reaction_list(chat, &ms), true).into()
    }

    /// The baked tile of `url` once its media loaded, or `None` while a load or a bake runs. While the selected or
    /// the unselected tile bakes after a click, the other one shows.
    fn tile(&self, url: &str, size: (u32, u32), selected: bool) -> Option<&image::Handle> {
        let tile = |selected| self.baked(url, size, Look::Tile(selected));
        tile(selected).or_else(|| self.tiles.get(&key(url, size, self.dark, Look::Tile(!selected)))?.as_ref())
    }

    /// An inline image cut to `size` like Desktop's `object-fit: cover`, once baked.
    pub(crate) fn cover_tile(&self, url: &str, size: (u32, u32)) -> Option<&image::Handle> {
        self.baked(url, size, Look::Inline)
    }

    /// A square thumbnail cut from the media, with rounded corners: the renderer draws a `Cover` image past its box.
    pub(crate) fn thumb_tile(&self, url: &str, side: u32, radius: u8) -> Option<&image::Handle> {
        self.baked(url, (side, side), Look::Thumb(radius))
    }

    /// The baked tile, or `None` while it bakes: a view asks for it.
    fn baked(&self, url: &str, size: (u32, u32), look: Look) -> Option<&image::Handle> {
        let Some(Ok(_)) = self.media_at(url) else { return None };
        let baked = self.tiles.get(&key(url, size, self.dark, look));
        if baked.is_none() {
            self.wants.ask(&self.wants.tiles, &(url.to_string(), size.0, size.1, look));
        }
        baked.and_then(Option::as_ref)
    }

    /// The frames of an animated inline image cut to `size` like Desktop's `object-fit: cover`, with the corners of a
    /// bubble, once a worker cut them: a view asks for them.
    pub(crate) fn cover_frames(&self, url: &str, size: (u32, u32)) -> Option<&Frames> {
        self.cut(url, Cut::Cover(size.0, size.1))
    }

    /// The frames of an animated emoji at 80% brightness, once a worker dimmed them: a view asks for them.
    pub(crate) fn dimmed_frames(&self, url: &str) -> Option<&Frames> {
        self.cut(url, Cut::Dim)
    }

    fn cut(&self, url: &str, how: Cut) -> Option<&Frames> {
        let Some(Ok(_)) = self.media_at(url) else { return None };
        let cut = self.cuts.get(&cut_key(url, how));
        if cut.is_none() {
            self.wants.ask(&self.wants.cuts, &(url.to_string(), how));
        }
        cut.and_then(Option::as_ref)
    }

    /// Cuts or dims the frames of the animations that a view asked for, on worker threads.
    pub(crate) fn cut_animations(&mut self) -> Task<UiMessage> {
        let wanted = std::mem::take(&mut *self.wants.cuts.borrow_mut());
        let tasks: Vec<_> = (wanted.into_iter())
            .filter_map(|(url, how)| {
                let k = cut_key(&url, how);
                // A view asks again for a cut whose media went out of the cache.
                let Some(Some(Ok(frames))) = self.media.get(&url) else { return None };
                let frames = frames.clone();
                (!self.cuts.contains_key(&k)).then(|| {
                    let cut = move || match how {
                        Cut::Cover(w, h) => cut_frames(&frames, (w, h)),
                        Cut::Dim => dim_frames(&frames, 0.8),
                    };
                    (k, async move { tokio::task::spawn_blocking(cut).await.expect("animation cut task") })
                })
            })
            .collect();
        self.cuts.extend(tasks.iter().map(|(k, _)| (k.clone(), None)));
        Task::batch(
            tasks
                .into_iter()
                .map(|(k, task)| Task::perform(task, move |f| UiMessage::Loaded(loads::Msg::Cut(k.clone(), f)))),
        )
    }

    /// Bakes the tiles that a view asked for, on worker threads.
    pub(crate) fn bake_tiles(&mut self) -> Task<UiMessage> {
        let wanted = std::mem::take(&mut *self.wants.tiles.borrow_mut());
        let (border, outline) = (self.pal.prefs_border, self.pal.accent);
        let tasks: Vec<_> = (wanted.into_iter())
            .filter_map(|(url, w, h, look)| {
                let k = key(&url, (w, h), self.dark, look);
                let (ring, radius) = match look {
                    Look::Tile(true) => ((outline, 2.0), RADIUS),
                    Look::Tile(false) => ((border, 0.5), RADIUS),
                    Look::Inline => ((Color::TRANSPARENT, 0.0), 17.0),
                    Look::Thumb(r) => ((Color::TRANSPARENT, 0.0), f32::from(r)),
                };
                // A view asks again for a tile whose media went out of the cache.
                let Some(Some(Ok(frames))) = self.media.get(&url) else { return None };
                let src = frames[0].0.clone();
                (!self.tiles.contains_key(&k)).then(|| {
                    let bake = move || bake(&src, (w, h), ring, radius, BAKE);
                    let task = async move { tokio::task::spawn_blocking(bake).await.expect("tile bake task") };
                    (k, task)
                })
            })
            .collect();
        self.tiles.extend(tasks.iter().map(|(k, _)| (k.clone(), None)));
        Task::batch(
            tasks
                .into_iter()
                .map(|(k, task)| Task::perform(task, move |h| UiMessage::Loaded(loads::Msg::Tile(k.clone(), h)))),
        )
    }

    /// Desktop's `GalleryAttachments` of the messages of a gallery: each tile opens its item in the carousel, and a
    /// selected message has an outline on its tile.
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
            let selected = self.pane().selected_msgs.contains(&m.id);
            let more = (hidden > 0 && i + 1 == tiles.len()).then(|| {
                let pill = label(format!("+{hidden}"), 12.0, 15.0, [4.0, 10.0], 100.0);
                container(pill).align_right(Fill).align_bottom(Fill).padding(6)
            });
            let at = |tile: El<'a>| -> El<'a> {
                container(tile).padding(Padding::ZERO.left(x - (bw - w) / 2.0).top(y - (bh - h) / 2.0)).into()
            };
            // Desktop's placeholder of media that a bridge loads only when the user asks: a round download button in
            // the brand color, with a spinner while it downloads, when it is disabled but keeps its colors.
            if let Some(src) = self.needs_download(a) {
                let fetching = self.fetching.contains(src);
                let icon = match fetching {
                    true => crate::verify::spinner(16.0, p.accent),
                    false => tinted(&crate::MAT_DOWNLOAD, 20.0, p.accent),
                };
                let fetch = (!fetching).then(|| UiMessage::Loaded(loads::Msg::Fetch(src.clone())));
                let download = button(container(icon).center(40)).padding(0).on_press_maybe(fetch);
                let style = btn_style([p.received; 3], [p.accent; 3], 20.0);
                let download = download.style(move |t, s| match s {
                    button::Status::Disabled => style(t, button::Status::Active),
                    s => style(t, s),
                });
                let tile = stack![container(download).center(Fill)].push(more).width(w).height(h);
                let tile = container(tile).style(bordered_box(p.chip, p.prefs_border, 0.5, RADIUS));
                return at(container(tile).center_x(bw).center_y(bh).into());
            }
            let base: El<'a> = match url.and_then(|u| self.tile(u, size, selected)) {
                Some(t) => image(t.clone())
                    .width(bw)
                    .height(bh)
                    .content_fit(iced::ContentFit::Contain)
                    .rotation(iced::Rotation::Solid(iced::Radians(turn)))
                    .into(),
                // Desktop's `MediaImg` of the tile is the `ImageError` of a media that failed to load.
                None if !video && matches!(url.and_then(|u| self.media.get(u)), Some(Some(Err(_)))) => {
                    let error = container(self.image_error(w, h, RADIUS));
                    container(error.style(bordered_box(p.chip, p.prefs_border, 0.5, RADIUS)))
                        .center(bw)
                        .center_y(bh)
                        .into()
                }
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
            let marks = Stack::new().push(play).push(time).push(more).width(w).height(h);
            let marks = container(marks).padding(Padding::ZERO.left((bw - w) / 2.0).top((bh - h) / 2.0));
            let click = UiMessage::Gallery(Msg::Tile(m.id.clone()));
            at(mouse_area(stack![base, marks]).on_press(click).interaction(mouse::Interaction::Pointer).into())
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
        let image::Handle::Rgba { width, height, pixels, .. } = bake(&src, (30, 30), (Color::BLACK, 0.5), RADIUS, BAKE)
        else {
            unreachable!()
        };
        assert_eq!((width, height), (60, 60), "a tile bakes at twice its size");
        let at = |x: u32, y: u32| &pixels[((y * width + x) * 4) as usize..][..4];
        assert_eq!(at(0, 0)[3], 0, "a corner is clear");
        assert_eq!(at(30, 30), [255; 4], "the middle keeps the image");
        assert_eq!(at(30, 0), [0, 0, 0, 255], "the edge has the border");
        assert_eq!(at(30, 2), [255; 4], "the half pixel border is thin");
        let image::Handle::Rgba { pixels, .. } = bake(&src, (30, 30), (Color::BLACK, 2.0), RADIUS, BAKE) else {
            unreachable!()
        };
        assert_eq!(
            pixels[((2 * width + 30) * 4) as usize..][..4],
            [0, 0, 0, 255],
            "a selected tile has a 2 px outline"
        );
    }
}

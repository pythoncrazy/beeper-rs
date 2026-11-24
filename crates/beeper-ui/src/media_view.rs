//! The Media window, the carousel of a chat, and the decode and the load of the media that they show.

use crate::composer;
use crate::i18n::{tr, trn, trx};
use crate::message;
use crate::prelude::*;
use crate::windows;
use crate::{
    anim, closable_head, cubic_bezier, file_kind, i18n, pdf, relative_date, MEDIA_ICONS, ROW_ACTIONS, SYS_ARROW_BACK, SYS_DELETE,
    SYS_DOWNLOAD, SYS_EMOJI, SYS_REPLY, SYS_ROTATE,
};
use beeper_core::models::*;
use iced::keyboard::{self, key::Named};
use iced::widget::column;
use iced::{border, mouse, window};

/// ffmpeg over `bytes`, which it reads from a temporary file of its own: an MP4 or M4A with its `moov` atom at the
/// end, as many encoders write them, needs an input that ffmpeg can seek, which a pipe is not.
pub(crate) fn run_ffmpeg(bytes: &[u8], args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("beeper-rs-ffmpeg-{}-{n}", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
    let out = file.write_all(bytes).map_err(|e| e.to_string()).and_then(|()| {
        std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-i"])
            .arg(&path)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("Could not start ffmpeg: {e}"))
    });
    let _ = std::fs::remove_file(&path);
    let out = out?;
    match out.status.success() && !out.stdout.is_empty() {
        true => Ok(out.stdout),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// The suffix of the media key of a video that plays as a GIF: Desktop's `<video autoPlay loop muted>` of an
/// attachment with `isGif`, which the bridges send for the GIFs of WhatsApp, Telegram and others.
pub(crate) const LOOP: &str = "#loop";

/// The frames of a video that plays as a GIF, at most 15 per second, 320 px on the long side and 8 seconds, so that it
/// takes no more memory than a GIF of its size.
pub(crate) fn decode_loop(bytes: &[u8]) -> Result<Frames, String> {
    const FPS: u128 = 15;
    let scale = format!("fps={FPS},scale='if(gt(iw,ih),min(320,iw),-2)':'if(gt(iw,ih),-2,min(320,ih))'");
    let out = run_ffmpeg(bytes, &["-t", "8", "-an", "-vf", &scale, "-f", "image2pipe", "-vcodec", "png", "pipe:1"])?;
    // ffmpeg writes the PNGs back to back, and each ends with its IEND chunk and its CRC.
    const END: &[u8] = b"IEND\xaeB`\x82";
    let mut frames = Vec::new();
    let mut rest = &out[..];
    while let Some(at) = rest.windows(END.len()).position(|w| w == END) {
        let (png, next) = rest.split_at(at + END.len());
        let px = ::image::load_from_memory(png).map_err(|e| format!("Could not decode a frame: {e}"))?.into_rgba8();
        frames.push((image::Handle::from_rgba(px.width(), px.height(), px.into_raw()), 1000 / FPS));
        rest = next;
    }
    match frames.is_empty() {
        true => Err("The video has no frames".into()),
        false => Ok(frames.into()),
    }
}

/// The handle of the pixels of a decoded image.
pub(crate) fn handle(px: ::image::RgbaImage) -> image::Handle {
    image::Handle::from_rgba(px.width(), px.height(), px.into_raw())
}

pub(crate) fn decode_media(bytes: Vec<u8>) -> Result<Frames, String> {
    // A decode here, on a worker thread, keeps the full size decode out of the first frame that draws the image.
    let still = |bytes: &[u8]| -> Result<Frames, String> {
        let px = beeper_core::media::decode_upright(bytes).or_else(|e| {
            // Formats the image crate lacks and Chromium shows, like AVIF, through ffmpeg.
            let png = run_ffmpeg(bytes, &["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"]).map_err(|_| e)?;
            beeper_core::media::decode_upright(&png)
        })?;
        Ok(Frames::from([(handle(px.into_rgba8()), 0)]))
    };
    // Desktop's `transcodeImage` of a HEIC or HEIF photo, which ffmpeg cannot read: its own HEIF decoder, then ffmpeg
    // for an AVIF in a HEIF box.
    if beeper_core::media::heif(&bytes) {
        return still(&bytes);
    }
    if bytes.get(4..8) == Some(b"ftyp") {
        return still(&run_ffmpeg(&bytes, &["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])?);
    }
    // Desktop's `Lottie` of an animated sticker: a TGS, or Lottie JSON.
    if let Some(a) = crate::lottie::sniff(&bytes) {
        let a = a?;
        let still = a.render(0, a.fit(crate::lottie::STILL));
        return Ok(Frames::lottie(still, a));
    }
    match animation(&bytes) {
        None => still(&bytes),
        Some(frames) => Ok(frames?.into_iter().map(|(px, ms)| (handle(px), ms)).collect()),
    }
}

/// The kind of animation that `bytes` start with, like Desktop's `RAe` sniff: a GIF, an APNG with its `acTL` chunk
/// before its image data, or a WebP with the animation flag of its `VP8X` header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Animated {
    Gif,
    Png,
    WebP,
}

pub(crate) fn animated(bytes: &[u8]) -> Option<Animated> {
    if bytes.starts_with(b"GIF8") {
        return Some(Animated::Gif);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut at = 8;
        while let Some(head) = bytes.get(at..at + 8) {
            match &head[4..] {
                b"acTL" => return Some(Animated::Png),
                b"IDAT" => return None,
                _ => at += 12 + u32::from_be_bytes(head[..4].try_into().expect("4 bytes")) as usize,
            }
        }
        return None;
    }
    let webp = bytes.len() >= 21 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    (webp && &bytes[12..16] == b"VP8X" && bytes[20] & 2 != 0).then_some(Animated::WebP)
}

/// The frames of an animated GIF, PNG or WebP, whole and in order, with their delays in milliseconds, or `None`
/// for any other image. A GIF of one frame has one frame.
pub(crate) fn animation(bytes: &[u8]) -> Option<Result<Vec<(::image::RgbaImage, u128)>, String>> {
    use ::image::AnimationDecoder;
    let kind = animated(bytes)?;
    let r = std::io::Cursor::new(bytes);
    let frames = match kind {
        Animated::Gif => ::image::codecs::gif::GifDecoder::new(r).map(|d| d.into_frames().collect_frames()),
        Animated::Png => ::image::codecs::png::PngDecoder::new(r).and_then(|d| d.apng()).map(|d| d.into_frames().collect_frames()),
        Animated::WebP => ::image::codecs::webp::WebPDecoder::new(r).map(|d| d.into_frames().collect_frames()),
    };
    let frames = frames.and_then(|f| f).map_err(|e| format!("Could not decode the animation: {e}"));
    let frame = |f: ::image::Frame| {
        let (n, d) = f.delay().numer_denom_ms();
        // Browsers play delays of 10 ms or less at 100 ms.
        let ms = match n / d.max(1) {
            0..=10 => 100,
            ms => ms,
        };
        (f.into_buffer(), u128::from(ms))
    };
    Some(frames.and_then(|f| match f.is_empty() {
        true => Err("The animation has no frames".into()),
        false => Ok(f.into_iter().map(frame).collect()),
    }))
}

/// The attachment that a carousel shows, its quarter turns and when the last one started, if it shows the Media window items, the inline size of
/// the video that opened it, which plays again when the carousel closes on it, where its menu is open, the first
/// item of the filmstrip after a scroll, and the page that a PDF shows, from 1.
#[derive(Clone)]
pub(crate) struct Carousel {
    pub(crate) mid: String,
    pub(crate) ai: usize,
    pub(crate) turns: u8,
    pub(crate) turned: Option<std::time::Instant>,
    pub(crate) in_media: bool,
    pub(crate) resume: Option<(u32, u32)>,
    pub(crate) menu: Option<Point>,
    pub(crate) strip: Option<usize>,
    pub(crate) page: usize,
}

/// Desktop's Media window: its tab (Media, Links or Docs), its size, its filters (sent by all, you or others; oldest
/// first; the text of the caption search field), and if its filter menu is open.
pub(crate) struct MediaWindow {
    pub(crate) id: window::Id,
    pub(crate) tab: usize,
    pub(crate) size: Size,
    pub(crate) sent_by: u8,
    pub(crate) oldest: bool,
    pub(crate) search: Option<String>,
    /// The caption search of the items: the trimmed text of the search field 300 ms after its last change, like
    /// Desktop's `setSearchText`.
    pub(crate) query: String,
    /// The count of the changes of the search field, so that only the last one searches.
    pub(crate) typed: u64,
    /// The loaded items in their order, as (message, attachment or link index), like Desktop's `items`.
    pub(crate) items: Vec<(Arc<Message>, usize)>,
    /// Whether more items can load, and the time and row of the message past which they load.
    pub(crate) more: bool,
    pub(crate) cursor: Option<(i64, i64)>,
    /// When the running load of items started. The skeleton tiles pulse from then.
    pub(crate) loading: Option<std::time::Instant>,
    /// The count of the filter changes, so that a load for older filters does nothing, like Desktop's `generation`.
    pub(crate) generation: u64,
    /// Desktop's `advanceViewerAfterLoad`: the viewer steps to the next item when the running load ends.
    pub(crate) advance: bool,
    /// The tab before the last tab change, and when the change was.
    pub(crate) tab_at: Option<(usize, std::time::Instant)>,
    /// Where the filter menu is open.
    pub(crate) menu: Option<Point>,
    /// Desktop's `carouselState` of the Media window: its viewer.
    pub(crate) viewer: Option<Carousel>,
    /// The item whose context menu is open, as (message, attachment or link index), and where.
    pub(crate) item_menu: Option<(String, usize, Point)>,
    pub(crate) select: bool,
    pub(crate) picked: Vec<(String, usize)>,
    /// The (chat, message) pairs that the delete dialog asks about, and how many items it skips.
    pub(crate) deleting: Option<(Vec<(String, String)>, usize)>,
}

impl MediaWindow {
    /// A Media window of `size` on the newest images and videos, before its first load.
    fn new(id: window::Id, size: Size) -> Self {
        beeper_core::with_defaults!(Self {
            id,
            size,
            more: true,
        }, default: tab, sent_by, oldest, search, query, typed, items, cursor, loading, generation, advance, tab_at,
            menu, viewer, item_menu, select, picked, deleting)
    }

    /// The loaded message `mid`.
    fn message(&self, mid: &str) -> Option<&Arc<Message>> {
        self.items.iter().find(|(m, _)| m.id == mid).map(|(m, _)| m)
    }

    /// Whether the item `k` of the message `mid` is picked.
    fn is_picked(&self, mid: &str, k: usize) -> bool {
        self.picked.iter().any(|(m, i)| m == mid && *i == k)
    }
}

/// The time of the slide of Desktop's `BrandSegmentControl` highlight, in seconds.
pub(crate) const TAB_SLIDE: f32 = 0.25;

/// The left edge and the width of a `BrandSegmentControl` highlight at the part `t` of its slide between two
/// segments of `widths`, 4 px apart, with the `cubic-bezier(.4, 0, .2, 1)` curve.
pub(crate) fn slide(widths: &[f32], (from, to): (usize, usize), t: f32) -> (f32, f32) {
    let left = |i: usize| widths[..i].iter().sum::<f32>() + 4.0 * i as f32;
    let k = cubic_bezier(t.min(1.0), [0.4, 0.0, 0.2, 1.0]);
    (left(from) + (left(to) - left(from)) * k, widths[from] + (widths[to] - widths[from]) * k)
}

/// Desktop's `limit` of a read of the Media window items, in messages.
const MEDIA_PAGE: usize = 50;
/// The most reads of a load of Desktop's Media window without an item.
const MEDIA_READS: usize = 5;

const MEDIA_TABS: [&str; 3] = ["Media", "Links", "Docs"];

/// The filters of a load of the Media window items, the time and row that it loads past, and the filter count.
pub(crate) struct MediaAsk {
    tab: usize,
    sent_by: u8,
    oldest: bool,
    query: String,
    cursor: Option<(i64, i64)>,
    generation: u64,
}

impl MediaAsk {
    pub(crate) fn of(w: &MediaWindow) -> Self {
        let (tab, sent_by, oldest, query) = (w.tab, w.sent_by, w.oldest, w.query.clone());
        Self { tab, sent_by, oldest, query, cursor: w.cursor, generation: w.generation }
    }
}

/// The items of a load of the Media window, whether more can load, and the time and row past which they load.
#[derive(Clone)]
pub struct MediaPage {
    generation: u64,
    items: Vec<(Arc<Message>, usize)>,
    more: bool,
    cursor: Option<(i64, i64)>,
}

/// Desktop's Media window `loadMore` over its `searchRoomMessages`: reads of 50 messages past the cursor, newest or
/// oldest first by time and row, until one has items of the tab, at most 5. The caption search is the message search
/// with partial matches. Like Desktop, a read skips deleted, hidden and view-once messages and low priority chats,
/// and the load then drops the items of message requests.
pub(crate) fn media_page(st: &BeeperStore, ask: &MediaAsk) -> MediaPage {
    let kinds: &[&str] = match ask.tab {
        0 => &["img", "video"],
        1 => &["link"],
        _ => &["unknown"],
    };
    let (mut cursor, mut more, mut items) = (ask.cursor, true, Vec::new());
    for _ in 0..MEDIA_READS {
        let mut q = Query::default().has(Some(kinds));
        if ask.sent_by > 0 {
            q = q.sent(ask.sent_by == 1);
        }
        if !ask.query.is_empty() {
            q = q.text(&ask.query);
        }
        if let Some(at) = cursor {
            q = q.past_row(at, ask.oldest);
        }
        let mut rows = Vec::new();
        st.scan_by_time(&q, ask.oldest, |n, m| {
            if m.is_deleted != Some(true)
                && !matches!(m.message_type.as_deref(), Some("HIDDEN" | "REACTION"))
                && m.view_limit.is_none()
                && st.chat(&m.chat_id).flags & CHAT_LOW_PRIORITY == 0
            {
                rows.push((n, m));
            }
            rows.len() <= MEDIA_PAGE
        });
        more = rows.len() > MEDIA_PAGE;
        rows.truncate(MEDIA_PAGE);
        cursor = rows.last().map(|(n, m)| (iso_millis(&m.timestamp), *n));
        // Desktop's `getItemsFromMessage`: the images and videos, the links or the files of the message.
        for (_, m) in rows {
            let parts: Vec<usize> = match ask.tab {
                1 => (0..m.links.len()).collect(),
                _ => {
                    let atts = m.attachments.iter().enumerate();
                    atts.filter(|(_, a)| kinds.contains(&a.attachment_type.as_str())).map(|(k, _)| k).collect()
                }
            };
            let m = Arc::new(m);
            items.extend(parts.into_iter().map(|k| (m.clone(), k)));
        }
        if !more || cursor.is_none() || !items.is_empty() {
            break;
        }
    }
    items.retain(|(m, _)| st.chat(&m.chat_id).flags & CHAT_REQUEST == 0);
    MediaPage { generation: ask.generation, items, more, cursor }
}

/// Runs the running load of the Media window to its end, as the worker thread would.
#[cfg(test)]
pub(crate) fn finish_media_load(app: &mut BeeperGuiApp) {
    while let Some(w) = app.media_window.as_ref().filter(|w| w.loading.is_some()) {
        let (id, page) = (w.id, media_page(&app.store.read(), &MediaAsk::of(w)));
        let _ = app.update(UiMessage::Loaded(loads::Msg::MediaPage(id, page)));
    }
}

/// Loads all the items of the Media window.
#[cfg(test)]
pub(crate) fn load_all_media(app: &mut BeeperGuiApp) {
    finish_media_load(app);
    while app.media_window.as_ref().is_some_and(|w| w.more) {
        let _ = app.update(UiMessage::Media(Msg::More));
        finish_media_load(app);
    }
}

/// The opacity of Desktop's `media-viewer-pulse` at `t` seconds: from 0.55 to 1 and back in 1.2 s, `ease-in-out`.
fn pulse(t: f32) -> f32 {
    let half = (t / 0.6).rem_euclid(2.0);
    let k = cubic_bezier(if half < 1.0 { half } else { 2.0 - half }, [0.42, 0.0, 0.58, 1.0]);
    0.55 + 0.45 * k
}

/// The items that the Media window filmstrip shows, as a range: from the scrolled item `from`, or around the current
/// one, like Desktop's `scrollToIndex(i, { align: "center" })`.
pub(crate) fn filmstrip(size: &Size, at: usize, len: usize, from: Option<usize>) -> (usize, usize) {
    let n = ((size.width - 20.0) / 52.0) as usize;
    let start = from.unwrap_or(at.saturating_sub(n / 2)).min(len.saturating_sub(n));
    (start, (start + n).min(len))
}

/// Desktop's Media window date group of a day, like "Today", "Last week" or "March 2025".
fn media_group(d: i64) -> String {
    let t = today();
    let week = t - (t + 3).rem_euclid(7);
    let ymd = civil_from_days(d);
    match t - d {
        0 => tr("Today").into(),
        1 => tr("Yesterday").into(),
        _ if d >= week => tr("This week").into(),
        _ if d >= week - 7 => tr("Last week").into(),
        _ if ymd.0 == civil_from_days(t).0 => i18n::date(i18n::Date::Month, ymd, (0, 0)),
        _ => i18n::date(i18n::Date::MonthYear, ymd, (0, 0)),
    }
}

/// The space under the last row of the Media window list.
const MEDIA_END: f32 = 8.0;

/// The height of a date header of the Media window: Desktop's `dateHeader` padding of 12 and 6 around its
/// `body-lg` label and its `label-sm` dates.
fn media_head() -> f32 {
    18.0 + rem(20.0) + rem(14.0)
}

/// The loaded items of the Media window by Desktop's groups of days, in their order.
fn media_day_groups(w: &MediaWindow) -> Vec<(String, Vec<(&Message, usize)>)> {
    let mut groups: Vec<(String, Vec<(&Message, usize)>)> = Vec::new();
    for (m, k) in &w.items {
        let label = media_group(day_num(&m.timestamp));
        match groups.last_mut() {
            Some((l, g)) if *l == label => g.push((m, *k)),
            _ => groups.push((label, vec![(m, *k)])),
        }
    }
    groups
}

/// The columns and the tile size of the Media window grid in a list of `area`, whose groups have `tiles` tiles each,
/// with `skeletons` skeleton rows under them. Like Desktop, the grid has `max(3, floor(width / 160))` columns of
/// `1fr` with 10 px sides and 4 px gaps, measured on the grid with its scrollbar, and the rows lose the width of the
/// solid scrollbar when they overflow the list, so that every tile has the same size.
pub(crate) fn media_grid(area: Size, tiles: &[usize], skeletons: usize) -> (usize, f32) {
    let cols = ((area.width / 160.0) as usize).max(3);
    let tile = |room: f32| (room - 20.0 - 4.0 * (cols - 1) as f32) / cols as f32;
    let rows = tiles.iter().map(|n| n.div_ceil(cols)).sum::<usize>() + skeletons;
    let height = |tile: f32| tiles.len() as f32 * media_head() + rows as f32 * (tile + 4.0) + MEDIA_END;
    match height(tile(area.width)) > area.height {
        true => (cols, tile(area.width - GUTTER)),
        false => (cols, tile(area.width)),
    }
}

impl BeeperGuiApp {
    /// The media at `url`. A view that shows a URL that no task loads asks for it.
    pub(crate) fn media_at(&self, url: &str) -> Option<&Result<Frames, String>> {
        match self.media.get(url) {
            Some(m) => m.as_ref(),
            None => {
                self.wants.ask(&self.wants.media, url);
                None
            }
        }
    }

    pub(crate) fn media_mut(&mut self) -> &mut MediaWindow {
        self.media_window.as_mut().expect("the Media window is open")
    }

    /// The carousel of the window that `view` draws or that sent the message: the Media window viewer, else the
    /// carousel of the pane, like Desktop's `carouselState` of each window.
    pub(crate) fn carousel(&self) -> Option<&Carousel> {
        match self.in_media.get() {
            true => self.media_window.as_ref().and_then(|w| w.viewer.as_ref()),
            false => self.pane().carousel.as_ref(),
        }
    }

    pub(crate) fn carousel_mut(&mut self) -> &mut Option<Carousel> {
        match self.in_media.get() {
            true => &mut self.media_mut().viewer,
            false => &mut self.pane.carousel,
        }
    }

    /// Like Desktop, the carousel of each window closes when a sync deletes its item, and a "Media in %s" window
    /// closes with its carousel.
    pub(crate) fn close_gone_carousels(&mut self) -> Task<UiMessage> {
        let close = |app: &mut Self| {
            if app.carousel().is_some() && app.carousel_items(&app.store.read()).is_none() {
                *app.carousel_mut() = None;
            }
        };
        close(self);
        for id in self.chat_windows.iter().map(|w| w.0).collect::<Vec<_>>() {
            self.swap_window(id);
            close(self);
            self.swap_window(id);
        }
        if self.media_window.is_some() {
            self.as_media(close);
        }
        let closed = |id: &window::Id| self.chat_windows.iter().any(|w| w.0 == *id && w.1.carousel.is_none());
        Task::batch(self.carousel_windows.iter().filter(|w| closed(&w.0)).map(|w| self.close_window(w.0)))
    }

    /// Runs `f` as the Media window, whose carousel is its viewer.
    pub(crate) fn as_media<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let was = self.in_media.replace(true);
        let out = f(self);
        self.in_media.set(was);
        out
    }

    /// Desktop's `reset` after a filter changes: drops the items, the selection and the viewer, and loads again.
    fn media_reset(&mut self) -> Task<UiMessage> {
        let w = self.media_mut();
        w.generation += 1;
        (w.items, w.more, w.cursor, w.loading, w.advance) = (Vec::new(), true, None, None, false);
        w.picked.clear();
        let close = match self.media_mut().viewer.is_some() {
            true => self.as_media(|s| s.media_view(Msg::CloseCarousel)),
            false => Task::none(),
        };
        Task::batch([close, self.media_more()])
    }

    /// Desktop's `setSearchText`: searches the trimmed text of the search field when it changed.
    fn media_search(&mut self) -> Task<UiMessage> {
        let w = self.media_mut();
        let query = w.search.as_deref().unwrap_or("").trim();
        if query == w.query {
            return Task::none();
        }
        w.query = query.to_string();
        self.media_reset()
    }

    /// Desktop's `loadMore`: loads the next Media window items on a worker thread, unless a load runs or all loaded.
    pub(crate) fn media_more(&mut self) -> Task<UiMessage> {
        let w = self.media_mut();
        if w.loading.is_some() || !w.more {
            return Task::none();
        }
        w.loading = Some(std::time::Instant::now());
        self.media_read()
    }

    fn media_read(&self) -> Task<UiMessage> {
        let w = self.media_window.as_ref().expect("the Media window is open");
        let (store, ask, id) = (self.store.clone(), MediaAsk::of(w), w.id);
        let read = async move { tokio::task::spawn_blocking(move || media_page(&store.read(), &ask)).await.expect("media page thread") };
        Task::perform(read, move |page| UiMessage::Loaded(loads::Msg::MediaPage(id, page)))
    }

    /// The end of a load of the Media window `id`: like Desktop, it adds the items, loads again when it found none
    /// and more can load, and steps the viewer that waits for them.
    pub(crate) fn media_paged(&mut self, id: window::Id, page: MediaPage) -> Task<UiMessage> {
        let w = self.media_window.as_mut().filter(|w| w.id == id && w.generation == page.generation);
        let Some(w) = w else { return Task::none() };
        let again = page.items.is_empty() && page.more && page.cursor.is_some();
        w.items.extend(page.items);
        (w.more, w.cursor) = (page.more, page.cursor);
        if again {
            return self.media_read();
        }
        w.loading = None;
        match std::mem::take(&mut w.advance) {
            true => self.as_media(|s| s.media_view(Msg::CarouselStep(true))),
            false => Task::none(),
        }
    }

    /// Desktop's filmstrip loads more items when it shows one of the last 10 loaded items.
    fn media_viewer_more(&mut self) -> Task<UiMessage> {
        let Some(strip) = self.carousel().filter(|c| c.in_media).map(|c| c.strip) else { return Task::none() };
        let Some((items, at)) = self.carousel_items(&self.store.read()) else { return Task::none() };
        let size = self.media_window.as_ref().expect("the Media window is open").size;
        let (_, end) = filmstrip(&size, at, items.len(), strip);
        match items.len() - end < 10 {
            true => self.media_more(),
            false => Task::none(),
        }
    }

    /// Starts the loads of the media that a view asked for, when a client or HTTPS can load them.
    pub(crate) fn load_media(&mut self) -> Task<UiMessage> {
        // The pages of PDFs are media that draw from their PDF.
        let pdfs = self.load_pdfs();
        let client = self.store.read().matrix.clone();
        let loadable = |u: &String| client.is_some() || u.starts_with("https://") || u.starts_with("data:");
        let (urls, rest) = std::mem::take(&mut *self.wants.media.borrow_mut()).into_iter().partition(loadable);
        *self.wants.media.borrow_mut() = rest;
        let urls: Vec<String> = urls.into_iter().filter(|u| !self.media.contains_key(u)).collect();
        self.media.extend(urls.iter().map(|u| (u.clone(), None)));
        let audio = Task::batch([self.load_audio(), self.auto_transcribe(), self.load_vcards()]);
        let bookmarks = self.load_bookmarks();
        let tiles = Task::batch([self.bake_tiles(), self.cut_animations()]);
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (client.clone(), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the media semaphore stays open");
                    let (u, looped) = u.strip_suffix(LOOP).map_or((u.clone(), false), |u| (u.to_string(), true));
                    let bytes = beeper_core::matrix::media(client, u.clone()).await?;
                    let decode = move || if looped { decode_loop(&bytes) } else { decode_media(bytes) };
                    let frames = tokio::task::spawn_blocking(decode).await.expect("media decode task");
                    frames.inspect_err(|e| eprintln!("media decode failed: {}: {e}", beeper_core::matrix::logged(&u)))
                },
                move |r| UiMessage::Loaded(loads::Msg::Media(url, r)),
            )
        }))
        .chain(pdfs)
        .chain(audio)
        .chain(bookmarks)
        .chain(tiles)
    }

    /// The contact cards that the timeline asked for, read from their `.vcf` files, like Desktop's `Wfe` fetch.
    fn load_vcards(&mut self) -> Task<UiMessage> {
        let Some(client) = self.store.read().matrix.clone() else { return Task::none() };
        let urls: Vec<String> = std::mem::take(&mut *self.wants.vcards.borrow_mut()).into_iter().collect();
        let urls: Vec<String> = urls.into_iter().filter(|u| !self.vcards.contains_key(u)).collect();
        self.vcards.extend(urls.iter().map(|u| (u.clone(), None)));
        Task::batch(urls.into_iter().map(|url| {
            let read = beeper_core::matrix::media(Some(client.clone()), url.clone());
            let card = async move { read.await.map(|b| crate::vcard::parse_vcard(&String::from_utf8_lossy(&b))) };
            Task::perform(card, move |r| UiMessage::Loaded(loads::Msg::VCard(url.clone(), r)))
        }))
    }

    /// The contact card of a `.vcf` attachment once it is read, which the view asks for.
    pub(crate) fn vcard_at(&self, url: &str) -> Option<&Result<crate::vcard::VCard, String>> {
        match self.vcards.get(url) {
            Some(c) => c.as_ref(),
            None => {
                self.wants.ask(&self.wants.vcards, url);
                None
            }
        }
    }

    /// Desktop's `hasNextViewerItem`: the Media window viewer has a next item while more items can load.
    fn more_media(&self, car: &Carousel) -> bool {
        car.in_media && self.media_window.as_ref().is_some_and(|w| w.more)
    }

    /// The (message, attachment index) items of the carousel, and the position of the shown one: like Desktop's
    /// `getCarouselAttachments`, the attachments of the open chat other than audio, oldest first, or the loaded Media
    /// window items. It is `None` when the shown item is gone: a sync deleted its message, or the item was dropped.
    pub(crate) fn carousel_items(&self, st: &BeeperStore) -> Option<(Vec<(String, usize)>, usize)> {
        let c = self.carousel().expect("the carousel is open");
        let items: Vec<_> = match c.in_media {
            true => {
                st.stored_message(&c.mid)?;
                let w = self.media_window.as_ref().expect("the Media window is open");
                w.items.iter().map(|(m, k)| (m.id.clone(), *k)).collect()
            }
            false => {
                let chat = &st.loaded(&c.mid)?.chat_id;
                let messages = st.timeline(chat);
                messages
                    .flat_map(|m| {
                        let shown = |(_, a): &(usize, &Attachment)| a.attachment_type != "audio";
                        m.attachments.iter().enumerate().filter(shown).map(|(ai, _)| (m.id.clone(), ai))
                    })
                    .collect()
            }
        };
        let at = items.iter().position(|(m, k)| *m == c.mid && *k == c.ai)?;
        Some((items, at))
    }

    /// The size of the window of the open carousel.
    fn carousel_area(&self) -> Size {
        match self.carousel().expect("the carousel is open").in_media {
            true => self.media_window.as_ref().expect("the Media window is open").size,
            false => self.pane().size,
        }
    }

    /// The source and the size of the video that the open carousel shows, if it shows a video that is not a GIF. Desktop
    /// shows the video at most at its own size, `100vw - 100px` wide and `90vh` high.
    pub(crate) fn carousel_video(&self) -> Option<(String, (u32, u32))> {
        let c = self.carousel()?;
        let st = self.store.read();
        let m = st.stored_message(&c.mid)?;
        let a = m.attachments.get(c.ai)?;
        let video = a.attachment_type == "video" && a.is_gif != Some(true) && self.needs_download(a).is_none();
        let src = a.src_url.clone().filter(|_| video)?;
        let area = self.carousel_area();
        let size = a.size.as_ref();
        let (w, h) = size.map_or((240.0, 160.0), |s| (s.width.unwrap_or(240) as f32, s.height.unwrap_or(160) as f32));
        let k = ((area.width - 100.0) / w).min(0.9 * area.height / h).min(1.0);
        Some((src, ((w * k).round() as u32, (h * k).round() as u32)))
    }

    pub(crate) fn carousel_key(&mut self, key: &Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
        let items = self.carousel_items(&self.store.read());
        let Some((items, at)) = items else {
            return self.update(UiMessage::Media(Msg::CloseCarousel));
        };
        let c = self.carousel().cloned().expect("the carousel is open");
        let video = self.carousel_video().and_then(|(src, _)| self.video.as_ref().filter(|v| v.url == src));
        let (video, full) = (video.is_some(), video.is_some_and(video::Video::is_full));
        match key.as_ref() {
            Key::Named(Named::Escape) if full => self.video(video::Msg::Fullscreen),
            Key::Named(Named::Escape) if self.pane.emoji.is_some() => self.update(UiMessage::CloseModals),
            Key::Named(Named::Escape) if c.menu.is_some() => self.update(UiMessage::Media(Msg::CarouselMenu(false))),
            Key::Named(Named::Escape) => self.update(UiMessage::Media(Msg::CloseCarousel)),
            Key::Character("f") if mods.is_empty() && video => self.video(video::Msg::Fullscreen),
            Key::Named(Named::ArrowLeft) if mods.is_empty() && at > 0 => self.update(UiMessage::Media(Msg::CarouselStep(false))),
            Key::Named(Named::ArrowRight) if mods.is_empty() && (at + 1 < items.len() || self.more_media(&c)) => {
                self.update(UiMessage::Media(Msg::CarouselStep(true)))
            }
            Key::Character("d") if mods.is_empty() => self.update(UiMessage::Message(message::Msg::Download(c.mid, Some(c.ai)))),
            Key::Character("r") if mods.is_empty() => self.update(UiMessage::Media(Msg::RotateCarousel)),
            _ => Task::none(),
        }
    }

    /// The React and Reply actions of the chat carousel on message `m`, like Desktop's `ChatCarousel`: React opens the
    /// emoji picker 4 px below the React button, with its right edge at the right edge of the button, and Reply closes
    /// the carousel. A "Media in %s" window starts the reply in its thread window, Desktop's opener.
    fn carousel_actions(&self, car: &Carousel, m: &Message, c: &Chat) -> (Option<UiMessage>, Option<UiMessage>) {
        let live = !car.in_media && m.is_deleted != Some(true);
        let reply = (c.messages.reply && live).then(|| {
            let start = UiMessage::Composer(composer::Msg::StartReply(m.id.clone()));
            let start = match self.drawn_opener() {
                Some(opener) => UiMessage::InWindow(opener, Box::new(start)),
                None => start,
            };
            UiMessage::Commands(vec![UiMessage::Media(Msg::CloseCarousel), start])
        });
        let right = self.pane().size.width - 8.0 - 28.0 * if reply.is_some() { 4.0 } else { 3.0 };
        let react = (c.messages.react && live).then(|| UiMessage::Message(message::Msg::ReactAt(m.id.clone(), Point::new(right, 39.0))));
        (react, reply)
    }

    /// The carousel, or `None` until the next store change closes a carousel whose item is gone.
    pub(crate) fn view_carousel<'a>(&'a self, car: &Carousel) -> Option<El<'a>> {
        if let Some(full) = self.view_full_video(true) {
            return Some(full);
        }
        let video = self.carousel_video();
        let p = &self.pal;
        let st = self.store.read();
        let (items, at) = self.carousel_items(&st)?;
        let (mid, ai) = (car.mid.as_str(), car.ai);
        let m = &stored(&st, mid);
        let c = st.chat(&m.chat_id);
        let a = &m.attachments[ai];
        let h24 = st.settings.pref("timeFormat") == 1;
        let white = Color::WHITE;
        // Desktop's carousel controls have a tooltip with their key.
        let control = |h: &svg::Handle, label: &'static str, key: Vec<&'static str>, msg: Option<UiMessage>| -> El<'a> {
            let b = button(tinted(h, 20.0, white)).bare(white);
            self.tip(b.on_press_maybe(msg).into(), label, key, tooltip::Position::Bottom).into()
        };
        let action = |s| trx(s, "attachment carousel action");
        let (face, name) = match car.in_media {
            true => {
                let u = c.participants.items.iter().find(|u| u.id == m.sender_id);
                let name = Self::sender_name(&st, m);
                (u.map(|u| self.face(crate::user_img(u).as_deref(), name, 28.0, 0.0)), name)
            }
            false => (Some(self.chat_avatar(c, 28.0, 0.0)), c.title.as_str()),
        };
        let title = row![]
            .push(face)
            // `.carousel-thread-title` and `.carousel-timestamp` are 14 px and 12 px.
            .push(crate::style::fixed_ell(name, 600.0, 14.0, Weight::Medium).color(white))
            .push(crate::style::fixed(format_short_ts(&m.timestamp, h24), 12.0, Weight::Normal).color(white.scale_alpha(0.7)))
            .spacing(8)
            .align_y(Center);
        let go = car.in_media.then(|| {
            control(&MEDIA_ICONS[3], tr("Go to Message"), vec![], Some(UiMessage::Message(message::Msg::GoTo(c.id.clone(), mid.into()))))
        });
        // The Media window viewer has Delete when you can delete the message, like Desktop's `MediaViewer`.
        let delete = (car.in_media && deletable(&st, m))
            .then(|| control(&SYS_DELETE, tr("Delete"), vec![], Some(UiMessage::Media(Msg::AskDelete(vec![(mid.into(), ai)])))));
        // Desktop shows Fullscreen in place of Rotate for a video.
        let turn_or_full = match &video {
            Some((src, _)) => {
                let full = self.video.as_ref().filter(|v| v.url == *src).map(|_| UiMessage::Video(video::Msg::Fullscreen));
                control(&video::GLYPHS[3], tr("Fullscreen"), vec!["F"], full)
            }
            None => control(&SYS_ROTATE, action("Rotate"), vec!["R"], Some(UiMessage::Media(Msg::RotateCarousel))),
        };
        let (react, reply) = self.carousel_actions(car, m, c);
        let react = react.map(|msg| control(&SYS_EMOJI, action("React"), vec![], Some(msg)));
        let reply = reply.map(|msg| control(&SYS_REPLY, action("Reply"), vec![], Some(msg)));
        let download = Some(UiMessage::Message(message::Msg::Download(mid.to_string(), Some(ai))));
        let controls = row![]
            .push(go)
            .push(delete)
            .push(react)
            .push(reply)
            .push(turn_or_full)
            .push(control(&SYS_DOWNLOAD, action("Download"), vec!["D"], download))
            .push(control(&ROW_ACTIONS[6], action("Close"), vec!["Escape"], Some(UiMessage::Media(Msg::CloseCarousel))))
            .spacing(8);
        let head = stack![container(title).center_x(Fill).center_y(34), container(controls).align_right(Fill).center_y(34)];
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        // Desktop turns the image by a quarter in `.2s` with the CSS `ease` curve.
        let quarter = -std::f32::consts::FRAC_PI_2;
        let turn = car.turned.map_or((0.0, 0.0, self.started), |at| {
            let before = f32::from((car.turns + 3) % 4);
            (quarter * before, quarter * (before + 1.0), at)
        });
        let loaded = url.and_then(|u| self.media_at(u));
        let frames = loaded.and_then(|f| f.as_ref().ok());
        // Desktop's `renderAttachment`: media that a bridge loads only when the user asks is a file row that asks for
        // it, a video plays with its controls and does not zoom, a PDF shows a page with its page controls, and
        // another file is a link that downloads it.
        let mut pages = None;
        let main: El<'a> = match (a.attachment_type.as_str(), &video) {
            _ if self.needs_download(a).is_some() => container(self.view_file_row(m, ai, a, None)).center(Fill).into(),
            ("video", Some((src, (w, h)))) => {
                let (w, h) = (*w as f32, *h as f32);
                let poster: El<'a> = match frames {
                    Some(f) => image(f[0].0.clone()).width(w).height(h).into(),
                    None => Space::new().width(w).height(h).into(),
                };
                let video = self.view_video(mid, src, poster, (w, h), video::Place::Carousel);
                let video = mouse_area(video).on_press(UiMessage::Noop);
                container(video).center(Fill).padding(8).into()
            }
            // Desktop's `ImageError` fills the place of the image, and a click on it stays in the carousel.
            ("img" | "video", ..) if loaded.is_some_and(Result::is_err) => {
                let error = mouse_area(self.image_error(Fill, Fill, 0.0)).on_press(UiMessage::Noop);
                container(error).padding(8).into()
            }
            ("img" | "video", ..) => {
                let shown: El<'a> = match frames {
                    Some(frames) => {
                        let play = self.autoplays(&st);
                        anim::turning(frames, play, self.started, turn, |f| image(f).content_fit(iced::ContentFit::ScaleDown))
                    }
                    None => Space::new().into(),
                };
                container(zoom::zoom(shown, vlist::key((mid, ai)), UiMessage::Media(Msg::CloseCarousel))).padding(8).into()
            }
            _ if let Some(url) = a.src_url.as_deref().filter(|_| pdf::is_pdf(a)) => {
                let (page, controls) = self.view_pdf(url, car.page, turn, self.carousel_area());
                pages = Some(controls);
                container(zoom::zoom(page, vlist::key((mid, ai)), UiMessage::Media(Msg::CloseCarousel))).padding(8).into()
            }
            _ => {
                let name = txt(crate::message::attachment_name(a), 13.0).color(white);
                let link = button(name).bare(white);
                container(link.on_press(UiMessage::Message(message::Msg::Download(mid.to_string(), Some(ai))))).center(Fill).into()
            }
        };
        let step = |next: bool| -> El<'a> {
            let arrow = svg(SYS_ARROW_BACK.clone())
                .width(20)
                .height(20)
                .rotation(if next { std::f32::consts::PI } else { 0.0 })
                .style(move |_, _| svg::Style { color: Some(white) });
            let b = button(container(arrow).center(32)).padding(0).style(btn_style([Color { a: 0.1, ..p.strong }; 3], [white; 3], 32.0));
            let (label, key, at) = match next {
                true => (tr("Next Attachment"), "→", tooltip::Position::Top),
                false => (tr("Previous Attachment"), "←", tooltip::Position::Bottom),
            };
            let b = self.tip(b.on_press(UiMessage::Media(Msg::CarouselStep(next))).into(), label, vec![key], at);
            let b = container(b).center_y(Fill).padding([0, 24]);
            if next {
                b.align_right(Fill).into()
            } else {
                b.into()
            }
        };
        let strip = car.in_media.then(|| {
            let w = self.media_window.as_ref().expect("the Media window is open");
            let (start, end) = filmstrip(&w.size, at, items.len(), car.strip);
            let thumbs = w.items[start..end].iter().enumerate().map(|(k, (m, ai))| -> El<'a> {
                let (mid, a) = (&m.id, &m.attachments[*ai]);
                let edge = if start + k == at { p.accent } else { CLEAR };
                let face = container(self.thumb(a, 44.0, 6.0)).padding(2);
                let face = face.style(bordered_box(white.scale_alpha(0.1), edge, 2.0, 8.0));
                let open = UiMessage::Media(Msg::OpenCarousel(mid.clone(), *ai, true));
                button(face).bare(white).on_press(open).into()
            });
            let row = container(Row::with_children(thumbs).spacing(4)).padding([6, 10]).width(Fill);
            let scroll = |d: mouse::ScrollDelta| {
                let by = match d {
                    mouse::ScrollDelta::Lines { x, y } => -(x + y),
                    mouse::ScrollDelta::Pixels { x, y } => -(x + y) / 52.0,
                };
                UiMessage::Media(Msg::ScrollFilmstrip(by))
            };
            let row = mouse_area(row.style(fill_box(Color::BLACK.scale_alpha(0.45), 0.0))).on_scroll(scroll);
            container(row.on_press(UiMessage::Noop)).align_bottom(Fill)
        });
        // Desktop's `carousel-reactions` in a chat: the reactions of the message, 16 px above the bottom in the middle.
        let reactions = (!car.in_media && !m.reactions.is_empty() && m.is_deleted != Some(true)).then(|| {
            let look = PillLook {
                group: c.chat_type == "group",
                own: Color { a: 0.6, ..p.accent },
                own_bg: None,
                other: Color::BLACK.scale_alpha(0.6),
                hover: None,
                fg: (white, white),
                ring: (white.scale_alpha(0.2), 1.0),
                gap: 4.0,
            };
            let pills = mouse_area(self.reaction_pills(&st, c, &[m], &look)).on_press(UiMessage::Noop);
            container(pills).align_bottom(Fill).center_x(Fill).padding(Padding::new(24.0).top(0.0).bottom(16.0))
        });
        let main = mouse_area(main).on_right_press(UiMessage::Media(Msg::CarouselMenu(true)));
        let body = column![container(head).padding(8).height(50), main];
        let layer = stack![body]
            .push((at > 0).then(|| step(false)))
            .push((at + 1 < items.len() || self.more_media(car)).then(|| step(true)))
            .push(video.and_then(|(src, _)| self.view_carousel_video_controls(&src)))
            .push(pages)
            .push(reactions)
            .push(strip)
            .push(car.menu.map(|at| self.view_carousel_menu(&st, car, m, c, at)));
        let scrim = container(layer).width(Fill).height(Fill).style(fill_box(Color::BLACK.scale_alpha(0.85), 0.0));
        // The carousel hides the window under it from the cursor.
        Some(mouse_area(scrim).interaction(mouse::Interaction::Idle).on_press(UiMessage::Media(Msg::CloseCarousel)).into())
    }

    /// Desktop's carousel menu: Download, Copy for an image, and the extra items. In a chat, the extra items are Reply…,
    /// React…, Forward… and Delete…, and all but React… close the carousel first. In the Media window, they are Go to
    /// Message and Delete….
    fn view_carousel_menu<'a>(&'a self, st: &BeeperStore, car: &Carousel, m: &Message, c: &Chat, at: Point) -> El<'a> {
        let p = &self.pal;
        let (mid, ai) = (car.mid.clone(), car.ai);
        let close = |msg| UiMessage::Commands(vec![UiMessage::Media(Msg::CloseCarousel), msg]);
        let action = |s| trx(s, "attachment carousel context menu action").to_string();
        let image = m.attachments[ai].attachment_type == "img";
        let mut rows = vec![Some((action("Download"), UiMessage::Message(message::Msg::Download(mid.clone(), Some(ai)))))];
        rows.extend(image.then(|| Some((action("Copy"), UiMessage::Message(message::Msg::CopyImage(mid.clone(), ai))))));
        let deletable = deletable(st, m);
        let (first, delete) = match car.in_media {
            true => (
                vec![(tr("Go to Message").into(), UiMessage::Message(message::Msg::GoTo(c.id.clone(), mid.clone())))],
                deletable.then(|| (tr("Delete…").into(), UiMessage::Media(Msg::AskDelete(vec![(mid.clone(), ai)])))),
            ),
            false => {
                let (react, reply) = self.carousel_actions(car, m, c);
                // Desktop forwards only from the window of the chat, not from a "Media in %s" window.
                let forward = self.drawn_opener().is_none().then(|| close(UiMessage::Forward(forward::Msg::Open(vec![mid.clone()]))));
                let first = [(tr("Reply…"), reply), (tr("React…"), react), (tr("Forward…"), forward)];
                let first = first.into_iter().filter_map(|(label, msg)| msg.map(|msg| (label.to_string(), msg)));
                let delete = close(UiMessage::Message(message::Msg::AskDelete(vec![mid.clone()])));
                (first.collect(), deletable.then(|| (tr("Delete…").into(), delete)))
            }
        };
        rows.push(None);
        rows.extend(first.into_iter().map(Some));
        rows.extend(delete.map(|d| [None, Some(d)]).into_iter().flatten());
        let h = 8.0 + rows.iter().map(|r| if r.is_some() { menu_pitch() } else { MENU_RULE }).sum::<f32>();
        let rows = rows.into_iter().map(|r| match r {
            Some((label, msg)) => {
                let msg = UiMessage::Commands(vec![UiMessage::Media(Msg::CarouselMenu(false)), msg]);
                menu_item(p, None, label, "", Some(msg))
            }
            None => menu_rule(p),
        });
        let menu = crate::menu_card(p, Column::with_children(rows), 180);
        let area = self.carousel_area();
        let at = Padding::ZERO.left(at.x.min(area.width - 188.0)).top(at.y.min(area.height - h - 8.0));
        crate::backdrop(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
            .on_press(UiMessage::Media(Msg::CarouselMenu(false)))
            .on_right_press(UiMessage::Media(Msg::CarouselMenu(false)))
            .into()
    }

    pub(crate) fn view_media<'a>(&'a self, w: &'a MediaWindow) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let header = self.media_header(w);
        let rule = || container(Space::new()).width(Fill).height(0.5).style(fill_box(p.chip, 0.0));
        let search_row = w.search.as_deref().map(|q| column![self.media_search_row(q), rule()]);
        // Like Desktop's `ResizeObserver` on the grid, the tiles follow the size of the list.
        let list = iced::widget::responsive(move |area| self.media_list(w, area));
        let title = container(semi(tr("Media"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let footer = w.select.then(|| column![rule(), self.media_footer(w)]);
        let pane = column![header, rule()].push(search_row).push(list).push(footer);
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.card, 12.0));
        let base = container(column![title, pane]).padding(8).width(Fill).height(Fill).style(fill_box(p.window, 0.0));
        let item_menu = self.media_item_menu(&st, w);
        let menu = w.menu.map(|at| self.media_filter_menu(w, at));
        drop(st);
        // The viewer handles its messages as the Media window, whose carousel it is.
        let id = w.id;
        let carousel = w.viewer.as_ref().and_then(|c| self.view_carousel(c));
        let carousel = carousel.map(|c| c.map(move |m| UiMessage::InWindow(id, Box::new(m))));
        let ask = w.deleting.as_ref().map(|(pairs, _)| self.media_delete_ask(pairs.len()));
        // The menus and the delete dialog of the viewer show over it.
        stack![base].push(self.view_toast(false)).push(carousel).push(menu).push(item_menu).push(ask).into()
    }

    /// A round button of the header of the Media window, filled while it is on.
    fn media_control<'a>(&self, glyph: &svg::Handle, size: f32, on: bool, msg: UiMessage) -> El<'a> {
        let p = &self.pal;
        let (bg, fg) = if on { (p.chip, p.strong) } else { (CLEAR, p.on_secondary) };
        let b = button(container(tinted(glyph, size, fg)).center(28)).padding(0);
        b.style(btn_style([bg, p.chip, p.chip], [fg; 3], 12.0)).on_press(msg).into()
    }

    /// The header of the Media window: Desktop's `BrandSegmentControl` of the tabs, whose highlight slides to the
    /// selected tab in `0.25s`, and Select, Filter and Search.
    fn media_header<'a>(&'a self, w: &MediaWindow) -> El<'a> {
        let p = &self.pal;
        let [_, _, _, _, tune_icon, search_icon, done_icon] = &*MEDIA_ICONS;
        let widths = MEDIA_TABS.map(|l| text_width(tr(l), 11.0, Weight::Normal) + 16.0);
        let (from, t) = w.tab_at.map_or((w.tab, 1.0), |(from, at)| (from, at.elapsed().as_secs_f32() / TAB_SLIDE));
        let (x, hw) = slide(&widths, (from, w.tab), t);
        let highlight = container(Space::new().width(hw).height(18)).style(fill_box(p.chip, 20.0));
        let tabs = MEDIA_TABS.iter().zip(widths).enumerate().map(|(i, (label, width))| -> El<'a> {
            let label = txt(tr(label), 11.0).color(p.strong);
            button(container(label).center_x(Fill))
                .width(width)
                .bare(p.strong)
                .padding([2, 8])
                .on_press(UiMessage::Media(Msg::Tab(i)))
                .into()
        });
        // The highlight layer comes first, under the labels, so it sets the size of the stack.
        let total = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
        let under = container(highlight).padding(Padding::ZERO.left(x)).width(total);
        let tabs = stack![under, Row::with_children(tabs).spacing(4)];
        let tabs = container(tabs).padding(4).style(fill_box(p.card, 20.0));
        let tip = |el, label| self.tip(el, label, vec![], tooltip::Position::Bottom);
        let search = UiMessage::Media(Msg::Search(w.search.is_none().then(String::new)));
        let controls = row![
            tip(self.media_control(done_icon, 18.0, w.select, UiMessage::Media(Msg::Select(!w.select))), tr("Select")),
            tip(self.media_control(tune_icon, 18.0, false, UiMessage::Media(Msg::Menu(true))), tr("Filter")),
            tip(self.media_control(search_icon, 18.0, w.search.is_some(), search), tr("Search")),
        ]
        .spacing(4);
        container(row![tabs, Space::new().width(Fill), controls].spacing(8).align_y(Center)).padding([8, 10]).center_y(52).into()
    }

    /// The caption search field of the Media window, with its Close button.
    fn media_search_row<'a>(&'a self, q: &str) -> El<'a> {
        let p = &self.pal;
        let input = text_input(tr("Search by caption"), q)
            .id("media-search")
            .on_input(|q| UiMessage::Media(Msg::Search(Some(q))))
            .size(13)
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
        let close = self.media_control(&ROW_ACTIONS[6], 16.0, false, UiMessage::Media(Msg::Search(None)));
        let close = self.tip(close, tr("Close"), vec![], tooltip::Position::Bottom);
        let line = row![tinted(&MEDIA_ICONS[5], 16.0, p.on_secondary), input, close].spacing(6).align_y(Center);
        container(line).padding([6, 10]).into()
    }

    /// The list of the Media window in `area`: the groups of items, the skeleton rows while items load, or
    /// "Nothing here".
    fn media_list<'a>(&'a self, w: &'a MediaWindow, area: Size) -> El<'a> {
        let p = &self.pal;
        if w.items.is_empty() && w.loading.is_none() {
            return container(txt(tr("Nothing here"), 13.0).color(p.on_secondary)).center_x(Fill).padding(16).into();
        }
        let groups = media_day_groups(w);
        // Desktop's pulsing skeleton rows while items load: three without items, else one under them.
        let skeletons = w.loading.map_or(0, |_| if w.items.is_empty() { 3 } else { 1 });
        let sizes: Vec<usize> = groups.iter().map(|(_, g)| if w.tab == 0 { g.len() } else { 0 }).collect();
        let (cols, tile) = media_grid(area, &sizes, skeletons);
        let st = self.store.read();
        let body = self.media_groups(&st, w, &groups, cols, tile);
        drop(st);
        // Desktop loads more items when the list shows one of its last 8 rows, and again when a load adds rows.
        let row_h = if w.tab == 0 { tile + 4.0 } else { 52.0 };
        let end = (w.more && w.viewer.is_none()).then(|| {
            iced::widget::sensor(Space::new().width(Fill))
                .key((w.generation, w.items.len()))
                .anticipate(8.0 * row_h)
                .on_show(|_| UiMessage::Media(Msg::More))
        });
        let fade = w.loading.map_or(1.0, |at| pulse(at.elapsed().as_secs_f32()));
        let skeleton = |_| -> El<'a> {
            let tiles = (0..cols).map(|_| -> El<'a> {
                let face = container(Space::new()).width(tile).height(tile);
                face.style(fill_box(p.chip.scale_alpha(fade), 8.0)).into()
            });
            let row = container(Row::with_children(tiles).spacing(4));
            row.padding(Padding::new(0.0).bottom(4.0).left(10.0).right(10.0)).into()
        };
        let body = body.push(end).extend((0..skeletons).map(skeleton));
        scrollable(body.push(Space::new().height(MEDIA_END))).direction(bar()).height(Fill).style(scroll_style(p.scroll)).into()
    }

    /// The items of the Media window by Desktop's groups of days, each under its label and its dates: rows of
    /// `cols` tiles on the Media tab, else a row for each link or file.
    fn media_groups<'a>(
        &'a self, st: &BeeperStore, w: &MediaWindow, groups: &[(String, Vec<(&Message, usize)>)], cols: usize, tile: f32,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        groups.iter().fold(Column::new(), |col, (label, group)| {
            let days = group.iter().map(|&(m, _)| day_num(&m.timestamp));
            let (lo, hi) = (days.clone().min().expect("a group has items"), days.max().expect("a group has items"));
            let full = |d| i18n::date(i18n::Date::Medium, civil_from_days(d), (0, 0));
            let range = if lo == hi { full(lo) } else { format!("{} - {}", full(lo), full(hi)) };
            let range = txt(range, 11.0).color(p.on_secondary).wrapping(iced::widget::text::Wrapping::None);
            let head = column![semi(label.clone(), 15.0).line_height(lh(20.0)).color(p.strong), range];
            let col = col.push(container(head).padding(Padding::new(6.0).top(12.0).left(10.0).right(10.0)));
            match w.tab {
                0 => group.chunks(cols).fold(col, |col, chunk| {
                    let tiles = chunk.iter().map(|&(m, k)| self.media_tile(st, w, m, k, tile));
                    col.push(container(Row::with_children(tiles).spacing(4)).padding(Padding::new(0.0).bottom(4.0).left(10.0).right(10.0)))
                }),
                _ => group.iter().fold(col, |col, &(m, k)| col.push(self.media_row(st, w, m, k))),
            }
        })
    }

    /// The check mark of an item of the Media window while it selects.
    fn media_check<'a>(&self, on: bool) -> El<'a> {
        let p = &self.pal;
        let (bg, edge) = if on { (p.accent, p.accent) } else { (Color::BLACK.scale_alpha(0.25), Color::WHITE) };
        let mark = on.then(|| tinted(&MEDIA_ICONS[6], 14.0, Color::WHITE));
        container(mark).center(20).style(bordered_box(bg, edge, 1.5, 10.0)).into()
    }

    /// A tile of the Media tab: the image or video `k` of `m`, with its GIF or video badge and its sender, which
    /// opens the viewer, or picks it while the window selects.
    fn media_tile<'a>(&'a self, st: &BeeperStore, w: &MediaWindow, m: &Message, k: usize, tile: f32) -> El<'a> {
        let p = &self.pal;
        let a = &m.attachments[k];
        let badge = match (a.is_gif == Some(true), a.attachment_type == "video") {
            (true, _) => Some(row![semi(tr("GIF"), 11.0)]),
            (false, true) => {
                let time = a.duration.map(|d| semi(voice::clock(d as f32), 11.0));
                Some(row![tinted(&MEDIA_ICONS[1], 12.0, Color::WHITE)].push(time).spacing(2).align_y(Center))
            }
            _ => None,
        };
        let badge = badge.map(|b| {
            let b = container(b).padding([1, 4]).style(move |_: &Theme| container::Style {
                text_color: Some(Color::WHITE),
                ..fill_box(Color::BLACK.scale_alpha(0.55), 8.0)(&Theme::Dark)
            });
            container(b).padding(4)
        });
        let fade = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
            .add_stop(0.0, CLEAR)
            .add_stop(1.0, Color::BLACK.scale_alpha(0.55));
        let name = ell(Self::sender_name(st, m), tile - 12.0, 11.0, Weight::Semibold).color(Color::WHITE);
        let name = container(name).width(Fill).padding(Padding::new(4.0).top(10.0).left(6.0).right(6.0));
        let name = container(name.style(move |_| container::Style {
            background: Some(fade.into()),
            border: border::rounded(border::Radius::default().bottom(8)),
            ..Default::default()
        }))
        .align_bottom(Fill);
        let on = w.is_picked(&m.id, k);
        let mark = w.select.then(|| container(self.media_check(on)).align_right(Fill).padding(4));
        let ring = on.then(|| container(Space::new()).width(Fill).height(Fill).style(bordered_box(CLEAR, p.accent, 2.0, 8.0)));
        let face = stack![self.thumb(a, tile, 8.0)].push(badge).push(name).push(mark).push(ring);
        let face = face.width(tile).height(tile);
        let open = match w.select || self.mods.control() {
            true => UiMessage::Media(Msg::Pick(m.id.clone(), k)),
            false => UiMessage::Media(Msg::OpenCarousel(m.id.clone(), k, true)),
        };
        let b = button(face).bare(p.strong);
        mouse_area(b.on_press(open)).on_right_press(UiMessage::Media(Msg::ItemMenu(Some((m.id.clone(), k))))).into()
    }

    /// A row of the Links or Docs tab: the link `k` of `m`, which opens, or its file, which downloads, with the sender
    /// and the date, or which it picks while the window selects.
    fn media_row<'a>(&'a self, st: &BeeperStore, w: &MediaWindow, m: &Message, k: usize) -> El<'a> {
        let p = &self.pal;
        let text_w = w.size.width - 16.0 - 20.0 - 48.0 - 8.0 - 150.0;
        let sub = |s: String| txt(s, 11.0).color(p.on_secondary);
        let icon =
            |glyph, size| -> El<'a> { container(tinted(glyph, size, p.on_secondary)).center(40).style(fill_box(p.chip, 8.0)).into() };
        let (thumb, title, line, press) = match w.tab {
            // Desktop's link thumb: the preview image of the link, or the link icon.
            1 => {
                let l = &m.links[k];
                let title = crate::links::shown_title(l);
                let thumb = match l.img.as_deref().map(|u| self.media_at(u)) {
                    Some(Some(Ok(frames))) => {
                        image(frames[0].0.clone()).width(40).height(40).content_fit(iced::ContentFit::Cover).border_radius(8.0).into()
                    }
                    Some(_) => container(Space::new()).width(40).height(40).style(fill_box(p.chip, 8.0)).into(),
                    None => icon(&MEDIA_ICONS[2], 16.0),
                };
                (thumb, title, l.url.clone(), UiMessage::OpenUrl(l.url.clone()))
            }
            _ => {
                let a = &m.attachments[k];
                let kind = file_kind(a.mime_type.as_deref());
                let line = std::iter::once(kind.to_string()).chain(a.file_size.map(human_size));
                let name = a.file_name.clone().unwrap_or_else(|| tr("Attachment").into());
                let download = UiMessage::Message(message::Msg::Download(m.id.clone(), Some(k)));
                (icon(&MEDIA_ICONS[0], 18.0), name, line.collect::<Vec<_>>().join(" · "), download)
            }
        };
        let text = column![
            ell(&title, text_w, 13.0, Weight::Semibold).line_height(lh(16.0)).color(p.strong),
            ell(&line, text_w, 11.0, Weight::Normal).line_height(lh(14.0)).color(p.on_secondary),
        ]
        .width(Fill);
        let meta = column![sub(Self::sender_name(st, m).to_string()), sub(relative_date(&m.timestamp))].align_x(Right);
        let on = w.is_picked(&m.id, k);
        let line = row![thumb, text, meta].push(w.select.then(|| self.media_check(on))).spacing(8).align_y(Center);
        let picking = w.select || self.mods.control();
        let press = if picking { UiMessage::Media(Msg::Pick(m.id.clone(), k)) } else { press };
        let bg = if on { p.chip } else { CLEAR };
        let b = button(line).padding([6, 10]).width(Fill);
        let b = b.style(btn_style([bg, p.chip, p.chip], [p.strong; 3], 0.0)).on_press(press);
        mouse_area(b).on_right_press(UiMessage::Media(Msg::ItemMenu(Some((m.id.clone(), k))))).into()
    }

    /// The bar of the Media window while it selects: the count and size of the picked items, Delete, Save, Select
    /// all and Cancel.
    fn media_footer<'a>(&self, w: &MediaWindow) -> El<'a> {
        let p = &self.pal;
        let bytes: u64 = match w.tab {
            1 => 0,
            _ => {
                let shown = w.items.iter().filter(|(m, k)| w.is_picked(&m.id, *k));
                shown.filter_map(|(m, k)| m.attachments[*k].file_size).sum()
            }
        };
        let n = w.picked.len();
        let count = match bytes {
            0 => trn("%d selected", "%d selected", n, &[&n]),
            _ => trn("%1$d selected (%2$s)", "%1$d selected (%2$s)", n, &[&n, &human_size(bytes)]),
        };
        let act = |label, msg: Option<UiMessage>| -> El<'a> {
            let a = if msg.is_some() { 1.0 } else { 0.5 };
            let b = button(semi(label, 11.0).color(p.strong.scale_alpha(a))).padding([4, 8]);
            b.style(btn_style([p.chip.scale_alpha(a); 3], [p.strong; 3], 12.0)).on_press_maybe(msg).into()
        };
        let picked = w.picked.clone();
        let bar = row![
            semi(count, 13.0).color(p.strong),
            Space::new().width(Fill),
            act(tr("Delete"), (n > 0).then(|| UiMessage::Media(Msg::AskDelete(picked)))),
            act(tr("Save"), (n > 0 && w.tab != 1).then_some(UiMessage::Media(Msg::Save))),
            act(tr("Select all"), Some(UiMessage::Media(Msg::PickAll))),
            act(tr("Cancel"), Some(UiMessage::Media(Msg::Select(false)))),
        ]
        .spacing(4)
        .align_y(Center);
        container(bar).padding([8, 10]).center_y(44).into()
    }

    /// The context menu of an item of the Media window: Go to Message, Copy Link or Download and Copy Image, and
    /// Delete. A filter change can drop the item of an open menu.
    fn media_item_menu<'a>(&self, st: &BeeperStore, w: &MediaWindow) -> Option<El<'a>> {
        let p = &self.pal;
        let (mid, k, at) = w.item_menu.as_ref()?;
        let m = w.message(mid)?;
        let act = |label: &str, msg| menu_item(p, None, label.into(), "", Some(UiMessage::Media(Msg::ItemAct(Box::new(msg)))));
        let go = act(tr("Go to Message"), UiMessage::Message(message::Msg::GoTo(m.chat_id.clone(), mid.clone())));
        let mut actions = match w.tab {
            1 => vec![act(tr("Copy Link"), UiMessage::Message(message::Msg::CopyLink(m.links[*k].url.clone())))],
            _ => std::iter::once(act(tr("Download"), UiMessage::Message(message::Msg::Download(mid.clone(), Some(*k)))))
                .chain(
                    (m.attachments[*k].attachment_type == "img")
                        .then(|| act(tr("Copy Image"), UiMessage::Message(message::Msg::CopyImage(mid.clone(), *k)))),
                )
                .collect(),
        };
        actions.extend(deletable(st, m).then(|| act(tr("Delete…"), UiMessage::Media(Msg::AskDelete(vec![(mid.clone(), *k)])))));
        let h = 8.0 + menu_pitch() * (1 + actions.len()) as f32 + MENU_RULE;
        let col = Column::with_children([go, menu_rule(p)].into_iter().chain(actions));
        let menu = crate::menu_card(p, col, 180);
        let at = Padding::ZERO.left(at.x.min(w.size.width - 180.0 - 8.0)).top(at.y.min(w.size.height - h - 8.0));
        let menu = container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill);
        Some(
            crate::backdrop(menu)
                .on_press(UiMessage::Media(Msg::ItemMenu(None)))
                .on_right_press(UiMessage::Media(Msg::ItemMenu(None)))
                .into(),
        )
    }

    /// Desktop's filter menu of the Media window, a native menu at the cursor: Sent By and Sort by.
    fn media_filter_menu<'a>(&self, w: &MediaWindow, at: Point) -> El<'a> {
        let p = &self.pal;
        let heading = |label| container(med(label, 11.0).color(p.sub)).padding([6, 13]);
        let radio = |label: &str, on: bool, msg| {
            let lead = container(on.then(|| ic(icon::CHECK, 14.0))).width(22);
            menu_item(p, Some(lead.into()), label.into(), "", Some(msg))
        };
        let col = column![
            heading(tr("Sent By")),
            radio(tr("All"), w.sent_by == 0, UiMessage::Media(Msg::SentBy(0))),
            radio(tr("You"), w.sent_by == 1, UiMessage::Media(Msg::SentBy(1))),
            radio(tr("Others"), w.sent_by == 2, UiMessage::Media(Msg::SentBy(2))),
            menu_rule(p),
            heading(tr("Sort by")),
            radio(tr("Newest"), !w.oldest, UiMessage::Media(Msg::Oldest(false))),
            radio(tr("Oldest"), w.oldest, UiMessage::Media(Msg::Oldest(true))),
        ];
        let menu = crate::menu_card(p, col, 180);
        let at = Padding::ZERO.top(at.y).left(at.x.min(w.size.width - 8.0 - 180.0));
        crate::backdrop(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
            .on_press(UiMessage::Media(Msg::Menu(false)))
            .into()
    }

    /// The dialog that asks to delete the `n` messages of the picked items.
    fn media_delete_ask<'a>(&'a self, n: usize) -> El<'a> {
        let body = match n {
            1 => tr("Are you sure you want to delete this message?").to_string(),
            _ => trn("Are you sure you want to delete %d message?", "Are you sure you want to delete %d messages?", n, &[&n]),
        };
        let title = trn("Delete message", "Delete messages", n, &[]);
        self.dialog(
            None,
            title,
            body,
            vec![(tr("Delete"), UiMessage::Media(Msg::Delete(true)))],
            (tr("Cancel"), UiMessage::Media(Msg::Delete(false))),
        )
    }

    pub(crate) fn view_quick_look<'a>(&self, title: &str, detail: &str) -> El<'a> {
        let p = &self.pal;
        let head = closable_head(p, title, 14.0, UiMessage::CloseModals);
        container(column![head, txt(detail.to_string(), 13.0).color(p.sub)].spacing(10).padding(16))
            .width(420)
            .style(shadow_box(p.elevated, p.chip, 12.0, 30.0))
            .into()
    }

    /// Desktop's filmstrip scrolls with the wheel. A step or a new item centers the current item again.
    pub(crate) fn scroll_filmstrip(&mut self, by: f32) -> Task<UiMessage> {
        let Some((items, at)) = self.carousel_items(&self.store.read()) else { return Task::none() };
        let size = self.media_window.as_ref().expect("the Media window is open").size;
        let c = self.carousel_mut().as_mut().expect("the carousel is open");
        let (start, _) = filmstrip(&size, at, items.len(), c.strip);
        c.strip = Some((start as f32 + by).round().max(0.0) as usize);
        self.media_viewer_more()
    }

    /// The square cover crop of an image or video poster, or an empty tile while it loads.
    /// Like Desktop's `MediaImg`, a media that loads has the shimmering `--skeleton-bg` over its tile, and one that
    /// failed has `ImageError`.
    pub(crate) fn thumb<'a>(&self, a: &Attachment, size: f32, r: f32) -> El<'a> {
        let p = &self.pal;
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        if let Some(tile) = url.and_then(|u| self.thumb_tile(u, size.round() as u32, r.round() as u8)) {
            return image(tile.clone()).width(size).height(size).into();
        }
        if let Some(Some(Err(_))) = url.and_then(|u| self.media.get(u)) {
            return self.image_error(size, size, r);
        }
        let loading = url.map(|_| anim::skeleton(Fill, Fill, r, p.skeleton, self.started));
        container(loading).width(size).height(size).style(fill_box(p.chip, r)).into()
    }

    /// The name Desktop shows for the sender of a message.
    fn sender_name<'s>(st: &'s BeeperStore, m: &'s Message) -> &'s str {
        match m.is_sender == Some(true) {
            true => st.me().full_name.as_deref().expect("self name"),
            false => m.sender_name.as_deref().expect("sender name"),
        }
    }
}

#[derive(Clone)]
pub enum Msg {
    /// Show an attachment of a message full screen, in the Media window with `true`.
    OpenCarousel(String, usize, bool),
    /// Show the previous (`false`) or next (`true`) image of the carousel.
    CarouselStep(bool),
    /// Show a page of the PDF of the carousel, from 1.
    PdfPage(usize),
    RotateCarousel,
    CloseCarousel,
    /// Open the carousel menu at the cursor, or close it.
    CarouselMenu(bool),
    /// Scroll the filmstrip of the Media window by a number of items.
    ScrollFilmstrip(f32),
    /// Open Desktop's Media window, or focus it.
    Open,
    /// Show the Media, Links or Docs tab of the Media window.
    Tab(usize),
    /// Load more items in the Media window.
    More,
    /// Show the Media window items that all, you or others sent.
    SentBy(u8),
    /// Show the oldest Media window items first.
    Oldest(bool),
    /// Change the text of the Media window search field, or close the search with `None`.
    Search(Option<String>),
    /// The 300 ms after a change of the search field: the search starts if the field has not changed since.
    Searched(u64),
    Menu(bool),
    /// Open the context menu of a Media window item, or close it with `None`.
    ItemMenu(Option<(String, usize)>),
    /// Close the Media window item menu and run its action.
    ItemAct(Box<UiMessage>),
    Select(bool),
    /// Select or unselect a Media window item, as (message, attachment or link index).
    Pick(String, usize),
    PickAll,
    /// Download the selected Media window attachments.
    Save,
    /// Ask to delete Media window items.
    AskDelete(Vec<(String, usize)>),
    /// Delete the items that the Media window dialog asks about, or cancel with `false`.
    Delete(bool),
}

impl BeeperGuiApp {
    pub(crate) fn media_view(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            // The Media window items show in its viewer.
            Msg::OpenCarousel(_, _, true) if !self.in_media.get() => return self.as_media(|s| s.media_view(msg)),
            Msg::OpenCarousel(mid, ai, in_media) => {
                // Desktop's `AttachmentCarousel` closes Picture in Picture when it opens.
                self.stop_pip();
                self.leave_carousel_video();
                if in_media {
                    self.media_mut().advance = false;
                }
                *self.carousel_mut() = Some(beeper_core::with_defaults!(Carousel {
                    mid,
                    ai,
                    in_media,
                    page: 1,
                }, default: turns, turned, resume, menu, strip));
                self.fit_carousel_video();
                self.reload_failed();
                return Task::batch([self.load_media(), self.media_viewer_more()]);
            }
            Msg::CarouselStep(next) => {
                let c = self.carousel().cloned().expect("the carousel is open");
                if c.in_media && !next {
                    self.media_mut().advance = false;
                }
                let Some((items, at)) = self.carousel_items(&self.store.read()) else { return Task::none() };
                // The second click of a double click on an arrow can step past the first or the last image.
                let to = if next { Some(at + 1) } else { at.checked_sub(1) };
                let Some(item) = to.and_then(|to| items.into_iter().nth(to)) else {
                    // Desktop's `viewerNext` past the last loaded item steps when the next load ends.
                    if next && self.more_media(&c) {
                        self.media_mut().advance = true;
                        return self.media_more();
                    }
                    return Task::none();
                };
                return self.carousel_to(item);
            }
            Msg::ScrollFilmstrip(by) => return self.scroll_filmstrip(by),
            Msg::PdfPage(n) => self.carousel_mut().as_mut().expect("the carousel is open").page = n,
            Msg::CarouselMenu(open) => {
                let at = self.cursor.get();
                self.carousel_mut().as_mut().expect("the carousel is open").menu = open.then_some(at);
            }
            Msg::RotateCarousel => {
                let c = self.carousel_mut().as_mut().expect("the carousel is open");
                (c.turns, c.turned) = ((c.turns + 1) % 4, Some(std::time::Instant::now()));
            }
            // Like the unmount of Desktop's carousel, the close lets go of its PDFs.
            Msg::CloseCarousel => {
                let task = self.close_carousel_video();
                if self.carousel_mut().take().is_some_and(|c| c.in_media) {
                    self.media_mut().advance = false;
                }
                self.pdfs.clear();
                return task;
            }
            msg @ (Msg::Open
            | Msg::Tab(..)
            | Msg::SentBy(..)
            | Msg::Oldest(..)
            | Msg::Search(..)
            | Msg::Searched(..)
            | Msg::Menu(..)
            | Msg::Select(..)
            | Msg::Pick(..)
            | Msg::PickAll
            | Msg::Save
            | Msg::AskDelete(..)
            | Msg::Delete(..)
            | Msg::ItemMenu(..)
            | Msg::ItemAct(..)
            | Msg::More) => return self.media_window(msg),
        }
        self.settle()
    }

    /// The Media window: its tabs, filters, search, selection and items.
    fn media_window(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open => {
                if let Some(w) = &self.media_window {
                    return self.raise(w.id);
                }
                // Desktop's `media-viewer` window state, and `center: "parent"` of its 1000 by 720 features.
                const FEATURES: Size = Size::new(1000.0, 720.0);
                let settings = self.kept_size("media-viewer", FEATURES);
                let (size, (id, open)) = (settings.size, self.open_window(settings));
                self.media_window = Some(MediaWindow::new(id, size));
                let parent = self.main_window.map(|main| (main, self.pane.size * self.zoom));
                let opened = open.then(move |id| {
                    let center = parent.map(|(main, around)| windows::center_over(id, main, around, FEATURES));
                    Task::done(UiMessage::Window(windows::Msg::Opened(id))).chain(center.unwrap_or_else(Task::none))
                });
                return Task::batch([opened, self.media_more()]);
            }
            Msg::Tab(tab) => {
                let w = self.media_mut();
                if w.tab != tab {
                    (w.tab_at, w.tab) = (Some((w.tab, std::time::Instant::now())), tab);
                    return self.media_reset();
                }
            }
            Msg::SentBy(n) => {
                let w = self.media_mut();
                w.menu = None;
                if w.sent_by != n {
                    w.sent_by = n;
                    return self.media_reset();
                }
            }
            Msg::Oldest(on) => {
                let w = self.media_mut();
                w.menu = None;
                if w.oldest != on {
                    w.oldest = on;
                    return self.media_reset();
                }
            }
            Msg::Search(q) => {
                let w = self.media_mut();
                let open = q.is_some() && w.search.is_none();
                (w.search, w.typed) = (q, w.typed + 1);
                // Desktop closes the search at once, and searches 300 ms after the last change of the field.
                let search = match w.search.is_some() {
                    true => {
                        let searched = UiMessage::Media(Msg::Searched(w.typed));
                        self.timers.media.after(std::time::Duration::from_millis(300), searched)
                    }
                    false => {
                        self.timers.media.cancel();
                        self.media_search()
                    }
                };
                return match open {
                    true => Task::batch([search, iced::widget::operation::focus("media-search")]),
                    false => search,
                };
            }
            Msg::Searched(n) => {
                // The window can close before the wait ends.
                if self.media_window.as_ref().is_some_and(|w| w.typed == n) {
                    return self.media_search();
                }
            }
            Msg::Menu(open) => {
                let at = self.cursor.get();
                self.media_mut().menu = open.then_some(at);
            }
            Msg::Select(on) => {
                let w = self.media_mut();
                w.select = on;
                w.picked.clear();
            }
            Msg::Pick(mid, k) => {
                let w = self.media_mut();
                w.select = true;
                match w.picked.iter().position(|(m, i)| *m == mid && *i == k) {
                    Some(at) => _ = w.picked.remove(at),
                    None => w.picked.push((mid, k)),
                }
            }
            Msg::PickAll => {
                let w = self.media_mut();
                w.picked = w.items.iter().map(|(m, k)| (m.id.clone(), *k)).collect();
            }
            Msg::Save => {
                let w = self.media_window.as_ref().expect("the Media window is open");
                let files = if w.tab == 1 { Vec::new() } else { w.picked.clone() };
                return Task::batch(
                    files.into_iter().map(|(mid, k)| self.update(UiMessage::Message(message::Msg::Download(mid, Some(k))))),
                );
            }
            Msg::AskDelete(items) => {
                let st = self.store.read();
                let w = self.media_window.as_ref().expect("the Media window is open");
                let items: Vec<&Arc<Message>> = items.iter().filter_map(|(mid, _)| w.message(mid)).collect();
                let mut pairs: Vec<(String, String)> = Vec::new();
                for m in &items {
                    if deletable(&st, m) && !pairs.iter().any(|(_, x)| *x == m.id) {
                        pairs.push((m.chat_id.clone(), m.id.clone()));
                    }
                }
                let skipped = items.iter().filter(|m| !deletable(&st, m)).count();
                drop(st);
                match pairs.is_empty() {
                    true => self.status_banner = tr("These messages can’t be deleted").into(),
                    false => self.media_mut().deleting = Some((pairs, skipped)),
                }
            }
            Msg::Delete(go) => {
                let (pairs, skipped) = self.media_mut().deleting.take().expect("the delete dialog is open");
                if go {
                    let mut st = self.store.write();
                    for (cid, mid) in &pairs {
                        st.redact_message(cid, mid).expect("delete message");
                    }
                    drop(st);
                    if skipped > 0 {
                        self.status_banner =
                            trn("Skipped %d item that can’t be deleted", "Skipped %d items that can’t be deleted", skipped, &[&skipped]);
                    }
                    // Desktop's `removeItems`: the viewer of a deleted item shows the item that takes its place.
                    let gone = |m: &str| pairs.iter().any(|(_, x)| x == m);
                    let w = self.media_mut();
                    let shown = w.viewer.as_ref().map(|c| (c.mid.clone(), c.ai));
                    let at = shown.and_then(|(mid, ai)| w.items.iter().position(|(m, k)| m.id == mid && *k == ai));
                    let at = at.filter(|&at| gone(&w.items[at].0.id));
                    let before = at.map_or(0, |at| w.items[..at].iter().filter(|(m, _)| gone(&m.id)).count());
                    w.items.retain(|(m, _)| !gone(&m.id));
                    w.picked.retain(|(m, _)| !gone(m));
                    let last = w.items.len().saturating_sub(1);
                    let next = at.map(|at| w.items.get((at - before).min(last)).map(|(m, k)| (m.id.clone(), *k)));
                    match next {
                        Some(Some(item)) => return self.as_media(|s| s.carousel_to(item)),
                        Some(None) => return self.as_media(|s| s.media_view(Msg::CloseCarousel)),
                        None => {}
                    }
                }
            }
            Msg::ItemMenu(item) => {
                let at = self.cursor.get();
                self.media_mut().item_menu = item.map(|(mid, k)| (mid, k, at));
            }
            Msg::ItemAct(msg) => {
                self.media_mut().item_menu = None;
                return self.update(*msg);
            }
            Msg::More => return self.media_more(),
            _ => unreachable!("not a media_window message"),
        }
        self.settle()
    }

    /// Shows the carousel item `(mid, ai)` from the start.
    fn carousel_to(&mut self, (mid, ai): (String, usize)) -> Task<UiMessage> {
        self.leave_carousel_video();
        let c = self.carousel_mut().as_mut().expect("the carousel is open");
        (c.mid, c.ai, c.resume, c.menu, c.strip, c.page) = (mid, ai, None, None, None, 1);
        self.fit_carousel_video();
        self.reload_failed();
        Task::batch([self.load_media(), self.media_viewer_more()])
    }

    /// Desktop's carousel shows its item in a new `<img>`, which loads a media that failed to load again.
    fn reload_failed(&mut self) {
        let c = self.carousel().expect("the carousel is open");
        let url = {
            let st = self.store.read();
            let shown = |a: &Attachment| a.poster_img.clone().or_else(|| a.src_url.clone());
            st.stored_message(&c.mid).and_then(|m| m.attachments.get(c.ai).and_then(shown))
        };
        if let Some(url) = url.filter(|u| self.media.get(u).is_some_and(|m| m.as_ref().is_some_and(Result::is_err))) {
            self.media.remove(&url);
        }
    }
}

#[cfg(test)]
mod ffmpeg_tests {
    /// An MP4 with its `moov` atom at the end, as ffmpeg writes it by default, still gives its first frame.
    #[test]
    fn a_video_with_its_index_at_the_end_decodes() {
        let path = std::env::temp_dir().join(format!("beeper-rs-moov-end-{}.mp4", std::process::id()));
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=1:size=64x48:rate=10"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&path)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return eprintln!("no ffmpeg to make the test video");
        }
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let moov = bytes.windows(4).position(|w| w == b"moov").unwrap();
        let mdat = bytes.windows(4).position(|w| w == b"mdat").unwrap();
        assert!(moov > mdat, "the index comes after the media");
        let frames = super::decode_media(bytes).expect("the first frame");
        assert_eq!(frames.len(), 1);
    }

    /// A HEIC photo, whose `ftyp` box ffmpeg 6 takes for an MP4 that it cannot read, shows like Desktop's transcode.
    #[test]
    fn a_heic_photo_decodes_without_ffmpeg() {
        let bytes = include_bytes!("../../beeper-core/tests/heic/rotated.heic").to_vec();
        let frames = super::decode_media(bytes).expect("the photo");
        assert_eq!(frames.len(), 1);
        let iced::widget::image::Handle::Rgba { width, height, .. } = &frames[0].0 else { panic!("decoded pixels") };
        assert_eq!((*width, *height), (64, 32), "upright by its rotation");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use beeper_core::config::rfc3339_from_millis;
    use iced_test::simulator::{simulator, Simulator};

    /// Adds `n` images with the captions of `text` to the note to self, a minute apart in 2027, the last one newest.
    fn photos(app: &BeeperGuiApp, n: usize, text: impl Fn(usize) -> Option<&'static str>) {
        let note = "!note_self:beeper.com";
        let mut st = app.store.write();
        let base = st.messages[*st.timeline_indices(note).last().expect("a note")].clone();
        for i in 0..n {
            let a =
                Attachment { attachment_type: "img".into(), src_url: Some(format!("https://example.com/{i}.png")), ..Default::default() };
            let timestamp = rfc3339_from_millis(1_800_000_000_000 + i as i64 * 60_000);
            let (id, sort_key, text) = (format!("$page{i:03}"), format!("{:016}", 9_000_000 + i), text(i));
            let links = Vec::new();
            let text = text.map(Into::into);
            st.upsert_message(Message { id, sort_key, timestamp, attachments: vec![a], links, text, ..base.clone() });
        }
    }

    fn ids(app: &BeeperGuiApp) -> Vec<String> {
        app.media_window.as_ref().expect("the Media window is open").items.iter().map(|(m, _)| m.id.clone()).collect()
    }

    fn page(range: impl DoubleEndedIterator<Item = usize>) -> Vec<String> {
        range.rev().map(|i| format!("$page{i:03}")).collect()
    }

    /// Desktop's Media window loads 50 messages at a time, newest first by time, shows skeleton rows while it loads
    /// and loads more 8 rows before the end of the list. A filter change loads again from the start, and drops the
    /// loads for the filters before it.
    #[test]
    fn media_window_pages_like_desktop() {
        let mut app = demo_app("media-pages");
        photos(&app, 120, |_| None);
        let _ = app.update(UiMessage::Media(Msg::Open));
        let id = app.media_window.as_ref().unwrap().id;
        let size = Size::new(1000.0, 720.0);
        let _ = app.update(UiMessage::Window(windows::Msg::Resized(id, size)));
        assert!(app.media_window.as_ref().unwrap().loading.is_some(), "the window loads when it opens");
        assert!(simulator(app.view(id)).find(tr("Nothing here")).is_err(), "skeleton rows show while it loads");
        finish_media_load(&mut app);
        assert_eq!(ids(&app), page(70..120), "the first load has the newest 50 messages");
        let w = app.media_window.as_ref().unwrap();
        assert!(w.more && w.loading.is_none());
        let more = {
            let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
            let _ = sim.simulate([iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))]);
            sim.into_messages().any(|m| matches!(m, UiMessage::Media(Msg::More)))
        };
        assert!(more, "nine rows of tiles are near the end of the list");
        let _ = app.update(UiMessage::Media(Msg::More));
        finish_media_load(&mut app);
        assert_eq!(ids(&app)[50..70], page(50..70)[..], "the next load has the next 50");
        let stale = media_page(&app.store.read(), &MediaAsk::of(app.media_window.as_ref().unwrap()));
        let _ = app.update(UiMessage::Media(Msg::Oldest(true)));
        let _ = app.update(UiMessage::Loaded(loads::Msg::MediaPage(id, stale)));
        assert!(ids(&app).is_empty(), "a load for the filters before does nothing");
        load_all_media(&mut app);
        let w = app.media_window.as_ref().unwrap();
        assert!(w.items.windows(2).all(|p| iso_millis(&p[0].0.timestamp) <= iso_millis(&p[1].0.timestamp)));
        let oldest: Vec<String> = page(0..120).into_iter().rev().collect();
        assert!(ids(&app).ends_with(&oldest), "oldest first");
    }

    /// Desktop's Media window viewer steps past the last loaded item when the next load ends.
    #[test]
    fn media_viewer_waits_for_the_next_load_like_desktop() {
        let mut app = demo_app("media-advance");
        photos(&app, 60, |_| None);
        let _ = app.update(UiMessage::Media(Msg::Open));
        finish_media_load(&mut app);
        let last = ids(&app).pop().expect("50 items");
        let _ = app.update(UiMessage::Media(Msg::OpenCarousel(last.clone(), 0, true)));
        assert!(app.media_window.as_ref().unwrap().loading.is_some(), "the filmstrip loads more near the end");
        let id = app.media_window.as_ref().unwrap().id;
        let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::Media(Msg::CarouselStep(true)))));
        let viewer = |app: &BeeperGuiApp| app.media_window.as_ref().unwrap().viewer.as_ref().unwrap().mid.clone();
        assert_eq!(viewer(&app), last, "the viewer waits");
        finish_media_load(&mut app);
        assert_eq!(viewer(&app), "$page009", "and steps when the load ends");
    }

    /// The message of a click on the free space of the carousel of window `id` of `size`, which closes it.
    fn backdrop(app: &BeeperGuiApp, id: window::Id, size: Size) -> Vec<UiMessage> {
        let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
        sim.point_at(Point::new(4.0, size.height / 2.0));
        let _ = sim.simulate(iced_test::simulator::click());
        sim.into_messages().collect()
    }

    fn closes(m: &UiMessage) -> bool {
        matches!(m, UiMessage::Media(Msg::CloseCarousel))
    }

    /// Desktop's `carouselState` of each window: the carousel of the main window, that of a chat window and the Media
    /// window viewer are open at once, each window draws its own, and each steps and closes on its own.
    #[test]
    fn each_window_has_its_carousel_like_desktop() {
        let mut app = demo_app("window-carousels");
        photos(&app, 3, |_| None);
        let note = "!note_self:beeper.com";
        let _ = app.update(UiMessage::SelectChat(note.into()));
        let _ = app.update(UiMessage::Media(Msg::OpenCarousel("$page000".into(), 0, false)));
        let _ = app.update(UiMessage::Window(windows::Msg::OpenChat(note.into())));
        let chat = app.chat_windows[0].0;
        let in_chat = |app: &mut BeeperGuiApp, m| drop(app.update(UiMessage::InWindow(chat, Box::new(UiMessage::Media(m)))));
        in_chat(&mut app, Msg::OpenCarousel("$page002".into(), 0, false));
        let _ = app.update(UiMessage::Media(Msg::Open));
        let media = app.media_window.as_ref().unwrap().id;
        load_all_media(&mut app);
        let _ = app.update(UiMessage::Media(Msg::OpenCarousel("$page001".into(), 0, true)));
        let shown = |app: &BeeperGuiApp| {
            let viewer = &app.media_window.as_ref().unwrap().viewer;
            [&app.pane.carousel, &app.chat_windows[0].1.carousel, viewer].map(|c| c.as_ref().map(|c| c.mid.clone()))
        };
        let mids = |m: [Option<&str>; 3]| m.map(|m| m.map(String::from));
        assert_eq!(shown(&app), mids([Some("$page000"), Some("$page002"), Some("$page001")]), "all three are open");
        let (main, size) = (app.main_window.unwrap(), Size::new(1000.0, 720.0));
        assert!(backdrop(&app, main, size).iter().any(closes), "the main window draws its carousel");
        let mine = |m: &UiMessage, id| matches!(m, UiMessage::InWindow(w, m) if *w == id && closes(m));
        assert!(backdrop(&app, chat, size).iter().any(|m| mine(m, chat)), "the chat window draws its own");
        assert!(backdrop(&app, media, size).iter().any(|m| mine(m, media)), "and so does the Media window");
        let _ = app.update(UiMessage::Media(Msg::CarouselStep(true)));
        let after = shown(&app);
        assert!(after[0] != Some("$page000".into()) && after[1..] == shown(&app)[1..], "a step moves one carousel");
        assert_eq!(after[1..], mids([None, Some("$page002"), Some("$page001")])[1..]);
        let _ = app.update(UiMessage::WindowKey(media, Key::Named(Named::Escape), keyboard::Modifiers::empty()));
        assert_eq!(shown(&app)[1..], mids([None, Some("$page002"), None])[1..], "Escape closes only the viewer");
        assert!(shown(&app)[0].is_some());
        in_chat(&mut app, Msg::CloseCarousel);
        assert!(shown(&app)[0].is_some() && shown(&app)[1].is_none(), "the chat window closes its own");
    }

    /// Desktop's thread window shows its carousel in its own "Media in %s" window, at least 800 by 600, which a
    /// carousel of the thread window reuses. Its Reply starts the reply in the thread window and closes it, and it
    /// has no Forward….
    #[test]
    fn thread_windows_open_media_in_windows_like_desktop() {
        let mut app = demo_app("thread-media");
        photos(&app, 2, |_| None);
        let note = "!note_self:beeper.com";
        let _ = app.update(UiMessage::Thread(crate::threads::Msg::OpenWindow(note.into(), "$page000".into())));
        let thread = app.chat_windows[0].0;
        let open = |app: &mut BeeperGuiApp, mid: &str| {
            let open = UiMessage::Media(Msg::OpenCarousel(mid.into(), 0, false));
            drop(app.update(UiMessage::InWindow(thread, Box::new(open))));
        };
        open(&mut app, "$page001");
        assert!(app.chat_windows[0].1.carousel.is_none(), "the thread window shows no carousel of its own");
        assert_eq!(app.carousel_windows.len(), 1);
        let (id, opener) = app.carousel_windows[0];
        assert_eq!(opener, thread);
        fn pane(app: &BeeperGuiApp, id: window::Id) -> &crate::Pane {
            &app.chat_windows.iter().find(|w| w.0 == id).expect("the window").1
        }
        assert_eq!(pane(&app, id).size, Size::new(800.0, 700.0), "at least 800 by 600 and the size of the thread window");
        let title = {
            let st = app.store.read();
            st.chats[st.resolve_chat_idx(note).unwrap()].title.clone()
        };
        assert_eq!(app.title(id), format!("Media in {title}"));
        open(&mut app, "$page000");
        assert_eq!(app.carousel_windows.len(), 1, "the next carousel shows in the same window");
        assert_eq!(pane(&app, id).carousel.as_ref().unwrap().mid, "$page000");
        app.cursor.set(Point::new(300.0, 300.0));
        let _ = app.update(UiMessage::InWindow(id, Box::new(UiMessage::Media(Msg::CarouselMenu(true)))));
        let mut sim = Simulator::with_size(Default::default(), pane(&app, id).size, app.view(id));
        assert!(sim.find("Forward…").is_err(), "no Forward…");
        let _ = sim.click("Reply…").expect("Reply…");
        for m in sim.into_messages().collect::<Vec<_>>() {
            let _ = app.update(m);
        }
        assert!(pane(&app, id).carousel.is_none(), "Reply closes the carousel, and its window with it");
        assert_eq!(app.chat_windows[0].1.reply_to_message_id.as_deref(), Some("$page000"), "the thread window replies");
        let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
        assert!(app.carousel_windows.is_empty() && app.chat_windows.len() == 1);
        open(&mut app, "$page001");
        let id = app.carousel_windows[0].0;
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), keyboard::Modifiers::empty()));
        assert!(pane(&app, id).carousel.is_none(), "Escape closes the carousel of the window");
    }

    /// Desktop's `renderAttachment`: an item that the bridge loads only when the user asks is the file row that asks
    /// for it, and an image that failed to load is Desktop's `ImageError`, not empty space.
    #[test]
    fn carousel_loading_states_like_desktop() {
        let mut app = demo_app("carousel-states");
        photos(&app, 2, |_| None);
        let (url, broken) = ("https://example.com/0.png".to_string(), "https://example.com/1.png".to_string());
        {
            let mut st = app.store.write();
            let i = st.message_idx("$page000");
            st.update_message(i, |m| m.attachments[0].file_name = Some("phone.jpg".into()));
        }
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let reload = format!("{} (HTTP 502): the phone has it", beeper_core::matrix::MEDIA_RELOAD_NEEDED);
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(url.clone(), Err(reload))));
        let _ = app.update(UiMessage::Media(Msg::OpenCarousel("$page000".into(), 0, false)));
        let fetches = |app: &BeeperGuiApp| {
            let mut sim = simulator(app.view_carousel(app.pane.carousel.as_ref().expect("the carousel is open")).unwrap());
            let _ = sim.click("phone.jpg");
            let fetch = |m| matches!(m, UiMessage::Loaded(loads::Msg::Fetch(ref u)) if *u == url);
            sim.into_messages().any(fetch)
        };
        assert!(fetches(&app), "the file row asks for the media");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Fetched(url.clone(), Ok(()))));
        assert!(!fetches(&app), "a downloaded image shows");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(broken.clone(), Err("HTTP 500".into()))));
        let _ = app.update(UiMessage::Media(Msg::CarouselStep(true)));
        assert!(!matches!(app.media.get(&broken), Some(Some(Err(_)))), "the carousel loads a failed image again");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(broken, Err("HTTP 500".into()))));
        assert!(app.needs_download.is_empty(), "another error needs no download");
        let car = app.pane.carousel.clone().expect("the carousel is open");
        assert!(simulator(app.view_carousel(&car).unwrap()).find("phone.jpg").is_err(), "the next item shows");
    }

    /// Desktop's timeline mounts new `<img>`s when a chat opens, which load a media that failed again; a media that
    /// waits for the user to ask for it keeps its download button.
    #[test]
    fn an_opened_chat_loads_failed_media_again_like_desktop() {
        let mut app = demo_app("failed-media-again");
        let (ok, broken, phone) = ("mxc://x/ok", "mxc://x/broken", "mxc://x/phone");
        let reload = format!("{} (HTTP 400): the phone has it", beeper_core::matrix::MEDIA_RELOAD_NEEDED);
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(ok.into(), Ok(Frames::from([])))));
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(broken.into(), Err("HTTP 502".into()))));
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(phone.into(), Err(reload))));
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        assert!(matches!(app.media.get(ok), Some(Some(Ok(_)))), "a loaded media stays");
        assert!(!app.media.contains_key(broken), "a failed media loads again");
        assert!(matches!(app.media.get(phone), Some(Some(Err(_)))) && app.needs_download.contains(phone));
    }

    /// Desktop's caption search: 300 ms after the last change of the field, the message search with partial matches
    /// over the captions and the file names. Closing the search shows all the items at once.
    #[test]
    fn media_window_searches_captions_like_desktop() {
        let mut app = demo_app("media-search");
        photos(&app, 3, |i| [Some("Sunset at the beach"), Some("a sunny day"), None][i]);
        let _ = app.update(UiMessage::Media(Msg::Open));
        let id = app.media_window.as_ref().unwrap().id;
        load_all_media(&mut app);
        let all = ids(&app);
        let typed = |app: &mut BeeperGuiApp, q: &str| {
            let _ = app.update(UiMessage::Media(Msg::Search(Some(q.into()))));
            app.media_window.as_ref().unwrap().typed
        };
        let _ = typed(&mut app, "");
        let early = typed(&mut app, " su");
        let last = typed(&mut app, " sun ");
        let _ = app.update(UiMessage::Media(Msg::Searched(early)));
        assert_eq!(app.media_window.as_ref().unwrap().query, "", "a change of the field waits for the next");
        let _ = app.update(UiMessage::Media(Msg::Searched(last)));
        assert_eq!(app.media_window.as_ref().unwrap().query, "sun", "the search takes the trimmed text");
        load_all_media(&mut app);
        let found = ids(&app);
        assert!(found.starts_with(&page(0..2)), "a caption word starts with the text: {found:?}");
        assert!(!found.contains(&"$page002".into()));
        let last = typed(&mut app, "unny");
        let _ = app.update(UiMessage::Media(Msg::Searched(last)));
        load_all_media(&mut app);
        assert!(simulator(app.view(id)).find(tr("Nothing here")).is_ok(), "the search matches the start of words");
        let _ = app.update(UiMessage::Media(Msg::Search(None)));
        load_all_media(&mut app);
        assert_eq!(ids(&app), all, "closing the search shows all items");
        {
            let mut st = app.store.write();
            let i = st.find_message("$page002").expect("a photo");
            st.update_message(i, |m| {
                let a = &mut m.attachments[0];
                (a.attachment_type, a.file_name) = ("unknown".into(), Some("quokka-ledger.pdf".into()));
            });
        }
        let _ = app.update(UiMessage::Media(Msg::Tab(2)));
        let last = typed(&mut app, "ledg");
        let _ = app.update(UiMessage::Media(Msg::Searched(last)));
        load_all_media(&mut app);
        assert_eq!(ids(&app), ["$page002"], "the search finds file names");
    }

    /// The squares that the Media window draws for its tiles that have no image yet, and its list.
    fn grid_squares(app: &BeeperGuiApp, size: Size) -> (Vec<iced::Rectangle>, iced::Rectangle) {
        let id = app.media_window.as_ref().unwrap().id;
        let mut sim = Simulator::with_size(Default::default(), size, app.view(id));
        let (mut squares, mut list) = (Vec::new(), None);
        let _ = sim.find(|c: iced_test::selector::Candidate<'_>| -> Option<()> {
            match c {
                iced_test::selector::Candidate::Container { bounds, .. }
                    if bounds.width > 60.0 && (bounds.width - bounds.height).abs() < 0.01 =>
                {
                    squares.push(bounds)
                }
                iced_test::selector::Candidate::Scrollable { bounds, .. } if list.is_none() => list = Some(bounds),
                _ => {}
            }
            None
        });
        // A tile has a few boxes of its size.
        squares.dedup_by(|a, b| a == b);
        (squares, list.expect("the list"))
    }

    /// Like Desktop's `repeat(n, 1fr)` rows, every tile of the Media window has the same size at a width that does
    /// not divide evenly, and the rows end 10 px before the scrollbar when they overflow the list, or before the
    /// edge when they do not.
    #[test]
    fn media_tiles_share_the_row_like_desktop() {
        let mut app = demo_app("media-grid");
        photos(&app, 40, |_| None);
        let _ = app.update(UiMessage::Media(Msg::Open));
        let id = app.media_window.as_ref().unwrap().id;
        finish_media_load(&mut app);
        for (size, overflows) in [(Size::new(990.0, 700.0), true), (Size::new(990.0, 2400.0), false)] {
            let _ = app.update(UiMessage::Window(windows::Msg::Resized(id, size)));
            let (squares, list) = grid_squares(&app, size);
            assert!(squares.len() >= 6, "the tiles show: {squares:?}");
            let w = squares[0].width;
            assert!(squares.iter().all(|b| (b.width - w).abs() < 0.01), "one size: {squares:?}");
            assert_eq!(squares.iter().filter(|b| (b.y - squares[0].y).abs() < 0.01).count(), 6, "six columns");
            let right = squares.iter().map(|b| b.x + b.width).fold(0.0, f32::max);
            let gutter = if overflows { GUTTER } else { 0.0 };
            assert!((list.x + list.width - gutter - 10.0 - right).abs() < 0.01, "{size:?}: {right} in {list:?}");
            assert!((media_grid(list.size(), &[40], 0).1 - w).abs() < 0.01, "the grid of the list");
        }
    }

    /// Desktop's skeleton tiles pulse from 0.55 to full opacity and back in 1.2 s.
    #[test]
    fn skeleton_tiles_pulse_like_desktop() {
        let round = |t: f32| (pulse(t) * 100.0).round() / 100.0;
        assert_eq!([0.0, 0.6, 1.2, 1.8].map(round), [0.55, 1.0, 0.55, 1.0]);
        assert!([0.3, 0.9].iter().all(|&t| (pulse(t) - 0.775).abs() < 1e-3), "halfway at a quarter and three quarters");
    }
}

//! The Media window, the carousel of a chat, and the decode and the load of the media that they show.

use crate::forward;
use crate::i18n::{tr, trn, trx};
use crate::style::{
    bar, bordered_box, btn_style, ell, fill_box, ic, icon, input_style, med, scroll_style, semi, shadow_box,
    text_width, txt, Bare, CLEAR,
};
use crate::{
    anim, closable_head, cubic_bezier, day_num, deletable, file_kind, format_short_ts, human_size, i18n, iso_millis,
    menu_item, menu_rule, relative_date, stored, tinted, today, video, vlist, voice, zoom, BeeperGuiApp, El, Frames,
    PillLook, UiMessage, AVATAR_SLOTS, MEDIA_ICONS, MENU_ITEM, ROW_ACTIONS, SYS_ARROW_BACK, SYS_DELETE, SYS_DOWNLOAD,
    SYS_EMOJI, SYS_REPLY, SYS_ROTATE,
};
use beeper_core::config::civil_from_days;
use beeper_core::models::*;
use beeper_core::store::{BeeperStore, Query};
use iced::font::Weight;
use iced::keyboard::{self, key::Named, Key};
use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, stack, svg, text_input, tooltip, Column, Row, Space,
};
use iced::{border, mouse, window, Center, Color, Fill, Padding, Pixels, Point, Right, Size, Task, Theme};
use std::rc::Rc;

pub(crate) fn run_ffmpeg(bytes: &[u8], args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("ffmpeg")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg("pipe:0")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    std::thread::scope(|s| {
        s.spawn(move || {
            let _ = stdin.write_all(bytes);
        });
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        match out.status.success() && !out.stdout.is_empty() {
            true => Ok(out.stdout),
            false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        }
    })
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
        false => Ok(frames),
    }
}

pub(crate) fn decode_media(bytes: Vec<u8>) -> Result<Frames, String> {
    use ::image::AnimationDecoder;
    // A decode here, on a worker thread, keeps the full size decode out of the first frame that draws the image.
    let still = |bytes: &[u8]| -> Result<Frames, String> {
        let px = ::image::load_from_memory(bytes).map_err(|e| format!("Could not decode the image: {e}"))?;
        let px = px.into_rgba8();
        Ok(vec![(image::Handle::from_rgba(px.width(), px.height(), px.into_raw()), 0)])
    };
    if bytes.get(4..8) == Some(b"ftyp") {
        return still(&run_ffmpeg(&bytes, &["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])?);
    }
    if !bytes.starts_with(b"GIF8") {
        return still(&bytes);
    }
    let gif = ::image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let frames = gif.into_frames().collect_frames().map_err(|e| format!("Could not decode the GIF: {e}"))?;
    assert!(!frames.is_empty(), "a decoded GIF has frames");
    let frame = |f: ::image::Frame| {
        let (n, d) = f.delay().numer_denom_ms();
        let px = f.into_buffer();
        // Browsers play delays of 10 ms or less at 100 ms.
        let ms = match n / d {
            0..=10 => 100,
            ms => ms,
        };
        (image::Handle::from_rgba(px.width(), px.height(), px.into_raw()), u128::from(ms))
    };
    Ok(frames.into_iter().map(frame).collect())
}

/// The attachment that a carousel shows, its quarter turns and when the last one started, if it shows the Media window items, the inline size of
/// the video that opened it, which plays again when the carousel closes on it, where its menu is open, and the first
/// item of the filmstrip after a scroll.
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
}

/// Desktop's Media window: its tab (Media, Links or Docs), its size, how many items it shows,
/// its filters (sent by all, you or others; oldest first; the caption search), and if its filter menu is open.
pub(crate) struct MediaWindow {
    pub(crate) id: window::Id,
    pub(crate) tab: usize,
    pub(crate) size: Size,
    pub(crate) shown: usize,
    pub(crate) sent_by: u8,
    pub(crate) oldest: bool,
    pub(crate) search: Option<String>,
    /// The tab before the last tab change, and when the change was.
    pub(crate) tab_at: Option<(usize, std::time::Instant)>,
    /// Where the filter menu is open.
    pub(crate) menu: Option<Point>,
    /// The item whose context menu is open, as (message, attachment or link index), and where.
    pub(crate) item_menu: Option<(String, usize, Point)>,
    pub(crate) select: bool,
    pub(crate) picked: Vec<(String, usize)>,
    /// The (chat, message) pairs that the delete dialog asks about, and how many items it skips.
    pub(crate) deleting: Option<(Vec<(String, String)>, usize)>,
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

/// Desktop's Media window page size.
pub(crate) const MEDIA_PAGE: usize = 50;

const MEDIA_TABS: [&str; 3] = ["Media", "Links", "Docs"];

/// The items of the Media window tab, as (index of `media`, attachment or link index), in its order. `media` holds the
/// stored messages with media. Like Desktop, it skips low priority chats and message requests.
pub(crate) fn media_items(st: &BeeperStore, media: &[Message], w: &MediaWindow) -> Vec<(usize, usize)> {
    let (tab, needle) = (w.tab, w.search.as_deref().unwrap_or("").trim().to_lowercase());
    let shown = |m: &&Message| {
        m.is_deleted != Some(true)
            && st.chat(&m.chat_id).flags & (CHAT_LOW_PRIORITY | CHAT_REQUEST) == 0
            && match w.sent_by {
                0 => true,
                1 => m.is_sender == Some(true),
                2 => m.is_sender != Some(true),
                n => unreachable!("sent by {n}"),
            }
            && m.text.as_deref().unwrap_or("").to_lowercase().contains(&needle)
    };
    let mut items: Vec<_> = media
        .iter()
        .enumerate()
        .filter(|(_, m)| shown(m))
        .flat_map(|(i, m)| {
            let parts: Vec<usize> = match tab {
                1 => (0..m.links.len()).collect(),
                _ => {
                    let kinds: &[&str] = if tab == 0 { &["img", "video"] } else { &["unknown"] };
                    let atts = m.attachments.iter().enumerate();
                    atts.filter(|(_, a)| kinds.contains(&a.attachment_type.as_str())).map(|(k, _)| k).collect()
                }
            };
            let t = iso_millis(&m.timestamp);
            parts.into_iter().map(move |k| (t, i, k))
        })
        .collect();
    items.sort_by_key(|&(t, ..)| if w.oldest { t } else { -t });
    items.into_iter().map(|(_, i, k)| (i, k)).collect()
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

    /// Show the first page of the Media window items after a filter changes.
    pub(crate) fn media_reload(&mut self) -> Task<UiMessage> {
        self.media_mut().shown = MEDIA_PAGE;
        self.load_media()
    }

    /// Starts the loads of the media that a view asked for, when a client or HTTPS can load them.
    pub(crate) fn load_media(&mut self) -> Task<UiMessage> {
        let client = self.store.read().matrix.clone();
        let loadable = |u: &String| client.is_some() || u.starts_with("https://") || u.starts_with("data:");
        let (urls, rest) = std::mem::take(&mut *self.wants.media.borrow_mut()).into_iter().partition(loadable);
        *self.wants.media.borrow_mut() = rest;
        let urls: Vec<String> = urls.into_iter().filter(|u| !self.media.contains_key(u)).collect();
        self.media.extend(urls.iter().map(|u| (u.clone(), None)));
        let audio = Task::batch([self.load_audio(), self.auto_transcribe()]);
        let bookmarks = self.load_bookmarks();
        let tiles = self.bake_tiles();
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (client.clone(), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the media semaphore stays open");
                    let (u, looped) = u.strip_suffix(LOOP).map_or((u.clone(), false), |u| (u.to_string(), true));
                    let bytes = beeper_core::matrix::media(client, u).await?;
                    let decode = move || if looped { decode_loop(&bytes) } else { decode_media(bytes) };
                    tokio::task::spawn_blocking(decode).await.expect("media decode task")
                },
                move |r| UiMessage::MediaLoaded(url, r),
            )
        }))
        .chain(audio)
        .chain(bookmarks)
        .chain(tiles)
    }

    /// The stored messages with media for the Media window, newest first. The memo keeps them until the store changes.
    pub(crate) fn media_messages(&self, st: &BeeperStore) -> Rc<[Message]> {
        let memo = self.memo();
        if let Some(all) = memo.and_then(|m| m.media.borrow().clone()) {
            return all;
        }
        let mut all = Vec::new();
        st.scan(&Query::default().media(), |m| {
            all.push(m);
            true
        });
        let all: Rc<[Message]> = all.into();
        if let Some(m) = memo {
            *m.media.borrow_mut() = Some(all.clone());
        }
        all
    }

    /// The (message, attachment index) items of the carousel, and the position of the shown one: like Desktop's
    /// `getCarouselAttachments`, the attachments of the open chat other than audio, oldest first, or the Media window
    /// items. It is `None` when the shown item is gone: a sync deleted its message, or its chat left the Media window.
    pub(crate) fn carousel_items(&self, st: &BeeperStore) -> Option<(Vec<(String, usize)>, usize)> {
        let c = self.carousel.as_ref().expect("the carousel is open");
        let items: Vec<_> = match c.in_media {
            true => {
                let media = self.media_messages(st);
                let items = media_items(st, &media, self.media_window.as_ref().expect("the Media window is open"));
                items.into_iter().map(|(i, k)| (media[i].id.clone(), k)).collect()
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

    /// The source and the size of the video that the open carousel shows, if it shows a video that is not a GIF. Desktop
    /// shows the video at most at its own size, `100vw - 100px` wide and `90vh` high.
    pub(crate) fn carousel_video(&self) -> Option<(String, (u32, u32))> {
        let c = self.carousel.as_ref()?;
        let st = self.store.read();
        let m = st.stored_message(&c.mid)?;
        let a = m.attachments.get(c.ai)?;
        let src = a.src_url.clone().filter(|_| a.attachment_type == "video" && a.is_gif != Some(true))?;
        let area = match c.in_media {
            true => self.media_window.as_ref().expect("the Media window is open").size,
            false => self.pane.size,
        };
        let size = a.size.as_ref();
        let (w, h) = size.map_or((240.0, 160.0), |s| (s.width.unwrap_or(240) as f32, s.height.unwrap_or(160) as f32));
        let k = ((area.width - 100.0) / w).min(0.9 * area.height / h).min(1.0);
        Some((src, ((w * k).round() as u32, (h * k).round() as u32)))
    }

    pub(crate) fn carousel_key(&mut self, key: &Key, mods: keyboard::Modifiers) -> Task<UiMessage> {
        let items = self.carousel_items(&self.store.read());
        let Some((items, at)) = items else {
            return self.update(UiMessage::CloseCarousel);
        };
        let c = self.carousel.clone().expect("the carousel is open");
        let video = self.carousel_video().and_then(|(src, _)| self.video.as_ref().filter(|v| v.url == src));
        let (video, full) = (video.is_some(), video.is_some_and(video::Video::is_full));
        match key.as_ref() {
            Key::Named(Named::Escape) if full => self.video(video::Msg::Fullscreen),
            Key::Named(Named::Escape) if self.pane.emoji.is_some() => self.update(UiMessage::CloseModals),
            Key::Named(Named::Escape) if c.menu.is_some() => self.update(UiMessage::CarouselMenu(false)),
            Key::Named(Named::Escape) => self.update(UiMessage::CloseCarousel),
            Key::Character("f") if mods.is_empty() && video => self.video(video::Msg::Fullscreen),
            Key::Named(Named::ArrowLeft) if mods.is_empty() && at > 0 => self.update(UiMessage::CarouselStep(false)),
            Key::Named(Named::ArrowRight) if mods.is_empty() && at + 1 < items.len() => {
                self.update(UiMessage::CarouselStep(true))
            }
            Key::Character("d") if mods.is_empty() => self.update(UiMessage::Download(c.mid, Some(c.ai))),
            Key::Character("r") if mods.is_empty() => self.update(UiMessage::RotateCarousel),
            _ => Task::none(),
        }
    }

    /// The React and Reply actions of the chat carousel on message `m`, like Desktop's `ChatCarousel`: React opens the
    /// emoji picker 4 px below the React button, with its right edge at the right edge of the button, and Reply closes
    /// the carousel.
    fn carousel_actions(&self, car: &Carousel, m: &Message, c: &Chat) -> (Option<UiMessage>, Option<UiMessage>) {
        let live = !car.in_media && m.is_deleted != Some(true);
        let reply = (c.messages.reply && live)
            .then(|| UiMessage::Commands(vec![UiMessage::CloseCarousel, UiMessage::StartReply(m.id.clone())]));
        let right = self.pane.size.width - 8.0 - 28.0 * if reply.is_some() { 4.0 } else { 3.0 };
        let react = (c.messages.react && live).then(|| UiMessage::ReactAt(m.id.clone(), Point::new(right, 39.0)));
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
        let control =
            |h: &svg::Handle, label: &'static str, key: Vec<&'static str>, msg: Option<UiMessage>| -> El<'a> {
                let b = button(tinted(h, 20.0, white)).bare(white);
                self.tip(b.on_press_maybe(msg).into(), label, key, tooltip::Position::Bottom).into()
            };
        let action = |s| trx(s, "attachment carousel action");
        let (face, name) = match car.in_media {
            true => {
                let u = c.participants.items.iter().find(|u| u.id == m.sender_id);
                let name = Self::sender_name(&st, m);
                (u.map(|u| self.face(u.img_url.as_deref(), name, 28.0, 0.0)), name)
            }
            false => (Some(self.chat_avatar(c, 28.0, 0.0)), c.title.as_str()),
        };
        let title = row![]
            .push(face)
            .push(ell(name, 600.0, 14.0, Weight::Medium).color(white))
            .push(txt(format_short_ts(&m.timestamp, h24), 12.0).color(white.scale_alpha(0.7)))
            .spacing(8)
            .align_y(Center);
        let go = car.in_media.then(|| {
            control(
                &MEDIA_ICONS[3],
                tr("Go to Message"),
                vec![],
                Some(UiMessage::GoToMessage(c.id.clone(), mid.into())),
            )
        });
        // The Media window viewer has Delete when you can delete the message, like Desktop's `MediaViewer`.
        let delete = (car.in_media && deletable(&st, m)).then(|| {
            control(&SYS_DELETE, tr("Delete"), vec![], Some(UiMessage::MediaAskDelete(vec![(mid.into(), ai)])))
        });
        // Desktop shows Fullscreen in place of Rotate for a video.
        let turn_or_full = match &video {
            Some((src, _)) => {
                let full =
                    self.video.as_ref().filter(|v| v.url == *src).map(|_| UiMessage::Video(video::Msg::Fullscreen));
                control(&video::GLYPHS[3], tr("Fullscreen"), vec!["F"], full)
            }
            None => control(&SYS_ROTATE, action("Rotate"), vec!["R"], Some(UiMessage::RotateCarousel)),
        };
        let (react, reply) = self.carousel_actions(car, m, c);
        let react = react.map(|msg| control(&SYS_EMOJI, action("React"), vec![], Some(msg)));
        let reply = reply.map(|msg| control(&SYS_REPLY, action("Reply"), vec![], Some(msg)));
        let download = Some(UiMessage::Download(mid.to_string(), Some(ai)));
        let controls = row![]
            .push(go)
            .push(delete)
            .push(react)
            .push(reply)
            .push(turn_or_full)
            .push(control(&SYS_DOWNLOAD, action("Download"), vec!["D"], download))
            .push(control(&ROW_ACTIONS[6], action("Close"), vec!["Escape"], Some(UiMessage::CloseCarousel)))
            .spacing(8);
        let head =
            stack![container(title).center_x(Fill).center_y(34), container(controls).align_right(Fill).center_y(34)];
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        // Desktop turns the image by a quarter in `.2s` with the CSS `ease` curve.
        let quarter = -std::f32::consts::FRAC_PI_2;
        let turn = car.turned.map_or((0.0, 0.0, self.started), |at| {
            let before = f32::from((car.turns + 3) % 4);
            (quarter * before, quarter * (before + 1.0), at)
        });
        let frames = url.and_then(|u| self.media_at(u)).and_then(|f| f.as_ref().ok());
        // Desktop's `renderAttachment`: a video plays with its controls and does not zoom, and a file is a link that
        // downloads it.
        let main: El<'a> = match (a.attachment_type.as_str(), &video) {
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
            ("img" | "video", ..) => {
                let shown: El<'a> = match frames {
                    Some(frames) => {
                        let frames = anim::playing(frames, st.settings.on("autoplayGifs"));
                        anim::turning(frames, self.started, turn, |f| image(f).content_fit(iced::ContentFit::ScaleDown))
                    }
                    None => Space::new().into(),
                };
                container(zoom::zoom(shown, vlist::key((mid, ai)), UiMessage::CloseCarousel)).padding(8).into()
            }
            _ => {
                let name = txt(a.file_name.clone().expect("attachment file name"), 13.0).color(white);
                let link = button(name).bare(white);
                container(link.on_press(UiMessage::Download(mid.to_string(), Some(ai)))).center(Fill).into()
            }
        };
        let step = |next: bool| -> El<'a> {
            let arrow = svg(SYS_ARROW_BACK.clone())
                .width(20)
                .height(20)
                .rotation(if next { std::f32::consts::PI } else { 0.0 })
                .style(move |_, _| svg::Style { color: Some(white) });
            let b = button(container(arrow).center(32)).padding(0).style(btn_style(
                [Color { a: 0.1, ..p.strong }; 3],
                [white; 3],
                32.0,
            ));
            let (label, key, at) = match next {
                true => (tr("Next Attachment"), "→", tooltip::Position::Top),
                false => (tr("Previous Attachment"), "←", tooltip::Position::Bottom),
            };
            let b = self.tip(b.on_press(UiMessage::CarouselStep(next)).into(), label, vec![key], at);
            let b = container(b).center_y(Fill).padding([0, 24]);
            if next {
                b.align_right(Fill).into()
            } else {
                b.into()
            }
        };
        let strip = car.in_media.then(|| {
            let size = self.media_window.as_ref().expect("the Media window is open").size;
            let (start, end) = filmstrip(&size, at, items.len(), car.strip);
            let thumbs = items[start..end].iter().enumerate().map(|(k, (mid, ai))| -> El<'a> {
                let a = &stored(&st, mid).attachments[*ai];
                let edge = if start + k == at { p.accent } else { CLEAR };
                let face = container(self.thumb(a, 44.0, 6.0)).padding(2);
                let face = face.style(bordered_box(white.scale_alpha(0.1), edge, 2.0, 8.0));
                let open = UiMessage::OpenCarousel(mid.clone(), *ai, true);
                button(face).padding(0).style(btn_style([CLEAR; 3], [white; 3], 8.0)).on_press(open).into()
            });
            let row = container(Row::with_children(thumbs).spacing(4)).padding([6, 10]).width(Fill);
            let scroll = |d: mouse::ScrollDelta| {
                let by = match d {
                    mouse::ScrollDelta::Lines { x, y } => -(x + y),
                    mouse::ScrollDelta::Pixels { x, y } => -(x + y) / 52.0,
                };
                UiMessage::ScrollFilmstrip(by)
            };
            let row = mouse_area(row.style(fill_box(Color::BLACK.scale_alpha(0.45), 0.0))).on_scroll(scroll);
            container(row.on_press(UiMessage::Noop)).align_bottom(Fill)
        });
        // Desktop's `carousel-reactions` in a chat: the reactions of the message, 16 px above the bottom in the middle.
        let reactions = (!car.in_media && !m.reactions.is_empty() && m.is_deleted != Some(true)).then(|| {
            let look = PillLook {
                group: c.chat_type == "group",
                own: Color { a: 0.6, ..p.accent },
                other: Color::BLACK.scale_alpha(0.6),
                hover: None,
                fg: (white, white),
                ring: (white.scale_alpha(0.2), 1.0),
                gap: 4.0,
            };
            let pills = mouse_area(self.reaction_pills(&st, c, &[m], &look)).on_press(UiMessage::Noop);
            container(pills).align_bottom(Fill).center_x(Fill).padding(Padding::new(24.0).top(0.0).bottom(16.0))
        });
        let main = mouse_area(main).on_right_press(UiMessage::CarouselMenu(true));
        let body = column![container(head).padding(8).height(50), main];
        let layer = stack![body]
            .push((at > 0).then(|| step(false)))
            .push((at + 1 < items.len()).then(|| step(true)))
            .push(video.and_then(|(src, _)| self.view_carousel_video_controls(&src)))
            .push(reactions)
            .push(strip)
            .push(car.menu.map(|at| self.view_carousel_menu(&st, car, m, c, at)));
        let scrim = container(layer).width(Fill).height(Fill).style(fill_box(Color::BLACK.scale_alpha(0.85), 0.0));
        // The carousel hides the window under it from the cursor.
        Some(mouse_area(scrim).interaction(mouse::Interaction::Idle).on_press(UiMessage::CloseCarousel).into())
    }

    /// Desktop's carousel menu: Download, Copy for an image, and the extra items. In a chat, the extra items are Reply…,
    /// React…, Forward… and Delete…, and all but React… close the carousel first. In the Media window, they are Go to
    /// Message and Delete….
    fn view_carousel_menu<'a>(&'a self, st: &BeeperStore, car: &Carousel, m: &Message, c: &Chat, at: Point) -> El<'a> {
        let p = &self.pal;
        let (mid, ai) = (car.mid.clone(), car.ai);
        let close = |msg| UiMessage::Commands(vec![UiMessage::CloseCarousel, msg]);
        let action = |s| trx(s, "attachment carousel context menu action").to_string();
        let image = m.attachments[ai].attachment_type == "img";
        let mut rows = vec![Some((action("Download"), UiMessage::Download(mid.clone(), Some(ai))))];
        rows.extend(image.then(|| Some((action("Copy"), UiMessage::CopyImage(mid.clone(), ai)))));
        let deletable = deletable(st, m);
        let (first, delete) = match car.in_media {
            true => (
                vec![(tr("Go to Message").into(), UiMessage::GoToMessage(c.id.clone(), mid.clone()))],
                deletable.then(|| (tr("Delete…").into(), UiMessage::MediaAskDelete(vec![(mid.clone(), ai)]))),
            ),
            false => {
                let (react, reply) = self.carousel_actions(car, m, c);
                let forward = close(UiMessage::Forward(forward::Msg::Open(vec![mid.clone()])));
                let first = [(tr("Reply…"), reply), (tr("React…"), react), (tr("Forward…"), Some(forward))];
                let first = first.into_iter().filter_map(|(label, msg)| msg.map(|msg| (label.to_string(), msg)));
                let delete = close(UiMessage::AskDeleteMessage(vec![mid.clone()]));
                (first.collect(), deletable.then(|| (tr("Delete…").into(), delete)))
            }
        };
        rows.push(None);
        rows.extend(first.into_iter().map(Some));
        rows.extend(delete.map(|d| [None, Some(d)]).into_iter().flatten());
        let h = 8.0 + rows.iter().map(|r| if r.is_some() { MENU_ITEM } else { 11.0 }).sum::<f32>();
        let rows = rows.into_iter().map(|r| match r {
            Some((label, msg)) => {
                let msg = UiMessage::Commands(vec![UiMessage::CarouselMenu(false), msg]);
                menu_item(p, None, label, "", Some(msg))
            }
            None => menu_rule(p),
        });
        let menu = container(Column::with_children(rows).padding([4, 0])).width(180);
        let menu = menu.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let area = match car.in_media {
            true => self.media_window.as_ref().expect("the Media window is open").size,
            false => self.pane.size,
        };
        let at = Padding::ZERO.left(at.x.min(area.width - 188.0)).top(at.y.min(area.height - h - 8.0));
        mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
            .on_press(UiMessage::CarouselMenu(false))
            .on_right_press(UiMessage::CarouselMenu(false))
            .into()
    }

    pub(crate) fn view_media<'a>(&'a self, w: &MediaWindow) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let [file_icon, video_icon, link_icon, _, tune_icon, search_icon, done_icon] = &*MEDIA_ICONS;
        let picking = w.select || self.mods.control();
        let picked = |mid: &str, k: usize| w.picked.iter().any(|(m, i)| m == mid && *i == k);
        let check = |on: bool| -> El<'a> {
            let (bg, edge) = if on { (p.accent, p.accent) } else { (Color::BLACK.scale_alpha(0.25), Color::WHITE) };
            let mark = on.then(|| tinted(done_icon, 14.0, Color::WHITE));
            container(mark).center(20).style(bordered_box(bg, edge, 1.5, 10.0)).into()
        };
        let media = self.media_messages(&st);
        let items = media_items(&st, &media, w);
        // Desktop's `label-sm` (11/14) and `body-sm` (13/16) type of the Media window.
        let sub = |s: String| txt(s, 11.0).color(p.on_secondary);
        // Desktop's `BrandSegmentControl`: the highlight slides to the selected tab in `0.25s`.
        let widths = MEDIA_TABS.map(|l| text_width(tr(l), 11.0, Weight::Normal) + 16.0);
        let (from, t) = w.tab_at.map_or((w.tab, 1.0), |(from, at)| (from, at.elapsed().as_secs_f32() / TAB_SLIDE));
        let (x, hw) = slide(&widths, (from, w.tab), t);
        let highlight = container(Space::new().width(hw).height(18)).style(fill_box(p.chip, 20.0));
        let tabs = MEDIA_TABS.iter().zip(widths).enumerate().map(|(i, (label, width))| -> El<'a> {
            let label = txt(tr(label), 11.0).color(p.strong);
            button(container(label).center_x(Fill))
                .width(width)
                .padding([2, 8])
                .style(btn_style([CLEAR; 3], [p.strong; 3], 20.0))
                .on_press(UiMessage::MediaTab(i))
                .into()
        });
        // The highlight layer comes first, under the labels, so it sets the size of the stack.
        let total = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
        let under = container(highlight).padding(Padding::ZERO.left(x)).width(total);
        let tabs = stack![under, Row::with_children(tabs).spacing(4)];
        let tabs = container(tabs).padding(4).style(fill_box(p.card, 20.0));
        let control = |glyph: &svg::Handle, size: f32, on: bool, msg| -> El<'a> {
            let (bg, fg) = if on { (p.chip, p.strong) } else { (CLEAR, p.on_secondary) };
            let b = button(container(tinted(glyph, size, fg)).center(28)).padding(0);
            b.style(btn_style([bg, p.chip, p.chip], [fg; 3], 12.0)).on_press(msg).into()
        };
        let search = UiMessage::MediaSearch(w.search.is_none().then(String::new));
        let controls = row![
            self.tip(
                control(done_icon, 18.0, w.select, UiMessage::MediaSelect(!w.select)),
                tr("Select"),
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(tune_icon, 18.0, false, UiMessage::MediaMenu(true)),
                tr("Filter"),
                vec![],
                tooltip::Position::Bottom
            ),
            self.tip(
                control(search_icon, 18.0, w.search.is_some(), search),
                tr("Search"),
                vec![],
                tooltip::Position::Bottom
            ),
        ]
        .spacing(4);
        let header = container(row![tabs, Space::new().width(Fill), controls].spacing(8).align_y(Center))
            .padding([8, 10])
            .center_y(52);
        let rule = || container(Space::new()).width(Fill).height(0.5).style(fill_box(p.chip, 0.0));
        let search_row = w.search.as_deref().map(|q| {
            let input = text_input(tr("Search by caption"), q)
                .id("media-search")
                .on_input(|q| UiMessage::MediaSearch(Some(q)))
                .size(13)
                .padding(0)
                .style(input_style(CLEAR, p.strong, p.on_secondary, p.accent, 0.0));
            let close = self.tip(
                control(&ROW_ACTIONS[6], 16.0, false, UiMessage::MediaSearch(None)),
                tr("Close"),
                vec![],
                tooltip::Position::Bottom,
            );
            let line = row![tinted(search_icon, 16.0, p.on_secondary), input, close].spacing(6).align_y(Center);
            column![container(line).padding([6, 10]), rule()]
        });
        let inner = w.size.width - 16.0;
        let cols = ((inner / 160.0) as usize).max(3);
        let tile = (inner - 20.0 - 4.0 * (cols - 1) as f32) / cols as f32;
        let text_w = inner - 20.0 - 48.0 - 8.0 - 150.0;
        let mut groups: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
        for &(i, k) in items.iter().take(w.shown) {
            let label = media_group(day_num(&media[i].timestamp));
            match groups.last_mut() {
                Some((l, g)) if *l == label => g.push((i, k)),
                _ => groups.push((label, vec![(i, k)])),
            }
        }
        let body = groups.iter().fold(Column::new(), |col, (label, group)| {
            let days = group.iter().map(|&(i, _)| day_num(&media[i].timestamp));
            let (lo, hi) = (days.clone().min().expect("a group has items"), days.max().expect("a group has items"));
            let full = |d| i18n::date(i18n::Date::Medium, civil_from_days(d), (0, 0));
            let range = if lo == hi { full(lo) } else { format!("{} - {}", full(lo), full(hi)) };
            let head = column![semi(label.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong), sub(range)];
            let col = col.push(container(head).padding(Padding::new(6.0).top(12.0).left(10.0).right(10.0)));
            match w.tab {
                0 => group.chunks(cols).fold(col, |col, chunk| {
                    let tiles = chunk.iter().map(|&(i, k)| -> El<'a> {
                        let m = &media[i];
                        let a = &m.attachments[k];
                        let badge = match (a.is_gif == Some(true), a.attachment_type == "video") {
                            (true, _) => Some(row![semi(tr("GIF"), 11.0)]),
                            (false, true) => {
                                let time = a.duration.map(|d| semi(voice::clock(d as f32), 11.0));
                                Some(row![tinted(video_icon, 12.0, Color::WHITE)].push(time).spacing(2).align_y(Center))
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
                        let name =
                            ell(Self::sender_name(&st, m), tile - 12.0, 11.0, Weight::Semibold).color(Color::WHITE);
                        let name =
                            container(name).width(Fill).padding(Padding::new(4.0).top(10.0).left(6.0).right(6.0));
                        let name = container(name.style(move |_| container::Style {
                            background: Some(fade.into()),
                            border: border::rounded(border::Radius::default().bottom(8)),
                            ..Default::default()
                        }))
                        .align_bottom(Fill);
                        let on = picked(&m.id, k);
                        let mark = w.select.then(|| container(check(on)).align_right(Fill).padding(4));
                        let ring = on.then(|| {
                            container(Space::new())
                                .width(Fill)
                                .height(Fill)
                                .style(bordered_box(CLEAR, p.accent, 2.0, 8.0))
                        });
                        let face = stack![self.thumb(a, tile, 8.0)].push(badge).push(name).push(mark).push(ring);
                        let face = face.width(tile).height(tile);
                        let open = match picking {
                            true => UiMessage::MediaPick(m.id.clone(), k),
                            false => UiMessage::OpenCarousel(m.id.clone(), k, true),
                        };
                        let b = button(face).padding(0).style(btn_style([CLEAR; 3], [p.strong; 3], 8.0));
                        mouse_area(b.on_press(open))
                            .on_right_press(UiMessage::MediaItemMenu(Some((m.id.clone(), k))))
                            .into()
                    });
                    col.push(
                        container(Row::with_children(tiles).spacing(4))
                            .padding(Padding::new(0.0).bottom(4.0).left(10.0).right(10.0)),
                    )
                }),
                _ => group.iter().fold(col, |col, &(i, k)| {
                    let m = &media[i];
                    let icon = |glyph, size| -> El<'a> {
                        container(tinted(glyph, size, p.on_secondary)).center(40).style(fill_box(p.chip, 8.0)).into()
                    };
                    let (thumb, title, line, press) = match w.tab {
                        // Desktop's link thumb: the preview image of the link, or the link icon.
                        1 => {
                            let l = &m.links[k];
                            let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
                            let title = if l.title.is_empty() { host.to_string() } else { l.title.clone() };
                            let thumb = match l.img.as_deref().map(|u| self.media_at(u)) {
                                Some(Some(Ok(frames))) => image(frames[0].0.clone())
                                    .width(40)
                                    .height(40)
                                    .content_fit(iced::ContentFit::Cover)
                                    .border_radius(8.0)
                                    .into(),
                                Some(_) => {
                                    container(Space::new()).width(40).height(40).style(fill_box(p.chip, 8.0)).into()
                                }
                                None => icon(link_icon, 16.0),
                            };
                            (thumb, title, l.url.clone(), UiMessage::OpenUrl(l.url.clone()))
                        }
                        _ => {
                            let a = &m.attachments[k];
                            let kind = file_kind(a.mime_type.as_deref());
                            let line = std::iter::once(kind.to_string()).chain(a.file_size.map(human_size));
                            let name = a.file_name.clone().unwrap_or_else(|| tr("Attachment").into());
                            (
                                icon(file_icon, 18.0),
                                name,
                                line.collect::<Vec<_>>().join(" · "),
                                UiMessage::Download(m.id.clone(), Some(k)),
                            )
                        }
                    };
                    let text = column![
                        ell(&title, text_w, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(p.strong),
                        ell(&line, text_w, 11.0, Weight::Normal).line_height(Pixels(14.0)).color(p.on_secondary),
                    ]
                    .width(Fill);
                    let meta = column![sub(Self::sender_name(&st, m).to_string()), sub(relative_date(&m.timestamp))]
                        .align_x(Right);
                    let on = picked(&m.id, k);
                    let line = row![thumb, text, meta].push(w.select.then(|| check(on))).spacing(8).align_y(Center);
                    let press = if picking { UiMessage::MediaPick(m.id.clone(), k) } else { press };
                    let bg = if on { p.chip } else { CLEAR };
                    let b = button(line).padding([6, 10]).width(Fill);
                    let b = b.style(btn_style([bg, p.chip, p.chip], [p.strong; 3], 0.0)).on_press(press);
                    col.push(mouse_area(b).on_right_press(UiMessage::MediaItemMenu(Some((m.id.clone(), k)))))
                }),
            }
        });
        let more = w.shown < items.len();
        let list: El<'a> = match items.is_empty() {
            true => container(txt(tr("Nothing here"), 13.0).color(p.on_secondary)).center_x(Fill).padding(16).into(),
            false => scrollable(body.push(Space::new().height(8)))
                .direction(bar())
                .height(Fill)
                .style(scroll_style(p.scroll))
                .on_scroll(
                    move |v| if more && v.relative_offset().y > 0.8 { UiMessage::MediaMore } else { UiMessage::Noop },
                )
                .into(),
        };
        let title = container(semi(tr("Media"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let footer = w.select.then(|| {
            let bytes: u64 = match w.tab {
                1 => 0,
                _ => w.picked.iter().filter_map(|(mid, k)| stored(&st, mid).attachments[*k].file_size).sum(),
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
                act(tr("Delete"), (n > 0).then(|| UiMessage::MediaAskDelete(picked))),
                act(tr("Save"), (n > 0 && w.tab != 1).then_some(UiMessage::MediaSave)),
                act(tr("Select all"), Some(UiMessage::MediaPickAll)),
                act(tr("Cancel"), Some(UiMessage::MediaSelect(false))),
            ]
            .spacing(4)
            .align_y(Center);
            column![rule(), container(bar).padding([8, 10]).center_y(44)]
        });
        let pane = column![header, rule()].push(search_row).push(list).push(footer);
        let pane = container(pane).width(Fill).height(Fill).style(fill_box(p.card, 12.0));
        let base = container(column![title, pane]).padding(8).width(Fill).height(Fill).style(fill_box(p.window, 0.0));
        let item_menu = w.item_menu.as_ref().map(|(mid, k, at)| {
            let m = &stored(&st, mid);
            let act =
                |label: &str, msg| menu_item(p, None, label.into(), "", Some(UiMessage::MediaItemAct(Box::new(msg))));
            let go = act(tr("Go to Message"), UiMessage::GoToMessage(m.chat_id.clone(), mid.clone()));
            let mut actions = match w.tab {
                1 => vec![act(tr("Copy Link"), UiMessage::CopyLink(m.links[*k].url.clone()))],
                _ => std::iter::once(act(tr("Download"), UiMessage::Download(mid.clone(), Some(*k))))
                    .chain(
                        (m.attachments[*k].attachment_type == "img")
                            .then(|| act(tr("Copy Image"), UiMessage::CopyImage(mid.clone(), *k))),
                    )
                    .collect(),
            };
            actions.extend(
                deletable(&st, m).then(|| act(tr("Delete…"), UiMessage::MediaAskDelete(vec![(mid.clone(), *k)]))),
            );
            let h = 8.0 + MENU_ITEM * (1 + actions.len()) as f32 + 11.0;
            let col = Column::with_children([go, menu_rule(p)].into_iter().chain(actions));
            let menu = container(col.padding([4, 0])).width(180).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let at = Padding::ZERO.left(at.x.min(w.size.width - 180.0 - 8.0)).top(at.y.min(w.size.height - h - 8.0));
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MediaItemMenu(None))
                .on_right_press(UiMessage::MediaItemMenu(None))
        });
        // Desktop's filter menu is a native menu at the cursor.
        let menu = w.menu.map(|at| {
            let heading = |label| container(med(label, 11.0).color(p.sub)).padding([6, 13]);
            let radio = |label: &str, on: bool, msg| {
                let lead = container(on.then(|| ic(icon::CHECK, 14.0))).width(22);
                menu_item(p, Some(lead.into()), label.into(), "", Some(msg))
            };
            let col = column![
                heading(tr("Sent By")),
                radio(tr("All"), w.sent_by == 0, UiMessage::MediaSentBy(0)),
                radio(tr("You"), w.sent_by == 1, UiMessage::MediaSentBy(1)),
                radio(tr("Others"), w.sent_by == 2, UiMessage::MediaSentBy(2)),
                menu_rule(p),
                heading(tr("Sort by")),
                radio(tr("Newest"), !w.oldest, UiMessage::MediaOldest(false)),
                radio(tr("Oldest"), w.oldest, UiMessage::MediaOldest(true)),
            ];
            let menu = container(col.padding([4, 0])).width(180).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let at = Padding::ZERO.top(at.y).left(at.x.min(w.size.width - 8.0 - 180.0));
            mouse_area(container(mouse_area(menu).on_press(UiMessage::Noop)).padding(at).width(Fill).height(Fill))
                .on_press(UiMessage::MediaMenu(false))
        });
        drop(st);
        let carousel = self.carousel.as_ref().filter(|c| c.in_media).and_then(|c| self.view_carousel(c));
        let ask = w.deleting.as_ref().map(|(pairs, _)| {
            let n = pairs.len();
            let body = match n {
                1 => tr("Are you sure you want to delete this message?").to_string(),
                _ => trn(
                    "Are you sure you want to delete %d message?",
                    "Are you sure you want to delete %d messages?",
                    n,
                    &[&n],
                ),
            };
            let title = trn("Delete message", "Delete messages", n, &[]);
            self.dialog(
                title,
                body,
                vec![(tr("Delete"), UiMessage::MediaDelete(true))],
                (tr("Cancel"), UiMessage::MediaDelete(false)),
            )
        });
        // The menus and the delete dialog of the viewer show over it.
        stack![base].push(self.view_toast(false)).push(carousel).push(menu).push(item_menu).push(ask).into()
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
        let c = self.carousel.as_mut().expect("the carousel is open");
        let (start, _) = filmstrip(&size, at, items.len(), c.strip);
        c.strip = Some((start as f32 + by).round().max(0.0) as usize);
        Task::none()
    }

    /// The square cover crop of an image or video poster, or an empty tile while it loads.
    pub(crate) fn thumb<'a>(&self, a: &Attachment, size: f32, r: f32) -> El<'a> {
        let url = a.poster_img.as_ref().or(a.src_url.as_ref());
        match url.and_then(|u| self.media_at(u)) {
            Some(Ok(frames)) => image(frames[0].0.clone())
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(r)
                .into(),
            _ => container(Space::new()).width(size).height(size).style(fill_box(self.pal.chip, r)).into(),
        }
    }

    /// The name Desktop shows for the sender of a message.
    fn sender_name<'s>(st: &'s BeeperStore, m: &'s Message) -> &'s str {
        match m.is_sender == Some(true) {
            true => st.me().full_name.as_deref().expect("self name"),
            false => m.sender_name.as_deref().expect("sender name"),
        }
    }
}

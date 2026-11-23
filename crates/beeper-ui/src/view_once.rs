//! Desktop's view-limited ("view once") media: the pill that takes the place of the media
//! (`ViewLimitedMediaAttachment`), the store of the views (`ViewLimitedMediaStore`), the viewer
//! (`ViewLimitedMediaViewer`) and the Play button of the chat list (`ViewLimitedPlayButton`).

use crate::i18n::{tr, trx};
use crate::style::{bordered_box, bordered_btn, btn_style, ell, fill_box, med, semi, txt, Bare};
use crate::toasts::Toast;
use crate::{anim, chat_list, media_view, sound, tinted, video, voice, BeeperGuiApp, El, Frames, UiMessage};
use beeper_core::config::find_network;
use beeper_core::models::{Chat, Message, NetworkKind};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{button, column, container, image, mouse_area, row, stack, svg, Space};
use iced::{Background, Center, Color, Fill, Length, Padding, Subscription, Task};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// Desktop's `view-once`, `view-replay` and `play-fill-rounded`.
static GLYPHS: LazyLock<[svg::Handle; 3]> = LazyLock::new(|| {
    let dashed = r#"stroke="currentColor" stroke-dasharray="2 2" stroke-miterlimit="3.864" stroke-width="1.5""#;
    [
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 16 16"><rect width="12" height="12" x="2" y="2" {dashed} rx="6"/><path fill="currentColor" d="M8.483 11.085c-.358 0-.604-.242-.604-.609v-4.16h-.024l-.85.591a.5.5 0 0 1-.322.109.415.415 0 0 1-.427-.435c0-.185.072-.322.262-.455l1.152-.798c.282-.193.475-.23.725-.23.43 0 .689.262.689.681v4.697c0 .367-.242.609-.6.609"/></svg>"#
        ),
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 16 16"><path {dashed} d="M2 8a6 6 0 0 1 12 0A6 6 0 0 1 2 8Z"/><path fill="currentColor" d="M5.993 9.98V5.79q0-.261.135-.387a.45.45 0 0 1 .32-.129.64.64 0 0 1 .324.094l3.499 2.04q.192.113.28.219.092.105.092.258 0 .15-.091.258-.089.105-.281.216l-3.499 2.042a.64.64 0 0 1-.325.094.45.45 0 0 1-.32-.129q-.134-.129-.134-.387"/><path stroke="currentColor" stroke-linecap="round" stroke-width="1.5" d="M8 14A6 6 0 1 1 8 2"/></svg>"#
        ),
        r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 10 11"><path fill="currentColor" d="M1.306.42q.3 0 .598.17l6.914 4.037q.375.222.539.416a.67.67 0 0 1 .164.457.68.68 0 0 1-.164.457q-.165.194-.54.416L1.905 10.41q-.3.17-.598.17a.8.8 0 0 1-.586-.24q-.24-.234-.24-.71V1.37q0-.482.24-.716A.82.82 0 0 1 1.306.42"/></svg>"#
            .to_string(),
    ]
    .map(|s| svg::Handle::from_memory(s.into_bytes()))
});

/// Desktop's `ViewLimitedMediaStore` (`MS`): the views of this session, the open viewer and the Play queue. Matrix
/// event IDs are the message IDs here, so its event and message sets are one set.
#[derive(Default)]
pub(crate) struct ViewOnce {
    /// `usedUpEventIDs`: the media whose last view was here.
    used_up: HashSet<String>,
    /// `localRemaining`: the views left after the views here.
    left: HashMap<String, u64>,
    /// `openedMessageIDs`.
    opened: HashSet<String>,
    /// `pendingViews`: the view of the open media until it shows, as its account, its limit and the views left after.
    pending: HashMap<String, (String, Value, u64)>,
    viewer: Option<Viewer>,
    queue: Option<Queue>,
}

/// Desktop's `playQueue`: the media of a chat that play one after the other.
struct Queue {
    chat: String,
    messages: VecDeque<String>,
    current: Option<String>,
    last_played: Option<String>,
    skipped: bool,
}

/// Desktop's `viewLimitedMediaViewer` and the state of its `ViewLimitedMediaViewer`.
#[derive(Default)]
struct Viewer {
    mid: String,
    kind: String,
    url: String,
    size: Option<(f32, f32)>,
    replay: bool,
    /// `timeLimit`: how long a view lasts, in milliseconds.
    time: Option<u64>,
    sender: String,
    timestamp: String,
    account_id: String,
    /// `src`: the media once it is in memory.
    shown: Option<Shown>,
    /// The media showed and counted as viewed.
    viewed: bool,
    /// When the time of the view runs out.
    deadline: Option<Instant>,
    /// Where a voice note is, and since when it plays.
    audio: (f32, Option<Instant>),
}

/// Desktop's `loadIntoMemory`: the decoded media of the viewer.
#[derive(Clone)]
pub enum Shown {
    Image(Frames),
    Audio(voice::Clip),
    Video(Arc<video::Media>),
}

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `open` from the pill of a message.
    Open(String),
    Loaded(String, Result<Shown, String>),
    /// The Close button, Escape or a click beside the media: the next media of the queue plays after an image that
    /// has no time limit.
    Dismiss,
    /// Desktop's `onEnded` of a video or a voice note.
    Ended,
    /// Plays or pauses the voice note.
    Toggle,
    Tick,
    /// Desktop's `playAll` from the chat list Play button.
    PlayAll(String),
    /// The bridge took the view, or why not.
    Counted(String, Result<(), String>),
}

/// The network of a message is Snapchat, whose media Desktop calls a Snap.
fn snap(m: &Message) -> bool {
    find_network(&m.account_id).is_some_and(|n| n.kind == NetworkKind::Snapchat)
}

/// Desktop's `R1t`: the label of the pill.
fn label(m: &Message) -> &'static str {
    match m.view_once_kind() {
        Some("audio") => trx("Voice note", "view-limited media pill"),
        _ if snap(m) => trx("Snap", "view-limited media pill"),
        Some("video") => trx("Video", "view-limited media pill"),
        _ => trx("Photo", "view-limited media pill"),
    }
}

/// Desktop's `N1t`: the label of the pill that opens the media.
fn action(m: &Message) -> &'static str {
    match m.view_once_kind() {
        Some("audio") => trx("Play voice note", "view-limited media pill"),
        _ if snap(m) => trx("View Snap", "view-limited media pill"),
        Some("video") => trx("View video", "view-limited media pill"),
        _ => trx("View photo", "view-limited media pill"),
    }
}

/// Desktop's `cVe`: the preview text of view-once media, `aVe` while it can be viewed and `lVe` once it expired.
pub(crate) fn preview(m: &Message) -> Option<&'static str> {
    let ctx = "view-limited media preview";
    let words = match (m.view_expired, m.view_once()) {
        (true, _) => ["Expired voice note", "Expired Snap", "Expired video", "Expired photo"],
        (false, true) => ["Sent a voice note", "Sent a Snap", "Sent a video", "Sent a photo"],
        (false, false) => return None,
    };
    let kind = if m.view_expired { m.view_kind.as_deref() } else { m.attachments.first().map(|a| &*a.attachment_type) };
    Some(trx(
        match kind {
            Some("audio") => words[0],
            _ if snap(m) => words[1],
            Some("video") => words[2],
            _ => words[3],
        },
        ctx,
    ))
}

/// Desktop's `RS`: HEIC and HEIF images, which Desktop cannot open.
fn heic(m: &Message) -> bool {
    m.attachments.first().is_some_and(|a| {
        let mime = a.mime_type.as_deref().unwrap_or_default();
        let name = a.file_name.as_deref().unwrap_or_default().to_ascii_lowercase();
        mime.starts_with("image/heic")
            || mime.starts_with("image/heif")
            || name.ends_with(".heic")
            || name.ends_with(".heif")
    })
}

/// The look of Desktop's `ViewLimitedMediaIndicator`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Variant {
    /// Your own media, in the colors of your bubbles.
    Filled,
    /// Media that you viewed for the last time, or that expired.
    Outlined,
    /// The media of a quoted message.
    Quote,
}

impl ViewOnce {
    /// Desktop's `canPlay`: media that you received and did not view for the last time here.
    pub(crate) fn can_play(&self, m: &Message) -> bool {
        m.is_sender != Some(true) && m.view_once() && !self.used_up.contains(&m.id)
    }

    /// Desktop's `isUnopened`: media that nobody opened, as far as the bridge and this session know.
    pub(crate) fn unopened(&self, m: &Message) -> bool {
        self.can_play(m) && m.views_left.is_none() && !self.opened.contains(&m.id)
    }

    /// The viewer shows the media of message `mid`.
    pub(crate) fn shows(&self, mid: &str) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.mid == mid)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.viewer.is_some()
    }

    /// Desktop's `nextView`: the limit of the next view and the views left after it.
    fn next_view(&self, m: &Message) -> Option<(Value, u64)> {
        let mut limit = m.view_once_limit()?;
        if limit["type"] != "count" {
            return Some((limit, 0));
        }
        let count = limit["count"].as_u64()?;
        let n = count.min(self.left.get(&m.id).copied().unwrap_or(count));
        limit["count"] = n.into();
        (n > 0).then_some((limit, n.saturating_sub(1)))
    }
}

impl Viewer {
    /// The time left of the view, once it started.
    fn left(&self) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// The time of the view ran out.
    fn timed_out(&self) -> bool {
        self.left().is_some_and(|l| l.is_zero())
    }

    fn clip(&self) -> Option<&voice::Clip> {
        match &self.shown {
            Some(Shown::Audio(c)) => Some(c),
            _ => None,
        }
    }

    /// Where the voice note is.
    fn audio_pos(&self) -> f32 {
        let (at, since) = self.audio;
        let secs = self.clip().map_or(0.0, voice::Clip::secs);
        (at + since.map_or(0.0, |s| s.elapsed().as_secs_f32())).min(secs)
    }

    /// Plays the voice note from where it is, or stops it.
    fn output(&self) {
        let clip = self.clip().filter(|_| self.audio.1.is_some());
        sound::voice(clip.map(|c| {
            let from = (self.audio_pos() / c.secs().max(f32::EPSILON) * c.samples.len() as f32) as usize;
            (c.samples.clone(), from.min(c.samples.len()), 1.0)
        }));
    }
}

impl BeeperGuiApp {
    pub(crate) fn view_once(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open(mid) => return self.open_view_once(&mid).unwrap_or_else(Task::none),
            Msg::Loaded(mid, r) => return self.view_once_loaded(mid, r),
            Msg::Dismiss => {
                let Some(v) = self.view_once.viewer.as_ref() else { return Task::none() };
                let next = !matches!(v.kind.as_str(), "video" | "audio") && v.time.is_none() && v.viewed;
                return self.close_view_once(next);
            }
            Msg::Ended => {
                let v = self.view_once.viewer.as_mut().expect("the media of an open viewer ends");
                if v.time.is_none() || v.timed_out() {
                    return self.close_view_once(true);
                }
                match v.kind.as_str() {
                    "audio" => {
                        v.audio = (0.0, Some(Instant::now()));
                        v.output();
                    }
                    _ => return self.video(video::Msg::Toggle),
                }
            }
            Msg::Toggle => {
                let v = self.view_once.viewer.as_mut().expect("an open viewer plays");
                v.audio = match v.audio.1 {
                    Some(_) => (v.audio_pos(), None),
                    None => (v.audio.0, Some(Instant::now())),
                };
                v.output();
            }
            Msg::Tick => {
                let Some(v) = self.view_once.viewer.as_mut() else { return Task::none() };
                if v.audio.1.is_some() && v.clip().is_some_and(|c| v.audio_pos() >= c.secs()) {
                    v.audio = (0.0, None);
                    v.output();
                    return self.view_once(Msg::Ended);
                }
                // A video or a voice note that plays goes on after the time runs out, until it ends or pauses.
                let playing = match v.kind.as_str() {
                    "video" => self.video.as_ref().is_some_and(video::Video::playing),
                    "audio" => v.audio.1.is_some(),
                    _ => false,
                };
                if v.timed_out() && !playing {
                    return self.close_view_once(true);
                }
            }
            Msg::PlayAll(chat) => return self.play_all(chat),
            Msg::Counted(mid, Err(e)) => eprintln!("the bridge did not take the view of {mid}: {e}"),
            Msg::Counted(_, Ok(())) => {}
        }
        Task::none()
    }

    /// Desktop's `open`: the viewer of the media of message `mid` while the media loads into memory, or `None` when the
    /// media cannot open.
    fn open_view_once(&mut self, mid: &str) -> Option<Task<UiMessage>> {
        let st = self.store.read();
        let m = st.loaded(mid)?.clone();
        let (sender, client) = (crate::sender_label(st.chat(&m.chat_id), &m), st.matrix.clone());
        drop(st);
        let a = m.attachments.first()?;
        let vo = &self.view_once;
        if vo.used_up.contains(mid) || vo.pending.contains_key(mid) || vo.viewer.is_some() {
            return None;
        }
        let p = self.provision(&m.account_id);
        if p.bridge_id.is_empty() || p.login_id.is_empty() {
            eprintln!("view-once media {mid} has no bridge login");
            return None;
        }
        let (limit, after) = vo.next_view(&m)?;
        let url = a.src_url.clone().filter(|_| m.unresolved_media.is_none())?;
        if heic(&m) {
            self.fail(Toast::new(tr("This media can't be opened on desktop yet.").into()));
            return None;
        }
        let viewer = Viewer {
            mid: mid.into(),
            kind: a.attachment_type.clone(),
            url: url.clone(),
            size: a.size.as_ref().and_then(|s| Some((s.width? as f32, s.height? as f32))),
            replay: m.view_once_replay(),
            time: m.view_once_time(),
            sender,
            timestamp: m.timestamp.clone(),
            account_id: m.account_id.clone(),
            ..Default::default()
        };
        self.view_once.pending.insert(mid.into(), (m.account_id.clone(), limit, after));
        self.audio.pause();
        if let Some(v) = self.video.as_mut() {
            v.pause();
        }
        let kind = viewer.kind.clone();
        self.view_once.viewer = Some(viewer);
        let mid = mid.to_string();
        Some(Task::perform(
            async move {
                let bytes = beeper_core::matrix::media(client, url.clone()).await?;
                let decode = move || match kind.as_str() {
                    "video" => video::probe(&url, &bytes).map(|m| Shown::Video(Arc::new(m))),
                    "audio" => voice::decode(&bytes).map(Shown::Audio),
                    _ => media_view::decode_media(bytes).map(Shown::Image),
                };
                tokio::task::spawn_blocking(decode).await.expect("view-once media decode task")
            },
            move |r| UiMessage::ViewOnce(Msg::Loaded(mid, r)),
        ))
    }

    /// The media in memory shows and counts as viewed, like Desktop's `onLoad` and `onLoadedData`.
    fn view_once_loaded(&mut self, mid: String, r: Result<Shown, String>) -> Task<UiMessage> {
        let showing = self.view_once.shows(&mid);
        let shown = match r {
            Ok(shown) if showing => shown,
            Ok(_) => return Task::none(),
            Err(e) => {
                eprintln!("view-once media {mid} failed to load: {e}");
                if showing {
                    self.fail(Toast::new(tr("Could not open this media.").into()));
                    self.view_once.pending.remove(&mid);
                    self.view_once.viewer = None;
                }
                return self.skip_queued(&mid);
            }
        };
        if let Shown::Video(media) = &shown {
            let v = self.view_once.viewer.as_ref().expect("the viewer shows the media");
            let (w, h) = self.view_once_video_size(v.size);
            let size = (w.round() as u32, h.round() as u32);
            self.play_loaded(mid.clone(), v.url.clone(), size, media.clone());
        }
        let v = self.view_once.viewer.as_mut().expect("the viewer shows the media");
        let audio = matches!(shown, Shown::Audio(_));
        v.shown = Some(shown);
        v.viewed = true;
        v.deadline = v.time.map(|t| Instant::now() + Duration::from_millis(t));
        // Desktop's `<audio autoPlay>`.
        if audio {
            v.audio = (0.0, Some(Instant::now()));
            v.output();
        }
        if let Some(q) = self.view_once.queue.as_mut().filter(|q| q.current.as_ref() == Some(&mid)) {
            q.last_played = Some(mid.clone());
        }
        self.mark_viewed(mid)
    }

    /// Desktop's `markViewed`: counts the view here and tells the bridge.
    fn mark_viewed(&mut self, mid: String) -> Task<UiMessage> {
        let vo = &mut self.view_once;
        let Some((account, limit, after)) = vo.pending.remove(&mid) else { return Task::none() };
        vo.left.insert(mid.clone(), after);
        vo.opened.insert(mid.clone());
        if after == 0 {
            vo.used_up.insert(mid.clone());
        }
        let p = self.provision(&account);
        let eid = mid.clone();
        Task::perform(beeper_core::matrix::view_limited_media(p, eid, limit), move |r| {
            UiMessage::ViewOnce(Msg::Counted(mid, r))
        })
    }

    /// Desktop's `close`: the viewer closes, and the queue goes on with `next`, else it stops.
    fn close_view_once(&mut self, next: bool) -> Task<UiMessage> {
        let Some(v) = self.view_once.viewer.take() else { return Task::none() };
        self.view_once.pending.remove(&v.mid);
        if self.video.as_ref().is_some_and(|x| x.mid == v.mid) {
            self.video = None;
        }
        if v.audio.1.is_some() {
            sound::voice(None);
        }
        match self.view_once.queue.as_ref().filter(|q| q.current.as_ref() == Some(&v.mid)) {
            Some(_) if next => self.play_next(),
            Some(_) => {
                self.view_once.queue = None;
                Task::none()
            }
            None => Task::none(),
        }
    }

    /// Desktop's `playAll`: the received media at the end of the chat, oldest first.
    fn play_all(&mut self, chat: String) -> Task<UiMessage> {
        if self.view_once.viewer.is_some() || self.view_once.queue.is_some() {
            return Task::none();
        }
        let mut st = self.store.write();
        st.load_chat(&chat);
        let shown: Vec<&Message> = st.timeline(&chat).filter(|m| !m.silent() && m.is_deleted != Some(true)).collect();
        let vo = &mut self.view_once;
        let from = shown.iter().rposition(|m| !vo.can_play(m)).map_or(0, |i| i + 1);
        let messages: VecDeque<String> = shown[from..].iter().map(|m| m.id.clone()).collect();
        // With nothing to play, the Play button of the chat goes away.
        if let Some(last) = shown.last().filter(|_| messages.is_empty()) {
            vo.used_up.insert(last.id.clone());
        }
        drop(st);
        vo.queue = Some(Queue { chat, messages, current: None, last_played: None, skipped: false });
        self.play_next()
    }

    /// Desktop's `playNext`: opens the next media of the queue. After the last one, the chat is read when its last
    /// message played, and a queue that played nothing shows an error.
    fn play_next(&mut self) -> Task<UiMessage> {
        let Some(q) = self.view_once.queue.as_mut() else { return Task::none() };
        let Some(mid) = q.messages.pop_front() else {
            let q = self.view_once.queue.take().expect("a queue");
            if q.current.is_some() && q.last_played.is_none() {
                self.fail(Toast::new(tr("Could not open this media.").into()));
            } else if let Some(last) = q.last_played.filter(|_| !q.skipped) {
                let st = self.store.read();
                let newest = st.resolve_chat_idx(&q.chat).and_then(|i| st.chats[i].preview.as_ref()).map(|m| &m.id);
                if newest == Some(&last) {
                    drop(st);
                    return self.update(UiMessage::ChatList(chat_list::Msg::MarkRead(q.chat)));
                }
            }
            return Task::none();
        };
        q.current = Some(mid.clone());
        match self.open_view_once(&mid) {
            Some(task) => task,
            None => self.skip_queued(&mid),
        }
    }

    /// The media of message `mid` did not open: the queue goes on when it is the current one.
    fn skip_queued(&mut self, mid: &str) -> Task<UiMessage> {
        match self.view_once.queue.as_mut().filter(|q| q.current.as_deref() == Some(mid)) {
            Some(q) => {
                q.skipped = true;
                self.play_next()
            }
            None => Task::none(),
        }
    }

    /// Desktop's `playAction` of a chat row: the last message of a chat that is no merged chat is view-once media that
    /// you can view.
    pub(crate) fn plays(&self, c: &Chat) -> bool {
        c.merge.is_none() && c.preview.as_ref().is_some_and(|m| self.view_once.can_play(m))
    }

    /// The countdown, the end of a voice note and the spinner while the viewer is open.
    pub(crate) fn view_once_ticks(&self) -> Option<Subscription<UiMessage>> {
        self.view_once
            .viewer
            .as_ref()
            .map(|_| iced::time::every(Duration::from_millis(50)).map(|_| UiMessage::ViewOnce(Msg::Tick)))
    }

    /// The size of a video in the viewer: at most its own size, `min(90vw, 720px)` wide and `100vh - 150px` high.
    fn view_once_video_size(&self, size: Option<(f32, f32)>) -> (f32, f32) {
        let area = self.pane.size;
        let (w, h) = size.unwrap_or((240.0, 160.0));
        let k = ((0.9 * area.width).min(720.0) / w).min((area.height - 150.0).max(1.0) / h).min(1.0);
        (w * k, h * k)
    }

    /// Desktop's `ViewLimitedMediaIndicator` (`Ty`): the kind of the media, after its time limit or its view-once or
    /// replay icon.
    pub(crate) fn view_once_indicator<'a>(&self, m: &Message, variant: Variant, (bg, fg): (Color, Color)) -> El<'a> {
        let p = &self.pal;
        let lead: El<'a> = match m.view_once_time() {
            Some(t) => {
                let chip = if variant == Variant::Filled { fg.scale_alpha(0.4) } else { p.chip };
                let clock =
                    med(crate::units::seconds(t), 12.0).color(if variant == Variant::Filled { fg } else { p.sub });
                container(clock).center_y(20).padding([0, 6]).style(fill_box(chip, 10.0)).into()
            }
            None => tinted(&GLYPHS[usize::from(m.view_once_replay())], 16.0, fg),
        };
        let line = row![lead, txt(label(m), 13.0).color(fg)].spacing(4).align_y(Center);
        match variant {
            Variant::Filled => container(line).padding(Padding::new(8.0).right(12.0)).style(fill_box(bg, 17.0)).into(),
            Variant::Outlined => container(line).padding([6, 9]).style(bordered_box(bg, p.received, 2.0, 17.0)).into(),
            Variant::Quote => line.into(),
        }
    }

    /// Desktop's `ViewLimitedMediaAttachment` (`vfe`): your own media is filled, media that you viewed for the last
    /// time is outlined, and the media that you can view is a button with the brand color of its network around it.
    pub(crate) fn view_once_pill<'a>(&self, m: &Message, sent: (Color, Color)) -> El<'a> {
        let p = &self.pal;
        if m.is_sender == Some(true) {
            return self.view_once_indicator(m, Variant::Filled, sent);
        }
        if self.view_once.used_up.contains(&m.id) {
            return self.view_once_indicator(m, Variant::Outlined, (p.card, p.text));
        }
        let edge = find_network(&m.account_id).map_or(p.border_neutrals, |n| {
            let [r, g, b] = n.brand_rgb;
            Color::from_rgb8(r, g, b)
        });
        let line = row![tinted(&GLYPHS[2], 10.0, p.fg), med(action(m), 12.0).color(p.fg)].spacing(6).align_y(Center);
        button(line)
            .padding(Padding::from([6, 10]).left(12.0))
            .style(bordered_btn([p.card; 3], [p.fg; 3], edge, 2.0, 17.0))
            .on_press(UiMessage::ViewOnce(Msg::Open(m.id.clone())))
            .into()
    }

    /// Desktop's `ViewLimitedPlayButton`: plays the received media at the end of a chat.
    pub(crate) fn view_once_play<'a>(&self, chat: &str) -> El<'a> {
        let p = &self.pal;
        let glyph = container(tinted(&GLYPHS[2], 9.0, Color::WHITE)).center(22);
        button(glyph)
            .padding(0)
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 11.0))
            .on_press(UiMessage::ViewOnce(Msg::PlayAll(chat.into())))
            .into()
    }

    /// Desktop's `ViewLimitedMediaViewer`: the media over the window, with its sender, its time and its network, and a
    /// bar of its playback or of the time left of the view.
    pub(crate) fn view_view_once(&self) -> Option<El<'_>> {
        let v = self.view_once.viewer.as_ref()?;
        let white = Color::WHITE;
        let h24 = self.store.read().settings.pref("timeFormat") == 1;
        let close = button(tinted(&voice::ICONS[4], 20.0, white)).bare(white).padding(4);
        let title = column![
            ell(&v.sender, 0.5 * self.pane.size.width, 13.0, Weight::Semibold).color(white),
            txt(crate::time::format_short_ts(&v.timestamp, h24), 11.0).color(white.scale_alpha(0.7)),
        ]
        .align_x(Center);
        let brand = svg(crate::style::brand_round(crate::brand_key(&v.account_id))).width(24).height(24);
        let header = row![
            container(close.on_press(UiMessage::ViewOnce(Msg::Dismiss))).width(Fill),
            mouse_area(title).on_press(UiMessage::Noop),
            container(mouse_area(brand).on_press(UiMessage::Noop)).align_right(Fill),
        ]
        .spacing(8)
        .align_y(Center);
        let video = self.video.as_ref().filter(|x| x.mid == v.mid);
        let bar = |share: f32| -> El<'_> {
            let part = |f: f32| Length::FillPortion((f.clamp(0.0, 1.0) * 1000.0).round().max(1.0) as u16);
            let fill = container(Space::new()).width(part(share)).height(Fill).style(fill_box(white, 3.0));
            let track = row![fill, Space::new().width(part(1.0 - share))];
            container(track).width(Fill).height(6).style(fill_box(white.scale_alpha(0.35), 3.0)).into()
        };
        let progress = match v.kind.as_str() {
            "video" => Some(bar(video.map_or(0.0, video::Video::progress))),
            "audio" => Some(bar(v.clip().map_or(0.0, |c| v.audio_pos() / c.secs().max(f32::EPSILON)))),
            // Desktop's timer bar of an image grows over the time of the view.
            _ => v.time.map(|t| bar(v.left().map_or(0.0, |l| 1.0 - l.as_millis() as f32 / t as f32))),
        };
        let limit: El<'_> = match v.time {
            Some(t) => {
                let left = v.left().map_or(t, |l| l.as_millis() as u64);
                semi(crate::units::seconds(left), 15.0).color(white).into()
            }
            None => tinted(&GLYPHS[usize::from(v.replay)], 24.0, white),
        };
        let shade = Linear::new(iced::Radians(std::f32::consts::PI))
            .add_stop(0.0, Color::BLACK.scale_alpha(0.45))
            .add_stop(1.0, Color::TRANSPARENT);
        let top =
            container(row![].push(progress).push(Space::new().width(Fill)).push(limit).spacing(12).align_y(Center))
                .padding(16)
                .width(Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Gradient(shade.into())),
                    ..Default::default()
                });
        let square = |inner: El<'static>| -> El<'static> {
            container(inner).center(240).style(fill_box(white.scale_alpha(0.08), 24.0)).into()
        };
        let media: El<'_> = match (&v.shown, video) {
            (None, _) => square(crate::verify::spinner(32.0, white)),
            (Some(Shown::Video(_)), Some(_)) => {
                let (w, h) = self.view_once_video_size(v.size);
                let surface = self.view_video(&v.mid, &v.url, Space::new().into(), (w, h), video::Place::Carousel);
                // Desktop's mute button is at half opacity while the video is muted.
                let k = if video.is_some_and(|x| x.muted) { 0.5 } else { 1.0 };
                let mute = button(container(tinted(&chat_list::TAG_ICONS[2], 16.0, white.scale_alpha(k))).center(32))
                    .padding(0)
                    .style(btn_style([Color::BLACK.scale_alpha(0.45 * k); 3], [white; 3], 16.0))
                    .on_press(UiMessage::Video(video::Msg::Mute));
                stack![surface, container(mute).align_bottom(Fill).padding(10)].into()
            }
            (Some(Shown::Video(_)), None) => square(Space::new().into()),
            (Some(Shown::Audio(_)), _) => {
                let glyph = &voice::ICONS[usize::from(v.audio.1.is_some())];
                let toggle = button(container(tinted(glyph, 32.0, white)).center(72))
                    .padding(0)
                    .style(btn_style([white.scale_alpha(0.16); 3], [white; 3], 36.0))
                    .on_press(UiMessage::ViewOnce(Msg::Toggle));
                square(toggle.into())
            }
            (Some(Shown::Image(frames)), _) => {
                let (mw, mh) = ((0.9 * self.pane.size.width).min(720.0), (self.pane.size.height - 150.0).max(1.0));
                let still = move |f| image(f).content_fit(iced::ContentFit::ScaleDown).border_radius(24.0);
                container(anim::gif(frames, self.started, still)).max_width(mw).max_height(mh).into()
            }
        };
        let card = mouse_area(stack![media, top]).on_press(UiMessage::Noop);
        let body = container(card).center(Fill).padding(16);
        let overlay = container(column![container(header).padding(8).height(50), body])
            .width(Fill)
            .height(Fill)
            .style(fill_box(Color::BLACK.scale_alpha(0.9), 0.0));
        Some(mouse_area(overlay).on_press(UiMessage::ViewOnce(Msg::Dismiss)).into())
    }
}

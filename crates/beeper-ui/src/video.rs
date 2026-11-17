//! Desktop's inline video player (`Video/index.tsx`): frames streamed from ffmpeg, the sound track, and the hover
//! controls.
use crate::i18n::{tr, trx};
use crate::media_view;
use crate::message;
use crate::style::{btn_style, fill_box, shadow_box, Bare, CLEAR};
use crate::{ic, icon, menu_item, sound, tinted, view_once, BeeperGuiApp, El, UiMessage, MENU_ITEM};
use iced::gradient::Linear;
use iced::widget::{button, column, container, hover, image, mouse_area, row, slider, stack, svg, Column, Space};
use iced::{window, Background, Center, Color, Fill, Padding, Subscription, Task};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const FPS: f32 = 30.0;
pub(crate) const SPEED_MENU_W: f32 = 110.0;
const SPEED_MENU_H: f32 = 8.0 + crate::voice::RATES.len() as f32 * MENU_ITEM;
/// The width of the menu of the Picture in Picture window, which leaves room for the speed submenu beside it in a
/// window 400 px wide.
const PIP_MENU_W: f32 = 200.0;
const HZ: f32 = 48_000.0;

/// Desktop's `replay10`, `forward10`, `more`, `full-screen`, `close` and `picture-in-picture`.
pub(crate) static GLYPHS: LazyLock<[svg::Handle; 6]> = LazyLock::new(|| {
    [
        "M10 18.333a7.3 7.3 0 0 1-2.927-.594 7.6 7.6 0 0 1-2.375-1.604 7.6 7.6 0 0 1-1.604-2.375 7.3 7.3 0 0 1-.594-2.927h1.667q0 2.437 1.698 4.135T10 16.666t4.135-1.698 1.698-4.135q0-2.437-1.698-4.135T10 5h-.125l1.292 1.291L10 7.5 6.667 4.166 10 .833l1.167 1.208-1.292 1.292H10q1.563 0 2.927.594t2.375 1.604a7.6 7.6 0 0 1 1.604 2.375q.594 1.364.594 2.927a7.3 7.3 0 0 1-.594 2.927 7.6 7.6 0 0 1-1.604 2.375 7.6 7.6 0 0 1-2.375 1.604 7.3 7.3 0 0 1-2.927.594m-2.5-5v-3.75H6.25v-1.25h2.5v5zm2.917 0a.8.8 0 0 1-.594-.24.8.8 0 0 1-.24-.593V9.166q0-.354.24-.593.24-.24.594-.24h1.666q.354 0 .594.24t.24.593V12.5q0 .354-.24.593a.8.8 0 0 1-.594.24zm.416-1.25h.834v-2.5h-.834z",
        "M7.5 13.332v-3.75H6.25v-1.25h2.5v5zm2.917 0a.8.8 0 0 1-.594-.24.8.8 0 0 1-.24-.593V9.165q0-.354.24-.593.24-.24.594-.24h1.666q.354 0 .594.24t.24.593V12.5q0 .354-.24.593a.8.8 0 0 1-.594.24zm.416-1.25h.834v-2.5h-.834zM10 18.332a7.3 7.3 0 0 1-2.927-.594 7.6 7.6 0 0 1-2.375-1.604 7.6 7.6 0 0 1-1.604-2.375 7.3 7.3 0 0 1-.594-2.927q0-1.562.594-2.927.593-1.364 1.604-2.375a7.6 7.6 0 0 1 2.375-1.604A7.3 7.3 0 0 1 10 3.332h.125L8.833 2.04 10 .832l3.333 3.333L10 7.5 8.833 6.29 10.125 5H10q-2.437 0-4.135 1.698t-1.698 4.135 1.698 4.135T10 16.665t4.135-1.698 1.698-4.135H17.5a7.3 7.3 0 0 1-.594 2.927 7.6 7.6 0 0 1-1.604 2.375 7.6 7.6 0 0 1-2.375 1.604 7.3 7.3 0 0 1-2.927.594",
        "M10 16.666q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 15q0-.688.49-1.177t1.177-.49 1.177.49q.49.489.49 1.177 0 .687-.49 1.177t-1.177.49m0-5q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 10q0-.688.49-1.177T10 8.333t1.177.49q.49.489.49 1.177 0 .687-.49 1.177t-1.177.49m0-5q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 5q0-.688.49-1.177T10 3.333t1.177.49q.49.489.49 1.177 0 .687-.49 1.177T10 6.667",
        "M2.43 17.33v-4.139h1.656v2.484H6.57v1.655zm10.762 0v-1.655h2.484V13.19h1.655v4.14zM2.431 6.57v-4.14H6.57v1.656H4.086V6.57zm13.245 0V4.085h-2.484V2.43h4.14v4.139z",
        "m5.333 15.834-1.166-1.167L8.833 10 4.167 5.334l1.166-1.167L10 8.834l4.667-4.667 1.166 1.167L11.167 10l4.666 4.667-1.166 1.167L10 11.167z",
        "M12.557 4.25q.864 0 1.304.44.446.442.447 1.3V8.71h1.004q.864 0 1.31.44.446.435.446 1.294v3.566q0 .855-.446 1.295-.446.445-1.31.445H9.919q-.87 0-1.315-.44-.44-.44-.44-1.3v-.81H4.687q-.87 0-1.316-.436-.44-.44-.44-1.3V5.991q0-.86.44-1.3.446-.44 1.316-.44zM9.979 9.745q-.382 0-.58.199-.2.193-.2.585v3.394q0 .393.2.586.199.199.58.198h5.274q.37 0 .57-.198.203-.193.203-.586V10.53q0-.391-.204-.585-.198-.2-.57-.2zM4.747 5.292q-.376 0-.58.199-.199.193-.2.586v5.296q.001.391.2.59.204.2.58.199h3.416v-1.72q0-.858.44-1.293.447-.44 1.316-.44h3.347V6.077q0-.393-.2-.586-.198-.199-.574-.199z",
    ]
    .map(glyph)
});

/// Desktop's `VideoRenderErrorIcon`, the Material `error_outline`.
static ERROR: LazyLock<svg::Handle> = LazyLock::new(|| {
    glyph_in("0 0 24 24", "M11 15h2v2h-2zm0-8h2v6h-2zm.99-5C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8z")
});

/// Desktop's `speed025` to `speed2`, the icons of `voice::RATES`.
static SPEEDS: LazyLock<[svg::Handle; 8]> = LazyLock::new(|| {
    [
        "M3 14v-1.5h1.5V14zm8.5 0v-1.5H15v-1.75h-3.5V6h5v1.5H13v1.75h2q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.44.44-1.06.44zm-6 0v-3.25q0-.619.44-1.06.442-.44 1.06-.44h2V7.5H5.5V6H9q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.442.44-1.06.44H7v1.75h3.5V14z",
        "M2.5 14v-1.5H4V14zm8.5 0 2.313-4L11 6h1.73l1.457 2.5L15.626 6h1.73l-2.313 4 2.312 4h-1.729l-1.437-2.5-1.459 2.5zm-6 0v-1.5h3.5v-1.75H5V6h5v1.5H6.5v1.75h2q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.442.44-1.06.44z",
        "M3.5 14v-1.5H5V14zm7.5 0v-1.5h3.5v-1.75H11V6h5v1.5h-3.5v1.75h2q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.442.44-1.06.44zm-4.5 0 2-6.5H5V6h3.708q.536 0 .914.375t.378.917q0 .041-.062.396L8 14z",
        "M5.5 14V7.5H4V6h3v8zm4.146 0 2.312-4-2.312-4h1.729l1.438 2.5L14.27 6H16l-2.312 4L16 14h-1.73l-1.457-2.5-1.438 2.5z",
        "M5 14v-1.5h1.5V14zm8.5 0v-1.5H17v-1.75h-3.5V6h5v1.5H15v1.75h2q.62 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.44.44-1.06.44zm-6 0v-3.25q0-.619.44-1.06.442-.44 1.06-.44h2V7.5H7.5V6H11q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.442.44-1.06.44H9v1.75h3.5V14zm-5 0V7.5H1V6h3v8z",
        "M5 14v-1.5h1.5V14zm-2.5 0V7.5H1V6h3v8zM13 14l2.313-4L13 6h1.73l1.437 2.5L17.604 6h1.75l-2.312 4 2.312 4h-1.75l-1.437-2.5-1.438 2.5zm-5.5 0v-1.5h3v-1.75h-3V6H12v1.5H9v1.75h1.5q.619 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.442.44-1.06.44z",
        "M6 14v-1.5h1.5V14zm7.5 0v-1.5H17v-1.75h-3.5V6h5v1.5H15v1.75h2q.62 0 1.06.44.44.442.44 1.06v1.75q0 .619-.44 1.06-.44.44-1.06.44zm-10 0V7.5H2V6h3v8zM9 14l2-6.5H7.5V6h3.708q.536 0 .914.375t.378.917q0 .041-.062.396L10.5 14z",
        "M4 14v-3.25q0-.619.44-1.06.442-.44 1.06-.44h2V7.5H4V6h3.5q.619 0 1.06.44Q9 6.883 9 7.5v1.75q0 .619-.44 1.06-.442.44-1.06.44h-2v1.75H9V14zm6 0 2.313-4L10 6h1.73l1.457 2.5L14.626 6h1.73l-2.313 4 2.312 4h-1.729l-1.437-2.5-1.459 2.5z",
    ]
    .map(glyph)
});

fn glyph(d: &str) -> svg::Handle {
    glyph_in("0 0 20 20", d)
}

fn glyph_in(view: &str, d: &str) -> svg::Handle {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path d="{d}"/></svg>"#);
    svg::Handle::from_memory(svg.into_bytes())
}

/// Where a video shows: in the timeline as attachment `ai` of its message, in the carousel, or on the full screen.
#[derive(Clone, Copy, PartialEq)]
pub enum Place {
    Inline(usize),
    Carousel,
    Full,
    /// Desktop's Picture in Picture window.
    Pip,
}

/// Desktop's `video-progress-bar`: a 3 px white bar over a dim one, and a hidden slider that seeks.
fn video_progress<'a>(v: &Video) -> El<'a> {
    let frac = v.progress();
    let bar = |c: Color| container(Space::new()).width(Fill).height(3).style(fill_box(c, 100.0));
    let part = |f: f32| iced::Length::FillPortion((f * 1000.0).round().max(1.0) as u16);
    let done = row![bar(Color::WHITE).width(part(frac)), Space::new().width(part(1.0 - frac))];
    let seek = slider(0.0..=1.0, frac, |f| UiMessage::Video(Msg::Seek(f)))
        .step(0.001_f32)
        .height(11)
        .on_release(UiMessage::Video(Msg::SeekEnd))
        .style(crate::style::hidden_slider);
    stack![container(stack![bar(Color::WHITE.scale_alpha(0.2)), done]).center_y(11), seek].into()
}

/// A menu of the Picture in Picture window at `x`, `y`, which keeps its clicks.
fn placed(x: f32, y: f32, menu: El<'_>) -> El<'_> {
    let menu = mouse_area(menu).on_press(UiMessage::Noop);
    container(menu).padding(Padding::ZERO.left(x.max(0.0)).top(y.max(0.0))).into()
}

/// Desktop's `jA` time format, `mm:ss` with the minutes not cut at an hour.
fn mmss(secs: f32) -> String {
    let s = secs as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// A downloaded video: a temporary file for ffmpeg to seek in, its length, and its mono 48 kHz sound.
pub struct Media {
    path: PathBuf,
    secs: f32,
    sound: Option<Arc<[f32]>>,
}

impl Drop for Media {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The inline video that played last, at `at` seconds when playback started at `since`.
pub struct Video {
    pub mid: String,
    pub url: String,
    size: (u32, u32),
    media: Option<Arc<Media>>,
    at: f32,
    since: Option<Instant>,
    /// Frames from `start` seconds on, of which `shown` came out.
    frames: Option<Receiver<image::Handle>>,
    start: f32,
    shown: usize,
    frame: Option<image::Handle>,
    pub rate: f32,
    pub muted: bool,
    /// The frame size before the video filled the screen, and before it went to Picture in Picture.
    full: Option<(u32, u32)>,
    inline: Option<(u32, u32)>,
    /// The "Playback Speed" submenu is open.
    pub sub: bool,
    /// The video could not load.
    failed: bool,
}

#[derive(Clone)]
pub enum Msg {
    Play(String, String, u32, u32),
    /// A click on the inline video of attachment `ai` of a message, which is `w` by `h`.
    Open(String, usize, u32, u32),
    Loaded(String, Result<Arc<Media>, String>),
    Toggle,
    Skip(f32),
    Seek(f32),
    SeekEnd,
    Tick,
    Rate(f32),
    Mute,
    Fullscreen,
    /// Desktop's `openInPip` of the video menu.
    OpenPip,
    /// A right click on the video: Desktop's video menu in place of the message menu.
    Menu(String),
    Sub,
    /// Desktop's `VisibilityMonitor`: the inline video of a message came into view, or left it.
    Seen(String, bool),
    /// Opens the video menu of the Picture in Picture window at the cursor, or closes it.
    PipMenu(bool),
    /// An item of the menu of the Picture in Picture window, which closes the menu.
    PipItem(Box<UiMessage>),
    /// Desktop's "Show in Chat" of the menu of the Picture in Picture window.
    ShowInChat,
    /// Desktop's "Restore from Picture in Picture" button over the inline video.
    Restore,
    /// Desktop's `handleClosePip`, the Close button of the Picture in Picture window.
    ClosePip,
}

fn run(program: &str, args: &[&str], path: &Path) -> Result<Vec<u8>, String> {
    let out = Command::new(program).args(["-v", "error", "-i"]).arg(path).args(args).output();
    let out = out.map_err(|e| format!("Could not start {program}: {e}"))?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Store `bytes` for ffmpeg and read the length and sound track.
pub(crate) fn probe(url: &str, bytes: &[u8]) -> Result<Media, String> {
    let path = std::env::temp_dir().join(format!("beeper-rs-video-{:x}", crate::style::name_hash(url)));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    // Dropping `media` on an error removes the file.
    let mut media = Media { path, secs: 0.0, sound: None };
    let info = run("ffprobe", &["-show_entries", "format=duration:stream=codec_type", "-of", "csv=p=0"], &media.path)?;
    let info = String::from_utf8_lossy(&info);
    media.secs = info.lines().last().and_then(|l| l.trim().parse().ok()).ok_or("no video duration")?;
    if info.lines().any(|l| l == "audio") {
        let pcm = run("ffmpeg", &[&["-vn"], &crate::voice::PCM[..]].concat(), &media.path)?;
        media.sound = Some(crate::voice::samples(&pcm).into());
    }
    Ok(media)
}

/// Decode `path` from `from` seconds at `FPS`, `size` pixels, two frames ahead of the player.
fn stream(path: &Path, from: f32, (w, h): (u32, u32)) -> Receiver<image::Handle> {
    let (tx, rx) = sync_channel(2);
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &from.to_string(), "-i"])
        .arg(path)
        .args(["-vf", &format!("fps={FPS},scale={w}:{h}:flags=fast_bilinear"), "-f", "rawvideo"])
        .args(["-pix_fmt", "rgba", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("ffmpeg ran for the probe");
    std::thread::spawn(move || {
        let mut out = child.stdout.take().expect("piped stdout");
        let mut px = vec![0; (w * h * 4) as usize];
        while out.read_exact(&mut px).is_ok() && tx.send(image::Handle::from_rgba(w, h, px.clone())).is_ok() {}
        let _ = child.kill();
        let _ = child.wait();
    });
    rx
}

impl Video {
    /// A video of message `mid` that plays as soon as it loads.
    fn new(mid: String, url: String, size: (u32, u32)) -> Self {
        Video {
            mid,
            url,
            size,
            media: None,
            at: 0.0,
            since: Some(Instant::now()),
            frames: None,
            start: 0.0,
            shown: 0,
            frame: None,
            rate: 1.0,
            muted: false,
            full: None,
            inline: None,
            sub: false,
            failed: false,
        }
    }

    pub fn playing(&self) -> bool {
        self.since.is_some()
    }

    pub fn is_full(&self) -> bool {
        self.full.is_some()
    }

    fn pos(&self) -> f32 {
        self.at + self.since.map_or(0.0, |s| s.elapsed().as_secs_f32() * self.rate)
    }

    /// The part of the loaded video that played.
    pub(crate) fn progress(&self) -> f32 {
        self.media.as_ref().map_or(0.0, |m| (self.pos() / m.secs).clamp(0.0, 1.0))
    }

    /// Desktop's `openPipWindow`: the size of the Picture in Picture window, 400 px on the longer side.
    pub(crate) fn pip_size(&self) -> (u32, u32) {
        let k = self.size.0.max(1) as f32 / self.size.1.max(1) as f32;
        match k >= 1.0 {
            true => (400, (400.0 / k).round() as u32),
            false => ((400.0 * k).round() as u32, 400),
        }
    }

    /// Decode the frames for the Picture in Picture window, or back at the inline size.
    pub(crate) fn set_pip(&mut self, on: bool) {
        match on {
            true => {
                self.inline = Some(self.size);
                self.resize(self.pip_size());
            }
            false => {
                if let Some(size) = self.inline.take() {
                    self.resize(size);
                }
            }
        }
    }

    /// Pause at the current position.
    pub fn pause(&mut self) {
        if self.since.is_some() {
            (self.at, self.since) = (self.pos(), None);
            sound::voice(None);
        }
    }

    /// Play from the current position, once the video is loaded.
    fn play(&mut self) {
        if self.since.is_none() && self.media.is_some() {
            self.since = Some(Instant::now());
            self.restart();
        }
    }

    /// Decode the frames at `size` from the current position on.
    fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        if self.media.is_some() {
            (self.at, self.since) = (self.pos(), self.since.map(|_| Instant::now()));
            self.restart();
        }
    }

    /// Restart the frames and the sound at the current position.
    fn restart(&mut self) {
        let media = self.media.as_ref().expect("a loaded video");
        (self.start, self.shown) = (self.at, 0);
        self.frames = Some(stream(&media.path, self.at, self.size));
        self.output();
    }

    /// Play the sound from the current position, unless the video is paused or muted.
    fn output(&self) {
        let from = (self.pos() * HZ) as usize;
        let sound = self.media.as_ref().expect("a loaded video").sound.as_ref();
        let sound = sound.filter(|s| from < s.len() && !self.muted);
        sound::voice(self.since.and(sound).map(|s| (s.clone(), from, self.rate)));
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.pause();
    }
}

impl BeeperGuiApp {
    pub(crate) fn video(&mut self, msg: Msg) -> Task<UiMessage> {
        if let Msg::Play(..) | Msg::Toggle | Msg::SeekEnd = msg {
            self.audio.pause();
        }
        match msg {
            Msg::Play(mid, url, w, h) => {
                let client = self.store.read().matrix.clone();
                let u = url.clone();
                self.video = Some(Video::new(mid, url.clone(), (w, h)));
                return Task::perform(
                    async move {
                        let bytes = beeper_core::matrix::media(client, u.clone()).await?;
                        let probe = move || probe(&u, &bytes).map(Arc::new);
                        tokio::task::spawn_blocking(probe).await.expect("video probe task")
                    },
                    move |r| UiMessage::Video(Msg::Loaded(url, r)),
                );
            }
            Msg::Loaded(url, r) => match (self.video.as_mut().filter(|v| v.url == url), r) {
                (Some(v), Ok(media)) => {
                    v.media = Some(media);
                    v.since = Some(Instant::now());
                    v.restart();
                }
                (Some(v), Err(e)) => {
                    eprintln!("video failed: {e}");
                    (v.failed, v.since) = (true, None);
                }
                (None, _) => {}
            },
            // Desktop pauses the inline video and opens the carousel, where the video goes on at the same time, play
            // state and speed.
            Msg::Open(mid, ai, w, h) => {
                let task = self.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, ai, false)));
                self.carousel.as_mut().expect("the carousel is open").resume = Some((w, h));
                return task;
            }
            Msg::Toggle => {
                let v = self.video.as_mut().expect("a played video");
                match v.since.is_some() {
                    true => v.pause(),
                    false => v.play(),
                }
            }
            Msg::Skip(by) => {
                let v = self.video.as_mut().expect("a played video");
                let secs = v.media.as_ref().expect("a loaded video").secs;
                v.at = (v.pos() + by).clamp(0.0, secs);
                v.since = v.since.map(|_| Instant::now());
                v.restart();
            }
            Msg::Seek(frac) => {
                let v = self.video.as_mut().expect("a played video");
                v.pause();
                v.at = frac * v.media.as_ref().expect("a loaded video").secs;
            }
            Msg::SeekEnd => {
                let v = self.video.as_mut().expect("a played video");
                v.since = Some(Instant::now());
                v.restart();
            }
            Msg::Rate(rate) => {
                let v = self.video.as_mut().expect("a played video");
                (v.at, v.rate, v.since) = (v.pos(), rate, v.since.map(|_| Instant::now()));
                v.restart();
                (self.pane.message_menu, self.pip_menu) = (None, None);
            }
            Msg::Mute => {
                let v = self.video.as_mut().expect("a played video");
                v.muted ^= true;
                v.output();
                self.pane.message_menu = None;
            }
            Msg::Sub => (self.video.as_mut().expect("a played video").sub, self.pane.remind_sub) = (true, false),
            Msg::Menu(mid) => {
                let open = self.update(UiMessage::Message(message::Msg::OpenMenu(mid)));
                self.pane.video_menu = true;
                return open;
            }
            Msg::OpenPip => {
                self.pane.message_menu = None;
                (self.pip_wanted, self.pip_manual) = (true, true);
            }
            // Desktop's `handleVisibilityChange`: a playing video that leaves the view goes on in Picture in Picture,
            // and one that went there by itself comes back inline when its inline video is in view again.
            Msg::Seen(mid, false) => {
                let video = self.video.as_ref().filter(|v| v.mid == mid && v.media.is_some());
                let playing = video.is_some_and(|v| v.playing() && !v.is_full());
                self.pip_wanted |= playing && self.pip.is_none();
            }
            Msg::Seen(mid, true) if !self.pip_manual && self.pip.as_ref().is_some_and(|p| p.1 == mid) => {
                return self.restore_pip();
            }
            Msg::Seen(..) => {}
            Msg::PipMenu(open) => {
                self.pip_menu = open.then(|| self.cursor.get());
                self.video.iter_mut().for_each(|v| v.sub = false);
            }
            Msg::PipItem(msg) => {
                self.pip_menu = None;
                return self.update(*msg);
            }
            // Desktop shows the main window, jumps to the message and restores the video there.
            Msg::ShowInChat => {
                let mid = self.pip.as_ref().expect("the Picture in Picture window is open").1.clone();
                let cid = self.store.read().loaded(&mid).map(|m| m.chat_id.clone());
                let jump = cid.map_or_else(Task::none, |cid| self.jump_to(cid, mid));
                return Task::batch([self.set_hidden(false), jump, self.restore_pip()]);
            }
            Msg::Restore => return self.restore_pip(),
            Msg::ClosePip => self.stop_pip(),
            Msg::Fullscreen => {
                let (id, area) = match self.carousel.as_ref().filter(|c| c.in_media && self.carousel_shows_video()) {
                    Some(_) => {
                        let w = self.media_window.as_ref().expect("the Media window is open");
                        (w.id, w.size)
                    }
                    None => (self.main_window.expect("the main window is open"), self.pane.size),
                };
                let v = self.video.as_mut().expect("a played video");
                let (mode, size) = match v.full.take() {
                    Some(before) => (window::Mode::Windowed, before),
                    None => {
                        v.full = Some(v.size);
                        let (w, h) = (v.size.0 as f32, v.size.1 as f32);
                        let k = (area.width / w).min(area.height / h);
                        (window::Mode::Fullscreen, ((w * k) as u32, (h * k) as u32))
                    }
                };
                v.resize(size);
                self.pane.message_menu = None;
                return window::set_mode(id, mode);
            }
            Msg::Tick => {
                let v = self.video.as_mut().expect("only a played video ticks");
                if v.pos() >= v.media.as_ref().expect("a loaded video").secs {
                    (v.at, v.since) = (0.0, None);
                    v.restart();
                    if self.view_once.shows(&v.mid) {
                        return self.view_once(view_once::Msg::Ended);
                    }
                }
                let due = (((v.pos() - v.start) * FPS) as usize + 1).max(1);
                while v.shown < due {
                    match v.frames.as_ref().and_then(|f| f.try_recv().ok()) {
                        Some(f) => (v.frame, v.shown) = (Some(f), v.shown + 1),
                        None => break,
                    }
                }
            }
        }
        Task::none()
    }

    /// Plays the video of message `mid` in the view-once viewer, from media that the viewer loaded.
    pub(crate) fn play_loaded(&mut self, mid: String, url: String, size: (u32, u32), media: Arc<Media>) {
        self.audio.pause();
        let mut v = Video::new(mid, url, size);
        v.media = Some(media);
        v.restart();
        self.video = Some(v);
    }

    /// Desktop's `restoring` Picture in Picture state: the video goes on inline, and the window closes.
    fn restore_pip(&mut self) -> Task<UiMessage> {
        let Some((id, _)) = self.pip.take() else { return Task::none() };
        (self.pip_manual, self.pip_menu) = (false, None);
        if let Some(v) = self.video.as_mut() {
            v.set_pip(false);
        }
        window::close(id)
    }

    /// Desktop's `setPipState(null)`: the Picture in Picture window closes, and its video stops.
    pub(crate) fn stop_pip(&mut self) {
        if self.pip.is_some() {
            self.video = None;
        }
    }

    /// The carousel is open and shows the played video.
    fn carousel_shows_video(&self) -> bool {
        let src = self.carousel_video().map(|(src, _)| src);
        self.video.as_ref().is_some_and(|v| Some(&v.url) == src.as_ref())
    }

    /// Decode the played video at the carousel size when the carousel shows it.
    pub(crate) fn fit_carousel_video(&mut self) {
        let Some((src, size)) = self.carousel_video() else { return };
        if let Some(v) = self.video.as_mut().filter(|v| v.url == src) {
            v.resize(size);
        }
    }

    /// Desktop unmounts the video of the carousel when the carousel shows another item.
    pub(crate) fn leave_carousel_video(&mut self) {
        let src = self.carousel_video().map(|(src, _)| src);
        if let Some(v) = self.video.as_mut().filter(|v| Some(&v.url) == src.as_ref()) {
            v.pause();
        }
    }

    /// Desktop unmounts the video of the carousel when the carousel closes. When the carousel closes on the inline
    /// video that opened it, Desktop plays the inline video again.
    pub(crate) fn close_carousel_video(&mut self) -> Task<UiMessage> {
        let Some((src, _)) = self.carousel_video() else { return Task::none() };
        let c = self.carousel.as_ref().expect("the carousel is open");
        match (self.video.as_mut().filter(|v| v.url == src), c.resume) {
            (Some(v), Some(size)) => {
                v.resize(size);
                v.play();
            }
            (Some(v), None) => v.pause(),
            (None, Some((w, h))) => return self.video(Msg::Play(c.mid.clone(), src, w, h)),
            (None, None) => {}
        }
        Task::none()
    }

    pub(crate) fn video_ticks(&self) -> Option<Subscription<UiMessage>> {
        let v = self.video.as_ref().filter(|v| v.media.is_some() && (v.playing() || v.shown == 0));
        v.map(|_| iced::time::every(Duration::from_secs_f32(1.0 / FPS)).map(|_| UiMessage::Video(Msg::Tick)))
    }

    /// The video of message `mid` over `poster`, `w` by `h`, at `place`. The inline video of the played video tells
    /// when it comes into view and leaves it, like Desktop's `VisibilityMonitor`, an `IntersectionObserver` without
    /// a threshold.
    pub(crate) fn view_video<'a>(
        &self,
        mid: &str,
        url: &str,
        poster: El<'a>,
        size: (f32, f32),
        place: Place,
    ) -> El<'a> {
        let video = self.view_video_at(mid, url, poster, size, place);
        match place {
            Place::Inline(_) if self.video.as_ref().is_some_and(|v| v.url == url) => {
                let (key, mid) = (mid.to_string(), mid.to_string());
                let seen = move |on| UiMessage::Video(Msg::Seen(mid.clone(), on));
                let sensor = iced::widget::sensor(video).key(key).on_hide(seen(false));
                sensor.on_show(move |_| seen(true)).into()
            }
            _ => video,
        }
    }

    fn view_video_at<'a>(&self, mid: &str, url: &str, poster: El<'a>, (w, h): (f32, f32), place: Place) -> El<'a> {
        let v = self.video.as_ref().filter(|v| v.url == url);
        let r = if matches!(place, Place::Inline(_)) { 17.0 } else { 0.0 };
        // Desktop's `inPip`: the inline video of the video in Picture in Picture shows at 10% opacity, under the
        // "Restore from Picture in Picture" button when it went there from the menu.
        if let (Place::Inline(_), Some(v)) = (place, v.filter(|_| self.pip.as_ref().is_some_and(|p| p.1 == mid))) {
            let dim: El<'a> = match v.frame.clone() {
                Some(f) => image(f).width(w).height(h).border_radius(r).opacity(0.1_f32).into(),
                None => Space::new().width(w).height(h).into(),
            };
            let restore = self.pip_manual.then(|| {
                let glyph = container(tinted(&GLYPHS[5], 24.0, Color::BLACK)).padding(8);
                let glyph = glyph.style(fill_box(Color::from_rgb8(0xef, 0xef, 0xf0), 99.0));
                button(container(glyph).center_x(w).center_y(h))
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [CLEAR; 3], r))
                    .on_press(UiMessage::Video(Msg::Restore))
            });
            return stack![dim].push(restore).into();
        }
        let base: El<'a> = match v.and_then(|v| v.frame.clone()) {
            Some(f) => image(f).width(w).height(h).border_radius(r).into(),
            None => poster,
        };
        // A click on the inline video opens the carousel, and elsewhere it plays or pauses the video.
        let click = match place {
            Place::Inline(ai) => Some(UiMessage::Video(Msg::Open(mid.into(), ai, w.round() as u32, h.round() as u32))),
            _ => v.is_some_and(|v| v.media.is_some()).then_some(UiMessage::Video(Msg::Toggle)),
        };
        // Desktop's `video-error` card: the icon, the text and a Download button, at least 200 px wide.
        if v.is_some_and(|v| v.failed) {
            let p = &self.pal;
            let edge = p.text.scale_alpha(0.5);
            let download = button(crate::semi(trx("Download", "video error action"), 13.0).color(p.text))
                .padding([4, 10])
                .style(crate::style::bordered_btn([CLEAR; 3], [p.text; 3], edge, 1.0, 99.0))
                .on_press(UiMessage::Message(message::Msg::Download(mid.into(), None)));
            let text = crate::semi(tr("Something went wrong displaying video"), 13.0).color(p.text);
            let (text, download) = (
                container(text).padding(Padding::ZERO.top(13.0)),
                container(download).padding(Padding::ZERO.top(10.0)),
            );
            let card = column![tinted(&ERROR, 32.0, p.text), text, download].align_x(Center);
            let card = container(card).center_x(w.max(200.0)).center_y(h);
            return card.style(crate::style::bordered_box(p.window, edge, 1.0, 8.0)).into();
        }
        let surface = button(base).padding(0).style(btn_style([CLEAR; 3], [CLEAR; 3], r)).on_press_maybe(click);
        let Some(v) = v else {
            let play = button(tinted(&crate::voice::ICONS[0], 20.0, Color::from_rgb8(0x20, 0x21, 0x24)))
                .padding(8)
                .style(btn_style([Color::from_rgb8(0xef, 0xef, 0xf0); 3], [Color::BLACK; 3], 18.0))
                .on_press(UiMessage::Video(Msg::Play(mid.into(), url.into(), w.round() as u32, h.round() as u32)));
            return stack![surface, container(play).center_x(w).center_y(h)].into();
        };
        // The carousel shows its own controls and menu.
        if place == Place::Carousel {
            return surface.into();
        }
        let menu = match place {
            Place::Pip => Msg::PipMenu(true),
            _ => Msg::Menu(mid.into()),
        };
        let surface = mouse_area(surface).on_right_press(UiMessage::Video(menu.clone()));
        let on = |glyph: &svg::Handle, msg: UiMessage| -> El<'a> { self.video_button(v, glyph, msg) };
        let skips = self.video_skips(v);
        // Desktop's Close button takes the place of Fullscreen in the Picture in Picture window.
        let corner = match place {
            Place::Pip => on(&GLYPHS[4], UiMessage::Video(Msg::ClosePip)),
            _ => on(&GLYPHS[3], UiMessage::Video(Msg::Fullscreen)),
        };
        let corners = row![on(&GLYPHS[2], UiMessage::Video(menu)), Space::new().width(Fill), corner];
        let progress = video_progress(v);
        let shade = Linear::new(iced::Radians(std::f32::consts::PI))
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::BLACK.scale_alpha(0.66));
        let controls = container(column![skips.spacing(16).align_y(Center), progress].spacing(8).align_x(Center))
            .padding(Padding::new(10.0).top(32.0).bottom(12.0))
            .width(Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(shade.into())),
                border: iced::border::rounded(iced::border::bottom(r)),
                ..Default::default()
            });
        hover(surface, column![container(corners).padding(8), Space::new().height(Fill), controls].width(w).height(h))
    }

    /// Desktop's `PictureInPictureBrowserWindow`: the video that went on out of its chat, filling the window, and its
    /// video menu.
    pub(crate) fn view_pip(&self) -> El<'_> {
        let Some(v) = self.video.as_ref() else { return Space::new().into() };
        let (w, h) = v.pip_size();
        let (w, h) = (w as f32, h as f32);
        let video = self.view_video(&v.mid, &v.url, Space::new().into(), (w, h), Place::Pip);
        let video = container(video).center(Fill).style(fill_box(Color::BLACK, 0.0));
        let menu = self.pip_menu.and_then(|at| Some((at, self.video_menu_rows(&v.mid, true)?)));
        let Some((at, rows)) = menu else { return video.into() };
        // Desktop's video menu is a native menu at the cursor, which here stays inside the window, with the speed
        // submenu level with its row and beside the menu where it fits.
        let menu_h = 8.0 + rows.iter().map(|r| r.1).sum::<f32>();
        let col = Column::with_children(rows.into_iter().map(|r| r.0)).padding([4, 0]);
        let menu = container(col).width(PIP_MENU_W).style(shadow_box(self.pal.menu, self.pal.menu_border, 6.0, 16.0));
        let x = if at.x + PIP_MENU_W > w { at.x - PIP_MENU_W } else { at.x };
        let y = at.y.min(h - menu_h);
        let sub = self.view_speed_menu(&v.mid).map(|sub| {
            let right = x + PIP_MENU_W + SPEED_MENU_W <= w;
            let sub_x = if right { x + PIP_MENU_W } else { x - SPEED_MENU_W };
            placed(sub_x, (y + 3.0 * MENU_ITEM).min(h - SPEED_MENU_H), sub)
        });
        let close = mouse_area(Space::new().width(Fill).height(Fill))
            .on_press(UiMessage::Video(Msg::PipMenu(false)))
            .on_right_press(UiMessage::Video(Msg::PipMenu(false)));
        stack![video, close, placed(x, y, menu.into())].push(sub).into()
    }

    /// The video that fills the screen, like Desktop's `screenfull` toggle on the `<video>`, if it is the video of the
    /// carousel (`in_carousel`) or of the timeline.
    pub(crate) fn view_full_video(&self, in_carousel: bool) -> Option<El<'_>> {
        let v = self.video.as_ref().filter(|v| v.full.is_some() && self.carousel_shows_video() == in_carousel)?;
        let area = match self.carousel.as_ref().filter(|c| in_carousel && c.in_media) {
            Some(_) => self.media_window.as_ref().expect("the Media window is open").size,
            None => self.pane().size,
        };
        let (w, h) = (v.size.0 as f32, v.size.1 as f32);
        let k = (area.width / w).min(area.height / h);
        let video = self.view_video(&v.mid, &v.url, Space::new().into(), (w * k, h * k), Place::Full);
        Some(container(video).center(Fill).style(fill_box(Color::BLACK, 0.0)).into())
    }

    /// A white control of the played video, which works once the video is loaded.
    fn video_button<'a>(&self, v: &Video, glyph: &svg::Handle, msg: UiMessage) -> El<'a> {
        button(tinted(glyph, 20.0, Color::WHITE))
            .bare(Color::WHITE)
            .on_press_maybe(v.media.is_some().then_some(msg))
            .into()
    }

    /// Desktop's Rewind, Play or Pause, and Fast Forward buttons.
    fn video_skips<'a>(&self, v: &Video) -> iced::widget::Row<'a, UiMessage> {
        let skip = |i: usize, by| self.video_button(v, &GLYPHS[i], UiMessage::Video(Msg::Skip(by)));
        let toggle =
            self.video_button(v, &crate::voice::ICONS[usize::from(v.playing())], UiMessage::Video(Msg::Toggle));
        row![skip(0, -10.0), toggle, skip(1, 10.0)]
    }

    /// Desktop's `video-controls-carousel`: a row 28 px above the bottom of the window, 8 px from its sides, with the
    /// skip buttons, the progress bar, the speed button and the time, if the carousel video plays.
    pub(crate) fn view_carousel_video_controls<'a>(&self, src: &str) -> Option<El<'a>> {
        let v = self.video.as_ref().filter(|v| v.url == src && v.media.is_some())?;
        let secs = v.media.as_ref().expect("a loaded video").secs;
        let k = crate::voice::RATES.iter().position(|&r| r == v.rate).expect("a rate of the list");
        let next = crate::voice::RATES[(k + 1) % crate::voice::RATES.len()];
        let speed = self.video_button(v, &SPEEDS[k], UiMessage::Video(Msg::Rate(next)));
        let time = crate::semi(format!("{}/{}", mmss(v.pos().min(secs)), mmss(secs)), 11.0).color(Color::WHITE);
        let bar = row![self.video_skips(v).spacing(12), video_progress(v), speed, time].spacing(12).align_y(Center);
        let bar = mouse_area(container(bar).padding([8, 12])).on_press(UiMessage::Noop);
        Some(container(bar).align_bottom(Fill).padding(Padding::new(8.0).top(0.0).bottom(28.0)).into())
    }

    /// The video of message `mid` while it has a menu.
    fn menu_video(&self, mid: &str) -> Option<&Video> {
        self.video.as_ref().filter(|v| v.mid == mid && v.media.is_some())
    }

    /// Desktop's video menu (`getContextMenuItems`): Fullscreen, Open in PiP, Download, Mute and Playback Speed, and
    /// in the Picture in Picture window (`pip`) Show in Chat, Download, Mute and Playback Speed.
    pub(crate) fn video_menu_rows(&self, mid: &str, pip: bool) -> Option<Vec<(El<'_>, f32)>> {
        let v = self.menu_video(mid)?;
        let p = &self.pal;
        let item = |label: &str, msg| {
            let msg = if pip { UiMessage::Video(Msg::PipItem(Box::new(msg))) } else { msg };
            (menu_item(p, None, label.into(), "", Some(msg)), MENU_ITEM)
        };
        let action = |label| trx(label, "video context menu action");
        let speed = mouse_area(menu_item(p, None, tr("Playback Speed").into(), "›", Some(UiMessage::Video(Msg::Sub))))
            .on_enter(UiMessage::Video(Msg::Sub));
        let head = match pip {
            true => vec![item(tr("Show in Chat"), UiMessage::Video(Msg::ShowInChat))],
            false => vec![
                item(tr("Fullscreen"), UiMessage::Video(Msg::Fullscreen)),
                item(action("Open in PiP"), UiMessage::Video(Msg::OpenPip)),
            ],
        };
        let tail = [
            item(action("Download"), UiMessage::Message(message::Msg::Download(mid.into(), None))),
            item(action(if v.muted { "Unmute" } else { "Mute" }), UiMessage::Video(Msg::Mute)),
            (speed.into(), MENU_ITEM),
        ];
        Some(head.into_iter().chain(tail).collect())
    }

    /// The "Playback Speed" submenu.
    pub(crate) fn view_speed_menu(&self, mid: &str) -> Option<El<'_>> {
        let v = self.menu_video(mid).filter(|v| v.sub)?;
        let p = &self.pal;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let rates = crate::voice::RATES.map(|r| {
            let label = if r == 1.0 { tr("Normal").into() } else { format!("{r}x") };
            menu_item(p, lead(v.rate == r), label, "", Some(UiMessage::Video(Msg::Rate(r))))
        });
        let list = container(Column::with_children(rates).padding([4, 0])).width(SPEED_MENU_W);
        Some(list.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0)).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows;
    use beeper_core::store::NOTE_CHAT_ID;
    use iced::keyboard::{key::Named, Key, Modifiers};
    use iced_test::simulator::{simulator, Simulator};

    const CLIP: &str = "https://example.com/clip.mp4";

    /// A demo app whose note chat ends with the video `$clip`, which plays.
    fn playing(name: &str) -> BeeperGuiApp {
        use beeper_core::models::{Attachment, AttachmentSize, Message};
        let mut app = crate::fuzz::demo_app(name);
        {
            let mut st = app.store.write();
            let base = st.messages[*st.timeline_indices(NOTE_CHAT_ID).last().expect("a message")].clone();
            let size = Some(AttachmentSize { width: Some(320), height: Some(180) });
            let a =
                Attachment { attachment_type: "video".into(), src_url: Some(CLIP.into()), size, ..Default::default() };
            let sort_key = format!("{:016}", 9_000_000);
            st.upsert_message(Message { id: "$clip".into(), sort_key, attachments: vec![a], text: None, ..base });
        }
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        (app.pane.size, app.main_window) = (iced::Size::new(1000.0, 700.0), Some(window::Id::unique()));
        let _ = app.update(UiMessage::Video(Msg::Play("$clip".into(), CLIP.into(), 320, 180)));
        let path = std::env::temp_dir().join(format!("beeper-rs-{name}-{}", std::process::id()));
        let media = Arc::new(Media { path, secs: 10.0, sound: None });
        let _ = app.update(UiMessage::Video(Msg::Loaded(CLIP.into(), Ok(media))));
        app
    }

    /// Another chat than the note chat of `playing`.
    fn other_chat(app: &BeeperGuiApp) -> String {
        let st = app.store.read();
        app.visible_chats(&st).iter().find(|c| c.id != NOTE_CHAT_ID).expect("a chat").id.clone()
    }

    /// Draws the main window, scrolls the timeline up by `up` px, and hands the app what the inline video saw.
    fn scroll(app: &mut BeeperGuiApp, up: f32) -> Vec<bool> {
        use iced::mouse::{self, ScrollDelta};
        let msgs: Vec<UiMessage> = {
            let mut sim = Simulator::with_size(Default::default(), iced::Size::new(1000.0, 700.0), app.view_main());
            sim.point_at(iced::Point::new(700.0, 350.0));
            let redraw = || iced::Event::Window(window::Event::RedrawRequested(Instant::now()));
            let delta = ScrollDelta::Pixels { x: 0.0, y: up };
            let _ = sim.simulate([redraw(), iced::Event::Mouse(mouse::Event::WheelScrolled { delta }), redraw()]);
            sim.into_messages().filter(|m| matches!(m, UiMessage::Video(Msg::Seen(..)))).collect()
        };
        let seen = msgs.iter().map(|m| matches!(m, UiMessage::Video(Msg::Seen(_, true)))).collect();
        msgs.into_iter().for_each(|m| drop(app.update(m)));
        seen
    }

    /// Desktop's video menu takes the place of the message menu for a right click on the video, and its Open in PiP
    /// keeps the video in Picture in Picture while its chat stays open.
    #[test]
    fn the_video_menu_like_desktop() {
        let mut app = crate::fuzz::demo_app("video-menu");
        let chat = app.visible_chats(&app.store.read())[0].id.clone();
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let mid = {
            let st = app.store.read();
            st.timeline_indices(&chat).last().map(|&i| st.messages[i].id.clone()).expect("a message")
        };
        let url = "mxc://x/video".to_string();
        let _ = app.update(UiMessage::Video(Msg::Play(mid.clone(), url.clone(), 320, 180)));
        let path = std::env::temp_dir().join(format!("beeper-rs-video-menu-{}", std::process::id()));
        let _ = app.update(UiMessage::Video(Msg::Loaded(url, Ok(Arc::new(Media { path, secs: 10.0, sound: None })))));
        let has = |app: &BeeperGuiApp, label: &str| simulator(app.view_message_menu(&mid).0).find(label).is_ok();
        let _ = app.update(UiMessage::Video(Msg::Menu(mid.clone())));
        assert!(["Fullscreen", "Open in PiP", "Download", "Mute", "Playback Speed"].iter().all(|l| has(&app, l)));
        assert!(!has(&app, "Reply…"), "in place of the message menu");
        let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(mid.clone())));
        assert!(has(&app, "Reply…") && !has(&app, "Open in PiP"), "the message menu has no video items");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        assert!(app.pip.is_some(), "Picture in Picture opens in the chat of the video");
        let _ = app.update(UiMessage::StoreChanged);
        assert!(app.pip.is_some(), "and stays while the chat is open");
    }

    /// Desktop's Picture in Picture of a video whose chat closes: a window of 400 px on its longer side, which stays
    /// when the chat opens again until the video is in view, and closing the window stops the video.
    #[test]
    fn a_playing_video_goes_on_in_picture_in_picture_like_desktop() {
        let mut app = playing("pip");
        let other = other_chat(&app);
        assert_eq!(app.video.as_ref().map(Video::pip_size), Some((400, 225)));
        let _ = app.update(UiMessage::SelectChat(other.clone()));
        let pip = app.pip.clone().expect("a Picture in Picture window");
        assert!(app.video.as_ref().is_some_and(|v| v.playing() && v.size == (400, 225)), "the video goes on there");
        let _ = app.update(UiMessage::SelectChat(NOTE_CHAT_ID.into()));
        assert!(app.pip.is_some(), "the window waits for the video to come into view");
        assert_eq!(scroll(&mut app, 0.0), [true]);
        assert!(app.pip.is_none(), "the window closes when the video is in view");
        assert!(app.video.as_ref().is_some_and(|v| v.size == (320, 180)), "and the video goes on inline");
        let _ = app.update(UiMessage::SelectChat(other));
        let id = app.pip.clone().expect("Picture in Picture again").0;
        assert_ne!(id, pip.0);
        let _ = app.update(UiMessage::Window(windows::Msg::Closed(id)));
        assert!(app.pip.is_none() && app.video.is_none(), "closing the window stops the video");
    }

    /// Desktop's `maybeCreatePip`: a playing video that scrolls out of view goes on in Picture in Picture, and comes
    /// back inline when it scrolls into view again. A paused video stays inline.
    #[test]
    fn a_video_that_scrolls_away_goes_on_in_picture_in_picture_like_desktop() {
        let mut app = playing("pip-scroll");
        assert_eq!(scroll(&mut app, 0.0), [true], "the inline video is in view");
        assert!(app.pip.is_none());
        assert_eq!(scroll(&mut app, 600.0), [true, false], "and leaves it");
        assert!(app.pip.as_ref().is_some_and(|p| p.1 == "$clip") && !app.pip_manual, "Picture in Picture opens");
        assert!(app.video.as_ref().is_some_and(|v| v.playing() && v.size == (400, 225)), "the video goes on there");
        assert_eq!(scroll(&mut app, 0.0), [true], "back in view");
        assert!(app.pip.is_none(), "the window closes");
        assert!(app.video.as_ref().is_some_and(|v| v.playing() && v.size == (320, 180)), "the video goes on inline");
        let _ = app.update(UiMessage::Video(Msg::Toggle));
        assert_eq!(scroll(&mut app, 600.0), [true, false]);
        assert!(app.pip.is_none(), "a paused video stays inline");
    }

    /// Desktop's Open in PiP: the video stays in Picture in Picture while it is in view, and the inline video shows
    /// the "Restore from Picture in Picture" button, which brings it back.
    #[test]
    fn open_in_pip_restores_from_the_chat_like_desktop() {
        let mut app = playing("pip-restore");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        assert!(app.pip.is_some() && app.pip_manual);
        assert_eq!(scroll(&mut app, 0.0), [true]);
        assert!(app.pip.is_some(), "in view, it stays in Picture in Picture");
        let msgs: Vec<UiMessage> = {
            let mut sim = Simulator::with_size(Default::default(), iced::Size::new(1000.0, 700.0), app.view_main());
            sim.point_at(iced::Point::new(700.0, 480.0));
            let _ = sim.simulate(iced_test::simulator::click());
            sim.into_messages().collect()
        };
        assert!(msgs.iter().any(|m| matches!(m, UiMessage::Video(Msg::Restore))), "a click on the video restores it");
        assert!(!msgs.iter().any(|m| matches!(m, UiMessage::Video(Msg::Open(..)))), "in place of the carousel");
        let _ = app.update(UiMessage::Video(Msg::Restore));
        assert!(app.pip.is_none() && !app.pip_manual);
        assert!(app.video.as_ref().is_some_and(|v| v.playing() && v.size == (320, 180)), "the video goes on inline");
    }

    /// Desktop's video menu in the Picture in Picture window: Show in Chat, Download, Mute and Playback Speed. Show in
    /// Chat opens the chat of the video, where the video goes on. Escape closes the menu before the window.
    #[test]
    fn the_picture_in_picture_menu_like_desktop() {
        let mut app = playing("pip-menu");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        let _ = app.update(UiMessage::SelectChat(other_chat(&app)));
        let _ = app.update(UiMessage::Video(Msg::PipMenu(true)));
        let mut sim = simulator(app.view_pip());
        assert!(["Show in Chat", "Download", "Mute", "Playback Speed"].iter().all(|&l| sim.find(l).is_ok()));
        assert!(sim.find("Fullscreen").is_err() && sim.find("Open in PiP").is_err());
        let _ = sim.click("Show in Chat").expect("the Show in Chat item");
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        msgs.into_iter().for_each(|m| drop(app.update(m)));
        assert!(app.pip.is_none() && app.pip_menu.is_none(), "the window closes");
        assert_eq!(app.pane.selected_chat_id.as_deref(), Some(NOTE_CHAT_ID), "on the chat of the video");
        assert!(app.video.as_ref().is_some_and(|v| v.playing() && v.size == (320, 180)), "where the video goes on");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        let id = app.pip.as_ref().expect("Picture in Picture").0;
        let _ = app.update(UiMessage::Video(Msg::PipMenu(true)));
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), Modifiers::empty()));
        assert!(app.pip_menu.is_none() && app.pip.is_some(), "Escape closes the menu");
        let _ = app.update(UiMessage::Video(Msg::ClosePip));
        assert!(app.pip.is_none() && app.video.is_none(), "Close stops the video");
    }

    /// Desktop closes Picture in Picture when the carousel opens and when another audio track plays.
    #[test]
    fn the_carousel_and_audio_close_picture_in_picture_like_desktop() {
        let mut app = playing("pip-carousel");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel("$clip".into(), 0, false)));
        assert!(app.pip.is_none() && app.video.is_none(), "the carousel closes it");
        use crate::voice;
        let mut app = playing("pip-audio");
        let _ = app.update(UiMessage::Video(Msg::OpenPip));
        let clip = voice::Clip { samples: vec![0.0; 480].into(), peaks: vec![1.0].into() };
        let _ = app.update(UiMessage::Voice(voice::Msg::Loaded("mxc://a".into(), Ok(clip))));
        let _ =
            app.update(UiMessage::Voice(voice::Msg::Toggle(NOTE_CHAT_ID.into(), "$voice".into(), "mxc://a".into())));
        assert!(app.pip.is_none() && app.video.is_none(), "a playing track closes it");
    }
}

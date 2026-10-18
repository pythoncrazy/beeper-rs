//! Desktop's inline video player (`Video/index.tsx`): frames streamed from ffmpeg, the sound track, and the hover
//! controls.
use crate::i18n::tr;
use crate::style::{btn_style, fill_box, shadow_box, CLEAR};
use crate::{ic, icon, menu_item, menu_rule, sound, tinted, BeeperGuiApp, El, UiMessage, MENU_ITEM};
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
const HZ: f32 = 48_000.0;

/// Desktop's `replay10`, `forward10`, `more` and `full-screen`.
static GLYPHS: LazyLock<[svg::Handle; 4]> = LazyLock::new(|| {
    [
        "M10 18.333a7.3 7.3 0 0 1-2.927-.594 7.6 7.6 0 0 1-2.375-1.604 7.6 7.6 0 0 1-1.604-2.375 7.3 7.3 0 0 1-.594-2.927h1.667q0 2.437 1.698 4.135T10 16.666t4.135-1.698 1.698-4.135q0-2.437-1.698-4.135T10 5h-.125l1.292 1.291L10 7.5 6.667 4.166 10 .833l1.167 1.208-1.292 1.292H10q1.563 0 2.927.594t2.375 1.604a7.6 7.6 0 0 1 1.604 2.375q.594 1.364.594 2.927a7.3 7.3 0 0 1-.594 2.927 7.6 7.6 0 0 1-1.604 2.375 7.6 7.6 0 0 1-2.375 1.604 7.3 7.3 0 0 1-2.927.594m-2.5-5v-3.75H6.25v-1.25h2.5v5zm2.917 0a.8.8 0 0 1-.594-.24.8.8 0 0 1-.24-.593V9.166q0-.354.24-.593.24-.24.594-.24h1.666q.354 0 .594.24t.24.593V12.5q0 .354-.24.593a.8.8 0 0 1-.594.24zm.416-1.25h.834v-2.5h-.834z",
        "M7.5 13.332v-3.75H6.25v-1.25h2.5v5zm2.917 0a.8.8 0 0 1-.594-.24.8.8 0 0 1-.24-.593V9.165q0-.354.24-.593.24-.24.594-.24h1.666q.354 0 .594.24t.24.593V12.5q0 .354-.24.593a.8.8 0 0 1-.594.24zm.416-1.25h.834v-2.5h-.834zM10 18.332a7.3 7.3 0 0 1-2.927-.594 7.6 7.6 0 0 1-2.375-1.604 7.6 7.6 0 0 1-1.604-2.375 7.3 7.3 0 0 1-.594-2.927q0-1.562.594-2.927.593-1.364 1.604-2.375a7.6 7.6 0 0 1 2.375-1.604A7.3 7.3 0 0 1 10 3.332h.125L8.833 2.04 10 .832l3.333 3.333L10 7.5 8.833 6.29 10.125 5H10q-2.437 0-4.135 1.698t-1.698 4.135 1.698 4.135T10 16.665t4.135-1.698 1.698-4.135H17.5a7.3 7.3 0 0 1-.594 2.927 7.6 7.6 0 0 1-1.604 2.375 7.6 7.6 0 0 1-2.375 1.604 7.3 7.3 0 0 1-2.927.594",
        "M10 16.666q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 15q0-.688.49-1.177t1.177-.49 1.177.49q.49.489.49 1.177 0 .687-.49 1.177t-1.177.49m0-5q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 10q0-.688.49-1.177T10 8.333t1.177.49q.49.489.49 1.177 0 .687-.49 1.177t-1.177.49m0-5q-.687 0-1.177-.49A1.6 1.6 0 0 1 8.333 5q0-.688.49-1.177T10 3.333t1.177.49q.49.489.49 1.177 0 .687-.49 1.177T10 6.667",
        "M2.43 17.33v-4.139h1.656v2.484H6.57v1.655zm10.762 0v-1.655h2.484V13.19h1.655v4.14zM2.431 6.57v-4.14H6.57v1.656H4.086V6.57zm13.245 0V4.085h-2.484V2.43h4.14v4.139z",
    ]
    .map(|d| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});

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
    url: String,
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
    /// The inline frame size while the video fills the screen.
    full: Option<(u32, u32)>,
    /// The "Playback Speed" submenu is open.
    pub sub: bool,
}

#[derive(Clone)]
pub enum Msg {
    Play(String, String, u32, u32),
    Loaded(String, Result<Arc<Media>, String>),
    Toggle,
    Skip(f32),
    Seek(f32),
    SeekEnd,
    Tick,
    Rate(f32),
    Mute,
    Fullscreen,
    Sub,
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
fn probe(url: &str, bytes: &[u8]) -> Result<Media, String> {
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
    pub fn playing(&self) -> bool {
        self.since.is_some()
    }

    pub fn is_full(&self) -> bool {
        self.full.is_some()
    }

    fn pos(&self) -> f32 {
        self.at + self.since.map_or(0.0, |s| s.elapsed().as_secs_f32() * self.rate)
    }

    /// Pause at the current position.
    pub fn pause(&mut self) {
        if self.since.is_some() {
            (self.at, self.since) = (self.pos(), None);
            sound::voice(None);
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
                let client = self.store.read().unwrap().matrix.clone();
                let u = url.clone();
                self.video = Some(Video {
                    mid,
                    url: url.clone(),
                    size: (w, h),
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
                    sub: false,
                });
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
                (Some(_), Err(e)) => {
                    eprintln!("video failed: {e}");
                    self.video = None;
                    self.status_banner = tr("Something went wrong displaying video").into();
                }
                (None, _) => {}
            },
            Msg::Toggle => {
                let v = self.video.as_mut().expect("a played video");
                if v.since.is_some() {
                    v.pause();
                } else if v.media.is_some() {
                    v.since = Some(Instant::now());
                    v.restart();
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
                self.pane.message_menu = None;
            }
            Msg::Mute => {
                let v = self.video.as_mut().expect("a played video");
                v.muted ^= true;
                v.output();
                self.pane.message_menu = None;
            }
            Msg::Sub => self.video.as_mut().expect("a played video").sub = true,
            Msg::Fullscreen => {
                let v = self.video.as_mut().expect("a played video");
                let (mode, size) = match v.full.take() {
                    Some(inline) => (window::Mode::Windowed, inline),
                    None => {
                        v.full = Some(v.size);
                        let (w, h) = (v.size.0 as f32, v.size.1 as f32);
                        let k = (self.pane.size.width / w).min(self.pane.size.height / h);
                        (window::Mode::Fullscreen, ((w * k) as u32, (h * k) as u32))
                    }
                };
                (v.at, v.since, v.size) = (v.pos(), v.since.map(|_| Instant::now()), size);
                v.restart();
                self.pane.message_menu = None;
                return window::set_mode(self.main_window.expect("the main window is open"), mode);
            }
            Msg::Tick => {
                let v = self.video.as_mut().expect("only a played video ticks");
                if v.pos() >= v.media.as_ref().expect("a loaded video").secs {
                    (v.at, v.since) = (0.0, None);
                    v.restart();
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

    pub(crate) fn video_ticks(&self) -> Option<Subscription<UiMessage>> {
        let v = self.video.as_ref().filter(|v| v.media.is_some() && (v.playing() || v.shown == 0));
        v.map(|_| iced::time::every(Duration::from_secs_f32(1.0 / FPS)).map(|_| UiMessage::Video(Msg::Tick)))
    }

    /// The video of message `mid` over `poster`, `w` by `h`.
    pub(crate) fn view_video<'a>(&self, mid: &str, url: &str, poster: El<'a>, w: f32, h: f32) -> El<'a> {
        let v = self.video.as_ref().filter(|v| v.url == url);
        let r = if v.is_some_and(|v| v.full.is_some()) { 0.0 } else { 17.0 };
        let base: El<'a> = match v.and_then(|v| v.frame.clone()) {
            Some(f) => image(f).width(w).height(h).border_radius(r).into(),
            None => poster,
        };
        let Some(v) = v else {
            let play = button(tinted(&crate::voice::ICONS[0], 20.0, Color::from_rgb8(0x20, 0x21, 0x24)))
                .padding(8)
                .style(btn_style([Color::from_rgb8(0xef, 0xef, 0xf0); 3], [Color::BLACK; 3], 18.0))
                .on_press(UiMessage::Video(Msg::Play(mid.into(), url.into(), w.round() as u32, h.round() as u32)));
            return stack![base, container(play).center_x(w).center_y(h)].into();
        };
        let on = |glyph: &svg::Handle, msg: UiMessage| -> El<'a> {
            button(tinted(glyph, 20.0, Color::WHITE))
                .padding(0)
                .style(btn_style([CLEAR; 3], [Color::WHITE; 3], 0.0))
                .on_press_maybe(v.media.is_some().then_some(msg))
                .into()
        };
        let skip = |i: usize, by| on(&GLYPHS[i], UiMessage::Video(Msg::Skip(by)));
        let toggle = on(&crate::voice::ICONS[usize::from(v.playing())], UiMessage::Video(Msg::Toggle));
        let skips = row![skip(0, -10.0), toggle, skip(1, 10.0)];
        let corners = row![
            on(&GLYPHS[2], UiMessage::OpenMessageMenu(mid.into())),
            Space::new().width(Fill),
            on(&GLYPHS[3], UiMessage::Video(Msg::Fullscreen))
        ];
        let frac = v.media.as_ref().map_or(0.0, |m| (v.pos() / m.secs).clamp(0.0, 1.0));
        let bar = |c: Color| container(Space::new()).width(Fill).height(3).style(fill_box(c, 100.0));
        let progress = bar(Color::WHITE).width((w - 20.0) * frac);
        let seek = slider(0.0..=1.0, frac, |f| UiMessage::Video(Msg::Seek(f)))
            .step(0.001_f32)
            .height(11)
            .on_release(UiMessage::Video(Msg::SeekEnd))
            .style(crate::style::hidden_slider);
        let progress = stack![container(stack![bar(Color::WHITE.scale_alpha(0.2)), progress]).center_y(11), seek];
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
        let surface = button(base).padding(0).style(btn_style([CLEAR; 3], [CLEAR; 3], r));
        let surface = surface.on_press_maybe(v.media.is_some().then_some(UiMessage::Video(Msg::Toggle)));
        let surface = mouse_area(surface).on_right_press(UiMessage::OpenMessageMenu(mid.into()));
        hover(surface, column![container(corners).padding(8), Space::new().height(Fill), controls].width(w).height(h))
    }

    /// The video that fills the screen, like Desktop's `screenfull` toggle on the `<video>`.
    pub(crate) fn view_full_video(&self) -> Option<El<'_>> {
        let v = self.video.as_ref().filter(|v| v.full.is_some())?;
        let (w, h) = (v.size.0 as f32, v.size.1 as f32);
        let k = (self.pane().size.width / w).min(self.pane().size.height / h);
        let video = self.view_video(&v.mid, &v.url, Space::new().into(), w * k, h * k);
        Some(container(video).center(Fill).style(fill_box(Color::BLACK, 0.0)).into())
    }

    /// The video of message `mid` while it has a menu.
    fn menu_video(&self, mid: &str) -> Option<&Video> {
        self.video.as_ref().filter(|v| v.mid == mid && v.media.is_some())
    }

    /// Desktop's video menu items above the message menu: Fullscreen, Download, Mute and Playback Speed.
    pub(crate) fn video_menu_rows(&self, mid: &str) -> Option<Vec<(El<'_>, f32)>> {
        let v = self.menu_video(mid)?;
        let p = &self.pal;
        let item = |label, hotkey, msg| (menu_item(p, None, tr(label).into(), hotkey, Some(msg)), MENU_ITEM);
        let speed = mouse_area(menu_item(p, None, tr("Playback Speed").into(), "›", Some(UiMessage::Video(Msg::Sub))))
            .on_enter(UiMessage::Video(Msg::Sub));
        Some(vec![
            item(
                if v.full.is_some() { "Exit Fullscreen" } else { "Fullscreen" },
                "",
                UiMessage::Video(Msg::Fullscreen),
            ),
            item("Download", "", UiMessage::Download(mid.into(), None)),
            item(if v.muted { "Unmute" } else { "Mute" }, "", UiMessage::Video(Msg::Mute)),
            (speed.into(), MENU_ITEM),
            (menu_rule(p), 11.0),
        ])
    }

    /// The "Playback Speed" submenu, level with its row.
    pub(crate) fn view_speed_menu(&self, mid: &str) -> Option<El<'_>> {
        let v = self.menu_video(mid).filter(|v| v.sub)?;
        let p = &self.pal;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let rates = crate::voice::RATES.map(|r| {
            let label = if r == 1.0 { tr("Normal").into() } else { format!("{r}x") };
            menu_item(p, lead(v.rate == r), label, "", Some(UiMessage::Video(Msg::Rate(r))))
        });
        let list = container(Column::with_children(rates).padding([4, 0])).width(SPEED_MENU_W);
        Some(
            column![Space::new().height(3.0 * MENU_ITEM), list.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))]
                .into(),
        )
    }
}

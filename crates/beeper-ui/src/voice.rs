//! Desktop's audio player (`Audio.tsx`): decoding, the waveform, playback speed, and autoplay of the next
//! voice note.
use crate::style::{btn_style, fill_box, hidden_slider, ic, icon, semi, shadow_box, text_width, txt, CLEAR};
use crate::{menu_item, sound, tinted, tr, BeeperGuiApp, El, UiMessage, AVATAR_SLOTS, MENU_ITEM};
use beeper_core::models::{Attachment, Message};
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, slider, stack, svg, Column, Row, Space};
use iced::{Center, Color, Fill, Padding, Pixels, Point, Subscription, Task};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// `PlaybackControl`'s speeds.
pub(crate) const RATES: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
pub(crate) const HZ: usize = 48_000;
const PEAKS: usize = 1024;
/// `.waveform-container`'s min-width.
const WAVE_W: f32 = 165.0;

/// Desktop's `play-fill`, `pause-fill`, `rewind`, `forward` and `close`.
pub(crate) static ICONS: LazyLock<[svg::Handle; 5]> = LazyLock::new(|| {
    [
        ("0 0 20 20", "M6.667 15.834V4.167L15.833 10z"),
        ("0 0 20 20", "M11.667 15.834V4.167H15v11.667zm-6.667 0V4.167h3.333v11.667z"),
        ("0.167 0 20 20", "M4.75 15V5h1.667v10zm10.833 0-7.5-5 7.5-5z"),
        ("0.5 0 20 20", "M14.25 15V5h1.667v10zm-9.167 0V5l7.5 5z"),
        ("0 0 20 20", "m5.333 15.834-1.166-1.167L8.833 10 4.167 5.334l1.166-1.167L10 8.834l4.667-4.667 1.166 1.167L11.167 10l4.666 4.667-1.166 1.167L10 11.167z"),
    ]
    .map(|(view, d)| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});
/// `GlobalAudioControls` hides after a track ends and nothing plays for this long.
const HIDE_AFTER: Duration = Duration::from_secs(10);

/// Decoded audio: mono 48 kHz samples and their normalized peaks.
#[derive(Clone)]
pub struct Clip {
    samples: Arc<[f32]>,
    pub(crate) peaks: Arc<[f32]>,
}

impl Clip {
    pub(crate) fn new(samples: Vec<f32>) -> Self {
        let peaks: Vec<f32> =
            samples.chunks(samples.len().div_ceil(PEAKS)).map(|c| c.iter().fold(0f32, |m, s| m.max(s.abs()))).collect();
        let top = peaks.iter().copied().fold(f32::EPSILON, f32::max);
        Clip { samples: samples.into(), peaks: peaks.iter().map(|p| p / top).collect() }
    }

    pub(crate) fn secs(&self) -> f32 {
        self.samples.len() as f32 / HZ as f32
    }
}

/// The current track: the audio that plays or last played, at `at` seconds when playback started at `since`.
struct Track {
    chat: String,
    mid: String,
    url: String,
    at: f32,
    since: Option<Instant>,
}

pub struct Player {
    clips: HashMap<String, Option<Result<Clip, String>>>,
    now: Option<Track>,
    rate: f32,
    pub menu: Option<Point>,
    /// The Playback Speed submenu of the message menu is open.
    pub sub: bool,
    /// `menu` is the right-click menu of the global player.
    pub pill: bool,
    /// The global player is closed until a track plays.
    closed: bool,
    /// When the current track ended, while nothing plays since.
    ended: Option<Instant>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            clips: HashMap::new(),
            now: None,
            rate: 1.0,
            menu: None,
            sub: false,
            pill: false,
            closed: false,
            ended: None,
        }
    }
}

#[derive(Clone)]
pub enum Msg {
    Loaded(String, Result<Clip, String>),
    Toggle(String, String, String),
    Seek(String, String, String, f32),
    SeekEnd,
    Menu,
    PillMenu,
    Sub,
    Rate(f32),
    Tick,
    Skip(bool),
    Close,
    Hide,
}

/// The `-f f32le` arguments that make ffmpeg write mono `HZ` samples.
pub(crate) const PCM: [&str; 7] = ["-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"];

/// The samples of ffmpeg's `PCM` output.
pub(crate) fn samples(pcm: &[u8]) -> Vec<f32> {
    pcm.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

/// Decode any audio, like the Ogg Opus voice notes of every network, to mono.
pub fn decode(bytes: &[u8]) -> Result<Clip, String> {
    Ok(Clip::new(samples(&crate::run_ffmpeg(bytes, &PCM)?)))
}

/// Desktop's duration format, `00:05` or `01:00:05`.
pub(crate) fn clock(secs: f32) -> String {
    let s = secs as u32;
    let hours = if s >= 3600 { format!("{:02}:", s / 3600) } else { String::new() };
    format!("{hours}{:02}:{:02}", s % 3600 / 60, s % 60)
}

/// The heights of WaveSurfer's bars over `width`, as (bar width, gap, height, minimum height).
pub(crate) fn bars(peaks: &[f32], width: f32, (bar, gap, tall, min): (f32, f32, f32, f32)) -> Vec<f32> {
    let n = ((width + gap) / (bar + gap)) as usize;
    (0..n)
        .map(|k| {
            let span = &peaks[k * peaks.len() / n..((k + 1) * peaks.len() / n).max(k * peaks.len() / n + 1)];
            (span.iter().copied().fold(0.0, f32::max) * tall).round().max(min)
        })
        .collect()
}

/// `Audio.tsx`'s WaveSurfer: 2px bars with 2px gaps, 20px tall and at least 8px.
const BUBBLE_BARS: (f32, f32, f32, f32) = (2.0, 2.0, 20.0, 8.0);

impl Player {
    pub(crate) fn clip(&self, url: &str) -> Option<&Clip> {
        self.clips.get(url)?.as_ref()?.as_ref().ok()
    }

    /// The position of `url`, and whether it plays, when it is the current track.
    pub(crate) fn state(&self, url: &str) -> Option<(f32, bool)> {
        self.now.as_ref().filter(|t| t.url == url).map(|t| (self.pos(t), t.since.is_some()))
    }

    /// Forget `url`, and stop it when it is the current track.
    pub(crate) fn forget(&mut self, url: &str) {
        if self.now.as_ref().is_some_and(|t| t.url == url) {
            self.pause();
            self.now = None;
        }
        self.clips.remove(url);
    }

    fn pos(&self, t: &Track) -> f32 {
        let secs = self.clip(&t.url).expect("the current track is decoded").secs();
        (t.at + t.since.map_or(0.0, |s| s.elapsed().as_secs_f32() * self.rate)).min(secs)
    }

    /// Restart the output at the current position, or stop it when paused.
    fn output(&self) {
        let t = self.now.as_ref().expect("a current track");
        let samples = &self.clip(&t.url).expect("the current track is decoded").samples;
        let from = ((self.pos(t) * HZ as f32) as usize).min(samples.len());
        sound::voice(t.since.map(|_| (samples.clone(), from, self.rate)));
    }

    /// Pause at the current position, if playing.
    pub(crate) fn pause(&mut self) {
        let Some(at) = self.now.as_ref().filter(|t| t.since.is_some()).map(|t| self.pos(t)) else { return };
        let t = self.now.as_mut().expect("a current track");
        (t.at, t.since) = (at, None);
        sound::voice(None);
    }
}

impl BeeperGuiApp {
    pub(crate) fn voice(&mut self, msg: Msg) -> Task<UiMessage> {
        if let (Msg::Toggle(..) | Msg::SeekEnd, Some(v)) = (&msg, self.video.as_mut()) {
            v.pause();
        }
        let audio = &mut self.audio;
        match msg {
            Msg::Loaded(url, clip) => {
                audio.clips.insert(url, Some(clip));
            }
            Msg::Toggle(chat, mid, url) => match &mut audio.now {
                Some(t) if t.mid == mid && t.since.is_some() => audio.pause(),
                Some(t) if t.mid == mid => {
                    t.since = Some(Instant::now());
                    audio.output();
                }
                _ => {
                    audio.now = Some(Track { chat, mid, url, at: 0.0, since: Some(Instant::now()) });
                    audio.output();
                }
            },
            Msg::Skip(forward) => {
                let (chat, id, url) =
                    self.neighbor(&self.store.read(), forward).expect("only a present neighbor skips");
                self.audio.pause();
                self.audio.now.as_mut().expect("a current track").at = 0.0;
                return self.voice(Msg::Toggle(chat, id, url));
            }
            Msg::Close => {
                audio.pause();
                (audio.now, audio.closed) = (None, true);
            }
            Msg::Hide => audio.closed |= audio.ended.is_some_and(|e| e.elapsed() >= HIDE_AFTER),
            Msg::Seek(chat, mid, url, frac) => {
                let at = frac * audio.clip(&url).expect("only a decoded track seeks").secs();
                match &mut audio.now {
                    Some(t) if t.mid == mid => {
                        t.at = at;
                        t.since = t.since.map(|_| Instant::now());
                    }
                    _ => {
                        sound::voice(None);
                        audio.now = Some(Track { chat, mid, url, at, since: None });
                    }
                }
            }
            Msg::SeekEnd => audio.output(),
            Msg::Menu => (audio.menu, audio.pill) = (Some(self.cursor.get()), false),
            Msg::PillMenu => (audio.menu, audio.pill, audio.sub) = (Some(self.cursor.get()), true, false),
            Msg::Sub => audio.sub = true,
            Msg::Rate(rate) => {
                let playing = audio.now.as_ref().is_some_and(|t| t.since.is_some());
                audio.pause();
                (audio.rate, audio.menu, audio.sub) = (rate, None, false);
                self.pane.message_menu = None;
                if playing {
                    audio.now.as_mut().expect("a current track").since = Some(Instant::now());
                    audio.output();
                }
            }
            Msg::Tick => {
                let t = audio.now.as_ref().expect("only a playing track ticks");
                if audio.pos(t) < audio.clip(&t.url).expect("the current track is decoded").secs() {
                    return Task::none();
                }
                audio.now = Some(Track { at: 0.0, since: None, ..audio.now.take().expect("a current track") });
                audio.ended = Some(Instant::now());
                let st = self.store.read();
                let next = self.neighbor(&st, true).filter(|_| st.settings.on("autoPlayVoiceNotes"));
                drop(st);
                return match next {
                    Some((chat, id, url)) => self.voice(Msg::Toggle(chat, id, url)),
                    None => {
                        Task::perform(async { tokio::time::sleep(HIDE_AFTER).await }, |()| UiMessage::Voice(Msg::Hide))
                    }
                };
            }
        }
        if self.audio.now.as_ref().is_some_and(|t| t.since.is_some()) {
            (self.audio.closed, self.audio.ended) = (false, None);
        }
        Task::none()
    }

    /// The decoded audio message before or after the current track in its chat, like `prevAudio` and `nextAudio`.
    fn neighbor(&self, st: &BeeperStore, forward: bool) -> Option<(String, String, String)> {
        let t = self.audio.now.as_ref()?;
        let queue = st.chat_msg_indices.get(&t.chat).into_iter().flatten().map(|&i| &st.messages[i]);
        let queue: Vec<_> = queue.filter_map(|m| Some((&m.id, audio_url(m)?))).collect();
        let k = queue.iter().position(|(id, _)| **id == t.mid)?;
        let (id, url) = if forward { queue.get(k + 1) } else { k.checked_sub(1).map(|k| &queue[k]) }?;
        self.audio.clip(url).map(|_| (t.chat.clone(), id.to_string(), url.to_string()))
    }

    /// Fetch and decode the audio of the open chat.
    pub(crate) fn load_audio(&mut self) -> Task<UiMessage> {
        let st = self.store.read();
        let Some(client) = st.matrix.clone() else { return Task::none() };
        let chat = self.pane.selected_chat_id.as_ref().and_then(|c| st.chat_msg_indices.get(c));
        let urls: Vec<String> = chat
            .into_iter()
            .flatten()
            .filter_map(|&i| audio_url(&st.messages[i]))
            .filter(|u| !self.audio.clips.contains_key(*u))
            .map(str::to_string)
            .collect();
        drop(st);
        self.audio.clips.extend(urls.iter().map(|u| (u.clone(), None)));
        Task::batch(urls.into_iter().map(|url| {
            let (client, u) = (Some(client.clone()), url.clone());
            Task::perform(
                async move {
                    let _slot = AVATAR_SLOTS.acquire().await.expect("the media semaphore stays open");
                    let bytes = beeper_core::matrix::media(client, u).await?;
                    tokio::task::spawn_blocking(move || decode(&bytes)).await.expect("audio decode task")
                },
                move |r| UiMessage::Voice(Msg::Loaded(url, r)),
            )
        }))
    }

    pub(crate) fn audio_ticks(&self) -> Option<Subscription<UiMessage>> {
        self.audio
            .now
            .as_ref()
            .filter(|t| t.since.is_some())
            .map(|_| iced::time::every(Duration::from_millis(50)).map(|_| UiMessage::Voice(Msg::Tick)))
    }

    /// Whether `a` shows as an audio player: audio that is not known to fail decoding.
    pub(crate) fn playable(&self, a: &Attachment) -> bool {
        a.attachment_type == "audio"
            && a.src_url.as_ref().is_some_and(|u| !matches!(self.audio.clips.get(u), Some(Some(Err(_)))))
    }

    /// Desktop's `.audio-player`, in the bubble colors.
    pub(crate) fn view_audio<'a>(&self, m: &Message, a: &Attachment, mine: bool, fg: Color, bg: Color) -> El<'a> {
        let url = a.src_url.clone().expect("a playable attachment has a URL");
        let clip = self.audio.clip(&url);
        let track = self.audio.now.as_ref().filter(|t| t.mid == m.id);
        let ready = track.filter(|_| clip.is_some());
        let secs = clip.map_or(a.duration.unwrap_or(0.0) as f32, Clip::secs);
        let pos = ready.map(|t| self.audio.pos(t));
        let total = clock(secs);
        let time = pos.map_or(total.clone(), |pos| format!("{}/{total}", clock(pos)));
        let rate = format!("{}x", self.audio.rate);
        let rate_w = ready.map_or(0.0, |_| 8.0 + text_width(&rate, 11.0, Weight::Semibold) + 8.0);
        let wave_w =
            (WAVE_W + text_width(&total, 11.0, Weight::Normal) - text_width(&time, 11.0, Weight::Normal) - rate_w)
                .max(0.0);
        let toggle = clip.map(|_| UiMessage::Voice(Msg::Toggle(m.chat_id.clone(), m.id.clone(), url.clone())));
        let (wave, progress) = (if mine { fg.scale_alpha(0.5) } else { self.pal.on_secondary }, fg);
        let wave: El<'a> = match clip {
            Some(c) => waveform([&m.chat_id, &m.id, &url], c, pos.unwrap_or(0.0), wave_w, (wave, progress)),
            None => Space::new().width(wave_w).height(20).into(),
        };
        let playing = track.is_some_and(|t| t.since.is_some());
        let play = button(tinted(&ICONS[usize::from(playing)], 20.0, fg))
            .padding(0)
            .style(btn_style([CLEAR; 3], [fg; 3], 0.0))
            .on_press_maybe(toggle);
        let speed = ready.map(|_| {
            button(semi(rate, 11.0).line_height(Pixels(14.0)).color(Color::WHITE))
                .padding([1, 4])
                .style(btn_style([fg.scale_alpha(0.2); 3], [Color::WHITE; 3], 2.0))
                .on_press(UiMessage::Voice(Msg::Menu))
        });
        let line = row![play, wave, txt(time, 11.0).line_height(Pixels(14.0)).color(fg)].push(speed);
        let transcript = a.transcript.clone().map(|t| {
            button(txt(t, 12.0).color(fg.scale_alpha(0.75)))
                .padding(0)
                .style(btn_style([CLEAR; 3], [fg; 3], 0.0))
                .on_press(UiMessage::RetranscribeVoice(m.id.clone()))
        });
        let width = 22.0 + 20.0 + 8.0 + WAVE_W + 8.0 + text_width(&total, 11.0, Weight::Normal);
        container(column![line.spacing(8).align_y(Center)].push(transcript).spacing(6))
            .width(width)
            .padding([8, 11])
            .style(fill_box(bg, 22.0))
            .into()
    }

    /// `PlaybackControl`'s speed menu.
    pub(crate) fn view_rate_menu(&self) -> El<'_> {
        let p = &self.pal;
        let lead = |on: bool| Some(container(on.then(|| ic(icon::CHECK, 14.0))).width(22).into());
        let items = RATES.map(|r| {
            menu_item(p, lead(self.audio.rate == r), format!("{r}x"), "", Some(UiMessage::Voice(Msg::Rate(r))))
        });
        container(Column::with_children(items).padding([4, 0]))
            .width(rate_menu_w())
            .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
            .into()
    }

    /// The Playback Speed row of the message menu of a voice message.
    pub(crate) fn voice_menu_row(&self, m: &Message) -> Option<(El<'_>, f32)> {
        m.attachments.first().filter(|a| self.playable(a))?;
        let open = UiMessage::Voice(Msg::Sub);
        let row = menu_item(&self.pal, None, tr("Playback Speed").into(), "›", Some(open.clone()));
        Some((mouse_area(row).on_enter(open).into(), MENU_ITEM))
    }

    /// The Playback Speed submenu of the message menu, `top` below the top of the menu.
    pub(crate) fn view_voice_speed(&self, top: f32) -> Option<El<'_>> {
        self.audio.sub.then(|| column![Space::new().height(top), self.view_rate_menu()].into())
    }

    /// `GlobalAudioControls`, `w` wide, with the parts that Desktop's container queries hide at that width.
    pub(crate) fn view_global_audio<'a>(&self, st: &BeeperStore, w: f32) -> Option<El<'a>> {
        let t = self.audio.now.as_ref().filter(|_| !self.audio.closed)?;
        let clip = self.audio.clip(&t.url)?;
        let (p, pos) = (&self.pal, self.audio.pos(t));
        let icon_btn = |i: usize, size: f32, color: Color, msg: Option<UiMessage>| -> El<'a> {
            let b = button(tinted(&ICONS[i], size, color)).padding(0).style(btn_style([CLEAR; 3], [color; 3], 0.0));
            b.on_press_maybe(msg).into()
        };
        let skip = |i: usize, forward: bool| {
            let msg = self.neighbor(st, forward).map(|_| UiMessage::Voice(Msg::Skip(forward)));
            icon_btn(i, 20.0, if msg.is_some() { p.strong } else { p.rule }, msg)
        };
        let toggle = UiMessage::Voice(Msg::Toggle(t.chat.clone(), t.mid.clone(), t.url.clone()));
        let play = icon_btn(usize::from(t.since.is_some()), 20.0, p.strong, Some(toggle));
        let buttons: El<'a> = match w > 101.0 {
            true => row![skip(2, false), play, skip(3, true)].align_y(Center).into(),
            false => play,
        };
        let total = format!(" / {}", clock(clip.secs()));
        let elapsed = (w > 135.0).then(|| clock(pos) + if w > 205.0 { &total } else { "" });
        let time_w = 4.0 + elapsed.as_ref().map_or(0.0, |e| text_width(e, 11.0, Weight::Normal) + 8.0) + 10.0;
        let elapsed = elapsed.map(|e| txt(e, 11.0).line_height(Pixels(14.0)).color(p.strong));
        let close = icon_btn(4, 10.0, p.strong, Some(UiMessage::Voice(Msg::Close)));
        let time = row![].push(elapsed).push(close).spacing(8).align_y(Center).padding(Padding::ZERO.left(4));
        let chat = st.resolve_chat_idx(&t.chat).map(|i| &st.chats[i]).filter(|_| w > 163.0);
        let avatar = chat.map(|c| {
            let b = button(self.chat_avatar(c, 28.0, 0.0)).padding(0).style(btn_style([CLEAR; 3], [p.strong; 3], 0.0));
            b.on_press(UiMessage::SelectChat(c.id.clone()))
        });
        let wave_w = w - 16.0 - avatar.as_ref().map_or(0.0, |_| 28.0) - if w > 101.0 { 60.0 } else { 20.0 } - time_w;
        let wave = (w > 249.0)
            .then(|| waveform([&t.chat, &t.mid, &t.url], clip, pos, wave_w.max(35.0), (p.on_secondary, p.strong)));
        let gap = || Space::new().width(Fill);
        let line = row![].push(avatar).push(gap()).push(buttons).push(gap()).push(wave).push(gap()).push(time);
        let pill = container(line.align_y(Center)).width(w).padding([6, 8]).style(fill_box(p.card, 24.0));
        Some(mouse_area(pill).on_right_press(UiMessage::Voice(Msg::PillMenu)).into())
    }

    /// The right-click menu of `GlobalAudioControls` `top` below its Playback Speed submenu.
    pub(crate) fn view_pill_menu(&self, top: f32) -> El<'_> {
        let (p, open) = (&self.pal, UiMessage::Voice(Msg::Sub));
        let item = mouse_area(menu_item(p, None, tr("Playback Speed").into(), "›", Some(open.clone()))).on_enter(open);
        let menu =
            container(column![item].padding([4, 0])).width(233).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        row![column![Space::new().height(top), menu]].push(self.audio.sub.then(|| self.view_rate_menu())).into()
    }
}

/// WaveSurfer's bars of `c` at `pos` seconds, `width` wide in the wave and progress colors, with a hidden scrubber.
fn waveform<'a>(
    [chat, mid, url]: [&str; 3],
    c: &Clip,
    pos: f32,
    width: f32,
    (wave, progress): (Color, Color),
) -> El<'a> {
    let heights = bars(&c.peaks, width, BUBBLE_BARS);
    let done = pos / c.secs() * heights.len() as f32;
    let bars = heights.iter().enumerate().fold(Row::new().spacing(2).align_y(Center), |r, (k, &h)| {
        let color = if (k as f32 + 0.5) < done { progress } else { wave };
        r.push(container(Space::new()).width(2).height(h).style(fill_box(color, 2.0)))
    });
    let [chat, mid, url] = [chat, mid, url].map(str::to_string);
    let seek = slider(0.0..=1.0, pos / c.secs(), move |v| {
        UiMessage::Voice(Msg::Seek(chat.clone(), mid.clone(), url.clone(), v))
    })
    .step(0.001_f32)
    .height(20)
    .on_release(UiMessage::Voice(Msg::SeekEnd))
    .style(hidden_slider);
    stack![container(bars).center_y(20), seek].width(width).into()
}

pub(crate) fn rate_menu_w() -> f32 {
    (10.0 + 16.0 + 22.0 + text_width("0.25x", 13.0, Weight::Normal)).max(80.0)
}

/// The URL of the first audio attachment of `m`, like Desktop's play queue entries.
fn audio_url(m: &Message) -> Option<&str> {
    m.attachments.iter().find(|a| a.attachment_type == "audio")?.src_url.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_draws_like_desktop() {
        assert_eq!([clock(5.9), clock(65.0), clock(3605.0)], ["00:05", "01:05", "01:00:05"]);
        let b = bars(&[0.0, 0.1, 1.0, 0.5], 14.0, BUBBLE_BARS);
        assert_eq!(b, [8.0, 8.0, 20.0, 10.0]);
        assert!(decode(b"not ogg").is_err());
        let clip = decode(include_bytes!("../assets/voice_test.ogg")).unwrap();
        assert!((clip.secs() - 1.5).abs() < 0.01, "{}", clip.secs());
        assert_eq!(clip.peaks.iter().copied().fold(0.0, f32::max), 1.0);
    }

    /// A demo app with two decoded 10 ms voice notes, `v0` and `v1`, in the first chat.
    fn two_notes(name: &str) -> (BeeperGuiApp, String) {
        let dir = std::env::temp_dir().join(format!("BeeperRs_voice_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = beeper_core::config::CentralConfig::new(dir, "127.0.0.1".into(), 23390, None);
        cfg.demo = true;
        let mut st = beeper_core::store::BeeperStore::from_config(cfg);
        let (chat, account) = (st.chats[0].id.clone(), st.chats[0].account_id.clone());
        for (i, url) in ["mxc://a", "mxc://b"].iter().enumerate() {
            st.upsert_message(
                serde_json::from_value(serde_json::json!({
                    "id": format!("v{i}"), "accountID": account, "chatID": chat, "senderID": "s",
                    "sortKey": beeper_core::store::sort_key(i as i64), "timestamp": "2026-01-02T03:04:05Z",
                    "attachments": [{ "type": "audio", "srcURL": url }],
                }))
                .expect("message JSON"),
            );
        }
        let mut app = BeeperGuiApp::new(beeper_core::store::StoreLock::shared(st));
        let short = Clip { samples: vec![0.0; 480].into(), peaks: vec![1.0].into() };
        for url in ["mxc://a", "mxc://b"] {
            let _ = app.voice(Msg::Loaded(url.into(), Ok(short.clone())));
        }
        (app, chat)
    }

    #[test]
    fn plays_the_next_voice_note_when_one_ends() {
        let (mut app, chat) = two_notes("autoplay");
        let ended = |app: &mut BeeperGuiApp| {
            let _ = app.voice(Msg::Toggle(chat.clone(), "v0".into(), "mxc://a".into()));
            std::thread::sleep(Duration::from_millis(20));
            let _ = app.voice(Msg::Tick);
            let t = app.audio.now.as_ref().unwrap();
            (t.mid.clone(), t.since.is_some())
        };
        assert_eq!(ended(&mut app), ("v1".into(), true));
        app.store.write().settings.set_pref("autoPlayVoiceNotes", 0);
        assert_eq!(ended(&mut app), ("v0".into(), false));
    }

    #[test]
    fn global_player_like_desktop() {
        let (mut app, chat) = two_notes("pill");
        let shows = |app: &BeeperGuiApp| app.view_global_audio(&app.store.read(), 234.0).is_some();
        assert!(!shows(&app));
        let _ = app.voice(Msg::Toggle(chat.clone(), "v0".into(), "mxc://a".into()));
        assert!(shows(&app));
        let st = app.store.read();
        assert_eq!((app.neighbor(&st, false), app.neighbor(&st, true).map(|n| n.1)), (None, Some("v1".into())));
        drop(st);
        let _ = app.voice(Msg::Skip(true));
        let t = app.audio.now.as_ref().unwrap();
        assert_eq!((t.mid.as_str(), t.since.is_some()), ("v1", true));
        let _ = app.voice(Msg::PillMenu);
        assert!(app.audio.menu.is_some() && app.audio.pill && !app.audio.sub);
        let _ = app.voice(Msg::Sub);
        let _ = app.voice(Msg::Rate(1.5));
        assert_eq!((app.audio.menu, app.audio.sub, app.audio.rate), (None, false, 1.5));
        let _ = app.voice(Msg::Close);
        assert!(app.audio.now.is_none() && !shows(&app));
        app.store.write().settings.set_pref("autoPlayVoiceNotes", 0);
        let _ = app.voice(Msg::Toggle(chat, "v1".into(), "mxc://b".into()));
        std::thread::sleep(Duration::from_millis(20));
        let _ = app.voice(Msg::Tick);
        let _ = app.voice(Msg::Hide);
        assert!(shows(&app), "the player stays for ten seconds after the end");
        app.audio.ended = Some(Instant::now() - HIDE_AFTER);
        let _ = app.voice(Msg::Hide);
        assert!(!shows(&app));
    }
}

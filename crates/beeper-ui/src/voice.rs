//! Desktop's audio player (`Audio.tsx`): decoding, the waveform, playback speed, and autoplay of the next
//! voice note.
use crate::settings;
use crate::style::rem;
use crate::style::{btn_style, clamped, fill_box, hidden_slider, ic, icon, semi, shadow_box, text_width, txt, Bare};
use crate::{menu_item, menu_pitch, sound, tinted, tr, BeeperGuiApp, El, UiMessage, AVATAR_SLOTS};
use beeper_core::models::{Attachment, Message, Transcription};
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, mouse_area, row, slider, stack, svg, Column, Row, Space};
use iced::{Center, Color, Fill, Padding, Pixels, Point, Subscription, Task};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// `PlaybackControl`'s speeds.
pub(crate) const RATES: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
pub(crate) const HZ: usize = 48_000;
const PEAKS: usize = 1024;
/// `.waveform-container`'s min-width.
const WAVE_W: f32 = 165.0;
/// `.transcribed-message`'s max-width and line clamp.
const QUOTE_W: f32 = 290.0;
const QUOTE_LINES: usize = 40;

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
    pub(crate) samples: Arc<[f32]>,
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

#[derive(Default)]
pub struct Player {
    clips: HashMap<String, Option<Result<Clip, String>>>,
    now: Option<Track>,
    /// Desktop's `lastPosition` of the other tracks of the chat of the current track, by message.
    stops: HashMap<String, f32>,
    rate: f32,
    pub menu: Option<Point>,
    /// The Playback Speed submenu of the message menu is open.
    pub sub: bool,
    /// The Transcription Language submenu of the message menu is open.
    pub lang: bool,
    /// `menu` is the right-click menu of the global player.
    pub pill: bool,
    /// The global player is closed until a track plays.
    closed: bool,
    /// When the current track ended, while nothing plays since.
    ended: Option<Instant>,
    /// Desktop's `audio-player-position`: the global player is this far above the bottom of the sidebar.
    pub bottom: f32,
    /// The drag of the global player: the cursor height and `bottom` at the press, and if the drag started.
    grab: Option<(f32, f32, bool)>,
    /// Where `bottom` is saved.
    saved: PathBuf,
    /// The messages with a clamped transcript that shows all its lines.
    more: HashSet<String>,
    /// Desktop's `aiStore.transcriptionState` by message: the language, and whether it failed, else it is in flight.
    transcribing: HashMap<String, (String, bool)>,
    /// The transcription that waits for the privacy notice: the message, a new transcription, and the language.
    pub(crate) asked: Option<(String, bool, Option<String>)>,
}

/// Desktop's `activationConstraint`: a drag starts after the cursor moves this far.
const DRAG_START: f32 = 5.0;
/// The height of `GlobalAudioControls` with the chat avatar.
const PILL_H: f32 = 40.0;

impl Player {
    /// The player, with the position of the global player that `data` keeps, or Desktop's 8 px.
    pub(crate) fn new(data: &Path) -> Self {
        let saved = data.join("audio-player-position");
        let bottom = match std::fs::read_to_string(&saved) {
            Ok(json) => {
                let at: serde_json::Value = serde_json::from_str(&json).expect("a saved audio player position");
                at["y"].as_f64().expect("a numeric audio player position") as f32
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 8.0,
            Err(e) => panic!("read {}: {e}", saved.display()),
        };
        Self { rate: 1.0, bottom, saved, ..Default::default() }
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
    LangSub,
    /// A language of the Transcription Language submenu: a new transcription of the message in that language.
    Language(String, String),
    Rate(f32),
    Tick,
    Skip(bool),
    Close,
    Hide,
    /// A press on the global player, a cursor move while it is pressed, and the release.
    Grab,
    Drag(f32),
    Drop,
    /// "Show more" or "Show less" of the transcript of a message.
    More(String),
    /// Desktop's `transcribeAttachment` of a message: again when true, in a language or else the default one.
    Transcribe(String, bool, Option<String>),
    /// The transcript of a message in a language, or the error toast, which an automatic transcription does not show.
    Transcribed(String, String, Result<String, Option<String>>),
    /// The answer to the privacy notice of transcriptions.
    Allow(bool),
    /// A frame of the spinner of a transcription.
    Spin,
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
    /// Makes `track` the current track, like Desktop's `toggleTrack`: the track that played keeps its position, and a
    /// track of another chat forgets the positions of the old chat. The new track starts where it stopped.
    fn switch(&mut self, mut track: Track) {
        if let Some(old) = self.now.take() {
            if old.chat == track.chat {
                self.stops.insert(old.mid.clone(), self.pos(&old));
            } else {
                self.stops.clear();
            }
        }
        if let Some(at) = self.stops.remove(&track.mid) {
            track.at = at;
        }
        self.now = Some(track);
    }

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
            Msg::Transcribe(mid, again, language) => return self.transcribe(mid, again, language, false),
            Msg::Transcribed(mid, language, Ok(text)) => {
                self.audio.transcribing.remove(&mid);
                let t =
                    Transcription { transcription: text, engine: "openai-whisper".into(), language: Some(language) };
                self.store.write().transcribed(&mid, t);
                return Task::none();
            }
            Msg::Transcribed(mid, language, Err(toast)) => {
                self.audio.transcribing.insert(mid, (language, true));
                self.status_banner = toast.unwrap_or_default();
                return Task::none();
            }
            Msg::Allow(on) => {
                self.confirm = None;
                // Without a voice note, the notice asks for Talk to Type.
                let asked = self.audio.asked.take();
                let set =
                    self.handle(UiMessage::Settings(settings::Msg::SetPref("transcribeWithOpenai", u8::from(on))));
                let next = match asked {
                    _ if !on => Task::none(),
                    Some((mid, again, language)) => self.transcribe(mid, again, language, false),
                    None => self.record(crate::record::Msg::Talk),
                };
                return Task::batch([set, next]);
            }
            Msg::Spin => {}
            Msg::Loaded(url, clip) => {
                audio.clips.insert(url, Some(clip));
            }
            Msg::Toggle(chat, mid, url) => match &mut audio.now {
                Some(t) if t.mid == mid && t.since.is_some() => audio.pause(),
                Some(t) if t.mid == mid => {
                    t.since = Some(Instant::now());
                    audio.output();
                }
                // Desktop's `toggleTrack` closes Picture in Picture when another track plays.
                _ => {
                    audio.pause();
                    audio.switch(Track { chat, mid, url, at: 0.0, since: Some(Instant::now()) });
                    audio.output();
                    self.stop_pip();
                }
            },
            Msg::Skip(forward) => {
                let (chat, id, url) =
                    self.neighbor(&self.store.read(), forward).expect("only a present neighbor skips");
                return self.voice(Msg::Toggle(chat, id, url));
            }
            Msg::Close => {
                audio.pause();
                (audio.now, audio.closed) = (None, true);
            }
            Msg::Hide => audio.closed |= audio.ended.is_some_and(|e| e.elapsed() >= HIDE_AFTER),
            // Desktop drags the global player up and down in the sidebar and saves where it is.
            Msg::Grab => audio.grab = Some((self.cursor.get().y, audio.bottom, false)),
            Msg::Drag(y) => {
                let (y0, b0, started) = audio.grab.expect("a pressed player drags");
                if started || (y0 - y).abs() >= DRAG_START {
                    let top = (self.pane.size.height - PILL_H - 8.0).max(0.0);
                    (audio.bottom, audio.grab) = ((b0 + y0 - y).clamp(0.0, top), Some((y0, b0, true)));
                }
            }
            Msg::Drop => {
                if audio.grab.take().expect("a pressed player drops").2 {
                    let at = serde_json::json!({ "x": 8, "y": audio.bottom }).to_string();
                    std::fs::write(&audio.saved, at).unwrap_or_else(|e| panic!("write {}: {e}", audio.saved.display()));
                }
            }
            Msg::Seek(chat, mid, url, frac) => {
                let at = frac * audio.clip(&url).expect("only a decoded track seeks").secs();
                match &mut audio.now {
                    Some(t) if t.mid == mid => {
                        t.at = at;
                        t.since = t.since.map(|_| Instant::now());
                    }
                    _ => {
                        audio.pause();
                        audio.switch(Track { chat, mid, url, at, since: None });
                        audio.now.as_mut().expect("the new track").at = at;
                    }
                }
            }
            Msg::SeekEnd => audio.output(),
            Msg::Menu => (audio.menu, audio.pill) = (Some(self.cursor.get()), false),
            Msg::PillMenu => (audio.menu, audio.pill, audio.sub) = (Some(self.cursor.get()), true, false),
            Msg::Sub => (audio.sub, audio.lang, self.pane.remind_sub) = (true, false, false),
            Msg::LangSub => (audio.sub, audio.lang, self.pane.remind_sub) = (false, true, false),
            // Desktop's `retranscribeWithLanguage`: the paywall when no free transcriptions are left.
            Msg::Language(mid, code) => {
                (audio.lang, self.pane.message_menu) = (false, None);
                let used_up = self.store.read().features.free_uses("voice-transcription").is_some_and(|(u, l)| u >= l);
                if used_up {
                    return self.handle(UiMessage::Settings(settings::Msg::Open(crate::SettingsTab::BeeperPlus)));
                }
                if !self.audio.transcribing.get(&mid).is_some_and(|s| !s.1) {
                    return self.transcribe(mid, true, Some(code), false);
                }
            }
            Msg::More(id) => {
                if !audio.more.remove(&id) {
                    audio.more.insert(id);
                }
            }
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

    /// The chat of the current track.
    pub(crate) fn track_chat(&self) -> Option<&String> {
        self.audio.now.as_ref().map(|t| &t.chat)
    }

    /// The decoded audio message before or after the current track in its chat, like `prevAudio` and `nextAudio`.
    fn neighbor(&self, st: &BeeperStore, forward: bool) -> Option<(String, String, String)> {
        let t = self.audio.now.as_ref()?;
        let queue = st.timeline(&t.chat);
        let queue: Vec<_> = queue.filter_map(|m| Some((&m.id, audio_url(m)?))).collect();
        let k = queue.iter().position(|(id, _)| **id == t.mid)?;
        let (id, url) = if forward { queue.get(k + 1) } else { k.checked_sub(1).map(|k| &queue[k]) }?;
        self.audio.clip(url).map(|_| (t.chat.clone(), id.to_string(), url.to_string()))
    }

    /// Fetch and decode the audio of the open chat.
    pub(crate) fn load_audio(&mut self) -> Task<UiMessage> {
        let st = self.store.read();
        let Some(client) = st.matrix.clone() else { return Task::none() };
        let chat = self.pane.selected_chat_id.as_ref().map(|c| st.timeline_indices(c));
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

    /// The cursor moves and the release while the global player is pressed.
    pub(crate) fn audio_drag(&self) -> Option<Subscription<UiMessage>> {
        self.audio.grab.map(|_| {
            iced::event::listen_with(|e, _, _| match e {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(UiMessage::Voice(Msg::Drag(position.y)))
                }
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(_)) => Some(UiMessage::Voice(Msg::Drop)),
                _ => None,
            })
        })
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

    /// Desktop's `.audio-player`, in the bubble colors, with the time `ts` in its color.
    pub(crate) fn view_audio<'a>(
        &self,
        m: &Message,
        a: &Attachment,
        mine: bool,
        ts: Option<(String, Color)>,
        fg: Color,
        bg: Color,
    ) -> El<'a> {
        let url = a.src_url.clone().expect("a playable attachment has a URL");
        let clip = self.audio.clip(&url);
        let track = self.audio.now.as_ref().filter(|t| t.mid == m.id);
        let ready = track.filter(|_| clip.is_some());
        let secs = clip.map_or(a.duration.unwrap_or(0.0) as f32, Clip::secs);
        let pos = ready.map(|t| self.audio.pos(t));
        // Desktop's waveform shows where a track stopped.
        let stop = self.audio.stops.get(&m.id).copied().filter(|_| clip.is_some());
        let total = clock(secs);
        let time = pos.map_or(total.clone(), |pos| format!("{}/{total}", clock(pos)));
        let rate = format!("{}x", self.audio.rate);
        let rate_w = ready.map_or(0.0, |_| 8.0 + text_width(&rate, 11.0, Weight::Semibold) + 8.0);
        let ts_w = ts.as_ref().map_or(0.0, |(t, _)| text_width(t, 11.0, Weight::Normal));
        let stamp = |(t, c): (String, Color)| txt(t, 11.0).color(c);
        let contents = 20.0 + 8.0 + WAVE_W + 8.0 + text_width(&total, 11.0, Weight::Normal);
        // Desktop's `transcribed-message`: the transcript in quotes, in the bubble text color, at most `QUOTE_W` wide.
        // Over `QUOTE_LINES` lines, it is clamped and "Show more" shows all of it. The time floats after the text,
        // else it goes beside "Show more". It is `stale` at half opacity while a new transcription is in flight.
        let stale = self.audio.transcribing.get(&m.id).is_some_and(|s| !s.1);
        let quote_fg = if stale { fg.scale_alpha(0.5) } else { fg };
        let (quote, inner) = match a.transcription.as_ref().filter(|t| !t.transcription.is_empty()) {
            None => (None, contents + ts.as_ref().map_or(0.0, |_| 8.0 + ts_w)),
            Some(t) => {
                let q = format!("\"{}\"", t.transcription);
                let (all, shown) = (
                    clamped(&q, QUOTE_W, usize::MAX, 13.0, Weight::Normal),
                    clamped(&q, QUOTE_W, QUOTE_LINES, 13.0, Weight::Normal),
                );
                let (over, open) = (all != shown, self.audio.more.contains(&m.id));
                let float = ts.clone().filter(|_| !over || open);
                let nbsp = text_width("\u{a0}", 13.0, Weight::Normal);
                let pad =
                    float.as_ref().map_or(String::new(), |_| "\u{a0}".repeat(((ts_w + 4.0) / nbsp).ceil() as usize));
                let body = format!("{}{pad}", if over && !open { &shown } else { &all });
                let w = text_width(&body, 13.0, Weight::Normal).min(QUOTE_W);
                let body = txt(body, 13.0).color(quote_fg).wrapping(Wrapping::WordOrGlyph);
                let body = stack![body].push(float.map(|t| container(stamp(t)).align_right(Fill).align_bottom(Fill)));
                let footer = over.then(|| {
                    let label = semi(if open { tr("Show less") } else { tr("Show more") }, 13.0);
                    let toggle = button(label.line_height(Pixels(rem(16.0))).color(self.pal.accent))
                        .padding(0)
                        .style(|_, _| button::Style::default())
                        .on_press(UiMessage::Voice(Msg::More(m.id.clone())));
                    row![toggle, Space::new().width(Fill)].push(ts.clone().filter(|_| !open).map(stamp))
                });
                (Some(column![body].push(footer)), contents.max(w))
            }
        };
        let ts = ts.filter(|_| quote.is_none()).map(stamp);
        let wave_w = (WAVE_W + inner - contents - ts.as_ref().map_or(0.0, |_| 8.0 + ts_w)
            + text_width(&total, 11.0, Weight::Normal)
            - text_width(&time, 11.0, Weight::Normal)
            - rate_w)
            .max(0.0);
        let toggle = clip.map(|_| UiMessage::Voice(Msg::Toggle(m.chat_id.clone(), m.id.clone(), url.clone())));
        let (wave, progress) = (if mine { fg.scale_alpha(0.5) } else { self.pal.on_secondary }, fg);
        let wave: El<'a> = match clip {
            Some(c) => waveform([&m.chat_id, &m.id, &url], c, pos.or(stop).unwrap_or(0.0), wave_w, (wave, progress)),
            None => Space::new().width(wave_w).height(20).into(),
        };
        let playing = track.is_some_and(|t| t.since.is_some());
        let play = button(tinted(&ICONS[usize::from(playing)], 20.0, fg)).bare(fg).on_press_maybe(toggle);
        let speed = ready.map(|_| {
            button(semi(rate, 11.0).color(Color::WHITE))
                .padding([1, 4])
                .style(btn_style([fg.scale_alpha(0.2); 3], [Color::WHITE; 3], 2.0))
                .on_press(UiMessage::Voice(Msg::Menu))
        });
        let line = row![play, wave, txt(time, 11.0).color(fg)].push(speed).push(ts);
        container(column![line.spacing(8).align_y(Center)].push(quote).spacing(6))
            .width(22.0 + inner)
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

    /// The "Playback Speed" row of the message menu of a playable audio message, and its "Transcription Language"
    /// row when the account can transcribe.
    pub(crate) fn voice_menu_rows(&self, st: &BeeperStore, m: &Message) -> Vec<(El<'_>, f32)> {
        let Some(a) = m.attachments.first().filter(|a| a.attachment_type == "audio") else { return Vec::new() };
        let row = |label: &'static str, open: Msg| -> (El<'_>, f32) {
            let open = UiMessage::Voice(open);
            let row = menu_item(&self.pal, None, tr(label).into(), "›", Some(open.clone()));
            (mouse_area(row).on_enter(open).into(), menu_pitch())
        };
        let speed = self.playable(a).then(|| row("Playback Speed", Msg::Sub));
        let language = st.feature("voice-transcription").on.then(|| row("Transcription Language", Msg::LangSub));
        speed.into_iter().chain(language).collect()
    }

    /// The open submenu of the message menu of `m` and its width: the Playback Speed menu `top` below the top of the
    /// menu, or the Transcription Language menu below it, at most `room` tall.
    pub(crate) fn view_voice_sub(&self, st: &BeeperStore, m: &Message, top: f32, room: f32) -> Option<(El<'_>, f32)> {
        if self.audio.sub {
            return Some((column![Space::new().height(top), self.view_rate_menu()].into(), rate_menu_w()));
        }
        if !self.audio.lang {
            return None;
        }
        let a = &m.attachments[0];
        let top = top + if self.playable(a) { menu_pitch() } else { 0.0 };
        // Desktop's `getTranscriptionLanguage`: the language of the transcription, else the default language.
        let pref = beeper_core::prefs::TRANSCRIPTION_CODES[st.settings.pref("transcriptionLanguage") as usize];
        let current = a.transcription.as_ref().and_then(|t| t.language.as_deref()).unwrap_or(pref);
        let p = &self.pal;
        let item = |&(code, name): &(&'static str, &'static str)| {
            let lead = Some(container((code == current).then(|| ic(icon::CHECK, 14.0))).width(22).into());
            let pick = UiMessage::Voice(Msg::Language(m.id.clone(), code.into()));
            menu_item(p, lead, tr(name).into(), "", Some(pick))
        };
        let langs = &beeper_core::prefs::TRANSCRIPTION_LANGUAGES;
        let items = std::iter::once(item(&langs[0])).chain([crate::menu_rule(p)]).chain(langs[1..].iter().map(item));
        let names = langs.iter().map(|(_, n)| text_width(tr(n), 13.0, Weight::Normal));
        let w = 10.0 + 16.0 + 22.0 + names.fold(0.0, f32::max);
        let h = (8.0 + 11.0 + (langs.len() as f32) * menu_pitch()).min((room - top).max(4.0 * menu_pitch()));
        let list = iced::widget::scrollable(Column::with_children(items).padding([4, 0])).height(h);
        let menu = container(list).width(w).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        Some((column![Space::new().height(top), menu].into(), w))
    }

    /// `GlobalAudioControls`, `w` wide, with the parts that Desktop's container queries hide at that width.
    pub(crate) fn view_global_audio<'a>(&self, st: &BeeperStore, w: f32) -> Option<El<'a>> {
        let t = self.audio.now.as_ref().filter(|_| !self.audio.closed)?;
        let clip = self.audio.clip(&t.url)?;
        let (p, pos) = (&self.pal, self.audio.pos(t));
        let icon_btn = |i: usize, size: f32, color: Color, msg: Option<UiMessage>| -> El<'a> {
            let b = button(tinted(&ICONS[i], size, color)).bare(color);
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
        let elapsed = elapsed.map(|e| txt(e, 11.0).color(p.strong));
        let close = icon_btn(4, 10.0, p.strong, Some(UiMessage::Voice(Msg::Close)));
        let time = row![].push(elapsed).push(close).spacing(8).align_y(Center).padding(Padding::ZERO.left(4));
        let chat = st.resolve_chat_idx(&t.chat).map(|i| &st.chats[i]).filter(|_| w > 163.0);
        let avatar = chat.map(|c| {
            let b = button(self.chat_avatar(c, 28.0, 0.0)).bare(p.strong);
            b.on_press(UiMessage::SelectChat(c.id.clone()))
        });
        let wave_w = w - 16.0 - avatar.as_ref().map_or(0.0, |_| 28.0) - if w > 101.0 { 60.0 } else { 20.0 } - time_w;
        let wave = (w > 249.0)
            .then(|| waveform([&t.chat, &t.mid, &t.url], clip, pos, wave_w.max(35.0), (p.on_secondary, p.strong)));
        let gap = || Space::new().width(Fill);
        let line = row![].push(avatar).push(gap()).push(buttons).push(gap()).push(wave).push(gap()).push(time);
        let pill = container(line.align_y(Center)).width(w).padding([6, 8]).style(fill_box(p.card, 24.0));
        let pill = mouse_area(pill).on_press(UiMessage::Voice(Msg::Grab));
        Some(pill.on_right_press(UiMessage::Voice(Msg::PillMenu)).into())
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
    // A view-once voice note plays only in its viewer.
    if m.view_once() {
        return None;
    }
    m.attachments.iter().find(|a| a.attachment_type == "audio")?.src_url.as_deref()
}

/// The extensions that Whisper reads (`wke`), and the extensions that Desktop changes to MP3 first (`nre`).
const WHISPER_EXTS: [&str; 10] = [".mp3", ".mp4", ".mpeg", ".mpga", ".m4a", ".wav", ".webm", ".ogg", ".oga", ".opus"];
const TO_MP3: [&str; 4] = [".caf", ".aac", ".mp4", ".m4a"];
/// Whisper's largest file (`Ske`).
const WHISPER_MAX: usize = 25 * 1024 * 1024;

/// Desktop's `getAudioExtension` (`_ke`) without its MIME database: the MIME subtype, else the file extension.
fn audio_ext(a: &Attachment) -> String {
    let known = |e: &str| WHISPER_EXTS.contains(&e) || TO_MP3.contains(&e);
    let mime = a.mime_type.as_deref().and_then(|m| m.split(';').next()?.split_once('/')).map(|(_, t)| format!(".{t}"));
    let name = a.file_name.as_deref().and_then(|n| n.rfind('.').map(|i| n[i..].to_lowercase()));
    match (mime, name) {
        (Some(m), _) if known(&m) => m,
        (_, Some(n)) if known(&n) => n,
        (m, n) => m.or(n).unwrap_or_default(),
    }
}

/// Desktop's `Mke.transcribe` after the download: MP3 for the formats that Whisper does not read, and the toast of
/// the files that Whisper rejects (`Sre`).
fn whisper_audio(bytes: Vec<u8>, ext: &str) -> Result<Vec<u8>, String> {
    let (bytes, ext) = match TO_MP3.contains(&ext) {
        true => (crate::run_ffmpeg(&bytes, &["-f", "mp3", "pipe:1"])?, ".mp3"),
        false => (bytes, ext),
    };
    let rejected = |why: String| format!("{why} for OpenAI Whisper, and this audio can't be transcribed another way.");
    if !ext.is_empty() && !WHISPER_EXTS.contains(&ext) {
        return Err(rejected(format!("Unsupported file extension ({ext})")));
    }
    if bytes.len() > WHISPER_MAX {
        return Err(rejected(format!("File size too large ({})", crate::human_size(bytes.len() as u64))));
    }
    Ok(bytes)
}

/// Desktop's `ai` and `spinner` icons.
static TRANSCRIBE_ICONS: LazyLock<[svg::Handle; 2]> = LazyLock::new(|| {
    [
        ("0 0 12 15", "M6.855 14.08a.35.35 0 0 1-.251-.1.46.46 0 0 1-.135-.257 17 17 0 0 0-.3-1.582q-.157-.639-.41-1.05a1.95 1.95 0 0 0-.655-.655q-.405-.247-1.038-.399a15 15 0 0 0-1.552-.275.42.42 0 0 1-.27-.13.38.38 0 0 1-.1-.263q0-.152.1-.264a.42.42 0 0 1 .27-.128 15 15 0 0 0 1.558-.24q.639-.147 1.05-.4a2 2 0 0 0 .661-.667q.252-.415.404-1.06.159-.65.282-1.594a.4.4 0 0 1 .129-.252q.111-.1.257-.1.153 0 .258.1a.43.43 0 0 1 .135.252q.135.943.293 1.588t.404 1.06q.252.416.662.668.411.246 1.038.393.631.14 1.558.252a.42.42 0 0 1 .264.134q.1.106.1.258a.37.37 0 0 1-.106.264.4.4 0 0 1-.258.129q-.925.105-1.564.252-.633.14-1.043.386a1.9 1.9 0 0 0-.662.668q-.252.417-.405 1.06-.152.651-.28 1.595a.46.46 0 0 1-.136.257.35.35 0 0 1-.258.1M2.742 7.57q-.21 0-.234-.228a8 8 0 0 0-.153-.903q-.075-.333-.246-.498-.17-.17-.51-.252a10 10 0 0 0-.925-.181q-.24-.036-.24-.24 0-.205.21-.24.604-.112.944-.2.345-.093.516-.252.175-.164.251-.486.083-.323.153-.902.023-.23.234-.229.217 0 .24.223.077.591.153.925.081.334.252.504.17.171.51.252.339.083.943.164a.24.24 0 0 1 .152.077.22.22 0 0 1 .065.164q0 .198-.211.24-.61.11-.95.205-.339.087-.51.252-.17.164-.251.492-.076.322-.153.897a.26.26 0 0 1-.076.158.24.24 0 0 1-.164.058M5.59 3.434q-.135 0-.152-.141-.064-.357-.112-.557a.63.63 0 0 0-.152-.31.66.66 0 0 0-.31-.164 5 5 0 0 0-.58-.13q-.147-.023-.147-.152 0-.128.14-.152.375-.076.586-.135a.66.66 0 0 0 .31-.164.6.6 0 0 0 .153-.304q.047-.2.112-.551.017-.147.152-.147.13 0 .152.141.059.357.106.562A.7.7 0 0 0 6 1.541q.106.105.31.164.211.052.592.123.141.024.141.152 0 .129-.14.153-.375.07-.587.135a.66.66 0 0 0-.31.164.7.7 0 0 0-.152.31 6 6 0 0 0-.112.551q-.023.14-.152.14"),
        ("0 0 24 24", "M2.629 13.569a.792.792 0 1 0 1.562-.26A7.917 7.917 0 1 1 12 19.917a.792.792 0 1 0-.001 1.583 9.5 9.5 0 1 0-9.371-7.931"),
    ]
    .map(|(view, d)| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});
/// `.spin-icon` turns once in 1.2 s.
const SPIN: Duration = Duration::from_millis(1200);

impl BeeperGuiApp {
    /// Desktop's `transcribeAttachment`, after its privacy notice (`oL`): the Whisper transcript of the first
    /// attachment of the message `mid`, in `language` or the default language. Desktop's `fetchLatestRelation` of an
    /// earlier transcription is not done: its endpoint is not known.
    fn transcribe(&mut self, mid: String, again: bool, language: Option<String>, quiet: bool) -> Task<UiMessage> {
        let st = self.store.read();
        if !st.settings.on("transcribeWithOpenai") {
            drop(st);
            (self.audio.asked, self.confirm) = (Some((mid, again, language)), Some(crate::ASK_TRANSCRIBE));
            return Task::none();
        }
        let a = crate::stored(&st, &mid).attachments.swap_remove(0);
        assert!(again || a.transcription.is_none(), "the message has no transcription yet");
        let default = beeper_core::prefs::TRANSCRIPTION_CODES[st.settings.pref("transcriptionLanguage") as usize];
        let language = language.unwrap_or_else(|| default.into());
        let code = beeper_core::prefs::transcription_language(&language).map(|_| language.clone());
        let (client, s) = (st.matrix.clone(), &st.beeper_state);
        let (base, token) = (beeper_core::cloud::ai_services(&s.homeserver), s.access_token.clone());
        drop(st);
        self.audio.transcribing.insert(mid.clone(), (language.clone(), false));
        let url = a.src_url.clone().expect("an audio attachment has a URL");
        let run = async move {
            let toast = move |t: String| (!quiet).then_some(t);
            let failed = |_| toast(tr("Failed to transcribe audio message").into());
            let bytes = beeper_core::matrix::media(client, url).await.map_err(failed)?;
            let ext = audio_ext(&a);
            let mp3 = tokio::task::spawn_blocking(move || whisper_audio(bytes, &ext));
            let bytes = mp3.await.expect("the MP3 task").map_err(toast)?;
            beeper_core::cloud::transcribe(&base, &token, (bytes, beeper_core::cloud::VOICE_NOTE_FILE), code.as_deref())
                .await
                .map_err(failed)
        };
        Task::perform(run, move |r| UiMessage::Voice(Msg::Transcribed(mid, language, r)))
    }

    /// Desktop's `autoTranscribeVoiceNote`: quiet transcriptions of the shown voice notes that asked for one.
    pub(crate) fn auto_transcribe(&mut self) -> Task<UiMessage> {
        let asked = std::mem::take(&mut *self.wants.transcribe.borrow_mut());
        let st = self.store.read();
        // A shown message can get a transcription or leave the store before this frame.
        let open = |mid: &String| st.loaded(mid).is_some_and(|m| m.attachments[0].transcription.is_none());
        let asked: Vec<_> =
            asked.into_iter().filter(|mid| !self.audio.transcribing.contains_key(mid) && open(mid)).collect();
        drop(st);
        Task::batch(asked.into_iter().map(|mid| self.transcribe(mid, false, None, true)).collect::<Vec<_>>())
    }

    /// The frames of the spinners while a transcription is in flight.
    pub(crate) fn transcribe_ticks(&self) -> Option<Subscription<UiMessage>> {
        let busy = self.audio.transcribing.values().any(|(_, failed)| !failed);
        busy.then(|| iced::time::every(Duration::from_millis(50)).map(|_| UiMessage::Voice(Msg::Spin)))
    }

    /// Desktop's `Transcribe` under the audio message `m`: the button, the spinner, the error, and the free uses.
    pub(crate) fn view_transcribe<'a>(&self, st: &BeeperStore, m: &Message) -> Option<El<'a>> {
        let a = m.attachments.first().filter(|a| a.attachment_type == "audio")?;
        let p = &self.pal;
        let state = self.audio.transcribing.get(&m.id);
        let (done, sent) = (a.transcription.is_some(), !crate::pending(st, m));
        let can = st.feature("voice-transcription").on && sent && !done && state.is_none_or(|s| s.1);
        // Desktop's `autoTranscribeVoiceNote` runs when a voice note shows.
        let s = &st.settings;
        if can
            && state.is_none()
            && a.is_voice_note == Some(true)
            && s.on("autoTranscribe")
            && s.on("transcribeWithOpenai")
        {
            self.wants.ask(&self.wants.transcribe, m.id.as_str());
        }
        let free = st.features.free_uses("voice-transcription");
        let used_up = sent && !can && free.is_some_and(|(used, limit)| used >= limit);
        let small = |s: String, c: Color| txt(s, 11.0).color(c);
        let count = free.filter(|&(used, limit)| used > 0 && limit > used).map(|(used, limit)| {
            let n = (limit - used) as usize;
            small(crate::trn("%d free transcription left", "%d free transcriptions left", n, &[&n]), p.sub)
        });
        let ai = || tinted(&TRANSCRIBE_ICONS[0], 16.0, p.accent);
        let line: El<'a> = match state {
            Some((language, false)) => {
                let turn = self.started.elapsed().as_secs_f32() / SPIN.as_secs_f32() * std::f32::consts::TAU;
                let spinner = svg(TRANSCRIBE_ICONS[1].clone()).width(10).height(10);
                let spinner = spinner.rotation(iced::Rotation::Solid(iced::Radians(turn)));
                let label = match beeper_core::prefs::transcription_language(language) {
                    Some(name) => crate::trf("Transcribing in %s…", &[&name]),
                    None => tr("Transcribing…").into(),
                };
                let sub = p.sub;
                let spinner = spinner.style(move |_, _| svg::Style { color: Some(sub) });
                row![spinner, small(label, p.sub)].spacing(2).align_y(Center).into()
            }
            _ if !done && !can && !used_up => return None,
            Some((language, true)) => {
                let again = Msg::Transcribe(m.id.clone(), done, Some(language.clone()));
                let again = button(semi(tr("Try Again"), 11.0).color(p.accent))
                    .padding(0)
                    .style(|_, _| button::Style::default())
                    .on_press(UiMessage::Voice(again));
                row![small(tr("Error transcribing audio").into(), p.sub), again].spacing(4).into()
            }
            _ if done => row![ai()].push(count?).spacing(4).align_y(Center).into(),
            _ => {
                let fresh = free.is_some_and(|(used, limit)| used == 0 && limit > 0);
                let label = small(tr(if fresh { "Try Transcribe" } else { "Transcribe" }).into(), p.accent);
                let press = match used_up {
                    true => UiMessage::Settings(settings::Msg::Open(crate::SettingsTab::BeeperPlus)),
                    false => UiMessage::Voice(Msg::Transcribe(m.id.clone(), false, None)),
                };
                let count = count.filter(|_| free.is_some_and(|(used, limit)| used < limit));
                button(row![ai(), label].push(count).spacing(4).align_y(Center))
                    .padding(0)
                    .style(|_, _| button::Style::default())
                    .on_press(press)
                    .into()
            }
        };
        Some(container(line).height(16).align_y(Center).padding([0.0, crate::BUBBLE_PAD_X]).into())
    }
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

    /// Desktop's transcript: in quotes, with room for the time after it, and a click on it does nothing. Over 40
    /// lines it is clamped, and "Show more" shows all of it.
    #[test]
    fn transcripts_like_desktop() {
        use beeper_core::models::Transcription;
        use iced_test::selector::{Candidate, Target};
        let quote = |c: Candidate<'_>| {
            Some(Target::from(c)).filter(|t| matches!(t, Target::Text { content, .. } if content.starts_with('"')))
        };
        let content = |t: Target| match t {
            Target::Text { content, .. } => content,
            _ => unreachable!("the selector finds texts"),
        };
        let mut app = crate::fuzz::demo_app("voice-transcript");
        let _ = app.update(UiMessage::SelectChat("!wa_eric:beeper.com".into()));
        let mut sim = iced_test::simulator(app.view_main());
        let text = content(sim.click(quote).expect("the transcript shows in quotes"));
        let want =
            "\"Native Rust + Iced GPU rendering makes Beeper feel instantaneous across Linux, macOS, and Windows.\"";
        assert!(text.strip_prefix(want).is_some_and(|pad| pad.chars().all(|c| c == '\u{a0}') && !pad.is_empty()));
        assert!(sim.into_messages().all(|m| !matches!(m, UiMessage::Voice(_))), "the click does nothing");

        let long =
            Transcription { transcription: ["word"; 600].join(" "), engine: "openai-whisper".into(), language: None };
        app.store.write().set_transcription("msg_1000c", long);
        let mut sim = iced_test::simulator(app.view_main());
        assert!(content(sim.find(quote).unwrap()).ends_with('…'), "the transcript is clamped");
        let _ = sim.click("Show more").expect("a long transcript has Show more");
        for m in sim.into_messages() {
            let _ = app.update(m);
        }
        let mut sim = iced_test::simulator(app.view_main());
        assert!(content(sim.find(quote).unwrap()).trim_end_matches('\u{a0}').ends_with("word\""), "all of it shows");
        let _ = sim.click("Show less").expect("an open transcript has Show less");
    }

    /// Desktop's `Transcribe` row: the privacy notice, the spinner, the error with "Try Again", the transcript, and
    /// the paywall when no free transcriptions are left.
    #[test]
    fn transcribe_like_desktop() {
        let mut app = crate::fuzz::demo_app("voice-transcribe");
        let id = "msg_1000c".to_string();
        let untranscribed = |app: &mut BeeperGuiApp| {
            let mut st = app.store.write();
            let i = st.find_message("msg_1000c").unwrap();
            st.update_message(i, |m| m.attachments[0].transcription = None);
        };
        app.store.write().features = beeper_core::features::Flags::free_plan();
        untranscribed(&mut app);
        let _ = app.update(UiMessage::SelectChat("!wa_eric:beeper.com".into()));
        let click = |app: &mut BeeperGuiApp, label: &str| -> Vec<UiMessage> {
            let mut sim = iced_test::simulator(app.view_main());
            let _ = sim.click(label).unwrap_or_else(|e| panic!("{label}: {e:?}"));
            sim.into_messages().collect()
        };
        let has = |app: &BeeperGuiApp, s: &str| iced_test::simulator(app.view_main()).find(s).is_ok();
        for m in click(&mut app, "Try Transcribe") {
            let _ = app.update(m);
        }
        assert_eq!(app.confirm, Some(crate::ASK_TRANSCRIBE), "the privacy notice comes first");
        let _ = app.update(UiMessage::Voice(Msg::Allow(false)));
        assert!(!app.store.read().settings.on("transcribeWithOpenai") && app.audio.transcribing.is_empty());
        let _ = app.update(UiMessage::Voice(Msg::Transcribe(id.clone(), false, Some("de".into()))));
        let _ = app.update(UiMessage::Voice(Msg::Allow(true)));
        assert!(app.store.read().settings.on("transcribeWithOpenai"));
        assert!(has(&app, "Transcribing in German…") && app.transcribe_ticks().is_some());
        let failed = Err(Some("Failed to transcribe audio message".to_string()));
        let _ = app.update(UiMessage::Voice(Msg::Transcribed(id.clone(), "de".into(), failed)));
        assert!(has(&app, "Error transcribing audio") && app.status_banner == "Failed to transcribe audio message");
        let again = click(&mut app, "Try Again");
        assert!(matches!(&again[..], [UiMessage::Voice(Msg::Transcribe(m, false, Some(l)))] if *m == id && l == "de"));
        let _ = app.update(UiMessage::Voice(Msg::Transcribed(id.clone(), "de".into(), Ok("Hallo".into()))));
        let mut st = app.store.write();
        let i = st.find_message(&id).unwrap();
        let want = Transcription {
            transcription: "Hallo".into(),
            engine: "openai-whisper".into(),
            language: Some("de".into()),
        };
        assert_eq!(st.messages[i].attachments[0].transcription, Some(want));
        drop(st);
        assert!(!has(&app, "Error transcribing audio") && !has(&app, "Transcribe"));
        app.store.write().features.usage = serde_json::json!({ "voice-transcription": { "used": 5 } });
        untranscribed(&mut app);
        let paywall = click(&mut app, "Transcribe");
        assert!(
            matches!(&paywall[..], [UiMessage::Settings(settings::Msg::Open(crate::SettingsTab::BeeperPlus))]),
            "no free uses are left"
        );
    }

    /// Desktop's "Transcription Language" submenu transcribes again in the picked language, and its automatic
    /// transcription starts a quiet transcription of a shown voice note in the default language.
    #[test]
    fn transcription_language_and_auto_transcribe_like_desktop() {
        let mut app = crate::fuzz::demo_app("voice-language");
        let id = "msg_1000c".to_string();
        app.store.write().features = beeper_core::features::Flags::free_plan();
        let _ = app.update(UiMessage::SelectChat("!wa_eric:beeper.com".into()));
        let _ = app.update(UiMessage::Message(crate::message::Msg::OpenMenu(id.clone())));
        let _ = app.update(UiMessage::Voice(Msg::LangSub));
        let mut sim = iced_test::simulator(app.view_main());
        assert!(sim.find("Transcription Language").is_ok() && sim.find("Auto-detect").is_ok());
        let _ = sim.click("German").expect("the languages of Whisper");
        let picked: Vec<_> = sim.into_messages().collect();
        assert!(matches!(&picked[..], [UiMessage::Voice(Msg::Language(m, l))] if *m == id && l == "de"));
        for m in picked {
            let _ = app.update(m);
        }
        assert!(app.pane.message_menu.is_none() && app.confirm == Some(crate::ASK_TRANSCRIBE));
        assert_eq!(app.audio.asked, Some((id.clone(), true, Some("de".into()))));
        let _ = app.update(UiMessage::Ask(None));
        {
            let mut st = app.store.write();
            let i = st.find_message(&id).unwrap();
            st.update_message(i, |m| m.attachments[0].transcription = None);
            st.settings.set_pref("transcribeWithOpenai", 1);
            st.settings.set_pref("transcriptionLanguage", 14);
        }
        assert!(app.auto_transcribe_asked(&id).is_none(), "auto-transcribe is off");
        app.store.write().settings.set_pref("autoTranscribe", 1);
        assert_eq!(app.auto_transcribe_asked(&id), Some(("en".to_string(), false)));
    }

    impl BeeperGuiApp {
        /// Draws the open chat, starts the automatic transcriptions, and gives the state of the transcription of `mid`.
        fn auto_transcribe_asked(&mut self, mid: &str) -> Option<(String, bool)> {
            let _ = iced_test::simulator(self.view_main());
            let _ = self.auto_transcribe();
            self.audio.transcribing.get(mid).cloned()
        }
    }

    /// Desktop's `getAudioExtension` and the files that Whisper rejects.
    #[test]
    fn whisper_files_like_desktop() {
        let ext = |mime: Option<&str>, name: Option<&str>| {
            let a = Attachment {
                mime_type: mime.map(str::to_string),
                file_name: name.map(str::to_string),
                ..Default::default()
            };
            audio_ext(&a)
        };
        assert_eq!(ext(Some("audio/ogg; codecs=opus"), Some("a.bin")), ".ogg");
        assert_eq!(ext(Some("audio/x-m4a"), Some("Note.M4A")), ".m4a");
        assert_eq!(ext(Some("audio/amr"), None), ".amr");
        assert_eq!(ext(None, None), "");
        assert_eq!(whisper_audio(b"OggS".to_vec(), ".ogg"), Ok(b"OggS".to_vec()));
        let toast =
            "Unsupported file extension (.amr) for OpenAI Whisper, and this audio can't be transcribed another way.";
        assert_eq!(whisper_audio(Vec::new(), ".amr"), Err(toast.into()));
        let big = whisper_audio(vec![0; WHISPER_MAX + 1], "").unwrap_err();
        assert!(
            big.starts_with("File size too large (")
                && big.ends_with("for OpenAI Whisper, and this audio can't be transcribed another way.")
        );
    }

    /// A demo app with two decoded 10 ms voice notes, `v0` and `v1`, in the first chat.
    fn two_notes(name: &str) -> (BeeperGuiApp, String) {
        let cfg = beeper_core::testing::demo_config(&format!("voice-{name}"));
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

    /// Desktop's `lastPosition`: a voice note plays again where it stopped, until a track of another chat plays.
    #[test]
    fn voice_notes_resume_like_desktop() {
        let (mut app, chat) = two_notes("resume");
        let play = |app: &mut BeeperGuiApp, mid: &str, url: &str, chat: &str| {
            let _ = app.voice(Msg::Toggle(chat.into(), mid.into(), url.into()));
            app.audio.now.as_ref().unwrap().at
        };
        let _ = app.voice(Msg::Seek(chat.clone(), "v0".into(), "mxc://a".into(), 0.5));
        assert_eq!(play(&mut app, "v1", "mxc://b", &chat), 0.0);
        assert!((app.audio.stops["v0"] - 0.005).abs() < 1e-6, "the first note keeps its position");
        assert!((play(&mut app, "v0", "mxc://a", &chat) - 0.005).abs() < 1e-6, "it plays again from there");
        let _ = play(&mut app, "v1", "mxc://b", "!other:beeper.com");
        assert!(!app.audio.stops.contains_key("v0"), "another chat forgets the positions");
    }

    /// Desktop's drag of the global player: it starts after 5 px, moves the player up and down, and saves the place.
    #[test]
    fn global_player_drags_like_desktop() {
        let (mut app, _) = two_notes("pill-drag");
        assert_eq!(app.audio.bottom, 8.0, "Desktop's default place");
        app.cursor.set(Point::new(100.0, 500.0));
        let _ = app.voice(Msg::Grab);
        let _ = app.voice(Msg::Drag(497.0));
        assert_eq!(app.audio.bottom, 8.0, "a move under 5 px does not drag");
        let _ = app.voice(Msg::Drag(450.0));
        let _ = app.voice(Msg::Drop);
        assert_eq!(app.audio.bottom, 58.0);
        assert_eq!(Player::new(&app.store.read().config.data_dir).bottom, 58.0, "the place stays");
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

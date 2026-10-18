//! Desktop's audio player (`Audio.tsx`): Ogg Opus decoding, the waveform, playback speed, and autoplay of the next
//! voice note.
use crate::style::{btn_style, fill_box, hidden_slider, ic, icon, semi, shadow_box, text_width, txt, CLEAR};
use crate::{menu_item, sound, tinted, BeeperGuiApp, El, UiMessage, AVATAR_SLOTS};
use beeper_core::models::{Attachment, Message};
use iced::font::Weight;
use iced::widget::{button, column, container, row, slider, stack, svg, Column, Row, Space};
use iced::{Center, Color, Pixels, Point, Subscription, Task};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// `PlaybackControl`'s speeds.
const RATES: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
pub(crate) const HZ: usize = 48_000;
const PEAKS: usize = 1024;
/// `.waveform-container`'s min-width.
const WAVE_W: f32 = 165.0;

/// Desktop's `play-fill` and `pause-fill`.
pub(crate) static ICONS: LazyLock<[svg::Handle; 2]> = LazyLock::new(|| {
    ["M6.667 15.834V4.167L15.833 10z", "M11.667 15.834V4.167H15v11.667zm-6.667 0V4.167h3.333v11.667z"].map(|d| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});

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
}

impl Default for Player {
    fn default() -> Self {
        Self { clips: HashMap::new(), now: None, rate: 1.0, menu: None }
    }
}

#[derive(Clone)]
pub enum Msg {
    Loaded(String, Result<Clip, String>),
    Toggle(String, String, String),
    Seek(String, String, String, f32),
    SeekEnd,
    Menu,
    Rate(f32),
    Tick,
}

/// Decode an Ogg Opus stream, like the voice notes of every network, to mono.
pub fn decode(bytes: &[u8]) -> Result<Clip, String> {
    let mut ogg = ogg::PacketReader::new(std::io::Cursor::new(bytes));
    let mut next = || ogg.read_packet().map_err(|e| e.to_string());
    let head = next()?.ok_or("an empty Ogg stream")?.data;
    if !head.starts_with(b"OpusHead") {
        return Err("not an Ogg Opus stream".into());
    }
    let channels = usize::from(head[9]);
    let skip = usize::from(u16::from_le_bytes([head[10], head[11]]));
    next()?.ok_or("no OpusTags packet")?;
    let mut opus = opus_decoder::OpusDecoder::new(HZ as u32, channels).map_err(|e| format!("{e:?}"))?;
    let (mut pcm, mut samples, mut end) = (vec![0.0; 5760 * channels], Vec::new(), 0);
    while let Some(packet) = next()? {
        let n = opus.decode_float(&packet.data, &mut pcm, false).map_err(|e| format!("{e:?}"))?;
        samples.extend(pcm[..n * channels].chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
        end = packet.absgp_page();
    }
    // The last page's granule position ends the stream, which trims the padding of its last packet.
    samples.truncate(usize::try_from(end).expect("a granule position fits in memory"));
    samples.drain(..skip.min(samples.len()));
    if samples.is_empty() {
        return Err("no audio".into());
    }
    Ok(Clip::new(samples))
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
            Msg::Menu => audio.menu = Some(self.cursor.get()),
            Msg::Rate(rate) => {
                let playing = audio.now.as_ref().is_some_and(|t| t.since.is_some());
                audio.pause();
                (audio.rate, audio.menu) = (rate, None);
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
                let (chat, mid) = (t.chat.clone(), t.mid.clone());
                audio.now = Some(Track { at: 0.0, since: None, ..audio.now.take().expect("a current track") });
                let st = self.store.read().unwrap();
                if !st.settings.on("autoPlayVoiceNotes") {
                    return Task::none();
                }
                let queue = st.chat_msg_indices.get(&chat).into_iter().flatten().map(|&i| &st.messages[i]);
                let mut queue = queue.filter_map(|m| Some((&m.id, audio_url(m)?)));
                let next = queue.find(|(id, _)| **id == mid).and(queue.next());
                if let Some((id, url)) = next.filter(|(_, url)| self.audio.clip(url).is_some()) {
                    let msg = Msg::Toggle(chat, id.clone(), url.to_string());
                    drop(st);
                    return self.voice(msg);
                }
            }
        }
        Task::none()
    }

    /// Fetch and decode the audio of the open chat.
    pub(crate) fn load_audio(&mut self) -> Task<UiMessage> {
        let st = self.store.read().unwrap();
        let Some(client) = st.matrix.clone() else { return Task::none() };
        let chat = self.selected_chat_id.as_ref().and_then(|c| st.chat_msg_indices.get(c));
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
                    beeper_core::matrix::media(client, u).await.and_then(|b| decode(&b))
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
            Some(c) => {
                let heights = bars(&c.peaks, wave_w, BUBBLE_BARS);
                let done = pos.map_or(0.0, |p| p / secs) * heights.len() as f32;
                let bars = heights.iter().enumerate().fold(Row::new().spacing(2).align_y(Center), |r, (k, &h)| {
                    let color = if (k as f32 + 0.5) < done { progress } else { wave };
                    r.push(container(Space::new()).width(2).height(h).style(fill_box(color, 2.0)))
                });
                let (chat, mid) = (m.chat_id.clone(), m.id.clone());
                let seek = slider(0.0..=1.0, pos.map_or(0.0, |p| p / secs), move |v| {
                    UiMessage::Voice(Msg::Seek(chat.clone(), mid.clone(), url.clone(), v))
                })
                .step(0.001_f32)
                .height(20)
                .on_release(UiMessage::Voice(Msg::SeekEnd))
                .style(hidden_slider);
                stack![container(bars).center_y(20), seek].width(wave_w).into()
            }
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
        let w = (10.0 + 16.0 + 22.0 + text_width("0.25x", 13.0, Weight::Normal)).max(80.0);
        container(Column::with_children(items).padding([4, 0]))
            .width(w)
            .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
            .into()
    }
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

    #[test]
    fn plays_the_next_voice_note_when_one_ends() {
        let dir = std::env::temp_dir().join("BeeperRs_voice_autoplay");
        let mut cfg = beeper_core::config::CentralConfig::new(dir, "127.0.0.1".into(), 23390, None);
        cfg.demo = true;
        let mut st = beeper_core::store::BeeperStore::from_config(cfg);
        let (chat, account) = (st.chats[0].id.clone(), st.chats[0].account_id.clone());
        for (i, url) in ["mxc://a", "mxc://b"].iter().enumerate() {
            st.upsert_message(
                serde_json::from_value(serde_json::json!({
                    "id": format!("v{i}"), "accountID": account, "chatID": chat, "senderID": "s",
                    "sortKey": format!("{i}"), "timestamp": "2026-01-02T03:04:05Z",
                    "attachments": [{ "type": "audio", "srcURL": url }],
                }))
                .expect("message JSON"),
            );
        }
        let mut app = BeeperGuiApp::new(Arc::new(std::sync::RwLock::new(st)));
        let short = Clip { samples: vec![0.0; 480].into(), peaks: vec![1.0].into() };
        for url in ["mxc://a", "mxc://b"] {
            let _ = app.voice(Msg::Loaded(url.into(), Ok(short.clone())));
        }
        let ended = |app: &mut BeeperGuiApp| {
            let _ = app.voice(Msg::Toggle(chat.clone(), "v0".into(), "mxc://a".into()));
            std::thread::sleep(Duration::from_millis(20));
            let _ = app.voice(Msg::Tick);
            let t = app.audio.now.as_ref().unwrap();
            (t.mid.clone(), t.since.is_some())
        };
        assert_eq!(ended(&mut app), ("v1".into(), true));
        app.store.write().unwrap().settings.set_pref("autoPlayVoiceNotes", 0);
        assert_eq!(ended(&mut app), ("v0".into(), false));
    }
}

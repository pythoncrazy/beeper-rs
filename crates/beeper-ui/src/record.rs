//! Desktop's `AudioBar`: record a voice note from the microphone, pause to play it back, and send it as Ogg Opus.
use crate::style::{bordered_box, btn_style, fill_box, hex, hidden_slider, txt, CLEAR};
use crate::voice::{self, Clip, HZ, ICONS};
use crate::{tinted, BeeperGuiApp, El, UiMessage};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{mouse, renderer, Clipboard, Renderer as _, Shell};
use iced::keyboard::key::Named;
use iced::widget::{button, container, responsive, row, slider, stack, svg, text_editor, tooltip, Row, Space};
use iced::{
    window, Center, Color, Element, Event, Fill, Length, Padding, Rectangle, Renderer, Size, Subscription, Task,
};
use rodio::microphone::{available_inputs, MicrophoneBuilder};
use std::io::Write;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

/// The voice player's key for the paused take.
pub(crate) const TAKE: &str = "recording";
/// `createRealtimeWaveform` takes a level each 75 ms.
const STEP: usize = HZ * 75 / 1000;
/// WaveSurfer's bars in `AudioBar`: 3px wide with its default 1.5px gap, 24px tall and at least 6px.
const BARS: (f32, f32, f32, f32) = (3.0, 1.5, 24.0, 6.0);
/// `AudioBar`'s WaveSurfer `waveColor` and `progressColor`.
const WAVE: [Color; 2] = [hex(0x787d87), hex(0x007aff)];

/// Desktop's `record` icon.
static RECORD: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(&br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7" fill="none" stroke="currentColor" stroke-width="2"/><circle cx="10" cy="10" r="4" fill="currentColor"/></svg>"#[..])
});

/// What the capture thread shares: mono 48 kHz samples, and the levels of the live waveform.
#[derive(Default)]
struct Take {
    samples: Mutex<Vec<f32>>,
    levels: Mutex<Vec<f32>>,
    paused: AtomicBool,
    stop: AtomicBool,
}

pub struct Recorder(Arc<Take>);

/// Desktop's `TalkToType`: the take of the microphone, then "Typing your message..." while Whisper transcribes it.
pub struct Talk {
    take: Recorder,
    typing: bool,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.0.stop.store(true, Relaxed);
    }
}

#[derive(Clone)]
pub enum Msg {
    Start,
    Ended(Result<(), String>),
    Pause,
    Resume,
    Cancel,
    Key(Named),
    Tick,
    Encoded(String, Option<String>, Clip, Vec<f32>, Result<Vec<u8>, String>),
    /// Desktop's `SHOW_TRANSCRIBE_BAR`: Talk to Type.
    Talk,
    /// The transcript of Talk to Type, or its error.
    Typed(Result<String, String>),
}

/// Desktop's `enumerateDevices`: the system default and the other inputs, by name.
pub(crate) fn inputs() -> Result<(String, Vec<String>), String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let system =
        rodio::cpal::default_host().default_input_device().and_then(|d| Some(d.description().ok()?.name().to_string()));
    let mut names: Vec<String> = available_inputs().map_err(|e| e.to_string())?.iter().map(|i| i.to_string()).collect();
    names.retain(|n| Some(n) != system.as_ref());
    Ok((system.unwrap_or_default(), names))
}

/// Capture the named microphone into `take` until it stops, like `getUserMedia`. As in Desktop, an unplugged
/// microphone records from the system default.
fn capture(take: &Take, name: &str) -> Result<(), String> {
    let b = MicrophoneBuilder::new();
    let chosen =
        available_inputs().map_err(|e| e.to_string())?.into_iter().find(|i| !name.is_empty() && i.to_string() == name);
    if chosen.is_none() && !name.is_empty() {
        eprintln!("Selected audio input {name:?} unavailable, falling back to system default");
    }
    let mic = chosen.map_or_else(|| b.default_device(), |i| b.device(i)).and_then(|b| b.default_config());
    let mic = mic.map_err(|e| e.to_string())?.open_stream().map_err(|e| e.to_string())?;
    let one = NonZero::new(1).expect("one channel");
    let hz = NonZero::new(HZ as u32).expect("a sample rate");
    let mut chunk = Vec::with_capacity(STEP);
    for s in rodio::source::UniformSourceIterator::new(mic, one, hz) {
        if take.stop.load(Relaxed) {
            return Ok(());
        }
        if take.paused.load(Relaxed) {
            continue;
        }
        chunk.push(s);
        if chunk.len() == STEP {
            // The analyser's 128 time-domain values: RMS times 3, at most 1.
            let tail = &chunk[STEP - 128..];
            let rms = (tail.iter().map(|x| x * x).sum::<f32>() / 128.0).sqrt();
            take.levels.lock().unwrap().push((rms * 3.0).min(1.0));
            take.samples.lock().unwrap().append(&mut chunk);
        }
    }
    Err("the microphone stopped".into())
}

/// Encode mono 48 kHz samples as Ogg Opus at 128 kbps, like `AudioBar`'s `MediaRecorder`.
fn encode(samples: &[f32]) -> Result<Vec<u8>, String> {
    let args = "-v error -f f32le -ar 48000 -ac 1 -i pipe:0 -c:a libopus -b:a 128k -f ogg pipe:1";
    let mut child = std::process::Command::new("ffmpeg")
        .args(args.split(' '))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("a piped stdin");
    let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let writer = std::thread::spawn(move || stdin.write_all(&pcm));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    writer.join().expect("the ffmpeg writer does not panic").map_err(|e| e.to_string())?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// A WAV file of mono 16-bit samples at `HZ`, like the `audio/wav` of Desktop's Talk to Type recorder.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let n = |v: usize| u32::try_from(v).expect("a WAV under 4 GiB").to_le_bytes();
    let format = [1u16, 1].map(u16::to_le_bytes).concat();
    let rate = [n(HZ), n(HZ * 2)].concat();
    let block = [2u16, 16].map(u16::to_le_bytes).concat();
    [&b"RIFF"[..], &n(36 + data.len()), b"WAVEfmt ", &n(16), &format, &rate, &block, b"data", &n(data.len()), &data]
        .concat()
}

/// `toWaveform`: at most 100 of the live levels.
fn waveform(levels: &[f32]) -> Vec<f32> {
    let n = levels.len().min(100);
    (0..n).map(|k| levels[k * levels.len() / n]).collect()
}

/// `createRealtimeWaveform`'s canvas: the newest level at the right, 3px bars with 2px gaps, each at least 6px.
struct Live<'a>(&'a Take, Color);

impl<M> Widget<M, iced::Theme, Renderer> for Live<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(24.0))
    }

    fn layout(&mut self, _: &mut widget::Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, 24.0, Size::ZERO))
    }

    fn update(
        &mut self,
        _: &mut widget::Tree,
        event: &Event,
        _: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            shell.request_redraw_at(*now + Duration::from_millis(75));
        }
    }

    fn draw(
        &self,
        _: &widget::Tree,
        renderer: &mut Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let b = layout.bounds();
        let levels = self.0.levels.lock().unwrap();
        for (k, v) in levels.iter().rev().enumerate() {
            let x = b.width - (k + 1) as f32 * 5.0;
            if x < 0.0 {
                break;
            }
            let h = 6.0 + v * (b.height - 4.0 - 6.0);
            let bounds = Rectangle { x: b.x + x, y: b.y + (b.height - h) / 2.0, width: 3.0, height: h };
            let quad = renderer::Quad { bounds, border: iced::border::rounded(1.5), ..Default::default() };
            renderer.fill_quad(quad, self.1);
        }
    }
}

impl BeeperGuiApp {
    pub(crate) fn record(&mut self, msg: Msg) -> Task<UiMessage> {
        if matches!(msg, Msg::Start) && !self.supports(crate::Picker::Audio) {
            return Task::none();
        }
        // Desktop's Talk to Type keys: space and enter stop the take, and escape cancels it.
        if let (Msg::Key(k), Some(t)) = (&msg, &self.talk) {
            return match k {
                Named::Escape => {
                    self.talk = None;
                    Task::none()
                }
                _ if t.typing => Task::none(),
                _ => self.talk_done(),
            };
        }
        let paused = || self.recorder.as_ref().is_some_and(|r| r.0.paused.load(Relaxed));
        match msg {
            Msg::Talk => {
                let st = self.store.read();
                if !st.feature("talk-to-type").on {
                    drop(st);
                    return self.handle(UiMessage::OpenSettings(crate::SettingsTab::BeeperPlus));
                }
                // Desktop's privacy notice (`oL`) asks first.
                if !st.settings.on("transcribeWithOpenai") {
                    drop(st);
                    self.confirm = Some(crate::ASK_TRANSCRIBE);
                    return Task::none();
                }
                let name = st.settings.microphone.clone();
                drop(st);
                let take = Arc::new(Take::default());
                self.talk = Some(Talk { take: Recorder(take.clone()), typing: false });
                let run = async move {
                    tokio::task::spawn_blocking(move || capture(&take, &name)).await.expect("capture runs")
                };
                return Task::perform(run, |r| UiMessage::Record(Msg::Ended(r)));
            }
            Msg::Typed(Ok(text)) => {
                self.talk = None;
                let end = text_editor::Action::Move(text_editor::Motion::DocumentEnd);
                let paste = text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(text)));
                let edits = [end, paste].map(|a| self.handle(UiMessage::ComposerEdit(a)));
                return Task::batch(edits.into_iter().chain([iced::widget::operation::focus("composer-input")]));
            }
            Msg::Typed(Err(e)) => {
                self.talk = None;
                self.status_banner = crate::trf("Something went wrong transcribing the audio recording: %s", &[&e]);
            }
            Msg::Start => {
                let take = Arc::new(Take::default());
                self.recorder = Some(Recorder(take.clone()));
                let name = self.store.read().settings.microphone.clone();
                let run = async move {
                    tokio::task::spawn_blocking(move || capture(&take, &name)).await.expect("capture runs")
                };
                return Task::perform(run, |r| UiMessage::Record(Msg::Ended(r)));
            }
            Msg::Ended(Ok(())) => {}
            Msg::Ended(Err(e)) => {
                (self.recorder, self.talk) = (None, None);
                self.status_banner = format!("Could not record audio: {e}");
            }
            Msg::Pause => {
                let take = &self.recorder.as_ref().expect("a recording pauses").0;
                let samples = take.samples.lock().unwrap().clone();
                if samples.is_empty() {
                    return Task::none();
                }
                take.paused.store(true, Relaxed);
                return self.voice(voice::Msg::Loaded(TAKE.into(), Ok(Clip::new(samples))));
            }
            Msg::Resume => {
                self.audio.forget(TAKE);
                self.recorder.as_ref().expect("a paused recording resumes").0.paused.store(false, Relaxed);
            }
            Msg::Cancel => {
                self.audio.forget(TAKE);
                self.recorder = None;
            }
            Msg::Key(Named::Escape) => return self.record(Msg::Cancel),
            Msg::Key(Named::Space) => return self.record(if paused() { Msg::Resume } else { Msg::Pause }),
            Msg::Key(Named::Enter) if paused() => return self.send_take(),
            Msg::Key(Named::Enter) => return self.record(Msg::Pause),
            Msg::Key(k) => unreachable!("the recorder does not take {k:?}"),
            Msg::Tick => {}
            Msg::Encoded(chat, reply_to, clip, wave, Ok(bytes)) => {
                let ms = (clip.secs() * 1000.0) as u64;
                let mut st = self.store.write();
                let key = format!("recorded:{}", st.next_id("voice"));
                let att = beeper_core::models::Attachment {
                    attachment_type: "audio".into(),
                    duration: Some(f64::from(clip.secs())),
                    file_name: Some("voice-note.ogg".into()),
                    file_size: Some(bytes.len() as u64),
                    is_voice_note: Some(true),
                    mime_type: Some("audio/ogg".into()),
                    src_url: Some(key.clone()),
                    ..Default::default()
                };
                let (txn, reply_to, thread) = self.pane.compose(&mut st, &chat, None, reply_to, Some(att));
                if thread.is_none() {
                    st.mark_chat_read(&chat, true);
                }
                let info = beeper_core::matrix::MediaInfo::Voice { ms, wave };
                let (name, mime) = ("voice-note.ogg".into(), "audio/ogg".into());
                let out = beeper_core::matrix::Outgoing::Media {
                    txn,
                    bytes,
                    name,
                    mime,
                    info,
                    caption: None,
                    reply_to,
                    thread,
                    gif: false,
                };
                st.queue(&chat, out);
                drop(st);
                self.refresh_theme_and_tray();
                return self.voice(voice::Msg::Loaded(key, Ok(clip)));
            }
            Msg::Encoded(.., Err(e)) => self.status_banner = format!("Could not send the voice note: {e}"),
        }
        Task::none()
    }

    /// Desktop's Talk to Type after the take: the Whisper transcript of the WAV take.
    fn talk_done(&mut self) -> Task<UiMessage> {
        let t = self.talk.as_mut().expect("Talk to Type records");
        t.typing = true;
        t.take.0.stop.store(true, Relaxed);
        let samples = std::mem::take(&mut *t.take.0.samples.lock().unwrap());
        let st = self.store.read();
        let (base, token) =
            (beeper_core::cloud::ai_services(&st.beeper_state.homeserver), st.beeper_state.access_token.clone());
        drop(st);
        let file = beeper_core::cloud::TALK_FILE;
        let run = async move { beeper_core::cloud::transcribe(&base, &token, (wav(&samples), file), None).await };
        Task::perform(run, |r| UiMessage::Record(Msg::Typed(r)))
    }

    /// `prepareAudioForSending`: stop the recording, encode it, and send it to the open chat.
    pub(crate) fn send_take(&mut self) -> Task<UiMessage> {
        let r = self.recorder.take().expect("a recording sends");
        let take = &r.0;
        self.audio.forget(TAKE);
        let samples = std::mem::take(&mut *take.samples.lock().unwrap());
        if samples.is_empty() {
            return Task::none();
        }
        let wave = waveform(&take.levels.lock().unwrap());
        let chat = self.pane.selected_chat_id.clone().expect("a chat is selected");
        let reply_to = self.pane.reply_to_message_id.take();
        let clip = Clip::new(samples.clone());
        let run = async move { tokio::task::spawn_blocking(move || encode(&samples)).await.expect("encoding runs") };
        Task::perform(run, move |r| UiMessage::Record(Msg::Encoded(chat, reply_to, clip, wave, r)))
    }

    pub(crate) fn record_events(&self) -> Option<Subscription<UiMessage>> {
        use iced::keyboard::{Event::KeyPressed, Key};
        let space = iced::event::listen_with(|e, _, _| match e {
            Event::Keyboard(KeyPressed { key: Key::Named(Named::Space), .. }) => {
                Some(UiMessage::Record(Msg::Key(Named::Space)))
            }
            _ => None,
        });
        if self.talk.is_some() {
            return Some(space);
        }
        let paused = self.recorder.as_ref()?.0.paused.load(Relaxed);
        let tick = (!paused).then(|| iced::time::every(Duration::from_secs(1)).map(|_| UiMessage::Record(Msg::Tick)));
        Some(Subscription::batch([space].into_iter().chain(tick)))
    }

    /// `AudioBar`: the cancel button and the recorder, in place of the composer input.
    pub(crate) fn view_recorder<'a>(&'a self, r: &'a Recorder) -> El<'a> {
        let p = &self.pal;
        let [ring, rest, fg, wave] = p.audio_bar;
        let paused = r.0.paused.load(Relaxed);
        let icon_btn = |h: &svg::Handle, msg| {
            button(tinted(h, 20.0, fg)).padding(0).style(btn_style([CLEAR; 3], [fg; 3], 0.0)).on_press(msg)
        };
        let cancel = button(tinted(&crate::ROW_ACTIONS[6], 20.0, p.on_secondary))
            .padding(8)
            .style(btn_style([CLEAR; 3], [p.on_secondary; 3], 18.0))
            .on_press(UiMessage::Record(Msg::Cancel));
        let take = self.audio.clip(TAKE).filter(|_| paused);
        let state = self.audio.state(TAKE);
        let secs = r.0.samples.lock().unwrap().len() as f32 / HZ as f32;
        let (time, middle, lead): (String, El<'a>, Option<El<'a>>) = match take {
            Some(c) => {
                let (pos, playing) = state.unwrap_or((0.0, false));
                let toggle = voice::Msg::Toggle(String::new(), TAKE.into(), TAKE.into());
                let play = icon_btn(&ICONS[usize::from(playing)], UiMessage::Voice(toggle));
                let (peaks, frac) = (c.peaks.clone(), pos / c.secs());
                let bars = responsive(move |size| {
                    let heights = voice::bars(&peaks, size.width, BARS);
                    let done = frac * heights.len() as f32;
                    let row = Row::new().spacing(BARS.1).align_y(Center);
                    let row = heights.iter().enumerate().fold(row, |r, (k, &h)| {
                        let color = WAVE[usize::from((k as f32 + 0.5) < done)];
                        r.push(container(Space::new()).width(BARS.0).height(h).style(fill_box(color, 1.5)))
                    });
                    container(row).center_y(24).into()
                });
                let seek = slider(0.0..=1.0, frac, |v| {
                    UiMessage::Voice(voice::Msg::Seek(String::new(), TAKE.into(), TAKE.into(), v))
                })
                .step(0.001_f32)
                .height(24)
                .on_release(UiMessage::Voice(voice::Msg::SeekEnd))
                .style(hidden_slider);
                let wave = stack![bars, seek].width(Fill).height(24);
                (format!("{}/{}", voice::clock(pos), voice::clock(c.secs())), wave.into(), Some(play.into()))
            }
            None => {
                let dot = container(Space::new()).width(6).height(6).style(fill_box(hex(0xdc362e), 3.0));
                (voice::clock(secs), Element::new(Live(&r.0, wave)), Some(dot.into()))
            }
        };
        let (label, act, msg) = match paused {
            true => ("Resume Recording", &*RECORD, Msg::Resume),
            false => ("Pause Recording", &ICONS[1], Msg::Pause),
        };
        let action = self.tip(icon_btn(act, UiMessage::Record(msg)).into(), label, vec![], tooltip::Position::Top);
        let middle = container(middle).padding([0, 6]).width(Fill);
        let bar = Row::new().push(lead).push(middle).push(txt(time, 12.0).color(fg)).push(action);
        let edge = if paused { rest } else { ring };
        let bar = container(bar.spacing(8).align_y(Center))
            .padding(Padding::new(0.0).left(14.0).right(14.0))
            .width(Fill)
            .center_y(36)
            .style(bordered_box(CLEAR, edge, 2.0, 24.0));
        row![cancel, bar].align_y(Center).width(Fill).into()
    }
}

impl BeeperGuiApp {
    /// Desktop's `TalkToType` in place of the composer input: the cancel button, and "Talk then press space" with the
    /// live waveform in a yellow ring, or "Typing your message...".
    pub(crate) fn view_talk<'a>(&'a self, t: &'a Talk) -> El<'a> {
        let p = &self.pal;
        let cancel = button(tinted(&ICONS[4], 20.0, p.on_secondary))
            .padding(8)
            .style(btn_style([CLEAR; 3], [p.on_secondary; 3], 18.0))
            .on_press(UiMessage::Record(Msg::Key(Named::Escape)));
        let cancel = self.tip(cancel.into(), crate::tr("Cancel Talk to Type"), vec![], tooltip::Position::Top);
        let dot = container(Space::new()).width(8).height(8).style(fill_box(YELLOW, 4.0));
        let line = match t.typing {
            true => row![dot, txt(crate::tr("Typing your message..."), 13.0).color(p.sub)],
            false => row![dot, txt(crate::tr("Talk then press space"), 13.0).color(p.sub)]
                .push(container(Element::new(Live(&t.take.0, p.on_secondary))).padding(Padding::ZERO.left(8.0))),
        };
        let wrapper = container(line.spacing(8).align_y(Center))
            .padding([0, 14])
            .width(Fill)
            .center_y(36)
            .style(bordered_box(CLEAR, YELLOW, 2.0, 24.0));
        let wrapper = iced::widget::mouse_area(wrapper);
        let wrapper = if t.typing { wrapper } else { wrapper.on_press(UiMessage::Record(Msg::Key(Named::Space))) };
        row![cancel, wrapper].align_y(Center).width(Fill).into()
    }
}

/// `--color-solids-yellow`, the color of Talk to Type.
const YELLOW: Color = hex(0xebaa00);

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's Talk to Type: the privacy notice, "Talk then press space", "Typing your message...", and the
    /// transcript at the end of the draft. A take is a mono 16-bit WAV.
    #[test]
    fn talk_to_type_like_desktop() {
        let w = wav(&[0.0, 1.0]);
        assert!(w.len() == 48 && w.starts_with(b"RIFF") && &w[8..16] == b"WAVEfmt " && w[44..] == [0, 0, 0xff, 0x7f]);
        let mut app = crate::fuzz::demo_app("talk");
        let mut plus = beeper_core::features::Flags::free_plan();
        app.store.write().features = plus.clone();
        let _ = app.update(UiMessage::SelectChat("!wa_eric:beeper.com".into()));
        let has = |app: &BeeperGuiApp, s: &str| iced_test::simulator(app.view_main()).find(s).is_ok();
        let _ = app.update(UiMessage::Record(Msg::Talk));
        assert!(app.talk.is_none() && app.confirm.is_none(), "Talk to Type has no free uses");
        plus.flags["features"]["*"]["voice-transcription"] = serde_json::json!(true);
        app.store.write().features = plus;
        let _ = app.update(UiMessage::Record(Msg::Talk));
        assert!(app.confirm == Some(crate::ASK_TRANSCRIBE) && app.talk.is_none(), "the privacy notice comes first");
        let notice = "Talk to Type works by sharing audio clips with OpenAI via our servers. You can always change this in Settings.";
        assert!(has(&app, notice));
        let _ = app.update(UiMessage::Voice(voice::Msg::Allow(true)));
        assert!(app.talk.is_some() && has(&app, "Talk then press space"));
        let _ = app.update(UiMessage::Record(Msg::Key(Named::Space)));
        assert!(has(&app, "Typing your message..."));
        let _ = app.update(UiMessage::Record(Msg::Typed(Ok("hello there".into()))));
        assert!(app.talk.is_none() && app.pane.composer.text().trim_end().ends_with("hello there"));
        let _ = app.update(UiMessage::Record(Msg::Talk));
        let _ = app.update(UiMessage::Record(Msg::Key(Named::Escape)));
        assert!(app.talk.is_none(), "escape cancels");
        let _ = app.update(UiMessage::Record(Msg::Talk));
        let _ = app.update(UiMessage::Record(Msg::Key(Named::Enter)));
        let _ = app.update(UiMessage::Record(Msg::Typed(Err("HTTP 500".into()))));
        assert_eq!(app.status_banner, "Something went wrong transcribing the audio recording: HTTP 500");
    }

    #[test]
    fn pauses_to_preview_and_cancels_like_desktop() {
        assert_eq!(waveform(&[0.5; 3]), [0.5; 3]);
        let levels: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        assert_eq!(waveform(&levels)[..3], [0.0, 10.0, 20.0]);
        let dir = std::env::temp_dir().join(format!("BeeperRs_record_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = beeper_core::config::CentralConfig::new(dir, "127.0.0.1".into(), 23390, None);
        cfg.demo = true;
        let st = beeper_core::store::BeeperStore::from_config(cfg);
        let mut app = BeeperGuiApp::new(beeper_core::store::StoreLock::shared(st));
        let _ = app.record(Msg::Start);
        let take = app.recorder.as_ref().expect("a recording").0.clone();
        let _ = app.record(Msg::Key(Named::Space));
        assert!(!take.paused.load(Relaxed), "an empty take does not pause");
        take.samples.lock().unwrap().extend([0.1; HZ]);
        let _ = app.record(Msg::Key(Named::Enter));
        assert!(take.paused.load(Relaxed) && app.audio.clip(TAKE).is_some_and(|c| c.secs() == 1.0));
        let _ = app.record(Msg::Key(Named::Space));
        assert!(!take.paused.load(Relaxed) && app.audio.clip(TAKE).is_none());
        let other = app.store.read().chats[1].id.clone();
        let _ = app.update(UiMessage::SelectChat(other));
        assert_eq!(app.status_banner, "Stop recording to switch chats");
        let _ = app.record(Msg::Key(Named::Escape));
        assert!(app.recorder.is_none() && take.stop.load(Relaxed));
    }
}

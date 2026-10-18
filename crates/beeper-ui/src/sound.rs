//! Desktop's message sound effects, and the voice note player's output.

use std::num::NonZero;
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Sound {
    Send,
    Receive,
}

enum Command {
    Effect(Sound, f32),
    /// Mono 48 kHz samples, played from the given sample at the given speed; `None` stops them.
    Voice(Option<(Arc<[f32]>, usize, f32)>),
}

const WAVS: [&[u8]; 2] = [include_bytes!("../assets/sound_send.wav"), include_bytes!("../assets/sound_receive.wav")];

/// Play `sound` at `volume` (0 to 1). Like Desktop, a sound plays at most once every 400 ms.
pub fn play(sound: Sound, volume: f32) {
    send(Command::Effect(sound, volume));
}

/// Replace the playing voice note with `samples[from..]` at `speed`, or stop it.
pub fn voice(clip: Option<(Arc<[f32]>, usize, f32)>) {
    send(Command::Voice(clip));
}

fn send(command: Command) {
    static PLAYER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
    let tx = PLAYER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::spawn(move || {
            let mut stream = match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(s) => s,
                Err(e) => return eprintln!("sound output failed: {e}"),
            };
            stream.log_on_drop(false);
            let mut last: [Option<Instant>; 2] = [None; 2];
            let mut _voice: Option<rodio::Player> = None;
            for command in rx {
                match command {
                    Command::Effect(sound, volume) => {
                        let i = sound as usize;
                        if last[i].is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
                            continue;
                        }
                        last[i] = Some(Instant::now());
                        let player =
                            rodio::play(stream.mixer(), std::io::Cursor::new(WAVS[i])).expect("decode a bundled sound");
                        player.set_volume(volume);
                        player.detach();
                    }
                    Command::Voice(clip) => {
                        // Dropping the previous player stops it.
                        _voice = clip.map(|(samples, from, speed)| {
                            let player = rodio::Player::connect_new(stream.mixer());
                            let (mono, hz) = (NonZero::new(1).unwrap(), NonZero::new(48_000).unwrap());
                            player.append(rodio::buffer::SamplesBuffer::new(mono, hz, samples[from..].to_vec()));
                            player.set_speed(speed);
                            player
                        });
                    }
                }
            }
        });
        tx
    });
    // The player thread ends when the system has no audio output.
    let _ = tx.send(command);
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_sounds_decode() {
        use rodio::Source;
        for (wav, ms) in super::WAVS.into_iter().zip([312, 375]) {
            let d = rodio::Decoder::new(std::io::Cursor::new(wav)).unwrap();
            assert_eq!(d.total_duration().unwrap().as_millis(), ms);
        }
    }
}

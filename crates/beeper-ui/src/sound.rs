//! Desktop's message sound effects.

use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Sound {
    Send,
    Receive,
}

const WAVS: [&[u8]; 2] = [include_bytes!("../assets/sound_send.wav"), include_bytes!("../assets/sound_receive.wav")];

/// Play `sound` at `volume` (0 to 1). Like Desktop, a sound plays at most once every 400 ms.
pub fn play(sound: Sound, volume: f32) {
    static PLAYER: OnceLock<mpsc::Sender<(Sound, f32)>> = OnceLock::new();
    let tx = PLAYER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(Sound, f32)>();
        std::thread::spawn(move || {
            let mut stream = match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(s) => s,
                Err(e) => return eprintln!("sound effects failed: {e}"),
            };
            stream.log_on_drop(false);
            let mut last: [Option<Instant>; 2] = [None; 2];
            for (sound, volume) in rx {
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
        });
        tx
    });
    // The player thread ends when the system has no audio output.
    let _ = tx.send((sound, volume));
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

//! Notification sounds and the ringtone: WAV files rendered by `scripts/sounds.py`, built into the binary and
//! played through the system's default output (PipeWire or PulseAudio via ALSA on Linux,
//! WASAPI on Windows).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The sound files, by the ids of [`mostik_core::NOTIFICATION_SOUNDS`]; `none` has none.
const FILES: &[(&str, &[u8])] = &[
    ("marimba", include_bytes!("../sounds/marimba.wav")),
    ("chime", include_bytes!("../sounds/chime.wav")),
    ("drop", include_bytes!("../sounds/drop.wav")),
    ("knock", include_bytes!("../sounds/knock.wav")),
    ("glass", include_bytes!("../sounds/glass.wav")),
    ("electric_piano", include_bytes!("../sounds/electric_piano.wav")),
    ("retro", include_bytes!("../sounds/retro.wav")),
];

/// The ring of an incoming call: one pass of a loop, rendered to join seamlessly end to start.
const RINGTONE: &[u8] = include_bytes!("../sounds/ringtone.wav");

/// Rings the ringtone round and round for as long as `keep` says so (asked every 50 ms, so the
/// ringing stops at once). Blocks: run it on a thread of its own.
pub fn ring_while(keep: impl Fn() -> bool) {
    if let Err(e) = ring_loop(&keep) {
        eprintln!("ringtone: {e}");
    }
}

fn ring_loop(keep: &dyn Fn() -> bool) -> Result<(), Box<dyn std::error::Error>> {
    use rodio::Source;

    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    let player = rodio::Player::connect_new(sink.mixer());
    player.append(rodio::Decoder::new(std::io::Cursor::new(RINGTONE))?.repeat_infinite());
    while keep() {
        std::thread::sleep(Duration::from_millis(50));
    }
    player.stop();
    Ok(())
}

/// A burst of messages gives one sound, not a pile of them.
const MIN_GAP: Duration = Duration::from_millis(700);

static LAST_PLAYED_MS: AtomicU64 = AtomicU64::new(0);

/// Plays sound `id` on a thread of its own; an unknown id or `none` is silence.
pub fn play(id: &str) {
    let Some(&(name, wav)) = FILES.iter().find(|(name, _)| *name == id) else {
        return;
    };
    let now = mostik_yandex::clock::now_ms();
    let last = LAST_PLAYED_MS.swap(now, Ordering::SeqCst);
    if now.saturating_sub(last) < MIN_GAP.as_millis() as u64 {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = play_blocking(wav) {
            eprintln!("sound {name}: {e}");
        }
    });
}

/// Opens the default output for the length of one sound: an output kept open would show the
/// app as playing all the time.
fn play_blocking(wav: &'static [u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    let player = rodio::play(sink.mixer(), std::io::Cursor::new(wav))?;
    player.sleep_until_end();
    // The device buffer still holds the end of the sound.
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}

#[cfg(test)]
mod tests {
    use mostik_core::NOTIFICATION_SOUNDS;

    use super::*;

    #[test]
    fn every_offered_sound_has_a_file() {
        for id in NOTIFICATION_SOUNDS.iter().filter(|id| **id != "none") {
            let wav = FILES.iter().find(|(name, _)| name == id).map(|(_, wav)| wav);
            assert!(wav.is_some_and(|w| w.starts_with(b"RIFF")), "no WAV for sound {id}");
        }
        assert_eq!(FILES.len() + 1, NOTIFICATION_SOUNDS.len());
        assert!(RINGTONE.starts_with(b"RIFF"));
    }
}

//! Notification sounds: WAV files rendered by `scripts/sounds.py`, built into the binary and
//! played through the system's default output (PipeWire or PulseAudio via ALSA on Linux,
//! WASAPI on Windows).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The sound files, by the ids of [`mostik_core::NOTIFICATION_SOUNDS`]; `none` has none.
const FILES: &[(&str, &[u8])] = &[
    ("chime", include_bytes!("../sounds/chime.wav")),
    ("bell", include_bytes!("../sounds/bell.wav")),
    ("marimba", include_bytes!("../sounds/marimba.wav")),
    ("drop", include_bytes!("../sounds/drop.wav")),
    ("pop", include_bytes!("../sounds/pop.wav")),
    ("knock", include_bytes!("../sounds/knock.wav")),
    ("glass", include_bytes!("../sounds/glass.wav")),
];

/// A burst of messages gives one sound, not a pile of them.
const MIN_GAP: Duration = Duration::from_millis(700);

static LAST_PLAYED_MS: AtomicU64 = AtomicU64::new(0);

/// Plays sound `id` on a thread of its own; an unknown id or `none` is silence.
pub fn play(id: &str) {
    let Some(&(name, wav)) = FILES.iter().find(|(name, _)| *name == id) else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
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
    }
}

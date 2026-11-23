//! Voice messages: the Ogg Opus file is decoded here and played through the system's default
//! output, as the notification sounds are (see `sound.rs`): WebKitGTK plays audio through
//! GStreamer, which the AppImage does not carry. One voice message plays at a time.

use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Opus always decodes at 48 kHz.
const RATE: u32 = 48_000;
/// The longest frame an Opus packet holds: 120 ms.
const MAX_FRAME: usize = 5760;
/// The longest voice message decoded: Telemost takes ten minutes; a longer file is cut.
const MAX_SECONDS: usize = 20 * 60;

/// A decoded voice message: interleaved samples at 48 kHz.
#[derive(Debug)]
pub struct Audio {
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn duration_ms(&self) -> u64 {
        (self.samples.len() / usize::from(self.channels.max(1))) as u64 * 1000 / u64::from(RATE)
    }
}

/// Decodes an Ogg Opus file (RFC 7845): the `OpusHead` packet, the `OpusTags` packet, then audio
/// packets; the first `pre-skip` samples of the decoder are not part of the sound.
pub fn decode(ogg: &[u8]) -> Result<Audio, String> {
    let mut reader = ogg::PacketReader::new(std::io::Cursor::new(ogg));
    let head = reader
        .read_packet()
        .map_err(|e| e.to_string())?
        .ok_or("the voice message is empty")?;
    let head = &head.data;
    if head.len() < 19 || !head.starts_with(b"OpusHead") {
        return Err("the voice message is not Ogg Opus".into());
    }
    let channels = head[9];
    // Mapping family 0 (mono or stereo) is all a voice message uses.
    if !(1..=2).contains(&channels) || head[18] != 0 {
        return Err(format!("unsupported Opus channels: {channels}, mapping {}", head[18]));
    }
    let channels = usize::from(channels);
    let pre_skip = usize::from(u16::from_le_bytes([head[10], head[11]])) * channels;
    let mut decoder = opus_rs::OpusDecoder::new(RATE as i32, channels).map_err(str::to_owned)?;
    let max = MAX_SECONDS * RATE as usize * channels;
    let mut samples = Vec::new();
    let mut frame = vec![0f32; MAX_FRAME * channels];
    let mut tags = true;
    while let Some(packet) = reader.read_packet().map_err(|e| e.to_string())? {
        // The comment header comes before the audio.
        if std::mem::take(&mut tags) && packet.data.starts_with(b"OpusTags") {
            continue;
        }
        let n = decoder
            .decode(&packet.data, MAX_FRAME, &mut frame)
            .map_err(|e| format!("Opus: {e}"))?;
        samples.extend_from_slice(&frame[..(n * channels).min(frame.len())]);
        if samples.len() >= max {
            samples.truncate(max);
            break;
        }
    }
    samples.drain(..pre_skip.min(samples.len()));
    Ok(Audio {
        channels: channels as u16,
        samples,
    })
}

/// The voice message playing now.
struct Playing {
    file_id: String,
    stop: Arc<AtomicBool>,
    /// Where it is, in milliseconds from its start.
    at_ms: Arc<AtomicU64>,
}

static PLAYING: Mutex<Option<Playing>> = Mutex::new(None);

/// Plays `audio` of voice message `file_id` from `from_ms` on a thread of its own, stopping the one
/// playing before. `ended` runs when it plays to its end or cannot play; not when [`stop`] stops it.
pub fn play(file_id: &str, audio: Arc<Audio>, from_ms: u64, ended: impl FnOnce() + Send + 'static) {
    stop();
    let stop = Arc::new(AtomicBool::new(false));
    let at_ms = Arc::new(AtomicU64::new(from_ms));
    if let Ok(mut now) = PLAYING.lock() {
        *now = Some(Playing {
            file_id: file_id.to_owned(),
            stop: stop.clone(),
            at_ms: at_ms.clone(),
        });
    }
    std::thread::spawn(move || {
        if let Err(e) = play_blocking(&audio, from_ms, &stop, &at_ms) {
            eprintln!("voice message: {e}");
        }
        // Done, unless it was stopped or another one took its place meanwhile.
        let mine = PLAYING
            .lock()
            .map(|mut now| {
                let mine = now.as_ref().is_some_and(|p| Arc::ptr_eq(&p.stop, &stop));
                if mine {
                    *now = None;
                }
                mine
            })
            .unwrap_or(false);
        if mine {
            ended();
        }
    });
}

/// Stops the voice message playing, if one is. Returns it and where it stopped, in milliseconds.
pub fn stop() -> Option<(String, u64)> {
    let playing = PLAYING.lock().ok()?.take()?;
    playing.stop.store(true, Ordering::SeqCst);
    Some((playing.file_id, playing.at_ms.load(Ordering::SeqCst)))
}

fn play_blocking(audio: &Audio, from_ms: u64, stop: &AtomicBool, at_ms: &AtomicU64) -> Result<(), String> {
    let channels = NonZero::new(audio.channels).ok_or("no channels")?;
    let rate = NonZero::new(RATE).ok_or("no rate")?;
    let frames_from = usize::try_from(from_ms * u64::from(RATE) / 1000).unwrap_or(usize::MAX);
    let start = frames_from
        .saturating_mul(usize::from(audio.channels))
        .min(audio.samples.len());
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
    sink.log_on_drop(false);
    let player = rodio::Player::connect_new(sink.mixer());
    player.append(rodio::buffer::SamplesBuffer::new(
        channels,
        rate,
        audio.samples[start..].to_vec(),
    ));
    while !player.empty() {
        if stop.load(Ordering::SeqCst) {
            player.stop();
            return Ok(());
        }
        at_ms.store(from_ms + player.get_pos().as_millis() as u64, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
    }
    // The device buffer still holds the end of the sound.
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz tone, encoded by ffmpeg's libopus at 16 kbit/s.
    const VOICE: &[u8] = include_bytes!("../testdata/voice.ogg");

    #[test]
    fn an_ogg_opus_file_decodes_to_its_sound() {
        let audio = decode(VOICE).unwrap();
        assert_eq!(audio.channels, 1);
        // A second, give or take the last frame.
        assert!((980..=1020).contains(&audio.duration_ms()), "{}", audio.duration_ms());
        // A tone, not silence: ffmpeg's sine is at 1/8 of full scale, so its loudness is about 0.088.
        let mid = &audio.samples[12_000..36_000];
        let rms = (mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32).sqrt();
        assert!((0.07..0.11).contains(&rms), "{rms}");
        // 440 Hz crosses zero about 880 times a second.
        let crossings = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((400..480).contains(&crossings), "{crossings}");
    }

    /// Plays through the default output: run it with ALSA sent to a null device, so nothing is
    /// heard: `ALSA_CONFIG_PATH=<file with "pcm.!default { type null }"> cargo test -- --ignored`.
    /// A null device takes the sound as fast as it comes, so nothing here depends on time.
    #[test]
    #[ignore]
    fn plays_to_the_end_and_stops_when_asked() {
        let (tx, rx) = std::sync::mpsc::channel();
        let sent = |id: &'static str| {
            let tx = tx.clone();
            move || tx.send(id).unwrap()
        };
        play("a", Arc::new(decode(VOICE).unwrap()), 0, sent("a"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("a"));
        assert!(stop().is_none());
        // Ten minutes of silence: stopped, it does not say it ended; it says where it stopped.
        let long = || {
            Arc::new(Audio {
                channels: 1,
                samples: vec![0.0; 600 * RATE as usize],
            })
        };
        play("b", long(), 200, sent("b"));
        let (id, at) = stop().unwrap();
        assert_eq!((id.as_str(), at >= 200), ("b", true));
        // Another one in its place stops it without its end.
        play("c", long(), 0, sent("c"));
        play("d", Arc::new(decode(VOICE).unwrap()), 0, sent("d"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("d"));
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn what_is_not_ogg_opus_is_refused() {
        assert!(decode(b"").is_err());
        assert!(decode(b"RIFF....WAVE").is_err());
        let mut broken = VOICE.to_vec();
        broken.truncate(200);
        assert!(decode(&broken).map_or(true, |a| a.samples.len() < 48_000));
    }
}

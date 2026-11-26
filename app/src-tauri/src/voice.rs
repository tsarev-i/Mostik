//! Voice messages: the Ogg Opus file is decoded here and played through the system's default
//! output, as the notification sounds are (see `sound.rs`): WebKitGTK plays audio through
//! GStreamer, which the AppImage does not carry. One voice message plays at a time.
//!
//! Any chat member can send a voice message, and the Opus decoder (`opus-rs`) is young Rust with
//! a lot of `unsafe` code, so it never runs in the app's own process, which holds the session:
//! [`decode_apart`] starts the app's binary again as a decoder ([`child_main`]), hands it the
//! file and takes back the sound (#64). A decoder that crashes or runs wild takes only itself
//! down. The sound is kept small (#63): mono 16-bit at 24 kHz, an eighth of the 48 kHz `f32`
//! stereo the decoder gives, cut at Telemost's ten minutes.

use std::io::{Read, Write};
use std::num::NonZero;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The rate voice messages are decoded and kept at: speech needs no more. One of the rates an
/// Opus decoder gives out.
pub const RATE: u32 = 24_000;
/// The longest frame an Opus packet holds, 120 ms, at [`RATE`].
const MAX_FRAME: usize = (RATE / 1000 * 120) as usize;
/// The longest voice message kept: Telemost records ten minutes at most; the rest of a longer
/// file is cut. Its sound then takes under 30 MB.
pub const MAX_MS: u64 = 10 * 60 * 1000 + 10_000;
/// The flag that starts the binary as a voice message decoder instead of the app.
const CHILD_FLAG: &str = "--decode-voice";
/// The largest file the decoder reads: as the download limit (`backend::voice`).
pub const FILE_LIMIT: u64 = 32 * 1024 * 1024;
/// How long the decoder may take: a ten-minute message decodes in a second or two.
const DECODE_TIMEOUT: Duration = Duration::from_secs(30);

/// A decoded voice message: mono samples at [`RATE`].
#[derive(Debug)]
pub struct Audio {
    pub samples: Vec<i16>,
}

impl Audio {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / u64::from(RATE)
    }
}

/// How many samples `ms` milliseconds take at [`RATE`].
fn samples_in(ms: u64) -> usize {
    usize::try_from(ms.saturating_mul(u64::from(RATE)) / 1000).unwrap_or(usize::MAX)
}

/// Decodes an Ogg Opus file (RFC 7845): the `OpusHead` packet, the `OpusTags` packet, then audio
/// packets; the first `pre-skip` samples of the decoder are not part of the sound. Stereo is
/// mixed down to mono; decoding stops at `max_ms` of sound. Runs in the decoder process only
/// ([`child_main`]), and in tests.
fn decode(ogg: &[u8], max_ms: u64) -> Result<Vec<i16>, String> {
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
    // Pre-skip counts samples at 48 kHz.
    let pre_skip = usize::from(u16::from_le_bytes([head[10], head[11]])) * RATE as usize / 48_000;
    let mut decoder = opus_rs::OpusDecoder::new(RATE as i32, channels).map_err(str::to_owned)?;
    let max = samples_in(max_ms).saturating_add(pre_skip);
    let mut samples: Vec<i16> = Vec::new();
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
        let decoded = &frame[..(n * channels).min(frame.len())];
        samples.extend(decoded.chunks(channels).map(|s| {
            let mono = s.iter().sum::<f32>() / channels as f32;
            (mono.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
        }));
        if samples.len() >= max {
            samples.truncate(max);
            break;
        }
    }
    samples.drain(..pre_skip.min(samples.len()));
    Ok(samples)
}

/// The decoder process: when the binary was started with [`CHILD_FLAG`] (see [`decode_apart`]),
/// reads an Ogg Opus file on stdin and writes its sound on stdout, little-endian `i16`. Returns
/// the exit code; `None` when the binary was started as the app.
pub fn child_main() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(CHILD_FLAG) {
        return None;
    }
    let max_ms = args.next().and_then(|a| a.parse().ok()).unwrap_or(MAX_MS).min(MAX_MS);
    limit_resources();
    let mut ogg = Vec::new();
    if let Err(e) = std::io::stdin().take(FILE_LIMIT).read_to_end(&mut ogg) {
        eprintln!("voice decoder: {e}");
        return Some(1);
    }
    let samples = match decode(&ogg, max_ms) {
        Ok(samples) => samples,
        Err(e) => {
            eprintln!("{e}");
            return Some(1);
        }
    };
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = std::io::stdout().lock();
    match out.write_all(&bytes).and_then(|()| out.flush()) {
        Ok(()) => Some(0),
        Err(e) => {
            eprintln!("voice decoder: {e}");
            Some(1)
        }
    }
}

/// Bounds what the decoder process may take, in case a crafted file sends it astray: CPU time,
/// and on Linux its memory. The parent also stops it after [`DECODE_TIMEOUT`].
fn limit_resources() {
    #[cfg(target_os = "linux")]
    {
        let limit = |resource, value: libc::rlim_t| {
            let rl = libc::rlimit {
                rlim_cur: value,
                rlim_max: value,
            };
            // SAFETY: setrlimit reads the struct it is given and changes only this process.
            unsafe { libc::setrlimit(resource, &rl) };
        };
        limit(libc::RLIMIT_CPU, DECODE_TIMEOUT.as_secs());
        // The decoder needs a few tens of MB; the binary's libraries are mapped, not counted.
        limit(libc::RLIMIT_DATA, 512 * 1024 * 1024);
    }
}

/// Decodes voice message `ogg` in a process of its own (see the module notes), keeping at most
/// `max_ms` of it. Blocks: run it off the async runtime.
pub fn decode_apart(ogg: Vec<u8>, max_ms: u64) -> Result<Audio, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    run_decoder(Command::new(exe), ogg, max_ms.min(MAX_MS), DECODE_TIMEOUT)
}

/// Runs decoder `command` (see [`child_main`]) on `ogg`: its sound, at most `max_ms` of it, or why
/// it failed; one that has not finished in `timeout` is killed.
fn run_decoder(mut command: Command, ogg: Vec<u8>, max_ms: u64, timeout: Duration) -> Result<Audio, String> {
    command
        .arg(CHILD_FLAG)
        .arg(max_ms.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a debug build is a console program, and would open a console.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("voice decoder: {e}"))?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("voice decoder: no pipes".into());
    };
    // Written on a thread of its own, so a decoder that writes before it has read all does not
    // wait on a full pipe while this one waits on it.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&ogg);
    });
    let cap = samples_in(max_ms) as u64 * 2;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.take(cap + 1).read_to_end(&mut bytes);
        let mut why = String::new();
        let _ = stderr.take(4096).read_to_string(&mut why);
        let _ = tx.send((read.map(|_| bytes), why));
    });
    let got = rx.recv_timeout(timeout);
    if got.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = writer.join();
    let (bytes, why) = got.map_err(|_| "voice decoder: took too long".to_owned())?;
    let bytes = bytes.map_err(|e| format!("voice decoder: {e}"))?;
    if !status.success() {
        let why = why.trim();
        return Err(match why.lines().last() {
            Some(line) if !line.is_empty() => line.to_owned(),
            _ => format!("voice decoder: {status}"),
        });
    }
    if bytes.len() as u64 > cap {
        return Err("voice decoder: longer than asked".into());
    }
    let samples = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(Audio { samples })
}

/// A decoded voice message from a sample on, for the output: it reads the kept samples where
/// they are, so a play, a pause or a seek does not copy them.
struct Samples {
    audio: Arc<Audio>,
    at: usize,
}

impl Iterator for Samples {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let s = *self.audio.samples.get(self.at)?;
        self.at += 1;
        Some(rodio::Sample::from(s) / rodio::Sample::from(i16::MAX))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.audio.samples.len().saturating_sub(self.at);
        (left, Some(left))
    }
}

impl rodio::Source for Samples {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.audio.samples.len().saturating_sub(self.at))
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::<u16>::MIN
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(RATE).unwrap_or(NonZero::<u32>::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        let left = self.audio.samples.len().saturating_sub(self.at) as u64;
        Some(Duration::from_micros(left * 1_000_000 / u64::from(RATE)))
    }
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
        if let Err(e) = play_blocking(audio, from_ms, &stop, &at_ms) {
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

fn play_blocking(audio: Arc<Audio>, from_ms: u64, stop: &AtomicBool, at_ms: &AtomicU64) -> Result<(), String> {
    let at = samples_in(from_ms).min(audio.samples.len());
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
    sink.log_on_drop(false);
    let player = rodio::Player::connect_new(sink.mixer());
    player.append(Samples { audio, at });
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

    fn audio(samples: Vec<i16>) -> Audio {
        Audio { samples }
    }

    #[test]
    fn an_ogg_opus_file_decodes_to_its_sound() {
        let audio = audio(decode(VOICE, MAX_MS).unwrap());
        // A second, give or take the last frame.
        assert!((980..=1020).contains(&audio.duration_ms()), "{}", audio.duration_ms());
        // A tone, not silence: ffmpeg's sine is at 1/8 of full scale, so its loudness is about 0.088.
        let mid = &audio.samples[6_000..18_000];
        let rms = (mid.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / mid.len() as f64).sqrt() / 32767.0;
        assert!((0.07..0.11).contains(&rms), "{rms}");
        // 440 Hz crosses zero about 880 times a second.
        let crossings = mid.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!((400..480).contains(&crossings), "{crossings}");
    }

    /// The sound stops at the length asked for: a file of tiny packets cannot unfold into
    /// hundreds of megabytes (#63).
    #[test]
    fn decoding_stops_at_the_length_asked() {
        let short = decode(VOICE, 300).unwrap();
        assert!(short.len() <= samples_in(300), "{}", short.len());
        assert!(short.len() >= samples_in(250), "{}", short.len());
    }

    /// Mutated files (bits flipped, bytes dropped or repeated, cut short) are refused or decoded
    /// within the length asked, never more: a cheap fuzz of the decoder (#64). It runs in the
    /// test process; in the app it runs apart, where a panic is only a failed decoding.
    #[test]
    fn mutated_files_stay_within_bounds() {
        let mut seed: u64 = 0x5eed;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let mut file = VOICE.to_vec();
            for _ in 0..1 + next() % 8 {
                let at = (next() as usize) % file.len();
                match next() % 4 {
                    0 => file[at] ^= 1 << (next() % 8),
                    1 => file[at] = next() as u8,
                    2 => {
                        file.remove(at);
                    }
                    _ => file.insert(at, file[at]),
                }
            }
            if next() % 5 == 0 {
                file.truncate((next() as usize) % file.len());
            }
            let decoded = std::panic::catch_unwind(|| decode(&file, 2000));
            if let Ok(Ok(samples)) = decoded {
                assert!(samples.len() <= samples_in(2000) + MAX_FRAME, "{}", samples.len());
            }
        }
    }

    /// The app's side of the decoder process, with a stand-in decoder: the sound it writes, the
    /// reason it gives, too long an answer, one that never comes.
    #[cfg(unix)]
    #[test]
    fn the_decoder_process_is_heard_out() {
        let fake = |script: &str| {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(script);
            c
        };
        let ms = Duration::from_millis;
        // Two samples, 1 and -2, after the whole file is read.
        let audio = run_decoder(
            fake(r"cat > /dev/null; printf '\001\000\376\377'"),
            VOICE.to_vec(),
            1000,
            ms(5000),
        )
        .unwrap();
        assert_eq!(audio.samples, [1, -2]);
        let refused = run_decoder(
            fake("cat > /dev/null; echo 'the voice message is not Ogg Opus' >&2; exit 1"),
            vec![1, 2],
            1000,
            ms(5000),
        );
        assert_eq!(refused.unwrap_err(), "the voice message is not Ogg Opus");
        // More than 10 ms of sound (240 samples, 480 bytes) when 10 ms were asked for.
        let long = run_decoder(fake("head -c 1000 /dev/zero"), vec![], 10, ms(5000));
        assert!(long.is_err());
        let started = std::time::Instant::now();
        let stuck = run_decoder(fake("sleep 30"), vec![], 1000, ms(300));
        assert_eq!(stuck.unwrap_err(), "voice decoder: took too long");
        assert!(started.elapsed() < Duration::from_secs(10));
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
        let voice = || Arc::new(audio(decode(VOICE, MAX_MS).unwrap()));
        play("a", voice(), 0, sent("a"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("a"));
        assert!(stop().is_none());
        // Ten minutes of silence: stopped, it does not say it ended; it says where it stopped.
        let long = || Arc::new(audio(vec![0; 600 * RATE as usize]));
        play("b", long(), 200, sent("b"));
        let (id, at) = stop().unwrap();
        assert_eq!((id.as_str(), at >= 200), ("b", true));
        // Another one in its place stops it without its end.
        play("c", long(), 0, sent("c"));
        play("d", voice(), 0, sent("d"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("d"));
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn what_is_not_ogg_opus_is_refused() {
        assert!(decode(b"", MAX_MS).is_err());
        assert!(decode(b"RIFF....WAVE", MAX_MS).is_err());
        let mut broken = VOICE.to_vec();
        broken.truncate(200);
        assert!(decode(&broken, MAX_MS).map_or(true, |a| a.len() < 24_000));
    }

    /// The sound source starts where asked and reads the kept samples as they are.
    #[test]
    fn the_source_reads_from_where_it_starts() {
        use rodio::Source;
        let kept = Arc::new(audio(vec![0, i16::MAX, i16::MIN + 1, 0]));
        let source = Samples {
            audio: kept.clone(),
            at: 1,
        };
        assert_eq!(source.channels().get(), 1);
        assert_eq!(source.sample_rate().get(), RATE);
        assert_eq!(source.collect::<Vec<_>>(), [1.0, -1.0, 0.0]);
        // Nothing was copied: the source shared the samples.
        assert_eq!(Arc::strong_count(&kept), 1);
    }
}

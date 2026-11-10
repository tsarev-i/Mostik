//! Notification sounds. Synthesised with Web Audio: our own, not Slack's sounds.

use std::cell::RefCell;

use web_sys::{AudioContext, OscillatorType};

/// Selectable sounds: identifiers for the preferences.
pub const SOUNDS: [&str; 4] = ["chime", "drop", "knock", "none"];

thread_local! {
    static CTX: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
}

/// One note: frequency, start and duration in seconds, waveform, volume.
struct Note(f32, f64, f64, OscillatorType, f32);

fn notes(sound: &str) -> Vec<Note> {
    use OscillatorType::{Sine, Triangle};
    match sound {
        "chime" => vec![Note(880.0, 0.0, 0.35, Sine, 0.25), Note(1318.5, 0.09, 0.45, Sine, 0.18)],
        "drop" => vec![Note(1200.0, 0.0, 0.12, Sine, 0.3), Note(600.0, 0.05, 0.18, Sine, 0.2)],
        "knock" => vec![
            Note(220.0, 0.0, 0.08, Triangle, 0.4),
            Note(196.0, 0.13, 0.08, Triangle, 0.35),
        ],
        _ => vec![],
    }
}

/// Plays sound `sound`. Web Audio errors are not critical: we just stay silent.
pub fn play(sound: &str) {
    let notes = notes(sound);
    if notes.is_empty() {
        return;
    }
    CTX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match AudioContext::new() {
                Ok(c) => *slot = Some(c),
                Err(e) => leptos::logging::warn!("sound: AudioContext unavailable: {e:?}"),
            }
        }
        let Some(ctx) = slot.as_ref() else { return };
        let _ = ctx.resume();
        let now = ctx.current_time();
        for Note(freq, start, dur, wave, volume) in notes {
            let (osc, gain) = match (ctx.create_oscillator(), ctx.create_gain()) {
                (Ok(o), Ok(g)) => (o, g),
                (o, g) => {
                    leptos::logging::warn!("sound: nodes not created: {:?} {:?}", o.err(), g.err());
                    continue;
                }
            };
            osc.set_type(wave);
            osc.frequency().set_value(freq);
            let g = gain.gain();
            let t0 = now + start;
            let _ = g.set_value_at_time(0.0001, t0);
            let _ = g.exponential_ramp_to_value_at_time(volume, t0 + 0.01);
            let _ = g.exponential_ramp_to_value_at_time(0.0001, t0 + dur);
            if osc.connect_with_audio_node(&gain).is_err() || gain.connect_with_audio_node(&ctx.destination()).is_err()
            {
                continue;
            }
            let _ = osc.start_with_when(t0);
            let _ = osc.stop_with_when(t0 + dur + 0.05);
        }
    });
}

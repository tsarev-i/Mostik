//! Voice messages: the player with its bars and the text Telemost made of the speech, as the
//! official client shows them. The sound plays in the backend (see `bridge::play_voice`), one
//! voice message at a time.

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::Voice;

use super::state::{Ctx, ctx};
use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::{bridge, ops};

/// Bars of the player, whatever the length of the message.
const BARS: usize = 40;
/// Characters of the text shown before "More", as in the official client.
const SHORT_TEXT: usize = 180;
/// The tallest bar, in pixels.
const BAR_MAX: f64 = 22.0;

/// The voice message playing or paused, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceNow {
    pub file_id: String,
    pub duration_ms: u64,
    pub state: VoiceState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VoiceState {
    /// Asked to play from `from_ms`: the sound is on its way. `request` tells this asking from
    /// an earlier one, whose answer may still come.
    Loading {
        from_ms: u64,
        request: u64,
    },
    /// Playing from `from_ms` since `since` (`Date.now()`, milliseconds).
    Playing {
        from_ms: u64,
        since: f64,
    },
    Paused {
        at_ms: u64,
    },
}

impl VoiceNow {
    /// Where it is at time `now` (`Date.now()`), in milliseconds.
    pub fn at_ms(&self, now: f64) -> u64 {
        let at = match self.state {
            VoiceState::Loading { from_ms, .. } => from_ms,
            VoiceState::Playing { from_ms, since } => from_ms + (now - since).max(0.0) as u64,
            VoiceState::Paused { at_ms } => at_ms,
        };
        at.min(self.duration_ms)
    }

    fn playing(&self) -> bool {
        matches!(self.state, VoiceState::Loading { .. } | VoiceState::Playing { .. })
    }
}

thread_local! {
    /// Counts the requests to play.
    static REQUESTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Plays voice message `file_id` from `from_ms`; in the browser demo only the player moves.
fn play(c: Ctx, file_id: String, duration_ms: u64, from_ms: u64) {
    let request = REQUESTS.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    let state = VoiceState::Loading { from_ms, request };
    c.voice.set(Some(VoiceNow {
        file_id: file_id.clone(),
        duration_ms,
        state,
    }));
    let id = file_id.clone();
    let started = move |c: Ctx, duration_ms: u64| {
        c.voice_clock.set(js_sys::Date::now());
        c.voice.set(Some(VoiceNow {
            file_id: file_id.clone(),
            duration_ms,
            state: VoiceState::Playing {
                from_ms,
                since: js_sys::Date::now(),
            },
        }));
    };
    if !c.live {
        return started(c, duration_ms);
    }
    spawn_local(async move {
        let result = bridge::play_voice(&id, from_ms).await;
        // A pause, another message or another request to play meanwhile wins.
        if c.voice
            .with_untracked(|v| v.as_ref().is_none_or(|v| v.file_id != id || v.state != state))
        {
            return;
        }
        match result {
            Ok(duration) => started(c, duration.max(1)),
            Err(e) => {
                c.voice.set(None);
                ops::failed(c, tr().err_voice, e);
            }
        }
    });
}

/// Pauses the voice message playing where it is.
fn pause(c: Ctx) {
    let Some(now) = c.voice.get_untracked() else {
        return;
    };
    let id = now.file_id.clone();
    c.voice.set(Some(VoiceNow {
        state: VoiceState::Paused {
            at_ms: now.at_ms(js_sys::Date::now()),
        },
        ..now
    }));
    if c.live {
        spawn_local(async move {
            // The backend knows better where the sound stopped.
            if let Ok(Some(at_ms)) = bridge::stop_voice().await {
                c.voice.update(|v| {
                    if let Some(v) = v
                        && v.file_id == id
                        && matches!(v.state, VoiceState::Paused { .. })
                    {
                        v.state = VoiceState::Paused {
                            at_ms: at_ms.min(v.duration_ms),
                        };
                    }
                });
            }
        });
    }
}

/// The voice message `file_id` played to its end: the player goes back to the start.
pub fn ended(c: Ctx, file_id: &str) {
    if c.voice
        .with_untracked(|v| v.as_ref().is_some_and(|v| v.file_id == file_id && v.playing()))
    {
        c.voice.set(None);
    }
}

/// Moves the player on while a voice message plays; one that has run past its end without the
/// backend saying so stops.
pub fn tick(c: Ctx) {
    let now = js_sys::Date::now();
    let Some(v) = c.voice.get_untracked().filter(VoiceNow::playing) else {
        return;
    };
    c.voice_clock.set(now);
    // The browser demo has no backend to tell the end.
    let done = match v.state {
        VoiceState::Playing { .. } if !c.live => v.at_ms(now) >= v.duration_ms,
        _ => overdue(&v, now),
    };
    if done {
        c.voice.set(None);
    }
}

/// Played well past its end: the end the backend tells of was lost.
fn overdue(v: &VoiceNow, now: f64) -> bool {
    match v.state {
        VoiceState::Playing { from_ms, since } => from_ms as f64 + (now - since) > v.duration_ms as f64 + 3000.0,
        _ => false,
    }
}

/// The loudness of each bar: the loudest of the levels it covers. No levels make a flat line.
fn bars(levels: &[u8], n: usize) -> Vec<u8> {
    if levels.is_empty() {
        return vec![0; n];
    }
    (0..n)
        .map(|i| {
            let from = i * levels.len() / n;
            let to = ((i + 1) * levels.len() / n).max(from + 1).min(levels.len());
            levels[from.min(levels.len() - 1)..to]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// `m:ss`.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The text cut at the last space before [`SHORT_TEXT`] characters, or `None` when it is shorter.
fn short_text(text: &str) -> Option<String> {
    let cut = text.char_indices().nth(SHORT_TEXT)?.0;
    let head = &text[..cut];
    let head = head.rfind(' ').map_or(head, |i| &head[..i]);
    Some(format!("{}…", head.trim_end()))
}

/// One line about a voice message in a list: 🎤 and its text, or that it is a voice message.
pub(super) fn voice_line(v: &Voice) -> String {
    let text = if v.recognized && !v.text.is_empty() {
        v.text.as_str()
    } else {
        tr().voice_message
    };
    format!("🎤 {text}")
}

/// A voice message in the feed: play and pause, the bars (a click on them plays from there), its
/// length, and the text Telemost made of the speech.
#[component]
pub(super) fn VoiceCard(voice: Voice) -> impl IntoView {
    let c = ctx();
    let Voice {
        file_id,
        duration_ms,
        waveform,
        text,
        recognized,
    } = voice;
    let levels = bars(&waveform, BARS);
    let mine = {
        let id = file_id.clone();
        Memo::new(move |_| c.voice.with(|v| v.as_ref().filter(|v| v.file_id == id).cloned()))
    };
    let length = move || mine.with(|m| m.as_ref().map_or(duration_ms, |m| m.duration_ms)).max(1);
    let at = move || {
        let now = c.voice_clock.get();
        mine.with(|m| m.as_ref().map_or(0, |m| m.at_ms(now)))
    };
    let playing = move || mine.with(|m| m.as_ref().is_some_and(VoiceNow::playing));
    let toggle = {
        let id = file_id.clone();
        move |_| match mine.get_untracked() {
            Some(m) if m.playing() => pause(c),
            Some(m) => play(
                c,
                id.clone(),
                m.duration_ms,
                m.at_ms(js_sys::Date::now()) % m.duration_ms.max(1),
            ),
            None => play(c, id.clone(), duration_ms, 0),
        }
    };
    let seek = {
        let id = file_id.clone();
        move |e: ev::MouseEvent| {
            let Some(el) = e
                .current_target()
                .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            else {
                return;
            };
            let rect = el.get_bounding_client_rect();
            if rect.width() <= 0.0 {
                return;
            }
            let share = ((f64::from(e.client_x()) - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let length = length();
            play(c, id.clone(), length, (share * length as f64) as u64);
        }
    };
    let played = move || (at() as usize * BARS) / length() as usize;
    let time = move || match mine.with(|m| m.is_some()) {
        true => clock(at()),
        false => clock(length()),
    };
    let transcript = (!text.is_empty()).then(|| {
        if !recognized {
            return view! { <div class="voice-text pending">{tr().voice_recognizing}</div> }.into_any();
        }
        match short_text(&text) {
            None => view! { <div class="voice-text">{text}</div> }.into_any(),
            Some(short) => {
                let more = RwSignal::new(false);
                view! {
                    <div class="voice-text">
                        {move || if more.get() { text.clone() } else { short.clone() }}
                        {move || (!more.get()).then(|| view! {
                            " "<button class="voice-more" on:click=move |_| more.set(true)>{tr().voice_more}</button>
                        })}
                    </div>
                }
                .into_any()
            }
        }
    });
    view! {
        <div class="voice">
            <div class="voice-player">
                <button class="voice-play" title=move || if playing() { tr().voice_pause } else { tr().voice_play } on:click=toggle>
                    {move || if playing() { view! { <Icon icon=I::Pause size=14/> } } else { view! { <Icon icon=I::Play size=14/> } }}
                </button>
                <div class="voice-wave" on:click=seek>
                    {levels.into_iter().enumerate().map(|(i, level)| {
                        let height = f64::from(level.max(2)) / 16.0 * BAR_MAX;
                        view! { <span class="voice-bar" class:played=move || i < played() style=format!("height:{height:.1}px")></span> }
                    }).collect_view()}
                </div>
                <span class="voice-time">{time}</span>
            </div>
            {transcript}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_take_the_loudest_level_they_cover() {
        assert_eq!(bars(&[1, 9, 3, 4], 2), [9, 4]);
        // Fewer levels than bars: each level covers some bars.
        assert_eq!(bars(&[5, 7], 4), [5, 5, 7, 7]);
        assert_eq!(bars(&[], 3), [0, 0, 0]);
        assert_eq!(bars(&[15; 1000], BARS).len(), BARS);
    }

    #[test]
    fn a_long_text_is_cut_at_a_space() {
        assert_eq!(short_text("short"), None);
        let long = "word ".repeat(50);
        let short = short_text(&long).unwrap();
        assert!(short.ends_with("word…"), "{short}");
        assert!(short.chars().count() <= SHORT_TEXT + 1);
    }

    #[test]
    fn where_the_player_is() {
        let v = |state| VoiceNow {
            file_id: "f".into(),
            duration_ms: 10_000,
            state,
        };
        assert_eq!(
            v(VoiceState::Loading {
                from_ms: 2000,
                request: 1
            })
            .at_ms(0.0),
            2000
        );
        let playing = v(VoiceState::Playing {
            from_ms: 1000,
            since: 5000.0,
        });
        assert_eq!(playing.at_ms(7500.0), 3500);
        assert_eq!(playing.at_ms(99_000.0), 10_000);
        assert!(overdue(&playing, 5000.0 + 12_001.0));
        assert!(!overdue(&playing, 5000.0 + 11_000.0));
        assert_eq!(v(VoiceState::Paused { at_ms: 4000 }).at_ms(1e12), 4000);
        assert_eq!(clock(65_400), "1:05");
    }
}

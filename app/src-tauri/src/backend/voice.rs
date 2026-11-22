//! Voice messages: downloaded, decoded and played here (see `crate::voice`), one at a time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mostik_ipc::VoiceEnded;
use mostik_ipc::cmd::{self, Args, Out, VoiceArgs};
use tauri::{AppHandle, Emitter};

use super::{AppState, live, web_error};
use crate::voice::{self, Audio};

/// A voice message played to its end, or that could not play: [`VoiceEnded`].
pub const VOICE_EVENT: &str = "mostik://voice";

/// The largest voice message file downloaded: ten minutes of Opus speech is a few megabytes.
const VOICE_LIMIT: u64 = 32 * 1024 * 1024;

/// How many decoded voice messages are kept, so a pause, a seek or a second listen does not
/// download them again.
const KEPT: usize = 4;

static KEPT_AUDIO: Mutex<Vec<(String, Arc<Audio>)>> = Mutex::new(Vec::new());

/// Counts the clicks: a voice message that took long to download does not start after the user
/// has paused it or played another one.
static CLICKS: AtomicU64 = AtomicU64::new(0);

fn kept(file_id: &str) -> Option<Arc<Audio>> {
    let mut all = KEPT_AUDIO.lock().ok()?;
    let i = all.iter().position(|(id, _)| id == file_id)?;
    // The latest used goes last, so the oldest goes first.
    let entry = all.remove(i);
    let audio = entry.1.clone();
    all.push(entry);
    Some(audio)
}

fn keep(file_id: &str, audio: Arc<Audio>) {
    if let Ok(mut all) = KEPT_AUDIO.lock() {
        all.retain(|(id, _)| id != file_id);
        all.push((file_id.to_owned(), audio));
        let over = all.len().saturating_sub(KEPT);
        all.drain(..over);
    }
}

/// Plays voice message `file_id` from `from_ms`, stopping the one playing. Returns its length in
/// milliseconds; when it ends by itself, [`VOICE_EVENT`] says so.
#[tauri::command]
pub async fn play_voice(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: Args<cmd::PlayVoice>,
) -> Result<Out<cmd::PlayVoice>, String> {
    let VoiceArgs { file_id, from_ms } = args;
    let click = CLICKS.fetch_add(1, Ordering::SeqCst) + 1;
    let audio = match kept(&file_id) {
        Some(audio) => audio,
        None => {
            let live = live(&state).await?;
            let (bytes, _) = live
                .web
                .fetch_file(&file_id, None, VOICE_LIMIT)
                .await
                .map_err(|e| web_error(&app, e))?;
            let audio = tauri::async_runtime::spawn_blocking(move || voice::decode(&bytes))
                .await
                .map_err(|e| e.to_string())??;
            let audio = Arc::new(audio);
            keep(&file_id, audio.clone());
            audio
        }
    };
    let duration = audio.duration_ms();
    if CLICKS.load(Ordering::SeqCst) == click {
        let ended = VoiceEnded {
            file_id: file_id.clone(),
        };
        voice::play(&file_id, audio, from_ms.min(duration), move || {
            let _ = app.emit(VOICE_EVENT, &ended);
        });
    }
    Ok(duration)
}

/// Stops the voice message playing. Returns where it stopped, in milliseconds.
#[tauri::command]
pub fn stop_voice() -> Out<cmd::StopVoice> {
    CLICKS.fetch_add(1, Ordering::SeqCst);
    voice::stop().map(|(_, at)| at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_voice_messages_are_kept() {
        let audio = || {
            Arc::new(Audio {
                channels: 1,
                samples: vec![],
            })
        };
        for i in 0..KEPT {
            keep(&format!("f{i}"), audio());
        }
        // A listen makes it the latest: the oldest one goes instead.
        assert!(kept("f0").is_some());
        keep("new", audio());
        assert!(kept("f0").is_some());
        assert!(kept("f1").is_none());
        assert!(kept("new").is_some());
        assert_eq!(KEPT_AUDIO.lock().unwrap().len(), KEPT);
    }
}

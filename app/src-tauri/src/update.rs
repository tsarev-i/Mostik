//! Self-update from GitHub releases, as in Slack: a background check shows an "Update" button,
//! a click downloads the new version, installs it and restarts Mostik. The background check can
//! be turned off in Preferences → Updates, which also checks and installs on demand.
//!
//! The release workflow publishes `latest.json` with minisign signatures of the bundles; the
//! updater plugin checks them against the public key in tauri.conf.json before installing.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Event with the new [`UpdateStatus`].
pub const UPDATE_EVENT: &str = "mostik://update";

/// First check soon after the start, then every few hours while the app keeps running.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

/// What the UI shows in the top bar and in Preferences → Updates.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// No update known.
    #[default]
    None,
    /// This build cannot update itself: it runs from `cargo` or was not installed from a bundle.
    Unsupported,
    /// A newer release is out: show the "Update" button.
    Available { version: String },
    /// Downloading and installing; `percent` is unknown without a Content-Length.
    Installing { version: String, percent: Option<u8> },
}

/// The update found by the last check, the status shown in the UI and the background check switch.
///
/// A check and an install both change `update` and the status; they do it while holding the
/// `update` lock, and `installing` keeps a check that was waiting for the network from bringing
/// the "Update" button back (and with it a second install) in the middle of an install.
pub struct UpdateState {
    update: Mutex<Option<Update>>,
    status: Mutex<UpdateStatus>,
    installing: AtomicBool,
    /// Background checks are on until the UI passes the saved preference.
    auto: AtomicBool,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            update: Mutex::new(None),
            status: Mutex::new(if supported() {
                UpdateStatus::None
            } else {
                UpdateStatus::Unsupported
            }),
            installing: AtomicBool::new(false),
            auto: AtomicBool::new(true),
        }
    }
}

/// Only the bundles the updater can replace: the AppImage and the NSIS setup.
/// Builds run from `cargo` or installed some other way are left alone.
fn supported() -> bool {
    matches!(
        tauri::utils::platform::bundle_type(),
        Some(BundleType::AppImage | BundleType::Nsis)
    )
}

fn set_status(app: &AppHandle, status: UpdateStatus) {
    let state = app.state::<UpdateState>();
    let mut current = state.status.lock().unwrap_or_else(|e| e.into_inner());
    if *current != status {
        *current = status.clone();
        let _ = app.emit(UPDATE_EVENT, &status);
    }
}

fn status(app: &AppHandle) -> UpdateStatus {
    app.state::<UpdateState>()
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Starts the periodic update check in the background; it skips its turns while turned off.
pub fn start(app: &AppHandle) {
    if !supported() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if app.state::<UpdateState>().auto.load(Ordering::Relaxed)
                && let Err(e) = check(&app).await
            {
                eprintln!("update check failed: {e}");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

async fn check(app: &AppHandle) -> Result<UpdateStatus, String> {
    if matches!(status(app), UpdateStatus::Installing { .. } | UpdateStatus::Unsupported) {
        return Ok(status(app));
    }
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let new = match &found {
        Some(update) => UpdateStatus::Available {
            version: update.version.clone(),
        },
        None => UpdateStatus::None,
    };
    let state = app.state::<UpdateState>();
    let mut update = state.update.lock().unwrap_or_else(|e| e.into_inner());
    // An install may have started while this check waited for the network: it wins.
    if state.installing.load(Ordering::SeqCst) {
        return Ok(status(app));
    }
    *update = found;
    set_status(app, new.clone());
    Ok(new)
}

/// Current status, for a UI that starts after the check has already finished.
#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    status(&app)
}

/// Turns the background check on or off (Preferences → Updates). Turning it on checks at once.
#[tauri::command]
pub fn set_auto_update(app: AppHandle, enabled: bool) {
    let was = app.state::<UpdateState>().auto.swap(enabled, Ordering::Relaxed);
    if enabled && !was && supported() {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = check(&app).await {
                eprintln!("update check failed: {e}");
            }
        });
    }
}

/// Checks for a new version now ("Check for updates" in Preferences).
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateStatus, String> {
    check(&app).await.map_err(|e| {
        format!(
            "{}: {e}",
            crate::backend::text(&app, "Не удалось проверить обновления", "Could not check for updates")
        )
    })
}

/// Downloads and installs the update found by the last check, then restarts Mostik.
/// On Windows the NSIS setup takes over: the app exits and the setup starts it again.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    let (update, version) = {
        let mut found = state.update.lock().unwrap_or_else(|e| e.into_inner());
        if state.installing.load(Ordering::SeqCst) {
            return Err(crate::backend::text(
                &app,
                "Обновление уже устанавливается",
                "The update is already installing",
            )
            .to_owned());
        }
        let Some(update) = found.take() else {
            return Err(crate::backend::text(&app, "Обновление не найдено", "No update found").to_owned());
        };
        state.installing.store(true, Ordering::SeqCst);
        let version = update.version.clone();
        set_status(
            &app,
            UpdateStatus::Installing {
                version: version.clone(),
                percent: Some(0),
            },
        );
        (update, version)
    };
    let mut received = 0u64;
    let progress = |chunk: usize, total: Option<u64>| {
        received += chunk as u64;
        let percent = total.filter(|t| *t > 0).map(|t| (received * 100 / t).min(100) as u8);
        set_status(
            &app,
            UpdateStatus::Installing {
                version: version.clone(),
                percent,
            },
        );
    };
    match update.download_and_install(progress, || {}).await {
        Ok(()) => {
            #[cfg(target_os = "linux")]
            if let Some(appimage) = std::env::var_os("APPIMAGE") {
                match relaunch_appimage(&appimage) {
                    Ok(()) => {
                        app.exit(0);
                        return Ok(());
                    }
                    Err(e) => eprintln!("relaunch failed, restarting as is: {e}"),
                }
            }
            app.restart()
        }
        Err(e) => {
            // Keep the update so that the button can try again.
            let mut found = state.update.lock().unwrap_or_else(|e| e.into_inner());
            *found = Some(update);
            state.installing.store(false, Ordering::SeqCst);
            set_status(&app, UpdateStatus::Available { version });
            drop(found);
            Err(format!(
                "{}: {e}",
                crate::backend::text(&app, "Не удалось обновить Мостик", "Could not update Mostik")
            ))
        }
    }
}

/// Starts the updated AppImage. `AppHandle::restart` would pass on what the old AppImage set up
/// for itself: its keepalive pipe (the old runtime and mount then live as long as the new app),
/// its mount as the working directory and in `PATH`, `LD_LIBRARY_PATH` and the other search
/// paths. The old mount goes away once this process exits, so all of that is dropped here.
#[cfg(target_os = "linux")]
fn relaunch_appimage(appimage: &std::ffi::OsStr) -> Result<(), String> {
    close_fds_on_exec();
    let mut command = std::process::Command::new(appimage);
    command.args(std::env::args_os().skip(1));
    if let Some(appdir) = crate::appimage::appdir() {
        command.env_clear();
        command.envs(crate::appimage::clean_env(std::env::vars_os(), &appdir));
    }
    // AppRun keeps the directory the app was started from in OWD; HOME if that is gone too.
    let cwd = ["OWD", "HOME"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .find(|d| d.is_dir() && !std::env::var_os("APPDIR").is_some_and(|a| d.starts_with(a)))
        .unwrap_or_else(|| "/".into());
    command.current_dir(cwd);
    command
        .spawn()
        .map(drop)
        .map_err(|e| format!("{}: {e}", appimage.to_string_lossy()))
}

/// The AppImage runtime keeps its FUSE mount, and itself, alive while anyone holds its keepalive
/// pipe. The restarted app would inherit the pipe and keep the old version mounted until it exits,
/// so every descriptor above stdio is marked close-on-exec before the restart.
#[cfg(target_os = "linux")]
fn close_fds_on_exec() {
    let Ok(dir) = std::fs::read_dir("/proc/self/fd") else {
        return;
    };
    let fds: Vec<i32> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd > 2)
        .collect();
    for fd in fds {
        // SAFETY: fcntl on a descriptor number only reads or sets its flags; a descriptor that
        // has already been closed (such as the one of the directory listing) returns an error.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

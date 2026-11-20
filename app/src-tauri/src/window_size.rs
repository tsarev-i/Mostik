//! The main window opens at the size it had when Mostik last closed, and maximized if it was.
//!
//! The size is followed as the window is resized and written to `window.json` in the app data
//! folder a moment after it settles, so it survives an exit that never gets to an exit event (the
//! session ending, a signal). A maximized or full-screen window keeps the size it had before: that
//! is the one it gets back when it is restored.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalSize, Manager, Runtime, Window};

const FILE: &str = "window.json";

/// How long the size has to stay put before it is written.
const SETTLE: Duration = Duration::from_millis(700);

/// The main window's size in logical pixels and whether it was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

impl Size {
    /// The saved size, kept within the window's minimum and a sane maximum: a damaged file or
    /// one from a much larger screen must not give a window that cannot be used.
    fn sane(self, min: (f64, f64)) -> Option<Self> {
        let ok = |v: f64| v.is_finite() && v > 0.0;
        if !ok(self.width) || !ok(self.height) {
            return None;
        }
        Some(Self {
            width: self.width.clamp(min.0, 16_384.0),
            height: self.height.clamp(min.1, 16_384.0),
            maximized: self.maximized,
        })
    }
}

/// The size being followed and a counter that tells the latest write from earlier ones.
#[derive(Default)]
pub struct WindowSize {
    size: Mutex<Option<Size>>,
    generation: AtomicU64,
}

fn path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|dir| dir.join(FILE))
}

/// The size the main window had, if it was saved; `min` is the window's minimum size.
pub fn load<R: Runtime>(app: &AppHandle<R>, min: (f64, f64)) -> Option<Size> {
    let text = std::fs::read_to_string(path(app)?).ok()?;
    let size: Size = serde_json::from_str(&text).ok()?;
    let size = size.sane(min)?;
    *app.state::<WindowSize>().size.lock().unwrap_or_else(|e| e.into_inner()) = Some(size);
    Some(size)
}

/// Follows a resize of the main window and writes the size once it settles.
pub fn resized<R: Runtime>(window: &Window<R>) {
    let app = window.app_handle();
    let state = app.state::<WindowSize>();
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let full = window.is_fullscreen().unwrap_or(false);
    {
        let mut size = state.size.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = size.unwrap_or(Size {
            width: 0.0,
            height: 0.0,
            maximized,
        });
        next.maximized = maximized;
        // Maximized or full screen, the size to come back to stays the one before.
        if !maximized && !full {
            let Ok(inner) = window.inner_size() else {
                return;
            };
            let scale = window.scale_factor().unwrap_or(1.0);
            let logical: LogicalSize<f64> = inner.to_logical(scale);
            next.width = logical.width;
            next.height = logical.height;
        }
        if next.width <= 0.0 || *size == Some(next) {
            return;
        }
        *size = Some(next);
    }
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE).await;
        let state = app.state::<WindowSize>();
        if state.generation.load(Ordering::SeqCst) == generation {
            write(&app);
        }
    });
}

/// Writes the size followed so far; on exit, too, in case the last resize has not settled.
pub fn write<R: Runtime>(app: &AppHandle<R>) {
    let Some(size) = *app.state::<WindowSize>().size.lock().unwrap_or_else(|e| e.into_inner()) else {
        return;
    };
    let Some(path) = path(app) else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_vec(&size).unwrap_or_default()));
    if let Err(e) = written {
        eprintln!("could not save the window size: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_size_is_kept_usable() {
        let min = (900.0, 600.0);
        let size = |width, height| Size {
            width,
            height,
            maximized: false,
        };
        assert_eq!(size(1400.0, 900.0).sane(min), Some(size(1400.0, 900.0)));
        assert_eq!(size(300.0, 200.0).sane(min), Some(size(900.0, 600.0)));
        assert_eq!(size(1e9, 700.0).sane(min), Some(size(16_384.0, 700.0)));
        assert_eq!(size(f64::NAN, 700.0).sane(min), None);
        assert_eq!(size(0.0, 700.0).sane(min), None);
        let text = serde_json::to_string(&Size {
            width: 1300.5,
            height: 820.0,
            maximized: true,
        })
        .unwrap();
        assert_eq!(text, r#"{"width":1300.5,"height":820.0,"maximized":true}"#);
    }
}

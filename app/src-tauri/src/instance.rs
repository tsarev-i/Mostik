//! One Mostik at a time: a second start shows the window of the running one and exits, also when
//! that window is hidden in the tray (#68).
//!
//! On Linux the running Mostik listens on a Unix socket of its own data directory, so a copy with
//! other data (`XDG_DATA_HOME`) runs on its own. Before the restart after an update the socket goes
//! away, or the new process would hand over to the old one, which is about to exit. On Windows
//! `tauri-plugin-single-instance` does it (see `main.rs`); on macOS the system starts an app once.

use tauri::AppHandle;

/// Whether this process is the one to run. If another Mostik with the same data runs, it is told
/// to show its window and this one should exit.
#[cfg(target_os = "linux")]
pub fn claim(app: &AppHandle) -> bool {
    imp::claim(app)
}

#[cfg(not(target_os = "linux"))]
pub fn claim(_app: &AppHandle) -> bool {
    true
}

/// Lets the next start run on its own: the restart after an update and the exit.
#[cfg(target_os = "linux")]
pub fn release() {
    imp::release();
}

#[cfg(not(target_os = "linux"))]
pub fn release() {}

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};

    /// What a second start sends.
    const SHOW: &str = "show";

    /// The socket this process listens on, and its inode: removed only while it is still ours.
    static OWN: OnceLock<(PathBuf, u64)> = OnceLock::new();

    pub fn claim(app: &AppHandle) -> bool {
        let Some(path) = socket_path(app) else { return true };
        // Two starts at once take turns: otherwise each could find no one answering, and the
        // second would remove the socket the first has just made.
        let _turn = lock(&path.with_extension("lock"));
        for _ in 0..2 {
            if let Ok(mut other) = UnixStream::connect(&path) {
                let _ = other.set_write_timeout(Some(Duration::from_secs(2)));
                if writeln!(other, "{SHOW}").is_ok() {
                    eprintln!("Mostik is already running: showing its window");
                    return false;
                }
            }
            // No one answers: a socket left by a Mostik that is gone.
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(listener) => {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        let _ = OWN.set((path.clone(), meta.ino()));
                    }
                    listen(app.clone(), listener);
                    return true;
                }
                // Another start got there first: hand over to it.
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
                Err(e) => {
                    eprintln!("single instance: {}: {e}", path.display());
                    return true;
                }
            }
        }
        true
    }

    /// An exclusive `flock` on `path`, held until the file is dropped; none if it cannot be had.
    fn lock(path: &std::path::Path) -> Option<std::fs::File> {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()?;
        // SAFETY: plain syscall on a descriptor this function owns.
        (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0).then_some(file)
    }

    pub fn release() {
        if let Some((path, ino)) = OWN.get()
            && std::fs::metadata(path).is_ok_and(|m| m.ino() == *ino)
        {
            let _ = std::fs::remove_file(path);
        }
    }

    fn listen(app: AppHandle, listener: UnixListener) {
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() && line.trim() == SHOW {
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || crate::show_main(&app2));
                }
            }
        });
    }

    /// In the user's runtime directory, named after the data directory: a socket path is limited
    /// to 108 bytes, which a data directory deep in the tree would exceed.
    fn socket_path(app: &AppHandle) -> Option<PathBuf> {
        let data = app.path().app_data_dir().ok()?;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute() && d.is_dir())
            .map(|d| d.join(&app.config().identifier))
            .unwrap_or_else(|| data.clone());
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .ok()?;
        Some(dir.join(format!(
            "instance-{:016x}.sock",
            fnv1a(data.as_os_str().as_encoded_bytes())
        )))
    }

    /// FNV-1a: the same name in every version of Mostik, unlike `DefaultHasher`.
    pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn socket_names_stay_the_same() {
        assert_eq!(super::imp::fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(super::imp::fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}

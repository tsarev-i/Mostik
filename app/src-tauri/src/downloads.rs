//! Attachments saved to "Downloads".
//!
//! The file name comes from someone else's message, so it is made into one plain file name that
//! every supported system accepts as is: no path separators or drive prefixes (on Windows
//! `C:x.dll` would leave "Downloads" for the current directory of drive C), no NTFS stream
//! (`a.txt:b`), no characters Windows refuses, no device names (`CON`, `NUL.txt`) and no dots or
//! spaces at the end, which Windows silently drops. The same rules apply everywhere, so a name
//! looks the same on every system.

use std::path::{Component, Path, PathBuf};

/// Characters Windows does not allow in file names, besides the control characters.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Windows device names: with any extension (`NUL.txt`) they still mean the device.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Longest name kept, in bytes: file systems allow 255, and a copy gets ` (n)` added.
const MAX_LEN: usize = 200;

/// The longest extension kept apart when a name is shortened or numbered.
const MAX_EXT: usize = 16;

/// The name to save an attachment under: one plain file name, see the module docs.
pub fn safe_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || FORBIDDEN.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut safe = replaced.trim().trim_end_matches(['.', ' ']).to_owned();
    // A leading dot hides the file on Linux and macOS.
    if safe.is_empty() || safe.starts_with('.') {
        safe.insert_str(0, "file");
    }
    let device = safe.split('.').next().unwrap_or_default().trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(device)) {
        safe.insert(0, '_');
    }
    shorten(&safe)
}

/// Name and extension (with its dot): numbered copies go between them.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= MAX_EXT => name.split_at(i),
        _ => (name, ""),
    }
}

/// Cuts a long name down to [`MAX_LEN`] bytes, keeping the extension.
fn shorten(name: &str) -> String {
    if name.len() <= MAX_LEN {
        return name.to_owned();
    }
    let (stem, ext) = split_ext(name);
    let mut end = MAX_LEN - ext.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", stem[..end].trim_end_matches(['.', ' ']))
}

/// Creates a new file `name` (from [`safe_name`]) in `dir`; if that name is taken, `name (1)`,
/// `name (2)` and so on. The file is created only if it does not exist, in the same step, so
/// nothing that appears meanwhile is overwritten. Returns its path and the file to write.
pub async fn create_new(dir: &Path, name: &str) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    let (stem, ext) = split_ext(name);
    for n in 0..10_000 {
        let candidate = if n == 0 {
            name.to_owned()
        } else {
            format!("{stem} ({n}){ext}")
        };
        let path = inside(dir, &candidate)?;
        match tokio::fs::File::create_new(&path).await {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many files with this name",
    ))
}

/// `dir/name`, refused unless `name` is one plain component, so the file lands right in `dir`.
fn inside(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let mut parts = Path::new(name).components();
    let plain = matches!((parts.next(), parts.next()), (Some(Component::Normal(n)), None) if n == name);
    let path = dir.join(name);
    if plain && path.parent() == Some(dir) {
        Ok(path)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a plain file name: {name}"),
        ))
    }
}

/// Marks a saved file as downloaded from the internet (the `Zone.Identifier` stream browsers
/// write), so Windows warns before running a program or script from it and Office opens it in
/// Protected View. A file system without streams (FAT) just keeps the file unmarked.
#[cfg(windows)]
pub fn mark_from_internet(path: &Path) {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    if let Err(e) = std::fs::write(&stream, "[ZoneTransfer]\r\nZoneId=3\r\n") {
        eprintln!("could not mark {} as downloaded: {e}", path.display());
    }
}

/// Marks a saved file as downloaded from the internet (the `com.apple.quarantine` attribute
/// browsers set), so Gatekeeper checks a program or script before it runs from it.
#[cfg(target_os = "macos")]
pub fn mark_from_internet(path: &Path) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let marked = std::process::Command::new("xattr")
        .args(["-w", "com.apple.quarantine", &format!("0081;{now:x};Mostik;")])
        .arg(path)
        .status();
    if !marked.is_ok_and(|s| s.success()) {
        eprintln!("could not mark {} as downloaded", path.display());
    }
}

/// Linux has no common mark for downloaded files: what runs is up to its execute bit, which a
/// saved file does not get.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn mark_from_internet(_path: &Path) {}

/// Shows a saved file in its folder, selected, as Slack does, instead of opening it: an attachment
/// comes from someone else, and opening `invoice.pdf.exe` would run it.
#[cfg(windows)]
pub fn reveal(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // As one raw argument: explorer wants the quotes around the path only, after the comma.
    // The name has no quotes (see `safe_name`).
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not show the file: {e}"))
}

/// Shows a saved file in Finder, selected.
#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not show the file: {e}"))
}

/// Shows a saved file in the file manager, selected (`org.freedesktop.FileManager1`, which
/// Nautilus, Dolphin, Nemo and Thunar provide); without it, opens its folder.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn reveal(path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut shown = Command::new("gdbus");
    shown
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("['{}']", file_uri(path)))
        .arg("")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::appimage::outside(&mut shown);
    if shown.status().is_ok_and(|s| s.success()) {
        return Ok(());
    }
    let mut folder = Command::new("xdg-open");
    folder.arg(path.parent().unwrap_or(path));
    crate::appimage::outside(&mut folder);
    folder
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not show the file: {e}"))
}

/// `file://` URI of an absolute path, every byte but the unreserved ones and `/` escaped (so a
/// quote in a name cannot end the GVariant string it goes into).
#[cfg(not(windows))]
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn file_uris_escape_all_but_plain_characters() {
        assert_eq!(
            file_uri(Path::new("/home/u/Загрузки/a b'c.pdf")),
            "file:///home/u/%D0%97%D0%B0%D0%B3%D1%80%D1%83%D0%B7%D0%BA%D0%B8/a%20b%27c.pdf"
        );
    }

    #[test]
    fn plain_names_stay() {
        assert_eq!(safe_name("report.pdf"), "report.pdf");
        assert_eq!(safe_name("Отчёт за май (итог).docx"), "Отчёт за май (итог).docx");
        assert_eq!(safe_name("archive.tar.gz"), "archive.tar.gz");
    }

    #[test]
    fn separators_drives_and_streams_go() {
        assert_eq!(safe_name("../../.bashrc"), "file.._.._.bashrc");
        assert_eq!(safe_name("..\\evil.exe"), "file.._evil.exe");
        assert_eq!(safe_name("C:x.dll"), "C_x.dll");
        assert_eq!(safe_name("C:\\Windows\\x.dll"), "C__Windows_x.dll");
        assert_eq!(safe_name("a.txt:Zone.Identifier"), "a.txt_Zone.Identifier");
        assert_eq!(safe_name("/etc/passwd"), "_etc_passwd");
    }

    #[test]
    fn characters_windows_refuses_go() {
        assert_eq!(safe_name("a<b>c\"d|e?f*g.txt"), "a_b_c_d_e_f_g.txt");
        assert_eq!(safe_name("tab\there\u{0}.txt"), "tab_here_.txt");
    }

    #[test]
    fn device_names_are_not_devices() {
        assert_eq!(safe_name("CON"), "_CON");
        assert_eq!(safe_name("nul.txt"), "_nul.txt");
        assert_eq!(safe_name("Com1.tar.gz"), "_Com1.tar.gz");
        assert_eq!(safe_name("LPT9 .log"), "_LPT9 .log");
        assert_eq!(safe_name("conin$"), "_conin$");
        // Only the whole first part counts.
        assert_eq!(safe_name("console.log"), "console.log");
        assert_eq!(safe_name("COM10.txt"), "COM10.txt");
    }

    #[test]
    fn trailing_dots_and_spaces_and_empty_names() {
        assert_eq!(safe_name("notes.txt. . "), "notes.txt");
        assert_eq!(safe_name("  spaced  "), "spaced");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name(".env"), "file.env");
    }

    #[test]
    fn long_names_keep_the_extension() {
        let long = format!("{}.pdf", "я".repeat(300));
        let safe = safe_name(&long);
        assert!(safe.len() <= MAX_LEN, "{} bytes", safe.len());
        assert!(safe.ends_with("я.pdf"));
    }

    #[test]
    fn only_plain_names_go_into_the_dir() {
        let dir = Path::new("downloads");
        assert_eq!(inside(dir, "a.txt").unwrap(), dir.join("a.txt"));
        for bad in ["../a.txt", "sub/a.txt", "..", ".", "", "/abs"] {
            assert!(inside(dir, bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn copies_are_numbered_and_nothing_is_overwritten() {
        let dir = std::env::temp_dir().join(format!("mostik-downloads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &'static str, body: &'static str| {
            let dir = dir.clone();
            async move {
                let (path, mut file) = create_new(&dir, name).await.unwrap();
                file.write_all(body.as_bytes()).await.unwrap();
                file.flush().await.unwrap();
                path
            }
        };
        let first = save("a.txt", "1").await;
        let second = save("a.txt", "2").await;
        let bare = save("README", "3").await;
        let bare2 = save("README", "4").await;
        assert_eq!(first, dir.join("a.txt"));
        assert_eq!(second, dir.join("a (1).txt"));
        assert_eq!(bare, dir.join("README"));
        assert_eq!(bare2, dir.join("README (1)"));
        assert_eq!(std::fs::read(&first).unwrap(), b"1");
        assert_eq!(std::fs::read(&second).unwrap(), b"2");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

//! "Report a bug" in the profile menu: the browser opens a new GitHub issue, its form already
//! holding the version of Mostik and the system it runs on. The app sends nothing itself: the
//! user describes the bug, adds a screenshot and submits the issue there, signed in to GitHub.

use mostik_ipc::cmd::{self, Out};
use tauri::Url;

use super::open_external;

/// New issue in the repository.
const NEW_ISSUE: &str = "https://github.com/tsarev-i/Mostik/issues/new";
/// The issue form, `.github/ISSUE_TEMPLATE/bug.yml`; the query fills its fields by their ids,
/// [`VERSION_FIELD`] and [`SYSTEM_FIELD`]. GitHub takes forms from the default branch only, and a
/// form renamed or removed there, or a field id changed, breaks the link in the copies already
/// installed.
const TEMPLATE: &str = "bug.yml";
const VERSION_FIELD: &str = "version";
const SYSTEM_FIELD: &str = "system";

#[tauri::command]
pub fn report_bug() -> Result<Out<cmd::ReportBug>, String> {
    open_external(issue_url(&version(), &system()).as_str())
}

/// The form with `version` and `system` filled in. No `labels` in the query: GitHub answers it
/// with 404 to anyone who may not label issues, so the form sets the label itself.
fn issue_url(version: &str, system: &str) -> Url {
    let mut url = Url::parse(NEW_ISSUE).expect("a valid address");
    url.query_pairs_mut()
        .append_pair("template", TEMPLATE)
        .append_pair(VERSION_FIELD, version)
        .append_pair(SYSTEM_FIELD, system);
    url
}

/// The version as in "About", and whether this is a debug build.
fn version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if cfg!(debug_assertions) {
        format!("{version} (debug)")
    } else {
        version.to_owned()
    }
}

/// One line each: the system and processor, the desktop and window system (Linux), the AppImage
/// if Mostik runs from one, and the web engine the interface runs in. No names, accounts or
/// paths: the issue is public.
fn system() -> String {
    let mut lines = vec![format!("{}, {}", os_name(), std::env::consts::ARCH)];
    #[cfg(target_os = "linux")]
    {
        let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let desktop = [
            set("XDG_CURRENT_DESKTOP"),
            window_system(
                set("GDK_BACKEND").as_deref(),
                set("WAYLAND_DISPLAY").is_some(),
                set("DISPLAY").is_some(),
            ),
        ];
        let desktop = desktop.into_iter().flatten().collect::<Vec<_>>();
        if !desktop.is_empty() {
            lines.push(desktop.join(", "));
        }
        if std::env::var_os("APPIMAGE").is_some() {
            lines.push("AppImage".to_owned());
        }
    }
    if let Ok(engine) = tauri::webview_version() {
        lines.push(format!("{WEB_ENGINE} {engine}"));
    }
    lines.join("\n")
}

/// The window system GTK opens Mostik's windows on, picked as GTK picks it: the first backend of
/// `GDK_BACKEND` (`*` for any) that has a display, Wayland before X11. Not `XDG_SESSION_TYPE`,
/// which is the session's: with `GDK_BACKEND=x11` Mostik runs on XWayland in a Wayland session.
#[cfg(any(target_os = "linux", test))]
fn window_system(gdk_backend: Option<&str>, wayland: bool, x11: bool) -> Option<String> {
    let has = |backend: &str| match backend {
        "wayland" => wayland,
        "x11" => x11,
        _ => false,
    };
    let any = || ["wayland", "x11"].into_iter().find(|b| has(b));
    let chosen = match gdk_backend {
        Some(list) => list.split(',').map(str::trim).find_map(|b| match b {
            "*" => any(),
            b => ["wayland", "x11"].into_iter().find(|known| *known == b && has(b)),
        }),
        None => any(),
    }?;
    Some(
        match (chosen, wayland) {
            ("wayland", _) => "Wayland",
            (_, true) => "X11 (XWayland)",
            _ => "X11",
        }
        .to_owned(),
    )
}

#[cfg(target_os = "linux")]
const WEB_ENGINE: &str = "WebKitGTK";
#[cfg(target_os = "windows")]
const WEB_ENGINE: &str = "WebView2";
#[cfg(target_os = "macos")]
const WEB_ENGINE: &str = "WebKit";

/// "Fedora Linux 43 (Workstation Edition)", from os-release.
#[cfg(target_os = "linux")]
fn os_name() -> String {
    ["/etc/os-release", "/usr/lib/os-release"]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|text| os_release_name(&text))
        })
        .unwrap_or_else(|| "Linux".to_owned())
}

/// `PRETTY_NAME` of an os-release file, or `NAME` if it has none.
#[cfg(any(target_os = "linux", test))]
fn os_release_name(text: &str) -> Option<String> {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
            .filter(|v| !v.is_empty())
    };
    value("PRETTY_NAME").or_else(|| value("NAME"))
}

/// "macOS 15.1".
#[cfg(target_os = "macos")]
fn os_name() -> String {
    let version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    format!("macOS {version}").trim_end().to_owned()
}

/// "Windows 11 24H2 (26100)". The registry, not `GetVersionEx`, which reports Windows 8 to an app
/// whose manifest does not list newer versions; Windows 11 still calls itself "Windows 10" in
/// `ProductName`, so the name comes from the build number.
#[cfg(target_os = "windows")]
fn os_name() -> String {
    let build = registry_text("CurrentBuild").and_then(|b| b.parse::<u32>().ok());
    let release = registry_text("DisplayVersion");
    windows_name(build, release.as_deref())
}

#[cfg(any(target_os = "windows", test))]
fn windows_name(build: Option<u32>, release: Option<&str>) -> String {
    let mut name = match build {
        Some(b) if b >= 22000 => "Windows 11".to_owned(),
        Some(_) => "Windows 10".to_owned(),
        None => "Windows".to_owned(),
    };
    if let Some(release) = release.filter(|r| !r.is_empty()) {
        name = format!("{name} {release}");
    }
    if let Some(build) = build {
        name = format!("{name} ({build})");
    }
    name
}

/// A string value of `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
#[cfg(target_os = "windows")]
fn registry_text(value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::{HSTRING, w};

    let name = HSTRING::from(value);
    let mut buf = [0u16; 128];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: the key and value names are NUL-terminated UTF-16, and `size` is the size of `buf`
    // in bytes, which the call does not write past.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    };
    if result.is_err() {
        return None;
    }
    // `size` counts the terminating NUL, in bytes.
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_gets_the_version_and_system() {
        let url = issue_url("0.7.1", "Arch Linux, x86_64\nKDE, Wayland\nWebKitGTK 2.50.1");
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(url.path(), "/tsarev-i/Mostik/issues/new");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query,
            [
                ("template".into(), "bug.yml".into()),
                ("version".into(), "0.7.1".into()),
                (
                    "system".into(),
                    "Arch Linux, x86_64\nKDE, Wayland\nWebKitGTK 2.50.1".into()
                ),
            ]
        );
        // Line breaks and `&` reach the form as text, not as query syntax.
        let odd = issue_url("1 & 2", "a=b\nc#d");
        assert!(!odd.as_str().contains('\n'));
        assert_eq!(odd.fragment(), None);
        assert_eq!(odd.query_pairs().count(), 3);
    }

    /// The form takes the fields by id: renaming one in `bug.yml` must change this file too.
    #[test]
    fn the_form_has_the_fields_filled_in() {
        let form = include_str!("../../../../.github/ISSUE_TEMPLATE/bug.yml");
        for field in [VERSION_FIELD, SYSTEM_FIELD] {
            assert!(
                form.contains(&format!("id: {field}\n")),
                "no field `{field}` in the form"
            );
        }
    }

    #[test]
    fn os_release_gives_the_pretty_name() {
        let arch = "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n";
        assert_eq!(os_release_name(arch).as_deref(), Some("Arch Linux"));
        let fedora = "NAME=\"Fedora Linux\"\nVERSION=\"43\"\nPRETTY_NAME='Fedora Linux 43 (KDE Plasma)'\n";
        assert_eq!(os_release_name(fedora).as_deref(), Some("Fedora Linux 43 (KDE Plasma)"));
        assert_eq!(os_release_name("NAME=Gentoo\n").as_deref(), Some("Gentoo"));
        // `PRETTY_NAME_X` is another key, and an empty value is none.
        assert_eq!(
            os_release_name("PRETTY_NAME_X=x\nPRETTY_NAME=\"\"\nNAME=Void\n").as_deref(),
            Some("Void")
        );
        assert_eq!(os_release_name("ID=unknown\n"), None);
    }

    #[test]
    fn windows_is_named_by_its_build() {
        assert_eq!(windows_name(Some(26100), Some("24H2")), "Windows 11 24H2 (26100)");
        assert_eq!(windows_name(Some(19045), Some("22H2")), "Windows 10 22H2 (19045)");
        assert_eq!(windows_name(None, None), "Windows");
    }

    #[test]
    fn the_window_system_is_the_one_gtk_picks() {
        let ws = |backend, wayland, x11| window_system(backend, wayland, x11);
        // A Wayland session: Wayland, unless the app is sent to XWayland.
        assert_eq!(ws(None, true, true).as_deref(), Some("Wayland"));
        assert_eq!(ws(Some("x11"), true, true).as_deref(), Some("X11 (XWayland)"));
        assert_eq!(ws(Some("wayland,x11"), true, true).as_deref(), Some("Wayland"));
        assert_eq!(ws(Some("*"), true, true).as_deref(), Some("Wayland"));
        // An X11 session, also when GDK_BACKEND asks for a Wayland it does not have.
        assert_eq!(ws(None, false, true).as_deref(), Some("X11"));
        assert_eq!(ws(Some("wayland,x11"), false, true).as_deref(), Some("X11"));
        assert_eq!(ws(Some(" x11 "), false, true).as_deref(), Some("X11"));
        // No display GTK can use.
        assert_eq!(ws(Some("broadway"), true, true), None);
        assert_eq!(ws(None, false, false), None);
    }
}

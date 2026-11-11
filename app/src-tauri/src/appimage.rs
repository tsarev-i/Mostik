//! Starting other programs from inside an AppImage.
//!
//! The AppImage runtime points `LD_LIBRARY_PATH`, `PATH`, `GIO_MODULE_DIR`, `XDG_DATA_DIRS` and
//! the like into its mount. A program started from Mostik with that environment would load the
//! image's libraries instead of its own: the browser crashes on them, and the AppImage restarted
//! after an update would hold on to the old mount. Every such program starts via [`outside`].

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// The mount of the running AppImage, if Mostik runs from one.
fn appdir() -> Option<String> {
    std::env::var("APPDIR").ok().filter(|d| !d.is_empty())
}

/// Prepares `command` to run outside the AppImage: the environment from [`clean_env`] and a
/// working directory off the mount (AppRun starts the app in it), so that a browser that outlives
/// Mostik does not keep the old mount busy. Outside an AppImage nothing changes.
pub fn outside(command: &mut Command) {
    let Some(appdir) = appdir() else {
        return;
    };
    command.env_clear();
    command.envs(clean_env(std::env::vars_os(), &appdir));
    // AppRun keeps the directory the app was started from in OWD; HOME if that is gone too.
    let cwd = ["OWD", "HOME"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|d| d.is_dir() && !d.starts_with(&appdir))
        .unwrap_or_else(|| "/".into());
    command.current_dir(cwd);
}

/// The environment without what points into the AppImage mount `appdir`: such entries of
/// path lists are removed, and a variable left with nothing is dropped.
fn clean_env(vars: impl IntoIterator<Item = (OsString, OsString)>, appdir: &str) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter_map(|(key, value)| match value.to_str() {
            Some(v) if v.contains(appdir) => {
                let kept: Vec<&str> = v
                    .split(':')
                    .filter(|part| !part.is_empty() && !part.contains(appdir))
                    .collect();
                (!kept.is_empty()).then(|| (key, kept.join(":").into()))
            }
            _ => Some((key, value)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::clean_env;

    fn cleaned(vars: &[(&str, &str)], appdir: &str) -> Vec<(String, String)> {
        let vars = vars.iter().map(|(k, v)| ((*k).into(), (*v).into()));
        clean_env(vars, appdir)
            .into_iter()
            .map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
            .collect()
    }

    #[test]
    fn mount_entries_are_dropped() {
        let env = cleaned(
            &[
                ("PATH", "/tmp/.mount_Old/usr/bin/:/usr/bin:/tmp/.mount_Old/bin"),
                ("LD_LIBRARY_PATH", "/tmp/.mount_Old/usr/lib"),
                ("XDG_DATA_DIRS", "/tmp/.mount_Old/usr/share:/usr/share"),
                ("GTK_DATA_PREFIX", "/tmp/.mount_Old"),
                ("PWD", "/tmp/.mount_Old/usr"),
                ("HOME", "/home/me"),
            ],
            "/tmp/.mount_Old",
        );
        let expected = [
            ("PATH", "/usr/bin"),
            ("XDG_DATA_DIRS", "/usr/share"),
            ("HOME", "/home/me"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        assert_eq!(env, expected);
    }

    #[test]
    fn a_mount_path_inside_a_value_counts_too() {
        let env = cleaned(
            &[("GST_PLUGIN_PATH", "x=/tmp/.mount_Old/lib:/usr/lib")],
            "/tmp/.mount_Old",
        );
        assert_eq!(env, [("GST_PLUGIN_PATH".to_owned(), "/usr/lib".to_owned())]);
    }
}

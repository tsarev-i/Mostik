//! Spelling dictionaries Mostik carries on Linux (#50). WebKitGTK checks spelling through enchant,
//! which finds hunspell dictionaries only where the system has put them, and many systems have none
//! for Russian or English. The Linux binary holds `ru_RU` and `en_US` (see `dictionaries/README.md`),
//! writes them to the app data folder at start and points enchant there with `ENCHANT_CONFIG_DIR`:
//! enchant reads `<dir>/hunspell`, and the system's dictionaries as well. Words added with "Learn
//! Spelling" are kept in that folder too.

use std::io;
use std::path::{Path, PathBuf};

/// The dictionaries with their licenses and the note on where they come from, by their path in the
/// enchant folder: the licenses ask for their notices to go with every copy.
const FILES: [(&str, &[u8]); 7] = [
    ("hunspell/en_US.aff", include_bytes!("../dictionaries/en_US.aff")),
    ("hunspell/en_US.dic", include_bytes!("../dictionaries/en_US.dic")),
    (
        "hunspell/LICENSE-en_US",
        include_bytes!("../dictionaries/LICENSE-en_US"),
    ),
    ("hunspell/ru_RU.aff", include_bytes!("../dictionaries/ru_RU.aff")),
    ("hunspell/ru_RU.dic", include_bytes!("../dictionaries/ru_RU.dic")),
    (
        "hunspell/LICENSE-ru_RU",
        include_bytes!("../dictionaries/LICENSE-ru_RU"),
    ),
    (
        "hunspell/README-dictionaries.md",
        include_bytes!("../dictionaries/README.md"),
    ),
];

/// `identifier` in tauri.conf.json: the app data folder is named after it. Tauri does not run yet
/// when the dictionaries are written.
const IDENTIFIER: &str = "dev.mostik.client";

const ENV: &str = "ENCHANT_CONFIG_DIR";

/// The enchant folder in the app data folder, where Tauri keeps it: `$XDG_DATA_HOME` or
/// `~/.local/share`.
fn enchant_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(data.join(IDENTIFIER).join("enchant"))
}

/// Writes the dictionaries and points enchant at them. To be called first in `main`, while the
/// process has one thread: it sets an environment variable. A folder the user has set for enchant
/// is left as it is.
pub fn install() {
    if std::env::var_os(ENV).is_some() {
        return;
    }
    let Some(dir) = enchant_dir() else {
        return;
    };
    if let Err(e) = write_files(&dir) {
        eprintln!("spelling dictionaries not written to {}: {e}", dir.display());
        return;
    }
    // SAFETY: `main` calls this before it starts Tauri, so no other thread reads the environment.
    unsafe { std::env::set_var(ENV, &dir) };
}

/// Writes each file that is missing or differs from the one Mostik carries, through a temporary
/// file, so a copy cut short never stays.
fn write_files(dir: &Path) -> io::Result<()> {
    for (name, bytes) in FILES {
        let path = dir.join(name);
        let same = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64)
            && std::fs::read(&path).is_ok_and(|have| have == bytes);
        if same {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = path.with_extension("part");
        std::fs::write(&part, bytes)?;
        std::fs::rename(&part, &path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_is_tauris() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], IDENTIFIER);
    }

    #[test]
    fn dictionaries_are_written_once_and_put_right() {
        let dir = std::env::temp_dir().join(format!("mostik-spelling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_files(&dir).unwrap();
        let aff = dir.join("hunspell/ru_RU.aff");
        assert_eq!(std::fs::read(&aff).unwrap(), FILES[3].1);
        assert!(
            std::fs::read_to_string(dir.join("hunspell/en_US.aff"))
                .unwrap()
                .starts_with("SET UTF-8")
        );
        // A damaged copy is written again; an intact one stays as it is.
        std::fs::write(&aff, b"broken").unwrap();
        let dic = dir.join("hunspell/en_US.dic");
        let before = std::fs::metadata(&dic).unwrap().modified().unwrap();
        write_files(&dir).unwrap();
        assert_eq!(std::fs::read(&aff).unwrap(), FILES[3].1);
        assert_eq!(std::fs::metadata(&dic).unwrap().modified().unwrap(), before);
        assert!(!dir.join("hunspell/ru_RU.part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

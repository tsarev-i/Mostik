//! Keyboard shortcuts of the platform, as in Slack: Cmd on macOS where Ctrl is elsewhere, and
//! their keys written as the system writes them (`⇧⌘X` on macOS, `Ctrl+Shift+X` elsewhere).

use leptos::ev::KeyboardEvent;

use crate::settings::on_mac;

/// The modifier of the app's shortcuts is held: Cmd on macOS, Ctrl elsewhere. On macOS Ctrl stays
/// the text field's: there Ctrl+B, Ctrl+K and the like move the caret and delete text.
pub fn command(e: &KeyboardEvent) -> bool {
    if on_mac() { e.meta_key() } else { e.ctrl_key() }
}

/// A shortcut as this platform writes it. `keys` is written as on Windows and Linux, the key last:
/// `"Ctrl+Shift+X"`, `"Ctrl+="`; its Ctrl is the [`command`] modifier.
pub fn label(keys: &str) -> String {
    written(keys, on_mac())
}

/// Modifiers in the order macOS menus show them; Ctrl of a shortcut is Cmd there.
const MAC_MODIFIERS: [(&str, &str); 3] = [("Alt", "⌥"), ("Shift", "⇧"), ("Ctrl", "⌘")];

fn written(keys: &str, mac: bool) -> String {
    if !mac {
        return keys.to_owned();
    }
    let (modifiers, key) = keys.rsplit_once('+').unwrap_or(("", keys));
    let held: Vec<&str> = modifiers.split('+').collect();
    let mut out: String = MAC_MODIFIERS
        .iter()
        .filter(|(name, _)| held.contains(name))
        .map(|(_, symbol)| *symbol)
        .collect();
    out.push_str(match key {
        "Enter" => "↩",
        key => key,
    });
    out
}

/// "Add link" in a message field. On Linux Ctrl+Shift+U never reaches the page: GTK and IBus take
/// it for typing a character by its Unicode code. The field takes both.
pub fn add_link() -> String {
    label(if crate::settings::on_linux() {
        "Ctrl+Shift+K"
    } else {
        "Ctrl+Shift+U"
    })
}

/// Back and forward through the opened chats: Alt+← / Alt+→ as in Slack, Cmd+[ / Cmd+] on macOS,
/// where Option+arrows in a text field move the caret by word.
pub fn back_forward() -> (String, String) {
    if on_mac() {
        (label("Ctrl+["), label("Ctrl+]"))
    } else {
        ("Alt+←".to_owned(), "Alt+→".to_owned())
    }
}

/// The keys that send a message and those that start a new line, by "Preferences → Advanced".
pub fn send_and_newline(enter_sends: bool) -> (String, String) {
    if enter_sends {
        (label("Enter"), label("Shift+Enter"))
    } else {
        (label("Ctrl+Enter"), label("Enter"))
    }
}

/// Enter sends the message, rather than starting a new line. With Enter sending, Shift+Enter (or
/// Ctrl+Enter, as before) starts a new line; otherwise the [`command`] modifier with it sends.
pub fn enter_sends_message(e: &KeyboardEvent, enter_sends: bool) -> bool {
    if enter_sends {
        !e.shift_key() && !e.ctrl_key()
    } else {
        command(e)
    }
}

#[cfg(test)]
mod tests {
    use super::written;

    #[test]
    fn keys_are_written_for_the_platform() {
        for keys in ["Ctrl+K", "Ctrl+Shift+X", "Ctrl+=", "Ctrl+-", "Shift+Enter", "Alt+←"] {
            assert_eq!(written(keys, false), keys);
        }
        assert_eq!(written("Ctrl+K", true), "⌘K");
        assert_eq!(written("Ctrl+Shift+X", true), "⇧⌘X");
        assert_eq!(written("Shift+Ctrl+U", true), "⇧⌘U");
        assert_eq!(written("Ctrl+=", true), "⌘=");
        assert_eq!(written("Ctrl+-", true), "⌘-");
        assert_eq!(written("Ctrl+0", true), "⌘0");
        assert_eq!(written("Ctrl+,", true), "⌘,");
        assert_eq!(written("Ctrl+Enter", true), "⌘↩");
        assert_eq!(written("Shift+Enter", true), "⇧↩");
        assert_eq!(written("Enter", true), "↩");
        assert_eq!(written("Alt+Shift+↑", true), "⌥⇧↑");
        assert_eq!(written("Esc", true), "Esc");
    }
}

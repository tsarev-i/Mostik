//! Keyboard shortcuts, as in Slack: Cmd on macOS where Ctrl is elsewhere, their keys written as the
//! system writes them (`⇧⌘X` on macOS, `Ctrl+Shift+X` elsewhere), and the user's own keys for the
//! [`Action`]s, set in "Preferences → Advanced".

use std::cell::RefCell;
use std::collections::BTreeMap;

use leptos::ev::KeyboardEvent;

use crate::i18n::tr;
use crate::settings::{on_linux, on_mac};

/// The modifier of the app's shortcuts is held: Cmd on macOS, Ctrl elsewhere. On macOS Ctrl stays
/// the text field's: there Ctrl+B, Ctrl+K and the like move the caret and delete text.
pub fn command(e: &KeyboardEvent) -> bool {
    if on_mac() { e.meta_key() } else { e.ctrl_key() }
}

/// Keys that cannot be changed, as this platform writes them. `keys` is written as on Windows and
/// Linux, the key last: `"Ctrl+Enter"`, `"Shift+Enter"`; its Ctrl is the [`command`] modifier.
pub fn label(keys: &str) -> String {
    written(keys, on_mac())
}

/// Modifiers in the order macOS menus show them; Ctrl of [`label`] is Cmd there.
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

/// What a shortcut the user can change does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Search,
    Back,
    Forward,
    Preferences,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Bold,
    Italic,
    Strike,
    Link,
}

impl Action {
    pub const ALL: [Self; 11] = [
        Self::Search,
        Self::Back,
        Self::Forward,
        Self::Preferences,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomReset,
        Self::Bold,
        Self::Italic,
        Self::Strike,
        Self::Link,
    ];

    /// Its key among the user's shortcuts in the settings.
    pub fn id(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Preferences => "preferences",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::ZoomReset => "zoom_reset",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Strike => "strike",
            Self::Link => "link",
        }
    }

    /// Its name in the list of shortcuts.
    pub fn name(self) -> &'static str {
        let t = tr();
        match self {
            Self::Search => t.sc_search,
            Self::Back => t.back,
            Self::Forward => t.forward,
            Self::Preferences => t.preferences,
            Self::ZoomIn => t.sc_zoom_in,
            Self::ZoomOut => t.sc_zoom_out,
            Self::ZoomReset => t.sc_zoom_reset,
            Self::Bold => t.fmt_bold,
            Self::Italic => t.fmt_italic,
            Self::Strike => t.fmt_strike,
            Self::Link => t.fmt_link,
        }
    }

    /// The standard keys, the one shown first, as in Slack. Back and forward are Alt+← / Alt+→,
    /// and Cmd+[ / Cmd+] on macOS, where Option+arrows in a text field move the caret by word. On
    /// Linux "Add link" is shown with K: GTK and IBus take Ctrl+Shift+U for typing a character by
    /// its Unicode code, and it never reaches the page.
    fn standard(self, mac: bool, linux: bool) -> Vec<Shortcut> {
        let cmd = |code: &str| Shortcut::command(code, false, mac);
        let cmd_shift = |code: &str| Shortcut::command(code, true, mac);
        let alt = |code: &str| Shortcut {
            alt: true,
            ..Shortcut::key(code)
        };
        let meta = |code: &str| Shortcut {
            meta: true,
            ..Shortcut::key(code)
        };
        let history = |arrow: &str, bracket: &str| {
            if mac {
                vec![meta(bracket), alt(arrow)]
            } else {
                vec![alt(arrow), meta(bracket)]
            }
        };
        match self {
            Self::Search => vec![cmd("KeyK")],
            Self::Back => history("ArrowLeft", "BracketLeft"),
            Self::Forward => history("ArrowRight", "BracketRight"),
            Self::Preferences => vec![cmd("Comma")],
            Self::ZoomIn => vec![cmd("Equal"), cmd_shift("Equal"), cmd("NumpadAdd")],
            Self::ZoomOut => vec![cmd("Minus"), cmd_shift("Minus"), cmd("NumpadSubtract")],
            Self::ZoomReset => vec![cmd("Digit0"), cmd("Numpad0")],
            Self::Bold => vec![cmd("KeyB")],
            Self::Italic => vec![cmd("KeyI")],
            Self::Strike => vec![cmd_shift("KeyX")],
            Self::Link if linux => vec![cmd_shift("KeyK"), cmd_shift("KeyU")],
            Self::Link => vec![cmd_shift("KeyU"), cmd_shift("KeyK")],
        }
    }

    /// The standard keys pressed: as the app took them before they could be changed. Zoom goes by
    /// the character, so that it works where `=` is on another key.
    fn standard_pressed(self, e: &KeyboardEvent) -> bool {
        let cmd = command(e) && !e.alt_key();
        let code = e.code();
        let history = |arrow: &str, bracket: &str| {
            (e.alt_key() && !e.ctrl_key() && e.key() == arrow && !(on_mac() && in_text_field()))
                || (e.meta_key() && !e.shift_key() && code == bracket)
        };
        match self {
            Self::Search => cmd && !e.shift_key() && code == "KeyK",
            Self::Back => history("ArrowLeft", "BracketLeft"),
            Self::Forward => history("ArrowRight", "BracketRight"),
            Self::Preferences => cmd && !e.shift_key() && code == "Comma",
            Self::ZoomIn => cmd && matches!(e.key().as_str(), "=" | "+"),
            Self::ZoomOut => cmd && matches!(e.key().as_str(), "-" | "_"),
            Self::ZoomReset => cmd && e.key() == "0",
            Self::Bold => cmd && !e.shift_key() && code == "KeyB",
            Self::Italic => cmd && !e.shift_key() && code == "KeyI",
            Self::Strike => cmd && e.shift_key() && code == "KeyX",
            Self::Link => cmd && e.shift_key() && matches!(code.as_str(), "KeyU" | "KeyK"),
        }
    }
}

/// Whether the focus is in a text field: an input, a text area or an editable element.
fn in_text_field() -> bool {
    leptos::prelude::document().active_element().is_some_and(|el| {
        matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA")
            || wasm_bindgen::JsCast::dyn_ref::<web_sys::HtmlElement>(&el).is_some_and(|el| el.is_content_editable())
    })
}

/// A key and the modifiers held with it. The key is taken by its place (`KeyboardEvent.code`), so
/// a shortcut works in any keyboard layout (in Russian the K key gives "л"). Saved as
/// `Ctrl+Shift+KeyX`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on macOS, the Windows or Super key elsewhere.
    pub meta: bool,
    pub code: String,
}

/// Keys that are only modifiers: a shortcut needs one more.
const MODIFIER_CODES: [&str; 13] = [
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
    "OSLeft",
    "OSRight",
    "CapsLock",
    "Fn",
    "FnLock",
];

impl Shortcut {
    fn key(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            ..Self::default()
        }
    }

    /// `code` with the [`command`] modifier of the platform.
    fn command(code: &str, shift: bool, mac: bool) -> Self {
        Self {
            ctrl: !mac,
            meta: mac,
            shift,
            ..Self::key(code)
        }
    }

    /// The keys of `e`; none while only modifiers are down.
    pub fn of(e: &KeyboardEvent) -> Option<Self> {
        Self::from_parts(e.code(), e.ctrl_key(), e.alt_key(), e.shift_key(), e.meta_key())
    }

    fn from_parts(code: String, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<Self> {
        if code.is_empty() || code.contains('+') || MODIFIER_CODES.contains(&code.as_str()) {
            return None;
        }
        Some(Self {
            ctrl,
            alt,
            shift,
            meta,
            code,
        })
    }

    /// How it is saved: the modifiers held, then the key's code.
    pub fn spec(&self) -> String {
        let mut out = String::new();
        for (held, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.meta, "Meta+"),
        ] {
            if held {
                out.push_str(name);
            }
        }
        out.push_str(&self.code);
        out
    }

    /// A shortcut saved by [`Shortcut::spec`]; nothing for one this version cannot read.
    pub fn parse(spec: &str) -> Option<Self> {
        let (modifiers, code) = spec.rsplit_once('+').unwrap_or(("", spec));
        let mut s = Self::from_parts(code.to_owned(), false, false, false, false)?;
        for name in modifiers.split('+').filter(|m| !m.is_empty()) {
            match name {
                "Ctrl" => s.ctrl = true,
                "Alt" => s.alt = true,
                "Shift" => s.shift = true,
                "Meta" => s.meta = true,
                _ => return None,
            }
        }
        Some(s)
    }

    /// As this platform writes it: `⇧⌘X` on macOS, `Ctrl+Shift+X` on Linux and Windows.
    pub fn label(&self) -> String {
        self.written(on_mac(), on_linux())
    }

    fn written(&self, mac: bool, linux: bool) -> String {
        let key = key_name(&self.code, mac);
        if mac {
            let mut out = String::new();
            for (held, symbol) in [(self.ctrl, "⌃"), (self.alt, "⌥"), (self.shift, "⇧"), (self.meta, "⌘")] {
                if held {
                    out.push_str(symbol);
                }
            }
            return out + &key;
        }
        let meta = if linux { "Super" } else { "Win" };
        let mut parts: Vec<&str> = [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, meta),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, name)| name)
        .collect();
        parts.push(&key);
        parts.join("+")
    }

    fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.meta
    }
}

/// The name of a key by its code, as the key is labelled.
fn key_name(code: &str, mac: bool) -> String {
    if let Some(letter) = code.strip_prefix("Key") {
        return letter.to_owned();
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        return digit.to_owned();
    }
    if let Some(key) = code.strip_prefix("Numpad") {
        let key = match key {
            "Add" => "+",
            "Subtract" => "-",
            "Multiply" => "*",
            "Divide" => "/",
            "Decimal" => ".",
            "Enter" if mac => "↩",
            key => key,
        };
        return format!("Num {key}");
    }
    let name = match code {
        "Equal" => "=",
        "Minus" => "-",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backslash" | "IntlBackslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backquote" => "`",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "Enter" if mac => "↩",
        "Backspace" if mac => "⌫",
        "Delete" if mac => "⌦",
        "Tab" if mac => "⇥",
        "Escape" => "Esc",
        code => code,
    };
    name.to_owned()
}

/// Why new keys were not taken for an action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// A key without Ctrl, Alt or Cmd would fire while typing.
    NeedModifier,
    /// The system's or a text field's: copy, paste, undo, Enter, the macOS app menu.
    System,
    /// Already the keys of another action.
    Taken(Action),
}

impl Refusal {
    pub fn text(self) -> String {
        match self {
            Self::NeedModifier => tr().sc_need_modifier(on_mac()),
            Self::System => tr().sc_system.to_owned(),
            Self::Taken(other) => tr().sc_taken(other.name()),
        }
    }
}

/// The user's keys of `action` from the settings (`custom`), else its standard ones.
fn keys_of(custom: &BTreeMap<String, String>, action: Action, mac: bool, linux: bool) -> Vec<Shortcut> {
    match custom.get(action.id()).and_then(|spec| Shortcut::parse(spec)) {
        Some(own) => vec![own],
        None => action.standard(mac, linux),
    }
}

/// The keys of `action` as this platform writes them.
pub fn shown(custom: &BTreeMap<String, String>, action: Action) -> String {
    keys_of(custom, action, on_mac(), on_linux())
        .first()
        .map(Shortcut::label)
        .unwrap_or_default()
}

/// Whether `new` can become the keys of `action`.
pub fn check(new: &Shortcut, action: Action, custom: &BTreeMap<String, String>) -> Result<(), Refusal> {
    check_on(new, action, custom, on_mac(), on_linux())
}

fn check_on(
    new: &Shortcut,
    action: Action,
    custom: &BTreeMap<String, String>,
    mac: bool,
    linux: bool,
) -> Result<(), Refusal> {
    let function_key =
        new.code.len() > 1 && new.code.starts_with('F') && new.code[1..].bytes().all(|b| b.is_ascii_digit());
    if !new.has_modifier() && !function_key {
        return Err(Refusal::NeedModifier);
    }
    let command = if mac {
        new.meta && !new.ctrl
    } else {
        new.ctrl && !new.meta
    };
    let edit_key = matches!(new.code.as_str(), "KeyA" | "KeyC" | "KeyV" | "KeyX" | "KeyZ" | "KeyY");
    let mac_menu = mac && new.meta && matches!(new.code.as_str(), "KeyQ" | "KeyW" | "KeyH" | "KeyM");
    let fixed_key = matches!(new.code.as_str(), "Enter" | "NumpadEnter" | "Escape" | "Tab");
    if (command && !new.alt && edit_key) || mac_menu || fixed_key {
        return Err(Refusal::System);
    }
    match Action::ALL
        .into_iter()
        .find(|&other| other != action && keys_of(custom, other, mac, linux).contains(new))
    {
        Some(other) => Err(Refusal::Taken(other)),
        None => Ok(()),
    }
}

/// Makes `new` the keys of `action`; one of its standard keys brings the standard ones back.
pub fn assign(custom: &mut BTreeMap<String, String>, action: Action, new: &Shortcut) {
    assign_on(custom, action, new, on_mac(), on_linux());
}

fn assign_on(custom: &mut BTreeMap<String, String>, action: Action, new: &Shortcut, mac: bool, linux: bool) {
    if action.standard(mac, linux).contains(new) {
        custom.remove(action.id());
    } else {
        custom.insert(action.id().to_owned(), new.spec());
    }
}

thread_local! {
    /// The user's keys from the settings, for the key handlers, which run outside the components.
    static CUSTOM: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Takes the user's keys from the settings (see `apply_settings`).
pub fn set_custom(custom: &BTreeMap<String, String>) {
    CUSTOM.with(|c| *c.borrow_mut() = custom.clone());
}

/// The keys of `action` are pressed: the user's own, or the standard ones unless they are the
/// user's keys of another action.
pub fn pressed(e: &KeyboardEvent, action: Action) -> bool {
    CUSTOM.with(|custom| {
        let custom = custom.borrow();
        let own = |a: Action| custom.get(a.id()).and_then(|spec| Shortcut::parse(spec));
        match own(action) {
            Some(keys) => Shortcut::of(e).is_some_and(|pressed| pressed == keys),
            None => {
                action.standard_pressed(e)
                    && !Shortcut::of(e).is_some_and(|pressed| {
                        Action::ALL
                            .into_iter()
                            .any(|other| own(other).as_ref() == Some(&pressed))
                    })
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_written_for_the_platform() {
        for keys in ["Ctrl+K", "Ctrl+Shift+X", "Ctrl+=", "Ctrl+-", "Shift+Enter", "Alt+←"] {
            assert_eq!(written(keys, false), keys);
        }
        assert_eq!(written("Ctrl+K", true), "⌘K");
        assert_eq!(written("Ctrl+Shift+X", true), "⇧⌘X");
        assert_eq!(written("Shift+Ctrl+U", true), "⇧⌘U");
        assert_eq!(written("Ctrl+=", true), "⌘=");
        assert_eq!(written("Ctrl+,", true), "⌘,");
        assert_eq!(written("Ctrl+Enter", true), "⌘↩");
        assert_eq!(written("Shift+Enter", true), "⇧↩");
        assert_eq!(written("Enter", true), "↩");
        assert_eq!(written("Esc", true), "Esc");
    }

    fn keys(spec: &str) -> Shortcut {
        Shortcut::parse(spec).unwrap()
    }

    #[test]
    fn shortcuts_are_saved_and_read_back() {
        for spec in [
            "Ctrl+KeyK",
            "Ctrl+Alt+Shift+Meta+Digit0",
            "Meta+BracketLeft",
            "F5",
            "Alt+ArrowLeft",
        ] {
            assert_eq!(keys(spec).spec(), spec);
        }
        assert_eq!(Shortcut::parse("Shift+Ctrl+KeyK"), Some(keys("Ctrl+Shift+KeyK")));
        assert_eq!(Shortcut::parse(""), None);
        assert_eq!(Shortcut::parse("Ctrl+"), None);
        assert_eq!(Shortcut::parse("Hyper+KeyK"), None);
        assert_eq!(Shortcut::parse("Ctrl+ShiftLeft"), None);
    }

    #[test]
    fn shortcuts_are_written_for_the_platform() {
        let mac = |spec: &str| keys(spec).written(true, false);
        let linux = |spec: &str| keys(spec).written(false, true);
        let windows = |spec: &str| keys(spec).written(false, false);
        assert_eq!(mac("Meta+KeyK"), "⌘K");
        assert_eq!(mac("Ctrl+Alt+Shift+Meta+KeyJ"), "⌃⌥⇧⌘J");
        assert_eq!(mac("Meta+BracketLeft"), "⌘[");
        assert_eq!(mac("Alt+ArrowLeft"), "⌥←");
        assert_eq!(mac("Meta+Enter"), "⌘↩");
        assert_eq!(linux("Ctrl+Shift+KeyX"), "Ctrl+Shift+X");
        assert_eq!(linux("Ctrl+Equal"), "Ctrl+=");
        assert_eq!(linux("Ctrl+Digit0"), "Ctrl+0");
        assert_eq!(linux("Ctrl+NumpadAdd"), "Ctrl+Num +");
        assert_eq!(linux("Alt+ArrowRight"), "Alt+→");
        assert_eq!(linux("Meta+KeyJ"), "Super+J");
        assert_eq!(windows("Meta+KeyJ"), "Win+J");
        assert_eq!(linux("F5"), "F5");
    }

    #[test]
    fn standard_keys_are_shown_for_the_platform() {
        let shown = |action: Action, mac: bool, linux: bool| action.standard(mac, linux)[0].written(mac, linux);
        assert_eq!(shown(Action::Search, true, false), "⌘K");
        assert_eq!(shown(Action::Search, false, true), "Ctrl+K");
        assert_eq!(shown(Action::Back, true, false), "⌘[");
        assert_eq!(shown(Action::Back, false, true), "Alt+←");
        assert_eq!(shown(Action::Preferences, true, false), "⌘,");
        assert_eq!(shown(Action::ZoomReset, false, false), "Ctrl+0");
        assert_eq!(shown(Action::Strike, true, false), "⇧⌘X");
        assert_eq!(shown(Action::Link, false, true), "Ctrl+Shift+K");
        assert_eq!(shown(Action::Link, false, false), "Ctrl+Shift+U");
        assert_eq!(shown(Action::Link, true, false), "⇧⌘U");
    }

    #[test]
    fn new_keys_are_checked() {
        let none = BTreeMap::new();
        let linux = |spec: &str, action: Action, custom: &BTreeMap<String, String>| {
            check_on(&keys(spec), action, custom, false, true)
        };
        let mac = |spec: &str, action: Action| check_on(&keys(spec), action, &none, true, false);
        assert_eq!(linux("Ctrl+KeyJ", Action::Search, &none), Ok(()));
        assert_eq!(linux("F6", Action::Search, &none), Ok(()));
        assert_eq!(linux("KeyJ", Action::Search, &none), Err(Refusal::NeedModifier));
        assert_eq!(linux("Shift+KeyJ", Action::Search, &none), Err(Refusal::NeedModifier));
        assert_eq!(linux("Ctrl+KeyC", Action::Bold, &none), Err(Refusal::System));
        assert_eq!(linux("Ctrl+Shift+KeyZ", Action::Bold, &none), Err(Refusal::System));
        assert_eq!(linux("Ctrl+Enter", Action::Bold, &none), Err(Refusal::System));
        assert_eq!(mac("Meta+KeyV", Action::Bold), Err(Refusal::System));
        assert_eq!(mac("Meta+KeyQ", Action::Search), Err(Refusal::System));
        // Ctrl+C on macOS is not copy.
        assert_eq!(mac("Ctrl+KeyC", Action::Search), Ok(()));
        // Taken by another action, its shown keys or the others it takes.
        assert_eq!(
            linux("Ctrl+KeyB", Action::Search, &none),
            Err(Refusal::Taken(Action::Bold))
        );
        assert_eq!(
            linux("Ctrl+Shift+KeyU", Action::Search, &none),
            Err(Refusal::Taken(Action::Link))
        );
        assert_eq!(mac("Alt+ArrowLeft", Action::Search), Err(Refusal::Taken(Action::Back)));
        // Its own standard keys are fine.
        assert_eq!(linux("Ctrl+KeyK", Action::Search, &none), Ok(()));
        // Keys given up by an action are free; the new keys of another are not.
        let mut custom = BTreeMap::new();
        assign_on(&mut custom, Action::Bold, &keys("Ctrl+KeyJ"), false, true);
        assert_eq!(linux("Ctrl+KeyB", Action::Italic, &custom), Ok(()));
        assert_eq!(
            linux("Ctrl+KeyJ", Action::Italic, &custom),
            Err(Refusal::Taken(Action::Bold))
        );
    }

    #[test]
    fn standard_keys_are_not_saved() {
        let mut custom = BTreeMap::new();
        assign_on(&mut custom, Action::Search, &keys("Ctrl+KeyJ"), false, true);
        assert_eq!(custom.get("search").map(String::as_str), Some("Ctrl+KeyJ"));
        assert_eq!(keys_of(&custom, Action::Search, false, true), [keys("Ctrl+KeyJ")]);
        assign_on(&mut custom, Action::Search, &keys("Ctrl+KeyK"), false, true);
        assert!(custom.is_empty());
        assign_on(&mut custom, Action::ZoomIn, &keys("Ctrl+Shift+Equal"), false, true);
        assert!(custom.is_empty());
        // Unreadable keys from another version count as the standard ones.
        custom.insert("bold".to_owned(), "Hyper+KeyB".to_owned());
        assert_eq!(keys_of(&custom, Action::Bold, false, true), [keys("Ctrl+KeyB")]);
    }
}

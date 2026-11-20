//! Mostik preferences modelled on Slack's Preferences. Stored in `localStorage`,
//! some of them (notifications, language) are passed to the backend.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorMode {
    Light,
    #[default]
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NotifyMode {
    #[default]
    All,
    DirectAndMentions,
    Nothing,
}

/// Which conversations the sidebar shows: "Filter conversations by" in Slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SidebarFilter {
    /// Hide conversations without new messages in the last 30 days.
    #[default]
    Active,
    Unreads,
    Mentions,
    All,
}

impl SidebarFilter {
    /// The choices in the order the menus list them.
    pub const ALL: [Self; 4] = [Self::Active, Self::Unreads, Self::Mentions, Self::All];
}

/// Conversation order in the sidebar sections: "Sort conversations by" in Slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SidebarSort {
    Alphabetical,
    #[default]
    Recency,
    /// The order the user dragged the chats into; sections never reordered stay by recency.
    Custom,
}

impl SidebarSort {
    /// The choices in the order the menus list them.
    pub const ALL: [Self; 3] = [Self::Alphabetical, Self::Recency, Self::Custom];
}

/// Sidebar themes in the picker's order: Mostik's own first (the first one is the default), then the classic ones.
/// Own colours and names, not copies of Slack themes.
pub const THEMES: [&str; 7] = ["dusk", "moss", "twilight", "midnight", "forest", "ocean", "sunset"];
/// How many of [`THEMES`] are Mostik's own; the rest are the classic ones.
pub const OWN_THEMES: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub color_mode: ColorMode,
    pub theme: String,
    pub notify: NotifyMode,
    pub notify_preview: bool,
    /// Notification sound, one of [`mostik_core::NOTIFICATION_SOUNDS`].
    pub sound: String,
    pub inline_media: bool,
    pub clock24: bool,
    pub mark_read_on_open: bool,
    /// Show who read the user's own messages; the read markers Mostik sends do not depend on it.
    pub show_read_receipts: bool,
    pub enter_sends: bool,
    /// UI zoom in percent, like "Accessibility → Zoom" in Slack.
    pub zoom: u32,
    pub sidebar_filter: SidebarFilter,
    pub sidebar_sort: SidebarSort,
    /// Hide muted conversations from the sidebar (except mentions).
    pub hide_muted: bool,
    /// Check GitHub releases for a new version in the background and show "Update" in the top bar.
    pub auto_update: bool,
    /// The tray icon on Linux and Windows.
    pub tray_icon: TrayIcon,
    /// The user's own keys of shortcuts, by [`crate::keys::Action::id`]; an action without an
    /// entry has its standard keys.
    pub shortcuts: BTreeMap<String, String>,
}

/// The tray icon from "Preferences → Appearance" on Linux and Windows; macOS always shows the white one,
/// coloured by the menu bar (see `badge.rs` in the backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrayIcon {
    /// The white truss on transparency: reads on a dark panel, is lost on a light one.
    White,
    /// The app icon.
    App,
}

impl TrayIcon {
    /// The choices in the order "Preferences" list them.
    pub const ALL: [Self; 2] = [Self::White, Self::App];

    /// Its picture in the UI assets.
    pub fn preview(self) -> &'static str {
        match self {
            Self::White => "tray/white.png",
            Self::App => "tray/app.png",
        }
    }
}

impl Default for TrayIcon {
    /// White on Linux, where panels are mostly dark; the app icon on Windows.
    fn default() -> Self {
        if on_linux() { Self::White } else { Self::App }
    }
}

/// The app runs on Linux.
pub fn on_linux() -> bool {
    user_agent_has("Linux")
}

/// The app runs on macOS.
pub fn on_mac() -> bool {
    user_agent_has("Macintosh")
}

fn user_agent_has(os: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains(os))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::system(),
            color_mode: ColorMode::Dark,
            theme: THEMES[0].to_owned(),
            notify: NotifyMode::All,
            notify_preview: true,
            sound: "marimba".to_owned(),
            inline_media: true,
            clock24: true,
            mark_read_on_open: true,
            show_read_receipts: true,
            enter_sends: true,
            zoom: 100,
            sidebar_filter: SidebarFilter::Active,
            sidebar_sort: SidebarSort::Recency,
            hide_muted: false,
            auto_update: true,
            tray_icon: TrayIcon::default(),
            shortcuts: BTreeMap::new(),
        }
    }
}

/// Zoom steps: as in Slack's drop-down and for Ctrl+= / Ctrl+-.
pub const ZOOM_LEVELS: [u32; 11] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200];

/// The next zoom step up (`up`) or down from `current`.
pub fn zoom_step(current: u32, up: bool) -> u32 {
    if up {
        ZOOM_LEVELS.into_iter().find(|&z| z > current).unwrap_or(200)
    } else {
        ZOOM_LEVELS.into_iter().rev().find(|&z| z < current).unwrap_or(50)
    }
}

const KEY: &str = "mostik.settings";

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = crate::storage::load(KEY).unwrap_or_default();
        // A theme an older version offered falls back to the default.
        if !THEMES.contains(&s.theme.as_str()) {
            s.theme = Self::default().theme;
        }
        // A sound an older version offered and this one does not.
        if !mostik_core::NOTIFICATION_SOUNDS.contains(&s.sound.as_str()) {
            s.sound = Self::default().sound;
        }
        s
    }

    pub fn save(&self) {
        crate::storage::save(KEY, self);
    }

    /// The effective mode, resolving "same as system".
    pub fn dark(&self) -> bool {
        match self.color_mode {
            ColorMode::Dark => true,
            ColorMode::Light => false,
            ColorMode::System => web_sys::window()
                .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
                .is_none_or(|m| m.matches()),
        }
    }
}

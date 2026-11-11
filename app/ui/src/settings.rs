//! Mostik preferences modelled on Slack's Preferences. Stored in `localStorage`,
//! some of them (notifications, language) are passed to the backend.

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

/// Conversation order in the sidebar sections: "Sort conversations by" in Slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SidebarSort {
    Alphabetical,
    #[default]
    Recency,
    /// The order the user dragged the chats into; sections never reordered stay by recency.
    Custom,
}

/// Sidebar themes. Own colours and names, not copies of Slack themes.
pub const THEMES: [&str; 5] = ["aubergine", "midnight", "forest", "ocean", "sunset"];
/// Theme colour swatches for the picker: (rail, highlight).
pub const THEME_SWATCHES: [(&str, &str); 5] = [
    ("#3f1745", "#7c3c85"),
    ("#1b1d21", "#1164a3"),
    ("#123829", "#2bac76"),
    ("#0f2b46", "#2d8fd5"),
    ("#4a1d12", "#e8912d"),
];

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
    pub enter_sends: bool,
    /// UI zoom in percent, like "Accessibility → Zoom" in Slack.
    pub zoom: u32,
    pub sidebar_filter: SidebarFilter,
    pub sidebar_sort: SidebarSort,
    /// Hide muted conversations from the sidebar (except mentions).
    pub hide_muted: bool,
    /// Check GitHub releases for a new version in the background and show "Update" in the top bar.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::system(),
            color_mode: ColorMode::Dark,
            theme: THEMES[0].to_owned(),
            notify: NotifyMode::All,
            notify_preview: true,
            sound: "chime".to_owned(),
            inline_media: true,
            clock24: true,
            mark_read_on_open: true,
            enter_sends: true,
            zoom: 100,
            sidebar_filter: SidebarFilter::Active,
            sidebar_sort: SidebarSort::Recency,
            hide_muted: false,
            auto_update: true,
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

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = storage()
            .and_then(|s| s.get_item(KEY).ok().flatten())
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        // A sound an older version offered and this one does not.
        if !mostik_core::NOTIFICATION_SOUNDS.contains(&s.sound.as_str()) {
            s.sound = Self::default().sound;
        }
        s
    }

    pub fn save(&self) {
        if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(self)) {
            let _ = s.set_item(KEY, &v);
        }
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

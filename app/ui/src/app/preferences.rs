//! "Preferences" modelled on Slack's Preferences, with the "Add custom emoji" dialog.

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::{Lang, tr};
use crate::icons::{I, Icon};
use crate::keys::{self, Action};
use crate::modal::Modal;
use crate::settings::{
    ColorMode, NotifyMode, OWN_THEMES, Settings, SidebarFilter, SidebarSort, THEMES, TrayIcon, on_mac,
};
use crate::{bridge, ops};

use super::org::current_org;
use super::state::{Ctx, ctx};

#[derive(Clone, Copy, PartialEq)]
pub enum PrefTab {
    Notifications,
    Appearance,
    Messages,
    Language,
    MarkRead,
    Home,
    Accessibility,
    Advanced,
    Updates,
    Emoji,
}

/// "Preferences" modelled on Slack's Preferences (only what applies to Telemost).
#[component]
pub(super) fn PreferencesDialog() -> impl IntoView {
    let c = ctx();
    let tab = c.pref_tab;
    let tabs = [
        (PrefTab::Notifications, I::Bell, tr().p_notifications),
        (PrefTab::Appearance, I::Star, tr().p_appearance),
        (PrefTab::Messages, I::Chat, tr().p_messages_media),
        (PrefTab::Language, I::Globe, tr().p_language_region),
        (PrefTab::Home, I::Home, tr().p_home),
        (PrefTab::MarkRead, I::Check, tr().p_mark_as_read),
        (PrefTab::Accessibility, I::Users, tr().p_accessibility),
        (PrefTab::Advanced, I::Kebab, tr().p_advanced),
        (PrefTab::Emoji, I::Smile, tr().custom_emoji),
        (PrefTab::Updates, I::Download, tr().p_updates),
    ];
    view! {
        <Modal class="prefs" on_close=Callback::new(move |_| c.prefs.set(false))>
            <header>
                <h2>{tr().preferences}</h2>
                <button class="icon-btn" title=tr().close on:click=move |_| c.prefs.set(false)>
                    <Icon icon=I::X/>
                </button>
            </header>
            <div class="prefs-body">
                <nav class="prefs-nav">
                    {tabs.into_iter().map(|(id, icon, label)| view! {
                        <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>
                            <Icon icon=icon size=16/><span>{label}</span>
                        </button>
                    }).collect_view()}
                </nav>
                <section class="prefs-section">
                    {move || match tab.get() {
                        PrefTab::Notifications => view! { <PrefNotifications/> }.into_any(),
                        PrefTab::Appearance => view! { <PrefAppearance/> }.into_any(),
                        PrefTab::Messages => view! { <PrefMessages/> }.into_any(),
                        PrefTab::Language => view! { <PrefLanguage/> }.into_any(),
                        PrefTab::Home => view! { <PrefHome/> }.into_any(),
                        PrefTab::MarkRead => view! { <PrefMarkRead/> }.into_any(),
                        PrefTab::Accessibility => view! { <PrefAccessibility/> }.into_any(),
                        PrefTab::Advanced => view! { <PrefAdvanced/> }.into_any(),
                        PrefTab::Updates => view! { <PrefUpdates/> }.into_any(),
                        PrefTab::Emoji => view! { <PrefEmoji/> }.into_any(),
                    }}
                </section>
            </div>
        </Modal>
    }
}

/// Radio button bound to a preferences field.
fn radio<T: PartialEq + Copy + Send + Sync + 'static>(
    c: Ctx,
    name: &'static str,
    label: impl Into<String>,
    value: T,
    get: fn(&Settings) -> T,
    set: fn(&mut Settings, T),
) -> impl IntoView {
    view! {
        <label class="pref-radio">
            <input type="radio" name=name prop:checked=move || c.settings.with(|s| get(s) == value)
                on:change=move |_| c.settings.update(|s| set(s, value))/>
            <span>{label.into()}</span>
        </label>
    }
}

fn checkbox(c: Ctx, label: &'static str, get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) -> impl IntoView {
    view! {
        <label class="pref-check">
            <input type="checkbox" prop:checked=move || c.settings.with(get)
                on:change=move |e| { let v = event_target_checked(&e); c.settings.update(|s| set(s, v)) }/>
            <span>{label}</span>
        </label>
    }
}

#[component]
fn PrefNotifications() -> impl IntoView {
    let c = ctx();
    let example = move |_| {
        if !c.live {
            return c.error.set(Some(tr().only_in_app_notifications.into()));
        }
        ops::play_sound(c, c.settings.with_untracked(|s| s.sound.clone()));
        spawn_local(async move {
            if let Err(e) = bridge::test_notification().await {
                c.error.set(Some(e));
            }
        });
    };
    let n = |s: &Settings| s.notify;
    let set_n = |s: &mut Settings, v| s.notify = v;
    view! {
        <h3>{tr().p_notify_about}</h3>
        {radio(c, "notify", tr().p_notify_all, NotifyMode::All, n, set_n)}
        {radio(c, "notify", tr().p_notify_direct, NotifyMode::DirectAndMentions, n, set_n)}
        {radio(c, "notify", tr().p_notify_nothing, NotifyMode::Nothing, n, set_n)}
        <div class="pref-gap"></div>
        {checkbox(c, tr().p_notify_preview, |s| s.notify_preview, |s, v| s.notify_preview = v)}
        <h3 class="pref-h">{tr().p_sound}</h3>
        <select class="pref-select" on:change=move |e| {
            let v = event_target_value(&e);
            ops::play_sound(c, v.clone());
            c.settings.update(|s| s.sound = v);
        }>
            {mostik_core::NOTIFICATION_SOUNDS.into_iter().map(|id| view! {
                <option value=id selected=move || c.settings.with(|s| s.sound == id)>{tr().sound_name(id)}</option>
            }).collect_view()}
        </select>
        <p class="pref-hint">{tr().p_notify_explain}</p>
        <button class="link" on:click=example>{tr().p_show_example}</button>
    }
}

#[component]
fn PrefAppearance() -> impl IntoView {
    let c = ctx();
    let m = |s: &Settings| s.color_mode;
    let set_m = |s: &mut Settings, v| s.color_mode = v;
    view! {
        <h3>{tr().p_color_mode}</h3>
        <div class="pref-row">
            {radio(c, "mode", tr().p_mode_light, ColorMode::Light, m, set_m)}
            {radio(c, "mode", tr().p_mode_dark, ColorMode::Dark, m, set_m)}
            {radio(c, "mode", tr().p_mode_system, ColorMode::System, m, set_m)}
        </div>
        <h3 class="pref-h">{tr().p_themes}</h3>
        <div class="theme-group">{tr().themes_own}</div>
        {theme_buttons(c, 0..OWN_THEMES)}
        <div class="theme-group">{tr().themes_classic}</div>
        {theme_buttons(c, OWN_THEMES..THEMES.len())}
        // The tray icon does not follow the mode or the theme: the panel's colour is the
        // system's. macOS has no choice: the menu bar colours the white icon for itself.
        {(!on_mac()).then(|| view! {
            <h3 class="pref-h">{tr().p_tray}</h3>
            <div class="themes">
                {TrayIcon::ALL.into_iter().enumerate().map(|(i, icon)| view! {
                    <button class=move || if c.settings.with(|s| s.tray_icon == icon) { "theme selected" } else { "theme" }
                        on:click=move |_| c.settings.update(|s| s.tray_icon = icon)>
                        <img class="tray-preview" class:white={icon == TrayIcon::White} src=icon.preview() alt=""/>
                        <span>{tr().tray_icon_names[i]}</span>
                    </button>
                }).collect_view()}
            </div>
        })}
    }
}

/// Buttons of the themes `range` of [`THEMES`]: a swatch and the name.
fn theme_buttons(c: Ctx, range: std::ops::Range<usize>) -> impl IntoView {
    view! {
        <div class="themes">
            {range.map(|i| {
                let id = THEMES[i];
                view! {
                    <button class=move || if c.settings.with(|s| s.theme == id) { "theme selected" } else { "theme" }
                        on:click=move |_| c.settings.update(|s| s.theme = id.to_owned())>
                        <span class="swatch" data-theme=id></span>
                        <span>{tr().theme_names[i]}</span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn PrefMessages() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_messages_media}</h3>
        {checkbox(c, tr().p_inline_media, |s| s.inline_media, |s, v| s.inline_media = v)}
        <p class="pref-hint">{tr().p_inline_media_hint}</p>
        <h3 class="pref-h">{tr().p_clock}</h3>
        {checkbox(c, tr().p_clock24, |s| s.clock24, |s, v| s.clock24 = v)}
    }
}

/// "Accessibility": UI zoom, like the Accessibility → Zoom section in Slack.
#[component]
fn PrefAccessibility() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_zoom}</h3>
        <select class="pref-select" on:change=move |e| {
            if let Ok(z) = event_target_value(&e).parse::<u32>() {
                c.settings.update(|s| s.zoom = z);
            }
        }>
            {crate::settings::ZOOM_LEVELS.into_iter().map(|z| view! {
                <option value=z.to_string() selected=move || c.settings.with(|s| s.zoom == z)>{format!("{z}%")}</option>
            }).collect_view()}
        </select>
        <p class="pref-hint">{move || c.settings.with(|s| {
            let key = |action| keys::shown(&s.shortcuts, action);
            tr().zoom_hint(&key(Action::ZoomIn), &key(Action::ZoomOut), &key(Action::ZoomReset))
        })}</p>
    }
}

#[component]
fn PrefLanguage() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_language}</h3>
        <select class="pref-select" on:change=move |e| {
            let lang = if event_target_value(&e) == "en" { Lang::En } else { Lang::Ru };
            c.settings.update(|s| s.lang = lang);
        }>
            <option value="ru" selected=move || c.settings.with(|s| s.lang == Lang::Ru)>"Русский"</option>
            <option value="en" selected=move || c.settings.with(|s| s.lang == Lang::En)>"English"</option>
        </select>
        <p class="pref-hint">{tr().p_language_hint}</p>
    }
}

/// "Home", like Preferences → Home in Slack: filter, sorting and muted conversations.
#[component]
fn PrefHome() -> impl IntoView {
    let c = ctx();
    let gf = |s: &Settings| s.sidebar_filter;
    let sf = |s: &mut Settings, v| s.sidebar_filter = v;
    let gs = |s: &Settings| s.sidebar_sort;
    let ss = |s: &mut Settings, v| s.sidebar_sort = v;
    view! {
        <h3>{tr().filter_by}</h3>
        {SidebarFilter::ALL.into_iter().map(|f| view! {
            {radio(c, "sb-filter", tr().sidebar_filter(f), f, gf, sf)}
            {(f == SidebarFilter::Active).then(|| view! { <p class="pref-hint">{tr().p_active_hint}</p> })}
        }).collect_view()}
        <h3 class="pref-h">{tr().sort_by}</h3>
        {SidebarSort::ALL.into_iter().map(|o| radio(c, "sb-sort", tr().sidebar_sort(o), o, gs, ss)).collect_view()}
        <h3 class="pref-h">{tr().p_muted}</h3>
        {checkbox(c, tr().hide_muted, |s| s.hide_muted, |s, v| s.hide_muted = v)}
    }
}

#[component]
fn PrefMarkRead() -> impl IntoView {
    let c = ctx();
    let g = |s: &Settings| s.mark_read_on_open;
    let set = |s: &mut Settings, v| s.mark_read_on_open = v;
    view! {
        <h3>{tr().p_when_view}</h3>
        {radio(c, "read", tr().p_read_on_open, true, g, set)}
        {radio(c, "read", tr().p_leave_unread, false, g, set)}
        <h3 class="pref-h">{tr().p_read_receipts}</h3>
        {checkbox(c, tr().p_show_read_receipts, |s| s.show_read_receipts, |s, v| s.show_read_receipts = v)}
    }
}

/// "Updates": the background check switch, the installed version, a manual check and the install.
#[component]
fn PrefUpdates() -> impl IntoView {
    let c = ctx();
    view! {
        <h3>{tr().p_updates}</h3>
        {checkbox(c, tr().p_auto_update, |s| s.auto_update, |s, v| s.auto_update = v)}
        <p class="pref-hint">{tr().p_auto_update_hint}</p>
        <h3 class="pref-h">{tr().version(env!("CARGO_PKG_VERSION"))}</h3>
        <UpdateActions/>
    }
}

/// The update status with what can be done about it: "Check for updates", "Update" with the
/// install progress. In Preferences → Updates and on the login screen (`login`), where it lets a
/// user whose sign-in broke get a fixed version (#47); there a build that cannot update itself
/// shows nothing. A manual check works with the background checks turned off.
#[component]
pub(super) fn UpdateActions(#[prop(optional)] login: bool) -> impl IntoView {
    let c = ctx();
    // Result of the last manual check: `Some(Ok)` is "up to date", `Some(Err)` the error text.
    let checked: RwSignal<Option<Result<(), String>>> = RwSignal::new(None);
    let checking = RwSignal::new(false);
    let check = move |_| {
        checking.set(true);
        checked.set(None);
        spawn_local(async move {
            let result = bridge::check_update().await;
            checking.try_set(false);
            checked.try_set(Some(result.map(|status| c.update.set(status))));
        });
    };
    move || {
        let status = if c.live {
            c.update.get()
        } else {
            bridge::UpdateStatus::Unsupported
        };
        match status {
            bridge::UpdateStatus::Unsupported if login => ().into_any(),
            bridge::UpdateStatus::Unsupported => view! { <p class="pref-hint flush">{tr().update_unsupported}</p> }.into_any(),
            bridge::UpdateStatus::Available { version } => view! {
                <p>{tr().update_available(&version)}</p>
                <div class="pref-actions">
                    <button class="btn primary small" on:click=move |_| ops::install_update(c)>{tr().update_button}</button>
                </div>
            }
            .into_any(),
            bridge::UpdateStatus::Installing { percent, .. } => view! {
                <div class="pref-actions">
                    <button class="btn primary small" disabled=true>{tr().updating(percent)}</button>
                </div>
            }
            .into_any(),
            bridge::UpdateStatus::None => view! {
                {move || match checked.get() {
                    Some(Ok(())) => Some(view! { <p>{tr().up_to_date}</p> }.into_any()),
                    Some(Err(e)) => Some(view! { <p class="pref-error">{e}</p> }.into_any()),
                    None => None,
                }}
                <div class="pref-actions">
                    <button class="btn small" disabled=move || checking.get() on:click=check>
                        {move || if checking.get() { tr().checking_updates } else { tr().check_updates }}
                    </button>
                </div>
            }
            .into_any(),
        }
    }
}

/// Custom emoji of the organization, as in Slack's "Customize workspace": "Add emoji" opens the
/// same dialog as the emoji picker; your own can be deleted.
#[component]
fn PrefEmoji() -> impl IntoView {
    let c = ctx();
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    // The emoji to confirm deleting (its message time).
    let deleting: RwSignal<Option<Timestamp>> = RwSignal::new(None);
    // Runs once Telemost answers, maybe after the preferences closed.
    let done = move |r: Result<Vec<mostik_core::custom_emoji::EmojiSet>, String>| match r {
        Ok(sets) => {
            c.custom.set(sets);
            deleting.try_set(None);
        }
        Err(e) => {
            error.try_set(Some(e));
        }
    };
    let row =
        move |e: mostik_core::custom_emoji::CustomEmoji, chat: ChatId, taken: bool| {
            let (me, author) = c.snap.with_untracked(|s| {
                let author = s
                    .user(&e.owner)
                    .filter(|u| u.has_name())
                    .map_or_else(|| tr().unknown_user.to_owned(), |u| u.display_name.clone());
                (s.me.clone(), author)
            });
            let mine = e.owner == me;
            let ts = e.ts;
            let code = mostik_core::custom_emoji::caption(&e.name);
            let remove = move |_| {
                let chat = chat.clone();
                spawn_local(async move { done(bridge::delete_custom_emoji(&chat.0, ts).await) });
            };
            let actions = mine.then(|| view! {
            <span class="emoji-actions">
                {move || if deleting.get() == Some(ts) {
                    let remove = remove.clone();
                    view! {
                        <button class="btn small" on:click=move |_| deleting.set(None)>{tr().cancel}</button>
                        <button class="btn danger small" on:click=remove>{tr().delete}</button>
                    }.into_any()
                } else {
                    view! {
                        <button class="btn small" on:click=move |_| deleting.set(Some(ts))>{tr().delete}</button>
                    }.into_any()
                }}
            </span>
        });
            view! {
                <div class="emoji-row">
                    {crate::custom::image(&e, "emoji-preview")}
                    <span class="emoji-label">
                        <span class="emoji-code">{code}</span>
                        <span class="emoji-author">{format!("{} {author}", tr().emoji_author)}</span>
                        {taken.then(|| view! { <span class="emoji-taken">{tr().emoji_taken}</span> })}
                    </span>
                    {actions}
                </div>
            }
        };
    let list = move || {
        let me = c.snap.with(|s| s.me.clone());
        match crate::custom::current(c) {
            Some(set) if !set.emoji.is_empty() || set.taken.iter().any(|e| e.owner == me) => {
                let chat = set.chat_id.clone();
                let taken: Vec<_> = set.taken.into_iter().filter(|e| e.owner == me).collect();
                // Your own first, newest on top; then the others', newest on top.
                let mut emoji = set.emoji;
                emoji.sort_by_key(|e| (e.owner != me, std::cmp::Reverse(e.ts)));
                view! {
                    <div class="emoji-list">
                        {taken.into_iter().map(|e| row(e, chat.clone(), true)).collect_view()}
                        {emoji.into_iter().map(|e| row(e, chat.clone(), false)).collect_view()}
                    </div>
                }
                .into_any()
            }
            _ => view! { <p class="pref-hint flush">{tr().no_custom_emoji}</p> }.into_any(),
        }
    };
    view! {
        <h3>{tr().custom_emoji}</h3>
        <p class="pref-hint flush">{tr().custom_emoji_hint}</p>
        {(!c.live).then(|| view! { <p class="pref-error">{tr().emoji_offline}</p> })}
        <div class="pref-actions">
            <button class="btn primary small" disabled=!c.live on:click=move |_| c.adding_emoji.set(true)>{tr().add_emoji}</button>
        </div>
        {move || error.get().map(|e| view! { <p class="pref-error">{e}</p> })}
        {list}
    }
}

/// "Add custom emoji", as in Slack: opened by "Add emoji" at the bottom of the emoji picker.
/// The emoji goes to the organization's service chat and shows up in the picker's custom section.
#[component]
pub(super) fn AddEmojiDialog() -> impl IntoView {
    let c = ctx();
    let file: RwSignal<Option<bridge::PendingFile>> = RwSignal::new(None);
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let close = move || c.adding_emoji.set(false);
    let pick = move |_| {
        spawn_local(async move {
            // The dialog may be closed while the file dialog is open: its signals are gone then.
            match bridge::pick_emoji_image().await {
                Ok(Some(f)) => {
                    if name.try_with_untracked(|n| n.trim().is_empty()) == Some(true) {
                        name.try_set(mostik_core::custom_emoji::name_from_file(&f.name));
                    }
                    file.try_set(Some(f));
                    error.try_set(None);
                }
                Ok(None) => {}
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    let save = move || {
        if !c.live {
            return error.set(Some(tr().emoji_offline.into()));
        }
        let Some(f) = file.get_untracked() else { return };
        let n = name.get_untracked().trim().trim_matches(':').to_owned();
        let org = current_org(c).map(|o| o.id);
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match bridge::add_custom_emoji(&f.path, &n, org).await {
                Ok(sets) => {
                    c.custom.set(sets);
                    close();
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    };
    let ready =
        move || !busy.get() && file.with(Option::is_some) && name.with(|n| !n.trim().trim_matches(':').is_empty());
    view! {
        <Modal class="add-emoji" on_close=Callback::new(move |_| close())>
            <header>
                <h2>{tr().add_emoji_title}</h2>
                <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
            </header>
            <p class="muted">{tr().custom_emoji_hint}</p>
            <label class="field-label">{tr().emoji_upload_step}</label>
            <div class="add-emoji-upload">
                <div class="add-emoji-preview">
                    {move || file.get().map(|f| view! { <img src=bridge::local_file_url(&f.path) alt=""/> })}
                </div>
                <div>
                    <p class="muted">{tr().emoji_image_hint}</p>
                    <button class="btn small" disabled=move || busy.get() on:click=pick>
                        {move || if file.with(Option::is_some) { tr().emoji_change_image } else { tr().emoji_choose_image }}
                    </button>
                </div>
            </div>
            <label class="field-label">{tr().emoji_name_step}</label>
            <div class="name-input"><span>":"</span>
                <input class="text-input bare" placeholder=tr().emoji_name_ph maxlength="40" prop:value=move || name.get()
                    on:input=move |e| name.set(event_target_value(&e))
                    on:keydown=move |e: ev::KeyboardEvent| if e.key() == "Enter" && ready() { e.prevent_default(); save() }/>
                <span>":"</span>
            </div>
            <p class="muted">{tr().emoji_name_hint}</p>
            {move || error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
            <div class="modal-actions">
                <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                <button class="btn primary" disabled=move || !ready() on:click=move |_| save()>
                    {move || if busy.get() { tr().emoji_adding } else { tr().save }}
                </button>
            </div>
        </Modal>
    }
}

/// "Advanced": what Enter does in a message field, and the keyboard shortcuts of this platform,
/// Cmd in place of Ctrl on macOS. The keys that send follow the Enter choice above them; those of
/// the app's own actions can be changed.
#[component]
fn PrefAdvanced() -> impl IntoView {
    let c = ctx();
    let g = |s: &Settings| s.enter_sends;
    let set = |s: &mut Settings, v| s.enter_sends = v;
    // The action whose new keys are awaited, and why the last ones were not taken.
    let recording: RwSignal<Option<Action>> = RwSignal::new(None);
    let refused: RwSignal<Option<(Action, String)>> = RwSignal::new(None);
    let general = move || {
        let mut rows = vec![
            Row::Action(Action::Search),
            Row::Action(Action::Back),
            Row::Action(Action::Forward),
            Row::Action(Action::Preferences),
            Row::Fixed(tr().sc_close, "Esc".to_owned()),
            Row::Action(Action::ZoomIn),
            Row::Action(Action::ZoomOut),
            Row::Action(Action::ZoomReset),
        ];
        // The system's own, from the app menu of macOS.
        if on_mac() {
            rows.push(Row::Fixed(tr().sc_hide_window, keys::label("Ctrl+W")));
            rows.push(Row::Fixed(tr().sc_quit, keys::label("Ctrl+Q")));
        }
        rows
    };
    let field = move || {
        let (send, newline) = keys::send_and_newline(c.settings.with(|s| s.enter_sends));
        vec![
            Row::Fixed(tr().sc_send, send),
            Row::Fixed(tr().sc_newline, newline),
            Row::Fixed(tr().sc_edit_last, "↑".to_owned()),
            Row::Action(Action::Bold),
            Row::Action(Action::Italic),
            Row::Action(Action::Strike),
            Row::Action(Action::Link),
        ]
    };
    let any_own = move || c.settings.with(|s| !s.shortcuts.is_empty());
    view! {
        <h3>{tr().p_enter_to}</h3>
        {radio(c, "enter", tr().p_enter_send, true, g, set)}
        {radio(c, "enter", tr().enter_newline(&keys::label("Ctrl+Enter")), false, g, set)}
        <h3 class="pref-h">{tr().p_shortcuts}</h3>
        <p class="pref-hint flush">{tr().sc_change_hint}</p>
        <div class="shortcut-group">{tr().sc_general}</div>
        {shortcut_list(general(), recording, refused)}
        <div class="shortcut-group">{tr().sc_message_field}</div>
        {move || shortcut_list(field(), recording, refused)}
        {move || any_own().then(|| view! {
            <button class="link" on:click=move |_| {
                recording.set(None);
                refused.set(None);
                c.settings.update(|s| s.shortcuts.clear());
            }>{tr().sc_reset_all}</button>
        })}
    }
}

/// A row of the shortcut list: an action whose keys can be changed, or fixed keys.
enum Row {
    Action(Action),
    Fixed(&'static str, String),
}

/// Actions with their keys, one per row.
fn shortcut_list(
    rows: Vec<Row>,
    recording: RwSignal<Option<Action>>,
    refused: RwSignal<Option<(Action, String)>>,
) -> impl IntoView {
    view! {
        <dl class="shortcuts">
            {rows.into_iter().map(|row| match row {
                Row::Action(action) => view! {
                    <dt>{action.name()}</dt>
                    <dd><ShortcutKeys action recording refused/></dd>
                }.into_any(),
                Row::Fixed(name, keys) => view! {
                    <dt>{name}</dt>
                    <dd><kbd class="keycap">{keys}</kbd></dd>
                }.into_any(),
            }).collect_view()}
        </dl>
    }
}

/// The keys of an action: a click waits for new keys, Esc or a click elsewhere cancels; keys taken
/// by another action or the system are refused with the reason. "Restore default" goes back to the
/// standard keys.
#[component]
fn ShortcutKeys(
    action: Action,
    recording: RwSignal<Option<Action>>,
    refused: RwSignal<Option<(Action, String)>>,
) -> impl IntoView {
    let c = ctx();
    let button = NodeRef::<leptos::html::Button>::new();
    let waiting = move || recording.get() == Some(action);
    let own = move || c.settings.with(|s| s.shortcuts.contains_key(action.id()));
    // Also on `blur`, which a button focused when "Preferences" closes gets after the list is
    // disposed: the signals may be gone then (as in #61).
    let stop = move || {
        if recording.try_get_untracked() == Some(Some(action)) {
            recording.try_set(None);
        }
    };
    // The keys go to this button only: the app's own shortcuts and Esc closing "Preferences" are
    // listened to on the window, which they do not reach.
    let on_key = move |e: ev::KeyboardEvent| {
        if recording.get_untracked() != Some(action) {
            return;
        }
        e.prevent_default();
        e.stop_propagation();
        let plain = !(e.ctrl_key() || e.alt_key() || e.shift_key() || e.meta_key());
        if e.key() == "Escape" && plain {
            refused.set(None);
            return stop();
        }
        let Some(new) = keys::Shortcut::of(&e) else {
            return;
        };
        match c.settings.with_untracked(|s| keys::check(&new, action, &s.shortcuts)) {
            Ok(()) => {
                refused.set(None);
                recording.set(None);
                c.settings.update(|s| keys::assign(&mut s.shortcuts, action, &new));
            }
            Err(why) => refused.set(Some((action, why.text()))),
        }
    };
    view! {
        <button class="keycap edit" class:waiting=waiting node_ref=button
            title=move || if waiting() { tr().sc_press_title } else { tr().sc_change }
            on:click=move |_| {
                refused.set(None);
                recording.set(Some(action));
                // A click does not focus a button in WebKit on macOS.
                if let Some(el) = button.get_untracked() {
                    let _ = el.focus();
                }
            }
            on:keydown=on_key
            on:blur=move |_| stop()>
            {move || if waiting() {
                tr().sc_press.to_owned()
            } else {
                c.settings.with(|s| keys::shown(&s.shortcuts, action))
            }}
        </button>
        {move || own().then(|| view! {
            <button class="icon-btn shortcut-reset" title=tr().sc_reset on:click=move |_| {
                refused.set(None);
                c.settings.update(|s| {
                    s.shortcuts.remove(action.id());
                });
            }><Icon icon=I::Reply size=14/></button>
        })}
        {move || refused.get().filter(|(a, _)| *a == action).map(|(_, why)| view! {
            <div class="shortcut-refused">{why}</div>
        })}
    }
}

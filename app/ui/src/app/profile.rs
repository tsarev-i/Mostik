//! The user and other people: the avatar menu in the rail, the status dialog and the profile pane.

use leptos::prelude::*;
use leptos::task::spawn_local;
use mostik_core::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::modal::Modal;
use crate::{bridge, ops, time};

use super::avatar::{Avatar, availability_label, presence_class, status_icon, status_text};
use super::org::org_name;
use super::state::ctx;

/// Avatar menu in the rail, as in Slack: name and status, "Update your status", away / active,
/// pausing notifications for a while, profile, preferences, sign out.
#[component]
pub(super) fn MeMenu() -> impl IntoView {
    let c = ctx();
    let me = Memo::new(move |_| c.snap.with(|s| s.user(&s.me).cloned()));
    let pause_open = RwSignal::new(false);
    let close = move || c.me_menu.set(false);
    let set = move |availability: Availability, duration: Option<u64>| {
        close();
        let status = me.get_untracked().and_then(|u| u.status);
        ops::set_status(c, availability, status, duration);
    };
    // Paused here or on another device; an ended pause is cleared every second.
    let paused = Memo::new(move |_| c.snap.with(|s| s.paused));
    let pauses = move || {
        [
            (30 * 60, tr().pause_30m),
            (60 * 60, tr().pause_1h),
            (2 * 60 * 60, tr().pause_2h),
            (seconds_until_tomorrow_morning(), tr().pause_tomorrow),
        ]
    };
    view! {
        <div class="me-menu" on:mouseleave=move |_| close()>
            {move || me.get().map(|u| {
                let line = match (paused.get().and_then(|p| p.until), u.status.clone()) {
                    (Some(until), _) => format!("{} {}", tr().paused_until, time::hm(until)),
                    (_, Some(st)) if st.in_meeting => status_text(&st),
                    (_, Some(st)) => format!("{} {}", st.emoji, st.text).trim().to_owned(),
                    _ => availability_label(&u).to_owned(),
                };
                view! {
                    <div class="me-head">
                        <Avatar user=u.clone() size=36/>
                        <div>
                            <div class="me-name">{u.display_name.clone()}</div>
                            <div class="me-status"><span class=presence_class(&u)></span>{line}</div>
                        </div>
                    </div>
                }
            })}
            <button class="me-status-btn" on:click=move |_| { close(); c.status_dialog.set(true); }>
                <Icon icon=I::Smile size=16/><span>{tr().update_status}</span>
            </button>
            <div class="me-sep"></div>
            {move || {
                let away = me.get().is_some_and(|u| u.availability == Availability::Away);
                let dnd = paused.get().is_some() || me.get().is_some_and(|u| u.availability == Availability::DoNotDisturb);
                view! {
                    <button on:click=move |_| set(if away { Availability::Default } else { Availability::Away }, None)>
                        {if away { tr().set_active } else { tr().set_away }}
                    </button>
                    {if dnd {
                        view! { <button on:click=move |_| set(Availability::Default, None)>{tr().resume_notifications}</button> }.into_any()
                    } else {
                        view! {
                            <div class="me-sub" on:mouseenter=move |_| pause_open.set(true) on:mouseleave=move |_| pause_open.set(false)>
                                <button class="me-sub-btn" on:click=move |_| pause_open.set(true)>
                                    <span>{tr().pause_for}</span><Icon icon=I::ChevronRight size=14/>
                                </button>
                                <Show when=move || pause_open.get()>
                                    <div class="me-submenu">
                                        {pauses().into_iter().map(|(secs, label)| view! {
                                            <button on:click=move |_| set(Availability::DoNotDisturb, Some(secs))>{label}</button>
                                        }).collect_view()}
                                    </div>
                                </Show>
                            </div>
                        }.into_any()
                    }}
                }
            }}
            <div class="me-sep"></div>
            <button on:click=move |_| {
                close();
                let me = c.snap.with_untracked(|s| s.me.clone());
                ops::open_profile(c, me);
            }>{tr().profile}</button>
            <button on:click=move |_| { close(); c.prefs.set(true); }>{tr().preferences}</button>
            {c.live.then(|| view! {
                <div class="me-sep"></div>
                <button on:click=move |_| {
                    close();
                    ops::sign_out(c);
                }>{move || tr().sign_out_of(&org_name(c))}</button>
            })}
        </div>
    }
}

/// Until 9:00 the next day: "Until tomorrow" in the notification pause, as in Slack.
fn seconds_until_tomorrow_morning() -> u64 {
    seconds_until_midnight() + 9 * 3600
}

/// "Update your status", as in Slack: emoji, text, presets and when to clear it.
#[component]
pub(super) fn StatusDialog() -> impl IntoView {
    let c = ctx();
    let current = c.snap.with_untracked(|s| s.user(&s.me).and_then(|u| u.status.clone()));
    let availability = c
        .snap
        .with_untracked(|s| s.user(&s.me).map(|u| u.availability).unwrap_or_default());
    let emoji = RwSignal::new(current.as_ref().map(|s| s.emoji.clone()).unwrap_or_default());
    let text = RwSignal::new(current.as_ref().map(|s| s.text.clone()).unwrap_or_default());
    let clear = RwSignal::new(0u64);
    let close = move || c.status_dialog.set(false);
    let save = move || {
        let (e, t) = (emoji.get_untracked(), text.get_untracked());
        let status = (!e.trim().is_empty() || !t.trim().is_empty()).then(|| CustomStatus {
            emoji: e.trim().to_owned(),
            text: t.trim().to_owned(),
            in_meeting: false,
        });
        let d = clear.get_untracked();
        ops::set_status(c, availability, status, (d > 0).then_some(d));
        close();
    };
    let clear_options: [(u64, &'static str); 5] = [
        (0, tr().clear_never),
        (30 * 60, tr().clear_30m),
        (60 * 60, tr().clear_1h),
        (4 * 60 * 60, tr().clear_4h),
        (seconds_until_midnight(), tr().clear_today),
    ];
    let emoji_at: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);
    view! {
        <Modal on_close=Callback::new(move |_| close())>
            <header>
                <h2>{tr().update_status}</h2>
                <button class="icon-btn" title=tr().close on:click=move |_| close()><Icon icon=I::X/></button>
            </header>
            <div class="status-input">
                // The emoji is picked, as in Slack: a text field would need the OS emoji input.
                <button class="emoji-input" title=tr().section_emoji
                    on:click=move |e| emoji_at.set(Some(crate::emoji::anchor_of(&e)))>
                    {move || match emoji.get() {
                        e if e.is_empty() => view! { <Icon icon=I::Smile size=20/> }.into_any(),
                        e => view! { <span>{e}</span> }.into_any(),
                    }}
                </button>
                <input class="text-input bare" maxlength="100" placeholder=tr().status_placeholder prop:value=move || text.get()
                    on:input=move |e| text.set(event_target_value(&e))/>
                <Show when=move || !emoji.get().is_empty() || !text.get().is_empty()>
                    <button class="icon-btn" title=tr().clear_status on:click=move |_| { emoji.set(String::new()); text.set(String::new()); }>
                        <Icon icon=I::X size=14/>
                    </button>
                </Show>
            </div>
            {move || emoji_at.get().map(|at| view! {
                <crate::emoji::EmojiPicker source=crate::emoji::Source::Text anchor=(at.0 + 312.0, at.1)
                    on_close=Callback::new(move |_| emoji_at.set(None))
                    on_pick=Callback::new(move |glyph: String| {
                        emoji_at.set(None);
                        crate::emoji::remember(crate::emoji::Source::Text, &glyph);
                        emoji.set(glyph);
                    })/>
            })}
            <div class="field-label">{tr().for_your_org}</div>
            <div class="status-presets">
                {tr().status_presets.into_iter().map(|(e, t)| view! {
                    <button class="status-preset" on:click=move |_| { emoji.set(e.to_owned()); text.set(t.to_owned()); }>
                        <span class="preset-emoji">{e}</span><span>{t}</span>
                    </button>
                }).collect_view()}
            </div>
            <label class="field-label">{tr().clear_after}</label>
            <select class="pref-select" on:change=move |e| clear.set(event_target_value(&e).parse().unwrap_or(0))>
                {clear_options.into_iter().map(|(secs, label)| view! { <option value=secs.to_string()>{label}</option> }).collect_view()}
            </select>
            <div class="modal-actions">
                <button class="btn" on:click=move |_| close()>{tr().cancel}</button>
                <button class="btn primary" on:click=move |_| save()>{tr().save}</button>
            </div>
        </Modal>
    }
}

fn seconds_until_midnight() -> u64 {
    let now = js_sys::Date::new_0();
    let left = (23 - now.get_hours()) * 3600 + (59 - now.get_minutes()) * 60 + (60 - now.get_seconds());
    u64::from(left.max(60))
}

/// "Profile" panel, as in Slack: a large photo, name, title, buttons, contacts.
#[component]
pub(super) fn ProfilePane() -> impl IntoView {
    let c = ctx();
    let data = Memo::new(move |_| {
        let id = c.profile.get()?;
        let user = c.snap.with(|s| s.user(&id).cloned());
        let me = c.snap.with(|s| s.me == id);
        let profile = c.profiles.with(|p| p.get(&id).cloned());
        Some((id, user, profile, me))
    });
    view! {
        <aside class="thread-pane">
            <header class="tp-header">
                <div class="tp-title">{tr().profile}</div>
                <button class="icon-btn" title=tr().close on:click=move |_| c.profile.set(None)><Icon icon=I::X/></button>
            </header>
            <div class="tp-scroll">
                {move || data.get().map(|(id, user, profile, me)| {
                    let name = profile
                        .as_ref()
                        .map(|p| p.display_name.clone())
                        .filter(|n| !n.is_empty())
                        .or_else(|| user.as_ref().map(|u| u.display_name.clone()))
                        .unwrap_or_else(|| tr().user.to_owned());
                    let base = user.clone().unwrap_or_else(|| User::named(id.clone(), name.clone()));
                    let face = User { avatar: base.avatar.clone().or_else(|| profile.as_ref().and_then(|p| p.avatar.clone())), ..base };
                    let title = profile.as_ref().and_then(|p| p.position.clone()).or_else(|| user.as_ref().and_then(|u| u.title.clone()));
                    let department = profile.as_ref().and_then(|p| p.department.clone());
                    let organization = profile.as_ref().and_then(|p| p.organization.clone());
                    let email = profile.as_ref().and_then(|p| p.email.clone());
                    let nickname = profile.as_ref().and_then(|p| p.nickname.clone());
                    let msg_id = id.clone();
                    let call_id = id.clone();
                    view! {
                        <div class="profile">
                            <div class="profile-photo"><Avatar user=face size=300/></div>
                            <div class="profile-name-row">
                                <h2 class="profile-name">{name}</h2>
                                {(me && c.live).then(|| view! {
                                    <button class="link" on:click=move |_| spawn_local(async move {
                                        if let Err(e) = bridge::open_yandex_id().await { c.error.set(Some(e)); }
                                    })>{tr().edit_profile}</button>
                                })}
                            </div>
                            {title.map(|t| view! { <div class="profile-title">{t}</div> })}
                            {user.as_ref().and_then(|u| u.status.clone()).map(|st| view! {
                                <div class="profile-line"><span class="preset-emoji">{status_icon(&st, 18)}</span>{status_text(&st)}</div>
                            })}
                            {user.clone().map(|u| view! {
                                <div class="profile-line"><span class=presence_class(&u)></span>{availability_label(&u)}</div>
                            })}
                            {me.then(|| view! {
                                <div class="profile-line"><Icon icon=I::Clock size=16/>
                                    {move || format!("{} {}", time::hm((c.now.get() * 1000.0) as Timestamp), tr().local_time)}
                                </div>
                            })}
                            {(!me).then(|| view! {
                                <div class="profile-actions">
                                    <button class="btn" on:click=move |_| ops::message_user(c, msg_id.clone())>
                                        <Icon icon=I::Chat size=16/><span>{tr().message_button}</span>
                                    </button>
                                    <button class="btn" on:click={let id = call_id.clone(); move |_| ops::call_user(c, id.clone())}>
                                        <Icon icon=I::Video size=16/><span>{tr().call}</span>
                                    </button>
                                </div>
                            })}
                        </div>
                        {(email.is_some() || nickname.is_some()).then(|| view! {
                        <div class="profile-block">
                            <h3>{tr().contact_info}</h3>
                            {email.map(|e| view! {
                                <div class="profile-field">
                                    <span class="pf-icon"><Icon icon=I::Mail size=18/></span>
                                    <div><div class="pf-label">{tr().email_address}</div><div class="pf-value link-text">{e}</div></div>
                                </div>
                            })}
                            {nickname.map(|n| view! {
                                <div class="profile-field">
                                    <span class="pf-icon"><Icon icon=I::At size=18/></span>
                                    <div><div class="pf-label">{tr().nickname}</div><div class="pf-value">{n}</div></div>
                                </div>
                            })}
                        </div>
                        })}
                        {(department.is_some() || organization.is_some()).then(|| view! {
                            <div class="profile-block">
                                {organization.map(|o| view! {
                                    <div class="profile-field">
                                        <span class="pf-icon"><Icon icon=I::Users size=18/></span>
                                        <div><div class="pf-label">{tr().organization}</div><div class="pf-value">{o}</div></div>
                                    </div>
                                })}
                                {department.map(|d| view! {
                                    <div class="profile-field">
                                        <span class="pf-icon"><Icon icon=I::Home size=18/></span>
                                        <div><div class="pf-label">{tr().department}</div><div class="pf-value">{d}</div></div>
                                    </div>
                                })}
                            </div>
                        })}
                    }
                })}
            </div>
        </aside>
    }
}

//! The rail on the left, as in Slack: the organization tile, home, DMs, activity and the user's
//! avatar.

use leptos::prelude::*;

use crate::i18n::tr;
use crate::icons::{I, Icon};

use super::avatar::{Avatar, avatar_url, status_icon, status_tip};
use super::org::{org_avatar, org_name};
use super::profile::MeMenu;
use super::state::{CtxMenu, CtxTarget, ViewMode, ctx};

#[component]
pub(super) fn Rail() -> impl IntoView {
    let c = ctx();
    let org = move || org_name(c);
    let initial = move || org_name(c).chars().next().unwrap_or('?').to_string();
    let me = move || c.snap.with(|s| s.user(&s.me).cloned());
    let item = move |icon: I, label: &'static str, mode: ViewMode| {
        view! {
            <button class="rail-item" class:active=move || c.view.get() == mode || (mode == ViewMode::Home && c.view.get().is_home_page())
                on:click=move |_| c.view.set(mode)>
                <span class="rail-icon"><Icon icon=icon size=20/>
                    {(mode == ViewMode::Activity).then_some(move || {
                        let n: u32 = c.snap.with(|s| s.chats.iter().map(|ch| ch.mentions).sum());
                        (n > 0).then(|| view! { <span class="rail-badge">{n}</span> })
                    })}
                </span>
                <span class="rail-label">{label}</span>
            </button>
        }
    };
    view! {
        <nav class="rail">
            // As in Slack: the organization tile opens the organization switcher.
            <button class="ws-tile" title=org
                on:mousedown=|e| e.stop_propagation()
                on:click=move |e| {
                    if c.ctx_menu.with_untracked(|m| m.as_ref().is_some_and(|m| matches!(m.target, CtxTarget::Organizations))) {
                        return c.ctx_menu.set(None);
                    }
                    let Some(el) = e.current_target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()) else { return };
                    let r = el.get_bounding_client_rect();
                    c.ctx_menu.set(Some(CtxMenu { x: r.right() + 8.0, y: r.top(), target: CtxTarget::Organizations }));
                }>
                {initial}
                {move || org_avatar(c).map(|id| {
                    let failed = RwSignal::new(false);
                    view! { <Show when=move || !failed.get()><img class="ws-img" src=avatar_url(&id, 72) alt="" on:error=move |_| failed.set(true)/></Show> }
                })}
            </button>
            {item(I::Home, tr().rail_home, ViewMode::Home)}
            {item(I::Chat, tr().rail_dms, ViewMode::Dms)}
            {item(I::Activity, tr().rail_activity, ViewMode::Activity)}
            <div class="rail-spacer"></div>
            // As in Slack: with a status set, an emoji badge sits above the avatar and together they form one "capsule".
            <button class=move || if me().is_some_and(|u| u.status.is_some()) { "rail-me with-status" } else { "rail-me" }
                title=tr().profile_and_prefs on:click=move |_| c.me_menu.update(|v| *v = !*v)>
                {move || me().and_then(|u| u.status).map(|st| {
                    view! { <span class="rail-status" data-tip=status_tip(&st)>{status_icon(&st, 18)}</span> }
                })}
                {move || me().map(|u| view! { <Avatar user=u size=36 presence=true/> })}
            </button>
            <Show when=move || c.me_menu.get()>
                <MeMenu/>
            </Show>
        </nav>
    }
}

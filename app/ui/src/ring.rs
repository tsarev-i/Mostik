//! The incoming call popup (the window opened at `index.html#ring`), like Slack's: who is calling,
//! "Decline" and "Accept". The backend rings, and closes the window once the call stops ringing.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::app::Avatar;
use crate::bridge;
use crate::i18n::tr;
use crate::icons::{I, Icon};
use crate::settings::Settings;

/// Whether this window is the popup.
pub fn is_popup() -> bool {
    window().location().hash().is_ok_and(|h| h == "#ring")
}

#[component]
pub fn RingPopup() -> impl IntoView {
    // The look and language of the main window (the settings are shared through localStorage).
    let st = Settings::load();
    crate::i18n::set_current(st.lang);
    if let Some(root) = document().document_element() {
        let _ = root.set_attribute("lang", st.lang.code());
        let _ = root.set_attribute("data-mode", if st.dark() { "dark" } else { "light" });
        let _ = root.set_attribute("data-theme", &st.theme);
    }
    let ring = RwSignal::new(None::<bridge::Ring>);
    let busy = RwSignal::new(false);
    spawn_local(async move {
        match bridge::ring_info().await {
            Ok(r) => ring.set(r),
            Err(e) => leptos::logging::warn!("ring_info: {e}"),
        }
    });
    let answer = move |accept: bool| {
        busy.set(true);
        spawn_local(async move {
            if let Err(e) = bridge::answer_ring(accept).await {
                leptos::logging::warn!("answer_ring: {e}");
                busy.set(false);
            }
        });
    };
    view! {
        <div class="ring">
            {move || ring.get().map(|r| view! {
                <div class="ring-caller">
                    <span class="ring-face"><Avatar user=r.caller.clone() size=56/></span>
                    <div class="ring-text">
                        <div class="ring-name">{r.caller.display_name.clone()}</div>
                        <div class="ring-sub">{tr().ring_calling}</div>
                    </div>
                </div>
            })}
            <div class="ring-actions">
                <button class="ring-btn decline" disabled=move || busy.get() on:click=move |_| answer(false)>
                    <Icon icon=I::PhoneOff size=18/><span>{tr().ring_decline}</span>
                </button>
                <button class="ring-btn accept" disabled=move || busy.get() on:click=move |_| answer(true)>
                    <Icon icon=I::Phone size=18/><span>{tr().ring_accept}</span>
                </button>
            </div>
        </div>
    }
}

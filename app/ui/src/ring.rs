//! The incoming call dialog, like Slack's: who is calling, "Decline" and "Accept". It opens inside
//! the main window like the preferences and stays in place; the backend rings and closes it once
//! the call stops ringing.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::app::Avatar;
use crate::bridge;
use crate::i18n::tr;
use crate::icons::{I, Icon};

#[component]
pub fn RingDialog(
    /// The ringing call; the caller's name and photo may arrive a moment later.
    ring: RwSignal<Option<bridge::Ring>>,
) -> impl IntoView {
    let busy = RwSignal::new(false);
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
        // Only the buttons close it: a stray click beside the dialog must not decline the call.
        <div class="modal-backdrop ring-backdrop">
            <div class="modal ring-modal" role="alertdialog" aria-label=tr().call_incoming>
                <header>
                    <h2>{tr().call_incoming}</h2>
                </header>
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
        </div>
    }
}

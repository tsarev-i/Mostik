//! Mostik user interface (Leptos CSR, compiled to wasm).
//!
//! For now this is a placeholder screen; chats, threads and the rest follow.

use leptos::prelude::*;

/// Root component: the placeholder screen shown until the real UI lands.
#[component]
fn App() -> impl IntoView {
    view! {
        <main class="stub">
            <img class="stub-logo" src="app-icon.png" alt="" />
            <h1>"Mostik"</h1>
            <p class="stub-note">"Unofficial Yandex Telemost client"</p>
        </main>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

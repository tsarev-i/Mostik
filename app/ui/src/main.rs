//! Mostik user interface (Leptos CSR, compiled to wasm).

mod app;
mod bridge;
mod emoji;
mod folders;
mod hidden;
mod i18n;
mod icons;
mod ops;
mod order;
mod ring;
mod settings;
mod time;

fn main() {
    console_error_panic_hook::set_once();
    if ring::is_popup() {
        leptos::mount::mount_to_body(ring::RingPopup);
    } else {
        leptos::mount::mount_to_body(app::App);
    }
}

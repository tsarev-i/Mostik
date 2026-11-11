//! Mostik user interface (Leptos CSR, compiled to wasm).

mod app;
mod bridge;
mod emoji;
mod i18n;
mod icons;
mod ops;
mod order;
mod settings;
mod time;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}

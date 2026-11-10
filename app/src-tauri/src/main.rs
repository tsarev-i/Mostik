//! Mostik desktop shell.
//!
//! For now it only opens the main window with the UI embedded from `app/ui/dist`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to start Mostik");
}

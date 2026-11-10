//! Mostik — неофициальный клиент Яндекс Телемост.

mod api;
mod auth;
mod conference;

fn main() {
    println!("mostik {}", env!("CARGO_PKG_VERSION"));
}

//! Mostik domain model.
//!
//! The types depend neither on the Telemost transport nor on the UI:
//! `mostik-yandex` builds them and `mostik-ui` renders them. [`event`] holds the live events
//! (what changed in the chats); the rest of what the backend and the UI send each other is in
//! `mostik-ipc`.

pub mod custom_emoji;
pub mod event;
pub mod feed;
pub mod markup;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
pub mod model;
pub mod reactions;
pub mod receipts;
pub mod split;

pub use model::*;

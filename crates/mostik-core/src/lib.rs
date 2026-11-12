//! Mostik domain model.
//!
//! The types depend neither on the Telemost transport nor on the UI:
//! `mostik-yandex` builds them and `mostik-ui` renders them.

pub mod custom_emoji;
pub mod feed;
pub mod markup;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
pub mod model;
pub mod reactions;
pub mod receipts;

pub use model::*;

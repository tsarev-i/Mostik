//! Client for the internal protocol of the Yandex Telemost web client.
//!
//! This is an **unofficial** implementation: the protocol is undocumented and was reconstructed
//! from the public web client. Mostik is not affiliated with Yandex.

pub mod clock;
pub mod convert;
pub mod diag;
pub mod endpoints;
pub mod identity;
pub mod net;
pub mod passport;
pub mod rpc;
pub mod session;
pub mod web;
pub mod wire;
pub mod xiva;
pub mod xiva_client;

pub use mostik_core::reactions;

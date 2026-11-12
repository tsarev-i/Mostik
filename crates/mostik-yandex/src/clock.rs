//! The wall clock in the units Telemost counts in.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Time since the Unix epoch; zero for a clock set before it.
fn since_epoch() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    since_epoch().as_millis() as u64
}

/// Microseconds since the Unix epoch: the unit of message times ([`mostik_core::Timestamp`]).
pub fn now_mcs() -> u64 {
    since_epoch().as_micros() as u64
}

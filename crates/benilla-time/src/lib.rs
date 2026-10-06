//! Platform-neutral wall and monotonic clocks.
//!
//! Benilla code uses this crate instead of selecting a clock backend itself.
//! Native targets retain the standard library's clock types, while browser
//! WASM uses clocks backed by `performance.now()` and `Date.now()`.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
pub use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

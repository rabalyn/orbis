//! Clocks that also work in a browser, where `crate::wtime::Instant` and `SystemTime` panic.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(target_arch = "wasm32")]
pub use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Waits on a thread of its own. A browser page cannot wait, so there it returns at once, and
/// the caller's pacing is kept by whoever runs it again (see `worker`).
pub fn pause(time: Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(time);
    #[cfg(target_arch = "wasm32")]
    let _ = time;
}

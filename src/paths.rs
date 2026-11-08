//! The user's folders. A browser has none, so there nothing is kept on disk.

#[cfg(not(target_arch = "wasm32"))]
pub use dirs::{cache_dir, config_dir};

#[cfg(target_arch = "wasm32")]
pub fn config_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn cache_dir() -> Option<std::path::PathBuf> {
    None
}

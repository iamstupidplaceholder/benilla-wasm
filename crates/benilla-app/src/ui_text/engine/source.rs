//! Font byte IO only. The text engine owns face registration, metrics and rasterization.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub(super) use browser::FontSource;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::FontSource;

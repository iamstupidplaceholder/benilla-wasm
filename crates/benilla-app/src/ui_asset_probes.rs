//! Host adapters for the original Lua texture/font/texel-size probes.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(any(target_arch = "wasm32", feature = "port-gates"))]
mod sizes;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::{install, UiAssetProbesPlugin};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::install;
#[cfg(feature = "port-gates")]
pub use sizes::verify_async_texture_size_probe;

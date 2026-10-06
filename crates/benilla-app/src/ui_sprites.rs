//! UI paint host access only; image policies and caches belong to benilla-assets.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::UiSprites;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::UiSprites;

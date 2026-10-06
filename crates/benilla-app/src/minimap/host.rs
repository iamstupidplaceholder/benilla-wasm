//! Minimap file/art IO only. Geometry, tile selection, blips and compositing
//! remain the production MinimapPlugin systems.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::MinimapAssetsPlugin;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::MinimapAssetsPlugin;

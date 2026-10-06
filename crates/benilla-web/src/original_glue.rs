//! The original client's glue rendering vocabulary, compiled into the browser
//! entry without copying its widgets or art rules.
//!
//! This is the first compile-time boundary of the login/glue slice. The
//! original screen and booth can be joined here as their synchronous asset
//! reads are moved onto the browser gateway.

#[path = "../../benilla-app/src/glue/add_material.rs"]
pub(crate) mod add_material;
#[path = "../../benilla-app/src/glue/art.rs"]
pub(crate) mod art;
#[path = "../../benilla-app/src/glue/backdrop.rs"]
pub(crate) mod backdrop;
#[path = "../../benilla-app/src/glue/widgets.rs"]
pub(crate) mod widgets;

/// The production glue layout scale (from `benilla-app/src/glue/mod.rs`).
pub(crate) fn screen_scale(window: Option<&bevy::window::Window>) -> f32 {
    window
        .map(|window| (window.height() / 768.0).min(2.2))
        .unwrap_or(1.0)
}

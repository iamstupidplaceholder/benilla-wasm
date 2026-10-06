//! The `uiScale` dial and the seam scale (decision 0582 + 0584).
//!
//! Shared leaf: the production `ui_script` host and the browser `ui_script`
//! facade both resolve `UiScaleCvar`/`seam_scale` through here, so the dial's
//! law and default live once. Pure arithmetic plus one env read; no VM, no
//! renderer, no platform seams.

use bevy::prelude::*;

/// The reference's `uiScale` cvar — the user dial on TOP of the 768-virtual base (decision 0584):
/// px-per-UI-unit multiplies by it, so the VM's virtual screen is `768/uiScale` units tall — the
/// same law that makes the reference's `uiScale = 768/screenH` its known pixel-perfect setting.
/// Folded into the one seam scale by [`seam_scale`]; at `1.0` (this `Default`, what every test
/// pins) the pipeline is bit-identical to the pre-dial 0582 behavior. The app inserts
/// [`default_ui_scale`] instead — the director's taste default, `WOW_UI_SCALE=` per-run override —
/// until a real cvar system subsumes this dial (0582's named residual).
#[derive(Resource)]
pub(crate) struct UiScaleCvar(pub(crate) f32);

impl Default for UiScaleCvar {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Shipped default: 100%, requested on 2026-10-05. Existing saved values and
/// WOW_UI_SCALE overrides still win. This is a fixed application default, not
/// the reference's automatic height-dependent max(768/H, 0.9) scale law.
pub(crate) const DEFAULT_UI_SCALE: f32 = 1.0;

/// The app's `uiScale`: `WOW_UI_SCALE=` if set (clamped to the plausible dial range), else
/// [`DEFAULT_UI_SCALE`] — the env override makes taste iteration a relaunch, not a rebuild.
pub(crate) fn default_ui_scale() -> f32 {
    std::env::var("WOW_UI_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v.clamp(0.5, 1.5))
        .unwrap_or(DEFAULT_UI_SCALE)
}

/// THE seam scale `s` (decision 0582 + the 0584 dial): window px per UI unit —
/// `windowH/768 × uiScale`. Every crossing of the VM boundary uses this one number: extraction
/// ×s out, mouse ÷s in, measures at ×s answered ÷s, the atlas bakes at ×s. A degenerate window
/// (h ≤ 0, the pre-winit frame) is identity.
pub(crate) fn seam_scale(window_h: f32, ui_scale: f32) -> f32 {
    if window_h > 0.0 {
        window_h / 768.0 * ui_scale
    } else {
        1.0
    }
}

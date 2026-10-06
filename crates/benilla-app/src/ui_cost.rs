//! The `[ui-cost]` instrument gate (decision 1174): who wants the UI pass measured.
//!
//! Shared leaf: the production `ui_script` host (and its `extract` flattening)
//! and the browser `ui_script` facade resolve `UiCostWanted` /
//! `ui_cost_enabled` through here, so the render pass reads one gate on both
//! targets. Pure state plus one cached env read; no VM, no renderer.

use bevy::prelude::*;

/// Does anything want the frame-cost table filled in this run? Measuring the split costs a clock read
/// per phase plus the churn strings, so the pass only pays when asked.
///
/// `WOW_UI_COST=1` (this module's own `[ui-cost]` line) arms it inline; the hover recorder arms it
/// by setting this — the direction that keeps the pass free of the instrument's name, and the
/// reason the flag is a resource rather than an env read here (decision 1174). Default `false` is
/// the player answer.
#[derive(Resource, Default)]
pub(crate) struct UiCostWanted(pub(crate) bool);

/// Optional controller self-time. No resource means no clock reads in normal play.
#[derive(Resource, Default)]
pub(crate) struct ControllerFrameCost(pub(crate) f64);

pub(crate) struct ControllerCostGuard<'a> {
    start: benilla_time::Instant,
    cost: &'a mut ControllerFrameCost,
}

impl ControllerFrameCost {
    pub(crate) fn measure(&mut self) -> ControllerCostGuard<'_> {
        ControllerCostGuard {
            start: benilla_time::Instant::now(),
            cost: self,
        }
    }
}

impl Drop for ControllerCostGuard<'_> {
    fn drop(&mut self) {
        self.cost.0 = self.start.elapsed().as_secs_f64() * 1000.0;
    }
}

/// instrument for the UI epoch-gate lane, 0730's warm slice): one `[ui-cost]` line per frame with
/// each phase's wall μs, the quad counts, the layout gate's decision, and whether the diff found
/// the produced quads changed. Untraced by design — the campaign grades untraced cpu_ms, and
/// `trace_chrome` inflates exactly the fine-grained spans this measures (0718's calibration).
pub(crate) fn ui_cost_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("WOW_UI_COST").as_deref() == Ok("1"))
}

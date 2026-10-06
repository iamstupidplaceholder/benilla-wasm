//! Host presentation admission, separate from Lua visibility and lifecycle.
use super::{model::Model, UiScript};
use crate::{order::ZTarget, widget::FrameHandle};
use std::collections::HashSet;

impl Model {
    /// Pointer input follows the last painted rectangle, not a future layout.
    /// Hover-driven size animation must not make a still-painted frame vanish
    /// from hit testing while the next paint is catching up.
    pub(super) fn pointer_rect(&self, h: FrameHandle) -> Option<&crate::layout::Rect> {
        match &self.presented_input {
            Some(shown) => shown.get(&h),
            None => self.resolved.get(&h),
        }
    }

    pub(super) fn input_presented(&self, h: FrameHandle) -> bool {
        self.presented_input.as_ref().is_none_or(|shown| {
            shown
                .get(&h)
                .is_some_and(|rect| self.resolved.get(&h) == Some(rect))
        })
    }

    fn owner(&self, target: ZTarget) -> Option<FrameHandle> {
        match target {
            ZTarget::Frame(h) => Some(h),
            ZTarget::Region(h) => self.arena.region(h).map(|r| r.owner),
        }
    }

    fn domain(&self, mut h: FrameHandle) -> FrameHandle {
        loop {
            let Some(frame) = self.arena.frame(h) else {
                return h;
            };
            if frame.toplevel {
                return h;
            }
            let Some(parent) = frame.parent else { return h };
            let Some(p) = self.arena.frame(parent) else {
                return h;
            };
            // Screen containers are not a dialog: independent children must
            // not hold the action bars/chat hostage to another child's art.
            if matches!(p.name.as_deref(), Some("UIParent" | "GlueParent"))
                || (p.parent.is_none()
                    && !matches!(
                        p.strata,
                        crate::order::Strata::Fullscreen | crate::order::Strata::FullscreenDialog
                    ))
            {
                return h;
            }
            h = parent;
        }
    }

    fn beneath(&self, mut h: FrameHandle, roots: &HashSet<FrameHandle>) -> bool {
        loop {
            if roots.contains(&h) {
                return true;
            }
            let Some(parent) = self.arena.frame(h).and_then(|f| f.parent) else {
                return false;
            };
            h = parent;
        }
    }
}

impl UiScript {
    /// Original fullscreen Lua hides UIParent before showing its replacement.
    /// The host must retain the previous paint until that replacement is admitted,
    /// without undoing Lua visibility or enabling input on the hidden UI.
    pub fn presentation_replacement_pending(&self) -> bool {
        let model = self.model_ref();
        let ui_hidden = model
            .arena
            .iter_frames()
            .any(|(_, f)| f.name.as_deref() == Some("UIParent") && !f.effective_visible);
        ui_hidden
            && model.pending_art_domains.iter().any(|h| {
                !model.admitted_domains.contains(h)
                    && model.arena.frame(*h).is_some_and(|f| {
                        f.effective_visible
                            && matches!(
                                f.strata,
                                crate::order::Strata::Fullscreen
                                    | crate::order::Strata::FullscreenDialog
                            )
                    })
            })
    }

    pub fn presentation_art_revision(&self) -> u64 {
        self.model_ref().presentation_art_revision
    }
    /// Atomically publish which original UI domains are absent from this paint.
    /// Does not run Hide/Show, change focus or change Lua's IsShown result.
    pub fn admit_presentation(&mut self, pending: &[ZTarget]) -> HashSet<ZTarget> {
        let mut model = self.model_mut();
        let domains: HashSet<_> = pending
            .iter()
            .filter_map(|t| model.owner(*t))
            .map(|h| model.domain(h))
            .collect();
        // A domain becomes repaintable once its entire current request batch
        // has resolved. Track already-admitted dialogs too: they stay visible,
        // but their cached quads must be refreshed after byte arrival.
        if model
            .pending_art_domains
            .iter()
            .any(|h| !domains.contains(h))
        {
            model.presentation_art_revision = model.presentation_art_revision.wrapping_add(1);
        }
        model.pending_art_domains = domains.clone();
        let domains: HashSet<_> = domains
            .into_iter()
            .filter(|h| !model.admitted_domains.contains(h))
            .collect();
        let blocked: HashSet<_> = if domains.is_empty() {
            HashSet::new()
        } else {
            model
                .arena
                .iter_frames()
                .filter(|(h, _)| model.beneath(*h, &domains))
                .map(|(h, _)| h)
                .collect()
        };
        let excluded = self_targets(&model, &blocked);
        // Admission is sticky for the VM lifetime, not reset by tab switches
        // or Hide/Show. New artwork cannot make an already shown dialog blink.
        let admitted: Vec<_> = model
            .arena
            .iter_frames()
            .filter(|(h, f)| f.effective_visible && !blocked.contains(h))
            .map(|(h, _)| model.domain(h))
            .collect();
        model.admitted_domains.extend(admitted);
        model.presented_input = Some(
            model
                .arena
                .iter_frames()
                .filter(|(h, f)| f.effective_visible && !blocked.contains(h))
                .filter_map(|(h, _)| model.resolved.get(&h).copied().map(|r| (h, r)))
                .collect(),
        );
        // A press cannot complete on a window which disappeared from glass.
        let cancelled: Vec<_> = model
            .mouse_down_on
            .values()
            .copied()
            .filter(|h| blocked.contains(h))
            .collect();
        for h in cancelled {
            super::button::edge(&mut model, h, crate::widget::ButtonState::on_mouse_up);
        }
        for h in &blocked {
            if let Some(frame) = model.arena.frame_mut(*h) {
                if let crate::widget::KindState::EditBox(eb) = &mut frame.kind_state {
                    eb.drag_active = false;
                }
            }
        }
        model.mouse_down_on.retain(|_, h| !blocked.contains(h));
        if model.mouse_capture.is_some_and(|h| blocked.contains(&h)) {
            model.mouse_capture = None;
        }
        if model
            .drag
            .as_ref()
            .is_some_and(|d| blocked.contains(&d.source))
        {
            model.drag = None;
        }
        if model
            .slider_drag
            .as_ref()
            .is_some_and(|d| blocked.contains(&d.slider))
        {
            model.slider_drag = None;
        }
        if model
            .color_drag
            .as_ref()
            .is_some_and(|d| blocked.contains(&d.frame))
        {
            model.color_drag = None;
        }
        if model
            .moving
            .as_ref()
            .is_some_and(|d| blocked.contains(&d.frame))
        {
            model.moving = None;
        }
        if model
            .sizing
            .as_ref()
            .is_some_and(|d| blocked.contains(&d.frame))
        {
            model.sizing = None;
        }
        excluded
    }
}

fn self_targets(model: &Model, blocked: &HashSet<FrameHandle>) -> HashSet<ZTarget> {
    let mut targets = HashSet::new();
    for (h, f) in model.arena.iter_frames() {
        if blocked.contains(&h) {
            targets.insert(ZTarget::Frame(h));
            targets.extend(f.regions.iter().copied().map(ZTarget::Region));
        }
    }
    targets
}

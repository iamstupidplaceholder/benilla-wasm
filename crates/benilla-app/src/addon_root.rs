//! The one AddOns root (decision 1185): where addons live on disk.
//!
//! Shared leaf: the production addon registry and the font engine's loose-file
//! resolve (plus the browser `ui_script` facade) ask one question — "where do
//! addons live" — however it is asked. Pure path logic over one env fact;
//! `None` under `$WOW_CAPTURE`, so a deterministic baseline never depends on
//! what is installed on the machine that runs it (0008).

use std::path::PathBuf;

/// **The** addon folder — `<benilla-config>/AddOns/` — or `None` when there is none to read.
///
/// **One root, never two** (decision 1185, the director's call). 1184 searched the WoW install's
/// `Interface/AddOns/` as well and merged the results. Two roots means an addon's identity depends
/// on which folder won, a name can be shadowed, and "where is this addon loaded from" stops having
/// one answer. If the install's folder ever becomes reachable it will be *instead of* this one and
/// by explicit choice, not alongside it.
///


const ADDONS_DIR: &str = "AddOns";

/// It is ours for the same reason 1180 moved our state out of the install: benilla reads a WoW
/// install, it does not live in one. Resolved through [`crate::local_state`], the only module
/// allowed to compute that path (0954) — which is already `None` under `$WOW_CAPTURE`, so a
/// deterministic baseline cannot depend on what somebody has installed (0008).
///
/// `pub(crate)` since 1322: every asset an addon ships needs the same folder — the sprite
/// decoder's `Interface\AddOns\` loose-file resolve, the `SetTexture` probe, and (2103) the font
/// engine's face loader plus the `SetFont` probe all map that virtual prefix onto this root, so
/// there is exactly one answer to "where do addons live" however the question is asked. The first
/// three are wired in one place (`ui_script::lifecycle::install_addon_asset_resolvers`); the face
/// loader reads it here directly, at engine construction, because it is not a VM concern.
pub(crate) fn root() -> Option<PathBuf> {
    root_from(crate::local_state::home())
}

/// [`root`] with its one environment fact passed in, so it is testable without touching the
/// process environment.
///
/// Not tidiness: `install::candidates_from` exists for exactly this reason (1175's own flake — a
/// test that *sets* an env var poisons every other test running concurrently in the same process,
/// and the failure then moves around with scheduling).
pub(crate) fn root_from(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|h| h.join(ADDONS_DIR))
}

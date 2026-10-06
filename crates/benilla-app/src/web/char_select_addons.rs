//! Browser boundary for the optional AddOns panel.
//!
//! Addon discovery and enable files are host-filesystem services. The original
//! character-select screen remains intact, but hides this optional button until
//! a browser addon store is deliberately introduced.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub(super) struct AddonsPanel {
    pub(super) open: bool,
}

impl AddonsPanel {
    pub(super) fn open_for(&mut self, _realm: String, _characters: Vec<String>) {
        self.open = false;
    }

    pub(super) fn any_installed() -> bool {
        false
    }
}

pub(super) fn drive_addons_panel() {}

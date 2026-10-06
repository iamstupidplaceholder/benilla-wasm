//! Admission of asynchronous game catalogs before the production FrameXML load.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct UiGameDataReady(pub(crate) bool);

#[cfg(target_arch = "wasm32")]
#[path = "web/ui_game_data.rs"]
mod browser;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::{entry_wait_status, UiGameDataPlugin};

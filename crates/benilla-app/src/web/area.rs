//! Browser publication point for the production AreaTable catalog.
//!
//! Delivery remains asynchronous, but consumers see the same catalog type as native. Until the
//! browser DBC loader inserts it, their existing `Option<Res<AreaTableRes>>` path stays empty.

use bevy::prelude::*;

#[derive(Resource)]
pub(crate) struct AreaTableRes(pub(crate) benilla_formats::AreaTableCatalog);

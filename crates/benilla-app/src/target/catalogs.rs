//! Targeting's catalog IO boundary. Consumers keep the original typed resources
//! and decoders; a pending browser read is not a successful empty catalog.

use super::{ring::Factions, scan::CreatureTypes};
use bevy::prelude::*;

#[cfg(any(target_arch = "wasm32", feature = "port-gates"))]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::TargetCatalogsPlugin;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::TargetCatalogsPlugin;

#[cfg(feature = "port-gates")]
pub(super) use browser::verify_delivery;

fn publish_factions(commands: &mut Commands, source: &mut impl benilla_formats::FileSource) {
    match benilla_formats::load_faction_catalog(source) {
        Ok(catalog) => {
            info!("faction catalog: {} template rows", catalog.len());
            commands.insert_resource(Factions(catalog));
        }
        Err(e) => warn!("faction catalog unavailable, ring stays neutral: {e:#}"),
    }
}

fn publish_creature_types(commands: &mut Commands, source: &mut impl benilla_formats::FileSource) {
    match benilla_formats::load_creature_type_flags(source) {
        Ok(flags) => {
            info!("creature types: {} rows", flags.len());
            commands.insert_resource(CreatureTypes(flags));
        }
        Err(e) => warn!("CreatureType.dbc unavailable, critters stay TAB-able: {e:#}"),
    }
}

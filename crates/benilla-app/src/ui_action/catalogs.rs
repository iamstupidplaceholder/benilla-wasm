//! Spell table IO only. The original publication policy and decoders are shared;
//! action/cast/targeting consumers do not select a host backend.

use super::{SpellMechanics, Spells};
use benilla_formats::FileSource;
use bevy::prelude::*;

#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
pub(crate) struct ActionCatalogsReady(pub(crate) bool);

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::ActionCatalogsPlugin;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::ActionCatalogsPlugin;

fn publish_mechanics(commands: &mut Commands, source: &mut impl FileSource) {
    match benilla_formats::load_spell_mechanic_catalog(source) {
        Ok(catalog) => {
            debug!("ui_action: {} spell-mechanic name(s)", catalog.len());
            commands.insert_resource(SpellMechanics { catalog });
        }
        Err(e) => warn!(
            "ui_action: SpellMechanic.dbc failed to load — the crowd-control refusal drops the \
             mechanic name: {e:#}"
        ),
    }
}

fn publish_spells(commands: &mut Commands, source: &mut impl FileSource) -> bool {
    match benilla_formats::load_spell_catalog(source) {
        Ok(catalog) => {
            let forms = benilla_formats::load_shapeshift_forms(source).unwrap_or_else(|e| {
                warn!("ui_action: SpellShapeshiftForm.dbc failed — stance paging off: {e:#}");
                Default::default()
            });
            let ranges = benilla_formats::load_spell_ranges(source).unwrap_or_else(|e| {
                warn!("ui_action: SpellRange.dbc failed — range indicator off: {e:#}");
                benilla_formats::SpellRangeCatalog::default()
            });
            let cast_times = benilla_formats::load_spell_cast_times(source).unwrap_or_else(|e| {
                warn!("ui_action: SpellCastTimes.dbc failed — cast-time cell off: {e:#}");
                Default::default()
            });
            let durations = benilla_formats::load_spell_durations(source).unwrap_or_else(|e| {
                warn!("ui_action: SpellDuration.dbc failed — $d/$o tokens off: {e:#}");
                Default::default()
            });
            let radii = benilla_formats::load_spell_radii(source).unwrap_or_else(|e| {
                warn!("ui_action: SpellRadius.dbc failed — $a token off: {e:#}");
                Default::default()
            });
            info!(
                "ui_action: {} spells in the display catalog, {} shapeshift forms, {} range rows, \
                 {} cast times, {} durations, {} radii",
                catalog.len(),
                forms.len(),
                ranges.len(),
                cast_times.len(),
                durations.len(),
                radii.len()
            );
            commands.insert_resource(Spells {
                catalog,
                forms,
                ranges,
                cast_times,
                durations,
                radii,
            });
            true
        }
        Err(e) => {
            warn!("ui_action: Spell.dbc failed to load — bar icons disabled: {e:#}");
            false
        }
    }
}

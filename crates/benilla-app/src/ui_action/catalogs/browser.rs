//! Async delivery of the original spell decoder's inputs. A request must reach
//! Ready or Failed before decode; Pending never activates a native miss fallback.

use benilla_assets::RawFile;
use benilla_formats::MemoryFileSource;
use bevy::{asset::LoadState, prelude::*};

const SPELL_FILES: &[&str] = &[
    "DBFilesClient/Spell.dbc",
    "DBFilesClient/SpellIcon.dbc",
    "DBFilesClient/SpellDispelType.dbc",
    "DBFilesClient/SpellCategory.dbc",
    "DBFilesClient/SpellShapeshiftForm.dbc",
    "DBFilesClient/SpellRange.dbc",
    "DBFilesClient/SpellCastTimes.dbc",
    "DBFilesClient/SpellDuration.dbc",
    "DBFilesClient/SpellRadius.dbc",
];
const MECHANIC_FILES: &[&str] = &["DBFilesClient/SpellMechanic.dbc"];

#[derive(Resource)]
struct Delivery {
    spells: Option<Vec<(&'static str, Handle<RawFile>)>>,
    mechanics: Option<Vec<(&'static str, Handle<RawFile>)>>,
}

pub(crate) struct ActionCatalogsPlugin;

impl Plugin for ActionCatalogsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<super::ActionCatalogsReady>()
            .add_systems(Startup, request)
            .add_systems(Update, publish.before(crate::ui_script::UiFeed));
    }
}

fn request(mut commands: Commands, server: Res<AssetServer>) {
    let load = |paths: &[&'static str]| {
        paths
            .iter()
            .map(|&path| (path, server.load(format!("mpq://{path}"))))
            .collect()
    };
    commands.insert_resource(Delivery {
        spells: Some(load(SPELL_FILES)),
        mechanics: Some(load(MECHANIC_FILES)),
    });
}

fn collect(
    loads: &[(&str, Handle<RawFile>)],
    raw: &Assets<RawFile>,
    server: &AssetServer,
) -> Option<MemoryFileSource> {
    // Unlike faction group publication, the original spell loader has optional
    // inputs. Finish each transport first, then let that loader own miss policy.
    if loads.iter().any(|(_, handle)| {
        raw.get(handle).is_none()
            && !matches!(
                server.get_load_state(handle.id()),
                Some(LoadState::Failed(_))
            )
    }) {
        return None;
    }
    let mut bytes = Vec::new();
    for (path, handle) in loads {
        match raw.get(handle) {
            Some(file) => bytes.push((*path, file.0.clone())),
            None => warn!(
                "ui_action: table delivery failed for {path}: {:?}",
                server.get_load_state(handle.id())
            ),
        }
    }
    Some(MemoryFileSource::new(bytes))
}

fn publish(
    mut commands: Commands,
    delivery: Option<ResMut<Delivery>>,
    raw: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
    mut ready: ResMut<super::ActionCatalogsReady>,
) {
    let Some(mut delivery) = delivery else { return };
    if let Some(loads) = &delivery.spells {
        if let Some(mut source) = collect(loads, &raw, &server) {
            let admitted = super::publish_spells(&mut commands, &mut source);
            delivery.spells = None;
            ready.0 = admitted;
        }
    }
    if let Some(loads) = &delivery.mechanics {
        if let Some(mut source) = collect(loads, &raw, &server) {
            super::publish_mechanics(&mut commands, &mut source);
            delivery.mechanics = None;
        }
    }
    if delivery.spells.is_none() && delivery.mechanics.is_none() {
        commands.remove_resource::<Delivery>();
    }
}

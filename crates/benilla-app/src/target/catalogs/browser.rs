//! Gateway byte delivery only; reaction/scan policy stays in production targeting.

use benilla_assets::RawFile;
use benilla_formats::MemoryFileSource;
use bevy::{asset::LoadState, prelude::*};

const FACTIONS: &[&str] = &[
    "DBFilesClient/FactionTemplate.dbc",
    "DBFilesClient/Faction.dbc",
    "DBFilesClient/FactionGroup.dbc",
];
const CREATURE_TYPES: &[&str] = &["DBFilesClient/CreatureType.dbc"];

#[derive(Resource)]
struct Delivery {
    factions: Option<Vec<(&'static str, Handle<RawFile>)>>,
    creatures: Option<Vec<(&'static str, Handle<RawFile>)>>,
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct TargetCatalogsPlugin;

#[cfg(target_arch = "wasm32")]
impl Plugin for TargetCatalogsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, request)
            .add_systems(Update, publish.before(crate::ui_script::UiFeed));
    }
}

#[cfg(target_arch = "wasm32")]
fn request(mut commands: Commands, server: Res<AssetServer>) {
    let load = |paths: &[&'static str]| {
        paths
            .iter()
            .map(|&path| (path, server.load(format!("mpq://{path}"))))
            .collect()
    };
    commands.insert_resource(Delivery {
        factions: Some(load(FACTIONS)),
        creatures: Some(load(CREATURE_TYPES)),
    });
}

#[cfg(feature = "port-gates")]
pub(in crate::target) fn verify_delivery() {
    use super::super::{ring::Factions, scan::CreatureTypes};
    use bevy::asset::AssetPlugin;

    fn dbc(fields: u32, rows: &[Vec<u32>]) -> Vec<u8> {
        let mut bytes = b"WDBC".to_vec();
        for word in [rows.len() as u32, fields, fields * 4, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for row in rows {
            assert_eq!(row.len(), fields as usize);
            for word in row {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        bytes.push(0);
        bytes
    }
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        benilla_assets::RawFilePlugin,
    ))
    .add_systems(Update, publish);
    let raw = app.world_mut().resource_mut::<Assets<RawFile>>();
    let factions: Vec<_> = FACTIONS
        .iter()
        .map(|&path| (path, raw.reserve_handle()))
        .collect();
    let creatures: Vec<_> = CREATURE_TYPES
        .iter()
        .map(|&path| (path, raw.reserve_handle()))
        .collect();
    let fh: Vec<_> = factions.iter().map(|(_, h)| h.clone()).collect();
    let ch = creatures[0].1.clone();
    app.world_mut().insert_resource(Delivery {
        factions: Some(factions),
        creatures: Some(creatures),
    });
    app.update();
    assert!(!app.world().contains_resource::<Factions>());
    assert!(!app.world().contains_resource::<CreatureTypes>());

    let mut critter = vec![0; 11];
    critter[0] = 8;
    critter[10] = 1;
    let mut totem = vec![0; 11];
    totem[0] = 11;
    app.world_mut()
        .resource_mut::<Assets<RawFile>>()
        .insert(ch.id(), RawFile(dbc(11, &[critter, totem])))
        .unwrap();
    app.update();
    assert!(app.world().resource::<CreatureTypes>().0.no_tab_target(8));
    assert!(!app.world().resource::<CreatureTypes>().0.no_tab_target(11));
    assert!(
        !app.world().contains_resource::<Factions>(),
        "one catalog must not fabricate the other"
    );

    // Deliver reverse request order. Factions is published only after all three
    // files can be read by the unchanged production decoder.
    app.world_mut()
        .resource_mut::<Assets<RawFile>>()
        .insert(fh[2].id(), RawFile(dbc(12, &[])))
        .unwrap();
    let mut faction = vec![0; 37];
    faction[0] = 72;
    faction[1] = u32::MAX;
    app.world_mut()
        .resource_mut::<Assets<RawFile>>()
        .insert(fh[1].id(), RawFile(dbc(37, &[faction])))
        .unwrap();
    app.update();
    assert!(!app.world().contains_resource::<Factions>());
    let mut a = vec![0; 14];
    a[0] = 1;
    a[1] = 72;
    a[3] = 1;
    a[5] = 2;
    let mut b = vec![0; 14];
    b[0] = 2;
    b[1] = 73;
    b[3] = 2;
    app.world_mut()
        .resource_mut::<Assets<RawFile>>()
        .insert(fh[0].id(), RawFile(dbc(14, &[a, b])))
        .unwrap();
    app.update();
    let catalog = &app.world().resource::<Factions>().0;
    assert_eq!(
        catalog
            .template(1)
            .unwrap()
            .reaction_toward(catalog.template(2).unwrap()),
        benilla_formats::Reaction::Hostile
    );
    assert!(
        catalog.reputation_faction(72).is_none(),
        "negative reputation index means no reputation slot"
    );
    assert!(!app.world().contains_resource::<Delivery>());

    // Malformed fully delivered bytes are terminal, not a pending or empty
    // successful catalog. Other independently admitted resources remain intact.
    app.world_mut().remove_resource::<Factions>();
    app.world_mut()
        .resource_mut::<Assets<RawFile>>()
        .insert(fh[0].id(), RawFile(vec![0xff]))
        .unwrap();
    app.world_mut().insert_resource(Delivery {
        factions: Some(
            FACTIONS
                .iter()
                .zip(fh)
                .map(|(&path, h)| (path, h))
                .collect(),
        ),
        creatures: None,
    });
    app.update();
    assert!(!app.world().contains_resource::<Factions>());
    assert!(!app.world().contains_resource::<Delivery>());
    assert!(app.world().resource::<CreatureTypes>().0.no_tab_target(8));

    // Exercise a real AssetServer IO failure (not manually assigned LoadState).
    let missing = app
        .world()
        .resource::<AssetServer>()
        .load::<RawFile>("benilla-target-gate-missing/FactionTemplate.dbc");
    app.world_mut().insert_resource(Delivery {
        factions: Some(vec![(FACTIONS[0], missing)]),
        creatures: None,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.world().contains_resource::<Delivery>() {
        assert!(
            std::time::Instant::now() < deadline,
            "failed host IO must finish the job, not stay Pending"
        );
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.world().contains_resource::<Factions>());
    assert!(app.world().resource::<CreatureTypes>().0.no_tab_target(8));
}

enum Readiness {
    Pending,
    Ready(MemoryFileSource),
    Failed(String),
}

fn collect(
    loads: &[(&str, Handle<RawFile>)],
    raw: &Assets<RawFile>,
    server: &AssetServer,
) -> Readiness {
    for (path, handle) in loads {
        if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            return Readiness::Failed(format!("{path}: {error}"));
        }
    }
    if loads.iter().any(|(_, handle)| raw.get(handle).is_none()) {
        return Readiness::Pending;
    }
    Readiness::Ready(MemoryFileSource::new(
        loads
            .iter()
            .map(|(path, handle)| (*path, raw.get(handle).unwrap().0.clone())),
    ))
}

fn publish(
    mut commands: Commands,
    delivery: Option<ResMut<Delivery>>,
    raw: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
) {
    let Some(mut delivery) = delivery else { return };
    if let Some(loads) = &delivery.factions {
        match collect(loads, &raw, &server) {
            Readiness::Pending => {}
            Readiness::Ready(mut source) => {
                super::publish_factions(&mut commands, &mut source);
                delivery.factions = None;
            }
            Readiness::Failed(error) => {
                warn!("faction catalog delivery failed, ring stays neutral: {error}");
                delivery.factions = None;
            }
        }
    }
    if let Some(loads) = &delivery.creatures {
        match collect(loads, &raw, &server) {
            Readiness::Pending => {}
            Readiness::Ready(mut source) => {
                super::publish_creature_types(&mut commands, &mut source);
                delivery.creatures = None;
            }
            Readiness::Failed(error) => {
                warn!("CreatureType.dbc delivery failed, critters stay TAB-able: {error}");
                delivery.creatures = None;
            }
        }
    }
    if delivery.factions.is_none() && delivery.creatures.is_none() {
        commands.remove_resource::<Delivery>();
    }
}

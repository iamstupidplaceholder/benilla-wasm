//! Opt-in nonvisual diagnostics. Fixtures are real components/resources; every
//! event runs through the production handler table, never a test transport or
//! replacement gameplay implementation.

use benilla_protocol::{
    EntityKind, MonsterMoveFacing, MoveSpeeds, ObjectFields, SessionEvent, SpeedKind, SplineMode,
};
use bevy::ecs::system::RunSystemOnce;

use super::*;

fn app() -> App {
    let mut app = App::new();
    app.init_resource::<AddonInfoReply>()
        .init_resource::<SelfGuid>()
        .init_resource::<NetStatus>()
        .init_resource::<crate::names::NameCache>()
        .init_resource::<ServerTime>()
        .init_resource::<ServerWallClock>()
        .init_resource::<GuidIndex>()
        .add_message::<EnteredWorldMessage>()
        .add_message::<TeleportMessage>()
        .add_message::<SpeedChangeMessage>();
    session_entry::register(&mut app);
    object_stream::register(&mut app);
    app
}

fn dispatch(app: &mut App, events: Vec<SessionEvent>) {
    handlers::dispatch(app.world_mut(), events, |_, unclaimed| {
        panic!("port contract events were not claimed: {unclaimed:?}");
    });
}

fn entry(guid: u64, name: &str, addon_info: Option<Vec<String>>) -> SessionEvent {
    SessionEvent::Connected {
        self_guid: guid,
        name: name.into(),
        billing_time_rested: 0,
        tutorial_flags: None,
        addon_info,
    }
}

/// Repeat world entry must wipe stale player names/addon verdicts; teleport
/// filtering must see the self guid published earlier in the same drain.
pub fn verify_session_entry() {
    let mut app = app();
    dispatch(
        &mut app,
        vec![entry(1, "First", Some(vec!["Addon".into()]))],
    );
    let teleport = |guid, counter| SessionEvent::Teleport {
        guid,
        counter,
        position: [1.0, 2.0, 3.0],
        orientation: 0.5,
    };
    dispatch(
        &mut app,
        vec![
            teleport(2, 0),
            entry(2, "Second", None),
            teleport(1, 1),
            teleport(2, 2),
            SessionEvent::TimeSpeed {
                hours: 12,
                minutes: 30,
                day_serial: 42,
                timescale: 1.0,
            },
            SessionEvent::ServerUnixTime {
                unix_time: 1_700_000_000,
            },
        ],
    );
    let world = app.world();
    let names = world.resource::<crate::names::NameCache>();
    assert_eq!(names.peek(1), None);
    assert_eq!(names.peek(2), Some("Second"));
    assert_eq!(world.resource::<SelfGuid>().0, Some(2));
    assert!(world.resource::<AddonInfoReply>().0.is_none());
    assert!(world.resource::<NetStatus>().connected);
    assert_eq!(world.resource::<Messages<EnteredWorldMessage>>().len(), 2);
    let mut teleports = world
        .resource::<Messages<TeleportMessage>>()
        .iter_current_update_messages();
    assert_eq!(teleports.next().unwrap().counter, 2);
    assert!(teleports.next().is_none());
    assert!(world.resource::<ServerTime>().0.is_some());
    assert!(world.resource::<ServerWallClock>().0.is_some());
}

fn walk(transport: Option<u64>, stop: bool) -> SessionEvent {
    SessionEvent::MonsterMove {
        guid: 7,
        transport,
        start: [0.0; 3],
        spline_id: 1,
        path: vec![[0.0; 3], [10.0, 0.0, 0.0]],
        facing: MonsterMoveFacing::None,
        stop,
        duration_ms: 1000,
        flying: false,
        run_mode: true,
    }
}

/// Exercise the real spline installer/sampler, authoritative correction,
/// root gate and deck attachment, including two packets in one drain.
pub fn verify_object_motion() {
    let mut app = app();
    let entity = app.world_mut().spawn((Guid(7), Transform::default())).id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(7, entity);
    dispatch(&mut app, vec![walk(None, false)]);
    let spline = app
        .world()
        .get::<Spline>(entity)
        .expect("production handler installs a spline");
    let (end, _, pitch) = spline.sample(Instant::now() + Duration::from_secs(2));
    assert_eq!(end, [10.0, 0.0, 0.0]);
    assert_eq!(pitch, 0.0);
    dispatch(
        &mut app,
        vec![SessionEvent::ObjectMove {
            guid: 7,
            position: [3.0, 4.0, 5.0],
            orientation: 0.25,
        }],
    );
    assert!(app.world().get::<Spline>(entity).is_none());
    assert_eq!(
        app.world().get::<Transform>(entity).unwrap().translation,
        benilla_assets::coords::wow_to_bevy([3.0, 4.0, 5.0])
    );
    dispatch(&mut app, vec![walk(Some(99), false)]);
    assert_eq!(
        app.world()
            .get::<crate::transport::TransportRider>(entity)
            .unwrap()
            .transport_guid,
        99
    );
    dispatch(&mut app, vec![walk(None, true)]);
    assert!(app.world().get::<Spline>(entity).is_none());
    assert!(app
        .world()
        .get::<crate::transport::TransportRider>(entity)
        .is_none());
    dispatch(
        &mut app,
        vec![
            SessionEvent::SplineMoveMode {
                guid: 7,
                mode: SplineMode::Root,
                apply: true,
            },
            SessionEvent::SplineMoveMode {
                guid: 7,
                mode: SplineMode::Hover,
                apply: true,
            },
            walk(None, false),
        ],
    );
    assert!(
        app.world().get::<Spline>(entity).is_none(),
        "root refuses a new path"
    );
    assert!(
        app.world().get::<UnitMoveModes>(entity).unwrap().hovering(),
        "successive grants merge"
    );
    dispatch(
        &mut app,
        vec![
            SessionEvent::SplineMoveMode {
                guid: 7,
                mode: SplineMode::Root,
                apply: false,
            },
            walk(None, false),
        ],
    );
    assert!(
        app.world().get::<Spline>(entity).is_some(),
        "same-drain unroot permits a path"
    );
    assert!(
        app.world().get::<UnitMoveModes>(entity).unwrap().hovering(),
        "unroot preserves hover"
    );
    dispatch(&mut app, vec![walk(None, false), walk(None, true)]);
    assert!(
        app.world().get::<Spline>(entity).is_none(),
        "same-drain stop lands after start"
    );
}

// Fixture plumbing for the still-unpeeled create boundary: invoke the ORIGINAL
// object-create operation and land its pending stores/speeds through Commands.
// This is not a replacement create handler installed in either client.
fn create_speed_fixture(
    In(event): In<SessionEvent>,
    mut commands: Commands,
    mut index: ResMut<GuidIndex>,
    mut transforms: Query<&mut Transform>,
    mut stores: Query<&mut ObjectStore>,
    mut edges: MessageWriter<FieldChanged>,
    names: Res<crate::names::NameCache>,
    templates: Res<crate::go_templates::GameObjectTemplates>,
    net: Res<NetCommands>,
) {
    let SessionEvent::ObjectCreate {
        guid,
        kind,
        display_id,
        position,
        orientation,
        scale,
        speeds,
        mover,
        transport_progress,
        transport,
        spline,
        fields,
    } = event
    else {
        panic!("create fixture only accepts ObjectCreate")
    };
    let mut pending = std::collections::HashMap::new();
    let mut stage = object_stream::SpeedStage::default();
    object_stream::object_create(
        guid,
        kind,
        display_id,
        position,
        orientation,
        scale,
        speeds,
        mover,
        transport_progress,
        transport,
        spline,
        fields,
        &mut commands,
        &mut index,
        &mut transforms,
        &mut stores,
        &mut pending,
        &mut edges,
        &mut stage,
        &names,
        &templates,
        &net,
    );
    for (guid, fields) in pending {
        commands.entity(index.0[&guid]).insert(ObjectStore(fields));
    }
    stage.flush(&mut commands, &index);
}

/// The actual create operation followed by table-owned speed changes in one
/// drain: latest packet wins, unrelated slots survive, only self gets ACK input.
pub fn verify_speed_changes() {
    let mut app = app();
    let (commands, _receiver) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(commands))
        .init_resource::<crate::go_templates::GameObjectTemplates>()
        .add_message::<FieldChanged>();
    app.world_mut().resource_mut::<SelfGuid>().0 = Some(7);
    let create = SessionEvent::ObjectCreate {
        guid: 7,
        kind: EntityKind::Player,
        display_id: None,
        position: [0.0; 3],
        orientation: 0.0,
        scale: 1.0,
        speeds: Some(MoveSpeeds {
            run: 11.2,
            walk: 2.5,
            swim: 4.0,
            ..Default::default()
        }),
        mover: None,
        transport_progress: None,
        transport: None,
        spline: None,
        fields: ObjectFields::default(),
    };
    let force = |guid, counter, speed| SessionEvent::ForceSpeedChange {
        guid,
        kind: SpeedKind::Run,
        counter,
        speed,
    };
    handlers::dispatch(
        app.world_mut(),
        vec![
            create,
            force(7, 1, 7.0),
            SessionEvent::SpeedChanged {
                guid: 7,
                kind: SpeedKind::Swim,
                speed: 6.0,
            },
            force(7, 2, 8.0),
        ],
        |world, events| {
            for event in events {
                world
                    .run_system_once_with(create_speed_fixture, event)
                    .expect("original create runs");
            }
        },
    );
    let entity = app.world().resource::<GuidIndex>().0[&7];
    let speeds = app.world().get::<UnitSpeeds>(entity).unwrap().0;
    assert_eq!(speeds.run, 8.0);
    assert_eq!(speeds.walk, 2.5);
    assert_eq!(speeds.swim, 6.0);
    // A foreign tracked mover changes speed without asking our controller to ACK.
    let foreign = app
        .world_mut()
        .spawn(UnitSpeeds(MoveSpeeds::default()))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(8, foreign);
    dispatch(
        &mut app,
        vec![force(8, 3, 9.0), force(999, 4, 5.0), force(7, 5, 10.0)],
    );
    assert_eq!(app.world().get::<UnitSpeeds>(foreign).unwrap().0.run, 9.0);
    assert!(!app.world().resource::<GuidIndex>().0.contains_key(&999));
    let messages = app.world().resource::<Messages<SpeedChangeMessage>>();
    let counters: Vec<_> = messages
        .iter_current_update_messages()
        .map(|m| m.counter)
        .collect();
    assert_eq!(counters, [1, 2, 5]);
    // Self-by-guid still forwards the required ACK input when the entity is absent.
    app.world_mut().resource_mut::<SelfGuid>().0 = Some(999);
    dispatch(&mut app, vec![force(999, 6, 12.0)]);
    assert_eq!(
        app.world().resource::<Messages<SpeedChangeMessage>>().len(),
        4
    );
}

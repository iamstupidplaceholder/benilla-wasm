//! Runs the installed production player plugin without GPU, sockets or an alternate mover.

use super::*;
use crate::net::*;

pub fn verify_player_lifecycle() {
    let mut app = App::new();
    app.add_plugins(
        benilla_world::boot::tuned_default_plugins(Window::default())
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::WgpuSettings {
                    backends: None,
                    ..default()
                }
                .into(),
                ..default()
            }),
    );
    app.add_plugins(benilla_world::world_plugins::WorldPlugins);
    benilla_assets::register_asset_loaders(&mut app);
    // Retain the real world's physics/query resources, not its GPU startup and
    // streaming schedules. This gate runs only the installed PlayerPlugin's
    // schedules: WorldPlugins cannot open GPU assets in a no-device fixture.
    {
        let mut schedules = app
            .world_mut()
            .resource_mut::<bevy::ecs::schedule::Schedules>();
        schedules.remove(Startup);
        schedules.remove(Update);
    }
    app.insert_state(crate::char_select::ClientState::InWorld)
        .configure_sets(
            Update,
            crate::char_select::InWorldGated
                .run_if(in_state(crate::char_select::ClientState::InWorld)),
        )
        .init_resource::<crate::cvars::Cvars>()
        .init_resource::<crate::char_select::Roster>()
        .init_resource::<crate::loading_screen::LoadingScreen>()
        .init_resource::<crate::loading_screen::EntryCover>()
        .init_resource::<crate::bindings::BindingsState>()
        .init_resource::<crate::ui_cast::LocalMoveStart>()
        .init_resource::<crate::ui_loot::LootMoveStart>()
        .init_resource::<crate::ui_script::InspectMode>()
        .init_resource::<crate::ui_script::UiKeyboardCapture>()
        .init_resource::<crate::ui_script::PlayerUiClickConsumed>()
        .init_resource::<crate::ui_script::PointerOverUi>()
        .init_resource::<crate::camera_shake::CameraShakes>()
        .init_resource::<GuidIndex>()
        .init_resource::<SelfGuid>()
        .add_message::<TeleportMessage>()
        .add_message::<WorldportMessage>()
        .add_message::<SpeedChangeMessage>()
        .add_message::<MoveModeMessage>()
        .add_message::<KnockBackMessage>()
        .add_message::<SelfMoveMessage>()
        .add_message::<ClientControlMessage>()
        .add_message::<LoggedOutMessage>()
        .add_message::<DisconnectedMessage>()
        .add_message::<crate::creature_anim::SheathRequest>()
        .add_message::<crate::creature_anim::HardLanding>()
        .add_message::<crate::creature_anim::MountFlourish>()
        .add_plugins(PlayerPlugin);
    let (sender, receiver) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(sender));
    app.finish();
    app.cleanup();
    app.world_mut().run_schedule(Startup);
    app.world_mut().run_schedule(Update);
    assert!(
        app.world().contains_resource::<PlayerCapsule>(),
        "production startup owns the capsule"
    );
    let mut cameras = app
        .world_mut()
        .query_filtered::<Entity, With<benilla_world::view::WorldCamera>>();
    assert_eq!(cameras.iter(app.world()).count(), 1);

    app.world_mut().resource_mut::<SelfGuid>().0 = Some(7);
    // A streamed body, with the real controller's required component contract.
    let body = app
        .world_mut()
        .spawn((
            SelfPlayer,
            NetEntity {
                kind: benilla_protocol::EntityKind::Player,
                display_id: None,
                scale: 1.0,
            },
            Transform::from_translation(wow_to_bevy([1.0, 2.0, 30.0])),
            UnitSpeeds(benilla_protocol::MoveSpeeds {
                run: 7.0,
                ..default()
            }),
            ObjectStore(benilla_protocol::ObjectFields::default()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(7, body);
    app.world_mut().run_schedule(Update);
    assert!(app.world().get::<Embodied>(body).is_some());
    assert!(app.world().get::<ActiveMover>(body).is_some());
    assert!(app.world().resource::<Player>().active);
    app.world_mut().write_message(SpeedChangeMessage {
        guid: 7,
        kind: benilla_protocol::SpeedKind::Run,
        counter: 42,
        speed: 9.0,
    });
    app.world_mut().run_schedule(Update);
    let commands: Vec<_> = receiver.try_iter().collect();
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, ClientCommand::ForceSpeedAck { counter: 42, .. })),
        "installed production controller must send speed ACK, got {commands:?}"
    );
    app.world_mut().write_message(TeleportMessage {
        guid: 7,
        counter: 43,
        position: [10.0, 20.0, 30.0],
        orientation: 0.5,
    });
    app.world_mut().run_schedule(Update);
    assert_eq!(
        app.world().resource::<Player>().pos,
        wow_to_bevy([10.0, 20.0, 30.0])
    );
    assert!(receiver.try_iter().any(|c| matches!(
        c,
        ClientCommand::TeleportAck {
            guid: 7,
            counter: 43
        }
    )));
    // The real net teardown removes the streamed body and identity before it
    // publishes LoggedOutMessage. Leaving a resolvable body would re-seat the
    // controller in this fixture, unlike an actual ended session.
    app.world_mut().despawn(body);
    app.world_mut().resource_mut::<GuidIndex>().0.clear();
    app.world_mut().resource_mut::<SelfGuid>().0 = None;
    app.world_mut().write_message(LoggedOutMessage);
    app.world_mut().run_schedule(Update);
    assert!(!app.world().resource::<Player>().active);
}

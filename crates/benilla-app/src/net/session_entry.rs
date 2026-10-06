//! Shared world-entry, self-teleport and clock handlers. Transport adapters
//! deliver SessionEvents; neither native nor browser IO applies game state.

use benilla_protocol::{SessionEvent, SessionEventKind};
use bevy::prelude::*;

use super::{
    AddonInfoReply, EnteredWorldMessage, GameTime, NetHandlerApp, NetStatus, SelfGuid, ServerTime,
    ServerWallClock, TeleportMessage,
};
use crate::names::NameCache;

pub(super) fn register(app: &mut App) {
    app.net_handler(SessionEventKind::Connected, connected)
        .net_handler(SessionEventKind::Teleport, teleport)
        .net_handler(SessionEventKind::TimeSpeed, time_speed)
        .net_handler(SessionEventKind::ServerUnixTime, server_unix_time);
}

fn connected(
    In(event): In<SessionEvent>,
    mut addon_reply: ResMut<AddonInfoReply>,
    mut self_guid: ResMut<SelfGuid>,
    mut status: ResMut<NetStatus>,
    mut names: ResMut<NameCache>,
    mut entered_world: MessageWriter<EnteredWorldMessage>,
) {
    let SessionEvent::Connected {
        self_guid: guid,
        name,
        billing_time_rested,
        tutorial_flags,
        addon_info,
    } = event
    else {
        return;
    };
    self_guid.0 = Some(guid);
    status.connected = true;
    status.last_reason = None;
    info!("net: in world as {name} (guid {guid})");
    // Production policy: player/pet names are session scoped, creature templates
    // survive. Seed self and addon verdict before UI sees the entry message.
    names.clear_world_session();
    names.insert_player(guid, name, None);
    addon_reply.0 = addon_info;
    entered_world.write(EnteredWorldMessage {
        billing_time_rested,
        tutorial_flags,
    });
}

fn teleport(
    In(event): In<SessionEvent>,
    self_guid: Res<SelfGuid>,
    mut teleports: MessageWriter<TeleportMessage>,
) {
    let SessionEvent::Teleport {
        guid,
        counter,
        position,
        orientation,
    } = event
    else {
        return;
    };
    if self_guid.0 == Some(guid) {
        teleports.write(TeleportMessage {
            guid,
            counter,
            position,
            orientation,
        });
    }
}

fn time_speed(In(event): In<SessionEvent>, mut server_time: ResMut<ServerTime>) {
    let SessionEvent::TimeSpeed {
        hours,
        minutes,
        day_serial,
        timescale,
    } = event
    else {
        return;
    };
    if server_time.0.is_none() {
        info!("net: server game-time {hours:02}:{minutes:02} (drives lighting)");
    }
    server_time.0 = Some(GameTime::new(hours, minutes, day_serial, timescale));
}

fn server_unix_time(In(event): In<SessionEvent>, mut clock: ResMut<ServerWallClock>) {
    let SessionEvent::ServerUnixTime { unix_time } = event else {
        return;
    };
    if clock.0.is_none() {
        let local = benilla_time::SystemTime::now()
            .duration_since(benilla_time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        info!(
            "net: server wall clock {unix_time} (local clock differs by {}s)",
            local - i64::from(unix_time)
        );
    }
    clock.sample(unix_time);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<AddonInfoReply>()
            .init_resource::<SelfGuid>()
            .init_resource::<NetStatus>()
            .init_resource::<NameCache>()
            .init_resource::<ServerTime>()
            .init_resource::<ServerWallClock>()
            .add_message::<EnteredWorldMessage>()
            .add_message::<TeleportMessage>();
        register(&mut app);
        app
    }

    fn dispatch(app: &mut App, events: Vec<SessionEvent>) {
        super::super::handlers::dispatch(app.world_mut(), events, |_, events| {
            panic!("entry handlers did not claim {events:?}");
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

    #[test]
    fn entry_clears_stale_names_and_addon_verdict() {
        let mut app = app();
        dispatch(
            &mut app,
            vec![entry(1, "First", Some(vec!["Addon".into()]))],
        );
        dispatch(&mut app, vec![entry(2, "Second", None)]);
        let world = app.world();
        let names = world.resource::<NameCache>();
        assert_eq!(names.peek(1), None);
        assert_eq!(names.peek(2), Some("Second"));
        assert_eq!(world.resource::<SelfGuid>().0, Some(2));
        assert!(world.resource::<AddonInfoReply>().0.is_none());
        assert!(world.resource::<NetStatus>().connected);
        assert_eq!(world.resource::<Messages<EnteredWorldMessage>>().len(), 2);
    }

    #[test]
    fn teleport_uses_entry_guid_in_wire_order_and_filters_other_units() {
        let mut app = app();
        let teleport = |guid, counter| SessionEvent::Teleport {
            guid,
            counter,
            position: [1.0, 2.0, 3.0],
            orientation: 0.5,
        };
        dispatch(
            &mut app,
            vec![
                teleport(1, 0),
                entry(1, "Self", None),
                teleport(2, 1),
                teleport(1, 2),
            ],
        );
        let messages = app.world().resource::<Messages<TeleportMessage>>();
        let mut events = messages.iter_current_update_messages();
        assert_eq!(events.next().unwrap().counter, 2);
        assert!(events.next().is_none());
    }

    #[test]
    fn both_server_clocks_are_owned_and_populated() {
        let mut app = app();
        dispatch(
            &mut app,
            vec![
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
        assert!(app.world().resource::<ServerTime>().0.is_some());
        assert!(app.world().resource::<ServerWallClock>().0.is_some());
        assert_eq!(
            app.world()
                .resource::<super::super::handlers::NetHandlers>()
                .census()
                .len(),
            4
        );
    }
}

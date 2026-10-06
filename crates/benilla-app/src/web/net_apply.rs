//! Browser admission point for the production network model.
//!
//! Native currently drains every `SessionEvent` through one large system which also owns all
//! FrameScript windows and audio queues. Those consumers are not wasm-ready yet. This backend
//! keeps the production channel, components and messages, but applies only the world/session core;
//! unsupported UI events remain decoded and can be admitted as their plugins are ported.

use benilla_protocol::SessionEvent;
use bevy::prelude::*;

use super::{
    CharActionResultMessage, CharListMessage, CharacterLoginFailedMessage, DisconnectedMessage,
    DropTally, DroppedOpcodes, GuidIndex, LoginFailedMessage, LoginQueuedMessage,
    LoginStageMessage, NetEvents, NetStatus, PendingTransfer, PendingTransferInfo,
    RealmListMessage, SelfGuid, WorldportMessage,
};

pub(crate) fn apply_net_updates(world: &mut World) {
    let events: Vec<_> = world.resource::<NetEvents>().0.try_iter().collect();
    super::handlers::dispatch(world, events, |world, unclaimed| {
        for event in unclaimed {
            apply_event(world, event);
        }
    });
}

fn apply_event(world: &mut World, event: SessionEvent) {
    match event {
        SessionEvent::LoginStage { stage } => {
            world.write_message(LoginStageMessage { stage });
        }
        SessionEvent::LoginQueued { position, realm } => {
            world.write_message(LoginQueuedMessage { position, realm });
        }
        SessionEvent::LoginFailed {
            refusal,
            reason,
            terminal,
            dial,
        } => {
            world.write_message(LoginFailedMessage {
                refusal,
                reason,
                terminal,
                dial,
            });
        }
        SessionEvent::RealmList { realms } => {
            world.write_message(RealmListMessage { realms });
        }
        SessionEvent::CharacterList { characters, realm } => {
            let mut status = world.resource_mut::<NetStatus>();
            status.last_reason = None;
            world.write_message(CharListMessage { characters, realm });
        }
        SessionEvent::CharActionResult { action, code } => {
            world.write_message(CharActionResultMessage { action, code });
        }
        SessionEvent::CharacterLoginFailed { result } => {
            world.write_message(CharacterLoginFailedMessage { result });
        }
        SessionEvent::Worldport {
            map_id,
            position,
            orientation,
            needs_ack,
        } => {
            clear_streamed(world);
            let transport_entry =
                world
                    .resource_mut::<PendingTransfer>()
                    .0
                    .take()
                    .and_then(|pending| {
                        if pending.map_id == map_id {
                            pending.transport_entry
                        } else {
                            None
                        }
                    });
            world.write_message(WorldportMessage {
                map_id,
                position,
                orientation,
                needs_ack,
                transport_entry,
            });
        }
        SessionEvent::TransferPending {
            map_id,
            transport_entry,
        } => {
            world.resource_mut::<PendingTransfer>().0 = Some(PendingTransferInfo {
                map_id,
                transport_entry,
            });
        }
        SessionEvent::TransferAborted { .. } => {
            world.resource_mut::<PendingTransfer>().0 = None;
        }
        SessionEvent::ClientControl { mover, allow_move } => {
            world.write_message(super::ClientControlMessage { mover, allow_move });
        }
        SessionEvent::PacketDropped {
            opcode,
            unparseable,
        } => {
            let mut dropped = world.resource_mut::<DroppedOpcodes>();
            let tally = dropped.0.entry(opcode).or_insert_with(DropTally::default);
            if unparseable {
                tally.unparseable += 1;
            } else {
                tally.unknown += 1;
            }
        }
        SessionEvent::Disconnected { reason, end } => {
            clear_streamed(world);
            world.resource_mut::<SelfGuid>().0 = None;
            {
                let mut status = world.resource_mut::<NetStatus>();
                status.connected = false;
                status.last_reason = Some(reason.clone());
            }
            world.write_message(DisconnectedMessage::new(reason, end));
        }
        SessionEvent::LoggedOut => {
            clear_streamed(world);
            world.resource_mut::<SelfGuid>().0 = None;
            world.write_message(super::LoggedOutMessage);
        }
        _ => {}
    }
}

fn clear_streamed(world: &mut World) {
    let entities: Vec<_> = world
        .resource_mut::<GuidIndex>()
        .0
        .drain()
        .map(|(_, entity)| entity)
        .collect();
    for entity in entities {
        let _ = world.despawn(entity);
    }
}

//! Intentional QoL, not a claim of vanilla parity: stop auto-repeat when the
//! selected target's replicated health already establishes that it is a corpse,
//! rather than waiting for the server to validate the next scheduled shot.

use bevy::prelude::*;

use crate::net::{NetCommands, ObjectStore, SelfPlayer};
use crate::target::Selection;

use super::AutoRepeatActive;

pub(super) fn stop_on_dead_target(
    selection: Res<Selection>,
    stores: Query<&ObjectStore>,
    player: Query<Entity, With<SelfPlayer>>,
    mut auto_repeat: ResMut<AutoRepeatActive>,
    mut commands: Commands,
    net: Res<NetCommands>,
) {
    if auto_repeat.0.is_none() {
        return;
    }
    // Use actual health, not the dead-looking/feign flags. Missing snapshots
    // are not evidence of death; unit_is_dead guards on a known max health.
    let dead = selection
        .target
        .and_then(|e| stores.get(e).ok())
        .is_some_and(|s| s.0.unit_is_dead());
    if !dead {
        return;
    }
    let Ok(entity) = player.single() else {
        return;
    };
    debug!("ui_action: QoL auto-repeat cancel — selected target is dead");
    // Reuse the full cancellation: checked/flash state, load/hold idle,
    // nocked ammo, and the protocol command. Do not touch launched missiles,
    // change selection, sheath the weapon, or synthesize a cast error.
    crate::creature_anim::cancel_auto_repeat_local(
        Some(entity),
        &mut auto_repeat,
        &mut commands,
        &net,
    );
}

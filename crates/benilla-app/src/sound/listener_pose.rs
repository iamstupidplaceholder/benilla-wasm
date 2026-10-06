//! Original listener-pose policy, shared by native and browser host outputs.
use crate::player::{head_height, CameraPivot, Player};
use bevy::prelude::*;
pub(super) fn pose(
    at_character: bool,
    player: &Player,
    cinematic: Option<&crate::cinematic::Cinematic>,
    avatar: Option<(&Transform, Option<&CameraPivot>)>,
    camera: Option<&Transform>,
) -> Option<(Vec3, Quat)> {
    let flying = cinematic.is_some_and(crate::cinematic::Cinematic::is_playing);
    if at_character && player.active && !player.detached && !flying {
        if let Some((t, pivot)) = avatar {
            return Some((
                player.pos + Vec3::Y * head_height(pivot, t.scale.x),
                Quat::from_rotation_y(player.facing()),
            ));
        }
    }
    camera.map(|t| (t.translation, t.rotation))
}

pub(super) fn worn_chest_material(
    store: Option<&crate::net::ObjectStore>,
    items: &crate::items::Items,
    net: &crate::net::NetCommands,
) -> Option<u32> {
    const EQUIPMENT_SLOT_CHEST: u8 = 4;
    let guid = store?.0.player_inv_slot(EQUIPMENT_SLOT_CHEST)?;
    let entry = items.object(guid)?.object_entry()?;
    Some(items.held(entry, net)?.material)
}

//! Restore world lighting after the transitional browser glue presentation.
//! Camera, worldport pose and streaming focus belong to the production PlayerPlugin.

use bevy::prelude::*;

use benilla_world::{lighting::SharedLightBuffer, schedule::WorldStage};

use crate::net::WorldportMessage;

pub(crate) struct WebWorldLightPlugin;

impl Plugin for WebWorldLightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, restore_world_light.in_set(WorldStage::Input));
    }
}

fn restore_world_light(
    mut commands: Commands,
    mut worldports: MessageReader<WorldportMessage>,
    shared_light: Option<Res<SharedLightBuffer>>,
) {
    for _ in worldports.read() {
        // The glue stage publishes its authored studio table while it is visible. At entry the
        // ordinary world model lane must return to the table LightingPlugin updates each frame.
        if let Some(light) = shared_light.as_deref() {
            benilla_world::model_render::publish_off_world_model_light(
                &mut commands,
                light.0.clone(),
            );
        }
    }
}

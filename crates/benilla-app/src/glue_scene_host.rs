//! Renderer-facing host for the production glue scene.
//!
//! The native portrait subsystem and the browser composition build this host
//! differently, but the authored scene runtime only needs these identities.
//! Keeping the host free of portrait/gameplay state lets the existing
//! `sync_glue_scene` move across the platform boundary unchanged.

use bevy::{camera::visibility::RenderLayers, prelude::*};

/// Render objects owned by the fullscreen glue scene.
#[derive(Resource)]
pub(crate) struct GlueSceneHost {
    pub(crate) target: Handle<Image>,
    pub(crate) layer: RenderLayers,
    pub(crate) character_root: Entity,
    pub(crate) scene_root: Entity,
    pub(crate) pet_root: Entity,
}

/// Identifies the one camera rendering [`GlueSceneHost`].
#[derive(Component)]
pub(crate) struct GlueSceneCamera;

/// A collapsed rig rendered by an off-world stage rather than the world view.
/// Its parking authority is the stage camera, not world-frustum policy.
#[derive(Component)]
pub(crate) struct StageRig;

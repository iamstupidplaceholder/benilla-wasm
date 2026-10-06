//! The renderer slice needed by a live M2 scene outside the streamed world.
//!
//! Login/create/select backgrounds, model panes and similar stages still use the production M2
//! renderer.  They do not, however, own a map, physics world, terrain streamer or gameplay
//! camera.  This plugin is the narrow composition seam for that case: the same material, rig,
//! palette and material-animation machinery as [`crate::world_plugins::WorldPlugins`], without
//! making an off-world host manufacture the rest of a world merely to satisfy system parameters.

use benilla_assets::materials::WowModelMaterial;
use bevy::{pbr::MaterialPlugin, prelude::*};

/// Production M2 rendering for a live, off-world stage.
pub struct OffWorldModelPlugin;

impl Plugin for OffWorldModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WowModelMaterial>::default())
            .add_plugins(crate::billboard::BillboardPlugin)
            .add_plugins(crate::rig_palette::plugin)
            .add_plugins(crate::rig_anim::plugin)
            .add_plugins(crate::instance_tint::plugin)
            .add_plugins(crate::mat_anim_table::plugin)
            .init_resource::<crate::model_render::ModelMaterials>()
            .init_resource::<crate::model_render::FarSideTwins>()
            .init_resource::<crate::model_forms::ModelForms>();

        crate::doodad_anim::off_world_material_animation(app);
        crate::model_render::off_world_material_lifecycle(app);
    }
}

/// Production M2 particle simulation and effect rendering for an off-world stage.
///
/// This deliberately installs the ordinary [`crate::particles::ParticlePlugin`]. Its world-only
/// inputs (portal/water gates and terrain collision) are optional service adapters inside the
/// simulator, while layered booth cameras remain first-class particle views. Consequently an
/// off-world host does not need to fabricate a map, physics world or gameplay camera, and it does
/// not get a second particle implementation.
pub struct OffWorldParticlePlugin;

impl Plugin for OffWorldParticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::particles::ParticlePlugin);
    }
}

/// Complete production renderer for a scene which is not attached to the streamed world.
///
/// This is the composition root browser glue scenes use: it registers the engine shaders once,
/// then installs the ordinary model, particle and FFX implementations behind their off-world
/// service seams.
pub struct OffWorldScenePlugin;

impl Plugin for OffWorldScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::shaders::plugin)
            .add_plugins(OffWorldModelPlugin)
            .add_plugins(OffWorldParticlePlugin)
            .add_plugins(crate::ffx_glow::FfxGlowPlugin);
    }
}

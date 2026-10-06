//! WebGL2-compatible counterparts of the production terrain and model materials.
//!
//! They deliberately keep the production extension binding numbers and texture shapes, while
//! replacing the unbounded shared storage buffer with a compact uniform. Native materials remain
//! untouched; these types are the browser renderer's compatibility lane.

use bevy::{
    camera::primitives::MeshAabb,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, Face, ShaderType},
    shader::ShaderRef,
};

use crate::{submesh_to_static_mesh, ModelBlend, ModelSubmesh};

pub type WebTerrainMaterial = ExtendedMaterial<StandardMaterial, WebTerrainExtension>;
pub type WebModelMaterial = ExtendedMaterial<StandardMaterial, WebModelExtension>;

/// Prefix of the production light buffer that terrain and the first model fallback actually need.
/// Every field is a `vec4`, keeping the uniform layout valid on WebGL2 without extra padding.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct WebLightUniform {
    pub ambient: Vec4,
    pub diffuse: Vec4,
    pub sun: Vec4,
    pub specular: Vec4,
    pub fog_color: Vec4,
    pub fog_params: Vec4,
}

impl Default for WebLightUniform {
    fn default() -> Self {
        Self {
            ambient: Vec4::new(0.28, 0.32, 0.38, 1.0),
            diffuse: Vec4::new(0.82, 0.68, 0.48, 1.0),
            sun: Vec4::new(0.35, -0.82, 0.45, 1.0),
            specular: Vec4::new(0.55, 0.50, 0.40, 18.0),
            fog_color: Vec4::new(0.18, 0.16, 0.23, 1.0),
            fog_params: Vec4::new(95.0, 235.0, 0.0, 280.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Clone, TypePath)]
pub struct WebTerrainExtension {
    #[uniform(90)]
    pub light: WebLightUniform,
    #[texture(100, dimension = "2d_array", visibility(fragment))]
    #[sampler(105, visibility(fragment))]
    pub layer_array: Handle<Image>,
    #[texture(104, dimension = "2d_array", visibility(fragment))]
    pub alpha_array: Handle<Image>,
    #[texture(110, dimension = "2d_array", visibility(fragment))]
    pub shadow_array: Handle<Image>,
    #[uniform(106)]
    pub params: Vec4,
}

impl MaterialExtension for WebTerrainExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://benilla_assets/shaders/web_terrain.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://benilla_assets/shaders/web_terrain.wgsl".into()
    }
}

#[derive(Asset, AsBindGroup, Clone, TypePath)]
pub struct WebModelExtension {
    #[uniform(90)]
    pub light: WebLightUniform,
    /// RGB material tint; A is the compact WebGL lane's lighting mix (0 = fullbright, 1 = lit).
    #[uniform(100)]
    pub tint: Vec4,
    /// XY texture translation; ZW reserved for the later affine texture animation.
    #[uniform(100)]
    pub uv_anim: Vec4,
}

impl MaterialExtension for WebModelExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://benilla_assets/shaders/web_model.wgsl".into()
    }
}

/// Register only the compatibility shaders needed by the browser probe.
pub fn register_web_shaders(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/web_terrain.wgsl");
    bevy::asset::embedded_asset!(app, "shaders/web_model.wgsl");
}

/// Spawn one decoded model batch through the WebGL2 compatibility probe lane.
///
/// This is deliberately not a scene-runtime API.  The temporary login backdrop
/// smoke check uses it only to prove gateway M2/BLP delivery, authored-camera
/// framing and render-target composition.  Production glue animation remains
/// owned by `benilla-app::portrait::glue_booth`; WebGL support belongs under its
/// `WowModelMaterial`/palette/material-animation/effect renderer services.
pub fn spawn_web_model_batch(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WebModelMaterial>,
    submesh: &ModelSubmesh,
    transform: Transform,
    name: String,
) -> Entity {
    let mesh = submesh_to_static_mesh(&submesh.geometry);
    let bounds = mesh.compute_aabb();
    let alpha_mode = match submesh.blend {
        ModelBlend::Opaque => AlphaMode::Opaque,
        ModelBlend::AlphaTest => AlphaMode::Mask(crate::materials::VANILLA_ALPHA_KEY_REF),
        ModelBlend::Blend if submesh.additive => AlphaMode::Add,
        ModelBlend::Blend => AlphaMode::Blend,
        ModelBlend::Mod | ModelBlend::Mod2x => AlphaMode::Blend,
    };
    let mut light = WebLightUniform::default();
    light.fog_params = Vec4::new(300.0, 760.0, 0.0, 920.0);
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: submesh.texture.clone(),
            alpha_mode,
            double_sided: submesh.two_sided,
            cull_mode: (!submesh.two_sided).then_some(Face::Back),
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: WebModelExtension {
            light,
            tint: Vec4::new(1.0, 1.0, 1.0, if submesh.emissive { 0.0 } else { 1.0 }),
            uv_anim: Vec4::ZERO,
        },
    });
    let entity = commands
        .spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            transform,
            Name::new(name),
        ))
        .id();
    if let Some(bounds) = bounds {
        commands.entity(entity).insert(bounds);
    }
    entity
}

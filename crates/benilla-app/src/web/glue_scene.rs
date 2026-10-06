//! Browser host for the production glue-scene renderer.
//!
//! This module owns only the platform-facing objects: a WebGL-compatible render target, one
//! camera and the three stable scene roots. The actual `UI_*` model selection, authored camera,
//! lighting, rig animation, material animation and model spawning are the unchanged production
//! systems from `portrait/glue_booth.rs` and `portrait/booth.rs`.

use bevy::{
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, ClearColorConfig, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::Hdr,
    },
};

use crate::{
    glue_scene_host::{GlueSceneCamera, GlueSceneHost},
    portrait::{
        face_booth_billboards, push_booth_mat_alpha, sync_glue_booth_web, sync_glue_pet_web,
        CreateScene, PortraitImages, PortraitSource, GLUE_SLOT,
    },
    portrait::{pillarbox_glue_scene, sync_glue_scene},
};

const GLUE_RENDER_LAYER: usize = 30;
const INITIAL_SIZE: UVec2 = UVec2::new(1024, 768);

pub(crate) struct WebGlueScenePlugin;

impl Plugin for WebGlueScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::entities::WebAppearancePlugin)
            .init_resource::<CreateScene>()
            .add_systems(Startup, setup_glue_scene)
            .add_systems(
                Update,
                (
                    sync_glue_scene,
                    pillarbox_glue_scene,
                    sync_glue_booth_web,
                    sync_glue_pet_web,
                )
                    .chain()
                    .after(crate::entities::WebAppearanceReady),
            )
            .add_systems(Update, face_booth_billboards)
            .add_systems(
                Update,
                push_booth_mat_alpha
                    .after(benilla_world::doodad_anim::sample_mat_anim),
            );
    }
}

fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0; 8],
        // Match the production glue booth's unencoded float intermediate. FFXGlow owns the
        // gamma-byte combine and its single decode; an sRGB target before that pass decodes the
        // scene too early and is the observed over-bright browser result.
        TextureFormat::Rgba16Float,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image
}

fn setup_glue_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut portraits: ResMut<PortraitImages>,
    window: Query<&Window>,
) {
    let size = window
        .single()
        .map(|window| {
            UVec2::new(
                window.physical_width().max(1),
                window.physical_height().max(1),
            )
        })
        .unwrap_or(INITIAL_SIZE);
    let target = images.add(target_image(size));
    portraits
        .0
        .insert(GLUE_SLOT.to_owned(), PortraitSource::Live(target.clone()));

    let layer = RenderLayers::layer(GLUE_RENDER_LAYER);
    let character_root = commands
        .spawn((Transform::IDENTITY, Visibility::Visible, layer.clone()))
        .id();
    let scene_root = commands
        .spawn((Transform::IDENTITY, Visibility::Visible, layer.clone()))
        .id();
    let pet_root = commands
        .spawn((Transform::IDENTITY, Visibility::Visible, layer.clone()))
        .id();

    commands.spawn((
        Camera3d::default(),
        Hdr,
        Camera {
            order: -10,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: bevy::camera::CameraOutputMode::Write {
                blend_state: None,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
            },
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        Tonemapping::None,
        Msaa::Off,
        bevy::light::cluster::ClusterConfig::None,
        benilla_world::ffx_glow::FfxGlow::GLUE_SCENE,
        layer.clone(),
        GlueSceneCamera,
        Name::new("browser production glue-scene camera"),
    ));
    commands.insert_resource(GlueSceneHost {
        target,
        layer,
        character_root,
        scene_root,
        pet_root,
    });
}

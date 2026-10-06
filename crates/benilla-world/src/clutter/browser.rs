//! Async host inputs for the original scatter, chunk merge and material builder.
use super::*;
use benilla_assets::{MaterialKey, RawFile, SpatialCache};
use bevy::{asset::LoadState, ecs::system::SystemParam};
use std::collections::HashSet;

const FILES: &[&str] = &[
    "DBFilesClient/GroundEffectTexture.dbc",
    "DBFilesClient/GroundEffectDoodad.dbc",
];

#[derive(Resource)]
struct CatalogDelivery(Vec<(&'static str, Handle<RawFile>)>);

#[derive(Resource, Default)]
pub(super) struct Inputs {
    models: HashMap<String, Handle<RawFile>>,
    textures: SpatialCache<String, Handle<Image>>,
    failed: HashSet<String>,
    materials: SpatialCache<MaterialKey, Handle<WowModelMaterial>>,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ClutterGeometry>()
        .init_resource::<Inputs>()
        .add_systems(Startup, request)
        .add_systems(
            Update,
            (publish, evict, scope_inputs)
                .chain()
                .before(stream_chunk_clutter),
        );
}

fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CatalogDelivery(
        FILES
            .iter()
            .map(|&p| (p, server.load(format!("mpq://{p}"))))
            .collect(),
    ));
}

fn publish(
    mut commands: Commands,
    delivery: Option<Res<CatalogDelivery>>,
    files: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
) {
    let Some(delivery) = delivery else { return };
    if delivery.0.iter().any(|(_, h)| {
        files.get(h).is_none()
            && !matches!(server.get_load_state(h.id()), Some(LoadState::Failed(_)))
    }) {
        return;
    }
    let mut source =
        benilla_formats::MemoryFileSource::new(delivery.0.iter().filter_map(|(p, h)| {
            match files.get(h) {
                Some(f) => Some((*p, f.0.clone())),
                None => {
                    warn!(
                        "clutter: delivery failed for {p}: {:?}",
                        server.get_load_state(h.id())
                    );
                    None
                }
            }
        }));
    match load_ground_effect_catalog(&mut source, true) {
        Ok(catalog) => {
            commands.insert_resource(GroundClutter { catalog });
        }
        Err(error) => warn!("clutter: catalog unavailable: {error:#}"),
    }
    commands.remove_resource::<CatalogDelivery>();
}

fn evict(mut changes: MessageReader<crate::world_map::MapChange>, mut inputs: ResMut<Inputs>) {
    if !changes.is_empty() {
        changes.clear();
        *inputs = Inputs::default();
    }
}

fn scope_inputs(mut scope: crate::art_scope::ArtScope, mut inputs: ResMut<Inputs>) {
    scope.apply(&mut inputs.textures, crate::art_scope::ArtSlot::ClutterGeo);
    scope.apply(&mut inputs.materials, crate::art_scope::ArtSlot::ClutterGeo);
}

#[derive(SystemParam)]
pub(super) struct Host<'w> {
    inputs: ResMut<'w, Inputs>,
    models: Res<'w, Assets<RawFile>>,
    server: Res<'w, AssetServer>,
    light: Res<'w, crate::model_render::SharedModelLight>,
}

impl Host<'_> {
    pub(super) fn ready(
        &mut self,
        wanted: &[(String, Vec<ShadedPlacement>)],
        geometry: &mut ClutterGeometry,
        images: &Assets<Image>,
    ) -> bool {
        let mut ready = true;
        for (path, _) in wanted {
            let subs = if let Some(subs) = geometry.0.fetch(path) {
                subs
            } else {
                if self.inputs.failed.contains(path) {
                    continue;
                }
                let handle = self
                    .inputs
                    .models
                    .entry(path.clone())
                    .or_insert_with(|| self.server.load(benilla_assets::m2_url(path)))
                    .clone();
                if let Some(model) = self.models.get(&handle) {
                    let dir = path.rfind(['\\', '/']).map(|i| &path[..i]).unwrap_or("");
                    let subs = match benilla_formats::parse_m2_render_submeshes(&model.0, dir, &[])
                    {
                        Ok(subs) => subs,
                        Err(error) => {
                            warn!("clutter: model {path} decode failed: {error:#}");
                            self.inputs.failed.insert(path.clone());
                            self.inputs.models.remove(path);
                            continue;
                        }
                    };
                    geometry.0.insert(path.clone(), subs.clone());
                    self.inputs.models.remove(path);
                    subs
                } else if let Some(LoadState::Failed(error)) =
                    self.server.get_load_state(handle.id())
                {
                    warn!("clutter: model {path} delivery/decode failed: {error}");
                    self.inputs.failed.insert(path.clone());
                    self.inputs.models.remove(path);
                    continue;
                } else {
                    ready = false;
                    continue;
                }
            };
            for sub in &subs {
                let Some(texture) = sub.texture.as_deref() else {
                    continue;
                };
                let url = benilla_assets::texture_url(texture, (sub.wrap_x, sub.wrap_y));
                if self.inputs.failed.contains(&url) {
                    continue;
                }
                let handle = self
                    .inputs
                    .textures
                    .or_insert_with(url.clone(), || self.server.load(&url))
                    .clone();
                if images.get(&handle).is_none() {
                    if let Some(LoadState::Failed(error)) = self.server.get_load_state(handle.id())
                    {
                        warn!("clutter: texture {texture} delivery/decode failed: {error}");
                        self.inputs.failed.insert(url);
                    } else {
                        ready = false;
                    }
                }
            }
        }
        ready
    }

    pub(super) fn model_material(
        &mut self,
        texture: Option<&str>,
        blend: ModelBlend,
        two_sided: bool,
        alpha_cutoff: Option<f32>,
        fade_far: Option<f32>,
        is_wmo: bool,
        is_fade_variant: bool,
        wrap: (bool, bool),
        _images: &mut Assets<Image>,
        materials: &mut Assets<WowModelMaterial>,
    ) -> Handle<WowModelMaterial> {
        let resolved = texture.and_then(|path| {
            let url = benilla_assets::texture_url(path, wrap);
            if self.inputs.failed.contains(&url) {
                return None;
            }
            self.inputs
                .textures
                .fetch(&url)
                .map(|h| (benilla_assets::normalize_path(path), h))
        });
        WorldAssets::resolved_model_material(
            &mut self.inputs.materials,
            &self.light.0,
            resolved,
            blend,
            two_sided,
            alpha_cutoff,
            fade_far,
            is_wmo,
            is_fade_variant,
            wrap,
            materials,
        )
    }
}

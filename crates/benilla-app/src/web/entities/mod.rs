//! Browser composition of the production off-world appearance pipeline.
//!
//! DBC files arrive asynchronously through the same gateway-backed `mpq://` source as models and
//! textures. Once present, the unchanged catalog decoders and display/preview builders take over.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use benilla_assets::{M2Model, RawFile, RawFilePlugin, WmoModel};
use benilla_formats::{CharCreateCatalog, CharSections, CharacterGeosets, MemoryFileSource};
use bevy::{asset::LoadState, prelude::*};

use super::{
    attach::{build_glue_pet, build_glue_preview, glue_preview_spec},
    display::{build_parts, new_creature_display, DisplayModel, ModelHandle},
    equipment::ItemDisplays,
    CharCreate, Characters, Creatures, SkinComposites, SkinKey, SkinSections,
};

use crate::portrait::{GluePetBake, GluePreview, GluePreviewBake};

const DBC_FILES: &[&str] = &[
    "DBFilesClient/SpellItemEnchantment.dbc",
    "DBFilesClient/ItemVisuals.dbc",
    "DBFilesClient/ItemVisualEffects.dbc",
    "DBFilesClient/AnimationData.dbc",
    "DBFilesClient/SpellVisual.dbc",
    "DBFilesClient/SpellVisualKit.dbc",
    "DBFilesClient/SpellVisualEffectName.dbc",
    "DBFilesClient/SpellChainEffects.dbc",
    "DBFilesClient/UnitBlood.dbc",
    "DBFilesClient/UnitBloodLevels.dbc",
    "DBFilesClient/EnvironmentalDamage.dbc",
    "DBFilesClient/CreatureModelData.dbc",
    "DBFilesClient/CreatureDisplayInfo.dbc",
    "DBFilesClient/CreatureDisplayInfoExtra.dbc",
    "DBFilesClient/ChrRaces.dbc",
    "DBFilesClient/CharBaseInfo.dbc",
    "DBFilesClient/CharStartOutfit.dbc",
    "DBFilesClient/CharSections.dbc",
    "DBFilesClient/CharHairGeosets.dbc",
    "DBFilesClient/CharacterFacialHairStyles.dbc",
    "DBFilesClient/HelmetGeosetVisData.dbc",
    "DBFilesClient/ItemDisplayInfo.dbc",
    "DBFilesClient/Map.dbc",
    "DBFilesClient/Light.dbc",
    "DBFilesClient/LightIntBand.dbc",
    "DBFilesClient/LightFloatBand.dbc",
    "DBFilesClient/LightParams.dbc",
    "DBFilesClient/LightSkybox.dbc",
];

#[derive(Resource)]
struct CatalogLoads(Vec<(&'static str, Handle<RawFile>)>);

#[derive(Resource)]
struct CatalogFailure(String);

#[derive(Default)]
struct CatalogTrace {
    waiting_frames: u16,
    reported_wait: bool,
}

#[derive(Default)]
struct DisplayTrace {
    built: HashSet<u32>,
    missing_bodies: HashSet<(u8, u8)>,
    waiting_frames: HashMap<u32, u16>,
}

#[derive(Default)]
struct RequestTrace {
    last: Option<(crate::portrait::GlueLook, u8)>,
}

struct SkinLoadJob {
    groups: Vec<SkinLoadGroup>,
}

struct SkinLoadGroup {
    candidates: Vec<String>,
    next: usize,
    file: Option<(String, Handle<RawFile>)>,
    done: bool,
}

#[derive(Default)]
struct SkinLoadJobs {
    active: HashMap<SkinKey, SkinLoadJob>,
    failed: HashSet<SkinKey>,
}

pub(crate) struct WebAppearancePlugin;

#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct WebAppearanceReady;

impl Plugin for WebAppearancePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RawFilePlugin)
            .init_resource::<SkinComposites>()
            .init_resource::<GluePreviewBake>()
            .init_resource::<GluePetBake>()
            .add_systems(Startup, request_catalogs)
            .add_systems(
                Update,
                (
                    publish_catalogs,
                    trace_appearance_request,
                    prefetch_skin_composite,
                    update_preview_display_models,
                    (build_glue_preview, build_glue_pet),
                )
                    .chain()
                    .in_set(WebAppearanceReady),
            );
    }
}

/// Fetch the exact BLP inputs selected by the production body-compositor plan, then execute that
/// same compositor over the resident bytes. Only delivery is browser-specific; atlas layout,
/// layer order, equipment fallbacks and mip blending remain `CharSections::composite_body`.
#[allow(clippy::too_many_arguments)]
fn prefetch_skin_composite(
    preview: Res<GluePreview>,
    char_create: Option<Res<CharCreate>>,
    sections: Option<Res<SkinSections>>,
    displays: Option<Res<ItemDisplays>>,
    asset_server: Res<AssetServer>,
    raw_files: Res<Assets<RawFile>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<SkinComposites>,
    mut jobs: Local<SkinLoadJobs>,
) {
    let (Some(sections), Some(displays)) = (sections.as_deref(), displays.as_deref()) else {
        return;
    };
    if let Some(spec) = preview
        .look
        .zip(char_create.as_deref())
        .and_then(|(look, create)| glue_preview_spec(look, create))
    {
        composites.1.insert(spec.skin_key());
    }
    let mut keys = std::mem::take(&mut composites.1);
    keys.extend(jobs.active.keys().copied());
    for key in keys {
        deliver_skin_composite(
            key,
            sections,
            displays,
            &asset_server,
            &raw_files,
            &mut images,
            &mut composites,
            &mut jobs,
        );
    }
}

/// The same delivery/compositor path serves preview and live consumer requests.
#[allow(clippy::too_many_arguments)]
fn deliver_skin_composite(
    key: SkinKey,
    sections: &SkinSections,
    displays: &ItemDisplays,
    asset_server: &AssetServer,
    raw_files: &Assets<RawFile>,
    images: &mut Assets<Image>,
    composites: &mut SkinComposites,
    jobs: &mut SkinLoadJobs,
) {
    if composites.0.fetch(&key).is_some() || jobs.failed.contains(&key) {
        return;
    }

    if !jobs.active.contains_key(&key) {
        let body_slots = key.equip;
        let worn = body_slots.map(|display_id| {
            if display_id == 0 {
                None
            } else {
                displays.catalog.get(display_id)
            }
        });
        let groups = sections.0.body_texture_candidates(
            key.race,
            key.sex,
            key.skin,
            key.face,
            key.facial_hair,
            key.hair_style,
            key.hair_color,
            worn,
            key.emblem,
            key.tabard_preview,
        );
        if groups.is_empty() {
            error!(
                "browser body atlas: no production texture plan for race {}, sex {}, skin {}",
                key.race, key.sex, key.skin
            );
            jobs.failed.insert(key);
            return;
        }
        info!(
            "browser body atlas: requesting {} source layer(s) for race {}, sex {}",
            groups.len(),
            key.race,
            key.sex
        );
        let groups = groups
            .into_iter()
            .map(|candidates| {
                let path = candidates[0].clone();
                let url = format!("mpq://{}", path.replace('\\', "/").to_ascii_lowercase());
                let handle = asset_server.load::<RawFile>(url);
                SkinLoadGroup {
                    candidates,
                    next: 1,
                    file: Some((path, handle)),
                    done: false,
                }
            })
            .collect();
        jobs.active.insert(key, SkinLoadJob { groups });
        return;
    }

    let Some(job) = jobs.active.get_mut(&key) else {
        return;
    };
    let mut waiting = false;
    for group in &mut job.groups {
        if group.done {
            continue;
        }
        let Some((_, handle)) = group.file.as_ref() else {
            group.done = true;
            continue;
        };
        match asset_server.load_state(handle) {
            LoadState::NotLoaded | LoadState::Loading => waiting = true,
            LoadState::Loaded => group.done = true,
            LoadState::Failed(_) => {
                if let Some(path) = group.candidates.get(group.next).cloned() {
                    group.next += 1;
                    let url = format!("mpq://{}", path.replace('\\', "/").to_ascii_lowercase());
                    group.file = Some((path, asset_server.load::<RawFile>(url)));
                    waiting = true;
                } else {
                    group.file = None;
                    group.done = true;
                }
            }
        }
    }
    if waiting || job.groups.iter().any(|group| !group.done) {
        return;
    }

    let mut source = MemoryFileSource::new(
        job.groups
            .iter()
            .filter_map(|group| group.file.as_ref())
            .filter_map(|(path, handle)| {
                raw_files
                    .get(handle)
                    .map(|file| (path.clone(), file.0.clone()))
            }),
    );
    let body_slots = key.equip;
    let worn = body_slots.map(|display_id| {
        if display_id == 0 {
            None
        } else {
            displays.catalog.get(display_id)
        }
    });
    let composed = sections.0.composite_body(
        &mut source,
        key.race,
        key.sex,
        key.skin,
        key.face,
        key.facial_hair,
        key.hair_style,
        key.hair_color,
        worn,
        key.emblem,
        key.tabard_preview,
    );
    match composed {
        Ok(Some(atlas)) => {
            let image = benilla_assets::repeat_texture_authored(
                benilla_assets::for_upload(atlas),
                (true, true),
            );
            let handle = images.add(image);
            composites.0.insert(key, handle);
            jobs.active.remove(&key);
            info!(
                "browser body atlas: race {}, sex {} composite ready",
                key.race, key.sex
            );
        }
        Ok(None) => {
            error!(
                "browser body atlas: production compositor found no base for race {}, sex {}",
                key.race, key.sex
            );
            jobs.active.remove(&key);
            jobs.failed.insert(key);
        }
        Err(error) => {
            error!(
                "browser body atlas: composite failed for race {}, sex {}: {error:#}",
                key.race, key.sex
            );
            jobs.active.remove(&key);
            jobs.failed.insert(key);
        }
    }
}

fn request_catalogs(mut commands: Commands, assets: Res<AssetServer>) {
    info!(
        "browser appearance: requesting {} production DBC catalogs",
        DBC_FILES.len()
    );
    commands.insert_resource(CatalogLoads(
        DBC_FILES
            .iter()
            .map(|&path| (path, assets.load(format!("mpq://{path}"))))
            .collect(),
    ));
}

fn publish_catalogs(
    mut commands: Commands,
    loads: Option<Res<CatalogLoads>>,
    files: Res<Assets<RawFile>>,
    assets: Res<AssetServer>,
    ready: Option<Res<CharCreate>>,
    mut trace: Local<CatalogTrace>,
) {
    if ready.is_some() {
        return;
    }
    let Some(loads) = loads else { return };
    for (path, handle) in &loads.0 {
        if let LoadState::Failed(error) = assets.load_state(handle) {
            let message = format!("{path}: {error}");
            error!("browser appearance DBC failed: {message}");
            commands.insert_resource(CatalogFailure(message));
            commands.remove_resource::<CatalogLoads>();
            return;
        }
    }
    let Some(mut source) = loads
        .0
        .iter()
        .map(|(path, handle)| files.get(handle).map(|file| (*path, file.0.clone())))
        .collect::<Option<Vec<_>>>()
        .map(MemoryFileSource::new)
    else {
        trace.waiting_frames = trace.waiting_frames.saturating_add(1);
        if trace.waiting_frames >= 180 && !trace.reported_wait {
            let pending = loads
                .0
                .iter()
                .filter(|(_, handle)| files.get(handle).is_none())
                .map(|(path, handle)| format!("{path}={:?}", assets.load_state(handle)))
                .collect::<Vec<_>>()
                .join(", ");
            warn!("browser appearance DBC still pending: {pending}");
            trace.reported_wait = true;
        }
        return;
    };

    let parsed = (|| -> Result<_> {
        match benilla_formats::load_item_visual_catalog(&mut source) {
            Ok(visuals) => {
                commands.insert_resource(super::ItemGlows::new(visuals));
            }
            Err(error) => warn!("browser item visual catalog unavailable: {error:#}"),
        }
        crate::creature_anim::publish_browser_catalogs(&mut commands, &mut source)?;
        Ok((
            benilla_formats::load_creature_catalog(&mut source)?,
            CharCreateCatalog::load(&mut source)?,
            CharacterGeosets::load(&mut source)?,
            CharSections::load(&mut source)?,
            benilla_formats::load_item_display_catalog(&mut source)?,
            benilla_formats::load_map_catalog(&mut source)?,
            benilla_formats::LightCatalog::load(&mut source)?,
            benilla_formats::load_enchant_catalog(&mut source)?,
        ))
    })();
    match parsed {
        Ok((creatures, char_create, characters, sections, displays, maps, lights, enchants)) => {
            commands.insert_resource(crate::items::Enchants(enchants));
            info!(
                "browser appearance catalogs ready: {} creature displays, {} item displays",
                creatures.len(),
                displays.len(),
            );
            commands.insert_resource(Creatures {
                catalog: creatures,
                models: HashMap::new(),
            });
            commands.insert_resource(CharCreate(char_create));
            commands.insert_resource(Characters(characters));
            commands.insert_resource(SkinSections(sections));
            commands.insert_resource(ItemDisplays {
                catalog: displays,
                models: HashMap::new(),
            });
            commands.insert_resource(benilla_assets::MapCatalogRes(maps));
            benilla_world::lighting::publish_light_catalog(&mut commands, lights);
            // Parsed catalogs own their data; release the raw DBC handles so the browser asset
            // cache may reclaim those response buffers.
            commands.remove_resource::<CatalogLoads>();
        }
        Err(error) => {
            let message = format!("{error:#}");
            error!("browser appearance catalogs failed: {message}");
            commands.insert_resource(CatalogFailure(message));
            // Do not retry the same deterministic parse every frame.
            commands.remove_resource::<CatalogLoads>();
        }
    }
}

/// Report the appearance service at the moment a screen actually asks for an actor. Startup asset
/// chatter is intentionally noisy in a browser; tying this line to the selected look keeps the
/// relevant state next to the race-background log instead of losing it above the console buffer.
fn trace_appearance_request(
    preview: Res<GluePreview>,
    loads: Option<Res<CatalogLoads>>,
    ready: Option<Res<CharCreate>>,
    failure: Option<Res<CatalogFailure>>,
    assets: Res<AssetServer>,
    mut trace: Local<RequestTrace>,
) {
    let Some(look) = preview.look else {
        trace.last = None;
        return;
    };
    let status = if ready.is_some() {
        0
    } else if failure.is_some() {
        2
    } else if loads.is_some() {
        1
    } else {
        3
    };
    if trace.last == Some((look, status)) {
        return;
    }
    trace.last = Some((look, status));

    let (race, sex) = look.body();
    let pet = look.pet().map_or(0, |pet| pet.display_id);
    match status {
        0 => info!("browser appearance request: race {race}, sex {sex}, pet {pet}; catalogs ready"),
        1 => {
            let pending = loads
                .as_deref()
                .into_iter()
                .flat_map(|loads| &loads.0)
                .filter(|(_, handle)| !assets.load_state(handle).is_loaded())
                .map(|(path, handle)| format!("{path}={:?}", assets.load_state(handle)))
                .collect::<Vec<_>>()
                .join(", ");
            warn!(
                "browser appearance request: race {race}, sex {sex}, pet {pet}; DBC pending: {pending}"
            );
        }
        2 => error!(
            "browser appearance request: race {race}, sex {sex}, pet {pet}; catalogs unavailable: {}",
            failure
                .as_deref()
                .map_or("unknown failure", |f| f.0.as_str())
        ),
        _ => error!(
            "browser appearance request: race {race}, sex {sex}, pet {pet}; catalog service missing"
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn update_preview_display_models(
    preview: Res<GluePreview>,
    char_create: Option<Res<CharCreate>>,
    mut creatures: Option<ResMut<Creatures>>,
    mut items: Option<ResMut<ItemDisplays>>,
    mut glows: Option<ResMut<super::ItemGlows>>,
    mut fx: Option<ResMut<super::spell_fx::SpellFx>>,
    model_assets: (Res<Assets<M2Model>>, Res<Assets<WmoModel>>),
    mut forms: ResMut<benilla_world::model_forms::ModelForms>,
    mut meshes: ResMut<Assets<Mesh>>,
    asset_server: Res<AssetServer>,
    mut mats: benilla_world::model_render::M2BatchMaterials,
    mut uv_reg: ResMut<benilla_world::doodad_anim::UvAnimMaterials>,
    mut anim_table: ResMut<benilla_world::mat_anim_table::MatAnimTable>,
    mut trace: Local<DisplayTrace>,
) {
    if !mats.ready() {
        return;
    }
    let (Some(char_create), Some(creatures)) = (char_create.as_deref(), creatures.as_deref_mut())
    else {
        return;
    };
    let mut wanted = Vec::new();
    if let Some(look) = preview.look {
        let (race, sex) = look.body();
        match char_create.0.body_display(race, sex) {
            Some(display_id) => wanted.push(display_id),
            None => {
                if trace.missing_bodies.insert((race, sex)) {
                    warn!("browser appearance: no body display for race {race}, sex {sex}");
                }
            }
        }
        wanted.extend(look.pet().map(|pet| pet.display_id));
    }
    let mut uv = benilla_world::model_render::EntityUvLane {
        reg: &mut uv_reg,
        table: &mut anim_table,
        instance: None,
    };
    for display_id in wanted {
        if !creatures.models.contains_key(&display_id) {
            let model = new_creature_display(&creatures.catalog, display_id, &asset_server);
            if model.names_a_model() {
                info!("browser appearance: requested creature display {display_id}");
            } else {
                warn!("browser appearance: display {display_id} resolves to no model");
            }
            creatures.models.insert(display_id, model);
        }
        if let Some(model) = creatures.models.get_mut(&display_id) {
            if model.parts.is_none() {
                // The streamed world owns a demand-driven ModelForms furnisher. A glue-only
                // browser app deliberately has no terrain/world plugin, so its adapter fulfils
                // this off-world demand directly, like the production backdrop booth does.
                ensure_web_forms(model, &model_assets.0, &mut forms, &mut meshes);
                build_parts(
                    model,
                    &model_assets.0,
                    &model_assets.1,
                    &mut forms,
                    &asset_server,
                    &mut mats,
                    &mut uv,
                    false,
                );
                if let Some(parts) = model.parts.as_ref() {
                    if trace.built.insert(display_id) {
                        info!(
                            "browser appearance: creature display {display_id} built ({} parts)",
                            parts.len()
                        );
                    }
                    trace.waiting_frames.remove(&display_id);
                } else {
                    let frames = trace.waiting_frames.entry(display_id).or_default();
                    *frames = frames.saturating_add(1);
                    if *frames == 180 {
                        let state = match &model.handle {
                            ModelHandle::M2(handle) => asset_server.load_state(handle),
                            ModelHandle::Wmo(handle) => asset_server.load_state(handle),
                            ModelHandle::None => LoadState::NotLoaded,
                        };
                        warn!(
                            "browser appearance: display {display_id} still waiting; asset={state:?}"
                        );
                    }
                }
            }
        }
    }
    if let Some(items) = items.as_deref_mut() {
        for model in items
            .models
            .values_mut()
            .filter(|model| model.parts.is_none())
        {
            ensure_web_forms(model, &model_assets.0, &mut forms, &mut meshes);
            build_parts(
                model,
                &model_assets.0,
                &model_assets.1,
                &mut forms,
                &asset_server,
                &mut mats,
                &mut uv,
                false,
            );
        }
    }
    // Off-world spell kits (character-select ghost) use the same cached models
    // and original part builder as item glows; no world entity is needed.
    for model in glows
        .as_deref_mut()
        .into_iter()
        .flat_map(|cache| cache.models.values_mut())
        .chain(
            fx.as_deref_mut()
                .into_iter()
                .flat_map(|cache| cache.models.values_mut()),
        )
        .filter(|model| model.parts.is_none())
    {
        ensure_web_forms(model, &model_assets.0, &mut forms, &mut meshes);
        build_parts(
            model,
            &model_assets.0,
            &model_assets.1,
            &mut forms,
            &asset_server,
            &mut mats,
            &mut uv,
            false,
        );
    }
}

/// Fulfil render-form requests without installing the streamed world's terrain scheduler and its
/// unrelated `ViewFocus`/`StreamActivity` resources.
fn ensure_web_forms(
    display: &DisplayModel,
    m2s: &Assets<M2Model>,
    forms: &mut benilla_world::model_forms::ModelForms,
    meshes: &mut Assets<Mesh>,
) {
    match &display.handle {
        ModelHandle::M2(handle) => {
            if let Some(model) = m2s.get(handle) {
                forms.ensure_now_rigged(handle, &model.submeshes, meshes);
            }
        }
        // CreatureDisplayInfo and ItemDisplayInfo resolve to M2. Keep the exhaustive arm so the
        // shared display type can retain its world-side WMO variant without widening an off-world
        // API the browser appearance lane cannot request.
        ModelHandle::Wmo(_) => {}
        ModelHandle::None => {}
    }
}

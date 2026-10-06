use benilla_assets::coords::{placement_rotation, wow_to_bevy};
use benilla_assets::web_materials::{
    register_web_shaders, spawn_web_model_batch, WebLightUniform, WebModelExtension,
    WebModelMaterial, WebTerrainExtension, WebTerrainMaterial,
};
use benilla_assets::{
    chunks_to_mesh, for_upload, m2_url, normalize_client_path, texture_url, wmo_url,
    world_art_image, AdtLoader, AdtTile, BlpGpuPolicy, BlpGpuSupportPlugin, BlpImageLoader,
    BlpMipChain, BlpTexels, FrameXmlSource, FrameXmlSourceLoader, LuaSource, LuaSourceLoader,
    M2Model, M2ModelLoader, ModelBlend, ModelSubmesh, WdtIndex, WdtIndexLoader, WebAssetStats,
    WmoModel, WmoModelLoader,
};
use benilla_lua::WebLuaSession;
use benilla_ui::web_script::{WebDialog, WebUiScript};
#[allow(dead_code)]
mod original_glue;
mod web_glue;
use bevy::{
    asset::{AssetApp, LoadState, RenderAssetUsages},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::Indices,
    pbr::{ExtendedMaterial, MaterialPlugin, PbrPlugin},
    prelude::*,
    render::render_resource::{
        Extent3d, PrimitiveTopology, TextureDimension, TextureFormat, TextureViewDescriptor,
        TextureViewDimension,
    },
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

const ARRAY_SIZE: u32 = 64;
const TERRAIN_WIDTH: f32 = 210.0;
const TERRAIN_DEPTH: f32 = 145.0;
const DEFAULT_PROBE_BLP: &str = "interface/icons/inv_misc_questionmark.blp";
const A2_WDT: &str = "mpq://world/maps/azeroth/azeroth.wdt";
const A2_ADT: &str = "mpq://world/maps/azeroth/azeroth_31_49.adt";
const A2_TILE: (u32, u32) = (31, 49);
const L2_GLOBAL_STRINGS: &str = "mpq://interface/framexml/globalstrings.lua";
const L4_BASIC_CONTROLS: &str = "mpq://interface/framexml/basiccontrols.xml";

fn main() {
    let probe_mode = query_value("probe").as_deref() == Some("1");
    let force_decode = force_decoded_blp();
    let probe_asset = query_value("asset")
        .and_then(|raw| normalize_client_path(&raw).ok())
        .unwrap_or_else(|| DEFAULT_PROBE_BLP.to_owned());
    let probe_missing = query_value("probe404").is_some_and(|value| value != "0");
    let mut app = App::new();
    #[cfg(target_arch = "wasm32")]
    benilla_assets::register_web_mpq_source(&mut app, "/assets/v1/file");
    app.insert_resource(WebProbeConfig {
        asset: probe_asset,
        probe_missing,
    })
    .insert_non_send_resource(WebLuaProbe::default())
    .insert_resource(ClearColor(Color::srgb(0.055, 0.045, 0.075)))
    .insert_resource(BlpGpuPolicy { force_decode })
    .add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: if probe_mode {
                        "Benilla Web — compatibility probes"
                    } else {
                        "Benilla Web — Account Login"
                    }
                    .into(),
                    canvas: Some("#benilla".into()),
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            })
            .set(PbrPlugin {
                prepass_enabled: false,
                add_default_deferred_lighting_plugin: false,
                use_gpu_instance_buffer_builder: false,
                ..default()
            }),
    )
    .add_plugins(register_web_shaders)
    .add_plugins(BlpGpuSupportPlugin)
    .add_plugins((
        MaterialPlugin::<WebTerrainMaterial>::default(),
        MaterialPlugin::<WebModelMaterial>::default(),
    ))
    .init_resource::<WorldTileProbe>();
    if probe_mode {
        app.add_systems(Startup, (run_l1_lua_probe, setup).chain())
            .add_systems(
                Update,
                (
                    animate_scene,
                    move_camera,
                    observe_remote_blp,
                    drive_l2_lua,
                    drive_l4_dialog,
                    drive_a2_terrain,
                    drive_a3_models,
                )
                    .chain(),
            )
            .add_systems(Update, drive_world_tile);
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, poll_world_entry.before(drive_world_tile));
    } else {
        app.init_resource::<original_glue::art::GlueArt>();
        app.add_systems(Startup, web_glue::setup)
            .add_systems(Update, (web_glue::drive, web_glue::drive_scene));
    }
    app.init_asset::<WdtIndex>();
    app.init_asset::<AdtTile>();
    app.init_asset::<M2Model>();
    app.init_asset::<WmoModel>();
    app.init_asset::<LuaSource>();
    app.init_asset::<FrameXmlSource>();
    app.register_asset_loader(BlpImageLoader);
    app.register_asset_loader(WdtIndexLoader);
    app.register_asset_loader(AdtLoader);
    app.register_asset_loader(M2ModelLoader);
    app.register_asset_loader(WmoModelLoader);
    app.register_asset_loader(LuaSourceLoader);
    app.register_asset_loader(FrameXmlSourceLoader);
    app.run();
}

fn run_l1_lua_probe() {
    let report = benilla_lua::run_l1_probe();
    if let Some(error) = &report.baseline_error {
        error!(
            "L1 rilua baseline FAILED: {}/{} capabilities passed: {error}",
            report.baseline_passed, report.baseline_total
        );
    } else {
        info!(
            "L1 rilua baseline PASS: {}/{} capabilities, VM heap={} bytes",
            report.baseline_passed, report.baseline_total, report.gc_bytes
        );
    }
    if report.vanilla_gaps.is_empty() {
        info!(
            "L1 Vanilla dialect PASS: {}/{} measured client behaviors",
            report.vanilla_passed, report.vanilla_total
        );
    } else {
        info!(
            "L1 Vanilla dialect gaps: {}/{} pass; port required for [{}]",
            report.vanilla_passed,
            report.vanilla_total,
            report.vanilla_gaps.join(", ")
        );
    }
}

struct L3FrameDefinition {
    name: String,
    on_load: String,
    on_click: String,
}

fn framexml_child<'a>(
    element: &'a benilla_ui::framexml::Element,
    tag: &str,
) -> Option<&'a benilla_ui::framexml::Element> {
    element
        .children
        .iter()
        .find(|child| child.tag.eq_ignore_ascii_case(tag))
}

fn parse_l3_frame_xml() -> Result<L3FrameDefinition, String> {
    let xml = include_str!("../../../web/l3-frame.xml");
    let doc = benilla_ui::framexml::parse(xml).map_err(|error| error.to_string())?;
    let mut buttons = doc.items.iter().filter_map(|item| match item {
        benilla_ui::framexml::TopLevel::Instance(element)
            if element.tag.eq_ignore_ascii_case("Button") =>
        {
            Some(element)
        }
        _ => None,
    });
    let button = buttons
        .next()
        .ok_or_else(|| "L3 FrameXML has no <Button>".to_owned())?;
    if buttons.next().is_some() {
        return Err("L3 FrameXML has more than one <Button>".to_owned());
    }
    let name = button
        .name()
        .ok_or_else(|| "L3 Button is unnamed".to_owned())?;
    let scripts =
        framexml_child(button, "Scripts").ok_or_else(|| "L3 Button has no <Scripts>".to_owned())?;
    let handler = |tag: &str| {
        framexml_child(scripts, tag)
            .map(|node| node.body.clone())
            .filter(|body| !body.trim().is_empty())
            .ok_or_else(|| format!("L3 Button has no <{tag}> body"))
    };
    Ok(L3FrameDefinition {
        name: name.to_owned(),
        on_load: handler("OnLoad")?,
        on_click: handler("OnClick")?,
    })
}

#[cfg(target_arch = "wasm32")]
fn force_decoded_blp() -> bool {
    query_value("blp").is_some_and(|value| value.eq_ignore_ascii_case("rgba"))
}

#[cfg(not(target_arch = "wasm32"))]
fn force_decoded_blp() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn query_value(name: &str) -> Option<String> {
    let query = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&query)
        .ok()?
        .get(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn query_value(_name: &str) -> Option<String> {
    None
}

#[derive(Resource)]
struct WebProbeConfig {
    asset: String,
    probe_missing: bool,
}

#[derive(Default)]
struct WebLuaProbe {
    source: Option<Handle<LuaSource>>,
    basic_controls: Option<Handle<FrameXmlSource>>,
    session: Option<WebLuaSession>,
    l4_ui: Option<WebUiScript>,
    l4_dialog: Option<WebDialog>,
    stage: L2Stage,
    l3_ready: bool,
    l4_stage: L4Stage,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum L2Stage {
    #[default]
    Loading,
    Ready,
    Failed,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum L4Stage {
    #[default]
    Loading,
    Ready,
    Failed,
}

#[derive(Component)]
struct WebLuaStatus;

#[derive(Component)]
struct WebLuaButton;

#[derive(Component)]
struct L3FrameButton;

#[derive(Component)]
struct L3FrameText;

#[derive(Component)]
struct L4DialogRoot;

#[derive(Component)]
struct L4DialogButton;

#[derive(Component)]
struct L4DialogButtonText;

#[derive(Component)]
struct ProbeCamera;

#[derive(Component)]
struct ProbeModel;

#[derive(Component)]
struct ProbeScaffold;

#[derive(Component)]
struct A2Terrain;

#[derive(Resource)]
struct ProbeMaterials {
    model: Handle<WebModelMaterial>,
}

#[derive(Resource)]
struct A2TerrainProbe {
    stage: A2Stage,
    wdt: Option<Handle<WdtIndex>>,
    adt: Option<Handle<AdtTile>>,
    entity: Option<Entity>,
    generation: u32,
    world_offset: Vec3,
}

#[derive(Resource, Default)]
struct WorldTileProbe {
    stage: WorldTileStage,
    map_name: String,
    tile: (u32, u32),
    position: Vec3,
    wdt: Option<Handle<WdtIndex>>,
    adt: Option<Handle<AdtTile>>,
    entity: Option<Entity>,
}

#[derive(Default, PartialEq, Eq)]
enum WorldTileStage {
    #[default]
    Idle,
    LoadingWdt,
    LoadingAdt,
    Live,
    Failed,
}

#[derive(Resource)]
struct A3ModelProbe {
    stage: A3Stage,
    generation: u32,
    m2: Option<Handle<M2Model>>,
    wmo: Option<Handle<WmoModel>>,
    m2_transform: Transform,
    wmo_transform: Transform,
    m2_label: String,
    wmo_label: String,
    next_m2_batch: usize,
    next_wmo_batch: usize,
    entities: Vec<Entity>,
    fallbacks: A3FallbackStats,
}

#[derive(Default)]
struct A3FallbackStats {
    batches: usize,
    multiply_blend: usize,
    billboard_or_env_map: usize,
    custom_depth: usize,
    animated_material: usize,
    wmo_lighting: usize,
    missing_texture: usize,
}

impl A3FallbackStats {
    fn record(&mut self, submesh: &ModelSubmesh) {
        let multiply_blend = matches!(submesh.blend, ModelBlend::Mod | ModelBlend::Mod2x);
        let billboard_or_env_map = submesh.billboard.is_some() || submesh.env_map;
        let custom_depth = submesh.no_depth_write || submesh.no_depth_test;
        let animated_material =
            submesh.alpha_anim.is_some() || submesh.uv_anim.is_some() || submesh.rgb_anim.is_some();
        let wmo_lighting = submesh.interior || submesh.window || submesh.sidn.is_some();
        let missing_texture = submesh.texture.is_none();
        let any = multiply_blend
            || billboard_or_env_map
            || custom_depth
            || animated_material
            || wmo_lighting
            || missing_texture;

        self.batches += any as usize;
        self.multiply_blend += multiply_blend as usize;
        self.billboard_or_env_map += billboard_or_env_map as usize;
        self.custom_depth += custom_depth as usize;
        self.animated_material += animated_material as usize;
        self.wmo_lighting += wmo_lighting as usize;
        self.missing_texture += missing_texture as usize;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum A3Stage {
    Waiting,
    Loading,
    Spawning,
    Live,
    Releasing(u8),
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum A2Stage {
    LoadingWdt,
    LoadingAdt,
    Live,
    Unloading(u8),
    Unloaded,
    Failed,
}

#[derive(Resource)]
struct RemoteBlpProbe {
    logical: String,
    primary: Handle<Image>,
    alias_url: String,
    alias: Option<Handle<Image>>,
    missing: Option<Handle<Image>>,
    probe_missing: bool,
    stage: RemoteProbeStage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteProbeStage {
    Primary,
    CachedAlias,
    Missing,
    Complete,
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    web_probe: Res<WebProbeConfig>,
    mut lua_probe: NonSendMut<WebLuaProbe>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut terrain_materials: ResMut<Assets<WebTerrainMaterial>>,
    mut model_materials: ResMut<Assets<WebModelMaterial>>,
) {
    let light = WebLightUniform::default();
    let terrain = terrain_materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: WebTerrainExtension {
            light,
            layer_array: images.add(layer_array()),
            alpha_array: images.add(alpha_array()),
            shadow_array: images.add(shadow_array()),
            params: Vec4::new(8.0, 0.0, 0.0, 0.0),
        },
    });

    commands.spawn((
        Mesh3d(meshes.add(terrain_mesh())),
        MeshMaterial3d(terrain.clone()),
        Visibility::Hidden,
        ProbeScaffold,
        Name::new("G2 production terrain material"),
    ));

    let upload = for_upload(synthetic_bc1_texture());
    let chosen_format = upload.format;
    info!(
        "G2 BLP probe: {:?} (append ?blp=rgba to force the decoded path)",
        chosen_format
    );
    let model_texture = images.add(world_art_image(upload, (true, true)));
    let model = model_materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(model_texture),
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: WebModelExtension {
            light,
            tint: Vec4::ONE,
            uv_anim: Vec4::ZERO,
        },
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(20.0, 30.0, 20.0))),
        MeshMaterial3d(model.clone()),
        Transform::from_xyz(0.0, 21.0, 0.0),
        Visibility::Hidden,
        ProbeModel,
        ProbeScaffold,
        Name::new("G2 animated model material"),
    ));

    let primary_url = texture_url(&web_probe.asset, (true, true));
    let primary = asset_server.load::<Image>(primary_url.clone());
    let duplicate = asset_server.load::<Image>(primary_url.clone());
    info!(
        "A1 handle dedup: {} -> same_id={} ({primary_url})",
        web_probe.asset,
        primary.id() == duplicate.id()
    );
    let remote_material = model_materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(primary.clone()),
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: WebModelExtension {
            light,
            tint: Vec4::ONE,
            uv_anim: Vec4::ZERO,
        },
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(24.0, 24.0, 24.0))),
        MeshMaterial3d(remote_material),
        Transform::from_xyz(38.0, 18.0, -8.0),
        Visibility::Hidden,
        ProbeScaffold,
        Name::new("A1 remote MPQ BLP"),
    ));
    commands.insert_resource(RemoteBlpProbe {
        logical: web_probe.asset.clone(),
        primary,
        alias_url: texture_url(&web_probe.asset, (false, false)),
        alias: None,
        missing: None,
        probe_missing: web_probe.probe_missing,
        stage: RemoteProbeStage::Primary,
    });

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 82.0, 132.0).looking_at(Vec3::new(0.0, 7.0, 0.0), Vec3::Y),
        Msaa::Off,
        ProbeCamera,
    ));
    commands.insert_resource(ProbeMaterials { model });
    commands.insert_resource(A2TerrainProbe {
        stage: A2Stage::LoadingWdt,
        wdt: Some(asset_server.load(A2_WDT)),
        adt: None,
        entity: None,
        generation: 1,
        world_offset: Vec3::ZERO,
    });
    commands.insert_resource(A3ModelProbe {
        stage: A3Stage::Waiting,
        generation: 0,
        m2: None,
        wmo: None,
        m2_transform: Transform::IDENTITY,
        wmo_transform: Transform::IDENTITY,
        m2_label: String::new(),
        wmo_label: String::new(),
        next_m2_batch: 0,
        next_wmo_batch: 0,
        entities: Vec::new(),
        fallbacks: A3FallbackStats::default(),
    });
    lua_probe.source = Some(asset_server.load(L2_GLOBAL_STRINGS));
    lua_probe.basic_controls = Some(asset_server.load(L4_BASIC_CONTROLS));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(12.0),
                width: Val::Px(350.0),
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(8.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.035, 0.06, 0.84)),
            Name::new("L2 real Lua script probe"),
        ))
        .with_children(|ui| {
            ui.spawn((
                Text::new("L2: loading GlobalStrings.lua from MPQ..."),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                WebLuaStatus,
            ));
            ui.spawn((
                Button,
                WebLuaButton,
                Node {
                    width: Val::Px(150.0),
                    height: Val::Px(32.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.23, 0.18, 0.34)),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("Run Lua handler"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
            ui.spawn((
                Button,
                L3FrameButton,
                Node {
                    width: Val::Px(170.0),
                    height: Val::Px(32.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.17, 0.27, 0.23)),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("FrameXML: loading..."),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    L3FrameText,
                ));
            });
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(135.0),
                width: Val::Px(285.0),
                padding: UiRect::all(Val::Px(14.0)),
                row_gap: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.06, 0.09, 0.94)),
            Visibility::Hidden,
            L4DialogRoot,
            Name::new("ScriptErrors from BasicControls.xml"),
        ))
        .with_children(|dialog| {
            dialog.spawn((
                Text::new("ScriptErrors dialog: shipped OnClick hides this frame"),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            dialog
                .spawn((
                    Button,
                    L4DialogButton,
                    Node {
                        width: Val::Px(128.0),
                        height: Val::Px(32.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.36, 0.22, 0.18)),
                    Name::new("ScriptErrorsButton"),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new("..."),
                        TextFont {
                            font_size: 15.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        L4DialogButtonText,
                    ));
                });
        });
    info!(
        "A2 generation 1: loading WDT, then tile {}_{}; press R once to unload and again to reload",
        A2_TILE.0, A2_TILE.1
    );
}

fn observe_remote_blp(asset_server: Res<AssetServer>, mut probe: ResMut<RemoteBlpProbe>) {
    match probe.stage {
        RemoteProbeStage::Primary => match asset_server.load_state(&probe.primary) {
            LoadState::Loaded => {
                info!("A1 primary BLP loaded through mpq://: {}", probe.logical);
                probe.alias = Some(asset_server.load(probe.alias_url.clone()));
                probe.stage = RemoteProbeStage::CachedAlias;
            }
            LoadState::Failed(ref error) => {
                error!("A1 primary BLP failed: {}: {error}", probe.logical);
                probe.stage = RemoteProbeStage::Complete;
            }
            _ => {}
        },
        RemoteProbeStage::CachedAlias => {
            let Some(alias) = &probe.alias else {
                return;
            };
            match asset_server.load_state(alias) {
                LoadState::Loaded => {
                    info!(
                        "A1 sampler alias loaded; web reader should report cache HIT for {}",
                        probe.logical
                    );
                    if probe.probe_missing {
                        probe.missing =
                            Some(asset_server.load("mpq://__benilla__/a1_expected_missing.blp"));
                        probe.stage = RemoteProbeStage::Missing;
                    } else {
                        probe.stage = RemoteProbeStage::Complete;
                    }
                }
                LoadState::Failed(ref error) => {
                    error!("A1 cached sampler alias failed: {error}");
                    probe.stage = RemoteProbeStage::Complete;
                }
                _ => {}
            }
        }
        RemoteProbeStage::Missing => {
            let Some(missing) = &probe.missing else {
                return;
            };
            match asset_server.load_state(missing) {
                LoadState::Failed(ref error) => {
                    info!("A1 expected 404 completed as a clean asset error: {error}");
                    probe.stage = RemoteProbeStage::Complete;
                }
                LoadState::Loaded => {
                    error!("A1 missing-file sentinel unexpectedly exists in the MPQ chain");
                    probe.stage = RemoteProbeStage::Complete;
                }
                _ => {}
            }
        }
        RemoteProbeStage::Complete => {}
    }
}

fn drive_l2_lua(
    asset_server: Res<AssetServer>,
    sources: Res<Assets<LuaSource>>,
    mut probe: NonSendMut<WebLuaProbe>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Query<Ref<Interaction>, With<WebLuaButton>>,
    l3_buttons: Query<Ref<Interaction>, With<L3FrameButton>>,
    mut statuses: Query<&mut Text, (With<WebLuaStatus>, Without<L3FrameText>)>,
    mut l3_texts: Query<&mut Text, (With<L3FrameText>, Without<WebLuaStatus>)>,
) {
    let Ok(mut status) = statuses.single_mut() else {
        return;
    };

    if probe.stage == L2Stage::Loading {
        let Some(handle) = probe.source.clone() else {
            return;
        };
        match asset_server.load_state(&handle) {
            LoadState::Loaded => {
                let Some(source) = sources.get(&handle) else {
                    return;
                };
                match WebLuaSession::from_global_strings(&source.0) {
                    Ok(mut session) => {
                        let (close, all) = session.shipped_labels();
                        info!(
                            "L2 GlobalStrings.lua PASS: {} bytes from MPQ, CLOSE={close:?}, ALL={all:?}",
                            source.0.len()
                        );
                        *status = Text::new(format!(
                            "L2: GlobalStrings.lua loaded ({} bytes)\nCLOSE={close} / ALL={all}\nClick below or press L",
                            source.0.len()
                        ));
                        match parse_l3_frame_xml().and_then(|frame| {
                            let label = session.install_frame_script(
                                &frame.name,
                                &frame.on_load,
                                &frame.on_click,
                            )?;
                            Ok((frame.name, label))
                        }) {
                            Ok((name, label)) => {
                                info!("L3 FrameXML OnLoad PASS: {name}, SetText={label:?}");
                                if let Ok(mut text) = l3_texts.single_mut() {
                                    *text = Text::new(label);
                                }
                                probe.l3_ready = true;
                            }
                            Err(error) => {
                                error!("L3 FrameXML setup failed: {error}");
                                if let Ok(mut text) = l3_texts.single_mut() {
                                    *text = Text::new("L3 setup failed");
                                }
                            }
                        }
                        probe.session = Some(session);
                        probe.stage = L2Stage::Ready;
                    }
                    Err(error) => {
                        error!("L2 GlobalStrings.lua execution failed: {error}");
                        *status = Text::new(format!("L2 Lua error: {error}"));
                        probe.stage = L2Stage::Failed;
                    }
                }
            }
            LoadState::Failed(ref error) => {
                error!("L2 GlobalStrings.lua asset failed: {error}");
                *status = Text::new(format!("L2 MPQ load failed: {error}"));
                probe.stage = L2Stage::Failed;
            }
            _ => {}
        }
    }

    if probe.stage != L2Stage::Ready {
        return;
    }
    let clicked = buttons
        .iter()
        .any(|interaction| interaction.is_changed() && *interaction == Interaction::Pressed);
    if clicked || keys.just_pressed(KeyCode::KeyL) {
        let Some(session) = probe.session.as_mut() else {
            return;
        };
        match session.click() {
            Ok(label) => {
                info!("L2 persistent Lua handler PASS: {label}");
                *status = Text::new(format!("L2: {label}\nClick again to advance Lua state"));
            }
            Err(error) => {
                error!("L2 persistent Lua handler failed: {error}");
                *status = Text::new(format!("L2 Lua handler error: {error}"));
                probe.stage = L2Stage::Failed;
                return;
            }
        }
    }

    let l3_clicked = l3_buttons
        .iter()
        .any(|interaction| interaction.is_changed() && *interaction == Interaction::Pressed);
    if !probe.l3_ready || !l3_clicked {
        return;
    }
    let Some(session) = probe.session.as_mut() else {
        return;
    };
    match session.frame_click() {
        Ok(label) => {
            info!("L3 FrameXML OnClick PASS: host SetText={label:?}");
            if let Ok(mut text) = l3_texts.single_mut() {
                *text = Text::new(label);
            }
        }
        Err(error) => {
            error!("L3 FrameXML OnClick failed: {error}");
            if let Ok(mut text) = l3_texts.single_mut() {
                *text = Text::new("L3 OnClick failed");
            }
            probe.l3_ready = false;
        }
    }
}

fn drive_l4_dialog(
    asset_server: Res<AssetServer>,
    xml_sources: Res<Assets<FrameXmlSource>>,
    lua_sources: Res<Assets<LuaSource>>,
    mut probe: NonSendMut<WebLuaProbe>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Query<Ref<Interaction>, With<L4DialogButton>>,
    mut roots: Query<&mut Visibility, With<L4DialogRoot>>,
    mut labels: Query<&mut Text, With<L4DialogButtonText>>,
) {
    if probe.stage != L2Stage::Ready {
        return;
    }
    let Ok(mut visibility) = roots.single_mut() else {
        return;
    };

    if probe.l4_stage == L4Stage::Loading {
        let Some(handle) = probe.basic_controls.clone() else {
            return;
        };
        match asset_server.load_state(&handle) {
            LoadState::Loaded => {
                let Some(source) = xml_sources.get(&handle) else {
                    return;
                };
                let result = (|| {
                    let xml = std::str::from_utf8(&source.0).map_err(|error| error.to_string())?;
                    let lua_handle = probe
                        .source
                        .as_ref()
                        .ok_or_else(|| "L4 has no GlobalStrings.lua handle".to_owned())?;
                    let lua_source = lua_sources
                        .get(lua_handle)
                        .ok_or_else(|| "L4 GlobalStrings.lua bytes are unavailable".to_owned())?;
                    let mut ui = WebUiScript::new()?;
                    ui.exec_lua(&lua_source.0, "@Interface\\FrameXML\\GlobalStrings.lua")?;
                    let dialog = ui.install_dialog(xml, "ScriptErrors")?;
                    Ok::<_, String>((ui, dialog))
                })();
                match result {
                    Ok((ui, dialog)) => {
                        info!(
                            "L4 shared UI READY: BasicControls.xml ({} bytes), ScriptErrors inherits DialogBoxFrame, button={}, text={:?}",
                            source.0.len(),
                            dialog.button_name,
                            dialog.label,
                        );
                        if let Ok(mut text) = labels.single_mut() {
                            *text = Text::new(dialog.label.clone());
                        }
                        *visibility = Visibility::Inherited;
                        probe.l4_ui = Some(ui);
                        probe.l4_dialog = Some(dialog);
                        probe.l4_stage = L4Stage::Ready;
                    }
                    Err(error) => {
                        error!("L4 shipped FrameXML setup failed: {error}");
                        probe.l4_stage = L4Stage::Failed;
                    }
                }
            }
            LoadState::Failed(ref error) => {
                error!("L4 BasicControls.xml asset failed: {error}");
                probe.l4_stage = L4Stage::Failed;
            }
            _ => {}
        }
    }

    if probe.l4_stage != L4Stage::Ready {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        let Some(frame) = probe.l4_dialog.as_ref().map(|dialog| dialog.frame) else {
            return;
        };
        let Some(ui) = probe.l4_ui.as_mut() else {
            return;
        };
        if let Err(error) = ui.set_shown(frame, true) {
            error!("L4 dialog reopen failed: {error}");
            probe.l4_stage = L4Stage::Failed;
            return;
        }
        *visibility = Visibility::Inherited;
        info!("L4 ScriptErrors dialog reopened by P");
    }
    let clicked = buttons
        .iter()
        .any(|interaction| interaction.is_changed() && *interaction == Interaction::Pressed);
    if !clicked {
        return;
    }
    let Some((button, frame)) = probe
        .l4_dialog
        .as_ref()
        .map(|dialog| (dialog.button, dialog.frame))
    else {
        return;
    };
    let Some(ui) = probe.l4_ui.as_mut() else {
        return;
    };
    match ui.click(button).and_then(|()| ui.visible(frame)) {
        Ok(false) => {
            *visibility = Visibility::Hidden;
            info!("L4 shipped OnClick PASS: this:GetParent():Hide() hid ScriptErrors");
        }
        Ok(true) => {
            error!("L4 shipped OnClick returned, but ScriptErrors is still visible");
            probe.l4_stage = L4Stage::Failed;
        }
        Err(error) => {
            error!("L4 shipped OnClick failed: {error}");
            probe.l4_stage = L4Stage::Failed;
        }
    }
}

fn animate_scene(
    time: Res<Time>,
    handles: Res<ProbeMaterials>,
    mut terrain_materials: ResMut<Assets<WebTerrainMaterial>>,
    mut model_materials: ResMut<Assets<WebModelMaterial>>,
    mut models: Query<&mut Transform, With<ProbeModel>>,
) {
    let seconds = time.elapsed_secs();
    let sun = Vec3::new(
        0.45 + (seconds * 0.22).sin() * 0.22,
        -0.82,
        0.35 + (seconds * 0.17).cos() * 0.18,
    )
    .normalize();

    for (_, material) in terrain_materials.iter_mut() {
        material.extension.light.sun = sun.extend(1.0);
    }
    if let Some(material) = model_materials.get_mut(&handles.model) {
        material.extension.light.sun = sun.extend(1.0);
        material.extension.uv_anim.x = (seconds * 0.06).fract();
        material.extension.tint = Vec4::new(
            0.88 + (seconds * 0.9).sin() * 0.12,
            0.90 + (seconds * 0.7).cos() * 0.10,
            1.0,
            1.0,
        );
    }
    for mut transform in &mut models {
        transform.rotation = Quat::from_rotation_y(seconds * 0.42)
            * Quat::from_rotation_x((seconds * 0.31).sin() * 0.12);
    }
}

fn move_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    world: Res<WorldTileProbe>,
    mut camera: Query<&mut Transform, With<ProbeCamera>>,
) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let forward = (transform.rotation * Vec3::NEG_Z)
        .with_y(0.0)
        .normalize_or_zero();
    let right = (transform.rotation * Vec3::X)
        .with_y(0.0)
        .normalize_or_zero();
    let mut movement = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        movement += forward;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        movement -= forward;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        movement -= right;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        movement += right;
    }
    if keys.pressed(KeyCode::KeyQ) {
        movement += Vec3::Y;
    }
    if keys.pressed(KeyCode::KeyE) {
        movement -= Vec3::Y;
    }
    if movement != Vec3::ZERO {
        transform.translation += movement.normalize() * 160.0 * time.delta_secs();
        let center = if world.stage == WorldTileStage::Live {
            world.position
        } else {
            Vec3::ZERO
        };
        transform.translation.x = transform
            .translation
            .x
            .clamp(center.x - 480.0, center.x + 480.0);
        transform.translation.y = transform
            .translation
            .y
            .clamp(center.y + 18.0, center.y + 520.0);
        transform.translation.z = transform
            .translation
            .z
            .clamp(center.z - 480.0, center.z + 620.0);
    }
}

#[cfg(target_arch = "wasm32")]
fn poll_world_entry(mut probe: ResMut<WorldTileProbe>, asset_server: Res<AssetServer>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let key = JsValue::from_str("benillaWorldEntry");
    let Ok(entry) = js_sys::Reflect::get(window.as_ref(), &key) else {
        return;
    };
    if entry.is_undefined() || entry.is_null() {
        return;
    }
    let _ = js_sys::Reflect::delete_property(window.as_ref(), &key);
    let decoded = (|| {
        let map = js_sys::Reflect::get(&entry, &JsValue::from_str("map"))
            .ok()?
            .as_f64()? as u32;
        let coordinates = js_sys::Reflect::get(&entry, &JsValue::from_str("position")).ok()?;
        let coordinates = js_sys::Array::from(&coordinates);
        if coordinates.length() != 3 {
            return None;
        }
        let position = [
            coordinates.get(0).as_f64()? as f32,
            coordinates.get(1).as_f64()? as f32,
            coordinates.get(2).as_f64()? as f32,
        ];
        position
            .iter()
            .all(|value| value.is_finite())
            .then_some((map, position))
    })();
    let Some((map, position)) = decoded else {
        error!("N4 world entry has invalid map or coordinates");
        return;
    };
    let map_name = match map {
        0 => "Azeroth",
        1 => "Kalimdor",
        _ => {
            error!("N4 map {map} is not a supported vanilla continent yet");
            return;
        }
    };
    let (tile_x, tile_y) = benilla_formats::world_to_tile(position[0], position[1]);
    if position[0].abs() > 32.0 * 533.333_3 || position[1].abs() > 32.0 * 533.333_3 {
        error!("N4 position {:?} is outside the continent grid", position);
        return;
    }
    probe.map_name = map_name.to_owned();
    probe.tile = (tile_x, tile_y);
    probe.position = wow_to_bevy(position);
    probe.wdt = Some(asset_server.load(format!("mpq://world/maps/{map_name}/{map_name}.wdt")));
    probe.adt = None;
    probe.stage = WorldTileStage::LoadingWdt;
    info!(
        "N4 loading server location: map {map} {map_name} tile {}_{}, wow={position:?}",
        probe.tile.0, probe.tile.1
    );
}

fn drive_world_tile(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut probe: ResMut<WorldTileProbe>,
    wdts: Res<Assets<WdtIndex>>,
    adts: Res<Assets<AdtTile>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WebTerrainMaterial>>,
    mut camera: Query<&mut Transform, With<ProbeCamera>>,
) {
    match probe.stage {
        WorldTileStage::LoadingWdt => {
            let Some(handle) = probe.wdt.as_ref() else {
                return;
            };
            match asset_server.load_state(handle) {
                LoadState::Loaded => {
                    let Some(wdt) = wdts.get(handle) else { return };
                    if !wdt.has_tile(probe.tile.0, probe.tile.1) {
                        error!("N4 WDT has no tile {}_{}", probe.tile.0, probe.tile.1);
                        probe.stage = WorldTileStage::Failed;
                        return;
                    }
                    probe.adt = Some(asset_server.load(format!(
                        "mpq://world/maps/{0}/{0}_{1}_{2}.adt",
                        probe.map_name, probe.tile.0, probe.tile.1
                    )));
                    probe.stage = WorldTileStage::LoadingAdt;
                }
                LoadState::Failed(ref error) => {
                    error!("N4 WDT load failed: {error}");
                    probe.stage = WorldTileStage::Failed;
                }
                _ => {}
            }
        }
        WorldTileStage::LoadingAdt => {
            let Some(handle) = probe.adt.as_ref() else {
                return;
            };
            match asset_server.load_state(handle) {
                LoadState::Loaded => {
                    let Some(adt) = adts.get(handle) else { return };
                    let chunks: Vec<_> = adt.chunks.iter().zip(&adt.shading).collect();
                    let Some(mesh) = chunks_to_mesh(&chunks) else {
                        error!("N4 ADT has no terrain mesh");
                        probe.stage = WorldTileStage::Failed;
                        return;
                    };
                    if let Some(old) = probe.entity.take() {
                        commands.entity(old).try_despawn();
                    }
                    let mut light = WebLightUniform::default();
                    light.fog_params = Vec4::new(300.0, 760.0, 0.0, 920.0);
                    let material = materials.add(ExtendedMaterial {
                        base: StandardMaterial {
                            base_color: Color::WHITE,
                            perceptual_roughness: 1.0,
                            ..default()
                        },
                        extension: WebTerrainExtension {
                            light,
                            layer_array: adt.layer_array.clone(),
                            alpha_array: adt.alpha_array.clone(),
                            shadow_array: adt.shadow_array.clone(),
                            params: Vec4::new(8.0, 0.0, 0.0, 0.0),
                        },
                    });
                    probe.entity = Some(
                        commands
                            .spawn((
                                Mesh3d(meshes.add(mesh)),
                                MeshMaterial3d(material),
                                Transform::default(),
                                Name::new("N4 server-position terrain"),
                            ))
                            .id(),
                    );
                    if let Ok(mut camera) = camera.single_mut() {
                        let focus = probe.position;
                        *camera = Transform::from_translation(focus + Vec3::new(0.0, 28.0, 42.0))
                            .looking_at(focus, Vec3::Y);
                    }
                    probe.stage = WorldTileStage::Live;
                    info!(
                        "N4 LIVE: server tile {}_{} in {}",
                        probe.tile.0, probe.tile.1, probe.map_name
                    );
                }
                LoadState::Failed(ref error) => {
                    error!("N4 ADT load failed: {error}");
                    probe.stage = WorldTileStage::Failed;
                }
                _ => {}
            }
        }
        WorldTileStage::Idle | WorldTileStage::Live | WorldTileStage::Failed => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_a2_terrain(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    asset_server: Res<AssetServer>,
    stats: Res<WebAssetStats>,
    mut probe: ResMut<A2TerrainProbe>,
    wdts: Res<Assets<WdtIndex>>,
    adts: Res<Assets<AdtTile>>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WebTerrainMaterial>>,
    world: Res<WorldTileProbe>,
    mut camera: Query<&mut Transform, With<ProbeCamera>>,
    scaffold: Query<Entity, With<ProbeScaffold>>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        match probe.stage {
            A2Stage::Live => {
                if let Some(entity) = probe.entity.take() {
                    commands.entity(entity).try_despawn();
                }
                probe.adt = None;
                probe.wdt = None;
                probe.stage = A2Stage::Unloading(4);
                info!("A2 unload requested; waiting for Bevy asset cleanup");
                return;
            }
            A2Stage::Unloaded | A2Stage::Failed => {
                probe.generation += 1;
                probe.wdt = Some(asset_server.load(A2_WDT));
                probe.adt = None;
                probe.stage = A2Stage::LoadingWdt;
                info!(
                    "A2 generation {}: reloading WDT/ADT (the web reader should report cache HITs)",
                    probe.generation
                );
                return;
            }
            A2Stage::Unloading(_) => {
                info!("A2 is still releasing assets; press R again after the unload report");
                return;
            }
            A2Stage::LoadingWdt | A2Stage::LoadingAdt => {
                info!("A2 load is already in progress");
                return;
            }
        }
    }

    match probe.stage {
        A2Stage::LoadingWdt => {
            let Some(handle) = probe.wdt.clone() else {
                return;
            };
            match asset_server.load_state(handle.id()) {
                LoadState::Loaded => {
                    let Some(wdt) = wdts.get(&handle) else {
                        return;
                    };
                    if !wdt.has_tile(A2_TILE.0, A2_TILE.1) {
                        error!(
                            "A2 WDT says Azeroth tile {}_{} is absent",
                            A2_TILE.0, A2_TILE.1
                        );
                        probe.stage = A2Stage::Failed;
                        return;
                    }
                    info!(
                        "A2 WDT loaded and confirms tile {}_{}",
                        A2_TILE.0, A2_TILE.1
                    );
                    probe.adt = Some(asset_server.load(A2_ADT));
                    probe.stage = A2Stage::LoadingAdt;
                }
                LoadState::Failed(ref error) => {
                    error!("A2 WDT load failed: {error}");
                    probe.stage = A2Stage::Failed;
                }
                _ => {}
            }
        }
        A2Stage::LoadingAdt => {
            let Some(handle) = probe.adt.clone() else {
                return;
            };
            match asset_server.load_state(handle.id()) {
                LoadState::Loaded => {
                    let Some(adt) = adts.get(&handle) else {
                        return;
                    };
                    let parts: Vec<_> = adt.chunks.iter().zip(&adt.shading).collect();
                    let Some(mesh) = chunks_to_mesh(&parts) else {
                        error!("A2 ADT decoded, but its terrain mesh is empty");
                        probe.stage = A2Stage::Failed;
                        return;
                    };
                    // bevy 0.18 dropped `Mesh::compute_aabb`; same math through
                    // `Aabb3d::from_point_cloud` over the position attribute (still
                    // `None` when the mesh carries no computable positions).
                    let bounds = mesh
                        .attribute(Mesh::ATTRIBUTE_POSITION)
                        .and_then(|positions| match positions {
                            bevy::mesh::VertexAttributeValues::Float32x3(points)
                                if !points.is_empty() =>
                            {
                                Some(bevy::math::bounding::Aabb3d::from_point_cloud(
                                    bevy::math::Isometry3d::IDENTITY,
                                    points.iter().copied(),
                                ))
                            }
                            _ => None,
                        });
                    let Some(bounds) = bounds else {
                        error!("A2 ADT mesh has no computable bounds");
                        probe.stage = A2Stage::Failed;
                        return;
                    };
                    let center = Vec3::from((bounds.min + bounds.max) * 0.5);
                    let half = Vec3::from((bounds.max - bounds.min) * 0.5);
                    let world_offset = Vec3::new(-center.x, -(center.y - half.y), -center.z);
                    let transform = Transform::from_translation(world_offset);
                    let mesh_bytes = mesh.count_vertices() * 56
                        + mesh.indices().map_or(0, |indices| indices.len() * 4);
                    let array_bytes = [&adt.layer_array, &adt.alpha_array, &adt.shadow_array]
                        .into_iter()
                        .filter_map(|image| images.get(image))
                        .filter_map(|image| image.data.as_ref())
                        .map(Vec::len)
                        .sum::<usize>();
                    let decoded_bytes = decoded_adt_bytes(adt);
                    let mut terrain_light = WebLightUniform::default();
                    terrain_light.fog_params = Vec4::new(300.0, 760.0, 0.0, 920.0);
                    let material = materials.add(ExtendedMaterial {
                        base: StandardMaterial {
                            base_color: Color::WHITE,
                            perceptual_roughness: 1.0,
                            ..default()
                        },
                        extension: WebTerrainExtension {
                            light: terrain_light,
                            layer_array: adt.layer_array.clone(),
                            alpha_array: adt.alpha_array.clone(),
                            shadow_array: adt.shadow_array.clone(),
                            params: Vec4::new(8.0, 0.0, 0.0, 0.0),
                        },
                    });
                    let entity = commands
                        .spawn((
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(material),
                            transform,
                            A2Terrain,
                            Name::new("A2 Azeroth 31_49 terrain"),
                        ))
                        .id();
                    for entity in &scaffold {
                        commands.entity(entity).try_despawn();
                    }
                    if world.stage == WorldTileStage::Idle {
                        if let Ok(mut camera) = camera.single_mut() {
                            let horizontal = half.x.max(half.z);
                            let eye = Vec3::new(
                                0.0,
                                (half.y * 1.8 + 135.0).max(190.0),
                                (horizontal * 1.18).max(430.0),
                            );
                            *camera = Transform::from_translation(eye)
                                .looking_at(Vec3::new(0.0, half.y * 0.45, 0.0), Vec3::Y);
                        }
                    }
                    probe.entity = Some(entity);
                    probe.world_offset = world_offset;
                    probe.stage = A2Stage::Live;
                    let net = stats.snapshot();
                    info!(
                        "A2 LIVE generation {}: chunks={}, doodads={} (available to A3), wmos={} (available to A3), decoded_cpu~{}, mesh_gpu~{}, texture_gpu~{}, HTTP={} fetches/{} bytes, WASM cache={} hits/{} entries/{} bytes; press R to unload",
                        probe.generation,
                        adt.chunks.len(),
                        adt.doodads.len(),
                        adt.wmos.len(),
                        human_bytes(decoded_bytes),
                        human_bytes(mesh_bytes),
                        human_bytes(array_bytes),
                        net.fetches,
                        net.transferred_bytes,
                        net.cache_hits,
                        net.cache_entries,
                        net.cache_resident_bytes,
                    );
                }
                LoadState::Failed(ref error) => {
                    error!("A2 ADT load failed: {error}");
                    probe.stage = A2Stage::Failed;
                }
                _ => {}
            }
        }
        A2Stage::Unloading(frames) if frames > 1 => {
            probe.stage = A2Stage::Unloading(frames - 1);
        }
        A2Stage::Unloading(_) => {
            probe.stage = A2Stage::Unloaded;
            let net = stats.snapshot();
            info!(
                "A2 UNLOADED: Bevy assets now adt={}, images={}, meshes={}, materials={}; WASM byte cache intentionally remains {} entries/{} bytes (bounded, enables reload). Press R to reload",
                adts.len(),
                images.len(),
                meshes.len(),
                materials.len(),
                net.cache_entries,
                net.cache_resident_bytes,
            );
        }
        A2Stage::Live | A2Stage::Unloaded | A2Stage::Failed => {}
    }
}

const A3_BATCHES_PER_FRAME: usize = 8;

#[allow(clippy::too_many_arguments)]
fn drive_a3_models(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    stats: Res<WebAssetStats>,
    terrain: Res<A2TerrainProbe>,
    adts: Res<Assets<AdtTile>>,
    m2s: Res<Assets<M2Model>>,
    wmos: Res<Assets<WmoModel>>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WebModelMaterial>>,
    mut probe: ResMut<A3ModelProbe>,
    world: Res<WorldTileProbe>,
    mut camera: Query<&mut Transform, With<ProbeCamera>>,
) {
    if terrain.stage != A2Stage::Live {
        match probe.stage {
            A3Stage::Waiting => {}
            A3Stage::Releasing(frames) if frames > 1 => {
                probe.stage = A3Stage::Releasing(frames - 1);
            }
            A3Stage::Releasing(_) => {
                probe.stage = A3Stage::Waiting;
                info!(
                    "A3 UNLOADED: m2={}, wmo={}, images={}, meshes={}, web_model_materials={}",
                    m2s.len(),
                    wmos.len(),
                    images.len(),
                    meshes.len(),
                    materials.len(),
                );
            }
            _ => {
                for entity in probe.entities.drain(..) {
                    commands.entity(entity).try_despawn();
                }
                probe.m2 = None;
                probe.wmo = None;
                probe.next_m2_batch = 0;
                probe.next_wmo_batch = 0;
                probe.fallbacks = A3FallbackStats::default();
                probe.stage = A3Stage::Releasing(4);
                info!("A3 model release requested with the terrain generation");
            }
        }
        return;
    }

    if probe.generation != terrain.generation {
        let Some(adt_handle) = terrain.adt.as_ref() else {
            return;
        };
        let Some(adt) = adts.get(adt_handle) else {
            return;
        };
        let Some(wmo) = adt.wmos.iter().find(|w| {
            w.model
                .to_ascii_lowercase()
                .contains("abbeygate\\abbeygate01.wmo")
        }) else {
            error!("A3 could not find the abbey-gate placement in tile 31_49");
            probe.stage = A3Stage::Failed;
            probe.generation = terrain.generation;
            return;
        };
        let wmo_world = wow_to_bevy(wmo.position) + terrain.world_offset;
        let Some(doodad) = adt
            .doodads
            .iter()
            .filter(|d| d.model.to_ascii_lowercase().contains("elwynntreemid01.mdx"))
            .min_by(|a, b| {
                let a = wow_to_bevy(a.position) + terrain.world_offset;
                let b = wow_to_bevy(b.position) + terrain.world_offset;
                a.xz()
                    .distance_squared(wmo_world.xz())
                    .total_cmp(&b.xz().distance_squared(wmo_world.xz()))
            })
        else {
            error!("A3 could not find the ElwynnTreeMid01 placement in tile 31_49");
            probe.stage = A3Stage::Failed;
            probe.generation = terrain.generation;
            return;
        };

        probe.generation = terrain.generation;
        probe.m2_label = doodad.model.clone();
        probe.wmo_label = wmo.model.clone();
        probe.m2_transform = Transform {
            translation: wow_to_bevy(doodad.position) + terrain.world_offset,
            rotation: placement_rotation(doodad.rotation),
            scale: Vec3::splat(doodad.scale),
        };
        probe.wmo_transform = Transform {
            translation: wow_to_bevy(wmo.position) + terrain.world_offset,
            rotation: placement_rotation(wmo.rotation),
            scale: Vec3::ONE,
        };
        probe.m2 = Some(asset_server.load(m2_url(&doodad.model)));
        probe.wmo = Some(asset_server.load(wmo_url(&wmo.model)));
        probe.next_m2_batch = 0;
        probe.next_wmo_batch = 0;
        probe.fallbacks = A3FallbackStats::default();
        probe.stage = A3Stage::Loading;
        info!(
            "A3 generation {}: loading M2 '{}' and WMO '{}' from their real ADT placements",
            probe.generation, probe.m2_label, probe.wmo_label
        );
        return;
    }

    match probe.stage {
        A3Stage::Loading => {
            let (Some(m2_handle), Some(wmo_handle)) = (probe.m2.clone(), probe.wmo.clone()) else {
                return;
            };
            let m2_state = asset_server.load_state(m2_handle.id());
            let wmo_state = asset_server.load_state(wmo_handle.id());
            if let LoadState::Failed(error) = &m2_state {
                error!("A3 M2 load failed for '{}': {error}", probe.m2_label);
                probe.stage = A3Stage::Failed;
                return;
            }
            if let LoadState::Failed(error) = &wmo_state {
                error!("A3 WMO load failed for '{}': {error}", probe.wmo_label);
                probe.stage = A3Stage::Failed;
                return;
            }
            if matches!(m2_state, LoadState::Loaded) && matches!(wmo_state, LoadState::Loaded) {
                let (Some(m2), Some(wmo)) = (m2s.get(&m2_handle), wmos.get(&wmo_handle)) else {
                    return;
                };
                info!(
                    "A3 assets decoded: M2 batches={}, WMO batches={}; spawning at most {} per frame",
                    m2.submeshes.len(),
                    wmo.submeshes.len(),
                    A3_BATCHES_PER_FRAME,
                );
                probe.stage = A3Stage::Spawning;
            }
        }
        A3Stage::Spawning => {
            let (Some(m2_handle), Some(wmo_handle)) = (probe.m2.clone(), probe.wmo.clone()) else {
                return;
            };
            let (Some(m2), Some(wmo)) = (m2s.get(&m2_handle), wmos.get(&wmo_handle)) else {
                return;
            };
            let mut budget = A3_BATCHES_PER_FRAME;
            while budget > 0 && probe.next_m2_batch < m2.submeshes.len() {
                let batch = probe.next_m2_batch;
                probe.fallbacks.record(&m2.submeshes[batch]);
                let entity = spawn_web_model_batch(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &m2.submeshes[batch],
                    probe.m2_transform,
                    format!("A3 M2 {} batch {batch}", probe.m2_label),
                );
                probe.entities.push(entity);
                probe.next_m2_batch += 1;
                budget -= 1;
            }
            while budget > 0 && probe.next_wmo_batch < wmo.submeshes.len() {
                let batch = probe.next_wmo_batch;
                probe.fallbacks.record(&wmo.submeshes[batch]);
                let entity = spawn_web_model_batch(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &wmo.submeshes[batch],
                    probe.wmo_transform,
                    format!("A3 WMO {} batch {batch}", probe.wmo_label),
                );
                probe.entities.push(entity);
                probe.next_wmo_batch += 1;
                budget -= 1;
            }
            if probe.next_m2_batch == m2.submeshes.len()
                && probe.next_wmo_batch == wmo.submeshes.len()
            {
                probe.stage = A3Stage::Live;
                let midpoint =
                    (probe.m2_transform.translation + probe.wmo_transform.translation) * 0.5;
                let separation = probe
                    .m2_transform
                    .translation
                    .xz()
                    .distance(probe.wmo_transform.translation.xz());
                if world.stage == WorldTileStage::Idle {
                    if let Ok(mut camera) = camera.single_mut() {
                        let eye = midpoint
                            + Vec3::new(
                                separation.max(70.0) * 0.55,
                                separation.max(70.0) * 0.75,
                                separation.max(70.0) * 1.25,
                            );
                        *camera = Transform::from_translation(eye)
                            .looking_at(midpoint + Vec3::Y * 12.0, Vec3::Y);
                    }
                }
                let net = stats.snapshot();
                info!(
                    "A3 LIVE generation {}: M2 {}/{} batches at {:?}, WMO {}/{} batches at {:?}, fallbacks={{batches:{}, multiply:{}, billboard_env:{}, depth:{}, material_anim:{}, wmo_light:{}, missing_texture:{}}}, HTTP={} fetches/{} bytes, cache={} hits/{} entries/{} bytes",
                    probe.generation,
                    probe.next_m2_batch,
                    m2.submeshes.len(),
                    probe.m2_transform.translation,
                    probe.next_wmo_batch,
                    wmo.submeshes.len(),
                    probe.wmo_transform.translation,
                    probe.fallbacks.batches,
                    probe.fallbacks.multiply_blend,
                    probe.fallbacks.billboard_or_env_map,
                    probe.fallbacks.custom_depth,
                    probe.fallbacks.animated_material,
                    probe.fallbacks.wmo_lighting,
                    probe.fallbacks.missing_texture,
                    net.fetches,
                    net.transferred_bytes,
                    net.cache_hits,
                    net.cache_entries,
                    net.cache_resident_bytes,
                );
            }
        }
        A3Stage::Waiting | A3Stage::Live | A3Stage::Releasing(_) | A3Stage::Failed => {}
    }
}

fn decoded_adt_bytes(adt: &AdtTile) -> usize {
    let mut bytes = std::mem::size_of_val(adt.doodads.as_slice())
        + std::mem::size_of_val(adt.wmos.as_slice())
        + std::mem::size_of_val(adt.shading.as_slice());
    for chunk in &adt.chunks {
        bytes += std::mem::size_of_val(chunk.positions.as_slice());
        bytes += std::mem::size_of_val(chunk.normals.as_slice());
        bytes += std::mem::size_of_val(chunk.uvs.as_slice());
        bytes += std::mem::size_of_val(chunk.indices.as_slice());
        bytes += chunk.alpha_map.as_ref().map_or(0, Vec::len);
        bytes += chunk.shadow.as_ref().map_or(0, Vec::len);
        bytes += chunk.layer_textures.iter().map(String::len).sum::<usize>();
    }
    bytes
}

fn human_bytes(bytes: usize) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    format!("{:.2} MiB", bytes as f64 / MIB)
}

fn terrain_mesh() -> Mesh {
    const COLUMNS: u32 = 49;
    const ROWS: u32 = 35;
    let mut positions = Vec::with_capacity((COLUMNS * ROWS) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut uv1 = Vec::with_capacity(positions.capacity());
    let mut colors = Vec::with_capacity(positions.capacity());

    for row in 0..ROWS {
        let v = row as f32 / (ROWS - 1) as f32;
        let z = (v - 0.5) * TERRAIN_DEPTH;
        for column in 0..COLUMNS {
            let u = column as f32 / (COLUMNS - 1) as f32;
            let x = (u - 0.5) * TERRAIN_WIDTH;
            let height = terrain_height(x, z);
            let dx = 0.22 * (x * 0.035).cos() * (z * 0.045).cos();
            let dz = -0.27 * (x * 0.035).sin() * (z * 0.045).sin();
            positions.push([x, height, z]);
            normals.push(Vec3::new(-dx, 1.0, -dz).normalize().to_array());
            uvs.push([u, v]);
            uv1.push([0.0, 0.0]);
            colors.push([0.0, 1.0, 2.0, 3.0]);
        }
    }

    let mut indices = Vec::with_capacity(((COLUMNS - 1) * (ROWS - 1) * 6) as usize);
    for row in 0..ROWS - 1 {
        for column in 0..COLUMNS - 1 {
            let top_left = row * COLUMNS + column;
            let bottom_left = (row + 1) * COLUMNS + column;
            indices.extend_from_slice(&[
                top_left,
                bottom_left,
                top_left + 1,
                top_left + 1,
                bottom_left,
                bottom_left + 1,
            ]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn terrain_height(x: f32, z: f32) -> f32 {
    (x * 0.035).sin() * (z * 0.045).cos() * 6.0
}

fn layer_array() -> Image {
    let mut data = Vec::with_capacity((ARRAY_SIZE * ARRAY_SIZE * 4 * 4) as usize);
    let layers = [
        ([50_i16, 102, 45], 8_u8),
        ([116, 74, 40], 18),
        ([105, 112, 124], 150),
        ([189, 166, 108], 38),
    ];
    for (layer_index, (base, sheen)) in layers.into_iter().enumerate() {
        for y in 0..ARRAY_SIZE {
            for x in 0..ARRAY_SIZE {
                let checker = (((x / 8 + y / 8 + layer_index as u32) & 1) as i16 * 16) - 8;
                let grain = ((x * 17 + y * 29 + layer_index as u32 * 11) % 17) as i16 - 8;
                for channel in base {
                    data.push((channel + checker + grain).clamp(0, 255) as u8);
                }
                data.push(sheen);
            }
        }
    }
    array_image(data, 4, TextureFormat::Rgba8Unorm, true)
}

fn alpha_array() -> Image {
    let mut layer = Vec::with_capacity((ARRAY_SIZE * ARRAY_SIZE * 4) as usize);
    for y in 0..ARRAY_SIZE {
        for x in 0..ARRAY_SIZE {
            let uv = Vec2::new(
                x as f32 / (ARRAY_SIZE - 1) as f32,
                y as f32 / (ARRAY_SIZE - 1) as f32,
            );
            let dirt = smoothstep(0.18, 0.78, uv.x + (uv.y * 11.0).sin() * 0.08);
            let rock = (1.0 - uv.distance(Vec2::new(0.48, 0.48)) / 0.36).clamp(0.0, 1.0);
            let sand = smoothstep(0.48, 0.92, 1.0 - uv.y) * (1.0 - rock * 0.8);
            layer.extend_from_slice(&[
                (dirt * 255.0) as u8,
                (rock * 255.0) as u8,
                (sand * 255.0) as u8,
                255,
            ]);
        }
    }
    let mut data = layer.clone();
    data.extend_from_slice(&layer);
    array_image(data, 2, TextureFormat::Rgba8Unorm, false)
}

fn shadow_array() -> Image {
    let mut layer = Vec::with_capacity((ARRAY_SIZE * ARRAY_SIZE) as usize);
    for y in 0..ARRAY_SIZE {
        for x in 0..ARRAY_SIZE {
            let uv = Vec2::new(x as f32, y as f32) / ARRAY_SIZE as f32;
            let cloud = (uv.x * 17.0).sin() * (uv.y * 13.0).cos() * 0.12 + 0.18;
            let diagonal = (uv.y - (0.72 - uv.x * 0.55)).abs();
            let ridge = (1.0 - diagonal / 0.09).clamp(0.0, 1.0) * 0.58;
            layer.push(((cloud + ridge).clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    let mut data = layer.clone();
    data.extend_from_slice(&layer);
    array_image(data, 2, TextureFormat::R8Unorm, false)
}

fn array_image(data: Vec<u8>, layers: u32, format: TextureFormat, repeat: bool) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: ARRAY_SIZE,
            height: ARRAY_SIZE,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let address_mode = if repeat {
        ImageAddressMode::Repeat
    } else {
        ImageAddressMode::ClampToEdge
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

fn synthetic_bc1_texture() -> BlpMipChain {
    const SIZE: u32 = 64;
    let mut blocks = Vec::with_capacity(((SIZE / 4) * (SIZE / 4) * 8) as usize);
    for y in 0..SIZE / 4 {
        for x in 0..SIZE / 4 {
            let color = if (x + y) & 1 == 0 {
                rgb565(36, 132, 202)
            } else {
                rgb565(229, 112, 38)
            };
            blocks.extend_from_slice(&color.to_le_bytes());
            blocks.extend_from_slice(&color.to_le_bytes());
            blocks.extend_from_slice(&0_u32.to_le_bytes());
        }
    }
    BlpMipChain {
        width: SIZE,
        height: SIZE,
        texels: BlpTexels::Bc1,
        mips: vec![blocks],
    }
}

fn rgb565(red: u8, green: u8, blue: u8) -> u16 {
    (u16::from(red) >> 3) << 11 | (u16::from(green) >> 2) << 5 | (u16::from(blue) >> 3)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

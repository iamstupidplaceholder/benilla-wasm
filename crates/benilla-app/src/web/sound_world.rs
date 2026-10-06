//! World audio host transport, catalog delivery and channel lifetime pump.
use super::{listener_pose, math, Admission, SoundConfig};
use benilla_assets::RawFile;
use benilla_formats::MemoryFileSource;
use benilla_world::{schedule::WorldStage, view::WorldCamera};
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(super) struct AudioListener {
    pub(super) pos: Vec3,
    pub(super) rot: Quat,
}
#[derive(Resource)]
pub(super) struct Materials(pub(super) benilla_formats::MaterialCatalog);
pub(super) use super::listener_pose::worn_chest_material;

pub(super) struct Channel {
    pub(super) category: super::kit::SoundCategory,
    pub(super) category_amp: f32,
    pub(super) wet: bool,
    pub(super) gain: f32,
    pub(super) latch: super::kit::Latch,
    pub(super) ui: bool,
    pub(super) kit: u32,
    pub(super) source: Option<Entity>,
    pub(super) pos: Option<Vec3>,
    pub(super) min_dist: f32,
    pub(super) cutoff: f32,
    pub(super) volume: f32,
    pub(super) bus: u8,
    pub(super) playing: bool,
    pub(super) cancelled: bool,
}
pub(super) struct SoundOutput {
    pub(super) mixer: Option<super::mixer::Mixer>,
    pub(super) glue_streams: usize,
    pub(super) zone_streams: usize,
    pub(super) cinematic_streams: usize,
    pub(super) channels: std::collections::HashMap<u32, Channel>,
    listener: AudioListener,
}
impl Default for SoundOutput {
    fn default() -> Self {
        Self {
            mixer: Some(super::mixer::Mixer),
            glue_streams: 0,
            zone_streams: 0,
            cinematic_streams: 0,
            channels: Default::default(),
            listener: Default::default(),
        }
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
import { audio } from '../../../audio/host.js';
export function worldSoundListener(x,y,z,qx,qy,qz,qw) { audio.listener({x,y,z,qx,qy,qz,qw}); }
export function worldSoundUpdate(id,amp,x,y,z) { audio.update({id,amp,x,y,z}); }
export function worldSoundStop(id) { audio.stop(id); }
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=worldSoundListener)]
    fn send_listener(x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=worldSoundUpdate)]
    fn send_update(id: u32, amp: f32, x: f32, y: f32, z: f32);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=worldSoundStop)]
    fn send_stop(id: u32);
}
pub(super) fn amp(channel: &Channel, listener: Vec3) -> f32 {
    channel.volume
        * channel.gain
        * channel.category_amp
        * channel.pos.map_or(1.0, |p| {
            let d = math::dist_sq(listener, p);
            math::fmod_rolloff(d, channel.min_dist)
                * if channel.cutoff > 0.0 {
                    math::near_field_atten(d, channel.cutoff)
                } else {
                    1.0
                }
        })
}
pub(super) fn initial_amp(out: &SoundOutput, channel: &Channel) -> f32 {
    amp(channel, out.listener.pos)
}
pub(super) fn send_channel(out: &SoundOutput, id: u32) {
    if let Some(channel) = out.channels.get(&id) {
        let pos = channel.pos.unwrap_or_default();
        send_update(id, amp(channel, out.listener.pos), pos.x, pos.y, pos.z);
    }
}
pub(super) fn stop_channel(out: &mut SoundOutput, id: u32) {
    if let Some(c) = out.channels.get_mut(&id) {
        if !c.cancelled {
            c.cancelled = true;
            send_stop(id);
        }
    }
}
pub(super) fn stop_ui(out: &mut SoundOutput) {
    let ids: Vec<_> = out
        .channels
        .iter()
        .filter(|(_, c)| c.ui)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        stop_channel(out, id);
    }
}

const FILES: &[&str] = &[
    "DBFilesClient/NPCSounds.dbc",
    "DBFilesClient/TerrainTypeSounds.dbc",
    "DBFilesClient/DeathThudLookups.dbc",
    "DBFilesClient/GameObjectDisplayInfo.dbc",
    "DBFilesClient/SheatheSoundLookups.dbc",
    "DBFilesClient/SoundWaterType.dbc",
    "DBFilesClient/SoundProviderPreferences.dbc",
    "DBFilesClient/VocalUISounds.dbc",
    "DBFilesClient/ItemGroupSounds.dbc",
    "DBFilesClient/CreatureSoundData.dbc",
    "DBFilesClient/CreatureDisplayInfo.dbc",
    "DBFilesClient/CreatureModelData.dbc",
    "DBFilesClient/Material.dbc",
    "DBFilesClient/WeaponSwingSounds2.dbc",
    "DBFilesClient/WeaponImpactSounds.dbc",
    "DBFilesClient/AreaTable.dbc",
    "DBFilesClient/ZoneMusic.dbc",
    "DBFilesClient/ZoneIntroMusicTable.dbc",
    "DBFilesClient/SoundAmbience.dbc",
    "DBFilesClient/ChrRaces.dbc",
];
#[derive(Resource)]
struct Delivery(Vec<(String, Handle<RawFile>)>);
fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Delivery(
        FILES
            .iter()
            .map(|p| (p.to_string(), server.load(format!("mpq://{p}"))))
            .collect(),
    ));
}
fn publish(
    mut commands: Commands,
    delivery: Option<Res<Delivery>>,
    files: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
) {
    let Some(delivery) = delivery else {
        return;
    };
    if delivery.0.iter().any(|(_, h)| {
        files.get(h).is_none()
            && !matches!(
                server.get_load_state(h.id()),
                Some(bevy::asset::LoadState::Failed(_))
            )
    }) {
        return;
    }
    let mut source = MemoryFileSource::new(
        delivery
            .0
            .iter()
            .filter_map(|(p, h)| files.get(h).map(|f| (p.clone(), f.0.clone()))),
    );
    match benilla_formats::load_creature_voice_catalog(&mut source) {
        Ok(c) => {
            info!("sound: {} original creature voice rows delivered", c.len());
            commands.insert_resource(super::creature::CreatureVoices(c));
        }
        Err(e) => warn!("sound: creature voice catalog failed: {e:#}"),
    }
    match benilla_formats::load_material_catalog(&mut source) {
        Ok(c) => {
            commands.insert_resource(Materials(c));
        }
        Err(e) => warn!("sound: material catalog failed: {e:#}"),
    }
    match benilla_formats::load_weapon_swing_catalog(&mut source) {
        Ok(c) => {
            commands.insert_resource(super::combat::WeaponSwings(c));
        }
        Err(e) => warn!("sound: weapon swing catalog failed: {e:#}"),
    }
    match benilla_formats::load_weapon_impact_catalog(&mut source) {
        Ok(c) => {
            commands.insert_resource(super::combat::WeaponImpacts(c));
        }
        Err(e) => warn!("sound: weapon impact catalog failed: {e:#}"),
    }
    commands.remove_resource::<Delivery>();
    macro_rules! catalog {
        ($loader:ident,$ty:path) => {
            match benilla_formats::$loader(&mut source) {
                Ok(c) => {
                    commands.insert_resource($ty(c));
                }
                Err(e) => warn!("sound: {}: {e:#}", stringify!($loader)),
            }
        };
    }
    catalog!(load_npc_greeting_catalog, super::greeting::NpcGreetings);
    catalog!(load_death_thud_catalog, super::death_thud::DeathThuds);
    catalog!(load_gameobject_sounds, super::gameobject::GoSounds);
    catalog!(load_sheathe_sound_catalog, super::sheathe::SheatheSounds);
    catalog!(load_water_sound_catalog, super::liquid_loop::WaterSounds);
    catalog!(load_sound_provider_catalog, super::reverb::SoundProviders);
    catalog!(load_vocal_ui_sounds, super::vocal::VocalUiSounds);
    catalog!(load_item_group_sounds, super::ui::ItemSounds);
    match benilla_formats::load_area_sound_catalog(&mut source) {
        Ok(c) => {
            commands.insert_resource(super::zone::AreaSounds(c));
        }
        Err(e) => warn!("sound: area audio catalog: {e:#}"),
    }
    match benilla_formats::load_exploration_sound_catalog(&mut source) {
        Ok(c) => {
            commands.insert_resource(super::zone::ExplorationSounds(c));
        }
        Err(e) => warn!("sound: exploration audio catalog: {e:#}"),
    }
}

fn pump(
    mut out: NonSendMut<SoundOutput>,
    mut listener: ResMut<AudioListener>,
    config: Res<SoundConfig>,
    player: Res<crate::player::Player>,
    cinematic: Option<Res<crate::cinematic::Cinematic>>,
    avatar: Query<(&Transform, Option<&crate::player::CameraPivot>), With<crate::net::Embodied>>,
    camera: Query<&Transform, (With<WorldCamera>, Without<crate::net::Embodied>)>,
    transforms: Query<&Transform, Without<Camera3d>>,
    mut admission: ResMut<Admission>,
    mut last_hold: Local<bool>,
) {
    if config.world_hold && !*last_hold {
        admission.epoch = super::sound_reset();
        admission.live.clear();
        out.channels.clear();
    }
    *last_hold = config.world_hold;
    if let Some((pos, rot)) = listener_pose::pose(
        config.listener_at_character,
        &player,
        cinematic.as_deref(),
        avatar.single().ok(),
        camera.single().ok(),
    ) {
        listener.pos = pos;
        listener.rot = rot;
        out.listener.pos = pos;
        out.listener.rot = rot;
        send_listener(pos.x, pos.y, pos.z, rot.x, rot.y, rot.z, rot.w);
    }
    let mut stop = Vec::new();
    for (&id, c) in &mut out.channels {
        if c.category != super::kit::SoundCategory::Sfx {
            c.category_amp = config.category_amp(c.category);
        }
        if c.cancelled {
            continue;
        }
        if let Some(source) = c.source {
            match transforms.get(source) {
                Ok(t) => {
                    if c.pos.is_some() {
                        c.pos = Some(t.translation);
                    }
                }
                Err(_) => {
                    stop.push(id);
                    continue;
                }
            }
        }
        if c.pos.is_some_and(|p| {
            c.cutoff > 0.0 && !math::audible(math::dist_sq(listener.pos, p), c.cutoff)
        }) {
            stop.push(id);
        }
    }
    for id in stop {
        stop_channel(&mut out, id);
    }
    for (&id, c) in &out.channels {
        if !c.cancelled {
            send_channel(&out, id);
        }
    }
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(Startup, request)
        .add_systems(Update, publish)
        .add_systems(
            Update,
            pump.in_set(WorldStage::Stream).after(super::play_ui_audio),
        );
}

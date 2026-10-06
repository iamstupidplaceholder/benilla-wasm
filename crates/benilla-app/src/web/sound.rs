//! Browser sound host adapter. Original Lua intents/catalogs/variation math feed
//! the original Kira renderer in a dedicated worker. Original SFX/music/ambience
//! systems use the same async host seam; streaming decode and mix are isolated
//! from both the main thread and the PCM AudioWorklet.

use benilla_assets::RawFile;
use benilla_formats::{MemoryFileSource, SoundKitCatalog};
use benilla_ui::script::{SoundRequest, UiScript};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::prelude::*;

#[path = "../sound/combat.rs"]
mod combat;
#[path = "../sound/cvars.rs"]
mod cvars;
use cvars::toggle_mute;
#[path = "../sound/footsteps.rs"]
pub(crate) mod footsteps;
#[path = "sound_kit.rs"]
mod kit;
#[path = "../sound/listener_pose.rs"]
mod listener_pose;
#[path = "../sound/math.rs"]
mod math;
#[path = "sound_mixer.rs"]
mod mixer;
#[path = "../sound/selection.rs"]
mod selection;
#[path = "../sound/spell.rs"]
mod spell;
#[path = "sound_world.rs"]
mod world;
type HostAssets = mixer::SoundAssets;
#[path = "../sound/cinematic.rs"]
mod cinematic;
#[path = "../sound/glue.rs"]
mod glue;
#[path = "../sound/interior.rs"]
pub(crate) mod interior;
#[path = "../sound/weather.rs"]
mod weather;
#[path = "../sound/zone.rs"]
mod zone;
use world::{worn_chest_material, AudioListener, Materials, SoundOutput};
#[path = "../sound/anim_events.rs"]
mod anim_events;
#[path = "../sound/creature.rs"]
mod creature;
#[path = "../sound/death_thud.rs"]
mod death_thud;
#[path = "../sound/emitter_pool.rs"]
mod emitter_pool;
#[path = "../sound/emote.rs"]
mod emote;
#[path = "../sound/gameobject.rs"]
mod gameobject;
#[path = "../sound/greeting.rs"]
mod greeting;
#[path = "../sound/liquid_loop.rs"]
mod liquid_loop;
#[path = "../sound/message.rs"]
mod message;
#[path = "../sound/missile.rs"]
mod missile;
#[path = "../sound/money.rs"]
mod money;
#[path = "../sound/mount.rs"]
mod mount;
#[path = "../sound/reverb.rs"]
mod reverb;
#[path = "../sound/sheathe.rs"]
mod sheathe;
#[path = "../sound/ui.rs"]
mod ui;
#[path = "../sound/vocal.rs"]
mod vocal;
#[path = "../sound/water.rs"]
mod water;

#[wasm_bindgen(inline_js = r#"
import { audio } from '../../../audio/host.js';
export function soundStatus() { return audio.status(); }
export function soundEvents() { return audio.takeEvents(); }
export function soundPlay(id, path, volume, frequency, looping, spatial, x,y,z,amp,preScaled,wet,protectedVoice) { return audio.play({id, path, volume, frequency, looping, spatial,x,y,z,amp,preScaled,wet,protected:protectedVoice}); }
export function soundDecode(path, bytes) { audio.decode(path, bytes); }
export function soundFail(path, error) { audio.fail(path, error); }
export function soundControls(master, sfx, enabled, gate, limiter) { audio.controls({master, sfx, enabled, gate, limiter}); }
export function soundReset() { return audio.reset(); }
export function soundDecodes() { return audio.decodes; }
export function soundStreamBytes(id,bytes){audio.streamBytes(id,bytes);}
export function soundStreamFail(id,error){audio.streamFail(id,error);}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = soundStatus)]
    fn sound_status() -> String;
    #[wasm_bindgen(js_name = soundEvents)]
    fn sound_events() -> String;
    #[wasm_bindgen(js_name = soundPlay)]
    fn sound_play(
        id: u32,
        path: &str,
        volume: f32,
        frequency: i32,
        looping: bool,
        spatial: bool,
        x: f32,
        y: f32,
        z: f32,
        amp: f32,
        pre_scaled: bool,
        wet: bool,
        protected: bool,
    ) -> bool;
    #[wasm_bindgen(js_name = soundDecode)]
    fn sound_decode(path: &str, bytes: &[u8]);
    #[wasm_bindgen(js_name = soundFail)]
    fn sound_fail(path: &str, error: &str);
    #[wasm_bindgen(js_name = soundControls)]
    fn sound_controls(master: f32, sfx: f32, enabled: bool, gate: bool, limiter: bool);
    #[wasm_bindgen(js_name = soundReset)]
    fn sound_reset() -> u32;
    #[wasm_bindgen(js_name = soundDecodes)]
    fn sound_decodes() -> u32;
    #[wasm_bindgen(js_name=soundStreamBytes)]
    fn sound_stream_bytes(id: u32, bytes: &[u8]);
    #[wasm_bindgen(js_name=soundStreamFail)]
    fn sound_stream_fail(id: u32, error: &str);
}

#[derive(Resource, Default)]
pub(crate) struct Admission {
    catalog_request: Option<Handle<RawFile>>,
    catalog: Option<SoundKitCatalog>,
    picker: selection::VariationPicker,
    files: HashMap<String, Handle<RawFile>>,
    failed: HashSet<String>,
    live: HashMap<u32, u32>,
    next_id: u32,
    epoch: u32,
    session: Option<u64>,
    stream_files: HashMap<String, Vec<u32>>,
    sfx_files: HashSet<String>,
}

#[derive(serde::Deserialize)]
struct AudioEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: u32,
    #[serde(default)]
    epoch: u32,
    path: Option<String>,
    error: Option<String>,
}

fn deliver_audio(
    mut admission: ResMut<Admission>,
    server: Res<AssetServer>,
    files: Res<Assets<RawFile>>,
    mut out: NonSendMut<SoundOutput>,
) {
    if admission.catalog_request.is_none() {
        admission.catalog_request = Some(server.load("mpq://DBFilesClient/SoundEntries.dbc"));
    }
    if admission.catalog.is_none() && !admission.failed.contains("catalog") {
        let handle = admission.catalog_request.as_ref().expect("requested");
        if let Some(file) = files.get(handle) {
            let mut source = MemoryFileSource::new([(
                "DBFilesClient/SoundEntries.dbc".to_owned(),
                file.0.clone(),
            )]);
            match benilla_formats::load_sound_kit_catalog(&mut source) {
                Ok(catalog) => {
                    info!("sound: {} original kits admitted", catalog.len());
                    admission.catalog = Some(catalog);
                }
                Err(error) => {
                    error!("sound: SoundEntries decode failed: {error:#}");
                    admission.failed.insert("catalog".into());
                }
            }
        } else if let Some(bevy::asset::LoadState::Failed(error)) =
            server.get_load_state(handle.id())
        {
            if admission.failed.insert("catalog".into()) {
                error!("sound: SoundEntries delivery failed: {error}");
            }
        }
    }
    let events: Vec<AudioEvent> = match serde_json::from_str(&sound_events()) {
        Ok(events) => events,
        Err(error) => {
            error!("sound: invalid audio backend event: {error}");
            return;
        }
    };
    for event in events {
        match event.kind.as_str() {
            "need" => {
                if let Some(path) = event.path {
                    admission.sfx_files.insert(path.clone());
                    if admission.failed.contains(&path) {
                        sound_fail(&path, "previous terminal asset failure");
                    } else if admission.files.len() >= 32 {
                        sound_fail(&path, "audio delivery queue full");
                    } else {
                        admission
                            .files
                            .entry(path.clone())
                            .or_insert_with(|| server.load(format!("mpq://{path}")));
                    }
                }
            }
            "need-stream" => {
                if let Some(path) = event.path {
                    if admission.files.len() >= 8 {
                        sound_stream_fail(event.id, "stream delivery queue full");
                    } else {
                        admission
                            .stream_files
                            .entry(path.clone())
                            .or_default()
                            .push(event.id);
                        admission
                            .files
                            .entry(path.clone())
                            .or_insert_with(|| server.load(format!("mpq://{path}")));
                    }
                }
            }
            "ended" | "dropped" | "failed" | "stolen" if event.epoch == admission.epoch => {
                admission.live.remove(&event.id);
                out.channels.remove(&event.id);
                if let Some(error) = event.error {
                    warn!("sound: channel {} failed: {error}", event.id);
                }
            }
            "playing" if event.epoch == admission.epoch => {
                if let Some(c) = out.channels.get_mut(&event.id) {
                    c.playing = true;
                }
            }
            "fatal" => {
                admission.live.clear();
                out.channels.clear();
                error!(
                    "sound: browser output failed: {}",
                    event.error.unwrap_or_default()
                );
            }
            _ => {}
        }
    }
    let mut finished = Vec::new();
    for (path, handle) in &admission.files {
        if let Some(file) = files.get(handle) {
            if let Some(ids) = admission.stream_files.get(path) {
                for id in ids {
                    if file.0.len() > 32 * 1024 * 1024 {
                        sound_stream_fail(*id, "encoded stream exceeds 32 MiB budget");
                    } else {
                        sound_stream_bytes(*id, &file.0);
                    }
                }
            }
            if !admission.sfx_files.contains(path) {
                finished.push((path.clone(), false));
                continue;
            }
            if file.0.len() > 8 * 1024 * 1024 {
                sound_fail(path, "compressed SFX exceeds 8 MiB budget");
                finished.push((path.clone(), true));
            } else {
                sound_decode(path, &file.0);
                finished.push((path.clone(), false));
            }
        } else if let Some(bevy::asset::LoadState::Failed(error)) =
            server.get_load_state(handle.id())
        {
            sound_fail(path, &error.to_string());
            if let Some(ids) = admission.stream_files.get(path) {
                for id in ids {
                    sound_stream_fail(*id, &error.to_string());
                }
            }
            warn!("sound: {path} delivery failed: {error}");
            finished.push((path.clone(), true));
        }
    }
    for (path, failed) in finished {
        admission.files.remove(&path);
        admission.stream_files.remove(&path);
        admission.sfx_files.remove(&path);
        if failed {
            admission.failed.insert(path);
        }
    }
}

impl Admission {
    fn loaded_catalog(&self) -> Option<&SoundKitCatalog> {
        self.catalog.as_ref()
    }
    fn request(
        &mut self,
        request: SoundRequest,
        out: &mut SoundOutput,
        config: &SoundConfig,
        server: &HostAssets,
    ) {
        let result = match request {
            SoundRequest::KitId(id) => kit::play_kit(
                self,
                server,
                out,
                config,
                Vec3::ZERO,
                kit::KitRef::Id(id),
                None,
                kit::SoundCategory::Sfx,
            ),
            SoundRequest::KitName(name) => kit::play_kit(
                self,
                server,
                out,
                config,
                Vec3::ZERO,
                kit::KitRef::Name(&name),
                None,
                kit::SoundCategory::Sfx,
            ),
            SoundRequest::File(path) => {
                kit::play_file(self, server, out, config, &path, kit::SoundCategory::Sfx)
            }
        };
        if matches!(&result, Ok(true)) {
            if let Some(channel) = out.channels.get_mut(&self.next_id) {
                channel.ui = true;
            }
        }
        if let Err(error) = result {
            warn!("sound: UI request failed: {error:#}");
        }
    }

    fn roll(&mut self) -> u32 {
        self.picker.roll()
    }
    fn id_by_name(&self, name: &str) -> Option<u32> {
        self.catalog.as_ref()?.by_name(name).map(|k| k.id)
    }
    fn variations(&self, id: u32) -> usize {
        self.catalog
            .as_ref()
            .and_then(|c| c.get(id))
            .map_or(0, |k| k.files.len())
    }
    fn pick_stream(&mut self, id: u32) -> Option<(String, f32)> {
        selection::pick_stream(self.catalog.as_ref()?, &mut self.picker, id)
    }
}

fn play_ui_audio(
    script: Option<NonSendMut<UiScript>>,
    mut admission: ResMut<Admission>,
    mut out: NonSendMut<SoundOutput>,
    server: Res<HostAssets>,
    config: Res<SoundConfig>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut logout: MessageReader<crate::net::LoggedOutMessage>,
    mut disconnected: MessageReader<crate::net::DisconnectedMessage>,
    mut last_controls: Local<Option<(f32, f32, bool, bool, bool)>>,
) {
    let ended = logout.read().count() + disconnected.read().count() != 0;
    if ended {
        admission.epoch = sound_reset();
        admission.live.clear();
        out.channels.clear();
    }
    let controls = (
        if config.muted { 0.0 } else { config.master },
        config.sfx,
        config.enabled,
        config.background_sound || windows.single().map_or(true, |w| w.focused),
        config.limiter,
    );
    if *last_controls != Some(controls) {
        sound_controls(controls.0, controls.1, controls.2, controls.3, controls.4);
        *last_controls = Some(controls);
    }
    if let Some(mut script) = script {
        let session = script.session();
        if admission.session != Some(session) {
            admission.session = Some(session);
            // /reload replaces Lua, not the world/caster session. Cancel old UI
            // intents only; active or Pending spell loops must survive it.
            world::stop_ui(&mut out);
        }
        let requests = script.take_sounds();
        for request in requests {
            if !config.world_hold {
                admission.request(request, &mut out, &config, &server);
            }
        }
    }
}

/// Production glue/UI sound request.
pub(crate) use glue::GlueSound;

pub(crate) use emote::EmoteSounds;
pub(crate) use greeting::NpcGreetingRequest;
pub(crate) use message::MessageSounds;
pub(crate) use ui::{AutoEquipSound, LootPickupSound};

/// Public sound resources referenced by production systems outside the sound subsystem.
pub(crate) type SoundKits = Admission;

pub(crate) use zone::ExplorationSounds;

pub(crate) mod output {
    pub(crate) fn device_open() -> bool {
        super::sound_status() == "running"
    }
}

/// Keep the production configuration shape so CVar/UI code can compile before audio is ported.
#[derive(Resource)]
pub(crate) struct SoundConfig {
    pub enabled: bool,
    pub muted: bool,
    pub master: f32,
    pub sfx: f32,
    pub music: f32,
    pub ambience: f32,
    pub music_enabled: bool,
    pub ambience_enabled: bool,
    pub error_speech: bool,
    pub background_sound: bool,
    pub reverb: bool,
    pub world_hold: bool,
    pub music_suppressed: bool,
    pub limiter: bool,
    pub listener_at_character: bool,
    pub emote_sounds: bool,
    pub zone_music_no_delay: bool,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            muted: false,
            master: 1.0,
            sfx: 1.0,
            music: 0.4,
            ambience: 0.6,
            music_enabled: true,
            ambience_enabled: true,
            error_speech: true,
            background_sound: false,
            reverb: false,
            world_hold: false,
            music_suppressed: false,
            limiter: true,
            listener_at_character: true,
            emote_sounds: true,
            zone_music_no_delay: false,
        }
    }
}
impl SoundConfig {
    fn category_amp(&self, cat: kit::SoundCategory) -> f32 {
        cvars::category_amp(self, cat)
    }
}
fn world_audio_live(
    state: Res<State<crate::char_select::ClientState>>,
    player: Res<crate::player::Player>,
) -> bool {
    *state.get() == crate::char_select::ClientState::InWorld && player.active
}

pub(crate) fn kit_decodes() -> u32 {
    sound_decodes()
}

pub(crate) struct BrowserSoundPlugin;
pub(crate) type SoundPlugin = BrowserSoundPlugin;

impl Plugin for BrowserSoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SoundConfig>()
            .init_resource::<Admission>()
            .init_resource::<HostAssets>()
            .init_resource::<AudioListener>()
            .insert_non_send_resource(SoundOutput::default())
            .add_observer(cvars::on_cvar)
            .add_systems(PreUpdate, cvars::feed_world_hold)
            .add_systems(
                Update,
                (
                    toggle_mute,
                    cvars::feed_music_suppression
                        .in_set(benilla_world::schedule::WorldStage::Stream),
                ),
            )
            .init_resource::<MessageSounds>()
            .add_message::<GlueSound>()
            .add_message::<NpcGreetingRequest>()
            .add_message::<LootPickupSound>()
            .add_message::<AutoEquipSound>()
            .add_systems(
                Update,
                (
                    deliver_audio,
                    play_ui_audio.after(crate::ui_script::UiInput),
                )
                    .chain(),
            );
        world::plugin(app);
        spell::plugin(app);
        footsteps::plugin(app);
        combat::plugin(app);
        glue::plugin(app);
        zone::plugin(app);
        interior::plugin(app);
        weather::plugin(app);
        cinematic::plugin(app);
        creature::plugin(app);
        greeting::plugin(app);
        emote::plugin(app);
        ui::plugin(app);
        message::plugin(app);
        vocal::plugin(app);
        emitter_pool::plugin(app);
        gameobject::plugin(app);
        death_thud::plugin(app);
        liquid_loop::plugin(app);
        missile::plugin(app);
        money::plugin(app);
        mount::plugin(app);
        water::plugin(app);
        sheathe::plugin(app);
        reverb::plugin(app);
        app.add_systems(
            Update,
            anim_events::route_anim_events.in_set(benilla_world::schedule::WorldStage::Present),
        );
        app.add_systems(
            Update,
            kit::pump_channels.in_set(benilla_world::schedule::WorldStage::Present),
        );
    }
}

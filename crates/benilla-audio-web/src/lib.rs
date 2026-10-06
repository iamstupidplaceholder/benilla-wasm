//! Small browser audio module. Reuses the native Kira fork and original limiter;
//! no Bevy, filesystem, native device or std::thread in the browser render path.
use kira::backend::{Backend, Renderer};
use kira::effect::volume_control::{VolumeControlBuilder, VolumeControlHandle};
use kira::sound::{
    static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings},
    PlaybackState,
};
use kira::sound::{
    streaming::{
        ExternalDecode, ExternalStreamData, ExternalStreamHandle, StreamingSoundData,
        StreamingSoundSettings,
    },
    FromFileError,
};
use kira::{
    effect::reverb::{ReverbBuilder, ReverbHandle},
    track::{SendTrackBuilder, SendTrackHandle},
    Mix,
};
use kira::{
    listener::ListenerHandle,
    track::{SpatialTrackBuilder, SpatialTrackHandle},
};
use kira::{AudioManager, AudioManagerSettings, Decibels, Frame};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct StreamDecoder {
    decoder: ExternalDecode<FromFileError>,
    rate: u32,
}
#[wasm_bindgen]
pub struct StreamBlock {
    indices: Vec<u32>,
    pcm: Vec<f32>,
    ended: bool,
}
#[wasm_bindgen]
impl StreamBlock {
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    pub fn pcm(&self) -> Vec<f32> {
        self.pcm.clone()
    }
    pub fn ended(&self) -> bool {
        self.ended
    }
}
#[wasm_bindgen]
impl StreamDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<Self, JsValue> {
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(error("encoded stream exceeds 32 MiB budget"));
        }
        let data =
            StreamingSoundData::from_cursor(std::io::Cursor::new(bytes.to_vec())).map_err(error)?;
        let rate = data.sample_rate();
        Ok(Self {
            decoder: data.external_decode().map_err(error)?,
            rate,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn read(&mut self) -> Result<StreamBlock, JsValue> {
        let frames = self.decoder.read(4096).map_err(error)?;
        Ok(StreamBlock {
            indices: frames.iter().map(|(i, _)| *i as u32).collect(),
            pcm: frames.iter().flat_map(|(_, f)| [f.left, f.right]).collect(),
            ended: self.decoder.finished(),
        })
    }
}

#[path = "../../benilla-app/src/sound/limiter.rs"]
mod limiter;
#[path = "../../benilla-app/src/sound/mixer_params.rs"]
mod mixer_params;
use mixer_params::{amp_to_db as db, glide};
#[path = "../../benilla-app/src/sound/meter.rs"]
mod meter;
#[path = "../../benilla-app/src/sound/voice_slot.rs"]
mod voice_slot;

const PCM_BUDGET: usize = 64 * 1024 * 1024;
const FILE_BUDGET: usize = 16 * 1024 * 1024;
const VOICES: usize = 12;

pub struct BrowserBackend {
    renderer: Option<Renderer>,
}
impl Backend for BrowserBackend {
    type Settings = u32;
    type Error = std::convert::Infallible;
    fn setup(rate: u32, _: usize) -> Result<(Self, u32), Self::Error> {
        Ok((Self { renderer: None }, rate))
    }
    fn start(&mut self, renderer: Renderer) -> Result<(), Self::Error> {
        self.renderer = Some(renderer);
        Ok(())
    }
}

fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub struct DecodedSound {
    data: StaticSoundData,
}
#[wasm_bindgen]
impl DecodedSound {
    pub fn sample_rate(&self) -> u32 {
        self.data.sample_rate
    }
    pub fn pcm(&self) -> Vec<f32> {
        self.data
            .frames
            .iter()
            .flat_map(|f| [f.left, f.right])
            .collect()
    }
}

#[wasm_bindgen]
pub fn decode_sound(bytes: &[u8]) -> Result<DecodedSound, JsValue> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(error("compressed SFX exceeds 8 MiB budget"));
    }
    let data =
        StaticSoundData::from_cursor_bounded(std::io::Cursor::new(bytes.to_vec()), FILE_BUDGET / 8)
            .map_err(error)?;
    if data.frames.is_empty() {
        return Err(error("decoded sound has no frames"));
    }
    Ok(DecodedSound { data })
}

struct Voice {
    stealable: bool,
    use_sfx: bool,
    path: String,
    handle: StaticSoundHandle,
    volume: f32,
    track: Option<SpatialTrackHandle>,
}

#[wasm_bindgen]
pub struct AudioEngine {
    manager: AudioManager<BrowserBackend>,
    cache: HashMap<String, StaticSoundData>,
    order: VecDeque<String>,
    resident: usize,
    voices: HashMap<u32, Voice>,
    master: VolumeControlHandle,
    gate: VolumeControlHandle,
    limiter_on: Arc<AtomicBool>,
    sfx: f32,
    listener: ListenerHandle,
    streams: HashMap<u32, ExternalStreamHandle>,
    reverb: ReverbHandle,
    reverb_send: SendTrackHandle,
    stolen: Vec<u32>,
}

#[wasm_bindgen]
impl AudioEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32) -> Result<AudioEngine, JsValue> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err(error("invalid browser sample rate"));
        }
        let limiter_on = Arc::new(AtomicBool::new(true));
        let level = Arc::new(meter::MixLevel::default());
        let mut main = kira::track::MainTrackBuilder::new();
        let master = main.add_effect(VolumeControlBuilder::new(Decibels::IDENTITY));
        limiter::install(&mut main, &level, &limiter_on);
        let gate = main.add_effect(VolumeControlBuilder::new(Decibels::IDENTITY));
        let mut manager = AudioManager::new(AudioManagerSettings::<BrowserBackend> {
            backend_settings: sample_rate,
            main_track_builder: main,
            internal_buffer_size: 128,
            ..Default::default()
        })
        .map_err(error)?;
        let listener = manager
            .add_listener(
                mint::Vector3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                mint::Quaternion {
                    v: mint::Vector3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    s: 1.0,
                },
            )
            .map_err(error)?;
        let mut send = SendTrackBuilder::new().volume(Decibels::SILENCE);
        let reverb = send.add_effect(
            ReverbBuilder::new()
                .mix(Mix::WET)
                .feedback(0.5)
                .damping(0.5),
        );
        let reverb_send = manager.add_send_track(send).map_err(error)?;
        Ok(Self {
            manager,
            cache: HashMap::new(),
            order: VecDeque::new(),
            resident: 0,
            voices: HashMap::new(),
            master,
            gate,
            limiter_on,
            sfx: 1.0,
            listener,
            streams: HashMap::new(),
            reverb,
            reverb_send,
            stolen: Vec::new(),
        })
    }

    pub fn upload(&mut self, path: String, rate: u32, pcm: &[f32]) -> Result<(), JsValue> {
        if self.cache.contains_key(&path) {
            return Ok(());
        }
        if pcm.len() % 2 != 0
            || pcm.len() == 0
            || pcm.len() * 4 > FILE_BUDGET
            || !(8000..=192000).contains(&rate)
            || pcm.iter().any(|s| !s.is_finite())
        {
            return Err(error("invalid or over-budget decoded PCM"));
        }
        let bytes = pcm.len() * 4;
        let mut scanned = self.order.len();
        while self.resident + bytes > PCM_BUDGET && scanned > 0 {
            scanned -= 1;
            let key = self.order.pop_front().expect("cache order");
            if self.voices.values().any(|v| v.path == key) {
                self.order.push_back(key);
                continue;
            }
            if let Some(old) = self.cache.remove(&key) {
                self.resident -= old.frames.len() * 8;
            }
        }
        if self.resident + bytes > PCM_BUDGET {
            return Err(error("PCM cache budget occupied by live voices"));
        }
        let frames: Arc<[Frame]> = pcm
            .chunks_exact(2)
            .map(|s| Frame {
                left: s[0],
                right: s[1],
            })
            .collect();
        self.cache.insert(
            path.clone(),
            StaticSoundData {
                sample_rate: rate,
                frames,
                settings: StaticSoundSettings::default(),
                slice: None,
            },
        );
        self.order.push_back(path);
        self.resident += bytes;
        Ok(())
    }

    pub fn has(&self, path: &str) -> bool {
        self.cache.contains_key(path)
    }

    pub fn play(
        &mut self,
        id: u32,
        path: &str,
        volume: f32,
        frequency: i32,
        looping: bool,
        spatial: bool,
        use_sfx: bool,
        wet: bool,
        protected: bool,
        x: f32,
        y: f32,
        z: f32,
    ) -> Result<bool, JsValue> {
        let stealable = self
            .voices
            .iter()
            .filter(|(_, v)| v.stealable)
            .map(|(id, v)| {
                (
                    *id as usize,
                    v.volume * if v.use_sfx { self.sfx } else { 1.0 },
                )
            });
        match voice_slot::pick_voice_slot(
            stealable,
            self.voices.len() + self.streams.len(),
            volume * if use_sfx { self.sfx } else { 1.0 },
        ) {
            voice_slot::VoiceSlot::Free => {}
            voice_slot::VoiceSlot::Denied => {
                return Ok(false);
            }
            voice_slot::VoiceSlot::Steal(id) => {
                if let Some(mut old) = self.voices.remove(&(id as u32)) {
                    old.handle.stop(glide());
                    self.stolen.push(id as u32);
                }
            }
        }
        let original = self
            .cache
            .get(path)
            .ok_or_else(|| error("sound not resident in audio worker"))?;
        let mut data = original.volume(db(volume * if use_sfx { self.sfx } else { 1.0 }));
        if frequency > 0 {
            data = data.playback_rate(frequency as f64 / data.sample_rate as f64);
        }
        if looping {
            data = data.loop_region(..);
        }
        let (track, handle) = if spatial {
            let mut track = self
                .manager
                .add_spatial_sub_track(
                    self.listener.id(),
                    mint::Vector3 { x, y, z },
                    SpatialTrackBuilder::new()
                        .attenuation_function(None)
                        .sound_capacity(1)
                        .persist_until_sounds_finish(true)
                        .with_send(
                            &self.reverb_send,
                            if wet {
                                Decibels::IDENTITY
                            } else {
                                Decibels::SILENCE
                            },
                        ),
                )
                .map_err(error)?;
            let handle = track.play(data).map_err(error)?;
            (Some(track), handle)
        } else {
            (None, self.manager.play(data).map_err(error)?)
        };
        self.voices.insert(
            id,
            Voice {
                path: path.to_owned(),
                use_sfx,
                stealable: !looping && !protected,
                handle,
                volume,
                track,
            },
        );
        Ok(true)
    }

    pub fn listener(&mut self, x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32) {
        self.listener
            .set_position(mint::Vector3 { x, y, z }, glide());
        self.listener.set_orientation(
            mint::Quaternion {
                v: mint::Vector3 {
                    x: qx,
                    y: qy,
                    z: qz,
                },
                s: qw,
            },
            glide(),
        );
    }
    pub fn update(&mut self, id: u32, amp: f32, x: f32, y: f32, z: f32) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.volume = amp;
            voice.handle.set_volume(
                db(amp * if voice.use_sfx { self.sfx } else { 1.0 }),
                glide(),
            );
            if let Some(track) = &mut voice.track {
                track.set_position(mint::Vector3 { x, y, z }, glide());
            }
        }
    }
    pub fn stop(&mut self, id: u32) {
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.handle.stop(glide());
        }
        if let Some(stream) = self.streams.get_mut(&id) {
            stream.playback.stop(glide());
        }
    }

    pub fn stream_open(&mut self, id: u32, rate: u32, volume: f32) -> Result<(), JsValue> {
        if self.streams.len() >= 8 {
            return Err(error("stream capacity reached"));
        }
        let mut settings = StreamingSoundSettings::default();
        settings.volume = db(volume).into();
        let handle = self
            .manager
            .play(ExternalStreamData {
                sample_rate: rate,
                settings,
            })
            .map_err(error)?;
        self.streams.insert(id, handle);
        Ok(())
    }
    pub fn stream_slots(&self, id: u32) -> usize {
        self.streams.get(&id).map_or(0, |h| h.slots())
    }
    pub fn stream_feed(
        &mut self,
        id: u32,
        indices: &[u32],
        pcm: &[f32],
        ended: bool,
    ) -> Result<(), JsValue> {
        self.streams
            .get_mut(&id)
            .ok_or_else(|| error("stream no longer live"))?
            .feed(indices, pcm, ended)
            .map_err(error)
    }
    pub fn stream_command(&mut self, id: u32, command: &str, volume: f32, ms: u32) {
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        let tween = kira::Tween {
            duration: std::time::Duration::from_millis(ms as u64),
            ..Default::default()
        };
        match command {
            "volume" => stream.playback.set_volume(db(volume), tween),
            "stop" => stream.playback.stop(tween),
            "pause" => stream.playback.pause(tween),
            "resume" => stream.playback.resume(tween),
            _ => {}
        }
    }
    pub fn voice_command(&mut self, id: u32, command: &str, volume: f32, ms: u32) {
        let Some(voice) = self.voices.get_mut(&id) else {
            return;
        };
        let tween = kira::Tween {
            duration: std::time::Duration::from_millis(ms as u64),
            ..Default::default()
        };
        match command {
            "volume" => {
                voice.volume = volume;
                voice.handle.set_volume(
                    db(volume * if voice.use_sfx { self.sfx } else { 1.0 }),
                    tween,
                );
            }
            "stop" => voice.handle.stop(tween),
            "pause" => voice.handle.pause(tween),
            "resume" => voice.handle.resume(tween),
            _ => {}
        }
    }
    pub fn stream_position(&self, id: u32) -> f64 {
        self.streams.get(&id).map_or(0.0, |h| h.playback.position())
    }
    pub fn reverb(&mut self, feedback: f64, damping: f64, amp: f32) {
        let snap = kira::Tween {
            duration: std::time::Duration::ZERO,
            ..Default::default()
        };
        self.reverb.set_feedback(feedback, snap);
        self.reverb.set_damping(damping, snap);
        self.reverb_send.set_volume(db(amp), snap);
    }
    pub fn stream_state(&self, id: u32) -> String {
        self.streams
            .get(&id)
            .map_or("Stopped".into(), |h| format!("{:?}", h.playback.state()))
    }

    pub fn controls(&mut self, master: f32, sfx: f32, enabled: bool, gate: bool, limiter: bool) {
        self.master.set_volume(db(master), glide());
        self.gate.set_volume(
            if gate {
                Decibels::IDENTITY
            } else {
                Decibels::SILENCE
            },
            glide(),
        );
        self.limiter_on.store(limiter, Ordering::Relaxed);
        self.sfx = if enabled { sfx } else { 0.0 };
        for voice in self.voices.values_mut() {
            voice.handle.set_volume(
                db(voice.volume * if voice.use_sfx { self.sfx } else { 1.0 }),
                glide(),
            );
        }
    }

    pub fn stop_all(&mut self) {
        for voice in self.voices.values_mut() {
            voice.handle.stop(glide());
        }
        // Keep handles/resident ownership until the real fade completes.
    }

    pub fn render(&mut self, out: &mut [f32]) {
        let renderer = self
            .manager
            .backend_mut()
            .renderer
            .as_mut()
            .expect("backend started");
        renderer.on_start_processing();
        renderer.process(out, 2);
    }

    pub fn take_ended(&mut self) -> Vec<u32> {
        let mut ended: Vec<_> = self
            .voices
            .iter()
            .filter(|(_, v)| v.handle.state() == PlaybackState::Stopped)
            .map(|(id, _)| *id)
            .collect();
        ended.extend(
            self.streams
                .iter()
                .filter(|(_, s)| s.playback.state() == PlaybackState::Stopped)
                .map(|(id, _)| *id),
        );
        for id in &ended {
            self.voices.remove(id);
            self.streams.remove(id);
        }
        ended
    }
    pub fn take_stolen(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.stolen)
    }
}

//! Host handle/source adapters for the original music/ambience schedulers.
//! A source is a validated asynchronous path, never fabricated audio bytes.
use anyhow::{anyhow, Result};
use bevy::prelude::*;
use std::{marker::PhantomData, sync::Mutex};
#[path = "../sound/mixer_params.rs"]
mod params;
pub(super) use kira::sound::FromFileError;
pub(super) use kira::{Decibels, Tween};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PlaybackState {
    Pending,
    Playing,
    Pausing,
    Paused,
    WaitingToResume,
    Resuming,
    Stopping,
    Stopped,
}
#[derive(Resource, Default)]
pub(super) struct SoundAssets {
    pub(super) chain: Mutex<PathSource>,
}
#[derive(Default)]
pub(super) struct PathSource;
#[derive(Clone)]
pub(super) struct Source {
    path: String,
    volume: f32,
    looping: bool,
}
impl PathSource {
    pub(super) fn read(&self, path: &str) -> Result<Source> {
        Ok(Source {
            path: benilla_formats::normalize_client_path(path)?,
            volume: 1.0,
            looping: false,
        })
    }
}
impl SoundAssets {
    pub(super) fn read_file_or_loose(&self, path: &str) -> Option<Source> {
        self.chain.lock().ok()?.read(path).ok()
    }
}
impl Source {
    pub(super) fn volume(mut self, db: Decibels) -> Self {
        self.volume = db.as_amplitude();
        self
    }
}
pub(super) fn stream_from_bytes(source: Source) -> Result<Source> {
    Ok(source)
}
pub(super) fn loop_from_bytes(mut source: Source) -> Result<Source> {
    source.looping = true;
    Ok(source)
}
pub(super) fn amp_to_db(amp: f32) -> Decibels {
    params::amp_to_db(amp)
}
pub(super) fn fade(ms: u64) -> Tween {
    Tween {
        duration: std::time::Duration::from_millis(ms),
        ..Default::default()
    }
}
pub(super) fn glide() -> Tween {
    fade(15)
}
pub(super) fn declick() -> Tween {
    glide()
}
pub(super) fn snap() -> Tween {
    fade(0)
}
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
import {audio} from '../../../audio/host.js';
export function musicOpen(path,amp){return audio.openStream(path,amp);}
export function musicCommand(id,command,amp,ms){audio.streamCommand(id,command,amp,ms);}
export function musicState(id){return audio.streamState(id);}
export function musicForget(id){audio.forgetStream(id);}
export function staticOpen(path,amp,looping){return audio.openStatic(path,amp,looping);}
export function staticCommand(id,command,amp,ms){audio.staticCommand(id,command,amp,ms);}
export function musicReverb(feedback,damping,amp){audio.reverb({feedback,damping,amp});}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=musicOpen)]
    fn open(path: &str, amp: f32) -> u32;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=musicCommand)]
    fn command(id: u32, command: &str, amp: f32, ms: u32);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=musicState)]
    fn state(id: u32) -> String;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=musicForget)]
    fn forget(id: u32);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=staticOpen)]
    fn static_open(path: &str, amp: f32, looping: bool) -> u32;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=staticCommand)]
    fn static_command(id: u32, command: &str, amp: f32, ms: u32);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=musicReverb)]
    fn reverb(feedback: f64, damping: f64, amp: f32);
}
#[derive(serde::Deserialize)]
struct Snapshot {
    #[serde(default, rename = "mixSeconds")]
    mix_seconds: f64,
    state: String,
    position: f64,
    error: Option<String>,
}
pub(super) struct StreamingSoundHandle<E> {
    id: u32,
    is_static: bool,
    _error: PhantomData<E>,
}
pub(super) type StaticSoundHandle = StreamingSoundHandle<FromFileError>;
impl<E> StreamingSoundHandle<E> {
    fn snapshot(&self) -> Snapshot {
        serde_json::from_str(&state(self.id)).unwrap_or(Snapshot {
            state: "Stopped".into(),
            position: 0.0,
            mix_seconds: 0.0,
            error: Some("invalid stream backend state".into()),
        })
    }
    pub(super) fn state(&self) -> PlaybackState {
        match self.snapshot().state.as_str() {
            "Pending" => PlaybackState::Pending,
            "Playing" => PlaybackState::Playing,
            "Paused" => PlaybackState::Paused,
            "Pausing" => PlaybackState::Pausing,
            "WaitingToResume" => PlaybackState::WaitingToResume,
            "Resuming" => PlaybackState::Resuming,
            "Stopping" => PlaybackState::Stopping,
            _ => PlaybackState::Stopped,
        }
    }
    pub(super) fn position(&self) -> f64 {
        self.snapshot().position
    }
    fn command(&self, kind: &str, amp: f32, t: Tween) {
        if self.is_static {
            static_command(self.id, kind, amp, t.duration.as_millis() as u32)
        } else {
            command(self.id, kind, amp, t.duration.as_millis() as u32)
        }
    }
    pub(super) fn stop(&mut self, t: Tween) {
        self.command("stop", 0.0, t)
    }
    pub(super) fn pause(&mut self, t: Tween) {
        self.command("pause", 0.0, t)
    }
    pub(super) fn resume(&mut self, t: Tween) {
        self.command("resume", 0.0, t)
    }
    pub(super) fn set_volume(&mut self, db: Decibels, t: Tween) {
        self.command("volume", db.as_amplitude(), t)
    }
}
impl<E> Drop for StreamingSoundHandle<E> {
    fn drop(&mut self) {
        forget(self.id);
    }
}
pub(super) struct Mixer;
impl Mixer {
    pub(super) fn ready(&self) -> bool {
        super::sound_status() == "running"
    }
    pub(super) fn set_reverb(&mut self, preset: Option<&benilla_formats::SoundProvider>) {
        let (feedback, damping, db) = preset.map_or((0.0, 0.0, Decibels::SILENCE), |p| {
            params::freeverb_projection(p.decay_time, p.decay_hf_ratio, p.room_hf, p.room, p.reverb)
        });
        reverb(feedback, damping, db.as_amplitude());
    }
    pub(super) fn play_stream(
        &mut self,
        source: Source,
    ) -> Result<StreamingSoundHandle<FromFileError>> {
        let id = open(&source.path, source.volume);
        if id == 0 {
            return Err(anyhow!(
                "browser audio not unlocked/ready or stream queue full"
            ));
        }
        Ok(StreamingSoundHandle {
            id,
            is_static: false,
            _error: PhantomData,
        })
    }
    pub(super) fn play_2d(&mut self, source: Source) -> Result<StaticSoundHandle> {
        let id = static_open(&source.path, source.volume, source.looping);
        if id == 0 {
            return Err(anyhow!("browser audio not ready or channel queue full"));
        }
        Ok(StreamingSoundHandle {
            id,
            is_static: true,
            _error: PhantomData,
        })
    }
}
pub(super) struct StreamWatch {
    label: &'static str,
    last: f64,
    stall: f64,
    last_mix: f64,
    warned: bool,
}
impl StreamWatch {
    pub(super) fn new(label: &'static str) -> Self {
        Self {
            label,
            last: 0.0,
            stall: 0.0,
            last_mix: 0.0,
            warned: false,
        }
    }
    pub(super) fn reset(&mut self) {
        self.last = 0.0;
        self.stall = 0.0;
        self.last_mix = 0.0;
        self.warned = false;
    }
    pub(super) fn feed<E>(&mut self, handle: &StreamingSoundHandle<E>, _dt: f64) {
        if super::sound_status() != "running" {
            self.reset();
            return;
        }
        let snapshot = handle.snapshot();
        // Main-thread frame stalls do not prove the independent mixer stalled.
        // Only charge fresh Worker mix-clock progress against stream progress.
        let dt = if self.last_mix == 0.0 {
            0.0
        } else {
            (snapshot.mix_seconds - self.last_mix).max(0.0)
        };
        self.last_mix = snapshot.mix_seconds;
        if let Some(error) = snapshot.error {
            if !self.warned {
                warn!("{}: {error}", self.label);
                self.warned = true;
            }
            return;
        }
        if handle.state() != PlaybackState::Playing {
            self.stall = 0.0;
            return;
        }
        if snapshot.position > self.last {
            self.stall = 0.0;
            self.last = snapshot.position;
            self.warned = false;
        } else {
            self.stall += dt;
        }
        if self.stall > 1.0 && !self.warned {
            warn!(
                "{}: stream position stalled for {:.2}s",
                self.label, self.stall
            );
            self.warned = true;
        }
    }
}

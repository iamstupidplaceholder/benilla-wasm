//! Original sound CVar observer, shared by both output adapters.
use super::SoundConfig;
pub(super) fn toggle_mute(keys: Res<ButtonInput<KeyCode>>, mut config: ResMut<SoundConfig>) {
    if crate::run_mode::dev_chord(&keys, KeyCode::KeyM) {
        config.muted = !config.muted;
        info!("sound {}", if config.muted { "muted" } else { "unmuted" });
    }
}
pub(super) fn feed_music_suppression(
    mut config: ResMut<SoundConfig>,
    cinematic: Option<Res<crate::cinematic::Cinematic>>,
) {
    let playing = cinematic
        .as_deref()
        .is_some_and(crate::cinematic::Cinematic::is_playing);
    if config.music_suppressed != playing {
        config.music_suppressed = playing;
    }
}
pub(super) fn category_amp(config: &SoundConfig, cat: super::kit::SoundCategory) -> f32 {
    if !config.enabled {
        return 0.0;
    }
    match cat {
        super::kit::SoundCategory::Sfx => config.sfx,
        super::kit::SoundCategory::Music if config.music_enabled => config.music,
        super::kit::SoundCategory::Ambience if config.ambience_enabled => config.ambience,
        _ => 0.0,
    }
}
use bevy::prelude::*;

pub(super) fn feed_world_hold(
    mut config: ResMut<SoundConfig>,
    screen: Res<crate::loading_screen::LoadingScreen>,
) {
    let hold = screen.covering();
    if config.world_hold != hold {
        config.world_hold = hold;
    }
}
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut sound: ResMut<SoundConfig>) {
    let v = ev.num();
    match ev.key().as_str() {
        "mastervolume" => sound.master = v.clamp(0.0, 1.0),
        "soundvolume" => sound.sfx = v.clamp(0.0, 1.0),
        "musicvolume" => sound.music = v.clamp(0.0, 1.0),
        "ambiencevolume" => sound.ambience = v.clamp(0.0, 1.0),
        "mastersoundeffects" => sound.enabled = v != 0.0,
        "enablemusic" => sound.music_enabled = v != 0.0,
        "enableambience" => sound.ambience_enabled = v != 0.0,
        "enableerrorspeech" => sound.error_speech = v != 0.0,
        "sound_enablesoundwhengameisinbg" => sound.background_sound = v != 0.0,
        "soundreverb" => sound.reverb = v != 0.0,
        "soundoutputlimiter" => sound.limiter = v != 0.0,
        "soundlisteneratcharacter" => sound.listener_at_character = v != 0.0,
        "emotesounds" => sound.emote_sounds = v != 0.0,
        "soundzonemusicnodelay" => sound.zone_music_no_delay = v != 0.0,
        _ => {}
    }
}

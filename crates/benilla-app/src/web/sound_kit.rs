//! Async host admission seam for the original sound trigger modules.
//! No gameplay routing or decoder here: original modules provide the intent,
//! common selection/math gates it, Kira in the Worker owns actual playback.
use super::world::{send_channel, stop_channel, Channel};
use super::{math, selection, sound_play, sound_status, Admission, SoundConfig, SoundOutput};
use anyhow::{anyhow, Result};
use bevy::prelude::*;
pub(super) type SoundKits = Admission;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SoundCategory {
    Sfx,
    Music,
    Ambience,
}
pub(super) enum KitRef<'a> {
    Id(u32),
    Name(&'a str),
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Bus(pub(super) u8);
impl Bus {
    pub(super) const ERROR_SPEECH: Self = Self(1);
    pub(super) const DEFAULT: Self = Self(0);
    pub(super) const EXERTION: Self = Self(5);
    pub(super) const WEAPON_SWING: Self = Self(6);
    pub(super) const INJURY: Self = Self(7);
    pub(super) const SELF_INJURY: Self = Self(8);
    pub(super) const FOOTSTEP: Self = Self(9);
    pub(super) const MELEE_IMPACT: Self = Self(10);
}
#[derive(Clone, Copy)]
pub(super) struct Volume(pub(super) f32);
impl Default for Volume {
    fn default() -> Self {
        Self(1.0)
    }
}
#[derive(Clone, Copy, Default)]
pub(super) struct PlayExtras {
    pub(super) latch: Latch,
    pub(super) source: Option<Entity>,
    pub(super) force_loop: bool,
    pub(super) variant: Option<usize>,
    pub(super) dedupe_exempt: bool,
    pub(super) bus: Bus,
    pub(super) volume_mult: Volume,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Latch {
    #[default]
    None,
    Greeting,
    Voice(u8),
    ObjectSound,
}
pub(super) use selection::bark_chance_pass;
pub(super) use selection::{
    EXERTION_CHANCE_CREATURE, EXERTION_CHANCE_PLAYER, INJURY_CHANCE_CREATURE, INJURY_CHANCE_PLAYER,
};

pub(super) fn kit_looping(kits: &SoundKits, id: u32) -> bool {
    kits.catalog
        .as_ref()
        .and_then(|c| c.get(id))
        .is_some_and(|k| k.flags & benilla_formats::sound_kit_flags::LOOPING != 0)
}
pub(super) fn object_sound_playing(out: &SoundOutput, source: Entity) -> bool {
    out.channels
        .values()
        .any(|c| c.source == Some(source) && c.latch == Latch::ObjectSound && !c.cancelled)
}

pub(super) fn stop_source_kit(out: &mut SoundOutput, source: Entity, kit: u32) {
    let ids: Vec<_> = out
        .channels
        .iter()
        .filter(|(_, c)| c.source == Some(source) && c.kit == kit)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        stop_channel(out, id);
    }
}

pub(super) fn play_kit(
    kits: &mut SoundKits,
    assets: &super::HostAssets,
    out: &mut SoundOutput,
    config: &SoundConfig,
    listener: Vec3,
    reference: KitRef<'_>,
    pos: Option<Vec3>,
    category: SoundCategory,
) -> Result<bool> {
    play_kit_ext(
        kits,
        assets,
        out,
        config,
        listener,
        reference,
        pos,
        category,
        PlayExtras::default(),
    )
}

pub(super) fn play_kit_ext(
    kits: &mut SoundKits,
    _: &super::HostAssets,
    out: &mut SoundOutput,
    config: &SoundConfig,
    listener: Vec3,
    reference: KitRef<'_>,
    pos: Option<Vec3>,
    category: SoundCategory,
    extras: PlayExtras,
) -> Result<bool> {
    if config.world_hold || sound_status() != "running" {
        return Ok(false);
    }
    let Some(catalog) = kits.catalog.as_ref() else {
        return Ok(false);
    };
    let kit = match reference {
        KitRef::Id(id) => catalog.get(id),
        KitRef::Name(name) => catalog.by_name(name),
    }
    .ok_or_else(|| anyhow!("unknown sound kit"))?;
    if pos.is_some_and(|p| {
        kit.distance_cutoff > 0.0 && !math::audible(math::dist_sq(listener, p), kit.distance_cutoff)
    }) {
        return Ok(false);
    }
    if selection::bus_at_cap(out.channels.values().map(|c| c.bus), extras.bus.0) {
        return Ok(false);
    }
    let live = out.channels.values().filter(|c| c.kit == kit.id).count();
    if selection::no_duplicates_blocks(extras.dedupe_exempt, kit.flags, live)
        || selection::same_kit_cap_blocks(extras.dedupe_exempt, live)
    {
        return Ok(false);
    }
    let weights: Vec<_> = kit.files.iter().map(|(_, w)| *w).collect();
    if weights.is_empty() {
        return Ok(false);
    }
    let pick = match extras.variant {
        Some(i) if i < weights.len() => i,
        Some(_) => return Ok(false),
        None => kits.picker.pick(kit.id, &weights),
    };
    let channel = Channel {
        category,
        category_amp: if category == SoundCategory::Sfx {
            1.0
        } else {
            config.category_amp(category)
        },
        wet: kit.eax_def != 0,
        gain: 1.0,
        latch: extras.latch,
        ui: false,
        kit: kit.id,
        source: extras.source,
        pos,
        min_dist: kit.min_distance,
        cutoff: kit.distance_cutoff,
        volume: kits
            .picker
            .volume(kit.flags, kit.volume, extras.volume_mult.0),
        bus: extras.bus.0,
        playing: false,
        cancelled: false,
    };
    let frequency = kits.picker.frequency(kit.flags).unwrap_or(0);
    let looping = extras.force_loop || kit.flags & benilla_formats::sound_kit_flags::LOOPING != 0;
    let path = kit.files[pick].0.clone();
    enqueue(kits, out, path, channel, frequency, looping)
}

fn enqueue(
    kits: &mut SoundKits,
    out: &mut SoundOutput,
    path: String,
    channel: Channel,
    frequency: i32,
    looping: bool,
) -> Result<bool> {
    let path = benilla_formats::normalize_client_path(&path)?;
    kits.next_id = kits.next_id.wrapping_add(1).max(1);
    let id = kits.next_id;
    let pos = channel.pos.unwrap_or_default();
    if !sound_play(
        id,
        &path,
        channel.volume,
        frequency,
        looping,
        channel.pos.is_some(),
        pos.x,
        pos.y,
        pos.z,
        super::world::initial_amp(out, &channel),
        channel.category != SoundCategory::Sfx,
        channel.wet,
        channel.latch != Latch::None,
    ) {
        return Ok(false);
    }
    // This is a reservation, not confirmation of Playing. Updates are stored
    // worker-side even when bytes are Pending; stop cancels that same request.
    kits.live.insert(id, channel.kit);
    out.channels.insert(id, channel);
    send_channel(out, id);
    Ok(true)
}

pub(super) fn play_file(
    kits: &mut SoundKits,
    _: &super::HostAssets,
    out: &mut SoundOutput,
    config: &SoundConfig,
    path: &str,
    _: SoundCategory,
) -> Result<bool> {
    if config.world_hold || sound_status() != "running" {
        return Ok(false);
    }
    enqueue(
        kits,
        out,
        path.to_owned(),
        Channel {
            category: SoundCategory::Sfx,
            category_amp: 1.0,
            wet: false,
            gain: 1.0,
            latch: Latch::None,
            ui: true,
            kit: 0,
            source: None,
            pos: None,
            min_dist: 0.0,
            cutoff: 0.0,
            volume: 1.0,
            bus: 0,
            playing: false,
            cancelled: false,
        },
        0,
        false,
    )
}
pub(super) const STAND_CHANCE: u32 = 40;
pub(super) const STAND_COOLDOWN: benilla_time::Duration = benilla_time::Duration::from_secs(10);
pub(super) fn kit_name(kits: &SoundKits, id: u32) -> Option<&str> {
    Some(&kits.catalog.as_ref()?.get(id)?.name)
}
pub(super) fn kit_cutoff(kits: &SoundKits, id: u32) -> Option<f32> {
    Some(kits.catalog.as_ref()?.get(id)?.distance_cutoff)
}
pub(super) fn source_playing(out: &SoundOutput, source: Entity) -> bool {
    out.channels
        .values()
        .any(|c| c.source == Some(source) && c.latch == Latch::Greeting && !c.cancelled)
}
pub(super) fn source_kit_playing(out: &SoundOutput, source: Entity, kit: u32) -> bool {
    out.channels
        .values()
        .any(|c| c.source == Some(source) && c.kit == kit && !c.cancelled)
}
pub(super) fn stop_source(out: &mut SoundOutput, source: Entity) -> usize {
    let ids: Vec<_> = out
        .channels
        .iter()
        .filter(|(_, c)| c.source == Some(source))
        .map(|(id, _)| *id)
        .collect();
    let count = ids.len();
    for id in ids {
        stop_channel(out, id);
    }
    count
}
pub(super) fn unit_voice_state(out: &SoundOutput, source: Entity) -> Option<u8> {
    out.channels.values().find_map(|c| {
        if c.source == Some(source) && !c.cancelled {
            if let Latch::Voice(state) = c.latch {
                Some(state)
            } else {
                None
            }
        } else {
            None
        }
    })
}
pub(super) fn stop_unit_voice(out: &mut SoundOutput, source: Entity) {
    let ids: Vec<_> = out
        .channels
        .iter()
        .filter(|(_, c)| c.source == Some(source) && matches!(c.latch, Latch::Voice(_)))
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        stop_channel(out, id);
    }
}
pub(super) fn set_source_kit_gain(out: &mut SoundOutput, source: Entity, kit: u32, gain: f32) {
    for c in out.channels.values_mut() {
        if c.source == Some(source) && c.kit == kit {
            c.gain = gain.clamp(0.0, 1.0);
        }
    }
}
pub(super) fn pump_channels(out: NonSend<SoundOutput>) {
    for (&id, c) in &out.channels {
        if !c.cancelled {
            send_channel(&out, id);
        }
    }
}

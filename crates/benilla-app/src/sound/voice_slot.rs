//! Original native voice-cap decision, independent of host handles.
pub(crate) const SOFTWARE_CHANNELS: usize = 12;
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VoiceSlot {
    Free,
    Steal(usize),
    Denied,
}
pub(crate) fn pick_voice_slot(
    stealable: impl Iterator<Item = (usize, f32)>,
    live: usize,
    amp: f32,
) -> VoiceSlot {
    if live < SOFTWARE_CHANNELS {
        return VoiceSlot::Free;
    }
    match stealable.min_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((i, old)) if old < amp => VoiceSlot::Steal(i),
        _ => VoiceSlot::Denied,
    }
}

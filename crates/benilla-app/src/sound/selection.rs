//! Original kit variation state, shared by native playback and async host admission.
use super::math;

pub(super) const BUS_CAP: [u32; 13] = [0x7fff_ffff, 1, 2, 2, 1, 1, 2, 2, 1, 6, 4, 1, 2];
pub(super) const EXERTION_CHANCE_CREATURE: u32 = 70;
pub(super) const EXERTION_CHANCE_PLAYER: u32 = 35;
pub(super) const INJURY_CHANCE_CREATURE: u32 = 60;
pub(super) const INJURY_CHANCE_PLAYER: u32 = 30;
pub(super) fn bus_at_cap(live: impl Iterator<Item = u8>, bus: u8) -> bool {
    let cap = BUS_CAP[usize::from(bus)];
    cap != BUS_CAP[0] && live.filter(|b| *b == bus).count() as u32 >= cap
}
pub(super) fn bark_chance_pass(threshold: u32, roll: u32) -> bool {
    ((101u64 * u64::from(roll)) >> 32) as u32 <= threshold
}
pub(super) fn pick_stream(
    catalog: &benilla_formats::SoundKitCatalog,
    picker: &mut VariationPicker,
    id: u32,
) -> Option<(String, f32)> {
    let kit = catalog.get(id)?;
    if kit.files.is_empty() {
        return None;
    }
    let weights: Vec<_> = kit.files.iter().map(|(_, w)| *w).collect();
    let pick = picker.pick(id, &weights);
    Some((kit.files[pick].0.clone(), kit.volume))
}
use benilla_formats::sound_kit_flags;
use std::collections::HashMap;

pub(super) fn no_duplicates_blocks(exempt: bool, flags: u32, live: usize) -> bool {
    !exempt && flags & sound_kit_flags::NO_DUPLICATES != 0 && live > 0
}
pub(super) fn same_kit_cap_blocks(exempt: bool, live: usize) -> bool {
    !exempt && live >= 2
}

struct PickState {
    remaining: Vec<u32>,
}

pub(super) struct VariationPicker {
    pick: HashMap<u32, PickState>,
    rng: u32,
}

impl Default for VariationPicker {
    fn default() -> Self {
        Self {
            pick: HashMap::new(),
            rng: 0x9e37_79b9,
        }
    }
}

impl VariationPicker {
    pub(super) fn roll(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// 0x45bb70: weighted selection with depletion; this is the original kit player.
    pub(super) fn pick(&mut self, kit: u32, weights: &[u32]) -> usize {
        if weights.len() == 1 {
            return 0;
        }
        let st = self.pick.entry(kit).or_insert_with(|| PickState {
            remaining: weights.to_vec(),
        });
        let mut total: u32 = st.remaining.iter().sum();
        if total == 0 {
            st.remaining.copy_from_slice(weights);
            total = st.remaining.iter().sum();
        }
        if total == 0 {
            return 0;
        }
        let r = self.roll() % total;
        let st = self.pick.get_mut(&kit).expect("pool inserted above");
        let mut acc = 0u32;
        for (i, w) in st.remaining.iter().enumerate() {
            acc += w;
            if acc > r {
                st.remaining[i] -= 1;
                return i;
            }
        }
        st.remaining.len() - 1
    }

    pub(super) fn volume(&mut self, flags: u32, base: f32, mult: f32) -> f32 {
        let draw =
            (flags & sound_kit_flags::VARY_VOLUME != 0).then(|| math::variation_draw(self.roll()));
        math::variation_volume(draw, base, mult)
    }

    pub(super) fn frequency(&mut self, flags: u32) -> Option<i32> {
        (flags & sound_kit_flags::VARY_PITCH != 0)
            .then(|| math::variation_pitch_freq(math::variation_draw(self.roll())))
    }
}

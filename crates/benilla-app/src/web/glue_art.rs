//! Browser delivery for the original complete GlueArt assembler. No second art inventory.

use benilla_assets::{RawFile, SpriteCache, SpriteFiles, SpriteStore};
use bevy::prelude::*;

use crate::glue::{add_material::AddUiMaterial, art::GlueArt};
use crate::glue_strings::GlueStrings;

pub(crate) fn deliver_glue_art(
    server: Res<AssetServer>,
    raw: Res<Assets<RawFile>>,
    mut files: ResMut<SpriteFiles>,
    mut cache: ResMut<SpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<AddUiMaterial>>,
    mut pending: Local<GlueArt>,
    mut art: ResMut<GlueArt>,
    strings: Option<Res<GlueStrings>>,
    mut timing: Local<(f64, f64, u64)>,
) {
    if art.ready() {
        return;
    }
    let mut source = SpriteStore::new(&mut cache, files.reader(&server, &raw))
        .with_budget(benilla_time::Duration::from_millis(4));
    let start = benilla_time::Instant::now();
    pending.ensure_loaded(&mut source, &mut images, &mut materials);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    timing.0 += ms;
    timing.1 = timing.1.max(ms);
    timing.2 += 1;
    if ms >= 16.0 {
        info!("startup cost: glue art assembly {ms:.1}ms this frame");
    }
    // Keep partial delivery private: original screen builders see either the
    // complete terminal art set or no set, never Pending mistaken for missing.
    if !pending.ready() || strings.is_none() {
        return;
    }
    *art = std::mem::take(&mut *pending);
    info!("startup cost: glue art total_cpu={:.1}ms max_frame={:.1}ms passes={}", timing.0, timing.1, timing.2);
    info!("browser glue art READY: complete original inventory published; races={}, classes={}, arrows={}/{}, borders={}/{}",
        art.races.is_some(), art.classes.is_some(), art.arrow_left.is_some(),
        art.arrow_right.is_some(), art.panel_border.is_some(), art.name_border.is_some());
}

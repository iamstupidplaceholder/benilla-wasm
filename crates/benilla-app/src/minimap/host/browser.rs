//! Asynchronous real bytes, then the original catalog/cache implementations.

use super::super::{blips, MinimapAssets};
use benilla_assets::RawFile;
use benilla_formats::{MemoryFileSource, MinimapTranslate, ShapeshiftForm};
use bevy::{asset::LoadState, prelude::*};
use std::collections::HashMap;

const FILES: &[&str] = &[
    "Textures/Minimap/md5translate.trs",
    "DBFilesClient/SpellShapeshiftForm.dbc",
    "DBFilesClient/SpellIcon.dbc",
];

#[derive(Resource)]
struct Delivery(Vec<(&'static str, Handle<RawFile>)>);
#[derive(Resource)]
struct ArtDelivery {
    translate: Option<MinimapTranslate>,
    forms: Option<HashMap<u32, ShapeshiftForm>>,
}

pub(crate) struct MinimapAssetsPlugin;
impl Plugin for MinimapAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, request).add_systems(
            Update,
            (decode, publish_art)
                .chain()
                .before(crate::ui_script::UiFeed),
        );
    }
}

fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Delivery(
        FILES
            .iter()
            .map(|&path| (path, server.load(format!("mpq://{path}"))))
            .collect(),
    ));
}

fn decode(
    mut commands: Commands,
    delivery: Option<Res<Delivery>>,
    raw: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
) {
    let Some(delivery) = delivery else { return };
    if delivery.0.iter().any(|(_, handle)| {
        raw.get(handle).is_none()
            && !matches!(
                server.get_load_state(handle.id()),
                Some(LoadState::Failed(_))
            )
    }) {
        return;
    }
    let mut bytes = Vec::new();
    for (path, handle) in &delivery.0 {
        match raw.get(handle) {
            Some(file) => bytes.push((*path, file.0.clone())),
            None => warn!(
                "minimap: file delivery failed for {path}: {:?}",
                server.get_load_state(handle.id())
            ),
        }
    }
    commands.remove_resource::<Delivery>();
    let mut source = MemoryFileSource::new(bytes);
    let translate = match benilla_formats::load_minimap_translate(&mut source) {
        Ok(translate) => translate,
        Err(e) => {
            error!("minimap: md5translate.trs failed, minimap disabled: {e:#}");
            return;
        }
    };
    info!("minimap: md5translate.trs — {} tiles", translate.len());
    let forms = match benilla_formats::load_shapeshift_forms(&mut source) {
        Ok(forms) => Some(forms),
        Err(e) => {
            warn!("minimap: SpellShapeshiftForm.dbc failed — no shapeshift creature-type override: {e:#}");
            None
        }
    };
    commands.insert_resource(ArtDelivery {
        translate: Some(translate),
        forms,
    });
}

fn publish_art(
    mut commands: Commands,
    delivery: Option<ResMut<ArtDelivery>>,
    mut sprites: crate::ui_sprites::UiSprites,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut delivery) = delivery else { return };
    let mask = sprites.mask_texture_ready("Textures\\MinimapMask", &mut images);
    let arrow = sprites.sprite_texture_ready(blips::PLAYER_ARROW_TEXTURE, &mut images);
    let poi = sprites.sprite_texture_ready("Interface\\Minimap\\POIIcons", &mut images);
    let object_icons = sprites.sprite_texture_ready("Interface\\Minimap\\ObjectIcons", &mut images);
    let mut pending = false;
    let mut rim_arrows = blips::RimArrowArt::default();
    for kind in blips::RimArrow::ALL {
        match sprites.sprite_texture_ready(kind.texture(), &mut images) {
            Some(result) => rim_arrows.set(kind, result),
            None => pending = true,
        }
    }
    let (Some(mask), Some(arrow), Some(poi), Some(object_icons)) = (mask, arrow, poi, object_icons)
    else {
        return;
    };
    if pending {
        return;
    }
    if mask.is_none() {
        warn!("minimap: MinimapMask.blp missing — the map will draw square");
    }
    commands.insert_resource(MinimapAssets {
        translate: delivery
            .translate
            .take()
            .expect("minimap art job consumed once"),
        forms: delivery.forms.take(),
        mask,
        arrow,
        poi,
        object_icons,
        rim_arrows,
    });
    commands.remove_resource::<ArtDelivery>();
}

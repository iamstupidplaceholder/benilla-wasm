//! Original synchronous setup, after native MPQ admission.

use super::super::{blips, MinimapAssets};
use benilla_assets::{AssetSet, LockRecover, WorldAssets};
use bevy::prelude::*;

pub(crate) struct MinimapAssetsPlugin;
impl Plugin for MinimapAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup.after(AssetSet::Open));
    }
}

fn setup(
    mut commands: Commands,
    world_assets: Option<ResMut<WorldAssets>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut assets) = world_assets else {
        return;
    };
    let translate = {
        let mut chain = assets.chain.lock_recover();
        benilla_formats::load_minimap_translate(&mut *chain)
    };
    match translate {
        Ok(translate) => {
            info!("minimap: md5translate.trs — {} tiles", translate.len());
            let mask = assets.mask_texture("Textures\\MinimapMask", &mut images);
            let arrow = assets.sprite_texture(blips::PLAYER_ARROW_TEXTURE, &mut images);
            let poi = assets.sprite_texture("Interface\\Minimap\\POIIcons", &mut images);
            let mut rim_arrows = blips::RimArrowArt::default();
            for kind in blips::RimArrow::ALL {
                rim_arrows.set(kind, assets.sprite_texture(kind.texture(), &mut images));
            }
            let object_icons =
                assets.sprite_texture("Interface\\Minimap\\ObjectIcons", &mut images);
            if mask.is_none() {
                warn!("minimap: MinimapMask.blp missing — the map will draw square");
            }
            let forms = {
                let mut chain = assets.chain.lock_recover();
                match benilla_formats::load_shapeshift_forms(&mut *chain) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        warn!("minimap: SpellShapeshiftForm.dbc failed — no shapeshift creature-type override: {e:#}");
                        None
                    }
                }
            };
            commands.insert_resource(MinimapAssets {
                translate,
                mask,
                arrow,
                poi,
                rim_arrows,
                object_icons,
                forms,
            });
        }
        Err(e) => error!("minimap: md5translate.trs failed, minimap disabled: {e:#}"),
    }
}

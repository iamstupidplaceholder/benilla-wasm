use benilla_assets::{AssetSet, LockRecover, WorldAssets};
use bevy::prelude::*;

pub(crate) struct ActionCatalogsPlugin;

impl Plugin for ActionCatalogsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (load_spells, load_mechanics).after(AssetSet::Open));
    }
}

fn load_spells(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    super::publish_spells(&mut commands, &mut *chain);
}

fn load_mechanics(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    super::publish_mechanics(&mut commands, &mut *chain);
}

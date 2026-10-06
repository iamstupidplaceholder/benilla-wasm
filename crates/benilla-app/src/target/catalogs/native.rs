//! The original startup reads, after native MPQ admission.

use benilla_assets::{AssetSet, LockRecover, WorldAssets};
use bevy::prelude::*;

pub(crate) struct TargetCatalogsPlugin;

impl Plugin for TargetCatalogsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (load_factions, load_creature_types).after(AssetSet::Open),
        );
    }
}

fn load_factions(mut commands: Commands, world_assets: Option<Res<WorldAssets>>) {
    let Some(world_assets) = world_assets else {
        return;
    };
    let mut chain = world_assets.chain.lock_recover();
    super::publish_factions(&mut commands, &mut *chain);
}

fn load_creature_types(mut commands: Commands, world_assets: Option<Res<WorldAssets>>) {
    let Some(world_assets) = world_assets else {
        return;
    };
    let mut chain = world_assets.chain.lock_recover();
    super::publish_creature_types(&mut commands, &mut *chain);
}

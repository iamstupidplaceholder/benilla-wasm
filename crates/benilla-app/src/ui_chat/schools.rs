//! Host admission of the original school vocabulary; shared combat composition
//! receives the catalog through its existing engine-owned VM model.
use bevy::prelude::*;

#[derive(Resource)]
pub(crate) struct DamageSchools(pub benilla_formats::ResistanceCatalog);

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load(mut commands: Commands, assets: Option<Res<benilla_assets::WorldAssets>>) {
    let Some(assets) = assets else { return };
    use benilla_assets::LockRecover;
    match benilla_formats::load_resistance_catalog(&mut *assets.chain.lock_recover()) {
        Ok(catalog) => {
            commands.insert_resource(DamageSchools(catalog));
        }
        Err(error) => warn!("combat log: resistance catalog unavailable: {error:#}"),
    };
}

pub(super) fn feed(
    catalog: Option<Res<DamageSchools>>,
    mut script: Option<NonSendMut<benilla_ui::script::UiScript>>,
) {
    if let (Some(catalog), Some(script)) = (catalog, script.as_mut()) {
        script.set_damage_school_names(&catalog.0.names);
    }
}

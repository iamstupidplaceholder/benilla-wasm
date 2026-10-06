//! Exact synchronous existence; actual bytes/decoding for asynchronous dimensions.

use super::sizes::{refresh_layout, SizeBroker};
use benilla_assets::{RawFile, SpriteFileReader, SpriteFiles, SpriteRead, SpriteSource};
use benilla_ui::script::UiScript;
use bevy::prelude::*;

pub(crate) struct UiAssetProbesPlugin;

impl Plugin for UiAssetProbesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SizeBroker>().add_systems(
            Update,
            (poll, refresh_layout)
                .chain()
                .before(crate::ui_script::UiInput),
        );
    }
}

fn installed(path: &str) -> bool {
    benilla_interface_files::contains(path)
        .expect("VM asset probes require the admitted presence index")
}

pub(crate) fn install(world: &mut World, script: &mut UiScript) {
    assert!(
        benilla_interface_files::ready(),
        "VM probes installed before file/index delivery"
    );
    script.set_texture_probe(Box::new(|path| {
        benilla_assets::sprite_candidates(path)
            .iter()
            .any(|c| installed(c))
    }));
    let seen = std::cell::RefCell::new(std::collections::HashMap::<String, bool>::new());
    script.set_font_probe(Box::new(move |path| {
        if let Some(&found) = seen.borrow().get(path) {
            return found;
        }
        let found = installed(path);
        seen.borrow_mut().insert(path.to_owned(), found);
        found
    }));
    let broker = world.resource::<SizeBroker>().clone();
    script.set_texture_size_probe(Box::new(move |path| {
        broker.probe_resident(path, &mut ResidentSource)
    }));
}

struct ResidentSource;
impl SpriteSource for ResidentSource {
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        if !installed(path) {
            return SpriteRead::Missing;
        }
        match benilla_interface_files::read(path) {
            Some(bytes) => SpriteRead::Bytes(bytes),
            None => SpriteRead::Pending,
        }
    }
}

struct InstalledSource<'a>(SpriteFileReader<'a>);
impl SpriteSource for InstalledSource<'_> {
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        if installed(path) {
            self.0.read_installed(path)
        } else {
            SpriteRead::Missing
        }
    }
}

fn poll(
    broker: Res<SizeBroker>,
    server: Res<AssetServer>,
    raw: Res<Assets<RawFile>>,
    mut files: ResMut<SpriteFiles>,
) {
    if !benilla_interface_files::ready() {
        return;
    }
    broker.poll(&mut InstalledSource(files.reader(&server, &raw)));
}

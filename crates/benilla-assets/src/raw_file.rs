//! Undecoded client files through the ordinary AssetServer. Native MPQ and
//! browser gateway readers both implement the same Reader boundary; no host
//! selection or parsing policy belongs in consumers of these bytes.

use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    prelude::*,
    reflect::TypePath,
};

#[derive(Asset, TypePath)]
pub struct RawFile(pub Vec<u8>);

#[derive(Default, TypePath)]
pub struct RawFileLoader;

impl AssetLoader for RawFileLoader {
    type Asset = RawFile;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _context: &mut LoadContext<'_>,
    ) -> Result<RawFile, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(RawFile(bytes))
    }

    fn extensions(&self) -> &[&str] {
        &[
            "dbc", "blp", "tga", "lua", "xml", "toc", "ttf", "bfi", "trs", "m2", "wav", "mp3",
        ]
    }
}

/// Install once alongside typed loaders. Requests explicitly name `RawFile`,
/// so loading a raw BLP/Lua/M2 file does not replace its Image/LuaSource/model decoder.
pub struct RawFilePlugin;

impl Plugin for RawFilePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<RawFile>()
            .register_asset_loader(RawFileLoader);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{LuaSource, LuaSourceLoader};
    use bevy::asset::{AssetPlugin, LoadState};
    use std::time::{Duration, Instant};

    #[test]
    fn asset_server_preserves_bytes_and_typed_loaders_and_reports_missing() {
        let root = std::env::temp_dir().join(format!("benilla-raw-file-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Invalid UTF-8 must survive unchanged; this adapter never executes Lua.
        let bytes = b"-- original bytes\n\xff\x00";
        std::fs::write(root.join("source.lua"), bytes).unwrap();
        let font_bytes = b"\x00\x01\x00\x00\xff\x80";
        std::fs::write(root.join("face.ttf"), font_bytes).unwrap();
        let metadata_bytes = b"BCPI\x01\x00\x00\x00\x00\x00\x00\x00";
        std::fs::write(root.join("presence.bfi"), metadata_bytes).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin {
                file_path: root.to_str().unwrap().into(),
                ..default()
            })
            .add_plugins(RawFilePlugin)
            .init_asset::<LuaSource>()
            .register_asset_loader(LuaSourceLoader);
        let server = app.world().resource::<AssetServer>().clone();
        let raw = server.load::<RawFile>("source.lua");
        let typed = server.load::<LuaSource>("source.lua");
        let font = server.load::<RawFile>("face.ttf");
        let metadata = server.load::<RawFile>("presence.bfi");
        let missing = server.load::<RawFile>("missing.dbc");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.update();
            let raw_ready = app
                .world()
                .resource::<Assets<RawFile>>()
                .get(&raw)
                .is_some();
            let typed_ready = app
                .world()
                .resource::<Assets<LuaSource>>()
                .get(&typed)
                .is_some();
            if raw_ready
                && typed_ready
                && app
                    .world()
                    .resource::<Assets<RawFile>>()
                    .get(&font)
                    .is_some()
                && matches!(server.load_state(&missing), LoadState::Failed(_))
                && app
                    .world()
                    .resource::<Assets<RawFile>>()
                    .get(&metadata)
                    .is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "raw={:?}, typed={:?}, missing={:?}",
                server.load_state(&raw),
                server.load_state(&typed),
                server.load_state(&missing)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.world()
                .resource::<Assets<RawFile>>()
                .get(&raw)
                .unwrap()
                .0,
            bytes
        );
        assert_eq!(
            app.world()
                .resource::<Assets<LuaSource>>()
                .get(&typed)
                .unwrap()
                .0,
            bytes
        );
        // Byte delivery is not font validation; the production registrar does that.
        assert_eq!(
            app.world()
                .resource::<Assets<RawFile>>()
                .get(&font)
                .unwrap()
                .0,
            font_bytes
        );
        let metadata = &app
            .world()
            .resource::<Assets<RawFile>>()
            .get(&metadata)
            .unwrap()
            .0;
        assert_eq!(metadata, metadata_bytes);
        assert!(benilla_formats::ChainPresenceIndex::decode(metadata).is_ok());
        drop(app);
        std::fs::remove_file(root.join("source.lua")).unwrap();
        std::fs::remove_file(root.join("face.ttf")).unwrap();
        std::fs::remove_file(root.join("presence.bfi")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

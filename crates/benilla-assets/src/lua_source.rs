//! Raw Lua source from the MPQ chain. Parsing and execution belong to `benilla-lua`.

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::reflect::TypePath;

/// A client script's original bytes; Lua source is not necessarily UTF-8.
#[derive(Asset, TypePath)]
pub struct LuaSource(pub Vec<u8>);

#[derive(Default, TypePath)]
pub struct LuaSourceLoader;

impl AssetLoader for LuaSourceLoader {
    type Asset = LuaSource;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _ctx: &mut LoadContext<'_>,
    ) -> Result<LuaSource, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(LuaSource(bytes))
    }

    fn extensions(&self) -> &[&str] {
        &["lua"]
    }
}

//! Raw FrameXML bytes from the MPQ chain. Parsing stays in the UI layer.

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::reflect::TypePath;

#[derive(Asset, TypePath)]
pub struct FrameXmlSource(pub Vec<u8>);

#[derive(Default, TypePath)]
pub struct FrameXmlSourceLoader;

impl AssetLoader for FrameXmlSourceLoader {
    type Asset = FrameXmlSource;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _ctx: &mut LoadContext<'_>,
    ) -> Result<FrameXmlSource, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(FrameXmlSource(bytes))
    }

    fn extensions(&self) -> &[&str] {
        &["xml"]
    }
}

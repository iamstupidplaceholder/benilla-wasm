//! Cursor byte delivery only; selection, fallback and payload compositing stay shared.
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub(super) struct CursorAssets<'w> {
    #[cfg(not(target_arch = "wasm32"))]
    assets: Option<ResMut<'w, benilla_assets::WorldAssets>>,
    #[cfg(target_arch = "wasm32")]
    cache: ResMut<'w, benilla_assets::SpriteCache>,
    #[cfg(target_arch = "wasm32")]
    files: ResMut<'w, benilla_assets::SpriteFiles>,
    #[cfg(target_arch = "wasm32")]
    server: Res<'w, AssetServer>,
    #[cfg(target_arch = "wasm32")]
    raw: Res<'w, Assets<benilla_assets::RawFile>>,
}

impl CursorAssets<'_> {
    /// Outer None is Pending/unavailable host, inner None is a terminal failure.
    pub(super) fn mode(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.assets
                .as_deref_mut()
                .map(|a| a.decode_cursor(path, images))
        }
        #[cfg(target_arch = "wasm32")]
        {
            use benilla_assets::{SpriteRead, SpriteSource};
            match self
                .files
                .reader(&self.server, &self.raw)
                .read_installed(path)
            {
                SpriteRead::Pending => None,
                SpriteRead::Missing => Some(None),
                SpriteRead::Bytes(bytes) => Some(
                    benilla_formats::blp_to_rgba(&bytes)
                        .map(|(w, h, rgba)| images.add(benilla_assets::cursor_texture(w, h, rgba)))
                        .map_err(|e| warn!("cursor: {path} decode failed: {e}"))
                        .ok(),
                ),
            }
        }
    }

    pub(super) fn payload(&mut self, path: &str) -> Option<Option<Vec<u8>>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.assets
                .as_deref_mut()
                .map(|a| super::decode_payload_cursor_rgba(a, path))
        }
        #[cfg(target_arch = "wasm32")]
        {
            use benilla_assets::{SpriteAssets, SpriteStore};
            let mut store =
                SpriteStore::new(&mut self.cache, self.files.reader(&self.server, &self.raw));
            let rgba = super::decode_payload_cursor_rgba(&mut store, path);
            (!store.take_pending()).then_some(rgba)
        }
    }
}

//! Browser paint uses the original sprite cache with asynchronous host reads.

use benilla_assets::{
    RawFile, SpriteCache, SpriteFileReader, SpriteFiles, SpriteRead, SpriteSource,
};
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub(crate) struct UiSprites<'w> {
    cache: ResMut<'w, SpriteCache>,
    files: ResMut<'w, SpriteFiles>,
    server: Res<'w, AssetServer>,
    raw: Res<'w, Assets<RawFile>>,
}

impl UiSprites<'_> {
    pub(crate) fn as_mut(&mut self) -> Option<&mut Self> {
        Some(self)
    }
    pub(crate) fn has_pending(&self) -> bool {
        self.cache.has_pending()
    }
    pub(crate) fn begin_paint(&mut self) {
        self.cache.begin_paint();
    }

    pub(crate) fn mask_texture_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        let mut source = PendingSource {
            reader: self.files.reader(&self.server, &self.raw),
            pending: false,
        };
        let result = self
            .cache
            .auxiliary_load(|cache| cache.mask_texture(&mut source, path, images));
        (!source.pending).then_some(result)
    }

    pub(crate) fn sprite_texture_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        let mut source = PendingSource {
            reader: self.files.reader(&self.server, &self.raw),
            pending: false,
        };
        let result = self
            .cache
            .auxiliary_load(|cache| cache.sprite_texture(&mut source, path, images));
        (!source.pending).then_some(result)
    }

    pub(crate) fn sprite_texture_tiled_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        let mut source = PendingSource {
            reader: self.files.reader(&self.server, &self.raw),
            pending: false,
        };
        let result = self
            .cache
            .auxiliary_load(|cache| cache.sprite_texture_tiled(&mut source, path, images));
        (!source.pending).then_some(result)
    }

    pub(crate) fn sprite_texture(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.cache.sprite_texture(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            images,
        )
    }
    pub(crate) fn sprite_texture_wrapped(
        &mut self,
        path: &str,
        wrap: (bool, bool),
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.cache.sprite_texture_wrapped(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            wrap,
            images,
        )
    }
    pub(crate) fn sprite_texture_tiled(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.cache.sprite_texture_tiled(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            images,
        )
    }
    pub(crate) fn portrait_texture(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.cache.portrait_texture(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            images,
        )
    }
    pub(crate) fn emblem_mask_texture(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.cache.emblem_mask_texture(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            images,
        )
    }
    pub(crate) fn resampled_sprite(
        &mut self,
        path: &str,
        size: (u32, u32),
        images: &mut Assets<Image>,
        resample: impl FnOnce(&[u8], u32, u32, u32, u32) -> Vec<u8>,
    ) -> Option<Handle<Image>> {
        self.cache.resampled_sprite(
            &mut self.files.reader(&self.server, &self.raw),
            path,
            size,
            images,
            resample,
        )
    }
    pub(crate) fn generated_sprite(
        &mut self,
        key: &'static str,
        images: &mut Assets<Image>,
        make: impl FnOnce() -> (u32, u32, Vec<u8>),
    ) -> Handle<Image> {
        self.cache.generated_sprite(key, images, make)
    }
}

/// Per-call readiness does not reset or borrow paint's accumulated pending flag.
struct PendingSource<'a> {
    reader: SpriteFileReader<'a>,
    pending: bool,
}

impl SpriteSource for PendingSource<'_> {
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        let result = self.reader.read_installed(path);
        self.pending |= matches!(result, SpriteRead::Pending);
        result
    }
}

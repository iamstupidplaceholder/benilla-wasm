//! Host-neutral sprite access: original caches/decoders, separate from byte delivery.

use bevy::prelude::*;

use crate::{SpriteCache, SpriteRead, SpriteSource};

/// The image operations an art assembler needs, independent of an MPQ chain or renderer table.
pub trait SpriteAssets {
    fn sprite_texture(&mut self, path: &str, images: &mut Assets<Image>) -> Option<Handle<Image>>;
    fn decode_rgba(&mut self, path: &str) -> Option<(u32, u32, Vec<u8>)>;
    /// A batch with pending reads must retry, not publish missing art.
    fn take_pending(&mut self) -> bool;
    /// Hosts may pace assembly between groups; one decoder call remains atomic.
    fn should_yield(&self) -> bool { false }
}

/// One delivery batch. Pending tracking is isolated from the cache's UI-paint tracking.
pub struct SpriteStore<'a, S> {
    cache: &'a mut SpriteCache,
    source: S,
    pending: bool,
    budget: Option<(benilla_time::Instant, benilla_time::Duration)>,
}

impl<'a, S: SpriteSource> SpriteStore<'a, S> {
    pub fn new(cache: &'a mut SpriteCache, source: S) -> Self {
        Self {
            cache,
            source,
            pending: false,
            budget: None,
        }
    }

    pub fn with_budget(mut self, budget: benilla_time::Duration) -> Self {
        self.budget = Some((benilla_time::Instant::now(), budget));
        self
    }
}

struct TrackingSource<'a, S> {
    source: &'a mut S,
    pending: &'a mut bool,
}

impl<S: SpriteSource> SpriteSource for TrackingSource<'_, S> {
    fn store_description(&self) -> &str {
        self.source.store_description()
    }
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        let read = self.source.read_installed(path);
        *self.pending |= matches!(read, SpriteRead::Pending);
        read
    }
    fn read_loose(&mut self, path: &str) -> SpriteRead {
        let read = self.source.read_loose(path);
        *self.pending |= matches!(read, SpriteRead::Pending);
        read
    }
}

impl<S: SpriteSource> SpriteAssets for SpriteStore<'_, S> {
    fn sprite_texture(&mut self, path: &str, images: &mut Assets<Image>) -> Option<Handle<Image>> {
        let mut source = TrackingSource {
            source: &mut self.source,
            pending: &mut self.pending,
        };
        self.cache
            .auxiliary_load(|cache| cache.sprite_texture(&mut source, path, images))
    }
    fn decode_rgba(&mut self, path: &str) -> Option<(u32, u32, Vec<u8>)> {
        let mut source = TrackingSource {
            source: &mut self.source,
            pending: &mut self.pending,
        };
        self.cache
            .auxiliary_load(|cache| cache.decode_rgba(&mut source, path))
            .flatten()
    }
    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
    fn should_yield(&self) -> bool {
        self.budget.is_some_and(|(start, budget)| start.elapsed() >= budget)
    }
}

impl SpriteAssets for crate::WorldAssets {
    fn sprite_texture(&mut self, path: &str, images: &mut Assets<Image>) -> Option<Handle<Image>> {
        crate::WorldAssets::sprite_texture(self, path, images)
    }
    fn decode_rgba(&mut self, path: &str) -> Option<(u32, u32, Vec<u8>)> {
        crate::WorldAssets::decode_rgba(self, path)
    }
    fn take_pending(&mut self) -> bool {
        false
    }
}

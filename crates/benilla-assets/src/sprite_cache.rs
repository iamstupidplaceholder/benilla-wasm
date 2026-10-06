//! Original WorldAssets UI image caches, without world/GPU-light/MPQ ownership.
//! Only completed reads are cached. Image construction remains in world_assets/images.

use crate::sprite_source::decode_completed;
use crate::{
    mask_image, portrait_image, sprite_image, sprite_image_wrapped, sprite_key, SpriteSource,
};
use bevy::prelude::*;
use std::collections::HashMap;

type Pixels = (u32, u32, Vec<u8>);

#[derive(Resource, Default)]
pub struct SpriteCache {
    pending: bool,
    sprites: HashMap<String, Option<Handle<Image>>>,
    tiled_sprites: HashMap<(String, bool, bool), Option<Handle<Image>>>,
    resampled_sprites: HashMap<(String, u32, u32), Option<Handle<Image>>>,
    resample_sources: HashMap<String, Option<Pixels>>,
    decoded_sources: HashMap<String, Option<Pixels>>,
    portraits: HashMap<String, Option<Handle<Image>>>,
    masks: HashMap<String, Option<Handle<Image>>>,
    generated: HashMap<&'static str, Handle<Image>>,
}

impl SpriteCache {
    /// Held quads cannot be reused while a previous conversion awaits bytes.
    pub fn has_pending(&self) -> bool {
        self.pending
    }

    /// Full paint records only its currently visible pending paths.
    pub fn begin_paint(&mut self) {
        self.pending = false;
    }

    /// Other consumers (minimap setup/mask changes) retry their own pending IO.
    /// Those loads must not mark unrelated held UI quads as awaiting paint bytes.
    pub fn auxiliary_load<T>(&mut self, load: impl FnOnce(&mut Self) -> T) -> T {
        let paint_pending = self.pending;
        let result = load(self);
        self.pending = paint_pending;
        result
    }

    fn decode(&mut self, source: &mut impl SpriteSource, path: &str) -> Option<Option<Pixels>> {
        let decoded = decode_completed(source, path);
        self.pending |= decoded.is_none();
        decoded
    }
    /// Raw pixels for original art assemblers. Pending is never cached as a miss.
    pub fn decode_rgba(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
    ) -> Option<Option<Pixels>> {
        let key = sprite_key(path);
        if let Some(cached) = self.decoded_sources.get(&key) {
            return Some(cached.clone());
        }
        let decoded = self.decode(source, path)?;
        self.decoded_sources.insert(key, decoded.clone());
        Some(decoded)
    }
    /// Original loose-root change policy: keep hits, evict sprite/portrait/mask misses.
    pub fn clear_misses(&mut self) {
        self.decoded_sources.retain(|_, v| v.is_some());
        self.sprites.retain(|_, v| v.is_some());
        self.tiled_sprites.retain(|_, v| v.is_some());
        self.portraits.retain(|_, v| v.is_some());
        self.masks.retain(|_, v| v.is_some());
    }

    /// Original sRGB/clamp/mip-0 sprite; keys, hits and terminal misses are unchanged.
    pub fn sprite_texture(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = sprite_key(path);
        if let Some(cached) = self.sprites.get(&key) {
            return cached.clone();
        }
        let loaded = self
            .decode(source, path)?
            .map(|(w, h, rgba)| images.add(sprite_image(w, h, rgba)));
        self.sprites.insert(key, loaded.clone());
        loaded
    }

    /// Exact-size caller resampling; reuse original decoded pixels across sizes.
    pub fn resampled_sprite(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        (w, h): (u32, u32),
        images: &mut Assets<Image>,
        resample: impl FnOnce(&[u8], u32, u32, u32, u32) -> Vec<u8>,
    ) -> Option<Handle<Image>> {
        let key = (path.to_string(), w, h);
        if let Some(cached) = self.resampled_sprites.get(&key) {
            return cached.clone();
        }
        if !self.resample_sources.contains_key(path) {
            let decoded = self.decode(source, path)?;
            self.resample_sources.insert(path.to_string(), decoded);
        }
        let made = self.resample_sources[path]
            .as_ref()
            .map(|(sw, sh, src)| images.add(sprite_image(w, h, resample(src, *sw, *sh, w, h))));
        self.resampled_sprites.insert(key, made.clone());
        made
    }

    pub fn sprite_texture_tiled(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.sprite_texture_wrapped(source, path, (true, true), images)
    }

    /// Samplers are baked into Image, so each wrap pair retains its own cache entry.
    pub fn sprite_texture_wrapped(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        wrap: (bool, bool),
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = (sprite_key(path), wrap.0, wrap.1);
        if let Some(cached) = self.tiled_sprites.get(&key) {
            return cached.clone();
        }
        let loaded = self
            .decode(source, path)?
            .map(|(w, h, rgba)| images.add(sprite_image_wrapped(w, h, rgba, wrap)));
        self.tiled_sprites.insert(key, loaded.clone());
        loaded
    }

    /// Original caller-generated colour-picker sprites, built once per static key.
    pub fn generated_sprite(
        &mut self,
        key: &'static str,
        images: &mut Assets<Image>,
        make: impl FnOnce() -> Pixels,
    ) -> Handle<Image> {
        if let Some(cached) = self.generated.get(key) {
            return cached.clone();
        }
        let (w, h, rgba) = make();
        let handle = images.add(sprite_image(w, h, rgba));
        self.generated.insert(key, handle.clone());
        handle
    }

    pub fn portrait_texture(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = sprite_key(path);
        if let Some(cached) = self.portraits.get(&key) {
            return cached.clone();
        }
        let loaded = self
            .decode(source, path)?
            .map(|(w, h, rgba)| images.add(portrait_image(w, h, rgba)));
        self.portraits.insert(key, loaded.clone());
        loaded
    }

    /// Original emblem law: white RGB carrying the source alpha, not a new mask format.
    pub fn emblem_mask_texture(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = format!("emblem-mask:{}", sprite_key(path));
        if let Some(cached) = self.sprites.get(&key) {
            return cached.clone();
        }
        let loaded = self.decode(source, path)?.map(|(w, h, mut rgba)| {
            for px in rgba.as_chunks_mut::<4>().0 {
                px[0] = 0xFF;
                px[1] = 0xFF;
                px[2] = 0xFF;
            }
            images.add(sprite_image(w, h, rgba))
        });
        self.sprites.insert(key, loaded.clone());
        loaded
    }

    /// Coverage mask stays linear/clamp, distinct from ordinary sRGB sprite images.
    pub fn mask_texture(
        &mut self,
        source: &mut impl SpriteSource,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = sprite_key(path);
        if let Some(cached) = self.masks.get(&key) {
            return cached.clone();
        }
        let loaded = self
            .decode(source, path)?
            .map(|(w, h, rgba)| images.add(mask_image(w, h, rgba)));
        self.masks.insert(key, loaded.clone());
        loaded
    }
}

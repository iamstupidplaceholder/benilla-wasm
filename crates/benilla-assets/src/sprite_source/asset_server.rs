//! Async host byte delivery, using the registered mpq:// reader and RawFileLoader.
//! On browser that reader is HTTP-backed. No sprite cache or decoding lives here.

use super::{SpriteRead, SpriteSource};
use crate::RawFile;
use bevy::{asset::LoadState, prelude::*};
use std::collections::{HashMap, HashSet};

#[derive(Resource, Default)]
pub struct SpriteFiles {
    presence: Option<fn(&str) -> Option<bool>>,
    requests: HashMap<String, Handle<RawFile>>,
    reported_failures: HashSet<String>,
}

impl SpriteFiles {
    /// Exact host inventory, when admitted. Unknown must stay unknown until
    /// metadata arrives; a confirmed absent candidate is not a delivery error.
    pub fn with_presence_probe(probe: fn(&str) -> Option<bool>) -> Self {
        Self {
            presence: Some(probe),
            ..Self::default()
        }
    }
    pub fn reader<'a>(
        &'a mut self,
        server: &'a AssetServer,
        assets: &'a Assets<RawFile>,
    ) -> SpriteFileReader<'a> {
        SpriteFileReader {
            files: self,
            server,
            assets,
        }
    }
}

pub struct SpriteFileReader<'a> {
    files: &'a mut SpriteFiles,
    server: &'a AssetServer,
    assets: &'a Assets<RawFile>,
}

impl SpriteSource for SpriteFileReader<'_> {
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        if self
            .files
            .presence
            .is_some_and(|probe| probe(path) == Some(false))
        {
            return SpriteRead::Missing;
        }
        let key = crate::normalize_path(path);
        let handle = self.files.requests.entry(key.clone()).or_insert_with(|| {
            self.server
                .load::<RawFile>(format!("mpq://{}", key.replace('\\', "/")))
        });
        if let Some(file) = self.assets.get(&*handle) {
            return SpriteRead::Bytes(file.0.clone());
        }
        if let Some(LoadState::Failed(error)) = self.server.get_load_state(handle.id()) {
            if self.files.reported_failures.insert(key) {
                warn!("sprite_source: {path} delivery failed: {error}");
            }
            return SpriteRead::Missing;
        }
        SpriteRead::Pending
    }
}

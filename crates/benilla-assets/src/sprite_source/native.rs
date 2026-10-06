//! Native installed/loose read boundary; no resolution or decoding policy here.

use super::{SpriteRead, SpriteSource};
use crate::{loose_addon_file, LockRecover};
use benilla_formats::Chain;
use std::{path::Path, sync::Mutex};

pub(crate) struct NativeSpriteSource<'a> {
    pub chain: &'a Mutex<Chain>,
    pub loose_root: Option<&'a Path>,
}

impl SpriteSource for NativeSpriteSource<'_> {
    fn store_description(&self) -> &str {
        if self.loose_root.is_some() {
            "patch chain or the AddOns folder"
        } else {
            "patch chain"
        }
    }
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        match self.chain.lock_recover().read(path) {
            Ok(bytes) => SpriteRead::Bytes(bytes),
            Err(_) => SpriteRead::Missing,
        }
    }

    fn read_loose(&mut self, path: &str) -> SpriteRead {
        let bytes = self
            .loose_root
            .and_then(|root| loose_addon_file(root, path))
            .and_then(|file| std::fs::read(file).ok());
        bytes.map_or(SpriteRead::Missing, SpriteRead::Bytes)
    }
}

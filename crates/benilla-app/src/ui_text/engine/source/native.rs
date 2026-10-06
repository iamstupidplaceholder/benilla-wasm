//! Original font stores: the app's patch chain, then its one loose AddOns root.

use benilla_assets::{LockRecover, WorldAssets};
use std::sync::{Arc, Mutex};

pub(in crate::ui_text::engine) struct FontSource {
    // Same engine-private access retained for the original native font tests.
    pub(in crate::ui_text::engine) chain: Arc<Mutex<benilla_formats::Chain>>,
    pub(in crate::ui_text::engine) loose_root: Option<std::path::PathBuf>,
}

impl FontSource {
    pub(in crate::ui_text::engine) fn open(assets: Option<&WorldAssets>) -> Option<Self> {
        let assets = assets?;
        Some(Self {
            chain: assets.chain.clone(),
            loose_root: crate::ui_script::addons::root(),
        })
    }

    pub(in crate::ui_text::engine) fn read_client(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        self.chain.lock_recover().read(path)
    }

    pub(in crate::ui_text::engine) fn read(&self, path: &str) -> Option<Vec<u8>> {
        benilla_assets::read_chain_or_loose(&self.chain, self.loose_root.as_deref(), path)
    }
}

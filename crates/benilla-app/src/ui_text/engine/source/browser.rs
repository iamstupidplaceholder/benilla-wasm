//! Real resident TTF bytes delivered by AssetServer, not a fabricated WorldAssets.

use benilla_assets::WorldAssets;

pub(in crate::ui_text::engine) struct FontSource;

impl FontSource {
    pub(in crate::ui_text::engine) fn pending(&self, path: &str) -> bool {
        benilla_interface_files::request_font(path)
    }
    pub(in crate::ui_text::engine) fn open(_assets: Option<&WorldAssets>) -> Option<Self> {
        benilla_interface_files::ready().then_some(Self)
    }

    pub(in crate::ui_text::engine) fn read_client(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        benilla_interface_files::try_read(path)
    }

    pub(in crate::ui_text::engine) fn read(&self, path: &str) -> Option<Vec<u8>> {
        benilla_interface_files::read(path)
    }
}

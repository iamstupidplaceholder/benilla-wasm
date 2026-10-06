//! Host byte source for the existing synchronous FrameXML/Lua loader.
//! Browser publication is atomic: no partial snapshot is visible to the loader.
//! Missing files stay missing; this crate does not parse or execute UI content.

mod addons;
#[cfg(any(target_arch = "wasm32", test))]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
pub use addons::AddonCatalog;

/// The production manifest's two-store rule: paths name installed files,
/// bare filenames name the client's own compiled-in UI tree.
pub fn is_install_path(entry: &str) -> bool {
    entry.contains('\\') || entry.contains('/')
}

pub fn read(path: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::read(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        browser::read(path)
    }
}

/// Whether host delivery is complete. Native reads are synchronous; browser
/// boot and entry must wait until the resident snapshot is published.
pub fn ready() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
    #[cfg(target_arch = "wasm32")]
    {
        browser::ready()
    }
}

/// Byte read retaining the IO error for boot-file diagnostics.
pub fn try_read(path: &str) -> anyhow::Result<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::try_read(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        browser::try_read(path)
    }
}

/// Exact installed-file existence, independent of delivered byte residency.
/// Pending/no install is an error, never an invented negative lookup verdict.
pub fn contains(path: &str) -> anyhow::Result<bool> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::contains(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        browser::contains(path)
    }
}

/// Publish a complete, successfully delivered install-file snapshot.
#[cfg(target_arch = "wasm32")]
pub use browser::publish;
#[cfg(target_arch = "wasm32")]
pub use browser::{publish_with_addons, installed_addons};
#[cfg(target_arch = "wasm32")]
pub use browser::{finish_font, font_revision, request_font, take_font_requests};

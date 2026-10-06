//! Local text-state storage. Callers own filenames, formats and session policy;
//! only this crate chooses filesystem versus origin-scoped browser storage.
//! No Bevy or Lua dependency, and no backend types in the public API.

use std::{io, path::Path};

#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod backend;
#[cfg(target_arch = "wasm32")]
#[path = "browser.rs"]
mod backend;
#[cfg(any(target_arch = "wasm32", test))]
mod key;

pub const STATE_DIR: &str = "benilla-config";

/// Resolve the state root. `dev_source_dir` is the optional application crate
/// directory; shipped applications supply `None`, not a compiled source path.
pub fn home(dev_source_dir: Option<&Path>) -> Option<std::path::PathBuf> {
    backend::home(dev_source_dir)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    backend::read_to_string(path.as_ref())
}

/// Read saved text as bytes, preserving native legacy Lua/source encodings.
/// Browser entries are the UTF-8 text written by `write_atomic`.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    backend::read(path.as_ref())
}

/// Replace a complete text entry without exposing an intermediate partial write.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    backend::write_atomic(path, contents)
}

/// Missing entries report `NotFound` on both platforms.
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    backend::remove_file(path.as_ref())
}

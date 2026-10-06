use crate::key::browser_key;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn home(_dev_source_dir: Option<&Path>) -> Option<PathBuf> {
    if std::env::var_os("WOW_CAPTURE").is_some() {
        return None;
    }
    Some(PathBuf::from(crate::STATE_DIR))
}

fn storage() -> io::Result<web_sys::Storage> {
    web_sys::window()
        .ok_or_else(|| io::Error::other("browser window unavailable"))?
        .local_storage()
        .map_err(storage_error)?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "browser local storage unavailable",
            )
        })
}

fn storage_error(error: wasm_bindgen::JsValue) -> io::Error {
    io::Error::other(format!("browser local storage: {error:?}"))
}

pub(super) fn read_to_string(path: &Path) -> io::Result<String> {
    storage()?
        .get_item(&browser_key(path)?)
        .map_err(storage_error)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "state entry not found"))
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    let storage = storage()?;
    let key = browser_key(path)?;
    if storage.get_item(&key).map_err(storage_error)?.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "state entry not found",
        ));
    }
    storage.remove_item(&key).map_err(storage_error)
}

pub(super) fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    // A failed setItem leaves the previous entry intact. Do not replace errors
    // with an in-memory fallback that only pretends to persist.
    storage()?
        .set_item(&browser_key(path)?, contents)
        .map_err(storage_error)
}

pub(super) fn read(path: &Path) -> io::Result<Vec<u8>> {
    read_to_string(path).map(String::into_bytes)
}

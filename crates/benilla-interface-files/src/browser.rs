//! Resident bytes delivered by AssetServer; never attempts filesystem IO.

use benilla_formats::{ChainPresenceIndex, FileSource, MemoryFileSource};
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

struct Snapshot {
    bytes: MemoryFileSource,
    presence: ChainPresenceIndex,
    addons: crate::AddonCatalog,
    requested: HashSet<String>,
    failed: HashSet<String>,
    requests: Vec<String>,
    revision: u64,
}

fn files() -> &'static Mutex<Option<Snapshot>> {
    static FILES: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();
    FILES.get_or_init(|| Mutex::new(None))
}

pub fn publish(entries: impl IntoIterator<Item = (String, Vec<u8>)>, presence: ChainPresenceIndex) {
    publish_with_addons(entries, presence, crate::AddonCatalog::default());
}

pub fn publish_with_addons(
    entries: impl IntoIterator<Item = (String, Vec<u8>)>,
    presence: ChainPresenceIndex,
    addons: crate::AddonCatalog,
) {
    let snapshot = Snapshot {
        bytes: MemoryFileSource::new(entries),
        presence,
        addons,
        requested: HashSet::new(),
        failed: HashSet::new(),
        requests: Vec::new(),
        revision: 1,
    };
    *files().lock().expect("interface source lock poisoned") = Some(snapshot);
}

pub fn request_font(path: &str) -> bool {
    let mut files = files().lock().expect("interface source lock poisoned");
    let Some(snapshot) = files.as_mut() else {
        return true;
    };
    let key = path.replace('\\', "/").to_ascii_lowercase();
    if snapshot.bytes.read_file(path).is_ok()
        || snapshot.failed.contains(&key)
        || !(snapshot.addons.contains(path) || snapshot.presence.contains(path))
    {
        return false;
    }
    if snapshot.requested.insert(key) {
        snapshot.requests.push(path.to_owned());
    }
    true
}

pub fn take_font_requests() -> Vec<String> {
    let mut files = files().lock().expect("interface source lock poisoned");
    files
        .as_mut()
        .map(|s| std::mem::take(&mut s.requests))
        .unwrap_or_default()
}

pub fn finish_font(path: &str, bytes: Option<Vec<u8>>) {
    let mut files = files().lock().expect("interface source lock poisoned");
    if let Some(snapshot) = files.as_mut() {
        if let Some(bytes) = bytes {
            snapshot.bytes.insert(path, bytes);
        } else {
            snapshot
                .failed
                .insert(path.replace('\\', "/").to_ascii_lowercase());
        }
        snapshot.revision += 1;
    }
}

pub fn font_revision() -> u64 {
    files()
        .lock()
        .expect("interface source lock poisoned")
        .as_ref()
        .map_or(0, |s| s.revision)
}

pub(super) fn ready() -> bool {
    files()
        .lock()
        .expect("interface source lock poisoned")
        .is_some()
}

pub(super) fn try_read(path: &str) -> anyhow::Result<Vec<u8>> {
    files()
        .lock()
        .expect("interface source lock poisoned")
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("interface delivery pending: {path}"))?
        .bytes
        .read_file(path)
}

pub(super) fn contains(path: &str) -> anyhow::Result<bool> {
    let files = files().lock().expect("interface source lock poisoned");
    let snapshot = files
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("interface delivery pending: {path}"))?;
    Ok(snapshot.addons.contains(path) || snapshot.presence.contains(path))
}

pub fn installed_addons() -> Vec<(String, String)> {
    files()
        .lock()
        .expect("interface source lock poisoned")
        .as_ref()
        .map(|s| {
            s.addons
                .manifests()
                .map(|(name, path)| (name.to_owned(), path.to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn read(path: &str) -> Option<Vec<u8>> {
    match try_read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::debug!("ui_script: {path} is not resident: {error:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_publication_preserves_bytes_and_replaces_old_snapshot() {
        assert!(!ready());
        assert!(try_read("pending.xml").is_err());
        assert!(contains("pending.xml").is_err());
        let empty_index =
            || ChainPresenceIndex::decode(b"BCPI\x01\x00\x00\x00\x00\x00\x00\x00").unwrap();
        publish(
            [(r"Interface\FrameXML\Locale.lua".into(), vec![0xff, 0x80, 0])],
            empty_index(),
        );
        assert!(ready());
        let mut first = read("interface/framexml/LOCALE.lua").unwrap();
        first[0] = 0;
        assert_eq!(
            read(r"Interface\FrameXML\Locale.lua").unwrap(),
            [0xff, 0x80, 0]
        );
        assert!(read("missing.xml").is_none());
        // Existence follows the archive index, not resident-byte membership.
        assert!(!contains(r"Interface\FrameXML\Locale.lua").unwrap());
        publish([("empty.lua".into(), vec![])], empty_index());
        assert!(read(r"Interface\FrameXML\Locale.lua").is_none());
        assert_eq!(read("empty.lua"), Some(vec![]));
    }
}

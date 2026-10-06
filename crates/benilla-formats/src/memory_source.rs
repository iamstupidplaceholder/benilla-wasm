//! Resident files delivered by an asynchronous host, exposed to the existing
//! synchronous decoders. This is a byte source, not an alternative decoder or
//! a fallback for failed IO: absent files remain errors.

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use crate::FileSource;

#[derive(Default)]
pub struct MemoryFileSource {
    files: HashMap<String, Vec<u8>>,
}

impl MemoryFileSource {
    pub fn insert(&mut self, name: &str, bytes: Vec<u8>) {
        self.files.insert(key(name), bytes);
    }
    pub fn new(files: impl IntoIterator<Item = (impl AsRef<str>, Vec<u8>)>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(name, bytes)| (key(name.as_ref()), bytes))
                .collect(),
        }
    }
}

fn key(name: &str) -> String {
    name.replace('\\', "/").to_ascii_lowercase()
}

impl FileSource for MemoryFileSource {
    fn read_file(&mut self, name: &str) -> Result<Vec<u8>> {
        self.files
            .get(&key(name))
            .cloned()
            .ok_or_else(|| anyhow!("asset not resident: {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpq_spellings_share_identity() {
        let mut source = MemoryFileSource::new([("DBFilesClient/Map.dbc", vec![0, 255, 42])]);
        assert_eq!(
            source.read_file(r"dbfilesclient\MAP.DBC").unwrap(),
            [0, 255, 42]
        );
    }

    #[test]
    fn later_entry_overrides_and_reads_do_not_consume() {
        let mut source = MemoryFileSource::new([("a/file", vec![1]), (r"A\FILE", vec![2])]);
        let mut first = source.read_file("a/file").unwrap();
        first[0] = 3;
        assert_eq!(source.read_file("a/file").unwrap(), [2]);
    }

    #[test]
    fn missing_is_not_an_empty_file() {
        let mut source = MemoryFileSource::new([("empty", vec![])]);
        assert!(source.read_file("empty").unwrap().is_empty());
        let error = source.read_file("missing").unwrap_err();
        assert!(error.to_string().contains("missing"));
    }
}

//! Browser implementation of the existing `mpq://` Bevy asset source.
//!
//! Reads stay asynchronous all the way to `fetch()`. The gateway receives only a validated logical
//! client path; sampler markers are stripped before both the request and the bounded byte cache.

use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
};

use benilla_formats::normalize_client_path;
use bevy::{
    asset::{
        AssetApp,
        io::{
            AssetReader, AssetReaderError, AssetSourceBuilder, ErasedAssetReader, PathStream,
            Reader, VecReader,
        },
    },
    prelude::*,
};

use crate::{MPQ_SOURCE, WebAssetStats, strip_sampler_marker};

const DEFAULT_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
struct WebMpqAssetReader {
    endpoint: Arc<str>,
    cache: Arc<Mutex<WebByteCache>>,
    stats: WebAssetStats,
}

struct WebByteCache {
    entries: HashMap<String, Vec<u8>>,
    insertion_order: VecDeque<String>,
    resident_bytes: usize,
    limit_bytes: usize,
}

impl WebByteCache {
    fn new(limit_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            resident_bytes: 0,
            limit_bytes,
        }
    }

    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.entries.get(path).cloned()
    }

    fn insert(&mut self, path: String, bytes: Vec<u8>) -> (usize, usize) {
        if bytes.len() > self.limit_bytes || self.entries.contains_key(&path) {
            return (self.entries.len(), self.resident_bytes);
        }
        while self.resident_bytes.saturating_add(bytes.len()) > self.limit_bytes {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.resident_bytes = self.resident_bytes.saturating_sub(old.len());
            }
        }
        self.resident_bytes = self.resident_bytes.saturating_add(bytes.len());
        self.insertion_order.push_back(path.clone());
        self.entries.insert(path, bytes);
        (self.entries.len(), self.resident_bytes)
    }
}

impl WebMpqAssetReader {
    fn new(endpoint: String, stats: WebAssetStats) -> Self {
        Self {
            endpoint: endpoint.into(),
            cache: Arc::new(Mutex::new(WebByteCache::new(DEFAULT_CACHE_BYTES))),
            stats,
        }
    }
}

impl AssetReader for WebMpqAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let raw = path
            .to_str()
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))?;
        let stripped = strip_sampler_marker(raw);
        let logical =
            normalize_client_path(stripped.as_deref().unwrap_or(raw)).map_err(|error| {
                AssetReaderError::Io(Arc::new(std::io::Error::other(error.to_string())))
            })?;

        if let Some(bytes) = self
            .cache
            .lock()
            .expect("web MPQ cache poisoned")
            .get(&logical)
        {
            self.stats.record_cache_hit();
            debug!("web mpq cache HIT: {logical} ({} bytes)", bytes.len());
            return Ok(VecReader::new(bytes));
        }

        let encoded = encode_url_path(&logical);
        let bytes = crate::web_delivery::read(self.endpoint.as_ref(), Path::new(&encoded)).await?;
        self.stats.record_fetch(bytes.len());
        debug!("web mpq cache MISS: {logical} ({} bytes)", bytes.len());
        let (entries, resident_bytes) = self
            .cache
            .lock()
            .expect("web MPQ cache poisoned")
            .insert(logical, bytes.clone());
        self.stats.record_cache_residency(entries, resident_bytes);
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // MPQ assets never have Bevy sidecar metadata. Avoid a guaranteed extra HTTP 404.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

/// Register the browser `mpq://` source before Bevy's `AssetPlugin` is installed.
pub fn register_web_mpq_source(app: &mut App, endpoint: impl Into<String>) {
    let stats = WebAssetStats::default();
    let reader = WebMpqAssetReader::new(endpoint.into(), stats.clone());
    app.insert_resource(stats);
    app.register_asset_source(
        MPQ_SOURCE,
        AssetSourceBuilder::new(move || -> Box<dyn ErasedAssetReader> { Box::new(reader.clone()) }),
    );
}

/// Separate HTTP metadata namespace; never pretend an index is an MPQ file.
struct WebPresenceAssetReader(String);

impl AssetReader for WebPresenceAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Ok(VecReader::new(
            crate::web_delivery::read(&self.0, path).await?,
        ))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // The gateway publishes the binary index, not Bevy .meta sidecars.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

pub fn register_web_presence_source(app: &mut App, endpoint: impl Into<String>) {
    register_web_metadata_source(app, "mpqmeta", endpoint);
}

pub fn register_web_metadata_source(
    app: &mut App,
    source: &'static str,
    endpoint: impl Into<String>,
) {
    let endpoint = endpoint.into();
    app.register_asset_source(
        source,
        AssetSourceBuilder::new(move || -> Box<dyn ErasedAssetReader> {
            Box::new(WebPresenceAssetReader(endpoint.clone()))
        }),
    );
}

fn encode_url_path(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_each_path_component_without_hiding_slashes() {
        assert_eq!(
            encode_url_path("interface/icons/inv misc(1).blp"),
            "interface/icons/inv%20misc%281%29.blp"
        );
    }
}

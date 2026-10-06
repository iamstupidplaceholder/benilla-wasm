//! Small shared counters for the browser `mpq://` reader.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use bevy::prelude::Resource;

#[derive(Resource, Clone, Default)]
pub struct WebAssetStats {
    inner: Arc<WebAssetStatsInner>,
}

#[derive(Default)]
struct WebAssetStatsInner {
    fetches: AtomicUsize,
    cache_hits: AtomicUsize,
    transferred_bytes: AtomicUsize,
    cache_entries: AtomicUsize,
    cache_resident_bytes: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WebAssetStatsSnapshot {
    pub fetches: usize,
    pub cache_hits: usize,
    pub transferred_bytes: usize,
    pub cache_entries: usize,
    pub cache_resident_bytes: usize,
}

impl WebAssetStats {
    pub fn snapshot(&self) -> WebAssetStatsSnapshot {
        WebAssetStatsSnapshot {
            fetches: self.inner.fetches.load(Ordering::Relaxed),
            cache_hits: self.inner.cache_hits.load(Ordering::Relaxed),
            transferred_bytes: self.inner.transferred_bytes.load(Ordering::Relaxed),
            cache_entries: self.inner.cache_entries.load(Ordering::Relaxed),
            cache_resident_bytes: self.inner.cache_resident_bytes.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_fetch(&self, bytes: usize) {
        self.inner.fetches.fetch_add(1, Ordering::Relaxed);
        self.inner
            .transferred_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn record_cache_hit(&self) {
        self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_cache_residency(&self, entries: usize, bytes: usize) {
        self.inner.cache_entries.store(entries, Ordering::Relaxed);
        self.inner
            .cache_resident_bytes
            .store(bytes, Ordering::Relaxed);
    }
}

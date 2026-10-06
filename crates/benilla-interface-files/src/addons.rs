//! Validated host catalog only. TOC metadata and load order belong to the UI loader.

use std::collections::{BTreeMap, HashMap};

use anyhow::{bail, Context, Result};
use benilla_formats::normalize_client_path;

#[derive(Clone, Default)]
pub struct AddonCatalog {
    files: BTreeMap<String, String>,
    manifests: BTreeMap<String, (String, String)>,
}

impl AddonCatalog {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 64 * 1024 * 1024 {
            bail!("addon catalog exceeds size limit");
        }
        let text = std::str::from_utf8(bytes).context("addon catalog is not UTF-8")?;
        let text = text
            .strip_prefix("BADDONS1\n")
            .context("invalid addon catalog version")?;
        if !text.is_empty() && !text.ends_with('\n') {
            bail!("truncated addon catalog");
        }
        let mut catalog = Self::default();
        let mut folders = HashMap::<String, String>::new();
        for path in text.lines() {
            if catalog.files.len() >= 100_000 {
                bail!("addon catalog file count exceeds limit");
            }
            let key = normalize_client_path(path)?;
            let rel = key
                .strip_prefix("interface/addons/")
                .context("addon file outside AddOns")?;
            let (folder_key, _) = rel.split_once('/').context("addon file has no folder")?;
            let folder_key = folder_key.to_owned();
            let components: Vec<_> = path.split('/').collect();
            if components.len() < 4 || path.contains('\\') {
                bail!("addon catalog must use install-relative forward-slash paths");
            }
            let folder = components[2];
            if let Some(previous) = folders.insert(folder_key.clone(), folder.to_owned()) {
                if previous != folder {
                    bail!("ambiguous addon folder spelling: {folder}");
                }
            }
            if catalog.files.insert(key, path.to_owned()).is_some() {
                bail!("duplicate case-insensitive addon file: {path}");
            }
            if components.len() == 4 && components[3].eq_ignore_ascii_case(&format!("{folder}.toc"))
            {
                catalog
                    .manifests
                    .insert(folder_key, (folder.to_owned(), path.to_owned()));
            }
        }
        for folder in folders.keys() {
            if !catalog.manifests.contains_key(folder) {
                bail!("catalog addon has no own TOC: {folder}");
            }
        }
        Ok(catalog)
    }

    pub fn contains(&self, path: &str) -> bool {
        normalize_client_path(path).is_ok_and(|key| self.files.contains_key(&key))
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files.values().map(String::as_str)
    }

    /// Authored folder name and its actual (possibly differently-cased) TOC path.
    /// Consumers apply the original registry ordering, not this map's ordering.
    pub fn manifests(&self) -> impl Iterator<Item = (&str, &str)> {
        self.manifests
            .values()
            .map(|(name, path)| (name.as_str(), path.as_str()))
    }
}

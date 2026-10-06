//! Read-only loose addon host. No TOC interpretation, Lua execution or load order.
//! Only indexed files in TOC-bearing addon folders can be addressed by clients.

use super::*;

pub(crate) const ROUTE: &str = "/assets/v1/addons.bai";
const PREFIX: &str = "interface/addons/";
const MAX_FILES: usize = 100_000;
const MAX_DEPTH: usize = 64;

struct LooseFile {
    logical: String,
    path: PathBuf,
    size: u64,
}

pub(crate) struct LooseAddons {
    root: PathBuf,
    files: HashMap<String, LooseFile>,
    pub(crate) catalog: CachedAsset,
}

impl LooseAddons {
    /// One explicit root, or the installed client's Interface/AddOns folder.
    /// Discovery only lists the supplied installation, never arbitrary home paths.
    pub(crate) fn open(explicit: Option<PathBuf>, data: &Path, limit: u64) -> Result<Self> {
        let root = match explicit {
            Some(root) => Some(root.canonicalize().context("opening explicit addon root")?),
            None => match data.canonicalize()?.parent() {
                Some(install) => child_dir(install, "Interface")?
                    .map(|p| child_dir(&p, "AddOns"))
                    .transpose()?
                    .flatten(),
                None => None,
            },
        };
        let mut files = HashMap::new();
        if let Some(root) = root.as_ref() {
            let mut addon_names = std::collections::HashSet::new();
            for entry in std::fs::read_dir(root).context("listing addon root")? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue; // includes symlink directories: never follow them
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("addon folder name is not UTF-8"))?;
                let wanted = format!("{name}.toc");
                let mut has_toc = false;
                for file in std::fs::read_dir(entry.path())? {
                    let file = file?;
                    if file.file_type()?.is_file()
                        && file
                            .file_name()
                            .to_str()
                            .is_some_and(|s| s.eq_ignore_ascii_case(&wanted))
                    {
                        has_toc = true;
                        break;
                    }
                }
                if has_toc {
                    if name.contains('\\') || !addon_names.insert(name.to_ascii_lowercase()) {
                        bail!("invalid or ambiguous case-insensitive addon folder name");
                    }
                    scan(
                        &entry.path(),
                        &format!("Interface/AddOns/{name}"),
                        0,
                        &mut files,
                    )?;
                }
            }
        }
        let mut names: Vec<_> = files.values().map(|f| f.logical.clone()).collect();
        names.sort();
        // Versioned textual file catalog. It exposes logical names only; the
        // client will use its existing TOC parser and original registration walk.
        let mut bytes = b"BADDONS1\n".to_vec();
        for name in names {
            if bytes.len().saturating_add(name.len()).saturating_add(1) as u64 > limit {
                bail!("addon catalog exceeds gateway metadata limit");
            }
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(b'\n');
        }
        let catalog = CachedAsset {
            etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
            bytes: Arc::new(bytes),
            archive: "loose-addon-catalog".into(),
        };
        println!("Loose addon catalog: {} files", files.len());
        Ok(Self {
            root: root.unwrap_or_default(),
            files,
            catalog,
        })
    }

    pub(crate) fn size(&self, logical: &str) -> Option<u64> {
        self.files.get(logical).map(|f| f.size)
    }

    pub(crate) fn read(&self, logical: &str, limit: u64) -> Result<Vec<u8>> {
        let entry = self
            .files
            .get(logical)
            .context("loose addon file not indexed")?;
        // Recheck confinement at read time, not just during startup discovery.
        let path = entry
            .path
            .canonicalize()
            .context("resolving loose addon file")?;
        if !path.starts_with(&self.root) {
            bail!("loose addon file escapes configured root");
        }
        let file = File::open(&path).context("opening loose addon file")?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > limit {
            bail!("loose addon file is not regular or exceeds gateway limit");
        }
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            bail!("loose addon file grew beyond gateway limit");
        }
        Ok(bytes)
    }
}

fn child_dir(root: &Path, wanted: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(wanted))
        {
            if found.is_some() {
                bail!("ambiguous case-insensitive directory {wanted}");
            }
            found = Some(entry.path().canonicalize()?);
        }
    }
    Ok(found)
}

fn scan(
    dir: &Path,
    prefix: &str,
    depth: usize,
    files: &mut HashMap<String, LooseFile>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("loose addon directory nesting exceeds limit");
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("addon file name is not UTF-8"))?;
        if name.contains('\\') {
            bail!("backslash in loose addon filesystem name");
        }
        let authored = format!("{prefix}/{name}");
        let logical = normalize_client_path(&authored)?;
        if !logical.starts_with(PREFIX) || logical.contains(['\r', '\n']) {
            bail!("invalid loose addon logical name");
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            scan(&entry.path(), &authored, depth + 1, files)?;
        } else if kind.is_file() {
            if files.len() >= MAX_FILES {
                bail!("loose addon file count exceeds limit");
            }
            if files
                .insert(
                    logical.clone(),
                    LooseFile {
                        logical: authored,
                        path: entry.path(),
                        size: entry.metadata()?.len(),
                    },
                )
                .is_some()
            {
                bail!("ambiguous case-insensitive addon file {logical}");
            }
        }
    }
    Ok(())
}

//! Read-only probing of original vanilla archive sources at gateway startup.
//! Each admitted language owns an immutable base-plus-locale patch chain.

use super::*;
use benilla_formats::locales::{LOCALE_CODES, LocaleCatalog};

pub(crate) const ROUTE: &str = "/assets/v1/locales.bli";
pub(crate) struct LocaleAssets {
    pub chain: Chain,
    pub presence: CachedAsset,
    pub macro_icons: CachedAsset,
}

pub(crate) struct Locales {
    pub catalog: CachedAsset,
    pub assets: HashMap<String, LocaleAssets>,
}

fn assets(chain: Chain, limit: u64) -> Result<LocaleAssets> {
    let presence = presence::build(&chain, limit)?;
    let bytes = benilla_formats::load_macro_icons(&chain)?
        .join("\n")
        .into_bytes();
    let macro_icons = CachedAsset {
        etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
        bytes: Arc::new(bytes),
        archive: "localized-macro-icons".into(),
    };
    Ok(LocaleAssets {
        chain,
        presence,
        macro_icons,
    })
}

fn compatible_dbcs(base: &Chain, overlay: &Chain, limit: u64) -> Result<()> {
    for entry in overlay
        .list()?
        .into_iter()
        .filter(|e| e.name.to_ascii_lowercase().ends_with(".dbc"))
    {
        let read = |chain: &Chain| -> Result<Vec<u8>> {
            let size = chain
                .file_size(&entry.name)
                .context("DBC not in base/locale chain")?;
            if size > limit || size < 20 {
                bail!("DBC exceeds limit or has no header: {}", entry.name);
            }
            chain.read(&entry.name)
        };
        let old = read(base)?;
        let new = read(overlay)?;
        // Record counts and string block size can differ in authored patches;
        // the original parser requires the vanilla field count and record width.
        if &old[..4] != b"WDBC" || &new[..4] != b"WDBC" || old[8..16] != new[8..16] {
            bail!("incompatible locale DBC schema: {}", entry.name);
        }
    }
    Ok(())
}
const PROBES: &[&str] = &[
    "Interface/FrameXML/GlobalStrings.lua",
    "Interface/GlueXML/GlueStrings.lua",
];

fn probe(chain: &Chain, limit: u64) -> Result<()> {
    for path in PROBES {
        let size = chain
            .file_size(path)
            .with_context(|| format!("missing {path}"))?;
        if size == 0 || size > limit.min(8 * 1024 * 1024) {
            bail!("locale probe exceeds limit or is empty: {path} ({size} bytes)");
        }
        let bytes = chain.read(path)?;
        if bytes.is_empty() || bytes.len() as u64 != size {
            bail!("invalid locale probe bytes: {path}");
        }
    }
    Ok(())
}

pub(crate) fn build(data: &Path, base: &Chain, limit: u64) -> Result<Locales> {
    probe(base, limit).context("probing current enUS base UI data")?;
    let mut codes = vec!["enUS".to_owned()];
    let mut localized = HashMap::new();
    localized.insert("enUS".into(), assets(base.clone(), limit)?);
    // Only canonical, immediate locale directories. No recursive scans, loose UI
    // overrides or followed directory symlinks; the original Chain owns MPQ order.
    let mut names = Vec::new();
    for (count, entry) in std::fs::read_dir(data)?.enumerate() {
        if count >= 1024 {
            bail!("locale directory discovery exceeds entry limit");
        }
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(code) = LOCALE_CODES
            .iter()
            .find(|code| code.eq_ignore_ascii_case(&name))
        {
            names.push(((*code).to_owned(), entry.path()));
        }
    }
    names.sort_by(|a, b| a.0.cmp(&b.0));
    for (code, path) in names {
        if codes.contains(&code) {
            continue;
        }
        let result = Chain::open(&path).and_then(|chain| {
            probe(&chain, limit)?;
            compatible_dbcs(base, &chain, limit)?;
            assets(base.with_overlay(chain), limit)
        });
        match result {
            Ok(language) => {
                println!("locale candidate: {code} (readable original UI string archives)");
                localized.insert(code.clone(), language);
                codes.push(code);
            }
            Err(error) => eprintln!("locale candidate {code} not admitted: {error:#}"),
        }
    }
    let bytes = format!("BLOCALES1\n{}\n", codes.join("\n")).into_bytes();
    LocaleCatalog::decode(&bytes)?;
    Ok(Locales {
        assets: localized,
        catalog: CachedAsset {
            etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
            bytes: Arc::new(bytes),
            archive: "locale-candidates".into(),
        },
    })
}

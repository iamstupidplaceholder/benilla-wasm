//! Host-discovered locale candidates, not a claim of complete client translation.

use anyhow::{Context, Result, bail};

pub const LOCALE_CODES: &[&str] = &[
    "enUS", "enGB", "deDE", "frFR", "esES", "esMX", "ruRU", "koKR", "zhCN", "zhTW",
];

pub fn locale_label(code: &str) -> &str {
    match code {
        "enUS" => "English (US)",
        "enGB" => "English (UK)",
        "deDE" => "German",
        "frFR" => "French",
        "esES" => "Spanish (Spain)",
        "esMX" => "Spanish (Mexico)",
        "ruRU" => "Russian",
        "koKR" => "Korean",
        "zhCN" => "Chinese (Simplified)",
        "zhTW" => "Chinese (Traditional)",
        _ => code,
    }
}

/// The first row names today's base chain; additional rows are readable locale
/// overlay candidates. No filesystem paths or executable text cross this boundary.
#[derive(Clone, Debug)]
pub struct LocaleCatalog {
    pub codes: Vec<String>,
}

impl LocaleCatalog {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 {
            bail!("locale catalog exceeds size limit");
        }
        let text = std::str::from_utf8(bytes)
            .context("locale catalog is not UTF-8")?
            .strip_prefix("BLOCALES1\n")
            .context("invalid locale catalog version")?;
        if !text.ends_with('\n') {
            bail!("truncated locale catalog");
        }
        let mut codes = Vec::new();
        for code in text.lines() {
            if !LOCALE_CODES.contains(&code) || codes.iter().any(|saved| saved == code) {
                bail!("invalid or duplicate locale code: {code}");
            }
            codes.push(code.to_owned());
        }
        if codes.first().map(String::as_str) != Some("enUS") {
            bail!("locale catalog does not identify the current enUS base chain");
        }
        Ok(Self { codes })
    }
}

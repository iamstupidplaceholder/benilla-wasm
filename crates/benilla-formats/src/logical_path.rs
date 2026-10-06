//! Validation and canonicalisation for paths inside the WoW virtual filesystem.
//!
//! MPQ lookup is case-insensitive and treats both slash spellings alike. The HTTP asset gateway
//! additionally needs to ensure that a client path can never escape into the host filesystem.

use anyhow::{bail, Result};

/// Validate a client-authored MPQ path and return its one cache/URL identity.
///
/// The result is ASCII-lowercase with forward slashes. Absolute paths, empty components,
/// traversal, drive prefixes and control/non-ASCII bytes are rejected before either the browser
/// cache or the gateway sees them.
pub fn normalize_client_path(raw: &str) -> Result<String> {
    if raw.is_empty() {
        bail!("empty client path");
    }
    if !raw.is_ascii() || raw.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("client path must contain printable ASCII only");
    }

    let forward = raw.replace('\\', "/");
    if forward.starts_with('/') || forward.contains(':') {
        bail!("absolute paths and drive prefixes are not allowed");
    }
    if forward
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("empty and traversal path components are not allowed");
    }

    Ok(forward.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_mpq_spelling() {
        assert_eq!(
            normalize_client_path("Interface\\Icons\\INV Misc.blp").unwrap(),
            "interface/icons/inv misc.blp"
        );
    }

    #[test]
    fn rejects_host_paths_and_traversal() {
        for bad in [
            "",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "interface/../realmlist.wtf",
            "interface//icons/foo.blp",
            "interface/./icons/foo.blp",
            "interface/é.blp",
        ] {
            assert!(normalize_client_path(bad).is_err(), "accepted {bad:?}");
        }
    }
}

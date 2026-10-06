use std::{
    io,
    path::{Component, Path},
};

pub(crate) fn browser_key(path: &Path) -> io::Result<String> {
    let mut components = path.components();
    if components.next() != Some(Component::Normal(crate::STATE_DIR.as_ref())) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state path outside benilla-config",
        ));
    }
    let mut parts = Vec::new();
    for component in components {
        let Component::Normal(part) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid state path component",
            ));
        };
        parts.push(
            part.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 state path")
            })?,
        );
    }
    if parts.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state path names a directory",
        ));
    }
    Ok(format!("benilla:state:v1:{}", parts.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserve_scope_and_reject_non_state_paths() {
        assert_eq!(
            browser_key(Path::new("benilla-config/camera/Realm-Name.txt")).unwrap(),
            "benilla:state:v1:camera/Realm-Name.txt"
        );
        assert_ne!(
            browser_key(Path::new("benilla-config/macros/account.txt")).unwrap(),
            browser_key(Path::new("benilla-config/macros/Realm-Name.txt")).unwrap()
        );
        for path in [
            "/benilla-config/config.toml",
            "benilla-config/../outside",
            "DBFilesClient/Map.dbc",
            "benilla-config",
        ] {
            assert!(browser_key(Path::new(path)).is_err(), "{path}");
        }
    }
}

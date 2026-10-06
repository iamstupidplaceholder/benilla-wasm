use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn home(dev_source_dir: Option<&Path>) -> Option<PathBuf> {
    if std::env::var_os("WOW_CAPTURE").is_some() {
        return None;
    }
    if let Some(root) = std::env::var_os("BENILLA_HOME") {
        return Some(root.into());
    }
    if let Some(root) = dev_source_dir.and_then(dev_project_root) {
        return Some(root.join(crate::STATE_DIR));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .map(|dir| dir.join(crate::STATE_DIR))
}

// Preserve the primary-checkout rule: linked worktrees share their native state.
fn dev_project_root(source: &Path) -> Option<PathBuf> {
    let here = source.ancestors().nth(2)?;
    let dot_git = here.join(".git");
    if dot_git.is_file() {
        if let Some(gitdir) = std::fs::read_to_string(&dot_git).ok().and_then(|text| {
            text.trim()
                .strip_prefix("gitdir:")
                .map(|path| path.trim().to_owned())
        }) {
            if let Some(primary) = Path::new(&gitdir)
                .ancestors()
                .nth(2)
                .and_then(|dir| dir.parent())
            {
                if primary.is_dir() {
                    return Some(primary.to_path_buf());
                }
            }
        }
    }
    Some(here.to_path_buf())
}

pub(super) fn read_to_string(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

pub(super) fn read(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

pub(super) fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    use io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip_replace_and_delete() {
        let root = std::env::temp_dir().join(format!("benilla-settings-{}", std::process::id()));
        let path = root.join("camera/Realm-Name.txt");
        assert_eq!(
            read_to_string(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        write_atomic(&path, "first\n").unwrap();
        assert_eq!(read_to_string(&path).unwrap(), "first\n");
        write_atomic(&path, "second\n").unwrap();
        assert_eq!(read_to_string(&path).unwrap(), "second\n");
        assert!(!path.with_extension("tmp").exists());
        remove_file(&path).unwrap();
        assert_eq!(
            remove_file(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn linked_worktree_resolves_to_primary_checkout() {
        let root =
            std::env::temp_dir().join(format!("benilla-settings-worktree-{}", std::process::id()));
        let primary = root.join("primary");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(primary.join(".git/worktrees/slot")).unwrap();
        std::fs::create_dir_all(worktree.join("crates/benilla-app")).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                primary.join(".git/worktrees/slot").display()
            ),
        )
        .unwrap();
        assert_eq!(
            dev_project_root(&worktree.join("crates/benilla-app")),
            Some(primary.clone())
        );
        std::fs::remove_file(worktree.join(".git")).unwrap();
        assert_eq!(
            dev_project_root(&worktree.join("crates/benilla-app")),
            Some(worktree.clone())
        );
        for dir in [
            worktree.join("crates/benilla-app"),
            worktree.join("crates"),
            worktree,
            primary.join(".git/worktrees/slot"),
            primary.join(".git/worktrees"),
            primary.join(".git"),
            primary,
            root,
        ] {
            std::fs::remove_dir(dir).unwrap();
        }
    }
}

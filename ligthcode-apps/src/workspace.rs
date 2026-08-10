use std::path::{Path, PathBuf};

/// Resolve the workspace identity for a working directory: the Git repository
/// root when inside one, otherwise the canonicalized directory.
pub fn resolve(cwd: &Path) -> PathBuf {
    if let Some(root) = git_root(cwd) {
        return root;
    }
    normalize(cwd)
}

/// Canonicalize a path, resolving symlinks, `.` and `..`. Falls back to the
/// raw path when canonicalization fails (e.g. the dir does not exist yet).
pub fn normalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Walk up from `start` looking for a `.git` entry (dir or worktree file).
pub fn git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(normalize(start));
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return Some(d);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

/// A stable, filesystem-independent id for a workspace path (FNV-1a → hex).
/// Used as the session storage directory name.
pub fn workspace_id(workspace: &Path) -> String {
    let s = workspace.to_string_lossy();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn git_root_detected_from_nested_dir() {
        let d = std::env::temp_dir().join(format!("lc_ws_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("src").join("auth")).unwrap();
        fs::create_dir_all(d.join(".git")).unwrap();
        assert_eq!(
            git_root(&d.join("src").join("auth")).unwrap(),
            std::fs::canonicalize(&d).unwrap()
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn non_git_project_uses_canonical_dir() {
        let d = std::env::temp_dir().join(format!("lc_ws_ng_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let ws = resolve(&d);
        assert_eq!(ws, std::fs::canonicalize(&d).unwrap());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn resolve_returns_git_root_for_nested_cwd() {
        let d = std::env::temp_dir().join(format!("lc_ws_r_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::create_dir_all(d.join("src")).unwrap();
        let ws = resolve(&d.join("src"));
        assert_eq!(ws, std::fs::canonicalize(&d).unwrap());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn workspace_id_is_stable_and_distinct() {
        let a = Path::new("/tmp/project-a");
        let b = Path::new("/tmp/project-b");
        let a1 = workspace_id(a);
        let a2 = workspace_id(a);
        assert_eq!(a1, a2);
        assert_ne!(a1, workspace_id(b));
    }
}

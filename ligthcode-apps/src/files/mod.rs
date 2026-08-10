use ignore::WalkBuilder;
use std::path::Path;

/// Directories never shown in the picker, even without a .gitignore entry.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "coverage",
    ".cache",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    ".terraform",
    "vendor",
];

/// One result in the mention picker: a normalized `/`-separated relative path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub path: String,
    pub is_dir: bool,
}

/// A cached index of repository files and directories, respecting .gitignore.
pub struct FileIndex {
    files: Vec<String>,
    dirs: Vec<String>,
}

impl FileIndex {
    /// Walk the repository once and index every non-ignored file/directory.
    pub fn build(root: &Path) -> Self {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let mut walk = WalkBuilder::new(root);
        walk.require_git(false) // respect .gitignore even outside a git checkout
            .hidden(true)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                !(is_dir && SKIP_DIRS.contains(&name.as_ref()))
            });
        for entry in walk.build() {
            let Ok(e) = entry else { continue };
            let Ok(rel) = e.path().strip_prefix(root) else {
                continue;
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let s = rel.to_string_lossy().replace('\\', "/");
            let ft = e.file_type();
            if ft.is_some_and(|t| t.is_dir()) {
                dirs.push(s);
            } else if ft.is_some_and(|t| t.is_file()) {
                files.push(s);
            }
        }
        files.sort();
        dirs.sort();
        Self { files, dirs }
    }

    /// Query the index. An empty query lists top-level entries; a query whose
    /// directory part already exists navigates inside it; otherwise prefix and
    /// fuzzy matches over the whole repository.
    pub fn query(&self, q: &str, limit: usize) -> Vec<Match> {
        let q = q.trim_start_matches('/');
        if q.is_empty() {
            return self.top_level(limit);
        }
        if let Some((dir, last)) = q.rsplit_once('/') {
            let dir = dir.trim_end_matches('/');
            if !dir.is_empty() && self.dir_exists(dir) {
                return self.under(&format!("{dir}/"), last, limit);
            }
        }
        self.match_all(q, limit)
    }

    fn dir_exists(&self, dir: &str) -> bool {
        self.dirs.iter().any(|d| d == dir)
            || self.files.iter().any(|f| f.starts_with(&format!("{dir}/")))
    }

    fn top_level(&self, limit: usize) -> Vec<Match> {
        let mut out: Vec<Match> = self
            .dirs
            .iter()
            .filter(|d| !d.contains('/'))
            .map(|d| Match {
                path: d.clone(),
                is_dir: true,
            })
            .chain(
                self.files
                    .iter()
                    .filter(|f| !f.contains('/'))
                    .map(|f| Match {
                        path: f.clone(),
                        is_dir: false,
                    }),
            )
            .collect();
        out.sort();
        out.truncate(limit);
        out
    }

    fn under(&self, prefix: &str, last: &str, limit: usize) -> Vec<Match> {
        let last_l = last.to_lowercase();
        let mut out: Vec<Match> = Vec::new();
        let include = |rest: &str| -> bool {
            let rest_l = rest.to_lowercase();
            last_l.is_empty() || rest_l.starts_with(&last_l) || fuzzy_match(&last_l, &rest_l)
        };
        for d in self.dirs.iter() {
            if let Some(rest) = d.strip_prefix(prefix) {
                if !rest.is_empty() && include(rest) {
                    out.push(Match {
                        path: d.clone(),
                        is_dir: true,
                    });
                }
            }
        }
        for f in self.files.iter() {
            if let Some(rest) = f.strip_prefix(prefix) {
                if include(rest) {
                    out.push(Match {
                        path: f.clone(),
                        is_dir: false,
                    });
                }
            }
        }
        out.sort();
        out.truncate(limit);
        out
    }

    fn match_all(&self, q: &str, limit: usize) -> Vec<Match> {
        let ql = q.to_lowercase();
        let mut scored: Vec<(u8, Match)> = Vec::new();
        let mut add = |path: &String, is_dir: bool| {
            let sl = path.to_lowercase();
            let score = if sl.starts_with(&ql) {
                0
            } else if sl.contains(&ql) {
                1
            } else if fuzzy_match(&ql, &sl) {
                2
            } else {
                return;
            };
            scored.push((
                score,
                Match {
                    path: path.clone(),
                    is_dir,
                },
            ));
        };
        for d in self.dirs.iter() {
            add(d, true);
        }
        for f in self.files.iter() {
            add(f, false);
        }
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.path.cmp(&b.1.path)));
        scored.into_iter().take(limit).map(|(_, m)| m).collect()
    }
}

/// Subsequence match: `authsvc` matches `src/auth/auth.service.ts`.
pub fn fuzzy_match(query: &str, candidate: &str) -> bool {
    let mut chars = query.chars();
    let mut next = chars.next();
    for c in candidate.chars() {
        if let Some(qc) = next {
            if c == qc {
                next = chars.next();
            }
        } else {
            break;
        }
    }
    next.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(root: &Path) {
        for d in [
            "src/auth",
            "src/payment",
            "src/users",
            "node_modules/x",
            "target/debug",
            "tests",
        ] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "src/main.rs",
            "src/auth/auth.service.ts",
            "src/auth/auth.service.spec.ts",
            "src/auth/auth.controller.ts",
            "src/auth/auth.guard.ts",
            "src/payment/pay.ts",
            "src/users/user.go",
            "tests/auth_test.rs",
            "package.json",
            "node_modules/x/lib.js",
            "target/debug/app",
        ] {
            fs::write(root.join(f), "").unwrap();
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_fidx_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn respects_ignore_and_skip_dirs() {
        let d = temp("ignore");
        fs::write(d.join(".gitignore"), "users/\n").unwrap();
        tree(&d);
        let idx = FileIndex::build(&d);
        let all: Vec<String> = idx.files.iter().chain(idx.dirs.iter()).cloned().collect();
        // node_modules/target are skipped explicitly.
        assert!(!all.iter().any(|p| p.contains("node_modules")));
        assert!(!all.iter().any(|p| p.contains("target/")));
        // .gitignore (users/) respected.
        assert!(!all.iter().any(|p| p.contains("users/")));
        // Real files present.
        assert!(all.iter().any(|p| p == "src/auth/auth.service.ts"));
        assert!(all.len() >= 8);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn prefix_and_fuzzy_search() {
        let d = temp("search");
        tree(&d);
        let idx = FileIndex::build(&d);
        let prefix = idx.query("auth.ser", 10);
        assert!(prefix.iter().any(|m| m.path == "src/auth/auth.service.ts"));
        // fuzzy: authsvc
        let fuzzy = idx.query("authsvc", 10);
        assert!(fuzzy.iter().any(|m| m.path == "src/auth/auth.service.ts"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn directory_navigation() {
        let d = temp("dirnav");
        tree(&d);
        let idx = FileIndex::build(&d);
        let kids = idx.query("src/auth/", 20);
        assert!(kids
            .iter()
            .any(|m| m.path == "src/auth/auth.service.ts" && !m.is_dir));
        assert!(kids.iter().any(|m| m.path == "src/auth/auth.guard.ts"));
        // Partial directory query.
        let kids = idx.query("src/auth/gu", 20);
        assert!(kids.iter().any(|m| m.path == "src/auth/auth.guard.ts"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn empty_query_lists_top_level() {
        let d = temp("top");
        tree(&d);
        let idx = FileIndex::build(&d);
        let top = idx.query("", 50);
        assert!(top.iter().any(|m| m.path == "src" && m.is_dir));
        assert!(top.iter().any(|m| m.path == "package.json" && !m.is_dir));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn fuzzy_subsequence() {
        assert!(fuzzy_match("authsvc", "src/auth/auth.service.ts"));
        assert!(fuzzy_match("main", "src/main.rs"));
        assert!(!fuzzy_match("zzz", "src/main.rs"));
    }
}

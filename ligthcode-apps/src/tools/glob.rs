use super::{bound, try_run, Tool, ToolDef, MAX_GREP_MATCHES};
use ignore::gitignore::GitignoreBuilder;
use ignore::WalkBuilder;
use serde_json::{json, Value};

pub struct Glob;

#[async_trait::async_trait]
impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "glob".into(),
            description: "Find files by glob pattern (e.g. **/*.rs, src/**/mod.rs), respecting .gitignore. Returns up to 200 matching paths relative to the search root.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Glob pattern to match against file paths"},
                    "path": {"type": "string", "description": "Directory to search in (default: .)"}
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or("glob: missing 'pattern' argument")?;
        let root = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        if pattern.is_empty() {
            return Err("glob: pattern must not be empty".to_string());
        }

        // ripgrep --files is fast; fall back to the in-process walker when missing.
        let args = vec![
            "--files".to_string(),
            "--color".to_string(),
            "never".to_string(),
            "--no-require-git".to_string(),
            "-g".to_string(),
            pattern.to_string(),
            root.to_string(),
        ];
        if let Some(res) = try_run("rg", &args, root, 120).await {
            if res.timed_out {
                return Err(format!(
                    "glob: rg timed out searching '{pattern}' in {root}"
                ));
            }
            return match res.code {
                Some(0) => {
                    let mut paths: Vec<&str> = res.stdout.lines().take(MAX_GREP_MATCHES).collect();
                    let truncated = res.stdout.lines().count() > MAX_GREP_MATCHES;
                    if truncated {
                        paths.push("[truncated at MAX_GREP_MATCHES matches]");
                    }
                    if paths.is_empty() {
                        Ok(format!("glob: no files match '{pattern}' in {root}"))
                    } else {
                        Ok(bound(paths.join("\n")))
                    }
                }
                Some(1) => Ok(format!("glob: no files match '{pattern}' in {root}")),
                Some(c) => Err(format!("glob: rg failed (exit {c}): {}", res.stderr.trim())),
                _ => Err("glob: rg failed".to_string()),
            };
        }

        let pattern = pattern.to_string();
        let root = root.to_string();
        Ok(
            tokio::task::spawn_blocking(move || walker_glob(&pattern, &root))
                .await
                .unwrap_or_else(|_| "glob: search interrupted".to_string()),
        )
    }
}

/// In-process fallback glob (used when ripgrep is unavailable).
fn walker_glob(pattern: &str, root: &str) -> String {
    let mut builder = GitignoreBuilder::new(root);
    if builder.add_line(None, pattern).is_err() {
        return format!("glob: invalid pattern '{pattern}'");
    }
    let Ok(matcher) = builder.build() else {
        return format!("glob: invalid pattern '{pattern}'");
    };
    let mut out = Vec::new();
    for entry in WalkBuilder::new(root).require_git(false).build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        if is_dir {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > super::MAX_FILE_BYTES {
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => r,
            Err(_) => entry.path(),
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if matcher
            .matched_path_or_any_parents(entry.path(), is_dir)
            .is_ignore()
        {
            out.push(rel_str);
            if out.len() >= MAX_GREP_MATCHES {
                out.push(format!("[truncated at {MAX_GREP_MATCHES} matches]"));
                return bound(out.join("\n"));
            }
        }
    }
    if out.is_empty() {
        format!("glob: no files match '{pattern}' in {root}")
    } else {
        bound(out.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_glob_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn glob_finds_rust_files() {
        let d = temp_dir("rust");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/main.rs"), "").unwrap();
        std::fs::write(d.join("src/lib.rs"), "").unwrap();
        std::fs::write(d.join("README.md"), "").unwrap();
        let tool = Glob;
        let out = tool
            .execute(serde_json::json!({"pattern": "**/*.rs", "path": d}))
            .await
            .unwrap();
        assert!(out.contains("src/main.rs"), "got: {out}");
        assert!(out.contains("src/lib.rs"));
        assert!(!out.contains("README.md"));
        std::fs::remove_dir_all(&d).ok();
    }
}

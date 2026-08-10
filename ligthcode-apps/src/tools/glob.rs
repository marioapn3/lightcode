use super::{bound, Tool, ToolDef, MAX_GREP_MATCHES};
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

        // Reuse gitignore's glob matcher: a matched path reports Match::Ignore.
        let mut builder = GitignoreBuilder::new(root);
        builder
            .add_line(None, pattern)
            .map_err(|e| format!("glob: invalid pattern '{pattern}': {e}"))?;
        let matcher = builder
            .build()
            .map_err(|e| format!("glob: invalid pattern '{pattern}': {e}"))?;

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
                    return Ok(bound(out.join("\n")));
                }
            }
        }
        if out.is_empty() {
            Ok(format!("glob: no files match '{pattern}' in {root}"))
        } else {
            Ok(bound(out.join("\n")))
        }
    }
}

use super::{bound, try_run, Tool, ToolDef, MAX_FILE_BYTES, MAX_GREP_MATCHES};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};

pub struct Grep;

#[async_trait::async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "grep".into(),
            description:
                "Search files for a regex pattern, respecting .gitignore. Returns path:line:match."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Regex to search for"},
                    "path": {"type": "string", "description": "Directory or file to search (default: .)"}
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or("grep: missing 'pattern' argument")?;
        let root = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        // Validate the regex up front so both backends share the same error.
        Regex::new(pattern).map_err(|e| format!("grep: invalid regex '{pattern}': {e}"))?;

        // ripgrep is 10-50x faster than the in-process walker; prefer it and
        // fall back to the walker when rg is not installed.
        if !pattern.starts_with('-') {
            let args = vec![
                "--no-heading".to_string(),
                "--line-number".to_string(),
                "--color".to_string(),
                "never".to_string(),
                "--max-columns".to_string(),
                "200".to_string(),
                "--no-require-git".to_string(),
                pattern.to_string(),
                root.to_string(),
            ];
            if let Some(res) = try_run("rg", &args, root, 120).await {
                if res.timed_out {
                    return Err(format!(
                        "grep: rg timed out searching '{pattern}' in {root}"
                    ));
                }
                return match res.code {
                    Some(0) => Ok(format_rg_matches(&res.stdout)),
                    Some(1) => Ok(format!("grep: no matches for '{pattern}' in {root}")),
                    Some(c) => Err(format!("grep: rg failed (exit {c}): {}", res.stderr.trim())),
                    _ => Err("grep: rg failed".to_string()),
                };
            }
        }

        let pattern = pattern.to_string();
        let root = root.to_string();
        Ok(
            tokio::task::spawn_blocking(move || walker_grep(&pattern, &root))
                .await
                .unwrap_or_else(|_| "grep: search interrupted".to_string()),
        )
    }
}

/// Keep the first `MAX_GREP_MATCHES` `path:line:content` rows from rg.
fn format_rg_matches(stdout: &str) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for line in stdout.lines() {
        out.push_str(line);
        out.push('\n');
        count += 1;
        if count >= MAX_GREP_MATCHES {
            out.push_str(&format!("[truncated at {MAX_GREP_MATCHES} matches]\n"));
            return bound(out);
        }
    }
    bound(out)
}

/// In-process fallback search (used when ripgrep is unavailable).
fn walker_grep(pattern: &str, root: &str) -> String {
    let mut builder = WalkBuilder::new(root);
    builder.require_git(false);
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return format!("grep: invalid regex '{pattern}'"),
    };
    let mut out = String::new();
    let mut count = 0usize;
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for (i, line) in content.lines().enumerate() {
            if re.is_match(line) {
                let clipped: String = line.chars().take(200).collect();
                out.push_str(&format!("{}:{}:{clipped}\n", entry.path().display(), i + 1));
                count += 1;
                if count >= MAX_GREP_MATCHES {
                    out.push_str(&format!("[truncated at {MAX_GREP_MATCHES} matches]\n"));
                    return bound(out);
                }
            }
        }
    }
    if out.is_empty() {
        format!("grep: no matches for '{pattern}' in {root}")
    } else {
        bound(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_grep_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn grep_finds_matches_and_skips_ignored() {
        let d = temp_dir("hit");
        std::fs::write(d.join("a.rs"), "fn auth() {}\nfn other() {}\n").unwrap();
        std::fs::write(d.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(d.join("ignored.txt"), "auth secret\n").unwrap();
        let tool = Grep;
        let out = tool
            .execute(serde_json::json!({"pattern": "auth", "path": d}))
            .await
            .unwrap();
        assert!(out.contains("a.rs"), "match file present: {out}");
        assert!(
            !out.contains("ignored.txt"),
            "gitignored file excluded: {out}"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn grep_no_matches() {
        let d = temp_dir("miss");
        std::fs::write(d.join("a.rs"), "fn x() {}\n").unwrap();
        let tool = Grep;
        let out = tool
            .execute(serde_json::json!({"pattern": "zzz_none", "path": d}))
            .await
            .unwrap();
        assert!(out.contains("no matches"), "got: {out}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn grep_invalid_regex_errors() {
        let tool = Grep;
        let out = tool.execute(serde_json::json!({"pattern": "([" })).await;
        assert!(out.is_err(), "invalid regex must error");
    }
}

use super::{bound, Tool, ToolDef, MAX_FILE_BYTES, MAX_GREP_MATCHES};
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
        let re =
            Regex::new(pattern).map_err(|e| format!("grep: invalid regex '{pattern}': {e}"))?;

        let mut out = String::new();
        let mut count = 0usize;
        for entry in WalkBuilder::new(root).build() {
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
                        return Ok(out);
                    }
                }
            }
        }
        if out.is_empty() {
            Ok(format!("grep: no matches for '{pattern}' in {root}"))
        } else {
            Ok(bound(out))
        }
    }
}

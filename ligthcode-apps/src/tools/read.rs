use super::{bound, Tool, ToolDef, MAX_FILE_BYTES};
use serde_json::{json, Value};

pub struct ReadFile;

#[async_trait::async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "read_file".into(),
            description: "Read a text file from disk, returning its contents with line numbers. \
Optionally read a line range with `offset` (0-based line, default 0) and `limit` (max lines)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to read"},
                    "offset": {"type": "number", "description": "0-based line to start reading from (default 0)"},
                    "limit": {"type": "number", "description": "Max number of lines to read (default: all)"}
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("read_file: missing 'path' argument")?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("read_file {path}: {e}"))?;
        if meta.len() > MAX_FILE_BYTES {
            return Ok(format!(
                "read_file {path}: file is {} bytes (limit {MAX_FILE_BYTES}). Use grep for targeted search.",
                meta.len()
            ));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("read_file {path}: {e}"))?;
        if bytes.contains(&0) {
            return Err(format!(
                "read_file {path}: binary file (contains NUL bytes)"
            ));
        }
        let content =
            String::from_utf8(bytes).map_err(|_| format!("read_file {path}: not valid UTF-8"))?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = offset.min(all_lines.len());
        let end = match limit {
            Some(n) => (start + n).min(all_lines.len()),
            None => all_lines.len(),
        };
        let selected = &all_lines[start..end];
        if selected.is_empty() {
            return Ok(format!(
                "read_file {path}: no lines in range (file has {} lines)",
                all_lines.len()
            ));
        }
        let numbered = selected
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:>5} | {}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(bound(format!(
            "=== {path} (lines {}-{}) ===\n{numbered}",
            start + 1,
            end
        )))
    }
}

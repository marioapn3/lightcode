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
            description: "Read a text file from disk, returning its contents with line numbers."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to read"}
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
        let numbered = content
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{:>5} | {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(bound(format!("=== {path} ===\n{numbered}")))
    }
}

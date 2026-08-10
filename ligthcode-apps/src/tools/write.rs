use super::{Tool, ToolDef};
use serde_json::{json, Value};
use std::path::Path;

pub struct WriteFile;

#[async_trait::async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "write_file".into(),
            description: "Write content to a file, creating parent directories as needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path of the file to write"},
                    "content": {"type": "string", "description": "Full new content of the file"}
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("write_file: missing 'path' argument")?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("write_file: missing 'content' argument")?;

        let p = Path::new(path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("write_file {path}: create dirs: {e}"))?;
            }
        }
        tokio::fs::write(p, content)
            .await
            .map_err(|e| format!("write_file {path}: {e}"))?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }
}

use super::{bound, Tool, ToolDef};
use serde_json::{json, Value};

pub struct ListDirectory;

#[async_trait::async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_directory".into(),
            description: "List entries in a directory. Subdirectories are suffixed with '/'."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Directory to list (default: .)"}
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| format!("list_directory {path}: {e}"))?;
        let mut names = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| format!("list_directory {path}: {e}"))?
        {
            let ft = entry
                .file_type()
                .await
                .map_err(|e| format!("list_directory {path}: {e}"))?;
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                name.push('/');
            }
            names.push(name);
        }
        names.sort();
        Ok(bound(format!(
            "{} entries in {path}:\n{}",
            names.len(),
            names.join("\n")
        )))
    }
}

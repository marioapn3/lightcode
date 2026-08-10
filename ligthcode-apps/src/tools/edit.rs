use super::{Tool, ToolDef, MAX_FILE_BYTES};
use serde_json::{json, Value};

pub struct EditFile;

#[async_trait::async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "edit_file".into(),
            description: "Replace old_string with new_string in a file. Errors unless there is exactly one match (or replace_all is true).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path of the file to edit"},
                    "old_string": {"type": "string", "description": "Text to find"},
                    "new_string": {"type": "string", "description": "Replacement text"},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("edit_file: missing 'path' argument")?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or("edit_file: missing 'old_string' argument")?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or("edit_file: missing 'new_string' argument")?;
        if old_string.is_empty() {
            return Err("edit_file: old_string must not be empty".to_string());
        }

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("edit_file {path}: {e}"))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!(
                "edit_file {path}: file too large ({MAX_FILE_BYTES} byte limit)"
            ));
        }
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("edit_file {path}: {e}"))?;

        let occurrences = content.matches(old_string).count();
        if occurrences == 0 {
            return Err(format!("edit_file {path}: old_string not found"));
        }
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !replace_all && occurrences > 1 {
            return Err(format!(
                "edit_file {path}: found {occurrences} matches; set replace_all=true or make old_string unique"
            ));
        }
        let replaced = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        tokio::fs::write(path, replaced)
            .await
            .map_err(|e| format!("edit_file {path}: {e}"))?;

        let count = if replace_all { occurrences } else { 1 };
        Ok(format!(
            "edited {path}: replaced {count} occurrence(s)\n- {old_string}\n+ {new_string}"
        ))
    }
}

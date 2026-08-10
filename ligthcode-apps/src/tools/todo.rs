use super::{Tool, ToolDef};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct Todo {
    content: String,
    status: String,
    #[serde(default)]
    id: usize,
}

fn todos_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| ".".into())
        .join(".lightcode-todos.json")
}

fn write_todos(todos: &[Todo]) -> Result<String, String> {
    let mut out: Vec<Todo> = todos
        .iter()
        .enumerate()
        .map(|(i, t)| Todo {
            content: t.content.clone(),
            status: t.status.clone(),
            id: i,
        })
        .collect();
    let _ = &mut out;
    std::fs::write(
        todos_path(),
        serde_json::to_string_pretty(&out).unwrap_or_default(),
    )
    .map_err(|e| format!("todowrite: {e}"))?;
    Ok(format!(
        "{} todo item(s) saved to {}",
        out.len(),
        todos_path().display()
    ))
}

pub struct TodoWrite;

#[async_trait::async_trait]
impl Tool for TodoWrite {
    fn name(&self) -> &'static str {
        "todowrite"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "todowrite".into(),
            description: "Replace the working todo list with the given items and return the updated list. Each item: {content, status (\"pending\"|\"in_progress\"|\"completed\")}.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string"},
                                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let todos: Vec<Todo> = args
            .get("todos")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if todos.is_empty() {
            // Empty list clears the file.
            let _ = std::fs::remove_file(todos_path());
            return Ok("todo list cleared".to_string());
        }
        write_todos(&todos)
    }
}

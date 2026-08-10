use super::{bound, exec, Tool, ToolDef};
use serde_json::{json, Value};

/// Run a git command in the current directory, erroring on non-zero exit.
async fn git_run(args: &[String]) -> Result<String, String> {
    let mut full = vec!["--no-pager".to_string()];
    full.extend_from_slice(args);
    let res = exec::run("git", &full, ".", 30).await;
    if res.timed_out {
        return Err("git command timed out".to_string());
    }
    if res.code != Some(0) {
        let detail = if res.stderr.trim().is_empty() {
            format!("exit code {}", res.code.unwrap_or(-1))
        } else {
            res.stderr.trim().to_string()
        };
        return Err(format!("git {} failed: {detail}", args.join(" ")));
    }
    Ok(bound(res.stdout))
}

pub struct GitDiff;

#[async_trait::async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "git_diff".into(),
            description: "Show uncommitted working-tree changes (optionally for one path).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Restrict diff to this path (optional)"}
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let mut cmd = vec!["diff".to_string()];
        if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
            cmd.push("--".into());
            cmd.push(p.into());
        }
        git_run(&cmd).await
    }
}

pub struct GitStatus;

#[async_trait::async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "git_status".into(),
            description: "Show working-tree status in short format.".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String, String> {
        git_run(&["status".into(), "--short".into()]).await
    }
}

pub struct GitLog;

#[async_trait::async_trait]
impl Tool for GitLog {
    fn name(&self) -> &'static str {
        "git_log"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "git_log".into(),
            description: "Show recent commit history (one line per commit).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "n": {"type": "number", "description": "Number of commits (default 10)"}
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(10);
        git_run(&["log".into(), "--oneline".into(), "-n".into(), n.to_string()]).await
    }
}

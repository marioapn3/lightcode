use super::{bound, exec, Tool, ToolDef};
use serde_json::{json, Value};

pub struct Shell;

#[async_trait::async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shell".into(),
            description: "Run a shell command. Output is captured and bounded. Timeout applies."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command to run"},
                    "workdir": {"type": "string", "description": "Working directory (default: .)"},
                    "timeout_seconds": {"type": "number", "description": "Max seconds (default 120, max 600)"}
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("shell: missing 'command' argument")?;
        let workdir = args.get("workdir").and_then(|v| v.as_str()).unwrap_or(".");
        let timeout = args
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .min(600);

        let res = exec::run("sh", &["-c".into(), command.to_string()], workdir, timeout).await;

        let mut out = format!("$ {command}\n");
        if !res.stdout.is_empty() {
            out.push_str("--- stdout ---\n");
            out.push_str(&res.stdout);
            out.push('\n');
        }
        if !res.stderr.is_empty() {
            out.push_str("--- stderr ---\n");
            out.push_str(&res.stderr);
            out.push('\n');
        }
        if res.timed_out {
            out.push_str("(timed out and killed)\n");
        } else {
            out.push_str(&format!("exit code: {}\n", res.code.unwrap_or(-1)));
        }
        Ok(bound(out))
    }
}

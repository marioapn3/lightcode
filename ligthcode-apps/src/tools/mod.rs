pub mod edit;
pub mod exec;
pub mod git;
pub mod glob;
pub mod grep;
pub mod list;
pub mod patch;
pub mod read;
pub mod shell;
pub mod todo;
pub mod web;
pub mod write;

pub use crate::providers::ToolDef;
use serde_json::Value;

/// Run a command, returning `None` when the program could not be spawned
/// (e.g. ripgrep not installed) so callers can fall back gracefully.
async fn try_run(
    program: &str,
    args: &[String],
    workdir: &str,
    timeout_secs: u64,
) -> Option<exec::CmdResult> {
    let res = exec::run(program, args, workdir, timeout_secs).await;
    if res.code.is_none() && !res.timed_out {
        None
    } else {
        Some(res)
    }
}

/// Bound tool output so a huge file/result never floods the LLM context.
pub const MAX_TOOL_OUTPUT: usize = 32 * 1024;
pub const MAX_GREP_MATCHES: usize = 200;
/// Files larger than this are not read/grep'd in full.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn def(&self) -> ToolDef;
    async fn execute(&self, args: Value) -> Result<String, String>;
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
    defs: Vec<ToolDef>,
}

impl Registry {
    pub fn default() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(read::ReadFile),
            Box::new(grep::Grep),
            Box::new(list::ListDirectory),
            Box::new(write::WriteFile),
            Box::new(edit::EditFile),
            Box::new(shell::Shell),
            Box::new(git::GitDiff),
            Box::new(git::GitStatus),
            Box::new(git::GitLog),
            Box::new(glob::Glob),
            Box::new(patch::ApplyPatch),
            Box::new(todo::TodoWrite),
            Box::new(web::WebFetch),
            Box::new(web::WebSearch),
        ];
        let defs = tools.iter().map(|t| t.def()).collect();
        Self { tools, defs }
    }

    pub fn defs(&self) -> &[ToolDef] {
        &self.defs
    }

    /// Tool schema for the `task` sub-agent tool. The engine handles execution
    /// specially (it is not a regular tool).
    pub fn task_def() -> ToolDef {
        ToolDef {
            name: "task".into(),
            description: "Delegate a task to a sub-agent that can use the same tools. \
Pass a focused, self-contained instruction as `prompt`; returns the sub-agent's final answer."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "Short description of the subtask"},
                    "prompt": {"type": "string", "description": "Self-contained instruction for the sub-agent"},
                    "model": {"type": "string", "description": "Optional model override for the sub-agent"}
                },
                "required": ["description", "prompt"]
            }),
        }
    }

    /// Tool schema for the `question` tool. The engine routes it to the UI.
    pub fn question_def() -> ToolDef {
        ToolDef {
            name: "question".into(),
            description: "Ask the user a multiple-choice question. Use only when you need a \
decision only the user can make. Options are short labels."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "The question to ask"},
                    "options": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Short answer options (2-5)"
                    }
                },
                "required": ["prompt", "options"]
            }),
        }
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String, String> {
        match self.tools.iter().find(|t| t.name() == name) {
            Some(t) => t.execute(args).await,
            None => Err(format!("unknown tool: {name}")),
        }
    }
}

/// Truncate output at a UTF-8 boundary, noting the full size.
pub fn bound(s: String) -> String {
    if s.len() <= MAX_TOOL_OUTPUT {
        return s;
    }
    let mut end = MAX_TOOL_OUTPUT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[truncated: {} of {} bytes]", &s[..end], end, s.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_tools_{}_{}", std::process::id(), name));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn output_is_bounded() {
        let out = bound("x".repeat(100_000));
        assert!(out.len() < 40_000);
        assert!(out.contains("truncated"));
    }

    #[tokio::test]
    async fn read_file_lists_lines() {
        let d = temp_dir("read");
        let f = d.join("a.txt");
        fs::write(&f, "one\ntwo\n").unwrap();
        let out = read::ReadFile
            .execute(json!({"path": f.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("one"));
        assert!(out.contains("1 |"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn read_file_rejects_binary() {
        let d = temp_dir("bin");
        let f = d.join("b.bin");
        fs::write(&f, [0u8, 1, 2, 3]).unwrap();
        let err = read::ReadFile
            .execute(json!({"path": f.to_string_lossy()}))
            .await
            .unwrap_err();
        assert!(err.contains("binary"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn grep_finds_matches_with_line_numbers() {
        let d = temp_dir("grep");
        fs::write(d.join("a.rs"), "fn foo() {}\n").unwrap();
        fs::write(d.join("b.txt"), "no match here\n").unwrap();
        let out = grep::Grep
            .execute(json!({"pattern": "foo", "path": d.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("a.rs:1"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn list_directory_shows_entries() {
        let d = temp_dir("list");
        fs::write(d.join("x.txt"), "").unwrap();
        fs::create_dir(d.join("sub")).unwrap();
        let out = list::ListDirectory
            .execute(json!({"path": d.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("x.txt"));
        assert!(out.contains("sub/"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let reg = Registry::default();
        let err = reg.execute("nope", json!({})).await.unwrap_err();
        assert!(err.contains("unknown tool"));
    }

    #[tokio::test]
    async fn write_file_creates_file_with_dirs() {
        let d = temp_dir("write");
        let f = d.join("nested").join("x.txt");
        let out = write::WriteFile
            .execute(json!({"path": f.to_string_lossy(), "content": "hello"}))
            .await
            .unwrap();
        assert!(out.contains("wrote 5 bytes"));
        assert_eq!(fs::read_to_string(&f).unwrap(), "hello");
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_occurrence() {
        let d = temp_dir("edit1");
        let f = d.join("a.txt");
        fs::write(&f, "foo bar foo\n").unwrap();
        let out = edit::EditFile
            .execute(json!({"path": f.to_string_lossy(), "old_string": "bar", "new_string": "baz"}))
            .await
            .unwrap();
        assert!(out.contains("replaced 1 occurrence"));
        assert_eq!(fs::read_to_string(&f).unwrap(), "foo baz foo\n");
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn edit_file_errors_on_ambiguous_match() {
        let d = temp_dir("edit2");
        let f = d.join("a.txt");
        fs::write(&f, "aaa\naaa\n").unwrap();
        let err = edit::EditFile
            .execute(json!({"path": f.to_string_lossy(), "old_string": "aaa", "new_string": "bbb"}))
            .await
            .unwrap_err();
        assert!(err.contains("matches"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn shell_runs_command_and_reports_exit_code() {
        let out = shell::Shell
            .execute(json!({"command": "echo lightcode-ok"}))
            .await
            .unwrap();
        assert!(out.contains("lightcode-ok"));
        assert!(out.contains("exit code: 0"));
    }

    #[tokio::test]
    async fn shell_reports_failure() {
        let out = shell::Shell
            .execute(json!({"command": "exit 3"}))
            .await
            .unwrap();
        assert!(out.contains("exit code: 3"));
    }

    #[tokio::test]
    async fn glob_finds_matching_files() {
        let d = temp_dir("glob");
        fs::create_dir_all(d.join("src").join("nested")).unwrap();
        fs::write(d.join("src").join("main.rs"), "").unwrap();
        fs::write(d.join("src").join("nested").join("mod.rs"), "").unwrap();
        fs::write(d.join("src").join("main.py"), "").unwrap();
        fs::create_dir_all(d.join("target")).unwrap();
        fs::write(d.join("target").join("build.rs"), "").unwrap();
        fs::write(d.join(".gitignore"), "target/\n").unwrap();

        let out = glob::Glob
            .execute(json!({"pattern": "**/*.rs", "path": d.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("src/main.rs"));
        assert!(out.contains("src/nested/mod.rs"));
        assert!(!out.contains("main.py"));
        // .gitignore of target/ is respected by WalkBuilder in the temp dir context
        assert!(!out.contains("target/build.rs"));
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_patch_updates_adds_deletes() {
        let d = temp_dir("patch");
        let a = d.join("a.txt");
        let b = d.join("b.txt");
        let del = d.join("del.txt");
        fs::write(&a, "one\ntwo\nthree\n").unwrap();
        fs::write(&b, "start\n").unwrap();
        fs::write(&del, "bye\n").unwrap();

        let patch_text = format!(
            "*** Begin Patch\n\
             *** Update File: {}\n\
             @@ -1,3 +1,3 @@\n\
              one\n\
            -two\n\
            +TWO\n\
              three\n\
             *** Add File: {}\n\
             +new file line 1\n\
             +new file line 2\n\
             *** Delete File: {}\n\
             *** End Patch",
            a.display(),
            b.display(),
            del.display()
        );
        let out = patch::ApplyPatch
            .execute(json!({"patch": patch_text}))
            .await
            .unwrap();
        assert!(out.contains("3 change"));
        assert_eq!(fs::read_to_string(&a).unwrap(), "one\nTWO\nthree\n");
        assert_eq!(
            fs::read_to_string(&b).unwrap(),
            "new file line 1\nnew file line 2\n"
        );
        assert!(!del.exists());
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_patch_rejects_mismatched_hunk() {
        let d = temp_dir("patch_bad");
        let a = d.join("a.txt");
        fs::write(&a, "xxx\n").unwrap();
        let patch_text = format!(
            "*** Begin Patch\n\
             *** Update File: {}\n\
             @@ -1,1 +1,1 @@\n\
            -nope\n\
             *** End Patch",
            a.display()
        );
        let err = patch::ApplyPatch
            .execute(json!({"patch": patch_text}))
            .await
            .unwrap_err();
        assert!(err.contains("does not fit"));
        assert_eq!(fs::read_to_string(&a).unwrap(), "xxx\n");
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_patch_moves_file() {
        let d = temp_dir("patch_move");
        let from = d.join("old.rs");
        let to = d.join("new.rs");
        fs::write(&from, "fn main() {}\n").unwrap();
        let patch_text = format!(
            "*** Begin Patch\n\
             *** Move File: {} → {}\n\
             *** End Patch",
            from.display(),
            to.display()
        );
        patch::ApplyPatch
            .execute(json!({"patch": patch_text}))
            .await
            .unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "fn main() {}\n");
        fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn todowrite_saves_list() {
        let d = temp_dir("todo");
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(&d).unwrap();
        let out = todo::TodoWrite
            .execute(json!({"todos": [
                {"content": "fix bug", "status": "in_progress"},
                {"content": "write tests", "status": "pending"}
            ]}))
            .await
            .unwrap();
        std::env::set_current_dir(orig).unwrap();
        assert!(out.contains("2 todo item(s)"));
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.join(".lightcode-todos.json")).unwrap())
                .unwrap();
        assert_eq!(saved[0]["content"], "fix bug");
        assert_eq!(saved[0]["id"], 0);
        fs::remove_dir_all(&d).ok();
    }
}

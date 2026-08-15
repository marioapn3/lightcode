pub mod policy;

use crate::providers::ToolCall;

pub use policy::Level;
pub use policy::Policy;

/// Categories of tool actions subject to the permission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Read,
    Write,
    Edit,
    Shell,
}

/// How the user answered a permission prompt.
#[derive(Debug)]
pub enum Choice {
    Allow,
    /// Deny, optionally with a message fed back to the model.
    Deny {
        feedback: Option<String>,
    },
    AllowForSession,
    /// Allow and remember this action across restarts (persisted per session).
    Always,
}

/// Which action a tool name maps to. Read-only tools (including git reads) are automatic.
pub fn action_for(tool_name: &str) -> Action {
    match tool_name {
        "write_file" => Action::Write,
        "edit_file" | "apply_patch" => Action::Edit,
        "shell" => Action::Shell,
        _ => Action::Read,
    }
}

pub fn action_name(a: Action) -> &'static str {
    match a {
        Action::Read => "read",
        Action::Write => "write",
        Action::Edit => "edit",
        Action::Shell => "shell",
    }
}

pub fn action_from_name(name: &str) -> Option<Action> {
    match name {
        "read" => Some(Action::Read),
        "write" => Some(Action::Write),
        "edit" => Some(Action::Edit),
        "shell" => Some(Action::Shell),
        _ => None,
    }
}

/// A stable, human-readable target for permission rule matching.
pub fn tool_target_for_policy(tc: &ToolCall) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| tc.arguments.get(*k).and_then(|v| v.as_str()))
            .unwrap_or("")
    };
    match tc.name.as_str() {
        "shell" => pick(&["command"]).to_string(),
        "read_file" | "write_file" | "edit_file" | "list_directory" | "glob" => {
            pick(&["path"]).to_string()
        }
        "grep" => pick(&["pattern"]).to_string(),
        "web_fetch" => pick(&["url"]).to_string(),
        "web_search" => pick(&["query"]).to_string(),
        "apply_patch" => "patch".to_string(),
        _ => String::new(),
    }
}

/// Human-readable description used in the permission prompt.
///
/// Shell commands are prefixed with `$` so the UI can highlight them.
pub fn describe_tool(tc: &ToolCall) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| tc.arguments.get(*k).and_then(|v| v.as_str()))
            .unwrap_or("")
    };
    match tc.name.as_str() {
        "shell" => format!("$ {}", pick(&["command"])),
        "write_file" => format!("Write file: {}", pick(&["path"])),
        "edit_file" => format!("Edit file: {}", pick(&["path"])),
        "apply_patch" => format!("Apply patch to: {}", pick(&["path"])),
        "read_file" => format!("Read file: {}", pick(&["path"])),
        "list_directory" => format!("List directory: {}", pick(&["path"])),
        "glob" => format!("Glob: {}", pick(&["pattern"])),
        "grep" => format!("Search: {}", pick(&["pattern"])),
        "web_fetch" => format!("Fetch: {}", pick(&["url"])),
        "web_search" => format!("Search web: {}", pick(&["query"])),
        _ => format!("Call {}({})", tc.name, pick(&["path", "target", "name"])),
    }
}

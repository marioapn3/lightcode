use super::Action;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Allow,
    #[default]
    Ask,
    Deny,
}

/// One wildcard rule: `{ permission, pattern, action }`. Rules are evaluated
/// in order and the last match wins, mirroring opencode's ruleset.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// "read" | "write" | "edit" | "shell" | "*"
    pub permission: String,
    /// Glob matched against the tool target (path, command, url, ...).
    pub pattern: String,
    #[serde(default)]
    pub action: Level,
}

/// Per-action permission levels from `[permissions]` config, plus an optional
/// wildcard ruleset. Read actions are always allowed by default.
#[derive(Debug, Clone, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub write: Level,
    #[serde(default)]
    pub edit: Level,
    #[serde(default)]
    pub shell: Level,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            write: Level::Ask,
            edit: Level::Ask,
            shell: Level::Ask,
            rules: Vec::new(),
        }
    }
}

impl Policy {
    pub fn level_for(&self, action: Action) -> Level {
        match action {
            Action::Read => Level::Allow,
            Action::Write => self.write,
            Action::Edit => self.edit,
            Action::Shell => self.shell,
        }
    }

    /// Resolve the effective level for an action applied to a target, honoring
    /// the wildcard ruleset (last match wins).
    pub fn level_for_target(&self, action: Action, target: &str) -> Level {
        let mut level = self.level_for(action);
        for rule in self.rules.iter().rev() {
            if (rule.permission == "*" || rule.permission == action_name(action))
                && glob_match(&rule.pattern, target)
            {
                level = rule.action;
                break;
            }
        }
        level
    }
}

fn action_name(a: Action) -> &'static str {
    match a {
        Action::Read => "read",
        Action::Write => "write",
        Action::Edit => "edit",
        Action::Shell => "shell",
    }
}

/// Simple glob matcher supporting `*` (any run within a segment), `**` (across
/// segments, including zero), and `?` (single char).
fn glob_match(pattern: &str, target: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = target.chars().collect();
    match_impl(&p, &t)
}

fn match_impl(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            if p.len() >= 2 && p[1] == '*' {
                // `**` matches across segments, including zero directories.
                if p.len() >= 3 && p[2] == '/' && match_impl(&p[3..], t) {
                    return true; // `**/` matches nothing
                }
                if p.len() == 2 && match_impl(&p[2..], t) {
                    return true; // trailing `**` matches nothing
                }
                if !t.is_empty() && match_impl(p, &t[1..]) {
                    return true;
                }
                false
            } else {
                // `*` matches any run within a single path segment.
                let mut skip = 0;
                while skip <= t.len() {
                    if skip > 0 && t[skip - 1] == '/' {
                        break;
                    }
                    if match_impl(&p[1..], &t[skip..]) {
                        return true;
                    }
                    skip += 1;
                }
                false
            }
        }
        '?' => !t.is_empty() && t[0] != '/' && match_impl(&p[1..], &t[1..]),
        c => !t.is_empty() && t[0] == c && match_impl(&p[1..], &t[1..]),
    }
}

/// Best-effort detection of destructive/high-impact shell commands.
/// These always require an explicit prompt even if the policy allows shell.
pub fn is_dangerous_command(command: &str) -> bool {
    let c = command.trim().to_lowercase();
    if ["sudo", "mkfs", "dd", "shutdown", "reboot", "poweroff"]
        .iter()
        .any(|p| c.starts_with(p))
    {
        return true;
    }
    if c.contains("rm -rf") || c.contains("rm -r -f") {
        return true;
    }
    if c.contains("git push") && c.contains("--force") {
        return true;
    }
    if (c.contains("curl") || c.contains("wget")) && (c.contains("| sh") || c.contains("| bash")) {
        return true;
    }
    if c.contains("chmod") && c.contains("777") && c.contains(" /") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::action_for;

    #[test]
    fn read_is_always_allowed() {
        let p = Policy {
            shell: Level::Deny,
            ..Policy::default()
        };
        assert_eq!(p.level_for(Action::Read), Level::Allow);
        assert_eq!(p.level_for(Action::Shell), Level::Deny);
    }

    #[test]
    fn defaults_to_ask() {
        let p = Policy::default();
        assert_eq!(p.level_for(Action::Edit), Level::Ask);
    }

    #[test]
    fn dangerous_command_detection() {
        assert!(is_dangerous_command("rm -rf /tmp/x"));
        assert!(is_dangerous_command("sudo apt install"));
        assert!(is_dangerous_command("curl http://x | sh"));
        assert!(is_dangerous_command("git push --force origin main"));
        assert!(!is_dangerous_command("cargo test"));
        assert!(!is_dangerous_command("rm target/debug -f"));
    }

    #[test]
    fn action_mapping() {
        assert_eq!(action_for("read_file"), Action::Read);
        assert_eq!(action_for("git_status"), Action::Read);
        assert_eq!(action_for("write_file"), Action::Write);
        assert_eq!(action_for("edit_file"), Action::Edit);
        assert_eq!(action_for("shell"), Action::Shell);
    }

    #[test]
    fn ruleset_last_match_wins() {
        let policy = Policy {
            write: Level::Allow,
            rules: vec![
                Rule {
                    permission: "write".into(),
                    pattern: "**/*.lock".into(),
                    action: Level::Deny,
                },
                Rule {
                    permission: "*".into(),
                    pattern: "**/vendor/**".into(),
                    action: Level::Deny,
                },
                Rule {
                    permission: "write".into(),
                    pattern: "src/**".into(),
                    action: Level::Allow,
                },
            ],
            ..Policy::default()
        };
        // Rule 1 denies lock files.
        assert_eq!(
            policy.level_for_target(Action::Write, "config/app.lock"),
            Level::Deny
        );
        // Rule 3 allows src/**, overrides the earlier lock rule for src.
        assert_eq!(
            policy.level_for_target(Action::Write, "src/app.lock"),
            Level::Allow
        );
        // vendor/** denied for any action via the "*" rule.
        assert_eq!(
            policy.level_for_target(Action::Edit, "vendor/lib.js"),
            Level::Deny
        );
        // No rule matches → base level.
        assert_eq!(
            policy.level_for_target(Action::Write, "other/file.rs"),
            Level::Allow
        );
    }

    #[test]
    fn glob_matcher() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("**/*.rs", "src/deep/main.rs"));
        assert!(glob_match("src/**", "src/a/b/c"));
        assert!(glob_match("src/?", "src/a"));
        assert!(!glob_match("*.rs", "main.txt"));
        assert!(!glob_match("src/*", "src/a/b"));
    }
}

pub mod context;
pub mod engine;

use crate::permissions::{Action, Choice, Policy};
use crate::providers::{Message, Provider};
use crate::session::Session;
use crate::tools::Registry;
use std::collections::HashMap;
use std::collections::HashSet;
use tokio::sync::{mpsc, oneshot, watch};

pub const MAX_ITERATIONS: usize = 50;
pub const COMPACTION_KEEP_TAIL: usize = 12;

/// The agent's execution mode. Modes affect real runtime behavior (tool
/// permissions + system prompt), not just the TUI label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    /// Read-only analysis: inspect, search, plan. No mutations.
    Plan,
    /// Standard coding mode with normal permissions.
    Build,
    /// Autonomous execution with minimal confirmation for routine actions.
    Auto,
}

impl AgentMode {
    pub fn label(&self) -> &'static str {
        match self {
            AgentMode::Plan => "PLAN",
            AgentMode::Build => "BUILD",
            AgentMode::Auto => "AUTO",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            AgentMode::Plan => "Analyze and create a plan",
            AgentMode::Build => "Implement changes",
            AgentMode::Auto => "Execute autonomously",
        }
    }

    pub fn next(&self) -> AgentMode {
        match self {
            AgentMode::Plan => AgentMode::Build,
            AgentMode::Build => AgentMode::Auto,
            AgentMode::Auto => AgentMode::Plan,
        }
    }

    pub fn from_str(s: &str) -> Option<AgentMode> {
        match s.trim().to_lowercase().as_str() {
            "plan" => Some(AgentMode::Plan),
            "build" => Some(AgentMode::Build),
            "auto" => Some(AgentMode::Auto),
            _ => None,
        }
    }

    /// Mode-specific system instructions, appended before each turn.
    pub fn instructions(&self) -> &'static str {
        match self {
            AgentMode::Plan => {
                "Analyze the repository and produce an implementation plan. \
Do not modify files. Use read-only tools only."
            }
            AgentMode::Build => {
                "Implement the user's request. Inspect the repository, make the \
necessary changes, and verify the implementation."
            }
            AgentMode::Auto => {
                "Complete the user's task autonomously. Inspect, implement, test, \
diagnose failures, and iterate. Avoid unnecessary confirmation requests. \
Respect all tool and safety restrictions."
            }
        }
    }
}

pub const SYSTEM_PROMPT: &str = r#"
You are LightCode, an autonomous coding agent working inside a software repository.

Your job is to understand the user's intent, inspect the codebase, make the necessary changes, and verify your work.

## Core Principles

- Inspect before acting. Never guess when the repository can provide the answer.
- Prefer existing patterns, abstractions, and conventions over introducing new ones.
- Make the smallest correct change that solves the problem.
- Preserve existing behavior unless the user explicitly asks to change it.
- Keep the codebase clean, consistent, and maintainable.
- Do not modify unrelated files or refactor unnecessarily.
- Treat user instructions as the source of truth, but use repository context to determine the correct implementation.

## Tool Usage

- Use repository tools to inspect files, search for symbols, trace dependencies, and understand context.
- Read relevant files before modifying them.
- Search for existing implementations before creating new ones.
- Use terminal/build/test tools when they can validate your changes.
- After making changes, verify that the implementation is correct whenever practical.
- If a tool fails, diagnose the failure and try a reasonable alternative.
- Never claim that something works unless you have sufficient evidence.

## Coding Behavior

- Follow the project's existing architecture, naming conventions, formatting, and style.
- Reuse existing utilities and abstractions when appropriate.
- Avoid speculative code and unnecessary complexity.
- Handle errors explicitly and consistently with the surrounding code.
- Consider edge cases and backwards compatibility when relevant.
- Do not silently weaken validation, security, or error handling just to make code pass.
- Avoid changing APIs, schemas, or public behavior unless required.

## Workflow

For non-trivial tasks:

1. Understand the request.
2. Inspect the relevant part of the repository.
3. Identify the root cause or correct implementation point.
4. Plan the minimal change.
5. Modify the code.
6. Run relevant tests, type checks, builds, or linters.
7. Review the resulting changes for unintended side effects.
8. Summarize what changed and how it was verified.

For simple tasks, avoid unnecessary exploration and act directly when the intent is clear.

## Communication

- Be concise and technical.
- Do not explain every tool call or internal step.
- Ask for clarification only when the request is genuinely ambiguous or requires a decision that cannot be inferred safely.
- When blocked, clearly state what is missing and why.
- At the end of a task, briefly report:
  - what changed,
  - important files affected,
  - and what verification was performed.

## Important

You are an agent operating on a real codebase. Optimize for correctness, minimal changes, and verifiable results—not for producing code as quickly as possible.
"#;

/// Events emitted by the agent to a UI consumer (TUI or simple stdout).
#[derive(Debug)]
pub enum AgentEvent {
    Text(String),
    Reasoning(String),
    ToolStart {
        name: String,
        args: String,
    },
    ToolOutput {
        name: String,
        output: String,
    },
    /// A filesystem change captured before/after a mutation tool, rendered as
    /// a dedicated diff block.
    Diff {
        file: String,
        body: String,
    },
    Permission {
        prompt: String,
        respond: oneshot::Sender<Choice>,
    },
    /// A `question` tool call: ask the user to pick from options.
    Question {
        prompt: String,
        options: Vec<String>,
        respond: oneshot::Sender<Option<String>>,
    },
    Done {
        ok: bool,
        message: String,
    },
    /// A manual compaction finished; `removed` is how many messages were folded.
    Compact {
        removed: usize,
    },
}

pub struct Agent {
    pub provider: Box<dyn Provider>,
    pub tools: Registry,
    pub history: Vec<Message>,
    pub verbose: bool,
    pub policy: Policy,
    pub prompter: Box<dyn FnMut(&str) -> Choice + Send>,
    pub display_stream: bool,
    pub session: Option<Session>,
    pub max_context_tokens: usize,
    pub max_iterations: usize,
    /// Configured named agents, applied via [`Self::set_agent`].
    pub agent_defs: HashMap<String, crate::config::AgentDef>,
    pub agent_name: String,
    /// All usable providers by name, for runtime provider switching.
    pub provider_map: HashMap<String, Box<dyn Provider>>,
    /// Repository root for @-mention context resolution.
    pub repo_root: Option<std::path::PathBuf>,
    pub file_index: Option<crate::files::FileIndex>,
    pub mode: AgentMode,
    /// True while streaming a turn if any text/tool output was emitted; used to
    /// retry a stream that drops before producing any output.
    pub stream_progress: bool,
    events: Option<mpsc::Sender<AgentEvent>>,
    cancel: Option<watch::Receiver<bool>>,
    session_allowed: HashSet<Action>,
}

impl Agent {
    pub fn new(
        provider: Box<dyn Provider>,
        tools: Registry,
        verbose: bool,
        policy: Policy,
        prompter: Box<dyn FnMut(&str) -> Choice + Send>,
    ) -> Self {
        let mut history = vec![Message::System {
            content: SYSTEM_PROMPT.to_string(),
        }];
        if let Some(text) = load_project_instructions() {
            history.push(Message::System {
                content: format!("Project instructions:\n{text}"),
            });
        }
        Self {
            provider,
            tools,
            history,
            verbose,
            policy,
            prompter,
            display_stream: false,
            session: None,
            max_context_tokens: 60_000,
            max_iterations: MAX_ITERATIONS,
            agent_defs: HashMap::new(),
            agent_name: "default".to_string(),
            provider_map: HashMap::new(),
            repo_root: None,
            file_index: None,
            mode: AgentMode::Build,
            stream_progress: false,
            events: None,
            cancel: None,
            session_allowed: HashSet::new(),
        }
    }

    pub fn set_display_stream(&mut self, on: bool) {
        self.display_stream = on;
    }

    pub fn set_session(&mut self, session: Option<Session>) {
        self.session = session;
    }

    pub fn set_max_context_tokens(&mut self, n: usize) {
        self.max_context_tokens = n;
    }

    pub fn set_max_iterations(&mut self, n: usize) {
        self.max_iterations = n;
    }

    /// Route events to a UI channel instead of direct terminal output.
    pub fn set_events(&mut self, tx: Option<mpsc::Sender<AgentEvent>>) {
        self.events = tx;
    }

    /// Watch channel that, when set to true, interrupts the running agent.
    pub fn set_cancel(&mut self, rx: Option<watch::Receiver<bool>>) {
        self.cancel = rx;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|r| *r.borrow())
    }

    /// Register all usable providers so the UI can switch provider at runtime.
    pub fn set_provider_map(&mut self, map: HashMap<String, Box<dyn Provider>>) {
        self.provider_map = map;
    }

    /// Switch the active model, swapping to another provider when needed.
    pub fn set_model_with_provider(&mut self, provider: &str, model: &str) {
        if let Some(p) = self.provider_map.get(provider) {
            let mut cloned = p.clone_box();
            cloned.set_model(model);
            self.provider = cloned;
        } else {
            self.provider.set_model(model);
        }
    }

    /// Seed the session-approved actions from a previously persisted "always" list.
    pub fn set_approved(&mut self, actions: &[String]) {
        for a in actions {
            if let Some(action) = crate::permissions::action_from_name(a) {
                self.session_allowed.insert(action);
            }
        }
    }

    /// Configure the named agents available to [`Self::set_agent`].
    pub fn set_agent_defs(&mut self, defs: HashMap<String, crate::config::AgentDef>) {
        self.agent_defs = defs;
    }

    /// Switch execution mode. Persisted to the session when one is attached.
    pub fn set_mode(&mut self, mode: AgentMode) {
        self.mode = mode;
        if let Some(s) = &self.session {
            let _ = s.save_mode(mode.label());
        }
    }

    /// Switch to a named agent ("default" restores the base system prompt).
    pub fn set_agent(&mut self, name: &str) {
        self.agent_name = name.to_string();
        let first_system = |history: &mut Vec<Message>, text: String| {
            if let Some(Message::System { content }) = history.first_mut() {
                *content = text;
            }
        };
        if name == "default" {
            first_system(&mut self.history, SYSTEM_PROMPT.to_string());
            return;
        }
        let Some(def) = self.agent_defs.get(name).cloned() else {
            return;
        };
        if let Some(m) = def.model {
            self.provider.set_model(&m);
        }
        if let Some(sp) = def.system_prompt {
            first_system(&mut self.history, sp);
        }
    }

    /// Manually compact the conversation history (the `/compact` command).
    pub async fn compact_manual(&mut self) -> anyhow::Result<usize> {
        let before = self.history.len();
        self.compact().await?;
        Ok(before.saturating_sub(self.history.len()))
    }

    fn push(&mut self, m: Message) {
        if let Some(s) = &self.session {
            let _ = s.append(&m);
        }
        self.history.push(m);
    }
}

/// Discover project instructions (AGENTS.md preferred, then LIGHTCODE.md) by
/// walking up from the working directory.
fn load_project_instructions() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    find_instructions_in(&cwd)
}

/// Instructions discovery rooted at a specific directory (testable).
fn find_instructions_in(start: &std::path::Path) -> Option<String> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        for name in ["AGENTS.md", "LIGHTCODE.md"] {
            let candidate = d.join(name);
            if candidate.is_file() {
                if let Ok(text) = std::fs::read_to_string(&candidate) {
                    return Some(text);
                }
            }
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_md_is_preferred_over_lightcode_md() {
        let tmp = std::env::temp_dir().join(format!("lightcode_instr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("AGENTS.md"), "agents content").unwrap();
        std::fs::write(tmp.join("LIGHTCODE.md"), "lightcode content").unwrap();

        let found = find_instructions_in(&tmp);
        assert_eq!(found.as_deref(), Some("agents content"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn new_agent_starts_with_system_prompt() {
        let agent = Agent::new(
            Box::new(crate::providers::openai::OpenAiProvider::new(
                Default::default(),
                "k".into(),
            )),
            Registry::default(),
            false,
            Policy::default(),
            Box::new(|_| Choice::Deny { feedback: None }),
        );
        assert!(matches!(agent.history[0], Message::System { .. }));
    }
}

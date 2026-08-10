use super::{context, Agent, AgentEvent, MAX_ITERATIONS};
use crate::permissions::{self, Action, Choice, Level};
use crate::providers::{Message, StreamEvent, ToolCall};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::io::Write;
use tokio::sync::{mpsc, oneshot, watch};

const SUMMARY_PROMPT: &str =
    "Summarize the earlier conversation below, preserving important facts, \
decisions, file paths, and any in-progress work. Be concise but complete.";

/// Result of the permission check for a tool call.
enum ToolApproval {
    Allowed,
    Denied { feedback: Option<String> },
}

/// Mutation tools that are always blocked in PLAN mode.
fn plan_blocks(name: &str) -> bool {
    matches!(
        name,
        "write_file" | "edit_file" | "apply_patch" | "shell" | "task" | "todowrite"
    )
}

impl ToolApproval {
    #[cfg(test)]
    fn matches_allowed(&self) -> bool {
        matches!(self, ToolApproval::Allowed)
    }
}

impl Agent {
    /// Run one user request through the iterative tool loop.
    /// Returns the final assistant text once the model stops calling tools.
    pub async fn run(&mut self, input: &str) -> Result<String> {
        if self.is_cancelled() {
            return Err(anyhow!("interrupted"));
        }
        // Resolve @-mentions into a system context block (files/dirs metadata).
        let root = self.repo_root.clone();
        if let Some(root) = root {
            if let Some(ctx) = self.resolve_mentions(input, &root) {
                self.push(Message::System { content: ctx });
            }
        }
        // Mode instructions shape the turn (centralized in AgentMode).
        self.push(Message::System {
            content: format!(
                "You are operating in {} mode.\n{}",
                self.mode.label(),
                self.mode.instructions()
            ),
        });
        self.push(Message::User {
            content: input.to_string(),
        });

        for _ in 0..MAX_ITERATIONS {
            if self.is_cancelled() {
                return Err(anyhow!("interrupted"));
            }
            if context::estimate_tokens(&self.history) > self.max_context_tokens {
                self.compact().await?;
            }
            let mut defs = self.tools.defs().to_vec();
            defs.push(crate::tools::Registry::task_def());
            defs.push(crate::tools::Registry::question_def());
            let history = self.history.clone();
            let (content, reasoning, tool_calls) =
                self.complete_with_retry(&history, &defs).await?;

            if tool_calls.is_empty() {
                let text = content.unwrap_or_default();
                self.push(Message::Assistant {
                    content: Some(text.clone()),
                    reasoning,
                    tool_calls: vec![],
                });
                return Ok(text);
            }

            self.push(Message::Assistant {
                content,
                reasoning,
                tool_calls: tool_calls.clone(),
            });

            for tc in &tool_calls {
                if self.is_cancelled() {
                    return Err(anyhow!("interrupted"));
                }
                if self.verbose {
                    eprintln!("  → {} {}", tc.name, tc.arguments);
                }
                if let Some(tx) = &self.events {
                    let _ = tx
                        .send(AgentEvent::ToolStart {
                            name: tc.name.clone(),
                            args: tc.arguments.to_string(),
                        })
                        .await;
                }
                // Snapshot files before a mutation so we can show the real diff.
                let snapshot = self.mutation_snapshot(tc);
                let result = if self.mode == super::AgentMode::Plan && plan_blocks(&tc.name) {
                    format!(
                        "tool error: {} is blocked in PLAN mode (read-only). Use read/grep/glob/list/git instead.",
                        tc.name
                    )
                } else {
                    match self.allow_tool(tc).await {
                        ToolApproval::Allowed => {
                            if tc.name == "task" {
                                self.run_subagent(tc).await
                            } else if tc.name == "question" {
                                self.run_question(tc).await
                            } else {
                                match self.tools.execute(&tc.name, tc.arguments.clone()).await {
                                    Ok(s) => s,
                                    Err(e) => format!("tool error: {e}"),
                                }
                            }
                        }
                        ToolApproval::Denied { feedback } => match feedback {
                            Some(msg) => {
                                format!("permission denied: the user denied {}: {msg}", tc.name)
                            }
                            None => format!(
                                "permission denied: {} blocked by permission policy",
                                tc.name
                            ),
                        },
                    }
                };
                if let Some(tx) = &self.events {
                    let _ = tx
                        .send(AgentEvent::ToolOutput {
                            name: tc.name.clone(),
                            output: result.clone(),
                        })
                        .await;
                }
                if !snapshot.is_empty() {
                    self.emit_diffs(&snapshot).await;
                }
                self.push(Message::Tool {
                    tool_call_id: tc.id.clone(),
                    content: result,
                });
            }
        }

        Err(anyhow!(
            "agent did not finish within {MAX_ITERATIONS} iterations"
        ))
    }

    /// Flat (non-recursive) agent loop used by sub-agents. The `task` tool is
    /// not advertised, and nested delegation is rejected. Keeping this a
    /// separate function avoids an infinitely-sized future from `run`.
    pub async fn run_flat(&mut self, input: &str) -> Result<String> {
        if self.is_cancelled() {
            return Err(anyhow!("interrupted"));
        }
        self.push(Message::User {
            content: input.to_string(),
        });

        for _ in 0..MAX_ITERATIONS {
            if self.is_cancelled() {
                return Err(anyhow!("interrupted"));
            }
            if context::estimate_tokens(&self.history) > self.max_context_tokens {
                self.compact().await?;
            }
            let rx = self
                .provider
                .complete_stream(&self.history, self.tools.defs())
                .await
                .context("provider error")?;
            let (content, reasoning, tool_calls) = self.drive_stream(rx).await?;

            if tool_calls.is_empty() {
                let text = content.unwrap_or_default();
                self.push(Message::Assistant {
                    content: Some(text.clone()),
                    reasoning,
                    tool_calls: vec![],
                });
                return Ok(text);
            }

            self.push(Message::Assistant {
                content,
                reasoning,
                tool_calls: tool_calls.clone(),
            });

            for tc in &tool_calls {
                if self.is_cancelled() {
                    return Err(anyhow!("interrupted"));
                }
                let result = if self.mode == super::AgentMode::Plan && plan_blocks(&tc.name) {
                    format!(
                        "tool error: {} is blocked in PLAN mode (read-only). Use read/grep/glob/list/git instead.",
                        tc.name
                    )
                } else {
                    match self.allow_tool(tc).await {
                        ToolApproval::Allowed => {
                            if tc.name == "task" {
                                "tool error: task: nested sub-agents are not supported".to_string()
                            } else {
                                match self.tools.execute(&tc.name, tc.arguments.clone()).await {
                                    Ok(s) => s,
                                    Err(e) => format!("tool error: {e}"),
                                }
                            }
                        }
                        ToolApproval::Denied { feedback } => match feedback {
                            Some(msg) => {
                                format!("permission denied: the user denied {}: {msg}", tc.name)
                            }
                            None => format!(
                                "permission denied: {} blocked by permission policy",
                                tc.name
                            ),
                        },
                    }
                };
                self.push(Message::Tool {
                    tool_call_id: tc.id.clone(),
                    content: result,
                });
            }
        }

        Err(anyhow!(
            "agent did not finish within {MAX_ITERATIONS} iterations"
        ))
    }

    /// Resolve `@path` mentions in `input` into a context block for the model.
    fn resolve_mentions(&mut self, input: &str, root: &std::path::Path) -> Option<String> {
        let index = self
            .file_index
            .get_or_insert_with(|| crate::files::FileIndex::build(root));
        crate::mentions::resolve_context(input, root, index)
    }

    /// Snapshot the files a mutation tool is about to touch (content or None).
    fn mutation_snapshot(&self, tc: &ToolCall) -> Vec<(std::path::PathBuf, Option<String>)> {
        let root = self
            .repo_root
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if !matches!(tc.name.as_str(), "write_file" | "edit_file" | "apply_patch") {
            return Vec::new();
        }
        crate::diff::affected_paths(&tc.name, &tc.arguments, &root)
            .into_iter()
            .map(|p| {
                let old = crate::diff::read_opt(&p);
                (p, old)
            })
            .collect()
    }

    /// Emit a `Diff` event for each file that actually changed.
    async fn emit_diffs(&mut self, snapshot: &[(std::path::PathBuf, Option<String>)]) {
        let Some(tx) = &self.events else {
            return;
        };
        for (path, body) in crate::diff::diffs_after(snapshot) {
            let file = path.to_string_lossy().replace('\\', "/");
            if tx.send(AgentEvent::Diff { file, body }).await.is_err() {
                return;
            }
        }
    }

    /// Start a completion and drive its stream, retrying once when the stream
    /// drops before producing any output (a transient server/proxy failure).
    async fn complete_with_retry(
        &mut self,
        messages: &[Message],
        defs: &[crate::providers::ToolDef],
    ) -> Result<(Option<String>, Option<String>, Vec<ToolCall>)> {
        self.stream_progress = false;
        let rx = self
            .provider
            .complete_stream(messages, defs)
            .await
            .context("provider error")?;
        match self.drive_stream(rx).await {
            Ok(v) => Ok(v),
            Err(e) if !self.stream_progress => {
                // Nothing was emitted, so a retry cannot duplicate output.
                crate::log_line!("completion stream dropped before output; retrying: {e}");
                let rx = self
                    .provider
                    .complete_stream(messages, defs)
                    .await
                    .context("provider error on retry")?;
                self.drive_stream(rx).await
            }
            Err(e) => Err(e),
        }
    }

    /// Run the `task` tool: delegate to a nested sub-agent with a cloned provider.
    async fn run_subagent(&mut self, tc: &ToolCall) -> String {
        let prompt = tc
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if prompt.trim().is_empty() {
            return "tool error: task: missing 'prompt' argument".to_string();
        }
        let model = tc
            .arguments
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let provider = self.provider.clone_box();
        let mut sub = Agent::new(
            provider,
            crate::tools::Registry::default(),
            false,
            self.policy.clone(),
            Box::new(|_| Choice::Deny { feedback: None }),
        );
        sub.set_max_context_tokens(self.max_context_tokens);
        if let Some(m) = model {
            sub.provider.set_model(&m);
        }
        // Box the nested run so its recursive type is erased. The sub-agent
        // runs in flat mode, so its future type does not reference `run`.
        let fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> =
            Box::pin(sub.run_flat(&prompt));
        match fut.await {
            Ok(text) => format!("[sub-agent] {}\n", text.trim()),
            Err(e) => format!("tool error: task failed: {e}"),
        }
    }

    /// Run the `question` tool: ask the user to pick an option via the UI.
    async fn run_question(&mut self, tc: &ToolCall) -> String {
        let prompt = tc
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let options: Vec<String> = tc
            .arguments
            .get("options")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if prompt.is_empty() || options.len() < 2 {
            return "tool error: question: need a prompt and at least 2 options".to_string();
        }

        if let Some(tx) = &self.events {
            let (respond, rx) = oneshot::channel();
            let send_ok = tx
                .send(AgentEvent::Question {
                    prompt,
                    options,
                    respond,
                })
                .await
                .is_ok();
            if !send_ok {
                return "tool error: question: no UI available".to_string();
            }
            return match rx.await {
                Ok(Some(answer)) => format!("User chose: {answer}"),
                Ok(None) => "User dismissed the question.".to_string(),
                Err(_) => "tool error: question channel closed".to_string(),
            };
        }
        // Non-interactive: print to stderr and read a numbered choice.
        use std::io::BufRead;
        eprintln!("{prompt}");
        for (i, o) in options.iter().enumerate() {
            eprintln!("  {}. {o}", i + 1);
        }
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return "User did not answer.".to_string();
        }
        match line.trim().parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => {
                format!("User chose: {}", options[n - 1])
            }
            _ => "User did not answer.".to_string(),
        }
    }

    /// Ask the user (or policy) whether this tool call may run.
    async fn allow_tool(&mut self, tc: &ToolCall) -> ToolApproval {
        let action = permissions::action_for(&tc.name);
        if self.session_allowed.contains(&action) {
            return ToolApproval::Allowed;
        }
        let target = permissions::tool_target_for_policy(tc);
        let level = self.policy.level_for_target(action, &target);
        if level == Level::Deny {
            return ToolApproval::Denied { feedback: None };
        }
        let mut must_prompt = level == Level::Ask;
        let mut dangerous = false;
        if action == Action::Shell {
            if let Some(cmd) = tc.arguments.get("command").and_then(|v| v.as_str()) {
                if permissions::policy::is_dangerous_command(cmd) {
                    must_prompt = true;
                    dangerous = true;
                }
            }
        }
        if !must_prompt {
            return ToolApproval::Allowed;
        }
        // AUTO mode auto-approves routine actions but still prompts for
        // dangerous commands and always respects Deny rules above.
        if self.mode == super::AgentMode::Auto && !dangerous {
            return ToolApproval::Allowed;
        }
        let prompt = permissions::describe_tool(tc);

        // Event-driven mode: ask the UI, wait for its answer.
        if let Some(tx) = &self.events {
            let (respond, rx) = oneshot::channel();
            let send_ok = tx
                .send(AgentEvent::Permission { prompt, respond })
                .await
                .is_ok();
            if !send_ok {
                return ToolApproval::Denied { feedback: None };
            }
            return match rx.await {
                Ok(Choice::Allow) => ToolApproval::Allowed,
                Ok(Choice::AllowForSession) => {
                    self.session_allowed.insert(action);
                    ToolApproval::Allowed
                }
                Ok(Choice::Always) => {
                    self.session_allowed.insert(action);
                    if let Some(s) = &self.session {
                        let _ = s.approve_always(permissions::action_name(action));
                    }
                    ToolApproval::Allowed
                }
                Ok(Choice::Deny { feedback }) => ToolApproval::Denied { feedback },
                _ => ToolApproval::Denied { feedback: None },
            };
        }

        match (self.prompter)(&prompt) {
            Choice::Allow => ToolApproval::Allowed,
            Choice::AllowForSession => {
                self.session_allowed.insert(action);
                ToolApproval::Allowed
            }
            Choice::Always => {
                self.session_allowed.insert(action);
                if let Some(s) = &self.session {
                    let _ = s.approve_always(permissions::action_name(action));
                }
                ToolApproval::Allowed
            }
            Choice::Deny { .. } => ToolApproval::Denied { feedback: None },
        }
    }

    /// Replace the oldest messages (beyond a kept tail) with a model-generated summary.
    pub(crate) async fn compact(&mut self) -> Result<()> {
        let mut new_history: Vec<Message> = Vec::new();
        if let Some(Message::System { .. }) = self.history.first() {
            new_history.push(self.history.remove(0));
        }
        let keep = super::COMPACTION_KEEP_TAIL.min(self.history.len());
        let split = self.history.len() - keep;
        if split == 0 {
            return Ok(());
        }
        let dropped = self.history.drain(..split).collect::<Vec<_>>();
        let summary = match self.summarize(&dropped).await {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => format!(
                "[{} earlier messages compacted without summary]",
                dropped.len()
            ),
        };
        new_history.push(Message::System {
            content: format!("Summary of earlier conversation:\n{summary}"),
        });
        new_history.append(&mut self.history);
        self.history = new_history;
        Ok(())
    }

    async fn summarize(&mut self, messages: &[Message]) -> Result<String> {
        let mut ctx = vec![Message::System {
            content: SUMMARY_PROMPT.to_string(),
        }];
        ctx.extend(messages.iter().cloned());
        let (content, _, _) = self.complete_with_retry(&ctx, &[]).await?;
        Ok(content.unwrap_or_default())
    }
    /// Drain a streaming response, emitting text events and assembling tool calls.
    /// Stops early when the cancel watch fires.
    async fn drive_stream(
        &mut self,
        mut rx: mpsc::Receiver<StreamEvent>,
    ) -> Result<(Option<String>, Option<String>, Vec<ToolCall>)> {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut arg_bufs: Vec<String> = Vec::new();
        let mut out = std::io::stdout();

        loop {
            let ev = if let Some(cancel) = &mut self.cancel {
                tokio::select! {
                    e = rx.recv() => e,
                    _ = cancel_wait(cancel) => return Err(anyhow!("interrupted")),
                }
            } else {
                rx.recv().await
            };
            let Some(ev) = ev else { break };

            match ev {
                StreamEvent::Text(t) => {
                    self.stream_progress = true;
                    content.push_str(&t);
                    if self.display_stream {
                        let _ = out.write_all(t.as_bytes());
                        let _ = out.flush();
                    }
                    if let Some(tx) = &self.events {
                        if tx.send(AgentEvent::Text(t)).await.is_err() {
                            return Err(anyhow!("ui channel closed"));
                        }
                    }
                }
                StreamEvent::Reasoning(t) => {
                    reasoning.push_str(&t);
                    if let Some(tx) = &self.events {
                        if tx.send(AgentEvent::Reasoning(t)).await.is_err() {
                            return Err(anyhow!("ui channel closed"));
                        }
                    }
                }
                StreamEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments,
                } => {
                    self.stream_progress = true;
                    while tool_calls.len() <= index {
                        tool_calls.push(ToolCall {
                            id: String::new(),
                            name: String::new(),
                            arguments: Value::Null,
                        });
                        arg_bufs.push(String::new());
                    }
                    if let Some(id) = id {
                        tool_calls[index].id = id;
                    }
                    if let Some(name) = name {
                        tool_calls[index].name = name;
                    }
                    if let Some(a) = arguments {
                        arg_bufs[index].push_str(&a);
                    }
                }
                StreamEvent::Done => break,
                StreamEvent::Error(e) => return Err(anyhow!("provider stream error: {e}")),
            }
        }

        for (i, tc) in tool_calls.iter_mut().enumerate() {
            if let Ok(v) = serde_json::from_str::<Value>(&arg_bufs[i]) {
                tc.arguments = v;
            }
        }
        let content = if content.is_empty() {
            None
        } else {
            Some(content)
        };
        let reasoning = if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        };
        Ok((content, reasoning, tool_calls))
    }
}

async fn cancel_wait(rx: &mut watch::Receiver<bool>) {
    // Consume any already-sent value so an unrelated `send` doesn't look like a cancel.
    if *rx.borrow_and_update() {
        return;
    }
    let _ = rx.changed().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentMode;
    use crate::permissions::{Action, Choice, Level, Policy};
    use crate::providers::{Provider, ProviderError, ToolDef};
    use crate::tools::Registry;
    use serde_json::json;

    use std::collections::HashMap;
    use std::collections::VecDeque;

    use std::sync::Mutex;

    #[derive(Clone)]
    struct ProviderResponse {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    }

    #[derive(Clone)]
    struct MockProvider {
        responses: std::sync::Arc<Mutex<VecDeque<ProviderResponse>>>,
        errors: std::sync::Arc<Mutex<usize>>,
        models_set: std::sync::Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDef],
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, ProviderError> {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            let error_now = {
                let mut e = self.errors.lock().unwrap();
                if *e > 0 {
                    *e -= 1;
                    1
                } else {
                    0
                }
            };
            let response = if error_now > 0 {
                None // don't consume a response when this call errors
            } else {
                self.responses.lock().unwrap().pop_front()
            };
            tokio::spawn(async move {
                if error_now > 0 {
                    let _ = tx.send(StreamEvent::Error("mock failure".into())).await;
                    return;
                }
                if let Some(resp) = response {
                    if let Some(c) = resp.content {
                        let _ = tx.send(StreamEvent::Text(c)).await;
                    }
                    for (i, tc) in resp.tool_calls.into_iter().enumerate() {
                        let _ = tx
                            .send(StreamEvent::ToolCallDelta {
                                index: i,
                                id: Some(tc.id),
                                name: Some(tc.name),
                                arguments: Some(tc.arguments.to_string()),
                            })
                            .await;
                    }
                }
                let _ = tx.send(StreamEvent::Done).await;
            });
            Ok(rx)
        }

        fn set_model(&mut self, model: &str) {
            self.models_set.lock().unwrap().push(model.to_string());
        }

        fn clone_box(&self) -> Box<dyn Provider> {
            // Fresh response queue (like a real provider has no state), but
            // model-set tracking is shared so a provider switch is observable.
            Box::new(MockProvider {
                responses: std::sync::Arc::new(Mutex::new(VecDeque::new())),
                errors: std::sync::Arc::new(Mutex::new(0)),
                models_set: self.models_set.clone(),
            })
        }
    }

    fn mock(responses: VecDeque<ProviderResponse>) -> MockProvider {
        MockProvider {
            responses: std::sync::Arc::new(Mutex::new(responses)),
            errors: std::sync::Arc::new(Mutex::new(0)),
            models_set: std::sync::Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn deny_prompter() -> Box<dyn FnMut(&str) -> Choice + Send> {
        Box::new(|_| Choice::Deny { feedback: None })
    }

    fn plain_response(text: &str) -> ProviderResponse {
        ProviderResponse {
            content: Some(text.into()),
            tool_calls: vec![],
        }
    }

    fn shell_call(id: &str, command: &str) -> ProviderResponse {
        ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: "shell".into(),
                arguments: json!({"command": command}),
            }],
        }
    }

    #[tokio::test]
    async fn runs_tool_then_returns_final_answer() {
        let f = std::env::temp_dir().join(format!("lightcode_agent_{}.txt", std::process::id()));
        std::fs::write(&f, "hello world").unwrap();

        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: json!({"path": f.to_string_lossy()}),
            }],
        });
        responses.push_back(plain_response("file was read"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let out = agent.run("read the file").await.unwrap();
        assert_eq!(out, "file was read");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("hello world")
        )));

        std::fs::remove_file(&f).ok();
    }

    #[tokio::test]
    async fn tool_error_is_fed_back_to_model() {
        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "does_not_exist".into(),
                arguments: json!({}),
            }],
        });
        responses.push_back(plain_response("handled the error"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let out = agent.run("try the tool").await.unwrap();
        assert_eq!(out, "handled the error");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("unknown tool")
        )));
    }

    #[tokio::test]
    async fn provider_error_propagates_with_context() {
        let provider = mock(VecDeque::new());
        *provider.errors.lock().unwrap() = 2; // fails on first attempt AND retry

        let mut agent = Agent::new(
            Box::new(provider),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let err = agent.run("hi").await.unwrap_err();
        let chain: Vec<_> = err.chain().map(|c| c.to_string()).collect();
        assert!(chain.iter().any(|c| c.contains("provider stream error")));
        assert!(chain.iter().any(|c| c.contains("mock failure")));
    }

    #[tokio::test]
    async fn transient_stream_error_is_retried() {
        // First attempt fails before any output, retry succeeds.
        let provider = mock(VecDeque::new());
        *provider.errors.lock().unwrap() = 1;
        *provider.responses.lock().unwrap() = VecDeque::from([plain_response("recovered")]);

        let mut agent = Agent::new(
            Box::new(provider),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let out = agent.run("hi").await.unwrap();
        assert_eq!(out, "recovered");
    }

    #[tokio::test]
    async fn permission_denied_blocks_tool() {
        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "echo hi"));
        responses.push_back(plain_response("ok"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let out = agent.run("run it").await.unwrap();
        assert_eq!(out, "ok");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("permission denied")
        )));
    }

    #[tokio::test]
    async fn allow_for_session_skips_further_prompts() {
        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "echo first"));
        responses.push_back(shell_call("call_2", "echo second"));
        responses.push_back(plain_response("done"));

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let prompter: Box<dyn FnMut(&str) -> Choice + Send> = {
            let calls = calls.clone();
            Box::new(move |_: &str| {
                let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Choice::AllowForSession
                } else {
                    Choice::Deny { feedback: None }
                }
            })
        };

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            prompter,
        );
        let out = agent.run("run both").await.unwrap();
        assert_eq!(out, "done");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "second shell call must not prompt again"
        );
        assert!(agent.session_allowed.contains(&Action::Shell));
        assert!(!agent.history.iter().any(
            |m| matches!(m, Message::Tool { content, .. } if content.contains("permission denied"))
        ));
    }

    #[tokio::test]
    async fn dangerous_command_prompts_even_when_shell_allowed() {
        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "rm -rf /tmp/lightcode_danger"));
        responses.push_back(plain_response("not run"));

        let policy = Policy {
            shell: Level::Allow,
            ..Policy::default()
        };
        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            policy,
            deny_prompter(),
        );
        let out = agent.run("clean up").await.unwrap();
        assert_eq!(out, "not run");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("permission denied")
        )));
    }

    #[tokio::test]
    async fn compaction_summarizes_old_messages() {
        let mut responses = VecDeque::new();
        responses.push_back(plain_response("SUMMARY_TEXT"));
        responses.push_back(plain_response("final answer"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        agent.set_max_context_tokens(10);
        for i in 0..30 {
            agent.history.push(Message::User {
                content: format!("msg {i}"),
            });
        }

        let out = agent.run("finish").await.unwrap();
        assert_eq!(out, "final answer");
        let has_summary = agent.history.iter().any(|m| {
            matches!(
                m,
                Message::System { content } if content.contains("SUMMARY_TEXT")
            )
        });
        assert!(
            has_summary,
            "compact() must replace old messages with a summary"
        );
        assert!(
            agent.history.len() < 35,
            "history should shrink after compaction"
        );
    }

    #[tokio::test]
    async fn always_persists_to_session_meta() {
        let dir =
            std::env::temp_dir().join(format!("lightcode_agent_always_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // create_in takes the dir directly — no global env var to race with.
        let session = crate::session::storage::create_in(&dir).unwrap();

        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "echo first"));
        responses.push_back(shell_call("call_2", "echo second"));
        responses.push_back(plain_response("done"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            Box::new(|_| Choice::Always),
        );
        agent.set_session(Some(session.clone()));
        let out = agent.run("run both").await.unwrap();
        assert_eq!(out, "done");

        let approved = session.approved_actions();
        assert!(approved.iter().any(|a| a == "shell"));
        // A fresh agent seeded from the persisted list must not prompt again.
        let mut agent2 = Agent::new(
            Box::new(mock(VecDeque::new())),
            Registry::default(),
            false,
            Policy::default(),
            Box::new(|_| Choice::Always),
        );
        agent2.set_approved(&approved);
        assert!(agent2
            .allow_tool(&ToolCall {
                id: "x".into(),
                name: "shell".into(),
                arguments: json!({"command": "echo ok"}),
            })
            .await
            .matches_allowed());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn task_tool_runs_subagent() {
        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "task".into(),
                arguments: json!({"description": "x", "prompt": "do a thing"}),
            }],
        });
        responses.push_back(plain_response("parent done"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let out = agent.run("delegate").await.unwrap();
        assert_eq!(out, "parent done");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("[sub-agent]")
        )));
    }

    #[tokio::test]
    async fn question_tool_asks_via_events() {
        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "question".into(),
                arguments: json!({"prompt": "pick one", "options": ["a", "b", "c"]}),
            }],
        });
        responses.push_back(plain_response("got answer"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        agent.set_events(Some(tx));
        let agent_task = tokio::spawn(async move {
            let r = agent.run("ask").await;
            (r, agent)
        });

        // The UI sees the Question event and answers.
        let mut answered = false;
        loop {
            let Some(ev) = rx.recv().await else { break };
            if let AgentEvent::Question {
                respond, options, ..
            } = ev
            {
                assert_eq!(options, vec!["a", "b", "c"]);
                let _ = respond.send(Some("b".to_string()));
                answered = true;
                break;
            }
        }
        assert!(answered, "question event must be emitted");
        let (res, agent) = agent_task.await.unwrap();
        res.unwrap();
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("User chose: b")
        )));
    }

    #[tokio::test]
    async fn switching_provider_swaps_provider_and_model() {
        let mut responses_a = VecDeque::new();
        responses_a.push_back(plain_response("from A"));
        let provider_b = mock(VecDeque::new());
        let models_b = provider_b.models_set.clone();

        let mut agent = Agent::new(
            Box::new(mock(responses_a)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let mut map: HashMap<String, Box<dyn Provider>> = HashMap::new();
        map.insert("b".to_string(), Box::new(provider_b));
        agent.set_provider_map(map);

        // Unknown provider: nothing changes.
        agent.set_model_with_provider("nope", "m1");
        assert!(models_b.lock().unwrap().is_empty());

        // Real switch applies the model to the swapped provider.
        agent.set_model_with_provider("b", "mB");
        assert!(models_b.lock().unwrap().contains(&"mB".to_string()));
    }

    #[tokio::test]
    async fn plan_mode_blocks_mutation_tools() {
        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "write_file".into(),
                arguments: json!({"path": "/tmp/x.txt", "content": "x"}),
            }],
        });
        responses.push_back(plain_response("ok"));

        let policy = Policy {
            write: Level::Allow, // passes permission; mode must still block
            ..Policy::default()
        };
        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            policy,
            deny_prompter(),
        );
        agent.mode = AgentMode::Plan;
        let out = agent.run("write a file").await.unwrap();
        assert_eq!(out, "ok");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("blocked in PLAN mode")
        )));
        // Read tools remain available in PLAN.
        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "grep".into(),
                arguments: json!({"pattern": "x", "path": "."}),
            }],
        });
        responses.push_back(plain_response("done"));
        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        agent.mode = AgentMode::Plan;
        agent.run("search").await.unwrap();
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if !content.contains("blocked in PLAN mode")
        )));
    }

    #[tokio::test]
    async fn auto_mode_auto_approves_routine_actions() {
        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "echo auto-ran"));
        responses.push_back(plain_response("done"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(), // shell = Ask
            deny_prompter(),   // would deny if prompted
        );
        agent.mode = AgentMode::Auto;
        let out = agent.run("run it").await.unwrap();
        assert_eq!(out, "done");
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::Tool { content, .. } if content.contains("auto-ran") && !content.contains("permission denied")
        )));
    }

    #[test]
    fn mode_cycles_plan_build_auto() {
        let mut m = AgentMode::Plan;
        assert_eq!(m.next(), AgentMode::Build);
        m = m.next();
        assert_eq!(m.next(), AgentMode::Auto);
        m = m.next();
        assert_eq!(m.next(), AgentMode::Plan);
        assert_eq!(AgentMode::from_str("plan"), Some(AgentMode::Plan));
        assert_eq!(AgentMode::from_str("PLAN"), Some(AgentMode::Plan));
        assert_eq!(AgentMode::from_str("Auto"), Some(AgentMode::Auto));
        assert_eq!(AgentMode::from_str("nope"), None);
    }

    #[tokio::test]
    async fn mode_prompt_is_injected_per_turn() {
        let mut responses = VecDeque::new();
        responses.push_back(plain_response("hello"));
        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        agent.mode = AgentMode::Plan;
        agent.run("analyze").await.unwrap();
        assert!(agent.history.iter().any(|m| matches!(
            m,
            Message::System { content } if content.contains("PLAN mode") && content.contains("read-only")
        )));
    }

    #[tokio::test]
    async fn edit_file_emits_diff_event() {
        let dir = std::env::temp_dir().join(format!("lc_agent_diff_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old line\n").unwrap();

        let mut responses = VecDeque::new();
        responses.push_back(ProviderResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "edit_file".into(),
                arguments: json!({
                    "path": file.to_string_lossy(),
                    "old_string": "old line",
                    "new_string": "new line",
                }),
            }],
        });
        responses.push_back(plain_response("done"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy {
                edit: Level::Allow,
                ..Policy::default()
            },
            deny_prompter(),
        );
        agent.repo_root = Some(dir.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        agent.set_events(Some(tx));
        let agent_task = tokio::spawn(async move {
            let r = agent.run("edit the file").await;
            (r, agent)
        });

        let mut saw_diff = false;
        loop {
            let Some(ev) = rx.recv().await else { break };
            if let AgentEvent::Diff { file: f, body } = ev {
                saw_diff = true;
                assert!(f.ends_with("a.txt"));
                assert!(body.contains("-old line"), "diff: {body}");
                assert!(body.contains("+new line"), "diff: {body}");
                break;
            }
        }
        assert!(saw_diff, "edit_file must emit a Diff event");
        let (res, _) = agent_task.await.unwrap();
        res.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn events_are_emitted_and_cancel_works() {
        let mut responses = VecDeque::new();
        responses.push_back(shell_call("call_1", "echo hi"));
        responses.push_back(plain_response("done"));

        let mut agent = Agent::new(
            Box::new(mock(responses)),
            Registry::default(),
            false,
            Policy::default(),
            deny_prompter(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        agent.set_events(Some(tx));
        agent.set_cancel(Some(cancel_rx));

        let task = tokio::spawn(async move {
            let r = agent.run("go").await;
            (r, agent)
        });

        let (done_tx, mut done_rx) = oneshot::channel();
        let collector = tokio::spawn(async move {
            let mut saw_tool = false;
            let mut saw_permission = false;
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(AgentEvent::ToolStart { .. }) => saw_tool = true,
                        Some(AgentEvent::Permission { respond, .. }) => {
                            saw_permission = true;
                            let _ = respond.send(Choice::Allow);
                        }
                        Some(_) | None => {}
                    },
                    _ = &mut done_rx => break,
                }
            }
            (saw_tool, saw_permission)
        });

        let (result, mut agent) = task.await.unwrap();
        result.unwrap();
        done_tx.send(()).ok();
        let (saw_tool, saw_permission) = collector.await.unwrap();
        assert!(saw_tool && saw_permission);

        // Second run: cancel mid-flight.
        let (cancel_tx2, cancel_rx2) = watch::channel(false);
        agent.set_cancel(Some(cancel_rx2));
        let task = tokio::spawn(async move {
            let r = agent.run("another").await;
            (agent, r)
        });
        cancel_tx2.send(true).ok();
        let (agent2, res) = task.await.unwrap();
        assert!(res.is_err());
        assert!(agent2.is_cancelled());
    }
}

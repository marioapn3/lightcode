//! Autonomous goal execution: a turn loop that drives the existing agent,
//! verifies results, and evaluates completion with evidence before continuing.

use crate::agent::{Agent, AgentEvent};
use crate::permissions::policy::is_dangerous_command;
use crate::providers::{Message, Provider, StreamEvent};
use crate::session::Session;
use crate::tools::exec;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;

/// A goal's lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    Pending,
    Running,
    Evaluating,
    Completed,
    Failed,
    MaxTurnsReached,
    Cancelled,
}

impl GoalStatus {
    pub fn label(&self) -> &'static str {
        match self {
            GoalStatus::Pending => "pending",
            GoalStatus::Running => "running",
            GoalStatus::Evaluating => "evaluating",
            GoalStatus::Completed => "completed",
            GoalStatus::Failed => "failed",
            GoalStatus::MaxTurnsReached => "max turns reached",
            GoalStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_stopped(&self) -> bool {
        matches!(
            self,
            GoalStatus::Completed
                | GoalStatus::Failed
                | GoalStatus::MaxTurnsReached
                | GoalStatus::Cancelled
        )
    }
}

/// Actual result of running one declared verification command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub command: String,
    pub exit_code: i32,
    pub success: bool,
    pub output: String,
}

/// Structured completion judgment from the goal evaluator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalEvaluation {
    pub completed: bool,
    pub reason: String,
    pub remaining_work: Vec<String>,
    pub evidence: Vec<String>,
}

impl GoalEvaluation {
    pub fn incomplete(reason: impl Into<String>) -> Self {
        GoalEvaluation {
            completed: false,
            reason: reason.into(),
            remaining_work: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

/// A single file touched by the goal (git short status: M/A/D/R).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub status: String,
    pub path: String,
}

/// A goal's persistent state. Serialized to the session dir as `<id>.goal.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub description: String,
    pub verify_commands: Vec<String>,
    pub max_turns: u32,
    pub current_turn: u32,
    pub status: GoalStatus,
    /// Unix seconds when the goal started.
    pub started_at: u64,
    pub last_evaluation: Option<GoalEvaluation>,
    pub changed_files: Vec<FileChange>,
    /// Final status message shown in the TUI.
    #[serde(default)]
    pub message: String,
}

impl Goal {
    pub fn new(description: String, verify_commands: Vec<String>, max_turns: u32) -> Self {
        Goal {
            description,
            verify_commands,
            max_turns: max_turns.max(1),
            current_turn: 0,
            status: GoalStatus::Pending,
            started_at: now_secs(),
            last_evaluation: None,
            changed_files: Vec::new(),
            message: String::new(),
        }
    }
}

/// Parsed `/goal` input, decoupled from the persistent `Goal`.
#[derive(Debug, Clone)]
pub struct GoalSpec {
    pub description: String,
    pub verify_commands: Vec<String>,
    pub max_turns: u32,
}

impl GoalSpec {
    pub fn to_goal(&self) -> Goal {
        Goal::new(
            self.description.clone(),
            self.verify_commands.clone(),
            self.max_turns,
        )
    }
}

/// Parse `/goal <objective>` input. Supports:
///
/// ```text
/// fix all failing tests
/// verify:
///   npm test
///   npm run lint
/// max_turns: 8
/// ```
///
/// `verify:` lines and `max_turns:` are optional; when absent the evaluator
/// determines verification from the objective and repository.
pub fn parse_goal(input: &str, default_max_turns: u32) -> GoalSpec {
    let mut description = String::new();
    let mut verify_commands = Vec::new();
    let mut max_turns = default_max_turns;
    let mut in_verify = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("max_turns:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                max_turns = n.max(1);
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("verify:") {
            in_verify = true;
            let cmd = rest.trim();
            if !cmd.is_empty() {
                verify_commands.push(cmd.to_string());
            }
            continue;
        }
        if in_verify {
            if !trimmed.is_empty() {
                verify_commands.push(trimmed.to_string());
            }
            continue;
        }
        if !description.is_empty() {
            description.push('\n');
        }
        description.push_str(trimmed);
    }
    GoalSpec {
        description: description.trim().to_string(),
        verify_commands,
        max_turns: max_turns.max(1),
    }
}

/// UI events emitted by the goal manager. High-level only; never raw reasoning.
#[derive(Debug)]
pub enum GoalUiEvent {
    Started { description: String, max_turns: u32 },
    Turn { turn: u32, max_turns: u32 },
    Verification(VerificationResult),
    Evaluation(GoalEvaluation),
    Finished { status: GoalStatus, turns: u32, seconds: u64, message: String },
}

/// Evidence given to the evaluator for one completion judgment.
pub struct GoalContext {
    /// The agent's final summary text for the turn.
    pub turn_summary: String,
    pub verification: Vec<VerificationResult>,
    /// Bounded tail of tool results from this turn.
    pub history_excerpt: String,
}

/// Completion evaluator. Must judge evidence, never the agent's claims alone.
#[async_trait::async_trait]
pub trait GoalEvaluator: Send + Sync {
    async fn evaluate(&self, goal: &Goal, ctx: &GoalContext) -> Result<GoalEvaluation>;
}

const EVAL_SYSTEM_PROMPT: &str = "\
You are a goal completion evaluator for a coding agent. Your job is to decide, \
from evidence, whether a coding goal is actually complete.

Use ONLY the provided evidence: declared verification command results, the \
agent's summary of what it changed and tested, changed files, and tool history. \
Do not trust a claim like 'tests should pass' without evidence.

Be strict. A goal is COMPLETE only when its declared verification commands all \
passed (exit code 0) and there is no evidence that the objective is unmet. When \
verification failed or work clearly remains, report INCOMPLETE with concrete \
remaining work.

Respond with ONLY a JSON object and nothing else:
{\"completed\": true|false, \"reason\": \"...\", \"remaining_work\": [\"...\"], \"evidence\": [\"...\"]}";

/// Model-based evaluator using a (possibly dedicated) provider.
pub struct ModelGoalEvaluator {
    pub provider: Box<dyn Provider>,
}

#[async_trait::async_trait]
impl GoalEvaluator for ModelGoalEvaluator {
    async fn evaluate(&self, goal: &Goal, ctx: &GoalContext) -> Result<GoalEvaluation> {
        let user = evaluator_prompt(goal, ctx);
        let text = complete_text(&*self.provider, EVAL_SYSTEM_PROMPT, &user).await?;
        match parse_evaluation_json(&text) {
            Some(ev) => Ok(ev),
            None => Err(anyhow!(
                "unparseable evaluator output: {}",
                crate::tools::bound(text)
            )),
        }
    }
}

fn evaluator_prompt(goal: &Goal, ctx: &GoalContext) -> String {
    let mut out = String::new();
    out.push_str(&format!("GOAL: {}\n\n", goal.description));
    if !goal.verify_commands.is_empty() {
        out.push_str("DECLARED VERIFICATION COMMANDS (must all pass):\n");
        for c in &goal.verify_commands {
            out.push_str(&format!("  $ {c}\n"));
        }
        out.push('\n');
        out.push_str("VERIFICATION RESULTS:\n");
        for v in &ctx.verification {
            let mark = if v.success { "PASS" } else { "FAIL" };
            out.push_str(&format!(
                "  [{mark}] exit {} $ {}\n",
                v.exit_code, v.command
            ));
            let tail: String = v.output.chars().take(800).collect();
            if !tail.is_empty() {
                out.push_str(&format!("      {}\n", tail.replace('\n', "\n      ")));
            }
        }
        out.push('\n');
    }
    if let Some(e) = &goal.last_evaluation {
        out.push_str("PREVIOUS EVALUATION (was NOT complete):\n");
        out.push_str(&format!("  reason: {}\n", e.reason));
        for r in &e.remaining_work {
            out.push_str(&format!("  remaining: {r}\n"));
        }
        out.push('\n');
    }
    if !goal.changed_files.is_empty() {
        out.push_str("FILES CHANGED SO FAR:\n");
        for c in &goal.changed_files {
            out.push_str(&format!("  {} {}\n", c.status, c.path));
        }
        out.push('\n');
    }
    if !ctx.history_excerpt.is_empty() {
        out.push_str("RECENT TOOL RESULTS (bounded):\n");
        out.push_str(&ctx.history_excerpt);
        out.push('\n');
    }
    out.push_str(&format!("CURRENT TURN SUMMARY (agent's report):\n{}\n", ctx.turn_summary));
    out
}

/// Drain a streaming completion into plain text (no tools, no UI events).
pub async fn complete_text(provider: &dyn Provider, system: &str, user: &str) -> Result<String> {
    let msgs = vec![
        Message::System {
            content: system.to_string(),
        },
        Message::User {
            content: user.to_string(),
        },
    ];
    let rx = provider
        .complete_stream(&msgs, &[])
        .await
        .map_err(|e| anyhow!("evaluator provider error: {e}"))?;
    let mut rx = rx;
    let mut out = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            StreamEvent::Text(t) => out.push_str(&t),
            StreamEvent::Error(e) => return Err(anyhow!("evaluator provider error: {e}")),
            _ => {}
        }
    }
    Ok(out)
}

/// Extract a JSON object (tolerating markdown fences and prose) into a
/// `GoalEvaluation`. Missing fields default to a strict incomplete.
pub fn parse_evaluation_json(text: &str) -> Option<GoalEvaluation> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let strings = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(GoalEvaluation {
        completed: value.get("completed").and_then(|c| c.as_bool()).unwrap_or(false),
        reason: value
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string(),
        remaining_work: strings("remaining_work"),
        evidence: strings("evidence"),
    })
}

/// Abstract the piece of the agent the goal loop drives, so the loop is
/// testable without a live provider.
#[async_trait::async_trait]
pub trait TurnRunner: Send {
    async fn run_turn(&mut self, prompt: &str) -> Result<String>;
    fn is_cancelled(&self) -> bool;
    fn session(&self) -> Option<&Session>;
    fn repo_root(&self) -> Option<PathBuf>;
}

#[async_trait::async_trait]
impl TurnRunner for Agent {
    async fn run_turn(&mut self, prompt: &str) -> Result<String> {
        self.run(prompt).await
    }

    fn is_cancelled(&self) -> bool {
        Agent::is_cancelled(self)
    }

    fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    fn repo_root(&self) -> Option<PathBuf> {
        self.repo_root.clone()
    }
}

/// Signature of the current failure state, to stop repeated ineffective turns.
fn failure_fingerprint(goal: &Goal, verification: &[VerificationResult]) -> String {
    let mut s = String::new();
    for v in verification.iter().filter(|v| !v.success) {
        s.push_str(&format!("{}:{};", v.command, v.exit_code));
    }
    if let Some(e) = &goal.last_evaluation {
        for r in &e.remaining_work {
            s.push_str(&format!("{r};"));
        }
    }
    s
}

const MAX_REPEATED_FAILURES: u32 = 3;

/// Orchestrates the goal loop around the agent's existing turn pipeline.
pub struct GoalManager {
    pub goal: Goal,
    evaluator: Box<dyn GoalEvaluator>,
    started: Instant,
    last_fingerprint: String,
    repeated: u32,
}

impl GoalManager {
    pub fn new(goal: Goal, evaluator: Box<dyn GoalEvaluator>) -> Self {
        GoalManager {
            goal,
            evaluator,
            started: Instant::now(),
            last_fingerprint: String::new(),
            repeated: 0,
        }
    }

    /// Run the goal to completion, cancellation, or a stopping condition.
    pub async fn run<G: TurnRunner>(
        &mut self,
        runner: &mut G,
        events: &mpsc::Sender<AgentEvent>,
    ) -> GoalStatus {
        crate::log_line!("goal.started {}", self.goal.description);
        self.goal.status = GoalStatus::Running;
        self.emit(events, GoalUiEvent::Started {
            description: self.goal.description.clone(),
            max_turns: self.goal.max_turns,
        })
        .await;
        self.persist(&mut *runner);

        while self.goal.current_turn < self.goal.max_turns {
            if runner.is_cancelled() {
                self.finish(GoalStatus::Cancelled, "Goal cancelled".into());
                break;
            }
            self.goal.current_turn += 1;
            self.goal.status = GoalStatus::Running;
            crate::log_line!("goal.turn.started {} {}", self.goal.current_turn, self.goal.description);
            self.emit(events, GoalUiEvent::Turn {
                turn: self.goal.current_turn,
                max_turns: self.goal.max_turns,
            })
            .await;

            let prompt = self.turn_prompt();
            let turn_text = match runner.run_turn(&prompt).await {
                Ok(t) => t,
                Err(e) => {
                    if runner.is_cancelled() {
                        self.finish(GoalStatus::Cancelled, "Goal cancelled".into());
                    } else {
                        crate::log_line!("goal.turn.error {}", e);
                        self.finish(
                            GoalStatus::Failed,
                            format!("Turn {} failed: {e}", self.goal.current_turn),
                        );
                    }
                    break;
                }
            };
            crate::log_line!("goal.turn.completed {}", self.goal.current_turn);
            self.goal.changed_files = current_changes(runner.repo_root()).await;

            self.goal.status = GoalStatus::Evaluating;
            crate::log_line!("goal.evaluation.started");
            let verification = self.run_verification(&mut *runner, events).await;

            let excerpt = history_excerpt(&mut *runner).await;
            let ctx = GoalContext {
                turn_summary: turn_text,
                verification: verification.clone(),
                history_excerpt: excerpt,
            };
            let evaluation = match self.evaluator.evaluate(&self.goal, &ctx).await {
                Ok(e) => e,
                Err(e) => {
                    crate::log_line!("goal.evaluation.error {}", e);
                    GoalEvaluation::incomplete(format!("evaluator error: {e}"))
                }
            };
            crate::log_line!("goal.evaluation.completed completed={}", evaluation.completed);
            self.goal.last_evaluation = Some(evaluation.clone());
            self.emit(events, GoalUiEvent::Evaluation(evaluation.clone()))
                .await;
            self.persist(&mut *runner);

            let verify_ok = self.goal.verify_commands.is_empty()
                || verification.iter().all(|v| v.success);
            if verify_ok && evaluation.completed {
                let reason = if evaluation.reason.is_empty() {
                    "Goal completed".into()
                } else {
                    evaluation.reason.clone()
                };
                self.finish(GoalStatus::Completed, reason);
                break;
            }

            // Loop prevention: the same failing signature three times in a row
            // means the agent is not making progress.
            let fp = failure_fingerprint(&self.goal, &verification);
            if fp.is_empty() {
                self.repeated = 0;
                self.last_fingerprint.clear();
            } else if fp == self.last_fingerprint {
                self.repeated += 1;
                if self.repeated >= MAX_REPEATED_FAILURES {
                    self.finish(
                        GoalStatus::Failed,
                        format!(
                            "No progress after {} turns on the same failures.",
                            self.repeated
                        ),
                    );
                    break;
                }
            } else {
                self.repeated = 1;
                self.last_fingerprint = fp;
            }
        }

        if self.goal.status == GoalStatus::Running || self.goal.status == GoalStatus::Evaluating {
            self.finish(
                GoalStatus::MaxTurnsReached,
                format!("Maximum turns reached: {}", self.goal.max_turns),
            );
        }
        let secs = self.started.elapsed().as_secs();
        self.emit(events, GoalUiEvent::Finished {
            status: self.goal.status,
            turns: self.goal.current_turn,
            seconds: secs,
            message: self.goal.message.clone(),
        })
        .await;
        self.persist(&mut *runner);
        crate::log_line!("goal.finished {} turns={}", self.goal.status.label(), self.goal.current_turn);
        self.goal.status
    }

    fn finish(&mut self, status: GoalStatus, message: String) {
        crate::log_line!("goal.{} {}", status.label(), message);
        self.goal.status = status;
        self.goal.message = message;
    }

    /// Build the bounded prompt for the next agent turn. The agent's own
    /// history carries the full conversation; this carries goal orchestration
    /// context only.
    fn turn_prompt(&self) -> String {
        let g = &self.goal;
        let mut out = format!(
            "You are working on a multi-turn goal. Continue from the current repository state; \
do not restart from scratch.\n\nGOAL: {}\n\nTURN: {}/{}\n",
            g.description, g.current_turn, g.max_turns
        );
        if !g.verify_commands.is_empty() {
            out.push_str("\nDECLARED VERIFICATION (must pass; run them before finishing):\n");
            for c in &g.verify_commands {
                out.push_str(&format!("  $ {c}\n"));
            }
        }
        if let Some(e) = &g.last_evaluation {
            out.push_str("\nPREVIOUS EVALUATION (goal was NOT complete):\n");
            out.push_str(&format!("  {}\n", e.reason));
            if !e.remaining_work.is_empty() {
                out.push_str("  Remaining work:\n");
                for r in &e.remaining_work {
                    out.push_str(&format!("    - {r}\n"));
                }
            }
        }
        if !g.changed_files.is_empty() {
            out.push_str("\nFiles changed so far:\n");
            for c in &g.changed_files {
                out.push_str(&format!("  {} {}\n", c.status, c.path));
            }
        }
        out.push_str(
            "\nWork autonomously toward this goal: inspect, implement, verify, then report \
what you changed and what verification you ran. Run the declared verification commands \
before you finish.",
        );
        out
    }

    /// Run declared verification commands, emitting a result event per command.
    /// Dangerous commands are blocked (safety boundary, not a bypass).
    async fn run_verification<G: TurnRunner>(
        &self,
        runner: &mut G,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<VerificationResult> {
        let mut out = Vec::new();
        let cwd = runner
            .repo_root()
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        for command in &self.goal.verify_commands {
            let result = if is_dangerous_command(command) {
                VerificationResult {
                    command: command.clone(),
                    exit_code: -1,
                    success: false,
                    output: "blocked: command is not allowed as a verification command".into(),
                }
            } else {
                let res = exec::run("sh", &["-c".into(), command.clone()], &cwd, 300).await;
                let mut output = String::new();
                if !res.stdout.is_empty() {
                    output.push_str(&format!("--- stdout ---\n{}\n", res.stdout));
                }
                if !res.stderr.is_empty() {
                    output.push_str(&format!("--- stderr ---\n{}\n", res.stderr));
                }
                if res.timed_out {
                    output.push_str("(timed out and killed)\n");
                }
                let code = res.code.unwrap_or(-1);
                VerificationResult {
                    command: command.clone(),
                    exit_code: code,
                    success: !res.timed_out && code == 0,
                    output: crate::tools::bound(output),
                }
            };
            crate::log_line!(
                "goal.verify $ {} success={} exit={}",
                result.command,
                result.success,
                result.exit_code
            );
            self.emit(events, GoalUiEvent::Verification(result.clone()))
                .await;
            out.push(result);
        }
        out
    }

    /// Persist the goal snapshot to the attached session (workspace-scoped).
    fn persist<G: TurnRunner>(&self, runner: &mut G) {
        if let Some(session) = runner.session() {
            if let Ok(json) = serde_json::to_string_pretty(&self.goal) {
                let _ = session.save_goal_json(&json);
            }
        }
    }

    async fn emit(&self, events: &mpsc::Sender<AgentEvent>, ev: GoalUiEvent) {
        let _ = events.send(AgentEvent::Goal(ev)).await;
    }
}

/// Collect changed files via `git status --short` (non-git repos: empty).
async fn current_changes(root: Option<PathBuf>) -> Vec<FileChange> {
    let Some(root) = root else {
        return Vec::new();
    };
    let cwd = root.to_string_lossy().into_owned();
    let res = exec::run(
        "git",
        &["status".into(), "--short".into(), "--porcelain".into()],
        &cwd,
        30,
    )
    .await;
    if res.code != Some(0) {
        return Vec::new();
    }
    res.stdout
        .lines()
        .filter_map(parse_status_line)
        .collect()
}

/// Parse one `git status --short` line into a `FileChange`.
/// Format: two status columns (`XY`), a space, then the path.
fn parse_status_line(line: &str) -> Option<FileChange> {
    let bytes = line.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let x = bytes[0] as char;
    let y = bytes[1] as char;
    let path = line[3..].trim().to_string();
    if path.is_empty() {
        return None;
    }
    let st = if y == 'D' || x == 'D' {
        "D"
    } else if y == 'M' || x == 'M' {
        "M"
    } else if x == 'A' || x == '?' || y == '?' {
        "A"
    } else if x == 'R' {
        "R"
    } else {
        "M"
    };
    Some(FileChange {
        status: st.to_string(),
        path,
    })
}

/// A bounded tail of recent tool results, for the evaluator's evidence.
async fn history_excerpt<G: TurnRunner>(_runner: &mut G) -> String {
    // The agent's own history is the source of tool results, but TurnRunner
    // cannot expose it without cloning; the turn summary + verification
    // results already carry the evidence. Kept for future enrichment.
    String::new()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(completed: bool, reason: &str, remaining: &[&str]) -> GoalEvaluation {
        GoalEvaluation {
            completed,
            reason: reason.into(),
            remaining_work: remaining.iter().map(|s| s.to_string()).collect(),
            evidence: Vec::new(),
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        cancelled: bool,
        session: Option<Session>,
        root: Option<PathBuf>,
        /// (result, is_cancelled_after) per call; errors stored as strings.
        results: Vec<(Result<String, String>, bool)>,
        next: usize,
        prompts: Vec<String>,
    }

    impl FakeRunner {}

    #[async_trait::async_trait]
    impl TurnRunner for FakeRunner {
        async fn run_turn(&mut self, prompt: &str) -> Result<String> {
            self.prompts.push(prompt.to_string());
            let (r, cancelled) = self
                .results
                .get(self.next)
                .cloned()
                .unwrap_or((Ok("ok".into()), false));
            self.next += 1;
            self.cancelled = cancelled;
            r.map_err(|e| anyhow!(e))
        }
        fn is_cancelled(&self) -> bool {
            self.cancelled
        }
        fn session(&self) -> Option<&Session> {
            self.session.as_ref()
        }
        fn repo_root(&self) -> Option<PathBuf> {
            self.root.clone()
        }
    }

    #[derive(Clone)]
    struct FixedEvaluator(GoalEvaluation);

    #[async_trait::async_trait]
    impl GoalEvaluator for FixedEvaluator {
        async fn evaluate(&self, _goal: &Goal, _ctx: &GoalContext) -> Result<GoalEvaluation> {
            Ok(self.0.clone())
        }
    }

    /// Returns each configured evaluation in order, then repeats the last.
    struct SeqEvaluator {
        evals: Vec<GoalEvaluation>,
        idx: std::sync::atomic::AtomicUsize,
    }

    impl SeqEvaluator {
        fn new(evals: Vec<GoalEvaluation>) -> Self {
            SeqEvaluator {
                evals,
                idx: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl GoalEvaluator for SeqEvaluator {
        async fn evaluate(&self, _goal: &Goal, _ctx: &GoalContext) -> Result<GoalEvaluation> {
            let i = self.idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let i = i.min(self.evals.len().saturating_sub(1));
            Ok(self.evals[i].clone())
        }
    }

    fn manager(verify: Vec<&str>, max_turns: u32) -> GoalManager {
        let spec = GoalSpec {
            description: "fix tests".into(),
            verify_commands: verify.into_iter().map(|s| s.to_string()).collect(),
            max_turns,
        };
        GoalManager::new(
            spec.to_goal(),
            Box::new(FixedEvaluator(eval(false, "not yet", &["one thing"]))),
        )
    }

    fn channel() -> mpsc::Sender<AgentEvent> {
        let (tx, _rx) = mpsc::channel(64);
        tx
    }

    #[tokio::test]
    async fn parse_goal_extracts_verify_and_max_turns() {
        let spec = parse_goal(
            "fix all failing tests\nverify:\n  npm test\n  npm run lint\nmax_turns: 8",
            10,
        );
        assert_eq!(spec.description, "fix all failing tests");
        assert_eq!(spec.verify_commands, vec!["npm test", "npm run lint"]);
        assert_eq!(spec.max_turns, 8);
    }

    #[tokio::test]
    async fn parse_goal_inline_verify() {
        let spec = parse_goal("make backend compile\nverify: npm run build", 10);
        assert_eq!(spec.verify_commands, vec!["npm run build"]);
        assert_eq!(spec.max_turns, 10);
    }

    #[tokio::test]
    async fn lifecycle_pending_to_completed() {
        let mut m = manager(vec![], 5);
        m.evaluator = Box::new(SeqEvaluator::new(vec![
            eval(false, "still working", &["auth tests"]),
            eval(true, "all tests pass", &[]),
        ]));
        let mut r = FakeRunner {
            results: vec![
                (Ok("did it".into()), false),
                (Ok("done".into()), false),
            ],
            ..Default::default()
        };
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::Completed);
        assert_eq!(m.goal.current_turn, 2);
        assert!(m.goal.last_evaluation.as_ref().unwrap().completed);
        assert_eq!(r.prompts.len(), 2);
        assert!(r.prompts[0].contains("GOAL: fix tests"));
        // Second turn receives the previous evaluation feedback.
        assert!(r.prompts[1].contains("PREVIOUS EVALUATION"));
    }

    #[tokio::test]
    async fn incomplete_goal_runs_next_turn_with_feedback() {
        let mut m = manager(vec![], 5);
        // Each turn reports a DIFFERENT remaining item (progress), so the loop
        // keeps going instead of tripping the repeated-failure guard.
        m.evaluator = Box::new(SeqEvaluator::new(vec![
            eval(false, "5 items left", &["item 5"]),
            eval(false, "4 items left", &["item 4"]),
            eval(false, "3 items left", &["item 3"]),
            eval(false, "2 items left", &["item 2"]),
            eval(false, "1 item left", &["item 1"]),
        ]));
        let mut r = FakeRunner {
            results: vec![
                (Ok("a".into()), false),
                (Ok("b".into()), false),
                (Ok("c".into()), false),
            ],
            ..Default::default()
        };
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::MaxTurnsReached);
        assert_eq!(m.goal.current_turn, 5);
        assert_eq!(r.prompts.len(), 5);
        assert!(r.prompts[3].contains("PREVIOUS EVALUATION"));
    }

    #[tokio::test]
    async fn max_turns_reached() {
        let mut m = manager(vec![], 2);
        let mut r = FakeRunner {
            results: vec![(Ok("x".into()), false), (Ok("y".into()), false)],
            ..Default::default()
        };
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::MaxTurnsReached);
        assert_eq!(m.goal.message, "Maximum turns reached: 2");
    }

    #[tokio::test]
    async fn cancellation_stops_loop() {
        let mut m = manager(vec![], 5);
        let mut r = FakeRunner {
            results: vec![
                (Ok("a".into()), false),
                (Ok("b".into()), true),
            ],
            ..Default::default()
        };
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::Cancelled);
        assert_eq!(m.goal.current_turn, 2);
    }

    #[tokio::test]
    async fn evaluator_error_does_not_corrupt_state() {
        let mut m = manager(vec![], 5);
        let mut r = FakeRunner {
            results: vec![(Ok("a".into()), false)],
            ..Default::default()
        };
        struct Failing;
        #[async_trait::async_trait]
        impl GoalEvaluator for Failing {
            async fn evaluate(&self, _g: &Goal, _c: &GoalContext) -> Result<GoalEvaluation> {
                Err(anyhow!("boom"))
            }
        }
        m.evaluator = Box::new(Failing);
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        // Error becomes an incomplete evaluation; loop keeps running, state intact.
        assert_eq!(status, GoalStatus::MaxTurnsReached);
        assert_eq!(m.goal.status, GoalStatus::MaxTurnsReached);
        assert!(m.goal.last_evaluation.is_some());
        assert!(m.goal.last_evaluation.as_ref().unwrap().reason.contains("evaluator error"));
    }

    #[tokio::test]
    async fn loop_prevention_stops_repeated_failures() {
        let mut m = manager(vec![], 10);
        let mut r = FakeRunner {
            results: vec![
                (Ok("a".into()), false),
                (Ok("b".into()), false),
                (Ok("c".into()), false),
            ],
            ..Default::default()
        };
        // Every evaluation reports the same remaining work → same fingerprint.
        m.evaluator = Box::new(FixedEvaluator(eval(
            false,
            "same failure",
            &["payment service test still failing"],
        )));
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::Failed);
        assert!(m.goal.message.contains("No progress"));
        assert_eq!(m.goal.current_turn, 3);
    }

    #[tokio::test]
    async fn command_verification_uses_real_exit_codes() {
        let mut m = manager(vec![], 3);
        // Real shell: `true` exits 0, `false` exits 1.
        let mut r = FakeRunner {
            results: vec![(Ok("x".into()), false)],
            ..Default::default()
        };
        m.goal.verify_commands = vec!["true".into(), "false".into()];
        m.evaluator = Box::new(FixedEvaluator(eval(true, "done", &[])));
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        // Hard verification gate: `false` fails → goal stays incomplete.
        assert_ne!(status, GoalStatus::Completed);
        let evals = m.goal.last_evaluation;
        assert!(evals.is_some());
    }

    #[tokio::test]
    async fn changed_files_parsed_from_status_line() {
        let c = parse_status_line(" M src/auth/service.rs").unwrap();
        assert_eq!(c.status, "M");
        assert_eq!(c.path, "src/auth/service.rs");
        let a = parse_status_line("?? new.txt").unwrap();
        assert_eq!(a.status, "A");
        let d = parse_status_line(" D gone.rs").unwrap();
        assert_eq!(d.status, "D");
    }

    #[test]
    fn parse_evaluation_json_tolerates_fences() {
        let ev = parse_evaluation_json(
            "Here is the result:\n```json\n{\"completed\": true, \"reason\": \"all pass\", \
             \"remaining_work\": [], \"evidence\": [\"npm test exit 0\"]}\n```",
        )
        .unwrap();
        assert!(ev.completed);
        assert_eq!(ev.reason, "all pass");
        assert_eq!(ev.evidence, vec!["npm test exit 0"]);
    }

    #[tokio::test]
    async fn goal_persists_to_session_and_roundtrips() {
        let _guard = crate::session::storage::tests::ENV_LOCK.lock().unwrap();
        let base =
            std::env::temp_dir().join(format!("lightcode_goal_persist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::env::set_var("LIGHTCODE_DATA_DIR", &base);

        let s = crate::session::storage::create().unwrap();
        let goal = Goal::new(
            "fix all failing tests".into(),
            vec!["npm test".into()],
            5,
        );
        let json = serde_json::to_string(&goal).unwrap();
        s.save_goal_json(&json).unwrap();

        let loaded: Goal =
            serde_json::from_str(&s.load_goal_json().unwrap().unwrap()).unwrap();
        assert_eq!(loaded.description, "fix all failing tests");
        assert_eq!(loaded.verify_commands, vec!["npm test"]);
        assert_eq!(loaded.max_turns, 5);
        assert_eq!(loaded.status, GoalStatus::Pending);

        // The snapshot lives in THIS session's workspace dir, not a global path.
        let found = crate::session::storage::sessions_dir()
            .join("workspaces")
            .read_dir()
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .any(|e| e.path().join(format!("{}.goal.json", s.id)).is_file());
        assert!(found, "goal file exists in a workspace-scoped dir");

        std::env::remove_var("LIGHTCODE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn manager_persists_status_through_run() {
        let _guard = crate::session::storage::tests::ENV_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("lightcode_goal_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::env::set_var("LIGHTCODE_DATA_DIR", &base);

        let s = crate::session::storage::create().unwrap();
        let mut m = manager(vec![], 5);
        m.evaluator = Box::new(SeqEvaluator::new(vec![
            eval(false, "still working", &["x"]),
            eval(true, "done", &[]),
        ]));
        let mut r = FakeRunner {
            results: vec![(Ok("a".into()), false), (Ok("b".into()), false)],
            session: Some(s.clone()),
            ..Default::default()
        };
        let tx = channel();
        let status = m.run(&mut r, &tx).await;
        assert_eq!(status, GoalStatus::Completed);

        let loaded: Goal = serde_json::from_str(&s.load_goal_json().unwrap().unwrap()).unwrap();
        assert_eq!(loaded.status, GoalStatus::Completed);
        assert_eq!(loaded.current_turn, 2);
        assert_eq!(loaded.description, "fix tests");

        std::env::remove_var("LIGHTCODE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&base);
    }

    struct FixedTextProvider(String);

    #[async_trait::async_trait]
    impl crate::providers::Provider for FixedTextProvider {
        async fn complete_stream(
            &self,
            _messages: &[crate::providers::Message],
            _tools: &[crate::providers::ToolDef],
        ) -> std::result::Result<tokio::sync::mpsc::Receiver<crate::providers::StreamEvent>, crate::providers::ProviderError>
        {
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            let text = self.0.clone();
            tokio::spawn(async move {
                let _ = tx.send(crate::providers::StreamEvent::Text(text)).await;
                let _ = tx.send(crate::providers::StreamEvent::Done).await;
            });
            Ok(rx)
        }
        fn set_model(&mut self, _m: &str) {}
        fn clone_box(&self) -> Box<dyn crate::providers::Provider> {
            Box::new(FixedTextProvider(self.0.clone()))
        }
    }

    #[tokio::test]
    async fn model_evaluator_parses_structured_output() {
        let provider = Box::new(FixedTextProvider(
            r#"{"completed": false, "reason": "3 tests failing", "remaining_work": ["payment", "refresh"], "evidence": ["npm test exit 1"]}"#.into(),
        ));
        let evaluator = ModelGoalEvaluator { provider };
        let goal = Goal::new("fix tests".into(), vec!["npm test".into()], 3);
        let ctx = GoalContext {
            turn_summary: "fixed some".into(),
            verification: vec![VerificationResult {
                command: "npm test".into(),
                exit_code: 1,
                success: false,
                output: "fail".into(),
            }],
            history_excerpt: String::new(),
        };
        let ev = evaluator.evaluate(&goal, &ctx).await.unwrap();
        assert!(!ev.completed);
        assert_eq!(ev.remaining_work, vec!["payment", "refresh"]);
        assert!(ev.reason.contains("3 tests"));
    }

    #[tokio::test]
    async fn model_evaluator_rejects_freeform_output() {
        let provider = Box::new(FixedTextProvider("Done! Tests should pass.".into()));
        let evaluator = ModelGoalEvaluator { provider };
        let goal = Goal::new("fix tests".into(), vec![], 3);
        let ctx = GoalContext {
            turn_summary: "done".into(),
            verification: vec![],
            history_excerpt: String::new(),
        };
        // Free-form text is NOT accepted as evidence of completion.
        assert!(evaluator.evaluate(&goal, &ctx).await.is_err());
    }
}

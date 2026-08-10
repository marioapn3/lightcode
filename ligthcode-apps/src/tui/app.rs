use crate::agent::AgentMode;
use crate::permissions::Choice;
use crate::providers::Message;
use crate::tui::editor::{EditorAction, TextEditor};
use serde_json::Value;
use tokio::sync::oneshot;

/// Pastes larger than this are stored as a compact placeholder line instead of
/// being flattened into the visible buffer.
const PASTE_DIRECT_MAX_LINES: usize = 20;
const PASTE_DIRECT_MAX_CHARS: usize = 2000;

/// A paste placeholder is a line that collapses a large pasted block.
pub struct PasteEntry {
    pub placeholder: String,
    pub content: String,
}

/// Open @-mention autocomplete state.
pub struct MentionPicker {
    pub query: String,
    /// Byte range in the composer text that the selected result replaces.
    pub start: usize,
    pub end: usize,
    pub results: Vec<crate::files::Match>,
    pub selected: usize,
}

/// A selectable model from any configured provider.
#[derive(Clone)]
pub struct ModelItem {
    pub provider: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone)]
pub struct StatusInfo {
    pub model: String,
    pub provider: String,
    pub session: String,
    pub cwd: String,
    /// Resolved workspace identity (git root or normalized cwd).
    pub workspace: String,
    pub models: Vec<ModelItem>,
    /// Persisted prompt history (most recent first).
    pub history: Vec<String>,
    /// Named agents `(name, model)` from config.
    pub agents: Vec<(String, String)>,
    /// Agent mode restored from the session, if any.
    pub mode: AgentMode,
}

/// Open agent-mode selector.
pub struct ModePicker {
    pub modes: Vec<(AgentMode, &'static str)>,
    pub selected: usize,
}

/// Open theme selector.
pub struct ThemePicker {
    pub themes: Vec<String>,
    pub selected: usize,
}

pub struct ModelPicker {
    pub models: Vec<ModelItem>,
    pub selected: usize,
}

pub struct AgentPicker {
    /// `(name, model)` for each named agent, plus "default".
    pub agents: Vec<(String, String)>,
    pub selected: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToolState {
    Running,
    Success,
    Failed,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToolKind {
    Read,
    Grep,
    List,
    Write,
    Edit,
    Shell,
    Git,
    Fetch,
    Search,
    Context,
    Other,
}

impl ToolKind {
    pub fn from_name(name: &str) -> ToolKind {
        match name {
            "read_file" => ToolKind::Read,
            "grep" => ToolKind::Grep,
            "list_directory" => ToolKind::List,
            "write_file" => ToolKind::Write,
            "edit_file" => ToolKind::Edit,
            "shell" => ToolKind::Shell,
            "git_diff" | "git_status" | "git_log" => ToolKind::Git,
            "web_fetch" => ToolKind::Fetch,
            "web_search" => ToolKind::Search,
            "context" => ToolKind::Context,
            _ => ToolKind::Other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ToolKind::Read => "read",
            ToolKind::Grep => "grep",
            ToolKind::List => "ls",
            ToolKind::Write => "write",
            ToolKind::Edit => "edit",
            ToolKind::Shell => "shell",
            ToolKind::Git => "git",
            ToolKind::Fetch => "fetch",
            ToolKind::Search => "search",
            ToolKind::Context => "context",
            ToolKind::Other => "tool",
        }
    }
}

/// The human-readable target of a tool call, extracted from its JSON arguments.
pub fn tool_target(name: &str, args: &Value) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| args.get(*k).and_then(|v| v.as_str()))
            .unwrap_or("")
    };
    match ToolKind::from_name(name) {
        ToolKind::Read | ToolKind::Write | ToolKind::Edit | ToolKind::List => {
            pick(&["path"]).to_string()
        }
        ToolKind::Grep => pick(&["pattern"]).to_string(),
        ToolKind::Shell => pick(&["command"]).to_string(),
        ToolKind::Fetch => pick(&["url"]).to_string(),
        ToolKind::Search => pick(&["query"]).to_string(),
        _ => String::new(),
    }
}

pub struct ToolBlock {
    pub kind: ToolKind,
    pub target: String,
    pub state: ToolState,
    pub output: String,
}

/// High-level phase of a run of related tool activity.
#[derive(Clone, Copy, PartialEq)]
pub enum ActivityPhase {
    Investigating,
    Modifying,
    Running,
}

impl ActivityPhase {
    pub fn label(&self) -> &'static str {
        match self {
            ActivityPhase::Investigating => "Investigating",
            ActivityPhase::Modifying => "Modifying files",
            ActivityPhase::Running => "Running",
        }
    }

    pub fn done_label(&self) -> &'static str {
        match self {
            ActivityPhase::Investigating => "Investigated",
            ActivityPhase::Modifying => "Modified",
            ActivityPhase::Running => "Ran",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ActivityStatus {
    Running,
    Success,
    Failed,
}

/// One tool invocation inside an activity group.
pub struct ActivityItem {
    pub name: String,
    pub kind: ToolKind,
    pub target: String,
    pub status: ActivityStatus,
    pub output: String,
}

/// A group of related tool activity (e.g. a repository investigation).
pub struct ActivityBlock {
    pub phase: ActivityPhase,
    pub items: Vec<ActivityItem>,
    pub done: bool,
}

/// Structured conversation block. The renderer owns per-type drawing.
pub enum UiBlock {
    User(String),
    Assistant {
        text: String,
    },
    /// Compact reasoning status: shown as "◌ Thinking..." while streaming and
    /// "✓ Thought for Xs" once done. Raw reasoning text is never shown.
    Reasoning {
        started: std::time::Instant,
        done: Option<std::time::Duration>,
    },
    Tool(ToolBlock),
    /// A group of related tool activity (e.g. a repository investigation).
    Activity(ActivityBlock),
    Diff {
        file: String,
        body: String,
    },
    Error(String),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Command {
    NewSession,
    Sessions,
    Models,
    Agent,
    Themes,
    Status,
    Stats,
    Help,
    Clear,
    Debug,
    Quit,
}

pub struct CommandPalette {
    pub commands: Vec<Command>,
    pub selected: usize,
    pub filter: String,
}

pub struct DiffViewer {
    pub body: String,
    pub scroll: usize,
}

#[derive(Clone)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Clone)]
pub struct SessionPicker {
    pub sessions: Vec<SessionItem>,
    pub selected: usize,
    pub filter: String,
}

pub struct PermissionRequest {
    pub prompt: String,
    pub respond: Option<oneshot::Sender<Choice>>,
    /// When true, keystrokes edit a feedback message; Enter denies with it.
    pub entering_feedback: bool,
    pub feedback: String,
}

pub struct QuestionRequest {
    pub prompt: String,
    pub options: Vec<String>,
    pub selected: usize,
    pub respond: Option<oneshot::Sender<Option<String>>>,
}

pub struct App {
    pub content: Vec<UiBlock>,
    pub input: TextEditor,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub draft: String,
    pub busy: bool,
    pub scroll: usize,
    pub auto_scroll: bool,
    pub show_tool_output: bool,
    pub pending_permission: Option<PermissionRequest>,
    pub pending_question: Option<QuestionRequest>,
    pub model_picker: Option<ModelPicker>,
    pub agent_picker: Option<AgentPicker>,
    pub command_palette: Option<CommandPalette>,
    pub session_picker: Option<SessionPicker>,
    pub diff_viewer: Option<DiffViewer>,
    pub leader_active: bool,
    pub suggestions: Vec<String>,
    pub pending: usize,
    pub status: StatusInfo,
    pub spinner: usize,
    pub quit: bool,
    pub last_error: Option<String>,
    pub toast: Option<(String, std::time::Instant)>,
    pub pastes: Vec<PasteEntry>,
    /// Index of the timeline item the user has selected (Ctrl+↑/↓).
    pub selected: Option<usize>,
    /// Timeline item indices explicitly expanded by the user.
    pub expanded: std::collections::HashSet<usize>,
    /// Cached repository file index for @-mentions.
    pub file_index: Option<crate::files::FileIndex>,
    pub mention_picker: Option<MentionPicker>,
    pub mode: AgentMode,
    pub mode_picker: Option<ModePicker>,
    pub theme_picker: Option<ThemePicker>,
    /// Mouse drag selection in content-area coordinates `(row, col)`.
    pub mouse_sel: Option<crate::tui::select::Selection>,
    pub mouse_dragging: bool,
    /// Content-area row under the mouse pointer (for the active-line highlight).
    pub mouse_hover: Option<usize>,
    /// Content area geometry + last scroll, set each frame for mouse mapping.
    pub content_area: ratatui::layout::Rect,
    pub content_scroll: usize,
    /// Per-timeline-item terminal row ranges `(start, end)`, set each frame.
    pub item_ranges: Vec<(usize, usize)>,
}

impl App {
    pub fn new(status: StatusInfo) -> Self {
        Self {
            content: Vec::new(),
            input: TextEditor::new(),
            history: status.history.clone(),
            history_idx: None,
            draft: String::new(),
            busy: false,
            scroll: 0,
            auto_scroll: true,
            show_tool_output: false,
            pending_permission: None,
            pending_question: None,
            model_picker: None,
            agent_picker: None,
            command_palette: None,
            session_picker: None,
            diff_viewer: None,
            leader_active: false,
            suggestions: Vec::new(),
            pending: 0,
            mode: status.mode,
            status,
            spinner: 0,
            quit: false,
            last_error: None,
            toast: None,
            pastes: Vec::new(),
            selected: None,
            expanded: std::collections::HashSet::new(),
            file_index: None,
            mention_picker: None,
            mode_picker: None,
            theme_picker: None,
            mouse_sel: None,
            mouse_dragging: false,
            mouse_hover: None,
            content_area: ratatui::layout::Rect::default(),
            content_scroll: 0,
            item_ranges: Vec::new(),
        }
    }

    /// Cycle to the next mode (PLAN → BUILD → AUTO → PLAN).
    pub fn cycle_mode(&mut self) -> AgentMode {
        self.mode = self.mode.next();
        self.mode
    }

    pub fn set_mode(&mut self, mode: AgentMode) {
        self.mode = mode;
        self.mode_picker = None;
    }

    pub fn open_mode_picker(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        self.mode_picker = Some(ModePicker {
            modes: vec![
                (AgentMode::Plan, AgentMode::Plan.description()),
                (AgentMode::Build, AgentMode::Build.description()),
                (AgentMode::Auto, AgentMode::Auto.description()),
            ],
            selected: 0,
        });
    }

    pub fn open_theme_picker(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        let themes: Vec<String> = crate::tui::theme::Theme::all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let selected = themes
            .iter()
            .position(|n| n == crate::tui::theme::Theme::current().name)
            .unwrap_or(0);
        self.theme_picker = Some(ThemePicker { themes, selected });
    }

    /// Refresh the @-mention picker from the composer's current text.
    pub fn update_mentions(&mut self) {
        let text = self.input.text();
        let cursor = self.input.cursor_byte_offset();
        match crate::mentions::mention_at_cursor(&text, cursor) {
            Some(m) => {
                if self.file_index.is_none() {
                    let cwd = std::path::PathBuf::from(&self.status.cwd);
                    self.file_index = Some(crate::files::FileIndex::build(&cwd));
                }
                let results = self
                    .file_index
                    .as_ref()
                    .map(|idx| idx.query(&m.path, 20))
                    .unwrap_or_default();
                self.mention_picker = Some(MentionPicker {
                    query: m.path.clone(),
                    start: m.start,
                    end: m.end,
                    results,
                    selected: 0,
                });
            }
            None => self.mention_picker = None,
        }
    }

    /// Replace the current mention with the selected result and close the picker.
    pub fn select_mention(&mut self) -> bool {
        if let Some(p) = self.mention_picker.take() {
            if let Some(m) = p.results.get(p.selected) {
                let replacement = format!("@{}", m.path);
                self.input.replace_range(p.start, p.end, &replacement);
                self.mention_picker = None;
                return true;
            }
        }
        false
    }

    /// Start or continue a reasoning status block; returns true when a new one
    /// was pushed (reasoning just began).
    pub fn begin_or_continue_reasoning(&mut self) -> bool {
        if matches!(
            self.content.last_mut(),
            Some(UiBlock::Reasoning { done: None, .. })
        ) {
            return false;
        }
        self.content.push(UiBlock::Reasoning {
            started: std::time::Instant::now(),
            done: None,
        });
        true
    }

    /// Mark every open reasoning block as finished (called when the model moves
    /// on to text, a tool call, or completion).
    pub fn finalize_reasoning(&mut self) {
        for item in self.content.iter_mut() {
            if let UiBlock::Reasoning { started, done } = item {
                if done.is_none() {
                    *done = Some(std::time::Instant::now().saturating_duration_since(*started));
                }
            }
        }
    }

    /// True when the most recent item is an unfinished reasoning block.
    /// Is a timeline item expanded? (user-toggle or global "show all".)
    pub fn item_expanded(&self, index: usize) -> bool {
        self.show_tool_output || self.expanded.contains(&index)
    }

    /// Move timeline selection to the previous/next collapsible item.
    pub fn select_prev(&mut self) {
        self.step_selection(-1);
    }

    pub fn select_next(&mut self) {
        self.step_selection(1);
    }

    /// Expand/collapse the selected timeline item (or the given index).
    pub fn toggle_expanded(&mut self, index: usize) {
        if !self.expanded.insert(index) {
            self.expanded.remove(&index);
        }
    }

    /// Mark the current activity group as finished (once the phase changes or a
    /// new block is pushed after it).
    pub fn finalize_activity(&mut self) {
        if let Some(UiBlock::Activity(a)) = self.content.last_mut() {
            a.done = true;
        }
    }

    /// Which activity phase a tool belongs to.
    pub fn phase_for_tool(name: &str) -> ActivityPhase {
        match name {
            "write_file" | "edit_file" | "apply_patch" => ActivityPhase::Modifying,
            "shell" | "task" | "todowrite" | "question" => ActivityPhase::Running,
            _ => ActivityPhase::Investigating,
        }
    }

    /// Plain text of a conversation block, ready to copy.
    pub fn block_text(block: &UiBlock) -> String {
        match block {
            UiBlock::User(t) => t.clone(),
            UiBlock::Assistant { text } => text.clone(),
            UiBlock::Reasoning { .. } => String::new(),
            UiBlock::Tool(tb) => {
                if tb.output.is_empty() {
                    tb.target.clone()
                } else {
                    format!("{}\n{}", tb.target, tb.output.trim_end())
                }
            }
            UiBlock::Activity(a) => {
                let mut out = String::new();
                for item in &a.items {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&item.target);
                    if !item.output.is_empty() {
                        out.push('\n');
                        out.push_str(item.output.trim_end());
                    }
                }
                out
            }
            UiBlock::Diff { body, .. } => body.clone(),
            UiBlock::Error(t) => t.clone(),
        }
    }

    /// The text to copy: the selected timeline item, else the most recent
    /// assistant message, else the last non-empty block.
    pub fn copy_target_text(&self) -> String {
        if let Some(i) = self.selected {
            if let Some(block) = self.content.get(i) {
                let t = App::block_text(block);
                if !t.is_empty() {
                    return t;
                }
            }
        }
        for block in self.content.iter().rev() {
            match block {
                UiBlock::Assistant { text } if !text.is_empty() => return text.clone(),
                _ => {}
            }
        }
        for block in self.content.iter().rev() {
            let t = App::block_text(block);
            if !t.is_empty() {
                return t;
            }
        }
        String::new()
    }

    /// Extract the current mouse-drag selection from the rendered content.
    pub fn copy_mouse_selection(&self) -> Option<String> {
        let sel = self.mouse_sel.as_ref()?;
        let w = self.content_area.width as usize;
        if w == 0 {
            return None;
        }
        let lines = crate::tui::render::build_lines(self, w);
        let text = crate::tui::select::extract_text(&lines, w, self.content_scroll, sel);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn step_selection(&mut self, dir: isize) {
        if self.content.is_empty() {
            return;
        }
        let mut cur = match self.selected {
            Some(i) => i as isize,
            None if dir > 0 => -1,
            None => self.content.len() as isize,
        };
        loop {
            cur += dir;
            if cur < 0 || cur as usize >= self.content.len() {
                self.selected = None;
                return;
            }
            let selectable = matches!(
                self.content[cur as usize],
                UiBlock::Tool(_)
                    | UiBlock::Activity(_)
                    | UiBlock::Diff { .. }
                    | UiBlock::Reasoning { .. }
            );
            if selectable {
                self.selected = Some(cur as usize);
                return;
            }
        }
    }

    /// Handle a terminal bracketed-paste payload as one logical operation.
    /// Small pastes insert directly; large ones become a compact placeholder.
    pub fn handle_paste(&mut self, text: &str) {
        // Normalize CRLF (common in copied Windows content).
        let text = text.replace("\r\n", "\n");
        let lines = text.lines().count();
        let chars = text.chars().count();
        if lines <= PASTE_DIRECT_MAX_LINES && chars <= PASTE_DIRECT_MAX_CHARS {
            self.input.apply(EditorAction::Paste(text));
        } else {
            // Start the placeholder on its own line when pasting mid-line.
            if self.input.cursor().col > 0 {
                self.input.apply(EditorAction::InsertNewline);
            }
            let base = format!(
                "[Pasted text · {} lines · {} chars]",
                lines,
                human_count(chars)
            );
            let placeholder = unique_placeholder(&base, &self.pastes);
            self.pastes.push(PasteEntry {
                placeholder: placeholder.clone(),
                content: text,
            });
            self.input.apply(EditorAction::Paste(placeholder));
            self.show_toast("Pasted content collapsed. Alt+P to expand.");
        }
        self.update_suggestions();
    }

    /// Expand a paste placeholder under the cursor into its full content.
    pub fn expand_paste_at_cursor(&mut self) -> bool {
        let row = self.input.cursor().row;
        let line: String = self.input.line_graphemes(row).concat();
        if let Some(entry) = self.pastes.iter().find(|p| p.placeholder == line) {
            let content = entry.content.clone();
            self.input.replace_current_line(&content);
            self.update_suggestions();
            return true;
        }
        false
    }

    /// True if the given line is a paste placeholder (for styling).
    pub fn is_paste_placeholder(&self, line: &str) -> bool {
        self.pastes.iter().any(|p| p.placeholder == line)
    }

    /// Show a transient toast for ~4 seconds.
    pub fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    pub fn push(&mut self, item: UiBlock) {
        if !self.auto_scroll {
            self.pending += 1;
        }
        self.content.push(item);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.pending = 0;
    }

    /// Take the typed input as a prompt and mark the agent as busy.
    pub fn submit(&mut self) -> Option<String> {
        let text = self.input.text();
        let expanded = expand_pastes(&text, &self.pastes);
        let trimmed = expanded.trim();
        if trimmed.is_empty() {
            return None;
        }
        self.history.retain(|h| h != trimmed);
        self.history.insert(0, trimmed.to_string());
        self.history_idx = None;
        self.draft.clear();
        crate::history::push(trimmed);
        self.input.clear();
        self.suggestions.clear();
        self.pastes.clear();
        self.mention_picker = None;
        self.push(UiBlock::User(trimmed.to_string()));
        let mentions = crate::mentions::parse_mentions(trimmed);
        if !mentions.is_empty() {
            let target = mentions
                .iter()
                .map(|m| format!("@{}", m.path))
                .collect::<Vec<_>>()
                .join("  ");
            self.push(UiBlock::Tool(ToolBlock {
                kind: ToolKind::Other,
                target,
                state: ToolState::Success,
                output: String::new(),
            }));
        }
        self.busy = true;
        self.last_error = None;
        Some(trimmed.to_string())
    }

    pub fn clear_view(&mut self) {
        self.content.clear();
        self.input.clear();
        self.suggestions.clear();
        self.pastes.clear();
        self.mention_picker = None;
        self.selected = None;
        self.expanded.clear();
        self.scroll = 0;
        self.auto_scroll = true;
    }

    pub fn open_model_picker(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        if self.status.models.is_empty() {
            self.model_picker = Some(ModelPicker {
                models: vec![ModelItem {
                    provider: self.status.provider.clone(),
                    id: self.status.model.clone(),
                    name: self.status.model.clone(),
                }],
                selected: 0,
            });
        } else {
            self.model_picker = Some(ModelPicker {
                models: self.status.models.clone(),
                selected: 0,
            });
        }
    }

    pub fn open_command_palette(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        self.command_palette = Some(CommandPalette {
            commands: vec![
                Command::NewSession,
                Command::Sessions,
                Command::Models,
                Command::Agent,
                Command::Themes,
                Command::Status,
                Command::Stats,
                Command::Help,
                Command::Clear,
                Command::Debug,
                Command::Quit,
            ],
            selected: 0,
            filter: String::new(),
        });
    }

    pub fn open_agent_picker(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        let mut agents: Vec<(String, String)> = self
            .status
            .agents
            .iter()
            .map(|(n, m)| (n.clone(), m.clone()))
            .collect();
        agents.insert(0, ("default".to_string(), String::new()));
        self.agent_picker = Some(AgentPicker {
            agents,
            selected: 0,
        });
    }

    pub fn command_label(cmd: Command) -> &'static str {
        match cmd {
            Command::NewSession => "new session   mulai sesi baru",
            Command::Sessions => "sessions      daftar & pindah sesi",
            Command::Models => "models        ganti model",
            Command::Agent => "agent         ganti mode/agent",
            Command::Themes => "themes        ganti tema warna",
            Command::Status => "status        info sesi / workspace",
            Command::Stats => "stats         hitungan pesan & token",
            Command::Help => "help          pintasan & slash command",
            Command::Clear => "clear         bersihkan layar",
            Command::Debug => "debug         info sesi",
            Command::Quit => "quit          keluar",
        }
    }

    pub fn command_filter_text(cmd: Command) -> &'static str {
        match cmd {
            Command::NewSession => "new session",
            Command::Sessions => "sessions",
            Command::Models => "models",
            Command::Agent => "agent",
            Command::Themes => "themes",
            Command::Status => "status",
            Command::Stats => "stats",
            Command::Help => "help",
            Command::Clear => "clear",
            Command::Debug => "debug",
            Command::Quit => "quit",
        }
    }

    /// Filtered commands for the palette, by current filter text.
    pub fn palette_commands(palette: &CommandPalette) -> Vec<Command> {
        let f = palette.filter.to_lowercase();
        if f.is_empty() {
            return palette.commands.clone();
        }
        palette
            .commands
            .iter()
            .copied()
            .filter(|c| App::command_filter_text(*c).contains(&f))
            .collect()
    }

    /// Rebuild the command/model suggestion list from the current input.
    pub fn update_suggestions(&mut self) {
        let text = self.input.text().trim().to_string();
        self.suggestions.clear();

        if text.starts_with('/') {
            for cmd in [
                "/models",
                "/sessions",
                "/new",
                "/clear",
                "/status",
                "/help",
                "/compact",
                "/fork",
                "/rename",
                "/debug",
                "/quit",
                "/exit",
            ] {
                if cmd.starts_with(&text) || text.starts_with(cmd) {
                    self.suggestions.push(format!("{cmd}  <enter>"));
                }
            }
            if text.starts_with("/models") {
                if self.status.models.is_empty() {
                    self.suggestions
                        .push(format!("  model: {}", self.status.model));
                } else {
                    let mut last_provider = String::new();
                    for m in &self.status.models {
                        if m.provider != last_provider {
                            self.suggestions.push(format!("  ── {} ──", m.provider));
                            last_provider = m.provider.clone();
                        }
                        self.suggestions.push(format!("    {}", m.id));
                    }
                }
            }
        }
    }

    pub fn open_session_picker(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        self.session_picker = Some(SessionPicker {
            sessions: self.list_sessions(),
            selected: 0,
            filter: String::new(),
        });
    }

    pub fn list_sessions(&self) -> Vec<SessionItem> {
        crate::session::storage::list()
            .unwrap_or_default()
            .into_iter()
            .map(|m| SessionItem {
                id: m.id,
                title: m.title,
                created_at: m.created_at,
            })
            .collect()
    }

    /// Rebuild the on-screen conversation from persisted messages.
    fn content_from_history(&mut self, messages: &[Message]) {
        self.content.clear();
        self.selected = None;
        self.expanded.clear();
        for m in messages {
            match m {
                Message::User { content } => {
                    self.content.push(UiBlock::User(content.clone()));
                }
                Message::Assistant {
                    content,
                    tool_calls,
                    ..
                } => {
                    if let Some(c) = content {
                        self.content.push(UiBlock::Assistant { text: c.clone() });
                    }
                    if !tool_calls.is_empty() {
                        for tc in tool_calls {
                            self.content.push(UiBlock::Tool(ToolBlock {
                                kind: ToolKind::from_name(&tc.name),
                                target: tool_target(&tc.name, &tc.arguments),
                                state: ToolState::Success,
                                output: String::new(),
                            }));
                        }
                    }
                }
                Message::Tool { content, .. } => {
                    if let Some(UiBlock::Tool(tb)) = self.content.last_mut() {
                        if tb.output.is_empty() {
                            tb.output = content.clone();
                            tb.state = ToolState::Success;
                            continue;
                        }
                    }
                    self.content.push(UiBlock::Tool(ToolBlock {
                        kind: ToolKind::from_name("shell"),
                        target: String::new(),
                        state: ToolState::Success,
                        output: content.clone(),
                    }));
                }
                Message::System { .. } => {}
            }
        }
        self.scroll_to_bottom();
    }

    /// Switch the on-screen view to a saved session.
    pub fn load_session(&mut self, id: &str) -> anyhow::Result<()> {
        let s = crate::session::storage::open(id)?;
        let msgs = s.load_history()?;
        self.content_from_history(&msgs);
        self.status.session = id.to_string();
        Ok(())
    }

    /// Start a fresh session: new id, empty view.
    pub fn new_session(&mut self) -> anyhow::Result<()> {
        let s = crate::session::storage::create()?;
        self.content.clear();
        self.selected = None;
        self.expanded.clear();
        self.status.session = s.id;
        Ok(())
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_idx.is_none() {
            // preserve the current draft so the user can return to it
            self.draft = self.input.text();
        }
        let idx = match self.history_idx {
            Some(i) if i > 0 => i - 1,
            _ => self.history.len() - 1,
        };
        self.history_idx = Some(idx);
        self.input.load_text(&self.history[idx]);
        self.input.scroll_to_cursor(5);
    }

    pub fn history_next(&mut self) {
        match self.history_idx {
            Some(i) if i + 1 < self.history.len() => {
                let idx = i + 1;
                self.history_idx = Some(idx);
                self.input.load_text(&self.history[idx]);
                self.input.scroll_to_cursor(5);
            }
            Some(_) | None => {
                self.history_idx = None;
                self.input.load_text(&self.draft);
                self.input.scroll_to_cursor(5);
            }
        }
    }
}

fn human_count(n: usize) -> String {
    if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn unique_placeholder(base: &str, pastes: &[PasteEntry]) -> String {
    if !pastes.iter().any(|p| p.placeholder == base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} #{n}");
        if !pastes.iter().any(|p| p.placeholder == candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Replace paste placeholder lines with their stored full content.
pub fn expand_pastes(text: &str, pastes: &[PasteEntry]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        match pastes.iter().find(|p| p.placeholder == line) {
            Some(entry) => {
                out.push(&entry.content);
            }
            None => out.push(line),
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;
    use serde_json::json;

    fn temp_session_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lightcode_app_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn load_session_rebuilds_conversation() {
        let _guard = crate::session::storage::tests::ENV_LOCK.lock().unwrap();
        let d = temp_session_dir("loadsession");
        std::env::set_var("LIGHTCODE_DATA_DIR", &d);
        let s = crate::session::storage::create().unwrap();
        s.append(&Message::User {
            content: "hello there".into(),
        })
        .unwrap();
        s.append(&Message::Assistant {
            content: Some("hi".into()),
            reasoning: None,
            tool_calls: vec![],
        })
        .unwrap();
        s.append(&Message::Assistant {
            content: Some("done".into()),
            reasoning: None,
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "shell".into(),
                arguments: json!({"command": "ls"}),
            }],
        })
        .unwrap();
        s.append(&Message::Tool {
            tool_call_id: "c1".into(),
            content: "file.rs".into(),
        })
        .unwrap();

        let mut app = App::new(StatusInfo {
            model: "m".into(),
            provider: "p".into(),
            session: String::new(),
            cwd: ".".into(),
            workspace: ".".into(),
            models: vec![],
            history: vec![],
            agents: vec![],
            mode: crate::agent::AgentMode::Build,
        });
        app.load_session(&s.id).unwrap();
        assert_eq!(app.status.session, s.id);
        assert_eq!(app.content.len(), 4);
        assert!(matches!(&app.content[0], UiBlock::User(t) if t == "hello there"));
        assert!(matches!(&app.content[1], UiBlock::Assistant { text } if text == "hi"));
        assert!(matches!(&app.content[2], UiBlock::Assistant { text } if text == "done"));
        assert!(matches!(&app.content[3], UiBlock::Tool(tb) if tb.output == "file.rs"));
        std::fs::remove_dir_all(&d).ok();
        std::env::remove_var("LIGHTCODE_DATA_DIR");
    }

    fn test_app() -> App {
        App::new(StatusInfo {
            model: "m".into(),
            provider: "p".into(),
            session: String::new(),
            cwd: ".".into(),
            workspace: ".".into(),
            models: vec![],
            history: vec![],
            agents: vec![],
            mode: crate::agent::AgentMode::Build,
        })
    }

    #[test]
    fn small_paste_inserts_directly() {
        let mut app = test_app();
        app.handle_paste("hello world");
        assert_eq!(app.input.text(), "hello world");
        assert!(app.pastes.is_empty());

        // A second paste lands at the cursor.
        app.input
            .apply(crate::tui::editor::EditorAction::MoveToDocEnd);
        app.handle_paste("hello\nworld\nfoo\nbar");
        assert_eq!(app.input.text(), "hello worldhello\nworld\nfoo\nbar");
        assert!(app.pastes.is_empty());
        // Multiline paste is one logical op: undoing removes all of it.
        app.input.apply(crate::tui::editor::EditorAction::Undo);
        assert_eq!(app.input.text(), "hello world");
    }

    #[test]
    fn large_paste_becomes_placeholder() {
        let mut app = test_app();
        let big = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.handle_paste(&big);
        assert_eq!(app.pastes.len(), 1);
        let text = app.input.text();
        assert!(text.contains("[Pasted text · 500 lines ·"));
        assert!(app.is_paste_placeholder(text.trim()));
    }

    #[test]
    fn submit_expands_placeholder_to_full_content() {
        let mut app = test_app();
        let big = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.handle_paste(&big);
        let placeholder = app.input.text().trim().to_string();
        let prompt = app.submit().unwrap();
        assert_eq!(prompt, big);
        assert!(app.pastes.is_empty());
        assert!(!prompt.contains("[Pasted text"));
        let _ = placeholder;
    }

    #[test]
    fn expand_paste_at_cursor_replaces_line() {
        let mut app = test_app();
        let big = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.handle_paste(&big);
        assert_eq!(app.pastes.len(), 1);
        app.input
            .apply(crate::tui::editor::EditorAction::MoveToDocStart);
        assert!(app.expand_paste_at_cursor());
        assert_eq!(app.input.text(), big);
    }

    #[test]
    fn paste_normalizes_crlf() {
        let mut app = test_app();
        app.handle_paste("a\r\nb\r\nc");
        assert_eq!(app.input.text(), "a\nb\nc");
    }

    #[test]
    fn reasoning_lifecycle_finalizes() {
        let mut app = test_app();
        // First reasoning chunk starts a block; further chunks continue it.
        assert!(app.begin_or_continue_reasoning());
        assert!(!app.begin_or_continue_reasoning());
        // Moving on finalizes it.
        app.finalize_reasoning();
        match &app.content[0] {
            UiBlock::Reasoning { done: Some(d), .. } => assert!(*d >= std::time::Duration::ZERO),
            _ => panic!("expected done reasoning"),
        }
        // A later reasoning pass starts a fresh block.
        assert!(app.begin_or_continue_reasoning());
        assert_eq!(app.content.len(), 2);
    }

    #[test]
    fn timeline_selection_moves_between_tools() {
        let mut app = test_app();
        app.push(UiBlock::User("q".into()));
        app.push(UiBlock::Assistant {
            text: "answer".into(),
        });
        app.push(UiBlock::Tool(ToolBlock {
            kind: ToolKind::Grep,
            target: "foo".into(),
            state: ToolState::Success,
            output: "a:1:x".into(),
        }));
        app.select_next();
        assert_eq!(app.selected, Some(2));
        app.toggle_expanded(2);
        assert!(app.expanded.contains(&2));
        // Only collapsible items are selectable: User/Assistant skipped.
        app.select_next();
        assert_eq!(app.selected, None); // wraps out
        app.select_prev();
        assert_eq!(app.selected, Some(2));
    }

    #[test]
    fn paste_then_edit_then_submit() {
        let mut app = test_app();
        app.handle_paste("hello\nworld");
        app.input
            .apply(crate::tui::editor::EditorAction::MoveToDocEnd);
        app.input
            .apply(crate::tui::editor::EditorAction::DeleteBackward);
        app.input
            .apply(crate::tui::editor::EditorAction::InsertChar('X'));
        let prompt = app.submit().unwrap();
        assert_eq!(prompt, "hello\nworlX");
    }

    fn repo_app(name: &str) -> (App, std::path::PathBuf) {
        let d =
            std::env::temp_dir().join(format!("lightcode_mention_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src").join("auth.service.ts"), "service").unwrap();
        std::fs::write(d.join("src").join("auth.controller.ts"), "controller").unwrap();
        std::fs::write(d.join("src").join("main.rs"), "fn main(){}").unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
        let mut app = test_app();
        app.status.cwd = d.to_string_lossy().into_owned();
        (app, d)
    }

    #[test]
    fn mention_picker_opens_selects_and_positions_cursor() {
        let (mut app, d) = repo_app("picker");
        app.input
            .apply(crate::tui::editor::EditorAction::Paste("@auth.s".into()));
        app.update_mentions();
        let p = app.mention_picker.as_ref().expect("picker must open");
        assert_eq!(p.query, "auth.s");
        assert!(p.results.iter().any(|m| m.path == "src/auth.service.ts"));
        app.select_mention();
        assert!(app.mention_picker.is_none());
        let text = app.input.text();
        assert!(text.starts_with('@'));
        assert!(
            text.contains("auth.service.ts"),
            "selected path in text: {text}"
        );
        assert_eq!(
            app.input.cursor_byte_offset(),
            text.len(),
            "cursor after mention"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn esc_close_keeps_input_untouched() {
        let (mut app, d) = repo_app("esc");
        app.input
            .apply(crate::tui::editor::EditorAction::Paste("@auth.s".into()));
        app.update_mentions();
        assert!(app.mention_picker.is_some());
        app.mention_picker = None; // Esc
        assert_eq!(app.input.text(), "@auth.s"); // input unchanged
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn mention_picker_closes_when_cursor_leaves_mention() {
        let (mut app, d) = repo_app("leave");
        app.input.apply(crate::tui::editor::EditorAction::Paste(
            "@auth.s done".into(),
        ));
        app.input
            .apply(crate::tui::editor::EditorAction::MoveToDocEnd);
        app.update_mentions();
        assert!(app.mention_picker.is_none(), "cursor past mention → closed");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn multiple_mentions_survive_selection() {
        let (mut app, d) = repo_app("multi");
        app.input.apply(crate::tui::editor::EditorAction::Paste(
            "compare @auth.s".into(),
        ));
        app.update_mentions();
        app.select_mention();
        let text = app.input.text();
        assert!(text.starts_with("compare @src/"));
        // Keep typing a second mention.
        app.input
            .apply(crate::tui::editor::EditorAction::Paste(" and @main".into()));
        app.update_mentions();
        assert!(app.mention_picker.is_some());
        app.select_mention();
        let text = app.input.text();
        assert!(text.contains("@src/main.rs"), "second mention: {text}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn copy_target_uses_selection_else_last_assistant() {
        let mut app = test_app();
        app.push(UiBlock::User("question".into()));
        app.push(UiBlock::Assistant {
            text: "the answer".into(),
        });
        app.push(UiBlock::Tool(ToolBlock {
            kind: ToolKind::Shell,
            target: "$ echo hi".into(),
            state: ToolState::Success,
            output: "hi\nexit code: 0".into(),
        }));
        // No selection → last assistant message.
        assert_eq!(app.copy_target_text(), "the answer");
        // Selected tool block → its command + output.
        app.selected = Some(2);
        let t = app.copy_target_text();
        assert!(t.contains("echo hi"));
        assert!(t.contains("exit code: 0"));
    }
}

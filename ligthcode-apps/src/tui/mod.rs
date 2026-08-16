pub mod app;
pub mod editor;
pub mod keys;
pub mod md;
pub mod render;
pub mod select;
pub mod theme;

use crate::agent::{Agent, AgentEvent, AgentMode};
use crate::permissions::Choice;
use crate::providers::Message;
use crate::tui::app::{
    tool_target, App, Command, PermissionRequest, StatusInfo, ToolBlock, ToolKind, ToolState,
    UiBlock,
};
use crate::tui::editor::EditorAction;
use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::spawn_blocking;

fn short_title(cwd: &str) -> String {
    let max = 40;
    if cwd.chars().count() <= max {
        cwd.to_string()
    } else {
        let cut: String = cwd.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Commands from the UI to the agent task.
pub enum UiCommand {
    Run(String),
    SetModel {
        provider: String,
        model: String,
    },
    SetAgent(String),
    SetMode(AgentMode),
    LoadSession(String),
    Compact,
    /// Run an autonomous goal loop (see `crate::goal`).
    RunGoal(crate::goal::Goal),
}

/// Run the interactive TUI for an agent. The agent is owned here and driven in a task.
pub async fn run_tui(mut agent: Agent, status: StatusInfo) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        event::EnableBracketedPaste,
        event::EnableMouseCapture,
        terminal::SetTitle(format!("LightCode · {}", short_title(&status.cwd)))
    )?;
    // Keyboard enhancement (kitty protocol): lets terminals report Shift+Enter,
    // Option+arrows, etc. as distinct codes. Best-effort on older terminals.
    let _ = crossterm::execute!(
        stdout,
        event::PushKeyboardEnhancementFlags(
            event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );

    let (agent_tx, mut agent_rx) = mpsc::channel(256);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCommand>(16);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    agent.set_events(Some(agent_tx.clone()));
    agent.set_cancel(Some(cancel_rx));
    let agent_cancel = cancel_tx.clone();

    // The agent lives in a task and processes one prompt at a time.
    let agent_task = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                UiCommand::SetModel { provider, model } => {
                    agent.set_model_with_provider(&provider, &model)
                }
                UiCommand::SetAgent(name) => agent.set_agent(&name),
                UiCommand::SetMode(mode) => agent.set_mode(mode),
                UiCommand::LoadSession(id) => {
                    if let Ok(s) = crate::session::storage::open(&id) {
                        if let Ok(msgs) = s.load_history() {
                            let mut new_hist: Vec<Message> = agent
                                .history
                                .iter()
                                .take_while(|m| matches!(m, Message::System { .. }))
                                .cloned()
                                .collect();
                            new_hist.extend(msgs);
                            agent.history = new_hist;
                        }
                        agent.set_session(Some(s));
                    }
                }
                UiCommand::Compact => {
                    if let Ok(removed) = agent.compact_manual().await {
                        let _ = agent_tx.send(AgentEvent::Compact { removed }).await;
                    }
                }
                UiCommand::Run(prompt) => {
                    // Reset a prior cancellation so queued prompts start fresh.
                    let _ = agent_cancel.send(false);
                    let result = agent.run(&prompt).await;
                    let interrupted = agent.is_cancelled();
                    let _ = agent_tx
                        .send(match (result, interrupted) {
                            (Ok(m), _) => AgentEvent::Done {
                                ok: true,
                                message: m,
                            },
                            // Cancelled runs are not errors to display.
                            (Err(_), true) => AgentEvent::Done {
                                ok: true,
                                message: String::new(),
                            },
                            (Err(e), false) => AgentEvent::Done {
                                ok: false,
                                message: e.to_string(),
                            },
                        })
                        .await;
                }
                UiCommand::RunGoal(goal) => {
                    let _ = agent_cancel.send(false);
                    let evaluator = Box::new(crate::goal::ModelGoalEvaluator {
                        provider: agent.evaluator_provider(),
                    });
                    let mut mgr = crate::goal::GoalManager::new(goal, evaluator);
                    let status = mgr.run(&mut agent, &agent_tx).await;
                    let message = mgr.goal.message.clone();
                    let _ = agent_tx
                        .send(AgentEvent::Done {
                            ok: status == crate::goal::GoalStatus::Completed,
                            message,
                        })
                        .await;
                }
            }
        }
    });

    // Blocking crossterm reader forwarding events onto a channel.
    // Uses poll() with a timeout so the thread can observe the shutdown flag and exit.
    let (key_tx, mut key_rx) = mpsc::channel::<Event>(64);
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let key_task = {
        let running = running.clone();
        spawn_blocking(move || {
            while running.load(std::sync::atomic::Ordering::Relaxed) {
                if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    continue;
                }
                if let Ok(ev) = event::read() {
                    if key_tx.blocking_send(ev).is_err() {
                        break;
                    }
                }
            }
        })
    };

    let mut app = App::new(status);
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Redraw only when something actually changed. Idle frames cost nothing.
    let mut needs_redraw = true;
    while !app.quit {
        if needs_redraw {
            terminal.draw(|f| render::draw(f, &mut app))?;
            needs_redraw = false;
        }

        tokio::select! {
            _ = tick.tick() => {
                app.spinner = (app.spinner + 1) % 8;
                needs_redraw = app.busy || app.goal_active() || app.toast.is_some();
            }
            ke = key_rx.recv() => {
                if let Some(ke) = ke {
                    match ke {
                        Event::Key(k) => handle_key(&mut app, k, &cmd_tx, &cancel_tx).await,
                        Event::Paste(text) => {
                            if !app.busy {
                                app.handle_paste(&text);
                            }
                        }
                        Event::Mouse(m) => handle_mouse(&mut app, m),
                        _ => {}
                    }
                    needs_redraw = true;
                }
            }
            ae = agent_rx.recv() => {
                if let Some(ae) = ae {
                    handle_agent(&mut app, ae);
                    // Coalesce: drain every queued event before the next redraw so
                    // streaming chunks batch into one frame instead of one redraw
                    // per token.
                    while let Ok(more) = agent_rx.try_recv() {
                        handle_agent(&mut app, more);
                    }
                    needs_redraw = true;
                }
            }
        }
    }

    let _ = cancel_tx.send(true);
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    drop(cmd_tx); // unblock the agent task's recv loop so it can finish
    let _ = agent_task.await;
    let _ = key_task.await; // exits within ~100ms once `running` is false
    let _ = crossterm::execute!(stdout, event::PopKeyboardEnhancementFlags);
    crossterm::execute!(
        stdout,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    ratatui::restore();
    Ok(())
}

async fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    cmd_tx: &mpsc::Sender<UiCommand>,
    cancel_tx: &watch::Sender<bool>,
) {
    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
        return;
    }
    // Holding Enter must not resubmit / cancel repeatedly.
    if key.kind == KeyEventKind::Repeat && key.code == KeyCode::Enter {
        return;
    }

    // Fullscreen diff viewer consumes keys while open.
    if app.diff_viewer.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(d) = app.diff_viewer.as_mut() {
                    d.scroll = d.scroll.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Some(d) = app.diff_viewer.as_mut() {
                    d.scroll += 1;
                }
            }
            KeyCode::PageUp => {
                if let Some(d) = app.diff_viewer.as_mut() {
                    d.scroll = d.scroll.saturating_sub(10);
                }
            }
            KeyCode::PageDown => {
                if let Some(d) = app.diff_viewer.as_mut() {
                    d.scroll += 10;
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => app.diff_viewer = None,
            KeyCode::Char('y') | KeyCode::Char('c') => {
                if let Some(d) = &app.diff_viewer {
                    set_system_clipboard(&d.body);
                    app.show_toast("Diff copied to clipboard.");
                }
            }
            _ => {}
        }
        return;
    }

    // Session picker consumes keys while open.
    if app.session_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if app.session_picker.as_mut().is_some_and(|p| p.selected > 0) {
                    app.session_picker.as_mut().unwrap().selected -= 1;
                }
            }
            KeyCode::Down => {
                if let Some(p) = app.session_picker.as_mut() {
                    let items = filtered_sessions(p);
                    if p.selected + 1 < items.len() {
                        p.selected += 1;
                    }
                }
            }
            KeyCode::PageUp => {
                if let Some(p) = app.session_picker.as_mut() {
                    p.selected = p.selected.saturating_sub(10);
                }
            }
            KeyCode::PageDown => {
                if let Some(p) = app.session_picker.as_mut() {
                    let items = filtered_sessions(p);
                    p.selected = (p.selected + 10).min(items.len().saturating_sub(1));
                }
            }
            KeyCode::Char(c) => {
                if let Some(p) = app.session_picker.as_mut() {
                    p.filter.push(c);
                    p.selected = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = app.session_picker.as_mut() {
                    p.filter.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Enter => {
                if let Some(p) = app.session_picker.take() {
                    let items = filtered_sessions(&p);
                    if let Some(item) = items.get(p.selected.min(items.len().saturating_sub(1))) {
                        switch_session(app, &item.id, cmd_tx).await;
                    }
                }
            }
            KeyCode::Delete => {
                let mut refreshed = None;
                if let Some(p) = app.session_picker.as_mut() {
                    let items = filtered_sessions(p);
                    if let Some(item) = items.get(p.selected.min(items.len().saturating_sub(1))) {
                        let _ = crate::session::storage::delete(&item.id);
                        refreshed = Some(app.list_sessions());
                    }
                }
                if let Some(sessions) = refreshed {
                    if let Some(p) = app.session_picker.as_mut() {
                        p.sessions = sessions;
                        p.selected = 0;
                    }
                }
            }
            KeyCode::Esc => app.session_picker = None,
            _ => {}
        }
        return;
    }

    // Theme picker consumes keys while open; moving previews the theme live.
    if app.theme_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(p) = app.theme_picker.as_mut() {
                    if p.selected > 0 {
                        p.selected -= 1;
                        preview_theme(&app.theme_picker);
                    }
                }
                return;
            }
            KeyCode::Down => {
                if let Some(p) = app.theme_picker.as_mut() {
                    if p.selected + 1 < p.themes.len() {
                        p.selected += 1;
                        preview_theme(&app.theme_picker);
                    }
                }
                return;
            }
            KeyCode::PageUp => {
                if let Some(p) = app.theme_picker.as_mut() {
                    let old = p.selected;
                    p.selected = p.selected.saturating_sub(5);
                    if p.selected != old {
                        preview_theme(&app.theme_picker);
                    }
                }
                return;
            }
            KeyCode::PageDown => {
                if let Some(p) = app.theme_picker.as_mut() {
                    let old = p.selected;
                    p.selected = (p.selected + 5).min(p.themes.len().saturating_sub(1));
                    if p.selected != old {
                        preview_theme(&app.theme_picker);
                    }
                }
                return;
            }
            KeyCode::Enter => {
                if let Some(p) = app.theme_picker.take() {
                    if let Some(name) = p.themes.get(p.selected) {
                        if let Some(theme) = crate::tui::theme::Theme::by_name(name) {
                            crate::tui::theme::Theme::set(&theme);
                            crate::tui::theme::save(&theme);
                            app.show_toast(format!("Theme → {}", theme.name));
                        }
                    }
                }
                return;
            }
            KeyCode::Esc => {
                if let Some(p) = app.theme_picker.take() {
                    if let Some(theme) = crate::tui::theme::Theme::by_name(&p.prev) {
                        crate::tui::theme::Theme::set(&theme);
                    }
                }
                return;
            }
            _ => {}
        }
    }

    // Mode picker consumes keys while open.
    if app.mode_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(p) = app.mode_picker.as_mut() {
                    if p.selected > 0 {
                        p.selected -= 1;
                    }
                }
                return;
            }
            KeyCode::Down => {
                if let Some(p) = app.mode_picker.as_mut() {
                    if p.selected + 1 < p.modes.len() {
                        p.selected += 1;
                    }
                }
                return;
            }
            KeyCode::Enter => {
                if let Some(p) = app.mode_picker.take() {
                    if let Some((mode, _)) = p.modes.get(p.selected) {
                        set_mode(app, *mode, cmd_tx).await;
                    }
                }
                return;
            }
            KeyCode::Esc => {
                app.mode_picker = None;
                return;
            }
            _ => {}
        }
    }

    // Agent picker consumes keys while open.
    if app.agent_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if app.agent_picker.as_mut().is_some_and(|p| p.selected > 0) {
                    app.agent_picker.as_mut().unwrap().selected -= 1;
                }
            }
            KeyCode::Down => {
                if let Some(p) = app.agent_picker.as_mut() {
                    if p.selected + 1 < p.agents.len() {
                        p.selected += 1;
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(p) = app.agent_picker.take() {
                    if let Some((name, _)) = p.agents.get(p.selected) {
                        let _ = cmd_tx.send(UiCommand::SetAgent(name.clone())).await;
                        app.show_toast(format!("Agent → {name}"));
                    }
                }
            }
            KeyCode::Esc => app.agent_picker = None,
            _ => {}
        }
        return;
    }

    // Model picker consumes keys while open.
    if app.model_picker.is_some() {
        let picker = app.model_picker.as_mut().unwrap();
        match key.code {
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                let total = filtered_model_count(picker);
                if picker.selected + 1 < total {
                    picker.selected += 1;
                }
            }
            KeyCode::PageUp => {
                picker.selected = picker.selected.saturating_sub(10);
            }
            KeyCode::PageDown => {
                let total = filtered_model_count(picker);
                picker.selected = (picker.selected + 10).min(total.saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(p) = app.model_picker.take() {
                    let filtered = filtered_model_indices(&p);
                    if let Some(&idx) =
                        filtered.get(p.selected.min(filtered.len().saturating_sub(1)))
                    {
                        if let Some(item) = p.models.get(idx) {
                            app.status.model = item.name.clone();
                            app.status.provider = item.provider.clone();
                            let provider = item.provider.clone();
                            let model = item.id.clone();
                            let _ = cmd_tx.send(UiCommand::SetModel { provider, model }).await;
                        }
                    }
                }
            }
            KeyCode::Esc => app.model_picker = None,
            _ => {
                // Full editor editing (Cmd+Delete, word nav, selection, paste).
                if let Some(action) = keys::map_key(key) {
                    let clip = (action == EditorAction::Copy || action == EditorAction::Cut)
                        .then(|| app.model_picker.as_ref().unwrap().search.selected_text())
                        .flatten();
                    if action == EditorAction::PasteClipboard
                        && app
                            .model_picker
                            .as_ref()
                            .unwrap()
                            .search
                            .clipboard_is_empty()
                    {
                        if let Ok(text) = get_system_clipboard() {
                            app.model_picker
                                .as_mut()
                                .unwrap()
                                .search
                                .apply(EditorAction::Paste(text));
                        }
                    } else {
                        app.model_picker.as_mut().unwrap().search.apply(action);
                    }
                    if let Some(text) = clip {
                        set_system_clipboard(&text);
                    }
                    app.model_picker.as_mut().unwrap().selected = 0;
                }
            }
        }
        return;
    }

    // Command palette consumes keys while open.
    if app.command_palette.is_some() {
        match key.code {
            KeyCode::Up => {
                let cmds = App::palette_commands(app.command_palette.as_ref().unwrap());
                if app.command_palette.as_mut().is_some_and(|p| p.selected > 0) {
                    app.command_palette.as_mut().unwrap().selected -= 1;
                }
                let _ = cmds;
            }
            KeyCode::Down => {
                let cmds = App::palette_commands(app.command_palette.as_ref().unwrap());
                if let Some(p) = app.command_palette.as_mut() {
                    if p.selected + 1 < cmds.len() {
                        p.selected += 1;
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(p) = app.command_palette.as_mut() {
                    p.filter.push(c);
                    p.selected = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = app.command_palette.as_mut() {
                    p.filter.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Enter => {
                if let Some(p) = app.command_palette.take() {
                    let cmds = App::palette_commands(&p);
                    if let Some(cmd) = cmds.get(p.selected.min(cmds.len().saturating_sub(1))) {
                        run_command(app, *cmd, cmd_tx).await;
                    }
                }
            }
            KeyCode::Esc => app.command_palette = None,
            _ => {}
        }
        return;
    }

    // Question dialog consumes keys until answered.
    if app.pending_question.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(q) = app.pending_question.as_mut() {
                    if q.selected > 0 {
                        q.selected -= 1;
                    }
                }
            }
            KeyCode::Down => {
                if let Some(q) = app.pending_question.as_mut() {
                    if q.selected + 1 < q.options.len() {
                        q.selected += 1;
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(q) = app.pending_question.take() {
                    if let Some(respond) = q.respond {
                        let answer = q.options.get(q.selected).cloned();
                        let _ = respond.send(answer);
                    }
                }
            }
            KeyCode::Esc => {
                if let Some(q) = app.pending_question.take() {
                    if let Some(respond) = q.respond {
                        let _ = respond.send(None);
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // Permission prompt consumes keys until answered.
    if app.pending_permission.is_some() {
        if app.pending_permission.as_ref().unwrap().entering_feedback {
            match key.code {
                KeyCode::Char(c) => {
                    if let Some(p) = app.pending_permission.as_mut() {
                        p.feedback.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(p) = app.pending_permission.as_mut() {
                        p.feedback.pop();
                    }
                }
                KeyCode::Enter => {
                    if let Some(p) = app.pending_permission.take() {
                        if let Some(respond) = p.respond {
                            let feedback = if p.feedback.trim().is_empty() {
                                None
                            } else {
                                Some(p.feedback.trim().to_string())
                            };
                            let _ = respond.send(Choice::Deny { feedback });
                        }
                    }
                }
                KeyCode::Esc => {
                    if let Some(p) = app.pending_permission.as_mut() {
                        p.entering_feedback = false;
                        p.feedback.clear();
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => answer_permission(app, Choice::Allow),
            KeyCode::Char('a') | KeyCode::Char('s') => {
                answer_permission(app, Choice::AllowForSession)
            }
            KeyCode::Char('w') => answer_permission(app, Choice::Always),
            KeyCode::Char('r') => {
                if let Some(p) = app.pending_permission.as_mut() {
                    p.entering_feedback = true;
                }
            }
            KeyCode::Esc | KeyCode::Char('n') => {
                answer_permission(app, Choice::Deny { feedback: None })
            }
            _ => {}
        }
        return;
    }

    // Leader key (Ctrl+X): next key runs a command; a which-key panel shows the map.
    if app.leader_active {
        app.leader_active = false;
        let cmd = match key.code {
            KeyCode::Char('n') => Some(Command::NewSession),
            KeyCode::Char('l') => Some(Command::Sessions),
            KeyCode::Char('m') => Some(Command::Models),
            KeyCode::Char('g') => Some(Command::Agent),
            KeyCode::Char('s') => Some(Command::Stats),
            KeyCode::Char('c') => Some(Command::Clear),
            KeyCode::Char('h') => Some(Command::Help),
            KeyCode::Char('d') => Some(Command::Debug),
            KeyCode::Char('q') => Some(Command::Quit),
            KeyCode::Char('y') => {
                let text = app.copy_target_text();
                if !text.is_empty() {
                    set_system_clipboard(&text);
                    app.show_toast(format!("Copied {} chars.", text.chars().count()));
                }
                None
            }
            _ => None,
        };
        if let Some(cmd) = cmd {
            run_command(app, cmd, cmd_tx).await;
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                app.quit = true;
                return;
            }
            KeyCode::Char('k') => {
                if !app.busy {
                    app.open_command_palette();
                }
                return;
            }
            KeyCode::Char('x') => {
                app.leader_active = true;
                return;
            }
            KeyCode::Char('g') => {
                app.scroll = 0;
                app.auto_scroll = false;
                return;
            }
            KeyCode::Char('d') => {
                if let Some(body) = app.content.iter().rev().find_map(|b| match b {
                    UiBlock::Diff { body, .. } => Some(body.clone()),
                    _ => None,
                }) {
                    app.diff_viewer = Some(app::DiffViewer { body, scroll: 0 });
                }
                return;
            }
            KeyCode::Up => {
                app.select_prev();
                return;
            }
            KeyCode::Down => {
                app.select_next();
                return;
            }
            // Other control keys (Ctrl+A/E/W/U/Y/Z/J) fall through to the editor mapping.
            _ => {}
        }
    }

    // Timeline selection: Enter expands/collapses the selected item, Esc clears it.
    // A typed prompt takes priority: Enter submits it instead of toggling.
    if app.selected.is_some() {
        match key.code {
            KeyCode::Enter => {
                if app.input.text().trim().is_empty() {
                    if let Some(i) = app.selected {
                        app.toggle_expanded(i);
                    }
                    return;
                }
            }
            KeyCode::Esc => {
                app.selected = None;
                return;
            }
            _ => {}
        }
    }

    // @-mention picker: ↑/↓ navigate, Enter/Tab select, Esc closes untouched.
    if app.mention_picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(p) = app.mention_picker.as_mut() {
                    if p.selected > 0 {
                        p.selected -= 1;
                    }
                }
                return;
            }
            KeyCode::Down => {
                if let Some(p) = app.mention_picker.as_mut() {
                    if p.selected + 1 < p.results.len() {
                        p.selected += 1;
                    }
                }
                return;
            }
            KeyCode::Enter | KeyCode::Tab => {
                app.select_mention();
                return;
            }
            KeyCode::Esc => {
                app.mention_picker = None;
                return;
            }
            _ => {}
        }
    }

    // Copy terminal output to the OS clipboard.
    // macOS: Cmd+C copies the composer selection, or the selected/last output
    // when the composer has nothing selected. Ctrl+O works everywhere.
    let is_copy = (key.modifiers.contains(KeyModifiers::SUPER) && key.code == KeyCode::Char('c'))
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o'));
    if is_copy {
        if key.modifiers.contains(KeyModifiers::SUPER) && app.input.selected_text().is_some() {
            // Composer selection → editor copy path handles it below.
        } else if let Some(text) = app.copy_mouse_selection() {
            set_system_clipboard(&text);
            app.show_toast(format!(
                "Copied selection ({} chars) to clipboard.",
                text.chars().count()
            ));
            return;
        } else {
            let text = app.copy_target_text();
            if !text.is_empty() {
                set_system_clipboard(&text);
                app.show_toast(format!(
                    "Copied {} chars to clipboard.",
                    text.chars().count()
                ));
            } else {
                app.show_toast("Nothing to copy yet.");
            }
            return;
        }
    }

    match key.code {
        KeyCode::Esc => {
            if app.busy {
                let _ = cancel_tx.send(true);
                app.auto_scroll = true;
            } else if app.goal.is_some() {
                app.goal = None;
                app.mouse_sel = None;
            } else {
                app.mouse_sel = None;
                app.input.clear();
                app.suggestions.clear();
            }
        }
        KeyCode::Enter => {
            // New line: Alt+Enter (Option) or Shift+Enter where the terminal reports it.
            if key.modifiers.contains(KeyModifiers::ALT)
                || key.modifiers.contains(KeyModifiers::SHIFT)
            {
                if !app.busy {
                    app.input.apply(EditorAction::InsertNewline);
                    app.update_suggestions();
                    app.update_mentions();
                }
                return;
            }
            if app.busy {
                // Interrupt the current run and queue this prompt for the next turn.
                if !app.input.text().trim().is_empty() {
                    let trimmed = app.input.text().trim().to_string();
                    // /goal subcommands (cancel/status/…) work while a goal runs.
                    if trimmed.starts_with("/goal") {
                        handle_goal_command(app, &trimmed, cmd_tx, cancel_tx).await;
                        return;
                    }
                    let _ = cancel_tx.send(true);
                    if let Some(prompt) = app.submit() {
                        let _ = cmd_tx.send(UiCommand::Run(prompt)).await;
                    }
                }
                return;
            }
            if !app.busy {
                let text = app.input.text();
                let trimmed = text.trim();
                // Empty composer + a collapsed block with output: Enter expands it
                // (submitting an empty prompt is a no-op anyway).
                if trimmed.is_empty() {
                    if let Some(i) = app.last_collapsible_with_output() {
                        app.toggle_expanded(i);
                        return;
                    }
                }
                match trimmed {
                    "/models" => {
                        app.open_model_picker();
                        return;
                    }
                    "/agent" => {
                        app.open_agent_picker();
                        return;
                    }
                    "/themes" => {
                        app.open_theme_picker();
                        return;
                    }
                    "/mode" => {
                        app.open_mode_picker();
                        return;
                    }
                    _ if trimmed.starts_with("/mode ") => {
                        let name = trimmed.trim_start_matches("/mode ").trim();
                        match AgentMode::from_str(name) {
                            Some(mode) => {
                                set_mode(app, mode, cmd_tx).await;
                                app.input.clear();
                                app.suggestions.clear();
                                app.mention_picker = None;
                            }
                            None => {
                                app.show_toast(format!(
                                    "Unknown mode: {name}. Available: plan, build, auto"
                                ));
                            }
                        }
                        return;
                    }
                    "/sessions" => {
                        app.open_session_picker();
                        return;
                    }
                    "/new" => {
                        run_command(app, Command::NewSession, cmd_tx).await;
                        return;
                    }
                    "/status" => {
                        run_command(app, Command::Status, cmd_tx).await;
                        return;
                    }
                    "/stats" => {
                        run_command(app, Command::Stats, cmd_tx).await;
                        return;
                    }
                    "/compact" => {
                        let _ = cmd_tx.send(UiCommand::Compact).await;
                        app.show_toast("Compacting conversation…");
                        return;
                    }
                    "/fork" => {
                        if let Ok(forked) =
                            crate::session::storage::fork(&app.status.session.clone())
                        {
                            app.load_session(&forked.id).ok();
                            let id = forked.id.clone();
                            let _ = cmd_tx.send(UiCommand::LoadSession(id)).await;
                            app.show_toast("Session forked.");
                        }
                        return;
                    }
                    _ if trimmed.starts_with("/rename ") => {
                        let title = trimmed.trim_start_matches("/rename ").trim();
                        if title.is_empty() {
                            app.show_toast("Usage: /rename <title>");
                        } else {
                            let id = app.status.session.clone();
                            match crate::session::storage::rename(&id, title) {
                                Ok(()) => app.show_toast(format!("Session renamed → {title}")),
                                Err(e) => app.show_toast(format!("Rename failed: {e}")),
                            }
                        }
                        return;
                    }
                    "/help" => {
                        run_command(app, Command::Help, cmd_tx).await;
                        return;
                    }
                    "/clear" => {
                        app.clear_view();
                        return;
                    }
                    "/debug" => {
                        run_command(app, Command::Debug, cmd_tx).await;
                        return;
                    }
                    "/quit" | "/exit" => {
                        app.quit = true;
                        return;
                    }
                    _ if trimmed.starts_with("/goal") => {
                        handle_goal_command(app, trimmed, cmd_tx, cancel_tx).await;
                        return;
                    }
                    _ => {}
                }
                if let Some(prompt) = app.submit() {
                    app.suggestions.clear();
                    let _ = cancel_tx.send(false);
                    let _ = cmd_tx.send(UiCommand::Run(prompt)).await;
                }
            }
        }
        // Up/Down: cursor moves inside multiline input; at the first/last line they recall history.
        // An empty composer navigates the timeline above instead (OpenCode-style),
        // so Up never silently replaces an empty prompt with an old one.
        KeyCode::Up => {
            if app.input.is_empty() {
                app.select_prev();
                return;
            }
            if app.input.cursor().row == 0 {
                app.history_prev();
            } else {
                app.input.apply(EditorAction::MoveUp);
            }
        }
        KeyCode::Down => {
            if app.input.is_empty() {
                app.select_next();
                return;
            }
            if app.input.cursor().row + 1 >= app.input.line_count() {
                app.history_next();
            } else {
                app.input.apply(EditorAction::MoveDown);
            }
        }
        KeyCode::PageUp => {
            app.auto_scroll = false;
            app.scroll = app.scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            app.scroll = app.scroll.saturating_add(10);
        }
        KeyCode::Tab => app.show_tool_output = !app.show_tool_output,
        // Shift+Tab cycles the agent mode (applies next turn when busy).
        KeyCode::BackTab => {
            let mode = app.cycle_mode();
            set_mode(app, mode, cmd_tx).await;
        }
        _ => {
            // Typing/editing stays enabled while busy so the user can compose
            // the next prompt while the agent runs (Enter queues it).
            // Alt+P expands a paste placeholder under the cursor.
            if key.code == KeyCode::Char('p')
                && key.modifiers.contains(KeyModifiers::ALT)
                && app.expand_paste_at_cursor()
            {
                return;
            }
            if let Some(action) = keys::map_key(key) {
                if action == EditorAction::PasteClipboard && app.input.clipboard_is_empty() {
                    if let Ok(text) = get_system_clipboard() {
                        app.input.apply(EditorAction::Paste(text));
                    }
                } else {
                    let clip = (action == EditorAction::Copy || action == EditorAction::Cut)
                        .then(|| app.input.selected_text())
                        .flatten();
                    app.input.apply(action);
                    if let Some(text) = clip {
                        set_system_clipboard(&text);
                    }
                }
                app.update_suggestions();
                app.update_mentions();
            }
        }
    }
}

/// Handle `/goal ...` typed into the composer.
async fn handle_goal_command(
    app: &mut App,
    trimmed: &str,
    cmd_tx: &mpsc::Sender<UiCommand>,
    cancel_tx: &watch::Sender<bool>,
) {
    let rest = trimmed.trim_start_matches("/goal").trim();
    if rest.is_empty() {
        app.show_toast("/goal <objective> [verify: <cmd>] [max_turns: N]");
        return;
    }
    let lower = rest.to_lowercase();
    match lower.as_str() {
        "status" => {
            goal_status(app);
            app.input.clear();
            app.suggestions.clear();
            return;
        }
        "cancel" => {
            app.input.clear();
            app.suggestions.clear();
            let _ = cancel_tx.send(true);
            app.show_toast("Cancelling goal…");
            return;
        }
        "continue" | "retry" => {
            let Some(saved) = crate::session::storage::open(&app.status.session)
                .ok()
                .and_then(|s| s.load_goal_json().ok().flatten())
                .and_then(|json| serde_json::from_str::<crate::goal::Goal>(&json).ok())
            else {
                app.show_toast("No saved goal in this session.");
                return;
            };
            let mut goal = saved;
            if goal.status.is_stopped() && goal.current_turn >= goal.max_turns {
                // Stopped at the cap: extend the cap so the loop can continue.
                goal.max_turns += goal.max_turns.max(1);
            }
            if goal.status == crate::goal::GoalStatus::Completed {
                app.show_toast("Goal already completed.");
                return;
            }
            if lower == "retry" {
                // Retry the last turn: back up one so it runs again.
                goal.current_turn = goal.current_turn.saturating_sub(1);
            }
            begin_goal(app, &goal, cmd_tx).await;
            return;
        }
        _ => {}
    }
    // New goal.
    let spec = crate::goal::parse_goal(rest, app.status.max_goal_turns);
    if spec.description.is_empty() {
        app.show_toast("/goal <objective> [verify: <cmd>] [max_turns: N]");
        return;
    }
    let goal = spec.to_goal();
    begin_goal(app, &goal, cmd_tx).await;
}

/// Render the goal as a user message, mark busy, and dispatch the goal loop.
async fn begin_goal(app: &mut App, goal: &crate::goal::Goal, cmd_tx: &mpsc::Sender<UiCommand>) {
    if app.goal_active() {
        app.show_toast("A goal is already running. Cancel it first (/goal cancel).");
        return;
    }
    app.input.clear();
    app.suggestions.clear();
    app.push(UiBlock::User(format!("[goal] {}", goal.description)));
    app.busy = true;
    app.goal = None; // the Started event rebuilds a fresh panel
    let _ = cmd_tx.send(UiCommand::RunGoal(goal.clone())).await;
}

/// Push the current goal status into the timeline (`/goal status`).
fn goal_status(app: &mut App) {
    // Live panel wins; otherwise fall back to the persisted goal for this
    // session (e.g. after a restart).
    let saved = if app.goal.is_none() {
        crate::session::storage::open(&app.status.session)
            .ok()
            .and_then(|s| s.load_goal_json().ok().flatten())
            .and_then(|json| serde_json::from_str::<crate::goal::Goal>(&json).ok())
    } else {
        None
    };
    let (description, status, turn, max, evaluation, message) = match (app.goal.as_ref(), saved) {
        (Some(g), _) => (
            g.description.clone(),
            g.status.label().to_string(),
            g.turn,
            g.max_turns,
            g.evaluation.clone(),
            g.message.clone(),
        ),
        (None, Some(g)) => (
            g.description.clone(),
            g.status.label().to_string(),
            g.current_turn,
            g.max_turns,
            g.last_evaluation.clone(),
            g.message.clone(),
        ),
        (None, None) => {
            app.push(UiBlock::Assistant {
                text: "No goal in this session.".into(),
            });
            return;
        }
    };
    let mut lines = format!("**Goal**\n\n{description}\n\n");
    lines.push_str(&format!("status: `{status}` · turn `{turn}/{max}`\n"));
    if let Some(e) = &evaluation {
        let verdict = if e.completed {
            "complete"
        } else {
            "incomplete"
        };
        lines.push_str(&format!("evaluator: `{verdict}` — {}\n", e.reason));
        for r in &e.remaining_work {
            lines.push_str(&format!("• {r}\n"));
        }
    }
    if !message.is_empty() {
        lines.push_str(&format!("\n{message}"));
    }
    app.push(UiBlock::Assistant { text: lines });
}

fn filtered_sessions(p: &app::SessionPicker) -> Vec<app::SessionItem> {
    let f = p.filter.to_lowercase();
    if f.is_empty() {
        return p.sessions.clone();
    }
    p.sessions
        .iter()
        .filter(|s| s.title.to_lowercase().contains(&f) || s.id.contains(&f))
        .cloned()
        .collect()
}

async fn switch_session(app: &mut App, id: &str, cmd_tx: &mpsc::Sender<UiCommand>) {
    if app.load_session(id).is_ok() {
        let _ = cmd_tx.send(UiCommand::LoadSession(id.to_string())).await;
    }
}

/// Apply an agent mode change: update the UI display, inform the agent task,
/// and (when busy) note it applies to the next turn.
async fn set_mode(app: &mut App, mode: AgentMode, cmd_tx: &mpsc::Sender<UiCommand>) {
    app.set_mode(mode);
    if app.busy {
        app.show_toast(format!(
            "Mode → {} (applies to the next turn)",
            mode.label()
        ));
    } else {
        app.show_toast(format!("Mode → {}", mode.label()));
    }
    let _ = cmd_tx.send(UiCommand::SetMode(mode)).await;
}

fn set_system_clipboard(text: &str) {
    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string()));
}

/// Handle a terminal mouse event: left-drag selects content, wheel scrolls.
fn handle_mouse(app: &mut App, m: event::MouseEvent) {
    use event::{MouseButton, MouseEventKind};
    // Content-area coordinates.
    let row = m.row.saturating_sub(app.content_area.y) as usize;
    let col = m.column as usize;
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Click in the composer places the caret (macOS-standard).
            let comp = app.composer_area;
            if !app.busy && m.row >= comp.y && m.row < comp.y + comp.height {
                let vr = (m.row - comp.y) as usize;
                let vc = (m.column.saturating_sub(comp.x)) as usize;
                if let Some(cur) = crate::tui::render::composer_cursor_at(
                    &app.input,
                    comp.width as usize,
                    comp.height as usize,
                    vr,
                    vc,
                ) {
                    app.input.set_cursor(cur);
                    app.input.scroll_to_cursor(comp.height as usize);
                    return;
                }
            }
            if !app.busy && row < app.content_area.height as usize {
                app.mouse_dragging = true;
                app.mouse_hover = Some(row);
                let pos = (row, col);
                app.mouse_sel = Some((pos, pos));
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            app.mouse_hover = Some(row);
            if let Some((anchor, _)) = app.mouse_sel {
                let pos = (row, col);
                app.mouse_sel = Some((anchor, pos));
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            app.mouse_dragging = false;
            // Click (press + release on the same row, no drag): toggle the
            // timeline item under the pointer. Drag-select still works via the
            // mouse_sel path in the renderer.
            if let Some((anchor, focus)) = app.mouse_sel.take() {
                if anchor == focus && row < app.content_area.height as usize {
                    let abs = app.content_scroll + anchor.0;
                    for (idx, &(s, e)) in app.item_ranges.iter().enumerate() {
                        if abs >= s && abs <= e {
                            app.selected = Some(idx);
                            app.toggle_expanded(idx);
                            break;
                        }
                    }
                }
            }
        }
        MouseEventKind::Moved => {
            app.mouse_hover = if row < app.content_area.height as usize {
                Some(row)
            } else {
                None
            };
        }
        MouseEventKind::ScrollUp => {
            app.mouse_sel = None;
            app.mouse_hover = None;
            app.auto_scroll = false;
            app.scroll = app.scroll.saturating_sub(3);
        }
        MouseEventKind::ScrollDown => {
            app.mouse_sel = None;
            app.mouse_hover = None;
            app.scroll = app.scroll.saturating_add(3);
        }
        _ => {}
    }
}

fn get_system_clipboard() -> Result<String> {
    let mut c = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let text = c.get_text().map_err(|e| anyhow!("clipboard: {e}"))?;
    Ok(text)
}

fn answer_permission(app: &mut App, choice: Choice) {
    if let Some(p) = app.pending_permission.take() {
        if let Some(respond) = p.respond {
            let _ = respond.send(choice);
        }
    }
}

/// Apply the currently selected theme live (preview), without persisting.
fn preview_theme(picker: &Option<app::ThemePicker>) {
    if let Some(p) = picker {
        if let Some(name) = p.themes.get(p.selected) {
            if let Some(theme) = crate::tui::theme::Theme::by_name(name) {
                crate::tui::theme::Theme::set(&theme);
            }
        }
    }
}

/// Indices of models matching the picker's search text, in original order.
fn filtered_model_indices(p: &app::ModelPicker) -> Vec<usize> {
    let q = p.search.text().trim().to_lowercase();
    if q.is_empty() {
        return (0..p.models.len()).collect();
    }
    p.models
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.provider.to_lowercase().contains(&q)
                || m.id.to_lowercase().contains(&q)
                || m.name.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

fn filtered_model_count(p: &app::ModelPicker) -> usize {
    filtered_model_indices(p).len()
}
async fn run_command(app: &mut App, cmd: Command, cmd_tx: &mpsc::Sender<UiCommand>) {
    match cmd {
        Command::NewSession => {
            if app.new_session().is_ok() {
                let id = app.status.session.clone();
                let _ = cmd_tx.send(UiCommand::LoadSession(id)).await;
            }
        }
        Command::Sessions => app.open_session_picker(),
        Command::Models => app.open_model_picker(),
        Command::Agent => app.open_agent_picker(),
        Command::Themes => app.open_theme_picker(),
        Command::Status => {
            let cwd = app.status.cwd.clone();
            let info = format!(
                "**Status**\n\nmodel: `{}`\nsession: `{}`\ncwd: `{}`",
                app.status.model, app.status.session, cwd
            );
            app.push(UiBlock::Assistant { text: info });
        }
        Command::Stats => {
            let (users, assistants, tools, chars) =
                app.content
                    .iter()
                    .fold((0usize, 0usize, 0usize, 0usize), |acc, b| {
                        let (u, a, t, c) = acc;
                        match b {
                            UiBlock::User(x) => (u + 1, a, t, c + x.chars().count()),
                            UiBlock::Assistant { text } => (u, a + 1, t, c + text.chars().count()),
                            UiBlock::Reasoning { .. } => (u, a, t, c),
                            UiBlock::Tool(tb) => (u, a, t + 1, c + tb.output.chars().count()),
                            UiBlock::Activity(act) => {
                                let tc: usize =
                                    act.items.iter().map(|i| i.output.chars().count()).sum();
                                (u, a, t + act.items.len(), c + tc)
                            }
                            UiBlock::Diff { body, .. } => (u, a, t + 1, c + body.chars().count()),
                            UiBlock::Error(x) => (u, a, t, c + x.chars().count()),
                        }
                    });
            let info = format!(
                "**Stats**\n\nuser: `{users}` · assistant: `{assistants}` · tool: `{tools}`\n\
estimated tokens (chars/4): `{}`\nmodel: `{}`\nsession: `{}`",
                chars / 4,
                app.status.model,
                app.status.session
            );
            app.push(UiBlock::Assistant { text: info });
        }
        Command::Help => {
            let help =
                "**Pintasan**\n\n`Ctrl+K` palette · `Ctrl+C` keluar · `Esc` batal/berhenti\n\
`Cmd+C` salin output (pilih dulu dgn `Ctrl+↑/↓`, atau pesan terakhir) · `Ctrl+O` sama\n\
`Enter` kirim · `Shift+Enter` baris baru · `↑/↓` riwayat\n\
`Shift+Tab` ganti mode (PLAN/BUILD/AUTO) · `Ctrl+↑/↓` pilih item\n\
`Enter` (saat terpilih) buka/tutup output · `Tab` semua output\n\
`Ctrl+J` baris baru\n\n\
**Slash**: `/mode` `/mode plan|build|auto` `/goal <objective>` `/goal status|cancel|continue|retry` `/models` `/sessions` `/new` `/clear` `/status` `/help` `/debug` `/quit`";
            app.push(UiBlock::Assistant {
                text: help.to_string(),
            });
        }
        Command::Clear => app.clear_view(),
        Command::Debug => {
            let cwd = app.status.cwd.clone();
            let info = format!(
                "**Debug**\n\nmodel: `{}`\nsession: `{}`\ncwd: `{}`\n\n`/debug` — /quit untuk keluar",
                app.status.model, app.status.session, cwd
            );
            app.push(UiBlock::Assistant { text: info });
        }
        Command::Quit => app.quit = true,
    }
}

/// Extract the file path from a unified `git diff` output.
fn diff_file(body: &str) -> String {
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(idx) = rest.rfind(" b/") {
                return rest[idx + 3..].to_string();
            }
        }
    }
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            return rest.to_string();
        }
    }
    "git diff".to_string()
}

/// Best-effort tool outcome classification from its output text.
fn classify_tool_output(output: &str) -> ToolState {
    if output.starts_with("tool error:") || output.starts_with("permission denied") {
        return ToolState::Failed;
    }
    if let Some(idx) = output.rfind("exit code: ") {
        if let Ok(code) = output[idx + "exit code: ".len()..].trim().parse::<i32>() {
            if code != 0 {
                return ToolState::Failed;
            }
        }
    }
    ToolState::Success
}

fn handle_agent(app: &mut App, ev: AgentEvent) {
    match ev {
        AgentEvent::Text(t) => {
            app.finalize_reasoning();
            app.finalize_activity();
            if t.trim().is_empty() {
                return;
            }
            if let Some(UiBlock::Assistant { text }) = app.content.last_mut() {
                text.push_str(&t);
            } else {
                app.push(UiBlock::Assistant { text: t });
            }
        }
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
        } => {
            app.context_tokens = input_tokens;
            app.total_input_tokens += input_tokens;
            app.total_output_tokens += output_tokens;
        }
        AgentEvent::Reasoning(_) => {
            app.begin_or_continue_reasoning();
        }
        AgentEvent::ToolStart { name, args } => {
            app.finalize_reasoning();
            let target = serde_json::from_str::<serde_json::Value>(&args)
                .map(|v| tool_target(&name, &v))
                .unwrap_or_default();
            let item = app::ActivityItem {
                name: name.clone(),
                kind: ToolKind::from_name(&name),
                target,
                status: app::ActivityStatus::Running,
                output: String::new(),
            };
            if name == "git_diff" {
                // git_diff renders as a dedicated diff block on completion.
                return;
            }
            let phase = App::phase_for_tool(&name);
            match app.content.last_mut() {
                Some(UiBlock::Activity(a)) if a.phase == phase && !a.done => {
                    a.items.push(item);
                }
                _ => {
                    app.finalize_activity();
                    app.content.push(UiBlock::Activity(app::ActivityBlock {
                        phase,
                        items: vec![item],
                        done: false,
                    }));
                }
            }
        }
        AgentEvent::Diff { file, body } => {
            app.finalize_activity();
            app.push(UiBlock::Diff { file, body });
        }
        AgentEvent::ToolOutput { name, output } => {
            // git_diff becomes a dedicated diff block.
            if name == "git_diff" {
                app.finalize_activity();
                app.push(UiBlock::Diff {
                    file: diff_file(&output),
                    body: output,
                });
                return;
            }
            let status = if classify_tool_output(&output) == ToolState::Failed {
                app::ActivityStatus::Failed
            } else {
                app::ActivityStatus::Success
            };
            // Fill the running item in the current activity (if any).
            if let Some(UiBlock::Activity(a)) = app.content.last_mut() {
                if let Some(item) = a
                    .items
                    .iter_mut()
                    .rev()
                    .find(|it| it.name == name && it.output.is_empty())
                {
                    item.output = output;
                    item.status = status;
                    return;
                }
            }
            // Fallback: standalone tool block (e.g. session replay).
            app.push(UiBlock::Tool(ToolBlock {
                kind: ToolKind::from_name(&name),
                target: String::new(),
                state: classify_tool_output(&output),
                output: output.clone(),
            }));
        }
        AgentEvent::Permission {
            prompt,
            diff,
            respond,
        } => {
            app.pending_permission = Some(PermissionRequest {
                prompt,
                diff,
                respond: Some(respond),
                entering_feedback: false,
                feedback: String::new(),
            });
        }
        AgentEvent::Question {
            prompt,
            options,
            respond,
        } => {
            app.pending_question = Some(app::QuestionRequest {
                prompt,
                options,
                selected: 0,
                respond: Some(respond),
            });
        }
        AgentEvent::Compact { removed } => {
            if removed > 0 {
                app.show_toast(format!(
                    "Conversation compacted ({removed} messages folded)."
                ));
            } else {
                app.show_toast("Nothing to compact yet.");
            }
        }
        AgentEvent::Goal(ev) => {
            app.handle_goal_event(ev);
        }
        AgentEvent::Done { ok, message } => {
            app.finalize_reasoning();
            app.finalize_activity();
            app.busy = false;
            app.pending_permission = None;
            app.pending_question = None;
            for item in app.content.iter_mut() {
                if let UiBlock::Tool(tb) = item {
                    if tb.state == ToolState::Running {
                        tb.state = ToolState::Success;
                    }
                }
            }
            // Completed blocks changed, so cached rendering is stale.
            app.content_ver += 1;
            if !ok {
                app.last_error = Some(message.clone());
                app.push(UiBlock::Error(message));
            }
            app.auto_scroll = true;
            app.pending = 0;
        }
    }
}

use super::app::{ActivityStatus, App, ToolBlock, ToolKind, ToolState, UiBlock};
use super::editor::TextEditor;
use super::md;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;

const SPINNER: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
const HEADER_HEIGHT: u16 = 1;
const STATUS_HEIGHT: u16 = 1;
const MAX_PROSE_WIDTH: usize = 110;
const MAX_COMPOSER_LINES: u16 = 8;

pub fn draw(f: &mut Frame, app: &mut App) {
    // Outer frame border around the whole UI.
    f.render_widget(
        Block::bordered().border_style(Style::default().fg(Color::Indexed(238))),
        f.area(),
    );
    let outer = Block::bordered().inner(f.area());
    let composer_w = (outer.width as usize).saturating_sub(2).max(1);
    let input_h = composer_height(&app.input, composer_w).saturating_add(2);
    let chunks = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(input_h),
        Constraint::Length(STATUS_HEIGHT),
    ])
    .split(outer);

    draw_header(f, app, chunks[0]);
    draw_content(f, app, chunks[1]);
    draw_composer(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);

    if !app.suggestions.is_empty() && !app.busy {
        draw_suggestions(f, app, chunks[2]);
    }
    if app.mention_picker.is_some() {
        draw_mention_picker(f, app, chunks[2]);
    }
    if let Some(p) = &app.pending_permission {
        draw_permission(f, chunks[1], &p.prompt, p.entering_feedback, &p.feedback);
    }
    if let Some(q) = &app.pending_question {
        draw_question(f, chunks[1], q);
    }
    if let Some(p) = &app.model_picker {
        draw_model_picker(f, chunks[1], p);
    }
    if let Some(p) = &app.agent_picker {
        draw_agent_picker(f, chunks[1], p);
    }
    if let Some(p) = &app.mode_picker {
        draw_mode_picker(f, chunks[1], p);
    }
    if let Some(p) = &app.command_palette {
        draw_command_palette(f, chunks[1], p);
    }
    if let Some(p) = &app.session_picker {
        draw_session_picker(f, chunks[1], p, &app.status.workspace);
    }
    if app.leader_active {
        draw_which_key(f);
    }
    if let Some(d) = &app.diff_viewer {
        draw_diff_viewer(f, d);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let max = (area.width as usize).saturating_sub(6).max(10);
    let path = short_path(&app.status.cwd, max / 2);
    let mut title = format!("LightCode · {path}");
    if app.busy {
        title.push_str(&format!(" {}", SPINNER[app.spinner % SPINNER.len()]));
    }
    let mode = format!("[{}]", app.mode.label());
    let mode_color = match app.mode {
        crate::agent::AgentMode::Plan => Color::Cyan,
        crate::agent::AgentMode::Build => Color::Green,
        crate::agent::AgentMode::Auto => Color::Yellow,
    };
    let right = app.status.model.clone();
    let right_len = right.chars().count() + mode.chars().count() + 3;
    let title_len = title.chars().count();
    let pad = area.width as usize;
    let spacer = if title_len + right_len + 1 < pad {
        " ".repeat(pad - title_len - right_len - 1)
    } else {
        String::new()
    };
    f.render_widget(
        Line::from(vec![
            Span::styled(
                title,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(spacer, Style::default()),
            Span::styled(right, Style::default().fg(Color::DarkGray)),
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(
                mode,
                Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        area,
    );
}

fn draw_content(f: &mut Frame, app: &mut App, area: Rect) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    let inner = area;

    let width = inner.width as usize;
    app.content_area = inner;
    let (lines, item_ranges) = build_lines_with_ranges(app, width);
    app.item_ranges = item_ranges;
    let total = lines.len();
    let height = inner.height as usize;
    let max = total.saturating_sub(height);
    if app.auto_scroll {
        app.scroll = max;
    } else if app.scroll >= max {
        app.auto_scroll = true;
        app.pending = 0;
        app.scroll = max;
    }
    let scroll = app.scroll.min(max);
    app.content_scroll = scroll;

    if app.mouse_sel.is_some() {
        // Mouse selection active: render pre-wrapped rows with the highlight.
        let rows =
            super::select::visible_rows(&lines, width, scroll, height, app.mouse_sel.as_ref());
        f.render_widget(Paragraph::new(rows), inner);
    } else {
        let mut render_lines = lines;
        // Subtle active-line highlight under the mouse pointer.
        if let Some(h) = app.mouse_hover {
            let li = scroll + h;
            if li < render_lines.len() {
                if let Some(line) = render_lines.get_mut(li) {
                    for span in line.spans.iter_mut() {
                        span.style = span.style.bg(Color::Indexed(236));
                    }
                }
            }
        }
        f.render_widget(
            Paragraph::new(render_lines)
                .scroll((scroll as u16, 0))
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    if app.pending > 0 {
        let label = format!("  ↓ {} baru  ", app.pending);
        let label_len = label.chars().count() as u16;
        let x = inner.x + inner.width.saturating_sub(label_len);
        let y = inner.y + inner.height.saturating_sub(1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                label,
                Style::default().fg(Color::Cyan),
            ))),
            Rect::new(x, y, label_len.min(area.width), 1),
        );
    }
}

pub(crate) fn build_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    build_lines_with_ranges(app, width).0
}

/// Build the timeline and also return, per timeline item, the inclusive range of
/// terminal rows it occupies (`(start, end)` in the wrapped output). Used for
/// click-to-expand mapping.
pub(crate) fn build_lines_with_ranges(
    app: &App,
    width: usize,
) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
    if app.content.is_empty() {
        return (welcome_lines(&app.status.cwd), Vec::new());
    }
    let mut out = Vec::new();
    let mut raw_ranges: Vec<(usize, usize)> = Vec::new();
    let mut first = true;
    for (index, item) in app.content.iter().enumerate() {
        let raw_start = out.len();
        if let UiBlock::User(_) = item {
            if !first {
                out.push(Line::from(""));
            }
        }
        let selected = app.selected == Some(index);
        match item {
            UiBlock::User(text) => {
                let mut body = Vec::new();
                let content_w = width.saturating_sub(4).max(10);
                for line in wrap_text(text, content_w) {
                    body.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(Color::White),
                    )));
                }
                wrap_bordered(&mut out, "You", width, body, false, None);
            }
            UiBlock::Assistant { text } => {
                out.push(Line::from(Span::styled(
                    "LightCode",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )));
                let wrapped = md_wrap(text, MAX_PROSE_WIDTH);
                for item in md::render(&wrapped) {
                    match item {
                        md::MdItem::Prose(line) => out.push(line),
                        md::MdItem::Code { lang, lines } => {
                            render_code_block(&mut out, lang.as_deref(), &lines, width);
                        }
                    }
                }
            }
            UiBlock::Reasoning { started, done } => {
                let icon = if done.is_some() {
                    "✓".to_string()
                } else {
                    format!("{}", SPINNER[app.spinner % SPINNER.len()])
                };
                let label = match done {
                    Some(d) => format!("  {icon} Thought for {:.1}s", d.as_secs_f32()),
                    None => format!("  {icon} Thinking..."),
                };
                let color = if done.is_some() {
                    Color::DarkGray
                } else {
                    Color::Yellow
                };
                let marker = if selected { "▶" } else { " " };
                out.push(Line::from(Span::styled(
                    format!("{marker}{label}"),
                    Style::default().fg(color),
                )));
                let _ = started;
            }
            UiBlock::Tool(tb) => {
                let expanded = app.item_expanded(index);
                out.push(tool_line(tb, selected));
                match tb.kind {
                    ToolKind::Shell | ToolKind::Write | ToolKind::Edit => {
                        if expanded {
                            tool_body(&mut out, tb);
                        } else {
                            render_tool_collapsed(&mut out, tb, selected);
                        }
                    }
                    _ => render_tool_output(&mut out, tb, expanded, selected),
                }
            }
            UiBlock::Activity(a) => {
                let expanded = app.item_expanded(index);
                let icon = if a.done { "✓" } else { "◌" };
                let header = if a.done {
                    format!("  {icon} {}", a.phase.done_label())
                } else {
                    format!("  {icon} {}...", a.phase.label())
                };
                let header_color = if a.done {
                    Color::DarkGray
                } else {
                    Color::Yellow
                };
                let marker = if selected { "▶" } else { " " };
                out.push(Line::from(Span::styled(
                    format!("{marker}{header}"),
                    Style::default().fg(header_color),
                )));
                let mut has_output = false;
                for item in &a.items {
                    has_output |= !item.output.is_empty();
                    let (icon, color) = match item.status {
                        ActivityStatus::Running => ("◌", Color::Yellow),
                        ActivityStatus::Success => ("✓", Color::Green),
                        ActivityStatus::Failed => ("✗", Color::Red),
                    };
                    let action = activity_item_label(item);
                    out.push(Line::from(vec![
                        Span::styled("    ".to_string(), Style::default()),
                        Span::styled(icon.to_string(), Style::default().fg(color)),
                        Span::styled(format!(" {action}"), Style::default().fg(Color::White)),
                    ]));
                    if expanded && !item.output.is_empty() {
                        for line in item.output.lines() {
                            out.push(Line::from(Span::styled(
                                format!("      {}", truncate(line, 160)),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                    }
                }
                if !expanded && has_output {
                    out.push(Line::from(Span::styled(
                        "      [Show output — Enter]".to_string(),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
            UiBlock::Diff { file, body } => {
                let expanded = app.item_expanded(index);
                render_diff_block(&mut out, file, body, expanded, width, selected);
            }
            UiBlock::Error(text) => {
                out.push(Line::from(Span::styled(
                    format!("⚠ {}", text.lines().next().unwrap_or("")),
                    Style::default().fg(Color::Red),
                )));
                for line in text.lines().skip(1) {
                    out.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Red),
                    )));
                }
            }
        }
        first = false;
        raw_ranges.push((raw_start, out.len()));
    }
    // Responsive wrapping: fit every line to the terminal width, word-wrapping
    // prose and hard-breaking long tokens so nothing overflows to the right.
    let mut wrapped = Vec::with_capacity(out.len());
    let mut ranges = Vec::with_capacity(raw_ranges.len());
    for (s, e) in raw_ranges {
        let ws = wrapped.len();
        let count = e - s;
        for line in out.drain(..count) {
            wrapped.extend(wrap_line(&line, width));
        }
        ranges.push((ws, wrapped.len().saturating_sub(1)));
    }
    (wrapped, ranges)
}

fn welcome_lines(cwd: &str) -> Vec<Line<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    vec![
        Line::from(Span::styled("LightCode", bold)),
        Line::from(Span::styled(short_path(cwd, 60), dim)),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("What would you like to build?", dim)),
        Line::from(""),
        Line::from(Span::styled("  › \"Explain this repository\"", dim)),
        Line::from(Span::styled("  › \"Fix the failing tests\"", dim)),
        Line::from(Span::styled(
            "  › \"Refactor the authentication service\"",
            dim,
        )),
    ]
}

/// Human-readable, semantic label for one activity item.
fn activity_item_label(item: &super::app::ActivityItem) -> String {
    let t = item.target.trim();
    let action = match item.kind {
        ToolKind::Read => format!("read {t}"),
        ToolKind::Grep => fmt_target("searched", t),
        ToolKind::List => format!("listed {t}"),
        ToolKind::Write => format!("wrote {t}"),
        ToolKind::Edit => format!("edited {t}"),
        ToolKind::Shell => shell_semantic(&item.target),
        ToolKind::Git => git_semantic(&item.name),
        ToolKind::Fetch => format!("fetched {t}"),
        ToolKind::Search => fmt_target("searched", t),
        ToolKind::Context => format!("context {t}"),
        ToolKind::Other => match item.name.as_str() {
            "glob" => fmt_target("searched", t),
            "apply_patch" => "applied patch".to_string(),
            "task" => "ran subagent".to_string(),
            "todowrite" => "updated todos".to_string(),
            "question" => "asked a question".to_string(),
            _ => format!("{} {}", item.name, t),
        },
    };
    action
}

fn fmt_target(action: &str, target: &str) -> String {
    if target.is_empty() {
        action.to_string()
    } else {
        format!("{action} {target}")
    }
}

/// Semantic summary of a shell command (git → "inspected history", tests → "ran npm test").
fn shell_semantic(command: &str) -> String {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix("git ") {
        let sub = rest.trim();
        if sub.starts_with("status") {
            "checked git status".to_string()
        } else if sub.starts_with("log") {
            "inspected git history".to_string()
        } else if sub.starts_with("diff") {
            "checked git diff".to_string()
        } else if let Some(h) = sub.strip_prefix("show ") {
            format!(
                "inspected commit {}",
                h.trim().chars().take(7).collect::<String>()
            )
        } else {
            format!("ran git {sub}")
        }
    } else {
        let mut words = c.split_whitespace();
        let prog = words.next().unwrap_or("command");
        let arg = words.next().unwrap_or("");
        if arg.is_empty() {
            prog.to_string()
        } else {
            format!("{prog} {arg}")
        }
    }
}

fn git_semantic(name: &str) -> String {
    match name {
        "git_status" => "checked git status".to_string(),
        "git_log" => "inspected git history".to_string(),
        "git_diff" => "checked git diff".to_string(),
        _ => format!("ran {name}"),
    }
}

fn tool_line(tb: &ToolBlock, selected: bool) -> Line<'static> {
    let (icon, color) = match tb.state {
        ToolState::Running => ("◌", Color::Yellow),
        ToolState::Success => ("✓", Color::Green),
        ToolState::Failed => ("✗", Color::Red),
    };
    let marker = if selected { "▶ " } else { "  " };
    let target = truncate(&tb.target, 100);
    let target_span = match tb.kind {
        ToolKind::Shell => Span::styled(
            format!("$ {target}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        ToolKind::Write | ToolKind::Edit => Span::styled(
            target,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        _ => Span::styled(target, Style::default().fg(Color::White)),
    };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Cyan)),
        Span::styled(
            icon.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(
            format!("{:<6}", tb.kind.label()),
            Style::default().fg(color),
        ),
        target_span,
    ])
}

/// Extra body for a tool block rendered before the raw output.
fn tool_body(out: &mut Vec<Line<'static>>, tb: &ToolBlock) {
    match tb.kind {
        ToolKind::Shell => {
            for line in tb.output.lines() {
                out.push(Line::from(Span::styled(
                    format!("  {}", truncate(line, 200)),
                    Style::default().fg(Color::White),
                )));
            }
        }
        ToolKind::Write | ToolKind::Edit => {
            for line in tb.output.lines() {
                out.push(Line::from(Span::styled(
                    format!("  {}", truncate(line, 200)),
                    diff_style(line),
                )));
            }
        }
        _ => {}
    }
}

/// A one-line summary of a tool's output (grep → "N matches", shell → exit code).
fn tool_summary(tb: &ToolBlock) -> Option<String> {
    let out = tb.output.trim();
    if out.is_empty() {
        return None;
    }
    let s = match tb.kind {
        ToolKind::Grep => {
            let n = out
                .lines()
                .filter(|l| {
                    let mut it = l.splitn(3, ':');
                    it.next();
                    it.next().is_some_and(|x| x.parse::<usize>().is_ok())
                })
                .count();
            format!("{n} matches")
        }
        ToolKind::Read => {
            let n = out.lines().filter(|l| l.contains(" | ")).count();
            format!("{n} lines")
        }
        ToolKind::List => {
            let first = out.lines().next().unwrap_or("").trim().to_string();
            if first.is_empty() {
                format!("{} lines", out.lines().count())
            } else {
                first
            }
        }
        ToolKind::Shell => {
            if let Some(i) = out.find("test result: ok.") {
                let rest = &out[i..];
                if let Some(p) = rest.split_whitespace().find(|w| w.parse::<usize>().is_ok()) {
                    format!("{p} passed")
                } else {
                    "tests passed".to_string()
                }
            } else if let Some(i) = out.rfind("exit code: ") {
                let code = out[i + "exit code: ".len()..].trim();
                // Successful exit codes are implementation details; only show failures.
                if code == "0" {
                    return None;
                }
                format!("exit {code}")
            } else {
                truncate(out.lines().next().unwrap_or(""), 60)
            }
        }
        ToolKind::Write | ToolKind::Edit => truncate(out.lines().next().unwrap_or(""), 80),
        ToolKind::Git => {
            let n = out.lines().count();
            if n <= 1 {
                truncate(out, 60)
            } else {
                format!("{n} lines")
            }
        }
        _ => {
            let n = out.lines().count();
            if n <= 1 {
                return None;
            }
            format!("{n} lines")
        }
    };
    Some(s)
}

/// Collapsed tool block: summary line + expand hint.
fn render_tool_collapsed(out: &mut Vec<Line<'static>>, tb: &ToolBlock, selected: bool) {
    if let Some(summary) = tool_summary(tb) {
        let style = if selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        out.push(Line::from(Span::styled(format!("      {summary}"), style)));
    }
    if tb.output.lines().count() > 1 || tb.output.len() > 120 {
        out.push(Line::from(Span::styled(
            "      [Show output — Enter]".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    }
}

fn render_tool_output(
    out: &mut Vec<Line<'static>>,
    tb: &ToolBlock,
    expanded: bool,
    selected: bool,
) {
    if tb.output.trim().is_empty() {
        return;
    }
    if !expanded {
        render_tool_collapsed(out, tb, selected);
        return;
    }
    let lines: Vec<&str> = tb.output.lines().collect();
    for line in lines {
        out.push(Line::from(Span::styled(
            format!("  {}", truncate(line, 180)),
            diff_style(line),
        )));
    }
}

/// A parsed row of a unified diff.
struct DiffRow {
    old: Option<u32>,
    new: Option<u32>,
    kind: char,
    text: String,
}

fn parse_unified_diff(body: &str) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let mut old_ln = 0u32;
    let mut new_ln = 0u32;
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            if let (Some(a), Some(c)) = (hunk_start(rest, '-'), hunk_start(rest, '+')) {
                old_ln = a;
                new_ln = c;
            }
            rows.push(DiffRow {
                old: None,
                new: None,
                kind: '@',
                text: line.to_string(),
            });
            continue;
        }
        if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            rows.push(DiffRow {
                old: None,
                new: None,
                kind: 'h',
                text: line.to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            rows.push(DiffRow {
                old: Some(old_ln),
                new: None,
                kind: '-',
                text: rest.to_string(),
            });
            old_ln += 1;
        } else if let Some(rest) = line.strip_prefix('+') {
            rows.push(DiffRow {
                old: None,
                new: Some(new_ln),
                kind: '+',
                text: rest.to_string(),
            });
            new_ln += 1;
        } else if let Some(rest) = line.strip_prefix(' ') {
            rows.push(DiffRow {
                old: Some(old_ln),
                new: Some(new_ln),
                kind: ' ',
                text: rest.to_string(),
            });
            old_ln += 1;
            new_ln += 1;
        } else {
            rows.push(DiffRow {
                old: None,
                new: None,
                kind: 'h',
                text: line.to_string(),
            });
        }
    }
    rows
}

fn hunk_start(rest: &str, sign: char) -> Option<u32> {
    let idx = rest.find(sign)?;
    let digits: String = rest[idx + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn render_diff_block(
    out: &mut Vec<Line<'static>>,
    file: &str,
    body: &str,
    expanded: bool,
    width: usize,
    selected: bool,
) {
    let rows = parse_unified_diff(body);
    let width = width.max(10);
    // Reserve room for the "│ " border + " 12345 12345 │ " number gutter.
    let gutter = 2 + 2 + 5 + 1 + 5 + 1 + 1; // indent + old:5 + space + new:5 + space + │ + space
    let text_w = width.saturating_sub(gutter + 4);
    let shown = rows.len().min(if expanded { rows.len() } else { 12 });
    let mut body_lines: Vec<Line> = Vec::new();
    for row in &rows[..shown] {
        match row.kind {
            '@' => body_lines.push(Line::from(Span::styled(
                truncate_width(&row.text, width - 6),
                Style::default().fg(Color::Cyan),
            ))),
            'h' => body_lines.push(Line::from(Span::styled(
                truncate_width(&row.text, width - 6),
                Style::default().fg(Color::DarkGray),
            ))),
            _ => {
                let old = row.old.map(|n| n.to_string()).unwrap_or_default();
                let new = row.new.map(|n| n.to_string()).unwrap_or_default();
                let (sign, color) = match row.kind {
                    '-' => ("- ", Color::Red),
                    '+' => ("+ ", Color::Green),
                    _ => ("  ", Color::DarkGray),
                };
                body_lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {old:>5} {new:>5} │ "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(
                            "{sign}{}",
                            truncate_width(&row.text, text_w.saturating_sub(2))
                        ),
                        Style::default().fg(color),
                    ),
                ]));
            }
        }
    }
    let hint = if rows.len() > shown {
        Some(format!("… +{} baris · [Show — Enter]", rows.len() - shown))
    } else {
        None
    };
    wrap_bordered(out, file, width, body_lines, selected, hint.as_deref());
}

/// Draw a bordered block: top/title, padded body rows, optional hint, bottom.
fn wrap_bordered(
    out: &mut Vec<Line<'static>>,
    title: &str,
    width: usize,
    body: Vec<Line<'static>>,
    selected: bool,
    hint: Option<&str>,
) {
    let gray = Style::default().fg(Color::DarkGray);
    let width = width.max(10);
    let dashes = width - 2; // "┌" + dashes + "┐"
    let content_w = width - 4; // "│ " + content + " │"
    let t = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let dash_count = dashes.saturating_sub(t.chars().count()).max(2);
    let top_color = if selected {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    out.push(Line::from(Span::styled(
        format!("┌{t}{}┐", "─".repeat(dash_count)),
        Style::default().fg(top_color),
    )));
    for line in body {
        let w: usize = line
            .spans
            .iter()
            .map(|s| display_w(s.content.as_ref()))
            .sum();
        let pad = content_w.saturating_sub(w);
        let mut spans = vec![Span::styled("│ ", gray)];
        spans.extend(line.spans);
        spans.push(Span::styled(format!("{} │", " ".repeat(pad)), gray));
        out.push(Line::from(spans));
    }
    if let Some(h) = hint {
        let pad = content_w.saturating_sub(display_w(h));
        out.push(Line::from(vec![
            Span::styled("│ ".to_string(), gray),
            Span::styled(h.to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} │", " ".repeat(pad)), gray),
        ]));
    }
    out.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(dashes)),
        gray,
    )));
}

/// A bordered code block with a language header and line numbers.
fn render_code_block(
    out: &mut Vec<Line<'static>>,
    lang: Option<&str>,
    lines: &[String],
    width: usize,
) {
    let width = width.max(10);
    let gray = Style::default().fg(Color::DarkGray);
    let num_w = lines.len().to_string().len();
    // Row layout: "│ " + gutter(number + " │ ") + code + "│".
    let gutter = 2 + num_w + 3; // "│ ", num_w, " │ "
    let content_w = width.saturating_sub(gutter + 1);
    let t = lang.unwrap_or("code");
    let dash = width.saturating_sub(t.chars().count() + 3).max(2);
    out.push(Line::from(Span::styled(
        format!("┌ {t} {}┐", "─".repeat(dash)),
        gray,
    )));
    for (i, line) in lines.iter().enumerate() {
        let code = truncate_width(line, content_w);
        let pad = content_w.saturating_sub(display_w(&code));
        out.push(Line::from(vec![
            Span::styled(format!("│ {:>num_w$} │ ", i + 1), gray),
            Span::styled(format!("{code}{}", " ".repeat(pad)), code_style()),
            Span::styled("│".to_string(), gray),
        ]));
    }
    if lines.is_empty() {
        out.push(Line::from(Span::styled(
            format!("│ {} │", " ".repeat(width.saturating_sub(4))),
            gray,
        )));
    }
    out.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(width - 2)),
        gray,
    )));
}

fn code_style() -> Style {
    Style::default().fg(Color::Yellow)
}

/// Display width of a string in terminal columns.
fn display_w(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Wrap a plain text into segments that fit `width` columns: word-wrap at
/// spaces and hard-break tokens longer than the width.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut last_space: Option<usize> = None; // byte offset just past the last space
    for g in text.graphemes(true) {
        let gw = display_w(g);
        let is_space = g.trim().is_empty();
        if is_space {
            cur.push_str(g);
            cur_w += gw;
            last_space = Some(cur.len());
        } else {
            if cur_w + gw > width {
                if let Some(bs) = last_space {
                    let rest = cur.split_off(bs);
                    out.push(cur);
                    cur = rest;
                    last_space = None;
                    cur_w = display_w(&cur);
                } else if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
            }
            cur.push_str(g);
            cur_w += gw;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Wrap a styled line to fit `width` columns, preserving span styling.
fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let segments = wrap_text(&text, width);
    let tokens: Vec<(String, Style)> = line
        .spans
        .iter()
        .flat_map(|s| {
            s.content
                .as_ref()
                .graphemes(true)
                .map(|g| (g.to_string(), s.style))
        })
        .collect();
    let mut idx = 0usize;
    segments
        .into_iter()
        .map(|seg| {
            let seg_bytes = seg.len();
            let mut parts: Vec<(String, Style)> = Vec::new();
            let mut used = 0usize;
            while used < seg_bytes {
                let (g, st) = &tokens[idx];
                parts.push((g.clone(), *st));
                used += g.len();
                idx += 1;
            }
            line_from_parts(&parts)
        })
        .collect()
}

/// Number of terminal rows a text occupies when wrapped at `width`.
fn wrapped_rows(text: &str, width: usize) -> usize {
    wrap_text(text, width).len()
}

/// Merge consecutive (grapheme, style) pairs into styled spans.
fn line_from_parts(parts: &[(String, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (g, style) in parts {
        match spans.last_mut() {
            Some(Span { content, style: s }) if *s == *style => {
                content.to_mut().push_str(g);
            }
            _ => spans.push(Span::styled(g.clone(), *style)),
        }
    }
    Line::from(spans)
}

/// Truncate a string to at most `max` display columns (UTF-8 safe).
fn truncate_width(s: &str, max: usize) -> String {
    if display_w(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    format!("{out}…")
}

fn draw_composer(f: &mut Frame, app: &mut App, area: Rect) {
    let is_paste: Vec<bool> = (0..app.input.line_count())
        .map(|row| {
            let text: String = app.input.line_graphemes(row).concat();
            app.is_paste_placeholder(&text)
        })
        .collect();

    let ed = &mut app.input;
    let block = Block::bordered()
        .title(" Input ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 1 || inner.width < 2 {
        return;
    }
    let width = inner.width as usize;
    if ed.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "› Ask LightCode...".to_string(),
                Style::default().fg(Color::DarkGray),
            ))),
            inner,
        );
        return;
    }

    // Wrap every logical line into terminal rows; locate the cursor row/col.
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cursor_trow = 0usize;
    let mut cursor_col = 0usize;
    for li in 0..ed.line_count() {
        let flag = is_paste.get(li).copied().unwrap_or(false);
        let styled = composer_line(ed, li, flag);
        let wrapped = wrap_line(&styled, width);
        if li == ed.cursor().row {
            let (r, c) = pos_of_col(&wrapped, 2 + ed.cursor_col_width());
            cursor_trow = rows.len() + r;
            cursor_col = c;
        }
        rows.extend(wrapped);
    }

    let height = inner.height as usize;
    let scroll = cursor_trow.saturating_sub(height.saturating_sub(1));
    let scroll = scroll.min(rows.len().saturating_sub(height));
    let end = (scroll + height).min(rows.len());
    let visible: Vec<Line> = rows[scroll..end].to_vec();
    f.render_widget(Paragraph::new(visible), inner);

    let x = inner.x + (cursor_col.min(inner.width as usize - 1) as u16);
    let y = inner.y + (cursor_trow - scroll) as u16;
    f.set_cursor_position((x, y));
}

/// Height the composer should occupy: the wrapped row count, capped.
fn composer_height(ed: &TextEditor, width: usize) -> u16 {
    let mut rows = 0usize;
    for li in 0..ed.line_count() {
        let text: String = ed.line_graphemes(li).concat();
        rows += wrapped_rows(&text, width);
    }
    rows.clamp(1, MAX_COMPOSER_LINES as usize) as u16
}

/// Which wrapped row of `wrapped` contains display column `col`, and the column
/// within that row.
fn pos_of_col(wrapped: &[Line<'static>], col: usize) -> (usize, usize) {
    let mut w = 0usize;
    for (r, line) in wrapped.iter().enumerate() {
        let lw = display_w(&line.to_string());
        if w + lw > col {
            return (r, col.saturating_sub(w).min(lw));
        }
        w += lw;
    }
    if wrapped.is_empty() {
        return (0, 0);
    }
    let last = wrapped.len() - 1;
    let lw = display_w(&wrapped[last].to_string());
    (last, col.saturating_sub(w).min(lw))
}

/// One line of the composer: prefix + graphemes, with selection highlighting.
/// Paste placeholders get a distinct look.
fn composer_line(ed: &TextEditor, row: usize, is_paste: bool) -> Line<'static> {
    let prefix = if row == 0 { "› " } else { "  " };
    let graphemes = ed.line_graphemes(row);
    let sel = ed.selection_for_row(row);
    let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Cyan))];
    if is_paste && sel.is_none() {
        spans.push(Span::styled(
            graphemes.concat(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }
    match sel {
        Some((s, e)) => {
            let s = s.min(graphemes.len());
            let e = e.min(graphemes.len());
            let before: String = graphemes[..s].concat();
            let selected: String = graphemes[s..e].concat();
            let after: String = graphemes[e..].concat();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            spans.push(Span::styled(
                selected,
                Style::default().fg(Color::Black).bg(Color::White),
            ));
            if !after.is_empty() {
                spans.push(Span::raw(after));
            }
        }
        None => {
            if !graphemes.is_empty() {
                spans.push(Span::raw(graphemes.concat()));
            }
        }
    }
    Line::from(spans)
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let mut left = format!("{} · {}", app.status.provider, app.status.model);
    if app.busy {
        left.push_str(&format!(" · {}", SPINNER[app.spinner % SPINNER.len()]));
    } else if let Some(err) = &app.last_error {
        left.push_str(&format!(" · ⚠ {}", truncate(err, 60)));
    }
    let right = "⌘K commands · ⌘X leader".to_string();
    let pad = area.width as usize;
    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let spacer = if left_len + right_len + 1 < pad {
        " ".repeat(pad - left_len - right_len - 1)
    } else {
        String::new()
    };
    f.render_widget(
        Line::from(vec![
            Span::styled(left, Style::default().fg(Color::DarkGray)),
            Span::styled(spacer, Style::default()),
            Span::styled(right, Style::default().fg(Color::DarkGray)),
        ]),
        area,
    );

    // Transient toast, bottom-right, disappears after ~4s.
    if let Some((msg, at)) = &app.toast {
        if at.elapsed() < std::time::Duration::from_secs(4) {
            let label = format!("  {msg}  ");
            let len = label.chars().count() as u16;
            let x = area.x + area.width.saturating_sub(len);
            let y = area.y.saturating_sub(1);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))),
                Rect::new(x, y, len.min(area.width), 1),
            );
        }
    }
}

fn draw_mention_picker(f: &mut Frame, app: &App, input_area: Rect) {
    let Some(p) = &app.mention_picker else { return };
    if p.results.is_empty() {
        return;
    }
    // Grow to the available space above the composer so many results fit.
    let avail = input_area.y as usize;
    let max_h = avail.saturating_sub(1).clamp(4, 20);
    let height = (p.results.len() as u16 + 2).clamp(4, max_h as u16);
    let area = Rect::new(
        input_area.x,
        input_area.y.saturating_sub(height),
        input_area.width,
        height,
    );
    if area.height < 3 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(format!(" Files · @{} ", p.query))
            .border_style(Style::default().fg(Color::DarkGray)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let mut lines = Vec::new();
    for (i, m) in p.results.iter().enumerate() {
        let marker = if i == p.selected { "› " } else { "  " };
        let label = if m.is_dir {
            format!("{}/", m.path)
        } else {
            m.path.clone()
        };
        let style = if i == p.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(format!("{marker}{label}"), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_suggestions(f: &mut Frame, app: &App, input_area: Rect) {
    let height = (app.suggestions.len() as u16 + 2).clamp(3, 12);
    let area = Rect::new(
        input_area.x,
        input_area.y.saturating_sub(height),
        input_area.width,
        height,
    );
    if area.height < 3 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" suggestions ")
            .border_style(Style::default().fg(Color::DarkGray)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let lines: Vec<Line> = app
        .suggestions
        .iter()
        .map(|s| {
            Line::from(Span::styled(
                s.clone(),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_question(f: &mut Frame, app_area: Rect, q: &super::app::QuestionRequest) {
    let height = (q.options.len() as u16 + 4).clamp(6, 14);
    let area = centered_rect(64, height, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" Question ")
            .border_style(Style::default().fg(Color::Yellow)),
        area,
    );
    let inner = area.inner(Margin::new(2, 1));
    let mut lines = vec![Line::from(Span::styled(
        q.prompt.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    for (i, opt) in q.options.iter().enumerate() {
        let marker = if i == q.selected { "▶ " } else { "  " };
        let style = if i == q.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(format!("{marker}{opt}"), style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[↑/↓] pilih   [Enter] jawab   [Esc] batal",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_permission(
    f: &mut Frame,
    app_area: Rect,
    prompt: &str,
    entering_feedback: bool,
    feedback: &str,
) {
    let area = centered_rect(64, 8, app_area);
    if area.width < 10 || area.height < 4 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" LightCode wants to run ")
            .border_style(Style::default().fg(Color::Yellow)),
        area,
    );
    let inner = area.inner(Margin::new(2, 1));
    let mut lines = vec![Line::from(Span::styled(
        prompt.to_string(),
        Style::default().fg(Color::White),
    ))];
    lines.push(Line::from(""));
    if entering_feedback {
        lines.push(Line::from(Span::styled(
            format!("Alasan penolakan: {}", feedback),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(Span::styled(
            "[Enter] tolak dengan alasan   [Esc] batal",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "[Enter] Allow    [Esc] Deny    [A] Session    [W] Always    [R] Deny + alasan",
            Style::default().fg(Color::Cyan),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_mode_picker(f: &mut Frame, app_area: Rect, picker: &super::app::ModePicker) {
    let height = picker.modes.len() as u16 + 4;
    let area = centered_rect(56, height, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" Agent Mode ")
            .border_style(Style::default().fg(Color::Cyan)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let mut lines = Vec::new();
    for (i, (mode, desc)) in picker.modes.iter().enumerate() {
        let marker = if i == picker.selected { "› " } else { "  " };
        let style = if i == picker.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{:<6} {}", mode.label(), desc),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Shift+Tab to cycle modes",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_agent_picker(f: &mut Frame, app_area: Rect, picker: &super::app::AgentPicker) {
    let height = (picker.agents.len() as u16 + 2).clamp(5, 12);
    let area = centered_rect(50, height, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" Agent ")
            .border_style(Style::default().fg(Color::Magenta)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let mut lines = Vec::new();
    for (i, (name, model)) in picker.agents.iter().enumerate() {
        let marker = if i == picker.selected { "▶ " } else { "  " };
        let label = if model.is_empty() {
            name.clone()
        } else {
            format!("{name}  ·  {model}")
        };
        let style = if i == picker.selected {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(format!("{marker}{label}"), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_model_picker(f: &mut Frame, app_area: Rect, picker: &super::app::ModelPicker) {
    let height = (picker.models.len() as u16 + 4).clamp(5, 16);
    let area = centered_rect(62, height, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" Models ")
            .border_style(Style::default().fg(Color::Cyan)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    // Build every row (headers + items) and scroll the paragraph so the
    // selected item stays visible; headers push content down naturally.
    let mut full_lines: Vec<Line> = Vec::new();
    let mut selected_line = 0usize;
    let mut last_provider = String::new();
    for (i, item) in picker.models.iter().enumerate() {
        if item.provider != last_provider {
            full_lines.push(Line::from(Span::styled(
                format!("── {} ──", item.provider),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )));
            last_provider = item.provider.clone();
        }
        let marker = if i == picker.selected { "▶ " } else { "  " };
        let label = if item.name.is_empty() {
            item.id.clone()
        } else {
            format!("{}  ·  {}", item.id, item.name)
        };
        let style = if i == picker.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        full_lines.push(Line::from(Span::styled(format!("{marker}{label}"), style)));
        if i == picker.selected {
            selected_line = full_lines.len() - 1;
        }
    }
    let visible = inner.height as usize;
    let scroll = selected_line.saturating_sub(visible.saturating_sub(1));
    f.render_widget(Paragraph::new(full_lines).scroll((scroll as u16, 0)), inner);
}

fn draw_command_palette(f: &mut Frame, app_area: Rect, palette: &super::app::CommandPalette) {
    let cmds = App::palette_commands(palette);
    let area = centered_rect(50, cmds.len() as u16 + 4, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" Commands ")
            .border_style(Style::default().fg(Color::Cyan)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let mut lines = vec![Line::from(Span::styled(
        format!("› {}", palette.filter),
        Style::default().fg(Color::DarkGray),
    ))];
    for (i, cmd) in cmds.iter().enumerate() {
        let marker = if i == palette.selected { "▶ " } else { "  " };
        let style = if i == palette.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}", App::command_label(*cmd)),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_session_picker(
    f: &mut Frame,
    app_area: Rect,
    picker: &super::app::SessionPicker,
    workspace: &str,
) {
    let cmds: Vec<_> = super::filtered_sessions(picker);
    let height = (cmds.len() as u16 + 4).clamp(6, 16);
    let area = centered_rect(72, height, app_area);
    if area.width < 10 || area.height < 5 {
        return;
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(format!(" Sessions · {} ", short_path(workspace, 40)))
            .border_style(Style::default().fg(Color::Cyan)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let mut full_lines = vec![Line::from(Span::styled(
        format!(
            "› {}   [Enter] buka  [Del] hapus  [Esc] tutup",
            picker.filter
        ),
        Style::default().fg(Color::DarkGray),
    ))];
    let mut selected_line = 1usize;
    for (i, item) in cmds.iter().enumerate() {
        let marker = if i == picker.selected { "▶ " } else { "  " };
        let style = if i == picker.selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let label = if item.title.is_empty() {
            format!("{}  ·  {}", item.id, item.created_at)
        } else {
            format!("{}  ·  {}  ·  {}", item.title, item.id, item.created_at)
        };
        full_lines.push(Line::from(Span::styled(format!("{marker}{label}"), style)));
        if i == picker.selected {
            selected_line = full_lines.len() - 1;
        }
    }
    let visible = inner.height as usize;
    let scroll = selected_line.saturating_sub(visible.saturating_sub(1));
    f.render_widget(Paragraph::new(full_lines).scroll((scroll as u16, 0)), inner);
}

fn draw_which_key(f: &mut Frame) {
    let rows = vec![
        ("n", "new session"),
        ("l", "sessions"),
        ("m", "models"),
        ("g", "agent"),
        ("s", "stats"),
        ("c", "clear"),
        ("h", "help"),
        ("d", "debug"),
        ("q", "quit"),
    ];
    let width = 34u16;
    let height = rows.len() as u16 + 2;
    let x = f.area().width.saturating_sub(width + 2);
    let y = 1;
    let area = Rect::new(x, y, width.min(f.area().width), height);
    f.render_widget(Clear, area);
    f.render_widget(
        Block::bordered()
            .title(" ⌘X leader ")
            .border_style(Style::default().fg(Color::Cyan)),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, label)| {
            Line::from(vec![
                Span::styled(
                    format!("  {k} "),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(label.to_string(), Style::default().fg(Color::White)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_diff_viewer(f: &mut Frame, d: &super::app::DiffViewer) {
    let area = f.area();
    f.render_widget(Clear, area);
    let rows = parse_unified_diff(&d.body);
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for row in &rows {
        let (prefix, color) = match row.kind {
            '@' => ("@@", Color::Cyan),
            '-' => ("  ", Color::Red),
            '+' => ("  ", Color::Green),
            ' ' => ("  ", Color::DarkGray),
            _ => ("  ", Color::DarkGray),
        };
        lines.push(Line::from(vec![
            Span::styled(
                prefix.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(truncate(&row.text, 500), Style::default().fg(color)),
        ]));
    }
    let scroll = d.scroll.min(lines.len().saturating_sub(1));
    f.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)).block(
            Block::bordered()
                .title(" Diff — ↑/↓ scroll, Esc tutup ")
                .border_style(Style::default().fg(Color::Green)),
        ),
        area,
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = (area.width * percent_x / 100).min(area.width);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, w.min(area.width), height.min(area.height))
}

fn short_path(p: &str, max: usize) -> String {
    let p = if let Some(home) = std::env::var_os("HOME") {
        match p.strip_prefix(home.to_string_lossy().as_ref()) {
            Some(rest) if !rest.is_empty() => format!("~{rest}"),
            _ => p.to_string(),
        }
    } else {
        p.to_string()
    };
    if p.chars().count() <= max {
        p
    } else {
        let keep = max.saturating_sub(1);
        let left = keep / 2;
        let head: String = p.chars().take(left).collect();
        let tail: String = p
            .chars()
            .rev()
            .take(keep - left)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    }
}

/// Wrap prose lines at a readable width, leaving code fences untouched.
fn md_wrap(text: &str, width: usize) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in text.split('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push(line.to_string());
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }
        out.extend(wrap_prose(line, width));
    }
    out.join("\n")
}

fn wrap_prose(line: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split_inclusive(' ') {
        if !cur.is_empty() && cur.chars().count() + word.chars().count() > width {
            out.push(cur.trim_end().to_string());
            cur = String::new();
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur.trim_end().to_string());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn diff_style(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_diff_with_line_numbers() {
        let body = "diff --git a/x.rs b/x.rs\nindex abc..def 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-println!(\"old\");\n+println!(\"new\");\n fn b() {}\n";
        let rows = parse_unified_diff(body);
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[4].kind, '@');
        assert_eq!(rows[5].kind, ' ');
        assert_eq!(rows[5].old, Some(1));
        assert_eq!(rows[5].new, Some(1));
        assert_eq!(rows[6].kind, '-');
        assert_eq!(rows[6].old, Some(2));
        assert_eq!(rows[6].new, None);
        assert_eq!(rows[7].kind, '+');
        assert_eq!(rows[7].old, None);
        assert_eq!(rows[7].new, Some(2));
        assert_eq!(rows[8].new, Some(3));
    }

    #[test]
    fn wraps_prose_at_width() {
        let line = "aaa bbb ccc ddd";
        let out = wrap_prose(line, 8);
        assert_eq!(out.len(), 2);
        assert!(out[0].chars().count() <= 8);
    }

    #[test]
    fn wrap_line_breaks_long_tokens() {
        // Word wrap: prose fits the width.
        let l = Line::raw("aaa bbb ccc ddd".to_string());
        let wrapped = wrap_line(&l, 8);
        assert!(wrapped.len() > 1);
        for w in &wrapped {
            assert!(display_w(&w.to_string()) <= 8, "row too wide: {w}");
        }
        // Long unbreakable token must be hard-broken, not overflow.
        let long = "a".repeat(30);
        let l = Line::raw(long.clone());
        let wrapped = wrap_line(&l, 10);
        assert!(wrapped.len() >= 3, "long token must be split");
        for w in &wrapped {
            assert!(display_w(&w.to_string()) <= 10, "overflow: {w}");
        }
        let joined: String = wrapped.iter().map(|l| l.to_string()).collect();
        assert_eq!(joined, long);
    }

    #[test]
    fn short_path_replaces_home() {
        let home = std::env::var_os("HOME").unwrap();
        let p = format!("{}/Code/lightcode", home.to_string_lossy());
        assert!(short_path(&p, 100).starts_with("~/Code/lightcode"));
    }

    fn tb(kind: ToolKind, output: &str) -> ToolBlock {
        ToolBlock {
            kind,
            target: String::new(),
            state: ToolState::Success,
            output: output.to_string(),
        }
    }

    #[test]
    fn tool_summaries() {
        assert_eq!(
            tool_summary(&tb(ToolKind::Grep, "a.rs:1:foo\nb.rs:2:foo\n")),
            Some("2 matches".to_string())
        );
        assert_eq!(
            tool_summary(&tb(
                ToolKind::Shell,
                "$ cargo test\n...\ntest result: ok. 42 passed\n"
            )),
            Some("42 passed".to_string())
        );
        assert_eq!(
            tool_summary(&tb(ToolKind::Shell, "$ true\nexit code: 0\n")),
            None // successful exit codes are implementation details
        );
        assert_eq!(
            tool_summary(&tb(ToolKind::Shell, "$ false\nexit code: 1\n")),
            Some("exit 1".to_string())
        );
        assert_eq!(
            tool_summary(&tb(
                ToolKind::Read,
                "=== a.rs ===\n    1 | fn x\n    2 | fn y\n"
            )),
            Some("2 lines".to_string())
        );
        assert!(tool_summary(&tb(ToolKind::Other, "ok")).is_none());
    }

    #[test]
    fn bordered_block_has_edges_and_title() {
        let mut out = Vec::new();
        let body = vec![Line::from(Span::styled(
            "  hello".to_string(),
            Style::default(),
        ))];
        wrap_bordered(&mut out, "src/a.rs", 30, body, false, None);
        let first = out.first().unwrap().to_string();
        let last = out.last().unwrap().to_string();
        assert!(first.starts_with('┌') && first.ends_with('┐'));
        assert!(first.contains("src/a.rs"));
        assert!(last.starts_with('└') && last.ends_with('┘'));
        // body row is bordered and padded to the width
        assert_eq!(out[1].to_string().chars().count(), 30);
    }

    #[test]
    fn truncate_width_respects_columns() {
        assert_eq!(truncate_width("abcdef", 10), "abcdef");
        let t = truncate_width("abcdefghijklmnop", 8);
        assert!(t.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(t.as_str()) <= 8);
    }

    #[test]
    fn code_block_has_single_border() {
        let mut out = Vec::new();
        render_code_block(&mut out, Some("rust"), &["fn main() {}".to_string()], 40);
        assert!(out[0].to_string().starts_with('┌'));
        assert!(out.last().unwrap().to_string().starts_with('└'));
        let row = out[1].to_string();
        assert!(row.starts_with("│ 1 │ "), "expected gutter, got: {row}");
        assert!(!row.starts_with("│ │"), "double border: {row}");
        assert!(row.ends_with('│'));
        assert_eq!(row.chars().count(), 40, "row must be exactly width wide");
    }
}

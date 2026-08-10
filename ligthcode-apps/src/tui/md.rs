use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

fn code_style() -> Style {
    Style::default().fg(Color::Yellow)
}

fn heading_style() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

/// A rendered markdown item. Code fences become dedicated `Code` blocks so the
/// TUI can draw them with a border, line numbers, and a language header.
pub enum MdItem {
    Prose(Line<'static>),
    Code {
        lang: Option<String>,
        lines: Vec<String>,
    },
}

/// Render markdown-ish text into styled items. Supports paragraphs, headings,
/// lists, inline code, code fences, and bold. Unclosed code fences render the
/// remainder as a code block (streaming-friendly).
pub fn render(text: &str) -> Vec<MdItem> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_lang: Option<String> = None;
    let mut fence_lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end();
        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
                out.push(MdItem::Code {
                    lang: fence_lang.take(),
                    lines: std::mem::take(&mut fence_lines),
                });
                continue;
            }
            fence_lines.push(line.to_string());
            continue;
        }
        if line.trim_start().starts_with("```") {
            in_fence = true;
            fence_lang =
                Some(line.trim_start_matches('`').trim().to_string()).filter(|s| !s.is_empty());
            continue;
        }
        if line.trim().is_empty() {
            out.push(MdItem::Prose(Line::from("")));
            continue;
        }
        if let Some(rest) = heading_of(line) {
            out.push(MdItem::Prose(Line::from(Span::styled(
                rest.to_string(),
                heading_style(),
            ))));
            continue;
        }
        if let Some((indent, rest)) = list_marker(line) {
            let mut spans = vec![Span::styled(
                if indent { "    " } else { "" }.to_string(),
                Style::default(),
            )];
            spans.extend(inline(rest));
            out.push(MdItem::Prose(Line::from(spans)));
            continue;
        }
        out.push(MdItem::Prose(Line::from(inline(line))));
    }
    if in_fence {
        out.push(MdItem::Code {
            lang: fence_lang,
            lines: fence_lines,
        });
    }
    out
}

/// Split a line into spans: `code`, **bold**, and plain text.
fn inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    // First split on code backticks; even segments are plain (may hold **bold**).
    for (i, part) in text.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            spans.push(Span::styled(part.to_string(), code_style()));
        } else {
            spans.extend(bold(part));
        }
    }
    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }
    spans
}

fn bold(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, part) in text.split("**").enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            spans.push(Span::styled(
                part.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(part.to_string()));
        }
    }
    spans
}

fn heading_of(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t.starts_with('#') {
        Some(t.trim_start_matches('#').trim_start())
    } else {
        None
    }
}

fn list_marker(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    let indented = line.len() != t.len();
    for p in ["- ", "* ", "• ", "> ", "+ "] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some((indented, rest));
        }
    }
    let b = t.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_digit() && b[1] == b'.' {
        return Some((indented, t[2..].trim_start()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[MdItem]) -> String {
        lines
            .iter()
            .map(|item| match item {
                MdItem::Prose(l) => l.to_string(),
                MdItem::Code { lines, .. } => lines.join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_paragraphs_headings_and_lists() {
        let text = "# Title\n\nhello world\n- item one\n- item two\n";
        let out = plain(&render(text));
        assert!(out.contains("Title"));
        assert!(out.contains("hello world"));
        assert!(out.contains("item one"));
        assert!(out.contains("item two"));
    }

    #[test]
    fn keeps_code_fence_content() {
        let text = "```rust\nfn main() {}\n```\nafter\n";
        let items = render(text);
        let code = items
            .iter()
            .find_map(|i| match i {
                MdItem::Code { lang, lines } => {
                    Some((lang.clone().unwrap_or_default(), lines.join("\n")))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(code.0, "rust");
        assert!(code.1.contains("fn main() {}"));
        assert!(plain(&items).contains("after"));
    }

    #[test]
    fn unclosed_fence_is_streaming_safe() {
        let text = "```\npartial code";
        let items = render(text);
        assert!(matches!(
            &items[..],
            [MdItem::Code { lines, .. }] if lines.join("\n") == "partial code"
        ));
    }

    #[test]
    fn inline_code_split_keeps_text() {
        let text = "run `cargo test` now";
        let items = render(text);
        let MdItem::Prose(line) = &items[0] else {
            panic!("expected prose");
        };
        let spans = &line.spans;
        assert!(spans.len() >= 3);
        assert_eq!(spans[0].content, "run ");
        assert_eq!(spans[1].content, "cargo test");
        assert_eq!(spans[2].content, " now");
    }
}

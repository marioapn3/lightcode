use crate::files::FileIndex;
use std::path::Path;

/// A structurally-represented file mention parsed from composer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMention {
    /// Byte offset of the `@` in the text.
    pub start: usize,
    /// Byte offset just after the mention text.
    pub end: usize,
    /// The mention text after `@` (query while typing, resolved path after select).
    pub path: String,
}

fn is_path_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            '(' | ')' | ',' | '"' | '\'' | '`' | '!' | '?' | ';' | ':' | '|' | '<' | '>'
        )
}

/// True when the `@` at byte `idx` starts a file mention: preceded by the start
/// of the text or a non-alphanumeric char (so `user@host` is not a mention).
pub fn is_mention_at(text: &str, idx: usize) -> bool {
    if text.as_bytes().get(idx) != Some(&b'@') {
        return false;
    }
    if idx > 0 {
        let prev = text[..idx].chars().next_back().unwrap();
        if prev.is_alphanumeric() {
            return false;
        }
    }
    true
}

/// The mention currently being typed at `cursor` (byte offset), if any.
pub fn mention_at_cursor(text: &str, cursor: usize) -> Option<FileMention> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let at = before.rfind('@')?;
    if !is_mention_at(text, at) {
        return None;
    }
    let query = &before[at + 1..];
    if query.chars().all(is_path_char) {
        Some(FileMention {
            start: at,
            end: cursor,
            path: query.to_string(),
        })
    } else {
        None
    }
}

/// Parse every mention in a full prompt.
pub fn parse_mentions(text: &str) -> Vec<FileMention> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = text.len();
    while i < bytes {
        let c = text[i..].chars().next().unwrap();
        if c == '@' && is_mention_at(text, i) {
            let mut end = i + 1;
            while end < bytes {
                let ch = text[end..].chars().next().unwrap();
                if is_path_char(ch) {
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
            out.push(FileMention {
                start: i,
                end,
                path: text[i + 1..end].to_string(),
            });
            i = end;
        } else {
            i += c.len_utf8();
        }
    }
    out
}

/// Resolve mentions against the repository and build an agent-context block.
/// Directories are never dumped wholesale: only a metadata + shallow listing.
pub fn resolve_context(prompt: &str, root: &Path, index: &FileIndex) -> Option<String> {
    let mentions = parse_mentions(prompt);
    if mentions.is_empty() {
        return None;
    }
    let mut out = String::from("Referenced files (@mentions):\n");
    let mut budget = 48 * 1024;
    for m in mentions {
        let rel = Path::new(&m.path);
        let full = root.join(rel);
        if m.path.is_empty() {
            continue;
        }
        if full.is_file() {
            match std::fs::read(&full) {
                Ok(bytes) if bytes.len() <= 1024 * 1024 => {
                    let text = String::from_utf8_lossy(&bytes);
                    let take = text.len().min(budget);
                    out.push_str(&format!("--- {}\n{}", m.path, &text[..take]));
                    if take < text.len() {
                        out.push_str("\n[file truncated]\n");
                    }
                    budget = budget.saturating_sub(take);
                }
                Ok(_) => out.push_str(&format!(
                    "--- {} [too large to inline; use tools]\n",
                    m.path
                )),
                Err(e) => out.push_str(&format!("--- {} [unreadable: {e}]\n", m.path)),
            }
        } else if full.is_dir() {
            let entries: Vec<String> = std::fs::read_dir(&full)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            let total = entries.len();
            let shown = entries
                .iter()
                .take(60)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let path = &m.path;
            out.push_str(&format!(
                "Directory {path}: {total} top-level entries. Use tools (list_directory/glob/read_file) for details.\n  {shown}{}\n",
                if total > 60 { ", ..." } else { "" }
            ));
        } else {
            let _ = index;
            let path = &m.path;
            out.push_str(&format!("Mentioned path {path} does not exist.\n"));
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mention_after_whitespace_not_in_email() {
        assert!(is_mention_at("fix @src/a.ts", 4));
        assert!(is_mention_at("@src/a.ts", 0));
        assert!(!is_mention_at("user@host", 4));
        assert!(!is_mention_at("email:user@x.com", 10));
    }

    #[test]
    fn detects_mention_at_cursor() {
        let m = mention_at_cursor("fix the bug in @auth.ser", 24).unwrap();
        assert_eq!(m.start, 15);
        assert_eq!(m.path, "auth.ser");
        // Cursor past the mention (after a space) is not a mention.
        assert!(mention_at_cursor("fix @a.ts done", 13).is_none());
        // No '@' before the cursor.
        assert!(mention_at_cursor("just typing", 5).is_none());
    }

    #[test]
    fn parses_multiple_mentions() {
        let text = "compare @src/a.ts and @src/b.ts fix";
        let ms = parse_mentions(text);
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].path, "src/a.ts");
        assert_eq!(ms[1].path, "src/b.ts");
        assert_eq!(&text[ms[1].start..ms[1].end], "@src/b.ts");
    }

    #[test]
    fn parses_mention_at_start_and_middle() {
        assert_eq!(parse_mentions("@a.ts fix")[0].path, "a.ts");
        assert_eq!(parse_mentions("fix @a.ts")[0].path, "a.ts");
        assert!(parse_mentions("no mentions here").is_empty());
        assert!(parse_mentions("an email user@example.com here").is_empty());
    }

    #[test]
    fn stops_at_whitespace_and_punctuation() {
        let ms = parse_mentions("@a.ts, @b.ts");
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].path, "a.ts");
    }

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lightcode_mentions_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src").join("sub")).unwrap();
        std::fs::write(d.join("src").join("a.rs"), "fn x() {}\n").unwrap();
        std::fs::write(d.join("src").join("sub").join("b.rs"), "fn y() {}\n").unwrap();
        d
    }

    #[test]
    fn resolve_context_loads_files_and_lists_dirs() {
        let d = temp_repo("ctx");
        let index = crate::files::FileIndex::build(&d);
        let ctx = resolve_context("fix @src/a.rs and @src", &d, &index).unwrap();
        assert!(ctx.contains("fn x() {}"), "file content inlined");
        assert!(ctx.contains("Directory src"), "dir mention listed");
        assert!(ctx.contains("a.rs"), "dir listing includes entries");
        // Directory content is metadata, not the file bodies.
        assert!(
            !ctx.contains("fn y() {}"),
            "dir listing must not inline files"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn resolve_context_handles_nonexistent() {
        let d = temp_repo("missing");
        let index = crate::files::FileIndex::build(&d);
        let ctx = resolve_context("check @src/nope.ts", &d, &index).unwrap();
        assert!(ctx.contains("does not exist"));
        assert!(resolve_context("no mentions", &d, &index).is_none());
        std::fs::remove_dir_all(&d).ok();
    }
}

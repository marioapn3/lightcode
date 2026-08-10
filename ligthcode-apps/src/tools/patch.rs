use super::{Tool, ToolDef, MAX_FILE_BYTES};
use serde_json::{json, Value};
use std::path::Path;

pub struct ApplyPatch;

#[async_trait::async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "apply_patch".into(),
            description: "Apply a multi-file patch to the working tree. Patch format:\n\
*** Begin Patch\n\
*** Update File: <path>\n\
@@ -<old_start>[,<old_count>] +<new_start>[,<new_count>] @@\n\
 context lines\n\
-removed lines\n\
+added lines\n\
*** Add File: <path>\n\
+file content (each line prefixed with +)\n\
*** Delete File: <path>\n\
*** Move File: <from> → <to>\n\
*** End Patch\n\
Hunks are applied against the original file; an error means the patch does not fit."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "patch": {"type": "string", "description": "The patch text in the format above"}
                },
                "required": ["patch"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let patch = args
            .get("patch")
            .and_then(|v| v.as_str())
            .ok_or("apply_patch: missing 'patch' argument")?;
        if !patch.contains("*** Begin Patch") || !patch.contains("*** End Patch") {
            return Err(
                "apply_patch: patch must start with '*** Begin Patch' and end with '*** End Patch'"
                    .to_string(),
            );
        }

        let mut changed = Vec::new();
        let mut errors = Vec::new();
        let sections = split_sections(patch);
        for (action, path, body) in sections {
            let result = match action {
                "Update" => update_file(&path, body).await,
                "Add" => add_file(&path, body).await,
                "Delete" => delete_file(&path).await,
                // For Move, the whole `from → to` target is in the path; body is empty.
                "Move" => move_file(&path).await,
                other => Err(format!("unknown section type '{other}'")),
            };
            match result {
                Ok(msg) => changed.push(msg),
                Err(e) => errors.push(e),
            }
        }

        let mut out = String::new();
        if !changed.is_empty() {
            out.push_str(&format!("Applied {} change(s):\n", changed.len()));
            for c in &changed {
                out.push_str(&format!("  - {c}\n"));
            }
        }
        if !errors.is_empty() {
            out.push_str(&format!("Failed on {} change(s):\n", errors.len()));
            for e in &errors {
                out.push_str(&format!("  - {e}\n"));
            }
            return Err(out);
        }
        if changed.is_empty() {
            return Err("apply_patch: patch contains no file changes".to_string());
        }
        Ok(out)
    }
}

/// Split the patch into `(action, target, body)` sections.
fn split_sections(patch: &str) -> Vec<(&str, String, &str)> {
    let mut sections = Vec::new();
    let mut cur_action: Option<(&str, String)> = None;
    let mut cur_body_start = 0usize;

    let lines = patch.split_inclusive('\n');
    let mut offset = 0usize;
    for l in lines {
        let trimmed = l.trim();
        if let Some(rest) = trimmed.strip_prefix("*** ") {
            // close previous section
            if let Some((action, path)) = cur_action.take() {
                sections.push((action, path, &patch[cur_body_start..offset]));
            }
            if rest == "Begin Patch" || rest == "End Patch" {
                offset += l.len();
                continue;
            }
            // Format: "<Action> File: <target>" (or "Move File: a → b")
            let (action, target) = rest
                .split_once(" File: ")
                .map(|(a, t)| (a, t.trim()))
                .unwrap_or_else(|| (rest, ""));
            if target.is_empty() {
                offset += l.len();
                continue;
            }
            cur_action = Some((action, target.to_string()));
            cur_body_start = offset + l.len();
        }
        offset += l.len();
    }
    if let Some((action, path)) = cur_action.take() {
        sections.push((action, path, &patch[cur_body_start..]));
    }
    sections
}

/// Apply unified-diff hunks to an existing file.
async fn update_file(path: &str, body: &str) -> Result<String, String> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("update {path}: {e}"))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("update {path}: file too large"));
    }
    let original = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("update {path}: {e}"))?;

    // Split original into lines. Trailing newline yields a trailing "" element that
    // lines up with unified-diff line numbering.
    let file_lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    let mut result: Vec<String> = Vec::new();
    let mut pos = 0usize; // position in the ORIGINAL file_lines
    let mut applied_hunks = 0usize;

    // body.split("@@") yields [preamble, range1, body1, range2, body2, ...].
    let parts: Vec<&str> = body.split("@@").collect();
    let mut i = 1;
    while i + 1 < parts.len() {
        let range = parts[i].trim();
        let hunk_body = parts[i + 1];
        i += 2;
        let Some((old_start, _new_start, _old_count)) = parse_range(range) else {
            continue;
        };
        // Copy untouched lines that precede this hunk's first line (old_start is 1-based).
        while pos + 1 < old_start {
            if pos >= file_lines.len() {
                return Err(format!(
                    "update {path}: hunk starts at line {old_start} but file has {} lines",
                    file_lines.len().saturating_sub(1)
                ));
            }
            result.push(file_lines[pos].clone());
            pos += 1;
        }

        // Apply the hunk body lines; they consume the original file from line old_start.
        for line in hunk_body.lines() {
            if line.is_empty() {
                continue;
            }
            match line.as_bytes()[0] {
                b'-' => {
                    let rest = &line[1..];
                    expect_line(path, pos, &file_lines, rest)?;
                    pos += 1;
                }
                b'+' => result.push(line[1..].to_string()),
                _ => {
                    let rest = line.strip_prefix(' ').unwrap_or(line);
                    expect_line(path, pos, &file_lines, rest)?;
                    result.push(rest.to_string());
                    pos += 1;
                }
            }
        }
        applied_hunks += 1;
    }

    if applied_hunks == 0 {
        return Err(format!("update {path}: no valid hunks found"));
    }
    while pos < file_lines.len() {
        result.push(file_lines[pos].clone());
        pos += 1;
    }

    let joined = result.join("\n");
    tokio::fs::write(path, joined.as_bytes())
        .await
        .map_err(|e| format!("update {path}: {e}"))?;
    Ok(format!("updated {path} ({applied_hunks} hunk(s))"))
}

fn expect_line(
    path: &str,
    pos: usize,
    file_lines: &[String],
    expected: &str,
) -> Result<(), String> {
    let actual = file_lines
        .get(pos)
        .ok_or_else(|| format!("update {path}: patch does not fit (line {})", pos + 1))?;
    if actual != expected {
        return Err(format!(
            "update {path}: patch does not fit — expected `{expected}` at line {}, found `{actual}`",
            pos + 1
        ));
    }
    Ok(())
}

/// Parse `-old_start[,old_count] +new_start[,new_count]` from a hunk header.
fn parse_range(spec: &str) -> Option<(usize, usize, usize)> {
    let s = spec.trim();
    let rest = s.strip_prefix('-')?;
    let mut split = rest.split_whitespace();
    let old = split.next()?;
    let (old_start, old_count) = parse_start(old)?;
    let rest = split.next()?;
    let rest = rest.strip_prefix('+')?;
    let (new_start, _new_count) = parse_start(rest)?;
    Some((old_start, new_start, old_count))
}

fn parse_start(s: &str) -> Option<(usize, usize)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

async fn add_file(path: &str, body: &str) -> Result<String, String> {
    let mut content = String::new();
    for line in body.lines() {
        content.push_str(line.strip_prefix('+').unwrap_or(line));
        content.push('\n');
    }
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("add {path}: create dirs: {e}"))?;
        }
    }
    let bytes = content.len();
    tokio::fs::write(path, content)
        .await
        .map_err(|e| format!("add {path}: {e}"))?;
    Ok(format!("added {path} ({bytes} bytes)"))
}

async fn delete_file(path: &str) -> Result<String, String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(format!("deleted {path}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("delete {path}: file does not exist"))
        }
        Err(e) => Err(format!("delete {path}: {e}")),
    }
}

async fn move_file(target: &str) -> Result<String, String> {
    let mut parts = target.split('→');
    let from = parts.next().map(str::trim).filter(|s| !s.is_empty());
    let to = parts.next().map(str::trim).filter(|s| !s.is_empty());
    let (Some(from), Some(to)) = (from, to) else {
        return Err(format!(
            "move: expected '*** Move File: from → to', got '{target}'"
        ));
    };
    let content = tokio::fs::read(from)
        .await
        .map_err(|e| format!("move {from}: {e}"))?;
    if let Some(parent) = Path::new(to).parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("move: create dirs: {e}"))?;
        }
    }
    tokio::fs::write(to, content)
        .await
        .map_err(|e| format!("move {from} → {to}: {e}"))?;
    tokio::fs::remove_file(from)
        .await
        .map_err(|e| format!("move {from}: {e}"))?;
    Ok(format!("moved {from} → {to}"))
}

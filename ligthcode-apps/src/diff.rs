use serde_json::Value;
use std::path::{Path, PathBuf};

const CONTEXT: usize = 3;

/// Which files a mutation tool touches, resolved against `root`.
pub fn affected_paths(tool: &str, args: &Value, root: &Path) -> Vec<PathBuf> {
    let pick = |k: &str| args.get(k).and_then(|v| v.as_str());
    let mut out = Vec::new();
    match tool {
        "write_file" | "edit_file" => {
            if let Some(p) = pick("path") {
                out.push(root.join(p));
            }
        }
        "apply_patch" => {
            if let Some(patch) = pick("patch") {
                for line in patch.lines() {
                    let t = line.trim();
                    if let Some(rest) = t.strip_prefix("*** ") {
                        if let Some(path) = rest.split_once(" File: ").map(|(_, p)| p.trim()) {
                            if !path.is_empty() && !path.contains("Begin") && !path.contains("End")
                            {
                                let p = path.split('→').next().unwrap_or(path).trim();
                                out.push(root.join(p));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}

/// Read a file's content, or None if missing/binary/unreadable.
pub fn read_opt(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Generate unified diffs for files snapshotted before a mutation, skipping
/// files whose content did not change.
pub fn diffs_after(before: &[(PathBuf, Option<String>)]) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for (path, old) in before {
        let new = read_opt(path);
        if old == &new {
            continue;
        }
        let body = match (old, new) {
            (Some(o), Some(n)) => unified_diff(o.as_str(), n.as_str()),
            (None, Some(n)) => all_added(n.as_str()),
            (Some(o), None) => all_deleted(o.as_str()),
            (None, None) => continue,
        };
        out.push((path.clone(), body));
    }
    out
}

/// A unified diff between two texts (line-based, bounded LCS).
pub fn unified_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let mut p = 0;
    while p < old_lines.len() && p < new_lines.len() && old_lines[p] == new_lines[p] {
        p += 1;
    }
    let mut so = old_lines.len();
    let mut sn = new_lines.len();
    while so > p && sn > p && old_lines[so - 1] == new_lines[sn - 1] {
        so -= 1;
        sn -= 1;
    }
    // Expand the middle region so hunks carry context lines from both sides.
    let lo = p.saturating_sub(CONTEXT);
    let hi_old = (so + CONTEXT).min(old_lines.len());
    let hi_new = (sn + CONTEXT).min(new_lines.len());
    if hi_old <= lo && hi_new <= lo {
        return String::new();
    }
    let ops = lcs_ops(&old_lines[lo..hi_old], &new_lines[lo..hi_new]);
    build_hunks(ops, lo, lo)
}

fn all_added(new: &str) -> String {
    let lines: Vec<&str> = new.split('\n').collect();
    let mut out = format!("@@ -0,0 +1,{} @@\n", lines.len());
    for l in lines {
        out.push_str(&format!("+{l}\n"));
    }
    out
}

fn all_deleted(old: &str) -> String {
    let lines: Vec<&str> = old.split('\n').collect();
    let mut out = format!("@@ -1,{} +0,0 @@\n", lines.len());
    for l in lines {
        out.push_str(&format!("-{l}\n"));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Op<'a> {
    Eq(&'a str),
    Del(&'a str),
    Ins(&'a str),
}

/// LCS-based diff of the middle region. Falls back to delete-all/insert-all
/// when the region is too large to keep the DP table bounded.
fn lcs_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let n = a.len();
    let m = b.len();
    if n as u64 * m as u64 > 4_000_000 {
        let mut ops = Vec::new();
        for x in a {
            ops.push(Op::Del(x));
        }
        for x in b {
            ops.push(Op::Ins(x));
        }
        return ops;
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            ops.push(Op::Eq(a[i - 1]));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            ops.push(Op::Del(a[i - 1]));
            i -= 1;
        } else {
            ops.push(Op::Ins(b[j - 1]));
            j -= 1;
        }
    }
    while i > 0 {
        ops.push(Op::Del(a[i - 1]));
        i -= 1;
    }
    while j > 0 {
        ops.push(Op::Ins(b[j - 1]));
        j -= 1;
    }
    ops.reverse();
    ops
}

/// Emit hunks with context, merging nearby changes. `old_start`/`new_start`
/// are the 0-based line indices where the ops begin.
fn build_hunks(ops: Vec<Op>, old_start: usize, new_start: usize) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    while i < ops.len() {
        while i < ops.len() && matches!(ops[i], Op::Eq(_)) {
            i += 1;
        }
        if i >= ops.len() {
            break;
        }
        let mut h_start = i;
        let mut back = 0;
        while h_start > 0 && back < CONTEXT && matches!(ops[h_start - 1], Op::Eq(_)) {
            h_start -= 1;
            back += 1;
        }
        let mut h_end = i;
        while h_end < ops.len() {
            if matches!(ops[h_end], Op::Del(_) | Op::Ins(_)) {
                h_end += 1;
            } else {
                let mut ctx = 0;
                let mut k = h_end;
                while k < ops.len() && ctx < CONTEXT && matches!(ops[k], Op::Eq(_)) {
                    ctx += 1;
                    k += 1;
                }
                h_end = k;
                break;
            }
        }
        // Merge the next change if it falls within the context window.
        loop {
            let mut k = h_end;
            while k < ops.len() && matches!(ops[k], Op::Eq(_)) {
                k += 1;
            }
            if k >= ops.len() || k - h_end >= CONTEXT * 2 {
                break;
            }
            let mut j = k;
            while j < ops.len() {
                if matches!(ops[j], Op::Del(_) | Op::Ins(_)) {
                    j += 1;
                } else {
                    let mut c2 = 0;
                    let mut kk = j;
                    while kk < ops.len() && c2 < CONTEXT && matches!(ops[kk], Op::Eq(_)) {
                        c2 += 1;
                        kk += 1;
                    }
                    j = kk;
                    break;
                }
            }
            h_end = j;
        }
        // Line numbers before the hunk.
        let mut o = old_start;
        let mut n = new_start;
        for op in &ops[..h_start] {
            match op {
                Op::Eq(_) | Op::Del(_) => o += 1,
                Op::Ins(_) => {}
            }
        }
        for op in &ops[..h_start] {
            match op {
                Op::Eq(_) | Op::Ins(_) => n += 1,
                Op::Del(_) => {}
            }
        }
        let mut old_count = 0;
        let mut new_count = 0;
        for op in &ops[h_start..h_end] {
            match op {
                Op::Eq(_) => {
                    old_count += 1;
                    new_count += 1;
                }
                Op::Del(_) => old_count += 1,
                Op::Ins(_) => new_count += 1,
            }
        }
        let oh = if old_count == 0 { o } else { o + 1 };
        let nh = if new_count == 0 { n } else { n + 1 };
        out.push_str(&format!("@@ -{oh},{old_count} +{nh},{new_count} @@\n"));
        for op in &ops[h_start..h_end] {
            match op {
                Op::Eq(l) => out.push_str(&format!(" {l}\n")),
                Op::Del(l) => out.push_str(&format!("-{l}\n")),
                Op::Ins(l) => out.push_str(&format!("+{l}\n")),
            }
        }
        out.push('\n');
        i = h_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_diff_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn modified_file_produces_minus_plus() {
        let old = "line1\nold line\nline3\n";
        let new = "line1\nnew line\nline3\n";
        let d = unified_diff(old, new);
        assert!(d.contains("-old line"));
        assert!(d.contains("+new line"));
        assert!(d.contains(" line1"));
        assert!(d.contains("@@"));
    }

    #[test]
    fn unchanged_text_has_no_hunks() {
        assert!(unified_diff("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn new_file_is_all_additions() {
        let d = all_added("a\nb\n");
        assert!(d.contains("+a"));
        assert!(d.contains("+b"));
        assert!(d.contains("@@ -0,0 +1,3 @@"));
    }

    #[test]
    fn deleted_file_is_all_removals() {
        let d = all_deleted("a\nb\n");
        assert!(d.contains("-a"));
        assert!(d.contains("-b"));
    }

    #[test]
    fn snapshots_diff_only_changed_files() {
        let d = temp("snap");
        let a = d.join("a.txt");
        let b = d.join("b.txt");
        fs::write(&a, "one\ntwo\n").unwrap();
        fs::write(&b, "same\n").unwrap();
        let before = vec![(a.clone(), read_opt(&a)), (b.clone(), read_opt(&b))];
        // modify a only
        fs::write(&a, "one\nCHANGED\n").unwrap();
        let diffs = diffs_after(&before);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].1.contains("-two"));
        assert!(diffs[0].1.contains("+CHANGED"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn affected_paths_for_apply_patch() {
        let d = temp("ap");
        let root = d.clone();
        let patch = "*** Begin Patch\n*** Update File: a.rs\n@@ ... @@\n*** Add File: new.rs\n+x\n*** Delete File: old.rs\n*** End Patch";
        let args = serde_json::json!({"patch": patch});
        let paths = affected_paths("apply_patch", &args, &root);
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&d.join("a.rs")));
        assert!(paths.contains(&d.join("new.rs")));
        assert!(paths.contains(&d.join("old.rs")));
        fs::remove_dir_all(&d).ok();
    }
}

use crate::providers::Message;
use crate::session::Session;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub title: String,
    /// Actions permanently approved ("always") during the session, persisted
    /// so restarts do not re-prompt.
    #[serde(default)]
    pub approved: Vec<String>,
    /// Agent mode ("plan" | "build" | "auto"), persisted across restarts.
    #[serde(default)]
    pub mode: Option<String>,
    /// Workspace identity this session belongs to (git root / normalized dir).
    #[serde(default)]
    pub workspace: Option<String>,
    /// Directory the session was started from.
    #[serde(default)]
    pub cwd: Option<String>,
}

static CURRENT_WORKSPACE: std::sync::Mutex<Option<std::path::PathBuf>> =
    std::sync::Mutex::new(None);

/// Set the current workspace; all unscoped operations target it.
pub fn set_workspace(ws: &std::path::Path) {
    if let Ok(mut slot) = CURRENT_WORKSPACE.lock() {
        *slot = Some(ws.to_path_buf());
    }
}

fn current_workspace() -> std::path::PathBuf {
    CURRENT_WORKSPACE
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()))
}

/// Storage directory for a workspace: `sessions/workspaces/<ws_id>`.
pub fn workspace_dir(ws: &std::path::Path) -> PathBuf {
    sessions_dir()
        .join("workspaces")
        .join(crate::workspace::workspace_id(ws))
}

/// Where sessions live: `$LIGHTCODE_DATA_DIR` or the platform data dir.
/// macOS: `~/Library/Application Support/lightcode/sessions`.
pub fn sessions_dir() -> PathBuf {
    std::env::var_os("LIGHTCODE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(platform_data_dir)
}

fn platform_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("lightcode")
            .join("sessions")
    } else {
        home.join(".local")
            .join("share")
            .join("lightcode")
            .join("sessions")
    }
}

pub fn create() -> Result<Session> {
    let ws = current_workspace();
    let s = create_in(&workspace_dir(&ws))?;
    let meta = SessionMeta {
        id: s.id.clone(),
        created_at: now_string(),
        title: String::new(),
        approved: Vec::new(),
        mode: None,
        workspace: Some(ws.to_string_lossy().into_owned()),
        cwd: std::env::current_dir()
            .ok()
            .map(|c| c.to_string_lossy().into_owned()),
    };
    write_meta(&s.dir, &meta)?;
    Ok(s)
}

pub fn open(id: &str) -> Result<Session> {
    let ws = current_workspace();
    let dir = workspace_dir(&ws);
    match open_in(&dir, id) {
        Ok(s) => Ok(s),
        Err(_) => {
            // Search other workspaces + unscoped (flat root) for a precise error.
            if let Some((found_ws, _)) = find_session_anywhere(id) {
                bail!(
                    "session '{id}' belongs to workspace '{found_ws}'; current workspace is '{}'. \
                     Use `lightcode session adopt {id}` to move it here.",
                    ws.display()
                );
            }
            bail!("session '{id}' does not exist");
        }
    }
}

pub fn list() -> Result<Vec<SessionMeta>> {
    list_in(&workspace_dir(&current_workspace()))
}

pub fn delete(id: &str) -> Result<()> {
    delete_in(&workspace_dir(&current_workspace()), id)
}

pub fn rename(id: &str, title: &str) -> Result<()> {
    rename_in(&workspace_dir(&current_workspace()), id, title)
}

/// Create a copy of a session under a new id. The copy shares the original's
/// message history; the title is suffixed with "(fork)".
pub fn fork(id: &str) -> Result<Session> {
    let ws = current_workspace();
    let s = fork_in(&workspace_dir(&ws), id)?;
    let meta_path = s.dir.join(format!("{}.meta.json", s.id));
    if let Ok(text) = std::fs::read_to_string(&meta_path) {
        if let Ok(mut meta) = serde_json::from_str::<SessionMeta>(&text) {
            meta.workspace = Some(ws.to_string_lossy().into_owned());
            meta.cwd = std::env::current_dir()
                .ok()
                .map(|c| c.to_string_lossy().into_owned());
            write_meta(&s.dir, &meta)?;
        }
    }
    Ok(s)
}

/// List sessions across every workspace, plus un-scoped sessions at the root.
pub fn list_all() -> Result<Vec<(String, SessionMeta)>> {
    let mut out = Vec::new();
    let base = sessions_dir().join("workspaces");
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let ws_path = entry.path();
            let label = ws_label(&ws_path);
            for m in list_in(&ws_path)? {
                out.push((label.clone(), m));
            }
        }
    }
    for m in list_in(&sessions_dir())? {
        out.push(("(unscoped)".to_string(), m));
    }
    out.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
    Ok(out)
}

fn ws_label(ws_dir: &Path) -> String {
    let id = ws_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // find a meta to read the workspace path
    if let Ok(rd) = std::fs::read_dir(ws_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(_id) = name.strip_suffix(".meta.json") {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    if let Ok(m) = serde_json::from_str::<SessionMeta>(&text) {
                        if let Some(ws) = m.workspace {
                            return ws;
                        }
                    }
                }
            }
        }
    }
    id
}

/// Move a session (from any workspace or legacy) into the current workspace.
/// Move a session (from any workspace or the unscoped root) into the current
/// workspace.
pub fn adopt(id: &str) -> Result<Session> {
    let ws = current_workspace();
    let ws_dir = workspace_dir(&ws);
    std::fs::create_dir_all(&ws_dir).context("creating workspace sessions dir")?;
    let source = find_session_anywhere(id)
        .map(|(_, dir)| dir)
        .ok_or_else(|| anyhow!("session '{id}' does not exist"))?;
    adopt_from(source, id, &ws, &ws_dir)
}

/// Adopt every un-scoped session (at the sessions root) into the current
/// workspace. Returns how many were moved.
pub fn adopt_all() -> Result<usize> {
    let ws = current_workspace();
    let ws_dir = workspace_dir(&ws);
    std::fs::create_dir_all(&ws_dir).context("creating workspace sessions dir")?;
    let root = sessions_dir();
    let mut moved = 0usize;
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Ok(0);
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".meta.json") {
            if adopt_from(root.clone(), id, &ws, &ws_dir).is_ok() {
                moved += 1;
            }
        }
    }
    Ok(moved)
}

fn adopt_from(source: PathBuf, id: &str, ws: &Path, ws_dir: &Path) -> Result<Session> {
    let from_meta = source.join(format!("{id}.meta.json"));
    let from_jsonl = source.join(format!("{id}.jsonl"));
    let to_meta = ws_dir.join(format!("{id}.meta.json"));
    let to_jsonl = ws_dir.join(format!("{id}.jsonl"));
    std::fs::copy(&from_meta, &to_meta).context("copying session meta")?;
    if from_jsonl.is_file() {
        std::fs::copy(&from_jsonl, &to_jsonl).context("copying session history")?;
    }
    if let Ok(text) = std::fs::read_to_string(&to_meta) {
        if let Ok(mut meta) = serde_json::from_str::<SessionMeta>(&text) {
            meta.workspace = Some(ws.to_string_lossy().into_owned());
            meta.cwd = std::env::current_dir()
                .ok()
                .map(|c| c.to_string_lossy().into_owned());
            write_meta(ws_dir, &meta)?;
        }
    }
    std::fs::remove_file(&from_meta).ok();
    std::fs::remove_file(&from_jsonl).ok();
    Ok(Session {
        id: id.to_string(),
        dir: ws_dir.to_path_buf(),
    })
}

/// Locate a session id anywhere (other workspaces + unscoped root).
/// Returns (workspace_label, dir).
fn find_session_anywhere(id: &str) -> Option<(String, PathBuf)> {
    let base = sessions_dir().join("workspaces");
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            if entry.path().join(format!("{id}.meta.json")).is_file() {
                let label = ws_label(&entry.path());
                return Some((label, entry.path()));
            }
        }
    }
    if sessions_dir().join(format!("{id}.meta.json")).is_file() {
        return Some(("(unscoped)".to_string(), sessions_dir()));
    }
    None
}

pub fn create_in(dir: &Path) -> Result<Session> {
    std::fs::create_dir_all(dir).context("creating sessions directory")?;
    let id = new_id();
    let meta = SessionMeta {
        id: id.clone(),
        created_at: now_string(),
        title: String::new(),
        approved: Vec::new(),
        mode: None,
        workspace: None,
        cwd: None,
    };
    write_meta(dir, &meta)?;
    Ok(Session {
        id,
        dir: dir.to_path_buf(),
    })
}

pub fn open_in(dir: &Path, id: &str) -> Result<Session> {
    if !dir.join(format!("{id}.meta.json")).is_file() {
        bail!("session '{id}' does not exist");
    }
    Ok(Session {
        id: id.to_string(),
        dir: dir.to_path_buf(),
    })
}

pub fn list_in(dir: &Path) -> Result<Vec<SessionMeta>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir).context("reading sessions directory")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".meta.json") {
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                if let Ok(m) = serde_json::from_str::<SessionMeta>(&text) {
                    out.push(m);
                } else {
                    out.push(SessionMeta {
                        id: id.to_string(),
                        created_at: String::new(),
                        title: String::new(),
                        approved: Vec::new(),
                        mode: None,
                        workspace: None,
                        cwd: None,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

pub fn delete_in(dir: &Path, id: &str) -> Result<()> {
    std::fs::remove_file(dir.join(format!("{id}.meta.json"))).ok();
    std::fs::remove_file(dir.join(format!("{id}.jsonl"))).ok();
    Ok(())
}

pub fn rename_in(dir: &Path, id: &str, title: &str) -> Result<()> {
    let meta_path = dir.join(format!("{id}.meta.json"));
    let text = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("session '{id}' does not exist"))?;
    let mut meta =
        serde_json::from_str::<SessionMeta>(&text).context("parsing session metadata")?;
    meta.title = title.trim().chars().take(200).collect();
    write_meta(dir, &meta)?;
    Ok(())
}

pub fn fork_in(dir: &Path, id: &str) -> Result<Session> {
    let original = open_in(dir, id)?;
    let messages = load_messages(&original)?;
    let new_id = new_id();
    let src_meta_path = dir.join(format!("{id}.meta.json"));
    let title = if let Ok(text) = std::fs::read_to_string(&src_meta_path) {
        if let Ok(m) = serde_json::from_str::<SessionMeta>(&text) {
            format!("{} (fork)", m.title.trim())
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let meta = SessionMeta {
        id: new_id.clone(),
        created_at: now_string(),
        title,
        approved: Vec::new(),
        mode: None,
        workspace: None,
        cwd: None,
    };
    write_meta(dir, &meta)?;
    if !messages.is_empty() {
        let path = dir.join(format!("{new_id}.jsonl"));
        let mut f = std::fs::File::create(&path).context("creating fork session file")?;
        for m in &messages {
            let line = serde_json::to_string(m)?;
            writeln!(f, "{line}").context("writing fork message")?;
        }
    }
    Ok(Session {
        id: new_id,
        dir: dir.to_path_buf(),
    })
}

pub(crate) fn append_message(session: &Session, m: &Message) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(session.dir.join(format!("{}.jsonl", session.id)))
        .context("opening session file")?;
    let line = serde_json::to_string(m)?;
    writeln!(f, "{line}").context("writing session message")?;

    if let Message::User { content } = m {
        let meta_path = session.dir.join(format!("{}.meta.json", session.id));
        if let Ok(text) = std::fs::read_to_string(&meta_path) {
            if let Ok(mut meta) = serde_json::from_str::<SessionMeta>(&text) {
                if meta.title.is_empty() && !content.trim().is_empty() {
                    meta.title = content.trim().chars().take(50).collect();
                    write_meta(&session.dir, &meta)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn load_messages(session: &Session) -> Result<Vec<Message>> {
    let path = session.dir.join(format!("{}.jsonl", session.id));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading session file"),
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.trim().is_empty() {
            out.push(serde_json::from_str(line).context("parsing session message")?);
        }
    }
    Ok(out)
}

/// Export payload for a session (used by `session export`).
#[derive(Serialize, Deserialize)]
pub struct SessionExport {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub messages: Vec<Message>,
}

pub fn export_session(id: &str) -> Result<SessionExport> {
    export_in(&workspace_dir(&current_workspace()), id)
}

pub fn export_in(dir: &Path, id: &str) -> Result<SessionExport> {
    let s = open_in(dir, id)?;
    let meta_path = dir.join(format!("{id}.meta.json"));
    let (title, created_at) = if let Ok(text) = std::fs::read_to_string(&meta_path) {
        if let Ok(m) = serde_json::from_str::<SessionMeta>(&text) {
            (m.title, m.created_at)
        } else {
            (String::new(), String::new())
        }
    } else {
        (String::new(), String::new())
    };
    Ok(SessionExport {
        id: id.to_string(),
        title,
        created_at,
        messages: load_messages(&s)?,
    })
}

/// Import an exported session under a fresh id, returning the new session.
pub fn import_session(json: &str) -> Result<Session> {
    let ws = current_workspace();
    let s = import_in(&workspace_dir(&ws), json)?;
    // stamp the imported session with the current workspace
    if let Ok(text) = std::fs::read_to_string(s.dir.join(format!("{}.meta.json", s.id))) {
        if let Ok(mut meta) = serde_json::from_str::<SessionMeta>(&text) {
            meta.workspace = Some(ws.to_string_lossy().into_owned());
            meta.cwd = std::env::current_dir()
                .ok()
                .map(|c| c.to_string_lossy().into_owned());
            write_meta(&s.dir, &meta)?;
        }
    }
    Ok(s)
}

pub fn import_in(dir: &Path, json: &str) -> Result<Session> {
    let export: SessionExport = serde_json::from_str(json).context("parsing session export")?;
    std::fs::create_dir_all(dir).context("creating sessions directory")?;
    let id = new_id();
    let meta = SessionMeta {
        id: id.clone(),
        created_at: now_string(),
        title: export.title,
        approved: Vec::new(),
        mode: None,
        workspace: None,
        cwd: None,
    };
    write_meta(dir, &meta)?;
    if !export.messages.is_empty() {
        let path = dir.join(format!("{id}.jsonl"));
        let mut f = std::fs::File::create(&path).context("creating imported session file")?;
        for m in &export.messages {
            let line = serde_json::to_string(m)?;
            writeln!(f, "{line}").context("writing imported message")?;
        }
    }
    Ok(Session {
        id,
        dir: dir.to_path_buf(),
    })
}

fn write_meta(dir: &Path, meta: &SessionMeta) -> Result<()> {
    let path = dir.join(format!("{}.meta.json", meta.id));
    std::fs::write(&path, serde_json::to_vec(meta)?).context("writing session metadata")
}

/// Read the persisted "always" approved action list for a session.
pub(crate) fn read_approved(session: &Session) -> Vec<String> {
    let path = session.dir.join(format!("{}.meta.json", session.id));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<SessionMeta>(&text)
        .map(|m| m.approved)
        .unwrap_or_default()
}

/// Append an action to the persisted "always" approved list.
pub(crate) fn add_approved(session: &Session, action: &str) -> Result<()> {
    let meta_path = session.dir.join(format!("{}.meta.json", session.id));
    let text = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("session '{}' does not exist", session.id))?;
    let mut meta =
        serde_json::from_str::<SessionMeta>(&text).context("parsing session metadata")?;
    if !meta.approved.iter().any(|a| a == action) {
        meta.approved.push(action.to_string());
        write_meta(&session.dir, &meta)?;
    }
    Ok(())
}

/// Read the persisted agent mode for a session (None → default Build).
pub(crate) fn read_mode(session: &Session) -> Option<String> {
    let meta_path = session.dir.join(format!("{}.meta.json", session.id));
    let text = std::fs::read_to_string(&meta_path).ok()?;
    serde_json::from_str::<SessionMeta>(&text)
        .ok()
        .and_then(|m| m.mode)
}

/// Persist the agent mode for a session.
pub(crate) fn save_mode(session: &Session, mode: &str) -> Result<()> {
    let meta_path = session.dir.join(format!("{}.meta.json", session.id));
    let text = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("session '{}' does not exist", session.id))?;
    let mut meta =
        serde_json::from_str::<SessionMeta>(&text).context("parsing session metadata")?;
    meta.mode = Some(mode.to_string());
    write_meta(&session.dir, &meta)?;
    Ok(())
}

/// Read the workspace identity stored on a session.
#[cfg(test)]
pub(crate) fn read_workspace(session: &Session) -> Option<String> {
    let path = session.dir.join(format!("{}.meta.json", session.id));
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<SessionMeta>(&text)
        .ok()
        .and_then(|m| m.workspace)
}

fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}-{}-{seq}", std::process::id())
}

fn now_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Serializes tests that set the `LIGHTCODE_DATA_DIR` env var, which several
    /// modules read via `sessions_dir()`.
    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("lightcode_session_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn session_roundtrip_and_list() {
        let d = temp_dir("roundtrip");
        let s = create_in(&d).unwrap();
        s.append(&Message::User {
            content: "hello world".into(),
        })
        .unwrap();

        let loaded = s.load_history().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(matches!(&loaded[0], Message::User { content } if content == "hello world"));

        let metas = list_in(&d).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].title, "hello world");

        delete_in(&d, &s.id).unwrap();
        assert!(list_in(&d).unwrap().is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn open_missing_session_fails() {
        let d = temp_dir("missing");
        assert!(open_in(&d, "nope").is_err());
        std::fs::remove_dir_all(&d).ok();
    }
    #[test]
    fn mode_persistence_roundtrip() {
        let d = temp_dir("mode");
        let s = create_in(&d).unwrap();
        assert_eq!(s.read_mode(), None);
        s.save_mode("auto").unwrap();
        assert_eq!(s.read_mode().as_deref(), Some("auto"));
        // Reopening from disk keeps it.
        let reopened = open_in(&d, &s.id).unwrap();
        assert_eq!(reopened.read_mode().as_deref(), Some("auto"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn workspace_scoping_isolates_projects() {
        let _guard = ENV_LOCK.lock().unwrap();
        let base = temp_dir("wsscope");
        std::env::set_var("LIGHTCODE_DATA_DIR", &base);
        let a = base.join("proj-a");
        let b = base.join("proj-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        set_workspace(&a);
        let s1 = create().unwrap();
        assert!(s1
            .read_workspace()
            .is_some_and(|w| w == a.to_string_lossy()));

        set_workspace(&b);
        assert!(
            list().unwrap().is_empty(),
            "project B must not see A sessions"
        );
        let s2 = create().unwrap();
        let ids: Vec<String> = list().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![s2.id.clone()]);

        set_workspace(&a);
        let ids: Vec<String> = list().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![s1.id.clone()]);

        // Resume from the wrong workspace must fail loudly.
        set_workspace(&b);
        let err = open(&s1.id).unwrap_err();
        assert!(err.to_string().contains("belongs to workspace"), "{err}");

        // Adopt moves it into the current workspace.
        adopt(&s1.id).unwrap();
        let ids: Vec<String> = list().unwrap().into_iter().map(|m| m.id).collect();
        assert!(ids.contains(&s1.id));

        std::env::remove_var("LIGHTCODE_DATA_DIR");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn legacy_sessions_migrate_safely() {
        let _guard = ENV_LOCK.lock().unwrap();
        let base = temp_dir("legacy");
        std::env::set_var("LIGHTCODE_DATA_DIR", &base);
        // Simulate an old flat session at the sessions root (no workspace).
        let old_id = "legacy-1";
        std::fs::write(
            base.join(format!("{old_id}.meta.json")),
            format!(r#"{{"id":"{old_id}","created_at":"1","title":"old"}}"#),
        )
        .unwrap();
        std::fs::write(base.join(format!("{old_id}.jsonl")), "").unwrap();

        // Not visible in a normal workspace listing (stays put, never deleted).
        set_workspace(&base.join("proj"));
        std::fs::create_dir_all(base.join("proj")).unwrap();
        assert!(list().unwrap().is_empty());
        // Still present on disk, listed under (unscoped).
        assert!(list_all()
            .unwrap()
            .iter()
            .any(|(ws, m)| ws == "(unscoped)" && m.id == old_id));
        // Adoptable individually and in bulk.
        adopt(old_id).unwrap();
        let ids: Vec<String> = list().unwrap().into_iter().map(|m| m.id).collect();
        assert!(ids.iter().any(|i| i == old_id));

        std::env::remove_var("LIGHTCODE_DATA_DIR");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn adopt_all_moves_unscoped_sessions() {
        let _guard = ENV_LOCK.lock().unwrap();
        let base = temp_dir("adoptall");
        std::env::set_var("LIGHTCODE_DATA_DIR", &base);
        for i in 0..3 {
            let id = format!("u-{i}");
            std::fs::write(
                base.join(format!("{id}.meta.json")),
                format!(r#"{{"id":"{id}","created_at":"1","title":"{id}"}}"#),
            )
            .unwrap();
        }
        set_workspace(&base.join("proj"));
        std::fs::create_dir_all(base.join("proj")).unwrap();
        let n = adopt_all().unwrap();
        assert_eq!(n, 3);
        assert_eq!(list().unwrap().len(), 3);
        assert!(!base.join("u-0.meta.json").is_file()); // moved out
        std::env::remove_var("LIGHTCODE_DATA_DIR");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn export_and_import_session() {
        let d = temp_dir("export");
        let s = create_in(&d).unwrap();
        s.append(&Message::User {
            content: "hello".into(),
        })
        .unwrap();

        let export = export_in(&d, &s.id).unwrap();
        assert_eq!(export.title, "hello");
        assert_eq!(export.messages.len(), 1);

        let imported = import_in(&d, &serde_json::to_string(&export).unwrap()).unwrap();
        assert_ne!(imported.id, s.id);
        assert_eq!(imported.load_history().unwrap().len(), 1);
        assert_eq!(list_in(&d).unwrap().len(), 2);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rename_and_fork_session() {
        let d = temp_dir("fork");
        let s = create_in(&d).unwrap();
        s.append(&Message::User {
            content: "hello".into(),
        })
        .unwrap();

        rename_in(&d, &s.id, "my custom title").unwrap();
        let metas = list_in(&d).unwrap();
        assert_eq!(metas[0].title, "my custom title");

        let fork = fork_in(&d, &s.id).unwrap();
        assert_ne!(fork.id, s.id);
        let fork_metas = list_in(&d).unwrap();
        assert_eq!(fork_metas.len(), 2);
        assert!(fork_metas[0].title.contains("fork") || fork_metas[1].title.contains("fork"));
        let fork_history = fork.load_history().unwrap();
        assert_eq!(fork_history.len(), 1);
        assert!(matches!(&fork_history[0], Message::User { content } if content == "hello"));

        std::fs::remove_dir_all(&d).ok();
    }
}

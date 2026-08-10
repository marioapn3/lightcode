pub mod storage;

use crate::providers::Message;
use anyhow::{Context, Result};

/// A persistent session: message history appended to `<dir>/<id>.jsonl`.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    dir: std::path::PathBuf,
}

impl Session {
    pub fn append(&self, m: &Message) -> anyhow::Result<()> {
        storage::append_message(self, m)
    }

    pub fn load_history(&self) -> anyhow::Result<Vec<Message>> {
        storage::load_messages(self)
    }

    pub fn approved_actions(&self) -> Vec<String> {
        storage::read_approved(self)
    }

    pub fn approve_always(&self, action: &str) -> anyhow::Result<()> {
        storage::add_approved(self, action)
    }

    pub fn read_mode(&self) -> Option<String> {
        storage::read_mode(self)
    }

    #[cfg(test)]
    pub fn read_workspace(&self) -> Option<String> {
        storage::read_workspace(self)
    }

    pub fn save_mode(&self, mode: &str) -> anyhow::Result<()> {
        storage::save_mode(self, mode)
    }
}

pub fn cmd_list() -> Result<()> {
    let metas = storage::list()?;
    if metas.is_empty() {
        println!("no sessions in this workspace");
        return Ok(());
    }
    for m in metas {
        println!("{}  {}  {}", m.id, m.created_at, m.title);
    }
    Ok(())
}

pub fn cmd_list_all() -> Result<()> {
    let all = storage::list_all()?;
    if all.is_empty() {
        println!("no sessions anywhere");
        return Ok(());
    }
    for (workspace, m) in all {
        println!("[{}] {}  {}  {}", workspace, m.id, m.created_at, m.title);
    }
    Ok(())
}

pub fn cmd_adopt(id: &str) -> Result<()> {
    let s = storage::adopt(id)?;
    println!("adopted {id} into current workspace → {}", s.dir.display());
    Ok(())
}

pub fn cmd_show(id: &str) -> Result<()> {
    let s = storage::open(id)?;
    let msgs = s.load_history()?;
    if msgs.is_empty() {
        println!("session {id} is empty");
        return Ok(());
    }
    for m in msgs {
        match m {
            Message::System { content } => println!("[system] {content}"),
            Message::User { content } => println!("[user] {content}"),
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if let Some(c) = content {
                    println!("[assistant] {c}");
                }
                for tc in tool_calls {
                    println!("  → {} {}", tc.name, tc.arguments);
                }
            }
            Message::Tool { content, .. } => println!("[tool] {content}"),
        }
    }
    Ok(())
}

pub fn cmd_delete(id: &str) -> Result<()> {
    storage::delete(id)?;
    println!("deleted session {id}");
    Ok(())
}

pub fn cmd_rename(id: &str, title: &str) -> Result<()> {
    storage::rename(id, title)?;
    println!("renamed session {id} → {title}");
    Ok(())
}

pub fn cmd_fork(id: &str) -> Result<()> {
    let s = storage::fork(id)?;
    println!("forked {id} → {}", s.id);
    Ok(())
}

pub fn cmd_export(id: &str) -> Result<()> {
    let export = storage::export_session(id)?;
    println!("{}", serde_json::to_string_pretty(&export)?);
    Ok(())
}

pub fn cmd_import(path: &std::path::Path) -> Result<()> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let s = storage::import_session(&json)?;
    println!("imported → {}", s.id);
    Ok(())
}

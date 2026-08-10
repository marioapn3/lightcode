use std::io::Write;
use std::path::PathBuf;

const MAX_HISTORY: usize = 200;

fn history_path() -> PathBuf {
    crate::session::storage::sessions_dir().join("prompt-history.json")
}

/// Load previously submitted prompts (most recent first, capped).
pub fn load() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(history_path()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

/// Append a prompt to the persistent history.
pub fn push(prompt: &str) {
    let mut items = load();
    items.retain(|p| p != prompt);
    items.insert(0, prompt.to_string());
    items.truncate(MAX_HISTORY);
    let _ = std::fs::create_dir_all(history_path().parent().unwrap());
    let Ok(mut f) = std::fs::File::create(history_path()) else {
        return;
    };
    let _ = f.write_all(serde_json::to_string(&items).unwrap_or_default().as_bytes());
}

/// Prompts matching `query` (case-insensitive substring), most recent first.
pub fn suggestions(query: &str, limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    load()
        .into_iter()
        .filter(|p| !p.is_empty() && p.to_lowercase().contains(&q))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_load_roundtrip_and_filter() {
        let _guard = crate::session::storage::tests::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lightcode_history_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LIGHTCODE_DATA_DIR", &dir);

        push("explain the router");
        push("refactor auth");
        push("explain the router"); // dedup, moves to front
        let items = load();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "explain the router");

        let sug = suggestions("auth", 10);
        assert_eq!(sug, vec!["refactor auth".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("LIGHTCODE_DATA_DIR");
    }
}

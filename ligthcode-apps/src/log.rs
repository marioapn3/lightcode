use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Initialize logging. Enabled when `LIGHTCODE_LOG` is set:
///   LIGHTCODE_LOG=1        → default log file next to sessions
///   LIGHTCODE_LOG=/path    → that file
/// When disabled, `write` is a no-op.
pub fn init() {
    let path = match std::env::var("LIGHTCODE_LOG") {
        Ok(v) if !v.is_empty() && v != "0" && v != "false" => {
            if v == "1" || v == "true" {
                default_path()
            } else {
                Some(PathBuf::from(v))
            }
        }
        _ => None,
    };
    let _ = LOG_PATH.set(path);
}

fn default_path() -> Option<PathBuf> {
    Some(crate::session::storage::sessions_dir().join("lightcode.log"))
}

/// Append a line to the log file (no-op when logging is disabled).
pub fn write(msg: &str) {
    let Some(path) = LOG_PATH.get().and_then(|p| p.as_ref()) else {
        return;
    };
    let line = format!("{} {msg}\n", timestamp());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = now % 86400;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    format!("[{:02}:{:02}:{:02}]", h, m, sec)
}

#[macro_export]
macro_rules! log_line {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Cap on bytes kept from a child process's stdout/stderr; anything beyond is drained and dropped.
pub const OUTPUT_LIMIT: usize = 64 * 1024;

pub struct CmdResult {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub timed_out: bool,
}

/// Run `program` with `args` in `workdir`, capturing bounded stdout/stderr.
/// The process is killed if it outlives `timeout_secs`.
pub async fn run(program: &str, args: &[String], workdir: &str, timeout_secs: u64) -> CmdResult {
    let mut child = match Command::new(program)
        .args(args)
        .current_dir(workdir)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return CmdResult {
                stdout: String::new(),
                stderr: format!("failed to spawn {program}: {e}"),
                code: None,
                timed_out: false,
            }
        }
    };

    let drain = {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        tokio::spawn(async move {
            let mut out_buf = Vec::new();
            let mut err_buf = Vec::new();
            if let Some(s) = stdout {
                read_limited(s, &mut out_buf).await;
            }
            if let Some(s) = stderr {
                read_limited(s, &mut err_buf).await;
            }
            (out_buf, err_buf)
        })
    };

    let (code, timed_out) = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(_)) => (None, false),
        Err(_) => (None, true),
    };

    // Draining stops once pipes close; don't wait forever if a grandchild holds them.
    let (out_buf, err_buf) =
        match tokio::time::timeout(std::time::Duration::from_secs(5), drain).await {
            Ok(Ok(b)) => b,
            _ => (Vec::new(), Vec::new()),
        };

    CmdResult {
        stdout: String::from_utf8_lossy(&out_buf).into_owned(),
        stderr: String::from_utf8_lossy(&err_buf).into_owned(),
        code,
        timed_out,
    }
}

async fn read_limited<R: AsyncRead + Unpin>(mut r: R, buf: &mut Vec<u8>) {
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = OUTPUT_LIMIT.saturating_sub(buf.len());
                if room > 0 {
                    buf.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
    }
}

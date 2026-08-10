use super::StreamEvent;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

/// Read a `text/event-stream` response, parse each `data:` record with `parse`,
/// forward the resulting events, and finally send `Done`.
pub async fn stream_sse(
    resp: reqwest::Response,
    tx: &mpsc::Sender<StreamEvent>,
    parse: impl Fn(&Value) -> Vec<StreamEvent>,
) {
    let status = resp.status();
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(c) => buf.extend_from_slice(&c),
            Err(e) => {
                let snippet: String = String::from_utf8_lossy(&buf).chars().take(300).collect();
                crate::log_line!(
                    "sse stream error: {e} (status {status}); received so far: {snippet:?}"
                );
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "{e} — status {status}, received: {snippet:?}"
                    )))
                    .await;
                return;
            }
        }
        let mut start = 0;
        while let Some(rel) = find_sse_boundary(&buf[start..]) {
            let record = &buf[start..start + rel];
            start += rel + 2;
            for line in record.split(|b| *b == b'\n') {
                let Ok(s) = std::str::from_utf8(line) else {
                    continue;
                };
                let s = s.trim();
                let Some(data) = s.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    let _ = tx.send(StreamEvent::Done).await;
                    return;
                }
                let Ok(v) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                for ev in parse(&v) {
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
            }
        }
        buf.drain(..start);
    }
    let _ = tx.send(StreamEvent::Done).await;
}

fn find_sse_boundary(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

use super::{client, MAX_RESPONSE_BYTES};
use futures_util::StreamExt;
use regex::Regex;

/// Fetch a URL and return readable text. HTML is stripped to text; non-HTML is returned raw.
pub async fn fetch_text(url: &str) -> Result<String, String> {
    let resp = client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("web_fetch {url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("web_fetch {url}: HTTP {status}"));
    }

    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("web_fetch {url}: {e}"))?;
        let room = MAX_RESPONSE_BYTES.saturating_sub(buf.len());
        if room == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    let text = String::from_utf8_lossy(&buf).into_owned();
    let text = if looks_like_html(&text) {
        html_to_text(&text)
    } else {
        text
    };
    if text.trim().is_empty() {
        return Err(format!("web_fetch {url}: empty response"));
    }
    Ok(text)
}

fn looks_like_html(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.contains("<html")
        || lower.contains("<body")
        || lower.contains("<head")
}

/// Crude but effective HTML -> text: drop scripts/styles, strip tags, decode entities.
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for pat in [
        r"(?s)<script.*?</script>",
        r"(?s)<style.*?</style>",
        r"(?s)<head.*?</head>",
        r"(?s)<noscript.*?</noscript>",
    ] {
        let re = Regex::new(pat).unwrap();
        s = re.replace_all(&s, " ").into_owned();
    }
    let re_tag = Regex::new(r"<[^>]+>").unwrap();
    s = re_tag.replace_all(&s, " ").into_owned();
    s = decode_entities(&s);
    let re_space = Regex::new(r"[ \t]+").unwrap();
    s = re_space.replace_all(&s, " ").into_owned();
    s.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_to_text() {
        let html = r#"<html><head><style>.x{}</style></head><body>
            <h1>Hello &amp; world</h1>
            <p>Some <b>bold</b> text.</p>
            <script>alert('bad');</script>
        </body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("Hello & world"));
        assert!(text.contains("Some bold text."));
        assert!(!text.contains("alert"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode_entities("a &lt; b &gt; c &quot;d&quot;"),
            "a < b > c \"d\""
        );
    }
}

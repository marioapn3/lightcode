use super::client;
use regex::Regex;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search the web. Uses Tavily when TAVILY_API_KEY is set, otherwise scrapes DuckDuckGo.
pub async fn search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
    if let Ok(key) = std::env::var("TAVILY_API_KEY") {
        if !key.trim().is_empty() {
            return tavily_search(query, max_results, &key).await;
        }
    }
    duckduckgo_search(query, max_results).await
}

async fn tavily_search(
    query: &str,
    max_results: usize,
    api_key: &str,
) -> Result<Vec<SearchResult>, String> {
    let body = json!({
        "query": query,
        "max_results": max_results,
        "search_depth": "basic"
    });
    let resp = client()
        .post("https://api.tavily.com/search")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("web_search: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("web_search: {e}"))?;
    if !status.is_success() {
        return Err(format!("web_search: Tavily HTTP {status}: {text}"));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("web_search: invalid response: {e}"))?;
    let mut out = Vec::new();
    if let Some(results) = v["results"].as_array() {
        for r in results {
            out.push(SearchResult {
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
                snippet: r["content"].as_str().unwrap_or("").to_string(),
            });
            if out.len() >= max_results {
                break;
            }
        }
    }
    Ok(out)
}

async fn duckduckgo_search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(query));
    let resp = client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("web_search: {e}"))?;
    let status = resp.status();
    let html = resp.text().await.map_err(|e| format!("web_search: {e}"))?;
    if !status.is_success() {
        return Err(format!("web_search: DuckDuckGo HTTP {status}"));
    }
    Ok(parse_ddg(&html, max_results))
}

/// Parse DuckDuckGo HTML results. Anchors and snippets appear in matching order.
pub fn parse_ddg(html: &str, max_results: usize) -> Vec<SearchResult> {
    let re_a = Regex::new(r#"class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let re_snippet = Regex::new(r#"class="result__snippet"[^>]*>(.*?)</a>"#).unwrap();
    let anchors: Vec<_> = re_a.captures_iter(html).collect();
    let snippets: Vec<_> = re_snippet.captures_iter(html).collect();

    let mut out = Vec::new();
    for (i, a) in anchors.iter().enumerate() {
        out.push(SearchResult {
            title: strip_tags(&a[2]).trim().to_string(),
            url: decode_ddg_href(&a[1]),
            snippet: snippets
                .get(i)
                .map(|c| strip_tags(&c[1]).trim().to_string())
                .unwrap_or_default(),
        });
        if out.len() >= max_results {
            break;
        }
    }
    out
}

/// DuckDuckGo redirect URLs look like `//duckduckgo.com/l/?uddg=<encoded>&rut=...`.
fn decode_ddg_href(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let rest = &href[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        return percent_decode(&rest[..end]);
    }
    percent_decode(href)
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(hex);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_tags(s: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(s, " ").into_owned()
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ddg_html() {
        let html = r#"<html><body>
            <div class="result">
              <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.stripe.com%2Fapi&rut=x">Stripe API Reference</a>
              <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x&rut=y">Official Stripe API docs.</a>
            </div>
            <div class="result">
              <a rel="nofollow" class="result__a" href="https://example.com/2">Second Result</a>
              <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x&rut=y">Second snippet.</a>
            </div>
        </body></html>"#;
        let results = parse_ddg(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Stripe API Reference");
        assert_eq!(results[0].url, "https://docs.stripe.com/api");
        assert_eq!(results[0].snippet, "Official Stripe API docs.");
        assert_eq!(results[1].url, "https://example.com/2");
    }

    #[test]
    fn decodes_percent_encoding() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a%2Bb"), "a+b");
    }

    #[test]
    fn urlencodes_query() {
        assert_eq!(urlencode("stripe api v2"), "stripe+api+v2");
    }
}

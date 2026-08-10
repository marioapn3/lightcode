use super::{bound, Tool, ToolDef};
use crate::web;
use serde_json::{json, Value};

pub struct WebFetch;

#[async_trait::async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "web_fetch".into(),
            description:
                "Fetch a URL and return its text content (HTML is stripped to text, bounded)."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "URL to fetch"}
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("web_fetch: missing 'url' argument")?;
        let text = web::fetch::fetch_text(url).await?;
        Ok(bound(format!("=== {url} ===\n{text}")))
    }
}

pub struct WebSearch;

#[async_trait::async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: "web_search".into(),
            description: "Search the web and return titles, URLs and snippets (bounded).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "num_results": {"type": "number", "description": "Max results (default 5, max 10)"}
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("web_search: missing 'query' argument")?;
        let max = args
            .get("num_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .clamp(1, 10) as usize;

        let results = web::search::search(query, max).await?;
        if results.is_empty() {
            return Ok(format!("web_search: no results for '{query}'"));
        }
        let mut out = format!("Search results for '{query}':\n");
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}\n   {}\n   {}\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }
        Ok(bound(out))
    }
}

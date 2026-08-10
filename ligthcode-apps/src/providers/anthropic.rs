use super::{sse, Message, Provider, ProviderError, StreamEvent, ToolDef};
use crate::config::ProviderConfig;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;

/// Anthropic Messages API provider. Also serves Anthropic-compatible endpoints
/// (e.g. OpenCode Go's `/v1/messages` models) via a `base_url` override.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(cfg: ProviderConfig, api_key: String) -> Self {
        let model = if cfg.model.is_empty() {
            "claude-sonnet-4-5".to_string()
        } else {
            cfg.model
        };
        let base_url = cfg
            .base_url
            .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client with static builder config cannot fail");
        Self {
            client,
            api_key,
            model,
            base_url,
            max_tokens: cfg.max_tokens.unwrap_or(8192),
        }
    }

    /// Translate our history into Anthropic's `system` string + messages array.
    /// Consecutive messages with the same role are merged.
    fn to_wire(&self, messages: &[Message]) -> (String, Vec<Value>) {
        let mut system = String::new();
        let mut msgs: Vec<Value> = Vec::new();
        for m in messages {
            match m {
                Message::System { content } => {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(content);
                }
                Message::User { content } => msgs.push(json!({
                    "role": "user",
                    "content": [{"type": "text", "text": content}]
                })),
                Message::Assistant {
                    content,
                    reasoning: _,
                    tool_calls,
                } => {
                    let mut blocks: Vec<Value> = Vec::new();
                    if let Some(t) = content {
                        if !t.is_empty() {
                            blocks.push(json!({"type": "text", "text": t}));
                        }
                    }
                    for tc in tool_calls {
                        let input = if tc.arguments.is_null() {
                            json!({})
                        } else {
                            tc.arguments.clone()
                        };
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": input
                        }));
                    }
                    if blocks.is_empty() {
                        blocks.push(json!({"type": "text", "text": ""}));
                    }
                    msgs.push(json!({"role": "assistant", "content": blocks}));
                }
                Message::Tool {
                    tool_call_id,
                    content,
                } => msgs.push(json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": tool_call_id, "content": content}]
                })),
            }
        }

        // Merge adjacent same-role messages (Anthropic requires alternation).
        let mut merged: Vec<Value> = Vec::new();
        for m in msgs {
            if let Some(last) = merged.last_mut() {
                if last["role"] == m["role"] {
                    if let (Some(a), Some(b)) =
                        (last["content"].as_array_mut(), m["content"].as_array())
                    {
                        a.extend(b.iter().cloned());
                        continue;
                    }
                }
            }
            merged.push(m);
        }
        (system, merged)
    }

    fn to_wire_tools(&self, tools: &[ToolDef]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect()
    }

    /// Extract events from one Anthropic SSE record.
    fn parse_event(v: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        let Some(t) = v["type"].as_str() else {
            return out;
        };
        match t {
            "content_block_start" => {
                if v["content_block"]["type"] == "tool_use" {
                    out.push(StreamEvent::ToolCallDelta {
                        index: v["index"].as_u64().unwrap_or(0) as usize,
                        id: v["content_block"]["id"].as_str().map(String::from),
                        name: v["content_block"]["name"].as_str().map(String::from),
                        arguments: None,
                    });
                }
            }
            "content_block_delta" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                match v["delta"]["type"].as_str() {
                    Some("text_delta") => {
                        if let Some(t) = v["delta"]["text"].as_str() {
                            out.push(StreamEvent::Text(t.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(t) = v["delta"]["thinking"].as_str() {
                            out.push(StreamEvent::Reasoning(t.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(p) = v["delta"]["partial_json"].as_str() {
                            out.push(StreamEvent::ToolCallDelta {
                                index,
                                id: None,
                                name: None,
                                arguments: Some(p.to_string()),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "message_stop" => out.push(StreamEvent::Done),
            "error" => {
                if let Some(m) = v["error"]["message"].as_str() {
                    out.push(StreamEvent::Error(m.to_string()));
                }
            }
            _ => {}
        }
        out
    }
}

#[async_trait::async_trait]
impl Provider for AnthropicProvider {
    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>, ProviderError> {
        let (system, msgs) = self.to_wire(messages);
        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("max_tokens".into(), json!(self.max_tokens));
        if !system.is_empty() {
            body.insert("system".into(), json!(system));
        }
        body.insert("messages".into(), json!(msgs));
        if !tools.is_empty() {
            body.insert("tools".into(), json!(self.to_wire_tools(tools)));
            body.insert("tool_choice".into(), json!({"type": "auto"}));
        }

        let (tx, rx) = mpsc::channel(64);
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/messages", self.base_url);
        let body = Value::Object(body);
        let model = self.model.clone();
        crate::log_line!("anthropic request model={model} url={url}");

        tokio::spawn(async move {
            let mut request = client.post(&url).header("anthropic-version", "2023-06-01");
            if !api_key.is_empty() {
                request = request.header("x-api-key", &api_key).bearer_auth(&api_key);
            }
            let resp = match request.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    crate::log_line!("anthropic send error: {e}");
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                    return;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                crate::log_line!(
                    "anthropic non-2xx status={status} body={}",
                    &text[..text.len().min(400)]
                );
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "{status}: {}",
                        &text[..text.len().min(400)]
                    )))
                    .await;
                return;
            }
            crate::log_line!("anthropic stream start status={status} model={model}");
            sse::stream_sse(resp, &tx, AnthropicProvider::parse_event).await;
        });

        Ok(rx)
    }

    fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    fn clone_box(&self) -> Box<dyn Provider> {
        Box::new(Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            max_tokens: self.max_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;

    #[test]
    fn translates_history_to_anthropic_format() {
        let p = AnthropicProvider {
            client: reqwest::Client::new(),
            api_key: "k".into(),
            model: "m".into(),
            base_url: "b".into(),
            max_tokens: 1024,
        };
        let msgs = vec![
            Message::System {
                content: "You are LightCode.".into(),
            },
            Message::User {
                content: "hi".into(),
            },
            Message::Assistant {
                content: Some("thinking".into()),
                reasoning: Some("chain of thought".into()),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "grep".into(),
                    arguments: json!({"pattern": "foo"}),
                }],
            },
            Message::Tool {
                tool_call_id: "c1".into(),
                content: "a match".into(),
            },
            Message::User {
                content: "thanks".into(),
            },
        ];
        let (system, msgs) = p.to_wire(&msgs);
        assert_eq!(system, "You are LightCode.");
        assert_eq!(msgs.len(), 3, "consecutive users must be merged");
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"][0]["type"], "text");
        assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
        assert_eq!(msgs[1]["content"][1]["name"], "grep");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "c1");
        assert_eq!(msgs[2]["content"][1]["text"], "thanks");
    }

    #[test]
    fn parses_text_and_tool_deltas() {
        let v = json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "Hel"}});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::Text(t)] if t == "Hel"
        ));

        let v = json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "tool_use", "id": "toolu_1", "name": "grep", "input": {}}});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::ToolCallDelta { index: 1, id: Some(i), name: Some(n), arguments: None }]
            if i == "toolu_1" && n == "grep"
        ));

        let v = json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"pat"}});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::ToolCallDelta { index: 1, arguments: Some(a), .. }] if a == "{\"pat"
        ));

        let v = json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "let me think"}});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::Reasoning(r)] if r == "let me think"
        ));
    }

    #[test]
    fn parses_error_and_stop() {
        let v = json!({"type": "error", "error": {"message": "boom"}});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::Error(e)] if e == "boom"
        ));
        let v = json!({"type": "message_stop"});
        assert!(matches!(
            &AnthropicProvider::parse_event(&v)[..],
            [StreamEvent::Done]
        ));
    }
}

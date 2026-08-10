use super::{sse, Message, Provider, ProviderError, StreamEvent, ToolDef};
use crate::config::ProviderConfig;
use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio::sync::mpsc;

/// OpenAI-compatible chat completions provider (streaming). Works with OpenAI and any
/// compatible endpoint (OpenRouter, Azure via base_url, local servers, ...).
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(cfg: ProviderConfig, api_key: String) -> Self {
        let model = if cfg.model.is_empty() {
            "gpt-4o-mini".to_string()
        } else {
            cfg.model
        };
        let base_url = cfg
            .base_url
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client with static builder config cannot fail");
        Self {
            client,
            api_key,
            model,
            base_url,
        }
    }

    fn build_body(&self, messages: &[Message], tools: &[ToolDef]) -> Value {
        let mut body: Map<String, Value> = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("messages".into(), json!(self.to_wire(messages)));
        body.insert("stream".into(), json!(true));
        if !tools.is_empty() {
            body.insert("tools".into(), json!(self.to_wire_tools(tools)));
            body.insert("tool_choice".into(), json!("auto"));
        }
        Value::Object(body)
    }

    fn to_wire(&self, messages: &[Message]) -> Vec<Value> {
        messages
            .iter()
            .map(|m| match m {
                Message::System { content } => {
                    json!({"role": "system", "content": content})
                }
                Message::User { content } => json!({"role": "user", "content": content}),
                Message::Assistant {
                    content,
                    reasoning: _,
                    tool_calls,
                } => json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": tool_calls.iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {"name": tc.name, "arguments": tc.arguments.to_string()},
                    })).collect::<Vec<_>>(),
                }),
                Message::Tool {
                    tool_call_id,
                    content,
                } => json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                }),
            })
            .collect()
    }

    fn to_wire_tools(&self, tools: &[ToolDef]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect()
    }

    /// Extract events from one streaming delta chunk.
    fn parse_delta(v: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        let Some(delta) = v["choices"][0]["delta"].as_object() else {
            return out;
        };
        if let Some(c) = delta.get("content").and_then(|x| x.as_str()) {
            out.push(StreamEvent::Text(c.to_string()));
        }
        // o-series / reasoning models expose their chain of thought here.
        if let Some(r) = delta.get("reasoning_content").and_then(|x| x.as_str()) {
            if !r.is_empty() {
                out.push(StreamEvent::Reasoning(r.to_string()));
            }
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
            for tc in tcs {
                out.push(StreamEvent::ToolCallDelta {
                    index: tc["index"].as_u64().unwrap_or(0) as usize,
                    id: tc["id"].as_str().map(String::from),
                    name: tc["function"]["name"].as_str().map(String::from),
                    arguments: tc["function"]["arguments"].as_str().map(String::from),
                });
            }
        }
        out
    }
}

#[async_trait::async_trait]
impl Provider for OpenAiProvider {
    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>, ProviderError> {
        let (tx, rx) = mpsc::channel(64);
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.base_url);
        let body = self.build_body(messages, tools);
        let model = self.model.clone();
        crate::log_line!(
            "openai request model={model} url={url} messages={}",
            messages.len()
        );

        tokio::spawn(async move {
            let mut request = client.post(&url);
            if !api_key.is_empty() {
                request = request.bearer_auth(&api_key);
            }
            let resp = match request.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    crate::log_line!("openai send error: {e}");
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                    return;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                crate::log_line!(
                    "openai non-2xx status={status} body={}",
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
            crate::log_line!("openai stream start status={status} model={model}");
            sse::stream_sse(resp, &tx, OpenAiProvider::parse_delta).await;
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;

    #[test]
    fn wire_messages_match_openai_format() {
        let p = OpenAiProvider {
            model: "m".into(),
            api_key: "k".into(),
            base_url: "b".into(),
            client: reqwest::Client::new(),
        };
        let msgs = vec![
            Message::User {
                content: "hi".into(),
            },
            Message::Assistant {
                content: None,
                reasoning: None,
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "grep".into(),
                    arguments: json!({"pattern": "foo"}),
                }],
            },
            Message::Tool {
                tool_call_id: "c1".into(),
                content: "match".into(),
            },
        ];
        let wire = p.to_wire(&msgs);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[1]["role"], "assistant");
        assert_eq!(wire[1]["tool_calls"][0]["type"], "function");
        assert_eq!(
            wire[1]["tool_calls"][0]["function"]["arguments"],
            "{\"pattern\":\"foo\"}"
        );
        assert_eq!(wire[2]["role"], "tool");
        assert_eq!(wire[2]["tool_call_id"], "c1");
    }

    #[test]
    fn parse_delta_extracts_text() {
        let v = json!({"choices": [{"delta": {"content": "hel"}}]});
        let events = OpenAiProvider::parse_delta(&v);
        assert!(matches!(&events[..], [StreamEvent::Text(t)] if t == "hel"));
    }

    #[test]
    fn parse_delta_extracts_tool_call_fragments() {
        let v = json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call_1",
                "function": {"name": "grep", "arguments": "{\"pat"}
            }]}}]
        });
        let events = OpenAiProvider::parse_delta(&v);
        assert!(matches!(
            &events[..],
            [StreamEvent::ToolCallDelta { index: 0, id: Some(i), name: Some(n), arguments: Some(a) }]
            if i == "call_1" && n == "grep" && a == "{\"pat"
        ));
    }
}

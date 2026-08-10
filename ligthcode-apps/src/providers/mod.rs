pub mod anthropic;
pub mod openai;
pub mod sse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

/// A single tool invocation requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Provider-agnostic message history. Each provider translates these to its wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        /// Reasoning/thinking text, kept for history replay but never re-sent.
        #[serde(default)]
        reasoning: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// Schema for a tool, advertised to the provider so it can call it.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Incremental chunks from a streaming completion.
#[derive(Debug)]
pub enum StreamEvent {
    Text(String),
    /// Model reasoning / thinking content (e.g. Anthropic thinking blocks, o-series `reasoning_content`).
    Reasoning(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
    Done,
    Error(String),
}

#[derive(Debug)]
pub struct ProviderError(pub String);

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderError {}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        ProviderError(e.to_string())
    }
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Start a streaming completion. Events arrive on the returned channel.
    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>, ProviderError>;

    /// Switch the active model at runtime (used by the TUI model picker).
    fn set_model(&mut self, model: &str);

    /// Clone this provider as a boxed trait object (used by sub-agents).
    fn clone_box(&self) -> Box<dyn Provider>;
}

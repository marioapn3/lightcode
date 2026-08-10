use crate::providers::Message;

/// Rough token estimate: characters / 4. Good enough for compaction thresholds.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    let chars: usize = messages.iter().map(measure).sum();
    chars / 4
}

fn measure(m: &Message) -> usize {
    match m {
        Message::System { content } | Message::User { content } => content.chars().count(),
        Message::Assistant {
            content,
            reasoning,
            tool_calls,
        } => {
            content.as_ref().map_or(0, |c| c.chars().count())
                + reasoning.as_ref().map_or(0, |r| r.chars().count())
                + tool_calls
                    .iter()
                    .map(|t| t.arguments.to_string().chars().count())
                    .sum::<usize>()
        }
        Message::Tool { content, .. } => content.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;
    use serde_json::json;

    #[test]
    fn counts_text() {
        let msgs = vec![
            Message::System {
                content: "abcd".into(),
            },
            Message::User {
                content: "efghijkl".into(),
            },
        ];
        assert_eq!(estimate_tokens(&msgs), 3); // 12 chars / 4
    }

    #[test]
    fn counts_tool_calls_and_results() {
        let msgs = vec![
            Message::Assistant {
                content: Some("hi".into()),
                reasoning: None,
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "grep".into(),
                    arguments: json!({"pattern": "xyz"}),
                }],
            },
            Message::Tool {
                tool_call_id: "c1".into(),
                content: "01234567".into(),
            },
        ];
        let tokens = estimate_tokens(&msgs);
        assert!(tokens > 4, "tool call args and result must count: {tokens}");
    }
}

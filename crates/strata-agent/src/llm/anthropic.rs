//! Anthropic Messages API provider.

use serde_json::{json, Value};

use super::{ApiKey, ChatRequest, ChatResponse, ContentBlock, Provider, StopReason, ToolChoice, Usage};
use crate::error::AgentError;

const DEFAULT_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

pub struct Anthropic {
    api_key: ApiKey,
    url: String,
    pub(super) default_model: String,
}

impl Anthropic {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key: ApiKey::new(api_key),
            url: DEFAULT_URL.to_string(),
            default_model: DEFAULT_MODEL.to_string(),
        }
    }

}

impl Provider for Anthropic {
    fn endpoint(&self) -> &str {
        &self.url
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("x-api-key", self.api_key.as_str())
            .header("anthropic-version", API_VERSION)
    }

    fn build_body(
        &self,
        request: &ChatRequest,
        model: &str,
        max_tokens: u32,
    ) -> Value {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    super::Role::User => "user",
                    super::Role::Assistant => "assistant",
                };
                let content: Vec<Value> = m.content.iter().map(content_to_wire).collect();
                json!({ "role": role, "content": content })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
        });

        if let Some(system) = &request.system {
            body["system"] = json!(system);
        }

        if !request.tools.is_empty() && !matches!(request.tool_choice, ToolChoice::None) {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            body["tools"] = json!(tools);

            match &request.tool_choice {
                ToolChoice::Auto => body["tool_choice"] = json!({"type": "auto"}),
                ToolChoice::Required => body["tool_choice"] = json!({"type": "any"}),
                ToolChoice::None => unreachable!(),
            }
        }

        body
    }

    fn parse_response(&self, body: Value) -> Result<ChatResponse, AgentError> {
        let content_arr = body["content"]
            .as_array()
            .ok_or_else(|| AgentError::Parse("missing content array".into()))?;

        let mut content = Vec::new();
        for block in content_arr {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default().to_string();
                    content.push(ContentBlock::Text { text });
                }
                Some("tool_use") => {
                    let id = block["id"]
                        .as_str()
                        .ok_or_else(|| AgentError::Parse("tool_use missing id".into()))?
                        .to_string();
                    let name = block["name"]
                        .as_str()
                        .ok_or_else(|| AgentError::Parse("tool_use missing name".into()))?
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
                _ => {} // skip unknown block types
            }
        }

        let stop_reason = match body["stop_reason"].as_str() {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = Usage {
            input_tokens: body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: body["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            stop_reason,
            usage,
        })
    }
}

fn content_to_wire(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let mut v = json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": content });
            if *is_error {
                v["is_error"] = json!(true);
            }
            v
        }
    }
}

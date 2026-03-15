//! OpenAI-compatible provider (OpenAI, OpenRouter, Ollama, Together, Groq, etc).

use serde_json::{json, Value};

use super::{ApiKey, ChatRequest, ChatResponse, ContentBlock, Provider, StopReason, ToolChoice, Usage};
use crate::error::AgentError;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o";

pub struct OpenAi {
    api_key: ApiKey,
    chat_url: String,
    pub(super) default_model: String,
}

impl OpenAi {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key: ApiKey::new(api_key),
            chat_url: format!("{DEFAULT_BASE_URL}/chat/completions"),
            default_model: DEFAULT_MODEL.to_string(),
        }
    }

    /// Create with a custom base URL. Requires HTTPS unless the host is localhost/127.0.0.1
    /// (for local providers like Ollama).
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let is_local = base_url.contains("://localhost") || base_url.contains("://127.0.0.1");
        assert!(
            base_url.starts_with("https://") || is_local,
            "base_url must use HTTPS (got: {base_url})"
        );
        let base = base_url.trim_end_matches('/');
        Self {
            api_key: ApiKey::new(api_key),
            chat_url: format!("{base}/chat/completions"),
            default_model: DEFAULT_MODEL.to_string(),
        }
    }
}

impl Provider for OpenAi {
    fn endpoint(&self) -> &str {
        &self.chat_url
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(self.api_key.as_str())
    }

    fn build_body(
        &self,
        request: &ChatRequest,
        model: &str,
        max_tokens: u32,
    ) -> Value {
        let mut messages: Vec<Value> = Vec::new();

        // OpenAI puts system prompt as a message with role "system".
        if let Some(system) = &request.system {
            messages.push(json!({ "role": "system", "content": system }));
        }

        for m in &request.messages {
            match m.role {
                super::Role::User => {
                    for block in &m.content {
                        match block {
                            ContentBlock::Text { text } => {
                                messages.push(json!({ "role": "user", "content": text }));
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            } => {
                                // OpenAI has no is_error field; prepend marker to content.
                                let body = if *is_error {
                                    format!("[ERROR] {content}")
                                } else {
                                    content.clone()
                                };
                                messages.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": body,
                                }));
                            }
                            _ => {}
                        }
                    }
                }
                super::Role::Assistant => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for block in &m.content {
                        match block {
                            ContentBlock::Text { text } => {
                                text_parts.push(text.as_str());
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": input.to_string(),
                                    }
                                }));
                            }
                            _ => {}
                        }
                    }

                    let mut msg = json!({ "role": "assistant" });
                    let combined = text_parts.join("");
                    if !combined.is_empty() {
                        msg["content"] = json!(combined);
                    } else {
                        // Providers like OpenRouter require explicit null.
                        msg["content"] = Value::Null;
                    }
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = json!(tool_calls);
                    }
                    messages.push(msg);
                }
            }
        }

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
        });

        if !request.tools.is_empty() {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools);

            match &request.tool_choice {
                ToolChoice::Auto => body["tool_choice"] = json!("auto"),
                ToolChoice::Required => body["tool_choice"] = json!("required"),
                ToolChoice::None => body["tool_choice"] = json!("none"),
            }
        }

        body
    }

    fn parse_response(&self, body: Value) -> Result<ChatResponse, AgentError> {
        let choice = body["choices"]
            .get(0)
            .ok_or_else(|| AgentError::Parse("missing choices array".into()))?;

        let message = &choice["message"];
        let mut content = Vec::new();
        let mut has_tool_calls = false;

        // Text content.
        if let Some(text) = message["content"].as_str() {
            if !text.is_empty() {
                content.push(ContentBlock::Text {
                    text: text.to_string(),
                });
            }
        }

        // Tool calls.
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or_default().to_string();
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).map_err(|e| {
                    AgentError::Parse(format!(
                        "malformed tool arguments for {name}: {e}"
                    ))
                })?;
                content.push(ContentBlock::ToolUse { id, name, input });
                has_tool_calls = true;
            }
        }

        // Derive stop_reason from tool call presence, not just finish_reason.
        // Some providers (OpenRouter) return "stop" even when tool calls are present.
        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            match choice["finish_reason"].as_str() {
                Some("length") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            }
        };

        let usage = Usage {
            input_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            stop_reason,
            usage,
        })
    }
}

//! Multi-provider LLM client with function calling support.
//!
//! Canonical types (`ChatRequest`, `ChatResponse`, etc.) are provider-agnostic.
//! Each provider module converts to/from these types and the provider's wire format.

mod anthropic;
mod openai;

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::AgentError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// API key wrapper that redacts its value in Debug/Display output.
#[derive(Clone)]
pub(crate) struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

pub use anthropic::Anthropic;
pub use openai::OpenAi;

/// Test helper: parse a raw JSON response using a provider's parser.
#[doc(hidden)]
pub fn test_parse_response(
    provider: &dyn Provider,
    body: serde_json::Value,
) -> Result<ChatResponse, crate::error::AgentError> {
    provider.parse_response(body)
}

/// Test helper: build a request body using a provider's serializer.
#[doc(hidden)]
pub fn test_build_body(
    provider: &dyn Provider,
    request: &ChatRequest,
    model: &str,
    max_tokens: u32,
) -> serde_json::Value {
    provider.build_body(request, model, max_tokens)
}

// --- Provider trait ---

pub trait Provider: Send + Sync {
    fn endpoint(&self) -> &str;
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder;
    fn build_body(
        &self,
        request: &ChatRequest,
        model: &str,
        max_tokens: u32,
    ) -> serde_json::Value;
    fn parse_response(&self, body: serde_json::Value) -> Result<ChatResponse, AgentError>;
}

// --- Client ---

const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Multi-provider LLM client.
pub struct LlmClient {
    http: Client,
    provider: Box<dyn Provider>,
    model: String,
    max_tokens: u32,
}

impl LlmClient {
    /// Connect to the Anthropic Messages API.
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        let provider = Anthropic::new(api_key.into());
        let model = provider.default_model.clone();
        Self {
            http: Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("failed to build HTTP client"),
            provider: Box::new(provider),
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Connect to an OpenAI-compatible API (OpenAI, OpenRouter, Ollama, etc).
    pub fn openai(api_key: impl Into<String>) -> Self {
        let provider = OpenAi::new(api_key.into());
        let model = provider.default_model.clone();
        Self {
            http: Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("failed to build HTTP client"),
            provider: Box::new(provider),
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Connect to an OpenAI-compatible API at a custom base URL.
    /// Use this for OpenRouter, Ollama, Together, Groq, etc.
    pub fn openai_compatible(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let provider = OpenAi::with_base_url(api_key.into(), base_url.into());
        let model = provider.default_model.clone();
        Self {
            http: Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("failed to build HTTP client"),
            provider: Box::new(provider),
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Send a chat request and return the parsed response.
    pub async fn send(&self, request: &ChatRequest) -> Result<ChatResponse, AgentError> {
        let body = self
            .provider
            .build_body(request, &self.model, self.max_tokens);

        let req = self.http.post(self.provider.endpoint()).json(&body);
        let req = self.provider.auth(req);

        let resp = req.send().await.map_err(AgentError::Http)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::Api { status, body });
        }

        let raw: serde_json::Value = resp.json().await.map_err(AgentError::Http)?;
        self.provider.parse_response(raw)
    }
}

// --- Canonical types (provider-agnostic) ---

/// A conversation request to build up before sending.
#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub tool_choice: ToolChoice,
}

/// Controls how the model uses tools.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToolChoice {
    /// Let the model decide (default).
    #[default]
    Auto,
    /// Model must use a tool.
    Required,
    /// Model must not use tools.
    None,
}

impl ChatRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.messages.push(Message::user_text(text));
    }

    pub fn push_assistant(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: Role::Assistant,
            content,
        });
    }

    /// Push one tool result. If the last message is already a User message
    /// containing tool results, appends to it (required by Anthropic's
    /// alternating-turn constraint when handling multiple tool calls).
    pub fn push_tool_result(&mut self, tool_use_id: String, content: String, is_error: bool) {
        let block = ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        };

        // Append to the existing user/tool-result message if possible.
        if let Some(last) = self.messages.last_mut() {
            if last.role == Role::User
                && last
                    .content
                    .iter()
                    .all(|b| matches!(b, ContentBlock::ToolResult { .. }))
            {
                last.content.push(block);
                return;
            }
        }

        self.messages.push(Message {
            role: Role::User,
            content: vec![block],
        });
    }
}

// --- Message types ---

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A block within a message. Provider modules convert to/from their wire formats.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

// --- Tool definition ---

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// --- Response types ---

#[derive(Clone, Debug)]
pub struct ChatResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

impl ChatResponse {
    /// Extract all text blocks concatenated.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extract tool use blocks as (id, name, input) tuples.
    pub fn tool_calls(&self) -> Vec<ToolCall<'_>> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some(ToolCall { id, name, input }),
                _ => None,
            })
            .collect()
    }

    /// Whether the model wants to call tools.
    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }
}

/// A reference to a tool call within a response.
#[derive(Debug)]
pub struct ToolCall<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub input: &'a serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

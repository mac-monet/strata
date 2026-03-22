//! HTTP server with A2A (Agent-to-Agent) protocol support.
//!
//! Implements the minimal A2A subset: `message/send` (synchronous) and the Agent Card endpoint.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use serde::{Deserialize, Serialize};

use alloy::signers::local::PrivateKeySigner;

use crate::agent::{self, AgentConfig};
use crate::error::AgentError;
use crate::llm::{self, LlmClient};
use crate::pipeline::TransitionOutput;
use crate::poster::{self, PosterConfig};
use crate::prover::{self, ProverConfig};
use crate::tools::ToolExecutor;

/// Optional on-chain posting configuration.
pub struct PostingConfig {
    pub poster: PosterConfig,
    pub signer: PrivateKeySigner,
}

// --- JSON-RPC error codes ---

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INTERNAL_ERROR: i32 = -32603;

// --- Shared state ---

pub struct AppState<E: RStorage + Clock + Metrics> {
    pub(crate) config: tokio::sync::Mutex<AgentConfig>,
    pub(crate) client: LlmClient,
    pub(crate) executor: tokio::sync::Mutex<ToolExecutor<E>>,
    pub(crate) transitions: tokio::sync::Mutex<Vec<TransitionOutput>>,
    pub(crate) posting: Option<PostingConfig>,
    pub(crate) prover: Option<ProverConfig>,
}

impl<E: RStorage + Clock + Metrics> AppState<E> {
    pub fn new(
        config: AgentConfig,
        client: LlmClient,
        executor: ToolExecutor<E>,
        posting: Option<PostingConfig>,
        prover: Option<ProverConfig>,
    ) -> Self {
        Self {
            config: tokio::sync::Mutex::new(config),
            client,
            executor: tokio::sync::Mutex::new(executor),
            transitions: tokio::sync::Mutex::new(Vec::new()),
            posting,
            prover,
        }
    }
}

// --- JSON-RPC types ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: serde_json::Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

fn rpc_error(
    status: StatusCode,
    id: serde_json::Value,
    code: i32,
    message: impl Into<String>,
) -> (StatusCode, Json<JsonRpcResponse>) {
    (
        status,
        Json(JsonRpcResponse::error(id, code, message.into())),
    )
}

// --- A2A message types ---

#[derive(Deserialize)]
struct SendMessageParams {
    message: A2aMessage,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct A2aMessage {
    message_id: String,
    role: String,
    parts: Vec<A2aPart>,
}

#[derive(Serialize, Deserialize)]
struct A2aPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct A2aTask {
    id: String,
    status: A2aTaskStatus,
}

#[derive(Serialize)]
struct A2aTaskStatus {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<A2aMessage>,
}

// --- Handlers ---

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn agent_card<E: RStorage + Clock + Metrics>(
    State(_state): State<Arc<AppState<E>>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "Strata Agent",
        "description": "General-purpose agent with persistent, verifiable memory",
        "version": "0.1.0",
        "capabilities": { "streaming": false, "pushNotifications": false },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": "general",
            "name": "General",
            "description": "General purpose agent with persistent memory",
            "tags": ["memory", "general"]
        }]
    }))
}

async fn handle_a2a<E: RStorage + Clock + Metrics + 'static>(
    State(state): State<Arc<AppState<E>>>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<JsonRpcResponse>) {
    let rpc: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                serde_json::Value::Null,
                PARSE_ERROR,
                format!("parse error: {e}"),
            );
        }
    };

    if rpc.jsonrpc != "2.0" {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            rpc.id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        );
    }

    match rpc.method.as_str() {
        "message/send" => handle_message_send(state, rpc.id, rpc.params).await,
        _ => rpc_error(
            StatusCode::OK,
            rpc.id,
            METHOD_NOT_FOUND,
            format!("method not found: {}", rpc.method),
        ),
    }
}

async fn handle_message_send<E: RStorage + Clock + Metrics>(
    state: Arc<AppState<E>>,
    id: serde_json::Value,
    params: serde_json::Value,
) -> (StatusCode, Json<JsonRpcResponse>) {
    let send_params: SendMessageParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                id,
                INVALID_REQUEST,
                format!("invalid params: {e}"),
            );
        }
    };

    let text = send_params
        .message
        .parts
        .iter()
        .filter_map(|p| p.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");

    if text.is_empty() {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            id,
            INVALID_REQUEST,
            "message contains no text parts",
        );
    }

    let messages = vec![llm::Message::user_text(&text)];
    let mut config = state.config.lock().await;
    let mut executor = state.executor.lock().await;

    match agent::interact(&mut config, &state.client, &mut executor, &messages).await {
        Ok(result) => {
            if let Some(transition) = result.transition {
                if let Some(posting) = &state.posting {
                    let proof_bytes = if let Some(prover_config) = &state.prover {
                        match prover::prove(prover_config, &transition).await {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                return rpc_error(
                                    StatusCode::OK,
                                    id,
                                    INTERNAL_ERROR,
                                    format!("proof generation failed: {e}"),
                                );
                            }
                        }
                    } else {
                        vec![] // mock mode
                    };
                    if let Err(e) = post_with_retry(posting, &transition, proof_bytes).await {
                        return rpc_error(
                            StatusCode::OK,
                            id,
                            INTERNAL_ERROR,
                            format!("transition succeeded locally but posting failed: {e}"),
                        );
                    }
                }
                state.transitions.lock().await.push(transition);
            }

            let task = A2aTask {
                id: uuid::Uuid::new_v4().to_string(),
                status: A2aTaskStatus {
                    state: "completed",
                    message: Some(A2aMessage {
                        message_id: uuid::Uuid::new_v4().to_string(),
                        role: "agent".into(),
                        parts: vec![A2aPart {
                            text: Some(result.response),
                        }],
                    }),
                },
            };

            match serde_json::to_value(task) {
                Ok(v) => (StatusCode::OK, Json(JsonRpcResponse::success(id, v))),
                Err(e) => rpc_error(
                    StatusCode::OK,
                    id,
                    INTERNAL_ERROR,
                    format!("failed to serialize response: {e}"),
                ),
            }
        }
        Err(e) => rpc_error(StatusCode::OK, id, INTERNAL_ERROR, e.to_string()),
    }
}

// --- Posting with retry ---

const POST_MAX_RETRIES: u32 = 5;
const POST_RETRY_BASE_MS: u64 = 500;

async fn post_with_retry(
    posting: &PostingConfig,
    transition: &TransitionOutput,
    proof_bytes: Vec<u8>,
) -> Result<(), AgentError> {
    let nonce = transition.new_state.nonce.get();
    let mut last_err = None;
    for attempt in 0..POST_MAX_RETRIES {
        match poster::post(
            &posting.poster,
            posting.signer.clone(),
            proof_bytes.clone(),
            transition.public_values,
            transition,
        )
        .await
        {
            Ok(hash) => {
                eprintln!("posted transition nonce={nonce}, tx={hash}");
                return Ok(());
            }
            Err(e) => {
                eprintln!(
                    "post attempt {}/{POST_MAX_RETRIES} failed for nonce={nonce}: {e}",
                    attempt + 1
                );
                last_err = Some(e);
                if attempt + 1 < POST_MAX_RETRIES {
                    let delay = POST_RETRY_BASE_MS * 2u64.pow(attempt);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AgentError::Poster("posting failed".into())))
}

// --- Router + startup ---

pub fn router<E: RStorage + Clock + Metrics + 'static>(state: Arc<AppState<E>>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/.well-known/agent.json", get(agent_card::<E>))
        .route("/a2a", post(handle_a2a::<E>))
        .with_state(state)
}

pub async fn run<E: RStorage + Clock + Metrics + 'static>(
    state: Arc<AppState<E>>,
    addr: std::net::SocketAddr,
) -> Result<(), AgentError> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AgentError::Agent(format!("failed to bind: {e}")))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| AgentError::Agent(format!("server error: {e}")))
}

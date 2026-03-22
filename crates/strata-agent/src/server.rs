//! HTTP server with A2A (Agent-to-Agent) protocol support.
//!
//! Implements the minimal A2A subset: `message/send` (synchronous) and the Agent Card endpoint.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use commonware_codec::Encode;
use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use serde::{Deserialize, Serialize};

use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;

use crate::agent::{self, AgentConfig};
use crate::batch::PendingBatch;
use crate::identity::IdentityConfig;
use crate::llm::{self, LlmClient};
use crate::pipeline::TransitionOutput;
use crate::poster::PosterConfig;
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
    /// Pending transitions awaiting batch proof + post. Drained by the
    /// background batch task. `None` when posting is disabled.
    pub(crate) pending_batch: Option<Arc<PendingBatch>>,
    pub(crate) identity: IdentityConfig,
    pub(crate) rollup_address: Address,
    pub proofs_dir: PathBuf,
}

impl<E: RStorage + Clock + Metrics> AppState<E> {
    pub fn new(
        config: AgentConfig,
        client: LlmClient,
        executor: ToolExecutor<E>,
        pending_batch: Option<Arc<PendingBatch>>,
        identity: IdentityConfig,
        rollup_address: Address,
    ) -> Self {
        Self {
            config: tokio::sync::Mutex::new(config),
            client,
            executor: tokio::sync::Mutex::new(executor),
            transitions: tokio::sync::Mutex::new(Vec::new()),
            pending_batch,
            identity,
            rollup_address,
            proofs_dir: PathBuf::from("proofs"),
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
    State(state): State<Arc<AppState<E>>>,
) -> Json<serde_json::Value> {
    let mut card = serde_json::json!({
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
    });

    let id = &state.identity;
    card["identity"] = serde_json::json!({
        "erc8004": {
            "agentId": id.agent_id,
            "registry": format!("{:#x}", id.registry_address),
            "chain": "eip155:8453"
        }
    });

    Json(card)
}

async fn agent_registration<E: RStorage + Clock + Metrics>(
    State(state): State<Arc<AppState<E>>>,
) -> Json<serde_json::Value> {
    let id = &state.identity;
    let base_url = id.agent_base_url.trim_end_matches('/');

    Json(serde_json::json!({
        "type": "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
        "name": "Strata Agent",
        "description": "A persistent, verifiable AI agent whose cognitive state lives on-chain as a custom rollup. Every memory and state transition is ZK-proven and posted to Base.",
        "image": "",
        "services": [
            {
                "name": "A2A",
                "endpoint": format!("{base_url}/.well-known/agent.json"),
                "version": "0.1.0"
            },
            {
                "name": "agentWallet",
                "endpoint": format!("eip155:8453:{:#x}", state.rollup_address)
            }
        ],
        "registrations": [{
            "agentId": id.agent_id,
            "agentRegistry": format!("eip155:8453:{:#x}", id.registry_address)
        }],
        "supportedTrust": [],
        "active": true,
        "x402Support": false
    }))
}

async fn get_proof<E: RStorage + Clock + Metrics>(
    State(state): State<Arc<AppState<E>>>,
    Path(nonce): Path<u64>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = state.proofs_dir.join(format!("{nonce}.json"));
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(v) => (StatusCode::OK, Json(v)),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("corrupt proof file: {e}")})),
            ),
        },
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no proof found for nonce {nonce}")})),
        ),
    }
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
                let nonce = transition.new_state.nonce.get();
                eprintln!("transition nonce={nonce} buffered for batch");

                // Persist proof to disk.
                if let Err(e) = save_proof(&state.proofs_dir, &transition).await {
                    eprintln!("warning: failed to save proof for nonce {nonce}: {e}");
                }

                // Buffer for the background batch task.
                if let Some(pending) = &state.pending_batch {
                    pending.lock().await.push(transition);
                } else {
                    // No posting configured — just record locally.
                    state.transitions.lock().await.push(transition);
                }
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

// --- Proof persistence ---

async fn save_proof(dir: &std::path::Path, t: &TransitionOutput) -> Result<(), String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("create proofs dir: {e}"))?;

    let nonce = t.new_state.nonce.get();
    let body = serde_json::json!({
        "nonce": nonce,
        "oldState": {
            "soulHash": format!("0x{}", hex::encode(t.old_state.soul_hash.as_bytes())),
            "vectorIndexRoot": format!("0x{}", hex::encode(t.old_state.vector_index_root.as_bytes())),
            "nonce": t.old_state.nonce.get(),
        },
        "newState": {
            "soulHash": format!("0x{}", hex::encode(t.new_state.soul_hash.as_bytes())),
            "vectorIndexRoot": format!("0x{}", hex::encode(t.new_state.vector_index_root.as_bytes())),
            "nonce": t.new_state.nonce.get(),
        },
        "memoryContent": format!("0x{}", hex::encode(t.record.encode())),
    });

    let path = dir.join(format!("{nonce}.json"));
    let bytes = serde_json::to_vec_pretty(&body).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))?;

    eprintln!("proof saved: {}", path.display());
    Ok(())
}

// --- Router + startup ---

pub fn router<E: RStorage + Clock + Metrics + 'static>(state: Arc<AppState<E>>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/.well-known/agent.json", get(agent_card::<E>))
        .route(
            "/.well-known/agent-registration.json",
            get(agent_registration::<E>),
        )
        .route("/proof/{nonce}", get(get_proof::<E>))
        .route("/a2a", post(handle_a2a::<E>))
        .with_state(state)
}

pub async fn run<E: RStorage + Clock + Metrics + 'static>(
    state: Arc<AppState<E>>,
    addr: std::net::SocketAddr,
) -> Result<(), crate::error::AgentError> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| crate::error::AgentError::Agent(format!("failed to bind: {e}")))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::AgentError::Agent(format!("server error: {e}")))
}

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use commonware_runtime::{deterministic, Runner as _};
use http_body_util::BodyExt;
use hyper::Request;
use serde_json::json;
use strata_vector_db::VectorDB;
use tower::ServiceExt;

use strata_agent::agent::AgentConfig;
use strata_agent::batch::PendingBatch;
use strata_agent::llm::{
    ChatRequest, ChatResponse, ContentBlock, LlmClient, Message, Role, StopReason, Usage,
};
use strata_agent::identity::IdentityConfig;
use strata_agent::server::{self, AppState};
use strata_agent::tools::ToolExecutor;

fn test_identity() -> IdentityConfig {
    IdentityConfig {
        agent_id: 1,
        registry_address: alloy::primitives::Address::ZERO,
        agent_base_url: "http://localhost:3000".into(),
        rpc_url: String::new(),
    }
}

fn make_app_state(
    db: VectorDB<deterministic::Context>,
    client: LlmClient,
) -> Arc<AppState<deterministic::Context>> {
    let mut state = AppState::new(
        AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        },
        client,
        ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
        None, // no pending_batch
        test_identity(),
        alloy::primitives::Address::ZERO,
    );
    state.proofs_dir = tempfile::tempdir().unwrap().into_path();
    Arc::new(state)
}

fn dummy_client() -> LlmClient {
    LlmClient::anthropic("test-key")
}

fn mock_text_client(text: &str) -> LlmClient {
    let text = text.to_string();
    LlmClient::mock(move |_req| {
        Ok(ChatResponse {
            content: vec![ContentBlock::Text {
                text: text.clone(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
            },
        })
    })
}

#[test]
fn health_returns_200() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("health", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());
        let app = server::router(state);

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    });
}

#[test]
fn agent_card_returns_valid_json() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("card", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());
        let app = server::router(state);

        let req = Request::builder()
            .uri("/.well-known/agent.json")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let card: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(card["name"], "Strata Agent");
        assert_eq!(card["version"], "0.1.0");
        assert_eq!(card["capabilities"]["streaming"], false);
        assert!(!card["skills"].as_array().unwrap().is_empty());
    });
}

#[test]
fn unknown_method_returns_32601() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("unknown-method", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());
        let app = server::router(state);

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/stream",
            "params": {}
        });

        let req = Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(rpc["jsonrpc"], "2.0");
        assert_eq!(rpc["error"]["code"], -32601);
    });
}

#[test]
fn malformed_json_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("malformed", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());
        let app = server::router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .body(Body::from("not json"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(rpc["error"]["code"], -32700);
    });
}

#[test]
fn invalid_jsonrpc_version_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("bad-version", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());
        let app = server::router(state);

        let body = json!({
            "jsonrpc": "1.0",
            "id": 1,
            "method": "message/send",
            "params": {}
        });

        let req = Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(rpc["error"]["code"], -32600);
    });
}

#[test]
fn message_send_returns_completed_task() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("msg-send", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let client = mock_text_client("Hello from the agent!");
        let state = make_app_state(db, client);
        let app = server::router(state);

        let body = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "message/send",
            "params": {
                "message": {
                    "messageId": "msg-1",
                    "role": "user",
                    "parts": [{"text": "Hi there"}]
                }
            }
        });

        let req = Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(rpc["jsonrpc"], "2.0");
        assert_eq!(rpc["id"], 42);
        assert!(rpc["error"].is_null());

        let task = &rpc["result"];
        assert_eq!(task["status"]["state"], "completed");
        assert_eq!(
            task["status"]["message"]["parts"][0]["text"],
            "Hello from the agent!"
        );
        assert_eq!(task["status"]["message"]["role"], "agent");
    });
}

// --- Helper for multi-turn tests ---

async fn send_a2a(
    state: &Arc<AppState<deterministic::Context>>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let app = server::router(state.clone());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/a2a")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn count_user_messages(req: &ChatRequest) -> usize {
    req.messages
        .iter()
        .filter(|m| {
            m.role == Role::User
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { .. }))
        })
        .count()
}

// --- Multi-turn A2A tests ---

#[test]
fn multi_turn_conversation_preserves_history() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("multi-turn", &context);
        let db = VectorDB::new(context, config).await.unwrap();

        // Mock that records how many user text messages it received.
        let seen_counts = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let counts = seen_counts.clone();
        let client = LlmClient::mock(move |req: &ChatRequest| {
            let n = count_user_messages(req);
            counts.lock().unwrap().push(n);
            Ok(ChatResponse {
                content: vec![ContentBlock::Text {
                    text: format!("turn with {n} user messages"),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            })
        });

        let state = make_app_state(db, client);

        // Turn 1: new session (no taskId).
        let rpc = send_a2a(&state, "message/send", json!({
            "message": {
                "messageId": "m1", "role": "user",
                "parts": [{"text": "Hello"}]
            }
        }))
        .await;
        let task_id = rpc["result"]["id"].as_str().unwrap().to_string();

        // Turn 2: same session.
        send_a2a(&state, "message/send", json!({
            "taskId": task_id,
            "message": {
                "messageId": "m2", "role": "user",
                "parts": [{"text": "Follow up"}]
            }
        }))
        .await;

        // Turn 3: same session.
        send_a2a(&state, "message/send", json!({
            "taskId": task_id,
            "message": {
                "messageId": "m3", "role": "user",
                "parts": [{"text": "Third message"}]
            }
        }))
        .await;

        let counts = seen_counts.lock().unwrap();
        assert_eq!(counts.as_slice(), &[1, 2, 3]);
    });
}

#[test]
fn tasks_get_returns_last_status() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("tasks-get", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let client = mock_text_client("I remember you");
        let state = make_app_state(db, client);

        // Send a message to create a session.
        let rpc = send_a2a(&state, "message/send", json!({
            "message": {
                "messageId": "m1", "role": "user",
                "parts": [{"text": "hi"}]
            }
        }))
        .await;
        let task_id = rpc["result"]["id"].as_str().unwrap().to_string();

        // Retrieve the task.
        let rpc = send_a2a(&state, "tasks/get", json!({ "id": task_id })).await;
        assert!(rpc["error"].is_null());
        assert_eq!(rpc["result"]["id"], task_id);
        assert_eq!(rpc["result"]["status"]["state"], "completed");
        assert_eq!(
            rpc["result"]["status"]["message"]["parts"][0]["text"],
            "I remember you"
        );
    });
}

#[test]
fn tasks_get_unknown_id_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("tasks-get-unknown", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = make_app_state(db, dummy_client());

        let rpc = send_a2a(&state, "tasks/get", json!({ "id": "bogus-id" })).await;
        assert_eq!(rpc["error"]["code"], -32001);
    });
}

#[test]
fn separate_sessions_have_independent_history() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("separate-sessions", &context);
        let db = VectorDB::new(context, config).await.unwrap();

        let seen_counts = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let counts = seen_counts.clone();
        let client = LlmClient::mock(move |req: &ChatRequest| {
            let n = count_user_messages(req);
            counts.lock().unwrap().push(n);
            Ok(ChatResponse {
                content: vec![ContentBlock::Text {
                    text: "ok".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            })
        });

        let state = make_app_state(db, client);

        // Session A: turn 1.
        let rpc_a = send_a2a(&state, "message/send", json!({
            "message": {
                "messageId": "a1", "role": "user",
                "parts": [{"text": "Session A turn 1"}]
            }
        }))
        .await;
        let id_a = rpc_a["result"]["id"].as_str().unwrap().to_string();

        // Session B: turn 1.
        let rpc_b = send_a2a(&state, "message/send", json!({
            "message": {
                "messageId": "b1", "role": "user",
                "parts": [{"text": "Session B turn 1"}]
            }
        }))
        .await;
        let id_b = rpc_b["result"]["id"].as_str().unwrap().to_string();

        // Session A: turn 2.
        send_a2a(&state, "message/send", json!({
            "taskId": id_a,
            "message": {
                "messageId": "a2", "role": "user",
                "parts": [{"text": "Session A turn 2"}]
            }
        }))
        .await;

        // Session B: turn 2.
        send_a2a(&state, "message/send", json!({
            "taskId": id_b,
            "message": {
                "messageId": "b2", "role": "user",
                "parts": [{"text": "Session B turn 2"}]
            }
        }))
        .await;

        // Each session should see [1, 2] user messages, interleaved: [1, 1, 2, 2].
        let counts = seen_counts.lock().unwrap();
        assert_eq!(counts.as_slice(), &[1, 1, 2, 2]);
    });
}

// --- E2E: multi-turn conversation with memory + batch proving ---

fn make_app_state_with_batch(
    db: VectorDB<deterministic::Context>,
    client: LlmClient,
    pending: Arc<PendingBatch>,
) -> Arc<AppState<deterministic::Context>> {
    let mut state = AppState::new(
        AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        },
        client,
        ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
        Some(pending),
        test_identity(),
        alloy::primitives::Address::ZERO,
    );
    state.proofs_dir = tempfile::tempdir().unwrap().into_path();
    Arc::new(state)
}

/// E2E test: multi-turn conversation where the agent remembers things across
/// turns, producing state transitions that accumulate in the pending batch.
///
/// Turn 1: user says "remember the sky is blue" → agent calls remember tool → transition nonce 1
/// Turn 2: user says "remember grass is green" → agent calls remember tool → transition nonce 2
/// Turn 3: user says "what do you know?" → agent responds (no tool call) → no transition
///
/// Verifies:
/// - Session history grows across turns (turn 2 sees 2 user messages, turn 3 sees 3)
/// - Transitions with correct nonces are buffered in PendingBatch
/// - Turn without memory update produces no transition
/// - Trail includes assistant + tool-result messages from tool rounds
#[test]
fn e2e_multi_turn_remember_with_batch() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("e2e-batch", &context);
        let db = VectorDB::new(context, config).await.unwrap();

        // Track how many user messages the LLM sees per call.
        let call_log = Arc::new(Mutex::new(Vec::<usize>::new()));

        // Mock client that simulates: remember tool call → final text response.
        // Calls alternate: tool_use, end_turn, tool_use, end_turn, end_turn (no tool for turn 3).
        let responses: Arc<Mutex<Vec<ChatResponse>>> = Arc::new(Mutex::new(vec![
            // Turn 1: remember "the sky is blue"
            ChatResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "remember".into(),
                    input: json!({"text": "the sky is blue"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Usage { input_tokens: 10, output_tokens: 5 },
            },
            ChatResponse {
                content: vec![ContentBlock::Text { text: "I'll remember that the sky is blue.".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 15, output_tokens: 8 },
            },
            // Turn 2: remember "grass is green"
            ChatResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "call_2".into(),
                    name: "remember".into(),
                    input: json!({"text": "grass is green"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Usage { input_tokens: 20, output_tokens: 5 },
            },
            ChatResponse {
                content: vec![ContentBlock::Text { text: "Got it, grass is green.".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 25, output_tokens: 8 },
            },
            // Turn 3: no tool call
            ChatResponse {
                content: vec![ContentBlock::Text { text: "I know the sky is blue and grass is green.".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 30, output_tokens: 12 },
            },
        ]));

        let log = call_log.clone();
        let resps = responses.clone();
        let client = LlmClient::mock(move |req: &ChatRequest| {
            let n = count_user_messages(req);
            log.lock().unwrap().push(n);
            let mut q = resps.lock().unwrap();
            if q.is_empty() {
                return Err(strata_agent::error::AgentError::Agent("no more responses".into()));
            }
            Ok(q.remove(0))
        });

        let pending = Arc::new(PendingBatch::default());
        let state = make_app_state_with_batch(db, client, pending.clone());

        // --- Turn 1: remember the sky is blue ---
        let rpc = send_a2a(&state, "message/send", json!({
            "message": {
                "messageId": "m1", "role": "user",
                "parts": [{"text": "Remember the sky is blue"}]
            }
        }))
        .await;
        assert!(rpc["error"].is_null(), "turn 1 error: {}", rpc["error"]);
        let task_id = rpc["result"]["id"].as_str().unwrap().to_string();
        assert_eq!(
            rpc["result"]["status"]["message"]["parts"][0]["text"],
            "I'll remember that the sky is blue."
        );

        // --- Turn 2: remember grass is green (same session) ---
        let rpc = send_a2a(&state, "message/send", json!({
            "taskId": task_id,
            "message": {
                "messageId": "m2", "role": "user",
                "parts": [{"text": "Remember grass is green"}]
            }
        }))
        .await;
        assert!(rpc["error"].is_null(), "turn 2 error: {}", rpc["error"]);
        assert_eq!(
            rpc["result"]["status"]["message"]["parts"][0]["text"],
            "Got it, grass is green."
        );

        // --- Turn 3: query (no memory update, same session) ---
        let rpc = send_a2a(&state, "message/send", json!({
            "taskId": task_id,
            "message": {
                "messageId": "m3", "role": "user",
                "parts": [{"text": "What do you know?"}]
            }
        }))
        .await;
        assert!(rpc["error"].is_null(), "turn 3 error: {}", rpc["error"]);
        assert_eq!(
            rpc["result"]["status"]["message"]["parts"][0]["text"],
            "I know the sky is blue and grass is green."
        );

        // --- Verify conversation history grew correctly ---
        // Turn 1: 1 user msg (2 LLM calls: tool_use + end_turn, both see 1 user msg)
        // Turn 2: 2 user msgs (2 LLM calls: tool_use + end_turn, both see 2 user msgs)
        // Turn 3: 3 user msgs (1 LLM call: end_turn, sees 3 user msgs)
        let log = call_log.lock().unwrap();
        assert_eq!(log.as_slice(), &[1, 1, 2, 2, 3]);

        // --- Verify batch has 2 transitions with correct nonces ---
        let batch = pending.lock().await;
        assert_eq!(batch.len(), 2, "expected 2 transitions in batch");
        assert_eq!(batch[0].old_state.nonce.get(), 0);
        assert_eq!(batch[0].new_state.nonce.get(), 1);
        assert_eq!(batch[1].old_state.nonce.get(), 1);
        assert_eq!(batch[1].new_state.nonce.get(), 2);

        // Verify state continuity: batch[1].old_state == batch[0].new_state
        assert_eq!(
            batch[1].old_state.vector_index_root,
            batch[0].new_state.vector_index_root,
            "state roots should chain"
        );
        assert_eq!(
            batch[1].old_state.soul_hash,
            batch[0].new_state.soul_hash,
            "soul hashes should match"
        );

        // Verify memory content in transition records.
        assert_eq!(batch[0].record.contents.len(), 1);
        assert_eq!(batch[1].record.contents.len(), 1);

        // --- Verify proofs were saved to disk ---
        let proof_1 = state.proofs_dir.join("1.json");
        let proof_2 = state.proofs_dir.join("2.json");
        assert!(proof_1.exists(), "proof file for nonce 1 should exist");
        assert!(proof_2.exists(), "proof file for nonce 2 should exist");

        let p1: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&proof_1).unwrap()).unwrap();
        assert_eq!(p1["nonce"], 1);
        assert_eq!(p1["oldState"]["nonce"], 0);
        assert_eq!(p1["newState"]["nonce"], 1);

        let p2: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&proof_2).unwrap()).unwrap();
        assert_eq!(p2["nonce"], 2);
        assert_eq!(p2["oldState"]["nonce"], 1);
        assert_eq!(p2["newState"]["nonce"], 2);

        // --- Verify tasks/get returns latest status ---
        let rpc = send_a2a(&state, "tasks/get", json!({ "id": task_id })).await;
        assert!(rpc["error"].is_null());
        assert_eq!(rpc["result"]["status"]["state"], "completed");
        assert_eq!(
            rpc["result"]["status"]["message"]["parts"][0]["text"],
            "I know the sky is blue and grass is green."
        );
    });
}

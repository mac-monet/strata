mod common;

use std::sync::Arc;

use axum::body::Body;
use commonware_runtime::{deterministic, Runner as _};
use http_body_util::BodyExt;
use hyper::Request;
use serde_json::json;
use strata_vector_db::VectorDB;
use tower::ServiceExt;

use strata_agent::agent::AgentConfig;
use strata_agent::llm::{ChatResponse, ContentBlock, LlmClient, StopReason, Usage};
use strata_agent::server::{self, AppState};
use strata_agent::tools::ToolExecutor;

fn make_app_state(
    db: VectorDB<deterministic::Context>,
    client: LlmClient,
) -> Arc<AppState<deterministic::Context>> {
    Arc::new(AppState::new(
        AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        },
        client,
        ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
        None, // no on-chain posting
    ))
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

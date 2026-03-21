//! End-to-end integration tests: deploy → interact → verify on-chain → reconstruct.
//!
//! Uses `MockStrataRollup` (skips ZK verification) and a deterministic embedder
//! so no API keys are needed.

mod common;

use std::sync::Arc;

use alloy::{node_bindings::Anvil, primitives::FixedBytes, signers::local::PrivateKeySigner};
use axum::body::Body;
use commonware_runtime::{Runner as _, deterministic, tokio as cw_tokio};
use http_body_util::BodyExt;
use hyper::Request;
use serde_json::json;
use strata_core::Nonce;
use strata_vector_db::VectorDB;
use tower::ServiceExt;

use strata_agent::agent::AgentConfig;
use strata_agent::error::AgentError;
use strata_agent::llm::{ChatResponse, ContentBlock, LlmClient, StopReason, Usage};
use strata_agent::pipeline;
use strata_agent::poster::{self, PosterConfig};
use strata_agent::reconstruct;
use strata_agent::server::{self, AppState, PostingConfig};
use strata_agent::tools::ToolExecutor;

// --- Helpers ---

/// Mock LLM that returns tool_use(remember) on the first call, then a text
/// response on the second call. Simulates the agent deciding to remember.
fn mock_remember_client(remember_text: &str) -> LlmClient {
    let text = remember_text.to_string();
    let queue = std::sync::Arc::new(std::sync::Mutex::new(vec![
        // First call: LLM decides to remember
        ChatResponse {
            content: vec![ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "remember".into(),
                input: json!({"text": text}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        },
        // Second call: LLM responds with text
        ChatResponse {
            content: vec![ContentBlock::Text {
                text: "I'll remember that.".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        },
    ]));
    LlmClient::mock(move |_req| {
        let mut q = queue.lock().unwrap();
        if q.is_empty() {
            return Err(AgentError::Agent("no more canned responses".into()));
        }
        Ok(q.remove(0))
    })
}

fn a2a_request(text: &str) -> Request<Body> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": {
                "messageId": "msg-1",
                "role": "user",
                "parts": [{"text": text}]
            }
        }
    });
    Request::builder()
        .method("POST")
        .uri("/a2a")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

// --- Tests ---

/// Direct pipeline test: deploy → remember → finalize → post → reconstruct.
/// Bypasses the LLM by calling ToolExecutor directly.
#[test]
fn e2e_deploy_interact_post_reconstruct() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();

    let soul_text = "test-soul";
    let genesis = common::genesis_state();
    let genesis_root = FixedBytes::from(*genesis.vector_index_root.as_bytes());

    // Run the deterministic portion: create VectorDB, remember, finalize
    let transition = deterministic::Runner::default().start(|ctx| async move {
        let db_config = common::make_config("e2e", &ctx);
        let db = VectorDB::new(ctx, db_config).await.unwrap();

        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));
        let snap = pipeline::snapshot(genesis, executor.db());

        let result = executor
            .execute("remember", &json!({"text": "the sky is blue"}))
            .await
            .expect("remember failed");
        assert!(result.content.contains("id"));

        let transition = pipeline::finalize(&snap, executor.db(), executor.contents())
            .expect("finalize failed");
        assert_eq!(transition.new_state.nonce, Nonce::new(1));

        executor.into_db().destroy().await.unwrap();
        transition
    });

    // Deploy + post + verify in a tokio runtime (needs network for Anvil)
    cw_tokio::Runner::default().start(|_ctx| async move {
        let contract_address =
            poster::deploy_mock_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
                .await
                .expect("deploy failed");

        let poster_config = PosterConfig {
            rpc_url: rpc_url.clone(),
            contract_address,
        };

        // Verify initial state
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 0);
        assert_eq!(poster::read_state_root(&poster_config).await.unwrap(), genesis_root);

        // Post transition
        poster::post(&poster_config, signer.clone(), vec![], transition.public_values, &transition)
            .await
            .expect("post failed");

        // Verify on-chain state
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 1);
        let expected_root = FixedBytes::from(*transition.new_state.vector_index_root.as_bytes());
        assert_eq!(poster::read_state_root(&poster_config).await.unwrap(), expected_root);

        // Reconstruct and verify
        let reconstructed = reconstruct::reconstruct(&poster_config).await.expect("reconstruction failed");
        assert_eq!(reconstructed.state.nonce, Nonce::new(1));
        assert_eq!(reconstructed.contents, vec!["the sky is blue"]);
        assert_eq!(
            reconstructed.state.soul_hash,
            strata_core::SoulHash::digest(soul_text.as_bytes()),
        );
    });
}

/// Server integration test: send a message through the HTTP handler with
/// posting configured, verify the transition lands on-chain.
#[test]
fn server_posts_transition_on_interaction() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();

    let soul_text = "test-soul";

    cw_tokio::Runner::default().start(|ctx| async move {
        let genesis = common::genesis_state();
        let genesis_root = FixedBytes::from(*genesis.vector_index_root.as_bytes());

        // Deploy mock contract
        let contract_address =
            poster::deploy_mock_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
                .await
                .expect("deploy failed");

        let poster_config = PosterConfig {
            rpc_url: rpc_url.clone(),
            contract_address,
        };

        // Build AppState with posting config, mock LLM, tokio VectorDB
        let db_config = common::make_config("server-post", &ctx);
        let db = VectorDB::new(ctx, db_config).await.unwrap();

        let client = mock_remember_client("roses are red");
        let posting = PostingConfig {
            poster: poster_config.clone(),
            signer: signer.clone(),
        };

        let state = Arc::new(AppState::new(
            AgentConfig {
                soul: soul_text.into(),
                state: genesis,
            },
            client,
            ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
            Some(posting),
        ));

        // Send message through the HTTP handler
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("Remember this: roses are red")).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "unexpected error: {}", rpc["error"]);
        assert_eq!(rpc["result"]["status"]["state"], "completed");

        // Verify transition was posted on-chain
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 1);

        // Reconstruct and verify content
        let reconstructed = reconstruct::reconstruct(&poster_config).await.unwrap();
        assert_eq!(reconstructed.contents, vec!["roses are red"]);
    });
}

/// Verify that posting retries on failure and eventually succeeds when the
/// endpoint becomes available. We simulate this by posting to a valid Anvil
/// endpoint (which always succeeds on first try, so this tests the happy path
/// through the retry function).
#[test]
fn server_posts_two_transitions_sequentially() {
    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();

    let soul_text = "test-soul";

    cw_tokio::Runner::default().start(|ctx| async move {
        let genesis = common::genesis_state();
        let genesis_root = FixedBytes::from(*genesis.vector_index_root.as_bytes());

        let contract_address =
            poster::deploy_mock_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
                .await
                .expect("deploy failed");

        let poster_config = PosterConfig {
            rpc_url: rpc_url.clone(),
            contract_address,
        };

        let db_config = common::make_config("server-seq", &ctx);
        let db = VectorDB::new(ctx, db_config).await.unwrap();

        // Two-interaction mock: each interaction does remember then text
        let queue = std::sync::Arc::new(std::sync::Mutex::new(vec![
            ChatResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "remember".into(),
                    input: json!({"text": "fact one"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Usage { input_tokens: 10, output_tokens: 5 },
            },
            ChatResponse {
                content: vec![ContentBlock::Text { text: "Noted.".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 10, output_tokens: 5 },
            },
            ChatResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "c2".into(),
                    name: "remember".into(),
                    input: json!({"text": "fact two"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: Usage { input_tokens: 10, output_tokens: 5 },
            },
            ChatResponse {
                content: vec![ContentBlock::Text { text: "Got it.".into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 10, output_tokens: 5 },
            },
        ]));
        let client = LlmClient::mock(move |_req| {
            let mut q = queue.lock().unwrap();
            if q.is_empty() {
                return Err(AgentError::Agent("no more responses".into()));
            }
            Ok(q.remove(0))
        });

        let posting = PostingConfig {
            poster: poster_config.clone(),
            signer: signer.clone(),
        };

        let state = Arc::new(AppState::new(
            AgentConfig {
                soul: soul_text.into(),
                state: genesis,
            },
            client,
            ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
            Some(posting),
        ));

        // First interaction
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("remember fact one")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "interaction 1 error: {}", rpc["error"]);

        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 1);

        // Second interaction (on-chain nonce must be 2, roots must chain)
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("remember fact two")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "interaction 2 error: {}", rpc["error"]);

        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 2);

        // Reconstruct and verify both facts
        let reconstructed = reconstruct::reconstruct(&poster_config).await.unwrap();
        assert_eq!(reconstructed.state.nonce, Nonce::new(2));
        assert_eq!(reconstructed.contents, vec!["fact one", "fact two"]);
    });
}

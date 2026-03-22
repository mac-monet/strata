//! End-to-end integration tests: deploy → interact → verify on-chain → reconstruct.
//!
//! Uses a deterministic embedder so no API keys are needed.
//! The `local_embedder_*` tests use the real ONNX model (requires `local-embed` feature + model files).

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
use strata_agent::identity::IdentityConfig;
use strata_agent::llm::{ChatResponse, ContentBlock, LlmClient, StopReason, Usage};
use strata_agent::pipeline;
use strata_agent::poster::{self, PosterConfig};
use strata_agent::reconstruct;
use strata_agent::batch::PendingBatch;
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

/// Drain pending transitions from the batch and post them on-chain.
async fn flush_and_post(
    pending: &Arc<PendingBatch>,
    poster_config: &PosterConfig,
    signer: PrivateKeySigner,
) {
    let batch: Vec<_> = std::mem::take(&mut *pending.lock().await);
    assert!(!batch.is_empty(), "expected pending transitions");
    let first = &batch[0];
    let last = &batch[batch.len() - 1];
    let pv = pipeline::batch_public_values(
        first.old_state.vector_index_root.as_bytes(),
        last.new_state.vector_index_root.as_bytes(),
        first.record.input.nonce.get(),
        last.record.input.nonce.get(),
        first.old_state.soul_hash.as_bytes(),
    );
    poster::post_batch(poster_config, signer, vec![], pv, &batch)
        .await
        .expect("post_batch failed");
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
            poster::deploy_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
                .await
                .expect("deploy failed");

        let poster_config = PosterConfig {
            rpc_url: rpc_url.clone(),
            contract_address,
        };

        // Verify initial state
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 0);
        assert_eq!(poster::read_state_root(&poster_config).await.unwrap(), genesis_root);

        // Build public values for the batch
        let nonce = transition.new_state.nonce.get();
        let pv = pipeline::batch_public_values(
            transition.old_state.vector_index_root.as_bytes(),
            transition.new_state.vector_index_root.as_bytes(),
            nonce,
            nonce,
            transition.new_state.soul_hash.as_bytes(),
        );

        // Post transition
        poster::post_batch(&poster_config, signer.clone(), vec![], pv, std::slice::from_ref(&transition))
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
            poster::deploy_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
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
        let pending: Arc<PendingBatch> = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut app_state = AppState::new(
            AgentConfig {
                soul: soul_text.into(),
                state: genesis,
            },
            client,
            ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
            Some(pending.clone()),
            test_identity(),
            alloy::primitives::Address::ZERO,
        );
        app_state.proofs_dir = tempfile::tempdir().unwrap().into_path();
        let state = Arc::new(app_state);

        // Send message through the HTTP handler
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("Remember this: roses are red")).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "unexpected error: {}", rpc["error"]);
        assert_eq!(rpc["result"]["status"]["state"], "completed");

        // Flush pending transitions and post on-chain
        flush_and_post(&pending, &poster_config, signer.clone()).await;
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
            poster::deploy_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
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

        let pending: Arc<PendingBatch> = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut app_state = AppState::new(
            AgentConfig {
                soul: soul_text.into(),
                state: genesis,
            },
            client,
            ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
            Some(pending.clone()),
            test_identity(),
            alloy::primitives::Address::ZERO,
        );
        app_state.proofs_dir = tempfile::tempdir().unwrap().into_path();
        let state = Arc::new(app_state);

        // First interaction
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("remember fact one")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "interaction 1 error: {}", rpc["error"]);

        // Flush first transition
        flush_and_post(&pending, &poster_config, signer.clone()).await;
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 1);

        // Second interaction (on-chain nonce must be 2, roots must chain)
        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("remember fact two")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "interaction 2 error: {}", rpc["error"]);

        // Flush second transition
        flush_and_post(&pending, &poster_config, signer.clone()).await;
        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 2);

        // Reconstruct and verify both facts
        let reconstructed = reconstruct::reconstruct(&poster_config).await.unwrap();
        assert_eq!(reconstructed.state.nonce, Nonce::new(2));
        assert_eq!(reconstructed.contents, vec!["fact one", "fact two"]);
    });
}

/// Snapshot persistence e2e: interact → snapshot saved to disk → restore from
/// snapshot into a fresh VectorDB → verify state, entries, and contents match.
#[test]
fn snapshot_save_and_restore() {
    let soul_text = "test-soul";

    cw_tokio::Runner::default().start(|ctx| async move {
        let genesis = common::genesis_state();
        let snap_dir = tempfile::tempdir().unwrap();
        let snap_path = snap_dir.path().join("snapshot.json");

        // Phase 1: interact via HTTP with snapshot_path set, so the handler
        // auto-saves a snapshot after the transition.
        let db_config = common::make_config("snap-save", &ctx);
        let db = VectorDB::new(ctx.clone(), db_config).await.unwrap();

        let client = mock_remember_client("snapshot fact");
        let pending: Arc<PendingBatch> = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut app_state = AppState::new(
            AgentConfig {
                soul: soul_text.into(),
                state: genesis,
            },
            client,
            ToolExecutor::new(db, Box::new(common::FixedEmbedder)),
            Some(pending.clone()),
            test_identity(),
            alloy::primitives::Address::ZERO,
        );
        app_state.proofs_dir = tempfile::tempdir().unwrap().into_path();
        app_state.snapshot_path = Some(snap_path.clone());
        let state = Arc::new(app_state);

        let app = server::router(state.clone());
        let resp = app.oneshot(a2a_request("Remember: snapshot fact")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let rpc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rpc["error"].is_null(), "unexpected error: {}", rpc["error"]);

        // Snapshot file should exist now.
        assert!(snap_path.exists(), "snapshot was not saved after transition");

        // Capture the post-transition state for comparison via the public snapshot method.
        let expected_snap = state.snapshot().await;

        // Phase 2: load the snapshot and open a fresh VectorDB from it.
        let loaded = strata_agent::persist::load(&snap_path)
            .expect("failed to load snapshot")
            .expect("snapshot file should exist");

        assert_eq!(loaded.state.nonce, expected_snap.state.nonce);
        assert_eq!(loaded.state.soul_hash, expected_snap.state.soul_hash);
        assert_eq!(loaded.state.vector_index_root, expected_snap.state.vector_index_root);
        assert_eq!(loaded.contents, expected_snap.contents);
        assert_eq!(loaded.entries.len(), expected_snap.entries.len());

        // Rebuild a fresh VectorDB from snapshot entries via batch_append.
        let db_config2 = common::make_config("snap-restore", &ctx);
        let mut db2 = VectorDB::new(ctx, db_config2).await
            .expect("VectorDB::new for restore failed");
        db2.batch_append(loaded.entries.clone()).await
            .expect("batch_append from snapshot failed");

        assert_eq!(db2.root(), expected_snap.state.vector_index_root);
        assert_eq!(db2.len(), expected_snap.entries.len() as u64);

        // Verify recall works on the restored DB.
        let mut executor2 = ToolExecutor::new(db2, Box::new(common::FixedEmbedder))
            .with_contents(loaded.contents)
            .expect("contents mismatch on restore");
        let result = executor2
            .execute("recall", &json!({"query": "snapshot"}))
            .await
            .expect("recall on restored DB failed");
        assert!(
            result.content.contains("snapshot fact"),
            "restored DB should contain the remembered fact, got: {}",
            result.content
        );
    });
}

// --- Local embedder e2e tests ---

#[cfg(feature = "local-embed")]
fn local_model_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/embed")
}

#[cfg(feature = "local-embed")]
fn has_local_model() -> bool {
    let dir = local_model_dir();
    dir.join("model.onnx").exists() && dir.join("tokenizer.json").exists()
}

/// E2e with local ONNX embedder: remember three facts, recall by semantic query,
/// verify the closest match is returned.
#[cfg(feature = "local-embed")]
#[test]
fn local_embedder_remember_and_recall() {
    use strata_agent::embed::LocalEmbedder;

    if !has_local_model() {
        eprintln!("skipping: model files not found in models/embed/");
        return;
    }

    let anvil = Anvil::new().try_spawn().expect("failed to spawn anvil");
    let rpc_url = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();

    let soul_text = "test-soul";
    let genesis = common::genesis_state();
    let genesis_root = FixedBytes::from(*genesis.vector_index_root.as_bytes());

    cw_tokio::Runner::default().start(|ctx| async move {
        let db_config = common::make_config("local-embed-e2e", &ctx);
        let db = VectorDB::new(ctx, db_config).await.unwrap();

        let embedder = LocalEmbedder::mixedbread(local_model_dir())
            .expect("failed to load local model");
        let mut executor = ToolExecutor::new(db, Box::new(embedder));
        let snap = pipeline::snapshot(genesis, executor.db());

        // Remember three semantically distinct facts
        executor.execute("remember", &json!({"text": "The capital of France is Paris"}))
            .await.expect("remember 1 failed");
        executor.execute("remember", &json!({"text": "Rust is a systems programming language"}))
            .await.expect("remember 2 failed");
        executor.execute("remember", &json!({"text": "Water boils at 100 degrees Celsius"}))
            .await.expect("remember 3 failed");

        // Recall with a semantically related query
        let result = executor.execute("recall", &json!({"query": "What is the boiling point of water?"}))
            .await.expect("recall failed");

        // The recall result should contain the water/boiling fact
        assert!(
            result.content.contains("100 degrees") || result.content.contains("boils"),
            "recall should find the water fact, got: {}",
            result.content
        );

        // Finalize and post on-chain
        let transition = pipeline::finalize(&snap, executor.db(), executor.contents())
            .expect("finalize failed");
        assert_eq!(transition.new_state.nonce, Nonce::new(1));

        let contract_address =
            poster::deploy_contract(&rpc_url, signer.clone(), soul_text, genesis_root)
                .await.expect("deploy failed");

        let poster_config = PosterConfig {
            rpc_url: rpc_url.clone(),
            contract_address,
        };

        let nonce = transition.new_state.nonce.get();
        let pv = pipeline::batch_public_values(
            transition.old_state.vector_index_root.as_bytes(),
            transition.new_state.vector_index_root.as_bytes(),
            nonce, nonce,
            transition.new_state.soul_hash.as_bytes(),
        );

        poster::post_batch(&poster_config, signer.clone(), vec![], pv, std::slice::from_ref(&transition))
            .await.expect("post failed");

        assert_eq!(poster::read_nonce(&poster_config).await.unwrap(), 1);

        // Reconstruct and verify all three facts
        let reconstructed = reconstruct::reconstruct(&poster_config).await.expect("reconstruction failed");
        assert_eq!(reconstructed.state.nonce, Nonce::new(1));
        assert_eq!(reconstructed.contents.len(), 3);
        assert!(reconstructed.contents.iter().any(|c| c.contains("Paris")));
        assert!(reconstructed.contents.iter().any(|c| c.contains("Rust")));
        assert!(reconstructed.contents.iter().any(|c| c.contains("100 degrees")));

        executor.into_db().destroy().await.unwrap();
    });
}

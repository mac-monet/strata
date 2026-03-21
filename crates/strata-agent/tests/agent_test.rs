mod common;

use commonware_runtime::{deterministic, Runner as _};
use serde_json::json;
use std::sync::{Arc, Mutex};
use strata_core::Nonce;
use strata_vector_db::VectorDB;

use strata_agent::agent::{self, AgentConfig};
use strata_agent::error::AgentError;
use strata_agent::llm::{self, ChatRequest, ChatResponse, ContentBlock, LlmClient, StopReason, Usage};
use strata_agent::tools::ToolExecutor;

// --- Helpers ---

/// Build a mock LlmClient that returns canned responses in sequence.
fn mock_client(responses: Vec<ChatResponse>) -> LlmClient {
    let queue = Arc::new(Mutex::new(responses));
    LlmClient::mock(move |_req| {
        let mut q = queue.lock().unwrap();
        if q.is_empty() {
            return Err(AgentError::Agent("no more canned responses".into()));
        }
        Ok(q.remove(0))
    })
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
    }
}

fn tool_use_response(id: &str, name: &str, input: serde_json::Value) -> ChatResponse {
    ChatResponse {
        content: vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
    }
}

fn max_tokens_response() -> ChatResponse {
    ChatResponse {
        content: vec![ContentBlock::Text {
            text: "partial...".into(),
        }],
        stop_reason: StopReason::MaxTokens,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
    }
}

// --- Tests ---

#[test]
fn text_only_response() {
    let client = mock_client(vec![text_response("Hello, world!")]);

    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("text-only", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let mut agent_config = AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        };

        let messages = vec![llm::Message::user_text("Hi")];
        let result = agent::interact(&mut agent_config, &client, &mut executor, &messages)
            .await
            .unwrap();

        assert_eq!(result.response, "Hello, world!");
        assert!(result.transition.is_none());
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn remember_then_respond() {
    let client = mock_client(vec![
        tool_use_response("call_1", "remember", json!({"text": "the sky is blue"})),
        text_response("I remembered that."),
    ]);

    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("remember", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let mut agent_config = AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        };

        let messages = vec![llm::Message::user_text("Remember the sky is blue")];
        let result = agent::interact(&mut agent_config, &client, &mut executor, &messages)
            .await
            .unwrap();

        assert_eq!(result.response, "I remembered that.");
        assert!(result.transition.is_some());

        let transition = result.transition.unwrap();
        assert_eq!(transition.record.new_entries.len(), 1);
        assert_eq!(transition.new_state.nonce, Nonce::new(1));
        // Usage accumulates across both LLM calls
        assert_eq!(result.usage.input_tokens, 20);
        assert_eq!(result.usage.output_tokens, 10);

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn auto_recall_injects_memory_context() {
    // Mock client that captures the system prompt and verifies it contains memory context.
    let captured_system = Arc::new(Mutex::new(String::new()));
    let captured = captured_system.clone();
    let client = LlmClient::mock(move |req: &ChatRequest| {
        if let Some(sys) = &req.system {
            *captured.lock().unwrap() = sys.clone();
        }
        Ok(text_response("I know about the sky."))
    });

    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("auto-recall", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        // Pre-populate a memory via the remember tool
        let result = executor
            .execute("remember", &json!({"text": "the sky is blue"}))
            .await
            .unwrap();
        assert!(!result.is_error);

        let mut agent_config = AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        };

        let messages = vec![llm::Message::user_text("What color is the sky?")];
        let result = agent::interact(&mut agent_config, &client, &mut executor, &messages)
            .await
            .unwrap();

        assert_eq!(result.response, "I know about the sky.");

        // Verify the system prompt contained the auto-recalled memory
        {
            let system = captured_system.lock().unwrap();
            assert!(
                system.contains("the sky is blue"),
                "system prompt should contain recalled memory, got: {system}"
            );
            assert!(
                system.contains("Relevant memories"),
                "system prompt should contain memory header, got: {system}"
            );
        }

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn auto_recall_with_multi_turn_messages() {
    // Verify auto_recall uses the last user message for embedding,
    // and that recalled memory appears in the system prompt.
    let captured_system = Arc::new(Mutex::new(String::new()));
    let captured = captured_system.clone();
    let client = LlmClient::mock(move |req: &ChatRequest| {
        if let Some(sys) = &req.system {
            *captured.lock().unwrap() = sys.clone();
        }
        Ok(text_response("I recall both facts."))
    });

    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("multi-turn", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        // Store two distinct memories
        executor
            .execute("remember", &json!({"text": "apples are red"}))
            .await
            .unwrap();
        executor
            .execute("remember", &json!({"text": "bananas are yellow"}))
            .await
            .unwrap();

        let mut agent_config = AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        };

        // Multi-turn: the last user message should drive recall
        let messages = vec![
            llm::Message::user_text("Tell me about fruit"),
            llm::Message {
                role: llm::Role::Assistant,
                content: vec![llm::ContentBlock::Text {
                    text: "Sure, what would you like to know?".into(),
                }],
            },
            llm::Message::user_text("What color are bananas?"),
        ];
        let result = agent::interact(&mut agent_config, &client, &mut executor, &messages)
            .await
            .unwrap();

        assert_eq!(result.response, "I recall both facts.");

        let system = captured_system.lock().unwrap();
        assert!(
            system.contains("Relevant memories"),
            "system prompt should contain memory header, got: {system}"
        );

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn max_tokens_error() {
    let client = mock_client(vec![max_tokens_response()]);

    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("max-tokens", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let mut agent_config = AgentConfig {
            soul: "You are a test agent.".into(),
            state: common::genesis_state(),
        };

        let messages = vec![llm::Message::user_text("Generate a long response")];
        let result = agent::interact(&mut agent_config, &client, &mut executor, &messages).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("max tokens"), "got: {err}");

        executor.into_db().destroy().await.unwrap();
    });
}

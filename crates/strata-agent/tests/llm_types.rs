//! Tests for LLM canonical types and provider wire format conversion.

use serde_json::json;
use strata_agent::llm::*;

// --- Canonical type tests ---

#[test]
fn chat_request_builds_correctly() {
    let mut req = ChatRequest::new().system("You are a helpful agent.");
    req.push_user("Hello");

    assert_eq!(req.system.as_deref(), Some("You are a helpful agent."));
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, Role::User);
}

#[test]
fn tool_definition_serializes() {
    let tool = Tool {
        name: "remember".into(),
        description: "Store a memory".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"]
        }),
    };

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "remember");
    assert_eq!(json["input_schema"]["type"], "object");
}

#[test]
fn chat_request_push_tool_result() {
    let mut req = ChatRequest::new();
    req.push_user("Remember this");
    req.push_assistant(vec![ContentBlock::ToolUse {
        id: "toolu_1".into(),
        name: "remember".into(),
        input: json!({"text": "important fact"}),
    }]);
    req.push_tool_result("toolu_1".into(), "Stored".into(), false);

    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[2].role, Role::User);
}

#[test]
fn multiple_tool_results_batch_into_one_message() {
    let mut req = ChatRequest::new();
    req.push_assistant(vec![
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "recall".into(),
            input: json!({}),
        },
        ContentBlock::ToolUse {
            id: "t2".into(),
            name: "remember".into(),
            input: json!({"text": "hi"}),
        },
    ]);
    // Push two tool results — they should batch into one User message.
    req.push_tool_result("t1".into(), "result 1".into(), false);
    req.push_tool_result("t2".into(), "result 2".into(), false);

    // Should be 2 messages total (assistant + one batched user), not 3.
    assert_eq!(req.messages.len(), 2);
    assert_eq!(req.messages[1].role, Role::User);
    assert_eq!(req.messages[1].content.len(), 2);
}

#[test]
fn tool_result_after_user_text_creates_new_message() {
    let mut req = ChatRequest::new();
    req.push_user("Hello");
    req.push_tool_result("t1".into(), "result".into(), false);

    // Should NOT batch into the user text message.
    assert_eq!(req.messages.len(), 2);
}

// --- Anthropic provider tests ---

mod anthropic_tests {
    use super::*;

    fn parse(raw: serde_json::Value) -> ChatResponse {
        let provider = Anthropic::new("fake-key".into());
        test_parse_response(&provider, raw).unwrap()
    }

    fn build(req: &ChatRequest) -> serde_json::Value {
        let provider = Anthropic::new("fake-key".into());
        test_build_body(&provider, req, "claude-test", 1024)
    }

    #[test]
    fn parses_text_response() {
        let resp = parse(json!({
            "content": [{ "type": "text", "text": "Hello!" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        }));

        assert_eq!(resp.text(), "Hello!");
        assert!(!resp.has_tool_calls());
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn parses_tool_use() {
        let resp = parse(json!({
            "content": [
                { "type": "text", "text": "I'll remember that." },
                {
                    "type": "tool_use",
                    "id": "toolu_abc",
                    "name": "remember",
                    "input": { "text": "The user likes Rust" }
                }
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 20, "output_tokens": 15 }
        }));

        assert!(resp.has_tool_calls());
        let calls = resp.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_abc");
        assert_eq!(calls[0].name, "remember");
        assert_eq!(calls[0].input["text"], "The user likes Rust");
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn build_body_system_is_top_level() {
        let req = ChatRequest::new().system("Be helpful.");
        let body = build(&req);

        assert_eq!(body["system"], "Be helpful.");
        // Should NOT be in messages.
        let msgs = body["messages"].as_array().unwrap();
        assert!(msgs.iter().all(|m| m["role"] != "system"));
    }

    #[test]
    fn build_body_tool_result_includes_is_error() {
        let mut req = ChatRequest::new();
        req.push_tool_result("toolu_1".into(), "something broke".into(), true);
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        let tool_result = &msgs[0]["content"][0];
        assert_eq!(tool_result["is_error"], true);
    }

    #[test]
    fn build_body_tool_result_omits_is_error_when_false() {
        let mut req = ChatRequest::new();
        req.push_tool_result("toolu_1".into(), "ok".into(), false);
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        let tool_result = &msgs[0]["content"][0];
        assert!(tool_result.get("is_error").is_none());
    }
}

// --- OpenAI-compatible provider tests ---

mod openai_tests {
    use super::*;

    fn parse(raw: serde_json::Value) -> ChatResponse {
        let provider = OpenAi::new("fake-key".into());
        test_parse_response(&provider, raw).unwrap()
    }

    fn build(req: &ChatRequest) -> serde_json::Value {
        let provider = OpenAi::new("fake-key".into());
        test_build_body(&provider, req, "gpt-4o-test", 1024)
    }

    #[test]
    fn parses_text_response() {
        let resp = parse(json!({
            "choices": [{
                "message": { "role": "assistant", "content": "Hello!" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }));

        assert_eq!(resp.text(), "Hello!");
        assert!(!resp.has_tool_calls());
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn parses_tool_calls() {
        let resp = parse(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "remember",
                            "arguments": "{\"text\":\"important\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 15 }
        }));

        assert!(resp.has_tool_calls());
        let calls = resp.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_123");
        assert_eq!(calls[0].name, "remember");
        assert_eq!(calls[0].input["text"], "important");
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn stop_reason_from_tool_presence_not_finish_reason() {
        // OpenRouter sometimes returns "stop" even when tool calls are present.
        let resp = parse(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "recall", "arguments": "{}" }
                    }]
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 5 }
        }));

        // Should detect ToolUse from the presence of tool calls, not finish_reason.
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn malformed_arguments_returns_error() {
        let provider = OpenAi::new("fake-key".into());
        let result = test_parse_response(
            &provider,
            json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": { "name": "recall", "arguments": "not json{{" }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 5, "completion_tokens": 5 }
            }),
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("malformed tool arguments"));
    }

    #[test]
    fn build_body_system_is_role_message() {
        let req = ChatRequest::new().system("Be helpful.");
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "Be helpful.");
    }

    #[test]
    fn build_body_tool_result_is_role_tool() {
        let mut req = ChatRequest::new();
        req.push_tool_result("call_1".into(), "result data".into(), false);
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "call_1");
        assert_eq!(msgs[0]["content"], "result data");
    }

    #[test]
    fn build_body_tool_result_error_prepends_marker() {
        let mut req = ChatRequest::new();
        req.push_tool_result("call_1".into(), "something broke".into(), true);
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["content"], "[ERROR] something broke");
    }

    #[test]
    fn build_body_assistant_tool_use_has_null_content() {
        let mut req = ChatRequest::new();
        req.push_assistant(vec![ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "remember".into(),
            input: json!({"text": "hi"}),
        }]);
        let body = build(&req);

        let msgs = body["messages"].as_array().unwrap();
        let assistant_msg = &msgs[0];
        assert!(assistant_msg["content"].is_null());
        assert!(assistant_msg["tool_calls"].is_array());

        // Arguments should be a stringified JSON, not a nested object.
        let args = assistant_msg["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(args, r#"{"text":"hi"}"#);
    }

    #[test]
    fn openrouter_url_construction() {
        let provider = OpenAi::with_base_url(
            "key".into(),
            "https://openrouter.ai/api/v1".into(),
        );
        let endpoint = Provider::endpoint(&provider);
        assert!(endpoint.ends_with("/chat/completions"));
        assert!(endpoint.contains("openrouter.ai"));
    }

    #[test]
    fn trailing_slash_stripped() {
        let provider = OpenAi::with_base_url(
            "key".into(),
            "https://openrouter.ai/api/v1/".into(),
        );
        let endpoint = Provider::endpoint(&provider);
        assert!(!endpoint.contains("//chat"));
    }
}

mod common;

use commonware_runtime::{deterministic, Runner as _};
use serde_json::json;
use strata_vector_db::VectorDB;

use strata_agent::tools::{self, ToolExecutor};

#[test]
fn definitions_returns_three_tools() {
    let defs = tools::definitions();
    assert_eq!(defs.len(), 3);
    assert_eq!(defs[0].name, "recall");
    assert_eq!(defs[1].name, "remember");
    assert_eq!(defs[2].name, "bash");
}

#[test]
fn remember_and_recall_round_trip() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("round-trip", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        // Remember something
        let result = executor
            .execute("remember", &json!({"text": "the sky is blue"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("\"id\":0"));

        // Recall it
        let result = executor
            .execute("recall", &json!({"query": "sky color"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("the sky is blue"));

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn recall_empty_db() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("empty-recall", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let result = executor
            .execute("recall", &json!({"query": "anything"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "No memories found.");

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn remember_increments_ids() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("incr-ids", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let r0 = executor
            .execute("remember", &json!({"text": "first"}))
            .await
            .unwrap();
        assert!(r0.content.contains("\"id\":0"));

        let r1 = executor
            .execute("remember", &json!({"text": "second"}))
            .await
            .unwrap();
        assert!(r1.content.contains("\"id\":1"));

        assert_eq!(executor.contents().len(), 2);
        assert_eq!(executor.contents()[0], "first");
        assert_eq!(executor.contents()[1], "second");

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn unknown_tool_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("unknown", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let result = executor
            .execute("nonexistent", &json!({}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("unknown tool"));

        executor.into_db().destroy().await.unwrap();
    });
}

#[test]
fn missing_params_returns_parse_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("bad-params", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let mut executor = ToolExecutor::new(db, Box::new(common::FixedEmbedder));

        let result = executor.execute("recall", &json!({})).await;
        assert!(result.is_err());

        let result = executor.execute("remember", &json!({})).await;
        assert!(result.is_err());

        let result = executor.execute("bash", &json!({})).await;
        assert!(result.is_err());

        executor.into_db().destroy().await.unwrap();
    });
}

// Bash tests run under tokio because tokio::process::Command requires a reactor.
#[tokio::test]
async fn bash_echo() {
    let result = tools::execute_bash("echo hello", std::time::Duration::from_secs(5), 64 * 1024)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content.trim(), "hello");
}

#[tokio::test]
async fn bash_nonzero_exit() {
    let result = tools::execute_bash("exit 42", std::time::Duration::from_secs(5), 64 * 1024)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("exit code: 42"));
}

#[tokio::test]
async fn bash_captures_stderr() {
    let result =
        tools::execute_bash("echo oops >&2", std::time::Duration::from_secs(5), 64 * 1024)
            .await
            .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("[stderr]"));
    assert!(result.content.contains("oops"));
}

#[tokio::test]
async fn bash_timeout() {
    let result =
        tools::execute_bash("sleep 10", std::time::Duration::from_millis(100), 64 * 1024)
            .await
            .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
}

//! Agent tool dispatch: recall, remember, bash.
//!
//! `ToolExecutor` bridges LLM tool calls to their implementations. It holds the
//! VectorDB (for recall/remember) and an Embedder (for generating binary embeddings
//! from text). Bash commands run in a child process with a configurable timeout.

use std::time::Duration;

use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use serde_json::json;
use strata_core::{ContentHash, MemoryEntry, MemoryId};
use strata_vector_db::VectorDB;
use tokio::process::Command;

use crate::embed::Embedder;
use crate::error::AgentError;
use crate::llm;

const DEFAULT_BASH_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024; // 64 KiB
const DEFAULT_RECALL_K: usize = 5;

/// Tool definitions for the LLM function calling interface.
pub fn definitions() -> Vec<llm::Tool> {
    vec![
        llm::Tool {
            name: "recall".into(),
            description: "Search memory for relevant entries. Returns the most similar memories."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    }
                },
                "required": ["query"]
            }),
        },
        llm::Tool {
            name: "remember".into(),
            description: "Store a new memory entry. Returns the memory ID.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text to remember"
                    }
                },
                "required": ["text"]
            }),
        },
        llm::Tool {
            name: "bash".into(),
            description: "Execute a shell command and return its output.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        },
    ]
}

/// Result of executing a tool.
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(content: String) -> Self {
        Self {
            content,
            is_error: false,
        }
    }

    fn err(content: String) -> Self {
        Self {
            content,
            is_error: true,
        }
    }
}

/// Executes agent tools against shared state (VectorDB + Embedder).
pub struct ToolExecutor<E: RStorage + Clock + Metrics> {
    db: VectorDB<E>,
    embedder: Box<dyn Embedder>,
    contents: Vec<String>,
    bash_timeout: Duration,
    max_output_bytes: usize,
}

impl<E: RStorage + Clock + Metrics> ToolExecutor<E> {
    pub fn new(db: VectorDB<E>, embedder: Box<dyn Embedder>) -> Self {
        Self {
            db,
            embedder,
            contents: Vec::new(),
            bash_timeout: DEFAULT_BASH_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    pub fn with_bash_timeout(mut self, timeout: Duration) -> Self {
        self.bash_timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max: usize) -> Self {
        self.max_output_bytes = max;
        self
    }

    /// Provide existing content texts (e.g. from reconstruction replay).
    /// Returns an error if the length doesn't match the DB's entry count.
    pub fn with_contents(mut self, contents: Vec<String>) -> Result<Self, AgentError> {
        if contents.len() as u64 != self.db.len() {
            return Err(AgentError::Tool(format!(
                "contents length {} != db length {}",
                contents.len(),
                self.db.len()
            )));
        }
        self.contents = contents;
        Ok(self)
    }

    /// Execute a tool call by name with the given input JSON.
    pub async fn execute(
        &mut self,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<ToolOutput, AgentError> {
        match name {
            "recall" => self.recall(input),
            "remember" => self.remember(input).await,
            "bash" => self.bash(input).await,
            _ => Ok(ToolOutput::err(format!("unknown tool: {name}"))),
        }
    }

    fn recall(&self, input: &serde_json::Value) -> Result<ToolOutput, AgentError> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Parse("recall requires a 'query' string".into()))?;

        let embedding = self.embedder.embed(query)?;
        let results = self.db.query(&embedding, DEFAULT_RECALL_K);

        if results.is_empty() {
            return Ok(ToolOutput::ok("No memories found.".into()));
        }

        let entries: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let id = r.entry.id.get() as usize;
                let text = self
                    .contents
                    .get(id)
                    .map(|s| s.as_str())
                    .unwrap_or("[content unavailable]");
                json!({
                    "id": r.entry.id.get(),
                    "text": text,
                    "distance": r.distance,
                })
            })
            .collect();

        Ok(ToolOutput::ok(serde_json::to_string(&entries).unwrap()))
    }

    async fn remember(&mut self, input: &serde_json::Value) -> Result<ToolOutput, AgentError> {
        let text = input
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Parse("remember requires a 'text' string".into()))?;

        let embedding = self.embedder.embed(text)?;
        let content_hash = ContentHash::digest(text.as_bytes());
        let id = MemoryId::new(self.db.len());
        let entry = MemoryEntry::new(id, embedding, content_hash);

        self.db
            .append(entry)
            .await
            .map_err(|e| AgentError::Tool(format!("failed to append memory: {e}")))?;

        self.contents.push(text.to_owned());

        Ok(ToolOutput::ok(json!({"id": id.get()}).to_string()))
    }

    async fn bash(&self, input: &serde_json::Value) -> Result<ToolOutput, AgentError> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Parse("bash requires a 'command' string".into()))?;

        execute_bash(command, self.bash_timeout, self.max_output_bytes).await
    }

    /// Access the underlying VectorDB.
    pub fn db(&self) -> &VectorDB<E> {
        &self.db
    }

    /// Access the underlying VectorDB mutably.
    pub fn db_mut(&mut self) -> &mut VectorDB<E> {
        &mut self.db
    }

    /// Access stored content texts.
    pub fn contents(&self) -> &[String] {
        &self.contents
    }

    /// Consume the executor and return the underlying VectorDB.
    pub fn into_db(self) -> VectorDB<E> {
        self.db
    }
}

/// Execute a shell command with timeout and output size limits.
pub async fn execute_bash(
    command: &str,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<ToolOutput, AgentError> {
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => return Ok(ToolOutput::err(format!("failed to execute command: {e}"))),
    };

    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut out = String::new();
            if !stdout.is_empty() {
                out.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("[stderr]\n");
                out.push_str(&stderr);
            }

            if out.len() > max_output_bytes {
                let boundary = out.floor_char_boundary(max_output_bytes);
                out.truncate(boundary);
                out.push_str("\n[output truncated]");
            }

            if output.status.success() {
                Ok(ToolOutput::ok(out))
            } else {
                let code = output.status.code().unwrap_or(-1);
                if out.is_empty() {
                    Ok(ToolOutput::err(format!("exit code: {code}")))
                } else {
                    Ok(ToolOutput::err(format!("{out}\nexit code: {code}")))
                }
            }
        }
        Ok(Err(e)) => Ok(ToolOutput::err(format!(
            "failed to execute command: {e}"
        ))),
        Err(_) => Ok(ToolOutput::err(format!(
            "command timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

//! Agent loop orchestration: LLM conversation with tool execution and state transitions.

use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use strata_core::CoreState;

use crate::error::AgentError;
use crate::llm::{self, ChatRequest, LlmClient, StopReason};
use crate::pipeline::{self, TransitionOutput};
use crate::tools::{self, ToolExecutor};

/// Maximum number of tool-use rounds before aborting.
const MAX_TOOL_ROUNDS: usize = 32;

/// Configuration for an agent instance.
pub struct AgentConfig {
    /// Soul document text (becomes system prompt).
    pub soul: String,
    /// Current state (updated after each transition).
    pub state: CoreState,
}

/// Result of running one interaction through the agent loop.
#[derive(Debug)]
pub struct InteractionResult {
    /// Agent's final text response.
    pub response: String,
    /// Present if `remember()` was called during the interaction.
    pub transition: Option<TransitionOutput>,
    /// Token usage for the interaction.
    pub usage: llm::Usage,
    /// Messages generated during this interaction (assistant + tool results).
    /// The caller can append these to a session history for multi-turn conversations.
    pub trail: Vec<llm::Message>,
}

/// Run the agent loop: LLM conversation with tool execution.
///
/// Takes a slice of messages (for multi-turn extensibility).
/// The soul document is prepended as the system prompt.
/// On a successful transition, `config.state` is updated to the new state.
pub async fn interact<E: RStorage + Clock + Metrics>(
    config: &mut AgentConfig,
    client: &LlmClient,
    executor: &mut ToolExecutor<E>,
    messages: &[llm::Message],
) -> Result<InteractionResult, AgentError> {
    let snap = pipeline::snapshot(config.state, executor.db());

    // Auto-recall: extract text from the last user message, query memory,
    // and build a system prompt that includes relevant context.
    let user_text = messages
        .iter()
        .rev()
        .find(|m| m.role == llm::Role::User)
        .map(|m| m.text());

    let memory_context = match user_text {
        Some(ref text) if !text.is_empty() => executor.auto_recall(text).await?,
        _ => None,
    };

    let system = match &memory_context {
        Some(ctx) => format!("{}\n\n{ctx}", config.soul),
        None => config.soul.clone(),
    };

    let mut request = ChatRequest::new()
        .system(&system)
        .tools(tools::definitions());

    for msg in messages {
        request.messages.push(msg.clone());
    }

    let input_len = request.messages.len();
    let mut total_usage = llm::Usage::default();
    let mut rounds = 0usize;

    loop {
        let response = client.send(&request).await?;
        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;

        let stop_reason = response.stop_reason;

        match stop_reason {
            StopReason::EndTurn => {
                let text = response.text();
                request.push_assistant(response.content);
                let transition = if executor.db().len() > snap.leaf_count {
                    let t = pipeline::finalize(&snap, executor.db(), executor.contents())?;
                    config.state = t.new_state;
                    Some(t)
                } else {
                    None
                };
                let trail = request.messages.split_off(input_len);
                return Ok(InteractionResult {
                    response: text,
                    transition,
                    usage: total_usage,
                    trail,
                });
            }
            StopReason::ToolUse => {
                rounds += 1;
                if rounds > MAX_TOOL_ROUNDS {
                    return Err(AgentError::Agent(format!(
                        "exceeded max tool rounds ({MAX_TOOL_ROUNDS})"
                    )));
                }
                let tool_calls: Vec<_> = response
                    .tool_calls()
                    .iter()
                    .map(|c| (c.id.to_owned(), c.name.to_owned(), c.input.clone()))
                    .collect();
                request.push_assistant(response.content);
                for (id, name, input) in &tool_calls {
                    let (content, is_error) =
                        match executor.execute(name, input).await {
                            Ok(output) => (output.content, output.is_error),
                            Err(e) => (e.to_string(), true),
                        };
                    request.push_tool_result(id.clone(), content, is_error);
                }
            }
            StopReason::MaxTokens => {
                return Err(AgentError::Agent("max tokens reached".into()));
            }
        }
    }
}

//! Agent error types.

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("HTTP request failed: {0}")]
    Http(reqwest::Error),

    #[error("API error (status {status}): {body}")]
    Api { status: u16, body: String },

    #[error("failed to parse response: {0}")]
    Parse(String),

    #[error("embedding error: {0}")]
    Embed(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("pipeline error: {0}")]
    Pipeline(String),

    #[error("prover error: {0}")]
    Prover(String),

    #[error("poster error: {0}")]
    Poster(String),
}

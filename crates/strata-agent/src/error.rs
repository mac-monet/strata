//! Agent error types.

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("HTTP request failed: {0}")]
    Http(reqwest::Error),

    #[error("API error (status {status}): {body}")]
    Api { status: u16, body: String },

    #[error("failed to parse response: {0}")]
    Parse(String),
}

//! Embedding client: float vector → binary embedding conversion.
//!
//! Calls an OpenAI-compatible embedding API, then binarizes the resulting
//! float vector by thresholding each dimension at the median.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use strata_core::{BinaryEmbedding, EMBEDDING_WORDS};

use crate::error::AgentError;

const OPENAI_EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
const DEFAULT_MODEL: &str = "text-embedding-3-small";

/// Number of float dimensions expected from the embedding model.
/// `text-embedding-3-small` returns 1536 dims; we only use the first 256
/// (one per bit in our [u64; 4] binary embedding).
const BINARY_DIMS: usize = EMBEDDING_WORDS * 64; // 256

/// Client for generating binary embeddings from text.
pub struct EmbedClient {
    http: Client,
    api_key: String,
    api_url: String,
    model: String,
}

impl EmbedClient {
    pub fn new(api_key: String) -> Self {
        Self {
            http: Client::new(),
            api_key,
            api_url: OPENAI_EMBED_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Generate a binary embedding for the given text.
    pub async fn embed(&self, text: &str) -> Result<BinaryEmbedding, AgentError> {
        let body = EmbedRequest {
            model: &self.model,
            input: text,
        };

        let resp = self
            .http
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(AgentError::Http)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::Api { status, body });
        }

        let response: EmbedResponse = resp.json().await.map_err(AgentError::Http)?;
        let data = response
            .data
            .into_iter()
            .next()
            .ok_or_else(|| AgentError::Parse("empty embedding response".into()))?;
        Ok(binarize(&data.embedding))
    }
}

/// Convert a float embedding to a 256-bit binary embedding.
///
/// Strategy: threshold each of the first 256 dimensions at the median value.
/// Dimensions above median → 1, at or below → 0.
pub fn binarize(floats: &[f32]) -> BinaryEmbedding {
    let dims = floats.len().min(BINARY_DIMS);
    if dims == 0 {
        return BinaryEmbedding::default();
    }
    let slice = &floats[..dims];

    // Find median by sorting a copy (average of two middle values).
    let mut sorted = slice.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 0 && mid > 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };

    // Pack bits into u64 words.
    let mut words = [0u64; EMBEDDING_WORDS];
    for (i, &val) in slice.iter().enumerate() {
        if val > median {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            words[word_idx] |= 1u64 << bit_idx;
        }
    }

    BinaryEmbedding::new(words)
}

// --- Wire types ---

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

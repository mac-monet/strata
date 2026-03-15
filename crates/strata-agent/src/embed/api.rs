//! OpenAI-compatible embedding API client.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use strata_core::BinaryEmbedding;

use super::{Embedder, Threshold, binarize};
use crate::error::AgentError;

const OPENAI_EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
const DEFAULT_MODEL: &str = "text-embedding-3-small";

/// Generates embeddings by calling an OpenAI-compatible HTTP API.
pub struct ApiEmbedder {
    http: Client,
    api_key: String,
    api_url: String,
    model: String,
    threshold: Threshold,
}

impl ApiEmbedder {
    pub fn new(api_key: String) -> Self {
        Self {
            http: Client::new(),
            api_key,
            api_url: OPENAI_EMBED_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            threshold: Threshold::Median,
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

    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        self.threshold = threshold;
        self
    }

    /// Async embed — call the API and binarize the result.
    pub async fn embed_async(&self, text: &str) -> Result<BinaryEmbedding, AgentError> {
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
        Ok(binarize(&data.embedding, self.threshold))
    }
}

impl Embedder for ApiEmbedder {
    fn embed(&self, text: &str) -> Result<BinaryEmbedding, AgentError> {
        // Block on the async call. The caller should use embed_async() directly
        // when already in an async context. This impl exists for trait uniformity.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.embed_async(text))
        })
    }
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

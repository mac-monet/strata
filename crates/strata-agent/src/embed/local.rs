//! Local embedding via fastembed (ONNX Runtime).
//!
//! Runs the model on-device — no API calls, no network needed.
//! Enabled with the `local-embed` feature flag.

use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use strata_core::BinaryEmbedding;

use super::{Embedder, Threshold, binarize};
use crate::error::AgentError;

/// Generates embeddings locally using fastembed.
pub struct LocalEmbedder {
    model: Mutex<TextEmbedding>,
    threshold: Threshold,
}

impl LocalEmbedder {
    /// Create with a built-in fastembed model.
    /// Downloads model weights on first run and caches them locally.
    pub fn new(model_name: EmbeddingModel) -> Result<Self, AgentError> {
        let model = TextEmbedding::try_new(InitOptions::new(model_name))
            .map_err(|e| AgentError::Embed(e.to_string()))?;
        Ok(Self {
            model: Mutex::new(model),
            threshold: Threshold::Median,
        })
    }

    /// Create with mixedbread's mxbai-embed-large-v1 and zero threshold
    /// (the recommended configuration for this model).
    pub fn mixedbread() -> Result<Self, AgentError> {
        let model =
            TextEmbedding::try_new(InitOptions::new(EmbeddingModel::MxbaiEmbedLargeV1))
                .map_err(|e| AgentError::Embed(e.to_string()))?;
        Ok(Self {
            model: Mutex::new(model),
            threshold: Threshold::Zero,
        })
    }

    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        self.threshold = threshold;
        self
    }
}

impl Embedder for LocalEmbedder {
    fn embed(&self, text: &str) -> Result<BinaryEmbedding, AgentError> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| AgentError::Embed(e.to_string()))?;
        let embeddings = model
            .embed(vec![text], None)
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        let floats = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| AgentError::Embed("model returned no embeddings".into()))?;

        Ok(binarize(&floats, self.threshold))
    }
}

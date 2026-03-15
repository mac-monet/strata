//! Local embedding via tract (pure-Rust ONNX inference).
//!
//! Runs the model on-device — no API calls, no network needed.
//! Enabled with the `local-embed` feature flag.

use std::path::Path;

use tokenizers::Tokenizer;
use tract_onnx::prelude::*;

use strata_core::BinaryEmbedding;

use super::{Embedder, Threshold, binarize};
use crate::error::AgentError;

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// Generates embeddings locally using tract ONNX inference.
pub struct LocalEmbedder {
    plan: Plan,
    tokenizer: Tokenizer,
    threshold: Threshold,
}

impl LocalEmbedder {
    /// Create from local ONNX model and tokenizer files.
    pub fn from_files(
        model_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
    ) -> Result<Self, AgentError> {
        let symbols = SymbolScope::default();
        let s = symbols.sym("S");

        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| AgentError::Embed(e.to_string()))?
            .with_input_fact(
                0,
                InferenceFact::dt_shape(i64::datum_type(), [1.to_dim(), s.to_dim()]),
            )
            .map_err(|e| AgentError::Embed(e.to_string()))?
            .with_input_fact(
                1,
                InferenceFact::dt_shape(i64::datum_type(), [1.to_dim(), s.to_dim()]),
            )
            .map_err(|e| AgentError::Embed(e.to_string()))?
            .into_optimized()
            .map_err(|e| AgentError::Embed(e.to_string()))?
            .into_runnable()
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        Ok(Self {
            plan: model,
            tokenizer,
            threshold: Threshold::Zero,
        })
    }

    /// Create with mixedbread mxbai-embed-large-v1 defaults (zero threshold).
    ///
    /// `model_dir` should contain `model.onnx` and `tokenizer.json`.
    pub fn mixedbread(model_dir: impl AsRef<Path>) -> Result<Self, AgentError> {
        let dir = model_dir.as_ref();
        Self::from_files(dir.join("model.onnx"), dir.join("tokenizer.json"))
    }

    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        self.threshold = threshold;
        self
    }
}

impl Embedder for LocalEmbedder {
    fn embed(&self, text: &str) -> Result<BinaryEmbedding, AgentError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        let seq_len = ids.len();

        let input_ids = tract_ndarray::Array2::from_shape_vec(
            (1, seq_len),
            ids.iter().map(|&x| x as i64).collect(),
        )
        .map_err(|e| AgentError::Embed(e.to_string()))?;

        let attention_mask = tract_ndarray::Array2::from_shape_vec(
            (1, seq_len),
            mask.iter().map(|&x| x as i64).collect(),
        )
        .map_err(|e| AgentError::Embed(e.to_string()))?;

        let outputs = self
            .plan
            .run(tvec![input_ids.into_tvalue(), attention_mask.into_tvalue()])
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        let output = outputs[0]
            .to_array_view::<f32>()
            .map_err(|e| AgentError::Embed(e.to_string()))?;

        let floats = mean_pool(&output, mask);
        Ok(binarize(&floats, self.threshold))
    }
}

/// Mean-pool token embeddings weighted by the attention mask.
fn mean_pool(output: &tract_ndarray::ArrayViewD<f32>, mask: &[u32]) -> Vec<f32> {
    let shape = output.shape(); // [1, seq_len, hidden_dim]
    let hidden_dim = shape[2];
    let seq_len = shape[1];
    let mut pooled = vec![0.0f32; hidden_dim];
    let mut count = 0.0f32;

    for t in 0..seq_len {
        let w = mask[t] as f32;
        if w == 0.0 {
            continue;
        }
        count += w;
        for d in 0..hidden_dim {
            pooled[d] += output[[0, t, d]] * w;
        }
    }

    if count > 0.0 {
        for v in &mut pooled {
            *v /= count;
        }
    }
    pooled
}

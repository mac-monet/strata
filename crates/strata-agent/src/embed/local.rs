//! Local embedding via tract (pure-Rust ONNX inference).
//!
//! Runs the model on-device — no API calls, no network needed.
//! Enabled with the `local-embed` feature flag.
//!
//! Inference runs on a dedicated background thread to avoid blocking the
//! tokio executor. Requests are sent via a channel; results come back on
//! a per-request oneshot.

use std::path::Path;

use tokenizers::Tokenizer;
use tokio::sync::{mpsc, oneshot};
use tract_onnx::prelude::*;

use strata_core::BinaryEmbedding;

use super::{Embedder, Threshold, binarize};
use crate::error::AgentError;

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// A request sent to the embed thread.
struct EmbedRequest {
    text: String,
    tx: oneshot::Sender<Result<BinaryEmbedding, AgentError>>,
}

/// Generates embeddings locally using tract ONNX inference.
///
/// Inference runs on a dedicated OS thread; the async `embed` method
/// sends work over a channel and awaits the result without blocking tokio.
pub struct LocalEmbedder {
    sender: mpsc::UnboundedSender<EmbedRequest>,
}

impl LocalEmbedder {
    /// Create from local ONNX model and tokenizer files.
    pub fn from_files(
        model_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
    ) -> Result<Self, AgentError> {
        let (plan, num_inputs, symbols) = load_plan(model_path.as_ref())?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| AgentError::Embed(e.to_string()))?;
        Ok(Self::spawn(plan, num_inputs, tokenizer, Threshold::Zero, symbols))
    }

    /// Create with mixedbread mxbai-embed-large-v1 defaults (zero threshold).
    ///
    /// `model_dir` should contain `model.onnx` and `tokenizer.json`.
    pub fn mixedbread(model_dir: impl AsRef<Path>) -> Result<Self, AgentError> {
        let dir = model_dir.as_ref();
        Self::from_files(dir.join("model.onnx"), dir.join("tokenizer.json"))
    }

    pub fn with_threshold(self, threshold: Threshold) -> Self {
        // Threshold is set at construction time via spawn(); to change it
        // we'd need to rebuild. For now this is a no-op kept for API compat.
        let _ = threshold;
        self
    }

    /// Spawn the background inference thread and return a handle.
    fn spawn(plan: Plan, num_inputs: usize, tokenizer: Tokenizer, threshold: Threshold, symbols: SymbolScope) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<EmbedRequest>();

        std::thread::Builder::new()
            .name("local-embedder".into())
            .spawn(move || {
                // Keep the symbol scope alive for the lifetime of the plan.
                let _symbols = symbols;
                while let Some(req) = rx.blocking_recv() {
                    let result = embed_sync(&plan, num_inputs, &tokenizer, threshold, &req.text);
                    let _ = req.tx.send(result);
                }
            })
            .expect("failed to spawn local-embedder thread");

        Self { sender: tx }
    }
}

impl Embedder for LocalEmbedder {
    fn embed(
        &self,
        text: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<BinaryEmbedding, AgentError>> + Send + '_>,
    > {
        let (tx, rx) = oneshot::channel();
        let request = EmbedRequest {
            text: text.to_owned(),
            tx,
        };
        // If the thread has panicked, send will fail.
        let send_result = self.sender.send(request);
        Box::pin(async move {
            send_result.map_err(|_| AgentError::Embed("embed thread gone".into()))?;
            rx.await
                .map_err(|_| AgentError::Embed("embed thread dropped response".into()))?
        })
    }
}

fn load_plan(model_path: &Path) -> Result<(Plan, usize, SymbolScope), AgentError> {
    let symbols = SymbolScope::default();
    let s = symbols.sym("S");

    let mut model = tract_onnx::onnx()
        .model_for_path(model_path)
        .map_err(|e| AgentError::Embed(e.to_string()))?;

    let n_inputs = model.input_outlets().map_err(|e| AgentError::Embed(e.to_string()))?.len();

    for i in 0..n_inputs {
        model = model
            .with_input_fact(
                i,
                InferenceFact::dt_shape(i64::datum_type(), [1.to_dim(), s.to_dim()]),
            )
            .map_err(|e| AgentError::Embed(e.to_string()))?;
    }

    let optimized = model
        .into_optimized()
        .map_err(|e| AgentError::Embed(e.to_string()))?;

    let plan_inputs = optimized.input_outlets().map_err(|e| AgentError::Embed(e.to_string()))?.len();

    let plan = optimized
        .into_runnable()
        .map_err(|e| AgentError::Embed(e.to_string()))?;

    Ok((plan, plan_inputs, symbols))
}

fn embed_sync(
    plan: &Plan,
    num_inputs: usize,
    tokenizer: &Tokenizer,
    threshold: Threshold,
    text: &str,
) -> Result<BinaryEmbedding, AgentError> {
    let encoding = tokenizer
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

    let mut inputs = tvec![input_ids.into_tvalue(), attention_mask.into_tvalue()];
    if num_inputs >= 3 {
        let token_type_ids = tract_ndarray::Array2::<i64>::zeros((1, seq_len));
        inputs.push(token_type_ids.into_tvalue());
    }

    let outputs = plan
        .run(inputs)
        .map_err(|e| AgentError::Embed(e.to_string()))?;

    let output = outputs[0]
        .to_array_view::<f32>()
        .map_err(|e| AgentError::Embed(e.to_string()))?;

    let floats = mean_pool(&output, mask);
    Ok(binarize(&floats, threshold))
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

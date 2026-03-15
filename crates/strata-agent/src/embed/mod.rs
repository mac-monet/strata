//! Embedding generation: float vector → binary embedding conversion.
//!
//! Two backends:
//! - `ApiEmbedder` — calls an OpenAI-compatible embedding API over HTTP
//! - `LocalEmbedder` — runs a model locally via fastembed (feature `local-embed`)
//!
//! Both produce float vectors that are binarized into `BinaryEmbedding`.

mod api;
mod binarize;

#[cfg(feature = "local-embed")]
mod local;

pub use api::ApiEmbedder;
pub use binarize::{binarize, Threshold};

#[cfg(feature = "local-embed")]
pub use local::LocalEmbedder;

use strata_core::BinaryEmbedding;

use crate::error::AgentError;

/// Trait for generating binary embeddings from text.
pub trait Embedder: Send + Sync {
    /// Generate a binary embedding for the given text.
    fn embed(&self, text: &str) -> Result<BinaryEmbedding, AgentError>;
}

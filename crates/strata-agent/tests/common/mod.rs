//! Shared test helpers for strata-agent integration tests.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};

use commonware_runtime::deterministic;
use strata_core::{BinaryEmbedding, CoreState, Nonce, SoulHash, VectorRoot};
use strata_proof::{Keccak256Hasher, compute_root};
use strata_vector_db::Config as JournaledConfig;

use strata_agent::embed::Embedder;
use strata_agent::error::AgentError;

/// Deterministic embedder for testing: derives the embedding from a hash of
/// the input text so that different texts produce different embeddings.
#[allow(dead_code)]
pub struct FixedEmbedder;

impl Embedder for FixedEmbedder {
    fn embed(
        &self,
        text: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<BinaryEmbedding, AgentError>> + Send + '_>,
    > {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let id = hasher.finish();
        Box::pin(async move { Ok(BinaryEmbedding::test_from_id(id)) })
    }
}

/// Build a `JournaledConfig` for VectorDB tests. The suffix is used to
/// create unique partition names so tests don't collide.
pub fn make_config(suffix: &str, context: &deterministic::Context) -> JournaledConfig {
    let page_size = NonZeroU16::new(4096).unwrap();
    let page_cache_size = NonZeroUsize::new(8).unwrap();
    JournaledConfig {
        journal_partition: format!("test-journal-{suffix}"),
        metadata_partition: format!("test-meta-{suffix}"),
        items_per_blob: NonZeroU64::new(1000).unwrap(),
        write_buffer: NonZeroUsize::new(1024).unwrap(),
        thread_pool: None,
        page_cache: commonware_runtime::buffer::paged::CacheRef::from_pooler(
            context,
            page_size,
            page_cache_size,
        ),
    }
}

/// A genesis `CoreState` with empty DB root and nonce 0.
#[allow(dead_code)]
pub fn genesis_state() -> CoreState {
    let root = compute_root::<Keccak256Hasher>(0, &[]);
    CoreState {
        soul_hash: SoulHash::digest(b"test-soul"),
        vector_index_root: VectorRoot::new(root),
        nonce: Nonce::new(0),
    }
}

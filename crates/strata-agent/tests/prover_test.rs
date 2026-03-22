//! Tests for the prover module.

use std::path::PathBuf;

use strata_agent::prover::{ProofLevel, ProverConfig};

#[test]
fn proof_level_as_str() {
    assert_eq!(ProofLevel::App.as_str(), "app");
    assert_eq!(ProofLevel::Evm.as_str(), "evm");
}

#[test]
fn prover_config_defaults() {
    let config = ProverConfig::new(PathBuf::from("/tmp/strata-openvm"), ProofLevel::App);
    assert_eq!(config.openvm_dir, PathBuf::from("/tmp/strata-openvm"));
    assert_eq!(config.proof_level.as_str(), "app");
    assert_eq!(config.timeout.as_secs(), 600);
}

#[tokio::test]
async fn prove_missing_openvm_dir_returns_error() {
    use strata_core::*;
    use strata_proof::Witness;

    let config = ProverConfig::new(
        PathBuf::from("/nonexistent/strata-openvm"),
        ProofLevel::App,
    );

    let state = CoreState {
        soul_hash: SoulHash::digest(b"test"),
        vector_index_root: VectorRoot::new([0u8; 32]),
        nonce: Nonce::new(0),
    };

    let transition = strata_agent::pipeline::TransitionOutput {
        record: TransitionRecord::new(
            Input::new(
                Nonce::new(1),
                InputPayload::MemoryUpdate,
                InputSignature::default(),
            ),
            vec![],
            vec![],
        ),
        witness: Witness {
            old_peaks: vec![],
            old_leaf_count: 0,
            new_entries: vec![],
        },
        old_state: state,
        new_state: CoreState {
            soul_hash: SoulHash::digest(b"test"),
            vector_index_root: VectorRoot::new([0u8; 32]),
            nonce: Nonce::new(1),
        },
    };

    let result = strata_agent::prover::prove_batch(&config, &[transition]).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("prover"), "expected prover error, got: {err}");
}

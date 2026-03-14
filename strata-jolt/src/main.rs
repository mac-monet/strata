use std::time::Instant;

use jolt_sdk::UntrustedAdvice;
use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Blake3Hasher, Witness, compute_root};
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // --- Build test data ---
    let genesis_root = compute_root::<Blake3Hasher>(0, &[]);
    let state = CoreState {
        soul_hash: SoulHash::digest(b"test-soul"),
        vector_index_root: VectorRoot::new(genesis_root),
        nonce: Nonce::new(0),
    };

    let entries = vec![MemoryEntry::new(
        MemoryId::new(0),
        BinaryEmbedding::new([1, 2, 3, 4]),
        ContentHash::digest(b"hello world"),
    )];

    let witness = Witness {
        old_peaks: vec![],
        old_leaf_count: 0,
        new_entries: entries,
    };

    let nonce = 1u64;

    // --- Jolt setup ---
    let target_dir = "/tmp/jolt-guest-targets";
    let mut program = guest::compile_verify_transition(target_dir);

    // Analyze first to get cycle count
    info!("Analyzing guest cycle count...");
    let summary = guest::analyze_verify_transition(
        state,
        nonce,
        UntrustedAdvice::new(witness.clone()),
    );
    summary
        .write_to_file("summary.txt".into())
        .unwrap();
    info!("Analysis written to summary.txt");

    let shared = guest::preprocess_shared_verify_transition(&mut program);
    let prover_prep = guest::preprocess_prover_verify_transition(shared.clone());
    let verifier_setup = prover_prep.generators.to_verifier_setup();
    let verifier_prep =
        guest::preprocess_verifier_verify_transition(shared, verifier_setup, None);

    let prove = guest::build_prover_verify_transition(program, prover_prep);
    let verify = guest::build_verifier_verify_transition(verifier_prep);

    info!("Proving...");
    let t = Instant::now();
    let (output, proof, io) = prove(state, nonce, UntrustedAdvice::new(witness));
    info!("Prover runtime: {} s", t.elapsed().as_secs_f64());

    let is_valid = verify(state, nonce, output, io.panic, proof);
    info!("output: {:?}", output);
    info!("valid: {is_valid}");
    assert!(is_valid);
}

//! Host prover binary for strata-openvm.
//!
//! Proof-of-concept: builds test data, compiles the guest, runs it,
//! and (when OpenVM tooling is installed) proves and verifies.
//!
//! Usage:
//!   cd strata-openvm
//!   cargo openvm build
//!   cargo openvm run
//!   cargo openvm prove app
//!
//! This file serves as documentation and a placeholder host binary.
//! Full integration with the agent runtime will come later.

use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Keccak256Hasher, Witness, compute_root};

fn main() {
    // Build test data matching strata-jolt's test case.
    let genesis_root = compute_root::<Keccak256Hasher>(0, &[]);
    let state = CoreState {
        soul_hash: SoulHash::digest(b"test-soul"),
        vector_index_root: VectorRoot::new(genesis_root),
        nonce: Nonce::new(0),
    };

    let entries = vec![MemoryEntry::new(
        MemoryId::new(0),
        BinaryEmbedding::test_from_id(1),
        ContentHash::digest(b"hello world"),
    )];

    let witness = Witness {
        old_peaks: vec![],
        old_leaf_count: 0,
        new_entries: entries,
    };

    let nonce = 1u64;

    println!("State: {:?}", state);
    println!("Nonce: {}", nonce);
    println!("Witness entries: {}", witness.new_entries.len());
    println!();
    println!("To prove this transition:");
    println!("  cd strata-openvm");
    println!("  cargo openvm build");
    println!("  cargo openvm run --input <test_input>");
    println!("  cargo openvm prove app --input <test_input>");

    // Run the transition locally (non-ZK) as a sanity check.
    let new_state =
        strata_proof::transition::<Keccak256Hasher>(state, Nonce::new(nonce), &witness)
            .expect("transition failed");
    println!();
    println!("Local transition succeeded:");
    println!("  old root:  {:?}", state.vector_index_root);
    println!("  new root:  {:?}", new_state.vector_index_root);
    println!("  new nonce: {:?}", new_state.nonce);
    println!("  soul hash: {:?}", new_state.soul_hash);
    println!();
    println!("Public values layout (104 bytes):");
    println!("  [0..32]   oldRoot");
    println!("  [32..64]  newRoot");
    println!("  [64..72]  nonce (u64 BE)");
    println!("  [72..104] soulHash");
}

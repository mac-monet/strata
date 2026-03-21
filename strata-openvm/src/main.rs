//! Host prover binary for strata-openvm.
//!
//! Two modes:
//!   1. **Demo** (no args): builds test data, runs local transition, prints info.
//!   2. **Prove** (`prove --input <file> --level <level>`): reads JSON input,
//!      serializes for OpenVM StdIn, and invokes the prover.
//!
//! Usage:
//!   cd strata-openvm
//!   cargo run                                      # demo mode
//!   cargo run -- prove --input input.json --level app

use std::env;
use std::fs;
use std::path::PathBuf;

use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Keccak256Hasher, Witness, compute_root};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "prove" {
        prove_mode(&args[2..]);
    } else {
        demo_mode();
    }
}

/// Parse `--input <path>` and `--level <level>` from args.
struct ProveArgs {
    input: PathBuf,
    level: String,
}

fn parse_prove_args(args: &[String]) -> ProveArgs {
    let mut input = None;
    let mut level = String::from("app");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                i += 1;
                input = Some(PathBuf::from(&args[i]));
            }
            "--level" => {
                i += 1;
                level = args[i].clone();
            }
            _ => {}
        }
        i += 1;
    }
    ProveArgs {
        input: input.expect("--input <file> is required"),
        level,
    }
}

/// Prove mode: read JSON input, serialize for OpenVM, invoke prover.
fn prove_mode(args: &[String]) {
    let prove_args = parse_prove_args(args);

    let json_str = fs::read_to_string(&prove_args.input)
        .unwrap_or_else(|e| panic!("failed to read input file: {e}"));

    let input: serde_json::Value =
        serde_json::from_str(&json_str).unwrap_or_else(|e| panic!("invalid JSON: {e}"));

    let state: CoreState =
        serde_json::from_value(input["state"].clone()).unwrap_or_else(|e| panic!("bad state: {e}"));
    let nonce: Nonce = serde_json::from_value(input["nonce"].clone())
        .unwrap_or_else(|e| panic!("bad nonce: {e}"));
    let witness: Witness = serde_json::from_value(input["witness"].clone())
        .unwrap_or_else(|e| panic!("bad witness: {e}"));

    // Run local verification first as a sanity check.
    let new_state = strata_proof::transition::<Keccak256Hasher>(state, nonce, &witness)
        .expect("local transition verification failed");

    println!("Local verification passed.");
    println!("  old root:  {:?}", state.vector_index_root);
    println!("  new root:  {:?}", new_state.vector_index_root);
    println!("  nonce:     {:?}", new_state.nonce);

    // TODO: Once OpenVM SDK is fully integrated:
    // 1. Build StdIn with state, nonce, witness via openvm_sdk::StdIn
    // 2. Compile guest if needed
    // 3. Run `cargo openvm prove <level>` with serialized input
    // 4. Write proof to `strata-openvm-guest.<level>.proof`
    //
    // For now, print what would be done.
    println!();
    println!("Proving at level: {}", prove_args.level);
    println!("To complete proving, run:");
    println!("  cargo openvm build");
    println!(
        "  cargo openvm prove {} --input <serialized>",
        prove_args.level
    );
}

/// Demo mode: original behavior — build test data and run local transition.
fn demo_mode() {
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

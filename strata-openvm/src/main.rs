//! Host prover binary for strata-openvm.
//!
//! Subcommands:
//!   1. **Demo** (no args): builds test data, runs local transition, prints info.
//!   2. **Prove** (`prove --input <file> --level <level>`): reads JSON input,
//!      builds the guest, and generates a ZK proof via the OpenVM SDK.
//!   3. **Generate verifier** (`generate-verifier`): outputs the Solidity verifier
//!      contract for on-chain proof verification.
//!
//! Usage:
//!   cd strata-openvm
//!   cargo run                                      # demo mode
//!   cargo run -- prove --input input.json --level app
//!   cargo run -- prove --input input.json --level evm
//!   cargo run -- generate-verifier

use std::env;
use std::fs;
use std::path::PathBuf;

use openvm_build::GuestOptions;
use openvm_sdk::{Sdk, StdIn, config::SdkVmConfig};
use openvm_transpiler::elf::Elf;

use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Keccak256Hasher, Witness, compute_root};

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("prove") => prove_mode(&args[2..]),
        Some("generate-verifier") => generate_verifier_mode(),
        _ => demo_mode(),
    }
}

// ---------------------------------------------------------------------------
// Prove mode
// ---------------------------------------------------------------------------

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
            "--input" if i + 1 < args.len() => {
                i += 1;
                input = Some(PathBuf::from(&args[i]));
            }
            "--level" if i + 1 < args.len() => {
                i += 1;
                level = args[i].clone();
            }
            "--input" | "--level" => {
                panic!("{} requires a value", args[i]);
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

/// Public values size in bytes. Our layout is 104 bytes (26 u32 words).
/// Round up to 128 for alignment headroom.
const PUBLIC_VALUES_SIZE: usize = 128;

/// Build the SDK from openvm.toml config.
fn build_sdk() -> Sdk {
    let toml_content =
        fs::read_to_string("openvm.toml").expect("failed to read openvm.toml");
    let mut app_config: openvm_sdk::config::AppConfig<SdkVmConfig> =
        SdkVmConfig::from_toml(&toml_content).expect("failed to parse openvm.toml");

    // Override public values size — our guest outputs 104 bytes.
    app_config.app_vm_config.system.config =
        app_config.app_vm_config.system.config.with_public_values(PUBLIC_VALUES_SIZE);

    Sdk::new(app_config).expect("failed to create SDK")
}

/// Build the SDK and compile the guest ELF.
fn build_sdk_and_elf() -> (Sdk, Elf) {
    let sdk = build_sdk();

    eprintln!("building guest ELF...");
    let elf = sdk
        .build(GuestOptions::default(), "guest", &None, None)
        .expect("failed to build guest");
    eprintln!("guest ELF built.");

    (sdk, elf)
}

/// Prepare StdIn from the transition inputs, matching the guest's read order:
///   1. CoreState
///   2. u64 (nonce)
///   3. Witness
fn prepare_stdin(state: &CoreState, nonce: u64, witness: &Witness) -> StdIn {
    let mut stdin = StdIn::default();
    stdin.write(state);
    stdin.write(&nonce);
    stdin.write(witness);
    stdin
}

/// Prove mode: read JSON input, build guest, generate proof.
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

    // Local sanity check.
    let new_state = strata_proof::transition::<Keccak256Hasher>(state, nonce, &witness)
        .expect("local transition verification failed");
    eprintln!("local verification passed");
    eprintln!("  old root: {:?}", state.vector_index_root);
    eprintln!("  new root: {:?}", new_state.vector_index_root);

    let (sdk, elf) = build_sdk_and_elf();
    let stdin = prepare_stdin(&state, nonce.get(), &witness);

    match prove_args.level.as_str() {
        "app" => prove_app(&sdk, elf, stdin),
        "evm" => prove_evm(&sdk, elf, stdin),
        other => panic!("unknown proof level: {other} (expected: app, evm)"),
    }
}

/// Generate an app-level proof (fast, not on-chain verifiable).
fn prove_app(sdk: &Sdk, elf: Elf, stdin: StdIn) {
    eprintln!("generating app proof...");
    let (proof, commit) = sdk.prove(elf, stdin).expect("app proof generation failed");
    eprintln!("app proof generated.");

    // Write proof bytes.
    let proof_bytes = openvm_sdk::codec::Encode::encode_to_vec(&proof)
        .expect("failed to encode proof");
    let proof_path = "strata-openvm-guest.app.proof";
    fs::write(proof_path, &proof_bytes)
        .unwrap_or_else(|e| panic!("failed to write {proof_path}: {e}"));
    eprintln!("proof written to {proof_path} ({} bytes)", proof_bytes.len());

    // Write commit.
    write_commit("app", &commit);
}

/// Generate an EVM-level proof (slow, on-chain verifiable via Halo2).
fn prove_evm(sdk: &Sdk, elf: Elf, stdin: StdIn) {
    eprintln!("generating EVM proof (this may take several minutes)...");
    let evm_proof = sdk.prove_evm(elf, stdin).expect("EVM proof generation failed");
    eprintln!("EVM proof generated.");

    // Write proof_data = accumulator ++ proof (this is what goes on-chain).
    let mut proof_bytes = Vec::new();
    proof_bytes.extend_from_slice(&evm_proof.proof_data.accumulator);
    proof_bytes.extend_from_slice(&evm_proof.proof_data.proof);

    let proof_path = "strata-openvm-guest.evm.proof";
    fs::write(proof_path, &proof_bytes)
        .unwrap_or_else(|e| panic!("failed to write {proof_path}: {e}"));
    eprintln!("proof written to {proof_path} ({} bytes)", proof_bytes.len());

    // Write public values.
    let pv_path = "strata-openvm-guest.evm.public_values";
    fs::write(pv_path, &evm_proof.user_public_values)
        .unwrap_or_else(|e| panic!("failed to write {pv_path}: {e}"));

    // Write commit.
    write_commit("evm", &evm_proof.app_commit);

    // Also write the full EVM proof as JSON for debugging.
    let json_path = "strata-openvm-guest.evm.proof.json";
    let json = serde_json::to_string_pretty(&evm_proof)
        .expect("failed to serialize EVM proof");
    fs::write(json_path, &json)
        .unwrap_or_else(|e| panic!("failed to write {json_path}: {e}"));
}

/// Write the app execution commit to a JSON file.
fn write_commit(level: &str, commit: &openvm_sdk::commit::AppExecutionCommit) {
    let commit_json = serde_json::json!({
        "app_exe_commit": hex::encode(commit.app_exe_commit.as_slice()),
        "app_vm_commit": hex::encode(commit.app_vm_commit.as_slice()),
    });
    let path = format!("strata-openvm-guest.{level}.commit.json");
    let json = serde_json::to_string_pretty(&commit_json).unwrap();
    fs::write(&path, &json)
        .unwrap_or_else(|e| panic!("failed to write {path}: {e}"));
    eprintln!("commit written to {path}");
}

// ---------------------------------------------------------------------------
// Generate verifier mode
// ---------------------------------------------------------------------------

fn generate_verifier_mode() {
    let sdk = build_sdk();

    eprintln!("generating Halo2 verifier contract...");
    let verifier = sdk
        .generate_halo2_verifier_solidity()
        .expect("failed to generate verifier");

    // Write the OpenVM verifier (the contract our StrataRollup calls).
    let verifier_path = "../contracts/src/OpenVmHalo2Verifier.sol";
    fs::write(verifier_path, &verifier.openvm_verifier_code)
        .unwrap_or_else(|e| panic!("failed to write {verifier_path}: {e}"));
    eprintln!("verifier written to {verifier_path}");

    // Write the Halo2 verifier (low-level pairing check).
    let halo2_path = "../contracts/src/Halo2Verifier.sol";
    fs::write(halo2_path, &verifier.halo2_verifier_code)
        .unwrap_or_else(|e| panic!("failed to write {halo2_path}: {e}"));
    eprintln!("Halo2 verifier written to {halo2_path}");
}

// ---------------------------------------------------------------------------
// Demo mode
// ---------------------------------------------------------------------------

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

    // Write test input JSON for use with `prove` subcommand.
    let input_json = serde_json::json!({
        "state": state,
        "nonce": Nonce::new(nonce),
        "witness": witness,
    });
    let json_str = serde_json::to_string_pretty(&input_json).unwrap();
    fs::write("test-input.json", &json_str).expect("failed to write test-input.json");
    eprintln!("wrote test-input.json");

    // Run the transition locally as a sanity check.
    let new_state =
        strata_proof::transition::<Keccak256Hasher>(state, Nonce::new(nonce), &witness)
            .expect("transition failed");

    println!("Local transition succeeded:");
    println!("  old root:  {:?}", state.vector_index_root);
    println!("  new root:  {:?}", new_state.vector_index_root);
    println!("  new nonce: {:?}", new_state.nonce);
    println!("  soul hash: {:?}", new_state.soul_hash);

    // Build SDK and execute (no proof) to verify guest compatibility.
    let (sdk, elf) = build_sdk_and_elf();
    let stdin = prepare_stdin(&state, nonce, &witness);

    eprintln!("executing guest program (no proof)...");
    let public_values = sdk
        .execute(elf, stdin)
        .expect("guest execution failed");
    eprintln!("guest execution succeeded, {} bytes of public output", public_values.len());

    println!();
    println!("Public values ({} bytes):", public_values.len());
    println!("  [0..32]   oldRoot:  {}", hex::encode(&public_values[..32]));
    println!("  [32..64]  newRoot:  {}", hex::encode(&public_values[32..64]));
    println!("  [64..72]  nonce:    {}", hex::encode(&public_values[64..72]));
    println!("  [72..104] soulHash: {}", hex::encode(&public_values[72..104]));
}

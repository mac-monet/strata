use sha2::{Digest, Sha256};
use std::time::Instant;
use tracing::info;

const DEPTH: usize = 16;
const EMPTY_LEAF: [u8; 32] = [0u8; 32];
const ENTRY_SIZE: usize = 32 + 4 + DEPTH * 32;

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// Fixed-depth binary Merkle tree with cached internal nodes.
/// Parameterized by hash function for SHA-256 vs Blake3 comparison.
struct MerkleTree {
    /// nodes[level][index] — level 0 = leaves, level DEPTH = root
    nodes: Vec<Vec<[u8; 32]>>,
    hash_pair: fn(&[u8; 32], &[u8; 32]) -> [u8; 32],
}

fn hash_pair_sha256(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    sha256(&buf)
}

fn hash_pair_blake3(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    blake3_hash(&buf)
}

impl MerkleTree {
    fn new(hash_pair: fn(&[u8; 32], &[u8; 32]) -> [u8; 32]) -> Self {
        let num_leaves = 1usize << DEPTH;
        let mut nodes = Vec::with_capacity(DEPTH + 1);

        nodes.push(vec![EMPTY_LEAF; num_leaves]);

        for level in 1..=DEPTH {
            let prev = &nodes[level - 1];
            let size = prev.len() / 2;
            let mut level_nodes = Vec::with_capacity(size);
            for i in 0..size {
                level_nodes.push(hash_pair(&prev[i * 2], &prev[i * 2 + 1]));
            }
            nodes.push(level_nodes);
        }

        Self { nodes, hash_pair }
    }

    fn root(&self) -> [u8; 32] {
        self.nodes[DEPTH][0]
    }

    fn siblings(&self, leaf_index: u32) -> [[u8; 32]; DEPTH] {
        let mut siblings = [[0u8; 32]; DEPTH];
        let mut idx = leaf_index as usize;
        for level in 0..DEPTH {
            siblings[level] = self.nodes[level][idx ^ 1];
            idx >>= 1;
        }
        siblings
    }

    fn insert(&mut self, index: u32, leaf: [u8; 32]) {
        let mut idx = index as usize;
        self.nodes[0][idx] = leaf;

        for level in 0..DEPTH {
            let parent = idx / 2;
            let left = self.nodes[level][parent * 2];
            let right = self.nodes[level][parent * 2 + 1];
            self.nodes[level + 1][parent] = (self.hash_pair)(&left, &right);
            idx = parent;
        }
    }
}

/// Build serialized batch data: for each entry, capture siblings BEFORE inserting.
fn generate_batch(
    tree: &mut MerkleTree,
    count: u32,
    leaf_hash_fn: fn(&[u8]) -> [u8; 32],
) -> Vec<u8> {
    let mut data = Vec::with_capacity(count as usize * ENTRY_SIZE);
    for i in 0..count {
        let leaf_hash = leaf_hash_fn(&i.to_le_bytes());
        let siblings = tree.siblings(i);

        data.extend_from_slice(&leaf_hash);
        data.extend_from_slice(&i.to_le_bytes());
        for s in &siblings {
            data.extend_from_slice(s);
        }

        tree.insert(i, leaf_hash);
    }
    data
}

fn benchmark_sha256(batch_size: u32) {
    info!("======== SHA-256  batch={batch_size} ========");

    let t = Instant::now();
    let mut tree = MerkleTree::new(hash_pair_sha256);
    let old_root = tree.root();
    let batch_data = generate_batch(&mut tree, batch_size, sha256);
    let expected_root = tree.root();
    info!("Data gen:  {:.2}s", t.elapsed().as_secs_f64());

    let target_dir = "/tmp/jolt-guest-targets";
    let t = Instant::now();
    let mut program = guest::compile_transition(target_dir);
    info!("Compile:   {:.2}s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let shared = guest::preprocess_shared_transition(&mut program);
    let prover_prep = guest::preprocess_prover_transition(shared.clone());
    let verifier_setup = prover_prep.generators.to_verifier_setup();
    let verifier_prep =
        guest::preprocess_verifier_transition(shared, verifier_setup, None);
    info!("Preproc:   {:.2}s", t.elapsed().as_secs_f64());

    let prove = guest::build_prover_transition(program, prover_prep);
    let verify = guest::build_verifier_transition(verifier_prep);

    let t = Instant::now();
    let (output, proof, io) = prove(old_root, 0u64, 1u64, batch_data.clone(), batch_size);
    let prove_time = t.elapsed();
    info!("Prove:     {:.2}s", prove_time.as_secs_f64());

    if io.panic {
        panic!("Guest panicked during SHA-256 proving!");
    }

    let t = Instant::now();
    let valid = verify(old_root, 0u64, 1u64, batch_data, batch_size, output, io.panic, proof);
    let verify_time = t.elapsed();
    info!("Verify:    {:.2}s", verify_time.as_secs_f64());

    assert!(valid, "SHA-256 proof verification failed!");
    assert_eq!(output, expected_root, "SHA-256 root mismatch!");

    info!(
        "RESULT:    SHA-256  batch={batch_size}  prove={:.2}s  verify={:.2}s",
        prove_time.as_secs_f64(),
        verify_time.as_secs_f64(),
    );
    info!("");
}

fn benchmark_blake3(batch_size: u32) {
    info!("======== Blake3   batch={batch_size} ========");

    let t = Instant::now();
    let mut tree = MerkleTree::new(hash_pair_blake3);
    let old_root = tree.root();
    let batch_data = generate_batch(&mut tree, batch_size, blake3_hash);
    let expected_root = tree.root();
    info!("Data gen:  {:.2}s", t.elapsed().as_secs_f64());

    let target_dir = "/tmp/jolt-guest-targets";
    let t = Instant::now();
    let mut program = guest::compile_transition_blake3(target_dir);
    info!("Compile:   {:.2}s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let shared = guest::preprocess_shared_transition_blake3(&mut program);
    let prover_prep = guest::preprocess_prover_transition_blake3(shared.clone());
    let verifier_setup = prover_prep.generators.to_verifier_setup();
    let verifier_prep =
        guest::preprocess_verifier_transition_blake3(shared, verifier_setup, None);
    info!("Preproc:   {:.2}s", t.elapsed().as_secs_f64());

    let prove = guest::build_prover_transition_blake3(program, prover_prep);
    let verify = guest::build_verifier_transition_blake3(verifier_prep);

    let t = Instant::now();
    let (output, proof, io) = prove(old_root, 0u64, 1u64, batch_data.clone(), batch_size);
    let prove_time = t.elapsed();
    info!("Prove:     {:.2}s", prove_time.as_secs_f64());

    if io.panic {
        panic!("Guest panicked during Blake3 proving!");
    }

    let t = Instant::now();
    let valid = verify(old_root, 0u64, 1u64, batch_data, batch_size, output, io.panic, proof);
    let verify_time = t.elapsed();
    info!("Verify:    {:.2}s", verify_time.as_secs_f64());

    assert!(valid, "Blake3 proof verification failed!");
    assert_eq!(output, expected_root, "Blake3 root mismatch!");

    info!(
        "RESULT:    Blake3   batch={batch_size}  prove={:.2}s  verify={:.2}s",
        prove_time.as_secs_f64(),
        verify_time.as_secs_f64(),
    );
    info!("");
}

pub fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut use_blake3 = false;
    let mut batch_sizes = Vec::new();

    for arg in &args {
        if arg == "--blake3" {
            use_blake3 = true;
        } else if arg == "--both" {
            use_blake3 = true; // handled below
        } else {
            batch_sizes.push(arg.parse::<u32>().expect("usage: jolt-merkle [--blake3|--both] <batch_size> ..."));
        }
    }

    if batch_sizes.is_empty() {
        batch_sizes.push(1);
    }

    let run_both = args.iter().any(|a| a == "--both");

    for &bs in &batch_sizes {
        if run_both {
            benchmark_sha256(bs);
            benchmark_blake3(bs);
        } else if use_blake3 {
            benchmark_blake3(bs);
        } else {
            benchmark_sha256(bs);
        }
    }
}

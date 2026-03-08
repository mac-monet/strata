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

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    sha256(&buf)
}

/// Fixed-depth binary Merkle tree with cached internal nodes.
struct MerkleTree {
    /// nodes[level][index] — level 0 = leaves, level DEPTH = root
    nodes: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    fn new() -> Self {
        let num_leaves = 1usize << DEPTH;
        let mut nodes = Vec::with_capacity(DEPTH + 1);

        // Level 0: all empty leaves
        nodes.push(vec![EMPTY_LEAF; num_leaves]);

        // Build empty tree bottom-up
        for level in 1..=DEPTH {
            let prev = &nodes[level - 1];
            let size = prev.len() / 2;
            let mut level_nodes = Vec::with_capacity(size);
            for i in 0..size {
                level_nodes.push(hash_pair(&prev[i * 2], &prev[i * 2 + 1]));
            }
            nodes.push(level_nodes);
        }

        Self { nodes }
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

        // Recompute path to root
        for level in 0..DEPTH {
            let parent = idx / 2;
            let left = self.nodes[level][parent * 2];
            let right = self.nodes[level][parent * 2 + 1];
            self.nodes[level + 1][parent] = hash_pair(&left, &right);
            idx = parent;
        }
    }
}

/// Build serialized batch data: for each entry, capture siblings BEFORE inserting.
fn generate_batch(tree: &mut MerkleTree, count: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(count as usize * ENTRY_SIZE);
    for i in 0..count {
        let leaf_hash = sha256(&i.to_le_bytes());
        let siblings = tree.siblings(i);

        // Serialize: leaf_hash || index (u32 LE) || siblings[0..DEPTH]
        data.extend_from_slice(&leaf_hash);
        data.extend_from_slice(&i.to_le_bytes());
        for s in &siblings {
            data.extend_from_slice(s);
        }

        tree.insert(i, leaf_hash);
    }
    data
}

fn benchmark(batch_size: u32) {
    info!("========================================");
    info!("  batch_size = {batch_size}");
    info!("========================================");

    // Generate test data
    let t = Instant::now();
    let mut tree = MerkleTree::new();
    let old_root = tree.root();
    let batch_data = generate_batch(&mut tree, batch_size);
    let expected_root = tree.root();
    info!(
        "Data gen:  {:.2}s  (old_root={}, expected_root={})",
        t.elapsed().as_secs_f64(),
        &hex::encode(old_root)[..16],
        &hex::encode(expected_root)[..16],
    );

    // Compile guest to RISC-V (cached after first run)
    let target_dir = "/tmp/jolt-guest-targets";
    let t = Instant::now();
    let mut program = guest::compile_transition(target_dir);
    info!("Compile:   {:.2}s", t.elapsed().as_secs_f64());

    // Preprocess
    let t = Instant::now();
    let shared = guest::preprocess_shared_transition(&mut program);
    let prover_prep = guest::preprocess_prover_transition(shared.clone());
    let verifier_setup = prover_prep.generators.to_verifier_setup();
    let verifier_prep =
        guest::preprocess_verifier_transition(shared, verifier_setup, None);
    info!("Preproc:   {:.2}s", t.elapsed().as_secs_f64());

    // Build prover + verifier
    let prove = guest::build_prover_transition(program, prover_prep);
    let verify = guest::build_verifier_transition(verifier_prep);

    // Prove
    let t = Instant::now();
    let (output, proof, io) = prove(
        old_root,
        0u64,
        1u64,
        batch_data.clone(),
        batch_size,
    );
    let prove_time = t.elapsed();
    info!("Prove:     {:.2}s", prove_time.as_secs_f64());

    // Check for guest panic
    if io.panic {
        panic!("Guest panicked during proving!");
    }

    // Verify
    let t = Instant::now();
    let valid = verify(
        old_root,
        0u64,
        1u64,
        batch_data,
        batch_size,
        output,
        io.panic,
        proof,
    );
    let verify_time = t.elapsed();
    info!("Verify:    {:.2}s", verify_time.as_secs_f64());

    info!("Output:    {}", hex::encode(output));
    info!("Valid:     {valid}");
    assert!(valid, "Proof verification failed!");
    assert_eq!(output, expected_root, "Root mismatch!");

    info!(
        "RESULT:    batch={batch_size}  prove={:.2}s  verify={:.2}s",
        prove_time.as_secs_f64(),
        verify_time.as_secs_f64(),
    );
    info!("");
}

pub fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let batch_sizes: Vec<u32> = std::env::args()
        .skip(1)
        .map(|s| s.parse().expect("usage: jolt-merkle <batch_size> ..."))
        .collect();

    let batch_sizes = if batch_sizes.is_empty() {
        vec![1]
    } else {
        batch_sizes
    };

    for &bs in &batch_sizes {
        benchmark(bs);
    }
}

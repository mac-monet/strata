// Jolt guest: proves nonce verification + merkle tree update.
//
// Two variants: SHA-256 and Blake3, each as a separate provable function.
// Using guest-std mode for Vec params. Production would use no_std + UntrustedAdvice.

use jolt_inlines_blake3::Blake3;
use jolt_inlines_sha2::Sha256;

const DEPTH: usize = 16;
const EMPTY_LEAF: [u8; 32] = [0u8; 32];

/// Per-entry serialized size: 32 (leaf) + 4 (index) + DEPTH * 32 (siblings)
const ENTRY_SIZE: usize = 32 + 4 + DEPTH * 32;

#[inline(always)]
fn hash_pair_sha256(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    Sha256::digest(&buf)
}

#[inline(always)]
fn hash_pair_blake3(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    Blake3::digest(&buf)
}

fn compute_root_sha256(leaf: &[u8; 32], index: u32, siblings: &[[u8; 32]; DEPTH]) -> [u8; 32] {
    let mut current = *leaf;
    let mut idx = index;
    for i in 0..DEPTH {
        if idx & 1 == 0 {
            current = hash_pair_sha256(&current, &siblings[i]);
        } else {
            current = hash_pair_sha256(&siblings[i], &current);
        }
        idx >>= 1;
    }
    current
}

fn compute_root_blake3(leaf: &[u8; 32], index: u32, siblings: &[[u8; 32]; DEPTH]) -> [u8; 32] {
    let mut current = *leaf;
    let mut idx = index;
    for i in 0..DEPTH {
        if idx & 1 == 0 {
            current = hash_pair_blake3(&current, &siblings[i]);
        } else {
            current = hash_pair_blake3(&siblings[i], &current);
        }
        idx >>= 1;
    }
    current
}

/// Parse batch data and run merkle verification using the provided hash functions.
/// Returns the new merkle root.
fn verify_batch(
    old_root: [u8; 32],
    old_nonce: u64,
    input_nonce: u64,
    batch_data: &[u8],
    num_entries: u32,
    compute_root_fn: fn(&[u8; 32], u32, &[[u8; 32]; DEPTH]) -> [u8; 32],
) -> [u8; 32] {
    assert!(input_nonce == old_nonce + 1);

    let n = num_entries as usize;
    assert!(batch_data.len() == n * ENTRY_SIZE);

    let mut current_root = old_root;

    for i in 0..n {
        let off = i * ENTRY_SIZE;

        let mut leaf_hash = [0u8; 32];
        leaf_hash.copy_from_slice(&batch_data[off..off + 32]);

        let index = u32::from_le_bytes([
            batch_data[off + 32],
            batch_data[off + 33],
            batch_data[off + 34],
            batch_data[off + 35],
        ]);

        let mut siblings = [[0u8; 32]; DEPTH];
        for j in 0..DEPTH {
            let s_off = off + 36 + j * 32;
            siblings[j].copy_from_slice(&batch_data[s_off..s_off + 32]);
        }

        let expected = compute_root_fn(&EMPTY_LEAF, index, &siblings);
        assert!(expected == current_root);

        current_root = compute_root_fn(&leaf_hash, index, &siblings);
    }

    current_root
}

// Cycle cost per entry: ~230K (SHA-256), TBD (Blake3)
// max_trace_length must be next power of 2 above total cycles.
//   1 entry  →    244K cycles → 2^22 (4.2M)   → ~4.5s prove,  <10 GB RAM
//  10 entries →   2.3M cycles → 2^22 (4.2M)   → ~23s prove,   <10 GB RAM
// 100 entries →  23.2M cycles → 2^25 (33.5M)  → needs ~32 GB RAM
#[jolt::provable(max_trace_length = 4194304, stack_size = 65536, max_input_size = 65536)]
fn transition(
    old_root: [u8; 32],
    old_nonce: u64,
    input_nonce: u64,
    batch_data: Vec<u8>,
    num_entries: u32,
) -> [u8; 32] {
    verify_batch(old_root, old_nonce, input_nonce, &batch_data, num_entries, compute_root_sha256)
}

// Blake3: ~50K cycles/entry. 100 entries = 5.4M → needs 2^23 (8.4M), <10 GB RAM
#[jolt::provable(max_trace_length = 8388608, stack_size = 65536, max_input_size = 65536)]
fn transition_blake3(
    old_root: [u8; 32],
    old_nonce: u64,
    input_nonce: u64,
    batch_data: Vec<u8>,
    num_entries: u32,
) -> [u8; 32] {
    verify_batch(old_root, old_nonce, input_nonce, &batch_data, num_entries, compute_root_blake3)
}

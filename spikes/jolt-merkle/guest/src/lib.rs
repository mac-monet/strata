// Jolt guest: proves nonce verification + merkle tree update.
//
// Using guest-std mode for Vec params. Production would use no_std + UntrustedAdvice.

use jolt_inlines_sha2::Sha256;

const DEPTH: usize = 16;
const EMPTY_LEAF: [u8; 32] = [0u8; 32];

/// Per-entry serialized size: 32 (leaf) + 4 (index) + DEPTH * 32 (siblings)
const ENTRY_SIZE: usize = 32 + 4 + DEPTH * 32;

#[inline(always)]
fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    Sha256::digest(&buf)
}

fn compute_root(leaf: &[u8; 32], index: u32, siblings: &[[u8; 32]; DEPTH]) -> [u8; 32] {
    let mut current = *leaf;
    let mut idx = index;
    for i in 0..DEPTH {
        if idx & 1 == 0 {
            current = hash_pair(&current, &siblings[i]);
        } else {
            current = hash_pair(&siblings[i], &current);
        }
        idx >>= 1;
    }
    current
}

// Cycle cost: ~230K per entry (32 SHA-256 hashes each).
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
    // Nonce check
    assert!(input_nonce == old_nonce + 1);

    let n = num_entries as usize;
    assert!(batch_data.len() == n * ENTRY_SIZE);

    let mut current_root = old_root;

    for i in 0..n {
        let off = i * ENTRY_SIZE;

        // Parse leaf hash
        let mut leaf_hash = [0u8; 32];
        leaf_hash.copy_from_slice(&batch_data[off..off + 32]);

        // Parse leaf index
        let index = u32::from_le_bytes([
            batch_data[off + 32],
            batch_data[off + 33],
            batch_data[off + 34],
            batch_data[off + 35],
        ]);

        // Parse sibling hashes
        let mut siblings = [[0u8; 32]; DEPTH];
        for j in 0..DEPTH {
            let s_off = off + 36 + j * 32;
            siblings[j].copy_from_slice(&batch_data[s_off..s_off + 32]);
        }

        // Verify old position was empty
        let expected = compute_root(&EMPTY_LEAF, index, &siblings);
        assert!(expected == current_root);

        // Compute new root with inserted leaf
        current_root = compute_root(&leaf_hash, index, &siblings);
    }

    current_root
}

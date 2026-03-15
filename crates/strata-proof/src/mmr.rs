use alloc::vec::Vec;

use crate::error::TransitionError;
use crate::hasher::{GuestHasher, compute_root, leaf_digest, node_digest};

/// Convert a 0-based leaf index to its MMR node position.
///
/// `pos = 2*N - popcount(N)`
pub fn leaf_position(leaf_index: u64) -> u64 {
    2 * leaf_index - leaf_index.count_ones() as u64
}

/// Total number of nodes in an MMR with `leaf_count` leaves.
///
/// `size = 2*N - popcount(N)`
///
/// Note: `mmr_size(N) == leaf_position(N)` by design — the size of an MMR
/// with N leaves equals the position where the (N+1)th leaf would go.
pub fn mmr_size(leaf_count: u64) -> u64 {
    2 * leaf_count - leaf_count.count_ones() as u64
}

/// Simulate MMR appends in place, updating peaks and leaf count.
///
/// Each entry is appended as a leaf, then merged with existing peaks
/// according to the MMR structure (number of merges = trailing 1-bits
/// of the old leaf count).
pub fn simulate_appends<H: GuestHasher>(
    peaks: &mut Vec<[u8; 32]>,
    leaf_count: &mut u64,
    entries_bytes: &[Vec<u8>],
) {
    for entry_bytes in entries_bytes {
        let leaf_pos = mmr_size(*leaf_count);
        let leaf = leaf_digest::<H>(leaf_pos, entry_bytes);

        let merges = leaf_count.trailing_ones();

        let mut current = leaf;
        let mut next_pos = leaf_pos + 1;

        for _ in 0..merges {
            let left = peaks.pop().expect("peak must exist for merge");
            current = node_digest::<H>(next_pos, &left, &current);
            next_pos += 1;
        }

        peaks.push(current);
        *leaf_count += 1;
    }
}

/// Verify an MMR append sequence and return the new root.
///
/// 1. Validates that peak count matches `old_leaf_count.count_ones()`
/// 2. Reconstructs the old root from peaks, verifies it matches `expected_old_root`
/// 3. Simulates appending `entries_bytes` to produce new peaks
/// 4. Returns the new root digest
pub fn verify_append<H: GuestHasher>(
    old_peaks: &[[u8; 32]],
    old_leaf_count: u64,
    expected_old_root: &[u8; 32],
    entries_bytes: &[Vec<u8>],
) -> Result<[u8; 32], TransitionError> {
    // Validate peak count
    let expected_peak_count = old_leaf_count.count_ones();
    if old_peaks.len() != expected_peak_count as usize {
        return Err(TransitionError::InvalidPeakCount {
            expected: expected_peak_count,
            actual: old_peaks.len(),
        });
    }

    // Reconstruct old root, verify match
    let old_root = compute_root::<H>(old_leaf_count, old_peaks);
    if old_root != *expected_old_root {
        return Err(TransitionError::OldRootMismatch);
    }

    // Simulate appends
    let mut peaks = old_peaks.to_vec();
    let mut count = old_leaf_count;
    simulate_appends::<H>(&mut peaks, &mut count, entries_bytes);

    // Compute and return new root
    Ok(compute_root::<H>(count, &peaks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Keccak256Hasher;

    #[test]
    fn leaf_position_known_values() {
        assert_eq!(leaf_position(0), 0);
        assert_eq!(leaf_position(1), 1);
        assert_eq!(leaf_position(2), 3);
        assert_eq!(leaf_position(3), 4);
        assert_eq!(leaf_position(4), 7);
        assert_eq!(leaf_position(5), 8);
        assert_eq!(leaf_position(6), 10);
        assert_eq!(leaf_position(7), 11);
    }

    #[test]
    fn mmr_size_known_values() {
        assert_eq!(mmr_size(0), 0);
        assert_eq!(mmr_size(1), 1);
        assert_eq!(mmr_size(2), 3);
        assert_eq!(mmr_size(3), 4);
        assert_eq!(mmr_size(4), 7);
        assert_eq!(mmr_size(5), 8);
        assert_eq!(mmr_size(6), 10);
        assert_eq!(mmr_size(7), 11);
    }

    #[test]
    fn mmr_size_equals_leaf_position() {
        for i in 0..100 {
            assert_eq!(mmr_size(i), leaf_position(i));
        }
    }

    #[test]
    fn empty_root_is_hash_of_zero_leaf_count() {
        let root = compute_root::<Keccak256Hasher>(0, &[]);
        // Empty root = Keccak256(0u64_be_bytes) = Keccak256([0u8; 8])
        assert_ne!(root, [0u8; 32], "empty root should not be all zeros");
    }

    #[test]
    fn single_leaf_append() {
        let mut peaks = Vec::new();
        let mut count = 0u64;
        let entry = b"test entry";

        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &[entry.to_vec()]);

        assert_eq!(count, 1);
        assert_eq!(peaks.len(), 1); // popcount(1) = 1

        let root = compute_root::<Keccak256Hasher>(count, &peaks);
        assert_ne!(root, [0u8; 32]);
    }

    #[test]
    fn two_leaf_append_merges_to_single_peak() {
        let mut peaks = Vec::new();
        let mut count = 0u64;

        simulate_appends::<Keccak256Hasher>(
            &mut peaks,
            &mut count,
            &[b"entry0".to_vec(), b"entry1".to_vec()],
        );

        assert_eq!(count, 2);
        assert_eq!(peaks.len(), 1); // popcount(2) = 1
    }

    #[test]
    fn three_leaf_append_has_two_peaks() {
        let mut peaks = Vec::new();
        let mut count = 0u64;

        simulate_appends::<Keccak256Hasher>(
            &mut peaks,
            &mut count,
            &[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
        );

        assert_eq!(count, 3);
        assert_eq!(peaks.len(), 2); // popcount(3) = 2
    }

    #[test]
    fn verify_append_rejects_wrong_peak_count() {
        let old_root = compute_root::<Keccak256Hasher>(0, &[]);

        // Provide 1 peak for an empty MMR (should be 0)
        let result = verify_append::<Keccak256Hasher>(
            &[[0u8; 32]],
            0,
            &old_root,
            &[b"entry".to_vec()],
        );

        assert_eq!(
            result,
            Err(TransitionError::InvalidPeakCount {
                expected: 0,
                actual: 1,
            })
        );
    }

    #[test]
    fn verify_append_rejects_wrong_old_root() {
        let wrong_root = [0xFFu8; 32];

        let result = verify_append::<Keccak256Hasher>(
            &[],
            0,
            &wrong_root,
            &[b"entry".to_vec()],
        );

        assert_eq!(result, Err(TransitionError::OldRootMismatch));
    }

    #[test]
    fn verify_append_from_empty() {
        let empty_root = compute_root::<Keccak256Hasher>(0, &[]);
        let entries = vec![b"alpha".to_vec(), b"beta".to_vec()];

        let new_root = verify_append::<Keccak256Hasher>(&[], 0, &empty_root, &entries).unwrap();

        // Build the same root manually
        let mut peaks = Vec::new();
        let mut count = 0u64;
        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &entries);
        let expected = compute_root::<Keccak256Hasher>(count, &peaks);

        assert_eq!(new_root, expected);
    }

    #[test]
    fn verify_append_incremental() {
        // Build initial state with 2 entries
        let mut peaks = Vec::new();
        let mut count = 0u64;
        let first_batch = vec![b"a".to_vec(), b"b".to_vec()];
        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &first_batch);
        let mid_root = compute_root::<Keccak256Hasher>(count, &peaks);

        // Verify appending a second batch
        let second_batch = vec![b"c".to_vec()];
        let new_root =
            verify_append::<Keccak256Hasher>(&peaks, count, &mid_root, &second_batch).unwrap();

        // Build expected from scratch
        let mut full_peaks = Vec::new();
        let mut full_count = 0u64;
        let all = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        simulate_appends::<Keccak256Hasher>(&mut full_peaks, &mut full_count, &all);
        let expected = compute_root::<Keccak256Hasher>(full_count, &full_peaks);

        assert_eq!(new_root, expected);
    }
}

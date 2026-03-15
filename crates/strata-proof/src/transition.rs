use alloc::vec::Vec;
use commonware_codec::Encode;

use crate::error::TransitionError;
use crate::hasher::GuestHasher;
use crate::mmr::verify_append;
use crate::witness::Witness;
use strata_core::{CoreState, Nonce, VectorRoot};

/// Pure transition function for ZK guest verification.
///
/// Verifies nonce and MMR append correctness, returning the new [`CoreState`].
/// Signature verification is an application-level concern handled outside the proof.
pub fn transition<H: GuestHasher>(
    state: CoreState,
    nonce: Nonce,
    witness: &Witness,
) -> Result<CoreState, TransitionError> {
    // 1. Verify nonce
    let expected_nonce = state.nonce.next();
    if nonce != expected_nonce {
        return Err(TransitionError::InvalidNonce {
            expected: expected_nonce,
            actual: nonce,
        });
    }

    // 2. Serialize new entries
    let entries_bytes: Vec<Vec<u8>> = witness
        .new_entries
        .iter()
        .map(|e| e.encode().to_vec())
        .collect();

    // 3. Verify MMR append
    let new_root_bytes = verify_append::<H>(
        &witness.old_peaks,
        witness.old_leaf_count,
        state.vector_index_root.as_bytes(),
        &entries_bytes,
    )?;

    // 4. Return new state
    Ok(CoreState {
        soul_hash: state.soul_hash,
        vector_index_root: VectorRoot::new(new_root_bytes),
        nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Keccak256Hasher, Witness};
    use alloc::vec;
    use strata_core::{BinaryEmbedding, ContentHash, MemoryEntry, MemoryId, SoulHash};

    fn sample_entry(id: u64, text: &[u8]) -> MemoryEntry {
        MemoryEntry::new(
            MemoryId::new(id),
            BinaryEmbedding::new([id, id + 1, id + 2, id + 3]),
            ContentHash::digest(text),
        )
    }

    fn genesis_root() -> [u8; 32] {
        crate::compute_root::<Keccak256Hasher>(0, &[])
    }

    #[test]
    fn valid_transition_from_genesis() {
        let initial_root = genesis_root();

        let state = CoreState {
            soul_hash: SoulHash::digest(b"test-soul"),
            vector_index_root: VectorRoot::new(initial_root),
            nonce: Nonce::new(0),
        };

        let entries = vec![sample_entry(0, b"hello")];

        let witness = Witness {
            old_peaks: vec![],
            old_leaf_count: 0,
            new_entries: entries,
        };

        let new_state =
            transition::<Keccak256Hasher>(state, Nonce::new(1), &witness).unwrap();

        assert_eq!(new_state.soul_hash, state.soul_hash);
        assert_eq!(new_state.nonce, Nonce::new(1));
        assert_ne!(new_state.vector_index_root, state.vector_index_root);
    }

    #[test]
    fn rejects_wrong_nonce() {
        let state = CoreState {
            soul_hash: SoulHash::digest(b"soul"),
            vector_index_root: VectorRoot::new(genesis_root()),
            nonce: Nonce::new(5),
        };

        let witness = Witness {
            old_peaks: vec![],
            old_leaf_count: 0,
            new_entries: vec![],
        };

        let result = transition::<Keccak256Hasher>(state, Nonce::new(1), &witness);
        assert_eq!(
            result,
            Err(TransitionError::InvalidNonce {
                expected: Nonce::new(6),
                actual: Nonce::new(1),
            })
        );
    }

    #[test]
    fn rejects_old_root_mismatch() {
        let state = CoreState {
            soul_hash: SoulHash::digest(b"soul"),
            vector_index_root: VectorRoot::new([0xAA; 32]),
            nonce: Nonce::new(0),
        };

        let witness = Witness {
            old_peaks: vec![],
            old_leaf_count: 0,
            new_entries: vec![sample_entry(0, b"data")],
        };

        let result = transition::<Keccak256Hasher>(state, Nonce::new(1), &witness);
        assert_eq!(result, Err(TransitionError::OldRootMismatch));
    }
}

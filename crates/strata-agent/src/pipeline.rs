//! State transition pipeline: snapshot before interaction, finalize after.
//!
//! Captures VectorDB state before an agent interaction (`snapshot`), then after
//! all `remember` calls are complete, packages the diff into a `TransitionRecord`,
//! `Witness`, and 112-byte public values, verified locally via `strata_proof::transition`.

use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use strata_core::{
    CoreState, Input, InputPayload, InputSignature, MemoryContent, MemoryId, TransitionRecord,
};
use strata_proof::{Keccak256Hasher, Witness};
use strata_vector_db::VectorDB;

use crate::error::AgentError;

/// Snapshot of agent state before an interaction.
pub struct Snapshot {
    pub state: CoreState,
    pub leaf_count: u64,
    pub peaks: Vec<[u8; 32]>,
}

/// Output of a finalized transition.
#[derive(Debug)]
pub struct TransitionOutput {
    pub record: TransitionRecord,
    pub witness: Witness,
    pub old_state: CoreState,
    pub new_state: CoreState,
}

/// Capture a snapshot of the current state and VectorDB before an interaction.
pub fn snapshot<E: RStorage + Clock + Metrics>(state: CoreState, db: &VectorDB<E>) -> Snapshot {
    Snapshot {
        state,
        leaf_count: db.len(),
        peaks: db.peak_digests(),
    }
}

/// Finalize a transition after an interaction. Collects new entries from the DB,
/// builds a `TransitionRecord` and `Witness`, verifies locally, and returns the output.
///
/// `contents` must be the full content history (indexed by `MemoryId`), matching
/// `ToolExecutor::contents()`. Entries at `snap.leaf_count..db.len()` are used.
pub fn finalize<E: RStorage + Clock + Metrics>(
    snap: &Snapshot,
    db: &VectorDB<E>,
    contents: &[String],
) -> Result<TransitionOutput, AgentError> {
    let new_len = db.len();
    if new_len <= snap.leaf_count {
        return Err(AgentError::Pipeline("no new memories to commit".into()));
    }

    if (contents.len() as u64) < new_len {
        return Err(AgentError::Pipeline(format!(
            "contents length {} < db length {}",
            contents.len(),
            new_len,
        )));
    }

    // Collect new entries from the DB
    let new_entries: Vec<_> = (snap.leaf_count..new_len)
        .map(|i| {
            db.get(MemoryId::new(i)).ok_or_else(|| {
                AgentError::Pipeline(format!("missing entry for MemoryId({i})"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .copied()
        .collect();

    // Build MemoryContent for each new entry
    let new_contents: Vec<_> = (snap.leaf_count as usize..new_len as usize)
        .map(|i| {
            let text = &contents[i];
            MemoryContent::new(MemoryId::new(i as u64), text.as_bytes().to_vec())
        })
        .collect();

    // Build input
    let new_nonce = snap.state.nonce.next();
    let input = Input::new(new_nonce, InputPayload::MemoryUpdate, InputSignature::default());

    // Build and validate the transition record
    let record = TransitionRecord::new(input, new_entries.clone(), new_contents);
    record
        .validate()
        .map_err(|e| AgentError::Pipeline(format!("record validation failed: {e}")))?;

    // Build witness
    let witness = Witness {
        old_peaks: snap.peaks.clone(),
        old_leaf_count: snap.leaf_count,
        new_entries,
    };

    // Local verification via strata_proof
    let new_state = strata_proof::transition::<Keccak256Hasher>(snap.state, new_nonce, &witness)
        .map_err(|e| AgentError::Pipeline(format!("transition verification failed: {e}")))?;

    // Verify the new root matches the DB
    if new_state.vector_index_root != db.root() {
        return Err(AgentError::Pipeline(
            "new state root does not match DB root".into(),
        ));
    }

    Ok(TransitionOutput {
        record,
        witness,
        old_state: snap.state,
        new_state,
    })
}

/// Build 112-byte public values for a batch of transitions.
///
/// Layout: `old_root[32] || new_root[32] || start_nonce[8] || end_nonce[8] || soul_hash[32]`
pub fn batch_public_values(
    old_root: &[u8; 32],
    new_root: &[u8; 32],
    start_nonce: u64,
    end_nonce: u64,
    soul_hash: &[u8; 32],
) -> [u8; 112] {
    let mut pv = [0u8; 112];
    pv[0..32].copy_from_slice(old_root);
    pv[32..64].copy_from_slice(new_root);
    pv[64..72].copy_from_slice(&start_nonce.to_be_bytes());
    pv[72..80].copy_from_slice(&end_nonce.to_be_bytes());
    pv[80..112].copy_from_slice(soul_hash);
    pv
}

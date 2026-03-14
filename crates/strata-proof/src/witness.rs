use alloc::vec::Vec;
use strata_core::MemoryEntry;

/// Witness data for guest transition verification.
///
/// Provides the peak digests and leaf count of the old MMR state,
/// plus the new entries to append. The host constructs this from
/// its MMR state and passes it to the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Witness {
    /// Peak digests of the old MMR, in decreasing height order.
    pub old_peaks: Vec<[u8; 32]>,
    /// Number of leaves in the old MMR.
    pub old_leaf_count: u64,
    /// New entries to append in this transition.
    pub new_entries: Vec<MemoryEntry>,
}

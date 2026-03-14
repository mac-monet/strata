/// Errors from VectorDB operations.
#[derive(Debug)]
pub enum VectorDbError {
    MmrInit(String),
    MmrApply(String),
    ProofGeneration(String),
    SyncFailed(String),
    IndexMismatch { entries: u64, mmr_leaves: u64 },
    NoNewEntries,
}

impl core::fmt::Display for VectorDbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MmrInit(e) => write!(f, "MMR init failed: {e}"),
            Self::MmrApply(e) => write!(f, "MMR apply failed: {e}"),
            Self::ProofGeneration(e) => write!(f, "proof generation failed: {e}"),
            Self::SyncFailed(e) => write!(f, "sync failed: {e}"),
            Self::IndexMismatch { entries, mmr_leaves } => write!(
                f,
                "index/MMR mismatch: {entries} entries provided but MMR has {mmr_leaves} leaves"
            ),
            Self::NoNewEntries => write!(f, "no new entries since old leaf count"),
        }
    }
}

impl std::error::Error for VectorDbError {}

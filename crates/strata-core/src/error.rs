use crate::{MemoryId, Nonce};
use core::fmt;

/// Schema-level validation failures for canonical transition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// The stored operator public key bytes cannot be decoded as ed25519.
    MalformedOperatorKey,
    /// The signed nonce does not match the next expected state nonce.
    InvalidNonce { expected: Nonce, actual: Nonce },
    /// The input signature does not verify against the authorized operator key.
    InvalidSignature,
    /// New memory entries must be strictly increasing by `MemoryId`.
    NewEntriesOutOfOrder,
    /// New memory entries must be active when first appended.
    InactiveNewEntry { memory_id: MemoryId },
    /// Deactivated ids must be strictly increasing.
    DeactivatedIdsOutOfOrder,
    /// A transition cannot add and deactivate the same memory id.
    ConflictingMemoryId { memory_id: MemoryId },
    /// Every new memory entry must have exactly one corresponding content blob.
    ContentCountMismatch { entries: usize, contents: usize },
    /// Content blobs must line up one-for-one with the new entry ids.
    ContentIdMismatch {
        expected: MemoryId,
        actual: MemoryId,
    },
    /// The posted content does not hash to the committed `content_hash`.
    ContentHashMismatch { memory_id: MemoryId },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedOperatorKey => f.write_str("operator key is not valid ed25519"),
            Self::InvalidNonce { expected, actual } => {
                write!(
                    f,
                    "invalid nonce: expected {:?}, got {:?}",
                    expected, actual
                )
            }
            Self::InvalidSignature => f.write_str("input signature did not verify"),
            Self::NewEntriesOutOfOrder => {
                f.write_str("new memory entries must be strictly increasing by id")
            }
            Self::InactiveNewEntry { memory_id } => {
                write!(f, "new memory entry {:?} must start active", memory_id)
            }
            Self::DeactivatedIdsOutOfOrder => {
                f.write_str("deactivated ids must be strictly increasing")
            }
            Self::ConflictingMemoryId { memory_id } => {
                write!(
                    f,
                    "memory {:?} cannot be added and deactivated together",
                    memory_id
                )
            }
            Self::ContentCountMismatch { entries, contents } => write!(
                f,
                "content count mismatch: expected {entries} content blobs, got {contents}"
            ),
            Self::ContentIdMismatch { expected, actual } => write!(
                f,
                "content id mismatch: expected {:?}, got {:?}",
                expected, actual
            ),
            Self::ContentHashMismatch { memory_id } => {
                write!(f, "content hash mismatch for memory {:?}", memory_id)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValidationError {}

use crate::{MemoryId, Nonce};
use core::fmt;

/// Schema-level validation failures for canonical transition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// The signed nonce does not match the next expected state nonce.
    InvalidNonce { expected: Nonce, actual: Nonce },
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
            Self::InvalidNonce { expected, actual } => {
                write!(
                    f,
                    "invalid nonce: expected {:?}, got {:?}",
                    expected, actual
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

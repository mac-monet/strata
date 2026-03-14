use core::fmt;
use strata_core::Nonce;

/// Errors from guest transition verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransitionError {
    InvalidNonce { expected: Nonce, actual: Nonce },
    OldRootMismatch,
    InvalidPeakCount { expected: u32, actual: usize },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNonce { expected, actual } => {
                write!(f, "invalid nonce: expected {expected:?}, got {actual:?}")
            }
            Self::OldRootMismatch => f.write_str("witness old root does not match state root"),
            Self::InvalidPeakCount { expected, actual } => {
                write!(f, "invalid peak count: expected {expected}, got {actual}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TransitionError {}

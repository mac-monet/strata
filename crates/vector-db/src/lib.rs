#![forbid(unsafe_code)]

mod db;
mod error;
pub mod keccak;
mod query;

pub use db::{VectorDB, VectorDBWitness};
pub use error::VectorDbError;
pub use query::{QueryResult, hamming_distance};

// Re-export journaled::Config for callers to construct.
pub use commonware_storage::mmr::journaled::Config;

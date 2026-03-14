#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

mod error;
mod primitives;
mod types;

pub use error::ValidationError;
pub use primitives::{
    BinaryEmbedding, ContentHash, EMBEDDING_WORDS, InputSignature, MemoryId, Nonce,
    OperatorPublicKey, SoulHash, VectorRoot,
};
pub use types::{
    CoreState, GenesisConfig, Input, InputPayload, MemoryContent, MemoryContentCfg, MemoryEntry,
    TransitionRecord, TransitionRecordCfg,
};

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

mod error;
mod hasher;
mod mmr;
mod transition;
mod witness;

pub use error::TransitionError;
pub use hasher::Keccak256Hasher;
pub use hasher::{GuestHasher, compute_root, leaf_digest, node_digest};
pub use mmr::{leaf_position, mmr_size, simulate_appends, verify_append};
pub use transition::transition;
pub use witness::Witness;

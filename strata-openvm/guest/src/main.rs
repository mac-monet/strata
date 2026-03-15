#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;

use strata_core::{CoreState, Nonce};
use strata_proof::{GuestHasher, Witness};

openvm::entry!(main);

/// Keccak256 hasher using OpenVM's native precompile.
///
/// Buffers `update()` calls, hashes on `finalize()` via the precompile.
struct OpenVmKeccak {
    buf: Vec<u8>,
}

impl GuestHasher for OpenVmKeccak {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn update(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn finalize(&mut self) -> [u8; 32] {
        let hash = openvm_keccak256_guest::keccak256(&self.buf);
        self.buf.clear();
        hash
    }
}

/// Public values layout (104 bytes, fixed offsets):
///   [0..32]   oldRoot   — vector_index_root before transition
///   [32..64]  newRoot   — vector_index_root after transition
///   [64..72]  nonce     — new nonce (u64 big-endian)
///   [72..104] soulHash  — soul document hash (carried through unchanged)
fn main() {
    let state: CoreState = openvm::io::read();
    let nonce: u64 = openvm::io::read();
    let witness: Witness = openvm::io::read();

    let old_root: [u8; 32] = *state.vector_index_root.as_bytes();
    let soul_hash: [u8; 32] = *state.soul_hash.as_bytes();

    let new_state = strata_proof::transition::<OpenVmKeccak>(
        state,
        Nonce::new(nonce),
        &witness,
    )
    .expect("transition failed");

    let new_root: [u8; 32] = *new_state.vector_index_root.as_bytes();
    let new_nonce: u64 = new_state.nonce.get();

    // Reveal full state transition as public output.
    openvm::io::reveal_to_host(&old_root);
    openvm::io::reveal_to_host(&new_root);
    openvm::io::reveal_to_host(&new_nonce.to_be_bytes());
    openvm::io::reveal_to_host(&soul_hash);
}

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
        let mut out = [0u8; 32];
        openvm_keccak256_guest::native_keccak256(
            self.buf.as_ptr(),
            self.buf.len(),
            out.as_mut_ptr(),
        );
        self.buf.clear();
        out
    }
}

/// Reveal a byte slice as public values starting at the given u32-word index.
///
/// Each 4-byte chunk is written as a LE u32 at the corresponding word position.
/// The byte slice length must be a multiple of 4.
fn reveal_bytes(data: &[u8], word_index: usize) {
    for (i, chunk) in data.chunks_exact(4).enumerate() {
        let x = u32::from_le_bytes(chunk.try_into().unwrap());
        openvm::io::reveal_u32(x, word_index + i);
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

    // Reveal full state transition as public output (104 bytes).
    reveal_bytes(&old_root, 0);                      // [0..32]   word indices 0..8
    reveal_bytes(&new_root, 8);                      // [32..64]  word indices 8..16
    reveal_bytes(&new_nonce.to_be_bytes(), 16);      // [64..72]  word indices 16..18
    reveal_bytes(&soul_hash, 18);                    // [72..104] word indices 18..26

    // Zero-fill remaining words to constrain the full public values buffer.
    for i in 26..32 {
        openvm::io::reveal_u32(0, i);
    }
}

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
/// Each 4-byte chunk is written as a BE u32 at the corresponding word position.
/// The byte slice length must be a multiple of 4.
fn reveal_bytes(data: &[u8], word_index: usize) {
    for (i, chunk) in data.chunks_exact(4).enumerate() {
        let x = u32::from_be_bytes(chunk.try_into().unwrap());
        openvm::io::reveal_u32(x, word_index + i);
    }
}

/// Public values layout (112 bytes, fixed offsets):
///   [0..32]   oldRoot     — vector_index_root before first transition
///   [32..64]  newRoot     — vector_index_root after last transition
///   [64..72]  startNonce  — first nonce in batch (u64 big-endian)
///   [72..80]  endNonce    — last nonce in batch (u64 big-endian)
///   [80..112] soulHash    — soul document hash (carried through unchanged)
fn main() {
    let mut state: CoreState = openvm::io::read();
    let count: u64 = openvm::io::read();

    let old_root: [u8; 32] = *state.vector_index_root.as_bytes();
    let soul_hash: [u8; 32] = *state.soul_hash.as_bytes();

    let mut start_nonce: u64 = 0;
    let mut end_nonce: u64 = 0;

    for i in 0..count {
        let nonce: u64 = openvm::io::read();
        let witness: Witness = openvm::io::read();

        if i == 0 {
            start_nonce = nonce;
        }
        end_nonce = nonce;

        state = strata_proof::transition::<OpenVmKeccak>(
            state,
            Nonce::new(nonce),
            &witness,
        )
        .expect("transition failed");
    }

    let new_root: [u8; 32] = *state.vector_index_root.as_bytes();

    // Reveal full batch transition as public output (112 bytes).
    reveal_bytes(&old_root, 0);                          // [0..32]   word indices 0..8
    reveal_bytes(&new_root, 8);                          // [32..64]  word indices 8..16
    reveal_bytes(&start_nonce.to_be_bytes(), 16);        // [64..72]  word indices 16..18
    reveal_bytes(&end_nonce.to_be_bytes(), 18);          // [72..80]  word indices 18..20
    reveal_bytes(&soul_hash, 20);                        // [80..112] word indices 20..28

    // Zero-fill remaining words to constrain the full public values buffer.
    for i in 28..32 {
        openvm::io::reveal_u32(0, i);
    }
}

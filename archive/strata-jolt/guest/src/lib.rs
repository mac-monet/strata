use strata_core::{CoreState, Nonce};
use strata_proof::Witness;

/// Blake3 implementation of GuestHasher using the blake3 crate directly.
///
/// Avoids pulling in commonware-cryptography (which has RISC-V unfriendly C deps).
struct JoltBlake3 {
    hasher: blake3::Hasher,
}

impl strata_proof::GuestHasher for JoltBlake3 {
    fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    fn finalize(&mut self) -> [u8; 32] {
        *self.hasher.finalize().as_bytes()
    }
}

#[jolt::provable(max_trace_length = 65536, stack_size = 65536, heap_size = 1048576)]
fn verify_transition(
    state: CoreState,
    nonce: u64,
    advice: jolt::UntrustedAdvice<Witness>,
) -> CoreState {
    let witness = &*advice;
    strata_proof::transition::<JoltBlake3>(state, Nonce::new(nonce), witness)
        .expect("transition verification failed")
}

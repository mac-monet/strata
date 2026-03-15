/// Minimal hash abstraction for guest transition logic.
///
/// Avoids `commonware_cryptography::Hasher`'s `Default + Clone + Send + Sync`
/// bounds that may not hold for ZK VM inline hash implementations.
pub trait GuestHasher {
    fn new() -> Self;
    fn update(&mut self, data: &[u8]);
    fn finalize(&mut self) -> [u8; 32];
}

/// Keccak256 implementation of [`GuestHasher`] wrapping `alloy_primitives::Keccak256`.
pub struct Keccak256Hasher {
    inner: alloy_primitives::Keccak256,
}

impl GuestHasher for Keccak256Hasher {
    fn new() -> Self {
        Self {
            inner: alloy_primitives::Keccak256::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    fn finalize(&mut self) -> [u8; 32] {
        let hasher = core::mem::replace(&mut self.inner, alloy_primitives::Keccak256::new());
        hasher.finalize().into()
    }
}

/// Compute a leaf digest matching `StandardHasher::leaf_digest`.
///
/// Layout: `Hash(pos_u64_be ++ element_bytes)`
pub fn leaf_digest<H: GuestHasher>(pos: u64, element: &[u8]) -> [u8; 32] {
    let mut h = H::new();
    h.update(&pos.to_be_bytes());
    h.update(element);
    h.finalize()
}

/// Compute a node digest matching `StandardHasher::node_digest`.
///
/// Layout: `Hash(pos_u64_be ++ left_32 ++ right_32)`
pub fn node_digest<H: GuestHasher>(pos: u64, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = H::new();
    h.update(&pos.to_be_bytes());
    h.update(left);
    h.update(right);
    h.finalize()
}

/// Compute a root digest matching `StandardHasher::root`.
///
/// Layout: `Hash(leaf_count_u64_be ++ peak1 ++ peak2 ++ ...)`
pub fn compute_root<H: GuestHasher>(leaf_count: u64, peaks: &[[u8; 32]]) -> [u8; 32] {
    let mut h = H::new();
    h.update(&leaf_count.to_be_bytes());
    for peak in peaks {
        h.update(peak);
    }
    h.finalize()
}

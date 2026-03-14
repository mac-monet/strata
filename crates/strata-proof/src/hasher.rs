/// Minimal hash abstraction for guest transition logic.
///
/// Avoids `commonware_cryptography::Hasher`'s `Default + Clone + Send + Sync`
/// bounds that may not hold for Jolt inline hash implementations.
pub trait GuestHasher {
    fn new() -> Self;
    fn update(&mut self, data: &[u8]);
    fn finalize(&mut self) -> [u8; 32];
}

/// Blake3 implementation of [`GuestHasher`] wrapping `commonware_cryptography::blake3::Blake3`.
#[cfg(feature = "blake3")]
pub struct Blake3Hasher {
    inner: commonware_cryptography::blake3::Blake3,
}

#[cfg(feature = "blake3")]
impl GuestHasher for Blake3Hasher {
    fn new() -> Self {
        use commonware_cryptography::Hasher as _;
        Self {
            inner: commonware_cryptography::blake3::Blake3::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        use commonware_cryptography::Hasher as _;
        self.inner.update(data);
    }

    fn finalize(&mut self) -> [u8; 32] {
        use commonware_cryptography::Hasher as _;
        let digest = self.inner.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(digest.as_ref());
        bytes
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

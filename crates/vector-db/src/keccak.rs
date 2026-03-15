//! Keccak256 implementation of the `commonware_cryptography::Hasher` trait.
//!
//! Wraps `alloy_primitives::Keccak256` to integrate with commonware's
//! `StandardHasher` for position-aware MMR hashing.

use bytes::{Buf, BufMut};
use commonware_codec::{Error as CodecError, FixedSize, Read, ReadExt, Write};
use commonware_math::algebra::Random;
use commonware_utils::{Array, Span, hex};
use core::{
    fmt::{self, Debug, Display},
    ops::Deref,
};
use rand_core::CryptoRngCore;

const DIGEST_LENGTH: usize = 32;

/// Keccak256 hasher implementing `commonware_cryptography::Hasher`.
#[derive(Debug, Default)]
pub struct Keccak256 {
    hasher: alloy_primitives::Keccak256,
}

impl Clone for Keccak256 {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl commonware_cryptography::Hasher for Keccak256 {
    type Digest = Digest;

    fn update(&mut self, message: &[u8]) -> &mut Self {
        self.hasher.update(message);
        self
    }

    fn finalize(&mut self) -> Self::Digest {
        let hasher = core::mem::replace(&mut self.hasher, alloy_primitives::Keccak256::new());
        Digest(hasher.finalize().into())
    }

    fn reset(&mut self) -> &mut Self {
        self.hasher = alloy_primitives::Keccak256::new();
        self
    }
}

/// Digest of a Keccak256 hashing operation.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct Digest(pub [u8; DIGEST_LENGTH]);

impl Write for Digest {
    fn write(&self, buf: &mut impl BufMut) {
        self.0.write(buf);
    }
}

impl Read for Digest {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let array = <[u8; DIGEST_LENGTH]>::read(buf)?;
        Ok(Self(array))
    }
}

impl FixedSize for Digest {
    const SIZE: usize = DIGEST_LENGTH;
}

impl Span for Digest {}

impl Array for Digest {}

impl From<[u8; DIGEST_LENGTH]> for Digest {
    fn from(value: [u8; DIGEST_LENGTH]) -> Self {
        Self(value)
    }
}

impl AsRef<[u8]> for Digest {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Digest {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex(&self.0))
    }
}

impl Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex(&self.0))
    }
}

impl commonware_cryptography::Digest for Digest {
    const EMPTY: Self = Self([0u8; DIGEST_LENGTH]);
}

impl Random for Digest {
    fn random(mut rng: impl CryptoRngCore) -> Self {
        let mut array = [0u8; DIGEST_LENGTH];
        rng.fill_bytes(&mut array);
        Self(array)
    }
}

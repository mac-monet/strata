use crate::ValidationError;
use bytes::{Buf, BufMut};
use commonware_codec::{DecodeExt, Error as CodecError, FixedSize, Read, ReadExt, Write};
use commonware_cryptography::{Hasher, Sha256, ed25519};

const HASH_BYTES: usize = 32;
const PUBLIC_KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;

/// Number of `u64` words in a fixed-width binary embedding.
pub const EMBEDDING_WORDS: usize = 4;

macro_rules! impl_fixed_bytes_type {
    ($name:ident, $len:expr) => {
        impl $name {
            pub const fn new(bytes: [u8; $len]) -> Self {
                Self(bytes)
            }

            pub const fn into_inner(self) -> [u8; $len] {
                self.0
            }

            pub const fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }
        }

        impl From<[u8; $len]> for $name {
            fn from(value: [u8; $len]) -> Self {
                Self(value)
            }
        }

        impl From<$name> for [u8; $len] {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl Write for $name {
            fn write(&self, buf: &mut impl BufMut) {
                self.0.write(buf);
            }
        }

        impl Read for $name {
            type Cfg = ();

            fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
                Ok(Self(<[u8; $len]>::read(buf)?))
            }
        }

        impl FixedSize for $name {
            const SIZE: usize = $len;
        }
    };
}

macro_rules! impl_fixed_u64_type {
    ($name:ident) => {
        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            pub const fn next(self) -> Self {
                Self(self.0 + 1)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl Write for $name {
            fn write(&self, buf: &mut impl BufMut) {
                self.0.write(buf);
            }
        }

        impl Read for $name {
            type Cfg = ();

            fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
                Ok(Self(u64::read(buf)?))
            }
        }

        impl FixedSize for $name {
            const SIZE: usize = u64::SIZE;
        }
    };
}

fn sha256_array(bytes: &[u8]) -> [u8; HASH_BYTES] {
    let digest = Sha256::hash(bytes);
    let mut array = [0u8; HASH_BYTES];
    array.copy_from_slice(digest.as_ref());
    array
}

fn copy_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut array = [0u8; N];
    array.copy_from_slice(bytes);
    array
}

/// SHA-256 digest of the soul document text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct SoulHash([u8; HASH_BYTES]);

impl SoulHash {
    pub fn digest(bytes: &[u8]) -> Self {
        Self(sha256_array(bytes))
    }
}

impl_fixed_bytes_type!(SoulHash, HASH_BYTES);

/// SHA-256 digest of stored memory content bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct ContentHash([u8; HASH_BYTES]);

impl ContentHash {
    pub fn digest(bytes: &[u8]) -> Self {
        Self(sha256_array(bytes))
    }
}

impl_fixed_bytes_type!(ContentHash, HASH_BYTES);

/// Commitment to the current authenticated memory index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct VectorRoot([u8; HASH_BYTES]);

impl_fixed_bytes_type!(VectorRoot, HASH_BYTES);

/// Authorized operator public key bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct OperatorPublicKey([u8; PUBLIC_KEY_BYTES]);

impl OperatorPublicKey {
    pub fn decode(self) -> Result<ed25519::PublicKey, ValidationError> {
        ed25519::PublicKey::decode(self.0.as_slice())
            .map_err(|_| ValidationError::MalformedOperatorKey)
    }
}

impl From<ed25519::PublicKey> for OperatorPublicKey {
    fn from(value: ed25519::PublicKey) -> Self {
        Self(copy_array(value.as_ref()))
    }
}

impl_fixed_bytes_type!(OperatorPublicKey, PUBLIC_KEY_BYTES);

/// Signature over a canonical `Input`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct InputSignature([u8; SIGNATURE_BYTES]);

impl InputSignature {
    pub(crate) fn decode(self) -> ed25519::Signature {
        ed25519::Signature::decode(self.0.as_slice())
            .expect("64-byte ed25519 signature bytes always decode")
    }
}

impl Default for InputSignature {
    fn default() -> Self {
        Self([0u8; SIGNATURE_BYTES])
    }
}

impl From<ed25519::Signature> for InputSignature {
    fn from(value: ed25519::Signature) -> Self {
        Self(copy_array(value.as_ref()))
    }
}

impl_fixed_bytes_type!(InputSignature, SIGNATURE_BYTES);

/// Monotonic transition counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct Nonce(u64);

impl_fixed_u64_type!(Nonce);

/// Monotonic memory identifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct MemoryId(u64);

impl_fixed_u64_type!(MemoryId);

/// Fixed-width binary embedding used for memory indexing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct BinaryEmbedding([u64; EMBEDDING_WORDS]);

impl BinaryEmbedding {
    pub const fn new(words: [u64; EMBEDDING_WORDS]) -> Self {
        Self(words)
    }

    pub const fn into_inner(self) -> [u64; EMBEDDING_WORDS] {
        self.0
    }

    pub const fn as_words(&self) -> &[u64; EMBEDDING_WORDS] {
        &self.0
    }
}

impl From<[u64; EMBEDDING_WORDS]> for BinaryEmbedding {
    fn from(value: [u64; EMBEDDING_WORDS]) -> Self {
        Self(value)
    }
}

impl From<BinaryEmbedding> for [u64; EMBEDDING_WORDS] {
    fn from(value: BinaryEmbedding) -> Self {
        value.0
    }
}

impl Write for BinaryEmbedding {
    fn write(&self, buf: &mut impl BufMut) {
        for word in self.0 {
            word.write(buf);
        }
    }
}

impl Read for BinaryEmbedding {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let mut words = [0u64; EMBEDDING_WORDS];
        for word in &mut words {
            *word = u64::read(buf)?;
        }
        Ok(Self(words))
    }
}

impl FixedSize for BinaryEmbedding {
    const SIZE: usize = EMBEDDING_WORDS * u64::SIZE;
}

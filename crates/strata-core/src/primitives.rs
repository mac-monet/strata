use bytes::{Buf, BufMut};
use commonware_codec::{Error as CodecError, FixedSize, Read, ReadExt, Write};

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

fn keccak256_array(bytes: &[u8]) -> [u8; HASH_BYTES] {
    alloy_primitives::keccak256(bytes).into()
}

#[cfg(feature = "serde")]
fn copy_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut array = [0u8; N];
    array.copy_from_slice(bytes);
    array
}

/// Keccak256 digest of the soul document text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct SoulHash([u8; HASH_BYTES]);

impl SoulHash {
    pub fn digest(bytes: &[u8]) -> Self {
        Self(keccak256_array(bytes))
    }
}

impl_fixed_bytes_type!(SoulHash, HASH_BYTES);

/// Keccak256 digest of stored memory content bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct ContentHash([u8; HASH_BYTES]);

impl ContentHash {
    pub fn digest(bytes: &[u8]) -> Self {
        Self(keccak256_array(bytes))
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

impl_fixed_bytes_type!(OperatorPublicKey, PUBLIC_KEY_BYTES);

/// Signature over a canonical `Input`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct InputSignature([u8; SIGNATURE_BYTES]);

impl Default for InputSignature {
    fn default() -> Self {
        Self([0u8; SIGNATURE_BYTES])
    }
}

impl_fixed_bytes_type!(InputSignature, SIGNATURE_BYTES);

#[cfg(feature = "serde")]
impl serde::Serialize for InputSignature {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for InputSignature {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = InputSignature;
            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                f.write_str("64 bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<InputSignature, E> {
                if v.len() != SIGNATURE_BYTES {
                    return Err(E::invalid_length(v.len(), &self));
                }
                Ok(InputSignature(copy_array(v)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<InputSignature, A::Error> {
                let mut arr = [0u8; SIGNATURE_BYTES];
                for (i, byte) in arr.iter_mut().enumerate() {
                    *byte = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                }
                Ok(InputSignature(arr))
            }
        }
        deserializer.deserialize_bytes(Visitor)
    }
}

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

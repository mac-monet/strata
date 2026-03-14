use crate::{
    BinaryEmbedding, ContentHash, InputSignature, MemoryId, Nonce, OperatorPublicKey, SoulHash,
    ValidationError, VectorRoot,
};
use alloc::vec::Vec;
use bytes::{Buf, BufMut};
use commonware_codec::{
    Encode, EncodeSize, Error as CodecError, FixedSize, RangeCfg, Read, ReadExt, Write,
};
use commonware_cryptography::{Signer as _, Verifier as _, ed25519};

/// Domain separation namespace for signed inputs.
pub const INPUT_SIGNATURE_NAMESPACE: &[u8] = b"strata/input/v1";

/// Immutable configuration fixed at genesis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GenesisConfig {
    pub soul_hash: SoulHash,
    pub operator_key: OperatorPublicKey,
    pub initial_vector_index_root: VectorRoot,
}

impl GenesisConfig {
    pub const fn new(
        soul_hash: SoulHash,
        operator_key: OperatorPublicKey,
        initial_vector_index_root: VectorRoot,
    ) -> Self {
        Self {
            soul_hash,
            operator_key,
            initial_vector_index_root,
        }
    }

    pub const fn genesis_state(&self) -> CoreState {
        CoreState {
            soul_hash: self.soul_hash,
            vector_index_root: self.initial_vector_index_root,
            nonce: Nonce::new(0),
        }
    }
}

impl Write for GenesisConfig {
    fn write(&self, buf: &mut impl BufMut) {
        self.soul_hash.write(buf);
        self.operator_key.write(buf);
        self.initial_vector_index_root.write(buf);
    }
}

impl Read for GenesisConfig {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        Ok(Self {
            soul_hash: SoulHash::read(buf)?,
            operator_key: OperatorPublicKey::read(buf)?,
            initial_vector_index_root: VectorRoot::read(buf)?,
        })
    }
}

impl FixedSize for GenesisConfig {
    const SIZE: usize = SoulHash::SIZE + OperatorPublicKey::SIZE + VectorRoot::SIZE;
}

/// Minimal canonical state committed for the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CoreState {
    pub soul_hash: SoulHash,
    pub vector_index_root: VectorRoot,
    pub nonce: Nonce,
}

impl CoreState {
    pub fn advance(
        &self,
        next_nonce: Nonce,
        next_vector_index_root: VectorRoot,
    ) -> Result<Self, ValidationError> {
        let expected = self.nonce.next();
        if next_nonce != expected {
            return Err(ValidationError::InvalidNonce {
                expected,
                actual: next_nonce,
            });
        }

        Ok(Self {
            soul_hash: self.soul_hash,
            vector_index_root: next_vector_index_root,
            nonce: next_nonce,
        })
    }
}

impl Write for CoreState {
    fn write(&self, buf: &mut impl BufMut) {
        self.soul_hash.write(buf);
        self.vector_index_root.write(buf);
        self.nonce.write(buf);
    }
}

impl Read for CoreState {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        Ok(Self {
            soul_hash: SoulHash::read(buf)?,
            vector_index_root: VectorRoot::read(buf)?,
            nonce: Nonce::read(buf)?,
        })
    }
}

impl FixedSize for CoreState {
    const SIZE: usize = SoulHash::SIZE + VectorRoot::SIZE + Nonce::SIZE;
}

/// Single memory leaf committed into the vector index MMR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryEntry {
    pub id: MemoryId,
    pub embedding: BinaryEmbedding,
    pub content_hash: ContentHash,
}

impl MemoryEntry {
    pub const fn new(
        id: MemoryId,
        embedding: BinaryEmbedding,
        content_hash: ContentHash,
    ) -> Self {
        Self {
            id,
            embedding,
            content_hash,
        }
    }
}

impl Write for MemoryEntry {
    fn write(&self, buf: &mut impl BufMut) {
        self.id.write(buf);
        self.embedding.write(buf);
        self.content_hash.write(buf);
    }
}

impl Read for MemoryEntry {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        Ok(Self {
            id: MemoryId::read(buf)?,
            embedding: BinaryEmbedding::read(buf)?,
            content_hash: ContentHash::read(buf)?,
        })
    }
}

impl FixedSize for MemoryEntry {
    const SIZE: usize = MemoryId::SIZE + BinaryEmbedding::SIZE + ContentHash::SIZE;
}

/// Canonical transition intent. MVP only supports memory updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InputPayload {
    MemoryUpdate,
}

impl Write for InputPayload {
    fn write(&self, buf: &mut impl BufMut) {
        let tag = match self {
            Self::MemoryUpdate => 0u8,
        };
        tag.write(buf);
    }
}

impl Read for InputPayload {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        match u8::read(buf)? {
            0 => Ok(Self::MemoryUpdate),
            tag => Err(CodecError::InvalidEnum(tag)),
        }
    }
}

impl FixedSize for InputPayload {
    const SIZE: usize = u8::SIZE;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SignedInput {
    nonce: Nonce,
    payload: InputPayload,
}

impl Write for SignedInput {
    fn write(&self, buf: &mut impl BufMut) {
        self.nonce.write(buf);
        self.payload.write(buf);
    }
}

impl FixedSize for SignedInput {
    const SIZE: usize = Nonce::SIZE + InputPayload::SIZE;
}

/// Signed input authorized by the operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Input {
    pub nonce: Nonce,
    pub payload: InputPayload,
    pub signature: InputSignature,
}

impl Input {
    pub fn new_signed(nonce: Nonce, payload: InputPayload, signer: &ed25519::PrivateKey) -> Self {
        let signed = SignedInput { nonce, payload };
        let message = signed.encode();
        let signature = signer.sign(INPUT_SIGNATURE_NAMESPACE, message.as_ref());

        Self {
            nonce,
            payload,
            signature: signature.into(),
        }
    }

    pub fn verify(&self, operator_key: &OperatorPublicKey) -> Result<(), ValidationError> {
        let public_key = operator_key.decode()?;
        let signed = SignedInput {
            nonce: self.nonce,
            payload: self.payload,
        };
        let message = signed.encode();

        if public_key.verify(
            INPUT_SIGNATURE_NAMESPACE,
            message.as_ref(),
            &self.signature.decode(),
        ) {
            Ok(())
        } else {
            Err(ValidationError::InvalidSignature)
        }
    }
}

impl Write for Input {
    fn write(&self, buf: &mut impl BufMut) {
        self.nonce.write(buf);
        self.payload.write(buf);
        self.signature.write(buf);
    }
}

impl Read for Input {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        Ok(Self {
            nonce: Nonce::read(buf)?,
            payload: InputPayload::read(buf)?,
            signature: InputSignature::read(buf)?,
        })
    }
}

impl FixedSize for Input {
    const SIZE: usize = Nonce::SIZE + InputPayload::SIZE + InputSignature::SIZE;
}

/// Appended content bytes for a new memory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryContent {
    pub memory_id: MemoryId,
    pub bytes: Vec<u8>,
}

impl MemoryContent {
    pub fn new(memory_id: MemoryId, bytes: Vec<u8>) -> Self {
        Self { memory_id, bytes }
    }

    pub fn content_hash(&self) -> ContentHash {
        ContentHash::digest(&self.bytes)
    }
}

impl Write for MemoryContent {
    fn write(&self, buf: &mut impl BufMut) {
        self.memory_id.write(buf);
        self.bytes.write(buf);
    }
}

impl EncodeSize for MemoryContent {
    fn encode_size(&self) -> usize {
        self.memory_id.encode_size() + self.bytes.encode_size()
    }
}

/// Decode limits for a single content blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryContentCfg {
    pub max_bytes: usize,
}

impl Default for MemoryContentCfg {
    fn default() -> Self {
        Self {
            max_bytes: u32::MAX as usize,
        }
    }
}

impl Read for MemoryContent {
    type Cfg = MemoryContentCfg;

    fn read_cfg(buf: &mut impl Buf, cfg: &Self::Cfg) -> Result<Self, CodecError> {
        Ok(Self {
            memory_id: MemoryId::read(buf)?,
            bytes: Vec::<u8>::read_cfg(buf, &(RangeCfg::new(0..=cfg.max_bytes), ()))?,
        })
    }
}

/// Canonical replay payload for one accepted transition.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransitionRecord {
    pub input: Input,
    pub new_entries: Vec<MemoryEntry>,
    pub contents: Vec<MemoryContent>,
}

impl TransitionRecord {
    pub fn new(
        input: Input,
        new_entries: Vec<MemoryEntry>,
        contents: Vec<MemoryContent>,
    ) -> Self {
        Self {
            input,
            new_entries,
            contents,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.new_entries.len() != self.contents.len() {
            return Err(ValidationError::ContentCountMismatch {
                entries: self.new_entries.len(),
                contents: self.contents.len(),
            });
        }

        for (entry, content) in self.new_entries.iter().zip(&self.contents) {
            if content.memory_id != entry.id {
                return Err(ValidationError::ContentIdMismatch {
                    expected: entry.id,
                    actual: content.memory_id,
                });
            }

            if content.content_hash() != entry.content_hash {
                return Err(ValidationError::ContentHashMismatch {
                    memory_id: entry.id,
                });
            }
        }

        Ok(())
    }

    pub fn validate_against(
        &self,
        state: &CoreState,
        operator_key: &OperatorPublicKey,
    ) -> Result<(), ValidationError> {
        let expected = state.nonce.next();
        if self.input.nonce != expected {
            return Err(ValidationError::InvalidNonce {
                expected,
                actual: self.input.nonce,
            });
        }

        self.input.verify(operator_key)?;
        self.validate()
    }

    pub fn apply(
        &self,
        state: &CoreState,
        operator_key: &OperatorPublicKey,
        next_vector_index_root: VectorRoot,
    ) -> Result<CoreState, ValidationError> {
        self.validate_against(state, operator_key)?;
        state.advance(self.input.nonce, next_vector_index_root)
    }
}

impl Write for TransitionRecord {
    fn write(&self, buf: &mut impl BufMut) {
        self.input.write(buf);
        self.new_entries.write(buf);
        self.contents.write(buf);
    }
}

impl EncodeSize for TransitionRecord {
    fn encode_size(&self) -> usize {
        self.input.encode_size() + self.new_entries.encode_size() + self.contents.encode_size()
    }
}

/// Decode limits for `TransitionRecord`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionRecordCfg {
    pub max_new_entries: usize,
    pub max_contents: usize,
    pub content: MemoryContentCfg,
}

impl TransitionRecordCfg {
    pub fn permissive() -> Self {
        Self {
            max_new_entries: u32::MAX as usize,
            max_contents: u32::MAX as usize,
            content: MemoryContentCfg::default(),
        }
    }
}

impl Read for TransitionRecord {
    type Cfg = TransitionRecordCfg;

    fn read_cfg(buf: &mut impl Buf, cfg: &Self::Cfg) -> Result<Self, CodecError> {
        Ok(Self {
            input: Input::read(buf)?,
            new_entries: Vec::<MemoryEntry>::read_cfg(
                buf,
                &(RangeCfg::new(0..=cfg.max_new_entries), ()),
            )?,
            contents: Vec::<MemoryContent>::read_cfg(
                buf,
                &(RangeCfg::new(0..=cfg.max_contents), cfg.content.clone()),
            )?,
        })
    }
}

impl Default for TransitionRecordCfg {
    fn default() -> Self {
        Self::permissive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use commonware_codec::{Decode, DecodeExt, Encode};
    use commonware_cryptography::ed25519;

    fn sample_signer() -> ed25519::PrivateKey {
        ed25519::PrivateKey::from_seed(7)
    }

    fn sample_operator_key() -> OperatorPublicKey {
        sample_signer().public_key().into()
    }

    fn sample_entry(id: u64, text: &[u8]) -> (MemoryEntry, MemoryContent) {
        let memory_id = MemoryId::new(id);
        let content = MemoryContent::new(memory_id, text.to_vec());
        let entry = MemoryEntry::new(
            memory_id,
            BinaryEmbedding::new([id, id + 1, id + 2, id + 3]),
            content.content_hash(),
        );

        (entry, content)
    }

    #[test]
    fn genesis_state_uses_genesis_config() {
        let config = GenesisConfig::new(
            SoulHash::digest(b"soul"),
            sample_operator_key(),
            VectorRoot::new([9u8; 32]),
        );

        assert_eq!(
            config.genesis_state(),
            CoreState {
                soul_hash: SoulHash::digest(b"soul"),
                vector_index_root: VectorRoot::new([9u8; 32]),
                nonce: Nonce::new(0),
            }
        );
    }

    #[test]
    fn input_signatures_round_trip_and_verify() {
        let signer = sample_signer();
        let input = Input::new_signed(Nonce::new(1), InputPayload::MemoryUpdate, &signer);
        let encoded = input.encode();
        let decoded = Input::decode(encoded).unwrap();

        assert_eq!(decoded, input);
        assert_eq!(decoded.verify(&signer.public_key().into()), Ok(()));
    }

    #[test]
    fn transition_record_round_trips() {
        let signer = sample_signer();
        let input = Input::new_signed(Nonce::new(1), InputPayload::MemoryUpdate, &signer);
        let (entry_a, content_a) = sample_entry(1, b"alpha");
        let (entry_b, content_b) = sample_entry(2, b"beta");
        let record = TransitionRecord::new(
            input,
            vec![entry_a, entry_b],
            vec![content_a, content_b],
        );

        let encoded = record.encode();
        let decoded =
            TransitionRecord::decode_cfg(encoded, &TransitionRecordCfg::default()).unwrap();

        assert_eq!(decoded, record);
        assert_eq!(decoded.validate(), Ok(()));
    }

    #[test]
    fn transition_record_rejects_hash_mismatch() {
        let signer = sample_signer();
        let input = Input::new_signed(Nonce::new(1), InputPayload::MemoryUpdate, &signer);
        let (entry, _) = sample_entry(1, b"alpha");
        let wrong_content = MemoryContent::new(MemoryId::new(1), b"not alpha".to_vec());
        let record = TransitionRecord::new(input, vec![entry], vec![wrong_content]);

        assert_eq!(
            record.validate(),
            Err(ValidationError::ContentHashMismatch {
                memory_id: MemoryId::new(1)
            })
        );
    }

    #[test]
    fn apply_advances_state_when_transition_is_valid() {
        let signer = sample_signer();
        let operator_key: OperatorPublicKey = signer.public_key().into();
        let initial_state = CoreState {
            soul_hash: SoulHash::digest(b"soul"),
            vector_index_root: VectorRoot::new([0u8; 32]),
            nonce: Nonce::new(0),
        };
        let input = Input::new_signed(Nonce::new(1), InputPayload::MemoryUpdate, &signer);
        let (entry, content) = sample_entry(1, b"alpha");
        let record = TransitionRecord::new(input, vec![entry], vec![content]);

        let next_state = record
            .apply(&initial_state, &operator_key, VectorRoot::new([1u8; 32]))
            .unwrap();

        assert_eq!(
            next_state,
            CoreState {
                soul_hash: initial_state.soul_hash,
                vector_index_root: VectorRoot::new([1u8; 32]),
                nonce: Nonce::new(1),
            }
        );
    }

    #[test]
    fn apply_rejects_wrong_nonce() {
        let signer = sample_signer();
        let operator_key: OperatorPublicKey = signer.public_key().into();
        let state = CoreState {
            soul_hash: SoulHash::digest(b"soul"),
            vector_index_root: VectorRoot::new([0u8; 32]),
            nonce: Nonce::new(4),
        };
        let input = Input::new_signed(Nonce::new(1), InputPayload::MemoryUpdate, &signer);
        let (entry, content) = sample_entry(1, b"alpha");
        let record = TransitionRecord::new(input, vec![entry], vec![content]);

        assert_eq!(
            record.apply(&state, &operator_key, VectorRoot::new([1u8; 32])),
            Err(ValidationError::InvalidNonce {
                expected: Nonce::new(5),
                actual: Nonce::new(1),
            })
        );
    }
}

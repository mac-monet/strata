use crate::error::VectorDbError;
use crate::query::{self, QueryResult};
use commonware_codec::Encode;
use crate::keccak::Keccak256;
use commonware_runtime::{Clock, Metrics, Storage as RStorage};
use commonware_storage::mmr::{
    Readable,
    journaled::{self, Mmr},
    Location, Proof, StandardHasher,
};
use strata_core::{BinaryEmbedding, MemoryEntry, MemoryId, VectorRoot};

type KeccakDigest = <Keccak256 as commonware_cryptography::Hasher>::Digest;

/// Authenticated append-only vector database backed by a Journaled MMR.
///
/// Stores `MemoryEntry` leaves in the MMR (as serialized bytes) and maintains
/// an in-memory index for hamming-distance queries. The MMR only stores digests;
/// the raw entry data lives in the in-memory index, rebuilt from caller-provided
/// entries on `open()`.
pub struct VectorDB<E: RStorage + Clock + Metrics> {
    mmr: Mmr<E, KeccakDigest>,
    hasher: StandardHasher<Keccak256>,
    index: Vec<MemoryEntry>,
}

/// Witness for a range of new entries appended to the vector DB.
pub struct VectorDBWitness {
    pub old_root: VectorRoot,
    pub new_root: VectorRoot,
    pub new_entries: Vec<MemoryEntry>,
    pub proof: Proof<KeccakDigest>,
    pub start_location: Location,
}

fn digest_to_root(digest: &KeccakDigest) -> VectorRoot {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(digest.as_ref());
    VectorRoot::new(bytes)
}

impl<E: RStorage + Clock + Metrics> VectorDB<E> {
    /// Create a new empty VectorDB.
    pub async fn new(
        context: E,
        config: journaled::Config,
    ) -> Result<Self, VectorDbError> {
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mmr = Mmr::init(context, &mut hasher, config)
            .await
            .map_err(|e| VectorDbError::MmrInit(e.to_string()))?;

        Ok(Self {
            mmr,
            hasher,
            index: Vec::new(),
        })
    }

    /// Open a VectorDB, replaying existing entries into the in-memory index.
    ///
    /// The MMR recovers its state from the journal on disk. The caller provides
    /// the entries (from replaying `TransitionRecord`s) to populate the index.
    ///
    /// Returns `IndexMismatch` if the number of entries doesn't match the MMR's
    /// recovered leaf count.
    pub async fn open(
        context: E,
        config: journaled::Config,
        entries: Vec<MemoryEntry>,
    ) -> Result<Self, VectorDbError> {
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mmr = Mmr::init(context, &mut hasher, config)
            .await
            .map_err(|e| VectorDbError::MmrInit(e.to_string()))?;

        let mmr_leaves = mmr.leaves().as_u64();
        let entry_count = entries.len() as u64;
        if entry_count != mmr_leaves {
            return Err(VectorDbError::IndexMismatch {
                entries: entry_count,
                mmr_leaves,
            });
        }

        Ok(Self {
            mmr,
            hasher,
            index: entries,
        })
    }

    /// Append a single entry. Returns the new root.
    pub async fn append(
        &mut self,
        entry: MemoryEntry,
    ) -> Result<VectorRoot, VectorDbError> {
        let leaf = entry.encode();
        {
            let mut batch = self.mmr.new_batch();
            batch.add(&mut self.hasher, &leaf);
            let changeset = batch.merkleize(&mut self.hasher).finalize();
            self.mmr
                .apply(changeset)
                .map_err(|e| VectorDbError::MmrApply(e.to_string()))?;
        }
        self.index.push(entry);
        Ok(self.root())
    }

    /// Append multiple entries in a single batch. Returns the new root.
    pub async fn batch_append(
        &mut self,
        entries: Vec<MemoryEntry>,
    ) -> Result<VectorRoot, VectorDbError> {
        {
            let mut batch = self.mmr.new_batch();
            for entry in &entries {
                let leaf = entry.encode();
                batch.add(&mut self.hasher, &leaf);
            }
            let changeset = batch.merkleize(&mut self.hasher).finalize();
            self.mmr
                .apply(changeset)
                .map_err(|e| VectorDbError::MmrApply(e.to_string()))?;
        }
        self.index.extend(entries);
        Ok(self.root())
    }

    /// Query the index for the top-k nearest entries by hamming distance.
    pub fn query(&self, embedding: &BinaryEmbedding, k: usize) -> Vec<QueryResult> {
        query::top_k(&self.index, embedding, k)
    }

    /// Get an entry by its MemoryId.
    ///
    /// MemoryId is a monotonic 0-based counter matching insertion order,
    /// so `id.get()` is used directly as the index into the entries vector.
    pub fn get(&self, id: MemoryId) -> Option<&MemoryEntry> {
        self.index.get(id.get() as usize)
    }

    /// Current MMR root as a VectorRoot.
    pub fn root(&self) -> VectorRoot {
        digest_to_root(&self.mmr.root())
    }

    /// Peak digests of the MMR in decreasing height order.
    ///
    /// Used by the host to construct a [`Witness`] for guest transition verification.
    pub fn peak_digests(&self) -> Vec<[u8; 32]> {
        self.mmr
            .peak_iterator()
            .map(|(pos, _height)| {
                let digest = Readable::get_node(&self.mmr, pos)
                    .expect("peak node must exist");
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(digest.as_ref());
                bytes
            })
            .collect()
    }

    /// All entries in the index, for snapshotting.
    pub fn entries(&self) -> &[MemoryEntry] {
        &self.index
    }

    /// Number of entries in the index.
    pub fn len(&self) -> u64 {
        self.index.len() as u64
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Generate a witness for entries appended since `old_leaf_count`.
    pub async fn witness(
        &self,
        old_leaf_count: u64,
    ) -> Result<VectorDBWitness, VectorDbError> {
        let current_len = self.len();
        if old_leaf_count >= current_len {
            return Err(VectorDbError::NoNewEntries);
        }

        let new_entries = self.index[old_leaf_count as usize..].to_vec();
        let start_location = Location::new(old_leaf_count);

        // Compute old root by creating a temporary view — but we can just
        // use the serialized entries to generate the range proof against
        // the current root.
        let proof = self
            .mmr
            .range_proof(start_location..Location::new(current_len))
            .await
            .map_err(|e| VectorDbError::ProofGeneration(e.to_string()))?;

        // For the old_root, we need to use the historical root. The MMR doesn't
        // directly expose historical roots, but the witness consumer can reconstruct
        // from the proof. We store the current root as new_root.
        let new_root = self.root();

        // The old_root needs to be tracked by the caller (from their last known state).
        // We set it to default here — the caller fills it in from their CoreState.
        let old_root = VectorRoot::default();

        Ok(VectorDBWitness {
            old_root,
            new_root,
            new_entries,
            proof,
            start_location,
        })
    }

    /// Persist MMR state to disk.
    pub async fn sync(&self) -> Result<(), VectorDbError> {
        self.mmr
            .sync()
            .await
            .map_err(|e| VectorDbError::SyncFailed(e.to_string()))
    }

    /// Destroy the MMR's persistent storage.
    pub async fn destroy(self) -> Result<(), VectorDbError> {
        self.mmr
            .destroy()
            .await
            .map_err(|e| VectorDbError::SyncFailed(e.to_string()))
    }
}

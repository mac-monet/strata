# Provable Binary Vector Database

The vector database is the agent's unified memory system. It serves as both the persistent knowledge store and the retrieval index. Every memory the agent has lives here as a binary embedding with a pointer to full content in blobs.

By collapsing what was previously a separate "world model" and "retrieval index" into a single system, we eliminate redundancy and simplify the architecture. One storage system, one commitment, one proof mechanism. The agent's identity comes from the soul document; all other context is retrieved on demand via semantic search.

## Why Binary Embeddings

Traditional vector databases use floating-point embeddings and cosine similarity. This is impractical in ZK circuits because floating-point arithmetic is expensive to prove.

Binary embeddings solve this completely:
- Each embedding is a fixed-length bit vector (e.g., 256 bits = `[u64; 4]`)
- Similarity is measured by **hamming distance**: XOR the two vectors and count the set bits (popcount)
- XOR and popcount are cheap, native integer operations — trivial on RISC-V and efficient in Jolt

## Data Model

```rust
struct MemoryEntry {
    id: u64,
    embedding: [u64; 4],    // 256-bit binary vector
    content_hash: [u8; 32], // hash of full content posted as calldata
}
```

The vector DB is backed by Commonware's Journaled MMR for persistent authenticated storage, with an in-memory index for fast queries. See [specs/vector-db.md](./specs/vector-db.md) for implementation details.

The MMR is pure append-only — entries are never modified or removed. The soul document (committed via `soul_hash` in `CoreState`) provides the agent's identity context. All other memories are retrieved on demand via hamming distance search.

### What Gets Stored

Each entry represents a discrete piece of the agent's memory:

- **Learned facts**: "Commonware provides 17 primitives for building distributed systems including storage, consensus, and p2p."
- **Interaction summaries**: "On March 5th, Bob asked about binary embeddings for ZK-friendly vector search. I explained hamming distance."
- **Decision records**: "I chose Rhai over Lua for the skills layer because of AST compilation and Rust-native embedding."
- **Relationship knowledge**: "Alice is a Rust developer I've collaborated with on merkle tree implementations."
- **Skill descriptions**: "I have a skill for fetching GitHub PR data via the API."

Memories accumulate over time. At agent-memory scale (thousands to tens of thousands of entries), this is manageable for flat-scan retrieval.

## Query

At agent-memory scale, a flat scan is sufficient and actually preferable for provability:

```rust
fn query(db: &VectorDB, query_embedding: [u64; 4], k: usize) -> Vec<MemoryEntry> {
    // For each entry in the index:
    //   1. XOR query_embedding with entry.embedding
    //   2. Popcount the result (hamming distance)
    // Sort by distance, return top-k
}
```

No complex indexing structures (HNSW, IVF) are needed. A flat scan is:
- Deterministic and easy to prove
- Fast enough at agent-memory scale
- Simpler to commit to (Journaled MMR handles the MMR commitment)

## Proving

The ZK proof for a vector query verifies:
- The set of vectors queried matches the committed index (merkle proof)
- Hamming distances were computed correctly for each entry
- The returned results are the actual top-k nearest neighbors
- No entries were skipped or excluded

This gives **verifiable recall** — a novel primitive. In standard RAG systems, you trust that the retrieval was honest. With a provable vector DB, the retrieval itself is mathematically guaranteed.

Retrieval proving is a standalone proof — not bundled into every state transition. State transition proofs (merkle updates, constraint checks) are covered in the [proof boundary spec](./specs/proof-boundary.md).

## Generating Binary Embeddings

Two approaches:

1. **Native binary models**: some embedding models produce binary embeddings directly (e.g., Cohere, MixedBread mxbai-embed)
2. **Binarization**: take any embedding model's float output and apply sign thresholding — positive dimensions become 1, negative become 0. Loses some precision but works well in practice.

Embedding generation happens off-chain (outside the ZK proof). The resulting binary vector is provided as witness data to the state transition, which commits it via the Journaled MMR.

## Reconstruction

On normal restart, the Journaled MMR loads the vector DB from disk via `init()` — no reconstruction needed. If local storage is lost, the vector DB can be reconstructed from on-chain data:

1. Pull calldata containing memory content (verified against `content_hash` in each entry)
2. Replay all appends through `new()` + `batch_append()`
3. Verify the resulting root matches the on-chain `vector_index_root`

The soul document provides identity on startup. The full index is then available for on-demand retrieval.

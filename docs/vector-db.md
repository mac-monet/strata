# Provable Binary Vector Database

The vector database is the agent's unified memory system. It serves as both the persistent knowledge store and the retrieval index. Every memory the agent has — from core identity context to incidental facts — lives here as a binary embedding with a pointer to full content in blobs.

By collapsing what was previously a separate "world model" and "retrieval index" into a single system, we eliminate redundancy and simplify the architecture. One storage system, one commitment, one proof mechanism.

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
    content_hash: [u8; 32], // pointer to full content in data archive
    core: bool,             // core memory: always loaded into agent context
    active: bool,           // false = soft deleted (superseded by newer version)
}
```

The vector DB is backed by Commonware's QMDB (keyless variant) for persistent authenticated storage, with an in-memory index for fast queries. See [specs/vector-db.md](./specs/vector-db.md) for implementation details.

### Core Memory

Entries tagged `core: true` are the agent's identity context — the memories that are always loaded into the LLM's context regardless of what query is being processed. These are the equivalent of what a "world model" would have stored: the agent's understanding of itself, its key relationships, its ongoing projects, its most important knowledge.

Core memories answer the question: "if this agent wakes up with no input, what does it need to know to be itself?"

Non-core entries are retrieved on demand via semantic search during interactions.

### What Gets Stored

Each entry represents a discrete piece of the agent's memory:

- **Identity context** (core): "I am a research assistant focused on cryptography. I value precision and cite sources."
- **Relationship knowledge** (core): "Alice is a Rust developer I've collaborated with on merkle tree implementations. She's skeptical of new frameworks."
- **Ongoing context** (core): "I'm currently helping with the Strata project, a ZK rollup for persistent agents."
- **Learned facts** (non-core): "Commonware provides 17 primitives for building distributed systems including storage, consensus, and p2p."
- **Interaction summaries** (non-core): "On March 5th, Bob asked about binary embeddings for ZK-friendly vector search. I explained hamming distance."
- **Decision records** (non-core): "I chose Rhai over Lua for the skills layer because of AST compilation and Rust-native embedding."
- **Skill descriptions** (non-core): "I have a skill for fetching GitHub PR data via the API."

### Consolidation

Core memories are periodically consolidated by the LLM — multiple related memories may be merged into a single, updated summary. This keeps the core set compact. Consolidation is a state transition: old entries are replaced by new ones, and the merkle root is updated. The proof verifies structural integrity of the update.

Non-core memories accumulate over time. At agent-memory scale (thousands to tens of thousands of entries), this is manageable for flat-scan retrieval.

## Query

At agent-memory scale, a flat scan is sufficient and actually preferable for provability:

```rust
fn query(db: &VectorDB, query_embedding: [u64; 4], k: usize) -> Vec<MemoryEntry> {
    // For each entry:
    //   1. XOR query_embedding with entry.embedding
    //   2. Popcount the result (hamming distance)
    // Sort by distance, return top-k
}

fn load_core(db: &VectorDB) -> Vec<MemoryEntry> {
    // Return all entries where core == true
    // Always loaded into agent context
}
```

No complex indexing structures (HNSW, IVF) are needed. A flat scan is:
- Deterministic and easy to prove
- Fast enough at agent-memory scale
- Simpler to commit to (QMDB handles the MMR commitment)

## Proving

The ZK proof for a vector query verifies:
- The set of vectors queried matches the committed index (merkle proof)
- Hamming distances were computed correctly for each entry
- The returned results are the actual top-k nearest neighbors
- No entries were skipped or excluded

This gives **verifiable recall** — a novel primitive. In standard RAG systems, you trust that the retrieval was honest. With a provable vector DB, the retrieval itself is mathematically guaranteed.

## Generating Binary Embeddings

Two approaches:

1. **Native binary models**: some embedding models produce binary embeddings directly (e.g., Cohere, MixedBread mxbai-embed)
2. **Binarization**: take any embedding model's float output and apply sign thresholding — positive dimensions become 1, negative become 0. Loses some precision but works well in practice.

Embedding generation happens off-chain (outside the ZK proof). The resulting binary vector is provided as witness data to the state transition, which commits it via QMDB.

## Reconstruction

On normal restart, QMDB loads the vector DB from disk — no reconstruction needed. If local storage is lost, the vector DB can be fully reconstructed from the data archive:

1. Pull archived data (verified against the data archive commitment)
2. Re-embed all memories using the same embedding model
3. Rebuild QMDB and the in-memory index
4. Verify the resulting MMR root matches the on-chain `vector_index_root`

When reconstructing, core memories are loaded first — they give the agent its identity before any interaction occurs. The full index is then available for on-demand retrieval.

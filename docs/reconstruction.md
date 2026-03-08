# Reconstruction

Reconstruction is the defining property of Strata. Any party can independently rebuild and verify a Strata agent from publicly available data. No permission needed, no trust in the original operator required.

## The Claim

If the L1 and the calldata exist, the agent is alive. It doesn't matter if the original server goes down, the company folds, or the operator disappears. The agent's identity, memory, and full history are recoverable by anyone.

## Reconstruction Flow

### Full Replay (Trustless)

1. **Read the genesis transaction** from L1 — this contains the initial soul document and empty state
2. **Download all calldata** from subsequent state root transactions
3. **Verify content** against `content_hash` in each memory entry — confirms authenticity
4. **Replay every state transition** from genesis — each transition is deterministic:
   - Apply each batch of memory appends in order
   - Rebuild the vector index via `new()` + `batch_append()`
5. **Compare the final state root** against the on-chain `vector_index_root`
6. If the roots match, the reconstruction is verified — you have a provably correct copy of the agent

### Proof-Based (Fast)

1. **Read the latest state root and ZK proof** from L1
2. **Verify the proof** — confirms the state root is valid without replaying
3. **Download the current state** (vector index entries, soul document)
4. The agent is ready to operate immediately

## What Gets Reconstructed

| Component | Source | Verification |
|-----------|--------|-------------|
| Soul document | On-chain state | Direct read |
| Vector index (all memories) | Replay appends from calldata | Merkle root comparison against `vector_index_root` |
| Memory content | Calldata | Hash comparison against `content_hash` in each entry |

## Running the Reconstructed Agent

Once reconstructed, the agent needs:

1. **An LLM** — any compatible model can serve as the agent's reasoning engine
2. **An embedding model** — to generate query embeddings for memory retrieval
3. **Host bindings** — API access, network, etc. (environment-dependent)

The soul document becomes the system prompt. Core memories from the vector DB are always loaded as context. Non-core memories are retrieved on demand. Different operators may use different LLMs, which affects the quality of the agent's reasoning but not its identity or memory.

## Implications

### Immortality
The agent survives its creator. As long as the L1 and calldata persist, the agent can be brought back.

### Forkability
Anyone can snapshot the agent's state and create a variant — same memories, different soul. Or same soul, different memories. Agent lineage becomes possible.

### Auditability
The full memory history is replayable. Every memory entry, every batch — all traceable back to genesis.

### Portability
The agent is not locked to any operator, any LLM provider, or any infrastructure. It's defined by its data, not its runtime.

### Verifiability
Two independent parties reconstructing the same agent from the same data will arrive at identical state roots. There is exactly one correct version of the agent at any point in time.

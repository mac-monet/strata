# Reconstruction

Reconstruction is the defining property of Strata. Any party can independently rebuild and verify a Strata agent from publicly available data. No permission needed, no trust in the original operator required.

## The Claim

If the L1 and blob data exist, the agent is alive. It doesn't matter if the original server goes down, the company folds, or the operator disappears. The agent's identity, memory, capabilities, and full history are recoverable by anyone.

## Reconstruction Flow

### Full Replay (Trustless)

1. **Read the genesis block** from L1 — this contains the initial soul document and empty state
2. **Download all blobs** from the blob layer
3. **Verify blobs** against the on-chain MMR commitment — confirms they're authentic and complete
4. **Replay every state transition** from genesis — each transition is deterministic Rust code:
   - Apply each interaction in order
   - Run each consolidation
   - Rebuild the vector index
5. **Compare the final state root** against the on-chain commitment
6. If the roots match, the reconstruction is verified — you have a provably correct copy of the agent

### Proof-Based (Fast)

1. **Read the latest state root and ZK proof** from L1
2. **Verify the proof** — confirms the state root is valid without replaying
3. **Download the current state** (vector index entries, soul document)
4. **Download blobs** for full history (optional — only needed for auditability, not for running the agent)
5. The agent is ready to operate immediately

### Snapshot-Based (Practical)

1. **Find the most recent memory snapshot** in blob data
2. **Verify it** against the on-chain state at that point
3. **Replay only the transitions since the snapshot**
4. Faster than full replay, still trustless

## What Gets Reconstructed

| Component | Source | Verification |
|-----------|--------|-------------|
| Soul document | On-chain state | Direct read |
| Vector index (all memories) | On-chain merkle tree or blob data | Merkle root comparison |
| Skills (Rhai ASTs) | Blob data | MMR inclusion proof |
| Interaction history | Blob data | MMR inclusion proof |
| Reasoning traces | Blob data | MMR inclusion proof |

## Running the Reconstructed Agent

Once reconstructed, the agent needs:

1. **An LLM** — any compatible model can serve as the agent's reasoning engine
2. **A Rhai runtime** — to execute the agent's self-built skills
3. **Host bindings** — API access, network, etc. (environment-dependent)

The soul document becomes the system prompt. Core memories from the vector DB are always loaded as context. Non-core memories are retrieved on demand. Skills give the agent its capabilities. Different operators may use different LLMs, which affects the quality of the agent's reasoning but not its identity or memory.

## Implications

### Immortality
The agent survives its creator. As long as the L1 and blob data persist, the agent can be brought back.

### Forkability
Anyone can snapshot the agent's state and create a variant — same memories, different soul constraints. Or same soul, different memories. Agent lineage becomes possible.

### Auditability
The full cognitive history is replayable. Every memory consolidation, every decision, every skill creation — all traceable back to genesis.

### Portability
The agent is not locked to any operator, any LLM provider, or any infrastructure. It's defined by its data, not its runtime.

### Verifiability
Two independent parties reconstructing the same agent from the same data will arrive at identical state roots. There is exactly one correct version of the agent at any point in time.

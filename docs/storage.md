# Storage Layer

The storage layer handles the agent's raw data — the full record that sits beneath the compressed on-chain state. Conversation logs, reasoning traces, raw fact extractions, and memory snapshots all live here. The data is committed on-chain via a compact MMR root.

Where the data is actually posted is an implementation detail. For the MVP on Base (which does not support blobs), data is posted as calldata. In the future, this can migrate to EIP-4844 blobs or an external DA layer without changing the architecture — the MMR commitment and verification logic remain the same regardless of where the underlying data lives.

## Architecture

```
On-chain:  data_archive_root (MMR root, ~32 bytes)
               │
               ▼
Data:      Full interaction data, committed and ordered
           (MVP: calldata on Base | Future: blobs, DA layer)
```

## MMR / QMDB

The blob archive uses a Merkle Mountain Range (MMR) or QMDB-style authenticated data structure for commitments. This is a natural fit because blob data is append-only:

- New interactions are appended, never modified
- Efficient inclusion proofs ("this conversation happened")
- Ordering guarantees ("this interaction came before that one")
- Compact root that grows logarithmically
- Efficient appends without recomputing the entire tree

## Built on Commonware

The storage layer is built on Commonware primitives:

- **commonware-storage**: persistence and retrieval from an abstract store
- **commonware-codec**: serialization of blob data
- **commonware-cryptography**: hashing and signing commitments

Commonware provides the authenticated data structure primitives. Strata builds the agent-specific blob schema and commitment logic on top.

## What Gets Stored in Blobs

### Interaction Logs
Full conversation transcripts — every message the agent received and every response it produced. These are the raw material from which the world model is derived.

### Reasoning Traces
When the agent makes a decision, the reasoning behind it — what context was retrieved, what the LLM considered, why it chose a particular action. Important for auditability.

### Raw Fact Extractions
Before consolidation, the LLM extracts structured facts from interactions. These accumulate in blob storage until the next consolidation cycle compresses them into world model summaries.

### Memory Snapshots
Periodic checkpoints of the full agent state for faster reconstruction. Instead of replaying from genesis, a reconstructor can start from the nearest snapshot.

### Skill Definitions
Rhai ASTs for the agent's self-created tools (see [skills.md](./skills.md)). Stored as blob data, committed on-chain.

## Blob Lifecycle

```
Interaction occurs
    │
    ▼
Raw data serialized via commonware-codec
    │
    ▼
Appended to blob archive
    │
    ▼
MMR root updated (state transition, proven in Jolt)
    │
    ▼
New blob_archive_root committed on-chain
```

## Verification

Anyone can verify the blob archive:
1. Download all blobs
2. Reconstruct the MMR from the blob sequence
3. Compare the computed root against the on-chain commitment
4. If they match, the blobs are authentic and complete

## Relationship to Core State

Blobs are the "receipts" — the full, detailed record. Core state is the "summary" — the compressed, actionable knowledge. The agent operates on core state day-to-day but blobs make everything auditable and reconstructable.

The vector index is derived from archived data (via embedding). But the agent doesn't need the archive to function — it needs it to be verified and reconstructed.

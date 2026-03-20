# Vector DB Migration: MMR to QMDB

The current vector DB is append-only — memories can be added but never updated or deleted. This document describes the planned migration from Commonware's Journaled MMR to QMDB Current, which adds mutable key-value semantics while preserving authenticated merkle commitments.

## Motivation

The append-only model means the agent can't correct outdated knowledge, remove irrelevant memories, or consolidate duplicates. When contradictory memories exist (e.g., "the API endpoint is X" and later "the API endpoint changed to Y"), the LLM must resolve the conflict at inference time with no structural support.

QMDB Current solves this by adding upsert and delete operations to the authenticated storage layer. The agent gains explicit control over its knowledge, with the LLM making semantic decisions about when to update or forget.

## What QMDB Current Provides

QMDB (Quick Merkle Database) is a Commonware storage primitive that extends the MMR into a mutable authenticated database. It models state as a complete history of operations (assigns and deletes) stored in an append-only log, with an activity bitmap tracking which entries are currently active.

Two variants exist:
- **`qmdb::any`** — proves a key was assigned a value *at some point in history*
- **`qmdb::current`** — additionally proves a key *currently* holds a value (via authenticated bitmap)

`qmdb::current` is the right fit — the agent needs to prove what it currently knows, not what it once knew.

Key properties:
- **Upsert**: `write(key, Some(value))` — insert or replace
- **Delete**: `write(key, None)` — remove from active state
- **Authenticated root**: canonical root = `hash(ops_root || grafted_mmr_root || partial_chunk_hash)`
- **Current-value proofs**: range proofs include bitmap chunks proving activity status
- **Inactivity floor**: compaction process prunes old operations for memory efficiency
- **Same hasher trait**: uses `commonware_cryptography::Hasher` — existing Keccak256 adapter works

Reference: [commonware.xyz/blogs/adb-any](https://commonware.xyz/blogs/adb-any)

## What Changes

### Storage Backend

| | Current (MMR Journaled) | Future (QMDB Current) |
|---|---|---|
| Data model | Append-only leaves | Key-value with operation log |
| Mutations | Append only | Upsert + delete |
| Root | MMR root | Canonical root (ops + bitmap) |
| Proofs | Range inclusion | Range + activity status |
| Current state | Implied by caller | Explicit via activity bitmap |
| Recovery | Caller provides entries on `open()` | Operation log replay reconstructs snapshot |
| Async | Mixed | Fully async |

### Agent Primitives

Current:
- `recall(query)` — search memory
- `remember(text)` — store new memory

After migration:
- `recall(query)` — search memory (unchanged)
- `remember(text)` — store new memory (unchanged)
- `update(id, text)` — replace an existing memory with new content
- `forget(id)` — delete a memory

The LLM drives update/forget decisions. During recall, returned memories include their IDs. The LLM can then say "memory #42 is outdated" and explicitly update or remove it. The semantic judgment stays with the LLM — the storage just provides the primitive.

### API Migration

| Current | After Migration |
|---|---|
| `append(entry)` | `write(entry.id, Some(value))` |
| `batch_append(entries)` | single batch with multiple `write()` calls |
| `query(embedding, k)` | unchanged — still flat-scan hamming on in-memory snapshot |
| `get(id)` | `get(&id)` (now backed by snapshot, not caller-provided index) |
| `root()` | `root()` (canonical root instead of MMR root) |
| `witness(old_leaf_count)` | `range_proof(start, count)` |
| N/A | `write(id, None)` — delete |
| N/A | `write(id, Some(new_value))` — update |

### Proof Model

The ZK guest currently verifies MMR range proofs (contiguous leaf inclusion). After migration, it verifies QMDB range proofs which include:
- Operation log continuity (same as MMR range proof)
- Activity bitmap chunks (proving entries are current, not deleted)
- Grafted MMR structure (bitmap chunks embedded at grafting height)

The `Witness` type in `strata-proof` will need to change to carry the new proof structure. The transition function's verification logic changes accordingly.

### Reconstruction

Current: replay all transitions via `new()` + `batch_append()`, compare final root.

After migration: replay the operation log (appends, updates, deletes). QMDB reconstructs the snapshot deterministically from the log. Final canonical root compared against on-chain state.

The reconstruction flow is structurally the same — replay from genesis, verify root. The log format changes from raw appends to structured operations.

## What Stays the Same

- **Keccak256 adapter** — QMDB uses the same `commonware_cryptography::Hasher` trait
- **Query logic** — hamming distance flat scan is unchanged
- **`MemoryEntry` type** — same fields, keyed by `id` (already a monotonic u64)
- **Embedding generation** — unchanged, still off-chain
- **Proof boundary** — QMDB proofs are still verified in the guest, everything else stays host-side
- **Calldata posting** — memory content still posted as calldata for reconstruction

## Keying Strategy

Memories are keyed by their `id: u64` (monotonically increasing). The LLM references memories by ID when deciding to update or forget them.

Content-addressed keying (key = embedding) was considered but rejected — even minor rephrasing produces different binary embeddings, so almost nothing would collide. Semantic dedup (hamming threshold) was also rejected — the threshold is a policy decision that's too blunt for the semantic judgment required. ID-addressed with explicit LLM decisions is the cleanest model.

## Dependencies

The migration requires:
- `commonware-storage` `qmdb::current::unordered::fixed` (ALPHA stability)
- Existing `commonware-storage` `mmr::journaled` dependency is replaced, not added alongside
- `commonware-storage` `mmr::proof` may still be needed for guest-side verification (TBD based on QMDB's proof verification API)

## Open Questions

- **QMDB stability**: `qmdb` is currently ALPHA in commonware. Migration should wait for BETA or verify the API is stable enough.
- **Guest proof verification**: need to confirm QMDB provides `no_std`-compatible proof verification for the ZK guest, or if we need to extract the verification logic.
- **Inactivity floor policy**: how aggressively to compact old operations. Affects storage size and proof generation speed.
- **Batch semantics**: current batching (accumulate appends, periodically merkleize) maps to QMDB's `new_batch() → write() → merkleize() → finalize() → apply_batch()` lifecycle, but need to verify the interaction with the activity bitmap.

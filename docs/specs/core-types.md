# Core Types Spec

Shared types used by both the Jolt guest and the host. Everything in `strata-core` must be `no_std` compatible (with `alloc` where needed).

Serialization is handled via `commonware-codec` for deterministic hashing. JSON serialization (`serde`) is added for host-side convenience but not required in the guest.

## CoreState

The on-chain state. All fixed-size — no variable-length fields.

```rust
struct CoreState {
    soul_hash: [u8; 32],
    vector_index_root: [u8; 32],
    nonce: u64,
}
```

- `soul_hash` — SHA-256 hash of the soul document text. The full text lives in the rollup contract's public storage. On startup/reconstruction, the host reads the text from the contract and verifies it against this hash.
- `vector_index_root` — MMR root over all `MemoryEntry` values in the vector DB (from QMDB).
- `nonce` — monotonically increasing counter. Incremented on every proven state transition.

### GenesisConfig

Immutable configuration fixed at genesis. Produces the initial `CoreState`.

```rust
struct GenesisConfig {
    soul_hash: [u8; 32],
    operator_key: [u8; 32],
    initial_vector_index_root: [u8; 32],
}
```

- `soul_hash` — SHA-256 hash of the soul document text.
- `operator_key` — ed25519 public key authorized to sign inputs.
- `initial_vector_index_root` — starting MMR root (typically the empty tree root).

`GenesisConfig::genesis_state()` produces:

```rust
CoreState {
    soul_hash: config.soul_hash,
    vector_index_root: config.initial_vector_index_root,
    nonce: 0,
}
```

## MemoryEntry

A single memory in the vector DB. Append-only — modifications and deletions are new appends that deactivate old entries.

```rust
struct MemoryEntry {
    id: u64,
    embedding: [u64; 4],
    content_hash: [u8; 32],
    core: bool,
    active: bool,
}
```

- `id` — unique, monotonically increasing. Assigned by the host on creation.
- `embedding` — 256-bit binary vector. Generated off-chain by an embedding model.
- `content_hash` — SHA-256 hash of the full text content. Content is posted as calldata.
- `core` — if true, always loaded into the LLM's context.
- `active` — if false, entry has been superseded or deleted. Excluded from queries.

Fixed size: 8 + 32 + 32 + 1 + 1 = 74 bytes per entry.

## Input

The external trigger for a state transition. Carries authorization.

```rust
struct Input {
    nonce: u64,
    signature: [u8; 64],
    payload: InputPayload,
}

enum InputPayload {
    MemoryUpdate,
    // Future variants:
    // Consolidation — merge core memories into fewer entries
    // SoulAmendment { new_soul_hash: [u8; 32] } — change the soul document
    // SkillMutation — add/modify a skill (Rhai AST)
}
```

- `nonce` — must equal `current_state.nonce + 1`. Guest rejects otherwise.
- `signature` — ed25519 signature over the serialized payload + nonce. Proves the authorized operator submitted this transition.
- `payload` — what kind of transition this is. MVP only has `MemoryUpdate`. The actual memory diff and content blobs live outside the `Input`: the replay path uses `TransitionRecord`, while the proving path uses `Witness`.

### Why payload is separate from witness

The Input says *what's happening* and *who authorized it*. The Witness provides *the data needed to verify it*. This maps to the rollup model: Input = transaction, Witness = execution trace.

## Witness

Data the host prepares for the guest to verify a state transition. Contains everything the guest needs to check the merkle update.

```rust
struct Witness {
    new_entries: Vec<MemoryEntry>,
    deactivated_ids: Vec<u64>,
    merkle_proofs: Vec<MerkleProof>,
}
```

- `new_entries` — memory entries to append. The guest verifies these are correctly incorporated into the new merkle root.
- `deactivated_ids` — IDs of entries being marked `active: false`. The guest verifies the deactivation is reflected in the new root.
- `merkle_proofs` — QMDB-generated proofs for each operation. The guest verifies these against the old and new roots.

A single witness can cover one interaction or many (batched). The guest doesn't care — it just verifies the diff.

### MerkleProof

Type alias or re-export from `commonware-storage`. The exact shape depends on QMDB's proof format. Not defined here — it comes from the dependency.

## TransitionRecord

Canonical replay payload for one accepted state transition. Used for persistence and reconstruction — distinct from the `Witness` which is the guest-side proving payload.

```rust
struct TransitionRecord {
    input: Input,
    delta: MemoryDelta,
    contents: Vec<MemoryContent>,
}
```

- `input` — the signed input that authorized this transition.
- `delta` — the structured memory diff (new entries + deactivations).
- `contents` — the full content blobs for each new entry, posted as calldata.

### MemoryDelta

```rust
struct MemoryDelta {
    new_entries: Vec<MemoryEntry>,
    deactivated_ids: Vec<u64>,
}
```

- `new_entries` — entries to append. Must be strictly increasing by id. Must all be `active: true`.
- `deactivated_ids` — IDs being marked inactive. Must be strictly increasing.
- A delta cannot add and deactivate the same id.

### MemoryContent

```rust
struct MemoryContent {
    memory_id: u64,
    bytes: Vec<u8>,
}
```

Each new entry must have exactly one corresponding `MemoryContent`. The content's SHA-256 hash must match the entry's `content_hash`.

### ValidationError

Validation failures returned by `TransitionRecord::validate()` and `validate_against()`:

- `MalformedOperatorKey` — operator key bytes are not valid ed25519
- `InvalidNonce` — signed nonce doesn't match expected `state.nonce + 1`
- `InvalidSignature` — signature doesn't verify against operator key
- `NewEntriesOutOfOrder` — new entry IDs not strictly increasing
- `InactiveNewEntry` — a new entry has `active: false`
- `DeactivatedIdsOutOfOrder` — deactivated IDs not strictly increasing
- `ConflictingMemoryId` — same ID appears in both new entries and deactivated IDs
- `ContentCountMismatch` — number of content blobs doesn't match number of new entries
- `ContentIdMismatch` — content blob ID doesn't match its corresponding entry ID
- `ContentHashMismatch` — content hash doesn't match committed `content_hash`

`validate_against(state, operator_key)` checks nonce before signature (cheap check first).

## Transition Function

The guest's entry point:

```rust
fn transition(state: CoreState, input: Input, witness: Witness) -> CoreState
```

Returns the new `CoreState` with updated `vector_index_root` and incremented `nonce`. The `soul_hash` is carried through unchanged (no soul amendments in MVP).

Panics (fails to generate proof) if:
- `input.nonce != state.nonce + 1`
- Signature is invalid
- Merkle proofs don't verify
- Old root doesn't match `state.vector_index_root`

## What lives where

| Type | Crate | `no_std` | Notes |
|------|-------|----------|-------|
| `GenesisConfig` | `strata-core` | yes | Fixed-size, produces initial `CoreState` |
| `CoreState` | `strata-core` | yes | Fixed-size, all arrays |
| `MemoryEntry` | `strata-core` | yes | Fixed-size |
| `Input` | `strata-core` | yes | Fixed-size |
| `InputPayload` | `strata-core` | yes | MVP: single variant |
| `TransitionRecord` | `strata-core` | yes | Needs `alloc` for Vec fields |
| `MemoryDelta` | `strata-core` | yes | Needs `alloc` for Vec fields |
| `MemoryContent` | `strata-core` | yes | Needs `alloc` for content bytes |
| `ValidationError` | `strata-core` | yes | Validation failure enum |
| `Witness` | proof layer (`strata-guest` or `strata-proof-types`) | yes | Guest-side proving payload, intentionally separate from canonical replay data |
| `MerkleProof` | re-export from commonware | yes | Defined by QMDB |
| `QueryResult` | `strata-vector-db` | no | Host-only, not needed in guest |
| `VectorDB` | `strata-vector-db` | no | Host-only, wraps QMDB |

## Serialization

All core types implement `commonware-codec::{Read, Write}` for deterministic binary serialization. This is used for:
- Hashing `MemoryEntry` as MMR leaves
- Hashing `CoreState` for on-chain commitment
- Serializing `Input` for signature verification

Host-side types additionally derive `serde::{Serialize, Deserialize}` for JSON.

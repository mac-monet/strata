# Implementation Plan

This document captures the decisions made during architecture review and defines what needs to be built for the MVP. An agent should be able to implement this plan by following the steps in order.

## Architecture Decisions

These decisions supersede anything in the existing docs that conflicts.

### ZK Prover: OpenVM (not Jolt)

Switch from Jolt to OpenVM for ZK proving.

**Why:**
- OpenVM has a production Solidity verifier (audited by Cantina, 330K gas on-chain). Jolt has no on-chain verifier and building one would take 2-3 months + audit.
- OpenVM is faster and uses less memory than Jolt (benchmarked: ~3-10x faster, 3-5x less RAM).
- The migration is scoped to the proving layer only — the transition function is pure Rust and prover-agnostic.

**References:**
- OpenVM docs: https://docs.openvm.dev/book/writing-apps/overview/
- OpenVM Solidity SDK: https://docs.openvm.dev/book/writing-apps/solidity-sdk/
- OpenVM example: https://github.com/openvm-org/openvm-example-fibonacci

### Hash Function: Keccak256 (not Blake3)

Switch from Blake3 to Keccak256 for all commitments.

**Why:**
- OpenVM has a native keccak precompile (`openvm-keccak256` crate) — the hash is proven as an optimized circuit, not as software RISC-V instructions. Blake3 has no precompile and would run as slow software emulation.
- Keccak is an EVM opcode (30 gas base + 6 gas/word). On-chain hash verification is essentially free.
- Keccak is fast natively for host-side operations and reconstruction.
- Blake3 was chosen because it was 4.3x faster than SHA-256 in Jolt's RISC-V. That benchmark is irrelevant in OpenVM where precompile vs software is the dominant factor.

### Tool Execution: Monty (not Rhai)

Use Pydantic's Monty — a minimal Python interpreter written in Rust — instead of Rhai for tool execution.

**Why:**
- LLMs write excellent Python but mediocre Rhai. The Cloudflare "code mode" pattern (LLM writes code that calls host functions, reducing round trips) works best with a language the LLM knows well.
- Monty is sandboxed by default — no filesystem, no network, no env vars unless explicitly exposed as host functions.
- Monty is snapshotable to bytes — execution state can be serialized and stored for reconstruction.
- Microsecond startup, no container overhead.
- Rust-native — embeds directly, no FFI.

**Host functions exposed to Monty:**

```
fetch(url, method, headers, body) -> response    # HTTP requests (A2A, APIs, x402)
recall(query_embedding) -> memories               # Vector DB hamming distance search
remember(text) -> memory_id                       # Store new memory entry
sign(message) -> signature                        # Produce a signature
call_contract(to, method, args) -> result         # On-chain write (via rollup contract)
read_contract(to, method, args) -> result         # On-chain read
```

**Monty reference:** https://github.com/pydantic/monty

**Deferred:** For MVP, Monty integration is a strong-to-have. Start with fixed Rust host functions behind standard LLM function calling. Add Monty when the round-trip cost of sequential tool calls becomes a bottleneck.

### On-Chain Interactions: Host Functions + alloy

The agent interacts with on-chain contracts through host functions backed by alloy (Rust Ethereum library).

**Why host functions instead of an in-sandbox eth library:**
- Monty's stdlib is too limited for a full eth library (no classes, limited stdlib)
- The host builds transactions from structured arguments, which is simpler and more reliable
- Soul hard constraints are checked before execution (spending limits, allowed contracts)

**Security model:**
- There is no private key risk because the agent IS the rollup contract. Authorization comes from the ZK proof, not a signing key.
- The operator's key posts proof batches to L1 but doesn't control agent funds.
- The rollup contract holds funds and only releases them when a valid proof authorizes the action.

### Agent Runtime: Purpose-Built (no framework)

Build a lean host runtime from scratch. No Rig, no OpenClaw fork, no agent framework.

**Why:**
- Rig adds indirection for provider abstraction we don't need (targeting 1-2 LLM providers)
- OpenClaw/Moltis/IronClaw are personal assistant platforms with channels, voice, scheduling — all irrelevant
- The host layer is ~500-1k lines of Rust: LLM client, tool dispatch, embedding generation, state transition pipeline

---

## Implementation Steps

### Step 1: Switch hash function to Keccak256 — DONE

All hashing switched from Blake3 to Keccak256 via `alloy-primitives`.

**What landed:**
- `strata-core`: `blake3` dep replaced with `alloy-primitives`. `SoulHash::digest()` and `ContentHash::digest()` use `alloy_primitives::keccak256()`.
- `strata-proof`: `Keccak256Hasher` implements `GuestHasher`, wrapping `alloy_primitives::Keccak256`. Uses `core::mem::replace` in `finalize()` because alloy's `Keccak256::finalize()` consumes self. The old `blake3` feature gate is removed.
- `strata-vector-db`: New `keccak` module (`crates/vector-db/src/keccak.rs`) implements `commonware_cryptography::Hasher` for `Keccak256`, with a `Digest` newtype implementing all required trait bounds (`Array`, `Span`, `Random`, `FixedSize`, `Write`, `Read`, etc.). `db.rs` uses `StandardHasher<Keccak256>` throughout.
- All tests updated and passing (40 Rust tests).
- All docs updated: Blake3/SHA-256 → Keccak256.

### Step 2: Migrate proving from Jolt to OpenVM — DONE

**What landed:**
- `strata-jolt/` moved to `archive/strata-jolt/`.
- `strata-openvm/` created outside the workspace (in root `Cargo.toml` exclude list).
- Guest program (`strata-openvm/guest/src/main.rs`) uses `OpenVmKeccak` hasher backed by `openvm_keccak256_guest::keccak256()` native precompile.
- `openvm.toml` enables rv32i, rv32m, io, and keccak precompiles.
- Host binary (`strata-openvm/src/main.rs`) is a placeholder that runs the transition locally as a sanity check.

### Step 3: Write the rollup contract — DONE

**What landed:**
- Foundry project in `contracts/` with `forge-std` as submodule.
- `StrataRollup.sol` with operator access control, virtual `_verify()` hook for mock testing.
- State continuity verification — contract checks all 4 public values fields against on-chain state.
- 10 Foundry tests passing (deployment, transitions, events, access control, state mismatch, short publicValues).

---

## What Exists Now (reference for Step 4)

### Public Values Layout (104 bytes)

The ZK guest reveals 4 values. The contract verifies all of them. This is the interface between the prover and the rollup contract.

```
Offset  Size  Field      Description
0       32    oldRoot    vector_index_root before transition (must match contract.stateRoot)
32      32    newRoot    vector_index_root after transition (stored as new stateRoot)
64      8     nonce      new nonce, u64 big-endian (must equal contract.nonce + 1)
72      32    soulHash   soul document hash (must match contract.soulHash)
```

Total: 104 bytes. Fixed offsets, no ABI encoding — raw byte slicing on-chain.

### Contract ABI

```solidity
contract StrataRollup {
    bytes32 public stateRoot;
    uint64  public nonce;
    bytes32 public soulHash;
    address public operator;
    address public verifier;
    bytes32 public appExeCommit;
    bytes32 public appVmCommit;

    error OnlyOperator();
    error InvalidPublicValues();  // publicValues.length < 104
    error StateMismatch();        // oldRoot/nonce/soulHash don't match on-chain state
    error VerificationFailed();   // ZK proof verification failed

    constructor(
        string memory _soulText,   // stored as keccak256 hash
        address _verifier,
        address _operator,
        bytes32 _appExeCommit,
        bytes32 _appVmCommit,
        bytes32 _initialStateRoot
    );

    function submitTransition(
        bytes calldata publicValues,   // 104 bytes (layout above)
        bytes calldata proofData,      // ZK proof bytes
        bytes calldata memoryContent   // DA-only, not read on-chain
    ) external;  // reverts OnlyOperator, InvalidPublicValues, StateMismatch, VerificationFailed
}
```

The `_verify()` method is `internal view virtual` — overridden in tests with a no-op mock. In production it calls `verifier.staticcall(...)`.

### Keccak Adapter for Commonware MMR

`strata-vector-db` exports `pub mod keccak` containing:
- `keccak::Keccak256` — implements `commonware_cryptography::Hasher`
- `keccak::Digest` — newtype over `[u8; 32]` implementing `commonware_cryptography::Digest`

The host uses `StandardHasher<keccak::Keccak256>` for all MMR operations. The guest uses `strata_proof::Keccak256Hasher` (a separate, lighter implementation via `GuestHasher` trait). Both produce identical hashes — verified by cross-validation tests.

### Dependency Choices Already Locked In

| Dep | Crate | Notes |
|-----|-------|-------|
| `alloy-primitives` | strata-core, strata-proof, vector-db | Keccak256 (one-shot + incremental). NOT `tiny-keccak`. |
| `commonware-cryptography` | vector-db | `Hasher` trait for MMR integration |
| `commonware-storage` | vector-db | Journaled MMR |
| `commonware-codec` | core, proof | Deterministic serialization |
| `serde` | core (optional), proof (optional) | Feature-gated. Required for OpenVM guest IO. |

### Crate Status

| Crate | Role | Status |
|-------|------|--------|
| `strata-core` | Canonical shared types, serialization, validation (no_std) | Done |
| `strata-proof` | ZK transition logic — MMR ops, nonce validation, integrity checks | Done |
| `strata-vector-db` | Binary vector DB over Journaled MMR — hamming queries, merkle commitment | Done |
| `strata-openvm` | OpenVM guest program + proving scaffold | Done (scaffold) |
| `contracts` | StrataRollup Solidity contract with state continuity verification | Done |
| `strata-agent` | Host runtime — LLM, tools, prover, L1, A2A, reconstruction | **Next** |

---

### Step 4: Build `strata-agent` (host runtime)

**Location:** `crates/strata-agent/`

This is the single host-side crate. Everything outside the proof boundary lives here. Add it to the workspace `members` in root `Cargo.toml`.

**Key dependencies:** `reqwest` (HTTP), `axum` (server), `alloy` (L1), `tokio` (async runtime), `serde_json`, plus workspace crates `strata-core`, `strata-proof`, `strata-vector-db`.

Build incrementally in this order (each sub-step produces something testable):

#### 4a: LLM Client
- HTTP client (use `reqwest`) that calls the Anthropic API (Claude)
- Support system prompt + conversation history + function calling
- Parse tool use responses, return structured tool calls
- The soul document text becomes the system prompt
- ~100-200 lines

#### 4b: Embedding Client
- Call an embedding API to generate embeddings from text
- Convert float embeddings to binary vectors (threshold at median)
- Return `BinaryEmbedding` ([u64; 4])
- ~50-100 lines

#### 4c: Tool Dispatch
- Define host functions as an enum or trait: `recall`, `remember`, `fetch`, `sign`, `call_contract`, `read_contract`
- `recall` queries `VectorDB::query()` with a binary embedding
- `remember` appends to `VectorDB` via `VectorDB::append()`
- Match LLM function call responses to host function implementations
- Execute and return results to the LLM
- ~100-200 lines

#### 4d: State Transition Pipeline
- After LLM produces memory updates (via `remember` tool calls):
  1. Generate embeddings for new text (4b)
  2. Append to VectorDB (get new MMR root)
  3. Build `Witness` from `VectorDB::witness()` — provides old peaks, old leaf count, new entries
  4. Build 104-byte public values: `[old_root ++ new_root ++ nonce_be ++ soul_hash]`
  5. Invoke OpenVM prover (via CLI subprocess or SDK) — deferred for MVP, can just run transition locally
  6. Get proof back
- The `VectorDB::peak_digests()` method gives you the old peaks for the witness
- ~100-200 lines

#### 4e: L1 Posting
- Use `alloy` to interact with the StrataRollup contract on Base
- Build `submitTransition(publicValues, proofData, memoryContent)` call
- `publicValues` = 104 bytes (see layout above)
- `proofData` = ZK proof bytes from OpenVM
- `memoryContent` = serialized memory content for DA (raw bytes, not read on-chain)
- The operator wallet signs and submits the L1 transaction
- ~100-150 lines

#### 4f: HTTP/A2A Server
- HTTP server (use `axum`) exposing:
  - `POST /a2a` — A2A endpoint for receiving messages from other agents
  - `GET /.well-known/agent.json` — Agent Card endpoint (JSON describing capabilities)
  - `GET /health` — Health/status endpoint
- Parse incoming A2A messages, feed into the state transition pipeline
- ~200-300 lines

#### 4g: Reconstruction
- Given only a contract address on Base:
  1. Read `soulHash` from contract, find the deployment tx to recover soul text from constructor args
  2. Read all `submitTransition` calldata from transaction history (use alloy provider + event filtering on `StateTransition` events)
  3. Extract `memoryContent` from each tx's third calldata argument
  4. Replay all state transitions from genesis: create `VectorDB::new()`, then `batch_append()` each transition's entries
  5. Use `StandardHasher::<strata_vector_db::keccak::Keccak256>::new()` for MMR operations during replay
  6. Verify final `VectorDB::root()` matches on-chain `stateRoot`
- This is the killer demo feature
- ~200-300 lines

#### 4h: x402 Middleware
- Wrap the HTTP client with x402 payment handling
- On 402 response: extract payment details, pay from rollup contract, retry
- ~50-100 lines

### Step 5: Monty Integration (strong-to-have)

After the core agent works with fixed tool calling:

1. Add `pydantic-monty` (Rust crate) as a dependency to `strata-agent`.
2. Register host functions (`fetch`, `recall`, `remember`, `sign`, `call_contract`, `read_contract`) with the Monty runtime.
3. When the LLM decides to use code mode: it writes a Python script, Monty executes it, host functions are fulfilled by the existing Rust implementations.
4. Results feed into the state transition pipeline as before.

### Step 6: Demo

The MVP demo tells this story:

1. **Create** — deploy StrataRollup contract on Base with a soul document. Spin up strata-agent.
2. **Interact** — have conversations. Watch the agent build memories, query the vector DB, respond. Each interaction produces a ZK proof and posts state to L1.
3. **Verify** — show a proof being verified on-chain (330K gas). Show the state root updating.
4. **Kill** — shut down the agent. Delete all local state.
5. **Reconstruct** — run reconstruction from the contract address alone. Download soul + calldata + state roots. Replay from genesis.
6. **Prove** — interact with the reconstructed agent. Show it remembers everything. Verify the state root matches.

---

## Updated Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| Language | Rust | no_std for guest, std for host |
| ZK Prover | **OpenVM** (was Jolt) | Production Solidity verifier, 330K gas |
| Hash Function | **Keccak256** (was Blake3) | OpenVM precompile + EVM opcode |
| Infrastructure | Commonware primitives | storage, codec, cryptography, runtime |
| Tool Runtime | **Monty** (was Rhai) | Minimal Python interpreter in Rust, sandboxed |
| On-Chain | alloy + Base L1 | StrataRollup contract, ERC-8004 |
| Embeddings | Binary vectors, hamming distance | Unchanged |
| LLM | Claude API (via reqwest) | No framework, direct HTTP |

## What's Deferred

- **Full A2A** (streaming, push notifications) — basic request/response is sufficient
- **x402 inbound** (getting paid) — outbound only for MVP
- **Memory consolidation** — core memories accumulate without compression
- **Snapshots** — full replay from genesis is fine at MVP scale
- **Monty** — start with fixed tool calling, add Monty when round-trip overhead becomes a bottleneck

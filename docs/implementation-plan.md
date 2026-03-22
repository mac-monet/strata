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

### Tool Execution: Bash + Container (minimal primitives)

MVP uses three tools: `recall`, `remember`, `bash`. Inspired by Pi agent's philosophy — what you leave out matters more than what you put in. The LLM composes primitives to do anything complex.

**Why bash, not dedicated tools:**
- Bash is the universal escape hatch. `curl` for HTTP, `cast` for on-chain reads, `python3` for scripting — all available via bash.
- `recall` and `remember` must be dedicated tools because they touch internal state (vector DB) not exposed via CLI. Everything else goes through bash.
- Avoids enumerating every possible action as a dedicated tool. The LLM already knows how to compose shell commands.

**Sandboxing:** The container is the sandbox. The agent runs in a Docker container (or Firecracker microVM). Bash executes inside the container with limited filesystem, network policies, and resource limits. No need for application-level sandboxing.

**Future codemode evolution:**
- Evaluate whether raw Python in container (`python3 -c '...'` via bash) is sufficient for structured scripting
- If snapshotability or determinism is needed, consider Monty (pure-Rust Python interpreter) or similar
- On-chain actions may evolve to Weiroll VM (composable transaction scripts) for batching multiple contract interactions atomically

### Runtime Model

The agent is a rollup. It's a long-running server that processes interactions and periodically commits state roots to L1.

```
┌─ Container (Docker / Firecracker) ─────┐
│                                         │
│  strata-agent binary                    │
│  ├─ axum server (A2A input)             │
│  ├─ agent loop (LLM + tools)            │
│  │   ├─ recall  → vector DB             │
│  │   ├─ remember → vector DB            │
│  │   └─ bash → sandboxed shell          │
│  ├─ state transition pipeline           │
│  └─ L1 poster (periodic commitments)    │
│                                         │
│  vector DB state (local filesystem)     │
└─────────────────────────────────────────┘
         │
         ▼ periodic
    L1 (Base) StrataRollup contract
```

**One process = one agent = one soul.** Each strata agent is its own container with its own vector DB, soul, and contract.

**Input sources (MVP):** A2A messages via HTTP.
**Input sources (future):** On-chain event watching, cron/scheduled tasks, webhooks.

### On-Chain Interactions

For MVP, on-chain reads/writes go through bash via foundry CLI tools (`cast call`, `cast send`). The LLM composes these as needed.

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
| `strata-openvm` | OpenVM guest program + host prover binary | Done (app proofs working) |
| `contracts` | StrataRollup Solidity contract with state continuity verification | Done |
| `strata-agent` | Host runtime — LLM, tools, prover, L1, A2A, reconstruction | Done (4a-4i) |

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

**Design philosophy:** Minimal tool set inspired by Pi agent. What you leave out matters more than what you put in. The LLM composes primitives to do anything complex.

**MVP tools (3):**
- `recall(query)` — search memory. Embeds query, runs `VectorDB::query()` hamming distance search, returns matching entries.
- `remember(text)` — store memory. Embeds text, appends to `VectorDB` via `VectorDB::append()`, returns memory ID.
- `bash(command)` — execute a shell command. The escape hatch — covers fetch (curl), on-chain reads (cast call), signing, and anything else the LLM can compose from CLI tools.

`recall` and `remember` must be dedicated tools because they touch internal state (vector DB) not exposed via CLI. Everything else goes through bash.

**Future tool evolution:**

| Tool | Domain | MVP | Future |
|------|--------|-----|--------|
| `recall` / `remember` | Memory | dedicated | same |
| `bash` | Off-chain actions | shell escape hatch | **Monty** (sandboxed Python repl, codemode) |
| `bash` (via `cast`) | On-chain actions | shell escape hatch | **Weiroll VM** (composable tx scripts) |

The progression: bash → Monty for off-chain, bash → Weiroll for on-chain. Monty and Weiroll both follow the same pattern — the LLM writes a script, the runtime executes it. This avoids enumerating every possible action as a dedicated tool.

**Implementation:**
- Define a `Tool` enum with three variants: `Recall`, `Remember`, `Bash`
- Match LLM function call responses to tool implementations
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

### Step 5: Codemode — Monty + Weiroll (strong-to-have)

After the core agent works with recall + remember + bash:

**5a: Monty (off-chain codemode)**
1. Add `pydantic-monty` (Rust crate) as a dependency to `strata-agent`.
2. Register host functions (`fetch`, `recall`, `remember`, `sign`) with the Monty runtime.
3. Replace `bash` tool with `codemode` — the LLM writes a Python script, Monty executes it, host functions are fulfilled by existing Rust implementations.
4. Results feed into the state transition pipeline as before.

**5b: Weiroll VM (on-chain codemode)**
1. Integrate Weiroll VM for composable on-chain transaction scripting.
2. The LLM writes a Weiroll script that batches multiple contract interactions into a single atomic transaction.
3. Replaces bash-via-cast for on-chain actions with a purpose-built, sandboxed execution environment.

### Step 4i: OpenVM SDK Integration — DONE

**What landed:**
- Fixed all OpenVM crate dependencies from crates.io placeholders to git refs (v1.5.0).
- Guest program updated to v1.5.0 API (`native_keccak256` intrinsic, `reveal_u32` for public values).
- Host binary (`strata-openvm/src/main.rs`) fully implemented with three subcommands:
  - `cargo run` — demo mode: executes guest in VM, verifies public values match local transition.
  - `cargo run -- prove --input <file> --level app` — generates real STARK proof (~seconds, ~7.5 GB RAM).
  - `cargo run -- generate-verifier` — generates Solidity verifier contracts from SDK.
- Agent prover module wired with optional `ProverConfig` (`PROVER_DIR` / `PROOF_LEVEL` env vars).
- Server generates real proofs when prover is configured, falls back to `vec![]` (mock) when not.
- Public values buffer set to 128 bytes with unused words zero-filled (soundness fix).

**Verification decision: off-chain STARK, not on-chain EVM proofs.**

EVM-verifiable proofs require wrapping STARK proofs in Halo2/BN254 (the "STARK → EVM bridge"). This is:
- Slow: ~10+ minutes per proof on consumer hardware (STARK aggregation + Halo2 circuit synthesis + KZG proving over BN254).
- RAM-heavy: 16-32 GB peak. Failed on 16 GB machine (needed KZG SRS params file not yet downloaded).
- Expensive to host: $300-500/mo for a dedicated prover box.

App-level STARK proofs are fast (~seconds) and fully verify the transition's correctness. The only thing lost is *on-chain* verification — anyone can still verify the proof off-chain by re-running the guest program.

**Current approach:**
- Contract uses operator signature trust (`MockStrataRollup` pattern).
- App STARK proofs generated per transition, served via API for independent verification.
- Memory content posted as calldata for reconstruction/DA.
- Proof bytes NOT posted on-chain (2 MB per proof = too expensive for calldata).

**Future path to on-chain verification:**
1. **Batch proving** — accumulate N transitions, prove once/day. Amortizes Halo2 wrapping cost to ~$0.10-0.50/day on a spot instance.
2. **Proving networks** (Succinct, Gevulot) — outsource Halo2 wrapping for ~$0.01-0.10/proof.
3. **Native STARK verifier precompiles** on L2s — eliminates Halo2 wrapping entirely.
4. The EVM proof code is already written (`--level evm`) — just needs KZG params + beefy hardware.

### Step 6: Demo

The MVP demo tells this story:

1. **Create** — deploy StrataRollup contract on Base with a soul document. Spin up strata-agent.
2. **Interact** — have conversations. Watch the agent build memories, query the vector DB, respond. Each interaction produces a ZK proof and posts state to L1.
3. **Verify** — show the state root updating on-chain. Show the STARK proof can be independently verified off-chain by anyone.
4. **Kill** — shut down the agent. Delete all local state.
5. **Reconstruct** — run reconstruction from the contract address alone. Download soul + calldata + state roots. Replay from genesis.
6. **Prove** — interact with the reconstructed agent. Show it remembers everything. Verify the state root matches.

---

## Updated Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| Language | Rust | no_std for guest, std for host |
| ZK Prover | **OpenVM** (was Jolt) | STARK proofs for off-chain verification; EVM proofs deferred (see Step 4i) |
| Hash Function | **Keccak256** (was Blake3) | OpenVM precompile + EVM opcode |
| Infrastructure | Commonware primitives | storage, codec, cryptography, runtime |
| Tool Runtime | **Monty** | Minimal Python interpreter in Rust, sandboxed, snapshotable |
| On-Chain | alloy + Base L1 | StrataRollup contract, ERC-8004 |
| Embeddings | Binary vectors, hamming distance | Unchanged |
| LLM | Claude API (via reqwest) | No framework, direct HTTP |

## What's Deferred

- **Full A2A** (streaming, push notifications) — basic request/response is sufficient
- **x402 inbound** (getting paid) — outbound only for MVP
- **Memory consolidation** — core memories accumulate without compression
- **Snapshots** — full replay from genesis is fine at MVP scale
- **Codemode (Monty)** — start with bash as escape hatch, add Monty for ephemeral Python codemode when round-trip overhead becomes a bottleneck

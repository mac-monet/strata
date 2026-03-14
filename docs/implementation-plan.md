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

**What changes:**
- `strata-jolt/` is retired. Replace with `strata-openvm/` (or inline into `strata-agent`).
- The OpenVM guest program is a thin wrapper that reads inputs via `openvm::io::read()`, calls the existing `strata_proof::transition()`, and reveals the new state root via `openvm::io::reveal_bytes32()`.
- The host orchestration (currently 70 lines of Jolt SDK calls in `strata-jolt/src/main.rs`) is replaced by OpenVM CLI commands or the OpenVM SDK.
- All core crates (`strata-core`, `strata-proof`, `strata-vector-db`) are unchanged.

**OpenVM guest entry point (replaces Jolt guest):**

```rust
#![no_std]
#![no_main]

openvm::entry!(main);

fn main() {
    let state: CoreState = openvm::io::read();
    let nonce: u64 = openvm::io::read();
    let witness: Witness = openvm::io::read();

    let new_state = strata_proof::transition::<Keccak256Hasher>(
        state, Nonce::new(nonce), &witness
    ).expect("transition failed");

    openvm::io::reveal_bytes32(new_state.vector_index_root.as_bytes());
}
```

**OpenVM Solidity verifier (rollup contract):**

```solidity
import "openvm-solidity-sdk/v1.5/OpenVmHalo2Verifier.sol";

contract StrataRollup {
    IOpenVmHalo2Verifier verifier;
    bytes32 public stateRoot;

    function submitTransition(
        bytes calldata publicValues,
        bytes calldata proofData,
        bytes32 appExeCommit,
        bytes32 appVmCommit
    ) external {
        verifier.verify(publicValues, proofData, appExeCommit, appVmCommit);
        stateRoot = bytes32(publicValues);
    }
}
```

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

**What changes:**
- `strata-proof`: implement `Keccak256Hasher` for the existing `GuestHasher` trait. Remove `Blake3Hasher` (or keep for tests).
- `strata-core`: update `ContentHash::digest()` and `SoulHash::digest()` to use keccak256. Update the spec docs that reference SHA-256 or Blake3.
- `strata-vector-db`: switch the MMR hasher from `commonware_cryptography::blake3` to a keccak-based hasher.
- Guest `Cargo.toml`: add `openvm-keccak256` dependency.
- Guest `openvm.toml`: add `[app_vm_config.keccak]` to enable the precompile.

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

**How it works:**
1. Agent receives a task/interaction
2. LLM generates a Python script that orchestrates host function calls
3. Monty executes the script in sandbox, pausing at each host function call
4. Host fulfills the call (HTTP, chain interaction, DB query) and returns the result
5. Script completes, results feed into the state transition pipeline
6. The script itself is outside the proof boundary (same as LLM inference)

**What gets proven:** The state transitions that result from script execution (memory appends, nonce increments, merkle root updates). The script is the proposer's strategy — the proof verifies the bookkeeping, not the strategy.

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

**Flow:**
1. LLM (or Monty script) calls `call_contract(to, method, args)`
2. Host builds the transaction using alloy
3. Hard constraints are checked (spending limits, allowed targets)
4. Transaction is submitted as part of the proof batch
5. Rollup contract verifies the proof and executes the action
6. Result is recorded as a state transition (memory entry with tx hash)

### Agent Runtime: Purpose-Built (no framework)

Build a lean host runtime from scratch. No Rig, no OpenClaw fork, no agent framework.

**Why:**
- Rig adds indirection for provider abstraction we don't need (targeting 1-2 LLM providers)
- OpenClaw/Moltis/IronClaw are personal assistant platforms with channels, voice, scheduling — all irrelevant
- The host layer is ~500-1k lines of Rust: LLM client, tool dispatch, embedding generation, state transition pipeline

**What `strata-agent` contains:**
- LLM client (call Claude/OpenAI API, handle streaming, function calling)
- Tool dispatch loop (match function calls → run handlers → return results)
- Embedding generation (text → binary vectors via embedding API)
- Witness preparation (build Witness from proposed memory updates)
- OpenVM prover invocation (prove state transitions)
- L1 posting (submit proofs + calldata to rollup contract via alloy)
- HTTP/A2A server (receive interactions, serve Agent Card)
- Reconstruction replay (rebuild agent from on-chain data)
- x402 HTTP client middleware (automatic payment on 402 responses)

---

## Implementation Steps

### Step 1: Switch hash function to Keccak256

**Crates affected:** `strata-core`, `strata-proof`, `strata-vector-db`

1. In `strata-proof/src/hasher.rs`: add `Keccak256Hasher` implementing `GuestHasher`. For native (non-OpenVM) builds, use the `tiny-keccak` crate or similar. For OpenVM guest builds, use `openvm-keccak256`.
2. In `strata-core`: update `ContentHash::digest()` and `SoulHash::digest()` to use keccak256 instead of blake3/sha256. These are used on the host side, so use `tiny-keccak` or equivalent.
3. In `strata-vector-db`: replace the `commonware_cryptography::blake3` hasher with a keccak-based hasher for the Journaled MMR.
4. Update all tests to use `Keccak256Hasher`.
5. Update docs that reference SHA-256 or Blake3 as the hash function.

### Step 2: Migrate proving from Jolt to OpenVM

**Crates affected:** new `strata-openvm/` directory, `strata-proof` (minor)

1. Create `strata-openvm/` as a new directory (outside the workspace, like `strata-jolt/`).
2. Write the OpenVM guest program as a `no_std` binary that:
   - Reads `CoreState`, nonce, and `Witness` via `openvm::io::read()`
   - Calls `strata_proof::transition::<Keccak256Hasher>()`
   - Reveals the new `vector_index_root` via `openvm::io::reveal_bytes32()`
3. Add `serde::Serialize`/`Deserialize` derives to all types that pass through `openvm::io::read()` — `CoreState`, `Nonce`, `Witness`, `MemoryEntry` in `strata-core` and `strata-proof`. The `serde` feature flag already exists; make it required for the OpenVM guest.
4. Create `openvm.toml` config enabling `[app_vm_config.keccak]` for the keccak precompile.
5. Verify the guest builds with `cargo openvm build`.
6. Verify local execution with `cargo openvm run`.
7. Generate keys and prove with `cargo openvm keygen` and `cargo openvm prove app`.
8. Retire `strata-jolt/` (move to `archive/` or delete).

### Step 3: Write the rollup contract

**Location:** `contracts/`

1. Initialize a Foundry project in `contracts/`.
2. Install the OpenVM Solidity SDK: `forge install openvm-org/openvm-solidity-sdk`.
3. Write `StrataRollup.sol`:
   - State: `stateRoot` (bytes32), `nonce` (uint64), `soulText` (string, set at construction), `verifier` (IOpenVmHalo2Verifier address)
   - `constructor(string soulText, address verifier)` — sets initial state, stores soul text
   - `submitTransition(bytes publicValues, bytes proofData, bytes32 appExeCommit, bytes32 appVmCommit)` — verifies proof via OpenVM verifier, updates stateRoot
   - `getSoulText()` — returns the soul document (public, anyone can read)
   - `getStateRoot()` — returns current state root
4. Write tests using Foundry's test framework.
5. Optionally add ERC-8004 identity registration in the constructor.

### Step 4: Build `strata-agent` (host runtime)

**Location:** `crates/strata-agent/`

This is the single host-side crate. Build it incrementally:

#### 4a: LLM Client
- HTTP client (use `reqwest`) that calls the Anthropic API (Claude)
- Support system prompt + conversation history + function calling
- Parse tool use responses, return structured tool calls
- ~100-200 lines

#### 4b: Embedding Client
- Call an embedding API to generate embeddings from text
- Convert float embeddings to binary vectors (threshold at median)
- Return `BinaryEmbedding` ([u64; 4])
- ~50-100 lines

#### 4c: Tool Dispatch
- Define host functions as an enum or trait: `fetch`, `recall`, `remember`, `sign`, `call_contract`, `read_contract`
- Match LLM function call responses to host function implementations
- Execute and return results to the LLM
- ~100-200 lines

#### 4d: State Transition Pipeline
- After LLM produces memory updates:
  1. Generate embeddings for new text
  2. Append to VectorDB (get new MMR root)
  3. Build Witness (old peaks, old leaf count, new entries)
  4. Invoke OpenVM prover (via CLI or SDK)
  5. Get proof back
- ~100-200 lines

#### 4e: L1 Posting
- Use alloy to interact with the StrataRollup contract on Base
- Submit proof + public values via `submitTransition()`
- Post raw memory content as calldata alongside the proof
- ~100-150 lines

#### 4f: HTTP/A2A Server
- HTTP server (use `axum`) exposing:
  - A2A endpoint for receiving messages from other agents
  - Agent Card endpoint (JSON describing capabilities)
  - Health/status endpoint
- Parse incoming A2A messages, feed into the state transition pipeline
- ~200-300 lines

#### 4g: Reconstruction
- Given only a contract address on Base:
  1. Read soul text from contract
  2. Read all calldata (memory content) from transaction history
  3. Replay all state transitions from genesis
  4. Rebuild VectorDB from replayed entries
  5. Verify final state root matches on-chain state root
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

## Updated Crates

| Crate | Role | Status |
|-------|------|--------|
| `strata-core` | Canonical shared types, serialization, validation (no_std) | Done — needs keccak migration |
| `strata-proof` | ZK transition logic — MMR ops, nonce validation, integrity checks | Done — needs keccak migration |
| `strata-vector-db` | Binary vector DB over Journaled MMR — hamming queries, merkle commitment | Done — needs keccak migration |
| `strata-openvm` | OpenVM guest program + proving scaffold | **To build** (replaces strata-jolt) |
| `strata-agent` | Host runtime — LLM, tools, prover, L1, A2A, reconstruction | **To build** |
| `contracts` | StrataRollup Solidity contract with OpenVM verifier | **To build** |

## What's Deferred

- **Full A2A** (streaming, push notifications) — basic request/response is sufficient
- **x402 inbound** (getting paid) — outbound only for MVP
- **Memory consolidation** — core memories accumulate without compression
- **Snapshots** — full replay from genesis is fine at MVP scale
- **Monty** — start with fixed tool calling, add Monty when round-trip overhead becomes a bottleneck

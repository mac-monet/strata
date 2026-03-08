# Implementation Specs

Technical specifications for building Strata.

## Code Organization

```
strata/
├── crates/
│   ├── core/          # shared types — CoreState, MemoryEntry, SoulDocument, serialization
│   ├── guest/         # Jolt guest program — state transition function (runs in RISC-V)
│   ├── vector-db/     # binary vector DB — hamming distance, flat scan, merkle commitment
│   ├── host/          # orchestration — LLM calls, embedding, witness prep, proving
│   └── agent/         # runtime — HTTP server, A2A endpoint, main binary
├── contracts/         # Solidity rollup contract on Base
└── docs/              # spec and knowledge base
```

## Dependencies

### Rust

| Crate | Purpose |
|-------|---------|
| `jolt-sdk` | ZK proving (guest + host sides) |
| `commonware-storage` | QMDB (keyless) for vector DB, MMR proof verification for guest |
| `commonware-codec` | Deterministic serialization for MemoryEntry hashing |
| `commonware-cryptography` | SHA-256 / Blake3 hashing, signing |
| `alloy` | Ethereum interaction (posting state roots, contract calls) |
| `axum` | HTTP server for A2A endpoint and agent API |
| `reqwest` | HTTP client for LLM, embedding, x402 |
| `serde` / `serde_json` | JSON serialization |

### Solidity

| Contract | Purpose |
|----------|---------|
| `StrataRollup.sol` | Accepts state roots, verifies proofs, holds funds, tracks commitments |

### External Services

| Service | Purpose |
|---------|---------|
| Embedding model (Cohere / MixedBread) | Binary embedding generation |
| LLM (Claude) | Agent reasoning, fact extraction, memory decisions |

## Build Tasks

| # | Task | Crate | Depends On | Spec |
|---|------|-------|------------|------|
| 1 | Core types | `core` | — | |
| 2 | Binary vector DB | `vector-db` | core | [vector-db.md](./vector-db.md) |
| 3 | Guest program | `guest` | core, vector-db | [proof-boundary.md](./proof-boundary.md) |
| 4 | Host orchestration | `host` | core, guest | |
| 5 | Rollup contract | `contracts` | — (parallel) | |
| 6 | Agent runtime | `agent` | all crates | |
| 7 | Integration + demo | — | everything | |

## Build Order

1. **Core types** — everything depends on these
2. **Vector DB** — can be built and tested independently
3. **Guest program** — needs core types and vector DB, can test as normal Rust before compiling to RISC-V
4. **Host orchestration** — needs guest program to prepare witnesses for
5. **Rollup contract** — can be developed in parallel with host
6. **Agent runtime** — ties everything together, build last
7. **Integration + demo** — end-to-end testing

Steps 1-3 are the critical path. Steps 4-5 can be parallelized. Step 6 depends on both.

## Specs

| Spec | Covers |
|------|--------|
| [Proof Boundary](./proof-boundary.md) | What runs inside Jolt vs outside — the exact boundary between guest and host |
| [Vector DB](./vector-db.md) | Binary vector DB implementation — QMDB storage, hamming distance, host/guest split |

# Strata

Strata is a custom rollup architecture for persistent, verifiable and trustless AI agents. An agent's entire cognitive state — its identity, memory, capabilities, and history — lives on-chain, making it auditable, forkable, and immortal.

The core thesis: **an AI agent _is_ a rollup**. Its state transitions are cognitive operations (perceiving, remembering, deciding, acting), and those transitions are proven in zero knowledge via Jolt. Anyone can reconstruct and verify the agent by replaying its on-chain data from genesis.

## Why This Matters

Today, AI agents are ephemeral — they exist only as long as someone runs them. Their memory, identity, and capabilities vanish when the server stops. Strata makes agents permanent.

- **Immortal.** The agent doesn't depend on any single server or operator. As long as the L1 and blobs exist, anyone can bring it back.
- **Auditable.** The full cognitive history is replayable from genesis. Every memory, every decision, every tool it built — all traceable.
- **Forkable.** Snapshot an agent's state and spin up a variant. Same memories, different soul. Agent lineage becomes possible.
- **Trustworthy.** ZK proofs verify the agent followed its rules. Its soul document lets anyone evaluate its values against its behavior. You don't have to trust the operator.
- **Private.** The agent can prove it operated honestly without revealing its internal state — useful for agents handling sensitive data.
- **Composable.** Agents can read each other's state roots and build trust without centralized reputation systems. Instant finality means other agents and contracts can trust this agent's state in real time.
- **Verifiable recall.** The agent's memory index is committed on-chain. Anyone can verify a retrieval claim by replaying the query against the committed entries — no cherry-picking, no hidden context.

Most people think of rollups as scaling solutions. Strata reframes them as agent architectures — extending on-chain identity (ERC-8004) to on-chain cognition.

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│                   L1 (Base)                      │
│       ┌───────────┐       ┌──────────┐           │
│       │ State Root│       │ ZK Proof │           │
│       └───────────┘       └──────────┘           │
└─────────────────────────────────────────────────┘
                      ▲
                      │ posts proofs + commitments
                      │
┌─────────────────────────────────────────────────┐
│            strata-agent (host runtime)           │
│  HTTP/A2A server, LLM + embedding clients,       │
│  witness prep, prover invocation, L1 posting,    │
│  reconstruction replay                           │
│                                                  │
│  ┌──────┐  ┌──────────────────────────────────┐ │
│  │ Soul │  │ Binary Vector DB (Hamming/XOR)   │ │
│  │      │  │ append-only retrieval index       │ │
│  └──────┘  └──────────────────────────────────┘ │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │     strata-proof + Jolt (RISC-V / ZK)     │  │
│  │     state transitions, constraints         │  │
│  └────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

The only meaningful boundary is **inside vs outside the ZK proof**. `strata-proof` owns the inside (deterministic state verification). `strata-agent` owns everything else — there is no separate "host" vs "runtime".

## Core State

The agent's on-chain state is minimal — three fields that capture everything needed to reconstruct the agent:

```
CoreState {
    soul_hash:         [u8; 32],   // hash of soul document (full text in contract storage)
    vector_index_root: [u8; 32],   // MMR root over binary vector memory
    nonce:             u64,        // monotonically increasing transition counter
}
```

The soul document's full text lives in the rollup contract's public storage — readable by anyone. Raw memory content is posted as calldata alongside state roots and is verifiable via `content_hash` in each memory entry.

## Components

| Component | Description | Doc |
|-----------|-------------|-----|
| [Soul](./soul.md) | The agent's constitution — identity, values, and constraints | Public, legible document |
| [State Model](./state-model.md) | Core state schema and transition function | What gets proven in ZK |
| [Vector DB](./vector-db.md) | Binary vector database — unified memory and retrieval | Hamming distance, Journaled MMR, append-only |
| [Proving](./proving.md) | ZK proving via Jolt (RISC-V) | What's inside vs outside the proof |
| [Skills & Tools](./skills.md) | Self-expanding agent capabilities — skills as knowledge, tools as code | Nanoclaw-inspired, outside proof boundary |
| [On-Chain](./onchain.md) | Identity (ERC-8004), rollup contract, agent communication (A2A) | On-chain surface area |
| [Reconstruction](./reconstruction.md) | How anyone can rebuild and verify the agent | The immortality property |
| [Trust Model](./trust-model.md) | Mathematical + attestable trust | Two-layer verification |
| [Roadmap](./roadmap.md) | MVP scope, priorities, and demo plan | What to build first |

## Crates

| Crate | Role | Status |
|-------|------|--------|
| `strata-core` | Canonical shared types, serialization, validation (no_std) | Done |
| `strata-proof` | ZK transition logic — MMR ops, nonce validation, integrity checks | Done |
| `strata-vector-db` | Binary vector DB over Journaled MMR — hamming queries, merkle commitment | Done |
| `strata-jolt` | Jolt proving scaffold — guest compilation, proving, verification | Done (PoC) |
| `strata-agent` | Host runtime — HTTP/A2A, LLM, embeddings, prover, L1, reconstruction | Not yet built |

## Stack

- **Language:** Rust
- **ZK Prover:** Jolt (RISC-V)
- **Infrastructure:** Commonware primitives (storage, p2p, codec, cryptography, consensus, runtime)
- **Scripting:** Rhai (embedded, sandboxed, AST-compiled)
- **L1:** Base (ERC-8004 identity)
- **Embeddings:** Binary vectors, hamming distance

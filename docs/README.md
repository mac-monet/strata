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
│               Strata Agent Rollup                │
│                                                  │
│  ┌──────┐  ┌──────────────────────────────────┐ │
│  │ Soul │  │ Binary Vector DB (Hamming/XOR)   │ │
│  │      │  │ core memory + retrieval index     │ │
│  └──────┘  └──────────────────────────────────┘ │
│                                                  │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │        Jolt Prover (RISC-V / ZK)          │  │
│  │     state transitions, constraints         │  │
│  └────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

## Core State

The agent's on-chain state is minimal — three fields that capture everything needed to reconstruct the agent:

```
CoreState {
    soul:              SoulDocument,   // identity, values, constraints
    vector_index_root: Hash,           // merkle root over binary vector memory
    nonce:             u64,            // monotonically increasing transition counter
}
```

Raw interaction data (conversation logs, reasoning traces) is posted as calldata alongside state roots and is verifiable via `content_hash` in each memory entry.

## Components

| Component | Description | Doc |
|-----------|-------------|-----|
| [Soul](./soul.md) | The agent's constitution — identity, values, and constraints | Public, legible document |
| [State Model](./state-model.md) | Core state schema and transition function | What gets proven in ZK |
| [Vector DB](./vector-db.md) | Binary vector database — unified memory and retrieval | Hamming distance, QMDB, core memory tagging |
| [Proving](./proving.md) | ZK proving via Jolt (RISC-V) | What's inside vs outside the proof |
| [Skills & Tools](./skills.md) | Self-expanding agent capabilities — skills as knowledge, tools as code | Nanoclaw-inspired, outside proof boundary |
| [On-Chain](./onchain.md) | Identity (ERC-8004), rollup contract, agent communication (A2A) | On-chain surface area |
| [Reconstruction](./reconstruction.md) | How anyone can rebuild and verify the agent | The immortality property |
| [Trust Model](./trust-model.md) | Mathematical + attestable trust | Two-layer verification |
| [Roadmap](./roadmap.md) | MVP scope, priorities, and demo plan | What to build first |

## Stack

- **Language:** Rust
- **ZK Prover:** Jolt (RISC-V)
- **Infrastructure:** Commonware primitives (storage, p2p, codec, cryptography, consensus, runtime)
- **Scripting:** Rhai (embedded, sandboxed, AST-compiled)
- **L1:** Base (ERC-8004 identity)
- **Embeddings:** Binary vectors, hamming distance

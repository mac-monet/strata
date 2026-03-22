# AGENTS.md

## What is Strata?

Strata is a ZK rollup for AI cognition. The agent's entire cognitive state — identity, memory, and decisions — is committed on-chain to a custom rollup contract on Base. Every memory is a binary vector indexed in a Merkle Mountain Range, and a ZK proof program verifies state transitions. The agent carries an on-chain soul document declaring its values and constraints.

## Live Deployment

**URL:** https://strata-agent-production.up.railway.app/

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Web chat interface |
| `/a2a` | POST | A2A JSON-RPC endpoint (message/send, tasks/get) |
| `/.well-known/agent.json` | GET | A2A Agent Card — capabilities, skills, live state, ERC-8004 identity |
| `/.well-known/agent-registration.json` | GET | ERC-8004 registration document with trust model |
| `/proof/{nonce}` | GET | Retrieve ZK proof for a state transition by nonce |
| `/health` | GET | Health check |

## How to Interact

### Via A2A (Agent-to-Agent Protocol)

Send a JSON-RPC request to `/a2a`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "message/send",
  "params": {
    "message": {
      "role": "user",
      "parts": [{ "type": "text", "text": "What do you know about ERC-8004?" }]
    }
  }
}
```

Multi-turn conversations are supported via `contextId` — include the `contextId` from a previous response to continue a session.

### Via Web UI

Visit the root URL in a browser for a chat interface.

## On-Chain Presence

| Component | Details |
|-----------|---------|
| **Rollup Contract** | `0x97D08bfD7A3fa7bD5E888C0a0bE26691dB9c4087` on Base Mainnet |
| **ERC-8004 Agent ID** | 26268 |
| **ERC-8004 Registry** | `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` on Base Mainnet |
| **Chain** | Base Mainnet (eip155:8453) |

The rollup contract stores the soul document in public storage and accepts state root commitments. The ERC-8004 registration links the agent's on-chain identity to its A2A endpoint and rollup contract.

## Agent Capabilities

- **Verifiable Memory** — persistent semantic memory using a custom binary vector database. Memories are embedded as 256-bit binary vectors, indexed in a Merkle Mountain Range, and committed on-chain. Retrieval uses hamming distance (XOR + popcount), chosen for ZK-friendliness and on-chain efficiency.
- **On-Chain Query** — can execute shell commands to query blockchain state, read contracts, and investigate on-chain activity.
- **State Verification** — the agent's state root, soul hash, and memory index root are committed on-chain. Anyone can verify integrity by reading the rollup contract or reconstructing the agent from genesis.

## Architecture

```
L1 (Base)
├── StrataRollup contract (state roots, soul document, commitments)
└── ERC-8004 Registry (agent identity, metadata)
         ▲
         │ posts commitments
         │
strata-agent (Rust)
├── A2A server (Axum)
├── LLM client (Venice)
├── Embedding client (binary vectors)
├── Binary Vector DB (Journaled MMR, hamming distance)
├── Soul document (system prompt + constraints)
├── ZK proof program (RISC-V, state transition verification)
└── Reconstruction engine (replay from genesis)
```

## Crates

| Crate | Role |
|-------|------|
| `strata-core` | Shared types, serialization, validation (no_std) |
| `strata-proof` | ZK transition logic — MMR ops, nonce validation, integrity checks |
| `strata-vector-db` | Binary vector DB over Journaled MMR — hamming queries, merkle commitment |
| `strata-openvm` | ZK proving scaffold — RISC-V guest program |
| `strata-agent` | Host runtime — HTTP/A2A, LLM, embeddings, L1 posting, reconstruction |

## Key Design Decisions

- **Binary vectors over floats** — 256-bit vectors are ZK-friendly (hamming distance = XOR + popcount, native on RISC-V) and orders of magnitude cheaper for on-chain storage than float32 embeddings.
- **Agent = rollup** — cognitive state transitions are the rollup's state transition function. The agent's nonce, soul hash, and memory index root form the complete on-chain state.
- **Soul document** — a plain-text constitution stored in public contract storage. Declares the agent's values and hard constraints. Readable by anyone.
- **Reconstruction** — the agent can be fully reconstructed from on-chain data alone by replaying state transitions from genesis. No trust in the operator required.

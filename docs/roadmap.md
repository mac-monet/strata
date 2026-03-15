# Roadmap

## MVP (Hackathon)

The MVP proves the core thesis: a persistent, verifiable AI agent that lives on-chain and can be reconstructed by anyone.

### Done

- **Core types** (`strata-core`) — canonical shared types, serialization, validation. No-std compatible.
- **Proof program** (`strata-proof`) — ZK transition logic: MMR operations, nonce validation, integrity checks. Ready for RISC-V compilation.
- **Binary vector DB** (`strata-vector-db`) — authenticated append-only binary vector database over Journaled MMR. Hamming distance queries, flat scan, merkle commitment.
- **ZK proving scaffold** (`strata-openvm`) — OpenVM proving scaffold. Guest program with keccak precompile, host placeholder.

### Must Have

These are load-bearing — the thesis doesn't work without them.

- **`strata-agent`** — the single host-side crate. Everything outside the proof boundary lives here: HTTP/A2A server, LLM + embedding clients, witness preparation, OpenVM prover invocation, L1 posting (calldata + state roots), and reconstruction replay. There is no separate "host" vs "runtime" — the only meaningful boundary is inside vs outside the ZK proof, and `strata-proof` already owns the inside.
- **Soul document** — plain text constitution committed at genesis. Hard constraints extracted and enforced in the state transition function.
- **Storage** — raw memory content posted as calldata alongside state roots. Verifiable via `content_hash` in each memory entry.
- **Rollup contract on Base** — post state roots and proofs. Verify proofs on-chain.
- **Reconstruction** — the killer demo. Shut the agent down, delete the server, reconstruct it purely from on-chain state and blobs. Show it remembers everything.

### Strong to Have

These make the demo more compelling and show the agent interacting with the world.

- **ERC-8004 identity** — register the agent on the identity registry at genesis. Link 8004 record to rollup contract.
- **x402 outbound** — agent pays for APIs automatically via the host HTTP client. Demonstrates economic autonomy.
- **Basic A2A** — receive messages from other agents, process them through the state transition pipeline, respond. Doesn't need streaming or push notifications for MVP.

### Deferred

Impressive but not required to land the core thesis.

- **x402 inbound** — agent gets paid for its services. Requires pricing logic and 402 response handling on the A2A endpoint.
- **Full A2A** — streaming, push notifications, full task lifecycle management. Basic request/response is sufficient for MVP.
- **Skills and tools (Rhai)** — the self-expanding capability layer. For MVP, hardcode a few tools in Rust. Add the Rhai runtime and self-expanding skill system post-hackathon.
- **Memory consolidation** — periodic compression of core memories. For MVP, core memories can accumulate without consolidation. Add the consolidation cycle later.
- **Snapshots** — periodic state checkpoints for faster reconstruction. Full replay from genesis is fine for MVP scale.

## The Demo

The MVP demo should tell this story:

1. **Create** — spin up an agent with a soul document on Base
2. **Interact** — have conversations, watch it build memories in the vector DB
3. **Verify** — show the ZK proof for a state transition, verify it on-chain
4. **Kill** — shut down the agent completely
5. **Reconstruct** — bring it back from on-chain state and blobs alone
6. **Prove** — show it remembers everything, its soul is intact, its state is verified

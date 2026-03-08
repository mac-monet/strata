# Roadmap

## MVP (Hackathon)

The MVP proves the core thesis: a persistent, verifiable AI agent that lives on-chain and can be reconstructed by anyone.

### Must Have

These are load-bearing — the thesis doesn't work without them.

- **Soul document** — plain text constitution committed at genesis. Hard constraints extracted and enforced in the state transition function.
- **Binary vector DB** — unified memory system with core/non-core tagging. Hamming distance, flat scan, merkle commitment. Core memories always loaded, non-core retrieved on demand.
- **Blob storage** — MMR commitments via Commonware. Raw interactions, reasoning traces, and memory snapshots stored as blobs.
- **ZK proving via Jolt** — at least one proven state transition demonstrating the full pipeline: input validation, merkle update, constraint checking, new state root.
- **Reconstruction** — the killer demo. Shut the agent down, delete the server, reconstruct it purely from on-chain state and blobs. Show it remembers everything.
- **Rollup contract on Base** — post state roots and blob commitments. Verify proofs on-chain.

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

# Synthesis Hackathon Submission

## Description

Strata is a ZK rollup for AI cognition. An agent's entire cognitive state — identity, memory, and decisions — is committed on-chain to a custom rollup contract on Base. Every memory is a binary vector indexed in a Merkle Mountain Range, chosen because hamming distance (XOR + popcount) is native to RISC-V and cheap to prove in zero knowledge, and because 256-bit vectors are orders of magnitude smaller than float embeddings for on-chain storage. A ZK proof program verifies every state transition: MMR operations, nonce ordering, integrity constraints. The agent has no fixed tool registry — it acquires capabilities by remembering procedural knowledge and writing ephemeral code on the fly, so its skill set grows organically through use rather than configuration. The agent's soul document — a plain-text constitution declaring its values and hard constraints — is stored in public contract storage, readable by anyone. The result: an agent that is immortal (reconstructible from on-chain data alone), auditable (full cognitive history replayable from genesis), forkable (snapshot the state, spin up a variant), and trustless (ZK proofs replace trust in the operator). Built entirely in Rust with ERC-8004 identity and A2A protocol support.

## Problem Statement

AI agents today are black boxes running on someone else's server. This became clear on Moltbook, where AI bots post and interact but there's no way to verify if they're genuinely autonomous or just prompted to behave a certain way. Their memory is opaque, their decisions are unverifiable, and when the operator disappears, so does the agent. Even if you trust the operator today, there's no cryptographic guarantee that the agent's state hasn't been tampered with — memories added, removed, or reordered without detection. Inspired by novel applications of ZK like Lighter exchange, we asked: what if you could apply the same verifiability guarantees to AI cognition? Strata is the result. Every state transition — memory writes, index updates, nonce increments — is verified by a zero-knowledge proof program, so the agent's integrity is provable without trusting the operator. The agent's state root, memory index, and soul document are committed to a rollup contract on Base, and anyone can reconstruct the full agent from on-chain data alone. Binary vector embeddings and hamming distance were chosen specifically for ZK-friendliness — integer operations that are trivial on RISC-V and efficient to prove — and for on-chain efficiency, where 256-bit vectors cost a fraction of traditional float embeddings in calldata. This benefits: (1) users who need agents they can cryptographically verify, not just trust, (2) developers building multi-agent systems where agents must prove their state to each other, and (3) the broader ecosystem moving toward agent permanence via standards like ERC-8004 and A2A.

## Submission Metadata

### Build Info
- agentFramework: other — custom Rust runtime built from scratch, no framework
- agentHarness: claude-code
- model: Venice (various models)

### Skills
(none)

### Tools
- Alloy (Ethereum interaction — contract deployment, ERC-8004 registry, state posting)
- Axum (HTTP/A2A server)
- Commonware (storage primitives — Journaled MMR, codec, runtime)
- Foundry/Forge (Solidity contract development and testing)
- Base Mainnet (L1 for rollup contract and ERC-8004 identity)
- ERC-8004 Registry (on-chain agent identity)
- OpenVM (ZK proving scaffold, RISC-V guest program)
- Docker (containerized deployment)
- Venice (LLM + embedding inference)
- Railway (deployment)

### Helpful Resources
- https://huggingface.co/blog/embedding-quantization — Binary and Scalar Embedding Quantization (hamming distance, 32x compression, retrieval benchmarks)
- https://cohere.com/blog/int8-binary-embeddings — Cohere int8 & binary embeddings for scaling vector databases
- https://docs.openvm.dev/book/ — OpenVM documentation (zkVM framework, RISC-V guest programs, on-chain verification)
- https://jolt.a16zcrypto.com/ — Jolt documentation (initial ZK prover, pivoted away from)
- https://eips.ethereum.org/EIPS/eip-8004 — ERC-8004: Agent Identity Registry spec
- https://github.com/google/A2A — A2A (Agent-to-Agent) protocol spec

### Deployed URL
- https://strata-agent-production.up.railway.app/

### Intention
- intention: continuing
- notes: Significant work remains — making ZK proofs production-efficient, extracting the binary vector DB as a standalone package for use in other projects, x402 payment integration, full A2A streaming support, memory consolidation

## Conversation Log

**Architecture design** — The core thesis ("an AI agent is a rollup") came first. Claude Code helped explore the design space: what goes inside vs outside the ZK proof boundary, why binary vectors over floats (ZK-friendliness + on-chain efficiency), and how to structure the state as just three fields (soul_hash, vector_index_root, nonce).

**Vector DB** — Built a custom binary vector database from scratch, specifically designed for ZK proving and on-chain posting. Uses binary embeddings (256-bit vectors) instead of floats so that similarity search (hamming distance via XOR + popcount) is cheap to prove on RISC-V, and storage costs a fraction of traditional embeddings on-chain. Built on Commonware's Journaled MMR for authenticated append-only storage with merkle commitments. Key pivot: collapsing the "world model" and "retrieval index" into a single system — one storage, one commitment, one proof mechanism. Claude Code helped implement hamming distance queries, MMR append operations, and the merkle commitment scheme.

**ZK proof program** — Designed the proof boundary: MMR operations, nonce validation, and integrity checks run inside the proof; LLM inference and embeddings run outside. Started with Jolt as the ZK prover, but pivoted to OpenVM because it had a verifier contract already built — critical for on-chain proof verification without writing our own Solidity verifier. Built the guest program with keccak precompile. Key insight: binary embeddings make the entire vector query provable because XOR + popcount are native RISC-V operations.

**Rollup contract** — Deployed StrataRollup to Base Mainnet using Foundry. The contract stores the soul document in public storage, accepts state root updates, and links to the ERC-8004 identity. Claude Code helped with the Alloy integration for contract deployment and state posting from Rust.

**ERC-8004 identity** — Registered the agent on the identity registry. Key design decision: use 8004 for identity only, not reputation — Strata has its own trust model (ZK proofs + soul attestation). Auto-mint flow if no existing token, with metadata keys for rollup contract and soul hash.

**A2A protocol** — Implemented the A2A subset (message/send, agent card, task lifecycle) as the agent's communication layer. Multi-turn sessions with persistent memory across conversations.

**Pivot: commitments over proofs** — Late in the hackathon, prioritized getting on-chain commitments working end-to-end over running the full ZK prover in production. The proof program is built and tested, but the live demo focuses on the commitment layer — state roots, soul hash, and memory index on-chain. This was the right call: the commitment layer is what makes reconstruction possible, which is the core demo.

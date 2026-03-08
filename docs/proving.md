# ZK Proving via Jolt

Strata uses Jolt, a RISC-V based ZK prover, to generate proofs of correct state transitions. The state transition function is written in Rust, compiled to RISC-V, and proven via Jolt. This means the agent's cognitive bookkeeping is mathematically verified without requiring anyone to trust the operator.

## Why Jolt

- **Write Rust, prove RISC-V**: no circuit DSLs, no custom constraint systems. The state transition function is normal Rust code.
- **General purpose**: RISC-V is a full instruction set. Any computation expressible in Rust can be proven.
- **Performance**: Jolt is designed for fast proving with minimal overhead.

## The Proof Boundary

The most important design decision is what goes inside the proof vs what stays outside.

### Inside the Proof (Guest Program)

The Jolt guest program handles all deterministic state operations:

**State validation**
- Input is well-formed and correctly signed
- Nonce is monotonically increasing (no replay, no reordering)

**Merkle tree operations**
- Vector index tree updates (adding/consolidating memory entries)
- Membership and inclusion proofs

### Outside the Proof (Host / Operator)

These operations are inherently non-deterministic or too expensive for ZK:

**LLM inference**
- Generating responses to interactions
- Extracting facts from conversations
- Producing consolidated summaries for core memories
- Deciding if new memories should be tagged as core

**Embedding generation**
- Converting text to binary vectors via an embedding model

**Skill execution**
- Running Rhai scripts that interact with external APIs and services

**Witness preparation**
- The host runs the LLM, prepares the proposed state update, and provides it to the guest as witness data
- The guest verifies that the update is structurally valid and constraint-compliant

## Proof Flow

```
1. Input arrives (interaction, consolidation trigger, etc.)
          │
2. Host (off-chain) runs LLM inference
   ├── extracts facts
   ├── generates memory updates
   ├── produces embeddings
   └── prepares witness data
          │
3. Guest program (in Jolt) receives:
   ├── current state root
   ├── input
   └── witness data (proposed updates from LLM)
          │
4. Guest verifies:
   ├── input validity (signature, nonce)
   ├── merkle updates (vector index)
   └── outputs new state root
          │
5. Jolt generates proof of correct execution
          │
6. Proof + new state root posted to L1
```

## What the Proof Guarantees

- **Integrity**: the new state was derived correctly from the old state + input
- **Consistency**: no replay, no reordering, no skipped transitions

## What the Proof Does Not Guarantee

- **Intelligence**: the LLM's responses are good or helpful
- **Semantic accuracy**: the summaries faithfully represent the raw data
- **Completeness**: the agent noticed everything it should have noticed

These are attestable properties, not provable ones. They are evaluated by examining the agent's behavior against its soul document over time.

## Privacy

ZK proofs enable a powerful privacy property: the agent can prove it followed its rules without revealing its internal state. This means:

- Private memories stay private
- Sensitive interactions aren't exposed
- The proof says "I operated honestly" without showing _how_ or _what_

This is especially valuable for agents handling sensitive data — financial decisions, personal information, private negotiations. The agent proves behavioral integrity without sacrificing confidentiality.

## Instant Finality

Unlike optimistic rollups (which have a 7-day challenge window), ZK proofs provide instant finality. The moment the proof is verified on L1, the agent's state is provably correct. This matters when other agents or contracts need to trust this agent's state in real time.

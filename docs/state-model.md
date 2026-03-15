# State Model

The state model defines what the agent "knows" and "is" at any point in time, and how that state evolves through interactions.

## Core State

The on-chain state is minimal — three fields that fully capture the agent's mind:

```rust
struct CoreState {
    soul_hash: [u8; 32],
    vector_index_root: [u8; 32],
    nonce: u64,
}
```

- **soul_hash**: Keccak256 hash of the soul document text. Full text lives in the rollup contract's public storage (see [soul.md](./soul.md))
- **vector_index_root**: MMR root over the binary vector memory — the agent's unified knowledge store and retrieval index (see [vector-db.md](./vector-db.md))
- **nonce**: Monotonically increasing counter for state transitions

Raw interaction data (conversation logs, reasoning traces) is posted as calldata alongside state roots. It's verifiable via `content_hash` in each `MemoryEntry` without needing a separate commitment. This is everything needed to reconstruct the agent.

## State Transitions

Every change to the agent's state is a state transition. The transition function takes the current state and an input, and produces a new state and an optional action:

```rust
fn transition(state: CoreState, input: Input) -> (CoreState, Option<Action>)
```

### Input Types

- **Interaction**: a message, query, or event from the outside world
- **Consolidation**: a periodic compression of core memories in the vector DB (see [vector-db.md](./vector-db.md))
- **Skill mutation**: the agent creates or modifies a skill (see [skills.md](./skills.md))
- **Soul amendment**: a modification to the soul document

### What the Transition Function Does

1. **Validates the input** — signed, nonced, well-formed
2. **Updates the vector index** — new memory entries added, core memories consolidated
3. **Produces an action** — the agent's response or external action, included as witness data

### Inside vs Outside the Proof

Not everything happens inside the ZK proof. The boundary is:

**Inside ZK proof (proven):**
- State validation (nonces, signatures, ordering)
- Vector index merkle tree updates (adding/consolidating memories)
- Structural integrity of all data

**Outside ZK proof (`strata-agent`, trusted to operator):**
- LLM inference (generating responses, extracting facts, summarizing)
- Skill execution (Rhai scripts interacting with external world)
- Embedding generation (producing binary vectors from text)
- HTTP/A2A server, witness preparation, L1 posting, reconstruction replay

The LLM is the **proposer** — it suggests what to remember and what to do. The ZK prover is the **verifier** — it proves the bookkeeping was done honestly and the rules were followed. The proof doesn't guarantee the agent is smart, it guarantees the agent is honest.

## Nonce and Ordering

Every transition has a monotonically increasing nonce. This prevents:
- Replay attacks (re-submitting old interactions)
- State rewriting (inserting transitions out of order)
- Forked histories (diverging from the canonical state)

## Genesis

The genesis state contains:
- The initial soul document
- An empty vector index
- Nonce 0

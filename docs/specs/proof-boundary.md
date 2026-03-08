# Proof Boundary

This document defines exactly what runs inside the Jolt guest program (proven in ZK) and what runs in the host (trusted to the operator).

## Guiding Principle

Prove the **bookkeeping**, not the **thinking**. The ZK proof guarantees the agent's state was updated correctly and its rules were followed. It does not guarantee the agent is intelligent or that its decisions are good.

The LLM is the **proposer** — it suggests state updates. Jolt is the **verifier** — it checks that those updates are structurally valid and constraint-compliant.

## Inside the Guest (Proven)

The guest program is a single function:

```rust
fn transition(
    current_state: CoreState,
    input: Input,
    witness: Witness,
) -> (CoreState, Option<Action>)
```

### Input Validation

- Verify the input nonce is exactly `current_nonce + 1` (prevents replay and reordering)
- Verify the input signature matches an authorized key
- Verify the input is well-formed (correct enum variant, required fields present)

### Vector DB Merkle Updates

When the witness proposes adding or modifying memory entries:

- Verify the old merkle root matches `current_state.vector_index_root`
- Apply the insertions/modifications to the merkle tree
- Compute the new merkle root
- The new root becomes `new_state.vector_index_root`

The witness provides: the new memory entries (id, embedding, content hash, core flag) and the merkle proof path for each insertion point.

The guest verifies: the merkle proofs are valid, the tree update is correct, the new root is computed honestly.

The guest does NOT verify: whether the embedding is "correct" for the content, whether the core flag is appropriate, whether the content is useful. Those are LLM decisions provided as witness data.

### Data Archive MMR Update

When new interaction data is archived:

- Verify the old MMR root matches `current_state.blob_archive_root`
- Append the new data hash to the MMR
- Compute the new MMR root
- The new root becomes `new_state.blob_archive_root`

The witness provides: the data hash being appended and the MMR proof path.

The guest verifies: the MMR append is structurally correct.

### Soul Constraint Checking

For each hard constraint in the soul document:

- Evaluate the constraint against the proposed state transition
- If any constraint is violated, reject the transition (return error / panic)

Examples of constraints the guest can enforce:
- **Data rules**: "content hash must not match any entry in the forbidden list"
- **Spending rules**: "action value must not exceed X"
- **Memory rules**: "entries tagged as critical must not be deleted"
- **Disclosure rules**: "action must include AI disclosure flag"

Constraints are compiled from the soul document into check functions at genesis. They are part of the guest program.

### Vector Queries (Retrieval Proving)

When the agent claims it retrieved specific memories for an interaction:

- Take the query embedding from the witness
- XOR + popcount against all entries in the committed index
- Verify the claimed top-k results are actually the nearest neighbors
- No entries were skipped or excluded

This proves **verifiable recall** — the agent retrieved honestly.

### Nonce Update

- Increment the nonce: `new_state.nonce = current_state.nonce + 1`

## Outside the Guest (Host)

### LLM Inference

- Generating responses to interactions
- Extracting facts and memories from conversations
- Deciding what to remember (core vs non-core)
- Producing consolidated summaries for core memories
- Deciding which tool to invoke

The LLM's outputs are packaged as witness data for the guest. The guest trusts the content but verifies the structure.

### Embedding Generation

- Converting text to 256-bit binary vectors via an embedding model
- The resulting vector is provided as witness data
- The guest commits it to the merkle tree but does not verify it represents the text

### Tool Execution

- Running tools (Rhai scripts or hardcoded Rust) that interact with external APIs
- Reading from external data sources
- Side effects (sending messages, making payments)
- Results feed back as inputs to the next state transition

### A2A Communication

- Receiving and parsing A2A messages
- Formatting and sending A2A responses
- Managing task lifecycle
- The communication itself is off-chain; the cognitive effects (memory updates) go through the guest

### Witness Preparation

The host is responsible for packaging everything the guest needs:

```rust
struct Witness {
    // Memory updates proposed by the LLM
    new_entries: Vec<MemoryEntry>,
    modified_entries: Vec<(EntryId, MemoryEntry)>,
    deleted_entries: Vec<EntryId>,

    // Merkle proofs for each update
    merkle_proofs: Vec<MerkleProof>,

    // Data to append to the archive
    archive_data_hash: Hash,
    mmr_proof: MmrProof,

    // For retrieval proving
    query_embedding: Option<[u64; 4]>,
    claimed_results: Option<Vec<MemoryEntry>>,

    // The action the agent wants to take
    proposed_action: Option<Action>,
}
```

## Summary

| Operation | Where | Why |
|-----------|-------|-----|
| Nonce validation | Guest | Deterministic, prevents replay |
| Signature verification | Guest | Deterministic, proves authorization |
| Merkle tree updates | Guest | Deterministic, proves state integrity |
| MMR updates | Guest | Deterministic, proves archive integrity |
| Soul constraint checks | Guest | Deterministic, proves rule compliance |
| Vector query verification | Guest | Deterministic, proves honest retrieval |
| LLM inference | Host | Non-deterministic, too expensive for ZK |
| Embedding generation | Host | Non-deterministic, external model |
| Tool execution | Host | Side-effectful, environment-dependent |
| A2A communication | Host | Network I/O, side-effectful |
| Witness preparation | Host | Orchestration logic |

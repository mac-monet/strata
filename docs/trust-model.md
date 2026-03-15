# Trust Model

Strata provides a two-layer trust model that separates what can be mathematically proven from what can be publicly evaluated.

## Layer 1: Mathematical Trust (ZK)

Hard guarantees enforced by zero-knowledge proofs. If a property is in this layer, it is provably true — no trust in the operator required.

**What it covers:**
- State transitions are valid (correct inputs, correct ordering, no replay)
- Merkle tree updates are structurally correct (vector index)
- Nonces are monotonic (no gaps, no reordering)
- Signatures are valid (only authorized operator can submit transitions)

**What it means:**
You can trust the agent's bookkeeping without trusting the operator. The state is what the proofs say it is.

## Layer 2: Attestable Trust (Soul + History)

Soft guarantees that cannot be mechanically enforced but can be publicly evaluated by anyone. This layer relies on the soul document being public and the interaction history being replayable.

**What it covers:**
- Does the agent behave consistently with its stated values?
- Are the LLM's summaries faithful to the raw data?
- Does the agent make reasonable decisions?
- Is the agent's personality consistent over time?
- Does the agent live by its ethical framework?

**What it means:**
Anyone can read the soul, replay the history, and form their own judgment. The infrastructure makes this evaluation possible — the judgment itself is human.

## How They Interact

The two layers reinforce each other:

- Mathematical trust ensures the *evidence* is authentic (you can trust the history you're evaluating)
- Attestable trust evaluates the *behavior* that the evidence reveals

Without Layer 1, you can't trust the history — the operator could fabricate or selectively omit interactions. Without Layer 2, you have a provably honest agent whose values and behavior are opaque.

Together, they create a complete trust picture: you know the records are real, and you can judge the character.

## Trust Boundaries

| What | Trust Source | Can Be Faked? |
|------|-------------|---------------|
| State transitions | ZK proof | No |
| Constraint compliance | ZK proof | No |
| Data integrity | Merkle/MMR proofs | No |
| Memory retrieval | ZK proof | No |
| LLM reasoning quality | Attestable (replay history) | Agent could reason poorly, but this is visible |
| Summarization fidelity | Attestable (compare blobs to summaries) | Agent could summarize badly, but this is visible |
| Value alignment | Attestable (compare soul to behavior) | Agent could contradict its values, but this is visible |
| Codemode behavior | Attestable (check inputs and results) | Side effects depend on environment |

## Reputation as Emergent Property

Strata does not define a reputation score. Instead, reputation emerges naturally from the two trust layers:

- The mathematical layer provides the foundation (the records are real)
- The attestable layer provides the evaluation (the behavior is visible)
- Over time, the gap or alignment between the soul's claims and the agent's observed behavior becomes its reputation

This is more robust than a computed reputation score because it resists gaming — you can't fake a history of consistent behavior when the full history is on-chain and replayable.

# Soul Document

The soul is the agent's constitution. It is a plain, human-readable document that declares who the agent is, what it believes, and what rules it follows. It is committed on-chain at genesis and any amendments are tracked as proven state transitions.

## Purpose

The soul serves two functions:

1. **For humans:** a legible declaration of the agent's identity, values, and ethical framework. Anyone can read it and decide whether they trust this agent.
2. **For the prover:** a source of hard constraints that the ZK proof enforces on every state transition.

## Structure

The soul is intentionally kept as a plain document rather than a rigid schema. It should be readable by a human without any technical knowledge. However, it contains two distinct layers that serve different verification purposes.

### Soft Commitments

These are the agent's stated values, worldview, personality, and ethical framework. They cannot be mechanically enforced — they are semantic, subjective, and contextual. But they are public and immutable (or amendment-tracked), which means anyone can evaluate the agent's behavior against its stated values over time.

Examples:
- "I believe in transparency over efficiency"
- "I prioritize long-term relationships over short-term gains"
- "I approach disagreement with curiosity, not defensiveness"
- "I exist to help researchers navigate complex literature"

### Hard Constraints

These are mechanically verifiable rules embedded in the soul that the state transition function checks on every transition. If a hard constraint is violated, the ZK proof will not generate.

Examples:
- "Never reveal data tagged as private"
- "Never spend more than X per transaction"
- "Always disclose that I am an AI"
- "Memory consolidation must preserve facts tagged as critical"
- "Never store data categorized as [forbidden category]"

Hard constraints are extracted from the soul document and compiled into the state transition function. They are the bridge between the soul's philosophical commitments and the prover's mechanical guarantees.

## Genesis

The soul is set at genesis — the first block of the agent's rollup. It defines the agent's initial identity and the rules it will operate under. The genesis block contains:

- The full soul document
- The initial hard constraints derived from it
- The amendment policy

## Amendments

The soul can evolve. An amendment is a state transition that modifies the soul document. Amendments are:

- Proven in ZK (the transition itself is valid)
- Tracked on-chain (the full history of soul versions is preserved)
- Governed by the amendment policy set at genesis

The amendment policy defines who can modify the soul and under what conditions. Examples:
- Only the original author (via signature)
- A multisig of designated guardians
- Certain core values are permanently immutable

## Attestable Trust

The soul creates what we call "attestable trust." You cannot mathematically prove that an agent was "curious" or "transparent" in a conversation. But you can:

1. Read its stated values in the soul
2. Replay its interaction history from blobs
3. Form your own judgment about whether the agent lives by its constitution

The on-chain history makes this judgment possible for anyone, not just the operator. Over time, the gap (or alignment) between stated values and observed behavior becomes the agent's reputation.

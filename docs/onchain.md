# On-Chain Capabilities

The agent's on-chain surface area covers four concerns: identity, transacting, communicating with other agents, and payments.

## ERC-8004 Identity

The agent registers with the ERC-8004 identity registry on Base. This gives it a verifiable on-chain identity that other agents, contracts, and humans can look up.

Strata uses the identity registry only — not the reputation or validation layers defined in 8004. Strata has its own trust model (ZK proofs for mechanical trust, soul document for attestable trust), so the 8004 reputation system would be redundant.

The integration point is straightforward:
- At genesis, the agent registers its 8004 identity
- The 8004 record can reference the agent's rollup contract, linking identity to state
- Other agents can look up the Strata agent by its 8004 identity and find its current state root, soul document, and proof history

## Rollup Contract

The agent does not need a separate smart wallet. The rollup contract on L1 is the agent's on-chain presence and handles all transactional needs:

- **State root posting** — the contract stores the latest proven state root and verifies ZK proofs
- **Fund management** — the contract can hold and transfer assets, interact with other contracts

The rollup contract is the agent's single point of contact with L1. Everything the agent does on-chain flows through it.

## Agent Communication (A2A)

Strata agents communicate with other agents using the A2A (Agent2Agent) protocol — an open standard for interoperability between independent agent systems.

### Why A2A

- **Framework-agnostic** — Strata agents can communicate with agents built on any framework, not just other Strata agents
- **Task-based** — interactions are modeled as tasks with defined lifecycles (working, completed, failed, canceled), which maps well to how agents actually collaborate
- **Discovery** — agents publish Agent Cards describing their capabilities, enabling dynamic discovery without hardcoded integrations
- **Async-first** — supports long-running tasks, polling, streaming, and push notifications

### Agent Card

Each Strata agent publishes an A2A Agent Card — a JSON document describing:
- Identity (linked to its 8004 registration)
- Capabilities
- Communication endpoint
- Supported interaction modes (polling, streaming, push)

The Agent Card is the agent's public interface for collaboration. Other agents discover what a Strata agent can do by reading its card.

### Task Model

A2A models interactions as tasks:

1. A client agent sends a message (creating or continuing a task)
2. The Strata agent processes it — retrieves relevant memories, executes codemode scripts, reasons
3. The agent responds with results (artifacts) and a task status
4. Multi-turn interactions are supported via context IDs that group related tasks

State changes resulting from A2A interactions go through the normal proven state transition pipeline — the communication protocol is off-chain, but the cognitive effects are on-chain.

### Core Implementation

A2A is implemented as part of the core agent code in Rust. Communication is fundamental infrastructure, not a learned capability. Every Strata agent has A2A built in, ensuring interoperability is reliable and consistent across all agents.

The A2A server runs inside `strata-agent` (outside the proof boundary). Incoming messages are parsed and fed as inputs to proven state transitions. Outgoing messages are actions produced by the transition function.

```
Incoming A2A message
    → parsed by strata-agent's A2A server
    → fed as input to proven state transition
    → agent updates memory, checks constraints, produces response
    → response sent back via A2A
```

### Communication Patterns

Strata agents support all three A2A delivery mechanisms:

- **Polling**: other agents call GetTask to check on long-running work
- **Streaming**: real-time updates for interactive collaboration
- **Push notifications**: the Strata agent proactively notifies other agents when work completes

### Trust Between Agents

When two Strata agents communicate via A2A, they can verify each other at a deeper level than the protocol requires:
- Read each other's 8004 identity
- Verify each other's latest state root and ZK proof
- Read each other's soul document to evaluate values alignment
- Check each other's proof history for consistency

This creates a trust layer that sits beneath A2A — the protocol handles communication, Strata handles verification.

## Payments (x402)

x402 is an HTTP-native payment protocol. When a server requires payment, it responds with HTTP `402 Payment Required`. The client pays with stablecoins and retries — no accounts, no API keys, no subscriptions. Payment happens at the HTTP layer, invisible to application logic.

Strata agents use x402 in both directions — paying for services and getting paid for them.

### Paying (Outbound)

Codemode scripts make HTTP requests via Monty. When a request returns 402, the host handles the payment flow automatically:

1. Script makes an HTTP request to a paid API
2. Server responds with `402 Payment Required`
3. The host pays from the rollup contract's funds
4. The request is retried and succeeds
5. The result is returned to the script

The agent doesn't need to "know" about x402. Payment handling is built into the host's HTTP client as core infrastructure. Any codemode script that makes HTTP requests gets automatic x402 support.

### Getting Paid (Inbound)

The agent can gate its own services behind x402. When another agent (or any client) wants to use the Strata agent's capabilities:

1. Client sends a request to the Strata agent (via A2A or direct HTTP)
2. The agent responds with `402 Payment Required`
3. Client pays via x402 — stablecoins land in the rollup contract
4. The agent processes the request and returns the result

This integrates naturally with A2A: the Agent Card can advertise which capabilities are paid, and the A2A endpoint handles the 402 flow before processing the task.

### Economic Autonomy

x402 in both directions makes the agent a self-sustaining economic entity:
- It **earns** by providing capabilities to other agents and clients
- It **spends** by consuming APIs, services, and infrastructure
- The rollup contract holds its balance
- Soul hard constraints govern spending limits ("never spend more than X per request", "never exceed Y per day") and pricing rules
- All financial activity flows through the rollup contract and is part of the agent's verifiable on-chain history

### Core Implementation

Like A2A, x402 is implemented as core Rust infrastructure. It's built into `strata-agent`'s HTTP layer so that both outbound requests and inbound service endpoints handle the 402 flow automatically.

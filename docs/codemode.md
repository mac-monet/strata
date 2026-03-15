# Codemode and Capabilities

Inspired by the nanoclaw design philosophy: customization is code changes, not configuration. The agent starts minimal and expands its capabilities through accumulated procedural knowledge — memories that describe how to accomplish tasks.

## How the Agent Acts

The agent has three primitives:

- **recall(query)** — search memory via semantic similarity
- **remember(text)** — store new knowledge
- **codemode** — write and execute Python via Monty

When the agent needs to act in the world (make HTTP requests, process data, interact with contracts), it writes Python on the fly. The LLM generates a script, Monty (a minimal Python interpreter written in Rust) executes it in a sandbox, and the results feed back into the conversation and state transitions.

Scripts are **ephemeral** — generated, executed, and discarded. The agent doesn't store tools. It stores *knowledge about how to do things*, and writes fresh code each time informed by that knowledge.

## Procedural Knowledge as Memory

The agent's capabilities grow through memory, not through tool registration. There is no distinction between a "skill" and any other memory — both are entries in the vector DB retrieved via semantic search.

Examples of memories that encode capabilities:

- "To get token prices from Uniswap, query the v3 subgraph at `api.thegraph.com/subgraphs/name/uniswap/uniswap-v3` with a GraphQL query like `{ token(id: \"0x...\") { derivedETH } }`"
- "The GitHub API requires a Bearer token in the Authorization header. PR data is at `api.github.com/repos/{owner}/{repo}/pulls/{number}`"
- "To check if a contract is verified on Basescan, use `curl https://api.basescan.org/api?module=contract&action=getsourcecode&address={addr}`"

When the agent encounters a task, it recalls relevant memories. Some of those memories happen to contain procedural knowledge — API endpoints, query patterns, data formats. The LLM uses this context to write Python that accomplishes the task.

The agent learns a new capability by remembering how to do it. That's it.

## Codemode Execution

### Flow

```
1. Agent receives input
2. recall() retrieves relevant memories (some procedural, some not)
3. LLM writes a Python script informed by those memories
4. Monty executes the script in a sandbox
5. Results feed back into the conversation
6. Any new knowledge is stored via remember()
7. State changes go through the proven transition pipeline
```

### Why Monty

Monty is a minimal Python interpreter written in Rust by Pydantic:

- **Sandboxed**: no filesystem, network, or environment access by default. The host controls what the script can access.
- **LLM-native**: LLMs write excellent Python — far better than any niche scripting language.
- **Rust-native**: embeds without FFI overhead, sub-microsecond startup.
- **Snapshotable**: interpreter state can be serialized to bytes — useful for reconstruction and pause/resume.

### Why Ephemeral Scripts

Storing scripts as separate artifacts (in blobs, with versioning and lifecycle management) adds complexity without clear benefit:

- The LLM regenerates essentially the same code from the same procedural memory. The memory is the durable part.
- Fresh generation adapts to current context — the script might handle edge cases differently based on the conversation.
- No blob storage, no tool registry, no versioning, no separate reconstruction path.
- Complex scripts the agent wants to reuse can be `remember()`'d as memories containing code snippets.

## Codemode Operates Outside the Proof

The ZK proof covers the agent's core state machine — memory management, constraint checking, merkle updates. Codemode operates in the host environment, outside the proof boundary.

Why:
- Scripts are side-effectful (API calls, network requests, data processing)
- Proving script execution would require recompiling the ZK guest on every change
- The core value proposition (persistent, verifiable state) doesn't depend on proving code execution

What gets proven:
- **Memory updates** — when codemode results cause the agent to `remember()` something, that memory update is a proven state transition like any other.

What doesn't get proven:
- **Script execution** — the Python runs in Monty in the host. This is trusted to the operator, same as LLM inference.

## Auditability

Codemode scripts are ephemeral — like LLM inference traces, the internal process is not stored on-chain. What IS auditable:

- **Inputs**: the memories recalled and the interaction that triggered the script
- **Results**: memory updates (via `remember()`) are proven state transitions committed on-chain
- **Responses**: the agent's replies are part of the interaction record

This is the same trust model as LLM reasoning — you can verify what went in and what came out, but the execution itself is trusted to the operator.

## Design Philosophy

From nanoclaw:
- **Customization = code changes.** No configuration sprawl. Want different behavior? Modify the code. The codebase is small enough that it's safe to make changes.
- **Capabilities over features.** Instead of adding features to the codebase, the agent accumulates procedural knowledge. Each agent's capability set is shaped by what it actually encounters.

A Strata agent begins with a small, fixed core. It acquires capabilities by remembering how to do things and writing code to do them. No tool registration, no capability manifests — just memory and code.

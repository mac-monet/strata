# Skills and Tools

Inspired by the nanoclaw design philosophy: customization is code changes, not configuration. The agent starts minimal and expands through discrete, composable skills rather than accumulated features.

## Skills vs Tools

These are two distinct concepts:

- **Skill** = knowledge. A memory entry in the vector DB that describes how and when to do something. "To fetch a GitHub PR, call the github-pr tool with the repo and PR number." Skills are retrieved via semantic search like any other memory.
- **Tool** = executable code. A Rhai script stored in blobs that actually performs the action. Tools are invoked by the agent when a relevant skill is retrieved.

The skill is the *knowledge of how and when to use a tool*. The tool is the *implementation*.

## Design Philosophy

From nanoclaw:
- **Customization = code changes.** No configuration sprawl. Want different behavior? Modify the code. The codebase is small enough that it's safe to make changes.
- **Skills over features.** Instead of adding features to the codebase, contributors submit skills that transform your fork. You end up with clean code that does exactly what you need.

A Strata agent begins with a small, fixed core. It acquires skills (knowledge) and builds tools (code) as needed. Each agent's capability set is shaped by what it actually encounters.

## Flow

```
1. Agent receives input
2. Vector DB retrieval finds a relevant skill (memory)
   → "I know how to do this — use the github-pr tool with these parameters"
3. Agent invokes the referenced tool (Rhai script)
4. Tool executes in the host environment (API calls, parsing, etc.)
5. Results feed back as input to the next proven state transition
```

## Tools Operate Outside the Proof

The ZK proof covers the agent's core state machine — memory management, constraint checking, merkle updates. Tools operate in the host environment, outside the proof boundary.

Why:
- Tools are side-effectful (API calls, network requests, file operations)
- Tools may change frequently as the agent adapts
- Proving tool execution would require recompiling the ZK guest on every change
- The core value proposition (persistent, verifiable state) doesn't depend on proving tool execution

What gets proven:
- **Tool creation/modification** — the fact that the agent decided to add or change a tool is a state transition. The tool definition (Rhai AST) is committed in blob storage.
- **Skill creation** — the knowledge entry added to the vector DB is a proven state transition like any other memory update.
- **Result integration** — when a tool produces output that updates agent state, that state update goes through the normal proven transition.

What doesn't get proven:
- **Tool execution** — the Rhai script runs in the host, interacts with the world, and returns results. This is trusted to the operator, same as LLM inference.

## Tools Are Runtime-Agnostic

Since tools execute outside the proof boundary and are stored as scripts in blobs, the runtime is an implementation detail. The reference implementation uses Rhai, but an operator could interpret the same logic in any language. The on-chain commitment is the script content — what executes it is irrelevant to the proof.

The chain cares about *what tools the agent has* (committed in blobs) and *what results they produced* (fed into proven state transitions). It does not care about the execution environment.

## Why Rhai (Reference Implementation)

Rhai is the reference tool runtime because it embeds cleanly in Rust:

- **Sandboxed**: scripts run in a controlled environment with explicit permissions
- **AST-compiled**: structured, deterministic representations that are compact and reconstructable
- **Rust-native**: embeds without FFI overhead
- **Simple**: small language surface area, easy for an LLM to write correctly

## Host Bindings

The tool runtime has access to a controlled set of host functions:

- HTTP requests (API interactions)
- Cryptographic operations (signing, hashing)
- State queries (reading the agent's own memories)
- Blob reads (accessing historical data)

These bindings are defined by the operator. Different operators reconstructing the same agent may expose different bindings — this affects what tools can do but not what the agent knows or believes.

## Lifecycle

### Creating a New Capability

1. The agent identifies a need
2. The LLM writes a Rhai script (the tool)
3. The script is compiled to an AST and stored in blobs (committed via MMR)
4. A skill (memory entry) is added to the vector DB describing the tool — what it does, when to use it, what parameters it takes
5. Both the blob commit and the memory update are proven state transitions

### Using a Capability

1. During an interaction, the agent queries the vector DB
2. A relevant skill is retrieved: "I have a tool for this"
3. The agent invokes the referenced tool
4. The tool executes in the host, returns results
5. Results feed into the next proven state transition

### Modifying a Capability

1. The agent decides a tool needs updating
2. The LLM produces a modified Rhai script
3. The new AST is stored in blobs (old version preserved in blob history)
4. The skill entry in the vector DB is updated
5. Both updates are proven state transitions

## Reconstruction

When reconstructing an agent, both skills and tools are restored:

1. Download blobs (verified against MMR commitment) — tools (Rhai ASTs) are extracted
2. Load tools into the runtime
3. Skills are already in the vector DB (restored as part of memory reconstruction)
4. The agent has its full capability set — both the knowledge of what it can do and the code to do it

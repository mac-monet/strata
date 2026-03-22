# strata-agent

A2A-compatible AI agent with verifiable state transitions, ZK batch proving, and on-chain posting.

## Prerequisites

- Rust (edition 2024)
- An LLM API key (Anthropic or OpenAI-compatible)
- (Optional) A Base RPC endpoint and operator key for on-chain posting
- (Optional) Pre-built OpenVM prover for ZK proofs

## Build

```bash
cargo build --release -p strata-agent
```

If using ZK proving, also build the prover host binary:

```bash
cd strata-openvm
cargo build --release
```

This produces `strata-openvm/target/release/strata-openvm-host`. The agent invokes this binary directly (not `cargo run`) to avoid recompilation overhead.

## Configuration

All configuration is via environment variables. Create a `.env` file in the project root or export them directly.

### Required

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` | LLM provider API key |

### Optional — Server

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | HTTP server port |
| `SOUL_FILE` | built-in `soul.md` | Path to a custom soul/system prompt file |

### Optional — On-chain posting

| Variable | Description |
|----------|-------------|
| `RPC_URL` | Base RPC endpoint (enables on-chain posting) |
| `OPERATOR_KEY` | Private key hex for signing transactions |
| `CONTRACT_ADDRESS` | Existing contract address (if omitted, deploys a new one) |
| `POST_INTERVAL` | Seconds between batch posts (default: `3600`) |
| `WAL_PATH` | Write-ahead log path (default: `./strata-batch.wal`) |

### Optional — ZK Proving

| Variable | Description |
|----------|-------------|
| `PROVER_DIR` | Path to `strata-openvm/` directory (enables proving) |
| `PROOF_LEVEL` | `app` (default, fast) or `evm` (on-chain verifiable) |

### Optional — ERC-8004 Identity

| Variable | Description |
|----------|-------------|
| `AGENT_ID` | Numeric agent ID in the registry |
| `REGISTRY_ADDRESS` | ERC-8004 registry contract address |
| `AGENT_BASE_URL` | Public URL of this agent |

### Optional — State Reconstruction

| Variable | Description |
|----------|-------------|
| `RECONSTRUCT` | Set to `1` or `true` to reconstruct state from on-chain data |

Reconstruction requires `CONTRACT_ADDRESS` and `RPC_URL`, and the VectorDB must be empty.

## Run

Minimal (chat only, no on-chain posting):

```bash
ANTHROPIC_API_KEY=sk-... cargo run --release -p strata-agent
```

With on-chain posting and ZK proving:

```bash
ANTHROPIC_API_KEY=sk-... \
RPC_URL=https://mainnet.base.org \
OPERATOR_KEY=0xdeadbeef... \
CONTRACT_ADDRESS=0x... \
PROVER_DIR=./strata-openvm \
PROOF_LEVEL=app \
POST_INTERVAL=300 \
cargo run --release -p strata-agent
```

## Endpoints

| Route | Description |
|-------|-------------|
| `GET /` | Chat web UI |
| `GET /health` | Health check |
| `GET /.well-known/agent.json` | A2A agent card |
| `POST /a2a` | A2A JSON-RPC endpoint |
| `GET /proofs/:nonce` | Retrieve proof for a given nonce |

## Architecture

```
chat / A2A request
    → LLM pipeline (tool execution, embeddings)
    → state transition + witness
    → PendingBatch buffer
    → batch loop (on interval):
        1. Write transitions to WAL
        2. Prove batch (ZK, via strata-openvm-host)
        3. Post on-chain (contract call)
        4. Truncate WAL on success
```

Failed batches are retried on the next tick. The WAL ensures transitions survive crashes. A semaphore limits proving to one process at a time to prevent OOM.

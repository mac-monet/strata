#!/usr/bin/env bash
# End-to-end batch prover loop test:
#   Anvil → Agent (Venice LLM + batch prover) → send messages → wait for batch → verify on-chain
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# --- Configuration ---
ANVIL_PORT=8545
AGENT_PORT=3000
RPC_URL="http://localhost:$ANVIL_PORT"
# Anvil's first account private key (deterministic)
OPERATOR_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# Short batch interval for testing (10 seconds)
POST_INTERVAL=10

cleanup() {
    echo ""
    echo "=== Cleaning up ==="
    [ -n "${ANVIL_PID:-}" ] && kill "$ANVIL_PID" 2>/dev/null && echo "killed anvil ($ANVIL_PID)"
    [ -n "${AGENT_PID:-}" ] && kill "$AGENT_PID" 2>/dev/null && echo "killed agent ($AGENT_PID)"
    rm -rf "$ROOT"/strata-journal* "$ROOT"/strata-meta* "$ROOT"/strata-batch.wal 2>/dev/null
    exit
}
trap cleanup EXIT INT TERM

# --- Step 1: Start Anvil ---
echo "=== Starting Anvil ==="
anvil --port "$ANVIL_PORT" --silent &
ANVIL_PID=$!
sleep 2
echo "anvil running (pid $ANVIL_PID)"

# --- Step 2: Start the agent ---
echo ""
echo "=== Starting Agent (batch interval=${POST_INTERVAL}s) ==="
RPC_URL="$RPC_URL" \
OPERATOR_KEY="$OPERATOR_KEY" \
PROVER_DIR="$ROOT/strata-openvm" \
PROOF_LEVEL=app \
POST_INTERVAL="$POST_INTERVAL" \
PORT="$AGENT_PORT" \
  cargo run -p strata-agent --release 2>&1 &
AGENT_PID=$!

# Wait for agent to be ready
echo "waiting for agent to start..."
for i in $(seq 1 30); do
    if curl -s "http://localhost:$AGENT_PORT/health" >/dev/null 2>&1; then
        echo "agent ready (pid $AGENT_PID)"
        break
    fi
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
        echo "ERROR: agent process died"
        exit 1
    fi
    sleep 2
done

# --- Step 3: Send multiple messages to build up a batch ---
echo ""
echo "=== Sending messages (will be batched) ==="

T1=$(date +%s)
RESP1=$(curl -s "http://localhost:$AGENT_PORT/a2a" \
    -H 'Content-Type: application/json' \
    -d '{
      "jsonrpc": "2.0", "id": 1, "method": "message/send",
      "params": { "message": { "messageId": "test-1", "role": "user",
        "parts": [{"text": "Remember that the sky is blue and water is wet."}] } }
    }')
T2=$(date +%s)
echo "Message 1 ($((T2-T1))s): $(echo "$RESP1" | python3 -c 'import sys,json; r=json.load(sys.stdin); print(r.get("result",{}).get("status",{}).get("state","ERROR"))')"

T1=$(date +%s)
RESP2=$(curl -s "http://localhost:$AGENT_PORT/a2a" \
    -H 'Content-Type: application/json' \
    -d '{
      "jsonrpc": "2.0", "id": 2, "method": "message/send",
      "params": { "message": { "messageId": "test-2", "role": "user",
        "parts": [{"text": "Remember that cats purr and dogs bark."}] } }
    }')
T2=$(date +%s)
echo "Message 2 ($((T2-T1))s): $(echo "$RESP2" | python3 -c 'import sys,json; r=json.load(sys.stdin); print(r.get("result",{}).get("status",{}).get("state","ERROR"))')"

# Check no on-chain posts yet (only the deploy tx)
TX_COUNT=$(cast rpc eth_getTransactionCount "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" "latest" --rpc-url "$RPC_URL" 2>/dev/null)
echo ""
echo "tx count before batch: $TX_COUNT (expect 0x1 = deploy only)"

# --- Step 4: Wait for batch to fire ---
echo ""
echo "=== Waiting for batch post (polling)... ==="
for i in $(seq 1 30); do
    TX_COUNT2=$(cast rpc eth_getTransactionCount "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" "latest" --rpc-url "$RPC_URL" 2>/dev/null)
    if [ "$TX_COUNT2" != "\"0x1\"" ]; then
        echo "tx count after batch: $TX_COUNT2 (batch posted after ${i}0s)"
        break
    fi
    sleep 10
done
if [ "$TX_COUNT2" = "\"0x1\"" ]; then
    echo "WARN: batch still not posted after 300s"
fi

# --- Step 5: Verify recall works ---
echo ""
echo "=== Sending recall message ==="
T1=$(date +%s)
RESP3=$(curl -s "http://localhost:$AGENT_PORT/a2a" \
    -H 'Content-Type: application/json' \
    -d '{
      "jsonrpc": "2.0", "id": 3, "method": "message/send",
      "params": { "message": { "messageId": "test-3", "role": "user",
        "parts": [{"text": "What color is the sky?"}] } }
    }')
T2=$(date +%s)
REPLY=$(echo "$RESP3" | python3 -c 'import sys,json; r=json.load(sys.stdin); print(r.get("result",{}).get("status",{}).get("message",{}).get("parts",[{}])[0].get("text","?"))' 2>/dev/null)
echo "Message 3 ($((T2-T1))s): $REPLY"

echo ""
echo "=== E2E batch prover test complete ==="

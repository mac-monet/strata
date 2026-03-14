# Storage Layer

The storage layer handles the agent's raw data — the full content behind each memory entry. For each `MemoryEntry` in the vector DB, the full text content is posted on-chain alongside the state root update, and is verifiable via the entry's `content_hash`.

## MVP: Calldata on Base

For the MVP on Base, raw content is posted as calldata in the same transaction that posts the new state root. This is simple and sufficient at agent-memory scale.

## Future: Blobs / DA

The storage transport is pluggable. The vector DB only stores `content_hash` — it doesn't care where the content lives. Future options:

- **EIP-4844 blobs** — cheaper than calldata for larger payloads
- **External DA layer** — Celestia, EigenDA, etc.

Changing the transport requires updating `strata-agent`'s posting and reconstruction logic (where to download from). The vector DB, guest program, and on-chain commitment are unchanged.

## Verification

Anyone can verify the stored content:
1. Download calldata from the relevant transactions
2. Hash each piece of content
3. Compare against the `content_hash` in the corresponding `MemoryEntry`
4. The `MemoryEntry` is committed via `vector_index_root`

## Reconstruction

When local storage is lost, the agent is reconstructed by:
1. Downloading all calldata from on-chain transactions
2. Verifying each piece against its `content_hash`
3. Replaying all appends to rebuild the vector DB
4. Verifying the final root matches the on-chain `vector_index_root`

## What Gets Stored

Each memory entry's full content is posted. This includes:
- Identity context, relationship knowledge, ongoing context (core memories)
- Learned facts, interaction summaries, decision records (non-core memories)

The content is what the LLM extracted and decided to remember. Raw conversation transcripts and reasoning traces are not stored in MVP — only the memories derived from them.

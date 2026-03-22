//! Reconstruction: recover full agent state from on-chain calldata.
//!
//! Given only a contract address, replays all `submitTransition` calldata to
//! rebuild the `MemoryEntry` list and content strings. The caller is responsible
//! for populating a `VectorDB` via `batch_append` and verifying the root.

use alloy::{
    consensus::Transaction,
    providers::{Provider, ProviderBuilder},
    rpc::types::Filter,
    sol_types::{SolCall, SolEvent},
};
use commonware_codec::Decode;
use strata_core::{
    CoreState, MemoryEntry, Nonce, SoulHash, TransitionRecord, TransitionRecordCfg, VectorRoot,
};

use crate::error::AgentError;
use crate::poster::{self, PosterConfig, StrataRollup};

/// Fully reconstructed agent state from on-chain data.
#[derive(Debug)]
pub struct ReconstructedState {
    pub state: CoreState,
    pub entries: Vec<MemoryEntry>,
    pub contents: Vec<String>,
}

/// Reconstruct the full agent state by replaying on-chain calldata.
///
/// Fetches all `StateTransition` events from the contract, decodes each
/// transaction's `memoryContent` calldata back into `TransitionRecord`s,
/// and flattens them into entries + content strings.
///
/// The caller should create a fresh `VectorDB`, call `batch_append` with
/// the returned entries, and verify `db.root()` matches
/// `state.vector_index_root`.
pub async fn reconstruct(config: &PosterConfig) -> Result<ReconstructedState, AgentError> {
    let err = |msg: String| AgentError::Reconstruct(msg);

    // 1. Read on-chain state
    let soul_hash_bytes = poster::read_soul_hash(config).await?;
    let state_root_bytes = poster::read_state_root(config).await?;
    let on_chain_nonce = poster::read_nonce(config).await?;

    // 2. Fetch StateTransition event logs
    let provider = ProviderBuilder::new().connect_http(
        config
            .rpc_url
            .parse()
            .map_err(|e| err(format!("invalid RPC URL: {e}")))?,
    );

    let event_sig = StrataRollup::StateTransition::SIGNATURE_HASH;
    let filter = Filter::new()
        .address(config.contract_address)
        .event_signature(event_sig)
        .from_block(0);

    let logs = provider
        .get_logs(&filter)
        .await
        .map_err(|e| err(format!("get_logs failed: {e}")))?;

    // With batching, each tx may contain multiple transitions, so the
    // number of logs (one per submitTransition call) can be less than the
    // on-chain nonce (which counts individual transitions).
    if logs.is_empty() && on_chain_nonce > 0 {
        return Err(err("expected logs but found none".into()));
    }

    // 3. Decode calldata from each transaction
    let cfg = TransitionRecordCfg::default();
    let mut records = Vec::with_capacity(logs.len());

    for log in &logs {
        let tx_hash = log
            .transaction_hash
            .ok_or_else(|| err("log missing transaction_hash".into()))?;

        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .map_err(|e| err(format!("get_transaction failed: {e}")))?
            .ok_or_else(|| err(format!("transaction {tx_hash} not found")))?;

        let input = tx.input();

        if input.len() < 4 {
            return Err(err("transaction input too short".into()));
        }

        let decoded_call =
            StrataRollup::submitTransitionCall::abi_decode(input)
                .map_err(|e| err(format!("ABI decode failed: {e}")))?;

        // Third field is memoryContent (the unnamed `_2` parameter).
        // May contain a single record (legacy) or length-prefixed batch.
        let memory_bytes = decoded_call._2;
        let batch = decode_memory_content(&memory_bytes, &cfg)
            .map_err(|e| err(format!("record decode failed for tx {tx_hash}: {e}")))?;
        records.extend(batch);
    }

    // Verify total transitions match on-chain nonce
    if records.len() as u64 != on_chain_nonce {
        return Err(err(format!(
            "decoded {} transitions but on-chain nonce is {on_chain_nonce}",
            records.len()
        )));
    }

    // 4. Flatten entries and contents
    let mut entries = Vec::new();
    let mut contents = Vec::new();

    for record in &records {
        entries.extend(record.new_entries.iter().cloned());
        for mc in &record.contents {
            let text = String::from_utf8(mc.bytes.clone())
                .map_err(|e| err(format!("content not valid UTF-8: {e}")))?;
            contents.push(text);
        }
    }

    // 5. Build CoreState
    let state = CoreState {
        soul_hash: SoulHash::new(*soul_hash_bytes),
        vector_index_root: VectorRoot::new(*state_root_bytes),
        nonce: Nonce::new(on_chain_nonce),
    };

    Ok(ReconstructedState {
        state,
        entries,
        contents,
    })
}

/// Decode memory content from calldata, handling both legacy (single record)
/// and batch (length-prefixed records) formats.
///
/// Batch format: `[u32 BE length][record bytes][u32 BE length][record bytes]...`
/// Legacy format: raw `TransitionRecord` bytes (no length prefix).
fn decode_memory_content(
    bytes: &[u8],
    cfg: &TransitionRecordCfg,
) -> Result<Vec<TransitionRecord>, String> {
    if bytes.is_empty() {
        return Err("empty memory content".into());
    }

    // Try batch format first: read length prefix and see if it makes sense.
    if bytes.len() >= 4 {
        let first_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        // Heuristic: if the first 4 bytes parse as a reasonable length that fits
        // within the remaining data, treat as batch format.
        if first_len > 0 && first_len <= bytes.len() - 4 {
            if let Ok(records) = decode_batch(bytes, cfg) {
                return Ok(records);
            }
        }
    }

    // Fall back to legacy single-record format.
    let record = TransitionRecord::decode_cfg(bytes, cfg)
        .map_err(|e| format!("legacy decode: {e}"))?;
    Ok(vec![record])
}

/// Decode length-prefixed batch of transition records.
fn decode_batch(
    mut bytes: &[u8],
    cfg: &TransitionRecordCfg,
) -> Result<Vec<TransitionRecord>, String> {
    let mut records = Vec::new();
    while bytes.len() >= 4 {
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        bytes = &bytes[4..];
        if bytes.len() < len {
            return Err(format!("truncated record: need {len} bytes, have {}", bytes.len()));
        }
        let record = TransitionRecord::decode_cfg(&bytes[..len], cfg)
            .map_err(|e| format!("batch record decode: {e}"))?;
        records.push(record);
        bytes = &bytes[len..];
    }
    if !bytes.is_empty() {
        return Err(format!("trailing {} bytes after batch", bytes.len()));
    }
    Ok(records)
}

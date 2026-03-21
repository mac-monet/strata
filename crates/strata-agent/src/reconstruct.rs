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

    if logs.len() as u64 != on_chain_nonce {
        return Err(err(format!(
            "log count {} != on-chain nonce {on_chain_nonce}",
            logs.len()
        )));
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

        // Third field is memoryContent (the unnamed `_2` parameter)
        let memory_bytes = decoded_call._2;

        let record = TransitionRecord::decode_cfg(memory_bytes, &cfg)
            .map_err(|e| err(format!("record decode failed: {e}")))?;

        records.push(record);
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

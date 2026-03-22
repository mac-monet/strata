//! Background batch prover and poster.
//!
//! Accumulates transitions from the request handler and periodically proves +
//! posts them on-chain as a single batch. Transitions are persisted to a
//! write-ahead log (WAL) so they survive crashes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use commonware_codec::{Decode, Encode};
use tokio::sync::watch;

use crate::error::AgentError;
use crate::pipeline::{self, TransitionOutput};
use crate::poster;
use crate::prover::{self, ProverConfig};
use crate::server::PostingConfig;

/// Configuration for the batch background task.
#[derive(Clone, Debug)]
pub struct BatchConfig {
    /// How often to flush accumulated transitions.
    pub interval: Duration,
    /// Path to the write-ahead log file.
    pub wal_path: PathBuf,
}

/// Shared buffer of unpublished transitions.
pub type PendingBatch = tokio::sync::Mutex<Vec<TransitionOutput>>;

/// Run the batch loop until the shutdown signal fires.
///
/// Drains `pending` on each tick (or on shutdown), proves the batch, and posts
/// the result on-chain.
pub async fn run(
    pending: Arc<PendingBatch>,
    posting: PostingConfig,
    prover: Option<ProverConfig>,
    config: BatchConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    // Reload any transitions from the WAL that weren't posted before a crash.
    if let Err(e) = reload_wal(&config.wal_path, &pending).await {
        eprintln!("batch: failed to reload WAL: {e}");
    }

    let mut interval = tokio::time::interval(config.interval);
    interval.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => {
                // Shutdown requested — do one final flush.
                eprintln!("batch: shutdown signal received, flushing...");
                flush(&pending, &posting, &prover, &config.wal_path).await;
                return;
            }
        }
        flush(&pending, &posting, &prover, &config.wal_path).await;
    }
}

/// Drain pending transitions, prove, and post.
async fn flush(
    pending: &PendingBatch,
    posting: &PostingConfig,
    prover: &Option<ProverConfig>,
    wal_path: &PathBuf,
) {
    let batch: Vec<TransitionOutput> = {
        let mut lock = pending.lock().await;
        if lock.is_empty() {
            return;
        }
        std::mem::take(&mut *lock)
    };

    let count = batch.len();
    let first = &batch[0];
    let last = &batch[count - 1];
    let start_nonce = first.record.input.nonce.get();
    let end_nonce = last.record.input.nonce.get();

    eprintln!(
        "batch: flushing {count} transitions (nonce {start_nonce}..{end_nonce})"
    );

    // Generate proof (if prover configured).
    if let Some(prover_config) = prover {
        match prover::prove_batch(prover_config, &batch).await {
            Ok(bytes) => {
                eprintln!("batch: proof generated ({} bytes)", bytes.len());
            }
            Err(e) => {
                eprintln!("batch: proof generation failed: {e}");
                // Put transitions back so they're retried next tick.
                pending.lock().await.splice(0..0, batch);
                return;
            }
        }
    }

    // Build public values.
    let public_values = pipeline::batch_public_values(
        first.old_state.vector_index_root.as_bytes(),
        last.new_state.vector_index_root.as_bytes(),
        start_nonce,
        end_nonce,
        first.old_state.soul_hash.as_bytes(),
    );

    // Post on-chain (proof bytes kept off-chain).
    match post_with_retry(posting, public_values, &batch).await {
        Ok(hash) => {
            eprintln!(
                "batch: posted nonce {start_nonce}..{end_nonce}, tx={hash}"
            );
            // Success — truncate WAL.
            let _ = tokio::fs::write(wal_path, b"").await;
        }
        Err(e) => {
            eprintln!("batch: posting failed: {e}");
            // Persist to WAL and put back for retry.
            if let Err(we) = append_wal(wal_path, &batch).await {
                eprintln!("batch: WAL write failed: {we}");
            }
            pending.lock().await.splice(0..0, batch);
        }
    }
}

const POST_MAX_RETRIES: u32 = 5;
const POST_RETRY_BASE_MS: u64 = 500;

async fn post_with_retry(
    posting: &PostingConfig,
    public_values: [u8; 112],
    transitions: &[TransitionOutput],
) -> Result<alloy::primitives::TxHash, AgentError> {
    let mut last_err = None;
    for attempt in 0..POST_MAX_RETRIES {
        match poster::post_batch(
            &posting.poster,
            posting.signer.clone(),
            vec![], // proof bytes off-chain
            public_values,
            transitions,
        )
        .await
        {
            Ok(hash) => return Ok(hash),
            Err(e) => {
                eprintln!(
                    "batch: post attempt {}/{POST_MAX_RETRIES} failed: {e}",
                    attempt + 1
                );
                last_err = Some(e);
                if attempt + 1 < POST_MAX_RETRIES {
                    let delay = POST_RETRY_BASE_MS * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AgentError::Poster("posting failed".into())))
}

// --- Write-ahead log ---

/// Append transitions to the WAL as newline-delimited JSON.
async fn append_wal(path: &PathBuf, transitions: &[TransitionOutput]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("open WAL: {e}"))?;

    for t in transitions {
        let line = serde_json::json!({
            "old_state": t.old_state,
            "new_state": t.new_state,
            "nonce": t.record.input.nonce,
            "witness": t.witness,
            "record_bytes": hex::encode(t.record.encode()),
        });
        let mut bytes = serde_json::to_vec(&line).map_err(|e| format!("serialize: {e}"))?;
        bytes.push(b'\n');
        file.write_all(&bytes)
            .await
            .map_err(|e| format!("write WAL: {e}"))?;
    }
    file.flush().await.map_err(|e| format!("flush WAL: {e}"))?;
    Ok(())
}

/// Reload transitions from the WAL into the pending buffer.
async fn reload_wal(path: &PathBuf, pending: &PendingBatch) -> Result<(), String> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return Ok(()), // no WAL or empty
    };

    let mut count = 0u32;
    let mut lock = pending.lock().await;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("WAL parse: {e}"))?;

        let old_state: strata_core::CoreState =
            serde_json::from_value(value["old_state"].clone())
                .map_err(|e| format!("WAL old_state: {e}"))?;
        let new_state: strata_core::CoreState =
            serde_json::from_value(value["new_state"].clone())
                .map_err(|e| format!("WAL new_state: {e}"))?;
        let witness: strata_proof::Witness =
            serde_json::from_value(value["witness"].clone())
                .map_err(|e| format!("WAL witness: {e}"))?;
        let record_hex = value["record_bytes"]
            .as_str()
            .ok_or("WAL missing record_bytes")?;
        let record_bytes = hex::decode(record_hex).map_err(|e| format!("WAL hex: {e}"))?;
        let record = strata_core::TransitionRecord::decode_cfg(
            &*record_bytes,
            &strata_core::TransitionRecordCfg::default(),
        )
        .map_err(|e| format!("WAL record decode: {e}"))?;

        lock.push(TransitionOutput {
            record,
            witness,
            old_state,
            new_state,
        });
        count += 1;
    }

    if count > 0 {
        eprintln!("batch: reloaded {count} transitions from WAL");
    }
    Ok(())
}

//! OpenVM prover integration via subprocess.
//!
//! Invokes the `strata-openvm-host` binary to serialize inputs in the correct
//! OpenVM format, compile the guest, and generate a ZK proof.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::AgentError;
use crate::pipeline::TransitionOutput;

/// Default timeout for the prover subprocess (10 minutes).
const PROVE_TIMEOUT: Duration = Duration::from_secs(600);

/// Monotonic counter for unique temp file names across concurrent tasks.
static PROVE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Proof aggregation level.
#[derive(Clone, Copy, Debug, Default)]
pub enum ProofLevel {
    /// Fast application-level proof (not on-chain verifiable).
    #[default]
    App,
    /// Halo2-wrapped proof, verifiable on-chain via EVM.
    Evm,
}

impl ProofLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Evm => "evm",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "evm" => Self::Evm,
            _ => Self::App,
        }
    }
}

/// Configuration for the OpenVM prover.
#[derive(Clone, Debug)]
pub struct ProverConfig {
    /// Path to the `strata-openvm/` directory containing host + guest crates.
    pub openvm_dir: PathBuf,
    /// Proof aggregation level.
    pub proof_level: ProofLevel,
    /// Timeout for the prover subprocess.
    pub timeout: Duration,
}

impl ProverConfig {
    pub fn new(openvm_dir: PathBuf, proof_level: ProofLevel) -> Self {
        Self {
            openvm_dir,
            proof_level,
            timeout: PROVE_TIMEOUT,
        }
    }
}

/// App execution commits read from the host's output file.
#[derive(Clone, Debug)]
pub struct AppCommit {
    /// Commitment to the guest executable (32 bytes).
    pub app_exe_commit: [u8; 32],
    /// Commitment to the VM configuration (32 bytes).
    pub app_vm_commit: [u8; 32],
}

/// Read the app execution commits from the host's commit JSON file.
///
/// The host writes `strata-openvm-guest.<level>.commit.json` containing:
/// ```json
/// { "app_exe_commit": "<hex>", "app_vm_commit": "<hex>" }
/// ```
pub async fn read_app_commit(config: &ProverConfig) -> Result<AppCommit, AgentError> {
    let path = config
        .openvm_dir
        .join(format!("strata-openvm-guest.{}.commit.json", config.proof_level.as_str()));

    let json_str = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| AgentError::Prover(format!("failed to read commit file {}: {e}", path.display())))?;

    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| AgentError::Prover(format!("invalid commit JSON: {e}")))?;

    let exe_hex = value["app_exe_commit"]
        .as_str()
        .ok_or_else(|| AgentError::Prover("missing app_exe_commit in commit file".into()))?;
    let vm_hex = value["app_vm_commit"]
        .as_str()
        .ok_or_else(|| AgentError::Prover("missing app_vm_commit in commit file".into()))?;

    let exe_bytes = hex::decode(exe_hex)
        .map_err(|e| AgentError::Prover(format!("bad app_exe_commit hex: {e}")))?;
    let vm_bytes = hex::decode(vm_hex)
        .map_err(|e| AgentError::Prover(format!("bad app_vm_commit hex: {e}")))?;

    if exe_bytes.len() != 32 || vm_bytes.len() != 32 {
        return Err(AgentError::Prover("commit bytes must be 32 bytes each".into()));
    }

    let mut exe = [0u8; 32];
    let mut vm = [0u8; 32];
    exe.copy_from_slice(&exe_bytes);
    vm.copy_from_slice(&vm_bytes);

    Ok(AppCommit {
        app_exe_commit: exe,
        app_vm_commit: vm,
    })
}

/// Generate a ZK proof for a batch of state transitions.
///
/// Returns the raw proof bytes on success. The host binary accepts the batch
/// JSON format with a `transitions` array.
pub async fn prove_batch(
    config: &ProverConfig,
    transitions: &[TransitionOutput],
) -> Result<Vec<u8>, AgentError> {
    if transitions.is_empty() {
        return Err(AgentError::Prover("empty batch".into()));
    }

    // Build batch input JSON.
    let batch_transitions: Vec<_> = transitions
        .iter()
        .map(|t| {
            serde_json::json!({
                "nonce": t.record.input.nonce,
                "witness": t.witness,
            })
        })
        .collect();

    let input_json = serde_json::json!({
        "state": transitions[0].old_state,
        "transitions": batch_transitions,
    });

    let json_str =
        serde_json::to_string(&input_json).map_err(|e| AgentError::Prover(e.to_string()))?;

    // Write JSON to a unique temp file to avoid races under concurrent use.
    let id = PROVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let input_file = config
        .openvm_dir
        .join(format!(".prove-input-{id}.json"));
    tokio::fs::write(&input_file, &json_str)
        .await
        .map_err(|e| AgentError::Prover(format!("failed to write input file: {e}")))?;

    // Invoke the host binary with a timeout.
    let result = tokio::time::timeout(
        config.timeout,
        tokio::process::Command::new("cargo")
            .args([
                "run",
                "--release",
                "--",
                "prove",
                "--input",
                input_file.to_str().unwrap_or(".prove-input.json"),
                "--level",
                config.proof_level.as_str(),
            ])
            .current_dir(&config.openvm_dir)
            .output(),
    )
    .await;

    // Clean up temp file (best-effort).
    let _ = tokio::fs::remove_file(&input_file).await;

    let output = result
        .map_err(|_| AgentError::Prover(format!("prover timed out after {:?}", config.timeout)))?
        .map_err(|e| AgentError::Prover(format!("failed to spawn prover: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::Prover(format!("prover failed: {stderr}")));
    }

    // Read proof output. The host binary writes the proof to
    // `<openvm_dir>/strata-openvm-guest.<level>.proof`.
    let proof_path = config
        .openvm_dir
        .join(format!("strata-openvm-guest.{}.proof", config.proof_level.as_str()));

    let proof_bytes = tokio::fs::read(&proof_path)
        .await
        .map_err(|e| AgentError::Prover(format!("failed to read proof file: {e}")))?;

    Ok(proof_bytes)
}

//! Snapshot persistence: save and restore agent state + memory index across restarts.
//!
//! Writes a JSON file containing `CoreState`, `Vec<MemoryEntry>`, and content texts.
//! On startup, if the snapshot exists, the agent loads it and opens the VectorDB
//! with the recovered entries (the MMR journal recovers its own merkle state).

use std::path::Path;

use serde::{Deserialize, Serialize};
use strata_core::{CoreState, MemoryEntry};

/// Persisted snapshot of agent state.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub state: CoreState,
    pub entries: Vec<MemoryEntry>,
    pub contents: Vec<String>,
}

/// Load a snapshot from disk. Returns `None` if the file doesn't exist.
pub fn load(path: &Path) -> Result<Option<Snapshot>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read(path).map_err(|e| format!("read snapshot: {e}"))?;
    let snap: Snapshot =
        serde_json::from_slice(&data).map_err(|e| format!("parse snapshot: {e}"))?;
    Ok(Some(snap))
}

/// Save a snapshot to disk (atomic via write-to-tmp + rename).
pub fn save(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let data = serde_json::to_vec(snapshot).map_err(|e| format!("serialize snapshot: {e}"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &data).map_err(|e| format!("write snapshot tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename snapshot: {e}"))?;
    Ok(())
}

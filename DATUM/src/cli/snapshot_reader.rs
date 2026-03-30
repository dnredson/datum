use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::config::Config;
use crate::core::model::Snapshot;

pub fn snapshot_path_from_config(cfg: &Config) -> Result<PathBuf> {
    let file_cfg = cfg
        .outputs
        .file_json
        .as_ref()
        .ok_or_else(|| anyhow!("outputs.file_json is not configured"))?;

    if !file_cfg.enabled {
        return Err(anyhow!("outputs.file_json.enabled=false; no snapshot file to read"));
    }

    Ok(PathBuf::from(&file_cfg.path))
}

pub fn read_snapshot_from_config(cfg: &Config) -> Result<Snapshot> {
    let path = snapshot_path_from_config(cfg)?;
    read_snapshot_from_path(&path)
}

pub fn read_snapshot_from_path(path: &PathBuf) -> Result<Snapshot> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read snapshot file: {}", path.display()))?;

    let snapshot: Snapshot = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse snapshot JSON: {}", path.display()))?;

    Ok(snapshot)
}
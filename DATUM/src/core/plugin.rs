use anyhow::Result;

use super::model::Snapshot;

/// Plugin nativo (Rust). Mais pra frente, um runner WASM vai seguir o mesmo contrato.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;

    /// Permite enriquecer snapshot (ex.: adicionar entidades/edges/events)
    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()>;
}

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::core::model::Snapshot;

pub struct FileJsonOutput {
    path: String,
}

impl FileJsonOutput {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    pub fn emit(&self, snapshot: &Snapshot) -> Result<()> {
        let json = serde_json::to_string_pretty(snapshot).context("serialize snapshot")?;

        // Escrita atômica simples: escreve em temp e renomeia
        let tmp_path = format!("{}.tmp", self.path);
        fs::write(&tmp_path, json).with_context(|| format!("write {}", tmp_path))?;

        // Renomeia para destino
        if Path::new(&self.path).exists() {
            // remove o destino antigo antes do rename no Windows; no Linux normalmente não precisa,
            // mas fica mais previsível.
            let _ = fs::remove_file(&self.path);
        }
        fs::rename(&tmp_path, &self.path).with_context(|| format!("rename to {}", self.path))?;
        Ok(())
    }
}

use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use datum::dcode::module_store::DCodeModuleStore;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcode-install")]
#[command(about = "Install a D-Code WASM module into this node's local content-addressed module store (ADR-0021)")]
struct Args {
    /// Path to the compiled `.wasm` module bytes.
    #[arg(long)]
    module_path: String,

    /// The module's declared `sha256` (from the canonical D-Serv artifact's
    /// `dcode.sha256`), bare hex or `sha256:`-prefixed. Verified against the
    /// actual bytes before installation -- a mismatch is refused.
    #[arg(long)]
    declared_sha256: String,

    /// Root directory of the local D-Code module store.
    #[arg(long, default_value = "dcode-store")]
    store_root: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let bytes = fs::read(&args.module_path)
        .with_context(|| format!("read D-Code module bytes from {}", args.module_path))?;

    let store = DCodeModuleStore::new(&args.store_root);
    let digest = store
        .install(&bytes, &args.declared_sha256)
        .map_err(|error| anyhow::anyhow!(error))
        .context("install D-Code module")?;

    println!(
        "mode=smartsentinel-dcode-install status=ok sha256={digest} store_root={} side_effects=local_module_store_write",
        args.store_root
    );
    Ok(())
}

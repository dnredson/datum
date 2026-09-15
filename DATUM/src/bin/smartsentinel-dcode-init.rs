use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use datum::dcode::local_identity::init_local_identity;

/// Independent review v4 (G1): explicitly initializes this installation's
/// LOCAL D-Node runtime identity. This is a one-time (or idempotent-repeat)
/// operational step, deliberately separate from Sentinel bootstrap/lease and
/// from canonical D-Node structural authority -- see
/// `local-dnode-identity-audit.txt`. The governed `smartsentinel-dcode-invoke`
/// CLI reads the identity this command writes; it never accepts a raw
/// per-invocation `--node-id` override.
#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcode-init")]
#[command(about = "Initialize this installation's local D-Node runtime identity for governed D-Code invocation (ADR-0021)")]
struct Args {
    /// The logical D-Node identity this installation represents. Must match
    /// the `node_id` DServer's canonical D-Node structural registry declares
    /// for the physical/logical node actually running this installation.
    #[arg(long)]
    node_id: String,

    /// Root directory for this D-Node's runtime state (local identity file,
    /// D-Code module store). A distinct root represents a distinct logical
    /// D-Node -- one host MAY run multiple installations against different
    /// roots to represent multiple logical D-Nodes.
    #[arg(long, default_value = "dcode-store")]
    dnode_root: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    init_local_identity(Path::new(&args.dnode_root), &args.node_id)
        .map_err(|error| anyhow::anyhow!(error))
        .context("initialize local D-Node identity")?;
    println!(
        "mode=smartsentinel-dcode-init status=ok node_id={} dnode_root={} side_effects=local_identity_write",
        args.node_id, args.dnode_root
    );
    Ok(())
}

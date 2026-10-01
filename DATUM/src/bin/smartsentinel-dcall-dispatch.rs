use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use datum::dcall::delivery::{dispatch_validated_emit, SourceExecutionContext};
use datum::dcall::evidence::DCallDeliveryOutcome;
use datum::dcode::evidence::DCodeInvocationOutcome;
use datum::dcode::invoke::invoke_dcode_governed_capturing_emits;
use datum::dcode::module_store::DCodeModuleStore;

/// Phase 113B4B: the missing CLI surface for the ALREADY GOVERNED, ALREADY
/// PROVEN production source-side D-Call dispatch path. This binary adds NO
/// new dispatch logic of its own -- it is a thin wrapper mirroring
/// `smartsentinel-dcode-invoke`'s own `invoke` subcommand's authorization/
/// artifact-fetch/local-identity discipline exactly, then hands the
/// resulting validated emits to the SAME `dcall::delivery::dispatch_validated_emit`
/// function already exercised end to end (real DServer, real MQTT broker,
/// real destination listener) in Phase 113B4A's `dcall_mqtt_governed_e2e.rs`
/// integration test. Before 113B4B, this composite "invoke source D-Code
/// AND dispatch its emit via the real governed D-Call path" operation was
/// reachable only from Rust test code, never from a standalone production
/// binary -- `smartsentinel-dcode-invoke` explicitly documents itself as
/// "no D-Call". This exists so the Phase-113 live multinode closure proof
/// can run through an actual production entrypoint instead of a test
/// binary.
#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcall-dispatch")]
#[command(
    about = "Governed production path: invoke one source D-Code D-Serv under DServer authority, capture its validated emits, and dispatch each through the real production D-Call grant/redemption/(HTTP or MQTT)-transport path."
)]
struct Cli {
    /// Base URL of the DServer instance, e.g. http://localhost:8080.
    #[arg(long)]
    dserver_url: String,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    application_id: String,
    /// The SOURCE service_id whose D-Code is invoked.
    #[arg(long)]
    service_id: String,
    /// ADR-0003 envelope input port -- must be one of the artifact's
    /// declared `ports.inputs`.
    #[arg(long)]
    input_port: String,
    /// Root directory of this D-Node's runtime state: the local D-Node
    /// identity file (see `smartsentinel-dcode-init`) AND the D-Code module
    /// store both live here. Mirrors `smartsentinel-dcode-invoke`'s own
    /// governed `--dnode-root`: local node identity is resolved ONLY from
    /// this root, never from a caller-supplied node-id argument.
    #[arg(long, default_value = "dcode-store")]
    dnode_root: String,
    #[arg(long, default_value = "-")]
    input_path: String,
}

fn read_input(input_path: &str) -> Result<Vec<u8>> {
    if input_path == "-" {
        use std::io::Read;
        let mut buffer = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buffer)
            .context("read input bytes from stdin")?;
        Ok(buffer)
    } else {
        std::fs::read(input_path).with_context(|| format!("read input bytes from {input_path}"))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let local_node_id = datum::dcode::local_identity::load_local_identity(Path::new(&cli.dnode_root))
        .map_err(|error| anyhow::anyhow!(error))
        .context("resolve local D-Node identity (run `smartsentinel-dcode-init` first)")?
        .node_id;
    // Phase 117H: governed preflight resolves the exact authorized D-Code
    // descriptor revision -- never the old logical fetch, which cannot
    // execute the valid governed scenario where multiple revisions coexist.
    let preflight = datum::dcode::client::fetch_governed_dcode_execution_material(
        &cli.dserver_url,
        &cli.project_id,
        &cli.application_id,
        &cli.service_id,
        &local_node_id,
    )
    .context("governed D-Code preflight (authorization + exact descriptor revision) from DServer")?;
    let authorization = preflight.authorization;
    let artifact = preflight.artifact;

    let input = read_input(&cli.input_path)?;
    let module_store = DCodeModuleStore::new(Path::new(&cli.dnode_root));
    let result = invoke_dcode_governed_capturing_emits(
        &module_store,
        &authorization,
        &artifact,
        Path::new(&cli.dnode_root),
        &cli.input_port,
        &input,
    )
    .map_err(|error| anyhow::anyhow!(error))
    .context("governed source D-Code invocation")?;

    let source_evidence_json = serde_json::to_string_pretty(result.evidence())
        .context("serialize source D-Code invocation evidence")?;
    println!("{source_evidence_json}");
    println!(
        "mode=smartsentinel-dcall-dispatch stage=source-invoke status={} outcome={:?} node_id={} service_id={}",
        if result.evidence().outcome == DCodeInvocationOutcome::Success { "ok" } else { "failed" },
        result.evidence().outcome,
        result.evidence().node_id,
        result.evidence().service_id
    );
    if result.evidence().outcome != DCodeInvocationOutcome::Success {
        std::process::exit(1);
    }

    let context = SourceExecutionContext {
        dserver_url: cli.dserver_url.clone(),
        dnode_root: cli.dnode_root.clone(),
    };

    let mut all_success = true;
    let mut any_dispatch = false;
    for (emit_index, emit) in result.validated_emits().iter().enumerate() {
        let port = emit.port().to_string();
        let delivery_results = dispatch_validated_emit(&context, &result, emit_index);
        for evidence in &delivery_results {
            any_dispatch = true;
            let evidence_json = serde_json::to_string_pretty(evidence)
                .context("serialize D-Call delivery evidence")?;
            println!("{evidence_json}");
            let succeeded = matches!(evidence.outcome, DCallDeliveryOutcome::Success);
            println!(
                "mode=smartsentinel-dcall-dispatch stage=dcall-dispatch emit_port={port} call_id={} status={} outcome={:?} grant_id={} error_code={}",
                evidence.call_id,
                if succeeded { "ok" } else { "failed" },
                evidence.outcome,
                evidence.grant_id,
                evidence.error_code.as_deref().unwrap_or("-"),
            );
            if !succeeded {
                all_success = false;
            }
        }
    }

    if !any_dispatch {
        eprintln!("no D-Call edge resolved for any validated emit -- nothing was dispatched");
        std::process::exit(1);
    }
    if !all_success {
        std::process::exit(1);
    }
    Ok(())
}

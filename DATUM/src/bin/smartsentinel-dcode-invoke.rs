use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand};
use datum::dcode::authorization::DCodeInvocationAuthorizationV1;
use datum::dcode::dserv_artifact::CanonicalDServArtifactV1;
use datum::dcode::evidence::DCodeInvocationOutcome;
use datum::dcode::invoke::invoke_dcode;
use datum::dcode::module_store::DCodeModuleStore;
use datum::dcode::client;
use datum::dcode::local_identity::load_local_identity;
use datum::dcode::worker::IsolatedWorkerExecutor;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcode-invoke")]
#[command(about = "Invoke one canonical D-Code D-Serv locally (ADR-0021) -- v0.1: input is supplied directly, no D-Call")]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    /// GOVERNED PRODUCTION PATH (independent review v2, R1): obtains
    /// authorization AND the authoritative canonical D-Serv artifact
    /// directly from DServer immediately before execution. Use this for
    /// real invocation.
    Invoke(InvokeArgs),
    /// INTERNAL/TEST-ONLY (independent review v2, R1): bypasses DServer
    /// entirely using locally-supplied authorization + artifact files.
    /// NEVER use this for real governed execution -- it exists only so
    /// unit/integration tests can exercise the invocation path without a
    /// running DServer.
    OfflineTestOnly(OfflineArgs),
}

#[derive(ClapArgs, Debug)]
struct InvokeArgs {
    /// Base URL of the DServer instance to obtain authorization/artifact
    /// from, e.g. http://localhost:8080.
    #[arg(long)]
    dserver_url: String,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    application_id: String,
    #[arg(long)]
    service_id: String,
    /// ADR-0003 envelope input port -- must be one of the artifact's
    /// declared `ports.inputs`.
    #[arg(long)]
    input_port: String,
    /// Root directory of this D-Node's runtime state: the local D-Node
    /// identity file (see `smartsentinel-dcode-init`) AND the D-Code module
    /// store both live here. Independent review v4 (G1): the GOVERNED path
    /// deliberately does NOT accept a raw `--node-id` argument -- accepting
    /// one would let a caller claim to BE any node it likes, which DServer
    /// authorization alone cannot disprove (DServer can only prove the
    /// REQUESTED node_id is structurally authorized to run this service, not
    /// that the calling process actually IS that node). `local_node_id` is
    /// resolved from this root's own identity file and used, unmodified by
    /// any command-line input, for BOTH the DServer authorization request
    /// and the local `invoke_dcode` equality check.
    #[arg(long, default_value = "dcode-store")]
    dnode_root: String,
    #[arg(long, default_value = "-")]
    input_path: String,
    #[arg(long)]
    evidence_out: Option<String>,
}

#[derive(ClapArgs, Debug)]
struct OfflineArgs {
    /// INTERNAL/TEST-ONLY: path to a `DCodeInvocationAuthorizationV1` JSON
    /// file. Never obtained from DServer in this mode.
    #[arg(long)]
    authorization_path: String,
    /// INTERNAL/TEST-ONLY: path to the `datum.dserv-artifact/1` JSON file.
    #[arg(long)]
    artifact_path: String,
    #[arg(long)]
    node_id: String,
    #[arg(long)]
    input_port: String,
    #[arg(long, default_value = "dcode-store")]
    store_root: String,
    #[arg(long)]
    worker_binary: Option<String>,
    #[arg(long, default_value = "-")]
    input_path: String,
    #[arg(long)]
    evidence_out: Option<String>,
}

struct Resolved {
    authorization: DCodeInvocationAuthorizationV1,
    artifact: CanonicalDServArtifactV1,
    node_id: String,
    input_port: String,
    store_root: String,
    /// Independent review v4 (G2): the GOVERNED path never sets this --
    /// `worker_binary` is only ever `Some` via `offline-test-only`'s explicit
    /// override. `main` resolves the trusted sibling worker whenever this is
    /// `None`.
    worker_binary: Option<String>,
    input_path: String,
    evidence_out: Option<String>,
}

fn default_worker_binary_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("current_exe should resolve");
    path.set_file_name(if cfg!(windows) { "smartsentinel-dcode-worker.exe" } else { "smartsentinel-dcode-worker" });
    path
}

fn read_input(input_path: &str) -> Result<Vec<u8>> {
    if input_path == "-" {
        use std::io::Read;
        let mut buffer = Vec::new();
        std::io::stdin().read_to_end(&mut buffer).context("read input bytes from stdin")?;
        Ok(buffer)
    } else {
        fs::read(input_path).with_context(|| format!("read input bytes from {input_path}"))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let resolved = match cli.mode {
        Mode::Invoke(args) => {
            // G1: `local_node_id` is resolved ONLY from this installation's
            // own local identity file -- never from a command-line argument
            // -- and is then used, unmodified, for BOTH the authorization
            // request below and the local equality check inside
            // `invoke_dcode`. This is what makes "authorization targets THIS
            // node" a true statement instead of a circular one: the SAME
            // value never round-trips through caller input on its way to
            // becoming the thing it is checked against.
            let local_node_id = load_local_identity(Path::new(&args.dnode_root))
                .map_err(|error| anyhow::anyhow!(error))
                .context("resolve local D-Node identity (run `smartsentinel-dcode-init` first)")?
                .node_id;
            let authorization = client::fetch_dcode_authorization(
                &args.dserver_url,
                &args.project_id,
                &args.application_id,
                &args.service_id,
                &local_node_id,
            )
            .context("fetch D-Code authorization from DServer (governed path)")?;
            let artifact = client::fetch_dserv_artifact(&args.dserver_url, &args.application_id, &args.service_id)
                .context("fetch canonical D-Serv artifact from DServer (governed path)")?;
            Resolved {
                authorization,
                artifact,
                node_id: local_node_id,
                input_port: args.input_port,
                store_root: args.dnode_root,
                // G2: the GOVERNED path never accepts a worker-binary
                // override -- production worker location is always the
                // trusted sibling of this executable.
                worker_binary: None,
                input_path: args.input_path,
                evidence_out: args.evidence_out,
            }
        }
        Mode::OfflineTestOnly(args) => {
            eprintln!(
                "WARNING: offline-test-only mode bypasses DServer authorization entirely -- NEVER use this for real governed execution."
            );
            let authorization_json = fs::read_to_string(&args.authorization_path)
                .with_context(|| format!("read authorization {}", args.authorization_path))?;
            let authorization: DCodeInvocationAuthorizationV1 =
                serde_json::from_str(&authorization_json).context("parse DCodeInvocationAuthorizationV1")?;
            let artifact_json = fs::read_to_string(&args.artifact_path)
                .with_context(|| format!("read artifact {}", args.artifact_path))?;
            let artifact: CanonicalDServArtifactV1 =
                serde_json::from_str(&artifact_json).context("parse datum.dserv-artifact/1 for invocation")?;
            Resolved {
                authorization,
                artifact,
                node_id: args.node_id,
                input_port: args.input_port,
                store_root: args.store_root,
                worker_binary: args.worker_binary,
                input_path: args.input_path,
                evidence_out: args.evidence_out,
            }
        }
    };

    let input = read_input(&resolved.input_path)?;
    let worker_binary = resolved.worker_binary.map(PathBuf::from).unwrap_or_else(default_worker_binary_path);
    let store_root: &Path = Path::new(&resolved.store_root);
    let module_store = DCodeModuleStore::new(store_root);
    let executor = IsolatedWorkerExecutor { worker_binary: &worker_binary, module_store_root: store_root };

    let evidence = invoke_dcode(
        &executor,
        &module_store,
        &resolved.authorization,
        &resolved.artifact,
        &resolved.node_id,
        &resolved.input_port,
        &input,
    );

    let evidence_json = serde_json::to_string_pretty(&evidence).context("serialize D-Code invocation evidence")?;
    match &resolved.evidence_out {
        Some(path) => fs::write(path, format!("{evidence_json}\n")).with_context(|| format!("write evidence to {path}"))?,
        None => println!("{evidence_json}"),
    }

    let succeeded = evidence.outcome == DCodeInvocationOutcome::Success;
    println!(
        "mode=smartsentinel-dcode-invoke status={} outcome={:?} node_id={} service_id={} host_mutation=false",
        if succeeded { "ok" } else { "failed" },
        evidence.outcome,
        evidence.node_id,
        evidence.service_id
    );

    // Independent review v2, R6: exit non-zero for any non-Success outcome
    // -- a prior version of this CLI always exited 0 and printed
    // "status=ok" even for StaleAuthority/ValidationFailed/ResourceLimit/
    // Trap, which an automated caller could not distinguish from real
    // success without parsing evidence JSON.
    if !succeeded {
        std::process::exit(1);
    }
    Ok(())
}

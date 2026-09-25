use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand};
use datum::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter;
use datum::agent::dmonitor_emission::DMonitorEmissionContext;
use datum::dcode::authorization::DCodeInvocationAuthorizationV1;
use datum::dcode::client;
use datum::dcode::dserv_artifact::CanonicalDServArtifactV1;
use datum::dcode::evidence::DCodeInvocationOutcome;
use datum::dcode::invoke::invoke_dcode;
use datum::dcode::local_identity::load_local_identity;
use datum::dcode::module_store::DCodeModuleStore;
use datum::dcode::worker::IsolatedWorkerExecutor;
use datum::outputs::dmonitor_http::DMonitorHttpOutput;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcode-invoke")]
#[command(
    about = "Invoke one canonical D-Code D-Serv locally (ADR-0021) -- v0.1: input is supplied directly, no D-Call"
)]
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
    /// Phase 115B: best-effort, opt-in publication of one canonical
    /// `DServRealization` DMonitor observation built from this invocation's
    /// own governed evidence, sent to `dserver_url` (never a second
    /// server). A publish failure is logged to stderr and NEVER changes
    /// this command's exit code -- monitoring must never destabilize the
    /// governed invocation path it observes (mirrors
    /// `dcall::dmonitor_publisher`'s own failure-isolation principle).
    #[arg(long, default_value_t = false)]
    dmonitor_publish: bool,
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
    /// `Some(dserver_url)` only when `--dmonitor-publish` was passed on the
    /// GOVERNED `invoke` path -- `offline-test-only` never publishes.
    dmonitor_publish_to: Option<String>,
}

fn default_worker_binary_path() -> PathBuf {
    datum::dcode::worker::trusted_sibling_worker_binary_path()
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
            let artifact = client::fetch_dserv_artifact(
                &args.dserver_url,
                &args.application_id,
                &args.service_id,
            )
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
                dmonitor_publish_to: args.dmonitor_publish.then_some(args.dserver_url),
            }
        }
        Mode::OfflineTestOnly(args) => {
            eprintln!(
                "WARNING: offline-test-only mode bypasses DServer authorization entirely -- NEVER use this for real governed execution."
            );
            let authorization_json = fs::read_to_string(&args.authorization_path)
                .with_context(|| format!("read authorization {}", args.authorization_path))?;
            let authorization: DCodeInvocationAuthorizationV1 =
                serde_json::from_str(&authorization_json)
                    .context("parse DCodeInvocationAuthorizationV1")?;
            let artifact_json = fs::read_to_string(&args.artifact_path)
                .with_context(|| format!("read artifact {}", args.artifact_path))?;
            let artifact: CanonicalDServArtifactV1 = serde_json::from_str(&artifact_json)
                .context("parse datum.dserv-artifact/1 for invocation")?;
            Resolved {
                authorization,
                artifact,
                node_id: args.node_id,
                input_port: args.input_port,
                store_root: args.store_root,
                worker_binary: args.worker_binary,
                input_path: args.input_path,
                evidence_out: args.evidence_out,
                // NEVER publish from the offline/test-only bypass path -- it
                // has no governed DServer-issued authorization to honestly
                // attribute an observation to.
                dmonitor_publish_to: None,
            }
        }
    };

    let input = read_input(&resolved.input_path)?;
    let worker_binary = resolved
        .worker_binary
        .map(PathBuf::from)
        .unwrap_or_else(default_worker_binary_path);
    let store_root: &Path = Path::new(&resolved.store_root);
    let module_store = DCodeModuleStore::new(store_root);
    let executor = IsolatedWorkerExecutor {
        worker_binary: &worker_binary,
        module_store_root: store_root,
    };

    let evidence = invoke_dcode(
        &executor,
        &module_store,
        &resolved.authorization,
        &resolved.artifact,
        &resolved.node_id,
        &resolved.input_port,
        &input,
    );

    let evidence_json =
        serde_json::to_string_pretty(&evidence).context("serialize D-Code invocation evidence")?;
    match &resolved.evidence_out {
        Some(path) => fs::write(path, format!("{evidence_json}\n"))
            .with_context(|| format!("write evidence to {path}"))?,
        None => println!("{evidence_json}"),
    }

    // Phase 115B: best-effort, opt-in DMonitor evidence publication. Never
    // affects this command's own exit code (see `--dmonitor-publish`'s own
    // doc comment) -- a failure here is logged and swallowed, exactly like
    // `dcall::dmonitor_publisher`'s own failure-isolation principle.
    if let Some(dserver_url) = &resolved.dmonitor_publish_to {
        let mut emitter = DCodeExecutionMonitorEmitter::new(
            DMonitorEmissionContext::new_process_epoch(&resolved.node_id),
            resolved.node_id.clone(),
        );
        match emitter.emit(&evidence) {
            Some(observation) => match DMonitorHttpOutput::new(dserver_url.clone(), evidence.project_id.clone(), 5) {
                Ok(output) => match serde_json::to_value(&observation) {
                    Ok(observation_json) => {
                        if let Err(error) = output.publish(&observation.observation_id, &observation_json) {
                            eprintln!("dmonitor_publish_failed observation_id={} error={error}", observation.observation_id);
                        }
                    }
                    Err(error) => eprintln!("dmonitor_publish_serialize_failed error={error}"),
                },
                Err(error) => eprintln!("dmonitor_publish_client_build_failed error={error}"),
            },
            None => eprintln!(
                "dmonitor_publish_skipped reason=node_identity_mismatch evidence_node_id={} local_node_id={}",
                evidence.node_id, resolved.node_id
            ),
        }
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

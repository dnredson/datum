//! Production MQTT D-Call delivery listener (Phase 113B3, ADR-0023,
//! section 29/15). Mirrors `smartsentinel-dcall-listen`'s own governance
//! discipline exactly: local D-Node identity is always resolved from
//! `--dnode-root`'s own identity file (never a CLI/request override, the
//! same G1-equivalent guard); destination service/DMap/DForward/artifact
//! identity is entirely grant-derived -- there is deliberately no
//! `--channel`/`--broker-uri`/`--service-id` flag, since all three are
//! authority DServer resolves from the CURRENT DIoT runtime binding, never
//! caller input.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use datum::dcall::delivery::run_mqtt_dcall_listener;
use datum::dcode::local_identity::load_local_identity;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcall-mqtt-listen")]
#[command(
    about = "Governed MQTT D-Call delivery listener (Phase 113B3) -- destination side of a via_diot_pubsub D-Call delivery"
)]
struct Args {
    /// Base URL of the DServer instance this D-Node belongs to.
    #[arg(long)]
    dserver_url: String,

    /// Root directory of this D-Node's runtime state (same convention as
    /// `smartsentinel-dcall-listen`).
    #[arg(long, default_value = "dcode-store")]
    dnode_root: String,

    /// The governed DIoT placement this listener serves. The exact runtime
    /// binding (broker URI, transport) is resolved from DServer's CURRENT
    /// authority-proven binding for this placement -- never a CLI flag.
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    application_id: String,
    #[arg(long)]
    placement_id: String,

    /// How often, while the governed MQTT session remains established, to
    /// emit a fresh `Healthy` DIoT DMonitor heartbeat observation. Purely
    /// operational emission cadence -- NOT the Phase-114D1 freshness/
    /// readiness policy DServer will evaluate evidence against.
    #[arg(long, default_value_t = 30)]
    dmonitor_heartbeat_interval_seconds: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.dmonitor_heartbeat_interval_seconds == 0 {
        anyhow::bail!("--dmonitor-heartbeat-interval-seconds must be greater than zero");
    }

    // G1-equivalent: local D-Node identity comes ONLY from this
    // installation's own identity file.
    let identity = load_local_identity(std::path::Path::new(&args.dnode_root))
        .map_err(|error| anyhow::anyhow!(error))
        .context("load local D-Node identity (run smartsentinel-dcode-init first)")?;

    println!(
        "mode=smartsentinel-dcall-mqtt-listen status=starting node_id={} project_id={} application_id={} placement_id={}",
        identity.node_id, args.project_id, args.application_id, args.placement_id
    );

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_signal = stop.clone();
    ctrlc::set_handler(move || stop_for_signal.store(true, Ordering::SeqCst))
        .context("install Ctrl-C handler")?;

    run_mqtt_dcall_listener(
        &args.dserver_url,
        &args.dnode_root,
        &args.project_id,
        &args.application_id,
        &args.placement_id,
        std::time::Duration::from_secs(args.dmonitor_heartbeat_interval_seconds),
        &stop,
    )
    .context("run MQTT D-Call delivery listener")?;

    println!(
        "mode=smartsentinel-dcall-mqtt-listen status=ok node_id={} shutdown=graceful",
        identity.node_id
    );
    Ok(())
}

//! Production cross-D-Node D-Call delivery listener (ADR-0022, section 18).
//!
//! This binary is the GOVERNED destination side of a remote D-Call
//! delivery. It never accepts a caller-provided node-id override (section
//! 18, consistent with Phase 111's G1 fix): local D-Node identity is always
//! resolved from `--dnode-root`'s own identity file
//! (`smartsentinel-dcode-init` must have already provisioned it). It never
//! accepts a caller-selected destination service/port/artifact -- those
//! come exclusively from the grant DServer issues, verified again at
//! redemption time (section 16, "DESTINATION SEQUENCE").

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use datum::dcall::client;
use datum::dcall::delivery::receive_governed_delivery;
use datum::dcall::listener::{run, DeliverRequest, DeliverResponse};
use datum::dcode::identity::sha256_hex;
use datum::dcode::local_identity::load_local_identity;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-dcall-listen")]
#[command(
    about = "Governed cross-D-Node D-Call delivery listener (ADR-0022) -- destination side of a remote D-Call delivery"
)]
struct Args {
    /// Base URL of the DServer instance this D-Node belongs to.
    #[arg(long)]
    dserver_url: String,

    /// Root directory of this D-Node's runtime state: the local D-Node
    /// identity file (see `smartsentinel-dcode-init`) AND the D-Code module
    /// store both live here. There is deliberately no `--node-id` flag --
    /// see the module doc.
    #[arg(long, default_value = "dcode-store")]
    dnode_root: String,

    /// Address to bind the listener to. Defaults to an EPHEMERAL loopback
    /// port -- section 18: "default bind SHOULD be loopback if no explicit
    /// value is supplied."
    #[arg(long, default_value = "127.0.0.1:0")]
    bind: String,

    /// The URL this D-Node's endpoint should be ADVERTISED as to DServer
    /// (i.e. what a remote source dispatcher will actually connect to).
    /// Section 18: "If bind != advertise URL: require explicit advertise
    /// URL" -- if omitted, this listener serves requests but does NOT
    /// register/update its DServer-side transport endpoint (an operator or
    /// a separate tool may still declare it explicitly via the ordinary
    /// `PUT /api/v1/dcall/endpoints/:node_id/:transport` API). This
    /// deliberately never derives an advertise URL from network-interface
    /// scanning.
    #[arg(long)]
    advertise_url: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // G1-equivalent for D-Call: local D-Node identity comes ONLY from this
    // installation's own identity file, never from a CLI/request override.
    let identity = load_local_identity(std::path::Path::new(&args.dnode_root))
        .map_err(|error| anyhow::anyhow!(error))
        .context("load local D-Node identity (run smartsentinel-dcode-init first)")?;
    let local_node_id = identity.node_id;

    let listener = TcpListener::bind(&args.bind)
        .with_context(|| format!("bind D-Call listener to {}", args.bind))?;
    let bound_addr = listener
        .local_addr()
        .context("resolve bound listener address")?;

    match &args.advertise_url {
        Some(advertise_url) => {
            client::declare_endpoint(&args.dserver_url, &local_node_id, "http", advertise_url)
                .context("declare this D-Node's D-Call transport endpoint with DServer")?;
            println!("mode=smartsentinel-dcall-listen status=ok node_id={local_node_id} bound={bound_addr} advertised={advertise_url} endpoint_declared=true");
        }
        None => {
            println!("mode=smartsentinel-dcall-listen status=ok node_id={local_node_id} bound={bound_addr} endpoint_declared=false reason=no_advertise_url_supplied");
        }
    }

    let dserver_url = args.dserver_url.clone();
    let dnode_root = args.dnode_root.clone();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_signal = stop.clone();
    ctrlc::set_handler(move || stop_for_signal.store(true, Ordering::SeqCst))
        .context("install Ctrl-C handler")?;

    run(
        listener,
        move |request: DeliverRequest| -> (u16, DeliverResponse) {
            handle_delivery(&dserver_url, &dnode_root, request)
        },
        stop,
    );

    println!("mode=smartsentinel-dcall-listen status=ok node_id={local_node_id} shutdown=graceful");
    Ok(())
}

/// Section 16 "DESTINATION SEQUENCE": redeem the grant (which independently
/// re-verifies token/expiry/destination-node/payload-identity/current
/// canonical authority server-side) BEFORE ever invoking D-Code -- a
/// request is never executed merely because it claims a grant_id/token.
fn handle_delivery(
    dserver_url: &str,
    dnode_root: &str,
    request: DeliverRequest,
) -> (u16, DeliverResponse) {
    use base64::Engine;
    let payload = match base64::engine::general_purpose::STANDARD.decode(&request.payload_b64) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                200,
                DeliverResponse {
                    outcome: "failed".to_string(),
                    error_code: Some("dcall_malformed_payload".to_string()),
                    message: Some("payload_b64 is not valid base64".to_string()),
                    dcode_evidence: None,
                },
            )
        }
    };

    // Self-review finding (dcall-payload-leakage-audit.txt): `redeem_grant`
    // only compares the CALLER-DECLARED `request.payload_sha256`/
    // `payload_size` against the grant's bound values -- it has no way to
    // see the actual decoded `payload` bytes at all. Without this local
    // recomputation, a request could declare the digest the grant expects
    // while carrying DIFFERENT actual bytes in `payload_b64`, passing
    // redemption but then executing on tampered content. Verify the
    // declared fields actually describe the bytes we decoded BEFORE
    // spending a real (single-use) grant redemption on them.
    let actual_sha256 = sha256_hex(&payload);
    if actual_sha256 != request.payload_sha256 || payload.len() as u64 != request.payload_size {
        return (
            200,
            DeliverResponse {
                outcome: "failed".to_string(),
                error_code: Some("dcall_payload_digest_mismatch".to_string()),
                message: Some(
                    "decoded payload_b64 does not match the declared payload_sha256/payload_size"
                        .to_string(),
                ),
                dcode_evidence: None,
            },
        );
    }

    match receive_governed_delivery(
        dserver_url,
        dnode_root,
        &request.grant_id,
        &request.token,
        &payload,
    ) {
        Ok(evidence) => {
            let evidence_json = serde_json::to_value(&evidence).unwrap_or(serde_json::Value::Null);
            (
                200,
                DeliverResponse {
                    outcome: "success".to_string(),
                    error_code: None,
                    message: None,
                    dcode_evidence: Some(evidence_json),
                },
            )
        }
        Err(error) => (
            200,
            DeliverResponse {
                outcome: "failed".to_string(),
                error_code: Some("dcall_destination_execution_failed".to_string()),
                message: Some(error.to_string()),
                dcode_evidence: None,
            },
        ),
    }
}

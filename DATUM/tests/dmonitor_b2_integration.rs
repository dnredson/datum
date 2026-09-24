//! Phase 114B1 + 114B2 integration proof: a realistic SmartSentinel
//! telemetry fixture, projected through the PRODUCTION compatibility
//! projector, published through the PRODUCTION DMonitor HTTP output, into a
//! REAL (freshly `cargo build`-ed) DServer process, then read back through
//! the real Phase-114B1 ingestion route/state logic. No fog/cloud/live VM
//! requirement -- an in-process local server fixture only, mirroring the
//! established `spawn_real_dserver` pattern from `dcall_two_node_integration.rs`
//! / `dcall_mqtt_governed_e2e.rs`.
//!
//! `#[ignore]`d for the same reason those tests are: it builds and spawns a
//! real DServer subprocess, which is too slow for the default fast test
//! loop. Run explicitly:
//!
//!   cargo test --locked --test dmonitor_b2_integration -- --ignored --test-threads=1

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use chrono::Utc;
use datum::agent::canonical_dmonitor::{DMonitorHealthV1, DMonitorSubjectV1};
use datum::agent::dmonitor_projection::{
    project_telemetry_to_dmonitor_observations, DMonitorProjectionContext,
};
use datum::outputs::dmonitor_http::{DMonitorHttpOutput, DMonitorPublishOutcome};
use datum::telemetry::{
    ContainerTelemetry, ControlPlaneTelemetry, CpuTelemetry, HostTelemetry, MemoryTelemetry,
    NetworkTelemetry, RuntimeTelemetry, SmartSentinelTelemetryEnvelope, SwapTelemetry,
    SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION,
};

const PROJECT: &str = "proj:dmonitor-b2-integration";
const OTHER_PROJECT: &str = "proj:dmonitor-b2-integration-other";
const NODE: &str = "fog-01";

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_port(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if Instant::now() > deadline {
            panic!("nothing listening on 127.0.0.1:{port} after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn dserver_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("dserver")
}

fn dserver_binary_path() -> PathBuf {
    dserver_dir()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) { "DServer.exe" } else { "DServer" })
}

/// Mirrors `dcall_mqtt_governed_e2e.rs`'s own `spawn_real_dserver` exactly
/// (this project's established pattern: reuse it, do not invent a new one),
/// minus the D-Code sibling-worker provisioning this test does not need.
fn spawn_real_dserver(tmp: &std::path::Path) -> (String, KillOnDrop) {
    let build_status = Command::new("cargo")
        .args(["build", "--locked", "--bin", "DServer"])
        .current_dir(dserver_dir())
        .status()
        .expect("run cargo build for DServer");
    assert!(build_status.success(), "cargo build --bin DServer failed");

    let port = free_port();
    let config_path = tmp.join("dserver-config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dmonitor-b2-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
        ),
    )
    .unwrap();

    let isolated_binary = tmp.join(if cfg!(windows) { "DServer.exe" } else { "DServer" });
    std::fs::copy(dserver_binary_path(), &isolated_binary).unwrap();
    let child = Command::new(isolated_binary)
        .env("DATUM_DSERVER_CONFIG_PATH", &config_path)
        .env("DATUM_CONTROL_STATE_DB_PATH", tmp.join("control-plane.db"))
        .env(
            "DATUM_OPERATIONAL_DGRAPH_STATE_PATH",
            tmp.join("operational-dgraph-state.json"),
        )
        .current_dir(tmp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real DServer process");

    wait_for_port(port, Duration::from_secs(20));
    (format!("http://127.0.0.1:{port}"), KillOnDrop(child))
}

/// A realistic, hand-built SmartSentinel telemetry fixture: host + one
/// container + one control-plane reachability probe -- deliberately small
/// (3 observations) to keep this integration proof fast while still
/// exercising every subject kind this bridge produces from a runtime source.
fn realistic_telemetry_fixture() -> SmartSentinelTelemetryEnvelope {
    SmartSentinelTelemetryEnvelope {
        schema_version: SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION.to_string(),
        project_id: None,
        node_id: NODE.to_string(),
        cycle_id: "cycle:b2-integration".to_string(),
        sentinel_instance_id: format!("{NODE}-sentinel"),
        sequence: 1,
        // Real "now" (not a fixed fictional past/future date): this fixture
        // is fed through a REAL server whose own `received_at_utc` is
        // stamped from its own wall clock, and this test later checks that
        // relationship -- a hardcoded date could land in the future or past
        // relative to whenever this test actually runs.
        observed_at_utc: Utc::now(),
        host: HostTelemetry {
            hostname: "fog-01-host".to_string(),
            os: Some("linux".to_string()),
            kernel: Some("6.1.0".to_string()),
            uptime_seconds: Some(7200),
            cpu: CpuTelemetry {
                logical_cores: Some(8),
                usage_percent: Some(33.33),
                load_1: Some(1.2),
                load_5: Some(1.5),
                load_15: Some(1.1),
            },
            memory: MemoryTelemetry {
                total_bytes: Some(16_000_000_000),
                used_bytes: Some(8_000_000_000),
                available_bytes: Some(8_000_000_000),
            },
            swap: SwapTelemetry {
                total_bytes: Some(2_000_000_000),
                used_bytes: Some(0),
            },
            filesystems: vec![],
            process_count: Some(200),
        },
        runtime: Some(RuntimeTelemetry {
            container_runtime: Some("docker".to_string()),
            container_runtime_available: Some(true),
            container_count: Some(1),
            container_runtime_error: None,
            containers: vec![ContainerTelemetry {
                entity_id: "entity:container:sensor-bridge-1".to_string(),
                name: "sensor-bridge".to_string(),
                image: Some("sensor-bridge:latest".to_string()),
                state: Some("running".to_string()),
                status: Some("Up 1 hour".to_string()),
            }],
            processes: vec![],
            services: vec![],
        }),
        network: Some(NetworkTelemetry {
            interfaces: vec![],
            control_plane: Some(ControlPlaneTelemetry {
                target: "http://dserver.local:8080".to_string(),
                reachable: true,
                rtt_ms: Some(5.5),
                error: None,
            }),
            probes: vec![],
            peers: vec![],
        }),
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

#[test]
#[ignore = "builds and spawns a real DServer subprocess; run explicitly via \
            `cargo test --locked --test dmonitor_b2_integration -- --ignored --test-threads=1`"]
fn b1_b2_integration_proof() {
    let tmp = std::env::temp_dir().join(format!("datum-dmonitor-b2-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (dserver_url, _guard) = spawn_real_dserver(&tmp);

    // 1/2. project a realistic telemetry fixture through the PRODUCTION
    // compatibility projector.
    let telemetry = realistic_telemetry_fixture();
    let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
    let monitor_instance_id = context.monitor_instance_id().to_string();
    let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context)
        .expect("projection must succeed for a well-formed fixture");
    assert_eq!(observations.len(), 3, "host + container + control-plane = 3 observations");

    // 3. POST through the PRODUCTION DMonitor HTTP output.
    let output = DMonitorHttpOutput::new(dserver_url.clone(), PROJECT.to_string(), 10).unwrap();
    for observation in &observations {
        let json = serde_json::to_value(observation).unwrap();
        let outcome = output.publish(&observation.observation_id, &json).unwrap();
        assert_eq!(outcome, DMonitorPublishOutcome::Accepted);
    }

    // Replaying the exact same observation must be reported idempotent by
    // the REAL server, proving the whole B1 ingestion path is exercised,
    // not merely reachable.
    let dnode_observation = observations
        .iter()
        .find(|o| matches!(o.subject, DMonitorSubjectV1::DNode { .. }))
        .unwrap();
    let replay_outcome = output
        .publish(
            &dnode_observation.observation_id,
            &serde_json::to_value(dnode_observation).unwrap(),
        )
        .unwrap();
    assert_eq!(replay_outcome, DMonitorPublishOutcome::IdempotentReplay);

    // 4/5. query resulting stream head through the REAL DServer route.
    let client = reqwest::blocking::Client::new();
    let head_url = format!(
        "{dserver_url}/api/v1/dmonitor/observations/{PROJECT}/{NODE}/{monitor_instance_id}/head"
    );
    let head_response: serde_json::Value = client.get(&head_url).send().unwrap().json().unwrap();

    // 6. verify identity/isolation/authority invariants.
    assert_eq!(head_response["observation"]["monitor_instance_id"], monitor_instance_id);
    assert_eq!(head_response["observation"]["node_id"], NODE);
    let head_sequence = head_response["observation"]["sequence"].as_u64().unwrap();
    assert_eq!(
        head_sequence,
        observations.iter().map(|o| o.sequence).max().unwrap(),
        "server-reported head sequence must match the highest sequence this test actually sent"
    );
    let expected_digest =
        datum::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(
            observations
                .iter()
                .max_by_key(|o| o.sequence)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(head_response["observation_digest"], expected_digest, "digest must be preserved exactly");
    assert!(
        head_response["received_at_utc"].as_str().is_some(),
        "server must have stamped received_at_utc"
    );
    assert_eq!(head_response["server_project_scope"], PROJECT);

    // No desired-state authority anywhere in the persisted, server-returned
    // record's facts.
    for observation_wire in [&head_response["observation"]] {
        let facts_text = observation_wire["facts"].to_string();
        assert!(!facts_text.contains("desired_state"), "server-returned facts must never carry desired_state");
    }

    // Project isolation: the SAME stream must not resolve under a different
    // project scope.
    let isolated_url = format!(
        "{dserver_url}/api/v1/dmonitor/observations/{OTHER_PROJECT}/{NODE}/{monitor_instance_id}/head"
    );
    let isolated_status = client.get(&isolated_url).send().unwrap().status();
    assert_eq!(isolated_status.as_u16(), 404, "a different project scope must never see this stream");

    // by-id lookup, scoped correctly, must resolve to the SAME record.
    let by_id_url = format!(
        "{dserver_url}/api/v1/dmonitor/observations/{PROJECT}/by-id/{}",
        dnode_observation.observation_id
    );
    let by_id_response: serde_json::Value = client.get(&by_id_url).send().unwrap().json().unwrap();
    assert_eq!(by_id_response["observation"]["observation_id"], dnode_observation.observation_id);

    // by-id lookup under the WRONG project must not leak the record either.
    let by_id_wrong_project_url = format!(
        "{dserver_url}/api/v1/dmonitor/observations/{OTHER_PROJECT}/by-id/{}",
        dnode_observation.observation_id
    );
    let by_id_wrong_status = client.get(&by_id_wrong_project_url).send().unwrap().status();
    assert_eq!(by_id_wrong_status.as_u16(), 404);

    // 7. evaluate freshness on the retrieved record. The freshness
    // ALGORITHM itself (Fresh/Stale/ClockSkew, boundary semantics) is
    // exhaustively unit-tested directly in `dserver::core::dmonitor_freshness`
    // (no shared crate exists per ADR-0007, so this integration test cannot
    // call it directly) -- what this step verifies end-to-end is that the
    // real, persisted-and-retrieved record actually carries the two
    // timestamps that evaluation depends on, and that they are sane
    // (received_at at or after observed_at, both parseable RFC 3339).
    let observed_at: chrono::DateTime<Utc> = head_response["observation"]["observed_at_utc"]
        .as_str()
        .unwrap()
        .parse()
        .expect("observed_at_utc must be valid RFC 3339");
    let received_at: chrono::DateTime<Utc> = head_response["received_at_utc"]
        .as_str()
        .unwrap()
        .parse()
        .expect("received_at_utc must be valid RFC 3339");
    assert!(
        received_at >= observed_at - chrono::Duration::seconds(5),
        "received_at ({received_at}) should not precede observed_at ({observed_at}) by more than ordinary clock skew"
    );
    let evaluated_at = Utc::now();
    let observed_age = evaluated_at - observed_at;
    assert!(observed_age.num_seconds() >= 0, "evaluating freshness now must yield a non-negative observed age");

    // Health projection sanity: the DNode observation's own health (fetched
    // via by-id, NOT the stream head -- the head is whichever observation
    // has the highest sequence, which in this fixture is the control-plane
    // reachability observation, not the host) must be exactly what the
    // projector assigned (Unknown), preserved end to end.
    assert_eq!(by_id_response["observation"]["health"], "unknown");
    let _ = DMonitorHealthV1::Unknown; // referenced for readability of intent above

    std::fs::remove_dir_all(&tmp).ok();
}

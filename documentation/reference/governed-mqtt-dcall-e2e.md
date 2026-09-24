# Governed MQTT D-Call end-to-end proof

`DATUM/tests/dcall_mqtt_governed_e2e.rs` (Phase 113B4A/113B4A.1) proves the
full governed `via_diot_pubsub` D-Call chain end to end through production
entrypoints only, against a real MQTT broker: real `DServer` process, real
DMap/2 proposal/acceptance flow, real DIoT runtime-binding registration,
real source and destination D-Code/WASM execution, a real
`smartsentinel-dcall-mqtt-listen` destination subprocess, and a real MQTT
publish/subscribe/redeem/response round trip. No internal-function shortcut
stands in for any production boundary in the primary success case.

Alongside the primary success proof, four real-broker failure-mode scenarios
are included: a legitimately stale runtime binding, an unreachable broker, a
broker that is reachable but has no destination listener running, and a
physical duplicate MQTT delivery of the same governed request. See the doc
comment on each test function for exactly which production seam each
scenario exercises, and an honest note on the two scenarios (stale binding,
duplicate delivery) that necessarily use the real `request_grant` HTTP call
and the real `dcall::mqtt` publish primitives directly, rather than the
single `dispatch_validated_emit` entrypoint, because that entrypoint issues
its own fresh grant and delivers in one call with no public seam to hold an
already-issued grant across a later change.

## Normal `cargo test` never needs a broker

Every test in this file is `#[ignore]`d. An ordinary `cargo test` run never
attempts to reach a broker, spawn `DServer`, or spawn the MQTT listener.

## Prerequisites to run it manually

- A Rust toolchain matching this repository's `rust-toolchain`/`Cargo.lock`.
- A real MQTT broker already reachable before the test starts. Either a
  native Mosquitto install or a container both work equally well -- Docker
  is **not** required, only a broker that is actually listening:
  - Docker: `docker run --rm -d -p 1883:1883 --name datum-it-mosquitto eclipse-mosquitto`
  - Native Mosquitto (Windows, Linux, macOS): start the service/daemon so it
    listens on `127.0.0.1:1883` (or whatever host/port you choose).
- `DATUM_TEST_MQTT_BROKER_URI`, set to that broker's `mqtt://host:port`. If
  unset, every test in the file prints a clear prerequisite message and
  returns immediately, before spawning anything.
- `DATUM_PROOF_PYTHON`, set to a real Python 3 interpreter, only if you are
  also running the separate Phase-112 hard gate
  (`dcall_two_node_integration.rs`) in the same session -- it works around a
  Windows Python-launcher/WindowsApps-alias subprocess-spawn bug that would
  otherwise make that unrelated test spuriously fail. It is not required by
  `dcall_mqtt_governed_e2e.rs` itself.

## Exact command (Windows / PowerShell)

```powershell
$env:DATUM_TEST_MQTT_BROKER_URI = "mqtt://127.0.0.1:1883"
Set-Location 'C:\Users\dnred\OneDrive\projetos\datum-phase113b\DATUM'
cargo test --locked --features dcall-test-support --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1
```

A broker must already be listening on `127.0.0.1:1883` (or whatever address
you substitute into `DATUM_TEST_MQTT_BROKER_URI`) before this command is
run -- the test does not start one.

## Exact command (Linux / macOS / bash)

```bash
export DATUM_TEST_MQTT_BROKER_URI="mqtt://127.0.0.1:1883"
cd DATUM
cargo test --locked --features dcall-test-support --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1
```

## What each test proves and its known limits

- `governed_mqtt_dcall_delivery_end_to_end`: the primary success proof.
- `real_broker_stale_runtime_binding_after_legitimate_change_fails_closed`:
  proves the redemption-side stale-binding protection for real, over a real
  broker and a real destination listener subprocess.
- `real_broker_unavailable_broker_fails_closed_without_execution`: an
  unreachable `mqtt://` broker produces a bounded, deterministic failure,
  never a false success and never a local/HTTP fallback.
- `real_broker_listener_absent_source_times_out_without_execution`: the
  broker is reachable and healthy, but no destination listener is running --
  the source publishes successfully and then times out waiting for a reply
  that never arrives, distinguishing this from broker-unavailable.
- `real_broker_duplicate_mqtt_delivery_executes_destination_exactly_once`:
  the exact same governed request delivered twice over the real broker
  executes the destination exactly once; the single-use grant is the only
  dedupe authority, and no parallel dedupe state exists anywhere.

None of these scenarios implement, simulate, or claim to close DMonitor,
placement migration, automatic delivery retry, or Stage-115 reconciliation.
Reply loss remains an acknowledged, undecided limitation: if a destination
redeems and executes but its MQTT response is lost, the source will report a
timeout/failure even though execution genuinely happened, and nothing here
automatically republishes the request. Broader recovery and evidence
reconciliation across that uncertainty belongs to a later phase.

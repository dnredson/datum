# O1 — SmartSentinel continuous service loop runbook

Status: implementation candidate; acceptance still requires the three-host 10-minute laboratory run.

Baseline preserved: Controlled WASM Deployment `150/150 — CLOSED`.

Branch: `agent/o1-smartsentinel-service-loop`.

## What O1 adds

O1 introduces a dedicated `smartsentinel` binary without changing the historical `datum run` CLI path.

The continuous SmartSentinel process:

- loads the existing DATUM configuration;
- resolves `node_id`, hostname, cycle interval and existing file/HTTP outputs;
- runs collection cycles continuously;
- emits a monotonically increasing heartbeat sequence;
- annotates the emitted host snapshot with SmartSentinel service-mode heartbeat evidence;
- survives collection or publication errors and continues to the next cycle;
- explicitly disables `auto_recovery` and `desired_state` in the in-memory service-mode configuration so this checkpoint is observation-only;
- supports deterministic graceful shutdown through a cross-platform stop file;
- supports `--max-cycles` for short smoke tests.

The heartbeat annotation is intentionally small and is not the O2 telemetry envelope. O2 will introduce the typed host telemetry contract.

## Build and smoke test

From `DATUM/`:

```text
cargo test
cargo build --bin smartsentinel
```

Short deterministic smoke test:

```text
cargo run --bin smartsentinel -- --config config.toml --cycle-s 1 --max-cycles 3
```

Expected log shape:

```text
mode=smartsentinel event=start node_id=... cycle_s=1 ... mutations=disabled
mode=smartsentinel event=heartbeat seq=1 status=ok ... side_effects=false
mode=smartsentinel event=heartbeat seq=2 status=ok ... side_effects=false
mode=smartsentinel event=heartbeat seq=3 status=ok ... side_effects=false
mode=smartsentinel event=shutdown reason=max_cycles completed_cycles=3 ... side_effects=false
```

The generated host entity contains an additional `smartsentinel` object:

```json
{
  "mode": "continuous",
  "heartbeat_sequence": 3,
  "service_started_at_utc": "...",
  "service_uptime_ms": 2000,
  "safe_to_auto_execute": false,
  "side_effects_performed": false
}
```

## Graceful shutdown

Default stop-file path:

```text
.smartsentinel-stop
```

Start normally:

```text
cargo run --bin smartsentinel -- --config config.toml
```

From another shell, request shutdown.

PowerShell:

```powershell
New-Item .smartsentinel-stop -ItemType File
```

Linux shell:

```bash
touch .smartsentinel-stop
```

The running process polls the stop file during the inter-cycle wait, removes it after acknowledging the request, and emits:

```text
mode=smartsentinel event=shutdown reason=stop_file ... side_effects=false
```

A stale stop file present before startup is treated as an error rather than silently starting and stopping.

## Three-host O1 acceptance run

Use the existing host-specific configuration on each node and start the dedicated binary.

`local-01`:

```text
cargo run --release --bin smartsentinel -- --config <local-config>
```

`fog-01`:

```text
cargo run --release --bin smartsentinel -- --config <fog-config>
```

`cloud-01`:

```text
cargo run --release --bin smartsentinel -- --config <cloud-config>
```

Keep all three processes running continuously for at least 10 minutes.

Acceptance evidence should record, for each host:

```text
start log
first heartbeat sequence
heartbeat after >= 10 minutes
no process exit/restart
HTTP/file output status across the run
graceful shutdown log
```

The O1 checkpoint is accepted only when all three current hosts complete that run with stable heartbeats.

## Safety boundary

The SmartSentinel service loop changes only an in-memory copy of configuration to disable mutation paths. It does not rewrite `config.toml`, the closed `150/150` artifacts, automatic deployment flags, or the existing `datum run` behavior.

O1 must not be used to claim real host telemetry completion: existing snapshot collectors are reused only to exercise a live continuous cycle. Typed CPU/RAM/uptime/process telemetry, cross-platform normalization and its explicit schema belong to O2.

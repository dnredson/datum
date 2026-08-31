# O1 — SmartSentinel continuous service loop runbook

Status: implementation candidate; acceptance still requires the three-host 10-minute laboratory run.

Baseline preserved: Controlled WASM Deployment `150/150 — CLOSED`.

Branch: `agent/o1-smartsentinel-service-loop`.

## What O1 adds

O1 introduces a dedicated `smartsentinel` binary without changing the historical `datum run` CLI path.

The continuous SmartSentinel process:

- loads the existing DATUM configuration;
- resolves `node_id`, hostname, collection interval and existing file/HTTP outputs;
- emits an independent lifecycle/liveness heartbeat, default 1 s;
- runs the existing collection/plugin cycle independently from heartbeat timing;
- annotates emitted host snapshots with the latest SmartSentinel heartbeat evidence;
- survives collection or publication errors and continues running;
- explicitly disables `auto_recovery` and `desired_state` in the in-memory service-mode configuration so this checkpoint is observation-only;
- supports graceful shutdown through Ctrl+C and a cross-platform stop file;
- supports `--max-cycles` for short collection smoke tests.

A slow collector must not make the Sentinel itself appear dead. `event=liveness` means the SmartSentinel process is alive; `event=collection` means an observation cycle finished. These are intentionally distinct states.

The heartbeat annotation is intentionally small and is not the O2 telemetry envelope. O2 will introduce the typed host/discovery telemetry contract, including OS/runtime/capability discovery.

## Build and smoke test

From `DATUM/`:

```text
cargo test
cargo build --bin smartsentinel
```

Short smoke test that deliberately allows a collection to be slower than heartbeat:

```text
cargo run --bin smartsentinel -- --config config.toml --node-id local-01 --cycle-s 1 --heartbeat-s 1 --max-cycles 2
```

Expected log shape:

```text
mode=smartsentinel event=start node_id=local-01 cycle_s=1 heartbeat_s=1 ... mutations=disabled
mode=smartsentinel event=liveness seq=1 node_id=local-01 status=alive ...
mode=smartsentinel event=liveness seq=2 node_id=local-01 status=alive ...
...
mode=smartsentinel event=collection seq=1 status=ok ... heartbeat_seq=N ... side_effects=false
...
mode=smartsentinel event=collection seq=2 status=ok ... heartbeat_seq=M ... side_effects=false
mode=smartsentinel event=shutdown reason=max_cycles completed_collections=2 heartbeat_seq=M ... side_effects=false
```

If Windows ping/CIM/Docker inspection makes a collection take several seconds, multiple `event=liveness` entries should still appear while that collection is running.

The generated host entity contains an additional `smartsentinel` object:

```json
{
  "mode": "continuous",
  "heartbeat_sequence": 7,
  "service_started_at_utc": "...",
  "service_uptime_ms": 6000,
  "safe_to_auto_execute": false,
  "side_effects_performed": false
}
```

## Platform plugins

The SmartSentinel orchestration layer remains cross-platform. Existing collectors continue to select themselves by operating system.

Examples:

```text
Windows:
  host_telemetry_windows
  process_windows_tasklist
  docker_windows_cli
  connection_ping_windows

Linux:
  host_telemetry_linux
  process_linux_ps
  docker_linux_cli
  connection_ping_linux
```

An unavailable optional runtime such as Docker is observation evidence, not a process-fatal error. During O3 this will be promoted from best-effort/skipped logging into typed runtime availability state.

## Graceful shutdown

### Ctrl+C

Start normally and press Ctrl+C. The process should request shutdown rather than terminate with the Windows `STATUS_CONTROL_C_EXIT` default behavior.

Expected final log:

```text
mode=smartsentinel event=shutdown reason=ctrl_c ... side_effects=false
```

A collector already in progress may finish or return early before the main service loop exits.

### Stop file

Default stop-file path:

```text
.smartsentinel-stop
```

PowerShell:

```powershell
New-Item .smartsentinel-stop -ItemType File
```

Linux shell:

```bash
touch .smartsentinel-stop
```

The heartbeat thread also watches this file, so the shutdown request is detected even while the collection thread is busy. After shutdown the file is removed.

Expected final log:

```text
mode=smartsentinel event=shutdown reason=stop_file ... side_effects=false
```

A stale stop file present before startup is treated as an error rather than silently starting and stopping.

## Local Windows validation evidence — 2026-08-31

Host identity used for smoke validation: `local-01`.

Independent liveness was validated with `--cycle-s 1 --heartbeat-s 1 --max-cycles 1`. The Windows collection took `11780 ms`, while lifecycle heartbeats continued independently from sequence 1 through 12 at approximately one-second intervals. The collection completed successfully with `entities=5`, `edges=3`, `events=1`, `actions=0`, `file_json=ok`, and the expected non-fatal `http_json=err(send snapshot to server)` because the dserver HTTP endpoint was not running for this smoke. Docker Desktop was unavailable and `docker_windows_cli` reported `status=skipped` without failing the cycle.

Ctrl+C shutdown was then validated by running the binary directly with `--cycle-s 5 --heartbeat-s 1`. The collection completed successfully after `10288 ms`, lifecycle heartbeat sequence reached 11, and shutdown ended cleanly with:

```text
mode=smartsentinel event=shutdown reason=ctrl_c completed_collections=1 heartbeat_seq=11 ... side_effects=false
```

The previous Windows `STATUS_CONTROL_C_EXIT` behavior was not observed.

Stop-file shutdown was also validated while a slow Windows collection was in progress. Lifecycle heartbeat sequence reached 7, the collection completed successfully after `10699 ms`, and the process ended cleanly with:

```text
mode=smartsentinel event=shutdown reason=stop_file completed_collections=1 heartbeat_seq=7 ... side_effects=false
```

After shutdown, `Test-Path .smartsentinel-stop` returned `False`, confirming the acknowledged stop request was consumed and removed. A separate stale-stop-file startup test then correctly refused to launch and returned `SmartSentinel stop file already exists: .smartsentinel-stop. Remove it before starting the agent.`

Local Windows evidence therefore validates independent lifecycle heartbeat, Ctrl+C graceful shutdown, active stop-file shutdown, and stale stop-file startup protection.

## fog-01 Linux smoke validation evidence — 2026-08-31

Host identity used for Linux smoke validation: `fog-01`.

The agent was run with `config.linux`, `--cycle-s 1`, `--heartbeat-s 1`, and `--max-cycles 1`. Lifecycle liveness advanced from sequence 1 through 7 while the collection was in progress. The Linux collection completed successfully after `6124 ms` with `entities=6`, `edges=3`, `events=2`, `actions=0`, `file_json=ok`, `http_json=disabled`, and `side_effects=false`. Shutdown then completed cleanly with `reason=max_cycles`.

This confirms that the same SmartSentinel lifecycle implementation runs on Linux while preserving heartbeat independence from collection duration. Ctrl+C shutdown and direct inspection of the Linux host snapshot remain to be checked on `fog-01` before the long acceptance run.

## Three-host O1 acceptance run

Use the existing host-specific configuration on each node and start the dedicated binary.

```text
local-01: cargo run --release --bin smartsentinel -- --config <local-config> --heartbeat-s 1
fog-01:   cargo run --release --bin smartsentinel -- --config <fog-config> --heartbeat-s 1
cloud-01: cargo run --release --bin smartsentinel -- --config <cloud-config> --heartbeat-s 1
```

Keep all three processes running continuously for at least 10 minutes.

Acceptance evidence should record, for each host:

```text
start log
first liveness sequence
liveness after >= 10 minutes
collection cycles continuing independently
no process exit/restart
HTTP/file output status across the run
graceful shutdown log
```

The O1 checkpoint is accepted only when all three current hosts complete that run with stable lifecycle heartbeats.

## Safety boundary

The SmartSentinel service loop changes only an in-memory copy of configuration to disable mutation paths. It does not rewrite `config.toml`, the closed `150/150` artifacts, automatic deployment flags, or the existing `datum run` behavior.

O1 must not be used to claim real host telemetry completion. Existing snapshot collectors are reused only to exercise a live continuous cycle. Typed OS, CPU/RAM/uptime/process/runtime capability discovery belongs to O2 and will support both known-node and previously-unregistered-node bootstrap paths.

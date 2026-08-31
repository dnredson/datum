# O1 — SmartSentinel continuous service loop — FINAL

Date: 2026-08-31

Status: `CLOSED / ACCEPTED`

Baseline preserved: Controlled WASM Deployment `150/150 — CLOSED`.

Branch validated: `agent/o1-smartsentinel-service-loop`.

## Scope closed by O1

O1 establishes a dedicated long-running `smartsentinel` binary while preserving the historical `datum run` path. The SmartSentinel service loop:

- emits lifecycle/liveness heartbeat independently from slow collection;
- continuously reuses the existing cross-platform collectors;
- preserves observation-only behavior with mutation paths disabled in-memory;
- survives best-effort collector/publication failures;
- supports graceful Ctrl+C and cross-platform stop-file shutdown;
- refuses startup when a stale stop file is present;
- uses node-scoped host identity so different nodes sharing an OS hostname do not collapse into one graph entity;
- enforces one SmartSentinel instance per logical `node_id` through an OS-backed exclusive lock;
- releases the instance lock automatically when the owning process exits.

Typed host/capability discovery and the formal telemetry envelope remain O2 work. The existing collectors are reused in O1 only to validate a real continuous agent lifecycle.

## Host identity correction

The laboratory exposed that `fog-01` and `cloud-01` both reported OS hostname `Xubuntu`. A hostname-derived host ID would therefore collide.

SmartSentinel service mode now uses logical node identity for the host entity while retaining the actual hostname as observed metadata:

```text
local-01 -> host:local-01  hostname=Luffy
fog-01   -> host:fog-01    hostname=Xubuntu
cloud-01 -> host:cloud-01  hostname=Xubuntu
```

Historical `datum run` behavior remains hostname-based for compatibility with the frozen baseline.

## Windows validation

Host: `local-01`.

Earlier smoke evidence validated independent lifecycle heartbeat during a slow Windows collection, graceful Ctrl+C, stop-file shutdown, stale-stop-file startup protection, non-fatal Docker unavailability, and non-fatal HTTP publication failure while dserver was not running.

Three-host acceptance run evidence:

```text
liveness=805
collections=52
shutdown reason=stop_file
completed_collections=52
heartbeat_seq=805
uptime_ms=807371
side_effects=false
stop file removed: yes
final host id: host:local-01
hostname: Luffy
node_id: local-01
```

This is approximately 13 minutes 27 seconds of continuous runtime.

Single-instance validation on Windows also passed. While one `local-01` SmartSentinel was active, a second process was rejected with an OS lock error:

```text
SmartSentinel instance already running for node_id=local-01
lock=...\smartsentinel-local-01.lock
os error 33
```

After the owning process ended, a new SmartSentinel instance could start normally, confirming that the file may remain on disk while the lock itself is owned and released by the OS.

## fog-01 Linux validation

Host: `fog-01`, Ubuntu 25.10, x86_64, 4 logical CPUs, approximately 8 GB RAM.

Earlier smoke evidence validated real Linux host collection, independent liveness and graceful Ctrl+C.

Three-host acceptance run evidence for the explicitly logged process:

```text
liveness=1088
collections=99
shutdown reason=stop_file
completed_collections=99
heartbeat_seq=1088
uptime_ms=1090479
side_effects=false
stop file removed: yes
final host id: host:fog-01
hostname: Xubuntu
node_id: fog-01
```

This is approximately 18 minutes 10 seconds of continuous runtime.

During analysis of that run, `snapshot.json` showed a later heartbeat than the process log. Timestamp comparison proved that a second SmartSentinel instance had been running concurrently from another earlier start and had overwritten the same output file. This exposed a real single-instance requirement rather than a heartbeat defect.

The resulting single-instance protection was then validated on Linux. While one `fog-01` process was active, a second was rejected:

```text
Error: SmartSentinel instance already running for node_id=fog-01
(lock=/tmp/smartsentinel-fog-01.lock): Resource temporarily unavailable (os error 11)
```

After the first process exited, a new process successfully acquired the same lock and completed a one-cycle smoke:

```text
event=start node_id=fog-01
lock_file=/tmp/smartsentinel-fog-01.lock
event=collection seq=1 status=ok
heartbeat_seq=7
side_effects=false
event=shutdown reason=max_cycles completed_collections=1
```

This confirms both rejection of concurrent ownership and automatic lock release after process termination.

## cloud-01 Linux validation

Host: `cloud-01`, Ubuntu 25.10, x86_64, 2 logical CPUs, approximately 4 GB RAM.

Smoke validation confirmed real Linux collection and node-scoped identity `host:cloud-01`. Ctrl+C was exercised while a second collection was in progress and ended through the controlled shutdown path.

Three-host acceptance run evidence:

```text
liveness=809
collections=74
shutdown reason=stop_file
completed_collections=74
heartbeat_seq=809
uptime_ms=813429
side_effects=false
stop file removed: yes
final host id: host:cloud-01
hostname: Xubuntu
node_id: cloud-01
```

This is approximately 13 minutes 33 seconds of continuous runtime.

## O1 acceptance decision

O1 is accepted because all required lifecycle properties were demonstrated across Windows and Linux:

```text
PASS independent heartbeat from collection duration
PASS continuous collection loop
PASS Windows execution
PASS Linux execution
PASS graceful Ctrl+C
PASS graceful stop-file shutdown
PASS stale-stop-file startup protection
PASS observation-only side_effects=false boundary
PASS non-fatal optional collector/publication failures
PASS node-scoped host identity without hostname collision
PASS three-host concurrent run >= 10 minutes
PASS single-instance protection on Linux
PASS single-instance protection on Windows
PASS lock release and restart after prior owner exits
```

The three-host long run does not need to be repeated solely for the single-instance fix because the lock is acquired only during process startup and does not alter the already validated steady-state heartbeat/collection loop after acquisition.

## Deferred service installation note

Running SmartSentinel as an operating-system service is desirable, but it is not required to close O1. A future governed WASM action may be used to install/register SmartSentinel as a service on the host rather than embedding OS service installation directly into the O1 lifecycle implementation.

## Next checkpoint

Proceed to O2: typed host and capability discovery.

O2 should formalize both bootstrap scenarios:

1. known node: SmartSentinel starts for an entity already known by dserver and retrieves expected installation/runtime/capability state;
2. unknown node: SmartSentinel discovers OS, resources, runtimes and tools, publishes a registration artifact/evidence record, and allows that node to become eligible for later D-Graph construction.

The first O2 contract should include at least OS/distribution/architecture, CPU, RAM, swap, filesystem, host uptime, process evidence, Python availability, Docker/container runtime availability, relevant runtime/tools/capabilities, source timestamp and stable node identity.

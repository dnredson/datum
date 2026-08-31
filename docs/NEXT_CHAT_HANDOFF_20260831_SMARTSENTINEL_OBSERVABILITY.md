# Next-chat handoff — SmartSentinel live observability

Date: 2026-08-31

## Repository state

- Repository: `dnredson/datum`
- `main`: `7e3e4922881f52f7f79d90ab2ebb75ba009393c0`
- Controlled WASM Deployment phase: `150/150 — CLOSED`
- Final checkpoint: `I44`
- Open pull requests after closure: none
- Local desktop `main` was synchronized and clean after the final merge.

Regression/evidence carried into the next phase:

```text
dserver validated baseline: 520 passed; 0 failed
DATUM final rerun:           431 passed; 0 failed
```

Final three-node I44 campaign:

```text
a-fog-scale        fog-01   result=24
m-local-aggregate  local-01 result=30
z-cloud-delta      cloud-01 result=20
```

Controlled state persisted source D-Graph, active manifest and three accepted live reports across dserver restart. Sentinel registrations and lifecycle attempts intentionally remained in memory and were reconstituted without artifact re-execution. Automatic governed WASM deployment remained unavailable.

## Current laboratory topology

```text
Desktop / local-01 — 192.168.10.101
  dserver
  local DATUM/SmartSentinel binary

VM1 / fog-01 — 192.168.10.103
  DATUM/SmartSentinel

VM2 / cloud-01 — 192.168.10.108
  DATUM/SmartSentinel
```

A second fog target (`fog-02`) should be added during the new phase for real failover/recovery experiments. It can initially be another VM and later be replaced or complemented by a Raspberry Pi or other physical fog node.

## Architectural boundary to preserve

- SmartSentinel is the distributed agent. Deployment is only one of its responsibilities.
- dserver is the central control-plane/orchestrator.
- The D-Graph is generated externally; dserver consumes, validates, enriches and interprets it.
- SmartSentinel should observe the host, runtime, services and network, publish evidence, execute explicitly authorized actions and report outcomes.
- The closed Controlled WASM phase remains a frozen baseline. The next phase must not rewrite its historical claims.

## What already exists and what is missing

The current DATUM local D-Graph builder is mostly configuration-derived. It represents configured runtimes, modules, flows and capabilities, but local health is `unknown` when no persisted observation exists. The current CLI explicitly treats that snapshot path as reporting/config metadata rather than a live Docker/system inspection path.

The dserver already has a resource-observation join contract that understands:

```text
CPU logical cores and usage percent
memory total and available bytes
process count
container count
supported capabilities
runtime/module/health counts
fresh/stale age
```

I42/I43/I44 proved that this evidence can be materialized into the observed D-Graph and used by readiness logic. In the I44 laboratory campaign, however, the resource observations were published by a helper script. The next phase must replace this injected evidence with telemetry collected and transmitted by the SmartSentinels themselves.

## User goal for the next phase

Make the SmartSentinels visibly run as continuous agents that observe the IoTinuum and transmit real telemetry to dserver.

At minimum each SmartSentinel should collect and publish:

```text
identity / last-seen / uptime
CPU usage and logical cores
RAM total / used / available
swap
filesystem capacity/free space
process count and selected process/service state
container runtime availability and container state/count
network interfaces, addresses and link state
RX/TX counters and errors/drops when available
control-plane reachability
peer-to-peer reachability between IoTinuum hosts
peer latency / RTT and recent loss/failure evidence
configured capabilities and actual runtime capabilities
service/module health
```

This evidence must become current state in the observed D-Graph rather than merely a separate metrics page.

## Target demonstration

The phase should culminate in a live demonstration with an actual device/workload path.

Baseline:

```text
device/sensor -> fog-01 -> downstream path
```

Failure experiment:

```text
1. fog-01 stops or becomes unreachable
2. SmartSentinels/dserver detect loss from fresh telemetry and peer observations
3. D-Graph marks fog-01 and affected links/flows unavailable or degraded
4. an alternative path through fog-02 is computed
5. controlled reroute is applied to the real test data path
6. dashboard shows the new active path and the failure reason
```

Recovery experiment:

```text
1. fog-01 returns
2. telemetry becomes healthy/fresh for a stabilization interval
3. D-Graph reflects recovery
4. policy proposes or performs a controlled return to the preferred/original path
5. dashboard shows the restored path and adaptation history
```

Do not claim “routing” if only metadata changes. The validation must include traffic or workload evidence proving that the active data path really changed.

## Dashboard goal

Add a deliberately small dashboard served with or beside dserver. It is an observability/debugging surface, not a product UI.

Minimum view:

```text
D-Graph nodes and edges
node state: healthy / degraded / unavailable / stale
CPU and RAM on each compute node
Sentinel last-seen / telemetry age
link reachability and RTT
current active device/service path
preferred/original path versus current path
current placement/flow binding
recent adaptation/failure event
```

A node click should expose the raw latest telemetry/evidence. Polling or SSE is enough initially; WebSocket infrastructure is not required for the first version.

## Recommended next-phase name

`SmartSentinel Live Observability & Adaptive IoTinuum`

Use a new checkpoint namespace (`O1` ... `O12`) rather than extending the closed `150/150` scale.

## First actions in the next chat

1. Create the new implementation branch from clean `main`.
2. Freeze a `SmartSentinelTelemetryEnvelope` schema and config for collection/publish/probe intervals.
3. Implement SmartSentinel long-running service/loop mode with heartbeat and graceful shutdown.
4. Collect real host CPU/RAM/uptime/process data on Windows/Linux.
5. Publish the first real telemetry from `local-01`, `fog-01`, and `cloud-01` to dserver.
6. Materialize it into the observed D-Graph and show it with a minimal dashboard before implementing failover.

## Safety / research constraints carried forward

- Keep historical Controlled WASM automatic execution flags unchanged.
- Separate observation from mutation.
- Use explicit schemas, freshness and source timestamps.
- Fail closed on stale or contradictory evidence.
- Introduce hysteresis/cooldown before any automatic adaptation.
- Record every adaptation decision, reason, previous path, new path and rollback/recovery result.
- First validate what-if and explicitly applied failover before enabling any bounded automatic routing policy.

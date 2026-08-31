# SmartSentinel Live Observability & Adaptive IoTinuum — Phase Plan

Status: proposed
Checkpoint namespace: `O1` ... `O12`
Starting baseline: Controlled WASM Deployment `150/150 — CLOSED`

## Phase objective

Turn the SmartSentinel from a primarily command-driven deployment/reporting client into a visibly running distributed observability agent for the IoTinuum, while keeping deployment as only one of its responsibilities.

The phase should produce a live, continuously changing D-Graph whose node/link/service state is derived from real SmartSentinel telemetry and that can support reproducible failover/recovery experiments.

The end-state demonstration is:

```text
real SmartSentinels running on multiple IoTinuum hosts
        |
        +--> host/runtime/service telemetry
        +--> peer connectivity + latency telemetry
        +--> heartbeat / freshness
        |
        v
      dserver
        |
        +--> telemetry state/history
        +--> observed D-Graph enrichment
        +--> failure/recovery detection
        +--> alternative-path reasoning
        |
        +--> minimal live dashboard
        |
        v
controlled data-path adaptation
primary fog -> backup fog -> recovered primary fog
```

## Why this phase comes next

The closed phase already proved controlled deployment, lifecycle evidence, resource-readiness joins and restart boundaries. The missing piece is that resource evidence is not yet produced continuously by the actual SmartSentinel process.

The current DATUM D-Graph builder is configuration-centric and reports health as unknown when runtime observations are absent. The dserver already understands a resource observation containing CPU, memory, process/container counts, capabilities and freshness. This phase connects those two sides with real collection and continuous publication.

## Design principles

1. **SmartSentinel is an agent, not a deploy command.** It runs continuously and observes the host even when no deployment is occurring.
2. **Telemetry is evidence.** Every observation carries source time, receive time, age and freshness.
3. **Graph state is derived from evidence.** Dashboard state must come from the same observed D-Graph/control-plane data used by adaptation logic.
4. **Observation and mutation stay separate.** Collection never silently reconfigures the system.
5. **No single bad sample triggers failover.** Failure/recovery use consecutive observations, hysteresis and cooldown.
6. **Routing claims require actual traffic evidence.** A graph-only edge change is not sufficient to claim rerouting.
7. **The 150/150 WASM phase remains frozen.** New automatic behavior, if eventually introduced, belongs to this new phase and is separately gated/audited.

## Proposed telemetry model

Introduce a versioned `SmartSentinelTelemetryEnvelope` with approximately this shape:

```json
{
  "schema_version": "0.1.0",
  "project_id": "lab-project",
  "node_id": "fog-01",
  "sentinel_instance_id": "fog-01-sentinel",
  "sequence": 1234,
  "observed_at_utc": "...",
  "host": {
    "hostname": "...",
    "os": "linux",
    "kernel": "...",
    "uptime_seconds": 12345,
    "cpu": {
      "logical_cores": 4,
      "usage_percent": 37.2,
      "load_1": 0.8,
      "load_5": 0.7,
      "load_15": 0.6
    },
    "memory": {
      "total_bytes": 8589934592,
      "used_bytes": 3221225472,
      "available_bytes": 5368709120
    },
    "swap": {
      "total_bytes": 2147483648,
      "used_bytes": 0
    },
    "process_count": 132,
    "filesystems": []
  },
  "runtime": {
    "container_runtime_available": true,
    "container_count": 3,
    "containers": [],
    "services": []
  },
  "network": {
    "interfaces": [],
    "control_plane": {
      "reachable": true,
      "rtt_ms": 1.8
    },
    "peers": [
      {
        "node_id": "local-01",
        "reachable": true,
        "rtt_ms": 0.9,
        "failure_count": 0
      }
    ]
  },
  "capabilities": [],
  "safe_to_auto_execute": false,
  "side_effects_performed": false
}
```

The final schema can evolve, but `node_id`, sequence, timestamps, CPU/RAM, runtime/service state, interfaces and peer observations should be first-class typed fields rather than an unstructured metadata blob.

## Collection cadence and freshness

Initial laboratory defaults:

```text
host collection interval:        5 s
telemetry publish interval:      5 s
peer probe interval:             5 s
UI refresh/poll:                 2 s
observability fresh threshold:  15 s
```

The new 15-second observability freshness is separate from the existing 300-second Controlled WASM resource-readiness threshold. Do not silently change the historical I42/I43/I44 contract.

Suggested failure state machine:

```text
healthy
  -> suspect      after 1 failed/stale interval
  -> unavailable  after 3 consecutive failed intervals

unavailable
  -> recovering   after first successful observation
  -> healthy      after 3 consecutive successful observations
```

Add a configurable stabilization/cooldown before returning traffic to a recovered preferred node; initial lab target: 30 seconds.

## Host metrics — minimum

Collect real values on Windows and Linux:

```text
hostname / OS / kernel
agent uptime / host uptime
CPU logical cores
CPU usage percent
load average where supported
RAM total / used / available
swap total / used
filesystem total/free
process count
selected configured process state
```

Use a Rust system-information library where it keeps the implementation cross-platform and testable. Avoid shell parsing as the primary collector when a library API exists.

## Runtime/service metrics — minimum

```text
container runtime available/unavailable
container count
configured container present/running/stopped
configured process/service present/running/missing
module health derived from actual runtime evidence
capabilities observed versus merely configured
```

The existing reporting-only D-Graph builder should remain available. Add live observations as a distinct source rather than redefining config metadata as runtime truth.

## Network/connectivity telemetry

### Interface evidence

For each relevant interface:

```text
name
up/down if available
addresses
RX/TX bytes
RX/TX errors/drops when available
```

### Peer evidence

Each SmartSentinel should have configured peer probe targets. The preferred first implementation is an agent-owned lightweight probe endpoint/listener so RTT is measured against the SmartSentinel host itself instead of an unrelated SSH/container service.

Per peer:

```text
peer node_id
reachable
latest RTT
short rolling min/mean/max or p50/p95 when enough samples exist
consecutive failures
last success
observation age
```

Active bandwidth tests are out of scope initially because they perturb the network. Passive counters + lightweight probes are enough for the first placement/failover experiments.

## dserver telemetry ingestion

Proposed API family:

```text
POST /api/v1/sentinels/{node_id}/telemetry
GET  /api/v1/sentinels/{node_id}/telemetry
GET  /api/v1/sentinels/telemetry
GET  /api/v1/sentinels/{node_id}/telemetry/history
```

The first implementation may retain a bounded in-memory history plus latest observation. Persistence can be added later after the live path is stable.

Validation must reject:

```text
node identity mismatch
non-monotonic or replayed sequence when relevant
invalid ranges
future timestamps beyond tolerated skew
oversized payloads
unknown schema versions
```

## D-Graph materialization

Enrich compute nodes with current telemetry:

```text
node.metadata.telemetry
node health/freshness
CPU/RAM/runtime/service state
last_seen / telemetry_age
```

Materialize measured network relationships as observed connectivity edges, for example:

```text
fog-01 --connectivity--> local-01
  reachable=true
  rtt_ms=0.9
  freshness=fresh
```

Keep desired/configured topology distinct from observed connectivity. A configured edge can exist while the observed edge is unavailable.

For flows, retain both:

```text
preferred/original path
current active path
```

This is necessary to reason about recovery back to the preferred path.

## Minimal dashboard

Implement the smallest useful UI before spending effort on frontend architecture.

Recommended first version:

```text
dserver serves static HTML/CSS/JS
browser consumes a dashboard-state endpoint
poll every ~2 s initially; SSE can replace polling later
SVG or a small graph library for topology rendering
```

Minimum screen:

- graph topology;
- compute node health color;
- CPU/RAM summary on nodes;
- SmartSentinel last-seen/freshness;
- measured peer RTT on connectivity edges;
- service/module state;
- preferred path and current path highlighted differently;
- latest adaptation/failure event;
- side panel with raw latest telemetry for the selected node.

Suggested visual semantics:

```text
green  = healthy/fresh
amber  = suspect/degraded/stale-near-threshold
red    = unavailable
 gray   = unknown/no evidence
```

Do not build authentication, complex layouts or historical charting before the live graph works.

## Failover/recovery experiment topology

Extend the lab to:

```text
local-01   192.168.10.101  control-plane / local stage
fog-01     192.168.10.103  preferred fog
fog-02     TBD             backup fog
cloud-01   192.168.10.108  cloud
sensor-01  simulator or physical device
```

A second fog VM is sufficient initially. Physical Raspberry Pi validation can follow without blocking the phase.

## Real data-path experiment

Use an actual small workload so rerouting is measurable. A sensor simulator should emit monotonically increasing sequence numbers at a fixed cadence through a defined routing/forwarding component.

Baseline:

```text
sensor-01 -> fog-01 -> downstream sink
```

Failure:

```text
fog-01 becomes unavailable
-> graph marks node/link state
-> alternative fog-02 passes capability/resource/connectivity gates
-> planner creates adaptation proposal
-> controlled apply changes the real forwarding target
-> sink continues receiving sensor sequence via fog-02
```

Recovery:

```text
fog-01 becomes healthy
-> stabilization interval passes
-> preferred path becomes eligible
-> revert proposal/application restores sensor-01 -> fog-01
```

Record before/after target, reason, telemetry snapshot IDs, decision timestamp, execution timestamp and outcome.

## Adaptation progression

Do not jump directly from telemetry to automatic mutation.

### Stage A — what-if only

Given an unavailable/degraded fog, return candidate alternate paths with reasons and scores. No side effects.

Initial deterministic candidate constraints:

```text
node fresh and healthy
required capability present
service/runtime compatible
memory/cpu guardrails satisfied
required network links reachable
```

Initial score can use a simple deterministic weighted cost:

```text
network RTT
CPU utilization
available memory
preference/priority penalty
```

This provides an interpretable baseline before adding AI/GNN policies.

### Stage B — explicit controlled apply

Operator/API explicitly applies a selected reroute. Audit every mutation and retain previous path for rollback.

### Stage C — bounded automatic routing experiment

Only after Stage B is reproducibly validated, allow an explicit lab policy such as:

```text
auto_failover_enabled=true
auto_return_to_preferred=true
```

This is a new, separately gated routing-adaptation capability. It must not change the closed phase's claim that canonical automatic governed WASM deployment is unavailable.

## Checkpoints

### O1 — long-running SmartSentinel service mode

- foreground daemon/service loop;
- heartbeat;
- configuration for intervals and dserver;
- graceful shutdown;
- visible logs showing collection/publish cycles;
- no deployment required for the agent to be useful.

Acceptance: three current hosts run SmartSentinel continuously for at least 10 minutes with stable heartbeats.

### O2 — real host telemetry

- CPU/RAM/swap/filesystem/uptime/process collection;
- typed telemetry envelope;
- Windows + Linux coverage;
- unit tests for validation and conversion.

Acceptance: local-01, fog-01 and cloud-01 report distinct real host values.

### O3 — runtime/service/container observation

- actual process/container/runtime inspection;
- map observations to configured modules;
- replace `unknown` with evidence-backed health where possible.

Acceptance: stopping a selected container/process changes SmartSentinel telemetry and module observed state without editing config.

### O4 — network/interface and control-plane telemetry

- interface/address/counters;
- dserver reachability and RTT;
- freshness/last-success evidence.

Acceptance: disconnect or block the control-plane path and observe deterministic state transition.

### O5 — peer connectivity/latency matrix

- lightweight SmartSentinel probe endpoint;
- configured peers;
- RTT/reachability/failure counters;
- rolling link evidence.

Acceptance: dashboard/API shows current local↔fog↔cloud peer RTTs from real probes.

### O6 — dserver telemetry ingestion and bounded history

- versioned ingestion endpoints;
- latest-by-node store;
- bounded recent history;
- timestamp/identity/schema validation;
- telemetry freshness state.

Acceptance: 10+ minutes of three-node publishing without unbounded memory growth or identity cross-talk.

### O7 — telemetry-aware observed D-Graph

- join latest SmartSentinel telemetry into graph nodes;
- materialize measured connectivity edges;
- distinguish desired/configured and observed state;
- expose preferred/current paths.

Acceptance: killing a Sentinel or network link changes graph state solely from real freshness/telemetry evidence.

### O8 — minimal live dashboard

- dserver-hosted minimal UI;
- graph visualization;
- CPU/RAM/node health;
- edge RTT/reachability;
- last-seen;
- selected-node raw evidence;
- active/preferred path display.

Acceptance: user can watch live values change while generating CPU load or stopping a node.

### O9 — second fog and failure/recovery state machine

- add `fog-02`;
- consecutive-failure logic;
- recovery hysteresis;
- cooldown/stabilization;
- audit state transitions.

Acceptance: fog-01 shutdown produces `healthy -> suspect -> unavailable`; restart produces `unavailable -> recovering -> healthy` without flapping.

### O10 — alternative-path what-if planner

- candidate fog filtering from capabilities/resources/connectivity;
- deterministic scoring;
- no mutation;
- reasons and rejected candidates visible.

Acceptance: with fog-01 unavailable, fog-02 is proposed when eligible; it is rejected when telemetry/capability/link evidence violates constraints.

### O11 — controlled real data-path failover

- actual sensor/workload stream;
- explicit apply action;
- flow binding changes to fog-02;
- audit and rollback data;
- dashboard path change.

Acceptance: after fog-01 failure, sensor sequence is received through fog-02; graph/dashboard and actual traffic agree.

### O12 — recovery to preferred path and phase-final reproducibility

- restore fog-01;
- stabilization gate;
- controlled return to preferred path;
- optional bounded automatic failover/return experiment only after explicit path is validated;
- final reproducible runbook and report.

Acceptance sequence:

```text
primary healthy -> traffic on fog-01
fog-01 failure -> detected from real telemetry
backup selection -> fog-02
traffic observed on fog-02
fog-01 recovery -> stabilization
preferred path restored -> fog-01
no false deployment-automation claim
complete event/audit trail
```

## Suggested implementation order inside the codebase

Start with the agent and ingestion path, not the dashboard:

```text
DATUM/SmartSentinel telemetry model
-> system collector
-> long-running agent loop
-> dserver telemetry API/store
-> observed D-Graph join
-> network peer probes
-> dashboard
-> failover planner
-> actual route mutation
```

This makes every later UI/adaptation feature consume the same evidence contract.

## Likely new modules

Names are provisional:

```text
DATUM/src/telemetry/model.rs
DATUM/src/telemetry/system.rs
DATUM/src/telemetry/runtime.rs
DATUM/src/telemetry/network.rs
DATUM/src/telemetry/client.rs
DATUM/src/agent/service_loop.rs

dserver/src/api/sentinel_telemetry.rs
dserver/src/core/sentinel_telemetry.rs
dserver/src/core/telemetry_dgraph_join.rs
dserver/src/core/adaptation_planner.rs

dserver/static/dashboard/*
```

Prefer extending existing snapshot/health structures where semantics match rather than duplicating models merely to match these provisional paths.

## Metrics for the research experiments

Besides functional PASS/FAIL, collect:

```text
telemetry publish interval / jitter
telemetry age at dserver
failure detection time
recovery detection time
adaptation decision time
adaptation apply time
end-to-end failover time
packet/message loss during failover
path restoration time
CPU/RAM overhead of SmartSentinel
telemetry bytes/sec per node
peer RTT distributions
false failover / flap count
```

These make the phase directly useful for papers instead of only producing a demo.

## Phase definition of done

The phase is complete when the user can open one dashboard and visibly observe real SmartSentinels running across the IoTinuum, including CPU/RAM/service/network state and host-to-host latency, then execute a reproducible fog failure/recovery campaign in which the D-Graph state, planner decision, dashboard path and real device traffic all agree.

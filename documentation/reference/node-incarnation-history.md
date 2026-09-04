# Node incarnation and historical telemetry semantics

DATUM distinguishes the identity of a logical node from the identity of the concrete host currently realizing that node.

This distinction is required for long-lived IoTinuum management. A logical node such as `fog-01` can survive hardware replacement, VM recreation, operating-system reinstall, agent restart, or resource upgrades, but those events do not all have the same meaning for telemetry history or AI-assisted management.

## Identity hierarchy

```text
node_id
  stable logical DATUM role/participant

host_incarnation_id
  one concrete host/VM incarnation realizing that node

capacity_profile_id
  structural capacity profile of that incarnation

instance_id
  one concrete SmartSentinel process lifetime
```

### `node_id`

`node_id` is the stable logical identity used by DATUM/D-Graph reasoning. Replacing a failed fog machine does not require inventing a new logical role if the replacement continues to represent `fog-01`.

### `host_incarnation_id`

`host_incarnation_id` identifies one concrete host incarnation. It is derived from a one-way hash of the best available machine anchor, preferring a system/hardware UUID and falling back to an operating-system machine identifier. If neither exists, hostname is only a low-confidence fallback.

Raw hardware/system identifiers are not emitted in the discovery report. The identifier is observational and is explicitly **not authenticated identity attestation**.

A different `host_incarnation_id` for the same `node_id` means the control plane must treat the host realization as changed.

Example:

```text
fog-01 / hostinc:A   -> old physical/virtual machine
fog-01 / hostinc:B   -> replacement machine
```

### `capacity_profile_id`

`capacity_profile_id` fingerprints structural resource capacity rather than instantaneous load. The current profile includes:

- logical CPU count;
- total RAM;
- total swap/pagefile capacity;
- filesystem mount identities and total capacities.

Instantaneous CPU use, used RAM, free RAM, and used filesystem space do not affect this identifier.

Therefore:

```text
same host_incarnation_id + same capacity_profile_id
  same host and same structural capacity

same host_incarnation_id + different capacity_profile_id
  same host incarnation, but capacity changed
  e.g. RAM upgrade, CPU/vCPU change, disk resize

different host_incarnation_id + any capacity_profile_id
  host incarnation changed
  e.g. hardware replacement or VM recreation
```

### `instance_id`

`instance_id` identifies one running SmartSentinel process lifetime. Restarting the agent changes `instance_id` but does not imply that the host or its capacity changed.

## Historical time-series rule

Telemetry must remain queryable by the stable logical `node_id`, but physical-resource history must be segmented by `host_incarnation_id`.

Conceptually:

```text
fog-01 history

hostinc:A
  2026-09-01 ... 2026-10-14
  RAM total = 8 GiB

hostinc:B
  2026-10-14 ...
  RAM total = 32 GiB
```

The logical node history remains continuous because both incarnations represented `fog-01`, while resource baselines are not silently blended across the replacement boundary.

## AI-management semantics

AI/GNN/planner consumers must receive incarnation and capacity-profile context together with telemetry. They must not interpret all samples sharing one `node_id` as homogeneous physical observations.

When `host_incarnation_id` changes, consumers should:

- emit/consume an explicit host-incarnation-change event;
- re-evaluate node capabilities and eligibility;
- reset or segment host-performance baselines;
- avoid comparing absolute RAM/disk/CPU-capacity values as though the physical host were unchanged;
- retain logical workload/history context attached to `node_id` where that context remains meaningful;
- treat previous incarnation-specific anomalies or learned thresholds as historical evidence, not automatically valid current thresholds.

When only `capacity_profile_id` changes, consumers should:

- emit/consume a capacity-profile-change event;
- keep the same host incarnation;
- re-evaluate placement/readiness because resource capacity changed;
- rebaseline capacity-sensitive metrics;
- preserve process/service/workload continuity where appropriate.

When only `instance_id` changes, consumers should:

- interpret the change primarily as agent lifecycle/restart;
- preserve host and resource historical baselines unless independent evidence shows a host or capacity change.

## Comparing telemetry across incarnations

Some metrics remain meaningful across host replacements, while others require normalization or segmentation.

Examples:

```text
CPU utilization percent
  comparable with caution across incarnations

RAM used bytes
  not directly comparable when RAM total changed

RAM utilization percent
  more comparable, but performance interpretation still depends on the incarnation

latency / workload completion time
  logically comparable for the same workload, but host-incarnation context must remain available
```

The system should therefore preserve raw metrics and the identity/capacity dimensions needed for later normalization instead of destroying history by overwriting the old node description.

## Control-plane evolution

O2 introduces the observation-side identifiers. A later dserver ingestion/materialization step should maintain node-scoped incarnation history and derive transitions such as:

```text
node_incarnation_changed
capacity_profile_changed
sentinel_instance_changed
```

The dserver may additionally assign monotonic node-scoped generations/revisions when these transitions are persisted. SmartSentinel should report observed identity evidence; it should not unilaterally declare global historical truth.

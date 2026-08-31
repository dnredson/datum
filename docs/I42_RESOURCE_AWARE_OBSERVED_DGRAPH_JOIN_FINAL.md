# I42 — Resource-Aware Observed D-Graph Join — Final

- checkpoint: `I42/148`
- predecessor: `I41/147`
- status: `validated`
- branch: `agent/i42-resource-aware-observed-join`

## Regression

```text
dserver fmt: PASS
dserver test: 514 passed; 0 failed
dserver build: PASS
```

I42 added six focused resource-observation/join tests on top of the validated I41 baseline.

## Live resource join

The validated controlled D-Graph used project `i42-live-project` and the existing fog/local/cloud placement chain.

Three latest local D-Graph snapshots were published through the existing node snapshot endpoint. Resource evidence was carried in `metadata.resource_observation`, while supported capabilities were taken from the existing `capabilities[]` snapshot entries.

Observed node evidence:

```text
fog-01
  freshness=stale
  age_seconds=629
  cpu_usage_percent=72.5
  logical_cores=4
  available_memory_bytes=2147483648
  process_count=137
  container_count=3
  capabilities=docker,wasm

local-01
  freshness=fresh
  age_seconds=29
  cpu_usage_percent=31.0
  logical_cores=16
  available_memory_bytes=17179869184
  process_count=221
  container_count=5
  capabilities=docker,wasm

cloud-01
  freshness=fresh
  age_seconds=89
  cpu_usage_percent=18.5
  logical_cores=32
  available_memory_bytes=51539607552
  process_count=184
  container_count=8
  capabilities=gpu,wasm
```

Aggregate join evidence matched the planned gate:

```text
observation_count=3
fresh_count=2
stale_count=1
missing_node_ids=[]
unmatched_node_ids=[]
freshness_threshold_seconds=300
automatic_placement_performed=false
deployment_side_effects_performed=false
governed_execution_performed=false
```

## Placement and dependency preservation

Resource evidence did not change placement:

```text
fog-scale-module       -> fog-01
local-aggregate-module -> local-01
cloud-delta-module     -> cloud-01
```

Dependency stages remained:

```text
stage 0 -> a-fog-scale
stage 1 -> m-local-aggregate -> depends_on a-fog-scale
stage 2 -> z-cloud-delta     -> depends_on m-local-aggregate
```

The final resource-aware observed D-Graph retained:

```text
validation_errors=0
report_count=0
```

No artifact delivery or WASM execution was required for this validation.

## Safety and non-claims

I42 is evidence-only. It does not:

- automatically move placements in response to CPU or memory values;
- trigger deployment or retry side effects;
- authorize governed execution;
- claim cryptographic trust in resource observations;
- replace the existing LocalDGraphSnapshot transport/storage path.

Freshness is explicitly represented with a fixed 300-second threshold, so downstream readiness/policy logic can distinguish fresh from stale observations without silently treating all telemetry as current.

## Result

I42/148 is validated. The controlled observed D-Graph now combines deployment structure, dependency stages, lifecycle/runtime evidence, node resource state, and capabilities while preserving the fail-closed execution boundary.

Next: `I43/149 — deployment readiness and governance evidence`.

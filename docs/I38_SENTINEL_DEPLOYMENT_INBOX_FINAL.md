# I38 — Sentinel Deployment Inbox Discovery Final

- checkpoint: `I38/144`
- predecessor: `I37/143`
- validation_date: `2026-08-31`
- status: `validated`
- branch: `agent/i38-sentinel-deployment-inbox`

## Closure

I38 adds read-only deployment inbox discovery for a configuration-bound Sentinel. Identity comes from `agent.node_id`; discovery reads only the existing node-scoped deployment endpoint and never acquires an artifact or executes guest code.

The read-only selector is:

```text
--assignment inbox
```

It remains separate from the I35 execution-capable transition:

```text
--assignment auto --execute
```

## Regression — PASS

```text
cargo fmt --manifest-path DATUM/Cargo.toml -- --check: PASS
cargo test --manifest-path DATUM/Cargo.toml: 426 passed; 0 failed
cargo build --manifest-path DATUM/Cargo.toml: PASS
```

The build completed successfully. DATUM emitted 43 warnings; they were non-blocking for this checkpoint.

## Live no-work gate — PASS

With a fresh disposable controlled state and no D-Graph/manifest published, `local-01` queried its inbox and returned:

```text
mode=sentinel_deployment_inbox
node_id=local-01
status=no_work
manifest_id=null
project_id=null
delivery_scope=null
assignment_count=0
assignment_ids=[]
```

Read-only evidence was:

```text
control_plane_fetch_performed=true
artifact_acquisition_performed=false
wasmtime_compiled=false
guest_execution_performed=false
report_published=false
governed_execution_performed=false
deployment_side_effects_performed=false
```

## Live one-assignment discovery — PASS

After publishing the existing I34 external D-Graph:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
```

three bound Sentinels discovered exactly their own node-scoped work:

```text
local-01 -> status=one_assignment -> [local-aggregate]
fog-01   -> status=one_assignment -> [fog-scale]
cloud-01 -> status=one_assignment -> [cloud-delta]
```

All three results reported `delivery_scope=node_slice` and retained the same read-only negative side-effect evidence.

## Cross-node isolation — PASS

A temporary configuration bound to an unassigned logical node:

```text
node_id=probe-01
```

returned:

```text
status=no_work
delivery_scope=empty
assignment_count=0
assignment_ids=[]
```

No `fog-scale`, `local-aggregate`, or `cloud-delta` assignment leaked into the probe result.

## No-artifact / no-report gate — PASS

The artifact repository on `192.168.10.101:8090` was deliberately unavailable throughout discovery:

```text
curl: (7) Failed to connect to 192.168.10.101:8090
```

Despite that, all inbox discovery calls succeeded.

Controlled live-report count remained unchanged:

```text
before=0
after=0
before == after: True
```

This provides direct live evidence that I38 discovery did not require artifact delivery, Wasmtime compilation, guest execution, or report publication.

## Safety and non-claims

Inbox mode rejects execution authorization and remains read-only with respect to deployment behavior. It does not provide:

- automatic polling;
- automatic deployment execution;
- multi-assignment execution scheduling;
- cryptographic Sentinel identity or host attestation;
- artifact integrity verification during discovery;
- guest execution or runtime report creation.

I38 therefore advances the Controlled WASM Deployment phase to `144/150`. The next checkpoint is `I39/145`, explicit multi-assignment scheduling.
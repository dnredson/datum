# I38 — Sentinel Deployment Inbox Discovery

- checkpoint: `I38/144`
- predecessor: `I37/143`
- status: `implementation_ready_for_validation`
- branch: `agent/i38-sentinel-deployment-inbox`

## Goal

I38 lets a configuration-bound Sentinel ask dserver whether the active controlled deployment currently contains work for its own `agent.node_id` without acquiring an artifact, compiling WASM, invoking Wasmtime, executing guest code, or publishing a runtime report.

The existing `wasm-deploy-live` command gains a separate read-only selector:

```text
--assignment inbox
```

This is intentionally distinct from the I35 execution selector:

```text
--assignment auto --execute
```

Normal live execution still requires `--execute`. Inbox discovery forbids `--execute`.

## Discovery contract

For a control-plane URL, the Sentinel:

```text
load config
  -> bind identity from agent.node_id
  -> validate HTTPS / explicit lab HTTP policy
  -> GET /api/v1/controlled-wasm/deployment/for-node/<agent.node_id>
  -> validate any returned node-scoped deployment for the bound node
  -> classify assignment count
  -> render discovery evidence
```

Classification:

```text
0 assignments -> no_work
1 assignment  -> one_assignment
2+ assignments -> multiple_assignments
```

A `404` from the exact node-scoped deployment endpoint when no controlled manifest is active is materialized as `no_work`, not as execution failure.

## Read-only evidence

Every discovery result explicitly reports:

```text
control_plane_fetch_performed=true
artifact_acquisition_performed=false
wasmtime_compiled=false
guest_execution_performed=false
report_published=false
governed_execution_performed=false
deployment_side_effects_performed=false
```

The control-plane GET is a network read. The negative fields refer specifically to deployment/artifact/guest-execution side effects.

## Fail-closed CLI rules

Inbox discovery rejects:

```text
--node-id override
--execute
WASM --arg values
--max-artifact-bytes
```

Identity comes only from `agent.node_id`. `--acquisition-timeout-ms`, when supplied in this read-only mode, is reused only as the bounded control-plane request timeout; no artifact request is performed.

The existing I35 gates remain unchanged:

```text
--assignment auto without --execute -> rejected
manual assignment selection in bound execution mode -> rejected
HTTP without --allow-insecure-http -> rejected
```

## Focused tests

I38 adds seven tests covering:

```text
explicit inbox selector separate from auto
--execute rejected in inbox mode
--node-id override rejected
WASM args/artifact knobs rejected
zero/one/multiple classification
read-only evidence and delivered assignment IDs
cross-node assignment leakage rejected by local-node validation
```

DATUM baseline before I38 is `419 passed; 0 failed`; expected I38 total is therefore `426 passed; 0 failed`.

## Regression gate

Run from the repository root:

```powershell
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Expected test result:

```text
426 passed; 0 failed
```

The observed full-suite count is authoritative.

## Live gate

Use the existing laboratory topology:

```text
192.168.10.101 desktop / dserver / local-01
192.168.10.103 fog Sentinel / fog-01
192.168.10.108 cloud Sentinel / cloud-01
```

### Gate A — no active deployment

Start dserver with a fresh disposable I38 controlled-state path and do not publish a D-Graph/manifest yet. Then run:

```powershell
DATUM\target\debug\datum.exe wasm-deploy-live `
  --config .i35-live\local.toml `
  --deployment http://192.168.10.101:8080 `
  --assignment inbox `
  --allow-insecure-http `
  --json
```

Expected semantic result:

```text
mode=sentinel_deployment_inbox
node_id=local-01
status=no_work
assignment_count=0
artifact_acquisition_performed=false
guest_execution_performed=false
report_published=false
```

### Gate B — one node-scoped assignment

Publish the existing independent I34 external D-Graph. Do not run any WASM assignment.

Run the same inbox command for `local-01`. Expected:

```text
status=one_assignment
assignment_count=1
assignment_ids=[local-aggregate]
delivery_scope=node_slice
```

Running on `fog-01` and `cloud-01` should likewise expose only their own assignment.

### Gate C — cross-node isolation

Use a temporary config whose `agent.node_id` has no assignment in the active manifest, for example `unassigned-01`.

Expected:

```text
status=no_work
assignment_count=0
```

No fog/local/cloud assignment may appear in that result.

### Gate D — no execution side effects

Keep the artifact HTTP repository stopped during inbox discovery. Discovery must still succeed because no artifact URI is fetched.

Before and after all discovery calls:

```text
GET /api/v1/controlled-wasm/live-reports -> count=0
```

No execution ID should be created by inbox discovery.

## Non-claims

I38 does not provide:

- automatic polling loop;
- automatic deployment execution;
- multi-assignment execution scheduling (I39);
- cryptographic Sentinel identity/host attestation;
- artifact integrity verification during discovery;
- guest execution or runtime report creation.

I38 is discovery only. It exposes work addressed to the bound logical Sentinel while retaining the explicit transition required before any execution-capable path.

Checkpoint remains `143/150` until DATUM regression and the I38 live discovery gate pass.
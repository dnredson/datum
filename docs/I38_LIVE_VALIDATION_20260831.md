# I38 — Live Sentinel Deployment Inbox Validation

- checkpoint: `I38/144`
- date: `2026-08-31`
- live_gate_status: `PASS`

Validated laboratory topology:

```text
192.168.10.101 desktop / dserver / local-01
192.168.10.103 fog Sentinel / fog-01
192.168.10.108 cloud Sentinel / cloud-01
```

## No active deployment

With fresh controlled state and no D-Graph or manifest published:

```text
local-01
status=no_work
assignment_count=0
manifest_id=null
project_id=null
delivery_scope=null
```

## Active node-scoped deployment

After publishing `i34-external-dgraph` / `manifest:i34-external-dgraph`:

```text
local-01 -> one_assignment -> local-aggregate
fog-01   -> one_assignment -> fog-scale
cloud-01 -> one_assignment -> cloud-delta
```

Each result was `delivery_scope=node_slice`.

## Cross-node isolation

A temporary `probe-01` Sentinel received:

```text
status=no_work
delivery_scope=empty
assignment_count=0
assignment_ids=[]
```

No assignment targeted at fog/local/cloud leaked into the probe result.

## Read-only evidence

Every result retained:

```text
control_plane_fetch_performed=true
artifact_acquisition_performed=false
wasmtime_compiled=false
guest_execution_performed=false
report_published=false
governed_execution_performed=false
deployment_side_effects_performed=false
```

The artifact server at `192.168.10.101:8090` was unavailable during the campaign and discovery still succeeded.

Controlled live-report count remained:

```text
before=0
after=0
equal=True
```

The I38 live gate therefore passed without artifact acquisition, Wasmtime compilation, guest execution, or report publication.
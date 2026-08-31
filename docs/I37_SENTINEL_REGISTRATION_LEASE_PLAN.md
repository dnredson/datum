# I37 — Sentinel Registration and Lease

- checkpoint: `I37/143`
- predecessor: `I36/142`
- status: `implementation_ready_for_validation`

I37 gives dserver an explicit source of Sentinel presence state independent from deployment execution requests.

## Registration model

A registration record contains:

```text
node_id
instance_id
stage
capabilities
registered_at_utc
last_seen_at_utc
lease_seconds
```

The materialized API view adds server-derived:

```text
status = active | stale
deployment_side_effects_performed = false
authenticated_identity_attested = false
```

Registration and heartbeat do mutate the registration store; `deployment_side_effects_performed=false` specifically states that presence tracking does not trigger deployment/artifact/WASM activity.

This explicitly prevents logical registration from being confused with cryptographic physical-host attestation.

## Endpoints

```text
PUT  /api/v1/sentinels/:node_id/registration
POST /api/v1/sentinels/:node_id/heartbeat
GET  /api/v1/sentinels
GET  /api/v1/sentinels/:node_id
```

The path `node_id` and payload `node_id` must match. Heartbeat `instance_id` must match the registered instance. A different instance cannot silently take over an existing logical node registration.

Lease status is always derived from dserver time. Clients cannot submit `active` or `stale` status.

Accepted lease range:

```text
1 <= lease_seconds <= 3600
```

Capabilities are normalized, sorted, non-empty, and unique.

## Storage scope

I37 intentionally keeps registration/lease state in memory. Persisting a live lease across restart could incorrectly claim that an agent is still active. After a dserver restart, a Sentinel must establish presence again in a later integration step.

This is independent from I36 deployment-state persistence.

## Focused tests

Seven core tests cover:

```text
new registration materializes active
lease expiry materializes stale
heartbeat refreshes last_seen
heartbeat instance mismatch rejected
lease bounds rejected
duplicate capabilities rejected
list counts active and stale registrations
```

The I36 dserver baseline is `488 passed; 0 failed`, so the expected minimum is `495 passed; 0 failed`; the observed full-suite count is authoritative.

## Regression gate

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml
```

## Live gate

Register three logical Sentinels:

```text
fog-01   instance=fog-i37   stage=fog
local-01 instance=local-i37 stage=local
cloud-01 instance=cloud-i37 stage=cloud
```

Use a short lease on one node to demonstrate `active -> stale`, then issue a heartbeat and demonstrate return to `active` without changing `registered_at_utc` or identity.

Also exercise:

```text
path/payload node mismatch -> 400
heartbeat instance mismatch -> 409
GET /api/v1/sentinels -> count=3, active/stale counts correct
```

No deployment D-Graph, manifest, assignment, artifact, or WASM execution is required for this checkpoint.

## Non-claims

I37 does not provide:

- cryptographic agent authentication or physical-host attestation;
- persistent/HA lease state;
- automatic deployment execution;
- canonical governed execution.

Checkpoint remains `142/150` until regression and live lease validation pass.
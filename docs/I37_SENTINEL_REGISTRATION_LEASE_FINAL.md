# I37 — Sentinel Registration and Lease Final

- checkpoint: `I37/143`
- predecessor: `I36/142`
- validation_date: `2026-08-31`
- status: `validated`

## Closure

I37 gives dserver an explicit logical Sentinel presence model independent from deployment execution requests.

Validated endpoints:

```text
PUT  /api/v1/sentinels/:node_id/registration
POST /api/v1/sentinels/:node_id/heartbeat
GET  /api/v1/sentinels
GET  /api/v1/sentinels/:node_id
```

A registration carries:

```text
node_id
instance_id
stage
capabilities
registered_at_utc
last_seen_at_utc
lease_seconds
```

The dserver derives `active` / `stale` from its own time and the lease. The client cannot submit lease status.

## Regression — PASS

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check: PASS
cargo test --manifest-path dserver/Cargo.toml: 495 passed; 0 failed
cargo build --manifest-path dserver/Cargo.toml: PASS
```

The expected baseline was exactly reached: I36 had 488 tests and I37 added seven focused tests.

## Live registration gate — PASS

Three logical Sentinels were registered:

```text
local-01 instance=local-i37 stage=local lease=300s
cloud-01 instance=cloud-i37 stage=cloud lease=300s
fog-01   instance=fog-i37   stage=fog   lease=3s
```

The aggregate registration view returned `count=3`. The intentionally short fog lease naturally expired while local/cloud remained active, yielding a valid snapshot of:

```text
count=3
active_count=2
stale_count=1
```

This is expected lease behavior, not a failure.

## Lease transition gate — PASS

For `fog-01`:

```text
registration -> active
wait beyond 3s lease -> stale
heartbeat from same instance -> active
```

The heartbeat preserved `registered_at_utc` and changed `last_seen_at_utc`:

```text
registered_at_utc unchanged: True
last_seen_at_utc changed: True
```

A later aggregate list could again show the fog registration as stale if more than three seconds elapsed. I37 therefore treats active/stale as a time-derived state, not as a sticky client flag.

## Fail-closed identity gates — PASS

Heartbeat from an unregistered instance for the same logical node was rejected:

```text
HTTP 409 Conflict
heartbeat instance_id 'IMPOSTOR' does not match registered instance 'fog-i37' for node 'fog-01'
```

A registration whose path node and payload node differed was rejected:

```text
HTTP 400 Bad Request
registration path node_id 'fog-01' does not match payload node_id 'cloud-01'
```

A different instance also cannot silently replace an existing logical node registration.

## Safety and non-claims

I37 materialized registration views explicitly retain:

```text
deployment_side_effects_performed=false
authenticated_identity_attested=false
```

Registration and heartbeat do mutate the in-memory presence store, but they do not trigger deployment, artifact acquisition, WASM execution, or governed execution.

I37 does not claim:

- cryptographic agent authentication;
- physical-host attestation;
- persistent/high-availability lease storage;
- automatic deployment execution;
- canonical governed execution.

Lease state is intentionally in-memory. After dserver restart a Sentinel must establish presence again rather than inherit a potentially false active lease.

I37 therefore advances the Controlled WASM Deployment phase to `143/150`.
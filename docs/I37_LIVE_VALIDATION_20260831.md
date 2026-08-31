# I37 — Live Sentinel Registration and Lease Validation

- checkpoint: `I37/143`
- date: `2026-08-31`
- live_gate_status: `PASS`
- regression_status: `pending final branch rerun`

## Registered logical Sentinels

The live dserver accepted three independent logical registrations:

```text
local-01 instance=local-i37 stage=local lease=300
cloud-01 instance=cloud-i37 stage=cloud lease=300
fog-01   instance=fog-i37   stage=fog   lease=3
```

Materialized views reported:

```text
authenticated_identity_attested=false
deployment_side_effects_performed=false
```

## Lease expiry

Because `fog-01` intentionally used a 3-second lease, the aggregate list later materialized:

```text
count=3
active_count=2
stale_count=1
```

with `fog-01` stale while `local-01` and `cloud-01` remained active.

A direct `GET /api/v1/sentinels/fog-01` after waiting returned:

```text
status=stale
```

This is expected evidence of server-derived lease expiration, not a failure of the three-node registration gate.

## Heartbeat refresh

A heartbeat using the same logical and instance identity:

```text
node_id=fog-01
instance_id=fog-i37
```

returned:

```text
status=active
```

and the client verified:

```text
registered_at_utc unchanged = true
last_seen_at_utc changed     = true
```

The 3-second lease can naturally become stale again before a later aggregate query; a final `3/3/0` snapshot is therefore not required for PASS. The required `stale -> active` transition was directly observed on the heartbeat response.

## Fail-closed identity gates

Heartbeat using an unregistered instance identity:

```text
node_id=fog-01
instance_id=IMPOSTOR
```

returned:

```text
HTTP 409
{"error":"heartbeat instance_id 'IMPOSTOR' does not match registered instance 'fog-i37' for node 'fog-01'"}
```

A registration request sent to the `fog-01` path with payload `node_id=cloud-01` returned:

```text
HTTP 400
{"error":"registration path node_id 'fog-01' does not match payload node_id 'cloud-01'"}
```

## Conclusion

The live gate proves:

```text
three independent logical Sentinel registrations accepted
active/stale status derived from dserver time
short lease expires deterministically
same-instance heartbeat refreshes presence
registered identity is not changed by heartbeat
instance mismatch rejected
path/payload node mismatch rejected
no deployment execution side effect claimed
no cryptographic identity attestation claimed
```

No D-Graph, deployment manifest, artifact acquisition, or WASM execution was required for this validation.

Checkpoint remains `142/150` until the final I37 branch regression (`fmt`, full dserver tests, build) passes.
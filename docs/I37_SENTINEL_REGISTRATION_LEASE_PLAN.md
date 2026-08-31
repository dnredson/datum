# I37 — Sentinel Registration and Lease Plan

- checkpoint: `I37/143`
- predecessor: `I36/142`
- status: `planned`

I37 gives dserver an explicit source of Sentinel presence state that is independent from deployment execution requests.

A Sentinel registration record should include:

```text
node_id
instance_id
stage
capabilities
registered_at_utc
last_seen_at_utc
lease_seconds
status = active | stale
```

The request path node identity and payload `node_id` must match. Lease status is derived from server time and must not be supplied by the client.

The first controlled implementation may remain in-memory; persistence of deployment state was the I36 objective. Registration persistence can be added later if it becomes necessary for the final integration, but restart must never falsely preserve an active lease without a fresh heartbeat.

Proposed endpoints:

```text
PUT /api/v1/sentinels/:node_id/registration
POST /api/v1/sentinels/:node_id/heartbeat
GET /api/v1/sentinels
GET /api/v1/sentinels/:node_id
```

PASS gates:

```text
fog/local/cloud register independently
all three return active immediately after registration
heartbeat updates last_seen without changing identity
path/payload node mismatch rejected
instance mismatch on heartbeat rejected
expired lease materializes stale
no deployment execution side effect
```

This is logical presence tracking only. It does not claim cryptographic physical-host attestation.
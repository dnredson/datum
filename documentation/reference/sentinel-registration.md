# SmartSentinel registration and heartbeat contract

SmartSentinel registration/liveness is implemented in `dserver/src/core/sentinel_registration.rs` and exposed by `dserver/src/api/sentinel_registration.rs`.

This contract establishes **which agent instance currently represents one logical DATUM node** and whether that registration is still within its lease window.

It is intentionally narrower than full telemetry.

## Registration request

```rust
pub struct SentinelRegistrationRequest {
    pub node_id: String,
    pub instance_id: String,
    pub stage: Option<String>,
    pub capabilities: Vec<String>,
    pub lease_seconds: u64,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `node_id` | Logical DATUM node identity. | Binds the agent to the same node identity used by graph, capability, snapshot and control-plane state. Must be non-empty and at most 128 characters. |
| `instance_id` | Identity of this concrete running SmartSentinel instance. | Distinguishes process/instance lifetime from stable logical node identity and prevents another instance from silently taking over an already registered node. Must be non-empty and at most 128 characters. |
| `stage` | Optional architectural role such as fog/cloud. | Provides control-plane context without making stage part of stable node identity. If supplied, it cannot be empty. |
| `capabilities` | List of capability labels exposed by the registered agent. | Lets dserver know what the agent claims it can participate in at registration time. Values are trimmed, sorted and must be unique/non-empty. This lightweight list is not a replacement for the richer typed node-capability model. |
| `lease_seconds` | Liveness lease duration. | Defines how long the registration remains active after `last_seen_at_utc` without a new heartbeat. Current validation allows 1–3600 seconds. |

## Why `node_id` and `instance_id` are separate

```text
node_id     = stable logical DATUM participant
instance_id = one concrete SmartSentinel process/lifetime representing it
```

A node can survive process restarts conceptually, while a new process should have a new instance identity. The separation also detects accidental concurrent/competing agents.

## Registration API identity rule

For:

```text
PUT /api/v1/sentinels/:node_id/registration
```

the path `node_id` must exactly match the payload `node_id` after trimming. dserver rejects mismatches with `400 Bad Request`.

This prevents a request addressed to one node from registering a payload claiming to represent another.

If the node is already registered by a **different** `instance_id`, dserver returns `409 Conflict`. Re-registration by the same instance can replace/refresh the record according to the registration flow.

## Heartbeat request

```rust
pub struct SentinelHeartbeatRequest {
    pub node_id: String,
    pub instance_id: String,
}
```

The heartbeat intentionally contains only identity.

| Field | Why it is required |
|---|---|
| `node_id` | Selects the logical DATUM node whose liveness is being refreshed. |
| `instance_id` | Proves the heartbeat claims to come from the same registered agent instance, rather than another process using the same node ID. |

The heartbeat endpoint rejects:

- path/payload node mismatch;
- unknown/unregistered node (`404`);
- instance mismatch (`409`).

A valid heartbeat updates `last_seen_at_utc` while keeping `registered_at_utc` unchanged.

## Stored registration record

```rust
pub struct SentinelRegistrationRecord {
    pub node_id: String,
    pub instance_id: String,
    pub stage: Option<String>,
    pub capabilities: Vec<String>,
    pub registered_at_utc: String,
    pub last_seen_at_utc: String,
    pub lease_seconds: u64,
}
```

| Field | Meaning / purpose |
|---|---|
| `registered_at_utc` | Time the current registration record/instance was established. It represents registration lifetime origin and must not be overwritten by every heartbeat. |
| `last_seen_at_utc` | Latest successful registration/heartbeat liveness time. Used to calculate lease status. |
| `lease_seconds` | Allowed liveness gap before the view becomes stale. |

The remaining identity/context fields preserve the normalized registration request.

## Lease status

The public view derives:

```rust
pub enum SentinelLeaseStatus {
    Active,
    Stale,
}
```

with conceptually:

```text
expires_at = last_seen_at_utc + lease_seconds
now <= expires_at -> active
now >  expires_at -> stale
```

This is a **liveness lease**, not a complete node-health state machine.

A stale registration means the control plane has not received liveness within the lease. It does not by itself prove the host is powered off; it means current agent liveness evidence has expired.

## Registration view

The public `SentinelRegistrationView` contains the record fields plus:

| Field | Meaning / purpose |
|---|---|
| `status` | Derived `active` or `stale` lease status at materialization time. It is computed rather than persisted as timeless truth. |
| `deployment_side_effects_performed` | Currently always `false`. Explicitly states that registration/liveness materialization did not perform deployment mutation. |
| `authenticated_identity_attested` | Currently always `false`. Prevents the system from implying that logical/instance identity has been cryptographically/authentically attested when that feature is not implemented. |

The last field is especially important: matching IDs in a payload are **logical contract validation**, not authenticated identity proof.

## Registration list

`SentinelRegistrationList` contains:

| Field | Purpose |
|---|---|
| `count` | Number of registration views. |
| `active_count` | Number whose lease is active at list materialization time. |
| `stale_count` | Number whose lease is stale. |
| `sentinels` | Sorted node-scoped views. |
| `deployment_side_effects_performed` | Explicitly `false` for list/read materialization. |

## Storage boundary

Current registration state is held in an in-memory `BTreeMap<String, SentinelRegistrationRecord>` protected by an `RwLock` and initialized through `OnceLock`.

This has important semantics:

- registrations are process-local control-plane state;
- dserver restart does not imply persistent registration continuity unless explicitly reconstituted by later behavior;
- registration/liveness is intentionally different from persisted historical deployment state.

## Registration is not telemetry

```text
registered + active lease
```

supports the claim:

```text
"this SmartSentinel instance has recently refreshed liveness for this logical node"
```

It does **not** alone support:

```text
CPU healthy
RAM sufficient
container runtime healthy
module healthy
peer reachable
network path healthy
```

Those require corresponding telemetry/observation evidence.

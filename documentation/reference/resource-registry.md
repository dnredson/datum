# Resource Registry

The Resource Registry represents compute resources that SmartSentinel has discovered independently of whether DATUM has already assigned them a logical node role.

This separates two questions that must not be conflated:

```text
What compute resource exists and what can it do?

What DATUM role, if any, should that resource perform?
```

A discovered resource can therefore be visible to planning before it is part of the active role topology or D-Graph.

## O2 discovery states

The implemented bootstrap/resource model distinguishes:

```text
known_bound_node
  resource/node context is known to the control plane and associated with a DATUM role

known_unbound_resource
  resource is known to the Resource Registry but has no assigned role
  eligible to become a planning/deployment candidate when planning_eligible=true

unknown_resource
  local resource identity is not present in the Resource Registry
  capability/resource discovery evidence exists locally but no control-plane record is assumed
```

SmartSentinel does not infer a missing state. Unexpected control-plane responses fail closed rather than being converted into discovery or binding decisions.

## Resource-aware bootstrap

`smartsentinel-bootstrap` begins with role-free local resource discovery. It derives the same resource lookup identity used by dserver and queries the Resource Registry before resolving logical-node context.

Conceptually:

```text
SmartSentinel starts
    |
    v
local role-free resource discovery
    |
    v
derive resource lookup id
    |
    v
GET /api/v1/resources/:resource_id
    |
    +--> resource has assigned_node_id --> known_bound_node
    |
    +--> resource exists, role is null --> known_unbound_resource
    |
    +--> resource absent --> unknown_resource
```

A requested `node_id` may still be provided explicitly or by configuration. If the registry reports that the same local resource is assigned to a different role, bootstrap rejects the conflict instead of silently accepting the requested role.

Bootstrap is observation/query-only in O2. It does not register an unknown resource, assign a role, take over an existing role, or mutate deployment state.

An existing logical-node context in dserver can still classify an explicitly requested role as `known_bound_node`; the Resource Registry binding is the implemented control-plane path for associating a discovered resource with a logical role.

## Role-free SmartSentinel discovery

`smartsentinel-resource-discovery` collects an observation-only resource envelope without requiring a DATUM `node_id`.

The normal DATUM configuration structure can still be reused. When `agent.node_id` is absent, the resource-discovery/bootstrap loader creates only an ephemeral internal collector scope. That synthetic value is not exported as a logical node identity.

The role-free envelope contains:

- SmartSentinel process `instance_id`;
- local observation host identifier;
- hardware fingerprint evidence when available;
- OS installation identity when available;
- platform/architecture;
- structural capacity profile;
- observed capabilities and availability;
- no role binding;
- no predecessor binding;
- no execution authorization;
- no host-side effects.

## Resource identity assigned by dserver

dserver derives a Resource Registry identity from the strongest stable evidence available.

### Canonical hardware identity

When `hardware_fingerprint_id` exists:

```text
resource_id = resource:hw:<digest>
identity_scope = canonical_hardware
planning_eligible = true
```

The digest is already a one-way SmartSentinel fingerprint. dserver does not need raw serial numbers or UUIDs.

### Provisional OS identity

When hardware cannot be identified but `os_installation_id` exists:

```text
resource_id = resource:os:<digest>
identity_scope = provisional_os
planning_eligible = true
```

This is intentionally provisional. It provides stable-enough continuity for discovery and planning in environments where the guest cannot see reliable hardware identifiers, while preserving the fact that physical identity is unknown.

A future observation with stronger hardware evidence can be correlated by the control plane without pretending the earlier OS-based identity was physical attestation.

### Ephemeral identity

If neither hardware nor OS installation identity is available:

```text
resource_id = resource:instance:<instance_id>
identity_scope = ephemeral_instance
planning_eligible = false
```

Such a report remains useful operational evidence but must not silently become a stable planning resource.

## Unbound resource state

New discoveries enter the registry as:

```text
state = available_unbound
assigned_node_id = null
```

Resource discovery does not assign a DATUM role. Role assignment is a separate governed control-plane operation.

Conceptually:

```text
SmartSentinel
    |
    | discovery
    v
Resource Registry
    |
    | resource:os:ABC
    | RAM 8 GiB
    | Python available
    | Wasmtime available
    | Docker present but unavailable
    | role = null
    v
Planner candidate pool
```

## Governed initial role binding

O2 includes an explicit control-plane operation for the first assignment of an unbound planning resource to a logical DATUM role.

The operation accepts only:

```text
transition_kind = initial_assignment
```

and creates an active binding record such as:

```text
binding_id = binding:<uuid>
resource_id = resource:os:ABC
node_id = fog-01
status = active
predecessor_binding_id = null
control_plane_authorized = true
automatic_assignment_performed = false
```

At the same time the Resource Registry record becomes:

```text
state = bound
assigned_node_id = fog-01
```

The binding freezes the identity/capacity context observed at assignment time, including `identity_scope`, optional hardware fingerprint, optional OS installation identity, and `capacity_profile_id`. This preserves evidence needed for later history and AI/GNN reasoning even if the resource is subsequently upgraded or replaced.

Initial binding is deliberately fail-closed:

```text
same resource -> same active node
  idempotent; return the existing binding

same resource -> different active node
  409 conflict

different resource -> node with an active binding
  409 conflict

non-planning-eligible resource
  rejected

hardware_replacement / rebind transition sent to initial endpoint
  rejected
```

Hardware replacement and controlled rebind are intentionally separate future operations because they must close the previous active binding and create a successor with explicit lineage:

```text
B2.predecessor_binding_id = B1
```

No historical binding is overwritten.

## Rediscovery

Repeated discovery of the same derived resource identity updates the latest observation while preserving:

- `first_seen_utc`;
- `last_seen_utc`;
- an `observation_count`;
- the most recent SmartSentinel `instance_id`;
- current discovery/capability/capacity evidence.

This makes repeated observation distinguishable from creation of a new resource. If the resource is already bound, rediscovery preserves its bound state and assigned node while updating the latest evidence.

## Planning semantics

The Resource Registry provides the future planner with candidate resources that are not necessarily present in the current D-Graph.

This enables questions such as:

```text
Which unbound resources currently satisfy the workload requirements?

Is there a discovered resource with more RAM/GPU/runtime capacity than the current fog node?

Can a failed role be rebound to an available resource?
```

A resource being technically eligible is not authorization to deploy or bind it. Capability availability, planning eligibility, execution policy, and operator/control-plane authorization remain separate concerns.

## Relation to resource observations

The existing `resource_observation` materialization path joins live observations to nodes already present in the observed D-Graph. A resource not represented in the graph is currently reported as unmatched.

The Resource Registry fills the preceding lifecycle stage:

```text
resource discovery
    -> Resource Registry
    -> planning candidate
    -> governed role binding
    -> D-Graph node participation
    -> live resource observation joined to that node
```

This avoids forcing every discovered machine to masquerade as a logical DATUM node simply to publish capacity.

## API

O2 exposes:

```text
POST /api/v1/resources/discovery
GET  /api/v1/resources
GET  /api/v1/resources/:resource_id
POST /api/v1/resources/:resource_id/binding
GET  /api/v1/resources/:resource_id/binding
GET  /api/v1/resource-bindings
```

All calls are scoped by `X-Datum-Project`.

The resource list includes `available_unbound_count` and `bound_count`. The binding list exposes active/history records retained by the current process-local proof implementation.

## Persistence boundary

The first O2 Resource Registry and role-binding implementation is intentionally process-local, matching the current Sentinel-registration proof path. It establishes identity, validation, deduplication, explicit assignment, conflict handling, API, and planning semantics without prematurely changing Firestore schemas.

Durable resource/binding history is required before production historical reasoning. A later persistence step must preserve resource identity, role-binding lineage, capacity revisions, and transitions across dserver restarts rather than overwriting history.

## AI context

AI/GNN/planner layers should eventually receive both bound and unbound resource context.

For example, the system can distinguish:

```text
fog-01 is degraded

resource:R1 is unbound and has required runtime/capacity
resource:R2 is unbound but lacks required container runtime
```

The deterministic planner can establish candidate eligibility first. An AI reasoning layer can then compare historical behavior, risk, expected performance, environmental context, prior outcomes without being asked to invent infrastructure that was never observed.

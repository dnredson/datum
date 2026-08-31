# Snapshot data model

The local DATUM snapshot model is defined in `DATUM/src/core/model.rs`.

A `Snapshot` captures one observation cycle as a transport/storage structure. It is deliberately generic: domain semantics are expressed through entity kinds, attributes, relationships, events, and action records.

## `Snapshot`

```rust
pub struct Snapshot {
    pub meta: Meta,
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
    pub actions: Vec<ActionRecord>,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `meta` | Identity/version/timing information for the entire cycle. | Lets consumers validate schema/agent versions and associate every item with one observation cycle and source node. |
| `entities` | Resources or domain objects observed/modelled in this cycle. | Provides graph vertices without hard-coding one top-level collection per domain type. |
| `edges` | Typed relationships between entities. | Preserves topology and semantic relationships such as hosting, execution, dependency, or flow participation. |
| `events` | Relevant state changes, warnings, failures, or lifecycle observations. | Carries facts that are temporal occurrences rather than durable entity attributes. |
| `actions` | Records of planned/attempted/completed local actions. | Keeps execution evidence correlated with the observation cycle and supports auditing/feedback. |

## `Meta`

```rust
pub struct Meta {
    pub schema_version: String,
    pub agent_version: String,
    pub cycle_id: String,
    pub observed_at_utc: DateTime<Utc>,
    pub node_id: String,
    pub host_id: String,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `schema_version` | Version of the snapshot contract. | Allows dserver/other consumers to reject, migrate, or interpret payloads safely as the schema evolves. It versions the **data contract**, not the agent binary. |
| `agent_version` | Version of the agent that produced the snapshot. | Enables provenance and compatibility debugging: two payloads using the same schema may have been produced by different software versions. |
| `cycle_id` | Identifier for the collection cycle. | Correlates all entities, edges, events, and actions produced together and distinguishes repeated observations from the same node. |
| `observed_at_utc` | UTC time associated with the snapshot cycle. | Establishes temporal context for freshness and ordering. UTC avoids ambiguity across hosts/time zones. |
| `node_id` | Logical DATUM node identity. | Primary semantic join key across SmartSentinel, dserver, capabilities, snapshots, and node-scoped state. It must not be replaced by hostname/IP. |
| `host_id` | Identifier for the underlying host as reported by the agent. | Separates DATUM's logical node identity from host-level identity and assists provenance/diagnostics when a logical node is associated with a particular machine. |

### `node_id` vs. `host_id`

`node_id` answers **which DATUM participant is this?**. `host_id` answers **which host identity produced/backs this observation?**. Keeping both avoids coupling the graph model to one hostname or network address.

## `Entity`

```rust
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
    pub ttl_s: u64,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `id` | Stable identifier of the entity within the relevant graph/domain scope. | Allows edges and later observations to refer to the same conceptual resource. |
| `kind` | Domain category of the entity. | Gives semantics to the generic entity primitive, e.g. distinguishing node/runtime/module/service-like resources. |
| `attrs` | Kind-specific JSON attributes. | Keeps the common graph primitive extensible without adding every domain field to every entity. Domain-specific attributes should still be documented and validated. |
| `observed_at_utc` | Time this entity state was observed. | Allows entity-level freshness to differ from other entities in the same broader system. |
| `ttl_s` | Time-to-live in seconds for the observation/entity state. | Defines an explicit validity horizon so stored data cannot be treated as current forever. |

### Why `attrs` is JSON

`attrs` gives DATUM flexibility to represent heterogeneous resources. The cost of that flexibility is weaker compile-time typing at this layer. For that reason, higher-level contracts must define expected fields and ranges for important entity kinds.

### Why an entity has its own timestamp

A snapshot has a cycle timestamp, but individual evidence may originate at different times or need independent freshness handling. Entity-level time keeps this possible without redefining the whole snapshot contract.

## `Edge`

```rust
pub struct Edge {
    pub from: String,
    pub to: String,
    pub rel: String,
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `from` | Source entity identifier. | Identifies the origin of the directed relationship. |
| `to` | Destination entity identifier. | Identifies the target of the directed relationship. |
| `rel` | Relationship type. | Tells consumers what the connection **means**; connectivity alone is not sufficient semantics. |
| `attrs` | Relationship-specific JSON attributes. | Holds properties that belong to the relationship rather than either endpoint, such as status/metadata. |
| `observed_at_utc` | Time the relationship/evidence was observed. | Supports freshness for dynamic relationships such as runtime or network observations. |

An edge is directed at the representation level. Consumers should not assume symmetry unless the specific relationship contract defines it.

## `Event`

```rust
pub struct Event {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub target: String,
    pub message: String,
    pub correlation_id: Option<String>,
    pub source_plugin: Option<String>,
    pub status: Option<String>,
    pub details: Option<JsonValue>,
    pub observed_at_utc: DateTime<Utc>,
}
```

| Field | Meaning | Why it exists |
|---|---|---|
| `id` | Event identity. | Supports deduplication/reference/audit. |
| `kind` | Event category. | Lets consumers group and interpret events without parsing free-form messages. |
| `severity` | Importance/impact classification. | Supports filtering and operator prioritization. |
| `target` | Resource/entity the event concerns. | Connects the occurrence to the affected system object. |
| `message` | Human-readable explanation. | Makes diagnostics understandable without requiring every consumer to decode internal fields. |
| `correlation_id` | Optional identifier shared with related actions/events. | Reconstructs multi-step operations and cause/effect chains. |
| `source_plugin` | Optional producing collector/plugin. | Records provenance when multiple collectors can emit events. |
| `status` | Optional event-specific state. | Allows a category to carry structured lifecycle/status meaning without overloading `message`. |
| `details` | Optional structured JSON detail. | Adds machine-readable context without changing the common event envelope. |
| `observed_at_utc` | Event observation time. | Establishes ordering and temporal relevance. |

## `ActionRecord`

`ActionRecord` represents execution-related evidence. It is not proof that execution occurred merely because a record exists; fields such as `dry_run`, `suppressed`, `status`, and `exit_code` determine what actually happened.

| Field | Meaning / purpose |
|---|---|
| `id` | Unique action-record identity. |
| `correlation_id` | Joins the record to related plan/event/execution context. |
| `origin` | Identifies where the action request came from. |
| `kind` | Classifies the action. |
| `phase` | Indicates the action's lifecycle/processing phase. |
| `status` | Structured outcome/state of the action. |
| `service_id` | Stable identity of the associated service/component. |
| `service_name` | Human-readable service name for diagnostics/operator output. |
| `target_kind` | Type/category of the execution target. |
| `target_id` | Identity of the execution target. |
| `reason` | Optional structured/human-readable rationale. |
| `message` | Optional additional outcome information. |
| `dry_run` | Explicitly marks evaluation without intended real side effects. This prevents dry-run success from being mistaken for mutation. |
| `suppressed` | Indicates that execution was intentionally prevented/suppressed. |
| `exit_code` | Optional process/tool exit result when such execution occurred. |
| `duration_ms` | Optional measured execution duration. |
| `stdout` | Optional captured standard output for evidence/debugging. |
| `stderr` | Optional captured standard error for evidence/debugging. |
| `observed_at_utc` | Time associated with the action observation/outcome. |

## Invariants and interpretation rules

- A snapshot is evidence from a cycle, not eternal truth.
- `schema_version` and `agent_version` answer different compatibility questions.
- `node_id`, hostname, host identity, and IP address must not be treated as interchangeable.
- `kind`/`rel` give semantic meaning to generic graph primitives.
- `attrs`/`details` are extension points, not permission to leave important fields undocumented.
- `dry_run=true` or `suppressed=true` means a record must not be presented as completed real-world mutation.
- Stored observations can become stale based on timestamp/TTL/freshness contracts.

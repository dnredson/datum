# dserver API overview

The current dserver API is implemented with Axum in `dserver/src/main.rs` and domain modules under `dserver/src/api/`.

This page is a **map of the API surface**. Detailed request/response field pages should be generated from each domain model and, later, from an OpenAPI contract so documentation and code cannot drift silently.

## Conventions

- `/health` is the service health endpoint.
- Versioned control-plane APIs use `/api/v1/...`.
- `:node_id`, `:task_id`, `:attempt_id`, and similar segments are path identities, not display names.
- A `GET` endpoint normally reads/materializes current control-plane information.
- `POST`, `PUT`, and `PATCH` can represent ingestion or state transitions; the HTTP method alone does **not** tell you whether the managed external environment was mutated.

## SmartSentinel registration / liveness

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/sentinels` | List known SmartSentinel registrations. |
| GET | `/api/v1/sentinels/:node_id` | Read one node's SmartSentinel registration. |
| PUT | `/api/v1/sentinels/:node_id/registration` | Create/replace node-scoped SmartSentinel registration information. |
| POST | `/api/v1/sentinels/:node_id/heartbeat` | Publish agent liveness heartbeat. |

A registration or heartbeat proves agent identity/liveness according to that contract; it is not a complete host-health report.

## Snapshot and node evidence

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/snapshots` | Ingest a DATUM snapshot. |
| GET | `/api/v1/nodes` | List node-oriented latest state. |
| GET | `/api/v1/nodes/:node_id/latest` | Get latest state for one logical node. |

The snapshot contract is documented in [Snapshot data model](../reference/snapshot-model.md).

## Capabilities

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/nodes/:node_id/capabilities` | Publish/update capability information for a node. |
| GET | `/api/v1/nodes/:node_id/capabilities` | Read capability information for a node. |
| GET | `/api/v1/capabilities` | List capability documents. |

The capability contract is documented in [Capabilities data model](../reference/capabilities-model.md).

## Action-result evidence

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/nodes/:node_id/action-results` | Publish action execution/outcome evidence. |
| GET | `/api/v1/nodes/:node_id/action-results/latest` | Read latest action result for a node. |
| GET | `/api/v1/action-results/latest` | List latest action results across nodes. |

Action results are evidence. Consumers must inspect status/dry-run/suppression/outcome fields rather than treating the existence of a report as successful mutation.

## WASM runtime/module evidence

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/nodes/:node_id/wasm-runtime-reports` | Publish WASM runtime evidence for a node. |
| GET | `/api/v1/nodes/:node_id/wasm-runtime-reports/latest` | Get latest node WASM runtime report. |
| GET | `/api/v1/wasm-runtime-reports/latest` | List latest WASM runtime reports. |
| POST / GET | `/api/v1/wasm-modules` | Register/list WASM module descriptors. |
| GET | `/api/v1/wasm-modules/:module_id` | Read a module descriptor. |

## Generic graph, analysis, topology and planning

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/graph/latest` | Read latest graph view. |
| GET | `/api/v1/graph/materialized` | Read materialized graph. |
| GET | `/api/v1/analysis/latest` | Build/read current analysis from graph state. |
| POST / GET | `/api/v1/topology` | Set/read project topology. |
| POST / GET | `/api/v1/dmap` | Set/read project DMap. |
| GET | `/api/v1/action-plan/latest` | Build latest action plan. |
| GET | `/api/v1/action-plan/materialized` | Read materialized action plan. |
| GET | `/api/v1/deployment-planning/latest` | Read latest deployment-planning result. |
| GET | `/api/v1/deployment-planning/for-node/:node_id` | Node-scoped deployment-planning view. |
| GET | `/api/v1/installation-plan/latest` | Latest installation plan. |
| GET | `/api/v1/installation-plan/for-node/:node_id` | Node-scoped installation plan. |

## Local mapping and installation tasks

The API contains endpoints for:

- materializing installation tasks globally or for one node;
- listing/getting node installation tasks;
- patching task status;
- approvals;
- readiness;
- local-mapping recommendations;
- local-mapping patch proposals;
- preflight reports;
- assisted-execution requests, claims, attempts, and current assisted-execution state.

These endpoints implement a deliberately staged workflow: recommendation/proposal/preflight/task/approval/request/claim/attempt are distinct records so the system can explain **why** execution is or is not allowed and reconstruct what happened later.

## Local D-Graph and materialization

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/nodes/:node_id/dgraph-snapshot` | Publish a node-local D-Graph snapshot. |
| GET | `/api/v1/nodes/:node_id/dgraph-snapshot/latest` | Read latest local D-Graph snapshot for one node. |
| GET | `/api/v1/dgraph-snapshots/latest` | List latest local D-Graph snapshots. |
| GET | `/api/v1/dgraph/materialized` | Read materialized multi-source D-Graph. |
| GET | `/api/v1/dgraph/materialized/summary` | Compact summary of materialized D-Graph. |

## D-Graph planning/readiness/contracts

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/dgraph/planner/dry-run` | Full dry-run planning result. |
| GET | `/api/v1/dgraph/planner/dry-run/summary` | Compact dry-run summary. |
| GET | `/api/v1/dgraph/readiness/summary` | Readiness evaluation summary. |
| GET | `/api/v1/dgraph/artifact-integrity` | Artifact-integrity contract/result. |
| GET | `/api/v1/dgraph/capability-negotiation` | Capability-negotiation contract/result. |
| GET | `/api/v1/dgraph/execution-allowlists` | Execution allowlist contract/result. |
| GET | `/api/v1/dgraph/resource-limits` | Resource-limit contract/result. |
| GET | `/api/v1/dgraph/rollback-audit` | Rollback/audit contract/result. |
| GET | `/api/v1/dgraph/execution-prerequisites` | Execution prerequisite contract/result. |
| GET | `/api/v1/dgraph/execution-safety` | Execution-safety contract/result. |
| GET | `/api/v1/dgraph/execution-policy` | Execution-policy contract/result. |
| GET | `/api/v1/dgraph/operator-approval` | Operator-approval contract/result. |

These are intentionally separate. DATUM's safety model does not collapse capability, resources, integrity, policy, approval, and allowlisting into one opaque decision.

## Controlled WASM baseline

| Method | Path | Purpose |
|---|---|---|
| POST / GET | `/api/v1/controlled-wasm/dgraph` | Store/read the controlled-WASM source D-Graph. |
| GET | `/api/v1/controlled-wasm/observed-dgraph` | Read evidence-enriched controlled-WASM graph. |
| POST / GET | `/api/v1/controlled-wasm/deployment-manifest` | Store/read active deployment manifest. |
| GET | `/api/v1/controlled-wasm/deployment/for-node/:node_id` | Node-scoped controlled deployment view. |
| POST / GET | `/api/v1/controlled-wasm/live-reports` | Publish/list live runtime/deployment reports. |
| POST / GET | `/api/v1/controlled-wasm/deployment-attempts` | Create/list controlled deployment attempts. |
| GET | `/api/v1/controlled-wasm/deployment-attempts/:attempt_id` | Read one attempt. |
| POST | `/api/v1/controlled-wasm/deployment-attempts/:attempt_id/transition` | Apply an explicit lifecycle transition. |
| POST | `/api/v1/controlled-wasm/deployment-attempts/:attempt_id/retry` | Request retry through the controlled lifecycle contract. |

This API family is a frozen/validated historical baseline. New SmartSentinel observability and later adaptive-IoTinuum behavior should extend the system without retroactively changing what this baseline claimed.

## Next API-documentation layer

The next documentation pass should generate a dedicated page per API family with:

1. request/response schema;
2. field-by-field semantics;
3. required vs. optional fields;
4. identity rules;
5. timestamps/freshness rules;
6. validation/error behavior;
7. side-effect classification;
8. example payloads grounded in tests/fixtures;
9. source-code location;
10. relationship to D-Graph entities/contracts.

OpenAPI should later become the machine-readable protocol reference; these pages remain the semantic explanation of **why** each field and endpoint exists.

# Assisted Execution Safety Model

## Purpose

This document captures the current DATUM/SmartSentinel assisted execution architecture and its safety boundaries before any assisted installation execution is implemented.

Current status:
- The end-to-end assisted installation/control chain exists as metadata only.
- No assisted installation execution exists.
- No claim, approval, readiness decision, or request grants execution permission by itself.

## Roles

### `dserver`

`dserver` is the global controller and planner.

Responsibilities:
- stores the global graph and aggregated node state
- builds analysis, action-plan, dispatch, deployment_planning, and installation_plan
- stores installation tasks and all assisted metadata
- stores operator approvals
- stores readiness evaluations
- stores assisted execution requests
- stores assisted execution claims and claim lifecycle status

Safety boundary:
- `dserver` never executes local node actions
- `dserver` must not run Docker, WASM, migrations, artifact transfer, or installation steps

### `DATUM`

`DATUM` is the local node agent and client.

Responsibilities:
- monitors the local node
- publishes capabilities
- publishes snapshots
- publishes action results
- consumes node-scoped dispatch
- performs only local guarded operations when explicitly allowed by existing guarded paths

Current assisted execution status:
- `DATUM` can view and submit assisted metadata
- `DATUM` can update claim lifecycle metadata
- `DATUM` currently does not perform assisted installation execution

## Current Metadata-Only Chain

Installation planning/control chain:

`deployment_planning`
-> `installation_plan`
-> `installation_tasks`
-> `installation_policy`
-> `operator approval`
-> `readiness`
-> `assisted execution request`
-> `assisted execution claim`
-> `claim lifecycle`

### `deployment_planning`

Meaning:
- global planning output derived by the server from dispatch feedback and capability context

Server endpoint(s):
- `GET /api/v1/deployment-planning/latest`
- `GET /api/v1/deployment-planning/for-node/:node_id`

Client command(s):
- `cargo run -- deployment-planning --config config.toml`
- `cargo run -- deploy-plan --config config.toml`

Execution:
- no

### `installation_plan`

Meaning:
- server-derived installation-oriented plan built from deployment planning

Server endpoint(s):
- `GET /api/v1/installation-plan/latest`
- `GET /api/v1/installation-plan/for-node/:node_id`

Client command(s):
- `cargo run -- installation-plan --config config.toml`
- `cargo run -- install-plan --config config.toml`

Execution:
- no

### `installation_tasks`

Meaning:
- server-materialized node-scoped installation tasks derived from installation plan items

Server endpoint(s):
- `POST /api/v1/installation-tasks/materialize/for-node/:node_id`
- `GET /api/v1/installation-tasks`
- `GET /api/v1/installation-tasks/for-node/:node_id`
- `GET /api/v1/installation-tasks/for-node/:node_id/:task_id`
- `PATCH /api/v1/installation-tasks/for-node/:node_id/:task_id/status`

Client command(s):
- `cargo run -- installation-tasks --config config.toml`
- `cargo run -- installation-tasks --config config.toml --materialize`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id>`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --status <status>`

Execution:
- no

### `installation_policy`

Meaning:
- policy metadata attached to installation tasks that describes whether a task is manual-only or a candidate for future assisted execution

Server endpoint(s):
- included in installation task responses

Client command(s):
- displayed by `installation-tasks` viewer modes

Execution:
- no

### `operator approval`

Meaning:
- metadata that records operator approval for a task when policy requires it

Server endpoint(s):
- `POST /api/v1/installation-tasks/for-node/:node_id/:task_id/approval`
- `GET /api/v1/installation-tasks/for-node/:node_id/:task_id/approval/latest`
- `GET /api/v1/installation-task-approvals/for-node/:node_id`

Client command(s):
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --approve --approved-by <name>`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --approval`
- `cargo run -- installation-tasks --config config.toml --approvals`

Execution:
- no

### `readiness`

Meaning:
- server-computed metadata that indicates whether a task is ready for assisted execution review, not for execution itself

Server endpoint(s):
- `GET /api/v1/installation-task-readiness/for-node/:node_id`
- `GET /api/v1/installation-tasks/for-node/:node_id/:task_id/readiness`

Client command(s):
- `cargo run -- installation-tasks --config config.toml --readiness`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --readiness`

Execution:
- no

### `assisted execution request`

Meaning:
- metadata that records a request for future assisted execution handling

Server endpoint(s):
- `POST /api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request`
- `GET /api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/latest`
- `GET /api/v1/assisted-execution-requests/for-node/:node_id`

Client command(s):
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --request-assisted-execution --requested-by <name> --reason <text>`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --execution-request`
- `cargo run -- installation-tasks --config config.toml --execution-requests`

Execution:
- no

### `assisted execution claim`

Meaning:
- metadata that records that a node or operator has claimed an assisted execution request for future processing

Server endpoint(s):
- `POST /api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim`
- `GET /api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/latest`
- `GET /api/v1/assisted-execution-claims/for-node/:node_id`

Client command(s):
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --request-id <request_id> --claim-assisted-execution --claimed-by <name> --reason <text>`
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --request-id <request_id> --execution-claim`
- `cargo run -- installation-tasks --config config.toml --execution-claims`

Execution:
- no

### `claim lifecycle`

Meaning:
- metadata-only status transitions on the latest claim, such as `claimed`, `released`, `expired`, `completed`, and `cancelled`

Server endpoint(s):
- `PATCH /api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/status`
- updated status is visible through latest claim and claim list endpoints

Client command(s):
- `cargo run -- installation-tasks --config config.toml --task-id <task_id> --request-id <request_id> --claim-status <status> --reason <text>`

Execution:
- no

## Safety Invariants

- `dserver` must never execute Docker, WASM, migration, artifact transfer, or local installation.
- `DATUM` must not treat approval, readiness, request, or claim as execution permission by itself.
- `safe_to_auto_execute` remains `false` across the assisted installation chain.
- Docker `run`, `pull`, and `rm` must not be introduced without an explicit future milestone and safety review.
- Existing guarded container execution only supports known local containers and only `docker inspect` and `docker start`.
- No installation task may be executed unless:
  - task exists
  - policy permits assisted mode
  - operator approval exists
  - readiness is `ready_for_assisted_execution`
  - assisted execution request exists
  - claim exists and is active
  - local client configuration explicitly enables the executor
  - executor-specific guardrails pass
- Even after all metadata exists, execution must remain opt-in.

## Current Non-Execution Guarantees

The following are not implemented:
- no artifact transfer
- no artifact verification
- no Docker run
- no Docker pull
- no container creation
- no WASM runtime
- no migration execution
- no flow reconfiguration execution
- no automatic installation execution
- no AI/LLM autonomous execution

## Candidate Next Implementation Paths

### Path A: Assisted execution no-op/dry-run executor

Description:
- `DATUM` would consume ready/requested/claimed task metadata
- it would generate an execution-attempt report
- it would not perform side effects

Why this matters:
- validates end-to-end executor wiring without changing local state
- produces observable attempt telemetry for later policy review
- is the safest first step

Recommendation:
- recommended next milestone

### Path B: WASM runtime v0

Description:
- `DATUM` would run a trivial local WASM module in a sandbox
- no filesystem or network access initially
- used only to validate runtime integration
- not connected to installation tasks at first

Why this matters:
- validates runtime boundaries separately from installation logic
- creates a future path for constrained local helpers

### Path C: Guarded local mapping preparation

Description:
- `DATUM` would suggest or generate local mapping changes
- no automatic config write initially
- operator review required

Why this matters:
- targets a narrow, reviewable local preparation workflow
- can remain non-destructive while proving task-to-operator handoff

## Proposed Next Milestones

- `S23`: Server assisted execution attempt/report contract
- `C20`: Client assisted execution no-op attempt publisher
- `I11`: Smoke test for no-op assisted execution attempt
- `S24/C21`: Optional guarded local preparation step
- later: WASM runtime v0
- later: artifact transfer/verification
- later: real assisted installation executor

## Summary

The system currently has a complete metadata-only assisted execution chain. That chain is useful for policy, orchestration, review, and smoke validation, but it is not an execution path. Any future move from metadata to execution should begin with a no-op or dry-run attempt model, preserve `safe_to_auto_execute = false`, and require explicit opt-in plus local guardrails before any side effects are possible.

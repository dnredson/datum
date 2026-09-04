# Action execution reports

`ActionExecutionReport` is defined in `DATUM/src/action_results/model.rs`. It is the node-side execution/outcome evidence contract associated with dispatch/action planning.

The key rule is: **a report is evidence about what happened; its existence alone does not mean the requested action executed successfully.**

## `ActionExecutionReport`

| Field | Meaning | Why it exists |
|---|---|---|
| `project_id` | Optional project/application scope. | Correlates a report with the correct DATUM project when infrastructure is shared. |
| `node_id` | Optional logical node identity producing/owning the report. | Joins the report to node-scoped dispatch, capability and D-Graph state. |
| `report_id` | Optional report identity. | Allows deduplication/reference and historical audit. |
| `dispatch_version` | Optional version of the dispatch contract that produced the action set. | Makes outcome interpretation compatible with evolving dispatch semantics. |
| `action_plan_id` | Optional originating action-plan identifier. | Links execution evidence back to the reasoning/planning result that requested it. |
| `dispatch_generated_at_utc` | Optional time when the dispatch was generated. | Supports ordering, age checks and causal reconstruction. |
| `reported_at_utc` | Optional time when this report was produced. | Distinguishes dispatch creation from outcome reporting time. |
| `source` | Optional report producer/origin. | Provides provenance when multiple runners/agents could produce execution evidence. |
| `results` | Per-action outcomes. | Preserves individual results instead of collapsing a multi-action dispatch into one global status. |
| `summary` | Optional aggregate counters. | Gives operators/control-plane code a compact view without losing the detailed `results`. |
| `metadata` | Optional auxiliary structured information. | Extension point for non-core data. |
| `extra` | Flattened additional fields. | Forward-compatible schema evolution. |

## `ActionExecutionResult`

| Field | Meaning / purpose |
|---|---|
| `action_id` | Required identity of the action whose outcome is being reported. |
| `action_kind` | Optional semantic type of the action. Useful for grouping/diagnostics and validating that an outcome matches the requested operation. |
| `runtime` | Optional runtime family/implementation involved in execution. |
| `target_node` | Optional node originally targeted by the action. |
| `assigned_node` | Optional node actually assigned for the action. Keeping this separate from `target_node` exposes planner/dispatch assignment decisions and prevents silent retargeting. |
| `execution_status` | Required structured outcome classification. This is the primary field describing what happened. |
| `validation_status` | Optional validation result associated with execution. Separates pre/execution validation from the execution outcome itself. |
| `side_effects_performed` | Optional explicit statement of whether real external state changes occurred. This is critical for distinguishing dry-run/blocked/would-execute from actual mutation. |
| `message` | Optional human-readable outcome explanation. |
| `started_at_utc` | Optional execution start timestamp. |
| `finished_at_utc` | Optional execution finish timestamp. Together with start time it supports duration/order analysis and audit. |
| `error` | Optional structured/free-form error information. Used when execution or validation fails. |
| `artifact` | Optional artifact reference associated with the action. Useful for deploy/install operations and later provenance checks. |
| `capability` | Optional structured capability context. Records what capability information influenced or accompanied the action. |
| `ai_context` | Optional structured AI-derived context. Keeps AI reasoning/context explicit and separable from deterministic execution status. |
| `metadata` | Optional auxiliary structured information. |
| `extra` | Forward-compatible additional fields. |

## `target_node` vs. `assigned_node`

These fields must not be merged conceptually:

```text
target_node   = requested/intended target
assigned_node = node selected/assigned for execution
```

If the system later performs placement or failover, this distinction is necessary to explain where the action was supposed to go versus where it was actually assigned.

## `side_effects_performed`

This field is one of the most important safety/audit fields in the report.

Examples:

```text
execution_status = would_execute
side_effects_performed = false
```

means the path was evaluated but did not mutate the environment.

```text
execution_status = started
side_effects_performed = true
```

can represent an operation that actually changed runtime state, subject to the exact executor contract.

Consumers should never infer side effects solely from a successful HTTP response or report presence.

## `ActionExecutionSummary`

The summary aggregates detailed results.

| Field | Meaning / purpose |
|---|---|
| `total_results` | Total number of per-action results included. |
| `dry_run` | Count classified as dry-run. |
| `blocked` | Count prevented by a gate/policy/contract. |
| `skipped` | Count intentionally not processed/executed. |
| `rejected` | Count rejected as invalid/ineligible. |
| `would_execute` | Count that passed enough checks to indicate they would execute in a non-mutating evaluation path. |
| `already_running` | Count whose desired runtime/container state was already satisfied. |
| `started` | Count successfully started according to the executor contract. |
| `not_installed` | Count where required runtime/component was not installed/present. |
| `missing_local_mapping` | Count blocked because a local mapping needed to translate logical intent to a concrete local resource was missing. |
| `docker_unavailable` | Count where Docker/container runtime was unavailable for the requested operation. |
| `failed` | Count of failed execution attempts. |
| `side_effects_performed` | Count of results explicitly reporting real side effects. |
| `extra` | Forward-compatible additional counters. |

## Why the summary contains domain-specific statuses

Statuses such as `missing_local_mapping` and `docker_unavailable` are intentionally more informative than a single `failed` bucket. They expose **why** an operation could not proceed, which is essential for DATUM's capability-aware and fail-closed planning/execution model.

## Interpretation rules

- `execution_status` is required per result; summary counters are derived/aggregate convenience, not a replacement for details.
- `would_execute` is not execution.
- `dry_run` is not execution.
- `blocked`/`rejected`/`skipped` are different control outcomes and should not be flattened into the same meaning.
- `side_effects_performed` should be treated as explicit evidence about mutation.
- AI context is supporting context; it does not override deterministic validation/execution fields.

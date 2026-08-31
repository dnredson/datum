# I39/145 — Explicit Multi-Assignment Scheduling Final

Status: VALIDATED

I39 closes the explicit multi-assignment scheduling checkpoint without enabling automatic governed execution.

## Regression

```text
DATUM fmt: PASS
DATUM test: 431 passed; 0 failed
DATUM build: PASS
```

## Validated batch contract

```text
--assignment inbox           -> read-only discovery
--assignment auto --execute  -> exactly one assignment
--assignment batch --execute -> two or more assignments
```

Batch scheduling is deterministic:

```text
scheduling_policy=assignment_id_ascending
failure_policy=stop_on_first_failure
```

All assignments are acquired, integrity-verified, and compile-preflighted before the first guest execution attempt. Execution remains explicitly authorized by the operator. No hidden retries are introduced.

## Live fixture

The I39 manifest intentionally listed the local assignments in reverse lexical order:

```text
z-local-second -> local-01
a-local-first  -> local-01
cloud-untouched -> cloud-01
```

The local inbox reported `multiple_assignments` with two local assignments.

The legacy `auto` path rejected the two-assignment local slice and created no execution report.

The explicit batch path executed:

```text
1. a-local-first  -> result 30
2. z-local-second -> result 30
```

Aggregate execution evidence:

```text
assignment_count=2
attempted_count=2
executed_count=2
failed_or_blocked_count=0
status=executed
```

## dserver evidence

Exactly two live reports were retained, both for `local-01`:

```text
a-local-first
z-local-second
```

Cross-node isolation remained intact:

```text
cloud-untouched report count = 0
```

Final assertions:

```text
report_count=2
a-local-first present=True
z-local-second present=True
cloud-untouched present=False
```

## Safety/non-claims

I39 does not add dependency-aware ordering, automatic polling/execution, retries, per-assignment argument vectors, cryptographic identity, or canonical governed execution.

The execution boundary remains explicit and fail-closed.

## Next

I40/146 derives dependency-aware deployment stages from graph/deployment semantics, rejects cycles and missing dependencies, and keeps execution explicit.
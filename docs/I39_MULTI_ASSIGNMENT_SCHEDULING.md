# I39 — Explicit Multi-Assignment Scheduling

- checkpoint: `I39/145`
- predecessor: `I38/144`
- status: `validated`
- branch: `agent/i39-multi-assignment-scheduling`

## Goal

I39 removes the I35 exactly-one-assignment execution limitation without introducing automatic execution. A configuration-bound Sentinel can explicitly execute a node slice containing two or more assignments through:

```text
--assignment batch --execute
```

The existing modes remain distinct:

```text
--assignment inbox           -> read-only discovery
--assignment auto --execute  -> exactly one assignment
--assignment batch --execute -> two or more assignments
```

## Scheduling contract

Batch execution uses a deterministic fixed policy:

```text
scheduling_policy=assignment_id_ascending
failure_policy=stop_on_first_failure
```

The Sentinel first validates the entire node-scoped deployment and derives a sorted assignment-id list. It then acquires, verifies, and compile-preflights every batch assignment before the first guest execution attempt. If any preflight fails, the batch fails closed before guest execution begins.

After successful preflight, assignments are attempted in sorted order. Each attempted assignment produces its own `LiveWasmExecutionReport`, which is published directly to dserver. A non-`executed` report is published and terminates the batch; later assignments are not attempted.

There are no hidden retries.

## Safety boundary

Batch mode:

- requires explicit `--execute`;
- forbids `--node-id` override;
- binds identity to configured `agent.node_id`;
- validates the returned node slice before scheduling;
- never schedules an assignment for another node;
- preserves `automatic_execution_allowed=false`;
- preserves `governed_execution_performed=false`.

The same scalar `--arg` vector is supplied to each batch assignment in this checkpoint. The live fixture therefore uses assignments with the same scalar signature.

## I35 compatibility

`--assignment auto` still calls the exact single-assignment selector and rejects slices containing zero or more than one assignment. A multi-assignment slice is accepted only by explicit `batch` mode.

## Live fixture

`scripts/fixtures/i39-multi-assignment-manifest.json` intentionally contains source-order assignments:

```text
z-local-second -> local-01
a-local-first  -> local-01
cloud-untouched -> cloud-01
```

Both local assignments use the validated `local-aggregate.wasm` artifact and `local_aggregate(i32,i32)->i32` function. Batch scheduling reverses the source order into deterministic lexicographic order:

```text
a-local-first
z-local-second
```

The cloud assignment never appears in the `local-01` batch.

## Regression — PASS

```text
DATUM fmt: PASS
DATUM test: 431 passed; 0 failed
DATUM build: PASS
```

## Live gate — PASS

The local inbox discovered two assignments and classified the slice as `multiple_assignments`.

The legacy single-assignment execution path remained fail-closed:

```text
--assignment auto --execute
-> rejected because local-01 received 2 assignments
-> live report count remained 0
```

Explicit batch execution:

```text
--assignment batch --execute --arg i32:24 --arg i32:6
```

Observed aggregate evidence matched the contract:

```text
mode=sentinel_bound_batch
node_id=local-01
scheduling_policy=assignment_id_ascending
failure_policy=stop_on_first_failure
assignment_count=2
attempted_count=2
executed_count=2
failed_or_blocked_count=0
status=executed
```

Execution order was deterministic despite the opposite source-manifest order:

```text
ordinal 1 -> a-local-first  -> result 30
ordinal 2 -> z-local-second -> result 30
```

dserver retained exactly two live reports for `local-01`, one per attempted assignment:

```text
a-local-first
z-local-second
```

The cross-node fixture assignment remained untouched:

```text
cloud-untouched report count = 0
```

Final compact report assertions:

```text
report_count=2
a-local-first present=True
z-local-second present=True
cloud-untouched present=False
```

## Non-claims

I39 does not provide:

- dependency-aware ordering (I40);
- automatic polling/execution;
- retries;
- per-assignment argument vectors;
- cryptographic Sentinel identity;
- canonical governed execution.

Checkpoint advances to `145/150`.
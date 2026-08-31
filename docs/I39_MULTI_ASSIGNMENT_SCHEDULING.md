# I39 — Explicit Multi-Assignment Scheduling

- checkpoint: `I39/145`
- predecessor: `I38/144`
- status: `implementation_ready_for_validation`
- branch: `agent/i39-multi-assignment-scheduling`

## Goal

I39 removes the I35 exactly-one-assignment execution limitation without introducing automatic execution. A configuration-bound Sentinel can explicitly execute a node slice containing two or more assignments through:

```text
--assignment batch --execute
```

The existing modes remain distinct:

```text
--assignment inbox          -> read-only discovery
--assignment auto --execute -> exactly one assignment
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

Both local assignments use the validated `local-aggregate.wasm` artifact and `local_aggregate(i32,i32)->i32` function. Batch scheduling must reverse the source order into deterministic lexicographic order:

```text
a-local-first
z-local-second
```

The cloud assignment must never appear in the `local-01` batch.

## Focused tests

I39 adds five focused tests on top of the I38 DATUM baseline of 426 tests:

```text
batch selector is explicit and distinct from auto/inbox
batch requires --execute
batch forbids node override
batch requires at least two assignments
batch order is assignment_id ascending
```

Expected minimum full-suite result:

```text
431 passed; 0 failed
```

The observed full-suite count remains authoritative.

## Regression gate

```powershell
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

## Planned live gate

1. start a fresh disposable controlled state;
2. start the existing artifact repository on `192.168.10.101:8090`;
3. POST `scripts/fixtures/i39-multi-assignment-manifest.json` to the controlled deployment-manifest endpoint;
4. verify local inbox reports `multiple_assignments` with two local assignments;
5. prove `--assignment auto --execute` rejects the two-assignment local slice before execution;
6. run `--assignment batch --execute --arg i32:24 --arg i32:6` on `local-01`;
7. require aggregate order `[a-local-first, z-local-second]`;
8. require two attempted/two executed/two reports;
9. require both results to be `30`;
10. require dserver live reports to contain exactly those two local assignment IDs and no `cloud-untouched` report.

## Non-claims

I39 does not provide:

- dependency-aware ordering (I40);
- automatic polling/execution;
- retries;
- per-assignment argument vectors;
- cryptographic Sentinel identity;
- canonical governed execution.

Checkpoint remains `144/150` until regression and live batch validation pass.
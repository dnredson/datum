# ADR-0015 — Runtime-kind dispatch seam (Stage 108)

- Status: Accepted and implemented (Stage 108, v2 -- closes two P1 findings
  from an independent review of v1; see "v2 closure" at the end).
- Decision refs: ADR-0002 (D-Map placement authority, unaffected by this
  stage), ADR-0006 (canonical digest, JCS+SHA-256), ADR-0011 (F2
  execution-content authorization, execution_generation/execution_projection_digest),
  ADR-0013/ADR-0014 (OCI catalog + runtime image identity, unaffected --
  Container behaviour is byte-for-byte unchanged by this stage).
- Scope: make the authoritative Stage-107 reconciliation path understand and
  dispatch more than one runtime kind (`container`, `native_process`)
  without implementing native-process execution. No process is spawned,
  signalled, or inspected by this stage. `target_nodes`/`target_stages`
  selection semantics are untouched (that transition is a later stage's
  scope). D-Map, D-Graph typed contracts, and the D-Deploy proposal/
  acceptance flow are untouched.

## Context

Before this stage, "runtime kind" was assumed to be `"container"` in at
least four independent places across two crates, none of which agreed with
each other by construction:

1. `dserver::core::operational_dgraph::validate_desired_service` --
   `!service.runtime.trim().eq_ignore_ascii_case("container")` (rejected
   anything else, worded "O2 vertical slice supports only container").
2. `dserver::core::service_artifact_registry::validate_artifact` --
   unconditionally required `artifact.runtime.container` to be `Some`.
3. `dserver::core::artifact_reconciliation_context::resolve_assignments_from_registry`
   -- `artifact.runtime.container.as_ref().ok_or_else(|| "... has no
   container runtime definition")?`, reached before any assignment could be
   constructed for any other kind.
4. `DATUM::agent::operational_reconciliation::validate_context` --
   `!service.runtime.eq_ignore_ascii_case("container")` (agent-side desired-
   service gate) AND, separately, `artifact.runtime_kind != "container"`
   (agent-side artifact-assignment gate) -- two independent string
   comparisons over two different structs, never kept in sync by
   construction.

A `NativeProcess`-kind artifact could not produce a reconciliation
assignment at all, let alone reach the agent, let alone be validated,
projected, or reported on. This is the P0-4 finding this ADR resolves.

## Decision: `RuntimeKind` is a dispatch seam, not a driver abstraction

```
enum RuntimeKind { Container, NativeProcess }
```

Wire strings: `"container"` / `"native_process"`, exact match only (no
case-insensitivity, no aliases). Unknown values fail closed with a named
error. Defined independently in `dserver::core::runtime_kind` and
`DATUM::agent::runtime_kind` -- no shared crate. Parity between the two
independent parsers is proven by (a) each crate's own test asserting the
literal wire-string constants, and (b) the cross-crate execution-projection
fixture tests (below), which exercise both parsers against the same
fixture content and require identical accept/reject behaviour and
identical resulting digests.

This is deliberately **not** a `RuntimeDriver` trait. The mature, correct
Container reconciliation path (`operational_reconciliation.rs`'s M1-M8
executors, `image_identity_verification.rs`, `execution_projection.rs`'s
container projection) is retained completely intact. The seam is exactly:

```
match runtime_kind {
    Container     => { /* existing path, unchanged */ }
    NativeProcess => { /* stable not_implemented_native_process_runtime stub */ }
}
```

A generic `RuntimeDriver` trait is deferred until a second real
implementation (Stage 109's native-process execution) exists to compare
against a genuinely shared interface -- introducing the abstraction now,
with only one real implementation and one stub, would be exactly the
premature-abstraction failure mode this project's review culture
(v1/v2/v3 architecture-roadmap self-reviews) already guards against.

## Early dispatch: before ANY Docker-specific observation

The agent's per-service loop (`run_operational_reconciliation_from_bootstrap`)
resolves the desired service's artifact assignment, then IMMEDIATELY
classifies the runtime dispatch via a pure function,
`classify_service_runtime_dispatch(service, assignment) ->
ServiceRuntimeDispatch { Error(String), NativeProcess, Container }`,
before `trusted_container_mapping_for_artifact`, `inspect_container`,
`inspect_container_exit_code`, `dependency_blockers_for_assignment`,
`resolve_effective_attachments`, the image-identity preflight, or any
Docker lifecycle/mutation logic is ever reached. A `NativeProcess`
classification (or a dispatch `Error`) short-circuits the loop iteration
via `continue`, pushing a stable, truthful report and never touching any
of the functions named above.

`classify_service_runtime_dispatch` takes no `Config` and performs no I/O
-- it cannot call `trusted_container_mapping_for_artifact` (which requires
`&Config`) even if it wanted to. This is a structural, type-level proof
that the native path cannot reach Docker, not merely a source-reading
claim; it is also directly unit-tested (see
`dispatch_never_yields_container_for_a_native_process_desired_service_regardless_of_assignment`),
which exhaustively checks every assignment shape (absent, matching,
mismatched, unknown) a `NativeProcess`-desired service could be paired
with and asserts none of them ever classify as `Container`.

The `NativeProcess` stub outcome (`build_native_process_stub_report`)
reports `execution_status = "not_implemented_native_process_runtime"`,
`host_mutation_performed = false`, `observed_before_running =
observed_after_running = false`, `trusted_container_name = trusted_image =
None`, and no `verified_image_identity` -- it never manufactures container
evidence for a native runtime. It never invokes Docker, never falls back
to Container, and never routes through `core::plugins::action_executor::ActionExecutorPlugin`
(the legacy shell-string-executing plugin engine this stage reuses nothing
from).

A dispatch `Error` (unknown runtime, or a desired/assigned runtime-kind
mismatch) produces its own distinct `execution_status =
"blocked_runtime_kind_contract_error"` report -- never confused with
`"not_implemented_native_process_runtime"` (contract understood, simply
unimplemented) or a generic unsupported-runtime message.

## Desired-state payload: `NativeProcessDeploymentSpecV1`

```
ServiceRuntimeSpec {
    kind: String,
    container: Option<ContainerDeploymentSpec>,       // unchanged
    native_process: Option<NativeProcessDeploymentSpecV1>,  // new, additive
}

NativeProcessDeploymentSpecV1 {
    process_name: String,
    entrypoint: String,
    args: Vec<String>,
    working_directory: Option<String>,
}
```

This is the ONE bounded choice Stage 108 makes (per the review's explicit
instruction not to require a field while calling it "finalized in Stage
109"): a minimal, complete, declarative desired-state payload. Every field
is required and tested within Stage 108 itself. It carries **no** content
hash -- `ServiceArtifactAcquisition` (already existing in
`ServiceArtifactControlPlane`, `{kind, source, revision, sha256}`) remains
the single future owner of native executable content identity; Stage 109
is what actually consumes `acquisition.sha256` for verification. It
carries **no** observed runtime state (PID, process start time, current
process state) -- see the next section.

Runtime-kind payload invariants (enforced at every validation layer:
`validate_artifact`, `validate_assignments`, the agent's `validate_context`):
`Container` requires `container: Some`, `native_process: None`;
`NativeProcess` requires `native_process: Some`, `container: None`. Mixed
or empty combinations are rejected. Unknown `kind` values fail closed.

## Desired vs. observed: no `ManagedProcessRuntimeState` exists yet

Deployment spec is DESIRED STATE. A future `ManagedProcessRuntimeState`
(PID, `/proc/<pid>/stat` start time, observed executable, process state)
is OBSERVED LOCAL RUNTIME STATE, and belongs to D-Node local persisted
state plus D-Monitor evidence/history -- never to `ServiceArtifact`,
`DGraph`, `DMap`, or the execution projection's desired contract. Stage
108 introduces no such type at all: no `libc` dependency, no signal
wrapper, no `/proc` parser, no process spawn. All of that is Stage 109's
scope, where it will actually be exercised against a real process.
`execution_generation` is defined and incremented only in response to a
change in `NativeProcessDeploymentSpecV1` (or, for containers,
`ContainerDeploymentSpec`) -- never in response to an observed process
restart, exactly mirroring how a container's own restart-with-unchanged-
image/command never bumps its generation either.

## Execution projection: a separate type, not a `null` sibling

`ArtifactExecutionProjectionV1` / `compute_projection` / `projection_digest`
/ `compute_projection_and_digest` (both crates) are left byte-for-byte
unchanged by this stage. ADR-0006 point 2 makes an added, always-`null`
sibling key digest-significant; bolting `native_process: null` onto the
existing container projection would perturb `execution_projection_digest`
(and therefore `execution_generation`) for EVERY existing container
artifact the moment this stage lands, with no genuine content change
behind it -- exactly the outcome this ADR must avoid.

Instead, a separate type exists on both sides:

```
NativeProcessExecutionProjectionV1 {
    projection: "datum.native-process-execution-projection/1",
    artifact_id, service_id, artifact_kind, runtime_kind,
    process_name, entrypoint, args, working_directory,
    dependencies, bindings, secret_refs, secret_bindings, wasm_safety,
}
```

`dserver::core::execution_projection::compute_native_projection`/
`native_projection_digest`/`compute_native_projection_and_digest` and
`DATUM::agent::execution_projection::compute_native_projection`/
`native_projection_digest`/`recompute_native_digest` are independent,
hand-written mappers -- proven equal over equivalent content by a real
shared fixture
(`scripts/fixtures/datum-contracts/vs1/execution-projection.cross-crate-mapper.native-process-stub.json`),
exercised by a Rust test in each crate that deserializes the fixture into
that crate's real types, runs the REAL mapper function, and asserts the
result equals the fixture's `expected_projection` and
`expected_projection_digest_sha256` -- exactly the same discipline the
existing `execution-projection.cross-crate-mapper.mqtt-broker.json` fixture
already established for Container.

Registry generation bookkeeping (`create`, `update`,
`migrate_legacy_registry_state`, `migrate_legacy_artifact_schema_versions`,
`validate_registry_state`'s digest-consistency check) now calls a single
runtime-dispatched entry point,
`execution_projection::compute_execution_projection_digest`, which routes
to the container-only computation for `Container` (byte-for-byte identical
behaviour to calling `compute_projection_and_digest` directly) and to the
native computation for `NativeProcess`. `apply_resolved_image_identity`
(OCI pin application) is left calling `compute_projection_and_digest`
directly and unchanged, since that function is only ever reachable for an
artifact that already has `runtime.container: Some` -- it is inherently
Container-only, and does not need the dispatcher.

**Hard requirements verified by test**: the existing container fixture
projection JSON and digest are unchanged (`cross_crate_fixture_produces_the_expected_projection_and_digest`,
unmodified, still green); a `NativeProcessDeploymentSpecV1` field change
(`process_name`/`entrypoint`/`args`/`working_directory`) changes the
native projection digest (four dedicated tests, one per field, in both
crates); DServer's and DATUM's independently computed native digests are
identical for equivalent content (the cross-crate fixture test); the
native projection never serializes any observed-state-shaped key (a
regression-guard test asserting the absence of `pid`/`proc_start_time_ticks`/
`observed_executable`/`state`/`started_at_utc` in the serialized JSON).

## `target_nodes`/`target_stages`: untouched by this stage

`resolve_assignments_from_registry`'s existing
`artifact.control_plane.target_nodes.iter().any(|node| node ==
&slice.node_id)` selection check is **not modified** by this stage. Making
an accepted `datum.dmap/1` authoritative for placement, and demoting
`target_nodes`/`target_stages` to an eligibility-only check, is a single,
atomic change reserved for the later stage that implements that flow --
splitting the removal of the old gate from the activation of the new one
across two stages (or across two commits within one stage) would leave an
intermediate release with no functioning placement carrier at all. A
dedicated regression test
(`target_nodes_selection_behaviour_is_unchanged_by_stage108`) asserts the
existing accept/reject behaviour is preserved exactly.

## Config: no native execution policy added

`DispatchConfig`'s `allow_container_execution`/`container_mappings` are
untouched. Stage 108's `NativeProcess` path performs no mutation, so no
`allow_native_process_execution`-equivalent policy is introduced merely to
support a stub that can never act on it. See "Stage 109 handoff" below for
the explicit, non-deferred statement of what Stage 109 must add before any
native execution is enabled.

## Stage 109 handoff (recorded here so it is not lost)

Before real native-process execution is enabled:

1. `ServiceArtifactAcquisition.sha256` (already existing on
   `ServiceArtifactControlPlane`) must become part of the NATIVE execution
   projection (`NativeProcessExecutionProjectionV1`), never the container
   one -- this is the acquisition-completeness model `required_completeness_gaps`
   currently deliberately omits for `NativeProcess` (Stage 108 gives it no
   image-completeness logic at all, rather than misapplying the
   container-shaped one).
2. Local node policy must gain an explicit, non-deferred
   `allow_native_process_execution`-equivalent gate plus a controlled
   managed-process/artifact root, mirroring `allow_container_execution`'s
   role today -- DServer authorizing a native service must never, by
   itself, be sufficient for a node to execute it.
3. `ManagedProcessRuntimeStateV1` (service_id, artifact_id, content_sha256,
   pid, proc_start_time_ticks, observed_executable, args_digest,
   started_at_utc, state), the `libc`-based `SIGTERM`/`SIGKILL` signal
   wrapper (declared as an explicit direct Cargo dependency, not assumed
   transitively available), and the `/proc/<pid>/stat` start-time parser
   all belong to Stage 109, where they are actually exercised against a
   real spawned/adopted process -- never introduced early as unused stubs.

## Non-goals (explicit)

No native process is spawned, signalled, or inspected. No `libc`
dependency exists in this stage. No `ManagedProcessRuntimeState` type
exists in this stage. `target_nodes`/`target_stages` semantics are
unchanged. `datum.dmap/1`/`datum.dgraph/1`/`datum.dcontinuum/1` typed
contracts are untouched. `core::plugins::action_executor::ActionExecutorPlugin`
gains no new caller. Container behaviour is byte-for-byte unchanged.

## v2 closure (independent review findings)

An independent review of v1 approved the overall design and found two P1
findings plus one P2 observation, closed as follows. No redesign, no
Stage 109 work, no D-Map/placement touch, and no container-executor
refactor occurred in this pass.

**P1-1 (DServer did not enforce desired-runtime / artifact-runtime
parity)**. Before this fix, the agent alone detected a
`service.runtime`/`assignment.runtime_kind` disagreement
(`classify_service_runtime_dispatch`); DServer itself could resolve and
validate an internally contradictory context (e.g. a desired `"container"`
service joined to a `"native_process"` artifact) and let it travel all the
way to the agent before anything caught it. Fixed in two places, both
failing closed with a stable `runtime_kind_mismatch` message:

1. `resolve_assignments_from_registry` now parses both
   `service.runtime` and the selected artifact's `runtime.kind`
   immediately after artifact selection, before constructing any
   assignment, and rejects disagreement (or either side being
   unparseable).
2. `validate_artifact_reconciliation_context` independently re-checks the
   same parity for every desired-service/assignment pair it matches --
   defense in depth against an already-built context that was tampered
   with or constructed by a future caller that bypasses
   `resolve_assignments_from_registry`.

`target_nodes`/`target_stages` semantics were not touched by this fix (the
parity check is a separate, independent gate from the eligibility-
selection check, and both regression tests -- parity and target_nodes
invariance -- pass together).

**P1-2 (native wire validation weaker than the registered contract)**.
`service_artifact_registry::validate_native_process_deployment_spec`
(the registry-level validator, run at artifact create/update time)
enforces bounded, NUL-free limits on every `NativeProcessDeploymentSpecV1`
field. The two DOWNSTREAM (wire) validators --
`artifact_reconciliation_context::validate_assignments` (DServer) and
`operational_reconciliation::validate_context` (agent) -- previously
enforced only a subset (non-empty `process_name`/`entrypoint`, with the
agent additionally checking `args`/`working_directory` bounds but neither
side checking `process_name`/`entrypoint` length or NUL-freedom). Fixed by:

- DServer: `validate_native_process_deployment_spec` was made
  `pub(crate)` and is now called DIRECTLY by
  `artifact_reconciliation_context::validate_assignments` -- intra-crate
  reuse of the exact same function, not a reimplementation.
- Agent: a small, independently hand-written mirror function,
  `native_process_deployment_spec_within_stage108_bounds`, enforces the
  identical limits (no shared crate, per this project's established
  cross-crate wire-parity discipline -- same discipline `RuntimeKind`
  itself already uses).

Both downstream validators now reject: a NUL byte or over-256-character
`process_name`; a NUL byte or over-4096-character `entrypoint`; more than
64 `args` entries or any single argument over 4096 characters or
containing a NUL byte; and a present-but-blank, NUL-containing, or
over-4096-character `working_directory`. A well-formed payload still
passes on both sides.

**P2 (missing-assignment semantics for `classify_service_runtime_dispatch(native_process, None)`)**.
Recorded as explicit Stage-109-before-real-mutation debt, per the review's
own instruction not to expand Stage 108 substantially for this
observation. A `datum_artifact_reconciliation_context` that DServer itself
produced can never actually exhibit this shape today: `validate_context`
(agent) and `validate_artifact_reconciliation_context` (DServer) both
already require `context.artifacts.len() == context.desired_services.len()`,
and DServer's `resolve_assignments_from_registry` never emits an
assignment-less desired service. The gap is theoretical under the current
server-produced-context invariants, but `classify_service_runtime_dispatch`
itself does not independently re-derive "every desired service must have a
matching assignment" the way DServer's context validator does -- if a
future stage (Stage 109, or a stricter one-to-one agent-side match)
changes that invariant, `"not_implemented_native_process_runtime"` would
be a misleading explanation for "no artifact was ever assigned" and should
become a distinct contract-error outcome at that time. Not fixed in this
pass, by explicit instruction.

# ADR-0016 — Native process runtime v0.1 (Stage 109)

- Status: Accepted and implemented (Stage 109, v1).
- Decision refs: ADR-0015 (runtime-kind dispatch seam, Stage 108 -- frozen;
  this stage replaces its `not_implemented_native_process_runtime` stub
  arm only, nothing else in ADR-0015 changes), ADR-0006 (canonical digest,
  JCS+SHA-256), ADR-0011/ADR-0012 (F2 execution-content authorization,
  finite lease -- reused directly, not reinvented).
- Scope: real, governed Linux native-process execution replacing Stage
  108's stub. `target_nodes`/`target_stages` placement semantics, D-Map,
  canonical D-Graph, deterministic placement, and unified D-Monitor are
  untouched -- none of that is Stage 109 work.

## Target: direct Linux process management, no Docker, no systemd

Stage 109's runtime target is a single Linux D-Node with no Docker
installed. A native process is spawned, observed, signalled, and adopted
directly by SmartSentinel via `std::process::Command` and a minimal
`libc`-based signal wrapper -- never via systemd unit generation, Kubernetes,
Nomad, a Docker wrapper, or shell strings. `libc` is now an explicit direct
dependency of `DATUM/Cargo.toml` (not assumed transitively available),
resolved offline against the version already present in the lock file.
Typed `program`/`argv`/`environment`/`working_directory` only -- no
`sh -c`, no PowerShell, no reuse of `core::plugins::action_executor::ActionExecutorPlugin`
or any other arbitrary-command-execution path.

The real runtime implementation is Linux-only
(`#[cfg(target_os = "linux")]`): pure decision logic (dispatch
classification, the reconciliation-action decision function, local-policy
evaluation, path containment, content hashing, the `/proc/<pid>/stat`
parser) is platform-independent and unit-tested on every development host;
real signal delivery, `/proc` reads, and process spawning are gated to
Linux. On any other platform, a real-mutation attempt fails closed with
the stable `native_process_runtime_unsupported_platform` outcome --
SmartSentinel never pretends to adopt or signal a process it cannot
actually observe.

## Closing Stage 108's P2 debt: missing assignment is a contract error

Stage 108 deliberately deferred `classify_service_runtime_dispatch(native_process,
None)` yielding the native path even with no matching artifact assignment
-- harmless while real execution was impossible. That is unacceptable now
that Stage 109 can mutate the host. `ServiceRuntimeDispatch` gained a new
variant, `MissingAssignment`, returned whenever a desired `native_process`
service has no matching artifact assignment; the per-service loop routes
it to `native_process_runtime::missing_artifact_assignment_report`
(`execution_status = "blocked_missing_artifact_assignment"`, no mutation),
distinct from `Error` (a runtime-kind mismatch between two present values)
and from every real execution outcome. A desired service enters real
native reconciliation only when exactly one matching artifact assignment
exists.

## Acquisition enters the wire snapshot and the native projection

`ServiceArtifactControlPlane.acquisition` (`ServiceArtifactAcquisition
{kind, source, revision, sha256}`, already existing since Stage 107) now
travels the full path: `ServiceArtifact` -> `ArtifactReconciliationAssignment`
(dserver) -> `OperationalArtifactAssignment` (agent, via a hand-mirrored
`OperationalServiceArtifactAcquisition` type -- no shared crate). Populated
if and only if `runtime_kind == "native_process"`; `None` for `Container`,
and never added to `ArtifactExecutionProjectionV1` (Container's execution
projection and digest remain byte-for-byte unchanged -- confirmed by the
existing container cross-crate fixture test, unmodified and still green).

`acquisition.sha256` is the SINGLE authoritative executable content hash.
`NativeProcessDeploymentSpecV1` gains no hash field of its own (no
`executable_sha256`, no `content_digest`) -- one executable byte sequence,
one declared identity, owned in exactly one place.

`NativeProcessExecutionProjectionV1` (both crates) gained one new field,
`acquisition_sha256: Option<String>`, populated from
`control_plane.acquisition.sha256`. This is a hard gate: before this field
existed, the authorized bytes could change (a new `acquisition.sha256`)
while the native projection digest stayed identical, which is forbidden --
content identity is execution-relevant for a native process exactly as
image identity is for a container. ADR-0006 point 2 applies: absence
serializes as JSON `null`, never omitted. Cross-crate digest parity
(dserver's and the agent's independently computed digests over equivalent
content) is proven by an updated shared fixture
(`scripts/fixtures/datum-contracts/vs1/execution-projection.cross-crate-mapper.native-process-stub.json`),
exercised by a real-mapper test on each side.

## Registry-level requirement, not a deferred completeness gap

Unlike a container image (which may exist unpinned and become
required-complete only later via `oci_image_pinning`'s async resolution
flow), Stage 109 builds no equivalent async "resolve native content
later" pipeline: the executable's content hash is declared by the operator
at artifact-authoring time. `service_artifact_registry::validate_artifact`
therefore HARD REQUIRES `control_plane.acquisition.sha256` to be a valid
64-character hex digest whenever `runtime.kind == "native_process"` --
this is a creation-time requirement, not a deferred gap. Both downstream
(wire) validators (`artifact_reconciliation_context::validate_assignments`
on DServer, defense in depth on the agent's `stage107a_enforcement::required_completeness_gaps`
and `native_process_runtime`'s own independent check) re-verify the same
requirement, and `required_completeness_gaps` on both sides additionally
records a stable `unpinned_native_process_content` gap as evidence -- OCI
image-completeness logic never runs against a `native_process` assignment
(runtime-dispatched, mirroring the existing Container/NativeProcess split
in that function). A real native process therefore never executes without
an exact, valid, matching content hash, independent of pinning mode.

## Native projection migration: no live registry affected today

Adding `acquisition_sha256` changes the Stage 108-era native projection
digest for any hypothetical already-persisted native artifact record.
Deliberately NOT handled by bumping `SERVICE_ARTIFACT_SCHEMA_VERSION` and
reusing `migrate_legacy_artifact_schema_versions`: that mechanism recomputes
EVERY artifact's generation unconditionally on migration (by design, per
its own doc comment), which would incorrectly bump every CONTAINER
artifact's `execution_generation` too, even though the container
projection/digest is completely unaffected -- explicitly forbidden by this
stage's own scope. No live persisted DServer registry with Stage
108-era native artifact records exists in this repository (native_process
was introduced in Stage 108 and never deployed live), so this is a
forward-looking policy decision, not a live migration performed here.
Should a future deployment ever have persisted native artifacts before
this change: `validate_registry_state`'s EXISTING digest-consistency check
(recomputes and compares `execution_projection_digest` on every load)
will correctly detect the mismatch and refuse to load the stale state
(fail closed), rather than silently trusting a stale digest. An operator
would then re-`update()` the affected native artifacts (an already-
supported, ordinary operation) to recompute generation/digest under the
new projection shape. This is the "explicitly reject the stale record with
a clear migration path" option, chosen because it requires no new code and
carries zero risk of the forbidden global-container-generation-bump
outcome.

## Local node execution policy

DServer authorization ALONE is never sufficient to spawn a native process.
`DispatchConfig` gained two new fields:

```
allow_native_process_execution: Option<bool>,  // default: denied
native_process_root: Option<String>,           // required, absolute Linux path
```

Real mutation requires `dispatch.enabled == true`,
`allow_native_process_execution == true`, AND a configured, absolute
(`/`-rooted) `native_process_root`. Deliberately NOT `allow_container_execution`
reused, and `container_mappings` is never consulted for native execution --
Docker availability plays no role in this decision at all
(`evaluate_local_native_policy`).

## Managed filesystem layout and content-addressed materialization

`native_process_acquisition::ManagedLayout` implements a deterministic
layout under `native_process_root`:

```
<root>/artifacts/<content-sha256>/<entrypoint>   (content-addressed, atomic materialize)
<root>/state/<stable-service-key>.json           (ManagedProcessRuntimeStateV1)
<root>/logs/<stable-service-key>/{stdout,stderr}.log
```

Every path is derived from an already-validated content-sha256, a stable
service key computed from `(project_id, node_id, service_id)`, or the
artifact's own `entrypoint`/`working_directory` -- the latter two are
ALWAYS validated for containment (`resolve_contained_path`, rejecting
absolute paths, `..` traversal, NUL bytes, and empty/invalid components)
before being joined onto any path. `entrypoint` is a relative path INSIDE
the content-addressed artifact directory (never a fixed filename, never an
artifact-controlled absolute host path) -- the executable actually invoked
is `<artifact directory>/<entrypoint>`. `working_directory`, when declared,
is likewise a relative path inside that same directory; when absent, the
working directory IS the artifact directory. Materialization is atomic
(temp file + rename), idempotent (identical content is reused, never
rewritten), and self-verifying (the final materialized bytes are re-hashed
and must match before the call returns; a corrupted on-disk copy is never
executed and never silently overwritten).

Single-file v0.1: one acquisition represents one executable/script. Archive
extraction, dependency installation, and language-specific package
management are explicitly deferred. A script (e.g. Python) may participate
if it is itself directly executable (valid shebang + execution permission)
-- SmartSentinel never invents shell interpretation on top of it.

## Native execution projection binds content identity; desired/observed stays frozen

The Stage 108 desired/observed separation is unchanged and reinforced:

- DESIRED: `NativeProcessDeploymentSpecV1`, `ServiceArtifactAcquisition`,
  `NativeProcessExecutionProjectionV1` (now including `acquisition_sha256`).
- OBSERVED LOCAL RUNTIME STATE: `ManagedProcessRuntimeStateV1
  {service_id, artifact_id, execution_generation, execution_projection_digest,
  content_sha256, pid, proc_start_time_ticks, observed_executable,
  args_digest, started_at_utc, state}` -- persisted locally by the agent,
  NEVER added to `ServiceArtifact`, `NativeProcessDeploymentSpecV1`,
  `DGraph`, `DMap`, or any execution projection. An ordinary process
  restart (or SmartSentinel-restart adoption) changes
  `pid`/`proc_start_time_ticks`/`started_at_utc`/`state` freely; none of
  that increments `execution_generation` or changes
  `execution_projection_digest` -- both remain server-owned, computed
  solely from desired-state fields.

## PID + `/proc` starttime identity; signal lifecycle

Process identity for adoption requires PID + `/proc/<pid>/stat` field 22
(`starttime`) + observed executable + argument digest, ALL matching the
persisted `ManagedProcessRuntimeStateV1` (`native_process_state::check_identity`,
a pure function tested exhaustively for every combination: confirmed match,
process absent, PID reused with differing starttime, executable mismatch,
args mismatch -- the last three NEVER adopt and NEVER signal the observed
PID). The `/proc/<pid>/stat` parser correctly locates the `comm` field's
closing parenthesis (which may itself contain spaces and parentheses) via
`rfind(')')` before indexing subsequent fields, rather than naively
splitting the whole line on whitespace from the start.

Signal delivery (`native_process_runtime::linux::send_signal`) is a
minimal, direct `libc::kill` wrapper: SIGTERM, a bounded poll wait (a fixed
local constant, `GRACEFUL_STOP_TIMEOUT`, never an artifact-controlled
value), fresh identity re-verification immediately before ANY signal
(including before the eventual SIGKILL), and SIGKILL only if the process
is still confirmed present after the timeout. A process disappearing
between checks is treated as a successful, idempotent stop -- never an
error. An identity mismatch at any point (PID reused, executable changed)
means the observed process is treated as absent and is NEVER signalled.

## Recovery/adoption requires no in-memory handle

A fresh SmartSentinel process recovers purely from persisted
`ManagedProcessRuntimeStateV1` plus real `/proc` reads -- never an
in-memory `Child` handle. `native_process_runtime::observe_persisted_identity`
is the one function both the ordinary reconciliation loop and a
post-restart recovery cycle use identically.

## Native mutation boundaries (N1-N6)

Parallel to Container's M1-M8, never reusing those identifiers:

```
N1  materialize/replace verified artifact bytes
N2  executable-permission mutation
N3  spawn native process
N4  SIGTERM managed process
N5  SIGKILL managed process (only recorded if actually needed)
N6  remove managed artifact/state (rollback)
```

Every boundary reuses the EXISTING `check_boundary`/`MutationBoundaryContext`/
`MutationBoundaryRecord`/`LeaseGuard` mechanism directly (`check_boundary`
and `MutationBoundaryContext` were made `pub(crate)` for this reuse, bodies
otherwise untouched) -- no parallel authorization mechanism was invented.
The finite lease is checked FRESH before each individual boundary, never
from a single cycle-start timestamp. `MutationBoundaryRecord`'s doc comment
no longer implies only M1-M8 can appear -- N1-N6 are equally valid
`boundary` values.

Partial-mutation truthfulness: if a later native step fails after an
earlier one succeeded (e.g. spawn succeeds but state persistence fails),
`host_mutation_performed` reflects the REAL host state, never `false` when
a mutation actually occurred. If state persistence fails right after a
successful spawn, the agent performs bounded compensation (terminates the
just-spawned child via the live handle) so it never knowingly leaves an
unmanaged process running, and reports the compensation truthfully.

## Reconciliation semantics

`decide_native_reconciliation_action` (pure, exhaustively unit-tested) maps
observed facts to exactly one action: `AlreadyConverged` (exact process
running, no drift -> no mutation), `Spawn` (no persisted state),
`Respawn` (persisted state but process gone / PID reused / identity
mismatch -> never adopts the wrong process, always respawns fresh),
`RestartForDrift` (confirmed identity but desired content/projection
differs -> governed stop then respawn), `ContentUnpinned` (hard block,
independent of pinning mode), `Rollback`/`RollbackNoOp`,
`UnsupportedCompletedDesiredState`, or `ObserveOnly` (platform unsupported,
local policy denied, unauthorized, execute_requested false, or an expired
lease/generation-mismatch gate -- in every `ObserveOnly` case, no mutation
occurs). `desired_state != "running"` (e.g. `"completed"`) is explicitly
NOT implemented for native processes in Stage 109 -- a stable
`unsupported_native_process_completed_desired_state` outcome is returned,
never coerced to running, never silently claimed converged.

## Rollback/removal

A rollback request stops the exact confirmed managed process (never an
unrelated PID) and removes the local runtime-state record. Stage 109
deliberately does NOT delete the content-addressed executable bytes on
rollback -- another service could in principle reference the same content
hash, and this stage's reference-counting model does not yet prove that is
safe. Rollback therefore removes the execution instance/state only;
content garbage collection is deferred, which is strictly safer than risky
shared-content deletion.

## Stage 113 (unified D-Monitor) handoff

Stage 109 exposes exactly the observation primitive Stage 113 needs to
reuse rather than reimplement native process identity:
`native_process_runtime::observe_persisted_identity` plus
`ManagedProcessRuntimeStateV1`'s `pid`/`content_sha256`/`state` fields. No
full CPU-percentage/RSS sampling infrastructure is built now -- that is
explicitly Stage 113's job.

## Deferred (explicitly, not silently)

systemd integration; an OCI-like native artifact registry; archive/package
extraction; Python virtualenv/package management; native process groups/
trees beyond what N1-N6 strictly need; Windows native-process management;
D-Map; canonical D-Graph; placement; adaptive migration; unified
D-Monitor; GNN export; a generic `RuntimeDriver` framework (Stage 108's
dispatch-seam-not-driver decision stands unchanged -- two real
implementations, Container and NativeProcess, still do not share an
abstraction beyond the plain `RuntimeKind` enum and dispatch match).

## v2 addendum: fault-injection hardening pass

A follow-up autonomous pass (v2) exercised the v0.1 implementation above
against an explicit state-machine model and a deliberate fault-injection
matrix, and fixed the following real defects it found (full detail in the
v2 review bundle's audit set):

- **Duplicate-spawn-on-unconfirmed-stop (the most serious finding).**
  `RestartForDrift`/`Rollback` originally proceeded to spawn a new process
  (or remove runtime state) as soon as a stop was *attempted*, regardless
  of whether it was actually *confirmed*. `StopResult` now carries a
  separate `confirmed_stopped` field, and `governed_stop` only reports it
  `true` on positive evidence (identity absent/mismatched before any
  signal, or a fresh post-signal observation confirming exit) -- SIGKILL
  itself is no longer trusted as proof of termination without a bounded
  confirmation poll. `RestartForDrift`/`Rollback` now block, unconfirmed,
  rather than ever spawning a duplicate or erasing recoverable identity
  state.
- **Identity-mismatch was routed to `Respawn` instead of blocking.** A
  live PID whose executable/args no longer match the persisted record
  (still the same OS process instance -- PID+starttime matched) was
  previously treated the same as an absent process and respawned over.
  It now blocks (`blocked_native_process_identity_mismatch`), matching
  the state-machine invariant that an unconfirmed-but-possibly-still-live
  identity must never be spawned over.
- **A procfs read failure after PID+starttime matched was silently
  treated as a match.** A new `IdentityCheck::Unavailable` outcome
  distinguishes "identity read raced away" from both confirmation and
  disproof; args identity is now genuinely re-verified from
  `/proc/<pid>/cmdline` at observation time (previously always `None`,
  i.e. never actually checked in production).
- **Local mutation policy and observation were conflated.** Turning off
  `allow_native_process_execution` previously also blinded the agent to
  whether its own managed process was still running. Observation now uses
  a separate, weaker gate (`dispatch.enabled` + a valid root only);
  mutation (including rollback) still requires the full policy.
- **Two concurrent reconciliation attempts for the same service could
  both spawn.** A non-blocking, per-service `fs2` advisory lock closes
  this for any caller shape this codebase does not already serialize
  (the long-running loop already has its own instance lock; the one-shot
  CLI did not).
- **A spawned child was never reaped in-process.** `std::mem::forget`
  guaranteed a zombie entry for any child that exited while the agent
  stayed up. A minimal single-purpose reaper thread (spawned only after
  durable state exists, owns exactly one `Child`, waits exactly once) now
  reaps it; recovery after a real agent restart is unaffected (unrelated
  mechanism, unchanged).
- **A native artifact's environment was never sanitized.** The spawned
  child previously inherited this agent's full process environment.
  `env_clear()` plus only the artifact's declared `bindings` now apply. A
  non-empty `secret_refs` on a native artifact fails closed
  unconditionally (Stage 109 v0.1 still does not implement native secret
  injection) rather than being silently ignored outside `required` mode.
- **Symlink escape and permission-drift were unhandled.** Managed-root
  path resolution now rejects a symlinked final path or symlinked
  ancestor. Materialization now distinguishes reuse-as-is, permission-
  repair-only (no content rewrite), fresh write, and governed corruption
  repair (content-addressing makes replacing corrupted bytes with the
  already-verified correct ones safe) -- each crossing only the N-boundary
  it actually needs.
- **Persisted state was trusted without validation.** A syntactically
  valid but semantically nonsensical runtime-state record (bad pid, bad
  hash shape, unknown schema) is now rejected
  (`invalid_managed_process_state`) rather than risking adoption.
- **The real Linux code path had never actually been compiled.** Because
  this stage's real implementation is developed on a non-Linux host,
  `#[cfg(target_os = "linux")]` code is never type-checked by an ordinary
  `cargo check` here. A manual symbol-by-symbol audit (a real
  `x86_64-unknown-linux-gnu` cross-check was attempted but blocked by the
  absence of a C cross-toolchain for this stage's TLS/crypto dependencies
  on this host) found and fixed several missing imports
  (`std::path::Path`, `chrono::Utc`,
  `native_process_state::check_identity`, `compute_args_digest`,
  `MANAGED_PROCESS_RUNTIME_STATE_SCHEMA`, `PROCESS_STATE_RUNNING`) that
  would have made the real spawn/stop path fail to compile on the actual
  target platform. This is recorded as an open risk: a real Linux
  CI/build step is the only way to close it with certainty, and is
  recommended before Stage 110.

No change to the frozen v0.1 acceptance criteria, the N1-N6 boundary
identifiers, the projection shape, or any Stage 107A/B/108 invariant --
this pass is entirely corrective, within the v0.1 scope already accepted
above.

## v3 addendum: independent-review source-vs-audit closure

An independent review of the v2 bundle found that several v2 AUDIT
DOCUMENTS claimed properties the packaged SOURCE did not actually
implement. Per the review's explicit mandate ("source is authoritative;
fix the code first, then regenerate every audit from the final code"),
this pass fixed the code and only then updated the audits. Full detail,
including exact before/after behavior and the regression test(s) for
each item, is in the v3 review bundle's `source-vs-audit-closure-v3.txt`.
Summary of what changed:

- Rollback now passes through the SAME full governance gate as every
  other mutation (`execute_requested`, `control_plane_authorized`,
  local native policy, a valid generation/projection binding) before it
  may act -- previously only the local policy check applied. Content/
  hash completeness deliberately does NOT gate rollback (removing an
  already-owned process must not require its executable source to still
  resolve).
- Rollback identity handling no longer collapses `ProcessAbsent`/
  `PidReused`/`Zombie` and `ExecutableMismatch`/`ArgsMismatch`/
  `Unavailable` into one `RollbackNoOp` variant. Three distinct outcomes
  now exist: a true no-op (no persisted state at all, zero mutation), a
  governed stale-state cleanup (a real, N6-gated removal), and an
  incomplete rollback (never signals, never removes state).
- N6 (runtime-state removal) is now a real `check_boundary` call,
  freshly checked immediately before every actual removal -- previously
  declared but never enforced.
- The stable service-filesystem key is now derived from the
  authoritative `OperationalReconciliationContext.project_id`/`node_id`
  the caller passes in, never from whether an authorization happens to
  be present this cycle (previously `""` when unauthenticated, and
  always the literal string `"node"`).
- `kill(pid, 0)`'s `EPERM` no longer means "absent" -- a new
  `PidPresence` type distinguishes a real `ESRCH` disappearance from
  "exists but unsignalable" and from a genuine observation failure. A
  `/proc/<pid>/stat` read failure is `ProcessAbsent` only for a real,
  concrete `ENOENT`; every other failure is `Unavailable`.
- A zombie (`/proc/<pid>/stat` state `Z`) is its own `IdentityCheck::Zombie`
  outcome -- never "running/converged", never signaled, safe for
  governed stale-state cleanup.
- The stop poll loops no longer treat `!= Confirmed` as proof of
  termination -- `Unavailable`/a mismatch mid-poll now correctly reports
  "stop incomplete", never a blind escalation to SIGKILL and never a
  false "confirmed stopped".
- Post-spawn identity is read from `/proc` alone, never fabricated: a
  failed `/proc/<pid>/exe` read no longer falls back to the desired
  executable path, and the persisted args digest is now computed from
  the REAL observed `/proc/<pid>/cmdline`, never the desired argv
  directly.
- `execution_status` for spawn/respawn/restart-for-drift no longer uses
  "some mutation happened" as a proxy for "the spawn succeeded" -- only
  `observed_after_running` (true exclusively once a real process was
  actually confirmed running) may report success; a partial mutation
  followed by a block now correctly reports the real block reason.
- N1 (materialize) no longer sets the executable permission bit as part
  of the write -- N2 is now a genuinely separate, freshly-checked step
  (`repair_executable_permission`, which also verifies the bit was
  actually set) for both a fresh write and a permission-only repair.
- A non-blocking per-service lock is now acquired ONLY when the decided
  action may actually mutate the host -- a purely observational cycle
  never creates a lock file, closing an observe-only-mode filesystem
  side effect; the lock path itself is resolved through the same
  symlink-safe `ManagedLayout::state_path` machinery as every other
  managed path.
- `acquisition.kind` must be exactly `native_process_executable` (never
  silently treated as a local file for any other declared kind), and
  `acquisition.source`/`entrypoint`/`working_directory` are now validated
  as POSIX-shaped paths (absolute for `source`, relative with no `.`/
  `..` for `entrypoint`/`working_directory`) using host-independent pure
  string logic -- both at DServer authoring/assignment-resolution time
  and as agent-side defense-in-depth -- never `std::path`-based, since
  DServer may run on a non-Linux host. The native content-hash contract
  is lowercase-hex only, matching the agent's case-sensitive hashing
  (DServer previously accepted uppercase).
- Partial filesystem mutations (a logs directory or a temp materialization
  file created before a later step failed) are no longer reported as
  `host_mutation_performed = false`.

## v4 addendum: final source closure

A further independent review of the v3 bundle found four remaining
concrete source defects (no new scope, no redesign). Full before/after
detail and regression tests: the v4 review bundle's
`final-source-closure-v4.txt`.

- **SIGTERM error handling confirmed a stop for every errno, not only
  `ESRCH`.** `linux::send_signal` already distinguished `ESRCH` from
  other failures for the string it returned, but `governed_stop`'s
  SIGTERM call site treated ANY error (including `EPERM`/`EACCES`/an
  unexpected syscall error) as a confirmed, idempotent stop. Fixed: a
  new typed `linux::SignalError { ProcessAbsent, Failed(String) }`
  replaces the string return; both the SIGTERM and SIGKILL call sites in
  `governed_stop` now match on it explicitly, and only `ProcessAbsent`
  (`ESRCH`) is ever treated as confirmed.
- **`ManagedLayout::materialize()` could still perform a chmod outside
  N2.** Its internal reclassification (a defense against another
  service, sharing the same content-addressed bytes, having already
  supplied correct-but-non-executable content between the caller's
  classification and this call) called `set_executable_permissions`
  directly, under N1's own lease check. Fixed: `materialize()` now NEVER
  sets executable permissions under any circumstance; this
  reclassification case returns a renamed
  `AlreadyPresentPermissionRepairNeeded` outcome instead, and the caller
  performs its own fresh N2 check before calling
  `repair_executable_permission` itself.
- **Agent-side wire validation (`validate_context`) still enforced only
  Stage 108-era bounds.** It checked native `entrypoint`/
  `working_directory` for length/NUL only (never the Stage 109 POSIX-
  relative-path contract), and never validated the native acquisition
  contract (`kind`/`source`/`sha256`) at all. Fixed: the validator is
  renamed to reflect Stage 109 and now enforces the identical POSIX-
  path/acquisition contract DServer enforces, independently
  hand-written (no shared crate) via two new pure-string helpers in
  `native_process_acquisition.rs`.
- **N6 path-resolution failure was falsely reported as a successful
  removal.** `governed_remove_state` treated `ManagedLayout::state_path`
  failing (e.g. a symlinked ancestor) identically to "nothing to
  remove," which could let rollback report success while the real
  state file remained present and unreachable. Fixed: a path-resolution
  failure now reports `removed: false` with an explicit blocked reason,
  and performs no filesystem mutation.

No architecture change, no new feature, no expanded scope -- this
addendum is entirely corrective, within the v0.1 scope already accepted
above.

## v5 addendum: real Linux validation closure

The v4 source closure passed portable (non-Linux) review, but this was
Stage 109's FIRST real execution on an actual Linux host:
`cargo test --locked --lib native_process_runtime -- --test-threads=1`.
It surfaced two compile-time defects invisible to a portable `cargo
check` (the real `#[cfg(target_os = "linux")]` spawn/signal path had
never actually been compiled before) and nine real test failures. No
redesign, no new feature, no change to N1-N6, the projection shape, or
any Stage 107A/B/108 invariant -- entirely corrective, within the v0.1
scope already accepted above.

**Compile fixes** (closing the exact risk the v2 addendum flagged: "the
real Linux code path had never actually been compiled"):

- `governed_materialize_and_spawn`'s `_recomputed_digest` parameter was
  never actually referenced under that name -- the real Linux spawn
  block referenced `recomputed_digest` (no leading underscore), which
  does not exist on a host where that block is compiled out entirely.
  Renamed to the real, referenced name; the resulting non-Linux
  "unused variable" warning (the parameter genuinely has no use outside
  the Linux-only block) is now suppressed explicitly via
  `#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]`,
  rather than by silently mismatching the two platforms' identifiers
  again.
- `SpawnError` gained `#[derive(Debug)]` -- required by `.unwrap()` on a
  `Result<_, SpawnError>` in the real Linux-only integration tests.

**L1 (the substantive fix): the observed-cmdline identity model was
wrong for interpreted (shebang) executables.** `read_proc_cmdline_args`
assumed "strip argv[0], and the remainder equals
`NativeProcessDeploymentSpecV1.args` exactly." Real Linux disproved
this: for a shebang script, the kernel execs the interpreter with
argv[0] = the interpreter, argv[1] = the script's own path, and only
argv[2..] = the real declared args -- stripping one leading element
still leaves the script's path mixed into what should be pure
desired-args identity, and no fixed stripping rule generalizes across
ELF binaries, `#!/bin/sh` scripts, `#!/usr/bin/env python3` scripts, and
unknown future interpreters without per-interpreter special-casing
(explicitly forbidden).

Fixed by changing the MODEL, not weakening the check: renamed to
`read_proc_cmdline`, which returns the COMPLETE observed
`/proc/<pid>/cmdline` vector exactly as the kernel recorded it,
including argv[0], with no attempt to reconstruct desired args from it.
The persisted field (`ManagedProcessRuntimeStateV1`) is renamed from
`args_digest` to `cmdline_digest` (a small, safe rename -- Stage 109
state was not yet committed/released) and documented explicitly as a
digest of the COMPLETE observed cmdline, i.e. observed RUNTIME identity,
deliberately distinct from and never compared against
`NativeProcessDeploymentSpecV1.args` (desired-state identity, used only
to spawn). Both identity-establishing call sites --
`spawn_and_persist` (persists the baseline immediately after a real
spawn) and `observe_persisted_identity` (re-verifies on every later
cycle) -- now read and hash the same complete vector the same way.
`IdentityCheck::ArgsMismatch` keeps its existing variant name for
call-site stability; only the underlying model changed. Fail-closed
behavior is completely unchanged: `check_identity` still never adopts,
signals, or spawns over an unconfirmed identity (Section AA/L9) --
only what is compared was wrong, never whether a mismatch blocks.

**L2: test-side identity fixtures were internally inconsistent.**
Several Linux-only tests manually built a `ManagedProcessRuntimeStateV1`
with `compute_args_digest(&[])` for a REAL spawned process, which always
has a non-empty observed cmdline (at minimum its own interpreter/path as
argv[0]) -- the runtime correctly refused to confirm identity against a
fixture that claimed a cmdline the real process never had. Fixed in the
test helper `state_for_real_pid` (now reads the real, currently-live
`/proc/<pid>/cmdline` itself) and in
`adoption_from_persisted_state_does_not_require_child_handle_for_observation`
(renamed from `adoption_after_discarding_the_child_handle_succeeds`, see
L5) -- production identity-checking logic was never weakened to make
this pass.

**L3: two raw-signal tests confused a zombie with a live process.**
`graceful_sigterm_stop_succeeds` and `unresponsive_process_is_force_killed`
polled `linux::pid_exists` (`kill(pid, 0)`) to detect process exit -- but
a direct child that has exited and not yet been reaped is a ZOMBIE, and
`kill(pid, 0)` still reports a zombie's PID as present. Fixed by polling
the OWNED `Child::try_wait()` handle instead (the only reliable "actually
exited AND reaped" signal available to a real parent), then asserting
`!pid_exists` only after that reap has actually happened.

**L4: `governed_stop` integration tests leaked zombies via
`std::mem::forget(child)`.** `governed_stop_confirms_a_cooperative_stop`,
`governed_stop_confirms_after_sigkill_for_an_unresponsive_process`, and
`expired_lease_blocks_governed_stop_and_never_duplicates_the_process` all
discarded the `Child` handle via `mem::forget` before ever calling
`wait()`, leaking a permanent zombie in the cargo test process for
`governed_stop`-confirmed exits. Fixed by keeping the handle (never
passed to `governed_stop` itself -- it continues to prove it needs only
persisted state + real `/proc`) solely for this test's own final reap,
called after `governed_stop` has already independently confirmed the
stop via its own Zombie-aware observation.

**L5: the adoption test overstated what it actually proved, and leaked a
zombie.** Renamed
`adoption_after_discarding_the_child_handle_succeeds` to
`adoption_from_persisted_state_does_not_require_child_handle_for_observation`
and documented explicitly that it does NOT simulate a true
parent-process restart/reparenting (this test process never actually
exits) -- only that `observe_persisted_identity` itself never receives
or needs a `Child` handle. The handle is now retained for the test's own
cleanup/reap rather than `mem::forget`-leaked.

**L6: the environment-determinism test bypassed the v4 N1/N2 split.**
`spawned_process_gets_only_explicitly_declared_environment_never_host_environment`
called the direct `spawn_and_persist` helper below
`governed_materialize_and_spawn`'s own N2 step, so the freshly
`materialize()`-written script (deliberately left non-executable since
v4) hit a real `EACCES` on spawn -- correct runtime behavior, stale test
setup. Fixed by calling `repair_executable_permission` explicitly before
spawning, matching what the real governed caller does; `materialize()`
itself was NOT changed to chmod again.

**L7: two "portable" stop-incomplete tests were not actually portable.**
`restart_for_drift_never_spawns_when_the_old_process_stop_is_not_confirmed`
and `rollback_never_removes_state_when_the_stop_is_not_confirmed` used a
fake PID `999_999`, relying on `governed_stop` having no real
observation mechanism on a non-Linux target. On a REAL Linux host, PID
`999_999` is legitimately `ProcessAbsent` (a genuine, CONFIRMED
idempotent stop), which is the opposite branch these tests intended to
exercise -- their outcome was accidentally dependent on ambient
process-table state rather than on the property under test. Fixed by
gating both to `#[cfg(not(target_os = "linux"))]` (with an updated
comment explaining exactly why) and adding two real-Linux equivalents in
`linux_integration_tests`
(`restart_for_drift_never_spawns_when_a_real_processs_identity_is_unconfirmed`,
`rollback_never_removes_state_when_a_real_processs_identity_is_unconfirmed`)
that spawn a genuine child, then deliberately corrupt only its persisted
`cmdline_digest` -- PID+starttime+executable all still match, so
`check_identity` reports `ArgsMismatch` (never `ProcessAbsent`), and the
same "never spawn/remove on an unconfirmed stop" invariant is proven
through `governed_stop`'s actual confirmation logic, plus a final
assertion that the real process was left completely untouched.

**L8: `governed_materialize_and_spawn`'s N3 lease-block reported a false
positive host mutation.** When materialization needed nothing (correct
bytes and permissions already present -- N1 and N2 both skipped) and the
N3 lease check itself was the first boundary reached and it blocked, the
function returned `host_mutation_performed: true` unconditionally,
misreporting a mutation that never happened. Fixed to return the
already-tracked `host_mutation_performed` variable (which correctly
reflects whatever N1/N2 activity, if any, actually occurred) instead of
a literal `true`.

**L9 (an explicit non-change, verified):** no identity check was
weakened. `IdentityCheck::ArgsMismatch`/`Unavailable` still block exactly
as before; `governed_stop` still never signals an unconfirmed identity.
Final identity remains PID + starttime + process state (Zombie-aware) +
observed `/proc/<pid>/exe` + complete observed `/proc/<pid>/cmdline`
digest -- only the MODEL of what the cmdline digest is computed over was
corrected (L1).

Portable (Windows) validation after this pass: `cargo test --locked
--lib` in `DATUM/` -- 799 passed, 0 failed (including the now-gated
`restart_for_drift_never_spawns_when_the_old_process_stop_is_not_confirmed`/
`rollback_never_removes_state_when_the_stop_is_not_confirmed`, which
remain exercised on this platform via `#[cfg(not(target_os =
"linux"))]`); `cargo build --locked --lib` -- 0 warnings. `dserver`'s
own suite (835 passed) was also re-run and is unaffected (it does not
reference any Stage 109 native-process type by name). This is Windows
(portable-logic-only) validation, NOT a substitute for a further real
Linux run -- Stage 109 is not declared complete from this platform. A
`stage109-native-process-runtime-review-v5-linux-closure.zip` transfer
package (containing the two corrected source files, this ADR, and a
manifest naming the exact Linux tests expected to exercise each fix
above) is prepared for the existing Linux VM; see that package's
manifest for the transfer/build/test steps and its SHA-256.

## v6 addendum: incorporating VM-proven corrections into the authoritative source

The v5 package was transferred to a real Linux VM and hand-patched there
for the concrete failures that first (real) Linux run exposed
(67 passed / 1 failed on the clean v5 candidate, per the VM
investigation), reaching a final manually-patched result of
`cargo test --locked --lib native_process_runtime -- --test-threads=1`
-> **68 passed, 0 failed**. This addendum incorporates exactly those
VM-proven corrections back into the authoritative Windows source at
`C:\Users\dnred\OneDrive\projetos\datum-claude`. No redesign; every fix
below is either a real runtime correction to `governed_stop`'s
post-signal identity handling, or test-only startup synchronization.

**Runtime fixes (affect production `governed_stop` behavior):**

- **Post-SIGKILL transient `Unavailable` (item 3).** After an identity
  was freshly `Confirmed`, N5 authorized, and SIGKILL successfully
  delivered, procfs can transiently lose `/proc/<pid>/exe` or
  `/proc/<pid>/cmdline` for a brief teardown window before `stat` reports
  `Zombie` or the PID disappears. The SIGKILL confirmation poll
  previously treated a single such `Unavailable` reading as an immediate
  stop failure. Fixed: within THIS SAME bounded poll window,
  `Unavailable` now continues polling exactly like `Confirmed` (never
  treated as success, never triggers another signal); a genuine
  `ExecutableMismatch`/`ArgsMismatch` still fails immediately, and running
  out the window without a positive disappearance still reports
  `NATIVE_PROCESS_STOP_INCOMPLETE` truthfully.
- **Post-SIGTERM transient `Unavailable` (item 5), symmetric fix.** The
  identical procfs-teardown-race tolerance applied to the graceful
  SIGTERM confirmation poll. The FINAL pre-SIGKILL identity check
  (unchanged, already correct, and explicitly re-verified) remains
  fail-closed: only an EXACT `Confirmed` identity there may proceed to
  N5/SIGKILL -- `Unavailable` at that specific, post-bounded-window
  instant withholds SIGKILL and reports the stop incomplete, exactly like
  a real mismatch. Transient tolerance mid-poll is never confused with
  permission to escalate once the window has actually expired.

**Test-only startup-synchronization fixes (no production behavior
change):**

- **Item 1** (`unresponsive_process_is_force_killed`) and **item 2**
  (`governed_stop_confirms_after_sigkill_for_an_unresponsive_process`):
  a `trap '' TERM` script could receive SIGTERM before its own shell had
  actually executed the `trap` statement -- a real fork/exec startup
  race, not test flakiness. Both now use a shared
  `temp_script_with_readiness`/`wait_for_readiness` handshake (a
  readiness marker file written by the script immediately after its
  setup lines, polled with a bounded wait before any signal is sent or
  any identity is built) that a new v5-added helper introduces.
- **Item 4** (`real_argv_identity_round_trips_and_detects_mismatch`):
  `/proc/<pid>/cmdline` was occasionally observed empty immediately after
  `spawn()` returns for a freshly-forked child (a distinct, narrower race
  than the post-signal teardown race fixed in `governed_stop` above).
  Fixed with the same readiness handshake for BOTH spawned processes
  before reading their real cmdlines.
- **Item 6** (`state_for_real_pid`, the shared Linux test helper): the
  same just-spawned-process `/proc/<pid>/cmdline` race, fixed with a
  bounded (~2s) retry loop reading `read_proc_cmdline` every ~10ms inside
  the helper itself, panicking only if the deadline elapses. Documented
  explicitly as a test-fixture-only accommodation; production
  `observe_persisted_identity` gains no retry behavior and remains
  fail-closed, single-observation.

**Explicitly preserved, none regressed (item 7):** full observed
`/proc/<pid>/cmdline` identity and `cmdline_digest` semantics (L1); no
interpreter special-casing; the N1/N2 materialize/permission split; typed
`SignalError`; ESRCH-only already-gone semantics; EPERM fail-closed;
zombie awareness; rollback/N6 governance; environment isolation (Section
AL); the exact acquisition contract; N3's truthful
`host_mutation_performed` (L8); the two real-Linux replacements for the
fake-PID-999999 tests (L7); and both v5 compile fixes
(`recomputed_digest`, `#[derive(Debug)] SpawnError`).

Windows validation after this pass (from the authoritative worktree):
`cargo build --locked --lib` (DATUM) -- 0 warnings; `cargo test --locked
--lib` (DATUM) -- 799 passed, 0 failed; `cargo test --locked` (DATUM,
all targets) -- 0 failed across every binary; `cargo build`/`cargo test
--locked` (dserver) -- 835+1 passed, 0 failed; both Python contract
checkers (`check_datum_contract_fixtures.py`,
`check_datum_contract_fixtures_selftest.py`) -- pass; `mkdocs build
--strict` -- exit 0; `git diff --check` -- clean (only pre-existing
CRLF/LF notices on unrelated already-tracked files, no new whitespace
errors).

This is, again, Windows (portable-logic-only) validation of source that
was ITSELF derived from a real, VM-proven 68/0 result -- it is NOT a
fresh Linux re-run of this exact v6 candidate. A
`stage109-native-process-runtime-review-v6-final-linux-proven.zip`
package is prepared and must undergo one final clean transfer/re-run on
the Linux VM before this stage is committed; see that package's
`FINAL_LINUX_CLOSURE.md` for the full closure record and its SHA-256.

### v6 follow-up: one full-`--lib` test-expectation fix (unrelated file)

With `native_process_runtime` itself at a real, VM-proven 68/0, running
the FULL library suite on Linux (`cargo test --locked --lib`, which this
repo's `[lib] test = false` / `doctest = false` setting means the
default `cargo test --locked` target never exercises -- `--lib` must be
passed explicitly) surfaced one further failure, unrelated to this
addendum's scope: `native_process_acquisition::tests::materialize_writes_and_reuses_idempotently`
expected a second `materialize()` call (before any N2 permission repair)
to report `AlreadyPresent` unconditionally. On real Unix/Linux this is
wrong: N1's fresh write deliberately leaves the file non-executable
(the existing, correct, unchanged N1/N2 separation), so a second call
before N2 correctly reports `AlreadyPresentPermissionRepairNeeded` --
`AlreadyPresent` is only correct on a non-Unix host, where there is no
POSIX executable bit to need repairing. This was a stale TEST
EXPECTATION, not a runtime defect -- `materialize()`/N1/N2 themselves
are unchanged. Fixed by branching the test's second-call assertion on
`#[cfg(unix)]` (asserting `AlreadyPresentPermissionRepairNeeded`, then
calling `repair_executable_permission` and asserting a THIRD call
reports the true `AlreadyPresent` no-op) vs. `#[cfg(not(unix))]`
(asserting `AlreadyPresent` directly, as before). Re-verified: `cargo
test --locked --lib` on Windows -- 799 passed, 0 failed (Windows takes
the `#[cfg(not(unix))]` branch, so this platform's count and behavior
are unchanged).

### v6 follow-up 2: final adoption-fixture correction (shell tail-call exec)

With every other Linux test green, `native_process_runtime`'s own
`adoption_from_persisted_state_does_not_require_child_handle_for_observation`
(the last of the 68) intermittently reported a legitimate
`IdentityCheck::ExecutableMismatch` on real Linux. Its fixture spawned a
`#!/bin/sh\nsleep 30\n` SCRIPT (via `temp_script`) and captured
`/proc/<pid>/exe` before calling `observe_persisted_identity`. Root
cause: when a shell script's LAST command is a simple, unconditional
invocation with nothing left for the shell to do afterward -- exactly
this script's shape -- many `/bin/sh` implementations execute it as a
tail call, directly `execve()`-ing the final command IN PLACE of the
shell's own process image (same PID, same starttime) rather than
forking a child. This is a real, POSIX-permitted shell optimization, not
a bug: `/proc/<pid>/exe` can race between "the shell binary" and "the
`sleep` binary" for the EXACT SAME pid+starttime, entirely outside this
agent's control. The runtime was CORRECT to report a mismatch when the
persisted snapshot and a fresh re-observation genuinely disagreed --
the FIXTURE, not the identity check, was unstable. Fixed by spawning
`sleep` DIRECTLY (`Command::new("sleep").arg("30")`, no shell, no
script, no `temp_script` indirection at all), so there is no
intermediate process image ever to exec away from. No interpreter
special-casing was added anywhere, and production
`observe_persisted_identity` gained no retry logic -- this is a
test-fixture-only correction, the last one from real Linux validation.
Re-verified on Windows (portable build only; this test is
`#[cfg(target_os = "linux")]`-gated and does not compile here, so
correctness was confirmed by manual review, consistent with this ADR's
existing "the real Linux code path is not type-checked on this host"
caveat): `cargo build --locked --lib` -- 0 warnings; `cargo test
--locked --lib` -- 799 passed, 0 failed (unaffected, since this test
does not exist in the Windows build).

Full real-Linux evidence after this exact correction (already obtained
on the VM, not re-run here): `native_process_runtime` filtered --
**68 passed, 0 failed**; `cargo test --locked --lib` (full library) --
**0 failed**; `cargo test --locked` (default target) -- **0 failed**.
Stage 109 native process runtime v0.1 is source-review-complete and
real-Linux-validated as of this addendum. Still not committed, pushed,
or merged -- that remains a separate, explicit decision.

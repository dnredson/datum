# ADR-0011 — F2: execution-content-bound authorization (Stages 107A / 107B)

- Status: Accepted as a **specification**. **Not implemented in Stage 107.**
- Decision refs: D-7; Round 2 correction 2; D-7a (round-3 model); N-1; N-2; N-3; DC-1..DC-5
- Implementation: Stage 107A (control-plane + agent), Stage 107B (catalog pinning).
- Sub-question **TD-1** (ADR-0012): **resolved** in Stage 107A design as
  **Option A** — immutable authorized snapshot with bounded lease (bounded
  staleness, not immediate revocation).

## Context (finding F2)

An operational reconciliation / rollback authorization binds project, D-Graph id
and revision, node and resource binding — but **not the execution content**. Traced
end to end:

- `PUT /api/v1/service-artifacts/:artifact_id` replaces the stored record
  (`dserver/src/core/service_artifact_registry.rs`).
- The reconciliation context joins the *current* artifacts at request time and
  passes the slice authorization through unchanged
  (`dserver/src/api/artifact_reconciliation.rs`, `artifact_reconciliation_context.rs`).
- DServer and the agent both re-check identity fields only
  (`core/operational_dgraph.rs`, `DATUM/src/agent/operational_reconciliation.rs`).
- The agent's local trusted mapping pins service name, container name and image
  reference string only.
- `network`, `volumes`, `configuration_mounts` and `command` drive
  `attachment_mismatch`, which recreates the container under the existing
  authorization.

Three sub-gaps: **F2a** mutable control-plane artifact content; **F2b** external
mutable content (image content behind mutable tags; bind-mounted configuration file
content, which drift never compares); **F2c** node-local *substitution* of
execution semantics (ports, network alias, secret environment-variable names come
from local configuration).

## Decision (specification)

### 1. `ArtifactExecutionProjectionV1` (correction 2)

A node-independent projection computed per artifact, covering **every field that
can alter runtime behaviour or a safety decision** — and, per the review
correction below, **only** such fields: the projection kind, `artifact_id`, canonical `service_id`, `artifact_kind`,
`runtime_kind`; `container_name`, `image_identity`, `command`, `network`,
`network_aliases`, `ports`, `volumes`,
`configuration_mounts` (source, target, read-only **and `declared_content_sha256`**);
`bindings` (with values, non-secret by contract); `secret_refs` (ordered);
`secret_bindings` (`secret_ref` + `target_environment_variable` + `version`,
**never secret values**); `dependencies`; `lifecycle` (each phase in order,
**program only**; args, working_dir and environment are not consumed by the
current executor and are excluded);
the four boolean `wasm_decision` safety flags (below).

- Digest = SHA-256 over the RFC 8785 JCS serialization (ADR-0006), so DServer and
  SmartSentinel derive the **same** digest from their different structs.

  **Projection scope correction (Stage 107A design review, finding 4) —
  `control_plane.version` is EVIDENCE ONLY, not projected.** It is reported
  (`artifact_version` in the agent's service report) but never read by any
  gate or executor in the v1 reconciliation path — it carries no execution
  semantics. **Consequence: a version-only PUT (no other field changed) is a
  semantic no-op and must NOT increment `execution_generation`.** This
  supersedes the earlier inclusion of `version` in the projected set.

  **`wasm_decision` field-by-field re-audit (finding 4) — three categories:**
  (A) transported **and** consumed to select execution behaviour ⇒ projected in
  full; (B) transported **and** safety-gated (a fail-closed precondition checks
  its value) ⇒ projected to the extent the gated value matters; (C) not
  consumed for behaviour selection, or not actually transported to the agent in
  a usable form ⇒ **not** projected; evidence/spec only. Evidence, read against
  `DATUM/src/agent/operational_reconciliation.rs`:
  - `explicit_invocation_required`, `host_imports_allowed`, `side_effects_allowed`,
    `safe_to_auto_execute` (all `bool`) — category **B**. `validate_context` /
    `validate_assignments` (both DServer and agent) `bail!`/reject unless these
    are exactly `{true, false, false, false}`; a change to any of them is a
    safety-gate transition. **Projected.**
  - `module_id`, `function_name` (`String`) — category **C**. The v1 agent's
    reconciler is `run_embedded_reconciler()`, which compiles a **hard-coded**
    `RECONCILER_WAT` constant and calls the fixed export name
    `OPERATIONAL_RECONCILER_FUNCTION`; it takes only
    `(desired_running: bool, observed_running: bool)` as parameters and never
    reads the artifact's `wasm_decision.module_id` / `function_name` to select
    a module or function. Validation checks only that both are non-empty — a
    precondition the artifact registry already enforces unconditionally at
    write time (`validate_artifact`), so no live empty↔non-empty transition can
    occur at the reconciliation layer either. Changing the *value* of a
    non-empty `module_id` / `function_name` today changes nothing the agent
    does. **Not projected.** Forward caveat: if a later stage makes the agent
    load/select a module dynamically by this identity, these fields move to
    category A and must be reclassified then.
  - `max_fuel`, `timeout_ms` (`Option<u64>`) — category **C**, on stronger
    grounds than module_id/function_name: these fields exist **only** on
    DServer's `ServiceArtifactWasmDecision`
    (`dserver/src/core/service_artifact_registry.rs`). The agent's own,
    independently defined type `ArtifactWasmDecisionPolicy`
    (`DATUM/src/agent/operational_reconciliation.rs`, lines 81-89) has **no**
    `max_fuel` / `timeout_ms` member; with no `deny_unknown_fields` anywhere in
    this path, `serde` silently drops those JSON fields on deserialization.
    `run_embedded_reconciler()` hard-codes `fuel_configured = 100_000` and
    never reads either field. **The agent cannot receive these values in any
    typed, usable form, so an "independent server/agent projection
    recomputation" of them is not possible today and must not be claimed.**
    **Not projected.** A prerequisite for ever projecting them: extend
    `ArtifactWasmDecisionPolicy` with the two fields **and** have the agent
    actually consume them (neither is true at `07400c8`).
  Full trace: `execution-projection-field-audit.txt` in the Stage 107A review
  bundle.
- Not projected: `control_plane.version` (evidence only, see above); `probes`,
  `health_checks`, `acquisition`, `target_stages`, non-alias metadata,
  timestamps; `wasm_decision.module_id`, `.function_name`, `.max_fuel`,
  `.timeout_ms` (see above).

  **Class-N re-verification (Stage 107A, against commit `07400c8`) — COMPLETED.**
  The provisional "class N" status of `probes`, `health_checks` and `acquisition`
  was re-traced against the real v1 operational reconciliation path
  (`dserver/src/core/artifact_reconciliation_context.rs`,
  `dserver/src/api/artifact_reconciliation.rs`,
  `DATUM/src/agent/operational_reconciliation.rs`,
  `DATUM/src/agent/service_artifact.rs`) for each of: mutation gating;
  host-command / execution selection; start / recreate; rollback; safety or
  authorization decision. Evidence:

  - `build_artifact_reconciliation_context()` constructs
    `ArtifactReconciliationAssignment` from a fixed field list that contains no
    `probes`, `health_checks` or `acquisition` member and never reads those
    fields; they are **not on the DServer → agent wire**.
  - `grep -rn "probes|health_check|\.acquisition|availability" DATUM/src/`
    returns **zero** hits in `operational_reconciliation.rs`. Every hit is in
    the separate observation / evidence path (`service_artifact.rs`
    `run_probe` / `derive_operational_state`, the `smartsentinel-service-probe`
    / `-service-evidence` binaries, `integrated_service_evidence.rs`) or is an
    unrelated symbol (`AcquisitionEffects` on the closed Controlled WASM
    Deployment prep result in `wasm_runtime/*.rs`; `acquisition_timeout_ms`
    CLI arg).
  - The agent's gating inputs are `control_plane_authorized`,
    `local_container_execution_allowed`, secret resolution, `dependency_blockers`
    (from `dependencies` only), `attachment_mismatch` (from
    network/volumes/configuration_mounts/command), `observed_before` /
    completion (from `docker inspect`), and `wasm.action_code`. The executors
    (`execute_trusted_container_intent`, `execute_lifecycle_phase`,
    `rollback_trusted_container_intent`) consume image, network, volumes,
    configuration_mounts, command, lifecycle, bindings, secret env names,
    `mapping.ports`, `mapping.service_name`. **None reads `probes`,
    `health_checks` or `acquisition`.** Convergence uses `docker inspect`, not
    artifact probes.

  **Result: `probes`, `health_checks` and `acquisition` are CONFIRMED class N**
  and remain excluded from `ArtifactExecutionProjectionV1`. Two caveats are
  recorded for future stages: (C-1) if a later stage makes the agent consume
  artifact `probes` / `health_checks` to decide re-mutation or rollback
  (health-gated convergence), that field is reclassified then; (C-2)
  `acquisition.sha256` for a future `git` / `command_bundle` artifact that
  enters a governed host-mutation path becomes an **external-input pin**
  (Section G verification interface) represented in the projection's
  verified-inputs section — not a projected `acquisition` struct member; the
  v1 container path at `07400c8` never consults `acquisition`. Full trace:
  `execution-projection-field-audit.txt` in the Stage 107A review bundle.

### 2. Generation model (D-7a, round-3)

Server-owned artifact fields `execution_generation: u64` and
`execution_projection_digest`.

- Create → `execution_generation = high_water(project, artifact_id) + 1`.
- PUT with a changed projection → increment **exactly once**, store the new digest.
- Semantic no-op PUT → no change.
- DELETE → remove the artifact, **retain the high-water** (DC-1).
- Authorization binds, per desired service,
  `{service_id, artifact_id, execution_generation, execution_projection_digest}`
  plus `execution_pinning_mode`.
- Validity requires the delivered artifact to match the bound `artifact_id` **and**
  `execution_generation` **and** `execution_projection_digest`; the agent also
  recomputes the digest from the delivered fields.
- **Issuance uses the current generation under a stable registry view.** Once
  issued, the authorization owns an immutable snapshot. Runtime compares delivered
  content to that snapshot's bindings, never to mutable registry state for
  retroactive revocation. A later registry update does not invalidate its finite lease.
- **No resurrection.** For `A(gen 4) -> B(gen 5) -> A(gen 6)`, an already-issued
  gen-4 snapshot may remain valid until its lease expires. A **new** authorization
  must bind current gen 6; gen 4 cannot obtain a new authorization after registry
  advancement. An expired gen-4 lease never becomes valid again merely because
  the content returned to the same bytes. Audit events do not revoke a valid lease.
- **`execution_projection_digest` is computed over `ArtifactExecutionProjectionV1`
  only** (§1). Because `control_plane.version` and the three non-behaviour-selecting
  `wasm_decision` fields are excluded from the projection (§1 correction), a PUT
  that changes only those fields does not change the digest and therefore does
  not increment `execution_generation` — it is a semantic no-op by construction,
  not a special case the increment rule has to detect separately.

### 3. Persistence (DC-4, D-7a assessment)

The generation model removes the cross-domain atomic-write ("two-lock") problem:
the only write is single-domain (content + generation + digest committed together
in the artifact registry's existing SQLite transaction). Residual staleness at use
time is **TD-1** (ADR-0012, Option A). `required` mode refuses authorization
unless the operational state and the artifact registry share one control-state
SQLite database, and records the backend in evidence.

**Migration / downgrade safety (Stage 107A design review, finding 5) — schema
version bump, not silent-default tolerance.** The earlier design ("new fields
are `#[serde(default)]`, so an old binary that re-saves the registry silently
drops them") is **unsafe**: an old (pre-107A) binary that loads a 107A-migrated
registry, ignores the unknown generation / high-water fields via
`#[serde(default)]`, and later performs its own write would silently regress
`execution_generation` and lose the durable high-water record — exactly the
monotonicity violation this ADR forbids. The corrected design:
- Add a **new, distinct state-envelope schema version**
  `SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION = "0.2.0"` for
  `ServiceArtifactRegistryState.schema_version` specifically (the per-artifact
  `ServiceArtifact.schema_version` stays `"0.1.0"`; old binaries never reach
  per-artifact validation because the envelope check below rejects first).
- On `ServiceArtifactRegistry::load_from_path`, before
  `validate_registry_state`: if the loaded `schema_version == "0.1.0"`, run an
  explicit, deterministic, **idempotent** migration (back-fill
  `execution_generation` / `execution_projection_digest` / the high-water map
  for every artifact, per `persistence-audit.txt` §3) fully validate the candidate `"0.2.0"` state, and **persist the
  validated migrated state (schema_version `"0.2.0"`) immediately**, before the server
  can serve any endpoint that could issue a Stage-107A authorization. This
  makes the migration itself transactional through the existing
  `persist_domain_json` single-transaction write, and running it again on an
  already-`"0.2.0"` state is a no-op.
- `validate_registry_state` keeps its existing **strict-equality** check
  (`state.schema_version != SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION` ⇒
  reject) against the *new* constant. This is a pre-existing repository property
  (the check was always strict equality, never `<=`), so an **old (pre-107A)
  binary opening a `"0.2.0"` database is already guaranteed to reject it**
  (`"0.2.0" != "0.1.0"` under the old binary's own hard-coded constant) via its
  existing `validate_registry_state` — it refuses to load, and therefore can
  never re-serialize and silently drop the new fields. DServer startup MUST
  treat that `Err` as fatal (abort startup) rather than falling back to an
  empty in-memory registry; this must be verified in Section J.
- **Downgrade behaviour is therefore reject-and-refuse-to-start, not
  soft-and-lossy.** Generation and high-water data are never silently dropped
  by an old binary, because an old binary never successfully opens a migrated
  database. This supersedes the earlier "soft non-destructive downgrade" claim.
- Content + generation + `execution_projection_digest` + the high-water map
  update all still commit **atomically** in one `persist_domain_json`
  transaction, for both an ordinary `mutate()` write and the one-time migration
  write.
- **Migration test obligations** (Section J/K): legacy `"0.1.0"` database
  migrates to `"0.2.0"` on load with deterministic, correctly back-filled
  generations; restart after migration is idempotent (no re-bump, state stays
  `"0.2.0"`); a simulated old-binary reader (validation against the literal
  `"0.1.0"` expectation) rejects a `"0.2.0"` database; delete/recreate
  monotonicity (§2) survives a restart that includes a migration.

### 4. N-1 — local policy narrows, never substitutes

Execution semantics (**what** runs) are control-plane contract and are projected:
ports, network and execution-relevant aliases, secret references and target
environment-variable names, configuration mount identity/target and content digest,
dependencies, bindings, image identity, command, volumes, lifecycle.

Node-local policy may only **narrow or deny**: allowed roots, trusted-mapping
allowlist, network allowlists, maximum resource / fuel limits,
`allow_container_execution`, `allow_dserv_execution`, `allowed_dnode_host_kinds`,
`minimum_execution_pinning_mode`, secret **value sources**, runtime availability.
A `local_policy_digest` (JCS+SHA-256, secret values excluded) is recorded in every
reconciliation report; DServer stores it for audit but does not authorize against
it.

### 5. N-2 — pinning modes

`execution_pinning_mode ∈ {legacy_partial, required}`, a project-level setting
changed only by an explicit recorded operator action; each authorization binds the
mode at issuance.

- `legacy_partial`: migration compatibility for the validated v1 path; class-A
  content fully bound; undeclared external digests listed as `unpinned_inputs`;
  existing local substitutions permitted only where the artifact leaves them
  undeclared and listed as `local_substitutions`; **every record carries
  `f2_status = partial_hardening`**.
- `required`: issuance refused unless the projection is `required`-complete for the
  node platform; the agent executes declared semantics only; image identity and
  configuration content verified before any create / recreate / start; needed for
  any "F2 closed" claim and for canonical VS1 evidence.
- **No silent legacy default.** New projects default to `required`; a persisted
  project with no mode after upgrade is treated as `required` (fail-closed); the
  agent default is `required`.
- **Pinning-mode changes are prospective, not retroactive (review finding P1-7,
  consistent with ADR-0012 Option A).** `execution_pinning_mode` is bound
  INTO the immutable snapshot at authorization issuance, exactly like the
  artifact bindings it sits beside. `ServiceArtifactRegistry::set_pinning_mode`
  changes what `authorize_operational_reconciliation` reads for the NEXT
  authorization it issues; it never reaches into an already-issued
  authorization record and rewrites its bound mode. Concretely: a
  `legacy_partial` authorization issued before an operator raises the
  project to `required` keeps enforcing `legacy_partial` semantics for the
  rest of its own finite lease -- it is not silently upgraded, and it is
  certainly not invalidated the instant the project mode changes. The
  project-mode change is visible starting with the next context fetch that
  needs a NEW authorization (an old one has already expired, or none
  exists), exactly the same "bounded staleness, not immediate revocation"
  property ADR-0012 establishes for artifact content. An agent's own
  `minimum_execution_pinning_mode` is a separate, purely LOCAL floor and may
  change effective enforcement immediately (it is evaluated fresh every
  cycle, is never part of any stored snapshot, and only ever makes
  enforcement stricter) -- that is not a form of revocation either, since it
  cannot invalidate anything already authorized; it can only refuse to use a
  new authorization that fails to meet the agent's own bar.

### 6. N-3 — catalog pinning is Stage 107B

Stage 107B (immediately after 107A) makes the VS1-path D-IoT artifacts
`required`-complete: OCI multi-platform `image_identity` (index digest, per-platform
manifest and config digests — a mutable tag is never assumed identical across
architectures), configuration `content_sha256`, and the N-1 execution semantics.
Each change increments `execution_generation` and requires fresh authorization.

### 7. Design completions (DC-1..DC-5)

DC-1 high-water survives DELETE. DC-2 artifact projection is node-independent;
per-node facts stay bound by graph revision + node + per-service binding.
DC-3 pre-mutation revalidation is a **candidate** mechanism, not adopted — it
depends on TD-1. DC-4 `required` mode refuses split persistence backends.
DC-5 server-owned generation fields: client-supplied values are rejected unless
equal to current.

## Closure classification

- **F2 closed** requires: TD-1 decided and its tests passing (**done**:
  ADR-0012 Accepted / Option A, with test obligations O-1..O-11); the class-N
  re-verification of `probes` / `health_checks` / `acquisition` completed against
  the actual mutation path (**done**: confirmed class N at `07400c8`, evidence
  above), with any field that influences executed behaviour or a safety decision
  moved into `ArtifactExecutionProjectionV1`; an integration test in `required`
  mode with both domains in one SQLite database; every artifact of the authorized
  node slice `required`-complete and verified; no local substitution; the full
  G.4.8 test set. The remaining items depend on the Stage 107A runtime
  implementation and the Stage 107B catalog pinning.
- **Partial hardening** is any other outcome, including 107A on the current catalog
  before 107B. Evidence states which.

## Consequences

- Stage 107 fixtures demonstrate the shape without any runtime:
  `artifact-execution-projection.mosquitto.legacy.json` (not `required`-complete),
  `artifact-execution-projection.mosquitto.required.json`,
  `execution-generation-rules.json`, `execution-pinning-modes.json`,
  `local-policy-digest.json`, `oci-image-identity.json`.
- 107A must specify TD-1 (ADR-0012) before implementation begins.

# ADR-0011 — F2: execution-content-bound authorization (Stages 107A / 107B)

- Status: Accepted as a **specification**. **Not implemented in Stage 107.**
- Decision refs: D-7; Round 2 correction 2; D-7a (round-3 model); N-1; N-2; N-3; DC-1..DC-5
- Implementation: Stage 107A (control-plane + agent), Stage 107B (catalog pinning).
- Open sub-question: **TD-1** (ADR-0012) must be decided before 107A implementation.

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
can alter runtime behaviour or a safety decision**: `artifact_id`, `version`,
`artifact_kind`, `runtime_kind`; `container_name`, `image_identity`, `command`,
`network`, `network_aliases`, `ports`, `volumes`,
`configuration_mounts` (source, target, read-only **and `content_sha256`**);
`environment_bindings` (with values, non-secret by contract);
`secret_bindings` (`secret_ref` + `target_environment_variable`, **never values**);
`dependencies`; `lifecycle` (program, args, working_dir, environment, in full);
all eight `wasm_decision` fields.

- Digest = SHA-256 over the RFC 8785 JCS serialization (ADR-0006), so DServer and
  SmartSentinel derive the **same** digest from their different structs. Struct
  mismatches (e.g. `max_fuel` / `timeout_ms` present only in the DServer struct)
  are resolved by adding fields, not by omitting content.
- Not projected: probes, `health_checks`, `acquisition`, `target_stages`,
  non-alias metadata, timestamps.

  **This "class N" classification of `probes`, `health_checks` and `acquisition`
  is provisional and MUST be re-verified against the actual mutation path before
  Stage 107A implementation.** Stage 107A must trace, for each of those three
  fields, whether it can influence any of:

  - mutation gating;
  - host-command or execution selection;
  - start / recreate behaviour;
  - rollback behaviour;
  - a safety or authorization decision.

  If a field can influence executed behaviour or a safety decision, it MUST be
  reclassified as execution / safety-relevant and included in
  `ArtifactExecutionProjectionV1` before F2 may be considered closed. Stage 107
  does not make or implement that reclassification; it only records the
  requirement.

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
- **No resurrection.** `A(gen 4) → B(gen 5) → A(gen 6)` with identical content
  digests leaves the gen-4 authorization invalid, because `4 != 6`.
- Revoked-state or audit events may be recorded, but **safety never depends on
  writing to the authorization domain** on an artifact change; invalidation is
  derived at use time by comparison.

### 3. Persistence (DC-4, D-7a assessment)

The generation model removes the cross-domain atomic-write ("two-lock") problem:
the only write is single-domain (content + generation + digest committed together
in the artifact registry's existing SQLite transaction). Residual staleness at use
time is **TD-1** (ADR-0012). `required` mode refuses authorization unless the
operational state and the artifact registry share one control-state SQLite
database, and records the backend in evidence.

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

- **F2 closed** requires: TD-1 decided and its tests passing; the class-N
  re-verification of `probes` / `health_checks` / `acquisition` completed against
  the actual mutation path, with any field that influences executed behaviour or a
  safety decision moved into `ArtifactExecutionProjectionV1`; an integration test
  in `required` mode with both domains in one SQLite database; every artifact of
  the authorized node slice `required`-complete and verified; no local
  substitution; the full G.4.8 test set.
- **Partial hardening** is any other outcome, including 107A on the current catalog
  before 107B. Evidence states which.

## Consequences

- Stage 107 fixtures demonstrate the shape without any runtime:
  `artifact-execution-projection.mosquitto.legacy.json` (not `required`-complete),
  `artifact-execution-projection.mosquitto.required.json`,
  `execution-generation-rules.json`, `execution-pinning-modes.json`,
  `local-policy-digest.json`, `oci-image-identity.json`.
- 107A must specify TD-1 (ADR-0012) before implementation begins.

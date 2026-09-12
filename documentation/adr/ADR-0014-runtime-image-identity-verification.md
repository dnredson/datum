# ADR-0014 — Runtime image identity verification (Stage 107B-B)

- Status: Accepted and implemented (Stage 107B-B, v6 — corrects one P0
  finding from the v5 independent review, on top of everything v5
  corrected from v4, v4 from v3, v3 from v2, and v2 from v1; see "Prior
  review rounds" at the end for a compact history, not an appended
  correction log).
- Decision refs: ADR-0011 (F2 execution-content authorization), ADR-0012
  (TOCTOU, Option A), ADR-0013 (OCI catalog image identity resolution,
  Stage 107B-A).
- Scope: only the SmartSentinel/DATUM agent's runtime verification that the
  Docker content it actually pulls/runs/observes matches the immutable
  identity DServer resolved and authorized. Configuration bind-mount content
  materialization (Stage 107B-C) and full required-mode E2E closure across
  the whole gap set (Stage 107B-D) are separate, later slices — not decided
  or implemented here.

## Context

Stage 107B-A proved that DServer can resolve and persist a registry-backed
image reference into a concrete, immutable, multi-platform OCI identity
(`ResolvedImageIdentityV1` / `OperationalResolvedImageIdentity`) before
authorization. Stage 107B-B makes the agent's own Docker-mutation path
(`operational_reconciliation.rs`'s M1–M8 executors) actually consult that
resolved identity, instead of trusting the mutable, control-plane-declared
tag string (`OperationalArtifactAssignment.image`, mirrored through the
node-local `DispatchContainerMapping.artifact` trust key) for everything.

This document describes the CURRENT (v6) design in full. It does not restate
every historical mistake from the v1 and v2 review rounds inline; those are
summarized once, compactly, in "Prior review rounds" at the end, for anyone
who needs the history. Everything above that section is simply how the
system behaves today.

## The three identities — the central model

Three distinct things are compared, and conflating any two of them is
exactly how the v1/v2 review rounds' P0 findings happened. Every section
below refers back to this split by name.

1. **Declared intent** (`OperationalArtifactAssignment.image`, e.g.
   `"eclipse-mosquitto:2"`). Catalog/operator intent and provenance. The
   exact string `trusted_container_mapping_for_artifact` matches against
   `DispatchContainerMapping.artifact` to decide whether this node trusts
   this container/image pairing at all — that node-local trust gate is
   evaluated first, unchanged. **Never** the runtime source of truth once a
   valid pin exists — not for `docker image inspect`, not for `docker pull`,
   not for the `docker run`/`create` image argument, not for the top-level
   attachment/drift comparison.
2. **Authorized target image** (`resolved_image_identity.platforms[i]`:
   `execution_reference`, `manifest_digest`, `config_digest`). The immutable
   identity DServer resolved and authorized before this cycle. Used for
   `docker image inspect`, M5 `docker pull`, the M1 `docker run` image
   argument, and as the expected `.Config.Image` value in pinned mode. A
   mismatch here (the CANDIDATE content itself disagreeing with the
   authorized manifest/config identity) is a real, unsafe mismatch: it
   blocks in every mode, and no execution is ever permitted (§6, case C).
3. **Existing container binding** (`docker inspect <container>`'s top-level
   `.Image` field — the image object/ID the container currently, actually
   runs). Used for drift/convergence: does an EXISTING container still run
   the authorized target, or does it need to be recreated? A mismatch here
   — the existing container bound to a DIFFERENT (older, or differently
   re-pinned) image object than the target — is runtime **drift**, not a
   terminal candidate mismatch. It must never be silently converged over,
   must never cause the wrong existing container to be started (M2), but
   — when execution is otherwise permitted — must allow a governed recreate
   (M3) onto the verified target (§6a).

## Decision

### 1. Digest-first execution (frozen)

Once an artifact's assignment declares `image_source_kind == "registry"` and
carries a structurally-valid `resolved_image_identity`, and a platform has
been selected for this node (§5), **every Docker command that touches image
content — `docker image inspect`, `docker pull`, the `docker run`/`create`
image argument, and the top-level attachment/drift comparison — uses the
immutable, fully-qualified `execution_reference`
(`"{repository}@{manifest_digest}"`), never the mutable declared tag.** A
failure to select or verify that identity produces an image-identity
outcome (§6); it is never silently replaced by a fallback to tag-based
execution.

Implementation: `execute_trusted_container_intent`
(`operational_reconciliation.rs`) takes a `runtime_image_override:
Option<&str>` parameter. `let image =
runtime_image_override.unwrap_or(declared_image); let pinned =
runtime_image_override.is_some();` are the two derived values threaded
through: `image` overrides the mutable tag everywhere it was previously
used (the `pull_image` lifecycle command, the internal attachment-drift
comparison, the `docker run` argument, both M3/M4 recursive re-entries);
`pinned` gates both the lifecycle `pull_image` no-op (§8) and the `--pull
never` argument (§7/§11) on `docker run`. Both, plus a new
`container_binding_drift: bool` (§6a), are threaded unchanged through the
recursive M3/M4 re-entries.

The **outer**, top-level attachment/drift comparison in the per-service
loop (which decides whether WASM sees an already-converged service, before
`execute_trusted_container_intent` is ever reached) uses the identical
`expected_runtime_image_reference`: the platform-selected immutable
`execution_reference` whenever the preflight (§2) selected one — regardless
of whether that reference has been verified, acquired, or is still merely a
`NeedsPull` candidate — and the declared tag only when no preflight ran at
all (`local_build`/`legacy_unclassified`/unresolved/structurally-invalid
registry pins). This is the single source of truth used consistently by the
outer comparison, `execute_trusted_container_intent`, M1, and the recursive
M3/M4 re-entries — there is exactly one place this value is computed per
service, per cycle.

### 2. New module: `image_identity_verification`

A narrow module (`DATUM/src/agent/image_identity_verification.rs`) owns
everything Docker-and-identity-specific: platform selection, RepoDigest
canonicalization, digest comparison, and a `DockerImageRuntime` trait (§8).
It knows nothing about leases, M1–M8 boundaries, or `execution_pinning_mode`
— those remain entirely owned by `operational_reconciliation.rs`.

The module exposes two, deliberately separate, phases:

- **Read-only preflight**: `evaluate_image_identity` (selects a platform,
  inspects the resulting reference, checks container binding when
  applicable) produces an `ImageIdentityEvaluation` — `evidence`
  (`VerifiedImageIdentity`, TARGET IMAGE only), `execution_reference`,
  `found_locally`, `verified` (TARGET IMAGE only), and (v3 addition, §6a)
  `container_binding: ContainerBindingObservation` (`NotApplicable`,
  `Matches`, `Mismatches { actual_image_id }`, `Unavailable` — a
  deliberately SEPARATE observation, never folded into `verified`/
  `reason_kind`). `classify_evaluation` turns the TARGET-image half into a
  `PreflightOutcome` (`Verified`, `KnownMismatch`, `NoSelectableReference`,
  `UnverifiedWithReference { execution_reference, reason_kind }`, or
  `NeedsPull { execution_reference }`). **Never calls `pull_image`.** Safe
  to call unconditionally, before any governance gate, and even in a
  non-executing cycle (§10).
- **Acquisition**: `acquire_pinned_image` — called ONLY for a `NeedsPull`
  outcome, ONLY by `operational_reconciliation.rs`'s
  `resolve_pinned_image_outcome` (§14), and ONLY after every non-image
  governance gate and a fresh M5 lease check have already passed. Pulls
  once, re-evaluates read-only, re-classifies, and never re-attempts a
  second pull regardless of the re-evaluation's outcome.

### 3. Authorized-vs-observed identity mapping

For the platform selected (§5):

| Authorized (`OperationalResolvedPlatformManifest`) | Observed (`docker image inspect <execution_reference>`) | Comparison |
|---|---|---|
| `manifest_digest` | `.RepoDigests[]`, canonicalized (§4) | at least one canonicalized entry equals `(repository, manifest_digest)` |
| `config_digest` | `.Id` | direct string equality (case-insensitive), after both are validated as well-formed `sha256:<64hex>` via the SAME `is_oci_digest_format` the rest of Stage 107B-A/B uses — never re-prefixed, so a value that already carries `sha256:` is never turned into `sha256:sha256:<hex>` |
| `os` / `architecture` | `.Os` / `.Architecture` | direct equality |
| `variant` | `.Variant` | see §5's variant policy |

This is the TARGET IMAGE comparison ONLY (`verified`/`reason_kind`); it is
completely independent of container binding.

**Existing container binding (§6a)** is a separate observation: the
container's top-level `.Image` field (the image ID/reference the container
was actually created from) is compared against the verified image's own
`.Id`, via `DockerImageRuntime::inspect_container_image_binding(container_name)
-> Result<Option<String>, String>`, never an unabstracted `Command::new`
inline in `operational_reconciliation.rs`. `Ok(None)` (recognized-missing)
and `Err(_)` (inspect failure, malformed JSON, missing `.Image` field) are
BOTH `ContainerBindingObservation::Unavailable` — a container known to
exist (the caller only ever passes a name for Running/Stopped) whose
binding evidence could not be obtained is never silently treated as
"nothing to check," and never silently treated as a match either.
`.Config.Image` (the literal argument string passed at `docker run`/
`create` time) remains useful as provenance/drift evidence for what runtime
argument was used, but is explicitly **not** treated as content-identity
proof — the top-level `.Image` field is.

**Daemon-version caveat, stated explicitly rather than assumed:** `.Id`
being the OCI config digest is true for the containerd-image-store-backed
Docker daemons this project targets; it is not a universal OCI guarantee on
every historical Docker storage driver. `.Variant` requires Docker API
≥ 1.30 (Docker ≥ 17.05); its absence is represented as `None`, never
inferred.

### 4. RepoDigest canonicalization

Docker prints `RepoDigests` entries in the "familiar" reference form
(Docker Hub official images and the implicit-registry rule drop the
`docker.io`/`library/` prefix Docker Hub applies implicitly), which does not
necessarily match the fully-qualified spelling
`resolved_image_identity.repository` was built with server-side (ADR-0013
§4). `canonicalize_repo_digest_entry` parses one observed entry into
`(fully_qualified_repository, digest)`, applying the exact same
implicit-registry / implicit-`library/`-namespace rule ADR-0013's
`parse_image_reference` uses — **duplicated, not imported**, so no server
dependency (`dserver`) is introduced into the agent crate. `repo_digests_contain`
then requires both the canonicalized repository AND the digest to match; an
unrelated repository that happens to share the same digest string is
rejected (the repository half is load-bearing, not decorative). Covered by
fixtures for: Docker Hub official (short vs. explicit `docker.io/library/…`
spellings), Docker Hub namespaced, `ghcr.io`, a custom host with a port, and
an unrelated-repository-same-digest rejection.

### 5. Platform / variant selection — fails closed, never guesses

Neither `OperationalArtifactAssignment` nor the pin-request payload carries
a per-node target-platform association: `resolved_image_identity.platforms`
can legitimately contain more than one platform (an operator resolved
several architectures at pin time), with no wire-level indication of which
one is "this node's." The agent determines that at runtime, from its own
Docker daemon's reported platform (`docker version --format
'{{.Server.Os}}/{{.Server.Arch}}'`) — never from `std::env::consts`, which
uses different vocabulary and can diverge from the daemon's actual
execution platform.

`select_authorized_platform` (rules, exactly as decided):
- filters authorized platforms to those matching the daemon's `os`/`architecture`;
- zero matches → `NoMatch` (→ `blocked_image_identity_unverified`,
  `no_matching_platform`);
- exactly one match → selected, UNLESS the daemon authoritatively reports a
  variant that actively disagrees with that one candidate's (a real
  conflict, classified as `NoMatch` — no authorized platform actually fits
  this daemon); an unknown daemon variant never blocks the single-candidate
  case (nothing to disambiguate against — mirrors ADR-0013's "index already
  confirmed it" leniency);
- two or more matches → the daemon's variant must be known and must narrow
  it to exactly one match; an unknown daemon variant, or a known variant
  matching zero or 2+ candidates (the latter should not occur given
  ADR-0013's platform-uniqueness constraint, handled defensively anyway) →
  `Ambiguous` (→ `blocked_image_identity_unverified`, `ambiguous_platform`).

**Never picks an arbitrary first candidate. Never infers a variant merely to
let execution proceed.** Documented limitation: Docker's CLI/API does not
reliably expose an authoritative ARM variant (v6/v7/v8) via `docker version`
— `PlatformIdentity.variant` from `DockerImageRuntime::server_platform` is
therefore always `None` in the current CLI implementation. This is an
honest gap, not a placeholder to "improve" into a guess: it means a catalog
artifact pinning two variants of the same architecture (e.g. `arm/v6` and
`arm/v7`) will always be `Ambiguous` on today's implementation, failing
closed. `Ambiguous`/`NoMatch` both classify as
`PreflightOutcome::NoSelectableReference` (§6), which is terminal in every
mode.

Once a platform is selected, the SAME variant-comparison rule applies to
the observed image's own `.Variant` (§3): absent is lenient (the platform
was already selected/confirmed by digest — mirrors ADR-0013's
`VariantAuthority::IndexDescriptorAlreadyConfirmed`), present-and-disagreeing
is `platform_mismatch`.

### 6. `required` vs. `legacy_partial` — the TARGET IMAGE mismatch-always-blocks rule

Three materially different cases for the TARGET IMAGE (§3), expressed
through `PreflightOutcome` (§2) and folded by `resolve_pinned_image_outcome`'s
`fold_terminal_outcome` helper — applied IDENTICALLY whether the outcome
came from the initial preflight or from the post-acquisition re-evaluation:

- **A. No trustworthy pin** (`local_build`, `legacy_unclassified`, or a
  `registry` artifact with no `resolved_image_identity` / a structurally
  invalid one): `required` blocks via the pre-existing
  `required_completeness_gaps` completeness gate (unchanged);
  `legacy_partial` continues via the mutable reference exactly as before
  Stage 107B-B — the preflight is never even computed (no `PreflightOutcome`
  exists for this artifact at all). Evidence is still recorded explicitly
  (§12b), never silently absent.
- **B. Pin exists, verification unavailable/incomplete.** Two sub-cases:
  - **B1 — no execution_reference could be selected at all**
    (`PreflightOutcome::NoSelectableReference`; sub-reasons
    `no_matching_platform`, `ambiguous_platform`, or a platform-query
    failure) → `blocked_image_identity_unverified`, **in EVERY mode,
    including `legacy_partial`** — there is no immutable reference to
    safely fall back to, so falling back to the mutable tag would silently
    defeat digest-first execution (§1).
  - **B2 — a reference WAS selected, but the image evidence itself could
    not be confirmed** (`PreflightOutcome::UnverifiedWithReference
    { execution_reference, reason_kind }`; sub-reason `inspect_unavailable`
    — container-binding unavailability is NEVER a `reason_kind` here, see
    §6a) → blocks in `required`; in `legacy_partial` (or any non-`required`
    mode) it **sets `execution_reference`**, so the artifact still executes
    the verified immutable reference rather than the mutable tag — evidence
    still truthfully records `target_verified: false`, never silently
    claiming success.
- **C. Pin exists, evidence actively contradicts it**
  (`PreflightOutcome::KnownMismatch`; sub-reasons `manifest_mismatch`,
  `config_mismatch`, `platform_mismatch`): **blocks in BOTH `required` and
  `legacy_partial`.** This is a known mismatch, not partial hardening —
  `legacy_partial` never means "execute the wrong content anyway." Note
  that `container_binding_mismatch` is deliberately NOT one of these
  sub-reasons (§6a) — a wrong EXISTING container is drift, not this kind of
  terminal mismatch.

A genuine `docker pull` command failure (registry/network problem) is
**not** an identity-verification outcome at all — it is handled exactly like
the pre-existing `pull_image` lifecycle command's own failure path: a
per-service `"failed"` status, never a `blocked_*` code, never propagated as
a fatal error for the whole reconciliation cycle.

### 6a. Existing-container binding drift — repair, never a terminal block

An EXISTING container's binding (§3) disagreeing with a TARGET IMAGE that
itself verifies correctly (`ContainerBindingObservation::Mismatches`) is the
signature of a **legitimate re-pin**: the authorization now names a new
target B, the immutable image B has already been verified locally, but the
running/stopped container was created from the prior image A and has not
yet been recreated. This is runtime drift, not a candidate-content problem,
and is handled distinctly from §6's TARGET IMAGE mismatch:

- It is folded into the pre-existing `attachment_mismatch` computation —
  both the OUTER (per-service loop) one and `execute_trusted_container_intent`'s
  own internal one (the latter via a `container_binding_drift: bool`
  parameter, OR'd in only for `ContainerState::Running`/`Stopped`, since a
  `Missing` container has no binding to drift) — so it forces exactly the
  same governed recreate (M3) path as any other attachment drift.
- **Two distinct observation points feed this, both correctly (v6 fix,
  closing a gap the v5→v6 review found):** (a) the INITIAL, pre-acquisition
  preflight, when the target image was already present locally — this
  feeds the OUTER `attachment_mismatch` (WASM's initial input,
  non-executing convergence) via the plain `container_binding_drift`
  variable, unchanged; (b) the FINAL, POST-M5 re-evaluation, when the
  initial preflight outcome was `NeedsPull` (target image absent locally —
  in that case the initial `container_binding` is necessarily
  `NotApplicable`, since there is no local target image object yet to
  compare the existing container against at all, so the initial
  `container_binding_drift` cannot see a mismatch that can only be
  discovered once the pull completes). The value actually passed to
  `execute_trusted_container_intent` is
  `container_binding_drift_for_execution`, re-derived immediately after
  `resolve_pinned_image_outcome` returns from
  `resolution.evidence.container_binding_status` (the FINAL evidence) via
  `container_binding_mismatches_from_status` — never the stale,
  pre-acquisition value. When no acquisition happens this cycle (the
  mismatch was already visible pre-acquisition, or the outcome was never
  `NeedsPull` at all), this re-derivation reads the SAME, unchanged
  evidence and agrees with the original value exactly — so this fix
  changes nothing about the previously-working (v3–v5) case, only adds the
  previously-missing one.
- It is **never** folded into `image_known_mismatch`/`stage107a_service_block`
  — a drifted existing container never produces
  `blocked_image_identity_mismatch`, and never permanently blocks M3 from
  ever repairing it (the v2-round bug this corrects: conflating "wrong
  candidate" and "existing container stale" made a legitimate re-pin
  deadlock forever).
- It is **never** a reason to start (M2) the existing, wrongly-bound
  container — because `container_binding_drift` forces
  `attachments_mismatch = true` before the M2/M1 `match observed_before`
  dispatch is reached, a Stopped container with drift always hits M3
  (recreate) first, never M2.
- For a NON-executing cycle, it still contributes to `attachment_mismatch`
  (so a drifted deployment is correctly reported non-converged, matching
  §10), but — like every other mutation — is never acted on: no recreate,
  no pull, observation only.
- Container-binding **UNAVAILABLE** (§3) remains distinct from a KNOWN
  mismatch: it is evidence-truthful (`container_binding_status`, §12,
  records `unavailable`) but does not, by itself, force a recreate the way
  a KNOWN wrong binding does — only `Mismatches` does. It is instead
  ENFORCED separately, per §6b.

### 6b. Existing-container binding UNAVAILABLE — a REUSE guard, enforced, never silently reused (v4, reshaped v5)

`ContainerBindingObservation::Unavailable` means the agent could not prove
ANYTHING about what image the existing container is actually bound to —
not a known-good match, not a known-bad mismatch. Unlike `Mismatches`
(§6a), there is no safe automatic remedy (recreating "just in case" would
be acting on a guess, not on evidence), so `required` mode fails closed —
but ONLY against *reusing* the existing container, never against anything
else:

- `container_binding_unavailable_in_required_mode_from` is a plain `bool`
  (v5 correction — see "Prior review rounds" below for why this is no
  longer an `Option<String>` block), true whenever `effective_mode ==
  required` AND the FINAL `container_binding_status` for this cycle
  (post-acquisition, if a pull happened) is `unavailable`. Computed
  directly from `container_binding_status`, never by routing through
  `PreflightOutcome`/`classify_evaluation` (which remain TARGET-image-only,
  unchanged).
- This `bool` is threaded directly into `execute_trusted_container_intent`
  as a REUSE guard, consulted ONLY AFTER the M3 (`attachments_mismatch`,
  §6a) and M4 (stopped + secrets need refresh) recreate checks have
  already had their chance to fire, and scoped to `Running`/`Stopped`
  only (a just-recreated `Missing` container has nothing existing to hold
  a reuse judgment about): if still true at that point, it returns the
  stable `blocked_image_identity_unverified` code directly — WITHOUT ever
  reaching `Running -> "already_running"` or `Stopped -> M2 start`. This
  means: a Stopped container with an unavailable binding and NO
  independent reason to recreate is blocked, never M2-started; the SAME
  container with an INDEPENDENTLY required M3 (attachment drift) or M4
  (secret refresh) recreate is NOT blocked — the recreate proceeds using
  the verified immutable target, which is the safe corrective action; a
  Running container with an unavailable binding and no drift is blocked
  from being reported `"already_running"`.
- For `action_code == 0` specifically (no host action requested this
  cycle — so no M3/M4/M2 was ever going to happen anyway), the SAME
  underlying fact is instead surfaced as `"image_identity_unconfirmed"`
  (§10), computed independently by `image_identity_unconfirmed_in_required_mode`
  from the ORIGINAL, unconditional preflight's `container_binding` (never
  the reuse guard above) — nothing is blocked because nothing was going to
  mutate anyway, but "converged" is still never claimed.
- `legacy_partial` (or any mode but `required`) is **never** affected by
  either mechanism — it keeps its existing, weaker "may still proceed"
  semantics (an unavailable binding does not, by itself, stop
  `legacy_partial` from starting/reusing/recreating the container). Only
  the evidence (`container_binding_status = unavailable`, §12) is required
  to stay truthful in that mode; the review explicitly requested
  preserving this asymmetry rather than tightening `legacy_partial` to
  match `required`.
- A non-executing ("suggested") cycle is unaffected by either mechanism
  above (both live inside the `execute_requested && control_plane_authorized`
  gate, §10/§14) — it never blocks and never claims convergence for an
  unavailable binding; the raw evidence is simply recorded.
- **Remove-only rollback (M8) is never affected by either mechanism at
  all** — `execute_trusted_container_intent` (which the reuse guard lives
  inside) is never called during rollback; `rollback_trusted_container_intent`
  is a completely separate function that never receives this `bool`. See
  §6c.

### 6c. Remove-only rollback (M8) is independent of every runtime image-verification outcome (v5)

A container whose target image is mismatching, unverifiable, has no
selectable platform, or has an unavailable binding is PRECISELY a
container that may need removing — none of those facts may ever prevent
`rollback_trusted_container_intent` (M8, `docker rm -f`) from running.
Concretely:

- `rollback_service_block_from(stage107a_service_block_before_image,
  attachment_policy_block)` computes the block the rollback branch of the
  per-service loop consults — its signature does not even ACCEPT
  `image_identity_final_block` (§6, TARGET-image mismatch/unverified/
  no-selectable-platform) as a parameter, so it is impossible BY
  CONSTRUCTION for any of those outcomes to reach it.
  `container_binding_unavailable_in_required_mode` (§6b) was never part of
  this composition either — it is threaded directly into
  `execute_trusted_container_intent`, which rollback never calls at all.
- Every OTHER, genuinely pre-existing Stage-107A rollback prerequisite is
  retained exactly as before: `execute_requested`, the separately-scoped
  `operation='rollback'` authorization, `control_plane_authorized`,
  generation/projection/completeness binding
  (`stage107a_service_block_before_image` — "exact service/artifact
  snapshot binding"), local attachment/secret substitution policy
  (`attachment_policy_block`), the local trusted-container mapping,
  secret-resolution gates, local execution policy, Docker observation, and
  a fresh M8 lease check (`check_boundary(..., "M8", ...)`) immediately
  before the actual `docker rm -f` — none of this changed.
- `mutation_gates_are_clear`'s `!rollback_requested` conjunct already
  independently guarantees no M5 acquisition is ever attempted during a
  rollback cycle, regardless of any image-identity outcome — unchanged
  since v3.
- Read-only image-identity evidence (`verified_image_identity`) is still
  recorded truthfully for a rollback cycle exactly as for any other cycle
  (the unconditional preflight, §10) — this section changes only whether a
  runtime image-verification MUTATION-BLOCK outcome may prevent M8, never
  whether evidence is observed/recorded.

This corrects the v3→v4 review round's own regression (not present before
v3): `stage107a_service_block` had accreted `image_identity_final_block`
and (in v4, before this reshape) `container_binding_unavailable_block`,
and the rollback branch consulted that SAME variable — meaning an
authorized, otherwise-valid rollback request could be refused solely
because the target image was unverifiable, which directly contradicts the
frozen M8 remove-only contract (§13 predates this and was never itself
wrong; the regression was introduced by later image-identity work reusing
the same block variable rollback already consulted for unrelated,
legitimate reasons).

### 7. Pre-existing local image

With digest-first execution (§1), the presence matrix collapses cleanly:
inspecting `docker image inspect repo@sha256:X` is unambiguous by
construction (it either matches an image object that exists locally,
however it got there, or it doesn't). Concretely:

1. Absent (recognized `Ok(None)` from `DockerImageRuntime::inspect_image`,
   never a raw inspect error — see §9): M5 pull by the same immutable
   reference, then immediately re-inspect and re-verify
   (`pull_disposition: pull_performed`).
2. Exact authorized identity already present: pull skipped entirely
   (`pull_disposition: already_present`).
3. Present but mismatching (wrong manifest, wrong config, wrong platform):
   **never pulled over and silently continued past** — reported as
   `blocked_image_identity_mismatch` (§6, case C) immediately.
4. Inspect unavailable/ambiguous: §6, case B.

A mutable tag being present locally is irrelevant once a pin exists — the
agent never asks Docker about the tag at all in that case, and `docker run`
itself is never permitted to acquire it either, via `--pull never` (§11).

### 8. Docker command abstraction

A narrow trait, `DockerImageRuntime` — `inspect_image`, `pull_image`,
`server_platform`, `inspect_container_image_binding` — is the only new
abstraction — the smallest one needed to exercise the actual
inspect/pull/platform-selection/container-binding decision flow
deterministically without a live daemon. Every other function in
`image_identity_verification` is a pure comparison over already-fetched
data (`select_authorized_platform`, `canonicalize_repo_digest_entry`,
`repo_digests_contain`, `compare_inspected_image`, `classify_evaluation`,
`not_applicable_pinning_evidence`), unit-tested directly with synthetic
values — mirroring this exact codebase's own established pattern in
`operational_reconciliation.rs` (e.g.
`inspected_container_matches_declared_attachments`) rather than the
heavier trait/fake pattern used for the container-execution and
OCI-registry-resolution abstractions elsewhere. `CliDockerImageRuntime` is
the production, `Command::new("docker", …)`-backed implementation;
`FakeDockerImageRuntime` (test-only) is a deterministic, queued-response
fake, matching the `FakeOciHttpTransport`/`container_executor::FakeRuntime`
precedent.

Several narrow, pure helpers in `operational_reconciliation.rs` make the
governance decisions themselves unit-testable without a live daemon:
`pinned_pull_policy_args(pinned: bool)` (`--pull never` iff pinned),
`build_governed_docker_run_args(...)` (the full M1 argument vector,
excluding only the live `ensure_docker_network` side effect, which the
caller performs separately before building args), `mutation_gates_are_clear(...)`
(§14 — every M5-eligibility conjunct, named and independently testable),
`image_known_mismatch_from`/`container_binding_drift_from` (§6a — the two
outcomes that may and may not force a recreate, kept syntactically
impossible to confuse), and `post_action_container_binding_check` (§9a).

`resolve_pinned_image_outcome` still performs one redundant-but-harmless
read-only container inspect (via the trait, not an unabstracted
`Command::new`) alongside `container_attachments_match`'s own internal one,
when a container already exists. This is a documented, accepted minor
inefficiency (one extra read-only Docker call per cycle for an
already-existing container), not a correctness concern; unifying the two
call sites remains deferred.

### 9. Recognized-absent vs. every other inspect failure

`DockerImageRuntime::inspect_image` returns `Ok(None)` **only** for a
recognized "no such image"/"no such object" Docker response — every other
failure (daemon unreachable, permission error, malformed JSON, command
execution failure, or a successful command that decoded valid JSON but
produced an empty array) is `Err(_)`, and `Err(_)` is never treated as proof
of absence. Only `Ok(None)` may lead to a pull attempt; `Err(_)` always
produces `blocked_image_identity_unverified` (`inspect_unavailable`)
without ever calling `pull_image`. The same rule applies symmetrically to
`inspect_container_image_binding`: `Ok(None)` and `Err(_)` are BOTH
`ContainerBindingObservation::Unavailable`, never silently "nothing to
check" (§3). This distinction lives entirely inside the trait contract (the
pure JSON-interpretation half is additionally factored into
`interpret_successful_image_inspect_json`, directly unit-tested with
synthetic `Vec<Value>` inputs), so `evaluate_image_identity`'s own logic
stays a simple match on `Result<Option<_>, _>` rather than needing its own
string-sniffing.

### 9a. Post-action container-binding verification

A successful pinned M1 create (a bare M1, or a governed M3/M4 recreate that
itself ends in a fresh M1 — reported `execution_status == "deployed"`) only
proves the IMAGE OBJECT verified; it does not, by itself, prove the
newly-created CONTAINER is actually bound to it. Immediately after such a
create, the per-service loop performs one additional READ-ONLY
`inspect_container_image_binding` call and folds the result through the
pure `post_action_container_binding_check` function:

- Matches the verified target's `actual_image_id`: evidence's
  `actual_container_image` is set to the observed id; `execution_status`/
  `message` are unchanged (`"deployed"` stands).
- Mismatches: evidence still records the (wrong) observed id, but
  `execution_status` becomes `"container_binding_unconfirmed"` with a
  truthful message — convergence is never claimed for a binding that does
  not match.
- Cannot be inspected (`Ok(None)`/`Err(_)`): same
  `"container_binding_unconfirmed"` status; full binding verification is
  never claimed.

This check is read-only and evidence-only: it never perturbs
`execution_projection`/`execution_generation`, and it introduces no new
executor abstraction — it is a narrow inline seam (a pure decision function
plus one trait call), not a rewrite of `execute_trusted_container_intent`.

### 10. Convergence — implemented for non-executing cycles too

The read-only preflight (§2) is computed **unconditionally**, once per
service, regardless of `execute_requested`/`control_plane_authorized`/
`rollback_requested` — it never mutates anything, so it never needs to wait
for a gate that exists to prevent mutation. Its `PreflightOutcome::KnownMismatch`
result (§6, TARGET IMAGE only) and `ContainerBindingObservation::Mismatches`
(§6a, EXISTING CONTAINER only) are both folded directly into the
pre-existing, unconditional `attachment_mismatch` computation (which
already runs for both executing and non-executing/"suggested" cycles): a
KNOWN, actively-contradicting target mismatch, or a KNOWN existing-container
drift, both make the observation non-converged even when `--execute` was
never passed. `NoSelectableReference`/`UnverifiedWithReference`/`Unavailable`
do **not** force non-convergence — unverifiable evidence remains
best-effort/suggested-only, never fail-closed, for a non-mutating
observation.

Critically, this evidence-and-drift effect is the ONLY thing a non-executing
cycle's image-identity result may have. `image_identity_final_block` (§14)
— the mutation-oriented `blocked_image_identity_*` status code — is only
ever computed when `execute_requested && control_plane_authorized`; for
every other cycle (non-executing, or executing-but-unauthorized),
`resolve_pinned_image_outcome` is never even called (so no acquisition can
be attempted and no M5 lease/boundary record is produced), and the raw
preflight evidence is recorded as-is. This means a non-executing cycle can
never be misreported as having had a governed mutation blocked — it falls
through to the ordinary `"suggested"` status exactly as any other
non-executing service does, with the image evidence/drift already
reflected in `attachment_mismatch` above.

Separately, `action_code_zero_may_report_converged` refuses `"converged"`
for a governed, authorized, EXECUTING cycle whenever `stage107a_service_block`
carries an image-identity block (mismatch OR unverified-in-`required`), OR
(v3 addition) whenever `image_identity_pending_in_required_mode` is set —
WASM reported `action_code == 0` (no host action requested) for a
`required`-mode artifact whose pinned image has never actually been
inspected locally (`PreflightOutcome::NeedsPull`, gated at
`execute_requested && control_plane_authorized`) — reported as the distinct,
non-mutating `"image_identity_unconfirmed"` status rather than a false
`"converged"`. `legacy_partial` is deliberately unaffected by this flag —
it keeps its existing, weaker "may still proceed" evidence semantics.

### 11. `--pull never` — M1 never acquires content outside M5

`pinned_pull_policy_args` adds `--pull never` to `docker run` whenever
`runtime_image_override.is_some()` (a verified pin is in effect this
cycle). Unpinned (mutable-tag) execution is completely unaffected — no flag
is added, preserving Docker's ordinary implicit-pull behavior exactly as
before this ADR. With this flag, the residual TOCTOU (the local image
being removed out-of-band between M5's verification and M1) becomes
truthful: M1 fails loudly (no such image), rather than Docker silently
performing an unrecorded second acquisition of the same content. Since M1
and M5 always name the same immutable reference, no WRONG content could
ever result even without this flag — but the flag is what keeps acquisition
itself an M5-only event, matching the M1/M5 boundary model exactly, not
merely "harmless in practice."

### 12. Evidence isolation, `pull_disposition`, and not-applicable evidence

`VerifiedImageIdentity` (`authorized_execution_reference`,
`authorized_manifest_digest`, `authorized_config_digest`,
`selected_platform`, `actual_repo_digests`, `actual_image_id`,
`actual_container_image`, `target_verified`, `reason_kind`,
`container_binding_status`, `checked_at_utc`, `pull_disposition`) is
attached to `OperationalServiceReconciliationReport.verified_image_identity`
— evidence only. It is **not** reachable from `OperationalArtifactAssignment`
at all, the only type `execution_projection::compute_projection`/
`recompute_digest` ever read, so no value inside it (including
`checked_at_utc`, `actual_container_image`, or `container_binding_status`)
can ever perturb `execution_generation`/`execution_projection_digest` —
structurally guaranteed by type separation, not merely by convention, and
confirmed by a dedicated regression test.

**§12a. `pull_disposition` vocabulary:** `already_present` / `pull_performed`
— mechanical facts about what was DONE, never a verification-outcome claim.
`target_verified`/`reason_kind` on the SAME evidence record carry the
actual TARGET-image result, so evidence can never contradict itself.
`pull_performed` means a pull happened and the evidence shown is the
RE-inspection that followed it, regardless of whether that re-inspection
then verified, mismatched, or remained unverified.

**§12b. Not-applicable evidence (v3 addition).** An artifact this module
was never even asked to pin-verify — `local_build`, `legacy_unclassified`,
an unresolved registry artifact (no `resolved_image_identity` at all), or
one whose pin failed the structural-validity check — still gets an explicit
`VerifiedImageIdentity` record: `target_verified: false`, `pull_disposition:
not_applicable_unpinned_source`, `container_binding_status: not_applicable`,
and a `reason_kind` naming which of those four cases applies
(`local_build_not_pinnable`, `legacy_unclassified_not_pinnable`,
`registry_unresolved`, `registry_pin_structurally_invalid`). This
construction requires a resolved trusted local container mapping too, not
merely an assignment — an assigned artifact with NO local mapping at all
is a separate, `blocked_local_mapping`-flavored concern this field never
attempts to describe, and is correctly left absent rather than mislabeled
as an unpinnable/invalid source. `verified_image_identity` is therefore
present whenever BOTH an artifact is assigned AND a trusted local mapping
resolves for it this cycle (i.e. whenever `trusted_container_name`/`trusted_image`
are also present), and its absence unambiguously means "no artifact was
assigned, or none has a local mapping" — never "everything was fine,
nothing needed checking." This matters because D-Monitor snapshots feed
analytics/GNN models downstream, where an absent field is easy to misread
as "nothing to report" rather than "not applicable." Deliberately NOT
added to `ArtifactExecutionProjectionV1` (evidence-only, per the isolation
rule above).

**§12c. Side-effect evidence (v3 addition).** A successful M5 acquisition —
`acquire_pinned_image`'s `pull_image` call itself returning `Ok(())` — is
recorded via `PinnedImageResolution.acquisition_performed: bool`, set
`true` in that exact case and OR'd into the per-service
`host_mutation_performed` (and therefore the cycle-wide
`side_effects_performed`) regardless of what the mandatory post-pull
re-verification subsequently finds. A pull that succeeds but is followed by
a blocking mismatch, or by a hard failure that prevents any container from
ultimately starting, is never reported as "no side effects" — the host's
local Docker image store genuinely changed, and operational evidence must
say so truthfully.

**§12d. `target_verified` / `container_binding_status` / `runtime_identity_verified()`
(v4 addition).** The evidence field formerly named `verified` is renamed
`target_verified`, specifically so it can never be misread as covering
EXISTING CONTAINER binding too (the v3→v4 review's finding: a downstream
consumer reading `verified: true` next to an absent
`actual_container_image` could not tell "no container to check" from
"binding inspection failed" from "container disappeared between
observations"). The EXISTING CONTAINER dimension is now an explicit,
ALWAYS-present string field, `container_binding_status` — one of
`not_applicable` / `matches` / `mismatches` / `unavailable` (mirroring
`ContainerBindingObservation` one-to-one via `as_status_str()`) — never
left to be inferred from `actual_container_image` being absent. A derived
convenience METHOD (never a second stored/serialized field, per the
review's explicit "no confusing duplicate truth fields" instruction),
`VerifiedImageIdentity::runtime_identity_verified()`, returns `true` only
when `target_verified && container_binding_status == "matches"` — the full
combination an operator or D-Monitor/GNN consumer actually cares about
when asking "is this service genuinely running the authorized content."
`ImageIdentityEvaluation.verified` (the internal, non-serialized,
target-only field `classify_evaluation` reads) is unchanged — it never
left the module and was never the source of the ambiguity this section
addresses.

### 13. Path B — explicit exclusion, recorded as debt

`DATUM/src/agent/service_deployment_reconciliation.rs` and
`DATUM/src/agent/integrated_service_evidence.rs` form a pre-existing,
parallel container-mutation path that predates (and is architecturally
disconnected from) the Option-A/Stage-107 authorized-snapshot model
entirely: it loads a `ServiceArtifact` from a **local file path**
(`integrated_service_evidence.rs`'s `load_service_artifact`), not from the
`operational-reconciliation-context` snapshot, and its own
`ContainerDeploymentSpec` (in `agent/service_artifact.rs`) has **no**
`image_source_kind`/`resolved_image_identity` field at all. It performs
real `docker rm -f`/`docker run` using the declared image string with zero
content verification of any kind.

Stage 107B-B does **not** modify this path — this is out of scope by
explicit decision. It must never be cited as satisfying any Stage 107B-B
guarantee. It is recorded here as explicit deferred technical debt: a
future decision is needed on whether to retire it or bring it under the
Option-A/Stage 107 authorization and image-identity model.

### 14. The gate-ordering rule and M5 eligibility

**No governed M5 `docker pull` (or any other governed mutation) may occur
before every OTHER governance/policy gate that already prohibits host
mutation at this point has been confirmed clear — including the case where
nothing needs to happen at all.** Concretely, the per-service loop in
`run_operational_reconciliation_from_bootstrap` proceeds in this order:

1. Observe the container (`inspect_container`) — unchanged, read-only.
2. Compute the read-only image-identity **preflight** (§2/§10) —
   unconditional, never pulls. Derive `image_known_mismatch` (§6, TARGET
   IMAGE only), `container_binding_drift` (§6a, EXISTING CONTAINER only),
   and `expected_runtime_image_reference` (§1) from it.
3. Fold `image_known_mismatch` and `container_binding_drift` into the
   unconditional `attachment_mismatch` (§10/§6a), computed against
   `expected_runtime_image_reference`.
4. Compute `stage107a_service_block_before_image` (generation/projection
   binding via `stage107a_service_check`, plus required-mode completeness
   via `required_completeness_gaps`), `config_content_block`,
   `dependency_blockers`, `attachment_policy_block`,
   `local_container_execution_allowed`, `batch_secret_resolution_error`,
   `secret_resolution_error`, `observation_error`, `completion_failed` — all
   pre-existing, per-service or cycle-wide gates, computed exactly as
   before.
5. Compute `mutation_gates_are_clear(...)` (§8): a named, individually
   testable function over EVERY one of: `execute_requested`,
   `control_plane_authorized`, `!rollback_requested`, a usable local
   mapping, `dependency_blockers` empty, the pre-image Stage 107A block
   clear, the config-content block clear, the attachment-policy block
   clear, local execution allowed, both secret-resolution gates clear, the
   Docker observation gate clear, the one-shot completion not having
   failed, and — critically — `wasm.action_code != 0`. Stage 107A treats
   `action_code == 0` as "no new host action requested," and M5 is itself a
   host mutation: it must never fire merely to populate or verify a cache
   when nothing else requires it.
6. Only if the preflight outcome is `NeedsPull` AND `mutation_gates_clear`
   is true, AND ONLY when `execute_requested && control_plane_authorized`
   (§10): `resolve_pinned_image_outcome` performs a fresh M5
   `check_boundary` (the lease can still independently block here — Stage
   107A lease semantics are completely unchanged) and, if that passes,
   calls `acquire_pinned_image`, recording `acquisition_performed` (§12c)
   the moment the pull itself succeeds.
7. The final TARGET-IMAGE outcome (verified / mismatch / unverified,
   whether from the original preflight or from the post-acquisition
   re-evaluation) is folded into `stage107a_service_block` via
   `fold_terminal_outcome` (§6) — ONLY for an executing, authorized cycle
   (§10) — and `pinned_execution_reference` is set whenever the outcome
   permits it. The FINAL `container_binding_status` for this same cycle
   (also post-acquisition, if a pull happened) additionally feeds
   `container_binding_unavailable_in_required_mode_from` (§6b) — a plain
   `bool`, v5 correction, NEVER folded into `stage107a_service_block` (that
   was the v4 shape; it was too broad in two ways at once — see §6b/§6c
   and "Prior review rounds" below).
8. `rollback_service_block_from` (§6c) is computed separately, for the
   rollback branch only, from `stage107a_service_block_before_image` and
   `attachment_policy_block` alone — it does not even accept
   `image_identity_final_block` as a parameter, so no runtime
   image-verification outcome can ever prevent M8.
9. M3/M4/M2/M1 proceed, each still independently re-checking a fresh lease
   immediately before its own mutation, with `container_binding_drift_for_execution`
   (v6: re-derived from step 7's FINAL evidence, never the stale
   pre-acquisition `container_binding_drift` computed in step 2 — §6a)
   additionally forcing M3 (never M2) for a drifted existing container,
   and the `container_binding_unavailable_in_required_mode` REUSE GUARD
   (consulted inside `execute_trusted_container_intent`, strictly AFTER
   the M3/M4 recreate checks) preventing M2 (or an "already
   running"/converged report) outright — but never an independently
   required M3/M4 recreate — for an unavailable binding in `required` mode
   (§6b). A successful M1 (fresh create) is followed by the read-only
   post-action binding check (§9a), which also refreshes
   `container_binding_status` to match what was just observed on the FRESH
   container.

`resolve_pinned_image_outcome`'s `mutation_gates_clear` parameter is the
ONLY thing it consults to decide whether a `NeedsPull` outcome may proceed
to acquisition — this makes the rule directly testable: passing
`mutation_gates_clear: false` for any reason must yield zero `pull_calls`,
proven once, generically, rather than needing many separate integration
tests that each stand up a full reconciliation cycle. `mutation_gates_are_clear`
itself is unit tested with every single conjunct flipped in isolation.

## Consequences

- A `registry`-sourced, correctly-pinned catalog artifact now executes
  content DATUM has independently verified against DServer's authorized
  identity — a tag existing locally, or a successful `docker pull`, is no
  longer sufficient on its own.
- A legitimate re-pin (new authorized target, old container still bound to
  the prior one) is repaired via a governed recreate rather than deadlocking
  as a permanent mismatch block (§6a).
- `local_build`/`legacy_unclassified` remain permanently unverifiable/
  `required`-ineligible, with explicit not-applicable evidence rather than
  silent absence (§12b).
- No change to `ArtifactExecutionProjectionV1`'s shape in either crate; no
  cross-crate fixture needed updating for this slice.
- Stage 107B-B is not full required-mode E2E closure (Stage 107B-D) and
  does not address configuration-mount content materialization (Stage
  107B-C).

## Deferred (explicitly, not implicitly)

- Unifying the two per-cycle container-inspect calls (one inside
  `container_attachments_match`, one inside `resolve_pinned_image_outcome`'s
  preflight) into a single fetch.
- A real ARM-variant-authoritative daemon query (today always `None`);
  would allow correctly selecting between two same-architecture variants
  instead of failing closed as `Ambiguous`.
- True cross-command Docker transactionality (not offered by the Docker
  CLI/API surface) — `--pull never` (§11) removes the specific acquisition-
  outside-M5 risk, but a daemon-level race between M5's verification and
  M1's `docker run` reading the same image object is still, in principle,
  irreducible with the CLI surface alone.
- Path B (`service_deployment_reconciliation.rs`/`integrated_service_evidence.rs`)
  retirement or integration into the Option-A/Stage 107 model.
- Stage 107B-C (configuration bind-mount content pinning) and Stage 107B-D
  (full required-mode E2E closure).
- A pre-existing (not Stage-107B-B-introduced, not touched by v4, v5, or
  v6) class of edge case, broadened by the v5→v6 review from the v4→v5
  review's narrower framing: `execute_trusted_container_intent`'s
  `Result<(String, String, bool)>` return shape collapses ANY `Err` from
  ANY step into a single, uniform `host_mutation_performed = false` —
  regardless of whether an EARLIER step in that SAME call already
  performed a real mutation. Concretely: `docker run` (M1) succeeds but a
  subsequent `post_install` lifecycle command then fails; M3's `docker rm
  -f` succeeds but its recursive recreate call fails at a later step; M4's
  `docker rm -f` succeeds but its recursive recreate call fails at a later
  step; or an earlier M7 (`docker network create`) / lifecycle-phase
  mutation is followed by a later failure in the same call. Recorded
  generically as: "partial host mutations can be under-reported when a
  later step in the same executor call returns Err." Not a Stage 107B-B
  P0/P1 — fixing it would require redesigning lifecycle-phase execution
  and/or `execute_trusted_container_intent`'s return shape, out of scope
  here; left for future D-Monitor/observability-truthfulness work.

## Prior review rounds (compact history)

- **v1 → v2** corrected three P0 findings — M5 could pull before other
  Stage 107A/107B-A governance gates were evaluated (§14); `legacy_partial`
  could fall back to the mutable tag when verification was unavailable
  (§6); a pinned `docker run` specified no pull policy, letting Docker's own
  implicit-pull silently re-acquire outside M5 (§11) — and six P1 findings
  — container-inspect failure degraded to "nothing to check" instead of
  `container_binding_unavailable` (§3/§9); the old lifecycle `pull_image`
  command could still, in principle, become an unabstracted second M5
  (§8); `pull_disposition` could contradict `verified` (§12a); a
  successful-but-empty `docker image inspect` was treated as absence (§9);
  the pinned-execution guarantees were only code-reviewed, not directly
  unit tested (§8); non-executing cycles never verified image identity at
  all (§10).
- **v2 → v3** (this document's current state) corrected four further P0
  findings — the outer, pre-acquisition attachment comparison still
  compared against the mutable declared tag instead of the immutable
  target (§1); an existing container's binding mismatch was conflated with
  a genuine candidate-content mismatch, permanently blocking a legitimate
  re-pin's repair (§6a); the M5-eligibility gate omitted several
  independently-blocking conditions, including `action_code == 0` (§14);
  `NeedsPull` reached only because no host action was requested could still
  be reported as fully verified convergence in `required` mode (§10) — and
  five P1 findings — a successful M5 pull was not reliably reflected in
  `host_mutation_performed` when a later step blocked (§12c); a freshly
  created pinned container's actual binding was never re-checked after M1
  (§9a); a non-executing cycle's image-identity result could still surface
  a mutation-oriented `blocked_image_identity_*` status even though no
  mutation was ever attempted (§10); `local_build`/`legacy_unclassified`/
  unresolved artifacts had no explicit evidence contract (§12b); this
  document's own field-presence comment had drifted from the already-
  unconditional preflight behavior (§12).
- **v3 → v4** corrected one further P0 finding —
  `ContainerBindingObservation::Unavailable` was not enforced anywhere: a
  `required`-mode service with a verified TARGET image but an unprovable
  EXISTING CONTAINER binding could still be reported converged (Running)
  or reach M2 (Stopped), because `classify_evaluation` never read
  container-binding state at all (correctly, per §6a/§6b's split) and
  nothing else was consulting it either (§6b) — and one further P1
  finding — the serialized evidence's `verified` field conflated TARGET
  and CONTAINER truth, and an absent `actual_container_image` was
  ambiguous between "not applicable," "inspection failed," and "container
  disappeared" (§12d): `verified` renamed to `target_verified`, and an
  explicit, always-present `container_binding_status` string
  (`not_applicable`/`matches`/`mismatches`/`unavailable`) added, with a
  derived `runtime_identity_verified()` convenience method rather than a
  second stored truth field.
- **v4 → v5** (this document's current state) corrected one further P0
  finding — v4's `container_binding_unavailable_block_in_required_mode`
  folded its result into the SAME GLOBAL `stage107a_service_block` that
  remove-only rollback (M8) also consulted, meaning an authorized rollback
  could be refused solely because the target image was
  mismatching/unverifiable/unselectable or the existing binding was
  unavailable — directly contradicting the frozen M8 remove-only contract
  (§6c, new: `rollback_service_block_from` now does not even accept an
  image-identity block as a parameter) — and one closely-related P1
  finding — that same v4 mechanism also blocked EVERY governed execution
  path for the service (including an independently-required M3/M4
  recreate, which is the SAFE corrective action for an unverifiable
  existing container) rather than acting as a narrower "do not reuse the
  unknown container" guard (§6b, reshaped:
  `container_binding_unavailable_in_required_mode_from` is now a plain
  `bool`, threaded directly into `execute_trusted_container_intent` and
  consulted only AFTER the M3/M4 recreate checks, never folded into
  `stage107a_service_block` at all).
- **v5 → v6** (this document's current state) corrected one further P0
  finding — `container_binding_drift` was computed exactly once, from the
  INITIAL, pre-acquisition preflight, and passed unchanged into
  `execute_trusted_container_intent`. When the initial preflight outcome
  was `NeedsPull` (target image absent locally), the initial
  `container_binding` observation is necessarily `NotApplicable` — there
  is no local target image object yet to compare the existing container
  against at all — so a REAL mismatch that the MANDATORY post-M5
  re-evaluation discovers (once the pull completes and the target image
  object exists to compare against) was silently lost: the stale,
  pre-acquisition `false` was still what
  `execute_trusted_container_intent` received, meaning a Stopped existing
  container bound to the WRONG image could in principle still reach M2
  even immediately after the agent had just observed the mismatch (§6a,
  fixed: a new `container_binding_drift_for_execution` value is re-derived
  from the FINAL, post-acquisition evidence
  (`resolve_pinned_image_outcome`'s returned `container_binding_status`)
  immediately after acquisition completes, and it is THIS value —  never
  the stale one — that is passed to `execute_trusted_container_intent`).
  The OUTER, pre-acquisition attachment comparison (WASM's initial input,
  non-executing convergence) correctly continues to use the original,
  initial-preflight `container_binding_drift`, unchanged, per the review's
  explicit instruction. No P1 findings this round; the review's own P2
  (side-effect-evidence-audit.txt's deferred-debt description) was
  broadened, as instructed, to cover M3/M4/M7/lifecycle-phase mutations
  generically, not fixed.

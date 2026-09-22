# Phase 113A review package — final closure pass 6.1 (diff-hygiene recovery)

This worktree contains the canonical DForward/DIoT and DMap/2 control-plane
foundation on baseline `1d7ec64f434ddc9ef5a952c4fd05262c03db07eb` (== HEAD ==
`origin/main`; nothing committed — all Phase-113A work is uncommitted).

The production schemas are `datum.dforward/1`, `datum.dmap/2`,
`datum.ddeploy-proposal/2`, and `datum.ddeploy-acceptance/2`. Historical VS1
DForward fixtures use `datum.x-fixture.vs1-dforward/1` and cannot claim the
production schema (regression-tested). DMap/1 and DDeploy/1 wire contracts
remain unchanged.

## What each closure pass established

**Pass 1** — `phase113_v2: DDeployStateV2` persists additively inside the
existing `DDeployStateV1`/`DOMAIN_DDEPLOY_STATE` blob. `active_placement_authority`
is the one common v1/v2 active-authority accessor, rejecting simultaneous
v1/v2 authority for the same key. `compute_project_node_commitments_versioned`
is genuinely wired into `accept_v2_proposal`'s real cross-application capacity
enforcement, with the same-application-replacement exclusion rule (exact-fit,
CPU/memory overcommit, v1→v2 and v2→v2 redeployment double-counting, and
atomic rejection are all regression-tested).

**Pass 2** — `active_placement_authority` was extended (proposal_id,
acceptance_id, exact DGraph ref+snapshot, and DMap/2-specific extras: DForward
ref, DCall realizations, DIoT placements) and migrated into the full
production D-Code/D-Call chain: `authorize_dcode_invocation`,
`derive_dcall_route`, and `post_request_grant` all consume this single shared
accessor instead of reading `active_by_application` directly — **this closes
the "Phase-112 consumers not yet migrated" gap the original ADR-0023
disclosed.** DMap/1 behavior is unchanged and regression-tested. A fail-closed
boundary exists in `post_request_grant`: when active authority is DMap/2 and
the D-Call's realization kind is `via_diot_pubsub` (no runtime transport
exists — that is explicitly Phase 113B), the request is rejected with
`dcall_realization_transport_not_implemented` BEFORE endpoint lookup, source
stale-authority rechecking, message-limit checks, or grant issuance. There is
no Phase-112 HTTP fallback.

**Pass 3** — real project-scoped `ServiceArtifact` registry resolution is
wired into DIoT proposal-time binding (`DIoTArtifactAuthorityBindingV2 {
artifact_id, execution_generation, execution_projection_digest }`, using only
fields the server genuinely owns), persisted inside the immutable
`DDeployProposalV2` and covered by `proposal_digest`. Acceptance re-resolves
the current registry and rejects stale generation/projection-digest atomically
(no active-state mutation on rejection) — **this closes the "live registry
resolution is a follow-on integration" gap.**

**Pass 3.1** — `DDeployStateV2::validate_integrity()` now requires every
acceptance's and every active authority's `proposal_id` to resolve to an
actually-existing proposal record, mirroring a precedent fix already applied
to v1. Closes a dangling-proposal-id relational-integrity gap that would have
let an active DMap/2 survive reload validation after its originating
proposal's governance evidence (including the DIoT artifact binding above)
disappeared from persisted state.

**Pass 4** — the first DDeploy/2 HTTP API surface: `POST
/api/v1/ddeploy/v2/proposals/validate`, `POST .../proposals`, `GET
.../proposals/:id`, `POST .../proposals/:id/accept`, `GET .../authority` — all
thin wrappers over the real core governance path, never duplicating capacity
or ServiceArtifact logic in the handler. There is no endpoint resembling
`set-active-dmap-v2`/`activate-map`; the only activation path is
immutable-proposal → explicit-acceptance → server materialization. **This
closes the "existing HTTP handlers remain v1-only" gap** for the five
governed operations above (broader v2 API surface area beyond these five
remains out of this scope).

**Pass 4.1** — `DDeployProposalV2` gained `source: ProposalSource` (reusing
v1's exact type) and `created_at_utc`, both digest-covered and
integrity-validated on reload. Acceptance gained `expected_proposal_digest`
CAS, checked before any state mutation, including on idempotent replay (a
correct digest replay still returns the same acceptance; a wrong digest still
rejects even on replay).

**Pass 5** — a production-safe, authority-proving node-projection entry point
(`dforward_projection.rs::governed_node_projection`) loads current accepted
authority via `active_placement_authority`, verifies it is genuinely
`datum.dmap/2`, and derives a strictly node-local, deterministic projection
that never represents `any_element` requirements as satisfied
(`readiness_authority=dmonitor`/`readiness_state=pending_fresh_evidence`,
never `ready=true`). The placeholder positive fixture is gone; a fully real,
mutually-consistent positive fixture bundle (DGraph/DContinuum/DForward/DMap-2/
DDeploy-proposal-2/DDeploy-acceptance-2) exists with every digest computed by
the real production canonicalization functions. All four production schemas
have a complete negative-fixture family (8 DForward + 12 DMap/2 + 6 DDeploy/2
= 26 cases), each independently reproduced by a real Rust test against the
actual validator/deserializer/`validate_integrity()` path. DServer/DATUM
mirror parity exists for both `canonical_dforward` and `canonical_dmap_v2`.

**Pass 6 (this pass, final closure)** — regenerated all evidence files from
current state; ran the full broad DServer and DATUM suites (not just focused
tests) for the first time since Pass 5; re-ran the real Phase-112 cross-node
D-Call subprocess integration proof (7/7); ran all 11 root checkers; ran
`mkdocs --strict`; performed the full 19-point adversarial state review
(all 19 independently verified true with code/test evidence — see below).

## Incident during Pass 6, recovered in Pass 6.1 — read before treating this branch as closed

A `cargo fmt` command intended to be scoped to 2 specific new files ran far
more broadly than intended during Pass 6, cosmetically reformatting roughly
158 tracked `.rs` files across both crates (dirty-path count rose from 47 to
195). Pass 6.1 performed a byte-safe recovery:

- 85 files confirmed byte-identical to HEAD (SHA-256 vs `git show HEAD:<path>`)
  — no action needed.
- 43 files confirmed pure accidental churn on files Phase 113A never
  intentionally touched — restored to exact HEAD bytes, each individually
  hash-verified.
- 42 files confirmed to mix genuine Phase-113A content with accidental
  churn (the sensitive category) — reconstructed via a `git diff -w`/`patch
  --fuzz` technique applied onto a clean HEAD base, each individually
  verified (normalized-content equality) against its pre-reconstruction
  content before being accepted. **39 of 42 succeeded.**
- **3 files could not be safely reconstructed even at maximum fuzz
  tolerance** (`dserver/src/core/ddeploy.rs` — 219 `-w` hunks, 13 still fail
  at `--fuzz=100`; `dserver/src/api/ddeploy.rs` — 24 hunks, 4 fail;
  `DATUM/src/dcall/delivery/tests/endpoint_authority.rs` — 7 hunks, all
  fail). These three remain in their pre-recovery state: functionally
  correct and fully test-verified, but still carrying accidental formatting
  churn on real content. This is a deliberate, safe, honestly-disclosed
  outcome — not an oversight — per the explicit instruction that a false
  restoration risking real content loss is strictly worse than leaving a
  file temporarily dirty.

Full functional/test/checker/docs validation was re-run AFTER this recovery
(not carried forward): every gate is green, including the required Phase-112
real integration proof (7/0). Full detail in `test-results.txt`.

**Because 3 files still carry accidental formatting churn on top of real
content, Phase 113A's own diff-cleanliness bar (matching Pass 1's standard)
is not fully met repository-wide, even though every functional/test/checker
gate is genuinely green and regression-free.** A follow-up pass could
attempt manual, section-by-section reconstruction of these 3 files
specifically (rather than whole-file automated patching, which has now been
tried and shown insufficient for files this large/complex) before this
branch is considered fully diff-clean.

## Adversarial state review (19 points) — all verified, zero OPEN

Every point below was independently verified against real code/tests during
Pass 6, not asserted: one active DMap authority per application (dual-
authority reject guard in `validate_persisted_state_integrity`); v1→v2/v2→v2
monotonic revisions (`v1_to_v2_redeployment_...`/`v2_to_v2_redeployment_...`
tests); no same-app double counting (3 dedicated tests); active DMap/2
participates in planner commitments (`compute_project_node_commitments_versioned`);
ServiceArtifact binding is server-owned (`resolve_diot_artifact_authority`);
stale ServiceArtifact acceptance fails atomically; proposal provenance and
timestamp are both digest-covered (tamper-after-creation tests); acceptance
CAS works including on replay; dangling proposal corruption is rejected;
active DMap/2 is consumed by D-Code and D-Call (tests in `dcall/tests.rs`);
exact DMap/2 digest propagates through authority evidence; `via_diot_pubsub`
cannot fall back to Phase-112 HTTP; governed node projection comes from
current accepted DMap/2; `any_element` remains DMonitor-pending; real positive
fixtures use no fake zero digests (the one grep hit was descriptive prose in
an `x_note` field, not an actual field value); all 4 production schemas have
unique wire-family ownership (checker-verified); VS1 DForward remains
`datum.x-fixture.vs1-dforward/1`; no MQTT/runtime Phase-113B implementation
exists anywhere (grep-verified).

## Explicitly deferred (Phase 113B, out of scope)

MQTT/middleware transport, retries, listeners, publishers, broker lifecycle,
cross-node readiness, DMonitor implementation, runtime DIoT execution,
recursive D-Engine dispatch, and automatic retry semantics are all
intentionally not implemented anywhere in this branch.

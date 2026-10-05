# Phase 137 — Proposal Review / Acceptance (Console)

## Purpose

Phase 136 (Placement Workspace) can generate, edit, validate, and persist a Pending placement proposal, but a Pending proposal is explicitly **not active authority** — nothing in Phase 136 can make it so. Phase 137 closes that loop with a single-proposal detail/review page that exposes the one authority transition DServer's `POST /api/v1/ddeploy/proposals/:id/accept` performs: turning a Pending proposal into the project's accepted placement for its application.

Accepting a proposal changes **placement authority only**. It does not deploy, reconcile runtime, invoke DCode, mutate DMonitor, or prove runtime convergence — those remain Phase 138's (not-yet-built) concern. This boundary is restated, never weakened, from Phase 136's own doc.

## Authority boundary — unchanged, restated

Planned (candidate) → Proposed (Pending proposal) → **Accepted (active authority)** → [Phase 138: deployed/reconciled/converged — not built]. Phase 137 owns exactly one arrow: Proposed → Accepted. It never reaches forward into the next one.

## Proposal lifecycle is exactly Pending/Accepted

`ProposalLifecycleState` has exactly two variants (confirmed by direct source read of `dserver/src/core/ddeploy.rs`): `Pending` and `Accepted`. There is no Rejected, Cancelled, Expired, Superseded, or Withdrawn state anywhere in the backend, and Phase 137 does not invent one. A Pending proposal that will never be accepted simply stays Pending forever — there is no action that removes or voids it. Multiple Pending proposals for the same application can coexist; this page never implies any one of them is canonical over another, since nothing in the backend ranks them.

## `accepted_by` is an audit label, not authentication

DServer has no application-layer authentication (ADR-0013 §6). `accepted_by` is a caller-supplied free-text label recorded verbatim on the `DDeployAcceptanceV1` record for audit/history purposes — it is never verified against any identity provider and must never be rendered or described as "authenticated user," "signed-in operator," or "verified identity." The Console collects it from an explicit, required text field labeled "Acceptance audit label," with inline help text disclosing exactly this.

## What acceptance actually does (backend, unmodified)

Re-confirmed this phase by direct source read, not assumed:

1. Proposal lookup (`NotFound` → 404 `proposal_not_found` if absent, or if it belongs to a different project — DServer masks a cross-project proposal as Not Found rather than leaking its existence).
2. Unconditional digest/tamper check against the proposal's own persisted digest, before the replay branch.
3. **Idempotent replay short-circuit**: if the proposal is already `Accepted`, the exact same `AcceptOutcome` is reconstructed historically — zero re-validation, zero mutation — with `idempotent_replay: true`. `currently_active` tells the truth about whether that materialization is still the live active D-Map (a later acceptance may have superseded it).
4. [Pending only] D-Continuum authority drift check (`check_dcontinuum_authority`) — `F_DCONTINUUM_NODE_UNREGISTERED` / `F_DNODE_STRUCTURAL_CONFLICT` / `F_DNODE_CAPABILITY_INVALID` findings on drift (`AcceptError::StaleAuthority`, 409).
5. Compare-and-swap on `expected_active_dmap_revision` against the current active revision (`AcceptError::Stale`, 409 `stale_active_dmap_revision`) — DServer is the sole authority on staleness; nothing client-side ever rebases a proposal to "fix" this.
6. Full structural re-validation (`validate_full`) and operational-projection build.
7. Global physical D-Node capacity gate (Phase 136A, frozen, unmodified) — `AcceptError::Invalid`/`Collision`/`DCodeBindingInvalid` → 422 `candidate_dmap_invalid` with real findings.
8. Commit: the active D-Map for the application advances, the acceptance record is persisted, and the proposal's own `.state`/`.accepted_acceptance_id` are set — the **only** production write to a proposal after its creation.
9. After locks are released, a real `dmap_accepted` event is emitted (`event_producers::acceptance_event_v1`) — already consumed by the existing Project Overview "recent events" read model. **Phase 137 adds no new audit page**; this event trail already exists.

None of this was modified. Every backend confidence test exercising these paths (stale CAS, stale D-Continuum authority, global capacity, concurrent accept, tamper, idempotent replay) was re-run, unchanged, against the untouched backend — see Validation below.

## Route

`GET /projects/:project_id/proposals/:proposal_id` → `{ kind: "project_proposal_review", projectId, proposalId }` (`console/src/app/router.ts`). `proposalReviewPath(projectId, proposalId)` builds it; `validProposalId` (an alias of `validProjectId` — proposal ids are server-generated, opaque, but share the same bounded/control-character-free shape guard). There is **no proposal-listing route or endpoint** — DServer exposes only `GET .../proposals/:id` (exact id), never `GET .../proposals`. The only entry point into this page is Phase 136's own "Review proposal" link, shown (not auto-navigated to) immediately after a successful "Create proposal."

## Files

- `console/src/pages/proposal-review.ts` — the page. Entirely new.
- `console/src/types/placement.ts` — extended with `OperationalProjectionRef`, `DDeployAcceptance`, `AcceptResponse`, `AcceptErrorFinding` (mirrors `dserver/src/core/ddeploy.rs`/`dserver/src/api/ddeploy.rs` field-for-field).
- `console/src/pages/placement-model.ts` — extended with `validAcceptResponse` (and its private helpers), reusing the existing `contractRef`/`canonicalDMap`/`timestamp`/`digest` validators verbatim.
- `console/src/api/placement.ts` — extended with `PlacementClient.acceptProposal(proposalId, acceptedBy, expectedProposalDigest?)`, the only call in this client that can mutate accepted placement authority.
- `console/src/pages/placement.ts` — `CHANGE_LABEL`/`CHANGE_TONE` exported for reuse; the "Create proposal" success state gained a "Review proposal" link (`proposalReviewPath`) alongside its existing "not active authority" copy, which is unchanged.
- `console/src/main.ts` — routes `project_proposal_review` to `proposalReviewView`.

No `proposal-review-model.ts` was added: every piece of pure logic this page needs (`placementDiff`, the response validators) already exists in `placement-model.ts` and is imported directly, never duplicated.

## What the page shows

- **Identity**: `proposal_id`, `project_id`, `application_id`, `source`, `created_at_utc`, `proposal_digest`, and a Pending/Accepted lifecycle badge — kept visually distinct from the authority-context section below, never merged into one generic "status."
- **References**: full `dgraph_ref`/`dcontinuum_ref` (id, revision, digest — digests are never hidden or truncated) and `expected_active_dmap_revision` ("none (first-deployment basis)" when null).
- **Current accepted authority (live)**: fetched fresh from `GET /active` on every load — never the proposal's own creation-time snapshot. No active D-Map at all is rendered as a normal, explicit state ("first deployment... a normal state, not an error"), never as an error or empty-table illusion.
- **Stale-basis banner**: shown only when the proposal is Pending and its `expected_active_dmap_revision` no longer equals the live active revision. Pure display — the Console never rebases the proposal or hides the mismatch; DServer alone decides whether acceptance actually fails for this reason.
- **Placement diff**: `placementDiff(currentPlacements, proposal.proposal.placements)`, the exact Phase 136 function, rendered as an accessible `Service / Current / Proposed / Change` table (no canvas — a governance review surface reads more clearly as a table, and Phase 136's visual canvas was explicitly optional). The proposal's own placement is labeled "Pending proposal" / "Accepted proposal" in the section heading — never "Candidate," since this is not a draft under local edit.
- **Source / provenance**: the `source` enum, honestly. `GET /proposals/:id` returns `DDeployProposalV1`, which (confirmed by direct source read this phase) **does not carry `planner_provenance`** — that field exists only on the backend's internal, never-exposed `ProposalRecord`. For a `deterministic`-sourced proposal this page explicitly discloses that detailed planner provenance (policy name/version, per-service decisions) is not retrievable from this lookup, rather than fabricating or silently omitting it. Source is shown for context only and is never treated as extra trust — DServer re-validates structurally and against capacity at acceptance time regardless of source.
- **Acceptance panel**: shown only when the proposal is Pending (an Accepted proposal shows its badge plus an explicit "no Reject/Cancel/Expire/Withdraw action" note instead — the control is never offered once accepted on initial load). Contains the required audit-label input, the literal no-preflight disclaimer, and the "Accept proposal" button — never Save/Apply/Activate/Deploy/Approve.
- **Phase 138 boundary note** and **navigation** ("Back to Placement" / "Create a new placement proposal," both the real `projectPath(...)/placement` route, never a hand-rolled or deployment-shaped link).

## Acceptance flow

1. Operator types a non-empty audit label (required; no default, never silently fabricated).
2. Clicking "Accept proposal" opens one `window.confirm` (consistent with every other Console confirmation precedent) naming the `proposal_id`, `application_id`, and the full diff summary, followed by all three required sentences: it changes placement authority only; it does not deploy or prove runtime convergence; it may still fail if authoritative state has changed since the page loaded.
3. On confirm: the button and audit-label input are disabled for the duration of exactly one in-flight request (a sequence token also guards against a stale response touching current state).
4. **Success** (`idempotent_replay` true or false): the page re-fetches **both** `GET .../proposals/:id` and `GET /active` — never permanently trusting the accept response's own `materialized_dmap`/`currently_active` as a lasting snapshot — then shows only real response fields (`acceptance_id`, `accepted_at_utc`, `accepted_by`, `proposal_digest`, `materialized_dmap_ref`, `operational_projection_ref`, and a truthful `currently_active` explanation). Post-accept copy states only that authority changed; it never says Deployed/Running/Healthy/Converged.
5. **Ambiguous transport failure** (`PlacementError` code `timeout`/`unavailable` — the HTTP response itself was never received, so the true outcome is unknown): the page never claims "Acceptance failed." It re-fetches the proposal; if that now shows `Accepted`, it reports the acceptance as confirmed; if still `Pending`, it reports the attempt did not go through and invites a retry; if the re-fetch itself also fails, only then does it show "Acceptance outcome could not be confirmed. Reload this page to check the proposal's current state before retrying."
6. **Definite rejection** (every other error code): the exact backend `code`/`message`/`findings` are shown verbatim — never replaced with generic prose. A stale-revision or stale-authority rejection additionally re-fetches `GET /active` (to refresh the live authority context) without touching the proposal itself or auto-rebasing anything.

## Non-goals (explicit)

- No proposal listing UI or endpoint.
- No v2 review/accept UI (v1 placement only).
- No Reject/Cancel/Expire/Withdraw/Delete-as-rejection control — proven absent by source-text-scan regression, not merely "not currently wired up."
- No new audit/history page — acceptance's existing `dmap_accepted` event already reaches the Project Overview read model.
- No client-side capacity/eligibility computation, no pretend preflight, no client-side CAS/locking standing in for the server's own compare-and-swap.
- No deployment, runtime reconciliation, DCode invocation, or DMonitor mutation. If DMonitor data is ever shown here it is explicitly contextual-only and was not required for this MVP (and is not currently shown).
- No navigation list item for "Proposals" — `PROJECT_SECTIONS` was not given a new entry; the only entry point remains Phase 136's direct link, matching the absence of a listing endpoint.
- Zero backend modification. Every backend behavior this page depends on was re-confirmed by reading, never changed.

## Tests

- `console/tests/proposal-review.test.mjs` — route parse/build round-trips (including malformed-encoding safety and length-mismatch rejection), and source-text-scan regressions proving: no Reject/Cancel/Expire/Withdraw control, no deployment/runtime-shaped call, no Deployed/Running/Healthy/Converged claim, `placementDiff`/`CHANGE_LABEL`/`CHANGE_TONE` reuse (never redefined), no "Candidate" label on the proposal's own placement, the exact "Accept proposal" button text, the exact no-preflight disclaimer, the exact three-sentence confirmation, the `accepted_by` audit-label disclosure (and absence of the forbidden authenticated-identity phrases), absence of client-side capacity arithmetic, verbatim `e.code`/`e.message`/`e.findings` preservation, the mandatory multi-call `active()` re-fetch, absence of any client-side storage-based locking, and that both `placement.ts` and `proposal-review.ts` use the real route builders for their cross-links.
- `console/tests/placement.test.mjs` — narrowed and updated: the pre-existing Phase 136 regression now scopes its "no acceptance action" scan to `pages/placement.ts` alone (the one file that must still never call accept), since `api/placement.ts`/`placement-model.ts`/`types/placement.ts` are shared files Phase 137 legitimately extended with the one real `acceptProposal` call. The method-count assertion now expects all six `PlacementClient` methods, including `acceptProposal`.
- `console/tests/proposal-review-fixture.mjs` — four deterministic scenarios (A/B/C/D below), each a self-contained project/application/revision combination so no two scenarios' `GET /active` responses collide.
- `console/tests/proposal-review-visual-fixture.mjs` + `.test.mjs` — a TEST ONLY HTTP server implementing exactly `GET .../proposals/:id`, `POST .../proposals/:id/accept`, and `GET .../active`, serving the four scenarios; validated end-to-end over real HTTP against the real Console validators. It deliberately has **no** `/deploy`-shaped or other Phase-138 endpoint, and an explicit test proves several plausible deployment/runtime-shaped guesses (`.../deploy`, `.../reconcile`) still fail with a non-200 status.

### The four fixture scenarios

| Scenario | Application | Expected basis | Current active revision | Outcome of accept |
|---|---|---|---|---|
| A — normal | app-1 | rev 3 | rev 3 | Succeeds, materializes rev 4 |
| B — stale | app-3 | rev 3 | rev 4 | 409 `stale_active_dmap_revision` |
| C — already accepted | app-4 | — | rev 4 (already materializes this proposal) | 200, `idempotent_replay: true` |
| D — first deployment | app-2 | none (null) | no active D-Map | Succeeds, materializes rev 1, all placements "added" |

### Defects found during manual browser review, and their fixes

Manual browser review (approved) exposed two real issues before this phase closed:

- **P1 — fixture-only URL-decoding bug.** `URL.pathname` preserves percent-encoding verbatim; it never decodes a path segment like `%3A` back to `:`. The real `PlacementClient` correctly `encodeURIComponent`s the proposal_id path segment (production behavior was always correct and was not changed), but `proposal-review-visual-fixture.mjs`'s own route matchers used the still-encoded captured segment directly as a `SCENARIOS` key, so a real browser request 404'd even though the fixture's own synthetic (un-encoded) contract tests stayed green. Fixed by decoding the captured proposal-id segment exactly once (`decodeProposalId()`), applied to both the GET and accept routes; malformed percent-encoding is handled deterministically (404, never a crash); the raw-colon form, unknown encoded IDs, and single-decode-only behavior are all covered by dedicated regressions in `proposal-review-visual-fixture.test.mjs`.
- **P1 — unwrapped diff table risked page-wide horizontal overflow.** `diffSection()`'s `<table>` had no `table-responsive` container and no `overflow-wrap` on its node-id cells, so a sufficiently long service_id/node_id in real data could force the whole page to scroll sideways at tablet widths, rather than the table alone. Fixed by wrapping the table in a `table-responsive` scroller and adding `entity-identity` (which carries `overflow-wrap: anywhere`) to the node-id cells, matching the rest of the Console's established identifier-rendering convention. Regression test added in `proposal-review.test.mjs`.

### Known limitation: DOM-level automated tests

Same pre-existing, environment-level limitation as Phase 135/135A/136: `happy-dom` is not installed here, so `proposal-review.ts`'s actual markup/interaction is not exercised by an automated DOM test. All automated coverage targets the DOM-independent pure logic it depends on (shared validators, `placementDiff`, route parsing) plus the real-HTTP fixture contract and source-text-scan regressions proving the governance boundaries. The manual review procedure below is the current path to verify actual rendered/interactive behavior.

## Manual visual review procedure

1. `cd console && node --experimental-strip-types tests/proposal-review-visual-fixture.mjs` (defaults to port 8139; override with `DATUM_PROPOSAL_REVIEW_FIXTURE_PORT`).
2. In another shell: `cd console && DATUM_DEV_API_TARGET=http://127.0.0.1:8139 npm run dev`.
3. Open the dev URL at each of:
   - `/projects/project-a/proposals/ddeploy-proposal:scenario-a` — **Scenario A**. Confirm: Pending badge; current authority shows rev 3; diff shows `service-a` moved node-a→node-b and `service-b` added; no stale banner. Enter an audit label, click "Accept proposal," confirm the dialog names `scenario-a`/`app-1` and the diff summary. Confirm success copy shows `acceptance_id`, re-fetched active now reads rev 4, and the Acceptance panel now shows "Accepted."
   - `/projects/project-a/proposals/ddeploy-proposal:scenario-b` — **Scenario B**. Confirm the stale-basis banner appears (expected rev 3 vs current rev 4). Accept anyway; confirm the page shows `stale_active_dmap_revision` verbatim, the proposal stays Pending and visible, and current authority is refreshed (still rev 4) without any auto-rebase.
   - `/projects/project-a/proposals/ddeploy-proposal:scenario-c` — **Scenario C**. Confirm the Accepted badge is shown with no accept control. (The page never auto-replays; this scenario's accept endpoint is reachable only for the automated contract test and `curl`/`Invoke-RestMethod`, matching the "no control exposed once Accepted on load" rule above.)
   - `/projects/project-a/proposals/ddeploy-proposal:scenario-d` — **Scenario D**. Confirm "No active D-Map yet... first deployment... a normal state, not an error," `expected_active_dmap_revision` shows "none (first-deployment basis)," and the diff shows `service-c` as "Added." Accept; confirm success materializes revision 1.
4. Resize to 768–900px: confirm every section stacks to one column with no horizontal overflow (the diff table scrolls independently inside its own container if needed).

## Validation run (final, post manual-review fixes)

- `console`: `npx tsc --noEmit` — clean.
- `console`: `npm run build` (Vite) — clean (pre-existing chunk-size advisory only, unrelated to this phase).
- `console`: `npm test` — full frontend suite, 230/230 passing (22 Phase 137 page/route tests + 10 fixture-contract tests, including the URL-decoding and table-responsive regressions, + 2 updated Phase 136 regression tests, all included).
- `console`: `npm audit --omit=dev` — 0 vulnerabilities.
- `git diff --check` — only line-ending (CRLF) advisories, no real whitespace/conflict-marker issues.
- `dserver`: `cargo test --locked core::ddeploy::tests::` — 241/241 passing, unmodified (covers stale CAS, stale D-Continuum authority, global capacity, concurrent accept, tamper, and idempotent-replay confidence tests this phase depends on).
- `dserver`: `cargo test --locked api::ddeploy::tests::` — 32/32 passing, unmodified.
- No backend file was edited in this phase.
- Manual browser review: **APPROVED** (operator-performed; not automated).

**Phase 137 has not been committed, staged, or pushed. Phase 138 has not started.**

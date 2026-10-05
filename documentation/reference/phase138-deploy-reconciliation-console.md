# Phase 138 — Deploy / Reconciliation Console

## Purpose

Phase 138C built the pull-based control-plane trigger primitive (`ReconciliationRequest`/claim/batch) a Console "Reconcile" workflow needs. Phase 138 adds the operator-facing workspace itself: accepted placement authority → inspect target nodes/services → Preview (zero mutation) → Authorize (permission only) → Reconcile (creates a request/batch) → track request/batch/attempt → show the command-side result honestly. This workspace **creates no runtime itself**. It only ever writes `ReconciliationRequest`/`OperationalReconciliationAuthorization` control state and reads back whatever the node-resident DATUM agent and DServer already report. No DMonitor-based health/convergence semantics were added beyond what this doc's own "Not built" section names; accepted placement authority semantics were not touched; `dserver` still cannot push anything to a node.

## Authority boundary — restated, not weakened

Accepted placement authority (`ddeploy`, Phase 137) → **ReconciliationRequest** (operator/system intent, Phase 138C) → **OperationalReconciliationAuthorization** (permission to mutate a host, pre-existing) → **ReconciliationAttempt** (agent-side command execution, Phase 138A) → **OperationalEvidenceEvent** (command-side evidence) → **DMonitor** (runtime observation, untouched, Phase 139+). Command success ≠ observed success. Observed ≠ healthy. Healthy ≠ converged. This workspace's own copy is scanned by `tests/reconciliation.test.mjs` to prove it never fabricates a `"Healthy"`/`"Converged"` status label and never references DMonitor at all.

## Route

`GET /projects/:project_id/applications/:application_id/deploy` → `{ kind: "project_application_deploy", projectId, applicationId }` (`console/src/app/router.ts`). `applicationDeployPath(projectId, applicationId)` builds it; `validApplicationId` is an alias of `validProjectId` (the same bounded/control-character-free opaque-id guard, since application ids are operator-chosen like project ids, not server-generated like proposal ids). **The route is application-named, but the execution it triggers is node + operational-D-Graph scoped** — see "Node-wide scope" below. There is no client-side fabrication of true per-application isolation anywhere in this workspace.

### Entry points

Added narrowly, not scattered across every view (Part A1's "where appropriate" leaves this to judgment): a "Go to Deploy / Reconciliation" link on `proposal-review.ts`'s own nav section, shown **only once a proposal's state is `accepted`** (never from a still-Pending proposal, which has nothing yet for reconciliation to target). Acceptance itself still ends at "placement authority accepted" — it never auto-navigates, and `tests/reconciliation.test.mjs` proves `proposal-review.ts` never imports the Reconciliation client or calls `createBatch` as a side effect of acceptance (Part AA).

## Data load (Part B)

On workspace load: `PlacementClient.active(applicationId)` (existing Phase 136/137 endpoint) for accepted placement authority, from which target node ids are derived (`active_dmap.placements`, de-duplicated node ids). Then, per node, `ReconciliationClient.slice(nodeId)` reads `GET /api/v1/nodes/:node_id/operational-dgraph-slice` — the **exact same endpoint the DATUM agent itself polls every cycle** (no separate, richer Console-only read model). That response carries `desired_services` (every service the node's operational D-Graph currently names, not filtered to this application — see "Node-wide scope") and the current `authorization`, if any. Nothing here reconstructs accepted authority from browser state; every value shown is read fresh from DServer on load.

## Preview (Part D)

"Preview reconciliation" creates a batch with `mode=preview` via `POST /api/v1/reconciliation-requests/batch` (the SAME batch endpoint Reconcile uses — Console never has two separate code paths for one-node vs. multi-node, or for preview vs. execute). The agent discovers and claims it through its existing poll; Phase 138C's own agent-side wiring forces `execute_requested = false` for a claimed `preview`-mode request **regardless of the process-level `--execute` capability** — Preview cannot mutate a host by construction, not by a client-side promise. Preview review renders exactly the fields the attempt record/report actually carry (service id, result — `No change`/`Mutated`/`Blocked`/`Unknown` — host-mutation flag, verbatim `execution_status`) and invents nothing richer.

## Authorization (Part E)

Reuses the pre-existing `OperationalReconciliationAuthorization` API unmodified: `POST /api/v1/nodes/:node_id/operational-reconciliation/authorize` to issue, and the `authorization` field already present on the slice read above to display current status. No parallel authorization concept was created. Status is computed **client-side only from fields the server already returned** (`allow_host_mutation`, `dgraph_id`/`dgraph_revision` vs. the slice's own current values, `expires_at_utc`) — `authorizationDisplayStatus` in `reconciliation-model.ts` is a label mapping over already-served data, never a re-implementation of DServer's own lease-expiry math (`should_restore_authorization` remains the one place that actually gates a mutation). Five states are shown: **Authorized for the current target**, **Not authorized**, **Authorization expired**, **Authorization pinned to a different target**, **No authorization issued**.

Authorization is never auto-issued by Reconcile (Part E3): the MVP flow is Preview → Authorize → Reconcile, each an explicit operator action with its own audit-label requirement. The Authorize control's own help text discloses: "Authorization permits host mutation for the exact pinned operational authority shown above. It does not itself start reconciliation." `authorized_by`, like Phase 137's `accepted_by` and Phase 138C's `requested_by`, is a caller-supplied **audit label**, never authenticated identity (ADR-0013 §6) — never rendered as "authenticated user."

Authorization is always issued **per node**, never in bulk — there is no "authorize all target nodes" action, so there is no multi-node partial-failure case to reconcile here (each node's Authorize button has its own independent confirm/submit/outcome). The displayed status is correspondingly always per-node too — there is deliberately no single aggregate "Authorized" badge that could misrepresent a mixed (some-nodes-authorized) state as fully authorized.

**Target exactness is enforced server-side, not merely displayed client-side**: the Authorize payload sends the `dgraph_id`/`dgraph_revision` from the Console's own last-loaded slice, but `build_reconciliation_authorization` (`dserver/src/core/operational_dgraph.rs`) independently re-validates both against the LIVE operational D-Graph before issuing anything, rejecting (400) if either has gone stale since the slice was last loaded. A Console holding a stale `dgraph_revision` therefore cannot silently authorize the wrong target — it gets a clear, visible rejection naming the live revision instead, surfaced verbatim in the action notice.

Because node slices (and therefore authorization status) are refreshed on every batch-poll tick while a reconciliation is in flight (see Polling below), a long-open page does not indefinitely keep showing a since-expired authorization as current — though, as stated above, this display is never authoritative regardless of freshness; the real gate is always server/agent-side at attempt time.

## Reconcile (Part F)

"Reconcile runtime" creates a batch with `mode=execute`. Button enablement only requires accepted authority + at least one target node to exist (Part F1) — it is **never** gated on authorization being present, since authorization may still fail per-node server/agent-side regardless of what the button looked like. Node derivation is entirely server-side (`post_reconciliation_batch` resolves nodes from accepted placements) — this workspace never supplies or sends a node list of its own (Part F2).

The confirm dialog (Part 13) explicitly states all four required elements, not merely implies them: the exact target node list, that this is **command-side reconciliation, not a health or convergence confirmation**, the node-wide scope disclosure (other co-located applications' services may also be reconciled), and that **authorization is a separate, independent gate** this action does not itself satisfy.

**Request alone never authorizes mutation.** A claimed, freshly-validated `execute`-mode request still passes through every pre-existing, unmodified gate before any host is touched: the process-level `--execute` capability, `OperationalReconciliationAuthorization.allow_host_mutation`, Phase 138A's own per-boundary live-revision/ownership check, and artifact/binding checks. Nothing in this phase shortens or bypasses that chain.

### `client_request_id` and transport ambiguity (Parts F3/O)

One `crypto.randomUUID()` is generated per **submission attempt**, not per click. The same id is reused on a retry **only when the prior attempt's own outcome was ambiguous** — a transport-level failure (`timeout`/`disconnected` from `ReconciliationClient`) where the HTTP response itself was never received, so the true outcome is unknown and retrying is safe exactly because the server's own idempotent `client_request_id` handling means a duplicate POST resolves to the same rows, never a second batch. Any other outcome (success, or a definite rejection such as `reconciliation_queue_full`/`reconciliation_request_target_stale`) generates a fresh id for the next, genuinely new attempt. The action button is disabled for the duration of one in-flight submission.

## Request/batch/attempt tracking (Parts G/K)

The current batch section shows, per member: node id, request status (`Pending · waiting for agent`, `Claimed by agent`, `Attempted`, `Stale`, `Expired` — never flattened to one badge), mode, and linked attempt id. Once a member's `linked_attempt_id` appears, its full attempt record (`GET /api/v1/reconciliation-attempts/:id`) is fetched once and rendered: status, timestamps, `authorization_id`, `services_truncated`, and a per-service table (service id, result, host-mutation flag, verbatim execution status). Every enum this workspace renders maps 1:1 to the backend's own: `ReconciliationRequestStatus`, `ReconciliationBatchStatus`, `ReconciliationAttemptStatus`, `ReconciliationActionResult` — Console never reproduces the backend's own state-machine logic (`effective_status`/`derive_batch_status` stay server-side, Part S); it only maps an already-authoritative enum value to a label and a tone.

### Partial (Part G2)

A batch with some nodes Succeeded and one Failed is shown as **Partial** (the server-computed `ReconciliationBatchStatus::Partial`), with every per-node row visible so the failed node is obvious. There is no automatic rollback — none exists in the backend, and this workspace does not invent one.

### Stale (Part G3)

A batch/request whose target authority changed after creation is shown as **Stale** with an explicit, `role="alert"` banner. It is never silently retargeted — the only recovery is creating a new Preview/Reconcile request against the then-current target, exactly like Phase 137's own stale-proposal-basis banner.

### Agent offline / expiry (Parts J/R)

A request that stays Pending is shown as "Waiting for node agent: `<node_id>`", never immediately reported as a failure. A request whose TTL lapses is shown as **Expired**, with guidance to create a new request — an expired request is never reused or silently revived.

### Unknown (Part G1)

An attempt whose status is `Unknown` is rendered as "Unknown (command-side result could not be determined)" — never promoted to Succeeded or Failed.

## Node-wide scope, not application-scoped (Parts H/35/36)

This is a real, confirmed architectural property surfaced (not introduced) by this phase: `POST /api/v1/reconciliation-requests/batch` resolves nodes from `application_id`, but each resulting request targets that node's **whole** operational D-Graph revision — the agent's reconciliation cycle has always been node-scoped, never application-scoped (Phase 138A's own "Attempt scope" doc comment). A node hosting services from multiple applications will have **all** of them reconciled when its request is claimed and run, not only the one the operator named.

This workspace discloses it in three places, never only in documentation: (1) a permanent `role="note"` banner above the action buttons, (2) the Reconcile confirmation dialog's own text, and (3) the "Target nodes" section's per-service table, which tags any service whose `application_id` differs from the workspace's own with an explicit **Other application** badge — sourced directly from `OperationalDesiredService.application_id`, which the slice endpoint already carries. Preview makes this concrete rather than abstract: it lists every service the node reconciliation would actually consider, co-located applications included.

## Claim-fencing limitation (Part I)

Phase 138C's own independent review disclosed that `claim_token` is a lease, not a strong distributed fencing token: nothing re-verifies claim ownership immediately before a mutation boundary. In the real operating model (one `smartsentinel` process per node, held by an exclusive OS-level file lock), a second concurrent claimant can only arise from a crash-and-restart (sequential, not concurrent) or an operator misconfiguration binding two processes to one `node_id` — both pre-existing, out-of-scope trust assumptions, not something this phase's own UI can fix. The workspace's own copy never overclaims: "A reconciliation request may be retried safely against desired state" is the stated guarantee; "Executed exactly once" is explicitly never said anywhere in this workspace (proven by `tests/reconciliation.test.mjs`).

## Backward compatibility / no-regression proofs

- **No auto-deploy on accept** (Part AA): `proposal-review.ts` never imports `ReconciliationClient` or calls `createBatch` — proven by source-text scan.
- **No push execution** (Part AB): neither `pages/reconciliation.ts` nor `api/reconciliation.ts` references any host-execution primitive (`child_process`, `ssh2`, `docker run`, `exec(`, `spawn(`) — proven by source-text scan. This workspace only ever calls existing DServer read/write-control-state endpoints over `fetch`.
- **No authority mutation** (Part AC): this workspace never imports or calls `acceptProposal`/`createProposal` — proven by source-text scan. Only request/authorization control state can change here.
- **Structured API errors** (Parts AG/P): the reconciliation-request family of endpoints (`dserver/src/api/reconciliation_request.rs`) gained an additive `code` field alongside the existing free-text `error` message (e.g. `reconciliation_queue_full`, `reconciliation_request_stale`, `reconciliation_request_target_stale`, `reconciliation_request_not_claimable`, `unknown_node`, `unknown_application`, `incompatible_request_reuse`) specifically so this workspace never parses English prose to decide how to react to a 409. The pre-existing `operational-dgraph-slice`/`operational-reconciliation/authorize` endpoints were deliberately left unchanged (`{"error": "<message>"}`, no code) — narrower than a whole-framework redesign, matching Part AG's own "only if needed" instruction.
- **Read-time status honesty**: `dserver`'s `get_reconciliation_request`/`list_recent_reconciliation_requests`/`get_reconciliation_batch` now serve `effective_status` (a pure, read-time-only projection), not the possibly-stale persisted `status` column, so this workspace is never shown a falsely-still-"Pending" row for a request that has actually already lapsed.
- **Queue full** (Part Q): shown as "Node reconciliation queue is full. Wait for an existing request to resolve, then try again." — the workspace never retries automatically.

## Polling (Part N)

Bounded, sequential `setTimeout` chains (never `setInterval`, never an overlapping second poll for the same batch) — 3s while the tab is visible, 15s while hidden (`document.visibilityState`, read fresh on every tick, no separate event listener or scheduler). Polling stops once the batch reaches a terminal status (`succeeded`/`partial`/`failed`/`unknown`/`stale`). Each poll tick also refreshes every target node's slice (and therefore its authorization display), piggybacking on this already-bounded cadence rather than adding a second, independent polling loop.

Independent review (Audit 27) found that `sequence` alone (bumped only by page load/navigation) does not guard against a **same-page** resubmission: creating a new batch while the previous one's poll chain still has a request in flight could let the old batch's late response overwrite the new batch's state. `pollToken` (already bumped by `stopPolling`/`startPolling` on every new submission, not only on navigation) is the actual per-batch generation guard — `loadBatch`, `loadLinkedAttempts`, and `loadSlices` all check `myPollToken !== pollToken` immediately before writing any shared state, not only inside `pollOnce`'s own post-await check. `tests/reconciliation.test.mjs` proves this guard is present in all three functions.

## Browser refresh (Part Z #12)

The most recent `reconciliation_batch_id` for a given `(project_id, application_id)` is cached in `sessionStorage` (best-effort only, never authoritative — a quota failure or private-browsing restriction degrades to "no cached batch," never a crash). On load, if a cached id exists, the workspace re-fetches it; a 404 (pruned/unknown) clears the cache silently rather than showing a permanent error.

## Fixture scenarios (Part Y)

`console/tests/reconciliation-visual-fixture.mjs` (TEST ONLY, never the automated suite) serves ten deterministic scenarios, selected by `application_id`: `app-preview-waiting` (A), `app-preview-done` (B), `app-authorized-succeeded` (C), `app-partial-3node` (D, three nodes `fog-01`/`cloud-01`/`rpi-01`), `app-stale` (E), `app-queue-full` (F), `app-offline-expired` (G), `app-first-deploy` (H), `app-cross-app` (I, exercises the node-wide disclosure), `app-unknown-result` (J). Batch status progresses across repeated polls of the same `reconciliation_batch_id` (Pending → Claimed → Attempted), so a human reviewer can actually watch the progression a real agent poll cycle would produce, not just see a frozen terminal state. `reconciliation-visual-fixture.test.mjs` proves every one of these responses validates against the real Console validators (`reconciliation-model.ts`), not merely eyeballed.

### Manual browser acceptance steps (Part Z)

1. `node --experimental-strip-types console/tests/reconciliation-visual-fixture.mjs` (listens on `127.0.0.1:8138` by default; override with `DATUM_RECONCILIATION_FIXTURE_PORT`).
2. `DATUM_DEV_API_TARGET=http://127.0.0.1:8138 npm --prefix console run dev`.
3. Open `http://127.0.0.1:5173/projects/project-a/applications/app-authorized-succeeded/deploy` — confirm accepted authority, target node, and "Authorized for the current target" all render.
4. Click **Preview reconciliation** (enter an audit label first) — confirm a batch appears immediately, Pending.
5. Reload the page — confirm the same batch is recovered from `sessionStorage` and polling resumes (browser refresh retains state).
6. Click **Reconcile runtime** — confirm the confirmation dialog names the target node and the node-wide disclosure, then confirm the batch progresses Pending → Claimed → Attempted → Succeeded over a few seconds.
7. Navigate to `.../applications/app-partial-3node/deploy`, Reconcile — confirm the batch reaches **Partial** and `rpi-01`'s row is visibly distinct (Failed attempt), with no rollback offered.
8. Navigate to `.../applications/app-stale/deploy`, Reconcile — confirm the batch reaches **Stale** with its alert banner and no silent retarget.
9. Navigate to `.../applications/app-queue-full/deploy`, Reconcile — confirm the explicit "queue is full" message with no automatic retry.
10. Navigate to `.../applications/app-offline-expired/deploy` — confirm the request shows **Expired**, not a failure.
11. Navigate to `.../applications/app-cross-app/deploy`, Preview — confirm the "Other application" badge appears on `service-other-app` in the target-nodes table.
12. Resize the browser to ~400px width — confirm every table scrolls within its own `table-responsive` wrapper, never the whole page horizontally.

## Non-goals / Phase 139 boundary (Part AD)

Not built, not started by this phase: container DMonitor producer work, runtime health interpretation, freshness-based convergence, automatic recovery, a self-healing policy loop, or any GNN/LLM autonomy. Nothing in this workspace reads DMonitor state (proven by source-text scan); a future Phase 139 contextual-observation panel, if ever added, must live in a visually separate area labeled "Observation," never merged into this workspace's own command-side status badges.

## Validation

Frontend: 44 new tests (17 model validator/display tests, 14 route/structural-regression tests, 13 visual-fixture contract tests) plus all 230 pre-existing tests, 274/274 passing; `tsc --noEmit` clean; `vite build` clean. Backend (Phase 138C foundation touched for the additive `code` field and read-time `effective_status` projection): full `dserver` suite re-run, 2427/2427 passing; `cargo build` clean for both `dserver` and `DATUM`. `git diff --check` clean.

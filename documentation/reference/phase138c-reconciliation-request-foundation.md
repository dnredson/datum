# Phase 138C — Reconciliation Request Foundation

## Purpose

Phase 138B's control-plane design audit concluded that a Console "Reconcile"/"Execute" button needs a narrow trigger primitive before Phase 138 can build on it: a durable, atomically-claimable `ReconciliationRequest` that the node-resident DATUM agent discovers through its existing poll and claims before running. Phase 138C implements exactly that primitive — creation, idempotency, atomic claim, a control-slice signal, agent-side claim+run wiring, request→attempt linkage, and a multi-node batch foundation — and nothing past it. **No Console UI was built. No push execution was added to `dserver`. Phase 139 was not started.**

## Pull-based architecture (unchanged, restated)

`dserver` still never pushes anything to a node. Creating or claiming a `ReconciliationRequestV1` only changes rows in `dserver`'s own SQLite storage; it does not contact the agent, open a connection to the node, or run anything. The agent remains the sole thing that ever makes an outbound call to perform host mutation, and it only learns of a claimable request by observing the `pending_request` field on the control slice it was already polling every cycle (`GET /api/v1/nodes/:node_id/operational-dgraph-slice`) — no new steady-state poll was added. A structural regression test (`api::reconciliation_request::tests::phase138c_reconciliation_request_foundation_never_makes_an_outbound_call_to_a_node`) scans all three new files' implementation code (comments excluded) and asserts none of them reference `reqwest`, `Client::new`, `Client::builder`, `ssh2`, or `std::process::Command`.

## Six objects, never conflated

1. **Accepted authority** (`core::ddeploy`) — what the operational D-Graph projection is derived from.
2. **ReconciliationRequest** (this phase) — a durable "please reconcile node N against revision R" trigger with claim/lifecycle state.
3. **OperationalReconciliationAuthorization** — the pre-existing permission gate; claiming a request does **not** check or grant this. It remains an entirely separate gate the agent's own Phase 138A mutation path still enforces at attempt time.
4. **ReconciliationAttempt** (Phase 138A) — the command-side record of what the agent actually reported attempting.
5. **OperationalEvidenceEvent** — the event stream both attempts and (now) requests are folded from.
6. **DMonitor** — observation authority; untouched by any of the above.

## Schema (`dserver/src/core/reconciliation_request.rs`)

`ReconciliationRequestV1` is scoped to `project_id` + `node_id` + `target_dgraph_id`/`target_dgraph_revision` — deliberately **no** singular `application_id` and **no** singular `accepted_dmap_ref`, mirroring the hard-won lesson from Phase 138A's own review: a node's operational D-Graph can span multiple applications, so a singular application-scoped field would misrepresent it. `mode` is exactly `"preview"` or `"execute"` (`validate_mode`), nothing else. `client_request_id` is a caller-supplied idempotency key, distinct from `reconciliation_request_id`/`reconciliation_attempt_id`/`authorization_id`, enforced unique per-project at the storage layer.

`ReconciliationRequestStatus`: `Pending | Claimed | Attempted | Stale | Expired`. `effective_status()` is a pure, lazily-evaluated function (no background sweeper, mirroring `LeaseGuard`'s own established precedent): a stored `Pending` row becomes `Expired` once its own TTL (`DEFAULT_REQUEST_TTL_SECONDS` = 900s) lapses; a stored `Claimed` row reverts to `Pending` once its claim lease (`DEFAULT_CLAIM_LEASE_SECONDS` = 120s) lapses, **unless** the request's own TTL has also lapsed, in which case `Expired` takes precedence; `Attempted`/`Stale`/`Expired` are terminal and never change.

`ReconciliationBatchStatus` (`Pending | Executing | Succeeded | Partial | Failed | Unknown | Stale`) is derived by `derive_batch_status` from a batch's member requests plus their linked attempts' statuses: any member still in-flight (`Pending`/`Claimed`) forces `Executing`; any member `Stale` forces `Stale` (a batch can never claim success while one member's target went stale); otherwise the status is derived analogously to `reconciliation_attempt`'s own per-service aggregation.

## Dedicated SQL storage, not the generic domain-JSON store

`dserver/src/storage/reconciliation_request.rs` is a dedicated SQLite table (`reconciliation_requests`), deliberately **not** `storage::control_state::{load_domain_json,persist_domain_json}`. That generic store is an unconditional `INSERT ... ON CONFLICT DO UPDATE` — last-write-wins, no compare-and-swap — which Phase 138B's own design audit judged unsafe for claim-racing. This module mirrors `storage::operational_evidence`'s own dedicated-table precedent instead, giving genuine SQL row-level atomicity to the one operation that actually needs it.

The claim itself is one atomic `UPDATE ... WHERE` statement:

```sql
UPDATE reconciliation_requests
SET status = 'claimed', claim_token = ?1, claimed_at_utc = ?2, claim_expires_at_utc = ?3, updated_at_utc = ?2
WHERE project_id = ?4 AND node_id = ?5 AND reconciliation_request_id = ?6
  AND expires_at_utc > ?7
  AND (status = 'pending' OR (status = 'claimed' AND claim_expires_at_utc IS NOT NULL AND claim_expires_at_utc <= ?7))
```

This single statement handles both a fresh claim and a reclaim-after-abandoned-lease (crash recovery) in one shot: exactly one of two concurrent claimants ever succeeds, proven by `two_concurrent_claimants_exactly_one_wins`, and an abandoned claim past its lease can be reclaimed by a different token, proven by `abandoned_claim_past_its_lease_can_be_reclaimed_by_a_different_token`. `create_request` is idempotent on `(project_id, client_request_id)` — a retried creation with the same key returns the original row untouched, never a duplicate. `link_attempt` is idempotent and only affects `claimed`/`attempted` rows; `mark_stale` only affects non-terminal rows.

## Request creation and target-freshness validation (`dserver/src/api/reconciliation_request.rs`)

`POST /api/v1/reconciliation-requests` validates the target against **live** operational authority (`operational_dgraph_slice_snapshot`) before persisting — `dserver` already has this data locally, so no node round trip is needed to reject a request pointed at an already-stale target. A `client_request_id` reused with a different node/target/mode is rejected as a 409 conflict ("incompatible client_request_id reuse"), never silently accepted against the original row's values.

`POST /api/v1/reconciliation-requests/batch` resolves every node with an accepted placement for a given `application_id` **server-side**, from the live operational D-Graph snapshot — the browser never supplies or derives the node list. Each member request's own idempotency key is derived as `"{client_request_id}:{node_id}"`, so retrying the same batch POST (e.g. after a lost response) re-resolves to the same member requests rather than creating duplicates. Bounded by `MAX_NODES_PER_BATCH` (50).

`POST /api/v1/nodes/:node_id/reconciliation-requests/:request_id/claim` re-validates the target is still live-current **before** attempting the claim: if the live operational D-Graph has moved on, the request is marked `stale` and the claim is refused with a 409, never silently claimed against a superseded target. A request addressed via the wrong node is masked as 404, never leaked as "exists but wrong node" — the same cross-project/cross-node masking convention established in Phase 137/138A.

## Three-layer stale protection (preserved, not collapsed)

1. Request **creation** pins the target against live authority at that moment.
2. **Claim** re-checks the target is still current before granting the claim.
3. Phase 138A's own per-boundary live-mutation guard (`check_boundary`/`LiveRevisionPin`) checks immediately before every individual host mutation, regardless of what request or claim preceded it.

Each layer is independent and the third is the ultimate authoritative backstop: even a successfully claimed, freshly-validated request still cannot bypass Phase 138A's own freshness guard at the moment of mutation.

## Control-slice signal (`core`/`api::operational_dgraph`)

`OperationalDGraphSlice` gained one additive field, `pending_request: Option<PendingRequestSignal>` (`{ reconciliation_request_id, mode, target_dgraph_id, target_dgraph_revision }`), populated by `resolve_pending_request` in the API layer: it loads claimable (`pending`/`claimed`) requests for that node, applies `effective_status` to each, keeps only the ones that resolve to `Pending` right now, and hands the oldest (`select_next_pending`, created-at ascending) to the caller. This reuses the **existing** control-slice poll the agent already performs every cycle — no new endpoint, no new poll.

## Agent-side claim and run (`DATUM/src/agent/operational_service_loop.rs`)

`OperationalControlSliceHead` gained the mirrored `pending_request: Option<OperationalPendingRequestHead>` field. Each cycle, after fetching the control slice head and before the existing event-driven/drift-driven skip logic, `try_claim_pending_request` runs:

1. No signal → `None`, ordinary cycle proceeds unchanged.
2. An unrecognized `mode` → rejected client-side, **no claim attempt is ever made** (proven by `pending_request_with_unknown_mode_is_rejected_without_claim_attempt`).
3. A target that doesn't match this same poll's own control-head identity → rejected client-side as a defensive inconsistency check, again with no claim attempt (`pending_request_with_target_mismatch_is_rejected_without_claim_attempt`).
4. Otherwise, `POST /api/v1/nodes/:node_id/reconciliation-requests/:request_id/claim` is attempted. Any non-success response (already claimed elsewhere, stale, terminal, expired) is treated as "nothing claimed this cycle," never as an error that aborts the cycle — this is an ordinary, expected outcome of claim racing.

A successful claim forces the surrounding cycle to run even when control is unchanged and drift verification isn't yet due (`!control_changed && !drift_due && claimed_request.is_none()` is the updated skip guard) — claiming the request is itself the trigger, recorded as a new `trigger = "reconciliation_request_claimed"`.

**Preview mode forces zero mutation for that one cycle regardless of the process-level `--execute` capability**: `execute_requested` is computed as `false` whenever the claimed request's mode is `"preview"`, and otherwise falls through unchanged to `options.execute`. A claimed `"execute"`-mode request does **not** itself grant execution — the process-level `--execute` flag remains the sole thing that can ever authorize a mutating cycle (Part F1 backward compatibility: manual dry-run and manual `--execute` invocations behave exactly as before this phase, since a process with no pending request ever claims nothing and `claimed_request` stays `None`).

## Request→attempt linkage (Part G)

`OperationalEvidenceEvent` (both the agent's and `dserver`'s copies) gained an additive `reconciliation_request_id: Option<String>` field, set via a new `with_reconciliation_request_id` builder mirroring `with_reconciliation_attempt_id` exactly. The agent threads the claimed request's id into the terminal reconciliation evidence event for that cycle (`write_reconciliation_result` → `record_reconciliation_event` → `record_event_inner`).

On `dserver`'s side, `fold_reconciliation_attempt` (`api/operational_evidence.rs`) now passes `event.reconciliation_request_id.as_deref()` into `fold_evidence_into_attempt` (which stores it on `ReconciliationAttemptRecordV1.reconciliation_request_id`, additive, `None` for a manual `--execute` cycle with no claimed request behind it), and — when the incoming evidence carries **both** a `reconciliation_request_id` and a `reconciliation_attempt_id` — verifies that the stored request names the SAME `node_id`/`target_dgraph_id` the evidence event itself reports, and only then calls `storage::reconciliation_request::link_attempt`, which atomically transitions that request to `Attempted` and records `linked_attempt_id`. A mismatch (or an unknown `reconciliation_request_id`) silently skips linking rather than linking to the wrong request or erroring the fold — this is a correctness guard against accidental misattribution (a stale id, a bug, two co-resident agents), not a defense against a fully adversarial agent; `event.node_id`/`event.dgraph_id` are self-reported with no independent re-verification, same trust model as `reconciliation_attempt_id` itself (ADR-0013 §6). This linkage is never inferred from time/proximity, only from the explicit ids the evidence itself carries, and a linking failure never rolls back the attempt fold that already happened (the attempt record remains the source of truth for execution history; the request's status is a secondary, best-effort view).

## Batch status aggregation

`GET /api/v1/reconciliation-requests/batch/:batch_id` loads every member request plus (for members that have one) their linked attempt's status, and returns the server-computed `derive_batch_status` result — Console must never re-derive this itself from the raw member list.

## Backward compatibility (Part T, verified)

- A manual dry-run cycle (no `--execute`, no pending request) behaves byte-identically to before this phase: `claimed_request` is always `None`, `execute_requested` falls through to `options.execute` unchanged, and no new evidence field is populated.
- A manual `--execute` cycle with no claimed request behind it: same as above, `reconciliation_request_id` stays `None` throughout, serialized JSON omits the field (`skip_serializing_if`).
- Phase 138A's own attempt records, evidence events, and every existing consumer of `OperationalDGraphSlice`/`OperationalEvidenceEvent` compile and behave unchanged — every new field is additive with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Phase 137's accepted-authority/acceptance semantics are untouched; `core::ddeploy` has no new code dependency from this phase (same structural-regression discipline as Phase 138A's own `source_has_no_code_dependency_on_ddeploy_dmonitor_or_legacy_deployment_lineage`).

## Known, disclosed limitations (carried over, not solved here)

- **Day-zero / installation tooling**: still entirely out of scope, unchanged from Phase 138A's own Part E.
- **Security model**: claiming a request requires no additional authentication beyond whatever already protects `dserver`'s HTTP API — `requested_by` remains an audit label only, never authentication (ADR-0013 §6, unchanged). This foundation assumes a trusted control plane, same as every other phase to date.
- **No Console UI**: nothing in this phase renders a "Reconcile" button or batch progress view; that is explicitly Phase 138's own scope.
- **No push execution**: `dserver` still cannot and does not initiate anything on a node; it only persists request/claim state for the agent to discover.
- **Claim is a lease, not a fencing token**: `claim_token` is returned to the agent and persisted, but nothing in this phase requires the agent to present it again before mutating, and no mutation boundary re-verifies that the process acting is still the current claim holder. In the normal operating model (exactly one `smartsentinel` process per node, enforced by `acquire_operational_lock`'s own exclusive OS-level file lock), a second concurrent claimant for the same node can only arise from a crash-and-restart (sequential, not concurrent) or an operator misconfiguration binding two processes to the same `node_id` (an already-existing, out-of-scope day-zero trust assumption — see Phase 138A's own Part E). Should such a dual-process scenario occur, Phase 138A's own per-boundary authorization/live-revision/ownership-label checks still apply independently to each process and are not bypassed by holding a (possibly stale) claim — bounding, but not eliminating, the blast radius. A stronger fix (threading claim ownership into `MutationBoundaryContext` itself, re-verified per boundary) was considered and deliberately deferred: it touches Phase 138A's own core guard and is a larger design change than this foundation's narrow scope, not a one-line hardening.
- **Node-wide, not application-scoped, reconciliation**: `POST /api/v1/reconciliation-requests/batch` resolves nodes from `application_id`, but each resulting request is scoped to that node's WHOLE operational D-Graph revision — the agent's reconciliation cycle has always been node-scoped, never application-scoped (Phase 138A's own "Attempt scope" doc comment). Triggering a batch reconcile for application A can therefore cause the agent to also reconcile (and potentially mutate) OTHER applications co-located on the same node(s). This is not a regression this phase introduced; it is the pre-existing node-scoped reconciliation architecture, now reachable through an application-named trigger. See the explicit doc comment on `post_reconciliation_batch` in `api/reconciliation_request.rs`. Any future Console surface built on this endpoint must present this truthfully (node-wide, not application-isolated) rather than imply per-application isolation that does not exist.

## Non-goals (explicit)

Phase 138 Console implementation, Phase 139, shared/external/system runtime reconciliation, container DMonitor producer work, and any change to accepted placement authority semantics were all explicitly out of scope for this phase and were not touched.

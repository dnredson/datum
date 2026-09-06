# SmartSentinel O2 checkpoint — passive observation trigger

Date: 2026-09-06
Branch: `agent/o2-passive-observation-trigger`
Baseline main: `0478a3bbb0a78ae135d3b737faa298402f16ca70` (step 65 LIVE PASS)

## Progress status

- 59/100 — LIVE PASS.
- 60/100 — LIVE PASS.
- 61/100 — LIVE PASS.
- 62/100 — LIVE PASS.
- 63/100 — LIVE PASS.
- 64/100 — LIVE PASS.
- 65/100 — LIVE PASS.
- 66/100 — LIVE PASS.

## Step 66 — automatic passive assessment on observation ingress

Step 66 converts the step-63/64 passive analysis from a manually invoked context query into an automatic event reaction for the existing dserver snapshot ingress path.

When `POST /api/v1/snapshots` accepts a SmartSentinel snapshot, dserver still performs the existing Firebase writes first:

1. raw snapshot;
2. latest snapshot;
3. node upsert.

After those writes succeed, dserver automatically invokes the same passive analysis acquisition/correlation core used by the manual passive endpoint and GraphQL assessment.

No second client call to `/api/v1/analysis-context/passive/:node_id` is required.

## Fail-soft ownership boundary

Snapshot persistence owns the ingest outcome.

If passive assessment cannot run (for example, no accepted Operational D-Graph exists yet), the telemetry ingest remains accepted. The response reports `passive_analysis.analysis_status=not_assessed` plus the analysis error rather than rolling back or rejecting the already accepted observation.

This preserves the architecture boundary:

- SmartSentinel observation is evidence and must not depend on placement/governance readiness;
- dserver assessment is derived context;
- absence of desired state must not erase or reject observed state.

## Passive trigger result

The trigger core records:

- trigger source;
- project/node;
- incoming cycle id and observation timestamp;
- processed timestamp;
- latest Firebase cycle id/timestamp used for assessment;
- whether the triggering observation timestamp matches the latest snapshot actually assessed;
- assessment status;
- optional full assessment;
- analysis error when not assessed;
- `side_effects_performed=false`.

For the `/api/v1/snapshots` response, dserver returns a compact `passive_analysis` summary including:

- `trigger_source=dserver_snapshot_ingest`;
- `analysis_status`;
- latest cycle/timestamp;
- notification/latest match flag;
- telemetry freshness;
- service comparisons;
- analysis error;
- `side_effects_performed=false`.

## Race semantics

The trigger compares the incoming `observed_at_utc` with the latest Firebase observation used by the assessment using RFC3339 instant equality.

This anticipates asynchronous provider notifications: if a notification for observation N is processed after N+1 has already become latest, dserver may safely assess N+1 and expose `notification_matches_latest=false` rather than pretending it assessed the exact notified snapshot.

## File-output versus HTTP envelope

Live validation confirmed an important existing contract distinction:

- `datum once --out <file>` writes a raw `Snapshot` object;
- the HTTP snapshot ingress expects the `SnapshotEnvelope` form `{ "protocol_version": "s0.v1", "snapshot": ... }` used by the SmartSentinel HTTP output implementation.

An initial POST of the raw file correctly returned HTTP 422. Wrapping the already generated real snapshot in the normal HTTP envelope produced the expected accepted ingest. No product-code change was required for this test-fixture correction.

## Live validation — PASS

Environment:

- project: `monitor-f49c8`;
- node: `fog-01`;
- active D-Graph: `dgraph:o2-mqtt-role-1`, revision 4;
- freshness policy: 120 seconds;
- desired service: MQTT container `running`.

Fresh real SmartSentinel/DATUM observation:

- cycle id: `e5687d04-c3d8-499d-82e0-0f16bb5781b9`;
- observed at: `2026-09-06T20:49:26.721243737Z`;
- entities: 9;
- edges: 7;
- events: 1;
- actions: 0.

The single `POST /api/v1/snapshots` response proved automatic passive triggering without a manual passive-analysis request:

- `accepted=true`;
- `trigger_source=dserver_snapshot_ingest`;
- `analysis_status=assessed`;
- `latest_cycle_id=e5687d04-c3d8-499d-82e0-0f16bb5781b9`;
- `latest_observed_at_utc=2026-09-06T20:49:26.721243737Z`;
- `notification_matches_latest=true`;
- `telemetry_freshness=fresh`;
- MQTT desired `running`;
- concrete observed runtime `running`;
- observed binding `true`;
- `comparison_status=converged`;
- `reason=observed_runtime_running`;
- `side_effects_performed=false`.

Exact-cycle proof returned `STEP66_EXACT_CYCLE_PASS` with incoming and assessed cycle both equal to `e5687d04-c3d8-499d-82e0-0f16bb5781b9`.

GraphQL immediately read the same latest Firebase observation and returned:

- latest cycle `e5687d04-c3d8-499d-82e0-0f16bb5781b9`;
- `telemetryFreshness=fresh`;
- `telemetryAgeSeconds=9`;
- MQTT `comparisonStatus=converged`;
- `reason=observed_runtime_running`;
- concrete runtime target preserved;
- `sideEffectsPerformed=false`.

Clock sanity check showed no material skew:

- desktop UTC: `2026-09-06T20:49:50.5024208Z`;
- fog UTC: `2026-09-06T20:49:47.716684332Z`.

The approximately three-second difference is far below the 120-second freshness window and does not affect the validation result.

## Firebase-direct path

Step 66 does not pretend Firestore itself provides an HTTP webhook to dserver.

It establishes the provider-neutral trigger core and proves the automatic dserver-ingress path. A following step can connect Firestore/Eventarc/Function (or an equivalent notification delivery mechanism) to the same trigger semantics for the architecture path where SmartSentinel writes Firebase directly while dserver is disconnected or not in the data path.

No polling is introduced in step 66.

## Closure statement

Step 66 is LIVE PASS.

PASS proves new observation ingress can automatically produce passive desired-vs-observed assessment while preserving observation ownership, exact-cycle traceability, explicit freshness semantics, GraphQL consistency, fail-soft ingest behavior, and zero reconciliation/placement side effects.

## Architecture statement

`SmartSentinel observes -> observation is persisted in Firebase -> dserver reacts passively when the observation enters its ingress path -> the same governed desired-vs-observed/freshness core assesses the affected node -> GraphQL may expose the result. External AI/GNN remains responsible for placement/reasoning; dserver does not mutate the D-Graph from this trigger; SmartSentinel/WASM reconciles only under explicit authorization; DATUM remains conceptual.`

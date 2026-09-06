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
- 66/100 — IMPLEMENTED, pending compile/live validation.

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

## Firebase-direct path

Step 66 does not pretend Firestore itself provides an HTTP webhook to dserver.

It establishes the provider-neutral trigger core and proves the automatic dserver-ingress path. A following step will connect Firestore/Eventarc/Function (or equivalent notification delivery) to the same trigger semantics for the architecture path where SmartSentinel writes Firebase directly while dserver is disconnected or not in the data path.

No polling is introduced in step 66.

## Planned live proof

1. update to `agent/o2-passive-observation-trigger`;
2. run the passive-trigger/context unit tests and build with locked dependencies;
3. start dserver with the existing SQLite/Firebase configuration and 120-second freshness policy;
4. on `fog-01`, run the real DATUM/SmartSentinel `once` collector with `DATUM/config.linux`, writing a fresh snapshot to `/tmp/step66-snapshot.json` while HTTP output remains disabled;
5. POST that one fresh snapshot to `/api/v1/snapshots` with `X-Datum-Project: monitor-f49c8`;
6. verify the same ingest response contains `passive_analysis.trigger_source=dserver_snapshot_ingest` and `analysis_status=assessed` without calling the manual passive-analysis endpoint;
7. verify latest cycle/timestamp match the incoming fresh snapshot and `notification_matches_latest=true`;
8. verify telemetry freshness is `fresh` under the 120-second policy;
9. because `datum-mqtt` is currently running and bound, verify MQTT desired `running` correlates to concrete runtime `running` and returns `converged` with fresh evidence;
10. verify `side_effects_performed=false`;
11. query GraphQL `node(...) { assessment { ... } }` and verify it sees the same latest fresh snapshot/correlation.

PASS proves new observation ingress can automatically produce passive desired-vs-observed assessment while preserving observation ownership, fail-soft ingestion, and zero reconciliation/placement side effects.

## Architecture statement

`SmartSentinel observes -> observation is persisted in Firebase -> dserver reacts passively when the observation enters its ingress path -> the same governed desired-vs-observed/freshness core assesses the affected node -> GraphQL may expose the result. External AI/GNN remains responsible for placement/reasoning; dserver does not mutate the D-Graph from this trigger; SmartSentinel/WASM reconciles only under explicit authorization; DATUM remains conceptual.`

# SmartSentinel O2 checkpoint — active AI context export

Date: 2026-09-07
Branch: `agent/o2-active-ai-context`
Baseline main: `00f7da755bceee3e336735b6c18a8ede7e01dd51` (step 66 closed)

## Progress status

- 59/100 — LIVE PASS.
- 60/100 — LIVE PASS.
- 61/100 — LIVE PASS.
- 62/100 — LIVE PASS.
- 63/100 — LIVE PASS.
- 64/100 — LIVE PASS.
- 65/100 — LIVE PASS.
- 66/100 — LIVE PASS.
- 67/100 — LIVE PASS; full dserver suite output still to be re-presented after integration.

## Step 67 — lossless active export for external AI/GNN

Step 67 extends the existing active analysis acquisition with an explicit read-only AI export selected through:

`POST /api/v1/analysis-context/active?format=ai`

The default endpoint behavior is preserved. Requests without `format=ai` continue returning the established OperationalAnalysisContext schema `0.2.0`.

The AI export is intentionally an evidence package rather than a model-specific feature tensor. It preserves:

- the complete accepted Operational D-Graph revision used for acquisition;
- every desired node and desired service from that revision;
- the complete active desired-vs-observed assessment;
- the latest raw Firebase document associated with each desired node inside the assessment;
- the complete decoded SnapshotEnvelope for each valid node snapshot, including entities, edges, events, actions, protocol/schema metadata and unknown extension fields;
- telemetry freshness, age, cycle identity and observation timestamp;
- explicit missing, stale and invalid evidence states;
- a deterministic evidence summary;
- `side_effects_performed=false`.

No model is executed, no placement decision is made, no D-Graph revision is written, no reconciliation authorization is created and no host action is executed by this export.

## Consistency model

The export declares:

`single_accepted_dgraph_revision_non_atomic_latest_per_node`

One accepted Operational D-Graph revision is read first and retained for the whole acquisition. Every desired node in that revision is then acquired concurrently from Firebase. Network reads are not a distributed atomic snapshot, so each node may reflect a different latest observation instant. This fact is represented rather than hidden.

The final assessment and exported snapshot list must match the same graph identity and node cardinality. Unexpected or duplicate assessed nodes are rejected by the export builder.

## Evidence validity and inference usability

A node snapshot is considered usable for inference only when:

1. a Firebase document is present;
2. its `payload` decodes as a valid SnapshotEnvelope;
3. project/node/cycle identity is internally consistent when those Firestore fields are present;
4. Firestore and payload observation timestamps denote the same RFC3339 instant when both are present;
5. the assessment timestamp matches the decoded snapshot timestamp;
6. telemetry freshness is `fresh` under the configured TTL.

Malformed or identity-mismatched evidence is exposed as `snapshot_status=invalid`, is not exported as a trusted decoded snapshot, and cannot retain a convergence claim. Its service comparisons are downgraded to `comparison_status=unknown`, `reason=invalid_telemetry`.

Completeness means fresh, valid evidence exists for every desired node. It does not mean every desired service is converged or healthy.

## Unit/build validation recorded before closure

The five unit tests in `active_ai_context` passed. They cover:

- complete multi-node graph/snapshot preservation and JSON round-trip;
- stale and missing nodes remaining explicit and unusable;
- invalid payload identity being unable to claim convergence;
- assessment/payload timestamp mismatch becoming invalid evidence;
- graph/assessment identity mismatch being rejected.

The dserver build passed with the five pre-existing warnings already observed in this phase.

The complete dserver test-suite output was not presented in the laboratory transcript before this checkpoint was finalized. Step 67 therefore does not fabricate a full-suite PASS; the suite is requested again after integration below.

## Live validation — PASS

Laboratory:

- project: `monitor-f49c8`;
- dserver: desktop Luffy, `192.168.10.101:8080`;
- active D-Graph: `dgraph:o2-mqtt-role-1`, revision 4;
- active graph at validation time: one desired node, `fog-01`;
- desired service: MQTT container `running`;
- freshness TTL: 120 seconds.

Fresh real SmartSentinel/DATUM observation used by the export:

- cycle id: `60ccd899-5a59-4e1c-8ffb-290dc1f37dbf`;
- entities: 8;
- edges: 5;
- events: 2;
- actions: 0.

The `format=ai` response proved:

- the accepted D-Graph was preserved rather than reconstructed from telemetry;
- the raw Firebase evidence and decoded SnapshotEnvelope were retained;
- telemetry was classified `fresh`;
- `evidence_summary.complete=true`;
- `evidence_summary.usable_node_count=1`;
- the exported cycle id exactly matched `60ccd899-5a59-4e1c-8ffb-290dc1f37dbf`;
- exact-cycle validation returned PASS;
- `side_effects_performed=false`.

## Real MQTT drift observed during the proof

The live export correctly exposed a real desired-vs-observed divergence for MQTT:

- desired state: `running`;
- observed service status: `unavailable`;
- observed binding: `false`;
- assessment: drift.

This is a valid laboratory observation and not an exporter failure. Step 67 is specifically required to preserve drift instead of normalizing it into a healthy state.

No automatic reconciliation was triggered from this result.

## Backward compatibility

The same endpoint without `format=ai` continued returning OperationalAnalysisContext schema `0.2.0`.

The AI export therefore adds an explicit representation without changing the established analysis response for existing callers.

## Deferred relay branch

The experimental Firestore/Eventarc relay remains deferred on:

`agent/o2-firestore-eventarc-passive-trigger`

It is not part of step 67 and must not be merged as part of this closure.

## Closure statement

Step 67 is LIVE PASS for the active AI export contract.

The implementation proves that dserver can actively acquire evidence for the accepted graph revision and expose a lossless, freshness-aware, side-effect-free package suitable as input to external AI/GNN processing, while preserving raw evidence and desired-vs-observed drift.

The post-integration laboratory checklist intentionally repeats the complete dserver suite because its complete output was not recorded before closure.

## Architecture statement

`DATUM remains the concept -> dserver governs and exposes the accepted Operational D-Graph -> SmartSentinel supplies observed evidence and performs only explicitly authorized local reconciliation -> dserver may assemble read-only active evidence -> external AI/GNN may consume that evidence and propose changes -> proposals never execute directly and never mutate the live D-Graph without the governed acceptance path.`

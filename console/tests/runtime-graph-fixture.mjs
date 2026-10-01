import { graphFixture, unevenGraphFixture } from "./graph-fixture.mjs";
// Explicit manual/test-only fixture; never imported by production entry points.
export function runtimeVisualReviewFixture() {
  const f = runtimeFixture(unevenGraphFixture());
  const unmatched = { ...f.runtime.value.evidence[0],
    service_id: "unmatched-service", logical_node_id: null,
    observation_id: "manual-review-unmatched-observation" };
  unmatched.id = JSON.stringify(["runtime_evidence", unmatched.project_id,
    unmatched.application_id, unmatched.service_id, unmatched.node_id,
    unmatched.reporting_node_id, unmatched.monitor_instance_id]);
  f.runtime.value.evidence.push(unmatched);
  f.temporal_quality.push({ ...f.temporal_quality[0], observation_id: unmatched.observation_id });
  f.runtime.value.included_evidence_count = f.runtime.value.evidence.length;
  f.runtime.value.total_evidence_count = f.runtime.value.evidence.length;
  return f;
}
export function runtimeFixture(logical = graphFixture(), populated = true) {
  const evidence = populated
    ? logical.graph.value.nodes
        .filter((n) => n.kind === "dserv")
        .flatMap((n) =>
          [0, 1].map((i) => ({
            id: JSON.stringify([
              "runtime_evidence",
              logical.project_id,
              n.application_id,
              n.canonical_id,
              `fog-${i}`,
              `fog-${i}`,
              `epoch-${i}`,
            ]),
            project_id: logical.project_id,
            application_id: n.application_id,
            service_id: n.canonical_id,
            node_id: `fog-${i}`,
            reporting_node_id: `fog-${i}`,
            monitor_instance_id: `epoch-${i}`,
            observation_id: `observation-${n.id}-${i}`,
            observation_digest: "sha256:" + "b".repeat(64),
            collector: "test-only",
            observed_at_utc: "2026-09-30T14:50:00Z",
            received_at_utc: "2026-09-30T15:00:00Z",
            observed_health_claim: "unknown",
            dmap_ref: null,
            logical_node_id: n.id,
          })),
        )
    : [];
  return {
    project_id: logical.project_id,
    logical,
    runtime_source_kind: "admitted_dserv_realization_evidence",
    runtime: {
      availability: "available",
      value: {
        completeness: "complete",
        total_evidence_count: evidence.length,
        included_evidence_count: evidence.length,
        evidence,
      },
    },
    temporal_quality: evidence.map((e) => ({
      project_id: e.project_id,
      application_id: e.application_id,
      node_id: e.reporting_node_id,
      observation_id: e.observation_id,
      source_observed_at_utc: e.observed_at_utc,
      server_received_at_utc: e.received_at_utc,
      communication_freshness: "fresh",
      observation_freshness: "stale",
      source_time_quality: "behind_or_delayed",
    })),
  };
}

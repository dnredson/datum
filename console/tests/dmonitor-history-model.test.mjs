import test from "node:test";
import assert from "node:assert/strict";
import {
  validDMonitorHistoryQueryResponse,
  HEALTH_LABEL,
  FRESHNESS_LABEL,
  subjectDisplayIdentity,
  subjectKindLabel,
} from "../src/pages/dmonitor-history-model.ts";
import { historicalObservationFixture, historyQueryResponseFixture } from "./dmonitor-history-fixture.mjs";

test("valid history response fixture round-trips", () => {
  assert.ok(validDMonitorHistoryQueryResponse(historyQueryResponseFixture(), "project-a"));
});

test("validator rejects a project scope mismatch", () => {
  assert.ok(!validDMonitorHistoryQueryResponse(historyQueryResponseFixture(), "project-b"));
});

test("validator accepts a +00:00-offset timestamp exactly as the real backend formatter produces (Phase 140A finding)", () => {
  const withOffset = historyQueryResponseFixture({ queried_at_utc: "2026-10-05T12:00:05.123456+00:00" });
  assert.ok(validDMonitorHistoryQueryResponse(withOffset));
});

test("validator rejects a subject with an unrecognized kind -- closed set, never a permissive catch-all", () => {
  const bad = historyQueryResponseFixture({
    observations: [historicalObservationFixture({ subject: { kind: "made_up_kind", node_id: "fog-01" } })],
  });
  assert.ok(!validDMonitorHistoryQueryResponse(bad));
});

test("validator rejects an invented health/freshness enum value", () => {
  const badHealth = historyQueryResponseFixture({ observations: [historicalObservationFixture({ health: "thriving" })] });
  assert.ok(!validDMonitorHistoryQueryResponse(badHealth));
  const badFreshness = historyQueryResponseFixture({ observations: [historicalObservationFixture({ freshness: "ancient" })] });
  assert.ok(!validDMonitorHistoryQueryResponse(badFreshness));
});

test("every health/freshness enum value this module validates has a corresponding display label", () => {
  for (const status of ["unknown", "healthy", "degraded", "unhealthy"]) assert.ok(HEALTH_LABEL[status]);
  for (const status of ["fresh", "stale", "clock_skew"]) assert.ok(FRESHNESS_LABEL[status]);
});

test("subjectDisplayIdentity extracts application/service identity for dserv_realization and null for dnode", () => {
  const dserv = historicalObservationFixture().subject;
  assert.deepEqual(subjectDisplayIdentity(dserv), { applicationId: "app-1", serviceId: "service-a" });
  assert.deepEqual(subjectDisplayIdentity({ kind: "dnode", node_id: "fog-01" }), { applicationId: null, serviceId: null });
});

test("subjectKindLabel covers every one of the 8 canonical subject kinds", () => {
  const kinds = ["dnode", "runtime_process", "dserv_realization", "dcode_execution", "dcall_delivery", "diot_realization", "network_reachability", "unbound"];
  for (const kind of kinds) {
    const label = subjectKindLabel({ kind, node_id: "fog-01", runtime_ref: "x", project_id: "p", application_id: "a", service_id: "s", call_id: "c", diot_id: "d", diot_placement_id: "dp", target: "t" });
    assert.ok(label && label.length > 0, `kind '${kind}' must have a non-empty label`);
  }
});

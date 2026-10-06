import test from "node:test";
import assert from "node:assert/strict";
import {
  validDMonitorHistoryQueryResponse,
  HEALTH_LABEL,
  FRESHNESS_LABEL,
  FRESHNESS_TONE,
  subjectDisplayIdentity,
  subjectKindLabel,
  defaultHistoryFilters,
  clientSideHistoryValidationError,
  historyObservationIdentityLine,
  MAX_HISTORY_WINDOW_SECONDS,
  DEFAULT_HISTORY_WINDOW_SECONDS,
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

// Phase 140B -- Part C: a useful, BOUNDED default (never unbounded history).
test("defaultHistoryFilters defaults to the most recent 24 hours, never unbounded, and no optional filter is pre-filled", () => {
  const now = new Date("2026-10-05T12:00:00.000Z");
  const filters = defaultHistoryFilters(now);
  assert.equal(filters.endUtc, now.toISOString());
  assert.equal(Date.parse(now.toISOString()) - Date.parse(filters.startUtc), DEFAULT_HISTORY_WINDOW_SECONDS * 1000);
  assert.equal(filters.applicationId, "");
  assert.equal(filters.nodeId, "");
  assert.equal(filters.serviceId, "");
  assert.equal(filters.limit, "");
});

// Part B/Audit 9/10 -- a >7-day window or start>end must be clearly
// blocked client-side BEFORE any request is sent, never silently narrowed.
test("clientSideHistoryValidationError rejects a window wider than 7 days without altering it", () => {
  const filters = { ...defaultHistoryFilters(), startUtc: "2026-09-01T00:00:00Z", endUtc: "2026-10-05T00:00:00Z" };
  const error = clientSideHistoryValidationError(filters);
  assert.ok(error);
  assert.match(error, /exceeds the maximum/);
  assert.match(error, new RegExp(`${MAX_HISTORY_WINDOW_SECONDS / 86_400} days`));
});

test("clientSideHistoryValidationError rejects start after end clearly", () => {
  const filters = { ...defaultHistoryFilters(), startUtc: "2026-10-05T00:00:00Z", endUtc: "2026-10-01T00:00:00Z" };
  const error = clientSideHistoryValidationError(filters);
  assert.ok(error);
  assert.match(error, /must not be after/);
});

test("clientSideHistoryValidationError accepts a window within 7 days and a valid limit", () => {
  const filters = { ...defaultHistoryFilters(), limit: "50" };
  assert.equal(clientSideHistoryValidationError(filters), undefined);
});

test("clientSideHistoryValidationError rejects a non-positive or non-integer limit", () => {
  assert.ok(clientSideHistoryValidationError({ ...defaultHistoryFilters(), limit: "0" }));
  assert.ok(clientSideHistoryValidationError({ ...defaultHistoryFilters(), limit: "-5" }));
  assert.ok(clientSideHistoryValidationError({ ...defaultHistoryFilters(), limit: "abc" }));
});

test("clientSideHistoryValidationError rejects an unparsable start/end timestamp", () => {
  assert.ok(clientSideHistoryValidationError({ ...defaultHistoryFilters(), startUtc: "not-a-timestamp" }));
  assert.ok(clientSideHistoryValidationError({ ...defaultHistoryFilters(), endUtc: "not-a-timestamp" }));
});

// Part E -- ClockSkew must never collapse into Stale, textually or by tone.
test("FRESHNESS_LABEL/FRESHNESS_TONE never collapse ClockSkew into Stale", () => {
  assert.notEqual(FRESHNESS_LABEL.clock_skew, FRESHNESS_LABEL.stale);
  assert.ok(FRESHNESS_LABEL.clock_skew.toLowerCase().includes("clock"));
  assert.notEqual(FRESHNESS_TONE.clock_skew, undefined);
});

test("historyObservationIdentityLine includes application/service only when the subject actually carries them, never PID/container-id", () => {
  const dserv = historicalObservationFixture().subject;
  const line = historyObservationIdentityLine(dserv);
  assert.match(line, /app: app-1/);
  assert.match(line, /service: service-a/);
  const dnodeLine = historyObservationIdentityLine({ kind: "dnode", node_id: "fog-01" });
  assert.doesNotMatch(dnodeLine, /app:|service:/);
});

test("subjectKindLabel covers every one of the 8 canonical subject kinds", () => {
  const kinds = ["dnode", "runtime_process", "dserv_realization", "dcode_execution", "dcall_delivery", "diot_realization", "network_reachability", "unbound"];
  for (const kind of kinds) {
    const label = subjectKindLabel({ kind, node_id: "fog-01", runtime_ref: "x", project_id: "p", application_id: "a", service_id: "s", call_id: "c", diot_id: "d", diot_placement_id: "dp", target: "t" });
    assert.ok(label && label.length > 0, `kind '${kind}' must have a non-empty label`);
  }
});

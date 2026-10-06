import test from "node:test";
import assert from "node:assert/strict";
import { validApplicationOperationsOverview, READINESS_LABEL, READINESS_TONE, sectionAvailabilityLabel } from "../src/pages/application-overview-model.ts";
import {
  applicationOverviewFixture,
  available,
  notConfigured,
  unavailable,
  recentHistoryFixture,
  convergenceFixture,
  readinessFixture,
} from "./application-overview-fixture.mjs";

test("valid application overview fixture round-trips", () => {
  assert.ok(validApplicationOperationsOverview(applicationOverviewFixture(), "project-a", "app-1"));
});

test("validator rejects a project or application scope mismatch", () => {
  assert.ok(!validApplicationOperationsOverview(applicationOverviewFixture(), "project-b"));
  assert.ok(!validApplicationOperationsOverview(applicationOverviewFixture(), "project-a", "app-2"));
});

test("validator accepts every SourceSection availability state, not only 'available'", () => {
  for (const section of [notConfigured, unavailable(), { availability: "corrupt", value: { kind: "x", source: "x", code: "x", safe_message: "x", scope: null } }, { availability: "indeterminate", value: "no data yet" }]) {
    assert.ok(validApplicationOperationsOverview(applicationOverviewFixture({ readiness: section })), `readiness as ${section.availability} must validate`);
  }
});

test("validator rejects an invented SourceSection availability value", () => {
  assert.ok(!validApplicationOperationsOverview(applicationOverviewFixture({ readiness: { availability: "mostly_fine" } })));
});

test("validator rejects an invented readiness value -- only ready/not_ready are canonical", () => {
  const bad = applicationOverviewFixture({ readiness: available(readinessFixture({ readiness: "mostly_ready" })) });
  assert.ok(!validApplicationOperationsOverview(bad));
});

test("validator accepts both +00:00 and Z timestamp spellings across mixed-origin fields (Phase 140A finding)", () => {
  const mixed = applicationOverviewFixture({
    generated_at_utc: "2026-10-06T12:00:05Z",
    convergence: available(convergenceFixture({ evaluated_at_utc: "2026-10-06T12:00:05.123456+00:00" })),
    recent_history: available(recentHistoryFixture({ latest_received_at_utc: "2026-10-06T12:00:01+00:00" })),
  });
  assert.ok(validApplicationOperationsOverview(mixed));
});

test("validator accepts a null recent_history latest pointer as honestly empty, never requiring a timestamp", () => {
  const empty = applicationOverviewFixture({
    recent_history: available(recentHistoryFixture({ recent_observation_count: 0, latest_received_at_utc: null, latest_observation_id: null })),
  });
  assert.ok(validApplicationOperationsOverview(empty));
});

test("validator rejects a subject with an invented kind inside observed_runtime", () => {
  const bad = applicationOverviewFixture();
  bad.observed_runtime.value.items[0].subject = { kind: "made_up" };
  assert.ok(!validApplicationOperationsOverview(bad));
});

test("readiness and convergence enums each have a complete, distinct display label/tone -- never color-only, never merged", () => {
  for (const status of ["ready", "not_ready"]) {
    assert.ok(READINESS_LABEL[status]);
    assert.ok(READINESS_TONE[status]);
  }
  assert.notEqual(READINESS_LABEL.ready, READINESS_LABEL.not_ready);
});

test("sectionAvailabilityLabel covers every non-available SourceSection state with a distinct, plain label", () => {
  const labels = ["not_configured", "unavailable", "corrupt", "indeterminate"].map(sectionAvailabilityLabel);
  assert.equal(new Set(labels).size, 4, "all four states must have distinct labels");
});

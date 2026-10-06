import test from "node:test";
import assert from "node:assert/strict";
import {
  validDMonitorApplicationConvergence,
  validEvaluateConvergenceResponse,
  CONVERGENCE_LABEL,
  FRESHNESS_LABEL,
  HEALTH_LABEL,
  latestAdmittedEvidence,
  isWaitingForFirstObservation,
  isConvergenceSettled,
  findServicePlacement,
} from "../src/pages/dmonitor-convergence-model.ts";
import {
  evidenceFixture,
  servicePlacementFixture,
  applicationConvergenceFixture,
  evaluateConvergenceResponseFixture,
} from "./dmonitor-convergence-fixture.mjs";

test("valid application convergence and full evaluate response fixtures round-trip", () => {
  assert.ok(validDMonitorApplicationConvergence(applicationConvergenceFixture(), "project-a", "app-1"));
  assert.ok(validEvaluateConvergenceResponse(evaluateConvergenceResponseFixture(), "project-a", "app-1"));
});

test("convergence validator rejects project/application scope mismatch and an unknown convergence value", () => {
  assert.ok(!validDMonitorApplicationConvergence(applicationConvergenceFixture(), "project-b"));
  assert.ok(!validDMonitorApplicationConvergence(applicationConvergenceFixture(), "project-a", "app-2"));
  const invalid = { ...applicationConvergenceFixture(), convergence: "healthy" };
  assert.ok(!validDMonitorApplicationConvergence(invalid));
});

test("convergence validator requires every admitted/excluded evidence field, rejects an invented freshness value", () => {
  const bad = applicationConvergenceFixture("app-1", {
    services: [servicePlacementFixture("service-a", { admitted_evidence: [evidenceFixture({ freshness: "degraded" })] })],
  });
  assert.ok(!validDMonitorApplicationConvergence(bad));
});

test("validEvaluateConvergenceResponse requires the diot field to at least be an object (structural decode only)", () => {
  const { diot: _diot, ...withoutDiot } = evaluateConvergenceResponseFixture();
  assert.ok(!validEvaluateConvergenceResponse(withoutDiot));
});

test("every enum variant this module validates has a corresponding display label -- no silent fallthrough", () => {
  for (const status of ["converged", "diverged", "unknown"]) assert.ok(CONVERGENCE_LABEL[status]);
  for (const status of ["fresh", "stale", "clock_skew"]) assert.ok(FRESHNESS_LABEL[status]);
  for (const status of ["unknown", "healthy", "degraded", "unhealthy"]) assert.ok(HEALTH_LABEL[status]);
});

test("Stale and Unknown remain textually distinct labels, never collapsed (Part M)", () => {
  assert.notEqual(FRESHNESS_LABEL.stale, CONVERGENCE_LABEL.unknown);
  assert.ok(FRESHNESS_LABEL.stale.toLowerCase().includes("stale"));
});

test("latestAdmittedEvidence picks the evidence with the latest received_at_utc, not simply the first", () => {
  const placement = servicePlacementFixture("service-a", {
    admitted_evidence: [
      evidenceFixture({ observation_id: "older", received_at_utc: "2026-10-05T11:00:00Z" }),
      evidenceFixture({ observation_id: "newer", received_at_utc: "2026-10-05T13:00:00Z" }),
    ],
  });
  assert.equal(latestAdmittedEvidence(placement)?.observation_id, "newer");
});
test("latestAdmittedEvidence returns undefined for zero evidence", () => {
  assert.equal(latestAdmittedEvidence(servicePlacementFixture("service-a", { admitted_evidence: [] })), undefined);
});

test("isWaitingForFirstObservation is true only when there is genuinely zero evidence, admitted or excluded", () => {
  assert.ok(isWaitingForFirstObservation(servicePlacementFixture("service-a", { admitted_evidence: [], excluded_evidence: [] })));
  assert.ok(!isWaitingForFirstObservation(servicePlacementFixture("service-a", { admitted_evidence: [evidenceFixture()] })));
  assert.ok(!isWaitingForFirstObservation(servicePlacementFixture("service-a", { admitted_evidence: [], excluded_evidence: [evidenceFixture({ admitted: false, exclusion_reason: "stale_evidence" })] })));
  assert.ok(isWaitingForFirstObservation(undefined));
});

test("isConvergenceSettled is false for unknown (keep polling) and true for converged/diverged (Part S)", () => {
  assert.ok(!isConvergenceSettled(applicationConvergenceFixture("app-1", { convergence: "unknown" })));
  assert.ok(isConvergenceSettled(applicationConvergenceFixture("app-1", { convergence: "converged" })));
  assert.ok(isConvergenceSettled(applicationConvergenceFixture("app-1", { convergence: "diverged" })));
  assert.ok(!isConvergenceSettled(undefined));
});

// Phase 140A finding -- `evaluated_at_utc` is real-backend-generated via
// plain `Utc::now().to_rfc3339()`, which renders as `+00:00`, never `Z`.
// The validator must accept BOTH spellings of the same UTC instant.
test("validator accepts a +00:00-offset timestamp exactly as the real backend formatter produces, not only a Z-suffixed one", () => {
  const withOffset = applicationConvergenceFixture("app-1", { evaluated_at_utc: "2026-10-05T12:00:05.123456+00:00" });
  assert.ok(validDMonitorApplicationConvergence(withOffset));
});

test("findServicePlacement looks up by exact service_id, never a prefix/fuzzy match", () => {
  const convergence = applicationConvergenceFixture("app-1", { services: [servicePlacementFixture("service-a"), servicePlacementFixture("service-ab")] });
  assert.equal(findServicePlacement(convergence, "service-a")?.service_id, "service-a");
  assert.equal(findServicePlacement(convergence, "service-x"), undefined);
  assert.equal(findServicePlacement(undefined, "service-a"), undefined);
});

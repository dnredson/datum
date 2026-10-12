import test from "node:test";
import assert from "node:assert/strict";
import {
  activeResponseFixture,
  candidateResponseFixture,
  validateResponseFixture,
  proposalResponseFixture,
} from "./placement-fixture.mjs";
import {
  validActiveResponse,
  validCandidateResponse,
  validValidateResponse,
  validProposalResponse,
  placementDiff,
} from "../src/pages/placement-model.ts";

test("deterministic active-response fixture validates", () => {
  assert.ok(validActiveResponse(activeResponseFixture(), "project-a"));
});
test("active response is rejected for a different project", () => {
  assert.ok(!validActiveResponse(activeResponseFixture("project-a"), "project-b"));
});
test("deterministic candidate-response fixture validates and carries no proposal identity", () => {
  const f = candidateResponseFixture();
  assert.ok(validCandidateResponse(f, "project-a", "app-1"));
  assert.ok(!("proposal_id" in f), "a candidate must never carry a proposal_id");
});
test("candidate response is rejected for a mismatched application_id", () => {
  assert.ok(!validCandidateResponse(candidateResponseFixture(), "project-a", "app-other"));
});
test("unknown schema on the candidate response is rejected, never silently coerced", () => {
  const f = { ...candidateResponseFixture(), schema: "datum.placement-candidate/2" };
  assert.ok(!validCandidateResponse(f, "project-a", "app-1"));
});
test("deterministic validate-response fixture validates and always carries the momentary-capacity notice", () => {
  const f = validateResponseFixture();
  assert.ok(validValidateResponse(f));
  assert.ok(f.capacity_check_is_momentary.length > 0);
});
test("valid must agree with an empty findings list", () => {
  const f = { ...validateResponseFixture(), valid: true, findings: [{ code: "x", message: "y" }] };
  assert.ok(!validValidateResponse(f), "valid=true with non-empty findings must be rejected as internally inconsistent");
});
test("deterministic proposal-response fixture validates as Pending, never Accepted by construction", () => {
  const f = proposalResponseFixture();
  assert.ok(validProposalResponse(f, "project-a", "app-1"));
  assert.equal(f.state, "pending");
});
test("proposal response accepts RFC3339 numeric UTC offsets emitted by the Rust backend", () => {
  const f = proposalResponseFixture();
  f.proposal.created_at_utc = "2026-10-12T20:48:48.273621300+00:00";
  assert.ok(validProposalResponse(f, "project-a", "app-1"));
});
test("proposal response accepts non-zero RFC3339 offsets but rejects a missing timezone", () => {
  const offset = proposalResponseFixture();
  offset.proposal.created_at_utc = "2026-10-12T17:48:48.273621300-03:00";
  assert.ok(validProposalResponse(offset, "project-a", "app-1"));

  const noZone = proposalResponseFixture();
  noZone.proposal.created_at_utc = "2026-10-12T20:48:48.273621300";
  assert.ok(!validProposalResponse(noZone, "project-a", "app-1"));
});
test("proposal response rejects an unknown lifecycle state", () => {
  const f = { ...proposalResponseFixture(), state: "activated" };
  assert.ok(!validProposalResponse(f, "project-a", "app-1"));
});

// ----------------------------------------------------------------- Diff

test("diff: moved, added, and unchanged are all distinguished by graph-local service_id only", () => {
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [
    { service_id: "service-a", node_id: "node-b" },
    { service_id: "service-b", node_id: "node-a" },
  ];
  const entries = placementDiff(current, candidate);
  assert.deepEqual(
    entries.map((e) => [e.service_id, e.change, e.from_node_id, e.to_node_id]),
    [
      ["service-a", "moved", "node-a", "node-b"],
      ["service-b", "added", null, "node-a"],
    ],
  );
});
test("diff: a service present on both sides at the same node is unchanged", () => {
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [{ service_id: "service-a", node_id: "node-a" }];
  const entries = placementDiff(current, candidate);
  assert.deepEqual(entries, [{ service_id: "service-a", from_node_id: "node-a", to_node_id: "node-a", change: "unchanged" }]);
});
test("diff: a service dropped from the candidate (present currently, absent from candidate) is removed", () => {
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [];
  const entries = placementDiff(current, candidate);
  assert.deepEqual(entries, [{ service_id: "service-a", from_node_id: "node-a", to_node_id: null, change: "removed" }]);
});
test("diff: no current placement and no candidate placement never appears in the diff at all", () => {
  const entries = placementDiff([], []);
  assert.deepEqual(entries, []);
});
test("diff: deterministic ordering by service_id regardless of input order", () => {
  const current = [];
  const candidate = [
    { service_id: "service-z", node_id: "node-a" },
    { service_id: "service-a", node_id: "node-a" },
  ];
  const entries = placementDiff(current, candidate);
  assert.deepEqual(entries.map((e) => e.service_id), ["service-a", "service-z"]);
});
test("diff computation does not mutate its inputs", () => {
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [{ service_id: "service-a", node_id: "node-b" }];
  const beforeCurrent = JSON.parse(JSON.stringify(current));
  const beforeCandidate = JSON.parse(JSON.stringify(candidate));
  placementDiff(current, candidate);
  assert.deepEqual(current, beforeCurrent);
  assert.deepEqual(candidate, beforeCandidate);
});

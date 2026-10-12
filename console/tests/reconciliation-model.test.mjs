import test from "node:test";
import assert from "node:assert/strict";
import {
  validReconciliationRequest,
  validReconciliationBatchResponse,
  validReconciliationBatchCreateResponse,
  validReconciliationAttemptRecord,
  validOperationalReconciliationAuthorization,
  validOperationalDGraphSlice,
  authorizationDisplayStatus,
  isTerminalRequestStatus,
  isTerminalBatchStatus,
  isOtherApplicationService,
  nextClientRequestId,
  REQUEST_STATUS_LABEL,
  BATCH_STATUS_LABEL,
  ATTEMPT_STATUS_LABEL,
  ACTION_RESULT_LABEL,
} from "../src/pages/reconciliation-model.ts";
import {
  activeResponseFixture,
  operationalDGraphSliceFixture,
  authorizationFixture,
  reconciliationRequestFixture,
  reconciliationBatchResponseFixture,
  reconciliationBatchCreateResponseFixture,
  reconciliationAttemptRecordFixture,
} from "./reconciliation-fixture.mjs";

test("valid reconciliation request fixture round-trips through the validator", () => {
  assert.ok(validReconciliationRequest(reconciliationRequestFixture("node-a"), "project-a"));
});
test("reconciliation request accepts RFC3339 numeric offsets emitted by the Rust backend", () => {
  const request = reconciliationRequestFixture("node-a");
  request.created_at_utc = "2026-10-12T20:48:48.273621300+00:00";
  request.expires_at_utc = "2026-10-12T21:48:48.273621300+00:00";
  request.updated_at_utc = "2026-10-12T17:49:00.000000000-03:00";
  assert.ok(validReconciliationRequest(request, "project-a"));
});
test("reconciliation request rejects timestamps without an RFC3339 timezone", () => {
  const request = reconciliationRequestFixture("node-a");
  request.updated_at_utc = "2026-10-12T20:49:00.000000000";
  assert.ok(!validReconciliationRequest(request, "project-a"));
});
test("reconciliation request validator rejects an unknown status/mode and a project scope mismatch", () => {
  assert.ok(!validReconciliationRequest({ ...reconciliationRequestFixture("node-a"), status: "done" }));
  assert.ok(!validReconciliationRequest({ ...reconciliationRequestFixture("node-a"), mode: "deploy" }));
  assert.ok(!validReconciliationRequest(reconciliationRequestFixture("node-a"), "project-b"));
});
test("reconciliation request validator requires every non-nullable field", () => {
  const { reconciliation_request_id, ...missing } = reconciliationRequestFixture("node-a");
  assert.ok(!validReconciliationRequest(missing));
});

test("valid batch response and batch-create response fixtures round-trip", () => {
  const requests = [reconciliationRequestFixture("node-a"), reconciliationRequestFixture("node-b")];
  assert.ok(validReconciliationBatchResponse(reconciliationBatchResponseFixture(requests), "project-a"));
  assert.ok(validReconciliationBatchCreateResponse(reconciliationBatchCreateResponseFixture(requests), "app-1"));
});
test("batch response validator rejects an unknown batch status", () => {
  const requests = [reconciliationRequestFixture("node-a")];
  const invalid = { ...reconciliationBatchResponseFixture(requests), status: "converged" };
  assert.ok(!validReconciliationBatchResponse(invalid));
});

test("valid attempt record fixture round-trips, including per-service results", () => {
  assert.ok(validReconciliationAttemptRecord(reconciliationAttemptRecordFixture("reconciliation-attempt:1", "node-a"), "project-a"));
});
test("attempt record validator rejects an invented action-result vocabulary", () => {
  const record = reconciliationAttemptRecordFixture("reconciliation-attempt:1", "node-a");
  record.services["service-a"].result = "created"; // not a real ReconciliationActionResult
  assert.ok(!validReconciliationAttemptRecord(record));
});
test("attempt record validator accepts services_truncated and an absent reconciliation_request_id (manual --execute backward compatibility)", () => {
  const record = reconciliationAttemptRecordFixture("reconciliation-attempt:1", "node-a", { services_truncated: true });
  delete record.reconciliation_request_id;
  assert.ok(validReconciliationAttemptRecord(record));
});

test("valid authorization and operational D-Graph slice fixtures round-trip", () => {
  const authorization = authorizationFixture("node-a");
  assert.ok(validOperationalReconciliationAuthorization(authorization));
  assert.ok(validOperationalDGraphSlice(operationalDGraphSliceFixture("node-a", { authorization }), "project-a", "node-a"));
});
test("operational D-Graph slice validator accepts an absent authorization (Part E1 'no authorization issued')", () => {
  assert.ok(validOperationalDGraphSlice(operationalDGraphSliceFixture("node-a")));
});

test("authorizationDisplayStatus distinguishes none/not_authorized/wrong_target/expired/authorized", () => {
  const now = Date.parse("2026-10-05T12:30:00Z");
  assert.equal(authorizationDisplayStatus(undefined, "dgraph:project-a", 7, now), "none");
  assert.equal(authorizationDisplayStatus(authorizationFixture("node-a", { allow_host_mutation: false }), "dgraph:project-a", 7, now), "not_authorized");
  assert.equal(authorizationDisplayStatus(authorizationFixture("node-a"), "dgraph:project-a", 8, now), "wrong_target");
  assert.equal(authorizationDisplayStatus(authorizationFixture("node-a", { expires_at_utc: "2026-10-05T12:00:00Z" }), "dgraph:project-a", 7, now), "expired");
  assert.equal(authorizationDisplayStatus(authorizationFixture("node-a"), "dgraph:project-a", 7, now), "authorized");
});
test("authorizationDisplayStatus treats a missing expires_at_utc as never expiring", () => {
  const authorization = authorizationFixture("node-a");
  delete authorization.expires_at_utc;
  assert.equal(authorizationDisplayStatus(authorization, "dgraph:project-a", 7, Date.parse("2099-01-01T00:00:00Z")), "authorized");
});

test("terminal status helpers match exactly the backend's own terminal states", () => {
  assert.ok(isTerminalRequestStatus("attempted"));
  assert.ok(isTerminalRequestStatus("stale"));
  assert.ok(isTerminalRequestStatus("expired"));
  assert.ok(!isTerminalRequestStatus("pending"));
  assert.ok(!isTerminalRequestStatus("claimed"));
  for (const status of ["succeeded", "partial", "failed", "unknown", "stale"]) assert.ok(isTerminalBatchStatus(status));
  for (const status of ["pending", "executing"]) assert.ok(!isTerminalBatchStatus(status));
});

test("isOtherApplicationService flags a service placed by a different application, never the selected one or an unknown one", () => {
  assert.ok(isOtherApplicationService({ application_id: "app-2" }, "app-1"));
  assert.ok(!isOtherApplicationService({ application_id: "app-1" }, "app-1"));
  assert.ok(!isOtherApplicationService({ application_id: "" }, "app-1"));
});

test("every enum variant this module validates has a corresponding display label -- no silent fallthrough", () => {
  for (const status of ["pending", "claimed", "attempted", "stale", "expired"]) assert.ok(REQUEST_STATUS_LABEL[status]);
  for (const status of ["pending", "executing", "succeeded", "partial", "failed", "unknown", "stale"]) assert.ok(BATCH_STATUS_LABEL[status]);
  for (const status of ["succeeded", "partial", "failed", "unknown"]) assert.ok(ATTEMPT_STATUS_LABEL[status]);
  for (const result of ["no_op", "mutated", "blocked", "unknown"]) assert.ok(ACTION_RESULT_LABEL[result]);
});

test("command-side result labels never claim runtime health (Part L)", () => {
  for (const label of Object.values(ATTEMPT_STATUS_LABEL)) {
    assert.doesNotMatch(label, /healthy|converged/iu);
  }
});

test("active response fixture matches the real Phase 137 ActiveResponse shape well enough to derive node ids", () => {
  const active = activeResponseFixture("app-1");
  const nodeIds = new Set(active.deployments[0].active_dmap.placements.map((p) => p.node_id));
  assert.deepEqual([...nodeIds].sort(), ["node-a", "node-b"]);
});

// Part F3/O -- Audits 18/19: client_request_id lifecycle.
test("nextClientRequestId: first-ever submission always generates a fresh id", () => {
  let calls = 0;
  const id = nextClientRequestId(undefined, true, () => `id-${++calls}`);
  assert.equal(id, "id-1");
  assert.equal(calls, 1);
});
test("nextClientRequestId: a known outcome (success or definite rejection) always gets a fresh id on the next submission, never the old one", () => {
  let calls = 0;
  const id = nextClientRequestId("old-id", true, () => `id-${++calls}`);
  assert.equal(id, "id-1");
  assert.notEqual(id, "old-id");
});
test("nextClientRequestId: an ambiguous (unknown) outcome reuses the SAME prior id, never generating a new one", () => {
  const generate = () => {
    throw new Error("must not generate a new id when retrying an ambiguous outcome");
  };
  const id = nextClientRequestId("old-id", false, generate);
  assert.equal(id, "old-id");
});
test("nextClientRequestId: an ambiguous outcome with no prior id still generates fresh (nothing to retry)", () => {
  let calls = 0;
  const id = nextClientRequestId(undefined, false, () => `id-${++calls}`);
  assert.equal(id, "id-1");
});

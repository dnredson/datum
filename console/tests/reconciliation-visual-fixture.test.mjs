// Proves the Phase 138 Deploy/Reconciliation visual fixture
// (reconciliation-visual-fixture.mjs) serves a contract-valid response for
// every scenario and route the workspace actually calls -- validated
// against the REAL Console validators, not merely eyeballed. Binds an
// ephemeral port (never the fixed manual-review port).
import test from "node:test";
import assert from "node:assert/strict";
import { createFixtureServer } from "./reconciliation-visual-fixture.mjs";
import {
  validOperationalDGraphSlice,
  validOperationalReconciliationAuthorization,
  validReconciliationBatchCreateResponse,
  validReconciliationBatchResponse,
  validReconciliationAttemptRecord,
  isOtherApplicationService,
} from "../src/pages/reconciliation-model.ts";

async function withServer(run) {
  const server = createFixtureServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  try {
    await run(`http://127.0.0.1:${port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

async function createBatch(baseUrl, applicationId, mode = "execute") {
  return fetch(`${baseUrl}/api/v1/reconciliation-requests/batch`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Datum-Project": "project-a" },
    body: JSON.stringify({ application_id: applicationId, mode, requested_by: "fixture-reviewer", client_request_id: `client:${applicationId}` }),
  });
}

test("scenario A: Preview stays Pending -- waiting for agent, never advances", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-preview-waiting", "preview")).json();
    assert.ok(validReconciliationBatchCreateResponse(created, "app-preview-waiting"));
    assert.equal(created.requests[0].status, "pending");
    const batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.ok(validReconciliationBatchResponse(batch, "project-a"));
    assert.equal(batch.status, "executing");
    assert.equal(batch.requests[0].status, "pending");
  });
});

test("scenario B: Preview completes with zero mutation (every service no_op)", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-preview-done", "preview")).json();
    let batch;
    for (let i = 0; i < 5; i++) {
      batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
      if (batch.status === "succeeded") break;
    }
    assert.ok(validReconciliationBatchResponse(batch, "project-a"));
    assert.equal(batch.status, "succeeded");
    const attemptId = batch.requests[0].linked_attempt_id;
    const attempt = await (await fetch(`${baseUrl}/api/v1/reconciliation-attempts/${encodeURIComponent(attemptId)}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.ok(validReconciliationAttemptRecord(attempt, "project-a"));
    assert.ok(Object.values(attempt.services).every((s) => s.result === "no_op" && s.host_mutation_performed === false));
  });
});

test("scenario C: Execute, Authorized + Succeeded, single node", async () => {
  await withServer(async (baseUrl) => {
    const slice = await (await fetch(`${baseUrl}/api/v1/nodes/node-auth-c/operational-dgraph-slice`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.ok(validOperationalDGraphSlice(slice, "project-a", "node-auth-c"));
    assert.ok(validOperationalReconciliationAuthorization(slice.authorization));
    assert.equal(slice.authorization.allow_host_mutation, true);

    const created = await (await createBatch(baseUrl, "app-authorized-succeeded")).json();
    let batch;
    for (let i = 0; i < 5; i++) {
      batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
      if (batch.status === "succeeded") break;
    }
    assert.equal(batch.status, "succeeded");
  });
});

test("scenario D: Execute, Partial across 3 nodes -- 2 succeed, 1 fails", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-partial-3node")).json();
    assert.equal(created.requests.length, 3);
    let batch;
    for (let i = 0; i < 5; i++) {
      batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
      if (batch.status === "partial") break;
    }
    assert.ok(validReconciliationBatchResponse(batch, "project-a"));
    assert.equal(batch.status, "partial");
    const failedNode = batch.requests.find((r) => r.node_id === "rpi-01");
    const attempt = await (await fetch(`${baseUrl}/api/v1/reconciliation-attempts/${encodeURIComponent(failedNode.linked_attempt_id)}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.equal(attempt.status, "failed");
  });
});

test("scenario E: Stale after authority changed -- never silently retargeted", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-stale")).json();
    let batch;
    for (let i = 0; i < 5; i++) {
      batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
      if (batch.status === "stale") break;
    }
    assert.equal(batch.status, "stale");
    assert.equal(batch.requests[0].status, "stale");
  });
});

test("scenario F: Queue full -- creation itself is refused with the structured code", async () => {
  await withServer(async (baseUrl) => {
    const response = await createBatch(baseUrl, "app-queue-full");
    assert.equal(response.status, 409);
    const body = await response.json();
    assert.equal(body.code, "reconciliation_queue_full");
  });
});

test("scenario G: Agent offline -- request expires, never reported as failure", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-offline-expired")).json();
    const batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.equal(batch.requests[0].status, "expired");
  });
});

test("scenario H: First deployment -- preview/execute work with no prior runtime", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-first-deploy")).json();
    let batch;
    for (let i = 0; i < 5; i++) {
      batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
      if (batch.status === "succeeded") break;
    }
    assert.equal(batch.status, "succeeded");
  });
});

test("scenario I: Cross-application node-wide disclosure -- node-a's slice carries an app-2 service while this workspace is app-1", async () => {
  await withServer(async (baseUrl) => {
    const slice = await (await fetch(`${baseUrl}/api/v1/nodes/node-a/operational-dgraph-slice`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.ok(validOperationalDGraphSlice(slice, "project-a", "node-a"));
    const otherAppServices = slice.desired_services.filter((s) => isOtherApplicationService(s, "app-1"));
    assert.ok(otherAppServices.length >= 1, "the fixture must actually exercise the cross-application disclosure path");
  });
});

test("scenario J: Unknown attempt result -- never fabricated as Succeeded or Failed", async () => {
  await withServer(async (baseUrl) => {
    const created = await (await createBatch(baseUrl, "app-unknown-result")).json();
    const batch = await (await fetch(`${baseUrl}/api/v1/reconciliation-requests/batch/${created.reconciliation_batch_id}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    const attempt = await (await fetch(`${baseUrl}/api/v1/reconciliation-attempts/${encodeURIComponent(batch.requests[0].linked_attempt_id)}`, { headers: { "X-Datum-Project": "project-a" } })).json();
    assert.ok(validReconciliationAttemptRecord(attempt, "project-a"));
    assert.equal(attempt.status, "unknown");
  });
});

test("authorize endpoint returns a valid authorization for any node", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/nodes/node-a/operational-reconciliation/authorize`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Datum-Project": "project-a" },
      body: JSON.stringify({ dgraph_id: "dgraph:project-a", dgraph_revision: 7, authorized_by: "fixture-reviewer", allow_host_mutation: true }),
    });
    assert.equal(response.status, 200);
    const authorization = await response.json();
    assert.ok(validOperationalReconciliationAuthorization(authorization));
  });
});

test("an unrelated unknown route still fails explicitly, never a silent 200", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/unrelated-route`);
    assert.equal(response.status, 404);
  });
});

test("unknown application_id in active/batch creation is reported, never a fabricated empty-success", async () => {
  await withServer(async (baseUrl) => {
    const active = await (await fetch(`${baseUrl}/api/v1/ddeploy/active?application_id=app-does-not-exist`)).json();
    assert.deepEqual(active.deployments, []);
    const response = await createBatch(baseUrl, "app-does-not-exist");
    assert.equal(response.status, 404);
  });
});

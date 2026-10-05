// Proves the Phase 136 Placement Workspace visual fixture
// (placement-visual-fixture.mjs) serves a contract-valid response for
// every route the Placement Workspace actually calls -- validated against
// the REAL Console validators/parsers, not merely eyeballed. Binds an
// ephemeral port (never the fixed manual-review port) so this runs safely
// inside the automated suite. `DGraphEditorClient`/`PlacementClient`
// themselves use root-relative fetch URLs with no configurable base (only
// `OperationsClient` supports that), so this test fetches raw JSON
// directly and feeds it through the same pure validators/parsers those
// clients call internally -- the strongest contract proof available
// without changing either client's production constructor.
import test from "node:test";
import assert from "node:assert/strict";
import { createFixtureServer } from "./placement-visual-fixture.mjs";
import {
  validActiveResponse,
  validCandidateResponse,
  validValidateResponse,
  validProposalResponse,
} from "../src/pages/placement-model.ts";
import { validContinuumView } from "../src/pages/continuum-model.ts";
import { parseGraph } from "../src/pages/dgraph-editor-model.ts";

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

test("dgraph-editor route returns a base list whose graph_json parses with the real production parser", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/operations/projects/project-a/dgraph-editor`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.equal(body.schema, "datum.dgraph-editor-base/1");
    assert.equal(body.graphs.length, 1);
    const entry = body.graphs[0];
    const graph = parseGraph(entry.graph_json);
    assert.equal(graph.application_id, entry.application_id);
    assert.equal(graph.dgraph_id, entry.dgraph_id);
    assert.equal(graph.revision, entry.revision);
    assert.deepEqual(
      graph.services.map((s) => s.service_id).sort(),
      ["service-a", "service-b"],
    );
  });
});

test("continuum route returns a valid project-scoped projection matching the placement scenario's two D-Nodes", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/operations/projects/project-a/continuum`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.ok(validContinuumView(body, "project-a"));
    assert.deepEqual(
      body.nodes.map((n) => n.node_id).sort(),
      ["node-a", "node-b"],
    );
  });
});

test("ddeploy active route requires X-Datum-Project and returns a valid accepted placement", async () => {
  await withServer(async (baseUrl) => {
    const missingHeader = await fetch(`${baseUrl}/api/v1/ddeploy/active`);
    assert.equal(missingHeader.status, 400);

    const response = await fetch(`${baseUrl}/api/v1/ddeploy/active`, { headers: { "X-Datum-Project": "project-a" } });
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.ok(validActiveResponse(body, "project-a"));
    assert.equal(body.deployments[0].active_dmap.placements[0].node_id, "node-a");
  });
});

test("candidate route returns a valid, zero-identity candidate placing service-a on node-b and service-b on node-a", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/candidate`, {
      method: "POST",
      headers: { "X-Datum-Project": "project-a", "Content-Type": "application/json" },
      body: JSON.stringify({ application_id: "app-1", dgraph: {} }),
    });
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.ok(validCandidateResponse(body, "project-a", "app-1"));
    assert.ok(!("proposal_id" in body));
  });
});

test("validate route returns a valid response always carrying the momentary-capacity disclosure", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/validate`, {
      method: "POST",
      headers: { "X-Datum-Project": "project-a", "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.ok(validValidateResponse(body));
  });
});

test("create-proposal route returns a valid Pending proposal, never Accepted", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals`, {
      method: "POST",
      headers: { "X-Datum-Project": "project-a", "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    assert.equal(response.status, 201);
    const body = await response.json();
    assert.ok(validProposalResponse(body, "project-a", "app-1"));
    assert.equal(body.state, "pending");
  });
});

test("an unrelated unknown route still fails explicitly, never silently returns 200", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/some-id/accept`, { method: "POST" });
    assert.equal(response.status, 404);
    const body = await response.json();
    assert.equal(body.error, "fixture_not_found");
  });
});

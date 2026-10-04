// Proves the Phase 135A integrated Console review fixture
// (continuum-visual-fixture.mjs) serves a contract-valid response for every
// shell route the Console actually requests while navigating, without
// turning the fixture into a wildcard fake server. Binds an ephemeral port
// (never the fixed manual-review port) so this runs safely inside the
// automated suite.
import test from "node:test";
import assert from "node:assert/strict";
import { createFixtureServer } from "./continuum-visual-fixture.mjs";
import { OperationsClient } from "../src/api/client.ts";

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

test("Continuum route still returns a valid datum.continuum-view/1 global projection", async () => {
  await withServer(async (baseUrl) => {
    const client = new OperationsClient(baseUrl);
    const data = await client.continuum(new AbortController().signal);
    assert.equal(data.schema, "datum.continuum-view/1");
    assert.equal(data.nodes.length, 3);
  });
});

test("Continuum route still returns a valid project-scoped projection with overlay", async () => {
  await withServer(async (baseUrl) => {
    const client = new OperationsClient(baseUrl);
    const data = await client.projectContinuum("project-a", new AbortController().signal);
    assert.equal(data.schema, "datum.continuum-view/1");
    assert.deepEqual(data.project_overlay, { project_id: "project-a", node_ids_in_use: ["node-a", "node-b"] });
  });
});

test("Overview route no longer returns fixture_not_found and passes the real Console contract validator", async () => {
  await withServer(async (baseUrl) => {
    const client = new OperationsClient(baseUrl);
    const data = await client.overview(new AbortController().signal);
    assert.equal(data.generated_at_utc, "2026-10-04T12:00:00Z");
    assert.equal(data.known_projects.page.items.length, 0, "honest empty state, not fabricated projects");
  });
});

test("Projects route reuses the same overview endpoint and also resolves to a valid, non-degraded projection", async () => {
  await withServer(async (baseUrl) => {
    const client = new OperationsClient(baseUrl);
    // console/src/main.ts renders the Projects page from the same client.overview() response.
    const data = await client.overview(new AbortController().signal);
    const degraded = Object.values(data).some(
      (v) => v && typeof v === "object" && "availability" in v && ["unavailable", "corrupt", "indeterminate"].includes(String(v.availability)),
    ) || data.source_issues.length > 0;
    assert.equal(degraded, false, "a fully not-configured/empty fixture must read as Connected, never Degraded");
  });
});

test("Catalog services list route returns a valid empty CatalogPage", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/catalog/services`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.deepEqual(body, { items: [], has_more: false, limit: 100 });
  });
});

test("Catalog artifacts route returns a valid empty pinned-artifact page", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/catalog/artifacts`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.deepEqual(body, { source: "pinned_operational_artifact_references", limit: 100, has_more: false, items: [] });
  });
});

test("there is no separate shell health/connectivity endpoint in the real contract -- connectivity is derived from each page's own request, which the fixture above now satisfies", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/health`);
    assert.equal(response.status, 404, "no such endpoint exists in the real Console client contract; one must not be invented");
  });
});

test("an unrelated unknown route still fails explicitly, never silently returns 200", async () => {
  await withServer(async (baseUrl) => {
    const response = await fetch(`${baseUrl}/api/v1/operations/projects/some-project`);
    assert.equal(response.status, 404);
    const body = await response.json();
    assert.equal(body.error.code, "fixture_not_found");
  });
});

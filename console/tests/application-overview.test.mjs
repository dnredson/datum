// Phase 141B -- Application Operational Context. `pages/application-overview.ts`
// is a DOM-building view function and is not unit-tested directly here for
// the same documented reason every other Console page uses: no DOM
// implementation is available in this environment. Its testable surface
// is: route parse/build, the API client's own request construction (mocked
// `fetch`), the shared model (covered in application-overview-model.test.mjs),
// and source-text-scan regressions proving the governance boundaries this
// phase must never cross.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { parseRoute, applicationOverviewPath, applicationDeployPath, projectPath } from "../src/app/router.ts";
import { ApplicationOverviewClient, ApplicationOverviewError } from "../src/api/application-overview.ts";
import { applicationOverviewFixture } from "./application-overview-fixture.mjs";

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

// ---- Route ----

test("application overview route parses and rebuilds, distinct from deploy", () => {
  const path = applicationOverviewPath("project-a", "app-1");
  assert.deepEqual(parseRoute(path), { kind: "project_application_overview", projectId: "project-a", applicationId: "app-1" });
  assert.notEqual(path, applicationDeployPath("project-a", "app-1"));
});
test("application overview route never collides with deploy, continuum, placement, or history routes", () => {
  assert.notEqual(parseRoute(applicationDeployPath("project-a", "app-1")).kind, "project_application_overview");
  assert.notEqual(parseRoute(`${projectPath("project-a")}/continuum`).kind, "project_application_overview");
  assert.notEqual(parseRoute(`${projectPath("project-a")}/placement`).kind, "project_application_overview");
  assert.notEqual(parseRoute(`${projectPath("project-a")}/history`).kind, "project_application_overview");
});
test("application overview route requires exactly /projects/:id/applications/:id -- deeper/malformed paths are Not Found", () => {
  assert.equal(parseRoute("/projects/a/applications/app-1/extra").kind, "not_found");
  assert.equal(parseRoute("/projects/a/applications//").kind, "not_found");
});

// ---- Client request construction ----

test("ApplicationOverviewClient.get requests the exact project/application-scoped path with the project header", async () => {
  const prior = globalThis.fetch;
  const requests = [];
  try {
    globalThis.fetch = async (url, init) => {
      requests.push({ url, init });
      return new Response(JSON.stringify(applicationOverviewFixture({ project_id: "proj/with space", application_id: "app one" })));
    };
    await new ApplicationOverviewClient("proj/with space").get("app one");
    assert.equal(requests.length, 1);
    const url = new URL(requests[0].url, "http://fixture");
    assert.equal(url.pathname, "/api/v1/operations/projects/proj%2Fwith%20space/applications/app%20one");
    assert.equal(requests[0].init.headers["X-Datum-Project"], "proj/with space");
    assert.equal(requests[0].init.method, "GET");
  } finally {
    globalThis.fetch = prior;
  }
});

test("ApplicationOverviewClient surfaces the backend's own error message rather than a generic failure", async () => {
  const prior = globalThis.fetch;
  try {
    globalThis.fetch = async () => new Response(JSON.stringify({ error: { code: "invalid_scope", message: "Invalid project identifier." } }), { status: 400 });
    await assert.rejects(
      new ApplicationOverviewClient("project-a").get("app-1"),
      (e) => e instanceof ApplicationOverviewError && e.message === "Invalid project identifier." && e.status === 400,
    );
  } finally {
    globalThis.fetch = prior;
  }
});

test("ApplicationOverviewClient rejects a response with an invented readiness value -- never permissively accepted", async () => {
  const prior = globalThis.fetch;
  try {
    const bad = applicationOverviewFixture();
    bad.readiness.value.readiness = "mostly_ready";
    globalThis.fetch = async () => new Response(JSON.stringify(bad));
    await assert.rejects(new ApplicationOverviewClient("project-a").get("app-1"), (e) => e instanceof ApplicationOverviewError && e.code === "invalid_response");
  } finally {
    globalThis.fetch = prior;
  }
});

// ---- Structural regressions on pages/application-overview.ts ----

test("application-overview.ts never references mutation/remediation/AI primitives", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  for (const forbidden of [
    "createBatch(",
    "authorize(",
    "acceptProposal",
    "createProposal",
    "createReconciliation",
    "reconcile(",
    "restart(",
    "OpenAI",
    "LLM",
    "GNN",
    "MCP",
    "child_process",
    "exec(",
    "spawn(",
  ]) {
    assert.ok(!code.includes(forbidden), `application-overview.ts must never reference '${forbidden}'`);
  }
});

test("application-overview.ts only ever calls client.get -- no mutating method exists to call", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  const calls = [...code.matchAll(/\bclient\.(\w+)\(/gu)].map((m) => m[1]);
  assert.ok(calls.length > 0);
  for (const call of calls) assert.equal(call, "get", `application-overview.ts must only ever call client.get(), saw .${call}(`);
  const clientCode = await sourceWithoutComments("../src/api/application-overview.ts");
  const methodNames = [...clientCode.matchAll(/\n\s+async (\w+)\(/gu)].map((m) => m[1]);
  assert.deepEqual(methodNames, ["get"], "ApplicationOverviewClient must expose exactly one method: get");
});

test("drill-down links use the existing router path builders, never a hand-built query-string history link (Part AB)", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  assert.ok(code.includes("applicationDeployPath("));
  assert.ok(code.includes("projectPath("));
  assert.ok(!/history\?.*application/iu.test(code), "must never fabricate an unsupported History query parameter (Phase 140 has no URL filter state)");
});

test("readiness is rendered as canonical Ready/Not Ready text+tone, never a third aggregate state, and convergence stays a separate section", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  assert.ok(code.includes("READINESS_LABEL[readiness.readiness]"));
  assert.ok(!/Healthy|Warning|Critical|Green|Yellow|Red/u.test(code.replace(/HEALTH_LABEL|HEALTH_TONE/gu, "")), "no business-health-styled aggregate labels");
  assert.ok(code.includes('sectionBox("Readiness"'));
  assert.ok(code.includes('sectionBox("Convergence"'));
});

test("recent monitoring evidence section never embeds raw facts or a historical convergence verdict", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  const start = code.indexOf('sectionBox("Recent monitoring evidence"');
  const body = code.slice(start, code.indexOf(");", start) + 2);
  assert.ok(!/\bfacts\b/iu.test(body));
  assert.ok(!/was_converged|historical.*convergence/iu.test(body));
});

test("stale/out-of-order responses cannot overwrite a newer load -- race-safe sequence token guard present on both branches", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  const start = code.indexOf("async function load(");
  const body = code.slice(start, code.indexOf("\n  }", start));
  assert.ok(body.includes("const token = ++sequence;"));
  assert.ok((body.match(/token !== sequence/g) ?? []).length >= 2);
});

test("current-vs-history wording stays distinct -- this page never claims live/current status for recorded evidence", async () => {
  const code = await sourceWithoutComments("../src/pages/application-overview.ts");
  assert.ok(/non-atomic/iu.test(code));
  assert.ok(!/current status/iu.test(code));
});

// Independent review Audits 74-77 -- critical isolation from every legacy
// observation/context pathway this phase explicitly must not depend on.
test("application-overview.ts and its client never reference the legacy analysis-context, Firebase observation, or legacy events pathways", async () => {
  for (const path of ["../src/pages/application-overview.ts", "../src/api/application-overview.ts", "../src/pages/application-overview-model.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of [
      "analysis-context",
      "analysis_context",
      "active_ai_context",
      "ai_gnn_data_contract",
      "get_latest_snapshot",
      "/events/latest",
      "/events/materialized",
      "build_global_events",
    ]) {
      assert.ok(!code.includes(forbidden), `must never reference legacy pathway '${forbidden}'`);
    }
  }
});

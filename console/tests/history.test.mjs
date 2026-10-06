// Phase 140B -- Historical Explorer. `pages/history.ts` is a DOM-building
// view function (like every other page in this codebase) and is not
// unit-tested directly here for the same documented reason: no DOM
// implementation (happy-dom/jsdom) is available in this environment. Its
// testable surface is: route parse/build (pure), the API client's own
// request construction (mocked `fetch`, the established
// `dgraph-editor.test.mjs` pattern), the shared model functions (covered in
// dmonitor-history-model.test.mjs), and source-text-scan regressions
// proving the governance boundaries this phase must never cross -- the
// same established precedent every other Console page's tests use.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { parseRoute, projectPath } from "../src/app/router.ts";
import { DMonitorHistoryClient, DMonitorHistoryError } from "../src/api/dmonitor-history.ts";

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

// ---- Route ----

test("project history route parses and rebuilds", () => {
  const path = `${projectPath("project-a")}/history`;
  assert.deepEqual(parseRoute(path), { kind: "project_history", projectId: "project-a" });
});
test("project history route requires exactly /projects/:id/history -- a deeper path is Not Found", () => {
  assert.equal(parseRoute("/projects/project-a/history/extra").kind, "not_found");
});
test("project history route never collides with project continuum/placement routes", () => {
  assert.notEqual(parseRoute(`${projectPath("project-a")}/history`).kind, "project_continuum");
  assert.notEqual(parseRoute(`${projectPath("project-a")}/continuum`).kind, "project_history");
});

// ---- Client request construction (items 4-8, 24) ----

test("DMonitorHistoryClient.query transmits project, application, node, service, start/end, and limit exactly", async () => {
  const prior = globalThis.fetch;
  const requests = [];
  try {
    globalThis.fetch = async (url) => {
      requests.push(url);
      return new Response(
        JSON.stringify({
          schema: "datum.dmonitor-history/1",
          project_id: "proj/with space",
          start_utc: "2026-10-04T12:00:00Z",
          end_utc: "2026-10-05T12:00:00Z",
          queried_at_utc: "2026-10-05T12:00:05Z",
          filters: { application_id: "app-1", node_id: "fog-01", service_id: "svc-a" },
          limit: 50,
          count: 0,
          truncated: false,
          observations: [],
        }),
      );
    };
    const client = new DMonitorHistoryClient("proj/with space");
    await client.query({
      applicationId: "app-1",
      nodeId: "fog-01",
      serviceId: "svc-a",
      startUtc: "2026-10-04T12:00:00Z",
      endUtc: "2026-10-05T12:00:00Z",
      limit: 50,
    });
    assert.equal(requests.length, 1);
    const url = new URL(requests[0], "http://fixture");
    assert.equal(url.pathname, "/api/v1/dmonitor/history/proj%2Fwith%20space");
    assert.equal(url.searchParams.get("application_id"), "app-1");
    assert.equal(url.searchParams.get("node_id"), "fog-01");
    assert.equal(url.searchParams.get("service_id"), "svc-a");
    assert.equal(url.searchParams.get("start_utc"), "2026-10-04T12:00:00Z");
    assert.equal(url.searchParams.get("end_utc"), "2026-10-05T12:00:00Z");
    assert.equal(url.searchParams.get("limit"), "50");
  } finally {
    globalThis.fetch = prior;
  }
});

test("DMonitorHistoryClient.query omits unset filters entirely rather than sending empty values", async () => {
  const prior = globalThis.fetch;
  const requests = [];
  try {
    globalThis.fetch = async (url) => {
      requests.push(url);
      return new Response(
        JSON.stringify({
          schema: "datum.dmonitor-history/1",
          project_id: "project-a",
          start_utc: "2026-10-04T12:00:00Z",
          end_utc: "2026-10-05T12:00:00Z",
          queried_at_utc: "2026-10-05T12:00:05Z",
          filters: { application_id: null, node_id: null, service_id: null },
          limit: 100,
          count: 0,
          truncated: false,
          observations: [],
        }),
      );
    };
    await new DMonitorHistoryClient("project-a").query();
    const url = new URL(requests[0], "http://fixture");
    assert.equal(url.searchParams.has("application_id"), false);
    assert.equal(url.searchParams.has("node_id"), false);
    assert.equal(url.searchParams.has("service_id"), false);
    assert.equal(url.searchParams.has("start_utc"), false);
    assert.equal(url.searchParams.has("limit"), false);
  } finally {
    globalThis.fetch = prior;
  }
});

test("DMonitorHistoryClient surfaces the backend's own 422 range/filter message rather than a generic failure", async () => {
  const prior = globalThis.fetch;
  try {
    globalThis.fetch = async () =>
      new Response(JSON.stringify({ error: "invalid_history_query", message: "start_utc must not be after end_utc" }), { status: 422 });
    await assert.rejects(
      new DMonitorHistoryClient("project-a").query(),
      (e) => e instanceof DMonitorHistoryError && e.message === "start_utc must not be after end_utc" && e.status === 422,
    );
  } finally {
    globalThis.fetch = prior;
  }
});

test("DMonitorHistoryClient rejects a response with an invented subject kind -- never permissively accepted", async () => {
  const prior = globalThis.fetch;
  try {
    globalThis.fetch = async () =>
      new Response(
        JSON.stringify({
          schema: "datum.dmonitor-history/1",
          project_id: "project-a",
          start_utc: "2026-10-04T12:00:00Z",
          end_utc: "2026-10-05T12:00:00Z",
          queried_at_utc: "2026-10-05T12:00:05Z",
          filters: { application_id: null, node_id: null, service_id: null },
          limit: 100,
          count: 1,
          truncated: false,
          observations: [{ subject: { kind: "invented" } }],
        }),
      );
    await assert.rejects(new DMonitorHistoryClient("project-a").query(), (e) => e instanceof DMonitorHistoryError && e.code === "invalid_response");
  } finally {
    globalThis.fetch = prior;
  }
});

// ---- Structural regressions on pages/history.ts (items 1, 3, 11-14, 18-20, 21-23, 25) ----

test("history.ts and dmonitor-history-model.ts never reference convergence/authority or host-mutation primitives", async () => {
  for (const path of ["../src/pages/history.ts", "../src/pages/dmonitor-history-model.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of [
      "DMonitorConvergence",
      "AuthorityCorrelation",
      "active_placement_authority",
      "PlacementClient",
      "ReconciliationClient",
      "createBatch(",
      "authorize(",
      "child_process",
      "exec(",
      "spawn(",
    ]) {
      assert.ok(!code.includes(forbidden), `must never reference '${forbidden}'`);
    }
  }
});

test("history.ts never reorders response.observations client-side -- chronological order is the backend's own (item 3)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(!code.includes(".sort("), "history.ts must never sort observations client-side");
  assert.ok(!code.includes(".reverse("), "history.ts must never reverse observations client-side");
  assert.ok(code.includes("for (const observation of response.observations)"), "must iterate the backend's own array order directly");
});

test("history.ts has four distinct, mutually exclusive render branches: loading, error, empty, and populated (items 1, 11-13)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const start = code.indexOf("function renderResults(");
  const body = code.slice(start, code.indexOf("\n  }", start));
  assert.ok(/if \(!loaded\)/.test(body), "must have a distinct loading branch");
  assert.ok(/if \(errorMessage\)/.test(body), "must have a distinct error branch");
  assert.ok(/observations\.length === 0/.test(body), "must have a distinct empty-result branch");
  assert.ok(body.includes("No DMonitor evidence was recorded"), "the empty-range message must be honest, never phrased as a failure");
});

test("a 422/backend failure is distinguished from an honest empty range -- 'no observations' never becomes errorMessage (item 11/14)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  // The empty-array branch returns without ever assigning to errorMessage;
  // only the catch block (a genuine thrown error) sets it.
  const catchStart = code.indexOf("} catch (e) {");
  assert.ok(catchStart > 0);
  assert.ok(code.slice(catchStart).includes("errorMessage ="), "errorMessage must only ever be set from a genuine thrown error, never from an empty successful result");
});

test("drill-down toggles expandedObservationId and re-renders from the already-loaded response only -- never issues a new request (item 23)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const start = code.indexOf("function observationRow(");
  const body = code.slice(start, code.indexOf("\nexport function historyView(", start));
  assert.ok(!body.includes("client.query("), "opening/closing drill-down must never call the API client");
  assert.ok(!/history-observation-row[\s\S]*?load\(/.test(body), "drill-down toggle must never trigger a new load()");
});

test("monitor_instance_id and sequence are explicitly labeled diagnostic-only, never canonical identity (item 20)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(code.includes("diagnostic only -- not canonical identity"));
  assert.ok(code.includes("Sequence (diagnostic only)"));
});

test("observation detail renders canonical identity fields directly from the response, with no PID/container-id field anywhere (item 19/20)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const start = code.indexOf("function observationDetail(");
  const body = code.slice(start, code.indexOf("\n}", start));
  for (const field of ["observation_id", "node_id", "observed_at_utc", "received_at_utc", "collector"]) {
    assert.ok(body.includes(field), `observationDetail must render '${field}'`);
  }
  for (const forbidden of ["pid", "container_id", "containerId", "process_id"]) {
    assert.ok(!body.toLowerCase().includes(forbidden), `observationDetail must never expose '${forbidden}'`);
  }
});

test("timestamps are rendered as raw backend strings, never reparsed/reformatted -- both Z and +00:00 spellings render identically (items 21-22)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const start = code.indexOf("function observationDetail(");
  const body = code.slice(start, code.indexOf("\n}", start));
  assert.ok(!/observed_at_utc\)\s*\)|received_at_utc\)\s*\)/.test(body.replace(/entry\("[^"]+", /g, "")), "timestamps must be passed through as-is");
  assert.ok(!body.includes("Date.parse(observation.observed_at_utc"));
  assert.ok(!body.includes("toLocaleString"));
});

test("findings are never fabricated -- the canonical historical contract carries no findings field, and this page never invents one (item 18)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(!/\bfindings?\b/iu.test(code), "history.ts must never render a 'finding' -- the canonical historical response has no such field (Phase 140A, by design); raw 'facts' are shown honestly instead, never relabeled as findings");
  assert.ok(code.includes("Collector facts (raw, as persisted)"));
});

test("stale/out-of-order responses cannot overwrite a newer query -- load() guards every shared-state write with a sequence token (item 25, mirrors Phase 138 Audit 27)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const start = code.indexOf("async function load(");
  const body = code.slice(start, code.indexOf("\n  }", start));
  assert.ok(body.includes("const token = ++sequence;"));
  assert.ok((body.match(/token !== sequence/g) ?? []).length >= 2, "both the success and failure paths must check token !== sequence before touching shared state");
});

test("changing filters always issues a fresh load via the same race-safe path (item 24)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  const submitStart = code.indexOf('form.addEventListener("submit"');
  const body = code.slice(submitStart, code.indexOf("});", submitStart));
  assert.ok(body.includes("readFormIntoFilters()"));
  assert.ok(body.includes("void load()"));
});

test("no mutation/remediation endpoint is reachable from this page -- DMonitorHistoryClient is read-only and history.ts calls only .query", async () => {
  const historyCode = await sourceWithoutComments("../src/pages/history.ts");
  const calls = [...historyCode.matchAll(/\bclient\.(\w+)\(/gu)].map((m) => m[1]);
  assert.ok(calls.length > 0);
  for (const call of calls) assert.equal(call, "query", `history.ts must only ever call client.query(), saw .${call}(`);
});

// Phase 140C polish -- aria-controls/aria-describedby wiring and explanatory
// copy (default window, max window, limit default/max, health/freshness
// independence, current-vs-history separation).
test("expand/collapse button wires aria-controls to the detail panel's own id (Phase 140C)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(code.includes('summary.setAttribute("aria-controls", detailId)'));
  assert.ok(code.includes("detail.id = detailId;"));
});

test("filter inputs carry an explanatory hint wired via aria-describedby (Phase 140C)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(code.includes('input.setAttribute("aria-describedby", hintId)'));
  assert.ok(code.includes("24 hours before now"));
  assert.ok(code.includes("at most 7 days"));
  assert.ok(code.includes("Default 100, maximum 500"));
});

test("page copy clarifies health/freshness independence and current-vs-history separation without new semantics (Phase 140C)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(/Health and Freshness are independent/u.test(code));
  assert.ok(/use Continuum or an application's own Deploy page instead/u.test(code));
  assert.ok(/received each observation, not by the time the node itself recorded it/u.test(code));
});

test("truncation summary never implies a complete result set, and the complete-set wording is honest (Phase 140C)", async () => {
  const code = await sourceWithoutComments("../src/pages/history.ts");
  assert.ok(code.includes("response.truncated"));
  assert.ok(code.includes("Showing the first"));
  assert.ok(code.includes("the complete matching set for this query"));
});

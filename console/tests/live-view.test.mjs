import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  correlateAlertsByNode,
  correlateEventsByNode,
  resolvePins,
  computeGlobalCounts,
  filterAlerts,
  filterEvents,
} from "../src/live/live-view.ts";
import { alertFixture, eventFixture } from "./wallboard-fixture.mjs";

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

function node(id, overrides = {}) {
  return {
    node_id: id,
    structure: { platform: { os: "linux", architecture: "amd64" }, capacity: { cpu_cores: 4, memory_bytes: 8_589_934_592 }, dnode_abi_versions: [] },
    accepted_placements: [],
    placements_truncated: false,
    observations: [],
    observations_truncated: false,
    ...overrides,
  };
}

// ---- correlation: only subject variants that actually carry node_id ----

test("correlateAlertsByNode indexes an alert whose subject carries node_id", () => {
  const alert = alertFixture("open", "a1"); // subject.node_id === "fog-01" (wallboard-fixture.mjs)
  const index = correlateAlertsByNode([alert]);
  assert.deepEqual(index.get("fog-01"), [alert]);
});

test("correlateAlertsByNode never fabricates a node association for a subject with no node_id", () => {
  const alert = alertFixture("open", "a1");
  const { node_id, ...subjectWithoutNode } = alert.subject;
  const noNodeAlert = { ...alert, subject: subjectWithoutNode };
  const index = correlateAlertsByNode([noNodeAlert]);
  assert.equal(index.size, 0);
});

test("correlateAlertsByNode preserves the backend's own relative order within a node's bucket", () => {
  const a = alertFixture("open", "a1");
  const b = alertFixture("acknowledged", "a2");
  const index = correlateAlertsByNode([a, b]);
  assert.deepEqual(index.get("fog-01"), [a, b]);
});

test("correlateEventsByNode mirrors the same node_id-only correlation rule", () => {
  const event = eventFixture(0); // subject.node_id === "fog-01"
  const index = correlateEventsByNode([event]);
  assert.deepEqual(index.get("fog-01"), [event]);
});

// ---- resolving pinned nodes against the live continuum ----

test("resolvePins preserves pin order and keeps a missing node as undefined, never dropping it", () => {
  const present = node("node-a");
  const resolved = resolvePins(["node-a", "node-missing"], [present]);
  assert.deepEqual(resolved, [
    { nodeId: "node-a", node: present },
    { nodeId: "node-missing", node: undefined },
  ]);
});

test("resolvePins on an empty pin list returns an empty array, not an error", () => {
  assert.deepEqual(resolvePins([], [node("node-a")]), []);
});

// ---- global counts strip: never a fabricated total, honest undefined on unavailable sources ----

test("computeGlobalCounts reports undefined (not zero) for an unavailable source", () => {
  const counts = computeGlobalCounts(undefined, undefined);
  assert.equal(counts.knownProjects, undefined);
  assert.equal(counts.knownNodes, undefined);
  assert.equal(counts.activeAlerts, undefined);
});

test("computeGlobalCounts counts known nodes directly from the continuum node list", () => {
  const continuum = { nodes: [node("a"), node("b")] };
  const counts = computeGlobalCounts(undefined, continuum);
  assert.equal(counts.knownNodes, 2);
});

// Review P1-3 regression guard: this count must NEVER be silently capped
// at the Wallboard preview panel's own 8-item bound.
test("computeGlobalCounts.activeAlerts reports the real count, not the Wallboard's 8-item preview cap", () => {
  const items = Array.from({ length: 12 }, (_, i) => alertFixture("open", `a${i}`));
  const overview = {
    known_projects: { page: { items: [], limit: 100, has_more: false }, coverage: { completeness: "complete", source_coverage: {} } },
    alerts: { availability: "available", value: { attention_ordered: { items, limit: 100, has_more: false } } },
    recent_event_count: { availability: "not_configured" },
    source_issues: [],
  };
  const counts = computeGlobalCounts(overview, undefined);
  assert.equal(counts.activeAlerts, 12);
});

// Review P1-3 structural regression: the Wallboard-specific, slice-to-8
// `activeAttention` must never be reintroduced into Live's rendering --
// only the unsliced `isActiveAttention` predicate belongs here.
test("live-view.ts never reuses wallboard.ts's sliced activeAttention helper", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(!code.includes("activeAttention("), "live-view.ts must use isActiveAttention, never the sliced activeAttention");
  assert.ok(code.includes("isActiveAttention"));
});

// Review P1-2 structural regression: rebuilding the node filter select must
// preserve whatever the operator currently has selected.
test("the node filter select preserves its current value across a rebuild", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  const start = code.indexOf("function renderNodeFilterOptions(");
  const body = code.slice(start, code.indexOf("\n  }", start));
  assert.ok(body.includes("previousValue"));
  assert.ok(/n\.node_id === previousValue/.test(body));
});

// Review P1-7 structural regression: a failure in one data source must
// never discard the other source's freshly-successful result.
test("poll() uses allSettled, never Promise.all, across the two data sources", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(code.includes("Promise.allSettled("));
  assert.ok(!code.includes("Promise.all("));
});

// Review P1-5 structural regression: cross-bundle Console links must go
// through the configurable base, never a bare same-origin-relative path.
test("cross-bundle Console links are built through consoleBaseUrl(), not a bare relative path", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(!/href\s*=\s*["']\/continuum["']/.test(code));
  assert.ok(!/href\s*=\s*applicationOverviewPath/.test(code), "must prefix with consoleBaseUrl(), not assign the bare path directly");
  assert.ok((code.match(/consoleBaseUrl\(\)/g) ?? []).length >= 3);
});

// Review P2-B structural regression: only `DATUM_CONSOLE_URL` may be
// exposed to client code, never the whole `DATUM_*` env family.
test("vite.config.ts injects only DATUM_CONSOLE_URL via define, never a broad DATUM_* envPrefix", async () => {
  const code = await sourceWithoutComments("../vite.config.ts");
  assert.ok(code.includes('"import.meta.env.DATUM_CONSOLE_URL"'), "must define exactly the DATUM_CONSOLE_URL key");
  assert.ok(!/envPrefix.*DATUM_/s.test(code), "must never add DATUM_ to envPrefix (that would expose the whole DATUM_* family)");
});

// consoleBaseUrl() must reference the literal `import.meta.env.DATUM_CONSOLE_URL`
// expression (no cast wrapping the object) so Vite's `define` replacement
// actually matches it.
test("consoleBaseUrl() reads the literal import.meta.env.DATUM_CONSOLE_URL expression", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(code.includes("import.meta.env.DATUM_CONSOLE_URL"));
  assert.ok(!code.includes("import.meta.env as Record"), "must not cast the whole import.meta.env object, which would break the define replacement");
});

// Review P1-4 structural regression: a Live profile must be renameable.
test("live-view.ts provides a profile rename control wired to persistActive()", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(code.includes("nameInput"));
  assert.ok(code.includes("commitName"));
  assert.ok(code.includes("persistActive()"));
});

// ---- filters never reorder, only remove non-matching entries ----

test("filterAlerts by severity preserves relative order of the remaining items", () => {
  const critical1 = { ...alertFixture("open", "a1"), severity: "critical" };
  const warning = { ...alertFixture("open", "a2"), severity: "warning" };
  const critical2 = { ...alertFixture("open", "a3"), severity: "critical" };
  const filtered = filterAlerts([critical1, warning, critical2], { severities: ["critical"], projectIds: [], nodeId: "" });
  assert.deepEqual(filtered, [critical1, critical2]);
});

test("filterAlerts by project id excludes a DServer-global alert (project_id null)", () => {
  const scoped = alertFixture("open", "a1");
  const global = { ...scoped, project_id: null };
  const filtered = filterAlerts([scoped, global], { severities: [], projectIds: ["alpha"], nodeId: "" });
  assert.deepEqual(filtered, [scoped]);
});

test("filterAlerts by node id uses the exact subject.node_id match, no partial match", () => {
  const onFog01 = alertFixture("open", "a1");
  const filtered = filterAlerts([onFog01], { severities: [], projectIds: [], nodeId: "fog-0" });
  assert.deepEqual(filtered, []);
});

test("empty filters pass every alert/event through unchanged", () => {
  const alerts = [alertFixture("open", "a1"), alertFixture("acknowledged", "a2")];
  assert.deepEqual(filterAlerts(alerts, { severities: [], projectIds: [], nodeId: "" }), alerts);
  const events = [eventFixture(0), eventFixture(1)];
  assert.deepEqual(filterEvents(events, { projectIds: [], nodeId: "" }), events);
});

// ---- structural regressions: Live is strictly read-only and AI-free ----

test("live/ source never references mutation, remediation, placement authority, or AI primitives", async () => {
  for (const path of ["../src/live/live-view.ts", "../src/live/live-profile.ts", "../src/live/main.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of [
      "createBatch(",
      "authorize(",
      "acceptProposal",
      "createProposal",
      "createReconciliation",
      "reconcile(",
      "restart(",
      "declare_dnode",
      "OpenAI",
      "LLM",
      "GNN",
      "MCP",
      "child_process",
      "exec(",
      "spawn(",
      "analysis-context",
      "analysis_context",
      "active_ai_context",
      "ai_gnn_data_contract",
    ]) {
      assert.ok(!code.includes(forbidden), `${path} must never reference '${forbidden}'`);
    }
  }
});

test("live-view.ts never issues a non-GET HTTP method", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  for (const method of ["method: \"POST\"", "method: \"PUT\"", "method: \"PATCH\"", "method: \"DELETE\"", "fetch("]) {
    assert.ok(!code.includes(method), `live-view.ts must never call '${method}' directly -- all reads go through the injected LiveDataSource`);
  }
});

test("live-view.ts never introduces a new node-health threshold or score", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  // Only DMonitor's own canonical labels/tones may be referenced; a raw
  // percentage threshold comparison would indicate a fabricated health rule.
  assert.ok(!/cpu[._-]?(percent|usage)\s*[<>]=?\s*\d/iu.test(code), "no CPU threshold comparison may exist in Live");
  assert.ok(!/unhealthy\s*=/iu.test(code), "no derived 'unhealthy =' assignment may exist in Live");
});

test("live-view.ts exposes exactly the LiveDataSource shape it requires (overview + continuum), no write method", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  const start = code.indexOf("export interface LiveDataSource {");
  const end = code.indexOf("}", start);
  const body = code.slice(start, end);
  assert.ok(body.includes("overview("));
  assert.ok(body.includes("continuum("));
  assert.ok(!/\b(post|put|patch|delete|create|accept|authorize)\w*\(/iu.test(body));
});

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
  computeNodeCardState,
} from "../src/live/live-view.ts";
import { isActiveAttention } from "../src/pages/wallboard.ts";
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

function evidence(health, freshness = "fresh", overrides = {}) {
  return {
    monitor_instance_id: "m1",
    freshness,
    health,
    authority_correlation: "exact_match",
    observed_at_utc: "2026-01-01T00:00:00Z",
    received_at_utc: "2026-01-01T00:00:00Z",
    age_seconds: 1,
    ...overrides,
  };
}
function observation(convergence, admittedEvidence = []) {
  return {
    project_id: "alpha",
    application_id: "app",
    service_id: "svc",
    placement_convergence: convergence,
    convergence_findings: [],
    admitted_evidence: admittedEvidence,
    admitted_evidence_truncated: false,
    excluded_evidence: [],
    excluded_evidence_truncated: false,
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

// ---- Glanceability follow-up: computeNodeCardState pure tone/state derivation ----

test("an active critical alert produces danger tone", () => {
  const state = computeNodeCardState(node("n1"), [alertFixture("open", "a1")]); // critical severity (wallboard-fixture.mjs)
  assert.equal(state.tone, "danger");
  assert.equal(state.label, "Attention required");
});

test("an active warning-severity alert produces warning tone", () => {
  const alert = { ...alertFixture("open", "a1"), severity: "warning" };
  const state = computeNodeCardState(node("n1"), [alert]);
  assert.equal(state.tone, "warning");
  assert.equal(state.label, "Warning");
});

test("no explicit negative verdict but an observation exists -> informational tone, never 'healthy'", () => {
  const n = node("n1", { observations: [observation("converged", [evidence("healthy", "fresh")])] });
  const state = computeNodeCardState(n, []);
  assert.equal(state.tone, "info");
  assert.equal(state.label, "Observed");
  assert.ok(!/healthy/iu.test(state.label));
});

test("no evidence at all (and no alert) -> neutral tone", () => {
  const state = computeNodeCardState(node("n1"), []);
  assert.equal(state.tone, "neutral");
  assert.equal(state.label, "No current evidence");
});

test("a node missing from the continuum (undefined) is always neutral", () => {
  const state = computeNodeCardState(undefined, []);
  assert.equal(state.tone, "neutral");
});

test("CPU percentage alone never changes tone -- computeNodeCardState never reads cpu/memory at all", () => {
  const n = node("n1", { resource_observation: { resource_id: "r", observed_at_utc: "t", last_seen_utc: "t", age_seconds: 1, cpu: { logical_cores: 1, usage_percent: 99.9 }, memory: { total_bytes: 100, used_bytes: 1, available_bytes: 99 }, swap: { total_bytes: 0, used_bytes: 0, available_bytes: 0 } } });
  const state = computeNodeCardState(n, []);
  // Host telemetry present but no alert/canonical negative verdict -- a
  // 99.9% CPU reading must never, by itself, push this past "informational".
  assert.equal(state.tone, "info");
});

test("memory percentage alone never changes tone -- same guarantee as CPU", () => {
  const n = node("n1", { resource_observation: { resource_id: "r", observed_at_utc: "t", last_seen_utc: "t", age_seconds: 1, cpu: { logical_cores: 1 }, memory: { total_bytes: 100, used_bytes: 99, available_bytes: 1 }, swap: { total_bytes: 0, used_bytes: 0, available_bytes: 0 } } });
  const state = computeNodeCardState(n, []);
  assert.equal(state.tone, "info");
});

test("absence of alerts alone never produces a positive health claim", () => {
  // No alerts, no observations, no resource_observation at all.
  const state = computeNodeCardState(node("n1"), []);
  assert.equal(state.tone, "neutral");
  assert.ok(!/healthy|ok|good/iu.test(state.label));
});

test("canonical unhealthy DMonitor evidence produces danger tone without redefining the enum", () => {
  const n = node("n1", { observations: [observation("converged", [evidence("unhealthy")])] });
  const state = computeNodeCardState(n, []);
  assert.equal(state.tone, "danger");
});

test("canonical diverged placement_convergence produces danger tone", () => {
  const n = node("n1", { observations: [observation("diverged")] });
  const state = computeNodeCardState(n, []);
  assert.equal(state.tone, "danger");
});

test("canonical degraded health or stale freshness produces warning tone (no active alert)", () => {
  const degraded = node("n1", { observations: [observation("converged", [evidence("degraded")])] });
  assert.equal(computeNodeCardState(degraded, []).tone, "warning");
  const stale = node("n2", { observations: [observation("converged", [evidence("healthy", "stale")])] });
  assert.equal(computeNodeCardState(stale, []).tone, "warning");
});

test("a resolved-only alert does not by itself produce a danger/warning tone (filtered through the same isActiveAttention gate the real caller uses)", () => {
  const resolved = alertFixture("resolved", "a1");
  // computeNodeCardState trusts its `activeAlerts` input is already
  // active-only (the same contract every caller in live-view.ts honors
  // via `activeAlertsOnly`/`isActiveAttention`) -- so the test replicates
  // that exact filtering step rather than feeding it a raw, unfiltered alert.
  const state = computeNodeCardState(node("n1"), [resolved].filter(isActiveAttention));
  assert.equal(state.tone, "neutral");
});

// ---- wording: Continuum node observations are placement-scoped ----

test("the empty-DMonitor-evidence wording is accurate: placement-scoped, not global absence", async () => {
  const code = await sourceWithoutComments("../src/live/live-view.ts");
  assert.ok(code.includes("No placement-scoped DMonitor evidence."));
  assert.ok(!code.includes("No current DMonitor evidence."), "must not reintroduce the overly broad wording");
});

// ---- structural: responsive / reduced-motion CSS ----

test("live.css respects prefers-reduced-motion and never adds a frontend stale/offline timeout", async () => {
  const css = await readFile(new URL("../src/live/live.css", import.meta.url), "utf8");
  assert.ok(css.includes("prefers-reduced-motion"));
  assert.ok(!/age_seconds\s*[<>]/u.test(css));
  assert.ok(!/setTimeout.*stale|offline/iu.test(css));
});

test("live.css node grid never exceeds its container width (auto-fit + minmax capped at 100%)", async () => {
  const css = await readFile(new URL("../src/live/live.css", import.meta.url), "utf8");
  assert.ok(css.includes(".live-node-grid"));
  assert.ok(/minmax\(min\([^,]+,\s*100%\)/u.test(css), "grid track minimum must be capped at 100% of the container to avoid horizontal overflow on narrow viewports");
});

// RPi visual-smoke regression: the shared terminal-metrics component's own
// fixed grid-template-columns + white-space:nowrap + overflow-x:auto
// (console.css) must be overridden to a wrapping layout specifically
// inside the compact node card, without touching console.css itself
// (which the Continuum page still relies on unchanged).
test("live.css overrides the shared terminal-metrics row to wrap instead of scrolling horizontally inside a node card", async () => {
  const css = await readFile(new URL("../src/live/live.css", import.meta.url), "utf8");
  assert.ok(css.includes(".live-node-card .terminal-metrics"), "must scope the override to the node card, never console.css globally");
  const rowRuleStart = css.indexOf(".live-node-card .terminal-metric-row {");
  assert.ok(rowRuleStart !== -1);
  const rowRule = css.slice(rowRuleStart, css.indexOf("}", rowRuleStart));
  assert.ok(rowRule.includes("flex-wrap: wrap"));
  assert.ok(rowRule.includes("white-space: normal"), "the row itself must allow wrapping -- only the bar/value spans keep nowrap");
  assert.ok(css.includes(".live-node-card .terminal-metrics {") && /overflow-x:\s*visible/.test(css), "the panel must not fall back to an internal horizontal scrollbar inside a node card");
});

test("console.css (shared with the Continuum page) is not modified by the Live-specific overflow fix", async () => {
  const css = await readFile(new URL("../src/styles/console.css", import.meta.url), "utf8");
  // Sanity: the original fixed-grid rule the override replaces for Live
  // still exists untouched for the Continuum page's own usage.
  assert.ok(css.includes("grid-template-columns: 5.25rem max-content minmax(7rem, 1fr);"));
});

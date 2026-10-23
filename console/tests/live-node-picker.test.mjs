import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  nodeProjectIds,
  nodeDiscoveryOptions,
  matchesNodeDiscoveryFilters,
  applyNodeSelection,
  EMPTY_NODE_DISCOVERY_FILTERS,
} from "../src/live/live-node-picker.ts";
import { isActiveAttention } from "../src/pages/wallboard.ts";
import { alertFixture } from "./wallboard-fixture.mjs";

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
function placement(projectId, applicationId = "app", serviceId = "svc") {
  return { project_id: projectId, application_id: applicationId, service_id: serviceId };
}
function observation(convergence) {
  return {
    project_id: "alpha",
    application_id: "app",
    service_id: "svc",
    placement_convergence: convergence,
    convergence_findings: [],
    admitted_evidence: [],
    admitted_evidence_truncated: false,
    excluded_evidence: [],
    excluded_evidence_truncated: false,
  };
}

// ---- project id extraction (accepted-placement based, never "ownership") ----

test("nodeProjectIds returns the distinct set of projects referencing a node via accepted placements", () => {
  const n = node("n1", { accepted_placements: [placement("alpha"), placement("beta"), placement("alpha")] });
  assert.deepEqual(nodeProjectIds(n), ["alpha", "beta"]);
});

test("a node referencing two projects matches a filter for either project", () => {
  const n = node("n1", { accepted_placements: [placement("alpha"), placement("beta")] });
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, projectId: "alpha" }, false));
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, projectId: "beta" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, projectId: "gamma" }, false));
});

// ---- search ----

test("search matches node_id case-insensitively, substring", () => {
  const n = node("Fog-Node-07");
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, search: "fog-node" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, search: "cloud" }, false));
});

// ---- OS / architecture ----

test("OS filter matches the literal platform.os field", () => {
  const n = node("n1", { structure: { platform: { os: "windows", architecture: "amd64" }, capacity: { cpu_cores: 2, memory_bytes: 1 }, dnode_abi_versions: [] } });
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, os: "windows" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, os: "linux" }, false));
});

test("architecture filter matches the literal platform.architecture field", () => {
  const n = node("n1", { structure: { platform: { os: "linux", architecture: "arm64" }, capacity: { cpu_cores: 2, memory_bytes: 1 }, dnode_abi_versions: [] } });
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, architecture: "arm64" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, architecture: "amd64" }, false));
});

// ---- telemetry / observation / alerts tri-state ----

test("telemetry filter distinguishes present vs absent host resource_observation", () => {
  const withTelemetry = node("n1", { resource_observation: { resource_id: "r", observed_at_utc: "t", last_seen_utc: "t", age_seconds: 1, cpu: { logical_cores: 1 }, memory: { total_bytes: 1, used_bytes: 1, available_bytes: 0 }, swap: { total_bytes: 0, used_bytes: 0, available_bytes: 0 } } });
  const withoutTelemetry = node("n2");
  assert.ok(matchesNodeDiscoveryFilters(withTelemetry, { ...EMPTY_NODE_DISCOVERY_FILTERS, telemetry: "present" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(withoutTelemetry, { ...EMPTY_NODE_DISCOVERY_FILTERS, telemetry: "present" }, false));
  assert.ok(matchesNodeDiscoveryFilters(withoutTelemetry, { ...EMPTY_NODE_DISCOVERY_FILTERS, telemetry: "absent" }, false));
});

test("observation filter distinguishes nodes with and without DMonitor evidence", () => {
  const withObs = node("n1", { observations: [observation("converged")] });
  const withoutObs = node("n2");
  assert.ok(matchesNodeDiscoveryFilters(withObs, { ...EMPTY_NODE_DISCOVERY_FILTERS, observation: "present" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(withoutObs, { ...EMPTY_NODE_DISCOVERY_FILTERS, observation: "present" }, false));
});

test("alerts filter uses the caller-supplied correlation flag, never re-derives it", () => {
  const n = node("n1");
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "present" }, true));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "present" }, false));
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "absent" }, false));
});

// Review P2-A fix: the "Active alerts" filter must use the exact same
// canonical active-alert semantics (`isActiveAttention`: open/acknowledged/
// in_progress) the rest of the Live page already uses -- never the raw
// correlated-alert count, which would also count resolved/closed alerts.
function hasActiveAlert(alerts) {
  return alerts.filter(isActiveAttention).length > 0;
}

test("a node with only a resolved alert is NOT matched by Active alerts: Present", () => {
  const resolved = alertFixture("resolved", "a1");
  const n = node("n1");
  assert.equal(hasActiveAlert([resolved]), false);
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "present" }, hasActiveAlert([resolved])));
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "absent" }, hasActiveAlert([resolved])));
});

test("a node with at least one active alert IS matched by Active alerts: Present", () => {
  const open = alertFixture("open", "a1");
  const n = node("n1");
  assert.equal(hasActiveAlert([open]), true);
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "present" }, hasActiveAlert([open])));
});

test("a mix of an active alert and a resolved alert still matches Active alerts: Present", () => {
  const open = alertFixture("open", "a1");
  const resolved = alertFixture("resolved", "a2");
  const n = node("n1");
  assert.equal(hasActiveAlert([open, resolved]), true);
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, alerts: "present" }, hasActiveAlert([open, resolved])));
});

test("convergence filter matches if ANY observation on the node has that literal convergence value", () => {
  const n = node("n1", { observations: [observation("converged"), observation("unknown")] });
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, convergence: "converged" }, false));
  assert.ok(matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, convergence: "unknown" }, false));
  assert.ok(!matchesNodeDiscoveryFilters(n, { ...EMPTY_NODE_DISCOVERY_FILTERS, convergence: "diverged" }, false));
});

// ---- discovery options ----

test("nodeDiscoveryOptions collects distinct, sorted project/OS/architecture values across all nodes", () => {
  const nodes = [
    node("n1", { accepted_placements: [placement("beta")], structure: { platform: { os: "linux", architecture: "arm64" }, capacity: { cpu_cores: 1, memory_bytes: 1 }, dnode_abi_versions: [] } }),
    node("n2", { accepted_placements: [placement("alpha")], structure: { platform: { os: "windows", architecture: "amd64" }, capacity: { cpu_cores: 1, memory_bytes: 1 }, dnode_abi_versions: [] } }),
  ];
  const options = nodeDiscoveryOptions(nodes);
  assert.deepEqual(options.projects, ["alpha", "beta"]);
  assert.deepEqual(options.os, ["linux", "windows"]);
  assert.deepEqual(options.architectures, ["amd64", "arm64"]);
});

// ---- selection application: order preservation + never silently dropping a missing pin ----

test("applyNodeSelection preserves the previous pinned order for ids that remain selected", () => {
  const result = applyNodeSelection(["c", "a", "b"], new Set(["a", "b", "c"]), ["a", "b", "c"]);
  assert.deepEqual(result, ["c", "a", "b"]);
});

test("applyNodeSelection appends newly-selected ids in discovery order, after the preserved existing order", () => {
  const result = applyNodeSelection(["b"], new Set(["b", "a", "c"]), ["a", "b", "c"]);
  assert.deepEqual(result, ["b", "a", "c"]);
});

test("applyNodeSelection removes an id that was explicitly unchecked", () => {
  const result = applyNodeSelection(["a", "b"], new Set(["a"]), ["a", "b"]);
  assert.deepEqual(result, ["a"]);
});

test("applyNodeSelection keeps a selected id that is absent from the current discovery list (unavailable, never silently dropped)", () => {
  const result = applyNodeSelection(["missing-node", "a"], new Set(["missing-node", "a"]), ["a"]);
  assert.deepEqual(result, ["missing-node", "a"]);
});

test("applyNodeSelection on an empty selection returns an empty list", () => {
  assert.deepEqual(applyNodeSelection(["a", "b"], new Set(), ["a", "b"]), []);
});

// ---- structural: no fog/edge/cloud/role inference from node_id or hostname ----

test("live-node-picker.ts never derives a node type/role from node_id or hostname", async () => {
  const code = await sourceWithoutComments("../src/live/live-node-picker.ts");
  for (const forbidden of ["fog", "edge", "cloud", "hostname", "startsWith(\"fog", "startsWith(\"edge", "startsWith(\"cloud"]) {
    assert.ok(!code.toLowerCase().includes(forbidden.toLowerCase()), `live-node-picker.ts must never reference '${forbidden}' (no node type/role inference)`);
  }
});

test("live-node-picker.ts filters are client-side only -- no fetch/HTTP call exists in this module", async () => {
  const code = await sourceWithoutComments("../src/live/live-node-picker.ts");
  assert.ok(!code.includes("fetch("));
  assert.ok(!/method:\s*["'](POST|PUT|PATCH|DELETE)["']/iu.test(code));
});

// Cancel must never call onApply -- only the Apply button does, and it
// must go through applyNodeSelection (never a hand-rolled replacement).
test("onApply is invoked from exactly the applyButton handler, never from Cancel/close", async () => {
  const code = await sourceWithoutComments("../src/live/live-node-picker.ts");
  const closeBody = code.slice(code.indexOf("function close(): void {"), code.indexOf("closeButton.addEventListener"));
  assert.ok(!closeBody.includes("onApply("), "close() must never call onApply");
  const applyHandlerStart = code.indexOf('applyButton.addEventListener("click"');
  const applyHandlerBody = code.slice(applyHandlerStart, code.indexOf("});", applyHandlerStart));
  assert.ok(applyHandlerBody.includes("onApply(applyNodeSelection("));
});

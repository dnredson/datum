import test from "node:test";
import assert from "node:assert/strict";
import cytoscape from "cytoscape";
import { runtimeFixture, runtimeVisualReviewFixture } from "./runtime-graph-fixture.mjs";
import { unevenGraphFixture } from "./graph-fixture.mjs";
import {
  validRuntimeGraph,
  runtimeElements,
} from "../src/pages/runtime-graph-model.ts";
import { layoutGroupedGraph } from "../src/pages/graph-model.ts";
import { graphVisualStyles } from "../src/components/graph-visuals.ts";
import { OperationsClient } from "../src/api/client.ts";
const fresh = () => runtimeFixture();
test("manual-only populated fixture preserves typed unmatched identity", () => {
  const f = runtimeVisualReviewFixture();
  assert.ok(validRuntimeGraph(f, f.project_id));
  assert.equal(f.runtime.value.evidence.filter((e) => e.logical_node_id === null).length, 1);
});
const duplicateObservation = (scenario) => {
  const f = fresh();
  const index = scenario === "application" ? 4 : scenario === "service" ? 2 : 1;
  if (scenario === "identical tuple") {
    f.runtime.value.evidence[index] = structuredClone(f.runtime.value.evidence[0]);
    f.temporal_quality[index] = structuredClone(f.temporal_quality[0]);
  } else {
    const e = f.runtime.value.evidence[index];
    if (scenario === "epoch") {
      e.node_id = f.runtime.value.evidence[0].node_id;
      e.reporting_node_id = f.runtime.value.evidence[0].reporting_node_id;
      e.id = JSON.stringify(["runtime_evidence", e.project_id, e.application_id,
        e.service_id, e.node_id, e.reporting_node_id, e.monitor_instance_id]);
      f.temporal_quality[index].node_id = e.reporting_node_id;
    }
    e.observation_id = f.runtime.value.evidence[0].observation_id;
    f.temporal_quality[index].observation_id = e.observation_id;
  }
  return f;
};
test("distinct canonical observation IDs are accepted without changing evidence", () => {
  const f = fresh(), before = structuredClone(f);
  assert.ok(validRuntimeGraph(f, "alpha"));
  assert.deepEqual(f, before);
});
for (const scenario of ["identical tuple", "node", "epoch", "application", "service"])
  test(`duplicate observation ID rejected without repair: ${scenario}`, () => {
    const f = duplicateObservation(scenario), before = structuredClone(f);
    assert.equal(validRuntimeGraph(f, "alpha"), false);
    assert.deepEqual(f, before);
  });
test("duplicate observation response fails closed with safe invalid_response", async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify(duplicateObservation("node")));
  try {
    await assert.rejects(new OperationsClient().runtimeGraph("alpha", new AbortController().signal),
      (error) => error.code === "invalid_response" &&
        error.message === "DServer returned an invalid runtime evidence projection.");
  } finally {
    globalThis.fetch = original;
  }
});
test("equivalent RFC3339 offset timestamps preserve canonical temporal provenance", () => {
  const f = fresh();
  f.runtime.value.evidence[0].observed_at_utc = "2026-09-30T11:50:00-03:00";
  assert.ok(validRuntimeGraph(f, "alpha"));
  f.runtime.value.evidence[0].observed_at_utc = "2026-09-30T11:51:00-03:00";
  assert.equal(validRuntimeGraph(f, "alpha"), false);
});
test("typed container resource usage is accepted and malformed usage fails closed", () => {
  const f = fresh();
  f.runtime.value.evidence[0].resource_usage = {
    cpu_percent: 3.25,
    memory_used_bytes: 12582912,
    memory_limit_bytes: 1073741824,
    pids: 7,
  };
  assert.ok(validRuntimeGraph(f, "alpha"));
  f.runtime.value.evidence[0].resource_usage.memory_used_bytes = 2147483648;
  assert.equal(validRuntimeGraph(f, "alpha"), false);
});

test("runtime source is admitted typed realization evidence, not desired/events/temporal", () => {
  assert.ok(validRuntimeGraph(fresh(), "alpha"));
  for (const source of [
    "accepted_dmap",
    "operational_dgraph",
    "datum_events",
    "temporal_quality",
  ]) {
    const f = fresh();
    f.runtime_source_kind = source;
    assert.equal(validRuntimeGraph(f, "alpha"), false);
  }
});
test("exact service/app identity rejects fuzzy and cross-project joins", () => {
  for (const field of ["project_id", "application_id", "service_id"]) {
    const f = fresh();
    f.runtime.value.evidence[0][field] = "different";
    assert.equal(validRuntimeGraph(f, "alpha"), false);
  }
  const f = fresh();
  f.runtime.value.evidence[0].logical_node_id = "app-b-one";
  assert.equal(validRuntimeGraph(f, "alpha"), false);
});
test("same service across applications and multiple independent epochs remain distinct", () => {
  const f = fresh();
  const records = f.runtime.value.evidence;
  assert.equal(new Set(records.map((e) => e.id)).size, records.length);
  assert.equal(records.filter((e) => e.service_id === "one").length, 4);
  assert.ok(validRuntimeGraph(f, "alpha"));
});
test("unavailable source is not zero and valid empty evidence has no health implication", () => {
  for (const availability of [
    "not_configured",
    "indeterminate",
    "unavailable",
    "corrupt",
  ]) {
    const f = fresh();
    f.runtime = {
      availability,
      value:
        availability === "indeterminate"
          ? "uncertain"
          : {
              code: "source_failed",
              source: "dmonitor",
              safe_message: "Safe failure",
            },
    };
    f.temporal_quality = [];
    assert.ok(validRuntimeGraph(f, "alpha"));
  }
  const f = runtimeFixture(undefined, false);
  assert.ok(validRuntimeGraph(f, "alpha"));
  assert.equal(
    runtimeElements(
      f.logical.graph.value,
      f,
      new Set(["app-a-one"]),
      null,
    ).filter((e) => e.data.kind === "runtime_evidence").length,
    0,
  );
});
test("collapsed default; expand/collapse/filter are local and preserve logical DCalls", () => {
  const f = fresh(),
    g = f.logical.graph.value,
    expanded = new Set();
  const collapsed = runtimeElements(g, f, expanded, null);
  expanded.add("app-a-one");
  const visible = runtimeElements(g, f, expanded, null);
  assert.equal(
    visible.filter((e) => e.data.kind === "runtime_evidence").length,
    2,
  );
  assert.deepEqual(
    visible.filter((e) => e.data.kind === "dcall"),
    collapsed.filter((e) => e.data.kind === "dcall"),
  );
  assert.equal(
    runtimeElements(g, f, expanded, "app-b").filter(
      (e) => e.data.kind === "runtime_evidence",
    ).length,
    0,
  );
  expanded.clear();
  assert.deepEqual(runtimeElements(g, f, expanded, null), collapsed);
});
test("runtime uses shared category and relation tokens, no DNode/placement vertices", () => {
  const f = fresh(),
    g = f.logical.graph.value;
  const visible = runtimeElements(
    g,
    f,
    new Set(g.nodes.map((n) => n.id)),
    null,
  );
  assert.ok(
    visible
      .filter((e) => e.data.kind === "runtime_evidence")
      .every((e) => e.data.visual_category === "runtime"),
  );
  assert.ok(
    visible
      .filter((e) => e.data.kind === "dserv")
      .every((e) => e.data.visual_category === "software"),
  );
  assert.ok(
    visible
      .filter((e) => e.data.kind === "realization")
      .every((e) => e.data.visual_relation === "realization"),
  );
  assert.ok(
    !visible.some((e) =>
      ["dnode", "placement", "physical"].includes(e.data.kind),
    ),
  );
});
for (const mode of ["collapsed", "one", "all", "filter", "long"])
  test(`headless runtime layout ${mode}: deterministic, resettable, non-overlapping`, () => {
    const f = runtimeFixture(unevenGraphFixture()),
      g = f.logical.graph.value;
    if (mode === "long")
      g.nodes.forEach((n) => (n.canonical_id += "x".repeat(400)));
    const expanded = new Set(
      mode === "collapsed"
        ? []
        : mode === "one"
          ? [g.nodes.find((n) => n.kind === "dserv").id]
          : g.nodes.filter((n) => n.kind === "dserv").map((n) => n.id),
    );
    const cy = cytoscape({
      headless: true,
      styleEnabled: true,
      elements: runtimeElements(
        g,
        f,
        expanded,
        mode === "filter" ? "application-B" : null,
      ),
      style: graphVisualStyles(),
      layout: { name: "preset" },
    });
    try {
      layoutGroupedGraph(cy);
      const initial = cy.nodes().map((n) => [n.id(), n.position()]);
      const groups = cy.nodes().filter((n) => n.data("kind") === "application");
      for (let i = 0; i < groups.length; i++)
        for (let j = i + 1; j < groups.length; j++) {
          const a = groups[i].boundingBox({ includeLabels: true }),
            b = groups[j].boundingBox({ includeLabels: true });
          assert.ok(
            a.x2 <= b.x1 || b.x2 <= a.x1 || a.y2 <= b.y1 || b.y2 <= a.y1,
          );
        }
      cy.nodes().forEach((n) => n.position({ x: 123, y: 456 }));
      layoutGroupedGraph(cy);
      assert.deepEqual(
        cy.nodes().map((n) => [n.id(), n.position()]),
        initial,
      );
    } finally {
      cy.destroy();
    }
  });
test("one scoped runtime request, no per-service fetch", async () => {
  const original = globalThis.fetch,
    calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(fresh()));
  };
  try {
    await new OperationsClient().runtimeGraph(
      "alpha",
      new AbortController().signal,
    );
    assert.equal(calls.length, 1);
    assert.equal(
      calls[0].url,
      "/api/v1/operations/projects/alpha/runtime-graph",
    );
    assert.equal(calls[0].init.headers["X-Datum-Project"], "alpha");
  } finally {
    globalThis.fetch = original;
  }
});

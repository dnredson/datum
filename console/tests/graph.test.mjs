import test from "node:test";
import assert from "node:assert/strict";
import cytoscape from "cytoscape";
import { graphFixture } from "./graph-fixture.mjs";
import {
  GRAPH_SOURCE_LABEL,
  validProjectGraph,
  graphElements,
  edgeCounts,
  layoutGroupedGraph,
  graphCoverageText,
} from "../src/pages/graph-model.ts";
import { parseRoute, projectPath } from "../src/app/router.ts";
import { OperationsClient } from "../src/api/client.ts";

test("source explicitly identifies accepted logical snapshots, not operational/runtime graph", () => {
  assert.match(
    GRAPH_SOURCE_LABEL,
    /Accepted application D-Graph snapshots.*logical structure/,
  );
  assert.equal(validProjectGraph(graphFixture(), "alpha"), true);
  const f = graphFixture();
  f.source_kind = "operational_project_graph";
  assert.equal(validProjectGraph(f, "alpha"), false);
});

function layoutFixture(counts, long = false) {
  const g = graphFixture().graph.value;
  const service = g.nodes.find((n) => n.kind === "dserv");
  const parent = g.nodes.find((n) => n.kind === "application");
  const app = g.applications[0];
  g.nodes = [];
  g.edges = [];
  g.applications = [];
  for (const [i, count] of counts.entries()) {
    const id = long
      ? `application-${i}-` + "long-identity-".repeat(20)
      : `app-${i}`;
    g.applications.push({ ...app, application_id: id });
    g.nodes.push({
      ...parent,
      id: `group-${i}`,
      canonical_id: id,
      application_id: id,
    });
    for (let j = 0; j < count; j++)
      g.nodes.push({
        ...service,
        id: `service-${i}-${j}`,
        canonical_id: `service-${j}`,
        parent_id: `group-${i}`,
        application_id: id,
      });
  }
  return g;
}
function layoutCore(graph, application = null) {
  return cytoscape({
    headless: true,
    styleEnabled: true,
    elements: graphElements(graph, application),
    layout: { name: "preset" },
    style: [
      {
        selector: "node",
        style: {
          width: 48,
          height: 34,
          label: "data(label)",
          "font-size": 11,
          "text-valign": "bottom",
          "text-margin-y": 7,
          "text-wrap": "wrap",
          "text-max-width": 140,
        },
      },
      {
        selector: 'node[kind = "application"]',
        style: {
          padding: 30,
          width: 180,
          height: 80,
          "min-width": 180,
          "min-height": 80,
          "text-valign": "top",
          "text-margin-y": -10,
          "text-max-width": 180,
          "border-width": 1,
        },
      },
    ],
  });
}
function positions(cy) {
  return cy.nodes().map((n) => ({ id: n.id(), ...n.position() }));
}
function assertNoOverlap(cy) {
  const groups = cy.nodes().filter((n) => n.data("kind") === "application");
  for (let i = 0; i < groups.length; i++)
    for (let j = i + 1; j < groups.length; j++) {
      const a = groups[i].boundingBox({ includeLabels: true }),
        b = groups[j].boundingBox({ includeLabels: true });
      assert.ok(
        a.x2 <= b.x1 || b.x2 <= a.x1 || a.y2 <= b.y1 || b.y2 <= a.y1,
        `overlapping groups ${groups[i].id()} / ${groups[j].id()}`,
      );
    }
}
for (const counts of [
  [2, 3, 2],
  [1, 8, 2, 12],
  [30, 1, 2, 1],
  [0, 2, 0],
])
  test(`two-level layout prevents measured compound overlap: ${counts}`, () => {
    const cy = layoutCore(layoutFixture(counts));
    layoutGroupedGraph(cy);
    assertNoOverlap(cy);
    cy.destroy();
  });
test("same graph and Reset restore identical coordinates after dragging", () => {
  const graph = layoutFixture([2, 3, 2]);
  const a = layoutCore(graph),
    b = layoutCore(graph);
  layoutGroupedGraph(a);
  layoutGroupedGraph(b);
  const expected = positions(a);
  assert.deepEqual(positions(b), expected);
  a.nodes()
    .filter((n) => n.data("kind") === "dserv")
    .positions(() => ({ x: -900, y: 1500 }));
  layoutGroupedGraph(a);
  assert.deepEqual(positions(a), expected);
  assertNoOverlap(a);
  a.destroy();
  b.destroy();
});
test("filter subset packs visible groups; returning to All restores original positions", () => {
  const graph = layoutFixture([1, 8, 2, 12]);
  const cy = layoutCore(graph);
  layoutGroupedGraph(cy);
  const original = positions(cy);
  const subset = {
    ...graph,
    nodes: graph.nodes.filter((n) =>
      ["app-1", "app-3"].includes(n.application_id),
    ),
  };
  cy.elements().remove();
  cy.add(graphElements(subset));
  layoutGroupedGraph(cy);
  assertNoOverlap(cy);
  layoutGroupedGraph(cy);
  assertNoOverlap(cy);
  cy.elements().remove();
  cy.add(graphElements(graph));
  layoutGroupedGraph(cy);
  assert.deepEqual(positions(cy), original);
  const one = layoutCore(graph, "app-1");
  layoutGroupedGraph(one);
  assert.equal(
    one.nodes().filter((n) => n.data("kind") === "application").length,
    1,
  );
  assertNoOverlap(one);
  cy.destroy();
  one.destroy();
});
test("long application labels reserve header space without overlapping service children", () => {
  const cy = layoutCore(layoutFixture([2, 3, 2], true));
  layoutGroupedGraph(cy);
  assertNoOverlap(cy);
  cy.nodes()
    .filter((n) => n.data("kind") === "application")
    .forEach((group) => {
      const bounds = group.boundingBox();
      group
        .children()
        .forEach((n) => assert.ok(n.boundingBox().y1 > bounds.y1 + 20));
    });
  cy.destroy();
});
test("self-calls, cycles and parallel edges cannot alter generated node positions", () => {
  const graph = graphFixture().graph.value;
  const cy = layoutCore(graph);
  layoutGroupedGraph(cy);
  const original = positions(cy);
  const e = graph.edges.find((edge) => edge.source !== edge.target);
  cy.add({ group: "edges", data: { ...e, id: "parallel-call" } });
  layoutGroupedGraph(cy);
  assert.deepEqual(positions(cy), original);
  assertNoOverlap(cy);
  assert.ok(
    cy.edges().some((edge) => edge.source().id() === edge.target().id()),
  );
  cy.destroy();
});
test("Complete coverage describes accepted source, not complete project inventory", () => {
  assert.match(
    graphCoverageText(graphFixture().graph.value),
    /2 of 2 active accepted application snapshots included/,
  );
  assert.match(
    graphCoverageText(graphFixture().graph.value),
    /Pending\/unaccepted applications are outside this source/,
  );
});
test("Partial nonempty coverage explicitly explains exact omissions and bounded preview", () => {
  const graph = graphFixture().graph.value;
  graph.coverage.completeness = "partial";
  graph.coverage.active_application_count = 5;
  const text = graphCoverageText(graph);
  assert.match(text, /Bounded graph preview: 2 of 5/);
  assert.match(text, /3 accepted snapshots were omitted by preview limits/);
  assert.match(
    text,
    /100 applications, 2,000 nodes including groups, 4,000 D-Calls/,
  );
  assert.match(text, /later snapshots are not considered/);
  assert.match(text, /not complete project application inventory/);
});
test("Partial bounded-empty coverage never becomes a true empty accepted source", () => {
  const graph = layoutFixture([]);
  graph.coverage = {
    ...graph.coverage,
    completeness: "partial",
    active_application_count: 1,
    included_application_count: 0,
  };
  assert.match(
    graphCoverageText(graph),
    /0 of 1 accepted application snapshots included/,
  );
  assert.match(
    graphCoverageText(graph),
    /1 accepted snapshot was omitted by preview limits/,
  );
});
test("graph route safely retains encoded project scope and future routes stay unavailable", () => {
  for (const id of ["alpha", "project one", "a/b", "a?b#c"])
    assert.deepEqual(parseRoute(`${projectPath(id)}/graph`), {
      kind: "project_graph",
      projectId: id,
    });
  assert.equal(parseRoute("/projects/a/graph/runtime").kind, "not_found");
});
test("graph client sends exactly one scoped GET and rejects cross-project response", async () => {
  const original = globalThis.fetch;
  const calls = [];
  let data = graphFixture("project one");
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(data));
  };
  try {
    const result = await new OperationsClient().graph(
      "project one",
      new AbortController().signal,
    );
    assert.equal(result.project_id, "project one");
    assert.equal(calls.length, 1);
    assert.equal(
      calls[0].url,
      "/api/v1/operations/projects/project%20one/graph",
    );
    assert.equal(calls[0].init.headers["X-Datum-Project"], "project one");
    assert.equal(calls[0].init.method, "GET");
    data = graphFixture("other");
    await assert.rejects(
      new OperationsClient().graph("project one", new AbortController().signal),
      (e) => e.code === "invalid_response",
    );
  } finally {
    globalThis.fetch = original;
  }
});
for (const field of ["nodes", "edges"])
  test(`explicit cross-project ${field} scope is rejected`, () => {
    const f = graphFixture();
    f.graph.value[field][0].project_id = "beta";
    assert.equal(validProjectGraph(f, "alpha"), false);
  });
test("canonical identities and duplicate service IDs across applications remain distinct", () => {
  const g = graphFixture().graph.value;
  const elements = graphElements(g);
  assert.equal(new Set(elements.map((e) => e.data.id)).size, 12);
  assert.equal(g.nodes.filter((n) => n.canonical_id === "one").length, 2);
  assert.equal(g.edges.filter((e) => e.canonical_id === "forward").length, 2);
});
test("application and D-Serv kinds differ while unsupported physical relationship is rejected", () => {
  const f = graphFixture();
  assert.deepEqual(
    new Set(
      graphElements(f.graph.value)
        .filter((e) => e.group === "nodes")
        .map((e) => e.data.kind),
    ),
    new Set(["application", "dserv"]),
  );
  assert.deepEqual(
    new Set(f.graph.value.edges.map((e) => e.kind)),
    new Set(["dcall"]),
  );
  f.graph.value.edges[0].kind = "placed_on";
  assert.equal(validProjectGraph(f, "alpha"), false);
});
test("D-Call cycles reciprocal calls and self-calls are accepted by contract and Cytoscape", () => {
  const f = graphFixture();
  assert.equal(validProjectGraph(f, "alpha"), true);
  const cy = cytoscape({
    headless: true,
    elements: graphElements(f.graph.value),
    layout: { name: "grid" },
  });
  try {
    assert.equal(cy.edges().length, 6);
    assert.equal(
      cy.edges().filter((e) => e.source().id() === e.target().id()).length,
      2,
    );
    assert.equal(cy.getElementById("app-a-forward").source().id(), "app-a-one");
    assert.equal(cy.getElementById("app-a-reverse").target().id(), "app-a-one");
  } finally {
    cy.destroy();
  }
});
test("deterministic element order ignores input ordering", () => {
  const f = graphFixture().graph.value;
  const original = graphElements(f);
  f.nodes.reverse();
  f.edges.reverse();
  assert.deepEqual(graphElements(f), original);
});
test("Cytoscape preserves opaque JSON-tuple IDs and compound references without parsing keys", () => {
  const g = graphFixture().graph.value;
  const keys = new Map(
    g.nodes.map((n) => [
      n.id,
      JSON.stringify([
        n.application_id,
        'logical:α/quoted"',
        n.kind,
        n.canonical_id,
      ]),
    ]),
  );
  for (const n of g.nodes) {
    n.id = keys.get(n.id);
    if (n.parent_id !== null) n.parent_id = keys.get(n.parent_id);
  }
  for (const e of g.edges) {
    e.id = JSON.stringify([
      e.application_id,
      'logical:α/quoted"',
      e.kind,
      e.canonical_id,
    ]);
    e.source = keys.get(e.source);
    e.target = keys.get(e.target);
  }
  const cy = cytoscape({
    headless: true,
    elements: graphElements(g),
    layout: { name: "grid" },
  });
  try {
    for (const n of g.nodes) {
      assert.equal(cy.getElementById(n.id).length, 1);
      if (n.parent_id !== null)
        assert.equal(cy.getElementById(n.id).parent().id(), n.parent_id);
    }
    assert.equal(cy.edges().length, 6);
  } finally {
    cy.destroy();
  }
});
test("application filtering is presentation-only and never mutates canonical data", () => {
  const f = graphFixture().graph.value;
  const before = JSON.stringify(f);
  const filtered = graphElements(f, "app-a");
  assert.equal(filtered.length, 6);
  assert.equal(JSON.stringify(f), before);
  assert.equal(graphElements(f, "missing").length, 0);
});
test("edge counts preserve self-call incoming/outgoing semantics", () => {
  assert.deepEqual(edgeCounts(graphFixture().graph.value, "app-a-one"), {
    incoming: 2,
    outgoing: 2,
  });
});
test("valid empty graph differs from not configured/unavailable/corrupt/indeterminate", () => {
  const f = graphFixture();
  Object.assign(f.graph.value, {
    applications: [],
    nodes: [],
    edges: [],
    coverage: {
      scope: "active_accepted_applications_only",
      completeness: "complete",
      active_application_count: 0,
      included_application_count: 0,
    },
  });
  assert.equal(validProjectGraph(f, "alpha"), true);
  for (const state of [
    "not_configured",
    "unavailable",
    "corrupt",
    "indeterminate",
  ]) {
    f.graph = {
      availability: state,
      ...(state === "not_configured"
        ? {}
        : {
            value:
              state === "indeterminate"
                ? "internal diagnostic"
                : {
                    source: "ddeploy",
                    code: "source_corrupt",
                    safe_message: "Safe bounded issue",
                  },
          }),
    };
    assert.equal(validProjectGraph(f, "alpha"), true);
  }
});
test("partial coverage is independent of included graph completeness", () => {
  const f = graphFixture();
  f.graph.value.coverage.completeness = "partial";
  f.graph.value.coverage.active_application_count = 5;
  assert.equal(validProjectGraph(f, "alpha"), true);
  f.graph.value.coverage.completeness = "complete";
  assert.equal(validProjectGraph(f, "alpha"), false);
});
test("actual graph revision and exact large declared memory remain typed", () => {
  const f = graphFixture();
  assert.equal(validProjectGraph(f, "alpha"), true);
  assert.equal(f.graph.value.applications[0].revision, 4);
  f.graph.value.applications[0].revision = null;
  assert.equal(validProjectGraph(f, "alpha"), false);
});
test("dangling and cross-application edges, duplicate IDs and missing parent are rejected", () => {
  for (const mutate of [
    (f) => (f.edges[0].target = "missing"),
    (f) => (f.edges[0].target = "app-b-one"),
    (f) => (f.nodes[1].id = f.nodes[0].id),
    (f) => (f.nodes[1].parent_id = null),
  ]) {
    const f = graphFixture();
    mutate(f.graph.value);
    assert.equal(validProjectGraph(f, "alpha"), false);
  }
});
test("long full canonical identity survives graph label abbreviation", () => {
  const f = graphFixture();
  const id = "service-".repeat(100);
  f.graph.value.nodes[1].canonical_id = id;
  assert.equal(validProjectGraph(f, "alpha"), true);
  assert.equal(f.graph.value.nodes[1].canonical_id, id);
  assert.ok(
    graphElements(f.graph.value).find((e) => e.data.id === "app-a-one").data
      .label.length <= 28,
  );
});
test("graph budgets reject oversized UI payloads and safe issue messages", () => {
  const f = graphFixture();
  f.graph.value.nodes = Array(2001).fill(f.graph.value.nodes[0]);
  assert.equal(validProjectGraph(f, "alpha"), false);
  f.graph = {
    availability: "unavailable",
    value: {
      source: "ddeploy",
      code: "source_unavailable",
      safe_message: "x".repeat(513),
    },
  };
  assert.equal(validProjectGraph(f, "alpha"), false);
});

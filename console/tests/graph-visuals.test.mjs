import test from "node:test";
import assert from "node:assert/strict";
import cytoscape from "cytoscape";
import { graphFixture } from "./graph-fixture.mjs";
import { graphElements } from "../src/pages/graph-model.ts";
import {
  GRAPH_CATEGORIES,
  GRAPH_RELATIONS,
  GRAPH_STATUS_OVERLAYS,
  graphVisualStyles,
  graphStatusOverlay,
  graphLegendItems,
  graphLegendItemsFor,
} from "../src/components/graph-visuals.ts";

test("stable category vocabulary preserves software/runtime/infrastructure distinction", () => {
  assert.deepEqual(Object.keys(GRAPH_CATEGORIES), [
    "infrastructure",
    "software",
    "runtime",
    "storage",
    "iot",
  ]);
  assert.equal(
    new Set(Object.values(GRAPH_CATEGORIES).map((c) => c.color)).size,
    5,
  );
  assert.equal(GRAPH_CATEGORIES.software.shape, "rectangle");
  assert.notEqual(
    GRAPH_CATEGORIES.software.color,
    GRAPH_CATEGORIES.runtime.color,
  );
  assert.match(
    GRAPH_CATEGORIES.infrastructure.meaning,
    /not project ownership/,
  );
});
test("current graph maps only canonical DServ and DCall; application grouping is neutral", () => {
  const elements = graphElements(graphFixture().graph.value);
  assert.ok(
    elements
      .filter((e) => e.data.kind === "dserv")
      .every((e) => e.data.visual_category === "software"),
  );
  assert.ok(
    elements
      .filter((e) => e.data.kind === "application")
      .every((e) => e.data.visual_category === undefined),
  );
  assert.ok(
    elements
      .filter((e) => e.group === "edges")
      .every((e) => e.data.visual_relation === "logical"),
  );
  assert.equal(
    elements.some((e) =>
      ["runtime", "infrastructure", "storage", "iot"].includes(
        e.data.visual_category,
      ),
    ),
    false,
  );
});
test("real Cytoscape styles retain purple software on selection and neutral containers", () => {
  const cy = cytoscape({
    headless: true,
    styleEnabled: true,
    elements: graphElements(graphFixture().graph.value),
    style: graphVisualStyles(),
  });
  const node = cy
    .nodes()
    .filter((n) => n.data("kind") === "dserv")
    .first();
  const fill = node.style("background-color");
  assert.equal(fill, "rgb(120,82,161)");
  assert.equal(node.style("shape"), "rectangle");
  node.select();
  assert.equal(node.style("background-color"), fill);
  const group = cy
    .nodes()
    .filter((n) => n.data("kind") === "application")
    .first();
  assert.notEqual(group.style("background-color"), fill);
  assert.equal(cy.edges().first().style("line-style"), "solid");
  assert.equal(cy.edges().first().style("target-arrow-shape"), "triangle");
  cy.destroy();
});
test("status overlay tokens never replace category fill/shape or infer status", () => {
  for (const status of Object.keys(GRAPH_STATUS_OVERLAYS)) {
    const overlay = graphStatusOverlay(status);
    assert.equal(overlay["outline-color"], GRAPH_STATUS_OVERLAYS[status].color);
    assert.equal("background-color" in overlay, false);
    assert.equal("shape" in overlay, false);
  }
  assert.equal(
    graphVisualStyles().some((s) => s.selector.includes("status")),
    false,
  );
});
test("relation vocabulary separates invocation, realization, placement, physical and observation", () => {
  assert.equal(GRAPH_RELATIONS.logical.line, "solid");
  assert.equal(GRAPH_RELATIONS.logical.arrow, "triangle");
  assert.match(GRAPH_RELATIONS.logical.meaning, /No deployment-order/);
  assert.equal(GRAPH_RELATIONS.realization.arrow, "vee");
  assert.equal(GRAPH_RELATIONS.placement.line, "dashed");
  assert.equal(GRAPH_RELATIONS.observation.line, "dotted");
  assert.equal(GRAPH_RELATIONS.physical.arrow, "none");
});
test("legend includes only actually present types with textual semantics", () => {
  const graph = graphFixture().graph.value;
  assert.deepEqual(
    graphLegendItems(graph).map((i) => i.id),
    ["software", "application-group", "logical"],
  );
  assert.deepEqual(graphLegendItems({ nodes: [], edges: [] }), []);
  assert.deepEqual(
    graphLegendItems({
      nodes: graph.nodes.filter((n) => n.kind === "application"),
      edges: [],
    }).map((i) => i.id),
    ["application-group"],
  );
  assert.match(
    graphLegendItems(graph).find((i) => i.id === "application-group").meaning,
    /Presentation-only/,
  );
  assert.deepEqual(
    graphLegendItemsFor(
      ["runtime", "software", "runtime"],
      ["placement", "realization"],
    ).map((i) => i.id),
    ["software", "runtime", "realization", "placement"],
  );
});

import test from "node:test";
import assert from "node:assert/strict";
import { continuumFixture, continuumProjectOverlayFixture } from "./continuum-fixture.mjs";
import { computeContinuumMetrics, continuumElements } from "../src/pages/continuum.ts";

test("summary metrics: node count and accepted placement count are grounded in the fixture", () => {
  const metrics = computeContinuumMetrics(continuumFixture());
  assert.equal(metrics.nodeCount, 3);
  assert.equal(metrics.acceptedPlacementCount, 3, "node-a (2) + node-b (1) + node-c (0)");
});

test("shared-node: node-a's two distinct project placements both count toward the total", () => {
  const f = continuumFixture();
  const nodeA = f.nodes.find((n) => n.node_id === "node-a");
  assert.equal(nodeA.accepted_placements.length, 2);
  const metrics = computeContinuumMetrics(f);
  assert.ok(metrics.acceptedPlacementCount >= 2);
});

test("nodes-with-observation vs. nodes-without-observation matches the honest-empty node-c scenario", () => {
  const metrics = computeContinuumMetrics(continuumFixture());
  assert.equal(metrics.nodesWithObservation, 2, "node-a and node-b each carry observations");
  assert.equal(metrics.nodesWithoutObservation, 1, "node-c has neither placement nor observation");
});

test("fresh/stale evidence counts are grounded in per-evidence freshness, admitted and excluded alike", () => {
  const metrics = computeContinuumMetrics(continuumFixture());
  assert.equal(metrics.freshCount, 2, "node-a's two admitted evidence entries are fresh");
  assert.equal(metrics.staleCount, 1, "node-b's one excluded evidence entry is stale");
  assert.equal(metrics.clockSkewCount, 0);
});

test("health counts are derived only from admitted evidence, never from excluded/historical evidence", () => {
  const metrics = computeContinuumMetrics(continuumFixture());
  assert.equal(metrics.healthyCount, 2, "both of node-a's admitted entries report healthy");
  assert.equal(metrics.degradedCount, 0);
  assert.equal(metrics.unhealthyCount, 0);
  assert.equal(metrics.unknownHealthCount, 0, "node-b's stale entry is excluded, so its health claim is not counted as current");
});

test("metrics computation does not mutate the input view", () => {
  const f = continuumFixture();
  const before = JSON.parse(JSON.stringify(f));
  computeContinuumMetrics(f);
  assert.deepEqual(f, before);
});

test("project overlay fixture metrics are identical to the global fixture's -- overlay never changes grounded counts", () => {
  const globalMetrics = computeContinuumMetrics(continuumFixture());
  const overlayMetrics = computeContinuumMetrics(continuumProjectOverlayFixture("project-a"));
  assert.deepEqual(globalMetrics, overlayMetrics);
});

test("canvas elements: one node per D-Node plus one node per accepted placement, no more", () => {
  const elements = continuumElements(continuumFixture());
  const dnodeElements = elements.filter((e) => e.data.kind === "dnode");
  const placementNodeElements = elements.filter((e) => e.data.kind === "placement_ref");
  assert.equal(dnodeElements.length, 3, "node-a, node-b, node-c");
  assert.equal(placementNodeElements.length, 3, "two on node-a, one on node-b, none on node-c");
});

test("canvas elements: placement-ref nodes use the existing software category, never infrastructure", () => {
  const elements = continuumElements(continuumFixture());
  for (const e of elements.filter((el) => el.data.kind === "placement_ref"))
    assert.equal(e.data.visual_category, "software");
  for (const e of elements.filter((el) => el.data.kind === "dnode"))
    assert.equal(e.data.visual_category, "infrastructure");
});

test("canvas elements: every placement reference is joined to its D-Node by a dedicated placement-relation edge, never D-Call", () => {
  const elements = continuumElements(continuumFixture());
  const placementEdges = elements.filter((e) => e.data.kind === "placement");
  assert.equal(placementEdges.length, 3);
  for (const edge of placementEdges) {
    assert.equal(edge.data.visual_relation, "placement");
    assert.notEqual(edge.data.visual_relation, "logical", "must never reuse D-Call styling");
  }
});

test("canvas elements: no fake D-Node-to-D-Node edge is synthesized when no links are declared", () => {
  const f = continuumFixture();
  assert.equal(f.links.length, 0);
  const elements = continuumElements(f);
  const dnodeIds = new Set(elements.filter((e) => e.data.kind === "dnode").map((e) => e.data.id));
  const physicalEdges = elements.filter((e) => e.data.kind === "physical_link");
  assert.equal(physicalEdges.length, 0);
  const edges = elements.filter((e) => e.data.source !== undefined);
  for (const edge of edges) {
    assert.ok(
      !(dnodeIds.has(edge.data.source) && dnodeIds.has(edge.data.target)),
      "no edge should directly connect two D-Nodes when no physical link is declared",
    );
  }
});

test("canvas elements: a real declared link produces a genuine physical-relation edge between two D-Nodes", () => {
  const f = { ...continuumFixture(), links: [{ link_id: "link-1", from_node_id: "node-a", to_node_id: "node-b" }] };
  const elements = continuumElements(f);
  const physicalEdges = elements.filter((e) => e.data.kind === "physical_link");
  assert.equal(physicalEdges.length, 1);
  assert.equal(physicalEdges[0].data.visual_relation, "physical");
  assert.equal(physicalEdges[0].data.source, "node-a");
  assert.equal(physicalEdges[0].data.target, "node-b");
});

test("canvas elements: observation state overlay reflects fresh admitted evidence vs. excluded-only evidence vs. no evidence", () => {
  const elements = continuumElements(continuumFixture());
  const byId = (id) => elements.find((e) => e.data.id === id);
  const nodeAPlacementFoo = byId("placement:node-a:project-a:app-1:service-foo");
  const nodeBPlacementBaz = byId("placement:node-b:project-a:app-2:service-baz");
  assert.equal(nodeAPlacementFoo.data.observation_state, "fresh");
  assert.equal(nodeBPlacementBaz.data.observation_state, "stale", "only excluded (stale) evidence exists for this placement");
});

test("canvas elements: node-c has no placement nodes or edges, only its bare D-Node", () => {
  const elements = continuumElements(continuumFixture());
  const nodeCRelated = elements.filter((e) => e.data.id === "node-c" || e.data.source === "node-c" || e.data.target === "node-c");
  assert.equal(nodeCRelated.length, 1, "only the bare D-Node itself, no placement or edge");
});

test("canvas element computation does not mutate the input view", () => {
  const f = continuumFixture();
  const before = JSON.parse(JSON.stringify(f));
  continuumElements(f);
  assert.deepEqual(f, before);
});

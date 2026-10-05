import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { placementContinuumNodes } from "./placement-fixture.mjs";
import { placementDiff } from "../src/pages/placement-model.ts";
import { placementElements } from "../src/pages/placement.ts";

// Mandatory Phase 136 regression (independent review, Audit 2): the
// acceptance route/action must be structurally absent from every Phase 136
// frontend file, never merely "not currently called". Mirrors the
// established source-text-scan precedent in dgraph-editor.test.mjs
// ("has no mutation endpoint").
test("Phase 136 client and workspace contain no acceptance/approval/activation route or call in actual code (comments may still document what must never be called)", async () => {
  const files = [
    "../src/api/placement.ts",
    "../src/pages/placement.ts",
    "../src/pages/placement-model.ts",
    "../src/types/placement.ts",
  ];
  const forbidden = /\/accept\b|accept_proposal|acceptProposal|\bapprove\b|\bApprove\b|activate placement|\bActivate\b/u;
  for (const path of files) {
    const source = await readFile(new URL(path, import.meta.url), "utf8");
    // Strip whole-line `//` comments first -- every file here documents,
    // in prose, the exact route Phase 136 must NEVER call (so a future
    // reader/reviewer can grep for it) without ever constructing it in
    // real code. This mirrors scanning PRODUCTION code only, the same
    // discipline `dserver`'s own `production_source()` test helpers use.
    const code = source
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n");
    assert.doesNotMatch(code, forbidden, `${path}'s actual code (comments excluded) must never reference an acceptance/approval/activation action`);
  }
});
test("PlacementClient exposes exactly the five read/candidate/validate/create/get methods -- no accept method exists", async () => {
  const source = await readFile(new URL("../src/api/placement.ts", import.meta.url), "utf8");
  const methods = [...source.matchAll(/^\s*async (\w+)\(/gmu)].map((m) => m[1]);
  assert.deepEqual(methods.sort(), ["active", "candidate", "createProposal", "getProposal", "validateCandidate"].sort());
});

function diffFor(current, candidate) {
  return placementDiff(current, candidate);
}

test("placementElements: one node per D-Node plus one node per distinct service in the diff", () => {
  const nodes = placementContinuumNodes();
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [
    { service_id: "service-a", node_id: "node-b" },
    { service_id: "service-b", node_id: "node-a" },
  ];
  const elements = placementElements(nodes, diffFor(current, candidate));
  const dnodeElements = elements.filter((e) => e.data.kind === "dnode");
  const serviceElements = elements.filter((e) => e.data.kind === "placement_draft");
  assert.equal(dnodeElements.length, 2, "node-a, node-b");
  assert.equal(serviceElements.length, 2, "service-a (moved), service-b (added)");
});

test("placementElements: service nodes use the existing software category, D-Nodes use infrastructure, never a new category", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(nodes, diffFor([], [{ service_id: "service-a", node_id: "node-a" }]));
  for (const e of elements.filter((el) => el.data.kind === "placement_draft")) assert.equal(e.data.visual_category, "software");
  for (const e of elements.filter((el) => el.data.kind === "dnode")) assert.equal(e.data.visual_category, "infrastructure");
});

test("placementElements: an unchanged service reuses the existing accepted placement relation, never the candidate token", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(nodes, diffFor([{ service_id: "service-a", node_id: "node-a" }], [{ service_id: "service-a", node_id: "node-a" }]));
  const edges = elements.filter((e) => e.data.source !== undefined);
  assert.equal(edges.length, 1, "unchanged has exactly one edge, no superseded edge");
  assert.equal(edges[0].data.visual_relation, "placement");
});

test("placementElements: a moved service shows BOTH its superseded (old) edge and its new candidate edge simultaneously", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(nodes, diffFor([{ service_id: "service-a", node_id: "node-a" }], [{ service_id: "service-a", node_id: "node-b" }]));
  const edges = elements.filter((e) => e.data.source !== undefined);
  assert.equal(edges.length, 2);
  const relations = edges.map((e) => e.data.visual_relation).sort();
  assert.deepEqual(relations, ["placement_candidate", "placement_superseded"]);
  const superseded = edges.find((e) => e.data.visual_relation === "placement_superseded");
  assert.equal(superseded.data.target, "node-a", "the superseded edge must point at the OLD node");
  const candidateEdge = edges.find((e) => e.data.visual_relation === "placement_candidate");
  assert.equal(candidateEdge.data.target, "node-b", "the candidate edge must point at the NEW node");
});

test("placementElements: an added service (no prior placement) shows only the new candidate edge", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(nodes, diffFor([], [{ service_id: "service-b", node_id: "node-a" }]));
  const edges = elements.filter((e) => e.data.source !== undefined);
  assert.equal(edges.length, 1);
  assert.equal(edges[0].data.visual_relation, "placement_candidate");
});

test("placementElements: a removed service (dropped from the candidate) shows only the old superseded edge, no new edge", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(nodes, diffFor([{ service_id: "service-a", node_id: "node-a" }], []));
  const edges = elements.filter((e) => e.data.source !== undefined);
  assert.equal(edges.length, 1);
  assert.equal(edges[0].data.visual_relation, "placement_superseded");
});

test("placementElements: service draft nodes carry the placement_state overlay attribute matching the diff change", () => {
  const nodes = placementContinuumNodes();
  const current = [{ service_id: "service-a", node_id: "node-a" }];
  const candidate = [
    { service_id: "service-a", node_id: "node-b" },
    { service_id: "service-b", node_id: "node-a" },
  ];
  const elements = placementElements(nodes, diffFor(current, candidate));
  const serviceA = elements.find((e) => e.data.id === "service:service-a");
  const serviceB = elements.find((e) => e.data.id === "service:service-b");
  assert.equal(serviceA.data.placement_state, "moved");
  assert.equal(serviceB.data.placement_state, "added");
});

test("placementElements: no fake D-Node-to-D-Node edge is ever synthesized -- every edge touches a service node", () => {
  const nodes = placementContinuumNodes();
  const elements = placementElements(
    nodes,
    diffFor([{ service_id: "service-a", node_id: "node-a" }], [{ service_id: "service-a", node_id: "node-b" }]),
  );
  const dnodeIds = new Set(elements.filter((e) => e.data.kind === "dnode").map((e) => e.data.id));
  const edges = elements.filter((e) => e.data.source !== undefined);
  for (const edge of edges) {
    assert.ok(!(dnodeIds.has(edge.data.source) && dnodeIds.has(edge.data.target)), "no edge should directly connect two D-Nodes");
  }
});

test("placementElements computation does not mutate its inputs", () => {
  const nodes = placementContinuumNodes();
  const before = JSON.parse(JSON.stringify(nodes));
  const entries = diffFor([{ service_id: "service-a", node_id: "node-a" }], [{ service_id: "service-a", node_id: "node-b" }]);
  const beforeEntries = JSON.parse(JSON.stringify(entries));
  placementElements(nodes, entries);
  assert.deepEqual(nodes, before);
  assert.deepEqual(entries, beforeEntries);
});

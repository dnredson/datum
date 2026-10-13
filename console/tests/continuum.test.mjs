import test from "node:test";
import assert from "node:assert/strict";
import { continuumFixture, continuumProjectOverlayFixture } from "./continuum-fixture.mjs";
import { validContinuumView } from "../src/pages/continuum-model.ts";

test("deterministic fixture validates as a global continuum view", () => {
  assert.ok(validContinuumView(continuumFixture()));
});
test("global view must carry no project_overlay at all", () => {
  const withOverlay = { ...continuumFixture(), project_overlay: { project_id: "project-a", node_ids_in_use: [] } };
  assert.ok(!validContinuumView(withOverlay), "a global-requested response must not carry a project_overlay");
});
test("project-scoped fixture validates with its own overlay", () => {
  const f = continuumProjectOverlayFixture("project-a");
  assert.ok(validContinuumView(f, "project-a"));
});
test("project overlay for a different project than requested is rejected", () => {
  const f = continuumProjectOverlayFixture("project-a");
  assert.ok(!validContinuumView(f, "project-b"), "overlay project_id must match the requested project exactly");
});
test("project overlay is required when a project was requested", () => {
  const f = continuumFixture();
  assert.ok(!validContinuumView(f, "project-a"), "a project-scoped request missing project_overlay must be rejected");
});
test("shared node: node-a carries two distinct accepted placements from two different projects", () => {
  const f = continuumFixture();
  const nodeA = f.nodes.find((n) => n.node_id === "node-a");
  assert.equal(nodeA.accepted_placements.length, 2);
  const projects = nodeA.accepted_placements.map((p) => p.project_id).sort();
  assert.deepEqual(projects, ["project-a", "project-b"]);
  assert.ok(validContinuumView(f));
});
test("node with no placement and no observation remains structurally valid (honest empty state)", () => {
  const f = continuumFixture();
  const nodeC = f.nodes.find((n) => n.node_id === "node-c");
  assert.equal(nodeC.accepted_placements.length, 0);
  assert.equal(nodeC.observations.length, 0);
  assert.ok(validContinuumView(f));
});
test("stale observation is excluded evidence, never admitted, and carries an exclusion reason", () => {
  const f = continuumFixture();
  const nodeB = f.nodes.find((n) => n.node_id === "node-b");
  const observation = nodeB.observations[0];
  assert.equal(observation.admitted_evidence.length, 0);
  assert.equal(observation.excluded_evidence.length, 1);
  assert.equal(observation.excluded_evidence[0].freshness, "stale");
  assert.equal(observation.excluded_evidence[0].exclusion_reason, "stale_evidence");
  assert.equal(observation.placement_convergence, "unknown");
});
test("project-scoped current host resource observation validates without replacing declared capacity", () => {
  const f = continuumProjectOverlayFixture("project-a");
  const nodeA = f.nodes.find((n) => n.node_id === "node-a");
  nodeA.resource_observation = {
    resource_id: "resource:hw:abc",
    observed_at_utc: "2026-10-04T11:59:50Z",
    last_seen_utc: "2026-10-04T11:59:55Z",
    age_seconds: 10,
    cpu: { logical_cores: 4, usage_percent: 12.5 },
    memory: { total_bytes: 4294967296, used_bytes: 1073741824, available_bytes: 3221225472 },
    swap: { total_bytes: 1073741824, used_bytes: 0, available_bytes: 1073741824 },
  };
  assert.equal(nodeA.structure.capacity.cpu_cores, 8, "declared planning capacity remains a separate fact");
  assert.ok(validContinuumView(f, "project-a"));
});

test("runtime usage on admitted placement evidence validates as typed telemetry", () => {
  const f = continuumFixture();
  const evidence = f.nodes[0].observations[0].admitted_evidence[0];
  evidence.runtime_usage = {
    cpu_percent: 2.5,
    memory_used_bytes: 10485760,
    memory_limit_bytes: 1073741824,
    pids: 6,
  };
  assert.ok(validContinuumView(f));
});

test("malformed live resource values are rejected rather than coerced", () => {
  const f = continuumProjectOverlayFixture("project-a");
  f.nodes[0].resource_observation = {
    resource_id: "resource:hw:abc",
    observed_at_utc: "2026-10-04T11:59:50Z",
    last_seen_utc: "2026-10-04T11:59:55Z",
    age_seconds: 10,
    cpu: { logical_cores: 4, usage_percent: 101 },
    memory: { total_bytes: 100, used_bytes: 20, available_bytes: 80 },
    swap: { total_bytes: 0, used_bytes: 0, available_bytes: 0 },
  };
  assert.ok(!validContinuumView(f, "project-a"));
});

test("unknown enum values anywhere are rejected, never silently coerced", () => {
  const f = continuumFixture();
  f.nodes[0].observations[0].placement_convergence = "not_a_real_state";
  assert.ok(!validContinuumView(f));
});
test("missing required structural fields are rejected", () => {
  const f = continuumFixture();
  delete f.nodes[0].structure.capacity;
  assert.ok(!validContinuumView(f));
});
// Phase 140A finding -- `generated_at_utc` is real-backend-generated via
// plain `Utc::now().to_rfc3339()`, which renders as `+00:00`, never `Z`.
// The validator must accept both spellings of the same UTC instant.
test("validator accepts a +00:00-offset timestamp exactly as the real backend formatter produces, not only a Z-suffixed one", () => {
  const f = { ...continuumFixture(), generated_at_utc: "2026-10-04T12:00:00.000000+00:00" };
  assert.ok(validContinuumView(f));
});

test("malformed schema/timestamp is rejected", () => {
  assert.ok(!validContinuumView({ ...continuumFixture(), schema: "datum.continuum-view/2" }));
  assert.ok(!validContinuumView({ ...continuumFixture(), generated_at_utc: "not-a-timestamp" }));
});
test("empty global registry (no nodes at all) is a valid, honest empty response", () => {
  const empty = { ...continuumFixture(), nodes: [] };
  assert.ok(validContinuumView(empty));
});
test("no declared links is valid; a malformed link is rejected", () => {
  assert.ok(validContinuumView(continuumFixture()));
  const withBadLink = { ...continuumFixture(), links: [{ link_id: "l1" }] };
  assert.ok(!validContinuumView(withBadLink));
});

// Phase 139B Part P -- Continuum must render container-sourced observations
// with ZERO code changes: `continuum-model.ts`'s validator and
// `continuum.ts`'s renderer carry no runtime-kind field/branch at all, so a
// container producer's own, SEPARATE `monitor_instance_id` stream (its own
// process epoch, per the DIoT multi-producer precedent -- never the native
// emitter's own stream) must validate and render exactly like any other
// opaque evidence entry.
test("a second, independently-epoched monitor_instance_id (as a container producer would emit) validates unmodified alongside the native stream", () => {
  const f = continuumFixture();
  const nodeA = f.nodes.find((n) => n.node_id === "node-a");
  nodeA.observations[0].admitted_evidence.push({
    monitor_instance_id: "dmonitor-instance:node-a:container-epoch-uuid",
    freshness: "fresh",
    health: "healthy",
    authority_correlation: "exact_match",
    observed_at_utc: "2026-10-04T11:59:35Z",
    received_at_utc: "2026-10-04T11:59:36Z",
    age_seconds: 25,
  });
  assert.ok(validContinuumView(f), "a container producer's own evidence stream must validate with no schema change");
  assert.equal(nodeA.observations[0].admitted_evidence.length, 2, "both the native and container evidence entries for the same placement must coexist");
});

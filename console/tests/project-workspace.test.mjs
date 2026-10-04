import test from "node:test";
import assert from "node:assert/strict";
import { projectFixture, sharedEvidence } from "./project-fixture.mjs";
import {
  applicationPreview,
  provenanceLabel,
  projectIssues,
  PROJECT_SECTIONS,
  evidenceScope,
} from "../src/pages/project-workspace.ts";
import { OperationsClient } from "../src/api/client.ts";

test("application accepted state comes exclusively from accepted projection, not proposal/event provenance", () => {
  const data = projectFixture();
  data.known_applications.page.items[0].provenance = [
    "ddeploy_proposal",
    "event",
    "dmonitor",
  ];
  assert.equal(
    applicationPreview(data, "mqtt").accepted.dmap_id,
    "accepted-map-only",
  );
  data.accepted_deployments.value.items = [];
  assert.equal(applicationPreview(data, "mqtt").accepted, undefined);
  assert.match(
    provenanceLabel(data.known_applications.page.items[0]),
    /proposal reference.*Event history.*evidence/,
  );
});

test("DNode correlated evidence is project-local, shared or explicitly unscoped without changing alert ownership", () => {
  assert.equal(
    evidenceScope(sharedEvidence("alpha"), "alpha"),
    "project_local",
  );
  assert.equal(evidenceScope(sharedEvidence("beta"), "alpha"), "cross_project");
  assert.equal(evidenceScope(sharedEvidence("alpha"), "beta"), "cross_project");
  assert.equal(evidenceScope(sharedEvidence(null), "alpha"), "unscoped");
});

test("client accepts globally correlated latest/recovery evidence but rejects internally contradictory typed scope", async () => {
  const original = globalThis.fetch;
  try {
    const data = projectFixture();
    const alert = data.alerts.value.attention_ordered.items[0];
    alert.subject = { kind: "dnode", node_id: "fog-01" };
    alert.latest_condition_event = sharedEvidence("beta");
    alert.recovery_event = sharedEvidence("beta", "recovery-beta");
    globalThis.fetch = async () => new Response(JSON.stringify(data));
    const result = await new OperationsClient().project(
      "alpha",
      new AbortController().signal,
    );
    assert.equal(
      result.alerts.value.attention_ordered.items[0].project_id,
      "alpha",
    );
    assert.equal(
      result.alerts.value.attention_ordered.items[0].latest_condition_event
        .project_id,
      "beta",
    );
    for (const key of ["latest_condition_event", "recovery_event"]) {
      alert[key].subject = {
        kind: "other",
        project_id: "contradictory",
        entity_id: "typed-id",
        entity_kind: "stream",
      };
      await assert.rejects(
        new OperationsClient().project("alpha", new AbortController().signal),
        (e) =>
          e.code === "invalid_response" && !e.message.includes("contradictory"),
      );
      alert[key].subject = { kind: "dnode", node_id: "fog-01" };
    }
    alert.subject = {
      kind: "other",
      project_id: "beta",
      entity_id: "typed-id",
      entity_kind: "stream",
    };
    await assert.rejects(
      new OperationsClient().project("alpha", new AbortController().signal),
      (e) => e.code === "invalid_response",
    );
  } finally {
    globalThis.fetch = original;
  }
});
test("application alerts are explicit scoped loaded active records, never pending closure/history", () => {
  const data = projectFixture();
  assert.equal(applicationPreview(data, "mqtt").activeAlertRecords, 3);
  assert.equal(applicationPreview(data, "different-app").activeAlertRecords, 0);
  data.alerts = { availability: "not_configured" };
  assert.equal(applicationPreview(data, "mqtt").activeAlertRecords, null);
});
test("future workspace sections remain informational with exact phase ownership", () => {
  assert.deepEqual(
    PROJECT_SECTIONS.map((s) => s[0]),
    [
      "Overview",
      "Services",
      "Graph",
      "Continuum",
      "Placement",
      "Deploy",
      "Runtime",
      "History",
    ],
  );
  assert.equal(PROJECT_SECTIONS[1][1], "133A / 133B");
});
test("project issue collection preserves safe structured degradation and deduplicates source references", () => {
  const data = projectFixture();
  const issue = {
    source: "dmonitor",
    kind: "source_corrupt",
    code: "invalid_snapshot",
    safe_message: "Observation source cannot be projected.",
    scope: { project_id: "alpha", application_id: null },
  };
  data.temporal_quality = { availability: "corrupt", value: issue };
  data.known_applications.coverage.source_coverage.dmonitor = {
    state: "corrupt",
    issue,
  };
  assert.deepEqual(projectIssues(data), [issue]);
});
test("project client makes one GET with matching route/header and validates canonical consumed fields", async () => {
  const original = globalThis.fetch;
  const calls = [];
  const data = projectFixture();
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(data));
  };
  try {
    assert.equal(
      (
        await new OperationsClient().project(
          "alpha",
          new AbortController().signal,
        )
      ).project_id,
      "alpha",
    );
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "/api/v1/operations/projects/alpha");
    assert.equal(calls[0].init.headers["X-Datum-Project"], "alpha");
    assert.equal(calls[0].init.method, "GET");
    data.accepted_deployments.value.items[0].revision = -1;
    await assert.rejects(
      new OperationsClient().project("alpha", new AbortController().signal),
      (e) => e.code === "invalid_response",
    );
  } finally {
    globalThis.fetch = original;
  }
});
test("project client rejects wrong-scope applications/events/alerts/temporal/accepted entries safely", async () => {
  const original = globalThis.fetch;
  try {
    for (const key of [
      "known_applications",
      "recent_events",
      "alerts",
      "temporal_quality",
      "accepted_deployments",
    ]) {
      const data = projectFixture();
      const page =
        key === "known_applications"
          ? data[key].page
          : key === "alerts"
            ? data[key].value.attention_ordered
            : data[key].value;
      page.items[0].project_id = "other-project";
      globalThis.fetch = async () => new Response(JSON.stringify(data));
      await assert.rejects(
        new OperationsClient().project("alpha", new AbortController().signal),
        (e) =>
          e.code === "invalid_response" && !e.message.includes("other-project"),
      );
    }
  } finally {
    globalThis.fetch = original;
  }
});

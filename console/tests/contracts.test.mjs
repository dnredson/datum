import test from "node:test";
import assert from "node:assert/strict";
import { parseRoute, projectPath, validProjectId } from "../src/app/router.ts";
import {
  discoveryEmptyMessage,
  temporalPresentation,
  lifecycleBadges,
  availabilityLabels,
} from "../src/components/presentation.ts";
import { OperationsClient, ApiError } from "../src/api/client.ts";

test("URL routes, deep encoded identities, malformed/unknown routes", () => {
  assert.deepEqual(parseRoute("/"), { kind: "overview" });
  assert.deepEqual(parseRoute("/projects"), { kind: "projects" });
  for (const id of ["project one", "a/b", "projeto-α", "a?b#c"])
    assert.deepEqual(parseRoute(projectPath(id)), {
      kind: "project",
      projectId: id,
    });
  for (const path of [
    "/unknown",
    "/projects/%FF",
    "/projects/%20",
    "/projects/a/services",
  ])
    assert.equal(parseRoute(path).kind, "not_found");
  for (const id of ["", " padded", "a\nb", "x".repeat(257)])
    assert.equal(validProjectId(id), false);
});
test("partial empty discovery never claims canonical zero", () => {
  assert.equal(
    discoveryEmptyMessage({ completeness: "partial", source_coverage: {} }),
    "No projects are currently discoverable from the available sources.",
  );
  assert.match(
    discoveryEmptyMessage({ completeness: "complete", source_coverage: {} }),
    /references/,
  );
});
test("temporal dimensions remain orthogonal; no Down badge", () => {
  assert.deepEqual(
    temporalPresentation({
      communication_freshness: "fresh",
      observation_freshness: "stale",
      source_time_quality: "behind_or_delayed",
    }),
    [
      ["Communication", "Fresh"],
      ["Observation", "Stale"],
      ["Source time", "Behind / delayed"],
    ],
  );
  assert.deepEqual(
    temporalPresentation({
      communication_freshness: "stale",
      observation_freshness: "stale",
      source_time_quality: "aligned_or_no_discrepancy",
    })[2],
    ["Source time", "Aligned / no discrepancy"],
  );
});
test("lifecycle and source labels preserve uncertainty", () => {
  assert.match(lifecycleBadges.resolved[0], /pending closure/);
  assert.match(lifecycleBadges.closed[0], /history/);
  assert.equal(availabilityLabels.not_configured, "Not configured");
  assert.equal(availabilityLabels.corrupt, "Integrity error");
});
test("client uses GET, encoded project scope and caller abort signal", async () => {
  const original = globalThis.fetch;
  const abort = new AbortController();
  globalThis.fetch = async (url, init) => {
    assert.equal(url, "/api/v1/operations/projects/project%20one");
    assert.equal(init.method, "GET");
    assert.equal(init.headers["X-Datum-Project"], "project one");
    assert.ok(init.signal instanceof AbortSignal);
    return new Response(
      JSON.stringify({
        error: { code: "invalid_scope", message: "Safe bounded message." },
      }),
      { status: 400 },
    );
  };
  try {
    await assert.rejects(
      new OperationsClient().project("project one", abort.signal),
      (e) =>
        e instanceof ApiError &&
        e.code === "invalid_scope" &&
        e.message === "Safe bounded message.",
    );
  } finally {
    globalThis.fetch = original;
  }
});
test("client rejects HTML/oversized messages and unsupported projection contracts", async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async () =>
      new Response(
        JSON.stringify({ error: { code: "bad", message: "x".repeat(513) } }),
        { status: 500 },
      );
    await assert.rejects(
      new OperationsClient().overview(new AbortController().signal),
      (e) => e.message === "The read request failed.",
    );
    globalThis.fetch = async () =>
      new Response("<html>raw secret</html>", { status: 500 });
    await assert.rejects(
      new OperationsClient().overview(new AbortController().signal),
      (e) => !e.message.includes("secret"),
    );
    globalThis.fetch = async () => new Response("{}", { status: 200 });
    await assert.rejects(
      new OperationsClient().overview(new AbortController().signal),
      (e) => e.code === "invalid_response",
    );
  } finally {
    globalThis.fetch = original;
  }
});

test("obsolete navigation abort is forwarded and never relabeled as a connection error", async () => {
  const original = globalThis.fetch;
  const controller = new AbortController();
  let entered;
  const gate = new Promise((resolve) => {
    entered = resolve;
  });
  globalThis.fetch = (_url, init) =>
    new Promise((_resolve, reject) => {
      init.signal.addEventListener("abort", () => reject(init.signal.reason), {
        once: true,
      });
      entered();
    });
  try {
    const pending = new OperationsClient().overview(controller.signal);
    await gate;
    const reason = new DOMException("Navigation changed", "AbortError");
    controller.abort(reason);
    await assert.rejects(pending, (error) => error === reason);
  } finally {
    globalThis.fetch = original;
  }
});

// Phase 138 -- Deploy / Reconciliation Console. `pages/reconciliation.ts` is
// a DOM-building view function (like Phase 136/137's own pages) and is not
// unit-tested directly here for the same documented reason: no DOM
// implementation (happy-dom/jsdom) is available in this environment. Its
// testable surface is: route parse/build (pure, `app/router.ts`), the
// shared response validators (already covered in reconciliation-model.test.mjs),
// and source-text-scan regressions proving the governance boundaries this
// phase must never cross -- the same established precedent
// proposal-review.test.mjs/placement.test.mjs use.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { parseRoute, applicationDeployPath, projectPath, validApplicationId } from "../src/app/router.ts";

test("application deploy route parses and rebuilds for a variety of project/application id shapes", () => {
  for (const [projectId, applicationId] of [
    ["project-a", "app-1"],
    ["a/b", "application one"],
    ["a?b#c", "x".repeat(200)],
  ]) {
    const path = applicationDeployPath(projectId, applicationId);
    assert.deepEqual(parseRoute(path), { kind: "project_application_deploy", projectId, applicationId });
  }
});
test("application deploy route requires exactly /projects/:id/applications/:id/deploy -- /projects/:id/applications/:id alone is Phase 141B's own distinct application overview route, not deploy; other shapes are Not Found", () => {
  assert.equal(parseRoute("/projects/a/applications").kind, "not_found");
  assert.deepEqual(parseRoute("/projects/a/applications/app-1"), { kind: "project_application_overview", projectId: "a", applicationId: "app-1" });
  assert.equal(parseRoute("/projects/a/applications/app-1/deploy/extra").kind, "not_found");
  assert.equal(parseRoute("/projects/a/applications//deploy").kind, "not_found");
});
test("application deploy route never crashes on malformed percent-encoding -- Not Found instead", () => {
  assert.equal(parseRoute("/projects/a/applications/%E0%A4%A/deploy").kind, "not_found");
  assert.equal(parseRoute("/projects/%E0%A4%A/applications/app-1/deploy").kind, "not_found");
});
test("applicationDeployPath rejects invalid project or application identifiers", () => {
  assert.throws(() => applicationDeployPath("", "app-1"));
  assert.throws(() => applicationDeployPath("project-a", ""));
  assert.throws(() => applicationDeployPath("project-a", "bad\u0000id"));
});
test("validApplicationId is the generic opaque-identifier guard", () => {
  assert.ok(validApplicationId("app-1"));
  assert.ok(!validApplicationId(""));
});
test("application deploy path nests under the owning project's own path, never a top-level /applications collection", () => {
  assert.ok(applicationDeployPath("project-a", "app-1").startsWith(`${projectPath("project-a")}/applications/`));
});
// A proposal-review route and a deploy route must never collide for the
// same project -- distinct literal path segments (proposals/<id> vs.
// applications/<id>/deploy) guarantee this, proven directly rather than
// assumed.
test("application deploy routes never collide with proposal review routes", () => {
  assert.notEqual(parseRoute("/projects/a/applications/x/deploy").kind, "project_proposal_review");
  assert.notEqual(parseRoute("/projects/a/proposals/x").kind, "project_application_deploy");
});

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

// Part AB -- dserver must never be asked, from this workspace, to push
// anything to a node; this workspace only ever POSTs request/authorization
// state and GETs read projections. Scoped to actual code (fetch/`http`
// usage), not this test's own comments, which legitimately name these
// forbidden words to explain the boundary.
test("reconciliation.ts and api/reconciliation.ts never reference host-execution primitives", async () => {
  for (const path of ["../src/pages/reconciliation.ts", "../src/api/reconciliation.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of ["child_process", "ssh2", "Deno.Command", "docker run", "exec(", "spawn("]) {
      assert.ok(!code.includes(forbidden), `${path} must never reference '${forbidden}'`);
    }
  }
});

// Part AC -- this workspace may only ever change request/authorization
// control state, never accepted placement authority itself. It must never
// import or call the ddeploy acceptance path.
test("reconciliation.ts never imports or calls the ddeploy proposal-acceptance path", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  assert.ok(!code.includes("acceptProposal"), "reconciliation.ts must never accept a proposal -- that remains Phase 137's own, separate action");
  assert.ok(!code.includes("createProposal"), "reconciliation.ts must never create a placement proposal");
});

// Part AA -- accepting a proposal must never automatically create a
// reconciliation request. The acceptance success handler in
// proposal-review.ts must never call the Reconciliation client.
test("proposal-review.ts never imports the Reconciliation client -- acceptance never auto-creates a reconciliation request", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.ok(!code.includes("ReconciliationClient"), "proposal-review.ts must never import ReconciliationClient");
  assert.ok(!code.includes("createBatch"), "proposal-review.ts must never create a reconciliation batch as a side effect of acceptance");
});

// Part M/AD -- DMonitor interpretation stays out of this phase entirely.
// Phase 138's own command-side-only files (model/client/types for request/
// batch/attempt/authorization) must stay exactly as DMonitor-free as they
// were before Phase 139B -- Phase 139B's own DMonitor integration lives
// entirely in `pages/reconciliation.ts` (the view) plus its own separate
// `api/dmonitor-convergence.ts`/`types/dmonitor-convergence.ts`/
// `pages/dmonitor-convergence-model.ts` files, never mixed into these.
test("Phase 138's command-side-only files never reference DMonitor", async () => {
  for (const path of ["../src/pages/reconciliation-model.ts", "../src/api/reconciliation.ts", "../src/types/reconciliation.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of ["DMonitor", "dmonitor"]) {
      assert.ok(!code.includes(forbidden), `${path} must never reference '${forbidden}'`);
    }
  }
});

// Phase 139B Part 44 -- `pages/reconciliation.ts` now legitimately imports
// DMonitor convergence (the one confirmed Phase 139A gap this phase
// closes), but it must never let that data drive any COMMAND-SIDE or
// AUTHORIZATION decision: request/attempt status, button enablement, or
// authorization status must never read `convergence`/`convergenceError`.
test("reconciliation.ts never lets DMonitor convergence drive button enablement, authorization, or command-side status", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  const canActLine = code.split("\n").find((line) => line.includes("const canAct ="));
  assert.ok(canActLine, "canAct must still be computed");
  assert.ok(!canActLine.includes("convergence"), "button enablement (canAct) must never read convergence state");
  assert.ok(
    !code.includes("authorizationDisplayStatus(") || !/authorizationDisplayStatus\([^)]*convergence/u.test(code),
    "authorizationDisplayStatus must never be called with convergence data",
  );
});

// Part G1/L -- "Healthy"/"Converged" must never appear as an actual status
// VALUE (a label this phase's own enums map to); `reconciliation-model.test.mjs`'s
// "command-side result labels never claim runtime health" already proves
// this for every label this module defines. This test proves the stronger,
// textual-source claim: the string literals themselves are absent from the
// view/model source as quoted badge-label text.
test("reconciliation.ts and reconciliation-model.ts never render a literal Healthy/Converged status label", async () => {
  for (const path of ["../src/pages/reconciliation.ts", "../src/pages/reconciliation-model.ts"]) {
    const code = await sourceWithoutComments(path);
    assert.ok(!code.includes('"Healthy"') && !code.includes("'Healthy'"));
    assert.ok(!code.includes('"Converged"') && !code.includes("'Converged'"));
  }
});

// Part I -- the claim-fencing limitation must never be papered over with a
// POSITIVE exactly-once claim. The phase's own disclosure prose legitimately
// says "not ... exactly-once" (a negation) -- so this checks the negation is
// present, not that the phrase is wholly absent.
test("reconciliation.ts discloses, never claims, exactly-once execution", async () => {
  const code = (await sourceWithoutComments("../src/pages/reconciliation.ts")).toLowerCase();
  assert.ok(code.includes("not a guarantee of exactly-once execution"), "the claim-fencing limitation must be disclosed as a negation, not an affirmative claim");
  assert.ok(!code.includes("executed exactly once") && !code.includes("runs exactly once"));
});

// Part U -- status must never be color-only; every badge usage pairs a
// `tone` with a non-empty text label sourced from the model's own
// label maps, which `reconciliation-model.test.mjs` already proves are
// complete (every enum variant has a label). This test proves the VIEW
// itself never calls the raw `badge()` helper with a literal empty label.
test("reconciliation.ts never renders an empty-label status badge", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  assert.ok(!/badge\(\s*""/u.test(code));
  assert.ok(!/badge\(\s*''/u.test(code));
});

// Independent review (Audit 27): a late response from an ABANDONED batch's
// poll chain must never clobber a newly-created batch's state.
// `pages/reconciliation.ts` is not DOM-testable here, so this proves the
// actual fix structurally: every function that writes to the shared
// `batch`/`slices`/`attempts` state after an `await` must check
// `myPollToken !== pollToken` (not only the page-level `sequence`, which is
// never bumped by a same-page resubmission) before applying its result.
test("loadBatch, loadLinkedAttempts, and loadSlices all guard their shared-state writes with myPollToken, not only sequence", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  for (const fn of ["async function loadBatch(", "async function loadLinkedAttempts(", "async function loadSlices("]) {
    const start = code.indexOf(fn);
    assert.ok(start >= 0, `${fn} must exist`);
    assert.ok(code.slice(start, start + 200).includes("myPollToken: number"), `${fn} must accept myPollToken`);
    const body = code.slice(start, start + 600);
    assert.ok(/myPollToken !== pollToken/u.test(body), `${fn} must check myPollToken !== pollToken before writing shared state`);
  }
});

// Phase 139B Part U -- the convergence integration must never be, or become,
// a remediation path. `DMonitorConvergenceClient` is structurally
// read-only: its ONLY method is `evaluate`. This proves the view can never
// call anything else on it, even if someone widens the client later without
// reading its own header comment.
test("DMonitorConvergenceClient exposes exactly one method -- evaluate -- and reconciliation.ts never calls any other method on it", async () => {
  const clientSource = await sourceWithoutComments("../src/api/dmonitor-convergence.ts");
  const methodNames = [...clientSource.matchAll(/\n\s+async (\w+)\(/gu)].map((m) => m[1]);
  assert.deepEqual(methodNames, ["evaluate"], "DMonitorConvergenceClient must expose exactly one method: evaluate");

  const viewCode = await sourceWithoutComments("../src/pages/reconciliation.ts");
  const calls = [...viewCode.matchAll(/\bdmonitorConvergenceClient\.(\w+)\(/gu)].map((m) => m[1]);
  assert.ok(calls.length > 0, "reconciliation.ts must actually use the convergence client");
  for (const call of calls) {
    assert.equal(call, "evaluate", `reconciliation.ts must only ever call dmonitorConvergenceClient.evaluate(), saw .${call}(`);
  }
});

// Phase 139B independent review (Audit 54) -- Phase 139A's own audit
// confirmed node-offline vs. missing-service is not canonically
// distinguished anywhere in the current DMonitor model, so the UI must
// never claim "Node offline" (a diagnosis the backend never actually
// proves). Only honest observation-availability language is allowed.
test("reconciliation.ts never claims a node is offline -- only observation-availability language (Audit 54)", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  assert.ok(!/node offline/iu.test(code), "reconciliation.ts must never render 'node offline' -- the backend does not canonically prove this");
});

// Phase 139B independent review (Audit 45) -- convergence is a live fact
// about CURRENT runtime state (unlike a reconciliation batch's own status,
// which is genuinely terminal once settled), so polling must never
// permanently stop once Converged/Diverged -- only slow down. Proves
// `pollConvergenceOnce` calls `scheduleConvergencePoll` unconditionally
// (never gated behind `isConvergenceSettled`), and that the schedule
// function itself is what varies the delay for the settled case.
test("convergence polling never terminally stops once settled -- only its cadence slows (Audit 45)", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  const start = code.indexOf("async function pollConvergenceOnce(");
  assert.ok(start >= 0, "pollConvergenceOnce must exist");
  const end = code.indexOf("\n  }", start);
  const body = code.slice(start, end);
  assert.ok(!/if\s*\(!isConvergenceSettled\(convergence\)\)\s*scheduleConvergencePoll/u.test(body), "pollConvergenceOnce must never gate rescheduling behind isConvergenceSettled -- it must always reschedule");
  assert.ok(/scheduleConvergencePoll\(token, myToken\);/u.test(body), "pollConvergenceOnce must unconditionally call scheduleConvergencePoll");
  const scheduleStart = code.indexOf("function scheduleConvergencePoll(");
  assert.ok(scheduleStart >= 0, "scheduleConvergencePoll must exist");
  const scheduleBody = code.slice(scheduleStart, code.indexOf("\n  }", scheduleStart));
  assert.ok(scheduleBody.includes("isConvergenceSettled(convergence)"), "the slower settled cadence must be chosen inside scheduleConvergencePoll itself");
});

// Phase 139B Part V -- this phase must never wire a new event/alert
// producer, and must never call any mutating client method (createBatch /
// authorize / cancel, etc.) from convergence-handling code.
test("convergence handling in reconciliation.ts never calls a mutating reconciliation-client method or an event/alert API", async () => {
  const code = await sourceWithoutComments("../src/pages/reconciliation.ts");
  const start = code.indexOf("async function loadConvergence(");
  assert.ok(start >= 0, "loadConvergence must exist");
  const end = code.indexOf("\nasync function ", start + 1);
  const body = code.slice(start, end > start ? end : start + 2000);
  for (const forbidden of ["createBatch(", "authorize(", "cancel(", "EventSource(", "WebSocket(", "/alerts"]) {
    assert.ok(!body.includes(forbidden), `loadConvergence must never call or reference '${forbidden}'`);
  }
});

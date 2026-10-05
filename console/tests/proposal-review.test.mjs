// Phase 137 -- Proposal Review / Acceptance. `pages/proposal-review.ts` is
// entirely a DOM-building view function (like Phase 136's `placementView`)
// and is not unit-tested directly here for the same documented reason: no
// DOM implementation (happy-dom/jsdom) is available in this environment.
// Its testable surface is: route parse/build (pure, `app/router.ts`), the
// shared response validators it relies on (`placement-model.ts`, already
// covered in placement-model.test.mjs), and source-text-scan regressions
// proving the governance boundaries this phase must never cross -- the
// same established precedent `placement.test.mjs`/`dgraph-editor.test.mjs`
// use for "has no mutation endpoint"-shaped guarantees.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { parseRoute, proposalReviewPath, projectPath, validProposalId } from "../src/app/router.ts";

test("proposal review route parses and rebuilds for a variety of project/proposal id shapes", () => {
  for (const [projectId, proposalId] of [
    ["project-a", "ddeploy-proposal:p1"],
    ["a/b", "proposal one"],
    ["a?b#c", "x".repeat(200)],
  ]) {
    const path = proposalReviewPath(projectId, proposalId);
    assert.deepEqual(parseRoute(path), { kind: "project_proposal_review", projectId, proposalId });
  }
});
test("proposal review route requires exactly /projects/:id/proposals/:id -- shorter or longer paths are Not Found", () => {
  assert.equal(parseRoute("/projects/a/proposals").kind, "not_found");
  assert.equal(parseRoute("/projects/a/proposals/").kind, "not_found");
  assert.equal(parseRoute("/projects/a/proposals/p1/accept").kind, "not_found");
  assert.equal(parseRoute("/projects/a/proposals/p1/extra/segment").kind, "not_found");
});
test("proposal review route never crashes on malformed percent-encoding -- Not Found instead", () => {
  assert.equal(parseRoute("/projects/a/proposals/%E0%A4%A").kind, "not_found");
  assert.equal(parseRoute("/projects/%E0%A4%A/proposals/p1").kind, "not_found");
});
test("proposalReviewPath rejects invalid project or proposal identifiers, matching validProjectId's own bounds", () => {
  assert.throws(() => proposalReviewPath("", "p1"));
  assert.throws(() => proposalReviewPath("project-a", ""));
  assert.throws(() => proposalReviewPath("project-a", "bad\u0000id"));
});
test("validProposalId is the generic opaque-identifier guard, not a project-id-specific one -- it accepts a server-shaped proposal id", () => {
  assert.ok(validProposalId("ddeploy-proposal:p1"));
  assert.ok(!validProposalId(""));
});
test("proposal review path nests under the owning project's own path, never a top-level /proposals collection", () => {
  assert.ok(proposalReviewPath("project-a", "p1").startsWith(`${projectPath("project-a")}/proposals/`));
});

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

test("proposal-review.ts exposes no Reject/Cancel/Expire/Withdraw/Delete-as-rejection control -- proposal lifecycle is exactly Pending/Accepted", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  // Scoped to actual CONTROL construction (a button/link whose label IS
  // one of these actions, or a method call invoking one) -- not to the
  // word appearing in disclosure prose, which legitimately says e.g.
  // "There is no Reject, Cancel, Expire, or Withdraw action" and "DServer
  // ... will reject acceptance while this remains stale" (the server's
  // verb, not a client control). PlacementClient's own exhaustive
  // six-method list (placement.test.mjs) already proves no such method
  // exists on the client at all.
  assert.doesNotMatch(code, /el\("(button|a)"[^)]*"(Reject|Cancel|Expire|Withdraw|Delete)"/u);
  assert.doesNotMatch(code, /placementClient\.(reject|cancel|expire|withdraw|delete)\w*\(/iu);
});
test("proposal-review.ts has no deployment/runtime-shaped action -- no /deploy call site, no DCode client, no DMonitor client", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  // Scoped to actual call sites/constructs, not the page's own required
  // prose disclosing what acceptance does NOT do (which legitimately
  // names "reconcile"/"DCode"/"DMonitor" in plain English).
  assert.doesNotMatch(code, /\.deploy\(|\/deploy[`"]|dcodeClient|DCodeClient|dmonitorClient|DMonitorClient/u);
});
test("proposal-review.ts never claims a deployed/running/healthy/converged outcome after acceptance", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.doesNotMatch(code, /\bDeployed\b|\bRunning\b|\bHealthy\b|\bConverged\b/u);
});
test("proposal-review.ts reuses placementDiff and the shared CHANGE_LABEL/CHANGE_TONE vocabulary, never redefining diff logic", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /import \{ placementDiff \} from "\.\/placement-model\.ts";/u);
  assert.match(code, /import \{ CHANGE_LABEL, CHANGE_TONE \} from "\.\/placement\.ts";/u);
  assert.doesNotMatch(code, /function placementDiff/u);
});
test("proposal-review.ts never labels the proposal's own placement \"Candidate\" -- it is a Pending/Accepted proposal, not a draft", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.doesNotMatch(code, /"Candidate"/u);
});
test("proposal-review.ts's Accept control is literally \"Accept proposal\", never Save/Apply/Activate/Deploy/Approve", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /"Accept proposal"/u);
  assert.doesNotMatch(code, /el\("button"[^)]*"(Save|Apply|Activate|Deploy|Approve)"/u);
});
test("proposal-review.ts carries the exact no-preflight disclaimer verbatim", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /No client-side review can guarantee acceptance\. DServer revalidates authoritative state at acceptance time\./u);
});
test("proposal-review.ts's confirmation names the proposal_id and application and states all three required authority-boundary sentences", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /window\.confirm\(/u);
  assert.match(code, /This changes placement authority only\./u);
  assert.match(code, /does not deploy or prove runtime convergence\./u);
  assert.match(code, /may still fail if authoritative state has changed/u);
});
test("proposal-review.ts discloses accepted_by as a non-authenticated audit label and never claims authenticated/verified identity", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /not authenticated identity/u);
  assert.doesNotMatch(code, /Authenticated user|Signed-in operator|Verified identity/u);
});
test("proposal-review.ts never computes client-side capacity/eligibility -- no residual/eligible_node_count arithmetic", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.doesNotMatch(code, /residual_(cpu|memory)|eligible_node_count|evaluate_global_node_capacity/u);
});
test("proposal-review.ts preserves exact backend error codes/messages/findings verbatim, never a generic-prose-only replacement", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /e\.code/u);
  assert.match(code, /e\.message/u);
  assert.match(code, /e\.findings/u);
});
test("proposal-review.ts re-fetches GET /active (mandatory) after both a successful and an ambiguous-outcome acceptance attempt", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  const activeCalls = [...code.matchAll(/placementClient\.active\(/gu)].length;
  assert.ok(activeCalls >= 2, "expected active() to be called from more than one place (initial load and post-accept reconciliation)");
});
test("proposal-review.ts never implements client-side CAS/locking pretending to replace the server's own compare-and-swap", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.doesNotMatch(code, /localStorage|sessionStorage|navigator\.locks/u);
});
test("placement.ts hands off to proposal review via the real route builder, never a hand-rolled path string", async () => {
  const code = await sourceWithoutComments("../src/pages/placement.ts");
  assert.match(code, /proposalReviewPath\(/u);
});
test("proposal-review.ts links back to Placement using the real route builder, never a hand-rolled path string, and never links a nonexistent deployment route", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /\$\{projectPath\(projectId\)\}\/placement/u);
  assert.doesNotMatch(code, /\/deploy"|\/projects\/[^$]*\/deploy/u);
});
test("proposal-review.ts's diff table is wrapped in a table-responsive scroller, so a long service/node id can never force page-wide horizontal overflow at tablet widths", async () => {
  const code = await sourceWithoutComments("../src/pages/proposal-review.ts");
  assert.match(code, /el\("div", "table-responsive"\)/u);
});

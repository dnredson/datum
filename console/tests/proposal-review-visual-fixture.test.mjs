// Proves the Phase 137 Proposal Review visual fixture
// (proposal-review-visual-fixture.mjs) serves a contract-valid response
// for every scenario and route the Review page actually calls -- validated
// against the REAL Console validators, not merely eyeballed. Binds an
// ephemeral port (never the fixed manual-review port). Also proves no
// deployment/runtime-shaped route exists, mirroring Phase 136's own
// "unrelated unknown route still fails explicitly" precedent.
import test from "node:test";
import assert from "node:assert/strict";
import { createFixtureServer } from "./proposal-review-visual-fixture.mjs";
import { validProposalResponse, validAcceptResponse, validActiveResponse } from "../src/pages/placement-model.ts";

async function withServer(run) {
  const server = createFixtureServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  try {
    await run(`http://127.0.0.1:${port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

test("scenario A: pending proposal on a normal basis; accept succeeds and materializes revision 4", async () => {
  await withServer(async (baseUrl) => {
    const proposalResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-a`);
    assert.equal(proposalResponse.status, 200);
    const proposal = await proposalResponse.json();
    assert.ok(validProposalResponse(proposal, "project-a", "app-1"));
    assert.equal(proposal.state, "pending");
    assert.equal(proposal.proposal.expected_active_dmap_revision, 3);

    const activeBefore = await (await fetch(`${baseUrl}/api/v1/ddeploy/active?application_id=app-1`)).json();
    assert.ok(validActiveResponse(activeBefore, "project-a"));
    assert.equal(activeBefore.deployments[0].active_dmap.revision, 3);

    const acceptResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-a/accept`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ accepted_by: "fixture-reviewer" }),
    });
    assert.equal(acceptResponse.status, 200);
    const accept = await acceptResponse.json();
    assert.ok(validAcceptResponse(accept, "project-a", "app-1"));
    assert.equal(accept.idempotent_replay, false);
    assert.equal(accept.currently_active, true);
    assert.equal(accept.materialized_dmap.revision, 4);
  });
});

test("scenario B: pending proposal's expected basis (rev 3) is stale against the current active revision (rev 4); accept fails with a real stale error", async () => {
  await withServer(async (baseUrl) => {
    const proposal = await (await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-b`)).json();
    assert.ok(validProposalResponse(proposal, "project-a", "app-3"));
    assert.equal(proposal.proposal.expected_active_dmap_revision, 3);

    const active = await (await fetch(`${baseUrl}/api/v1/ddeploy/active?application_id=app-3`)).json();
    assert.ok(validActiveResponse(active, "project-a"));
    assert.equal(active.deployments[0].active_dmap.revision, 4);
    assert.notEqual(proposal.proposal.expected_active_dmap_revision, active.deployments[0].active_dmap.revision);

    const acceptResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-b/accept`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ accepted_by: "fixture-reviewer" }),
    });
    assert.equal(acceptResponse.status, 409);
    const body = await acceptResponse.json();
    assert.equal(body.error, "stale_active_dmap_revision");
  });
});

test("scenario C: already-accepted proposal; re-accepting replays idempotently without a new mutation", async () => {
  await withServer(async (baseUrl) => {
    const proposal = await (await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-c`)).json();
    assert.ok(validProposalResponse(proposal, "project-a", "app-4"));
    assert.equal(proposal.state, "accepted");

    const acceptResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-c/accept`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ accepted_by: "second-reviewer" }),
    });
    assert.equal(acceptResponse.status, 200);
    const accept = await acceptResponse.json();
    assert.ok(validAcceptResponse(accept, "project-a", "app-4"));
    assert.equal(accept.idempotent_replay, true);
    assert.equal(accept.currently_active, true);
    // The replay's own acceptance record reflects the ORIGINAL acceptance,
    // never a fabricated new one for the second caller.
    assert.equal(accept.acceptance.accepted_by, "original-reviewer");
  });
});

test("scenario D: first deployment for app-2 -- no active D-Map exists yet, and accepting creates revision 1", async () => {
  await withServer(async (baseUrl) => {
    const proposal = await (await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-d`)).json();
    assert.ok(validProposalResponse(proposal, "project-a", "app-2"));
    assert.equal(proposal.proposal.expected_active_dmap_revision, null);

    const active = await (await fetch(`${baseUrl}/api/v1/ddeploy/active?application_id=app-2`)).json();
    assert.ok(validActiveResponse(active, "project-a"));
    assert.equal(active.deployments.length, 0);

    const acceptResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-d/accept`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ accepted_by: "fixture-reviewer" }),
    });
    assert.equal(acceptResponse.status, 200);
    const accept = await acceptResponse.json();
    assert.ok(validAcceptResponse(accept, "project-a", "app-2"));
    assert.equal(accept.materialized_dmap.revision, 1);
  });
});

// Regression for a confirmed manual-review bug: `URL.pathname` preserves
// percent-encoding verbatim (it never decodes a segment like `%3A` back to
// `:`), and the real Console's `PlacementClient` correctly
// `encodeURIComponent`s the proposal_id path segment -- so a real browser
// request arrives here with the colon encoded. Every scenario below uses
// the browser-encoded form (`%3A`) for both GET and accept, proving the
// fixture's route matchers decode the captured segment before using it as
// a SCENARIOS key. Raw-colon coverage is retained above (scenarios A-D,
// unrelated-route test) since production itself never sends that form but
// the fixture intentionally still supports it.
const ENCODED_SCENARIOS = [
  ["ddeploy-proposal%3Ascenario-a", "app-1"],
  ["ddeploy-proposal%3Ascenario-b", "app-3"],
  ["ddeploy-proposal%3Ascenario-c", "app-4"],
  ["ddeploy-proposal%3Ascenario-d", "app-2"],
];

test("GET proposal detail resolves correctly for every scenario using the browser-style encoded id (%3A)", async () => {
  await withServer(async (baseUrl) => {
    for (const [encodedId, applicationId] of ENCODED_SCENARIOS) {
      const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/${encodedId}`);
      assert.equal(response.status, 200, encodedId);
      const body = await response.json();
      assert.ok(validProposalResponse(body, "project-a", applicationId), encodedId);
      assert.equal(body.proposal.proposal_id, decodeURIComponent(encodedId));
    }
  });
});

test("POST accept resolves the correct contract for every scenario using the browser-style encoded id (%3A)", async () => {
  await withServer(async (baseUrl) => {
    const post = (encodedId) =>
      fetch(`${baseUrl}/api/v1/ddeploy/proposals/${encodedId}/accept`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ accepted_by: "fixture-reviewer" }),
      });

    const a = await post("ddeploy-proposal%3Ascenario-a");
    assert.equal(a.status, 200);
    const aBody = await a.json();
    assert.ok(validAcceptResponse(aBody, "project-a", "app-1"));
    assert.equal(aBody.idempotent_replay, false);
    assert.equal(aBody.materialized_dmap.revision, 4);

    const b = await post("ddeploy-proposal%3Ascenario-b");
    assert.equal(b.status, 409);
    assert.equal((await b.json()).error, "stale_active_dmap_revision");

    const c = await post("ddeploy-proposal%3Ascenario-c");
    assert.equal(c.status, 200);
    const cBody = await c.json();
    assert.ok(validAcceptResponse(cBody, "project-a", "app-4"));
    assert.equal(cBody.idempotent_replay, true);

    const d = await post("ddeploy-proposal%3Ascenario-d");
    assert.equal(d.status, 200);
    const dBody = await d.json();
    assert.ok(validAcceptResponse(dBody, "project-a", "app-2"));
    assert.equal(dBody.materialized_dmap.revision, 1);
  });
});

test("an encoded but unknown proposal id still fails explicitly as proposal_not_found, never matching another scenario", async () => {
  await withServer(async (baseUrl) => {
    const getResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal%3Adoes-not-exist`);
    assert.equal(getResponse.status, 404);
    assert.equal((await getResponse.json()).error, "proposal_not_found");

    const postResponse = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal%3Adoes-not-exist/accept`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ accepted_by: "fixture-reviewer" }),
    });
    assert.equal(postResponse.status, 404);
    assert.equal((await postResponse.json()).error, "proposal_not_found");
  });
});

test("malformed percent-encoding in the proposal id segment never crashes the fixture -- deterministic proposal_not_found", async () => {
  await withServer(async (baseUrl) => {
    // "%E0%A4%A" is a truncated multi-byte UTF-8 escape -- decodeURIComponent
    // throws URIError on it. The server must still answer, not hang or 500.
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/%E0%A4%A`);
    assert.equal(response.status, 404);
    assert.equal((await response.json()).error, "proposal_not_found");
  });
});

test("the captured proposal-id segment is decoded exactly once -- a literally percent-encoded colon in the data is never double-decoded into a real scenario id", async () => {
  await withServer(async (baseUrl) => {
    // "%253A" is "%25" (a literal "%") followed by "3A" -- decoding ONCE
    // yields the literal string "ddeploy-proposal%3Ascenario-a" (a percent
    // sign followed by "3A" as DATA), which matches no real scenario key.
    // Decoding TWICE would wrongly turn it into "ddeploy-proposal:scenario-a"
    // and match scenario A -- that would be the double-decode bug this
    // test guards against.
    const response = await fetch(`${baseUrl}/api/v1/ddeploy/proposals/ddeploy-proposal%253Ascenario-a`);
    assert.equal(response.status, 404);
    assert.equal((await response.json()).error, "proposal_not_found");
  });
});

test("an unrelated or deployment/runtime-shaped route still fails explicitly, never silently returns 200", async () => {
  await withServer(async (baseUrl) => {
    for (const path of [
      "/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-a/deploy",
      "/api/v1/ddeploy/deploy",
      "/api/v1/ddeploy/proposals/ddeploy-proposal:scenario-a/reconcile",
      "/api/v1/ddeploy/proposals/unknown-id",
    ]) {
      const response = await fetch(`${baseUrl}${path}`, { method: path.endsWith("/deploy") || path.endsWith("/reconcile") ? "POST" : "GET" });
      assert.notEqual(response.status, 200);
    }
  });
});

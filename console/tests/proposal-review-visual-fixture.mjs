// TEST ONLY. Serves the real response shapes for every endpoint the Phase
// 137 Proposal Review / Acceptance page calls, for manual visual review.
// Never used by the automated test suite (not matched by tests/*.test.mjs)
// and never a production persistence/authority path. Deliberately has NO
// handler for anything deployment/runtime-shaped (no `/deploy`, no DCode
// invocation, no DMonitor mutation) -- Phase 137 owns authority only, and
// this fixture must never let a reviewer believe otherwise by accident.
//
// Four scenarios, selected by proposal_id in the path:
//   A ddeploy-proposal:scenario-a -- Pending, normal basis, accept succeeds -> rev 4.
//   B ddeploy-proposal:scenario-b -- Pending, stale basis (expected rev 3, current rev 4); accept fails 409.
//   C ddeploy-proposal:scenario-c -- Already Accepted; accept replays idempotently (200).
//   D ddeploy-proposal:scenario-d -- First deployment for app-2; no active D-Map yet.
import http from "node:http";
import { fileURLToPath } from "node:url";
import {
  scenarioAProposal,
  scenarioAActive,
  scenarioAAccept,
  scenarioBProposal,
  scenarioBActive,
  scenarioBAcceptError,
  scenarioCProposal,
  scenarioCActive,
  scenarioCAccept,
  scenarioDProposal,
  scenarioDActive,
  scenarioDAccept,
} from "./proposal-review-fixture.mjs";

const send = (res, status, body) => {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
};

const SCENARIOS = {
  "ddeploy-proposal:scenario-a": { proposal: scenarioAProposal, active: scenarioAActive, accept: () => ({ status: 200, body: scenarioAAccept() }) },
  "ddeploy-proposal:scenario-b": { proposal: scenarioBProposal, active: scenarioBActive, accept: () => ({ status: 409, body: scenarioBAcceptError() }) },
  "ddeploy-proposal:scenario-c": { proposal: scenarioCProposal, active: scenarioCActive, accept: () => ({ status: 200, body: scenarioCAccept() }) },
  "ddeploy-proposal:scenario-d": { proposal: scenarioDProposal, active: scenarioDActive, accept: () => ({ status: 200, body: scenarioDAccept() }) },
};

// `URL.pathname` preserves percent-encoding verbatim -- it does NOT decode
// a segment like `%3A` back to `:`. The real Console's `PlacementClient`
// correctly `encodeURIComponent`s the proposal_id path segment (the
// production behavior is correct and is NOT changed by this fix), so a
// real browser request for "ddeploy-proposal:scenario-c" arrives here as
// ".../ddeploy-proposal%3Ascenario-c" -- this fixture's route matchers
// must decode that captured segment exactly once before using it as a
// SCENARIOS key, or every real browser request 404s even though the
// fixture's own synthetic (un-encoded) contract tests stay green. Decode
// ONLY the captured proposal-id segment, never the whole pathname (an
// encoded "/" inside an id, if one ever existed, must stay a boundary).
function decodeProposalId(encoded) {
  try {
    return decodeURIComponent(encoded);
  } catch {
    return null;
  }
}

function readBody(req) {
  return new Promise((resolve) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => resolve(raw));
  });
}

function activeByApplication(applicationId) {
  for (const scenario of Object.values(SCENARIOS)) {
    const active = scenario.active();
    const deployment = active.deployments.find((d) => d.application_id === applicationId);
    if (deployment) return { project_id: active.project_id, deployments: [deployment] };
  }
  return { project_id: "project-a", deployments: [] };
}

export function createFixtureServer() {
  return http.createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const path = url.pathname;

    const acceptMatch = path.match(/^\/api\/v1\/ddeploy\/proposals\/([^/]+)\/accept$/u);
    if (req.method === "POST" && acceptMatch) {
      await readBody(req);
      const proposalId = decodeProposalId(acceptMatch[1]);
      // Malformed percent-encoding is fixture-safe (never a crash) and
      // deterministic: it reports as "not found", exactly like any other
      // id this fixture has no scenario for -- no new error shape.
      const scenario = proposalId === null ? undefined : SCENARIOS[proposalId];
      if (!scenario) {
        send(res, 404, { error: "proposal_not_found", message: "No such TEST ONLY fixture proposal." });
        return;
      }
      const { status, body } = scenario.accept();
      send(res, status, body);
      return;
    }
    const getMatch = path.match(/^\/api\/v1\/ddeploy\/proposals\/([^/]+)$/u);
    if (req.method === "GET" && getMatch) {
      const proposalId = decodeProposalId(getMatch[1]);
      const scenario = proposalId === null ? undefined : SCENARIOS[proposalId];
      if (!scenario) {
        send(res, 404, { error: "proposal_not_found", message: "No such TEST ONLY fixture proposal." });
        return;
      }
      send(res, 200, scenario.proposal());
      return;
    }
    if (req.method === "GET" && path === "/api/v1/ddeploy/active") {
      const applicationId = url.searchParams.get("application_id");
      send(res, 200, activeByApplication(applicationId ?? "app-1"));
      return;
    }
    send(res, 404, { error: "fixture_not_found" });
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const port = Number(process.env.DATUM_PROPOSAL_REVIEW_FIXTURE_PORT ?? 8139);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid TEST ONLY fixture port.");
  createFixtureServer().listen(port, "127.0.0.1", () =>
    console.log(
      `TEST ONLY Phase137 Proposal Review fixture on http://127.0.0.1:${port}; no server persistence or authority. ` +
        "Scenarios: scenario-a (normal accept), scenario-b (stale), scenario-c (already accepted/idempotent replay), scenario-d (first deployment).",
    ),
  );
}

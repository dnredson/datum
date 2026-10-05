// TEST ONLY. Serves the real response shapes for every endpoint the
// Phase 138 Deploy/Reconciliation workspace calls, for manual visual
// review -- ddeploy active (accepted authority), operational-dgraph-slice
// (target nodes/services/authorization), operational-reconciliation/
// authorize, and reconciliation-requests/batch + reconciliation-attempts.
// Never used by the automated test suite (not matched by tests/*.test.mjs)
// and never a production persistence/authority path.
//
// Scenarios are selected by `application_id` (see SCENARIOS below). Batch
// status PROGRESSES across repeated GETs of the same
// reconciliation_batch_id (a small in-memory poll counter, reset on
// restart) so a human watching the Console can actually see
// Pending -> Claimed -> Attempted the way a real agent poll cycle would
// produce it.
import http from "node:http";
import { fileURLToPath } from "node:url";
import {
  activeResponseFixture,
  operationalDGraphSliceFixture,
  authorizationFixture,
  reconciliationRequestFixture,
  reconciliationAttemptRecordFixture,
} from "./reconciliation-fixture.mjs";
import { evidenceFixture, servicePlacementFixture, applicationConvergenceFixture, evaluateConvergenceResponseFixture } from "./dmonitor-convergence-fixture.mjs";

const send = (res, status, body) => {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
};

function readBody(req) {
  return new Promise((resolve) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => resolve(raw));
  });
}

// Each scenario names its own node(s), and a `steps` array: step[0] is what
// a batch looks like immediately after creation, step[N] is what the SAME
// batch looks like after N more polls (held at the last step once reached).
// A step is `{ perNode: { [node_id]: { status, attemptStatus? } } }`.
const SCENARIOS = {
  // A. Preview, Pending / waiting for agent -- never advances (the fixture
  // agent never claims it), so the operator can observe the waiting state.
  "app-preview-waiting": {
    nodeIds: ["node-a"],
    mode: "preview",
    steps: [{ perNode: { "node-a": { status: "pending" } } }],
  },
  // B. Preview completed, no mutations -- Attempted with every service NoOp.
  "app-preview-done": {
    nodeIds: ["node-a"],
    mode: "preview",
    steps: [
      { perNode: { "node-a": { status: "pending" } } },
      { perNode: { "node-a": { status: "claimed" } } },
      { perNode: { "node-a": { status: "attempted", attemptStatus: "succeeded", noMutation: true } } },
    ],
  },
  // C. Execute, Authorized + Succeeded, single node. Its own node id (never
  // shared with another scenario) so the slice fixture's authorization
  // state is unambiguous.
  "app-authorized-succeeded": {
    nodeIds: ["node-auth-c"],
    mode: "execute",
    authorized: true,
    steps: [
      { perNode: { "node-auth-c": { status: "pending" } } },
      { perNode: { "node-auth-c": { status: "claimed" } } },
      { perNode: { "node-auth-c": { status: "attempted", attemptStatus: "succeeded" } } },
    ],
  },
  // D. Execute, Partial across 3 nodes -- fog-01/cloud-01 succeed, rpi-01 fails.
  "app-partial-3node": {
    nodeIds: ["fog-01", "cloud-01", "rpi-01"],
    mode: "execute",
    authorized: true,
    steps: [
      { perNode: { "fog-01": { status: "pending" }, "cloud-01": { status: "pending" }, "rpi-01": { status: "pending" } } },
      { perNode: { "fog-01": { status: "claimed" }, "cloud-01": { status: "claimed" }, "rpi-01": { status: "claimed" } } },
      {
        perNode: {
          "fog-01": { status: "attempted", attemptStatus: "succeeded" },
          "cloud-01": { status: "attempted", attemptStatus: "succeeded" },
          "rpi-01": { status: "attempted", attemptStatus: "failed" },
        },
      },
    ],
  },
  // E. Stale request after authority changed -- goes Stale instead of
  // progressing past Claimed.
  "app-stale": {
    nodeIds: ["node-a"],
    mode: "execute",
    steps: [{ perNode: { "node-a": { status: "pending" } } }, { perNode: { "node-a": { status: "stale" } } }],
  },
  // F. Queue full -- batch CREATION itself is refused (see
  // handleBatchCreate below), never reaches a polling step.
  "app-queue-full": { nodeIds: ["node-a"], mode: "execute", createBehavior: "queue_full" },
  // G. Agent offline -- never advances past Pending, and the request's own
  // TTL is short in this fixture's narration (shown via the request's
  // `expires_at_utc`, already in the past) so Console can exercise the
  // Expired display without a real clock wait.
  "app-offline-expired": {
    nodeIds: ["node-a"],
    mode: "execute",
    steps: [{ perNode: { "node-a": { status: "expired" } } }],
  },
  // H. First deployment -- accepted authority exists but this is its first
  // ever reconciliation (no prior attempt history to contrast against). Own
  // node id so its authorization state is unambiguous.
  "app-first-deploy": {
    nodeIds: ["node-auth-h"],
    mode: "execute",
    authorized: true,
    steps: [
      { perNode: { "node-auth-h": { status: "pending" } } },
      { perNode: { "node-auth-h": { status: "attempted", attemptStatus: "succeeded" } } },
    ],
  },
  // I. Cross-application node-wide disclosure -- node-a's operational slice
  // (see operationalDGraphSliceFixture) already includes a service
  // belonging to app-2; this scenario's own application is app-1, so the
  // Console's Part H2 disclosure should visibly flag it.
  "app-cross-app": {
    nodeIds: ["node-a"],
    mode: "preview",
    steps: [{ perNode: { "node-a": { status: "attempted", attemptStatus: "succeeded", noMutation: true } } }],
  },
  // K. Command-side Succeeded, but the independent runtime observation has
  // since Diverged (e.g. authority moved on, or the runtime was observed
  // elsewhere) -- Phase 139B independent review Audit 47: command result
  // and observation must never be collapsed into one combined status, so
  // this exact "Succeeded command + Diverged observation" pairing must
  // render without contradiction. Own node id so its authorization state is
  // unambiguous.
  "app-succeeded-but-diverged": {
    nodeIds: ["node-auth-k"],
    mode: "execute",
    authorized: true,
    steps: [
      { perNode: { "node-auth-k": { status: "pending" } } },
      { perNode: { "node-auth-k": { status: "attempted", attemptStatus: "succeeded" } } },
    ],
  },
  // J. Unknown attempt result. Own node id so its authorization state is
  // unambiguous.
  "app-unknown-result": {
    nodeIds: ["node-auth-j"],
    mode: "execute",
    authorized: true,
    steps: [{ perNode: { "node-auth-j": { status: "attempted", attemptStatus: "unknown" } } }],
  },
};

// batch_id -> { applicationId, pollCount }
const batches = new Map();
let nextBatchSuffix = 1;

function scenarioFor(applicationId) {
  return SCENARIOS[applicationId];
}

function stepFor(scenario, pollCount) {
  const index = Math.min(pollCount, scenario.steps.length - 1);
  return scenario.steps[index];
}

// attempt_id -> { batchId, nodeId } -- a direct index, never re-derived by
// parsing the id string back apart (both ids may themselves legitimately
// contain colons, e.g. `reconciliation-batch:<application_id>:<n>`, which
// would make naive colon-splitting ambiguous).
const attemptIndex = new Map();

function requestsForStep(scenario, batchId, step) {
  return scenario.nodeIds.map((nodeId) => {
    const nodeStep = step.perNode[nodeId] ?? { status: "pending" };
    let attemptId = null;
    if (nodeStep.attemptStatus) {
      attemptId = `reconciliation-attempt:${batchId.replaceAll(":", "-")}-${nodeId}`;
      attemptIndex.set(attemptId, { batchId, nodeId });
    }
    return reconciliationRequestFixture(nodeId, {
      reconciliation_batch_id: batchId,
      mode: scenario.mode,
      status: nodeStep.status,
      linked_attempt_id: attemptId,
      ...(nodeStep.status === "expired" ? { expires_at_utc: "2020-01-01T00:00:00Z" } : {}),
    });
  });
}

function batchStatusForStep(step) {
  const statuses = Object.values(step.perNode).map((n) => n.status);
  if (statuses.some((s) => s === "pending" || s === "claimed")) return "executing";
  if (statuses.some((s) => s === "stale")) return "stale";
  if (statuses.every((s) => s === "expired")) return "unknown";
  const attemptStatuses = Object.values(step.perNode).map((n) => n.attemptStatus).filter(Boolean);
  if (attemptStatuses.every((s) => s === "succeeded")) return "succeeded";
  if (attemptStatuses.includes("failed")) return "partial";
  if (attemptStatuses.includes("unknown")) return "unknown";
  return "unknown";
}

function findAttempt(attemptId, batchId, nodeId, scenario, step) {
  const nodeStep = step.perNode[nodeId];
  if (!nodeStep?.attemptStatus) return undefined;
  const services = nodeStep.noMutation
    ? { [serviceForNode(nodeId)]: { service_id: serviceForNode(nodeId), result: "no_op", host_mutation_performed: false, execution_status: "already_running" } }
    : undefined;
  return reconciliationAttemptRecordFixture(attemptId, nodeId, {
    status: nodeStep.attemptStatus,
    reconciliation_request_id: `reconciliation-request:${nodeId}`,
    ...(services ? { services } : {}),
  });
}
function serviceForNode(nodeId) {
  return nodeId === "node-b" ? "service-b" : "service-a";
}

export function createFixtureServer() {
  return http.createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const path = url.pathname;

    if (req.method === "GET" && path === "/api/v1/ddeploy/active") {
      const applicationId = url.searchParams.get("application_id") ?? "app-1";
      const scenario = scenarioFor(applicationId);
      if (!scenario) {
        send(res, 200, { project_id: "project-a", deployments: [] });
        return;
      }
      const active = activeResponseFixture(applicationId);
      active.deployments[0].active_dmap.placements = scenario.nodeIds.map((nodeId) => ({ service_id: serviceForNode(nodeId), node_id: nodeId }));
      send(res, 200, active);
      return;
    }

    const sliceMatch = path.match(/^\/api\/v1\/nodes\/([^/]+)\/operational-dgraph-slice$/u);
    if (req.method === "GET" && sliceMatch) {
      const nodeId = decodeURIComponent(sliceMatch[1]);
      const applicationId = [...Object.keys(SCENARIOS)].find((id) => SCENARIOS[id].nodeIds.includes(nodeId));
      const scenario = applicationId ? SCENARIOS[applicationId] : undefined;
      const authorization = scenario?.authorized ? authorizationFixture(nodeId) : null;
      send(res, 200, operationalDGraphSliceFixture(nodeId, { authorization }));
      return;
    }

    const authorizeMatch = path.match(/^\/api\/v1\/nodes\/([^/]+)\/operational-reconciliation\/authorize$/u);
    if (req.method === "POST" && authorizeMatch) {
      const nodeId = decodeURIComponent(authorizeMatch[1]);
      send(res, 200, authorizationFixture(nodeId));
      return;
    }

    if (req.method === "POST" && path === "/api/v1/reconciliation-requests/batch") {
      const raw = await readBody(req);
      let payload;
      try {
        payload = JSON.parse(raw);
      } catch {
        send(res, 400, { error: "malformed JSON body", code: "invalid_request" });
        return;
      }
      const scenario = scenarioFor(payload.application_id);
      if (!scenario) {
        send(res, 404, { error: `application '${payload.application_id}' has no accepted placements.`, code: "unknown_application" });
        return;
      }
      if (scenario.createBehavior === "queue_full") {
        send(res, 409, { error: "node 'node-a' already has the maximum of 20 pending/claimed reconciliation requests; wait for one to resolve before creating another", code: "reconciliation_queue_full" });
        return;
      }
      const batchId = `reconciliation-batch:${payload.application_id}:${nextBatchSuffix++}`;
      batches.set(batchId, { applicationId: payload.application_id, pollCount: 0 });
      const step = stepFor(scenario, 0);
      send(res, 200, { reconciliation_batch_id: batchId, application_id: payload.application_id, requests: requestsForStep(scenario, batchId, step) });
      return;
    }

    const batchGetMatch = path.match(/^\/api\/v1\/reconciliation-requests\/batch\/([^/]+)$/u);
    if (req.method === "GET" && batchGetMatch) {
      const batchId = decodeURIComponent(batchGetMatch[1]);
      const tracked = batches.get(batchId);
      if (!tracked) {
        send(res, 404, { error: `reconciliation_batch_id '${batchId}' not found`, code: "not_found" });
        return;
      }
      const scenario = scenarioFor(tracked.applicationId);
      const step = stepFor(scenario, tracked.pollCount);
      if (tracked.pollCount < scenario.steps.length - 1) tracked.pollCount += 1;
      send(res, 200, {
        reconciliation_batch_id: batchId,
        project_id: "project-a",
        status: batchStatusForStep(step),
        requests: requestsForStep(scenario, batchId, step),
      });
      return;
    }

    const attemptMatch = path.match(/^\/api\/v1\/reconciliation-attempts\/([^/]+)$/u);
    if (req.method === "GET" && attemptMatch) {
      const attemptId = decodeURIComponent(attemptMatch[1]);
      const indexed = attemptIndex.get(attemptId);
      const tracked = indexed ? batches.get(indexed.batchId) : undefined;
      if (!indexed || !tracked) {
        send(res, 404, { error: `reconciliation_attempt_id '${attemptId}' not found`, code: "not_found" });
        return;
      }
      const scenario = scenarioFor(tracked.applicationId);
      // The attempt itself is immutable once it exists (Phase 138A: exactly
      // one terminal event per cycle) -- always read it from the step that
      // actually PRODUCED it (the batch's current poll count may since have
      // advanced further for other, still-polling members), never
      // re-derived from a later step.
      const step = stepFor(scenario, scenario.steps.findIndex((candidate) => candidate.perNode[indexed.nodeId]?.attemptStatus));
      const attempt = findAttempt(attemptId, indexed.batchId, indexed.nodeId, scenario, step);
      if (!attempt) {
        send(res, 404, { error: `reconciliation_attempt_id '${attemptId}' not found`, code: "not_found" });
        return;
      }
      send(res, 200, attempt);
      return;
    }

    const convergenceMatch = path.match(/^\/api\/v1\/dmonitor\/convergence\/([^/]+)\/([^/]+)\/evaluate$/u);
    if (req.method === "POST" && convergenceMatch) {
      const applicationId = decodeURIComponent(convergenceMatch[2]);
      send(res, 200, convergenceResponseFor(applicationId));
      return;
    }

    send(res, 404, { error: "fixture_not_found" });
  });
}

// Phase 139B (Part Y): deterministic convergence scenarios, selected by the
// SAME `application_id` the Phase 138 batch/active fixtures above already
// use where a natural pairing exists (so the deploy->observe handoff is
// visible on one application), plus a few convergence-only scenario ids for
// cases with no Phase 138 counterpart. Any application_id not explicitly
// listed gets the honest default: zero evidence yet (scenario A, "waiting
// for observation").
function convergenceResponseFor(applicationId) {
  switch (applicationId) {
    // B. Native Converged (reuses Phase 138's own authorized-succeeded node).
    case "app-authorized-succeeded":
      return evaluateConvergenceResponseFixture(applicationId, {
        services: [servicePlacementFixture("service-a", { desired_node_id: "node-auth-c" })],
      });
    // C. Container Converged.
    case "app-converged-container":
      return evaluateConvergenceResponseFixture(applicationId, {
        services: [servicePlacementFixture("container-service", { desired_node_id: "fog-01" })],
      });
    // D. Diverged / wrong authority ref -- admitted evidence exists but its
    // own dmap_ref does not match current authority.
    case "app-diverged-container":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "diverged",
        findings: [{ code: "authority_mismatch", message: "container-service observed with a dmap_ref that does not match current active authority" }],
        services: [
          servicePlacementFixture("container-service", {
            desired_node_id: "fog-01",
            convergence: "diverged",
            findings: [{ code: "authority_mismatch", message: "observed dmap_ref does not match current active authority" }],
            admitted_evidence: [evidenceFixture({ authority_correlation: "mismatch" })],
          }),
        ],
      });
    // E. Stopped/unhealthy observation -- still CONVERGED (the placement
    // identity is exactly right), health is Unhealthy. Deliberately proves
    // "Converged != Healthy" for containers exactly as it already holds for
    // native (Phase 139A's own confirmed, unchanged semantics).
    case "app-unhealthy-observed":
      return evaluateConvergenceResponseFixture(applicationId, {
        services: [
          servicePlacementFixture("container-service", {
            desired_node_id: "fog-01",
            convergence: "converged",
            admitted_evidence: [evidenceFixture({ health: "unhealthy" })],
            observed_health_claims: [{ monitor_instance_id: "dmonitor-instance:fog-01:fixed", health: "unhealthy" }],
          }),
        ],
      });
    // F. Stale observation -- evidence exists but aged out; cannot prove
    // CURRENT convergence (never silently Diverged either).
    case "app-stale-observation":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "unknown",
        findings: [{ code: "stale_evidence", message: "the only relevant evidence for service-a is stale" }],
        services: [
          servicePlacementFixture("service-a", {
            convergence: "unknown",
            findings: [{ code: "stale_evidence", message: "evidence is older than the configured freshness policy allows" }],
            admitted_evidence: [],
            excluded_evidence: [evidenceFixture({ freshness: "stale", admitted: false, exclusion_reason: "stale_evidence" })],
          }),
        ],
      });
    // G. ClockSkew -- the node's own reported observed_at_utc is ahead of
    // when dserver received it by more than the policy allows.
    case "app-clock-skew":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "unknown",
        findings: [{ code: "clock_skew", message: "service-a evidence observed_at_utc is ahead of received_at_utc beyond tolerance" }],
        services: [
          servicePlacementFixture("service-a", {
            convergence: "unknown",
            findings: [{ code: "clock_skew", message: "clock skew exceeds policy tolerance" }],
            admitted_evidence: [],
            excluded_evidence: [evidenceFixture({ freshness: "clock_skew", admitted: false, exclusion_reason: "clock_skew" })],
          }),
        ],
      });
    // H. Unknown / no trustworthy evidence at all (reuses Phase 138's own
    // unknown-attempt-result application).
    case "app-unknown-result":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "unknown",
        findings: [{ code: "no_evidence", message: "no DMonitor evidence has been received for service-a yet" }],
        services: [servicePlacementFixture("service-a", { convergence: "unknown", findings: [{ code: "no_evidence", message: "no evidence" }], admitted_evidence: [], excluded_evidence: [] })],
      });
    // I. Command Partial (one node Failed command-side) but that SAME
    // node's runtime remains independently OBSERVED Converged+Healthy --
    // proves command-side and observation are genuinely independent
    // (Part T). Reuses Phase 138's own 3-node partial-convergence batch.
    case "app-partial-3node":
      return evaluateConvergenceResponseFixture(applicationId, {
        services: [
          servicePlacementFixture("service-a", { desired_node_id: "fog-01" }),
          servicePlacementFixture("service-a", { desired_node_id: "cloud-01" }),
          servicePlacementFixture("service-a", { desired_node_id: "rpi-01" }),
        ],
      });
    // J. Superseded authority -- observation still carries the OLD
    // (rev 10) dmap_ref while current active authority has since moved to
    // rev 11; AuthorityCorrelationV1 must prevent false convergence.
    case "app-superseded-authority":
      return evaluateConvergenceResponseFixture(applicationId, {
        dmap_ref: { dmap_id: "dmap:app-superseded-authority", revision: 11, digest: `sha256:${"c".repeat(64)}` },
        convergence: "diverged",
        findings: [{ code: "authority_mismatch", message: "service-a evidence references dmap revision 10; current active authority is revision 11" }],
        services: [
          servicePlacementFixture("service-a", {
            convergence: "diverged",
            findings: [{ code: "authority_mismatch", message: "evidence dmap_ref revision 10 does not match current active authority revision 11" }],
            admitted_evidence: [evidenceFixture({ authority_correlation: "mismatch" })],
          }),
        ],
      });
    // K. Command Succeeded, observation Diverged -- the command attempt
    // itself truthfully reported success, but the runtime has since been
    // independently observed elsewhere (e.g. a later, out-of-band change).
    // Proves Audit 47's "Command Succeeded + Diverged" combination renders
    // without contradiction or silent reconciliation between the two
    // dimensions.
    case "app-succeeded-but-diverged":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "diverged",
        findings: [{ code: "wrong_node", message: "service-a observed on a node other than its desired placement" }],
        services: [
          servicePlacementFixture("service-a", {
            desired_node_id: "node-auth-k",
            convergence: "diverged",
            findings: [{ code: "wrong_node", message: "service-a observed on a node other than its desired placement" }],
            admitted_evidence: [evidenceFixture({ node_id: "node-b", observed_subject_node_id: "node-b" })],
          }),
        ],
      });
    // L. Multi-node partial convergence -- one service converged, a
    // different service on a different node diverged, in the SAME
    // application.
    case "app-multi-node-partial":
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "diverged",
        services: [
          servicePlacementFixture("service-a", { desired_node_id: "fog-01", convergence: "converged" }),
          servicePlacementFixture("service-b", {
            desired_node_id: "cloud-01",
            convergence: "diverged",
            findings: [{ code: "wrong_node", message: "service-b observed on a node other than its desired placement" }],
            admitted_evidence: [evidenceFixture({ node_id: "rpi-01", observed_subject_node_id: "rpi-01" })],
          }),
        ],
      });
    // A (default) -- command may have run or not; no trustworthy evidence
    // has arrived yet. Zero admitted AND zero excluded evidence -- never
    // fabricated as Diverged.
    default:
      return evaluateConvergenceResponseFixture(applicationId, {
        convergence: "unknown",
        findings: [{ code: "no_evidence", message: "no DMonitor evidence has been received yet" }],
        services: [servicePlacementFixture("service-a", { convergence: "unknown", findings: [{ code: "no_evidence", message: "no evidence" }], admitted_evidence: [], excluded_evidence: [] })],
      });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const port = Number(process.env.DATUM_RECONCILIATION_FIXTURE_PORT ?? 8138);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid TEST ONLY fixture port.");
  const convergenceOnlyScenarios = [
    "app-converged-container",
    "app-diverged-container",
    "app-unhealthy-observed",
    "app-stale-observation",
    "app-clock-skew",
    "app-superseded-authority",
    "app-multi-node-partial",
  ];
  createFixtureServer().listen(port, "127.0.0.1", () =>
    console.log(
      `TEST ONLY Phase 138/139B Deploy/Reconciliation + runtime-observation fixture on http://127.0.0.1:${port}; no server persistence or authority. ` +
        `Request/batch scenarios (open /projects/project-a/applications/<id>/deploy): ${Object.keys(SCENARIOS).join(", ")}. ` +
        `Convergence-only scenarios (same route, no batch needed): ${convergenceOnlyScenarios.join(", ")}.`,
    ),
  );
}

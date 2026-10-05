// TEST ONLY. Serves the real response shapes for every endpoint the Phase
// 136 Placement Workspace calls, for manual visual review -- dgraph-editor
// (accepted D-Graph read), operations/continuum (physical D-Node
// inventory), and ddeploy active/candidate/validate/proposals. Never used
// by the automated test suite (not matched by tests/*.test.mjs) and never
// a production persistence/authority path. Deterministic: service-a is
// currently accepted on node-a; the candidate moves it to node-b and
// newly places service-b on node-a.
import http from "node:http";
import { fileURLToPath } from "node:url";
import {
  placementContinuumNodes,
  placementDGraphFixture,
  activeResponseFixture,
  candidateResponseFixture,
  validateResponseFixture,
  proposalResponseFixture,
} from "./placement-fixture.mjs";

const send = (res, status, body) => {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
};

export function dgraphEditorBaseFixture() {
  const graph = placementDGraphFixture();
  return {
    schema: "datum.dgraph-editor-base/1",
    project_id: "project-a",
    graphs: [
      {
        application_id: graph.application_id,
        dgraph_id: graph.dgraph_id,
        revision: String(graph.revision),
        digest: `sha256:${"a".repeat(64)}`,
        graph_json: JSON.stringify(graph),
      },
    ],
  };
}

export function continuumViewFixture() {
  return {
    schema: "datum.continuum-view/1",
    generated_at_utc: "2026-10-04T12:00:00Z",
    dnode_registry_revision: 2,
    nodes: placementContinuumNodes(),
    links: [],
    project_overlay: { project_id: "project-a", node_ids_in_use: ["node-a"] },
    truncated: false,
  };
}

function readBody(req) {
  return new Promise((resolve) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => resolve(raw));
  });
}

export function createFixtureServer() {
  return http.createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const path = url.pathname;
    const projectHeader = req.headers["x-datum-project"];

    if (req.method === "GET" && path === "/api/v1/operations/projects/project-a/dgraph-editor") {
      send(res, 200, dgraphEditorBaseFixture());
      return;
    }
    if (req.method === "GET" && path === "/api/v1/operations/projects/project-a/continuum") {
      send(res, 200, continuumViewFixture());
      return;
    }
    if (req.method === "GET" && path === "/api/v1/ddeploy/active") {
      if (projectHeader !== "project-a") {
        send(res, 400, { error: "bad_request", message: "missing or invalid X-Datum-Project header" });
        return;
      }
      send(res, 200, activeResponseFixture());
      return;
    }
    if (req.method === "POST" && path === "/api/v1/ddeploy/candidate") {
      await readBody(req);
      send(res, 200, candidateResponseFixture());
      return;
    }
    if (req.method === "POST" && path === "/api/v1/ddeploy/proposals/validate") {
      await readBody(req);
      send(res, 200, validateResponseFixture());
      return;
    }
    if (req.method === "POST" && path === "/api/v1/ddeploy/proposals") {
      await readBody(req);
      send(res, 201, proposalResponseFixture());
      return;
    }
    const proposalGet = path.match(/^\/api\/v1\/ddeploy\/proposals\/([^/]+)$/u);
    if (req.method === "GET" && proposalGet) {
      send(res, 200, proposalResponseFixture());
      return;
    }
    send(res, 404, { error: "fixture_not_found" });
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const port = Number(process.env.DATUM_PLACEMENT_FIXTURE_PORT ?? 8138);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid TEST ONLY fixture port.");
  createFixtureServer().listen(port, "127.0.0.1", () =>
    console.log(
      `TEST ONLY Phase136 Placement Workspace fixture on http://127.0.0.1:${port}; no server persistence or authority. ` +
        "app-1: service-a currently on node-a, candidate moves it to node-b; service-b newly placed on node-a.",
    ),
  );
}

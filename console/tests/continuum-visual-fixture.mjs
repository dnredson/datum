// TEST ONLY. Serves the real datum.continuum-view/1 contract shape at the
// production Continuum routes, for manual visual review of the Continuum
// dashboard -- PLUS minimal, honest "not configured"/empty responses for
// the handful of other read-only shell endpoints the Console requests while
// navigating (Overview, Projects, Catalog), so an integrated manual review
// session does not show misleading "Disconnected"/"fixture_not_found"
// noise on unrelated pages. This remains a narrow, deterministic fixture,
// never a general-purpose fake DServer: every other route still fails
// explicitly. Never used by the automated test suite (not matched by
// tests/*.test.mjs) and never a production persistence/authority path.
import http from "node:http";
import { fileURLToPath } from "node:url";
import { continuumFixture, continuumProjectOverlayFixture } from "./continuum-fixture.mjs";

const send = (res, status, body) => {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
};

// Minimal valid `GlobalOverview` (console/src/types/operations.ts), honest
// empty/not-configured throughout -- no fabricated project, event, alert or
// deployment data. Served for BOTH /overview and /projects: main.ts renders
// the same `client.overview()` response via `overviewView()` or
// `projectsView()` depending on route, so one fixture response covers both.
export function minimalGlobalOverviewFixture() {
  const emptyDiscovery = () => ({
    page: { items: [], has_more: false, limit: 100 },
    coverage: { completeness: "complete", source_coverage: {} },
  });
  const notConfigured = { availability: "not_configured" };
  return {
    generated_at_utc: "2026-10-04T12:00:00Z",
    known_applications: emptyDiscovery(),
    known_projects: emptyDiscovery(),
    source_issues: [],
    managed_operational_project_count: notConfigured,
    canonical_application_count: notConfigured,
    structural_node_count: notConfigured,
    desired_service_assignment_count: notConfigured,
    alerts: notConfigured,
    recent_event_count: notConfigured,
    recent_events: notConfigured,
    projects_with_attention: notConfigured,
    temporal_quality: notConfigured,
    scheduler: notConfigured,
  };
}

// Minimal valid empty `CatalogPage` (console/src/api/catalog.ts `.list()`).
export function emptyCatalogServicesFixture() {
  return { items: [], has_more: false, limit: 100 };
}

// Minimal valid empty pinned-artifact page (`.artifacts()`).
export function emptyCatalogArtifactsFixture() {
  return { source: "pinned_operational_artifact_references", limit: 100, has_more: false, items: [] };
}

export function createFixtureServer() {
  return http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const path = url.pathname;
    if (req.method !== "GET") {
      send(res, 404, { error: { code: "fixture_not_found" } });
      return;
    }
    if (path === "/api/v1/operations/continuum") {
      send(res, 200, continuumFixture());
      return;
    }
    const projectMatch = path.match(/^\/api\/v1\/operations\/projects\/([^/]+)\/continuum$/u);
    if (projectMatch) {
      send(res, 200, continuumProjectOverlayFixture(decodeURIComponent(projectMatch[1])));
      return;
    }
    // Overview and Projects both read this one endpoint (console/src/main.ts).
    if (path === "/api/v1/operations/overview") {
      send(res, 200, minimalGlobalOverviewFixture());
      return;
    }
    if (path === "/api/v1/catalog/services" && !url.search) {
      send(res, 200, emptyCatalogServicesFixture());
      return;
    }
    if (path === "/api/v1/catalog/artifacts") {
      send(res, 200, emptyCatalogArtifactsFixture());
      return;
    }
    send(res, 404, { error: { code: "fixture_not_found" } });
  });
}

// Only bind the fixed, documented port when run directly (`node
// tests/continuum-visual-fixture.mjs`) for manual review. Tests import
// `createFixtureServer()` and bind an ephemeral port instead.
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const port = Number(process.env.DATUM_CONTINUUM_FIXTURE_PORT ?? 8137);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid TEST ONLY fixture port.");
  createFixtureServer().listen(port, "127.0.0.1", () =>
    console.log(
      `TEST ONLY Phase135A integrated Console review fixture on http://127.0.0.1:${port}; no server persistence or authority. ` +
        "Continuum: node-a 2 accepted placements fresh/healthy/exact_match/converged, node-b 1 accepted placement stale-excluded/unknown convergence, node-c empty. " +
        "Overview/Projects/Catalog: minimal honest not-configured/empty shell projections.",
    ),
  );
}

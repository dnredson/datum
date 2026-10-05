import "bootstrap/dist/css/bootstrap.min.css";
import "bootstrap-icons/font/bootstrap-icons.css";
import "./styles/console.css";
import { startRouter } from "./app/router.ts";
import type { Route } from "./app/router.ts";
import { ApiError, OperationsClient } from "./api/client.ts";
import { badge, el } from "./components/presentation.ts";
import { overviewView, projectsView, projectView } from "./pages/views.ts";
import type { GlobalOverview, ProjectOverview } from "./types/operations.ts";
import { projectGraphView } from "./pages/project-graph.ts";
import { catalogView } from "./pages/catalog.ts";
import { CatalogClient } from "./api/catalog.ts";
import { ModelingClient } from "./api/service-modeling.ts";
import {DGraphEditorClient} from "./api/dgraph-editor.ts";
import {dgraphEditorView} from "./pages/dgraph-editor.ts";
import { continuumView } from "./pages/continuum.ts";
import { PlacementClient } from "./api/placement.ts";
import { placementView } from "./pages/placement.ts";
import { proposalReviewView } from "./pages/proposal-review.ts";
import { ReconciliationClient } from "./api/reconciliation.ts";
import { reconciliationView } from "./pages/reconciliation.ts";

const root = document.querySelector<HTMLDivElement>("#app");
if (!root) throw new Error("Console root missing");
root.innerHTML = `<a class="skip-link" href="#workspace">Skip to workspace</a>
  <aside class="sidebar" id="sidebar" aria-label="Main navigation">
    <a href="/" data-route class="brand" aria-label="DATUM Console — Overview"><img class="datum-mark" src="/brand/datum-monochrome.png" alt="" width="48" height="48" /><span class="brand-label">DATUM<small>CONSOLE</small></span></a>
    <div class="nav-label">OPERATIONS</div>
    <nav><a href="/" data-route data-nav="overview" aria-label="Overview"><i class="bi bi-grid-1x2" aria-hidden="true"></i><span class="nav-text">Overview</span></a><a href="/projects" data-route data-nav="projects" aria-label="Projects"><i class="bi bi-collection" aria-hidden="true"></i><span class="nav-text">Projects</span></a><a href="/continuum" data-route data-nav="continuum" aria-label="Continuum"><i class="bi bi-diagram-3" aria-hidden="true"></i><span class="nav-text">Continuum</span></a></nav>
    <div class="sidebar-footer"><i class="bi bi-shield-check" aria-hidden="true"></i><span>Read-only foundation<small>No operational write actions</small></span></div>
  </aside>
  <div class="console-layout"><header class="topbar"><div class="topbar-left"><button id="menu-toggle" class="menu-toggle btn btn-sm" aria-controls="sidebar" aria-expanded="true" aria-label="Collapse navigation"><i class="bi bi-list" aria-hidden="true"></i></button><a class="topbar-product" href="/" data-route aria-label="DATUM Console — Overview"><img src="/brand/datum-logo-horizontal.png" alt="" width="108" height="36" /></a><span class="topbar-divider" aria-hidden="true">/</span><span id="page-title">Overview</span><span id="project-context" class="project-context"></span></div><div id="connection" class="connection" aria-live="polite" aria-label="DServer connection / projection quality"></div></header>
  <main id="workspace" tabindex="-1"><div class="workspace-toolbar"><span class="eyebrow">THIS DSERVER · OPERATIONS PROJECTIONS</span><button id="refresh" class="btn btn-sm btn-outline-secondary"><i class="bi bi-arrow-clockwise me-2" aria-hidden="true"></i>Refresh</button></div><div id="page" aria-live="polite"></div></main><footer class="console-footer">DATUM control plane <span>Evidence ≠ authority · Console foundation</span></footer></div>`;
const page = document.querySelector<HTMLDivElement>("#page")!;
const catalogNav = el("a");
const catalogIcon = el("i", "bi bi-journal-code"); catalogIcon.setAttribute("aria-hidden", "true");
catalogNav.append(catalogIcon, el("span", "nav-text", "Catalog"));
catalogNav.href = "/catalog"; catalogNav.dataset.route = ""; catalogNav.dataset.nav = "catalog";
catalogNav.setAttribute("aria-label", "Global Artifact & Service Catalog");
document.querySelector(".sidebar nav")!.append(catalogNav);
const refresh = document.querySelector<HTMLButtonElement>("#refresh")!;
const connection = document.querySelector<HTMLDivElement>("#connection")!;
const client = new OperationsClient();
let disposeGraph: (() => void) | undefined;
let disposeContinuum: (() => void) | undefined;
let editor:
  | { element: HTMLElement; mount: () => void; dispose: () => void; dirty: () => boolean; confirmLeave: () => boolean }
  | undefined;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const menuToggle = document.querySelector<HTMLButtonElement>("#menu-toggle")!;
const workspace = document.querySelector<HTMLElement>("#workspace")!;
const tabletNavigation = window.matchMedia("(max-width: 900px)");
function syncNavigation(): void {
  const expanded = sidebar.classList.contains("expanded");
  const compact = sidebar.classList.contains("compact");
  const hidden = tabletNavigation.matches && !expanded;
  root!.classList.toggle(
    "navigation-compact",
    !tabletNavigation.matches && compact,
  );
  sidebar.inert = hidden;
  sidebar.setAttribute("aria-hidden", String(hidden));
  menuToggle.setAttribute(
    "aria-expanded",
    String(tabletNavigation.matches ? expanded : !compact),
  );
  menuToggle.setAttribute(
    "aria-label",
    tabletNavigation.matches
      ? expanded
        ? "Close navigation"
        : "Open navigation"
      : compact
        ? "Expand navigation"
        : "Collapse navigation",
  );
  if (hidden && sidebar.contains(document.activeElement)) menuToggle.focus();
}
tabletNavigation.addEventListener("change", syncNavigation);
syncNavigation();
let controller: AbortController | undefined;
let currentRoute: Route = { kind: "overview" };
let connectionState: "Connecting" | "Connected" | "Degraded" | "Disconnected" =
  "Disconnected";
function setConnection(
  state: "Connecting" | "Connected" | "Degraded" | "Disconnected",
): void {
  connectionState = state;
  connection.replaceChildren(
    badge(
      state,
      state === "Connected"
        ? "success"
        : state === "Degraded"
          ? "warning"
          : state === "Disconnected"
            ? "danger"
            : "neutral",
    ),
    el("span", "small", "DServer"),
  );
  connection.title =
    "Console connectivity / projection quality, not canonical infrastructure health";
}
function degraded(data: GlobalOverview | ProjectOverview): boolean {
  return (
    Object.values(data).some(
      (v) =>
        v &&
        typeof v === "object" &&
        "availability" in v &&
        ["unavailable", "corrupt", "indeterminate"].includes(
          String(v.availability),
        ),
    ) ||
    ("source_issues" in data && data.source_issues.length > 0)
  );
}
function stateView(title: string, message: string, error = false): HTMLElement {
  const box = el("section", "console-card state-view");
  box.setAttribute("role", error ? "alert" : "status");
  box.append(
    el("div", "state-icon", error ? "!" : "·"),
    el("h1", "h4", title),
    el("p", "text-secondary mb-0", message),
  );
  return box;
}
async function render(route: Route): Promise<void> {
  if(editor && JSON.stringify(currentRoute)===JSON.stringify(route) && !editor.confirmLeave())return;
  editor?.dispose();editor=undefined;
  disposeGraph?.();
  disposeGraph = undefined;
  disposeContinuum?.();
  disposeContinuum = undefined;
  const navigationChanged =
    JSON.stringify(currentRoute) !== JSON.stringify(route);
  currentRoute = route;
  controller?.abort();
  controller = new AbortController();
  const signal = controller.signal;
  sidebar.classList.remove("expanded");
  syncNavigation();
  if (navigationChanged) workspace.focus();
  page.removeAttribute("aria-busy");
  const title =
    route.kind === "project_graph_editor" ? "DGraph editor" : route.kind === "catalog" ? "Catalog" : route.kind === "project_graph"
      ? "Graph"
      : route.kind === "continuum" || route.kind === "project_continuum"
        ? "Continuum"
        : route.kind === "project_placement"
          ? "Placement"
          : route.kind === "project_proposal_review"
            ? "Proposal review"
            : route.kind === "project_application_deploy"
              ? "Deploy / Reconciliation"
            : route.kind === "project"
            ? "Project"
            : route.kind === "projects"
              ? "Projects"
              : route.kind === "not_found"
                ? "Not found"
                : "Overview";
  document.title = `${title} · DATUM Console`;
  document.querySelector("#page-title")!.textContent = title;
  document.querySelector("#project-context")!.textContent =
    route.kind === "project" || route.kind === "project_graph" || route.kind === "project_graph_editor" || route.kind === "project_continuum" || route.kind === "project_placement" || route.kind === "project_proposal_review" || route.kind === "project_application_deploy"
      ? route.projectId
      : "";
  document.querySelector<HTMLElement>("#project-context")!.title =
    route.kind === "project" || route.kind === "project_graph" || route.kind === "project_graph_editor" || route.kind === "project_continuum" || route.kind === "project_placement" || route.kind === "project_proposal_review" || route.kind === "project_application_deploy"
      ? route.projectId
      : "";
  for (const nav of document.querySelectorAll<HTMLAnchorElement>(
    "[data-nav]",
  )) {
    const active =
      nav.dataset.nav === route.kind ||
      (nav.dataset.nav === "projects" &&
        (route.kind === "project" || route.kind === "project_graph" || route.kind === "project_graph_editor" || route.kind === "project_continuum" || route.kind === "project_placement" || route.kind === "project_proposal_review" || route.kind === "project_application_deploy"));
    if (active) nav.setAttribute("aria-current", "page");
    else nav.removeAttribute("aria-current");
  }
  refresh.disabled = route.kind === "not_found";
  if(route.kind === "project_graph_editor") {
    document.querySelector(".workspace-toolbar .eyebrow")!.textContent="PROJECT · LOCAL DGRAPH DRAFT";
    document.querySelector(".sidebar-footer span")!.textContent="Draft modeling only · no acceptance or execution";
    setConnection("Connecting");
    editor=dgraphEditorView(route.projectId,new DGraphEditorClient(route.projectId,signal),ready=>{if(!signal.aborted)setConnection(ready?"Connected":"Degraded");});
    page.replaceChildren(editor.element);editor.mount();return;
  }
  if (route.kind === "project_placement") {
    document.querySelector(".workspace-toolbar .eyebrow")!.textContent = "PROJECT · PLACEMENT CANDIDATE (NOT ACCEPTANCE)";
    document.querySelector(".sidebar-footer span")!.textContent = "Candidate / validate / propose only · acceptance remains a separate step";
    setConnection("Connecting");
    editor = placementView(
      route.projectId,
      new PlacementClient(route.projectId, signal),
      new DGraphEditorClient(route.projectId, signal),
      client,
      signal,
      (ready) => {
        if (!signal.aborted) setConnection(ready ? "Connected" : "Degraded");
      },
    );
    page.replaceChildren(editor.element);
    editor.mount();
    return;
  }
  if (route.kind === "project_proposal_review") {
    document.querySelector(".workspace-toolbar .eyebrow")!.textContent = "PROJECT · PROPOSAL REVIEW (AUTHORITY ONLY)";
    document.querySelector(".sidebar-footer span")!.textContent = "Acceptance changes placement authority only · never deployment or runtime";
    setConnection("Connecting");
    editor = proposalReviewView(
      route.projectId,
      route.proposalId,
      new PlacementClient(route.projectId, signal),
      (ready) => {
        if (!signal.aborted) setConnection(ready ? "Connected" : "Degraded");
      },
    );
    page.replaceChildren(editor.element);
    editor.mount();
    return;
  }
  if (route.kind === "project_application_deploy") {
    document.querySelector(".workspace-toolbar .eyebrow")!.textContent = "PROJECT · DEPLOY / RECONCILIATION (REQUEST + AUTHORIZATION ONLY)";
    document.querySelector(".sidebar-footer span")!.textContent = "Creates reconciliation requests only · the node agent is what actually mutates a host";
    setConnection("Connecting");
    editor = reconciliationView(
      route.projectId,
      route.applicationId,
      new PlacementClient(route.projectId, signal),
      new ReconciliationClient(route.projectId, signal),
      (ready) => {
        if (!signal.aborted) setConnection(ready ? "Connected" : "Degraded");
      },
    );
    page.replaceChildren(editor.element);
    editor.mount();
    return;
  }
  if (route.kind === "not_found") {
    if (connectionState === "Connecting") setConnection("Disconnected");
    page.replaceChildren(
      stateView(
        "Page not found",
        "This route is not part of the Console foundation. Use Overview or Projects to continue.",
      ),
    );
    return;
  }
  if (route.kind === "catalog") {
    document.querySelector(".workspace-toolbar .eyebrow")!.textContent = "GLOBAL · CATALOG MODELING";
    setConnection("Connecting");
    page.replaceChildren(catalogView(new CatalogClient(signal), ready => {
      if (!signal.aborted) setConnection(ready ? "Connected" : "Degraded");
    }, new ModelingClient(signal)));
    document.querySelector(".sidebar-footer span")!.textContent = "Catalog modeling only · operational actions remain read-only";
    return;
  }
  document.querySelector(".workspace-toolbar .eyebrow")!.textContent = "THIS DSERVER · OPERATIONS PROJECTIONS";
  document.querySelector(".sidebar-footer span")!.textContent = "Read-only foundation · No operational write actions";
  setConnection("Connecting");
  page.setAttribute("aria-busy", "true");
  page.replaceChildren(
    stateView(
      "Loading projection",
      "Reading DServer operational views. No empty counts are inferred while loading.",
    ),
  );
  refresh.disabled = true;
  try {
    if (route.kind === "project_graph") {
      const runtime = await client.runtimeGraph(route.projectId, signal);
      const data = runtime.logical;
      if (signal.aborted) return;
      const view = projectGraphView(data, runtime);
      page.replaceChildren(view.element);
      disposeGraph = view.dispose;
      view.mount();
      setConnection(
        ["unavailable", "corrupt", "indeterminate"].includes(
          runtime.runtime.availability,
        ) ||
          (runtime.runtime.availability === "available" &&
            runtime.runtime.value.completeness === "partial") ||
          ["unavailable", "corrupt", "indeterminate"].includes(
            data.graph.availability,
          ) ||
          (data.graph.availability === "available" &&
            data.graph.value.coverage.completeness === "partial")
          ? "Degraded"
          : "Connected",
      );
    } else if (route.kind === "project") {
      const data = await client.project(route.projectId, signal);
      if (signal.aborted) return;
      page.replaceChildren(projectView(data));
      setConnection(degraded(data) ? "Degraded" : "Connected");
    } else if (route.kind === "continuum" || route.kind === "project_continuum") {
      const projectId = route.kind === "project_continuum" ? route.projectId : undefined;
      const data = projectId
        ? await client.projectContinuum(projectId, signal)
        : await client.continuum(signal);
      if (signal.aborted) return;
      const continuumPage = continuumView(data, projectId);
      page.replaceChildren(continuumPage.element);
      disposeContinuum = continuumPage.dispose;
      continuumPage.mount();
      setConnection(data.truncated ? "Degraded" : "Connected");
    } else {
      const data = await client.overview(signal);
      if (signal.aborted) return;
      page.replaceChildren(
        route.kind === "projects" ? projectsView(data) : overviewView(data),
      );
      setConnection(degraded(data) ? "Degraded" : "Connected");
    }
  } catch (error) {
    if (signal.aborted) return;
    setConnection(
      error instanceof ApiError && error.status !== null
        ? "Degraded"
        : "Disconnected",
    );
    page.replaceChildren(
      stateView(
        "Projection unavailable",
        error instanceof ApiError
          ? `${error.message} (${error.code})`
          : "The read projection could not be displayed.",
        true,
      ),
    );
  } finally {
    if (!signal.aborted) {
      page.removeAttribute("aria-busy");
      refresh.disabled = false;
    }
  }
}
refresh.addEventListener("click", () => {
  if (!refresh.disabled) void render(currentRoute);
});
menuToggle.addEventListener("click", () => {
  sidebar.classList.toggle(tabletNavigation.matches ? "expanded" : "compact");
  syncNavigation();
});
function closeNavigation(event: KeyboardEvent): void {
  if (
    event.key === "Escape" &&
    tabletNavigation.matches &&
    sidebar.classList.contains("expanded")
  ) {
    sidebar.classList.remove("expanded");
    syncNavigation();
    menuToggle.focus();
  }
}
document.addEventListener("keydown", closeNavigation);
window.addEventListener("pagehide", () => {
  editor?.dispose();
  disposeGraph?.();
  disposeContinuum?.();
  controller?.abort();
});
setConnection("Disconnected");
const stopRouter = startRouter((route) => void render(route),()=>editor?.confirmLeave()??true);
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    editor?.dispose();
    disposeGraph?.();
    disposeContinuum?.();
    controller?.abort();
    stopRouter();
    tabletNavigation.removeEventListener("change", syncNavigation);
    document.removeEventListener("keydown", closeNavigation);
  });

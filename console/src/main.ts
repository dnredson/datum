import "bootstrap/dist/css/bootstrap.min.css";
import "bootstrap-icons/font/bootstrap-icons.css";
import "./styles/console.css";
import { startRouter } from "./app/router.ts";
import type { Route } from "./app/router.ts";
import { ApiError, OperationsClient } from "./api/client.ts";
import { badge, el } from "./components/presentation.ts";
import { overviewView, projectsView, projectView } from "./pages/views.ts";
import type { GlobalOverview, ProjectOverview } from "./types/operations.ts";

const root = document.querySelector<HTMLDivElement>("#app");
if (!root) throw new Error("Console root missing");
root.innerHTML = `<a class="skip-link" href="#workspace">Skip to workspace</a>
  <aside class="sidebar" id="sidebar" aria-label="Main navigation">
    <a href="/" data-route class="brand" aria-label="DATUM Console — Overview"><img class="datum-mark" src="/brand/datum-monochrome.png" alt="" width="48" height="48" /><span class="brand-label">DATUM<small>CONSOLE</small></span></a>
    <div class="nav-label">OPERATIONS</div>
    <nav><a href="/" data-route data-nav="overview" aria-label="Overview"><i class="bi bi-grid-1x2" aria-hidden="true"></i><span class="nav-text">Overview</span></a><a href="/projects" data-route data-nav="projects" aria-label="Projects"><i class="bi bi-collection" aria-hidden="true"></i><span class="nav-text">Projects</span></a></nav>
    <div class="sidebar-footer"><i class="bi bi-shield-check" aria-hidden="true"></i><span>Read-only foundation<small>No operational write actions</small></span></div>
  </aside>
  <div class="console-layout"><header class="topbar"><div class="topbar-left"><button id="menu-toggle" class="menu-toggle btn btn-sm" aria-controls="sidebar" aria-expanded="true" aria-label="Collapse navigation"><i class="bi bi-list" aria-hidden="true"></i></button><a class="topbar-product" href="/" data-route aria-label="DATUM Console — Overview"><img src="/brand/datum-logo-horizontal.png" alt="" width="108" height="36" /></a><span class="topbar-divider" aria-hidden="true">/</span><span id="page-title">Overview</span><span id="project-context" class="project-context"></span></div><div id="connection" class="connection" aria-live="polite" aria-label="DServer connection / projection quality"></div></header>
  <main id="workspace" tabindex="-1"><div class="workspace-toolbar"><span class="eyebrow">THIS DSERVER · OPERATIONS PROJECTIONS</span><button id="refresh" class="btn btn-sm btn-outline-secondary"><i class="bi bi-arrow-clockwise me-2" aria-hidden="true"></i>Refresh</button></div><div id="page" aria-live="polite"></div></main><footer class="console-footer">DATUM control plane <span>Evidence ≠ authority · Console foundation</span></footer></div>`;
const page = document.querySelector<HTMLDivElement>("#page")!;
const refresh = document.querySelector<HTMLButtonElement>("#refresh")!;
const connection = document.querySelector<HTMLDivElement>("#connection")!;
const client = new OperationsClient();
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
    route.kind === "project"
      ? "Project"
      : route.kind === "projects"
        ? "Projects"
        : route.kind === "not_found"
          ? "Not found"
          : "Overview";
  document.title = `${title} · DATUM Console`;
  document.querySelector("#page-title")!.textContent = title;
  document.querySelector("#project-context")!.textContent =
    route.kind === "project" ? route.projectId : "";
  document.querySelector<HTMLElement>("#project-context")!.title =
    route.kind === "project" ? route.projectId : "";
  for (const nav of document.querySelectorAll<HTMLAnchorElement>(
    "[data-nav]",
  )) {
    const active =
      nav.dataset.nav === route.kind ||
      (nav.dataset.nav === "projects" && route.kind === "project");
    if (active) nav.setAttribute("aria-current", "page");
    else nav.removeAttribute("aria-current");
  }
  refresh.disabled = route.kind === "not_found";
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
    if (route.kind === "project") {
      const data = await client.project(route.projectId, signal);
      if (signal.aborted) return;
      page.replaceChildren(projectView(data));
      setConnection(degraded(data) ? "Degraded" : "Connected");
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
refresh.addEventListener("click", () => void render(currentRoute));
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
  controller?.abort();
});
setConnection("Disconnected");
const stopRouter = startRouter((route) => void render(route));
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    controller?.abort();
    stopRouter();
    tabletNavigation.removeEventListener("change", syncNavigation);
    document.removeEventListener("keydown", closeNavigation);
  });

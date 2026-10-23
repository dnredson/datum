// Phase 142A -- explicit "Manage nodes" selector for the Area of Interest.
// Discovery is the GLOBAL `GET /api/v1/operations/continuum` node list
// (no project scoping of the inventory itself -- a node is never treated
// as "belonging" to a project). All filters here are client-side
// presentation filters over already-canonical fields; none of them
// invents a node type, role, or health verdict.
//
// Named/structured around "node" deliberately, not a generic "target"
// abstraction -- the brief asks to keep this extensible for a future
// Area of Interest covering other operational subjects (links,
// applications, D-Servs), but explicitly asks NOT to generalize
// `LiveViewV1.pinned_node_ids` now. This module is the natural seam for
// that future work; it is not forced into a premature abstraction today.
import type { ContinuumNode, ContinuumView } from "../types/continuum.ts";
import type { OperationalAlert } from "../types/operations.ts";
import { el, badge } from "../components/presentation.ts";

export type TriStateFilter = "any" | "present" | "absent";
export type ConvergenceFilterValue = "any" | "converged" | "diverged" | "unknown";

export interface NodeDiscoveryFilters {
  search: string;
  projectId: string; // "" = all projects
  os: string; // "" = all
  architecture: string; // "" = all
  telemetry: TriStateFilter; // host resource_observation presence
  observation: TriStateFilter; // DMonitor observation presence
  alerts: TriStateFilter; // active alert correlation presence
  convergence: ConvergenceFilterValue;
}

export const EMPTY_NODE_DISCOVERY_FILTERS: NodeDiscoveryFilters = {
  search: "",
  projectId: "",
  os: "",
  architecture: "",
  telemetry: "any",
  observation: "any",
  alerts: "any",
  convergence: "any",
};

// "project X references this node via an accepted placement" -- never
// "node belongs to project X" (same rule the Continuum page's own
// `project_overlay` already documents).
export function nodeProjectIds(node: ContinuumNode): string[] {
  return [...new Set(node.accepted_placements.map((p) => p.project_id))];
}

export interface NodeDiscoveryOptions {
  projects: string[];
  os: string[];
  architectures: string[];
}
export function nodeDiscoveryOptions(nodes: ContinuumNode[]): NodeDiscoveryOptions {
  const projects = new Set<string>();
  const os = new Set<string>();
  const architectures = new Set<string>();
  for (const node of nodes) {
    for (const p of nodeProjectIds(node)) projects.add(p);
    os.add(node.structure.platform.os);
    architectures.add(node.structure.platform.architecture);
  }
  return { projects: [...projects].sort(), os: [...os].sort(), architectures: [...architectures].sort() };
}

// Every predicate reads only fields the backend already returns. No
// node_id prefix/hostname heuristic, no fog/edge/cloud inference, no
// derived "node health" -- see the Phase 142A review's explicit ban on
// inventing a node type from `node_id`.
export function matchesNodeDiscoveryFilters(
  node: ContinuumNode,
  filters: NodeDiscoveryFilters,
  hasActiveAlert: boolean,
): boolean {
  if (filters.search && !node.node_id.toLowerCase().includes(filters.search.toLowerCase())) return false;
  if (filters.projectId && !nodeProjectIds(node).includes(filters.projectId)) return false;
  if (filters.os && node.structure.platform.os !== filters.os) return false;
  if (filters.architecture && node.structure.platform.architecture !== filters.architecture) return false;
  if (filters.telemetry === "present" && !node.resource_observation) return false;
  if (filters.telemetry === "absent" && node.resource_observation) return false;
  if (filters.observation === "present" && node.observations.length === 0) return false;
  if (filters.observation === "absent" && node.observations.length > 0) return false;
  if (filters.alerts === "present" && !hasActiveAlert) return false;
  if (filters.alerts === "absent" && hasActiveAlert) return false;
  if (filters.convergence !== "any" && !node.observations.some((o) => o.placement_convergence === filters.convergence))
    return false;
  return true;
}

// Preserves the existing pinned order for ids still selected; appends
// newly-selected ids in discovery order (not click order -- deterministic
// and simple). Never drops an id just because it is missing from
// `allKnownNodeIdsInOrder` -- the caller is responsible for keeping a
// previously-pinned-but-currently-missing id in `selected` unless the
// operator explicitly unchecked it (Phase 142A brief: never silently
// remove a temporarily-unavailable pinned node).
export function applyNodeSelection(
  previousPinned: string[],
  selected: ReadonlySet<string>,
  allKnownNodeIdsInOrder: string[],
): string[] {
  const kept = previousPinned.filter((id) => selected.has(id));
  const keptSet = new Set(kept);
  const added = allKnownNodeIdsInOrder.filter((id) => selected.has(id) && !keptSet.has(id));
  return [...kept, ...added];
}

export interface NodePickerController {
  element: HTMLElement;
  open: () => void;
}

export function createNodePicker(
  getContinuum: () => ContinuumView | undefined,
  getAlertsByNode: () => Map<string, OperationalAlert[]>,
  getPinned: () => string[],
  countActiveAlerts: (alerts: OperationalAlert[]) => number,
  onApply: (nextPinned: string[]) => void,
): NodePickerController {
  let draftSelected = new Set<string>();
  const filters: NodeDiscoveryFilters = { ...EMPTY_NODE_DISCOVERY_FILTERS };

  const overlay = el("div", "live-picker-overlay");
  overlay.hidden = true;
  const panel = el("div", "live-picker-panel console-card");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  panel.setAttribute("aria-label", "Manage nodes");
  overlay.append(panel);

  const header = el("div", "live-picker-header");
  header.append(el("h2", "h6 mb-0", "Manage nodes"));
  const closeButton = el("button", "btn btn-sm btn-outline-secondary", "Cancel");
  header.append(closeButton);
  panel.append(header);

  const searchInput = el("input", "form-control form-control-sm");
  searchInput.placeholder = "Search by node id…";
  searchInput.setAttribute("aria-label", "Search by node id");
  panel.append(searchInput);

  const filterRow = el("div", "live-filter-row live-picker-filters");
  const projectSelect = el("select", "form-select form-select-sm");
  const osSelect = el("select", "form-select form-select-sm");
  const archSelect = el("select", "form-select form-select-sm");
  const telemetrySelect = el("select", "form-select form-select-sm");
  const observationSelect = el("select", "form-select form-select-sm");
  const alertsSelect = el("select", "form-select form-select-sm");
  const convergenceSelect = el("select", "form-select form-select-sm");
  const triOptions = (label: string): [string, string][] => [
    ["any", `${label}: Any`],
    ["present", `${label}: Present`],
    ["absent", `${label}: Absent`],
  ];
  for (const [value, text] of triOptions("Telemetry")) telemetrySelect.append(new Option(text, value));
  for (const [value, text] of triOptions("DMonitor evidence")) observationSelect.append(new Option(text, value));
  for (const [value, text] of triOptions("Active alerts")) alertsSelect.append(new Option(text, value));
  for (const [value, text] of [
    ["any", "Convergence: Any"],
    ["converged", "Convergence: Converged"],
    ["diverged", "Convergence: Diverged"],
    ["unknown", "Convergence: Unknown"],
  ]) convergenceSelect.append(new Option(text, value));
  filterRow.append(projectSelect, osSelect, archSelect, telemetrySelect, observationSelect, alertsSelect, convergenceSelect);
  panel.append(filterRow);

  const list = el("div", "live-picker-list");
  panel.append(list);

  const missingSection = el("div", "live-picker-missing");
  panel.append(missingSection);

  const footer = el("div", "live-picker-footer");
  const countLabel = el("span", "small text-secondary", "");
  const applyButton = el("button", "btn btn-sm btn-primary", "Apply selection");
  footer.append(countLabel, applyButton);
  panel.append(footer);

  function renderOptions(): void {
    const options = nodeDiscoveryOptions(getContinuum()?.nodes ?? []);
    projectSelect.replaceChildren(new Option("All projects", ""), ...options.projects.map((p) => new Option(p, p)));
    osSelect.replaceChildren(new Option("All platforms", ""), ...options.os.map((o) => new Option(o, o)));
    archSelect.replaceChildren(new Option("All architectures", ""), ...options.architectures.map((a) => new Option(a, a)));
  }

  function nodeSummaryRow(node: ContinuumNode, alertCount: number): HTMLElement {
    const row = el("label", "live-picker-row");
    const checkbox = el("input", "form-check-input");
    checkbox.type = "checkbox";
    checkbox.checked = draftSelected.has(node.node_id);
    checkbox.addEventListener("change", () => {
      if (checkbox.checked) draftSelected.add(node.node_id);
      else draftSelected.delete(node.node_id);
      renderFooter();
    });
    const summary = el("div", "live-picker-row-summary");
    const projects = nodeProjectIds(node);
    summary.append(
      el("strong", "small", node.node_id),
      el(
        "span",
        "small text-secondary",
        `${node.structure.platform.os} / ${node.structure.platform.architecture}`,
      ),
      el("span", "small text-secondary", projects.length ? `Projects: ${projects.join(", ")}` : "No accepted placements"),
      el("span", "small text-secondary", node.resource_observation ? "Telemetry available" : "Telemetry unavailable"),
      alertCount > 0 ? badge(`${alertCount} active alert(s)`, "danger") : el("span", "small text-secondary", "No active alerts"),
    );
    row.append(checkbox, summary);
    return row;
  }

  function renderFooter(): void {
    countLabel.textContent = `${draftSelected.size} node(s) selected`;
  }

  function renderList(): void {
    const continuum = getContinuum();
    const alertsByNode = getAlertsByNode();
    const nodes = continuum?.nodes ?? [];
    list.replaceChildren();
    // Review P2-A fix: must use the SAME active-alert semantics as the row's
    // own badge (`countActiveAlerts`), never the raw correlated count --
    // otherwise a node with only resolved/closed alerts could match
    // "Active alerts: Present" while its own badge says "No active alerts".
    const visible = nodes.filter((n) => matchesNodeDiscoveryFilters(n, filters, countActiveAlerts(alertsByNode.get(n.node_id) ?? []) > 0));
    if (visible.length === 0) list.append(el("p", "empty-text", "No D-Nodes match the current filters."));
    for (const node of visible) {
      list.append(nodeSummaryRow(node, countActiveAlerts(alertsByNode.get(node.node_id) ?? [])));
    }
    const knownIds = new Set(nodes.map((n) => n.node_id));
    const missing = [...draftSelected].filter((id) => !knownIds.has(id));
    missingSection.replaceChildren();
    if (missing.length > 0) {
      missingSection.append(el("h3", "h6 small text-uppercase text-secondary mt-2", "Currently pinned, not found in Continuum"));
      for (const id of missing) {
        const row = el("label", "live-picker-row");
        const checkbox = el("input", "form-check-input");
        checkbox.type = "checkbox";
        checkbox.checked = true;
        checkbox.addEventListener("change", () => {
          if (checkbox.checked) draftSelected.add(id);
          else draftSelected.delete(id);
          renderFooter();
        });
        const summary = el("div", "live-picker-row-summary");
        summary.append(el("strong", "small", id), badge("Not currently available", "neutral"));
        row.append(checkbox, summary);
        missingSection.append(row);
      }
    }
    renderFooter();
  }

  searchInput.addEventListener("input", () => {
    filters.search = searchInput.value;
    renderList();
  });
  projectSelect.addEventListener("change", () => {
    filters.projectId = projectSelect.value;
    renderList();
  });
  osSelect.addEventListener("change", () => {
    filters.os = osSelect.value;
    renderList();
  });
  archSelect.addEventListener("change", () => {
    filters.architecture = archSelect.value;
    renderList();
  });
  telemetrySelect.addEventListener("change", () => {
    filters.telemetry = telemetrySelect.value as TriStateFilter;
    renderList();
  });
  observationSelect.addEventListener("change", () => {
    filters.observation = observationSelect.value as TriStateFilter;
    renderList();
  });
  alertsSelect.addEventListener("change", () => {
    filters.alerts = alertsSelect.value as TriStateFilter;
    renderList();
  });
  convergenceSelect.addEventListener("change", () => {
    filters.convergence = convergenceSelect.value as ConvergenceFilterValue;
    renderList();
  });

  function close(): void {
    overlay.hidden = true;
  }
  closeButton.addEventListener("click", close);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) close();
  });
  applyButton.addEventListener("click", () => {
    const allKnownIds = (getContinuum()?.nodes ?? []).map((n) => n.node_id);
    onApply(applyNodeSelection(getPinned(), draftSelected, allKnownIds));
    close();
  });

  function open(): void {
    filters.search = "";
    filters.projectId = "";
    filters.os = "";
    filters.architecture = "";
    filters.telemetry = "any";
    filters.observation = "any";
    filters.alerts = "any";
    filters.convergence = "any";
    searchInput.value = "";
    draftSelected = new Set(getPinned());
    renderOptions();
    projectSelect.value = "";
    osSelect.value = "";
    archSelect.value = "";
    telemetrySelect.value = "any";
    observationSelect.value = "any";
    alertsSelect.value = "any";
    convergenceSelect.value = "any";
    renderList();
    overlay.hidden = false;
  }

  return { element: overlay, open };
}

// Phase 135 / 135A -- Continuum View + Dashboard page. Read-only. Renders
// the physical D-Node inventory, accepted placement references, and
// DMonitor observation evidence as three visually and textually SEPARATE
// domains -- never collapsed into one status. See
// documentation/reference/phase135-continuum-view.md for the full
// authority-source rationale this page must never contradict.
//
// Three coexisting, independent ways to inspect the SAME data (Phase 135A,
// section 9): an optional Cytoscape canvas + inspector panel (best-effort,
// degrades gracefully), and an always-present, always-fully-detailed
// accessible card list that never depends on the canvas succeeding.
import cytoscape from "cytoscape";
import type { Core, ElementDefinition, StylesheetJson } from "cytoscape";
import type {
  ContinuumView,
  ContinuumNode,
  AcceptedPlacementRef,
  NodeObservation,
  ObservationEvidence,
  DMonitorFreshness,
  DMonitorHealth,
  AuthorityCorrelation,
  DMonitorConvergence,
} from "../types/continuum.ts";
import { el, badge } from "../components/presentation.ts";
import { terminalMetricPanel } from "../components/terminal-metrics.ts";
import type { Tone } from "../components/presentation.ts";
import { GRAPH_CATEGORIES, GRAPH_RELATIONS, graphVisualStyles } from "../components/graph-visuals.ts";

function bytes(value: number): string {
  if (value >= 1_073_741_824) return `${(value / 1_073_741_824).toFixed(1)} GiB`;
  if (value >= 1_048_576) return `${(value / 1_048_576).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}

const FRESHNESS_TONE: Record<DMonitorFreshness, Tone> = {
  fresh: "success",
  stale: "warning",
  clock_skew: "danger",
};
const FRESHNESS_LABEL: Record<DMonitorFreshness, string> = {
  fresh: "Fresh",
  stale: "Stale",
  clock_skew: "Clock skew",
};
const HEALTH_TONE: Record<DMonitorHealth, Tone> = {
  healthy: "success",
  degraded: "warning",
  unhealthy: "danger",
  unknown: "neutral",
};
const HEALTH_LABEL: Record<DMonitorHealth, string> = {
  healthy: "Healthy",
  degraded: "Degraded",
  unhealthy: "Unhealthy",
  unknown: "Unknown",
};
const CORRELATION_TONE: Record<AuthorityCorrelation, Tone> = {
  exact_match: "success",
  mismatch: "danger",
  ref_missing: "neutral",
  not_applicable: "neutral",
};
const CORRELATION_LABEL: Record<AuthorityCorrelation, string> = {
  exact_match: "Matches active authority",
  mismatch: "Does not match active authority",
  ref_missing: "No authority reference",
  not_applicable: "Not applicable",
};
const CONVERGENCE_TONE: Record<DMonitorConvergence, Tone> = {
  converged: "success",
  diverged: "danger",
  unknown: "neutral",
};
const CONVERGENCE_LABEL: Record<DMonitorConvergence, string> = {
  converged: "Converged",
  diverged: "Diverged",
  unknown: "Unknown",
};

// ----------------------------------------------------------- DASHBOARD METRICS

export interface ContinuumMetrics {
  nodeCount: number;
  acceptedPlacementCount: number;
  nodesWithObservation: number;
  nodesWithoutObservation: number;
  freshCount: number;
  staleCount: number;
  clockSkewCount: number;
  healthyCount: number;
  degradedCount: number;
  unhealthyCount: number;
  unknownHealthCount: number;
}

// Pure, exported for direct testing. Every count is derived ONLY from
// fields already present on `ContinuumView` -- no available/allocated/
// utilization/ownership value is computed anywhere here (Phase 135A,
// section 2). Freshness is counted per EVIDENCE entry (admitted + excluded
// -- freshness is DMonitor's own classification of that evidence,
// independent of whether it was admitted as current proof). Health is
// counted from ADMITTED evidence only -- deliberately conservative: an
// excluded (e.g. stale) entry's health claim is historical, not a current
// authoritative signal, so it is not mixed into the current-state health
// counts.
export function computeContinuumMetrics(data: ContinuumView): ContinuumMetrics {
  const metrics: ContinuumMetrics = {
    nodeCount: data.nodes.length,
    acceptedPlacementCount: 0,
    nodesWithObservation: 0,
    nodesWithoutObservation: 0,
    freshCount: 0,
    staleCount: 0,
    clockSkewCount: 0,
    healthyCount: 0,
    degradedCount: 0,
    unhealthyCount: 0,
    unknownHealthCount: 0,
  };
  for (const node of data.nodes) {
    metrics.acceptedPlacementCount += node.accepted_placements.length;
    if (node.observations.length === 0) {
      metrics.nodesWithoutObservation += 1;
      continue;
    }
    metrics.nodesWithObservation += 1;
    for (const observation of node.observations) {
      for (const e of [...observation.admitted_evidence, ...observation.excluded_evidence]) {
        if (e.freshness === "fresh") metrics.freshCount += 1;
        else if (e.freshness === "stale") metrics.staleCount += 1;
        else metrics.clockSkewCount += 1;
      }
      for (const e of observation.admitted_evidence) {
        if (e.health === "healthy") metrics.healthyCount += 1;
        else if (e.health === "degraded") metrics.degradedCount += 1;
        else if (e.health === "unhealthy") metrics.unhealthyCount += 1;
        else metrics.unknownHealthCount += 1;
      }
    }
  }
  return metrics;
}

function metricCard(label: string, value: number, tone: Tone = "neutral"): HTMLElement {
  const card = el("div", "continuum-metric-card");
  card.append(el("span", "continuum-metric-value", String(value)), el("span", "continuum-metric-label", label));
  if (tone !== "neutral") card.classList.add(`tone-${tone}`);
  return card;
}

function dashboardView(metrics: ContinuumMetrics): HTMLElement {
  const dashboard = el("section", "continuum-dashboard");
  dashboard.setAttribute("aria-label", "Continuum summary metrics");
  dashboard.append(
    metricCard("D-Nodes", metrics.nodeCount),
    metricCard("Accepted placements", metrics.acceptedPlacementCount),
    metricCard("Nodes with observation", metrics.nodesWithObservation),
    metricCard("Unknown / no observation", metrics.nodesWithoutObservation),
    metricCard("Fresh observations", metrics.freshCount, "success"),
    metricCard("Stale observations", metrics.staleCount, metrics.staleCount ? "warning" : "neutral"),
    metricCard("Clock skew observations", metrics.clockSkewCount, metrics.clockSkewCount ? "danger" : "neutral"),
    metricCard("Healthy", metrics.healthyCount, "success"),
    metricCard("Degraded", metrics.degradedCount, metrics.degradedCount ? "warning" : "neutral"),
    metricCard("Unhealthy", metrics.unhealthyCount, metrics.unhealthyCount ? "danger" : "neutral"),
    metricCard("Unknown health", metrics.unknownHealthCount),
  );
  return dashboard;
}

// ----------------------------------------------------------- CANVAS ELEMENTS

// Pure, exported for direct testing. One node per D-Node (infrastructure
// category) and one small compact node per accepted placement (software
// category, per Phase 135A section 3's explicit "visually distinct
// logical-software references" instruction), joined by the EXISTING
// `placement` relation edge style (dashed, never DCall's solid style).
// Observation is encoded ONLY as a border overlay on the placement node --
// never as a change to its fill color/shape, which remains the fixed
// "software" category identity regardless of observation state. No edge is
// ever drawn between two D-Nodes merely for layout: `links` entries (always
// empty in production today) are the ONLY source of D-Node-to-D-Node edges.
export function continuumElements(data: ContinuumView): ElementDefinition[] {
  const elements: ElementDefinition[] = [];
  for (const node of data.nodes) {
    elements.push({
      data: { id: node.node_id, label: node.node_id, visual_category: "infrastructure", kind: "dnode" },
    });
    for (const placement of node.accepted_placements) {
      const placementId = `placement:${node.node_id}:${placement.project_id}:${placement.application_id}:${placement.service_id}`;
      const observation = node.observations.find(
        (o) =>
          o.project_id === placement.project_id &&
          o.application_id === placement.application_id &&
          o.service_id === placement.service_id,
      );
      const observationState = !observation
        ? "unknown"
        : observation.admitted_evidence.some((e) => e.freshness === "fresh")
          ? "fresh"
          : observation.admitted_evidence.length > 0 || observation.excluded_evidence.length > 0
            ? "stale"
            : "unknown";
      elements.push({
        data: {
          id: placementId,
          label: placement.service_id,
          visual_category: "software",
          kind: "placement_ref",
          parent_node_id: node.node_id,
          project_id: placement.project_id,
          application_id: placement.application_id,
          service_id: placement.service_id,
          observation_state: observationState,
        },
      });
      elements.push({
        data: {
          id: `${placementId}::edge`,
          source: placementId,
          target: node.node_id,
          visual_relation: "placement",
          kind: "placement",
        },
      });
    }
  }
  for (const link of data.links) {
    elements.push({
      data: {
        id: link.link_id,
        source: link.from_node_id,
        target: link.to_node_id,
        visual_relation: "physical",
        kind: "physical_link",
      },
    });
  }
  return elements;
}

// Additive overlay styles only -- `graphVisualStyles()` itself is reused
// unmodified. Border-only, so category fill/shape is never contradicted.
function observationOverlayStyles(): StylesheetJson {
  return [
    { selector: 'node[observation_state = "fresh"]', style: { "border-color": "#357e58", "border-width": 3, "border-style": "solid" } },
    { selector: 'node[observation_state = "stale"]', style: { "border-color": "#c5a52b", "border-width": 3, "border-style": "dashed" } },
    { selector: 'node[observation_state = "unknown"]', style: { "border-color": "#7b8691", "border-width": 2, "border-style": "dotted" } },
  ];
}

// --------------------------------------------------------- INSPECTOR / LIST

function evidenceRow(e: ObservationEvidence, excludedReason?: string | null): HTMLElement {
  const row = el("li", "continuum-evidence-row");
  row.append(
    el("span", "small text-secondary", e.monitor_instance_id),
    badge(FRESHNESS_LABEL[e.freshness], FRESHNESS_TONE[e.freshness]),
    badge(HEALTH_LABEL[e.health], HEALTH_TONE[e.health]),
    badge(CORRELATION_LABEL[e.authority_correlation], CORRELATION_TONE[e.authority_correlation]),
    el("span", "small text-secondary", `observed ${e.age_seconds}s ago`),
  );
  if (e.runtime_usage) {
    const usage = e.runtime_usage;
    row.append(
      el(
        "span",
        "small",
        `CPU ${usage.cpu_percent.toFixed(1)}% · memory ${bytes(usage.memory_used_bytes)} / ${bytes(usage.memory_limit_bytes)} · ${usage.pids} PIDs`,
      ),
    );
  }
  if (excludedReason !== undefined) {
    row.append(badge(excludedReason ? `Excluded: ${excludedReason}` : "Excluded", "neutral"));
  }
  return row;
}

function observationView(observation: NodeObservation): HTMLElement {
  const box = el("div", "continuum-observation");
  const header = el("div", "continuum-observation-header");
  header.append(
    el("span", "small", `${observation.project_id} / ${observation.application_id} / ${observation.service_id}`),
    badge(CONVERGENCE_LABEL[observation.placement_convergence], CONVERGENCE_TONE[observation.placement_convergence]),
  );
  box.append(header);
  if (observation.admitted_evidence.length === 0 && observation.excluded_evidence.length === 0) {
    box.append(el("p", "small text-secondary mb-0", "No current DMonitor evidence."));
    return box;
  }
  const list = el("ul", "continuum-evidence-list");
  for (const e of observation.admitted_evidence) list.append(evidenceRow(e));
  for (const e of observation.excluded_evidence) list.append(evidenceRow(e, e.exclusion_reason));
  box.append(list);
  if (observation.admitted_evidence_truncated || observation.excluded_evidence_truncated)
    box.append(el("p", "small text-secondary", "Additional evidence for this placement was truncated."));
  return box;
}

// Shared by both the inspector panel and the accessible list -- a single
// implementation of the three explicit, never-collapsed sections (Phase
// 135A, section 6): STRUCTURE / ACCEPTED PLACEMENT / DMONITOR OBSERVATION.
function nodeDetail(node: ContinuumNode, projectNodeIds: Set<string> | undefined): HTMLElement {
  const container = el("div", "continuum-node-detail");

  const header = el("div", "continuum-node-header");
  const swatch = el("span", "continuum-category-swatch");
  swatch.style.backgroundColor = GRAPH_CATEGORIES.infrastructure.color;
  swatch.setAttribute("aria-hidden", "true");
  header.append(swatch, el("h2", "h6 mb-0", node.node_id));
  if (projectNodeIds?.has(node.node_id)) header.append(badge("Used by this project", "info"));
  container.append(header);

  container.append(el("h3", "h6 small text-uppercase text-secondary mt-3", "Structure"));
  const structure = el("dl", "continuum-structure-grid");
  const entry = (label: string, value: string) => {
    structure.append(el("dt", "small text-secondary", label), el("dd", "small", value));
  };
  entry("Platform (OS)", node.structure.platform.os);
  entry("Architecture", node.structure.platform.architecture);
  entry("CPU cores (declared)", String(node.structure.capacity.cpu_cores));
  entry("Memory (declared)", bytes(node.structure.capacity.memory_bytes));
  entry("D-Node ABI", node.structure.dnode_abi_versions.length ? node.structure.dnode_abi_versions.join(", ") : "None declared");
  container.append(structure);

  const resourceHeading = el("h3", "h6 small text-uppercase text-secondary mt-3", "Current host resources");
  container.append(resourceHeading);
  if (node.resource_observation) {
    const resource = node.resource_observation;
    const panel = terminalMetricPanel({
      title: `NODE :: ${node.node_id}`,
      subtitle: `${node.structure.platform.os} / ${node.structure.platform.architecture}`,
      metrics: [
        {
          label: "CPU",
          valueText:
            resource.cpu.usage_percent === undefined
              ? "usage unavailable"
              : `${resource.cpu.usage_percent.toFixed(1)}%`,
          ratio:
            resource.cpu.usage_percent === undefined
              ? null
              : resource.cpu.usage_percent / 100,
        },
        {
          label: "MEMORY",
          valueText: `${bytes(resource.memory.used_bytes)} / ${bytes(resource.memory.total_bytes)}`,
          ratio:
            resource.memory.total_bytes > 0
              ? resource.memory.used_bytes / resource.memory.total_bytes
              : null,
        },
        {
          label: "SWAP",
          valueText:
            resource.swap.total_bytes > 0
              ? `${bytes(resource.swap.used_bytes)} / ${bytes(resource.swap.total_bytes)}`
              : "not configured",
          ratio:
            resource.swap.total_bytes > 0
              ? resource.swap.used_bytes / resource.swap.total_bytes
              : null,
        },
      ],
      meta: [
        ["resource", resource.resource_id],
        ["cores", String(resource.cpu.logical_cores)],
        ["mem available", bytes(resource.memory.available_bytes)],
        ["swap available", bytes(resource.swap.available_bytes)],
        ["observed", `${resource.age_seconds}s ago`],
      ],
    });
    container.append(panel.element);
    container.append(
      el(
        "p",
        "small text-secondary mt-2",
        "Observed host resources are physical telemetry only. They do not replace declared planning capacity.",
      ),
    );
  } else {
    container.append(el("p", "small text-secondary", "No current host resource observation."));
  }

  const placementsHeading = el("h3", "h6 small text-uppercase text-secondary mt-3", "Accepted placement");
  placementsHeading.style.color = GRAPH_RELATIONS.placement.color;
  container.append(placementsHeading);
  if (node.accepted_placements.length === 0) {
    container.append(el("p", "small text-secondary", "No accepted workloads are placed on this node."));
  } else {
    const list = el("ul", "continuum-placement-list");
    for (const p of node.accepted_placements)
      list.append(el("li", "small", `${p.project_id} / ${p.application_id} / ${p.service_id}`));
    container.append(list);
    if (node.placements_truncated) container.append(el("p", "small text-secondary", "Additional placements were truncated."));
  }

  const observationHeading = el("h3", "h6 small text-uppercase text-secondary mt-3", "DMonitor observation");
  observationHeading.style.color = GRAPH_RELATIONS.observation.color;
  container.append(observationHeading);
  if (node.observations.length === 0) {
    container.append(el("p", "small text-secondary mb-0", "No current DMonitor evidence."));
  } else {
    for (const observation of node.observations) container.append(observationView(observation));
    if (node.observations_truncated) container.append(el("p", "small text-secondary", "Additional observations were truncated."));
  }
  return container;
}


function placementDetail(
  node: ContinuumNode,
  placement: AcceptedPlacementRef,
  observation: NodeObservation | undefined,
): HTMLElement {
  const container = el("div", "continuum-placement-detail");
  const header = el("div", "continuum-node-header");
  const swatch = el("span", "continuum-category-swatch");
  swatch.style.backgroundColor = GRAPH_CATEGORIES.software.color;
  swatch.setAttribute("aria-hidden", "true");
  header.append(swatch, el("h2", "h6 mb-0", placement.service_id), badge("Accepted placement", "info"));
  container.append(header);

  const identity = el("dl", "continuum-structure-grid");
  const entry = (label: string, value: string) => {
    identity.append(el("dt", "small text-secondary", label), el("dd", "small", value));
  };
  entry("Project", placement.project_id);
  entry("Application", placement.application_id);
  entry("D-Node", node.node_id);
  container.append(identity);

  container.append(el("h3", "h6 small text-uppercase text-secondary mt-3", "Current runtime usage"));
  const usageEvidence =
    observation?.admitted_evidence
      .filter(
        (e) =>
          e.runtime_usage !== undefined &&
          e.freshness === "fresh" &&
          e.authority_correlation === "exact_match",
      )
      .sort((a, b) => a.age_seconds - b.age_seconds) ?? [];
  if (usageEvidence.length === 0) {
    container.append(el("p", "small text-secondary", "No fresh, authority-matched runtime resource telemetry is currently available."));
  } else {
    const evidence = usageEvidence[0];
    const usage = evidence.runtime_usage!;
    const panel = terminalMetricPanel({
      title: `D-SERV :: ${placement.application_id} / ${placement.service_id}`,
      subtitle: `node ${node.node_id}`,
      metrics: [
        {
          label: "CPU",
          valueText: `${usage.cpu_percent.toFixed(1)}%`,
          ratio: usage.cpu_percent / 100,
        },
        {
          label: "MEMORY",
          valueText:
            usage.memory_limit_bytes > 0
              ? `${bytes(usage.memory_used_bytes)} / ${bytes(usage.memory_limit_bytes)}`
              : `${bytes(usage.memory_used_bytes)} / limit unavailable`,
          ratio:
            usage.memory_limit_bytes > 0
              ? usage.memory_used_bytes / usage.memory_limit_bytes
              : null,
        },
        {
          label: "PIDs",
          valueText: String(usage.pids),
          ratio: null,
        },
      ],
      meta: [
        ["freshness", FRESHNESS_LABEL[evidence.freshness]],
        ["authority", evidence.authority_correlation],
        ["monitor", evidence.monitor_instance_id],
        ["observed", `${evidence.age_seconds}s ago`],
      ],
    });
    container.append(panel.element);
  }

  container.append(el("h3", "h6 small text-uppercase text-secondary mt-3", "DMonitor observation"));
  if (observation) container.append(observationView(observation));
  else container.append(el("p", "small text-secondary", "No DMonitor realization evidence for this placement."));
  return container;
}

function nodeCard(node: ContinuumNode, projectNodeIds: Set<string> | undefined, onSelect: (id: string) => void): HTMLElement {
  const card = el("section", "continuum-node-card console-card");
  card.tabIndex = 0;
  card.setAttribute("aria-label", `D-Node ${node.node_id}`);
  card.addEventListener("click", () => onSelect(node.node_id));
  card.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(node.node_id);
    }
  });
  card.append(nodeDetail(node, projectNodeIds));
  return card;
}

export interface ContinuumFilters {
  search: string;
  freshness: "" | DMonitorFreshness;
  hasPlacement: boolean;
  hasObservation: boolean;
}

function matchesFilters(node: ContinuumNode, filters: ContinuumFilters): boolean {
  if (filters.search && !node.node_id.toLowerCase().includes(filters.search.toLowerCase())) return false;
  if (filters.hasPlacement && node.accepted_placements.length === 0) return false;
  if (filters.hasObservation && node.observations.length === 0) return false;
  if (filters.freshness) {
    const found = node.observations.some(
      (o) =>
        o.admitted_evidence.some((e) => e.freshness === filters.freshness) ||
        o.excluded_evidence.some((e) => e.freshness === filters.freshness),
    );
    if (!found) return false;
  }
  return true;
}

export interface ContinuumPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
}

export function continuumView(data: ContinuumView, projectId?: string): ContinuumPageView {
  let disposed = false;
  let cy: Core | undefined;
  let resize: ResizeObserver | undefined;

  const root = el("div", "page-content continuum-view");
  const header = el("header", "page-intro");
  header.append(
    badge(projectId ? "PROJECT CONTINUUM · READ ONLY" : "GLOBAL CONTINUUM · READ ONLY", "info"),
    el("h1", "", "Continuum"),
    el(
      "p",
      "small text-secondary",
      "Physical D-Nodes, accepted placement references, and DMonitor observation. No placement, deployment, or ownership action is available here.",
    ),
  );
  if (projectId) {
    header.append(
      el(
        "p",
        "small text-secondary",
        `Showing the global physical continuum. Accepted workloads for this project are highlighted below as "Used by this project" -- physical nodes are never project-owned.`,
      ),
    );
  }
  root.append(header);

  const metrics = computeContinuumMetrics(data);
  root.append(dashboardView(metrics));

  const filters: ContinuumFilters = { search: "", freshness: "", hasPlacement: false, hasObservation: false };
  const filterForm = el("form", "continuum-filters");
  filterForm.setAttribute("role", "search");
  const searchLabel = el("label", "small", "Node");
  const searchId = "continuum-filter-search";
  searchLabel.htmlFor = searchId;
  const searchInput = el("input", "form-control form-control-sm");
  searchInput.type = "search";
  searchInput.id = searchId;
  filterForm.append(searchLabel, searchInput);

  const freshnessLabel = el("label", "small", "Freshness");
  const freshnessId = "continuum-filter-freshness";
  freshnessLabel.htmlFor = freshnessId;
  const freshnessSelect = el("select", "form-select form-select-sm");
  freshnessSelect.id = freshnessId;
  for (const [value, label] of [
    ["", "Any"],
    ["fresh", "Fresh"],
    ["stale", "Stale"],
    ["clock_skew", "Clock skew"],
  ])
    freshnessSelect.append(new Option(label, value));
  filterForm.append(freshnessLabel, freshnessSelect);

  const placementCheckId = "continuum-filter-placement";
  const placementCheck = el("input", "form-check-input");
  placementCheck.type = "checkbox";
  placementCheck.id = placementCheckId;
  const placementLabel = el("label", "form-check-label small", "Has accepted placement");
  placementLabel.htmlFor = placementCheckId;
  const placementWrap = el("div", "form-check");
  placementWrap.append(placementCheck, placementLabel);
  filterForm.append(placementWrap);

  const observationCheckId = "continuum-filter-observation";
  const observationCheck = el("input", "form-check-input");
  observationCheck.type = "checkbox";
  observationCheck.id = observationCheckId;
  const observationLabel = el("label", "form-check-label small", "Has observation");
  observationLabel.htmlFor = observationCheckId;
  const observationWrap = el("div", "form-check");
  observationWrap.append(observationCheck, observationLabel);
  filterForm.append(observationWrap);
  root.append(filterForm);

  const projectNodeIds = data.project_overlay ? new Set(data.project_overlay.node_ids_in_use) : undefined;

  // ----------------------------------------------------- canvas + inspector
  const grid = el("div", "continuum-workspace-grid");
  const canvasArea = el("div", "continuum-canvas-area");
  const canvas = el("div", "continuum-canvas");
  canvas.setAttribute("role", "img");
  canvas.setAttribute(
    "aria-label",
    "Read-only visual overview of D-Nodes and accepted placements; use the accessible list below for keyboard inspection.",
  );
  const rendererNotice = el("p", "small text-secondary continuum-renderer-notice");
  canvasArea.append(canvas, rendererNotice);
  const inspector = el("section", "continuum-inspector");
  inspector.setAttribute("aria-label", "Selected Continuum element detail");
  const inspectorBody = el("div", "continuum-inspector-body");
  inspectorBody.setAttribute("aria-live", "polite");
  inspector.append(el("h2", "section-title", "Inspector"), inspectorBody);
  const clearInspector = () => {
    inspectorBody.replaceChildren(
      el("p", "small text-secondary", "Select a D-Node or accepted service placement in the canvas to inspect current resources and observation evidence."),
    );
  };
  clearInspector();
  grid.append(canvasArea, inspector);
  root.append(grid);

  function selectNode(nodeId: string): void {
    const node = data.nodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    inspectorBody.replaceChildren(nodeDetail(node, projectNodeIds));
    cy?.elements().unselect();
    cy?.getElementById(nodeId).select();
    const card = list.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(nodeId)}"]`);
    card?.focus();
    card?.scrollIntoView({ block: "nearest" });
  }

  function selectPlacement(
    elementId: string,
    nodeId: string,
    project: string,
    application: string,
    service: string,
  ): void {
    const node = data.nodes.find((candidate) => candidate.node_id === nodeId);
    const placement = node?.accepted_placements.find(
      (candidate) =>
        candidate.project_id === project &&
        candidate.application_id === application &&
        candidate.service_id === service,
    );
    if (!node || !placement) return;
    const observation = node.observations.find(
      (candidate) =>
        candidate.project_id === project &&
        candidate.application_id === application &&
        candidate.service_id === service,
    );
    inspectorBody.replaceChildren(placementDetail(node, placement, observation));
    cy?.elements().unselect();
    cy?.getElementById(elementId).select();
  }

  function draw(): void {
    cy?.destroy();
    cy = undefined;
    try {
      if (!document.createElement("canvas").getContext("2d")) throw new Error("canvas unavailable");
      cy = cytoscape({
        container: canvas,
        elements: continuumElements(data),
        minZoom: 0.15,
        maxZoom: 3,
        boxSelectionEnabled: false,
        layout: { name: "breadthfirst", directed: true, spacingFactor: 1.2 },
        style: [...graphVisualStyles(), ...observationOverlayStyles()],
      });
      cy.fit(undefined, 35);
      cy.on("tap", "node", (e) => {
        const target = e.target;
        if (target.data("kind") === "placement_ref") {
          selectPlacement(
            target.id(),
            String(target.data("parent_node_id")),
            String(target.data("project_id")),
            String(target.data("application_id")),
            String(target.data("service_id")),
          );
        } else {
          selectNode(target.id());
        }
      });
      rendererNotice.textContent = "Pan / zoom; node position is visual only and is never saved. Dashed edges are accepted placement references, not D-Call.";
    } catch {
      cy?.destroy();
      cy = undefined;
      rendererNotice.textContent = "Graph renderer unavailable in this browser. The accessible list below remains fully available.";
    }
  }

  // ------------------------------------------------------------ list + links
  const list = el("div", "continuum-node-list");
  root.append(list);

  function renderList(): void {
    list.replaceChildren();
    const matched = data.nodes.filter((n) => matchesFilters(n, filters));
    if (data.nodes.length === 0) {
      list.append(el("p", "text-secondary", "No D-Nodes are currently registered."));
      return;
    }
    if (matched.length === 0) {
      list.append(el("p", "text-secondary", "No D-Nodes match the current filters."));
      return;
    }
    for (const node of matched) {
      const card = nodeCard(node, projectNodeIds, selectNode);
      card.dataset.nodeId = node.node_id;
      list.append(card);
    }
  }
  searchInput.addEventListener("input", () => {
    filters.search = searchInput.value;
    renderList();
  });
  freshnessSelect.addEventListener("change", () => {
    filters.freshness = freshnessSelect.value as ContinuumFilters["freshness"];
    renderList();
  });
  placementCheck.addEventListener("change", () => {
    filters.hasPlacement = placementCheck.checked;
    renderList();
  });
  observationCheck.addEventListener("change", () => {
    filters.hasObservation = observationCheck.checked;
    renderList();
  });
  renderList();

  const linksSection = el("section", "continuum-links mt-3");
  linksSection.append(el("h2", "h6", "Physical links"));
  if (data.links.length === 0) {
    linksSection.append(el("p", "small text-secondary", "No physical topology links are currently declared."));
  } else {
    const linkList = el("ul", "small");
    for (const link of data.links)
      linkList.append(el("li", "", `${link.from_node_id} -> ${link.to_node_id} (${link.link_id})`));
    linksSection.append(linkList);
  }
  root.append(linksSection);

  if (data.truncated)
    root.append(el("p", "small text-secondary", "This view was bounded; not every D-Node may be shown."));

  return {
    element: root,
    mount: () => {
      if (disposed) return;
      draw();
      if (typeof ResizeObserver !== "undefined") {
        resize = new ResizeObserver(() => cy?.resize());
        resize.observe(canvas);
      }
    },
    dispose: () => {
      disposed = true;
      resize?.disconnect();
      cy?.destroy();
      cy = undefined;
    },
  };
}

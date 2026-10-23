// Phase 142A -- DATUM Live. A continuously-open, cross-project, read-only
// observation surface. It is NOT a reduced Console: no deploy, no
// reconciliation, no acceptance, no DGraph editing, and it introduces no
// new node-health score or threshold policy of its own. Every value shown
// is rendered exactly as DServer returned it; evidence is presented, never
// turned into a fabricated single verdict (see
// documentation/reference/phase142a-datum-live-foundation.md).
//
// Data sources are the existing `GET /api/v1/operations/overview` (global,
// cross-project alerts/events/discovery) and `GET /api/v1/operations/continuum`
// (global physical D-Node inventory, accepted placements, DMonitor
// observation) -- no new endpoint was introduced for this page.
import type { GlobalOverview, OperationalAlert, OperationalEvent } from "../types/operations.ts";
import type { ContinuumNode, ContinuumView } from "../types/continuum.ts";
import { el, badge, severityBadge, severityBadges } from "../components/presentation.ts";
import type { Tone } from "../components/presentation.ts";
import { terminalMetricPanel } from "../components/terminal-metrics.ts";
import { isActiveAttention, subjectLabel, duration as formatDuration, kindLabel } from "../pages/wallboard.ts";
import {
  bytes,
  nodeDetail,
  FRESHNESS_LABEL,
  FRESHNESS_TONE,
  HEALTH_LABEL,
  HEALTH_TONE,
  CONVERGENCE_LABEL,
  CONVERGENCE_TONE,
} from "../pages/continuum.ts";
import { projectPath, applicationOverviewPath } from "../app/router.ts";
import type {
  AlertSeverity,
  LiveViewV1,
} from "./live-profile.ts";
import {
  mergeAndSaveView,
  mergeRemoveAndSaveView,
  newLiveView,
  MIN_REFRESH_INTERVAL_SECONDS,
  MAX_REFRESH_INTERVAL_SECONDS,
  MAX_NAME_LENGTH,
} from "./live-profile.ts";
import { createNodePicker } from "./live-node-picker.ts";

export interface LiveDataSource {
  overview(signal: AbortSignal): Promise<GlobalOverview>;
  continuum(signal: AbortSignal): Promise<ContinuumView>;
}

// Review P1-3 fix: Live's own bound on how many active alerts it lists in
// one place, ALWAYS paired with a visible "more exist" notice -- never a
// silent cap borrowed from an unrelated preview panel's own constant.
const LIVE_ATTENTION_LIST_LIMIT = 50;

// Review P1-5 fix: cross-bundle navigation into the Console must not
// assume same-origin, since production serves Live and the Console from
// two different subdomains. Empty by default (same-origin, e.g. local dev
// where both are served by the same Vite server).
function consoleBaseUrl(): string {
  // Written as the literal `import.meta.env.DATUM_CONSOLE_URL` expression
  // (no cast on the object) so Vite's `define` (see vite.config.ts, review
  // P2-B) can statically replace exactly this reference -- never routed
  // through the broader `DATUM_*` env family.
  const configured = import.meta.env.DATUM_CONSOLE_URL;
  return configured ? configured.replace(/\/$/, "") : "";
}

// ------------------------------------------------------------- pure logic

// Preserves the backend's own `attention_ordered` relative order -- never
// re-sorted. Only the subject variants that actually carry `node_id`
// (`dnode`, `dserv`, `runtime_instance`, `dependency_edge`) are indexed;
// every other alert subject carries no node correlation at all and is
// honestly excluded from every node bucket, never guessed.
export function correlateAlertsByNode(alerts: OperationalAlert[]): Map<string, OperationalAlert[]> {
  const index = new Map<string, OperationalAlert[]>();
  for (const alert of alerts) {
    const nodeId = alert.subject.node_id;
    if (!nodeId) continue;
    const bucket = index.get(nodeId);
    if (bucket) bucket.push(alert);
    else index.set(nodeId, [alert]);
  }
  return index;
}
export function correlateEventsByNode(events: OperationalEvent[]): Map<string, OperationalEvent[]> {
  const index = new Map<string, OperationalEvent[]>();
  for (const event of events) {
    const nodeId = event.subject.node_id;
    if (!nodeId) continue;
    const bucket = index.get(nodeId);
    if (bucket) bucket.push(event);
    else index.set(nodeId, [event]);
  }
  return index;
}

export interface ResolvedPin {
  nodeId: string;
  node: ContinuumNode | undefined;
}
// Preserves the operator's own pinned order; a pinned node absent from the
// current continuum is kept in the list as `node: undefined` -- never
// silently dropped (Phase 142A brief, "Live sem... node_id que deixou de
// existir").
export function resolvePins(pinnedNodeIds: string[], nodes: ContinuumNode[]): ResolvedPin[] {
  const byId = new Map(nodes.map((n) => [n.node_id, n] as const));
  return pinnedNodeIds.map((nodeId) => ({ nodeId, node: byId.get(nodeId) }));
}

export interface GlobalCounts {
  knownProjects: number | undefined;
  knownNodes: number | undefined;
  activeAlerts: number | undefined;
  recentEvents: number | undefined;
  sourceIssues: number | undefined;
}
export function computeGlobalCounts(overview: GlobalOverview | undefined, continuum: ContinuumView | undefined): GlobalCounts {
  return {
    knownProjects: overview?.known_projects.page.items.length,
    knownNodes: continuum?.nodes.length,
    // The real count of active alerts in the loaded page -- never passed
    // through `activeAttention`'s own `.slice(0, 8)` preview bound, which
    // exists for the Wallboard panel, not for this count (review P1-3).
    activeAlerts:
      overview?.alerts.availability === "available"
        ? overview.alerts.value.attention_ordered.items.filter(isActiveAttention).length
        : undefined,
    recentEvents: overview?.recent_event_count.availability === "available" ? overview.recent_event_count.value : undefined,
    sourceIssues: overview?.source_issues.length,
  };
}

export interface ActivityFilters {
  severities: AlertSeverity[];
  projectIds: string[];
  nodeId: string;
}
export function filterAlerts(alerts: OperationalAlert[], filters: ActivityFilters): OperationalAlert[] {
  return alerts.filter(
    (a) =>
      (filters.severities.length === 0 || filters.severities.includes(a.severity as AlertSeverity)) &&
      (filters.projectIds.length === 0 || (a.project_id !== null && filters.projectIds.includes(a.project_id))) &&
      (filters.nodeId === "" || a.subject.node_id === filters.nodeId),
  );
}
export function filterEvents(events: OperationalEvent[], filters: Pick<ActivityFilters, "projectIds" | "nodeId">): OperationalEvent[] {
  return events.filter(
    (e) =>
      (filters.projectIds.length === 0 || (e.project_id !== null && filters.projectIds.includes(e.project_id))) &&
      (filters.nodeId === "" || e.subject.node_id === filters.nodeId),
  );
}

// --------------------------------------------------------------- render

// ---------------------------------------------------- node card tone/state
//
// Glanceability follow-up: a single PRESENTATION tone per card, derived
// ONLY from already-canonical backend evidence (alert severity, DMonitor
// convergence/health/freshness). This is never a new health authority --
// no CPU/memory threshold, no age_seconds cutoff, no node_id/hostname
// heuristic ever contributes to this tone. Absence of alerts never, by
// itself, produces a positive/healthy claim; it only ever reaches
// "informational" if there is at least one real observation to point to,
// and "neutral" otherwise.
export interface NodeCardState {
  tone: Tone;
  label: string;
}
const CARD_STATE_LABEL: Record<"danger" | "warning" | "info" | "neutral", string> = {
  danger: "Attention required",
  warning: "Warning",
  info: "Observed",
  neutral: "No current evidence",
};
function alertSeverityTone(severity: string): Tone {
  return severityBadges[severity]?.[1] ?? "neutral";
}
export function computeNodeCardState(node: ContinuumNode | undefined, activeAlerts: OperationalAlert[]): NodeCardState {
  if (!node) return { tone: "neutral", label: CARD_STATE_LABEL.neutral };
  // Strongest tone among the node's own active alerts, reusing the EXACT
  // same severity->tone mapping the rest of the Console already uses
  // (`severityBadges`) -- never a second, independently-invented ranking.
  let alertTone: Tone = "neutral";
  for (const alert of activeAlerts) {
    const tone = alertSeverityTone(alert.severity);
    if (tone === "danger") {
      alertTone = "danger";
      break;
    }
    // Reaching here means `tone` was not "danger" this iteration, and any
    // earlier "danger" assignment would already have broken the loop --
    // so `alertTone` can only be "neutral" or "warning" at this point.
    if (tone === "warning") alertTone = "warning";
    else if (tone === "info" && alertTone === "neutral") alertTone = "info";
  }
  const admittedEvidence = node.observations.flatMap((o) => o.admitted_evidence);
  const hasUnhealthyOrDiverged =
    node.observations.some((o) => o.placement_convergence === "diverged") ||
    admittedEvidence.some((e) => e.health === "unhealthy");
  const hasDegradedOrStale = admittedEvidence.some((e) => e.health === "degraded" || e.freshness === "stale");
  const hasAnyEvidence = node.observations.length > 0 || Boolean(node.resource_observation);

  if (alertTone === "danger" || hasUnhealthyOrDiverged) return { tone: "danger", label: CARD_STATE_LABEL.danger };
  if (alertTone === "warning" || hasDegradedOrStale) return { tone: "warning", label: CARD_STATE_LABEL.warning };
  if (alertTone === "info" || hasAnyEvidence) return { tone: "info", label: CARD_STATE_LABEL.info };
  return { tone: "neutral", label: CARD_STATE_LABEL.neutral };
}

function cpuRatio(usagePercent: number | undefined): number | null {
  return usagePercent === undefined ? null : Math.max(0, Math.min(1, usagePercent / 100));
}

function compactNodeCard(nodeId: string, node: ContinuumNode | undefined, activeAlerts: OperationalAlert[], onOpen: (nodeId: string) => void): HTMLElement {
  const state = computeNodeCardState(node, activeAlerts);
  const card = el("section", `console-card live-node-card live-node-card--${state.tone}`);
  card.tabIndex = 0;
  card.setAttribute("role", "button");
  card.setAttribute("aria-label", `D-Node ${nodeId} -- ${state.label} -- open detail`);
  const open = () => onOpen(nodeId);
  card.addEventListener("click", open);
  card.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      open();
    }
  });
  if (!node) {
    card.append(
      el("h3", "live-node-card-title mb-1", nodeId),
      badge("Not currently available", "neutral"),
      el("p", "small text-secondary mb-0 mt-1 live-node-card-prose", "This D-Node is not present in the current continuum registry."),
    );
    return card;
  }
  const header = el("div", "live-node-card-header");
  header.append(el("h3", "live-node-card-title mb-0", nodeId));
  // Explicit text label, never color alone -- the strongest available
  // backend-provided signal (alert severity or canonical DMonitor state),
  // reusing the same tone/label the rest of this function also applies to
  // the card's border/background.
  header.append(badge(state.label, state.tone));
  card.append(header);
  if (activeAlerts.length > 0) card.append(badge(`${activeAlerts.length} active alert(s)`, state.tone === "danger" ? "danger" : "warning"));

  const resource = node.resource_observation;
  if (resource) {
    const memRatio = resource.memory.total_bytes > 0 ? resource.memory.used_bytes / resource.memory.total_bytes : null;
    const panel = terminalMetricPanel({
      title: "HOST (OBSERVED)",
      metrics: [
        {
          label: "CPU",
          ratio: cpuRatio(resource.cpu.usage_percent),
          valueText: resource.cpu.usage_percent === undefined ? "n/a" : `${resource.cpu.usage_percent.toFixed(1)}%`,
        },
        {
          label: "MEMORY",
          ratio: memRatio,
          valueText: `${bytes(resource.memory.used_bytes)} / ${bytes(resource.memory.total_bytes)}`,
        },
      ],
      meta: [["Observed", `${resource.age_seconds}s ago`]],
    });
    card.append(panel.element);
  } else {
    card.append(el("p", "small text-secondary mb-0 live-node-card-prose", "No current host resource observation."));
  }

  if (node.observations.length > 0) {
    const chips = el("div", "live-node-card-chips");
    for (const observation of node.observations) {
      chips.append(badge(CONVERGENCE_LABEL[observation.placement_convergence], CONVERGENCE_TONE[observation.placement_convergence]));
    }
    card.append(chips);
    const latestEvidence = node.observations.flatMap((o) => o.admitted_evidence)[0];
    if (latestEvidence) {
      const evidenceChips = el("div", "live-node-card-chips");
      evidenceChips.append(
        badge(FRESHNESS_LABEL[latestEvidence.freshness], FRESHNESS_TONE[latestEvidence.freshness]),
        badge(HEALTH_LABEL[latestEvidence.health], HEALTH_TONE[latestEvidence.health]),
      );
      card.append(evidenceChips);
    }
  } else {
    // Continuum node observations are placement-scoped (one entry per
    // accepted placement referencing this node) -- this must never read as
    // "no DMonitor observation exists anywhere for this node".
    card.append(el("p", "small text-secondary mb-0 live-node-card-prose", "No placement-scoped DMonitor evidence."));
  }

  const placementProjects = [...new Set(node.accepted_placements.map((p) => p.project_id))];
  card.append(
    el(
      "p",
      "small text-secondary mb-0 live-node-card-prose",
      placementProjects.length
        ? `${node.accepted_placements.length} accepted placement(s) · ${placementProjects.join(", ")}`
        : "No accepted workloads placed on this node.",
    ),
  );
  return card;
}

function alertRow(alert: OperationalAlert): HTMLElement {
  const row = el("article", "attention-item");
  const heading = el("div", "attention-heading");
  heading.append(
    severityBadge(alert.severity),
    el("h3", "attention-condition", kindLabel(alert.kind)),
    badge(alert.lifecycle_status.replaceAll("_", " "), "neutral"),
  );
  const scope = el("div", "attention-scope");
  // Plain cross-bundle navigation into the Console entry point -- see the
  // identical note on the drawer's application links below.
  const projectLink = alert.project_id ? el("a", "project-link", alert.project_id) : el("span", "small", "DServer global");
  if (alert.project_id) (projectLink as HTMLAnchorElement).href = consoleBaseUrl() + projectPath(alert.project_id);
  scope.append(projectLink, el("span", "small", subjectLabel(alert.subject)));
  if (alert.application_id) scope.append(el("span", "small text-secondary", `Application · ${alert.application_id}`));
  row.append(
    heading,
    scope,
    el(
      "p",
      "small text-secondary mb-0",
      `${formatDuration(alert.condition_duration_seconds)} condition duration · ${alert.condition_confirmation_count} confirmations`,
    ),
  );
  return row;
}

function eventRow(event: OperationalEvent): HTMLElement {
  const row = el("article", "event-row");
  row.append(
    badge(event.classification.replaceAll("_", " "), event.classification === "recovery" ? "success" : "neutral"),
    el("strong", "small", kindLabel(event.kind)),
    event.project_id ? el("span", "small", event.project_id) : el("span", "small", "DServer global"),
    el("span", "small text-secondary", subjectLabel(event.subject)),
  );
  row.title = `${event.event_id} · occurred ${event.occurred_at_utc} · admitted ${event.admitted_at_utc}`;
  return row;
}

export interface LiveController {
  element: HTMLElement;
  dispose: () => void;
}

export function liveView(
  storage: Pick<Storage, "getItem" | "setItem">,
  dataSource: LiveDataSource,
  initialProfiles: LiveViewV1[],
  activeProfileId: string,
  onProfileChanged: (id: string) => void,
): LiveController {
  let disposed = false;
  let profiles = initialProfiles;
  let active = profiles.find((p) => p.id === activeProfileId) ?? profiles[0]!;
  let overview: GlobalOverview | undefined;
  let continuum: ContinuumView | undefined;
  let lastSuccessfulFetchUtc: string | undefined;
  let lastError: string | undefined;
  let paused = false;
  let fetchInFlight = false;
  let controller: AbortController | undefined;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let openDrawerNodeId: string | undefined;
  const activityFilters: ActivityFilters = { severities: active.filters?.severities ?? [], projectIds: active.filters?.project_ids ?? [], nodeId: "" };

  // Review P1-1 fix: always re-read the store immediately before writing,
  // so a profile created/edited in another tab/window in the meantime is
  // never silently clobbered by a stale in-memory snapshot.
  const persistActive = () => {
    profiles = mergeAndSaveView(storage, active);
  };

  const root = el("div", "page-content live-view");
  const header = el("header", "live-header");
  const titleRow = el("div", "live-title-row");
  titleRow.append(badge("DATUM LIVE · READ ONLY", "info"));
  const nameInput = el("input", "form-control form-control-sm live-name-input");
  nameInput.value = active.name;
  nameInput.setAttribute("aria-label", "Live name");
  nameInput.maxLength = MAX_NAME_LENGTH;
  titleRow.append(nameInput);
  const statusRow = el("div", "live-status-row");
  const connectionBadge = el("span", "status-badge tone-neutral", "Connecting");
  const ageLabel = el("span", "small text-secondary", "No update yet");
  const pauseButton = el("button", "btn btn-sm btn-outline-secondary", "Pause");
  const fullscreenButton = el("button", "btn btn-sm btn-outline-secondary", "Fullscreen");
  const consoleLink = el("a", "small", "Open in Console → Continuum");
  consoleLink.href = `${consoleBaseUrl()}/continuum`;
  statusRow.append(connectionBadge, ageLabel, pauseButton, fullscreenButton, consoleLink);
  header.append(titleRow, statusRow);

  const profileRow = el("div", "live-profile-row");
  const profileSelect = el("select", "form-select form-select-sm live-profile-select");
  const newProfileButton = el("button", "btn btn-sm btn-outline-secondary", "New Live");
  const deleteProfileButton = el("button", "btn btn-sm btn-outline-danger", "Delete");
  profileRow.append(profileSelect, newProfileButton, deleteProfileButton);
  header.append(profileRow);
  root.append(header);

  const countsStrip = el("div", "metric-grid summary-strip live-counts-strip");
  root.append(countsStrip);

  const aoiSection = el("section", "live-section console-card");
  const aoiHeader = el("div", "live-node-card-header");
  aoiHeader.append(el("h2", "section-title mb-0", "Area of Interest"));
  const manageNodesButton = el("button", "btn btn-sm btn-outline-secondary live-manage-nodes-button", "Manage nodes");
  aoiHeader.append(manageNodesButton);
  aoiSection.append(aoiHeader);
  const cardsGrid = el("div", "live-node-grid");
  aoiSection.append(cardsGrid);
  root.append(aoiSection);

  const drawer = el("aside", "live-drawer");
  drawer.hidden = true;
  drawer.setAttribute("aria-label", "D-Node detail");
  const drawerClose = el("button", "btn btn-sm btn-outline-secondary live-drawer-close", "Close");
  const drawerBody = el("div", "live-drawer-body");
  drawer.append(drawerClose, drawerBody);
  root.append(drawer);

  // One shared alert->node index, computed fresh from whatever `overview`
  // currently holds -- never a second alert-correlation implementation.
  function currentAlertsByNode(): Map<string, OperationalAlert[]> {
    return overview?.alerts.availability === "available"
      ? correlateAlertsByNode(overview.alerts.value.attention_ordered.items)
      : new Map<string, OperationalAlert[]>();
  }
  // Real count, never truncated to the Wallboard's own 8-item preview bound.
  function countActiveAlerts(alerts: OperationalAlert[]): number {
    return alerts.filter(isActiveAttention).length;
  }
  function activeAlertsOnly(alerts: OperationalAlert[]): OperationalAlert[] {
    return alerts.filter(isActiveAttention);
  }

  const nodePicker = createNodePicker(
    () => continuum,
    currentAlertsByNode,
    () => active.pinned_node_ids,
    countActiveAlerts,
    (nextPinned) => {
      active = { ...active, pinned_node_ids: nextPinned };
      persistActive();
      renderAreaOfInterest();
    },
  );
  root.append(nodePicker.element);
  manageNodesButton.addEventListener("click", () => nodePicker.open());

  const attentionSection = el("section", "live-section console-card");
  attentionSection.append(el("h2", "section-title", "Attention / Alarms"));
  const attentionFilterRow = el("div", "live-filter-row");
  const severitySelect = el("select", "form-select form-select-sm");
  severitySelect.multiple = true;
  for (const s of ["info", "warning", "high", "critical"] as const) severitySelect.append(new Option(s, s));
  const projectInput = el("input", "form-control form-control-sm");
  projectInput.placeholder = "Filter by project id…";
  const nodeFilterSelect = el("select", "form-select form-select-sm");
  nodeFilterSelect.append(new Option("All nodes", ""));
  attentionFilterRow.append(severitySelect, projectInput, nodeFilterSelect);
  attentionSection.append(attentionFilterRow);
  const attentionList = el("div", "attention-list");
  attentionSection.append(attentionList);
  root.append(attentionSection);

  const recentSection = el("section", "live-section console-card");
  recentSection.append(el("h2", "section-title", "Recent Activity"));
  const recentList = el("div", "event-list");
  recentSection.append(recentList);
  root.append(recentSection);

  function setConnectionBadge(label: string, tone: "neutral" | "success" | "warning" | "danger"): void {
    connectionBadge.textContent = label;
    connectionBadge.className = `status-badge tone-${tone}`;
  }

  function renderProfileSelect(): void {
    profileSelect.replaceChildren(...profiles.map((p) => new Option(p.name, p.id, false, p.id === active.id)));
  }

  function renderCounts(): void {
    const counts = computeGlobalCounts(overview, continuum);
    countsStrip.replaceChildren();
    const metric = (label: string, value: number | undefined) => {
      const box = el("div", "metric-card console-card");
      box.append(el("div", "section-title", label), el("div", "metric-value", value === undefined ? "—" : value.toLocaleString()));
      countsStrip.append(box);
    };
    metric("Known projects", counts.knownProjects);
    metric("Known D-Nodes", counts.knownNodes);
    metric("Active alerts (preview)", counts.activeAlerts);
    metric("Recent events (24h)", counts.recentEvents);
    metric("Source issues", counts.sourceIssues);
  }

  function openDrawer(nodeId: string): void {
    openDrawerNodeId = nodeId;
    drawer.hidden = false;
    renderDrawer();
  }
  function closeDrawer(): void {
    openDrawerNodeId = undefined;
    drawer.hidden = true;
  }
  drawerClose.addEventListener("click", closeDrawer);

  function renderDrawer(): void {
    if (!openDrawerNodeId || drawer.hidden) return;
    const node = continuum?.nodes.find((n) => n.node_id === openDrawerNodeId);
    drawerBody.replaceChildren();
    if (!node) {
      drawerBody.append(
        el("h2", "h6", openDrawerNodeId),
        badge("Not currently available", "neutral"),
        el("p", "small text-secondary", "This D-Node is not present in the current continuum registry."),
      );
      return;
    }
    const projectOverlay = continuum?.project_overlay ? new Set(continuum.project_overlay.node_ids_in_use) : undefined;
    drawerBody.append(nodeDetail(node, projectOverlay));
    if (node.accepted_placements.length > 0) {
      drawerBody.append(el("h3", "h6 small text-uppercase text-secondary mt-3", "Open in Console"));
      const links = el("ul", "live-drawer-links small mb-0");
      const seenApplications = new Set<string>();
      for (const placement of node.accepted_placements) {
        const key = `${placement.project_id}/${placement.application_id}`;
        if (seenApplications.has(key)) continue;
        seenApplications.add(key);
        const item = el("li");
        // A plain cross-bundle navigation (full page load into the Console
        // entry point) -- Live never mounts the Console's SPA router, so
        // these links must never carry `data-route`.
        const link = el("a", "", `${placement.project_id} / ${placement.application_id} →`);
        link.href = consoleBaseUrl() + applicationOverviewPath(placement.project_id, placement.application_id);
        item.append(link);
        links.append(item);
      }
      drawerBody.append(links);
    }
    const nodeAlerts = (currentAlertsByNode().get(openDrawerNodeId) ?? []).filter(isActiveAttention);
    drawerBody.append(el("h3", "h6 small text-uppercase text-secondary mt-3", "Active alerts referencing this node"));
    if (nodeAlerts.length === 0) drawerBody.append(el("p", "small text-secondary mb-0", "No active alerts reference this node."));
    else {
      for (const alert of nodeAlerts.slice(0, LIVE_ATTENTION_LIST_LIMIT)) drawerBody.append(alertRow(alert));
      if (nodeAlerts.length > LIVE_ATTENTION_LIST_LIMIT)
        drawerBody.append(el("p", "small text-secondary", `${nodeAlerts.length - LIVE_ATTENTION_LIST_LIMIT} more active alert(s) for this node are not shown in this preview.`));
    }
  }

  function renderAreaOfInterest(): void {
    cardsGrid.replaceChildren();
    const alertsByNode = currentAlertsByNode();
    const pins = resolvePins(active.pinned_node_ids, continuum?.nodes ?? []);
    if (pins.length === 0) {
      cardsGrid.append(el("p", "empty-text", "No D-Nodes selected yet. Use “Manage nodes” above to start watching one here."));
      return;
    }
    for (const pin of pins) {
      const card = compactNodeCard(pin.nodeId, pin.node, activeAlertsOnly(alertsByNode.get(pin.nodeId) ?? []), openDrawer);
      const removeButton = el("button", "btn btn-sm btn-outline-secondary live-node-remove", "×");
      removeButton.setAttribute("aria-label", `Remove ${pin.nodeId} from Area of Interest`);
      removeButton.addEventListener("click", (event) => {
        event.stopPropagation();
        active = { ...active, pinned_node_ids: active.pinned_node_ids.filter((id) => id !== pin.nodeId) };
        persistActive();
        renderAreaOfInterest();
      });
      card.append(removeButton);
      cardsGrid.append(card);
    }
  }

  // Review P1-2 fix: rebuilding this select's options (every poll tick,
  // since the known node list can change) must preserve whichever option
  // the operator currently has selected -- previously this silently reset
  // the filter to "All nodes" on every refresh.
  function renderNodeFilterOptions(): void {
    const previousValue = nodeFilterSelect.value;
    nodeFilterSelect.replaceChildren(
      new Option("All nodes", ""),
      ...(continuum?.nodes ?? []).map((n) => new Option(n.node_id, n.node_id, false, n.node_id === previousValue)),
    );
    nodeFilterSelect.value = previousValue;
  }

  function renderAttentionAndActivity(): void {
    attentionList.replaceChildren();
    if (overview?.alerts.availability === "available") {
      const active_ = overview.alerts.value.attention_ordered.items.filter(isActiveAttention);
      const filtered = filterAlerts(active_, activityFilters);
      if (filtered.length === 0) attentionList.append(el("p", "empty-text", "No active alerts match the current filters."));
      else {
        for (const alert of filtered.slice(0, LIVE_ATTENTION_LIST_LIMIT)) attentionList.append(alertRow(alert));
        if (filtered.length > LIVE_ATTENTION_LIST_LIMIT)
          attentionList.append(
            el("p", "small text-secondary", `${filtered.length - LIVE_ATTENTION_LIST_LIMIT} more active alert(s) match these filters but are not shown in this preview.`),
          );
      }
      if (overview.alerts.value.attention_ordered.has_more)
        attentionList.append(el("p", "small text-secondary", "More alerts exist beyond this loaded page."));
    } else {
      attentionList.append(el("p", "small text-secondary", "Alert evidence is not currently available."));
    }
    recentList.replaceChildren();
    if (overview?.recent_events.availability === "available") {
      const filtered = filterEvents(overview.recent_events.value.items, activityFilters);
      if (filtered.length === 0) recentList.append(el("p", "empty-text", "No recent events match the current filters."));
      else for (const event of filtered) recentList.append(eventRow(event));
      if (overview.recent_events.value.has_more)
        recentList.append(el("p", "small text-secondary", "More events exist beyond this loaded page."));
    } else {
      recentList.append(el("p", "small text-secondary", "Recent event evidence is not currently available."));
    }
  }

  function renderAll(): void {
    renderProfileSelect();
    renderCounts();
    renderNodeFilterOptions();
    renderAreaOfInterest();
    renderAttentionAndActivity();
    renderDrawer();
    ageLabel.textContent = fetchInFlight
      ? `Refreshing… (last update ${lastSuccessfulFetchUtc ? new Date(lastSuccessfulFetchUtc).toLocaleTimeString() : "never"})`
      : lastSuccessfulFetchUtc
        ? `Updated ${new Date(lastSuccessfulFetchUtc).toLocaleTimeString()}`
        : lastError
          ? "Never updated successfully"
          : "Loading…";
    if (document.activeElement !== nameInput) nameInput.value = active.name;
    root.classList.toggle("live-kiosk", active.kiosk_mode);
    root.classList.toggle("live-hide-secondary", active.hide_secondary_controls);
    pauseButton.textContent = paused ? "Resume" : "Pause";
  }

  // Review P1-4 fix: an operator-settable name (e.g. "Lab principal", "TV
  // do laboratório") persisted on blur/Enter -- previously a Live profile
  // could never be given a meaningful name at all.
  function commitName(): void {
    const trimmed = nameInput.value.trim();
    if (trimmed && trimmed !== active.name) {
      active = { ...active, name: trimmed.slice(0, MAX_NAME_LENGTH) };
      persistActive();
      renderProfileSelect();
    } else {
      nameInput.value = active.name;
    }
  }
  nameInput.addEventListener("blur", commitName);
  nameInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      nameInput.blur();
    }
  });

  severitySelect.addEventListener("change", () => {
    activityFilters.severities = [...severitySelect.selectedOptions].map((o) => o.value as AlertSeverity);
    active = { ...active, filters: { ...active.filters, severities: activityFilters.severities } };
    persistActive();
    renderAttentionAndActivity();
  });
  projectInput.addEventListener("input", () => {
    activityFilters.projectIds = projectInput.value.trim() ? [projectInput.value.trim()] : [];
    active = { ...active, filters: { ...active.filters, project_ids: activityFilters.projectIds } };
    persistActive();
    renderAttentionAndActivity();
  });
  nodeFilterSelect.addEventListener("change", () => {
    activityFilters.nodeId = nodeFilterSelect.value;
    renderAttentionAndActivity();
  });
  profileSelect.addEventListener("change", () => {
    const next = profiles.find((p) => p.id === profileSelect.value);
    if (!next) return;
    active = next;
    activityFilters.severities = active.filters?.severities ?? [];
    activityFilters.projectIds = active.filters?.project_ids ?? [];
    activityFilters.nodeId = "";
    onProfileChanged(active.id);
    renderAll();
  });
  newProfileButton.addEventListener("click", () => {
    const created = newLiveView(`Live ${profiles.length + 1}`);
    profiles = mergeAndSaveView(storage, created);
    active = created;
    onProfileChanged(active.id);
    renderAll();
  });
  deleteProfileButton.addEventListener("click", () => {
    const remaining = mergeRemoveAndSaveView(storage, active.id);
    // Never land on zero profiles, even if this was the only one left.
    profiles = remaining.length > 0 ? remaining : mergeAndSaveView(storage, newLiveView("Default"));
    active = profiles[0]!;
    onProfileChanged(active.id);
    renderAll();
  });
  pauseButton.addEventListener("click", () => {
    paused = !paused;
    pauseButton.textContent = paused ? "Resume" : "Pause";
    if (!paused) scheduleNextPoll(0);
  });
  fullscreenButton.addEventListener("click", () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void root.requestFullscreen?.();
  });

  function scheduleNextPoll(delayMs: number): void {
    if (pollTimer !== undefined) clearTimeout(pollTimer);
    if (disposed || paused) return;
    pollTimer = setTimeout(() => void poll(), delayMs);
  }

  async function poll(): Promise<void> {
    if (disposed || paused) return;
    controller?.abort();
    const myController = new AbortController();
    controller = myController;
    fetchInFlight = true;
    setConnectionBadge(lastSuccessfulFetchUtc ? "Refreshing" : "Connecting", "neutral");
    ageLabel.textContent = `Refreshing… (last update ${lastSuccessfulFetchUtc ? new Date(lastSuccessfulFetchUtc).toLocaleTimeString() : "never"})`;
    // Review P1-7 fix: `allSettled`, not `all` -- one source failing must
    // never discard the other source's freshly-successful result. Each of
    // `overview`/`continuum` is updated independently; whichever source
    // failed simply keeps its last good value (never cleared).
    const [overviewResult, continuumResult] = await Promise.allSettled([
      dataSource.overview(myController.signal),
      dataSource.continuum(myController.signal),
    ]);
    if (disposed || myController.signal.aborted) return;
    try {
      let anyFailure = false;
      if (overviewResult.status === "fulfilled") overview = overviewResult.value;
      else anyFailure = true;
      if (continuumResult.status === "fulfilled") continuum = continuumResult.value;
      else anyFailure = true;
      if (!anyFailure) {
        lastSuccessfulFetchUtc = new Date().toISOString();
        lastError = undefined;
        setConnectionBadge("Connected", "success");
      } else {
        const failure = overviewResult.status === "rejected" ? overviewResult.reason : continuumResult.status === "rejected" ? continuumResult.reason : undefined;
        lastError = failure instanceof Error ? failure.message : "One of the Live data sources could not be refreshed.";
        // A partial success still counts as a successful refresh cycle for
        // the age label -- whichever source DID succeed is now current.
        if (overviewResult.status === "fulfilled" || continuumResult.status === "fulfilled") lastSuccessfulFetchUtc = new Date().toISOString();
        setConnectionBadge(lastSuccessfulFetchUtc ? "Degraded" : "Disconnected", lastSuccessfulFetchUtc ? "warning" : "danger");
      }
      renderAll();
    } finally {
      // Always resets and reschedules, even if the branch above throws --
      // a rendering bug must never silently stop the continuous poll loop.
      fetchInFlight = false;
      scheduleNextPoll(Math.max(MIN_REFRESH_INTERVAL_SECONDS, Math.min(MAX_REFRESH_INTERVAL_SECONDS, active.refresh_interval_seconds)) * 1000);
    }
  }

  renderAll();
  scheduleNextPoll(0);

  return {
    element: root,
    dispose: () => {
      disposed = true;
      controller?.abort();
      if (pollTimer !== undefined) clearTimeout(pollTimer);
    },
  };
}

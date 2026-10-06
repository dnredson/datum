// Phase 140B -- Historical Explorer page. Read-only inspection/navigation
// over the Phase 140A canonical historical read model
// (`GET /api/v1/dmonitor/history/:project_id`). This page renders backend
// data exactly as returned -- it never recomputes health, freshness,
// convergence, authority correlation, clock quality, or desired state, and
// never reads `active_placement_authority` or any other authority source.
// If convergence/findings are absent from the canonical response (they
// are, by Phase 140A's own design -- see that module's doc comment), this
// page never fabricates them.
import { el, badge } from "../components/presentation.ts";
import { DMonitorHistoryClient, DMonitorHistoryError } from "../api/dmonitor-history.ts";
import type { DMonitorHistoryQueryResponse, HistoricalDMonitorObservation } from "../types/dmonitor-history.ts";
import {
  HEALTH_LABEL,
  HEALTH_TONE,
  FRESHNESS_LABEL,
  FRESHNESS_TONE,
  defaultHistoryFilters,
  clientSideHistoryValidationError,
  historyObservationIdentityLine,
} from "./dmonitor-history-model.ts";

export interface HistoryPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
  dirty: () => boolean;
  confirmLeave: () => boolean;
}

function observationDetail(observation: HistoricalDMonitorObservation): HTMLElement {
  const detail = el("div", "history-observation-detail");
  const grid = el("dl", "history-detail-grid");
  const entry = (label: string, value: string) => grid.append(el("dt", "small text-secondary", label), el("dd", "small", value));
  entry("Observation id", observation.observation_id);
  entry("Node", observation.node_id);
  entry("Observed at (agent-asserted)", observation.observed_at_utc);
  entry("Received at (server-asserted)", observation.received_at_utc);
  entry("Monitor instance (diagnostic only -- not canonical identity)", observation.monitor_instance_id);
  entry("Sequence (diagnostic only)", String(observation.sequence));
  entry("Collector", observation.collector);
  detail.append(grid);
  const factsHeading = el("h4", "h6 small text-uppercase text-secondary mt-2", "Collector facts (raw, as persisted)");
  detail.append(factsHeading);
  const facts = el("pre", "history-facts small");
  facts.textContent = JSON.stringify(observation.facts, null, 2);
  detail.append(facts);
  return detail;
}

function observationRow(observation: HistoricalDMonitorObservation, expanded: boolean, onToggle: () => void): HTMLElement {
  const row = el("div", `history-observation-row console-card${expanded ? " expanded" : ""}`);
  const summary = el("button", "history-observation-summary btn btn-sm btn-outline-secondary w-100 text-start");
  summary.type = "button";
  summary.setAttribute("aria-expanded", String(expanded));
  summary.addEventListener("click", onToggle);

  const line1 = el("div", "history-observation-line1");
  line1.append(el("span", "small entity-identity", observation.received_at_utc));
  line1.append(el("span", "small entity-identity ms-2", observation.node_id));
  line1.append(el("span", "small text-secondary ms-2", historyObservationIdentityLine(observation.subject)));
  summary.append(line1);

  const line2 = el("div", "history-observation-line2 mt-1");
  line2.append(badge(HEALTH_LABEL[observation.health], HEALTH_TONE[observation.health]));
  line2.append(badge(FRESHNESS_LABEL[observation.freshness], FRESHNESS_TONE[observation.freshness]));
  summary.append(line2);

  row.append(summary);
  if (expanded) row.append(observationDetail(observation));
  return row;
}

export function historyView(
  projectId: string,
  client: DMonitorHistoryClient,
  onReady: (ready: boolean) => void,
): HistoryPageView {
  let disposed = false;
  let sequence = 0;
  let filters = defaultHistoryFilters();
  let response: DMonitorHistoryQueryResponse | undefined;
  let loaded = false;
  let errorMessage: string | undefined;
  let expandedObservationId: string | undefined;

  const root = el("div", "page-content history-view");
  const header = el("header", "page-intro");
  header.append(
    badge("PROJECT HISTORY · READ ONLY", "info"),
    el("h1", "", "Historical Explorer"),
    el("p", "small text-secondary", `Project: ${projectId}`),
    el(
      "p",
      "small text-secondary",
      "Canonical, persisted DMonitor evidence over a bounded time window. This is a VIEW over what was actually recorded -- it never recomputes or reinterprets current placement authority, and it never shows a convergence verdict for a past observation.",
    ),
  );
  root.append(header);

  const form = el("form", "history-filters console-card");
  form.setAttribute("role", "search");
  form.setAttribute("aria-label", "Historical observation filters");

  function field(label: string, id: string, type: string, value: string): HTMLInputElement {
    const wrap = el("div", "history-field");
    const labelEl = el("label", "small", label);
    labelEl.htmlFor = id;
    const input = el("input", "form-control form-control-sm");
    input.type = type;
    input.id = id;
    input.value = value;
    wrap.append(labelEl, input);
    form.append(wrap);
    return input;
  }

  const startInput = field("Start (RFC 3339 UTC)", "history-start", "text", filters.startUtc);
  const endInput = field("End (RFC 3339 UTC)", "history-end", "text", filters.endUtc);
  const applicationInput = field("Application (optional)", "history-application", "text", filters.applicationId);
  const nodeInput = field("Node (optional)", "history-node", "text", filters.nodeId);
  const serviceInput = field("Service / logical subject (optional)", "history-service", "text", filters.serviceId);
  const limitInput = field("Result limit (optional)", "history-limit", "number", filters.limit);

  const submitButton = el("button", "btn btn-sm btn-primary mt-2", "Query");
  submitButton.type = "submit";
  form.append(submitButton);
  root.append(form);

  const validationBox = el("p", "small text-danger history-validation", "");
  validationBox.setAttribute("role", "alert");
  validationBox.hidden = true;
  root.append(validationBox);

  const body = el("div", "history-results");
  root.append(body);

  function readFormIntoFilters(): void {
    filters = {
      startUtc: startInput.value.trim(),
      endUtc: endInput.value.trim(),
      applicationId: applicationInput.value.trim(),
      nodeId: nodeInput.value.trim(),
      serviceId: serviceInput.value.trim(),
      limit: limitInput.value.trim(),
    };
  }

  function renderResults(): void {
    body.replaceChildren();
    if (!loaded) {
      body.append(el("p", "small text-secondary", "Loading historical observations…"));
      return;
    }
    if (errorMessage) {
      const box = el("section", "console-card state-view");
      box.setAttribute("role", "alert");
      box.append(el("h2", "h6", "Historical query failed"), el("p", "small text-secondary mb-0", errorMessage));
      body.append(box);
      return;
    }
    if (!response) return;
    const summary = el("p", "small text-secondary");
    summary.textContent = `${response.count} observation(s) between ${response.start_utc} and ${response.end_utc}, limit ${response.limit}.`;
    body.append(summary);
    if (response.truncated) {
      body.append(badge("More observations matched than the result limit -- narrow the window or raise the limit", "warning"));
    }
    if (response.observations.length === 0) {
      body.append(el("p", "small text-secondary", "No DMonitor evidence was recorded in this range for the given filters."));
      return;
    }
    const list = el("div", "history-observation-list");
    for (const observation of response.observations) {
      list.append(
        observationRow(observation, observation.observation_id === expandedObservationId, () => {
          expandedObservationId = expandedObservationId === observation.observation_id ? undefined : observation.observation_id;
          renderResults();
        }),
      );
    }
    body.append(list);
  }

  async function load(): Promise<void> {
    const token = ++sequence;
    loaded = false;
    errorMessage = undefined;
    renderResults();
    try {
      const result = await client.query({
        startUtc: filters.startUtc || undefined,
        endUtc: filters.endUtc || undefined,
        applicationId: filters.applicationId || undefined,
        nodeId: filters.nodeId || undefined,
        serviceId: filters.serviceId || undefined,
        limit: filters.limit ? Number(filters.limit) : undefined,
      });
      if (disposed || token !== sequence) return;
      response = result;
      loaded = true;
      expandedObservationId = undefined;
      renderResults();
      if (!disposed) onReady(true);
    } catch (e) {
      if (disposed || token !== sequence) return;
      loaded = true;
      response = undefined;
      // Part K -- a backend 422 carries its own specific, meaningful
      // range/filter explanation; surface it directly rather than a
      // generic failure message.
      errorMessage =
        e instanceof DMonitorHistoryError ? e.message : "The historical query could not be completed.";
      renderResults();
      if (!disposed) onReady(false);
    }
  }

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    readFormIntoFilters();
    const problem = clientSideHistoryValidationError(filters);
    validationBox.hidden = !problem;
    validationBox.textContent = problem ?? "";
    if (problem) return;
    void load();
  });

  readFormIntoFilters();
  void load();

  return {
    element: root,
    mount: () => {},
    dispose: () => {
      disposed = true;
      sequence++;
    },
    dirty: () => false,
    confirmLeave: () => true,
  };
}

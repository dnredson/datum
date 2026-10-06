// Phase 141B -- Application Operational Context page. Read-only
// presentation over the canonical, already-built backend overview
// (`GET /api/v1/operations/projects/:project_id/applications/:application_id`).
// This page never recomputes health, freshness, convergence, readiness, or
// desired state -- every value shown is rendered exactly as the backend
// returned it. Convergence and readiness are rendered as two
// STRUCTURALLY SEPARATE sections, never merged into one status; neither
// implies business/SLA health.
import { el, badge } from "../components/presentation.ts";
import { ApplicationOverviewClient, ApplicationOverviewError } from "../api/application-overview.ts";
import { projectPath, applicationDeployPath } from "../app/router.ts";
import type { ApplicationOperationsOverview } from "../types/application-overview.ts";
import type { SourceSection } from "../types/operations.ts";
import {
  READINESS_LABEL,
  READINESS_TONE,
  sectionAvailabilityLabel,
} from "./application-overview-model.ts";
import {
  CONVERGENCE_LABEL,
  CONVERGENCE_TONE,
} from "./dmonitor-convergence-model.ts";
import { HEALTH_LABEL, HEALTH_TONE, subjectDisplayIdentity, subjectKindLabel } from "./dmonitor-history-model.ts";

export interface ApplicationOverviewPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
  dirty: () => boolean;
  confirmLeave: () => boolean;
}

// Shared rendering for any `SourceSection<T>` -- honest availability
// states, never a second ad hoc "available: boolean" mechanism (Part J/AF).
function sectionBox<T>(title: string, section: SourceSection<T> | undefined, renderAvailable: (value: T) => HTMLElement[]): HTMLElement {
  const box = el("section", "console-card application-overview-section");
  box.append(el("h2", "h6", title));
  if (!section) {
    box.append(el("p", "small text-secondary", "Loading…"));
    return box;
  }
  if (section.availability === "available") {
    box.append(...renderAvailable(section.value));
    return box;
  }
  if (section.availability === "not_configured") {
    box.append(el("p", "small text-secondary mb-0", "Not configured."));
    return box;
  }
  if (section.availability === "indeterminate") {
    box.append(badge("Indeterminate", "neutral"), el("p", "small text-secondary mb-0 mt-1", section.value));
    return box;
  }
  box.append(
    badge(sectionAvailabilityLabel(section.availability), section.availability === "corrupt" ? "danger" : "warning"),
    el("p", "small text-secondary mb-0 mt-1", section.value.safe_message),
  );
  return box;
}

function findingsList(findings: { code: string; message: string }[]): HTMLElement {
  if (findings.length === 0) return el("p", "small text-secondary mb-0", "No findings.");
  const list = el("ul", "application-overview-findings small mb-0");
  for (const f of findings) list.append(el("li", "", `${f.code}: ${f.message}`));
  return list;
}

export function applicationOverviewView(
  projectId: string,
  applicationId: string,
  client: ApplicationOverviewClient,
  onReady: (ready: boolean) => void,
): ApplicationOverviewPageView {
  let disposed = false;
  let sequence = 0;
  let data: ApplicationOperationsOverview | undefined;
  let loaded = false;
  let errorMessage: string | undefined;

  const root = el("div", "page-content application-overview-view");
  const header = el("header", "page-intro");
  header.append(
    badge("APPLICATION OPERATIONAL CONTEXT · READ ONLY", "info"),
    el("h1", "", applicationId),
    el("p", "small text-secondary", `Project: ${projectId}`),
    el(
      "p",
      "small text-secondary",
      "A derived, non-atomic snapshot composed from several independently-sourced, independently-timestamped read models -- never a transactional point-in-time read, and never an authority of its own. Convergence (is the current placement correct) and Readiness (a fail-closed Ready/Not Ready governance gate) are kept separate on purpose and never collapsed into one status.",
    ),
  );
  const links = el("nav", "application-overview-links small");
  const link = (label: string, href: string) => {
    const a = el("a", "me-3");
    a.textContent = label;
    a.href = href;
    a.dataset.route = "";
    links.append(a);
  };
  link("Placement", `${projectPath(projectId)}/placement`);
  link("Deploy / Reconciliation", applicationDeployPath(projectId, applicationId));
  link("Continuum", `${projectPath(projectId)}/continuum`);
  link("History", `${projectPath(projectId)}/history`);
  header.append(links);
  root.append(header);

  const body = el("div", "application-overview-body");
  root.append(body);

  function renderAll(): void {
    body.replaceChildren();
    if (!loaded) {
      body.append(el("p", "small text-secondary", "Loading application operational context…"));
      return;
    }
    if (errorMessage) {
      const box = el("section", "console-card state-view");
      box.setAttribute("role", "alert");
      box.append(el("h2", "h6", "Application overview failed to load"), el("p", "small text-secondary mb-0", errorMessage));
      body.append(box);
      return;
    }
    if (!data) return;

    body.append(
      sectionBox("Current placement", data.accepted, (accepted) => {
        if (!accepted) return [el("p", "small text-secondary mb-0", "No accepted placement for this application.")];
        const grid = el("dl", "application-overview-grid");
        const entry = (label: string, value: string) => grid.append(el("dt", "small text-secondary", label), el("dd", "small", value));
        entry("D-Map", accepted.dmap_id);
        entry("Revision", String(accepted.revision));
        entry("Digest", accepted.digest);
        entry("Service → node", Object.entries(accepted.service_nodes).map(([s, n]) => `${s} → ${n}`).join(", ") || "—");
        return [grid];
      }),
    );

    body.append(
      sectionBox("Planned (proposal/candidate -- not current desired authority)", data.planned, (page) => {
        if (page.items.length === 0) return [el("p", "small text-secondary mb-0", "No pending proposals for this application.")];
        const list = el("ul", "application-overview-list small mb-0");
        for (const p of page.items) list.append(el("li", "", `${p.proposal_id} · ${p.lifecycle} · created ${p.created_at_utc}`));
        if (page.has_more) list.append(el("li", "text-secondary", "More pending proposals exist beyond this limit."));
        return [list];
      }),
    );

    body.append(
      sectionBox("Desired runtime (per accepted D-Map)", data.desired_runtime, (page) => {
        if (page.items.length === 0) return [el("p", "small text-secondary mb-0", "No desired runtime entries for this application.")];
        const list = el("ul", "application-overview-list small mb-0");
        for (const d of page.items) list.append(el("li", "", `${d.service_id} · ${d.runtime} · desired: ${d.desired_state}`));
        if (page.has_more) list.append(el("li", "text-secondary", "More desired runtime entries exist beyond this limit."));
        return [list];
      }),
    );

    body.append(
      sectionBox("Readiness", data.readiness, (readiness) => [
        badge(READINESS_LABEL[readiness.readiness], READINESS_TONE[readiness.readiness]),
        el("p", "small text-secondary", `Evaluated at ${readiness.evaluated_at_utc}. Fail-closed operational summary -- not business/application health.`),
        findingsList(readiness.findings.items),
      ]),
    );

    body.append(
      sectionBox("Convergence", data.convergence, (convergence) => [
        badge(CONVERGENCE_LABEL[convergence.convergence], CONVERGENCE_TONE[convergence.convergence]),
        el("p", "small text-secondary", `Evaluated at ${convergence.evaluated_at_utc}. Desired services: ${convergence.desired_service_count}. Unexpected realizations: ${convergence.unexpected_realization_count}.`),
        findingsList(convergence.findings.items),
      ]),
    );

    body.append(
      sectionBox("Observed runtime", data.observed_runtime, (page) => {
        if (page.items.length === 0) return [el("p", "small text-secondary mb-0", "No current DMonitor evidence.")];
        const list = el("div", "application-overview-observed-list");
        for (const o of page.items) {
          const row = el("div", "application-overview-observed-row");
          const { applicationId: appId, serviceId } = subjectDisplayIdentity(o.subject);
          row.append(
            el("span", "small entity-identity me-2", subjectKindLabel(o.subject)),
            el("span", "small text-secondary me-2", [appId && `app: ${appId}`, serviceId && `service: ${serviceId}`].filter(Boolean).join(" · ")),
            badge(HEALTH_LABEL[o.reported_health], HEALTH_TONE[o.reported_health]),
          );
          list.append(row);
        }
        if (page.has_more) list.append(badge("More observations exist beyond this limit", "warning"));
        return [list];
      }),
    );

    body.append(
      sectionBox("Latest command activity (command-side)", data.reconciliation_execution, (value) => [
        el("p", "small text-secondary mb-0", `${value} -- a command result is never the same fact as current observed runtime above.`),
      ]),
    );

    body.append(
      sectionBox("Alerts", data.alerts, (alerts) => {
        const total = Object.values(alerts.counts).reduce((a, b) => a + b, 0);
        if (total === 0) return [el("p", "small text-secondary mb-0", "No alerts recorded for this application.")];
        const grid = el("dl", "application-overview-grid");
        for (const [key, value] of Object.entries(alerts.counts)) {
          grid.append(el("dt", "small text-secondary", key), el("dd", "small", String(value)));
        }
        return [grid];
      }),
    );

    body.append(
      sectionBox("Recent events", data.recent_events, (page) => {
        if (page.items.length === 0) return [el("p", "small text-secondary mb-0", "No recent events recorded for this application.")];
        const list = el("ul", "application-overview-events small mb-0");
        for (const e of page.items) list.append(el("li", "", `${e.occurred_at_utc} (admitted ${e.admitted_at_utc}) · ${e.category}/${e.kind} · ${e.severity}`));
        if (page.has_more) list.append(el("li", "text-secondary", "More events exist beyond this limit."));
        return [list];
      }),
    );

    body.append(
      sectionBox("Recent monitoring evidence", data.recent_history, (history) => {
        const parts: HTMLElement[] = [
          el("p", "small mb-1", `Recent observations: ${history.recent_observation_count}`),
          el(
            "p",
            "small text-secondary mb-0",
            history.latest_received_at_utc
              ? `Latest observation received at ${history.latest_received_at_utc} (id ${history.latest_observation_id}).`
              : "No monitoring evidence recorded yet.",
          ),
        ];
        const historyLink = el("a", "small", "View History →");
        historyLink.href = `${projectPath(projectId)}/history`;
        historyLink.dataset.route = "";
        parts.push(historyLink);
        return parts;
      }),
    );
  }

  async function load(): Promise<void> {
    const token = ++sequence;
    loaded = false;
    errorMessage = undefined;
    renderAll();
    try {
      const result = await client.get(applicationId);
      if (disposed || token !== sequence) return;
      data = result;
      loaded = true;
      renderAll();
      if (!disposed) onReady(true);
    } catch (e) {
      if (disposed || token !== sequence) return;
      loaded = true;
      data = undefined;
      errorMessage = e instanceof ApplicationOverviewError ? e.message : "The application overview could not be loaded.";
      renderAll();
      if (!disposed) onReady(false);
    }
  }

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

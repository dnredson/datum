import type {
  GlobalOverview,
  ProjectOverview,
  SourceSection,
} from "../types/operations.ts";
import {
  availability,
  badge,
  coverageView,
  discoveryEmptyMessage,
  el,
  sectionValue,
  temporalView,
} from "../components/presentation.ts";
import { projectPath, validProjectId } from "../app/router.ts";

function card(title: string, description?: string): HTMLElement {
  const box = el("section", "console-card");
  box.append(el("h2", "section-title", title));
  if (description) box.append(el("p", "text-secondary small", description));
  return box;
}
function link(id: string): HTMLAnchorElement {
  const a = el("a", "project-link", id);
  if (validProjectId(id)) {
    a.href = projectPath(id);
    a.dataset.route = "";
  } else {
    a.setAttribute("aria-disabled", "true");
    a.title =
      "This discovered identity is not supported by the foundation route contract.";
  }
  return a;
}
function metric(
  title: string,
  source: SourceSection<number>,
  footnote: string,
): HTMLElement {
  const box = card(title);
  box.classList.add("metric-card");
  box.append(
    sectionValue(source, (count) =>
      el("div", "metric-value", count.toLocaleString()),
    ),
    el("p", "metric-note", footnote),
  );
  return box;
}
export { wallboardView as overviewView } from "./wallboard.ts";
export function projectsView(data: GlobalOverview): HTMLElement {
  const content = el("div", "page-content");
  const intro = el("div", "page-intro");
  intro.append(
    el("h1", "", "Known projects"),
    el(
      "p",
      "text-secondary",
      "Project references discovered from available operational sources. This is not an “all projects” registry.",
    ),
  );
  content.append(intro);
  const box = card("Discovered references");
  box.append(coverageView(data.known_projects.coverage));
  const page = data.known_projects.page;
  if (!page.items.length)
    box.append(
      el(
        "p",
        "empty-text mt-4",
        discoveryEmptyMessage(data.known_projects.coverage),
      ),
    );
  else {
    const list = el("div", "project-list mt-4");
    for (const id of page.items) {
      const row = el("div", "project-row");
      row.append(
        link(id),
        el("span", "small text-secondary", "Read-only project shell"),
      );
      list.append(row);
    }
    box.append(list);
  }
  if (page.has_more)
    box.append(
      el(
        "p",
        "small text-secondary mt-3",
        `Showing the first ${page.limit} references. Discovery pagination is not exposed by this foundation API.`,
      ),
    );
  content.append(box);
  return content;
}
export function projectView(data: ProjectOverview): HTMLElement {
  const content = el("div", "page-content");
  const intro = el("div", "page-intro");
  intro.append(
    badge("PROJECT CONTEXT", "info"),
    el("h1", "", data.project_id),
    el(
      "p",
      "text-secondary",
      "Read-only project foundation. Scope is not authorization.",
    ),
  );
  content.append(intro);
  const metrics = el("div", "metric-grid");
  const active: SourceSection<number> =
    data.alerts.availability === "available"
      ? {
          availability: "available",
          value:
            data.alerts.value.counts.open +
            data.alerts.value.counts.acknowledged +
            data.alerts.value.counts.in_progress,
        }
      : data.alerts;
  const resolved: SourceSection<number> =
    data.alerts.availability === "available"
      ? {
          availability: "available",
          value: data.alerts.value.counts.resolved_pending_close,
        }
      : data.alerts;
  const events: SourceSection<number> =
    data.recent_events.availability === "available"
      ? {
          availability: "available",
          value: data.recent_events.value.items.length,
        }
      : data.recent_events;
  metrics.append(
    metric(
      "Active alerts",
      active,
      "Condition lifecycle, not infrastructure health.",
    ),
    metric("Pending closure", resolved, "Resolved, not operationally active."),
    metric(
      "Recent events loaded",
      events,
      "Bounded page, not a total historical count.",
    ),
  );
  content.append(metrics);
  const apps = card("Known application references");
  apps.append(coverageView(data.known_applications.coverage));
  if (!data.known_applications.page.items.length)
    apps.append(
      el(
        "p",
        "empty-text mt-3",
        "No applications are currently discoverable from the inspected sources.",
      ),
    );
  for (const app of data.known_applications.page.items) {
    const row = el("div", "project-row");
    row.append(
      el("strong", "small", app.application_id),
      el(
        "span",
        "small text-secondary",
        app.provenance
          .map(
            (value) =>
              (
                ({
                  accepted_dmap: "Accepted D-Map",
                  ddeploy_proposal: "D-Deploy proposal",
                  operational_graph: "Operational deployment document",
                  dmonitor: "D-Monitor evidence",
                  event: "Event history",
                  alert: "Alert lifecycle",
                }) as Record<string, string>
              )[value] ?? value.replaceAll("_", " "),
          )
          .join(" · "),
      ),
    );
    apps.append(row);
  }
  if (data.known_applications.page.has_more)
    apps.append(
      el(
        "p",
        "small text-secondary mt-3",
        "Additional application references are not shown in this bounded page.",
      ),
    );
  content.append(apps);
  const temporal = card(
    "Temporal evidence",
    "Communication freshness, observation freshness and source-time quality are independent. No Down/Offline claim is derived here.",
  );
  temporal.append(
    sectionValue(data.temporal_quality, (page) => {
      const list = el("div");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No temporal evidence is available in this snapshot.",
          ),
        );
      page.items.slice(0, 5).forEach((v) => list.append(temporalView(v)));
      if (page.has_more || page.items.length > 5)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Temporal evidence preview is limited to five records.",
          ),
        );
      return list;
    }),
  );
  content.append(temporal);
  const future = card(
    "Workspace foundation",
    "Planned, accepted, desired runtime, observed runtime and convergence remain distinct backend contracts. Full workspace screens belong to later phases.",
  );
  const row = el("div", "source-row");
  row.append(
    el("span", "", "Accepted D-Map source"),
    availability(data.accepted_deployments),
  );
  future.append(row);
  content.append(future);
  content.append(
    el(
      "p",
      "snapshot-note",
      `Generated ${data.generated_at_utc} · Independent read-only source snapshots.`,
    ),
  );
  return content;
}

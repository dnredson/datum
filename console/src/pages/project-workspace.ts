import type {
  ProjectOverview,
  SourceSection,
  SourceIssue,
  KnownApplication,
  OperationalEvent,
} from "../types/operations.ts";
import {
  el,
  badge,
  availability,
  sectionValue,
  coverageView,
  severityBadge,
  lifecycleBadge,
  temporalView,
} from "../components/presentation.ts";
import { projectPath, applicationOverviewPath } from "../app/router.ts";
import {
  activeAttention,
  subjectLabel,
  kindLabel,
  duration,
} from "./wallboard.ts";

export const PROJECT_LIMITS = {
  applications: 20,
  attention: 8,
  temporal: 10,
  events: 10,
  accepted: 10,
} as const;
export const PROJECT_SECTIONS = [
  ["Overview", "130"],
  ["Services", "133A / 133B"],
  ["Graph", "131 / 132"],
  ["Continuum", "135"],
  ["Placement", "136"],
  ["Deploy", "138"],
  ["Runtime", "139"],
  ["History", "140"],
] as const;
const provenanceNames: Record<string, string> = {
  accepted_dmap: "Accepted D-Map",
  ddeploy_proposal: "D-Deploy proposal reference",
  operational_graph: "Operational deployment document",
  dmonitor: "D-Monitor evidence",
  event: "Event history",
  alert: "Alert lifecycle",
};
export function provenanceLabel(app: KnownApplication): string {
  return app.provenance
    .map((v) => provenanceNames[v] ?? v.replaceAll("_", " "))
    .join(" · ");
}
export function applicationPreview(
  data: ProjectOverview,
  applicationId: string,
) {
  const accepted =
    data.accepted_deployments.availability === "available"
      ? data.accepted_deployments.value.items.find(
          (a) => a.application_id === applicationId,
        )
      : undefined;
  const alertPage =
    data.alerts.availability === "available"
      ? data.alerts.value.attention_ordered
      : null;
  return {
    accepted,
    activeAlertRecords:
      alertPage === null
        ? null
        : alertPage.items.filter(
            (a) =>
              a.application_id === applicationId &&
              a.is_operationally_active &&
              ["open", "acknowledged", "in_progress"].includes(
                a.lifecycle_status,
              ),
          ).length,
  };
}
export function projectIssues(data: ProjectOverview): SourceIssue[] {
  const issues = new Map<string, SourceIssue>();
  const add = (issue: SourceIssue) =>
    issues.set(
      JSON.stringify([
        issue.source,
        issue.code,
        issue.scope,
        issue.safe_message,
      ]),
      issue,
    );
  for (const section of [
    data.operational_graph,
    data.canonical_application_count,
    data.alerts,
    data.recent_events,
    data.accepted_deployments,
    data.temporal_quality,
  ])
    if (
      section.availability === "unavailable" ||
      section.availability === "corrupt"
    )
      add(section.value);
  for (const source of Object.values(
    data.known_applications.coverage.source_coverage,
  ))
    if (source.issue) add(source.issue);
  return [...issues.values()].sort(
    (a, b) =>
      a.source.localeCompare(b.source) ||
      a.code.localeCompare(b.code) ||
      a.safe_message.localeCompare(b.safe_message),
  );
}
function card(title: string, name: string, explanation?: string) {
  const box = el("section", `console-card project-${name}`);
  box.append(el("h2", "section-title", title));
  if (explanation) box.append(el("p", "small text-secondary", explanation));
  return box;
}
function metric(label: string, section: SourceSection<number>, note: string) {
  const box = card(label, "metric");
  box.classList.add("metric-card");
  box.append(
    sectionValue(section, (n) => el("div", "metric-value", n.toLocaleString())),
    el("p", "metric-note", note),
  );
  return box;
}
function pageCount<T>(
  section: SourceSection<{ items: T[] }>,
): SourceSection<number> {
  return section.availability === "available"
    ? { availability: "available", value: section.value.items.length }
    : section;
}
function stamp(value: string, label: string) {
  const time = el(
    "time",
    "small text-secondary",
    `${label} · ${new Date(value).toLocaleString()}`,
  );
  time.dateTime = value;
  time.title = `${label}: ${value}`;
  return time;
}
function unavailableDimension(label: string, reason: string) {
  const row = el("div", "application-dimension");
  row.append(
    el("span", "small", label),
    badge("Not included", "neutral"),
    el("span", "small text-secondary", reason),
  );
  return row;
}
export function evidenceScope(
  event: OperationalEvent,
  workspaceProject: string,
) {
  return event.project_id === null
    ? "unscoped"
    : event.project_id === workspaceProject
      ? "project_local"
      : "cross_project";
}
function correlatedEvidence(
  event: OperationalEvent,
  project: string,
  label: string,
) {
  const box = el("div", "correlated-evidence small text-secondary mt-2");
  box.dataset.scope = evidenceScope(event, project);
  box.append(
    el(
      "p",
      "mb-1",
      box.dataset.scope === "cross_project"
        ? `${label} · correlated evidence from another project`
        : box.dataset.scope === "unscoped"
          ? `${label} · correlated evidence without explicit project scope`
          : `${label} · project-local correlated evidence`,
    ),
  );
  if (event.project_id !== null)
    box.append(
      el("p", "entity-identity mb-1", `Evidence project · ${event.project_id}`),
    );
  if (event.application_id !== null)
    box.append(
      el(
        "p",
        "entity-identity mb-1",
        `Evidence application · ${event.application_id}`,
      ),
    );
  box.append(
    el("p", "entity-identity mb-1", `Evidence · ${event.event_id}`),
    el("p", "entity-identity mb-1", `Subject · ${subjectLabel(event.subject)}`),
    stamp(event.occurred_at_utc, "Evidence occurrence"),
  );
  return box;
}
export function projectWorkspaceView(data: ProjectOverview): HTMLElement {
  const content = el("div", "page-content project-workspace");
  const breadcrumb = el("nav", "workspace-breadcrumb");
  breadcrumb.setAttribute("aria-label", "Breadcrumb");
  const projects = el("a", "", "Projects");
  projects.href = "/projects";
  projects.dataset.route = "";
  breadcrumb.append(
    projects,
    el("span", "", "/"),
    el("span", "entity-identity", data.project_id),
    el("span", "", "/ Overview"),
  );
  const header = el("header", "page-intro project-header");
  header.append(
    badge("PROJECT WORKSPACE", "info"),
    el("h1", "entity-identity", data.project_id),
    el(
      "p",
      "text-secondary",
      "Read-only operational context. Project scope is not authorization; no canonical project health score.",
    ),
  );
  const headerAttention = el("div", "identity-context header-attention");
  headerAttention.append(el("span", "small", "Active project attention"));
  if (data.alerts.availability === "available") {
    const counts = data.alerts.value.counts;
    const count = counts.open + counts.acknowledged + counts.in_progress;
    headerAttention.append(
      badge(`${count} active alerts`, count ? "warning" : "neutral"),
    );
  } else headerAttention.append(availability(data.alerts));
  header.append(headerAttention);
  content.append(breadcrumb, header);
  const nav = el("nav", "project-subnav");
  nav.setAttribute("aria-label", "Project workspace sections");
  for (const [name, phase] of PROJECT_SECTIONS) {
    if (name === "Overview" || name === "Graph" || name === "Continuum" || name === "Placement" || name === "History") {
      const link = el(
        "a",
        `project-section${name === "Overview" ? " current" : ""}`,
        name,
      );
      link.href =
        projectPath(data.project_id) +
        (name === "Graph" ? "/graph" : name === "Continuum" ? "/continuum" : name === "Placement" ? "/placement" : name === "History" ? "/history" : "");
      link.dataset.route = "";
      if (name === "Overview") link.setAttribute("aria-current", "page");
      nav.append(link);
    } else {
      const item = el(
        "span",
        "project-section planned",
        `${name} · Phase ${phase}`,
      );
      item.setAttribute("aria-disabled", "true");
      nav.append(item);
    }
  }
  content.append(nav);
  const counts =
    data.alerts.availability === "available" ? data.alerts.value.counts : null;
  const active: SourceSection<number> = counts
    ? {
        availability: "available",
        value: counts.open + counts.acknowledged + counts.in_progress,
      }
    : (data.alerts as SourceSection<number>);
  const resolved: SourceSection<number> = counts
    ? { availability: "available", value: counts.resolved_pending_close }
    : (data.alerts as SourceSection<number>);
  const issues = projectIssues(data);
  const strip = el("div", "metric-grid project-summary");
  strip.append(
    metric(
      "Known applications loaded",
      {
        availability: "available",
        value: data.known_applications.page.items.length,
      },
      `Discovered references, not registry. Coverage: ${data.known_applications.coverage.completeness}.`,
    ),
    metric("Active alerts", active, "Open / Acknowledged / In progress."),
    metric("Pending closure", resolved, "Resolved, not active attention."),
    metric(
      "Recent events loaded",
      pageCount(data.recent_events),
      "Bounded occurrence window; not history total.",
    ),
    metric(
      "Monitoring subjects loaded",
      pageCount(data.temporal_quality),
      "Evidence streams, not node inventory.",
    ),
    metric(
      "Accepted D-Maps loaded",
      pageCount(data.accepted_deployments),
      "Placement authority projection; not pending proposals.",
    ),
    metric(
      "Source issues reported",
      { availability: "available", value: issues.length },
      "Read/projection diagnostics, not runtime failures.",
    ),
  );
  content.append(strip);
  const context = card("Project context", "context");
  context.append(
    coverageView(data.known_applications.coverage),
    el(
      "p",
      "small text-secondary mt-2 mb-0",
      "A route identifies a requested scope, not proof of registration. No discoverable references or unreadable sources do not prove that this project does not exist.",
    ),
  );
  content.append(context);
  const primary = el("div", "workspace-grid project-primary");
  const attention = card(
    "Attention Now",
    "attention",
    "ALERT · project-scoped condition lifecycle in backend attention order. Confirmation count is not outage count.",
  );
  attention.append(
    sectionValue(data.alerts, (section) => {
      const list = el("div", "attention-list");
      const items = activeAttention(section.attention_ordered.items);
      if (!items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No active alerts in this loaded projection. This is not a positive runtime-health assertion.",
          ),
        );
      for (const a of items) {
        const row = el("article", "attention-item");
        const title = el("div", "attention-heading");
        title.append(
          severityBadge(a.severity),
          el("h3", "attention-condition", kindLabel(a.kind)),
          lifecycleBadge(a.lifecycle_status),
        );
        const scope = el("div", "identity-context");
        scope.append(
          el("span", "small entity-identity", subjectLabel(a.subject)),
        );
        if (a.application_id !== null)
          scope.append(
            el("span", "small", `Application · ${a.application_id}`),
          );
        row.append(
          title,
          scope,
          el(
            "p",
            "small mt-2 mb-1",
            `${a.condition_confirmation_count} confirmations · ${duration(a.condition_duration_seconds)} condition duration`,
          ),
          correlatedEvidence(
            a.latest_condition_event,
            data.project_id,
            "Latest condition evidence",
          ),
        );
        list.append(row);
      }
      if (
        section.attention_ordered.has_more ||
        section.attention_ordered.items.filter((a) => a.is_operationally_active)
          .length > items.length
      )
        list.append(
          el(
            "p",
            "small text-secondary",
            "Loaded attention preview is bounded; more items may require review.",
          ),
        );
      const recoveries = section.attention_ordered.items.filter(
        (a) => a.recovery_event,
      );
      if (recoveries.length) {
        const details = el("details", "mt-3 recovery-references");
        details.append(
          el(
            "summary",
            "small",
            "Recovery evidence · loaded alert references (not active attention)",
          ),
        );
        for (const a of recoveries.slice(0, PROJECT_LIMITS.attention)) {
          const reference = el("article", "mt-2");
          reference.append(
            el("p", "small entity-identity mb-1", `Alert · ${a.alert_id}`),
            lifecycleBadge(a.lifecycle_status),
            correlatedEvidence(
              a.recovery_event!,
              data.project_id,
              "Recovery evidence",
            ),
          );
          details.append(reference);
        }
        if (recoveries.length > PROJECT_LIMITS.attention)
          details.append(
            el("p", "small", "Recovery-reference preview is bounded."),
          );
        list.append(details);
      }
      return list;
    }),
  );
  primary.append(attention);
  const applications = card(
    "Known applications",
    "applications",
    "Discovered references with provenance. Dimensions remain independent; application-detail data is not fetched here.",
  );
  for (const app of data.known_applications.page.items.slice(
    0,
    PROJECT_LIMITS.applications,
  )) {
    const row = el("article", "application-item");
    const heading = el("h3", "small fw-semibold entity-identity");
    const overviewLink = el("a", "", app.application_id);
    overviewLink.href = applicationOverviewPath(data.project_id, app.application_id);
    overviewLink.dataset.route = "";
    heading.append(overviewLink);
    row.append(
      heading,
      el(
        "p",
        "small text-secondary application-provenance",
        provenanceLabel(app),
      ),
    );
    const preview = applicationPreview(data, app.application_id);
    const accepted = el("div", "application-dimension");
    accepted.append(el("span", "small", "Accepted placement"));
    if (preview.accepted)
      accepted.append(
        el(
          "span",
          "small entity-identity",
          `${preview.accepted.dmap_id} · revision ${preview.accepted.revision}`,
        ),
      );
    else if (data.accepted_deployments.availability !== "available")
      accepted.append(availability(data.accepted_deployments));
    else accepted.append(badge("Not in loaded accepted page", "neutral"));
    row.append(
      accepted,
      unavailableDimension(
        "Planned / pending proposal",
        "Proposal provenance is history, not proof of pending state.",
      ),
      unavailableDimension(
        "Desired runtime",
        "Project graph has aggregate assignments, not per-application desired runtime.",
      ),
      unavailableDimension(
        "Observed runtime",
        "Temporal receipt evidence is not a runtime realization.",
      ),
      unavailableDimension(
        "Convergence",
        "Application convergence projection is not included in this response.",
      ),
    );
    const alertState = el(
      "div",
      "application-dimension application-alert-state",
    );
    alertState.append(
      el("span", "small", "Alert source state"),
      availability(data.alerts),
    );
    row.append(alertState);
    if (preview.activeAlertRecords !== null)
      row.append(
        el(
          "p",
          "small mt-2 mb-0 application-alert-count",
          `${preview.activeAlertRecords} active alert records loaded for this application; bounded preview, not full count.`,
        ),
      );
    applications.append(row);
  }
  if (!data.known_applications.page.items.length)
    applications.append(
      el(
        "p",
        "empty-text",
        "No applications are currently discoverable from the inspected sources. This is not a canonical zero-application claim.",
      ),
    );
  if (
    data.known_applications.page.has_more ||
    data.known_applications.page.items.length > PROJECT_LIMITS.applications
  )
    applications.append(
      el(
        "p",
        "small text-secondary",
        "Application preview is bounded; additional references may exist beyond the displayed/loaded page. Pagination is not exposed.",
      ),
    );
  primary.append(applications);
  content.append(primary);
  const dimensions = card(
    "Operational dimensions",
    "dimensions",
    "Source availability and authoritative summaries, not one project status.",
  );
  dimensions.append(
    el("h3", "small fw-semibold", "Accepted placement"),
    sectionValue(data.accepted_deployments, (page) => {
      const list = el("div");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No accepted D-Maps in the loaded authority projection.",
          ),
        );
      for (const a of page.items.slice(0, PROJECT_LIMITS.accepted))
        list.append(
          el(
            "p",
            "small entity-identity",
            `Application · ${a.application_id} — D-Map · ${a.dmap_id} — revision ${a.revision}`,
          ),
        );
      if (page.has_more || page.items.length > PROJECT_LIMITS.accepted)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Accepted-placement preview is bounded.",
          ),
        );
      return list;
    }),
    el("h3", "small fw-semibold mt-3", "Desired project assignments"),
    sectionValue(data.operational_graph, (graph) =>
      el(
        "p",
        "small entity-identity",
        `Operational D-Graph · ${graph.dgraph_id} · revision ${graph.revision} · ${graph.node_count} structural nodes · ${graph.desired_service_assignment_count} desired service assignments. Desired is not observed or converged.`,
      ),
    ),
    unavailableDimension(
      "Pending proposals",
      "Not included by the project projection; no acceptance controls.",
    ),
    unavailableDimension(
      "Observed runtime",
      "Not included by the project projection.",
    ),
    unavailableDimension(
      "Reconciliation / convergence",
      "Not included by the project projection; no frontend convergence algorithm.",
    ),
  );
  content.append(dimensions);
  const temporal = card(
    "Temporal quality",
    "temporal",
    "Communication, Observation and Source time are independent. Recent receipt of old evidence is not a disconnected-node claim.",
  );
  temporal.append(
    sectionValue(data.temporal_quality, (page) => {
      const list = el("div");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No temporal evidence in this loaded snapshot.",
          ),
        );
      for (const record of page.items.slice(0, PROJECT_LIMITS.temporal)) {
        const entry = el("article", "temporal-entry");
        entry.append(
          el("strong", "small entity-identity", subjectLabel(record.subject)),
        );
        if (record.application_id !== null)
          entry.append(
            el("span", "small", `Application · ${record.application_id}`),
          );
        entry.append(temporalView(record));
        list.append(entry);
      }
      if (page.has_more || page.items.length > PROJECT_LIMITS.temporal)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Temporal preview is bounded; not a complete node inventory.",
          ),
        );
      return list;
    }),
  );
  content.append(temporal);
  const bottom = el("div", "workspace-grid");
  const events = card(
    "Recent project events",
    "events",
    "EVENT · immutable occurrence, newest occurrence first; not an active condition or Historical Explorer.",
  );
  events.append(
    sectionValue(data.recent_events, (page) => {
      const list = el("div");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No recent canonical events in the loaded project window.",
          ),
        );
      for (const event of page.items.slice(0, PROJECT_LIMITS.events)) {
        const row = el("article", "event-row");
        row.append(
          badge(
            event.classification.replaceAll("_", " "),
            event.classification === "recovery" ? "success" : "neutral",
          ),
          el("strong", "small", kindLabel(event.kind)),
          el("span", "small", `Category · ${event.category}`),
          el("span", "small entity-identity", `Project · ${event.project_id}`),
          el("span", "small entity-identity", subjectLabel(event.subject)),
        );
        if (event.application_id !== null)
          row.append(
            el("span", "small", `Application · ${event.application_id}`),
          );
        row.append(stamp(event.occurred_at_utc, "Occurrence"));
        row.title = `Event ${event.event_id} · admitted ${event.admitted_at_utc}`;
        if (event.temporal_context.availability === "invalid")
          row.append(badge("Temporal metadata invalid", "warning"));
        list.append(row);
      }
      if (page.has_more || page.items.length > PROJECT_LIMITS.events)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Recent-event preview is bounded. No history pagination is exposed here.",
          ),
        );
      return list;
    }),
  );
  bottom.append(events);
  const sources = card(
    "Source / projection availability",
    "sources",
    "SOURCE ISSUE · observability/read-model quality, not runtime failure.",
  );
  for (const [label, source] of [
    ["Operational D-Graph / desired assignments", data.operational_graph],
    ["Accepted D-Maps", data.accepted_deployments],
    ["Alert lifecycle", data.alerts],
    ["Event history", data.recent_events],
    ["D-Monitor temporal evidence", data.temporal_quality],
    ["Canonical application registry", data.canonical_application_count],
  ] as const) {
    const row = el("div", "source-row");
    row.append(el("span", "small", label), availability(source));
    sources.append(row);
  }
  for (const issue of issues)
    sources.append(
      el(
        "p",
        "small mt-2 mb-1",
        `${issue.source} · ${issue.code}: ${issue.safe_message}`,
      ),
    );
  bottom.append(sources);
  content.append(
    bottom,
    el(
      "p",
      "snapshot-note",
      `Generated ${data.generated_at_utc} · Manual refresh · independent source snapshots, not globally atomic.`,
    ),
  );
  return content;
}

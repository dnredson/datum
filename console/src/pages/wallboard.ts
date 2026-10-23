import type {
  GlobalOverview,
  OperationalAlert,
  OperationalSubject,
  SourceSection,
} from "../types/operations.ts";
import {
  availability,
  badge,
  coverageView,
  discoveryEmptyMessage,
  el,
  lifecycleBadge,
  sectionValue,
  severityBadge,
  temporalView,
} from "../components/presentation.ts";
import { projectPath, validProjectId } from "../app/router.ts";

export const WALLBOARD_LIMITS = {
  attention: 8,
  projects: 10,
  infrastructure: 6,
  temporal: 10,
  events: 15,
} as const;
// Pure predicate, no truncation -- exported separately (Phase 142A review
// P1-3) so a consumer needing the real count/full list (e.g. DATUM Live's
// own Attention section and per-node alert counts) never inherits this
// Wallboard preview panel's own `.slice(0, 8)` bound by accident.
export function isActiveAttention(alert: OperationalAlert): boolean {
  return (
    alert.is_operationally_active &&
    ["open", "acknowledged", "in_progress"].includes(alert.lifecycle_status)
  );
}
export function activeAttention(
  alerts: OperationalAlert[],
): OperationalAlert[] {
  // Phase 127 supplies attention order; never rank using a second health policy.
  return alerts.filter(isActiveAttention).slice(0, WALLBOARD_LIMITS.attention);
}
export function subjectLabel(subject: OperationalSubject): string {
  const labels: Record<string, string> = {
    dnode: "D-Node",
    dserv: "D-Serv",
    dcall: "D-Call",
    diot: "DIoT",
    runtime_instance: "Runtime instance",
    dmonitor_stream: "D-Monitor stream",
    dmap: "D-Map",
    deployment_proposal: "Deployment proposal",
  };
  const kind =
    subject.kind === "other" ? (subject.entity_kind ?? "other") : subject.kind;
  const identity =
    subject.dmap_id ??
    subject.proposal_id ??
    subject.service_id ??
    subject.node_id ??
    subject.entity_id ??
    subject.call_id ??
    subject.diot_id ??
    subject.artifact_id ??
    subject.application_id ??
    subject.project_id;
  return identity
    ? `${labels[kind] ?? kind.replaceAll("_", " ")} · ${identity}`
    : "DServer global";
}
export function projectPreviewMessages(
  loaded: number,
  hasMore: boolean,
): string[] {
  const messages: string[] = [];
  if (loaded > WALLBOARD_LIMITS.projects)
    messages.push(
      `Showing 10 of ${loaded} loaded references. View Projects for the remaining loaded references.`,
    );
  if (hasMore)
    messages.push(
      "Additional project references exist beyond the loaded discovery page. Console pagination is not yet exposed.",
    );
  return messages;
}
export function kindLabel(kind: string): string {
  const names: Record<string, string> = {
    observation_stale: "Observation stale",
    observation_missing: "Expected observation missing",
    observation_clock_skew: "Source-time skew",
    evaluation_before_receipt: "Evaluation before receipt",
    dmap_accepted: "D-Map accepted",
  };
  return names[kind] ?? kind.replaceAll("_", " ");
}
export function duration(seconds: number | null): string {
  if (seconds === null || seconds < 0) return "Time indeterminate";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}
function time(value: string, label: string): HTMLElement {
  const node = el(
    "time",
    "small text-secondary",
    new Date(value).toLocaleString(),
  );
  node.dateTime = value;
  node.title = `${label}: ${value}`;
  return node;
}
function projectLink(id: string): HTMLElement {
  const link = el("a", "project-link", id);
  if (validProjectId(id)) {
    link.href = projectPath(id);
    link.dataset.route = "";
  } else {
    link.setAttribute("aria-disabled", "true");
    link.title = "Identity cannot be routed by the current scope contract.";
  }
  return link;
}
function panel(title: string, explanation: string, name: string): HTMLElement {
  const box = el("section", `console-card wallboard-${name}`);
  box.append(
    el("h2", "section-title", title),
    el("p", "small text-secondary section-explanation", explanation),
  );
  return box;
}
function metric(
  label: string,
  value: SourceSection<number>,
  note: string,
): HTMLElement {
  const box = el("section", "console-card metric-card");
  box.append(
    el("h2", "section-title", label),
    sectionValue(value, (n) => el("div", "metric-value", n.toLocaleString())),
    el("p", "metric-note", note),
  );
  return box;
}
function counts(
  data: GlobalOverview,
  field: "active" | "resolved",
): SourceSection<number> {
  if (data.alerts.availability !== "available") return data.alerts;
  const c = data.alerts.value.counts;
  return {
    availability: "available",
    value:
      field === "active"
        ? c.open + c.acknowledged + c.in_progress
        : c.resolved_pending_close,
  };
}
export function wallboardView(data: GlobalOverview): HTMLElement {
  const content = el("div", "page-content wallboard");
  const intro = el("div", "page-intro");
  intro.append(
    el("h1", "", "Operational overview"),
    el(
      "p",
      "text-secondary",
      "Attention, evidence and recent occurrences across this DServer. No composite health score.",
    ),
  );
  content.append(intro);
  const summary = el("div", "metric-grid summary-strip");
  const scheduler = data.scheduler;
  const loss: SourceSection<number> =
    scheduler.availability !== "available"
      ? scheduler
      : scheduler.value.dropped_output_full === null ||
          scheduler.value.dropped_output_closed === null
        ? { availability: "not_configured" }
        : {
            availability: "available",
            value:
              scheduler.value.dropped_output_full +
              scheduler.value.dropped_output_closed,
          };
  summary.append(
    metric(
      "Managed projects",
      data.managed_operational_project_count,
      "Operational deployment documents; not all discovered projects.",
    ),
    metric(
      "Known applications loaded",
      {
        availability: "available",
        value: data.known_applications.page.items.length,
      },
      `Discovered references in this bounded page; not canonical inventory. Coverage: ${data.known_applications.coverage.completeness}${data.known_applications.page.has_more ? "; more references outside page" : ""}.`,
    ),
    metric(
      "Active alerts",
      counts(data, "active"),
      "Open / Acknowledged / In progress.",
    ),
    metric(
      "Pending closure",
      counts(data, "resolved"),
      "Resolved conditions, not active attention.",
    ),
    metric(
      "Recent events",
      data.recent_event_count,
      "Canonical occurrences in the last 24 hours.",
    ),
    metric(
      "Source issues reported",
      { availability: "available", value: data.source_issues.length },
      "Read-source / projection issues; not infrastructure failures.",
    ),
    metric(
      "Monitoring output loss",
      loss,
      "Full + closed output counters when configured; not durable delivery proof.",
    ),
  );
  content.append(summary);

  const attention = panel(
    "Attention Now",
    "ALERT · persistent condition lifecycle. Ordered by Phase 127; temporal evidence does not create duplicate attention items.",
    "attention",
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
            "No active alert items in this loaded projection. Resolved conditions remain pending closure.",
          ),
        );
      for (const alert of items) {
        const row = el("article", "attention-item");
        const heading = el("div", "attention-heading");
        heading.append(
          severityBadge(alert.severity),
          badge(alert.category.replaceAll("_", " "), "neutral"),
          el("h3", "attention-condition", kindLabel(alert.kind)),
          lifecycleBadge(alert.lifecycle_status),
        );
        const scope = el("div", "attention-scope");
        scope.append(
          alert.project_id
            ? projectLink(alert.project_id)
            : el("span", "small", "DServer global"),
          el("span", "small", subjectLabel(alert.subject)),
        );
        if (alert.application_id)
          scope.append(
            el(
              "span",
              "small text-secondary",
              `Application · ${alert.application_id}`,
            ),
          );
        if (alert.subject.node_id)
          scope.append(
            el(
              "span",
              "small text-secondary",
              `D-Node · ${alert.subject.node_id}`,
            ),
          );
        const evidence = el("div", "attention-evidence");
        evidence.append(
          el(
            "span",
            "small",
            `${duration(alert.condition_duration_seconds)} condition duration · ${alert.condition_confirmation_count} confirmations`,
          ),
          time(alert.first_observed_at_utc, "Condition first observed"),
          el(
            "span",
            "small text-secondary",
            `Latest evidence · ${alert.latest_condition_event.event_id}`,
          ),
        );
        evidence.append(
          time(
            alert.latest_condition_event.occurred_at_utc,
            "Latest condition occurrence",
          ),
        );
        const temporal = alert.latest_condition_event.temporal_context;
        if (
          temporal.availability === "available" &&
          temporal.value.evidence.length
        ) {
          const record = temporal.value.evidence[0]!;
          evidence.append(
            el(
              "span",
              "small text-secondary",
              `Receipt age at evaluation: ${duration(record.receipt_age_seconds)} · source age: ${duration(record.source_age_seconds)} · observed−received: ${record.observed_receipt_delta_seconds}s`,
            ),
          );
        } else if (temporal.availability === "invalid")
          evidence.append(badge("Temporal metadata invalid", "warning"));
        row.append(heading, scope, evidence);
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
            "small text-secondary mb-0",
            "Attention preview is bounded; more items may require review.",
          ),
        );
      return list;
    }),
  );
  content.append(attention);

  const grouping = el("div", "workspace-grid wallboard-group");
  const projects = panel(
    "Known projects",
    "Discovered references, not a canonical registry. Placement, runtime, monitoring and alert lifecycle are not one status.",
    "projects",
  );
  projects.append(coverageView(data.known_projects.coverage));
  if (!data.known_projects.page.items.length)
    projects.append(
      el(
        "p",
        "empty-text",
        discoveryEmptyMessage(data.known_projects.coverage),
      ),
    );
  for (const id of data.known_projects.page.items.slice(
    0,
    WALLBOARD_LIMITS.projects,
  )) {
    const row = el("article", "wallboard-project");
    const apps = data.known_applications.page.items.filter(
      (a) => a.project_id === id,
    ).length;
    row.append(
      projectLink(id),
      el(
        "p",
        "small text-secondary mb-2",
        `${apps} application references loaded · ${data.known_applications.coverage.completeness === "complete" && !data.known_applications.page.has_more ? "inspected sources fully scanned" : "discovery incomplete"}`,
      ),
    );
    if (data.projects_with_attention.availability === "available") {
      const count = data.projects_with_attention.value.items.find(
        (a) => a.project_id === id,
      );
      row.append(
        count
          ? badge(
              `${count.active_alert_count} active alerts`,
              count.critical_count ? "danger" : "warning",
            )
          : badge(
              data.projects_with_attention.value.has_more
                ? "Alert summary outside loaded page"
                : "No active alert attention",
              "neutral",
            ),
      );
    } else row.append(availability(data.projects_with_attention));
    if (data.temporal_quality.availability === "available") {
      const streams = data.temporal_quality.value.items.filter(
        (t) => t.project_id === id,
      );
      row.append(
        el(
          "p",
          "small mt-2 mb-1",
          `Monitoring: ${streams.length} evidence streams loaded${streams.length ? ` · ${streams.filter((t) => t.communication_freshness === "stale").length} stale receipts` : "; no freshness inference"}`,
        ),
      );
    } else
      row.append(
        el(
          "p",
          "small mt-2 mb-1",
          "Monitoring source cannot provide this summary.",
        ),
      );
    row.append(
      el(
        "p",
        "small text-secondary mb-0",
        "Placement / runtime: not included in this global summary. Open project context for available details.",
      ),
    );
    projects.append(row);
  }
  for (const message of projectPreviewMessages(
    data.known_projects.page.items.length,
    data.known_projects.page.has_more,
  ))
    projects.append(
      el("p", "small text-secondary project-continuation", message),
    );
  grouping.append(projects);

  const infrastructure = panel(
    "Infrastructure / continuum",
    "Structural registry and desired assignments are independent of observed evidence. No online or convergence inference.",
    "infrastructure",
  );
  for (const [label, value] of [
    ["Structural D-Nodes", data.structural_node_count],
    ["Desired service assignments", data.desired_service_assignment_count],
  ] as const) {
    const row = el("div", "source-row");
    row.append(
      el("span", "small", label),
      sectionValue(value, (n) => el("strong", "", n.toLocaleString())),
    );
    infrastructure.append(row);
  }
  infrastructure.append(
    sectionValue(data.temporal_quality, (page) => {
      const list = el("div");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No observed stream references loaded; this is not a node-down conclusion.",
          ),
        );
      for (const record of page.items.slice(
        0,
        WALLBOARD_LIMITS.infrastructure,
      )) {
        const row = el("div", "source-row");
        row.append(
          el("strong", "small entity-identity", subjectLabel(record.subject)),
          projectLink(record.project_id),
          el(
            "span",
            "small text-secondary",
            `Receipt age: ${duration(record.received_age_seconds)} · observed stream, not inventory`,
          ),
        );
        row.append(
          el("span", "small text-secondary", `D-Node · ${record.node_id}`),
        );
        if (record.application_id !== null)
          row.append(
            el(
              "span",
              "small text-secondary",
              `Application · ${record.application_id}`,
            ),
          );
        list.append(row);
      }
      if (page.has_more || page.items.length > WALLBOARD_LIMITS.infrastructure)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Observed-reference preview is bounded; multiple streams may refer to one node.",
          ),
        );
      return list;
    }),
  );
  grouping.append(infrastructure);
  content.append(grouping);

  const temporal = panel(
    "Temporal Quality",
    "Fresh communication + stale observation means recent receipt of old source evidence, not a disconnected node. Behind / delayed may reflect buffering, transport or source time.",
    "temporal",
  );
  temporal.append(
    sectionValue(data.temporal_quality, (page) => {
      const list = el("div", "temporal-list");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No temporal evidence loaded. Missing evidence is not Down.",
          ),
        );
      for (const record of page.items.slice(0, WALLBOARD_LIMITS.temporal)) {
        const entry = el("div", "temporal-entry");
        const identity = el(
          "strong",
          "small entity-identity",
          subjectLabel(record.subject),
        );
        const context = el("div", "identity-context");
        context.append(
          projectLink(record.project_id),
          el("span", "small text-secondary", `D-Node · ${record.node_id}`),
        );
        if (record.application_id !== null)
          context.append(
            el(
              "span",
              "small text-secondary",
              `Application · ${record.application_id}`,
            ),
          );
        const dimensions = temporalView(record);
        dimensions.querySelector(".node-label")?.remove();
        entry.append(identity, context, dimensions);
        list.append(entry);
      }
      if (page.has_more || page.items.length > WALLBOARD_LIMITS.temporal)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Temporal stream preview is bounded; not a complete node inventory.",
          ),
        );
      return list;
    }),
  );
  content.append(temporal);

  const bottom = el("div", "workspace-grid wallboard-group");
  const events = panel(
    "Recent Operational Events",
    "EVENT · immutable occurrence, not an active condition. Newest occurrence first; at most 15 records from the last 24 hours.",
    "events",
  );
  events.append(
    sectionValue(data.recent_events, (page) => {
      const list = el("div", "event-list");
      if (!page.items.length)
        list.append(
          el(
            "p",
            "empty-text",
            "No recent canonical events in this loaded window.",
          ),
        );
      for (const event of page.items.slice(0, WALLBOARD_LIMITS.events)) {
        const row = el("article", "event-row");
        const title = el("div", "event-title");
        title.append(
          badge(
            event.classification.replaceAll("_", " "),
            event.classification === "recovery" ? "success" : "neutral",
          ),
          el("strong", "small", kindLabel(event.kind)),
        );
        row.append(
          title,
          event.project_id
            ? projectLink(event.project_id)
            : el("span", "small", "DServer global"),
          el("span", "small text-secondary", subjectLabel(event.subject)),
          time(event.occurred_at_utc, "Occurrence time"),
        );
        if (event.application_id !== null)
          row.append(
            el(
              "span",
              "small text-secondary event-application",
              `Application · ${event.application_id}`,
            ),
          );
        row.title = `Event ${event.event_id} · ${event.source} · admitted ${event.admitted_at_utc}`;
        if (event.temporal_context.availability === "invalid")
          row.append(badge("Temporal metadata invalid", "warning"));
        list.append(row);
      }
      if (page.has_more || page.items.length > WALLBOARD_LIMITS.events)
        list.append(
          el(
            "p",
            "small text-secondary",
            "Only the newest preview is shown; this is not the Historical Explorer.",
          ),
        );
      return list;
    }),
  );
  bottom.append(events);
  const sources = panel(
    "Observability / projection",
    "SOURCE ISSUE · difficulty reading or interpreting a source, not proof of infrastructure failure.",
    "sources",
  );
  for (const [label, source] of [
    ["Events", data.recent_events],
    ["Alerts", data.alerts],
    ["D-Monitor", data.temporal_quality],
    ["Scheduler projection", data.scheduler],
  ] as const) {
    const row = el("div", "source-row");
    row.append(el("span", "small", label), availability(source));
    sources.append(row);
  }
  sources.append(
    sectionValue(data.scheduler, (status) => {
      const box = el("div", "scheduler-summary");
      const driver: Record<string, string> = {
        unwired: "Not wired",
        stopped: "Stopped",
        stopping: "Stopping",
        indeterminate: "Activity unknown",
      };
      box.append(
        badge(
          status.configured
            ? "Scheduler configured"
            : "Scheduler not configured",
          "neutral",
        ),
        el(
          "p",
          "small mt-2 mb-1",
          `Driver: ${driver[status.driver_activity] ?? "Activity unknown"} · owner lifecycle: ${status.lifecycle ?? "not configured"}`,
        ),
        el(
          "p",
          "small text-secondary mb-0",
          `Output full: ${status.dropped_output_full ?? "not available"} · output closed: ${status.dropped_output_closed ?? "not available"}. Owner lifecycle does not prove driver activity.`,
        ),
      );
      if (
        (status.dropped_output_full ?? 0) +
          (status.dropped_output_closed ?? 0) >
        0
      )
        box.append(badge("Monitoring output loss reported", "warning"));
      return box;
    }),
  );
  if (data.source_issues.length) {
    const notices = el("details", "source-notices");
    notices.append(
      el("summary", "", `Projection notices · ${data.source_issues.length}`),
    );
    for (const issue of data.source_issues)
      notices.append(
        el(
          "p",
          "small mt-2 mb-1",
          `${issue.source} · ${issue.code}: ${issue.safe_message}`,
        ),
      );
    sources.append(notices);
  }
  bottom.append(sources);
  content.append(bottom);
  content.append(
    el(
      "p",
      "snapshot-note",
      `Generated ${data.generated_at_utc} · Manual refresh · independent source snapshots, not globally atomic.`,
    ),
  );
  return content;
}

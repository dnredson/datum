import type {
  Coverage,
  SourceSection,
  TemporalQuality,
} from "../types/operations.ts";

export type Tone = "neutral" | "info" | "success" | "warning" | "danger";
export const availabilityLabels = {
  available: "Available",
  not_configured: "Not configured",
  unavailable: "Unavailable",
  corrupt: "Integrity error",
  indeterminate: "Indeterminate",
} as const;
export const severityBadges: Record<string, [string, Tone]> = {
  info: ["Info", "info"],
  warning: ["Warning", "warning"],
  high: ["High", "danger"],
  critical: ["Critical", "danger"],
};
export const lifecycleBadges: Record<string, [string, Tone]> = {
  open: ["Open", "warning"],
  acknowledged: ["Acknowledged", "info"],
  in_progress: ["In progress", "info"],
  resolved: ["Resolved · pending closure", "success"],
  closed: ["Closed · history", "neutral"],
};
export function discoveryEmptyMessage(coverage: Coverage): string {
  return coverage.completeness === "complete"
    ? "No project references were discovered in the scanned sources."
    : "No projects are currently discoverable from the available sources.";
}
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
export function badge(label: string, tone: Tone = "neutral"): HTMLElement {
  return el("span", `status-badge tone-${tone}`, label);
}
export function severityBadge(value: string): HTMLElement {
  const [label, tone] = severityBadges[value] ?? [
    "Unknown severity",
    "neutral",
  ];
  return badge(label, tone);
}
export function lifecycleBadge(value: string): HTMLElement {
  const [label, tone] = lifecycleBadges[value] ?? [
    "Unknown lifecycle",
    "neutral",
  ];
  return badge(label, tone);
}
export function availability(section: SourceSection<unknown>): HTMLElement {
  const tone: Tone =
    section.availability === "corrupt"
      ? "danger"
      : section.availability === "unavailable"
        ? "warning"
        : section.availability === "available"
          ? "success"
          : "neutral";
  return badge(availabilityLabels[section.availability], tone);
}
export function sectionValue<T>(
  section: SourceSection<T>,
  render: (value: T) => HTMLElement,
): HTMLElement {
  if (section.availability === "available") return render(section.value);
  const box = el("div", "source-state");
  box.append(availability(section));
  box.append(
    el(
      "p",
      "small mb-0 mt-2 text-secondary",
      section.availability === "not_configured"
        ? "This source is not configured. No zero or health claim is inferred."
        : "This section cannot currently provide a reliable projection. Other sections remain independent.",
    ),
  );
  return box;
}
export function coverageView(coverage: Coverage): HTMLElement {
  const box = el("div", "coverage");
  box.append(
    badge(
      coverage.completeness === "complete"
        ? "Inspected sources fully scanned"
        : coverage.completeness === "partial"
          ? "Discovery is partial"
          : "Discovery coverage is uncertain",
      coverage.completeness === "complete" ? "info" : "warning",
    ),
  );
  box.append(
    el(
      "p",
      "small text-secondary mt-2 mb-2",
      "Known references from inspected sources, not an authoritative inventory. Unreadable or unconfigured sources may contain additional references.",
    ),
  );
  const details = el("details", "coverage-details");
  details.append(el("summary", "small", "View source coverage"));
  const sources = el("div", "coverage-sources");
  const names: Record<string, string> = {
    events: "Event history",
    alerts: "Alert history",
    ddeploy: "Deployment state",
    dmonitor: "Observation evidence",
    operational_graph: "Operational deployment documents",
  };
  const states: Record<string, string> = {
    fully_scanned: "Scanned",
    truncated: "Scan limit reached",
    unavailable: "Unavailable",
    corrupt: "Integrity error",
    not_configured: "Not configured",
    not_applicable: "Not applicable",
    indeterminate: "Uncertain",
  };
  for (const [source, value] of Object.entries(coverage.source_coverage).sort(
    ([a], [b]) => a.localeCompare(b),
  )) {
    const row = el("div", "source-row");
    row.append(
      el("span", "small", names[source] ?? source.replaceAll("_", " ")),
      badge(
        states[value.state] ?? "Unknown coverage",
        value.state === "corrupt"
          ? "danger"
          : value.state === "unavailable" || value.state === "truncated"
            ? "warning"
            : "neutral",
      ),
    );
    sources.append(row);
  }
  details.append(sources);
  box.append(details);
  return box;
}
const temporalLabels: Record<string, string> = {
  fresh: "Fresh",
  stale: "Stale",
  missing_evidence: "Missing evidence",
  indeterminate: "Indeterminate",
  aligned_or_no_discrepancy: "Aligned / no discrepancy",
  source_ahead: "Ahead / skew",
  behind_or_delayed: "Behind / delayed",
  evaluation_ordering_anomaly: "Ordering anomaly",
};
export function temporalPresentation(
  value: TemporalQuality,
): [string, string][] {
  return [
    [
      "Communication",
      temporalLabels[value.communication_freshness] ?? "Unknown",
    ],
    ["Observation", temporalLabels[value.observation_freshness] ?? "Unknown"],
    ["Source time", temporalLabels[value.source_time_quality] ?? "Unknown"],
  ];
}
export function temporalView(value: TemporalQuality): HTMLElement {
  const row = el("div", "temporal-row");
  row.append(el("strong", "small node-label", value.node_id));
  for (const [label, status] of temporalPresentation(value)) {
    const dimension = el("div");
    dimension.append(
      el("span", "dimension-label", label),
      badge(
        status,
        status === "Fresh"
          ? "success"
          : [
                "Stale",
                "Ahead / skew",
                "Behind / delayed",
                "Ordering anomaly",
              ].includes(status)
            ? "warning"
            : "neutral",
      ),
    );
    row.append(dimension);
  }
  return row;
}

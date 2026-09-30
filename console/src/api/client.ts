import type { GlobalOverview, ProjectOverview } from "../types/operations.ts";
import { validProjectId } from "../app/router.ts";

export class ApiError extends Error {
  readonly code: string;
  readonly status: number | null;
  constructor(code: string, message: string, status: number | null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}
const states = new Set([
  "available",
  "not_configured",
  "unavailable",
  "corrupt",
  "indeterminate",
]);
type ObjectValue = Record<string, unknown>;
function object(value: unknown): value is ObjectValue {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function count(value: unknown): boolean {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function safeIssue(value: unknown): boolean {
  return (
    object(value) &&
    typeof value.source === "string" &&
    typeof value.code === "string" &&
    typeof value.safe_message === "string" &&
    new TextEncoder().encode(value.safe_message).length <= 512
  );
}
function section(value: unknown): boolean {
  if (
    !value ||
    typeof value !== "object" ||
    !("availability" in value) ||
    !states.has(String(value.availability))
  )
    return false;
  return (
    value.availability === "not_configured" ||
    ("value" in value &&
      (value.availability === "unavailable" || value.availability === "corrupt"
        ? safeIssue(value.value)
        : value.availability === "indeterminate"
          ? typeof value.value === "string"
          : true))
  );
}
function page(value: unknown, item: (v: unknown) => boolean): boolean {
  return (
    object(value) &&
    Array.isArray(value.items) &&
    value.items.length <= 1000 &&
    value.items.every(item) &&
    typeof value.has_more === "boolean" &&
    count(value.limit) &&
    Number(value.limit) >= 1 &&
    Number(value.limit) <= 1000
  );
}
function application(value: unknown): boolean {
  return (
    object(value) &&
    typeof value.project_id === "string" &&
    typeof value.application_id === "string" &&
    Array.isArray(value.provenance) &&
    value.provenance.every((v) => typeof v === "string")
  );
}
function payload(
  v: ObjectValue,
  key: string,
  predicate: (value: unknown) => boolean,
): boolean {
  const s = v[key];
  return object(s) && (s.availability !== "available" || predicate(s.value));
}
function discovery(value: unknown): boolean {
  if (
    !value ||
    typeof value !== "object" ||
    !("page" in value) ||
    !("coverage" in value)
  )
    return false;
  const page = value.page as {
    items?: unknown;
    has_more?: unknown;
    limit?: unknown;
  } | null;
  const coverage = value.coverage as {
    completeness?: unknown;
    source_coverage?: unknown;
  } | null;
  return (
    !!page &&
    Array.isArray(page.items) &&
    typeof page.has_more === "boolean" &&
    typeof page.limit === "number" &&
    !!coverage &&
    ["complete", "partial", "indeterminate"].includes(
      String(coverage.completeness),
    ) &&
    !!coverage.source_coverage
  );
}
function validate(value: unknown, project: boolean): void {
  const v = value as Record<string, unknown> | null;
  const sections = project
    ? [
        "operational_graph",
        "canonical_application_count",
        "alerts",
        "recent_events",
        "accepted_deployments",
        "temporal_quality",
      ]
    : [
        "managed_operational_project_count",
        "canonical_application_count",
        "structural_node_count",
        "desired_service_assignment_count",
        "alerts",
        "recent_event_count",
        "projects_with_attention",
        "temporal_quality",
        "scheduler",
      ];
  if (
    !v ||
    typeof v.generated_at_utc !== "string" ||
    !discovery(v.known_applications) ||
    sections.some((key) => !section(v[key])) ||
    (project
      ? typeof v.project_id !== "string"
      : !discovery(v.known_projects) || !Array.isArray(v.source_issues))
  ) {
    throw new ApiError(
      "invalid_response",
      "DServer returned an unsupported projection contract.",
      null,
    );
  }
  const numberSections = project
    ? ["canonical_application_count"]
    : [
        "managed_operational_project_count",
        "canonical_application_count",
        "structural_node_count",
        "desired_service_assignment_count",
        "recent_event_count",
      ];
  const knownApps = v.known_applications as ObjectValue;
  const valid =
    numberSections.every((key) => payload(v, key, count)) &&
    page(knownApps.page, application) &&
    payload(
      v,
      "alerts",
      (value) =>
        object(value) &&
        object(value.counts) &&
        [
          "open",
          "acknowledged",
          "in_progress",
          "resolved_pending_close",
          "historical_closed",
          "critical_attention",
          "warning_attention",
        ].every((key) => count((value.counts as ObjectValue)[key])),
    ) &&
    payload(v, "temporal_quality", (value) =>
      page(
        value,
        (item) =>
          object(item) &&
          typeof item.node_id === "string" &&
          ["fresh", "stale", "missing_evidence", "indeterminate"].includes(
            String(item.communication_freshness),
          ) &&
          ["fresh", "stale", "missing_evidence", "indeterminate"].includes(
            String(item.observation_freshness),
          ) &&
          [
            "aligned_or_no_discrepancy",
            "source_ahead",
            "behind_or_delayed",
            "evaluation_ordering_anomaly",
            "missing_evidence",
          ].includes(String(item.source_time_quality)),
      ),
    ) &&
    (project
      ? payload(v, "recent_events", (value) =>
          page(
            value,
            (item) => object(item) && typeof item.event_id === "string",
          ),
        )
      : page(
          (v.known_projects as ObjectValue).page,
          (item) => typeof item === "string",
        ) &&
        (v.source_issues as unknown[]).every(safeIssue) &&
        payload(v, "projects_with_attention", (value) =>
          page(
            value,
            (item) =>
              object(item) &&
              typeof item.project_id === "string" &&
              count(item.active_alert_count) &&
              count(item.critical_count) &&
              count(item.warning_count),
          ),
        ));
  if (!valid)
    throw new ApiError(
      "invalid_response",
      "DServer returned invalid projection values.",
      null,
    );
}
export class OperationsClient {
  private readonly baseUrl: string;
  private readonly timeoutMs: number;
  constructor(baseUrl = "", timeoutMs = 15000) {
    this.baseUrl = baseUrl;
    this.timeoutMs = timeoutMs;
  }
  private async request<T>(
    path: string,
    signal: AbortSignal,
    projectId?: string,
  ): Promise<T> {
    const controller = new AbortController();
    const abort = () => controller.abort(signal.reason);
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, this.timeoutMs);
    try {
      const headers: Record<string, string> = { Accept: "application/json" };
      if (projectId !== undefined) headers["X-Datum-Project"] = projectId;
      const response = await fetch(`${this.baseUrl}${path}`, {
        method: "GET",
        headers,
        signal: controller.signal,
        cache: "no-store",
      });
      let body: unknown;
      try {
        body = await response.json();
      } catch {
        throw new ApiError(
          "invalid_response",
          "DServer did not return JSON.",
          response.status,
        );
      }
      if (!response.ok) {
        const error = (
          body as { error?: { code?: unknown; message?: unknown } } | null
        )?.error;
        // Messages only from the documented bounded envelope, never raw HTML/diagnostics.
        const message =
          typeof error?.message === "string" &&
          new TextEncoder().encode(error.message).length <= 512
            ? error.message
            : "The read request failed.";
        throw new ApiError(
          typeof error?.code === "string" && /^[a-z_]{1,64}$/u.test(error.code)
            ? error.code
            : "request_failed",
          message,
          response.status,
        );
      }
      validate(body, projectId !== undefined);
      if (
        projectId !== undefined &&
        (body as ProjectOverview).project_id !== projectId
      )
        throw new ApiError(
          "scope_mismatch",
          "DServer returned a different project scope.",
          response.status,
        );
      return body as T;
    } catch (error) {
      if (signal.aborted) throw error;
      if (timedOut)
        throw new ApiError(
          "timeout",
          "DServer did not respond before the read timeout.",
          null,
        );
      if (error instanceof ApiError) throw error;
      throw new ApiError(
        "disconnected",
        "The Console could not connect to DServer.",
        null,
      );
    } finally {
      clearTimeout(timer);
      signal.removeEventListener("abort", abort);
    }
  }
  overview(signal: AbortSignal): Promise<GlobalOverview> {
    return this.request("/api/v1/operations/overview", signal);
  }
  project(id: string, signal: AbortSignal): Promise<ProjectOverview> {
    if (!validProjectId(id))
      return Promise.reject(
        new ApiError("invalid_scope", "Invalid project identifier.", null),
      );
    return this.request(
      `/api/v1/operations/projects/${encodeURIComponent(id)}`,
      signal,
      id,
    );
  }
}

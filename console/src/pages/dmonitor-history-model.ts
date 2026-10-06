// Phase 140A -- pure model for the DMonitor historical read: response
// validators and enum -> display mappings ONLY. No DOM, no network, no
// re-derivation of backend logic, exactly mirroring
// `dmonitor-convergence-model.ts`'s own established discipline. This
// module never computes or renders a convergence/authority verdict for a
// historical row -- the backend never provides one, and this file must
// never invent one client-side (the Phase 140A invariant: a view over
// persisted evidence, not a second monitoring model).
import type {
  DMonitorHistoryQueryResponse,
  HistoricalDMonitorObservation,
  DMonitorSubject,
  DMonitorFreshness,
  DMonitorHealth,
} from "../types/dmonitor-history.ts";

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string";
}
function optionalText(value: unknown): boolean {
  return value === undefined || typeof value === "string";
}
function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function bool(value: unknown): value is boolean {
  return typeof value === "boolean";
}
function nullableText(value: unknown): boolean {
  return value === null || typeof value === "string";
}
function timestamp(value: unknown): boolean {
  return (
    typeof value === "string" &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)$/u.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

const FRESHNESS = new Set(["fresh", "stale", "clock_skew"]);
const HEALTH = new Set(["unknown", "healthy", "degraded", "unhealthy"]);

function dmapRef(value: unknown): boolean {
  return value === undefined || (object(value) && text(value.id) && count(value.revision) && text(value.digest));
}
function runtimeBindingRef(value: unknown): boolean {
  return value === undefined || (object(value) && text(value.binding_id) && count(value.binding_revision) && text(value.descriptor_digest));
}

// Closed-set validator mirroring `DMonitorSubjectV1`'s 8 tagged variants
// exactly -- a `kind` this validator does not recognize is REJECTED, never
// permissively accepted (Part AW: no fictional states).
function subject(value: unknown): value is DMonitorSubject {
  if (!object(value) || !text(value.kind)) return false;
  switch (value.kind) {
    case "dnode":
      return text(value.node_id);
    case "runtime_process":
      return text(value.node_id) && text(value.runtime_ref);
    case "dserv_realization":
      return text(value.project_id) && text(value.application_id) && text(value.service_id) && text(value.node_id) && dmapRef(value.dmap_ref);
    case "dcode_execution":
      return text(value.project_id) && text(value.application_id) && text(value.service_id) && text(value.node_id) && optionalText(value.dcode_evidence_sha256);
    case "dcall_delivery":
      return text(value.project_id) && text(value.application_id) && text(value.call_id) && optionalText(value.grant_id) && optionalText(value.delivery_evidence_digest);
    case "diot_realization":
      return (
        text(value.project_id) &&
        text(value.application_id) &&
        text(value.diot_id) &&
        text(value.diot_placement_id) &&
        text(value.node_id) &&
        dmapRef(value.dmap_ref) &&
        dmapRef(value.dforward_ref) &&
        runtimeBindingRef(value.runtime_binding_ref)
      );
    case "network_reachability":
      return text(value.node_id) && text(value.target);
    case "unbound":
      return text(value.node_id) && optionalText(value.description);
    default:
      return false;
  }
}

function historicalObservation(value: unknown): value is HistoricalDMonitorObservation {
  return (
    object(value) &&
    text(value.observation_id) &&
    text(value.node_id) &&
    text(value.monitor_instance_id) &&
    count(value.sequence) &&
    text(value.collector) &&
    subject(value.subject) &&
    HEALTH.has(String(value.health)) &&
    timestamp(value.observed_at_utc) &&
    timestamp(value.received_at_utc) &&
    FRESHNESS.has(String(value.freshness)) &&
    "facts" in value
  );
}

function filtersEcho(value: unknown): boolean {
  return object(value) && nullableText(value.application_id) && nullableText(value.node_id) && nullableText(value.service_id);
}

export function validDMonitorHistoryQueryResponse(value: unknown, projectId?: string): value is DMonitorHistoryQueryResponse {
  return (
    object(value) &&
    text(value.schema) &&
    text(value.project_id) &&
    (projectId === undefined || value.project_id === projectId) &&
    timestamp(value.start_utc) &&
    timestamp(value.end_utc) &&
    timestamp(value.queried_at_utc) &&
    filtersEcho(value.filters) &&
    count(value.limit) &&
    count(value.count) &&
    bool(value.truncated) &&
    Array.isArray(value.observations) &&
    value.observations.every(historicalObservation)
  );
}

// Phase 140B -- mirrors `dserver/src/core/dmonitor_history.rs`'s own
// `MAX_HISTORY_WINDOW_SECONDS`/`DEFAULT_HISTORY_WINDOW_SECONDS` exactly, for
// a PROACTIVE client-side check only (Part B): this never silently narrows
// what is actually sent -- it only lets the UI show a clear validation
// message before even making the round trip. The backend's own 422
// rejection remains the authoritative bound in every case.
export const MAX_HISTORY_WINDOW_SECONDS = 7 * 86_400;
export const DEFAULT_HISTORY_WINDOW_SECONDS = 86_400;

export interface HistoryFilters {
  startUtc: string;
  endUtc: string;
  applicationId: string;
  nodeId: string;
  serviceId: string;
  limit: string;
}

export function defaultHistoryFilters(now: Date = new Date()): HistoryFilters {
  const start = new Date(now.getTime() - DEFAULT_HISTORY_WINDOW_SECONDS * 1000);
  return {
    startUtc: start.toISOString(),
    endUtc: now.toISOString(),
    applicationId: "",
    nodeId: "",
    serviceId: "",
    limit: "",
  };
}

// Part B -- a proactive, non-authoritative check only: never silently
// narrows the request, only blocks submission with a clear message so an
// obviously-oversized/invalid request never even round-trips. `undefined`
// means "no obvious problem" -- the backend's own validation still runs.
export function clientSideHistoryValidationError(filters: HistoryFilters): string | undefined {
  const start = Date.parse(filters.startUtc);
  const end = Date.parse(filters.endUtc);
  if (filters.startUtc && Number.isNaN(start)) return `start time '${filters.startUtc}' is not a valid RFC 3339 timestamp.`;
  if (filters.endUtc && Number.isNaN(end)) return `end time '${filters.endUtc}' is not a valid RFC 3339 timestamp.`;
  if (!Number.isNaN(start) && !Number.isNaN(end)) {
    if (start > end) return "Start time must not be after end time.";
    if ((end - start) / 1000 > MAX_HISTORY_WINDOW_SECONDS) {
      return `The requested window exceeds the maximum of ${MAX_HISTORY_WINDOW_SECONDS / 86_400} days. Narrow the start/end time instead of submitting -- the range is never silently shortened for you.`;
    }
  }
  if (filters.limit) {
    const limitValue = Number(filters.limit);
    if (!Number.isInteger(limitValue) || limitValue < 1) return "Result limit must be a positive whole number.";
  }
  return undefined;
}

export function historyObservationIdentityLine(subject: DMonitorSubject): string {
  const { applicationId, serviceId } = subjectDisplayIdentity(subject);
  const parts = [subjectKindLabel(subject)];
  if (applicationId) parts.push(`app: ${applicationId}`);
  if (serviceId) parts.push(`service: ${serviceId}`);
  return parts.join(" · ");
}

// ---- Display mappings (enum -> label/tone only; never re-derives state) ----

// Part BA -- labeled as RUNTIME OBSERVATION health, never business/
// application health, same discipline as the convergence model's own labels.
export const HEALTH_LABEL: Record<DMonitorHealth, string> = {
  unknown: "Unknown",
  healthy: "Healthy",
  degraded: "Degraded",
  unhealthy: "Unhealthy",
};
export const HEALTH_TONE: Record<DMonitorHealth, "neutral" | "info" | "success" | "warning" | "danger"> = {
  unknown: "neutral",
  healthy: "success",
  degraded: "warning",
  unhealthy: "danger",
};

export const FRESHNESS_LABEL: Record<DMonitorFreshness, string> = {
  fresh: "Fresh",
  stale: "Stale",
  clock_skew: "Clock skew",
};
export const FRESHNESS_TONE: Record<DMonitorFreshness, "neutral" | "info" | "success" | "warning" | "danger"> = {
  fresh: "success",
  stale: "warning",
  clock_skew: "warning",
};

// Human-readable (application_id, service-like id) extraction for display
// only -- never used to recompute identity/filtering, which stays entirely
// server-side. Mirrors `core::dmonitor_history::subject_logical_identity`'s
// own per-variant extraction, read-only.
export function subjectDisplayIdentity(value: DMonitorSubject): { applicationId: string | null; serviceId: string | null } {
  switch (value.kind) {
    case "dserv_realization":
    case "dcode_execution":
      return { applicationId: value.application_id, serviceId: value.service_id };
    case "dcall_delivery":
      return { applicationId: value.application_id, serviceId: value.call_id };
    case "diot_realization":
      return { applicationId: value.application_id, serviceId: value.diot_placement_id };
    default:
      return { applicationId: null, serviceId: null };
  }
}

export function subjectKindLabel(value: DMonitorSubject): string {
  switch (value.kind) {
    case "dnode":
      return "D-Node";
    case "runtime_process":
      return "Runtime process";
    case "dserv_realization":
      return "D-Serv realization";
    case "dcode_execution":
      return "D-Code execution";
    case "dcall_delivery":
      return "D-Call delivery";
    case "diot_realization":
      return "DIoT realization";
    case "network_reachability":
      return "Network reachability";
    case "unbound":
      return "Unbound";
  }
}

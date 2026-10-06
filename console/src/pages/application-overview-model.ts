// Phase 141B -- pure model for the application operational context:
// response validation and enum -> display mappings ONLY. No DOM, no
// network, no authority logic, no re-derivation of convergence/readiness
// (both are trusted verbatim from the backend, exactly like every other
// DMonitor-family model in this codebase already does).
import type {
  ApplicationOperationsOverview,
  AcceptedPlacementView,
  PlannedPlacementView,
  DesiredRuntimeView,
  ObservedRuntimeView,
  ApplicationConvergenceView,
  ApplicationReadinessView,
  RecentHistoryPointerView,
  DMonitorReadiness,
} from "../types/application-overview.ts";
import type { SourceSection, Page, AlertSection, OperationalEvent, TemporalQuality } from "../types/operations.ts";
import { validDMonitorSubject } from "./dmonitor-history-model.ts";

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string";
}
function nullableText(value: unknown): boolean {
  return value === null || typeof value === "string";
}
function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function nullableCount(value: unknown): boolean {
  return value === null || count(value);
}
function bool(value: unknown): value is boolean {
  return typeof value === "boolean";
}
// Phase 140A finding, reused here: real `_utc` fields in this response
// originate from TWO different generators -- `operations_read_models.rs`'s
// own `canonical_at()` (always forces a literal `Z`) AND the much older
// raw `Utc::now().to_rfc3339()` DMonitor ingestion path (`received_at_utc`
// on `recent_history`/`observed_runtime`, always `+00:00`). Both valid RFC
// 3339 spellings must validate.
function timestamp(value: unknown): boolean {
  return (
    typeof value === "string" &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)$/u.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}
function nullableTimestamp(value: unknown): boolean {
  return value === null || timestamp(value);
}

const CONVERGENCE = new Set(["converged", "diverged", "unknown"]);
const READINESS = new Set(["ready", "not_ready"]);

function finding(value: unknown): boolean {
  return object(value) && text(value.code) && text(value.message);
}
function page<T>(value: unknown, item: (v: unknown) => boolean): value is Page<T> {
  return (
    object(value) &&
    Array.isArray(value.items) &&
    value.items.every(item) &&
    count(value.limit) &&
    bool(value.has_more)
  );
}

// Generic `SourceSection<T>` validator -- mirrors the shape
// `types/operations.ts` already declares (`{availability:"available",
// value:T} | {availability:"not_configured"} | {availability:"unavailable"
// |"corrupt", value: SourceIssue} | {availability:"indeterminate",
// value:string}`), never a second `available: boolean` mechanism (Part J).
function sourceIssue(value: unknown): boolean {
  return (
    object(value) &&
    text(value.kind) &&
    text(value.source) &&
    text(value.code) &&
    text(value.safe_message) &&
    (value.scope === null || (object(value.scope) && nullableText(value.scope.project_id) && nullableText(value.scope.application_id)))
  );
}
function sourceSection<T>(value: unknown, available: (v: unknown) => v is T): value is SourceSection<T> {
  if (!object(value) || !text(value.availability)) return false;
  switch (value.availability) {
    case "available":
      return available(value.value);
    case "not_configured":
      return true;
    case "unavailable":
    case "corrupt":
      return sourceIssue(value.value);
    case "indeterminate":
      return text(value.value);
    default:
      return false;
  }
}

function accepted(value: unknown): value is AcceptedPlacementView {
  return (
    object(value) &&
    text(value.project_id) &&
    text(value.application_id) &&
    text(value.dmap_id) &&
    count(value.revision) &&
    text(value.digest) &&
    nullableText(value.acceptance_id) &&
    nullableText(value.dgraph_id) &&
    nullableCount(value.dgraph_revision) &&
    nullableText(value.dgraph_digest) &&
    object(value.service_nodes)
  );
}
function planned(value: unknown): value is PlannedPlacementView {
  return (
    object(value) &&
    text(value.proposal_id) &&
    timestamp(value.created_at_utc) &&
    text(value.lifecycle) &&
    text(value.schema) &&
    text(value.dgraph_id) &&
    object(value.service_nodes) &&
    count(value.total_service_placements) &&
    bool(value.placements_truncated)
  );
}
function desiredRuntime(value: unknown): value is DesiredRuntimeView {
  return object(value) && text(value.service_id) && text(value.runtime) && text(value.desired_state);
}
function observedRuntime(value: unknown): value is ObservedRuntimeView {
  return (
    object(value) &&
    text(value.observation_id) &&
    validDMonitorSubject(value.subject) &&
    timestamp(value.observed_at_utc) &&
    timestamp(value.received_at_utc) &&
    text(value.reported_health)
  );
}
function temporalQuality(value: unknown): value is TemporalQuality {
  // Deliberately permissive on this already-established, pre-existing
  // shape (`types/operations.ts`'s own `TemporalQuality`) -- this model
  // never re-validates a type it does not own; a structural `object()`
  // check plus the two identity fields it needs for rendering is enough.
  return object(value) && text(value.project_id) && text(value.node_id);
}
function convergence(value: unknown): value is ApplicationConvergenceView {
  return (
    object(value) &&
    timestamp(value.evaluated_at_utc) &&
    CONVERGENCE.has(String(value.convergence)) &&
    count(value.desired_service_count) &&
    count(value.unexpected_realization_count) &&
    page(value.findings, finding)
  );
}
function readiness(value: unknown): value is ApplicationReadinessView {
  return (
    object(value) &&
    timestamp(value.evaluated_at_utc) &&
    READINESS.has(String(value.readiness)) &&
    page(value.findings, finding)
  );
}
function recentHistory(value: unknown): value is RecentHistoryPointerView {
  return (
    object(value) &&
    count(value.recent_observation_count) &&
    nullableTimestamp(value.latest_received_at_utc) &&
    nullableText(value.latest_observation_id)
  );
}
function alertSection(value: unknown): value is AlertSection {
  return object(value) && object(value.counts);
}
function operationalEvent(value: unknown): value is OperationalEvent {
  return object(value) && text(value.event_id) && timestamp(value.occurred_at_utc) && timestamp(value.admitted_at_utc);
}

export function validApplicationOperationsOverview(
  value: unknown,
  projectId?: string,
  applicationId?: string,
): value is ApplicationOperationsOverview {
  return (
    object(value) &&
    timestamp(value.generated_at_utc) &&
    text(value.project_id) &&
    (projectId === undefined || value.project_id === projectId) &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    sourceSection<string>(value.canonical_application_registry, text) &&
    sourceSection<AcceptedPlacementView | null>(value.accepted, (v): v is AcceptedPlacementView | null => v === null || accepted(v)) &&
    sourceSection<Page<PlannedPlacementView>>(value.planned, (v): v is Page<PlannedPlacementView> => page(v, planned)) &&
    sourceSection<Page<DesiredRuntimeView>>(value.desired_runtime, (v): v is Page<DesiredRuntimeView> => page(v, desiredRuntime)) &&
    sourceSection<Page<ObservedRuntimeView>>(value.observed_runtime, (v): v is Page<ObservedRuntimeView> => page(v, observedRuntime)) &&
    sourceSection<Page<TemporalQuality>>(value.temporal_quality, (v): v is Page<TemporalQuality> => page(v, temporalQuality)) &&
    sourceSection<ApplicationConvergenceView>(value.convergence, convergence) &&
    sourceSection<ApplicationReadinessView>(value.readiness, readiness) &&
    sourceSection<string>(value.reconciliation_execution, text) &&
    sourceSection<AlertSection>(value.alerts, alertSection) &&
    sourceSection<Page<OperationalEvent>>(value.recent_events, (v): v is Page<OperationalEvent> => page(v, operationalEvent)) &&
    sourceSection<RecentHistoryPointerView>(value.recent_history, recentHistory)
  );
}

// ---- Display mappings (enum -> label/tone only; never re-derives state) ----

// Part U -- fail-closed operational summary wording, never business health.
export const READINESS_LABEL: Record<DMonitorReadiness, string> = {
  ready: "Ready",
  not_ready: "Not Ready",
};
export const READINESS_TONE: Record<DMonitorReadiness, "neutral" | "info" | "success" | "warning" | "danger"> = {
  ready: "success",
  not_ready: "danger",
};

// Part AF -- a caller rendering any section should branch on `availability`
// directly; this is a plain, honest label for whichever non-"available"
// state is present, never collapsing them into one "unavailable" word.
export function sectionAvailabilityLabel(availability: string): string {
  switch (availability) {
    case "not_configured":
      return "Not configured";
    case "unavailable":
      return "Unavailable";
    case "corrupt":
      return "Corrupt";
    case "indeterminate":
      return "Indeterminate";
    default:
      return availability;
  }
}

// Phase 139B -- pure model for the DMonitor convergence read: response
// validators and enum -> display mappings ONLY. No DOM, no network, no
// re-derivation of backend convergence logic (Part H/J5) --
// `evaluate_application_dserv_convergence` stays server-side and is
// trusted verbatim here, exactly like `reconciliation-model.ts` already
// does for request/batch/attempt status.
import type {
  EvaluateConvergenceResponse,
  DMonitorApplicationConvergence,
  DMonitorServicePlacementConvergence,
  DMonitorEvidenceSummary,
  DMonitorConvergence,
  DMonitorFreshness,
  DMonitorHealth,
} from "../types/dmonitor-convergence.ts";

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
function timestamp(value: unknown): boolean {
  return (
    typeof value === "string" &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?Z$/u.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

const FRESHNESS = new Set(["fresh", "stale", "clock_skew"]);
const HEALTH = new Set(["unknown", "healthy", "degraded", "unhealthy"]);
const CORRELATION = new Set(["exact_match", "ref_missing", "mismatch", "not_applicable"]);
const CONVERGENCE = new Set(["converged", "diverged", "unknown"]);

function finding(value: unknown): boolean {
  return object(value) && text(value.code) && text(value.message);
}

function evidence(value: unknown): value is DMonitorEvidenceSummary {
  return (
    object(value) &&
    text(value.observation_id) &&
    text(value.observation_digest) &&
    text(value.node_id) &&
    text(value.monitor_instance_id) &&
    count(value.sequence) &&
    timestamp(value.observed_at_utc) &&
    timestamp(value.received_at_utc) &&
    FRESHNESS.has(String(value.freshness)) &&
    HEALTH.has(String(value.health)) &&
    CORRELATION.has(String(value.authority_correlation)) &&
    text(value.observed_subject_node_id) &&
    bool(value.admitted) &&
    optionalText(value.exclusion_reason)
  );
}

function healthClaim(value: unknown): boolean {
  return object(value) && text(value.monitor_instance_id) && HEALTH.has(String(value.health));
}

function servicePlacement(value: unknown): value is DMonitorServicePlacementConvergence {
  return (
    object(value) &&
    text(value.service_id) &&
    text(value.desired_node_id) &&
    CONVERGENCE.has(String(value.convergence)) &&
    Array.isArray(value.findings) &&
    value.findings.every(finding) &&
    Array.isArray(value.admitted_evidence) &&
    value.admitted_evidence.every(evidence) &&
    Array.isArray(value.excluded_evidence) &&
    value.excluded_evidence.every(evidence) &&
    Array.isArray(value.observed_health_claims) &&
    value.observed_health_claims.every(healthClaim)
  );
}

function unexpectedRealization(value: unknown): boolean {
  return (
    object(value) &&
    text(value.service_id) &&
    Array.isArray(value.admitted_evidence) &&
    value.admitted_evidence.every(evidence) &&
    Array.isArray(value.excluded_evidence) &&
    value.excluded_evidence.every(evidence)
  );
}

function dmapRef(value: unknown): boolean {
  return value === null || (object(value) && text(value.dmap_id) && count(value.revision) && text(value.digest));
}

function freshnessPolicy(value: unknown): boolean {
  return object(value) && count(value.max_observed_age_seconds) && count(value.max_received_age_seconds) && count(value.max_future_clock_skew_seconds);
}

export function validDMonitorApplicationConvergence(value: unknown, projectId?: string, applicationId?: string): value is DMonitorApplicationConvergence {
  return (
    object(value) &&
    text(value.schema) &&
    text(value.project_id) &&
    (projectId === undefined || value.project_id === projectId) &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    timestamp(value.evaluated_at_utc) &&
    dmapRef(value.dmap_ref) &&
    freshnessPolicy(value.freshness_policy) &&
    CONVERGENCE.has(String(value.convergence)) &&
    Array.isArray(value.findings) &&
    value.findings.every(finding) &&
    Array.isArray(value.services) &&
    value.services.every(servicePlacement) &&
    Array.isArray(value.unexpected_realizations) &&
    value.unexpected_realizations.every(unexpectedRealization)
  );
}

// The full combined response -- validates the D-Serv half fully (the only
// half this phase renders) and only structurally confirms `diot` is an
// object (never fully validated/rendered here -- DIoT keeps its own,
// separate, already-complete surface).
export function validEvaluateConvergenceResponse(value: unknown, projectId?: string, applicationId?: string): value is EvaluateConvergenceResponse {
  return object(value) && validDMonitorApplicationConvergence(value, projectId, applicationId) && object(value.diot);
}

// ---- Display mappings (enum -> label/tone only; never re-derives state) ----

export const CONVERGENCE_LABEL: Record<DMonitorConvergence, string> = {
  converged: "Converged",
  diverged: "Diverged",
  unknown: "Unknown",
};
export const CONVERGENCE_TONE: Record<DMonitorConvergence, "neutral" | "info" | "success" | "warning" | "danger"> = {
  converged: "success",
  diverged: "danger",
  unknown: "neutral",
};

// Part M -- Stale (an observation existed but aged out) and Unknown (no
// trustworthy observation at all) are deliberately DIFFERENT labels, never
// collapsed, even though both can contribute to an overall `unknown`
// convergence result.
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

// Part J7 -- labeled "Runtime observation health" in the view, never
// "application health": this is the DMonitor collector's own belief about
// the subject, structurally independent of convergence (Part 16).
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

// Part K -- the most recent ADMITTED evidence for one service placement, if
// any (oldest-received-last is not assumed; this picks the evidence with
// the latest `received_at_utc`, the ordering dimension this data honestly
// supports -- never "the one listed first").
export function latestAdmittedEvidence(placement: DMonitorServicePlacementConvergence): DMonitorEvidenceSummary | undefined {
  return placement.admitted_evidence.reduce<DMonitorEvidenceSummary | undefined>((latest, candidate) => {
    if (!latest) return candidate;
    return Date.parse(candidate.received_at_utc) > Date.parse(latest.received_at_utc) ? candidate : latest;
  }, undefined);
}

// Part J2/32 -- a command-side Succeeded attempt with no admitted evidence
// yet for that service is "Waiting for observation", a UI-only workflow
// label derived from already-true facts -- never a backend convergence
// state, never persisted, never itself called "Converged"/"Diverged"/
// "Unknown" (Part J5's own explicit rule).
export function isWaitingForFirstObservation(placement: DMonitorServicePlacementConvergence | undefined): boolean {
  return !placement || (placement.admitted_evidence.length === 0 && placement.excluded_evidence.length === 0);
}

// Part S -- polling stop condition: once convergence is anything other
// than "unknown" (a definite Converged or a definite Diverged proof), keep
// showing it but stop polling aggressively. "Unknown" alone keeps polling,
// since more evidence arriving could resolve it either way.
export function isConvergenceSettled(response: DMonitorApplicationConvergence | undefined): boolean {
  return !!response && response.convergence !== "unknown";
}

export function findServicePlacement(
  convergence: DMonitorApplicationConvergence | undefined,
  serviceId: string,
): DMonitorServicePlacementConvergence | undefined {
  return convergence?.services.find((service) => service.service_id === serviceId);
}

// Phase 138 -- Deploy / Reconciliation Console pure model: response
// validators and enum -> display mappings ONLY. No DOM, no network, no
// re-derivation of backend state machines (Part S) -- `effective_status`/
// `derive_batch_status` live in dserver and are trusted verbatim here.
import type {
  ReconciliationRequest,
  ReconciliationRequestStatus,
  ReconciliationBatchResponse,
  ReconciliationBatchCreateResponse,
  ReconciliationBatchStatus,
  ReconciliationAttemptRecord,
  ReconciliationAttemptStatus,
  ReconciliationActionResult,
  ReconciliationServiceResult,
  OperationalReconciliationAuthorization,
  OperationalDesiredService,
  OperationalDGraphSlice,
} from "../types/reconciliation.ts";

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
function optionalCount(value: unknown): boolean {
  return value === undefined || count(value);
}
function bool(value: unknown): value is boolean {
  return typeof value === "boolean";
}
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
function nullableText(value: unknown): boolean {
  return value === null || typeof value === "string";
}

const REQUEST_STATUSES = new Set(["pending", "claimed", "attempted", "stale", "expired"]);
const BATCH_STATUSES = new Set(["pending", "executing", "succeeded", "partial", "failed", "unknown", "stale"]);
const ATTEMPT_STATUSES = new Set(["succeeded", "partial", "failed", "unknown"]);
const ACTION_RESULTS = new Set(["no_op", "mutated", "blocked", "unknown"]);
const MODES = new Set(["preview", "execute"]);

export function validReconciliationRequest(value: unknown, projectId?: string): value is ReconciliationRequest {
  if (!object(value)) return false;
  return (
    text(value.schema_version) &&
    text(value.reconciliation_request_id) &&
    text(value.client_request_id) &&
    nullableText(value.reconciliation_batch_id ?? null) &&
    text(value.project_id) &&
    (projectId === undefined || value.project_id === projectId) &&
    text(value.node_id) &&
    text(value.target_dgraph_id) &&
    count(value.target_dgraph_revision) &&
    MODES.has(String(value.mode)) &&
    text(value.requested_by) &&
    timestamp(value.created_at_utc) &&
    timestamp(value.expires_at_utc) &&
    REQUEST_STATUSES.has(String(value.status)) &&
    nullableText(value.claim_token ?? null) &&
    nullableTimestamp(value.claimed_at_utc ?? null) &&
    nullableTimestamp(value.claim_expires_at_utc ?? null) &&
    nullableText(value.linked_attempt_id ?? null) &&
    timestamp(value.updated_at_utc)
  );
}

export function validReconciliationBatchCreateResponse(value: unknown, applicationId?: string): value is ReconciliationBatchCreateResponse {
  return (
    object(value) &&
    text(value.reconciliation_batch_id) &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    Array.isArray(value.requests) &&
    value.requests.every((r) => validReconciliationRequest(r))
  );
}

export function validReconciliationBatchResponse(value: unknown, projectId?: string): value is ReconciliationBatchResponse {
  return (
    object(value) &&
    text(value.reconciliation_batch_id) &&
    text(value.project_id) &&
    (projectId === undefined || value.project_id === projectId) &&
    BATCH_STATUSES.has(String(value.status)) &&
    Array.isArray(value.requests) &&
    value.requests.every((r) => validReconciliationRequest(r, projectId))
  );
}

function serviceResult(value: unknown): value is ReconciliationServiceResult {
  return (
    object(value) &&
    text(value.service_id) &&
    ACTION_RESULTS.has(String(value.result)) &&
    bool(value.host_mutation_performed) &&
    optionalText(value.execution_status) &&
    optionalText(value.message)
  );
}

export function validReconciliationAttemptRecord(value: unknown, projectId?: string): value is ReconciliationAttemptRecord {
  if (!object(value)) return false;
  if (
    !(
      text(value.schema_version) &&
      text(value.reconciliation_attempt_id) &&
      optionalText(value.reconciliation_request_id) &&
      text(value.project_id) &&
      (projectId === undefined || value.project_id === projectId) &&
      text(value.node_id) &&
      text(value.dgraph_id) &&
      optionalCount(value.dgraph_revision) &&
      optionalText(value.authorization_id) &&
      timestamp(value.started_at_utc) &&
      (value.finished_at_utc === undefined || timestamp(value.finished_at_utc)) &&
      ATTEMPT_STATUSES.has(String(value.status)) &&
      object(value.services) &&
      bool(value.services_truncated) &&
      count(value.last_applied_sequence) &&
      timestamp(value.created_at_utc) &&
      timestamp(value.updated_at_utc)
    )
  )
    return false;
  return Object.values(value.services as Record<string, unknown>).every(serviceResult);
}

export function validOperationalReconciliationAuthorization(value: unknown): value is OperationalReconciliationAuthorization {
  return (
    object(value) &&
    text(value.authorization_id) &&
    text(value.project_id) &&
    text(value.dgraph_id) &&
    count(value.dgraph_revision) &&
    text(value.node_id) &&
    text(value.authorized_by) &&
    timestamp(value.authorized_at_utc) &&
    bool(value.allow_host_mutation) &&
    text(value.operation) &&
    text(value.persistence_scope) &&
    (value.expires_at_utc === undefined || timestamp(value.expires_at_utc))
  );
}

function desiredService(value: unknown): value is OperationalDesiredService {
  return object(value) && text(value.service_id) && text(value.runtime) && text(value.desired_state) && text(value.application_id);
}

export function validOperationalDGraphSlice(value: unknown, projectId?: string, nodeId?: string): value is OperationalDGraphSlice {
  if (!object(value)) return false;
  if (
    !(
      text(value.schema_version) &&
      text(value.dgraph_id) &&
      count(value.dgraph_revision) &&
      text(value.project_id) &&
      (projectId === undefined || value.project_id === projectId) &&
      text(value.node_id) &&
      (nodeId === undefined || value.node_id === nodeId) &&
      Array.isArray(value.desired_services) &&
      value.desired_services.every(desiredService)
    )
  )
    return false;
  return value.authorization === undefined || validOperationalReconciliationAuthorization(value.authorization);
}

// ---- Display mappings (enum -> label/tone only; never re-derives state) ----

export const REQUEST_STATUS_LABEL: Record<ReconciliationRequestStatus, string> = {
  pending: "Pending · waiting for agent",
  claimed: "Claimed by agent",
  attempted: "Attempted",
  stale: "Stale",
  expired: "Expired",
};
export const REQUEST_STATUS_TONE: Record<ReconciliationRequestStatus, "neutral" | "info" | "success" | "warning" | "danger"> = {
  pending: "info",
  claimed: "info",
  attempted: "success",
  stale: "warning",
  expired: "neutral",
};

export const BATCH_STATUS_LABEL: Record<ReconciliationBatchStatus, string> = {
  pending: "Pending",
  executing: "Executing",
  succeeded: "Succeeded",
  partial: "Partial",
  failed: "Failed",
  unknown: "Unknown",
  stale: "Stale",
};
export const BATCH_STATUS_TONE: Record<ReconciliationBatchStatus, "neutral" | "info" | "success" | "warning" | "danger"> = {
  pending: "info",
  executing: "info",
  succeeded: "success",
  partial: "warning",
  failed: "danger",
  unknown: "neutral",
  stale: "warning",
};

// Command-side result language only (Part L) -- never implies runtime
// health. "Succeeded" means the command-side reconciliation attempt
// completed; it is not a health or convergence claim.
export const ATTEMPT_STATUS_LABEL: Record<ReconciliationAttemptStatus, string> = {
  succeeded: "Succeeded (command-side)",
  partial: "Partial (mixed command-side outcomes)",
  failed: "Failed (command-side)",
  unknown: "Unknown (command-side result could not be determined)",
};
export const ATTEMPT_STATUS_TONE: Record<ReconciliationAttemptStatus, "neutral" | "info" | "success" | "warning" | "danger"> = {
  succeeded: "success",
  partial: "warning",
  failed: "danger",
  unknown: "neutral",
};

// Only the distinctions the backend itself actually exposes (Part X) --
// never invented. "No-op" / "Mutated" / "Blocked" / "Unknown" exactly
// mirror `ReconciliationActionResult`; there is no richer
// Create/Start/Replace/Remove vocabulary to surface.
export const ACTION_RESULT_LABEL: Record<ReconciliationActionResult, string> = {
  no_op: "No change",
  mutated: "Mutated",
  blocked: "Blocked",
  unknown: "Unknown",
};
export const ACTION_RESULT_TONE: Record<ReconciliationActionResult, "neutral" | "info" | "success" | "warning" | "danger"> = {
  no_op: "neutral",
  mutated: "success",
  blocked: "warning",
  unknown: "neutral",
};

// Part E1 -- authorization status, computed ONLY from fields the server
// already supplies (`allow_host_mutation`, `expires_at_utc`,
// `dgraph_revision` vs. the slice's own CURRENT `dgraph_revision`). This is
// not re-deriving dserver's own lease-expiry math (that remains
// `should_restore_authorization`'s job at the moment of actual mutation) --
// it is a DISPLAY-ONLY read of already-served fields, same spirit as every
// other enum->label mapping in this file. A clock-skewed browser can make
// this display momentarily wrong in either direction; it is never used to
// gate anything, only to inform the operator before they click Reconcile.
export type AuthorizationDisplayStatus = "authorized" | "not_authorized" | "expired" | "wrong_target" | "none";

export function authorizationDisplayStatus(
  authorization: OperationalReconciliationAuthorization | undefined,
  currentDgraphId: string,
  currentDgraphRevision: number,
  nowMs: number = Date.now(),
): AuthorizationDisplayStatus {
  if (!authorization) return "none";
  if (!authorization.allow_host_mutation) return "not_authorized";
  if (authorization.dgraph_id !== currentDgraphId || authorization.dgraph_revision !== currentDgraphRevision) return "wrong_target";
  if (authorization.expires_at_utc && Date.parse(authorization.expires_at_utc) <= nowMs) return "expired";
  return "authorized";
}

export const AUTHORIZATION_STATUS_LABEL: Record<AuthorizationDisplayStatus, string> = {
  authorized: "Authorized for the current target",
  not_authorized: "Not authorized",
  expired: "Authorization expired",
  wrong_target: "Authorization pinned to a different target",
  none: "No authorization issued",
};
export const AUTHORIZATION_STATUS_TONE: Record<AuthorizationDisplayStatus, "neutral" | "info" | "success" | "warning" | "danger"> = {
  authorized: "success",
  not_authorized: "neutral",
  expired: "warning",
  wrong_target: "warning",
  none: "neutral",
};

// Part G1 -- whether a request/batch status is terminal (stop polling it).
export function isTerminalRequestStatus(status: ReconciliationRequestStatus): boolean {
  return status === "attempted" || status === "stale" || status === "expired";
}
export function isTerminalBatchStatus(status: ReconciliationBatchStatus): boolean {
  return status === "succeeded" || status === "partial" || status === "failed" || status === "unknown" || status === "stale";
}

// Part H2/35/36 -- a desired service belongs to an application OTHER than
// the one this workspace was opened for. Pure, used to flag (never hide)
// cross-application services in preview/attempt displays.
export function isOtherApplicationService(service: { application_id: string }, applicationId: string): boolean {
  return service.application_id !== "" && service.application_id !== applicationId;
}

// Part F3/O -- the `client_request_id` lifecycle for one submission
// attempt. Reuses the PRIOR attempt's id only when that prior attempt's
// own outcome was never confirmed (a transport-level failure: the HTTP
// response itself was never received, so retrying is genuinely a retry of
// the SAME logical attempt, and the server's own idempotent handling of a
// repeated id is what prevents a duplicate reconciliation). Any other case
// -- no prior attempt, or a prior attempt whose outcome IS known (success
// or a definite rejection) -- always generates a fresh id, because that is
// a genuinely NEW operator action, never a retry of the old one.
export function nextClientRequestId(lastId: string | undefined, lastOutcomeKnown: boolean, generateId: () => string): string {
  if (!lastOutcomeKnown && lastId) return lastId;
  return generateId();
}

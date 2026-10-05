// Phase 138 -- Deploy / Reconciliation Console types. Mirrors
// `dserver/src/core/reconciliation_request.rs` and
// `dserver/src/core/reconciliation_attempt.rs` field-for-field, plus the
// relevant subset of `dserver/src/core/operational_dgraph.rs` already
// served by the existing operational-dgraph-slice / operational-dgraph
// read endpoints and the existing reconciliation-authorization issuance
// endpoints. Six distinct objects, never conflated, exactly as the backend
// keeps them separate:
//
//   Accepted placement authority (ddeploy/ActiveResponse, already in
//   types/placement.ts)
//     -> ReconciliationRequest (this file): operator/system intent to
//        reconcile, pinned to an exact operational D-Graph target.
//     -> OperationalReconciliationAuthorization (this file): PERMISSION to
//        mutate a host. Claiming a request does NOT grant this.
//     -> ReconciliationAttemptRecord (this file): the agent-side command
//        execution attempt a claimed request caused.
//     -> OperationalEvidenceEvent: command-side evidence (not surfaced as
//        its own Console type -- it is already folded into the attempt
//        record above).
//     -> DMonitor: runtime OBSERVATION authority -- out of scope here
//        entirely (Phase 139).
//
// Command success != observed success. Observed != healthy. Healthy !=
// converged. Nothing in this file or its consumers may imply otherwise.

export type ReconciliationMode = "preview" | "execute";
export type ReconciliationRequestStatus = "pending" | "claimed" | "attempted" | "stale" | "expired";

// `GET/POST .../reconciliation-requests[...]` response shape. Mirrors
// `ReconciliationRequestV1` field-for-field.
export interface ReconciliationRequest {
  schema_version: string;
  reconciliation_request_id: string;
  client_request_id: string;
  reconciliation_batch_id: string | null;
  project_id: string;
  node_id: string;
  target_dgraph_id: string;
  target_dgraph_revision: number;
  mode: ReconciliationMode;
  // Audit label only -- never authenticated identity (ADR-0013 §6), same
  // discipline as Phase 137's `accepted_by`. Never render as "authenticated
  // user" or similar.
  requested_by: string;
  created_at_utc: string;
  expires_at_utc: string;
  status: ReconciliationRequestStatus;
  claim_token: string | null;
  claimed_at_utc: string | null;
  claim_expires_at_utc: string | null;
  linked_attempt_id: string | null;
  updated_at_utc: string;
}

export type ReconciliationBatchStatus = "pending" | "executing" | "succeeded" | "partial" | "failed" | "unknown" | "stale";

// `GET /api/v1/reconciliation-requests/batch/:id` response. The
// SERVER-COMPUTED aggregate -- never re-derived client-side (Part S).
export interface ReconciliationBatchResponse {
  reconciliation_batch_id: string;
  project_id: string;
  status: ReconciliationBatchStatus;
  requests: ReconciliationRequest[];
}

// `POST /api/v1/reconciliation-requests/batch` response. No `status` field
// yet (the batch has just been created, every member is freshly Pending) --
// use `GET .../batch/:id` for the aggregate status once members may have
// progressed.
export interface ReconciliationBatchCreateResponse {
  reconciliation_batch_id: string;
  application_id: string;
  requests: ReconciliationRequest[];
}

export type ReconciliationAttemptStatus = "succeeded" | "partial" | "failed" | "unknown";
export type ReconciliationActionResult = "no_op" | "mutated" | "blocked" | "unknown";

export interface ReconciliationServiceResult {
  service_id: string;
  result: ReconciliationActionResult;
  host_mutation_performed: boolean;
  execution_status?: string;
  message?: string;
}

// `GET /api/v1/reconciliation-attempts/:id` response. Mirrors
// `ReconciliationAttemptRecordV1` field-for-field. COMMAND-SIDE record only
// -- "Succeeded" means the agent reported the reconciliation command
// completed; it is never evidence of runtime health or convergence.
export interface ReconciliationAttemptRecord {
  schema_version: string;
  reconciliation_attempt_id: string;
  reconciliation_request_id?: string;
  project_id: string;
  // Node-scoped, not application-scoped -- one attempt's `services` map may
  // legitimately include services belonging to applications OTHER than the
  // one the operator triggered this from (see Part H in the phase
  // documentation). Never filtered or hidden here.
  node_id: string;
  dgraph_id: string;
  dgraph_revision?: number;
  authorization_id?: string;
  started_at_utc: string;
  finished_at_utc?: string;
  status: ReconciliationAttemptStatus;
  services: Record<string, ReconciliationServiceResult>;
  services_truncated: boolean;
  last_applied_sequence: number;
  created_at_utc: string;
  updated_at_utc: string;
}

// The subset of `OperationalReconciliationAuthorization`
// (`dserver/src/core/operational_dgraph.rs`) Console actually renders.
// Extra backend fields (artifact_bindings, execution_pinning_mode,
// binding_id, resource_id, snapshot) are intentionally not modeled here --
// this is a DISPLAY projection, not a re-implementation of the backend
// type; `allow_host_mutation`/`expires_at_utc`/`dgraph_revision` are the
// only fields Console's own authorization-status logic reads.
export interface OperationalReconciliationAuthorization {
  authorization_id: string;
  project_id: string;
  dgraph_id: string;
  dgraph_revision: number;
  node_id: string;
  authorized_by: string;
  authorized_at_utc: string;
  allow_host_mutation: boolean;
  operation: string;
  persistence_scope: string;
  expires_at_utc?: string;
}

export interface OperationalDesiredService {
  service_id: string;
  runtime: string;
  desired_state: string;
  application_id: string;
}

// `GET /api/v1/nodes/:node_id/operational-dgraph-slice` response -- the
// subset Console renders. Reused directly from the SAME endpoint the agent
// itself polls every cycle (Part B/E); Console never gets a separate,
// richer read model for this.
export interface OperationalDGraphSlice {
  schema_version: string;
  dgraph_id: string;
  dgraph_revision: number;
  project_id: string;
  node_id: string;
  desired_services: OperationalDesiredService[];
  authorization?: OperationalReconciliationAuthorization;
}

// `POST /api/v1/nodes/:node_id/operational-reconciliation/authorize`
// request body.
export interface AuthorizeRequest {
  dgraph_id: string;
  dgraph_revision?: number;
  authorized_by: string;
  allow_host_mutation: boolean;
  valid_for_seconds?: number;
}

// The backend's error envelope for this family of endpoints: `{"error":
// "<message>", "code": "<stable_code>"}` -- NOT the ddeploy family's flat
// `{"error":"<code>",...}` shape (see `api/placement.ts`'s own comment on
// that distinction). `code` is additive (Phase 138 Part AG) specifically so
// Console can branch safely (e.g. `reconciliation_queue_full` vs
// `reconciliation_request_stale` vs `reconciliation_request_not_claimable`,
// all of which share HTTP 409) without parsing English prose.
export interface ReconciliationErrorBody {
  error: string;
  code: string;
}

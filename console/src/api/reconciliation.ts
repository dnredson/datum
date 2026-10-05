// Phase 138 -- Deploy / Reconciliation Console API client. Talks to three
// pre-existing endpoint families, never a bespoke new one of its own:
//   - `/api/v1/nodes/:node_id/operational-dgraph-slice` (read) and
//     `/api/v1/nodes/:node_id/operational-reconciliation/authorize` (write)
//     -- error envelope `{"error": "<message>"}`, no `code` field.
//   - `/api/v1/reconciliation-requests[...]` (Phase 138C) -- error envelope
//     `{"error": "<message>", "code": "<stable_code>"}` (Part AG addition).
//   - `/api/v1/reconciliation-attempts/:id` (Phase 138A, read-only).
// This client never calls anything that mutates a host directly -- it only
// ever creates/reads REQUEST and AUTHORIZATION state. The node-resident
// DATUM agent remains the sole thing that performs host mutation.
import type {
  OperationalDGraphSlice,
  OperationalReconciliationAuthorization,
  AuthorizeRequest,
  ReconciliationBatchCreateResponse,
  ReconciliationBatchResponse,
  ReconciliationAttemptRecord,
} from "../types/reconciliation.ts";
import {
  validOperationalDGraphSlice,
  validOperationalReconciliationAuthorization,
  validReconciliationBatchCreateResponse,
  validReconciliationBatchResponse,
  validReconciliationAttemptRecord,
} from "../pages/reconciliation-model.ts";

export class ReconciliationError extends Error {
  // `null` for the slice/authorize endpoint family, which carries no
  // structured code -- callers there must branch on `status`/message text
  // only, same as before this phase (Part AG scoped the code addition to
  // the reconciliation-request family alone).
  code: string | null;
  status: number | null;
  constructor(message: string, code: string | null, status: number | null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

const obj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

export class ReconciliationClient {
  private project: string;
  private signal?: AbortSignal;
  constructor(project: string, signal?: AbortSignal) {
    this.project = project;
    this.signal = signal;
  }

  private async request(method: "GET" | "POST", path: string, rawBody?: string): Promise<unknown> {
    const controller = new AbortController();
    let timedOut = false;
    const abort = () => controller.abort();
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, 15000);
    this.signal?.addEventListener("abort", abort, { once: true });
    if (this.signal?.aborted) controller.abort();
    try {
      const response = await fetch(path, {
        method,
        headers: {
          "X-Datum-Project": this.project,
          Accept: "application/json",
          ...(rawBody === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: rawBody,
        signal: controller.signal,
        cache: "no-store",
      });
      const raw = await response.text();
      if (raw.length > 2_097_152) throw new ReconciliationError("The response exceeded the accepted read size.", "invalid_response", response.status);
      let value: unknown;
      try {
        value = raw.length ? JSON.parse(raw) : null;
      } catch {
        throw new ReconciliationError("DServer did not return JSON.", "invalid_response", response.status);
      }
      if (!response.ok) {
        const errorText = obj(value) && typeof value.error === "string" && new TextEncoder().encode(value.error).length <= 2048 ? value.error : "The request failed.";
        const code = obj(value) && typeof value.code === "string" && /^[a-z_]{1,64}$/u.test(value.code) ? value.code : null;
        throw new ReconciliationError(errorText, code, response.status);
      }
      return value;
    } catch (e) {
      if (this.signal?.aborted) throw e;
      if (e instanceof ReconciliationError) throw e;
      throw new ReconciliationError(timedOut ? "DServer did not respond before the read timeout." : "The Console could not connect to DServer.", timedOut ? "timeout" : "disconnected", null);
    } finally {
      clearTimeout(timer);
      this.signal?.removeEventListener("abort", abort);
    }
  }

  // Reuses the EXACT endpoint the DATUM agent itself polls every cycle
  // (Part B/E) -- Console gets no separate, richer read model. Carries
  // `desired_services` (including services belonging to OTHER
  // applications co-located on this node, Part H2) and the current
  // `authorization`, if any.
  async slice(nodeId: string): Promise<OperationalDGraphSlice> {
    const value = await this.request("GET", `/api/v1/nodes/${encodeURIComponent(nodeId)}/operational-dgraph-slice`);
    if (!validOperationalDGraphSlice(value, this.project, nodeId)) throw new ReconciliationError("DServer returned an invalid operational D-Graph slice.", "invalid_response", null);
    return value;
  }

  // Part E2 -- issues (or re-issues) permission to mutate a host for the
  // exact pinned operational authority named in `request`. Does NOT itself
  // start reconciliation -- see this type's own doc comment.
  async authorize(nodeId: string, request: AuthorizeRequest): Promise<OperationalReconciliationAuthorization> {
    const body = JSON.stringify(request);
    const value = await this.request("POST", `/api/v1/nodes/${encodeURIComponent(nodeId)}/operational-reconciliation/authorize`, body);
    if (!validOperationalReconciliationAuthorization(value)) throw new ReconciliationError("DServer returned an invalid authorization response.", "invalid_response", null);
    return value;
  }

  // Part F2/I1 -- ALWAYS the batch endpoint, even for a single-node
  // application, so the rest of this client/UI only ever has to reason
  // about ONE shape (a batch of 1-or-more node-scoped requests) rather than
  // two parallel code paths. Server-side node derivation only -- this
  // method never accepts or sends a node list (Part F2's own explicit
  // requirement).
  async createBatch(applicationId: string, mode: "preview" | "execute", requestedBy: string, clientRequestId: string): Promise<ReconciliationBatchCreateResponse> {
    const body = JSON.stringify({ application_id: applicationId, mode, requested_by: requestedBy, client_request_id: clientRequestId });
    const value = await this.request("POST", "/api/v1/reconciliation-requests/batch", body);
    if (!validReconciliationBatchCreateResponse(value, applicationId)) throw new ReconciliationError("DServer returned an invalid batch-creation response.", "invalid_response", null);
    return value;
  }

  // Part I2/S -- the server-computed aggregate. Never re-derived here.
  async getBatch(batchId: string): Promise<ReconciliationBatchResponse> {
    const value = await this.request("GET", `/api/v1/reconciliation-requests/batch/${encodeURIComponent(batchId)}`);
    if (!validReconciliationBatchResponse(value, this.project)) throw new ReconciliationError("DServer returned an invalid batch response.", "invalid_response", null);
    return value;
  }

  // Phase 138A, read-only -- the command-side attempt a claimed request's
  // agent produced, once `linked_attempt_id` is known.
  async getAttempt(attemptId: string): Promise<ReconciliationAttemptRecord> {
    const value = await this.request("GET", `/api/v1/reconciliation-attempts/${encodeURIComponent(attemptId)}`);
    if (!validReconciliationAttemptRecord(value, this.project)) throw new ReconciliationError("DServer returned an invalid reconciliation attempt record.", "invalid_response", null);
    return value;
  }
}

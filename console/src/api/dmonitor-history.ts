// Phase 140A -- DMonitor historical read client. Wraps the new, read-only
// `GET /api/v1/dmonitor/history/:project_id` endpoint. Exposes exactly one
// method (`query`) -- no remediation, no mutation, no second method that
// could later grow one, mirroring `DMonitorConvergenceClient`'s own
// structural discipline exactly.
import type { DMonitorHistoryQuery, DMonitorHistoryQueryResponse } from "../types/dmonitor-history.ts";
import { validDMonitorHistoryQueryResponse } from "../pages/dmonitor-history-model.ts";

export class DMonitorHistoryError extends Error {
  code: string | null;
  status: number | null;
  constructor(message: string, code: string | null, status: number | null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

const obj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

export class DMonitorHistoryClient {
  private project: string;
  private signal?: AbortSignal;
  constructor(project: string, signal?: AbortSignal) {
    this.project = project;
    this.signal = signal;
  }

  // Read-only: GET only, no body, no mutation. `query` filters mirror the
  // backend's own logical-identity dimensions exactly -- never PID/
  // container-id/monitor_instance_id (Part A, that identity is deliberately
  // never exposed as a filter here, client or server side).
  async query(filters: DMonitorHistoryQuery = {}): Promise<DMonitorHistoryQueryResponse> {
    const params = new URLSearchParams();
    if (filters.applicationId) params.set("application_id", filters.applicationId);
    if (filters.nodeId) params.set("node_id", filters.nodeId);
    if (filters.serviceId) params.set("service_id", filters.serviceId);
    if (filters.startUtc) params.set("start_utc", filters.startUtc);
    if (filters.endUtc) params.set("end_utc", filters.endUtc);
    if (filters.limit !== undefined) params.set("limit", String(filters.limit));
    const query = params.toString();
    const path = `/api/v1/dmonitor/history/${encodeURIComponent(this.project)}${query ? `?${query}` : ""}`;

    const controller = new AbortController();
    let timedOut = false;
    const abort = () => controller.abort();
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, 15000);
    this.signal?.addEventListener("abort", abort, { once: true });
    if (this.signal?.aborted) controller.abort();
    let value: unknown;
    let status: number | null = null;
    try {
      const response = await fetch(path, {
        method: "GET",
        headers: { Accept: "application/json" },
        signal: controller.signal,
        cache: "no-store",
      });
      status = response.status;
      const raw = await response.text();
      if (raw.length > 4_194_304) throw new DMonitorHistoryError("The response exceeded the accepted read size.", "invalid_response", status);
      try {
        value = raw.length ? JSON.parse(raw) : null;
      } catch {
        throw new DMonitorHistoryError("DServer did not return JSON.", "invalid_response", status);
      }
      if (!response.ok) {
        const errorField = obj(value) ? value.error : undefined;
        const messageField = obj(value) && typeof value.message === "string" ? value.message : undefined;
        const code = typeof errorField === "string" && /^[a-z_]{1,64}$/u.test(errorField) ? errorField : null;
        const message = messageField ?? (typeof errorField === "string" ? errorField : "The historical query failed.");
        throw new DMonitorHistoryError(message, code, status);
      }
    } catch (e) {
      clearTimeout(timer);
      this.signal?.removeEventListener("abort", abort);
      if (this.signal?.aborted) throw e;
      if (e instanceof DMonitorHistoryError) throw e;
      throw new DMonitorHistoryError(timedOut ? "DServer did not respond before the read timeout." : "The Console could not connect to DServer.", timedOut ? "timeout" : "disconnected", null);
    }
    clearTimeout(timer);
    this.signal?.removeEventListener("abort", abort);
    if (!validDMonitorHistoryQueryResponse(value, this.project)) {
      throw new DMonitorHistoryError("DServer returned an invalid historical query response.", "invalid_response", null);
    }
    return value;
  }
}

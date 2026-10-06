// Phase 141B -- read-only client for the canonical application-scoped
// operational context (`GET /api/v1/operations/projects/:project_id/applications/:application_id`).
// Exposes exactly one method (`get`) -- no mutation, no remediation, no
// authority of any kind, mirroring `DMonitorConvergenceClient`/
// `DMonitorHistoryClient`'s own established single-method discipline
// exactly.
import type { ApplicationOperationsOverview } from "../types/application-overview.ts";
import { validApplicationOperationsOverview } from "../pages/application-overview-model.ts";

export class ApplicationOverviewError extends Error {
  code: string | null;
  status: number | null;
  constructor(message: string, code: string | null, status: number | null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

const obj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

export class ApplicationOverviewClient {
  private project: string;
  private signal?: AbortSignal;
  constructor(project: string, signal?: AbortSignal) {
    this.project = project;
    this.signal = signal;
  }

  // Read-only: GET only, no body, no mutation.
  async get(applicationId: string): Promise<ApplicationOperationsOverview> {
    const path = `/api/v1/operations/projects/${encodeURIComponent(this.project)}/applications/${encodeURIComponent(applicationId)}`;
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
        headers: { Accept: "application/json", "X-Datum-Project": this.project },
        signal: controller.signal,
        cache: "no-store",
      });
      status = response.status;
      const raw = await response.text();
      if (raw.length > 4_194_304) throw new ApplicationOverviewError("The response exceeded the accepted read size.", "invalid_response", status);
      try {
        value = raw.length ? JSON.parse(raw) : null;
      } catch {
        throw new ApplicationOverviewError("DServer did not return JSON.", "invalid_response", status);
      }
      if (!response.ok) {
        const errorField = obj(value) ? value.error : undefined;
        const code = typeof errorField === "string" && /^[a-z_]{1,64}$/u.test(errorField) ? errorField : null;
        const message = obj(value) && obj(value.error) && typeof value.error.message === "string" ? value.error.message : "The application overview request failed.";
        throw new ApplicationOverviewError(message, code, status);
      }
    } catch (e) {
      clearTimeout(timer);
      this.signal?.removeEventListener("abort", abort);
      if (this.signal?.aborted) throw e;
      if (e instanceof ApplicationOverviewError) throw e;
      throw new ApplicationOverviewError(timedOut ? "DServer did not respond before the read timeout." : "The Console could not connect to DServer.", timedOut ? "timeout" : "disconnected", null);
    }
    clearTimeout(timer);
    this.signal?.removeEventListener("abort", abort);
    if (!validApplicationOperationsOverview(value, this.project, applicationId)) {
      throw new ApplicationOverviewError("DServer returned an invalid application overview response.", "invalid_response", null);
    }
    return value;
  }
}

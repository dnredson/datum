// Phase 139B -- DMonitor convergence read client. Wraps the EXISTING,
// unmodified `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate`
// endpoint (Phase 114C) -- read-only, no mutation, no remediation. This
// client never creates a placement proposal, never accepts one, never
// creates a reconciliation request, and never authorizes host mutation; it
// only evaluates and reads back desired-vs-observed convergence for an
// already-accepted application.
import type { EvaluateConvergenceResponse, EvaluateConvergenceRequest } from "../types/dmonitor-convergence.ts";
import { validEvaluateConvergenceResponse } from "../pages/dmonitor-convergence-model.ts";

export class DMonitorConvergenceError extends Error {
  // This endpoint's own existing error envelope is `{"error": "<code or
  // message>", "message"?: "<text>"}` -- a THIRD distinct shape from both
  // the ddeploy family and the reconciliation-request family (see
  // `api/placement.ts`'s and `api/reconciliation.ts`'s own comments on
  // their respective shapes). `code` is `null` whenever the body did not
  // look like a stable bounded identifier.
  code: string | null;
  status: number | null;
  constructor(message: string, code: string | null, status: number | null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

const obj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

// Phase 114C's own `default_readiness_policy()` reasoning, reused verbatim
// as this client's own default request body -- never invented: 90s
// observed/received tolerance (three nominal 30s DIoT heartbeat periods),
// 10s clock-skew tolerance (same as every other DMonitor freshness policy
// already used in this codebase). Never silently widened/narrowed without
// re-reading that reasoning.
export const DEFAULT_FRESHNESS_POLICY: EvaluateConvergenceRequest = {
  max_observed_age_seconds: 90,
  max_received_age_seconds: 90,
  max_future_clock_skew_seconds: 10,
};

export class DMonitorConvergenceClient {
  private project: string;
  private signal?: AbortSignal;
  constructor(project: string, signal?: AbortSignal) {
    this.project = project;
    this.signal = signal;
  }

  private async request(path: string, rawBody: string): Promise<unknown> {
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
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: rawBody,
        signal: controller.signal,
        cache: "no-store",
      });
      const raw = await response.text();
      if (raw.length > 4_194_304) throw new DMonitorConvergenceError("The response exceeded the accepted read size.", "invalid_response", response.status);
      let value: unknown;
      try {
        value = raw.length ? JSON.parse(raw) : null;
      } catch {
        throw new DMonitorConvergenceError("DServer did not return JSON.", "invalid_response", response.status);
      }
      if (!response.ok) {
        const errorField = obj(value) ? value.error : undefined;
        const messageField = obj(value) && typeof value.message === "string" ? value.message : undefined;
        const code = typeof errorField === "string" && /^[a-z_]{1,64}$/u.test(errorField) ? errorField : null;
        const message = messageField ?? (typeof errorField === "string" ? errorField : "The convergence evaluation request failed.");
        throw new DMonitorConvergenceError(message, code, response.status);
      }
      return value;
    } catch (e) {
      if (this.signal?.aborted) throw e;
      if (e instanceof DMonitorConvergenceError) throw e;
      throw new DMonitorConvergenceError(timedOut ? "DServer did not respond before the read timeout." : "The Console could not connect to DServer.", timedOut ? "timeout" : "disconnected", null);
    } finally {
      clearTimeout(timer);
      this.signal?.removeEventListener("abort", abort);
    }
  }

  // Read-only: no mutation, no remediation, no authority of any kind.
  // `policy` defaults to `DEFAULT_FRESHNESS_POLICY` -- never invented ad
  // hoc per call site (Part J4).
  async evaluate(applicationId: string, policy: EvaluateConvergenceRequest = DEFAULT_FRESHNESS_POLICY): Promise<EvaluateConvergenceResponse> {
    const path = `/api/v1/dmonitor/convergence/${encodeURIComponent(this.project)}/${encodeURIComponent(applicationId)}/evaluate`;
    const value = await this.request(path, JSON.stringify(policy));
    if (!validEvaluateConvergenceResponse(value, this.project, applicationId)) {
      throw new DMonitorConvergenceError("DServer returned an invalid convergence response.", "invalid_response", null);
    }
    return value;
  }
}

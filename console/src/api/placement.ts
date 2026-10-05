import { canonicalJson } from "../pages/dgraph-editor-model.ts";
import type { DGraph } from "../pages/dgraph-editor-model.ts";
import {
  validCandidateResponse,
  validValidateResponse,
  validProposalResponse,
  validActiveResponse,
} from "../pages/placement-model.ts";
import type {
  CandidateResponse,
  ValidateResponse,
  ProposalResponse,
  ActiveResponse,
  PlacementRef,
  ProposalSource,
} from "../types/placement.ts";

// DServer's ddeploy API error envelope is FLAT: {"error": "<code>",
// "message"?: "...", "findings"?: [{"code","message"}]} -- never the
// nested {"error":{"code",...}} shape OperationsClient/DGraphEditorClient
// use. `findings` carries the backend's own truthful reason codes (e.g.
// "no_eligible_node", "unsatisfied_static_capacity") for a failed
// candidate/validate/create call -- the Console renders these verbatim,
// never a fabricated prose explanation.
export class PlacementError extends Error {
  code: string;
  findings: { code: string; message: string }[];
  constructor(code: string, message?: string, findings: { code: string; message: string }[] = []) {
    super(
      message ??
        (code === "invalid_response"
          ? "DServer returned an invalid placement response."
          : "Placement request failed. No local draft was lost."),
    );
    this.code = code;
    this.findings = findings;
  }
}

const obj = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
function findingsArray(v: unknown): { code: string; message: string }[] {
  return Array.isArray(v)
    ? v.filter((f): f is { code: string; message: string } => obj(f) && typeof f.code === "string" && typeof f.message === "string")
    : [];
}

// A structurally-valid-but-empty placeholder D-Continuum. Every ddeploy
// endpoint that accepts a caller-supplied `dcontinuum` field REPLACES its
// content with the server-owned authoritative one before using it for
// anything (ADR-0019) -- this placeholder exists only to satisfy the
// request body's own structural shape requirement; its content is never
// trusted or read.
function placeholderDcontinuum(projectId: string): string {
  return `{"schema":"datum.dcontinuum/1","project_id":${JSON.stringify(projectId)},"dcontinuum_id":"continuum:placeholder","revision":1,"nodes":[],"links":[]}`;
}

export class PlacementClient {
  private project: string;
  private signal?: AbortSignal;
  constructor(project: string, signal?: AbortSignal) {
    this.project = project;
    this.signal = signal;
  }

  private async request(path: string, rawBody?: string): Promise<unknown> {
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
      const response = await fetch(`/api/v1/ddeploy${path}`, {
        method: rawBody === undefined ? "GET" : "POST",
        headers: {
          "X-Datum-Project": this.project,
          ...(rawBody === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: rawBody,
        signal: controller.signal,
      });
      const raw = await response.text();
      if (raw.length > 2_097_152) throw new PlacementError("invalid_response");
      let value: unknown;
      try {
        value = JSON.parse(raw);
      } catch {
        throw new PlacementError("invalid_response");
      }
      if (!response.ok) {
        const error = obj(value) ? value.error : undefined;
        const code = typeof error === "string" && /^[a-z_]{1,64}$/u.test(error) ? error : "unavailable";
        const message = obj(value) && typeof value.message === "string" ? value.message : undefined;
        const findings = obj(value) ? findingsArray(value.findings) : [];
        throw new PlacementError(code, message, findings);
      }
      return value;
    } catch (e) {
      if (this.signal?.aborted) throw e;
      if (e instanceof PlacementError) throw e;
      throw new PlacementError(timedOut ? "timeout" : "unavailable");
    } finally {
      clearTimeout(timer);
      this.signal?.removeEventListener("abort", abort);
    }
  }

  // Read-only accepted authority for this project (optionally one
  // application) -- the CURRENT state every candidate is diffed against.
  // Never itself a candidate/proposal source.
  async active(applicationId?: string): Promise<ActiveResponse> {
    const query = applicationId ? `?application_id=${encodeURIComponent(applicationId)}` : "";
    const value = await this.request(`/active${query}`);
    if (!validActiveResponse(value, this.project)) throw new PlacementError("invalid_response");
    return value;
  }

  // Preview-only deterministic placement candidate. ZERO persistence --
  // see documentation/reference/phase136-placement-ui.md. Caller supplies
  // only application_id/dgraph; every other input (authoritative
  // D-Continuum, global commitment baseline, ServiceArtifact eligibility)
  // is server-derived.
  async candidate(applicationId: string, dgraph: DGraph): Promise<CandidateResponse> {
    const body = `{"application_id":${JSON.stringify(applicationId)},"dgraph":${canonicalJson(dgraph)}}`;
    const value = await this.request("/candidate", body);
    if (!validCandidateResponse(value, this.project, applicationId)) throw new PlacementError("invalid_response");
    return value;
  }

  // Read-only dry-run validation of a (possibly operator-edited) candidate
  // draft. ZERO persistence. `capacity_check_is_momentary` on the response
  // is always present -- never treat a `valid: true` result as a
  // reservation.
  async validateCandidate(
    applicationId: string,
    dgraph: DGraph,
    placements: PlacementRef[],
    source: ProposalSource,
  ): Promise<ValidateResponse> {
    const body = `{"application_id":${JSON.stringify(applicationId)},"source":${JSON.stringify(source)},"dgraph":${canonicalJson(dgraph)},"dcontinuum":${placeholderDcontinuum(this.project)},"placements":${JSON.stringify(placements)}}`;
    const value = await this.request("/proposals/validate", body);
    if (!validValidateResponse(value)) throw new PlacementError("invalid_response");
    return value;
  }

  // Persists an immutable PENDING proposal. NOT active authority -- Phase
  // 137 owns the only path that can make it so. Rejected (422, nothing
  // persisted) if the placements already violate current global D-Node
  // capacity, same as validateCandidate would report.
  async createProposal(
    applicationId: string,
    dgraph: DGraph,
    placements: PlacementRef[],
    source: ProposalSource,
  ): Promise<ProposalResponse> {
    const body = `{"application_id":${JSON.stringify(applicationId)},"source":${JSON.stringify(source)},"dgraph":${canonicalJson(dgraph)},"dcontinuum":${placeholderDcontinuum(this.project)},"placements":${JSON.stringify(placements)}}`;
    const value = await this.request("/proposals", body);
    if (!validProposalResponse(value, this.project, applicationId)) throw new PlacementError("invalid_response");
    return value;
  }

  async getProposal(proposalId: string): Promise<ProposalResponse> {
    const value = await this.request(`/proposals/${encodeURIComponent(proposalId)}`);
    if (!validProposalResponse(value, this.project)) throw new PlacementError("invalid_response");
    return value;
  }
}

// Phase 139B -- DMonitor derived desired-vs-observed convergence types.
// Mirrors `dserver/src/core/dmonitor_interpretation.rs`'s
// `DMonitorApplicationConvergenceV1`/`DMonitorApplicationDiotConvergenceV1`
// and `dserver/src/api/dmonitor_convergence.rs`'s `EvaluateConvergenceResponse`
// field-for-field. This is a DIFFERENT, richer shape than
// `types/continuum.ts`'s own `ObservationEvidence`/`NodeObservation` (that
// page's backend, `continuum_view.rs`, computes its own simplified,
// node-centric projection with a derived `age_seconds` -- it is not this
// same wire contract). The shared PRIMITIVE enums genuinely are identical
// between the two and are reused from `continuum.ts`, never redefined here
// (Part I1: never duplicate status enums inconsistently).
//
// No backend convergence code was changed to produce this phase's
// Console integration -- this file exists purely to decode the EXISTING
// `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate`
// response honestly.
import type {
  DMonitorFreshness,
  DMonitorHealth,
  AuthorityCorrelation,
  DMonitorConvergence,
  ConvergenceFinding,
} from "./continuum.ts";

export type { DMonitorFreshness, DMonitorHealth, AuthorityCorrelation, DMonitorConvergence, ConvergenceFinding };

export interface DMapRefSummary {
  dmap_id: string;
  revision: number;
  digest: string;
}

export interface DMonitorFreshnessPolicy {
  max_observed_age_seconds: number;
  max_received_age_seconds: number;
  max_future_clock_skew_seconds: number;
}

// `POST .../evaluate` request body -- a DIAGNOSTIC freshness policy only;
// the server generates `evaluated_at_utc` itself, never accepted here.
export type EvaluateConvergenceRequest = DMonitorFreshnessPolicy;

export interface DMonitorHealthClaim {
  monitor_instance_id: string;
  health: DMonitorHealth;
}

// One admitted-or-excluded D-Serv observation summary. Deliberately never
// includes the observation's own raw `facts` (evidence-oriented, not a
// telemetry dump -- same discipline backend-side).
export interface DMonitorEvidenceSummary {
  observation_id: string;
  observation_digest: string;
  node_id: string;
  monitor_instance_id: string;
  sequence: number;
  observed_at_utc: string;
  received_at_utc: string;
  freshness: DMonitorFreshness;
  health: DMonitorHealth;
  authority_correlation: AuthorityCorrelation;
  observed_subject_node_id: string;
  admitted: boolean;
  exclusion_reason?: string;
}

export interface DMonitorServicePlacementConvergence {
  service_id: string;
  desired_node_id: string;
  convergence: DMonitorConvergence;
  findings: ConvergenceFinding[];
  admitted_evidence: DMonitorEvidenceSummary[];
  excluded_evidence: DMonitorEvidenceSummary[];
  observed_health_claims: DMonitorHealthClaim[];
}

export interface DMonitorUnexpectedServiceEvidence {
  service_id: string;
  admitted_evidence: DMonitorEvidenceSummary[];
  excluded_evidence: DMonitorEvidenceSummary[];
}

// The D-Serv half of the response. NOT canonical desired-state authority --
// a derived, point-in-time interpretation, recomputed fresh every call.
export interface DMonitorApplicationConvergence {
  schema: string;
  project_id: string;
  application_id: string;
  evaluated_at_utc: string;
  dmap_ref: DMapRefSummary | null;
  freshness_policy: DMonitorFreshnessPolicy;
  convergence: DMonitorConvergence;
  findings: ConvergenceFinding[];
  services: DMonitorServicePlacementConvergence[];
  // Realizations for a service_id NOT in the desired placement set at all.
  // Only an entry with non-empty admitted_evidence actually escalates
  // `convergence` to "diverged" -- stale/historical-only evidence here is
  // preserved for transparency but proves nothing CURRENT.
  unexpected_realizations: DMonitorUnexpectedServiceEvidence[];
}

// DIoT mirror -- structurally decoded (the endpoint returns it combined,
// `#[serde(flatten)]` on the dserv half), but Phase 139B's own Console
// integration (the Deploy/Reconciliation workspace) does not render it;
// DIoT already has its own complete, separate convergence surface and nothing
// here changes that.
export interface DMonitorDiotEvidenceSummary extends DMonitorEvidenceSummary {
  dmap_correlation: AuthorityCorrelation;
  dforward_correlation: AuthorityCorrelation;
  runtime_binding_correlation: AuthorityCorrelation;
  runtime_binding_current: boolean;
}
export interface DMonitorDiotConvergence {
  project_id: string;
  application_id: string;
  diot_id: string;
  diot_placement_id: string;
  desired_node_id: string;
  convergence: DMonitorConvergence;
  findings: ConvergenceFinding[];
  admitted_evidence: DMonitorDiotEvidenceSummary[];
  excluded_evidence: DMonitorDiotEvidenceSummary[];
  observed_health_claims: DMonitorHealthClaim[];
}
export interface DMonitorUnexpectedDiotEvidence {
  diot_id: string;
  diot_placement_id: string;
  admitted_evidence: DMonitorDiotEvidenceSummary[];
  excluded_evidence: DMonitorDiotEvidenceSummary[];
}
export interface DMonitorApplicationDiotConvergence {
  schema: string;
  project_id: string;
  application_id: string;
  evaluated_at_utc: string;
  dmap_ref: DMapRefSummary | null;
  freshness_policy: DMonitorFreshnessPolicy;
  convergence: DMonitorConvergence;
  findings: ConvergenceFinding[];
  placements: DMonitorDiotConvergence[];
  unexpected_realizations: DMonitorUnexpectedDiotEvidence[];
}

// `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate`
// response. The backend flattens the D-Serv half's own fields to the top
// level (`#[serde(flatten)]`) and nests the DIoT half under `diot` --
// mirrored here by extending `DMonitorApplicationConvergence` directly.
export interface EvaluateConvergenceResponse extends DMonitorApplicationConvergence {
  diot: DMonitorApplicationDiotConvergence;
}

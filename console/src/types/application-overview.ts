// Phase 141B -- canonical application-scoped operational context. Mirrors
// `dserver/src/core/operations_read_models.rs`'s `ApplicationOperationsOverview`
// field-for-field, reusing the EXISTING Phase 127/135/139B shared primitive
// types wherever the same Rust type already backs them (`SourceSection`,
// `Page`, `AlertSection`, `OperationalEvent`, `TemporalQuality`,
// `DMonitorConvergence`, `ConvergenceFinding`) -- never redefining them
// inconsistently (Part Q).
//
// This is a DERIVED, READ-ONLY, NON-ATOMIC snapshot (Part I): sections are
// independently sourced and independently timestamped; there is no single
// transactional "as of" instant beyond each section's own
// `evaluated_at_utc`/`generated_at_utc`. Readiness and convergence are
// structurally SEPARATE fields (Part B2/V) -- this type never merges them,
// and never adds a third "overall status" beyond what each field already
// says on its own.
import type { SourceSection, Page, AlertSection, OperationalEvent, TemporalQuality } from "./operations.ts";
import type { DMonitorConvergence, ConvergenceFinding } from "./continuum.ts";

// Mirrors `DMonitorReadinessV1` (`dmonitor_readiness.rs`) -- exactly two
// states, fail-closed. Never a third "unknown" readiness state (that
// uncertainty lives in `findings`, same as the canonical engine itself).
export type DMonitorReadiness = "ready" | "not_ready";
export interface DMonitorReadinessFinding {
  code: string;
  message: string;
}

export interface AcceptedPlacementView {
  project_id: string;
  application_id: string;
  dmap_id: string;
  revision: number;
  digest: string;
  acceptance_id: string | null;
  dgraph_id: string | null;
  dgraph_revision: number | null;
  dgraph_digest: string | null;
  service_nodes: Record<string, string>;
}
export interface PlannedPlacementView {
  proposal_id: string;
  created_at_utc: string;
  lifecycle: string;
  schema: string;
  dgraph_id: string;
  service_nodes: Record<string, string>;
  total_service_placements: number;
  placements_truncated: boolean;
}
export interface DesiredRuntimeView {
  service_id: string;
  runtime: string;
  desired_state: string;
}

// Mirrors `DMonitorSubjectV1` (the same closed, 8-variant tagged union
// Phase 140A's own `types/dmonitor-history.ts` already defines) --
// re-exported from there rather than redefined a third time.
export type { DMonitorSubject } from "./dmonitor-history.ts";
import type { DMonitorSubject, DMonitorHealth } from "./dmonitor-history.ts";

export interface ObservedRuntimeView {
  observation_id: string;
  subject: DMonitorSubject;
  observed_at_utc: string;
  received_at_utc: string;
  reported_health: DMonitorHealth;
}

export interface ApplicationConvergenceView {
  evaluated_at_utc: string;
  convergence: DMonitorConvergence;
  desired_service_count: number;
  unexpected_realization_count: number;
  findings: Page<ConvergenceFinding>;
}
export interface ApplicationReadinessView {
  evaluated_at_utc: string;
  readiness: DMonitorReadiness;
  findings: Page<DMonitorReadinessFinding>;
}

// Phase 141B's own addition (Part C) -- evidence only, never a historical
// convergence/readiness verdict. `null` fields mean genuinely zero
// matching evidence, never "unavailable" (that is the enclosing
// `SourceSection` itself).
export interface RecentHistoryPointerView {
  recent_observation_count: number;
  latest_received_at_utc: string | null;
  latest_observation_id: string | null;
}

// `GET /api/v1/operations/projects/:project_id/applications/:application_id`
export interface ApplicationOperationsOverview {
  generated_at_utc: string;
  project_id: string;
  application_id: string;
  canonical_application_registry: SourceSection<string>;
  accepted: SourceSection<AcceptedPlacementView | null>;
  planned: SourceSection<Page<PlannedPlacementView>>;
  desired_runtime: SourceSection<Page<DesiredRuntimeView>>;
  observed_runtime: SourceSection<Page<ObservedRuntimeView>>;
  temporal_quality: SourceSection<Page<TemporalQuality>>;
  convergence: SourceSection<ApplicationConvergenceView>;
  readiness: SourceSection<ApplicationReadinessView>;
  reconciliation_execution: SourceSection<string>;
  alerts: SourceSection<AlertSection>;
  recent_events: SourceSection<Page<OperationalEvent>>;
  recent_history: SourceSection<RecentHistoryPointerView>;
}

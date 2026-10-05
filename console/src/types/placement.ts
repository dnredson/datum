// Phase 136 -- Placement Workspace response types. Mirrors
// `dserver/src/api/ddeploy.rs` field-for-field. Three states are kept
// textually and structurally separate here exactly as the backend keeps
// them separate: a CANDIDATE (`CandidateResponse`/`ValidateResponse`,
// never persisted), a PROPOSED object (`ProposalResponse`, persisted
// but not authoritative), and ACCEPTED authority (`ActiveResponse`,
// read-only here -- Phase 136 never calls `.../accept`).

export interface ContractRef {
  id: string;
  revision: number;
  digest: string;
}

export interface PlacementRef {
  service_id: string;
  node_id: string;
}

export interface PlacementDecisionRecord {
  service_id: string;
  node_id: string;
  required_dnode_abi: string;
  eligible_node_count: number;
  node_residual_cpu_cores_before: number;
  node_residual_memory_bytes_before: number;
  node_residual_cpu_cores_after: number;
  node_residual_memory_bytes_after: number;
}

export interface PlacementProvenance {
  planner_version: string;
  policy_name: string;
  policy_version: string;
  dgraph_id: string;
  dgraph_revision: number;
  dgraph_digest: string;
  dcontinuum_id: string;
  dcontinuum_revision: number;
  dcontinuum_digest: string;
  decisions: PlacementDecisionRecord[];
}

// `POST /api/v1/ddeploy/candidate` response. NEVER persisted -- carries no
// proposal_id. See documentation/reference/phase136-placement-ui.md.
export interface CandidateResponse {
  schema: "datum.placement-candidate/1";
  project_id: string;
  application_id: string;
  dgraph_ref: ContractRef;
  dcontinuum_ref: ContractRef;
  expected_active_dmap_revision: number | null;
  placements: PlacementRef[];
  planner_provenance: PlacementProvenance;
}

export interface ValidationFinding {
  code: string;
  message: string;
}

// `POST /api/v1/ddeploy/proposals/validate` response. Also never
// persisted. `capacity_check_is_momentary` is always present, regardless
// of `valid` -- see that field's own backend doc comment.
export interface ValidateResponse {
  valid: boolean;
  findings: ValidationFinding[];
  dgraph_ref: ContractRef;
  dcontinuum_ref: ContractRef;
  candidate_placements: PlacementRef[];
  capacity_check_is_momentary: string;
}

export type ProposalSource = "operator" | "deterministic" | "ai";
export type ProposalState = "pending" | "accepted";

export interface DDeployProposal {
  schema: string;
  proposal_id: string;
  project_id: string;
  application_id: string;
  source: ProposalSource;
  created_at_utc: string;
  expected_active_dmap_revision: number | null;
  dgraph_ref: ContractRef;
  dcontinuum_ref: ContractRef;
  placements: PlacementRef[];
}

// `POST /api/v1/ddeploy/proposals` and `GET /api/v1/ddeploy/proposals/:id`
// response. A Pending proposal is NOT active authority -- Phase 137 owns
// the only path (`.../accept`) that can make it so.
export interface ProposalResponse {
  proposal: DDeployProposal;
  proposal_digest: string;
  state: ProposalState;
}

export interface CanonicalDMap {
  schema: string;
  project_id: string;
  application_id: string;
  dmap_id: string;
  revision: number;
  dgraph_ref: ContractRef;
  dcontinuum_ref: ContractRef;
  placements: PlacementRef[];
}

export interface ActiveDeploymentSummary {
  project_id: string;
  application_id: string;
  active_dmap: CanonicalDMap;
  dmap_digest: string;
  proposal_id: string;
  acceptance_id: string;
  accepted_at_utc: string;
  operational_dgraph_id: string;
  operational_revision: number;
}

// `GET /api/v1/ddeploy/active` response. Read-only accepted authority --
// the CURRENT state a candidate is diffed against.
export interface ActiveResponse {
  project_id: string;
  deployments: ActiveDeploymentSummary[];
}

// Phase 137 -- the governed authority-transition response types. Mirrors
// `dserver/src/core/ddeploy.rs::DDeployAcceptanceV1`/`AcceptOutcome` and
// `dserver/src/api/ddeploy.rs::AcceptResponse` field-for-field.

export interface OperationalProjectionRef {
  operational_dgraph_id: string;
  revision: number;
}

// `accepted_by` is a CALLER-SUPPLIED AUDIT LABEL, never authenticated
// identity -- DServer has no application-layer authentication (ADR-0013
// §6). Never render this as "authenticated user" or similar.
export interface DDeployAcceptance {
  schema: string;
  acceptance_id: string;
  proposal_id: string;
  proposal_digest: string;
  project_id: string;
  application_id: string;
  accepted_at_utc: string;
  accepted_by: string;
  materialized_dmap_ref: ContractRef;
  operational_projection_ref: OperationalProjectionRef;
}

// `POST /api/v1/ddeploy/proposals/:id/accept` response. `materialized_dmap`
// is the D-Map THIS call's acceptance produced/replayed -- NOT necessarily
// the CURRENT active D-Map (see `currently_active`). `idempotent_replay`
// is true when the proposal was already Accepted; nothing was mutated on
// that path.
export interface AcceptResponse {
  acceptance: DDeployAcceptance;
  materialized_dmap: CanonicalDMap;
  operational_dgraph_id: string;
  operational_revision: number;
  idempotent_replay: boolean;
  // `true` iff `materialized_dmap` is still the CURRENT active D-Map for
  // this application right now. `GET /api/v1/ddeploy/active` remains the
  // authoritative source for "what is active right now" independent of
  // this field -- Phase 137 must re-fetch it after every acceptance,
  // never treat this response as a lasting snapshot.
  currently_active: boolean;
}

// The backend's flat ddeploy error envelope: `{"error": "<code>",
// "message"?, "findings"?}`. Exact finding codes (e.g.
// `project_node_cpu_capacity_exceeded`, `stale_active_dmap_revision`,
// `stale_authoritative_dcontinuum`, `proposal_digest_mismatch`,
// `proposal_not_found`) are preserved verbatim -- never paraphrased away.
export interface AcceptErrorFinding {
  code: string;
  message: string;
}

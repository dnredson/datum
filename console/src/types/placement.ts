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

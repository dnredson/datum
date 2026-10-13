// Phase 135 -- Continuum View response types. Mirrors
// `dserver/src/core/continuum_view.rs::ContinuumViewV1` field-for-field.
// Physical structure, accepted placement, and DMonitor observation are kept
// as three separate sections here exactly as the backend keeps them
// separate -- never collapse them into one status value in the UI layer.

export interface ContinuumNodePlatform {
  os: string;
  architecture: string;
}

export interface ContinuumNodeCapacity {
  cpu_cores: number;
  memory_bytes: number;
}

// DECLARED total static capacity only -- never "available"/"free"/
// "allocated". No current canonical source computes those; do not invent
// them in the UI either.
export interface ContinuumNodeStructure {
  platform: ContinuumNodePlatform;
  capacity: ContinuumNodeCapacity;
  dnode_abi_versions: string[];
}

export interface ObservedCpuResources {
  logical_cores: number;
  usage_percent?: number;
}

export interface ObservedMemoryResources {
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
}

export interface ProjectResourceObservation {
  resource_id: string;
  observed_at_utc: string;
  last_seen_utc: string;
  age_seconds: number;
  cpu: ObservedCpuResources;
  memory: ObservedMemoryResources;
  swap: ObservedMemoryResources;
}

export interface RuntimeResourceUsage {
  cpu_percent: number;
  memory_used_bytes: number;
  memory_limit_bytes: number;
  pids: number;
}

// PLACEMENT AUTHORITY only -- never ownership. See
// `documentation/reference/phase135-continuum-view.md`.
export interface AcceptedPlacementRef {
  project_id: string;
  application_id: string;
  service_id: string;
}

export type DMonitorFreshness = "fresh" | "stale" | "clock_skew";
export type DMonitorHealth = "unknown" | "healthy" | "degraded" | "unhealthy";
export type AuthorityCorrelation =
  | "exact_match"
  | "ref_missing"
  | "mismatch"
  | "not_applicable";
export type DMonitorConvergence = "converged" | "diverged" | "unknown";

export interface ObservationEvidence {
  monitor_instance_id: string;
  runtime_usage?: RuntimeResourceUsage;
  freshness: DMonitorFreshness;
  health: DMonitorHealth;
  authority_correlation: AuthorityCorrelation;
  observed_at_utc: string;
  received_at_utc: string;
  age_seconds: number;
}

export interface ExcludedObservationEvidence extends ObservationEvidence {
  exclusion_reason: string | null;
}

export interface ConvergenceFinding {
  code: string;
  message: string;
}

// DMonitor's own derived verdict for ONE accepted placement -- never a
// single collapsed per-node status. A node may have zero, one, or several
// of these (one per accepted placement referencing it).
export interface NodeObservation {
  project_id: string;
  application_id: string;
  service_id: string;
  placement_convergence: DMonitorConvergence;
  convergence_findings: ConvergenceFinding[];
  admitted_evidence: ObservationEvidence[];
  admitted_evidence_truncated: boolean;
  excluded_evidence: ExcludedObservationEvidence[];
  excluded_evidence_truncated: boolean;
}

export interface ContinuumNode {
  node_id: string;
  structure: ContinuumNodeStructure;
  resource_observation?: ProjectResourceObservation;
  accepted_placements: AcceptedPlacementRef[];
  placements_truncated: boolean;
  observations: NodeObservation[];
  observations_truncated: boolean;
}

export interface ContinuumLink {
  link_id: string;
  from_node_id: string;
  to_node_id: string;
}

// "project X's accepted workload currently references this node" -- NEVER
// "this node belongs to project X". See the module doc comment on the
// backend and the Phase 135 reference documentation.
export interface ProjectOverlay {
  project_id: string;
  node_ids_in_use: string[];
}

export interface ContinuumView {
  schema: "datum.continuum-view/1";
  generated_at_utc: string;
  dnode_registry_revision: number;
  nodes: ContinuumNode[];
  links: ContinuumLink[];
  project_overlay?: ProjectOverlay;
  truncated: boolean;
}

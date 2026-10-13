import type { ProjectGraph } from "./graph.ts";
import type { SourceSection, TemporalQuality } from "./operations.ts";
export interface RuntimeResourceUsage {
  cpu_percent: number;
  memory_used_bytes: number;
  memory_limit_bytes: number;
  pids: number;
}
export interface RuntimeEvidence {
  id: string;
  project_id: string;
  application_id: string;
  service_id: string;
  node_id: string;
  reporting_node_id: string;
  monitor_instance_id: string;
  observation_id: string;
  observation_digest: string;
  collector: string;
  observed_at_utc: string;
  received_at_utc: string;
  observed_health_claim: "unknown" | "healthy" | "degraded" | "unhealthy";
  resource_usage?: RuntimeResourceUsage;
  dmap_ref: { id: string; revision: number; digest: string } | null;
  logical_node_id: string | null;
}
export interface ProjectRuntimeGraph {
  project_id: string;
  logical: ProjectGraph;
  runtime_source_kind: "admitted_dserv_realization_evidence";
  runtime: SourceSection<{
    completeness: "complete" | "partial";
    total_evidence_count: number;
    included_evidence_count: number;
    evidence: RuntimeEvidence[];
  }>;
  temporal_quality: (TemporalQuality & {
    observation_id: string;
    application_id: string;
    source_observed_at_utc: string;
    server_received_at_utc: string;
  })[];
}

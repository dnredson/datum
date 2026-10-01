import type { SourceSection } from "./operations.ts";
export interface GraphApplication {
  application_id: string;
  dgraph_id: string;
  revision: number;
  digest: string;
  acceptance_id: string | null;
}
export interface GraphNode {
  id: string;
  kind: "application" | "dserv";
  canonical_id: string;
  application_id: string;
  parent_id: string | null;
  execution_abi: string | null;
  cpu_cores: number | null;
  memory_bytes: string | null;
}
export interface GraphEdge {
  id: string;
  kind: "dcall";
  canonical_id: string;
  application_id: string;
  source: string;
  target: string;
}
export interface GraphContent {
  coverage: {
    scope: "active_accepted_applications_only";
    completeness: "complete" | "partial";
    active_application_count: number;
    included_application_count: number;
  };
  applications: GraphApplication[];
  nodes: GraphNode[];
  edges: GraphEdge[];
}
export interface ProjectGraph {
  project_id: string;
  generated_at_utc: string;
  source_kind: "accepted_application_dgraph_snapshots";
  graph: SourceSection<GraphContent>;
}

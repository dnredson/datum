// Deliberate, small consumption subset of Phase 127 DTOs. Rust is canonical.
// Extra backend fields are not reconstructed or interpreted by the Console.
export interface SourceIssue {
  kind: string;
  source: string;
  code: string;
  safe_message: string;
  scope: { project_id: string | null; application_id: string | null } | null;
}
export type SourceSection<T> =
  | { availability: "available"; value: T }
  | { availability: "not_configured" }
  | { availability: "unavailable" | "corrupt"; value: SourceIssue }
  | { availability: "indeterminate"; value: string };
export interface Page<T> {
  items: T[];
  limit: number;
  has_more: boolean;
}
export interface Coverage {
  completeness: "complete" | "partial" | "indeterminate";
  source_coverage: Record<string, { state: string; issue?: SourceIssue }>;
}
export interface DiscoveryPage<T> {
  page: Page<T>;
  coverage: Coverage;
}
export interface KnownApplication {
  project_id: string;
  application_id: string;
  provenance: string[];
}
export interface AlertCounts {
  open: number;
  acknowledged: number;
  in_progress: number;
  resolved_pending_close: number;
  historical_closed: number;
  critical_attention: number;
  warning_attention: number;
}
export interface AlertSection {
  counts: AlertCounts;
}
export interface ProjectAttention {
  project_id: string;
  active_alert_count: number;
  critical_count: number;
  warning_count: number;
}
export type Freshness =
  "fresh" | "stale" | "missing_evidence" | "indeterminate";
export type SourceTimeQuality =
  | "aligned_or_no_discrepancy"
  | "source_ahead"
  | "behind_or_delayed"
  | "evaluation_ordering_anomaly"
  | "missing_evidence";
export interface TemporalQuality {
  node_id: string;
  communication_freshness: Freshness;
  observation_freshness: Freshness;
  source_time_quality: SourceTimeQuality;
}
export interface GlobalOverview {
  generated_at_utc: string;
  managed_operational_project_count: SourceSection<number>;
  canonical_application_count: SourceSection<number>;
  structural_node_count: SourceSection<number>;
  desired_service_assignment_count: SourceSection<number>;
  known_projects: DiscoveryPage<string>;
  known_applications: DiscoveryPage<KnownApplication>;
  alerts: SourceSection<AlertSection>;
  recent_event_count: SourceSection<number>;
  projects_with_attention: SourceSection<Page<ProjectAttention>>;
  temporal_quality: SourceSection<Page<TemporalQuality>>;
  scheduler: SourceSection<{ configured: boolean; driver_activity: string }>;
  source_issues: SourceIssue[];
  consistency: string;
}
export interface ProjectOverview {
  generated_at_utc: string;
  project_id: string;
  operational_graph: SourceSection<{
    dgraph_id: string;
    revision: number;
    node_count: number;
    desired_service_assignment_count: number;
  }>;
  canonical_application_count: SourceSection<number>;
  known_applications: DiscoveryPage<KnownApplication>;
  alerts: SourceSection<AlertSection>;
  recent_events: SourceSection<Page<{ event_id: string }>>;
  accepted_deployments: SourceSection<
    Page<{ application_id: string; dmap_id: string; revision: number }>
  >;
  temporal_quality: SourceSection<Page<TemporalQuality>>;
}

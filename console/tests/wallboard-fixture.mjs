// Test responses only. Never imported by the production application.
const at = "2026-09-30T15:00:00Z";
const available = (value) => ({ availability: "available", value });
const page = (items) => ({ items, limit: 100, has_more: false });
const subject = {
  kind: "dserv",
  project_id: "alpha",
  application_id: "mqtt",
  service_id: "mqtt-service",
  node_id: "fog-01",
};
export function eventFixture(index = 0) {
  const occurred = `2026-09-30T14:${(59 - index).toString().padStart(2, "0")}:00Z`;
  const age = (59 - index) * 60;
  return {
    event_id: `event-${index.toString().padStart(3, "0")}`,
    project_id: "alpha",
    application_id: "mqtt",
    category: "monitoring",
    kind: "observation_stale",
    source: "active_monitor",
    severity: "warning",
    subject: { ...subject },
    occurred_at_utc: occurred,
    admitted_at_utc: at,
    classification: "monitoring",
    temporal_context: {
      availability: "available",
      value: {
        server_evaluated_at_utc: occurred,
        evidence: [
          {
            observation_id: "obs-1",
            source_observed_at_utc: "2026-09-30T14:00:00Z",
            server_received_at_utc: "2026-09-30T14:00:00Z",
            observed_receipt_delta_seconds: 0,
            source_age_seconds: age,
            receipt_age_seconds: age,
          },
        ],
      },
    },
  };
}
export function alertFixture(status = "open", id = "alert-open") {
  const active = ["open", "acknowledged", "in_progress"].includes(status);
  const conditionSubject = { ...subject, service_id: `mqtt-service-${id}` };
  const latest = eventFixture();
  latest.event_id = `condition-${id}`;
  latest.subject = conditionSubject;
  latest.severity = "critical";
  return {
    alert_id: id,
    category: "monitoring",
    project_id: "alpha",
    application_id: "mqtt",
    kind: "observation_stale",
    severity: "critical",
    lifecycle_status: status,
    group: active
      ? "operationally_active"
      : status === "resolved"
        ? "pending_closure"
        : "historical_closed",
    subject: conditionSubject,
    is_operationally_active: active,
    first_observed_at_utc: "2026-09-30T14:00:00Z",
    last_observed_at_utc: "2026-09-30T14:59:00Z",
    condition_confirmation_count: 120,
    condition_duration_seconds: 3600,
    latest_condition_event: latest,
  };
}
export function wallboardFixture() {
  const coverage = {
    completeness: "partial",
    source_coverage: {
      events: { state: "fully_scanned" },
      alerts: { state: "fully_scanned" },
      dmonitor: { state: "not_configured" },
    },
  };
  const temporal = (
    node,
    communication,
    observation,
    source,
    received,
    observed,
  ) => ({
    project_id: "alpha",
    application_id: null,
    node_id: node,
    subject: {
      kind: "other",
      project_id: "alpha",
      entity_kind: "dmonitor_stream",
      entity_id: `alpha/${node}/monitor`,
    },
    server_received_at_utc: received,
    source_observed_at_utc: observed,
    evaluated_at_utc: at,
    received_age_seconds: 5,
    observed_age_seconds: 3600,
    observed_receipt_delta_seconds: -3595,
    communication_freshness: communication,
    observation_freshness: observation,
    source_time_quality: source,
  });
  const events = Array.from({ length: 20 }, (_, i) => eventFixture(i));
  events[0].classification = "recovery";
  events[0].kind = "observation_stale_cleared";
  const recovery = events[0].temporal_context.value.evidence[0];
  recovery.source_observed_at_utc = events[0].occurred_at_utc;
  recovery.server_received_at_utc = events[0].occurred_at_utc;
  recovery.source_age_seconds = 0;
  recovery.receipt_age_seconds = 0;
  events[1].classification = "control_plane";
  events[1].kind = "dmap_accepted";
  events[1].source = "ddeploy";
  events[1].temporal_context = { availability: "absent" };
  return {
    generated_at_utc: at,
    managed_operational_project_count: available(2),
    canonical_application_count: { availability: "not_configured" },
    structural_node_count: available(3),
    desired_service_assignment_count: available(6),
    known_projects: {
      page: page(["alpha", "beta"]),
      coverage: structuredClone(coverage),
    },
    known_applications: {
      page: page([
        {
          project_id: "alpha",
          application_id: "mqtt",
          provenance: ["event", "alert"],
        },
      ]),
      coverage: structuredClone(coverage),
    },
    alerts: available({
      counts: {
        open: 1,
        acknowledged: 1,
        in_progress: 1,
        resolved_pending_close: 1,
        historical_closed: 1,
        critical_attention: 3,
        warning_attention: 0,
      },
      attention_ordered: page([
        alertFixture("open"),
        alertFixture("acknowledged", "alert-ack"),
        alertFixture("in_progress", "alert-progress"),
        alertFixture("resolved", "alert-resolved"),
        alertFixture("closed", "alert-closed"),
      ]),
    }),
    recent_event_count: available(20),
    recent_events: available(page(events)),
    projects_with_attention: available(
      page([
        {
          project_id: "alpha",
          active_alert_count: 3,
          critical_count: 3,
          warning_count: 0,
        },
      ]),
    ),
    temporal_quality: available(
      page([
        temporal(
          "fog-01",
          "fresh",
          "stale",
          "behind_or_delayed",
          "2026-09-30T14:59:55Z",
          "2026-09-30T14:00:00Z",
        ),
        {
          ...temporal(
            "rpi-01",
            "stale",
            "stale",
            "aligned_or_no_discrepancy",
            "2026-09-30T14:00:00Z",
            "2026-09-30T14:00:00Z",
          ),
          received_age_seconds: 3600,
          observed_receipt_delta_seconds: 0,
        },
        {
          ...temporal(
            "cloud-01",
            "fresh",
            "fresh",
            "source_ahead",
            "2026-09-30T14:59:55Z",
            "2026-09-30T16:00:00Z",
          ),
          observed_age_seconds: -3600,
          observed_receipt_delta_seconds: 3605,
        },
      ]),
    ),
    scheduler: available({
      configured: false,
      driver_activity: "unwired",
      lifecycle: null,
      dropped_output_full: null,
      dropped_output_closed: null,
      completed_count: null,
    }),
    source_issues: [],
    consistency: "Independent source snapshots, not globally atomic",
  };
}

// Phase 141B deterministic ApplicationOperationsOverview fixtures.
export function available(value) {
  return { availability: "available", value };
}
export const notConfigured = { availability: "not_configured" };
export function unavailable(message = "snapshot acquisition failed") {
  return {
    availability: "unavailable",
    value: { kind: "source_unavailable", source: "dmonitor", code: "source_unavailable", safe_message: message, scope: null },
  };
}

export function acceptedFixture(overrides = {}) {
  return {
    project_id: "project-a",
    application_id: "app-1",
    dmap_id: "dmap:1",
    revision: 3,
    digest: `sha256:${"a".repeat(64)}`,
    acceptance_id: "acceptance-1",
    dgraph_id: "graph-1",
    dgraph_revision: 1,
    dgraph_digest: `sha256:${"b".repeat(64)}`,
    service_nodes: { "service-a": "fog-01" },
    ...overrides,
  };
}

export function observedRuntimeFixture(overrides = {}) {
  return {
    observation_id: "obs-1",
    subject: {
      kind: "dserv_realization",
      project_id: "project-a",
      application_id: "app-1",
      service_id: "service-a",
      node_id: "fog-01",
      dmap_ref: { id: "dmap:1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
    },
    observed_at_utc: "2026-10-06T12:00:00+00:00",
    received_at_utc: "2026-10-06T12:00:01+00:00",
    reported_health: "healthy",
    ...overrides,
  };
}

export function convergenceFixture(overrides = {}) {
  return {
    evaluated_at_utc: "2026-10-06T12:00:05Z",
    convergence: "converged",
    desired_service_count: 1,
    unexpected_realization_count: 0,
    findings: { items: [], limit: 100, has_more: false },
    ...overrides,
  };
}

export function readinessFixture(overrides = {}) {
  return {
    evaluated_at_utc: "2026-10-06T12:00:05Z",
    readiness: "ready",
    findings: { items: [], limit: 100, has_more: false },
    ...overrides,
  };
}

export function alertSectionFixture(overrides = {}) {
  return {
    counts: { open: 0, acknowledged: 0, in_progress: 0, resolved_pending_close: 0, historical_closed: 0, critical_attention: 0, warning_attention: 0 },
    attention_ordered: { items: [], limit: 25, has_more: false },
    ...overrides,
  };
}

export function operationalEventFixture(overrides = {}) {
  return {
    event_id: "event-1",
    project_id: "project-a",
    application_id: "app-1",
    category: "observation",
    kind: "observation_stale",
    source: "active_monitor",
    severity: "info",
    subject: { kind: "dserv", project_id: "project-a", application_id: "app-1", service_id: "service-a", node_id: "fog-01" },
    occurred_at_utc: "2026-10-06T11:00:00+00:00",
    admitted_at_utc: "2026-10-06T11:00:01+00:00",
    classification: "monitoring",
    temporal_context: { availability: "absent" },
    ...overrides,
  };
}

export function recentHistoryFixture(overrides = {}) {
  return {
    recent_observation_count: 1,
    latest_received_at_utc: "2026-10-06T12:00:01+00:00",
    latest_observation_id: "obs-1",
    ...overrides,
  };
}

export function applicationOverviewFixture(overrides = {}) {
  return {
    generated_at_utc: "2026-10-06T12:00:05Z",
    project_id: "project-a",
    application_id: "app-1",
    canonical_application_registry: notConfigured,
    accepted: available(acceptedFixture()),
    planned: available({ items: [], limit: 100, has_more: false }),
    desired_runtime: available({ items: [{ service_id: "service-a", runtime: "container", desired_state: "running" }], limit: 100, has_more: false }),
    observed_runtime: available({ items: [observedRuntimeFixture()], limit: 100, has_more: false }),
    temporal_quality: available({ items: [], limit: 100, has_more: false }),
    convergence: available(convergenceFixture()),
    readiness: available(readinessFixture()),
    reconciliation_execution: notConfigured,
    alerts: available(alertSectionFixture()),
    recent_events: available({ items: [], limit: 25, has_more: false }),
    recent_history: available(recentHistoryFixture()),
    ...overrides,
  };
}

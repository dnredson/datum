// Phase 139B deterministic DMonitor convergence fixtures.
export function evidenceFixture(overrides = {}) {
  return {
    observation_id: "dmonitor-obs:fog-01:1",
    observation_digest: `sha256:${"a".repeat(64)}`,
    node_id: "fog-01",
    monitor_instance_id: "dmonitor-instance:fog-01:fixed",
    sequence: 1,
    observed_at_utc: "2026-10-05T12:00:00Z",
    received_at_utc: "2026-10-05T12:00:01Z",
    freshness: "fresh",
    health: "healthy",
    authority_correlation: "exact_match",
    observed_subject_node_id: "fog-01",
    admitted: true,
    ...overrides,
  };
}

export function servicePlacementFixture(serviceId, overrides = {}) {
  return {
    service_id: serviceId,
    desired_node_id: "fog-01",
    convergence: "converged",
    findings: [{ code: "exact_match", message: `${serviceId} is converged` }],
    admitted_evidence: [evidenceFixture()],
    excluded_evidence: [],
    observed_health_claims: [{ monitor_instance_id: "dmonitor-instance:fog-01:fixed", health: "healthy" }],
    ...overrides,
  };
}

export function applicationConvergenceFixture(applicationId = "app-1", overrides = {}) {
  return {
    schema: "datum.dmonitor-convergence/1",
    project_id: "project-a",
    application_id: applicationId,
    evaluated_at_utc: "2026-10-05T12:00:05Z",
    dmap_ref: { dmap_id: `dmap:${applicationId}`, revision: 3, digest: `sha256:${"b".repeat(64)}` },
    freshness_policy: { max_observed_age_seconds: 90, max_received_age_seconds: 90, max_future_clock_skew_seconds: 10 },
    convergence: "converged",
    findings: [],
    services: [servicePlacementFixture("service-a")],
    unexpected_realizations: [],
    ...overrides,
  };
}

export function diotConvergenceFixture(applicationId = "app-1") {
  return {
    schema: "datum.dmonitor-diot-convergence/1",
    project_id: "project-a",
    application_id: applicationId,
    evaluated_at_utc: "2026-10-05T12:00:05Z",
    dmap_ref: null,
    freshness_policy: { max_observed_age_seconds: 90, max_received_age_seconds: 90, max_future_clock_skew_seconds: 10 },
    convergence: "unknown",
    findings: [{ code: "dmap_v1_no_diot_support", message: "no DIoT placements" }],
    placements: [],
    unexpected_realizations: [],
  };
}

export function evaluateConvergenceResponseFixture(applicationId = "app-1", overrides = {}) {
  return {
    ...applicationConvergenceFixture(applicationId, overrides),
    diot: diotConvergenceFixture(applicationId),
  };
}

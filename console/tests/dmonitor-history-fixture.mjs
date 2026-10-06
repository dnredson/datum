// Phase 140A deterministic DMonitor history fixtures.
export function historicalObservationFixture(overrides = {}) {
  return {
    observation_id: "dmonitor-obs:fog-01:1",
    node_id: "fog-01",
    monitor_instance_id: "dmonitor-instance:fog-01:fixed",
    sequence: 1,
    collector: "native_observe",
    subject: {
      kind: "dserv_realization",
      project_id: "project-a",
      application_id: "app-1",
      service_id: "service-a",
      node_id: "fog-01",
      dmap_ref: { id: "dmap:1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
    },
    health: "healthy",
    observed_at_utc: "2026-10-05T12:00:00+00:00",
    received_at_utc: "2026-10-05T12:00:01+00:00",
    freshness: "stale",
    facts: {},
    ...overrides,
  };
}

export function historyQueryResponseFixture(overrides = {}) {
  return {
    schema: "datum.dmonitor-history/1",
    project_id: "project-a",
    start_utc: "2026-10-04T12:00:00+00:00",
    end_utc: "2026-10-05T12:00:00+00:00",
    queried_at_utc: "2026-10-05T12:00:05+00:00",
    filters: { application_id: null, node_id: null, service_id: null },
    limit: 100,
    count: 1,
    truncated: false,
    observations: [historicalObservationFixture()],
    ...overrides,
  };
}

// Phase 135 deterministic Continuum View fixture. Mirrors the manual visual
// review scenario required by the Phase 135 brief: node-a shared by two
// projects with fresh/healthy observation, node-b with one placement and a
// stale/divergent-looking observation, node-c with neither placement nor
// observation at all.
export function continuumFixture() {
  return {
    schema: "datum.continuum-view/1",
    generated_at_utc: "2026-10-04T12:00:00Z",
    dnode_registry_revision: 3,
    nodes: [
      {
        node_id: "node-a",
        structure: {
          platform: { os: "linux", architecture: "amd64" },
          capacity: { cpu_cores: 8, memory_bytes: 17179869184 },
          dnode_abi_versions: ["datum-dnode/0"],
        },
        accepted_placements: [
          { project_id: "project-a", application_id: "app-1", service_id: "service-foo" },
          { project_id: "project-b", application_id: "app-9", service_id: "service-bar" },
        ],
        placements_truncated: false,
        observations: [
          {
            project_id: "project-a",
            application_id: "app-1",
            service_id: "service-foo",
            placement_convergence: "converged",
            convergence_findings: [],
            admitted_evidence: [
              {
                monitor_instance_id: "monitor-1",
                freshness: "fresh",
                health: "healthy",
                authority_correlation: "exact_match",
                observed_at_utc: "2026-10-04T11:59:30Z",
                received_at_utc: "2026-10-04T11:59:31Z",
                age_seconds: 30,
              },
            ],
            admitted_evidence_truncated: false,
            excluded_evidence: [],
            excluded_evidence_truncated: false,
          },
          {
            project_id: "project-b",
            application_id: "app-9",
            service_id: "service-bar",
            placement_convergence: "converged",
            convergence_findings: [],
            admitted_evidence: [
              {
                monitor_instance_id: "monitor-2",
                freshness: "fresh",
                health: "healthy",
                authority_correlation: "exact_match",
                observed_at_utc: "2026-10-04T11:59:40Z",
                received_at_utc: "2026-10-04T11:59:41Z",
                age_seconds: 20,
              },
            ],
            admitted_evidence_truncated: false,
            excluded_evidence: [],
            excluded_evidence_truncated: false,
          },
        ],
        observations_truncated: false,
      },
      {
        node_id: "node-b",
        structure: {
          platform: { os: "linux", architecture: "arm64" },
          capacity: { cpu_cores: 4, memory_bytes: 4294967296 },
          dnode_abi_versions: ["datum-dnode/0"],
        },
        accepted_placements: [
          { project_id: "project-a", application_id: "app-2", service_id: "service-baz" },
        ],
        placements_truncated: false,
        observations: [
          {
            project_id: "project-a",
            application_id: "app-2",
            service_id: "service-baz",
            placement_convergence: "unknown",
            convergence_findings: [
              { code: "stale_evidence", message: "only stale evidence exists for this placement" },
            ],
            admitted_evidence: [],
            admitted_evidence_truncated: false,
            excluded_evidence: [
              {
                monitor_instance_id: "monitor-3",
                freshness: "stale",
                health: "healthy",
                authority_correlation: "exact_match",
                observed_at_utc: "2026-10-04T10:00:00Z",
                received_at_utc: "2026-10-04T10:00:01Z",
                age_seconds: 7200,
                exclusion_reason: "stale_evidence",
              },
            ],
            excluded_evidence_truncated: false,
          },
        ],
        observations_truncated: false,
      },
      {
        node_id: "node-c",
        structure: {
          platform: { os: "linux", architecture: "arm64" },
          capacity: { cpu_cores: 1, memory_bytes: 536870912 },
          dnode_abi_versions: [],
        },
        accepted_placements: [],
        placements_truncated: false,
        observations: [],
        observations_truncated: false,
      },
    ],
    links: [],
    truncated: false,
  };
}
export function continuumProjectOverlayFixture(projectId = "project-a") {
  return {
    ...continuumFixture(),
    project_overlay: {
      project_id: projectId,
      node_ids_in_use: projectId === "project-a" ? ["node-a", "node-b"] : ["node-a"],
    },
  };
}

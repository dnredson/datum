// Phase 136 deterministic Placement Workspace fixtures. node-a/node-b are
// plain declared D-Nodes (reused shape from continuum fixtures, kept
// independent here so placement tests never depend on continuum's own
// fixture semantics). service-a is currently accepted on node-a; the
// candidate moves it to node-b and leaves service-b newly placed on
// node-a (it had no prior accepted placement).
export function placementContinuumNodes() {
  return [
    {
      node_id: "node-a",
      structure: {
        platform: { os: "linux", architecture: "amd64" },
        capacity: { cpu_cores: 8, memory_bytes: 17179869184 },
        dnode_abi_versions: ["datum-dnode/0"],
      },
      accepted_placements: [{ project_id: "project-a", application_id: "app-1", service_id: "service-a" }],
      placements_truncated: false,
      observations: [],
      observations_truncated: false,
    },
    {
      node_id: "node-b",
      structure: {
        platform: { os: "linux", architecture: "arm64" },
        capacity: { cpu_cores: 4, memory_bytes: 4294967296 },
        dnode_abi_versions: ["datum-dnode/0"],
      },
      accepted_placements: [],
      placements_truncated: false,
      observations: [],
      observations_truncated: false,
    },
  ];
}

// The accepted D-Graph `app-1` is planned/validated/proposed against
// throughout this fixture family -- service-a (currently on node-a) and
// service-b (never placed yet). Deliberately the same application_id/
// service_ids every other fixture here uses, so the visual fixture server
// and these unit fixtures stay internally consistent.
export function placementDGraphFixture() {
  return {
    schema: "datum.dgraph/1",
    application_id: "app-1",
    dgraph_id: "dgraph:app-1",
    revision: 3,
    services: [
      {
        service_id: "service-a",
        execution_requirement: { dnode_abi: "datum-dnode/0" },
        resource_requirement: { cpu_cores: 2, memory_bytes: 1073741824 },
      },
      {
        service_id: "service-b",
        execution_requirement: { dnode_abi: "datum-dnode/0" },
        resource_requirement: { cpu_cores: 1, memory_bytes: 536870912 },
      },
    ],
    calls: [],
  };
}

export function activeResponseFixture(projectId = "project-a") {
  return {
    project_id: projectId,
    deployments: [
      {
        project_id: projectId,
        application_id: "app-1",
        active_dmap: {
          schema: "datum.dmap/1",
          project_id: projectId,
          application_id: "app-1",
          dmap_id: "dmap:app-1",
          revision: 3,
          dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
          dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: `sha256:${"b".repeat(64)}` },
          placements: [{ service_id: "service-a", node_id: "node-a" }],
        },
        dmap_digest: `sha256:${"c".repeat(64)}`,
        proposal_id: "ddeploy-proposal:p1",
        acceptance_id: "acceptance:a1",
        accepted_at_utc: "2026-10-04T12:00:00Z",
        operational_dgraph_id: "operational:project-a",
        operational_revision: 3,
      },
    ],
  };
}

export function candidateResponseFixture(projectId = "project-a") {
  return {
    schema: "datum.placement-candidate/1",
    project_id: projectId,
    application_id: "app-1",
    dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
    dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: `sha256:${"b".repeat(64)}` },
    expected_active_dmap_revision: 3,
    // service-a MOVED (node-a -> node-b); service-b ADDED (no prior
    // accepted placement at all).
    placements: [
      { service_id: "service-a", node_id: "node-b" },
      { service_id: "service-b", node_id: "node-a" },
    ],
    planner_provenance: {
      planner_version: "0.1.0",
      policy_name: "deterministic-min-residual-first-fit",
      policy_version: "0.1.0",
      dgraph_id: "dgraph:app-1",
      dgraph_revision: 3,
      dgraph_digest: `sha256:${"a".repeat(64)}`,
      dcontinuum_id: "continuum:project-a",
      dcontinuum_revision: 1,
      dcontinuum_digest: `sha256:${"b".repeat(64)}`,
      decisions: [
        {
          service_id: "service-a",
          node_id: "node-b",
          required_dnode_abi: "datum-dnode/0",
          eligible_node_count: 1,
          node_residual_cpu_cores_before: 4,
          node_residual_memory_bytes_before: 4294967296,
          node_residual_cpu_cores_after: 3,
          node_residual_memory_bytes_after: 4294967295,
        },
        {
          service_id: "service-b",
          node_id: "node-a",
          required_dnode_abi: "datum-dnode/0",
          eligible_node_count: 1,
          node_residual_cpu_cores_before: 8,
          node_residual_memory_bytes_before: 17179869184,
          node_residual_cpu_cores_after: 7,
          node_residual_memory_bytes_after: 17179869183,
        },
      ],
    },
  };
}

export function validateResponseFixture() {
  return {
    valid: true,
    findings: [],
    dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
    dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: `sha256:${"b".repeat(64)}` },
    candidate_placements: [
      { service_id: "service-a", node_id: "node-b" },
      { service_id: "service-b", node_id: "node-a" },
    ],
    capacity_check_is_momentary:
      "This result reflects currently accepted D-Node capacity only, at the moment of this call. It is not a reservation or lease.",
  };
}

export function proposalResponseFixture(projectId = "project-a") {
  return {
    proposal: {
      schema: "datum.ddeploy-proposal/1",
      proposal_id: "ddeploy-proposal:p2",
      project_id: projectId,
      application_id: "app-1",
      source: "operator",
      created_at_utc: "2026-10-04T12:05:00Z",
      expected_active_dmap_revision: 3,
      dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: `sha256:${"a".repeat(64)}` },
      dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: `sha256:${"b".repeat(64)}` },
      placements: [
        { service_id: "service-a", node_id: "node-b" },
        { service_id: "service-b", node_id: "node-a" },
      ],
    },
    proposal_digest: `sha256:${"d".repeat(64)}`,
    state: "pending",
  };
}

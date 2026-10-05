// Phase 138 deterministic Deploy/Reconciliation fixtures. app-1 is accepted
// on node-a and node-b (operational_dgraph revision 7); node-a also hosts a
// service belonging to app-2 (the cross-application/node-wide disclosure
// scenario, Part H).
export function activeResponseFixture(applicationId = "app-1") {
  return {
    project_id: "project-a",
    deployments: [
      {
        project_id: "project-a",
        application_id: applicationId,
        active_dmap: {
          schema: "datum.canonical-dmap/1",
          project_id: "project-a",
          application_id: applicationId,
          dmap_id: `dmap:${applicationId}`,
          revision: 3,
          dgraph_ref: { id: `dgraph:${applicationId}`, revision: 3, digest: `sha256:${"a".repeat(64)}` },
          dcontinuum_ref: { id: "continuum:1", revision: 1, digest: `sha256:${"b".repeat(64)}` },
          placements: [
            { service_id: "service-a", node_id: "node-a" },
            { service_id: "service-b", node_id: "node-b" },
          ],
        },
        dmap_digest: `sha256:${"c".repeat(64)}`,
        proposal_id: "ddeploy-proposal:p1",
        acceptance_id: "ddeploy-acceptance:acc1",
        accepted_at_utc: "2026-10-05T12:00:00Z",
        operational_dgraph_id: "dgraph:project-a",
        operational_revision: 7,
      },
    ],
  };
}

export function operationalDGraphSliceFixture(nodeId, { authorization = null, extraServices = [] } = {}) {
  const baseServices =
    nodeId === "node-a"
      ? [
          { service_id: "service-a", runtime: "container", desired_state: "running", application_id: "app-1" },
          { service_id: "service-other-app", runtime: "container", desired_state: "running", application_id: "app-2" },
        ]
      : [{ service_id: "service-b", runtime: "container", desired_state: "running", application_id: "app-1" }];
  return {
    schema_version: "0.1.0",
    dgraph_id: "dgraph:project-a",
    dgraph_revision: 7,
    project_id: "project-a",
    node_id: nodeId,
    desired_services: [...baseServices, ...extraServices],
    ...(authorization ? { authorization } : {}),
  };
}

export function authorizationFixture(nodeId, overrides = {}) {
  return {
    authorization_id: `opauth:${nodeId}`,
    project_id: "project-a",
    dgraph_id: "dgraph:project-a",
    dgraph_revision: 7,
    node_id: nodeId,
    authorized_by: "operator",
    authorized_at_utc: "2026-10-05T12:00:00Z",
    allow_host_mutation: true,
    operation: "reconcile",
    persistence_scope: "durable",
    expires_at_utc: "2026-10-05T13:00:00Z",
    ...overrides,
  };
}

export function reconciliationRequestFixture(nodeId, overrides = {}) {
  return {
    schema_version: "0.1.0",
    reconciliation_request_id: `reconciliation-request:${nodeId}`,
    client_request_id: `client:${nodeId}`,
    reconciliation_batch_id: "reconciliation-batch:1",
    project_id: "project-a",
    node_id: nodeId,
    target_dgraph_id: "dgraph:project-a",
    target_dgraph_revision: 7,
    mode: "execute",
    requested_by: "operator",
    created_at_utc: "2026-10-05T12:00:00Z",
    expires_at_utc: "2026-10-05T12:15:00Z",
    status: "pending",
    claim_token: null,
    claimed_at_utc: null,
    claim_expires_at_utc: null,
    linked_attempt_id: null,
    updated_at_utc: "2026-10-05T12:00:00Z",
    ...overrides,
  };
}

export function reconciliationBatchResponseFixture(requests, status = "executing") {
  return {
    reconciliation_batch_id: "reconciliation-batch:1",
    project_id: "project-a",
    status,
    requests,
  };
}

export function reconciliationBatchCreateResponseFixture(requests, applicationId = "app-1") {
  return {
    reconciliation_batch_id: "reconciliation-batch:1",
    application_id: applicationId,
    requests,
  };
}

export function reconciliationAttemptRecordFixture(attemptId, nodeId, overrides = {}) {
  return {
    schema_version: "0.1.0",
    reconciliation_attempt_id: attemptId,
    reconciliation_request_id: `reconciliation-request:${nodeId}`,
    project_id: "project-a",
    node_id: nodeId,
    dgraph_id: "dgraph:project-a",
    dgraph_revision: 7,
    authorization_id: `opauth:${nodeId}`,
    started_at_utc: "2026-10-05T12:01:00Z",
    finished_at_utc: "2026-10-05T12:01:05Z",
    status: "succeeded",
    services: {
      [`service-${nodeId === "node-a" ? "a" : "b"}`]: {
        service_id: `service-${nodeId === "node-a" ? "a" : "b"}`,
        result: "mutated",
        host_mutation_performed: true,
        execution_status: "started",
      },
    },
    services_truncated: false,
    last_applied_sequence: 1,
    created_at_utc: "2026-10-05T12:01:00Z",
    updated_at_utc: "2026-10-05T12:01:05Z",
    ...overrides,
  };
}

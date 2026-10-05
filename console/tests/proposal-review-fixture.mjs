// Phase 137 deterministic Proposal Review / Acceptance fixtures. Reuses
// Phase 136's project/app/node identifiers (project-a, app-1, node-a/b)
// for scenarios A-C so the two phases' fixtures stay internally
// consistent; scenario D uses a second application (app-2) that has
// never had an accepted placement, to produce a genuine "no active D-Map"
// first-deployment state without contradicting app-1's Phase 136 history.
//
// Four scenarios, matching the manual-review walkthrough exactly:
//   A: Pending, normal basis -- accept succeeds, active D-Map becomes rev 4.
//   B: Pending, stale basis -- expected rev 3, current active is rev 4;
//      accepting returns a real stale-shaped 409.
//   C: Already Accepted -- GET shows state "accepted"; accept replays
//      idempotently (200, idempotent_replay: true).
//   D: First deployment -- no active D-Map exists yet for app-2; every
//      placement in the proposal is "added".
const DIGEST = (ch) => `sha256:${ch.repeat(64)}`;

export function scenarioAProposal() {
  return {
    proposal: {
      schema: "datum.ddeploy-proposal/1",
      proposal_id: "ddeploy-proposal:scenario-a",
      project_id: "project-a",
      application_id: "app-1",
      source: "operator",
      created_at_utc: "2026-10-04T12:10:00Z",
      expected_active_dmap_revision: 3,
      dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: DIGEST("a") },
      dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: DIGEST("b") },
      placements: [
        { service_id: "service-a", node_id: "node-b" },
        { service_id: "service-b", node_id: "node-a" },
      ],
    },
    proposal_digest: DIGEST("1"),
    state: "pending",
  };
}
export function scenarioAActive() {
  return {
    project_id: "project-a",
    deployments: [
      {
        project_id: "project-a",
        application_id: "app-1",
        active_dmap: {
          schema: "datum.dmap/1",
          project_id: "project-a",
          application_id: "app-1",
          dmap_id: "dmap:app-1",
          revision: 3,
          dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: DIGEST("a") },
          dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: DIGEST("b") },
          placements: [{ service_id: "service-a", node_id: "node-a" }],
        },
        dmap_digest: DIGEST("c"),
        proposal_id: "ddeploy-proposal:p1",
        acceptance_id: "acceptance:a1",
        accepted_at_utc: "2026-10-04T12:00:00Z",
        operational_dgraph_id: "operational:project-a",
        operational_revision: 3,
      },
    ],
  };
}
export function scenarioAAccept() {
  return {
    acceptance: {
      schema: "datum.ddeploy-acceptance/1",
      acceptance_id: "acceptance:scenario-a",
      proposal_id: "ddeploy-proposal:scenario-a",
      proposal_digest: DIGEST("1"),
      project_id: "project-a",
      application_id: "app-1",
      accepted_at_utc: "2026-10-04T12:11:00Z",
      accepted_by: "fixture-reviewer",
      materialized_dmap_ref: { id: "dmap:app-1", revision: 4, digest: DIGEST("4") },
      operational_projection_ref: { operational_dgraph_id: "operational:project-a", revision: 4 },
    },
    materialized_dmap: {
      schema: "datum.dmap/1",
      project_id: "project-a",
      application_id: "app-1",
      dmap_id: "dmap:app-1",
      revision: 4,
      dgraph_ref: { id: "dgraph:app-1", revision: 3, digest: DIGEST("a") },
      dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: DIGEST("b") },
      placements: [
        { service_id: "service-a", node_id: "node-b" },
        { service_id: "service-b", node_id: "node-a" },
      ],
    },
    operational_dgraph_id: "operational:project-a",
    operational_revision: 4,
    idempotent_replay: false,
    currently_active: true,
  };
}

// Scenario B uses its OWN application (app-3) rather than app-1 -- DServer's
// `/active` is keyed per project+application with exactly one current
// state, so a proposal "based on a now-stale revision" needs an
// application whose active revision has genuinely moved on, independent
// of scenario A's own app-1 active state.
export function scenarioBProposal() {
  const p = scenarioAProposal();
  return {
    ...p,
    proposal: { ...p.proposal, proposal_id: "ddeploy-proposal:scenario-b", application_id: "app-3", dgraph_ref: { id: "dgraph:app-3", revision: 3, digest: DIGEST("a") } },
    proposal_digest: DIGEST("2"),
  };
}
export function scenarioBActive() {
  const a = scenarioAActive();
  return {
    ...a,
    deployments: [
      {
        ...a.deployments[0],
        application_id: "app-3",
        active_dmap: { ...a.deployments[0].active_dmap, application_id: "app-3", dmap_id: "dmap:app-3", revision: 4 },
        acceptance_id: "acceptance:a2",
        operational_revision: 4,
      },
    ],
  };
}
export function scenarioBAcceptError() {
  return { error: "stale_active_dmap_revision", message: "Proposal's expected active D-Map revision (3) no longer matches the current active revision (4)." };
}

// Scenario C likewise uses its own application (app-4), already accepted
// at revision 4 -- distinct from scenario A's still-Pending app-1.
export function scenarioCProposal() {
  const p = scenarioAProposal();
  return {
    ...p,
    proposal: { ...p.proposal, proposal_id: "ddeploy-proposal:scenario-c", application_id: "app-4", dgraph_ref: { id: "dgraph:app-4", revision: 3, digest: DIGEST("a") } },
    proposal_digest: DIGEST("3"),
    state: "accepted",
  };
}
export function scenarioCActive() {
  const a = scenarioAActive();
  return {
    ...a,
    deployments: [
      {
        ...a.deployments[0],
        application_id: "app-4",
        active_dmap: {
          ...a.deployments[0].active_dmap,
          application_id: "app-4",
          dmap_id: "dmap:app-4",
          revision: 4,
          placements: [
            { service_id: "service-a", node_id: "node-b" },
            { service_id: "service-b", node_id: "node-a" },
          ],
        },
        acceptance_id: "acceptance:scenario-c",
        operational_revision: 4,
      },
    ],
  };
}
export function scenarioCAccept() {
  return {
    acceptance: {
      schema: "datum.ddeploy-acceptance/1",
      acceptance_id: "acceptance:scenario-c",
      proposal_id: "ddeploy-proposal:scenario-c",
      proposal_digest: DIGEST("3"),
      project_id: "project-a",
      application_id: "app-4",
      accepted_at_utc: "2026-10-04T12:09:00Z",
      accepted_by: "original-reviewer",
      materialized_dmap_ref: { id: "dmap:app-1", revision: 4, digest: DIGEST("4") },
      operational_projection_ref: { operational_dgraph_id: "operational:project-a", revision: 4 },
    },
    materialized_dmap: scenarioCActive().deployments[0].active_dmap,
    operational_dgraph_id: "operational:project-a",
    operational_revision: 4,
    idempotent_replay: true,
    currently_active: true,
  };
}

export function scenarioDProposal() {
  return {
    proposal: {
      schema: "datum.ddeploy-proposal/1",
      proposal_id: "ddeploy-proposal:scenario-d",
      project_id: "project-a",
      application_id: "app-2",
      source: "deterministic",
      created_at_utc: "2026-10-04T12:12:00Z",
      expected_active_dmap_revision: null,
      dgraph_ref: { id: "dgraph:app-2", revision: 1, digest: DIGEST("5") },
      dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: DIGEST("b") },
      placements: [{ service_id: "service-c", node_id: "node-a" }],
    },
    proposal_digest: DIGEST("6"),
    state: "pending",
  };
}
export function scenarioDActive() {
  return { project_id: "project-a", deployments: [] };
}
export function scenarioDAccept() {
  return {
    acceptance: {
      schema: "datum.ddeploy-acceptance/1",
      acceptance_id: "acceptance:scenario-d",
      proposal_id: "ddeploy-proposal:scenario-d",
      proposal_digest: DIGEST("6"),
      project_id: "project-a",
      application_id: "app-2",
      accepted_at_utc: "2026-10-04T12:13:00Z",
      accepted_by: "fixture-reviewer",
      materialized_dmap_ref: { id: "dmap:app-2", revision: 1, digest: DIGEST("7") },
      operational_projection_ref: { operational_dgraph_id: "operational:project-a", revision: 1 },
    },
    materialized_dmap: {
      schema: "datum.dmap/1",
      project_id: "project-a",
      application_id: "app-2",
      dmap_id: "dmap:app-2",
      revision: 1,
      dgraph_ref: { id: "dgraph:app-2", revision: 1, digest: DIGEST("5") },
      dcontinuum_ref: { id: "continuum:project-a", revision: 1, digest: DIGEST("b") },
      placements: [{ service_id: "service-c", node_id: "node-a" }],
    },
    operational_dgraph_id: "operational:project-a",
    operational_revision: 1,
    idempotent_replay: false,
    currently_active: true,
  };
}

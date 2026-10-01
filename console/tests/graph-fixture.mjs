// Test-only logical snapshot. No production/demo graph injection.
// Development manual review imports this test module explicitly; never bundled.
export function unevenGraphFixture(project = "phase131-visual-review") {
  const data = graphFixture(project),
    graph = data.graph.value;
  const app = graph.applications[0],
    parent = graph.nodes.find((n) => n.kind === "application"),
    service = graph.nodes.find((n) => n.kind === "dserv");
  graph.applications = [];
  graph.nodes = [];
  graph.edges = [];
  for (const [i, size] of [2, 3, 2, 8].entries()) {
    const id = `application-${String.fromCharCode(65 + i)}`;
    graph.applications.push({
      ...app,
      application_id: id,
      dgraph_id: `logical-${id}`,
    });
    graph.nodes.push({
      ...parent,
      id: `group-${i}`,
      canonical_id: id,
      application_id: id,
    });
    for (let j = 0; j < size; j++)
      graph.nodes.push({
        ...service,
        id: `node-${i}-${j}`,
        canonical_id: `service-${j}`,
        application_id: id,
        parent_id: `group-${i}`,
      });
    for (const [call, source, target] of [
      ["forward", 0, 1],
      ["reverse", 1, 0],
      ["self", 0, 0],
      ["parallel", 0, 1],
    ])
      graph.edges.push({
        id: `call-${i}-${call}`,
        canonical_id: call,
        application_id: id,
        kind: "dcall",
        source: `node-${i}-${source}`,
        target: `node-${i}-${target}`,
      });
  }
  graph.coverage.active_application_count = graph.applications.length;
  graph.coverage.included_application_count = graph.applications.length;
  return data;
}
export function graphFixture(project = "alpha") {
  const graph = {
    coverage: {
      scope: "active_accepted_applications_only",
      completeness: "complete",
      active_application_count: 2,
      included_application_count: 2,
    },
    applications: [],
    nodes: [],
    edges: [],
  };
  for (const application of ["app-a", "app-b"]) {
    graph.applications.push({
      application_id: application,
      dgraph_id: `logical-${application}`,
      revision: 4,
      digest: "sha256:" + "a".repeat(64),
      acceptance_id: `accepted-${application}`,
    });
    graph.nodes.push({
      id: `parent-${application}`,
      kind: "application",
      canonical_id: application,
      application_id: application,
      parent_id: null,
      execution_abi: null,
      cpu_cores: null,
      memory_bytes: null,
    });
    for (const service of ["one", "two"])
      graph.nodes.push({
        id: `${application}-${service}`,
        kind: "dserv",
        canonical_id: service,
        application_id: application,
        parent_id: `parent-${application}`,
        execution_abi: "datum-dnode/0",
        cpu_cores: 1,
        memory_bytes: "18446744073709551615",
      });
    for (const [id, source, target] of [
      ["forward", "one", "two"],
      ["reverse", "two", "one"],
      ["self", "one", "one"],
    ])
      graph.edges.push({
        id: `${application}-${id}`,
        kind: "dcall",
        canonical_id: id,
        application_id: application,
        source: `${application}-${source}`,
        target: `${application}-${target}`,
      });
  }
  return {
    project_id: project,
    generated_at_utc: "2026-09-30T15:00:00Z",
    source_kind: "accepted_application_dgraph_snapshots",
    graph: { availability: "available", value: graph },
  };
}

import type { Core, ElementDefinition, NodeSingular } from "cytoscape";
import type { GraphContent, ProjectGraph } from "../types/graph.ts";
export const GRAPH_SOURCE_LABEL =
  "Accepted application D-Graph snapshots · logical structure";
const object = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === "object" && !Array.isArray(v);
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0;
const count = (v: unknown): v is number =>
  typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const nullable = (v: unknown) => v === null || text(v);
export function validProjectGraph(
  v: unknown,
  project: string,
): v is ProjectGraph {
  if (
    !object(v) ||
    v.project_id !== project ||
    v.source_kind !== "accepted_application_dgraph_snapshots" ||
    !text(v.generated_at_utc) ||
    !/^\d{4}-\d\d-\d\dT.*Z$/u.test(v.generated_at_utc) ||
    !Number.isFinite(Date.parse(v.generated_at_utc)) ||
    !object(v.graph)
  )
    return false;
  const section = v.graph;
  if (section.availability === "not_configured") return true;
  if (section.availability === "indeterminate")
    return typeof section.value === "string";
  if (["unavailable", "corrupt"].includes(String(section.availability))) {
    const issue = section.value;
    return (
      object(issue) &&
      text(issue.source) &&
      text(issue.code) &&
      typeof issue.safe_message === "string" &&
      new TextEncoder().encode(issue.safe_message).length <= 512
    );
  }
  if (section.availability !== "available" || !object(section.value))
    return false;
  const g = section.value;
  if (
    !object(g.coverage) ||
    g.coverage.scope !== "active_accepted_applications_only" ||
    !["complete", "partial"].includes(String(g.coverage.completeness)) ||
    !count(g.coverage.active_application_count) ||
    !count(g.coverage.included_application_count) ||
    !Array.isArray(g.applications) ||
    g.applications.length > 100 ||
    !Array.isArray(g.nodes) ||
    g.nodes.length > 2000 ||
    !Array.isArray(g.edges) ||
    g.edges.length > 4000 ||
    g.coverage.included_application_count !== g.applications.length ||
    g.coverage.active_application_count < g.applications.length ||
    (g.coverage.completeness === "complete" &&
      g.coverage.active_application_count !== g.applications.length)
  )
    return false;
  const applications = new Set<string>();
  for (const a of g.applications) {
    if (
      !object(a) ||
      !text(a.application_id) ||
      applications.has(a.application_id) ||
      !text(a.dgraph_id) ||
      !count(a.revision) ||
      a.revision < 1 ||
      !text(a.digest) ||
      !nullable(a.acceptance_id)
    )
      return false;
    applications.add(a.application_id);
  }
  const nodes = new Map<string, Record<string, unknown>>();
  for (const n of g.nodes) {
    if (
      !object(n) ||
      !text(n.id) ||
      nodes.has(n.id) ||
      !text(n.canonical_id) ||
      !text(n.application_id) ||
      !applications.has(n.application_id) ||
      !["application", "dserv"].includes(String(n.kind)) ||
      !nullable(n.parent_id) ||
      !nullable(n.execution_abi) ||
      !(n.cpu_cores === null || count(n.cpu_cores)) ||
      !(
        n.memory_bytes === null ||
        (text(n.memory_bytes) &&
          /^(0|[1-9]\d{0,19})$/u.test(n.memory_bytes) &&
          BigInt(n.memory_bytes) <= 18446744073709551615n)
      ) ||
      (n.project_id !== undefined && n.project_id !== project)
    )
      return false;
    nodes.set(n.id, n);
  }
  for (const app of applications) {
    const parents = [...nodes.values()].filter(
      (n) => n.kind === "application" && n.application_id === app,
    );
    if (
      parents.length !== 1 ||
      !parents[0] ||
      parents[0].parent_id !== null ||
      parents[0].canonical_id !== app
    )
      return false;
  }
  for (const n of nodes.values()) {
    if (n.kind === "dserv") {
      const parent = nodes.get(String(n.parent_id));
      if (
        !parent ||
        parent.kind !== "application" ||
        parent.application_id !== n.application_id
      )
        return false;
    }
  }
  const ids = new Set(nodes.keys());
  for (const e of g.edges) {
    if (
      !object(e) ||
      !text(e.id) ||
      ids.has(e.id) ||
      e.kind !== "dcall" ||
      !text(e.canonical_id) ||
      !text(e.application_id) ||
      !applications.has(e.application_id) ||
      !text(e.source) ||
      !text(e.target) ||
      (e.project_id !== undefined && e.project_id !== project)
    )
      return false;
    const source = nodes.get(e.source),
      target = nodes.get(e.target);
    if (
      !source ||
      !target ||
      source.kind !== "dserv" ||
      target.kind !== "dserv" ||
      source.application_id !== e.application_id ||
      target.application_id !== e.application_id
    )
      return false;
    ids.add(e.id);
  }
  return true;
}
const compare = (a: { id: string }, b: { id: string }) =>
  a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
export function graphElements(
  graph: GraphContent,
  application: string | null = null,
): ElementDefinition[] {
  const nodes = graph.nodes
    .filter((n) => application === null || n.application_id === application)
    .slice()
    .sort(compare);
  const edges = graph.edges
    .filter((e) => application === null || e.application_id === application)
    .slice()
    .sort(compare);
  return [
    ...nodes.map((n) => ({
      group: "nodes" as const,
      data: {
        id: n.id,
        parent: n.parent_id ?? undefined,
        kind: n.kind,
        label:
          n.canonical_id.length > 28
            ? `${n.canonical_id.slice(0, 25)}…`
            : n.canonical_id,
      },
    })),
    ...edges.map((e) => ({
      group: "edges" as const,
      data: {
        id: e.id,
        source: e.source,
        target: e.target,
        kind: e.kind,
        label: "D-Call",
      },
    })),
  ];
}
export function edgeCounts(graph: GraphContent, id: string) {
  return {
    incoming: graph.edges.filter((e) => e.target === id).length,
    outgoing: graph.edges.filter((e) => e.source === id).length,
  };
}

/** Presentation only. Local service grids, then measured application boxes in rows.
 * Edges do not influence positions; no coordinates are persisted or used as authority.
 * Include labels in the outer bounds so headers and empty application groups fit too.
 */
export function layoutGroupedGraph(cy: Core): void {
  const groups = (
    cy
      .nodes()
      .filter((n) => n.data("kind") === "application")
      .toArray() as NodeSingular[]
  ).sort((a, b) => compare({ id: a.id() }, { id: b.id() }));
  const boxes = [];
  for (const group of groups) {
    const children = (
      group.children().nodes().toArray() as NodeSingular[]
    ).sort((a, b) => compare({ id: a.id() }, { id: b.id() }));
    const columns = Math.max(1, Math.ceil(Math.sqrt(children.length)));
    children.forEach((child, i) =>
      child.position({
        x: (i % columns) * 200,
        y: Math.floor(i / columns) * 160 + 60,
      }),
    );
    if (!children.length) group.position({ x: 0, y: 0 });
    const bounds = group.boundingBox({
      includeLabels: true,
      includeOverlays: false,
    });
    boxes.push({ group, children, bounds });
  }
  const columns = Math.max(1, Math.ceil(Math.sqrt(boxes.length)));
  const gutter = 80;
  let y = gutter;
  for (let start = 0; start < boxes.length; start += columns) {
    const row = boxes.slice(start, start + columns);
    const height = Math.max(...row.map(({ bounds }) => bounds.h));
    let x = gutter;
    for (const { group, children, bounds } of row) {
      const dx = x - bounds.x1,
        dy = y - bounds.y1;
      const movable = children.length ? children : [group];
      movable.forEach((node) => {
        const p = node.position();
        node.position({ x: p.x + dx, y: p.y + dy });
      });
      x += bounds.w + gutter;
    }
    y += height + gutter;
  }
}

export function graphCoverageText(graph: GraphContent): string {
  const {
    active_application_count: active,
    included_application_count: included,
    completeness,
  } = graph.coverage;
  if (completeness === "complete")
    return `${included} of ${active} active accepted application snapshots included. Pending/unaccepted applications are outside this source, not absent by inference.`;
  const omitted = active - included;
  return `Bounded graph preview: ${included} of ${active} accepted application snapshots included. ${omitted} accepted snapshot${omitted === 1 ? " was" : "s were"} omitted by preview limits (100 applications, 2,000 nodes including groups, 4,000 D-Calls). Inclusion stops at the first snapshot exceeding a limit; later snapshots are not considered. This is not complete project application inventory.`;
}

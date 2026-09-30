import type { ElementDefinition } from "cytoscape";
import type { GraphContent } from "../types/graph.ts";
import type { ProjectRuntimeGraph } from "../types/runtime-graph.ts";
import { validProjectGraph, graphElements } from "./graph-model.ts";
const object = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0;
const count = (v: unknown): v is number =>
  typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const timestamp = (v: unknown) =>
  text(v) && /^\d{4}-\d\d-\d\dT/u.test(v) && Number.isFinite(Date.parse(v));
export function validRuntimeGraph(
  v: unknown,
  project: string,
): v is ProjectRuntimeGraph {
  if (
    !object(v) ||
    v.project_id !== project ||
    !validProjectGraph(v.logical, project) ||
    v.runtime_source_kind !== "admitted_dserv_realization_evidence" ||
    !object(v.runtime) ||
    !Array.isArray(v.temporal_quality)
  )
    return false;
  const section = v.runtime;
  if (section.availability !== "available") {
    // Reuse safe SourceSection validation, without manufacturing logical content.
    return (
      v.temporal_quality.length === 0 &&
      validProjectGraph({ ...v.logical, graph: section }, project)
    );
  }
  const value = section.value;
  if (
    !object(value) ||
    !["complete", "partial"].includes(String(value.completeness)) ||
    !count(value.total_evidence_count) ||
    !count(value.included_evidence_count) ||
    !Array.isArray(value.evidence) ||
    value.evidence.length > 1000 ||
    value.included_evidence_count !== value.evidence.length ||
    value.total_evidence_count < value.evidence.length ||
    (value.completeness === "complete" &&
      value.total_evidence_count !== value.evidence.length) ||
    v.temporal_quality.length !== value.evidence.length
  )
    return false;
  const logical =
    v.logical.graph.availability === "available" ? v.logical.graph.value : null;
  const ids = new Set(
    logical ? [...logical.nodes, ...logical.edges].map((n) => n.id) : [],
  );
  const identities = new Set<string>();
  const observationIds = new Set<string>();
  for (const [i, e] of value.evidence.entries()) {
    if (
      !object(e) ||
      e.project_id !== project ||
      ![
        "id",
        "application_id",
        "service_id",
        "node_id",
        "monitor_instance_id",
        "observation_id",
        "observation_digest",
        "collector",
      ].every((k) => text(e[k])) ||
      ids.has(String(e.id)) ||
      !timestamp(e.observed_at_utc) ||
      !timestamp(e.received_at_utc) ||
      !["unknown", "healthy", "degraded", "unhealthy"].includes(
        String(e.observed_health_claim),
      )
    )
      return false;
    if (!text(e.reporting_node_id)) return false;
    const identity = JSON.stringify([
      e.project_id,
      e.application_id,
      e.service_id,
      e.node_id,
      e.reporting_node_id,
      e.monitor_instance_id,
    ]);
    if (
      e.id !==
      JSON.stringify([
        "runtime_evidence",
        e.project_id,
        e.application_id,
        e.service_id,
        e.node_id,
        e.reporting_node_id,
        e.monitor_instance_id,
      ])
    )
      return false;
    if (identities.has(identity) || observationIds.has(String(e.observation_id)))
      return false;
    identities.add(identity);
    observationIds.add(String(e.observation_id));
    ids.add(String(e.id));
    if (
      e.dmap_ref !== null &&
      (!object(e.dmap_ref) ||
        !text(e.dmap_ref.id) ||
        !count(e.dmap_ref.revision) ||
        e.dmap_ref.revision < 1 ||
        !text(e.dmap_ref.digest))
    )
      return false;
    if (e.logical_node_id !== null) {
      const node = logical?.nodes.find((n) => n.id === e.logical_node_id);
      if (
        !node ||
        node.kind !== "dserv" ||
        node.application_id !== e.application_id ||
        node.canonical_id !== e.service_id
      )
        return false;
    } else if (
      logical?.nodes.some(
        (n) =>
          n.kind === "dserv" &&
          n.application_id === e.application_id &&
          n.canonical_id === e.service_id,
      )
    )
      return false;
    const t = v.temporal_quality[i];
    if (
      !object(t) ||
      t.project_id !== project ||
      t.application_id !== e.application_id ||
      t.node_id !== e.reporting_node_id ||
      t.observation_id !== e.observation_id ||
      !["fresh", "stale", "indeterminate", "missing_evidence"].includes(
        String(t.communication_freshness),
      ) ||
      !["fresh", "stale", "indeterminate", "missing_evidence"].includes(
        String(t.observation_freshness),
      ) ||
      ![
        "aligned_or_no_discrepancy",
        "source_ahead",
        "behind_or_delayed",
        "evaluation_ordering_anomaly",
        "missing_evidence",
      ].includes(String(t.source_time_quality)) ||
      !timestamp(t.source_observed_at_utc) ||
      !timestamp(t.server_received_at_utc) ||
      Date.parse(String(t.source_observed_at_utc)) !==
        Date.parse(String(e.observed_at_utc)) ||
      Date.parse(String(t.server_received_at_utc)) !==
        Date.parse(String(e.received_at_utc))
    )
      return false;
  }
  return true;
}
export const realizationEdgeId = (id: string) =>
  JSON.stringify(["realization_relation", id]);
export function runtimeElements(
  graph: GraphContent,
  runtime: ProjectRuntimeGraph | undefined,
  expanded: ReadonlySet<string>,
  application: string | null,
): ElementDefinition[] {
  const elements = graphElements(graph, application);
  if (runtime?.runtime.availability !== "available") return elements;
  const nodes = new Map(graph.nodes.map((n) => [n.id, n]));
  for (const e of runtime.runtime.value.evidence) {
    const logical = e.logical_node_id
      ? nodes.get(e.logical_node_id)
      : undefined;
    if (
      !logical ||
      !expanded.has(logical.id) ||
      (application !== null && e.application_id !== application)
    )
      continue;
    elements.push({
      group: "nodes",
      data: {
        id: e.id,
        parent: logical.parent_id ?? undefined,
        kind: "runtime_evidence",
        visual_category: "runtime",
        label: `Evidence · ${e.observation_id.slice(0, 20)}`,
        logical_node_id: logical.id,
      },
    });
    elements.push({
      group: "edges",
      data: {
        id: realizationEdgeId(e.id),
        source: logical.id,
        target: e.id,
        kind: "realization",
        visual_relation: "realization",
        label: "Realization evidence",
      },
    });
  }
  return elements;
}

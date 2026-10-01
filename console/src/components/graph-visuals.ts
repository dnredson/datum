/** DATUM graph presentation contract. Tokens classify visuals, never authority.
 * Reserved categories do not create entities, relations, health or ownership.
 */
import type { StylesheetJson } from "cytoscape";
import type { GraphContent } from "../types/graph.ts";
import { el } from "./presentation.ts";

export const GRAPH_CATEGORIES = {
  infrastructure: {
    label: "Infrastructure",
    color: "#416d9c",
    shape: "round-rectangle",
    meaning: "Execution resource; not project ownership",
  },
  software: {
    label: "Logical software",
    color: "#7852a1",
    shape: "rectangle",
    meaning: "D-Serv · logical service/component, not a runtime instance",
  },
  runtime: {
    label: "Runtime realization",
    color: "#258a96",
    shape: "round-rectangle",
    meaning: "Realized instance, distinct from logical software",
  },
  storage: {
    label: "Data / storage",
    color: "#357e58",
    shape: "barrel",
    meaning: "Explicitly modeled storage entity",
  },
  iot: {
    label: "IoT / device",
    color: "#b58126",
    shape: "hexagon",
    meaning: "Explicitly modeled device entity",
  },
} as const;
export const GRAPH_RELATIONS = {
  logical: {
    label: "D-Call",
    color: "#71859a",
    line: "solid",
    arrow: "triangle",
    meaning:
      "Directed logical invocation; cycles, reciprocal calls and self-calls are allowed. No deployment-order implication.",
  },
  realization: {
    label: "Runtime realization",
    color: "#258a96",
    line: "solid",
    arrow: "vee",
    meaning: "Logical service → runtime instance",
  },
  placement: {
    label: "Placement",
    color: "#71859a",
    line: "dashed",
    arrow: "triangle",
    meaning: "Runtime/workload → resource; not exclusive ownership",
  },
  physical: {
    label: "Physical / network",
    color: "#84929e",
    line: "solid",
    arrow: "none",
    meaning:
      "Explicit physical/network relationship only; direction follows its source contract",
  },
  observation: {
    label: "Observation / monitoring",
    color: "#84929e",
    line: "dotted",
    arrow: "none",
    meaning: "Observational provenance, not logical invocation",
  },
} as const;
export const GRAPH_STATUS_OVERLAYS = {
  normal: { label: "Known normal / positive", color: "#357e58" },
  attention: { label: "Attention / indeterminate", color: "#c5a52b" },
  degraded: { label: "Degraded / high attention", color: "#c87526" },
  critical: { label: "Critical / failure", color: "#b33d48" },
  neutral: { label: "Unknown / not configured", color: "#7b8691" },
} as const;
const GROUP = {
  color: "#f0f3f6",
  border: "#98a3b0",
  shape: "round-rectangle",
} as const;
const TEXT = "#263847",
  SELECTION = "#244f70";

/** Future evidence-backed overlays must retain category fill/shape and text state.
 * This helper is not applied to Phase 131 entities; no health inference exists.
 */
export function graphStatusOverlay(status: keyof typeof GRAPH_STATUS_OVERLAYS) {
  return {
    "outline-color": GRAPH_STATUS_OVERLAYS[status].color,
    "outline-width": 3,
    "outline-opacity": 1,
  };
}

export function graphVisualStyles(): StylesheetJson {
  return [
    {
      selector: "node",
      style: {
        label: "data(label)",
        color: TEXT,
        "font-size": 11,
        "text-valign": "bottom",
        "text-margin-y": 7,
        "text-wrap": "wrap",
        "text-max-width": "140px",
        width: 48,
        height: 34,
      },
    },
    ...Object.entries(GRAPH_CATEGORIES).map(([category, token]) => ({
      selector: `node[visual_category = "${category}"]`,
      style: {
        "background-color": token.color,
        shape: token.shape,
        ...(category === "runtime" ? { width: 36, height: 26 } : {}),
      },
    })),
    {
      selector: 'node[kind = "application"]',
      style: {
        "background-color": GROUP.color,
        "border-color": GROUP.border,
        "border-width": 1,
        shape: GROUP.shape,
        "text-valign": "top",
        "text-margin-y": -10,
        "text-max-width": "180px",
        padding: "30px",
        "min-width": "180px",
        "min-height": "80px",
        width: 180,
        height: 80,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1.5,
        "curve-style": "bezier",
        label: "data(label)",
        "font-size": 9,
        color: TEXT,
      },
    },
    ...Object.entries(GRAPH_RELATIONS).map(([relation, token]) => ({
      selector: `edge[visual_relation = "${relation}"]`,
      style: {
        "line-color": token.color,
        "target-arrow-color": token.color,
        "line-style": token.line,
        "target-arrow-shape": token.arrow,
        ...(relation === "observation" ? { opacity: 0.55 } : {}),
      },
    })),
    {
      selector: ":selected",
      style: {
        "border-color": SELECTION,
        "border-width": 3,
        "line-color": SELECTION,
        "target-arrow-color": SELECTION,
      },
    },
  ];
}

/** Only canonical kinds supported by the current graph DTO activate legend entries.
 * Future adapters must explicitly map typed source entities/relationships.
 */
export function graphLegendItemsFor(
  presentCategories: Iterable<keyof typeof GRAPH_CATEGORIES>,
  presentRelations: Iterable<keyof typeof GRAPH_RELATIONS>,
  applicationGrouping = false,
) {
  const items: {
    id: string;
    label: string;
    meaning: string;
    color: string;
    shape?: string;
    line?: string;
    arrow?: string;
  }[] = [];
  const categories = new Set(presentCategories),
    relations = new Set(presentRelations);
  for (const id of Object.keys(
    GRAPH_CATEGORIES,
  ) as (keyof typeof GRAPH_CATEGORIES)[])
    if (categories.has(id)) items.push({ id, ...GRAPH_CATEGORIES[id] });
  if (applicationGrouping)
    items.push({
      id: "application-group",
      label: "Application grouping",
      meaning:
        "Presentation-only container of an accepted logical D-Graph snapshot; not a canonical vertex",
      color: GROUP.color,
      shape: GROUP.shape,
    });
  for (const id of Object.keys(
    GRAPH_RELATIONS,
  ) as (keyof typeof GRAPH_RELATIONS)[])
    if (relations.has(id)) items.push({ id, ...GRAPH_RELATIONS[id] });
  return items;
}

export function graphLegendItems(graph: Pick<GraphContent, "nodes" | "edges">) {
  return graphLegendItemsFor(
    graph.nodes.some((node) => node.kind === "dserv") ? ["software"] : [],
    graph.edges.some((edge) => edge.kind === "dcall") ? ["logical"] : [],
    graph.nodes.some((node) => node.kind === "application"),
  );
}

export function graphLegendView(
  graph: Pick<GraphContent, "nodes" | "edges">,
): HTMLElement {
  return renderGraphLegend(graphLegendItems(graph));
}

export function renderGraphLegend(
  items: ReturnType<typeof graphLegendItemsFor>,
): HTMLElement {
  const list = el("ul", "graph-legend-items");
  for (const item of items) {
    const row = el("li", "graph-legend-item");
    const swatch = el(
      "span",
      item.line
        ? "graph-relation-swatch"
        : `graph-category-swatch graph-shape-${item.shape}`,
    );
    swatch.setAttribute("aria-hidden", "true");
    if (item.line) {
      swatch.style.borderTop = `2px ${item.line} ${item.color}`;
      swatch.textContent = item.arrow !== "none" ? "→" : "";
      swatch.style.color = item.color;
    } else swatch.style.backgroundColor = item.color;
    row.append(swatch, el("span", "small", `${item.label} · ${item.meaning}`));
    list.append(row);
  }
  return list;
}

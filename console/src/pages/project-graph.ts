import cytoscape from "cytoscape";
import type { Core } from "cytoscape";
import type { ProjectGraph } from "../types/graph.ts";
import type {
  ProjectRuntimeGraph,
  RuntimeEvidence,
} from "../types/runtime-graph.ts";
import { runtimeElements, realizationEdgeId } from "./runtime-graph-model.ts";
import { el, badge, sectionValue } from "../components/presentation.ts";
import {
  terminalMetricPanel,
  type TerminalMetricPanelState,
} from "../components/terminal-metrics.ts";
import {
  graphVisualStyles,
  graphLegendItemsFor,
  renderGraphLegend,
} from "../components/graph-visuals.ts";
import { projectPath } from "../app/router.ts";
import { PROJECT_SECTIONS } from "./project-workspace.ts";
import {
  GRAPH_SOURCE_LABEL,
  edgeCounts,
  layoutGroupedGraph,
  graphCoverageText,
} from "./graph-model.ts";

function formatBytes(value: number): string {
  if (value >= 1_073_741_824) return `${(value / 1_073_741_824).toFixed(1)} GiB`;
  if (value >= 1_048_576) return `${(value / 1_048_576).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}


export interface ProjectGraphLiveOptions {
  fetchRuntime?: (signal: AbortSignal) => Promise<ProjectRuntimeGraph>;
  pollMs?: number;
}

function dservMetricState(
  logicalNodeId: string,
  applicationId: string,
  serviceId: string,
  source: ProjectRuntimeGraph | undefined,
  liveLabel?: string,
): TerminalMetricPanelState {
  const candidates =
    source?.runtime.availability === "available"
      ? source.runtime.value.evidence
          .filter((item) => {
            if (
              item.logical_node_id !== logicalNodeId ||
              item.resource_usage === undefined
            )
              return false;
            const temporal = source.temporal_quality.find(
              (candidate) => candidate.observation_id === item.observation_id,
            );
            return temporal?.observation_freshness === "fresh";
          })
          .sort(
            (a, b) =>
              Date.parse(b.received_at_utc) - Date.parse(a.received_at_utc),
          )
      : [];
  const latest = candidates[0];
  if (!latest?.resource_usage) {
    return {
      title: `D-SERV :: ${applicationId} / ${serviceId}`,
      subtitle: "fresh runtime telemetry unavailable",
      liveLabel,
      metrics: [
        { label: "CPU", valueText: "--", ratio: null },
        { label: "MEMORY", valueText: "--", ratio: null },
        { label: "PIDs", valueText: "--", ratio: null },
      ],
      meta: [["state", "no fresh container resource observation"]],
    };
  }
  const usage = latest.resource_usage;
  const temporal = source?.temporal_quality.find(
    (candidate) => candidate.observation_id === latest.observation_id,
  );
  return {
    title: `D-SERV :: ${applicationId} / ${serviceId}`,
    subtitle: `node ${latest.node_id}`,
    liveLabel,
    metrics: [
      {
        label: "CPU",
        valueText: `${usage.cpu_percent.toFixed(1)}%`,
        ratio: usage.cpu_percent / 100,
      },
      {
        label: "MEMORY",
        valueText:
          usage.memory_limit_bytes > 0
            ? `${formatBytes(usage.memory_used_bytes)} / ${formatBytes(usage.memory_limit_bytes)}`
            : `${formatBytes(usage.memory_used_bytes)} / limit unavailable`,
        ratio:
          usage.memory_limit_bytes > 0
            ? usage.memory_used_bytes / usage.memory_limit_bytes
            : null,
      },
      {
        label: "PIDs",
        valueText: String(usage.pids),
        ratio: null,
      },
    ],
    meta: [
      ["observation", latest.observation_id],
      ["monitor", latest.monitor_instance_id],
      ["freshness", temporal?.observation_freshness ?? "unavailable"],
      ["observed", latest.observed_at_utc],
    ],
  };
}

export function projectGraphView(
  data: ProjectGraph,
  runtime?: ProjectRuntimeGraph,
  live: ProjectGraphLiveOptions = {},
) {
  const expanded = new Set<string>();
  const evidence =
    runtime?.runtime.availability === "available"
      ? runtime.runtime.value.evidence
      : [];
  let cy: Core | undefined;
  let disposed = false;
  let resize: ResizeObserver | undefined;
  let mount = () => {};
  let liveTimer: ReturnType<typeof setTimeout> | undefined;
  let liveController: AbortController | undefined;
  let liveSelection: string | undefined;
  const stopLiveMetrics = () => {
    if (liveTimer !== undefined) clearTimeout(liveTimer);
    liveTimer = undefined;
    liveController?.abort();
    liveController = undefined;
    liveSelection = undefined;
  };
  const startLiveMetrics = (
    logicalNodeId: string,
    applicationId: string,
    serviceId: string,
    panel: ReturnType<typeof terminalMetricPanel>,
  ) => {
    stopLiveMetrics();
    if (!live.fetchRuntime) return;
    liveSelection = logicalNodeId;
    const pollMs = Math.max(1000, live.pollMs ?? 2000);
    const poll = async () => {
      if (disposed || liveSelection !== logicalNodeId || !live.fetchRuntime) return;
      const controller = new AbortController();
      liveController = controller;
      try {
        const next = await live.fetchRuntime(controller.signal);
        if (disposed || liveSelection !== logicalNodeId) return;
        panel.update(
          dservMetricState(
            logicalNodeId,
            applicationId,
            serviceId,
            next,
            `LIVE · ${(pollMs / 1000).toFixed(pollMs % 1000 === 0 ? 0 : 1)}s`,
          ),
        );
      } catch {
        if (!controller.signal.aborted && liveSelection === logicalNodeId) {
          const fallback = dservMetricState(
            logicalNodeId,
            applicationId,
            serviceId,
            undefined,
            "LIVE · retrying",
          );
          panel.update(fallback);
        }
      } finally {
        if (!disposed && liveSelection === logicalNodeId)
          liveTimer = setTimeout(() => void poll(), pollMs);
      }
    };
    liveTimer = setTimeout(() => void poll(), pollMs);
  };
  const element = el("div", "page-content project-graph");
  const breadcrumb = el("nav", "workspace-breadcrumb");
  breadcrumb.setAttribute("aria-label", "Breadcrumb");
  const projects = el("a", "", "Projects");
  projects.href = "/projects";
  projects.dataset.route = "";
  const overview = el("a", "entity-identity", data.project_id);
  overview.href = projectPath(data.project_id);
  overview.dataset.route = "";
  breadcrumb.append(
    projects,
    el("span", "", "/"),
    overview,
    el("span", "", "/ Graph"),
  );
  const header = el("header", "page-intro");
  const edit = el("a","btn btn-outline-secondary","Open local DGraph editor");
  edit.href=`${projectPath(data.project_id)}/graph/edit`;edit.dataset.route="";
  header.append(edit);
  header.append(
    badge("PROJECT GRAPH · READ ONLY", "info"),
    el("h1", "", "Logical graph overview"),
    el("p", "entity-identity", data.project_id),
    el("p", "small text-secondary", GRAPH_SOURCE_LABEL),
    el(
      "p",
      "small text-secondary",
      "Accepted logical snapshots remain separate from admitted DMonitor realization evidence. This is not an application registry, complete runtime topology or placement view.",
    ),
  );
  const nav = el("nav", "project-subnav");
  nav.setAttribute("aria-label", "Project workspace sections");
  for (const [name, phase] of PROJECT_SECTIONS) {
    if (name === "Overview" || name === "Graph") {
      const link = el(
        "a",
        `project-section${name === "Graph" ? " current" : ""}`,
        name,
      );
      link.href =
        projectPath(data.project_id) + (name === "Graph" ? "/graph" : "");
      link.dataset.route = "";
      if (name === "Graph") link.setAttribute("aria-current", "page");
      nav.append(link);
    } else {
      const item = el(
        "span",
        "project-section planned",
        `${name} · Phase ${phase}`,
      );
      item.setAttribute("aria-disabled", "true");
      nav.append(item);
    }
  }
  element.append(breadcrumb, header, nav);
  const runtimeDetails = el("section", "console-card graph-runtime-source");
  runtimeDetails.append(
    el("h2", "section-title", "Runtime realization evidence"),
  );
  const evidenceDetails = el("div", "graph-evidence-details");
  evidenceDetails.setAttribute("aria-live", "polite");
  const inspectEvidence = (e: RuntimeEvidence) => {
    evidenceDetails.replaceChildren(
      el("h3", "small fw-semibold", "Observed D-Serv realization evidence"),
    );
    for (const [label, value] of [
      [
        "Canonical observed subject",
        `${e.project_id} / ${e.application_id} / ${e.service_id} / ${e.node_id}`,
      ],
      ["Monitor epoch (not execution-instance ID)", e.monitor_instance_id],
      ["Reporting node (transport stream)", e.reporting_node_id],
      ["Observation ID", e.observation_id],
      ["Observation digest", e.observation_digest],
      ["Observed / associated node reference (not ownership)", e.node_id],
      ["Collector", e.collector],
      ["Source observation time", e.observed_at_utc],
      ["Server receipt time", e.received_at_utc],
      ["Collector health claim at observation time", e.observed_health_claim],
      [
        "Runtime kind / process / container / artifact identity",
        "Not included in this typed observation subject",
      ],
      [
        "Observed D-Map reference (not convergence)",
        e.dmap_ref
          ? `${e.dmap_ref.id} · revision ${e.dmap_ref.revision} · ${e.dmap_ref.digest}`
          : "Not included",
      ],
      [
        "Logical association",
        e.logical_node_id
          ? "Exact application/service identity in loaded accepted snapshot; not a current deployment/convergence verdict"
          : "Not matched to loaded accepted logical snapshots",
      ],
    ])
      evidenceDetails.append(
        el("p", "small entity-identity", `${label} · ${value}`),
      );
    if (e.resource_usage) {
      evidenceDetails.append(
        el("p", "small fw-semibold", "Container resource sample"),
        el("p", "small", `Current container CPU · ${e.resource_usage.cpu_percent.toFixed(1)}%`),
        el(
          "p",
          "small",
          `Current container memory · ${formatBytes(e.resource_usage.memory_used_bytes)} / ${formatBytes(e.resource_usage.memory_limit_bytes)}`,
        ),
        el("p", "small", `Current container PIDs · ${e.resource_usage.pids}`),
      );
    }
    // The client validates observation-ID uniqueness and positional evidence /
    // temporal correspondence before this view is created.
    const temporal = runtime?.temporal_quality.find(
      (t) => t.observation_id === e.observation_id,
    );
    if (temporal)
      for (const [label, value] of [
        ["Communication", temporal.communication_freshness],
        ["Observation", temporal.observation_freshness],
        ["Source time", temporal.source_time_quality],
      ])
        evidenceDetails.append(
          el("p", "small", `${label} · ${String(value).replaceAll("_", " ")}`),
        );
    evidenceDetails.append(
      el(
        "p",
        "small text-secondary",
        "An invocation can be realized at one instant without being a continuously running process. Evidence is not desired state, current health, convergence or resource reservation.",
      ),
    );
  };
  if (runtime)
    runtimeDetails.append(
      sectionValue(runtime.runtime, (value) => {
        const box = el("div");
        box.append(
          badge(
            value.completeness === "partial"
              ? "Partial evidence preview"
              : "Admitted evidence subjects included",
            value.completeness === "partial" ? "warning" : "info",
          ),
          el(
            "p",
            "small",
            `${value.included_evidence_count} of ${value.total_evidence_count} latest subject/monitor-epoch records included. Limit: 1,000; inclusion stops before the first service exceeding the budget, with whole-service omission.`,
          ),
          el(
            "p",
            "small text-secondary",
            "DMonitor observations, not an execution-instance registry. Independent monitor epochs are not distinct running processes. No loaded evidence does not mean stopped, failed or healthy.",
          ),
        );
        if (!value.evidence.length)
          box.append(
            el(
              "p",
              "empty-text",
              value.completeness === "partial"
                ? "No whole service evidence set fit inside this bounded preview."
                : "No admitted D-Serv realization evidence in this snapshot.",
            ),
          );
        const unmatched = value.evidence.filter((e) => !e.logical_node_id);
        if (unmatched.length) {
          box.append(
            el(
              "h3",
              "small fw-semibold",
              "Evidence not matched to loaded accepted logical snapshots",
            ),
          );
          for (const e of unmatched) {
            const button = el(
              "button",
              "btn btn-sm btn-outline-secondary entity-identity",
              `${e.application_id} / ${e.service_id} · ${e.observation_id}`,
            );
            button.type = "button";
            button.addEventListener("click", () => inspectEvidence(e));
            box.append(button);
          }
        }
        return box;
      }),
    );
  runtimeDetails.append(evidenceDetails);
  if (runtime) element.append(runtimeDetails);
  element.append(
    sectionValue(data.graph, (graph) => {
      const content = el("section", "console-card graph-structure");
      content.append(
        el("h2", "section-title", "Application structure"),
        badge(
          graph.coverage.completeness === "partial"
            ? "Partial accepted-snapshot coverage"
            : "Active accepted snapshots fully included",
          graph.coverage.completeness === "partial" ? "warning" : "info",
        ),
        el("p", "small text-secondary", graphCoverageText(graph)),
      );
      if (!graph.nodes.length) {
        content.append(
          el(
            "p",
            "empty-text",
            graph.coverage.completeness === "partial"
              ? "No accepted application snapshot fit inside the bounded graph preview. Source coverage is partial; this is not an empty project."
              : "No active accepted logical application graph in this snapshot. Other application definitions may exist outside this source.",
          ),
        );
        return content;
      }
      const toolbar = el("div", "graph-toolbar");
      toolbar.setAttribute("aria-label", "Read-only graph view controls");
      const fit = el("button", "btn btn-sm btn-outline-secondary", "Fit graph");
      fit.type = "button";
      const reset = el(
        "button",
        "btn btn-sm btn-outline-secondary",
        "Reset selection / view",
      );
      reset.type = "button";
      const filterLabel = el("label", "small", "Application snapshot ");
      const filter = el("select", "form-select form-select-sm graph-filter");
      filter.setAttribute("aria-label", "Application snapshot filter");
      const all = el("option", "", "All included application snapshots");
      all.value = "";
      filter.append(all);
      for (const app of graph.applications) {
        const option = el("option", "", app.application_id);
        option.value = app.application_id;
        filter.append(option);
      }
      filterLabel.append(filter);
      toolbar.append(fit, reset, filterLabel);
      content.append(toolbar);
      const grid = el("div", "graph-workspace-grid");
      const canvasArea = el("div", "graph-canvas-area");
      const canvas = el("div", "graph-canvas");
      canvas.setAttribute("role", "img");
      canvas.setAttribute(
        "aria-label",
        "Read-only logical application graph; use the accessible structure list for keyboard inspection.",
      );
      const rendererNotice = el(
        "p",
        "small text-secondary graph-renderer-notice",
      );
      canvasArea.append(canvas, rendererNotice);
      const details = el("section", "graph-details");
      details.setAttribute("aria-label", "Selected graph entity details");
      const detailBody = el("div", "graph-detail-body");
      detailBody.setAttribute("aria-live", "polite");
      details.append(el("h2", "section-title", "Selected entity"), detailBody);
      const clear = () => {
        stopLiveMetrics();
        detailBody.replaceChildren(
          el(
            "p",
            "small text-secondary",
            "Select an entity or D-Call in the graph or accessible list. Runtime resource samples are observation evidence only; no editing or deployment action is available.",
          ),
        );
      };
      clear();
      grid.append(canvasArea, details);
      content.append(grid);
      const select = (id: string) => {
        const observed = evidence.find(
          (e) => e.id === id || realizationEdgeId(e.id) === id,
        );
        if (observed) {
          stopLiveMetrics();
          inspectEvidence(observed);
          detailBody.replaceChildren(
            el(
              "p",
              "small entity-identity",
              `Realization evidence · ${observed.observation_id}`,
            ),
            el(
              "p",
              "small",
              "Typed evidence details are above the logical structure. The relationship means logical service → admitted realization evidence, not placement.",
            ),
          );
          cy?.elements().unselect();
          cy?.getElementById(id).select();
          return;
        }
        evidenceDetails.replaceChildren();
        const node = graph.nodes.find((n) => n.id === id),
          edge = graph.edges.find((e) => e.id === id);
        const entity = node ?? edge;
        if (!entity) return;
        const app = graph.applications.find(
          (a) => a.application_id === entity.application_id,
        )!;
        detailBody.replaceChildren(
          el(
            "h3",
            "small fw-semibold",
            node
              ? node.kind === "application"
                ? "Application grouping"
                : "D-Serv"
              : "D-Call",
          ),
        );
        for (const [label, value] of [
          ["Canonical ID", entity.canonical_id],
          ["Project context", data.project_id],
          ["Application", entity.application_id],
          ["D-Graph", app.dgraph_id],
          ["D-Graph revision", String(app.revision)],
          ["D-Graph digest", app.digest],
          ["Acceptance reference", app.acceptance_id ?? "Not included"],
        ])
          detailBody.append(
            el("p", "small entity-identity", `${label} · ${value}`),
          );
        if (node) {
          const counts = edgeCounts(graph, node.id);
          detailBody.append(
            el(
              "p",
              "small",
              `Incoming D-Calls · ${counts.incoming} / Outgoing D-Calls · ${counts.outgoing}`,
            ),
          );
          if (node.execution_abi !== null)
            detailBody.append(
              el(
                "p",
                "small entity-identity",
                `Declared execution ABI · ${node.execution_abi}`,
              ),
            );
          if (node.cpu_cores !== null)
            detailBody.append(
              el(
                "p",
                "small",
                `Declared minimum CPU cores · ${node.cpu_cores}`,
              ),
            );
          if (node.memory_bytes !== null)
            detailBody.append(
              el(
                "p",
                "small entity-identity",
                `Declared minimum memory bytes · ${node.memory_bytes}`,
              ),
            );
          if (node.kind === "dserv") {
            detailBody.append(
              el(
                "h3",
                "small fw-semibold mt-3",
                "Current runtime resource usage",
              ),
            );
            const panel = terminalMetricPanel(
              dservMetricState(
                node.id,
                node.application_id,
                node.canonical_id,
                runtime,
                live.fetchRuntime ? `LIVE · ${((live.pollMs ?? 2000) / 1000).toFixed((live.pollMs ?? 2000) % 1000 === 0 ? 0 : 1)}s` : undefined,
              ),
            );
            detailBody.append(panel.element);
            startLiveMetrics(
              node.id,
              node.application_id,
              node.canonical_id,
              panel,
            );
          } else {
            stopLiveMetrics();
          }
        } else if (edge) {
          const source = graph.nodes.find((n) => n.id === edge.source)!,
            target = graph.nodes.find((n) => n.id === edge.target)!;
          detailBody.append(
            el(
              "p",
              "small entity-identity",
              `Source D-Serv · ${source.canonical_id}`,
            ),
            el(
              "p",
              "small entity-identity",
              `Target D-Serv · ${target.canonical_id}`,
            ),
            el(
              "p",
              "small text-secondary",
              "Logical call direction, not deployment ordering or physical connectivity.",
            ),
          );
        }
        if (cy) {
          cy.elements().unselect();
          cy.getElementById(id).select();
        }
      };
      const list = el("section", "graph-accessible-list mt-3");
      list.append(
        el("h2", "section-title", "Accessible structure"),
        el(
          "p",
          "small text-secondary",
          "Keyboard-accessible alternative to canvas. Select an entity or relationship to inspect its full canonical identity.",
        ),
      );
      const listBody = el("div", "graph-entity-list");
      list.append(listBody);
      content.append(list);
      const populate = () => {
        listBody.replaceChildren();
        const current = filter.value || null;
        for (const app of graph.applications.filter(
          (a) => current === null || a.application_id === current,
        )) {
          const group = el("section", "graph-application-list");
          group.append(
            el(
              "h3",
              "small entity-identity",
              `Application · ${app.application_id}`,
            ),
            el(
              "p",
              "small entity-identity",
              `D-Graph · ${app.dgraph_id} · revision ${app.revision}`,
            ),
          );
          for (const entity of [
            ...graph.nodes.filter(
              (n) => n.application_id === app.application_id,
            ),
            ...graph.edges.filter(
              (e) => e.application_id === app.application_id,
            ),
          ]) {
            const button = el(
              "button",
              "graph-entity-button btn btn-sm btn-outline-secondary entity-identity",
              `${entity.kind === "dserv" ? "D-Serv" : entity.kind === "dcall" ? "D-Call" : "Application"} · ${entity.canonical_id}`,
            );
            button.type = "button";
            button.title = entity.canonical_id;
            button.dataset.entityId = entity.id;
            button.addEventListener("click", () => select(entity.id));
            group.append(button);
            if (entity.kind === "dserv" && runtime) {
              const toggle = el(
                "button",
                "btn btn-sm btn-outline-secondary graph-expand",
              );
              toggle.type = "button";
              toggle.dataset.serviceId = entity.id;
              toggle.setAttribute(
                "aria-expanded",
                String(expanded.has(entity.id)),
              );
              const associated = evidence.filter(
                (e) => e.logical_node_id === entity.id,
              );
              toggle.textContent =
                runtime.runtime.availability !== "available"
                  ? "Runtime source not available · inspect source state above"
                  : associated.length
                    ? `${expanded.has(entity.id) ? "Collapse" : "Expand"} ${associated.length} evidence record(s)`
                    : "No loaded realization evidence · no runtime state inferred";
              toggle.disabled = !associated.length;
              toggle.addEventListener("click", () => {
                if (expanded.has(entity.id)) expanded.delete(entity.id);
                else expanded.add(entity.id);
                clear();
                evidenceDetails.replaceChildren();
                populate();
                updateLegend();
                if (!disposed && element.isConnected) draw();
                // Reconstruction replaces the activated control. Restore focus by
                // application-qualified presentation identity, never label/order.
                const replacement = [...listBody.querySelectorAll<HTMLButtonElement>(
                  ".graph-expand",
                )].find((button) => button.dataset.serviceId === entity.id);
                replacement?.focus({ preventScroll: true });
              });
              group.append(toggle);
              if (expanded.has(entity.id))
                for (const e of associated) {
                  const entry = el(
                    "button",
                    "graph-entity-button btn btn-sm btn-outline-secondary entity-identity",
                    `Realization evidence · ${e.observation_id} · node reference ${e.node_id}`,
                  );
                  entry.type = "button";
                  entry.dataset.entityId = e.id;
                  entry.addEventListener("click", () => select(e.id));
                  group.append(entry);
                  const relation = el(
                    "button",
                    "graph-entity-button btn btn-sm btn-outline-secondary entity-identity",
                    `Realization relationship · ${entity.canonical_id} → ${e.observation_id}`,
                  );
                  relation.type = "button";
                  relation.addEventListener("click", () =>
                    select(realizationEdgeId(e.id)),
                  );
                  group.append(relation);
                }
            }
          }
          listBody.append(group);
        }
      };
      populate();
      const legend = el("section", "graph-legend mt-3");
      const legendItems = el("div", "graph-visible-legend");
      const updateLegend = () => {
        const current = filter.value || null;
        const visible = runtimeElements(graph, runtime, expanded, current);
        const hasRuntime = visible.some(
          (e) => e.data.kind === "runtime_evidence",
        );
        legendItems.replaceChildren(
          renderGraphLegend(
            graphLegendItemsFor(
              [
                ...(visible.some((e) => e.data.kind === "dserv")
                  ? ["software" as const]
                  : []),
                ...(hasRuntime ? ["runtime" as const] : []),
              ],
              [
                ...(visible.some((e) => e.data.kind === "dcall")
                  ? ["logical" as const]
                  : []),
                ...(hasRuntime ? ["realization" as const] : []),
              ],
              visible.some((e) => e.data.kind === "application"),
            ).map((item) =>
              item.id === "runtime"
                ? {
                    ...item,
                    label: "Runtime realization · evidence",
                    meaning:
                      "Admitted DServRealization observation, not an execution-instance registry",
                  }
                : item.id === "realization"
                  ? {
                      ...item,
                      label: "Realization relationship",
                      meaning:
                        "Logical service → admitted realization evidence; not placement",
                    }
                  : item,
            ),
          ),
        );
      };
      updateLegend();
      legend.append(
        el("h2", "section-title", "Legend / source"),
        legendItems,
        el(
          "p",
          "small text-secondary",
          "D-Code/artifacts and DIoT/D-Forward composition are separate contracts, not synthesized into this D-Graph. No D-Node, continuum, placement or health overlay.",
        ),
      );
      content.append(legend);
      const draw = () => {
        cy?.destroy();
        cy = undefined;
        try {
          if (!document.createElement("canvas").getContext("2d"))
            throw new Error("canvas unavailable");
          cy = cytoscape({
            container: canvas,
            elements: runtimeElements(
              graph,
              runtime,
              expanded,
              filter.value || null,
            ),
            minZoom: 0.15,
            maxZoom: 3,
            boxSelectionEnabled: false,
            layout: { name: "preset", fit: false },
            style: graphVisualStyles(),
          });
          layoutGroupedGraph(cy);
          cy.fit(undefined, 35);
          cy.on("tap", "node, edge", (e) => select(e.target.id()));
          rendererNotice.textContent =
            "Pan / zoom; dragging changes visual positions only and is never saved.";
        } catch {
          cy?.destroy();
          cy = undefined;
          rendererNotice.textContent =
            "Graph renderer unavailable in this browser. The accessible structure and details remain available.";
        }
      };
      fit.addEventListener("click", () => cy?.fit(undefined, 35));
      reset.addEventListener("click", () => {
        expanded.clear();
        evidenceDetails.replaceChildren();
        populate();
        updateLegend();
        clear();
        if (!disposed && element.isConnected) draw();
      });
      filter.addEventListener("change", () => {
        expanded.clear();
        evidenceDetails.replaceChildren();
        clear();
        populate();
        updateLegend();
        if (!disposed && element.isConnected) draw();
      });
      mount = () => {
        if (disposed) return;
        draw();
        if (typeof ResizeObserver !== "undefined") {
          resize = new ResizeObserver(() => cy?.resize());
          resize.observe(canvas);
        }
      };
      return content;
    }),
  );
  element.append(
    el(
      "p",
      "snapshot-note",
      `Generated ${data.generated_at_utc} · accepted snapshots, not globally atomic cross-source state.`,
    ),
  );
  return {
    element,
    mount: () => mount(),
    dispose: () => {
      disposed = true;
      stopLiveMetrics();
      resize?.disconnect();
      cy?.destroy();
      cy = undefined;
    },
  };
}

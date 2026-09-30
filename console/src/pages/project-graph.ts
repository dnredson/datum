import cytoscape from "cytoscape";
import type { Core } from "cytoscape";
import type { ProjectGraph } from "../types/graph.ts";
import { el, badge, sectionValue } from "../components/presentation.ts";
import { projectPath } from "../app/router.ts";
import { PROJECT_SECTIONS } from "./project-workspace.ts";
import {
  GRAPH_SOURCE_LABEL,
  graphElements,
  edgeCounts,
  layoutGroupedGraph,
  graphCoverageText,
} from "./graph-model.ts";

export function projectGraphView(data: ProjectGraph) {
  let cy: Core | undefined;
  let disposed = false;
  let resize: ResizeObserver | undefined;
  let mount = () => {};
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
  header.append(
    badge("PROJECT GRAPH · READ ONLY", "info"),
    el("h1", "", "Logical graph overview"),
    el("p", "entity-identity", data.project_id),
    el("p", "small text-secondary", GRAPH_SOURCE_LABEL),
    el(
      "p",
      "small text-secondary",
      "Only currently accepted application snapshots are shown. This is not a canonical application registry, operational deployment document, placement or observed runtime topology.",
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
        detailBody.replaceChildren(
          el(
            "p",
            "small text-secondary",
            "Select an entity or D-Call in the graph or accessible list. No editing, deployment or runtime status is exposed.",
          ),
        );
      };
      clear();
      grid.append(canvasArea, details);
      content.append(grid);
      const select = (id: string) => {
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
          }
          listBody.append(group);
        }
      };
      populate();
      const legend = el("section", "graph-legend mt-3");
      legend.append(
        el("h2", "section-title", "Legend / source"),
        el(
          "p",
          "small",
          "Application · presentation grouping of an accepted logical D-Graph snapshot. D-Serv · rounded rectangle, logical service. D-Call · directed arrow, logical call; cycles, reciprocal calls and self-calls are allowed. No deployment-order implication.",
        ),
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
            elements: graphElements(graph, filter.value || null),
            minZoom: 0.15,
            maxZoom: 3,
            boxSelectionEnabled: false,
            layout: { name: "preset", fit: false },
            style: [
              {
                selector: "node",
                style: {
                  "background-color": "#597a90",
                  shape: "round-rectangle",
                  label: "data(label)",
                  color: "#263847",
                  "font-size": 11,
                  "text-valign": "bottom",
                  "text-margin-y": 7,
                  "text-wrap": "wrap",
                  "text-max-width": "140px",
                  width: 48,
                  height: 34,
                },
              },
              {
                selector: 'node[kind = "application"]',
                style: {
                  "background-color": "#e8eef2",
                  "border-color": "#8199aa",
                  "border-width": 1,
                  shape: "round-rectangle",
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
                  "line-color": "#7f929e",
                  "target-arrow-color": "#7f929e",
                  "target-arrow-shape": "triangle",
                  "curve-style": "bezier",
                  label: "data(label)",
                  "font-size": 9,
                  color: "#425869",
                },
              },
              {
                selector: ":selected",
                style: {
                  "border-color": "#244f70",
                  "border-width": 3,
                  "line-color": "#244f70",
                  "target-arrow-color": "#244f70",
                },
              },
            ],
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
        clear();
        cy?.elements().unselect();
        if (cy) layoutGroupedGraph(cy);
        cy?.fit(undefined, 35);
      });
      filter.addEventListener("change", () => {
        clear();
        populate();
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
      resize?.disconnect();
      cy?.destroy();
      cy = undefined;
    },
  };
}

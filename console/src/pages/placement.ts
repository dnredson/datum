// Phase 136 -- Placement Workspace. Lets an operator inspect the current
// accepted placement for an application, generate a deterministic
// candidate, locally edit it (service -> node reassignment only, never
// drag-and-drop, never a D-Graph structure edit), validate it, and create
// a Pending proposal. NEVER calls accept -- Phase 137 (Review/Acceptance)
// owns that, exclusively. See
// documentation/reference/phase136-placement-ui.md for the full authority
// boundary this page must never contradict: planned != proposed !=
// accepted.
import cytoscape from "cytoscape";
import type { Core, ElementDefinition, StylesheetJson } from "cytoscape";
import { el, badge } from "../components/presentation.ts";
import type { Tone } from "../components/presentation.ts";
import { GRAPH_CATEGORIES, GRAPH_RELATIONS, graphVisualStyles } from "../components/graph-visuals.ts";
import type { DGraphEditorClient, GraphEntry } from "../api/dgraph-editor.ts";
import { EditorError } from "../api/dgraph-editor.ts";
import { PlacementClient, PlacementError } from "../api/placement.ts";
import { placementDiff } from "./placement-model.ts";
import type { PlacementDiffEntry } from "./placement-model.ts";
import { proposalReviewPath } from "../app/router.ts";
import type { OperationsClient } from "../api/client.ts";
import type { ContinuumNode, ContinuumView } from "../types/continuum.ts";
import type { CandidateResponse, PlacementRef, ActiveDeploymentSummary } from "../types/placement.ts";

function bytes(value: number): string {
  if (value >= 1_073_741_824) return `${(value / 1_073_741_824).toFixed(1)} GiB`;
  if (value >= 1_048_576) return `${(value / 1_048_576).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}

// Exported for reuse by Phase 137 (proposal-review.ts) -- the diff badge
// vocabulary must stay identical wherever a PlacementDiffEntry is shown.
export const CHANGE_LABEL: Record<PlacementDiffEntry["change"], string> = {
  unchanged: "Unchanged",
  moved: "Moved",
  added: "Added",
  removed: "Removed",
};
export const CHANGE_TONE: Record<PlacementDiffEntry["change"], Tone> = {
  unchanged: "neutral",
  moved: "warning",
  added: "success",
  removed: "danger",
};

// ----------------------------------------------------------- CANVAS ELEMENTS

// Pure, exported for direct testing. D-Nodes reuse Phase 135's existing
// `infrastructure` category verbatim. One software-category node per
// service appearing in EITHER the current accepted placement or the local
// candidate draft (never one per node -- a service has at most one
// placement at a time on each side). Edges distinguish state visually:
// unchanged reuses the EXISTING accepted `placement` relation untouched;
// a moved service shows BOTH its old (dimmed, `placement_superseded`) and
// new (`placement_candidate`) edge simultaneously; an added service shows
// only the new edge; a removed service shows only the old one. No new
// token is added to the shared graph-visuals.ts -- `placementOverlayStyles()`
// below is local and additive only, exactly mirroring Phase 135A's
// `observationOverlayStyles()` precedent.
export function placementElements(nodes: ContinuumNode[], diffEntries: PlacementDiffEntry[]): ElementDefinition[] {
  const elements: ElementDefinition[] = [];
  for (const node of nodes) {
    elements.push({ data: { id: node.node_id, label: node.node_id, visual_category: "infrastructure", kind: "dnode" } });
  }
  const knownNodeIds = new Set(nodes.map((n) => n.node_id));
  for (const entry of diffEntries) {
    const serviceNodeId = `service:${entry.service_id}`;
    elements.push({
      data: {
        id: serviceNodeId,
        label: entry.service_id,
        visual_category: "software",
        kind: "placement_draft",
        placement_state: entry.change,
      },
    });
    if (entry.from_node_id && knownNodeIds.has(entry.from_node_id) && entry.change !== "unchanged") {
      elements.push({
        data: {
          id: `${serviceNodeId}::superseded`,
          source: serviceNodeId,
          target: entry.from_node_id,
          visual_relation: "placement_superseded",
          kind: "placement_superseded",
        },
      });
    }
    if (entry.to_node_id && knownNodeIds.has(entry.to_node_id)) {
      elements.push({
        data: {
          id: `${serviceNodeId}::current`,
          source: serviceNodeId,
          target: entry.to_node_id,
          visual_relation: entry.change === "unchanged" ? "placement" : "placement_candidate",
          kind: "placement",
        },
      });
    }
  }
  return elements;
}

function placementOverlayStyles(): StylesheetJson {
  return [
    { selector: 'edge[visual_relation = "placement_candidate"]', style: { "line-color": "#6f42c1", "target-arrow-color": "#6f42c1", "line-style": "dashed", "target-arrow-shape": "triangle", width: 2.5 } },
    { selector: 'edge[visual_relation = "placement_superseded"]', style: { "line-color": "#9aa5b1", "target-arrow-color": "#9aa5b1", "line-style": "dotted", "target-arrow-shape": "none", opacity: 0.6 } },
    { selector: 'node[placement_state = "added"]', style: { "border-color": "#6f42c1", "border-width": 3, "border-style": "solid" } },
    { selector: 'node[placement_state = "moved"]', style: { "border-color": "#c5a52b", "border-width": 3, "border-style": "solid" } },
    { selector: 'node[placement_state = "removed"]', style: { "border-color": "#b33d48", "border-width": 2, "border-style": "dotted" } },
  ];
}

export interface PlacementPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
  dirty: () => boolean;
  confirmLeave: () => boolean;
}

export function placementView(
  projectId: string,
  placementClient: PlacementClient,
  dgraphEditorClient: DGraphEditorClient,
  operationsClient: OperationsClient,
  signal: AbortSignal,
  onReady: (ready: boolean) => void = () => {},
): PlacementPageView {
  let disposed = false;
  let cy: Core | undefined;
  let resize: ResizeObserver | undefined;
  let busy = false;
  let sequence = 0;

  let bases: GraphEntry[] = [];
  let continuumNodes: ContinuumNode[] = [];
  let selected: GraphEntry | undefined;
  let activeDeployment: ActiveDeploymentSummary | undefined;
  let candidate: CandidateResponse | undefined;
  let draftPlacements: PlacementRef[] = [];
  let candidateEdited = false;

  const dirty = () => draftPlacements.length > 0 && JSON.stringify(draftPlacements) !== JSON.stringify(candidate?.placements ?? []);
  const confirmLeave = () => !dirty() || window.confirm("Discard local placement candidate edits? Nothing has been proposed or accepted.");

  const root = el("div", "page-content placement-view");
  const header = el("header", "page-intro");
  header.append(
    badge("PLACEMENT · DMAP/1 ONLY · NOT ACCEPTANCE", "info"),
    el("h1", "", "Placement"),
    el(
      "p",
      "small text-secondary",
      "Generate, edit and validate a deterministic placement candidate. Creating a proposal persists it as Pending only -- it never becomes active authority here. Acceptance/review is a separate, later step.",
    ),
  );
  root.append(header);

  const picker = el("form", "placement-picker");
  picker.setAttribute("role", "search");
  const pickerLabel = el("label", "small", "Application");
  const pickerSelect = el("select", "form-select form-select-sm");
  pickerSelect.id = "placement-application-picker";
  pickerLabel.htmlFor = pickerSelect.id;
  picker.append(pickerLabel, pickerSelect);
  root.append(picker);

  const status = el("p", "placement-status");
  status.setAttribute("role", "status");
  root.append(status);

  const grid = el("div", "placement-workspace-grid");
  const canvasArea = el("div", "placement-canvas-area");
  const canvas = el("div", "placement-canvas");
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", "Read-only visual overview of D-Nodes and the placement candidate; use the accessible list below for keyboard editing.");
  const rendererNotice = el("p", "small text-secondary placement-renderer-notice");
  canvasArea.append(canvas, rendererNotice);
  const inspector = el("section", "placement-inspector");
  inspector.setAttribute("aria-label", "Selected service or D-Node detail");
  const inspectorBody = el("div", "placement-inspector-body");
  inspectorBody.setAttribute("aria-live", "polite");
  inspector.append(el("h2", "section-title", "Inspector"), inspectorBody);
  grid.append(canvasArea, inspector);
  root.append(grid);

  const diffSection = el("section", "placement-diff");
  diffSection.append(el("h2", "h6", "Current vs. candidate diff"));
  const diffList = el("ul", "placement-diff-list");
  diffSection.append(diffList);
  root.append(diffSection);

  const list = el("div", "placement-service-list");
  root.append(list);

  const actions = el("div", "placement-actions");
  const validateButton = el("button", "btn btn-outline-secondary", "Validate candidate");
  validateButton.type = "button";
  const createButton = el("button", "btn btn-primary", "Create proposal");
  createButton.type = "button";
  actions.append(validateButton, createButton);
  root.append(actions);

  const findings = el("section", "placement-findings");
  findings.tabIndex = -1;
  root.append(findings);

  function clearInspector(): void {
    inspectorBody.replaceChildren(el("p", "small text-secondary", "Select a service or D-Node in the canvas or the list below to inspect it."));
  }
  clearInspector();

  function setStatus(text: string): void {
    status.textContent = text;
  }

  function currentPlacements(): PlacementRef[] {
    return activeDeployment?.active_dmap.placements ?? [];
  }

  function diffEntries(): PlacementDiffEntry[] {
    return placementDiff(currentPlacements(), draftPlacements);
  }

  function nodeOptions(selectEl: HTMLSelectElement, currentValue: string): void {
    selectEl.replaceChildren();
    const unplaced = new Option("Unplaced", "");
    selectEl.append(unplaced);
    for (const node of [...continuumNodes].sort((a, b) => a.node_id.localeCompare(b.node_id))) {
      const compatible = node.structure.dnode_abi_versions.length === 0 || true; // ABI hint only via inspector text, never pre-filtered here.
      const option = new Option(`${node.node_id} (${node.structure.capacity.cpu_cores} cores, ${bytes(node.structure.capacity.memory_bytes)})`, node.node_id);
      void compatible;
      selectEl.append(option);
    }
    selectEl.value = currentValue;
  }

  function inspectService(serviceId: string): void {
    const service = selected?.graph.services.find((s) => s.service_id === serviceId);
    const entry = diffEntries().find((d) => d.service_id === serviceId);
    const container = el("div", "placement-service-detail");
    container.append(el("h3", "h6 mb-0", `DServ ${serviceId}`));
    if (entry) container.append(badge(CHANGE_LABEL[entry.change], CHANGE_TONE[entry.change]));

    container.append(el("h4", "h6 small text-uppercase text-secondary mt-3", "Requirements"));
    const requirements = el("dl", "placement-structure-grid");
    const add = (label: string, value: string) => requirements.append(el("dt", "small text-secondary", label), el("dd", "small", value));
    if (service) {
      add("D-Node ABI", service.execution_requirement.dnode_abi);
      add("Required CPU (cores)", String(service.resource_requirement.cpu_cores));
      add("Required memory", bytes(Number(service.resource_requirement.memory_bytes)));
    } else {
      requirements.append(el("dd", "small text-secondary", "Service requirements unavailable -- accepted D-Graph not loaded."));
    }
    container.append(requirements);

    const currentHeading = el("h4", "h6 small text-uppercase text-secondary mt-3", "Current accepted target");
    currentHeading.style.color = GRAPH_RELATIONS.placement.color;
    container.append(currentHeading, el("p", "small", entry?.from_node_id ?? "No accepted placement."));

    const candidateHeading = el("h4", "h6 small text-uppercase text-secondary mt-3", "Candidate target");
    container.append(candidateHeading);
    const reassign = el("select", "form-select form-select-sm");
    reassign.setAttribute("aria-label", `Candidate target D-Node for ${serviceId}`);
    nodeOptions(reassign, entry?.to_node_id ?? "");
    reassign.addEventListener("change", () => {
      draftPlacements = draftPlacements.filter((p) => p.service_id !== serviceId);
      if (reassign.value) draftPlacements.push({ service_id: serviceId, node_id: reassign.value });
      candidateEdited = true;
      render();
      inspectService(serviceId);
    });
    container.append(reassign);
    inspectorBody.replaceChildren(container);
  }

  function inspectNode(nodeId: string): void {
    const node = continuumNodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    const container = el("div", "placement-node-detail");
    const swatch = el("span", "placement-category-swatch");
    swatch.style.backgroundColor = GRAPH_CATEGORIES.infrastructure.color;
    swatch.setAttribute("aria-hidden", "true");
    const header2 = el("div", "placement-node-header");
    header2.append(swatch, el("h3", "h6 mb-0", node.node_id));
    container.append(header2);

    container.append(el("h4", "h6 small text-uppercase text-secondary mt-3", "Structure"));
    const structure = el("dl", "placement-structure-grid");
    structure.append(
      el("dt", "small text-secondary", "CPU cores (declared)"),
      el("dd", "small", String(node.structure.capacity.cpu_cores)),
      el("dt", "small text-secondary", "Memory (declared)"),
      el("dd", "small", bytes(node.structure.capacity.memory_bytes)),
      el("dt", "small text-secondary", "D-Node ABI"),
      el("dd", "small", node.structure.dnode_abi_versions.join(", ") || "None declared"),
    );
    container.append(structure);

    container.append(el("h4", "h6 small text-uppercase text-secondary mt-3", "Candidate services targeting this node"));
    const targeting = diffEntries().filter((d) => d.to_node_id === nodeId);
    if (!targeting.length) container.append(el("p", "small text-secondary", "No candidate placement currently targets this node."));
    else {
      const ul = el("ul", "placement-placement-list");
      for (const t of targeting) ul.append(el("li", "small", `${t.service_id} (${CHANGE_LABEL[t.change]})`));
      container.append(ul);
    }

    if (node.observations.length) {
      container.append(el("h4", "h6 small text-uppercase text-secondary mt-3", "DMonitor observation (contextual only)"));
      container.append(el("p", "small text-secondary", "Observed health does not affect placement eligibility -- shown for context only."));
      const ul = el("ul", "placement-evidence-list");
      for (const observation of node.observations) {
        ul.append(
          el(
            "li",
            "small",
            `${observation.service_id}: ${observation.placement_convergence}${observation.admitted_evidence[0] ? ` · ${observation.admitted_evidence[0].health}` : ""}`,
          ),
        );
      }
      container.append(ul);
    }
    inspectorBody.replaceChildren(container);
  }

  function renderDiff(): void {
    diffList.replaceChildren();
    const entries = diffEntries();
    if (!entries.length) {
      diffList.append(el("li", "small text-secondary", "No services to place -- the selected application's D-Graph declares none."));
      return;
    }
    for (const entry of entries) {
      const li = el("li", "placement-diff-row");
      li.append(
        el("span", "small entity-identity", entry.service_id),
        badge(CHANGE_LABEL[entry.change], CHANGE_TONE[entry.change]),
        el("span", "small text-secondary", `${entry.from_node_id ?? "unplaced"} → ${entry.to_node_id ?? "unplaced"}`),
      );
      diffList.append(li);
    }
  }

  function renderList(): void {
    list.replaceChildren();
    if (!selected) {
      list.append(el("p", "text-secondary", "Choose an application to inspect its placement."));
      return;
    }
    list.append(el("h2", "h6", "Services (accessible fallback)"));
    for (const entry of diffEntries()) {
      const card = el("section", "placement-service-card console-card");
      card.tabIndex = 0;
      card.setAttribute("aria-label", `DServ ${entry.service_id}`);
      card.addEventListener("click", () => {
        select({ kind: "service", id: entry.service_id });
      });
      card.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          select({ kind: "service", id: entry.service_id });
        }
      });
      card.append(
        el("span", "small entity-identity", entry.service_id),
        badge(CHANGE_LABEL[entry.change], CHANGE_TONE[entry.change]),
        el("span", "small text-secondary", `${entry.from_node_id ?? "unplaced"} → ${entry.to_node_id ?? "unplaced"}`),
      );
      list.append(card);
    }
  }

  function select(target: { kind: "service" | "node"; id: string }): void {
    if (target.kind === "service") inspectService(target.id);
    else inspectNode(target.id);
    cy?.elements().unselect();
    cy?.getElementById(target.kind === "service" ? `service:${target.id}` : target.id).select();
  }

  function draw(): void {
    cy?.destroy();
    cy = undefined;
    try {
      if (!document.createElement("canvas").getContext("2d")) throw new Error("canvas unavailable");
      cy = cytoscape({
        container: canvas,
        elements: placementElements(continuumNodes, diffEntries()),
        minZoom: 0.15,
        maxZoom: 3,
        boxSelectionEnabled: false,
        layout: { name: "breadthfirst", directed: true, spacingFactor: 1.2 },
        style: [...graphVisualStyles(), ...placementOverlayStyles()],
      });
      cy.fit(undefined, 35);
      cy.on("tap", "node", (e) => {
        const data = e.target.data();
        if (data.kind === "dnode") select({ kind: "node", id: e.target.id() });
        else if (data.kind === "placement_draft") select({ kind: "service", id: String(e.target.id()).replace(/^service:/, "") });
      });
      rendererNotice.textContent = "Pan / zoom; node position is visual only. Dashed purple edges are the local candidate; dotted gray edges show a service's prior accepted target.";
    } catch {
      cy?.destroy();
      cy = undefined;
      rendererNotice.textContent = "Graph renderer unavailable in this browser. The accessible list below remains fully available.";
    }
  }

  function render(): void {
    renderDiff();
    renderList();
    if (cy) draw();
  }

  async function loadApplications(): Promise<void> {
    if (!confirmLeave()) return;
    setStatus("Reading accepted application D-Graphs…");
    try {
      bases = await dgraphEditorClient.load();
      pickerSelect.replaceChildren(new Option("Choose an application", ""));
      for (const entry of bases) pickerSelect.append(new Option(`${entry.application_id} · ${entry.dgraph_id}@${entry.revision}`, entry.application_id));
      setStatus(bases.length ? "Choose an application to begin." : "No accepted applications exist in this project yet.");
      if (!disposed) onReady(true);
    } catch (e) {
      setStatus("Accepted applications unavailable.");
      notice(e instanceof EditorError ? e.message : "Accepted applications could not be read.");
      if (!disposed) onReady(false);
    }
  }

  async function loadContinuum(): Promise<void> {
    try {
      const view: ContinuumView = await operationsClient.projectContinuum(projectId, signal);
      continuumNodes = view.nodes;
    } catch {
      continuumNodes = [];
      notice("Physical D-Node inventory unavailable. The canvas and node picker will be empty until this is retried.");
    }
  }

  function notice(text: string): void {
    findings.replaceChildren(el("h2", "h6", "Notice"), el("p", "small", text));
    findings.focus();
  }

  async function selectApplication(applicationId: string): Promise<void> {
    if (!confirmLeave()) {
      pickerSelect.value = selected?.application_id ?? "";
      return;
    }
    const entry = bases.find((b) => b.application_id === applicationId);
    if (!entry) return;
    selected = entry;
    activeDeployment = undefined;
    candidate = undefined;
    draftPlacements = [];
    candidateEdited = false;
    findings.replaceChildren();
    const token = ++sequence;
    setStatus(`Loading current placement and generating a candidate for ${applicationId}…`);
    try {
      const [active, generated] = await Promise.all([
        placementClient.active(applicationId),
        placementClient.candidate(applicationId, entry.graph),
      ]);
      if (disposed || token !== sequence) return;
      activeDeployment = active.deployments[0];
      candidate = generated;
      draftPlacements = generated.placements.map((p) => ({ ...p }));
      setStatus(
        activeDeployment
          ? `Current accepted D-Map revision ${activeDeployment.active_dmap.revision}. Candidate generated by ${generated.planner_provenance.policy_name}@${generated.planner_provenance.policy_version}.`
          : `No accepted placement yet for ${applicationId} (first deployment). Candidate generated by ${generated.planner_provenance.policy_name}@${generated.planner_provenance.policy_version}.`,
      );
      render();
      clearInspector();
    } catch (e) {
      if (disposed || token !== sequence) return;
      if (e instanceof PlacementError && e.code === "candidate_dmap_invalid") {
        activeDeployment = (await placementClient.active(applicationId).catch(() => undefined))?.deployments[0];
        draftPlacements = [];
        candidate = undefined;
        setStatus("The deterministic planner could not produce a candidate for the current authoritative state.");
        notice(e.findings.map((f) => `${f.code}: ${f.message}`).join(" · ") || e.message);
        render();
      } else {
        setStatus("Candidate generation unavailable.");
        notice(e instanceof PlacementError ? `${e.message} (${e.code})` : "Candidate generation failed unexpectedly.");
      }
    }
  }

  validateButton.addEventListener("click", () => {
    if (busy || !selected) return;
    busy = true;
    validateButton.disabled = true;
    const token = sequence;
    setStatus("Validating candidate on DServer — no persistence…");
    void placementClient
      .validateCandidate(selected.application_id, selected.graph, draftPlacements, candidateEdited ? "operator" : "deterministic")
      .then((result) => {
        if (disposed || token !== sequence) return;
        findings.replaceChildren(
          el("h2", "h6", result.valid ? "Candidate structurally valid — NOT accepted" : "Candidate validation failed"),
          el("p", "small text-secondary", result.capacity_check_is_momentary),
        );
        for (const f of result.findings) findings.append(el("p", "small", `${f.code}: ${f.message}`));
        findings.focus();
        setStatus("Validation complete. Nothing was proposed, accepted, or reserved.");
      })
      .catch((e) => {
        if (disposed || token !== sequence) return;
        notice(e instanceof PlacementError ? `${e.message} (${e.code})` : "Validation unavailable.");
      })
      .finally(() => {
        busy = false;
        if (!disposed) validateButton.disabled = false;
      });
  });

  createButton.addEventListener("click", () => {
    if (busy || !selected) return;
    busy = true;
    createButton.disabled = true;
    const token = sequence;
    setStatus("Creating a Pending proposal — this does not accept or activate anything…");
    void placementClient
      .createProposal(selected.application_id, selected.graph, draftPlacements, candidateEdited ? "operator" : "deterministic")
      .then((result) => {
        if (disposed || token !== sequence) return;
        const reviewLink = el("a", "btn btn-outline-primary btn-sm", "Review proposal");
        reviewLink.setAttribute("href", proposalReviewPath(projectId, result.proposal.proposal_id));
        reviewLink.dataset.route = "";
        findings.replaceChildren(
          el("h2", "h6", "Proposal created — Pending, not accepted"),
          el("p", "small entity-identity", `proposal_id: ${result.proposal.proposal_id}`),
          el("p", "small text-secondary", "This proposal is not active authority. A separate, later review/acceptance step decides whether it becomes active."),
          reviewLink,
        );
        findings.focus();
        setStatus(`Proposal ${result.proposal.proposal_id} is Pending.`);
      })
      .catch((e) => {
        if (disposed || token !== sequence) return;
        if (e instanceof PlacementError && e.findings.length) {
          notice(`Proposal rejected -- not persisted: ${e.findings.map((f) => `${f.code}: ${f.message}`).join(" · ")}`);
        } else {
          notice(e instanceof PlacementError ? `${e.message} (${e.code})` : "Proposal creation unavailable.");
        }
      })
      .finally(() => {
        busy = false;
        if (!disposed) createButton.disabled = false;
      });
  });

  pickerSelect.addEventListener("change", () => {
    if (pickerSelect.value) void selectApplication(pickerSelect.value);
  });

  void (async () => {
    await Promise.all([loadApplications(), loadContinuum()]);
  })();

  return {
    element: root,
    mount: () => {
      if (disposed) return;
      draw();
      if (typeof ResizeObserver !== "undefined") {
        resize = new ResizeObserver(() => cy?.resize());
        resize.observe(canvas);
      }
    },
    dispose: () => {
      disposed = true;
      sequence++;
      resize?.disconnect();
      cy?.destroy();
      cy = undefined;
    },
    dirty,
    confirmLeave,
  };
}

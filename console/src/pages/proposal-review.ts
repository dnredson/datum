// Phase 137 -- Proposal Review / Acceptance. A single-proposal detail page:
// read a Pending or Accepted proposal by id, show it read-only against the
// LIVE accepted authority (never proposal-creation-time state), and -- for
// a Pending proposal only -- expose the one authority transition this
// phase owns: POST .../accept. Accepting changes placement AUTHORITY ONLY.
// It never deploys, reconciles runtime, invokes DCode, mutates DMonitor,
// or proves runtime convergence. There is no Reject/Cancel/Withdraw --
// proposal lifecycle is exactly Pending/Accepted. See
// documentation/reference/phase137-proposal-review-acceptance.md.
import { el, badge } from "../components/presentation.ts";
import { PlacementClient, PlacementError } from "../api/placement.ts";
import { placementDiff } from "./placement-model.ts";
import type { PlacementDiffEntry } from "./placement-model.ts";
import { CHANGE_LABEL, CHANGE_TONE } from "./placement.ts";
import { projectPath, applicationDeployPath } from "../app/router.ts";
import type { ProposalResponse, ActiveDeploymentSummary, AcceptResponse, PlacementRef } from "../types/placement.ts";

export interface ProposalReviewPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
  dirty: () => boolean;
  confirmLeave: () => boolean;
}

const SOURCE_LABEL: Record<string, string> = {
  operator: "Operator",
  deterministic: "Deterministic planner",
  ai: "AI",
};

// Ambiguous-transport PlacementError codes -- the HTTP response itself was
// never received, so the acceptance's outcome is genuinely unknown (it may
// have been applied anyway). Every other code means a real HTTP response
// came back, so the outcome IS known even when it's a rejection.
const AMBIGUOUS_CODES = new Set(["timeout", "unavailable"]);

export function proposalReviewView(
  projectId: string,
  proposalId: string,
  placementClient: PlacementClient,
  onReady: (ready: boolean) => void = () => {},
): ProposalReviewPageView {
  let disposed = false;
  let sequence = 0;
  let proposal: ProposalResponse | undefined;
  let activeDeployment: ActiveDeploymentSummary | undefined;
  let activeLoaded = false;
  let accepting = false;

  const root = el("div", "page-content proposal-review-view");
  const header = el("header", "page-intro");
  header.append(
    badge("PROPOSAL REVIEW · AUTHORITY ONLY · NOT DEPLOYMENT", "info"),
    el("h1", "", "Proposal review"),
    el(
      "p",
      "small text-secondary",
      "Accepting a proposal changes which placement is active authority for this application. It does not deploy, reconcile runtime, invoke DCode, or mutate DMonitor, and does not prove runtime convergence.",
    ),
  );
  root.append(header);

  const status = el("p", "proposal-review-status");
  status.setAttribute("role", "status");
  root.append(status);

  const body = el("div", "proposal-review-body");
  root.append(body);

  const auditLabelInput = el("input", "form-control form-control-sm proposal-review-audit-input");
  auditLabelInput.setAttribute("type", "text");
  auditLabelInput.setAttribute("placeholder", "e.g. operator initials, ticket id");
  const acceptButton = el("button", "btn btn-primary", "Accept proposal");
  acceptButton.setAttribute("type", "button");
  const acceptOutcome = el("div", "proposal-review-accept-outcome");
  acceptOutcome.setAttribute("role", "status");
  acceptOutcome.tabIndex = -1;

  function setStatus(text: string): void {
    status.textContent = text;
  }

  function setAcceptNotice(lines: string[]): void {
    acceptOutcome.replaceChildren();
    for (const line of lines) acceptOutcome.append(el("p", "small mb-1", line));
    acceptOutcome.focus();
  }

  function currentPlacements(): PlacementRef[] {
    return activeDeployment?.active_dmap.placements ?? [];
  }

  function diffEntries(): PlacementDiffEntry[] {
    return proposal ? placementDiff(currentPlacements(), proposal.proposal.placements) : [];
  }

  function identitySection(): HTMLElement {
    const p = proposal!;
    const section = el("section", "proposal-review-identity console-card");
    section.append(el("h2", "h6", "Proposal"));
    section.append(badge(p.state === "accepted" ? "Accepted" : "Pending", p.state === "accepted" ? "success" : "warning"));
    const dl = el("dl", "proposal-review-grid");
    const add = (label: string, value: string) => dl.append(el("dt", "small text-secondary", label), el("dd", "small entity-identity", value));
    add("proposal_id", p.proposal.proposal_id);
    add("project_id", p.proposal.project_id);
    add("application_id", p.proposal.application_id);
    add("source", SOURCE_LABEL[p.proposal.source] ?? p.proposal.source);
    add("created_at_utc", p.proposal.created_at_utc);
    add("proposal_digest", p.proposal_digest);
    section.append(dl);
    section.append(
      el(
        "p",
        "small text-secondary mt-2",
        "This proposal's content is immutable and cannot be edited here. The only action available on this page is acceptance.",
      ),
    );
    return section;
  }

  function referencesSection(): HTMLElement {
    const p = proposal!;
    const section = el("section", "proposal-review-references console-card");
    section.append(el("h2", "h6", "References"));
    const dl = el("dl", "proposal-review-grid");
    const add = (label: string, value: string) => dl.append(el("dt", "small text-secondary", label), el("dd", "small entity-identity", value));
    add("DGraph ref", `${p.proposal.dgraph_ref.id}@${p.proposal.dgraph_ref.revision} (${p.proposal.dgraph_ref.digest})`);
    add("DContinuum ref", `${p.proposal.dcontinuum_ref.id}@${p.proposal.dcontinuum_ref.revision} (${p.proposal.dcontinuum_ref.digest})`);
    add(
      "expected_active_dmap_revision",
      p.proposal.expected_active_dmap_revision === null ? "none (first-deployment basis)" : String(p.proposal.expected_active_dmap_revision),
    );
    section.append(dl);
    return section;
  }

  function authoritySection(): HTMLElement {
    const section = el("section", "proposal-review-authority console-card");
    section.append(el("h2", "h6", "Current accepted authority (live)"));
    if (!activeLoaded) {
      section.append(el("p", "small text-secondary", "Loading current authoritative placement…"));
      return section;
    }
    if (!activeDeployment) {
      section.append(badge("No active D-Map yet", "info"));
      section.append(
        el(
          "p",
          "small text-secondary",
          "This application has no accepted placement yet. Accepting this proposal would be its first deployment -- a normal state, not an error.",
        ),
      );
      return section;
    }
    const dl = el("dl", "proposal-review-grid");
    const add = (label: string, value: string) => dl.append(el("dt", "small text-secondary", label), el("dd", "small entity-identity", value));
    add("Active revision", String(activeDeployment.active_dmap.revision));
    add("acceptance_id", activeDeployment.acceptance_id);
    add("accepted_at_utc", activeDeployment.accepted_at_utc);
    add("operational_dgraph_id", `${activeDeployment.operational_dgraph_id}@${activeDeployment.operational_revision}`);
    section.append(dl);
    const expected = proposal?.proposal.expected_active_dmap_revision ?? null;
    if (proposal?.state === "pending" && expected !== null && expected !== activeDeployment.active_dmap.revision) {
      const staleBanner = el("p", "proposal-review-stale-banner");
      staleBanner.setAttribute("role", "alert");
      staleBanner.append(
        badge("Stale basis", "warning"),
        el(
          "span",
          "small",
          ` This proposal was based on active D-Map revision ${expected}, but the current active revision is ${activeDeployment.active_dmap.revision}. DServer revalidates authoritative state at acceptance time and will reject acceptance while this remains stale -- this page never silently rebases the proposal.`,
        ),
      );
      section.append(staleBanner);
    }
    return section;
  }

  function diffSection(): HTMLElement {
    const p = proposal!;
    const section = el("section", "proposal-review-diff console-card");
    const label = p.state === "pending" ? "Pending proposal" : "Accepted proposal";
    section.append(el("h2", "h6", `Placement diff — current authority vs. ${label}`));
    const entries = diffEntries();
    if (!entries.length) {
      section.append(el("p", "small text-secondary", "This proposal declares no placements."));
      return section;
    }
    // `table-responsive` (Bootstrap) scopes any horizontal overflow to this
    // scroller alone -- a long service_id/node_id must never force the
    // whole page to scroll sideways at tablet widths.
    const scroller = el("div", "table-responsive");
    const table = el("table", "table table-sm proposal-review-diff-table");
    const thead = el("thead");
    const headRow = el("tr");
    for (const columnLabel of ["Service", "Current", "Proposed", "Change"]) headRow.append(el("th", "small", columnLabel));
    thead.append(headRow);
    const tbody = el("tbody");
    for (const entry of entries) {
      const row = el("tr");
      const changeCell = el("td", "small");
      changeCell.append(badge(CHANGE_LABEL[entry.change], CHANGE_TONE[entry.change]));
      row.append(
        el("td", "small entity-identity", entry.service_id),
        el("td", "small entity-identity", entry.from_node_id ?? "unplaced"),
        el("td", "small entity-identity", entry.to_node_id ?? "unplaced"),
        changeCell,
      );
      tbody.append(row);
    }
    table.append(thead, tbody);
    scroller.append(table);
    section.append(scroller);
    return section;
  }

  function provenanceSection(): HTMLElement {
    const p = proposal!;
    const section = el("section", "proposal-review-provenance console-card");
    section.append(el("h2", "h6", "Source / provenance"));
    section.append(el("p", "small", `source: ${SOURCE_LABEL[p.proposal.source] ?? p.proposal.source}`));
    if (p.proposal.source === "deterministic") {
      section.append(
        el(
          "p",
          "small text-secondary",
          "Detailed planner provenance (policy name/version, per-service decisions) is not retrievable from this proposal lookup -- only the source is. It was shown at proposal-creation time in Placement. Its absence here implies nothing about validity; DServer re-validates this proposal's placements structurally and against capacity at acceptance time regardless of provenance.",
        ),
      );
    }
    section.append(
      el(
        "p",
        "small text-secondary",
        "Source indicates how this proposal's placements were produced. It is shown for context only and carries no extra trust -- DServer re-validates the proposal's content at acceptance time regardless of source.",
      ),
    );
    return section;
  }

  function acceptanceSection(): HTMLElement {
    const p = proposal!;
    const section = el("section", "proposal-review-acceptance console-card");
    section.append(el("h2", "h6", "Acceptance"));
    if (p.state !== "pending") {
      section.append(badge("Accepted", "success"));
      section.append(
        el(
          "p",
          "small text-secondary mt-2",
          "This proposal has already been accepted. There is no Reject, Cancel, Expire, or Withdraw action -- proposal lifecycle is exactly Pending/Accepted.",
        ),
      );
      return section;
    }
    section.append(
      el("p", "small text-secondary", "No client-side review can guarantee acceptance. DServer revalidates authoritative state at acceptance time."),
    );
    const labelId = "proposal-review-audit-label";
    const labelEl = el("label", "small", "Acceptance audit label");
    labelEl.htmlFor = labelId;
    auditLabelInput.id = labelId;
    const help = el(
      "p",
      "small text-secondary",
      "A caller-supplied audit label recorded with the acceptance -- not authenticated identity. DServer has no application-layer authentication; this is an operator-entered note in a trusted, local control plane.",
    );
    help.id = `${labelId}-help`;
    auditLabelInput.setAttribute("aria-describedby", help.id);
    section.append(labelEl, auditLabelInput, help, acceptButton, acceptOutcome);
    return section;
  }

  function futureNote(): HTMLElement {
    return el(
      "p",
      "small text-secondary proposal-review-future-note",
      "Deployment, runtime reconciliation, and convergence (Phase 138) are not implemented in this Console. Accepting a proposal here changes placement authority only.",
    );
  }

  function navSection(): HTMLElement {
    const nav = el("nav", "proposal-review-nav");
    const back = el("a", "btn btn-outline-secondary btn-sm", "Back to Placement");
    back.setAttribute("href", `${projectPath(projectId)}/placement`);
    back.dataset.route = "";
    const another = el("a", "btn btn-outline-secondary btn-sm", "Create a new placement proposal");
    another.setAttribute("href", `${projectPath(projectId)}/placement`);
    another.dataset.route = "";
    nav.append(back, another);
    // Phase 138 entry point -- only once this proposal is actually
    // Accepted authority (never from a still-Pending proposal, which has
    // nothing yet for reconciliation to target).
    if (proposal?.state === "accepted") {
      const deploy = el("a", "btn btn-primary btn-sm", "Go to Deploy / Reconciliation");
      deploy.setAttribute("href", applicationDeployPath(projectId, proposal.proposal.application_id));
      deploy.dataset.route = "";
      nav.append(deploy);
    }
    return nav;
  }

  function renderAll(): void {
    if (!proposal) return;
    body.replaceChildren(
      identitySection(),
      referencesSection(),
      authoritySection(),
      diffSection(),
      provenanceSection(),
      acceptanceSection(),
      futureNote(),
      navSection(),
    );
  }

  async function loadActive(token: number): Promise<void> {
    if (!proposal) return;
    activeLoaded = false;
    try {
      const active = await placementClient.active(proposal.proposal.application_id);
      if (disposed || token !== sequence) return;
      activeDeployment = active.deployments[0];
      activeLoaded = true;
      renderAll();
    } catch (e) {
      if (disposed || token !== sequence) return;
      activeLoaded = true;
      activeDeployment = undefined;
      renderAll();
      setStatus(e instanceof PlacementError ? `Current active authority is unavailable: ${e.message} (${e.code})` : "Current active authority is unavailable.");
    }
  }

  async function load(): Promise<void> {
    setStatus("Loading proposal…");
    const token = ++sequence;
    try {
      const loaded = await placementClient.getProposal(proposalId);
      if (disposed || token !== sequence) return;
      proposal = loaded;
      setStatus(`Proposal ${loaded.proposal.proposal_id} loaded.`);
      renderAll();
      if (!disposed) onReady(true);
      await loadActive(token);
    } catch (e) {
      if (disposed || token !== sequence) return;
      proposal = undefined;
      setStatus("Proposal unavailable.");
      body.replaceChildren(
        el("p", "small", e instanceof PlacementError ? `${e.message} (${e.code})` : "The proposal could not be loaded."),
      );
      if (!disposed) onReady(false);
    }
  }

  async function handleAcceptSuccess(result: AcceptResponse, token: number): Promise<void> {
    if (disposed || token !== sequence) return;
    const applicationId = proposal?.proposal.application_id;
    // Mandatory re-fetch -- never permanently trust this response's own
    // materialized_dmap/currently_active as a lasting snapshot of either
    // the proposal's own lifecycle state or current active authority.
    const [refreshedProposal, refreshedActive] = await Promise.allSettled([
      placementClient.getProposal(proposalId),
      applicationId ? placementClient.active(applicationId) : Promise.reject(new Error("no application")),
    ]);
    if (disposed || token !== sequence) return;
    if (refreshedProposal.status === "fulfilled") proposal = refreshedProposal.value;
    if (refreshedActive.status === "fulfilled") activeDeployment = refreshedActive.value.deployments[0];
    renderAll();
    const lines = [
      result.idempotent_replay ? "Proposal was already accepted." : "Proposal accepted.",
      `acceptance_id: ${result.acceptance.acceptance_id}`,
      `accepted_at_utc: ${result.acceptance.accepted_at_utc}`,
      `accepted_by: ${result.acceptance.accepted_by}`,
      `proposal_digest: ${result.acceptance.proposal_digest}`,
      `materialized_dmap_ref: ${result.acceptance.materialized_dmap_ref.id}@${result.acceptance.materialized_dmap_ref.revision} (${result.acceptance.materialized_dmap_ref.digest})`,
      `operational_projection_ref: ${result.acceptance.operational_projection_ref.operational_dgraph_id}@${result.acceptance.operational_projection_ref.revision}`,
      result.currently_active
        ? "This materialized D-Map is currently the active authority for this application."
        : "This materialized D-Map is NOT the currently active authority -- a later acceptance has since superseded it.",
      "This changed placement authority only. It did not deploy, reconcile runtime, or prove convergence.",
    ];
    setAcceptNotice(lines);
  }

  async function handleAcceptFailure(e: unknown, token: number): Promise<void> {
    if (disposed || token !== sequence) return;
    if (!(e instanceof PlacementError)) {
      setAcceptNotice(["Acceptance request failed unexpectedly."]);
      return;
    }
    if (AMBIGUOUS_CODES.has(e.code)) {
      setAcceptNotice(["Acceptance outcome is unknown -- checking current proposal state…"]);
      try {
        const refreshed = await placementClient.getProposal(proposalId);
        if (disposed || token !== sequence) return;
        proposal = refreshed;
        if (refreshed.state === "accepted") {
          const refreshedActive = await placementClient.active(refreshed.proposal.application_id).catch(() => undefined);
          if (disposed || token !== sequence) return;
          if (refreshedActive) activeDeployment = refreshedActive.deployments[0];
          renderAll();
          setAcceptNotice(["The acceptance request's outcome was initially unknown, but the proposal is now confirmed Accepted."]);
        } else {
          renderAll();
          setAcceptNotice([
            "The acceptance request's outcome was initially unknown. The proposal is still Pending -- acceptance did not go through. You may try again.",
          ]);
        }
      } catch {
        if (disposed || token !== sequence) return;
        setAcceptNotice(["Acceptance outcome could not be confirmed. Reload this page to check the proposal's current state before retrying."]);
      }
      return;
    }
    if ((e.code === "stale_active_dmap_revision" || e.code === "stale_authoritative_dcontinuum") && proposal) {
      try {
        const refreshedActive = await placementClient.active(proposal.proposal.application_id);
        if (disposed || token !== sequence) return;
        activeDeployment = refreshedActive.deployments[0];
        renderAll();
      } catch {
        /* keep the prior authority context if this refresh itself fails */
      }
    }
    const lines = [`Acceptance rejected: ${e.code}`, e.message];
    for (const finding of e.findings) lines.push(`${finding.code}: ${finding.message}`);
    setAcceptNotice(lines);
  }

  acceptButton.addEventListener("click", () => {
    if (accepting || !proposal || proposal.state !== "pending") return;
    const acceptedBy = auditLabelInput.value.trim();
    if (!acceptedBy) {
      setAcceptNotice(["An acceptance audit label is required."]);
      auditLabelInput.focus();
      return;
    }
    const entries = diffEntries();
    const summary = entries.length
      ? entries.map((entry) => `${entry.service_id}: ${entry.from_node_id ?? "unplaced"} -> ${entry.to_node_id ?? "unplaced"} (${CHANGE_LABEL[entry.change]})`).join("\n")
      : "No placement changes.";
    const confirmed = window.confirm(
      `Accept proposal ${proposal.proposal.proposal_id} for application ${proposal.proposal.application_id}?\n\n${summary}\n\nThis changes placement authority only. It does not deploy or prove runtime convergence. It may still fail if authoritative state has changed since this page loaded.`,
    );
    if (!confirmed) return;
    accepting = true;
    acceptButton.disabled = true;
    auditLabelInput.disabled = true;
    const token = ++sequence;
    setAcceptNotice(["Submitting acceptance…"]);
    void placementClient
      .acceptProposal(proposal.proposal.proposal_id, acceptedBy)
      .then((result) => handleAcceptSuccess(result, token))
      .catch((e) => handleAcceptFailure(e, token))
      .finally(() => {
        accepting = false;
        if (!disposed) {
          acceptButton.disabled = false;
          auditLabelInput.disabled = false;
        }
      });
  });

  void load();

  return {
    element: root,
    mount: () => {},
    dispose: () => {
      disposed = true;
      sequence++;
    },
    dirty: () => false,
    confirmLeave: () => true,
  };
}

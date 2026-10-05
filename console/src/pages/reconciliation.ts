// Phase 138 -- Deploy / Reconciliation Console. The operator workflow:
// accepted placement authority -> inspect target nodes/services -> Preview
// (mode=preview, zero mutation) -> review -> Authorize (per node, permission
// only) -> Reconcile (mode=execute) -> track request/batch/attempt ->
// show command-side result HONESTLY. This workspace never deploys by
// itself: it only creates ReconciliationRequest/batch state and reads back
// request/authorization/attempt projections the agent and dserver already
// produce. See documentation/reference/phase138-deploy-reconciliation-console.md.
import { el, badge } from "../components/presentation.ts";
import { PlacementClient, PlacementError } from "../api/placement.ts";
import { ReconciliationClient, ReconciliationError } from "../api/reconciliation.ts";
import { DMonitorConvergenceClient, DMonitorConvergenceError } from "../api/dmonitor-convergence.ts";
import type { ActiveDeploymentSummary } from "../types/placement.ts";
import type {
  OperationalDGraphSlice,
  ReconciliationRequest,
  ReconciliationBatchResponse,
  ReconciliationAttemptRecord,
  ReconciliationMode,
} from "../types/reconciliation.ts";
import type { EvaluateConvergenceResponse } from "../types/dmonitor-convergence.ts";
import {
  REQUEST_STATUS_LABEL,
  REQUEST_STATUS_TONE,
  BATCH_STATUS_LABEL,
  BATCH_STATUS_TONE,
  ATTEMPT_STATUS_LABEL,
  ATTEMPT_STATUS_TONE,
  ACTION_RESULT_LABEL,
  ACTION_RESULT_TONE,
  AUTHORIZATION_STATUS_LABEL,
  AUTHORIZATION_STATUS_TONE,
  authorizationDisplayStatus,
  isTerminalBatchStatus,
  isOtherApplicationService,
  nextClientRequestId,
} from "./reconciliation-model.ts";
import {
  CONVERGENCE_LABEL,
  CONVERGENCE_TONE,
  FRESHNESS_LABEL,
  FRESHNESS_TONE,
  HEALTH_LABEL,
  HEALTH_TONE,
  latestAdmittedEvidence,
  isWaitingForFirstObservation,
  isConvergenceSettled,
} from "./dmonitor-convergence-model.ts";

export interface ReconciliationPageView {
  element: HTMLElement;
  mount: () => void;
  dispose: () => void;
  dirty: () => boolean;
  confirmLeave: () => boolean;
}

// Part N -- bounded polling, never a complex scheduler. Slower while the
// tab is hidden (Part N1); never overlapping (one in-flight poll chain at a
// time per batch, guarded by `pollToken`).
const POLL_VISIBLE_MS = 3000;
const POLL_HIDDEN_MS = 15000;
// Phase 139B independent review (Audit 45) -- unlike a reconciliation
// batch's own status (genuinely terminal once Succeeded/Failed/Partial: that
// COMMAND attempt is over and will never itself change again), convergence
// is a live fact about CURRENT runtime state: a Converged service can still
// diverge later (the container can crash or be killed outside any command
// this page ever issued) with nobody having touched this page again. Polling
// therefore never permanently stops on settling -- it only slows down, so a
// later divergence is still eventually noticed without a manual refresh.
const POLL_SETTLED_MS = 60000;

// Part O -- ambiguous-transport codes: the HTTP response itself was never
// received, so the outcome is genuinely unknown and a retry MUST reuse the
// same client_request_id. Every other code means a real response came
// back, so a fresh operator action gets a fresh id.
const AMBIGUOUS_CODES = new Set<string | null>(["timeout", "disconnected"]);

function storageKey(projectId: string, applicationId: string): string {
  return `datum-console:reconciliation:${projectId}:${applicationId}`;
}

// Best-effort, per-viewer-browser convenience ONLY (Part Z #12, "browser
// refresh retains state") -- never authoritative. If it is unavailable
// (private browsing, quota) or stale (the batch no longer exists server
// side), the workspace falls back to "no batch yet" cleanly.
function loadCachedBatchId(projectId: string, applicationId: string): string | undefined {
  try {
    return sessionStorage.getItem(storageKey(projectId, applicationId)) ?? undefined;
  } catch {
    return undefined;
  }
}
function saveCachedBatchId(projectId: string, applicationId: string, batchId: string): void {
  try {
    sessionStorage.setItem(storageKey(projectId, applicationId), batchId);
  } catch {
    /* best-effort only */
  }
}
function clearCachedBatchId(projectId: string, applicationId: string): void {
  try {
    sessionStorage.removeItem(storageKey(projectId, applicationId));
  } catch {
    /* best-effort only */
  }
}

export function reconciliationView(
  projectId: string,
  applicationId: string,
  placementClient: PlacementClient,
  reconciliationClient: ReconciliationClient,
  dmonitorConvergenceClient: DMonitorConvergenceClient,
  onReady: (ready: boolean) => void = () => {},
): ReconciliationPageView {
  let disposed = false;
  let sequence = 0;
  let pollToken = 0;
  let convergencePollToken = 0;

  let activeDeployment: ActiveDeploymentSummary | undefined;
  let activeLoaded = false;
  let nodeIds: string[] = [];
  const slices = new Map<string, OperationalDGraphSlice>();
  const sliceErrors = new Map<string, string>();

  let batch: ReconciliationBatchResponse | undefined;
  const attempts = new Map<string, ReconciliationAttemptRecord>();
  const attemptErrors = new Map<string, string>();

  // Phase 139B -- OBSERVED runtime state, structurally independent of the
  // command-side request/batch/attempt state above. Loaded and polled on
  // its own cadence, regardless of whether a batch exists yet (Part Z#4 --
  // observation is relevant for an application that was never reconciled
  // from this page at all, e.g. deployed by a prior session).
  let convergence: EvaluateConvergenceResponse | undefined;
  let convergenceError: string | undefined;
  let convergenceLoaded = false;

  let submitting: ReconciliationMode | null = null;
  let authorizingNode: string | null = null;
  let lastClientRequestId: string | undefined;
  let lastSubmissionOutcomeKnown = true;

  const root = el("div", "page-content reconciliation-view");
  const header = el("header", "page-intro");
  header.append(
    badge("DEPLOY / RECONCILIATION · REQUEST + AUTHORIZATION ONLY", "info"),
    el("h1", "h4", `Reconcile runtime — ${applicationId}`),
    el(
      "p",
      "small text-secondary",
      "Requests node agents to make runtime state match the accepted operational target. This workspace never deploys by itself -- it creates a reconciliation request/batch the node-resident agent discovers and claims on its own poll, and reads back whatever the agent and DServer report. Command success is not runtime health, and runtime health is not convergence.",
    ),
  );
  root.append(header);

  const disclosure = el("p", "reconciliation-disclosure");
  disclosure.setAttribute("role", "note");
  disclosure.append(
    badge("Node-wide scope", "warning"),
    el(
      "span",
      "small",
      " This action requests reconciliation on the target nodes' current operational graph. Other services sharing those nodes' project operational graph may also be reconciled, not only this application's own services -- Preview below lists every service the node reconciliation would consider.",
    ),
  );
  root.append(disclosure);

  const status = el("p", "reconciliation-status");
  status.setAttribute("role", "status");
  root.append(status);

  const body = el("div", "reconciliation-body");
  root.append(body);

  function setStatus(text: string): void {
    status.textContent = text;
  }

  // ---- Shared operator audit-label input (Part E2/L -- audit label only, never authentication) ----
  const auditLabelInput = el("input", "form-control form-control-sm reconciliation-audit-input");
  auditLabelInput.setAttribute("type", "text");
  auditLabelInput.setAttribute("placeholder", "e.g. operator initials, ticket id");
  const auditLabelHelp = el(
    "p",
    "small text-secondary",
    "A caller-supplied audit label recorded with requests/authorizations -- not authenticated identity. DServer has no application-layer authentication; this is an operator-entered note in a trusted, local control plane.",
  );

  function requireAuditLabel(): string | undefined {
    const value = auditLabelInput.value.trim();
    if (!value) {
      setActionNotice(["An audit label is required before this action."]);
      auditLabelInput.focus();
      return undefined;
    }
    return value;
  }

  const actionNotice = el("div", "reconciliation-action-notice");
  actionNotice.setAttribute("role", "status");
  actionNotice.tabIndex = -1;
  function setActionNotice(lines: string[]): void {
    actionNotice.replaceChildren();
    for (const line of lines) actionNotice.append(el("p", "small mb-1", line));
    actionNotice.focus();
  }

  // ---- Data load ----

  function distinctNodeIds(deployment: ActiveDeploymentSummary | undefined): string[] {
    if (!deployment) return [];
    const ids = new Set(deployment.active_dmap.placements.map((p) => p.node_id));
    return [...ids].sort();
  }

  async function loadSlices(token: number, myPollToken: number): Promise<void> {
    const results = await Promise.allSettled(nodeIds.map((nodeId) => reconciliationClient.slice(nodeId)));
    if (disposed || token !== sequence || myPollToken !== pollToken) return;
    results.forEach((result, index) => {
      const nodeId = nodeIds[index]!;
      if (result.status === "fulfilled") slices.set(nodeId, result.value);
      else sliceErrors.set(nodeId, result.reason instanceof ReconciliationError ? result.reason.message : "This node's current operational target is unavailable.");
    });
  }

  // Independent review (Audit 27): `sequence` alone guards against
  // navigation/disposal, but it is NOT bumped when a new batch is
  // submitted within the SAME page view -- two batches can be in flight
  // across a user's "submit again while the previous one is still
  // polling" action, and `token` would be identical for both. `pollToken`
  // (already bumped by `stopPolling`/`startPolling` on every new
  // submission) is the right per-batch generation guard: every write to
  // the shared `batch`/`attempts` state below must check it too, not only
  // `pollOnce`'s own post-await check, or a late response from an
  // ABANDONED batch's poll can clobber a newly-created batch's state.
  async function loadBatch(batchId: string, token: number, myPollToken: number): Promise<void> {
    try {
      const loaded = await reconciliationClient.getBatch(batchId);
      if (disposed || token !== sequence || myPollToken !== pollToken) return;
      batch = loaded;
      await loadLinkedAttempts(token, myPollToken);
    } catch (e) {
      if (disposed || token !== sequence || myPollToken !== pollToken) return;
      if (e instanceof ReconciliationError && e.status === 404) {
        // The cached batch id no longer resolves (very old / pruned
        // control-state) -- forget it rather than show a permanent error.
        batch = undefined;
        clearCachedBatchId(projectId, applicationId);
        return;
      }
      setStatus(e instanceof ReconciliationError ? `Current request batch is unavailable: ${e.message}` : "Current request batch is unavailable.");
    }
  }

  async function loadLinkedAttempts(token: number, myPollToken: number): Promise<void> {
    if (!batch) return;
    const toFetch = batch.requests.filter((r) => r.linked_attempt_id && !attempts.has(r.linked_attempt_id) && !attemptErrors.has(r.linked_attempt_id));
    if (!toFetch.length) return;
    const results = await Promise.allSettled(toFetch.map((r) => reconciliationClient.getAttempt(r.linked_attempt_id!)));
    if (disposed || token !== sequence || myPollToken !== pollToken) return;
    results.forEach((result, index) => {
      const attemptId = toFetch[index]!.linked_attempt_id!;
      if (result.status === "fulfilled") attempts.set(attemptId, result.value);
      else attemptErrors.set(attemptId, result.reason instanceof ReconciliationError ? result.reason.message : "This attempt record is unavailable.");
    });
  }

  function stopPolling(): void {
    pollToken++;
  }

  function schedulePoll(token: number, myPollToken: number): void {
    const delay = document.visibilityState === "hidden" ? POLL_HIDDEN_MS : POLL_VISIBLE_MS;
    setTimeout(() => {
      if (disposed || token !== sequence || myPollToken !== pollToken) return;
      void pollOnce(token, myPollToken);
    }, delay);
  }

  async function pollOnce(token: number, myPollToken: number): Promise<void> {
    if (!batch) return;
    // Independent review (Audit 49): authorization status (shown per node)
    // was otherwise loaded exactly once, at page load -- a long-lived page
    // with an active reconciliation in flight could keep showing a stale
    // "Authorized" badge well past the authorization's own lease expiry.
    // Piggybacking the refresh on this ALREADY-established, bounded poll
    // cadence (rather than adding a second, independent polling loop) keeps
    // it reasonably fresh for exactly the window it matters most -- while a
    // reconciliation the operator is watching is actually in flight. This
    // display is still never authoritative (see `authorizationDisplayStatus`'s
    // own doc comment) -- the real gate remains server/agent-side.
    await Promise.allSettled([loadBatch(batch.reconciliation_batch_id, token, myPollToken), loadSlices(token, myPollToken)]);
    if (disposed || token !== sequence || myPollToken !== pollToken) return;
    renderAll();
    if (batch && !isTerminalBatchStatus(batch.status)) schedulePoll(token, myPollToken);
  }

  function startPolling(token: number): void {
    stopPolling();
    const myPollToken = pollToken;
    schedulePoll(token, myPollToken);
  }

  // Phase 139B -- a SEPARATE bounded poll loop for runtime observation,
  // independent of the batch poll loop above (Part S): it starts on page
  // load regardless of whether a batch exists, and is independently
  // restarted whenever a new submission might make a since-settled
  // convergence worth re-checking. Same visible/hidden cadence, same
  // never-overlapping/late-response-guarded discipline, own generation
  // counter (`convergencePollToken`) so a stale convergence poll chain can
  // never clobber a newer one, mirroring `pollToken`'s own exact role.
  function stopConvergencePolling(): void {
    convergencePollToken++;
  }

  function scheduleConvergencePoll(token: number, myToken: number): void {
    const delay = isConvergenceSettled(convergence)
      ? POLL_SETTLED_MS
      : document.visibilityState === "hidden"
        ? POLL_HIDDEN_MS
        : POLL_VISIBLE_MS;
    setTimeout(() => {
      if (disposed || token !== sequence || myToken !== convergencePollToken) return;
      void pollConvergenceOnce(token, myToken);
    }, delay);
  }

  async function loadConvergence(token: number, myToken: number): Promise<void> {
    try {
      const result = await dmonitorConvergenceClient.evaluate(applicationId);
      if (disposed || token !== sequence || myToken !== convergencePollToken) return;
      convergence = result;
      convergenceError = undefined;
    } catch (e) {
      if (disposed || token !== sequence || myToken !== convergencePollToken) return;
      // A transient refresh failure never discards the last KNOWN-GOOD
      // convergence result -- only a fresh success ever replaces it.
      convergenceError = e instanceof DMonitorConvergenceError ? e.message : "Runtime observation is unavailable.";
    }
    convergenceLoaded = true;
  }

  async function pollConvergenceOnce(token: number, myToken: number): Promise<void> {
    await loadConvergence(token, myToken);
    if (disposed || token !== sequence || myToken !== convergencePollToken) return;
    renderAll();
    // Audit 45 -- never a terminal stop, even once settled: only the
    // cadence slows (see `POLL_SETTLED_MS` above).
    scheduleConvergencePoll(token, myToken);
  }

  // ---- Sections ----

  function authoritySection(): HTMLElement {
    const section = el("section", "reconciliation-authority console-card");
    section.append(el("h2", "h6", "Application / accepted target"));
    if (!activeLoaded) {
      section.append(el("p", "small text-secondary", "Loading accepted placement authority…"));
      return section;
    }
    if (!activeDeployment) {
      section.append(badge("No accepted placement yet", "info"));
      section.append(
        el(
          "p",
          "small text-secondary mt-2",
          "This application has no accepted placement authority in this project. Accept a proposal in Placement before reconciliation can be requested.",
        ),
      );
      return section;
    }
    const dl = el("dl", "reconciliation-grid");
    const add = (label: string, value: string) => dl.append(el("dt", "small text-secondary", label), el("dd", "small entity-identity", value));
    add("application_id", applicationId);
    add("Active D-Map revision", String(activeDeployment.active_dmap.revision));
    add("acceptance_id", activeDeployment.acceptance_id);
    add("operational_dgraph_id", `${activeDeployment.operational_dgraph_id}@${activeDeployment.operational_revision}`);
    add("Target nodes", nodeIds.length ? nodeIds.join(", ") : "none");
    section.append(dl);
    return section;
  }

  function nodeAuthorizeAction(nodeId: string, slice: OperationalDGraphSlice): HTMLElement {
    const wrap = el("div", "reconciliation-node-authorize");
    const displayStatus = authorizationDisplayStatus(slice.authorization, slice.dgraph_id, slice.dgraph_revision);
    wrap.append(badge(AUTHORIZATION_STATUS_LABEL[displayStatus], AUTHORIZATION_STATUS_TONE[displayStatus]));
    if (slice.authorization) {
      const details = el("p", "small text-secondary mb-0 mt-1");
      details.textContent = `authorized_by: ${slice.authorization.authorized_by} · authorized_at_utc: ${slice.authorization.authorized_at_utc}${slice.authorization.expires_at_utc ? ` · expires_at_utc: ${slice.authorization.expires_at_utc}` : ""}`;
      wrap.append(details);
    }
    const authorizeButton = el("button", "btn btn-outline-secondary btn-sm mt-2", displayStatus === "authorized" ? "Re-authorize" : "Authorize");
    authorizeButton.setAttribute("type", "button");
    authorizeButton.disabled = authorizingNode !== null;
    authorizeButton.addEventListener("click", () => void handleAuthorize(nodeId, slice));
    wrap.append(authorizeButton);
    const note = el(
      "p",
      "small text-secondary mt-1 mb-0",
      "Authorization permits host mutation for the exact pinned operational authority shown above. It does not itself start reconciliation.",
    );
    wrap.append(note);
    return wrap;
  }

  function nodesSection(): HTMLElement {
    const section = el("section", "reconciliation-nodes console-card");
    section.append(el("h2", "h6", "Target nodes / operational D-Graph"));
    if (!nodeIds.length) {
      section.append(el("p", "small text-secondary", "No target nodes yet."));
      return section;
    }
    for (const nodeId of nodeIds) {
      const nodeBox = el("div", "reconciliation-node-box");
      const nodeHeader = el("div", "reconciliation-node-header");
      nodeHeader.append(el("strong", "small entity-identity", nodeId));
      nodeBox.append(nodeHeader);
      const slice = slices.get(nodeId);
      const sliceError = sliceErrors.get(nodeId);
      if (sliceError) {
        nodeBox.append(el("p", "small text-secondary mb-0", sliceError));
        section.append(nodeBox);
        continue;
      }
      if (!slice) {
        nodeBox.append(el("p", "small text-secondary mb-0", "Loading…"));
        section.append(nodeBox);
        continue;
      }
      nodeBox.append(el("p", "small text-secondary mb-1", `operational_dgraph: ${slice.dgraph_id}@${slice.dgraph_revision}`));
      // `table-responsive` scopes any horizontal overflow to this scroller
      // alone -- a long service_id must never force the whole page to
      // scroll sideways at tablet widths.
      const scroller = el("div", "table-responsive");
      const table = el("table", "table table-sm reconciliation-services-table");
      const thead = el("thead");
      const headRow = el("tr");
      for (const columnLabel of ["Service", "Application", "Runtime", "Desired state"]) headRow.append(el("th", "small", columnLabel));
      thead.append(headRow);
      const tbody = el("tbody");
      for (const service of slice.desired_services) {
        const row = el("tr");
        const otherApp = isOtherApplicationService(service, applicationId);
        const appCell = el("td", "small entity-identity", service.application_id || "(unknown)");
        if (otherApp) appCell.append(el("span", "small", " "), badge("Other application", "warning"));
        row.append(el("td", "small entity-identity", service.service_id), appCell, el("td", "small", service.runtime), el("td", "small", service.desired_state));
        tbody.append(row);
      }
      table.append(thead, tbody);
      scroller.append(table);
      nodeBox.append(scroller);
      nodeBox.append(nodeAuthorizeAction(nodeId, slice));
      section.append(nodeBox);
    }
    return section;
  }

  function actionsSection(): HTMLElement {
    const section = el("section", "reconciliation-actions console-card");
    section.append(el("h2", "h6", "Actions"));
    const labelId = "reconciliation-audit-label";
    const labelEl = el("label", "small", "Audit label");
    labelEl.htmlFor = labelId;
    auditLabelInput.id = labelId;
    auditLabelInput.setAttribute("aria-describedby", `${labelId}-help`);
    auditLabelHelp.id = `${labelId}-help`;
    section.append(labelEl, auditLabelInput, auditLabelHelp);

    const buttons = el("div", "reconciliation-action-buttons mt-2");
    const previewButton = el("button", "btn btn-outline-primary btn-sm me-2", "Preview reconciliation");
    previewButton.setAttribute("type", "button");
    const reconcileButton = el("button", "btn btn-primary btn-sm", "Reconcile runtime");
    reconcileButton.setAttribute("type", "button");

    const canAct = activeLoaded && !!activeDeployment && nodeIds.length > 0;
    previewButton.disabled = !canAct || submitting !== null;
    reconcileButton.disabled = !canAct || submitting !== null;
    if (!canAct) {
      const reason = el("p", "small text-secondary mb-0 mt-1", "Reconciliation actions are unavailable until accepted placement authority with at least one target node is loaded.");
      section.append(buttons, reason);
    } else {
      section.append(buttons);
    }
    buttons.append(previewButton, reconcileButton);
    previewButton.addEventListener("click", () => void submit("preview"));
    reconcileButton.addEventListener("click", () => void submit("execute"));
    section.append(actionNotice);
    return section;
  }

  function requestRow(request: ReconciliationRequest): HTMLElement {
    const row = el("tr");
    const statusCell = el("td", "small");
    statusCell.append(badge(REQUEST_STATUS_LABEL[request.status], REQUEST_STATUS_TONE[request.status]));
    row.append(
      el("td", "small entity-identity", request.node_id),
      statusCell,
      el("td", "small", request.mode),
      el("td", "small entity-identity", request.linked_attempt_id ?? "—"),
    );
    return row;
  }

  function attemptServiceTable(record: ReconciliationAttemptRecord): HTMLElement {
    const scroller = el("div", "table-responsive");
    const table = el("table", "table table-sm reconciliation-attempt-services-table");
    const thead = el("thead");
    const headRow = el("tr");
    for (const columnLabel of ["Service", "Result", "Host mutation", "Execution status"]) headRow.append(el("th", "small", columnLabel));
    thead.append(headRow);
    const tbody = el("tbody");
    for (const [serviceId, result] of Object.entries(record.services)) {
      const row = el("tr");
      const resultCell = el("td", "small");
      resultCell.append(badge(ACTION_RESULT_LABEL[result.result], ACTION_RESULT_TONE[result.result]));
      row.append(
        el("td", "small entity-identity", serviceId),
        resultCell,
        el("td", "small", result.host_mutation_performed ? "Yes" : "No"),
        el("td", "small", result.execution_status ?? "—"),
      );
      tbody.append(row);
    }
    table.append(thead, tbody);
    scroller.append(table);
    return scroller;
  }

  function attemptDetail(request: ReconciliationRequest): HTMLElement | undefined {
    if (!request.linked_attempt_id) return undefined;
    const record = attempts.get(request.linked_attempt_id);
    const error = attemptErrors.get(request.linked_attempt_id);
    const box = el("div", "reconciliation-attempt-detail console-card");
    box.append(el("h3", "h6", `Attempt ${request.linked_attempt_id} — ${request.node_id}`));
    if (error) {
      box.append(el("p", "small text-secondary mb-0", error));
      return box;
    }
    if (!record) {
      box.append(el("p", "small text-secondary mb-0", "Loading attempt detail…"));
      return box;
    }
    box.append(badge(ATTEMPT_STATUS_LABEL[record.status], ATTEMPT_STATUS_TONE[record.status]));
    if (record.services_truncated) {
      box.append(badge("Service results truncated", "warning"));
      box.append(
        el(
          "p",
          "small text-secondary mt-1 mb-0",
          "Not every service result is shown below -- this attempt reported more services than the server retains per attempt. The table below is incomplete, never treat it as the full picture.",
        ),
      );
    }
    const dl = el("dl", "reconciliation-grid mt-2");
    const add = (label: string, value: string) => dl.append(el("dt", "small text-secondary", label), el("dd", "small entity-identity", value));
    add("started_at_utc", record.started_at_utc);
    add("finished_at_utc", record.finished_at_utc ?? "—");
    add("authorization_id", record.authorization_id ?? "none");
    box.append(dl);
    if (Object.keys(record.services).length) box.append(attemptServiceTable(record));
    else box.append(el("p", "small text-secondary mb-0", "This attempt reported no per-service results."));
    return box;
  }

  function batchSection(): HTMLElement {
    const section = el("section", "reconciliation-batch console-card");
    section.append(el("h2", "h6", "Current request batch"));
    if (!batch) {
      section.append(el("p", "small text-secondary", "No reconciliation request has been created yet in this session. Use Preview or Reconcile runtime above."));
      return section;
    }
    const statusLine = el("p", "small");
    statusLine.append(badge(BATCH_STATUS_LABEL[batch.status], BATCH_STATUS_TONE[batch.status]));
    statusLine.append(el("span", "small ms-2 entity-identity", batch.reconciliation_batch_id));
    section.append(statusLine);
    if (batch.status === "stale") {
      const staleBanner = el("p", "reconciliation-stale-banner");
      staleBanner.setAttribute("role", "alert");
      staleBanner.append(
        badge("Stale", "warning"),
        el(
          "span",
          "small",
          " Accepted operational authority changed after this request was created. This request is never silently retargeted -- create a new Preview/Reconcile request against the current target.",
        ),
      );
      // Part 38 -- show the exact pinned-vs-current context where it is
      // actually available, rather than only the generic statement above.
      for (const request of batch.requests.filter((r) => r.status === "stale")) {
        const currentSlice = slices.get(request.node_id);
        staleBanner.append(
          el(
            "span",
            "small d-block mt-1",
            `${request.node_id}: this request was pinned to ${request.target_dgraph_id}@${request.target_dgraph_revision}` +
              (currentSlice ? `; current live target is ${currentSlice.dgraph_id}@${currentSlice.dgraph_revision}.` : "."),
          ),
        );
      }
      section.append(staleBanner);
    }
    const scroller = el("div", "table-responsive");
    const table = el("table", "table table-sm reconciliation-batch-table");
    const thead = el("thead");
    const headRow = el("tr");
    for (const columnLabel of ["Node", "Request status", "Mode", "Linked attempt"]) headRow.append(el("th", "small", columnLabel));
    thead.append(headRow);
    const tbody = el("tbody");
    for (const request of batch.requests) tbody.append(requestRow(request));
    table.append(thead, tbody);
    scroller.append(table);
    section.append(scroller);
    const waitingNodes = batch.requests.filter((r) => r.status === "pending").map((r) => r.node_id);
    if (waitingNodes.length) section.append(el("p", "small text-secondary mt-2 mb-0", `Waiting for node agent: ${waitingNodes.join(", ")}.`));
    const expiredNodes = batch.requests.filter((r) => r.status === "expired").map((r) => r.node_id);
    if (expiredNodes.length)
      section.append(
        el(
          "p",
          "small text-secondary mt-2 mb-0",
          `Expired, no longer actionable: ${expiredNodes.join(", ")}. Use Preview/Reconcile runtime above to create a new request.`,
        ),
      );
    for (const request of batch.requests) {
      const detail = attemptDetail(request);
      if (detail) section.append(detail);
    }
    return section;
  }

  // Phase 139B -- Part J1: kept visually and structurally SEPARATE from
  // Desired (authoritySection/nodesSection) and Command-side result
  // (batchSection/attemptDetail) -- never collapsed into one status.
  // "Command success is not runtime health, and runtime health is not
  // convergence" (this page's own header text) is enforced here by simply
  // never reusing a command-side badge/label for anything rendered below.
  function observationSection(): HTMLElement {
    const section = el("section", "reconciliation-observation console-card");
    section.append(el("h2", "h6", "Runtime observation / convergence"));
    section.append(
      el(
        "p",
        "small text-secondary",
        "Independently observed by the node agent's own DMonitor producer, compared against the accepted operational target. This is OBSERVATION, not a command result -- it may lag behind a just-completed Reconcile action (Part 39), and it may show a problem even when the last command-side attempt reported Succeeded, or vice versa.",
      ),
    );
    if (convergenceError) {
      section.append(el("p", "small text-secondary mb-2", `Latest refresh: ${convergenceError}`));
    }
    if (!convergenceLoaded) {
      section.append(el("p", "small text-secondary", "Loading runtime observation…"));
      return section;
    }
    if (!convergence) {
      section.append(el("p", "small text-secondary", "Runtime observation is currently unavailable."));
      return section;
    }
    const summary = el("p", "small");
    summary.append(badge(CONVERGENCE_LABEL[convergence.convergence], CONVERGENCE_TONE[convergence.convergence]));
    summary.append(el("span", "small ms-2 text-secondary", `evaluated_at_utc: ${convergence.evaluated_at_utc}`));
    section.append(summary);
    if (convergence.findings.length) {
      const findingsList = el("ul", "reconciliation-findings small mb-2");
      for (const finding of convergence.findings) findingsList.append(el("li", "", `${finding.code}: ${finding.message}`));
      section.append(findingsList);
    }
    if (!convergence.services.length) {
      section.append(el("p", "small text-secondary", "No desired service placements to observe."));
    } else {
      const scroller = el("div", "table-responsive");
      const table = el("table", "table table-sm reconciliation-observation-table");
      const thead = el("thead");
      const headRow = el("tr");
      for (const columnLabel of ["Service", "Node", "Convergence", "Freshness", "Runtime observation health", "Findings"]) headRow.append(el("th", "small", columnLabel));
      thead.append(headRow);
      const tbody = el("tbody");
      for (const service of convergence.services) {
        const row = el("tr");
        const convergenceCell = el("td", "small");
        if (isWaitingForFirstObservation(service)) {
          // Part J2/32/J5 -- a UI-only workflow label derived from already-
          // true facts (zero evidence yet), never itself a backend
          // convergence state and never rendered as Converged/Diverged.
          convergenceCell.append(badge("Waiting for observation", "info"));
        } else {
          convergenceCell.append(badge(CONVERGENCE_LABEL[service.convergence], CONVERGENCE_TONE[service.convergence]));
        }
        const latest = latestAdmittedEvidence(service);
        const freshnessCell = el("td", "small");
        const healthCell = el("td", "small");
        if (latest) {
          freshnessCell.append(badge(FRESHNESS_LABEL[latest.freshness], FRESHNESS_TONE[latest.freshness]));
          healthCell.append(badge(HEALTH_LABEL[latest.health], HEALTH_TONE[latest.health]));
        } else {
          freshnessCell.append(el("span", "small text-secondary", "—"));
          healthCell.append(el("span", "small text-secondary", "—"));
        }
        const findingsCell = el("td", "small");
        findingsCell.textContent = service.findings.map((f) => f.code).join(", ") || "—";
        row.append(el("td", "small entity-identity", service.service_id), el("td", "small entity-identity", service.desired_node_id), convergenceCell, freshnessCell, healthCell, findingsCell);
        tbody.append(row);
      }
      table.append(thead, tbody);
      scroller.append(table);
      section.append(scroller);
    }
    if (convergence.unexpected_realizations.length) {
      const unexpected = el("div", "reconciliation-unexpected mt-2");
      unexpected.append(badge("Unexpected realization(s)", "warning"));
      unexpected.append(
        el(
          "p",
          "small text-secondary mt-1 mb-0",
          `Observed for service id(s) not in this application's current desired placement: ${convergence.unexpected_realizations.map((u) => u.service_id).join(", ")}.`,
        ),
      );
      section.append(unexpected);
    }
    return section;
  }

  function fencingNote(): HTMLElement {
    return el(
      "p",
      "small text-secondary reconciliation-fencing-note",
      "A reconciliation request may be retried safely against desired state. This is not a guarantee of exactly-once execution -- see the phase documentation for the disclosed claim-fencing limitation.",
    );
  }

  function renderAll(): void {
    body.replaceChildren(authoritySection(), nodesSection(), actionsSection(), batchSection(), observationSection(), fencingNote());
  }

  // ---- Actions ----

  async function handleAuthorize(nodeId: string, slice: OperationalDGraphSlice): Promise<void> {
    if (authorizingNode) return;
    const authorizedBy = requireAuditLabel();
    if (!authorizedBy) return;
    const confirmed = window.confirm(
      `Authorize host mutation on node ${nodeId} for operational target ${slice.dgraph_id}@${slice.dgraph_revision}?\n\nThis permits mutation. It does not itself start reconciliation.`,
    );
    if (!confirmed) return;
    authorizingNode = nodeId;
    renderAll();
    const token = sequence;
    try {
      await reconciliationClient.authorize(nodeId, {
        dgraph_id: slice.dgraph_id,
        dgraph_revision: slice.dgraph_revision,
        authorized_by: authorizedBy,
        allow_host_mutation: true,
      });
      if (disposed || token !== sequence) return;
      const refreshed = await reconciliationClient.slice(nodeId).catch(() => undefined);
      if (disposed || token !== sequence) return;
      if (refreshed) slices.set(nodeId, refreshed);
      setActionNotice([`Authorization issued for node ${nodeId}.`]);
    } catch (e) {
      if (disposed || token !== sequence) return;
      setActionNotice([e instanceof ReconciliationError ? `Authorization failed: ${e.message}` : "Authorization failed unexpectedly."]);
    } finally {
      if (!disposed) {
        authorizingNode = null;
        renderAll();
      }
    }
  }

  async function submit(mode: ReconciliationMode): Promise<void> {
    if (submitting || !activeDeployment) return;
    const requestedBy = requireAuditLabel();
    if (!requestedBy) return;
    if (mode === "execute") {
      const confirmed = window.confirm(
        `Reconcile runtime for application ${applicationId}?\n\n` +
          `Target node(s): ${nodeIds.join(", ")}.\n\n` +
          `This is command-side reconciliation, not a health or convergence confirmation -- it requests the agent to make host state match the accepted target, and only reports what the agent attempted.\n\n` +
          `Node-wide scope: this reconciles the ENTIRE operational graph on those nodes -- other applications' services co-located there may also be reconciled, not only ${applicationId}'s own.\n\n` +
          `Authorization is a separate, independent gate: this action does not itself authorize host mutation. It will fail per node if that node lacks a current authorization, or if the target has changed since it was issued.`,
      );
      if (!confirmed) return;
    }
    // Part F3 -- a stable id for ONE submission attempt. Reused verbatim on
    // a retry only when the PRIOR attempt's own outcome was ambiguous
    // (transport-level: timeout/disconnected) -- never regenerated for an
    // ambiguous retry, and never reused once a real response (success or a
    // definite rejection) has been received.
    const clientRequestId = nextClientRequestId(lastClientRequestId, lastSubmissionOutcomeKnown, () => crypto.randomUUID());
    lastClientRequestId = clientRequestId;
    submitting = mode;
    lastSubmissionOutcomeKnown = false;
    renderAll();
    setActionNotice([mode === "preview" ? "Submitting preview request…" : "Submitting reconciliation request…"]);
    const token = sequence;
    try {
      const created = await reconciliationClient.createBatch(applicationId, mode, requestedBy, clientRequestId);
      lastSubmissionOutcomeKnown = true;
      if (disposed || token !== sequence) return;
      saveCachedBatchId(projectId, applicationId, created.reconciliation_batch_id);
      batch = { reconciliation_batch_id: created.reconciliation_batch_id, project_id: projectId, status: "pending", requests: created.requests };
      setActionNotice([
        mode === "preview" ? "Preview request created." : "Reconciliation request created.",
        `reconciliation_batch_id: ${created.reconciliation_batch_id}`,
        `Target nodes: ${created.requests.map((r) => r.node_id).join(", ")}`,
        "Waiting for node agent(s) to claim and run this request.",
      ]);
      renderAll();
      startPolling(token);
      // Phase 139B: a fresh submission may make an already-settled
      // convergence worth re-checking (e.g. a prior Diverged target that
      // this Reconcile is meant to fix) -- restart observation polling
      // rather than leaving it stopped at its last settled result.
      stopConvergencePolling();
      void pollConvergenceOnce(token, convergencePollToken);
    } catch (e) {
      if (disposed || token !== sequence) return;
      if (e instanceof ReconciliationError) {
        lastSubmissionOutcomeKnown = !AMBIGUOUS_CODES.has(e.code);
        if (e.code === "reconciliation_queue_full") {
          setActionNotice(["Node reconciliation queue is full. Wait for an existing request to resolve, then try again.", e.message]);
        } else if (e.code === "reconciliation_request_target_stale") {
          setActionNotice(["Accepted authority changed since this page loaded. Refresh and try again.", e.message]);
        } else if (AMBIGUOUS_CODES.has(e.code)) {
          setActionNotice(["This request's outcome is unknown -- the response was not received. Trying again will reuse the same request id, so retrying is safe.", e.message]);
        } else {
          setActionNotice([`Request failed: ${e.message}`]);
        }
      } else {
        lastSubmissionOutcomeKnown = true;
        setActionNotice(["Request failed unexpectedly."]);
      }
    } finally {
      if (!disposed) {
        submitting = null;
        renderAll();
      }
    }
  }

  // ---- Load ----

  async function load(): Promise<void> {
    setStatus(`Loading accepted authority for ${applicationId}…`);
    const token = ++sequence;
    try {
      const active = await placementClient.active(applicationId);
      if (disposed || token !== sequence) return;
      activeDeployment = active.deployments[0];
      activeLoaded = true;
      nodeIds = distinctNodeIds(activeDeployment);
      setStatus(activeDeployment ? `Accepted authority loaded for ${applicationId}.` : `No accepted authority yet for ${applicationId}.`);
      renderAll();
      if (!disposed) onReady(true);
      // Phase 139B: runtime observation starts immediately (never waiting
      // for the first scheduled tick), independent of whether any batch/
      // command-side activity exists for this session. Fire-and-forget --
      // it renders itself once loaded; `load()` does not block on it.
      stopConvergencePolling();
      void pollConvergenceOnce(token, convergencePollToken);
      await loadSlices(token, pollToken);
      if (disposed || token !== sequence) return;
      renderAll();
      const cachedBatchId = loadCachedBatchId(projectId, applicationId);
      if (cachedBatchId) {
        await loadBatch(cachedBatchId, token, pollToken);
        if (disposed || token !== sequence) return;
        renderAll();
        if (batch && !isTerminalBatchStatus(batch.status)) startPolling(token);
      }
    } catch (e) {
      if (disposed || token !== sequence) return;
      activeLoaded = true;
      activeDeployment = undefined;
      setStatus("Accepted authority is unavailable.");
      body.replaceChildren(el("p", "small", e instanceof PlacementError ? `${e.message} (${e.code})` : "Accepted authority could not be loaded."));
      if (!disposed) onReady(false);
    }
  }

  void load();

  return {
    element: root,
    mount: () => {},
    dispose: () => {
      disposed = true;
      sequence++;
      stopPolling();
      stopConvergencePolling();
    },
    dirty: () => false,
    confirmLeave: () => true,
  };
}

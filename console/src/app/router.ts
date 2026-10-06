export type Route =
  | { kind: "overview" }
  | { kind: "projects" }
  | { kind: "catalog" }
  | { kind: "continuum" }
  | { kind: "project"; projectId: string }
  | { kind: "project_graph"; projectId: string }
  | { kind: "project_graph_editor"; projectId: string }
  | { kind: "project_continuum"; projectId: string }
  | { kind: "project_placement"; projectId: string }
  | { kind: "project_history"; projectId: string }
  | { kind: "project_proposal_review"; projectId: string; proposalId: string }
  | { kind: "project_application_deploy"; projectId: string; applicationId: string }
  | { kind: "project_application_overview"; projectId: string; applicationId: string }
  | { kind: "not_found" };
export function validProjectId(id: string): boolean {
  return (
    id.length > 0 &&
    new TextEncoder().encode(id).length <= 256 &&
    id.trim() === id &&
    !/[\u0000-\u001f\u007f-\u009f]/u.test(id)
  );
}
// Proposal IDs are server-generated (`ddeploy-proposal:<uuid>`), never
// operator-chosen, but the same bounded/control-character-free shape
// check that guards a project_id is the right generic guard for any
// opaque identifier placed into a URL path segment.
export const validProposalId = validProjectId;
// Application ids are operator-chosen opaque strings (unlike proposal ids),
// but the same bounded/control-character-free shape check is still the
// right generic guard for any opaque identifier placed into a URL path
// segment.
export const validApplicationId = validProjectId;
export function parseRoute(pathname: string): Route {
  if (pathname === "/") return { kind: "overview" };
  if (pathname === "/catalog" || pathname === "/catalog/") return { kind: "catalog" };
  if (pathname === "/continuum" || pathname === "/continuum/") return { kind: "continuum" };
  if (pathname === "/projects" || pathname === "/projects/")
    return { kind: "projects" };
  const proposalMatch = /^\/projects\/([^/]+)\/proposals\/([^/]+)\/?$/u.exec(pathname);
  if (proposalMatch?.[1] && proposalMatch?.[2]) {
    try {
      const projectId = decodeURIComponent(proposalMatch[1]);
      const proposalId = decodeURIComponent(proposalMatch[2]);
      if (validProjectId(projectId) && validProposalId(proposalId))
        return { kind: "project_proposal_review", projectId, proposalId };
    } catch {
      /* malformed encoding is Not Found, never a crash */
    }
    return { kind: "not_found" };
  }
  // Phase 138 -- the underlying execution this workspace triggers is
  // node + operational-D-Graph scoped (never a true per-application
  // isolation boundary, see `documentation/reference/phase138-deploy-reconciliation-console.md`),
  // but the entry route is still named by application because that is the
  // operator's own unit of intent ("reconcile application X"). The route
  // itself never fakes application-scoped execution -- only the UI entry
  // point is application-named.
  const deployMatch = /^\/projects\/([^/]+)\/applications\/([^/]+)\/deploy\/?$/u.exec(pathname);
  if (deployMatch?.[1] && deployMatch?.[2]) {
    try {
      const projectId = decodeURIComponent(deployMatch[1]);
      const applicationId = decodeURIComponent(deployMatch[2]);
      if (validProjectId(projectId) && validApplicationId(applicationId))
        return { kind: "project_application_deploy", projectId, applicationId };
    } catch {
      /* malformed encoding is Not Found, never a crash */
    }
    return { kind: "not_found" };
  }
  // Phase 141B -- the application's own read-only operational context
  // (never a push-execution surface, unlike `/deploy` above). Distinct
  // literal path shape (no `/deploy` suffix) guarantees no collision with
  // the deploy route matched immediately above.
  const applicationOverviewMatch = /^\/projects\/([^/]+)\/applications\/([^/]+)\/?$/u.exec(pathname);
  if (applicationOverviewMatch?.[1] && applicationOverviewMatch?.[2]) {
    try {
      const projectId = decodeURIComponent(applicationOverviewMatch[1]);
      const applicationId = decodeURIComponent(applicationOverviewMatch[2]);
      if (validProjectId(projectId) && validApplicationId(applicationId))
        return { kind: "project_application_overview", projectId, applicationId };
    } catch {
      /* malformed encoding is Not Found, never a crash */
    }
    return { kind: "not_found" };
  }
  const match = /^\/projects\/([^/]+)(\/graph(?:\/edit)?|\/continuum|\/placement|\/history)?\/?$/u.exec(pathname);
  if (match?.[1]) {
    try {
      const projectId = decodeURIComponent(match[1]);
      if (validProjectId(projectId))
        return {
          kind:
            match[2] === "/graph/edit"
              ? "project_graph_editor"
              : match[2] === "/graph"
                ? "project_graph"
                : match[2] === "/continuum"
                  ? "project_continuum"
                  : match[2] === "/placement"
                    ? "project_placement"
                    : match[2] === "/history"
                      ? "project_history"
                      : "project",
          projectId,
        };
    } catch {
      /* malformed encoding is Not Found, never a crash */
    }
  }
  return { kind: "not_found" };
}
export function projectPath(id: string): string {
  if (!validProjectId(id)) throw new Error("Invalid project identifier");
  return `/projects/${encodeURIComponent(id)}`;
}
export function proposalReviewPath(projectId: string, proposalId: string): string {
  if (!validProjectId(projectId)) throw new Error("Invalid project identifier");
  if (!validProposalId(proposalId)) throw new Error("Invalid proposal identifier");
  return `/projects/${encodeURIComponent(projectId)}/proposals/${encodeURIComponent(proposalId)}`;
}
export function applicationDeployPath(projectId: string, applicationId: string): string {
  if (!validProjectId(projectId)) throw new Error("Invalid project identifier");
  if (!validApplicationId(applicationId)) throw new Error("Invalid application identifier");
  return `/projects/${encodeURIComponent(projectId)}/applications/${encodeURIComponent(applicationId)}/deploy`;
}
export function applicationOverviewPath(projectId: string, applicationId: string): string {
  if (!validProjectId(projectId)) throw new Error("Invalid project identifier");
  if (!validApplicationId(applicationId)) throw new Error("Invalid application identifier");
  return `/projects/${encodeURIComponent(projectId)}/applications/${encodeURIComponent(applicationId)}`;
}
export function startRouter(render: (route: Route) => void, canNavigate: () => boolean = () => true): () => void {
  let lastPath = window.location.pathname;
  const update = () => {
    if(window.location.pathname !== lastPath && !canNavigate()) { window.history.pushState(null,"",lastPath); return; }
    lastPath=window.location.pathname;
    render(parseRoute(lastPath));
  };
  const click = (event: MouseEvent) => {
    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>(
      "a[data-route]",
    );
    if (
      !link ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      event.shiftKey ||
      link.target ||
      link.hasAttribute("download")
    )
      return;
    const url = new URL(link.href);
    if (url.origin !== window.location.origin) return;
    event.preventDefault();
    if(!canNavigate())return;
    window.history.pushState(null, "", url.pathname);
    lastPath=url.pathname;
    update();
  };
  window.addEventListener("popstate", update);
  document.addEventListener("click", click);
  update();
  return () => {
    window.removeEventListener("popstate", update);
    document.removeEventListener("click", click);
  };
}

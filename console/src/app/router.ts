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
  | { kind: "not_found" };
export function validProjectId(id: string): boolean {
  return (
    id.length > 0 &&
    new TextEncoder().encode(id).length <= 256 &&
    id.trim() === id &&
    !/[\u0000-\u001f\u007f-\u009f]/u.test(id)
  );
}
export function parseRoute(pathname: string): Route {
  if (pathname === "/") return { kind: "overview" };
  if (pathname === "/catalog" || pathname === "/catalog/") return { kind: "catalog" };
  if (pathname === "/continuum" || pathname === "/continuum/") return { kind: "continuum" };
  if (pathname === "/projects" || pathname === "/projects/")
    return { kind: "projects" };
  const match = /^\/projects\/([^/]+)(\/graph(?:\/edit)?|\/continuum|\/placement)?\/?$/u.exec(pathname);
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

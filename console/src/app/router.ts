export type Route =
  | { kind: "overview" }
  | { kind: "projects" }
  | { kind: "project"; projectId: string }
  | { kind: "project_graph"; projectId: string }
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
  if (pathname === "/projects" || pathname === "/projects/")
    return { kind: "projects" };
  const match = /^\/projects\/([^/]+)(\/graph)?\/?$/u.exec(pathname);
  if (match?.[1]) {
    try {
      const projectId = decodeURIComponent(match[1]);
      if (validProjectId(projectId))
        return { kind: match[2] ? "project_graph" : "project", projectId };
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
export function startRouter(render: (route: Route) => void): () => void {
  const update = () => render(parseRoute(window.location.pathname));
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
    window.history.pushState(null, "", url.pathname);
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

import type { GlobalOverview } from "../types/operations.ts";
import {
  coverageView,
  discoveryEmptyMessage,
  el,
} from "../components/presentation.ts";
import { projectPath, validProjectId } from "../app/router.ts";

function card(title: string, description?: string): HTMLElement {
  const box = el("section", "console-card");
  box.append(el("h2", "section-title", title));
  if (description) box.append(el("p", "text-secondary small", description));
  return box;
}
function link(id: string): HTMLAnchorElement {
  const a = el("a", "project-link", id);
  if (validProjectId(id)) {
    a.href = projectPath(id);
    a.dataset.route = "";
  } else {
    a.setAttribute("aria-disabled", "true");
    a.title =
      "This discovered identity is not supported by the foundation route contract.";
  }
  return a;
}
export { wallboardView as overviewView } from "./wallboard.ts";
export function projectsView(data: GlobalOverview): HTMLElement {
  const content = el("div", "page-content");
  const intro = el("div", "page-intro");
  intro.append(
    el("h1", "", "Known projects"),
    el(
      "p",
      "text-secondary",
      "Project references discovered from available operational sources. This is not an “all projects” registry.",
    ),
  );
  content.append(intro);
  const box = card("Discovered references");
  box.append(coverageView(data.known_projects.coverage));
  const page = data.known_projects.page;
  if (!page.items.length)
    box.append(
      el(
        "p",
        "empty-text mt-4",
        discoveryEmptyMessage(data.known_projects.coverage),
      ),
    );
  else {
    const list = el("div", "project-list mt-4");
    for (const id of page.items) {
      const row = el("div", "project-row");
      row.append(
        link(id),
        el("span", "small text-secondary", "Read-only project workspace"),
      );
      list.append(row);
    }
    box.append(list);
  }
  if (page.has_more)
    box.append(
      el(
        "p",
        "small text-secondary mt-3",
        `Showing the first ${page.limit} references. Discovery pagination is not exposed by this foundation API.`,
      ),
    );
  content.append(box);
  return content;
}
export { projectWorkspaceView as projectView } from "./project-workspace.ts";

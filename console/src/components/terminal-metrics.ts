import { el } from "./presentation.ts";

export interface TerminalMetricRow {
  label: string;
  valueText: string;
  ratio?: number | null;
}

export interface TerminalMetricPanelState {
  title: string;
  subtitle?: string;
  metrics: TerminalMetricRow[];
  meta?: Array<[string, string]>;
  liveLabel?: string;
}

export interface TerminalMetricPanel {
  element: HTMLElement;
  update: (state: TerminalMetricPanelState) => void;
}

const BAR_WIDTH = 18;

export function terminalBar(ratio: number | null | undefined): string {
  if (ratio === null || ratio === undefined || !Number.isFinite(ratio)) {
    return `[${"·".repeat(BAR_WIDTH)}]`;
  }
  const bounded = Math.max(0, Math.min(1, ratio));
  const filled = Math.round(bounded * BAR_WIDTH);
  return `[${"█".repeat(filled)}${"░".repeat(BAR_WIDTH - filled)}]`;
}

export function terminalMetricPanel(initial: TerminalMetricPanelState): TerminalMetricPanel {
  const root = el("section", "terminal-metrics");
  root.setAttribute("role", "status");
  root.setAttribute("aria-live", "polite");

  const header = el("div", "terminal-metrics-header");
  const title = el("div", "terminal-metrics-title");
  const live = el("span", "terminal-metrics-live");
  header.append(title, live);

  const subtitle = el("div", "terminal-metrics-subtitle");
  const rows = el("div", "terminal-metrics-rows");
  const meta = el("div", "terminal-metrics-meta");
  root.append(header, subtitle, rows, meta);

  const update = (state: TerminalMetricPanelState) => {
    title.textContent = state.title;
    live.textContent = state.liveLabel ?? "";
    live.hidden = !state.liveLabel;
    subtitle.textContent = state.subtitle ?? "";
    subtitle.hidden = !state.subtitle;

    rows.replaceChildren();
    for (const metric of state.metrics) {
      const row = el("div", "terminal-metric-row");
      const label = el("span", "terminal-metric-label", metric.label);
      const bar = el("span", "terminal-metric-bar", terminalBar(metric.ratio));
      const value = el("span", "terminal-metric-value-text", metric.valueText);
      row.append(label, bar, value);
      rows.append(row);
    }

    meta.replaceChildren();
    for (const [labelText, valueText] of state.meta ?? []) {
      const row = el("div", "terminal-metric-meta-row");
      row.append(
        el("span", "terminal-metric-meta-label", labelText),
        el("span", "terminal-metric-meta-value", valueText),
      );
      meta.append(row);
    }
    meta.hidden = (state.meta ?? []).length === 0;
  };

  update(initial);
  return { element: root, update };
}

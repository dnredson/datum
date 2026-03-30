use anyhow::Result;
use crossterm::{
    cursor::{self, Hide, Show},
    event::{self, Event as CEvent, KeyCode},
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{stdout, Stdout, Write};
use std::time::{Duration, Instant};

use crate::cli::snapshot_reader::read_snapshot_from_config;
use crate::config;
use crate::core::model::{Edge, Entity, Event, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusMode {
    All,
    Alerts,
    Entities,
    Graph,
}

pub fn top_command(config_path: &str, refresh_ms: Option<u64>) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    let refresh_ms = refresh_ms.unwrap_or(cfg.agent.cycle_s.saturating_mul(1000));

    let mut out = stdout();

    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, Hide)?;

    let result = run_top_loop(&cfg, refresh_ms, &mut out);

    execute!(out, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run_top_loop(cfg: &crate::config::Config, refresh_ms: u64, out: &mut Stdout) -> Result<()> {
    let mut last_cycle_id: Option<String> = None;
    let mut focus = FocusMode::All;

    let mut current_snapshot: Option<Snapshot> = None;
    let mut snapshot_changed = false;

    let ui_tick = Duration::from_millis(200);
    let mut next_refresh_at = Instant::now();

    loop {
        let now = Instant::now();

        // Só relê o snapshot quando chega o momento do refresh
        if current_snapshot.is_none() || now >= next_refresh_at {
            match read_snapshot_from_config(cfg) {
                Ok(snapshot) => {
                    snapshot_changed = match &last_cycle_id {
                        Some(prev) => prev != &snapshot.meta.cycle_id,
                        None => true,
                    };
                    last_cycle_id = Some(snapshot.meta.cycle_id.clone());
                    current_snapshot = Some(snapshot);
                }
                Err(_) => {
                    current_snapshot = None;
                }
            }

            next_refresh_at = Instant::now() + Duration::from_millis(refresh_ms);
        }

        // Leitura de teclado
        if event::poll(ui_tick)? {
            if let CEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') => {
                        match read_snapshot_from_config(cfg) {
                            Ok(snapshot) => {
                                snapshot_changed = match &last_cycle_id {
                                    Some(prev) => prev != &snapshot.meta.cycle_id,
                                    None => true,
                                };
                                last_cycle_id = Some(snapshot.meta.cycle_id.clone());
                                current_snapshot = Some(snapshot);
                            }
                            Err(_) => {
                                current_snapshot = None;
                            }
                        }

                        next_refresh_at = Instant::now() + Duration::from_millis(refresh_ms);
                    }
                    KeyCode::Char('a') => focus = FocusMode::Alerts,
                    KeyCode::Char('e') => focus = FocusMode::Entities,
                    KeyCode::Char('g') => focus = FocusMode::Graph,
                    KeyCode::Char('t') => focus = FocusMode::All,
                    _ => {}
                }
            }
        }

        // Redesenha a UI usando snapshot em memória
        let (term_w, term_h) = terminal::size().unwrap_or((120, 40));
        execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

        match &current_snapshot {
            Some(snapshot) => {
                render_top(
                    out,
                    snapshot,
                    refresh_ms,
                    snapshot_changed,
                    term_w,
                    term_h,
                    focus,
                    next_refresh_at,
                )?;
            }
            None => {
                write_colored_line(out, "DATUM TOP", Color::Cyan, true)?;
                writeln!(out)?;
                write_colored_line(out, "failed to read snapshot", Color::Red, false)?;
                writeln!(out)?;
                writeln!(
                    out,
                    "[q] quit  [r] refresh  [a] alerts  [e] entities  [g] graph  [t] all"
                )?;
            }
        }

        out.flush()?;
    }

    Ok(())
}

fn render_top(
    out: &mut Stdout,
    snapshot: &Snapshot,
    refresh_ms: u64,
    changed: bool,
    _term_w: u16,
    term_h: u16,
    focus: FocusMode,
    next_refresh_at: Instant,
) -> Result<()> {
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &snapshot.entities {
        *by_kind.entry(entity.kind.as_str()).or_insert(0) += 1;
    }

    let refresh_s = refresh_ms as f64 / 1000.0;
    let status_text = if changed { "updated" } else { "unchanged" };
    let status_color = if changed { Color::Green } else { Color::DarkGrey };

    write_colored(out, "DATUM TOP", Color::Cyan, true)?;
    write!(
        out,
        "  node={}  observed_at={}  cycle=",
        snapshot.meta.node_id, snapshot.meta.observed_at_utc
    )?;
    write_colored(out, &snapshot.meta.cycle_id, Color::Yellow, false)?;
    writeln!(out)?;

    write!(out, "refresh={refresh_s:.1}s  snapshot_status=")?;
    write_colored(out, status_text, status_color, true)?;
    write!(out, "  focus=")?;
    write_colored(out, focus_label(focus), Color::Magenta, true)?;
    writeln!(out)?;
    writeln!(out)?;

    write_colored_line(out, "SUMMARY", Color::Blue, true)?;
    writeln!(
        out,
        "entities={}  edges={}  events={}  actions={}",
        snapshot.entities.len(),
        snapshot.edges.len(),
        snapshot.events.len(),
        snapshot.actions.len()
    )?;

    if !by_kind.is_empty() {
        write!(out, "kinds=")?;
        let mut first = true;
        for (kind, count) in by_kind {
            if !first {
                write!(out, ", ")?;
            }
            let color = kind_color(kind);
            write_colored(out, kind, color, false)?;
            write!(out, ":{}", count)?;
            first = false;
        }
        writeln!(out)?;
    }

    writeln!(out)?;

    let layout = compute_layout(term_h, focus);

    match focus {
        FocusMode::All => {
            render_alerts_section(out, snapshot, layout.events_lines)?;
            writeln!(out)?;
            render_entities_section(out, snapshot, layout.entities_lines)?;
            writeln!(out)?;
            render_graph_section(out, snapshot, layout.graph_lines)?;
        }
        FocusMode::Alerts => {
            render_alerts_section(out, snapshot, layout.events_lines)?;
        }
        FocusMode::Entities => {
            render_entities_section(out, snapshot, layout.entities_lines)?;
        }
        FocusMode::Graph => {
            render_graph_section(out, snapshot, layout.graph_lines)?;
        }
    }

    writeln!(out)?;
    render_footer(out, refresh_ms, next_refresh_at, changed)?;

    Ok(())
}

fn render_alerts_section(out: &mut Stdout, snapshot: &Snapshot, lines: usize) -> Result<()> {
    write_colored_line(out, "ALERTS / RECENT EVENTS", Color::Blue, true)?;

    let events = latest_events(snapshot, lines);
    if events.is_empty() {
        write_colored_line(out, "no events", Color::DarkGrey, false)?;
        return Ok(());
    }

    for ev in events {
        render_event_line(out, &ev)?;
    }

    Ok(())
}

fn render_entities_section(out: &mut Stdout, snapshot: &Snapshot, lines: usize) -> Result<()> {
    write_colored_line(out, "TOP ENTITIES", Color::Blue, true)?;

    let rows = render_entities(snapshot, lines);
    if rows.is_empty() {
        write_colored_line(out, "no entities", Color::DarkGrey, false)?;
        return Ok(());
    }

    for line in rows {
        render_entity_line_colored(out, &line)?;
    }

    Ok(())
}

fn render_graph_section(out: &mut Stdout, snapshot: &Snapshot, lines: usize) -> Result<()> {
    write_colored_line(out, "GRAPH", Color::Blue, true)?;

    let graph = render_graph(snapshot, lines);
    if graph.is_empty() {
        write_colored_line(out, "no graph lines", Color::DarkGrey, false)?;
        return Ok(());
    }

    for line in graph {
        render_graph_line_colored(out, &line)?;
    }

    Ok(())
}

fn render_footer(
    out: &mut Stdout,
    refresh_ms: u64,
    next_refresh_at: Instant,
    changed: bool,
) -> Result<()> {
    let remaining = next_refresh_at.saturating_duration_since(Instant::now());
    let secs = remaining.as_secs();
    let millis = remaining.subsec_millis();

    let countdown_color = if remaining.as_millis() < 1000 {
        Color::Yellow
    } else {
        Color::DarkGrey
    };

    write_colored(out, "next refresh in ", Color::DarkGrey, false)?;
    write_colored(
        out,
        &format!("{}.{:03}s", secs, millis),
        countdown_color,
        true,
    )?;
    write!(out, "  |  ")?;
    write_colored(out, "mode: ", Color::DarkGrey, false)?;
    write_colored(out, &format!("{} ms", refresh_ms), Color::White, false)?;
    write!(out, "  |  ")?;
    write_colored(out, "snapshot: ", Color::DarkGrey, false)?;
    write_colored(
        out,
        if changed { "fresh" } else { "same" },
        if changed { Color::Green } else { Color::DarkGrey },
        true,
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "[q] quit  [r] refresh  [a] alerts  [e] entities  [g] graph  [t] all"
    )?;

    Ok(())
}

struct LayoutBudget {
    events_lines: usize,
    entities_lines: usize,
    graph_lines: usize,
}

fn compute_layout(term_h: u16, focus: FocusMode) -> LayoutBudget {
    let reserved = 12usize;
    let usable = term_h.saturating_sub(reserved as u16) as usize;

    match focus {
        FocusMode::Alerts => LayoutBudget {
            events_lines: usable.max(6),
            entities_lines: 0,
            graph_lines: 0,
        },
        FocusMode::Entities => LayoutBudget {
            events_lines: 0,
            entities_lines: usable.max(6),
            graph_lines: 0,
        },
        FocusMode::Graph => LayoutBudget {
            events_lines: 0,
            entities_lines: 0,
            graph_lines: usable.max(8),
        },
        FocusMode::All => {
            if usable <= 12 {
                return LayoutBudget {
                    events_lines: 3,
                    entities_lines: 3,
                    graph_lines: 4,
                };
            }

            let events_lines = ((usable * 28) / 100).max(4);
            let entities_lines = ((usable * 34) / 100).max(4);
            let graph_lines = usable.saturating_sub(events_lines + entities_lines).max(6);

            LayoutBudget {
                events_lines,
                entities_lines,
                graph_lines,
            }
        }
    }
}

fn latest_events(snapshot: &Snapshot, limit: usize) -> Vec<Event> {
    let mut events = snapshot.events.clone();
    events.sort_by(|a, b| b.observed_at_utc.cmp(&a.observed_at_utc));
    events.into_iter().take(limit).collect()
}

fn render_entities(snapshot: &Snapshot, limit: usize) -> Vec<String> {
    let mut rows: Vec<String> = snapshot.entities.iter().map(format_entity_line).collect();
    rows.sort();
    rows.into_iter().take(limit).collect()
}

fn format_entity_line(entity: &Entity) -> String {
    let attrs = &entity.attrs;
    let mut extra = Vec::new();

    if let Some(v) = attrs.get("status").and_then(|v| v.as_str()) {
        extra.push(format!("status={v}"));
    }
    if let Some(v) = attrs.get("health").and_then(|v| v.as_str()) {
        extra.push(format!("health={v}"));
    }
    if let Some(v) = attrs.get("cpu_pct").and_then(|v| v.as_f64()) {
        extra.push(format!("cpu={v:.2}% {}", cpu_sparkline(v)));
    }
    if let Some(v) = attrs.get("mem_kb").and_then(|v| v.as_u64()) {
        extra.push(format!("mem_kb={v} {}", mem_sparkline_kb(v)));
    }
    if let Some(v) = attrs.get("available").and_then(|v| v.as_bool()) {
        extra.push(format!("available={v}"));
    }
    if let Some(v) = attrs.get("rtt_avg_ms").and_then(|v| v.as_f64()) {
        extra.push(format!("rtt_avg_ms={v:.2}"));
    }
    if let Some(v) = attrs.get("name").and_then(|v| v.as_str()) {
        extra.push(format!("name={v}"));
    }

    if extra.is_empty() {
        format!("{:<72} kind={}", entity.id, entity.kind)
    } else {
        format!("{:<72} kind={} {}", entity.id, entity.kind, extra.join(" "))
    }
}

fn cpu_sparkline(cpu_pct: f64) -> &'static str {
    match cpu_pct {
        x if x < 5.0 => "▁",
        x if x < 15.0 => "▂",
        x if x < 30.0 => "▃",
        x if x < 50.0 => "▄",
        x if x < 70.0 => "▅",
        x if x < 85.0 => "▆",
        x if x < 95.0 => "▇",
        _ => "█",
    }
}

fn mem_sparkline_kb(mem_kb: u64) -> &'static str {
    match mem_kb {
        x if x < 64 * 1024 => "▁",
        x if x < 128 * 1024 => "▂",
        x if x < 256 * 1024 => "▃",
        x if x < 512 * 1024 => "▄",
        x if x < 1024 * 1024 => "▅",
        x if x < 2 * 1024 * 1024 => "▆",
        x if x < 4 * 1024 * 1024 => "▇",
        _ => "█",
    }
}

fn render_graph(snapshot: &Snapshot, limit_lines: usize) -> Vec<String> {
    if snapshot.entities.is_empty() {
        return vec!["<empty snapshot>".to_string()];
    }

    let entity_ids: BTreeSet<String> = snapshot.entities.iter().map(|e| e.id.clone()).collect();

    let root = if entity_ids.contains(&snapshot.meta.host_id) {
        snapshot.meta.host_id.clone()
    } else {
        snapshot.entities[0].id.clone()
    };

    let mut outgoing: BTreeMap<String, Vec<&Edge>> = BTreeMap::new();
    for edge in &snapshot.edges {
        outgoing.entry(edge.from.clone()).or_default().push(edge);
    }

    let mut lines = Vec::new();
    let mut visited = BTreeSet::new();

    collect_graph_lines(
        &root,
        &outgoing,
        snapshot,
        "",
        true,
        &mut visited,
        &mut lines,
        limit_lines,
    );

    if lines.is_empty() {
        lines.push("<no graph lines>".to_string());
    }

    lines
}

fn collect_graph_lines(
    node_id: &str,
    outgoing: &BTreeMap<String, Vec<&Edge>>,
    snapshot: &Snapshot,
    prefix: &str,
    is_last: bool,
    visited: &mut BTreeSet<String>,
    lines: &mut Vec<String>,
    limit_lines: usize,
) {
    if lines.len() >= limit_lines {
        return;
    }

    let label = entity_label(snapshot, node_id);

    if prefix.is_empty() {
        lines.push(label);
    } else {
        let branch = if is_last { "└── " } else { "├── " };
        lines.push(format!("{prefix}{branch}{label}"));
    }

    if !visited.insert(node_id.to_string()) {
        return;
    }

    let Some(edges) = outgoing.get(node_id) else {
        return;
    };

    let next_prefix = if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    for (idx, edge) in edges.iter().enumerate() {
        if lines.len() >= limit_lines {
            return;
        }

        let last_edge = idx + 1 == edges.len();
        let edge_branch = if last_edge { "└── " } else { "├── " };
        lines.push(format!(
            "{}{}{} -> {}",
            next_prefix,
            edge_branch,
            edge.rel,
            entity_label(snapshot, &edge.to)
        ));

        let recurse_prefix = if last_edge {
            format!("{next_prefix}    ")
        } else {
            format!("{next_prefix}│   ")
        };

        if outgoing.contains_key(&edge.to) {
            collect_graph_lines(
                &edge.to,
                outgoing,
                snapshot,
                &recurse_prefix,
                true,
                visited,
                lines,
                limit_lines,
            );
        }
    }
}

fn entity_label(snapshot: &Snapshot, entity_id: &str) -> String {
    if let Some(Entity { kind, .. }) = snapshot.entities.iter().find(|e| e.id == entity_id) {
        format!("{entity_id} [{kind}]")
    } else {
        entity_id.to_string()
    }
}

fn render_event_line(out: &mut Stdout, ev: &Event) -> Result<()> {
    let sev_color = severity_color(&ev.severity);
    let kind_color = event_kind_color(&ev.kind);
    let alert = is_alert_event(ev);

    write!(out, "{}  ", ev.observed_at_utc)?;
    write_colored(out, &format!("{:<7}", ev.severity), sev_color, true)?;
    write!(out, " ")?;
    write_colored(out, &format!("{:<24}", ev.kind), kind_color, alert)?;
    if alert {
        write!(out, " ")?;
        write_colored(out, "●", Color::Red, true)?;
    }
    writeln!(out, " target={} msg={}", ev.target, ev.message)?;

    Ok(())
}

fn is_alert_event(ev: &Event) -> bool {
    let sev = ev.severity.to_ascii_lowercase();
    let kind = ev.kind.to_ascii_lowercase();
    sev == "critical"
        || sev == "error"
        || sev == "warning"
        || kind.contains("failed")
        || kind.contains("error")
        || kind.contains("down")
}

fn render_entity_line_colored(out: &mut Stdout, line: &str) -> Result<()> {
    if line.contains("kind=process") {
        write_colored_line(out, line, Color::Yellow, false)
    } else if line.contains("kind=container") {
        write_colored_line(out, line, Color::Cyan, false)
    } else if line.contains("kind=host") {
        write_colored_line(out, line, Color::Green, false)
    } else if line.contains("kind=connection") || line.contains("kind=check") {
        write_colored_line(out, line, Color::Magenta, false)
    } else {
        writeln!(out, "{line}")?;
        Ok(())
    }
}

fn render_graph_line_colored(out: &mut Stdout, line: &str) -> Result<()> {
    if line.contains("->") {
        write_colored_line(out, line, Color::DarkGrey, false)
    } else if line.contains("[host]") {
        write_colored_line(out, line, Color::Green, true)
    } else if line.contains("[container]") {
        write_colored_line(out, line, Color::Cyan, false)
    } else if line.contains("[process]") {
        write_colored_line(out, line, Color::Yellow, false)
    } else if line.contains("[connection]") || line.contains("[check]") {
        write_colored_line(out, line, Color::Magenta, false)
    } else {
        writeln!(out, "{line}")?;
        Ok(())
    }
}

fn severity_color(severity: &str) -> Color {
    match severity.to_ascii_lowercase().as_str() {
        "critical" => Color::Red,
        "error" => Color::Red,
        "warning" | "warn" => Color::Yellow,
        "info" => Color::Blue,
        "debug" => Color::DarkGrey,
        _ => Color::White,
    }
}

fn event_kind_color(kind: &str) -> Color {
    let k = kind.to_ascii_lowercase();
    if k.contains("failed") || k.contains("error") || k.contains("down") {
        Color::Red
    } else if k.contains("recovery") {
        Color::Green
    } else if k.contains("unbind") || k.contains("stale") {
        Color::Yellow
    } else {
        Color::White
    }
}

fn kind_color(kind: &str) -> Color {
    match kind {
        "host" => Color::Green,
        "container" => Color::Cyan,
        "process" => Color::Yellow,
        "connection" | "check" => Color::Magenta,
        _ => Color::White,
    }
}

fn focus_label(focus: FocusMode) -> &'static str {
    match focus {
        FocusMode::All => "all",
        FocusMode::Alerts => "alerts",
        FocusMode::Entities => "entities",
        FocusMode::Graph => "graph",
    }
}

fn write_colored(out: &mut Stdout, text: &str, color: Color, bold: bool) -> Result<()> {
    execute!(out, SetForegroundColor(color))?;
    if bold {
        execute!(out, SetAttribute(Attribute::Bold))?;
    }
    execute!(out, Print(text))?;
    execute!(out, ResetColor, SetAttribute(Attribute::Reset))?;
    Ok(())
}

fn write_colored_line(out: &mut Stdout, text: &str, color: Color, bold: bool) -> Result<()> {
    write_colored(out, text, color, bold)?;
    writeln!(out)?;
    Ok(())
}
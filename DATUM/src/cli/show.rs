use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

use crate::cli::commands::ShowCommand;
use crate::cli::snapshot_reader::read_snapshot_from_config;
use crate::config;
use crate::core::model::{Edge, Entity, Snapshot};

pub fn show_command(config_path: &str, target: ShowCommand) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    let snapshot = read_snapshot_from_config(&cfg)?;

    match target {
        ShowCommand::Snapshot => show_snapshot(&snapshot),
        ShowCommand::Events { limit } => show_events(&snapshot, limit),
        ShowCommand::Graph => show_graph(&snapshot),
    }

    Ok(())
}

fn show_snapshot(snapshot: &Snapshot) {
    println!("DATUM SNAPSHOT");
    println!("schema_version : {}", snapshot.meta.schema_version);
    println!("agent_version  : {}", snapshot.meta.agent_version);
    println!("cycle_id       : {}", snapshot.meta.cycle_id);
    println!("observed_at    : {}", snapshot.meta.observed_at_utc);
    println!("node_id        : {}", snapshot.meta.node_id);
    println!("host_id        : {}", snapshot.meta.host_id);
    println!();

    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &snapshot.entities {
        *by_kind.entry(entity.kind.as_str()).or_insert(0) += 1;
    }

    println!("SUMMARY");
    println!("entities : {}", snapshot.entities.len());
    println!("edges    : {}", snapshot.edges.len());
    println!("events   : {}", snapshot.events.len());
    println!("actions  : {}", snapshot.actions.len());
    println!();

    if !by_kind.is_empty() {
        println!("ENTITIES BY KIND");
        for (kind, count) in by_kind {
            println!("{:<16} {}", kind, count);
        }
    }
}

fn show_events(snapshot: &Snapshot, limit: usize) {
    println!("DATUM EVENTS");
    println!("cycle_id    : {}", snapshot.meta.cycle_id);
    println!("observed_at : {}", snapshot.meta.observed_at_utc);
    println!();

    let mut events = snapshot.events.clone();
    events.sort_by(|a, b| b.observed_at_utc.cmp(&a.observed_at_utc));

    for ev in events.into_iter().take(limit) {
        println!(
            "{}  {:<7} {:<24} target={} msg={}",
            ev.observed_at_utc, ev.severity, ev.kind, ev.target, ev.message
        );
    }
}

fn show_graph(snapshot: &Snapshot) {
    println!("DATUM GRAPH");
    println!("cycle_id    : {}", snapshot.meta.cycle_id);
    println!("observed_at : {}", snapshot.meta.observed_at_utc);
    println!();

    let entity_ids: BTreeSet<String> = snapshot.entities.iter().map(|e| e.id.clone()).collect();

    let mut outgoing: BTreeMap<String, Vec<&Edge>> = BTreeMap::new();
    for edge in &snapshot.edges {
        outgoing.entry(edge.from.clone()).or_default().push(edge);
    }

    let root = if entity_ids.contains(&snapshot.meta.host_id) {
        snapshot.meta.host_id.clone()
    } else {
        snapshot
            .entities
            .first()
            .map(|e| e.id.clone())
            .unwrap_or_else(|| "<empty>".to_string())
    };

    if root == "<empty>" {
        println!("snapshot has no entities");
        return;
    }

    print_graph_from(
        root.as_str(),
        &outgoing,
        snapshot,
        "",
        true,
        &mut BTreeSet::new(),
    );
}

fn print_graph_from(
    node_id: &str,
    outgoing: &BTreeMap<String, Vec<&Edge>>,
    snapshot: &Snapshot,
    prefix: &str,
    is_last: bool,
    visited: &mut BTreeSet<String>,
) {
    let label = entity_label(snapshot, node_id);

    if prefix.is_empty() {
        println!("{}", label);
    } else {
        let branch = if is_last { "└── " } else { "├── " };
        println!("{prefix}{branch}{label}");
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
        let last_edge = idx + 1 == edges.len();
        let edge_branch = if last_edge {
            "└── "
        } else {
            "├── "
        };
        println!(
            "{}{}{} -> {}",
            next_prefix,
            edge_branch,
            edge.rel,
            entity_label(snapshot, &edge.to)
        );

        let recurse_prefix = if last_edge {
            format!("{next_prefix}    ")
        } else {
            format!("{next_prefix}│   ")
        };

        if outgoing.contains_key(&edge.to) {
            print_graph_from(&edge.to, outgoing, snapshot, &recurse_prefix, true, visited);
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

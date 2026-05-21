use anyhow::{anyhow, Result};
use serde_json::json;

use crate::{
    config::Config,
    dgraph_snapshot::{
        build_dgraph_fixture_snapshot, build_local_dgraph_snapshot,
        client::{
            fetch_latest_local_dgraph_snapshot, fetch_latest_local_dgraph_snapshots,
            fetch_project_dgraph_materialized, fetch_project_dgraph_materialized_summary,
            fetch_project_dgraph_planner_dry_run, fetch_project_dgraph_planner_dry_run_summary,
            publish_local_dgraph_snapshot,
        },
        planner::{ProjectDGraphPlannerDryRunResponse, ProjectDGraphPlannerDryRunSummaryResponse},
        validate_local_dgraph_snapshot_publish_invariants, DGraphSnapshotFixtureScenario,
        LocalDGraphSnapshot, LocalDGraphSnapshotList, ProjectDGraphMaterialization,
        ProjectDGraphMaterializationSummary,
    },
};

enum DgraphSnapshotMode {
    Local,
    Publish,
    Latest,
    LatestAll,
    Materialized,
    MaterializedSummary,
    PlannerDryRun,
    PlannerSummary,
}

pub fn run_dgraph_snapshot_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    fixture_scenario: Option<&str>,
    publish: bool,
    latest: bool,
    latest_all: bool,
    materialized: bool,
    materialized_summary: bool,
    planner_dry_run: bool,
    planner_summary: bool,
    json_output: bool,
) -> Result<()> {
    let mode = validate_dgraph_snapshot_mode(
        fixture_scenario.is_some(),
        publish,
        latest,
        latest_all,
        materialized,
        materialized_summary,
        planner_dry_run,
        planner_summary,
    )?;
    let fixture_scenario = fixture_scenario
        .map(DGraphSnapshotFixtureScenario::parse)
        .transpose()?;

    match mode {
        DgraphSnapshotMode::Local => {
            let snapshot = match fixture_scenario {
                Some(scenario) => build_dgraph_fixture_snapshot(cfg, scenario),
                None => build_local_dgraph_snapshot(cfg),
            };
            print_local_snapshot_output(&snapshot, json_output);
            Ok(())
        }
        DgraphSnapshotMode::Publish => {
            let snapshot = match fixture_scenario {
                Some(scenario) => build_dgraph_fixture_snapshot(cfg, scenario),
                None => build_local_dgraph_snapshot(cfg),
            };
            validate_local_dgraph_snapshot_publish_invariants(&snapshot)?;

            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let node_id = resolve_node_id(cfg, node_id_override)?;
            let server_snapshot =
                publish_local_dgraph_snapshot(&base_url, &project_id, &node_id, &snapshot)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "local_snapshot": snapshot,
                        "server_snapshot": server_snapshot,
                        "published": true,
                        "side_effects_performed": false,
                    }))
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_local_dgraph_snapshot(&snapshot));
            print!("{}", format_published_snapshot(&server_snapshot));
            println!();
            if fixture_scenario.is_some() {
                println!("Note: publishing recorded fixture metadata only. No planner decision was executed.");
            } else {
                println!("Note: this command published local d-graph snapshot metadata only. It did not run planner logic, deploy modules, or reconfigure flows.");
            }
            Ok(())
        }
        DgraphSnapshotMode::Latest => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let node_id = resolve_node_id(cfg, node_id_override)?;
            let snapshot = fetch_latest_local_dgraph_snapshot(&base_url, &project_id, &node_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_latest_snapshot(&snapshot));
            Ok(())
        }
        DgraphSnapshotMode::LatestAll => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let snapshot_list = fetch_latest_local_dgraph_snapshots(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot_list)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_latest_snapshot_list(&snapshot_list));
            Ok(())
        }
        DgraphSnapshotMode::Materialized => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let materialized_response = fetch_project_dgraph_materialized(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&materialized_response)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!(
                "{}",
                format_materialized_dgraph(&materialized_response, &project_id)
            );
            Ok(())
        }
        DgraphSnapshotMode::MaterializedSummary => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let summary_response =
                fetch_project_dgraph_materialized_summary(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&summary_response)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!(
                "{}",
                format_materialized_dgraph_summary(&summary_response, &project_id)
            );
            Ok(())
        }
        DgraphSnapshotMode::PlannerDryRun => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let response = fetch_project_dgraph_planner_dry_run(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_planner_dry_run(&response, &project_id));
            Ok(())
        }
        DgraphSnapshotMode::PlannerSummary => {
            let base_url = resolve_base_url(cfg, url_override)?;
            let project_id = resolve_project_id(cfg, project_id_override)?;
            let response = fetch_project_dgraph_planner_dry_run_summary(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_planner_dry_run_summary(&response, &project_id));
            Ok(())
        }
    }
}

fn validate_dgraph_snapshot_mode(
    fixture: bool,
    publish: bool,
    latest: bool,
    latest_all: bool,
    materialized: bool,
    materialized_summary: bool,
    planner_dry_run: bool,
    planner_summary: bool,
) -> Result<DgraphSnapshotMode> {
    if fixture
        && (latest
            || latest_all
            || materialized
            || materialized_summary
            || planner_dry_run
            || planner_summary)
    {
        return Err(anyhow!(
            "--fixture cannot be combined with --latest, --latest-all, --materialized, --materialized-summary, --planner-dry-run, or --planner-summary"
        ));
    }

    let selected_modes = publish as u8
        + latest as u8
        + latest_all as u8
        + materialized as u8
        + materialized_summary as u8
        + planner_dry_run as u8
        + planner_summary as u8;
    if selected_modes > 1 {
        return Err(anyhow!(
            "--publish, --latest, --latest-all, --materialized, --materialized-summary, --planner-dry-run, and --planner-summary cannot be combined"
        ));
    }

    if publish {
        return Ok(DgraphSnapshotMode::Publish);
    }

    if latest {
        return Ok(DgraphSnapshotMode::Latest);
    }

    if latest_all {
        return Ok(DgraphSnapshotMode::LatestAll);
    }

    if materialized {
        return Ok(DgraphSnapshotMode::Materialized);
    }

    if materialized_summary {
        return Ok(DgraphSnapshotMode::MaterializedSummary);
    }

    if planner_dry_run {
        return Ok(DgraphSnapshotMode::PlannerDryRun);
    }

    if planner_summary {
        return Ok(DgraphSnapshotMode::PlannerSummary);
    }

    Ok(DgraphSnapshotMode::Local)
}

fn resolve_node_id(cfg: &Config, node_id_override: Option<&str>) -> Result<String> {
    node_id_override
        .map(str::to_string)
        .or_else(|| {
            let trimmed = cfg.agent.node_id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .ok_or_else(|| anyhow!("node_id is required in --node-id or agent.node_id"))
}

fn resolve_project_id(cfg: &Config, project_id_override: Option<&str>) -> Result<String> {
    project_id_override
        .map(str::to_string)
        .or_else(|| cfg.dispatch.as_ref().and_then(|dispatch| dispatch.project_id.clone()))
        .or_else(|| cfg.outputs.http.as_ref().and_then(|http| http.project_id.clone()))
        .ok_or_else(|| {
            anyhow!(
                "project_id is required in --project-id, dispatch.project_id, or outputs.http.project_id"
            )
        })
}

fn resolve_base_url(cfg: &Config, url_override: Option<&str>) -> Result<String> {
    url_override
        .map(str::to_string)
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .filter(|http| http.enabled)
                .map(|http| http.url.clone())
        })
        .ok_or_else(|| {
            anyhow!(
                "outputs.http.enabled=true or --url is required for d-graph snapshot API access"
            )
        })
}

fn print_local_snapshot_output(snapshot: &LocalDGraphSnapshot, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(snapshot)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return;
    }

    print!("{}", format_local_dgraph_snapshot(snapshot));
}

fn format_local_dgraph_snapshot(snapshot: &LocalDGraphSnapshot) -> String {
    let mut lines = vec![snapshot_heading(snapshot, false).to_string()];
    if let Some(scenario) = fixture_scenario_name(snapshot) {
        lines.push(format!("  scenario: {}", scenario));
    }
    append_snapshot_header_lines(&mut lines, snapshot);
    append_snapshot_count_lines(&mut lines, snapshot);

    if !snapshot.modules.is_empty() {
        lines.push(String::new());
        lines.push("Modules:".to_string());
        for module in &snapshot.modules {
            lines.push(format!("  - {}", module.module_id));
            if let Some(runtime) = module.runtime.as_deref() {
                lines.push(format!("    runtime: {}", runtime));
            }
            lines.push(format!("    source: {}", module.source));
            if let Some(container) = module.local_container_name.as_deref() {
                lines.push(format!("    container: {}", container));
            }
            if let Some(desired_state) = module.desired_state.as_deref() {
                lines.push(format!("    desired state: {}", desired_state));
            }
        }
    }

    if !snapshot.flows.is_empty() {
        lines.push(String::new());
        lines.push("Flows:".to_string());
        for flow in &snapshot.flows {
            lines.push(format!("  - {}", flow.flow_id));
            lines.push(format!("    modules: {}", flow.modules.join(", ")));
            lines.push(format!("    edges: {}", flow.edges.len()));
        }
    }

    lines.push(String::new());
    if is_fixture_snapshot(snapshot) {
        lines.push("Note: this is metadata-only fixture data. It did not inspect Docker, execute WASM, deploy modules, or reconfigure flows.".to_string());
    } else {
        lines.push("Note: this command only reports local d-graph metadata. It did not inspect Docker/container state, execute WASM, deploy modules, or modify config.".to_string());
    }
    lines.join("\n") + "\n"
}

fn format_published_snapshot(snapshot: &LocalDGraphSnapshot) -> String {
    let mut lines = vec![snapshot_heading(snapshot, true).to_string()];
    if let Some(scenario) = fixture_scenario_name(snapshot) {
        lines.push(format!("  scenario: {}", scenario));
    }
    if let Some(snapshot_id) = snapshot.snapshot_id.as_deref() {
        lines.push(format!("  snapshot_id: {}", snapshot_id));
    }
    lines.push(format!("  node_id: {}", snapshot.node_id));
    append_snapshot_count_lines(&mut lines, snapshot);
    lines.join("\n") + "\n"
}

fn is_fixture_snapshot(snapshot: &LocalDGraphSnapshot) -> bool {
    snapshot
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("fixture"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn fixture_scenario_name(snapshot: &LocalDGraphSnapshot) -> Option<&str> {
    snapshot
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("fixture_scenario"))
        .and_then(|value| value.as_str())
}

fn snapshot_heading(snapshot: &LocalDGraphSnapshot, published: bool) -> &'static str {
    match (published, is_fixture_snapshot(snapshot)) {
        (true, true) => "Published local d-graph fixture snapshot",
        (true, false) => "Published local d-graph snapshot",
        (false, true) => "Local d-graph fixture snapshot",
        (false, false) => "Local d-graph snapshot",
    }
}

fn format_latest_snapshot(snapshot: &LocalDGraphSnapshot) -> String {
    let mut lines = vec!["Latest local d-graph snapshot".to_string()];
    append_snapshot_header_lines(&mut lines, snapshot);
    append_snapshot_count_lines(&mut lines, snapshot);

    if !snapshot.modules.is_empty() {
        lines.push(String::new());
        lines.push("Modules:".to_string());
        for module in &snapshot.modules {
            lines.push(format!("  - {}", module.module_id));
            if let Some(runtime) = module.runtime.as_deref() {
                lines.push(format!("    runtime: {}", runtime));
            }
            lines.push(format!("    source: {}", module.source));
        }
    }

    lines.push(String::new());
    lines.push("Note: this command read local d-graph snapshot metadata only. It did not run planner logic, deploy modules, or reconfigure flows.".to_string());
    lines.join("\n") + "\n"
}

fn format_latest_snapshot_list(snapshot_list: &LocalDGraphSnapshotList) -> String {
    let mut lines = vec!["Latest local d-graph snapshots".to_string()];
    if let Some(project_id) = snapshot_list.project_id.as_deref() {
        lines.push(format!("  project: {}", project_id));
    }
    lines.push(format!(
        "  count: {}",
        snapshot_list
            .count
            .unwrap_or(snapshot_list.snapshots.len() as u64)
    ));

    if snapshot_list.snapshots.is_empty() {
        lines.push("  snapshots: none".to_string());
    } else {
        for snapshot in &snapshot_list.snapshots {
            lines.push(String::new());
            lines.push(format!("- node: {}", snapshot.node_id));
            if let Some(snapshot_id) = snapshot.snapshot_id.as_deref() {
                lines.push(format!("  snapshot_id: {}", snapshot_id));
            }
            lines.push(format!("  runtimes: {}", snapshot.runtimes.len()));
            lines.push(format!("  modules: {}", snapshot.modules.len()));
            lines.push(format!("  flows: {}", snapshot.flows.len()));
            lines.push(format!(
                "  safe to auto execute: {}",
                snapshot.safe_to_auto_execute
            ));
            lines.push(format!(
                "  side effects performed: {}",
                snapshot.side_effects_performed
            ));
        }
    }

    lines.push(String::new());
    lines.push("Note: these snapshots are metadata only. This command did not run planner logic, deploy modules, or reconfigure flows.".to_string());
    lines.join("\n") + "\n"
}

fn format_materialized_dgraph(
    response: &ProjectDGraphMaterialization,
    requested_project_id: &str,
) -> String {
    let project_id = response
        .project_id
        .as_deref()
        .unwrap_or(requested_project_id);
    let snapshot_count = response.source_snapshot_count.unwrap_or(0);
    let summary = response.summary.as_ref().cloned().unwrap_or_default();
    let node_ids = extract_ids(response.graph.as_ref(), "nodes", "node_id");
    let module_ids = extract_ids(response.graph.as_ref(), "modules", "module_id");

    let mut lines = vec![
        "Project d-graph materialized".to_string(),
        format!("Project: {}", project_id),
        format!("Source snapshots: {}", snapshot_count),
        format!("Nodes: {}", summary.node_count.unwrap_or(0)),
        format!("Runtimes: {}", summary.runtime_count.unwrap_or(0)),
        format!("Modules: {}", summary.module_count.unwrap_or(0)),
        format!("Flows: {}", summary.flow_count.unwrap_or(0)),
        format!("Relationships: {}", summary.relationship_count.unwrap_or(0)),
        format!("Validation errors: {}", response.validation_errors.len()),
        format!(
            "Side effects performed: {}",
            response.side_effects_performed.unwrap_or(false)
        ),
    ];

    if !node_ids.is_empty() {
        lines.push(format!("Node IDs: {}", node_ids.join(", ")));
    }

    if !module_ids.is_empty() {
        lines.push(format!("Module IDs: {}", module_ids.join(", ")));
    } else {
        lines.push("Module details: use --json for full graph".to_string());
    }

    lines.push(String::new());
    lines.push(
        "Note: this command did not run planner, deploy modules, or reconfigure flows.".to_string(),
    );
    lines.join("\n") + "\n"
}

fn format_materialized_dgraph_summary(
    response: &ProjectDGraphMaterializationSummary,
    requested_project_id: &str,
) -> String {
    let project_id = response
        .project_id
        .as_deref()
        .unwrap_or(requested_project_id);
    let snapshot_count = response.source_snapshot_count.unwrap_or(0);
    let summary = response.summary.as_ref().cloned().unwrap_or_default();

    let mut lines = vec![
        "Project d-graph summary".to_string(),
        format!("Project: {}", project_id),
        format!("Source snapshots: {}", snapshot_count),
        format!("Nodes: {}", summary.node_count.unwrap_or(0)),
        format!("Runtimes: {}", summary.runtime_count.unwrap_or(0)),
        format!("Modules: {}", summary.module_count.unwrap_or(0)),
        format!("Artifacts: {}", summary.artifact_count.unwrap_or(0)),
        format!("Flows: {}", summary.flow_count.unwrap_or(0)),
        format!("Relationships: {}", summary.relationship_count.unwrap_or(0)),
        format!(
            "Unhealthy targets: {}",
            summary.unhealthy_targets.unwrap_or(0)
        ),
        format!(
            "Pending deployment intents: {}",
            summary.pending_deployment_intents.unwrap_or(0)
        ),
        format!(
            "Proposed placements: {}",
            summary.proposed_placements.unwrap_or(0)
        ),
        format!(
            "Proposed reconfigurations: {}",
            summary.proposed_reconfigurations.unwrap_or(0)
        ),
        format!(
            "Side effects performed: {}",
            response.side_effects_performed.unwrap_or(false)
        ),
    ];

    lines.push(String::new());
    lines.push("Note: this command only reads materialized d-graph metadata. It did not run planner, deploy modules, or reconfigure flows.".to_string());
    lines.join("\n") + "\n"
}

fn format_planner_dry_run(
    response: &ProjectDGraphPlannerDryRunResponse,
    requested_project_id: &str,
) -> String {
    let project_id = response
        .project_id
        .as_deref()
        .unwrap_or(requested_project_id);
    let snapshot_count = response.source_snapshot_count.unwrap_or(0);
    let planner = response.planner.as_ref().cloned().unwrap_or_default();
    let summary = planner.summary.as_ref().cloned().unwrap_or_default();

    let mut lines = vec![
        "D-graph planner dry-run".to_string(),
        format!("Project: {}", project_id),
        format!("Source snapshots: {}", snapshot_count),
        format!("Status: {}", planner.status.as_deref().unwrap_or("unknown")),
        format!("Placements: {}", planner.placements.len()),
        format!("Reconfigurations: {}", planner.reconfigurations.len()),
        format!("Blocked: {}", planner.blocked.len()),
        format!("Warnings: {}", planner.warnings.len()),
        format!(
            "Side effects performed: {}",
            response.side_effects_performed.unwrap_or(false)
        ),
    ];

    if !response.materialization_validation_errors.is_empty() {
        lines.push(format!(
            "Materialization validation errors: {}",
            response.materialization_validation_errors.len()
        ));
    }

    lines.push(String::new());
    lines.push("Placements:".to_string());
    if planner.placements.is_empty() {
        lines.push("  none".to_string());
    } else {
        for placement in &planner.placements {
            lines.push(format!(
                "  - module: {}",
                placement.module_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    selected node: {}",
                placement.selected_node_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    runtime: {}",
                placement
                    .selected_runtime_id
                    .as_deref()
                    .unwrap_or("unknown")
            ));
            lines.push(format!(
                "    status: {}",
                placement.status.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    reason: {}",
                placement.reason.as_deref().unwrap_or("none")
            ));
        }
    }

    lines.push(String::new());
    lines.push("Reconfigurations:".to_string());
    if planner.reconfigurations.is_empty() {
        lines.push("  none".to_string());
    } else {
        for reconfiguration in &planner.reconfigurations {
            lines.push(format!(
                "  - flow: {}",
                reconfiguration.flow_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    module: {}",
                reconfiguration
                    .affected_module_id
                    .as_deref()
                    .unwrap_or("unknown")
            ));
            lines.push(format!(
                "    from: {}",
                reconfiguration.from_node_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    to: {}",
                reconfiguration.to_node_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    status: {}",
                reconfiguration.status.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    reason: {}",
                reconfiguration.reason.as_deref().unwrap_or("none")
            ));
        }
    }

    lines.push(String::new());
    lines.push("Blocked:".to_string());
    if planner.blocked.is_empty() {
        lines.push("  none".to_string());
    } else {
        for blocked in &planner.blocked {
            lines.push(format!(
                "  - target: {}/{}",
                blocked.target_kind.as_deref().unwrap_or("unknown"),
                blocked.target_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    reason: {}",
                blocked.reason.as_deref().unwrap_or("none")
            ));
        }
    }

    lines.push(String::new());
    lines.push("Warnings:".to_string());
    if planner.warnings.is_empty() {
        lines.push("  none".to_string());
    } else {
        for warning in &planner.warnings {
            lines.push(format!(
                "  - {}",
                warning.reason.as_deref().unwrap_or("unknown")
            ));
        }
    }

    if summary.placement_count.is_some()
        || summary.reconfiguration_count.is_some()
        || summary.blocked_count.is_some()
        || summary.warning_count.is_some()
    {
        lines.push(String::new());
        lines.push(format!(
            "Summary counts: placements={}, reconfigurations={}, blocked={}, warnings={}",
            summary.placement_count.unwrap_or(0),
            summary.reconfiguration_count.unwrap_or(0),
            summary.blocked_count.unwrap_or(0),
            summary.warning_count.unwrap_or(0)
        ));
    }

    lines.push(String::new());
    lines.push(
        "Note: proposed placements and reconfigurations are advisory only. This command did not execute them.".to_string(),
    );
    lines.join("\n") + "\n"
}

fn format_planner_dry_run_summary(
    response: &ProjectDGraphPlannerDryRunSummaryResponse,
    requested_project_id: &str,
) -> String {
    let project_id = response
        .project_id
        .as_deref()
        .unwrap_or(requested_project_id);
    let snapshot_count = response.source_snapshot_count.unwrap_or(0);
    let summary = response.summary.as_ref().cloned().unwrap_or_default();

    let mut lines = vec![
        "D-graph planner dry-run summary".to_string(),
        format!("Project: {}", project_id),
        format!("Source snapshots: {}", snapshot_count),
        format!(
            "Status: {}",
            response.status.as_deref().unwrap_or("unknown")
        ),
        format!("Placements: {}", response.placement_count.unwrap_or(0)),
        format!(
            "Reconfigurations: {}",
            response.reconfiguration_count.unwrap_or(0)
        ),
        format!("Blocked: {}", response.blocked_count.unwrap_or(0)),
        format!("Warnings: {}", response.warning_count.unwrap_or(0)),
        format!(
            "Candidate nodes: {}",
            summary.candidate_node_count.unwrap_or(0)
        ),
        format!(
            "Unhealthy nodes: {}",
            summary.unhealthy_node_count.unwrap_or(0)
        ),
        format!(
            "Degraded flows: {}",
            summary.degraded_flow_count.unwrap_or(0)
        ),
        format!(
            "Side effects performed: {}",
            response.side_effects_performed.unwrap_or(false)
        ),
    ];

    lines.push(String::new());
    lines.push("Note: this command only reads planner dry-run metadata. It did not deploy modules, execute installation, call Docker, or reconfigure flows.".to_string());
    lines.join("\n") + "\n"
}

fn extract_ids(
    graph: Option<&serde_json::Value>,
    array_field: &str,
    id_field: &str,
) -> Vec<String> {
    graph
        .and_then(|graph| graph.get(array_field))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get(id_field).and_then(|value| value.as_str()))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn append_snapshot_header_lines(lines: &mut Vec<String>, snapshot: &LocalDGraphSnapshot) {
    lines.push(format!("  node: {}", snapshot.node_id));
    if let Some(project_id) = snapshot.project_id.as_deref() {
        lines.push(format!("  project: {}", project_id));
    }
    if let Some(snapshot_id) = snapshot.snapshot_id.as_deref() {
        lines.push(format!("  snapshot_id: {}", snapshot_id));
    }
    if let Some(reported_at) = snapshot.reported_at_utc.as_deref() {
        lines.push(format!("  reported at: {}", reported_at));
    }
}

fn append_snapshot_count_lines(lines: &mut Vec<String>, snapshot: &LocalDGraphSnapshot) {
    lines.push(format!("  runtimes: {}", snapshot.runtimes.len()));
    lines.push(format!("  modules: {}", snapshot.modules.len()));
    lines.push(format!("  flows: {}", snapshot.flows.len()));
    lines.push(format!("  health entries: {}", snapshot.health.len()));
    lines.push(format!("  capabilities: {}", snapshot.capabilities.len()));
    lines.push(format!(
        "  safe to auto execute: {}",
        snapshot.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        snapshot.side_effects_performed
    ));
}

#[cfg(test)]
mod tests {
    use super::{
        format_latest_snapshot, format_latest_snapshot_list, format_local_dgraph_snapshot,
        format_materialized_dgraph, format_materialized_dgraph_summary, format_planner_dry_run,
        format_planner_dry_run_summary, format_published_snapshot, resolve_base_url,
        resolve_project_id, validate_dgraph_snapshot_mode, DgraphSnapshotMode,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, FileJsonConfig, HttpConfig, OutputsConfig},
        dgraph_snapshot::{
            materialized::{
                DGraphSummary, ProjectDGraphMaterialization, ProjectDGraphMaterializationSummary,
            },
            model::{
                validate_local_dgraph_snapshot_publish_invariants, LocalCapabilitySnapshot,
                LocalDGraphSnapshot, LocalDGraphSnapshotList, LocalHealthSnapshot,
                LocalModuleSnapshot, LocalRuntimeSnapshot,
            },
            planner::{
                DGraphPlacementDecision, DGraphPlannerBlockedItem, DGraphPlannerDryRun,
                DGraphPlannerSummary, DGraphPlannerWarning, DGraphReconfigurationDecision,
                ProjectDGraphPlannerDryRunResponse, ProjectDGraphPlannerDryRunSummaryResponse,
            },
        },
    };

    fn sample_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: Some(FileJsonConfig {
                    enabled: true,
                    path: "snapshot.json".to_string(),
                }),
                http: Some(HttpConfig {
                    enabled: true,
                    url: "http://localhost:8080/api/v1/snapshots".to_string(),
                    project_id: Some("smart-city-campinas".to_string()),
                    timeout_s: Some(10),
                }),
            },
            dispatch: Some(DispatchConfig {
                project_id: Some("dispatch-project".to_string()),
                ..DispatchConfig::default()
            }),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn sample_snapshot() -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            node_id: "fog-01".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            snapshot_id: Some("snapshot-1".to_string()),
            reported_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
            source: Some("client".to_string()),
            generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: "runtime-container-local".to_string(),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "local_config".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: "chirpstack-primary".to_string(),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("unknown".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: Some("nginx:stable".to_string()),
                local_mapping_key: Some("chirpstack-primary".to_string()),
                local_container_name: Some("chirpstack".to_string()),
                source: "service_registry".to_string(),
                metadata: None,
            }],
            flows: vec![],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: "chirpstack-primary".to_string(),
                status: "unknown".to_string(),
                reason: None,
                source: "local_config".to_string(),
                metadata: None,
            }],
            capabilities: vec![LocalCapabilitySnapshot {
                capability_id: "capability-container-runtime-configured".to_string(),
                capability_kind: "container_runtime_configured".to_string(),
                supported: true,
                source: "local_config".to_string(),
                metadata: None,
            }],
            metadata: Some(serde_json::json!({ "reporting_only": true })),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn cli_mode_validation_accepts_local_publish_and_latest_modes() {
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, false, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::Local
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, true, false, false, false, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::Publish
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, true, false, false, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::Latest
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, true, false, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::LatestAll
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, true, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::Materialized
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, false, true, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::MaterializedSummary
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, false, false, true, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::PlannerDryRun
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, false, false, false, true)
                .expect("mode should validate"),
            DgraphSnapshotMode::PlannerSummary
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(true, false, false, false, false, false, false, false)
                .expect("fixture mode should validate"),
            DgraphSnapshotMode::Local
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(true, true, false, false, false, false, false, false)
                .expect("fixture publish mode should validate"),
            DgraphSnapshotMode::Publish
        ));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        assert!(validate_dgraph_snapshot_mode(
            false, true, true, false, false, false, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, true, false, true, false, false, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, false, true, true, false, false, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, true, false, false, true, false, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, true, false, false, false, true, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, true, false, false, false, false, true, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, true, false, false, false, false, false, true
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, false, false, false, true, false, true, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            false, false, false, false, false, true, false, true
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            true, false, true, false, false, false, false, false
        )
        .is_err());
        assert!(validate_dgraph_snapshot_mode(
            true, false, false, false, false, false, true, false
        )
        .is_err());
    }

    #[test]
    fn local_dgraph_snapshot_without_publish_does_not_require_server_config_helpers() {
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false, false, false, false, false, false)
                .expect("mode should validate"),
            DgraphSnapshotMode::Local
        ));
    }

    #[test]
    fn resolve_project_id_prefers_override() {
        let cfg = sample_config();
        let project_id =
            resolve_project_id(&cfg, Some("override-project")).expect("project id should resolve");
        assert_eq!(project_id, "override-project");
    }

    #[test]
    fn resolve_base_url_uses_output_http() {
        let cfg = sample_config();
        let url = resolve_base_url(&cfg, None).expect("base URL should resolve");
        assert_eq!(url, "http://localhost:8080/api/v1/snapshots");
    }

    #[test]
    fn formatting_local_snapshot_includes_counts_and_read_only_note() {
        let rendered = format_local_dgraph_snapshot(&sample_snapshot());

        assert!(rendered.contains("Local d-graph snapshot"));
        assert!(rendered.contains("node: fog-01"));
        assert!(rendered.contains("project: smart-city-campinas"));
        assert!(rendered.contains("runtimes: 1"));
        assert!(rendered.contains("modules: 1"));
        assert!(rendered.contains("health entries: 1"));
        assert!(rendered.contains("capabilities: 1"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("chirpstack-primary"));
        assert!(rendered.contains("runtime: container"));
        assert!(rendered.contains("source: service_registry"));
        assert!(rendered.contains("container: chirpstack"));
        assert!(rendered.contains(
            "did not inspect Docker/container state, execute WASM, deploy modules, or modify config"
        ));
    }

    #[test]
    fn formatting_fixture_snapshot_includes_scenario_and_fixture_note() {
        let mut snapshot = sample_snapshot();
        snapshot.metadata = Some(serde_json::json!({
            "reporting_only": true,
            "fixture": true,
            "fixture_scenario": "degraded-node"
        }));

        let rendered = format_local_dgraph_snapshot(&snapshot);

        assert!(rendered.contains("Local d-graph fixture snapshot"));
        assert!(rendered.contains("scenario: degraded-node"));
        assert!(rendered.contains(
            "this is metadata-only fixture data. It did not inspect Docker, execute WASM, deploy modules, or reconfigure flows"
        ));
    }

    #[test]
    fn formatting_published_snapshot_includes_snapshot_id_and_counts() {
        let rendered = format_published_snapshot(&sample_snapshot());

        assert!(rendered.contains("Published local d-graph snapshot"));
        assert!(rendered.contains("snapshot_id: snapshot-1"));
        assert!(rendered.contains("node_id: fog-01"));
        assert!(rendered.contains("modules: 1"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
    }

    #[test]
    fn formatting_published_fixture_snapshot_includes_fixture_heading() {
        let mut snapshot = sample_snapshot();
        snapshot.metadata = Some(serde_json::json!({
            "fixture": true,
            "fixture_scenario": "missing-module"
        }));

        let rendered = format_published_snapshot(&snapshot);

        assert!(rendered.contains("Published local d-graph fixture snapshot"));
        assert!(rendered.contains("scenario: missing-module"));
    }

    #[test]
    fn formatting_latest_snapshot_includes_snapshot_metadata_and_note() {
        let rendered = format_latest_snapshot(&sample_snapshot());

        assert!(rendered.contains("Latest local d-graph snapshot"));
        assert!(rendered.contains("snapshot_id: snapshot-1"));
        assert!(rendered.contains("reported at: 2026-05-21T00:00:00Z"));
        assert!(rendered.contains("chirpstack-primary"));
        assert!(
            rendered.contains("did not run planner logic, deploy modules, or reconfigure flows")
        );
    }

    #[test]
    fn formatting_latest_snapshot_list_includes_count_and_node_summary() {
        let rendered = format_latest_snapshot_list(&LocalDGraphSnapshotList {
            project_id: Some("smart-city-campinas".to_string()),
            count: Some(1),
            snapshots: vec![sample_snapshot()],
        });

        assert!(rendered.contains("Latest local d-graph snapshots"));
        assert!(rendered.contains("project: smart-city-campinas"));
        assert!(rendered.contains("count: 1"));
        assert!(rendered.contains("- node: fog-01"));
        assert!(rendered.contains("snapshot_id: snapshot-1"));
        assert!(rendered.contains("modules: 1"));
    }

    #[test]
    fn formatting_materialized_summary_includes_counts_and_false_side_effects() {
        let rendered = format_materialized_dgraph_summary(
            &ProjectDGraphMaterializationSummary {
                project_id: Some("smart-city-campinas".to_string()),
                source_snapshot_count: Some(2),
                summary: Some(DGraphSummary {
                    node_count: Some(2),
                    runtime_count: Some(2),
                    module_count: Some(4),
                    artifact_count: Some(2),
                    flow_count: Some(1),
                    relationship_count: Some(8),
                    unhealthy_targets: Some(0),
                    pending_deployment_intents: Some(0),
                    proposed_placements: Some(0),
                    proposed_reconfigurations: Some(0),
                }),
                validation_errors: Vec::new(),
                side_effects_performed: Some(false),
            },
            "smart-city-campinas",
        );

        assert!(rendered.contains("Project d-graph summary"));
        assert!(rendered.contains("Project: smart-city-campinas"));
        assert!(rendered.contains("Source snapshots: 2"));
        assert!(rendered.contains("Nodes: 2"));
        assert!(rendered.contains("Modules: 4"));
        assert!(rendered.contains("Relationships: 8"));
        assert!(rendered.contains("Side effects performed: false"));
    }

    #[test]
    fn formatting_materialized_view_includes_validation_error_count_and_ids() {
        let rendered = format_materialized_dgraph(
            &ProjectDGraphMaterialization {
                project_id: Some("smart-city-campinas".to_string()),
                generated_at_utc: Some("2026-05-21T12:00:00Z".to_string()),
                source_snapshot_count: Some(2),
                graph: Some(serde_json::json!({
                    "nodes":[{"node_id":"fog-01"},{"node_id":"fog-02"}],
                    "modules":[{"module_id":"collector"},{"module_id":"broker"}]
                })),
                summary: Some(DGraphSummary {
                    node_count: Some(2),
                    runtime_count: Some(2),
                    module_count: Some(4),
                    artifact_count: Some(2),
                    flow_count: Some(1),
                    relationship_count: Some(8),
                    unhealthy_targets: Some(0),
                    pending_deployment_intents: Some(0),
                    proposed_placements: Some(0),
                    proposed_reconfigurations: Some(0),
                }),
                validation_errors: vec!["missing module reference".to_string()],
                side_effects_performed: Some(false),
                metadata: None,
            },
            "smart-city-campinas",
        );

        assert!(rendered.contains("Project d-graph materialized"));
        assert!(rendered.contains("Source snapshots: 2"));
        assert!(rendered.contains("Validation errors: 1"));
        assert!(rendered.contains("Node IDs: fog-01, fog-02"));
        assert!(rendered.contains("Module IDs: collector, broker"));
        assert!(rendered.contains("Side effects performed: false"));
    }

    #[test]
    fn formatting_planner_summary_includes_project_status_counts_and_false_side_effects() {
        let rendered = format_planner_dry_run_summary(
            &ProjectDGraphPlannerDryRunSummaryResponse {
                project_id: Some("smart-city-campinas".to_string()),
                source_snapshot_count: Some(2),
                status: Some("planned".to_string()),
                summary: Some(DGraphPlannerSummary {
                    placement_count: Some(1),
                    reconfiguration_count: Some(0),
                    blocked_count: Some(0),
                    warning_count: Some(0),
                    candidate_node_count: Some(2),
                    unhealthy_node_count: Some(0),
                    degraded_flow_count: Some(0),
                }),
                placement_count: Some(1),
                reconfiguration_count: Some(0),
                blocked_count: Some(0),
                warning_count: Some(0),
                side_effects_performed: Some(false),
                metadata: Some(serde_json::json!({"planner_mode":"dry_run"})),
            },
            "smart-city-campinas",
        );

        assert!(rendered.contains("D-graph planner dry-run summary"));
        assert!(rendered.contains("Project: smart-city-campinas"));
        assert!(rendered.contains("Source snapshots: 2"));
        assert!(rendered.contains("Status: planned"));
        assert!(rendered.contains("Placements: 1"));
        assert!(rendered.contains("Candidate nodes: 2"));
        assert!(rendered.contains("Side effects performed: false"));
        assert!(rendered.contains(
            "did not deploy modules, execute installation, call Docker, or reconfigure flows"
        ));
    }

    #[test]
    fn formatting_planner_dry_run_displays_placements_reconfigurations_blocked_and_warnings() {
        let rendered = format_planner_dry_run(
            &ProjectDGraphPlannerDryRunResponse {
                project_id: Some("smart-city-campinas".to_string()),
                source_snapshot_count: Some(2),
                materialization_validation_errors: vec!["partial graph".to_string()],
                planner: Some(DGraphPlannerDryRun {
                    project_id: Some("smart-city-campinas".to_string()),
                    planner_version: Some("0.1.0".to_string()),
                    generated_at_utc: Some("2026-05-21T12:00:00Z".to_string()),
                    status: Some("partially_blocked".to_string()),
                    placements: vec![DGraphPlacementDecision {
                        placement_id: Some("p1".to_string()),
                        module_id: Some("chirpstack-primary".to_string()),
                        selected_node_id: Some("fog-01".to_string()),
                        selected_runtime_id: Some("container:fog-01".to_string()),
                        reason: Some("available node".to_string()),
                        status: Some("proposed".to_string()),
                        metadata: None,
                    }],
                    reconfigurations: vec![DGraphReconfigurationDecision {
                        reconfiguration_id: Some("r1".to_string()),
                        flow_id: Some("flow-a".to_string()),
                        affected_module_id: Some("broker".to_string()),
                        from_node_id: Some("fog-01".to_string()),
                        to_node_id: Some("fog-02".to_string()),
                        reason: Some("failover".to_string()),
                        status: Some("proposed".to_string()),
                        metadata: None,
                    }],
                    blocked: vec![DGraphPlannerBlockedItem {
                        item_id: Some("b1".to_string()),
                        target_kind: Some("module".to_string()),
                        target_id: Some("chirpstack-primary".to_string()),
                        reason: Some("no candidate".to_string()),
                        metadata: None,
                    }],
                    warnings: vec![DGraphPlannerWarning {
                        warning_id: Some("w1".to_string()),
                        reason: Some("runtime fallback".to_string()),
                        metadata: None,
                    }],
                    summary: Some(DGraphPlannerSummary {
                        placement_count: Some(1),
                        reconfiguration_count: Some(1),
                        blocked_count: Some(1),
                        warning_count: Some(1),
                        candidate_node_count: Some(2),
                        unhealthy_node_count: Some(0),
                        degraded_flow_count: Some(0),
                    }),
                    side_effects_performed: Some(false),
                    metadata: None,
                }),
                side_effects_performed: Some(false),
                metadata: None,
            },
            "smart-city-campinas",
        );

        assert!(rendered.contains("D-graph planner dry-run"));
        assert!(rendered.contains("Status: partially_blocked"));
        assert!(rendered.contains("Placements: 1"));
        assert!(rendered.contains("Reconfigurations: 1"));
        assert!(rendered.contains("Blocked: 1"));
        assert!(rendered.contains("Warnings: 1"));
        assert!(rendered.contains("module: chirpstack-primary"));
        assert!(rendered.contains("selected node: fog-01"));
        assert!(rendered.contains("flow: flow-a"));
        assert!(rendered.contains("target: module/chirpstack-primary"));
        assert!(rendered.contains("runtime fallback"));
        assert!(rendered.contains("advisory only"));
    }

    #[test]
    fn formatting_planner_dry_run_shows_none_for_empty_sections() {
        let rendered = format_planner_dry_run(
            &ProjectDGraphPlannerDryRunResponse {
                project_id: Some("smart-city-campinas".to_string()),
                source_snapshot_count: Some(0),
                materialization_validation_errors: Vec::new(),
                planner: Some(DGraphPlannerDryRun {
                    project_id: Some("smart-city-campinas".to_string()),
                    planner_version: Some("0.1.0".to_string()),
                    generated_at_utc: None,
                    status: Some("empty".to_string()),
                    placements: Vec::new(),
                    reconfigurations: Vec::new(),
                    blocked: Vec::new(),
                    warnings: Vec::new(),
                    summary: Some(DGraphPlannerSummary::default()),
                    side_effects_performed: Some(false),
                    metadata: None,
                }),
                side_effects_performed: Some(false),
                metadata: None,
            },
            "smart-city-campinas",
        );

        assert!(rendered.contains("Placements:\n  none"));
        assert!(rendered.contains("Reconfigurations:\n  none"));
        assert!(rendered.contains("Blocked:\n  none"));
        assert!(rendered.contains("Warnings:\n  none"));
    }

    #[test]
    fn local_invariant_check_rejects_safe_to_auto_execute_true() {
        let snapshot = LocalDGraphSnapshot {
            safe_to_auto_execute: true,
            ..sample_snapshot()
        };

        assert!(validate_local_dgraph_snapshot_publish_invariants(&snapshot).is_err());
    }

    #[test]
    fn local_invariant_check_rejects_side_effects_performed_true() {
        let snapshot = LocalDGraphSnapshot {
            side_effects_performed: true,
            ..sample_snapshot()
        };

        assert!(validate_local_dgraph_snapshot_publish_invariants(&snapshot).is_err());
    }

    #[test]
    fn source_guard_confirms_no_execution_or_config_write_patterns() {
        let source = include_str!("dgraph_snapshot.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
            "run_embedded_wasm",
            "plan_dgraph_dry_run",
            "dispatch --execute",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "dgraph snapshot CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}

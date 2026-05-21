use anyhow::{anyhow, Result};
use serde_json::json;

use crate::{
    config::Config,
    dgraph_snapshot::{
        build_local_dgraph_snapshot,
        client::{
            fetch_latest_local_dgraph_snapshot, fetch_latest_local_dgraph_snapshots,
            publish_local_dgraph_snapshot,
        },
        validate_local_dgraph_snapshot_publish_invariants, LocalDGraphSnapshot,
        LocalDGraphSnapshotList,
    },
};

enum DgraphSnapshotMode {
    Local,
    Publish,
    Latest,
    LatestAll,
}

pub fn run_dgraph_snapshot_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    publish: bool,
    latest: bool,
    latest_all: bool,
    json_output: bool,
) -> Result<()> {
    let mode = validate_dgraph_snapshot_mode(publish, latest, latest_all)?;

    match mode {
        DgraphSnapshotMode::Local => {
            let snapshot = build_local_dgraph_snapshot(cfg);
            print_local_snapshot_output(&snapshot, json_output);
            Ok(())
        }
        DgraphSnapshotMode::Publish => {
            let snapshot = build_local_dgraph_snapshot(cfg);
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
            println!("Note: this command published local d-graph snapshot metadata only. It did not run planner logic, deploy modules, or reconfigure flows.");
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
    }
}

fn validate_dgraph_snapshot_mode(
    publish: bool,
    latest: bool,
    latest_all: bool,
) -> Result<DgraphSnapshotMode> {
    let selected_modes = publish as u8 + latest as u8 + latest_all as u8;
    if selected_modes > 1 {
        return Err(anyhow!(
            "--publish, --latest, and --latest-all cannot be combined"
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
    let mut lines = vec!["Local d-graph snapshot".to_string()];
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
    lines.push("Note: this command only reports local d-graph metadata. It did not inspect Docker/container state, execute WASM, deploy modules, or modify config.".to_string());
    lines.join("\n") + "\n"
}

fn format_published_snapshot(snapshot: &LocalDGraphSnapshot) -> String {
    let mut lines = vec!["Published local d-graph snapshot".to_string()];
    if let Some(snapshot_id) = snapshot.snapshot_id.as_deref() {
        lines.push(format!("  snapshot_id: {}", snapshot_id));
    }
    lines.push(format!("  node_id: {}", snapshot.node_id));
    append_snapshot_count_lines(&mut lines, snapshot);
    lines.join("\n") + "\n"
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
        format_published_snapshot, resolve_base_url, resolve_project_id,
        validate_dgraph_snapshot_mode, DgraphSnapshotMode,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, FileJsonConfig, HttpConfig, OutputsConfig},
        dgraph_snapshot::model::{
            validate_local_dgraph_snapshot_publish_invariants, LocalCapabilitySnapshot,
            LocalDGraphSnapshot, LocalDGraphSnapshotList, LocalHealthSnapshot, LocalModuleSnapshot,
            LocalRuntimeSnapshot,
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
            validate_dgraph_snapshot_mode(false, false, false).expect("mode should validate"),
            DgraphSnapshotMode::Local
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(true, false, false).expect("mode should validate"),
            DgraphSnapshotMode::Publish
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, true, false).expect("mode should validate"),
            DgraphSnapshotMode::Latest
        ));
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, true).expect("mode should validate"),
            DgraphSnapshotMode::LatestAll
        ));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        assert!(validate_dgraph_snapshot_mode(true, true, false).is_err());
        assert!(validate_dgraph_snapshot_mode(true, false, true).is_err());
        assert!(validate_dgraph_snapshot_mode(false, true, true).is_err());
    }

    #[test]
    fn local_dgraph_snapshot_without_publish_does_not_require_server_config_helpers() {
        assert!(matches!(
            validate_dgraph_snapshot_mode(false, false, false).expect("mode should validate"),
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
        ] {
            assert!(
                !implementation.contains(forbidden),
                "dgraph snapshot CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}

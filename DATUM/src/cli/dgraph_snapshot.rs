use anyhow::Result;

use crate::{
    config::Config,
    dgraph_snapshot::{build_local_dgraph_snapshot, LocalDGraphSnapshot},
};

pub fn run_dgraph_snapshot_command(cfg: &Config, json_output: bool) -> Result<()> {
    let snapshot = build_local_dgraph_snapshot(cfg);

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return Ok(());
    }

    print!("{}", format_local_dgraph_snapshot(&snapshot));
    Ok(())
}

fn format_local_dgraph_snapshot(snapshot: &LocalDGraphSnapshot) -> String {
    let mut lines = vec!["Local d-graph snapshot".to_string()];
    lines.push(format!("  node: {}", snapshot.node_id));
    if let Some(project_id) = snapshot.project_id.as_deref() {
        lines.push(format!("  project: {}", project_id));
    }
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

#[cfg(test)]
mod tests {
    use super::format_local_dgraph_snapshot;
    use crate::dgraph_snapshot::model::{
        LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalHealthSnapshot, LocalModuleSnapshot,
        LocalRuntimeSnapshot,
    };

    #[test]
    fn cli_formatting_includes_counts_and_read_only_note() {
        let rendered = format_local_dgraph_snapshot(&LocalDGraphSnapshot {
            node_id: "fog-01".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            generated_at_utc: None,
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
                artifact_ref: None,
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
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        });

        assert!(rendered.contains("Local d-graph snapshot"));
        assert!(rendered.contains("node: fog-01"));
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
        assert!(rendered.contains("did not inspect Docker/container state, execute WASM, deploy modules, or modify config"));
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

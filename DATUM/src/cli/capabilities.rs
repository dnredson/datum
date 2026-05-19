use anyhow::{anyhow, Result};

use crate::{
    capabilities::{
        builder::build_node_capabilities,
        client::publish_node_capabilities,
        model::{ContainerCapabilities, NodeCapabilities, ProcessCapabilities, WasmCapabilities},
    },
    config::Config,
};

pub fn run_capabilities_command(cfg: &Config, publish: bool) -> Result<()> {
    let project_id = resolve_project_id(cfg)?;
    let node_id = cfg.agent.node_id.clone();
    let capabilities = build_node_capabilities(cfg, &node_id, &project_id);

    print_capabilities(&capabilities, &node_id, &project_id);

    if publish {
        let http_cfg = cfg
            .outputs
            .http
            .as_ref()
            .filter(|http| http.enabled)
            .ok_or_else(|| {
                anyhow!("outputs.http.enabled=true is required to publish capabilities")
            })?;

        let response = publish_node_capabilities(http_cfg, &project_id, &node_id, &capabilities)?;
        println!();
        println!("Published capabilities for node {}", node_id);
        println!("Project: {}", project_id);
        println!("Server response: ok");
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string())
        );
    }

    Ok(())
}

fn resolve_project_id(cfg: &Config) -> Result<String> {
    cfg.dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.project_id.clone())
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .and_then(|http| http.project_id.clone())
        })
        .ok_or_else(|| {
            anyhow!("project_id is required in dispatch.project_id or outputs.http.project_id")
        })
}

fn print_capabilities(capabilities: &NodeCapabilities, node_id: &str, project_id: &str) {
    println!("Capabilities for node {}", node_id);
    println!("Project: {}", project_id);
    println!(
        "Stage: {}",
        capabilities.stage.as_deref().unwrap_or("unknown")
    );

    print_container(capabilities.runtimes.container.as_ref());
    print_wasm(capabilities.runtimes.wasm.as_ref());
    print_process(capabilities.runtimes.process.as_ref());

    if let Some(flow) = capabilities.flow.as_ref() {
        println!("Flow:");
        println!(
            "  mqtt_reconfigure: {}",
            flow.supports_mqtt_reconfigure.unwrap_or(false)
        );
        println!(
            "  http_reconfigure: {}",
            flow.supports_http_reconfigure.unwrap_or(false)
        );
        println!(
            "  bridge_reconfigure: {}",
            flow.supports_bridge_reconfigure.unwrap_or(false)
        );
        println!(
            "  proxy_reconfigure: {}",
            flow.supports_proxy_reconfigure.unwrap_or(false)
        );
    }
}

fn print_container(container: Option<&ContainerCapabilities>) {
    println!("Container:");
    if let Some(container) = container {
        println!("  available: {}", container.available);
        println!(
            "  engine: {}",
            container.engine.as_deref().unwrap_or("<none>")
        );
        println!(
            "  allow_execution: {}",
            container.allow_execution.unwrap_or(false)
        );
        println!(
            "  supports_existing_start: {}",
            container.supports_existing_start.unwrap_or(false)
        );
        println!(
            "  supports_restart: {}",
            container.supports_restart.unwrap_or(false)
        );
        println!(
            "  supports_create: {}",
            container.supports_create.unwrap_or(false)
        );
        println!(
            "  supports_pull: {}",
            container.supports_pull.unwrap_or(false)
        );
    } else {
        println!("  available: false");
    }
}

fn print_wasm(wasm: Option<&WasmCapabilities>) {
    println!("WASM:");
    if let Some(wasm) = wasm {
        println!("  available: {}", wasm.available);
    } else {
        println!("  available: false");
    }
}

fn print_process(process: Option<&ProcessCapabilities>) {
    println!("Process:");
    if let Some(process) = process {
        println!("  available: {}", process.available);
        println!(
            "  allow_execution: {}",
            process.allow_execution.unwrap_or(false)
        );
        println!(
            "  supports_restart_command: {}",
            process.supports_restart_command.unwrap_or(false)
        );
    } else {
        println!("  available: false");
    }
}

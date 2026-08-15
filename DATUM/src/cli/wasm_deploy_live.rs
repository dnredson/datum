use std::fs;

use anyhow::{anyhow, Context, Result};

use crate::{
    config,
    wasm_runtime::{
        deployment::{ArtifactAcquisitionPolicy, DatumNodeScopedDeployment},
        generic_runner::WasmScalarValue,
        live_deployment::{
            execute_prepared_assignment_live, prepare_single_assignment, LiveDeploymentContext,
            LiveWasmInvocationRequest,
        },
    },
};

pub fn run_wasm_deploy_live_command(
    config_path: &str,
    node_id_override: Option<&str>,
    deployment_path: &str,
    assignment_id: &str,
    scalar_args: &[String],
    execute: bool,
    invoked_by: &str,
    allow_insecure_http: bool,
    max_artifact_bytes: Option<usize>,
    acquisition_timeout_ms: Option<u64>,
    json_output: bool,
) -> Result<()> {
    if !execute {
        return Err(anyhow!(
            "live deployment is execution-capable; pass --execute to explicitly authorize this invocation"
        ));
    }

    let local_node_id = resolve_node_id(config_path, node_id_override)?;
    let deployment_text = fs::read_to_string(deployment_path).with_context(|| {
        format!("read node-scoped deployment JSON from {deployment_path}")
    })?;
    let deployment: DatumNodeScopedDeployment = serde_json::from_str(&deployment_text)
        .with_context(|| format!("parse node-scoped deployment JSON from {deployment_path}"))?;

    let mut policy = ArtifactAcquisitionPolicy::default();
    policy.allow_insecure_http = allow_insecure_http;
    if let Some(max_artifact_bytes) = max_artifact_bytes {
        policy.max_artifact_bytes = max_artifact_bytes;
    }
    if let Some(acquisition_timeout_ms) = acquisition_timeout_ms {
        policy.request_timeout_ms = acquisition_timeout_ms;
    }

    let args = scalar_args
        .iter()
        .map(|value| parse_scalar_arg(value))
        .collect::<Result<Vec<_>>>()?;
    let (prepared, limits) =
        prepare_single_assignment(&deployment, &local_node_id, assignment_id, &policy)?;
    let context = LiveDeploymentContext {
        manifest_id: deployment.manifest_id.clone(),
        project_id: deployment.project_id.clone(),
        node_id: local_node_id,
    };
    let report = execute_prepared_assignment_live(
        &prepared,
        &limits,
        &context,
        &LiveWasmInvocationRequest {
            invoked_by: invoked_by.to_string(),
            explicit_execute: true,
            args,
        },
    )?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .unwrap_or_else(|_| "{\"error\":\"failed to render live execution report\"}".to_string())
        );
        return Ok(());
    }

    println!("C66 live WASM deployment execution");
    println!("  execution_id: {}", report.execution_id);
    println!("  node_id: {}", report.node_id);
    println!("  assignment_id: {}", report.assignment_id);
    println!("  module_id: {}", report.module_id);
    println!("  function: {}", report.function_name);
    println!("  artifact: {}", report.artifact_content_id);
    println!("  status: {}", report.status);
    println!("  integrity_verified: {}", report.integrity_verified);
    println!("  wasmtime_instantiated: {}", report.wasmtime_instantiated);
    println!("  guest_execution_performed: {}", report.guest_execution_performed);
    println!("  fuel_consumed: {:?}", report.fuel_consumed);
    println!("  timeout_enforced: {}", report.timeout_enforced);
    println!("  governed_execution_performed: {}", report.governed_execution_performed);
    println!("  results: {:?}", report.results);
    if let Some(message) = report.message.as_deref() {
        println!("  message: {message}");
    }
    Ok(())
}

fn resolve_node_id(config_path: &str, node_id_override: Option<&str>) -> Result<String> {
    if let Some(node_id) = node_id_override {
        let node_id = node_id.trim();
        if node_id.is_empty() {
            return Err(anyhow!("--node-id must not be empty"));
        }
        return Ok(node_id.to_string());
    }

    let cfg = config::load_config(config_path)?;
    let node_id = cfg.agent.node_id.trim();
    if node_id.is_empty() {
        return Err(anyhow!("node_id is required in --node-id or agent.node_id"));
    }
    Ok(node_id.to_string())
}

pub fn parse_scalar_arg(value: &str) -> Result<WasmScalarValue> {
    let (kind, raw) = value.split_once(':').ok_or_else(|| {
        anyhow!(
            "WASM argument '{value}' must use TYPE:VALUE syntax (i32, i64, f32, or f64)"
        )
    })?;

    match kind.trim().to_ascii_lowercase().as_str() {
        "i32" => raw
            .trim()
            .parse::<i32>()
            .map(WasmScalarValue::I32)
            .map_err(|error| anyhow!("invalid i32 WASM argument '{value}': {error}")),
        "i64" => raw
            .trim()
            .parse::<i64>()
            .map(WasmScalarValue::I64)
            .map_err(|error| anyhow!("invalid i64 WASM argument '{value}': {error}")),
        "f32" => raw
            .trim()
            .parse::<f32>()
            .map(WasmScalarValue::F32)
            .map_err(|error| anyhow!("invalid f32 WASM argument '{value}': {error}")),
        "f64" => raw
            .trim()
            .parse::<f64>()
            .map(WasmScalarValue::F64)
            .map_err(|error| anyhow!("invalid f64 WASM argument '{value}': {error}")),
        other => Err(anyhow!(
            "unsupported WASM scalar argument type '{other}'; expected i32, i64, f32, or f64"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_scalar_arg;
    use crate::wasm_runtime::generic_runner::WasmScalarValue;

    #[test]
    fn parses_supported_scalar_arguments() {
        assert_eq!(parse_scalar_arg("i32:7").unwrap(), WasmScalarValue::I32(7));
        assert_eq!(parse_scalar_arg("i64:42").unwrap(), WasmScalarValue::I64(42));
        assert_eq!(parse_scalar_arg("f32:1.5").unwrap(), WasmScalarValue::F32(1.5));
        assert_eq!(parse_scalar_arg("f64:2.25").unwrap(), WasmScalarValue::F64(2.25));
    }

    #[test]
    fn rejects_argument_without_type_prefix() {
        let error = parse_scalar_arg("7").expect_err("untyped argument must fail");
        assert!(error.to_string().contains("TYPE:VALUE"));
    }

    #[test]
    fn rejects_unsupported_scalar_type() {
        let error = parse_scalar_arg("v128:0").expect_err("v128 must remain blocked");
        assert!(error.to_string().contains("unsupported"));
    }
}

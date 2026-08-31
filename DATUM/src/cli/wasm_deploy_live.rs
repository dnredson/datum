use std::{fs, time::Duration};

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, redirect::Policy as RedirectPolicy, Url};
use serde::Serialize;

use crate::{
    config,
    wasm_runtime::{
        deployment::{
            validate_node_scoped_deployment_for_local_node, ArtifactAcquisitionPolicy,
            DatumNodeDeploymentScope, DatumNodeScopedDeployment,
        },
        generic_runner::WasmScalarValue,
        live_deployment::{
            execute_prepared_assignment_live, prepare_single_assignment, LiveDeploymentContext,
            LiveWasmExecutionReport, LiveWasmInvocationRequest,
        },
    },
};

const CONTROL_PLANE_ASSIGNMENT_SELECTOR: &str = "auto";
const CONTROL_PLANE_INBOX_SELECTOR: &str = "inbox";
const DEFAULT_CONTROL_PLANE_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Serialize)]
struct SentinelBoundDeploymentResult {
    mode: &'static str,
    node_id: String,
    assignment_id: String,
    slice_url: String,
    report_url: String,
    status: String,
    artifact_content_id: String,
    results: Vec<WasmScalarValue>,
    fuel_consumed: Option<u64>,
    report_published: bool,
    execution_id: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct SentinelDeploymentInboxResult {
    mode: &'static str,
    node_id: String,
    status: &'static str,
    slice_url: String,
    manifest_id: Option<String>,
    project_id: Option<String>,
    delivery_scope: Option<&'static str>,
    assignment_count: usize,
    assignment_ids: Vec<String>,
    control_plane_fetch_performed: bool,
    artifact_acquisition_performed: bool,
    wasmtime_compiled: bool,
    guest_execution_performed: bool,
    report_published: bool,
    governed_execution_performed: bool,
    deployment_side_effects_performed: bool,
}

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
    if is_control_plane_mode(deployment_path) && is_inbox_selector(assignment_id) {
        return run_sentinel_bound_inbox_discovery(
            config_path,
            node_id_override,
            deployment_path,
            scalar_args,
            execute,
            allow_insecure_http,
            max_artifact_bytes,
            acquisition_timeout_ms,
            json_output,
        );
    }

    if !execute {
        return Err(anyhow!(
            "live deployment is execution-capable; pass --execute to explicitly authorize this invocation"
        ));
    }

    if is_control_plane_mode(deployment_path) {
        return run_sentinel_bound_control_plane_mode(
            config_path,
            node_id_override,
            deployment_path,
            assignment_id,
            scalar_args,
            invoked_by,
            allow_insecure_http,
            max_artifact_bytes,
            acquisition_timeout_ms,
            json_output,
        );
    }

    run_legacy_file_mode(
        config_path,
        node_id_override,
        deployment_path,
        assignment_id,
        scalar_args,
        invoked_by,
        allow_insecure_http,
        max_artifact_bytes,
        acquisition_timeout_ms,
        json_output,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_legacy_file_mode(
    config_path: &str,
    node_id_override: Option<&str>,
    deployment_path: &str,
    assignment_id: &str,
    scalar_args: &[String],
    invoked_by: &str,
    allow_insecure_http: bool,
    max_artifact_bytes: Option<usize>,
    acquisition_timeout_ms: Option<u64>,
    json_output: bool,
) -> Result<()> {
    let local_node_id = resolve_legacy_node_id(config_path, node_id_override)?;
    let deployment_text = fs::read_to_string(deployment_path).with_context(|| {
        format!("read node-scoped deployment JSON from {deployment_path}")
    })?;
    let deployment: DatumNodeScopedDeployment = serde_json::from_str(&deployment_text)
        .with_context(|| format!("parse node-scoped deployment JSON from {deployment_path}"))?;

    let policy = artifact_policy(
        allow_insecure_http,
        max_artifact_bytes,
        acquisition_timeout_ms,
    );
    let args = parse_scalar_args(scalar_args)?;
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

    render_legacy_report(&report, json_output)
}

#[allow(clippy::too_many_arguments)]
fn run_sentinel_bound_control_plane_mode(
    config_path: &str,
    node_id_override: Option<&str>,
    dserver_base: &str,
    assignment_selector: &str,
    scalar_args: &[String],
    invoked_by: &str,
    allow_insecure_http: bool,
    max_artifact_bytes: Option<usize>,
    acquisition_timeout_ms: Option<u64>,
    json_output: bool,
) -> Result<()> {
    validate_bound_mode_cli(node_id_override, assignment_selector)?;

    let cfg = config::load_config(config_path)?;
    let local_node_id = normalize_bound_node_id(&cfg.agent.node_id)?;
    let base_url = parse_control_plane_base_url(dserver_base, allow_insecure_http)?;
    let timeout_ms = acquisition_timeout_ms.unwrap_or(DEFAULT_CONTROL_PLANE_TIMEOUT_MS);
    if timeout_ms == 0 {
        return Err(anyhow!("control-plane timeout must be positive"));
    }
    let client = build_control_plane_client(timeout_ms)?;

    let slice_url = control_plane_endpoint(
        &base_url,
        &[
            "api",
            "v1",
            "controlled-wasm",
            "deployment",
            "for-node",
            &local_node_id,
        ],
    )?;
    let report_url = control_plane_endpoint(
        &base_url,
        &["api", "v1", "controlled-wasm", "live-reports"],
    )?;

    let response = client
        .get(slice_url.clone())
        .send()
        .with_context(|| format!("fetch Sentinel-bound deployment slice from {slice_url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!(
            "fetch Sentinel-bound deployment slice failed with HTTP {status}: {body}"
        ));
    }
    let deployment: DatumNodeScopedDeployment = response
        .json()
        .context("parse Sentinel-bound node-scoped deployment response")?;

    validate_node_scoped_deployment_for_local_node(&deployment, &local_node_id)?;
    let selected_assignment_id = select_single_assignment_id(&deployment)?;
    let policy = artifact_policy(
        allow_insecure_http,
        max_artifact_bytes,
        acquisition_timeout_ms,
    );
    let args = parse_scalar_args(scalar_args)?;
    let (prepared, limits) = prepare_single_assignment(
        &deployment,
        &local_node_id,
        &selected_assignment_id,
        &policy,
    )?;
    let context = LiveDeploymentContext {
        manifest_id: deployment.manifest_id.clone(),
        project_id: deployment.project_id.clone(),
        node_id: local_node_id.clone(),
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

    if report.status != "executed" {
        let detail = serde_json::to_string(&report)
            .unwrap_or_else(|_| "failed to serialize non-executed report".to_string());
        return Err(anyhow!(
            "Sentinel-bound live execution did not reach executed state; report={detail}"
        ));
    }

    let response = client
        .post(report_url.clone())
        .json(&report)
        .send()
        .with_context(|| format!("publish Sentinel live report to {report_url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!(
            "publish Sentinel live report failed with HTTP {status}: {body}"
        ));
    }
    let accepted: LiveWasmExecutionReport = response
        .json()
        .context("parse accepted Sentinel live report response")?;
    if accepted.execution_id != report.execution_id {
        return Err(anyhow!(
            "dserver acknowledged execution_id '{}' but Sentinel published '{}'",
            accepted.execution_id,
            report.execution_id
        ));
    }

    let result = SentinelBoundDeploymentResult {
        mode: "sentinel_bound_control_plane",
        node_id: local_node_id,
        assignment_id: selected_assignment_id,
        slice_url: slice_url.to_string(),
        report_url: report_url.to_string(),
        status: report.status.clone(),
        artifact_content_id: report.artifact_content_id.clone(),
        results: report.results.clone(),
        fuel_consumed: report.fuel_consumed,
        report_published: true,
        execution_id: report.execution_id.clone(),
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| {
                "{\"error\":\"failed to render Sentinel-bound deployment result\"}".to_string()
            })
        );
        return Ok(());
    }

    println!("I35 Sentinel-bound WASM deployment cycle");
    println!("  node_id: {}", result.node_id);
    println!("  assignment_id: {}", result.assignment_id);
    println!("  slice_url: {}", result.slice_url);
    println!("  status: {}", result.status);
    println!("  artifact: {}", result.artifact_content_id);
    println!("  fuel_consumed: {:?}", result.fuel_consumed);
    println!("  report_published: {}", result.report_published);
    println!("  execution_id: {}", result.execution_id);
    println!("  results: {:?}", result.results);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_sentinel_bound_inbox_discovery(
    config_path: &str,
    node_id_override: Option<&str>,
    dserver_base: &str,
    scalar_args: &[String],
    execute: bool,
    allow_insecure_http: bool,
    max_artifact_bytes: Option<usize>,
    acquisition_timeout_ms: Option<u64>,
    json_output: bool,
) -> Result<()> {
    validate_inbox_mode_cli(node_id_override, scalar_args, execute, max_artifact_bytes)?;

    let cfg = config::load_config(config_path)?;
    let local_node_id = normalize_bound_node_id(&cfg.agent.node_id)?;
    let base_url = parse_control_plane_base_url(dserver_base, allow_insecure_http)?;
    let timeout_ms = acquisition_timeout_ms.unwrap_or(DEFAULT_CONTROL_PLANE_TIMEOUT_MS);
    if timeout_ms == 0 {
        return Err(anyhow!("control-plane timeout must be positive"));
    }
    let client = build_control_plane_client(timeout_ms)?;
    let slice_url = control_plane_endpoint(
        &base_url,
        &[
            "api",
            "v1",
            "controlled-wasm",
            "deployment",
            "for-node",
            &local_node_id,
        ],
    )?;

    let response = client
        .get(slice_url.clone())
        .send()
        .with_context(|| format!("discover Sentinel deployment inbox from {slice_url}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return render_inbox_result(
            build_inbox_result(&local_node_id, &slice_url, None)?,
            json_output,
        );
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!(
            "discover Sentinel deployment inbox failed with HTTP {status}: {body}"
        ));
    }

    let deployment: DatumNodeScopedDeployment = response
        .json()
        .context("parse Sentinel deployment inbox node-scoped response")?;
    validate_node_scoped_deployment_for_local_node(&deployment, &local_node_id)?;

    render_inbox_result(
        build_inbox_result(&local_node_id, &slice_url, Some(&deployment))?,
        json_output,
    )
}

fn build_control_plane_client(timeout_ms: u64) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .redirect(RedirectPolicy::none())
        .build()
        .context("build bounded Sentinel control-plane HTTP client")
}

fn build_inbox_result(
    local_node_id: &str,
    slice_url: &Url,
    deployment: Option<&DatumNodeScopedDeployment>,
) -> Result<SentinelDeploymentInboxResult> {
    let (manifest_id, project_id, delivery_scope, assignment_ids) = match deployment {
        Some(deployment) => {
            validate_node_scoped_deployment_for_local_node(deployment, local_node_id)?;
            let assignment_ids = deployment
                .assignments
                .iter()
                .map(|assignment| assignment.assignment_id.trim().to_string())
                .collect::<Vec<_>>();
            (
                Some(deployment.manifest_id.clone()),
                Some(deployment.project_id.clone()),
                Some(delivery_scope_name(&deployment.delivery_scope)),
                assignment_ids,
            )
        }
        None => (None, None, None, Vec::new()),
    };

    Ok(SentinelDeploymentInboxResult {
        mode: "sentinel_deployment_inbox",
        node_id: local_node_id.to_string(),
        status: inbox_status(assignment_ids.len()),
        slice_url: slice_url.to_string(),
        manifest_id,
        project_id,
        delivery_scope,
        assignment_count: assignment_ids.len(),
        assignment_ids,
        control_plane_fetch_performed: true,
        artifact_acquisition_performed: false,
        wasmtime_compiled: false,
        guest_execution_performed: false,
        report_published: false,
        governed_execution_performed: false,
        deployment_side_effects_performed: false,
    })
}

fn render_inbox_result(result: SentinelDeploymentInboxResult, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| {
                "{\"error\":\"failed to render Sentinel deployment inbox result\"}".to_string()
            })
        );
        return Ok(());
    }

    println!("I38 Sentinel deployment inbox discovery");
    println!("  node_id: {}", result.node_id);
    println!("  status: {}", result.status);
    println!("  slice_url: {}", result.slice_url);
    println!("  assignment_count: {}", result.assignment_count);
    println!("  assignment_ids: {:?}", result.assignment_ids);
    println!(
        "  artifact_acquisition_performed: {}",
        result.artifact_acquisition_performed
    );
    println!("  wasmtime_compiled: {}", result.wasmtime_compiled);
    println!(
        "  guest_execution_performed: {}",
        result.guest_execution_performed
    );
    println!("  report_published: {}", result.report_published);
    Ok(())
}

fn artifact_policy(
    allow_insecure_http: bool,
    max_artifact_bytes: Option<usize>,
    acquisition_timeout_ms: Option<u64>,
) -> ArtifactAcquisitionPolicy {
    let mut policy = ArtifactAcquisitionPolicy::default();
    policy.allow_insecure_http = allow_insecure_http;
    if let Some(max_artifact_bytes) = max_artifact_bytes {
        policy.max_artifact_bytes = max_artifact_bytes;
    }
    if let Some(acquisition_timeout_ms) = acquisition_timeout_ms {
        policy.request_timeout_ms = acquisition_timeout_ms;
    }
    policy
}

fn parse_scalar_args(values: &[String]) -> Result<Vec<WasmScalarValue>> {
    values
        .iter()
        .map(|value| parse_scalar_arg(value))
        .collect::<Result<Vec<_>>>()
}

fn is_control_plane_mode(deployment: &str) -> bool {
    let deployment = deployment.trim().to_ascii_lowercase();
    deployment.starts_with("http://") || deployment.starts_with("https://")
}

fn is_inbox_selector(assignment_selector: &str) -> bool {
    assignment_selector.trim() == CONTROL_PLANE_INBOX_SELECTOR
}

fn validate_bound_mode_cli(node_id_override: Option<&str>, assignment_selector: &str) -> Result<()> {
    if node_id_override.is_some() {
        return Err(anyhow!(
            "--node-id is forbidden in Sentinel-bound control-plane mode; identity comes only from agent.node_id"
        ));
    }
    if assignment_selector.trim() != CONTROL_PLANE_ASSIGNMENT_SELECTOR {
        return Err(anyhow!(
            "Sentinel-bound control-plane mode requires --assignment auto; the agent selects the single assignment delivered in its own node slice"
        ));
    }
    Ok(())
}

fn validate_inbox_mode_cli(
    node_id_override: Option<&str>,
    scalar_args: &[String],
    execute: bool,
    max_artifact_bytes: Option<usize>,
) -> Result<()> {
    if node_id_override.is_some() {
        return Err(anyhow!(
            "--node-id is forbidden in Sentinel deployment inbox mode; identity comes only from agent.node_id"
        ));
    }
    if execute {
        return Err(anyhow!(
            "Sentinel deployment inbox discovery is read-only; --execute is forbidden"
        ));
    }
    if !scalar_args.is_empty() {
        return Err(anyhow!(
            "Sentinel deployment inbox discovery does not accept WASM --arg values"
        ));
    }
    if max_artifact_bytes.is_some() {
        return Err(anyhow!(
            "Sentinel deployment inbox discovery does not acquire artifacts; --max-artifact-bytes is not applicable"
        ));
    }
    Ok(())
}

fn inbox_status(assignment_count: usize) -> &'static str {
    match assignment_count {
        0 => "no_work",
        1 => "one_assignment",
        _ => "multiple_assignments",
    }
}

fn delivery_scope_name(scope: &DatumNodeDeploymentScope) -> &'static str {
    match scope {
        DatumNodeDeploymentScope::Full => "full",
        DatumNodeDeploymentScope::NodeSlice => "node_slice",
        DatumNodeDeploymentScope::Empty => "empty",
    }
}

fn normalize_bound_node_id(node_id: &str) -> Result<String> {
    let node_id = node_id.trim();
    if node_id.is_empty() {
        return Err(anyhow!(
            "agent.node_id must not be empty in Sentinel-bound control-plane mode"
        ));
    }
    Ok(node_id.to_string())
}

fn parse_control_plane_base_url(value: &str, allow_insecure_http: bool) -> Result<Url> {
    let url = Url::parse(value.trim()).with_context(|| format!("parse dserver URL '{value}'"))?;
    match url.scheme() {
        "https" => {}
        "http" if allow_insecure_http => {}
        "http" => {
            return Err(anyhow!(
                "insecure HTTP dserver access is disabled by default and must be explicitly enabled"
            ));
        }
        scheme => {
            return Err(anyhow!(
                "dserver URL scheme '{scheme}' is not allowed; expected https or explicitly enabled http"
            ));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("dserver URL must not embed credentials"));
    }
    Ok(url)
}

fn control_plane_endpoint(base: &Url, segments: &[&str]) -> Result<Url> {
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("dserver URL cannot be used as a hierarchical base URL"))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn select_single_assignment_id(deployment: &DatumNodeScopedDeployment) -> Result<String> {
    if deployment.assignments.len() != 1 {
        return Err(anyhow!(
            "Sentinel-bound control-plane mode requires exactly one assignment in the node slice; received {}",
            deployment.assignments.len()
        ));
    }
    let assignment_id = deployment.assignments[0].assignment_id.trim();
    if assignment_id.is_empty() {
        return Err(anyhow!("node-slice assignment_id must not be empty"));
    }
    Ok(assignment_id.to_string())
}

fn resolve_legacy_node_id(config_path: &str, node_id_override: Option<&str>) -> Result<String> {
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

fn render_legacy_report(report: &LiveWasmExecutionReport, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(report)
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
    use super::*;
    use crate::wasm_runtime::{
        deployment::{
            DatumDeploymentArtifact, DatumDeploymentAssignment, DatumDeploymentExecutionBoundary,
            DatumDeploymentRuntimeLimits, DatumNodeDeploymentScope,
        },
        generic_runner::WasmScalarValue,
    };

    fn test_view(assignment_count: usize) -> DatumNodeScopedDeployment {
        let assignments = (0..assignment_count)
            .map(|index| DatumDeploymentAssignment {
                assignment_id: format!("assignment-{index}"),
                module_id: format!("module-{index}"),
                function_name: "add".to_string(),
                artifact: DatumDeploymentArtifact {
                    artifact_id: format!("artifact-{index}"),
                    artifact_uri: "https://example.invalid/module.wasm".to_string(),
                    sha256: "0".repeat(64),
                    content_id: format!("sha256:{}", "0".repeat(64)),
                },
                target_node: "fog-01".to_string(),
                limits: DatumDeploymentRuntimeLimits {
                    fuel: Some(1_000_000),
                    timeout_ms: Some(1_000),
                },
            })
            .collect();
        DatumNodeScopedDeployment {
            manifest_version: "0.1.0".to_string(),
            manifest_id: "manifest-1".to_string(),
            project_id: "project-1".to_string(),
            node_id: "fog-01".to_string(),
            delivery_scope: if assignment_count == 0 {
                DatumNodeDeploymentScope::Empty
            } else {
                DatumNodeDeploymentScope::NodeSlice
            },
            assignments,
            execution_boundary: DatumDeploymentExecutionBoundary {
                explicit_invocation_required: true,
                automatic_execution_allowed: false,
                governed_execution_authorized: false,
            },
        }
    }

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

    #[test]
    fn recognizes_control_plane_mode_only_for_http_urls() {
        assert!(is_control_plane_mode("https://dserver.example"));
        assert!(is_control_plane_mode("http://127.0.0.1:8080"));
        assert!(!is_control_plane_mode("deployment.json"));
        assert!(!is_control_plane_mode("C:\\tmp\\deployment.json"));
    }

    #[test]
    fn bound_mode_forbids_node_override() {
        let error = validate_bound_mode_cli(Some("other-node"), "auto")
            .expect_err("bound mode must reject node override");
        assert!(error.to_string().contains("--node-id is forbidden"));
    }

    #[test]
    fn bound_mode_requires_auto_assignment_selector() {
        let error = validate_bound_mode_cli(None, "fog-scale")
            .expect_err("bound mode must reject operator assignment selection");
        assert!(error.to_string().contains("--assignment auto"));
    }

    #[test]
    fn bound_identity_requires_non_empty_agent_node_id() {
        assert_eq!(normalize_bound_node_id(" fog-01 ").unwrap(), "fog-01");
        assert!(normalize_bound_node_id("  ").is_err());
    }

    #[test]
    fn insecure_control_plane_http_is_fail_closed_by_default() {
        let error = parse_control_plane_base_url("http://127.0.0.1:8080", false)
            .expect_err("HTTP must be blocked by default");
        assert!(error.to_string().contains("disabled by default"));
        assert!(parse_control_plane_base_url("http://127.0.0.1:8080", true).is_ok());
        assert!(parse_control_plane_base_url("https://example.invalid", false).is_ok());
    }

    #[test]
    fn control_plane_endpoint_encodes_bound_node_id() {
        let base = Url::parse("https://example.invalid/old/path?x=1").unwrap();
        let url = control_plane_endpoint(
            &base,
            &["api", "v1", "controlled-wasm", "deployment", "for-node", "fog node"],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.invalid/api/v1/controlled-wasm/deployment/for-node/fog%20node"
        );
    }

    #[test]
    fn bound_mode_selects_only_single_delivered_assignment() {
        assert_eq!(
            select_single_assignment_id(&test_view(1)).unwrap(),
            "assignment-0"
        );
        assert!(select_single_assignment_id(&test_view(0)).is_err());
        assert!(select_single_assignment_id(&test_view(2)).is_err());
    }

    #[test]
    fn inbox_selector_is_explicit_and_separate_from_auto_execution_selector() {
        assert!(is_inbox_selector("inbox"));
        assert!(is_inbox_selector(" inbox "));
        assert!(!is_inbox_selector("auto"));
        assert!(!is_inbox_selector("fog-scale"));
    }

    #[test]
    fn inbox_mode_forbids_execute() {
        let error = validate_inbox_mode_cli(None, &[], true, None)
            .expect_err("inbox discovery must reject --execute");
        assert!(error.to_string().contains("--execute is forbidden"));
    }

    #[test]
    fn inbox_mode_forbids_node_override() {
        let error = validate_inbox_mode_cli(Some("other-node"), &[], false, None)
            .expect_err("inbox discovery must bind identity to config");
        assert!(error.to_string().contains("--node-id is forbidden"));
    }

    #[test]
    fn inbox_mode_forbids_execution_arguments_and_artifact_knobs() {
        let args = vec!["i32:7".to_string()];
        let args_error = validate_inbox_mode_cli(None, &args, false, None)
            .expect_err("inbox discovery must reject WASM args");
        assert!(args_error.to_string().contains("does not accept WASM --arg"));

        let artifact_error = validate_inbox_mode_cli(None, &[], false, Some(1024))
            .expect_err("inbox discovery must reject artifact size policy");
        assert!(artifact_error
            .to_string()
            .contains("does not acquire artifacts"));
    }

    #[test]
    fn inbox_status_distinguishes_zero_one_and_multiple_assignments() {
        assert_eq!(inbox_status(0), "no_work");
        assert_eq!(inbox_status(1), "one_assignment");
        assert_eq!(inbox_status(2), "multiple_assignments");
        assert_eq!(inbox_status(9), "multiple_assignments");
    }

    #[test]
    fn inbox_result_is_read_only_and_lists_only_delivered_assignment_ids() {
        let view = test_view(2);
        let url = Url::parse(
            "https://example.invalid/api/v1/controlled-wasm/deployment/for-node/fog-01",
        )
        .unwrap();
        let result = build_inbox_result("fog-01", &url, Some(&view)).unwrap();
        assert_eq!(result.status, "multiple_assignments");
        assert_eq!(result.assignment_count, 2);
        assert_eq!(
            result.assignment_ids,
            vec!["assignment-0".to_string(), "assignment-1".to_string()]
        );
        assert!(!result.artifact_acquisition_performed);
        assert!(!result.wasmtime_compiled);
        assert!(!result.guest_execution_performed);
        assert!(!result.report_published);
        assert!(!result.governed_execution_performed);
        assert!(!result.deployment_side_effects_performed);
    }

    #[test]
    fn inbox_result_rejects_cross_node_assignment_leak() {
        let mut view = test_view(1);
        view.assignments[0].target_node = "cloud-01".to_string();
        let url = Url::parse(
            "https://example.invalid/api/v1/controlled-wasm/deployment/for-node/fog-01",
        )
        .unwrap();
        let error = build_inbox_result("fog-01", &url, Some(&view))
            .expect_err("cross-node assignment must fail discovery validation");
        assert!(error.to_string().contains("target node"));
    }
}
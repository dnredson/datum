use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasmtime::{Config, Engine, ExternType, Instance, Module, Store, Val, ValType};

use crate::wasm_runtime::{
    deployment::{
        prepare_node_scoped_deployment, validate_node_scoped_deployment_for_local_node,
        ArtifactAcquisitionEvidence, ArtifactAcquisitionPolicy, DatumDeploymentAssignment,
        DatumDeploymentRuntimeLimits, DatumNodeScopedDeployment, PreparedDeploymentArtifact,
    },
    generic_runner::WasmScalarValue,
    WasmRuntimeLimits,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveDeploymentContext {
    pub manifest_id: String,
    pub project_id: String,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveWasmInvocationRequest {
    pub invoked_by: String,
    pub explicit_execute: bool,
    pub args: Vec<WasmScalarValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveWasmExecutionReport {
    pub execution_id: String,
    pub observed_at_utc: String,
    pub manifest_id: String,
    pub project_id: String,
    pub node_id: String,
    pub assignment_id: String,
    pub module_id: String,
    pub function_name: String,
    pub artifact_content_id: String,
    pub artifact_uri: String,
    pub invoked_by: String,
    pub explicit_execute: bool,
    pub status: String,
    pub message: Option<String>,
    pub args: Vec<WasmScalarValue>,
    pub results: Vec<WasmScalarValue>,
    pub acquisition: ArtifactAcquisitionEvidence,
    pub integrity_verified: bool,
    pub wasmtime_compiled: bool,
    pub wasmtime_instantiation_attempted: bool,
    pub wasmtime_instantiated: bool,
    pub start_function_execution_possible: bool,
    pub function_invocation_attempted: bool,
    pub guest_execution_performed: bool,
    pub guest_execution_may_have_occurred: bool,
    pub wasm_guest_side_effects: bool,
    pub governed_execution_performed: bool,
    pub host_mutation_performed: bool,
    pub safe_to_auto_execute: bool,
    pub fuel_configured: Option<u64>,
    pub fuel_consumed: Option<u64>,
    pub fuel_exhausted: bool,
    pub timeout_ms: Option<u64>,
    pub timeout_enforced: bool,
}

pub fn prepare_single_assignment(
    view: &DatumNodeScopedDeployment,
    local_node_id: &str,
    assignment_id: &str,
    policy: &ArtifactAcquisitionPolicy,
) -> Result<(PreparedDeploymentArtifact, DatumDeploymentRuntimeLimits)> {
    validate_node_scoped_deployment_for_local_node(view, local_node_id)?;

    let assignment_id = assignment_id.trim();
    if assignment_id.is_empty() {
        return Err(anyhow!("assignment_id must not be empty"));
    }

    let assignment = view
        .assignments
        .iter()
        .find(|assignment| assignment.assignment_id.trim() == assignment_id)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "assignment '{}' is not present in node-scoped deployment for '{}'",
                assignment_id,
                local_node_id
            )
        })?;

    let limits = assignment.limits.clone();
    let selected_view = DatumNodeScopedDeployment {
        manifest_version: view.manifest_version.clone(),
        manifest_id: view.manifest_id.clone(),
        project_id: view.project_id.clone(),
        node_id: view.node_id.clone(),
        delivery_scope: view.delivery_scope.clone(),
        assignments: vec![assignment],
        execution_boundary: view.execution_boundary.clone(),
    };

    let mut prepared = prepare_node_scoped_deployment(&selected_view, local_node_id, policy)?;
    let prepared = prepared
        .pop()
        .ok_or_else(|| anyhow!("selected assignment did not produce a prepared artifact"))?;
    Ok((prepared, limits))
}

pub fn execute_prepared_assignment_live(
    prepared: &PreparedDeploymentArtifact,
    limits: &DatumDeploymentRuntimeLimits,
    context: &LiveDeploymentContext,
    request: &LiveWasmInvocationRequest,
) -> Result<LiveWasmExecutionReport> {
    validate_live_context(prepared, context, request)?;

    let runtime_limits = WasmRuntimeLimits {
        fuel: limits.fuel.or(WasmRuntimeLimits::default().fuel),
        timeout_ms: limits.timeout_ms.or(WasmRuntimeLimits::default().timeout_ms),
    };

    let mut report = base_report(prepared, context, request, &runtime_limits);
    if !request.explicit_execute {
        report.status = "blocked".to_string();
        report.message = Some(
            "live WASM deployment requires explicit operator execution confirmation".to_string(),
        );
        return Ok(report);
    }

    if !prepared.preflight.integrity_verified || !prepared.preflight.wasmtime_compiled {
        report.status = "blocked".to_string();
        report.message = Some(
            "live execution requires a C65 artifact with verified integrity and successful Wasmtime preflight"
                .to_string(),
        );
        return Ok(report);
    }

    if runtime_limits.fuel == Some(0) {
        report.status = "blocked".to_string();
        report.fuel_exhausted = true;
        report.message = Some("live WASM execution requires a positive fuel budget".to_string());
        return Ok(report);
    }

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build Wasmtime engine for live deployment")?;
    let module = match Module::new(&engine, prepared.artifact.bytes()) {
        Ok(module) => module,
        Err(error) => {
            report.status = "failed".to_string();
            report.message = Some(format!("compile verified live deployment artifact failed: {error}"));
            return Ok(report);
        }
    };

    if let Some(import) = module.imports().next() {
        report.status = "blocked".to_string();
        report.message = Some(format!(
            "live deployment artifact imports host item '{}::{}'; host imports remain forbidden",
            import.module(),
            import.name()
        ));
        return Ok(report);
    }

    let function_type = match module
        .exports()
        .find(|export| export.name() == prepared.function_name)
        .map(|export| export.ty())
    {
        Some(ExternType::Func(function_type)) => function_type,
        Some(_) => {
            report.status = "blocked".to_string();
            report.message = Some(format!(
                "assigned export '{}' is not a function",
                prepared.function_name
            ));
            return Ok(report);
        }
        None => {
            report.status = "failed".to_string();
            report.message = Some(format!(
                "verified artifact no longer exposes assigned function '{}'",
                prepared.function_name
            ));
            return Ok(report);
        }
    };

    let parameter_types: Vec<ValType> = function_type.params().collect();
    let result_types: Vec<ValType> = function_type.results().collect();
    if let Some(message) = validate_scalar_signature(&request.args, &parameter_types, &result_types) {
        report.status = "blocked".to_string();
        report.message = Some(message);
        return Ok(report);
    }

    let mut store = Store::new(&engine, ());
    if let Some(fuel) = runtime_limits.fuel {
        store
            .set_fuel(fuel)
            .context("configure live deployment fuel budget")?;
    }
    let fuel_before = store.get_fuel().ok();

    report.wasmtime_instantiation_attempted = true;
    report.start_function_execution_possible = true;
    report.guest_execution_may_have_occurred = true;
    let instance = match Instance::new(&mut store, &module, &[]) {
        Ok(instance) => {
            report.wasmtime_instantiated = true;
            instance
        }
        Err(error) => {
            let fuel_after = store.get_fuel().ok();
            report.fuel_consumed = fuel_consumed(fuel_before, fuel_after);
            report.fuel_exhausted = is_fuel_exhaustion(&error);
            report.status = "failed".to_string();
            report.message = Some(format!(
                "explicit Wasmtime instantiation failed; a module start function may have executed before failure: {error}"
            ));
            return Ok(report);
        }
    };

    let function = match instance.get_func(&mut store, &prepared.function_name) {
        Some(function) => function,
        None => {
            report.status = "failed".to_string();
            report.message = Some(format!(
                "instantiated artifact does not expose assigned function '{}'",
                prepared.function_name
            ));
            return Ok(report);
        }
    };

    let params: Vec<Val> = request.args.iter().map(to_wasmtime_val).collect();
    let mut results: Vec<Val> = result_types
        .iter()
        .map(default_value_for_type)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| anyhow!("supported result types must have scalar placeholders"))?;

    report.function_invocation_attempted = true;
    report.guest_execution_performed = true;
    if let Err(error) = function.call(&mut store, &params, &mut results) {
        let fuel_after = store.get_fuel().ok();
        report.fuel_consumed = fuel_consumed(fuel_before, fuel_after);
        report.fuel_exhausted = is_fuel_exhaustion(&error);
        report.status = "failed".to_string();
        report.message = Some(format!(
            "explicit live WASM function invocation failed: {error}"
        ));
        return Ok(report);
    }

    let fuel_after = store.get_fuel().ok();
    report.fuel_consumed = fuel_consumed(fuel_before, fuel_after);
    report.results = results
        .iter()
        .map(from_wasmtime_val)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| anyhow!("live WASM function returned an unsupported runtime value"))?;
    report.status = "executed".to_string();
    report.message = Some(format!(
        "explicitly instantiated verified artifact '{}' and invoked function '{}' on node '{}'",
        prepared.identity.content_id, prepared.function_name, context.node_id
    ));
    Ok(report)
}

fn validate_live_context(
    prepared: &PreparedDeploymentArtifact,
    context: &LiveDeploymentContext,
    request: &LiveWasmInvocationRequest,
) -> Result<()> {
    if context.manifest_id.trim().is_empty()
        || context.project_id.trim().is_empty()
        || context.node_id.trim().is_empty()
    {
        return Err(anyhow!(
            "manifest_id, project_id, and node_id are required for live execution evidence"
        ));
    }
    if request.invoked_by.trim().is_empty() {
        return Err(anyhow!("invoked_by must not be empty"));
    }
    if prepared.target_node.trim() != context.node_id.trim() {
        return Err(anyhow!(
            "prepared assignment '{}' targets '{}' but live context is node '{}'",
            prepared.assignment_id,
            prepared.target_node,
            context.node_id
        ));
    }
    Ok(())
}

fn validate_scalar_signature(
    args: &[WasmScalarValue],
    parameter_types: &[ValType],
    result_types: &[ValType],
) -> Option<String> {
    if args.len() != parameter_types.len() {
        return Some(format!(
            "assigned WASM function expects {} arguments but explicit invocation supplied {}",
            parameter_types.len(),
            args.len()
        ));
    }

    for (index, (argument, expected)) in args.iter().zip(parameter_types).enumerate() {
        if !is_supported_scalar_type(expected) {
            return Some(format!(
                "assigned WASM function parameter {index} uses unsupported type {}",
                wasm_type_name(expected)
            ));
        }
        if !scalar_matches_type(argument, expected) {
            return Some(format!(
                "assigned WASM function parameter {index} expects {} but explicit invocation supplied {}",
                wasm_type_name(expected),
                scalar_type_name(argument)
            ));
        }
    }

    for (index, result_type) in result_types.iter().enumerate() {
        if !is_supported_scalar_type(result_type) {
            return Some(format!(
                "assigned WASM function result {index} uses unsupported type {}",
                wasm_type_name(result_type)
            ));
        }
    }
    None
}

fn base_report(
    prepared: &PreparedDeploymentArtifact,
    context: &LiveDeploymentContext,
    request: &LiveWasmInvocationRequest,
    limits: &WasmRuntimeLimits,
) -> LiveWasmExecutionReport {
    LiveWasmExecutionReport {
        execution_id: Uuid::new_v4().to_string(),
        observed_at_utc: Utc::now().to_rfc3339(),
        manifest_id: context.manifest_id.clone(),
        project_id: context.project_id.clone(),
        node_id: context.node_id.clone(),
        assignment_id: prepared.assignment_id.clone(),
        module_id: prepared.module_id.clone(),
        function_name: prepared.function_name.clone(),
        artifact_content_id: prepared.identity.content_id.clone(),
        artifact_uri: prepared.artifact_uri.clone(),
        invoked_by: request.invoked_by.clone(),
        explicit_execute: request.explicit_execute,
        status: "prepared".to_string(),
        message: None,
        args: request.args.clone(),
        results: Vec::new(),
        acquisition: prepared.acquisition.clone(),
        integrity_verified: prepared.preflight.integrity_verified,
        wasmtime_compiled: prepared.preflight.wasmtime_compiled,
        wasmtime_instantiation_attempted: false,
        wasmtime_instantiated: false,
        start_function_execution_possible: false,
        function_invocation_attempted: false,
        guest_execution_performed: false,
        guest_execution_may_have_occurred: false,
        wasm_guest_side_effects: false,
        governed_execution_performed: false,
        host_mutation_performed: prepared.acquisition.host_mutation_performed,
        safe_to_auto_execute: false,
        fuel_configured: limits.fuel,
        fuel_consumed: None,
        fuel_exhausted: false,
        timeout_ms: limits.timeout_ms,
        timeout_enforced: false,
    }
}

fn is_supported_scalar_type(value_type: &ValType) -> bool {
    matches!(
        value_type,
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
    )
}

fn scalar_matches_type(value: &WasmScalarValue, value_type: &ValType) -> bool {
    matches!(
        (value, value_type),
        (WasmScalarValue::I32(_), ValType::I32)
            | (WasmScalarValue::I64(_), ValType::I64)
            | (WasmScalarValue::F32(_), ValType::F32)
            | (WasmScalarValue::F64(_), ValType::F64)
    )
}

fn scalar_type_name(value: &WasmScalarValue) -> &'static str {
    match value {
        WasmScalarValue::I32(_) => "i32",
        WasmScalarValue::I64(_) => "i64",
        WasmScalarValue::F32(_) => "f32",
        WasmScalarValue::F64(_) => "f64",
    }
}

fn wasm_type_name(value_type: &ValType) -> &'static str {
    match value_type {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "v128",
        ValType::Ref(_) => "reference",
    }
}

fn to_wasmtime_val(value: &WasmScalarValue) -> Val {
    match value {
        WasmScalarValue::I32(value) => Val::I32(*value),
        WasmScalarValue::I64(value) => Val::I64(*value),
        WasmScalarValue::F32(value) => Val::F32(value.to_bits()),
        WasmScalarValue::F64(value) => Val::F64(value.to_bits()),
    }
}

fn from_wasmtime_val(value: &Val) -> Option<WasmScalarValue> {
    match value {
        Val::I32(value) => Some(WasmScalarValue::I32(*value)),
        Val::I64(value) => Some(WasmScalarValue::I64(*value)),
        Val::F32(value) => Some(WasmScalarValue::F32(f32::from_bits(*value))),
        Val::F64(value) => Some(WasmScalarValue::F64(f64::from_bits(*value))),
        _ => None,
    }
}

fn default_value_for_type(value_type: &ValType) -> Option<Val> {
    match value_type {
        ValType::I32 => Some(Val::I32(0)),
        ValType::I64 => Some(Val::I64(0)),
        ValType::F32 => Some(Val::F32(0)),
        ValType::F64 => Some(Val::F64(0)),
        _ => None,
    }
}

fn fuel_consumed(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    match (before, after) {
        (Some(before), Some(after)) if before >= after => Some(before - after),
        _ => None,
    }
}

fn is_fuel_exhaustion(error: &anyhow::Error) -> bool {
    error.to_string().to_ascii_lowercase().contains("fuel")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::*;
    use crate::wasm_runtime::deployment::{
        DatumDeploymentArtifact, DatumDeploymentExecutionBoundary, DatumNodeDeploymentScope,
    };

    fn write_artifact(source: &str) -> (PathBuf, Vec<u8>) {
        let bytes = wat::parse_str(source).expect("test WAT should compile");
        let path = std::env::temp_dir().join(format!("datum-c66-{}.wasm", Uuid::new_v4()));
        fs::write(&path, &bytes).expect("test artifact should be written");
        (path, bytes)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn view(path: &PathBuf, bytes: &[u8], function_name: &str) -> DatumNodeScopedDeployment {
        let digest = sha256_hex(bytes);
        DatumNodeScopedDeployment {
            manifest_version: "0.1.0".to_string(),
            manifest_id: "manifest-c66".to_string(),
            project_id: "project-c66".to_string(),
            node_id: "fog-01".to_string(),
            delivery_scope: DatumNodeDeploymentScope::NodeSlice,
            assignments: vec![DatumDeploymentAssignment {
                assignment_id: "assignment-1".to_string(),
                module_id: "module-1".to_string(),
                function_name: function_name.to_string(),
                artifact: DatumDeploymentArtifact {
                    artifact_id: "artifact-1".to_string(),
                    artifact_uri: url::Url::from_file_path(path)
                        .expect("test artifact path should become file URL")
                        .to_string(),
                    sha256: digest.clone(),
                    content_id: format!("sha256:{digest}"),
                },
                target_node: "fog-01".to_string(),
                limits: DatumDeploymentRuntimeLimits {
                    fuel: Some(1_000_000),
                    timeout_ms: Some(1_000),
                },
            }],
            execution_boundary: DatumDeploymentExecutionBoundary {
                explicit_invocation_required: true,
                automatic_execution_allowed: false,
                governed_execution_authorized: false,
            },
        }
    }

    fn context() -> LiveDeploymentContext {
        LiveDeploymentContext {
            manifest_id: "manifest-c66".to_string(),
            project_id: "project-c66".to_string(),
            node_id: "fog-01".to_string(),
        }
    }

    #[test]
    fn explicit_verified_add_executes_live() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
        );
        let deployment = view(&path, &bytes, "add");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: vec![WasmScalarValue::I32(2), WasmScalarValue::I32(3)],
            },
        )
        .expect("live invocation should return report");

        assert_eq!(report.status, "executed");
        assert_eq!(report.results, vec![WasmScalarValue::I32(5)]);
        assert!(report.integrity_verified);
        assert!(report.wasmtime_instantiated);
        assert!(report.function_invocation_attempted);
        assert!(report.guest_execution_performed);
        assert!(!report.wasm_guest_side_effects);
        assert!(!report.governed_execution_performed);
        assert!(!report.safe_to_auto_execute);
        assert!(!report.timeout_enforced);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_explicit_execute_blocks_before_instantiation() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
        );
        let deployment = view(&path, &bytes, "add");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: false,
                args: vec![WasmScalarValue::I32(2), WasmScalarValue::I32(3)],
            },
        )
        .expect("blocked invocation should return report");

        assert_eq!(report.status, "blocked");
        assert!(!report.wasmtime_instantiation_attempted);
        assert!(!report.guest_execution_performed);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn signature_mismatch_blocks_before_instantiation() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
        );
        let deployment = view(&path, &bytes, "add");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: vec![WasmScalarValue::I64(2), WasmScalarValue::I32(3)],
            },
        )
        .expect("signature rejection should return report");

        assert_eq!(report.status, "blocked");
        assert!(!report.wasmtime_instantiation_attempted);
        assert!(!report.guest_execution_performed);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_i64_invocation_executes() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "mul64") (param i64 i64) (result i64) local.get 0 local.get 1 i64.mul))"#,
        );
        let deployment = view(&path, &bytes, "mul64");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: vec![WasmScalarValue::I64(6), WasmScalarValue::I64(7)],
            },
        )
        .expect("i64 invocation should return report");

        assert_eq!(report.status, "executed");
        assert_eq!(report.results, vec![WasmScalarValue::I64(42)]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn trapping_function_records_guest_execution_and_failure() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "boom") unreachable))"#,
        );
        let deployment = view(&path, &bytes, "boom");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: Vec::new(),
            },
        )
        .expect("trap should return failed report");

        assert_eq!(report.status, "failed");
        assert!(report.wasmtime_instantiated);
        assert!(report.function_invocation_attempted);
        assert!(report.guest_execution_performed);
        assert!(!report.governed_execution_performed);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn start_trap_is_reported_as_possible_instantiation_guest_execution() {
        let (path, bytes) = write_artifact(
            r#"(module (func $start unreachable) (start $start) (func (export "noop")))"#,
        );
        let deployment = view(&path, &bytes, "noop");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let report = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &context(),
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: Vec::new(),
            },
        )
        .expect("start trap should return failed report");

        assert_eq!(report.status, "failed");
        assert!(report.wasmtime_instantiation_attempted);
        assert!(!report.wasmtime_instantiated);
        assert!(report.start_function_execution_possible);
        assert!(report.guest_execution_may_have_occurred);
        assert!(!report.function_invocation_attempted);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn selection_rejects_assignment_not_in_node_view_before_acquisition() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "noop")))"#,
        );
        let deployment = view(&path, &bytes, "noop");
        let error = prepare_single_assignment(
            &deployment,
            "fog-01",
            "missing-assignment",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("unknown assignment must be rejected");

        assert!(error.to_string().contains("not present"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn live_context_cannot_change_target_node() {
        let (path, bytes) = write_artifact(
            r#"(module (func (export "noop")))"#,
        );
        let deployment = view(&path, &bytes, "noop");
        let (prepared, limits) = prepare_single_assignment(
            &deployment,
            "fog-01",
            "assignment-1",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("assignment should prepare");
        let mut wrong_context = context();
        wrong_context.node_id = "fog-02".to_string();
        let error = execute_prepared_assignment_live(
            &prepared,
            &limits,
            &wrong_context,
            &LiveWasmInvocationRequest {
                invoked_by: "operator-test".to_string(),
                explicit_execute: true,
                args: Vec::new(),
            },
        )
        .expect_err("target node mismatch must fail before execution");

        assert!(error.to_string().contains("targets 'fog-01'"));
        let _ = fs::remove_file(path);
    }
}

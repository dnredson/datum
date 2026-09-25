//! DForward `any_element` dependency readiness API (Phase 114D2,
//! ADR-0010/ADR-0023/ADR-0024).
//!
//! Lock order: `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
//! DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION` -- the SAME
//! order Phase 114D0A/D1 already established. This handler never acquires
//! the D-Call lock, and never HTTP-calls DServer's own
//! `/api/v1/dmonitor/readiness/...` endpoint -- it calls the shared
//! `dmonitor_readiness`/`dforward_dependency_readiness` core functions
//! directly against one in-process state snapshot (section 29).
//!
//! Like the Phase 114D1 readiness endpoint, this route takes NO request
//! body and NO query parameters: the provider is resolved from CURRENT
//! accepted DForward topology, never a caller-supplied DForward object or
//! provider selection, and the readiness policy/evaluation time are
//! entirely server-owned.

use axum::{extract::Path, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::ddeploy::active_placement_authority;
use crate::core::dforward_dependency_readiness::{evaluate_dforward_dependency_readiness, DForwardDependencyReadinessV1};
use crate::core::dmonitor_readiness::default_readiness_policy;
use crate::core::dmonitor_state::load_state as load_dmonitor_state;
use crate::core::diot_runtime_binding::load_state as load_diot_runtime_binding_state;
use crate::core::dnode_registry::load_state as load_dnode_registry_state;
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

fn db_path() -> std::path::PathBuf {
    default_control_state_db_path()
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message.into()})))
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dforward dependency readiness transaction lock poisoned")
}

/// `GET /api/v1/dforward/dependency-readiness/:project_id/:application_id/:node_id`.
/// Read-only: performs no mutation, no remediation, and no placement/
/// provider selection decision.
pub async fn get_dforward_dependency_readiness(
    Path((project_id, application_id, node_id)): Path<(String, String, String)>,
) -> Result<JsonResponse<DForwardDependencyReadinessV1>, ApiError> {
    let policy = default_readiness_policy();

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction().lock().map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction().lock().map_err(lock_error)?;
    let _diot_runtime_binding_transaction = crate::api::diot_runtime_binding::diot_runtime_binding_transaction().lock().map_err(lock_error)?;
    let _dmonitor_transaction = crate::api::dmonitor::dmonitor_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state = load_dnode_registry_state(&db_path).map_err(internal_error)?;
    let diot_runtime_binding_registry = load_diot_runtime_binding_state(&db_path).map_err(internal_error)?;
    let authority = active_placement_authority(&ddeploy_state, &project_id, &application_id).map_err(internal_error)?;
    let dmonitor_state = load_dmonitor_state(&db_path).map_err(internal_error)?;

    // Server-generated; there is no request body/query parameter this
    // handler even parses that could override it.
    let evaluated_at_utc = Utc::now().to_rfc3339();

    let result = evaluate_dforward_dependency_readiness(
        &project_id,
        &application_id,
        &project_id,
        &node_id,
        authority.as_ref(),
        &dmonitor_state,
        &diot_runtime_binding_registry,
        &ddeploy_state,
        &dnode_registry_state,
        &policy,
        &evaluated_at_utc,
    );

    Ok(JsonResponse(result))
}

#[cfg(test)]
mod tests {
    fn production_source() -> String {
        let source = include_str!("dforward_dependency_readiness.rs");
        let production_source: String = source.lines().take_while(|line| line.trim() != "#[cfg(test)]").collect::<Vec<_>>().join("\n");
        assert!(production_source.len() < source.len());
        production_source
    }

    #[test]
    fn get_dependency_readiness_holds_the_established_lock_order() {
        let source = production_source();
        let start = source.find("pub async fn get_dforward_dependency_readiness(").unwrap();
        let body = &source[start..];
        let ddeploy_pos = body.find("ddeploy::ddeploy_transaction()").expect("must acquire DDEPLOY_TRANSACTION");
        let dnode_pos = body.find("dnode_registry::dnode_transaction()").expect("must acquire DNODE_TRANSACTION");
        let diot_runtime_binding_pos = body.find("diot_runtime_binding::diot_runtime_binding_transaction()").expect("must acquire DIOT_RUNTIME_BINDING_TRANSACTION");
        let dmonitor_pos = body.find("dmonitor::dmonitor_transaction()").expect("must acquire DMONITOR_TRANSACTION");
        assert!(
            ddeploy_pos < dnode_pos && dnode_pos < diot_runtime_binding_pos && diot_runtime_binding_pos < dmonitor_pos,
            "lock order must be DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION"
        );
        assert!(!body.contains("dcall_transaction()"), "dependency readiness must not acquire the DCall lock");
    }

    #[test]
    fn handler_takes_no_request_body_and_no_query_parameters() {
        let source = production_source();
        let start = source.find("pub async fn get_dforward_dependency_readiness(").unwrap();
        let signature_end = source[start..].find(") -> Result").unwrap() + start;
        let signature = &source[start..signature_end];
        assert!(!signature.contains("Json("), "dependency readiness must accept no request body: {signature}");
        assert!(!signature.contains("Query("), "dependency readiness must accept no query parameters: {signature}");
    }

    #[test]
    fn handler_never_calls_dmonitor_readiness_over_http() {
        let source = production_source();
        assert!(!source.contains("reqwest"), "must never make an internal HTTP call");
        assert!(!source.contains(".get(format!(") && !source.contains(".post(format!("), "must never issue an HTTP request to itself or any other DServer route");
    }

    #[test]
    fn handler_never_accepts_a_caller_supplied_dforward_or_provider() {
        let source = production_source();
        let start = source.find("pub async fn get_dforward_dependency_readiness(").unwrap();
        let signature_end = source[start..].find(") -> Result").unwrap() + start;
        // Skip past the function name itself (which legitimately contains
        // "dforward") -- only the PARAMETER LIST must never accept a
        // caller-supplied DForward object or provider selection.
        let params_start = start + source[start..].find('(').unwrap() + 1;
        let params = &source[params_start..signature_end];
        for forbidden in ["CanonicalDForwardV1", "provider", "DForward"] {
            assert!(!params.contains(forbidden), "handler parameter list must never accept a caller-supplied DForward/provider: found '{forbidden}' in {params}");
        }
    }

    #[test]
    fn evaluated_at_is_server_generated_from_utc_now() {
        let source = production_source();
        assert!(source.contains("Utc::now()"));
    }

    #[test]
    fn handler_is_read_only_never_persists_state() {
        let source = production_source();
        let start = source.find("pub async fn get_dforward_dependency_readiness(").unwrap();
        let body = &source[start..];
        for mutating in ["save_state", "persist", "accept_v2_proposal", "accept_proposal", "register_runtime_binding", "ingest_observation"] {
            assert!(!body.contains(mutating), "dependency readiness handler must be read-only, found call to '{mutating}'");
        }
    }
}

//! Server-owned DMonitor-backed readiness governance API (Phase 114D1,
//! ADR-0024).
//!
//! Lock order: `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
//! DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION` -- the SAME
//! order Phase 114D0A's convergence endpoint already established and
//! `api::diot_runtime_binding`'s own module doc reserves for exactly this
//! combination. This handler never acquires the D-Call lock.
//!
//! Unlike the diagnostic convergence endpoint (`api::dmonitor_convergence`),
//! this route takes NO request body and NO query parameters: readiness
//! policy and `evaluated_at_utc` are entirely server-owned (section 5/26).
//! A caller cannot influence the freshness window, the accepted-health set,
//! or the evaluation timestamp.
//!
//! This handler calls `core::dmonitor_readiness::evaluate_application_readiness`
//! directly against one in-process state snapshot -- it never makes an
//! internal HTTP call to the convergence endpoint (section 27).

use axum::{extract::Path, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::ddeploy::active_placement_authority;
use crate::core::dmonitor_readiness::{default_readiness_policy, evaluate_application_readiness, DMonitorApplicationReadinessV1};
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
    internal_error("dmonitor readiness transaction lock poisoned")
}

/// `GET /api/v1/dmonitor/readiness/:project_id/:application_id`. Read-only:
/// performs no mutation, no remediation, and no placement decision.
pub async fn get_dmonitor_readiness(
    Path((project_id, application_id)): Path<(String, String)>,
) -> Result<JsonResponse<DMonitorApplicationReadinessV1>, ApiError> {
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

    let result = evaluate_application_readiness(
        &project_id,
        &application_id,
        &project_id,
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
        let source = include_str!("dmonitor_readiness.rs");
        let production_source: String = source.lines().take_while(|line| line.trim() != "#[cfg(test)]").collect::<Vec<_>>().join("\n");
        assert!(production_source.len() < source.len());
        production_source
    }

    #[test]
    fn get_readiness_holds_the_established_lock_order() {
        let source = production_source();
        let start = source.find("pub async fn get_dmonitor_readiness(").unwrap();
        let body = &source[start..];
        let ddeploy_pos = body.find("ddeploy::ddeploy_transaction()").expect("must acquire DDEPLOY_TRANSACTION");
        let dnode_pos = body.find("dnode_registry::dnode_transaction()").expect("must acquire DNODE_TRANSACTION");
        let diot_runtime_binding_pos = body.find("diot_runtime_binding::diot_runtime_binding_transaction()").expect("must acquire DIOT_RUNTIME_BINDING_TRANSACTION");
        let dmonitor_pos = body.find("dmonitor::dmonitor_transaction()").expect("must acquire DMONITOR_TRANSACTION");
        assert!(
            ddeploy_pos < dnode_pos && dnode_pos < diot_runtime_binding_pos && diot_runtime_binding_pos < dmonitor_pos,
            "lock order must be DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION"
        );
        assert!(!body.contains("dcall_transaction()"), "readiness evaluation must not acquire the DCall lock");
    }

    #[test]
    fn handler_takes_no_request_body_and_no_query_parameters() {
        let source = production_source();
        let start = source.find("pub async fn get_dmonitor_readiness(").unwrap();
        let signature_end = source[start..].find(") -> Result").unwrap() + start;
        let signature = &source[start..signature_end];
        assert!(!signature.contains("Json("), "readiness must accept no request body: {signature}");
        assert!(!signature.contains("Query("), "readiness must accept no query parameters: {signature}");
    }

    #[test]
    fn handler_never_calls_the_diagnostic_convergence_http_route() {
        let source = production_source();
        assert!(!source.contains("/api/v1/dmonitor/convergence"), "readiness must call the shared core function directly, never HTTP-call the diagnostic convergence endpoint");
        assert!(!source.contains("reqwest"), "readiness must never make an internal HTTP call");
    }

    #[test]
    fn evaluated_at_is_server_generated_from_utc_now() {
        let source = production_source();
        assert!(source.contains("Utc::now()"));
    }

    #[test]
    fn handler_is_read_only_never_persists_state() {
        let source = production_source();
        let start = source.find("pub async fn get_dmonitor_readiness(").unwrap();
        let body = &source[start..];
        for mutating in ["save_state", "persist", "accept_v2_proposal", "accept_proposal", "register_runtime_binding", "ingest_observation"] {
            assert!(!body.contains(mutating), "readiness handler must be read-only, found call to '{mutating}'");
        }
        // Every state input must come through a `load_state` call.
        assert!(body.contains("load_state") || body.contains("load_dmonitor_state") || body.contains("load_dnode_registry_state") || body.contains("load_diot_runtime_binding_state"));
    }
}

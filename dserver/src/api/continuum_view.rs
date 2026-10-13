//! Phase 135 -- Continuum View read-only HTTP surface.
//!
//! Lock order: `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
//! DMONITOR_TRANSACTION` -- the same RELATIVE sequence
//! `api::dmonitor_readiness`/`api::dmonitor_convergence` already
//! established (`DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
//! DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION`), with
//! `DIOT_RUNTIME_BINDING_TRANSACTION` simply never acquired here -- this
//! view reads no D-IoT runtime binding state at all, and skipping a lock
//! in the middle of an established chain is safe precisely because the
//! locks actually taken are still taken in the SAME relative order (never
//! reversed), so no new deadlock ordering is introduced.
//!
//! Read-only: performs no mutation, no remediation, no placement decision,
//! and introduces no new persistence. `core::continuum_view::build_continuum_view`
//! itself takes only `&`-references to already-loaded state -- there is no
//! `&mut` parameter anywhere in its signature, so it is structurally
//! incapable of mutating any of the three states it composes.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    Json as JsonResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::continuum_view::{
    attach_resource_observations, build_continuum_view, ContinuumViewV1,
};
use crate::core::dmonitor_readiness::default_readiness_policy;
use crate::core::dmonitor_state::load_state as load_dmonitor_state;
use crate::core::dnode_registry::load_state as load_dnode_registry_state;
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

/// Mirrors every other project-identity limit already established in this
/// crate (Phase 134's editor: "project/identity 256 bytes").
const MAX_PROJECT_ID_BYTES: usize = 256;

fn db_path() -> std::path::PathBuf {
    default_control_state_db_path()
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("continuum view transaction lock poisoned")
}

fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn validate_project_id(project_id: &str) -> Result<&str, ApiError> {
    let trimmed = project_id.trim();
    if trimmed.is_empty() {
        return Err(bad_request("project_id must be non-empty"));
    }
    if trimmed.len() > MAX_PROJECT_ID_BYTES {
        return Err(bad_request(format!(
            "project_id must not exceed {MAX_PROJECT_ID_BYTES} bytes"
        )));
    }
    Ok(trimmed)
}

#[derive(Debug, Deserialize)]
pub struct ContinuumQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

/// `GET /api/v1/operations/continuum[?project_id=...]` -- the GLOBAL
/// Continuum View. The physical node set is always the full, global
/// registry; `project_id`, if given, only adds a [`ProjectOverlayV1`]
/// (`core::continuum_view::ProjectOverlayV1`) onto the SAME node list --
/// it never filters or re-scopes `nodes` itself.
pub async fn get_continuum_view(
    Query(query): Query<ContinuumQuery>,
) -> Result<JsonResponse<ContinuumViewV1>, ApiError> {
    let project_id = query
        .project_id
        .as_deref()
        .map(validate_project_id)
        .transpose()?;
    build_response(project_id).await
}

/// `GET /api/v1/operations/projects/:project_id/continuum` -- identical
/// physical data to the global route, with `project_id`'s overlay always
/// populated. Mirrors the existing `/api/v1/operations/projects/:project/dgraph-editor`
/// path convention (`api::dgraph_editor`).
pub async fn get_project_continuum_view(
    Path(project_id): Path<String>,
) -> Result<JsonResponse<ContinuumViewV1>, ApiError> {
    let project_id = validate_project_id(&project_id)?;
    build_response(Some(project_id)).await
}

async fn build_response(project_id: Option<&str>) -> Result<JsonResponse<ContinuumViewV1>, ApiError> {
    // Resource Registry state uses an independent lock/domain. Clone all
    // observation records BEFORE the established DDeploy/DNode/DMonitor lock
    // chain so this additive read cannot introduce a lock-order cycle.
    // Project scope is applied only when the observation is attached; the
    // D-Node inventory itself always remains global.
    let resource_records =
        crate::api::resource_registry::resource_registry_all_snapshot().map_err(internal_error)?;

    // Reuses DMonitor's own server-owned default freshness policy
    // (`core::dmonitor_readiness::default_readiness_policy`) -- never a
    // caller-suppliable or independently-invented one (Phase 135 brief,
    // section 12: "do not duplicate policy").
    let policy = default_readiness_policy().freshness_policy;

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dmonitor_transaction = crate::api::dmonitor::dmonitor_transaction()
        .lock()
        .map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state = load_dnode_registry_state(&db_path).map_err(internal_error)?;
    let dmonitor_state = load_dmonitor_state(&db_path).map_err(internal_error)?;

    // Server-generated; never accepted from the request (section 6/12 --
    // mirrors `get_dmonitor_readiness`'s identical rule).
    let now = Utc::now();

    let mut view = build_continuum_view(
        &dnode_registry_state,
        &ddeploy_state,
        &dmonitor_state,
        &policy,
        project_id,
        now,
    );
    attach_resource_observations(&mut view, &resource_records, project_id, now);
    Ok(JsonResponse(view))
}

#[cfg(test)]
mod tests {
    fn production_source() -> String {
        let source = include_str!("continuum_view.rs");
        let production_source: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(production_source.len() < source.len());
        production_source
    }

    #[test]
    fn build_response_holds_the_established_relative_lock_order() {
        let source = production_source();
        let start = source.find("async fn build_response(").unwrap();
        let body = &source[start..];
        let ddeploy_pos = body
            .find("ddeploy::ddeploy_transaction()")
            .expect("must acquire DDEPLOY_TRANSACTION");
        let dnode_pos = body
            .find("dnode_registry::dnode_transaction()")
            .expect("must acquire DNODE_TRANSACTION");
        let dmonitor_pos = body
            .find("dmonitor::dmonitor_transaction()")
            .expect("must acquire DMONITOR_TRANSACTION");
        assert!(
            ddeploy_pos < dnode_pos && dnode_pos < dmonitor_pos,
            "lock order must be DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DMONITOR_TRANSACTION"
        );
        assert!(
            !body.contains("diot_runtime_binding_transaction()"),
            "this view reads no D-IoT runtime binding state and must not acquire that lock"
        );
        assert!(!body.contains("dcall_transaction()"), "continuum view must not acquire the DCall lock");
    }

    #[test]
    fn handlers_take_no_request_body() {
        let source = production_source();
        for handler in ["get_continuum_view", "get_project_continuum_view"] {
            let start = source.find(&format!("pub async fn {handler}(")).unwrap();
            let signature_end = source[start..].find(") -> Result").unwrap() + start;
            let signature = &source[start..signature_end];
            assert!(!signature.contains("Json("), "{handler} must accept no request body: {signature}");
        }
    }

    #[test]
    fn evaluated_time_is_server_generated_from_utc_now() {
        let source = production_source();
        assert!(source.contains("Utc::now()"));
    }

    #[test]
    fn handler_is_read_only_never_persists_or_mutates_state() {
        let source = production_source();
        for mutating in [
            "persist_state",
            "persist_domain_json",
            "accept_v2_proposal",
            "accept_proposal",
            "create_proposal",
            "declare_dnode",
            "remove_dnode",
            "register_runtime_binding",
            "ingest_observation",
        ] {
            assert!(
                !source.contains(mutating),
                "continuum view must be read-only, found reference to '{mutating}'"
            );
        }
        assert!(source.contains("load_state") || source.contains("load_dmonitor_state") || source.contains("load_dnode_registry_state"));
    }

    #[test]
    fn resource_observation_is_additive_never_changes_physical_node_authority() {
        // The same D-Node Registry / DDeploy / DMonitor states always feed
        // build_continuum_view. Resource Registry telemetry is attached
        // AFTER the global node set is built; project scope may restrict its
        // observation source, but must never select a different D-Node source
        // or filter nodes.
        let source = production_source();
        let start = source.find("async fn build_response(").unwrap();
        let body = &source[start..];
        assert!(body.contains("build_continuum_view("));
        assert!(body.contains("attach_resource_observations("));
        assert_eq!(
            body.matches("load_dnode_registry_state(&db_path)").count(),
            1,
            "there must remain exactly one global D-Node Registry load"
        );
        let build_pos = body.find("let mut view = build_continuum_view(").unwrap();
        let attach_pos = body.find("attach_resource_observations(").unwrap();
        assert!(
            build_pos < attach_pos,
            "resource telemetry must be an additive observation joined after the physical Continuum is built"
        );
    }

    #[test]
    fn resource_registry_all_snapshot_occurs_before_the_established_transaction_lock_chain() {
        let source = production_source();
        let start = source.find("async fn build_response(").unwrap();
        let body = &source[start..];
        let resource_pos = body.find("resource_registry_all_snapshot").unwrap();
        let ddeploy_lock_pos = body.find("ddeploy::ddeploy_transaction()").unwrap();
        assert!(
            resource_pos < ddeploy_lock_pos,
            "Resource Registry observation must be cloned before acquiring the established Continuum lock chain"
        );
    }

    #[test]
    fn default_freshness_policy_is_reused_not_reinvented() {
        let source = production_source();
        assert!(source.contains("default_readiness_policy()"));
        assert!(
            !source.contains("DMonitorFreshnessPolicyV1 {"),
            "must reuse the server-owned default policy, never construct a new one here"
        );
    }
}

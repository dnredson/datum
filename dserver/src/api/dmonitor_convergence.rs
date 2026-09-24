//! DMonitor derived desired-vs-observed convergence diagnostic API (Phase
//! 114C, ADR-0024).
//!
//! Lock order: `DDEPLOY_TRANSACTION -> DMONITOR_TRANSACTION`, extending the
//! established chain (`api::diot_runtime_binding`'s own module doc states
//! the authoritative order this project already follows). This handler
//! never acquires the D-Node/D-Call/D-IoT-runtime-binding locks: the
//! comparison only needs the active D-Deploy placement authority and
//! persisted DMonitor evidence.
//!
//! This is a READ-ONLY diagnostic endpoint: it performs no mutation, no
//! remediation, and no placement decision. `POST` is used only because the
//! request body carries an explicit, per-request DIAGNOSTIC freshness
//! policy (never a server-owned governance policy -- 114D will define that
//! consumption path) -- the operation itself has no side effects.

use std::sync::{Mutex, OnceLock};

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::ddeploy::active_placement_authority;
use crate::core::dmonitor_freshness::{validate_freshness_policy, DMonitorFreshnessPolicyV1};
use crate::core::dmonitor_interpretation::{
    evaluate_application_dserv_convergence, DMonitorApplicationConvergenceV1,
};
use crate::core::dmonitor_state::load_state as load_dmonitor_state;
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DMONITOR_CONVERGENCE_EVAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// This evaluation is read-only against BOTH domains it reads, but still
/// takes a private lock of its own (rather than none) so a concurrent
/// D-Deploy acceptance and DMonitor ingestion cannot interleave with this
/// handler's two separate `load_state` calls in a way that mixes a
/// pre-transition authority read with a post-transition evidence read (or
/// vice versa) -- see [`db_path`]'s callers for the exact two reads this
/// protects.
fn dmonitor_convergence_eval_lock() -> &'static Mutex<()> {
    DMONITOR_CONVERGENCE_EVAL_LOCK.get_or_init(|| Mutex::new(()))
}

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
    internal_error("dmonitor convergence transaction lock poisoned")
}

fn bad_request(code: &'static str, message: impl Into<String>) -> ApiError {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        JsonResponse(json!({"error": code, "message": message.into()})),
    )
}

/// Strict, explicit DIAGNOSTIC freshness-policy input (section 8/27). The
/// server generates `evaluated_at_utc` itself; a caller cannot supply or
/// override it (this type has no such field to populate).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluateConvergenceRequest {
    pub max_observed_age_seconds: u64,
    pub max_received_age_seconds: u64,
    pub max_future_clock_skew_seconds: u64,
}

/// `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate`.
///
/// Project isolation (section 29): `project_id` is the path's authoritative
/// request scope. `active_placement_authority` is queried for EXACTLY this
/// `(project_id, application_id)` pair (an application belonging to a
/// different project cannot be named here), and
/// `evaluate_application_dserv_convergence` additionally filters every
/// DMonitor record it considers by `server_project_scope == project_id`
/// (Phase 114C's own second, independent guard -- see that function's doc
/// comment).
pub async fn post_evaluate_dmonitor_convergence(
    axum::extract::Path((project_id, application_id)): axum::extract::Path<(String, String)>,
    Json(request): Json<EvaluateConvergenceRequest>,
) -> Result<JsonResponse<DMonitorApplicationConvergenceV1>, ApiError> {
    let policy = DMonitorFreshnessPolicyV1 {
        max_observed_age_seconds: request.max_observed_age_seconds,
        max_received_age_seconds: request.max_received_age_seconds,
        max_future_clock_skew_seconds: request.max_future_clock_skew_seconds,
    };
    validate_freshness_policy(&policy).map_err(|error| bad_request("dmonitor_convergence_invalid_policy", error))?;

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dmonitor_transaction = crate::api::dmonitor::dmonitor_transaction()
        .lock()
        .map_err(lock_error)?;
    let _eval_lock = dmonitor_convergence_eval_lock().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let authority = active_placement_authority(&ddeploy_state, &project_id, &application_id)
        .map_err(internal_error)?;
    let dmonitor_state = load_dmonitor_state(&db_path).map_err(internal_error)?;

    // Server-generated; never accepted from the request body (section 27).
    let evaluated_at_utc = Utc::now().to_rfc3339();

    let result = evaluate_application_dserv_convergence(
        &project_id,
        &application_id,
        &project_id,
        authority.as_ref(),
        &dmonitor_state,
        &policy,
        &evaluated_at_utc,
    );

    Ok(JsonResponse(result))
}

#[cfg(test)]
mod tests {
    fn production_source() -> String {
        let source = include_str!("dmonitor_convergence.rs");
        let production_source: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(production_source.len() < source.len());
        production_source
    }

    #[test]
    fn post_evaluate_holds_the_established_lock_order() {
        let source = production_source();
        let start = source
            .find("pub async fn post_evaluate_dmonitor_convergence(")
            .unwrap();
        let body = &source[start..];
        let ddeploy_pos = body
            .find("ddeploy::ddeploy_transaction()")
            .expect("must acquire DDEPLOY_TRANSACTION");
        let dmonitor_pos = body
            .find("dmonitor::dmonitor_transaction()")
            .expect("must acquire DMONITOR_TRANSACTION");
        assert!(
            ddeploy_pos < dmonitor_pos,
            "DDEPLOY_TRANSACTION must be acquired before DMONITOR_TRANSACTION"
        );
        assert!(
            !body.contains("dnode_registry::dnode_transaction()")
                && !body.contains("dcall_transaction()")
                && !body.contains("diot_runtime_binding_transaction()"),
            "convergence evaluation must not acquire DNode/DCall/DIoT locks"
        );
    }

    #[test]
    fn request_type_rejects_unknown_fields_and_server_owned_evaluated_at() {
        let error = serde_json::from_str::<super::EvaluateConvergenceRequest>(
            r#"{"max_observed_age_seconds":60,"max_received_age_seconds":60,"max_future_clock_skew_seconds":10,"evaluated_at_utc":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn policy_validation_is_wired_before_any_lock_is_acquired() {
        let source = production_source();
        let start = source
            .find("pub async fn post_evaluate_dmonitor_convergence(")
            .unwrap();
        let body = &source[start..];
        let validate_pos = body
            .find("validate_freshness_policy")
            .expect("must call validate_freshness_policy");
        let lock_pos = body
            .find("ddeploy_transaction()")
            .expect("must acquire a lock");
        assert!(validate_pos < lock_pos, "policy must be validated before any lock is taken");
    }
}

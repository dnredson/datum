//! DIoT runtime-binding registry HTTP surface (Phase 113B1, ADR-0023).
//! Lock order: `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
//! DIOT_RUNTIME_BINDING_TRANSACTION`, extending the established
//! `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DCODE_TRANSACTION ->
//! DCALL_TRANSACTION` chain (`api::dcall`'s own module doc is the
//! authoritative statement of that order) with a NEW innermost lock private
//! to this domain -- registration never touches D-Code or D-Call state, so
//! it does not acquire either of those locks.
//!
//! `put_diot_runtime_binding` reads the currently active DMap/2 placement
//! authority (requires `DDEPLOY_TRANSACTION`) and the structural D-Node
//! registry (requires `DNODE_TRANSACTION`) to derive every identity field on
//! the binding except the caller-supplied `broker_uri`; no handler here ever
//! opens an MQTT connection or performs any network I/O against a broker.

use std::sync::{Mutex, OnceLock};

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::diot_runtime_binding::{
    load_state as load_binding_state, persist_state as persist_binding_state,
    register_runtime_binding, DIoTRuntimeBindingV1, RegisterDIoTRuntimeBindingError,
};
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DIOT_RUNTIME_BINDING_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

/// `pub(crate)`, innermost in this domain's lock chain (see module doc).
pub(crate) fn diot_runtime_binding_transaction() -> &'static Mutex<()> {
    DIOT_RUNTIME_BINDING_TRANSACTION.get_or_init(|| Mutex::new(()))
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
    internal_error("diot runtime binding transaction lock poisoned")
}

fn register_error(error: RegisterDIoTRuntimeBindingError) -> ApiError {
    match error {
        RegisterDIoTRuntimeBindingError::Invalid(finding) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": finding.code, "message": finding.message})),
        ),
        RegisterDIoTRuntimeBindingError::NoActiveDMap2Authority(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "diot_runtime_binding_no_active_dmap2", "message": message})),
        ),
        RegisterDIoTRuntimeBindingError::PlacementUnknown(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "diot_runtime_binding_placement_unknown", "message": message})),
        ),
        RegisterDIoTRuntimeBindingError::NodeUnknown(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "diot_runtime_binding_node_unknown", "message": message})),
        ),
        RegisterDIoTRuntimeBindingError::ArtifactAuthorityMissing(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "diot_runtime_binding_artifact_authority_missing", "message": message})),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRuntimeBindingRequest {
    pub broker_uri: String,
}

/// `PUT /api/v1/diot/runtime-bindings/:project_id/:application_id/:placement_id`.
/// `placement_id` MUST already be a DIoT placement in the currently active
/// DMap/2 -- every identity field besides `broker_uri` is derived from that
/// active authority, never from the request body.
pub async fn put_diot_runtime_binding(
    axum::extract::Path((project_id, application_id, placement_id)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
    Json(request): Json<RegisterRuntimeBindingRequest>,
) -> Result<JsonResponse<DIoTRuntimeBindingV1>, ApiError> {
    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _diot_runtime_binding_transaction = diot_runtime_binding_transaction()
        .lock()
        .map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let mut binding_state = load_binding_state(&db_path).map_err(internal_error)?;

    let binding = register_runtime_binding(
        &mut binding_state,
        &ddeploy_state,
        &dnode_registry_state,
        &project_id,
        &application_id,
        &placement_id,
        &request.broker_uri,
        &Utc::now().to_rfc3339(),
    )
    .map_err(register_error)?;

    persist_binding_state(&db_path, &binding_state).map_err(internal_error)?;
    Ok(JsonResponse(binding))
}

/// `GET /api/v1/diot/runtime-bindings/:project_id/:application_id/:placement_id`.
pub async fn get_diot_runtime_binding(
    axum::extract::Path((project_id, application_id, placement_id)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
) -> Result<JsonResponse<DIoTRuntimeBindingV1>, ApiError> {
    let state = load_binding_state(&db_path()).map_err(internal_error)?;
    state
        .get(&project_id, &application_id, &placement_id)
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!(
                        "no DIoT runtime binding registered for project '{project_id}' application '{application_id}' placement '{placement_id}'"
                    )
                })),
            )
        })
}

#[cfg(test)]
mod tests {
    fn production_source() -> String {
        let source = include_str!("diot_runtime_binding.rs");
        let production_source: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            production_source.len() < source.len(),
            "the '#[cfg(test)] mod tests' boundary was not found -- this test would otherwise scan its own forbidden-string literals"
        );
        production_source
    }

    fn fn_body(source: &str, fn_signature_prefix: &str) -> String {
        let start = source
            .find(fn_signature_prefix)
            .unwrap_or_else(|| panic!("'{fn_signature_prefix}' not found in source"));
        let rest = &source[start..];
        let next_fn = rest[fn_signature_prefix.len()..]
            .find("\npub async fn ")
            .unwrap_or(rest.len() - fn_signature_prefix.len());
        rest[..fn_signature_prefix.len() + next_fn].to_string()
    }

    #[test]
    fn put_diot_runtime_binding_holds_the_established_lock_order() {
        let body = fn_body(&production_source(), "pub async fn put_diot_runtime_binding(");
        let ddeploy_lock_pos = body
            .find("ddeploy::ddeploy_transaction()")
            .expect("put_diot_runtime_binding must acquire DDEPLOY_TRANSACTION");
        let dnode_lock_pos = body
            .find("dnode_registry::dnode_transaction()")
            .expect("put_diot_runtime_binding must acquire DNODE_TRANSACTION");
        let own_lock_pos = body
            .find("diot_runtime_binding_transaction()")
            .expect("put_diot_runtime_binding must acquire its own DIOT_RUNTIME_BINDING_TRANSACTION");
        assert!(
            ddeploy_lock_pos < dnode_lock_pos,
            "DDEPLOY_TRANSACTION must be acquired before DNODE_TRANSACTION"
        );
        assert!(
            dnode_lock_pos < own_lock_pos,
            "DNODE_TRANSACTION must be acquired before DIOT_RUNTIME_BINDING_TRANSACTION"
        );
        assert!(
            !body.contains("dcode_transaction()") && !body.contains("dcall_transaction()"),
            "registration never touches D-Code or D-Call state and must not acquire either lock"
        );
    }

    #[test]
    fn get_diot_runtime_binding_never_acquires_any_transaction_lock() {
        let source = production_source();
        let body = fn_body(&source, "pub async fn get_diot_runtime_binding(");
        assert!(
            !body.contains("_transaction()"),
            "a plain read must not acquire any write-path lock: {body}"
        );
    }

    #[test]
    fn register_runtime_binding_request_rejects_unknown_fields() {
        let error = serde_json::from_str::<super::RegisterRuntimeBindingRequest>(
            r#"{"broker_uri":"mqtt://broker.lab:1883","node_id":"fog-01"}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}

use std::{collections::BTreeMap, sync::{OnceLock, RwLock}};

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    Json as JsonResponse,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::sentinel_registration::{
    build_sentinel_registration_list, build_sentinel_registration_record,
    materialize_sentinel_registration, refresh_sentinel_registration,
    SentinelHeartbeatRequest, SentinelRegistrationList, SentinelRegistrationRecord,
    SentinelRegistrationRequest, SentinelRegistrationView,
};

type ApiError = (StatusCode, JsonResponse<Value>);

static SENTINEL_REGISTRATIONS: OnceLock<RwLock<BTreeMap<String, SentinelRegistrationRecord>>> =
    OnceLock::new();

fn registration_store() -> &'static RwLock<BTreeMap<String, SentinelRegistrationRecord>> {
    SENTINEL_REGISTRATIONS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

pub async fn put_sentinel_registration(
    Path(node_id): Path<String>,
    Json(payload): Json<SentinelRegistrationRequest>,
) -> Result<JsonResponse<SentinelRegistrationView>, ApiError> {
    let path_node = node_id.trim();
    if path_node != payload.node_id.trim() {
        return Err(bad_request(format!(
            "registration path node_id '{}' does not match payload node_id '{}'",
            node_id, payload.node_id
        )));
    }

    let now = Utc::now();
    let record = build_sentinel_registration_record(&payload, now).map_err(bad_request)?;
    {
        let mut registrations = registration_store().write().map_err(lock_error)?;
        if let Some(existing) = registrations.get(path_node) {
            if existing.instance_id != record.instance_id {
                return Err((
                    StatusCode::CONFLICT,
                    JsonResponse(json!({
                        "error": format!(
                            "node '{}' is already registered by instance '{}'",
                            path_node, existing.instance_id
                        )
                    })),
                ));
            }
        }
        registrations.insert(path_node.to_string(), record.clone());
    }

    let view = materialize_sentinel_registration(&record, now).map_err(bad_request)?;
    Ok(JsonResponse(view))
}

pub async fn post_sentinel_heartbeat(
    Path(node_id): Path<String>,
    Json(payload): Json<SentinelHeartbeatRequest>,
) -> Result<JsonResponse<SentinelRegistrationView>, ApiError> {
    let path_node = node_id.trim();
    if path_node != payload.node_id.trim() {
        return Err(bad_request(format!(
            "heartbeat path node_id '{}' does not match payload node_id '{}'",
            node_id, payload.node_id
        )));
    }

    let now = Utc::now();
    let refreshed = {
        let mut registrations = registration_store().write().map_err(lock_error)?;
        let existing = registrations.get(path_node).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!("Sentinel node '{}' is not registered", path_node)
                })),
            )
        })?;
        let refreshed = refresh_sentinel_registration(&existing, &payload, now)
            .map_err(|message| (StatusCode::CONFLICT, JsonResponse(json!({"error": message}))))?;
        registrations.insert(path_node.to_string(), refreshed.clone());
        refreshed
    };

    let view = materialize_sentinel_registration(&refreshed, now).map_err(bad_request)?;
    Ok(JsonResponse(view))
}

pub async fn get_sentinel_registration(
    Path(node_id): Path<String>,
) -> Result<JsonResponse<SentinelRegistrationView>, ApiError> {
    let now = Utc::now();
    let record = registration_store()
        .read()
        .map_err(lock_error)?
        .get(node_id.trim())
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!("Sentinel node '{}' is not registered", node_id)
                })),
            )
        })?;
    let view = materialize_sentinel_registration(&record, now).map_err(bad_request)?;
    Ok(JsonResponse(view))
}

pub async fn list_sentinel_registrations(
) -> Result<JsonResponse<SentinelRegistrationList>, ApiError> {
    let now = Utc::now();
    let records = registration_store()
        .read()
        .map_err(lock_error)?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let list = build_sentinel_registration_list(&records, now).map_err(bad_request)?;
    Ok(JsonResponse(list))
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({ "error": message })),
    )
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({
            "error": format!("Sentinel registration state lock poisoned: {error}")
        })),
    )
}

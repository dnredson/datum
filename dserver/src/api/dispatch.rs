use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::dispatch_policy::{apply_dispatch_policy, filter_dispatch_for_node},
    AppState,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    match project_id {
        Some(p) if !p.is_empty() => Ok(p),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        )),
    }
}

pub async fn publish_latest_dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw_plan = state
        .firebase
        .get_action_plan_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_action_plan_latest failed: {}", e),
            )
        })?;

    let plan = parse_firestore_payload_document(&raw_plan, "action plan")?;
    let dispatch = apply_dispatch_policy(&plan);

    state
        .firebase
        .store_dispatch_latest(&project_id, &dispatch)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store_dispatch_latest failed: {}", e),
            )
        })?;

    Ok(Json(dispatch))
}

pub async fn get_materialized_dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_dispatch_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_dispatch_latest failed: {}", e),
            )
        })?;

    let dispatch = parse_firestore_payload_document(&raw, "dispatch")?;

    Ok(Json(dispatch))
}

pub async fn get_dispatch_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_dispatch_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_dispatch_latest failed: {}", e),
            )
        })?;

    let dispatch = parse_firestore_payload_document(&raw, "dispatch")?;
    let scoped = filter_dispatch_for_node(&dispatch, &project_id, &node_id);

    Ok(Json(scoped))
}

fn parse_firestore_payload_document(
    raw: &serde_json::Value,
    label: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let payload = raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{label} payload missing"),
        ))?;

    serde_json::from_str(payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid {label} payload: {e}"),
        )
    })
}
#[cfg(test)]
mod tests {
    use super::{filter_dispatch_for_node, parse_firestore_payload_document};
    use axum::http::StatusCode;
    use serde_json::json;

    #[test]
    fn filters_dispatch_actions_by_target_node() {
        let dispatch = json!({
            "project_id": "smart-city-campinas",
            "actions": [
                { "action_id": "a1", "target_node": "fog-01" },
                { "action_id": "a2", "target_node": "fog-02" },
                { "action_id": "a3", "target_node": "fog-01" }
            ]
        });

        let scoped = filter_dispatch_for_node(&dispatch, "smart-city-campinas", "fog-01");

        assert_eq!(
            scoped.get("project_id").and_then(|v| v.as_str()),
            Some("smart-city-campinas")
        );
        assert_eq!(
            scoped.get("node_id").and_then(|v| v.as_str()),
            Some("fog-01")
        );
        assert_eq!(scoped["summary"]["assigned_actions"].as_u64(), Some(2));
        assert_eq!(
            scoped["summary"]["assigned_blocked_actions"].as_u64(),
            Some(0)
        );
        assert_eq!(
            scoped["summary"]["assigned_deferred_actions"].as_u64(),
            Some(0)
        );
        assert_eq!(scoped["actions"].as_array().map(|v| v.len()), Some(2));
    }

    #[test]
    fn returns_empty_actions_when_node_has_no_dispatch() {
        let dispatch = json!({
            "actions": [
                { "action_id": "a1", "target_node": "fog-02" }
            ]
        });

        let scoped = filter_dispatch_for_node(&dispatch, "smart-city-campinas", "fog-01");

        assert_eq!(scoped["summary"]["assigned_actions"].as_u64(), Some(0));
        assert_eq!(scoped["actions"].as_array().map(|v| v.len()), Some(0));
    }

    #[test]
    fn parses_firestore_payload_document() {
        let raw = json!({
            "fields": {
                "payload": {
                    "stringValue": "{\"actions\":[{\"target_node\":\"fog-01\"}]}"
                }
            }
        });

        let parsed =
            parse_firestore_payload_document(&raw, "dispatch").expect("payload should parse");

        assert_eq!(parsed["actions"].as_array().map(|v| v.len()), Some(1));
    }

    #[test]
    fn rejects_missing_firestore_payload_document() {
        let raw = json!({ "fields": {} });
        let err = parse_firestore_payload_document(&raw, "dispatch")
            .expect_err("missing payload should fail");

        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.1.contains("dispatch payload missing"));
    }
}

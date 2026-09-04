use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::AppState;

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

pub async fn list_nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let result = state.firebase.list_nodes(&project_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list_nodes failed: {}", e),
        )
    })?;

    Ok(Json(result))
}

pub async fn get_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let normalized_node_id = node_id.trim();

    let nodes = state.firebase.list_nodes(&project_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list_nodes failed while resolving node '{}': {}", normalized_node_id, e),
        )
    })?;

    if !node_exists(&nodes, normalized_node_id) {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "node '{}' is not known in project '{}'",
                normalized_node_id, project_id
            ),
        ));
    }

    let result = state
        .firebase
        .get_latest_snapshot(&project_id, normalized_node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_latest failed: {}", e),
            )
        })?;

    Ok(Json(result))
}

fn node_exists(nodes: &serde_json::Value, node_id: &str) -> bool {
    nodes
        .get("documents")
        .and_then(serde_json::Value::as_array)
        .map(|documents| {
            documents.iter().any(|document| {
                document
                    .get("fields")
                    .and_then(|fields| fields.get("node_id"))
                    .and_then(|value| value.get("stringValue"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    == Some(node_id)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::node_exists;

    #[test]
    fn node_exists_matches_project_node_document() {
        let nodes = json!({
            "documents": [
                {"fields": {"node_id": {"stringValue": "fog-01"}}},
                {"fields": {"node_id": {"stringValue": "cloud-01"}}}
            ]
        });

        assert!(node_exists(&nodes, "fog-01"));
        assert!(node_exists(&nodes, "cloud-01"));
        assert!(!node_exists(&nodes, "fog-02"));
    }
}

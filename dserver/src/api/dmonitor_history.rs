//! Canonical historical read-only query API over persisted DMonitor
//! evidence (Phase 140A). Read-only diagnostic/explorer endpoint: no
//! mutation, no remediation, no placement decision -- it never reads
//! `core::ddeploy`/`active_placement_authority` at all (see
//! `core::dmonitor_history`'s own doc comment for why).
//!
//! `GET /api/v1/dmonitor/history/:project_id` mirrors
//! `dmonitor_convergence`'s own project-id-in-path convention for the
//! DMonitor API family, with optional `application_id`/`node_id`/
//! `service_id`/`start_utc`/`end_utc`/`limit` query filters mirroring
//! `list_operational_evidence`'s own query-parameter convention.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    Json as JsonResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path as FsPath;

use crate::core::dmonitor_history::{query_dmonitor_history, DMonitorHistoryFilterV1, DMonitorHistoryQueryResponseV1};
use crate::core::dmonitor_readiness::default_readiness_policy;
use crate::core::dmonitor_state::load_state as load_dmonitor_state;
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

#[derive(Debug, Deserialize)]
pub struct DMonitorHistoryQuery {
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub service_id: Option<String>,
    #[serde(default)]
    pub start_utc: Option<String>,
    #[serde(default)]
    pub end_utc: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::UNPROCESSABLE_ENTITY, JsonResponse(json!({"error": "invalid_history_query", "message": message})))
}

fn internal_error(message: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message})))
}

/// The real query logic, parameterized by an explicit `db_path` rather than
/// reaching for the global configured path itself -- this is what makes a
/// genuine real-HTTP test possible without mutating any process-global
/// environment variable (a risk shared by every handler that instead reads
/// `default_control_state_db_path()` directly, which is why no other
/// DMonitor API handler in this codebase is exercised over real HTTP
/// either). The production handler below is a thin wrapper that supplies
/// the real configured path.
pub fn handle_dmonitor_history_query(
    db_path: &FsPath,
    project_id: &str,
    query: &DMonitorHistoryQuery,
) -> Result<DMonitorHistoryQueryResponseV1, ApiError> {
    let state = load_dmonitor_state(db_path).map_err(internal_error)?;
    let limit = query.limit.unwrap_or(100);
    let filter = DMonitorHistoryFilterV1 {
        application_id: query.application_id.clone(),
        node_id: query.node_id.clone(),
        service_id: query.service_id.clone(),
    };
    let queried_at_utc = chrono::Utc::now().to_rfc3339();
    let policy = default_readiness_policy().freshness_policy;
    query_dmonitor_history(
        &state,
        project_id,
        &filter,
        query.start_utc.as_deref(),
        query.end_utc.as_deref(),
        limit,
        &queried_at_utc,
        &policy,
    )
    .map_err(|message| bad_request(message))
}

pub async fn get_dmonitor_history(
    Path(project_id): Path<String>,
    Query(query): Query<DMonitorHistoryQuery>,
) -> Result<JsonResponse<DMonitorHistoryQueryResponseV1>, ApiError> {
    let db_path = default_control_state_db_path();
    handle_dmonitor_history_query(&db_path, &project_id, &query).map(JsonResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorHealthV1, DMonitorSubjectV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::dmonitor_state::{ingest_observation, persist_state, DMonitorStateV1};
    use axum::{routing::get, Router};

    fn observation(application_id: &str, service_id: &str, node_id: &str, observed_at_utc: &str, observation_id: &str, sequence: u64) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: observation_id.to_string(),
            project_id: Some("proj:lab".to_string()),
            node_id: node_id.to_string(),
            monitor_instance_id: "mon-1".to_string(),
            sequence,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test".to_string(),
            subject: DMonitorSubjectV1::DServRealization {
                project_id: "proj:lab".to_string(),
                application_id: application_id.to_string(),
                service_id: service_id.to_string(),
                node_id: node_id.to_string(),
                dmap_ref: None,
            },
            health: DMonitorHealthV1::Healthy,
            facts: serde_json::json!({}),
        }
    }

    struct Server {
        base: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// A real axum router + a real bound TCP listener + a real `reqwest`
    /// client, exercising the full extractor/serialization/status-code
    /// path -- not only the internal `query_dmonitor_history` helper
    /// (already covered by `core::dmonitor_history`'s own 19 tests). The
    /// temp db path is injected directly into this test-only route closure
    /// (never the global `DATUM_CONTROL_STATE_DB_PATH` env var), so this
    /// test is immune to interference from any other test in the same
    /// binary.
    async fn serve(db_path: std::path::PathBuf) -> Server {
        let router = Router::new().route(
            "/api/v1/dmonitor/history/:project_id",
            get(move |path, query| {
                let db_path = db_path.clone();
                async move {
                    let axum::extract::Path(project_id): axum::extract::Path<String> = path;
                    let axum::extract::Query(query): axum::extract::Query<DMonitorHistoryQuery> = query;
                    handle_dmonitor_history_query(&db_path, &project_id, &query).map(JsonResponse)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        Server {
            base,
            task: tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }),
        }
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("datum-phase140a-history-test-{name}-{}.json", std::process::id()));
        path
    }

    #[tokio::test]
    async fn real_http_query_returns_bounded_chronological_filtered_json() {
        let db_path = temp_db_path("basic");
        let _ = std::fs::remove_file(&db_path);
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, "proj:lab", observation("app-1", "svc-a", "fog-01", "2026-01-01T00:00:01Z", "obs-1", 1), "2026-01-01T00:00:01Z").unwrap();
        ingest_observation(&mut state, "proj:lab", observation("app-1", "svc-b", "fog-01", "2026-01-01T00:00:02Z", "obs-2", 2), "2026-01-01T00:00:02Z").unwrap();
        persist_state(&db_path, &state).unwrap();

        let server = serve(db_path.clone()).await;
        let client = reqwest::Client::new();
        let url = format!(
            "{}/api/v1/dmonitor/history/proj:lab?start_utc=2026-01-01T00:00:00Z&end_utc=2026-01-01T00:00:03Z&service_id=svc-a",
            server.base
        );
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["project_id"], "proj:lab");
        assert_eq!(body["observations"].as_array().unwrap().len(), 1);
        assert_eq!(body["observations"][0]["observation_id"], "obs-1");
        assert_eq!(body["truncated"], false);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn real_http_invalid_window_is_rejected_with_a_clear_422() {
        let db_path = temp_db_path("invalid-window");
        let _ = std::fs::remove_file(&db_path);
        persist_state(&db_path, &DMonitorStateV1::empty()).unwrap();
        let server = serve(db_path.clone()).await;
        let client = reqwest::Client::new();
        let url = format!(
            "{}/api/v1/dmonitor/history/proj:lab?start_utc=2026-01-02T00:00:00Z&end_utc=2026-01-01T00:00:00Z",
            server.base
        );
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 422);
        let body: Value = response.json().await.unwrap();
        assert!(body["message"].as_str().unwrap().contains("must not be after"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn real_http_default_window_never_returns_unbounded_history() {
        let db_path = temp_db_path("default-window");
        let _ = std::fs::remove_file(&db_path);
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, "proj:lab", observation("app-1", "svc-a", "fog-01", "2020-01-01T00:00:00Z", "obs-ancient", 1), "2020-01-01T00:00:00Z").unwrap();
        persist_state(&db_path, &state).unwrap();
        let server = serve(db_path.clone()).await;
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/dmonitor/history/proj:lab", server.base);
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["observations"].as_array().unwrap().len(), 0, "an ancient record must never appear in the DEFAULT (no start_utc/end_utc supplied) window");
        let _ = std::fs::remove_file(&db_path);
    }
}

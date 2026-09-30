//! Phase 128: thin read-only HTTP bridge to Phase 127 projections.
//! Every acquisition/projection runs on an owned, capacity-bounded blocking task.
use axum::{
    extract::{rejection::PathRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::{
    core::{
        ddeploy::DDeployStateV1,
        dmonitor_state::DMonitorStateV1,
        dnode_registry::DNodeRegistryStateV1,
        event_producers,
        operations_read_models::{
            OperationsReadError, OperationsRepository, OperationsSource, OperationsSources,
            OverviewQuery, ReadErrorKind,
        },
    },
    storage::control_state,
};

type SnapshotProvider = dyn Fn() -> Result<OperationsSources, OperationsReadError> + Send + Sync;
#[derive(Clone)]
pub struct OperationsApiRuntime {
    provider: Arc<SnapshotProvider>,
    permits: Arc<Semaphore>,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}
impl OperationsApiRuntime {
    pub fn from_process() -> Self {
        let events = event_producers::process_runtime();
        // Resolve once, not from an HTTP request. Do not initialize either DB.
        let control_path = std::env::current_dir()
            .map_err(|e| e.to_string())
            .and_then(|base| {
                let configured = control_state::default_control_state_db_path();
                let text = configured
                    .to_str()
                    .ok_or("unsupported control-state path encoding")?;
                event_producers::resolve_database_path(Some(text), &base).map_err(|e| e.to_string())
            });
        Self::new(move || {
            let mut sources = OperationsSources::default()
                .with_repository(OperationsRepository::from_runtime(&events)?);
            sources = match super::operational_dgraph::operations_graph_snapshot() {
                Ok(Some(graphs)) => sources.with_project_graphs(graphs),
                Ok(None) => sources,
                Err(e) => sources.with_source_error(
                    OperationsSource::ProjectGraphs,
                    issue(ReadErrorKind::SourceUnavailable, "operational_graphs", e),
                ),
            };
            let domains = [
                control_state::DOMAIN_DDEPLOY_STATE,
                control_state::DOMAIN_DMONITOR_STATE,
                control_state::DOMAIN_DNODE_REGISTRY,
            ];
            let blobs = control_path
                .as_ref()
                .map_err(|e| (ReadErrorKind::SourceUnavailable, e.clone()))
                .and_then(|p| {
                    control_state::read_domains_only(p, &domains).map_err(|e| {
                        let corrupt = matches!(&e, rusqlite::Error::SqliteFailure(code, _) if matches!(code.code,
                            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))
                            || matches!(&e, rusqlite::Error::InvalidColumnType(..));
                        (if corrupt { ReadErrorKind::SourceCorrupt } else { ReadErrorKind::SourceUnavailable }, e.to_string())
                    })
                });
            match blobs {
                Err((kind, e)) => {
                    for (source, name) in [
                        (OperationsSource::Deployment, "ddeploy"),
                        (OperationsSource::DMonitor, "dmonitor"),
                        (OperationsSource::Nodes, "dnode_registry"),
                    ] {
                        sources =
                            sources.with_source_error(source, issue(kind.clone(), name, e.clone()));
                    }
                }
                Ok(blobs) => {
                    for (domain, source, name) in [
                        (domains[0], OperationsSource::Deployment, "ddeploy"),
                        (domains[1], OperationsSource::DMonitor, "dmonitor"),
                        (domains[2], OperationsSource::Nodes, "dnode_registry"),
                    ] {
                        if let Some(Some(blob)) = blobs.get(domain) {
                            sources = match source {
                                OperationsSource::Deployment => match serde_json::from_str::<
                                    DDeployStateV1,
                                >(
                                    blob
                                ) {
                                    Ok(v) => sources.with_deployment(v),
                                    Err(e) => sources.with_source_error(
                                        source,
                                        issue(ReadErrorKind::SourceCorrupt, name, e.to_string()),
                                    ),
                                },
                                OperationsSource::DMonitor => match serde_json::from_str::<
                                    DMonitorStateV1,
                                >(
                                    blob
                                ) {
                                    Ok(v) => {
                                        sources.with_dmonitor(v, Default::default(), vec![])?
                                    }
                                    Err(e) => sources.with_source_error(
                                        source,
                                        issue(ReadErrorKind::SourceCorrupt, name, e.to_string()),
                                    ),
                                },
                                OperationsSource::Nodes => match serde_json::from_str::<
                                    DNodeRegistryStateV1,
                                >(
                                    blob
                                ) {
                                    Ok(v) => sources.with_nodes(v),
                                    Err(e) => sources.with_source_error(
                                        source,
                                        issue(ReadErrorKind::SourceCorrupt, name, e.to_string()),
                                    ),
                                },
                                _ => unreachable!(),
                            };
                        }
                    }
                }
            }
            Ok(sources)
        })
    }
    fn new(
        provider: impl Fn() -> Result<OperationsSources, OperationsReadError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider: Arc::new(provider),
            permits: Arc::new(Semaphore::new(4)),
            clock: Arc::new(Utc::now),
        }
    }
    async fn build<T: Serialize + Send + 'static>(
        &self,
        project: impl FnOnce(
                OperationsSources,
                DateTime<Utc>,
                OverviewQuery,
            ) -> Result<T, OperationsReadError>
            + Send
            + 'static,
    ) -> Result<Json<T>, ApiFailure> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiFailure::busy())?;
        let provider = self.provider.clone();
        let at = (self.clock)();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            project(provider()?, at, OverviewQuery::at(at))
        })
        .await
        .map_err(|_| ApiFailure::projection())?;
        result.map(Json).map_err(ApiFailure::from)
    }
}
fn issue(kind: ReadErrorKind, source: &str, detail: impl Into<String>) -> OperationsReadError {
    OperationsReadError::source_failure(kind, source, detail)
}

#[derive(Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    source: String,
    scope: Option<crate::core::operations_read_models::ReadIssueScope>,
}
struct ApiFailure {
    status: StatusCode,
    error: ErrorBody,
}
impl ApiFailure {
    fn fixed(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
                source: "operations_api".into(),
                scope: None,
            },
        }
    }
    fn invalid() -> Self {
        Self::fixed(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "A valid project route and matching X-Datum-Project header are required.",
        )
    }
    fn busy() -> Self {
        Self::fixed(
            StatusCode::SERVICE_UNAVAILABLE,
            "read_capacity_exhausted",
            "The read service is busy. Please retry.",
        )
    }
    fn projection() -> Self {
        Self::fixed(
            StatusCode::INTERNAL_SERVER_ERROR,
            "projection_failed",
            "The projection could not be produced.",
        )
    }
}
impl From<OperationsReadError> for ApiFailure {
    fn from(e: OperationsReadError) -> Self {
        let status = match e.kind {
            ReadErrorKind::InvalidQuery => StatusCode::BAD_REQUEST,
            ReadErrorKind::NotConfigured
            | ReadErrorKind::SourceUnavailable
            | ReadErrorKind::Storage => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            error: ErrorBody {
                code: e.code().into(),
                message: e.safe_message().into(),
                source: e.source().into(),
                scope: e.scope().cloned(),
            },
        }
    }
}
impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.error })),
        )
            .into_response()
    }
}
pub fn router(runtime: OperationsApiRuntime) -> Router {
    Router::new()
        .route("/api/v1/operations/overview", get(overview))
        .route(
            "/api/v1/operations/projects/:project_id",
            get(project_overview),
        )
        .route(
            "/api/v1/operations/projects/:project_id/graph",
            get(project_graph),
        )
        .route(
            "/api/v1/operations/projects/:project_id/runtime-graph",
            get(project_runtime_graph),
        )
        .with_state(runtime)
}
async fn overview(
    State(runtime): State<OperationsApiRuntime>,
) -> Result<impl IntoResponse, ApiFailure> {
    runtime
        .build(|sources, at, query| sources.global_overview(at, &query))
        .await
        .map(|body| ([("cache-control", "no-store")], body))
}
async fn project_overview(
    State(runtime): State<OperationsApiRuntime>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiFailure> {
    let Path(project) = path.map_err(|_| ApiFailure::invalid())?;
    if project.is_empty()
        || project.len() > 256
        || project.trim() != project
        || project.chars().any(char::is_control)
        || headers
            .get("x-datum-project")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            != Some(project.as_str())
    {
        return Err(ApiFailure::invalid());
    }
    runtime
        .build(move |sources, at, query| sources.project_overview(&project, at, &query))
        .await
        .map(|body| ([("cache-control", "no-store")], body))
}

async fn project_graph(
    State(runtime): State<OperationsApiRuntime>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiFailure> {
    let Path(project) = path.map_err(|_| ApiFailure::invalid())?;
    if project.is_empty()
        || project.len() > 256
        || project.trim() != project
        || project.chars().any(char::is_control)
        || headers
            .get("x-datum-project")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            != Some(project.as_str())
    {
        return Err(ApiFailure::invalid());
    }
    runtime
        .build(move |sources, at, _| sources.project_graph_overview(&project, at))
        .await
        .map(|body| ([("cache-control", "no-store")], body))
}

async fn project_runtime_graph(
    State(runtime): State<OperationsApiRuntime>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiFailure> {
    let Path(project) = path.map_err(|_| ApiFailure::invalid())?;
    if project.is_empty()
        || project.len() > 256
        || project.trim() != project
        || project.chars().any(char::is_control)
        || headers
            .get("x-datum-project")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            != Some(project.as_str())
    {
        return Err(ApiFailure::invalid());
    }
    runtime
        .build(move |sources, at, _| sources.project_runtime_graph(&project, at))
        .await
        .map(|body| ([("cache-control", "no-store")], body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;
    struct Server {
        base: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn serve(runtime: OperationsApiRuntime) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = router(runtime);
        Server {
            base,
            task: tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            }),
        }
    }
    fn fixed_runtime() -> OperationsApiRuntime {
        let mut runtime = OperationsApiRuntime::new(|| Ok(OperationsSources::default()));
        runtime.clock = Arc::new(|| "2026-09-30T15:00:00Z".parse().unwrap());
        runtime
    }
    #[tokio::test]
    async fn runtime_graph_api_scoped_read_only_projection_with_blocking_acquisition() {
        let async_thread = std::thread::current().id();
        let runtime = OperationsApiRuntime::new(move || {
            assert_ne!(std::thread::current().id(), async_thread);
            Ok(OperationsSources::default())
        });
        let server = serve(runtime).await;
        let path = format!("{}/api/v1/operations/projects/p/runtime-graph", server.base);
        let client = reqwest::Client::new();
        let response = client
            .get(&path)
            .header("X-Datum-Project", "p")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["project_id"], "p");
        assert_eq!(body["logical"]["project_id"], "p");
        assert_eq!(
            body["runtime_source_kind"],
            "admitted_dserv_realization_evidence"
        );
        assert_eq!(body["runtime"]["availability"], "not_configured");
        assert_eq!(
            client
                .get(&path)
                .header("X-Datum-Project", "q")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            client.post(&path).send().await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    #[tokio::test]
    async fn runtime_graph_source_failure_remains_successful_safe_and_separate() {
        let server = serve(OperationsApiRuntime::new(|| {
            Ok(OperationsSources::default().with_source_error(
                OperationsSource::DMonitor,
                issue(
                    ReadErrorKind::SourceCorrupt,
                    "dmonitor",
                    "SQL secret C:\\private.db",
                ),
            ))
        }))
        .await;
        let response = reqwest::Client::new()
            .get(format!(
                "{}/api/v1/operations/projects/p/runtime-graph",
                server.base
            ))
            .header("X-Datum-Project", "p")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["runtime"]["availability"], "corrupt");
        assert_eq!(body["logical"]["graph"]["availability"], "not_configured");
        assert_eq!(body["temporal_quality"], serde_json::json!([]));
        assert!(!body.to_string().contains("secret"));
        assert!(!body.to_string().contains("private.db"));
    }
    #[tokio::test]
    async fn graph_api_is_scoped_thin_read_projection_and_has_no_write_route() {
        let runtime = fixed_runtime();
        let expected = serde_json::to_value(
            OperationsSources::default()
                .project_graph_overview("project one", (runtime.clock)())
                .unwrap(),
        )
        .unwrap();
        let server = serve(runtime).await;
        let path = format!(
            "{}/api/v1/operations/projects/project%20one/graph",
            server.base
        );
        let client = reqwest::Client::new();
        let response = client
            .get(&path)
            .header("X-Datum-Project", "project one")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.json::<Value>().await.unwrap(), expected);
        assert_eq!(
            client
                .get(&path)
                .header("X-Datum-Project", "other")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            client.post(&path).send().await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    #[tokio::test]
    async fn graph_api_degrades_safely_without_leaking_diagnostics() {
        let server = serve(OperationsApiRuntime::new(|| {
            Ok(OperationsSources::default().with_source_error(
                OperationsSource::Deployment,
                issue(
                    ReadErrorKind::SourceCorrupt,
                    "ddeploy",
                    "SQL private-secret C:\\hidden.db",
                ),
            ))
        }))
        .await;
        let response = reqwest::Client::new()
            .get(format!(
                "{}/api/v1/operations/projects/p/graph",
                server.base
            ))
            .header("X-Datum-Project", "p")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(body.contains("corrupt"));
        assert!(!body.contains("private-secret"));
        assert!(!body.contains("hidden.db"));
    }
    #[tokio::test]
    async fn overview_is_exact_phase127_projection() {
        let runtime = fixed_runtime();
        let at = (runtime.clock)();
        let expected = serde_json::to_value(
            OperationsSources::default()
                .global_overview(at, &OverviewQuery::at(at))
                .unwrap(),
        )
        .unwrap();
        let server = serve(runtime).await;
        let response = reqwest::get(format!("{}/api/v1/operations/overview", server.base))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.json::<Value>().await.unwrap(), expected);
    }
    #[tokio::test]
    async fn degraded_sources_return_success_and_safe_issue() {
        let server = serve(OperationsApiRuntime::new(|| {
            Ok(OperationsSources::default().with_source_error(
                OperationsSource::DMonitor,
                issue(
                    ReadErrorKind::SourceCorrupt,
                    "dmonitor",
                    "SQL secret C:\\private\\credentials.db",
                ),
            ))
        }))
        .await;
        let response = reqwest::get(format!("{}/api/v1/operations/overview", server.base))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let text = response.text().await.unwrap();
        assert!(text.contains("source_corrupt"));
        assert!(!text.contains("credentials"));
        assert!(!text.contains("private"));
    }
    #[tokio::test]
    async fn top_level_failure_has_safe_error_envelope() {
        let server = serve(OperationsApiRuntime::new(|| {
            Err(issue(
                ReadErrorKind::InternalProjection,
                "query",
                "secret raw SQL",
            ))
        }))
        .await;
        let response = reqwest::get(format!("{}/api/v1/operations/overview", server.base))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["error"]["code"], "projection_failed");
        assert!(!body.to_string().contains("secret"));
    }
    #[tokio::test]
    async fn project_scoping_and_encoded_identity_are_preserved() {
        let server = serve(fixed_runtime()).await;
        let client = reqwest::Client::new();
        let path = format!("{}/api/v1/operations/projects/project%20one", server.base);
        let response = client
            .get(&path)
            .header("X-Datum-Project", "project one")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.json::<Value>().await.unwrap()["project_id"],
            "project one"
        );
        for header in [None, Some("other")] {
            let mut request = client.get(&path);
            if let Some(h) = header {
                request = request.header("X-Datum-Project", h);
            }
            assert_eq!(
                request.send().await.unwrap().status(),
                StatusCode::BAD_REQUEST
            );
        }
    }
    #[tokio::test]
    async fn invalid_path_is_safe_and_post_is_not_supported() {
        let server = serve(fixed_runtime()).await;
        let client = reqwest::Client::new();
        for path in ["%20", "%FF", "a%0Ab"] {
            let response = client
                .get(format!("{}/api/v1/operations/projects/{path}", server.base))
                .header("x-datum-project", "a")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"]["code"],
                "invalid_scope"
            );
        }
        assert_eq!(
            client
                .post(format!("{}/api/v1/operations/overview", server.base))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn blocking_projection_does_not_pin_async_worker_and_capacity_is_bounded() {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let entered = std::sync::Mutex::new(Some(entered_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release = std::sync::Mutex::new(release_rx);
        let mut runtime = OperationsApiRuntime::new(move || {
            entered.lock().unwrap().take().unwrap().send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
            Ok(OperationsSources::default())
        });
        runtime.permits = Arc::new(Semaphore::new(1));
        let cloned = runtime.clone();
        let task =
            tokio::spawn(async move { cloned.build(|s, at, q| s.global_overview(at, &q)).await });
        entered_rx.await.unwrap();
        // This task can only progress if the blocked provider is off this sole worker.
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        assert!(matches!(
            runtime.build(|s, at, q| s.global_overview(at, &q)).await,
            Err(ApiFailure {
                status: StatusCode::SERVICE_UNAVAILABLE,
                ..
            })
        ));
        release_tx.send(()).unwrap();
        assert!(task.await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn unavailable_configuration_maps_to_503_without_diagnostics() {
        let server = serve(OperationsApiRuntime::new(|| {
            Err(issue(
                ReadErrorKind::SourceUnavailable,
                "event_alert_configuration",
                "ENV=secret/path",
            ))
        }))
        .await;
        let response = reqwest::get(format!("{}/api/v1/operations/overview", server.base))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response.text().await.unwrap().contains("secret"));
    }
    #[test]
    fn read_only_control_acquisition_never_creates_missing_database() {
        let dir = std::env::temp_dir().join(format!("phase128-{}", uuid::Uuid::new_v4()));
        let path = dir.join("missing.db");
        assert!(
            control_state::read_domains_only(&path, &[control_state::DOMAIN_DMONITOR_STATE])
                .is_err()
        );
        assert!(!dir.exists());
    }
    #[test]
    fn read_only_control_snapshot_does_not_change_tables_or_payloads() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("phase128-{}.db", uuid::Uuid::new_v4()));
        // Fixture setup is a write; the production acquisition under test is not.
        control_state::persist_domain_json(
            &path,
            control_state::DOMAIN_DMONITOR_STATE,
            &serde_json::json!({"fixture":true}),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let blobs =
            control_state::read_domains_only(&path, &[control_state::DOMAIN_DMONITOR_STATE])
                .unwrap();
        assert!(blobs[control_state::DOMAIN_DMONITOR_STATE].is_some());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
}

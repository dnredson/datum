//! Catalog-only mutations. No execution/placement/monitoring producers.
use crate::core::{service_artifact_registry::ServiceArtifactRegistry, service_catalog::*};
use axum::{
    extract::{rejection::{JsonRejection,QueryRejection}, Path, State, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct CatalogApi {
    store: Option<CatalogStore>,
    artifacts: ServiceArtifactRegistry,
    capacity: Arc<Semaphore>,
}
impl CatalogApi {
    pub fn editor_api(&self) -> super::dgraph_editor::EditorApi {
        super::dgraph_editor::EditorApi::from_process().with_catalog(self.store.clone())
    }
    pub fn modeling_api(&self) -> super::service_modeling::ModelingApi {
        super::service_modeling::ModelingApi::new(self.store.clone(), self.artifacts.clone())
    }
    pub fn from_process(artifacts: ServiceArtifactRegistry) -> Self {
        let store = std::env::var(CATALOG_PATH_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .and_then(|v| {
                let path = PathBuf::from(v);
                // The catalog must not initialize schema in an execution/event DB.
                if ["DATUM_CONTROL_STATE_DB_PATH", "DATUM_EVENT_ALERT_DB_PATH"]
                    .iter()
                    .any(|key| {
                        std::env::var(key)
                            .ok()
                            .is_some_and(|v| PathBuf::from(v) == path)
                    })
                {
                    return None;
                }
                CatalogStore::initialize(&path).ok()
            });
        Self {
            store,
            artifacts,
            capacity: Arc::new(Semaphore::new(4)),
        }
    }
    async fn run<T: Serialize + Send + 'static>(
        &self,
        f: impl FnOnce(CatalogStore, ServiceArtifactRegistry) -> Result<T> + Send + 'static,
    ) -> Response {
        let Some(store) = self.store.clone() else {
            return error(CatalogError::Unavailable);
        };
        let Ok(permit) = self.capacity.clone().try_acquire_owned() else {
            return error(CatalogError::Unavailable);
        };
        let artifacts = self.artifacts.clone();
        match tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f(store, artifacts)
        })
        .await
        {
            Ok(Ok(value)) => Json(value).into_response(),
            Ok(Err(e)) => error(e),
            Err(_) => error(CatalogError::Unavailable),
        }
    }
}
fn error(e: CatalogError) -> Response {
    let (status, code, message) = match e {
        CatalogError::Invalid => (
            StatusCode::BAD_REQUEST,
            "invalid_definition",
            "Catalog definition or artifact reference is invalid.",
        ),
        CatalogError::Conflict => (
            StatusCode::CONFLICT,
            "revision_conflict",
            "The definition revision or source artifact changed. Reload before saving.",
        ),
        CatalogError::NotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "The requested catalog record is not available.",
        ),
        CatalogError::Corrupt => (
            StatusCode::SERVICE_UNAVAILABLE,
            "catalog_corrupt",
            "Catalog integrity cannot be verified.",
        ),
        CatalogError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "catalog_unavailable",
            "Catalog persistence is not configured or cannot be accessed.",
        ),
    };
    (
        status,
        Json(json!({"error": {"code": code, "safe_message": message}})),
    )
        .into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub definition: ServiceDefinitionDraft,
    pub expected_revision: u64,
}
pub fn router(api: CatalogApi) -> Router {
    Router::new()
        .route("/api/v1/catalog/services", get(list).post(create))
        .route("/api/v1/catalog/services/:id", get(latest))
        .route(
            "/api/v1/catalog/services/:id/revisions",
            axum::routing::post(revise),
        )
        .route(
            "/api/v1/catalog/services/:id/revisions/:revision",
            get(exact),
        )
        .route("/api/v1/catalog/artifacts", get(artifacts))
        .route("/api/v1/catalog/artifacts/:snapshot", get(snapshot))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(api)
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryQuery { limit: Option<usize>, offset: Option<usize>, search: Option<String> }
async fn list(State(api): State<CatalogApi>, query: std::result::Result<Query<DiscoveryQuery>, QueryRejection>) -> Response {
    let Ok(Query(query)) = query else { return error(CatalogError::Invalid); };
    if query.limit.is_none() && query.offset.is_none() && query.search.is_none() {
        return api.run(|s, _| s.list()).await;
    }
    api.run(move |s, _| s.discover(query.search.as_deref().unwrap_or(""),query.offset.unwrap_or(0),query.limit.unwrap_or(25))).await
}
async fn latest(State(api): State<CatalogApi>, Path(id): Path<String>) -> Response {
    api.run(move |s, _| s.get(&id, None)).await
}
async fn exact(
    State(api): State<CatalogApi>,
    Path((id, revision)): Path<(String, String)>,
) -> Response {
    let Ok(revision) = revision.parse::<u64>() else {
        return error(CatalogError::Invalid);
    };
    api.run(move |s, _| s.get(&id, Some(revision))).await
}
async fn snapshot(State(api): State<CatalogApi>, Path(id): Path<String>) -> Response {
    api.run(move |s, _| s.artifact(&id)).await
}
#[derive(Serialize)]
struct ArtifactPage {
    items: Vec<ArtifactSnapshot>,
    has_more: bool,
    limit: usize,
    source: &'static str,
}
async fn artifacts(State(api): State<CatalogApi>) -> Response {
    api.run(|_, registry| {
        let (items, has_more) = registry
            .with_stable_view(|state| {
                let mut all = Vec::new();
                for artifacts in state.artifacts_by_project.values() {
                    for a in artifacts {
                        let reference = ArtifactReference {
                            project_id: a.control_plane.project_id.clone(),
                            artifact_id: a.artifact_id.clone(),
                            execution_generation: a.control_plane.execution_generation,
                            execution_projection_digest: a
                                .control_plane
                                .execution_projection_digest
                                .clone(),
                        };
                        if let Ok(snapshot) = capture_artifact(a, &reference) {
                            all.push(snapshot);
                        }
                    }
                }
                all.sort_by(|a, b| {
                    (&a.source.project_id, &a.source.artifact_id)
                        .cmp(&(&b.source.project_id, &b.source.artifact_id))
                });
                let more = all.len() > 100;
                all.truncate(100);
                (all, more)
            })
            .map_err(|_| CatalogError::Unavailable)?;
        Ok(ArtifactPage {
            items,
            has_more,
            limit: 100,
            source: "pinned_operational_artifact_references",
        })
    })
    .await
}
async fn save(
    api: CatalogApi,
    id: Option<String>,
    payload: std::result::Result<Json<SaveRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = payload else {
        return error(CatalogError::Invalid);
    };
    if id
        .as_ref()
        .is_some_and(|id| id != &body.definition.service_definition_id)
        || (id.is_none() && body.expected_revision != 0)
        || (id.is_some() && body.expected_revision == 0)
    {
        return error(CatalogError::Invalid);
    }
    api.run(move |s, registry| {
        let captured = body
            .definition
            .artifact
            .as_ref()
            .map(|reference| {
                let artifact = registry
                    .get(&reference.project_id, &reference.artifact_id)
                    .map_err(|_| CatalogError::Invalid)?;
                capture_artifact(&artifact, reference)
            })
            .transpose()?;
        s.save(body.definition, captured, body.expected_revision)
    })
    .await
}
async fn create(
    State(api): State<CatalogApi>,
    payload: std::result::Result<Json<SaveRequest>, JsonRejection>,
) -> Response {
    save(api, None, payload).await
}
async fn revise(
    State(api): State<CatalogApi>,
    Path(id): Path<String>,
    payload: std::result::Result<Json<SaveRequest>, JsonRejection>,
) -> Response {
    save(api, Some(id), payload).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        service_artifact_registry::{ServiceArtifact, ServiceArtifactRegistryState},
        service_catalog::tests::{database_path, draft, snapshot, store},
    };
    #[tokio::test]
    async fn discovery_api_paging_query_bounds_and_legacy_contract_are_read_only() {
        let server=serve(true).await;
        let path=server.catalog_path.as_ref().unwrap();let s=CatalogStore::initialize(path).unwrap();
        for i in 0..100 {let mut d=draft();d.service_definition_id=format!("definition-{i:03}");d.name=format!("Discovery Service {i:03}");d.provides=vec![format!("example.provider-{i:03}")];s.save(d,snapshot(),0).unwrap();}
        let before=std::fs::read(path).unwrap();let client=reqwest::Client::new();let url=format!("{}/api/v1/catalog/services",server.base);
        let first:serde_json::Value=client.get(format!("{url}?offset=0")).send().await.unwrap().json().await.unwrap();
        assert_eq!(first["schema"],"datum.catalog-discovery/1");assert_eq!(first["limit"],25);assert_eq!(first["total"],100);assert_eq!(first["items"].as_array().unwrap().len(),25);
        let last:serde_json::Value=client.get(format!("{url}?offset=75&limit=25")).send().await.unwrap().json().await.unwrap();assert_eq!(last["has_more"],false);
        let found:serde_json::Value=client.get(&url).query(&[("search","EXAMPLE.PROVIDER-099")]).send().await.unwrap().json().await.unwrap();assert_eq!(found["total"],1);assert_eq!(found["items"][0]["definition"]["service_definition_id"],"definition-099");
        let legacy:serde_json::Value=client.get(&url).send().await.unwrap().json().await.unwrap();assert_eq!(legacy["limit"],100);assert!(legacy.get("schema").is_none());
        for query in ["offset=-1","limit=-1","limit=0","limit=101","offset=1000001","offset=18446744073709551616","search=%0A","page=-1","limit=25&limit=30"]{
            let r=client.get(format!("{url}?{query}")).send().await.unwrap();assert_eq!(r.status(),reqwest::StatusCode::BAD_REQUEST);let body=r.text().await.unwrap();assert!(!body.contains(query));assert!(!body.contains("rusqlite"));
        }
        assert_eq!(std::fs::read(path).unwrap(),before);
    }
    struct Server {
        base: String,
        task: tokio::task::JoinHandle<()>,
        catalog_path: Option<PathBuf>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn serve(configured: bool) -> Server {
        let a = snapshot();
        let mut artifact: ServiceArtifact =
            serde_json::from_str(include_str!("../../../artifacts/catalog/mosquitto.json"))
                .unwrap();
        artifact.artifact_id = a.source.artifact_id.clone();
        artifact.control_plane.project_id = a.source.project_id.clone();
        artifact.control_plane.execution_generation = 1;
        artifact.control_plane.execution_projection_digest = a.source.execution_projection_digest;
        artifact.runtime.kind = "native_process".into();
        artifact.runtime.container = None;
        artifact.runtime.native_process = Some(
            crate::core::service_artifact_registry::NativeProcessDeploymentSpecV1 {
                process_name: "test-only".into(),
                entrypoint: "bin/sniffer".into(),
                args: vec![],
                working_directory: None,
            },
        );
        artifact.control_plane.acquisition.sha256 = a.executable_sha256;
        let state: ServiceArtifactRegistryState=serde_json::from_value(json!({"schema_version":"0.2.0","artifacts_by_project":{artifact.control_plane.project_id.clone():[artifact]},"execution_generation_high_water":{},"project_pinning_modes":{}})).unwrap();
        let catalog_store = configured.then(store);
        let catalog_path = catalog_store
            .as_ref()
            .map(|s| database_path(s).to_path_buf());
        let api = CatalogApi {
            store: catalog_store,
            artifacts: ServiceArtifactRegistry::from_state(state),
            capacity: Arc::new(Semaphore::new(4)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        Server {
            base,
            catalog_path,
            task: tokio::spawn(async move {
                axum::serve(listener, router(api)).await.unwrap();
            }),
        }
    }
    #[tokio::test]
    async fn catalog_api_create_list_latest_exact_new_revision_and_conflict() {
        let server = serve(true).await;
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/catalog/services", server.base);
        let payload = json!({"definition":draft(),"expected_revision":0});
        let created = client.post(&url).json(&payload).send().await.unwrap();
        assert_eq!(created.status(), 200);
        assert_eq!(
            created.json::<DefinitionRevision>().await.unwrap().revision,
            1
        );
        assert_eq!(
            client
                .post(&url)
                .json(&payload)
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        let mut d = draft();
        d.name = "Revision two".into();
        assert_eq!(
            client
                .post(format!("{url}/p4-sniffer/revisions"))
                .json(&json!({"definition":d,"expected_revision":1}))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .get(format!("{url}/p4-sniffer"))
                .send()
                .await
                .unwrap()
                .json::<DefinitionRevision>()
                .await
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            client
                .get(format!("{url}/p4-sniffer/revisions/1"))
                .send()
                .await
                .unwrap()
                .json::<DefinitionRevision>()
                .await
                .unwrap()
                .definition
                .name,
            "P4 Sniffer"
        );
        assert_eq!(
            client
                .get(&url)
                .send()
                .await
                .unwrap()
                .json::<CatalogPage>()
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            client
                .delete(format!("{url}/p4-sniffer"))
                .send()
                .await
                .unwrap()
                .status(),
            405
        );
    }
    #[tokio::test]
    async fn catalog_api_malformed_scope_stale_source_invalid_revision_and_safe_errors() {
        let server = serve(true).await;
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/catalog/services", server.base);
        let bad = client
            .post(&url)
            .header("content-type", "application/json")
            .body("{sql/path secret")
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 400);
        assert!(!bad.text().await.unwrap().contains("sql/path"));
        let mut d = draft();
        d.artifact.as_mut().unwrap().project_id = "other-project".into();
        assert_eq!(
            client
                .post(&url)
                .json(&json!({"definition":d,"expected_revision":0}))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        d = draft();
        d.artifact.as_mut().unwrap().execution_generation = 2;
        assert_eq!(
            client
                .post(&url)
                .json(&json!({"definition":d,"expected_revision":0}))
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        assert_eq!(
            client
                .get(format!("{url}/p4-sniffer/revisions/0"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .get(format!("{url}/p4-sniffer/revisions/bad"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .get(format!("{url}/absent"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    #[tokio::test]
    async fn catalog_api_unconfigured_never_falls_back_to_memory_and_returns_safe_503() {
        let server = serve(false).await;
        let r = reqwest::get(format!("{}/api/v1/catalog/services", server.base))
            .await
            .unwrap();
        assert_eq!(r.status(), 503);
        assert_eq!(
            r.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "catalog_unavailable"
        );
    }
    #[tokio::test]
    async fn removed_catalog_get_and_post_fail_without_recreating_database() {
        let server = serve(true).await;
        let path = server.catalog_path.as_ref().unwrap();
        std::fs::remove_file(path).unwrap();
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/catalog/services", server.base);
        for r in [
            client.get(&url).send().await.unwrap(),
            client
                .post(&url)
                .json(&json!({"definition":draft(),"expected_revision":0}))
                .send()
                .await
                .unwrap(),
        ] {
            assert_eq!(r.status(), 503);
            assert_eq!(
                r.json::<serde_json::Value>().await.unwrap()["error"]["code"],
                "catalog_unavailable"
            );
            assert!(!path.exists());
        }
    }
    #[tokio::test]
    async fn damaged_catalog_schema_is_not_an_empty_api_success() {
        for table in ["catalog_definitions", "catalog_artifacts"] {
            let server = serve(true).await;
            let path = server.catalog_path.as_ref().unwrap();
            rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )
            .unwrap()
            .execute(&format!("DROP TABLE {table}"), [])
            .unwrap();
            let r = reqwest::get(format!("{}/api/v1/catalog/services", server.base))
                .await
                .unwrap();
            assert_eq!(r.status(), 503);
            assert_eq!(
                r.json::<serde_json::Value>().await.unwrap()["error"]["code"],
                "catalog_corrupt"
            );
        }
    }
    #[tokio::test]
    async fn composite_and_exact_dependency_api_are_modeling_only() {
        let server = serve(true).await;
        let c = reqwest::Client::new();
        let url = format!("{}/api/v1/catalog/services", server.base);
        let mut d = crate::core::service_catalog::tests::composite();
        let r = c
            .post(&url)
            .json(&json!({"definition":d,"expected_revision":0}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(r
            .json::<DefinitionRevision>()
            .await
            .unwrap()
            .artifact_snapshot_id
            .is_none());
        d.exact_dependencies = vec![DefinitionReference {
            definition_id: "missing".into(),
            revision: 1,
        }];
        assert_eq!(
            c.post(format!("{url}/magistrala/revisions"))
                .json(&json!({"definition":d,"expected_revision":1}))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
}

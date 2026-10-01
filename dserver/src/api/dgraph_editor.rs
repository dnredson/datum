//! Phase 134: read accepted snapshots and validate local application drafts. No writer.
use crate::core::{
    canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1},
    ddeploy::{self, DDeployStateV1},
};
use crate::storage::control_state;
use axum::{
    extract::{rejection::JsonRejection, Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
type Reader = dyn Fn(&str) -> Result<Vec<CanonicalDGraphV1>, &'static str> + Send + Sync;
#[derive(Clone)]
pub struct EditorApi {
    reader: Arc<Reader>,
    catalog: Option<crate::core::service_catalog::CatalogStore>,
    capacity: Arc<Semaphore>,
}
impl EditorApi {
    pub fn with_catalog(mut self, catalog: Option<crate::core::service_catalog::CatalogStore>) -> Self {
        self.catalog = catalog;
        self
    }
    pub fn from_process() -> Self {
        let path: PathBuf = control_state::default_control_state_db_path();
        Self::from_path(path)
    }
    fn from_path(path: PathBuf) -> Self {
        Self::new(move |project| {
            let blobs =
                control_state::read_domains_only(&path, &[control_state::DOMAIN_DDEPLOY_STATE])
                    .map_err(|_| "source_unavailable")?;
            let Some(Some(blob)) = blobs.get(control_state::DOMAIN_DDEPLOY_STATE) else {
                return Err("not_configured");
            };
            let state: DDeployStateV1 = serde_json::from_str(blob).map_err(|_| "source_corrupt")?;
            ddeploy::validate_persisted_state_integrity(&state).map_err(|_| "source_corrupt")?;
            let mut applications = BTreeSet::new();
            for a in state.active_by_application.values() {
                if a.active_dmap.project_id == project {
                    applications.insert(a.application_id.clone());
                }
            }
            for a in state.phase113_v2.active.values() {
                if a.map.project_id == project {
                    applications.insert(a.map.application_id.clone());
                }
            }
            if applications.len() > 100 {
                return Err("source_oversized");
            }
            applications
                .into_iter()
                .map(|a| {
                    let active = ddeploy::active_placement_authority(&state, project, &a)
                        .map_err(|_| "source_corrupt")?
                        .ok_or("source_corrupt")?;
                    let graph = active.dgraph_snapshot.ok_or("source_corrupt")?;
                    let digest = active.dgraph_digest.ok_or("source_corrupt")?;
                    if recompute_dgraph_digest(&graph).map_err(|_| "source_corrupt")? != digest {
                        return Err("source_corrupt");
                    }
                    Ok(graph)
                })
                .collect()
        })
    }
    pub fn new(
        reader: impl Fn(&str) -> Result<Vec<CanonicalDGraphV1>, &'static str> + Send + Sync + 'static,
    ) -> Self {
        Self {
            reader: Arc::new(reader),
            catalog: None,
            capacity: Arc::new(Semaphore::new(2)),
        }
    }
}
pub fn router(api: EditorApi) -> Router {
    Router::new()
        .route(
            "/api/v1/operations/projects/:project/dgraph-editor",
            get(load),
        )
        .route(
            "/api/v1/operations/projects/:project/dgraph-editor/validate",
            post(validate),
        )
        .layer(axum::extract::DefaultBodyLimit::max(524288))
        .with_state(api)
}
fn error(code: &'static str) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"code":code,"message":"Graph source unavailable; local draft is unchanged."}}))).into_response()
}
fn scope(p: &str) -> bool {
    !p.is_empty() && p.len() <= 256 && p.trim() == p && !p.chars().any(char::is_control)
}
async fn read(api: EditorApi, project: String) -> Result<Vec<CanonicalDGraphV1>, Response> {
    if !scope(&project) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_project"}})),
        )
            .into_response());
    }
    let permit = api
        .capacity
        .clone()
        .try_acquire_owned()
        .map_err(|_| error("busy"))?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let graphs = (api.reader)(&project)?;
        if graphs.len() > 100
            || graphs.iter().any(|g| {
                !g.validate().is_empty()
                    || g.services.len() > 2000
                    || g.calls.len() > 4000
                    || recompute_dgraph_digest(g).is_err()
            })
        {
            return Err("source_corrupt");
        }
        let mut ids = BTreeSet::new();
        for g in &graphs {
            if !ids.insert(&g.application_id) {
                return Err("source_corrupt");
            }
        }
        Ok(graphs)
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .map_err(|_| error("timeout"))?
        .map_err(|_| error("source_unavailable"))?
        .map_err(error)
}
async fn load(State(api): State<EditorApi>, Path(project): Path<String>) -> Response {
    match read(api, project.clone()).await {
        Ok(mut graphs) => {
            graphs.sort_by(|a, b| a.application_id.cmp(&b.application_id));
            let entries: Vec<_> = graphs.iter().map(|g| json!({"application_id":g.application_id,"dgraph_id":g.dgraph_id,"revision":g.revision.to_string(),"digest":recompute_dgraph_digest(g).unwrap(),"graph_json":serde_json::to_string(g).unwrap()})).collect();
            let body = json!({"schema":"datum.dgraph-editor-base/1","project_id":project,"graphs":entries});
            if body.to_string().len() > 2097152 {
                return error("source_oversized");
            }
            Json(body).into_response()
        }
        Err(e) => e,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Base {
    pub application_id: String,
    pub dgraph_id: String,
    pub revision: String,
    pub digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    graph_json: String,
    base: Option<Base>,
}
#[derive(Serialize)]
struct Finding {
    path: String,
    code: &'static str,
    message: &'static str,
}
fn f(path: impl Into<String>, code: &'static str, message: &'static str) -> Finding {
    Finding {
        path: path.into(),
        code,
        message,
    }
}
fn inspect(raw: &str) -> Result<CanonicalDGraphV1, Vec<Finding>> {
    if raw.len() > 262144 {
        return Err(vec![f("$", "draft_oversized", "Draft exceeds 256 KiB.")]);
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| vec![f("$", "invalid_json", "Expected canonical DGraph JSON.")])?;
    // Decode the original JSON, not a Value-normalized copy: known duplicate
    // fields must be rejected before normalization can discard one of them.
    let graph: CanonicalDGraphV1 = serde_json::from_str(raw).map_err(|_| {
        vec![f(
            "$",
            "invalid_shape",
            "Expected datum.dgraph/1 canonical fields and integer quantities.",
        )]
    })?;
    if serde_json::to_value(&graph).unwrap() != value {
        return Err(vec![f(
            "$",
            "unexpected_fields",
            "All canonical fields must be present; unknown fields are rejected.",
        )]);
    }
    if graph.services.len() > 2000 || graph.calls.len() > 4000 {
        return Err(vec![f(
            "$",
            "draft_oversized",
            "Maximum 2000 services and 4000 calls.",
        )]);
    }
    let mut findings = Vec::new();
    // Canonical validator is the authority. Local scans only locate its findings, never relax them.
    for finding in graph.validate() {
        let (path, message) = match finding.code {
            "schema_invalid" => ("schema", "Expected datum.dgraph/1 or datum.dgraph/2."),
            "catalog_reference_requires_v2" => ("schema", "Exact catalog references require datum.dgraph/2."),
            "invalid_service_definition_reference" => ("services.service_definition", "Expected bounded definition ID, exact revision and sha256 digest."),
            "revision_invalid" => ("revision", "Revision must be at least one."),
            "duplicate_service_id" => ("services", "Service IDs must be unique."),
            "duplicate_call_id" => ("calls", "Call IDs must be unique."),
            "invalid_dnode_abi" => (
                "services.execution_requirement.dnode_abi",
                "Expected datum-dnode/0.",
            ),
            "invalid_cpu_requirement" => (
                "services.resource_requirement.cpu_cores",
                "CPU cores must be a positive whole number.",
            ),
            "invalid_memory_requirement" => (
                "services.resource_requirement.memory_bytes",
                "Memory bytes must be a positive exact integer.",
            ),
            "dangling_call_source" => (
                "calls.source_service_id",
                "Source must name a declared service.",
            ),
            "dangling_call_target" => (
                "calls.target_service_id",
                "Target must name a declared service.",
            ),
            _ => ("$", "Canonical identity fields must be non-empty."),
        };
        if findings.len() < 32 {
            findings.push(f(path, finding.code, message));
        }
    }
    for id in [&graph.application_id, &graph.dgraph_id]
        .into_iter()
        .chain(graph.services.iter().map(|s| &s.service_id))
        .chain(graph.calls.iter().map(|c| &c.call_id))
    {
        if id.len() > 256 || id.trim() != id || id.chars().any(char::is_control) {
            if findings.len() < 32 {
                findings.push(f(
                    "$",
                    "invalid_editor_identity",
                    "Editor identities must be bounded, unpadded and control-free.",
                ));
            }
        }
    }
    if findings.is_empty() {
        if recompute_dgraph_digest(&graph).is_err() {
            return Err(vec![f(
                "$",
                "canonical_digest_invalid",
                "Canonical JCS digest requires safe-integer quantities (at most 2^53-1).",
            )]);
        }
        Ok(graph)
    } else {
        Err(findings)
    }
}
async fn validate(
    State(api): State<EditorApi>,
    Path(project): Path<String>,
    body: Result<Json<Request>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_request"}})),
        )
            .into_response();
    };
    let graph = match inspect(&request.graph_json) {
        Ok(g) => g, Err(findings) => return Json(json!({"schema":"datum.dgraph-editor-validation/1","valid":false,"findings":findings,"digest":null})).into_response()
    };
    let verification = api.clone();
    let graphs = match read(api, project).await {
        Ok(g) => g,
        Err(e) => return e,
    };
    let current = graphs
        .iter()
        .find(|g| g.application_id == graph.application_id);
    let matches = match (&request.base, current) {
        (None, None) => graph.revision == 1,
        (Some(base), Some(g)) => {
            base.application_id == g.application_id
                && base.dgraph_id == g.dgraph_id
                && base.revision == g.revision.to_string()
                && base.digest == recompute_dgraph_digest(g).unwrap()
                && graph.dgraph_id == g.dgraph_id
                && Some(graph.revision) == g.revision.checked_add(1)
        }
        _ => false,
    };
    if !matches {
        return (StatusCode::CONFLICT, Json(json!({"error":{"code":"base_revision_conflict","message":"Accepted base changed or draft identity/revision mismatches. Draft was not rebased."}}))).into_response();
    }
    if graph.services.iter().any(|s| s.service_definition.is_some()) {
        let Ok(permit) = verification.capacity.clone().try_acquire_owned() else { return error("busy"); };
        let candidate = graph.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            verify_references(&candidate, verification.catalog.as_ref())
        });
        let result = match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
            Ok(Ok(result)) => result,
            _ => return error("catalog_unavailable"),
        };
        match result {
            Err(code) => return error(code),
            Ok(findings) if !findings.is_empty() => return Json(json!({"schema":"datum.dgraph-editor-validation/1","valid":false,"findings":findings,"digest":null})).into_response(),
            _ => (),
        }
    }
    Json(json!({"schema":"datum.dgraph-editor-validation/1","valid":true,"findings":[],"digest":recompute_dgraph_digest(&graph).unwrap()})).into_response()
}

fn verify_references(graph: &CanonicalDGraphV1, store: Option<&crate::core::service_catalog::CatalogStore>) -> Result<Vec<Finding>, &'static str> {
    use crate::core::service_catalog::CatalogError;
    let mut findings = Vec::new();
    for (index, service) in graph.services.iter().enumerate() {
        let Some(reference) = &service.service_definition else { continue; };
        let store = store.ok_or("catalog_unavailable")?;
        let code = match store.get(&reference.definition_id, Some(reference.revision)) {
            Ok(record) if record.digest == reference.digest => continue,
            Ok(_) => "definition_digest_mismatch",
            Err(CatalogError::NotFound) => "definition_revision_not_found",
            Err(CatalogError::Corrupt) => return Err("catalog_corrupt"),
            Err(CatalogError::Unavailable) => return Err("catalog_unavailable"),
            Err(_) => "invalid_service_definition_reference",
        };
        if findings.len() < 32 { findings.push(f(format!("services[{index}].service_definition"), code, "Exact catalog revision and digest could not be verified. No latest fallback.")); }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    #[tokio::test]
    async fn exact_catalog_references_are_read_only_and_never_latest() {
        use crate::core::service_catalog::{CatalogStore, ServiceDefinitionDraft, ArtifactSnapshot};
        let path = std::env::temp_dir().join(format!("phase134b-catalog-{}.db", uuid::Uuid::new_v4()));
        let store = CatalogStore::initialize(&path).unwrap();
        let definition: ServiceDefinitionDraft = serde_json::from_value(json!({
            "service_definition_id":"magistrala","name":"Magistrala","artifact":null,"runtime_kind":null,
            "service_shape":"composite","composition":{"mode":"discovered","discovery":{"kind":"docker_compose","compose_files":["compose.yaml"]}},
            "provisioning":{"kind":"repository_command","repository_url":"https://github.com/absmach/magistrala.git","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","command":["make","run_latest"]},
            "tenancy":"shared_tenant_aware","tenant_unit":"domain","resources":{"cpu_millis":null,"memory_mb":null,"storage_mb":null,"capabilities":[]},
            "health_check":null,"monitoring":null,"secrets":[],"exact_dependencies":[],"requires":[],"provides":[]
        })).unwrap();
        let record = store.save(definition.clone(), None::<ArtifactSnapshot>, 0).unwrap();
        store.save(definition, None::<ArtifactSnapshot>, 1).unwrap();
        let before = std::fs::read(&path).unwrap();
        let mut g = graph(); g.schema = "datum.dgraph/2".into();
        g.services[0].service_definition = Some(crate::core::canonical_dgraph::ServiceDefinitionReference {
            definition_id:"magistrala".into(), revision:1, digest:record.digest.clone()
        });
        assert!(verify_references(&g, Some(&store)).unwrap().is_empty());
        let api = EditorApi::new(|_| Ok(vec![])).with_catalog(Some(store.clone()));
        let response = validate(State(api), Path("p".into()), Ok(Json(Request { graph_json:serde_json::to_string(&g).unwrap(),base:None }))).await;
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(),524288).await.unwrap()).unwrap();
        assert_eq!(body["valid"], true);
        for (id, revision, digest, code) in [
            ("missing",1,record.digest.clone(),"definition_revision_not_found"),
            ("magistrala",3,record.digest.clone(),"definition_revision_not_found"),
            ("magistrala",1,format!("sha256:{}","b".repeat(64)),"definition_digest_mismatch")
        ] {
            let r=g.services[0].service_definition.as_mut().unwrap();r.definition_id=id.into();r.revision=revision;r.digest=digest;
            assert_eq!(verify_references(&g,Some(&store)).unwrap()[0].code,code);
        }
        assert_eq!(std::fs::read(&path).unwrap(),before);
        assert_eq!(store.list().unwrap().items.len(),1);
        assert_eq!(verify_references(&g,None).err(),Some("catalog_unavailable"));
        let damaged = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
        damaged.execute("DROP TABLE catalog_definitions", []).unwrap();
        drop(damaged);
        assert_eq!(verify_references(&g,Some(&store)).err(),Some("catalog_corrupt"));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(verify_references(&g,Some(&store)).err(),Some("catalog_unavailable"));
        assert!(!path.exists());
    }
    fn graph() -> CanonicalDGraphV1 {
        serde_json::from_str(r#"{"schema":"datum.dgraph/1","application_id":"app","dgraph_id":"g","revision":1,"services":[{"service_id":"a","execution_requirement":{"dnode_abi":"datum-dnode/0"},"resource_requirement":{"cpu_cores":1,"memory_bytes":9007199254740991}}],"calls":[]}"#).unwrap()
    }
    async fn check(
        g: CanonicalDGraphV1,
        base: Option<Base>,
        source: Vec<CanonicalDGraphV1>,
    ) -> (StatusCode, Value) {
        let api = EditorApi::new(move |p| {
            if p == "p" {
                Ok(source.clone())
            } else {
                Ok(vec![])
            }
        });
        let response = validate(
            State(api),
            Path("p".into()),
            Ok(Json(Request {
                graph_json: serde_json::to_string(&g).unwrap(),
                base,
            })),
        )
        .await;
        let status = response.status();
        let body = to_bytes(response.into_body(), 524288).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
    #[tokio::test]
    async fn valid_empty_new_and_self_calls_are_non_mutating() {
        let mut g = graph();
        g.calls
            .push(crate::core::canonical_dgraph::CanonicalDCallV1 {
                call_id: "self".into(),
                source_service_id: "a".into(),
                target_service_id: "a".into(),
            });
        assert_eq!(check(g.clone(), None, vec![]).await.1["valid"], true);
        g.services.clear();
        g.calls.clear();
        assert_eq!(check(g, None, vec![]).await.1["valid"], true);
    }
    #[tokio::test]
    async fn invalid_nodes_edges_duplicates_and_abi_are_findings() {
        for case in ["cpu", "abi", "duplicate", "dangling", "duplicate-call"] {
            let mut g = graph();
            match case {
                "cpu" => g.services[0].resource_requirement.cpu_cores = 0,
                "abi" => g.services[0].execution_requirement.dnode_abi = "container".into(),
                "duplicate" => g.services.push(g.services[0].clone()),
                _ => {
                    g.calls
                        .push(crate::core::canonical_dgraph::CanonicalDCallV1 {
                            call_id: "c".into(),
                            source_service_id: "a".into(),
                            target_service_id: if case == "dangling" { "missing" } else { "a" }
                                .into(),
                        });
                    if case == "duplicate-call" {
                        g.calls.push(g.calls[0].clone());
                    }
                }
            }
            let (status, body) = check(g, None, vec![]).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["valid"], false);
            assert!(body["findings"].as_array().unwrap().len() > 0);
        }
    }
    #[tokio::test]
    async fn exact_base_conflict_and_max_u64_remain_lossless() {
        let g = graph();
        let base = Base {
            application_id: g.application_id.clone(),
            dgraph_id: g.dgraph_id.clone(),
            revision: "1".into(),
            digest: recompute_dgraph_digest(&g).unwrap(),
        };
        let mut draft = g.clone();
        draft.revision = 2;
        assert_eq!(
            check(draft.clone(), Some(base.clone()), vec![g.clone()])
                .await
                .1["valid"],
            true
        );
        assert_eq!(
            check(draft.clone(), None, vec![g.clone()]).await.0,
            StatusCode::CONFLICT
        );
        let scoped = g.clone();
        let wrong_project_api = EditorApi::new(move |project| {
            Ok(if project == "p" {
                vec![scoped.clone()]
            } else {
                vec![]
            })
        });
        let response = validate(
            State(wrong_project_api),
            Path("other-project".into()),
            Ok(Json(Request {
                graph_json: serde_json::to_string(&draft).unwrap(),
                base: Some(base.clone()),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let mut stale = g;
        stale.revision = 2;
        assert_eq!(
            check(draft, Some(base), vec![stale]).await.0,
            StatusCode::CONFLICT
        );
    }
    #[test]
    fn strict_body_bounds_and_no_unknown_semantics() {
        let mut v = json!(graph());
        v["placement"] = json!("no");
        assert!(inspect(&v.to_string()).is_err());
        assert!(inspect(&"x".repeat(262145)).is_err());
        let raw = serde_json::to_string(&graph()).unwrap()
            .replace("\"service_id\":\"a\"", "\"service_id\":\"hidden\",\"service_id\":\"a\"");
        assert!(inspect(&raw).is_err());
    }
    #[tokio::test]
    async fn missing_database_read_does_not_create_persistence() {
        let p = std::env::temp_dir().join(format!("phase134-missing-{}.db", uuid::Uuid::new_v4()));
        assert!(
            control_state::read_domains_only(&p, &[control_state::DOMAIN_DDEPLOY_STATE]).is_err()
        );
        assert!(!p.exists());
    }
    #[tokio::test]
    async fn real_http_empty_store_validation_leaves_database_byte_identical() {
        let p = std::env::temp_dir().join(format!("phase134-readonly-{}.db", uuid::Uuid::new_v4()));
        control_state::persist_domain_json(
            &p,
            control_state::DOMAIN_DDEPLOY_STATE,
            &DDeployStateV1::empty(),
        )
        .unwrap();
        let before = std::fs::read(&p).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let api = EditorApi::from_path(p.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router(api)).await.unwrap() });
        let client = reqwest::Client::new();
        let base = format!("http://{addr}/api/v1/operations/projects/p/dgraph-editor");
        let loaded: Value = client
            .get(&base)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(loaded["graphs"], json!([]));
        let body: Value = client
            .post(format!("{base}/validate"))
            .json(&json!({"graph_json":serde_json::to_string(&graph()).unwrap(),"base":null}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["valid"], true);
        assert_eq!(std::fs::read(&p).unwrap(), before);
        server.abort();
        // Exact test-created temporary database; no production file is touched.
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn oversized_jcs_integer_is_safe_finding_not_panic() {
        let mut g = graph();
        g.services[0].resource_requirement.memory_bytes = u64::MAX;
        let findings = inspect(&serde_json::to_string(&g).unwrap()).unwrap_err();
        assert_eq!(findings[0].code, "canonical_digest_invalid");
    }
}

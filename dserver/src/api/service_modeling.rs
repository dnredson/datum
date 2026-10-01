use crate::core::{
    service_artifact_registry::ServiceArtifactRegistry,
    service_catalog::CatalogStore,
    service_modeling::{self as modeling, ModelingRequest, ProviderError, ServiceModelingProvider},
};
use axum::{
    extract::{rejection::JsonRejection, Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct ModelingApi {
    store: Option<CatalogStore>,
    artifacts: ServiceArtifactRegistry,
    provider: Arc<dyn ServiceModelingProvider>,
    capacity: Arc<Semaphore>,
    deadline: std::time::Duration,
}
impl ModelingApi {
    pub fn new(store: Option<CatalogStore>, artifacts: ServiceArtifactRegistry) -> Self {
        Self {
            store,
            artifacts,
            provider: Arc::new(modeling::NotConfiguredProvider),
            capacity: Arc::new(Semaphore::new(2)),
            deadline: std::time::Duration::from_secs(30),
        }
    }
    /// Provider-neutral host injection; production configuration stays outside catalog authority.
    pub fn with_provider(mut self, p: Arc<dyn ServiceModelingProvider>) -> Self {
        self.provider = p;
        self
    }
}
pub fn router(api: ModelingApi) -> Router {
    Router::new()
        .route("/api/v1/catalog/modeling-assistant", get(status))
        .route("/api/v1/catalog/modeling-assistant/draft", post(draft))
        .layer(axum::extract::DefaultBodyLimit::max(32768))
        .with_state(api)
}
async fn status(State(api): State<ModelingApi>) -> Json<serde_json::Value> {
    Json(
        json!({"schema_version":"datum.service-modeling-status/1","state":if api.provider.identity().0=="not_configured"{"not_configured"}else{"available"},"provider":api.provider.identity().0,"model_ref":api.provider.identity().1,"repository_analysis":"not_included"}),
    )
}
fn error(e: ProviderError) -> Response {
    if let ProviderError::InvalidCandidate(findings) = e {
        return (StatusCode::BAD_GATEWAY,Json(json!({"error":{"code":"provider_invalid_candidate","message":"Provider output failed deterministic validation. No draft was applied.","findings":modeling::bounded_findings(&findings)}}))).into_response();
    }
    let (status, code) = match e {
        ProviderError::NotConfigured => (StatusCode::SERVICE_UNAVAILABLE, "not_configured"),
        ProviderError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
        ProviderError::InvalidResponse => (StatusCode::BAD_GATEWAY, "invalid_response"),
        ProviderError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        ProviderError::Indeterminate => (StatusCode::SERVICE_UNAVAILABLE, "indeterminate"),
        ProviderError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        ProviderError::InvalidCandidate(_) => unreachable!(),
    };
    (status,Json(json!({"error":{"code":code,"safe_message":"Modeling assistant could not produce a validated draft. Manual catalog authoring remains available."}}))).into_response()
}
async fn draft(
    State(api): State<ModelingApi>,
    body: Result<Json<ModelingRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_request"}})),
        )
            .into_response();
    };
    if !modeling::valid_request(&request) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_request"}})),
        )
            .into_response();
    }
    if api.provider.identity().0 == "not_configured" {
        return error(ProviderError::NotConfigured);
    }
    let Some(store) = api.store else {
        return error(ProviderError::Unavailable);
    };
    let Ok(permit) = api.capacity.try_acquire_owned() else {
        return error(ProviderError::Unavailable);
    };
    let deadline = api.deadline;
    let request_id = uuid::Uuid::new_v4().to_string();
    let identity = api.provider.identity();
    let safe_identity = |s: &str| {
        if s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
        {
            s.to_string()
        } else {
            "unverified".to_string()
        }
    };
    let provider_id = safe_identity(identity.0);
    let model = safe_identity(identity.1);
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let snapshots = request
            .artifact
            .as_ref()
            .map(|r| {
                api.artifacts
                    .get(&r.project_id, &r.artifact_id)
                    .map_err(|_| ProviderError::InvalidResponse)
                    .and_then(|a| {
                        crate::core::service_catalog::capture_artifact(&a, r)
                            .map_err(|_| ProviderError::InvalidResponse)
                    })
            })
            .transpose()?
            .into_iter()
            .collect();
        let context = modeling::build_context(&store, &request, snapshots)?;
        modeling::interpret(&request, &context, &store, api.provider.as_ref())
    });
    match tokio::time::timeout(deadline, task).await {
        Ok(Ok(Ok(value))) => Json(value).into_response(),
        Ok(Ok(Err(e))) => {
            if let ProviderError::InvalidCandidate(findings) = &e {
                let paths_and_codes: Vec<_> = modeling::bounded_findings(findings)
                    .iter()
                    .map(|f| format!("{}:{}", f.path, f.code))
                    .collect();
                tracing::warn!(provider=%provider_id,model=%model,request_id=%request_id,findings=?paths_and_codes,"service modeling candidate rejected");
            }
            error(e)
        }
        Ok(Err(_)) => error(ProviderError::Unavailable),
        Err(_) => error(ProviderError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        service_artifact_registry::ServiceArtifactRegistryState,
        service_catalog::tests::{composite, store},
        service_modeling::tests::{candidate, request, Mock},
    };
    fn api() -> (ModelingApi, CatalogStore) {
        let s = store();
        let state:ServiceArtifactRegistryState=serde_json::from_value(json!({"schema_version":"0.2.0","artifacts_by_project":{},"execution_generation_high_water":{},"project_pinning_modes":{}})).unwrap();
        (
            ModelingApi::new(Some(s.clone()), ServiceArtifactRegistry::from_state(state)),
            s,
        )
    }
    async fn serve(api: ModelingApi) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        (
            base,
            tokio::spawn(async move { axum::serve(listener, router(api)).await.unwrap() }),
        )
    }
    #[tokio::test]
    async fn production_is_not_configured_and_manual_catalog_is_untouched() {
        let (api, s) = api();
        let (base, task) = serve(api).await;
        let client = reqwest::Client::new();
        let status: serde_json::Value = client
            .get(format!("{base}/api/v1/catalog/modeling-assistant"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["state"], "not_configured");
        let r = client
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&request())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 503);
        assert_eq!(
            r.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "not_configured"
        );
        assert!(s.list().unwrap().items.is_empty());
        task.abort();
    }
    #[tokio::test]
    async fn injected_provider_success_is_read_only_and_exact_contract() {
        let (api, s) = api();
        let mut c = candidate();
        c.partial_definition =
            serde_json::from_value(serde_json::to_value(composite()).unwrap()).unwrap();
        let api = api.with_provider(Arc::new(Mock(Ok(serde_json::to_string(&c).unwrap()))));
        let (base, task) = serve(api).await;
        let mut r = request();
        r.artifact = None;
        r.intent = "a".repeat(40);
        let response = reqwest::Client::new()
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&r)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["validation"]["state"], "valid");
        assert_eq!(body["side_effects_performed"], false);
        assert!(s.list().unwrap().items.is_empty());
        task.abort();
    }
    #[tokio::test]
    async fn invalid_requests_and_provider_errors_are_safe() {
        let (api, _) = api();
        let api = api.with_provider(Arc::new(Mock(Ok("RAW provider secret/path stack".into()))));
        let (base, task) = serve(api).await;
        let client = reqwest::Client::new();
        let mut r = request();
        r.artifact = None;
        let response = client
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&r)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
        let raw = response.text().await.unwrap();
        assert!(!raw.contains("RAW"));
        assert!(!raw.contains("stack"));
        let mut v = serde_json::to_value(r).unwrap();
        v["catalog_context"] = json!({});
        let response = client
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&v)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        task.abort();
    }
    struct Slow;
    impl ServiceModelingProvider for Slow {
        fn identity(&self) -> (&str, &str) {
            ("test_only", "slow")
        }
        fn generate(&self, _: &modeling::ProviderInput) -> Result<String, ProviderError> {
            std::thread::sleep(std::time::Duration::from_millis(100));
            Err(ProviderError::Unavailable)
        }
    }
    #[tokio::test]
    async fn timed_out_worker_retains_capacity_until_completion() {
        let (api, _) = api();
        let mut api = api.with_provider(Arc::new(Slow));
        api.deadline = std::time::Duration::from_millis(5);
        let capacity = api.capacity.clone();
        let (base, task) = serve(api).await;
        let mut r = request();
        r.artifact = None;
        let response = reqwest::Client::new()
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&r)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 504);
        assert_eq!(capacity.available_permits(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while capacity.available_permits() != 2 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("completed worker must release its permit");
        assert_eq!(capacity.available_permits(), 2);
        task.abort();
    }
    #[tokio::test]
    async fn rejected_candidate_returns_bounded_findings_without_raw_body_or_credentials() {
        let (api, s) = api();
        let mut c = candidate();
        c.partial_definition = serde_json::from_value(json!(composite())).unwrap();
        c.partial_definition.get_mut("provisioning").unwrap()["commit"] = json!("main");
        c.warnings = vec!["RAW_PROVIDER_BODY_API_KEY_SECRET".into()];
        let api = api.with_provider(Arc::new(Mock(Ok(serde_json::to_string(&c).unwrap()))));
        let (base, task) = serve(api).await;
        let mut r = request();
        r.artifact = None;
        let response = reqwest::Client::new()
            .post(format!("{base}/api/v1/catalog/modeling-assistant/draft"))
            .json(&r)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
        let raw = response.text().await.unwrap();
        assert!(!raw.contains("RAW_PROVIDER_BODY_API_KEY_SECRET"));
        assert!(!raw.contains("make"));
        assert!(!raw.contains("github"));
        let body: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["error"]["code"], "provider_invalid_candidate");
        assert_eq!(body["error"]["findings"][0]["path"], "provisioning.commit");
        assert_eq!(body["error"]["findings"][0]["code"], "invalid_git_commit");
        assert!(s.list().unwrap().items.is_empty());
        task.abort();
    }
    #[test]
    fn findings_transport_is_bounded() {
        let f = modeling::ValidationFinding {
            path: "artifact".into(),
            code: "unknown_artifact".into(),
            message: "Expected an exact server-known selected pinned artifact.".into(),
        };
        let mut values = vec![f.clone(); 100];
        values[0].path = "x".repeat(129);
        values[1].message = "x".repeat(257);
        values[2].code = "x".repeat(65);
        let safe = modeling::bounded_findings(&values);
        assert_eq!(safe.len(), 16);
        assert!(safe
            .iter()
            .all(|f| f.path.len() <= 128 && f.code.len() <= 64 && f.message.len() <= 256));
    }
}

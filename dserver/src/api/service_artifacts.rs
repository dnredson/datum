use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    core::{
        oci_image_pinning::{pin_image_identity, ImagePinningError},
        oci_registry_resolver::{PlatformSpec, ReqwestOciHttpTransport},
        service_artifact_registry::{
            ServiceArtifact, ServiceArtifactList, ServiceArtifactRegistryError, ServiceArtifactRegistry,
        },
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    project_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "missing or invalid X-Datum-Project header" })),
        )
    })
}

fn map_registry_error(error: ServiceArtifactRegistryError) -> ApiError {
    let status = match &error {
        ServiceArtifactRegistryError::Invalid(_) => StatusCode::BAD_REQUEST,
        ServiceArtifactRegistryError::Conflict(_) => StatusCode::CONFLICT,
        ServiceArtifactRegistryError::NotFound(_) => StatusCode::NOT_FOUND,
        ServiceArtifactRegistryError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, JsonResponse(json!({ "error": error.message() })))
}

pub async fn list_service_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<ServiceArtifactList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .list(&project_id)
        .map(JsonResponse)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": error })),
            )
        })
}

pub async fn get_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .get(&project_id, &artifact_id)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub async fn post_service_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceArtifact>,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .create(&project_id, payload)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub async fn put_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ServiceArtifact>,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .update(&project_id, &artifact_id, payload)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub const OCI_RESOLVER_TOOL_VERSION: &str = "datum-oci-resolver/0.1.0";

#[derive(Debug, Deserialize)]
pub struct PlatformSpecPayload {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveImageIdentityRequest {
    pub target_platforms: Vec<PlatformSpecPayload>,
    /// A caller-supplied audit label / asserted actor label -- required,
    /// never inferred. v7 review P2-1: this route has no application-layer
    /// authentication (see the module doc comment below), so this value is
    /// NOT a verified operator identity; it is provenance only, exactly as
    /// trustworthy as whatever the caller chose to send.
    pub resolved_by: String,
}

fn map_image_pinning_error(error: ImagePinningError) -> ApiError {
    match &error {
        ImagePinningError::Registry(registry_error) => map_registry_error(registry_error.clone()),
        ImagePinningError::LocalBuildNotResolvable(_) => {
            (StatusCode::BAD_REQUEST, JsonResponse(json!({ "error": error.to_string() })))
        }
        ImagePinningError::LegacyUnclassifiedNotResolvable(_) => {
            (StatusCode::BAD_REQUEST, JsonResponse(json!({ "error": error.to_string() })))
        }
        // v6 review P0-2/P1-2: a disallowed host or an invalid target
        // platform is a CLIENT input problem (400), not an upstream
        // failure (502) -- these are refused before any outbound request
        // is ever made.
        ImagePinningError::Resolution(inner)
            if matches!(
                inner,
                crate::core::oci_registry_resolver::OciResolutionError::RegistryHostNotAllowed(_)
                    | crate::core::oci_registry_resolver::OciResolutionError::TokenRealmHostNotAllowed(_)
                    | crate::core::oci_registry_resolver::OciResolutionError::InvalidTargetPlatform(_)
                    | crate::core::oci_registry_resolver::OciResolutionError::InvalidReference(_)
            ) =>
        {
            (StatusCode::BAD_REQUEST, JsonResponse(json!({ "error": error.to_string() })))
        }
        ImagePinningError::Resolution(_) => {
            (StatusCode::BAD_GATEWAY, JsonResponse(json!({ "error": error.to_string() })))
        }
    }
}

/// Stage 107B-A (ADR-0013). Resolves `artifact_id`'s registry-backed image
/// reference into an immutable, multi-platform identity and persists it
/// BEFORE authorization -- never invoked implicitly during M1/reconciliation.
///
/// **v6 review P0-2 correction.** This is NOT a "separately authorized
/// administrative action" today -- DServer has no application-layer
/// authentication/authorization on this route. `X-Datum-Project` is request
/// scoping, not authorization. The actual safety boundary is
/// `oci_registry_resolver`'s registry-host / token-realm-host allow-list,
/// enforced before the first outbound request; see that module's
/// trust-boundary doc comment.
///
/// `pin_image_identity`'s transport is synchronous (mirrors
/// `ContainerRuntime`; no `async_trait` dependency exists in this
/// workspace), so the blocking HTTP work runs inside
/// `tokio::task::spawn_blocking` rather than directly on this async handler.
pub async fn post_resolve_image_identity(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ResolveImageIdentityRequest>,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    if payload.resolved_by.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "resolved_by must not be empty (a caller-supplied audit label; not a verified operator identity)" })),
        ));
    }
    let target_platforms: Vec<PlatformSpec> = payload
        .target_platforms
        .into_iter()
        .map(|platform| PlatformSpec { os: platform.os, architecture: platform.architecture, variant: platform.variant })
        .collect();
    // v6 review P1-2: validate at the API boundary too (defense in depth --
    // `resolve_image_identity` validates again regardless), so a malformed
    // request gets an immediate, precise 400 rather than surfacing through
    // the resolver as an opaque failure.
    if let Err(error) = crate::core::oci_registry_resolver::validate_target_platforms(&target_platforms) {
        return Err((StatusCode::BAD_REQUEST, JsonResponse(json!({ "error": error.to_string() }))));
    }
    let resolved_by = payload.resolved_by.trim().to_string();

    let registry = state.service_artifacts.clone();
    tokio::task::spawn_blocking(move || {
        let transport = ReqwestOciHttpTransport::new();
        pin_image_identity(&registry, &transport, &project_id, &artifact_id, &target_platforms, OCI_RESOLVER_TOOL_VERSION, &resolved_by)
    })
    .await
    .map_err(|join_error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            JsonResponse(json!({ "error": format!("image-identity resolve task failed: {join_error}") })),
        )
    })?
    .map(JsonResponse)
    .map_err(map_image_pinning_error)
}

pub async fn delete_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .delete(&project_id, &artifact_id)
        .map_err(map_registry_error)?;
    Ok(StatusCode::NO_CONTENT)
}

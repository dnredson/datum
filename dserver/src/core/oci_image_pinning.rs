//! Stage 107B-A (ADR-0013): the explicit image-identity pin operation.
//!
//! `pin_image_identity` is the ONLY way a `ServiceArtifact`'s
//! `resolved_image_identity` is ever set (enforced structurally: the public
//! create/update API rejects any client-submitted non-None value outright --
//! see `service_artifact_registry::normalize_artifact` -- and this function
//! never calls `ServiceArtifactRegistry::update`, only the internal
//! `apply_resolved_image_identity`). It is invoked via
//! `POST /api/v1/service-artifacts/:artifact_id/image-identity/resolve`,
//! NEVER implicitly during M1/reconciliation or authorization -- an image
//! must be immutably pinned BEFORE an authorization snapshot is issued, not
//! silently resolved at execution time.
//!
//! **v6 review P0-2 correction.** This is NOT, today, a "separately
//! authorized administrative action" -- DServer has no application-layer
//! authentication/authorization on this route (or on the ordinary
//! service-artifacts routes). The actual safety boundary against this being
//! an arbitrary outbound-request primitive is `oci_registry_resolver`'s
//! registry-host / token-realm-host allow-list, enforced before any
//! network call. Do not describe this endpoint as authorized anywhere in
//! this codebase until a real authentication/authorization layer exists.
//!
//! **v6 review P1-2/P1-3 flow.** `pin_image_identity` follows
//! resolve -> validate -> CAS apply, never resolve -> blindly persist:
//! `resolve_image_identity` itself validates the requested target platforms
//! before any network call; its result is then independently re-validated
//! via `validate_resolved_image_identity` (repository agreement, digest
//! validity, platform-set consistency, and digest-selector consistency)
//! BEFORE `apply_resolved_image_identity` is ever called, and
//! `apply_resolved_image_identity` itself re-validates defensively, one more
//! time, immediately before persisting -- so a fabricated or
//! internally-inconsistent candidate can never reach storage even if this
//! function's own pre-check were ever bypassed or a future caller misused
//! the internal API.
//!
//! **v5 review P0-2 (TOCTOU / lost-update race).** Network resolution
//! (`resolve_image_identity`) can take seconds and is deliberately performed
//! with NO registry lock held. To avoid a slow resolution silently
//! overwriting a newer artifact state with a stale result, this function
//! captures an `ImageResolutionExpectedState` snapshot BEFORE resolving, and
//! passes it to `apply_resolved_image_identity`, which re-checks it inside
//! ONE write-lock critical section and refuses to persist
//! (`Conflict("artifact_changed_during_image_resolution")`) if anything
//! changed in the meantime.

use crate::core::oci_registry_resolver::{resolve_image_identity, OciHttpTransport, OciResolutionError, PlatformSpec};
use crate::core::service_artifact_registry::{
    ImageResolutionExpectedState, ResolvedImageIdentityV1, ServiceArtifact, ServiceArtifactRegistry,
    ServiceArtifactRegistryError, IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED, IMAGE_SOURCE_KIND_LOCAL_BUILD,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImagePinningError {
    Registry(ServiceArtifactRegistryError),
    LocalBuildNotResolvable(String),
    LegacyUnclassifiedNotResolvable(String),
    Resolution(OciResolutionError),
}

impl std::fmt::Display for ImagePinningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(error) => write!(f, "registry error: {}", error.message()),
            Self::LocalBuildNotResolvable(image) => write!(
                f,
                "'{image}' has image_source_kind=local_build and cannot be resolved against a registry (no docker-image-inspect-based local trust is invented; see ADR-0013)"
            ),
            Self::LegacyUnclassifiedNotResolvable(image) => write!(
                f,
                "'{image}' has image_source_kind=legacy_unclassified and cannot be pinned until an operator explicitly classifies it as registry or local_build (see ADR-0013, v5 review P0-5)"
            ),
            Self::Resolution(error) => write!(f, "resolution error: {error}"),
        }
    }
}
impl std::error::Error for ImagePinningError {}

fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Resolves `artifact_id`'s image reference against the real registry (or
/// the injected fake, in tests) for exactly the requested `target_platforms`,
/// then persists the result via `ServiceArtifactRegistry::apply_resolved_image_identity`
/// -- the compare-and-swap boundary that refuses to persist if the artifact
/// changed while resolution was in flight (P0-2). Any resolution failure, or
/// a CAS conflict, returns before anything is persisted -- the registry is
/// left completely unchanged. `resolved_by` (P1-1, corrected by v7 review
/// P2-1) is a caller-supplied audit label / asserted actor label for
/// whoever/whatever triggered this pin -- NOT a verified operator identity,
/// since this route has no application-layer authentication; it is
/// persisted as administrative/audit metadata, excluded from the execution
/// projection.
pub fn pin_image_identity(
    registry: &ServiceArtifactRegistry,
    transport: &dyn OciHttpTransport,
    project_id: &str,
    artifact_id: &str,
    target_platforms: &[PlatformSpec],
    resolver_tool_version: &str,
    resolved_by: &str,
) -> Result<ServiceArtifact, ImagePinningError> {
    let artifact = registry.get(project_id, artifact_id).map_err(ImagePinningError::Registry)?;
    let container = artifact.runtime.container.as_ref().ok_or_else(|| {
        ImagePinningError::Registry(ServiceArtifactRegistryError::Invalid(format!(
            "artifact '{artifact_id}' has no container runtime definition"
        )))
    })?;
    if container.image_source_kind == IMAGE_SOURCE_KIND_LOCAL_BUILD {
        // Amendment 2: never invent local trust via docker-image-inspect --
        // a local_build artifact simply cannot be resolved in this slice.
        return Err(ImagePinningError::LocalBuildNotResolvable(container.image.clone()));
    }
    if container.image_source_kind == IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED {
        // P0-5: an unclassified artifact (migrated legacy data, or a
        // careless new-schema write missing the field) is never trusted
        // enough to pin -- an operator must explicitly classify it first.
        return Err(ImagePinningError::LegacyUnclassifiedNotResolvable(container.image.clone()));
    }

    // P0-2: capture the expected state BEFORE the slow network call, with
    // NO lock held during it.
    let expected = ImageResolutionExpectedState {
        execution_generation: artifact.control_plane.execution_generation,
        execution_projection_digest: artifact.control_plane.execution_projection_digest.clone(),
        image: container.image.clone(),
        image_source_kind: container.image_source_kind.clone(),
    };

    let resolved =
        resolve_image_identity(transport, &container.image, target_platforms).map_err(ImagePinningError::Resolution)?;

    // v6 review P1-2/P1-3: validate the FULLY resolved identity -- including
    // digest-selector consistency against the declared image reference --
    // before it is ever persisted, not after.
    crate::core::oci_registry_resolver::validate_resolved_image_identity(&container.image, &resolved)
        .map_err(|message| {
            ImagePinningError::Registry(ServiceArtifactRegistryError::Invalid(format!(
                "resolved image identity failed validation before persistence: {message}"
            )))
        })?;

    let resolved = ResolvedImageIdentityV1 {
        resolved_at_utc: now_utc(),
        resolver_tool_version: resolver_tool_version.to_string(),
        resolved_by: resolved_by.to_string(),
        ..resolved
    };

    registry
        .apply_resolved_image_identity(project_id, artifact_id, &expected, resolved)
        .map_err(ImagePinningError::Registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::oci_registry_resolver::test_support::{amd64, queue_mosquitto_single_manifest};
    use crate::core::oci_registry_resolver::FakeOciHttpTransport;
    use crate::core::service_artifact_registry::{
        ContainerDeploymentSpec, ServiceArtifactAcquisition, ServiceArtifactControlPlane, ServiceArtifactRegistryState,
        ServiceArtifactWasmDecision, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
        IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED, IMAGE_SOURCE_KIND_LOCAL_BUILD, IMAGE_SOURCE_KIND_REGISTRY,
        SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION, SERVICE_ARTIFACT_SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;

    fn artifact_with_image(image: &str, image_source_kind: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-mqtt".to_string(),
                    image: image.to_string(),
                    ports: Vec::new(),
                    network: None,
                    network_aliases: Vec::new(),
                    volumes: Vec::new(),
                    configuration_mounts: Vec::new(),
                    command: Vec::new(),
                    image_source_kind: image_source_kind.to_string(),
                    resolved_image_identity: None,
                }),
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "docker_container_running".to_string(),
                    params: BTreeMap::new(),
                    vantage_point: None,
                },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                project_id: "project-a".to_string(),
                name: "MQTT".to_string(),
                version: "2".to_string(),
                artifact_kind: "container".to_string(),
                target_nodes: vec!["fog-01".to_string()],
                target_stages: Vec::new(),
                dependencies: Vec::new(),
                acquisition: ServiceArtifactAcquisition { kind: "image".to_string(), ..Default::default() },
                wasm_decision: ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    fn empty_registry() -> ServiceArtifactRegistry {
        ServiceArtifactRegistry::from_state(ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            ..Default::default()
        })
    }

    #[test]
    fn pin_that_resolves_a_new_identity_increments_generation_exactly_once() {
        let registry = empty_registry();
        let created = registry
            .create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY))
            .unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);

        let transport = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&transport, &amd64());

        let pinned = pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
            .unwrap();
        assert_eq!(pinned.control_plane.execution_generation, 2);
        assert!(pinned.runtime.container.as_ref().unwrap().resolved_image_identity.is_some());
        assert_ne!(pinned.control_plane.execution_projection_digest, created.control_plane.execution_projection_digest);
    }

    #[test]
    fn resolved_by_is_recorded_on_the_persisted_identity() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();
        let transport = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&transport, &amd64());

        let pinned =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:alice")
                .unwrap();
        assert_eq!(
            pinned.runtime.container.as_ref().unwrap().resolved_image_identity.as_ref().unwrap().resolved_by,
            "operator:alice"
        );
    }

    #[test]
    fn idempotent_re_pin_is_a_generation_no_op() {
        let registry = empty_registry();
        registry
            .create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY))
            .unwrap();

        let transport_1 = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&transport_1, &amd64());
        let first =
            pin_image_identity(&registry, &transport_1, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap();
        assert_eq!(first.control_plane.execution_generation, 2);

        // Deliberately a DIFFERENT transport instance and a fresh
        // resolved_at_utc timestamp -- proves that resolving the SAME
        // logical identity again does not bump the generation, because
        // resolved_at_utc/resolver_tool_version/resolved_by/registry_host
        // are administrative and excluded from the projection (Amendment 3).
        let transport_2 = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&transport_2, &amd64());
        let second =
            pin_image_identity(&registry, &transport_2, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:different")
                .unwrap();
        assert_eq!(second.control_plane.execution_generation, 2, "identical resolved identity must not bump generation again");
        assert_eq!(second.control_plane.execution_projection_digest, first.control_plane.execution_projection_digest);
    }

    #[test]
    fn failed_resolution_leaves_the_artifact_completely_unchanged() {
        let registry = empty_registry();
        let created = registry
            .create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY))
            .unwrap();

        // No response queued -> RegistryUnreachable, before any persist.
        let transport = FakeOciHttpTransport::new();
        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(matches!(error, ImagePinningError::Resolution(_)), "{error:?}");

        let unchanged = registry.get("project-a", "mqtt").unwrap();
        assert_eq!(unchanged, created, "a failed resolution must never persist a partial write");
    }

    #[test]
    fn local_build_artifacts_are_refused_without_inventing_local_trust() {
        let registry = empty_registry();
        registry
            .create("project-a", artifact_with_image("datum/sma:dev", IMAGE_SOURCE_KIND_LOCAL_BUILD))
            .unwrap();

        let transport = FakeOciHttpTransport::new();
        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(matches!(error, ImagePinningError::LocalBuildNotResolvable(_)), "{error:?}");
        assert!(transport.calls().is_empty(), "a local_build artifact must never reach the resolver/transport at all");
    }

    // ---- P0-5: legacy_unclassified is never pinnable -------------------- //
    #[test]
    fn legacy_unclassified_artifacts_are_refused() {
        let registry = empty_registry();
        registry
            .create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED))
            .unwrap();

        let transport = FakeOciHttpTransport::new();
        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(matches!(error, ImagePinningError::LegacyUnclassifiedNotResolvable(_)), "{error:?}");
        assert!(transport.calls().is_empty(), "a legacy_unclassified artifact must never reach the resolver/transport at all");
    }

    // ---- P0-2: TOCTOU / lost-update concurrency tests -------------------- //

    /// A transport whose `execute` calls a closure BEFORE serving the queued
    /// response -- used to simulate "another writer commits a change while
    /// resolution is in flight" at the exact moment the network call would
    /// have happened in production.
    struct InterleavingTransport<'a> {
        inner: FakeOciHttpTransport,
        on_first_call: std::cell::RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }
    impl<'a> InterleavingTransport<'a> {
        fn new(inner: FakeOciHttpTransport, on_first_call: impl FnOnce() + 'a) -> Self {
            Self { inner, on_first_call: std::cell::RefCell::new(Some(Box::new(on_first_call))) }
        }
    }
    impl<'a> OciHttpTransport for InterleavingTransport<'a> {
        fn execute(
            &self,
            request: &crate::core::oci_registry_resolver::OciHttpRequest,
        ) -> Result<crate::core::oci_registry_resolver::OciHttpResponse, OciResolutionError> {
            if let Some(callback) = self.on_first_call.borrow_mut().take() {
                callback();
            }
            self.inner.execute(request)
        }
    }

    #[test]
    fn concurrent_image_put_during_resolution_is_rejected_as_a_conflict() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let fake = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&fake, &amd64());
        let registry_for_interleave = registry.clone();
        let transport = InterleavingTransport::new(fake, move || {
            // Simulate a concurrent PUT that changes the image while
            // resolution is "in flight" (i.e. between this transport call
            // and the eventual apply_resolved_image_identity CAS check).
            let mut changed = artifact_with_image("eclipse-mosquitto:2.1", IMAGE_SOURCE_KIND_REGISTRY);
            changed.artifact_id = "mqtt".to_string();
            registry_for_interleave.update("project-a", "mqtt", changed).unwrap();
        });

        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        match error {
            ImagePinningError::Registry(ServiceArtifactRegistryError::Conflict(message)) => {
                assert_eq!(message, "artifact_changed_during_image_resolution");
            }
            other => panic!("expected a Conflict, got {other:?}"),
        }

        // The concurrent PUT's content must survive untouched -- the stale
        // pin must not have overwritten it.
        let current = registry.get("project-a", "mqtt").unwrap();
        assert_eq!(current.runtime.container.as_ref().unwrap().image, "eclipse-mosquitto:2.1");
        assert!(current.runtime.container.as_ref().unwrap().resolved_image_identity.is_none());
    }

    #[test]
    fn concurrent_unrelated_projection_changing_put_during_resolution_is_rejected_as_a_conflict() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let fake = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&fake, &amd64());
        let registry_for_interleave = registry.clone();
        let transport = InterleavingTransport::new(fake, move || {
            // An unrelated but projection-changing update (network_aliases
            // is part of the projection) -- generation and digest both
            // advance, which must still be detected as a conflict even
            // though `image`/`image_source_kind` themselves are unchanged.
            let mut current = registry_for_interleave.get("project-a", "mqtt").unwrap();
            current.runtime.container.as_mut().unwrap().network_aliases = vec!["mqtt".to_string()];
            registry_for_interleave.update("project-a", "mqtt", current).unwrap();
        });

        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(
            matches!(error, ImagePinningError::Registry(ServiceArtifactRegistryError::Conflict(ref m)) if m == "artifact_changed_during_image_resolution"),
            "{error:?}"
        );
    }

    #[test]
    fn delete_recreate_during_resolution_is_rejected_as_a_conflict() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let fake = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&fake, &amd64());
        let registry_for_interleave = registry.clone();
        let transport = InterleavingTransport::new(fake, move || {
            registry_for_interleave.delete("project-a", "mqtt").unwrap();
            registry_for_interleave
                .create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY))
                .unwrap();
        });

        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(
            matches!(error, ImagePinningError::Registry(ServiceArtifactRegistryError::Conflict(ref m)) if m == "artifact_changed_during_image_resolution"),
            "{error:?}"
        );
    }

    #[test]
    fn source_kind_change_during_resolution_is_rejected_as_a_conflict() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let fake = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&fake, &amd64());
        let registry_for_interleave = registry.clone();
        let transport = InterleavingTransport::new(fake, move || {
            let mut current = registry_for_interleave.get("project-a", "mqtt").unwrap();
            current.runtime.container.as_mut().unwrap().image_source_kind = IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string();
            registry_for_interleave.update("project-a", "mqtt", current).unwrap();
        });

        let error =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap_err();
        assert!(
            matches!(error, ImagePinningError::Registry(ServiceArtifactRegistryError::Conflict(ref m)) if m == "artifact_changed_during_image_resolution"),
            "{error:?}"
        );
    }

    #[test]
    fn unchanged_artifact_during_resolution_still_succeeds() {
        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let fake = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&fake, &amd64());
        // Interleave a callback that reads but changes nothing -- proves
        // the CAS check does not spuriously reject an unchanged artifact.
        let registry_for_interleave = registry.clone();
        let transport = InterleavingTransport::new(fake, move || {
            let _ = registry_for_interleave.get("project-a", "mqtt").unwrap();
        });

        let pinned =
            pin_image_identity(&registry, &transport, "project-a", "mqtt", &[amd64()], "test-resolver/0.1.0", "operator:test")
                .unwrap();
        assert!(pinned.runtime.container.as_ref().unwrap().resolved_image_identity.is_some());
    }

    // ---- v6 review P1-4: platform order is canonical end to end --------- //
    #[test]
    fn re_pinning_the_same_platforms_in_a_different_request_order_is_a_generation_no_op() {
        use crate::core::oci_registry_resolver::test_support::{arm64, config_blob_body, config_digest_of, single_manifest_body};
        use crate::core::oci_registry_resolver::OciHttpResponse;
        use crate::core::sha256::sha256_hex;

        let registry = empty_registry();
        registry.create("project-a", artifact_with_image("eclipse-mosquitto:2", IMAGE_SOURCE_KIND_REGISTRY)).unwrap();

        let amd64_config_digest = config_digest_of(&amd64());
        let arm64_config_digest = config_digest_of(&arm64());
        let amd64_body = single_manifest_body(&amd64_config_digest);
        let arm64_body = single_manifest_body(&arm64_config_digest);
        let amd64_digest = format!("sha256:{}", sha256_hex(&amd64_body));
        let arm64_digest = format!("sha256:{}", sha256_hex(&arm64_body));
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                { "mediaType": "application/vnd.docker.distribution.manifest.v2+json", "digest": amd64_digest, "platform": { "os": "linux", "architecture": "amd64" } },
                { "mediaType": "application/vnd.docker.distribution.manifest.v2+json", "digest": arm64_digest, "platform": { "os": "linux", "architecture": "arm64" } },
            ],
        })
        .to_string()
        .into_bytes();

        let queue_all = |transport: &FakeOciHttpTransport| {
            transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", Ok(OciHttpResponse { status: 200, headers: Vec::new(), body: index_body.clone() }));
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{amd64_digest}"),
                Ok(OciHttpResponse { status: 200, headers: Vec::new(), body: amd64_body.clone() }),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{arm64_digest}"),
                Ok(OciHttpResponse { status: 200, headers: Vec::new(), body: arm64_body.clone() }),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{amd64_config_digest}"),
                Ok(OciHttpResponse { status: 200, headers: Vec::new(), body: config_blob_body(&amd64()) }),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{arm64_config_digest}"),
                Ok(OciHttpResponse { status: 200, headers: Vec::new(), body: config_blob_body(&arm64()) }),
            );
        };

        let transport_1 = FakeOciHttpTransport::new();
        queue_all(&transport_1);
        let first = pin_image_identity(&registry, &transport_1, "project-a", "mqtt", &[amd64(), arm64()], "test-resolver/0.1.0", "operator:a")
            .unwrap();

        let transport_2 = FakeOciHttpTransport::new();
        queue_all(&transport_2);
        // Re-pin requesting the SAME platforms in the OPPOSITE order.
        let second = pin_image_identity(&registry, &transport_2, "project-a", "mqtt", &[arm64(), amd64()], "test-resolver/0.1.0", "operator:b")
            .unwrap();

        assert_eq!(
            second.control_plane.execution_generation, first.control_plane.execution_generation,
            "requesting the same platform set in a different order must not bump the generation"
        );
        assert_eq!(second.control_plane.execution_projection_digest, first.control_plane.execution_projection_digest);
    }
}

use std::collections::BTreeMap;
use std::path::PathBuf;

use datum::agent::service_artifact::{
    load_service_artifact, validate_service_artifact, ContainerDeploymentSpec, ContainerPortMapping,
    ServiceArtifact, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
    SERVICE_ARTIFACT_SCHEMA_VERSION,
};

#[test]
fn mqtt_service_artifact_fixture_is_valid_and_declares_host_exposure() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/fixtures/o2-service-mqtt-artifact.json");
    let artifact = load_service_artifact(&path).expect("MQTT service artifact fixture should load");
    assert_eq!(artifact.service_id, "mqtt");
    let container = artifact
        .runtime
        .container
        .as_ref()
        .expect("MQTT artifact should use a container runtime");
    assert_eq!(container.container_name, "datum-mqtt");
    assert_eq!(container.ports.len(), 1);
    assert_eq!(container.ports[0].container_port, 1883);
    assert_eq!(container.ports[0].host_port, Some(1883));
    assert_eq!(artifact.probes.execution.kind, "docker_container_running");
    assert_eq!(
        artifact.probes.availability.as_ref().unwrap().kind,
        "mqtt_connect"
    );
}

#[test]
fn unsupported_execution_probe_is_rejected() {
    let artifact = ServiceArtifact {
        schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
        artifact_id: "service:test".to_string(),
        service_id: "test".to_string(),
        runtime: ServiceRuntimeSpec {
            kind: "container".to_string(),
            container: Some(ContainerDeploymentSpec {
                container_name: "test".to_string(),
                image: "example:test".to_string(),
                ports: vec![ContainerPortMapping {
                    container_port: 1234,
                    host_port: None,
                    protocol: "tcp".to_string(),
                }],
            }),
        },
        probes: ServiceProbeSet {
            execution: ServiceProbeSpec {
                kind: "arbitrary_shell".to_string(),
                params: BTreeMap::new(),
                vantage_point: None,
            },
            health: None,
            availability: None,
        },
        metadata: None,
    };
    assert!(validate_service_artifact(&artifact).is_err());
}

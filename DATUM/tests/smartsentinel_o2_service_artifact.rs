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
                network: None,
                volumes: Vec::new(),
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


#[test]
fn magistrala_bootstrap_db_catalog_contract_is_valid() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dserver/fixtures/e2e/service-artifacts/magistrala-bootstrap-db.json");
    let raw = std::fs::read_to_string(&path).expect("Bootstrap DB artifact should be readable");
    let document: serde_json::Value =
        serde_json::from_str(&raw).expect("Bootstrap DB artifact should be valid JSON");

    assert_eq!(
        document["artifact_id"].as_str(),
        Some("magistrala-bootstrap-db")
    );
    assert_eq!(
        document["runtime"]["container"]["image"].as_str(),
        Some("postgres:16.2-alpine")
    );
    assert_eq!(
        document["runtime"]["container"]["network"].as_str(),
        Some("datum-magistrala-net")
    );
    assert_eq!(
        document["runtime"]["container"]["volumes"][0]["name"].as_str(),
        Some("datum-magistrala-bootstrap-db-volume")
    );
    assert_eq!(
        document["control_plane"]["bindings"]["POSTGRES_DB"].as_str(),
        Some("bootstrap")
    );
    assert_eq!(
        document["control_plane"]["bindings"]["POSTGRES_USER"].as_str(),
        Some("datum")
    );
    assert_eq!(
        document["control_plane"]["secret_refs"][0].as_str(),
        Some("magistrala_bootstrap_db_postgres_password")
    );

    let artifact = load_service_artifact(&path)
        .expect("Bootstrap DB artifact should satisfy the SmartSentinel artifact schema");
    validate_service_artifact(&artifact)
        .expect("Bootstrap DB artifact should pass SmartSentinel validation");
}


#[test]
fn magistrala_bootstrap_db_tcp_probe_uses_host_parameter() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dserver/fixtures/e2e/service-artifacts/magistrala-bootstrap-db.json");
    let raw = std::fs::read_to_string(&path).expect("Bootstrap DB artifact should be readable");
    let document: serde_json::Value =
        serde_json::from_str(&raw).expect("Bootstrap DB artifact should be valid JSON");

    assert_eq!(
        document["probes"]["health"]["params"]["host"].as_str(),
        Some("127.0.0.1")
    );
    assert!(document["probes"]["health"]["params"]["target"].is_null());
}

#[test]
fn magistrala_atom_bootstrap_job_catalog_contract_is_valid() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dserver/fixtures/e2e/service-artifacts/magistrala-atom-bootstrap.json");
    let raw = std::fs::read_to_string(&path)
        .expect("Atom bootstrap artifact should be readable");
    let document: serde_json::Value =
        serde_json::from_str(&raw).expect("Atom bootstrap artifact should be valid JSON");

    assert_eq!(
        document["artifact_id"].as_str(),
        Some("magistrala-atom-bootstrap")
    );
    assert_eq!(
        document["runtime"]["container"]["image"].as_str(),
        Some("ghcr.io/absmach/magistrala/atom-bootstrap:v0.51.0")
    );
    assert_eq!(
        document["runtime"]["container"]["network"].as_str(),
        Some("datum-magistrala-net")
    );
    assert_eq!(
        document["runtime"]["container"]["ports"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        document["control_plane"]["dependencies"][0].as_str(),
        Some("magistrala-atom")
    );
    assert_eq!(
        document["control_plane"]["secret_refs"][0].as_str(),
        Some("magistrala_atom_bootstrap_admin_secret")
    );
    assert_eq!(
        document["metadata"]["completion_mode"].as_str(),
        Some("successful_exit")
    );

    let artifact = load_service_artifact(&path)
        .expect("Atom bootstrap artifact should satisfy the SmartSentinel artifact schema");
    validate_service_artifact(&artifact)
        .expect("Atom bootstrap artifact should pass SmartSentinel validation");
}

